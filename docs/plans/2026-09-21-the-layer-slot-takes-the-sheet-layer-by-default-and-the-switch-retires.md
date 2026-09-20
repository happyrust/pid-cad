# 图层槽默认取图纸图层，`OCS_PID_LAYER_MODE` 退役 · 小计划（2026-09-21 开单，只分析）

> 承接 `2026-09-07-jdim-driving-dimensions-and-layer-panel.md` L3 / D5（OCS `d0750567`，09-18）：开关 `OCS_PID_LAYER_MODE`
> 上线时定的是「选项可切、默认不切；默认值翻转另开一轮，等 DXF 下游消费方的要求定下来」。09-19 / 09-20 两单退役开关时都把它
> 登记不做（「另一条线，不混进来」）。**2026-09-21 用户指示「继续分析：接台账上还开着的口子（OCS_PID_LAYER_MODE 退役 …）」**，
> 本单把两种模式在语料上的差别量出来、把翻转与退役要动的地方列出来。**只分析，未开工；带 ⭕ 的决策按推荐落笔，等批。**

## 一句话

两种模式画出来的图**一模一样**（每个实体的颜色、线宽、开关状态都在实体自己身上），差别只在**图层表与实体的图层槽**：
`taxonomy` 给一张导入器自己的 16–18 层分类表（`PID-GEOMETRY` / `PID-TEXT` / `PID-SYMBOL` / `PID-POINT-*` / `PID-STYLE-*` / `PID-HIDDEN` …），
`sheet` 给图纸自己的图层表（`Default` / `Labels` / `ConsistencyChecks` / `HiddenObjects` …，含空层与开关状态）。分类在两种模式下都写在
`PID_SEMANTICS` 的 `role=` / `style=` 里，谁都不丢。D5 当初等的「DXF 下游要求」，台账上至今没有人来说过；图例识别那批 DXF
（`0版重新处理dxf-12张`）是 AutoCAD 出的图，规则文件里的图层名是 `DEVICE` / `0` / `工艺外线` 那一套，既不是 `PID-*` 也不是
SmartPlant 的名字——它没在等这个默认值。本单建议：**默认翻到 `sheet`，`taxonomy` 输出模式与开关一起退役**；分类层的消费者改读 XDATA。

## 事实（2026-09-21，OCS `68b924f2` 的 debug 版 `--export`，四图各两种模式，PowerShell 解 DXF 组码）

