# 特性面板：放不下的只读值悬停给全文 · 小计划（2026-09-20 开单并同日落地）

> 承接 `2026-09-20-a-placed-instance-states-its-own-driving-dimensions.md` 进度 F2 手工验收里的一处观察：特性面板默认停靠宽 250 时，
> `驱动尺寸（库默认）` / `驱动尺寸（本图实例）` 两行都截成 `Top 20.32 mm · Left 11…`，截图（`docs/evidence/2026-09-20-instance-driving-dimensions/`）
> 是把 `settings.json` 里的宽度临时改到 560 才看全的。**2026-09-20 用户在会话（fable-5-1-18）里指示「开个小单：特性面板 P&ID 长值换行或悬停显示完整值」**。
> 七条决策按推荐落笔。**落地**：OCS `dbc56890`（G1 + G2），G3 证据 + 本单收口随后一提交。**状态：已落地。**

## 一句话

特性面板的只读值行是一个不可编辑的 `text_input`（`ui/read_only.rs`），单行、超出就裁；值列宽是停靠宽的 6/11。本单让**放不下的只读值悬停时给出全文**
（`iced::widget::tooltip`，跟随光标），放得下的一切照旧；不换行、不改任何行的文案、不动模型层。P&ID 三行是起因，但受益的是所有只读行——
几何 `起点 322.3602, 167.4175, 0.0000` 在 250 宽下同样只露到 `0.0`，`扩展数据` 的 `PID_SEMANTICS: String(…)` 一行几百字更是从来没看全过。

## 事实（2026-09-20，OCS `cedaecd5` 的面板）

| 项 | 数 | 出处 |
|---|---|---|
| 行高 / 字号 | `ROW_H` 26、`FONT_SZ = ROW_H × 0.42 ≈ 10.9 px` | `ui/mod.rs`、`ui/properties.rs:26` |
| 标签列 : 值列 | `FillPortion(5) : FillPortion(6)` | `prop_row_with_active`（`ui/properties.rs`） |
| 值列文字可用宽 | 停靠宽 × 6/11 − 容器内边距 2+2 − 输入框内边距 6+6 − 边框 2 ≈ **停靠宽 × 0.545 − 18**：默认 250 → **≈ 118 pt ≈ 19–20 个 ASCII**；560 → ≈ 287 pt | 同上 + `ui/read_only.rs`（`padding([3, 6])`） |
| 停靠宽范围 | 默认 250，可拖 200–600（`DOCK_MIN_W` / `DOCK_MAX_W`，还受窗宽 45% 限） | `ui/dock.rs` |
| 视图拿得到宽 | `PropertiesPanel::view(&self, width: f32, auto_collapse: bool)`，`width` 就是停靠宽 | `ui/properties.rs:545`、`app/view/mod.rs:2912` |
| 三行 P&ID 长度 | `Top 20.32 mm · Left 114.30 mm · Right 114.30 mm` 47 字符 ≈ 280 pt（要停靠宽 ≥ ~545）；`Left 57.91 mm · Right 114.30 mm · Top 35.59 mm` 45；`172.21 × 71.18 mm` 17（放得下） | 2026-09-20 截图 |
| 其他会裁的只读行 | 几何 `起点` / `端点` / `增量` 三段坐标 26 字符；`扩展数据` 的 `xdata_value` 整条记录 `format!("{value:?}")` 逗号拼起来，数百字符；P&ID `名称`（`arrester breather valve(RD)` 27）、`管线号`（多条拼接） | `app/properties.rs:910–930`、`scene/cache/properties.rs` |
| 只读值行现有的四处渲染 | `render_ro_row`、`render_ro_with_tooltip_row`（值 + 「为什么不能编辑」的 tip）、`render_annotative_scale_row`、分组摘要行（`joined`）——都调 `read_only::field(value, FONT_SZ, Length::Fill)` | `ui/properties.rs:1842/1876/1898/1907` |
| 今天的唯一补救 | 拖分隔条加宽，或在值框里 Ctrl+A / Ctrl+C 拿全文（`read_only.rs` 文档注释） | — |
| 现成的样板 | `render_ro_with_tooltip_row` 已经把值框包进 `tooltip(field, text(tip).size(FONT_SZ), Position::Top)`，带边框圆角样式；图层面板名字列用 `Position::FollowCursor` | `ui/properties.rs:1902–1925`、`ui/window/layers.rs:744` |

