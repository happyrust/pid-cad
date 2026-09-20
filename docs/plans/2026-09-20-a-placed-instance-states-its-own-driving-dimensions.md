# 放置实例说出自己的驱动尺寸 · 解码 `0x00ED JFlavorHolder` · 小计划（2026-09-20 开单并同日落地）

> 承接 pid-parse `docs/analysis/2026-09-20-jflavorholder-carries-the-placed-instances-parameters.md`（同日分析，关 09-18 J3 分析 §8 第一条
> 开口「放置实例的实际参数在文件何处」）与本仓 `2026-09-18-driving-dimensions-reach-the-panel-as-library-defaults.md` 的 K-D1
> （面板只能给库默认或实际外框——前提从此不成立）。
> **2026-09-20 用户在会话（fable-5-1-8）里指示「开单并直接做」**，八条决策以下按推荐落笔、未另走门禁。
> **落地**：pid-parse `7498bd9`（F1）、OCS `cedaecd5`（F2 + 本单）。**状态：已落地。**

## 一句话

被放置的参数化实例，其实际参数不在 `JSymbolInformation` 副本里（那是库默认），而在实例存储自己的 `0x00ED JFlavorHolder` 里。
pid-parse 解码它落到 `PidSymbolVariable::instance_value_m`；OCS 把它写成放置实体的 `instance=`，特性面板在「驱动尺寸（库默认）」
下面多一行 **「驱动尺寸（本图实例）」**——DWG-0201 的 Manifold 从此显示 `Left 57.91 mm · Right 114.30 mm · Top 35.59 mm`，不再只有
库默认 `Top 20.32 · Left 114.30 · Right 114.30` 与外框 `172.21 × 71.18`。

## 事实（2026-09-20，五图）

| 实例 | `JSymbolInformation` 副本（库默认） | `JFlavorHolder` 值（米） | 对几何 |
|---|---|---|---|
| 0201 Manifold `/JSite396` | 0.1143 / 0.1143 / 0.02032 | **0.057909 / 0.114300 / 0.035590** | 宽 172.209 ✓ 高 71.18 ✓ 弧 r 35.590035 ✓ |
| 0201 ` Line2` | Right 0.0254 | 0.0254 | = 默认 |
| D06 Cone Roof Tank `/JSite151` | 0.06096 ×2、0.035306 ×2 | 同默认 | 几何是同一公式按毫米算（09-18 §5） |
| 工艺 Black Box `/JSite6963` | 0.0127 ×4 | **0.0127 / 0.113927 / 0.0127 / 0.078067** | 宽 0.126627 ✓ 高 0.090767 ✓ |
| A01 Drum `/JSite121` | — | Left + Right = 0.159459、2 × Top = 0.051917 | = 本体外框 ✓ |

模板存储里的 `JFlavorHolder` 是另一变体：不带值，`+40` 指自己的 `JSymbolInformation`。每个嵌套存储里 holder 数 = `JSymbolInformation` 数。

## 决策（按推荐落笔）

| # | 决策 | 结论 |
|---|---|---|
| P-F1 | 值落在 DTO 哪 | **`PidSymbolVariable::instance_value_m: Option<f64>`**，与库默认 `value_m` 并排（一处看两值）；模板本体为 `None`。备选「`PidSymbolDefinition::instance_variables` 平行表」多一层对位 |
| P-F2 | holder ↔ `JSymbolInformation` 怎么配 | 两条记录互不点名、流序不定（0201 holder 在后、工艺在前），**按变量数配、同数按流序对位**（`JSiteSymbolInformation::instance_values_of`）；语料每存储最多两对且变量数不同 |
| P-F3 | 两个变体都解 | **都框住**（模板形留 `symbol_information_ref`，可验证与 `parent_ref` 互指），只有实例形带值 |
| P-F4 | OCS 写哪 | 放置实体 `PID_SEMANTICS` 加 **`instance=<name>:<mm>;…`**，与 `driving=` 同格式同顺序（变量表序）；面板 `pid_instance` 行 |
| P-F5 | 面板文案 | **「驱动尺寸（本图实例）」** / `Driving dimensions (this drawing's instance)`，21 个语言目录同补 |
| P-F6 | 没拉过的实例也写 | **写**（值 = 默认）：一行说「这张图上放成了多大」，不该因为等于默认就消失 |
| P-F7 | 用实例值重算几何 / 校验 | **不做**：几何已在缓存里；本单只是把参数说出来 |
| P-F8 | 摘要行 | **不改**：`parametric_placements` 计数不变 |

## 工作项

