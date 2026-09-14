# JDim 驱动尺寸与图层面板重构 · 开发计划（2026-09-07）

> 承接 `2026-08-29-real-sheet-layers-and-jsite-geometry.md`（W1–W6 已于 08-31 落地；W4 于 09-07
> 以「嵌套 `LdcSite` = 内嵌符号定义缓存、变换在放置记录里」关闭，四曲线族解码器齐全）。
> 该计划「登记不做」里留下两条：**`JDim`（0x0115）标注解码**与**图层面板重构**（D2 第 3 步）。
> 本计划把这两条接过来。事实基础是 09-07 下午对四主图 + A01 的字节实跑（JDim 18 条逐条列出）、
> pid-parse 08-27 的 spacemap / tag-184 / tag-188 分析，以及 OCS 图层管理器现状。
> 带 ⭕ 的决策按推荐落笔、未经拍板，Plannotator 批注里划一笔即可翻案。
>
> **修订（2026-09-13，会话 fable-5-1-47 审核后）**：09-07 落盘至今两仓一行代码未动（pid-parse 自 `1f2060c` 无提交，
> `src` 里 `0x0057` / ViewFilterSet 零引用，`0x0115` 只在 `undecoded_census.rs` 里作丢弃计数；OCS 无 `PID_LAYER_MODE` /
> `PID_VIEW_FILTER`，图层管理器无图纸图层模式）——09-07 之后的工时全进了 DXF P&ID 识别线。本次改三处：
> ① **D8** XDATA 键名 `class=` 与既有实现撞车，改 `role=`；② **D7** 执行顺序改为 L2 先行（用户可见价值前置）；
> ③ **T** 项扩成台账清单（已做 / 待做）。基线复核：OCS `--test pid_import` 35/35、pid-parse `--lib` 1094 + `parse_real_files` 127 全绿。
> 状态：**Plannotator 批准**（2026-09-13 22:43，`{"decision":"approved"}`，无批注；见文末「门禁记录」）。
>
> **补记（2026-09-14，会话 fable-5-1-5 对账后）**：L2 第 1 步落地（OCS `39b1cc72`），其余各项未动。对照代码把 L2 的
> 三处口径补严，均不动决策表：① 识别线与导入线的边界补上 **attach** 那一半（原只写了 PURGE 与「不写 role」）；
> ② L2 第 4 步新词条的覆盖面从「四语言」改为 **21 本目录全覆盖**（`i18n` 的目录守护测试如今对全部目录生效）；
> ③ L2 第 2 步可见性公式补上**没有图纸图层的实体**怎么算。基线复核：OCS `--test pid_import` 38/38、
> pid-parse `--lib` 1094 + `parse_real_files` 127 全绿。
>
> **补记（2026-09-14 晚，会话 fable-5-1-14）**：**L2 四步全部落地**（`39b1cc72` → `f082393d` → `706e4eb7` → `018314a6`），
> user-guide `.pid` 一节随 L2 出口写入；`pid_import` 44/44。按 D7 下一项是 **L1**（pid-parse 解 `0x0057` 显示状态，
> 落地后只换 `is_hidden_sheet_layer` 一处初值）。
>
> **补记（2026-09-14 夜，会话 opus-5-24）**：**L1 落地**（pid-parse `132604f` + `b61de88`，OCS 消费端 `cf137fb0`）——
> `0x0057` 的长度账 53/53 做平，显示状态是按图层号索引的位图，图层显隐从此走文件事实，名字判据降为兜底
> （D6 的第一个分支成立）；`Dimension` / `Construction` 在定义缓存里实测为关，**D2「默认不画」得到文件背书**。
> `pid_import` 44 → 45，pid-parse `--lib` 1094 → 1097、`parse_real_files` 127 → 128。按 D7 下一项是 **J1**。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| D1 | JDim 是什么 | **不是页面标注**。18 条（四主图 14 + A01 4）**全部**在符号定义缓存 `JSite*/PSMcluster0` 里、全部坐在名为 `Dimension` 的图层上，是参数化符号的**驱动尺寸**（`0x006F` Standard Relation 的出参 13/13 都是它）。工作项按「解码 + 参数化链闭环 + 显示决策」定义，不按「画 14 条标注」定义 | ⭕ |
| D2 | JDim 画不画 | 默认**不画**：它在定义缓存的 `Dimension` 层上，与 `Construction` 同属定义内部构造几何；是否上屏由 L1 解出的视图过滤集显示状态裁决，不由名字猜。解码结果进 `JSiteNestedGeometry::dimensions` 作证据 + 进 OCS 特性面板 / 导入摘要 | ⭕ |
| D3 | JDim 取证路线 | 先 `imagdex.dex` 原生读取器（08-31 的 `tools/idalib_imagdex_*.py` 现成，按 `JDim Object` vtable slot 3 → `DoIO` 反编译），字节统计只做互证；`radsrvitem.dll!sub_564BA320`（igDimension 277）作第二对照。**没有原生读法坐实的字段不进 DTO**（保持 `raw`），与曲线族同一纪律 | ⭕ |
| D4 | 图层槽归谁 | **分两步**：先做「分类过滤机制 + 图层管理器的图纸图层视图」（L2，不动槽），再把槽换成图纸图层（L3，导入选项后置切换）。两步共用同一套按 XDATA 分类的逐实体可见性机制；L3 是否本轮做完见 D5 | ⭕ |
| D5 | L3 范围 | 本轮**做到「选项可切、默认不切」**：`OCS_PID_LAYER_MODE=sheet` 时槽=图纸图层名（原样，不加前缀），taxonomy 退到 XDATA `role=` / `style=`；探针 / 测试 / 图例线跟着改成按 role 取；默认值翻转另开一轮，等 DXF 下游消费方的要求定下来。变量名与 `OCS_PID_LEGEND_RULES` 同前缀（09-13 改，原写 `PID_LAYER_MODE`） | ⭕ |
| D6 | 图层显隐的事实源 | `0x0057 Top ViewFilterSet` 的显示状态字节（08-27 记为「`FF 02 00 …` 一段」，未解）→ 解出后**替代**现在按名字猜的隐藏类（`Hidden` / `HiddenObjects` / `Invisible`）；解不出则名字判据保留，登记缺口。**2026-09-14 结算**：解出（那「一段」是位图的 `FF` + u16 长度头），第一个分支成立——文件状态当判据、名字判据降为兜底（`PidSourceLayer::displayed` 为 `None` 时才用）；语料里唯一翻案的是 `Invisible`（文件说显示），无画出实体，输出不变。见 L1 进度 | ⭕ |
| D7 | 执行顺序 | ~~L1 → J1 → J2 → L2 → J3 → L3 → 台账~~ **2026-09-13 改为 L2 → L1 → J1 → J2 → J3 → L3 → T**：L2 是本轮唯一用户看得见的项，且自带「L1 未到之前用名字判据」的兜底，不必等 L1；J 线按 D2 默认不画，产出是证据与特性面板字段，排在 L2 之后不卡屏幕。一项一提交，先红后绿 | ⭕ |
| D8 | 分类维的 XDATA 键名（2026-09-13 审核发现） | 原稿 L2 要写 `class=`（geometry / text / symbol / …），但 `PID_SEMANTICS` 记录里 **`class=` 已被占用**：`src/io/pid.rs::attach_pid_metadata` 写的是 `_Data.xml` 语义对象的元素名（`PIDPipeline` / `PIDProcessVessel` …），特性面板读它当「类型」；DXF 识别线 W8 的 `pid_legend/xdata.rs` 也往同一记录写 `class=<识别类>`。照原稿写下去两种语义互相覆盖。**改用 `role=`**（值不变：geometry / text / symbol / symbol-label / point-ok\|warning\|error\|approved / annotation / connectivity / fill / frame），`style=` 照旧；术语「分类」相应改「角色（role）」 | ⭕ |

