# `.pid` GUI 验收（2026-09-24，计划 `2026-09-24-pid-import-status-and-next-steps.md` T3 / V1）

桌面自动化（cua-driver）开本机 debug 版 OCS（`D:\Rust\target\debug\OpenCADStudio.exe`，含 T2 与 pid-parse `686c9d5`），Vulkan 后端，窗口 1567 × 837。
命令都从命令行敲（`ZW` 窗口缩放、`LAYER`），F2 展开命令行历史。

| 文件 | 看什么 | 结论 |
|---|---|---|
| `0201-command-history.png` | 打开 DWG-0201 的 `.pid` 后的命令行历史 | ⑤ 单的三行：`P&ID import: 334 entities from 206 decoded records; 0 source records not drawn; 309 entities on 4 authored sheet layers (0 unresolved)`、`P&ID 图纸图层：4 个，其中 1 个初始关闭`、`P&ID 驱动尺寸：4 条，在 2 个模板本体上；2 个放置的参数化本体带库默认值`。图纸声明了米，**没有**单位回退那一行。第一行的 `0 source records not drawn` 是 pid-parse `686c9d5`（`DependencyObject` 按成员数校验）之后的数，之前是 1 |
| `0201-saved-dxf-reopened-command-history.png` | 打开由这张 `.pid` 另存的 DXF（与 `--export` 同一条 `io::save`）之后的命令行历史 | 只有「打开了 … — 335 实体」，**没有任何 P&ID 导入行**：摘要不随另存落盘 |
| `0201-layer-manager.png` | 图层管理器「图层」视图（图层槽单 H4） | 列的是原图自己的图层名（`ConsistencyChecks` / `Default` / `DrawingBorder` / `Labels` …），`Heat Trace` / `Hidden` / `HiddenObjects` / `Label` 按文件的显示位为关；`0201-layer-table-command-line.png` 是 `LAYER` 命令打印的同一张表，另有 `PID-FRAME` 开、`PID-CONNECTIVITY` / `PID-SYMBOL-LABEL` 关 |
| `0201-tags-before-after.png`（及三张单图） | 仪表位号特写，窗口 (350, 248)–(395, 292) mm。左：改前（`dbf62cb5` 导入另存的 DXF 重新打开）；中：改后、同一条路（T2 导入另存的 DXF 重新打开）；右：改后直接打开 `.pid` | 改前字高是段落默认 3.175 mm，`060101` 比气泡还宽、`LIA` 被 `L=300 mm` 压住；改后是 run 说的 7 pt = 2.469 mm，放得进气泡；直接打开 `.pid` 时按 run 的 Arial Narrow 画 |

顺带看到的（与本单无关，登记待查）：另存的 DXF 重新打开时，文字按 `txt` 笔画字体画（左、中），直接打开 `.pid` 才是 TrueType（右）——
DXF 的 STYLE 组码 3 写的是 `txt`，TrueType 字体名在重读时没有接回来。是 DXF 往返的问题，不是 `.pid` 导入的问题。
