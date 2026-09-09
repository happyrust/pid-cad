# 开发计划：SVG 导出 · 第三阶段（审核 + P8 收尾与验收补齐）

> 日期：2026-09-09
> 状态：Plannotator 批准（2026-09-09，无批注）；**P8.1 / P8.2 已实施**（§6.1，真图三行 FAIL 的定性见
> §6.2——判据局限，非导出缺陷）、**P8.4 已实施**（§6.3）、**P8.3 已实施**（§6.4；merge_lines 的答案：
> **只有 resvg 丢**，Chrome / Inkscape / librsvg 都画——拍板 3 落地为「就地记录、不升格」）、**P8.5 已实施**（§6.5，
> 2026-09-10：测试不再碰用户配置、Stamp 消息在 SVG 下拦住、SVG 文案的 Fluent key 建齐）；
> 剩 P8.6（待拍板 2）与「判据学会彩色墨 / 亚 2 px 覆盖」一条新债（§6.2、§6.4）。
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
| P5.3 | 兼容性实测：rsvg-convert / Inkscape / Chromium 对 22 语料 + 3 真图 | next-steps §3 | **已实施**（P8.3，§6.4） |
| G10 | 模型空间 SVG 不看出图区域（Window/Display/Limits/View，恒取 Extents） | next-steps §2/§10 | **已实施**（P8.4，§6.3） |
| Q1 | web 多页（顺序 N 次下载 vs zip 打包），PRINTALL 的 SVG 在 web 上明确报不可用 | next-steps §6 | 待拍板 |
| 债1 | `new_for_test` 用真实 `%APPDATA%\OpenCADStudio\settings.json`，走到 `save_config` 的测试会写用户配置（P4.3 踩过，已污染过一次 `plot.background`） | next-steps §11 | **已修**（P8.5，§6.5） |
| 债2 | P4.2 的三条新文案没进 21 份 Fluent 目录（回落英文） | next-steps §10 | **key 已建**（P8.5，§6.5；译文等翻译批次） |
| 债3 | `PlotFlag::Stamp` 在 SVG 目的地下仍会翻转偏好（复选框已禁用，点不出来，低危） | next-steps §10 | **已拦**（P8.5，§6.5） |
| 债4 | web 真机 `trunk serve` 下载没人手验过（P4.1 出口条件挂账） | next-steps §8 | 未验 |
| 线索 | merge_lines：resvg 新版是否已修；导出侧要不要裁掉页外墨（牵动两个门的字节与第一层冻结对照） | p52 §13 | **P8.3 已答**（§6.4）：Chrome / Inkscape / librsvg 都画那块填充，只有 resvg 0.45.1 丢 → 导出侧裁页外墨**不升格**；resvg 升级时重跑 `-Only corpus` 复验 |
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
   **→ P8.3 实测（§6.4）：只有 resvg 丢。** Chrome 152 / Inkscape 1.4.4 / librsvg 2.62.91 对最小对照
   `merge-lines-minimal-drops.svg` 与整页语料都画出填充，与 poppler 一致；按上面第二支落地——就地记录，
   不升格，resvg 升级期复验。
4. **p52 worktree 的去留**：合流后退役删除，还是留作下一个隔离期的模板？

## 5. 本轮审核的验证摘要

- 主树（mid-merge）：`cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes
  plot_destination_tests print_all_svg_tests` → 86 过 / 0 败 / 2 忽略。
- p52：`cargo test --lib -- io::svg_export io::pdf_export app::automation` → 73 过 / 0 败 / 4 忽略
  （例行栅格门 369 s 真跑，pdftoppm 25.07.0；注入门四对全抓）。
- 证据核对：`corpus-600dpi.tsv` 22 行（20 ok / 2 FAIL 有解释有对照）；`real-sheets-600dpi.tsv` **不存在**
  （README 声称存在，×）；两份 `raster_compare.rs` 哈希不同（主树 stray 为旧稿）。
- 记录核对：p52 §13 有 `REAL_SHEET_NUMBERS` / `CLIPPY_RESULT` 两个未填占位符。

## 6. 实施记录

### 6.1 P8.1 + P8.2（2026-09-09 晚）

- **主树 merge**：暂存的解决 + 工作树里没暂存的收尾（wgpu `buffers: &[Some(…)]` 适配、
  `prepare_open_geometry` 三元组、`resolve_plot_job` / `with_stable_atlas` 的图集稳定性重试、8 份 locale 的
  PID 文案、`Cargo.lock` 再生）一并提交为 `fb62909e`——测试绿的就是这个组合。
  `tests/dwg_acadsharp_samples.rs` 里的 `probe_ac1015_clayer_tmp` 调试探针是别的会话的临时物，**没并进去**。
