# P&ID 图纸识别 · 审核结论与四期开发计划（2026-09-09）

> 日期：2026-09-09 晚（21:20 合并版）
> 状态：**合并版 Plannotator 批准**（2026-09-09 21:14，`{"decision":"approved"}`，无批注；⭕ 均未被翻案，§4 八条待拍板按各自「建议」执行）。第一版 21:04 已批（无批注）；同一时刻另一会话写了一份姊妹计划
> `2026-09-09-pid-recognition-audit-and-next-steps.md`（也已批、无批注）。用户定：以本文件为骨架，并入姊妹计划的
> **三条**（线号规则外置 / 识别结果写进 XDATA / 拆文件），删掉姊妹文件，重新过 Plannotator。并入与未并入的清单见 §6。
> 带 ⭕ 的决策按推荐落笔、未经拍板，批注里划一笔即可翻案。
> **W0 + W1 + W2 + W3 + W4 + W5 + W6 + W7 + W8 已落地**（见各期「进度」）。
> 前置：`docs/plans/2026-09-07-pid-legend-recognition.md`（D1–D19，一期块族 / 二期炸开族 / 三期管线拓扑全部已落地）。
> 语料：`D:\work\plant-code\cad\0版重新处理dxf-12张`（CPECC 石楼油库 12 张 DXF）。
> 本文件 = 09-09 对 `io::pid_legend` / `io::pid_pipes` / `PIDLEGEND` / `PIDLINE` / 图例面板 / 规则 / 测试的审核结论 + 剩余工作的分期。只写「还没做的」与「怎么做」。
> P&ID 手动组合、取消组合、位号编辑及自动化接口见 `docs/plans/2026-09-10-pid-manual-groups-and-tagname.md`。

## 0. 一句话

识别内核三期全部到位、11 张可读的图今天实测数字与 D12–D19 记录逐字相同、`--test pid_legend` 17/17、lib 侧 20/20；
真正还欠的是**五块**：① 报告里 34 条「无主」有 25 条是噪音（范围标注 / 设备表行），真漏的只有鹤管 9 条；
② WS02 两张排水图没有验收、管线只接上 3/11；③ SP02 六张一条管线 run 都没有（D13 明写留下的），
而且它们的线号是 4 位顺序号（`100-CGA-0319-A1`）、`is_line_number` 写死 5 位一条都不认；
④ 识别结果困在 tab 内存与覆盖层里——实体上没有 XDATA、编辑器里没有导出、自动化口读不到；
⑤ 产品面——`PIDLINE` 没进自动补全、面板硬编码中文不走 `t!`、面板看不到例外、没有文档；`pid_legend.rs` 3096 行再往里加东西只会更难读。
顺序：**先小修与去噪、再纯搬家拆文件、再理规则、再补验收与 SP02 管线、数据落地与产品面随后**。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|---|---|---|
| D20 | 范围标注（`XV-0407A～0409A`、`XV-0320A/0320B`、`LA-0303～0307`）怎么处理 | **展开核对**而不是压掉：拆成成员位号，每个成员在图上有带号符号 → 这条是范围标注、单独一行报「范围标注 N 条（成员全在）」；有成员缺 → 那个成员照旧报无主——白得一次交叉验证 | ⭕ |
| D21 | 设备表行（同名位号已被某符号认领：SP02-06 `XV-0301` / `XV-0302` / `LA-0319`、SP02-05 `XV-0407C` / `0407F` / `0408E`）| D16 待议项拍板：**同名已被认领的不报无主**，归「重复位号 N 条」一行（不删、只分级）；规则开关 `orphans.claimed_elsewhere`（默认关 = 不报） | ⭕ |
| D22 | `report_orphans: false` 的类（流向箭头、橇块）还列 UNTAGGED 坐标串 | 不列坐标、只报「x/y 带号」——六张 SP02 上 126 个坐标全是内部流向箭头，没人看 | ⭕ |
| D23 | 块族外配位号仍是最近优先贪心（`recognise_with_units` 986–1044），炸开族已是最大匹配最短总距（D10） | 统一走 `match_pairs`；FF02 四张 `SHEETS` 断言护住（今天 BUV / XV 全配对，改后一字不动才算过） | ⭕ |
| D24 | `tab.pid_legend` 不随文档变化失效 | 记 `scene.geometry_epoch`；落后时面板标「已过期」、`PIDLINE` 先重识别 | ⭕ |
| D25 | 编辑器内的结构化出口 | `PIDLEGEND EXPORT <file.json\|csv>` + 自动化 op `{"op":"pid_legend"}`（JSON 结构 = `dxf_legend --json`，同一序列化器） | ⭕ |
| D26 | 拆文件（并入自姊妹计划 D8） | `pid_legend.rs` 3096 行拆成 `src/io/pid_legend/` 目录，**纯搬家**、不改一行逻辑，验收 = 12 张图报告逐字相同；排在一切加代码的期之前 | ⭕ |
| D27 | 线号规则外置（并入自姊妹计划 D3） | `is_line_number` / `line_family` 从代码搬进规则 `pipes.number_pattern`（正则）+ `pipes.family`（捕获组）；默认值 = 今天的硬编码，再加 SP02 族 4 位顺序号那一条。`regex` 已在依赖树里（`env_filter` 带进来的），加直接依赖不长树 | ⭕ |
| D28 | 识别结果落在哪（并入自姊妹计划 D2） | 复用 `.pid` 导入已有的 `PID_SEMANTICS` XDATA（`src/io/mod.rs:20`；键 `class` / `label` / `resolved`，特性面板「P&ID」组 `properties.rs:120` 已会读），**不另造一套**。`PIDLEGEND ON` = 覆盖层 + XDATA；`OFF` 只清覆盖层；新增 `PIDLEGEND PURGE` 连 XDATA 一起清 | ⭕ |
| D29 | 执行顺序 | W0 → W1 → W2 → W3 → W4 → W5 → W6 → W7 → W8 → W9；W10 等外部输入随时插入 | ⭕ |

## 1. 审核结论（2026-09-09 实测）

### 1.1 代码与树

| 项 | 状态 |
|---|---|
| 树 | main @ `b45d4065`（审核时；此后别的会话已推进到 `2ee7a60f`，都是 SVG 侧），PID 七个文件全部已提交、无未提交改动 |
| 体量 | `io/pid_legend.rs` 3096 行、`io/pid_pipes.rs` 694、`commands/pidlegend.rs` 557、`ui/window/pid_legend_list.rs` 348、`assets/pid-legend.json` 125、`examples/dxf_legend.rs` 221、`tests/pid_legend.rs` 1914 |
| 规则 | 块字典 15 条 + 图层前缀 4 + 圆规则 6 + 盘装 1 + 位号类 11 + 形状字典 49 条（含 12 条 `ignore`）+ 管线 4 参数 |
| 测试 | `cargo test --test pid_legend` **17 / 17**（0.42 s）；lib 侧 `cargo test --lib -- pid_legend pid_pipes pidlegend pidline` **20 / 20**（含 `pidlegend` 4 条命令级：ON/OFF/LIST/PIDLINE；第一次跑撞上另一会话占着 lib-test 可执行文件 LNK1104，等它结束后复跑绿） |
| 耗时 | `dxf_legend` 11 张全跑，每张 ≤ 0.2 s（含 DXF 解析）——识别不是 UI 阻塞点，D18 的 10 s 在 wires 细分 |

### 1.2 11 张图今天的数字（`dxf_legend`，第 12 张 FF02-05 审核时被 OCS 窗口 pid 66696 字节锁着：`os error 33`；2026-09-10 00:3x 用户关掉那张图后补验，见表内一行）

