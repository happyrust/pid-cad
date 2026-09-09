# 开发计划：SVG 导出 · 第二阶段（P4–P7）

> 日期：2026-09-08
> 状态：§6 五条已于 2026-09-08 拍板（全按建议）；**P4.1 已实施**（§8）、**P5.1 已实施**（§9）、**P6.1 已量**（§7，
> 结论：P6.2–P6.4 不做）、**P4.2 已实施**（§10）、**P4.3 已实施**（§11）、**P7 已实施**（§12）、**P5.2 已实施**（§13；真图栅格 FAIL **已定性=判据局限，非导出缺陷**，见 evidence README 的 triage 节）、**G10 已实施**（2026-09-09，`PlotRequest.dialog_area`，记录在
> `docs/plans/2026-09-09-svg-export-audit-and-next-steps.md` §6.3）、**P5.3 已实施**（2026-09-09，
> `scripts/svg-compat.ps1` + `compare_external_renders`，记录在 09-09 计划 §6.4；merge_lines 只有 resvg 丢）。
> 本文件的分期全部收口。
> 前置：`docs/plans/2026-09-07-dxf-to-svg-export.md`（v2）——P0 / P1 / P2（web 除外）/ R3 第一轮 / R1 已实施。
> 本文件只写「还没做的」与「怎么做」；已实施部分的记录仍在 v2 的 §11–§15，不重复。

## 0. 一句话

SVG 导出的**引擎已经完整**：一份 emitter、两个 sink、22 例共用语料、三层验收里的两层半、缺字严格模式，
今天 `cargo test --lib -- io::svg_export io::pdf_export app::automation` **61 过 / 0 败 / 2 忽略**。
剩下的全是**产品面与债务**：web 入口是哑的、出图对话框还不知道 SVG 这个目的地、R4 那个组间 cap/join
缓存缺陷两个后端一起带着、R3 第二梯队（流式 / 字形缓存 / release 基线）与第三层栅格对比还欠着。
下一步**先补入口与修债，再量体量，最后才优化**。

## 1. 现状核对（2026-09-08 实测，HEAD `ef149b1f`）

### 1.1 代码形状

| 文件 | 体量 | 职责 |
|---|---|---|
| `src/io/plot_types.rs` | 3 KB | `PlotWire` / `PdfPlotOptions` / `PlotGroupSplits` / `PdfPageInput::as_plot_page`（页级 CTB 优先规则唯一一份） |
| `src/io/plot_emit.rs` | 60 KB | **唯一的**纸面语义展开：`PlotOp`（15 变体，含 `FillMesh` / `BuiltinText`）、`PlotSink`、`RecordingSink`、`GlyphSnapshot` / `PlotAssets` / `PlotReport`、`emit_plot_content` |
| `src/io/pdf_export.rs` (+`legacy_reference.rs`) | 29 KB + 49 KB(test-only) | `PdfSink`（`FillMesh` 按原序展开成一三角一 `DrawPolygon`）、文档 / 保存 / 对话框；旧 exporter 逐字冻结做操作级回归 |
| `src/io/svg_export.rs` (+`tests.rs`) | 42 KB + 65 KB(test-only) | `SvgSink`、`write_svg_page` / `svg_page_to_string`、D3 文件层 `export_svg_pages`（编号 / 撞名 / 拒覆盖 / 临时文件发布 / `Partial` 点名）、`.svgz`、R5 网格提边界、R1 `MissingGlyphs::{Refuse,Report}` |
| `src/io/plot_corpus.rs` | 21 KB(test-only) | 22 例语料，PDF 与 SVG 共用 |
| `src/app/update/file.rs` | — | `PlotRequest` / `PlotJob` / `resolve_plot_job`（任务级字形快照 + 失配重排一次）/ `set_headless_model_page` / `on_svg_export_path_some` |
| `src/app/automation.rs` · `src/cli.rs` · `src/main.rs` | — | `--plot-svg IN OUT`、`--layout` / `--model`（`--paper` `--landscape` `--fit` `--scale`）、`--ctb`、`--dry-run`、`--force`、`--allow-missing-glyphs`、`--list-layouts` |
| `src/app/commands/{mod,display}.rs` · `update/mod.rs` | — | `EXPORTSVG` / `SVGOUT` → `Message::SvgExport` → 保存对话框 → `SvgExportPath` |

工作树：`svg_export.rs` / `tests.rs` / `plot_corpus.rs` 对 HEAD **只有 CRLF 差异**（`git diff --ignore-cr-at-eol` 为空），没有未提交的内容改动。
v2 计划的 §11–§15 与代码逐条对得上。

### 1.2 验收现况

| 层 | 状态 |
|---|---|
| 第一层：PDF 操作级 / 结构级 / 条件字节级 | ✓ 9/9（`FillMesh` 展开后与冻结 exporter 逐 op 逐位相同；存盘除 `/ID` 外字节相同） |
| 第二层：写出的 SVG 用 usvg 0.45.1 独立解析回纸面 mm 与操作流逐形状比 | ✓ 22 页 + 5 种故障注入全部被抓到 |
| 第三层：受控栅格（resvg） | **半** ✓ ——100 mm 标尺 / wipeout 遮盖 / 网格无缝 / 缺字局部检出做了；**PDF ↔ SVG 栅格对比没做**（树里没有锁定版 PDF 栅格引擎） |
| 兼容性（浏览器 / Inkscape / Illustrator / librsvg） | ✗ 未实测 |
| 真图 | 11 张（`cad/0版重新处理dxf-12张`，FF02-04 被锁）`--model --paper A1 --landscape --fit` 全部出图；R1 严格默认下 11/11 与 R3 批次 SHA-256 相同 |
| 性能 | debug 每张 8–12 s（含启动 + 解析 + 建场景）；**release 耗时与峰值内存没量** |
| wasm | `cargo check --lib --target wasm32-unknown-unknown` 通过（`plot_emit` / `plot_types` / `svg_export` 都进 web 构建） |

## 2. 缺口清单（按代码核对，非按计划抄）

