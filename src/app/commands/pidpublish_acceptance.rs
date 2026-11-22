//! PUB-7 验收：拦截率与一致性。错误级内置规则每条造一张只犯它的图，逐张发布都被门禁拦下、什么
//! 也不写；规则包把 DR-004、DR-102 停了，重复位号照样拦；经存盘操作存下的图读回来，摘要与存之前
//! 相同。

use super::*;
use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
use crate::pid::master_data::{self, Diameter, Entry, MasterData, PROJECT_FILE_VERSION};
use crate::pid::persist::PidAuthor;
use crate::pid::project_index::{
    summarize, DrawingSummary, SummaryMeta, SummarySource, DEFAULT_TOLERANCE,
};
use crate::pid::symbols::SymbolLibrary;
use crate::pid::topology::{handle_text, parse_handle, TopologyTable};
use std::sync::atomic::{AtomicU64, Ordering};

/// 错误级内置规则里本期取不到值、不出结果的两条（DRC 验收里的 `SILENT`）。
const SILENT: [&str; 2] = ["DR-020", "DR-109"];
/// `PIDINSERT pump` 插的块。
const PUMP: &str = "PID_SYM_equipment_pump";
/// `PIDINSERT gate_valve` 插的块。
const GATE_VALVE: &str = "PID_SYM_valve_gate_valve";

/// 项目目录，用完删掉。
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        // 测试并行跑：同一纳秒里建的两个靠计数分开。
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "ocs-pidaccept-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("临时目录");
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// 目录里现有的东西，连同子目录里的，各是相对路径和大小，排好序。
    fn listing(&self) -> Vec<(String, u64)> {
        fn walk(dir: &Path, root: &Path, found: &mut Vec<(String, u64)>) {
            for entry in fs::read_dir(dir).expect("读目录").flatten() {
                let path = entry.path();
                let name = path.strip_prefix(root).expect("在目录里");
                // 按路径取大小：Windows 上目录项里记的大小，开着的文件（编辑租约的锁文件）要过
                // 一阵才更新。
                let len = fs::metadata(&path).map_or(0, |meta| meta.len());
                found.push((name.to_string_lossy().into_owned(), len));
                if path.is_dir() {
                    walk(&path, root, found);
                }
            }
        }
        let mut found = Vec::new();
        walk(&self.0, &self.0, &mut found);
        found.sort();
        found
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 一个程序，开着一张新图。
fn fresh() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app
}

/// 协议 1 的一条控制请求，发给当前的图。
fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("test");
    req["request_id"] = json!(format!(
        "pidaccept-{}",
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    app.automation_op(&req.to_string())
}

/// 起 `cmd`，依次喂点，最后回车。
fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
    let started = request(app, json!({"op": "run", "cmd": cmd}));
    assert_eq!(started["ok"], true, "{cmd}：{started}");
    for [x, y] in points {
        let point = json!({"op": "input", "kind": "point", "point": [x, y, 0.0], "space": "wcs"});
        let fed = request(app, point);
        assert_eq!(fed["ok"], true, "{cmd} 喂点：{fed}");
    }
    request(app, json!({"op": "input", "kind": "enter"}));
    app.finish_all_pending_history();
}

/// 像在命令行上敲进去那样跑一行，它在后台起的任务也一直跑到完；不许报错。
fn run(app: &mut OpenCADStudio, line: &str) {
    app.command_line.last_error = None;
    let task = app.run_command_line(line);
    app.drive_headless_task(task).expect("命令的任务");
    assert_eq!(app.command_line.last_error, None, "{line}");
}

/// 用存盘操作把当前图存成 `path`。
fn save(app: &mut OpenCADStudio, path: &Path) {
    let saved = request(app, json!({"op": "save", "path": path, "overwrite": true}));
    assert_eq!(saved["ok"], true, "{saved}");
    assert!(!app.tabs[app.active_tab].dirty, "存完还是已修改");
}

/// 发 `ask`，应答 `scanning` 就再发：测试里后台读随应答跑完，再发就是结论。
fn settled(app: &mut OpenCADStudio, ask: &Value) -> Value {
    for _ in 0..5 {
        let answer = request(app, ask.clone());
        if answer["status"] != "scanning" {
            return answer;
        }
    }
    panic!("{ask}：一直在扫");
}

