# SmartPlant 备份包：检查点 C（新版浏览与导出界面）· 任务清单

> 日期：2026-11-20
> 状态：**已完成**（2026-11-20，`UI-1` – `UI-9` 全部 9 个任务均已按「先红后绿 + 固定门禁 + 一任务一提交」落地并通过隐藏桌面真实窗口 0 命令闭环验收；详见 `readings.md` §32–§40）。
> 配套：`plan-checkpoint-c.md`（分段、依赖图、检查点、风险、需要你做的事）；界面规格：`SPEC-sppid-screen.md`（S1–S4 已确认）、`SPEC-sppid-export-ui.md`（U1–U3 已确认）；画框内容：`02-screen-content.md`；共用约定：`01-shared-conventions.md`。
> 基线：`main @ 51b654a87`（检查点 A、检查点 B 与总验收 `ACC-1` 均已在库）。

## 0. 怎么读这份清单

共 9 个任务（`UI-1` – `UI-9`），分三段：

| 段 | 内容 | 任务 | 规格 | 是否依赖 `.pen` 定稿 |
|---|---|---|---|---|
| 1 | 状态与导出底座 | `UI-1` – `UI-2` | `SPEC-sppid-screen.md` §2.2、`SPEC-sppid-export-ui.md` §2.2/§4 | **否**（可先行） |
| 2 | 浏览画面（`sppid-screen`） | `UI-3` – `UI-6` | `SPEC-sppid-screen.md` | **是**（需 `Nav backup` 与浏览画框） |
| 3 | 导出界面与闭环验收（`sppid-export-ui`） | `UI-7` – `UI-9` | `SPEC-sppid-export-ui.md`、`00-capability-map.md` §5 | **是**（需导出对话框、进行中与结果画框） |

每个任务写六项：**做什么**、**验收**、**验证**、**依赖**、**文件**、**规模**（S / M）。类别标记沿用 `todo.md`（【接线】【界面】【验收】）。

## 1. 每个任务都要过的固定门禁

沿用 `todo.md` §1，按当前基线（`main @ 51b654a87`、`rustc 1.102.0-nightly (65ce8611f 2026-11-18)`）核对：

