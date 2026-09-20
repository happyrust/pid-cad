# 特性面板：放不下的只读值悬停给全文 · 小计划（2026-09-20 开单）

> 承接 `2026-09-20-a-placed-instance-states-its-own-driving-dimensions.md` 进度 F2 手工验收里的一处观察：特性面板默认停靠宽 250 时，
> `驱动尺寸（库默认）` / `驱动尺寸（本图实例）` 两行都截成 `Top 20.32 mm · Left 11…`，截图（`docs/evidence/2026-09-20-instance-driving-dimensions/`）
> 是把 `settings.json` 里的宽度临时改到 560 才看全的。**2026-09-20 用户在会话（fable-5-1-18）里指示「开个小单：特性面板 P&ID 长值换行或悬停显示完整值」**。
> 只开单，未做。带 ⭕ 的决策按推荐落笔，等批。

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

## 决策（按推荐落笔，等批）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| G-D1 | 换行还是悬停 | **悬停给全文**。换行要把这一行从固定 `ROW_H` 变成自适应高——值框是 `text_input`，单行；换成 `text().wrapping(Word)` 就丢掉选择 / Ctrl+C（`read_only.rs` 明写的卖点），换成 `text_editor` 太重；而且激活行高亮、焦点滚动都按整行 `ROW_H` 算。悬停不动几何、不丢复制，且几百字的 XDATA 也不至于把面板撑成一屏 | ⭕ |
| G-D2 | 什么时候挂 tip | **只在估算放不下时挂**：估算宽 = ASCII 0.6 × `FONT_SZ`、非 ASCII（CJK、`·`、`×`）1.0 × `FONT_SZ` 逐字累加；值列文字可用宽 = `width × 6/11 − 18`；估算 > 可用才包 tooltip。放得下的行不冒泡（`symbol`、`Default`、`8`、`172.21 × 71.18 mm`）。估算偏保守（0.6 而非 0.55）：多挂一个无害，漏挂才是没修 | ⭕ |
| G-D3 | tip 长什么样 | 全文原样、`FONT_SZ`、`Position::FollowCursor`（同图层面板名字 tip），框样式复用 `render_ro_with_tooltip_row` 那一套（底 `background.base`、边 `background.neutral` 1 px、圆角 4、内边距 6）；tip 里的文字 `wrapping(Word)` 并把 tip 容器限宽 ≈ 360 pt，几百字的 XDATA 值在 tip 里折行而不横穿屏幕 | ⭕ |
| G-D4 | 动不动模型层 | **不动**。溢出是渲染层按当下宽度才知道的事，`PropValue` 不加变体、`scene/cache/properties.rs` 与它的单测一字不改；`ReadOnlyWithTooltip`（说明为什么不能编辑）照旧，若它的值也放不下，tip 文本 = 全文 + 换行 + 原说明 | ⭕ |
| G-D5 | 覆盖哪几处 | `ui/properties.rs` 里四处 `read_only::field` 收成一个 `ro_value_field(value, panel_width)`，全都走 G-D2；对话框里定宽的 `read_only::field`（`plot.rs`、`style/*.rs`）不在本单 | ⭕ |
| G-D6 | 宽怎么传到行 | `view(width, …)` → `render_section(section, width)` → `render_prop_row(prop, label, width)` → 各 `render_*_row(…, width)`：四个签名加一个 `f32`；不存到 `self`（`view` 是 `&self`，也不想为一个数引入 `Cell`） | ⭕ |
| G-D7 | 三行 P&ID 文案要不要顺手缩短 | **不缩**：`Top 20.32 mm · Left 114.30 mm · Right 114.30 mm` 是 K-D5 定的口径（毫米两位、每项带单位），`scene::cache::properties` 单测与 `pid_import` 的 `instance=` 都钉着；缩成 `Top 20.32 · Left 114.30 · Right 114.30` 也还是 33 字符，250 宽下照样裁 | ⭕ |

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

（未开工）

## 门禁记录

- 2026-09-20：F2 手工验收观察 → 用户「开个小单：特性面板 P&ID 长值换行或悬停显示完整值」→ 本单（会话 fable-5-1-18）。七条决策等批。
