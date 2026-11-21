//! 设计规则检查（`SPEC-pid-drc.md`）。
//!
//! 现在有规则表达式（[`dsl`]：解析与三值求值）、§3.4 的名字表、规则包和检查结果的数据
//! （[`model`]）、内置包与项目包的读取和叠加（[`builtin_rules`]），以及检查器（[`runner`]）
//! 和它在摘要上取的事实（[`facts`]）。豁免记在图里（[`waivers`]、[`set_waivers`]），检查时按规则号
//! 和对象键对上结果。保存门禁（§4.3）查当前图、只跑单图规则：`block` 档在保存函数里拦
//! （[`gate_save`]），`warn` 档在保存成功后给一句汇总（[`save_summary`]）。

pub mod builtin_rules;
pub mod dsl;
pub mod facts;
pub mod model;
pub mod runner;

use std::path::Path;

use document::Document;
use serde_json::{json, Value};

use crate::pid::graph_data;
use crate::pid::master_data;
use crate::pid::persist::PID_AUTHOR_APP;
use crate::pid::project_index::{self, summarize, SummaryMeta, SummarySource};
use crate::pid::propagate::PidError;
use crate::pid::symbols::SymbolLibrary;
use crate::pid::type_lib::TypeLib;

pub use dsl::{parse, DslError, Expr, Fact, Facts, Truth, DSL_PARSE_ERROR};
pub use model::{
    Category, CheckReport, CheckResult, DrcError, Kind, Notice, ObjectRef, Rule, RuleGroup,
    RuleScope, RuleSet, SaveGate, Scope, ScopeKind, Severity, Source, Summary, Target, Trigger,
    Waiver, WaiverError, REPORT_VERSION, WAIVER_REASON_LIMIT,
};
pub use runner::{check, CheckInput};

/// 图级记录的名字：这张图上的豁免（§3.7）。
pub const PID_AUTHOR_DRC_WAIVERS: &str = "PID_AUTHOR_DRC_WAIVERS";

/// `block` 档拒绝保存时，消息里列出前几条错误（§4.3）。
const BLOCKED_LISTED: usize = 5;

/// 图里记着的豁免。读不出来的项跳过；理由在写的时候查（[`Waiver::new`]），这里不再查。
pub fn waivers(doc: &Document) -> Vec<Waiver> {
    graph_data::load(doc, PID_AUTHOR_DRC_WAIVERS)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect()
}

/// 把图里的豁免整条改成 `waivers`，空表删掉记录。撤销快照由调用方压。
pub fn set_waivers(doc: &mut Document, waivers: &[Waiver]) {
    let items: Vec<Value> = waivers
        .iter()
        .map(|waiver| {
            json!({
                "rule_id": waiver.rule_id,
                "object_key": waiver.object_key,
                "reason": waiver.reason,
                "by": waiver.by,
                "at": waiver.at,
            })
        })
        .collect();
    graph_data::store(doc, PID_AUTHOR_DRC_WAIVERS, &items);
}

/// 图里有没有 P&ID 记录。保存门禁先看这个：没有就放行，不读规则包也不做摘要（§4.3）。
pub fn has_pid_objects(doc: &Document) -> bool {
    doc.entities().any(|entity| {
        entity
            .common()
            .extended_data
            .get_record(PID_AUTHOR_APP)
            .is_some()
    })
}

/// 图纸文件 `path`（可以还不存在）在检查结果里的图号和 PBS 节点：按它所在目录的项目文件取
/// （[`project_index::drawing_of`]），项目文件读不出来时图号用文件名；没有 `path` 时图号是空的。
pub fn drawing_of(path: Option<&Path>) -> (String, Option<String>) {
    let dir = path.and_then(Path::parent);
    let file = path
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy());
    match (dir, file.as_deref()) {
        (Some(dir), Some(file)) => project_index::drawing_of(dir, file).unwrap_or_else(|_| {
            let stem = Path::new(file).file_stem().unwrap_or_default();
            (stem.to_string_lossy().into_owned(), None)
        }),
        _ => (String::new(), None),
    }
}

