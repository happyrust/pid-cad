# `load_pid` 把导入摘要交出来，不再经全局 `Mutex` 按路径回传 · 小计划（2026-09-22 开单、同日批准；Q1–Q3 已落地，Q4 / Q5 待做）

> 承接 pid-parse `docs/analysis/2026-09-21-parsing-pipeline-audit.md` ⑤（并顺手收 ③ 符号库路径、④ 460 行一口气、⑥ 单位判定无声）。
> 审核当日点了名（「OCS `docs/plans/2026-09-21-load-pid-returns-its-summary.md`（⑤）」）但没写出来；**2026-09-22 用户点选「补写 OCS ⑤ 单」→ 本单**。
> 同一审核开出的 S 单（`style_link` 改吃 `PidDocument`）已于 09-22 两仓落地（pid-parse `74bee65` / OCS `bbdc3d80`），G 单（`ParseProfile::Geometry`）已批、G1 已量。
> **2026-09-22 用户「批准 ⑤ 单九条决策并开工 Q1–Q3」→ 九条按推荐放行；Q1–Q3 已落地（OCS `edc6b495`，见「进度」）。**
> 落地时 **Q-D2 的载体改了**：不是 XRecord，而是文档自定义属性（`summary_info.custom_properties`，`PID_IMPORT_SUMMARY.<字段>`）——
> XRecord 要花一个句柄、分配器不退，取走后 `$HANDSEED` 与导入后所有对象句柄整体 +1，四图 `--export` 对不上字节；属性不花句柄，取走后文档一字不差。
> 决策的本意（随文档穿通用管线、开图完成时取走、不落盘）不变。Q4 拆段、Q5 台账待做。

## 一句话

`load_pid` 算完的 `ImportSummary` 今天塞进一个 **进程级 `static Mutex<BTreeMap<PathBuf, ImportSummary>>`**（`IMPORT_SUMMARIES`），
由打开完成回调按路径 `take_import_summary(&path)` 取走：同一路径并发打开会串、回调没触发就漏一条、`take_` 语义要求恰取一次，
测试为此自备一把 `SUMMARY_MAILBOX` 锁串行化。本单让 **`load_pid` 直接返回摘要**，摘要**随文档本身**穿过通用打开管线
（与 `PID_VIEW_FILTER` 同一条路：模型空间块记录扩展字典里的一条 XRecord），打开完成时从文档上**取走**，不落盘；
顺手把 460 行的 `load_pid` 切成四段，把命中的符号库路径与单位判定写进摘要。

## 事实（2026-09-22，OCS `bbdc3d80`）