| # | 缺口 | 证据 | 影响 |
|---|---|---|---|
| G1 | **web 上 `EXPORTSVG` 是哑的** | `svg_export.rs::pick_svg_path_owned`（wasm）恒回 `None` → `update/mod.rs` 的 `SvgExportPath(None) => Task::none()`；`file.rs:3482` 那句「SVG export is not available in the web version yet」**永远到不了** | 用户点了没反应、没提示。写入器本身跨平台，`svg_page_to_string` 与 `crate::sys::download_bytes`（DXF 另存、恢复报告已在用）都在 |
| G2 | **R4 组间 cap/join 缓存缺陷仍在** | `plot_emit.rs:496-497` 每个 render group 开头 `last_cap/last_join = Some(Round)`，两组之间**没有** `Save/Restore` 也没有发 `LineCap/LineJoin` 重置（`Restore` 只在 :743-748 页尾） | 第一组末尾留下 CTB 的 Butt/Miter、第二组第一根要 Round 时，Round **不会被发出**——PDF 与 SVG 一起错。v2 的 R4 点名「单独修」，至今未修 |
| G3 | **出图对话框不认识 SVG** | `ui/window/plot.rs:19-20` 目的地哨兵只有 `OUT_DEFAULT` / `OUT_PDF`；`to_file: bool`；动作按钮 `Export PDF`（:520）；`stamp` 开关（:163）对 SVG 不置灰 | D5「接入现有出图设置界面，只切换目标格式」只做了一半：设置是共享的，但**目的地选择不在对话框里**，`stamp=true` 到导出时才报错 |
| G4 | **GUI 只出当前一页** | `file.rs:3500` `PlotRequest::current_view()`；`PRINTALL` 那条「所有布局」只通 PDF | CLI 不带 `--layout` = 全部布局，GUI 做不到，两边不对称 |
| G5 | GUI 走 `force: true` | `file.rs:3518` | 单页时对话框已经确认过覆盖，没问题；一旦 G4 做了多页，编号页会**无确认覆盖** |
| G6 | D4 参数面剩余 | `cli.rs` 没有 `--units` / `--orientation <v>` / `--margins` / `--preset`；`set_headless_model_page` 只认 `A0…A4`（`file.rs:3422`） | 无单位图纸默认当 mm；ANSI / 自定义纸张进不了脚本 |
| G7 | R3 第二梯队 | `SvgSink.body: String` 整页内存拼接（`<defs>` 要在前）；`mesh_outline` 每次 `FillMesh` 现算（同一字形出现 N 次提 N 次）；无受限路径合并 | 体量与耗时上限未知——**先量再改** |
| G8 | 第三层另一半 + 兼容性 | 见 §1.2 | 「同一张图」的最终判据还差 PDF ↔ SVG 栅格对比与外部渲染器实测 |
| G9 | 小项 | 保存对话框过滤器只有 `svg`（`svg_export.rs:415`），`.svgz` 只能手输；`PlotReport.text_items` 只记不显示；PDF 对缺字仍旧宽松（历史行为，是产品决定不是 bug） | — |
| G10 | **模型空间 SVG 不看出图区域**（P4.2 时发现） | `Message::SvgExport` → `direct_plot_params()` 在模型空间恒取 Extents；对话框里 PDF 目的地按 Window / Display / Limits / View 裁（`PlotWindowExport`） | 对话框选了 Window 再选 SVG 目的地，出的是 Extents。修法要么 `PlotRequest` 带区域，要么 `direct_plot_params` 认区域（它被菜单 Export PDF 与打印共用，改它会改 PDF 行为）；单独一期。**已实施（2026-09-09）**：走第一条路，`PlotRequest.dialog_area`，`direct_plot_params` 没动——见 09-09 计划 §6.3 |

不在清单里、也**不打算做**的（沿用 v2 §5）：SVG 导入、按图层重组 `<g>`（`WireModel` 无图层字段）、`<pattern>` / `<mask>` / 渐变、
`<text>` 图章、透明底图模式。

## 3. 分期

原则：**入口先通、债先修、量了再优化**。每期一个提交（或极少几个），PDF 第一层护栏每期都要绿。

### P4 产品面收口（小、价值高）

**P4.1 web 导出（G1）** — ✓ 已实施，记录在 §8

- wasm 分支的 `Message::SvgExport` 不再经过「选路径」：直接 `resolve_plot_job(current_view)` → `svg_page_to_string` →
  `crate::sys::download_bytes("{stem}.svg", bytes)`；错误进命令行（与 native 同一套文案）。
- 严格缺字默认与 native 一致；`stamp=true` 同样显式报错。
- 多页：GUI 今天只出一页（G4），web **首版只下载一页**；等 G4 落地再决定「顺序触发 N 次下载」还是加 `zip` 依赖（§6 Q1）。
- 出口：`cargo check --target wasm32-unknown-unknown` 过；native 侧加一条用例钉住「web 路径拿到的字符串与 native 写盘的字节相同」
  （两边都是 `write_svg_page`，比较的是包装层没绕开它）；真机 `trunk serve` 点一次拿到下载（人工，记进 evidence）。

**P4.2 出图对话框里的 SVG 目的地（G3，D5 补完）** — ✓ 已实施，记录在 §10

- `OUT_SVG = "Save to SVG file…"` 进「Printer / plotter」下拉，与 `OUT_PDF` 同级；`to_file: bool` 换成
  `Destination { Printer, Pdf, Svg }`（或保留 `to_file` 再加一位，取改动最小者）；页面设置持久化的 `printer_name`
  照 `OUT_PDF` 的写法存哨兵。
- 动作按钮随目的地变：`Export SVG`；`stamp` 开关在 SVG 下**禁用并说明原因**（不是静默忽略）。
- 「打印 / 出图」主按钮走到 SVG 时复用 `Message::SvgExport` 的路径；`EXPORTSVG` / `SVGOUT` 命令保留为快捷入口。
- `pick_svg_path_owned` 的过滤器加 `svgz`（G9）。
- 出口：对话框三种目的地各走一遍（自动化测试用 `plot_dialog` 状态直接驱动，不开窗口）；页面设置保存 / 读回 `OUT_SVG` 不丢。

**P4.3 GUI 多页 SVG（G4 + G5）** — ✓ 已实施，记录在 §11

- `PRINTALL` 的布局勾选面（`print_all_layouts` / `print_all_settings_override`）对 SVG 开放：`PlotRequest::layouts(names)` →
  `export_svg_pages` 的 D3 编号。
- 覆盖：先算 `page_paths` 并查已存在的文件，**一次确认整个集合**（列出会覆盖的名字），确认后才 `force: true`；
  `Partial` 错误原样进命令行。
- 出口：两个布局 → `stem-001.svg` / `stem-002.svg`；已存在其一 → 先问；取消 → 磁盘不动。

### P5 债务与验收补齐

**P5.1 R4 修复（G2）** — ✓ 已实施，记录在 §9

- 修法：每个 render group 开头 `last_cap = None; last_join = None`（首根 wire 必发一次 cap/join），或在组边界显式发
  `LineCap(Round)/LineJoin(Round)`——取前者，Op 更少且语义直白。
- **这是行为变更**：PDF 的 Op 流在「第一组以 CTB Butt/Miter 结尾、第二组以 Round 开头」时多出两个 op。
  处理：语料加一条反例（先在旧实现上证明它输出错误的 cap），`legacy_reference.rs` **同步修**并在提交信息里点名这是
  唯一允许改动冻结文件的理由；其余 21 例逐位不变。
- 出口：反例在修前红、修后绿；`saved_pdf_bytes_match_the_frozen_exporter_apart_from_the_trailer_id` 仍全绿。

**P5.2 第三层：PDF ↔ SVG 栅格对比（G8）**

- 栅格引擎：**不进依赖树**。测试启动时探测 `mutool` 或 `pdftoppm`（`OCS_PDF_RASTERIZER` 指路径），没有就 `#[ignore]`
  并说明；锁定版本号写进 evidence。
- 口径照 v2 §8 第三层：600 dpi、白底、统一抗锯齿参数；**局部**边缘距离（1 px 初始）、平坦色块 1–2/255、缺失内容按局部着墨
  直接失败；先用「同一 PDF 在两个引擎里的差异」建噪声基线，再注入删字 / 错比例 / 错 dash phase / 交换 wipeout 顺序证明判据能抓。
- 语料：22 例 + 3 张真图（一张 FF、一张 SP、一张 WS）。
- 出口：阈值同时「不误报」与「不漏报」；结果与差异图进 `docs/evidence/2026-xx-xx-svg-pdf-raster/`。

