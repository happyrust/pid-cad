# SPEC：新版导出界面（`sppid-export-ui`）

> 状态：**已确认**（2026-11-20，答复「按照推荐方案继续下一步。」，U1–U3 均按建议定；实施计划与任务见 `plan-checkpoint-c.md`、`todo-checkpoint-c.md`，界面实现待 Pencil 设计稿定稿）。检查点 B 与总验收（`ACC-1`）的两个导出引擎（`src/io/pid_backup_export.rs`、`src/io/pid_backup_pdf.rs`）与命令层（`PIDBACKUPEXPORT`、`PIDBACKUPPDF`）均已验收通过，本规格定义检查点 C 的后半部分：在新版「备份数据包」画面中不敲命令完成 publish 文件与 PDF 的导出、状态查看与结果核对。
> 依据：`00-capability-map.md` §4、§5 检查点 C、§8 第 3 问；`SPEC-sppid-export.md` §5.4、§9 X6；`SPEC-sppid-pdf.md` §9 P1–P4；`02-screen-content.md` §4；需求 `REQ-MIG-009`、`REQ-MIG-010`（`docs/pid-platform/requirements/90-migration-and-interop.md`）。
> 共用约定见 `01-shared-conventions.md`。前提：`SPEC-sppid-export.md`、`SPEC-sppid-pdf.md` 已完成，且 `SPEC-sppid-screen.md` 与 Pencil 设计稿（`sppid-design`）定稿。

## 1. 目标

在「备份数据包」画面（`backup`）上提供图形化的 **「导出 publish 文件」** 与 **「导出 PDF」** 两条完整工作流，让用户**不敲命令**就能完成：

1. 点击按钮打开导出配置对话框，用系统目录选择器选输出目录（publish 还可改工厂名、选参考目录），提交前即时校验目录前提；
2. 发起后台导出后看到非阻塞的进行中状态（publish 显示已用秒数自动跳秒；PDF 显示已完成张数 / 总张数 + 已用秒数），期间可自由切去编辑器看图或操作；
3. 导出结束后在界面上直接查看结构化结果（写出数、失败/未写出图纸名单及原因、缺符号明细、参考目录比对四项计数与逐图明细、SQL Server 未验收提示），并支持一键打开输出目录或清单文件（`_Index.json` / `_PdfIndex.json`）；
4. 支持按照「先导 publish 文件、再往同一目录导 PDF」把 `<图名>_Data.xml` 与 `<图名>.pdf` 成对输出到同一个交付目录。

三条硬规矩：

- **引擎不重写、输出字节不变。** 界面直接调用 `pid_backup_export::{check, export_in_background}` 与 `pid_backup_pdf::{check, export_in_background}`，不解析命令行英文文本；同一包同一输入写出的字节与 `PIDBACKUPEXPORT` / `PIDBACKUPPDF` 100% 相同。
- **状态与命令层单一数据源。** 界面和命令层共享 `app.pid_backup_export` 与 `app.pid_backup_pdf` 状态：从界面点导出时命令行敲不带参数的 `PIDBACKUPEXPORT` / `PIDBACKUPPDF` 能查到同一进度和结果；反过来从命令行发起导出时，切到 `backup` 画面也能看到进行中和结果；导出期间清缓存同样被拦。
- **两条导出互不阻塞。** publish 正在跑时仍可发起 PDF 导出（到不同目录；若导到同一目录则等 publish 写完再点 PDF），反之亦然。

## 2. 现状

### 2.1 已落地的引擎与命令状态

- **publish 引擎与命令状态（`src/io/pid_backup_export.rs`、`src/app/commands/pidbackupexport.rs`）**：
  - 输入：`ExportRequest { package_dir, out_dir, plant, reference_dir }`（`DEFAULT_PLANT = "P01"`）；
  - 前提检查：`check(&request)` + `in_the_restore(&out_dir, &restored)`，不写任何文件即可返回 `ExportError`（`NotABackup`、`TargetInvalid`、`TargetNotEmpty { folder, names, count }`、`ReferenceInvalid` 等）；
  - 结果：`ExportOutcome`（`written`、`failed`、`pid_entries`、`templates`、`unmatched`、`without_drawing_no`、`unwritten`、`noted`、`other_problems`、`warnings`、`gate: Option<GateSummary>`、`sql_server()`、`files`、`index_path`、`elapsed`）；
  - 应用状态：`app.pid_backup_export`（`BackupExport`）已持有 `running: Option<Running { request, started }>`、`last: Option<Result<ExportOutcome, ExportError>>` 及可替换的测试替身函数 `export`。