| 项 | 数 | 出处 |
|---|---|---|
| 摘要的产地 | `load_pid`（`src/io/pid.rs:460–879`，**约 420 行**一个函数）末尾 `IMPORT_SUMMARIES.lock().insert(path, ImportSummary{…})`；`ImportSummary` 17 个字段（`drawn` / `decoded` / `missing` / `style_tables_failed` / 图层五项 / 驱动尺寸三项 / 本体三项） | `pid.rs:370–419`, `445–454`, `839–876` |
| 摘要的销地 | 打开完成回调 `on_file_opened`（`src/app/update/file.rs:1823`）`take_import_summary(&path)` → 命令行三行（实体 / 图层 / 参数化符号） | `file.rs:1817–1850` |
| 为什么是 Mutex | `load_pid` 的唯一调用方 `io/mod.rs:1245 read_pid_path` 要返回 **`acadrust::ReadOutcome`**（外部类型，「no room for extra freight」）；此后 `read_file_attempt` → `load_file_for_open` → `open_path_with_phase` → `Message::FileOpened(open_id, Ok((name, path, CadDocument, DerivedCaches)))` → `on_file_opened`，一路是各格式通用的管线 | `io/mod.rs:1064`, `875–884`, `887–`; `app/update/mod.rs:898–900`, `966–971`; `app/mod.rs:2151` |
| 已有的「随文档走」先例 | `PidViewFilter::store / load`：模型空间块记录扩展字典里键 `PID_VIEW_FILTER` 的 XRecord，字符串条目；`load_pid` 末尾 `filter.store(&mut doc)`，开图 / 撤销都靠它 | `pid_view_filter.rs:44–140`, `423–445`; `pid.rs:821–823` |
| 三处按路径取摘要的测试 | `tests/pid_import.rs:254 / 329 / 3369`，各持 `SUMMARY_MAILBOX`（`:82`）串行——它存在只因为邮箱按路径键、测试并行 | `tests/pid_import.rs` |
| 符号库命中路径 | `discover_symbol_library(path) -> Option<SymbolLibrary>`（env `PID_SYMBOL_LIBRARY`，否则从图纸目录向上 5 层找 `.sym`）；`SymbolLibrary::roots() -> &[PathBuf]` 已有；摘要里只有 `cache_bodies / library_bodies` 两个数，**没有路径**——同一 `.pid` 挪个目录可能画得不一样而摘要说不出为什么（审核 ③） | `pid.rs:2937`；pid-parse `symbol_library.rs:679–712` |
| 单位判定 | `mm_per_source_unit`（`pid.rs:960`）取第一个 Decoded 实体的 `units`，只认 `m` / `mm`，其他一律回退到米并 **`log::info!`** 一条；摘要不带。英制工程整体差 25.4 倍时只有一条 info（审核 ⑥） | `pid.rs:957–991` |
| `load_pid` 的关切 | 解析 → 备文档（线型 / `lineweight_display` / 图层表）→ 符号库 → 五张样式索引（09-22 起 `*_for_document`）→ 虚线线型 / 字体 → 语义索引 / APPID → **实体循环**（fill / symbology / style / build / bounds / 字高 / 拆行 / 图层分布 / 语义 / 量尺 / apply / role / XDATA / 槽 / hidden）→ 统计 → `report_import` → 页框 / 取景 → 视图过滤 → 摘要 | `pid.rs:460–879` |