## 决策（按推荐落笔；落地时的偏离见「进度 G1」）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| G-D1 | 换行还是悬停 | **悬停给全文**。换行要把这一行从固定 `ROW_H` 变成自适应高——值框是 `text_input`，单行；换成 `text().wrapping(Word)` 就丢掉选择 / Ctrl+C（`read_only.rs` 明写的卖点），换成 `text_editor` 太重；而且激活行高亮、焦点滚动都按整行 `ROW_H` 算。悬停不动几何、不丢复制，且几百字的 XDATA 也不至于把面板撑成一屏 | ✅ |
| G-D2 | 什么时候挂 tip | **只在估算放不下时挂**：估算宽 = ASCII 0.6 × `FONT_SZ`、非 ASCII（CJK、`·`、`×`）1.0 × `FONT_SZ` 逐字累加；值列文字可用宽 = `width × 6/11 − 18`；估算 > 可用才包 tooltip。放得下的行不冒泡（`symbol`、`Default`、`8`、`172.21 × 71.18 mm`）。估算偏保守（0.6 而非 0.55）：多挂一个无害，漏挂才是没修 | ✅ 系数与列宽公式按实测微调，见进度 |
| G-D3 | tip 长什么样 | 全文原样、`FONT_SZ`、`Position::FollowCursor`（同图层面板名字 tip），框样式复用 `render_ro_with_tooltip_row` 那一套（底 `background.base`、边 `background.neutral` 1 px、圆角 4、内边距 6）；tip 里的文字 `wrapping(Word)` 并把 tip 容器限宽 ≈ 360 pt，几百字的 XDATA 值在 tip 里折行而不横穿屏幕 | ✅ `WordOrGlyph`，见进度 |
| G-D4 | 动不动模型层 | **不动**。溢出是渲染层按当下宽度才知道的事，`PropValue` 不加变体、`scene/cache/properties.rs` 与它的单测一字不改；`ReadOnlyWithTooltip`（说明为什么不能编辑）照旧，若它的值也放不下，tip 文本 = 全文 + 换行 + 原说明 | ✅ |
| G-D5 | 覆盖哪几处 | `ui/properties.rs` 里四处 `read_only::field` 收成一个 `ro_value_field(value, panel_width)`，全都走 G-D2；对话框里定宽的 `read_only::field`（`plot.rs`、`style/*.rs`）不在本单 | ✅ |
| G-D6 | 宽怎么传到行 | `view(width, …)` → `render_section(section, width)` → `render_prop_row(prop, label, width)` → 各 `render_*_row(…, width)`：四个签名加一个 `f32`；不存到 `self`（`view` 是 `&self`，也不想为一个数引入 `Cell`） | ✅ 快捷特性浮窗传定宽 230，见进度 |
| G-D7 | 三行 P&ID 文案要不要顺手缩短 | **不缩**：`Top 20.32 mm · Left 114.30 mm · Right 114.30 mm` 是 K-D5 定的口径（毫米两位、每项带单位），`scene::cache::properties` 单测与 `pid_import` 的 `instance=` 都钉着；缩成 `Top 20.32 · Left 114.30 · Right 114.30` 也还是 33 字符，250 宽下照样裁 | ✅ |

## 工作项

- **G1 `src/ui/properties.rs`**（一提交）：
  `fn estimated_text_width(value: &str, font_size: f32) -> f32`、`fn ro_value_column_width(panel_width: f32) -> f32`、
  `fn ro_value_field<'a>(value: &'a str, panel_width: f32) -> Element<'a, Message>`（放得下 → 原 `read_only::field`；放不下 → 包 `tooltip`）；
  四处调用改走它；`width` 按 G-D6 下传；`render_ro_with_tooltip_row` 按 G-D4 合并 tip 文本。
- **G2 单测**（同一提交，`ui/properties.rs` 的 `mod tests`）：
  `estimated_text_width` 对纯 ASCII / 纯 CJK / 混合三例单调且 CJK 更宽；`ro_value_column_width(250.0)` 落在 [110, 125]、`(560.0)` 落在 [280, 295]；
  三行 P&ID 文案在 250 宽下「放不下」、560 宽下「放得下」，`symbol` / `172.21 × 71.18 mm` 两宽都「放得下」（钉的是判定函数，不是 iced 布局）。