| 项 | 数 | 出处 |
|---|---|---|
| 开关 | `LAYER_MODE_ENV = "OCS_PID_LAYER_MODE"`，`PidLayerMode::{Taxonomy（默认；未设 / 空 / 不认识都算它）, Sheet}`；`load_pid` = `load_pid_with_layer_mode(path, from_env())` | `io/pid.rs:180–240`、`488–496` |
| 机制 | 两种模式**都先按分类建实体**：`LAYER_TEXT` / `LAYER_SYMBOL` / `LAYER_GEOMETRY` / `LAYER_POINT*` / `PID-STYLE-*` 是构建期的**工作键**——`apply_symbology` 的 `painted` 判定（`1919–1922`）、图纸文字与符号文字的归类（`1702` / `1722` / `1749` / `1875` / `1912`）、角色 `role_of_layer`（`361–373`）都靠它。模式只在**两处**起作用：开头声明哪张图层表（`512–546`），末尾每个实体的槽填什么（`811–858`）：taxonomy = 隐藏的挪 `PID-HIDDEN`、`PID-STYLE-*` 按需声明；sheet = 槽填 authored name（按需声明、状态取文件），`symbol-label` 例外留 `PID-SYMBOL-LABEL`，无 authored name 的留分类层（隐藏且无名 → `PID-HIDDEN`） | `io/pid.rs` |
| 实体数 / 角色数 | 两种模式**逐图完全一致**：0201 335、0202 314、D06 60、工艺 687；角色 `role=` 计数逐项相同（0201：connectivity 25 / frame 1 / geometry 63 / point-ok 75 / point-warning 22 / symbol 81 / symbol-label 20 / text 48） | 本单 `--export` 对数 |
| taxonomy 图层表 | 0201 17 层 / 0202 18 / D06 16 / 工艺 16。`0` + 固定 13 层（`taxonomy_layers()`）+ 按需的 `PID-STYLE-*`（0201：PRIMARY-PIPING-NEW 24、ELECTRIC 1、CONNECT-TO-PROCESS 3；0202 四层 12 笔；D06 两层 4；工艺 两层 12）。**空层 4–7**：`0`、`PID-POINT-ERROR`（四图皆 0）、`PID-ANNOTATION`（四图皆 0）、`PID-POINT-APPROVED`（只工艺有 20）、`PID-FILL`（0201 / D06 为 0）、`PID-CONNECTIVITY`（只 0201 有 25）、`PID-GEOMETRY`（D06 为 0） | 同上 |
| sheet 图层表 | 0201 16 层 / 0202 15 / D06 15 / 工艺 17：图纸自己 12–15 层（含 `0`）+ 合成层 `PID-FRAME`（1）、`PID-SYMBOL-LABEL`（关）、`PID-CONNECTIVITY`（0201，关）、`PID-GEOMETRY`（只 0202：4 笔没有 authored name 的字形线）。**空层 8–11**（原图定义了但没画东西的）：`0` / `DrawingBorder` / `HeatTrace` / `Hidden`（关）/ `Label`（关）/ `NotClaimed` / `Notes` / `WaterMark`，另 `Heat Trace`（关；0201 / D06）、`ClaimedOnlyByOthers`（D06）、`LinkInfo_1` / `LinkInfo_2` / `NotesAG`（工艺）。四图里 **`PID-HIDDEN` 一次都没声明**——没有「隐藏且无名」的实体 | 同上 |
| sheet 下内容落在哪（0201） | `Default` 120 = symbol 81 + geometry 28 + point-ok 11；`Labels` 72 = text 47 + geometry 25；`ConsistencyChecks` 86 = point-ok 64 + point-warning 22；`HiddenObjects`（关）11 = geometry 10 + text 1。评审点（`point-*`）大多落 `ConsistencyChecks`，与 SmartPlant 一致性检查的语义相符 | 同上 |
| 颜色 / 线宽在谁身上 | 0201 taxonomy：geometry 63 / 63 实体自带颜色 + 线宽，symbol 81 / 81，point 97 / 97，text 47 / 48 自带颜色；**ByLayer 的只有** connectivity 25（`PID-CONNECTIVITY` 蓝、关）、symbol-label 20（灰、关）、frame 1——这三种在 sheet 下也留在各自 `PID-*` 层。所以 taxonomy 图层表上的颜色（`PID-TEXT` 绿、`PID-SYMBOL` 青、`PID-POINT` 品红 …）**没有一个实体在用**；两种模式屏幕与打印一致 | 同上（只对了 0201） |
| 分类层名在导入器之外的消费者 | `io/pid_view_filter.rs:372`（放开隐藏层时连带打开 `PID-HIDDEN` **或**同名图纸层——已经两种都认）；`app/layers.rs:64` 一句注释；探针 `pid_probe` / `pid_plot_dump` 已按 `role=`（09-07 D5）；图例识别 `assets/pid-legend.json` 的 `layers_any` / `pipe_layers` / `skip_layers` / `layer_prefixes` 全是客户 DXF 的图层名（`DEVICE` / `0` / `工艺外线` / `T-PIPE_DIESEL O` / `EVALVE_` …），**没有任何 `PID-*` 或 SmartPlant 图层名**——对 `.pid` 导入的图它两种模式下都找不到管线源层，与默认值无关 | `rg` |
| 测试 | `pid_import` 53 条按环境变量跑一遍（09-20 两模式 53 / 53 全绿）；**开关自己的 5 条**：`the_layer_mode_defaults_to_taxonomy_and_names_its_two_modes`、`the_two_layer_modes_agree_on_everything_but_the_slot`、`sheet_mode_files_every_entity_under_its_authored_layer_and_declares_the_drawings_layers`、`sheet_mode_layer_names_survive_dwg_and_dxf`、`sheet_mode_switching_a_hidden_sheet_layer_on_releases_its_own_layer` | `tests/pid_import.rs:3015–3200` |
| 文档 | user-guide 「用原图图层名作图层（`OCS_PID_LAYER_MODE`）」一段 + 「图纸图层视图」里按模式说的那句（`105` / `107`）；09-07 计划 D5 / 结算「本轮之后还开着的」/ 登记不做一行；09-19 / 09-20 两单各一行登记不做 | — |
| 「DXF 下游」是谁 | 台账里能对上的只有图例识别管线（`PIDLEGEND` → `dxf_legend --json` / CSV → plant 数据管线）。它吃的 `0版重新处理dxf-12张` 是 AutoCAD 出的图，不是 `.pid` 导出的；台账上**没有任何一条**记录某个消费方读过 `.pid` 导出 DXF 的图层名并提过要求。D5 等的那句话，等了三天没人说 | `2026-09-07-pid-legend-recognition.md`、`2026-09-09-…-audit-and-next-steps.md` C6 |