## 背景

### 线一 · JDim（0x0115）

`0x0115` 在 RAD 类表里叫 `JDim Object`，`ig*` 类表里对应 `igDimension`(277)，在原生「是不是图形」谓词
里（08-04 `annotation-families-risk`）。08-04 那篇判「全语料 0 命中、不建议写解码器」——那时只查了顶层
`Sheet*`；08-27 aux_hi 名册在嵌套存储里数出 14 条。**09-07 逐条实跑**（四主图 + A01）：

| 图 | 存储 | 条数 | 所在图层（该存储的 `JSheetLayer`）| payload 长度 |
|---|---|---:|---|---|
| D06 | `/JSite145/PSMcluster0` | 5 | 35 `Dimension` | 198 / 308 / 276 / 198 / 166 |
| DWG-0201 | `/JSite329/PSMcluster0` | 5 | 44 `Dimension`（3，parent = sheet 49）、507 `Dimension`（2，parent = sheet 501）| 194 / 308 / 198 / 194 / 194 |
| DWG-0202 | — | 0 | — | — |
| 工艺管道及仪表流程-1 | `/JSite7559/PSMcluster0` | 4 | 102 `Dimension` | 288 / 198 / 198 / 288 |
| A01（导出件）| `/JSite39/PSMcluster0` | 4 | 150 `Dimension` | 194 / 198 / 308 / 194 |

字节上已经看得见的（**未坐实，只作 J1 的起点**）：18 字节子头与七个图元族相同（oid / parent_ref =
所在定义 sheet / 图层 / `sub_type 0` / `index 1`）；`+18 u32 2`；`+26` 一个标志字（`0x0351` / `0x0051` /
`0x0341` / `0x0361`）；`+30 u32` = 主块长度（198 / 308 / 276 / 166 / 288 五种恰等于 `payload − 38`，194
那种少一个尾部 `u32`）；`+42 f64` 全是**英寸整倍数的长度**（3.81 / 10.16 / 12.7 / 20.32 / 25.4 / 35.56 /
63.5 / 114.3 mm）= 尺寸值；`+108/+116`、`+124/+132`、`+146/+154` 三对 f64 = 符号本地坐标点（同一记录里
三对几乎相同，是尺寸的两端 + 文字位）；`+162/+170 = ±1.0` 轴向；308 / 288 的长形带第二个块，工艺图两条
末尾 `+272 = π/2`；198 形比 194 形多一个尾部 `u32` oid（0x1C / 0x1D / 0x30 / 0x58 / 0x44 / 0x45 / 0x5D）。

