# 开发计划：SVG 导出 · 第三阶段（审核 + P8 收尾与验收补齐）

> 日期：2026-09-09
> 状态：待批准
> 前置：`docs/plans/2026-09-07-dxf-to-svg-export.md`（v2，P0 / P1 / P2 / R1 / R3 第一轮已实施）、
> `docs/plans/2026-09-08-svg-export-next-steps.md`（P4.1–P4.3 / P5.1 / P6.1（裁决 P6.2–P6.4 不做）/ P7 已实施；
> **P5.2 已在 `../OpenCADStudio-p52` worktree 实施、未收尾**，记录在其 §13）。
> 本文件 = 2026-09-09 审核结论 + 剩余工作的分期。只写「还没做的」与「怎么做」。

## 0. 一句话

引擎、三层验收里的两层半、产品入口（CLI / 对话框 / PRINTALL / web 单页）都齐了且**今天实测全绿**；
P5.2（PDF ↔ SVG 栅格对比）的代码与语料证据在 p52 worktree 已经绿，**差三件收尾事**（真图证据表、记录占位符、提交）；
功能面真正还欠的只剩 P5.3 兼容性实测、G10 出图区域、web 多页，外加一把点过名的小债。
顺序：**先收尾、再合流、再验收补齐，功能与债务最后**。

## 1. 审核结论（2026-09-09 实测）

### 1.1 两棵树、一场 merge

| 树 | 状态 | 本轮实测 |
|---|---|---|
| 主树 `OpenCADStudio`（main @ `1b3c4239`） | **origin/main 整合 merge 进行中**：冲突已解、已暂存、未提交（MERGE_HEAD = `4ddd19f6`；点名冲突过的有 `src/app/update/file.rs`、`src/io/mod.rs`、`Cargo.toml` 等 5 处） | `cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes plot_destination_tests print_all_svg_tests` → **86 过 / 0 败 / 2 忽略**（5.15 s，热 target）——**merge 没打破 SVG 导出** |
| p52 worktree `../OpenCADStudio-p52`（`p52-svg-raster` @ `0f716727`，= merge 前 HEAD 少一笔） | P5.2 的 4 个改动文件 + `docs/evidence/` + `raster_compare.rs` **全部未提交**，分支零提交 | `cargo test --lib -- io::svg_export io::pdf_export app::automation` → **73 过 / 0 败 / 4 忽略**（369 s，含例行栅格门真跑；注入门四对全抓） |

### 1.2 P5.2 实际到哪了

**做完且验证过的**：

- `src/io/raster_compare.rs`（test-only）：`PdfRasterizer::discover`（env → PATH → winget Links → Poppler 包目录，
  探版本进表头；pdftoppm 带 `-thinlinemode shape`）、`render_svg`（resvg 0.45.1，与第二层同一棵树）、
  `compare`（1 px 位移窗 / 2/255 值容差 / 32 px 瓦片 × 8 缺陷预算 / 页缘跳 2 px / 单向合成豁免——PDF 门把字形写成
  三角网、poppler 逐三角抗锯齿，网格内部欠涂、共享边裂白，豁免只给 PDF 侧、以「2 px 内有实心墨」为结构判据）、
  `diff_map`；判据自证 8 条单元用例（位移不误报、缺线抓得住、平色差抓得住、色相骗过灰度骗不过这里、
  页缘豁免带、底左对齐、尺寸差一步可比）。
- 例行门 `the_pdf_and_the_svg_rasterise_to_the_same_picture`（300 dpi 全语料，跳过 stamp / 文字 / 发丝线 /
  merge_lines 页，各有注释）+ 注入门 `the_raster_comparison_catches_the_planted_faults`（缺线 / 2% 错比例 /
  dash 错相位 / wipeout 次序，四对都先证干净配对是绿的）+ 两个 `#[ignore]` 证据倾倒（600 dpi）。
- `docs/evidence/2026-09-09-svg-pdf-raster/`：`corpus-600dpi.tsv` **22 行齐**（20 ok + 2 个说明过的 FAIL：
  亚像素发丝线 0.1 pt=0.83 px；merge_lines 是 resvg 0.45.1 的渲染器局限——isolate ∧ 远端 bbox ∧ 祖先 clip-path
  三件齐备才丢、poppler 全部变体画对，**最小对照一对已在目录里**）+ 3 张 diff 图 + README。

**没收尾的（本计划 P8.1）**：

1. `real-sheets-600dpi.tsv` **缺**——README 与 §13 记录都已声称它存在，**文实不符**；
   `dump_real_sheet_raster_evidence`（FF02-06 / SP02-05 / WS02-05，600 dpi）没真跑或没跑完。
2. §13 记录里 **两个占位符没填**：「真图三张 300 dpi：REAL_SHEET_NUMBERS」「clippy 对改动行 CLIPPY_RESULT」
  （且「300 dpi」与该表实际的 600 dpi 口径互相矛盾，要一并改对）。
3. 分支零提交；主树还混进一份**过期的** `src/io/raster_compare.rs`（无合成豁免的早期稿，无人引用、
   与 p52 版哈希不同）——合流时要删，以 p52 版为准。