## 决策（按推荐落笔，等批）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| H-D1 | 默认值翻不翻 | **翻到 `sheet`**。D4 定「槽归图纸图层」本来就是目标态，taxonomy 只是先落地的过渡；屏幕不变、分类不丢（`role=` / `style=`）、第三方查看器里看到 SmartPlant 自己的层名；D5 等的下游要求三天没人说，而唯一对得上的下游（图例识别）的规则里两套名字都没有——等下去没有信息增量。备选「继续等」：保持现状，什么都不动 | ⭕ |
| H-D2 | taxonomy 输出模式留不留 | **退役，开关一起删**（同 09-20 退 `OCS_PID_SYMBOL_SOURCE` 的口径：一轮没人用就退）。备选「留一轮 opt-in」：多维护一张 17 层表、一套双跑测试、一段 user-guide，换一条回到旧图的路——但旧图与新图**看起来一样**，这条路没人需要 | ⭕ |
| H-D3 | 构建期的工作键怎么办 | **不动**：`LAYER_TEXT` / `LAYER_SYMBOL` / `LAYER_GEOMETRY` / `LAYER_POINT*` / `LAYER_DISCIPLINE_PREFIX` 继续作构建期的分类键（`painted`、文字归类、`role_of_layer` 都靠它），只在末尾一律换成 authored name；`taxonomy_layers()` 只留合成层要用的那几条（`PID-FRAME` / `PID-SYMBOL-LABEL` / `PID-CONNECTIVITY` / `PID-GEOMETRY` / `PID-HIDDEN` / `PID-ANNOTATION` / `PID-FILL`？——`PID-FILL` 与 `PID-ANNOTATION` 的实体有 authored name 的就跟着走，没有的才留），`ensure_taxonomy_layer` 按需声明 | ⭕ |
| H-D4 | `PID-STYLE-*` 去哪 | **随 taxonomy 退**。样式名在 XDATA `style=`、特性面板「样式」行、图层管理器「图纸图层」视图的角色段里都有；按样式分层是 09-07 之前「分类层就是一切」时代的产物 | ⭕ |
| H-D5 | `PID-HIDDEN` | **只留兜底**：隐藏且没有 authored name 的实体才去 `PID-HIDDEN`（语料四图为 0，代码路径保留、有单测钉）。`pid_view_filter` 放开隐藏层时连带打开的那条已经是「`PID-HIDDEN` 或同名图纸层」，不动 | ⭕ |
| H-D6 | 测试口径 | `pid_import` 不再按环境变量双跑：`import_in_mode` / `import_in_sheet_mode` 折回 `import`；`the_layer_mode_defaults…` 与 `the_two_layer_modes_agree…` 删；三条 `sheet_mode_*` 去掉前缀成为默认口径的测试；`import_without_library` 里的 `from_env()` 改直调。凡是按 `PID-TEXT` / `PID-SYMBOL` **输出层名**断言的测试（`letters(LAYER_SYMBOL)` 那类在单测里、在换槽之前，不受影响；集成测试里按 `pid_records_with(role)` 的也不受影响），改按 `role=` 取 | ⭕ |
| H-D7 | 「下游要求」这道门怎么关 | **反过来说一句**而不是等：user-guide 写「导出的 DWG / DXF 图层名 = SmartPlant 原图图层名；要按导入器的分类取实体，读 XDATA `PID_SEMANTICS` 的 `role=` / `style=`」。真有消费方要 `PID-*` 分类层，那是给它加一个导出选项的事，不是导入默认值的事 | ⭕ |

