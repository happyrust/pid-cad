# `.pid` 解析与显示 · 现状盘点与下一步 · 开发计划（2026-09-24 开单，同日批准）

> 用户 2026-09-24（zhimo 会话 opus-5-5-1）：「分析 D:\work\plant-code\cad\OpenCADStudio 现在PID文件解析和显示的实现进度。并使用plannator 制定下一步的计划」。
> 基线：OCS `dbf62cb5`（= `pid-cad/main`；`ec14ce55` 之后只有文档提交）；pid-parse `681681c`（`codex/phase32c-bundle-closeout`，= origin）。
> 开单时只做分析与计划，**没改任何代码**。
> **2026-09-24 Plannotator 批准**（`{"decision":"approved"}`，无批注）：N-D1 – N-D11 按推荐放行；用户「按推荐顺序开工（T1 与 S 并行起步）」。
> 本单只管 SmartPlant `.pid` 这条线；DXF 图例识别（`PIDLEGEND` / `PIDGROUP`）有 09-09 / 09-10 两单，W0–W8、M0–M6 已全部落地，不在这里。

## 一句话

`.pid`「打得开、画得全」在现有语料上已经基本到头：五张图的记录族全覆盖（剩 13 条拒收 + 1 条无解码器，全部点名），
107/107 个符号放置画的是图纸自己缓存的本体，图层 / 显隐 / 线宽 / 颜色 / 虚线 / 驱动尺寸 / 语义都已接上，
四图 `--export` 三版字节一条线（今天用现成可执行又对了一次）。下一步的收益不在「再多解几条记录」，而在三处：
**① 文字按 run 取样式**——08-22 已定谳、一直没接线，是今天屏幕上最大的一处可见偏差（四图 155 条文字里 106 条的字高 / 字体会变）；
**② 欠着的 GUI 验收**——两张单留着「未验证」，两项等 SmartPlant 截图；**③ 语料只有 5 张**——之后几乎每条「登记不做」都卡在「语料没有」，
先把批量回归工具备好，新图一来就能一次量出新缺口。

## 现状（2026-09-24）

### 解析：pid-parse

| 层 | 现状 | 出处 |
|---|---|---|
| 容器与流 | CFB 读全部流；顺序语义 pass：summary → Drawing / General XML → `JSite`（含嵌套符号定义缓存与它自带的 `StyleCluster`）→ `PSMcluster0` / `StyleCluster` / 每条 `Sheet*` → dynamic attrs → PSM 表（含 `sheet_layers` / `view_filter_sets`）→ doc registry → crossref → geometry hints | 09-21 审核 §一 |
| Sheet 记录族 | 15 族注册表；`imagdex.dex` 的曲线族（线 / 点 / 折线 / 圆 / 弧 / 矩形 / B 样条）全部解码；`igTextBox` 三形状 260/260；`0x0115 JDim` 只收种类 1 | `task_plan.md`；08-22 分析 |
| 没画出来的 | 五图拒收 1 / 4 / 8 / 0 / 0（0201 一条 `0x00FA`；0202 四条、工艺八条 `0x0084` 折线——工艺那八条是退化两点线，已判正确拒收）；无解码器 0 / 0 / 0 / 0 / 1（A01 一条 `0x007B igGroup`，容器、无几何） | `tests/render_gap_census.rs`，今天 4/4 |
| 样式 | 按存储作用域取 `StyleCluster`：线宽 / 颜色 / 虚线四图 558/558 条解析；文字走两跳（`+14` → 段落样式 `+38` → 字符样式），184 条里 159 条出字高，25 条指向 0.254 mm 哨兵样式、正确拒收（OCS 回退 2.5 mm）。**`igTextBox` 自带的字符样式 run 没进 DTO**（08-22 定谳「run 赢」，未接线） | 07-27 快照批注；08-10 / 08-22 分析 |
| 符号 | 107/107 放置解到缓存本体；缓存本体带逐笔图层 / 显示位 / 样式（含虚线）；`.sym` 库 618 个只作补位；B 样条采样；`igArc2d` 顺时针 | 09-07 / 09-19 / 09-20 |
| 图层 | `JSheetLayer` 290/290，1240/1240 图元带 `sheet_layer_ref`；`Top ViewFilterSet` 给显示位 | Phase 41；09-14 |
| 参数化 | `JDim` → 模板驱动尺寸（名字 / 公式）；`0x00ED JFlavorHolder` → 放置实例参数 | 09-18 / 09-20 |
| 语义 | `_Data.xml` 的 `GraphicOID` 两跳挂到实体（类型 / 位号 / 管线号） | 08-07 |
| 管线 | 一张图只开一次（S 单）；`ParseProfile::Geometry` 只跑画图要的 pass（G 单）；0201 解析 debug 0.5 s | 09-22；`cd100b7` |

