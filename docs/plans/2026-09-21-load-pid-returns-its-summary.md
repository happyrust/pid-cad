# `load_pid` 把导入摘要交出来，不再经全局 `Mutex` 按路径回传 · 小计划（2026-09-22 开单，等批）

> 承接 pid-parse `docs/analysis/2026-09-21-parsing-pipeline-audit.md` ⑤（并顺手收 ③ 符号库路径、④ 460 行一口气、⑥ 单位判定无声）。
> 审核当日点了名（「OCS `docs/plans/2026-09-21-load-pid-returns-its-summary.md`（⑤）」）但没写出来；**2026-09-22 用户点选「补写 OCS ⑤ 单」→ 本单**。
> 同一审核开出的 S 单（`style_link` 改吃 `PidDocument`）已于 09-22 两仓落地（pid-parse `74bee65` / OCS `bbdc3d80`），G 单（`ParseProfile::Geometry`）已批、G1 已量。
> **只开单，未开工；带 ⭕ 的决策按推荐落笔，等批。**

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

## 决策（按推荐落笔，等批）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| Q-D1 | `load_pid` 的返回值 | **`Result<PidImport, String>`，`PidImport { document: CadDocument, summary: ImportSummary }`**。调用方要文档的 `.document`、要摘要的 `.summary`；测试直接读，`SUMMARY_MAILBOX` 删。备选「返回二元组」：同义，但具名结构能长（G 单后要带 profile、⑥ 要带单位） | ⭕ |
| Q-D2 | 摘要怎么穿过通用管线 | **随文档**：`ImportSummary::store(&mut doc)` 写成模型空间块记录扩展字典里键 **`PID_IMPORT_SUMMARY`** 的 XRecord（`key=value` 字符串条目，与 `PID_SEMANTICS` / `PID_VIEW_FILTER` 同一套写法），`read_pid_path` 在包进 `ReadOutcome` 前写；`on_file_opened` 用 **`ImportSummary::take(&mut self.tabs[i].scene.document)`** 读出并**删掉**那条 XRecord。备选「把 `ReadOutcome` 包进 OCS 自己的 `LoadedDrawing { outcome, import_summary }` 一路传到 `Message::FileOpened`」：类型上更诚实，但为一个 `.pid` 专属字段改通用打开管线四层 + 消息枚举，wasm 分支也要跟；文档已经在载 `PID_VIEW_FILTER`，同一条路多一条记录代价最小 | ⭕ |
| Q-D3 | 摘要落不落盘 | **不落**：`take` 读即删，另存 DWG / DXF 时字典里没有它。摘要说的是「这次导入」（画了多少、丢了多少、库在哪），不是图纸的属性；`PID_VIEW_FILTER` 落盘是因为它是用户状态。备选「留作出处」：重开 DWG 会再报一遍 P&ID 导入行，误导 | ⭕ |
| Q-D4 | 打开被取消 / 回调没触发 | 文档随 `Message::FileOpened` 一起被丢，摘要跟着走——**没有东西可漏**（今天邮箱里会留一条，等下次同路径导入覆盖）。`take` 找不到记录就不报（非 `.pid` 打开走的也是这条回调） | ⭕ |
| Q-D5 | `IMPORT_SUMMARIES` / `take_import_summary` | **删**，不留兼容壳：`rg` 全仓只有 `file.rs:1823` 与三条测试用它 | ⭕ |
| Q-D6 | 顺手 ③：符号库路径进摘要 | `ImportSummary.symbol_library: Vec<PathBuf>`（= `library.roots()`，无库为空）；命令行第一行末尾不加字（够长了），`report_import` 的「no symbol library found / looked up … at {:?}」两条日志已在说，摘要只是让测试与自动化读得到 | ⭕ |
| Q-D7 | 顺手 ⑥：单位判定 | `mm_per_source_unit` 回退到米那条 `info` 升 **`warn`**；`ImportSummary.units: PidUnits { Stated(String), AssumedMetre }`（或 `unit_source: &'static str`——实现时定），命令行**只在回退时**多半句「units assumed metre」。不改判定逻辑（语料全是米） | ⭕ |
| Q-D8 | 顺手 ④：拆段 | `load_pid` 切成四个私有函数，**只搬不改**：`prepare_document(parsed, path) -> (CadDocument, Vec<String> sheet_layers_off, page_mm…)`（备文档 / 图层表）、`resolve_styles(parsed, path, &mut doc) -> Styles { styles, style_names, text_heights, fills, dash_linetypes, fonts, style_tables_failed }`（五索引 + 线型 / 字体注册）、`build_document_entities(…) -> Built { drawn, decoded, symbol_bodies, sheet_layer_distribution, bounds, lettering_on_fallback, parametric_placements }`（实体循环）、`finish(…) -> ImportSummary`（`report_import` / 页框 / 取景 / 过滤 / 摘要）。验收是**四图 `--export` 字节相同**——搬家不许改一个数 | ⭕ |
| Q-D9 | 测试口径 | 三条按路径取摘要的测试改 `load_pid(&path)?.summary`，`SUMMARY_MAILBOX` 删；新增 ① `PidImport` 走 `io::load_file` 路时 `ImportSummary::take` 从文档读回与 `.summary` 逐字段相等、再 `take` 一次为 `None`；② 另存 DWG / DXF 再打开，字典里无 `PID_IMPORT_SUMMARY`；③ 回退单位那条：构造一张无单位的内存几何，摘要说 `AssumedMetre`、日志级别 warn（现有单测 `mm_per_source_unit` 若有就扩） | ⭕ |

