# 另存 DWG / DXF 后 `.pid` 文字的 TrueType 字体丢了：STYLE 只写了 `txt` · 小计划（2026-09-24 开单，同日批准并落地）

> 起因：计划 `2026-09-24-pid-import-status-and-next-steps.md` T3 截图时顺带看到（`docs/evidence/2026-09-24-pid-gui-check/0201-tags-before-after.png` 中栏 vs 右栏）；
> 用户「开一张小单：另存 DXF 后 PID 文字退成 txt 字体的问题」→ 本单（会话 opus-5-5-1）。开单时没改代码。
> **2026-09-24 Plannotator 批准**（`{"decision":"approved"}`，无批注），F-D1 – F-D4 按推荐执行；F1 – F4 见「进度」。

## 一句话

`.pid` 直接打开时文字按字符样式说的 TrueType 字体画（Arial Narrow、宋体……），另存 DWG / DXF 再打开就退成 `txt` 笔画字体：导入器只把字体名放在
`TextStyle::true_type_font` 里，`font_file` 留着 acadrust 的默认 `txt`，而 acadrust 读写 STYLE 时根本不碰 `true_type_font`——写出去是 `txt`，读回来也只有 `txt`。
第三方查看器拿到的同样是 `txt`。

## 事实（2026-09-24，OCS `f7bb8a33`，acadrust / cadcodec `5b682ed`）

| 项 | 事实 | 出处 |
|---|---|---|
| 现象 | 直接打开 `.pid`：位号按 Arial Narrow 画；由它另存的 DXF 重新打开：同样的字高，字形是 `txt` 笔画字体 | 证据三联图中栏 / 右栏 |
| 导入器怎么建样式 | `register_text_styles`：`TextStyle::new(name)` 后只设 `true_type_font = 字体名`；`font_file` 保持默认 `"txt"`，`height` 故意为 0 | `src/io/pid/text.rs:228–254`；acadrust `src/tables/textstyle.rs:80` |
| acadrust 写 DXF | STYLE 只写组码 3 = `font_file`、4 = `big_font_file`、`AcadAnnotative` XDATA；**`true_type_font` 不写** | cadcodec `src/io/dxf/writer/section_writer.rs:1210–1214` |
| acadrust 写 DWG | 同样只写 `font_file` / `big_font_file` 与注释性 EED | cadcodec `src/io/dwg/dwg_stream_writers/object_writer/mod.rs:911–946` |
| acadrust 读 | 组码 3 → `font_file`；不读 `ACAD` XDATA 里的字体名，读回来 `true_type_font` 为空 | cadcodec `src/io/dxf/reader/section_reader.rs:9278`；`rg true_type_font src/io` 零命中 |
| OCS 怎么挑字体 | `resolve_text_style`：`true_type_font` 非空就用它；否则看 `font_file`——`.shx` 找磁盘文件，其它取文件名主干当字体名（`arialn.ttf` → `arialn`，对不上族名 `Arial Narrow`） | `src/entities/text_support.rs:17–62` |
| 影响面 | 不止 `.pid`：OCS 里任何只靠 `true_type_font` 的样式另存都会丢；`.pid` 是每张图都中招的那一个（四图 `PID-*` 样式 3–5 个，全部 `txt`） | 四图 DXF 的 STYLE 表（`0201-tags-*` 那轮的 diff 脚本输出） |
| AutoCAD 的写法（待复核） | TrueType 样式的组码 3 写字体文件名（如 `arialn.ttf`），另在 STYLE 上挂 XDATA `ACAD`：`1000` 字体族名、`1071` 字符集 / 字宽标志 | DXF 参考，实现前对一份 AutoCAD 另存的 DXF 核一次 |

## 决策（等批；⭕ = 推荐）

| # | 决策 | 结论 |
|---|---|---|
| F-D1 | 修在哪 | ⭕ **先修 OCS 两头，再向 acadrust 上游提一处**。OCS：导入时把 `font_file` 写成该字体的 TrueType 文件名（用已有的 `fontdb` 按族名查），读回时 `resolve_text_style` 对 `.ttf` / `.otf` / `.ttc` 的 `font_file` 按文件查出族名——另存的图 OCS 自己认得、第三方按组码 3 也认得。acadrust：STYLE 读写 `ACAD` XDATA 的字体名（影响所有 TrueType 样式），写成 issue / PR 草稿，本单不 fork。备选：① 只改 acadrust——要 fork 或等上游，OCS 改钉 rev；② 只改 OCS 读侧——OCS 自己好了，第三方仍是 `txt` |
| F-D2 | 本机没装这个字体 | ⭕ `font_file` 保持 `txt`，导入日志点名「字体 X 本机没有，另存后会显示为 txt」；不编一个猜的文件名。备选：写 `<族名>.ttf` 占位——别的机器装了可能认得，但文件名多半不对（`Arial Narrow` 的文件是 `ARIALN.TTF`） |
| F-D3 | 验收 | ⭕ 四图 `--export`：只有 STYLE 表的组码 3 变（`txt` → 各自的 `.ttf`），实体与头变量字节不变；另存 DXF 重新打开，`resolve_text_style` 给出的族名与直接打开 `.pid` 相同（单测按本机字体条件跳过）；GUI 三联图中栏与右栏字形一致（补一张证据） |
| F-D4 | 影不影响已存的图 | ⭕ 不管：之前另存的 DWG / DXF 里已经是 `txt`，没有信息可恢复；user-guide 的 `.pid` 一节加一句「2026-09-24 前另存的图文字是 txt 字体」 |

## 工作项

