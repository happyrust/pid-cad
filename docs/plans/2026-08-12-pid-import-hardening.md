# P&ID 导入加固 · 开发计划（2026-08-12）

> 产出自一次 grilling 会话。带 ✅ 的决策已当面拍板；带 ⭕ 的按推荐落笔、**未经拍板**，
> 在批注里划一笔即可翻案。
>
> **进度（2026-08-12 晚）**：plannotator 会话未返回批注即被关闭，用户以「继续」放行。
> OCS 侧四件已完成并全绿：W1 保存闸门、W2 两层可见性、W5 四语言、W6 文件关联
> （`tests/pid_import.rs` 16 条 + `tests/pid_panel_localization.rs` 12 语言）。
>
> **终态（2026-08-13）**：全部七项完成，其中 W7 的结果超出计划。
> W3（注册表修牙）与 W4（普查认领改「起点相等」）按计划落地；W7 走到 Coverage Gap
> 分支后又被推翻——旋转角其实就在 `igTextBox` 记录里，存成方向对 `(cos, sin)`，
> 已接线，35+ 条竖排标签立起。后续超出本计划的收获：退役固定 68 开销、按原生
> 读序实现 `igTextBox` 三形状，全族 260 条零拒收，43 条缺失标签回到图上。
> 证据见 pid-parse 的 `docs/analysis/2026-08-12-*` 与 `2026-08-13-*` 六篇；
> 两篇中途的阴性结论（W7 Coverage Gap、A01 无字）均已挂撤回标记留档。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| D1 | 范围 | 双仓接线兑现：OCS 四件 + pid-parse 三件卡显示的；其余登记不做 | ✅ |
| D2 | 保存语义 | Save 静默转 Save As，默认同 stem 的 `.dwg`，原 `.pid` 永不可写 | ✅ |
| D3 | 可见层级 | 两层：表读失败补 `log::warn`；导入汇总一行推命令行 | ✅ |
| D4 | 文件关联 | 注册 ProgID 进「打开方式」，不抢默认，不做缩略图 | ✅ |
| D5 | 旋转角纪律 | native reader 为准 + fixture 交叉验证；纯 corpus 不接线；无果登记 | ⭕ |
| D6 | 静默洞修法 | 先修 census 报表口径；扫描→链式根治登记为后续 | ⭕ |
| D7 | 执行顺序 | 按风险降序 W1→W7（丢数据最先，取证压轴） | ⭕ |
| D8 | 落点 | 本文档进 `docs/plans/`；新建 `CONTEXT.md` 收录已敲定术语 | ⭕ |

## 背景

`.pid`（SmartPlant P&ID）经 `pid-parse` 解码、由 `src/io/pid.rs::load_pid` 投影成文档，
解析→导入这段完整且有回归覆盖（`tests/pid_import.rs`，14 条全绿）；缺口集中在
**导入之后**：保存路径会毁原图、缺口报告说给了没人听的通道、解析侧三件事直接压着显示保真。
本轮已顺手落地一项（见文末「第 0 项」）。

## 目标

一句话：**打开一张 `.pid`，看到的是图纸自己声明的样子；丢了什么有人告诉你；怎么操作都毁不掉原图。**

验收基线：`tests/pid_import.rs` 与 pid-parse 各 ratchet 测试全程保持绿（数值变更须随
analysis 文档）；两仓是路径依赖，pid-parse 侧每项完成后必须回跑 OCS 侧 `pid_import` 测试。

---

## 工作项

### W1 · 保存闸门（OCS）✅D2

**现状**：打开时 `current_path` 被设成 `.pid` 路径（`src/app/update/file.rs:1093`）；
保存按扩展名分发，`.pid` 落进 DWG 分支（`src/io/mod.rs:1552-1557`）——Ctrl+S 会用
DWG 字节**原子替换掉 SmartPlant 原图**。README 声明 "PID read"，代码没有闸门。

**改法**：引入「只读来源格式」判定（单一函数，见术语表）：写入目标扩展名为 `pid`
（忽略大小写）时拒绝执行写入，转 Save As 流程，对话框预填同 stem + `.dwg`。
覆盖三条入口：SAVE/QSAVE、关闭脏页时的保存提示、任何以 `current_path` 为目标的续体。
autosave 不动（`.sv$` 本就装 DWG 字节、打开按魔数嗅探）；edit-lease 维持现状（无害）。