### 1.3 遗留缺口盘点（按两份前置计划逐条核对）

| # | 缺口 | 出处 | 状态 |
|---|---|---|---|
| P5.3 | 兼容性实测：rsvg-convert / Inkscape / Chromium 对 22 语料 + 3 真图 | next-steps §3 | 未动 |
| G10 | 模型空间 SVG 不看出图区域（Window/Display/Limits/View，恒取 Extents） | next-steps §2/§10 | 未动 |
| Q1 | web 多页（顺序 N 次下载 vs zip 打包），PRINTALL 的 SVG 在 web 上明确报不可用 | next-steps §6 | 待拍板 |
| 债1 | `new_for_test` 用真实 `%APPDATA%\OpenCADStudio\settings.json`，走到 `save_config` 的测试会写用户配置（P4.3 踩过，已污染过一次 `plot.background`） | next-steps §11 | 未修 |
| 债2 | P4.2 的三条新文案没进 21 份 Fluent 目录（回落英文） | next-steps §10 | 未补 |
| 债3 | `PlotFlag::Stamp` 在 SVG 目的地下仍会翻转偏好（复选框已禁用，点不出来，低危） | next-steps §10 | 未拦 |
| 债4 | web 真机 `trunk serve` 下载没人手验过（P4.1 出口条件挂账） | next-steps §8 | 未验 |
| 线索 | merge_lines：resvg 新版是否已修；导出侧要不要裁掉页外墨（牵动两个门的字节与第一层冻结对照） | p52 §13 | 只记不做 |
| 线索 | 出图耗时 88–92% 在 `scene+pages`（场景层，与 SVG 无关） | next-steps §7 | 另开计划 |

不做的清单沿用 v2 §5 与 next-steps §2（SVG 导入、图层 `<g>`、`<pattern>`/`<mask>`/渐变、`<text>` 图章、
透明底图、P6.2–P6.4）。

## 2. 分期

原则不变：每期一个提交、PDF 第一层护栏每期都要绿；证据只增不删。

### P8.1 P5.2 收尾（小，半天；在 p52 worktree 里做）

- 单独跑 `dump_real_sheet_raster_evidence`（600 dpi、三张真图、单线程；A1 600 dpi 是 GB 级位图 ×2 引擎，
  预计十几分钟一张，跑前确认内存余量）→ 补齐 `real-sheets-600dpi.tsv` 与非零对的 `diff-real-*.png`。
- 真图若出说明不了的 FAIL：先按 corpus 的先例找结构性解释（文字合成豁免的边界、真图特有图元），
  解释不了就是导出缺陷，**升格为独立修复期**，本期只记录不藏。
- 填掉 §13 的两个占位符（含 300/600 dpi 口径矛盾）；对改动行跑 rustfmt / clippy 并把结果写进记录。
- README 与记录的「已存在」声称与磁盘对齐；一笔提交到 `p52-svg-raster`。
- 出口：evidence 目录与记录逐句对得上；`cargo test --lib -- io::svg_export io::pdf_export app::automation`
  复跑 73/0/4；分支上有这笔提交。

### P8.2 合流（小，半天；依赖主树 merge 先提交）

- 主树 origin/main 整合 merge 由它的会话收尾提交（不在本计划范围，但**是本期的前置**）。
- `p52-svg-raster` 并回 main：预计冲突集中在 `svg_export/tests.rs` 尾部、`automation.rs` 尾部、
  `io/mod.rs` 两行、plan 文档状态行——都是追加型，低风险。
- 删主树那份过期 stray `src/io/raster_compare.rs`（以 p52 版为准）；顺手确认主树 `docs/plans/` 里
  09-08 计划的状态行也更新成「P5.2 已实施」。
- 合流后在 main 重跑：上面那套 86 条的过滤 + p52 的 73 条过滤（此时应合成一套），例行栅格门必须还绿。
- 出口：main 上一套绿；`git worktree list` 里 p52 可以退役（留不留由用户）。

### P8.3 P5.3 兼容性实测（中，1–2 天；可与 P8.4 并行）

- `scripts/svg-compat.ps1`（Windows 本机）：22 语料 + 3 真图分别喂给机器上有的渲染器——resvg（已有）、
  `rsvg-convert`、`inkscape --export-type=png`、Chromium/Edge headless `--screenshot`；缺哪个 winget 装哪个，
  **都不进依赖树**；版本全部记进 evidence 表头。
- 重点样本人工看：负 scale + clip + 文字、细虚线、multiply + wipeout；**merge_lines 的最小对照一对直接
  喂给每个渲染器**——回答「isolate ∧ 远端 bbox ∧ clip-path 丢填充」到底是 resvg 一家的事还是生态共性，
  这决定「导出侧裁页外墨」要不要升格（见 §4 待拍板 3）。
- 出口：`docs/evidence/2026-xx-xx-svg-compat/` 一张表（渲染器 × 样本 × 结论）+ 重点样本 PNG 并列；
  发现的偏差各开一条后续，不当场修。