它在参数化链上的位置（08-27 `the-recordless-182-referrers-are-symbolinformation.md` §4.3）：

```text
JSymbolInformation（0x00BD）给 Double Value（0x00C7）起名 Left / Right / Top / Bottom
Standard Relation（0x006F）：出参 13/13 = JDim（0x0115），入参 = Double Value；公式 0E$1 / 0E$1+0.01 / …
spacemap tag 188：46 条成员，42 条在 0x0115 payload 里找到 → JDim 引用它约束的几何
```

所以 `DWG-0202` 一条都没有不是巧合——它没有参数化符号。09-07 上午发现的 `Parametric Manifold`
缓存本体 35.59 mm（库默认 20.32）就是这条链算出来的，JDim 解开才能把「按实例参数重算」从推断变成证据。

另有一条顺手可能收的账：Phase 33 那 638 条 `0x0010` 子记录的 CLSID 与 `0x0115` 同一个 GUID
（`1D1928C0-…-46`，`0x0010` 的 parent alias = `0x0115`）。JDim 的 `DoIO` 读到哪里、`0x0010` 的语义
就可能跟着落地；**不作本轮验收项**，只登记。

### 线二 · 图层面板

OCS 图层管理器（`src/ui/window/layers.rs`）是一张平表：名 / 开关 / 冻结 / 锁定 / 打印 / 颜色 / 线型 /
线宽 / 透明度 + 按视口冻结，可按列排序，有搜索框（#343），有图层状态（Layer States）。它展示的是
`document.layers`——`.pid` 导入后那就是 15 个合成层（`PID-GEOMETRY` / `PID-TEXT` / `PID-SYMBOL` /
`PID-SYMBOL-LABEL` / `PID-POINT(-WARNING/-ERROR/-APPROVED)` / `PID-ANNOTATION` / `PID-CONNECTIVITY` /
`PID-FILL` / `PID-FRAME` / `PID-HIDDEN`）+ 按样式名生成的 `PID-STYLE-*`。图纸自己的图层名只在 XDATA
（`sheet_layer=` / `sheet_layer_oid=`）与特性面板 P&ID 组里；隐藏类三名（`hidden` / `hiddenobjects` /
`invisible`）按**名字**归 `PID-HIDDEN` 默认关（四图 11 / 16 / 2 / 16 个实体）。

pid-parse 侧已有的事实：`JSheetLayer` 290/290 按存储解码、manager 对账；同名图层是多份对象，**每个
视图过滤集一份**（D06 `JSite145` 四个 `Default`）；`0x0057 Top ViewFilterSet` 49 条带一段**显示状态字节
未解**；`Default` 从不被 184 边指（49/49，「只登记偏离默认的图层」是未验证的读法）；`JSheetLayerGroup`
11 条未碰。**图层的显隐在文件里有事实，我们现在用的是名字。**

一个实体只有一个 layer 槽，这是 D2 第 3 步当年暂缓的原因：taxonomy 上挂着评审状态一键开关
（`PID-POINT-*`）、符号标签默认隐藏（`PID-SYMBOL-LABEL`）、discipline 分层（`PID-STYLE-*`）、
诊断层（`PID-CONNECTIVITY` / `PID-FRAME`）。消费 `PID-*` 名字的有：`src/io/pid.rs`、
`tests/pid_import.rs`（29 处 `on_layer` / `is_line_work`）、`examples/pid_probe.rs`、
`examples/pid_plot_dump.rs`；图例线（`pid_legend.rs`）用自己的 `PID-LEGEND-*`，不受影响。

## 目标

一句话：**JDim 从盲区变成有名有姓、有值有边的记录，参数化链在证据上闭环；图层管理器里看到
SmartPlant 自己的图层与它自己的显隐，而现有的一键开关一个不丢。**

验收基线：pid-parse `cargo test --all-targets` / clippy `-D warnings` / fmt / missing-docs 四门全绿；
OCS `pid_import` + `pid_panel_localization` 全绿；数值变更随 analysis 文档；每个新家族有 per-fixture
计数棘轮；pid-parse 每项完成后回跑 OCS `pid_import`。

---

## 工作项

### L1 · 视图过滤集显示状态解码（pid-parse）

**现状**：`0x0057 Top ViewFilterSet` 已知 `+0 oid / +12 形式号 / +16 JSheet / +20..+28 常量 / +32 未认`，
之后「一段显示状态字节 + 若干 {u32 字符数; UTF-16 图层名}」，49 条长度 160..490，未解码。
`0x0060`（每存储一个）恒 28 字节只有计数。

**改法**：① 先定长度账：名字表从哪个偏移起、显示状态段恰好多长（每图层一字节？一位？），四主图 + A01
49 条全部精确收尾才算读法成立；② 找 `viewfil.dex` 的 `IJPersist::DoIO`（同 08-31 的 idalib 路子）坐实
字段名；③ 解出 `(视图过滤集, 图层名) → 显示状态`，挂到 `PidDocument`（按存储），并回答 §6 那个空缺：
`Default` 是不是「未登记 = 显示」的基线；④ 顺带读 `JSheetLayerGroup`（11 条）的成员表，看是不是
面板分组的现成事实。