| 图 | 符号 | 带位号 | 无主 | 管线 run / 端口 / 断头 | 与 D12–D19 记录 |
|---|---:|---:|---:|---|---|
| FF02-04 | 108 | — | 0 | 119 / 112 ÷ 153 / 6 | 一字不动 |
| FF02-05（00:3x 补验） | 145 | 全部（BUV 24/24、XV 11/11、连接符 8/8） | 0 | 174 / 156 ÷ 201 / 6（84 段无线号，1214 mm） | 一字不动；`OCS_PID_SHEETS_REQUIRED=1` 下 `--test pid_legend` 21/21（FF02 套 ran 4/4）、lib 侧 24/24 |
| FF02-06 | 118 | — | 0 | 118 / 120 ÷ 167 / 8 | 一字不动 |
| FF02-07 | 155 | — | 0 | 192 / 178 ÷ 218 / 8 | 一字不动 |
| SP02-05 | 254 | 全部 | 11 | **无** | 一字不动 |
| SP02-06 | 131 | 62 | 5 | 无 | 一字不动（D16 后） |
| SP02-07 | 217 | — | 6 | 无 | 一字不动 |
| SP02-08 | 227 | — | 5 | 无 | 一字不动 |
| SP02-09 | 178 | — | 4 | 无 | 一字不动 |
| SP02-10 | 118 | — | 3 | 无 | 一字不动 |
| **WS02-05** | 11 | 0 | 0 | 17 / **3 ÷ 11** / **31** | **不在任何测试里** |
| **WS02-06** | 24 | 0 | 0 | 24 / 22 ÷ 33 / 14 | **不在任何测试里** |

### 1.3 34 条「无主」逐条分类（六张 SP02）

| 类 | 条数 | 例 | 性质 |
|---|---:|---|---|
| 范围标注（含 `～` 或 `/`） | **19** | `XV-0407A～0409A`、`XV-0320A/0320B`、`LA-0308～0313` | 不是位号；成员都已在图上带号（抽查 SP02-05 全部成员在 evalve 27/27 里） |
| 同名已认领（设备表行 / 联锁表） | **6** | SP02-06 `XV-0301` `XV-0302` `LA-0319`；SP02-05 `XV-0407C` `XV-0407F` `XV-0408E` | D16 点名的表行噪音 |
| 单号 `LA-` | **9** | 06 `LA-0302`；07 `LA-0304` `LA-0306`；08 `LA-0310` `LA-0313`；09 `LA-0316` `LA-0318`；10 `LA-0326` `LA-0327` | **真候选**：06–09 恰好各有 1 / 2 / 2 / 2 只鹤管块 UNTAGGED（共 7）与之对应；SP02-10 报告里**没有**鹤管块（`11111` 不在），那 2 条另查 |

也就是说：**真漏的只有鹤管这 9 条**，其余 25 条是报告口径问题。

### 1.4 代码审核发现

| # | 发现 | 位置 | 影响 |
|---|---|---|---|
| C1 | `PIDLINE` 没进自动补全注册表——`inventory::submit!(CommandRegistration{names:[…]})` 里只有 `"PIDLEGEND"` | `src/app/commands/mod.rs:641-642` | 命令行打 `PIDL` 补不出 `PIDLINE`；D19 加命令时漏了这一步（分发正常，测试过） |
| C2 | 面板全部字串硬编码中文（`P&ID 图例` / `先运行 PIDLEGEND ON…` / `符号 N（带位号 M）` / `管线 N 条 · N 族` / `（未编号）` / `Auto` / `Close`），不走 `t!`；21 份 Fluent 目录零条目；命令行回执是英文 | `src/ui/window/pid_legend_list.rs` 全文 | 一个面板两种语言；英文界面用户看到中文面板 |
| C3 | 面板没有「例外」区：`unknown_blocks` / `unknown_shapes` / `orphan_tags` 只进命令行报告 | 同上 | 审图最需要的「漏了什么」在面板上看不到 |
| C4 | `tab.pid_legend` 只在命令里写入、从不失效；编辑 / 删除 / 撤销后面板行的 bbox 与 `PIDLINE` 用的 `run.handles` 是旧的 | `src/app/document.rs:221`、`pidlegend.rs` | `pid_line_select` 会 `replace_selection` 一批可能已不存在的句柄；低危但真 |
| C5 | 块族外配位号是「按距离排序 → 贪心一对一」，与炸开族的最大匹配最短总距（D10）不一致 | `pid_legend.rs:986-1044` | FF02 上今天全对；一排等距块阀配一排偏置位号时会犯 D10 那个整排错位 |
| C6 | 识别结果困在内存里：`Recognition` 只挂在 tab（`document.rs:221`）+ 覆盖层实体；实体 XDATA 上什么都没写——而 `.pid` 导入的实体特性面板有「P&ID」组（`properties.rs:120`，读 `class` / `label` / `resolved`），DXF 识别出来的阀点开一片空白；`PIDLEGEND ON` 置脏，Ctrl+S 把 `PID-LEGEND-*` / `PID-PIPE-*` 层**写进原图**；`--json` 只有无头 `dxf_legend`；自动化 / MCP `ocs_read` 只有 `entities/query/records/layers/header`、`doc_api.rs` 0 处引用 `pid_legend` | `pidlegend.rs`、`automation.rs:972-1014`、`doc_api.rs` | 下游（plant 数据管线、MCP 代理）拿不到位号 / 线号表；特性面板对识别结果一无所知 |
| C7 | `tests/pid_legend.rs` 真图被锁时 `eprintln!("skipping …")` 后软跳过；cargo 成功时吞掉 stderr | `tests/pid_legend.rs:1303-1313` | 今天 FF02-05 就被锁着，17/17 绿但少一张，肉眼看不出 |
| C8 | README / user-guide 对 `PIDLEGEND` / `PIDLINE` / 规则文件 / `OCS_PID_LEGEND_RULES` 零字；唯一文档是 09-07 计划与模块 doc | `README.md`、`docs/user-guide.md` | 功能不可发现 |
| C9 | `report_orphans: false` 的类照旧列 UNTAGGED 坐标：流向箭头 SP02-05/06/07/08/09/10 = 13 / 28 / 30 / 27 / 20 / 8 个坐标 | `pid_legend.rs:2625-2632` | 报告里最长的六行全是噪音 |
| C10 | WS02 族块字典三条低可靠（`$VALVE$00000315` 排水阀、`$TwtSys$00000147` 排水节点、`RC`），两张图的图幅连接符 0/6 带号（旁边是「去往 / 来自 …」不是 DWG 号） | `assets/pid-legend.json`、09-07 §1 表 | 两张图识别出来的东西没人验过 |
| C11 | **线号格式写死**：`is_line_number` 要求 `<size>-<service>[-5 位-类别]`，`line_family` 同样写死。SP02-06 层 `0管线编号` 上 5 个线号 `100-CGA-0319-A1` / `100-QGA-0303-A1` / `25-LD-0314-B1` / `250-CGA-0311-A1` / `300-QGA-0312-A1`（DXF 直查证实）是 **4 位**顺序号，今天一条都不算线号 | `pid_pipes.rs:178-215` | SP02 管线拓扑（W7）没有线号可挂；姊妹计划 F2 发现 |
| C12 | **单文件 3096 行**：规则、块、圆、炸开、配对、报告、覆盖层全在一个文件，`exploded_symbols` 380 行一口气 | `pid_legend.rs` 顶层函数约 50 个 | 再往里加 SP02 管线 / XDATA 只会更难读；姊妹计划 F8 发现 |