**验收**：闸门函数单测（`.pid` 目标一律拒写）；手工验证 Ctrl+S 直接弹 Save As 且默认名
`<stem>.dwg`；关闭脏页走同一闸门。

**风险**：保存入口分支多，漏一条等于没堵。对策：判定收敛为单一函数，全部入口调它，
测试枚举入口。

### W2 · 两层可见性（OCS）✅D3

**现状**：`src/io/pid.rs:168/174/181` 三处 `unwrap_or_default()` 把样式/字高/填充表的
读失败吞成静默降级（全图退回 ByLayer 白线，无任何输出）；`report_import` 的全部告警走
`log`，从资源管理器启动时 stderr 没人看得见——报告写了，读者收不到。

**改法**：①三处失败各补一条 `log::warn`（带错误原文与文件路径）；②导入完成时向命令行推
**一行**汇总：画出实体数 / 无解码器与被拒收记录数 / 样式表未读出（仅发生时）。
约束：`acadrust::ReadOutcome` 是外部类型加不了字段，侧信道在 OCS 侧解决
（`read_pid_path` 返回处包一层或 thread-local，实现时定，不改 acadrust）。

**验收**：汇总行格式单测；坏 StyleCluster 场景下三条 warn 可见；手工开图命令行见一行。

**风险**：刷屏。对策：命令行只此一行，逐条明细留在 log（C 方案已否决）。

### W3 · 注册表假绿与 no-op 欠账（pid-parse）✅D1 范围内

**现状**：`sheet_families.rs:216-222` 标 `igBoundary2d` `emits_geometry: false`，而
`IgBoundary2dEmitter` 实际发 `Polyline{closed:true}`（fill 那轮起）；守护测试只读标志、
不对照 `EMITTERS`，是假绿。`igSmartFrame2d` 缺显式 no-op emitter，违反 `geometry.rs`
模块 doc 自立的「audit-only 必须注册显式 no-op」。三处注释（`geometry.rs:1068-1069`、
`model/sheet.rs:89-97`、EMITTERS 分组注释）陈述过期。

**改法**：`emits_geometry → true`；测试改为真对照 `EMITTERS` 的 no-op 集合（先故意翻转
标志验证测试会红）；补 `IgSmartFrame2dEmitter` 显式 no-op；同步三处注释。

**验收**：翻转标志测试红、复原绿；geometry golden snapshot 不变（纯记账，零行为变化）。

### W4 · 静默洞报表（pid-parse）⭕D6

**现状**：32 条被拒记录连 `refused_graphic_records` 都进不去——起点被三个扫描型家族
（`SubRecord0x0010` / `JStyleOverride` / `AttributeFragment`）的越界认领盖住
（`docs/analysis/2026-08-11-what-refuses-the-remaining-53.md` §「第二个静默洞」）。
OCS 的 `report_import` 因此**低报**。

**改法（推荐）**：census 的 claimed 口径排除非链对齐的扫描认领，让 32 条进 refused 告警；
「三族改链式认领」的根治**登记为后续项**（`teach/NOTES.md` 候选 4 已有 08-05 零剩余证据，
但动的是 7100 行的 `sheet_records.rs` 核心，本轮不背）。顺带补 pid-parse `CHANGELOG.md`
`[Unreleased]` 08-07 以来的公开行为变更条目（`igBoundary2d` 发实体、`igLine2d` 计数
238→368——对 OCS 可见却无账）。

**验收**：`tests/render_gap_census.rs` ratchet 更新为新实测值并逐 fixture 钉死
（refused 预期增量 ≈ +32）；新增一篇 analysis 记录口径变更的原因与数字；
回跑 OCS `pid_import` 全绿。

### W5 · 四语言 P&ID 面板（OCS）✅D1 范围内

**现状**：`ar-SA / es-ES / ja-JP / pt-BR` 四份 `.ftl` **完全没有** P&ID 组条目
（`item-tag` / `line-number` / `matched-by` 零命中；zh-CN 有 3 条）；`Language` 枚举四个
变体已存在；`tests/pid_panel_localization.rs` 只遍历 8 种语言。

**改法**：四份 `.ftl` 补齐 P&ID 组条目（真翻译，不是占位）；测试遍历列表加四个变体。

**验收**：先扩测试确认红（证明有牙），补条目后绿。

### W6 · 文件关联（OCS）✅D4