**验收**：49/49 收尾；四主图里名为 `Hidden` / `HiddenObjects` / `Invisible` 的层显示状态实测为「关」
（这是现有名字判据的对照组，若实测不为关，名字判据与文件事实谁对要写进分析文档）；`Dimension` /
`Construction` 在符号定义缓存里的状态有明确读数（供 D2 裁决）；棘轮 + 分析文档
`2026-09-xx-viewfilterset-carries-the-layer-display-state.md`。

**风险**：显示状态可能不在 `0x0057` 里而在 `SheetView`（`0x0076`）或别处——①的长度账做不平就换目标，
不硬凑；时间盒一个工作日，无果按 D6 保留名字判据、登记缺口。

**进度**：✅ 2026-09-14 pid-parse `132604f`（`feat(layers): read each sheet's layer display state from its Top ViewFilterSet`）
+ `b61de88`（分析文档 `2026-09-14-viewfilterset-carries-the-layer-display-state.md`、guide 3.4、CHANGELOG、
task_plan 指针）与 OCS `cf137fb0`（`pid: the file's own display bit decides which sheet layers an import starts with switched off`）。
**① 长度账做平**：`+32` 是**活动图层号**（恒 = `Default` 的图层号，53/53——08-27 记「没认」的就是这个字）；
`+38` 起 6 张**按图层号索引的位图**（各自带 `FF` + u16 长度：第一张显示、第二张读作可定位、后四张恒全 1 无读法）；
随后逐图层显示覆盖（认领状态层灰显，`0.18 mm` 细线）、12 个 0 字节、名字表——**名字后的 u16 是图层号，不是间隙**。
四主图 49 条 + A01 4 条 **53/53 精确收尾**，多一字节少一字节都没有。**② 原生读取器没读**（`viewfil.dex` 未碰）：
位图含义靠语料对照组读出，等级 **corpus**，是对 D3「没有原生读法坐实的字段不进 DTO」的一处让步，
覆盖项的 `kind` / 尾字与位图 3–6 按 raw 留着，已登记在分析文档「还开着的」。**③ 条目落到对象**：经
「集合的 JSheet → 登记它的 `JSheetLayerManager`（tag 183）→ 同名同号的图层」**309/309 各落唯一对象**
（`parent_ref` 走不通，0/309）；答案写进 `SheetLayer::displayed` / `locatable` / `view_filter_set_oid`、
`PidDocument::view_filter_sets`（按存储）与每个几何实体的 `PidSourceLayer::displayed`。**④ `JSheetLayerGroup`**
16 条逐字节同形，每存储一个单成员 `Default` 组——**不是分组事实**，面板不用它。
**对照组**：顶层 `Hidden` / `HiddenObjects` 10/10 关、`Default` 53/53 开、定义缓存里 `Dimension` / `Construction`
各 11/11 关（**D2「默认不画」由文件背书，不必翻案**）、`Label` / `Jacket` / `Heat Trace` 在定义里关；
**唯一分歧是 `Invisible`**——两个定义缓存里实测为**开**，名字判据说隐藏，文件说显示；语料里没有画出的实体
落在它上面，OCS 输出因此不变（分歧写进分析文档 §3）。§6 那个空缺（`Default` 从不被 184 边指）也有了形状：
显示状态是每层一位的位图、不是「只登记偏离基线者」的清单，184 边不再是显示状态的载体候选。
**消费端（OCS）**：新函数 `pid.rs::sheet_layer_is_hidden` 一处裁决——`PidSourceLayer::displayed` 有值听文件，
没有才落回 `is_hidden_sheet_layer` 名字判据（名字判据降级为兜底，文档随之改口）；
`PidViewFilter::initial`（扫成图文档按名字取）删掉，改由导入归层时顺手收集 off 集合交给新的
`with_layers_off`——初值只有这一条路。**`PID-HIDDEN` 归层照旧不动**：L2 第 3 步的开关依赖「开层连带放开 PID-HIDDEN」，
按 L3 的口径退役（L2 第 2 步那句「这一步落地那天归层逻辑就可以退役」与执行顺序里的同一说法，一并推迟到 L3，理由记在此）。
**验证**：pid-parse `--lib` 1094 → 1097、`parse_real_files` 127 → 128（新棘轮
`view_filter_sets_state_each_sheets_layer_display_and_close_exactly`），金样只多 `displayed` 一个字段重封，
解码器进 panic-safety 套；OCS `pid_import` **44 → 45**（新断言：过滤初值集合 = 文件说关的那些层，
且逐实体与 `PID-HIDDEN` / `invisible` 对齐，三张图各跑一遍）、`pid.rs` 新增单测一条、模块单测 6 条改口径，
`cargo clippy --lib --bins --tests` 零告警、rustfmt 改动文件干净（`tests/pid_import.rs:101` 是 HEAD 既有旧账）。
`cargo test --lib` 1150 过 / 3 失败——`app::automation::a_dry_run_plot_writes_nothing`、
`pidlegend::an_svg_plot_groups_a_tagged_symbol_with_its_tag`、`svg_export::a_page_style_table_wins_over_the_job_wide_one`
**单跑全绿**，是字形图集并行时的老飘忽（09-14 上一轮已记两条），与本项无关。**UI 未手工点过**。