- **PDF 引擎与命令状态（`src/io/pid_backup_pdf.rs`、`src/app/commands/pidbackuppdf.rs`）**：
  - 输入：`PdfRequest { package_dir, restored, out_dir, drawings }`；
  - 前提检查：`check(&request)`，只要目录里没有本次要写的同名 `.pdf` 或 `_PdfIndex.json`，允许目录里已有 publish 导出的 `_Data.xml` / `_Meta.xml` / `_Coverage.json` / `_Index.json`；
  - 进度：`PdfProgress`（原子计数 `done` / `total`）；
  - 结果：`PdfOutcome`（`pages: Vec<PdfPage>`、`written`、`failed`、`without_file`、`index_path`、`elapsed`）；
  - 应用状态：`app.pid_backup_pdf`（`BackupPdf`）已持有 `running: Option<Running { out_dir, drawings, progress }>`、`last: Option<Result<PdfOutcome, PdfError>>` 及可替换的测试替身函数 `export`。

### 2.2 尚缺的部分

1. 新界面上没有触发这两项导出的按钮、对话框、跳秒订阅（`Subscription`）和结构化结果视图。
2. `BackupExport` 与 `BackupPdf` 的字段目前为模块私有，需暴露 `pub(in crate::app)` 只读访问器与从界面直接传入结构化请求启动后台导出的入口函数；另 `BackupPdf::Running` 需补一个 `started: Instant` 字段以便界面显示 PDF 进行中的已用秒数。

## 3. 界面结构与状态

对应 `02-screen-content.md` §4，分四块：

### 3.1 包概要栏上的两个入口按钮

| 按钮 | 启用条件 | 禁用时提示 |
|---|---|---|
| **导出 publish 文件** | `BackupIndex` 已载入，包根目录含 `Manifest.txt`，且 `pid_backup_export` 不在 `Running` | 无包 /「当前目录不含备份清单 Manifest.txt，无法导出 publish 文件」/「正在导出 publish 文件…」 |
| **导出 PDF** | `BackupIndex` 已载入，至少 1 张图有 `.pid` 文件，且 `pid_backup_pdf` 不在 `Running` | 无包 /「备份包中没有可出图的 .pid 文件」/「正在导出 PDF…」 |

### 3.2 配置对话框（模态浮层，受 `chrome::shielded` 保护）

**（A）导出 publish 文件对话框（`PublishDialogState`）**
- **输出目录**：输入框 +「浏览…」文件夹选择按钮。每次改动路径即调 `in_the_restore` + `pid_backup_export::check` 做只读预检：若目录非空（`SPPID_EXPORT_TARGET_NOT_EMPTY`）或落在包/还原目录内（`SPPID_EXPORT_TARGET_INVALID`），当场在输入框下方红字显示具体原因（含前几个挡路文件名），「开始导出」置灰。
- **工厂名**：输入框，预填 `P01`；空串时提示必填。
- **参考目录（可选）**：输入框 +「浏览…」+「清空」按钮。非空时即时校验目录存在；说明文字注明「可选：与厂商参考 `_Data.xml` 比对（支持含子目录的验证集）」。
- **信息提示**：显示当前包 `.pid` 文件数；若为 SQL Server 包显示醒目提示「SQL Server 包的导出没有验收过」；底部附一句提示：「如需将 PDF 导出到同一交付目录配成对，请先导出 publish 文件，完成后再将 PDF 导出到该目录。」
- 按钮：「取消」「开始导出」。

**（B）导出 PDF 对话框（`PdfDialogState`）**
- **输出目录**：输入框 +「浏览…」按钮；若刚完成过一次 publish 导出，可提供快捷填入「使用上次 publish 输出目录」按钮方便配对。每次改动路径即调 `pid_backup_pdf::check` 做只读预检：若有同名 `.pdf` 或 `_PdfIndex.json` 已存在，当场红字列出前几个冲突文件名并置灰「开始导出」；若目录里只有 publish 文件则显示通过提示「可与该目录中的 publish 文件配对输出」。
- **信息提示**：显示总图纸数（含无文件张数）；若为 SQL Server 包同样提示「没有验收过」。
- 按钮：「取消」「开始导出」。

### 3.3 进行中状态条（U1 两处联动）

- 当 `pid_backup_export` 或 `pid_backup_pdf` 处于 `running.is_some()` 时，由 `iced::time::every(Duration::from_secs(1))` 驱动每秒刷新一次画面（仅在有导出运行且窗口处于新界面时挂订阅，平时零开销）：
  - **publish 进行中**：「正在导出 publish 文件到 `<目录>`（工厂名 `<Plant>`）· 已用 `12` 秒」（无百分比、无取消）。
  - **PDF 进行中**：「正在导出 PDF 到 `<目录>` · 第 `18 / 53` 张 · 已用 `19` 秒」（配按 `done / total` 计算的进度条，无取消）。