- **P5.2 收尾**：rustfmt（两文件 0 diff）与 clippy `--lib --tests`（改动行 0 告警）实跑并把 §13 的两个
  占位符换成事实；发现上一会话的真图证据跑还挂在后台，等它落盘（19:15）收下 `real-sheets-600dpi.tsv`。
  两笔提交在 `p52-svg-raster`：`e749d7cf`（P5.2 主体）+ `b17fdb27`（真图证据，三张全 FAIL 如实记录）。
- **合流**：`p52-svg-raster` 并回 main（`fc99218d`，自动合并零冲突）；主树过期 stray `raster_compare.rs`
  删除；合流后 `cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes
  plot_destination_tests print_all_svg_tests` **88 过 / 0 败 / 4 忽略**（含例行栅格门真跑 322 s）。

### 6.2 真图 FAIL 定性（2026-09-09 晚，`b45d4065`）

对生像素逐点核对（方法与量测数字在 evidence README「real-sheet triage」节）：**三张都是判据的局限，
不是导出缺陷**。SP02-05 / FF02-06 是 0.1 pt 发丝线撞上别的墨（poppler 把发丝线钉成整值一像素行、resvg 摊
真实覆盖；光纸上位移窗解释得掉，贴着红管线 / 表格线解释不掉）；WS02-05 是合成豁免不认识彩色墨
（标题蓝 `(0,38,128)`，`seam_ink`=96 按通道判，蓝的本通道 128 永远不合格）。三行 FAIL 挂注释留在证据里。
**新债**：判据学会彩色墨（按最暗通道认墨，重证四个注入故障）或真图证据升 1200 dpi——单独一期。

### 6.3 P8.4 G10 实施记录（2026-09-09）

**形状**：`PlotRequest` 长出 `dialog_area: Option<String>`（`None` = 历史行为，其余入口都发 None）；
出图对话框 commit 的 SVG 分支把 `d.area` 存进 `svg_export_dialog_area`（app 上的一格在途状态，
`current_view_svg_job` 用 `take()` 消费——同一个 `Message::SvgExport` 入口因此分得开「对话框提交」与
「EXPORTSVG 快捷命令」，后者永远拿不到区域，维持 Extents）；`resolve_plot_pages` 在 `layouts` 为空且带
区域时按 **PDF commit 同款调度**取页（`Display` / `Extents` / `Limits` / `Window` / `View: 名` →
`*_plot_job`，拒绝文案同一句 `Plot area is empty. Pick a larger window.`）；`"Layout"` 不进调度——它就是
`direct_plot_params` 本来画的纸面。保存对话框取消时清掉在途区域（`SvgExportPath(None)`）。
**没动 `direct_plot_params`**（菜单 Export PDF 与打印共用它，PDF 行为一位不变）。

| 文件 | 内容 |
|---|---|
| `src/app/update/file.rs` | `PlotRequest.dialog_area` + 构造函数；`resolve_plot_pages` 的区域调度；`current_view_svg_job` 消费在途区域；commit 的 SVG 分支存区域；`svg_plot_area_tests` ×4 |
| `src/app/mod.rs` | `svg_export_dialog_area: Option<String>` 字段 |
| `src/app/update/mod.rs` | `SvgExportPath(None)` 清在途区域 |

**出口条件对照**：对话框选 Window + SVG 目的地 → 出的是窗口不是 Extents（12 单位窗对 1010 单位 extents，
比例差 >10× 被钉住）✓；EXPORTSVG 在对话框仍留着 Window 时照旧出 Extents（逐位同一比例）✓；取消保存
对话框不留在途区域 ✓；空窗口拒绝且文案与 PDF 目的地同句（按 `t!` 比，本机中文局面下也对）✓；
PDF 第一层护栏与全部既有用例见下方验证数字。

**验证摘要**（2026-09-09 晚，提交前在最终字节上跑）：

- `cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes plot_destination_tests
  print_all_svg_tests svg_plot_area_tests` → **92 过 / 0 败 / 4 忽略**（314.9 s，含例行 PDF↔SVG 栅格门真跑；
  比 P8.2 合流后多的 4 条就是 `svg_plot_area_tests`）。PDF 第一层冻结对照在这一套里，**一位没动**。
- `cargo check --lib --target wasm32-unknown-unknown` → Finished；5 条告警全是 `../iced/Cargo.toml` 的
  lint 名写法弃用，与本期无关。