## 决策（2026-09-22 用户批准，九条按推荐执行；Q-D2 落地时改载体，见状态列）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| Q-D1 | `load_pid` 的返回值 | **`Result<PidImport, String>`，`PidImport { document: CadDocument, summary: ImportSummary }`**。调用方要文档的 `.document`、要摘要的 `.summary`；测试直接读，`SUMMARY_MAILBOX` 删。备选「返回二元组」：同义，但具名结构能长（G 单后要带 profile、⑥ 要带单位） | ✅ `edc6b495` |
| Q-D2 | 摘要怎么穿过通用管线 | **随文档**：`ImportSummary::store(&mut doc)` 写成模型空间块记录扩展字典里键 **`PID_IMPORT_SUMMARY`** 的 XRecord（`key=value` 字符串条目，与 `PID_SEMANTICS` / `PID_VIEW_FILTER` 同一套写法），`read_pid_path` 在包进 `ReadOutcome` 前写；`on_file_opened` 用 **`ImportSummary::take(&mut self.tabs[i].scene.document)`** 读出并**删掉**那条 XRecord。备选「把 `ReadOutcome` 包进 OCS 自己的 `LoadedDrawing { outcome, import_summary }` 一路传到 `Message::FileOpened`」：类型上更诚实，但为一个 `.pid` 专属字段改通用打开管线四层 + 消息枚举，wasm 分支也要跟；文档已经在载 `PID_VIEW_FILTER`，同一条路多一条记录代价最小。<br>**2026-09-22 落地时改载体**：XRecord 版做出来后四图 `--export` **全部对不上字节**——`ensure_xrecord` 走 `allocate_handle`，`CadDocument::next_handle` 是私有计数器、只涨不退，`take` 删掉对象后 `$HANDSEED` 与导入后每个新分配对象（`AcDbVariableDictionary` 等）的句柄仍整体 **+1**（D06 差 27 行，全是句柄）。改成 **`summary_info.custom_properties`** 里 `PID_IMPORT_SUMMARY.<字段>` 一字段一条（`symbol_library` 一根一条），不花句柄，`take` 后 `doc == 改前`（`CadDocument` 派生 `PartialEq`，连计数器都比）。acadrust `5b682ed` 的 DWG / DXF 写出器都不写自定义属性，所以「不落盘」现在是双保险；`take` 仍要做——MCP `record_api` 的 `summary_info` 集合读得到它。「随文档、开图完成时取走、不落盘」三句不变 | ✅ 载体改属性 `edc6b495` |
| Q-D3 | 摘要落不落盘 | **不落**：`take` 读即删，另存 DWG / DXF 时字典里没有它。摘要说的是「这次导入」（画了多少、丢了多少、库在哪），不是图纸的属性；`PID_VIEW_FILTER` 落盘是因为它是用户状态。备选「留作出处」：重开 DWG 会再报一遍 P&ID 导入行，误导 | ✅ |
| Q-D4 | 打开被取消 / 回调没触发 | 文档随 `Message::FileOpened` 一起被丢，摘要跟着走——**没有东西可漏**（今天邮箱里会留一条，等下次同路径导入覆盖）。`take` 找不到记录就不报（非 `.pid` 打开走的也是这条回调） | ✅ |
| Q-D5 | `IMPORT_SUMMARIES` / `take_import_summary` | **删**，不留兼容壳：`rg` 全仓只有 `file.rs:1823` 与三条测试用它 | ✅ |
| Q-D6 | 顺手 ③：符号库路径进摘要 | `ImportSummary.symbol_library: Vec<PathBuf>`（= `library.roots()`，无库为空）；命令行第一行末尾不加字（够长了），`report_import` 的「no symbol library found / looked up … at {:?}」两条日志已在说，摘要只是让测试与自动化读得到 | ✅ |
| Q-D7 | 顺手 ⑥：单位判定 | `mm_per_source_unit` 回退到米那条 `info` 升 **`warn`**；`ImportSummary.units: PidUnits { Stated(String), AssumedMetre }`（或 `unit_source: &'static str`——实现时定），命令行**只在回退时**多半句「units assumed metre」。不改判定逻辑（语料全是米）。<br>实现定为 `ImportSummary.unit: ImportUnit { Stated { unit, mm_per_unit }, AssumedMetre }`（`ImportUnit::read` 取代 `mm_per_source_unit`，`Projection::for_geometry` 吃它）；命令行那半句是独立一行 `P&ID import: the drawing states no coordinate unit; the metre was assumed.`，进 `locale_catalog` + 21 语种 | ✅ |
| Q-D8 | 顺手 ④：拆段 | `load_pid` 切成四个私有函数，**只搬不改**：`prepare_document(parsed, path) -> (CadDocument, Vec<String> sheet_layers_off, page_mm…)`（备文档 / 图层表）、`resolve_styles(parsed, path, &mut doc) -> Styles { styles, style_names, text_heights, fills, dash_linetypes, fonts, style_tables_failed }`（五索引 + 线型 / 字体注册）、`build_document_entities(…) -> Built { drawn, decoded, symbol_bodies, sheet_layer_distribution, bounds, lettering_on_fallback, parametric_placements }`（实体循环）、`finish(…) -> ImportSummary`（`report_import` / 页框 / 取景 / 过滤 / 摘要）。验收是**四图 `--export` 字节相同**——搬家不许改一个数 | ✅ 批；Q4 待做 |
| Q-D9 | 测试口径 | 三条按路径取摘要的测试改 `load_pid(&path)?.summary`，`SUMMARY_MAILBOX` 删；新增 ① `PidImport` 走 `io::load_file` 路时 `ImportSummary::take` 从文档读回与 `.summary` 逐字段相等、再 `take` 一次为 `None`；② 另存 DWG / DXF 再打开，字典里无 `PID_IMPORT_SUMMARY`；③ 回退单位那条：构造一张无单位的内存几何，摘要说 `AssumedMetre`、日志级别 warn（现有单测 `mm_per_source_unit` 若有就扩）。<br>落地口径：① 改为 `store` 进克隆再 `take` 回来逐字段相等、二次 `take` 为 `None`、**`take` 后文档与克隆前相等、两者另存 DXF 字节相同**；`io::load_file` 交出的文档不带任何 `PID_IMPORT_SUMMARY.` 属性（它自己取走交给日志）；② 两条路另存 DWG / DXF 再读回 `ImportSummary::load` 为 `None`；③ 单测构造五种内存实体组合（Inferred 无单位 + Decoded `m` / Decoded `mm` / Decoded 无单位 / 未知标签 `furlong` / ProbeOnly `mm`），级别 warn 由代码 `log::warn!` 定、未用日志捕获断言 | ✅ |

## 工作项

