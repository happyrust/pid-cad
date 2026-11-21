# 协同离线模式（REQ-COLL-015 落地 + 离线修改跨重启）方案

2026-11-17。范围：REQ-COLL-015「加入时的图纸对齐」全 4 条验收，加上「离线修改跨重启提交」（队列落盘）。REQ-COLL-014 的既有边界见 `SPEC-collab-client.md` §10；本方案把其中「排队只在内存、退出后不提交」一条收窄到「从没进过房间的图」。

## 0. 结论一页

- **三项协同状态随图保存**：`图号` / `服务器库标识` / `已应用修订` 写成第 5 个图级 XRecord（`PID_AUTHOR_COLLAB`），走 `graph_data::store` 既有机制，只写进**保存快照**、不动活文档——不弄脏 `dirty`、不进撤销栈。
- **`服务器库标识` = `repo_id`**：持久库在 SQLite `meta` 表首启生成一次 UUID 永久不变；无库（内存模式）每次启动随机一个。`welcome` 恒带它，客户端记下图所属房间的身份。
- **加入对账**：`hello` 新增 `repo_id`；文件里存了房间的图，客户端把它当「上一次会话的记忆」报出 `repo_id`+`since`。对不上、本地比服务器新、或日志不够补发 → 会话仍建立（能看房间），但服务器紧随 `welcome` 发 `AlignRefused{code,message}` 标记不一致：客户端标 `diverged`、显示「需要取得最新图纸」、停止提交（`diverged` 本就闸住 `pump()`）。
- **离线修改跨重启**：还没有得到答复的 `outbox`（没发出的，和已经发出还在等答复的）在保存时写成第二个新 XRecord（`PID_AUTHOR_COLLAB_QUEUE`），重开后重连照常提交。种子化让「曾经进过房间的图」在断线时的修改继续排队——这条链天然接上 `had_session` 语义。
- **不做**（本期边界）：从没进过房间的图，离线新建对象的回灌；离线删除/属性改的回补；`diverged` 的恢复入口。理由与后续见 §5。

## 1. 现状盘点

| 环节 | 现状 |
| --- | --- |
| `hello.since` / `welcome.replay_from` | 协议与服务器补发**已实现**（`protocol.rs`、`ws.rs::missed_ops`：整段日志齐全才补，`replay_from` 回应） |
| 客户端 `since` 来源 | `transport::Seen` 只记**本次运行**里的会话：`(server_url, drawing_id, revision)`；进程一关就没了 |
| 图级 XRecord 机制 | `graph_data::{store,load}` 已有四个 key、DWG 七版本往返零丢失（`dwg_roundtrip` 契约测试） |
| `已应用修订` | `collab.base_revision` 已经逐 op 跟踪（`Connected` / `OpApplied` 更新） |
| 断线排队 | `outbox` 内存队列已实现（REQ-COLL-014）；`had_session` 决定是否排队 |
| 拒绝对账 | **没有**：`since` 非法/日志不足时现在只是「不补发」，客户端靠事后对不上才标 `diverged` |
| 服务器库标识 | **没有**：库里无 `repo_id` 概念，`welcome` 不带它 |

## 2. 协议增量（`ocs_collab::protocol`）

```text
ClientMessage::Hello { .., repo_id: Option<String> }   // 新增，旧端忽略
ServerMessage::Welcome { .., repo_id: String }          // 新增，恒发；老服务器=空串
ServerMessage::AlignRefused { code, message }           // 新增：紧随 welcome 标记不一致
codes += REPO_MISMATCH / REVISION_AHEAD / REPLAY_GAP    // 拒绝对账三码
```

- `repo_id`：`hello` 里客户端报「这份文件上次的房间是谁」；`welcome` 里服务器答「我是哪个库」。两端一起发，向后兼容靠 `#[serde(default)]`。
- `AlignRefused`：会话照开（能看房间），这条消息标记「对不上，不补发」及原因码。发在 `welcome` 紧后、任何 `op_applied` 之前。
- 三个新码对照验收 3 的三种拒绝：`REPO_MISMATCH`（库标识对不上）、`REVISION_AHEAD`（本地修订比服务器新，含 `since < 0`）、`REPLAY_GAP`（操作日志不足以补发，含无库可补）。图号对不上属客户端侧（§3.3），服务器侧同图号下的可判情形只有这三种。
- `welcome.replay_from` 语义不变：补得齐才发，补不齐不发。

## 3. 设计决策

### D1 三项状态 = 第 5 个图级 XRecord `PID_AUTHOR_COLLAB`

