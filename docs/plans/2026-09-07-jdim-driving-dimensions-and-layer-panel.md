# JDim 驱动尺寸与图层面板重构 · 开发计划（2026-09-07）

> 承接 `2026-08-29-real-sheet-layers-and-jsite-geometry.md`（W1–W6 已于 08-31 落地；W4 于 09-07
> 以「嵌套 `LdcSite` = 内嵌符号定义缓存、变换在放置记录里」关闭，四曲线族解码器齐全）。
> 该计划「登记不做」里留下两条：**`JDim`（0x0115）标注解码**与**图层面板重构**（D2 第 3 步）。
> 本计划把这两条接过来。事实基础是 09-07 下午对四主图 + A01 的字节实跑（JDim 18 条逐条列出）、
> pid-parse 08-27 的 spacemap / tag-184 / tag-188 分析，以及 OCS 图层管理器现状。
> 带 ⭕ 的决策按推荐落笔、未经拍板，Plannotator 批注里划一笔即可翻案。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| D1 | JDim 是什么 | **不是页面标注**。18 条（四主图 14 + A01 4）**全部**在符号定义缓存 `JSite*/PSMcluster0` 里、全部坐在名为 `Dimension` 的图层上，是参数化符号的**驱动尺寸**（`0x006F` Standard Relation 的出参 13/13 都是它）。工作项按「解码 + 参数化链闭环 + 显示决策」定义，不按「画 14 条标注」定义 | ⭕ |
| D2 | JDim 画不画 | 默认**不画**：它在定义缓存的 `Dimension` 层上，与 `Construction` 同属定义内部构造几何；是否上屏由 L1 解出的视图过滤集显示状态裁决，不由名字猜。解码结果进 `JSiteNestedGeometry::dimensions` 作证据 + 进 OCS 特性面板 / 导入摘要 | ⭕ |
| D3 | JDim 取证路线 | 先 `imagdex.dex` 原生读取器（08-31 的 `tools/idalib_imagdex_*.py` 现成，按 `JDim Object` vtable slot 3 → `DoIO` 反编译），字节统计只做互证；`radsrvitem.dll!sub_564BA320`（igDimension 277）作第二对照。**没有原生读法坐实的字段不进 DTO**（保持 `raw`），与曲线族同一纪律 | ⭕ |
| D4 | 图层槽归谁 | **分两步**：先做「分类过滤机制 + 图层管理器的图纸图层视图」（L2，不动槽），再把槽换成图纸图层（L3，导入选项后置切换）。两步共用同一套按 XDATA 分类的逐实体可见性机制；L3 是否本轮做完见 D5 | ⭕ |
| D5 | L3 范围 | 本轮**做到「选项可切、默认不切」**：`PID_LAYER_MODE=sheet` 时槽=图纸图层名（原样，不加前缀），taxonomy 退到 XDATA `class=` / `style=`；探针 / 测试 / 图例线跟着改成按 class 取；默认值翻转另开一轮，等 DXF 下游消费方的要求定下来 | ⭕ |
| D6 | 图层显隐的事实源 | `0x0057 Top ViewFilterSet` 的显示状态字节（08-27 记为「`FF 02 00 …` 一段」，未解）→ 解出后**替代**现在按名字猜的隐藏类（`Hidden` / `HiddenObjects` / `Invisible`）；解不出则名字判据保留，登记缺口 | ⭕ |
| D7 | 执行顺序 | L1（pid-parse 地基）→ J1 → J2 → L2（OCS 可见价值）→ J3 → L3（选项后置）→ 台账；一项一提交，先红后绿 | ⭕ |

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
1. **导入端**把两维都写全：现有 `sheet_layer=` / `sheet_layer_oid=` 之外加 `class=`（geometry / text /
   symbol / symbol-label / point-ok|warning|error|approved / annotation / connectivity / fill / frame）
   与 `style=`（样式名，discipline 的来源）。零破坏。
2. **过滤机制**：文档级「P&ID 视图过滤」（按存储的图纸图层 on/off 集合 + class on/off 集合），应用 =
   对每个带 P&ID XDATA 的实体求 `invisible = !(sheet_layer_on && class_on)`；状态存文档 XDATA 记录
   （`PID_VIEW_FILTER`），DXF 往返后 `invisible` 位与记录同在。**初值取 L1 解出的显示状态**（L1 未到
   之前用现有名字判据）——这一步落地那天 `PID-HIDDEN` 的归层逻辑就可以退役成「`Hidden` 层 off」。
3. **图层管理器**加一个模式切换（合成层 / 图纸图层）：图纸图层模式列出该图的图层名（去重跨视图过滤
   集，按存储折叠，默认只看顶层存储）、实体计数、on/off 开关；class 作为第二段（复选）；搜索框沿用。
   Layer States 不动。