### 1.5 三期留下的、09-07 文档自己点名的

- **SP02 族管线拓扑**：D13 原文「SP02 族的管子（层 `0` 上的 LWPOLYLINE、无 `POINT`）不在此条范围」——六张图 `PIPE` 行为空、符号 `lines` 全空、`PIDLINE` 在 SP02 上无事可做。
- **图例背书**：12 张都没有图例张，`SPE-0100FF01-01` 说明书未提供；块含义表里 5 条「中等 / 低」（`$VALVE$00000501` 疑水流指示器、`$Attachment$00000194` 疑泡沫接口、`$VALVE$00000315`、`$TwtSys$00000147`、`RC`）。
- **鹤管 LA 旧账**（D11 原话「不在本条范围」）：见 §1.3 第三行。
- **FF02-04 字节锁**：09-07 已用内存映射绕过并补验；今天换成 FF02-05 被锁（用户正开着它编辑，autosave 20:43 还在写）。**不碰这个文件**；测试照旧跳过。

不做的清单沿用 09-07：真正没有轴向线可切的阀组（语料里一个都没有）、「按位号数量再分」。

## 2. 分期

原则：一期一提交；`--test pid_legend` 17 条与 lib 侧 20 条（`pid_legend pid_pipes pidlegend pidline` 过滤）每期都绿；改动识别口径的期，11 张图 `dxf_legend` 报告前后 diff **只允许动本期点名的行**。

### W0 · 小修与台账（小，半天）

- C1：`PIDLINE` 进 `inventory::submit!` 名单（一行 + 现有自动补全测试若有则加断言）。
- C7：真图软跳过改成可见——测试末尾 `eprintln!` 汇总「N 张跳过：…」，并加环境变量 `OCS_PID_SHEETS_REQUIRED=1` 时跳过即失败（收尾验证 / CI 用；默认不设，编辑器开着图也能跑）。
- 09-07 计划头部加一行「D19 后审核与四期计划见本文件」。
- 出口：`PIDL` 补全出两条；带 `OCS_PID_SHEETS_REQUIRED=1` 且 FF02-05 被锁时测试红、不设时绿并打印跳过清单。
- **进度（2026-09-09 21:5x，会话 fable-5-1-5）✅ 已落地，提交 `e60ad2c0`**：① `PIDLINE` 进注册表，新单测 `pidlegend_and_pidline_are_registered_for_autocomplete`；② 三个多张真图测试末尾打印 `ran N/M sheets[, skipped K: …]`，`load_sheet` / `open_sheet` 的跳过都经同一开关；③ 09-07 头部加了指向本文件的一行。验证：默认 `--test pid_legend` 17/17，末尾一行 `cpecc_sheets_every_symbol…: ran 3/4 sheets, skipped 1: DWG-0100FF02-05 …`（FF02-05 仍被 OCS pid 66696 锁着，`os error 33`）；`OCS_PID_SHEETS_REQUIRED=1` 同一张图让该测试红（16 过 1 败，panic 原话带锁错误与开关名）；lib 侧 `pidlegend pidline` 5/5；rustfmt / clippy 在改动处无新告警。顺带一记：内存映射读被锁 DXF 的实现（`io::read_dxf_path` 遇 `ERROR_LOCK_VIOLATION` 回落 `memmap2`）曾做出来并在锁着的 FF02-05 上读出 145 个符号、`--test pid_legend` 4/4，按本文件 §6「未并入」的决定已回退，补丁存 `%TEMP%\pid-w1-mmap-locked-dxf-read.patch`（3.5 KB），要的话一句话拉回来。

### W1 · 报告去噪（小，半天；D20 / D21 / D22）

- 范围标注展开：`～`、`/`、`~` 三种写法拆成员（`XV-0407A～0409A` → A 系列从 0407 到 0409；`XV-0407D/0407E` → 两条；`LA-0308～0313` → 六条）；成员全在带号符号里 → 归「范围标注」一行；有成员缺 → 那个成员单独报无主。认不出的分隔符照旧 ORPHAN。
- 同名已认领 → 「重复位号」一行（`orphans.claimed_elsewhere: false` 默认不报，开了才列）。
- `report_orphans: false` 的类不列 UNTAGGED 坐标（C9）。
- 出口：六张 SP02 的 ORPHAN 行只剩鹤管 9 条，被分流的每一条都出现在「范围标注」/「重复位号」行里（**总数守恒：34 条一条不少**）；`dxf_legend` 报告 diff 只动 ORPHAN / UNTAGGED / 新增两行；17 条测试不动；内存图测试加一条（一条范围标注 + 一条表行 + 一个真无主）。
- **进度（2026-09-09 22:3x，会话 fable-5-1-5）✅ 已落地，提交 `c5492f3c`**：`expand_range`（`～` / `~` 跨号或跨字母、`/` 枚举，后项继承前缀，> 50 个成员或跨号又跨字母不算）+ `Recognition.range_annotations` / `duplicate_tags`；范围按「每个成员都合类别形状」归类（`BUV-3101/3102` 本身不合 `BUV-9999`）；`TagRule.report_untagged`（缺省 = `report_orphans`，图幅连接符显式 `true` 保住 FF02-04 / WS02 那几条坐标）；`orphans.claimed_elsewhere` 进规则 JSON；`dxf_legend --json` 多两个字段。报告新行 `RANGE <类> annotations, k/n with every member on a symbol: …(missing …)` 与 `DUPLICATE <类> tags (a symbol already carries them): …`。验证：11 张 `dxf_legend` 报告与 20:54 基线 diff **只有** 6 张 SP02 的 ORPHAN / 流向箭头 UNTAGGED 行与新增 RANGE / DUPLICATE 行，六张 ORPHAN = 06 `LA-0302`、07 `LA-0304, LA-0306`、08 `LA-0310, LA-0313`、09 `LA-0316, LA-0318`、10 `LA-0326, LA-0327`、05 无（19 范围 / 6 重复 / 9 无主，与 §1.3 逐条相同）；FF02 / WS02 一字不动；`--test pid_legend` 19/19（新增内存图 `unclaimed_lettering_is_sorted_into_ranges_duplicates_and_orphans` 与真图 `cpecc_unclaimed_lettering_on_the_loading_islands_is_sorted` 6/6 张）、lib 侧 22/22（新增 `a_range_annotation_expands_to_the_tags_it_names`）；rustfmt 干净、clippy 改动处无告警。
- **顺带发现 C13（2026-09-10 00:4x 已修，提交 `56ef2c69`，会话 fable-5-1-20）**：根因是 `components()` 的并查集按 HashMap 迭代顺序合并、再按根下标排组；现在按每组**首笔画下标**排（= 图纸自身顺序）。12 张连跑三次报告逐字节相同，与修前只差三行 UNTAGGED 坐标的次序、无一处计数或配对变动；新增真图测试 `cpecc_symbols_come_in_the_same_order_every_run`（SP02-10 连读 4 次）。原记录：炸开族符号在 `symbols` 里的**顺序每次运行都不同**——同一张 SP02-10 连跑 4 次，球阀 UNTAGGED 坐标串 4 种顺序（`exploded_symbols` 里有 HashMap 迭代）。数字与配对不受影响，但面板行序每次 `PIDLEGEND ON` 都会跳、`End::Symbol(i)` 的下标不稳定、W4「报告逐字相同」的验收会被它搅浑。建议在 W4 开工前先把分量按（首笔画句柄或中心坐标）排稳，作为 W4 的前置一小步；这一改在最近优先打平手时可能翻一两个位号，要拿 11 张图 diff 兜住。