一条 JSON 对象入 `graph_data::store`：`{"drawing_id":"PID-1101","repo_id":"<uuid>","revision":12}`。
挂在模型空间扩展字典下，与既有四 key 共字典；`kind=graph_data` 前缀天然不被 `collect_orphan_props` 误收。DWG 七版本往返由 `dwg_roundtrip` 既有测试形态照着写一条同型断言即可（验收 1）。

**写入时机：保存时、写进 `document_for_save()` 的快照**，不写活文档。理由：

- 三项状态只是「这张文件上次与房间对到哪」的账，不该让图纸显得被改过（`dirty` 不动、撤销栈不进）。
- `save_tab_synchronously_protected_as` 先取快照再写盘：在 `document_for_save()` 之后、写盘之前把 XRecord 塞进 `snapshot`——天然满足「保存后重开读回不变」。
- 只在「被存的标签页恰是协同图」时写（`is_collab_tab(i)`），别的图不记账。

值来源：`collab.base_revision`（已应用修订）、welcome 里学到的 `repo_id`、`drawing_id` 用房间登记的那个。无会话而有旧记录 → 原样留着（它仍是「上次对到哪」）。

### D2 `repo_id` 持久化：库一级，不是会话一级

- `store.rs`：`meta(key='repo_id', value=uuid)`，`ensure`/建库时 `INSERT OR IGNORE` 生成一次；`Store::repo_id()` 读出。
- 无库（内存模式）：`AppState` 启动时生成随机 UUID——重启即换库，老文件自然对不上（这正是库标识要的效果：内存库重启后所有状态已清，让旧文件按一致处理反而是错的）。
- `AppState::repo_id()` 统一入口：`Some(store)` → 库的；`None` → 启动时生成那个。

### D3 hello 的 `since`/`repo_id` 怎么填（transport）

`Seen` 改记 `(repo_id, drawing_id, revision)`——`server_url` 比对废止，库标识取而代之（同一库换地址也能对账；同一地址换了库会被拒）。两个来源按新旧取：

1. `seen`：本次运行里上一个会话留下的（`Connected`/`OpApplied` 时 `repo_id` 从 welcome 记下）；
2. `told.file_state`：`answer_collab_tags` 每次拨号前从图上读到的 `PID_AUTHOR_COLLAB` 记录——**只在 `file_state.drawing == config.drawing_id` 时用**（文件记的是别的房间 → 不冒领）。

`hello` 填法：`repo_id = seen.repo_id.or(file.repo_id)`、`since = seen.revision.or(file.revision)`——`seen` 在前（同一库里后一场会话比存盘新）。两个都没有 → 老语义：全新加入，文件被当房间现状。

### D4 拒绝对账 = 建会话但标记不一致（用户裁决）

`hello` 里一旦报了 `repo_id`/`since`，就是一份「按一致处理」的申请；`missed_ops` 不能满足时不再静默欢迎：

- `repo_id` 与服务器库标识不符（含报了 `since` 却报不出 `repo_id`）→ `REPO_MISMATCH`；
- `since > current` 或 `since < 0` → `REVISION_AHEAD`；
- `since` 合法但日志不全（含无库）→ `REPLAY_GAP`。

**会话照开**：`welcome` 正常发（peers/claims 照旧，能看房间）；紧跟着发 `AlignRefused{code, message}`，不发补发（`replay_from=None`）。两个附带语义：

- **被标记的连接不登记 `existing_tags`**：这份文件按对不上的对待，它的位号不该占本房间的名额。
- 之后断线重连会再次对账再次标记——确定性行为，不存在「重拨烧日志」（会话是建立的）。

客户端侧：`AlignRefused` 映射为新 `CollabEvent::AlignRefused{code,message}` → `mark_diverged("需要取得最新图纸：<原因>")`；`diverged` 本就闸住 `pump()`，「不再提交」白得；本地绘图不受影响（继续画、继续排队但不发出）。

**后续（2026-11-17 定）**：单独的 `AlignRefused` 消息只是这一批的落地形式。`docs/plans/2026-11-16-pid-dev-spec/SPEC-collab-offline-sync.md` §10 第 2 条已定：任务 `OFF-2` 把拒绝改写进 `welcome` 的可选键 `align_refused: {code, message}`，去掉这条单独的消息。另外，落地的代码标的原因是「<服务器给的原因>（<码>）」，面板上没有「需要取得最新图纸」这一句，与上一段写的不同；这一句也由 `OFF-2` 补（同一份规格 §5.2、验收 OS-08）。