- ✅ **Q1 `src/io/pid.rs`**（`edc6b495`，按 Q-D1 / Q-D5 / Q-D6 / Q-D7）：`PidImport`；`load_pid -> Result<PidImport, String>`；`ImportSummary` 加 `symbol_library` / `unit`（并派生 `Debug / Clone / PartialEq`，不再 `Copy`）；`ImportUnit::read` 取代 `mm_per_source_unit`、回退升 warn；删 `IMPORT_SUMMARIES` / `take_import_summary`；`ImportSummary::store / load / take / log`（属性 `PID_IMPORT_SUMMARY.<字段>`，留在 `pid.rs`——`counts()` / `count_mut()` 两张表 + 三个小函数，不到 200 行，没到独立文件的分量）。
- ✅ **Q2 `src/io/mod.rs` + `src/app/update/file.rs`**（同一提交）：`read_pid_path` 解构 `PidImport`、`summary.store(&mut document)`；`on_file_opened` 改 `ImportSummary::take(&mut self.tabs[i].scene.document)`，回退单位时多一行；**`io::load_file` 也 `take`**——无头导出 / 块插入 / 测试走这条路，没有命令行给它显示，摘要转交 `ImportSummary::log`，文档交出去时干净。
- ✅ **Q3 `tests/pid_import.rs`**（同一提交，Q-D9）：`import_with_summary(name)` 取代三处 `SUMMARY_MAILBOX` 段；`import_without_library` 顺带断言 `symbol_library` 为空；新增 `the_summary_rides_the_document_once_and_is_never_saved`；`pid.rs` 单测新增 `the_summary_round_trips_through_its_properties_and_take_removes_them` / `the_unit_is_read_from_a_decoded_record_or_assumed_to_be_the_metre`。
- **Q4 拆段**（**第二个提交**，Q-D8）：基线 = `edc6b495` 的四图 `--export`；搬完对字节。**待做。**
- **Q5 台账**：user-guide `.pid` 一节「日志里另有一行说明…」处补一句摘要含符号库路径与单位；pid-parse 审核 ③ / ④ / ⑤ / ⑥ 四条改标已落地；本单头部写哈希。**待做**（本单头部 / 决策表 / 进度已随 Q1–Q3 更新）。

## 验收

- `rg "IMPORT_SUMMARIES|take_import_summary|SUMMARY_MAILBOX" src tests examples` 零命中。**✅ 09-22 零命中**（`docs/user-guide.md` 也零）。
- `pid_import` 全绿，条数 49 + 新增（Q-D9 ①②③）；`--lib io::pid` 不降。**✅ `pid_import` 49 → 50、`--lib io::pid` 52 → 54、`--lib i18n::` 3/3（21 语种键齐）。**
- 四图 `--export` DXF 与 `bbdc3d80` 的二进制**字节相同**（Q1–Q3 后一次、Q4 后再一次）；另存 DWG 的字典里无 `PID_IMPORT_SUMMARY`。**✅ Q1–Q3 后一次：基线取 `2e9e10f5`（G4 已证与 `bbdc3d80` 字节相同）debug 版四图，`edc6b495` 四图 SHA-256 逐一相等**（0201 188 308 B / 0202 189 053 B / D06 90 882 B / 工艺 319 578 B）；另存无记录由 `the_summary_rides_the_document_once_and_is_never_saved` 钉（DWG / DXF 各一次）。Q4 后再一次待做。
- GUI：打开 0201，命令行三行摘要与今天一字不差（单位是米，不多半句）；打开后立即另存 `.dwg` 再打开，命令行不再出 P&ID 导入行。**未验证**（会话无桌面；三行文案与 `t!` 键未动，多出的一行只在 `unit.is_assumed()` 时出，四图都 `Stated m`——由 `import_with_summary` 三处与新测试的 `unit` 断言钉）。

## 登记不做

| 项 | 理由 |
|---|---|
| 把 `ReadOutcome` 换成 OCS 自己的读出类型 | Q-D2 备选；`.pid` 一家的需求不动四层通用管线 |
| 摘要进特性面板 / 图层管理器 | 它是一次性的命令行头条；要看细节有日志与 `--export` 对数 |
| 单位判定改读更多来源（`igSmartFrame` / 模板名） | 审核 ⑥ 只要求「不无声」；判定逻辑另议，语料全是米 |
| B 系 / ANSI 页幅（审核 ⑦） | 语料没有；`infer_page_dimensions` 另开 |
| 拆 `load_pid` 成独立模块 | Q-D8 只切函数；文件 3.3k 行是否再拆等 G4 / ⑤ 都落地后看 |

