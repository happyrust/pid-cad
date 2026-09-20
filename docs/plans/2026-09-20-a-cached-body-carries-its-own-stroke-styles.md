# 缓存本体带上自己的逐笔样式 · 接缓存存储的 `StyleCluster` · 小计划（2026-09-20 开单）

> 承接 pid-parse `docs/analysis/2026-09-07-placement-tail-names-the-cached-definition.md`「还没做的」第二条
> （「缓存存储自己的 `StyleCluster` 未接：缓存本体现在不带自身笔画样式，只靠放置样式重涂；放置样式解析不到的那几条会落 `ByLayer`」）
> 与 `2026-09-19-draw-the-cached-body-first-and-the-library-only-when-the-drawing-carries-none.md` 的 P-D5 / 结算开口。
> 现状：`pid.rs::cached_body_entities` 拿 `PidSymbolDefinition::visible_primitives()` 直接 `shape_primitive`，**没有底涂**；库路径
> `place_primitive` 先按 `.sym` 自己的 `PrimitiveStyle`（颜色 / 线宽，**无 dash**）涂一层，再由 `apply_symbology` 按放置样式压掉颜色与线宽。
> 两条路今天都画不出符号内部的虚线。
> 事实基础：2026-09-20 开单时对四图逐本体实测（临时探针，未入库；数字见「事实」）。带 ⭕ 的决策按推荐落笔。
> **2026-09-20 用户在会话（fable-5-1-45）里批准 P-E1 … P-E8**，全按推荐；直接开 E1。
> **E1 落地**：pid-parse `7e69b8a`（同一会话）——`primitive_styles` / `visible_strokes()` / `JSite::stroke_styles` / `PrimitiveStyle::dash_mm` /
> 圆弧 `index` / `for_each_document` 分隔符；棘轮 `parse_real_files` 133 → 134，「事实」里的数字全部钉住。
> **E2 落地**：OCS `3b8af2ed` + pid-parse `1fd69db`（`DashPattern::from_segments_m`，单测造放置样式用；会话 fable-5-1-8 接手）——
> 缓存本体走 `place_primitive` 底涂、`paint_symbol_stroke` 按 `dash_mm` 点名 `PID-DASH-*`、`register_dash_linetypes` 多收一遍缓存本体；
> `pid_import` 54 → 55（工艺 45 / 0202 12 / D06 0 / 0201 0，四种环境组合全绿），`--lib io::pid::tests` 8 → 9。
> **状态：E1 / E2 / E3 全部落地。** 手工 GUI 特写已补：`docs/evidence/2026-09-20-cached-stroke-dash/`（工艺 `Xa` OPC 虚线圆 + 箭杆、
> 0202 阻火呼吸阀旁通与唇；同一提交 `--export` DXF 里 `PID-SYMBOL` 上带 `PID-DASH-*` 的实体工艺 45 / 0202 12，与集成测试一致）。

## 一句话

缓存本体的每一笔带上它自己存储 `/JSite<N>/StyleCluster` 里的颜色 / 线宽 / **虚线**，与库本体的 `StyledPrimitive` 是同一套词汇；
OCS 拿它做放置样式之下的**底涂**——放置的颜色 / 线宽照旧压在上面（P-D5 不变）。净效果两条：**符号内部的虚线第一次画对**
（语料 11 个放置 57 笔）；放置样式解析不到时的退路从 `ByLayer` 变成符号自己说的样式（语料 0 例，是正确性不是画面）。

## 事实（2026-09-20 实测，四图 107 个放置点名的 41 个本体）

