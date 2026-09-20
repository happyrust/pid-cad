# 放置实例说出自己的驱动尺寸——特性面板三行尺寸的手工验收截图（2026-09-20）

计划 `docs/plans/2026-09-20-a-placed-instance-states-its-own-driving-dimensions.md` 落地（OCS `cedaecd5`，pid-parse `7498bd9`）后，
用同一提交编出的 debug 版 `OpenCADStudio.exe DWG-0201GP06-01.pid --new-instance` 打开 0201，命令行 `ZOOM` (270,150)–(480,255) 拉到
Parametric Manifold 实例，点选它壳体的**下底线**（(322.36, 167.42) → (423.39, 167.42)，`PID-SYMBOL` 层）后截的整窗截图（1567 × 837，
窗口最大化，150% 缩放）；`*-crop.png` 是同一张截图特性面板 P&ID 一节的 2× 像素放大。默认环境：`OCS_PID_LAYER_MODE` 未设（taxonomy）。
不是 SmartPlant 截图，是 OCS 自己的特性面板，作 `scene::cache::properties` 单测与 `pid_import::a_placement_states_its_extent_and_a_parametric_one_its_library_defaults`
钉住的 `instance=` 文案的人眼对照。

| 文件 | 看什么 |
|---|---|
| `0201-manifold-panel.png` | 整窗：右侧画布里 172 × 71 的拉长罐下底线被选中（蓝色带三个夹点），左侧特性面板 P&ID 一节四行：`角色 symbol`、**`驱动尺寸（库默认） Top 20.32 mm · Left 114.30 mm · Right 114.30 mm`**、**`驱动尺寸（本图实例） Left 57.91 mm · Right 114.30 mm · Top 35.59 mm`**、**`本体尺寸 172.21 × 71.18 mm`**，其下 `图纸图层 Default` / `图层 OID 8`。`一般` 一节里颜色 `128,0,0`、图层 `PID-SYMBOL`、线宽 `0.35 mm` 是放置样式 `Equipment - New` 的 |
| `0201-manifold-panel-crop.png` | 同上 P&ID 一节 2×：三行尺寸并排——库默认与本图实例的 `Left` 一个 114.30 一个 57.91、`Top` 一个 20.32 一个 35.59，`Right` 同为 114.30；`本体尺寸` 172.21 = 57.91 + 114.30，71.18 = 2 × 35.59 |

为了三行数值不被截断，截图前把特性面板的停靠宽度从默认 250 改到 560（`%APPDATA%\OpenCADStudio\settings.json` 的
`dock.panels.properties.width`，截完已还原）：默认宽度下值列文字只有约 118 pt（停靠宽的 6/11 减内边距，约 19–20 个 ASCII 字符），
三行都只露出前半句（`Top 20.32 mm · Left 11…`），47 字符的那行要停靠宽 ≥ ~545 才看全。已开单 `docs/plans/2026-09-20-a-long-read-only-value-shows-itself-whole-on-hover.md`
→ 同日落地（OCS `dbc56890`），手工验收见下一节。

## 放不下的只读值悬停给全文——手工验收（2026-09-20，计划 `2026-09-20-a-long-read-only-value-shows-itself-whole-on-hover.md` G3）

OCS `dbc56890` 编出的 debug 版，同一张 0201、同一个 `ZOOM` (270,150)–(480,255)、同一条壳体下底线点选；停靠宽**默认 250**（`settings.json` 没改），
窗口最大化 2560 × 1368 物理像素（150% 缩放），截图仍为 1567 × 837 整窗。悬停是把真实鼠标（`SetCursorPos`）停到值框上约 0.7 s 再截——
iced 的 `tooltip` 只认真实光标位置，PostMessage 的 `WM_MOUSEMOVE` 进不去。拖分隔条用的也是真实鼠标，拖完拖回，`settings.json` 里的
`dock.panels.properties.width` 仍是 `250.0`。

| 文件 | 看什么 |
|---|---|
| `0201-manifold-hover-instance.png` | 整窗，停靠宽 250：光标停在 `驱动尺寸（本图实例）` 值框（框里仍是截断的 `Left 57.91 mm · Right 1`），右侧跟着光标冒出一条 tip：**`Left 57.91 mm · Right 114.30 mm · Top 35.59 mm`** 全文，单行、`FONT_SZ`、底 `background.base` 边 `background.neutral` 圆角 4 |
| `0201-manifold-hover-instance-crop.png` | 同上 P&ID 一节 2×：tip 压在 `驱动尺寸（库默认）` 与 `驱动尺寸（本图实例）` 两行之间，`本体尺寸 172.21 × 71.18 mm` 那行没被遮 |
| `0201-manifold-hover-role-crop.png` | 光标停在 `角色 symbol` 值框，P&ID 一节 2×：**没有 tip**（放得下的值不冒泡，G-D2） |
| `0201-manifold-hover-xdata.png` / `-crop.png` | 面板滚到底，光标停在 `扩展数据 › 应用程序` 值框（框里只露 `PID_SEM`）：tip 把整条 `PID_SEMANTICS: String("sheet_layer=Default"), String("sheet_layer_oid=8"), String("role=symbol"), String("style=Equipment - New"), String("extent=172.21x71.18"), String("driving=Top:20.32;Left:114.30;Right:114.30"), String("instance=Left:57.91;Right:114.30;Top:35.59")` **折成五行**，块宽约 360 pt（`RO_TIP_MAX_W`），贴着面板右缘，没横穿画布；光标在窗底附近，tip 自己翻到光标上方 |
| `0201-manifold-dock-600-crop.png` | 分隔条拖到最宽（`DOCK_MAX_W` 600）后 P&ID 一节 2×：三行尺寸**整句可见**——`Top 20.32 mm · Left 114.30 mm · Right 114.30 mm` / `Left 57.91 mm · Right 114.30 mm · Top 35.59 mm` / `172.21 × 71.18 mm`；光标停在 `驱动尺寸（本图实例）` 值框上，**没有 tip**（估算在 600 宽下判「放得下」；560 下估算仍偏保守地判「放不下」，见计划 G-D2 / 进度） |

同一提交的 `--export DWG-0201GP06-01.pid <out>.dxf` 对数（与截图同一份几何，PowerShell 直接解析 DXF 组码 1000）：带 `instance=` 的实体恰 **9**——
Manifold 的六笔（`PID-SYMBOL` 上 4 LINE + 2 ARC）与它的名字（`PID-SYMBOL-LABEL` 上 1 TEXT）同写 `instance=Left:57.91;Right:114.30;Top:35.59`
（旁边 `driving=Top:20.32;Left:114.30;Right:114.30`、`extent=172.21x71.18`），` Line2` 的一笔 LINE 与名字同写 `instance=Right:25.40`
（= 默认，P-F6）；带 `driving=` 而不带 `instance=` 的实体 **0**。与集成测试在内存文档上钉的一致。