## 进度

- **2026-09-22（会话 fable-5-1-47 → fable-5-1-19 接手）✅ Q1–Q3 落地，OCS `edc6b495`**（26 文件，+690 / −125：`pid.rs` / `mod.rs` / `file.rs` / `pid_import.rs` / `locale_catalog.rs` + 21 语种）。
  - 上一会话把 Q1–Q3 做到未提交（XRecord 载体），接手后先验：`cargo check --lib --tests --examples` 干净、`--lib io::pid` 54/54、`pid_import` 50/50——**但四图 `--export` 全部对不上字节**。`git diff --no-index` D06：27 行差，全是句柄——`$HANDSEED` `BD → BE`，`AcDbVariableDictionary` 及其六条 `DICTIONARYVAR` `B5…BB → B6…BC`。根因在 acadrust `5b682ed`：`ensure_xrecord → allocate_handle` 推进私有 `next_handle`，`take` 删掉对象后计数器不退，DXF 写出器 `compute_max_handle` 从 `next_handle()` 起算，所以导入后每个新对象整体 +1。`header.handle_seed` 虽公开但 `allocate_handle` 只向上取齐，退不回去。
  - **载体改成文档自定义属性**（Q-D2 状态列）：`store` 一字段一条 `(PID_IMPORT_SUMMARY.<key>, value)`、`symbol_library` 一根一条；`load` 按前缀收、无前缀条目则 `None`；`take` = `load` + `retain` 去前缀条目。`pid_view_filter.rs` 上一会话为共享 `owner` / `remove_xrecord` 做的改动随之撤回（不再需要）。单测断言 `store` 前克隆 == `take` 后（`CadDocument` 派生 `PartialEq`，含 `next_handle`），其他自定义属性（`Client=kept`）不被误删；集成测试断言 `take` 后文档与克隆前另存 DXF **字节相同**。
  - 顺手：新命令行行「P&ID import: the drawing states no coordinate unit; the metre was assumed.」进 `locale_catalog`（`common.pid-import-unit-assumed-metre`）与 21 个 `.ftl`，`--lib i18n::` 3/3。
  - **验证**：`--lib io::pid` **54/54**（+2）、`--test pid_import` **50/50**（+1）、`--lib i18n::` 3/3；`rg IMPORT_SUMMARIES|take_import_summary|SUMMARY_MAILBOX|SUMMARY_XRECORD_KEY` 在 `src` `tests` `examples` `docs/user-guide.md` 零命中；rustfmt 在 `pid.rs` / `pid_import.rs` 干净（`mod.rs` / `file.rs` 只剩 HEAD 就有的旧差）；clippy `--lib --tests` 在改动处零新增（`mod.rs:901 result_large_err` 等为 HEAD 旧告警）；**四图 `--export` 与 `2e9e10f5` 基线 SHA-256 逐一相等**（基线：`git stash` 回 HEAD 编 debug 版导出，再 `stash pop` 编本版导出）。
  - 未做：GUI 三行核对（无桌面）；Q4 拆段；Q5 user-guide 一句 + pid-parse 审核 ③④⑤⑥ 改标；两仓未 push。

## 门禁记录

- 2026-09-22：用户点选「补写 OCS ⑤ 单 …（去 IMPORT_SUMMARIES 全局 Mutex，顺手拆 load_pid 四段）」→ 本单（会话 fable-5-1-47）。九条决策等批。
- 2026-09-22：用户「批准 ⑤ 单九条决策并开工 Q1–Q3（load_pid 返回 PidImport、摘要随文档 XRecord 穿管线、删 IMPORT_SUMMARIES）」→ 九条放行（会话 fable-5-1-47 开工，fable-5-1-19 接手收尾）。Q-D2 载体由 XRecord 改属性是落地时按验收（字节相同）改的，本意不变；用户若要回 XRecord，`store / load / take` 三个函数换回去即可（~50 行），代价是接受句柄 +1、验收改口。
