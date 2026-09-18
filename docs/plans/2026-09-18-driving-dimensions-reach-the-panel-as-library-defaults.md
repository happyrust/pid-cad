# 驱动尺寸进特性面板 · 按「模板 / 库默认」口径 · 开发计划（2026-09-18）

> 承接 `2026-09-07-jdim-driving-dimensions-and-layer-panel.md`（D7 七项 2026-09-18 全部落地）。那份计划的 D2 后半句写
> 「解码结果进 `JSiteNestedGeometry::dimensions` 作证据 + 进 OCS 特性面板 / 导入摘要」，J2 落地时把 OCS 消费**推到 J3 之后**
> （先把数和公式对上再上面板），J3 结算又把口径改了：**放置实例身上没有属于它的尺寸值**——语料 18 条 JDim 全在五张
> 未被任何放置点名的模板 sheet 上，被点名的五个实例本体零 JDim、零关系、零 Double Value，只有一份变量值等于库默认的
> `SymbolInformation` 副本和算好的几何（D06 Tank 是同一公式按毫米再算、Manifold / Black Box / Drum 被拉过且参数不在
> 文件缓存里）。所以「把 20.32 标给一个画着 35.59 弧的 Manifold」是误导。本计划把 D2 后半句按 J3 §7 的两条出路落成
> 工作项：**（a）模板本体的驱动尺寸，明说是库默认；（b）实例本体的实际外框**；导入摘要计数无害，一并做。
> 事实基础：pid-parse `docs/analysis/2026-09-18-the-parametric-chain-closes-on-the-template-not-the-instance.md`
> （探针 `probe_parametric_chain_resolves_a_cached_body`，四主图 + A01 逐存储不抽样）、`2026-09-15-tag-188-…`、
> `2026-09-07-placement-tail-names-the-cached-definition.md`，以及 OCS `pid.rs` / `scene/cache/properties.rs` 现状。
> 带 ⭕ 的决策按推荐落笔、未经拍板，Plannotator 批注里划一笔即可翻案。
> 状态：**待门禁**（未过 Plannotator）。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| K-D1 | 面板上给放置实例看什么 | **两行，缺一不可**：「驱动尺寸（库默认）」= 配对到的**模板**本体上带名字的 JDim 值（`Top 20.32 mm · Left 114.3 mm · Right 114.3 mm`），标题里就写「库默认」，不写成实例的尺寸；「本体尺寸」= **实例**本体画出来的外框 `W × H mm`（Manifold 172.21 × 71.18，模板是 228.6 × 40.64）。两行并排，用户一眼看出「库默认 vs 实际」。**绝不**把 `PidSymbolDefinition::dimensions` 当成放置的尺寸展示（J3 §7） | ⭕ |
| K-D2 | 名字与配对在哪一层做 | **pid-parse**。名字 = `Standard Relation` 出参 JDim ← 入参 `Double Value` ← `SymbolInformation` 变量名（`Top` / `Left` / …）+ 公式字符串；配对 = 实例那份 `SymbolInformation` 的 `value_ref` 解到**另一存储**的 `Double Value`（4/5 对），解不到时按变量名 + 值全同配（工艺那对；四个 0.0127）。这两件都只靠已解开的四族记录与 `0x0115` 按 oid 接，等级 **corpus**（17/17、5/5），与 J3 的棘轮同一套判据；OCS 只看到 `PidNormalizedGeometry`，看不到存储，做不了 | ⭕ |
| K-D3 | 没有配对到模板的放置 | **不给任何一行**，也不猜：非参数化符号（`SymbolInformation` 无变量、`(0,0)` 那十几条）与配不上对的实例都没有「驱动尺寸」；「本体尺寸」也只在配上对的放置上给（它的意义是与库默认对照）。是否给所有符号一个外框行另开一轮 | ⭕ |
| K-D4 | 面板数据怎么到实体 | **写进放置每个实体的 `PID_SEMANTICS` XDATA**，两个新键：`driving=<name>:<mm>;<name>:<mm>;…`（模板的、按 JDim 在模板里的 oid 序）、`extent=<W>x<H>`（实例本体外框，mm，两位小数）。理由：特性面板逐实体读这条记录，已有 `class` / `role` / `label` / `oid` / `sheet_layer` 在里面；代价有界（0202 的 181 个符号实体 × 几十字节）；DWG / DXF 往返自然保留。文档级 XRecord + 按 `oid=` 查表被否——`oid=` 只在有 `_Data.xml` 命中时才写，参数化符号未必有 | ⭕ |
| K-D5 | 单位与精度 | JDim 值在 pid-parse 里是米（`value_m`）；面板与 XDATA 用**毫米、两位小数**（图纸单位；20.32 / 114.3 / 35.56 本来就是英寸整倍换出来的），不显示英寸。公式字符串（`0E$1+0.01`）**不上面板**，只留在 DTO 里给探针 / 测试 | ⭕ |
| K-D6 | 画不画 | 不变：09-07 计划 D2「默认不画」由 L1 文件事实背书（`Dimension` 层 11/11 关），本计划不动任何投影输出、不动 golden 实体 | ⭕ |
| K-D7 | 与「实例真实参数在文件何处」的关系 | J3 开口（放置记录尾部？`_Data.xml` 项属性？）**不在本计划里找**。本计划的「本体尺寸」行是从画出来的实体量的，不依赖那个答案；找到那天在面板上多加一行「实例参数」即可 | ⭕ |

