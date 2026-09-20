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
`dock.panels.properties.width`，截完已还原）：默认宽度下值列只有约 70 pt，三行都只露出前半句（`Top 20.32 mm · Left 11…`）。

同一提交的 `--export DWG-0201GP06-01.pid <out>.dxf` 对数（与截图同一份几何，PowerShell 直接解析 DXF 组码 1000）：带 `instance=` 的实体恰 **9**——
Manifold 的六笔（`PID-SYMBOL` 上 4 LINE + 2 ARC）与它的名字（`PID-SYMBOL-LABEL` 上 1 TEXT）同写 `instance=Left:57.91;Right:114.30;Top:35.59`
（旁边 `driving=Top:20.32;Left:114.30;Right:114.30`、`extent=172.21x71.18`），` Line2` 的一笔 LINE 与名字同写 `instance=Right:25.40`
（= 默认，P-F6）；带 `driving=` 而不带 `instance=` 的实体 **0**。与集成测试在内存文档上钉的一致。