### W2 · 鹤管 LA 位号归位（小–中，半天–1 天；取证先行）

- 查 07 的 `LA-0304` / `LA-0306` ↔ UNTAGGED 鹤管 (136, 268) / (279, 268)：是超出 25 mm、还是块 body 中心（30×10 mm 块、基点远离本体）算歪、还是被 `LA-0303～0307` 那条范围标注（W1 后已不在候选里）抢走。用 `dxf_legend --json` 的 `orphan_tags` + 符号 `at` 直接量。
- SP02-10 没有 `11111` 块却有 `LA-0326` / `LA-0327`：看图上鹤管怎么画的（炸开？别的块名？）；若是炸开形状，走字典 + `tag: {shape: "LA-9999*"}`。
- 出口：7 只 UNTAGGED 鹤管里能配的配上、配不上的每只写明原因；SP02-10 那两条查明去向；`cpecc_exploded_sheets_*` 加 LA 断言。
- **进度（2026-09-09 23:2x，会话 fable-5-1-16）✅ 已落地，提交 `68394097`**（会话 fable-5-1-20 接手后复验再提交：`--test pid_legend` 21/21、lib 侧 22/22、rustfmt / clippy 干净、11 张 `dxf_legend` 与 W1 后基线 diff 只有下述鹤管行、与 23:06 那份 W2 报告只差 C13 的球阀坐标顺序）。**取证**：① 07 的两只 UNTAGGED 不是超 25 mm、不是 body 中心算歪、也不是被范围标注抢走——是 **C5 的贪心**：六张岛图的鹤管一列 9.5 mm 一只，位号写在**自己那只的 body 下方**、左端对齐 body 左缘，从 body 中心量，每条位号离**下面那只**更近（13 mm 对 14 mm），最近优先把整列往下错一只：顶上那只没号、最底那条位号（离唯一够得着的那只 23 mm）成无主。06 / 08 / 09 同一机理（06 的 LA-0319 原先配在中间那只上也是错的）。② SP02-10 的 LA-0326 / 0327 各在 BV0326 / BV0327 正上方：卸车鹤管**不是 `11111` 块**，是层 `1` 上一条 10 顶点曲线加层 `0` 上一只 3.9 × 1.4 mm 的托架（三条线）——托架横杠 3.9 mm 与内杠 2.7 mm 都过 `pipe_stub_mm`，第一遍剩下曲线与两条 1.4 mm 竖线互不相触、各成单笔画组件被丢；第二遍托架横杠被两竖线加曲线端点三处夹住算 framed，但组件只含 1 条这样的 run，`recover_min_runs: 2` 挡掉。**改法**：D23 提前——块族 / 圆族的外配位号（含 `bubble` 规则）改走 `match_pairs`（最多配对、再最短总距，与炸开族同一函数；文字与气泡编进同一索引空间），`recognise_with_units` 里少 10 行；规则 `recover_min_runs` 2 → 1，字典加 `b6f54271`（鹤管总成，`tag.shape: LA-9999*`）。**验证**：11 张 `dxf_legend` 与 W1 后基线 diff **只有**五张岛图的鹤管行——06 3/3、07 7/7、08 8/8、09 7/7（`at` 13.0..23.1 mm）、10 新增 x2 2/2（8.2..9.2 mm，第二遍）、SP02-10 符号 118 → 120、五条 ORPHAN 行消失、六条 RANGE 全部 n/n——外加 C13 那条球阀坐标顺序抖动；FF02 / WS02 / SP02-05 与其余所有类别（取压球阀、电动阀 XV 气泡、蝶阀、图幅连接符、泵）一字不动，`recover_min_runs: 1` 单独跑 11 张 diff 为空。`--test pid_legend` **21/21**：新增内存图 `a_column_of_loading_arms_lettered_under_their_bodies_is_paired_arm_by_arm`（SP02-07 左列原尺寸，块 body 离基点两米）与真图 `cpecc_every_loading_arm_carries_its_own_tag`（五张、逐列自上而下的位号顺序 + 10 的 `b6f54271` 第二遍来源）；`cpecc_unclaimed_lettering_*` 改成无主 0 条、范围全齐；`cpecc_sp02_10_*` 表加 `loading-arm 2/2`；lib 侧 `builtin_rules_*` 钉住 `recover_min_runs` 与新字典条。rustfmt 干净、clippy 改动处无告警。**W3 的 C5 / D23 由此已完成**，W3 只剩 C4 / D24。

### W3 · 索引失效与配对一致性（小，半天；D23 / D24）

- C4：`Recognition` 记 `geometry_epoch`；面板落后时标题栏加「已过期」并把行变灰；`PIDLINE` / 面板族行点击前若落后先重识别（≤ 0.2 s，可以就地做）。
- C5：块族外配位号改走 `match_pairs`（源→位号→分量→汇，与 D10 同一函数）；FF02 四张 `SHEETS` 断言 + 内存块族图护住。
- 出口：单元测试「改图后面板过期、PIDLINE 重识别」；FF02 报告逐字不变。
- **进度（2026-09-09 23:4x，会话 fable-5-1-20）✅ 已落地，提交 `94301977`**（C5 / D23 已在 W2 一并完成，见上；本期只做 C4 / D24）。`DocumentTab.pid_legend` 从 `Option<Recognition>` 改成 `Option<PidLegendIndex { epoch, recognition }>`（`app/document.rs`）：`set_pid_legend` 记下当时的 `scene.geometry_epoch`，`pid_legend_is_stale` = 落后于当前 epoch——任何几何 bump 都算（图层开关也会标过期），重识别便宜、漏判贵；`PIDLEGEND ON` 在画完覆盖层**之后**盖戳，`OFF` 擦完后 `pid_legend_survives(was)` 保住原本当前的索引（覆盖层进出不算图变，识别本来就跳过 `PID-*` 层）。`refresh_pid_legend(i)`（`commands/pidlegend.rs`）：没读过或已过期就重识别，过期时命令行提示 `PIDLEGEND: the sheet changed since it was read -- read again, N symbols.`；`PIDLINE`、`pid_line_select`（面板管线行的 `PidLegendPickFamily` 也走它）、`PIDLEGEND LIST`（开面板时）都先经它。面板 `view` 多一个 `stale` 参数：标题栏加「已过期」、列表顶部一行「图已改动，列表可能不准 — PIDLEGEND LIST 重新读取」、全部行变灰（`row_text`）。`io::pid_legend::Recognition` 本身没动（epoch 是 scene 的概念，留在 tab 上）。验证：lib 侧 `pidlegend pidline pid_legend_list` **7/7**（新增 `a_changed_sheet_dates_the_index_and_it_is_read_again_before_use`：空图 LIST → 加一条线即过期 → PIDLINE / 面板 pick / LIST 各自重读、当前索引不重读；`pidlegend_on_draws_off_removes_and_on_twice_does_not_stack` 加断言 ON / 再 ON / OFF 都不让索引过期；FF02-06 / SP02-05 真图两条照常跑）；rustfmt 四个文件的格式差集合与 HEAD 相同（都是旧的）、clippy 改动处无告警（`document.rs` 三条 `large size difference` / `very complex type` 是旧的）。**W3 至此全部完成**；下一期按 D29 是 W4 拆文件，开工前先照 W1 记的 C13 把炸开族符号顺序排稳。

### W4 · 拆文件（中，半天–1 天；D26 / C12；纯搬家）