- **F1 导入侧**（`src/io/pid/text.rs`）：`register_text_styles` 查字体文件、写 `font_file`；查不到的记一行日志；`pid.rs` 单测 + `pid_import` 钉四图 `PID-*` 样式的 `font_file`。
- **F2 读回侧**（`src/entities/text_support.rs`）：`.ttf` / `.otf` / `.ttc` 的 `font_file` 按文件查族名，查不到退回今天的「文件名主干」；单测。
- **F3 验收**：四图 `--export` 逐项比（只许 STYLE 组码 3 变）、`pid_batch_report` 基线重出、GUI 三联图补一张。
- **F4 上游**：acadrust STYLE 读写 `ACAD` 字体名的 issue / PR 草稿（附最小复现：`TextStyle::with_truetype` 另存 DXF 再读，`true_type_font` 为空）。

## 登记不做

| 项 | 理由 |
|---|---|
| fork acadrust 并改钉 | F-D1：先走上游 |
| 字体替换表（没装时换成相近字体） | 另一个产品命题；F-D2 只要求说出来 |
| 恢复已另存的图 | F-D4 |

## 进度（2026-09-24，本次提交）

- **F1 导入侧**（`src/io/pid/text.rs`）：`register_text_styles` 多收 `path`，用 `sysfont::face_file_name` 把 `font_file` 写成字体文件名；本机没有的字体保持 `txt`，记一行 info 日志。
- **F2 读回侧**：`sysfont` 加 `face_file_name` / `family_of_file`（按文件名不分大小写找面，宽度变体回到它自己的名字——`ARIALN.TTF` → `Arial Narrow`，集合取第一个面）；
  `resolve_text_style` 对 `.ttf` / `.ttc` / `.otf` 的 `font_file` 先按文件查族名，查不到退回原来的「文件名主干」。
- **F3 验收**：
  - 四图 `--export` 与 T2 基线逐项比：**实体、头变量零差异**，只有 STYLE 记录的组码 3 变——0201 / 0202：`arial.ttf` / `ARIALN.TTF` / `simsunb.ttf` / `simfang.ttf` / `simsun.ttc`；
    D06：`ARIALN.TTF`；工艺：`arial.ttf` / `ARIALN.TTF`，`仿宋_GB2312` 本机没装、保持 `txt`（导入日志点名）。新哈希（本机字体下）：0201 `B9C0890C…` / 0202 `84743800…` / D06 `240B3785…` / 工艺 `E4B45DDA…`；
    **这些哈希随本机装了哪些字体而变**，换机器对哈希前先看 STYLE 组码 3。
  - 测试：`pid_import` 51 → **52**（新增 `a_pid_text_style_keeps_its_typeface_through_a_dxf_round_trip`：0201 另存 DXF 再开，每个装了字体的 `PID-*` 样式落到同一个字体文件，没装的保持 `txt`）；
    `--lib` 过滤 `sysfont` / `io::pid` / `text_support` **72/72**（`sysfont` 新增 `a_face_file_leads_back_to_its_family`，Arial / Arial Narrow / Times New Roman 往返）；clippy 在改动处零新增（`text_support.rs` 另有五条旧告警不在改动行）；rustfmt 干净。
  - 批量基线 CSV 重出（六张图哈希全换、其余列不变）；GUI：`docs/evidence/2026-09-24-pid-gui-check/0201-saved-dxf-font-before-after.png`，修后另存的 DXF 重开按 Arial Narrow 画。
  - 过程记录：一次 `--lib` 测试链接失败（`link.exe` 1104，测试可执行被占），重跑通过；OCS 主工作树里另一会话的 `rvt` 改动这时已能编译，本单在主树里验，只提交自己的文件。
- **F4 上游**：issue 草稿见下节，未提交到上游。
- **F-D4**：user-guide `.pid` 一节「文字」段补了另存字体与 2026-09-24 前旧图的说明。

## 附：给 acadrust 上游的 issue 草稿（F4，未提交）

> **STYLE records drop `TextStyle::true_type_font` on DXF and DWG write and read**
>
> `TextStyle::true_type_font` is kept in memory only. The DXF writer emits code 3 (`font_file`), code 4 (`big_font_file`) and the
> `AcadAnnotative` XDATA (`src/io/dxf/writer/section_writer.rs`, STYLE writer); the DWG writer likewise writes the two font files and the
> annotative EED (`src/io/dwg/dwg_stream_writers/object_writer/mod.rs`); neither reader fills `true_type_font`
> (`src/io/dxf/reader/section_reader.rs`, `src/io/dwg/dwg_document_builder.rs`). A style built with
> `TextStyle::with_truetype("T", "Arial Narrow")` therefore saves as `txt` and reads back with an empty face.
>
> Minimal repro: add that style to a `CadDocument`, write DXF, read it back, and `true_type_font` is `""` while `font_file` is `"txt"`.
>
> AutoCAD keeps a TrueType style's face on the STYLE record itself: code 3 holds the font file name (e.g. `arialn.ttf`) and an `ACAD`
> XDATA group carries the family (`1000`) with a `1071` word of pitch-and-family / charset / bold / italic bits (DWG: the same as EED under
> the `ACAD` appid). Proposal: when `true_type_font` is set, write that group beside the annotative one, and read it back into
> `true_type_font` (the `1071` word can be carried raw until it is modelled). Exact bit layout to be checked against an AutoCAD-saved file.

## 门禁记录

- 2026-09-24：用户经 zhimo「开一张小单：另存 DXF 后 PID 文字退成 txt 字体的问题」→ 本单（会话 opus-5-5-1），送 Plannotator 批注。F-D1 – F-D4 等批。
- 2026-09-24：Plannotator 批准，用户「Plannotator 里批准了，开工 F1 到 F4」→ 开工并同日落地（会话 opus-5-5-1）。