- `cargo clippy --lib --tests` → 0 error；改动行 0 告警（`file.rs` / `app/mod.rs` / `update/mod.rs` 的新行
  逐行过滤）。跑到这一步先撞上两条**既有**的 `approx_constant` 错误（deny-by-default，让 test target 根本
  编不过 clippy）：`doc_api.rs` 测试里的 `0.7071`、`scene/mod.rs` 测试里的 `3.14159`——换成 `FRAC_1_SQRT_2` /
  `PI`，**单独一笔提交**（`b8ae09ee`），不混进 G10。
- `rustfmt --check` 对三个改动文件：新增行 **0 diff**；`file.rs` 另有 120 余处既有格式差异，沿用「只体检新代码」，
  没顺手全文重排（那会把 diff 淹掉）。
- 未做人手 GUI 点验（无窗口驱动的是状态与消息，同 P4.2 的挂账，随 P8.6 的 web 真机验证一起销）。

### 6.4 P8.3 兼容性实测记录（2026-09-09 晚）

**形状**：`scripts/svg-compat.ps1`（Windows 本机，三步各可跳）——① 样本：`dump_the_corpus_for_a_human` 出 22 页
语料 + 栅格证据目录里的 merge_lines 最小对照一对；三张真图（FF02-06 / SP02-05 / WS02-05）走 debug 二进制
`--plot-svg --model --paper A1 --landscape --fit`（与栅格证据同一次出图）。② 渲染：Chrome / Edge headless
`--screenshot`（device scale = dpi/96，截图裁回页面；**`--force-gpu-mem-available-mb=4096`**，不然 A1 @ 300 dpi
第 6351 行以下整片白、不报错）、`inkscape.com --export-type=png --export-dpi`、librsvg 经 libvips / sharp
（winget 没有 `rsvg-convert` 包；sharp 装在 `%LOCALAPPDATA%\ocs-svg-compat\node`，密度取 √(72×dpi) 抵消 libvips
对 mm 页面的二次缩放）；版本进 `versions.txt`，缺的渲染器**留空列并注明**。③ 对比：新增 `#[ignore]` 用例
`compare_external_renders`（`OCS_SVG_COMPAT_DIR` / `OCS_SVG_COMPAT_DPI`）用 P5.2 同一个比较器把每张外部 PNG 与
resvg 同 dpi 渲染逐对比，写 `compat-<dpi>dpi.tsv` + diff 图，`--release` 跑。**都不进依赖树。**
resvg 是参照只因为它是树里能跑的那一个；FAIL = 「有分歧、去看」，不是谁错的裁决。

| 文件 | 内容 |
|---|---|
| `scripts/svg-compat.ps1` | 新，上述三步；`-Only corpus` / `-Only real`、`-Renderers`、`-Dpi`（600）/ `-RealDpi`（254）、`-SkipSamples` / `-SkipCompare` / `-NoInstall` / `-Force` |
| `src/io/svg_export/tests.rs` | `compare_external_renders`（ignored；渲染器 × 样本 → tsv 一行 + diff 图） |
| `docs/evidence/2026-09-09-svg-compat/` | README、`versions.txt`、`corpus-600dpi.tsv`、`real-sheets-254dpi.tsv` + `-structural.tsv`、`real-sheets-300dpi-superseded.tsv`、`calibration-150dpi.tsv`、8 张四引擎 4-up 对照图 |

**结果**：

- **语料 600 dpi**：24 样本 × 3 渲染器 = 72 对，**66 ok / 6 FAIL**；ok 的全部 0–1 缺陷像素（35 Mpx 一页），
  含计划点名人工看的负 scale+clip+文字（01–04）、细虚线（05）、multiply+wipeout（13）、文字（17）、CTB（15–16）。
  6 条 FAIL 全是 merge_lines 两页（语料页 + 最小对照 `drops`）× 三渲染器。
- **merge_lines 的答案：只有 resvg 丢。** Chrome 152 / Inkscape 1.4.4 / librsvg 2.62.91 在
  `merge-lines-minimal-drops.svg` 与整页语料上**都画出**那块 multiply 填充（与 resvg 的差 9119 / 9119 / 9214 px
  正是填充面积），`renders` 变体三家 0 缺陷；与 poppler 一致。→ §4 拍板 3 落地为第二支：导出侧裁页外墨
  **不升格**，就地记录，树里 resvg 升级时重跑 `scripts\svg-compat.ps1 -Only corpus` 复验。