### 显示：OCS

| 项 | 现状 | 出处 |
|---|---|---|
| 入口 | `src/io/pid.rs` 4 033 行（开单时误记为非空行数 3 879；S 之后拆成 `src/io/pid/` 九个文件）；`load_pid` 41 行编排：`prepare_document` → 找符号库 → `resolve_styles` → 读单位 → `build_document_entities` → `finish`，返回 `PidImport { document, summary }` | ⑤ 单 |
| 四图今天实测 | 0201 334 实体 / 206 条记录 / 1 条没画；0202 313 / 179 / 4；D06 59 / 25 / 0；工艺 686 / 427 / 8；缓存本体 20 / 23 / 6 / 58，库本体全 0 | 本单「验证」 |
| 字节基线 | 四图 `--export` SHA-256 = `2B1022B5…` / `340ED098…` / `763CAD1A…` / `B04C7215…`，与 `2e9e10f5` / `edc6b495` / `ec14ce55` 三版相同；单图导入 + 导出 0.4–0.9 s（debug） | 本单「验证」；⑤ 单 |
| 用户看得到的 | 只读源（`Ctrl+S` 改另存 DWG）；文件关联；导入摘要三行 + 单位回退一行（21 语种）；原图图层名与显隐；图层管理器「图纸图层」视图与 `PID_VIEW_FILTER`；特性面板 P&ID 组（类型 / 角色 / 本体尺寸 / 驱动尺寸库默认 / 本图实例 / 位号 / 图纸图层）；XDATA `PID_SEMANTICS` 随 DWG / DXF 保存 | user-guide `.pid` 一节 |
| 测试 | `pid_import` 50、`--lib io::pid` 54（⑤ 单 09-22 的记录，本轮未重跑） | ⑤ 单 |

### 还开着的（从各单「未验证 / 开口 / 登记不做」汇总）

| # | 项 | 性质 | 卡在哪 |
|---|---|---|---|
| 1 | 文字按 run 取样式 | 可见偏差 | 不卡：证据与三个坑 08-22 都写了（`pid-parse/docs/analysis/2026-08-22-run-beats-paragraph-default.md` §5） |
| 2 | ⑤ 单 GUI 三行核对；图层槽单 H4 截图 | 验收欠账 | 当时无桌面 / 桌面被占 |
| 3 | 逐笔线宽 vs 放置线宽；SmartPlant 截图对三例复核（09-19 P-D8） | 待裁 | 要一张 SmartPlant 截图 |
| 4 | 0202 四条 `0x0084`、0201 一条 `0x00FA` 拒收 | 未定性 | 没人量过（工艺八条已判正确） |
| 5 | `src/io/pid.rs` 4k 行拆不拆模块 | 结构 | ⑤ 单说「等 G4 / ⑤ 都落地后看」——已落地 |
| 6 | pid-parse `main` 落后工作分支 181 个提交（无分叉、可快进；本地 `main` 另有 5 个未推，都已含在工作分支里） | 仓库卫生 | OCS 按路径依赖 `../pid-parse`，谁把它切回 `main` 谁编不过 |
| 7 | B 系 / ANSI 页幅、英制单位、`igDimension` / `igBalloon` / `igLeader`、JDim 其余 7 种、`0x0010` 子记录、A01 `Default` 计数差 4 | 语料缺口 | 本机只有 4 张不同的 `.pid`（全盘按 CFB 魔数扫过）+ A01 |