- `src/io/pid_legend/` 目录：`mod.rs`（公开面：常量、`recognise*`、`Recognition` / `Recognized` / `UnknownShape`、re-export）、`rules.rs`（`Rules` / `TagRule` / `BlockRule` / `CircleRule` / `ShapeRules` / 形状语法）、`blocks.rs`（块 + 圆两遍、`stem_end`、`lettering_of`、单位判定）、`exploded.rs`（`Prim` / `AxisRun` / 分量 / 签名 / 去重 / 剪管切桥 / 第二遍 `recovered_symbols`）、`pairing.rs`（`match_pairs` 与外部配对——W3 之后两族都走它）、`report.rs`、`legend.rs`（层 / `legend_entities` / `pipe_entities` / `apply` / `clear`）。
- **不改一行逻辑**；单元测试随函数走；公开 API 路径不变（`crate::io::pid_legend::*`）。
- 出口：`dxf_legend` 跑 11 张的报告与 §1.2 基线**逐字相同**（`diff` 为空）；17 + 20 条测试全绿；`rg 'pid_legend::' src tests examples` 的每个路径仍然解析。
- 风险：与别的会话同树并行（今天主树 SVG 侧还在改 `src/app/*`）——本期只碰 `src/io/pid_legend*`，开工前 `git status` 确认这些文件干净；排在 W7 / W8 / W9 之前，趁还没加代码。
- **进度（2026-09-10 01:0x，会话 fable-5-1-20）✅ 已落地，提交 `6ddeea4d`**。`src/io/pid_legend/`：`mod.rs` 801 行（模块文档、`mod` / `pub use`、常量、`Recognized` / `RangeAnnotation` / `UnknownShape` / `Recognition` / `Lettering`、`recognise` / `recognise_with_units`、`split_tag` / `expand_range`）、`rules.rs` 521、`blocks.rs` 238、`exploded.rs` 1494、`pairing.rs` 102（`match_pairs`）、`report.rs` 168、`legend.rs` 307。**逐行搬家**：用脚本按行号范围切，旧文件与新七个文件做逐行多重集比对，差异只有 ① 4 条 `// ── xxx ──` 分节注释换成各文件的 `//!` 一句、② `include_str!` 相对路径多一层 `../`、③ 15 处兄弟模块要用的可见性加 `pub(super)`（`sane` / `grow` / `skip_for_box` / `place` / `Segment` / `stem_end` / `lettering_of` / `compose_tag` / `inner_tag` / `Exploded` 及两字段 / `exploded_symbols` / `fnv1a` / `match_pairs`，外加 `TagRule::wants_tag` / `reports_untagged`、`ShapeRules::applies`、`Rules::layer_fallback`）、④ rustfmt 把变长的 `stem_end` 签名折行、⑤ 测试辅助 `words()` 在三个模块各留一份。单元测试随函数走（rules 3 / blocks 2 / exploded 5 / legend 2 / mod 1）。**验收**：12 张 `dxf_legend` 报告与搬家前（C13 修后）那份**逐字节相同**（哈希相等）；`OCS_PID_SHEETS_REQUIRED=1` 下 `--test pid_legend` 22/22；lib 侧 `pid_legend pid_pipes pidlegend pidline pid_legend_list` 24/24；clippy 对新文件零告警；`cargo check --lib --tests --examples` 过，外部 `crate::io::pid_legend::*` 路径全部经 `mod.rs` 的 `pub use` 解析。

### W5 · 线号规则外置（小，半天；D27 / C11）

- `pipes.number_pattern`：正则数组，按序先中先得；默认 = 今天的硬编码 `^\d+-[A-Z]{1,3}(-\d{5}-[A-Z]\d)?$` + SP02 族的 `^\d+-[A-Z]{2,3}-\d{4}-[A-Z]\d$`。`pipes.family`：命名捕获组 `service` / `seq`，缺 `seq` 自成一族（`80-FS` 照旧）。
- `regex` 加为直接依赖（已在依赖树里，`env_filter` 带进来的，不长树）；`is_line_number` / `line_family` 改成读 `PipeRules`（签名变、调用点 3 处：`trace`、`PIDLINE`、面板）。
- 出口：单测：SP02-06 那 5 个线号 `is_line_number` 为真、`100-CGA-0319-A1` 归族 `CGA-0319`、`200-FS-31001-A2` 仍归 `FS-31001`、`80-FS` 仍自成一族；11 张报告逐字不变（SP02 今天没有管段，加了线号也不会多出 LINE 行）；`a_broken_rules_override_falls_back_to_the_builtin_rules` 照旧。
- **进度（2026-09-10）✅ 已落地，提交 `19656e4a` + `a18fa009`**。`PipeRules` 从规则文件读取完整匹配的正则数组和族模板，默认同时覆盖 FF02 的 5 位顺序号、SP02 的 4 位顺序号及无顺序号短码；无效正则或模板引用不存在的捕获组会拒绝整份覆盖规则。`trace` 建族索引，`PIDLINE` 与图例面板直接查索引；SP02-06 的五条真实 4 位线号均有集成断言。

### W6 · WS02 族补齐：规则 + 验收（中，1 天；C10）

- `dxf_pid_probe --verbose` 普查两张排水图：管线层名、31 个断头各落在哪（画到井 / 池边就停是画法；离别的端点 < 1 mm 是吸附没接上；管是不是画到排水节点小圆 r 0.5 mm 的圆心而不是圆周）、三个低可靠块旁边写的什么。
- 图幅连接符的「去往 / 来自 …」文字：`tag.shape` 允许 `去往*` / `来自*`（形状语法本就按字面匹配，中文字面即可），`report_orphans: false`。
- 两张进 `SHEETS`：`Expected` 加可选字段（无蝶阀 / 电动阀的图不该被迫填 0——按现有结构填 0 即可，注释写明）。
- 出口：两张图的符号数 / run / 端口 / 断头进测试；31 + 14 个断头每个有解释；三个低可靠块的含义**要么用户确认、要么标「待图例」不改名**。
- **进度（2026-09-11）✅ 已落地**。探针逐点核对了原来的 45 个断头：WS02-05 的 31 个里有 **21 个精确落在 8 只排水节点圆周**，新增块端口 `"rim"` 后端口 **3/11 → 11/11**、断头 **31 → 10**；WS02-06 的 14 个里有 **5 个精确落在 5 只漏斗的颈部相接点**，新增 `"stem-line-end"` 后端口 **22/33 → 27/33**、断头 **14 → 9**。剩余 19 个逐坐标锁进测试：WS02-05 = 4 条 DN110 建筑支管 + 4 处处理设施边界 + 1 段 2.3 mm 独立短线的两端；WS02-06 = 2 条图外来管 + 1 处池边 + 3 台图上泵体两侧共 6 端，均不是 <1 mm 的漏吸附。`TagRule` 增加兼容原 `shape` 的备选 `shapes` 与按类 `min_chars`，六只图幅连接符全部读到 `化粪池 / 生活污水处理装置 / 去往… / …来含油污水`；FF02 原有 `*DWG-*` 结果不变。三条低可靠字典项 `$VALVE$00000315` / `$TwtSys$00000147` / `RC` 未擅自改含义，显示名统一加「待图例」。两张 WS02 已并入 `SHEETS`，专项测试同时锁定六个连接符位号和全部剩余断头。**验证**：12 张报告全跑，FF02 五项数字与文本不变、SP02 仅 `RC` 显示名按本期加后缀；lib 侧 P&ID 过滤 **57 过 / 2 ignore**；`--test pid_legend` **27/27**；`cargo check --lib --tests --examples`、rustfmt 与 IDE 诊断通过。