**P5.3 兼容性实测（G8）** — ✓ 已实施（2026-09-09），记录在 09-09 计划 §6.4，证据在 `docs/evidence/2026-09-09-svg-compat/`

- 脚本（`scripts/svg-compat.ps1` / `.sh`）：把 22 例语料 + 3 张真图分别喂给 resvg（已有）、`rsvg-convert`（librsvg）、
  `inkscape --export-type=png`、Chromium headless（若机器上有）；输出 PNG 并列进 evidence，人工看重点样本：
  负 scale + clip + 文字、细虚线、multiply + wipeout。
- 不承诺 Illustrator（按 v2 D1 的口径「按版本实测，不承诺」）。
- 出口：一份 evidence 表，列出每个渲染器对每个重点样本的结论；发现的偏差各开一条后续。

### P6 R3 第二轮：先量再改（G7）

**P6.1 release 基线** — ✓ 已做，数据与裁决在 §7（P6.2–P6.4 据此**不做**）

- 11 张真图 release 构建 `--plot-svg --model --paper A1 --landscape --fit`：每张耗时（进程内分段：开图 / 建场景 / 出图）、
  峰值工作集、输出字节；记进本文件 §7。
- 出口：有数字才进 P6.2–P6.4；任何一项没有量出瓶颈就不做。

**P6.2 流式写出**

- 观察：`emit_plot_content` 只在**前奏**里发 `Clip`（`plot_emit.rs:429-451`，紧跟 `Concat`），第一条绘制 op 之后不会再有。
  所以 `<defs>` 在第一条绘制 op 到达时**已经完整**——不需要两趟：sink 缓冲到第一条 `FillRect/Stroke/Fill/FillMesh`，
  把 head + defs 一次写出，之后直接写 `out`。
- 护栏：绘制开始后再来 `Clip` → 显式错误（`SvgError::Invalid("clip after drawing")`），不静默回退。
- 出口：第二层 22 例不变；峰值内存对比 P6.1 有下降；`svg_page_to_string` 行为不变。

**P6.3 字形边界缓存**

- 现状：`emit_text` 对每个 quad 把 `ge.fill_tris`（字形局部坐标）经仿射映射后发 `FillMesh`，`SvgSink` 再对世界坐标三角
  做边缘抵消。同一字形出现 N 次就算 N 次。
- 改法：边界在**字形局部空间**算一次（按 `uv_key` 缓存在 `GlyphState`，一页一份），得到的是「顶点索引环」；
  `FillMesh` 增加 `outline: Option<Vec<Vec<PlotPoint>>>`，由同一个 `map` 映射；`PdfSink` 忽略它（Op 流不变），
  `SvgSink` 有就直接用、没有再现算。非流形字形缓存为 `None`，走现有兜底。
- 出口：PDF 第一层不变；SVG 输出**逐字节相同**（映射是同一函数、同一顺序）；P6.1 里的出图段耗时下降可量。

**P6.4 受限路径合并**（只在 P6.1 证明体量仍是问题时做）

- 边界照 v2 R3：不越过绘制顺序、不跨 wipeout / clip / blend 变化、不倒转线段、不拼接独立子路径、multiply 下有重叠不合并。
- 出口：第二层比较仍绿；体量 / 解析耗时都记录，不只看文件变小。

### P7 命令行剩余（G6） — ✓ 已实施，记录在 §12

- `--orientation portrait|landscape`（`--landscape` 保留为别名）。
- `--paper` 扩到 ANSI A–E 与 `WxH`（mm）自定义；纸张表放 `io/paper_sizes.rs` 一份，GUI 与 CLI 共用。
- `--units mm|in|...`：图纸 `$INSUNITS` 缺失或为 0 时**必须**给，否则报错；有单位的图纸给了则校验一致。
- `--margins`、`--preset`：`--preset preview-a3-fit` 之类命名预设只当便利默认，`--dry-run` 打印展开后的实际参数。
- 出口：每个拒绝路径有用例且**不落盘**；`--dry-run` 输出包含纸张 / 比例 / 范围 / 裁剪 / CTB / 字体告警。

## 4. 推荐顺序与规模

| 序 | 期 | 规模（预估） | 为什么排这里 |
|---|---|---|---|
| 1 | P4.1 web | 小（半天） | P2 唯一还红的出口条件；今天是**哑的**，用户感知最差 |
| 2 | P5.1 R4 | 小（半天） | 已知正确性缺陷，两个后端共享；改动集中在两行 + 一条反例 + 冻结文件同步 |
| 3 | P4.2 对话框 | 中（1–2 天） | 补完 D5；让 stamp 在源头置灰而不是导出时报错 |
| 4 | P6.1 基线 | 小（半天） | 后面三项要不要做全看它 |
| 5 | P6.2 / P6.3 | 中（各 1 天） | 有数字支撑再做；两项互不依赖 |
| 6 | P5.2 / P5.3 | 中（各 1–2 天，含工具安装） | 完成三层验收与兼容性；依赖外部工具，可与 P6 并行 |
| 7 | P4.3 GUI 多页 | 中（1 天） | 对称性问题，不阻塞任何人；做之前要先有 G5 的确认对话框 |
| 8 | P7 CLI | 中（1–2 天） | 便利面，最后做 |

## 5. 风险

- **R4 修复改 PDF 字节**（P5.1）：只在 CTB 带 cap/join 覆盖且跨组的图上；修前先用旧实现证明它错，再动冻结文件。
- **web 下载的字形快照**（P4.1）：wasm 单线程，`resolve_plot_job` 与下载在同一轮，不存在 native 那种「后台线程出图时编辑器继续烘焙」；
  但冷图集首图的失配重排逻辑照样走，要有用例。
- **对话框状态持久化**（P4.2）：`printer_name` 存哨兵的老配置文件要能读回；新哨兵对旧版本是「未知打印机」——旧版本回退到默认打印机，
  不崩即可。
- **流式写出的前提**（P6.2）：「`Clip` 只在前奏」是 emitter 今天的事实，不是接口承诺——所以要护栏，不能假设。
- **外部工具版本漂移**（P5.2 / P5.3）：版本写进 evidence；工具缺失时测试 `#[ignore]` 而不是假绿。

## 6. 拍板记录（2026-09-08，五条全按建议）

1. **web 多页**（P4.1）：首版只下载一页；「顺序触发 N 次下载」还是加 `zip` 依赖打包，G4 落地时再定。
2. **PDF 栅格引擎**（P5.2）：外部 `mutool` / `pdftoppm`，不进依赖树，缺了就 ignore。
3. **R4 修复**（P5.1）：接受它改变一小类 PDF 的字节并同步冻结文件——这是修 bug。
4. **对话框整合范围**（P4.2）：SVG 进「Printer / plotter」下拉当目的地。
5. **顺序**：按 §4 走。

## 7. 基线数据（P6.1，2026-09-08）

**怎么量**：`--plot-svg` 新增 `--timing`（`PlotSvgRequest.timing`），在进程内按三段打 stderr：
`read`（读文件 + 解析成 document；场景只被标脏）、`scene+pages`（几何真正构建、文字排版、页面解析、字形快照——
`resolve_plot_job` 这一段）、`write`（emitter + SVG 写入器 + 发布）。进程启动不在里面，用外层 wall 减 `total` 看。
峰值工作集从外面量：PowerShell 起进程后每 15 ms 读一次 `Process.PeakWorkingSet64`（OS 自己维护的峰值，轮询只是取走），
取进程退出前最后一次读数。每张跑两遍取第二遍（文件缓存热），第一遍的 wall / peak 另列供看抖动。
机器：Ryzen 9 7950X（32 线程）/ 63 GB；rustc 1.99.0-nightly (2026-08-07)；`[profile.release] strip = true`，其余默认；
命令 `--plot-svg IN OUT --model --paper A1 --landscape --fit --force --timing`。11 张全部与 R1 批次 SHA-256 逐字节相同。