**`OFF-2` 之后（2026-11-20）**：上面说的形式已经改掉。拒绝写在 `welcome` 的可选键 `align_refused: {code, message}` 里，没有单独的消息了。服务器给这条连接记一个「没对上」的标记，它发来的 `op` 一律回 `error NOT_ALIGNED`（带 `op_id`，不带 `rejected`），不裁决、不写日志、不广播。客户端不再标 `diverged`，而是记下「这次连接对不上」（`State.not_aligned`）：队列不发，房间的修改不写进本图，不申请占用，基线和库标识保持图里原来的，存盘照旧写它们；断线后下一次连接重新核对。重连时房间的修订比本图高而没有补发，客户端也这样处理（码记 `REPLAY_GAP`），以前是标 `diverged`。面板在原因下面加了一行「需要取得最新图纸」，命令行也这样说。另外，只报 `since`、不报 `repo_id` 的 `hello` 不再核对，照旧按日志补发（`SPEC-collab-offline-sync.md` §11.2 的 R3，`OFF-2` 开工时定为放宽服务器），所以第一条括号里的情形不再拒绝。

### D5 客户端种子化：`had_session`/`base_revision`/`repo_id` 从文件回填

`Resuming`→`Connected` 的接管滤网现在是 `had_session && from == base_revision`：纯新进程里 `had_session=false` 会让补发被当成「带 no-replay 的加入」，随后 `OpApplied` 又因 `revision <= base_revision` 被跳过。修法：把文件里的 `PID_AUTHOR_COLLAB` 当成「上一次会话的记忆」回填——`base_revision = rev`、`had_session = true`（这份文件确实进过房间：断线修改该排队正是这个语义）、`repo_id = repo`。

**播种点两个**：`answer_collab_tags`（拨号前，保证 `Resuming` 到达时滤网已就位）与 `broadcast()` 的 `!connected` 分支（兜底「打开就画、拨号未至」的窗口——打开即画的修改同样要排队）。两个点共用一个 `seed_file_state(doc)`，只在 `!had_session` 时生效、只在 `file.drawing == config.drawing_id` 时用。

### D6 队列落盘：`PID_AUTHOR_COLLAB_QUEUE` XRecord

`outbox` 里的全部 `Outgoing`（没发出的和已经发出还在等答复的都写，不按 `sent` 筛；每条取 `op_id`/`base_revision`/`payload`/`label`/`on` 五项，即 `pid::QueuedOp`）逐项序列化成 JSON，与 `PID_AUTHOR_COLLAB` 同一次保存写入；队空删记录。重开后：文件记录对得上 + 队列表在 → 逐项推回 `collab.outbox`（`steps` 置空、`sent=false`），之后的「补发→按序提交」完全走既有链路（`catching_up` 闸住 pump，补完放行）。

**被拒的边界**：重启后没有撤销步可撤——`Refused::account` 空转，被服务器拒的恢复 op 留在图上 → 标 `diverged`（如实，但撤不回）。写入 `SPEC-collab-client.md` §10 已知限制；自动移除被拒不做的理由见 §5。

### D7 离线新建对象回灌 —— 本期不做，列为后续

种子化已覆盖「曾经进过房间」的图：重开后断线期间的新建/删除/改属性照常进队列并落盘。**回灌的真缺口只剩「从没进过房间的图」**：那些对象在文件里但没有 op 可排。要做需要服务器在 welcome 下发「房间已知对象清单」、客户端把本地多出对象合成 insert op（新序列化路径）；离线**删除**无账可查（文件里没那对象了），只能做「新建回灌」半截语义。量不小且只做一半，列为本任务之后的独立任务，不掺进本期。

### D8 `diverged` 无恢复入口（维持边界）

REQ-COLL-014 的既有边界「标成不一致后只能取最新图重开」不变；本任务让它**更早、更准地**被标出来（拒绝对账即标），不新增恢复通道。（`OFF-2` 之后，拒绝对账不再标不一致，只是这次连接不提交，见 D4 末段。）

## 4. 改动清单

| 位置 | 改动 |
| --- | --- |
| `crates/ocs_collab/src/protocol.rs` | `Hello.repo_id`、`Welcome.repo_id`、三个新码 |
| `crates/ocs_collab_server/src/store.rs` | `meta` 表 + `repo_id()`；无库路径由 `AppState` 生成 |
| `crates/ocs_collab_server/src/ws.rs` | `hello` 加三项校验，失败发 `welcome` 紧接 `AlignRefused{code}`、不补发、不登记上报位号；`welcome` 带 `repo_id` |
| `src/app/collab/transport.rs` | `Seen` 换 `repo_id` 键；`hello.repo_id`/`since` 双来源填；`AlignRefused` 帧解析 |
| `src/app/collab/mod.rs` | `State.repo_id`、`seed_file_state`、三码 → `mark_diverged`、`apply(Resuming)` 与种子配套的滤网 |
| `src/app/collab/apply.rs` | `answer_collab_tags`/`broadcast` 调种子化；`Connected` 记 `repo_id` |
| `src/app/update/file.rs` | `save_tab_synchronously_protected_as` 的 snapshot 上写 `PID_AUTHOR_COLLAB`（+`QUEUE`） |
| `src/pid/collab_state.rs`（新文件，由 `src/pid/mod.rs` 导出） | 两个新 key 常量和两条记录的读写函数（不进 `graph_data::keys()` 的四key契约数组——那四个是 P&ID 语义，协同账是另一族；孤儿回收按 `kind=` 区分，不受影响）。`src/pid/graph_data.rs` 没有改 |
| `src/app/collab/collab_offline_queue.rs`（或新 `collab_align.rs`） | 场景测试 |

