# 真实图层接线与 JSite 几何盲区 · 开发计划（2026-08-29）

> 承接 `2026-08-12-pid-import-hardening.md`（W1–W7 已于 08-13 全部完成）。
> 本计划基于 08-29 对双仓的审核：OCS 侧文字保真（08-22）、符号库渲染（08-24/25)、
> 样式 discipline 归层（08-26）均已落地；pid-parse 侧 08-27 的 aux_hi 判决
> （`payload +8` = 图元所在图层，四主图 290/290 图层、1240/1240 对象满分）
> 把「图纸自己声明的图层」变成了可交付物。
> 带 ⭕ 的决策按推荐落笔、未经拍板，批注里划一笔即可翻案。
>
> **终态（2026-08-31 / 09-07；本文件 2026-09-13 才入库，此前一直是未跟踪文件）**：W1–W6 全部完成。
> pid-parse：`814c8da`（aux_hi / boundary 证据隔离）→ `4a6c302`（W1 `JSheetLayer` 290/290，manager 对账）
> → `3a93681`（W2 图元带 storage-local oid / name）→ `af8e802`（W5 折线闭合位：618 文件 / 31 折线 / 11 闭合）
> → `87c7225`（W4 变换缺口登记）。OCS：`19e69888`（W3 XDATA `sheet_layer=` / 特性面板 / 汇总行 /
> `PID-HIDDEN` 默认关 / DWG-DXF 往返）、`6e1f40d3`（W5 探针补 y 检查、符号库缺失提示、dump 闭合列）、
> `17ebb7ee`。W4 于 09-07 以「嵌套 `LdcSite` = 内嵌符号定义缓存，变换就在 `igSymbol2d` 放置记录里」关闭：
> pid-parse `58360d0` / `a0313b6` / `b0ed20f` / `89ffc7b`（圆 / 弧 / 矩形 / B 样条四族解码器，107/107 放置解析），
> OCS `27798a4f` / `e9cdee75`（无库时用缓存本体替掉占位圆点、B 样条按曲线画）。W6 见该节进度。
> 闭环台账：pid-parse `docs/plans/2026-08-31-real-sheet-layers-closeout.md`、
> `docs/analysis/2026-08-31-jsite-geometry-coverage-gap.md`；后继计划
> `2026-09-07-jdim-driving-dimensions-and-layer-panel.md`（JDim 与图层面板重构）。

## 决策记录

| # | 决策 | 结论 | 状态 |
|---|------|------|------|
| D1 | 范围 | 双仓接线：真实图层主线（W1→W3）+ JSite 几何盲区取证（W4）+ 上轮遗留尾巴（W5）+ 台账同步（W6） | ⭕ |
| D2 | 真实图层在 OCS 的落法 | 分三步：事实先行（XDATA + 汇总行）→ 隐藏语义（Hidden 类图层默认不可见）→ 图层面板重构**暂缓登记**；不替换现有 PID-* taxonomy | ⭕ |
| D3 | JSite emit 纪律 | ownership/变换证明先行，纯 corpus 规律不接线；时间盒两个工作日，无果登记 Coverage Gap | ⭕ |
| D4 | 执行顺序 | 先 pid-parse 地基（W1/W2）后 OCS 消费（W3），小件穿插（W5），取证压轴（W4） | ⭕ |
| D5 | 在途工作 | pid-parse 的 aux_hi 第二轮落地（sheet_records +88 / 测试 +466 / 两篇分析 / 四个 probe）**先提交再开新活**，保持一项一提交 | ⭕ |

## 背景

`.pid` 经 `pid-parse` 解码、由 `src/io/pid.rs::load_pid` 投影成 OCS 文档。上一轮计划的
目标句是「打开一张 `.pid`，看到的是图纸自己声明的样子」——经过文字保真、符号本体、
样式 discipline 三波之后，这句话还差的最大一块是**图层**：

- OCS 现在的 15 个图层（`PID-GEOMETRY` / `PID-TEXT` / `PID-SYMBOL` / `PID-STYLE-*` …）
  全部是导入器自造的 taxonomy；SmartPlant 用户在原软件图层面板里看到的
  `Default` / `Labels` / `HeatTrace` / `DrawingBorder` / `Hidden`…（带显隐状态）一个都不在。