- **两处联动（U1 已定）**：
  1. `backup` 画面内（包概要下方导出状态区，U3）显示完整的进行中条；
  2. 外壳底部状态栏右侧（`src/app/pid_shell/chrome.rs` 的 `status_parts`）顺带显示简短全局提示（如 `正在导出 publish · 12s` / `正在导出 PDF 18/53`），在其他画面（如编辑器）点击可一键切回 `backup` 画面。绝不锁死画面切换或画布操作。

### 3.4 结果摘要条与展开详情（U3）

在包概要下方显示 publish 与 PDF 各自最近一次状态摘要条（未导出时不占地方；完成后显示摘要行 +「查看详情」「打开输出目录」「打开清单文件」按钮，点击「查看详情」展开名单与比对明细）。直接读取结构化 `Result<ExportOutcome, ExportError>` 与 `Result<PdfOutcome, PdfError>` 展示三种形态：

1. **全部成功且无提醒**：绿色完成态，展示写出张数、总文件数、耗时、输出目录，配「打开输出目录」「打开清单文件」按钮。
2. **完成但有未写出 / 提醒 / 缺符号 / 比对结论**：警示完成态：
   - publish：展示核心计数；若给了参考目录，展示比对四格统计（一致 `clean` / 有差异 `dirty` / 另一版 `revision_mismatch` / 无参考 `unpaired`）及可展开滚动的逐图比对明细 `lines`；按分栏或折叠列表展示非空的 `unwritten`、`noted`、`other_problems`、`warnings`；
   - PDF：展示 `written` / `failed` / `without_file` / 缺符号张数与处数；可展开查看未写出图纸名单（图名 + `error`）与缺符号图纸名单（图名 + `missing_symbols`）；
   - SQL Server 包固定显示「该包为 SQL Server 备份，导出内容未经参考验收」提示条。
3. **整次失败**：错误态，显示错误码（如 `SPPID_EXPORT_UNSUPPORTED`）与错误原因全文，配「重新配置导出」按钮。

## 4. 数据与代码改动边界

- 修改 `src/app/commands/pidbackupexport.rs` 与 `src/app/commands/pidbackuppdf.rs`：保留原有命令行汇总字符串不变；为 `BackupPdf::Running` 补 `started: Instant`；将 `running` 状态与 `last` 结构化结果（`Option<&Result<ExportOutcome, ExportError>>` / `Option<&Result<PdfOutcome, PdfError>>`）暴露为 `pub(in crate::app)` 只读访问器，并提供从界面直接传入结构化请求启动后台导出的入口函数（与命令共用同一套忙碌检查、缓存清理拦截和完成消息派发）。
- 新建 `src/app/pid_shell/backup_export_ui.rs`（或作为 `src/app/pid_shell/backup/` 子模块）：对话框状态、即时校验、进行中卡片、摘要条与详情展开视图、打开本地目录/文件动作（测试中注入目录选择与打开目录替身，不弹真实系统资源管理器窗口）。
- 修改 `src/app/pid_shell/chrome.rs`：在 `status_parts` 中加入导出进行中的简短全局状态项（U1）。
- 订阅接线：在 `src/app/view/mod.rs` 的 `subscription()` 中，当 `pid_backup_export` 或 `pid_backup_pdf`（及 `pid_backup` 载入中）正在运行时挂载 1 秒计时消息刷新已用秒数与原子进度。

## 5. 成功标准

1. **不敲命令走通导出闭环**（检查点 C 第 1 条后半）：在「备份数据包」画面打开样本包 → 点「导出 publish 文件」选空目录完成导出，界面显示写出数、文件数、耗时（带参考目录时显示比对四项计数）→ 点「导出 PDF」选**同一个目录**完成导出，期间看得到张数递增，结束后看到写出数与缺符号统计，交付目录里 `<图名>_Data.xml` 与 `<图名>.pdf` 成对，且写出的文件与命令行工具 100% 逐字节相同。
2. **前置校验当场拦截**：
   - publish 对话框选非空目录、包内目录或不存在的参考目录时，不启动后台线程、不写文件，界面当场显示原因并禁用提交；
   - PDF 对话框选已有同名 `.pdf` 或 `_PdfIndex.json` 的目录时当场拦截，选只有 publish 文件的目录时放行；
   - 散放 `.pid` 目录（无 `Manifest.txt`）下，「导出 publish 文件」按钮禁用并说明原因，「导出 PDF」正常可用。