### J1 · JDim 字节取证与原生读取器（pid-parse）

**现状**：见背景表；18 条、六种长度、字段只在字节上看得见形状。原生侧两条入口：`imagdex.dex` 的
`JDim Object`（08-04 注册表把 `0x0115` 的 GUID 映到它）与 `radsrvitem.dll!sub_564BA320` 的五级
igDimension reader（08-04 已列出 `+18` / `+20` / `+32` 位域读法）。

**改法**：① 探针 `examples/probe_jdim_bytes.rs` 把 18 条按 D06 / 0201 / 工艺 / A01 逐字段摆开（09-07
临时探针的输出就是它的第一版）；② `tools/idalib_imagdex_jdim.py`：沿 08-31 的 vtable → slot 3 → `DoIO`
路径反编译 `JDim Object` 的读取器，得到字段读序与版本分支（`+18 u32 2` 疑为版本号，`+30` 疑为主块
长度）；③ 与 `radsrvitem` 的位域读法互证 `+26` 标志字；④ 写分析文档，明确到「哪些字段坐实、哪些留
raw」。

**验收**：六种长度全部按同一读法精确收尾；尺寸值（`+42`）与三对点（`+108..+154`）的语义有原生读法
背书；tag-188 那 42 条命中能在读法里落到具名字段（引用槽），剩 4 条有解释。

### J2 · JDim 解码器与缓存本体证据（pid-parse）

**现状**：`decode_nested_geometry` 只试七族解码器，JDim 记录在缓存里**静默跳过**，任何 warning 都不提它。

**改法**：`decode_jdims` 走记录链门（`sheet_record_starts`），`SheetIgDimensionDecoded` / DTO 只带坐实
字段 + `raw_tail`；挂 `JSiteNestedGeometry::dimensions`；`SHEET_RECORD_FAMILIES` 加一行
（`emits_geometry` 按 D2 = false，`trace_class` Decoded），`DECODED_TYPE_CODES` 13 → 14；缓存本体的
`PidSymbolDefinition` 带 `dimensions: Vec<…>`（值 + 两端点 + 所在图层），**不进 `primitives`**；
`build_normalized_geometry` 的缓存 warning 行加 `{dimensions} dimensions`；`parser_panic_safety` 收入口；
`render_gap_census` 若受影响重钉。

**验收**：棘轮 `jdims_are_the_driving_dimensions_of_parametric_bodies`：D06 5 / 0201 5 / 0202 0 / 工艺 4
（A01 4 软跳）；每条的 `parent_ref` 就是某个放置点名的定义 sheet（或未被点名的模板 sheet，如 `/JSite329`
sheet 49）；尺寸值集合 = 英寸整倍数；golden 只多字段不改实体。

### J3 · 参数化链闭环（pid-parse，证据）

**现状**：`0x006F` 已解（公式 + 操作数 oid），`0x00C7` 已解（值 + 名），JDim 是链上唯一没解的节点；
`Parametric Manifold` 的 35.59 mm 只是「对不上库默认」的推断。

**改法**：探针 `probe_parametric_chain_resolves_a_cached_body`：对 0201 `/JSite396`（Manifold 实例，
Imagineer Document）与 `/JSite329` sheet 49（Manifold 模板，Server Document）各取 JDim 值、Double Value、
公式，验证 **模板 JDim = 20.32、实例 JDim = 35.59** 且实例几何（两条 r 35.59 弧）落在 JDim 两端点上；
把 tag-188 边解释为「JDim → 被约束图元」并按 oid 对上具体 `igLine2d` / `igArc2d`。结论进
`2026-09-07-placement-tail-names-the-cached-definition.md` 的「参数化」一段与 CHANGELOG。

**验收**：一条棘轮 `a_parametric_instance_carries_its_own_dimension_values`（0201 Manifold；D06 Cone Roof
Tank 若同理则一并钉）；不改任何投影输出。

### L2 · 分类过滤机制 + 图层管理器「图纸图层」视图（OCS）

**现状**：图纸图层只在 XDATA / 特性面板；图层管理器只有一张 `document.layers` 平表；`common.invisible`
逐实体可见位已存在（命令驱动里有先例）。

**改法**：
1. **导入端**把两维都写全：现有 `sheet_layer=` / `sheet_layer_oid=` 之外加 `role=`（geometry / text /
   symbol / symbol-label / point-ok|warning|error|approved / annotation / connectivity / fill / frame；
   **不是 `class=`**，那个键已被语义对象类占用，见 D8）与 `style=`（样式名，discipline 的来源）。零破坏。
