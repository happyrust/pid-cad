# 缓存优先计划 C2 的手工验收截图（2026-09-20）

计划 `docs/plans/2026-09-19-draw-the-cached-body-first-and-the-library-only-when-the-drawing-carries-none.md` C2 落地（OCS `641cec3b`，
pid-parse `08fc95a` / `b6a70a7`）后，用同一提交编出的 debug 版 `OpenCADStudio.exe <图> --new-instance` 逐张打开语料，命令行 `ZOOM`
窗口拉到目标处截的整窗截图（1567 × 837，窗口最大化，150% 缩放）。默认环境：`OCS_PID_LAYER_MODE` 未设（taxonomy）、
`OCS_PID_SYMBOL_SOURCE` 未设（cache）；符号库 `test-file/symbols*` 在图纸目录旁能找到，所以这里看到的是**库在场时仍画缓存**。
不是 SmartPlant 截图（P-D8 的那张仍没有），是 OCS 自己画出来的样子，作 `a_placement_draws_the_body_the_drawing_carries_and_skips_its_hidden_layers`
逐笔钉住的数字的人眼对照。

| 文件 | 图 | 看什么 |
|---|---|---|
| `0201-overview.png` | DWG-0201GP06-01 | 整图：左侧橄榄色的放空管束、右侧栗色的 A3-06D01 拉长罐 |
| `0201-manifold.png` | 同上，`ZOOM` (270,150)–(480,255) | Parametric Manifold **实例**：172 × 71 的拉长罐，两端帽 r 35.59 **向外凸**，罐内**没有**四条 `Construction[OFF]` 轴线短线；顶上三个 Flanged Nozzle、人孔、量油孔，上方 LG / LT 量表（各一圈 12.70，没有 7.57 的 `Heat Trace` 外圈）。库本体时这里是 228.6 × 40.64 的模板，且按逆时针读弧时端帽是内凹的 |
| `gongyi-overview.png` | 工艺管道及仪表流程-1 | 整图：中央红色 Black Box 与橄榄色管线；概览里几排像点的东西是管线标签的宽间距文字，不是符号 |
| `gongyi-remarks.png` | 同上，`ZOOM` (735,535)–(765,560) | `注：` / `1、仪表位…` 两行行首各一个 **1.27 mm 的三线小标记**——`Remarks` 的缓存本体；库本体时是 27.21 × 23.13 的云线（5 线 8 弧 1 圆），一图 35 处 |
| `d06-overview.png` | D06 | 整图：左侧 Cone Roof Parametric Tank 实例（六条线 122.12 × 82.84），右侧管线上两个球阀与 PT |
| `d06-valves.png` | 同上，`ZOOM` (282,200)–(332,252) | Ball Valve Type 1 **只一圈** r 1.27（库多画的第二张 sheet 那圈 1.59 + 六条线没有了）、2 Way Ball 一圈 r 1.59、PT 气泡一圈 r 6.35（没有 `Heat Trace` 上的 7.57 外圈）、`BV01` 是图纸自己的文字 |

同一导入的 `--export <图> <out>.dxf` 对数（与截图同一份几何）：D06 全图恰三个 CIRCLE（r 1.27 @ (292.44, 214.58)、r 1.59 @ (321.66, 214.58)、
r 6.35 @ (301.72, 241.50)）；0201 两条 r 35.59 的 ARC——圆心 (322.36, 203.01) `90° → 270°`、(423.39, 203.01) `270° → 90°`，DXF 逆时针读法下
分别向左、向右凸出壳体，壳体 101.03、连端帽 172.21；工艺 35 个 `Remarks` 符号名标签落在 11 个位置（同一处叠放三次）。