/// 项目规则包：错误级的内置规则只留 `keep`（`None` 时一条不留），别的都停用；有错也存得了盘
/// （`warn`）。返回停用了的规则。
fn rule_pack(scratch: &Scratch, keep: Option<&str>) -> Vec<String> {
    let off: Vec<String> = builtin_rules::builtin()
        .rules
        .into_iter()
        .filter(|rule| rule.severity == Severity::Error && Some(rule.id.as_str()) != keep)
        .map(|rule| rule.id)
        .collect();
    let rules: Vec<Value> = off
        .iter()
        .map(|id| json!({ "id": id, "enabled": false }))
        .collect();
    let pack = json!({
        "version": 1,
        "pack": keep.map_or_else(|| "错误级规则全停".to_owned(), |id| format!("只留 {id}")),
        "rule_set_version": "P1",
        "save_gate": "warn",
        "rules": rules,
    });
    fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("写项目包");
    off
}

/// 项目主数据，照 DRC 验收用的那份：公称直径 DN50、DN80；管道等级 A1（碳钢，最大公称压力 16）、
/// B2（不锈钢，40）；流体介质工艺水、蒸汽、氯气。
fn master_data_in(scratch: &Scratch) {
    let diameter = |dn: u32| Diameter {
        dn,
        label: format!("DN{dn}"),
        outer_diameter: None,
        hidden: false,
    };
    let entry = |name: &str, attrs: &[(&str, &str)]| Entry {
        name: name.to_owned(),
        attrs: attrs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    };
    let data = MasterData {
        version: PROJECT_FILE_VERSION,
        revision: 0,
        updated_at: 0,
        legend_standard: None,
        nominal_diameters: vec![diameter(50), diameter(80)],
        pipe_classes: vec![
            entry("A1", &[("材质", "碳钢"), ("最大公称压力", "16")]),
            entry("B2", &[("材质", "不锈钢"), ("最大公称压力", "40")]),
        ],
        insulation_classes: Vec::new(),
        fluid_codes: vec![
            entry("工艺水", &[("相态", "液")]),
            entry("蒸汽", &[("相态", "气")]),
            entry("氯气", &[("相态", "气")]),
        ],
        pbs: Vec::new(),
    };
    master_data::write_project_file(&scratch.0, &data, None).expect("写项目主数据");
}

/// 图上用块 `block` 插的、带 P&ID 记录的实体，按句柄排。
fn inserts_of(app: &OpenCADStudio, block: &str) -> Vec<document::Handle> {
    let doc = app.active_document_for_test();
    let mut found: Vec<document::Handle> = doc
        .entities()
        .filter(|entity| {
            matches!(entity, document::EntityType::Insert(insert) if insert.block_name == block)
        })
        .map(|entity| entity.common().handle)
        .filter(|handle| PidAuthor::load(doc, *handle).is_some())
        .collect();
    found.sort_by_key(|handle| handle.value());
    found
}

fn record(app: &OpenCADStudio, handle: document::Handle) -> PidAuthor {
    PidAuthor::load(app.active_document_for_test(), handle).expect("P&ID 记录")
}

/// 直接改图上记录里的位号：空的、重的位号命令给不出来。
fn retag(app: &mut OpenCADStudio, handle: document::Handle, tag: &str) {
    let mut author = record(app, handle);
    author.tag = tag.to_owned();
    let i = app.active_tab;
    author.store(&mut app.tabs[i].scene.document, handle);
}

fn topology(app: &OpenCADStudio) -> TopologyTable {
    TopologyTable::load(app.active_document_for_test())
}

/// 直接改图上的拓扑表。
fn edit_topology(app: &mut OpenCADStudio, change: impl FnOnce(&mut TopologyTable)) {
    let i = app.active_tab;
    let doc = &mut app.tabs[i].scene.document;
    let mut table = TopologyTable::load(doc);
    change(&mut table);
    table.store(doc);
}

/// 管线 L-1001，两头都不接东西。
fn pipe_line(app: &mut OpenCADStudio) {
    draw(app, "PIDPIPE", &[[0.0, 0.0], [200.0, 0.0]]);
}