## 第 1 项的量（08-22 探针，本单未重跑）

| 口径 | 条目 | 带 run | 无 run | 两路一致 | **会变** | 字高 | 字体 | 颜色 | run 打架 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 棘轮四图 | 155 | 145 | 10 | 39 | **106** | 103 | 54 | 2 | 11 |
| 0201 | 58 | 52 | 6 | 22 | **30** | 29 | 12 | 1 | 0 |
| 0202 | 52 | 48 | 4 | 15 | **33** | 31 | 13 | 1 | 1 |
| 工艺 | 41 | 41 | 0 | 2 | **39** | 39 | 27 | 0 | 10 |
| D06 | 4 | 4 | 0 | 0 | **4** | 4 | 2 | 0 | 0 |

最显眼的两类：36 条标签段落默认 3.175 mm、run 说 7 pt = 2.469 mm（今天画大 28.6%）；11 条段落默认字体 `Braggadocio`（极粗展示体），
run 说 `Arial Narrow`。run 的字高全落在整半磅上（4.5–12 pt），段落侧是 1.5 / 2.5 / 3.5 mm 这类制图标称值——run 才是实际排出来的字号。
run 的形状：`igTextBox` 形状 2 / 3 带 `(u16 长度, u16 选择子, u32 样式 id)` 条目，选择子 1 指字符样式（320 条里 313 条是 `0x002C`），
选择子 2 是段落样式的重述；原生 `GetSize` 的不变式「选择子 1 长度和 = 字符数」语料 228/228。

## 决策（等批；⭕ = 推荐）

| # | 决策 | 结论 |
|---|---|---|
| N-D1 | 下一轮主线 | ⭕ **T 线：文字按 run 取样式**。最大的可见偏差、证据三条互相独立（厂商对象模型 + 原生不变式 + 语料点值）、坑已列好、两仓各一段。备选：① 先做批量回归（B 线）——长期最值，但今天只有 5 张图，量不出新东西；② 先拆 `pid.rs`（S）——纯搬家，零可见收益 |
| N-D2 | run 与段落怎么分工 | ⭕ 字高 / 字体 / 颜色取 run；对齐 / 行距仍取段落（它们是段落属性，只换入参会把 08-13 修好的 145 条对齐退回去）；无 run 的用段落默认——那正是厂商模型里段落字符样式该生效的地方，两跳链不删 |
| N-D3 | 一条文字里 run 不一致（四图 11 条，工艺占 10） | ⭕ 取覆盖字符最多的那条，压平**点名**：导入日志一行 + 摘要一个计数，不悄悄取第一条。备选：写成 MTEXT 内联格式码逐段保真（管道号里 `-` 与号段两种样式、`m^3` 的上标 `3`）——更准，但 `.pid` 文字今天是单行 `Text` + 拆行，改 MTEXT 牵动对齐 / 拆行 / 导出，另开单 |
| N-D4 | run 指到的不是字符样式（320 条里 7 条：1 条 `0x002D`、6 条 `0x002E`） | ⭕ 那条 run 当作解不出，退回段落默认并计数；不猜 |
| N-D5 | pid-parse 接口 | ⭕ `DecodedIgTextBoxRecord` 加 `runs: Vec<DecodedTextRun { len, selector, style_id }>`（`serde(default)`，形状 1 为空；不满足 `GetSize` 不变式的整条丢 run 并计数）；`style_link` 加 `text_styles_for_document`，**段落与 run 两份都交出去**、由 OCS 拼；旧 `text_heights_for_document` 留一轮，OCS 切过去后退役 |
| N-D6 | 验收口径 | ⭕ 四图 `--export` **预期会变**，不再比字节：改前 / 改后两份 DXF 逐实体比，**非文字实体零差异**；差异只许出现在 TEXT（字高 / 样式 / 颜色，以及多行标签随字高变的行距落点）、STYLE 表与图面范围；变的条数对上探针。新基线 SHA-256 写进本单。`style_link_ratchet` 里「3.175 mm 是最常见字高」那条注释与断言按 08-22 §3 改正 |
| N-D7 | `pid.rs` 拆模块 | ⭕ **拆，放在 T2 之前、与 T1 并行**：纯搬家成 `src/io/pid/`（`mod` / `summary` / `styles` / `build` / `symbols` / `text` / `page`，名字实现时定），验收四图 `--export` 与 `dbf62cb5` 字节相同；T2 的改动随后落在 `text` 里，diff 干净。备选：T 之后再拆 / 不拆 |
| N-D8 | GUI 验收欠账 | ⭕ 用本机 OCS 可执行 + 桌面自动化截图补 ⑤ 三行与 H4（证据进 `docs/evidence/`）；SmartPlant 那两项**要你给截图**，给了再裁（逐笔线宽裁成「逐笔赢」时改的是 `apply_symbology` 一处），不给就继续登记 |
| N-D9 | 批量回归（B 线） | ⭕ T 之后做：一个 example 对目录里每张 `.pid` 跑导入 + `--export`，出 CSV（实体 / 记录 / 没画 / 拒收族与条数 / 单位 / 页幅 / 缓存 vs 库 / 耗时 / 失败原因）；五图基线跟本单事实表逐数对上。**新语料要你提供**（SmartPlant 工程的 `.pid`，最好连同 `_Data.xml` 与 `Ref\Symbols`），有了才动第 7 项 |
| N-D10 | pid-parse `main` | ⭕ 快进到工作分支并推 origin——**动远端，要你点头**；不点头就只在本单登记 |
| N-D11 | 两处未定性拒收（0202 ×4、0201 ×1） | ⭕ 只定性不改码：探针 + 一页分析，结论写进 `render_gap_census` 注释；判成解码器缺口再开单 |