**现状**：`file_association.rs` 只注册 `.dwg/.dxf/.bak`；双击 `.pid` 与 OCS 无缘，
single-instance 转发用不上。

**改法**：`register_progid("OpenCADStudio.PID", "Smart P&ID Drawing (read-only import)")`；
`SupportedTypes` 加 `.pid`；`.pid\OpenWithProgids` 挂上；`unregister` 清理路径同步扩。
不抢默认（工程站上默认多半是 SmartPlant 本尊）；不做缩略图。

**验收**：手工——右键 `.pid`「打开方式」见 OCS；设为默认后双击经单实例转发正常打开；
unregister 后注册表键无残留。

### W7 · 旋转角取证（pid-parse）⭕D5

**现状**：`igTextBox` emitter 硬编码 `rotation: 0.0`（`geometry.rs:1245-1253`），实测
整张图 40 条文字全 `rot=0`，沿竖管的标签全部躺平。OCS 消费线已接好
（`text.rotation = rotation.to_degrees()`），解析侧坐实即零改动兑现。

**纪律（推荐）**：native reader 定位 `igTextBox` 读序为准（此路在 `igSmartFrame2d` 的
`sub_564464D0` 与 style.dll 上都走通过）；controlled fixture（旋转一条标签重发布）交叉
验证；**纯 corpus 统计的偏移不准接线**——GLine2d 与 JStyleOverride 锚点两次撤回都是这么
栽的；接错的旋转角是「像真话的假话」，比躺平伤得多。时间盒两个工作日等级，无果则以
Coverage Gap 登记，文字维持躺平兜底。

**验收**：analysis 文档含原生读序引用 + fixture 差分；ratchet 出现非零旋转；
`pid_probe` 实测 `rot≠0`；OCS 侧零代码变更。

---

## 执行顺序 ⭕D7

**W1 → W2 → W3 → W4 → W5 → W6 → W7**，一项一提交，每项自带测试先红后绿。
理由：会丢用户数据的最先；W3 是 W4 的记账地基（同在 pid-parse、census 与注册表口径相邻）；
W5/W6 小而独立随后清掉；取证性质的 W7 压轴，无果不卡盘。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 53 条拒收的三总体处置（A33 / B12 / C8） | W4 只让它们可见；处置需逐类取证，无界 |
| 扫描→链式认领根治 | 动 7100 行核心，证据在但收益本轮不需要；登记为 W4 后续 |
| v2 基类块字节账、`0x002C +34` 文字颜色、字体名偏移 | 纯取证项，对屏幕无直接回报 |
| `igDimension` / `igBalloon` / `igLeader`、曲线族 | 全语料 0 命中，无 fixture 可验 |
| `.pid` 缩略图提供器 | DWG 管线不通用，收益配不上 |
| 「无标题导入」模型（D 方案） | 概念更诚实但动打开/最近文件/监视多处，D2 已覆盖风险 |
| `build_entities` 的 Arc/Circle 死分支 | 前向兼容留位，曲线族解出即启用 |
| report_import 逐条推命令行 | 刷屏，D3 已定一行汇总 |
| Lesson 0003 教学材料 | `teach/NOTES.md` 自规「课件与业务代码分开」，另行推进 |

## 术语（`CONTEXT.md` 草案，随 D8 批准落盘）⭕D8

- **只读来源格式（read-only source format）**：OCS 能打开并投影成文档、但永远不作为写入
  目标的格式。Save 一律转 Save As。当前唯一成员：`.pid`。
  避免说：「导入格式」（含糊——DXF 也可称导入但可写）。
- **导入汇总行（import summary line）**：打开完成时推到命令行的单行报告：画出实体数、
  缺口计数、样式表可用性。避免：逐条告警刷屏。
- 证据语言**沿用 pid-parse 的 `CONTEXT.md`**（Decoded Data / Probe Evidence /
  Coverage Gap / Controlled Fixture），OCS 侧文档不得另造同义词。

## 第 0 项（已完成，待提交）

LWDISPLAY 默认可见：`load_pid` 置 `doc.header.lineweight_display = true`（`.pid` 无文件头
可读此标志，fresh 文档默认 false，wire shader 会把全部线宽折成发丝线）；配套回归
`the_widths_the_drawing_states_are_switched_on_for_display` 遍历四张 fixture。
本轮已改，14 条测试全绿，尚未提交——建议作为本计划首个提交一并走。