- pid-parse 08-27 已证明：每条图元记录信封 `+8`（`aux_hi`，现名 `sheet_layer_ref`）就是
  它所在图层的 oid；`shlyhp.dll` / `imagdex.dex` 原生侧证实厂商就管这条边叫 Layer
  （`HGeomGetLayer` / `HGeomPutLayer`）。当前只有 `igBoundary2d` 一个 DTO 带此字段，
  图层名字表（`JSheetLayer` 0x0081）尚无解码器。
- 同一刀顺带翻出 **JSite 几何盲区**：圆 12 / 弧 12 / 矩形 3 / B 样条 1 / 边界 9，全在
  嵌套 `JSite*/PSMcluster0` 存储里，而 `streams/cluster.rs` 只把叶名 `Sheet*` 的流交给
  几何解码器。`AGENTS.md` 里「语料无 igCircle2d/igArc2d」的说法已被订正为「顶层没有」。

## 目标

一句话：**打开一张 `.pid`，图纸自己声明的图层跟着每个图元进文档；SmartPlant 藏起来的，
OCS 也不亮；JSite 里的圆、弧、矩形不再是盲区。**

验收基线：OCS `tests/pid_import.rs` 与 pid-parse 各 ratchet 测试全程保持绿（数值变更须随
analysis 文档）；pid-parse 侧每项完成后回跑 OCS 侧 `pid_import` 测试。

---

## 工作项

### W1 · JSheetLayer 名字表解码（pid-parse）

**现状**：`0x0081 JSheetLayer` 的布局已在 `2026-08-27-tag-184-*.md` §4 钉死（`+12` 对象
计数、`+16` 图层号、`+20` 字符数 + UTF-16 名、第二个名全语料为空），四种长度精确收尾，
来源是 `shlyhp.dll::SheetLayer::IJPersistImp::Save` 的逐字段反汇编；tag-183 证明
`JSheetLayerManager` 全量登记 290/290。但 `src/` 里没有解码器——图层名到不了任何消费者手里。

**改法**：按存储解码 `JSheetLayer`：`oid → (名字, 对象计数, 图层号)`；`JSheetLayerManager`
的 183 表做全量对账；结果按存储分组挂上 import view（名字、计数、oid）。同名多份
（每视图过滤集一份）原样保留，不合并——消费者按 oid 找名字，不按名字找对象。

**验收**：四主图 290 个图层全解、名字集合与分析文档一致；per-fixture 精确计数 ratchet；
`A01/JSite204` 的 `Default` 声明 103 / 实际 107 那 4 个差额按未解释记账，不得悄悄吸收。

### W2 · `sheet_layer_ref` 提升到全部写图层家族（pid-parse）

**现状**：只有 `igBoundary2d` 带 `sheet_layer_ref`。aux_hi 名册给出的写图层家族共 12 个：
`0x0018` line 614 / `0x005E` point 246 / `0x004D` textbox 235 / `0x0084` linestring 137 /
`0x00CE` symbol 109 / `0x0013` boundary 24 / `0x0115` dim 14 / `0x0059` circle 12 /
`0x0061` arc 12 / `0x003D` smartframe 10 / `0x0020` rect 3 / `0x005D` bsp 1。
`PidGraphicEntity` 没有图层字段，OCS 只能拿到样式，拿不到图层。

**改法**：已有解码器的家族（line / point / textbox / linestring / symbol / boundary /
smartframe）decoded record 与 DTO 各加 `sheet_layer_ref`（信封 `aux` 高半段，读法各家族
一致）；`PidGraphicEntity` 增加图层字段（oid + 经 W1 名字表解析出的名字，按记录所在存储
解析）。StyleCluster 字形线（`+8 == 0`）保持无图层。**尚无解码器的家族（dim / circle /
arc / rect / bsp）不在本项**——它们是 W4 的对象。

**验收**：`psm_aux_hi_*` 棘轮口径改走 DTO（1240/1240 对象带层）；各家族既有精确计数
一条不变；OCS 侧 `pid_import` 回跑全绿。