## 工作项（批了再做）

- **Q1 `src/io/pid.rs`**（一提交，按 Q-D1 / Q-D5 / Q-D6 / Q-D7）：`PidImport`；`load_pid -> Result<PidImport, String>`；`ImportSummary` 加 `symbol_library` / `units`；`mm_per_source_unit` 回退升 warn 并把判定结果交出来；删 `IMPORT_SUMMARIES` / `take_import_summary`；`ImportSummary::store / take`（XRecord `PID_IMPORT_SUMMARY`，放 `pid.rs` 或新 `pid_import_summary.rs`——17+ 字段的 `key=value` 往返值得独立文件）。
- **Q2 `src/io/mod.rs` + `src/app/update/file.rs`**（同一提交）：`read_pid_path` 解构 `PidImport`、`summary.store(&mut document)`；`on_file_opened` 改 `ImportSummary::take(&mut self.tabs[i].scene.document)`，回退单位时多半句。
- **Q3 `tests/pid_import.rs`**（同一提交，Q-D9）。
- **Q4 拆段**（**第二个提交**，Q-D8）：先 Q1–Q3 落地、四图 `--export` 存基线，再搬；搬完对字节。
- **Q5 台账**：user-guide `.pid` 一节「日志里另有一行说明…」处补一句摘要含符号库路径与单位；pid-parse 审核 ③ / ④ / ⑤ / ⑥ 四条改标已落地；本单头部写哈希。

## 验收

- `rg "IMPORT_SUMMARIES|take_import_summary|SUMMARY_MAILBOX" src tests examples` 零命中。
- `pid_import` 全绿，条数 49 + 新增（Q-D9 ①②③）；`--lib io::pid` 不降。
- 四图 `--export` DXF 与 `bbdc3d80` 的二进制**字节相同**（Q1–Q3 后一次、Q4 后再一次）；另存 DWG 的字典里无 `PID_IMPORT_SUMMARY`。
- GUI：打开 0201，命令行三行摘要与今天一字不差（单位是米，不多半句）；打开后立即另存 `.dwg` 再打开，命令行不再出 P&ID 导入行。

## 登记不做

| 项 | 理由 |
|---|---|
| 把 `ReadOutcome` 换成 OCS 自己的读出类型 | Q-D2 备选；`.pid` 一家的需求不动四层通用管线 |
| 摘要进特性面板 / 图层管理器 | 它是一次性的命令行头条；要看细节有日志与 `--export` 对数 |
| 单位判定改读更多来源（`igSmartFrame` / 模板名） | 审核 ⑥ 只要求「不无声」；判定逻辑另议，语料全是米 |
| B 系 / ANSI 页幅（审核 ⑦） | 语料没有；`infer_page_dimensions` 另开 |
| 拆 `load_pid` 成独立模块 | Q-D8 只切函数；文件 3.3k 行是否再拆等 G4 / ⑤ 都落地后看 |

## 进度

（只开单，未开工。）

## 门禁记录

- 2026-09-22：用户点选「补写 OCS ⑤ 单 …（去 IMPORT_SUMMARIES 全局 Mutex，顺手拆 load_pid 四段）」→ 本单（会话 fable-5-1-47）。九条决策等批。