2. **过滤机制**：文档级「P&ID 视图过滤」（按存储的图纸图层 on/off 集合 + role on/off 集合），应用 =
   对每个带 P&ID XDATA 的实体求 `invisible = !(sheet_layer_on && role_on)`；**没有 `sheet_layer=` 的实体
   `sheet_layer_on` 视为 on**，只受 role 开关管——导入器自造的 frame / connectivity / symbol-label 和
   StyleCluster 字形线（`sheet_layer_ref == 0`）本来就不在任何图纸图层上，不能因为「没登记」被关掉；
   状态存文档 XDATA 记录（`PID_VIEW_FILTER`），DXF 往返后 `invisible` 位与记录同在。**初值先用现有名字判据**（`Hidden` /
   `HiddenObjects` / `Invisible` 三名 off）；L1 落地后换成解出的显示状态，只改初值函数一处——这一步落地
   那天 `PID-HIDDEN` 的归层逻辑就可以退役成「`Hidden` 层 off」。
3. **图层管理器**加一个模式切换（合成层 / 图纸图层）：图纸图层模式列出该图的图层名（去重跨视图过滤
   集，按存储折叠，默认只看顶层存储）、实体计数、on/off 开关；role 作为第二段（复选）；搜索框沿用。
   Layer States 不动。
4. 特性面板 P&ID 组加「角色」行（读 `role=`，与既有「类型」= `class=` 并列，两行都在时都显示）；
   导入摘要加「图纸图层 N 个，其中 M 个按文件状态关闭」。

**验收**：`pid_import` 新断言：每个 P&ID 实体带 `role=`，且带 `class=` 的实体两键并存、值域不交叉
（`role` 值 ∈ 上面那十个词，`class` 值 ∈ `_Data.xml` 元素名）；0202 在 XDATA 记录里关掉 `Labels` 后其
text 实体 `invisible`、开回来恢复；新词条（「角色」行等）**21 本 `locales/*/opencadstudio.ftl` 全覆盖**——
`i18n::tests::every_catalog_covers_and_formats_the_source_catalog` 对全部目录生效，少一本就红（09-14 改，
原写「`pid_panel_localization` 四语言」，那是 08-12 W5 的口径，早已过时）；手工验收：打开 0202，切到
图纸图层模式，看到 `Default / Labels / HeatTrace / …` 与计数，勾掉 `HeatTrace` 电伴热线消失。
**与 DXF 识别线的边界**：`pid_legend/xdata.rs` 只写 `class` / `label` / `lines` / `resolved`，不写 `role`；
识别线的 `PIDLEGEND PURGE` 清 XDATA 时只认 `resolved=legend:*` 的记录，`.pid` 导入写的记录不受影响——
L2 加断言钉住这两条，防止两条线以后互相踩。**attach 那一半（09-14 补）**：`pid_legend/xdata.rs::attach`
遇到已有非 `legend:*` 记录的实体一律跳过、不覆盖；第 1 步之后每个导入实体都带 `role=`，于是
`PIDLEGEND ON` 在 `.pid` 导入的图上**不再写任何 XDATA**（此前只有既无发布身份、又无图纸图层的实体可写）。
这是有意为之：覆盖等于丢 `role=`，随后 PURGE 会把整条记录删光。识别线是给 DXF 图的，`.pid` 导入自带身份，
两者不在同一张图上工作；若日后要让识别线在 `.pid` 图上补位号，改法是**合并记录**（保留导入键、追加识别键、
`resolved` 归识别线）而不是放开跳过，另开一项做。
**进度**：第 1 步 ✅ 2026-09-14 OCS `39b1cc72`（`pid: every imported entity states its role= and style= in XDATA`）：
`role=` 从实体建于其上的 `PID-*` 层读出、在 `PID-HIDDEN` 覆盖之前取；`style=` 不论是否换来 `PID-STYLE-*` 层都写；
APPID 注册改无条件；页面边框带 `role=frame`。`tests/pid_import.rs` +3 条（词表 / role 与层一致 / role-class 值域不交叉、
无 `legend:*` 记录 / `style=` 与 discipline 层互推）+ DWG/DXF 往返保 `role=` 的断言，35 → 38 全绿。
第 2 步 ✅ 2026-09-14 OCS `f082393d`（`pid: a stored view filter switches sheet layers and roles off, entity by entity`）：
新模块 `src/io/pid_view_filter.rs`——`PidViewFilter { layers_off, roles_off }`，存为 `*Model_Space` 块记录扩展字典里的
XRecord `PID_VIEW_FILTER`（每条 `layer_off=<名>` / `role_off=<角色>` 一个字串项；全开时删记录不留空账），`apply` 按
`invisible = !(sheet_layer_on && role_on)` 逐实体置位，只碰带 `sheet_layer=` 或 `role=` 的记录；导入末尾 `initial → store → apply`，
初值走 `is_hidden_sheet_layer`（从 `pid.rs` 搬进新模块，`PID-HIDDEN` 归层与过滤初值共用这一个函数，L1 只换它）。
`tests/pid_import.rs` +4 条：导入即存过滤且 HiddenObjects 全暗、**0202 关 `Labels` → 46 条 text（连同 46 条线 + 5 个填充）暗、
`Default` 照画、开回来全亮**、role 轴与无图纸图层实体只听 role、记录与 `invisible` 位过 DWG/DXF 往返；38 → 42 全绿。
**过渡期注意**：隐藏类实体现在同时被 `PID-HIDDEN`（层关）和 `invisible` 位遮住，只开层不再能看见它们，要等第 3 步的开关
（或一条命令）才能翻回来。
第 3 步 ✅ 2026-09-14 OCS `706e4eb7`（`layers: the Layer Manager shows a .pid import's own sheet layers and roles, each with a switch`）：
`pid_view_filter.rs` 加 `PidViewSummary`（按名去重的图纸图层 + 词表序的 role，各带实体计数与开关状态）和
`switch_sheet_layer` / `switch_role`（读记录 → 翻一个名 → 存回 → `apply`）；`ui/window/layers.rs` 加 `LayerView::{Layers, SheetLayers}`
与工具栏切换（只在图有 P&ID 语义时出现），图纸图层视图 = 名 / 计数 / 眼睛，`Roles` 第二段，搜索框沿用，New / Delete / Set Current 在该视图隐藏；
`Message::{LayerViewSet, PidSheetLayerToggle, PidRoleToggle}`，每次开关一个撤销步（文档快照，XRecord 与位一起回退）；摘要在开图 /
撤销重做 / 开窗 / 进视图时重读。**过渡期的解法**：把初始关闭的图纸图层开回来时，连带把 `PID-HIDDEN` 层打开（只在「开」且点亮了
落在该层上的实体时；「关」从不碰图层表），上面那条过渡期注意随之解除。**验收口径的一处偏差**：四个 fixture 画出的实体只落在
`ConsistencyChecks` / `Default` / `HiddenObjects` / `Labels` 四个原图图层上，计划里举例的 `HeatTrace` 在这批图里没有画出的实体、
不会列出；测试改用 `ConsistencyChecks`（0202 上 37 个）代替。新词条 5 条 21 本全覆盖。`pid_import` 42 → 44，模块单测 4 → 6。
第 4 步 ✅ 2026-09-14 OCS `018314a6`（`pid: the Properties panel shows an entity's role, and the import summary counts the sheet layers by name`）：
特性面板 P&ID 组加「角色」行（读 `role=`，紧跟「类型」，两键都在时两行都显示，只有 `role=` 的自造实体也有该组）；导入汇总加第二行
`P&ID sheet layers: N, of which M start switched off`（`ImportSummary` 新增 `sheet_layer_names` / `sheet_layers_off`，从 `PidViewSummary`
取，与图层管理器所列一致；原按存储 oid 计数的第一行不动；汇总改在过滤存入并应用之后取）。新词条 2 条 21 本全覆盖。
**L2 出口**：user-guide 补「打开 Smart P&ID（.pid）图纸」一节（只读来源与另存、两行汇总、符号库、`PID-*` 合成层表、图纸图层视图、
特性面板 P&ID 组），T 项对应条目转「已做」。