| 张 | DXF KB | wall ms（1/2 遍） | read | scene+pages | write | 进程内合计 | 峰值 WS MB（1/2 遍） | SVG KB |
|---|---|---|---|---|---|---|---|---|
| FF02-05 | 400 | 1065 / 1072 | 6 | 946 | 60 | 1012 | 79 / 79 | 2065 |
| FF02-06 | 377 | 1019 / 993 | 5 | 872 | 59 | 937 | 78 / 78 | 2024 |
| FF02-07 | 412 | 1076 / 1089 | 5 | 945 | 71 | 1022 | 85 / 85 | 2449 |
| SP02-05 | 745 | 1383 / 1346 | 13 | 1191 | 75 | 1280 | 106 / 106 | 2894 |
| SP02-06 | 566 | 1420 / 1454 | 8 | 1298 | 71 | 1378 | 85 / 88 | 1857 |
| SP02-07 | 702 | 1313 / 1293 | 12 | 1166 | 46 | 1225 | 93 / 90 | 1648 |
| SP02-08 | 722 | 1292 / 1293 | 12 | 1166 | 46 | 1225 | 90 / 93 | 1658 |
| SP02-09 | 602 | 1234 / 1227 | 9 | 1103 | 42 | 1156 | 87 / 88 | 1437 |
| SP02-10 | 458 | 1319 / 1313 | 7 | 1182 | 45 | 1234 | 87 / 88 | 1545 |
| WS02-05 | 200 | 958 / 943 | 2 | 845 | 24 | 872 | 69 / 70 | 905 |
| WS02-06 | 300 | 1197 / 1232 | 3 | 1119 | 32 | 1156 | 72 / 74 | 1045 |

**读数**：

- 一张 A1 P&ID release 下 **0.95–1.45 s** 出图（debug 是 8–15 s）。两遍之差 ≤ 4 %，峰值 WS 之差 ≤ 3 MB。
- **`write`（emitter + SVG 写入器）只占 2.5–7 %**：24–75 ms，2–3 MB 的文档。写入器不是瓶颈。
- **`scene+pages` 占 88–92 %**：几何构建 + 文字排版 + 字形快照。真要快，得看这一段（与 SVG 无关，PDF 出图同样付这笔）。
- 进程启动 + 退出（wall − 进程内合计）约 50–75 ms。
- 峰值工作集 **70–106 MB** 整进程；SVG body 在内存里拼的那一份是 1–3 MB，最多占 3 %。

**对 P6.2–P6.4 的裁决**（按 P6.1 出口条件「没量出瓶颈就不做」）：

- **P6.2 流式写出**：能省的是 body 那 1–3 MB（≤ 3 % 峰值）与 `write` 里的一部分（≤ 75 ms）。**不做**，除非以后有 A0 级几十 MB 的页。
- **P6.3 字形边界缓存**：只影响 `write` 段里 `FillMesh` 的边界提取；整段 `write` 才 24–75 ms。**不做**。
- **P6.4 受限路径合并**：目标是体量而不是耗时；体量在 R3 已做过一轮（−14 % 文本、−72 % svgz），没有新的消费者说太大。**不做**。
- 如果以后要优化「出一张图要多久」，量到的答案是 `scene+pages`，那是场景层的事，另开计划。

数据文件：`%TEMP%\ocs-pid-svg-p61\baseline.csv`（本机，未入库）。

## 8. P4.1 实施记录（2026-09-08）

**做了什么**：

| 文件 | 内容 |
|---|---|
| `src/io/svg_export.rs` | 新增跨平台的 **`svg_job_to_string(pages, plot_style, assets, options)`**：一个 job → 一份内存里的文档。只是 `as_plot_page` + `svg_page_to_string` 的包装，不是第二个写入器；**只接一页**——多页拒绝并说有几页（下载没有编号，不悄悄截成第一页），空 job 与文件层同一句拒绝。`PlotStyleTable` / `PdfPageInput` 的 import 不再按平台分叉；删掉 wasm 上恒回 `None` 的 `pick_svg_path_owned` 桩 |
| `src/app/update/file.rs` | 抽出 **`current_view_svg_job()`**（纸空间布局先把自己的页面设置装进对话框 → `resolve_plot_job(current_view)`），native 的 `on_svg_export_path_some` 与新增的 wasm **`on_svg_export_web(stem)`** 共用。web 分支：`current_view_svg_job` → `svg_job_to_string(…, SvgOptions::default())` → `sys::download_bytes("{stem}.svg")`，成功 `Exported: {stem}.svg`、失败 `Export failed: …`，与 native 同一套文案；严格缺字与图章拒绝都是同一段代码给的。整段同步——wasm 单线程，下载要落在触发它的那次用户手势里 |
| `src/app/update/mod.rs` · `src/app/mod.rs` | `Message::SvgExport` 的 wasm 分支不再「选路径」，直接 `on_svg_export_web(stem)`；`Message::SvgExportPath` 与它的两条 arm 只在 native 编译（web 上没有路径可回） |
| `src/io/svg_export/tests.rs` | 新增 2 条：`the_web_download_is_the_file_the_desktop_would_have_written`（三页——纯线、页级 CTB、共享快照的文字——分别走 `export_svg_pages` 落盘与 `svg_job_to_string`，**字节逐位相同**、`SvgReport` 相同）；`the_web_download_is_one_page_and_refuses_a_longer_job`（两页 → `Invalid`，错误里有「this job has 2」；空 job → 「no pages」） |

**出口条件对照**：

- `cargo check --lib --target wasm32-unknown-unknown` ✓；`cargo check --target wasm32-unknown-unknown`（含 bin）✓；native `cargo check` ✓。
  wasm 上 lib 的 5 条 dead-code 告警（`automation_action_names` / `action_names` / `block_name_from_file` /
  `PlotRequest::layouts` / `set_headless_model_page`）是**此前就有**的，全是只有 native 自动化才用的东西。
- `cargo test --lib -- io::svg_export io::pdf_export app::automation` **63 过 / 0 败 / 2 忽略**（原 61 + 新 2）。
- clippy（native 与 wasm，`--lib`）对改动的行 0 告警；`rustfmt --check` 对 `svg_export.rs` / `tests.rs` 0 diff，
  `file.rs` / `update/mod.rs` / `app/mod.rs` 的 fmt diff 全是既有的，不在改动的行里（沿用 P2「只格式化新代码」的口径）。
- **真机 `trunk serve` 点一次拿到下载：未做**（本机没装 trunk；也需要人眼确认浏览器真的弹出下载）。这一条留给下一次
  有 web 构建环境时补，并记进 evidence。
- 冷图集首图的失配重排（§5 第二条风险）：走的是同一个 `resolve_plot_job`，`app::automation` 里已有的用例
  （`stale_glyphs` 归零 / 第二次不再重排）对 web 路径同样成立，没有另写。