- **真图 254 dpi**（10 px/mm，A1 = 8410×5940，四引擎同尺寸）：9 对**按局部判据全 FAIL**（96–5956 缺陷 px，
  最差瓦片 24–44 > 预算 8），但**结构性缺陷 0**——每个缺陷像素两侧 2 px 内都有墨，没有缺失 / 位移 / 变色
  （逐对分类见 `real-sheets-254dpi-structural.tsv`）。成分：x = 507 mm 一根亚像素发丝线 resvg 50 % 覆盖 vs 其余
  三家 61 %；SP 右侧材料表细表格线 75 % 覆盖的边行 resvg 63 vs Inkscape 60（一级之差，沿 380 mm 表格线积成
  3798 px——Inkscape 计数偏高的全部原因）；Skia 超采样覆盖（39–47 vs 63）与笔画外淡边；红管线边行
  (194,92,92) vs (159,95,95)；密排 2.5–3.5 mm 文字的字形笔画边缘。最差瓦片 4-up 四引擎几何一致。
  **定性：判据局限（跨引擎 AA 覆盖策略在 ≤ 2 px 特征上不一致），非导出缺陷**；行保留 FAIL 加注，不调预算。
- 校准记录留档：先跑的 **300 dpi** 真图三渲染器都在底边框线一行 ~300 瓦片 FAIL（Chromium 尺寸与 resvg 完全一致
  也一样——横边落在分数像素行上，沿全长覆盖值不一致；Inkscape / librsvg 另把页面四舍五入成 9933 宽），254 dpi
  下整行消失；**150 dpi** 全集里 0.75 pt 线 = 1.5 px，细线页（dash / ctb / colour / two_groups、cairo 两家的
  pen_widths 08）全被 AA 量化绊倒，600 dpi 下同页全 0 → 不是导出的事。

**出口条件对照**（§2 P8.3）：脚本 ✓（缺渲染器留空注明，不假绿）；版本进表头 ✓；重点样本人工看 ✓（600 dpi 三引擎
0 缺陷 + 4-up 图）；merge_lines 最小对照喂给每个渲染器 ✓（resvg 一家）；evidence 一张表 + 重点 PNG 并列 ✓；
偏差各开后续 ✓（README「Follow-ups」：merge_lines 随 resvg 升级复验；真图跨引擎 FAIL 并入 §6.2 那期「判据学会
彩色墨」——扩成「彩色墨 / 亚 2 px 覆盖或 1200 dpi」，动阈值前先证注入故障仍被抓；导出侧无事可修）。

**验证摘要**（2026-09-09 晚）：

- `compare_external_renders`（release）：语料 600 dpi 72 对 → 303.7 s / 310.9 s 两次复跑同表；真图 254 dpi 9 对
  → 57.6 s。对比跑在本用例的最终逻辑上；其后唯一改动是 rustfmt 对一行 `format!` 的换行（无语义变化）。
- 真图缺陷结构性分类：临时 numpy 脚本（未入库）逐像素判「另一侧 2 px 内有无墨」→ 9 对 **0 结构性**。
- `rustfmt --check --edition 2021 src/io/svg_export/tests.rs` → **整文件 0 diff**。
- `cargo clippy --lib --tests` → **0 error**，Finished；`tests.rs` 整文件 0 命中（新用例所在的 1636–1745 行自然也是 0）；
  其余 1104 条 lib 告警全是既有的，不在改动处。
- `cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes plot_destination_tests
  print_all_svg_tests svg_plot_area_tests` → **92 过 / 0 败 / 5 忽略**（319.6 s，含例行 PDF↔SVG 栅格门真跑；
  多出的 1 条忽略就是 `compare_external_renders`）。本期没动生产代码，只加了一条 ignored 用例；
  跑它是守「每期护栏都要绿」的规矩。
- 没有 Illustrator（按 v2 D1 口径「按版本实测，不承诺」，本机没有）；Edge 152 装了但脚本取到 Chrome 就用 Chrome，
  两者同一 Blink，没有分列。

### 6.5 P8.5 小债打包实施记录（2026-09-10 凌晨，`p85-small-debts` 分支，在 `../OpenCADStudio-p52` worktree 里做）

主树当时正被 P&ID 图例那条会话连续提交（`68394097` → `6f0cf040`），同一棵工作树上再开一条会话就是 P5.2 第一稿被盖掉的
剧本重演，所以按 §3 的老规矩换到 worktree：`p52-svg-raster` 已并回 main，就在它上面从 main 拉出 `p85-small-debts`
（拍板 4 的「留作下一个隔离期的模板」就这样用上了）。**没动生产路径上的 PDF 代码**，`direct_plot_params` 与 PDF 字节一位没碰。