## 背景

### pid-parse 现在能给的

- `PidNormalizedGeometry::symbol_definitions: Vec<PidSymbolDefinition { reference: {site, sheet}, layers, primitives, dimensions }>`，
  `symbol_definition(reference)` 按放置的 `PidGraphicKind::SymbolInstance { definition: Some(ref) }` 查得到**实例**本体
  （OCS `pid.rs::build_entities` 已经这样取缓存本体画符号）。
- `PidSymbolDimension { oid, sheet_layer_ref, value_m, measured_oid, endpoints, group_ref }`——**没有名字、没有公式、
  不知道属于哪个变量**；而且它挂在**模板**的 `PidSymbolDefinition` 上（J2 棘轮「每条恰归一个本体」归的是模板 sheet），
  放置点名的实例本体上 `dimensions` 为空。所以 OCS 今天从放置出发 `symbol_definition(ref).dimensions` 取到的是 `[]`。
- 四族表达式记录已解：`DecodedSymbolInformationRecord { oid, parent_ref, extents, variables: [{name, value_ref}] }`、
  `DecodedDoubleValueRecord { oid, parent_ref, value }`、`DecodedStandardRelationRecord { oid, signature, operands, formula }`、
  `DecodedVariablesRecord`。J3 探针把它们与 JDim 按 oid 接成链、重算公式，17/17 闭合（常数按英寸），但只在探针与棘轮里，
  **不在 DTO 上**。

### 语料：五对模板 / 实例（J3 §2、§5）

| 图 | 符号 | 模板 sheet（未被点名） | 实例 sheet（被点名） | 模板驱动尺寸 | 实例外框 vs 模板外框 |
|---|---|---|---|---|---|
| D06 | Cone Roof Parametric Tank | `/JSite145` 15 | `/JSite151` 47 | 5 条：Top / Bottom → 35.56，Left / Right → 63.5，顶尖 12.7 | 122.12 × 82.844 vs 127.0 × 83.82（同公式按毫米再算） |
| 0201 | Parametric Manifold | `/JSite329` 49 | `/JSite396` 113 | 3 条：Top 20.32、Left 114.3、Right 114.3 | 172.209 × 71.18 vs 228.6 × 40.64（拉过，弧 r 35.59） |
| 0201 | ` Line2` | `/JSite329` 501 | `/JSite396` 119 | 2 条：Right 25.4；JDim 503（3.81）无公式、无名 | 25.4 × 3.81 = 模板 |
| 工艺 | Parametric Black Box | `/JSite7559` 72 | `/JSite6963` 21 | 4 条：Left / Right / Bottom / Top 12.7 | 126.627 × 90.767 vs 25.4 × 25.4（拉过） |
| A01 | Horizontal Drum | `/JSite39` 96 | `/JSite121` 481 | 4 条：Top 20.32、Left / Right 114.3、82 = Top/2 = 10.16 | 159.459 × 51.917 vs 228.6 × 40.64（拉过） |

18 条 JDim 里 17 条是某条关系的出参，1 条（0201 JDim 503）没有公式。17 条里 **15 条的入参是命名变量**（名字可追：
Top / Left / Right / Bottom），**2 条的入参是别的 JDim**（D06 JDim 19 = `0E($1+$2)/10`、A01 JDim 82 = `0E$1/2`，是派生尺寸，
有公式没名字）。实例那份 `SymbolInformation` 的变量名与值 5/5 对与模板全同 = 库默认。0202 没有参数化符号。

### OCS 现在有的