/// 项目里另一张没打开的图 PID-1102：另起一个程序画 `build`，直接写成文件。
fn other_drawing(scratch: &Scratch, build: impl FnOnce(&mut OpenCADStudio)) {
    let mut other = fresh();
    build(&mut other);
    let bytes = crate::io::save_to_bytes(
        other.active_document_for_test(),
        "dwg",
        document::DxfVersion::AC1032,
    )
    .expect("存 DWG");
    fs::write(scratch.path("PID-1102.dwg"), bytes).expect("写文件");
}

/// 在 `app` 的图上造出规则 `rule` 的一处违规，照 DRC 验收里这条规则命中的例子用命令画；命令给不出的
/// （位号空着、两台同位号、协同服务器配过的对）直接改图上的记录。要主数据的写项目主数据，跨图的
/// 在目录里放第二张图 PID-1102。
fn violate(rule: &str, app: &mut OpenCADStudio, scratch: &Scratch) {
    match rule {
        // 管道等级、公称直径都没填，两头都空着。
        "DR-001" | "DR-002" | "DR-014" => pipe_line(app),
        // 没有项目主数据就没有图例标准；泵的名称没填。
        "DR-009" | "DR-010" => draw(app, "PIDINSERT pump", &[[0.0, 0.0]]),
        "DR-003" => {
            draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
            let pump = inserts_of(app, PUMP)[0];
            retag(app, pump, "");
        }
        "DR-004" => {
            draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
            draw(app, "PIDINSERT pump", &[[100.0, 0.0]]);
            let pumps = inserts_of(app, PUMP);
            let tag = record(app, pumps[0]).tag;
            retag(app, pumps[1], &tag);
        }
        "DR-006" => {
            pipe_line(app);
            draw(app, "PIDCONNECTOR pipe_out", &[[200.0, 0.0]]);
        }
        "DR-007" => {
            master_data_in(scratch);
            pipe_line(app);
            draw(app, "PIDPIPE run", &[[0.0, 0.0], [200.0, 0.0]]);
            draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
            let valve = parse_handle(&topology(app).fittings[0].handle).expect("阀门句柄");
            let tag = record(app, valve).tag;
            run(app, "PIDSETPROP L-1001 管道等级 A1");
            run(app, &format!("PIDSETPROP {tag} 压力等级 40"));
        }
        "DR-011" => {
            pipe_line(app);
            draw(app, "PIDPIPE run", &[[0.0, 0.0], [200.0, 0.0]]);
            let pipe_run = parse_handle(&topology(app).pipe_runs[0].handle).expect("管路句柄");
            retag(app, pipe_run, "L-1001");
        }
        "DR-015" => draw(app, "PIDPIPE signal", &[[0.0, 50.0], [50.0, 50.0]]),
        "DR-021" | "DR-022" | "DR-103" | "DR-104" => {
            master_data_in(scratch);
            pipe_line(app);
            let props: &[&str] = match rule {
                "DR-021" => &["流体介质 氯气", "管道等级 A1"],
                "DR-022" => &["管道等级 C3"],
                "DR-103" => &["公称直径 100"],
                _ => &["流体介质 氨"],
            };
            for prop in props {
                run(app, &format!("PIDSETPROP L-1001 {prop}"));
            }
        }
        "DR-101" => {
            draw(app, "PIDINSERT gate_valve", &[[0.0, 0.0]]);
            let valve = inserts_of(app, GATE_VALVE)[0];
            retag(app, valve, "");
        }
        "DR-102" => {
            draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
            other_drawing(scratch, |other| {
                draw(other, "PIDINSERT pump", &[[40.0, 60.0]]);
            });
        }
        // 协同服务器配过对，对端图 PID-1102 在项目里，上面却没有回来的连接符。
        "DR-116" => {
            pipe_line(app);
            draw(app, "PIDCONNECTOR pipe_out", &[[200.0, 0.0]]);
            edit_topology(app, |table| {
                let plug = &mut table.connectors[0];
                plug.upstream = Some(plug.line_tag.clone());
                plug.peer_drawing = Some("PID-1102".to_owned());
                plug.downstream = Some("L-2001".to_owned());
            });
            other_drawing(scratch, pipe_line);
        }
        _ => panic!("{rule} 还没有造违规的办法"),
    }
}