1. **先红后绿。** 先写测试，看到因目标断言失败（非编译失败），再实现至全绿；验证既有行为的测试先用故意错的期望值确认能红，再改回。红、绿输出留存至 `D:\scratch\ocs-sppid\<日期>-<任务>\`。
2. `cargo test --offline --locked --lib`：测试总数不减少，不新增失败（当前 `51b654a87` 基线：7118 条，7061 过、55 忽略、2 失败——那 2 条是工作区未提交 `.pen` 的核对测试；`.pen` 定稿后核对是否全绿）。
3. `cargo clippy --offline --locked --lib` 警告不超过上一个提交在同编译器上的读数（当前 `656`）；加 `--profile test` 不超过当前读数（`1,051`）。两个数记入 `readings.md`。
4. 改动文件不新增 `rustfmt` 不一致：`scripts\fmt-newhunks.ps1 <改前的副本> <改后的文件>`，新增 0。
5. 动了 `src/io/` 或 `src/app/` 的任务，前后各跑一次 `python scripts/arch_check.py report`：`io` 开头各行数字不变大，不引入违规反向依赖。
6. 动了 `docs/pid-platform/` 的任务（`UI-3`）加跑文档一致性检查（`cargo test --offline --locked --test pid_platform_docs` 或对应文档检查入口）。
7. **并发编译避让**：动跑 `cargo` 前先检查本机是否有另一工作树（如 `ocs-sppid-exp2`）正在跑 `cargo`/`rustc`，避免争抢共享 `CARGO_TARGET_DIR`。
8. **一个任务一个提交，不推送，未经发话不碰 `docs/pid-platform/design/pid-platform.pen`。**

---

## 第 1 段：状态与导出底座（不依赖 `.pen`，可先行）

### UI-1 【接线】备份包载入计时与浏览只读访问器

- **做什么**：在 `PidBackupBrowser`（`src/ui/window/pid_backup_browser.rs`）与 `src/app/commands/pidbackup.rs` 中补齐新版浏览画面所需的只读数据访问与载入计时（`SPEC-sppid-screen.md` §2.2、§3、§9 S2–S3）：
  1. 记录最近一次备份包来源路径 `last_source: Option<PathBuf>` 与载入起始时刻 `loading_started_at: Option<Instant>`（进入 `LoadState::Loading` 时写入，载入完成或失败时保留 `last_source` 以便出错态展示来源路径）；
  2. 提供特殊装置组显示名纯函数（`Unregistered` → `"未登记图纸"`，`(ungrouped)` → `"未分组"`，S2）；
  3. 提供按选中装置组路径 + 搜索关键字过滤图纸序号的纯函数、按图纸序号统计图内对象类型分组及按 `(type_filter, item_query)` 过滤对象的纯函数，以及还原缓存目录/占用只读访问器，保持经典面板行为一字不变。
- **验收**：
  - [x] 进入 `LoadState::Loading(path)` 时 `loading_started_at` 有值、能读出已用秒数；失败退回 `Idle` 且 `error.is_some()` 时仍能读出刚才失败的来源路径。
  - [x] `Unregistered` 与 `(ungrouped)` 映射为「未登记图纸」「未分组」，普通装置组保持原名。
  - [x] 装置组子树过滤与名称大小写不敏感关键字过滤纯函数对含嵌套组、未登记图、缺文件图的测试索引返回精确序号列表。
  - [x] 图内对象按类型统计及按类型+关键字过滤纯函数结果正确；经典面板原有 `pid_backup` 测试全过。
- **验证**：`cargo test --offline --locked --lib pid_backup`
- **依赖**：无
- **文件**：`src/ui/window/pid_backup_browser.rs`、`src/app/commands/pidbackup.rs`
- **规模**：S
- **做完**（2026-11-20）：新增 `PidBackupBrowser::{last_source, loading_started_at, loading_elapsed, active_or_failed_source}`、`screen_group_label`、`group_total_drawings`、`filter_drawings`、`item_type_counts`、`filter_drawing_items` 及 `OpenCADStudio::{load_pid_backup, pick_backup_source}` 的 `pub(in crate::app)` 可见性；4 条 `pid_backup_ui1_*` 先红后绿（`pid_backup` 303 过 / 3 忽略 / 0 失败；全库 7122 条：7066 过 / 55 忽略 / 1 失败，仅工作区未提交 `.pen` 字重测试；clippy `656` / `1051`；`fmt-newhunks` 新增 0；`arch_check` 零变化）。读数见 `readings.md` §32，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui1\`。

### UI-2 【接线】导出状态结构化访问器、界面启动入口与 1 秒跳秒订阅

- **做什么**：打通 `BackupExport`、`BackupPdf` 对界面的结构化读写接口与运行期跳秒订阅（`SPEC-sppid-export-ui.md` §2.2、§3.3、§4）：
  1. `src/app/commands/pidbackuppdf.rs`：`BackupPdf::Running` 增加 `started: Instant`；暴露 `pub(in crate::app)` 只读方法读取当前运行状态（输出目录、已完成张数 `done`、总张数 `total`、已用秒数）及最近一次结构化结果 `Option<&Result<PdfOutcome, PdfError>>`；提供从界面直接传入输出目录启动后台 PDF 导出的入口函数，复用与命令行完全一致的忙碌检查、前置校验与完成消息派发（同时照旧更新命令行汇总，保证界面启动后敲裸 `PIDBACKUPPDF` 也能查到同一状态）。
  2. `src/app/commands/pidbackupexport.rs`：暴露 `pub(in crate::app)` 只读方法读取当前运行状态（输出目录、工厂名、参考目录、已用秒数）及最近一次结构化结果 `Option<&Result<ExportOutcome, ExportError>>`；提供从界面直接传入 `(out_dir, plant, reference_dir)` 启动后台 publish 导出的入口函数，与 `PIDBACKUPEXPORT` 共享同一状态与清缓存拦截。
  3. `src/app/view/mod.rs`：当窗口处于新界面且 `pid_backup` 处于 `Loading`、或 `pid_backup_export` / `pid_backup_pdf` 正在运行时，在 `subscription()` 中挂载 `iced::time::every(Duration::from_secs(1))` 触发界面重绘跳秒；空闲时零订阅开销。
- **验收**：
  - [x] 从界面入口函数发起 publish 导出与 PDF 导出后，运行期能读到输出目录、已用秒数及 PDF 的 `done / total`；同时敲不带参数的 `PIDBACKUPEXPORT` / `PIDBACKUPPDF` 返回同一运行中状态，清还原缓存同样被拒。
  - [x] 导出完成后，`last_outcome()` 直接返回结构化 `Ok(&ExportOutcome)` / `Err(&ExportError)` 与 `Ok(&PdfOutcome)` / `Err(&PdfError)`，无需反解英文字符串；从命令行发起的导出完成后访问器同样能读到结果。
  - [x] 前置校验不通过（如目录非空或同名 `.pdf` 冲突）时当场返回结构化错误，不覆盖上一次已完成导出的 `last` 记录。
  - [x] 任一导出或备份载入运行中时 `subscription()` 产出 1 秒定时订阅，三者均空闲时不挂该订阅。
- **验证**：`cargo test --offline --locked --lib pidbackupexport`；`cargo test --offline --locked --lib pidbackuppdf`
- **依赖**：UI-1
- **文件**：`src/app/commands/pidbackupexport.rs`、`src/app/commands/pidbackuppdf.rs`、`src/app/view/mod.rs`
- **规模**：M
- **做完**（2026-11-20）：`BackupExport` 与 `BackupPdf` 新增 `is_running`、`running_status`、`last_outcome` 只读访问器（`BackupPdf::Running` 补 `started: Instant`），`OpenCADStudio` 新增 `check_pid_backup_export_ui`、`start_pid_backup_export_ui`、`check_pid_backup_pdf_ui`、`start_pid_backup_pdf_ui` 并与命令行共享同一执行路径，顶层 `view::subscription()` 按 `pid_backup_wants_second_tick()` 挂载 1 秒 `Message::Tick` 订阅；4 条 `ui2_*` 先红后绿（`pidbackup` 70 过 / 4 忽略 / 0 失败；全库 7126 条；clippy `656` / `1051` 零新增；三个文件 `fmt-newhunks` 均新增 0；`arch_check` 零变化）。读数见 `readings.md` §33，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui2\`。

---

## 第 2 段：浏览画面（`sppid-screen`，依赖 Pencil 设计稿定稿）

### UI-3 【接线】导航三处接线与 `backup` 空画面骨架

- **做什么**：在 `docs/pid-platform/design/pid-platform.pen` 已定稿加入 `Nav backup` 后，三处同步注册 `PidScreen::Backup`（`SPEC-sppid-screen.md` §5.1、§6 第 1 条、§9 S1）：
  1. `docs/pid-platform/prototype/specs/screen-catalog.json` 加入 `backup` 条目（`02-screen-content.md` §6）；
  2. `docs/pid-platform/requirements/00-overview.md` 将 `REQ-MIG-008`、`REQ-MIG-009`、`REQ-MIG-010` 的承载画面更新为 `backup`；
  3. 新建 `src/app/pid_shell/backup.rs`（最小 `State`、`Message`、`VIEW = Some(view)`、`PID_SHELL:backup` 外框），并在 `src/app/pid_shell/mod.rs` 中接入 `PidScreen::Backup`（`id = "backup"`，`title = "备份数据包"`，`group = NavGroup::Design`（或照定稿 `.pen`），`needs_drawing = false`）与 `PIDSCREEN backup` 命令切换。
- **验收**：
  - [x] `pid_shell_startup::the_screens_are_the_ones_the_design_navigates_to` 通过（`.pen`、`screen-catalog.json`、`PidScreen::ALL` 三处完全一致）。
  - [x] `nav()` 列出 `Editor` 与 `Backup`；点击左侧导航「备份数据包」或执行 `PIDSCREEN backup` 切换到 `PidScreen::Backup`，画布上报 `PID_SHELL:backup` 矩形。
  - [x] `docs/pid-platform/` 文档检查测试通过。
- **验证**：`cargo test --offline --locked --lib pid_shell_startup`
- **依赖**：Pencil 设计稿定稿（`sppid-design`）
- **文件**：新建 `src/app/pid_shell/backup.rs`；修改 `src/app/pid_shell/mod.rs`、`src/app/pid_shell/pid_shell_startup.rs`、`docs/pid-platform/prototype/specs/screen-catalog.json`、`docs/pid-platform/requirements/00-overview.md`
- **规模**：S
- **做完**（2026-11-20）：新建 `src/app/pid_shell/backup.rs` 并在 `PidScreen::ALL`（`设计` 组紧跟 `Editor`）、`screen-catalog.json`、`00-overview.md`（`REQ-MIG-008..010`）及文档契约 `SCREEN_IDS` 同步注册 `backup`（`备份数据包`）；先红后绿（`pid_shell_startup` 14/14、`pid_shell_chrome` 11/11、`pytest scripts/pid-platform` 590/590；全库 7129 条：7073 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 均新增 0；`arch_check` 0 变化）。读数见 `readings.md` §34，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui3\`。

### UI-4 【界面】浏览画面：未打开、载入中、出错三种状态及缓存与经典面板入口

- **做什么**：按定稿设计稿实现 `backup.rs` 的「没打开」「载入中」「出错」三种整屏状态及包概要顶栏框架（`SPEC-sppid-screen.md` §3、§9 S3）：
  1. 未打开态：说明文案 +「打开备份 zip」「打开备份文件夹」两个主按钮（经可注入文件/目录选择器替身调 `start_pid_backup`）；
  2. 载入中态：显示来源路径、「正在还原并建立索引…」与已用秒数；
  3. 出错态：显示来源路径、错误原因全文（含 `EXP-1b` 无备份且无 `.pid` 报错）及「重新选择 zip」「重新选择文件夹」按钮；
  4. 已打开顶栏：显示清单概要字段（工厂名、版本、备份时间、来源路径与方式、图纸/文件/参考数据/警告计数）、切换备份包按钮，以及右上角次要操作「清理还原缓存」「在经典面板中查看库表」（S3，点后者切到编辑器并展开经典备份包面板）；全套颜色/字号/圆角取自 `pid_tokens`。
- **验收**：
  - [x] `iced_test` 无窗口测试覆盖「没打开」「载入中」「出错」三种状态的部件边界与文字，注入选择路径替身后能把状态推进到 `Loaded` 或 `Failed`。
  - [x] 包概要栏正确展示 Oracle 包（含清单字段）、SQL Server 包与散放 `.pid` 目录（无清单字段）三种概要信息。
  - [x] 点击「清理还原缓存」在无导出运行时触发清缓存，有导出运行时给出忙碌提示；点击「在经典面板中查看库表」切换到编辑器并打开经典面板。
  - [x] 亮色与暗色主题下无硬编码色值。
- **验证**：`cargo test --offline --locked --lib pid_backup_screen`
- **依赖**：UI-1、UI-3
- **文件**：`src/app/pid_shell/backup.rs`、新建 `src/app/pid_shell/pid_backup_screen.rs`
- **规模**：M
- **做完**（2026-11-20）：在 `src/app/pid_shell/backup.rs` 实现未打开、载入中（跳秒）、出错三态及包概要顶栏（含可注入 `SourcePicker`、清缓存忙碌拦截提示与经典面板切换），在 `src/app/commands/pidbackup.rs` 避免 `Backup` 画面下自动弹开经典停靠面板，新建 `src/app/pid_shell/pid_backup_screen.rs` 4 条快测试先红后绿（`pid_backup_screen` 4/4、`pid_backup_in_shell` 16/16；全库 7133 条：7077 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 均新增 0；`arch_check` 0 变化）。读数见 `readings.md` §35，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui4\`。

### UI-5 【界面】浏览画面：装置组树、图纸搜索清单、属性与图内对象详情

- **做什么**：按定稿设计稿实现「已打开（图可读）」与「已打开（图读不出/缺文件）」的三栏主体区（`SPEC-sppid-screen.md` §3、§5.2、§6 第 2–3 条）：
  1. 左栏装置组树：「全部图纸」+ 递归装置组节点（含直接张数/含下级总数，`Unregistered` 显示「未登记图纸」、`(ungrouped)` 显示「未分组」），点选过滤中间清单；
  2. 中栏图纸清单：顶部搜索输入框 + 可滚动图纸列表（显示图名、所属组、缺文件/读不出警示标记），选中图纸时若尚未加载 `DrawingItems` 自动触发后台读取；
  3. 右栏详情区：「图纸属性」（数据库全部键值列 + 图纸对象属性，按实际列数动态展示）与「图内对象」（按类型分组计数、类型筛选 + 关键字过滤、展开看接口属性）；当图纸缺文件或 `ItemsState::Failed(reason)` 时，保留数据库字段并在对象区清晰显示原因文字（不显示空表）。
- **验收**：
  - [x] 打开有嵌套装置组、未登记图、缺文件图、损坏图的测试包后，装置组切换与关键字搜索过滤结果精确。
  - [x] 选中可读图纸时自动后台读取并展示数据库属性与图内对象，按类型过滤与关键字过滤生效。
  - [x] 选中读不出图内对象的图纸（如 `D04` 情形）时，详情区清晰显示读不出原因而非空表；选中缺文件图纸时「在编辑器中打开」置灰并提示原因。
- **验证**：`cargo test --offline --locked --lib pid_backup_screen`
- **依赖**：UI-4
- **文件**：`src/app/pid_shell/backup.rs`、`src/app/pid_shell/pid_backup_screen.rs`
- **规模**：M
- **做完**（2026-11-20）：在 `src/app/pid_shell/backup.rs` 实现三栏主体（左栏递归装置组树与「未登记图纸」/「未分组」中文映射、中栏关键字搜索与状态标记图纸表、右栏数据库动态列+`DrawingNo` 与图内对象类型标签/关键字过滤/展开接口属性，及缺文件/对象读不出提示框），在 `pidbackup.rs` 配合备份加载自动初始化选中与首图预读；2 条 `pid_backup_screen_ui5_*` 先红后绿（`pid_backup_screen` 6/6；全库 7135 条：7079 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 新增 0；`arch_check` 0 变化）。读数见 `readings.md` §36，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui5\`。

### UI-6 【接线】从 `backup` 开图、已开标签页切换、失败留屏不弹窗与切屏状态保持

- **做什么**：打通从 `backup` 画面双击图纸行或点「在编辑器中打开」到 `Editor` 的完整流转（`SPEC-sppid-screen.md` §4、§6 第 2–4 条、§9 S4）：
  1. 若目标 `.pid` 已在某个标签页打开（`tab_showing`），直接切换到该标签页并 `PidShell::show(Editor)`，不重开第二份（`RD-6`）；
  2. 若尚未打开，标记 `backup.opening_drawing = Some(path)` 显示「正在打开…」并发起异步开图：成功（`on_file_opened`）后按窄画布自适应图幅（`RD-7`）并自动切到 `PidScreen::Editor`；失败（`FileOpenFailed`）时将错误原因记入 `backup.open_failures`、清除 `opening_drawing`、**留在 `PidScreen::Backup` 并在当前图详情条展示原因，不弹整窗 `ModalKind::Recovery`（S4）**（从编辑器命令打开失败仍照旧弹窗）；
  3. 验证切去 `Editor` 再经左侧导航或 `PIDSCREEN backup` 切回时，装置组、搜索词、选中图与过滤条件完整保持；在编辑器敲 `PIDBACKUPOPEN` 后切到 `backup` 画面直接同步显示该包。
- **验收**：
  - [x] 未打开的可读图点打开后切入 `Editor` 并新增标签页；已打开的图再点打开时标签页总数不变、直接激活原标签页。
  - [x] 打不开的图（如 `D03` 或损坏 `.pid`）从 `backup` 点打开后，仍留在 `PidScreen::Backup`、`dialog_is_up() == false`、不多出空白废标签页，且画面显示具体失败原因；从编辑器命令打开坏文件时恢复弹窗行为不变。
  - [x] `Backup -> Editor -> Backup` 往返后选中装置组、搜索词、选中图、对象过滤词原样保留。
- **验证**：`cargo test --offline --locked --lib pid_backup_screen`
- **依赖**：UI-5
- **文件**：`src/app/pid_shell/backup.rs`、`src/app/pid_shell/pid_backup_screen.rs`、`src/app/update/file_io.rs`（或对应 `FileOpenFailed` / `on_file_opened` 处理文件）
- **规模**：S
- **做完**（2026-11-20）：在 `src/app/pid_shell/backup.rs` 实现「在编辑器中打开」按钮与图纸行双击开图、已打开图纸 `tab_showing` 标签页直接切换、「正在打开…」状态与 `open_error` 内联提示条，在 `src/app/update/mod.rs` 的 `Message::FileOpened` 成功分支自动切入 `PidScreen::Editor`、失败分支按 S4 将从 `Backup` 发起的错误记回本画面不弹 `ModalKind::Recovery`（编辑器命令开坏图仍维持弹窗）；2 条 `pid_backup_screen_ui6_*` 先红后绿（`pid_backup_screen` 8/8；全库 7137 条：7081 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 新增 0；`arch_check` 0 变化）。读数见 `readings.md` §37，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui6\`。

---

## 第 3 段：导出界面与闭环验收（`sppid-export-ui` + 检查点 C）

### UI-7 【界面】publish 与 PDF 导出配置对话框及即时预检

- **做什么**：按定稿设计稿新建 `src/app/pid_shell/backup_export_ui.rs` 并在 `backup` 包概要栏接好「导出 publish 文件」「导出 PDF」两个独立入口与模态配置对话框（`SPEC-sppid-export-ui.md` §3.1、§3.2、§5 第 2 条）：
  1. 按钮启用/禁用与悬浮说明：无包、散放 `.pid`（无 `Manifest.txt` 禁 publish 但放行 PDF）、无带文件图纸、对应导出运行中四种禁用原因准确；
  2. publish 配置对话框：输出目录输入+浏览替身、工厂名（默认 `P01`，空串拦截）、可选参考目录输入+浏览+清空；每次修改路径即时只读调用 `in_the_restore` + `pid_backup_export::check`，目录非空、落在包/还原内或参考目录无效时当场红字列出原因（含前几个冲突文件名）并禁用「开始导出」；附 SQL Server 未验收提示及「同目录成对请先导 publish 再导 PDF」提示；
  3. PDF 配置对话框：输出目录输入+浏览替身，若已有上次 publish 成功输出目录则提供「使用上次 publish 输出目录」一键填入；每次修改路径即时只读调用 `pid_backup_pdf::check`，含同名 `.pdf` 或 `_PdfIndex.json` 时当场拦截，仅含 publish 文件时显示「可与该目录中的 publish 文件配对输出」并放行。
- **验收**：
  - [x] 散放 `.pid` 目录下「导出 publish 文件」禁用并提示缺少 `Manifest.txt`，「导出 PDF」可用。
  - [x] publish 对话框选非空目录、包内目录、还原目录、不存在的参考目录或空工厂名时，「开始导出」禁用且不启动后台导出、不写任何文件。
  - [x] PDF 对话框选已有同名 `.pdf` 或 `_PdfIndex.json` 的目录时禁用拦截；点「使用上次 publish 输出目录」填入仅含 `_Data.xml` / `_Index.json` 的目录时显示可配对提示并允许启动。
  - [x] SQL Server 包在两个对话框中均显示「没有验收过」提示。
- **验证**：`cargo test --offline --locked --lib pid_backup_export_ui`
- **依赖**：UI-2、UI-5
- **文件**：新建 `src/app/pid_shell/backup_export_ui.rs`；修改 `src/app/pid_shell/backup.rs`、`src/app/pid_shell/pid_backup_screen.rs`
- **规模**：M
- **做完**（2026-11-20）：新建 `src/app/pid_shell/backup_export_ui.rs` 与 `src/app/pid_shell/pid_backup_export_ui.rs`，在 `backup` 顶栏接入「导出 publish 文件」「导出 PDF」按钮禁用原因预判及两个模态配置对话框（即时只读调用 `check_pid_backup_export_ui` / `check_pid_backup_pdf_ui`、SQL Server 未验收提示、「使用上次 publish 输出目录」一键配对填入）；3 条 `pid_backup_export_ui_ui7_*` 先红后绿（`pid_backup_export_ui` 3/3；全库 7140 条：7084 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 新增 0；`arch_check` 0 变化）。读数见 `readings.md` §38，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui7\`。

### UI-8 【界面】导出进行中两处联动提示与双结果摘要条/展开详情

- **做什么**：按定稿设计稿实现进行中状态与三种导出结果视图（`SPEC-sppid-export-ui.md` §3.3、§3.4、§5 第 3–5 条、§7 U1–U3）：
  1. **进行中两处联动（U1）**：`backup` 画面包概要下方显示完整进行中条（publish 显示路径、工厂名、跳秒；PDF 显示路径、`done / total` 进度条与跳秒）；同时在 `src/app/pid_shell/chrome.rs` 的 `status_parts` 右侧加入简短全局状态项（在非 `Backup` 画面点击可切回 `Backup`）；
  2. **双结果摘要条与展开详情（U3）**：publish 与 PDF 完成后各自显示摘要条 +「查看详情」「打开输出目录」「打开清单文件」按钮（经可注入打开目录替身）；支持①全成功无提醒、②警示完成（展开可滚动查看 `unwritten`、`noted`、`other_problems`、`warnings`、PDF 缺符号图纸与明细、`GateSummary` 四格计数与逐图比对 `lines`、SQL Server 提示）、③整次报错（错误码 + 原因 +「重新配置导出」）三种形态；
  3. 验证 publish 与 PDF 同时在后台跑（到不同目录）互不阻塞，且 `src/rest_openapi.json` 与 MCP 摘要保持不变（U2）。
- **验收**：
  - [x] 替身卡住 publish 与 PDF 导出时，`backup` 画面同时显示两条独立进行中进度，切到 `Editor` 时外壳底部状态栏右侧显示全局简短提示且点击能切回 `Backup`，画布操作不锁死。
  - [x] 三种结果形态（全成功、带未写出/缺符号/参考比对四项统计与明细/SQL Server 提示、整次失败带 `SPPID_*` 错误码）在 `iced_test` 下逐一核对通过。
  - [x] 点击「打开输出目录」「打开清单文件」把准确路径传给注入的打开动作替身。
  - [x] 亮色与暗色双主题均走 `pid_tokens`，`src/rest_openapi.json` 零改动。
- **验证**：`cargo test --offline --locked --lib pid_backup_export_ui`；`cargo test --offline --locked --lib pid_shell_chrome`
- **依赖**：UI-7（开工前先检查并对齐 `collab-off2` 对 `chrome.rs` / `pid_shell_chrome.rs` 的改动，避免冲突）
- **文件**：`src/app/pid_shell/backup_export_ui.rs`、`src/app/pid_shell/backup.rs`、`src/app/pid_shell/chrome.rs`、`src/app/pid_shell/pid_shell_chrome.rs`
- **规模**：M
- **做完**（2026-11-20）：在 `backup.rs` 包概要下方挂载 `status_banners`（publish 跳秒进行中、PDF `done / total` 进度条与跳秒，以及全成、带未写出/缺符号/参考比对 `GateSummary` 四格统计与明细/SQL Server 提示、整次报错带 `SPPID_*` 错误码三种结果摘要条与展开滚动抽屉、可注入 `PathOpener` 打开目录/清单动作，另补内联路径输入直开条），在 `chrome.rs` 的 `status_parts` 接入全局简短运行提示与一键切回 `Backup`；2 条 `pid_backup_export_ui_ui8_*` 先红后绿（`pid_backup_export_ui` 5/5、`pid_shell_chrome` 11/11；全库 7142 条：7086 过 / 55 忽略 / 1 既有未提交 `.pen` 字重测试；clippy `656` / `1051` 零新增；`fmt-newhunks` 新增 0；`arch_check` 0 变化；`src/rest_openapi.json` 0 变化）。读数见 `readings.md` §39，证据在 `D:\scratch\ocs-sppid\2026-11-20-ui8\`。

### UI-9 【验收】检查点 C 真实窗口闭环与双主题对照验收

- **做什么**：在隐藏桌面上启动真实编译出的 `OpenCADStudio.exe`（不加 `--classic`），**全程不敲一条命令**，用界面点击与路径选择走完检查点 C 的全部三条标准（`00-capability-map.md` §5 检查点 C、`plan-checkpoint-c.md` §5）：
  1. 全厂包（目录，53 张）：进入「备份数据包」→ 打开目录 → 点选装置组树、搜索图纸 → 查看可读图纸属性与图内对象 → 选中 `D04` 核对读不出原因 → 选中 `D03` 点打开核对留在 `backup` 画面显示打不开原因且不弹恢复窗 → 打开一张可读图切到编辑器核对窄画布整幅可见、切回再点同一张图核对直接切原标签页 → 点「导出 publish 文件」（带参考目录）导到新交付目录 → 点「导出 PDF」用「使用上次 publish 输出目录」导到同一目录，观察张数进度与底栏提示 → 核对界面展示的结果统计、缺符号（15 张 22 处）与比对结论；
  2. 小包（`.zip`，7 张）：从 `.zip` 打开并同样走完同目录 publish + PDF 成对导出；
  3. 对两个交付目录跑 `scripts/sppid/check-pairs.ps1`（53 对 214 个文件、7 对 30 个文件），并与 `a85fa10` 基线及 `PDF-5` 做逐字节比对；
  4. 截取各关键状态的亮色与暗色画面截图，与 Pencil 设计稿画框逐张对照，把读数、截图位置与结论写入 `readings.md` 并提交检查点 C 验收汇报。
- **验收**：
  - [x] 全程 0 条手工命令完成全厂包目录与小包 `.zip` 的打开、浏览、开图、publish 导出、PDF 同目录配对导出与结果查看。
  - [x] `scripts/sppid/check-pairs.ps1` 对全厂包（53 对，214 个文件）与小包（7 对，30 个文件）退出码均为 0。
  - [x] 界面导出的 publish 文件与 `2026-11-20-exp3-rerun`（`a85fa10` 基线）逐字节相同（清单内运行路径除外），PDF 与 `PDF-5` 逐字节相同。
  - [x] 亮色与暗色各状态截图与 Pencil 设计稿画框一致，`the_screens_are_the_ones_the_design_navigates_to` 及固定门禁全绿。
  - [x] `readings.md` 完整记录检查点 C 验收表与证据目录位置（`D:\scratch\ocs-sppid\<日期>-checkpoint-c\`）。
- **验证**：真实程序隐藏桌面走查 + `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\sppid\check-pairs.ps1 <交付目录>`
- **依赖**：UI-6、UI-8
- **文件**：`docs/plans/2026-11-17-sppid-backup-publish/readings.md`、`docs/plans/2026-11-17-sppid-backup-publish/todo-checkpoint-c.md`
- **规模**：S
- **做完**（2026-11-20）：在隔离隐藏桌面（`CreateDesktopW`）上启动真实编译产物 `target\release\OpenCADStudio.exe`（不加 `--classic`），全程 0 条命令（脚本内置禁止 `op == "run"` 断言）走完三组真实窗口验收：亮色全厂包（53 张目录：默认白色亮色界面→备份空态→非备份目录错误态→53 张载入→装置组过滤与对象展开→`D04` 不可读原因→`D03` 开图失败留屏红条且不弹窗不增废标签页→可读图 `D07` 打开切编辑器整幅可见与二次点开复用标签页→带参考目录 `验收数据` 导出 publish→「使用上次 publish 输出目录」同目录导出 PDF 并在编辑器底栏核对 `正在导出 PDF 1/53` 点击回跳→展开双结果核对比对 `7 / 0 / 4 / 42` 与缺符号 `15 张 22 处`，共 16 张整窗截图）、亮色小包 `.zip`（7 张：打开→可读图开图→同目录成对导出，4 张整窗截图）、暗色主题对照（8+1 张整窗截图逐项对齐 Pencil 暗色画框）；`check-pairs.ps1` 全厂包 `53 对 / 214 文件` 与小包 `7 对 / 30 文件` 退出码均为 `0`；`compare_deliver.py` 核对全厂包 `160/160` publish 与 `54/54` PDF 全部逐字节一致，小包除临时还原绝对路径外 `22/22` publish 与 `8/8` PDF 逐字节一致；`pid_shell_startup`（含 `the_screens_are_the_ones_the_design_navigates_to`）14/14、`pytest scripts/pid-platform` 590/590、`pytest scripts/sppid` 159/159 全绿。完整读数见 `readings.md` §40，证据与 29 张整窗截图在 `D:\scratch\ocs-sppid\2026-11-20-checkpoint-c\`。