### P8.4 G10 模型空间出图区域（中，1 天）

- 修法照 next-steps §2 G10 的第一条路：`PlotRequest` 带区域（对话框的 Window / Display / Limits / View
  在提交 SVG 目的地时装进请求），`resolve_plot_job` 认它；**不动 `direct_plot_params`**——它被菜单
  Export PDF 与打印共用，动它就是改 PDF 行为。
- `EXPORTSVG` / `SVGOUT` 快捷命令维持现状（当前视图 / Extents），只有对话框路径带区域——与 PDF 目的地
  在同一对话框里的行为对齐。
- 出口：对话框选 Window + SVG 目的地 → 出的是窗口不是 Extents（用例驱动 `plot_dialog` 状态，不开窗口）；
  PDF 路径的 op 流与字节**不变**（第一层护栏）；`plot_destination_tests` 全绿。

### P8.5 小债打包（小，半天）

- 债1：`new_for_test` 给一个不落盘的配置路径（临时目录或内存），把 §11 那句「任何会走到 `save_config`
  的测试都有这个坑」翻掉；顺手删掉 P4.3 测试里「先喂 `last_saved_config`」的绕法。
- 债3：`PlotFlag::Stamp` 在 SVG 目的地下不再翻转偏好（拦在 update 处，一行 + 一条用例）。
- 债2：三条文案（`Save to SVG file…` / `Export SVG` / stamp 说明）进 21 份 Fluent——**跟下一次翻译批次走**，
  不单独开翻译会话；本期只把 en-US 的 key 建好。
- 出口：走 `save_config` 的测试在干净机器上不碰 `%APPDATA%`；Stamp 用例绿。

### P8.6 web 多页（中，1 天；待 §4 拍板 2）

- 建议首选**顺序触发 N 次下载**（零依赖、复用 `svg_job_to_string` 的单页路径，编号沿用 D3）；
  zip 打包等有真实消费者再说（加依赖是一笔树上的账）。
- 债4 顺路：起一次 `trunk serve` 人工点一遍单页与多页下载，记进 evidence（P4.1 挂的账一起销）。
- 出口：web PRINTALL 勾多布局能全部拿到；下载的字节与 native 落盘逐位相同的既有用例扩到多页。

## 3. 风险

- **P8.1 真图 600 dpi 的体量**：A1 @ 600 dpi ≈ 19842×14032 px，RGBA 单张 ~1.1 GB，两引擎并存峰值 2.5 GB+；
  63 GB 内存放得下，但要单线程、逐张跑，别与别的构建并发。
- **P8.2 的合流窗口**：主树 merge 未提交期间**不要**在主树上动 SVG 相关文件（P5.2 第一稿就是这么被盖掉的）；
  p52 并回前主树若又前进，rebase p52 那一笔即可（都是追加型改动）。
- **P8.3 外部工具漂移**：Inkscape / librsvg 版本写死进表头；工具缺失 = 表里留空并注明，不假绿。
- **P8.4 的边界**：区域裁剪只进 `PlotRequest`，谁都不许「顺手」改 `direct_plot_params`；PDF 字节有任何变化
  就是越界，第一层护栏当场抓。
- **merge_lines 的诱惑**：在 P8.3 拿到生态答案之前，不动导出侧的页外裁剪——它同时改两个门的字节，
  牵动第一层冻结对照，必须单独一期、先过拍板。

## 4. 待拍板

1. **分期顺序**：按 §2（P8.1 → P8.2 → P8.3 ∥ P8.4 → P8.5 → P8.6）走？还是把 G10（P8.4）提前到合流后第一件？
   建议按 §2——收尾与合流不做完，任何新代码都在给下一次覆盖事故供料。
2. **web 多页方案**（§6 Q1 悬了两天）：顺序 N 次下载（建议）还是 zip 依赖打包？
3. **merge_lines 后续**：若 P8.3 实测浏览器 / Inkscape 也丢那块填充（不是 resvg 一家的事），
   「导出侧裁掉页外墨」升格为正式一期（含冻结对照同步的完整仪式）；若只有 resvg 丢，
   就地记录、等 resvg 升级期再验。请提前给个倾向。
4. **p52 worktree 的去留**：合流后退役删除，还是留作下一个隔离期的模板？

## 5. 本轮审核的验证摘要

- 主树（mid-merge）：`cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes
  plot_destination_tests print_all_svg_tests` → 86 过 / 0 败 / 2 忽略。
- p52：`cargo test --lib -- io::svg_export io::pdf_export app::automation` → 73 过 / 0 败 / 4 忽略
  （例行栅格门 369 s 真跑，pdftoppm 25.07.0；注入门四对全抓）。
- 证据核对：`corpus-600dpi.tsv` 22 行（20 ok / 2 FAIL 有解释有对照）；`real-sheets-600dpi.tsv` **不存在**
  （README 声称存在，×）；两份 `raster_compare.rs` 哈希不同（主树 stray 为旧稿）。
- 记录核对：p52 §13 有 `REAL_SHEET_NUMBERS` / `CLIPPY_RESULT` 两个未填占位符。