/// PUB-7 验收第 1 条：错误级内置规则（本期不出结果的 DR-020、DR-109 除外）每条造一张只犯它的图，
/// 规则包里错误级的只留它；逐张发布都是 `PUBLISH_BLOCKED`，检查报告里有这条规则没豁免的错误，项目
/// 目录里什么也没多。18 张拦下 18 张。
#[test]
fn each_error_rule_blocks_the_drawing_that_breaks_it() {
    let rules: Vec<String> = builtin_rules::builtin()
        .rules
        .into_iter()
        .filter(|rule| rule.severity == Severity::Error && !SILENT.contains(&rule.id.as_str()))
        .map(|rule| rule.id)
        .collect();
    assert_eq!(rules.len(), 18, "{rules:?}");
    let mut missed = Vec::new();
    for rule in &rules {
        let scratch = Scratch::new();
        rule_pack(&scratch, Some(rule.as_str()));
        let mut app = fresh();
        violate(rule, &mut app, &scratch);
        save(&mut app, &scratch.path("PID-1101.dwg"));
        let before = scratch.listing();
        let answer = settled(&mut app, &json!({"op": "pid_publish"}));
        let reported = answer["report"]["results"]
            .as_array()
            .is_some_and(|results| {
                results.iter().any(|result| {
                    result["rule_id"] == rule.as_str()
                        && result["severity"] == "错误"
                        && result["waived"] == false
                })
            });
        if answer["code"] != PUBLISH_BLOCKED || !reported || scratch.listing() != before {
            missed.push(format!("{rule}：{answer}"));
        }
    }
    let blocked = rules.len() - missed.len();
    println!("PUB-7 blocked {blocked} / {}", rules.len());
    assert!(
        missed.is_empty(),
        "拦下 {blocked} / {} 张，没拦对的：\n{}",
        rules.len(),
        missed.join("\n")
    );
}

/// PUB-7 验收第 2 条：规则包把错误级的内置规则全停了，DR-004、DR-102 也在里面。本图两台泵同位号，
/// 或者另一张图上有同类型同位号的泵，发布照样是 `PUBLISH_BLOCKED`：检查没有错误，拦它的是门禁里不经
/// 规则的重复位号，各处都列出来；什么也不写。
#[test]
fn duplicate_tags_block_with_the_duplicate_rules_off() {
    for (rule, drawings) in [
        ("DR-004", ["PID-1101", "PID-1101"]),
        ("DR-102", ["PID-1101", "PID-1102"]),
    ] {
        let scratch = Scratch::new();
        let off = rule_pack(&scratch, None);
        assert!(off.iter().any(|id| id == rule), "{off:?}");
        let mut app = fresh();
        violate(rule, &mut app, &scratch);
        save(&mut app, &scratch.path("PID-1101.dwg"));
        let before = scratch.listing();
        let blocked = settled(&mut app, &json!({"op": "pid_publish"}));
        assert_eq!(blocked["code"], PUBLISH_BLOCKED, "{rule}：{blocked}");
        assert_eq!(blocked["gates"]["drc_errors"], 0, "{rule}：{blocked}");
        assert_eq!(blocked["gates"]["duplicate_tags"], 1, "{rule}：{blocked}");
        let places: Vec<&str> = blocked["duplicates"][0]["places"]
            .as_array()
            .expect("重复位号的各处")
            .iter()
            .filter_map(|place| place["drawing_id"].as_str())
            .collect();
        assert_eq!(places, drawings, "{rule}：{blocked}");
        assert_eq!(scratch.listing(), before, "{rule}：拦下了还写了东西");
    }
}