## 5. 验收映射与不做清单

| REQ-COLL-015 验收 | 覆盖方式 |
| --- | --- |
| 1. 三项随图保存、重开读回不变 | `graph_data` 同型 XRecord + DWG 往返断言（仿 `dwg_roundtrip` 写法，补进 `GRAPH_KEYS` 口径或并列断言） |
| 2. 加入报三项、服务器补发、补齐两端一致 | `hello.repo_id`/`since` + 既有补发链 + e2e 场景（存库、断、改、重连、比对两端位号/属性/拓扑） |
| 3. 三类对不上 → 拒绝+原因+显示「需要取得最新图纸」+不再提交 | 三码 + `AlignRefused` 紧随 `welcome` + `diverged` 闸住 `pump()`；单测 + 服务器进程场景 |
| 4. 不连服务器照常打开/绘制/保存、失败步骤 0 | XRecord 惰性；单测：带记录的图无服务器打开—画—存—记录仍在、无失败步骤 |

**本期不做**：从没进过房间的图的对象回灌（D7）；`diverged` 恢复入口（D8）；被拒不落库的恢复 op 自动回撤（D6 边界）；REQ-COLL-016（身份/登记/加密传输）；`--bind` 之外的部署项。

## 6. 风险与开放点

- **图号对不上的服务器侧兜底**：`hello` 不带单独 `file_drawing` 字段——文件图号与请求图号的比对全部在客户端（D5 的 `file.drawing == config.drawing_id`）。客户端出错时服务器无法复核；如要在协议里再压一道，加 `file_drawing: Option<String>` 让服务器也拒——冗余但便宜，倾向不加。
- **旧协议互操作**：同仓库两端同发，无旧端；外部老客户端报 `since` 无 `repo_id` → 按 `REPO_MISMATCH` 拒（无法核验库）——比现状严，属预期收紧。（`OFF-2` 已放宽：只报 `since` 的不核对，照旧补发，见 D4 末段。）
- **`document_for_save` 快照写 XRecord 的语义**：快照带着 `objects` 整体，`ensure_xrecord` 在其上建字典/记录即被序列化，不回写活文档——与「另存为不弄脏源图」的既有语义一致。
- **`catching_up` 与种子化的次序**：种子必须在 `Resuming` 到达前完成；`answer_collab_tags` 先于 hello 回答，序天然成立。`broadcast` 侧种子是兜底，不依赖拨号节奏。

## 7. 入库时的改正（2026-11-18）

这批改动入库（任务 `OFF-1`）时对着代码改正了本文的四处，代码没有改：

1. D1：记录的键名是 `drawing_id`、`repo_id`、`revision`（原文写的是 `drawing`、`repo`、`rev`）。
2. §0 与 D6：保存时写的是队列的全部条目，已经发出还在等答复的也写（原文写的是只写没发出的）。
3. §4：两个记录名常量在新文件 `src/pid/collab_state.rs` 里（原文写的是 `src/pid/graph_data.rs`）。
4. D4：补了「后续」一段。

文中其余的标识符（如 `told.file_state`、`seed_file_state(doc)`）是动手之前拟的名字，与代码里的不完全相同，以代码为准。核对时发现的问题记在 `docs/plans/2026-11-16-pid-dev-spec/SPEC-collab-offline-sync.md` §11。

## 8. `OFF-2` 之后的改正（2026-11-20）

任务 `OFF-2` 改了拒绝对账的形式和客户端的处理，本文相应改了三处：

1. D4：补了「`OFF-2` 之后」一段。
2. D8：注明拒绝对账不再标不一致。
3. §6：旧协议互操作那一条注明已放宽。

其余各段（包括 §4 的改动清单和 §5 的验收映射）是那批改动当时的说法，没有改。实现记录见 `docs/plans/2026-11-16-pid-dev-spec/SPEC-collab-offline-sync.md` §11.5。
