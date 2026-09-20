# 缓存本体逐笔样式计划 E2 的手工验收截图（2026-09-20）

计划 `docs/plans/2026-09-20-a-cached-body-carries-its-own-stroke-styles.md` E2 落地（OCS `3b8af2ed`，pid-parse `7e69b8a` / `1fd69db`）后，
用同一提交编出的 debug 版 `OpenCADStudio.exe <图> --new-instance` 逐张打开语料，命令行 `ZOOM` 两角拉到目标处截的整窗截图
（1567 × 837，窗口最大化，150% 缩放）；`*-crop.png` 是同一张截图目标处的 2× 像素放大，方便看虚线的断口。默认环境：`OCS_PID_LAYER_MODE` 未设
（taxonomy）、`OCS_PID_SYMBOL_SOURCE` 未设（cache）。不是 SmartPlant 截图，是 OCS 自己画出来的样子，作
`a_cached_strokes_dash_is_its_own_storages_not_its_placements` 钉住的数字（工艺 45 / 0202 12 / D06 0 / 0201 0）的人眼对照。

E2 之前这些笔画全是实线（两条路都是）；颜色与线宽照旧是放置样式的（P-E1 / P-D5）——工艺 OPC 的 `#00FEA0` 0.50 底涂在屏幕上是放置的橄榄 0.35。

| 文件 | 图 | 看什么 |
|---|---|---|
| `gongyi-xa-opc.png` | 工艺管道及仪表流程-1，`ZOOM` (585,204)–(625,232) | 「火炬气至总…」那条管线尽头的 `Xa` 去外单元接续符号（缓存本体 (7559, 155)，9 笔可见）：**r 3.81 的圆是虚线**，箭杆的两条 4.86 mm 横线与两条 1.27 mm 竖线也是虚线（3.5 / 1.75 的图样在 4.86 mm 上只够一段实 + 一个断口，断口在圆与箭杆之间那一小截），箭头两条斜边（各画两遍）实线。斜着的两条细绿线是 `PID-CONNECTIVITY` 诊断层，不是符号 |
| `gongyi-xa-opc-crop.png` | 同上，目标处 2× | 虚线圆的断口、箭杆左端与圆之间的空隙 |
| `0202-arrester.png` | DWG-0202GP06-01，`ZOOM` (292,252)–(318,281) | `RD060201` 阻火呼吸阀（arrester breather valve(RD)，缓存本体 (793, 2817)，21 笔可见）：主体（三格框、顶上的梯形帽、底下的支腿）实线；左侧那圈旁通——x ≈ 302.5 的 6.33 mm 竖线、两截横线、几段 0.46–0.6 mm 的小段和那条 17 顶点的 B 样条**唇**——虚线，竖线中段能看见断口。横穿的蓝色虚线是图纸自己的虚线管线（`PID-DASH`，E2 之前就有） |
| `0202-arrester-lip.png` | 同上，`ZOOM` (298,256)–(312,277) | 再近一点：左侧旁通竖线的两处断口、唇 |
| `0202-arrester-crop.png` | `0202-arrester.png` 目标处 2× | 同上放大 |

同一提交的 `--export <图> <out>.dxf` 对数（与截图同一份几何，PowerShell 直接解析 DXF 组码）：工艺 `PID-SYMBOL` 层上 `linetype` 为 `PID-DASH-*` 的
实体恰 **45**（九个 OPC 各 5：圆 r 3.81 + 两横两竖，全部 `PID-DASH-1`），0202 恰 **12**（阻火呼吸阀 7 线 + 1 条 LWPOLYLINE 唇，Wastewater Pit
四条 127 / 152.4 mm 的长横线，全部 `PID-DASH-2`）——与集成测试在内存文档上钉的数字一致，虚线样式落到了文件里。

Wastewater Pit 没截：它的四笔虚线是 y 226.5 / 239.2 / 251.9 / 264.6 的 127–152 mm 长线，整图视野下就看得见，不需要特写。