- `scene/cache/properties.rs::pid_semantics_section` 读 `PID_SEMANTICS` 的 `class` / `role` / `label` / `lines` / `resolved` /
  `sheet_layer` / `sheet_layer_oid`，逐键一行只读属性；`tests/pid_import.rs::role_and_class_are_separate_keys_…` 钉着这条记录
  的**键白名单**（8 个），新键要进白名单。
- `ImportSummary` 两行（画出 / 图纸图层），`app/update/file.rs` 在打开完成时推到命令行；新词条走 `locale_catalog.rs` + 21 本
  `locales/*/opencadstudio.ftl`，`i18n::tests::every_catalog_covers_and_formats_the_source_catalog` 少一本就红。
- `pid.rs::build_entities` 里放置的实体（本体笔画 + 符号名标签）在同一处建出，外框可以当场量。

## 目标

一句话：**选中一个参数化符号，特性面板告诉你它的库默认驱动尺寸是多少、它在这张图上实际画了多大——两个数分开写、
都不撒谎；导入摘要多一行说这张图有多少驱动尺寸、都在模板上。**

验收基线：pid-parse `cargo test --all-targets` / clippy `-D warnings` / fmt 全绿（现：`--lib` 1110、`parse_real_files` 130）；
OCS `pid_import` **两种 `OCS_PID_LAYER_MODE` 下都全绿**（现 50/50 × 2）、`i18n` 目录守护全绿；数值变更随 analysis 文档；
不改任何投影输出（golden 实体不变，schema 只多字段）。

---

## 工作项

### K1 · 驱动尺寸命名与实例 → 模板配对（pid-parse）

**现状**：见背景。名字与配对都在探针里，DTO 上没有。

**改法**：
1. `PidSymbolDimension` 加 `name: Option<String>`（出参它的那条关系的**入参**里、能解到 `SymbolInformation` 变量的那个名字；
   `0E($1+$2)/10` 这种入参是别的 JDim 的，名字取不到就 `None`，不编）与 `formula: Option<String>`（关系原文）。
2. `PidSymbolDefinition` 加 `variables: Vec<PidSymbolVariable { name, value_m }>`（本 sheet 的 `SymbolInformation` 变量 →
   `Double Value` 值，模板与实例都有）与 `template: Option<PidSymbolDefinitionRef>`（**只在实例上**：按 K-D2 的两步配对；
   模板自己与非参数化符号为 `None`）。
3. 配对与命名放在 `geometry.rs` 组装 `symbol_definitions` 的地方（缓存已按存储解开四族记录），不新加解码器；
   `parser_panic_safety` 不需要新入口。
4. 探针 `probe_parametric_chain_resolves_a_cached_body` 改成从新字段读，少一半自己 join 的代码；分析文档 §2 表加「配对依据」一列。

**验收**：棘轮 `a_placed_parametric_body_names_its_template_and_the_template_names_its_dimensions`：五对 5/5 配上（4 by
`value_ref`、1 by 名 + 值，A01 软跳）；**15/18 条 JDim 有名字**，无名的恰是三条——0201 JDim 503（无公式，`formula` 也 `None`）、
D06 JDim 19 与 A01 JDim 82（派生尺寸，`formula` 有 `name` 无）；每个实例的 `variables` 与模板逐条相等；模板的 `template` 为
`None`、非参数化定义的 `variables` 为空且 `template` 为 `None`；D06 五条名字 = {Top, Bottom, Left, Right} ∪ 一条 `None`。
golden 不变、schema 多字段重封；`--lib` / `parse_real_files` 只增不减。

**风险**：`value_ref` 跨存储解析要在 `PidDocument` 级而不是单存储级做——J3 探针已经这么做过，照搬；工艺那对靠名 + 值配是
弱证据，写进文档并在 DTO 文档注释里标 corpus。时间盒一个工作日。

### K2 · 特性面板两行（OCS）

**现状**：P&ID 组七个键、无尺寸信息。

**改法**：
1. `pid.rs::build_entities` 的 `SymbolInstance` 分支：取 `symbol_definition(ref)`，若其 `template` 为 `Some` → 取模板的
   `dimensions` 里 `name.is_some()` 的条目，按 oid 序拼 `driving=Top:20.32;Left:114.30;Right:114.30`（mm，两位）；量本次
   `built` 实体的外框（不含符号名标签那条 `Text`）拼 `extent=172.21x71.18`；两键经 `attach_pid_metadata` 写进该放置的**每个**
   实体（本体笔画与标签都写，标签也是这个放置的）。没有 `template` 的放置一个键都不写（K-D3）。