| 图 | 放置 | 被点名本体 | 可见笔画（按放置） | 其中 `index` 在本存储 `StyleCluster` 解析 | 其中**虚线** | 放置样式解析 |
|---|---:|---:|---:|---:|---|---:|
| D06 | 6 | 6 | 32 | 32 | 0 | 6/6 |
| 0201 | 20 | 17 | 81 | 81 | 0 | 20/20 |
| 0202 | 23 | 11 | 120 | 120 | **12**：arrester breather valve(RD) 21 笔可见里 8 笔（7 线 + 那条 B 样条唇）、Wastewater Pit 7 笔可见里 4 笔 | 23/23 |
| 工艺 | 58 | 7 | 237 | 237 | **45**：`Xa.sym` ×3 与 `Xa chu.sym` ×6，每个放置 9 笔可见里 5 笔（1 圆 + 4 线） | 58/58 |

- **全部解析**：470/470 可见笔画的 `index` 在各自存储的 `StyleCluster` 里落到 `0x002E JStyleSimpleLine`，直接或经 `0x0030 JStyleOverride`
  一跳——与图纸几何同一条「看它落在哪种记录上」的规则（`style_link` 模块文档）。关闭层上的笔画也全部解析（伴热线是 3.5/1.75 mm 虚线，
  与 P-D2 的「SmartPlant 不画」相容）。
- **与库逐笔一致**：库里有同名 `.sym` 的本体，缓存逐笔的颜色 / 线宽与 `.sym` 自己 `StyleCluster` 给的相同——Ball Valve Type 1 六线 0.13 +
  一圈 0.35（经 override 19 / 20）、Cap 两笔 `#00FFFF` 0.50、Off-Unit 五笔 `#00FEA0` 0.50 + 两笔黑 0.35、Dual Action Cyl Act 七笔
  `#00FEA0` 0.50、Parametric Manifold 六笔黑 0.35；库多出的只是 `.sym` 第二张 sheet 的笔画。库没有的 `Xa*.sym` 三种（工艺 12 个放置），
  缓存是唯一的样式来源。
- **虚线是缓存独有的信息**：`.sym` 读取器的 `PrimitiveStyle` 明写「dash 不带」；缓存这边 `resolve_line_style` 的 `dash` 现成。语料可见虚线
  57 笔 / 11 个放置全是 3.5/1.75 mm 一种图样，今天 OCS 都画成实线；D06 / 0201 的虚线只在 `Heat Trace[OFF]` 上，屏幕上本来就没有。
- **放置样式 107/107 解析**（`igSymbol2d +25`，全部实线；Wastewater Pit 的经 override 109）。09-07 文档担心的「解析不到落 `ByLayer`」在语料里
  是 0 个放置。
- **线宽**：放置线宽（0.18 / 0.35 / 0.50）与逐笔线宽（0.13 / 0.35 / 0.50）在多数本体上不同（球阀六线 0.13 压成放置的 0.35；DCS Field
  Mounted 0.13 / 0.35 压成 0.18）。08-24 文档在 SmartPlant 截图上**实测的是颜色**，线宽没测过。
- **文字**：缓存文字 100% 在关闭层（P-D4）；缓存 `JStyleTextPara` 解得出高度 1.50 / 2.46 / 2.50 / 3.17 / 6.35 mm、对齐 Left / Center、黑色，
  没有一条上屏。
- **模型缺口**：`DecodedIgCircle2dRecord` / `DecodedIgArc2dRecord` 两个 DTO 没带 `index`（解析层 `SheetIgCircle2dDecoded` /
  `SheetIgArc2dDecoded` 读了，`From` 里丢了）；线 / 折线 / 文字 / B 样条都带。
- **顺手发现**：`style_link::for_each_document` 用 `rsplit('/')` 找 `Sheet*` 叶子，但 `cfb` 0.14 在 Windows 上给的嵌套路径是
  `/JSite145\PSMcluster0`（只有第一段是 `/`），与 `cfb/reader.rs:80`、`streams/psm_tables.rs` 那几处的 `replace('\\', "/")` 不一致。
  今天无影响——四图的嵌套存储**一个 `Sheet*` 流都没有**（本体在 `PSMcluster0`）——但同一份文件在 Windows / Unix 上不该给不同结果，
  模块文档那句「每个 `JSite<n>/` 都是带自己 `Sheet*` + `StyleCluster` 的嵌套文档」也该按语料改成 `PSMcluster0` + `StyleCluster`。

