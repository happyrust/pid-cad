# 开发计划：SVG 导出 · 第二阶段（P4–P7）

> 日期：2026-09-08
> 状态：§6 五条已于 2026-09-08 拍板（全按建议）；**P4.1 已实施**（§8），其余待做。
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

**P4.2 出图对话框里的 SVG 目的地（G3，D5 补完）**

- `OUT_SVG = "Save to SVG file…"` 进「Printer / plotter」下拉，与 `OUT_PDF` 同级；`to_file: bool` 换成
  `Destination { Printer, Pdf, Svg }`（或保留 `to_file` 再加一位，取改动最小者）；页面设置持久化的 `printer_name`
  照 `OUT_PDF` 的写法存哨兵。
- 动作按钮随目的地变：`Export SVG`；`stamp` 开关在 SVG 下**禁用并说明原因**（不是静默忽略）。
- 「打印 / 出图」主按钮走到 SVG 时复用 `Message::SvgExport` 的路径；`EXPORTSVG` / `SVGOUT` 命令保留为快捷入口。
- `pick_svg_path_owned` 的过滤器加 `svgz`（G9）。
- 出口：对话框三种目的地各走一遍（自动化测试用 `plot_dialog` 状态直接驱动，不开窗口）；页面设置保存 / 读回 `OUT_SVG` 不丢。

**P4.3 GUI 多页 SVG（G4 + G5）**

- `PRINTALL` 的布局勾选面（`print_all_layouts` / `print_all_settings_override`）对 SVG 开放：`PlotRequest::layouts(names)` →
  `export_svg_pages` 的 D3 编号。
- 覆盖：先算 `page_paths` 并查已存在的文件，**一次确认整个集合**（列出会覆盖的名字），确认后才 `force: true`；
  `Partial` 错误原样进命令行。
- 出口：两个布局 → `stem-001.svg` / `stem-002.svg`；已存在其一 → 先问；取消 → 磁盘不动。

### P5 债务与验收补齐

**P5.1 R4 修复（G2）**

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

**P5.3 兼容性实测（G8）**

- 脚本（`scripts/svg-compat.ps1` / `.sh`）：把 22 例语料 + 3 张真图分别喂给 resvg（已有）、`rsvg-convert`（librsvg）、
  `inkscape --export-type=png`、Chromium headless（若机器上有）；输出 PNG 并列进 evidence，人工看重点样本：
  负 scale + clip + 文字、细虚线、multiply + wipeout。
- 不承诺 Illustrator（按 v2 D1 的口径「按版本实测，不承诺」）。
- 出口：一份 evidence 表，列出每个渲染器对每个重点样本的结论；发现的偏差各开一条后续。

### P6 R3 第二轮：先量再改（G7）

**P6.1 release 基线**

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

### P7 命令行剩余（G6）

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

## 7. 基线数据（P6.1 填）

（空。P6.1 完成后把 release 耗时 / 峰值工作集 / 字节数表填在这里，P6.2–P6.4 的前后对比也记在这里。）

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