## 工作项

### T 线（主线）

- **T1 pid-parse**（N-D2 / N-D4 / N-D5）：`igTextBox` 解码器把形状 2 / 3 的 run 条目交出来 → DTO `runs`；`text_styles_for_document`；
  run 打架 / 非字符样式 / 不变式失败三种计数；棘轮复现 08-22 的表（155：145 / 10 / 39 / 106，打架 11），`style_link_ratchet` 改正 3.175 mm 那条。
  `--lib`、`parse_real_files` 135、`render_gap_census` 4、`style_link_ratchet` 15、golden 不降；clippy `-D warnings` 零告警。
- **S OCS**（N-D7，与 T1 并行）：`pid.rs` → `src/io/pid/` 纯搬家；`--lib io::pid` 54、`pid_import` 50 不变；四图 `--export` 字节相同。
- **T2 OCS**（N-D2 / N-D3 / N-D6）：`resolve_styles` 改吃 `text_styles_for_document`；字高 / 字体 / 颜色取 run、对齐 / 行距取段落；压平计数进 `ImportSummary` 与日志；
  `register_text_styles` 多收 run 点名的字体；`pid_import` 钉 0201 / 0202 的新字高与字体（至少 3.175 → 2.469 mm 与 `Braggadocio` → `Arial Narrow` 各一例）；四图逐实体比对。
- **T3 证据与台账**：0201 / 0202 标签特写改前 / 改后截图（`docs/evidence/`）；user-guide `.pid` 一节补一句「文字字号按文字自己的格式，而不是样式表默认」；
  pid-parse `task_plan.md` 与 08-22 分析头部写「已接线」；本单写哈希。

### V 线（验收欠账，穿插，不占期）

- **V1**：打开 0201，命令行三行与 ⑤ 单写的一字不差、另存 `.dwg` 再打开不出 P&ID 导入行；图层管理器「图层」视图截图（图层槽单 H4：列的是原图图层名，`HiddenObjects` 为关）。两单改标。
- **V2**（等你的 SmartPlant 截图）：逐笔线宽裁决（工艺 `Xa` OPC 特写或 D06 Ball Valve Type 1 一张就够）；Ball Valve Type 1 / Remarks / Item Note & Label 三例复核。