**这一期没做的**：web 多页（等 G4，§6 Q1）；`.svgz` 下载（web 没有「按扩展名决定压缩」的入口，首版只出 `.svg`）；
`on_plot_export_path_some`（PDF）里同一段「纸空间先装页面设置」还是自己的一份，没有并进 `current_view_svg_job`——
那是 PDF 路径的改动，留给碰 PDF 时顺手做。

## 9. P5.1 实施记录（2026-09-08）

**修法**：与 §3 写的「每组开头置 `None`」不同——那样每一页第一组的首根 wire 都会多发一对 Round，17 例全变。
实际取的是**把 `last_cap` / `last_join` 提到组循环外**，用前奏刚发出的 Round 起始：组间没有 `Save/Restore`，图形状态
本来就是连续的，追踪器跟着连续即可。这样只有「第一组以非 Round 结尾、第二组要 Round」那一类多出两个 op，其余逐位不变。
`legacy_reference.rs` 同一处同样改法（文件头写明这是冻结后唯一一次有意改动、同一提交）。

| 文件 | 内容 |
|---|---|
| `src/io/plot_corpus.rs` | 反例 **`two groups, ctb cap across the split`**：`g1-butt`（ACI 1 → butt/miter）单独一组；第二组 `g2-round`（普通白线，默认 Round）+ `g2-butt`（ACI 1 再切回）。语料 17 → 18 例 |
| `src/io/plot_emit.rs` | 追踪器提到组循环外（+注释说明为什么组边界不是重置） |
| `src/io/pdf_export/legacy_reference.rs` | 同步同一改动 |
| `src/io/pdf_export.rs` | `the_second_group_says_round_again_after_a_ctb_butt`：走 `new_ops` 的 Op 流，按 `Save/Restore` 栈追踪 cap/join，三条 `DrawLine` 依次必须是 (Butt,Miter) / (Round,Round) / (Butt,Miter) |
| `src/io/svg_export/tests.rs` | `a_ctb_cap_left_by_the_first_group_does_not_leak_into_the_second`：**问写出的 SVG**（usvg 解析回来的 `stroke-linecap/linejoin`），不问 Op 流——第二层对比两边一起错时看不见这个 bug |

**出口条件对照**：

- 反例修前红、修后绿：两条新用例修前都失败，实际都是 `[(Butt, Miter) ×3]`；修 emitter 不修冻结文件时，
  `emitter_through_pdf_sink_matches_the_frozen_exporter_op_for_op` 只在反例上红（`legacy 22 ops, new 24 ops`，
  第 11 个 op 处多出 `SetLineCapStyle{Round}` / `SetLineJoinStyle{Round}`），其余 17 例照旧绿；冻结文件同步后全绿。
- 其余用例逐位不变：把 18 例的 `exact(normalized(new_ops))` 在修前 / 修后各 dump 一遍（单独跑、不并行，避开共享字形图集
  的抖动），17 个旧例文件哈希相同，只有反例多出那两行。第一次并行 dump 时 `ctb` 两例看似不同，是文字走共享图集被别的
  测试重烘导致的，与 R4 无关——单独跑即相同。
- `saved_pdf_bytes_match_the_frozen_exporter_apart_from_the_trailer_id` 仍绿。
- `cargo test --lib -- io::svg_export io::pdf_export app::automation` **65 过 / 0 败 / 2 忽略**（§8 的 63 + 新 2）。
- rustfmt 对五个改动文件 0 diff；clippy 对改动的行 0 告警（`plot_emit.rs` 的 4 条是既有的、不在改动处）。
- 真图：11 张 P&ID 用修后的 debug 二进制重跑 `--plot-svg … --model --paper A1 --landscape --fit`，**11/11 与 R1 批次
  （= R3 批次）SHA-256 逐字节相同**（8–15 s / 张）。意料之中：`--model` 出图第一组为空，追踪器起始就是前奏的 Round；
  会多两个 op 的只有「纸空间组以 CTB butt/miter 结尾、模型空间组以 Round 开头」的布局出图。

## 10. P4.2 实施记录（2026-09-08）

**形状**：`to_file: bool` 保留（它在每份已存配置与每个页面设置里），旁边加 `file_format: PlotFileFormat { Pdf, Svg }`
（serde 默认 `Pdf`，老配置读回来还是 PDF；老版本读新配置忽略这个字段，落在「文件」上）。三者合成一个答案
`PlotDialogState::destination() -> PlotDestination { Printer, Pdf, Svg }`，视图与提交都只问它。

| 文件 | 内容 |
|---|---|
| `src/ui/window/plot.rs` | `OUT_SVG = "Save to SVG file…"` 进 Printer / plotter 下拉，与 `OUT_PDF` 同级；`set_destination_name` / `destination_name`（页面设置存取哨兵的唯一一份逻辑）；`effective_stamp()`：SVG 下为 false；动作按钮 `Export SVG` / `Export PDF` / `Print`；「Plot stamp」在 SVG 下用 `check_enabled(…, false)` 置灰、显示实际生效值（关），下面一行说明「SVG has no plot stamp: it is device text in a font the file would not carry.」 |
| `src/app/update/file.rs` | `M::Printer` 走 `set_destination_name`；两处写 `ps.printer_name` 改 `destination_name()`；`load_plotsettings_into_dialog` 先认 `OUT_SVG`、再沿用「名字含 pdf 就是 PDF 文件」的老规矩；`<none>` 页面设置把格式复位成 PDF；`pdf_plot_options` 用 `effective_stamp()`（预览 / 打印 / PDF / SVG 都从这里拿选项）；`on_plot_dlg_commit` 非预览、目的地为 SVG 时直接 `Task::done(Message::SvgExport)`——与 `EXPORTSVG` / `SVGOUT` 同一条路（D5） |
| `src/io/svg_export.rs` | 保存对话框过滤器加「Compressed SVG Files (*.svgz)」（G9） |
| `src/app/update/file.rs`（tests） | `plot_destination_tests`：三种目的地 + 一个具名打印机各走一遍 `M::Printer`，`destination()` 对，`dialog_to_plotsettings().printer_name` 存的是哨兵 / 打印机名，切走再 `load_plotsettings_into_dialog` 读回不丢；「Microsoft Print to PDF」仍读成 PDF；stamp 在 PDF 下开、切到 SVG 时 `effective_stamp` 与 `pdf_plot_options().stamp` 都为 false 而偏好保留、切回 PDF 又回来；`{"to_file": true}` 这种老配置反序列化成 PDF |

**出口条件对照**：`cargo test --lib -- plot_destination_tests io::svg_export io::pdf_export app::automation`
**68 过 / 0 败 / 2 忽略**（§9 的 65 + 新 3）；`cargo check` native 与 wasm 都过（wasm 仍是那 5 条既有 dead-code）；
clippy 对改动的行 0 告警；rustfmt：`svg_export.rs` 0 diff，`plot.rs` 只多一处——`check_enabled(...)` 那一行落在一个
**本来就没格式化过**的 `column![]` 块里（HEAD 上同一位置的 `check(...)` 也在 rustfmt 的 diff 里），照 P2「只格式化新代码」
的口径没有动整块；`file.rs` 新增的测试模块 0 diff。

**没做 / 顺手发现的**：