/// 和项目索引往返测试同一张小图：一台泵左边接一个不在线上的阀门，一台容器；管线 L-1001 自己一根线，
/// 线头上一个流出连接符；管路 L-1001-01 从泵的出口到容器的入口，上面一个阀门、一个不单独编号的管件、
/// 一个打断符；信号线两根线接到一台仪表的入口；还有一根挂了别的类型的线。
fn small_drawing(app: &mut OpenCADStudio) {
    draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
    draw(app, "PIDINSERT gate_valve", &[[-28.0, 0.0]]);
    draw(app, "PIDINSERT vessel", &[[200.0, 12.0]]);
    draw(app, "PIDPIPE", &[[200.0, 100.0], [300.0, 100.0]]);
    draw(app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
    draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
    draw(app, "PIDFITTING transmitter", &[[40.0, 0.0]]);
    draw(app, "PIDBREAK", &[[150.0, 0.0]]);
    draw(
        app,
        "PIDPIPE signal",
        &[[0.0, 50.0], [50.0, 50.0], [50.0, 80.0]],
    );
    draw(app, "PIDINSERT transmitter", &[[58.0, 80.0]]);
    draw(app, "PIDCONNECTOR pipe_out", &[[300.0, 100.0]]);
    run(app, "PIDSETPROP L-1001 管道等级 A1A");
    run(app, "PIDSETPROP L-1001-01 公称直径 50");
    let pump = record(app, inserts_of(app, PUMP)[0]).tag;
    run(app, &format!("PIDSETPROP {pump} 设计温度 80"));
    draw(app, "LINE", &[[400.0, 0.0], [450.0, 0.0]]);
    let doc = app.active_document_for_test();
    let note = doc
        .entities()
        .filter(|entity| matches!(entity, document::EntityType::Line(_)))
        .map(|entity| entity.common().handle)
        .filter(|handle| PidAuthor::load(doc, *handle).is_none())
        .max_by_key(|handle| handle.value())
        .expect("刚画的线");
    let data: Vec<Value> = [
        "type=NOTE",
        "tag=N-1",
        "tag_source=manual",
        "tag_status=ok",
        r#"props={"说明":"图例"}"#,
    ]
    .iter()
    .map(|value| json!({"code": 1000, "value": value}))
    .collect();
    let handles = [handle_text(note)];
    let set = request(
        app,
        json!({"op": "xdata_set", "handles": handles, "app": "PID_AUTHOR", "data": data}),
    );
    assert_eq!(set["ok"], true, "{set}");
}

fn summary_of(doc: &document::Document, symbols: &SymbolLibrary) -> DrawingSummary {
    let meta = SummaryMeta {
        drawing_id: "PID-1101".to_owned(),
        file: Some("PID-1101.dwg".to_owned()),
        pbs: None,
        source: SummarySource::Open,
        fingerprint: None,
    };
    summarize(doc, symbols, meta, DEFAULT_TOLERANCE)
}

/// 往返允许不同的三项抹掉：`source`、`fingerprint` 和各处的句柄。
fn comparable(mut summary: DrawingSummary) -> DrawingSummary {
    summary.source = SummarySource::File;
    summary.fingerprint = None;
    for object in &mut summary.objects {
        object.handle.clear();
    }
    for segment in &mut summary.segments {
        segment.handle.clear();
    }
    for connector in &mut summary.connectors {
        connector.handle.clear();
    }
    summary
}

/// PUB-7 验收第 3 条：包里的图纸是从磁盘复制的，清单和报告是从程序里的图算的。一张什么都有的小图
/// 经存盘操作存下，从文件读回来，摘要与存之前相同（`source`、`fingerprint` 和各处的句柄除外）；存完
/// 程序里的图也还是这份摘要。
#[test]
fn a_drawing_saved_by_the_save_op_reads_back_with_the_same_summary() {
    let scratch = Scratch::new();
    let mut app = fresh();
    small_drawing(&mut app);
    let before = comparable(summary_of(app.active_document_for_test(), &app.pid_symbols));
    assert!(
        !before.objects.is_empty() && !before.segments.is_empty() && !before.connectors.is_empty(),
        "{before:?}"
    );
    let path = scratch.path("PID-1101.dwg");
    save(&mut app, &path);
    let kept = comparable(summary_of(app.active_document_for_test(), &app.pid_symbols));
    assert_eq!(kept, before, "存完程序里的图");
    let back = crate::io::load_file(&path).unwrap_or_else(|error| panic!("读回：{error}"));
    let read = comparable(summary_of(&back, &app.pid_symbols));
    assert_eq!(read, before, "读回的图");
}