## 工作项（批了再做）

- **H1 `src/io/pid.rs`**（一提交）：删 `LAYER_MODE_ENV` / `PidLayerMode` / `load_pid_with_layer_mode`（`load_pid` 直接走 sheet 口径）；`512–546` 只留 sheet 那支；
  `811–858` 只留 sheet 那支；`taxonomy_layers()` 收成合成层清单；`PID-STYLE-*` 的按需声明删（工作键 `LAYER_DISCIPLINE_PREFIX` 留）；
  那行 `log::info!("… =sheet …")` 删。
- **H2 测试**（同一提交）：按 H-D6。
- **H3 台账**：user-guide `105` / `107` 两段改写（按 H-D7）；09-07 计划 D5 / 结算 / 登记不做三处改标；09-19 / 09-20 两单登记不做那行改标；本单头部写哈希。
- **H4 手工验收**：默认环境打开 0201，图层管理器「图层」视图列的是 `Default` / `Labels` / `ConsistencyChecks` / `HiddenObjects`（关）…，
  `HiddenObjects` 打开后 10 笔几何 + 1 条文字显出来；`--export` DXF 图层表与本单事实一致；`OCS_PID_LAYER_MODE=taxonomy` 设了也没反应、无日志。
  一张图层管理器截图入 `docs/evidence/`。

## 验收

- `pid_import` 53 → **50**（删 2 合 1，见 H-D6；数字以落地为准）全绿，环境变量设不设都一样；`pid_panel_localization` 绿；`--lib io::pid::tests` 数字不变。
- `rg OCS_PID_LAYER_MODE|PidLayerMode|load_pid_with_layer_mode|LAYER_MODE_ENV` 在 `src/` `tests/` `docs/user-guide.md` 零命中。
- 四图 `--export` 的图层表 = 本单事实表 sheet 那行。

## 登记不做

| 项 | 理由 |
|---|---|
| 合成层改名（`PID-FRAME` / `PID-SYMBOL-LABEL` / `PID-CONNECTIVITY` / `PID-GEOMETRY` / `PID-HIDDEN`） | 它们是导入器自己造的东西，图纸里没有对应层；名字沿用 |
| 给 `.pid` 导入的图配图例识别规则（`PID-*` 或 SmartPlant 层名进 `pid-legend.json`） | 图例识别是另一条线（09-07 / 09-09 两单）；且 `.pid` 导入的图自带位号 / 管线号，不需要走识别 |
| 按视图过滤集分别呈现图层状态 | 09-07 登记不做，照旧 |
| 导出时可选输出分类层 | H-D7：真有人要再开单 |

## 进度

（只分析，未开工。事实表的对数临时文件已清。）

## 门禁记录

- 2026-09-21：用户「继续分析：接台账上还开着的口子（OCS_PID_LAYER_MODE 退役 / …）」→ 本单（会话 fable-5-1-28）。七条决策等批。