- **GUI 没有人手点过一遍**（无窗口驱动的是状态与消息，不是像素）。要看：打开 PLOT → Printer/plotter 选「Save to SVG file…」
  → 按钮变 Export SVG、Plot stamp 灰掉并出现说明 → 点 Export SVG 弹 SVG 保存框（过滤器里有 .svgz）。
- **模型空间下的出图区域**（新缺口 **G10**）：SVG 目的地走的是 `Message::SvgExport` → `direct_plot_params()`，在模型空间
  **永远按 Extents**，不看对话框的 Window / Display / Limits / View；PDF 目的地在同一对话框里走 `PlotWindowExport`
  是按区域裁的。要补得让 `PlotRequest` 带区域（或 `direct_plot_params` 认区域——但它也被菜单 Export PDF 与打印共用，
  改它会改 PDF 的行为），单独一期做，别顺手。
- 三条新文案（`Save to SVG file…`、`Export SVG`、stamp 说明）**没进 21 份 Fluent 目录**，与 P2 的 `Export as SVG` 一样
  回落英文；下次翻译批次一起补。**→ P8.5 已建 key**（2026-09-10，09-09 计划 §6.5）：21 份目录 + `locale_catalog.rs`
  都有了这几条（连同 web 多页提示与 PRINTALL 的 `SVG` 标签），非英文目录先放英文占位，等翻译批次。
- `PlotFlag::Stamp` 的消息在 SVG 下仍会翻转 `stamp` 偏好（复选框已禁用，正常点不出来），没加拦截。
  **→ P8.5 已拦**（2026-09-10，09-09 计划 §6.5）。

## 11. P4.3 实施记录（2026-09-08）

**形状**：PRINTALL 对话框多一个 **SVG** 按钮（与 PDF / Print 同排）→ `Message::PrintAllSvg` → 同一个 `pick_svg_path_owned`
选一个基名 → `PrintAllSvgPath` → `on_print_all_svg_path_some(base)`：勾选的布局 → `PlotRequest { layouts, use_current_settings:
print_all_settings_override }` → `resolve_plot_job` → **先** `page_paths(base, n)` 查已存在的文件：一个都没有就直接写
（`force: false`，中间冒出来的文件让写入器拒绝而不是覆盖）；有就把整套（已排好版的 `PlotJob`）存进 `pending_svg_export`，
弹 `ModalKind::SvgOverwrite`——「N of M files this plot writes already exist」+ 逐个文件名 + **Replace All / Cancel**。
Replace 才 `force: true`；Cancel 回到 PRINTALL（勾选不丢）；点 ✕ 关掉等于 Cancel（`close_active_modal` 把 pending 丢掉）。
三条 SVG 入口（单页对话框 / 命令、PRINTALL、覆盖确认）最后都落在同一个 `write_svg_job(job, base, force)`，
报告文案一致（`Exported: …` / `Export failed: …`，`Partial` 原样进命令行）。web 上按钮存在但报「Multi-page SVG export is
not available in the web version; EXPORTSVG downloads the current layout.」（§6 Q1 仍开）。

| 文件 | 内容 |
|---|---|
| `src/app/mod.rs` | `Message::{PrintAllSvg, PrintAllSvgPath, SvgOverwriteReplace, SvgOverwriteCancel}`、`ModalKind::SvgOverwrite`、`pending_svg_export: Option<PendingSvgExport { base, taken, job, background }>`（后三者 native only） |
| `src/app/update/mod.rs` | 四条消息的 arm；`close_active_modal` 对 `SvgOverwrite` 丢 pending |
| `src/app/update/file.rs` | `write_svg_job`（三个入口共用）、`on_print_all_svg_path_some`、`on_svg_overwrite_replace`；`on_svg_export_path_some` 改用 `write_svg_job(job, &path, true)` |
| `src/app/view/modal.rs` | `SvgOverwrite` 标题与 `svg_overwrite_dialog_window(taken, total)`（最多显示 8 行、多了滚动） |
| `src/ui/window/print_all.rs` | SVG 按钮 |
| `src/app/update/file.rs`（tests） | `print_all_svg_tests`：新图 + `Layout1` + `add_layout("Sheet B")`、两个都勾、`background=false` 同步跑——**两页 → `plot-001.svg` / `plot-002.svg`**；预放 `plot-002.svg` 垃圾 → 弹 `SvgOverwrite`、`taken == [plot-002.svg]`、job 两页、**页一也没写**；Cancel → 磁盘不动、回 PRINTALL；`CloseModal` → pending 丢、磁盘不动；Replace → 两页都写、旧文件成了文档 |

**出口条件对照**：`cargo test --lib -- print_all_svg_tests plot_destination_tests io::svg_export io::pdf_export app::automation`
**70 过 / 0 败 / 2 忽略**（§10 的 68 + 新 2）；`cargo check` native 与 wasm 都过；clippy 对改动的行 0 告警；rustfmt 对新代码 0 diff。
GUI 同样**没人手点过**。

**踩到的坑（重要）**：`on_print_all_svg_path_some` 与 PDF 一样调 `save_config()`，而测试里的 `OpenCADStudio::new_for_test()`
用的是**真实的** `%APPDATA%\OpenCADStudio\settings.json`——第一次跑测试把用户的 `plot.background` 写成了 false
（20:48:53，已手工改回 true，其余字段是从同一份配置读回再写出的，应当没变）。测试现在先
`app.last_saved_config = Some(app.current_config())` 让 `save_config` 无事可做。**任何会走到 `save_config` 的测试都有这个坑**，
`new_for_test` 该给一个不落盘的配置路径——单独一条债，没在这一期动。
**→ P8.5 已翻掉**（2026-09-10，09-09 计划 §6.5）：`cargo test` 下 `config::config_dir()` 指向进程私有的临时目录，
`settings.json` / 别名 / 上次目录都落在那里；两处「先喂 `last_saved_config`」的绕法已删。

## 12. P7 实施记录（2026-09-08）

**形状**：参数解析与裁决全在 `plot_svg_with` 里（`PlotSvgRequest` 多 `orientation` / `units` / `margins` / `preset` 四个字符串位），
`main.rs` 照旧只是抄写员。返回值从 `SvgBatch` 变成 `PlotSvgRun { batch, plan }`：`plan` 是这次出图用词说清的样子——
preset 展开成了什么、单位从哪来、逐页的纸张 / 比例 / 窗口 / 裁剪 / CTB / 缺字——`--dry-run` 把它打出来（排练的答案是计划，
不只是文件名）。

- **`--paper`**（表在 `io/paper_sizes.rs` 一份）：`PaperSize` 长出 ANSI A–E（英寸尺寸精确换算），GUI 的下拉
  （`PaperSize::ALL`）与页面设置的标签推断（`paper_label_from_dims`）同步长出来；`from_label` 认大小写与分隔
  （`ansi-b` = `ANSI B`）；`parse_paper` 另收 `WxH`（mm，正有限数），`plot_dialog_sheet_mm` 与 `file.rs:4740` 的
  硬编码 A 系列匹配都改走 `from_label`。
- **`--orientation portrait|landscape`**：`--landscape` 保留，两者 clap `conflicts_with`；都不说时**自定义 `WxH` 跟纸形走**
  （600x300 自然横放），标准纸照旧竖放。