## 决策

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| P-E1 | 放置样式与逐笔样式谁压谁 | **不变**（P-D5）：`apply_symbology` 的放置颜色 / 线宽仍压在整个本体上；逐笔样式做底涂，与库路径 `paint_symbol_stroke` 同一个位置。**虚线由逐笔样式决定**——放置样式自己带 dash 才覆盖（语料 107 个放置样式全是实线，所以不会发生）。理由：颜色是截图实测的、线宽没测，维持现状；虚线今天两条路都错，缓存是唯一带 dash 的源。备选「逐笔线宽压过放置线宽」要 SmartPlant 打印件或高倍截图量线宽，登记为开口 | ⭕ |
| P-E2 | 样式放在 `PidSymbolDefinition` 的哪 | **平行表** `primitive_styles: Vec<Option<PrimitiveStyle>>`——与 `primitive_layers` 同款（同长同序、`serde(default)`、旧数据缺项按 `None`），`primitives` 保持 `Vec<SymbolPrimitive>`；加 `visible_strokes()` 迭代 `(primitive, style)`，`visible_primitives()` 保留给只要形的调用方。备选：`primitives` 改成 `Vec<StyledPrimitive>`——OCS `PidSymbolSource::strokes` / `PlacementMeasures` / C1 棘轮全要跟着动，golden JSON 大改 | ⭕ |
| P-E3 | dash 进不进 `PrimitiveStyle` | **进**：`PrimitiveStyle { rgb, width_mm, dash_mm: Vec<f64> }`，空 = 实线；`.sym` 读取器 `style_of` 同时填上（`resolved.dash` 现成）——两个源一个词汇，库补位时虚线也顺带对了。代价：`Copy` 变 `Clone`（OCS `paint_symbol_stroke` 改收引用），加 `JsonSchema` 派生 | ⭕ |
| P-E4 | 样式表从哪来、存哪 | `parse_jsites` 读 `{base}/StyleCluster` → `DocumentStyleTable::from_stylecluster_bytes`（与 `.sym` 读取器同一个类型），用完即弃；解析结果落 **`JSite.stroke_styles: BTreeMap<u32, PrimitiveStyle>`**（style id → 样式，只收嵌套记录点名到的 id），可序列化，JSON 回读不丢；`embedded_symbol_definitions` 按 `record.index` 查表。备选：`#[serde(skip)]` 挂整张表——回读丢样式 | ⭕ |
| P-E5 | 圆 / 弧 DTO 没有 `index` | 补上 `index: u32`（`serde(default)`，`From` 透传）。语料圆 / 弧的 `index` 全解析——D06 球阀那圈 r 1.27 经 override 19 → 黑 0.35 | ⭕ |
| P-E6 | 文字样式 | **本单不做**：缓存文字 100% 在关闭层，接了也没有一条上屏；出现在开着的层上时再接（`resolve_text_height` 一次调用）。`primitive_styles` 里文字项为 `None` | ⭕ |
| P-E7 | 与退役单 `2026-09-20-retire-the-library-first-symbol-source.md` 的先后 | **本单先做也行、后做也行**：本单只给 `cached_body_entities` 多一行底涂，不碰 `source` 参数；谁后动谁照当时的签名改 | ⭕ |
| P-E8 | 虚线的 linetype 从哪来 | `register_dash_linetypes` 今天只收图纸自己的 `LineStyleIndex`；加一遍缓存本体（`geometry.symbol_definitions[*].primitive_styles`），同一个 `PID-DASH-<n>` 池、同一个 `dash_key`。库本体是懒解析（`build_entities` 里才知道用哪个 `.sym`），它的 dash 只在池里已有同一图样时画出，否则照旧实线并记一行日志——语料里库补位的放置是 0 个 | ⭕ |