| 债 | 形状 | 文件 |
|---|---|---|
| 债1 | `crate::config::config_dir()` 在 `cfg(test)` 下返回 `%TEMP%\OpenCADStudio-test-<pid>`（`OnceLock`，进程内稳定）；真实的平台解析搬进 `platform_config_dir()`，非测试构建一行不变。于是 `settings.json`、`ocad.pgp` 别名、`last_dir.txt` 在 `cargo test --lib` 下**全部**落进临时目录，`new()`（`new_for_test` 就是它）也从全默认配置起步，不再读开发机的设置。两处「先喂 `last_saved_config`」的绕法删掉。两条用例钉住：临时目录在 `temp_dir()` 下、不在用户目录下、进程内稳定；`AppConfig::default().save()` 写到的正是它 | `src/config.rs`；`src/app/update/file.rs`（`svg_plot_area_tests` / `print_all_svg_tests` 的两处绕法） |
| 债3 | `on_plot_dlg` 里 `PlotFlag::Stamp` 加守卫 `d.destination() != PlotDestination::Svg`——与相邻的 `Center if d.area != "Layout"` 同一写法；SVG 下这条消息是 no-op，PDF 下照旧翻转。用例 `the_stamp_switch_is_refused_under_svg_and_still_toggles_under_pdf` | `src/app/update/file.rs`（`plot_destination_tests`） |
| 债2 | `Export SVG`、`Save to SVG file…`、stamp 说明三条进 `locale_catalog.rs` 与 **21 份** Fluent 目录；顺手把同一批 SVG 文案里另两条也收了（P4.3 的 web 多页提示、PRINTALL 的 `SVG` 标签 → `common.svg`，对着 `common.pdf`）。`i18n::tests::every_catalog_covers_and_formats_the_source_catalog` 要求 21 份目录 key 集合完全一致，所以「只建 en-US」走不通——非英文目录先放**英文占位**（与目录里既有的 `PDF` 一类同款），译文跟下一次翻译批次；界面可见行为不变（之前就是回落英文）。`scripts/test_locales.py` 的缺口从 86 → 82，剩的全是 `model_ops.rs` 等别处的既有债 | `src/locale_catalog.rs`（+5 行）、`locales/*/opencadstudio.ftl`（各 +5 行，位置照 en-US 的邻居：`.export-pdf` / `.save-to-pdf-file` / `.merge-overlapping-lines` / `.shaded-viewport-options` / `common.pdf` 之后） |

**没做 / 边界**：`cfg(test)` 只覆盖本 crate 的单元测试（`cargo test --lib`）；`tests/` 下的集成测试以普通构建编译，仍走真实配置目录
（目前那几条都不碰 `save_config`）——要连它们也隔离，得走环境变量一类的运行期开关，不在本期。临时目录随 pid 走，跑完不清
（每次几 KB，`%TEMP%` 里可见 `OpenCADStudio-test-*`）。GUI 仍未人手点验（与 P8.4 同一挂账）。

**验证摘要**（2026-09-10 凌晨，提交前在最终字节上跑）：

- `cargo test --lib -- io::svg_export io::pdf_export app::automation io::paper_sizes plot_destination_tests
  print_all_svg_tests svg_plot_area_tests config::tests i18n::tests` → **104 过 / 0 败 / 5 忽略**（316.5 s，含例行
  PDF↔SVG 栅格门真跑）。比 P8.3 的 92 多出的：Stamp 守卫用例 1、`config::tests` 2、`i18n::tests` 整组（其中
  `every_catalog_covers_and_formats_the_source_catalog` 就是 21 份目录 key 集合一致的那道门）。PDF 第一层冻结对照在这一套里，
  **一位没动**。
- 隔离实证：跑完后 `%APPDATA%\OpenCADStudio\settings.json` 的修改时间仍是跑前的 23:25:35；`%TEMP%\OpenCADStudio-test-<pid>\`
  里出现 `settings.json` / `ocad.pgp` / `ocad.pgp.version`——正是原先会写进用户目录的三样。
- `scripts/test_locales.py`（改成打印全部缺口的临时副本）：缺口 86 → 82，SVG 相关五条全部消失；无「Missing Fluent targets」、无重复 key。
- `cargo clippy --lib --tests` → **0 error**，Finished；`config.rs` / `locale_catalog.rs` 整文件 0 命中，`file.rs` 改动行 0 命中；
  lib 告警总数仍是 1104（与 §6.4 记录同数，本期没添一条）。
- `cargo check --lib --target wasm32-unknown-unknown` → Finished（`config_dir` 的测试分支全在 `not(wasm32)` 之下）。
- `rustfmt --check --edition 2021`：`config.rs` 整文件 0 diff；`file.rs` 改动行 0 diff（另有 124 处既有差异，沿用「只体检新代码」）。