### W3 · OCS 消费真实图层（OCS）

**现状**：OCS 完全不知道图纸图层。三件事分开看：①图层作为**事实**（这个图元在图纸里
属于哪层）没有任何呈现；②图层的**隐藏语义**没接——SmartPlant 里 `Hidden` /
`HiddenObjects` / `Invisible` 图层上的对象用户是看不到的，OCS 全画出来了；③图层面板
展示的是合成 taxonomy 而非图纸声明。①②是保真缺陷，③是产品形态决策。

**改法**（按 D2 分三步）：

1. **事实先行**：每个带图层的实体把「图纸图层名」写进既有 P&ID XDATA 组（特性面板
   随之显示）；导入汇总行加一段图层计数。零破坏，检索与审计立刻可用。
2. **隐藏语义**：图纸图层名属于隐藏类（`Hidden` / `HiddenObjects` / `Invisible`，大小写
   与空格变体按语料实测收敛）的实体，归入合成层 `PID-HIDDEN`，该层默认关闭。
   打开图纸所见与 SmartPlant 一致；要看隐藏内容开一个层即可。
3. **图层面板重构**（真实图层 × 实体类别两维怎么共存）**登记暂缓**：现有 taxonomy 承载
   着评审状态一键开关、discipline 分层、符号标签默认隐藏等已交付语义，一个实体只有
   一个 layer 槽，替换是大动作，等 1/2 落地后按实际使用再议。

**验收**：0201/0202 打开后特性面板可见图纸图层名；语料里隐藏类图层上的对象（若有）
默认不可见且开层可见；`pid_import` 新增断言（图层名分布 + 隐藏层归属）先红后绿。

**风险**：语料四图隐藏类图层可能全空（D06 顶层十二个图层八个计数为 0）——若实测无
对象，第 2 步降级为"机制 + 单测 + 空跑"，不造假数据。

### W4 · JSite 几何盲区取证与解码（pid-parse，时间盒）

**现状**：37 条可画几何（圆 12 / 弧 12 / 矩形 3 / B 样条 1 / 边界 9）在嵌套
`JSite*/PSMcluster0`，管线只收叶名 `Sheet*` 的流，一条都进不了投影。nested ownership
历史上两次拒绝 emit（Phase 34-B/E）；新证据是 08-27 的三件套：这些记录自己带
`sheet_layer_ref`（层在同 JSite 的图层表里）、spacemap tag-181 父链、
`igSymbol2d jsite_ref` 已系到 tag-181 边（`4fa9ab4`）。

**改法**：取证先行——回答「JSite 存储里的几何以什么变换投影到页面」（候选：随
`igSymbol2d` 放置，即符号实例的本体；或独立页面内容）。证明后按
`igCircle2d → igArc2d → igRectangle2d → igBspCurve2d` 逐家族 decoder slice，emit 走既有
normalized 管线并带图层；边界 9 条随撤门自然进入。**纪律同 W7 旧例**：native reader /
controlled fixture 证据才接线，纯 corpus 统计不接线；时间盒两个工作日，无果以
Coverage Gap 登记，盲区维持现状兜底。

**验收**：analysis 文档含变换证据链；OCS 屏幕上出现此前缺失的圆/弧（若证明为符号本体，
则对照符号库渲染验证不重复）；ratchet 新增 per-fixture 计数。

### W5 · 探针与符号折线尾巴（OCS + pid-parse）

**现状**（`2026-08-24-two-texts-outside-the-frame.md` §7 明确记录仍未决，本轮已核实）：

- `examples/pid_probe.rs:121` 出界检查只测 x 不测 y（漏过 y = -127 的标签一次）；
  probe / plot dump 传相对路径时静默失去符号库（普查 180 → 23 无任何提示）。
- `pid_plot_dump` 的 `poly` 行不带闭合标志——图框在 dump 里只剩三段，误导读 dump 的人。
- `SymbolPrimitive::Polyline`（pid-parse `symbol_library.rs:159`）无 `is_closed` 字段，
  OCS `place_primitive` 相应画成开口。