## 工作项

### E1 pid-parse：缓存本体带样式（加法，一提交）

- P-E5：`DecodedIgCircle2dRecord` / `DecodedIgArc2dRecord` 加 `index`。
- P-E3：`PrimitiveStyle` 加 `dash_mm` + `JsonSchema`；`symbol_library::style_of` 填 dash；文档那句「dash 不带」改掉。
- P-E4：`streams/jsite.rs::parse_jsites` 读 `StyleCluster`（`cfb.open_stream` 接受 `/` 路径），只对 `nested_geometry` 里出现过的
  `index` 调 `resolve_line_style`，落 `JSite.stroke_styles`。
- P-E2：`geometry.rs::embedded_symbol_definitions` 的 `push` 多带一个 `style`，`PidSymbolDefinition::primitive_styles` +
  `visible_strokes()`；`schema.rs` 的 JSON Schema 随派生更新。
- 棘轮 `tests/parse_real_files.rs`：`a_cached_body_carries_the_stroke_styles_its_own_storage_states`——四图被点名本体的可见笔画
  **全部**有样式（按放置累计 32 / 81 / 120 / 237）；可见虚线 D06 0 / 0201 0 / 0202 12 / 工艺 45，图样都是 `[3.5, 1.75]`；
  缓存 vs 库颜色线宽逐笔一致钉三例（Ball Valve Type 1、Cap、Off-Unit）；D06 球阀那圈 r 1.27 → 黑 0.35。
- golden JSON：新字段非空，期望文件重生成，逐项看 diff **只多不改**。
- 顺手：`style_link::for_each_document` 路径分隔符归一（`replace('\\', "/")`），模块文档那句按语料改；`style_link_ratchet` 数字不应变
  （嵌套存储无 `Sheet*`），变了就是发现了新东西，停下来看。
- CHANGELOG 一节；`task_plan.md` 指针；09-07 文档「还没做的」第二条改标已接。

### E2 OCS：底涂 + 虚线（一提交）

- `paint_symbol_stroke(entity, style: Option<&PrimitiveStyle>, dash_linetypes)`：颜色 / 线宽照旧，`dash_mm` 非空且池里有 → `common.linetype`。
- `cached_body_entities`：改走 `visible_strokes()`，每笔先 `paint_symbol_stroke` 再交给 `apply_symbology`（调用处不变）；`library_body_entities`
  的 `place_primitive` 用同一个函数——库的 dash 顺带。
- `register_dash_linetypes(&mut doc, &styles, &geometry.symbol_definitions)`（P-E8）。
- 单测 `io::pid::tests`：合成一条——放置样式实线 + 笔画 `dash_mm=[3.5,1.75]` → 实体带 `PID-DASH-*` 且颜色 = 放置颜色、线宽 = 放置线宽；
  放置样式自己带 dash → 放置的赢。
- `tests/pid_import.rs`：`a_cached_strokes_dash_is_its_own_storages_not_its_placements`——工艺 `PID-SYMBOL` 上带 `PID-DASH-*` 的实体 = 45
  （9 个放置 × 5）、0202 = 12、D06 / 0201 = 0；颜色仍是放置的（Xa 的 `#00FEA0` 底涂被 `#808000` 压掉，
  `a_symbol_body_draws_in_the_style_its_placement_names` 继续绿）；`extent=` 一个都不变。
- user-guide「符号从哪来」补一句：缓存本体的笔画按符号自己的样式表决定虚实，颜色与线宽仍按放置。
- 手工：GUI 打开工艺放大一个 `Xa` OPC（虚线圆 + 四条虚线）、0202 的 arrester breather valve(RD) 唇是虚线；两张特写入
  `docs/evidence/2026-xx-xx-cached-stroke-dash/`。