3. **进行中非阻塞与双线并行**：替身卡住后台导出时，界面显示进行中状态、对应按钮禁用、清缓存被拒，但切去编辑器画线/缩放完全正常；publish 在跑时仍可启动 PDF 导出到另一目录，两项各自显示各自进度与结果。
4. **异常与明细完整展示**：用换坏一张 `.pid` 的小包或返回错误的替身验证，未写出图纸名单、原因、SQL Server 提示、整次错误码都能在界面结果区完整看到，不静默吞掉任何一项。
5. **双主题与门禁**：亮色与暗色均走 `pid_tokens`，与设计稿画框一致；除非在 U2 中另行决定加专用自动化操作，`src/rest_openapi.json` 与 MCP 工具摘要保持不变。

## 6. 测试

| 层 | 测什么 |
|---|---|
| 快测试（`iced_test` + 替身导出函数） | 按钮启用/禁用条件（无包、散放 `.pid`、运行中）；两个对话框的即时预检拦截与放行；替身卡住时的进行中显示、跳秒刷新、`PdfProgress` 张数刷新、切屏不丢状态、双导出并行；三种结果形态（全成、带未写出/缺符号/比对明细/SQL Server 提示、整次报错）的渲染核对 |
| 真实样本与真实窗口验收 | 隐藏桌面真实程序（不加 `--classic`），不敲命令走完打开小包/全厂包 → 界面导出 publish → 界面导出 PDF 到同一交付目录 → 用 `check-pairs.ps1` 核对成对并与 `EXP-3` / `PDF-5` 比字节；截取各状态暗/亮色图对照设计稿 |

## 7. 已定问题（U1–U3）

2026-11-20 答复「按照推荐方案继续下一步。」，三项均按建议定（若后续 Pencil 设计稿定稿时摆放另有调整，以设计稿为准）：

| # | 问题 | 定的结果 |
|---|---|---|
| U1 | **导出进行中的提示位置**（`02-screen-content.md` §8 第 4 项）：只放在 `backup` 画面内，还是在外壳底部状态栏也显示一条简短全局提示 | **已定（2026-11-20）：两处联动**——`backup` 画面内放完整的进行中条（含路径、张数/秒数），外壳底部状态栏右侧（`chrome::status_parts`）顺带显示简短提示（如 `正在导出 PDF 18/53`），切去编辑器看图时也能看见并可点击切回；若设计稿另有画法则照设计稿 |
| U2 | **是否在这一步同步补齐专用的自动化 / REST / MCP 导出发起与查状态操作**（`SPEC-sppid-export.md` §9 X6、`plan.md` §6） | **已定（2026-11-20）：与界面解耦，分两步**——界面直接读写内部 Rust 状态，不依赖自动化通道，本次不动 `src/rest_openapi.json` 与 MCP 摘要；若后续外部脚本需要非阻塞轮询，可在界面完成后单独排一个小任务 |
| U3 | **publish 与 PDF 两次结果在 `backup` 画面怎么摆**（`02-screen-content.md` §8 第 5 项） | **已定（2026-11-20）：包概要下方双状态摘要条**——未导出时不占地方，运行中显示各自进度，完成后显示摘要行 +「查看详情」「打开目录」「打开清单」按钮，点「查看详情」展开未写出名单、缺符号与比对明细；若设计稿另有画法则照设计稿 |

## 8. 需求对照

| 需求 | 本块 | 说明 |
|---|---|---|
| `REQ-MIG-009` 把备份数据包导出为 publish 文件 | 补齐界面闭环 | 与 `SPEC-sppid-export.md` 合起来完整覆盖 `REQ-MIG-009` |
| `REQ-MIG-010` 把备份数据包的图纸导出为 PDF | 补齐界面闭环 | 与 `SPEC-sppid-pdf.md` 合起来完整覆盖 `REQ-MIG-010` |
| 地图检查点 C 第 1、2 条 | 覆盖 | 不敲命令走完导出并看到结果；对话框、进行中、三种结果形态与设计稿暗/亮色一致 |

## 9. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-11-19 | 初稿（建议稿，列 U1–U3 推荐选项） |
| 2026-11-20 | 答复「按照推荐方案继续下一步。」，状态改为已确认；U1–U3 全部按建议定（§3.3、§3.4、§4、§7）；按 `src/app/commands/{pidbackupexport,pidbackuppdf}.rs`、`src/app/pid_shell/chrome.rs`、`src/app/view/mod.rs` 实际代码校准 §2 与 §4 |