- **G3 手工验收**：默认 250 宽打开 0201 点 Manifold，悬停 `驱动尺寸（本图实例）` 值框 → tip 给 `Left 57.91 mm · Right 114.30 mm · Top 35.59 mm` 全文；
  悬停 `角色 symbol` → 无 tip；悬停 `扩展数据` 那行 → tip 折行、宽不过 ~360 pt；拖到 560 → 三行都不再冒 tip。
  两张入 `docs/evidence/2026-09-20-instance-driving-dimensions/`（与起因同一夹，README 补两行）。
- **G4 台账**：本单头部写哈希；`2026-09-20-a-placed-instance-states-its-own-driving-dimensions.md` 进度 F2 那句观察补「→ 本单」。

## 验收

- `cargo test --lib ui::properties` 绿（5 → 5 + 新增），`cargo test --lib scene::cache::properties` 数字不变（G-D4）；`pid_panel_localization` 绿。
- `rustfmt --check src/ui/properties.rs` 干净；`cargo clippy --lib` 在 `ui/properties.rs` 零命中。
- G3 四个悬停场景与截图。

## 登记不做

| 项 | 理由 |
|---|---|
| 值行换行 / 自适应行高 | G-D1：丢复制、动行高几何；等有第二个非要整行看全的场景再议 |
| 改默认停靠宽 250 | 是用户布局的默认值，且 47 字符的行要 ≥ 545 才看全，改默认治不了本 |
| 缩短 P&ID 三行文案 | G-D7 |
| 对话框里定宽的只读框 | 各有自己的定宽与场景，不在特性面板这条路上 |
| 按真实字体量宽（`Paragraph` 测量） | 视图函数里拿不到渲染器；0.6 em 的保守估算够用，多挂一个 tip 无害 |

## 进度

### G1 + G2（OCS `dbc56890`，`src/ui/properties.rs` 一文件，+251 / −35）

照工作项做：`estimated_text_width` / `ro_value_column_width` / `ro_value_fits` / `ro_value_field` / `ro_tip` / `ro_tip_style`；四处 `read_only::field`
收成 `ro_value_field(value, panel_width)`（`render_ro_row`、`render_group_row` 的分组摘要、`render_annotative_scale_row`），`render_ro_with_tooltip_row`
按 G-D4 合并 tip 文本（值放不下 → `全文\n原说明`，`Position::Top` 不变）；宽按 G-D6 下传 `view` → `render_section` → `render_prop_row` →
各 `render_*_row`，`render_choice_row` 没有 combo 状态时退回的 `render_ro_row` 也带宽。单测 `a_read_only_value_the_column_cannot_show_whole_is_told_from_one_it_can`
（`ui::properties` 5 → 6）。五处偏离：

1. **估算系数**（G-D2）：ASCII 小写 / 数字 / 标点 **0.55** em、ASCII 大写 **0.65** em、全宽字（`is_wide`：东亚宽 / 全宽区段，CJK、谚文、全宽符号）1.0 em；
   计划写的是 ASCII 一律 0.6、非 ASCII 一律 1.0。按 2026-09-20 在面板字体上实测的 0.47–0.51 em 往上取整；`·` `×` 是 Latin-1，按 ASCII 计（计划把它们算作 1.0）。
2. **值列可用宽**（G-D2）：`(width − 10 − 8) × 6/11 − 18`——`scrollable` 内嵌的滚动条 10 px 与 `spacing(8)`（现为 `SCROLLBAR_W` / `SCROLLBAR_SPACING`）先从内容宽里扣掉，
   再按 5 : 6 分、再减列与值框的 18 px 装饰；计划写的是 `width × 6/11 − 18`。250 → **≈ 108 pt**（计划估 ≈ 118）、560 → ≈ 278、600 → ≈ 299。