### E3 台账

- 本单头部写落地哈希；09-19 计划结算「还开着的」那条改标已落；`remember` 一条（供以后 `supersedes`）；pid-parse `task_plan.md` 收口指针。

## 验收

- pid-parse：`cargo test --all-targets` 全绿（`parse_real_files` 133 → 134；golden 重生成后 diff 只多不改）；`cargo fmt --check` 干净；
  nightly 与 stable 的 `cargo clippy --all-targets -- -D warnings` 零告警；`cargo +1.95 check` 绿。
- OCS：`pid_import` 在 `OCS_PID_LAYER_MODE` 两值（退役单未动时再 × `OCS_PID_SYMBOL_SOURCE` 两值）全绿（54 → 55，以当时实数为准）；
  `--lib io::pid::tests` 全绿；`clippy --lib --test pid_import` 触碰处零告警；`rustfmt --check` 两文件干净。
- 时间盒：pid-parse 半个工作日 + OCS 半个工作日。

## 登记不做

| 项 | 理由 |
|---|---|
| 逐笔线宽压过放置线宽 | 没测过（08-24 只测了颜色）。要 SmartPlant 打印件或高倍截图量线宽；有了再议，一处 `apply_symbology` 的事。**2026-09-20 量了语料的赌注**（临时探针，未入库）：四图 107 个放置 470 笔可见笔画，逐笔线宽 = 放置线宽 **232**（49%）、逐笔更细 **150**（0.13 → 0.35 有 79 笔：阀体 / 法兰口 / 罐 / 人孔；0.35 → 0.50 有 50 笔：0202 的 ElecTraceLine 36、Item Note & Label 8…；0.13 → 0.18 有 18 笔：DCS 仪表框）、逐笔更粗 **88**（0.50 → 0.35 有 84 笔：Off-Unit / Off-Drawing / Xa / Xa chu / Cap / jinchuzhan2 / Dual Action Cyl Act；0.35 → 0.18 有 4 笔：PT / LG / LT 气泡）；差距都在 ISO 阶梯上一到两级。颜色作对照：470 笔里 350 笔（74%）逐笔颜色 ≠ 放置颜色，而 08-24 截图证明屏幕取放置色——放置样式确实在压本体。**七种本体内部自带线宽对比**（P-D5 抹平了）：Ball Valve Type 1 六线 0.13 + 一圈 0.35、Ball Valve Type 2 六 0.13 + 三 0.35、Manway-Large 四 0.13 + 一 0.35、Off-Unit 五 0.50 + 两 0.35、Xa / Xa chu 箭头四笔 0.50 + 虚线五笔 0.35、DCS Field Mounted 四 0.13 + 一 0.35。**一张截图就能裁**：工艺随便一个 `Xa` OPC 特写（箭头是否比虚线圆粗）或 D06 的 Ball Valve Type 1（六线是否比那一圈细）；08-24 那张 0201 截图若还在，看 LG / LT 气泡圈（本体 0.35、放置 0.18）比旁边 DCS 框（本体 0.13、放置 0.18）粗不粗也够。裁成「逐笔赢」时改动是 `apply_symbology` 对 `PID-SYMBOL` 跳过线宽 + 0201 调色板测试改键。**2026-09-21 补文档证据（没有截图，08-24 那张不在两仓里）**：Hexagon 的 Options Manager 帮助《Linear Patterns and Styles》写明 Symbology 视图是「按过滤器给项目选一条线型图案 + 线型样式」，而线型样式（Linear Style）「定义应用到图案上的格式，如**颜色与线宽**」（[docs.hexagonppm.com …/174149](https://docs.hexagonppm.com/r/en-US/Intergraph-Smart-P-ID-Options-Manager-Help/10/174149)）；SmartPlant P&ID 2014 R1 发行说明写「Options Manager Symbology 设置里为**符号表示**（symbol representations）增加了更多可选线宽（mm）」（[…/363527](https://docs.hexagonppm.com/r/en-US/SmartPlant-P-ID-Release-Bulletin/7.1/363527)）。放置样式（`igSymbol2d +25` 点名的那条 Symbology 样式，如 `Equipment - New #800000 0.35mm`）在 SmartPlant 里就是颜色 + 线宽一体应用到符号表示上的——08-24 在屏幕上证实了颜色那一半，线宽是同一条样式的另一半，文档里没有给它豁免的口子。**建议按文档口径维持 P-D5（放置线宽压过逐笔线宽），本行从「等截图裁」改为「已按文档裁；截图来了作复核」**——Ball Valve Type 1 六线 0.13 + 一圈 0.35 那类本体内部对比，在 SmartPlant 屏幕上也应是被抹平的 |
| 缓存文字的样式（高度 / 对齐 / 字色） | P-E6：语料没有一条上屏 |
| 缓存本体的填充 | 各存储 `StyleCluster` 里 `0x002A JStyleSimpleFill` 1–3 条，但嵌套几何族里没有 `igBoundary2d`，没有消费者 |
| 放置样式解析不到的专门夹具 | 语料 0 例；合成单测覆盖底涂即可 |
| 把探针入库 | E1 的棘轮就是复现；数字全在本单「事实」里 |

## 开单条件

无硬门槛，排期即做。退役单若先动，E2 的签名以退役后为准（P-E7）。

## 术语

- **底涂（undercoat）**：`paint_symbol_stroke` 按符号自己的样式先涂的那一层；`apply_symbology` 按放置样式在其上重涂颜色与线宽，
  不碰 linetype（除非放置样式自己带 dash）。
- **逐笔样式（per-stroke style）**：本体每个图元 `index` 在**本存储** `StyleCluster` 里解出的 `PrimitiveStyle`；样式 id 每个存储从 1 重数，
  跨存储查是 `style_link` 模块文档点名的那个错误。

## 进度

### E1（pid-parse `7e69b8a`，2026-09-20）

照工作项做，无偏离。棘轮 `a_cached_body_carries_the_stroke_styles_its_own_storage_states` 钉住：五图（含 A01）可见笔画按放置 81 / 120 /
32 / 237 / 6 全部有样式；虚线 0 / 12 / 0 / 45 / 0，图样都是 3.5 / 1.75；虚线本体恰是 0202 (793, 2817) 8/21、(793, 3934) 4/7 与工艺
(7559, 155) / (7559, 219) 各 5/9；Cap / Off-Unit / Manifold / Ball Valve Type 1 / Xa 的调色板按「事实」；缓存的每个 `(颜色, 线宽, 虚线)`
都是同名 `.sym` 某一笔画的（库容纳缓存，反向不成立——库合并了所有 sheet）。`.sym` 侧单测钉 arrester breather valve(RD).sym 七线一唇虚线。
golden 不变（快照只钉 `entities`），`style_link_ratchet` 15 不变，`--lib` 1110 → 1111。nightly 1.100 与 stable 1.97 `clippy -D warnings`
零告警、`+1.95 check` 绿、fmt 干净。A01 顺带进了棘轮（开单时没量它：6 笔可见、0 虚线）。

### E2（OCS `3b8af2ed` + pid-parse `1fd69db`，2026-09-20）

照工作项做，两处小偏离：

- `dash_key` / `build_dash_linetype` 改收**毫米段长切片**而不是 `&DashPattern`——两个源的共同词汇正是 `segments_mm()` / `dash_mm`，
  池子一把钥匙；`apply_symbology` 调用处改成 `dash_key(&dash.segments_mm())`。
- pid-parse 多一条 `DashPattern::from_segments_m`（`1fd69db`，上一会话已写好未提交）：`DashPattern` 字段私有，OCS 单测要造一个
  「放置样式自己带 dash」的 `ResolvedLineStyle` 只能这样来；读取器行为不变。

落地：`PidSymbolSource::styled_strokes`（`Cache` 走 `visible_strokes()`，`Library` 全部 + 平行表；`strokes` 改为它的投影，
`PlacementMeasures` 不变）；`place_primitive(primitive, style, at, pool)` 成为缓存 / 库两条路共用的一步；`paint_symbol_stroke` 多收池子，
`dash_mm` 非空且池里有 → `common.linetype`，池里没有 → `log::debug!` 一行、照旧实线（只有库本体会走到——缓存本体开头就全池了）；
`register_dash_linetypes(doc, styles, &geometry.symbol_definitions)` 先图纸后缓存，图纸原有的 `PID-DASH-n` 名字不动；`apply_symbology`
不改代码，只补文档（linetype 只在放置样式自己带 dash 时覆盖）；user-guide「符号从哪来」补一句。

验证：`--lib io::pid::tests` 8 → 9（合成本体三笔：底涂 `#00FEA0` 0.50 虚 / 实 / 无样式；实线橄榄放置样式压掉颜色线宽、留下虚线；
放置样式自己带 dash 时它赢；图纸图样先入池占 `PID-DASH-1`、本体图样 `PID-DASH-2`）；`pid_import` 54 → 55——
`a_cached_strokes_dash_is_its_own_storages_not_its_placements` 钉住 `PID-SYMBOL` 上带 `PID-DASH-*` 的实体：工艺 **45**（全部 ` 35 #808000`，
`#00FEA0` 底涂被压掉）、0202 **12**、D06 **0**、0201 **0**，图样全是 3.5 / 1.75；`OCS_PID_LAYER_MODE` × `OCS_PID_SYMBOL_SOURCE` 四种组合
55/55 全绿（`extent=` 与 0201 调色板测试原样通过）。`rustfmt --check` 两文件干净；`clippy --lib --test pid_import` 在 `pid.rs` / `pid_import.rs`
零命中（整仓另有 1142 条旧告警，与本单无关）。pid-parse 侧 `1fd69db`：单测 1 条、`clippy --all-targets -D warnings` 零告警、fmt 干净。

手工验收（同日补，`docs/evidence/2026-09-20-cached-stroke-dash/`）：debug 版 GUI 打开工艺 `ZOOM` 到 (585,204)–(625,232)——`Xa` OPC 的
r 3.81 圆是虚线、箭杆两横两竖是虚线（4.86 mm 上一段实 + 一个断口）、箭头斜边实线、颜色是放置的橄榄；0202 `ZOOM` 到 (292,252)–(318,281) 与
(298,256)–(312,277)——阻火呼吸阀 `RD060201` 左侧旁通竖线中段断口、唇是虚线、主体实线。同一提交 `--export` 的 DXF 用 PowerShell 解析组码：
`PID-SYMBOL` 上 `PID-DASH-*` 的实体工艺 **45**（`PID-DASH-1`）、0202 **12**（`PID-DASH-2`），与集成测试在内存文档上钉的一致。

## 门禁记录

- 2026-09-20：开单（OCS `6a4cbbad`，会话 fable-5-1-45），四图实测数字见「事实」；待门禁（P-E1 … P-E8 按推荐）。
- 2026-09-20：用户在同一会话批准 P-E1 … P-E8，全按推荐，直接开 E1。
- 2026-09-20：E1 落地（pid-parse `7e69b8a`，同一会话）；OCS 跟随编译的最小改动随 E2 提交。
- 2026-09-20：E2 落地（OCS `3b8af2ed`，pid-parse `1fd69db`；会话 fable-5-1-8 接手 fable-5-1-45 的交接）；E3 台账 `827f1f57`。
- 2026-09-20：手工 GUI 特写补齐（`docs/evidence/2026-09-20-cached-stroke-dash/`），DXF 对数 45 / 12；计划收口。