### L3 · 图层槽切换到图纸图层（OCS，选项后置）

**现状**：槽 = 合成 taxonomy；四处消费者按 `PID-*` 名字取实体。

**改法**：导入选项 `OCS_PID_LAYER_MODE`（`taxonomy` 默认 / `sheet`；环境变量，与 `OCS_PID_LEGEND_RULES`
同一命名法）。`sheet` 模式：实体 layer = 图纸图层名**原样**（`Default` / `Labels` / …，按 D5 不加前缀；
同名跨存储合并为一个 DXF 层，oid 仍在 XDATA）；没有图纸图层的 OCS 自造实体（连通链、符号名标签、
评审点符号）保留各自 `PID-*` 层；`PID-STYLE-*` / `PID-HIDDEN` 不再生成（discipline 走 `style=` + L2 过滤，
隐藏走文件状态）；`ensure_layer` 用 L1 的显示状态设初始 on/off。消费者改造：`pid_import.rs` 的
`on_layer` / `is_line_work` 改成按 XDATA `role=` 取的 `of_role`（两种模式下同一套断言都得过）；
`pid_probe` / `pid_plot_dump` 按 role 分组输出。

**验收**：两种模式下 `pid_import` 全绿；`sheet` 模式打开 0202：图层表就是 SmartPlant 的 16 个名字
（含状态），DXF 另存后在第三方查看器里图层名一致；默认值不翻转（D5）。

### T · 台账与共享记忆（双仓）

**已做（2026-09-13，收账一轮）**：

- 08-29 计划入库：OCS `a1ad4f2c`——此前一直是未跟踪文件、文内无进度；现头部有终态段（两仓提交号逐项）、
  W6 有进度行，OCS 3 条 `app::automation` 红测试的去向（上游 issue #941）第一次出现在 OCS 仓里。
- 两仓换行符归一：pid-parse 新增 `.gitattributes`（`* text=auto eol=lf` + 二进制夹具，`cd20e3b`）；
  两仓工作树 322 + 515 个 CRLF 文件逐字节改回 LF（blob 哈希与 HEAD 相同，无提交）；根因是系统级
  `core.autocrlf=true`，已用用户级 `git config --global core.autocrlf false` 压掉。

**已做（2026-09-14，L2 出口）**：