- **F1 pid-parse**（一提交）：`decode_flavor_holders` / `FlavorHolderDecoder`（`PSM_TYPE_CODE_JFLAVOR_HOLDER = 0x00ED`）；
  `JSiteSymbolInformation::flavor_holders` + `instance_values_of`；`streams/jsite.rs` 收；`PidSymbolVariable::instance_value_m`；
  棘轮 `a_placed_instance_states_its_own_parameters_in_its_flavor_holder`（五图）；CHANGELOG / 格式指南 / task_plan。
- **F2 OCS**（一提交）：`PlacementMeasures::instance` → `instance=`；`scene/cache/properties.rs` 解 `instance` 出 `pid_instance` 行；
  `locale_catalog.rs` + 21 个 `.ftl`；`pid_import::a_placement_states_its_extent_and_a_parametric_one_its_library_defaults` 加 `instance`
  一栏（四图五个放置：Manifold `Left:57.91;Right:114.30;Top:35.59`、Line2 `Right:25.40`、Tank `Left:60.96;Right:60.96;Bottom:35.31;Top:35.31`、
  Black Box `Left:12.70;Right:113.93;Bottom:12.70;Top:78.07`，非参数化放置无 `instance=`）；面板单测多一段；user-guide「符号的三个尺寸」。
- **F3 台账**：本单头部哈希；09-18 计划 K-D1 补一句；`remember`。

## 验收

- pid-parse：`cargo test --all-targets` 全绿（`parse_real_files` 134 → 135、`--lib` 1111 → 1112），golden 不变，`clippy --all-targets -D warnings`
  零告警，fmt 干净。
- OCS：`pid_import` 两种 `OCS_PID_LAYER_MODE` 全绿（53/53，测试数不变、断言多）；`--lib scene::cache::properties` / `i18n` / `io::pid::tests` 绿；
  `pid_panel_localization` 绿；`clippy --lib --test pid_import` 触碰处零命中；`rustfmt --check` 触碰文件干净。

## 登记不做

| 项 | 理由 |
|---|---|
| 实例形 `+20` 那个 oid、"Sheets" 后两个 u32 | 未坐实（398 / 598 / 204 / 7535，不是放置 graphic oid），留 raw `link` |
| 反汇编 `symbol.dex` `JFlavorHolder::Load` | 语料五对全对上，够用；字段要一个个坐实时再开 |
| 用实例参数驱动 / 重算几何 | P-F7 |

## 进度

### F1（pid-parse `7498bd9`）

照工作项做。一处小偏离：配对规则先写成「记录之后最近的 holder」，工艺的 holder 在记录**之前**（oid 13 @+0x4c6，记录 oid 27 @2415），
改成 P-F2 的按数按序。A01 的本体外框要按**全部**图元量（可见笔画量出 133.5，不是 159.459——端部在关闭层上）。

### F2（OCS `cedaecd5`）

照工作项做，无偏离。

**手工验收（2026-09-20，用户指示）**：起同一提交的 debug GUI 打开 0201，`ZOOM` 到 Manifold、点选壳体下底线，特性面板 P&ID 一节三行并排——
`驱动尺寸（库默认） Top 20.32 mm · Left 114.30 mm · Right 114.30 mm` / `驱动尺寸（本图实例） Left 57.91 mm · Right 114.30 mm · Top 35.59 mm` /
`本体尺寸 172.21 × 71.18 mm`。整窗 + P&ID 一节 2× 放大两张入 `docs/evidence/2026-09-20-instance-driving-dimensions/`（README 记了
`--export` DXF 对数：带 `instance=` 的实体恰 9 = Manifold 六笔 + 名字、` Line2` 一笔 + 名字）。一处观察：特性面板默认停靠宽度 250 下值列文字约 118 pt（≈ 19–20 个 ASCII），
三行都截成 `Top 20.32 mm · Left 11…`，截图时把宽度改到 560 才看全；面板文案本身没改 → 已开单
`2026-09-20-a-long-read-only-value-shows-itself-whole-on-hover.md`（放不下的只读值悬停给全文）→ **同日落地**（OCS `dbc56890`）：默认 250 宽下悬停
`驱动尺寸（本图实例）` 值框给全文 tip，悬停特写四张同入 `docs/evidence/2026-09-20-instance-driving-dimensions/`。

## 门禁记录

- 2026-09-20：分析（pid-parse `0e1a9b1`）→ 用户「开单并直接做」→ F1 `7498bd9` → F2 `cedaecd5`（会话 fable-5-1-8）。
- 2026-09-20：用户「起 GUI 打开 0201 点一下 Manifold，截一张三行尺寸的特性面板进 docs/evidence/」→ `docs/evidence/2026-09-20-instance-driving-dimensions/`
  两张（会话 fable-5-1-18，接手 fable-5-1-8）。