- **`--units mm|cm|m|km|in|ft|yd|mi`**：与图纸 `$INSUNITS`（`insunits_to_mm`，现 `pub(crate)`）之间的裁决在 `plot_unit_mm`：
  两边都有须一致（连 `--fit` 下撒谎也拒，谎话不该因为没人读它就过关）；图纸有单位它自己作答；都没有时**只在给了
  `--scale` 才是错**——fit 不读单位，这里对 v2 D4 表「无单位图纸要求 --units」收窄到真正需要它的那条路。比例按物理毫米折算
  （`set_headless_model_page` 把 `ratio × unit_mm` 写进对话框比例）：**米图 1:1000 与毫米图 1:1 逐字节同一张纸**（有用例钉住）。
  这对声明了 $INSUNITS 的图纸是行为变更（此前一律当 mm），fit 出图（含 11 张真图批次）不受影响。
- **`--margins M|H,V|L,B,R,T`**（mm，未旋转纸面上的左/下/右/上）：`window_to_sheet` 收进 `window_to_sheet_margins`——
  给了边距 fit **精确**贴边框（边距就是呼吸空间，5% 松弛不再叠加）、居中在框内；不给时旧公式原样走一遍，**字节不变**。
  边距吃光纸面在 `set_headless_model_page` 里点名拒绝；负数 / 三个值 / 读不出都在解析处拒绝。
- **`--preset preview-a4-fit / preview-a3-fit / preview-a1-fit`**：= `--model --paper X --orientation landscape --fit`，
  **只填没说的**（显式旗子赢）；配 `--layout` 是矛盾，拒绝；`--dry-run` 的 plan 第一行就是展开记录。
- **严格化**：layout 出图（点名或全部）带任何 model 专属旗子（`--paper` / `--orientation` / `--landscape` / `--fit` /
  `--scale` / `--units` / `--margins`）在**打开图纸之前**拒绝——会被静默忽略的旗子是陷阱不是便利（D4）。
  `--landscape` 从「layout 下静默忽略」改为同样拒绝，是这一期唯一收紧的旧旗子。

| 文件 | 内容 |
|---|---|
| `src/io/paper_sizes.rs` | ANSI A–E、`from_label`、`PaperSpec` / `parse_paper`、`window_to_sheet_margins`；测试 +4（ANSI 尺寸、拼法、解析拒绝、边距数学） |
| `src/ui/window/plot.rs` | `PlotDialogState.margins_mm: Option<[f64;4]>`（`serde(skip)`，GUI 永远 `None`，只有无头路径设置） |
| `src/app/update/file.rs` | `set_headless_model_page(paper, landscape, fit, scale, margins_mm, unit_mm)`；`plot_dialog_sheet_mm` / 页面设置标签走 `from_label`；`area_plot_job` 走 `window_to_sheet_margins` |
| `src/app/properties.rs` | `insunits_to_mm` / `insunits_name` 改 `pub(crate)`（其余原样） |
| `src/app/automation.rs` | 四个新字段、`PLOT_PRESETS` / `PLOT_UNITS` 表、`apply_plot_preset` / `stray_model_flag` / `plot_orientation_landscape` / `plot_unit_mm` / `parse_plot_margins` / `plan_num`、`PlotSvgRun` 与 plan 组装、`plot_svg_headless` 在 dry-run 下打印 plan；测试 +6 |
| `src/cli.rs` · `src/main.rs` | 四个新旗子（`--orientation` 与 `--landscape` clap 互斥）、request 抄写 |

**出口条件对照**（P7 的两条全中）：每个拒绝路径有用例且**不落盘**——未知纸张 / 读不出的 WxH、不是 portrait 或 landscape、
未知单位、单位与图纸矛盾（fit 下也拒）、无单位又给 scale、边距吃光纸面 / 负数 / 三个值、未知 preset、preset 配 layout、
七个 model 旗子落在 layout 出图上（图纸路径都是假的，证明拒绝先于打开）；`--dry-run` 输出含纸张 / 比例 / 范围 / 裁剪 /
CTB / 缺字告警（plan 逐页一行 + 已有的 per-page 告警行）。
`cargo test --lib -- io::paper_sizes io::svg_export io::pdf_export app::automation plot_destination_tests print_all_svg_tests`
**83 过 / 0 败 / 2 忽略**（§11 的 70 + paper_sizes 旧 3 + 新 10）；native / wasm `cargo check` 过（wasm dead-code 仍是已知那 5 条）；
clippy 对改动行 0 告警；rustfmt：`paper_sizes.rs` / `cli.rs` / `main.rs` 整文件 0 diff，其余文件新增 0 处（对 HEAD 基线逐处核对）。
**未做真机跑批**：11 张真图没用新旗子重跑（fit 路径字节不变有单元用例背书，release 批次留给下次要动比例时一起）。

**并发插曲**：同晚另一个会话在同一棵工作树上起了一版平行的 P7 开头（`PlotScale::FitMargins(f64)` 变体、
`NamedSheet` / `ANSI_SHEETS` / `SheetSpec` / `parse_sheet_spec` 一套，全部无人引用），用户裁决停掉那个会话由本会话收尾，
这些桩已删（等价能力都在：FitMargins ⊂ `--margins` 四边版，NamedSheet 表 ⊂ `PaperSize`）。同一棵树上多会话并行写码
没有锁就是互相覆盖，这次靠 mtime 与 `git status` 及时看见——协同要么编组拿写锁，要么分树。

## 13. P5.2 实施记录（2026-09-09）

**处境**：P5.2 的四个文件在主工作树上写完、还没提交，就被一场 origin/main 整合 merge 盖掉（merge 到本记录
落笔时仍未提交完）；靠 `%TEMP%\ocs-p52-fable-5-15-snapshot\` 逐字节恢复。按用户 2026-09-09 的裁决，P5.2 从此在
**自己的 worktree** 收尾：`../OpenCADStudio-p52`，分支 `p52-svg-raster`，基 `0f716727`（merge 前的 HEAD），
不再与别的会话同树。worktree 用自己的 `CARGO_TARGET_DIR`——这个 crate 每次 `cargo test` 都会重链接测试
可执行文件，共享 target 下两个并发 cargo 必有一个撞 LNK1104（本期实测撞上，与此前「并发编译错误」同根）。

**形状**（照 §3 P5.2 与 §6 Q2 的拍板：外部栅格引擎，不进依赖树，缺了就大声跳过）：

| 文件 | 内容 |
|---|---|
| `src/io/raster_compare.rs`（新，test-only） | `PdfRasterizer::discover`（`OCS_PDF_RASTERIZER` → winget Links → PATH，认 mutool / pdftoppm，探版本进表头；pdftoppm 带 `-thinlinemode shape`）、`render_svg`（resvg 0.45.1，与第二层同一棵树）、`compare`（1 px 位移窗、2/255 值容差、32 px 瓦片 × 8 px 预算、页缘跳 2 px，另有**单向合成豁免**：SVG 侧实心墨（≤96/255）内，PDF 只考「两像素内有没有全覆盖的墨」、不考它的值——PDF 门把字形写成三角网格，poppler 逐三角抗锯齿，网格内部欠涂 6–23%、共享边裂到近白，深度分不开真伪、结构分得开；SVG 侧永不豁免，PDF 比 SVG 更深也不豁免）、`diff_map` |
| `src/io/svg_export/tests.rs` | 例行门 `the_pdf_and_the_svg_rasterise_to_the_same_picture`：300 dpi 全语料（空间上比计划的 600 dpi/1 px 更严），跳过 stamp 页、文字页（并行跑时共享字形图集会翻动，留给单线程证据跑）、发丝线页与 merge_lines 页（都有注释说明与证据）；注入门 `the_raster_comparison_catches_the_planted_faults`：整线缺失 / 2% 错比例 / dash 错相位 / wipeout 次序换组，四对都先证明干净配对是绿的；证据倾倒 `dump_raster_evidence`（600 dpi 全语料含文字页，单线程） |
| `src/io/mod.rs` | `#[cfg(all(test, not(target_arch = "wasm32")))] pub mod raster_compare;`（+2 行） |
| `src/app/automation.rs` | `dump_real_sheet_raster_evidence`：FF02-06 / SP02-05 / WS02-05 三张真图（FF 取 -06 不取 -05：-05 常开在编辑器里，它的字节区锁连第二个进程的读都拒）走整条无头出图链路（开图 → CTB → Model → A1 横放 fit → `resolve_plot_job`），同一个 job 出两个门，600 dpi。先试过 300（真图线宽 ≥0.13 mm ≈ 1.5 px，在亚像素地板之上）：线条全过，但**每个标签都在冒斑**——整张图是 2.5–3.5 mm 的密排文字，300 dpi 下字形笔画只有 2–3 px，标签里几乎没有一个像素算得上实心墨，合成豁免无处立足；600 dpi 笔画内部才是真的实心（A1 600 dpi 单引擎 GB 级位图，手动证据跑负担得起） |
| `docs/evidence/2026-09-09-svg-pdf-raster/` | `corpus-600dpi.tsv`、逐对 `diff-*.png`、merge_lines 最小对照一对、README；`real-sheets-600dpi.tsv` **待跑**（见出口对照第四条） |