**改法**：probe 出界检查补 y；两个 example 在符号库缺失时向 stderr 明说一行；
`pid_plot_dump` poly 行加闭合列。`is_closed` 先取证：`.sym` 折线记录里有没有闭合位、
库里有没有实际闭合折线——有则两侧接线（pid-parse 读出 + OCS 透传），无则在分析文档
登记「未见闭合折线」后收口。

**验收**：单测 + 四 fixture probe/dump 输出对照；`is_closed` 若接线须有 fixture 证据。

### W6 · 台账与在途工作（双仓）

**现状**：pid-parse `task_plan.md`「当前阶段」停在 Phase 34-F（2026-07-10），实际已过
Phase 40 走到 spacemap/图层线，新读者会被带偏；OCS `cargo test --lib` 有 3 条
`app::automation` 红测试（`save_then_open_round_trips` 等，08-22 核对文档已证明与
PID 线无关，说好"报回去另派"但至今无人认领）；pid-parse 有一批已完成未提交的工作（D5）。

**改法**：①按 D5 先提交在途工作；②`task_plan.md`「当前阶段」段刷新为一段话 + 指向
最新 plans/analysis 的指针（不重写历史）；③OCS 3 条红测试开 issue 或修复，**不占用
本计划工时**，只保证有人认领。

**验收**：git log 出现在途工作的提交；task_plan.md 头部与实际一致；红测试有 issue 号
或修复提交。

**进度（2026-08-31）✅**：① 在途工作已随 `814c8da` 入库；② pid-parse `task_plan.md`「当前阶段」
刷新为「Phase 41 · 真实图层交付闭环（complete）」，并在 09-07 追加 W4 关闭与四族收口两段；
③ OCS `cargo test --lib` 里 3 条 `app::automation` 红测试（`save_then_open_round_trips` 等）经核对
与 PID 线无关，已转为上游 issue [#941](https://github.com/HakanSeven12/OpenCADStudio/issues/941)
独立处置，本计划不再背它。

---

## 执行顺序 ⭕D4

**W6①（提交在途）→ W1 → W2 → W3 → W5 → W4 → W6②③**，一项一提交，测试先红后绿。
理由：W1/W2 是 W3 的地基且都在 pid-parse，一鼓作气；W3 兑现用户可见价值；W5 小而独立
随后清掉；取证性质的 W4 压轴，无果不卡盘；台账刷新放最后与实况对齐。

## 登记不做（本轮）

| 项 | 理由 |
|---|---|
| 图层面板重构（真实图层 × 类别两维） | D2 第 3 步，等事实与隐藏语义落地后按使用再议 |
| 按视图的图层状态（每 ViewFilterSet 一份图层对象） | OCS 单模型空间，只取一份；视图切换是另一个产品命题 |
| `JDim`（0x0115）标注解码 | 14 条、坐在图层上，但需要独立取证，本轮不背 |
| `JSymbolInformation` / 公式驱动尺寸（0x006F）接 OCS | 解码刚落地，消费形态未想清，收益未明 |
| `0x0057 +32` 未认字段 | 已证明不是图层数（36/49），不用不猜 |
| `A01/JSite204` Default 计数差 4 | 非主语料、未解释记账，等新证据 |
| `204D4DD1` / `C3BE37E0` 接口名 | 佐证性质，`C3BE37E0` 疑似 `IJLayer` 无证据不采用 |
| OCS `app::automation` 3 条红测试的修复 | 与 PID 线无关，W6 只保证转出认领 |

## 术语（沿用两仓 `CONTEXT.md` 证据语言）

- **图纸图层（sheet layer）**：`JSheetLayer` 对象，图纸自己声明的图层；避免与 OCS
  合成层（`PID-*`）混称。每存储一套，同名多份按视图过滤集。
- **可画对象名册**：aux_hi 二分给出的 12 个写图层家族清单——语料自己声明的「什么算
  图元」，不依赖解码器认得谁。
- **隐藏类图层**：名字表意为隐藏的图纸图层（`Hidden` / `HiddenObjects` / `Invisible`），
  W3 第 2 步的对象；大小写与变体以语料实测收敛。