- **user-guide `.pid` 一节**：`docs/user-guide.md` 新增「打开 Smart P&ID（.pid）图纸」（放在「管理图层和属性」与
  「校正 P&ID 图例」之间）——只读来源与 Save As 闸门、两行导入汇总、符号库查找与 `PID_SYMBOL_LIBRARY`、`PID-*` 合成层表
  （含初始开关）、图纸图层视图与 role 开关、特性面板 P&ID 组（类型 / 角色 / 位号 / 管线号 / 匹配方式 / 图纸图层 / 图层 OID）。
  L3 落地时再补 `OCS_PID_LAYER_MODE`。

**待做（随本轮各项收尾）**：

- pid-parse `task_plan.md`「当前阶段」加本轮指针（L1 已刷，随 `b61de88`；J2 落地时再刷一次）。
- user-guide 在 L3 落地时补 `OCS_PID_LAYER_MODE` 一段。
- OCS `.context` 会话文件按惯例；每项落地 `remember` 一条。

---

## 执行顺序 ⭕D7（2026-09-13 修订）

**L2 → L1 → J1 → J2 → J3 → L3 → T**。

- **L2 先行**：本轮唯一用户看得见的项（图层管理器里出现 SmartPlant 自己的图层与开关），不依赖 J，
  也不必等 L1——初值用现有名字判据，L1 到了只换一处初值函数。D8 的键名改动在 L2 第 1 步落地。
- **L1 紧随**：解出显示状态就把 L2 的初值换成文件事实，`PID-HIDDEN` 归层退役；时间盒一个工作日，
  无果按 D6 保留名字判据。**2026-09-14 结算**：一个工作日内落地（pid-parse `132604f` + `b61de88`、OCS `cf137fb0`），
  初值已换成文件事实；`PID-HIDDEN` 归层**没有**退役——L2 第 3 步的开关靠它，退役并入 L3。
- **J1 → J2 → J3**：把 JDim 从盲区拿出来并闭环参数化链。按 D2 默认不画，所以排在屏幕价值之后；
  J1 两个工作日时间盒，原生读取器无果则 J2 只带坐实字段 + raw，不空转。
- **L3 最后**：选项后置、默认不切，风险隔离。
- **T** 随各项收尾，不单独占期。

原稿 L1 → J1 → J2 → L2 的理由是「J 的 D2 裁决与 L 的显隐初值都吃 L1」——D2 裁决只影响 J 线画不画
（默认不画，L1 之后若实测 `Dimension` 层为显示再翻），L 的初值有兜底；两者都不构成 L2 的前置。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 把 JDim 画成尺寸标注实体 | D1/D2：它是定义内部驱动尺寸，SmartPlant 自己也不在放置上画；除非 L1 实测 `Dimension` 层为显示 |
| `JBalloon`（0x0117）/ `JLeader`（0x0118）/ `0x00FF` | 全语料 0 条，无 fixture 不写；08-07 的图形类点名告警已覆盖 |
| `0x0010` 子记录语义（638 条） | 与 JDim 同 GUID，可能随 J1 顺带落地，但不作验收项 |
| 按视图过滤集分别呈现图层状态 | OCS 单模型空间，取一份（顶层存储、第一个集合）；多视图是另一个产品命题 |
| `OCS_PID_LAYER_MODE` 默认翻转 | 等 DXF 下游消费方（图例线那批 DXF 的用法）把要求说清 |
| 缓存 vs 库本体优先级、缓存 `StyleCluster` 接入 | 09-07 上午登记的两条，与本轮无耦合，另排 |
| A01 `/JSite204` `Default` 计数差 4 | 未解释记账，等新证据。（`0x0057 +32` 那一半 L1 顺带解了：活动图层号；A01 那两个 `Default` 是嵌套「Imagineer Document」正文的层，文件里没有它们的显示状态） |

## 术语

- **驱动尺寸（driving dimension）**：参数化符号定义里约束几何的尺寸对象（`JDim`），值由公式
  （Standard Relation）从命名变量（Double Value）算得；放置实例按它重算本体。与页面上的标注
  （annotation）不是一回事。
- **视图过滤集（ViewFilterSet）**：`0x0057` / `0x0060`，按视图存的图层选集与显示状态；「图纸图层
  显隐」的事实源。
- **角色（role）**：OCS 导入器给实体的角色标签（现在体现为 `PID-*` 合成层名），L2 起以 XDATA
  `role=` 存在，与图纸图层正交。**不叫「分类 / class」**：`class=` 在同一 XDATA 记录里已是语义对象的
  XML 元素名（`PIDPipeline` …），特性面板显示为「类型」（D8）。
- **图层槽（layer slot）**：DXF 实体唯一的 layer 字段；D4/D5 讨论的就是它归哪一维。

## 门禁记录

- 2026-09-13 22:43：修订版（D7 改序、D8 换键、T 扩充）过 Plannotator `annotate --gate --json`，
  **`{"decision":"approved"}`，无批注**。⭕ 均未被翻案，D1–D8 按各自「结论」执行；09-07 原稿此前未过门禁，
  本次是这份计划第一次批准。