### W7 · SP02 族管线拓扑（大，2–3 天；D13 留下的最大功能缺口；依赖 W4、W5）

- 管段来源：炸开第一遍判为「管长轴向线」的 `SheetRun`（> `pipe_stub_mm`，在 `layers_any` 与层 `0` / `工艺外线` 上）——它们今天已经算出来、只是被丢掉；不再另扫图层。
- 端口：D9 剪管头时**记下管头贴在本体上的那一点**（今天 `trim_pipe_stubs` 只剪不记）作为符号端口；圆符号照旧圆周；电动阀本体到 M 圆的杆不是端口。
- 线号：走 W5 的规则；SP02-06 层 `0管线编号` 5 个线号挂到 5 mm 内最近的 run；SP02-05 只有 `DN200 1.6MPa` 这类规格字、没有线号，只会有 `LINE (none)`；短支管（`PR-03xxA 1/2"` 旁那一截）预计大多没号，如实归 `NONE`。
- 其余复用 `pid_pipes::trace`（吸附 / 三通 / run / 顺延 / 画层 / `PIDLINE` / 面板族行）。**风险**：SP02 管线与符号笔画同层，符号笔画混进管段——靠「已成符号的笔画不进池子」（D11 已有）+ 长度下限挡；层 `0` 上还有仪表引线与表格线，先探再定阈值。
- 先在 SP02-10（最小，118 符号）上做，再 SP02-06（有线号），逐张扩到 05；每张的 run 数 / 端口接上率进测试。
- 出口：SP02-06 报告出现 `LINE 100-CGA-0319-A1 …` 等 5 行，`PIDLINE CGA-0319` 能整族选中；每条线上的符号清单人工核一张（SP02-06）；六张 SP02 的符号 / 位号数字与基线**逐字相同**；FF02 / WS02 一字不动。探的结论若是「画法上接不上」，本期降为只列线号不连拓扑、如实记录。
- **进度（2026-09-11）✅ 已落地**。`exploded` 现在保留第一遍已经判为管长的轴向线、源句柄与图层资格；最终符号使用的笔画全部排除，剪掉的管头贴体点及外部长轴线落在符号本体上的端点成为端口。新增规则 `shapes.pipe_layers`（符号层自动包含，另含 `0` / `-0` / `工艺外线` / `T-PIPE_DIESEL O`）与只对炸开族生效的 `pipe_number_mm = 10`（SP02-06 的 `100-QGA-0303-A1` 离线 7.7 mm；FF02 / WS02 仍是 5 mm）。候选连通分量必须触达过程端口或有效线号；接到仪表圈 / 执行器圆的整条引线链排除，表格线不进图。`pid_pipes::trace_with_strokes` 与块族共用吸附、三通、run、顺延、族索引、覆盖层及 `PIDLINE`，run 保留原实体句柄。六张结果（段 → run，已接端口/端口，断头）：05 `179→226, 101/121, 48`；06 `164→216, 181/219, 59`；07 `193→231, 226/308, 72`；08 `185→223, 223/325, 73`；09 `153→181, 173/259, 62`；10 `93→113, 74/104, 58`。SP02-05 如预期全归 `LINE (none)`；SP02-06 五条线号全部出现，其中 `100-CGA-0319-A1` 连到 `CVV0319 + FA0319`、`100-QGA-0303-A1` 连前图连接符、`25-LD-0314-B1` 连后图连接符、`300-QGA-0312-A1` 连 `XV-0302` 与出图箭头；族键 `CGA-0319` 可直接供 `PIDLINE`。六图段 / run / 端口 / 断头 / 线号集合、SP02-06 人工核线与所有 run 源句柄均进测试，符号数分别保持 `254 / 131 / 217 / 227 / 178 / 120`。**验证**：lib 侧 P&ID 过滤 **58 过 / 2 ignore**；`--test pid_legend` **28/28**；12 张报告中 FF02 / WS02 逐字不动，SP02 只新增 PIPE / LINE 行；`cargo check --lib --tests --examples`、rustfmt 与 IDE 诊断通过。

### W8 · 识别结果写进实体（中，1 天；D28 / C6 前半；依赖 W4）

- `Recognized` 加 `handles: Vec<Handle>`（块 = INSERT；圆 = CIRCLE；炸开 = 分量里全部笔画；第二遍同；W7 的端口记录顺路用它）。
- `pid_legend::attach(doc, &recognition)`：每个符号的实体写 `class=<class>`、`label=<位号>`（无位号不写）、`lines=<线号,…>`、`resolved=legend:<block|circle|shape:<id>>`；每条 run 的 `handles` 写 `class=PIDPipeline`、`label=<线号>`（两号 run 写两个 `label`）。写法与 `pid.rs:877-898` 同一套 `key=value` 字符串；APPID 缺就注册（同 `pid.rs:395-398`——DWG 写出时不在 APPID 表里的 XDATA 会被静默丢掉）。
- `PIDLEGEND ON` = 覆盖层 + XDATA；`OFF` 只清覆盖层；新增 `PIDLEGEND PURGE` = `OFF` + 清 XDATA；`REPORT` / `LIST` / `--json` 不写。
- `properties.rs::pid_semantics_section` 认 `lines` 键，`resolved` 以 `legend:` 开头时「匹配方式」显示「图例识别」。
- 出口：FF02-06 `PIDLEGEND ON` 后点 BUV-3201 那个 INSERT，特性面板「P&ID」组显示 类型 蝶阀 / 位号 BUV-3201 / 管线号 100-FW / 匹配方式 图例识别；另存 DWG 再打开仍在；`PURGE` 后该组消失、实体数回到原值；命令级测试各加一条。
- 风险：XDATA 让文档「脏」——本来 ON 就置 `dirty`，无新增；两条不同线号顺延到同一 run 的写法在面板上要说清（两个 `label`，面板取第一个并标 `+1`）。
- **进度（2026-09-11）✅ 已落地**。新增 `pid_legend/xdata.rs`：`attach` 按实体句柄稳定发布 `PID_SEMANTICS`，符号写显示类型 / 位号 / 所在线号及 `legend:block|circle|shape:<id>|group:<name>`，管段写 `PIDPipeline`、每条有效线号一个 `label` 及 `legend:pipe`（这个来源键让无号管段也能被 `PURGE` 精确识别）；共享同一实体的两条 run 会合并线号。写前注册带真实句柄的 APPID，替换 / 清理时同时丢掉旧 DWG raw EED；已有 `.pid` 的 `resolved=direct|dependency:…` 身份优先，不被图例结果覆盖。`PIDLEGEND ON` 现在把 XDATA 与覆盖层放进同一撤销步，`OFF` 只关覆盖层，新增 `PURGE` 同步清两者但永不碰 GROUP 描述；最后一个符号消失时再次 ON 会清掉旧发布。GROUP / UNGROUP / PIDGROUP 在 OFF 之后仍会刷新已发布 XDATA，解组不残留 `legend:group`。特性面板读取 `lines`，两线号管段显示首条 `+1`，所有 `legend:` 来源本地化显示「图例识别」。测试覆盖 block / circle / shape / group 来源、两线号管段、`.pid` 身份不覆盖、DXF + DWG 两轮保存（含 purge 后不被 raw EED 带回）、命令 ON / OFF / PURGE / 撤销重做、隐藏覆盖层后解组、空识别清旧数据，以及 FF02-06 的 BUV-3201 类型 / 位号 / 100-FW / 匹配来源。**验证**：lib 侧 `pid` 过滤 **64 过 / 2 ignore**；`--test pid_legend` **28/28**；`cargo check --lib --tests --examples`、`git diff --check` 通过；改动行无 Clippy 告警，新文件 rustfmt 干净，其余文件未增加既有 rustfmt 债。全语种 catalog 测试仍列出 9 个仅有 en-US / zh-CN 的 P&ID 新字串（此前 M4/M5 的 8 个 + 本期 1 个），按 W9 的 19 语种翻译批次收口。

