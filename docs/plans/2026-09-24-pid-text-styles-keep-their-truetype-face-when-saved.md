# 另存 DWG / DXF 后 `.pid` 文字的 TrueType 字体丢了：STYLE 只写了 `txt` · 小计划（2026-09-24 开单，等批）

> 起因：计划 `2026-09-24-pid-import-status-and-next-steps.md` T3 截图时顺带看到（`docs/evidence/2026-09-24-pid-gui-check/0201-tags-before-after.png` 中栏 vs 右栏）；
> 用户「开一张小单：另存 DXF 后 PID 文字退成 txt 字体的问题」→ 本单（会话 opus-5-5-1）。只开单，**没改代码**。带 ⭕ 的决策按推荐落笔，批注里划一笔即可翻案。

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

## 门禁记录

- 2026-09-24：用户经 zhimo「开一张小单：另存 DXF 后 PID 文字退成 txt 字体的问题」→ 本单（会话 opus-5-5-1），送 Plannotator 批注。F-D1 – F-D4 等批。