### B 线（T 之后）

- **B1**：批量回归 example + 五图 CSV 基线。
- **B2**（等新语料）：跑新图，按 CSV 开单。

### 卫生

- **H1**（N-D10，等点头）：pid-parse `main` 快进并推。
- **H2**（N-D11，随 T1 在 pid-parse 做）：两处拒收定性。

## 执行顺序 ⭕

**T1 ∥ S → T2 → T3**；H2 随 T1；V1 穿插；B1 接在 T3 后；H1 点头即做。时间盒：T 线两个工作日，S 半天，B1 半天。

## 验收

- T：N-D6 口径；T1 棘轮复现 08-22 的数；T2 后四图新基线与「会变」条数写进本单。
- S：四图 `--export` 与 `dbf62cb5` 字节相同；`pid_import` 50 / `--lib io::pid` 54。
- V1：截图进 `docs/evidence/`，⑤ 单与图层槽单的「未验证」改标。
- B1：五图 CSV 与本单「四图今天实测」逐数一致。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 多 run 富文本逐段保真（MTEXT） | N-D3 备选，另开单 |
| B 系 / ANSI 页幅、单位判定改读更多来源 | 语料没有（09-21 审核 ⑦、⑤ 单登记） |
| `igDimension` / `igBalloon` / `igLeader` / `0xFF`、JDim 其余 7 种 | 语料 0 条；图形类未知码已有点名告警 |
| `0x0010` 子记录语义（638 条） | 不影响显示 |
| 用实例参数重算几何（09-20 P-F7） | 放置画的已是缓存里的实例形 |
| 网页版打开 `.pid` | pid-parse 为读 SQL Server 备份带了 sqlite3 与 `oxidized-mdf`，不编 wasm；要做先把备份模块做成可选特性，另开单 |
| 按视图过滤集分别呈现图层状态；`.pid` 图配图例识别规则 | 09-07 / 09-21 已登记，照旧 |

## 验证（本单开单时）

- pid-parse `681681c`：`cargo test --test parse_real_files --test style_link_ratchet --test render_gap_census --test geometry_profile` → **135 / 15 / 4 / 2 全绿**。
- OCS：共享 target 里现成的 debug `OpenCADStudio.exe`（09-23 10:25 编；`ec14ce55` 之后 OCS 只有文档提交）对四图跑 `--export`：
  **SHA-256 与 ⑤ 单基线逐一相等**，大小 188 308 / 189 053 / 90 882 / 319 578 B；`RUST_LOG=info` 的导入日志给出「四图今天实测」那一行的数。
- 全盘（`D:\work`，深 6 层）按 CFB 魔数找 `.pid`：12 个文件，只有 4 张不同的图（0201 / 0202 / D06 / 工艺），外加 publish 目录的 A01。
- **未重跑**：OCS `pid_import` / `--lib io::pid`——共享 target 的 `deps` 已被清空，冷编译太久，本轮不插队；数字取 ⑤ 单 09-22 的记录。
  08-22 run 探针的数字取自分析文档，T1 第一步复现。

## 进度

### T1（pid-parse `886c431`）

