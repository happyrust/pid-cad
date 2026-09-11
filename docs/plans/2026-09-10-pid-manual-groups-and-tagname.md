# P&ID 图纸识别 · 手动操作（移动 / 复制粘贴 / 组合 / 位号）开发计划（2026-09-10）

> 日期：2026-09-10 晚
> 状态：**Plannotator 批准**（2026-09-10 19:2x，`{"decision":"approved"}`，无批注）。带 ⭕ 的决策按推荐落笔、未经拍板；§5 八条待拍板按各自「推荐」执行，批注里划一笔即可翻案。**M0 + M1 + M2 + M3 + M4 + M5 已落地**（见各期「进度」）。
> 前置：`docs/plans/2026-09-07-pid-legend-recognition.md`（D1–D19，识别内核三期）、
> `docs/plans/2026-09-09-pid-legend-recognition-audit-and-next-steps.md`（D20–D29，W0–W4 已落地，W5 线号规则外置正在另一会话进行：工作树里 `src/io/pid_pipes.rs` / `assets/pid-legend.json` / `Cargo.toml` 有未提交改动）。
> 语料：`D:\work\plant-code\cad\0版重新处理dxf-12张`（CPECC 石楼油库 12 张 DXF）。
> 需求原话（2026-09-10）：「给我们的 dxf 文件的 P&ID 图纸识别增加手动操作的能力：移动、复制粘贴文字和几何，组合和取消组合；组合会自动加上 tagName 属性，属性数据就是组合里面的文字，这个和自动识别加 tagName 的规则一致；tagName 可以手动修改。」

## 0. 一句话

识别是自动的，用户要能**用手改**：把文字与几何移动、复制、粘贴（这些命令今天都有，不用再写），把一团笔画 + 一条位号文字**组合**成一个符号——组合自动带上 `tagName`，值 = 组内文字按**识别同一套规则**读出来的位号，可以手改；取消组合就撤销。落点：DXF **原生 GROUP 对象**的描述字段（DXF 300）记 `tagName=…`，识别把这种组当成**优先级最高的符号来源**，于是 SVG 的 `<g tagName>`、图例面板、`PIDTAG`、`--json` 全部自然继承，一处都不用另写。