### W9 · 产品面：本地化、例外区、导出、自动化口、文档（中，1–2 天；C2 / C3 / C6 后半 / C8；D25）

- C2：面板与回执全部走 `t!`，en-US / zh-CN 两份 key 先建齐，其余 19 份**跟下一次翻译批次**（与 SVG P8.5 债2 同一批）。
- C3：面板底部加「例外」折叠区：未知块 / 未命名形状 / 无主位号 / 范围标注 / 重复位号，每行可点击跳转（无主位号跳到那段文字）。
- C6 后半：`io::pid_legend::export`——`to_json(&Recognition)`（把 `dxf_legend` 里的 `symbol_json` / `pipes_json` 搬进来，例子改调它）、`to_csv`（符号表 `class,label,tag,x_mm,y_mm,lines,source` + 管线表 `line,family,runs,length_mm,from,to`，from/to 取 run 两端的符号位号，三通 / 断头如实写 `tee` / `open`）；`PIDLEGEND EXPORT <file.json|file.csv>`（未识别先识别、`OFF` 之后照样能出）；自动化 op `{"op":"pid_legend","what":"recognise"|"report"|"export"}` 回同一 JSON（`ocs_read` 直接可用，MCP 代理不必再解析命令行）。**不改** `PIDLEGEND ON` 写进图的行为——那是 D2 定的显示方式；README 写明「保存前 `PIDLEGEND OFF`」。
- C8：README 功能表加一行 + user-guide 一节（`PIDLEGEND ON/OFF/REPORT/LIST/EXPORT/PURGE`、`PIDLINE`、层名、规则文件与 `OCS_PID_LEGEND_RULES`、`dxf_legend`）。
- **数据出口子阶段进度（2026-09-11）✅ 已落地**。新增 `pid_legend/export.rs` 作为唯一序列化器；`dxf_legend --json` 已删掉私有 `symbol_json` / `pipes_json` 并改调同一入口，单图 JSON 的外层数组、`file` 与全部既有字段及末尾换行保持不变。CSV 输出符号表和按线号汇总的管线表；每条线的 from/to 聚合其全部 run 端点，FF02-06 的 `100-FW` 行覆盖 16 只 `BUV-3201..3216`。新增 `PIDLEGEND EXPORT <.json|.csv>`，支持带空格路径、交互式路径提示、未运行 ON 直接导出及 OFF 后导出，不画覆盖层、不写 XDATA。逐行自动化新增 `pid_legend` 的 `recognise` / `report` / `export`，三者返回同一识别字段，后两者分别追加报告数组与写盘回执；错误动作、缺路径和错误扩展名均结构化拒绝。MCP 的 `ocs_read op=pid_legend` 直接提供只读 `recognise` / `report`，文件写出明确留在 `ocs_execute run` 的 `PIDLEGEND EXPORT`，不会从 read 工具产生副作用。README、MCP control guide 与 user-guide 已补命令、格式和请求示例。**验证**：lib 侧 `pid` 过滤 **71 过 / 2 ignore**；`--test pid_legend` **29/29**；MCP 模块 **11/11**；命令导出、逐行自动化、control/MCP 路由与只读约束定向测试、真实 FF02-06 CSV 覆盖测试、`dxf_legend` example 编译均通过；`cargo check --lib --tests --examples` 与 wasm32 lib check 通过；改动行无 Clippy / IDE 诊断，新增文件 rustfmt 干净，其余文件未增加既有 rustfmt 债。W9 尚余 C2 全量本地化、C3 例外区及 C8 其余完整用户文档。
- **例外区子阶段进度（2026-09-11）✅ 已落地**。图例面板底部新增默认折叠的「例外 N」，展开后按未知块 / 未命名形状 / 无主位号 / 范围标注 / 重复位号五类显示；各类数字直接从报告已有字段求和，口径不另造。识别内核新增仅供编辑器导航的 `ExceptionLocations`：未知块保留 INSERT，未知形状保留每个重复分量的笔画与 bbox，三类文字例外保留原 TEXT / MTEXT 句柄和锚点；范围标注缺失的成员仍跳到那条范围文字。每条明细都可选中原对象并缩放，报告文本及 `dxf_legend --json` 的既有 schema 均不增加字段。user-guide 已补例外区操作。**验证**：lib 侧 `pid` 过滤 **73 过 / 2 ignore**；`--test pid_legend` **29/29**；五类计数、unknown block / shape 位置、orphan / range / duplicate 文字位置及点击选择缩放均有定向断言；`cargo check --lib --tests --examples` 与 GUI build 通过。W9 尚余 C2 全量本地化和 C8 其余完整用户文档。
- **提前做掉的一条（2026-09-09 23:5x，会话 fable-5-1-20，用户 22:42 点名「有位号的和无位号的做一个分组过滤」）✅ 提交 `eddb568d`**：面板类内行序改成**先带位号（按位号排）、后无位号（自上而下、自左而右）**——顺带把 C13 的行序抖动挡在面板这一层；标题下加过滤行「全部 N / 有位号 M / 无位号 K」（`PidLegendFilter`，状态在 `OpenCADStudio.pid_legend_filter`，`Message::PidLegendFilter`）。「无位号」起初 = 该类应带位号而没配到的符号（不编号的类不算），**用户 2026-09-10 00:1x 改口：不编号的类也算进「无位号」**，提交 `a852d9b8`——现在两组是字面意义的有 / 无位号，每个符号必在其一，类头的「位号 m/n」区分「应带而缺」与「本就不编号」。单测 `rows_group_tagged_before_untagged_and_the_filters_take_one_group_each`；lib 侧 8/8；rustfmt 四个文件与 HEAD 同一套旧格式差、clippy 无新告警、`cargo build --bin` 过。C2 走 `t!` 时把这三个按钮的字一起搬。
- 出口：切英文界面面板全英文；面板例外区数字 = 命令行报告；GUI `EXPORT` 的 JSON 与 `dxf_legend --json` 对同一张图**逐字节相同**（同一序列化器）；CSV 管线表里 `100-FW` 的 from/to 覆盖 FF02-06 的 16 只 BUV；`automation_op` 单测一条。

### W10 · 图例背书（取证，等外部输入）

- 请用户提供图例张或说明书 `SPE-0100FF01-01`；拿到后逐条核 09-07 §1 块表的「含义」列，改 `label`、把「可靠度」列改成「图例背书」。
- 没有图例张时：五条低可靠块由现场确认；确认不了的**不改名**，`label` 后缀「（疑）」。
- 出口：块表每条要么有出处、要么有「疑」。

## 3. 风险