4. 特性面板 P&ID 组加 `Class`；导入摘要加「图纸图层 N 个，其中 M 个按文件状态关闭」。

**验收**：`pid_import` 新断言：每个 P&ID 实体带 `class=`；0202 在 XDATA 记录里关掉 `Labels` 后其 text
实体 `invisible`、开回来恢复；`pid_panel_localization` 四语言覆盖新词条；手工验收：打开 0202，切到
图纸图层模式，看到 `Default / Labels / HeatTrace / …` 与计数，勾掉 `HeatTrace` 电伴热线消失。

### L3 · 图层槽切换到图纸图层（OCS，选项后置）

**现状**：槽 = 合成 taxonomy；四处消费者按 `PID-*` 名字取实体。

**改法**：导入选项 `PID_LAYER_MODE`（`taxonomy` 默认 / `sheet`）。`sheet` 模式：实体 layer =
图纸图层名**原样**（`Default` / `Labels` / …，按 D5 不加前缀；同名跨存储合并为一个 DXF 层，oid 仍在
XDATA）；没有图纸图层的 OCS 自造实体（连通链、符号名标签、评审点符号）保留各自 `PID-*` 层；
`PID-STYLE-*` / `PID-HIDDEN` 不再生成（discipline 走 `style=` + L2 过滤，隐藏走文件状态）；
`ensure_layer` 用 L1 的显示状态设初始 on/off。消费者改造：`pid_import.rs` 的 `on_layer` / `is_line_work`
改成按 XDATA `class=` 取的 `of_class`（两种模式下同一套断言都得过）；`pid_probe` / `pid_plot_dump` 按
class 分组输出。

**验收**：两种模式下 `pid_import` 全绿；`sheet` 模式打开 0202：图层表就是 SmartPlant 的 16 个名字
（含状态），DXF 另存后在第三方查看器里图层名一致；默认值不翻转（D5）。

### T · 台账与共享记忆（双仓）

pid-parse `task_plan.md` 当前阶段加本轮指针；OCS `.context` 会话文件按惯例；每项落地 `remember` 一条。

---

## 执行顺序 ⭕D7

**L1 → J1 → J2 → L2 → J3 → L3 → T**。L1 先行是因为 J（D2 裁决）与 L（显隐初值）都吃它；J1/J2 紧接着
把 JDim 从盲区拿出来；L2 是用户可见价值，且不依赖 J；J3 是纯证据，插在 L2 之后不卡屏幕；L3 最后，
选项后置、默认不切，风险隔离。

时间盒：L1 一个工作日；J1 两个工作日（原生读取器无果则 J2 只带坐实字段 + raw，不空转）。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 把 JDim 画成尺寸标注实体 | D1/D2：它是定义内部驱动尺寸，SmartPlant 自己也不在放置上画；除非 L1 实测 `Dimension` 层为显示 |
| `JBalloon`（0x0117）/ `JLeader`（0x0118）/ `0x00FF` | 全语料 0 条，无 fixture 不写；08-07 的图形类点名告警已覆盖 |
| `0x0010` 子记录语义（638 条） | 与 JDim 同 GUID，可能随 J1 顺带落地，但不作验收项 |
| 按视图过滤集分别呈现图层状态 | OCS 单模型空间，取一份（顶层存储、第一个集合）；多视图是另一个产品命题 |
| `PID_LAYER_MODE` 默认翻转 | 等 DXF 下游消费方（图例线那批 DXF 的用法）把要求说清 |
| 缓存 vs 库本体优先级、缓存 `StyleCluster` 接入 | 09-07 上午登记的两条，与本轮无耦合，另排 |
| A01 `/JSite204` `Default` 计数差 4、`0x0057 +32` 语义 | 未解释记账，等新证据（L1 可能顺带碰到，碰到就记） |

## 术语

- **驱动尺寸（driving dimension）**：参数化符号定义里约束几何的尺寸对象（`JDim`），值由公式
  （Standard Relation）从命名变量（Double Value）算得；放置实例按它重算本体。与页面上的标注
  （annotation）不是一回事。
- **视图过滤集（ViewFilterSet）**：`0x0057` / `0x0060`，按视图存的图层选集与显示状态；「图纸图层
  显隐」的事实源。
- **分类（class）**：OCS 导入器给实体的角色标签（现在体现为 `PID-*` 合成层名），L2 起以 XDATA
  `class=` 存在，与图纸图层正交。
- **图层槽（layer slot）**：DXF 实体唯一的 layer 字段；D4/D5 讨论的就是它归哪一维。