- `IgTextBoxRun` / `DecodedTextRun`：解码器把形状 2 的那一条（`+22`，即它本来就在校验的 `count | 0x10000` 加 `+26` 样式 id）与形状 3 文本之后的 `A + B` 条交出来，DTO `runs`（`serde` 空则不写，`geometry.entities` 的 golden 不受影响）。
- `DocumentStyleTable::resolve_run_style`（一跳、只认 `JStyleTextChar`，N-D4）/ `::paragraph_layout`（段落的对齐 / 行距，不经第二跳）；`text_styles_for_document` / `_for_file` → `ResolvedTextStyle { paragraph, run, runs: TextRunStatus, alignment, line_spacing }`，`effective()` = run 的字高 / 颜色 / 字体 + 段落的对齐 / 行距（N-D2）；打架时取覆盖字符最多的样式（平局取先出现的，N-D3）。`text_heights_for_document` 一字未动（N-D5：留一轮）。
- 棘轮（`style_link_ratchet` 15 → 17）：四图 177 条 `igTextBox`（形状 1 / 2 / 3 = 10 / 155 / 12），选择子 1 / 2 = 257 / 12，选择子 1 全部指 `JStyleTextChar`（08-22 在六个 fixture 上数到的 7 条异类不在这四张图里），带 run 的 167 条 run 长度和全部等于字符数；155 条索引里 `uniform` 134 / `flattened x2` 11 / 无 run 10，**会变 106（字高 103、字体 54、颜色 2）、不变 49**，字体迁移 `Arial → Arial Narrow` 40 / `Arial → 仿宋_GB2312` 3 / `Braggadocio → Arial Narrow` 11——与 08-22 探针逐数相同；段落解不出、靠 run 救回来的 0 条（0.254 mm 那几条两条路都落在哨兵上）。`3.175 mm 是最常见字高` 那条注释改成「这是段落默认」。
- 验证：`--lib` 1118、`parse_real_files` 135、`render_gap_census` 4、`geometry_profile` 2、golden 1、`semantic_join` 2、`sheet_family_wiring` 1 全绿；`clippy --all-targets -D warnings` 零告警；rustfmt 干净。
- 过程记录：同一时段另一会话在 pid-parse 提交了 `8968bd7`（`sheet_probe` 性能），T1 叠在它上面，文件不相交。

### S（OCS，本次提交）

- `src/io/pid.rs` → `src/io/pid/`：`mod.rs`（常量 / 图层分类 / `PidImport` / `load_pid` / `prepare_document` / `finish` / `ensure_layer`）、`summary.rs`（`ImportSummary` / `ImportUnit` / `report_import`）、`styles.rs`（`Styles` / `resolve_styles` / 线型 / 填充 / `apply_symbology`）、`text.rs`（字高 / 颜色 / 对齐 / 拆行 / 文字样式）、`build.rs`（`Built` / `build_document_entities` / `build_entities` / `build_inferred`）、`metadata.rs`（`attach_pid_metadata` / `PlacementMeasures`）、`page.rs`（页框 / `Projection` / `Bounds` / 取景）、`symbols.rs`（缓存 / 库本体与笔画）、`tests.rs`。
- 只搬不改：脚本按顶层条目切（条目前紧贴的注释 / 属性随条目走），挪出去的条目、结构字段与方法一律 `pub(super)`，`mod.rs` 逐个 `use self::<子模块>::*`、子模块 `use super::*`；对外的 `ImportSummary` / `ImportUnit` / `SUMMARY_PROPERTY_PREFIX` 由 `mod.rs` `pub use` 出去，`crate::io::pid::…` 路径不变；随后 rustfmt（只有换行）。
- 验证：`cargo check --lib --tests` 干净；`--lib io::pid` **54/54**、`--test pid_import` **50/50**；**四图 `--export` 与基线 SHA-256 逐一相等**（`2B1022B5…` / `340ED098…` / `763CAD1A…` / `B04C7215…`，此时 pid-parse 已含 T1——T1 是加法，不改输出）。提交 OCS `9eaf8593`。
- 顺带的可见变化：日志的 target 从 `OpenCADStudio::io::pid` 变成 `…::io::pid::summary` / `…::styles` 等子模块；`RUST_LOG=OpenCADStudio::io::pid=info` 这类按前缀的过滤照旧生效。

### T2（OCS，本次提交）