## 决策记录（接 09-09 的 D29）

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| D30 | 位号存哪 | **`Group.description`**（DXF 300 / DWG 变长文本，acadrust 已读写：`section_writer.rs:6769`、`section_reader.rs:5491`、DWG `object_writer/objects.rs:1732`），写成 `key=value` 串，与 `PID_SEMANTICS` XDATA 同一套写法：`tagName=BUV-3101;tagSource=auto`。**不用组名**——组名在组字典里必须唯一，而两个符号共用一个位号是真实状态（电动阀本体 + 它的 XV 圈）；**不用组对象上的 XDATA**——acadrust `Group` 没有 `extended_data` 字段，要改上游。描述里我们只管这两个键，别的文字原样保留 | ⭕ |
| D31 | 哪些组算 P&ID 符号 | **描述里带 `tagName=` 的组**。`GROUP` 读出位号时写入；读不出的组不写标记（一团纯几何的普通 CAD 组不该被识别当符号），之后可在特性面板给它一个位号、标记随之出现。别的软件建的组（无标记）识别不碰 | ⭕ |
| D32 | 位号怎么从组内文字读——「与自动识别同一套规则」 | 抽成**一个纯函数** `pid_legend::tag_from_lettering(texts, rules)`，识别里的圈内位号那一路（`inner_tag` / `compose_tag` + `accepts_tag`）也改走它，规则只此一份：① 组内有文字合任何已知位号形状（`tag_classes[].shape`、`blocks[].tag.shape`、`circles[].tag.shape`）且长度 ≥ `tag_min_chars`（4）→ 取它，**原文照录**（`PR-0301A 1/2"` 的尾巴照旧带着，与 D17 同）；合形状的不止一条 → 按阅读序（上→下、左→右）用 ` + ` 连成一条，与阀组位号 `GV0326A + GV0326B` 同一写法（D8）；② 否则按仪表圈的合成规则：一行大写字母 + 一行数字 → `XV-3201`（`compose_tag`）；③ 否则组内**恰有一条** ≥ 4 字的文字 → 取它；④ 否则没有。范围标注（`XV-0407A～0409A`，`expand_range` 认得的）一律不算位号 | ⭕ |
| D33 | 手动组的类别 | 按证据强弱：① 成员里有块字典认识的 INSERT → 块规则的类；② 成员里有合圆规则的 CIRCLE（连同它圈内的文字）→ 圆规则的类；③ 位号合 `tag_classes` 某条形状或某块规则的 `tag.shape` → 那条的类；④ 都不合 → 新类 `manual`（规则 JSON 加 `manual_group: {class, label: "手动组合", color}`，颜色暂定浅灰 (200,200,200)）。` + ` 连成的位号按第一条定类 | ⭕ |
| D34 | 手动组与自动识别谁说了算 | **手动组赢**：成员一律从自动各遍里剔掉——INSERT 不再走块遍、CIRCLE 不再走圆遍（进 `used_circles`）、笔画不进炸开池、文字标 `taken_text`（配对拿不到它）。手动组的符号 `source = "group <组名>"`、`known = true`、`tag_distance_mm = Some(0)`；文字成员进 `tag_handles`、其余进 `handles`（`plot_groups` / `PIDTAG` 两者都取，现有代码不改） | ⭕ |
| D35 | 位号跟不跟文字变 | 两种来源：**`tagSource=auto`**（`GROUP` 默认）——有效位号在**每次识别时**从组内**当前**文字重读（D24 的过期机制已保证编辑后会重识别），复制粘贴出来的组改一下文字位号就跟着变，什么都不用做；描述里存的值只是给文件下游看的缓存，应用在碰到组的时刻刷新它（`GROUP`、`recreate_groups`（COPY / ARRAY / PASTE）、`PIDLEGEND ON`、`PIDGROUP AUTO`、文字编辑提交）。**`tagSource=manual`**（用户手改）——存的值就是真值，识别照录，不再从文字重读；手改成与自动读出一样的值 / 清空 → 退回 `auto` | ⭕ |
| D36 | 手改入口 | ① **特性面板**：选中任一成员，「P&ID」组出现可编辑行「位号 (tagName)」（`PropValue::EditText`，字段 `pid_tag`），外加只读的「来源 自动 / 手动」「组 *A1」「类型」；多选跨几个组 → `*VARIES*`；改动经 `begin_group_undo` / `commit_group_undo`，可撤销。② **命令 `PIDGROUP`**：裸 `PIDGROUP` = 把当前选择组合并自动取位号（等于 `GROUP` 但不问组名，组名用 `*A<n>`）、`PIDGROUP TAG <值>` = 给选中成员所在的组手设位号（manual）、`PIDGROUP AUTO` = 退回自动、`PIDGROUP OFF` = 取消组合（= `UNGROUP`）。`GROUP` / `UNGROUP`（功能区按钮、命令行、快捷键）本身照旧可用，行为里多了位号这一步；`PIDGROUP` 是给脚本 / 自动化 / 不想被问组名的人的 | ⭕ |
| D37 | `GROUP` 的回执 | 读出：`Group "*A1" created, tagName BUV-3101 (蝶阀).`；读不出：`Group "*A1" created, no tagName -- the group letters nothing that reads as a tag (2 texts). Set one in Properties or PIDGROUP TAG <tag>.`；` + ` 连成的照原文报 | ⭕ |
| D38 | 移动 / 复制之后覆盖层的矩形怎么办 | `PID-LEGEND-*` 的矩形是画进图的实体，不会跟符号走。**本计划不做自动重画**：D24 已把面板标「已过期」、`PIDLINE` / `PIDTAG` / 面板行用前都会重识别；矩形要重画就再 `PIDLEGEND ON`。只加一小步：`GROUP` / `UNGROUP` / `PIDGROUP` 结束时若图上有图例实体（`legend_handles` 非空）就在**同一撤销步**里重画（组合是明确的语义动作，矩形不跟着变最刺眼）；MOVE / COPY 后照旧靠 ON。每次编辑都自动 ON 会往撤销栈里塞快照、大图上每步 0.2 s + 几百个实体替换——不值 | ⭕ |
| D39 | 复制粘贴出来的位号 | `recreate_groups` 连描述一起拷（`let mut group = source`），副本位号与原件相同——这是**真实状态**（图上两只同号阀），报告照旧在 `DUPLICATE` 行列出，用户改文字（auto）或改位号（manual）解决。`auto` 组在 `recreate_groups` 里顺手重读一次（文字是拷的，值不会变，刷的是缓存）；`manual` 组的值照拷（副本是副本） | ⭕ |
| D40 | 手动组的管道端口 | 块族：成员 INSERT 的 `POINT` 端口照旧算，只是记到手动组那个符号的下标上（块遍今天算端口的那几行搬到手动组预处理里复用）；圆成员的圆周同理。炸开族：与 W7 同命（今天 SP02 一条管线 run 都没有），不在本计划 | ⭕ |
| D41 | 与 W8（识别结果写 XDATA）的关系 | W8 落地时 `attach` 给手动组符号写 `resolved=legend:group:<组名>`；`PIDLEGEND PURGE` 只清 XDATA，**永远不碰组描述**——那是用户的数据。W8 若先于本计划落地，本计划 M3 补这一行；若后于，W8 顺路写 | ⭕ |
| D42 | 排期 | **M0 → M1 → M2 → M3 → M4** 连做（M2 起用户就能用），**M5 / M6 随后**；插在 09-09 计划的 **W5 之后、W6 / W7 之前**——M3 改 `recognise_with_units`，W7 也要改它，先落地的少一次合并；W8 的 `attach` 要知道 `group` 这个来源 | ⭕ |

## 1. 现状审核（2026-09-10 实测代码）

### 1.1 已经有的（不用写）

| 能力 | 在哪 | 说明 |
|---|---|---|
| 移动 / 复制 / 阵列 | `src/modules/draw/modify/translate.rs`（MOVE）、`copy.rs`（COPY）、`src/app/commands/draw.rs:611-650` | 先选后动、有预览、有撤销 |
| 剪贴板 | `src/modules/draw/clipboard/*`：COPYCLIP / CUTCLIP / COPYBASE / PASTECLIP / PASTEORIG / PASTEBLOCK；`command_driver.rs:2745` `PasteClipboard`、`:4674` `finalize_paste` | 跨图纸粘贴带图层 / 线型 / 块定义 |
| 组 | `GROUP` / `UNGROUP`（`src/modules/draw/groups/*`，分发在 `src/app/commands/layers.rs:651-700`，`CmdResult::CreateGroup` / `DeleteGroups` 在 `command_driver.rs:2770 / 2785`）；`Scene::create_group`（`scene/group_layer.rs:23`）、`delete_groups_containing`（:154）、自动组名 `*A<n>`（`app/helpers.rs:464`） | 组是 DXF `GROUP` 对象，进 `ACAD_GROUP` 字典；`PICKSTYLE` 控制点一个选整组 |
| 复制保组 | `Scene::copy_complete_groups` / `recreate_groups`（`group_layer.rs:51 / 85`）；剪贴板在 `app/mod.rs:1534-1551` 快照整组 | 整组被拷才重建，副本组名 `NAME_COPYn`，描述连带拷走 |
| 组撤销 | `begin_group_undo` / `commit_group_undo`（`app/history.rs:573 / 588`） | 按组对象 + 字典的前后差记撤销——**改描述也自动在内** |
| 组选择 | `handles_expanded_for_selectable_groups` / `expand_selection_for_groups`（`group_layer.rs:181 / 205`） | 点一个成员选整组、悬停高亮同步 |
| 组描述字段 | acadrust `objects/group.rs` `description: String`（DXF 300）；DXF 写 `section_writer.rs:6769` / 读 `section_reader.rs:5491`；DWG 写 `object_writer/objects.rs:1732` | **OCS 今天没用过这个字段** |
| 识别索引过期 | D24：`DocumentTab.pid_legend` 记 `geometry_epoch`，`refresh_pid_legend`（`commands/pidlegend.rs:215`） | 任何编辑之后 `PIDLINE` / `PIDTAG` / 面板行 / LIST 用前都重识别（≤ 0.2 s） |
| 位号组的下游 | `plot_groups`（`io/pid_legend/groups.rs:25`）→ SVG `<g tagName>`；`PIDTAG` / `pid_group_select`（`commands/pidlegend.rs:441 / 501`）；面板 `ui/window/pid_legend_list.rs`；`dxf_legend --json` | **全部只读 `Recognition`**——手动组进了 `Recognition` 就全有了 |