**merge_lines 的裁决（本期唯一的实质发现，含一次修正）**：例行重跑抓到 `hatches, merge_lines` 整块红色
multiply 填充在 resvg 侧不见（poppler 画 (178,25,25)，resvg 出白），300 dpi 下 2209 缺陷像素。上一会话留下的
判断是「隔离层包围盒扫到远端几何就丢」——**最小复现证明这不完整**：isolate + 远端路径 + 页上 multiply 填充
（无 clip）渲染**正确**。真正的触发条件是**三件齐备**：`isolation:isolate` 图层 ∧ 图层内容包围盒扫到
~1.3e7 单位外的远端几何（这页是远离原点用例：出图窗口在世界 (500000, 4500000) mm，页上只有一块 multiply
填充，其余全部在 500 km 外、被页裁剪切掉）∧ 祖先 `<g clip-path>`。三缺一都正确：真实页去掉 isolation 渲染
正确、去掉 clip-path 渲染正确；最小对照一对只差 clip 包裹层，留在 evidence 目录。PDF 同一内容 poppler 全部
变体都画对 ⇒ **渲染器局限，不是导出缺陷**（第二层 usvg 解析回来的 multiply 结构也对着，
`merge_lines_multiplies_on_the_leaves_inside_an_isolated_page`）。例行门照发丝线的先例跳过并注明；600 dpi
证据表记 FAIL + 说明 + diff 图，不假绿。

**出口条件对照**（§3 P5.2 的四条）：

- 栅格引擎不进依赖树：✓ discover 顺序 env → winget → PATH，都没有就打印 SKIPPED 并检查零条（大声跳过，
  不是假绿）；版本号进两张 tsv 表头（本机 pdftoppm 25.07.0；mutool 不在）。
- 口径：✓ 白底、1 px 位移、2/255 值容差、32 px 瓦片 8 px 预算、页缘 2 px；证据 600 dpi，例行门 300 dpi
  （同 1 px 窗在半密度下空间上更严，跑进例行时长）。噪声地板没有第二个 PDF 引擎可对照（机器上只有
  pdftoppm），用干净语料自身的最差瓦片衡量：例行门全绿时最差干净瓦片见跑批输出，预算 8 px 在其上留有余量。
- 判据「不误报也不漏报」：✓ 600 dpi 全语料 20 行 ok，仅有的 2 行 FAIL 各有注明（亚像素发丝线、merge_lines
  渲染器局限）；四个注入故障全部被抓，且各自的干净配对先证明是绿的。
- 证据落盘：✓ 两张 tsv 齐。语料 22 行（20 ok / 2 FAIL 有解释有对照）；**真图三张全部 FAIL**（补跑于
  2026-09-09 18:32–19:15，600 dpi）：FF02-06 **527** 缺陷 px / 3 瓦片超额（worst 30）、SP02-05 **2340** / 27
  （worst 64）、WS02-05 **3785** / 74（worst 120）——对 2.8 亿像素的 A1 这是 0.0002–0.0014 %，但判据本来就是
  局部的。worst-tile 剪裁存在本目录（`crop-real-*.png`），首看三张不是一种病：WS 的缺陷全在**大号标题字形
  笔画内部**（网格接缝纹样——合成豁免在大字形上没接住，字色偏浅或 2 px 可达性不成立，待查）；SP 的缺陷贴着
  红色管线与箭头相交的那几段（**像绘制次序或箭头填充的真差异**）；FF 只有 3 瓦片、在文字与线的交界。
  **定性已完成（2026-09-09 晚，对生像素逐点核对）：三张都是判据的局限，不是导出缺陷**——
  SP/FF 是 0.1 pt 发丝线撞上别的墨（poppler 把发丝线钉成整像素行、resvg 摊真实覆盖，光纸上位移窗解释得掉，
  贴着红管线/表格线就解释不掉）；WS 是合成豁免不认识彩色墨（标题蓝 (0,38,128)，`seam_ink`=96 按通道判，
  蓝的本通道 128 永远不合格，poppler 网格欠涂在蓝通道被记账）。细节与量测数字在 evidence README 的
  「real-sheet triage」节。后续二选一（或都做）：豁免按「最暗通道」认墨、真图证据升 1200 dpi——单独一期，
  证明注入故障仍被抓才算数；在那之前这三行就挂着 FAIL 与注释，不调预算凑绿。

**验证数字**（2026-09-09 收尾复核）：`cargo test --lib -- io::svg_export io::pdf_export app::automation`
**73 过 / 0 败 / 4 忽略**（369 s；例行栅格门与注入门都是真跑，pdftoppm 25.07.0，四个注入故障全抓）；
600 dpi 证据表 22 行如上；真图三张 600 dpi **全部 FAIL**（527 / 2340 / 3785 缺陷 px，
见出口对照第四条与 `real-sheets-600dpi.tsv`，定性升格为独立一期）。rustfmt：`raster_compare.rs` / `tests.rs` 整文件 0 diff
（`rustfmt --check --edition 2021`，exit 0）；clippy `--lib --tests`（test cfg 才编译这批 test-only 代码）
对改动行 0 告警——`raster_compare.rs` 整文件零命中，`tests.rs` / `automation.rs` 新增行零命中
（两文件旧行的既有告警不动，沿用「只体检新代码」）。

**没做 / 留给后面的**：P5.3 兼容性实测（脚本 + 四个外部渲染器）与 G10 照旧在队列里；merge_lines 的两条
后续线索——resvg 新版是否已修（树里钉 0.45.1，没动）、导出侧要不要干脆裁掉整页外的墨（会同时改两个门的
字节，牵动第一层冻结对照，单独一期）——都只记不做。主树那场 origin/main 整合 merge 收尾后，本分支再并回去。