3. **两条 47 / 46 字符的驱动尺寸文案在 560 宽下估算仍判「放不下」**（≈ 286 / 280 pt 对 278 pt）——F2 的 560 宽截图里它们其实整句可见、还各余 35 / 50 pt，
   这是估算按设计偏保守；**600（`DOCK_MAX_W`）下判「放得下」**。单测钉 250 ✗ / 560 ✗ / 600 ✓，G3 的「拖到 560 → 不再冒 tip」相应改成拖到 600。
4. **tip 折行**（G-D3）：`Wrapping::WordOrGlyph` 而非 `Word`——XDATA 值里 `String("driving=Top:20.32;Left:114.30;Right:114.30"),` 这种没有空格的长 token 在 `Word` 下会撑破 360 pt。
5. tip 容器宽 = `min(估算(tip 文本) + 8, 360)`：单行 tip 留 8 pt 余量，估算偶尔短一丝也不至于折成两行。

另：`quick_view`（快捷特性浮窗）原来的字面量 `.width(230)` 收成 `QUICK_VIEW_W`，作为它的 `panel_width` 传下去（G-D6 的补充）。

### G3 手工验收（2026-09-20，`docs/evidence/2026-09-20-instance-driving-dimensions/`）

`dbc56890` 编出的 debug GUI 打开 0201，`ZOOM` (270,150)–(480,255)，点选 Manifold 壳体下底线，停靠宽默认 250，真实鼠标悬停（iced 的 tooltip 只认真实光标，
PostMessage 的 `WM_MOUSEMOVE` 进不去）：

| 场景 | 结果 | 文件 |
|---|---|---|
| 250 宽，悬停 `驱动尺寸（本图实例）` 值框 | tip 跟着光标给出 **`Left 57.91 mm · Right 114.30 mm · Top 35.59 mm`** 全文，单行 | `0201-manifold-hover-instance.png` + `-crop.png` |
| 250 宽，悬停 `角色 symbol` | **无 tip** | `0201-manifold-hover-role-crop.png` |
| 250 宽，面板滚到底，悬停 `扩展数据 › 应用程序`（`PID_SEMANTICS: String(…)` 七段） | tip **折成五行**，块宽 ≈ 360 pt，贴面板右缘，不横穿画布；光标近窗底，tip 翻到光标上方 | `0201-manifold-hover-xdata.png` + `-crop.png` |
| 分隔条拖到 600（最宽），悬停 `驱动尺寸（本图实例）` | 三行尺寸整句可见，**无 tip** | `0201-manifold-dock-600-crop.png` |

拖完拖回 250；`settings.json` 的 `dock.panels.properties.width` 仍 `250.0`（拖动会即时写盘，写成了 `250.00002`，手工改回 `250.0`）。GUI 已关，临时文件已清。

### G4 台账（证据 + 收口提交）

本单头部哈希；`2026-09-20-a-placed-instance-states-its-own-driving-dimensions.md` 进度 F2 那句观察补「→ 已落地」；证据 README 补一节。

### 验证

- `cargo test --lib ui::properties` **5 → 6** 绿；`cargo test --lib scene::cache::properties` **4 / 4**，一字未改（G-D4）；`pid_panel_localization` 1 绿。
- `rustfmt --check src/ui/properties.rs`：这文件从来不是 rustfmt 干净的——HEAD 43 处、现 41 处，**新增与改动的代码里 0 处**（41 处全是旧代码，如 `render_entity_link_row` 的签名折行）；没有顺手全文件格式化，免得 43 处噪音混进这一单。
- `cargo clippy --lib`：`ui/properties.rs` 命中 2 处，都是旧代码（第 168 行 `clone_on_copy`、第 865 行 `explicit_auto_deref`），改动处 0 命中；整仓另有 1142 条旧告警，与本单无关。

## 门禁记录

- 2026-09-20：F2 手工验收观察 → 用户「开个小单：特性面板 P&ID 长值换行或悬停显示完整值」→ 本单（会话 fable-5-1-18）。七条决策等批。
- 2026-09-20：G1 / G2 在上一会话（fable-5-1-18）写好未提交；fable-5-1-28 接手（用户交接「继续未完的部分，不必重问」）：核对 G1 / G2 并提交 `dbc56890`，
  做 G3 手工验收、G4 收口再一提交。七条决策未另走门禁，按推荐落地（偏离五处见进度 G1）。