### 1.2 识别里「位号规则」今天长什么样（D32 要抽的那一份）

| 规则 | 在哪 | 内容 |
|---|---|---|
| 长度门槛 | `Rules::accepts_tag`（`rules.rs:393`），`tag_min_chars` 默认 4（:318） | 少于 4 字的不是位号（`S` / `K`），提交 `972dc351` |
| 形状 | `shape_matches`（`rules.rs:399`）：`9` 数字、`?` 大写、`*` 其后任意 | `BUV-9999`、`BV9999*`、`*DWG-*`… 来自 `blocks[].tag.shape` / `tag_classes[].shape` / `circles[].tag.shape` |
| 圈内合成 | `compose_tag` / `inner_tag`（`blocks.rs:156 / 174`） | 1–5 个大写字母的一行 + 3–5 位数字（可带一个大写后缀）的一行 → `HS-0320A`；不合就原文空格相连（`S`、`E H`） |
| 原文照录 | 配对遍 `mod.rs:580-594` 直接 `lettering[key].value.clone()` | 位号带图上原文的尾巴（D17：` 1/2"`） |
| 阀组多位号 | 炸开遍 | ` + ` 相连（`GV0326A + GV0326B`） |
| 范围标注 | `expand_range`（`mod.rs`） | `～` / `~` / `/` 展开成员，不是位号 |

### 1.3 缺口

| # | 缺口 | 位置 | 影响 |
|---|---|---|---|
| G1 | 组没有位号：`GROUP` 只问组名，描述字段空着 | `command_driver.rs:2770` | 用户组合了一个符号，图上、SVG 里、面板里都不知道它是谁 |
| G2 | 识别没有人工覆盖口：配错 / 漏认的符号没有任何办法改 | `recognise_with_units`（`mod.rs:266-701`）不读 `doc.objects` 里的组 | D10 / D16 / D17 那类问题今天只能改规则 JSON |
| G3 | 特性面板「组」行只读、「P&ID」组只认 `.pid` 导入的 XDATA | `app/properties.rs:2103-2110`、`scene/cache/properties.rs:120` | 位号看不到、改不了 |
| G4 | MOVE / COPY 之后覆盖层矩形留在原地 | 设计如此（D2 画的是真实体） | D24 只标过期，不重画 |
| G5 | 复制出来的组带同一个位号，改了文字也没有东西跟着变 | `recreate_groups` 拷描述 | D35 的 `auto` 来源解决 |
| G6 | `PICKSTYLE` 组选择开着时，点一个成员选整组——要单独改组里那条文字得先 `PICKSTYLE 0` 或 Ctrl 选 | 现有行为 | 体验问题，文档里写清；不改 |

## 2. 分期

原则：一期一提交；`--test pid_legend`（今天 24 条）与 lib 侧 P&ID 过滤（`pid_legend pid_pipes pidlegend pidline pid_legend_list`）每期都绿；**12 张真图上一个 GROUP 对象都没有（2026-09-10 直查 DXF：12 张 `^GROUP$` 命中全为 0），所以 M1–M4 的 `dxf_legend` 报告与基线必须逐字相同**——任何一行变了就是回归。

### M0 · 取证与基线（小，半天）