- `resolve_styles` 改吃 `text_styles_for_document`，`Styles.text_heights` 装每条的 `effective()`（run 的字高 / 颜色 / 字体 + 段落的对齐 / 行距）——下游 `height_for` / `register_text_styles` / 实体循环一行没改。`ImportSummary.lettering_flattened`（随文档属性穿管线，`counts()` 14 → 15）+ `resolve_styles` 里一行 info 日志（N-D3）。`finish` 改收 `&Styles`，免得参数过 clippy 的七个。
- **四图 `--export` 逐实体比**（脚本按实体顺序比、去掉句柄类组码——换了文字样式表的图句柄会整体挪）：**非文字实体零差异，头变量零差异**；变的只有 TEXT：0201 23 条（字高 11、字高 + 样式 12）、0202 27 条（字高 13、字高 + 样式 12、只换样式 1、颜色 1）、D06 3 条（字高 2、字高 + 样式 1）、工艺 38 条（字高 11、字高 + 样式 27）；STYLE 表 D06 少了没人再用的 `PID-Arial`，工艺的 `PID-Braggadocio` 换成 `PID-Arial-Narrow`。比探针口径（30 / 33 / 4 / 39 条索引）少，是因为导入器只重设 `role=text` 的文字，落在符号名标签上的那几条照旧不动（`lettering_names_the_typeface…` 写着的范围）；0201 那一处颜色变化就在其中。
- **新基线**（debug）：0201 `5F082D23…` 188 539 B / 0202 `6FABDF0F…` 189 298 B / D06 `907EB0A9…` 90 714 B / 工艺 `9D0A54BD…` 319 848 B。日志：0202 压平 1 条、工艺 10 条；字高回退 0201 1 / 0202 4 / 工艺 16 条（没变——那些记录两条路都落在 0.254 mm 哨兵上）。
- `pid_import` 50 → **51**：`lettering_carries_the_height…`（0201：3.175 ×30 → ×21，2.469 ×15，另有 1.588 / 2.293 / 3.528 三个半磅值）与 `lettering_names_the_typeface…`（Arial 21 / Arial Narrow 8 → 9 / 20）按 run 重钉；颜色、对齐两条原样通过（对齐仍取段落）；新增 `a_label_letters_in_its_own_run_and_a_mixed_one_in_its_widest`（0201 `LIA` 2.469 mm、`PID-Arial-Narrow`、压平 0；工艺压平 10，管道号 `250-LNG-57602` 2.822 mm、`PID-Arial-Narrow`）。`--lib io::pid` **54/54**（往返单测多带 `lettering_flattened`）；clippy 在 `io::pid` 与 `pid_import` 零告警；rustfmt 干净。user-guide `.pid` 一节加「文字」一段。
- 提交 OCS `fde369e8`。
- 过程事故：第二遍 `pid_import` 编进了另一会话当时在 pid-parse `sheet_probe.rs` 上的临时改动（11:53 写入、随后还原成 HEAD），连通线相关三条测试红；确认 pid-parse `src` 干净后重跑全绿，前后各查一次 `git status -- src`。**OCS 按路径依赖 `../pid-parse`，别的会话正在改那棵树时本仓的验证会吃到半成品**——验证前后都看一眼 pid-parse `src` 是否干净。

### T3（部分）

- 台账已落：user-guide「文字」一段随 T2（`fde369e8`）；pid-parse `b1a4df4`——CHANGELOG 一条、08-22 分析头部标「已接线」、`task_plan.md` 指针。本单各项写了哈希。
- **还差改前 / 改后标签特写**：`--export` 只写 DWG / DXF（试过 `.svg`：`unsupported output format`），截图只能走 GUI——与 V1 一起等桌面（会在桌面上开 OCS 窗口，先问过再做）。改前的图用今天存下的基线 DXF（与 `dbf62cb5` 导入字节相同）打开即可，不必回退代码。

## 门禁记录

- 2026-09-24：用户经 zhimo「分析 … PID文件解析和显示的实现进度。并使用plannator 制定下一步的计划」→ 本单（会话 opus-5-5-1），送 Plannotator 批注。N-D1 – N-D11 等批。
- 2026-09-24：Plannotator `{"decision":"approved"}`（无批注）；用户「Plannotator 里批准了，按推荐顺序开工（T1 与 S 并行起步）」→ 十一条按推荐放行，T1 / S 开工（会话 opus-5-5-1）。