/// 查 `doc` 这一张图，只跑单图规则。`path` 是这张图的文件（可以还不存在）：主数据取它所在的
/// 目录，图号和 PBS 节点见 [`drawing_of`]。
pub fn check_drawing(
    doc: &Document,
    path: Option<&Path>,
    rules: &RuleSet,
    trigger: Trigger,
) -> CheckReport {
    let dir = path.and_then(Path::parent);
    let file = path
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    let (drawing_id, pbs) = drawing_of(path);
    let master = master_data::resolve(doc, dir);
    let meta = SummaryMeta {
        drawing_id: drawing_id.clone(),
        file,
        pbs,
        source: SummarySource::Open,
        fingerprint: None,
    };
    let summary = summarize(
        doc,
        &SymbolLibrary::load_from_env(),
        meta,
        rules.connect_tolerance,
    );
    let types = TypeLib::embedded();
    let recorded = [(drawing_id.clone(), waivers(doc))];
    check(&CheckInput {
        rules,
        master: &master,
        types: &types,
        drawings: std::slice::from_ref(&summary),
        index: None,
        others: &[],
        waivers: &recorded,
        trigger,
        scope: Scope {
            kind: ScopeKind::Drawing,
            drawing_id: Some(drawing_id),
            pbs: None,
            cross_drawing: false,
        },
    })
}

/// `block` 档的保存门禁（§4.3），保存函数在写盘之前调。`path` 是要写的文件，规则包取它所在的
/// 目录；没有路径时（网页版存成字节）只看内置包。
///
/// 图里没有 P&ID 记录、`save_gate` 不是 `block`，或者项目包读不出来（不知道是哪一档，由保存后的
/// [`save_summary`] 报）时放行；否则查这张图，有未豁免的错误就拒绝，消息列出前 5 条。
pub fn gate_save(doc: &Document, path: Option<&Path>) -> Result<(), PidError> {
    if !has_pid_objects(doc) {
        return Ok(());
    }
    let Ok(rules) = builtin_rules::load(path.and_then(Path::parent)) else {
        return Ok(());
    };
    if rules.save_gate != SaveGate::Block {
        return Ok(());
    }
    let report = check_drawing(doc, path, &rules, Trigger::BeforeSave);
    let errors: Vec<&CheckResult> = report
        .results
        .iter()
        .filter(|result| result.severity == Severity::Error && !result.waived)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    Err(PidError::RuleBlocked {
        errors: errors.len(),
        first: errors
            .iter()
            .take(BLOCKED_LISTED)
            .map(|result| format!("{} {}", result.rule_id, result.message))
            .collect(),
    })
}