2. `properties.rs::pid_semantics_section` 读 `driving` / `extent`：两行只读——「驱动尺寸（库默认）」值 `Top 20.32 mm · Left
   114.30 mm · Right 114.30 mm`，「本体尺寸」值 `172.21 × 71.18 mm`；紧跟「角色」行之后。新词条 2 条（两种界面语言的标题）
   21 本全覆盖。
3. `tests/pid_import.rs`：键白名单加 `driving` / `extent`；新断言——0201 Manifold 的放置实体 `driving=Top:20.32;Left:114.30;
   Right:114.30` 且 `extent=172.21x71.18`、` Line2` 只有 `Right:25.40`、D06 Tank 四个名字 + `extent=122.12x82.84`、工艺 Black Box
   四个 12.70；**所有非参数化符号实体两键皆无**；两键过 DWG / DXF 往返；**两种图层模式下相同**（并入
   `the_two_layer_modes_agree_on_everything_but_the_slot` 的三键比较，变五键）。

**验收**：上述断言全绿；手工：打开 0201，点 Manifold 任一笔画，P&ID 组多出两行、数字如上；点一个阀门，两行都不出现。

### K3 · 导入摘要第三行（OCS）

**现状**：两行。

**改法**：`ImportSummary` 加 `driving_dimensions`（模板上带名字的 JDim 总数）、`template_bodies`（有 `dimensions` 的定义数）、
`parametric_placements`（配上模板的放置数）；`file.rs` 多推一行
`P&ID driving dimensions: {dims} on {templates} template bodies; {placements} placed parametric bodies carry library defaults`
（0201：`4 on 2 template bodies; 2 placed …`——sheet 49 三条 + sheet 501 一条有名、503 无名不计；D06：`4 on 1; 1`，那条无名的
`0E($1+$2)/10` 出参不计；0202 全零时**这一行不推**，别给没有参数化符号的图凭空多一行）。新词条 1 条 21 本。

**验收**：`the_import_leaves_a_summary_the_app_can_show` 加三个数的断言（0201：4 / 2 / 2）；0202 三个数为零；
计数与 K2 写出的 `driving=` 键数一致（每个配上对的放置恰一组）。

### K4 · 台账（双仓）

- user-guide `.pid` 一节：特性面板那句的括号里加「驱动尺寸（库默认）」「本体尺寸」两项并说明**前者是符号库默认值不是
  这张图上的实际尺寸**；导入汇总那句加第三行。
- pid-parse CHANGELOG（K1）、guide 参数化链一节补「名字与配对已进 DTO」、`task_plan.md` 指针。
- 本计划头部补记 + 各项进度行 + 结算；09-07 计划 D7 结算行的「J 线的 OCS 消费」一条标已排入本计划。
- `remember` 一条：驱动尺寸只能按库默认展示、键名 `driving=` / `extent=`。

---

## 执行顺序 ⭕

**K1 → K2 → K3 → K4**。K2 / K3 都吃 K1 的名字与配对，K1 单独提交、先红后绿；K2 与 K3 各一提交；K4 随各项收尾。
pid-parse 每项完成后回跑 OCS `pid_import`（两种模式）。总时间盒两个工作日。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 找放置实例的真实参数 | J3 开口，K-D7：本计划不依赖它 |
| 给所有符号一个「本体尺寸」行 | K-D3：先只给参数化的，与库默认成对照；全给另议 |
| 面板上显示公式 / 英寸值 | K-D5：公式给探针，英寸对用户没意义 |
| 画驱动尺寸 / 打开 `Dimension` 层 | 09-07 D2 + L1 事实 |
| 图层管理器 / 导入选项里的任何新开关 | 本计划只读不改行为 |
| `SymbolInformation.extents` 两个数的语义 | J3 §8 未坐实，不进 DTO |

## 术语

- **库默认（library default）**：符号库模板身上的变量值与驱动尺寸；放置实例的 `SymbolInformation` 副本记的也是它，
  **不是**实例在图上的实际参数。
- **本体尺寸（body extent）**：放置实例画出来的笔画外框，mm；是这张图上的事实，与库默认无关。
- **配对（pairing）**：实例定义 → 模板定义的对应，靠 `value_ref` 跨存储解析（主）或变量名 + 值全同（次）。

## 门禁记录

- 2026-09-18：初稿，待 Plannotator `annotate --gate --json`。