- ~~核实 12 张 DXF 的 `objects` 里 `ObjectType::Group` 数量~~ 已查：全 0（§2 原则那条）。
- `Group.description` 往返：内存里建一个带 `tagName=BUV-3101;tagSource=auto` 的组，写 DXF 与 DWG 各一份再读回，描述逐字相同（DWG 那条路代码在、没实测过）；带中文与 `;` `=` 的描述也走一遍，定转义规则（推荐：值里不许出现 `;`，出现就整段按 `manual` 存进去时替换成全角 `；`——⭕）。
- 12 张 `dxf_legend` 报告存为本计划基线（W5 若已改基线就用 W5 之后的）。
- 出口：一条 lib 单测（往返）；基线文件在 `%TEMP%\pid-manual-groups\baseline\`。
- **进度（2026-09-10 19:4x，会话 fable-5-1-38）✅ 已落地**。① 往返：`tags::tests::a_group_description_survives_dxf_and_dwg_round_trips`——带 `tagName=BUV-3101;tagSource=manual;备注=阀门 A=B` 的组经 `io::save_to_bytes` → `io::load_bytes` DXF / DWG 各一遍，描述逐字相同、两个成员句柄都解析得到、`GroupTag::parse` 读回 manual；转义按推荐：值里的 `;` 写入时换成全角 `；`（`GroupTag::clean`）。② **顺带发现 C14**：acadrust 的 DXF 读取器把 GROUP 对象建成 `Group::new("")`——**组名为空**，名字只在 `ACAD_GROUP` 字典的键上（DWG 那条路 `dwg_document_builder.rs:1783-1789` 会从字典回填，DXF 那条不会）。今天 OCS 开 DXF 后特性面板「组」行、`unique_group_copy_name` 用的都是这个空名。本计划不依赖组名（D30），测试改按标记找组、名字只断言「字典键或 `name` 二者之一为 `VALVE-1`」；修法（开图后按字典回填 `name`，`io/mod.rs` 的 post-load fixups 里几行）列入 §5-9 待拍板。③ 基线：HEAD `19656e4a` 在旁路 worktree 里编出 `dxf_legend`（`%TEMP%\dxf_legend-baseline-19656e4a.exe`），12 张 `--verbose` 报告存 `%TEMP%\pid-manual-groups\baseline\`（哈希拼接前 16 位 `C1F38D25C1169F58`），12 张都读得到、无锁。

### M1 · 位号规则单点化 + 组描述读写（小–中，1 天；D30 / D32 / D33）

- `src/io/pid_legend/tags.rs`（新）：`tag_from_lettering(texts: &[TextAt], rules) -> Option<TagRead { value, how: Shape | Bubble | Single }>`（D32 四步）、`class_for_tag(tag, rules) -> Option<(class, label, color)>`（D33 ③）、`GroupTag { name: Option<String>, source: Auto | Manual }` 的 `parse(description)` / `write_into(description)`（只改自己两个键、别的文字原样保留）。
- `recognise_with_units` 里圈内位号那一路（`mod.rs:511-538`）改调 `tag_from_lettering`，行为不变（有真图 + 内存图断言护住）。
- `assets/pid-legend.json` 加 `manual_group`（D33 ④）——**这份文件 W5 正在改**，等 W5 提交后再动，或先拿写锁。
- `Scene`：`group_tag(gh)`、`set_group_tag(gh, GroupTag)`、`derive_group_tag(gh, rules) -> Option<String>`（收集成员里的 TEXT / MTEXT，MTEXT 剥 `\P`，与 `lettering_of` 同一读法）。
- 出口：单测——四步各一例、` + ` 连接与阅读序、范围标注拒收、`tag_min_chars`、描述解析 / 写回往返（含外来文字保留）；圈内位号改道后 `--test pid_legend` 23/23 与 12 张报告逐字不变。
- **进度（2026-09-10 19:5x，会话 fable-5-1-38）✅ 已落地**。`src/io/pid_legend/tags.rs`（新）：`tag_from_lettering(texts, shapes) -> Option<TagRead{value, how: Shape|Bubble|Single|Joined}>`——形状命中（多条 ` + ` 相连、按传入顺序）→ `compose_tag` 合成 → 单条 → 多条空格相连（`Joined`，就是原 `inner_tag` 的兜底，为的是圈内 `E H` 这类「有字不是位号」的信号不变）；`class_for_tag`（`tag_classes` → 块规则 → 形状字典 → 圆规则，` + ` 取首条，`ignore` 类跳过）；`GroupTag{name: Option, source: Auto|Manual}` 的 `parse` / `write_into` / `strip`（只动 `tagName` / `tagSource` 两个键，外来片段原位保留，重复键只留第一处，无 `tagSource` 读作 auto，大小写不敏感）；`derive_group_tag(doc, group, rules)`——先剔范围标注，形状 / 合成直接要，否则**恰有一条** ≥ `tag_min_chars` 的文字才算（`NO` 旁边一条 `ANY-THING` 仍读出，`DN100` + `1.6MPa` 读不出），最后再过一遍 `accepts_tag`（`XV` + `3201` 要先合成再量长度——第一版先量长度把 `XV` 滤掉了，测试抓住）。`blocks.rs`：抽 `lettering_value(entity)`（TEXT 对齐点优先 / MTEXT `\P` 换空格 / 空文字与非文字为 None），`lettering_of` 与组读字共用；删 `inner_tag`。`rules.rs`：`Rules::tag_shapes()`（四表去重）、`ManualGroupRule` 默认 `manual` / 手动组合 / (200,200,200)，`Rules.manual_group` 带 serde 默认——**JSON 一行没加也不用加**，W5 已提交（`19656e4a`）但没必要碰它。`mod.rs`：圈内位号那一路改调 `tag_from_lettering`（带 `shape` 的规则只收 `Shape` 读法，与原 `find` 语义一致）；`pub use` 新 API。`scene/group_layer.rs`：`groups_containing` / `group_tag` / `set_group_tag`（返回是否改动，不自带撤销——调用方包 `begin_group_undo` / `commit_group_undo`）/ `derive_group_tag` / `tagged_groups`。**验证**：lib 侧 `tags` 6/6（读法四路、类别、形状表、描述往返、组读字、DXF / DWG 往返）+ `blocks` 新增 `an_entity_reads_as_lettering_or_not` + `group_layer::a_group_reads_carries_and_drops_its_pid_tag`；`--test pid_legend` **24/24**（W2 之后已是 24 条，不是 23）、lib 侧 P&ID 过滤 42 过 / 2 ignore；**12 张 `dxf_legend --verbose` 报告与基线逐字相同（0 / 12 有差）**；rustfmt：`tags.rs` / `blocks.rs` / `rules.rs` / `mod.rs` 干净，`group_layer.rs` 只剩 HEAD 就有的两处旧差；clippy 改动文件零告警；`cargo check --lib --tests` 过。

### M2 · GROUP / UNGROUP / 复制粘贴接上（中，1 天；D35 / D36 / D37 / D38 / D39）

- `CmdResult::CreateGroup`（`command_driver.rs:2770`）：`create_group` 之后 `derive_group_tag`，读出就 `set_group_tag(auto)`，回执按 D37；规则 `Rules::load()` 一次。
- `recreate_groups`（`group_layer.rs:85`）：`auto` 的组重读一次（D39）。
- `UNGROUP` 不改（描述随组消失）；W8 已落地时顺手清成员上 `resolved=legend:group` 的 XDATA。
- 新命令 `PIDGROUP [TAG <值> | AUTO | OFF]`（`src/app/commands/pidlegend.rs`，与 `PIDLEGEND` / `PIDLINE` / `PIDTAG` 同一文件）：裸 = 选择 → 组合 → 自动位号（无选择先 `SelectObjectsCommand`）；`TAG` 走 `set_group_tag(manual)`，与读出值相同 / 空 → `auto`；进 `inventory::submit!` 注册表（W0 那条测试 `pidlegend_and_pidline_are_registered_for_autocomplete` 加 `PIDGROUP`）。
- D38 那一小步：三条命令结束时 `legend_handles` 非空就在同一撤销步里重画。
- 文字编辑提交（`DDEDIT` / `TEXTEDIT` / 特性面板改 `text`）之后：成员所在 `auto` 组刷新缓存值（找到文字编辑的公共落点再定，可能就是 `apply_property_op` + 文字编辑器的提交路径两处）。
- 出口（lib 命令级测试，内存图）：① 四笔阀 + `BUV-3101` 选中 → `GROUP` 回车 → 描述 `tagName=BUV-3101;tagSource=auto`、回执含 `蝶阀`；② 纯几何 `GROUP` → 无标记、回执说没读出；③ `PIDGROUP TAG XV-0001` → manual，再 `PIDGROUP AUTO` → 回 auto；④ `COPY` 一份 → 副本组描述相同、`DUPLICATE` 行出现；⑤ 改副本文字为 `BUV-3102` → 副本有效位号跟着变、原件不变；⑥ 撤销 / 重做各一步，描述随组对象回退；⑦ 剪贴板跨图纸粘贴保留 `tagName=`。
- **进度（2026-09-10 20:2x，会话 fable-5-1-38）✅ 已落地**。`commands/pidlegend.rs`：`make_pid_group`（GROUP / PIDGROUP 共用：建组 → `derive_group_tag` 读出就写 `auto`、读不出不写标记（D31）→ 图例已画就重画 → 一个撤销步）、`dissolve_pid_groups`（UNGROUP / PIDGROUP OFF 共用，同样重画）、`set_pid_group_tag`（手设值 = 读出值 / 空 → `auto`，否则 `manual`，D35）、`refresh_auto_group_tags`（`PIDLEGEND ON` 开头刷一遍所有 `auto` 组的缓存值）、`place_pid_legend`（从 ON 里抽出的画图例那一段，重画共用）、`redraw_pid_legend_if_drawn`（D38）；撤销：图上有图例实体时走 `push_undo_snapshot`（结构 + 实体一起进一步），没有就照旧 `begin_group_undo` / `commit_group_undo`（便宜）。`command_driver.rs` 的 `CreateGroup` / `DeleteGroups` 与 `commands/layers.rs` 的 UNGROUP 直达路都改调这两个助手；新命令 **`PIDGROUP [GROUP | TAG <值> | AUTO | OFF]`**（裸打 = 关键字提示、Enter = GROUP；`GROUP` 无选择先 `SelectObjectsCommand::plain`；`TAG` 在没组的选择上**先建组再手设**；进 `inventory` 注册表与 W0 那条补全测试）。回执：`Group "*A1" created. tagName BUV-3101 (蝶阀).` / `… No tagName: the group letters nothing that reads as a tag (0 texts). Set one in Properties or with PIDGROUP TAG <tag>.` / `Group "*A1": tagName XV-0001 (set by hand).`（GROUP 那半句走原有 `tf!`，其余英文，i18n 归 M4）。**两处按原计划缩水**：① D39 的「`recreate_groups` 里 auto 组重读一次」没做——副本文字与原件逐字相同，重读结果必然相同，`recreate_groups` 照旧连描述一起拷即可（测试 ⑤ 证明 ON 会把改过文字的副本刷成 `BUV-3102`）；② 「文字编辑提交后刷缓存」没做——文字编辑的落点散在就地编辑器 / 特性面板 / DDEDIT / 自动化四处，M3 的识别本来就从当前文字重读 `auto` 组（有效位号不受缓存影响），缓存刷新点先定为 GROUP / PIDGROUP / `PIDLEGEND ON` 三处，保存前刷新仍是 §5-6 待拍板。**验证**：lib 侧新增三条命令级测试 `group_reads_its_tag_from_the_lettering_into_the_description`（①②⑥：两次 GROUP、撤销两步、重做一步描述回来）、`pidgroup_groups_tags_by_hand_reads_again_and_dissolves`（③ + 手设值等于读出值退回 auto、TAG 无值内联走提示 Enter 取消 / 整条分发被拒、没组的选择 TAG 先建组、AUTO 读不出留 `tagName=`、OFF 只解散点到的那组、裸 PIDGROUP 只提示）、`a_copied_group_keeps_its_tag_and_the_legend_follows_group_changes`（④⑤⑦ + D38：`copy_entities` 副本带描述、改副本文字后 `PIDLEGEND ON` 刷成 `BUV-3102` 原件不动、图例已画时 UNGROUP 回执 `legend redrawn`、一步撤销组与图例一起回来、COPYCLIP → `{"op":"new"}` → `PASTECLIP 0,0` 跨文档带 `tagName=`）；P&ID + group 过滤 **51 过 / 2 ignore**、`--test pid_legend` 24/24、`cargo check --lib --tests` 过；rustfmt：`pidlegend.rs` 只剩 HEAD 就有的三处旧差，`command_driver.rs` / `layers.rs` 差集合与 HEAD 相同；clippy 改动处零告警。识别内核一行未动，12 张报告不必重跑。测试里踩到一件事：测试进程的界面语言是 zh-CN，`tf!` 的 GROUP / UNGROUP / COPYCLIP 回执是中文，断言只认我自己的英文尾巴和状态。

### M3 · 识别吃手动组（中，1–1.5 天；D31 / D33 / D34 / D40）

- `recognise_with_units` 开头加**手动组预处理**：遍历 `doc.objects` 里带 `tagName=` 的 `Group`（只认模型空间成员）→ 每组一个 `Recognized`：`source = "group <名>"`、类别按 D33、bbox 为成员并集（INSERT 按块遍同一套展开、TEXT 不算）、`at` = 非文字成员 bbox 中心、文字成员进 `tag_handles`、其余进 `handles`、位号按 D35（auto 重读 / manual 照录）；成员进排除集：块遍跳过、圆遍跳过并入 `used_circles`、`exploded_symbols` 多收一个排除集（与 `used_circles` 同一传法）、文字 `taken_text = true`。
- 端口（D40）：块成员的 `POINT` / 圆成员的圆周记到手动组符号的下标。
- 报告加一行 `GROUP <n> manual symbols, <m> tagged (<k> manual tags)`；`--json` 每个符号已有 `source` 字段，值 `group <名>` 即可分辨。
- 出口：内存图——① 炸开族：一只球阀的笔画 + `BV0301` 组合 → 1 个球阀带位号、`unknown_shapes` 空、无主 0；同图另一只**没组合**的球阀照旧走自动、位号 `BV0302` 照配；② 覆盖错配：两只阀一条位号，自动配给了错的那只（构造一例），把对的那只与位号组合 → 组合的那只得位号、另一只 UNTAGGED；③ 块族：`$VALVE$00000316` INSERT + `BUV-3101` 组合（D13 那张内存管线图）→ 类别蝶阀、端口照旧、`lines` 照旧 `["80-FW"]`；④ 无位号的手动组（特性面板设标记后又清空）→ 符号在、UNTAGGED；⑤ 12 张真图报告与基线逐字相同（图上没有组）。
- **进度（2026-09-11 07:3x，会话 gpt-5.6-sol-13）✅ 已落地**。新增 `manual_groups.rs`：识别开头先取描述带 `tagName=` 的 GROUP，只收模型空间成员；`auto` 每次从当前文字重读、`manual` 照录描述；类别严格按已知 INSERT → 合规则 CIRCLE → 位号形状 → `manual_group`；非文字成员并框、文字全进 `tag_handles`，来源为 `group <组名>`（DXF 读后 `Group::name` 为空时从 `ACAD_GROUP` 字典键回填到识别来源），并保留 `GroupOrigin{name, tag_source}` 给 M4。块的 bbox / POINT / insertion / stem-end 端口抽成 `blocks::placed_block`，自动块与手动组共用；合圆规则的组内 CIRCLE 保留 rim 端口，炸开符号里的装饰圆不误当端口。组成员在后续各遍统一排除：INSERT 跳过块遍、CIRCLE 跳过圆遍、文字先标 `taken_text`、线不参与盘装框判断、`exploded_symbols` 两遍都收排除集；手动组本身先占符号下标，所以管道端口仍指向它。报告只在有组时增加 `GROUP n manual symbols, m tagged (k manual tags)`，无组真图报告不变。新增集成测试 `a_grouped_block_owns_its_tag_body_and_pipe_ports`（auto 缓存故意过期仍读当前字、DXF 空组名走字典、块不双认、2 个 POINT 与 `80-FW` 不丢、manual 照录）及 `manual_groups_override_exploded_pairing_and_leave_other_shapes_automatic`（故意把组内位号放得更靠近另一只阀，仍由组拿走；另一只 UNTAGGED、第三只继续自动；空位号组仍是显式符号）。**验证**：`cargo check --lib --tests` 过；`--test pid_legend` **26/26**；`cargo test --lib pid` **51 过 / 2 ignore**；rustfmt 改动文件干净、clippy 改动文件零告警；12 张 `dxf_legend --verbose` 与 M0 基线 **12/12 逐字节相同**。

### M4 · 特性面板与图例面板（中，1 天；D36 ①）

- `app/properties.rs`：选中成员属于带标记的组时，「P&ID」组（`pid_semantics_section` 之外新建一段，两者并存：`.pid` 导入的那段照旧只读）加可编辑「位号 (tagName)」`pid_tag`、只读「来源」「组」「类型」；多选跨组 `*VARIES*`。
- `app/update/command.rs` 属性套用分支加 `"pid_tag"`（照 `"hyperlink"` 的写法 :2859），落到 `set_group_tag`，包在 `begin_group_undo` / `commit_group_undo` 里。
- 图例面板：手动组符号行尾加 `手` 角标（tooltip `手动组合 *A1 · 位号来源 自动/手动`）；`PidLegendFilter` 不加新档。
- 新字串全部走 `t!`，en-US / zh-CN 两份 key 先建（其余 19 份跟 W9 的翻译批次）。
- 出口：命令级测试——选成员、属性 `pid_tag` 设 `XV-0001` → 描述 manual、`PIDTAG XV-0001` 能找到；清空 → auto；撤销一步回到之前；面板行角标（`view` 的纯函数测试）。
- **进度（2026-09-11 07:5x，会话 gpt-5.6-sol-13）✅ 已落地**。`manual_groups::group_details` 把有效位号、auto/manual 来源、DXF 字典组名和 M3 同一类别优先级收成一个轻量接口，特性面板不必整图重识别也不会复制规则。选中带标记组的任一成员后，现有 `P&ID` 段（含 `.pid` XDATA 时直接并存）新增可编辑 `位号 (tagName)`、只读来源 / 组 / 类型；多选同组只算一次，跨组逐字段聚合，相异值显示 `*VARIES*`。`PropGeomCommit("pid_tag")` 不走算式求值：空值或等于当前文字读值 → auto，其余 → manual；对所选成员触及的所有带标记组一次提交，包 `begin_group_undo` / `commit_group_undo`，图例已画时同一步重画。组描述变化现在推进 scene epoch；组对象撤销 / 重做把成员记作语义变更，保证已缓存的 P&ID 索引双向过期，`PIDTAG` 不会读到撤销前的位号；UNGROUP 同样显式推进 epoch。图例面板手动组行尾新增本地化 `M` / `手` 角标，tooltip 为 `Manual group *A1 · Tag source Automatic/Manual` / `手动组合 *A1 · 位号来源 自动/手动`；en-US / zh-CN 新增两条翻译。新增命令级测试覆盖单成员入口、类型/来源/组显示、manual → 清空 auto、PIDTAG、撤销/重做索引、跨两组 `*VARIES*` 与批量赋值；面板纯函数测试覆盖角标和来源提示。**验证**：`cargo check --lib --tests` 过；`cargo test --lib pid` **53 过 / 2 ignore**；`--test pid_legend` **26/26**；clippy 改动行零新增告警；12 张 `dxf_legend --verbose` 与 M0 基线 **12/12 逐字节相同**。

### M5 · 体验收尾（小，半天；可选）

- `PIDGROUP` 进功能区（P&ID 一组：PIDLEGEND / PIDTAG / PIDGROUP）与右键菜单「组合为 P&ID 符号」。
- `GROUP` 在有 `PIDLEGEND` 索引的图上，组名默认值改成位号（唯一时）而不是 `*A<n>`——⭕ 只是好看，组名仍不是数据。
- 出口：GUI 实跑一遍 §3 的手工验收。
- **进度（2026-09-11 08:1x，会话 gpt-5.6-sol-13）✅ 代码已落地**。Draw 功能区新增 `P&ID` 面板：大按钮 `P&ID 图例` 直接切换/刷新列表，紧凑按钮 `查找 P&ID 位号` 与 `P&ID 组合` 分别进入 `PIDTAG`、`PIDGROUP GROUP`；三个按钮使用现有 report / find / group 图标。视口有选择时的右键菜单在移动/复制旁新增「组合为 P&ID 符号」，直接对当前选择执行 `PIDGROUP GROUP`。普通 `GROUP` 的默认名现在只在 P&ID 索引存在且仍新鲜、所选文字能按同一规则读出位号、该位号在识别结果里恰好出现一次、组字典也尚未占名时采用位号；否则仍安全回退 `*A<n>`，描述字段仍是数据真值。`derive_handles_tag` 从 `derive_group_tag` 抽出供建组前命名共用。补齐 M4 新字串及 M5 按钮/菜单字串在 `locale_catalog` 的映射，en-US / zh-CN 均可实际命中。新增功能区三按钮测试与 GROUP 唯一位号 / 过期索引 / 已占名回退测试。**验证**：`cargo check --lib --tests` 过；`cargo test --lib pid` **55 过 / 2 ignore**；`--test pid_legend` **26/26**；clippy 改动行零新增告警；改动文件未增加 rustfmt 债；12 张 `dxf_legend --verbose` 与 M0 基线 **12/12 逐字节相同**。GUI / CUA 演示按用户要求在提交后执行。

### M6 · 文档与自动化口（小，半天）

- user-guide 一节：GROUP / UNGROUP / PIDGROUP、特性面板位号、`tagName=` 描述格式、SVG `<g tagName>` 的来源、`PICKSTYLE` 提示（G6）；README 功能表一行。
- 自动化 op：`{"op":"pid_group","what":"create"|"tag"|"auto"|"off", …}`——若 W9 的 `{"op":"pid_legend"}` 先落地就挂在它下面。
- 09-07 / 09-09 两份计划头部各加一行指向本文件。

## 3. 手工验收（GUI，M4 之后）

1. 开 FF02-06，`PIDLEGEND ON`：118 个符号。选 `BUV-3201` 那只蝶阀块 + 它的文字，`GROUP` 回车 → 回执 `tagName BUV-3201 (蝶阀)`；面板行带 `手` 角标；`PIDLEGEND ON` 再跑数字仍是 118（手动组替代了自动那一只，不多不少）。
2. `COPY` 这组到空处 → 面板 119、`DUPLICATE` 一行 `BUV-3201`；`DDEDIT` 副本文字改 `BUV-3299` → 面板副本行变 `BUV-3299`，`DUPLICATE` 消失。
3. 选副本任一成员，特性面板「位号」改 `BUV-3300` → 行变、来源「手动」；再改回 `BUV-3299` → 来源「自动」。
4. 导出 SVG（`--plot-svg` 或菜单）→ 有 `<g tagName="BUV-3299">`，成员 = 块 + 文字的路径；`UNGROUP` 副本 → 再导出没有它（副本块回到自动识别：块名认得、位号按距离配 `BUV-3299`——**结果一样**，因为这张图的自动识别本来就对；覆盖口在自动配错的图上才看得出）。
5. 开 SP02-10（炸开族），选一只字典球阀 `5311cc3f` 的四笔 + 旁边随便一条 ≥ 4 字的文字 → `GROUP` → 位号按 D32 ③ 取那条文字；再 `UNGROUP` → 回到字典球阀、无位号。
6. 保存为 DXF 与 DWG 各一份，重开 → 组、描述、位号、面板行都在。

## 4. 风险

- **W5 在另一会话进行中**（`pid_pipes.rs` / `pid-legend.json` / `Cargo.toml` 未提交）：M1 要改 `pid-legend.json`、M3 要改 `mod.rs`——开工前 `git status`，JSON 那一行等 W5 提交后再加或先拿写锁；不碰 `pid_pipes.rs`。
- **M3 改的是识别的第一步**：排除集一错，12 张报告就变——这条验收（逐字相同）是硬的；排除只在图上真有带标记的组时才有东西，真图上没有，所以基线不该动一个字。
- **炸开族里组合一部分笔画**：剩下那些笔画会成为另一个分量、id 变化、可能出现 `UNKNOWN SHAPE` 行——这是覆盖的本意，回执里如实报，不"修"。
- **`auto` 的缓存值与有效值可能短暂不一致**（改了文字、还没到任何刷新点就保存）：外部读 DXF 的看到旧值。M2 把文字编辑提交列为刷新点后窗口很小；要完全堵上就在保存前统一刷一遍（⭕，见 §5-6）。
- **描述字段被别的软件改写**：只认 `tagName=` / `tagSource=` 两个键，别的照留；别的软件把整段描述清掉 → 标记没了 → 组退成普通组，识别按自动走，不崩。
- **`PICKSTYLE`**：默认点一个成员选整组，用户要单改组内文字得 `PICKSTYLE 0` / Ctrl 选——文档写清（G6），不改默认。
- **lib 侧测试与别的会话抢可执行文件**（09-09 记过 LNK1104）：跑前看一眼 `Get-Process OpenCADStudio-*`。

## 5. 待拍板

1. **D31**：只有带 `tagName=` 的组算符号（推荐），还是 OCS 里 `GROUP` 出来的每个组都算（读不出位号也进面板当「无位号」）？后者会把纯几何的普通组也拉进 P&ID 面板。
2. **D32 ①**：多条文字都合位号形状时 ` + ` 相连（推荐，与阀组同），还是取离几何中心最近的一条、其余不管？
3. **D35**：`auto` / `manual` 两种来源（推荐）——还是只有一种「存进去就是真值」、改文字后靠用户自己再点一次「自动」？
4. **D36 ②**：新命令名 `PIDGROUP`（与 PIDLEGEND / PIDLINE / PIDTAG 同族）可以吗？还是把 `TAG` / `AUTO` 挂到 `PIDTAG` 下面（`PIDTAG SET <值>`），不加新动词？
5. **D38**：组合命令结束时若图例已画就同步重画（推荐）；MOVE / COPY 之后不自动重画，靠 `PIDLEGEND ON`——够不够？要不要一个 `PIDLEGEND AUTO ON|OFF` 开关，每次编辑后自动重画（不进撤销栈）？
6. **保存前刷新 `auto` 组的缓存值**（D35 风险那条）：做（每次保存多一遍 ≤ 0.2 s 的识别）还是不做？
7. **D33 ④** `manual` 类的标签与颜色：「手动组合」浅灰 (200,200,200)，还是按位号前缀猜不出就白色与「未知块」同色？
8. **D42 排期**：插在 W5 之后、W6 / W7 之前（推荐）；还是等 W8（XDATA）一起做，位号一次写进实体？
9. **C14 组名回填**（M0 发现）：DXF 开图后 GROUP 的 `name` 为空、只在 `ACAD_GROUP` 字典键上——要不要在 `io/mod.rs` 的 post-load fixups 里按字典回填（几行，DWG 那条路 acadrust 自己已这么做）？与本计划无依赖，但特性面板「组」行与粘贴副本的 `_COPYn` 命名今天都在用这个空名。

## 6. 本轮审核的验证摘要

- 代码走读：`command_driver.rs` 的 `PasteClipboard` / `CreateGroup` / `DeleteGroups`（2745–2804）、`scene/group_layer.rs` 全文、`app/history.rs` 组撤销（560–610）、`app/mod.rs` 剪贴板组快照（1534–1551）、`app/commands/layers.rs` GROUP / UNGROUP 分发（651–700）、`app/helpers.rs:464`、`app/properties.rs`（442、2103–2110）、`app/update/command.rs` 属性套用 `hyperlink` 分支（2859）、`scene/cache/properties.rs:120`、`io/pid_legend/mod.rs` 全流程（266–701）与 `Recognized`（139–174）、`blocks.rs` `lettering_of` / `compose_tag` / `inner_tag`、`rules.rs` `TagRule` / `Rules` / `accepts_tag` / `shape_matches`、`groups.rs` `plot_groups`、`commands/pidlegend.rs` 全文、`ui/window/pid_legend_list.rs` 头部与 `rows`。
- acadrust（`D:\Rust\.cargo\git\checkouts\cadcodec-e54f29c93a89ba71\7b4c112`）：`objects/group.rs`（`description`、无 `extended_data`）、DXF 写 300 / 读 300、DWG 写变长文本——描述字段三条路都在，OCS 未用。
- 树：main @ `ed93fb1d`，未提交改动全是 W5（`pid_pipes.rs` +195/−47、`pid-legend.json` `number_pattern` / `family`、`Cargo.toml` `regex`）与 SVG 侧；本计划只新增本文件。
- 测试基数：`--test pid_legend` 24 条；lib 侧 `pidlegend` 模块 12 条（含 2 条 `#[ignore]` 真图整图）。
- 环境：plannotator 0.27.12。