- **FF02-05 正在被编辑**（OCS pid 66696，autosave 20:43 还在写）：本计划任何一期都不读写这个文件；测试照旧跳过、W0 的汇总行会把它列出来。它被保存过的话盘上就不是 09-05 那份，`SHEETS` 的 145 / 174 / 156÷201 / 6 要重对。
- **W1 改的是报告口径不是识别**：`symbols` 数与位号一个不许动，diff 只看 ORPHAN / UNTAGGED 行；别顺手「修」配对。
- **W4 是纯搬家**：任何一处「顺手改一下」都会让「报告逐字相同」这条验收失去意义；想改的记下来放到 W3 / W7 去。
- **W7 的诱惑**：把 `pipe_stub_mm` 抬高或让轴向线进第一遍连通会连带 44 条字典 id 一起变（D11 原话）——管段只从**已算出的** `SheetRun` 拿，第一遍一个字不动。
- **W3 的 D23**：贪心换最大匹配在 FF02 上应当零变化；若有任何一张变了，先查是新算法错还是旧结果本来就错，不能默认新的对。
- **W8 的 XDATA 与 `.pid` 导入共用 APPID**：键名沿用 `class` / `label` / `resolved`，新增 `lines`；`resolved` 用 `legend:` 前缀区分来源，特性面板两条路都能读、不能互相盖。
- **W9 的 EXPORT 与 ON 的关系**：导出不依赖画进图；`OFF` 之后 `EXPORT` 照样能出（识别留在 tab 上，D24 的过期判定除外）。
- **lib 侧测试与别的会话抢可执行文件**（今天 LNK1104）：跑 lib 测试前 `Get-Process OpenCADStudio-*` 看一眼，别与别的会话并发。
- **两份计划同时存在过**：姊妹文件按用户指示删除；它独有而本文件未并入的条目列在 §6，不算丢。

## 4. 待拍板

1. **顺序**：按 D29（W0 → W1 → W2 → W3 → W4 → W5 → W6 → W7 → W8 → W9）？还是把 W8 / W9 的数据落地（XDATA + `EXPORT` + 自动化 op）提到 W4 拆文件之后、WS02 与 SP02 管线之前——如果下游 plant 数据管线现在就要吃位号 / 线号表，这两条比 W6 / W7 都急。
2. **D20 范围标注**：展开核对（建议）还是只压掉？展开多 60 行代码，换来每张图一次免费的交叉验证。
3. **D21 设备表行**：「同名已认领就不报」是否成立？反例：同一张图两只同号阀（一只真漏），这条会把漏的那只藏起来——所以建议默认不报但**列进「重复位号」行**，不是删掉。
4. **W6 连接符**：WS02 的「去往 / 来自 …」文字算不算连接符的位号？算的话 `tag.shape` 允许中文字面。
5. **D27 线号规则**：`regex` 加为直接依赖（已在树里）可以吗？不可以就用现有形状语法（`9` / `?` / `*`）扩一个 `{n}` 计数写法，零新依赖。`DN100` / `DN200 1.6MPa` 这类规格字不算线号，同意否？
6. **D28 `PIDLEGEND PURGE`**：`OFF` 只清覆盖层、`PURGE` 连 XDATA 一起清——还是 `OFF` 就全清、不加新动词？建议分开：位号写进实体是有价值的数据，不该因为想关掉彩框就丢。
7. **导出格式**：JSON（主，同 `dxf_legend --json`）+ CSV 两个都出，还是只 JSON？
8. **图例张 / 说明书 `SPE-0100FF01-01`** 能否提供；不能的话 W10 走「现场确认 + （疑）后缀」。

## 5. 本轮审核的验证摘要

- `cargo test --test pid_legend` → **17 过 / 0 败**（0.42 s，编译 16.6 s）；FF02-05 被锁、按现有设计静默跳过（`dxf_legend` 直读同一文件报 `os error 33` 证实）。
- lib 侧 `cargo test --lib -- pid_legend pid_pipes pidlegend pidline` → **20 过 / 0 败**（0.40 s；808 filtered out）。第一次跑时另一会话的 `cargo`（pid 41788）正占着 `OpenCADStudio-56100d61748a32f1` lib-test 可执行文件、链接 LNK1104，等它结束后复跑即绿——§3 那条风险由此而来。
- `dxf_legend` 11 张全跑：符号数 / 位号 / 无主 / run / 端口 / 断头与 D12–D19 记录逐字相同；每张 ≤ 0.2 s。
- 34 条无主逐条分类：19 范围标注 / 6 同名已认领 / 9 单号 LA（§1.3）。
- SP02-06 DXF 直查：`^\d+-[A-Z]{2,3}-\d{4}-[A-Z]\d$` 命中 5 个不同线号（C11）。
- `PID_SEMANTICS` XDATA 现状核对：`src/io/mod.rs:20` 常量、`pid.rs:395-398` APPID 注册、`pid.rs:877-898` 写法、`properties.rs:120-141` 读 `class` / `label` / `resolved`；`regex` 在依赖树里（`cargo tree -i regex`：经 `env_filter`）。
- 代码走读：`recognise_with_units`（728–1104）、`report`（2595–2730）、`pidlegend.rs` 全文、`pid_legend_list.rs` 全文、`pid_pipes.rs` 头部与 `pipe_segments`、`commands/mod.rs` 自动补全注册表、`automation.rs` op 表、`assets/pid-legend.json` 全文。
- 环境：plannotator 0.27.12；语料目录 14 个文件（12 DXF + FF02-05 的 `.ocs.lock` 与 `.ocs-autosave.sv$`）。

## 6. 与姊妹计划的合并记录（2026-09-09 21:20）

姊妹文件 `2026-09-09-pid-recognition-audit-and-next-steps.md`（另一会话 21:01–21:03 写、21:02 Plannotator 秒批）已按用户指示删除。两份审核结论八成重叠（无主 34 条、WS02 零覆盖与 31 断头、SP02 无管线、面板硬编码中文、无导出 / 自动化口、软跳过、`PIDLINE` 未注册它没发现）。

**并入本文件的三条**（用户点名）：线号规则外置（D27 / C11 / W5）、识别结果写进 XDATA（D28 / W8）、拆文件（D26 / C12 / W4）。

**它独有、本次未并入的**（列在这里不算丢，要的话一句话拉回来）：

| 条目 | 它的说法 | 未并入的理由 |
|---|---|---|
| `classes` 类别表 | 规则里 `class → label / color / tag` 收成一张表，`blocks` / `tag_classes` / `dictionary` 只引用类别名；顺手解开 `flow-indicator` 撞名（FF02 的水流指示器块 vs SP02 的 FI 流量指示） | 用户只点了三条；撞名那一句值得做，可挂进 W5 一起（改类名 `water-flow-indicator`，层名随之变） |
| 内存映射读被锁的真图 | 测试里 `load_sheet` 打不开先用 `MemoryMappedFile` + `FileShare.ReadWrite` 绕字节范围锁 | 本文件 W0 只做「跳过可见」；绕锁读编辑器正开着的文件有读到半截的风险，09-07 是手工一次性用的 |
| `DUPLICATE` 行 | 同类同位号 ≥ 2 处（SP02-07 `HS-0320A` ×4）列坐标 | 盘装 / 现场同号是正常画法，先不报；W1 之后若还想要，一行规则 |
| 面板符号行点击 = 选中实体 | 与族行同一手势（依赖 `Recognized.handles`） | W8 有 `handles` 之后顺手可做，挂在 W9 |
| 形状命名工具进仓 | `dxf_legend --shapes DIR` 出 SVG 取代 `%TEMP%` 的两个 Python | 今天 11 张图未命名形状为 0，无实例；新语料来了再做 |
| 第二语料 | 请用户提供一张非 CPECC 的 P&ID DXF 验泛化 | 等外部输入，与 W10 同性质；有了就开一期 |
| 泛化盲点清单 | `lettering_of` 不读块内文字 / ATTRIB、MTEXT 格式码只剥 `\P`、单位判定二选一、`compose_tag` 长度写死 | 第二语料到了才有验收对象；先记 |