/// `warn` 档保存成功后命令行上的那一句（§4.3）：错误、警告、提示各几条，有豁免的再说豁免了
/// 几条。`path` 是刚写好的文件。图里没有 P&ID 记录或 `save_gate` 不是 `warn` 时没有这一句；
/// 项目包读不出来时说检查没跑和原因。
pub fn save_summary(doc: &Document, path: &Path) -> Option<String> {
    if !has_pid_objects(doc) {
        return None;
    }
    let rules = match builtin_rules::load(path.parent()) {
        Ok(rules) => rules,
        Err(error) => return Some(format!("规则检查没跑：{error}")),
    };
    if rules.save_gate != SaveGate::Warn {
        return None;
    }
    let counts = check_drawing(doc, Some(path), &rules, Trigger::BeforeSave).summary;
    let waived = if counts.waived > 0 {
        format!("，已豁免 {} 条", counts.waived)
    } else {
        String::new()
    };
    Some(format!(
        "规则检查：错误 {} 条，警告 {} 条，提示 {} 条{waived}",
        counts.errors, counts.warnings, counts.infos
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use document::DxfVersion;

    use super::*;
    use crate::pid::master_data::PROJECT_FILE;
    use crate::pid::project_index::synthetic_drawing;

    fn waiver(rule_id: &str, object_key: &str, reason: &str) -> Waiver {
        Waiver {
            rule_id: rule_id.to_owned(),
            object_key: object_key.to_owned(),
            reason: reason.to_owned(),
            by: "zhangsan".to_owned(),
            at: 1_794_800_000,
        }
    }

    /// 豁免记录写进图、存成 DWG 再打开，逐项相等（§3.7）。整条重写，空表不留记录；
    /// 读不出来的项跳过。这条记录不在四个图级 key 里（不进协同报文）。
    #[test]
    fn waivers_survive_a_dwg_save_and_reopen() {
        let mut doc = Document::new();
        assert!(waivers(&doc).is_empty());
        let written = vec![
            waiver("DR-014", "PIPE_LINE|L-1003", "放空管，端部通大气"),
            waiver("DR-008", "PUMP|P-101#N2", &"备用管口".repeat(50)),
            waiver("DR-113", "仪表回路|F-101", "控制器在 DCS 里"),
        ];
        set_waivers(&mut doc, &written);
        assert_eq!(waivers(&doc), written);
        let bytes =
            crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032).expect("save dwg 2018");
        let back = crate::io::load_bytes("waivers.dwg", bytes).expect("load dwg 2018");
        assert_eq!(waivers(&back), written);
        assert!(!graph_data::keys().contains(&PID_AUTHOR_DRC_WAIVERS));

        set_waivers(&mut doc, &written[1..]);
        assert_eq!(waivers(&doc), written[1..]);
        set_waivers(&mut doc, &[]);
        assert!(waivers(&doc).is_empty());
        assert_eq!(
            graph_data::load(&doc, PID_AUTHOR_DRC_WAIVERS),
            None,
            "空表删掉记录"
        );

        graph_data::store(
            &mut doc,
            PID_AUTHOR_DRC_WAIVERS,
            &[
                json!("不是对象"),
                json!({ "rule_id": "DR-001", "object_key": "PIPE_LINE|L-1003" }),
                json!({
                    "rule_id": "DR-014", "object_key": "PIPE_LINE|L-1003",
                    "reason": "放空管，端部通大气", "by": "zhangsan", "at": 1_794_800_000
                }),
            ],
        );
        assert_eq!(waivers(&doc), written[..1]);
    }

    /// 测试用的空目录，用完删掉。
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ocs-drc-save-gate-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// 放一份只写了 `save_gate` 的项目规则包。
        fn gate(&self, gate: &str) {
            let pack = json!({
                "version": 1, "pack": "保存门禁测试", "rule_set_version": "T1", "save_gate": gate
            });
            self.write(builtin_rules::PROJECT_PACK_FILE, &pack.to_string());
        }

        fn write(&self, file: &str, text: &str) {
            std::fs::write(self.0.join(file), text).unwrap();
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 存在 `path` 时这张图上没被豁免的错误。
    fn unwaived_errors(doc: &Document, path: &Path) -> Vec<CheckResult> {
        let rules = builtin_rules::load(path.parent()).expect("合法的规则包");
        check_drawing(doc, Some(path), &rules, Trigger::BeforeSave)
            .results
            .into_iter()
            .filter(|result| result.severity == Severity::Error && !result.waived)
            .collect()
    }

    fn listed(errors: &[CheckResult]) -> Vec<String> {
        errors
            .iter()
            .take(5)
            .map(|result| format!("{} {}", result.rule_id, result.message))
            .collect()
    }

    fn waive(errors: &[CheckResult]) -> Vec<Waiver> {
        errors
            .iter()
            .map(|result| {
                Waiver::new(
                    &result.rule_id,
                    &result.object_key,
                    "按设计如此",
                    "zhangsan",
                    1_794_800_000,
                )
                .expect("合法的豁免")
            })
            .collect()
    }

    /// 只有 `block` 档拦：有未豁免的错误就拒绝，码是 `RULE_BLOCKED`，消息列出前 5 条
    /// （§4.3）。没有项目包（内置包是 `warn`）、`warn`、`off` 和没有路径时都放行。
    #[test]
    fn save_gate_blocks_unwaived_errors_only_in_block_mode() {
        let dir = Dir::new("modes");
        let path = dir.0.join("PID-1101.dwg");
        let doc = synthetic_drawing(20);
        let errors = unwaived_errors(&doc, &path);
        assert!(
            errors
                .iter()
                .any(|result| result.rule_id == "DR-009" && result.object_key == "图纸|PID-1101"),
            "没配主数据，图纸该缺图例标准：{errors:?}"
        );

        assert_eq!(gate_save(&doc, Some(&path)), Ok(()), "内置包是 warn");
        for gate in ["off", "warn"] {
            dir.gate(gate);
            assert_eq!(gate_save(&doc, Some(&path)), Ok(()), "{gate}");
        }
        dir.gate("block");
        assert_eq!(gate_save(&doc, None), Ok(()), "没有路径只看内置包");
        let refused = gate_save(&doc, Some(&path)).expect_err("block 档有错误要拒绝");
        assert_eq!(refused.code(), "RULE_BLOCKED");
        assert_eq!(
            refused,
            PidError::RuleBlocked {
                errors: errors.len(),
                first: listed(&errors),
            }
        );
        let message = refused.to_string();
        assert!(message.starts_with("RULE_BLOCKED: "), "{message}");
        assert!(
            listed(&errors)
                .iter()
                .all(|line| message.contains(line.as_str())),
            "{message}"
        );
    }

    /// 错误都豁免了就放行（§4.3「改完或豁免完能存」）；还剩一条就只报那一条。
    #[test]
    fn save_gate_lets_a_drawing_through_once_its_errors_are_waived() {
        let dir = Dir::new("waived");
        dir.gate("block");
        let path = dir.0.join("PID-1101.dwg");
        let mut doc = synthetic_drawing(20);
        let errors = unwaived_errors(&doc, &path);
        assert!(!errors.is_empty());

        set_waivers(&mut doc, &waive(&errors[1..]));
        assert_eq!(
            gate_save(&doc, Some(&path)),
            Err(PidError::RuleBlocked {
                errors: 1,
                first: listed(&errors[..1]),
            })
        );
        set_waivers(&mut doc, &waive(&errors));
        assert_eq!(gate_save(&doc, Some(&path)), Ok(()));
    }

    /// 图号按项目文件取，与项目索引一致：图纸级结果的对象键、豁免都跟着它。项目文件没写这个
    /// 文件时用文件名。
    #[test]
    fn save_gate_names_the_drawing_the_way_the_project_index_does() {
        let dir = Dir::new("named");
        let project = json!({
            "version": 1,
            "drawings": [{ "file": "pid-1101.dwg", "drawing_id": "P&ID-1101", "pbs": null }]
        });
        dir.write(PROJECT_FILE, &project.to_string());
        let doc = synthetic_drawing(20);
        let keys = |file: &str| -> Vec<String> {
            unwaived_errors(&doc, &dir.0.join(file))
                .into_iter()
                .filter(|result| result.rule_id == "DR-009")
                .map(|result| result.object_key)
                .collect()
        };
        assert_eq!(keys("PID-1101.dwg"), ["图纸|P&ID-1101"]);
        assert_eq!(keys("PID-1102.dwg"), ["图纸|PID-1102"]);
    }

    /// 图里没有 P&ID 记录时三档都不查、不出汇总，连规则包都不读：坏了的项目包也不报（§4.3）。
    #[test]
    fn save_gate_leaves_drawings_without_pid_objects_alone() {
        let dir = Dir::new("plain");
        let path = dir.0.join("plain.dwg");
        let doc = Document::new();
        assert!(!has_pid_objects(&doc));
        assert!(has_pid_objects(&synthetic_drawing(20)));
        for gate in ["off", "warn", "block"] {
            dir.gate(gate);
            assert_eq!(gate_save(&doc, Some(&path)), Ok(()), "{gate}");
            assert_eq!(save_summary(&doc, &path), None, "{gate}");
        }
        dir.write(builtin_rules::PROJECT_PACK_FILE, "{ 坏了");
        assert_eq!(gate_save(&doc, Some(&path)), Ok(()));
        assert_eq!(save_summary(&doc, &path), None);
    }

    /// `warn` 档保存后的一句汇总：错误、警告、提示各几条，有豁免再说豁免了几条；`block`、
    /// `off` 不出。项目包坏了不拦保存，汇总里说检查没跑和原因（§4.3）。
    #[test]
    fn save_gate_summary_counts_the_results_in_warn_mode_only() {
        let dir = Dir::new("summary");
        let path = dir.0.join("PID-1101.dwg");
        let mut doc = synthetic_drawing(20);
        let rules = builtin_rules::load(Some(&dir.0)).expect("内置包");
        let counts = check_drawing(&doc, Some(&path), &rules, Trigger::BeforeSave).summary;
        let line = format!(
            "规则检查：错误 {} 条，警告 {} 条，提示 {} 条",
            counts.errors, counts.warnings, counts.infos
        );
        assert_eq!(
            save_summary(&doc, &path),
            Some(line.clone()),
            "内置包是 warn"
        );
        dir.gate("warn");
        assert_eq!(save_summary(&doc, &path), Some(line));
        for gate in ["off", "block"] {
            dir.gate(gate);
            assert_eq!(save_summary(&doc, &path), None, "{gate}");
        }

        dir.gate("warn");
        let errors = unwaived_errors(&doc, &path);
        set_waivers(&mut doc, &waive(&errors[..1]));
        assert_eq!(
            save_summary(&doc, &path),
            Some(format!(
                "规则检查：错误 {} 条，警告 {} 条，提示 {} 条，已豁免 1 条",
                counts.errors - 1,
                counts.warnings,
                counts.infos
            ))
        );

        dir.write(builtin_rules::PROJECT_PACK_FILE, "{ 坏了");
        assert_eq!(gate_save(&doc, Some(&path)), Ok(()), "项目包坏了不拦保存");
        let broken = save_summary(&doc, &path).expect("说检查没跑");
        assert!(broken.starts_with("规则检查没跑："), "{broken}");
        assert!(broken.contains("RULESET_SCHEMA_ERROR"), "{broken}");
    }

    /// 保存函数里的闸只拦人发起的保存：不过规则闸时（自动保存）照写，过闸时拒绝，目标文件
    /// 不出现（§4.3）。
    #[test]
    fn save_gate_in_the_save_function_skips_autosave_copies() {
        let dir = Dir::new("autosave");
        dir.gate("block");
        let doc = synthetic_drawing(20);
        let save = |path: &Path, rule_gate: bool| {
            crate::io::save_owned_as_version_atomic(
                doc.clone(),
                path,
                DxfVersion::AC1032,
                false,
                None,
                None,
                false,
                false,
                None,
                None,
                rule_gate,
            )
        };
        let copy = dir.0.join("PID-1101.sv$");
        save(&copy, false).expect("自动保存不过规则闸");
        assert!(copy.exists());
        let target = dir.0.join("PID-1101.dwg");
        let refused = save(&target, true).expect_err("人发起的保存要拦");
        assert_eq!(refused.code, Some("RULE_BLOCKED"), "{refused}");
        assert!(!target.exists(), "拒绝时不写文件");
    }
}
