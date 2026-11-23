# 智能 P&ID 平台 mission（P1 单机绘图闭环 + 多用户协同）：现状整理与 2026-11-10 合并记录

> 日期：2026-11-10
> 范围：Factory droid 跑的 mission「OpenCADStudio 智能 P&ID 平台：P1 单机绘图闭环 + 完整多用户协同」做到了哪一步、还剩什么、记录在案的欠账，以及今天把上游与 `upstream-merge-20261110` 并进 `main`、把中断时的半成品搬到新 `main` 上的经过和验证结果。本文不含新功能。
> mission 目录：`C:\Users\dpc\.factory\missions\ca49c5e7-cf3b-4a0a-a262-90f74b42a894`（missionId `mis_d77ca2e3`）。**这个目录只读，本次没有改过其中任何文件**；此后的进度记在本文。
> 基于：`main` = 本文所在的提交，其父提交是 `dd43f6aa2`（上游 `000814aff` 的合并）。mission 中断时的 `main` 是 `2e9e9dab2`。
> 证据：除标注「推断」「未复核」「mission 记录」的句子外，数字都是今天从 mission 文件、`git log` 和亲自跑的构建与测试里得到的；复算办法见 §12。
> **§1–§12 是第一轮（当天傍晚）结束时的状态。同日晚上的第二轮把 M6 收完了，执行记录在 §13，两处不一致时以 §13 为准。**
> **2026-11-20 阶段 A 结项（§19）：§0 第一条、§3 的里程碑表、§4 的断言表改成了结项时的状态。mission 目录仍然没有改过，这里记的断言状态是「有证据的建议状态」。**

## 0. 一页结论

- **2026-11-20 阶段 A 结项时（§19）**：228 条契约每条都有能重跑的检查，在 `3450558e6` 上全量复验都过了（`CC-7`），建议状态 228/228；mission 的状态文件没有改，仍是 179 passed、49 pending（§4、§19.3）。`ea7f4357b` 之后补上的：`cross-area-flows` 缺的三条（`003` 管线广播、`008` 离线排队、`009` 跨图连接符）；建连时上报本图已有的位号（§18.6 之 1）；属性变更带覆盖标记、改位号带真实来源（审查 N4）；属性超过 2 KB 的对象新建、改属性都广播；服务器 `--bind`；离线使用与上线后同步（`OFF-1` – `OFF-13`）（§19.1）。复验读数：主程序库 7239 条，7182 通过、55 默认不跑、2 失败，失败的是设计稿比对，与协同无关，记为已知问题（§19.5）；`ocs_collab` 98 条、`ocs_collab_server` 187 + 21 条全过；端到端八组 46 个场景加收尾检查共 830 项全过（§19.2）。服务器规格 S2 的四处实现选择确认为保持现状（§19.4）；客户端规格的 C1、C2、C4 与离线规格的 L1 – L12 是已知限制（§19.5）。契约字面与实现有出入、写明要裁决的 6 处（7 条），CP-A 定为都维持现在的实现、按定下的口径算（§19.6）。推送：CP-A 定为先把本地 `main` 合进来、在合并后的提交上重跑门禁，过了再推到 `fork/main`（§19.7）。

- **2026-11-11 交回时（§15–§18）**：独立审查的发现项里 B1、S1–S9、N1–N3、N5–N8 已修，N9 修了大半，N4 没修（§15.2、§17.5）。M7 的 `conflict-field-merge`、`conflict-object-topology`、`conflict-resolution` 三项做完（§14.5）；`cross-area-flows` 十二条里九条成立、三条没做（`003` 管线新建的广播、`008` 离线对账、`009` 连接符跨图），这一项不能记为完成（§18.2、§18.4）。代码的最终尖端 `ea7f4357b`：全量 20 个测试程序 6789 通过 / 0 失败 / 45 忽略，lib 8 遍每遍 6162 / 0 / 43，M6 的八个与 M7 的十一个端到端场景全部通过（§18.5）。要用户拍板的事在 §14.6、§15.6、§17.6、§18.6。

- **第二轮之后（§13）**：M6 的 4 项功能全部提交，`collab-client-broadcast` 在 `360dd0708`。M6 的 20 条断言逐条取了证：13 条成立，7 条部分成立（003、005、009、011、013、018、019；原来写的是 15 条成立、5 条各有一句不成立，2026-11-11 按独立审查更正，见 §13.3 末与 §15）——其中 003、005、013、018、019 与 009 卡在契约自己的写法上，等契约的主人裁决（§13.4），011 是放行名单里有写命令。协同传输测试的隔离已修；全量 20 个测试程序在 `8cd274a2a` 上 6571 通过 / 0 失败 / 43 忽略，并进另一条线最后两个提交之后 6575 / 1 / 43，那 1 条是并行时偶发的测试，单独重跑通过（§13.8）。`Cargo.lock` 的欠账已补，`--locked` 能用了。M7 没有开始。

- **mission 没有在跑。** `state.json` 是 `paused`，最后一条进度记录是 13:48:28 的 `mission_paused`。19:45:45 那次写入是 Factory 桌面端恢复编排会话时刷新了时间戳，之后没有任何新的运行（§2）。
- **进度**：7 个里程碑里 M1–M5 已关闭（实现、代码评审、逐条断言取证都做完）；M6 的 4 项功能做完 3 项并已提交，第 4 项 `collab-client-broadcast` 是工作区里的半成品；M6 的里程碑验证还没跑；M7 的 4 项没有开始。`features.json` 共 57 条：52 completed / 1 in_progress / 4 pending（§3）。
- **断言**：契约 228 条，179 passed、49 pending，没有 failed 或 blocked。pending 的是 `VAL-CLI` 20 条（M6）、`VAL-CONF` 17 条与 `VAL-CROSS` 12 条（M7）（§4）。
- **代码**：`main` 现在 = 上游 `origin/main` 到 `000814aff` + 今天 `upstream-merge-20261110` 上的工作（到 `fc37fbaf2`）+ mission 的全部提交。半成品已原样搬回工作区，只为跟随上游改名改了 18 处路径，仍是未提交的改动（§10）。
- **验证**（mission 的工具链与构建目录）：只含合并时 6465 通过 / 1 失败 / 43 忽略；加上半成品 6483 / 4 / 43。失败的都是协同传输那组测试和两条并行时偶发的既有测试，单独跑全部通过（§10.4）。
- **新查明的一件事**：协同传输那组测试整套跑时失败，主因不是等待时间太短，而是进程内全局的停止标志被别的测试置位后不复位。单线程、机器正在编译的情况下它们 14/14 通过；把 `batch_exit_saves_and_stops` 排到它们前面，单线程也必然失败（§6.1）。
- **下一轮**从修这组测试的隔离开始，接着收尾 `collab-client-broadcast`，再跑 M6 验证，然后 M7（§8）。

## 1. mission 是什么

### 1.1 目标

在现有 OpenCADStudio（Rust + iced 桌面 CAD）上实现 PRD 的两块能力：

1. **P&ID 绘图作者功能**：从调色板插入符号、按规则自动分配位号、类型化属性面板、绘制管线 / 管路 / 管段并维护拓扑、插入阀门时管线自动断开、移动时自动补位、页间连接符、DWG 往返无损。
2. **多用户实时协同编辑**：新建协同服务器（HTTP + WebSocket + SQLite），位号级占用、变更广播、三级权限判定链（角色矩阵 × PBS 节点 ACL 继承 × 图纸状态）、心跳 / 超时 / 接管、冲突检测与裁决、只追加的审计日志。

最终形态（`mission.md` 原话的意思）：多个设计师同时打开同一张图，服务器按位号授予占用、广播变更、冲突可裁决、每一步写审计；整张图存回 DWG 后位号与属性零丢失。

### 1.2 架构上已定的事

| 决策 | 选择 |
|---|---|
| 服务器 | workspace 里两个 crate：`crates/ocs_collab`（协议与纯逻辑，只许 serde / serde_json / uuid / chrono）、`crates/ocs_collab_server`（axum + tokio + rusqlite 的独立二进制） |
| 客户端 | `src/app/collab/`，同步 `tungstenite` + `std::thread` + futures channel，**不引 tokio** |
| 同步单元 | P&ID 语义操作（`set_tag`、`set_property`…），服务器不理解 DWG |
| 每实体语义 | XDATA，APPID `PID_AUTHOR`（与识别器的 `PID_SEMANTICS` 分开），单实体 2 KB 以内，超出挪到 XRecord |
| 图级数据 | 模型空间扩展字典下的 XRecord（类型库、位号规则、拓扑表、连接符表） |
| 变更广播钩子 | `push_undo_entry`（`src/app/history.rs`），所有撤销步的唯一汇聚点 |
| 端口 | 协同服务器 3240，GUI 验证实例 3210–3239；整体只许用 3210–3259 |

### 1.3 明确不做、不许做的事

- 不改 `D:\work\plant-code\cad\iced`（本地 iced fork，只读依赖）；发现它的问题只记录。
- 不用 Docker / Postgres / Redis 或任何没在 `services.yaml` 登记的服务；服务只监听 `127.0.0.1`。
- 不新增实体类型（符号用 `Insert` 承载）；识别器（`src/io/pid_legend/`）是复用，不改造。
- 不引入清单之外的新依赖；需要时回到编排层决定。
- 服务器不做鉴权（本机回环）；多实例部署不在范围内（`revision` 的唯一性只保证单进程，见 §6.2）。
- 被编排层裁定「不属已批准范围」而搁下的：ALIGN 拒绝消息的本地化、`handle_align_selected` 死代码、GUI `/api/v1/save` 失败时的结构化错误码、旋转 / 缩放 / 镜像管件时管段不跟随、打断符只支持直线管段、符号库没有 elbow、占用行不显示剩余时间。
- PRD 里 P1、P3 之外的部分不在这次 mission 里（推断：`mission.md` 只写了这两块，没有单列「不做」清单）。

### 1.4 mission 目录里有什么

| 文件 | 内容 |
|---|---|
| `mission.md` | 目标、架构、七个里程碑、测试与验证策略 |
| `features.json` | 57 条 feature：描述、前置、期望行为、认领的断言、状态、worker 会话 |
| `validation-contract.md` | 228 条断言的正文（判据、工具、证据要求） |
| `validation-state.json` | 每条断言的状态、证据、备注 |
| `AGENTS.md` | 所有 worker 与验证器必须遵守的规则、边界、踩过的坑（721 行） |
| `architecture.md` | 系统设计；附录 A 是契约决策登记，冲突时以它为准 |
| `services.yaml` | 构建、测试、探针命令与服务启停的唯一事实源 |
| `skills\*\SKILL.md` | 五类 worker 的工作流程 |
| `library\` | 每个 feature 交付后的实测事实、裁决记录（`pid-m3-decisions.md`）、探针脚本、补丁 |
| `handoffs\`（39 份）、`validation\`、`evidence\` | 交接单、评审与取证产物 |
| `progress_log.jsonl`（258 行）、`state.json` | 运行日志与状态 |

worker 与验证器用的模型是 `custom:DeepSeek-Flash-1`（`model-settings.json`）。

## 2. 运行状态与时间线

### 2.1 现在是否在跑

判断：**已暂停，没有 worker 在改这棵树。** 依据：

- `state.json`：`"state": "paused"`。
- `progress_log.jsonl` 最后三条：11:57:27 `mission_run_started`（collab-client-broadcast）、11:57:28 `worker_started`、13:48:28 `mission_paused`。之后没有 `mission_resumed` / `mission_run_started` / `worker_started`。
- 19:45:45 的写入：编排会话文件末尾在这一刻多了一条 `SessionStart` 钩子记录（`hookMatcher: "resume"`，跑的是 hindsight 的会话启动钩子），同一秒 `state.json` 的 `updatedAt` 被刷新、`progress_log.jsonl` 被重写但没有新增行。也就是桌面端打开并恢复了这个会话，没有开始新的运行；这条记录之后没有任何 assistant 消息。
- 那个 worker 的会话文件（`fac4a5df…`）最后写于 13:51:00，Factory 桌面端在 13:51:05 重启。
- 主检出：20:01 与 20:03 两次 `git status` 相同，半成品文件最晚的修改时间是 13:40:21，最近 30 分钟没有任何文件变化。
- 当时在跑的 cargo / rustc 进程只有两组，输出目录分别是 `ocs-upstream-20261110\target`（另一条线）和 `pid-parse` 的构建目录，都不是这棵树。
- `droid.exe`（PID 54712，13:51:21 起）是桌面端的守护进程，这段时间在写的是另一个项目（`D:\work\erp`）的会话。

### 2.2 时间线（本地时间）

| 时间 | 事件 |
|---|---|
| 11-02 01:39 | mission 接受；02:22 开始第一轮（当时 43 个 feature、224 条断言） |
| 11-02 全天 | M1–M3 实现；验证顺序是 M3 → M2 → M1，各有一轮不过、修复后通过；22:11 暂停 |
| 11-05 11:19 – 11-08 21:10 | M4：7 项功能 + 1 项修复；代码评审一轮过；用户测试第一轮 2 条不过（契约口径），改契约后第二轮过 |
| 11-08 21:27 – 11-10 00:10 | M5：7 项功能；代码评审三轮（补位没接到真实路径 → 修；撤销不回退拓扑表、ALIGN 谎报 → 修）；用户测试通过 |
| 11-10 00:12 – 11:54 | M6：传输层 02:00、心跳 03:52、占用 11:54（中间 worker 失败一次，10:22 恢复） |
| 11-10 11:57 | 开始 `collab-client-broadcast` |
| 11-10 13:48:28 | mission 暂停；worker 的最后一次工具调用在 13:51:00 |
| 11-10 19:45:45 | 桌面端恢复会话，只刷新了时间戳 |

期间共暂停 13 次、恢复 12 次，worker 中途失败 6 次（都在恢复后补完）。

## 3. 里程碑与功能

57 条 feature = 47 条实现 / 修复 + 10 条验证器（M1–M5 各一条代码评审、一条用户测试）。M6、M7 的验证器条目现在还没有；从进度日志看，它们是在该里程碑最后一项实现完成时生成的。

2026-11-20 起这张表是阶段 A 结项时的状态（§19）。M6、M7 两行和合计行原来是 2026-11-10 的：M6 完成 3、进行中 1，里程碑验证没跑；M7 的 4 项没有开始；合计 42 / 1 / 4，断言 179/228。「建议状态」是有能重跑的检查、复验都过了，但没有经过 mission 的验证器（mission 目录只读，§19.3）。

| 里程碑 | 实现 / 修复 | 完成 | 进行中 | 未开始 | 里程碑验证 | 断言 |
|---|---:|---:|---:|---:|---|---|
| M1 协同服务器骨架 | 7 | 7 | 0 | 0 | 评审 2 轮、用户测试 1 轮，已过 | `VAL-SRV` 30/30、`VAL-INFRA` 14/14 |
| M2 三级权限与审计检索 | 2 | 2 | 0 | 0 | 评审 2 轮、用户测试 1 轮，已过 | `VAL-PERM` 32/32、`VAL-CONF-018` |
| M3 P&ID 语义与位号引擎 | 13 | 13 | 0 | 0 | 评审 2 轮、用户测试 2 轮，已过 | `VAL-SEM` 38/38 |
| M4 绘图作者功能 | 8 | 8 | 0 | 0 | 评审 1 轮、用户测试 2 轮，已过 | `VAL-DRAW` 31/31 |
| M5 管线拓扑与管件 | 9 | 9 | 0 | 0 | 评审 3 轮、用户测试 1 轮，已过 | `VAL-PIPE` 32/32、`VAL-SEM-039` |
| M6 桌面协同接入 | 4 | 4 | 0 | 0 | mission 的验证器没跑；逐条取证见 §13.3，复验见 §19.2 | `VAL-CLI` 20/20（建议状态） |
| M7 冲突检测与裁决 | 4 | 4 | 0 | 0 | mission 的验证器没跑；前三项见 §14.5，`cross-area-flows` 见 §18.2 与 §19.1，复验见 §19.2 | `VAL-CONF` 17/17、`VAL-CROSS` 12/12（建议状态） |
| 合计 | 47 | 47 | 0 | 0 | | 228/228（179 条是 mission 记录的 passed，49 条是建议状态） |

### 3.1 M1–M3（11-02）

mission 自己没有提交这三个里程碑：当时仓库正处在用户发起的上游合并里，worker 只做精确 `git add` 并把补丁导出到 `library\`。代码是随用户的两个提交进 `main` 的：`ff7bc36d9`（11-02 18:58，上游合并提交里带进了已暂存的内容）和 `3598b3c17`（11-04 20:30，「WIP: AVEVA RVM import, collab server ws/perm, P&ID tag engine tweaks」，含 M1 / M2 的修复与 M3 的 D24–D27）。两者都是 `fork/main`（`ccfbcaf26`）的祖先。

| 里程碑 | feature（括号里是认领的断言数） |
|---|---|
| M1 | `val-infra-probe-baseline`(14)、`collab-crate-scaffold`(0)、`collab-http-health-session`(3)、`collab-ws-data-plane`(13)、`collab-audit-append-only`(5)、`fix-m1-http-startup-health`(2)、`fix-m1-ws-data-plane`(7) |
| M2 | `perm-check-chain`(24)、`perm-audit-query`(9) |
| M3 | `pid-type-lib`(3)、`pid-tag-rule-engine`(13)、`pid-xdata-persist`(6)、`pid-xrecord-graph-data`(4)、`pid-symbol-library`(4)、`pid-dwg-roundtrip`(2)、`pid-property-propagation`(3)、`pid-performance`(0)，以及五条修复：`pid-tag-engine-sequencer`(1)、`pid-persist-props-len`(0)、`pid-tag-engine-parse-props`(0)、`pid-type-lib-generation-assertions`(2)、`pid-type-lib-storage-unify`(0) |

### 3.2 M4–M6（mission 自己提交）

| 里程碑 | feature | 断言 | 状态 | 提交 |
|---|---|---:|---|---|
| M4 | `drawing-palette-panel` | 6 | 完成 | `5ba70bef4`（11-05） |
| M4 | `drawing-insert-symbol` | 7 | 完成 | `d412da7ec`（11-05） |
| M4 | `drawing-insert-interaction` | 5 | 完成 | `3cb2b5beb`（11-06） |
| M4 | `fix-m4-run-cancel` | 0 | 完成 | `7d93625e0`（11-06） |
| M4 | `drawing-tag-display` | 5 | 完成 | `f7964d7a2`（11-06） |
| M4 | `drawing-property-panel` | 5 | 完成 | `96b888464`（11-06） |
| M4 | `drawing-performance` | 2 | 完成 | `162fb6105`（11-08） |
| M4 | `drawing-tag-bulk-performance` | 1 | 完成 | `848f31e5b`（11-08） |
| M5 | `pipe-line-run-segment` | 7 | 完成 | `e094239a7`（11-09） |
| M5 | `pipe-property-propagation` | 6 | 完成 | `b4479cb70`（11-09） |
| M5 | `pipe-insert-fitting-split` | 6 | 完成 | `874fe27cf`（11-09） |
| M5 | `pipe-move-fitting-refill` | 4 | 完成 | `6514c6a04`（11-09） |
| M5 | `pipe-connectors` | 5 | 完成 | `ae5c7c662`（11-09） |
| M5 | `pipe-topology-persistence` | 3 | 完成 | `3c4034b85` + `fcf42f4ef`（11-09） |
| M5 | `pipe-break-symbol` | 2 | 完成 | `3b3c1c5fd`（11-09） |
| M5 | `fix-m5-refill-wiring` | 0 | 完成 | `339921db4`（11-09） |
| M5 | `fix-m5-refill-undo-and-align` | 0 | 完成 | `7c501668f`（11-09） |
| M6 | `collab-client-transport` | 5 | 完成，未验证 | `387a53b12`（11-10） |
| M6 | `collab-client-heartbeat` | 4 | 完成，未验证 | `999be9572`（11-10） |
| M6 | `collab-client-claims` | 5 | 完成，未验证 | `2e9e9dab2`（11-10） |
| M6 | `collab-client-broadcast` | 6 | **进行中**（工作区半成品，§7） | — |

另有一个不挂在任何 feature 下的编排层提交：`0e2353778`（「M5 开工准备：新建 src/pid/topology.rs 占位模块并接线」）。`fork/main..2e9e9dab2` 的 34 个 first-parent 提交里，22 个是 mission 的，其余 12 个是同期另一条线的 P&ID 备份浏览与一次上游合并（`7c5619b37`）。

### 3.3 M7（未开始）

| feature | 断言 | 一句话 |
|---|---|---|
| `conflict-field-merge` | `VAL-CONF-001..005` | 不同字段并发修改自动合并；同字段按 `base_revision` 落后方让位、原值进审计；基线相同且值不同才生成冲突条目 |
| `conflict-object-topology` | `VAL-CONF-006..009`、`017` | 一方删除另一方修改时删除生效并通知；拓扑级冲突生成待裁决条目且裁决前双方拓扑不变；删除被引用对象返回 `OBJECT_REFERENCED` |
| `conflict-resolution` | `VAL-CONF-010..016` | `POST /conflicts/{id}/resolve`（采用甲方 / 采用乙方 / 人工改值）、审计、不可重复裁决、非管理员 403、`GET /conflicts` 检索 |
| `cross-area-flows` | `VAL-CROSS-001..012` | 十二条跨 GUI、服务器、持久化的端到端流程 |

## 4. 验证契约与断言

`validation-contract.md` 里 `### VAL-…` 标题共 228 个，`validation-state.json` 也是 228 条，每条恰好被一个 feature 认领。

| 区域 | 前缀 | 条数 | passed | pending | 在哪个里程碑取的证 | 建议状态（2026-11-20） |
|---|---|---:|---:|---:|---|---:|
| 协同服务器骨架 | `VAL-SRV` | 30 | 30 | 0 | M1 | 30/30 |
| 验证基础设施 | `VAL-INFRA` | 14 | 14 | 0 | M1 | 14/14 |
| 三级权限与审计 | `VAL-PERM` | 32 | 32 | 0 | M2 | 32/32 |
| P&ID 语义与位号 | `VAL-SEM` | 39 | 39 | 0 | 38 条在 M3，`039` 在 M5 | 39/39 |
| 绘图作者功能 | `VAL-DRAW` | 31 | 31 | 0 | M4 | 31/31 |
| 管线拓扑与管件 | `VAL-PIPE` | 32 | 32 | 0 | M5 | 32/32 |
| 桌面协同接入 | `VAL-CLI` | 20 | 0 | 20 | —（逐条取证见 §13.3） | 20/20 |
| 冲突检测与裁决 | `VAL-CONF` | 18 | 1 | 17 | `018` 在 M2；其余见 §14.5 | 18/18 |
| 跨区域流程 | `VAL-CROSS` | 12 | 0 | 12 | —（§18.2、§19.1） | 12/12 |
| 合计 | | 228 | 179 | 49 | | 228/228 |

- 状态文件里只有 `passed` 和 `pending` 两种值，**没有 failed，也没有 blocked**。179 条 passed 都带 `evidence` 字段。
- 49 条 pending 的原因只有两类：功能做了、worker 自己端到端跑过，但里程碑验证没跑（`VAL-CLI-001..008`、`014..017`、`019`、`020`，共 14 条）；功能没做完或没开始（`VAL-CLI-009..013`、`018`，以及 `VAL-CONF` 17 条、`VAL-CROSS` 12 条，共 35 条）。mission 的规则是断言状态不得预填，必须由用户测试验证器取证后才能标 passed。
- **2026-11-20（§19.3）**：最后一列是这时加的。passed、pending 两栏是 mission 的状态文件，没有改。228 条每条在 `scripts/collab-e2e/README.md` 第 4 节对到单元测试或场景，`CC-7` 在 `3450558e6` 上复验都过了，证据位置在那张表的「复验证据」一列；没有经过 mission 的验证器，所以记作建议状态。契约字面与实现有出入的 34 条按 README 第 5.3 节记的口径算，其中 7 条在 CP-A 裁决了，都维持现在的实现（§19.6）。

曾经不过、后来修好或改了口径的（都已是 passed，列在这里是因为后面的工作会再碰到同样的坑）：

| 断言 | 当时的问题 | 处置 |
|---|---|---|
| `VAL-SRV-002/009/013/015/016/017` | M1 第一轮评审发现 5 处缺陷，原先的 passed 是假通过（端口冲突的失败启动会清空会话与占用、只读库的 `/health` 假健康、op 修订号读改写竞态、接管通知广播给了第三方、`claim()` 吞掉过期条目） | 全部修复，新增 `VAL-SRV-028/029/030` 锁定回归 |
| `VAL-PERM-017` | `POST /session` 建的会话对权限判定不可见 | 裁决 D31–D33，改读 `sessions` 表并补单人编辑上限 |
| `VAL-SEM-015/018/019` | 契约自相矛盾、缺位号序列断言、类型库两套落盘布局 | 裁决 D24–D27 |
| `VAL-SEM-038` | 两条子断言恒真 | 收窄，另立 `VAL-SEM-039`，M5 把 `gate_save` 接进真实保存路径后通过 |
| `VAL-DRAW-009/013` | 用 `capture` 的 `world_center` 当插入点（它是含位号文字的包围盒中点） | 改契约：位置读 `Insert.position` |
| `VAL-PIPE-019..022` | 补位只挂在 REST 通道，GUI 拖动、MOVE、grip_drag 都没接 | `fix-m5-refill-wiring`、`fix-m5-refill-undo-and-align` |

契约里已登记的口径（取证时照这些来，别按字面）：`VAL-DRAW-029` 是预留空号（M4 实际 31 条）；`VAL-CLI-003` 的第二个观测端可以用 Node 直连 WS 代替第二个 GUI；`VAL-CLI-019` 不禁 `smol`（它是 iced 自带依赖）；`VAL-CLI-008` 截图必须带 `wait_for_frame: true`，窗口截图的 `screen_bounds` 要除以 `scale_factor`；`VAL-DRAW-031` 走批量接口 `pid_insert_batch`。

## 5. mission 的记录与仓库实际不符的地方

1. **M1–M3 的 20 条 feature 标 completed，但 `workerSessionIds` 是空的**，进度日志里也没有对应的 `worker_completed` 与提交号（那一段是编排层自己和 Devin 做的）。代码在 `ff7bc36d9` / `3598b3c17` 里，不是 mission 的提交。`val-infra-probe-baseline`、`collab-crate-scaffold` 两份交接单的 `commitId` 写的是 `no-commit-merge-in-progress…`。
2. **`collab-client-broadcast` 标 in_progress，`currentWorkerSessionId` 指向 `fac4a5df…`**，那个会话已经随桌面端重启而结束。仓库这边它是未提交的改动。
3. **M6 的三条 completed feature，对应的 14 条断言还是 pending**：completed 的意思是 worker 交付并自查，不是验证通过。
4. **前置条件里引用了 5 个不存在的 feature id**：`collab-op-broadcast`、`perm-role-matrix`、`collab-ws-handshake`、`collab-heartbeat-timeout-takeover`、`collab-occupancy-claims`。这些是早期拆分时的名字，后来并进了 `collab-ws-data-plane`、`perm-check-chain`（推断）。M7 的三条 feature 还引用着其中两个。
5. **数量没跟着改**：`state.json` 的 `initialFeatureCount` 是 43，现在 57；契约「总览」表仍写合计 224 条、`VAL-SRV` 27 条、`VAL-SEM` 38 条，实际是 228、30、39。
6. **`pipe-topology-persistence` 只记了第二个提交** `fcf42f4ef`，第一个 `3c4034b85` 没记；`0e2353778` 不属于任何 feature。
7. **`AGENTS.md` 的 Git 一节还是任务开始时的描述**（工作树 313 项改动、合并进行中、不许 commit），后文「Known Pre-Existing Issues」才说 11-08 起可以正常提交。它还把 `D:\Rust\target-ocs-mission` 写成「118.8 GB 共享构建缓存」；今天量的是合并前 32.27 GB、合并后 44.49 GB，而 `D:\Rust\target` 现在只剩一个 `release` 目录、0.01 GB。
8. **M7 的部分地基其实已经在仓库里**（features.json 的前置条件看不出来）：`conflicts` 表与 `resolved_value` 列、`store.rs` 的 `insert_conflict` / `list_conflicts` / 裁决更新、协议 crate 的 `conflict.rs`（`resolve_change`、`commit`）、错误码 `OBJECT_REFERENCED`。还没有的是 `delete_object` 这个 op 类型、`/conflicts` 路由，以及把 `ws.rs` 里对 `base_revision` 的硬校验（不等就回 `STALE_BASE_REVISION`）换成三段判定。
9. **mission 之后 `main` 变了**：合并了 532 个上游提交，`codec` 改名 `document`、`app::document` 改名 `app::tab_document`，mission 文件里引用的行号（如 `src/app/mod.rs:589`、`src/app/view/mod.rs:3639`）大多已经不准，只能当线索用。

## 6. 已知问题、欠账、被推迟的事

### 6.1 协同传输那组测试整套跑会失败（mission 自己新加的测试；本轮没改）

> 第二轮已修（`3c778fdc4`）。除了下面查明的停止标志，还有两处进程级的状态在作怪；数据见 §13.5。

现象：`cargo test` 整套并行跑时，`app::collab::transport::tests` 里有 1–4 条报 `the client never reported a session`；只跑 `collab::` 则 62/62 通过。合并前的 `2e9e9dab2` 上就这样。

会失败的是四条用真 socket 的测试：`queued_claims_and_releases_reach_the_wire_with_the_session_id`、`the_session_emits_heartbeats_carrying_its_session_id`、`quitting_sends_a_close_frame_and_joins_the_thread`、`cancelling_the_subscription_ends_the_thread_promptly`。

原因（今天实测）：`transport.rs` 的 `STOP` 是进程内全局的 `AtomicBool`，`collab::shutdown()` 把它置为 `true` 后不复位（设计如此，因为进程正在退出）。而 `exit_app()` 与 `leave_keeping_recovery()` 都会调 `shutdown()`，同一个测试程序里别的测试只要走到退出路径就把它置位了，例如上游的 `app::commands::fileops::tests::batch_exit_saves_and_stops`（批处理跑到 `EXIT`）。此后再启动的客户端一上来就看到停止标志，连都不连。全仓只有 `quitting_…` 这条测试在自己结尾把它复位。

| 实验（合并后、不含半成品的 lib 测试程序） | 结果 |
|---|---|
| 只跑 `app::collab::transport`，`--test-threads=1`，同时机器在做 `-j 10` 的全量编译 | 14/14 通过，1.27 秒 |
| 再加上 `batch_exit_saves_and_stops`，`--test-threads=1`，用 `--shuffle-seed 1..8` 换 8 种顺序 | 7 种顺序失败 1–4 条，1 种全过 |
| 逐条对照 8 种顺序 | 四条真 socket 测试里，排在 `batch_exit…` 之后的每一条都失败，排在它之前的每一条都通过；没有例外。`quitting_…` 排在后面时自己也连不上，走不到结尾的复位 |

所以负载不是主因。等待窗口确实偏短（等连接是 60 次 × 20 ms 的轮询，共 1.2 秒），机器很忙时可能另外造成超时，但今天没有观察到单纯由它引起的失败。

下一轮的改法（建议，未做）：让 `shutdown()` 在没有登记过传输线程时直接返回，或者把停止标志做成每个客户端一份而不是进程全局；等连接改成等事件到达并给宽裕的上限，不靠固定次数的短轮询。改完后整套并行跑应当不再出现这四条。

### 6.2 mission 记录在案、没有修的（未复核，除非另注）

协同服务器：

- `commit_op` 的修订号唯一性来自进程内的互斥量，不是 SQL 层的比较交换。两个服务器进程共用一个库时会各自发出同一个修订号（mission 实测 12 轮里 11 轮）。M7 的冲突裁决不能假设 `base_revision` 跨进程唯一。
- `/health` 每次调用都取一次写锁；外部持写锁时返回 503 且耗时约 5.5 秒。轮询它的地方超时不要低于 6 秒。
- `POST /session` 建的会话没有心跳续期路径，90 秒后自动脱离占用集合。
- `FIXTURE-PERM.json` 无法表达 `drawings.revision`，`VAL-PERM-018` 靠三次 op 把基线抬到 3。
- `probe-m1-ws-030` 不能在同一个库上重跑；WS 探针要先用默认阈值、再用收紧阈值各跑一轮。

P&ID 语义与拓扑：

- `persist` 的溢出路径只按整条记录长度判一次，其余八键本身超 2048 字节时仍可能整条 EED 被丢；`dwg_roundtrip` 的快照对 props 溢出路径有盲区；`split_props` 会把非字符串的 JSON 标量变成字符串、键序重排。
- 拓扑表在删除管路后不立即清理，要到下一次 `PIDPIPE` / `PIDFITTING` 提交才清（`VAL-PIPE-003` 按意图判过）。
- 移动补位只处理平移；旋转、缩放、镜像管件时管段不跟随。打断符只按直线管段算。
- 符号库 7 个符号，没有 elbow（契约 `VAL-PIPE-006` 的例子用不了，改用 transmitter）。
- 连接符的 `peer` 冒号前必须是数字 `document_id`，契约示例写的是 `PID-1102:2B7`；`PIDPAIR` 是契约里没有、自建的配对命令。
- `CmdResult::AlignSelected` 全仓没有生产者，`handle_align_selected` 是死代码；ALIGN 的拒绝消息没进 `src/locale_catalog.rs`。

GUI 控制面与取证：

- 两个 GUI 实例并存时偶发 GPU panic（`wgpu Surface::configure`，根因在只读的 iced fork，`--safe-mode` 无效）。处置是实例消失时换端口重试一次；启动时优先 `--safe-mode`。
- `capture`：`world_center` 是含位号文字的包围盒中点，不是插入点；`view:"region"` 时返回的矩阵与裁剪后的像素错开约 90 px；`scope:"window"` 的 `screen_bounds` 是物理像素而 `ui_bounds` 是逻辑像素；默认不等帧。
- GUI 的 `/api/v1/save` 失败只回纯文本，没有结构化 `code`。
- 不带 `--new` 启动的验证实例可能被单实例机制转交给已在运行的实例后自己退出（退出码 0、无日志）；带 `--new` 时活动文档可能是没有路径的 Start 页，截图整张空白。
- M5 有三组 GUI 探针脚本没有归档（`VAL-PIPE-001..006/028`、`007..012`、`013..018`），复验要重写。

协同客户端：

- 协同面板的「占用」按钮只对已经在占用列表里的行渲染，对任意位号发起占用要用 `PIDCLAIM <位号>`；占用行不显示剩余时间。

### 6.3 构建与环境

- **`Cargo.lock` 比清单少 4 行**（第二轮已补，`0c9b7a529`，§13.1），所以 `cargo --locked` 必然失败，mission 一律用 `--offline`，每次构建后还原锁文件。今天看到这 4 行是：`OpenCADStudio` 依赖列表里的 `ocs_collab`、`tungstenite`（M6 往 `Cargo.toml` 加了依赖却没提交锁记录），以及外部路径依赖 `e3d-model` 的 `memmap2`、`rayon`。前两行是这个仓库自己的账。
- PATH 上的 `cargo` 是 huacnlee 的分叉版，无视 `CARGO_TARGET_DIR`；必须用工具链目录里的 `cargo.exe`。
- `cargo clippy --workspace --all-targets` 是既有失败（`examples/open_bench.rs`、一个 bench），门禁用 `-p OpenCADStudio` 与两个协同 crate 的 `--all-targets`（mission 记录；合并后没有重跑 clippy）。
- 并发编译时首次 `cargo test` 偶发 rustc 崩溃（只有回溯、没有 `error[E…]`），等并发编译结束后原样重跑。
- 整套并行跑时偶发的既有测试（单独跑、整模块跑都通过；不是 mission 的，也不是合并带来的，合并前的日志里就有）：`io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text`、`app::update::plugin_grips_tests::descriptor_moves_refused_by_the_model_keep_the_last_good_design`；另一条线今天还见过 `app::plugin_host::tests::quad_feed_has_no_v14_auto_tab_per_plugin` 与 `scene::resident_patch_shorter_runs_tests::a_layer_switched_off_inside_a_block_patches_the_references_in_place` 各一次。
- git 的全局代理配置指向 `127.0.0.1:7897`，今天那个端口没有监听，`git fetch` 要临时带 `-c http.proxy= -c https.proxy=` 才通（直连可达）。
- mission 留下的进程：三个 `pwsh`（`D:\scratch\m6-claims\start-e2e.ps1`、`start-ttl.ps1`、`start-e2e2.ps1`，分别起于 06:17、06:47、10:58）还活着；`D:\scratch\m6-broadcast\` 下有三个 `.ocs.lock`。本轮没有动它们。

## 7. 中断时的半成品：`collab-client-broadcast`

> 第二轮已收尾并提交（`360dd0708`）。下面「还空着或有疑问的」1–3 怎么定的见 §13.4；4（新建与删除不广播）仍然如此；5 已改成全部广播；6 里的提交有了，交接单与 library 笔记要写进只读的 mission 目录，没有写，内容并在 §13。种子图那个问题的原因见 §13.6。

认领 `VAL-CLI-009`（A 改位号后 B 的本地文档跟着变）、`010`（双方 `base_revision` 同步）、`011`（只读降级后拒绝写）、`012`（只读在界面上可见）、`013`（在线列表随 `peer_joined` 增长）、`018`（同一用户重复连入是两个独立会话）。

改动量：11 个已跟踪文件 +415 / −10，两个新文件共 843 行，新增 21 条单测。

| 文件 | 内容 | 成形程度 |
|---|---|---|
| `src/app/collab/broadcast.rs`（新，477 行） | 纯函数：撤销步的 `(handle, before, after)` → `set_tag` / `set_property` 变更；`op` 载荷的组装与拆解；把变更写回文档 | 成形，12 条单测 |
| `src/app/collab/apply.rs`（新，366 行） | `broadcast_history_delta`（出站钩子）、`apply_remote_changes`（入站合并，每个远端 op 一个撤销步，合并期间不回灌）、`readonly_refuses_command`（只读白名单） | 成形，6 条单测 |
| `src/app/collab/mod.rs` | `CollabEvent::OpApplied`（带 `op_id` 与 `changes`）、`own_ops`、`pending_remote`、`applied_ops`、`request_op`，`/collab/status` 多三个字段 | 成形 |
| `src/app/collab/messages.rs`、`transport.rs` | `op_applied` 解析带上变更列表；出站 `Outbound::Op`、`next_op_id` | 成形，1 条单测 |
| `src/app/history.rs`、`mod.rs`、`update/mod.rs` | 在 `push_undo_entry` 里调出站钩子；`applying_remote` 标志；收到事件后合并远端变更 | 成形 |
| `src/app/commands/mod.rs`、`command_driver/utilities.rs` | 两处命令入口加只读闸门，拒绝时给错误码 `COLLAB_READONLY`；`PIDTAG <旧> <新>` 整行分发 | 成形 |
| `src/app/commands/pidlegend.rs` | `PIDTAG <旧> <新>` 改位号（一个撤销步），找不到旧位号时错误码 `tag_absent` | 成形，2 条单测 |
| `src/pid/persist.rs` | `PidAuthor::from_entity`（从实体快照读，不碰文档） | 成形 |
| `crates/ocs_collab_server/src/http.rs` | 图纸状态改为只读时向整个房间广播 `readonly_downgrade` | 写了，**没有测试** |

暂停时的自查结果（worker 会话记录，在 `2e9e9dab2` 上）：`check` 与 `clippy -p OpenCADStudio` 通过；`--lib collab::` 62 通过；`--lib pidlegend` 21 通过 2 忽略；`ocs_collab` 68 通过；`http.rs` 改完之后只 `build` 过，没跑服务器测试。今天搬到合并后的 `main` 上，这几组结果一样（§10.4）。

端到端探针（`D:\scratch\m6-broadcast\probe-broadcast.ps1`）最后一次运行：准备步骤、`VAL-CLI-013`（两条）、`VAL-CLI-018`（两条）通过；然后在「找到带位号的实体」这一步失败——种子图纸上句柄 391 的 `PID_AUTHOR` 读回是空的——所以 `VAL-CLI-009..012` 一条也没跑到。worker 正在写 `seedcheck.ps1` 查这件事时被暂停。原因未知，可能是探针取数的写法，也可能是种子图保存 / 重开后记录确实不在。

还空着或有疑问的（读代码得出，推断，没有端到端证据）：

1. **`VAL-CLI-011` 的后半句多半过不了。** 契约要求降级后编辑权持有者 `u1` 仍能写。现在服务器把 `readonly_downgrade` 发给房间里的每个会话，客户端收到就无条件 `readonly = true`，持有者自己也被锁住。要么服务器只发给非持有者，要么客户端认出 `holder` 是自己时不降级。
2. **`VAL-CLI-012` 只做了一半。** 协同面板的横幅会显示「只读」，但 `readonly_holder` 只进了状态与 `/collab/status`，没有投影到面板，「编辑权持有者被标出」还没有。
3. **`VAL-CLI-010` 的最后一句多半过不了。** 它要求 `sessions` 表两行的 `base_revision` 都是 2；服务器的 op 分支在提交后只发布 `op_applied`，没有把新修订号写回任何一行会话记录（`sessions.base_revision` 只在建会话、接管、退场、失效这些时刻随整行写入）。需要服务器在 op 生效后刷新同图活跃会话的这一列。
4. 新建或删除的实体不广播（`change_of` 对缺一侧镜像的情况返回 `None`）。六条断言不要求，但 `VAL-CROSS-003`（插阀门后双方拓扑一致）会需要。
5. 一个撤销步里同一实体只取一处变更（位号优先，其次第一个不同的属性）。
6. 交接单、`library\collab-client-broadcast.md`、提交，都还没有。

## 8. 剩下的工作，建议顺序

> 第二轮做完了下表的 0、1、2 三行和「穿插要处理的」两件事；M6 验证里卡在契约写法上的 5 条见 §13.4。剩下的从第 3 行开始，开工前要先知道的事在 §13.10。

规模是推断，依据是前面同类 feature 的体量（M6 已交付的三项各 550–1400 行、worker 净用时 2–4 小时）。

| 顺序 | 工作 | 依赖 | 规模 | 断言 |
|---:|---|---|---|---|
| 0 | 修协同传输测试的隔离（§6.1），让整套并行跑稳定 | 无 | 小（几十行） | 不对应断言；M6 门禁 `--lib collab::` 与全量回归受益 |
| 1 | 收尾 `collab-client-broadcast`：定 §7 的 1–3 三点；面板标出持有者；给 `http.rs` 的广播补测试；查清种子图的 `PID_AUTHOR`；跑通端到端；写 library 笔记并提交 | 0（可并行） | 中小（代码大半已有） | `VAL-CLI-009..013`、`018` |
| 2 | M6 里程碑验证：代码评审 + 逐条取证，不过的建修复项 | 1 | 以往每个里程碑 1–3 轮 | `VAL-CLI` 全部 20 条 |
| 3 | `conflict-field-merge`：把 `ws.rs` 的基线硬校验换成三段判定，接上已有的 `conflict.rs` 与 `insert_conflict`，被覆盖的原值进审计 | 服务器侧，不依赖 2（mission 的排法是在 M6 之后） | 中 | `VAL-CONF-001..005` |
| 4 | `conflict-object-topology`：`delete_object`、对象级冲突的通知与审计、拓扑级待裁决条目、`OBJECT_REFERENCED` | 3 | 中到大（服务器要有「对象被谁引用」的概念） | `VAL-CONF-006..009`、`017` |
| 5 | `conflict-resolution`：`/conflicts` 的检索与裁决路由、管理员权限、审计、不可重复裁决 | 4 | 中（存储层函数已有） | `VAL-CONF-010..016` |
| 6 | `cross-area-flows`：十二条端到端流程，缺什么补什么 | 1、5 | 大，且不确定：`008`（断线期间离线编辑、重连对账）、`010`（并发插入的位号唯一性由服务器保证）、`003`（插阀门后双方拓扑一致）很可能要新功能 | `VAL-CROSS-001..012` |
| 7 | M7 里程碑验证 | 3–6 | 同 2 | `VAL-CONF` 17 条、`VAL-CROSS` 12 条 |
| 8 | 收尾：README、最终全量门禁（编排层待办清单的最后一项） | 7 | 小 | — |

穿插要处理的：`Cargo.lock` 的两行自家欠账（§6.3）；另一条线在 `upstream-merge-20261110` 上 `fc37fbaf2` 之后的提交要再并进 `main`（`main` 已经多了合并提交，不能再快进）。

## 9. mission 约定的工作方式（后面照这个来）

### 9.1 构建与测试

每条 cargo 命令都带固定工具链与独立构建目录，用工具链目录里的 cargo，不用 PATH 上那个：

```powershell
$env:RUSTUP_TOOLCHAIN = 'nightly-2026-10-25-x86_64-pc-windows-msvc'   # rustc 1.101.0-nightly
$env:CARGO_TARGET_DIR = 'D:\Rust\target-ocs-mission'
$c = 'D:\Rust\.rustup\toolchains\nightly-2026-10-25-x86_64-pc-windows-msvc\bin\cargo.exe'
& $c check  -p OpenCADStudio --offline
& $c clippy -p OpenCADStudio --offline
& $c test   -p OpenCADStudio --offline --lib collab::
& $c test   -p ocs_collab --offline
git checkout -- Cargo.lock        # 每组 --offline 命令之后立刻还原，绝不提交
```

- 可执行文件在 `D:\Rust\target-ocs-mission\debug\`（`OpenCADStudio.exe`、`ocs_collab_server.exe`）；仓库里 `.\target\debug\` 的是 PATH 上那个 cargo 的旧产物，不要用，也不要停掉在那里运行的实例（可能是用户自己开的）。
- worker 自查范围：`check -p <crate>` + `clippy -p <crate>` + 相关的一个测试目标；不跑 `--workspace` 的全量测试。
- 里程碑门禁：`check --workspace`；`clippy -p OpenCADStudio` 与 `clippy -p ocs_collab -p ocs_collab_server --all-targets`；该里程碑指定的测试目标（M6：`--lib collab::` + `-p ocs_collab`；M7：`-p ocs_collab` + `-p ocs_collab_server`）；`python -m pytest scripts/pid-platform -q`（558 条）。
- 其余基线：`--lib pid::` 96 通过 / 1 忽略；`--test pid_import` 54；`--test pid_legend` 29；`ocs_collab` 68；`ocs_collab_server` 100 + 11。今天合并后这些数都没变。
- 协同服务器必须先 `build` 再直接起 exe（`cargo run` 的进程号是 cargo 自己的，杀掉后真正的服务器成孤儿占着 3240）。
- 测试产物与证据一律放 `D:\scratch\`（`logs\`、`shots\`），不进仓库。

### 9.2 一项功能怎样算完成

1. 读 `features.json` 里这一项，把 `fulfills` 指向的断言逐条读清。
2. 新行为先写会失败的测试，再实现；测试不依赖网络和固定端口（用 `127.0.0.1:0`）。
3. 自查四条命令通过，还原 `Cargo.lock`。
4. 需要时做端到端：直接起服务器与 GUI 实例，按该 feature 自己那几行核对项验证，验证完杀掉自己起的进程。
5. 只暂存自己改的文件（绝不 `git add -A` / `git add .`），提交前看 `.git\MERGE_HEAD`、`CHERRY_PICK_HEAD`、`sequencer` 是否存在；存在就不提交，改为导出补丁。
6. 写交接单：做了什么、没做什么、跑过的命令与观察、交互核对、新增的测试、发现的问题（分 blocking / non_blocking / suggestion）；把实测事实与踩过的坑写进 `library\<feature>.md`。
7. 编排层逐条处置交接单里的问题：写进 `AGENTS.md` / 契约 / library，或者建 `fix-*` feature。

### 9.3 提交说明

中文。标题是「M<n> <主题>：<要点>」，例如「M6 客户端占用申请与显示：PIDCLAIM/PIDRELEASE、协同面板、占用着色」；正文用短横线列出改了什么，最后一段写端到端实测结果。droid 的提交带 `Co-authored-by: factory-droid[bot] …` 尾注。

### 9.4 里程碑验证

一个里程碑的实现全部完成后，先跑代码评审验证器（逐个 feature 评审 + 门禁命令），再跑用户测试验证器（按断言逐条取证，把状态与证据写进 `validation-state.json`）。任何一关不过就建修复 feature、重跑那一关。工具：服务器 HTTP 用 `curl.exe`；服务器 WS 用 Node 的 `.mjs` 脚本；GUI 用 `OpenCADStudio.exe <dwg> --http <port>` 加 REST（`/api/v1/run` 带显式 `document_id`、`/api/v1/capture`、`/api/v1/input`）。断言没有证据不得标 passed。最多 5 个验证器并发，Rust 编译与测试实际是串行的。

客户端协同代码的硬约束（改 `src/app/collab/` 前必读，出处 `AGENTS.md`「客户端线程模型硬约束」）：建连后设 100 ms 读超时；读超时当正常分支；收到 `Close` 立即退出循环；所有读错误都当断开；用 `TcpStream::connect_timeout` 加 `client()`，不用 `tungstenite::connect`；`close` 之后要 `flush`；线程里只能 `try_send`，并区分通道满与通道断开；订阅的 worker 必须是裸 `fn`。

## 10. 2026-11-10 的合并与验证

### 10.1 做了什么

| 步骤 | 结果 |
|---|---|
| 备份半成品 | 见 §11；补丁在干净的 `2e9e9dab2` 上还原出的 11 个文件与工作区逐字节相同（SHA-256），补丁应用后的树与 `git stash create` 的树同为 `7ddbb467…` |
| 半成品落成临时提交 | `backup/m6-wip-pre-merge-20261110` = `170aefb24`（13 个文件，不含 `.tgrep/`） |
| 取远端 | `origin/main`：`1f18cbbc1` → `000814aff`，新增 22 个提交；`fork/main` 没动（`ccfbcaf26`） |
| `main` 快进 | `2e9e9dab2` → `fc37fbaf2`（`upstream-merge-20261110` 当时的尖端：上游合并 `862ddb1e1` 加 7 个 P&ID 备份面板与文档提交） |
| 合并上游新提交 | `dd43f6aa2`「Merge upstream origin/main (000814aff) into the P&ID fork」 |
| 半成品搬回 | `git cherry-pick -n` 后取消暂存，仍是 11 个已修改文件加 2 个未跟踪文件 |

`upstream-merge-20261110` 那个分支与 worktree 没有被改动；那条线之后的提交需要再合并进 `main`。

### 10.2 上游 22 个提交的合并

内容：线型管理器（`-LINETYPE` 新建 / 改名 / 删除、`.lin` 写出、管理器界面）、ACIS 转换搬进新 crate `ocs_acis`（`opencadkernel` `d554319` → `802f751`，特性 `acis` → `brep`）、文字的 positioned glyph 共享与缓存上限、CHAMFER 的 Multiple / Polyline、VPCONTROL、注释性标注保存后保持大小、打印时大圆弧保持圆滑、三个基准测试。107 个文件，+14019 / −471。

冲突 3 个文件、4 处：

| 文件 | 两边各改了什么 | 取法 |
|---|---|---|
| `Cargo.lock` | 上游加 `ocs_acis`；fork 有 `ocs_collab`、`ocs_collab_server` | 都保留 |
| `src/app/update/mod.rs` | 上游加 `linetype_manager` 模块；fork 把 `file` 模块放宽为 `pub(in crate::app)` | 都保留 |
| `src/scene/text/ttf_glyph.rs`（两处） | fork 的 `build_shaped` 只在 `font_system` 锁内整形，轮廓展平与三角化在锁外做；上游在自己的锁内循环里加了 positioned glyph 缓存，`glyphs` 变成 `Vec<Arc<PlacedGlyph>>` | 保留 fork 的结构，把上游的缓存接进锁外的循环：`ShapedGlyph` 多带 `font_id`、`font_weight`，缓存键与上游逐字段相同 |

依赖：`opencadkernel` 的新版本联网取了一次（`cargo fetch`，给 cargo 调用的 git 临时去掉代理）。

拿不准的一处：`ttf_glyph.rs` 的取法是把两边的优化叠在一起，而不是二选一。缓存有自己的锁，只在查找与写入时持有，轮廓计算仍在所有锁之外；缓存只共享「字体、字重、face、字形、缩放、落点」逐位相同的字形，结果应与不共享时逐位一致。文字相关的测试都过了，但没有专门量过它对出图耗时与逐字节结果的影响。

### 10.3 半成品搬回时的冲突

只有一个文件有文本冲突：

| 文件 | 两边各改了什么 | 取法 |
|---|---|---|
| `src/app/command_driver/utilities.rs` | 同一条「或」条件链的末尾，上游加了 `ATTSYNC <块名>` 整行分发，半成品加了 `PIDTAG <旧> <新>` 整行分发 | 两条都保留，上游的在前 |

其余 10 个已跟踪文件自动合并。为跟随上游改名改了 18 处，别的一个字没动（逐行比对：把改名还原后，搬回前后的增删行完全相同）：

- `codec::` → `document::` 17 处：`apply.rs` 6、`broadcast.rs` 3、`pidlegend.rs` 7、`persist.rs` 1。
- `crate::app::document::DeltaSnapshot` → `crate::app::tab_document::DeltaSnapshot` 1 处（`apply.rs`）。

上游改了签名的那几个函数，半成品都没有调用。

### 10.4 验证

用的是 mission 的约定：工具链 `nightly-2026-10-25`（rustc 1.101.0-nightly）目录里的 cargo，`CARGO_TARGET_DIR=D:\Rust\target-ocs-mission`，`--offline`，`-j 10`。测试是 `cargo test --workspace --no-run` 之后逐个运行 20 个测试程序（另一条线今天用 stable 1.99.1 时是 18 个；多出的两个是上游新 crate `ocs_acis` 的单测与集成测试）。

| 状态 | `build --workspace` | `test --workspace --no-run` | 20 个测试程序合计 | 失败的 |
|---|---|---|---|---|
| 只含合并（`dd43f6aa2` 的内容，提交前） | 通过，356 秒 | 通过，274 秒 | **6465 通过 / 1 失败 / 43 忽略** | 传输组 1 条 |
| 合并 + 半成品 | 通过，144 秒 | 通过，186 秒 | **6483 通过 / 4 失败 / 43 忽略** | 传输组 3 条 + `filled_ttf_glyphs_export_as_real_text` |

对照：另一条线在 `925728489` 上的全量结果是 6313 / 3 / 43（3 条都是传输组）。多出来的通过数来自上游 22 个提交与备份面板后续提交新增的测试。合并 + 半成品比只含合并多的 18 条通过，是半成品自己的 21 条新单测扣掉这一次多失败的 3 条。

失败项的归类：

| 测试 | 归类 | 依据 |
|---|---|---|
| `app::collab::transport::tests::` 下的四条真 socket 测试（每次整套跑失败 1–4 条） | mission 自己的测试隔离问题，合并前就有；不是合并或半成品带来的 | §6.1；单独跑 14/14，`collab::` 整组 62/62 |
| `io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text` | 并行时偶发的既有测试 | 单独跑 3/3、整模块 20/20 通过；另一条线合并前在 `2e9e9dab2` 上的一次运行里就失败过 |
| `app::update::plugin_grips_tests::descriptor_moves_refused_by_the_model_keep_the_last_good_design`（后面多跑的三遍里出现一次） | 并行时偶发的上游测试 | 单独跑 3/3、整模块 43/43 通过；`2e9e9dab2` 与上游都有这条测试 |

合并 + 半成品的 lib 测试程序整套多跑了三遍，每遍都是 5911 通过 / 4 失败 / 41 忽略：传输组 3–4 条，外加至多一条上表里的偶发项。

半成品相关的分组，在合并后的 `main` 上与暂停前在 `2e9e9dab2` 上的结果一致：

| 分组 | 结果 |
|---|---|
| `--lib collab::` | 62 通过 |
| `--lib pidlegend` | 21 通过 / 2 忽略 |
| `--lib pid::` | 96 通过 / 1 忽略 |
| `ocs_collab` | 68 通过 |
| `ocs_collab_server` | 100 + 11 通过 |
| `--test pid_import` / `pid_legend` | 54 / 29 通过 |
| `python -m pytest scripts/pid-platform -q` | 558 通过 |

没有做的验证：`clippy`；快进之后、合并上游新提交之前那个状态（`fc37fbaf2`）没有单独构建，它的依据是另一条线自己的测试记录；任何要开窗口的端到端验证（GUI 实例、协同服务器、双实例）；wasm 目标的检查。

## 11. 备份与回退

| 位置 | 内容 |
|---|---|
| `D:\work\plant-code\cad\ocs-mission-backup-20261110\` | `git-status-short.txt`、`HEAD.txt`、`m6-wip.worktree.patch`（合并前的半成品补丁，基于 `2e9e9dab2`）、`m6-wip.index.patch`（空，当时没有暂存内容）、`untracked\`（两个新文件的原样拷贝）、`wip-sha256.txt`；搬回之后的版本：`m6-wip.post-merge.worktree.patch`、`untracked-post-merge\`、`wip-post-merge-sha256.txt`；`logs\`（构建与测试日志、运行脚本）、`analysis\`（分析脚本与各目录体积） |
| 分支 `backup/m6-wip-pre-merge-20261110` | `170aefb24`：`2e9e9dab2` 加半成品的全部 13 个文件 |
| `refs/backup/m6-wip-20261110` | `b3601266d`：`git stash create` 的快照（合并前，只含已跟踪文件的改动） |
| `refs/backup/main-pre-merge-20261110` | `2e9e9dab2`：合并前的 `main` |
| `refs/backup/m6-wip-post-merge-20261110` | `fda58d013`：搬回并改名之后的快照（基于 `dd43f6aa2`，只含已跟踪文件的改动） |
| `refs/backup/main-post-merge-20261110` | 本文所在的提交，即合并后的 `main`（回退之后还想回来时用） |

回到合并前的样子（`main` 在 `2e9e9dab2`，半成品是未提交改动）。这两条会丢掉主检出里当前未提交的内容，那些内容在上表里都有备份：

```powershell
git -C D:\work\plant-code\cad\OpenCADStudio reset --hard backup/m6-wip-pre-merge-20261110
git -C D:\work\plant-code\cad\OpenCADStudio reset --mixed refs/backup/main-pre-merge-20261110
```

第一条把分支、索引、工作区都放到「`2e9e9dab2` + 半成品」，第二条把分支指回 `2e9e9dab2` 而保留工作区，两个新文件随之变回未跟踪。`main` 没有 push 过，这样做不影响任何远端。

## 12. 数字的出处与复算

- feature 的数量与状态、每条断言的认领关系、断言状态分布：`features.json` 与 `validation-state.json`。分析脚本在备份目录的 `analysis\`：`mission_report.py features | fulfills | vstate | timeline`、`check_features.py`、`handoff_issues.py issues | dismiss`。
- 断言总数与分区：数 `validation-contract.md` 里以 `### VAL-` 开头的标题。
- 时间线与提交号：`progress_log.jsonl` 的 `worker_completed` 记录（时间是 UTC，本文换成了本地时间）；与 `git log --first-parent fork/main..2e9e9dab2` 对照。
- 暂停时 worker 跑过的命令与结果：会话文件 `C:\Users\dpc\.factory\sessions\-D-work-plant-code-cad-OpenCADStudio\fac4a5df-….jsonl`（`analysis\session_grep.py`、`session_tail.py`）。
- 构建与测试：备份目录 `logs\` 下的 `s2-*`（只含合并）、`s3-*`（合并 + 半成品）、`reruns\`、`transport-experiments\`；`run-step.ps1`、`run-tests.ps1`、`sum-tests.ps1` 可以原样重跑。
- 另一条线的数字（6313 / 3 / 43、合并前后的偶发失败）：`D:\work\plant-code\cad\ocs-upstream-20261110\target\stage1-logs\` 下的 `final-test.log`、`base-test-lib-run*.log`、`merged-test-lib-run*.log`，只读。

## 13. 执行记录：2026-11-10 第二轮（把 M6 收完）

这一节是 mission 进度的记账处：`C:\Users\dpc\.factory\` 仍然只读，这一轮没有往里写任何东西。§13.2 与 §13.3 两张表照 `features.json`、`validation-state.json` 的字段写，以后要同步回去可以照抄。前面各节是第一轮结束时的状态，与本节不一致的地方以本节为准。

本节的证据有两处：

- `E` = `D:\scratch\m6-validation\`：端到端验证的脚本（`m6h.py` 是脚手架，`t*.py` 一个场景一个文件，`run_all.ps1` 顺序跑）、种子数据 `seed-val-cli.json`、每个场景的结论 `logs\<场景>.result.txt`、原始取证 `logs\`、截图 `shots\`。
- `B` = `D:\work\plant-code\cad\ocs-mission-backup-20261110\logs\`：构建与测试日志（本轮的以 `r2` 开头）。

### 13.1 提交

`main` 从 `60ac81600`（第一轮的文档提交）走到本节所在的提交，全部在本地，没有推送。

| 提交 | 内容 |
|---|---|
| `111b22eb6` | 合并 `upstream-merge-20261110`（到 `3ccc72782`）：备份面板的后续两个提交，无冲突 |
| `0c9b7a529` | `Cargo.lock` 补上 4 条依赖边：`OpenCADStudio` → `ocs_collab`、`tungstenite`，外部路径依赖 `e3d-model` → `memmap2`、`rayon`。补之前 `cargo metadata --locked --offline` 退出码 101，补之后 0；此后的构建一律带 `--locked`，锁文件不再被改写 |
| `3c778fdc4` | 协同传输测试的隔离（§13.5） |
| `90a1695e4` | `ttf_glyph.rs` 合并取舍的核对（§13.7） |
| `070b9cde2` | 合并 `upstream-merge-20261110`（到 `f4fe14604`）：备份面板的后续四个提交，无冲突 |
| `360dd0708` | `collab-client-broadcast`：变更广播与只读降级（§13.2、§13.4） |
| `412b1d3f2` | 合并 `upstream-merge-20261110`（到 `c2a7db496`）：隐藏桌面上驱动编辑器的脚本与说明，无冲突 |
| `8cd274a2a` | 协同订阅不再写异步块，`VAL-CLI-019` 的检索式从 1 处命中变成 0（§13.3） |
| `6b2531199` | 这份执行记录 |
| `f8b28dc38` | 合并 `upstream-merge-20261110`（到 `36eeb97fe`）：那条线按根因修了并行时偶发失败的六条测试，文本预览多遮几处口令；无冲突 |
| `656e2bd07` | 上面这次合并之后的验证（§13.5 的补记、§13.8） |
| `9b56f216e` | 合并 `upstream-merge-20261110`（到 `23eae5d00`）：备份面板的分隔条可拖动、选中行自动滚入视野；无冲突。合并后 lib 测试整套并行一遍 6006 通过 / 0 失败 / 41 忽略，`collab` 分组 86 通过；全量 20 个测试程序与端到端没有在它上面重跑 |
| 本节最后一次改动所在的提交 | 上面这一行 |

每次合并之后都跑了备份面板的两个测试分组。五次依次是：`pid_backup` 136 通过 / 2 忽略、149 / 2、149 / 2、154 / 2、159 / 2；`pidbackup` 前四次 21 通过 / 1 忽略，第五次 24 / 1。多出来的是那条线新加的测试。那条线还在往前走，本节写完之后它再有新提交，要再并一次。

### 13.2 feature（照此同步回 `features.json`）

| feature id | 状态 | 提交 | `fulfills` | 说明 |
|---|---|---|---|---|
| `collab-client-broadcast` | `in_progress` → `completed` | `360dd0708` | `VAL-CLI-009`、`010`、`011`、`012`、`013`、`018` | 没有 droid 的 worker 会话，`completedWorkerSessionId` 无值可填；原先的 `currentWorkerSessionId`（`fac4a5df…`）应清空 |
| `collab-client-transport` | `completed`（不变） | 追加 `3c778fdc4`、`8cd274a2a` | `VAL-CLI-019` 等（不变） | 两个提交都是对这项已交付功能的修补：测试隔离、去掉异步块 |

不属于任何 feature 的两个提交：`0c9b7a529`（锁文件）、`90a1695e4`（合并核对）。

`collab-client-broadcast` 交付时与半成品（§7）不同的地方：

1. **对象按位号命名，不按实体句柄。** 半成品的 `object_ref` 是十六进制句柄；`architecture.md` §3.2 与契约里的每个例子（`VAL-CONF-001`、`VAL-PERM-030`、`/audit?tag=…`）写的都是位号，服务器的审计与 M7 的冲突判定也按它。现在 `set_tag` 用改名前的位号、`set_property` 用当前位号，句柄只作为 `handle` 提示随行：接收方只有在该句柄下的实体确实带着这个位号时才采信，否则按位号找唯一的那个符号。这样没有图纸的客户端（契约里的 Node 脚本）发来的变更，GUI 也能落到符号上。被替换的值用 `old_value`（服务器审计行读的键）。
2. **撤销与重做也广播。** 它们不经过 `push_undo_entry`，半成品里改回去的位号不会传给别人。
3. **一步里同一个符号的全部变化都广播**（半成品只取第一处）；`props` 溢出到 XRecord 的那一步不比属性，免得把「读不到」广播成「全删了」。
4. **找不到对象的远端变更会报出来**（`last_apply_error` 与命令行），半成品是悄悄跳过。
5. §7 里列的三处缺口：见 §13.4 的 1–3。

### 13.3 M6 的 20 条断言（照此同步回 `validation-state.json`）

取证都在 2026-11-10 23:36–23:48，用的是 `8cd274a2a` 的源码构建出来的两个可执行文件（`D:\Rust\target-ocs-mission\debug\OpenCADStudio.exe`、`ocs_collab_server.exe`）。GUI 实例起在单独的 Windows 桌面上，只走 `--http` 的本机 REST 通道；每个场景开始前和结束后各记一次指针位置与前台窗口，每一次都没有变，自己起的进程也都关掉了（`E\logs\run_all.summary.txt`，各场景 `result.txt` 的末尾）。

与契约前置条件的出入：图纸用的是 `E\work\` 下的副本（两个实例各一份，来自同一张种子图，§13.6）；实例带 `--new-instance --backend vulkan`，不带 `--new --log info`（给了图纸时 `--new` 本来就不起作用；`--new-instance` 是为了既不把图纸转交给别的编辑器、也不去占单实例端口——当晚机器上有另一条线的两个编辑器实例在跑）；服务器用构建好的可执行文件加 `--seed`，库文件是每个场景自己的。端口照契约：服务器 3240，GUI 3210 / 3211。

这些取证是实现者自己做的。mission 的规矩是由独立的验证器取证后才把断言标成 `passed`，所以下表的「建议状态」只是建议。

| 断言 | 建议状态 | 结论与要点 | 证据（都在 `E\` 下） |
|---|---|---|---|
| `VAL-CLI-001` | passed | 服务器日志一条入站 hello（u1、PID-1101）；客户端收到 welcome；`sessions` 新增 1 行，活跃、`base_revision=0` | `logs\t20_session.result.txt`、`t20-001-status.json`、`t20-001-sessions.json`、`t20-001-welcome-frame.json`、`t20-server-log.txt` |
| `VAL-CLI-002` | passed | 60 秒内入站 heartbeat 4 条；`last_heartbeat` 递增 4 次，相邻差 16、15、15、15 秒 | `logs\t20_session.result.txt`、`t20-002-heartbeat-lines.txt` |
| `VAL-CLI-003` | 见 §13.4 之 5 | 挂起 u1 后 88 秒 u2 收到 `peer_left`（u1 的会话），`sessions` 该行变失效，u2 的在线列表 2 → 1；第二观测端按裁决用普通 WS 客户端。裁决条目 2 的前半句「u2 侧收到 `peer_joined`（u1）」不成立 | `logs\t23_stale.result.txt`、`t23-u2-welcome.json`、`t23-u2-frames.txt`、`t23-sessions-before.json`、`t23-sessions-after.json`、`t23-server-log.txt` |
| `VAL-CLI-004` | passed | `PIDCLAIM P-1001` → `claim_granted`；`claims` 新增 1 行，`expires_at` 距当前 300 秒 | `logs\t21_claims.result.txt`、`t21-004-claims.json`、`t21-server-log.txt`、`t21-u1-collab-log.txt` |
| `VAL-CLI-005` | 见 §13.4 之 6 | u2 收到 `claim_denied`，带 `expires_at`；`claims` 仍只有 u1 那 1 行；u2 的面板与 `/collab/status` 显示占用者是 u1，命令行有占用失败的警告。帧里的 `holder` 是 u1 的会话号，不是契约写的「显示名或用户 id」 | `logs\t21_claims.result.txt`、`shots\t21-005-u2-denied.png`、`logs\t21-u2-collab-log.txt` |
| `VAL-CLI-006` | passed | 默认 TTL。到期那一秒 `claims` 行数 1 → 0；u1、u2 各收到一条 `claim_released reason=expired`；服务器日志有 TTL 回收行 | `logs\t22_ttl.result.txt`、`t22-server-log.txt`、`t22-u1-collab-log.txt`、`t22-u2-collab-log.txt` |
| `VAL-CLI-007` | passed | `PIDRELEASE P-1001` → 双方收到 `claim_released reason=released`；`claims` 该行删除 | `logs\t21_claims.result.txt`、`t21-server-log.txt` |
| `VAL-CLI-008` | passed | 窗口截图里面板占用行显示 P-1001 与 u1，符号与位号是占用色；`ui_bounds` 有 `pid-collab-row-P-1001`；status 的 `claims` 是 `{"P-1001":"u1"}`。占用色像素在视口截图上量：占用前 0，占用后 311（为什么不在窗口截图上量，见 §13.4 之 7） | `shots\t21-008-u1-claim.png`、`shots\t21-008-u1-viewport-before.png`、`shots\t21-008-u1-viewport-claim.png`、`logs\t21-008-capture-window.json`、`t21-008-capture-viewport.json`、`t21-008-ui-bounds.json` |
| `VAL-CLI-009` | 部分成立（2026-11-11 按审查 S9 更正，原记 passed） | u1 执行 `PIDTAG P-1001 P-1009`；双方收到同一条 `op_applied`（`op_id` 相同，修订 0 → 1）；u2 的 `PID_AUTHOR.tag` 读回 P-1009；两张截图的位号文字都是 P-1009。正文里「双方均确认收到对方的 `peer_joined`」不成立：后连入的 u2 没有收到关于 u1 的 `peer_joined`（它从 `welcome.peers` 得知 u1），`t10-u2-collab-log.txt` 里没有这一行，脚本当时也没有检查这一句——与 018 那一句是同一件事（§13.4 之 5） | `logs\t10_broadcast.result.txt`、`t10-009-u1-run.json`、`t10-009-u2-xdata.json`、`shots\t10-009-u1.png`、`shots\t10-009-u2.png`、`logs\t10-u1-collab-log.txt`、`t10-u2-collab-log.txt`、`t10-server-log.txt` |
| `VAL-CLI-010` | passed | u2 改 `设计压力`，出站 op 带 `base_revision=1`；服务器回修订 2、没有 error；u1 收到修订 2；`sessions` 两行的 `base_revision` 都是 2 | `logs\t10_broadcast.result.txt`、`t10-010-u2-put.json`、`t10-010-sessions.json`、`t10-server-log.txt` |
| `VAL-CLI-011` | 部分成立（2026-11-11 按审查 S1、S9 更正，原记 passed） | 图纸切只读后 u2 收到 `readonly_downgrade`（含 reason 与 holder）；u2 的 `PIDTAG` 返回 `ok=false`、`code=COLLAB_READONLY`，位号仍是 P-1009；u1 的同类命令 `ok=true`。持有者怎么认定，见 §13.4 之 2。按契约给的例子逐句有证据，但「写命令被拒」作为一类不成立：当时的只读放行名单里有会改图的命令（`PIDLEGEND ON/OFF/PURGE`、`PIDGROUP`、`AUDIT Yes`、`MEASURE`、`FIND`），走同一个入口会被放行。处理情况见 §15 | `logs\t10_broadcast.result.txt`、`t10-011-u2-status.json`、`t10-011-u2-run.json`、`t10-011-u1-run.json` |
| `VAL-CLI-012` | passed | u2 的协同面板横幅画「只读」，u1 那一行画「编辑权」；`ui_bounds` 报告 `pid-collab-readonly` 与 `pid-collab-holder-u1`；u1 的面板上两样都没有（对照） | `shots\t10-012-u2-readonly.png`、`shots\t10-012-u1-editable.png`、`logs\t10-012-u2-capture.json`、`t10-012-u2-ui-bounds.json` |
| `VAL-CLI-013` | 见 §13.4 之 4 | u1 收到 `peer_joined`（u2 的会话、`display_name` 非空），在线人数 1 → 2，截图 2 行；u2 的 `welcome.peers` 含 u1。「首次连入为空数组」「u2 的长度为 1」两句不成立：服务器把连入者自己也列进 `peers` | `logs\t10_broadcast.result.txt`、`t10-u1-status-alone.json`、`t10-u1-status-two.json`、`t10-u2-status-two.json`、`shots\t10-013-u1-alone.png`、`shots\t10-013-u1-two.png` |
| `VAL-CLI-014` | passed | 强杀服务器后客户端 30 秒仍存活（`/ready` 200）；每次失败后的等待 200、400、800、1600、3200、5000、5000 毫秒，连接尝试相隔 2.45 → 7.06 秒；同端口同库重启后 0.6 秒建起新会话。间隔是从客户端日志读的——服务器停着的时候没有服务器日志可读 | `logs\t24_reconnect.result.txt`、`t24-014-attempts.txt`、`t24-sessions-before.json`、`t24-sessions-after.json`、`t24-u1-collab-log.txt`、`t24-server-log.txt` |
| `VAL-CLI-015` | passed | 没有服务器时 GUI 启动 1.14 秒后 `/ready` 200；面板显示未连接；画圆 `ok=true`；15 秒内 5 次重连尝试 | `logs\t25_offline.result.txt`、`t25-015-status.json`、`t25-015-circle.json`、`shots\t25-015-offline.png`、`logs\t25-015-collab-log.txt` |
| `VAL-CLI-016` | passed | `QUIT` 后进程 0.4 秒退出；服务器记到该会话的 close 帧，`sessions.status` 变失效 | `logs\t20_session.result.txt`、`t20-server-log.txt`、`t20-u1-collab-log.txt` |
| `VAL-CLI-017` | passed | 不带 `--collab`、`settings.json` 的 `collab.server_url` 为空：60 秒里服务器日志没有任何入站行，`sessions` 0 → 0；面板显示未配置；`/ready` 200 | `logs\t25_offline.result.txt`、`t25-017-status.json`、`t25-017-server-log.txt`、`shots\t25-017-unconfigured.png` |
| `VAL-CLI-018` | 见 §13.4 之 5 | `sessions` 2 行 u1、活跃、会话号不同；先连入的实例收到后者的 `peer_joined`；前者 `QUIT` 后后者仍活跃。「两个实例互相收到 `peer_joined`」只对先连入的一方成立 | `logs\t11_same_user.result.txt`、`t11-sessions-two.json`、`t11-sessions-after.json`、`t11-a-collab-log.txt`、`t11-b-collab-log.txt`、`t11-server-log.txt` |
| `VAL-CLI-019` | 见 §13.4 之 8 | 依赖树里没有 `tokio`、`async-std`，有 `tungstenite`（`smol` 来自 iced，按裁决不计）；`src/app/collab/` 下检索式零命中（`8cd274a2a` 之前是 1 处）。「冷构建不超过 20 秒」不成立 | `logs\t26-cargo-tree.txt`；检索式由测试 `nothing_under_this_module_is_asynchronous_code` 钉住；构建耗时见 `B\r2*-summary.txt` |
| `VAL-CLI-020` | passed | 强杀服务器后 2 毫秒客户端日志出现 `read error: ConnectionReset`（不是 close 帧分支），进入退避；重启后建起新会话 | `logs\t24_reconnect.result.txt`、`t24-u1-collab-log.txt` |

合计（2026-11-11 按独立审查 S9 更正）：**13 条成立，7 条部分成立**（003、005、009、011、013、018、019）。原来写的是「15 条按契约原文全部成立；5 条各有一句按字面不成立」，把 009 和 011 算进了全部成立，不对：009 的正文里有「双方均确认收到对方的 `peer_joined`」，和 018 不成立的那一句是同一件事；011 的「写命令被拒」按契约的例子成立，作为一类不成立（放行名单里有写命令）。003、005、013、018、019 各有一句按字面不成立，行为本身都取了证，卡在契约的写法上（§13.4）。014 的「从服务器日志的失败连接时间戳可读出」做不到（§13.4 之 9），行为成立，仍记成立，审查也是这么记的。没有哪一条没验，也没有哪一条非动指针或抢焦点不可。契约允许「点面板或用等价命令」的三处（004 占用、007 释放、016 退出）用的都是命令，面板上的占用 / 释放按钮和窗口的关闭按钮这一轮没有端到端点过。

里程碑验证中修掉的：`VAL-CLI-019` 的检索式命中（`8cd274a2a`）；服务器此前不记入站 hello、claim、release、op 与占用到期，而 001、004、006、007、009、010、017 的证据都引服务器日志（并在 `360dd0708` 里）；客户端此前不记连接尝试与断开原因，014、015、020 要读（同上）。

### 13.4 定下来的事，和契约里对不上、需要拍板的事

按验证契约定下来、已经实现的三处（§7 列的缺口）：

1. **`VAL-CLI-010`**：op 生效后，服务器在同一事务里把该图全部活跃会话的 `sessions.base_revision` 改到新修订，内存注册表同步推进（不推进的话下一次心跳会把旧值写回去）。依据是 `architecture.md` §3.3「每个协同编辑会话共享同一 `base_revision`」。
2. **`VAL-CLI-011`**：图纸切只读时，服务器逐个会话单播 `readonly_downgrade`，编辑权持有者除外，持有者照常能写；客户端收到即只读。**契约没说在「把图纸状态改成只读」这个触发方式下谁算持有者**，现在的规则是切换那一刻最早建立的活跃编辑会话（按会话号指名，同一用户的另一个会话不算）。另外两种读法：持有者是执行切换的那个人（但切换要「授权配置」权限，场景里是管理员 u4，不在图上，与契约写的「u1（编辑权持有者）」对不上）；或者只有契约括号里的另一种触发——「另一会话接管编辑权」——才有持有者（服务器现在对单人编辑的第二个会话是直接拒绝，没有接管这条路）。倾向保留现在的规则，由契约的主人确认。
3. **`VAL-CLI-012`**：协同面板在持有者那一行画「编辑权」，横幅画「只读」，两者都能用 `ui_bounds` 读到。

契约自己对不上的地方（都没有为了过断言去改契约或改行为）：

4. **`welcome.peers` 含不含连入者自己。** `VAL-CLI-013` 与 `VAL-CROSS-006` 写的是不含（首次连入为空、第二个人看到长度 1）；`VAL-CLI-003` 2026-11-09 的裁决写的是含（「u2 的 `peers` 长度为 2」）。服务器现在含，面板靠会话号认出本机。倾向：不含——契约两条断言加 `architecture.md` 的例子都这么写，那条裁决谈的本是别的事（第二个观测端能不能用普通 WS 客户端）；改起来是服务器 `hello` 里少列一项，同时把裁决里的「2」改成「1」。这要契约的主人动笔，所以没有做。
5. **后连入的一方收不到关于先到者的 `peer_joined`。** 它是从 `welcome.peers` 得知对方的。`VAL-CLI-018` 的「两个实例互相收到 `peer_joined`」和 `VAL-CLI-003` 裁决条目 2 的「u2 侧收到 `peer_joined`（u1）」按字面都做不到（003 的条目 1 又要求 u2 的 welcome 里已经有 u1，两条不能同时成立）。倾向：把这两句改成「先到者收到 `peer_joined`，后到者在 `welcome.peers` 里看到对方」，行为不动。
6. **`claim_denied.holder` 是会话号还是用户。** `VAL-SRV-011` 写「`holder` 指向 A 的会话号」（M1 已按此验过），`VAL-CLI-005` 写「`holder` 为 u1 的显示名或用户 id」。现在帧里是会话号，客户端经在线列表换成用户再显示（面板与 `/collab/status` 是 u1；命令行那条警告还是会话号，是个小毛病）。倾向：`VAL-CLI-005` 改成「客户端显示的占用者是 u1」。
7. **`VAL-CLI-008` 的两条取证提示都不准。** `capture` 没有 `wait_for_frame` 这个请求参数，`waited_for_frame` 是响应里的字段，表示这次截图等到的是真的重绘还是防挂起的定时器；这一轮在隐藏桌面上看到的都是 `false`，图却是当时的画面。窗口截图的 `screen_bounds` 也不是窗口图上的像素：`control_screenshot` 把整张窗口图的尺寸交给 `compute_grounding`，算出来的是「视口铺满整个窗口」时的位置，横竖比例还不一样；视口截图（`scope:"viewport"`）是准的，所以像素在视口截图上量。这是 `capture` 既有的行为，没有改。
8. **`VAL-CLI-019` 的「冷构建不超过 20 秒」。** 这个 crate 增量重编一次 2–4 分钟（`B\r2*-summary.txt`），任何实现都到不了 20 秒；那个数字出自当初单独验证 tungstenite 的小工程。没有为它跑 `cargo clean`。倾向：删掉这一句，或改成只量 `ocs_collab` 这类小 crate。
9. **`VAL-CLI-014` 说重连间隔「从服务器日志的失败连接时间戳可读出」。** 服务器停着的时候没有日志；间隔是从客户端日志读的。倾向：把出处改成客户端日志。

### 13.5 协同传输测试的隔离（`3c778fdc4`）

根因有三处，都是客户端靠进程级的东西工作、而测试程序里别的测试也在动它们：

- 停止标志（§6.1 已查明）：`shutdown()` 置位后没人复位，上游的 `batch_exit_saves_and_stops` 一跑完，后起的客户端第一圈就读到「正在退出」。
- 会话参数的槽：每个应用 `apply_config()` 时都把自己的协同设置重新发布一遍（`new_for_test()` 里就有），没配协同就是空地址，客户端读到空地址便原地等。只修停止标志之后剩下的失败都是等不到会话，查下来是这个。
- 出站队列：也是进程级的一条。

修法：新增 `Link`，把一条传输线程用到的东西（参数来源、出站队列、退出信号、心跳间隔）收在一起传进去。应用自己的客户端用 `Link::app()`（两个进程级的槽照旧，iced 起订阅 worker 时传不了参数；退出信号是这条链路自己的）；测试里的客户端用互不相干的链路。`shutdown()` 只对登记在案的那条线程置位，没有线程在跑就什么都不留下。

lib 测试程序整套并行跑，传输组的失败条数：

| 状态 | 遍数 | 传输组失败 |
|---|---:|---|
| 改前 | 5 | 1、3、4、3、4 条 |
| 改前，加 `batch_exit…`、单线程、`--shuffle-seed 1..8` | 8 种顺序 | 7 种失败，共 21 条 |
| 只修停止标志 | 6 | 2、1、2、1、1、1 条 |
| 现在 | 8 | 0（17 条 × 8 遍） |
| 现在，同上单线程，`--shuffle-seed 1..40` | 40 种顺序 | 0（18 条 × 40） |

等待会话的那段：它本来就是有上限的轮询（60 次 × 20 毫秒），也不是失败的原因——连得上的时候，整套并行跑下来实测 10–31 毫秒；连不上的那些等满 10 秒也没用。现在收成一个辅助函数，10 毫秒一轮，上限 10 秒（只有失败时才会等满）。日志：`B\r2c-lib-full\`、`B\r2c-seeds\`、`B\r2b-lib-full\`。

那 8 遍里另有四条上游的测试零星失败，单独跑各 5 次都通过（`B\r2c-single\`）：

| 测试 | 8 遍里失败 | 原因 |
|---|---:|---|
| `io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text` | 3 | 进程级的文字图集：每次 `apply_settings` 都经 `set_textfill` 把图集重置一遍，而每个 `new_for_test()` 要走两次；别的测试在「排版」与「导出」之间建了个应用，这边就找不到字形。从代码读出来的，没有另做实验证实 |
| `app::automation::tests::the_job_carries_one_glyph_snapshot_that_covers_its_pages` | 1 | 同上 |
| `app::plugin_host::tests::quad_feed_has_no_v14_auto_tab_per_plugin` | 1 | 断言进程级的插件面板注册表是空的，别的测试正好登记了一个 |
| `ui::ribbon::qat::tests::a_command_items_look_is_worked_out_once_and_forgotten_with_the_tabs` | 1 | 缓存条数少了一条，看断言是别的测试动了进程级的功能区模块表；没有细查 |

这类失败改前就有（第一轮带半成品的 4 遍里出现过 2 次，§10.4）。前两条有个便宜的修法：`set_textfill` 在值没变时不重置图集（三行）。那是上游的代码，没有动。

补记：另一条线随后在 `2271924a0` 里按根因修了这四条（连同另外两条），都改在测试里，已并进 `main`（`f8b28dc38`）。它查明的原因与上表前三条一致；第四条（qat）上表猜错了，实际是 `apply_settings` 每次都 `set_language`，语言代数加一，以它为键的缓存按设计清空。

合并之后 lib 测试整套并行又跑了 11 遍（`B\r2m-lib-full\`、`B\r2y-lib-full\`，外加全量那一遍）：传输组 0 失败；9 遍全过，2 遍各失败 1 条，都在那六条之外——

| 测试 | 11 遍里失败 | 现象 |
|---|---:|---|
| `app::automation::tests::a_job_whose_text_the_atlas_no_longer_holds_is_laid_out_again` | 1 | 断言等于 0 的那个数得了 4。看名字与上表前两条同属字形图集一族；没有细查 |
| `io::svg_export::tests::a_page_style_table_wins_over_the_job_wide_one` | 1 | 同一页两种走法出的 SVG 不相同；没有细查 |

两条单独跑各 5 次都通过，所在模块整组也通过（128 条、38 条；`B\r2m-groups\`、`B\r2y-single\`）。这一轮没有碰这两个文件。

### 13.6 种子图上 `PID_AUTHOR` 读回为空的原因

是探针读数的写法，不是图也不是接口。`probe-broadcast.ps1` 把响应 `ConvertFrom-Json` 之后再 `Out-String`，然后用正则找 `tag=…`；PowerShell 排版嵌套对象时把里层印成 `{@{handle=391; xdata=}}`——`xdata` 那一格是空的——所以永远匹配不上。同一张种子图、同一个接口，直接读响应正文得到的是 `"tag=P-1001"`（`E\logs\t00_smoke.result.txt`）；离线拿同一段 JSON 走探针的写法，也得到那个空格。

另外从头走了一遍：打开基础图、插入一个容器（得到 P-1001，句柄 391）、保存、退出、在新实例里重开，`PID_AUTHOR.tag` 读回仍是 P-1001（`E\logs\t01_seed.result.txt`）。端到端用的种子图就是这样做出来的。

### 13.7 `ttf_glyph.rs` 的合并取舍（`90a1695e4`）

`dd43f6aa2` 合并时 `build_shaped` 两边都改了：上游加了定位字形缓存，本分支把描轮廓挪到了字体系统的锁外，合并取的是「锁外先问缓存」。核对办法：让同一批文字（本机前三个字体族 × 11 条，含共用前缀、重复字形、位号、`℃`、中文回退）先不经缓存排一遍，再经缓存排两遍，三遍的每个坐标按位比较。结果全等，取法不用改。把缓存键里的落笔横坐标临时改成常数，这条测试会失败（`"Agency FB" "AVA"`），说明它咬得住。

### 13.8 全量测试与门禁

在 `8cd274a2a` 上（`B\r2z-*`）：`build --workspace` 198 秒，`test --workspace --no-run` 256 秒，20 个测试程序全部退出码 0。

| | 通过 | 失败 | 忽略 |
|---|---:|---:|---:|
| 第一轮（合并 + 半成品） | 6483 | 4 | 43 |
| `8cd274a2a` | 6571 | 0 | 43 |
| `f8b28dc38`（并进另一条线最后两个提交之后，`B\r2y-*`） | 6575 | 1 | 43 |

传输组归零。最后那一遍失败的 1 条是 `io::svg_export::tests::a_page_style_table_wins_over_the_job_wide_one`，并行时偶发，单独重跑 5 次都通过（§13.5 的补记）；同一个测试程序随后整套又跑 4 遍，全过。

mission 的门禁（在 `8cd274a2a` 上；`clippy -p OpenCADStudio` 与 pytest 在最后那次合并之后又跑过，结果相同）：`check --workspace` 通过；`clippy -p OpenCADStudio` 571 条告警，与改前同数，落在本轮改动行上的 0 条；`clippy -p ocs_collab -p ocs_collab_server --all-targets` 0 条；lib 的 `collab` 分组 86 通过；`ocs_collab` 69、`ocs_collab_server` 105 + 11 通过；`pytest scripts/pid-platform` 558 通过。最后那次合并之后 `collab` 分组仍是 86 通过。`--locked` 现在能用，`services.yaml` 里「换成 `--offline`、构建后还原锁文件、绝不提交」那段说明已经过时。

没有做的：wasm 目标的检查。

### 13.9 磁盘与环境

- 构建前删了 `D:\Rust\target-ocs-mission\debug\incremental` 下 199 个旧子目录，18.14 GB（清单 `B\r2-incremental-delete-list.tsv`）。
- 中途又删了同一目录下 3 个已经没有用处的子目录，6.64 GB（清单 `B\r2-incremental-delete-list-2.tsv`）：它们最后写入于 20:43–20:46，此后 `Cargo.lock` 与合并改变了这个 crate 的构建单元，同样的编译都写进了新目录。当时 D 盘剩 18.1 GB，接下来的全量构建还要几 GB，不删会碰到 15 GB 的线。这一步不在开工时说好的范围里。
- 全量测试跑完后 D 盘剩约 20 GB。`D:\Rust\target-ocs-mission` 里还有能清的：`debug\build\OpenCADStudio\`（23:07 量得 141 个单元目录、12 GB），多数是 11-08 到 11-10 傍晚各次构建单元留下的旧产物，估计 8 GB 上下；没有动。`D:\work\plant-code\cad\OpenCADStudio\target` 没有碰。
- mission 留下的三个 `pwsh`（PID 22684 起于 06:17:20、32088 起于 06:47:43、33652 起于 10:58:37，都是 `D:\scratch\m6-claims\start-*.ps1`）和 `D:\scratch\m6-broadcast\`、`m6-claims\` 下的 5 个 `.ocs.lock` 不挡路——验证用的是自己的目录，3240 与 3210–3239 都空着——没有动。
- 验证期间机器上有另一条线的两个编辑器实例（`ocs-upstream-20261110\target\debug\OpenCADStudio.exe`，`--http 8251` / `8252`），没有碰。

### 13.10 还没做的、没核实的，M7 开工前要知道的

`collab-client-broadcast` 的已知限制：

- **只读门只管命令入口**（命令行、`dispatch_command`、REST 的 `run`）。REST 的直接写操作（`xdata_set`、实体增删、`undo` / `redo`）、属性面板、夹点、符号面板的点选都不经过它。这样改出来的位号或属性变化不会广播，命令行会提示本图已与别人不一致；新建或删除实体连提示都没有。`VAL-CROSS-004`（写被拒并留痕）与 `VAL-CROSS-011`（输入控件不可编辑）要的是一道总闸，得在 M7 做。
- **只读状态不跨重连**；后加入只读图纸的会话不会被降级；图纸改回可编辑时没有「恢复」消息。
- **会话不绑定标签页**：入站变更应用到当前活动标签页（活动的是起始页时不应用，只报出来），出站钩子对任何标签页的撤销步都生效。一个实例里只开一张图、且它是活动标签页时没有问题；`architecture.md` §5.4 设想的按标签页的会话字段没有实现。
- **一步一个 op，基线取排队时的值。** 两步编辑之间不等回执就发，第二个 op 会被服务器以 `STALE_BASE_REVISION` 拒绝——本地已改而别人没收到，命令行会说出来，但不会重发。M7 把基线硬校验换成三段判定时要一起处理。
- **一个 op 里批着走的后续变更（`changes`）服务器不认识**：审计只记第一处，M7 的冲突判定也只会看到第一处。
- 新建或删除的实体不广播；`props` 溢出到 XRecord 的符号，属性变化不广播。
- 远端变更在本地命令执行到一半时到达，会把那条命令的撤销步从中间截断；没有专门处理。

这一轮看到、没有改的：

- 传输线程的重试计数在会话建成后不归零。`CollabEvent::Connecting` 的注释写的是「成功会话后的第一次尝试从零数起」，实际上计数接着上次往下数，同一进程里断过几次之后，再断线一上来就是 5 秒的封顶间隔。契约的场景只断一次，不受影响。
- 占用被拒时命令行那条警告显示的是会话号（面板与状态里是用户）。
- 服务器给 `display_name` 用的是 `user_id`，没有去查 `users` 表里的显示名。
- §13.5 补记里剩下的两条零星失败。
- §6.2 里 mission 记录在案的问题没有复核，除了「`capture` 默认不等帧」「`scope:"window"` 的 `screen_bounds` 是物理像素」两条，实际情况见 §13.4 之 7。

M7 开工前：§13.4 的 2、4、5、6 最好先有结论，它们都落在 M7 要动的代码上（`hello`、`readonly_downgrade`、`claim_denied`）；`VAL-CROSS-004`（没有编辑权限的角色连入即降级）会用到「后加入的会话降级」，它和 `VAL-CROSS-011` 都会用到总闸；端到端验证可以直接用 `E\m6h.py`，或者仓库里另一条线提交的 `docs/automation/hidden_desktop.py`。

## 14. 执行记录：2026-11-11 第三轮（零星失败的根因、补跑、M7 开工）

本节的路径缩写：`B` = `D:\work\plant-code\cad\ocs-mission-backup-20261110\logs`，`E7` = `D:\scratch\m7-validation`（本轮的新证据；`D:\scratch\m6-validation` 里上一轮的证据没有动）。进度仍然不写回 mission 的文件，断言状态都是「建议状态」。

### 14.1 提交

`main` 上 `3d4281eb3` 之后（只走第一父提交），未推送：

| 提交 | 内容 |
|---|---|
| `09e9508c6` | 合并 `upstream-merge-20261110`（到 `d92c68ca9`）：备份还原把参考数据卷放回 `Ref`。在动图集那几个文件之前并的 |
| `75454fa6d` | 两条并行偶发失败按根因修：`set_textfill` 只在值真的变了才重置字形图集；出图按图集代数核对（§14.2） |
| `35749fc48` | 合并（到 `8200f9b88`）：备份面板的图纸节点多出工艺点、分支点等 |
| `821e11395` | `lasso_press_drag_selects` 不再受移动合并门限的墙钟影响（§14.2 末） |
| `13dad6ef8` | 协同会话绑定到它那张图的标签页（§14.4 末） |
| `cce14ba77` | 协同按位号引用对象的三处缺陷（§14.4） |
| `369dacba1` | M7 `conflict-field-merge`（§14.5.1） |
| `454db6685` | 合并（到 `1fb8423f3`）：备份面板读数据库转储、数据映射表 |
| `5a708d31d` | M7 `conflict-object-topology`（§14.5.2） |
| `dacf954ba` | M7 `conflict-resolution`（§14.5.3） |

三次合并都是自动合并、无冲突，与本线改的文件没有重叠。

feature（照此同步回 `features.json`）：`conflict-field-merge`、`conflict-object-topology`、`conflict-resolution` 由 `pending` 改 `completed`，完成提交依次是 `369dacba1`、`5a708d31d`、`dacf954ba`；`cross-area-flows` 仍是 `pending`。

### 14.2 两条零星失败的根因

`io::svg_export::tests::a_page_style_table_wins_over_the_job_wide_one` 与 `app::automation::tests::a_job_whose_text_the_atlas_no_longer_holds_is_laid_out_again`（§13.5 末尾那两条）。它们不在另一条线 `2271924a0` 修的六条里。

**共享的是哪一样、谁写谁读。** 进程级状态是 `scene::text::sdf_atlas::text_atlas()`（字形图集）和它的代数 `ATLAS_GENERATION`。字形四边形记着自己那一格的位置，图集一重置或长高，此前排好的四边形全部作废。

- 写的人有三拨。其一，每一条构造应用的测试：`new_for_test` → `new` → `apply_config` → `apply_settings` → `set_textfill` → `reset_font_entries` → `GlyphAtlas::reset`，不管 TEXTFILL 变没变都重置，一次构造两遍（`new` 里一遍，套默认配置又一遍），源码里 `new_for_test()` 有 865 处调用。其二，`sdf_atlas` 自己的单元测试：它们在私有的 `GlyphAtlas` 上做重置和长高，却也推进了全进程共用的代数。其三，图集长高（一个进程至多两次）和第二条测试自己那次 `reset`。
- 读的人。第一条测试的两页各自在写到第一段文字时取一次快照，中间隔着一次落盘；两次快照之间图集被重置，第一页有字、第二页没字（失败输出里两页的差别只有第一页多出来的那几条字形笔画）。第二条测试读的是 `resolve_plot_job`：排版、取快照、不一致就重排，最多 4 次。它有三种失败，定向复现里都见到了：4009 行与 4044 行，5 遍排版每一遍都被别的测试的重置打断；4058 行，`stale_glyphs` 是 0、`missing_glyphs` 是 0，画出的元素数却是 12 对 11。

**4058 行那一种不只是测试互相干扰，是产品缺陷。** `resolve_plot_job` 靠「四边形的键在不在快照里」判断这些页还能不能用。键是格子的位置，不是格子里的字：图集重置之后，格子按下一次排版的顺序重新填，旧四边形的键可以指向一个存在的格子、里面是别的字。这时数缺的键得 0，页被当成完好的交出去，出图上是错的字，没有任何报告。产品里触发它需要：图集被重置（改 TEXTFILL；或者选项窗口的「恢复默认」「关闭不保存」——`apply_settings` 此前也无条件重置），随后先排了别的文字，再在场景的下一帧之前出图（无头出图没有帧）。改写后的第二条测试把这个情形摆成确定的：标注是同一个字重复四次，烘在第一格；重置后在同一把锁里先烘另一个字。修之前它单独跑 10 遍 10 遍失败，画出来的是 `B` 的轮廓（`B\r3a-red-evidence.log`）。

**修法**（`75454fa6d`）

- `set_textfill`：值没变就什么都不做。对产品是少做事——启动、恢复默认、放弃选项时不再把所有字形重烘一遍、所有图纸的文字重排一遍；值真的变了照旧重置。
- 代数只属于进程级的那张图集：`GlyphAtlas` 带一个可选的计数器，`text_atlas()` 用 `ATLAS_GENERATION`，私有图集不带。
- `GlyphSnapshot` 记下取快照时的代数；`PlotAssets::letters(pages, 代数)` 要求「键都在」并且「快照的代数等于排版时的代数」。`resolve_plot_job` 每一遍先让场景与图集对齐（`Scene::resync_with_atlas`，就是 `update()` 里每帧做的那一段），排版，取快照，再核对；不成立就重排，仍然最多 4 次。
- 第一条测试的三次导出共用一份快照（同文件里的 `shared_assets` 就是为这个写的）。第二条测试改成上面说的确定情形，断言比的是整页的轮廓。

修完之后一个测试进程里图集至多动 4 次（长高 2 次，第二条测试自己的 2 次重置），少于 `resolve_plot_job` 的 5 遍，「5 遍都被打断」在构造上不再可能。

**先红后绿**：`setting_textfill_to_the_value_it_has_leaves_the_baked_glyphs_alone` 在旧行为下失败，改写后的第二条测试在旧的 `resolve_plot_job` 下 10 遍 10 遍失败，改后都通过；另加 `changing_textfill_rewinds_the_atlas_once`、`only_an_atlas_given_a_counter_moves_one`。

**改前改后的失败率**（同一台机器、同样的种子；`B\r3-before-*-repro\`、`B\r3-after-*-repro\`，脚本 `B\repro.ps1`）

| 跑法 | 遍数 | `a_page_style_table…` | `a_job_whose_text…` |
|---|---:|---|---|
| 定向：两条目标 + `app::command_driver`（217 条，都构造应用），打乱顺序。改前 | 600 | 83 | 3（4044 行 2 次，4009 行 1 次） |
| 同上，改前，另外两批 | 100 + 3 | 15 + 1 | 0 + 1（4058 行） |
| 同上，改后 | 600 | 0 | 0 |
| 定向：`command_driver` + `automation` + `control` + `plugin_host`（627 条）。改前 | 60 | 14 | 0 |
| 同上，改后 | 60 | 0 | 0 |
| lib 整套并行。改前（上一轮，§13.5） | 11 | 1 | 1 |
| lib 整套并行。改后（`B\r3c-lib-full\`） | 8 | 0 | 0 |

顺带的效果：lib 整套并行一遍从 52–58 秒降到 27–33 秒，省下的是每个测试应用重烘字形的时间。

**没有改、看到了的**

- 控制通道的 `plot` 动作（`src/app/control/actions.rs`）自己排页、不经 `resolve_plot_job`，SVG 用空的 `PlotAssets`（每页各取快照），PDF 在写的时候才取快照；`file.rs` 里几条 PDF 导出在工作线程上取快照。多布局出图中途图集长高，或者排版与取快照之间界面线程又排了别的字，这些路径没有上面的核对。它们不是这两条失败暴露的，这一轮没有动（§14.6 之 6）。
- `OpenCADStudio::new()` 读的是本机真实的 `settings.json`。那里 `textfill` 若是 `false`，每次 `new_for_test` 仍然会真的翻两次 TEXTFILL。本机与 CI 都是默认值。

**核对时新出现的第三条**：`app::automation::tests::lasso_press_drag_selects`，大干扰组改后的 60 遍里失败 1 遍（「drag must start (add=true rect=false)」）。不是共享状态，是墙钟门限：一次指针移动的处理耗时到了 8 毫秒（`MOVE_COALESCE_FROM_MS`），`on_viewport_move` 就把 `move_pass_due` 设到「现在 + 这次的耗时」，在那之前到达的移动被 `defer_pointer_move` 扣下，等下一帧再答复；测试连发 11 次移动、中间没有帧，机器忙的时候后面的移动全被扣下，拖拽起不来。另一条线在 `2271924a0` 里对 `rapid_drawing` 那条、上游在 `parameter_preview.rs` 里用的是同一个修法：每次移动之前把门限打开（`821e11395`）。它改前的失败率太低，没有单独量出来（同一组改图集之前 60 遍 0 次、之后 60 遍 1 次，都在噪声里）；改后的遍数见 §14.8。同一类的口子还有：直接发 `Message::ViewportMove` 的测试位置约 70 处，其中连发两次以上又断言「每一次都被答复」的都可能在机器很忙时失败，这一次只改了实际失败的那一条。

### 14.3 契约里那五处：查到的断言，做了什么，建议的措辞

**`welcome.peers` 含不含连入者自己——没有改。** 契约里提到 `welcome` 或 `peers` 的断言一共九条：

| 断言 | 相关的话 | 要求 |
|---|---|---|
| `VAL-CLI-001` | 「`peers` 为数组」 | 不分 |
| `VAL-CLI-003`（2026-11-09 的裁决条目 1、3） | 「`u2` 侧收到 `welcome` 且 `peers` 长度为 2、包含 `u1`」「`u2` 侧自己维护的在线列表计数从 2 变 1」 | **含自己**（图上只有 u1、u2 两个人） |
| `VAL-CLI-013` | 「`u1`…确认 `welcome.peers` 为空数组（首次连入）」「`u2` 的 `welcome.peers` 长度为 1 且含 `u1`」 | 不含自己 |
| `VAL-CROSS-006` | 「`u1` 的在线面板显示 0 名其他用户（`welcome.peers` 为空）」「`u2` 的 `welcome.peers` 含 `u1`」 | 不含自己 |
| `VAL-PERM-018` | 只读 `base_revision` | 不分 |
| `VAL-SRV-004`（mission 已记 passed） | 「`peers`（数组）」 | 不分 |
| `VAL-SRV-009`（passed） | B 收到 `welcome` | 不分 |
| `VAL-SRV-019`（passed） | 「`peers` 数组至少含 A 的 `session_id`」 | 不分 |
| `VAL-SRV-027`（passed） | 只读 `claims` | 不分 |

mission 记为 passed 的四条 `VAL-SRV` 都不要求含自己。但 `VAL-CLI-003` 要求，而且上一轮按这句取过证（`welcome.peers` 长度 2）。照「有一条要求含自己就不改、等用户」办：服务器没有动。两种定法各自要改的句子：

- 定为**不含自己**（倾向，`VAL-CLI-013`、`VAL-CROSS-006` 与 `architecture.md` 的例子都是这样）：`VAL-CLI-003` 裁决条目 1 原句「`u2` 侧收到 `welcome` 且 `peers` 长度为 2、包含 `u1`」改成「`u2` 侧收到 `welcome` 且 `peers` 长度为 1、包含 `u1`」；条目 3 原句「`u2` 侧自己维护的在线列表计数从 2 变 1（打印 `peersBefore` / `peersAfter`）」改成「`u2` 侧自己维护的在线人数（含自己）从 2 变 1（打印 `peersBefore` / `peersAfter`）」。行为上是服务器 `hello` 里少列一项。
- 定为**含自己**（现状）：`VAL-CLI-013` 原句「确认 `welcome.peers` 为空数组（首次连入）」改成「确认 `welcome.peers` 里只有自己（首次连入）」，「`u2` 的 `welcome.peers` 长度为 1 且含 `u1`」改成「长度为 2 且含 `u1`」；`VAL-CROSS-006` 原句「（`welcome.peers` 为空）」改成「（`welcome.peers` 里除自己外没有别人）」。

**后连入方的 `peer_joined`——行为不动。** 建议的措辞：`VAL-CLI-018` 原句「两个实例互相收到 `peer_joined`。」改成「先连入的实例收到后连入者的 `peer_joined`；后连入的实例在 `welcome.peers` 里看到先连入者。」`VAL-CLI-003` 正文原句「第二个观测端（`u2`，同一 `PID-1101`）连入并确认收到 `peer_joined`。」改成「第二个观测端（`u2`，同一 `PID-1101`）连入并确认 `welcome.peers` 含 `u1`。」；裁决条目 2 原句「`u2` 侧收到 `peer_joined`（`u1`）与后续 `peer_left`（`session_id` 与 `u1` 匹配）」改成「`u2` 侧的 `welcome.peers` 含 `u1`，并收到后续 `peer_left`（`session_id` 与 `u1` 匹配）」。

**`claim_denied.holder`——行为不动**（与 `VAL-SRV-011` 一致）。`VAL-CLI-005` 原句「`holder` 为 `u1` 的显示名或 `u1` 用户 id」改成「`holder` 为 `u1` 的会话号（同 `VAL-SRV-011`）；`u2` 的 `GET /api/v1/collab/status` 与协同面板把占用者显示为 `u1`」。

**`VAL-CLI-019` 的 20 秒——没有处理。** 原句「`cargo build -p OpenCADStudio` 冷构建（`cargo clean -p OpenCADStudio` 后）耗时不超过 20 秒。」建议删去，或改成「`cargo build -p ocs_collab` 冷构建耗时不超过 20 秒」；证据一栏的「构建耗时测量」随之删去或改对象。

**`VAL-CLI-011` 的持有者规则——照旧，契约未规定，待用户确认。** 代码注释里已写明（`http.rs` 的 `downgrade_all_but_the_holder`）。建议在原句「`u2` 收到 `readonly_downgrade`，载荷含 `reason` 与 `holder`。」后面加一句：「以改图纸状态触发时，编辑权持有者是切换那一刻该图上最早建立的活跃编辑会话，`holder` 是它的会话号；该图上没有编辑会话时 `holder` 为 `null`，所有会话都降级。」

§13.4 里另外两处也给出措辞：`VAL-CLI-008` 的取证提示里「`wait_for_frame`」应改为「响应里的 `waited_for_frame`」，「窗口截图的 `screen_bounds`」应改为「视口截图（`scope:"viewport"`）的 `screen_bounds`」；`VAL-CLI-014` 的「从服务器日志的失败连接时间戳可读出」应改为「从客户端日志的连接尝试时间戳可读出」。

### 14.4 `object_ref` 改成位号：四个问题

上一轮把 op 里的 `object_ref` 从实体句柄换成了位号（`architecture.md` §3.2 的每个载荷都这样写，服务器的审计与裁决也按它），句柄只作为 `handle` 提示随行。这是一处没人点名的设计变化，四个问题的回答：

1. **没有位号的对象怎么引用？** 不引用。管段、打断符、页间连接符的 `PID_AUTHOR.tag` 是空串，它们在线上没有名字：发送方不陈述它们的变更，接收方收到空的 `object_ref` 一律不应用、按「（无位号）」报成未应用。它们带的值是派生的（管段取管路的值），所以接收方应用完一个 op 之后在同一个撤销步里跑一遍属性下传（`propagate_pid_chain`，与本地改属性之后是同一个入口），由本图自己算。**这里查出一处会引用错对象的缺陷，已修**：此前空位号的变更会带着句柄发出去，接收方只要句柄下的实体位号也是空的就信了——两边各自打开文件之后各画过一条管，同一个句柄下就是两根不同的管段，别人那根管段的属性会写到我这根上。
2. **两个对象位号相同时怎么办？** 句柄提示来分：提示的句柄下那个实体确实带着这个位号，就是它；否则必须恰好只有一个对象带这个位号，没有或者多于一个都不应用，报未应用。服务器那一侧按位号记对象，两个同位号的对象在它那里是一个条目。两边都有重复位号、句柄又对不上（都是各自在会话里新建的）时说不清指哪个，结果是报未应用，不会改错。
3. **改位号这个操作引用旧位号还是新位号？应用之后下一个操作还找得到吗？** `set_tag` 的 `object_ref` 是旧位号（别人此刻还认的名字），`value` 是新位号；同一步里跟在后面的属性变更、以及此后的 op，都用新位号。接收方应用之后对象就带着新位号，找得到。服务器一侧（§14.5.1）：改名之后，没看到改名的会话仍用旧位号提交，服务器顺着改名历史找到对象，广播时把 `object_ref` 换成现在的位号。**查出一处该报没报的缺陷，已修**：旧位号在本图上挂在两个符号上（说不清指哪个）、而新位号正好另有其物时，这次改名既没应用也没报——`already_retagged` 的注释写的是「旧位号已经没有、新位号已经在」，代码只查了后一半。
4. **大小写与空白是否归一？** 不归一。比较的是存下来的字符串，逐字相等，大小写与首尾空白都算数——位号引擎的唯一校验、服务器的占用表与审计也都是这样比的。`PIDTAG <旧> <新>` 在命令行上找旧位号时不分大小写，但广播出去的 `object_ref` 取的是存着的那个字符串，不是用户敲的。没有查出缺陷。

四条红测试修之前都失败（`B\r3d-red.log`），修后通过（`cce14ba77`）：`a_change_naming_no_tag_is_not_applied_by_its_handle_alone`、`a_change_to_an_untagged_object_is_not_stated`、`a_retag_fitting_several_symbols_is_reported_even_when_the_new_tag_exists`、`a_remote_line_change_is_carried_down_to_runs_and_segments`。

**会话与标签页的绑定**（`13dad6ef8`）。入站变更此前写到当前活动的标签页：用户切到另一张图——比如同一张图的副本，位号相同——别人对协同图纸的改动会写到前面那张图上，是数据错误；反过来，在别的图上的编辑会被当成协同图纸的变更广播出去；只读闸也不分图。现在会话有一个 `document`，在它第一次需要文档时定在当时在前面的那张图上（启动时打开的那张），此后入站写到它、只有它的编辑才出站、只读闸只管它；它关了之后会话没有文档，入站变更报「协同的图纸已经关闭」。`GET /api/v1/collab/status` 多一个诊断字段 `document_id`。三条红测试修之前都失败（同一份 `r3d-red.log`）。

没有处理的：管路上手工改过的字段带覆盖标记（`__overrides`），标记不随变更广播，接收方的管路没有标记，之后管线再改同一字段两边会不一样；给一个原本没有位号的对象起位号，这一步对别人说不出来；绑定取的是「第一次用到时在前面的图」而不是「命令行上给的那个文件」。

### 14.5 M7：每项开工前列出的断言与取证办法

#### 14.5.1 `conflict-field-merge`（`VAL-CONF-001`–`005`）

mission 的说明（`features.json`）：不同字段并发修改自动合并且无冲突条目；同字段并发修改按 `base_revision` 判定落后方让位且无冲突条目；被覆盖的原值写入审计；`base_revision` 相同且值不同时生成冲突条目。

开工前服务器的行为：`base_revision` 必须等于当前修订，否则一律 `STALE_BASE_REVISION`（`ws.rs` 的 `op` → `Store::commit_op`）；服务器不记任何对象的字段值，`conflicts` 表与 `Store::insert_conflict` / `list_conflicts` / `resolve_conflict` 在 M1 已经建好但没有调用方，`ocs_collab::conflict` 里有一个 M1 留下的纯函数骨架。

| 断言 | 要点 | 准备怎样取证 |
|---|---|---|
| `VAL-CONF-001` | s1 改 `P-1001.设计压力` 得修订 1；s2 仍带基线 0 改同一对象的 `公称直径`，收到 `op_applied`（修订 2），没有 `conflict`；`conflicts` 0 行；两个字段都读得回 | 两个 WS 会话的帧序列；`select count(*) from conflicts`；服务器的语义读回 `GET /drawing/PID-1101/objects`（契约写的是 GUI 的 xdata 读回，这条断言全程不起 GUI，读服务器记下的字段值） |
| `VAL-CONF-002` | 同上，但 s2 改的是同一字段：s2 收到表示基线落后的 `error`，`conflicts` 0 行，字段最终值是先到的 `1.6MPa` | s2 的出站 op 与入站帧；`conflicts` 计数；语义读回 |
| `VAL-CONF-003` | 上一条在 `audit_log` 留一行：`object_ref='P-1001'`、`action` 表示属性覆盖、`user_id='u2'`、`change_text` 同时有 `2.5MPa` 与 `1.6MPa`，九个字段都非空 | `GET /audit?tag=P-1001`；`select * from audit_log where object_ref='P-1001' order by seq desc limit 1` |
| `VAL-CONF-004` | 两个会话基线相同、同一字段、值不同、服务器判为并列：`conflicts` 新增 1 行（`value_a` 是先到的，`value_b` 是后到的，`base_revision=0`，`status='待裁决'`，`resolution` 与 `resolved_by` 为 NULL），双方各收到 `conflict`，`conflict_id` 与表中一致 | `select * from conflicts`；双方的 `conflict` 帧 |
| `VAL-CONF-005` | s1 在修订 3 写了 `公称直径=DN50`；s2 带基线 1 写 `DN80`：让位，`conflicts` 行数不增，最终值 `DN50`，s2 收到覆盖通知 | `conflicts` 计数前后；语义读回；s2 的入站帧 |

单元测试先行（`ocs_collab` 的判定纯函数、`ocs_collab_server` 的 `ws` 处理与库表），再用 Python 起真服务器、两个 WS 会话、直接读 SQLite 走一遍契约的场景（契约写的工具是 Node 26 的 WebSocket；用 `E7` 里的标准库 WS 客户端，帧原文照样落盘）。这一项不起 GUI，不涉及指针与焦点。

**做了什么**

- `ocs_collab::conflict` 重写：`DrawingModel` 是服务器对一张图的全部所知——每个对象（按现在的位号）每个字段最后的值、生效的修订、是谁的会话写的——加上改位号的历史。`judge` 给出前两段的结论：基线之后没有别的会话写过这个字段，合并（`Apply`）；写过而且值不同，让位（`Yield`）。`collides` 是第三段的条件。M1 留下的骨架把「值不同且基线等于当前修订」一律判成冲突，那其实是最普通的改值，已经换掉，它的 12 条单测换成按契约写的 14 条。
- 服务器新增 `ops.rs`，`ws::op` 交给它：判定、提交、记账在一把锁里同步做完。库里新增 `ops` 表（每个生效的 op 一行，键是它产生的修订），重启后靠重放它重建 `DrawingModel`；`Store::apply_op` 把「修订加一、会话基线、审计行、ops 行」放在一个事务里。
- 让位：给提交者回 `error`，`code` 是新增的 `OP_SUPERSEDED`，`message` 写明基线、生效的修订、谁先写入、两个值；`audit_log` 记一行 `action='property_superseded'`（改位号是 `tag_superseded`），`change_text` 里两个值都在；修订不动，不生成冲突条目。
- 并列：服务器带 `--tie-window-ms N` 启动时，基线等于当前修订的 op 先暂存 N 毫秒再提交；这段时间里别的会话带同一基线写同一字段且值不同，就是并列——两个 op 都不生效，`conflicts` 表一行（`value_a` 是先到的），全房间广播 `conflict`，双方的提交各记一行审计（`action='conflict'`）。窗口到了没人并列，就正常提交。默认 N=0：不暂存，谁先到谁生效，后到的按第二段让位。
- `GET /drawing/{id}/objects[?object_ref=位号]`：服务器记下的对象与字段值，给没有图纸的客户端读回用。
- 一个 op 里批着走的后续变更（客户端「一步一个 op」）现在逐处判定：能合并的生效，落后的让位，修订只加一次，`op_applied.changes` 是生效的那几处的平铺列表，每一处各记一行审计。
- 改过位号之后，没看到改名的会话仍用旧位号提交：服务器顺着改名历史找到对象，广播时把 `object_ref` 换成现在的位号（§14.4 第三问在服务器这一侧的回答）。

**定下来的事**（契约没有写明或者写得对不上的，见 §14.6）

- 并列怎样才算。契约 002 与 004 的线上输入一模一样（两个会话都带基线 0、同一字段、值不同），一条要「落后方让位、无冲突条目」，一条要「生成冲突条目」，区别只在 004 那句「且服务器配置为无法用基线判定（例如两者 op 在同一 base_revision 窗口内、服务器判为并列）」。`architecture.md` A.4 给的机械条件是「两个 op 的 `base_revision` 均等于服务器当前 `revision`」——先到的一生效，后到的就不再等于了，所以只有先到的还没生效时才可能成立。于是做成一个可配置的窗口，默认关。
- 会话不会落后于自己：同一个会话接连两步改同一字段，第二步带的基线比第一步生效的修订旧，但它是更新的意图，照常生效。这也顺带解决了 §13.10 里「两步编辑之间不等回执就发，第二个 op 被 `STALE_BASE_REVISION` 拒绝」——对改属性、改位号这两类 op 不再发生，客户端不用重发。
- 三段判定只管 `set_property` 与 `set_tag`。别的 op（`insert_symbol`、拓扑类、未知类型）仍按老规矩：基线必须等于当前修订，否则 `STALE_BASE_REVISION`。它们的判定是下一项的事。
- 服务器按位号记对象。两个对象同一位号在它那里是一个条目。

**结果**

| 断言 | 建议状态 | 证据（`E7\logs\`） |
|---|---|---|
| `VAL-CONF-001` | passed | `c01_field_merge.result.txt`；`ws-c01a-s1.jsonl`、`ws-c01a-s2.jsonl`（s2 的 `op_applied` revision=2，没有 `conflict`）；`c01-001-objects.json`（设计压力=1.6MPa，公称直径=DN50） |
| `VAL-CONF-002` | passed | `ws-c01b-s2.jsonl`（`error` `OP_SUPERSEDED`，「基线 0 落后…」）；`c01-002-objects.json`（1.6MPa）；`conflicts` 0 行 |
| `VAL-CONF-003` | passed | `c01-003-audit-row.json`、`c01-003-audit-http.json`：`object_ref=P-1001`、`action=property_superseded`、`user_id=u2`，`change_text`=「set_property P-1001.设计压力: 2.5MPa 让位于 1.6MPa（基线 0 落后于修订 1，u1 先写入）」，九个字段非空 |
| `VAL-CONF-004` | passed（服务器带 `--tie-window-ms 1500`） | `c01-004-conflicts.json`（各列与契约逐项相同）；`ws-c01c-s1.jsonl`、`ws-c01c-s2.jsonl`（双方的 `conflict`，`conflict_id` 与表中一致）；窗口过后两个 op 都没有 `op_applied`，修订仍是 0 |
| `VAL-CONF-005` | passed | `ws-c01c-s2.jsonl`（`error` `OP_SUPERSEDED`）；`conflicts` 前后都是 1 行；`c01-005-objects.json`（DN50）；服务器日志 `c01-server-c-log.txt` |

按字面不成立的：001 的证据一栏写的是 GUI 的 `GET /api/v1/entities/<handle>/xdata`，而这条断言全程只有 WS 会话、没有 GUI，字段值是从服务器的 `GET /drawing/PID-1101/objects` 读回的。其余四条逐句成立。场景脚本 `E7\c01_field_merge.py`，24 项检查全过（`ocs_collab` 71 条、`ocs_collab_server` 118 + 12 条单测通过，两个 crate 的 clippy 0 告警）。提交 `369dacba1`。

#### 14.5.2 `conflict-object-topology`（`VAL-CONF-006`–`009`、`017`）

mission 的说明：一方删除另一方仍提交修改时删除生效、修改方收到通知与被覆盖原值、不生成冲突条目而记入审计；拓扑级连接关系双方变更后生成待裁决条目且裁决前不改变任何一方拓扑；删除被引用对象返回 `OBJECT_REFERENCED`。前置里写的「`delete_object` op 类型已加入协议」没有做过，这一项里加。

开工前：服务器只认 `set_property` / `set_tag`（上一项），别的 op 原样转发、基线必须等于当前修订；它不知道哪些对象存在、谁引用谁，也没有任何拓扑。

| 断言 | 要点 | 准备怎样取证 |
|---|---|---|
| `VAL-CONF-006` | s1 删除 `P-1002`，得 `op_applied`；s2 没看到这次删除就改 `P-1002.设计压力`：s2 收到通知，含删除前的旧值；语义查询里不再有 `P-1002`；删除没有被回滚 | 两个 WS 会话的帧；`GET /drawing/PID-1101/objects` 前后；`audit_log` 里该对象的行 |
| `VAL-CONF-007` | 上一条不进 `conflicts`（`object_ref='P-1002'` 0 行）；`audit_log` 至少新增 2 行（删除、被拒的修改），`change_text` 含旧值 | `select count(*) from conflicts where object_ref='P-1002'`；`GET /audit?tag=P-1002` |
| `VAL-CONF-008` | s1 对管线 `L-1001` 提交 `add_pipe_run`，s2 同一基线提交同一管线上的 `insert_fitting`：`conflicts` 新增 1 行，`object_ref='L-1001'`、`field` 表示拓扑、`value_a` / `value_b` 非空、`status='待裁决'`；双方各收到 `conflict` | `select * from conflicts where object_ref='L-1001'`；双方的 `conflict` 帧（服务器带 `--tie-window-ms`，理由同 004） |
| `VAL-CONF-009` | 裁决之前拓扑还是冲突前的样子，不是任何一方的版本；`status` 仍是 `待裁决` | 新增的服务器拓扑查询 `GET /drawing/PID-1101/topology`，冲突前后各读一次比对 |
| `VAL-CONF-017` | `P-1003` 是 `L-1001` 某条管路的端点；删除它被拒，`code` 表示被引用（A.4 定的是 `OBJECT_REFERENCED`），`message` 列出引用方位号；对象仍在语义查询里；`audit_log` 记下这次被拒的尝试 | 删除 op 的入站帧；语义查询；`audit_log` 该行 |

要定的线上形状（契约只给了 `add_pipe_run` 的 `line_tag` + `points`、`insert_fitting` 的 `run_tag` + `at`，没说端点怎样表达）：`add_pipe_run` 可带 `from` / `to` 两个位号表示两端接到哪个对象上；管路位号由 `tag` 给出，不给时服务器按 `{管线号}-{两位序号}` 取下一个。

**做了什么**

- `DrawingModel` 多记三样：被删的对象（连同删除时各字段的值）、管线的拓扑（管线 → 管路；管路的点、两端接到的对象、上面的管件；连接符）、每条管线的连接关系最后是谁在哪个修订改的。
- 新认六种 op：`delete_object`、`insert_symbol`、`create_pipe_line`、`add_pipe_run`、`insert_fitting`、`create_connector`。此前它们只是原样转发。
- **对象级**：对一个「在我的基线之后被别的会话删掉」的对象改字段，不生效；回 `error`，`code` 是新增的 `OBJECT_DELETED`，`message` 里有删除的修订、删除者和该字段删除前的值；审计记 `change_rejected_deleted`。删除本身照常生效，审计行里带上删除前各字段的值。不进 `conflicts`。看到了删除还照旧位号写，算重新建立这个对象。
- **被引用**：`delete_object` 的对象若是某条管路的端点（`from` / `to`），拒绝；`error` `OBJECT_REFERENCED`，`message` 列出引用它的管线；审计记 `delete_refused`。管线删掉之后引用随之解除。
- **拓扑级**：同一条管线的连接关系不逐字段合并。带并列窗口时，两个会话在同一基线上各改同一条管线（一个加管路、一个插管件），算并列：都不生效，`conflicts` 一行，`object_ref` 是管线位号，`field='topology'`，`value_a` / `value_b` 是两个 op 的原文。不带窗口时不会并列：后到的那个如果基线早于别人对这条管线的改动，回 `STALE_BASE_REVISION`，请它按现在的管线重来；别人没动过的管线照样合并。
- `GET /drawing/{id}/topology`：已经生效的拓扑。还在窗口里的、被冲突条目压着的都不在里面。
- 没带位号的管路由服务器取号（`L-1001-01`、`-02`…），写进广播与 `ops` 表，各端和重放得到的是同一个名字。

**定下来的事**

- 「被引用」现在只指管路端点。管路上的管件（比如带位号的阀门）被删时不拦——拦了的话，客户端本地已经删掉而服务器不认，两边就不一样了，而客户端删管件时怎样把管路接回去是 M5 的事，这一轮没有碰。
- 只有改字段、改连接关系这两类 op 会在并列窗口里暂存。删除、插入符号、新建管线不争任何东西，到了就生效；它们之后才到期的暂存 op 按落后者处理（对象已删就走对象级）。

**结果**

| 断言 | 建议状态 | 证据（`E7\logs\`） |
|---|---|---|
| `VAL-CONF-006` | passed | `c02_object_topology.result.txt`；`ws-c02a-s1.jsonl`（删除的 `op_applied` revision=2）、`ws-c02a-s2.jsonl`（`error` `OBJECT_DELETED`，「…删除前的值 1.0MPa」）；`c02-006-objects-after.json`（没有 P-1002，修订停在 2） |
| `VAL-CONF-007` | passed | `conflicts` 里 P-1002 为 0 行；`c02-006-audit.json`：新增 `delete_object`（「…删除前：设计压力=1.0MPa」）与 `change_rejected_deleted` 两行；`c02-007-audit-http.json` |
| `VAL-CONF-008` | passed（服务器带 `--tie-window-ms 1500`） | `c02-008-conflicts.json`（`object_ref=L-1001`、`field=topology`、`status=待裁决`，两个值是两个 op 的原文）；`ws-c02b-s1.jsonl`、`ws-c02b-s2.jsonl` 里的 `conflict` |
| `VAL-CONF-009` | passed | `c02-009-topology-before.json` 与 `c02-009-topology-after.json` 逐字相同（`pipe_runs` 为空、修订 1）；两个 op 都没有 `op_applied`；`status` 仍是 `待裁决` |
| `VAL-CONF-017` | passed | `c02-017-topology.json`（P-1003 是 `L-1001-01` 的 `from`）；`ws-c02a-s2.jsonl`（`error` `OBJECT_REFERENCED`，「被 L-1001 引用为管路端点」）；语义查询里 P-1003 仍在；`c02-017-audit.json`（`delete_refused`，`user_id=u2`） |

按字面不成立的：017 写「服务器返回 `ok=false`」，WS 的 `error` 帧没有 `ok` 这个键（`architecture.md` §3.1 的 `error` 是 `{code, message, level?}`）；错误码用的是 A.4 定的 `OBJECT_REFERENCED`，不是契约举例的 `REFERENCED_OBJECT`。其余逐句成立。场景脚本 `E7\c02_object_topology.py`，17 项检查全过；`c01_field_merge.py` 在新服务器上回归 24 项全过；`ocs_collab` 76 条、`ocs_collab_server` 123 + 12 条单测通过，clippy 0 告警。提交 `5a708d31d`。

#### 14.5.3 `conflict-resolution`（`VAL-CONF-010`–`016`）

mission 的说明：`POST /conflicts/{id}/resolve` 支持三种 `resolution`（采用甲方 / 采用乙方 / 人工改值），人工改值的自定义值存 `resolved_value` 列；裁决结果与双方原值写入审计；裁决后 `status` 变 `已裁决` 且不可重复裁决；非管理员裁决返回 403；`GET /conflicts` 可按 `drawing_id` 与 `status` 检索。

开工前：`Store::list_conflicts` / `resolve_conflict` 在 M1 就有，`conflicts.resolved_value` 列也在，但路由里没有 `/conflicts`，裁决之后也没有任何东西把选中的值写回图上。

| 断言 | 要点 | 准备怎样取证 |
|---|---|---|
| `VAL-CONF-010` | u4 对并列条目裁决「采用甲方」：200；行变 `已裁决`、`resolution='采用甲方'`、`resolved_by='u4'`；字段最终值是 `value_a` | 裁决响应；`select * from conflicts where conflict_id=…`；服务器语义读回 |
| `VAL-CONF-011` | 新条目裁决「采用乙方」：字段最终值是 `value_b` | 同上 |
| `VAL-CONF-012` | 新条目裁决「人工改值」，`value='2.0MPa'`：最终值 `2.0MPa`，自定义值能读回 | 同上，另看 `resolved_value` 列 |
| `VAL-CONF-013` | 每个被裁决的条目在 `audit_log` 至少一行：`user_id='u4'`、`object_ref` 是冲突对象、`change_text` 里 `value_a`、`value_b`、最终值都在 | `GET /audit?tag=P-1001`；`select change_text from audit_log where user_id='u4'` |
| `VAL-CONF-014` | 对已裁决的条目再裁决：409（或 `ok=false`），`code` 表示已裁决；行不变；审计不增 | 第二次的响应；行；审计行数前后 |
| `VAL-CONF-015` | u3（校对人员）裁决：403，`code` 表示权限拒绝，`level` 标明角色级；行仍是 `待裁决`、`resolution` 为 NULL | 403 的响应；行 |
| `VAL-CONF-016` | 至少 1 条待裁决、3 条已裁决时，`GET /conflicts?drawing_id=PID-1101&status=待裁决` 的 `items` 条数对，每项 `conflict_id` / `object_ref` / `field` / `status` 非空；`status=已裁决` 返回 3 条 | 两次列表的响应；`select status,count(*) from conflicts group by status` |

裁决由谁能做：照「改图纸状态」的先例，按「授权配置」这项操作走三级权限判定（角色矩阵里项目管理员允许、校对人员拒绝）。裁决选中的值由服务器作为一次 op 写回（修订加一、广播 `op_applied`），这样在线的各端当场改过来，离线的重连后也补得上。

**做了什么**

- `GET /conflicts[?drawing_id=&status=]` → `{"items":[…],"total":N}`，每项就是 `conflicts` 的一行。
- `POST /conflicts/{id}/resolve`，载荷 `{resolution, resolved_by, value?}`（`value` 是契约的写法，A.1 写的 `resolved_value` 也收；`resolved_by` 也可以写成 `user_id`）。先按条目所在的图做「授权配置」的权限判定，被拒时照权限接口的样子返回，另加一个 `level` 键（与 `denied_level` 同值，契约 015 读的是 `level`）。没有这个条目 404 `CONFLICT_UNKNOWN`；已经裁决过 409 `CONFLICT_ALREADY_RESOLVED`，带 `ok:false`，条目与审计都不动；`resolution` 不是三者之一、人工改值没给值，400。
- 裁决生效（`ops::resolve`）：选中的值由服务器写成一个 op——字段冲突是 `set_property` / `set_tag`，拓扑冲突是当初并列的那个操作本身——修订加一、进 `ops` 表、广播 `op_applied`（`op_id` 是 `resolve-<conflict_id>`）；审计记一行 `module='冲突裁决'`、`action='conflict_resolved'`、`user_id` 是裁决人，`change_text` 写「裁决 <id>：对象.字段 甲方 …，乙方 …，<resolution>，最终 …」；然后把条目标成 `已裁决`，人工改值的值进 `resolved_value` 列。

**结果**

| 断言 | 建议状态 | 证据（`E7\logs\`） |
|---|---|---|
| `VAL-CONF-010` | passed | `c03_resolution.result.txt`；`c03-010-request.json`、`c03-010-response.json`（200）；`c03-010-row.json`（已裁决 / 采用甲方 / u4）；语义读回 设计压力=1.6MPa；`ws-c03-s1.jsonl` 里的 `op_applied`（`resolve-…`） |
| `VAL-CONF-011` | passed | `c03-011-*.json`；最终值 2.5MPa |
| `VAL-CONF-012` | passed | `c03-012-*.json`；最终值 2.0MPa；`resolved_value=2.0MPa`（表里与响应里） |
| `VAL-CONF-013` | passed | `c03-013-audit-u4.json`：三个条目各一行，`user_id=u4`、`object_ref=P-1001`，`change_text` 里 1.6MPa、2.5MPa 与各自的最终值都在；`c03-013-audit-http.json` |
| `VAL-CONF-014` | passed | `c03-014-response.json`（409，`ok=false`，`CONFLICT_ALREADY_RESOLVED`）；行仍是 采用甲方 / u4；该条目的裁决审计行 1 → 1 |
| `VAL-CONF-015` | passed | `c03-015-response.json`（403，`PERM_DENIED_ROLE`，`level=role`）；条目仍 `待裁决`、`resolution` 为 NULL |
| `VAL-CONF-016` | passed | `c03-016-pending.json`（1 条，四个字段非空）、`c03-016-resolved.json`（3 条）、`c03-016-counts.json` |

七条逐句成立。场景脚本 `E7\c03_resolution.py`，20 项检查全过（服务器带 `--tie-window-ms 1200`，并列条目是这样造出来的）；`c01`、`c02` 回归各 24、17 项全过；`ocs_collab_server` 129 + 12 条单测通过（新增 6 条在 `http.rs`：三种裁决各自写回、广播并留痕；重复裁决；非管理员；按图纸与状态检索；拓扑条目的裁决；读不懂的裁决），clippy 0 告警。

`VAL-CONF-018`（审计日志改删返回 405）不在 M7 四项的 `fulfills` 里，M1 已经实现，`http.rs` 里有它的单测；这一轮没有另外取证。

到这里 `VAL-CONF` 的 18 条里 M7 负责的 17 条都有了建议状态。剩下的一项 `cross-area-flows`（`VAL-CROSS-001`–`012`）没有开工，见 §14.7。

### 14.6 定下来的事里要用户过目的

每件写现状、选项、倾向。

1. **并列窗口的默认值。** 现状：`--tie-window-ms` 默认 0，不暂存，两个人同时改同一字段时先到的生效、后到的让位，永远不会生成冲突条目；`VAL-CONF-004`、`-008`、`VAL-CROSS-007` 取证时要显式带这个参数。选项：保持 0；或者给一个几百毫秒的默认值，代价是每一步编辑的广播都晚这么久。倾向保持 0——契约 004 自己写的是「服务器配置为无法用基线判定」，M1–M6 的断言也都是在「到了就生效」上验的。
2. **`VAL-CLI-011` 的持有者规则**（§14.3）。现状：切换那一刻最早建立的活跃编辑会话。倾向保留，待确认。
3. **新增的两个错误码。** 让位用 `OP_SUPERSEDED`，改已删对象用 `OBJECT_DELETED`，都走 `error` 帧（A.4：「`op_rejected` 不存在」）。契约只说「`error` 表示基线落后」「被覆盖通知」，没给码名。选项：让位改回 `STALE_BASE_REVISION`。倾向分开：`STALE_BASE_REVISION` 的意思是「追上来再发一遍」，让位的意思是「不要再发」，客户端要分得开。
4. **「被引用」的范围。** 现状：只有管路端点（`add_pipe_run` 的 `from` / `to`）算引用，管路上的管件被删不拦。选项：管件也算。倾向不算，理由见 §14.5.2。
5. **三处契约字面。** 001 的证据栏写的是 GUI 的 xdata 读回，实际读的是服务器的语义读回；017 写「`ok=false`」，WS 的 `error` 帧没有这个键，码名是 A.4 的 `OBJECT_REFERENCED`；015 读的 `level` 现在与 `denied_level` 并存。建议把 001 的证据改成「服务器 `GET /drawing/{id}/objects` 的读回」，017 的「返回 `ok=false`，`code` 表示被引用不可删（如 `REFERENCED_OBJECT`）」改成「返回 `error`，`code='OBJECT_REFERENCED'`」。
6. **出图另外几条路没有图集核对**（§14.2 末）。现状：控制通道的 `plot` 动作与 PDF 导出不经 `resolve_plot_job`。选项：让它们也走同一遍核对（PDF 改成用作业带着的快照）；或者留着。倾向修，但它在这个 mission 之外，是上游出图的代码，等一句话再动。
7. **会话绑哪张图**（§14.4 末）。现状：第一次用到时在前面的那张图。选项：改成命令行上给的那个文件所在的标签页。倾向后者，要动打开文件的路径，没有做。（补记：`c1a94dfde` 做了——带文件启动时会话只认显示那个文件的标签页，文件还没载入时入站变更在队列里等；不带文件启动仍按在前面的图算。见 §15.2 的 B1。）

### 14.7 没做的、没核实的

- **`cross-area-flows`（`VAL-CROSS-001`–`012`）没有开工。** 它要的客户端功能，按上一轮 §13.10 的清单对一遍：
  - 两步编辑之间的基线过期与重发：对改属性、改位号已经由服务器的合并解决（§14.5.1）；客户端收到 `STALE_BASE_REVISION` 之后追上再发，没有做。
  - 会话与标签页的绑定：做了（`13dad6ef8`）。
  - 新建与删除实体的广播：服务器认 `insert_symbol` / `delete_object` / 拓扑类 op 了，客户端还不发、收到也不会建或删实体。
  - 只读总闸（命令、REST 直接写、撤销重做、属性面板、夹点）：没有做，仍然只有命令入口那一道。
  - 只读状态跨重连、后加入只读图纸的会话降级：没有做。
  - 客户端显示 `conflict`、处理 `OP_SUPERSEDED` / `OBJECT_DELETED`：没有做，现在只会把服务器的 `error` 原文印在命令行上。
  - 重连后补上错过的 op（服务器有 `ops` 表与 `Store::load_ops`，还没有接到 `hello` 上）：没有做。
  - 管路覆盖标记随变更广播：没有做。
  - 服务器保证并发插入的位号唯一（`VAL-CROSS-010`）：没有做。
- 远端变更在本地命令执行到一半时到达会截断那条命令的撤销步（§13.10），没有处理。
- §14.2 末尾两处产品上的观察（出图的另外几条路、`new()` 读真实的 `settings.json`）没有动。
- `VAL-CONF-018` 这一轮没有另外取证。
- wasm 目标的检查：mission 的 `AGENTS.md`、`services.yaml`、`mission.md`、验证契约里都没有 wasm 这个词，它不在 mission 的门禁里；没有跑。

### 14.8 全量测试与门禁

在图集与 lasso 两处修正之后的树上（`821e11395`，`B\r3c-*`）：

| | 结果 |
|---|---|
| 全量 20 个测试程序 | 6608 通过 / 0 失败 / 43 忽略，20 个都退出码 0 |
| lib 整套并行 8 遍 | 每遍 6030 通过 / 0 失败 / 41 忽略（`B\r3c-lib-full\summary.txt`） |
| 备份面板两个分组 | `pid_backup` 179 通过 / 2 忽略，`pidbackup` 24 通过 / 1 忽略 |
| 端到端八个场景（`E7\m6-rerun\`，M6 的 `run_all.ps1` 换了证据目录） | 108 项全过；每个场景前后指针位置与前台窗口不变；没有残留进程（`logs\run_all.summary.txt`） |

之后的每一项各自跑过相关的测试（见各节）。`test --workspace --no-run` 第一次用 `-j 10`，rustc 报 `memory allocation of … bytes failed` 崩了（`B\r3c-test-norun.crashed-oom.log`），那一遍不算；改用 `-j 4` 之后通过。

在最后的提交 `dacf954ba` 上已经构建完全部二进制与测试程序（`B\r3z-summary.txt`，03:15 结束），全量与端到端的重跑见本节后面的补记。

**补记（2026-11-11 09:19–09:25，`dacf954ba` 的构建，`B\r3z-*`）**

| | 结果 |
|---|---|
| 全量 20 个测试程序 | 6664 通过 / 0 失败 / 44 忽略（`B\r3z-test.log`） |
| lib 整套并行 8 遍（`--workspace` 构建出来的那个 lib 测试程序） | 7 遍 6054 通过 / 0 失败 / 42 忽略；**第 6 遍 1 条失败**：`app::update::viewport::rapid_drawing_tests::throttle_axis_lock_matches_authoritative_move`（`B\r3z-lib-full\run6.log`）。根因与修正见 §15.3 |
| 备份面板两个分组（并入 `454db6685` 之后） | `pid_backup` 196 通过 / 3 忽略，`pidbackup` 24 通过 / 1 忽略 |
| 协同分组 | 92 通过 |

这 8 遍在跑的时候，工作区里 `src/app/collab/` 与 `src/app/update/viewport.rs` 正在被编辑。测试程序是此前构建好的，运行时会读源码的测试只有一条（`nothing_under_this_module_is_asynchronous_code`，读 `src/app/collab/` 找四种异步写法），当时的改动里没有这四种写法，它也没有失败；别的测试读的是夹具与资源，不读源码。端到端八个场景在 `dacf954ba` 上没有单独重跑，放在 §15 这一批修正之后一起跑。

### 14.9 磁盘与环境

- 这一轮没有删过任何东西。D 盘开工时 21.8 GB，中途最低 18.4 GB，09:15 是 25.9 GB（机器上同时有别的构建在进出）。
- 03:14 会话被外部中断，09:17 接着做；中断时没有进行到一半的 git 操作，没有残留进程。
- mission 留下的三个 `pwsh` 与 `.ocs.lock` 没有碰。

## 15. 执行记录：2026-11-11 第四段（对照独立审查逐条修）

独立审查读的是 `3d4281eb3`，报告在 `D:\work\plant-code\cad\ocs-mission-backup-20261110\reviews\m6-review-3d4281eb3.md`。这一节逐条对照此后的提交。次序按「会写错数据的在前」：B1、S3、S6、S4，然后 S1、S2、S5、S7、S8。每做完一项提交一次，本节跟着改。

本节的路径缩写同前：`B` = `D:\work\plant-code\cad\ocs-mission-backup-20261110\logs`，`E7` = `D:\scratch\m7-validation`。

### 15.1 提交（`6cf766be5` 之后，都在 `main` 上，没有 push）

| 提交 | 内容 |
|---|---|
| `c1a94dfde` | B1 余下三处：入站变更等图纸、占用色只涂会话那张图、启动窗口不丢变更 |
| `71c95e90d` | 测试不再靠墙钟：指针移动的合并闸、双击窗口（§15.3） |
| `9cc429cce` | S3、S7、N2、N5：变更不再悄悄丢掉；丢了、应用不上都标出来并停止提交；服务器按提交次序发布 `op_applied` |
| `96f21cf91` | N8：正常退出时删掉自动化通道的描述文件 |
| `acc55ff73` | S6 余下四条：多实体对象按位号能找到、改名没应用不再当成已应用、认不出的变更要报、占用用图纸里的写法 |
| `a5f28544b` | N1、N7：会话建成后重连阶梯从头开始；异步写法的检索递归 |
| `be0582ea7` | S4、N3：撤销只撤自己的改动，别人的变更不进本地撤销栈（行为改动，单独提交） |
| `97ff6d5de` | S2、S8：只读由服务器把关；后加入的会话当场被告知；持有者在一步里选定 |
| `e8b4034de` | S1 与只读总闸：每一步编辑必经之处拦下；断线不解除只读；被拒的写操作带层级并上报留痕 |
| `a0f33b354` | `welcome.peers` 不含连入者自己（行为改动，单独提交） |

### 15.2 发现项对照表

「守着的测试」写测试名；lib 的在 `src/app/` 下对应模块的 `tests` 里，服务器的在 `crates/ocs_collab_server/src/` 下。

| 项 | 现在 | 提交 | 守着的测试 | 还差什么 |
|---|---|---|---|---|
| **B1** 会话不绑标签页 | 已修 | `13dad6ef8`、`c1a94dfde` | (a) 入站按绑定的标签页 id 找：`a_remote_change_lands_on_the_sessions_drawing_not_on_the_tab_in_front`；找不到时留在队列里：`a_change_that_arrives_before_the_drawing_is_open_waits_for_it`（未载入）、`a_change_for_a_closed_drawing_stays_queued_and_touches_no_other`（已关闭，走 `Message::TabClose`）。(b) `push_undo_entry`、撤销、重做只在绑定的那一页广播：`an_edit_on_another_drawing_is_not_broadcast_as_the_sessions`（三步都查）。(c) 占用色只涂绑定的那一页：`a_claim_colours_the_sessions_drawing_only`。(d) 启动窗口：`on_tick` 在队列非空时回去应用（上面第二条走的就是这条路）；带文件启动的会话只认显示那个文件的标签页：`a_session_started_on_a_file_waits_for_that_file_not_the_tab_in_front` | 会话的图关掉之后会话还连着（占用不释放，队列只进不出）；同一个文件关掉再开不会重新绑上。关图要不要结束会话，见 §15.6 |
| **S1** 只读放行名单里有写命令 | 已修 | `e8b4034de` | 名单：`commands_that_write_are_refused_and_the_forms_that_read_are_not`、`every_command_a_readonly_session_may_run_leaves_the_drawing_as_it_was`。总闸（名单之外的门）：`a_write_that_gets_past_the_command_gate_is_taken_back` | 名单现在只是「提前拒绝」，真正的保障是 `push_undo_entry` 处的总闸（§15.9）。`VAL-CLI-011` 要端到端重跑之后才能改回「成立」 |
| **S2** 只读只靠客户端自觉，断线即解除 | 已修 | `97ff6d5de`（服务器）、`e8b4034de`（客户端） | 服务器：`an_op_from_a_session_that_may_not_write_is_refused_and_put_on_record`、`on_a_readonly_drawing_only_the_holder_writes`（含后加入的会话当场降级）、`a_role_that_does_not_edit_joins_as_a_viewer_and_is_told`、`on_a_single_editor_drawing_the_first_editor_writes_and_the_second_does_not`。客户端：`a_session_that_may_not_write_still_may_not_while_disconnected` | 图纸离开只读时没有消息告诉被降级的会话；`welcome` 与紧跟的 `readonly_downgrade` 是两帧，中间有一次消息循环的空隙（期间发出的 op 服务器照样拒绝） |
| **S3** 变更丢了基线照样前进 | 已修 | `9cc429cce` | 通道满：`a_peers_change_waits_for_room_in_the_channel_instead_of_being_dropped`、`a_waiting_change_gives_up_when_the_subscription_is_gone`、`…_when_the_app_is_quitting`。修订不连续：`a_revision_that_skips_one_marks_the_session_and_stops_its_ops`、`a_revision_already_seen_is_dropped`、`coming_back_to_a_room_that_moved_on_marks_the_session`。应用不上：`a_change_this_drawing_cannot_follow_stops_the_sessions_writes`、`an_op_this_client_cannot_read_marks_the_session_too`。面板：`a_session_that_parted_from_the_room_draws_the_badge`。服务器按提交次序发布：`an_applied_op_is_announced_before_the_drawing_is_unlocked` | 标记之后没有自动恢复，只能取得最新图纸重开。重连时由服务器补发错过的 op 放在 `cross-area-flows`（`VAL-CROSS-008`）。自己的 op 被服务器拒绝后本地改动还在，也是一种不一致，同样在那里处理 |
| **S4** 入站变更进了本地撤销栈 | 已修 | `be0582ea7`（单独提交，退回只需 revert 它） | `undo_takes_back_this_persons_step_and_leaves_a_peers_change_alone`、`undo_puts_back_ones_own_field_and_keeps_the_field_a_peer_changed`、`undoing_a_value_a_peer_has_since_overwritten_changes_nothing_and_says_nothing`、`a_peers_change_leaves_this_persons_redo_in_place`、`an_undo_after_a_peers_retag_names_the_object_by_the_tag_it_has_now`；`pidpipe` 的 `undoing_ones_own_run_edit_keeps_what_a_peer_later_set_on_the_line` | 做法见 §15.7。属性溢出到 XRecord 的对象，撤销后别人的改动由重放写回，但向房间说的差里不含这类对象的属性 |
| **S5** 远端应用不调 `propagate_pid_chain` | 已修 | `cce14ba77` | `a_remote_line_change_is_carried_down_to_runs_and_segments`（`src/app/commands/pidpipe.rs`） | 拓扑表里的属性副本由 `propagate_pid_chain` 末尾的 `sync_props_from_document` 对齐 |
| **S6** `object_ref` 的边界 | 已修（管线类的新建除外） | `cce14ba77`、`acc55ff73`、`e403d035f`、`b4b302f4a` | 空位号对象：`a_change_naming_no_tag_is_not_applied_by_its_handle_alone`、`a_change_to_an_untagged_object_is_not_stated`。多个实体同一个位号：`a_line_drawn_as_several_entities_is_one_object_to_a_change_naming_its_tag`、`only_the_pieces_of_one_line_count_as_one_object`、`a_change_made_to_several_entities_of_one_object_is_stated_once`、`handles_that_only_partly_match_are_not_believed`；`pidpipe` 的 `a_remote_change_to_a_line_drawn_in_several_stretches_reaches_the_line`。`already_retagged`：`a_retag_fitting_several_symbols_is_reported_even_when_the_new_tag_exists`、`a_retag_this_drawing_never_had_is_reported_though_the_new_tag_is_here`。认不出的变更：`changes_this_client_cannot_read_are_named_not_dropped`、`an_op_read_only_in_part_is_applied_as_far_as_it_goes_and_said_to_be_short`。规范形式：`a_typed_tag_is_claimed_as_the_drawing_spells_it`、`a_typed_tag_fitting_two_spellings_stands_as_typed`、`a_release_finds_the_claim_by_the_spelling_it_was_made_under` | 两端各自新建的同位号对象：符号的新建现在广播，服务器把后到的那个改派到下一个空号（`two_symbols_placed_under_one_tag_end_up_with_two_tags`、`a_symbol_the_server_renumbered_is_renamed_where_it_was_placed`，端到端 `x10`）。只管独立放置的符号；管线、管路、管件的新建仍不广播（§18.4） |
| **S7** `op_id` 只在进程内唯一 | 已修 | `9cc429cce` | `two_sessions_never_make_the_same_op_id` | 被服务器拒绝的 op 的 id 仍留在 `own_ops` 里（最多 64 个）。id 带会话号之后它不会再被别人的 op 撞上 |
| **S8** 持有者规则的竞争与边界 | 已修 | `97ff6d5de` | `the_holder_is_the_first_to_have_joined_among_sessions_with_a_socket`、`switching_to_readonly_downgrades_everyone_but_the_holder`（持有者也收到点名自己的消息）；客户端 `a_downgrade_naming_this_session_as_the_holder_leaves_it_writing` | 规则本身（谁算持有者）仍待确认，见 §14.6 之 2。持有者只记在内存里 |
| **S9** 记账 | 已改 | 本文档 | — | §0 与 §13.3 改成 13 条成立、7 条部分成立（003、005、009、011、013、018、019） |
| **N1** 重试计数建成会话后不归零 | 已修 | `a5f28544b` | `a_session_that_came_up_starts_the_retry_ladder_again` | |
| **N2** `shutdown()` 预算过后无条件 `join()` | 已修 | `9cc429cce` | `a_wedged_worker_does_not_hold_the_quit_past_its_budget`、`a_worker_that_stops_is_joined_within_the_budget` | |
| **N3** 本地开着的撤销步被远端折叠关掉时不广播 | 已修 | `be0582ea7` | `a_step_still_open_when_a_peers_change_arrives_goes_out_as_this_persons` | 那一步仍然会被关掉（一条命令的撤销被分成两步），只是不再丢；不截断需要让记录器暂停，文档库没有这个接口 |
| **N4** `__overrides` 不随变更走；远端改位号一律 `manual` | 没修 | — | — | |
| **N5** 心跳把旧基线写回；`welcome` 的修订读在进房间之前 | 已修 | `9cc429cce` | `rewriting_a_session_row_never_takes_its_baseline_back`、`a_session_that_joins_hears_the_op_right_after_the_revision_it_was_told` | |
| **N6** 客户端日志不分级；连接地址原样进日志 | 已修 | `c1db1a03d` | 见 §17.5 | |
| **N7** 异步写法的检索只扫一层 | 已修 | `a5f28544b` | `nothing_under_this_module_is_asynchronous_code` 现在递归 | |
| **N8** 证据目录里的描述文件 | 已修 | `96f21cf91` | `a_retired_descriptor_is_gone`、`publishing_does_not_overwrite_a_descriptor_already_there`、`quitting_retires_the_descriptor_this_process_published` | 删除记录见 §15.4 |
| **N9** 证据的卫生 | 三条里修了两条半 | `c1db1a03d`，脚本在 `E7\m6-rerun-2\` | 见 §17.5 | 「各处截图只查文件存在」没有改 |

### 15.3 新查到的零星失败（不是协同的代码）

**`throttle_axis_lock_matches_authoritative_move`**：`dacf954ba` 的 lib 全量 8 遍里失败 1 遍（§14.8 补记）。和 `821e11395` 修的 `lasso_press_drag_selects` 是同一类：`on_viewport_move` 用墙钟量一遍指针移动处理的耗时，达到 8 毫秒就把下一次移动扣下；测试里图是空的，机器忙（当时另一条线在构建）时一遍超过 8 毫秒，测试紧接着发的第二次移动被扣下，光标停在第一次的位置——失败输出里 x=40 正是第一次移动的位置。`821e11395` 只改了那一条测试，当时写了「另有约 70 处 `Message::ViewportMove` 的测试属于同一类」。`71c95e90d` 把这一类从根上去掉：耗时由 `move_pass_cost` 给出，正式构建里是 `started.elapsed()`，测试构建里是测试自己声明的值（缺省为零），合并闸的测试改成声明耗时。

定向复现没有成功：`app::update::viewport` 一组 `--test-threads 64` 连跑 60 遍、`app::` 全组 64 线程连跑 12 遍，修之前修之后都是 0。它的修前失败率只有全量那一次，1/8。

**`double_clicking_a_hatch_starts_hatchedit_on_it`**：上面那次定向（viewport 组、64 线程、60 遍）里目标没有复现，这一条却失败了 18 遍（修合并闸之前的程序）与 32 遍（之后的程序）。测试把双击的第一下记成 100 毫秒之前，双击窗口 400 毫秒，留给第二下 300 毫秒；64 个线程同时建应用时线程等不到核。`71c95e90d` 把第一下的时刻记到时钟前面，修之后同样的 60 遍是 0。缺省线程数下这一条从来没有失败过。`block_editor_tests.rs` 里同样写法的一处一并改了。

| | 修之前 | 修之后 |
|---|---|---|
| `throttle_axis_lock…`，lib 全量缺省线程 | 1/8（`B\r3z-lib-full`） | 0/8（`B\r4c-lib-full`） |
| `throttle_axis_lock…`，viewport 组 64 线程 | 0/60（`B\r4b-before-repro`） | 0/60（`B\r4b-after-repro`） |
| `throttle_axis_lock…`，`app::` 全组 64 线程 | 0/12（`B\r4b-before64-repro`） | 0/12（`B\r4b-after64-repro`） |
| `double_clicking_a_hatch…`，viewport 组 64 线程 | 18/60、32/60（同上两个目录） | 0/60（`B\r4c-after-dblclick-repro`） |

### 15.4 N8：删掉的描述文件

- `D:\scratch\m6-validation\work\appdata\<实例>\OpenCADStudio\automation\<32 位十六进制>.json`，共 26 个：`seed-make` 1、`seed-reopen` 1、`smoke` 1、`t10-u1` 2、`t10-u2` 2、`t11-a` 3、`t11-b` 3、`t20-u1` 1、`t21-u1` 3、`t21-u2` 3、`t22-u1` 1、`t22-u2` 1、`t23-u1` 1、`t24-u1` 1、`t25-offline` 1、`t25-plain` 1。与审查报告的个数逐项相同。
- `E7\m6-rerun\work\appdata\<实例>\OpenCADStudio\automation\` 下 13 个（本轮重跑 M6 场景留下的，每个实例 1 个），同样删除。
- 逐个文件的完整路径与字节数在 `D:\work\plant-code\cad\ocs-mission-backup-20261110\n8-deleted-descriptors.txt`，不含内容。删之前确认过没有 `OpenCADStudio` 或 `ocs_collab_server` 进程在跑。
- 两个证据目录里别的东西没有动；16 个 `automation` 目录本身留着（空的）。
- 程序这边：正常退出时自己删（`96f21cf91`）。被强杀或崩溃的进程仍会留下描述文件。

### 15.5 测试

| 提交 | 跑了什么 | 结果 |
|---|---|---|
| `c1a94dfde` | lib 全量 1 遍；协同分组 | 6058 通过 / 0 失败 / 42 忽略；96 通过。红灯输出 `B\r4a-b1-red.log` |
| `71c95e90d`、`9cc429cce`、`96f21cf91`（同一次构建，三者改的文件互不相交） | lib 全量 8 遍；协同分组 | 每遍 6075 通过 / 0 失败 / 42 忽略（`B\r4c-lib-full\summary.txt`）；108 通过 |
| `9cc429cce` | 服务器三个测试程序；`clippy -p ocs_collab -p ocs_collab_server --all-targets` | `ocs_collab` 76、`ocs_collab_server` lib 132、main 12，全部通过；0 警告。红灯输出 `B\r4c-srv-red.log` |

| `acc55ff73`、`a5f28544b`（同一次构建，文件不相交） | lib 全量 3 遍；协同、`pidpipe` 分组 | 每遍 6087 通过 / 0 失败 / 42 忽略；119、27 通过。红灯输出 `B\r4d-s6-n1-red.log` |
| `be0582ea7` | lib 全量 3 遍；协同、`pidpipe` 分组 | 每遍 6094 通过 / 0 失败 / 42 忽略；125、28 通过。红灯输出 `B\r4e-s4-red.log`（改回旧行为后 7 条失败） |
| `97ff6d5de` | 服务器三个测试程序与 clippy；客户端 lib 全量 1 遍 | `ocs_collab` 78、`ocs_collab_server` lib 138、main 12 通过，0 警告；6095 通过 / 0 失败 / 42 忽略。红灯输出 `B\r4f-srv-red.log`（去掉把关后 5 条失败） |
| `e8b4034de` | lib 全量 3 遍；协同分组 | 每遍 6100 通过 / 0 失败 / 42 忽略；131 通过。红灯输出 `B\r4g-readonly-red.log`（3 条失败） |
| `a0f33b354` | 服务器三个测试程序与 clippy；客户端 lib 全量 1 遍；协同分组 | 78、138、12 通过，0 警告；6102 通过 / 0 失败 / 42 忽略；133 通过 |
| `653c934cd`（合并另一条线到 `d9931ad61` 之后） | `build --workspace`、`test --workspace --no-run -j 4`；全量 20 个测试程序；lib 全量 8 遍；`pid_backup`、`pidbackup`、协同分组 | 两次构建退出码 0（241 秒、357 秒）；20 个程序 6755 通过 / 0 失败 / 44 忽略（`B\r5a-test.log`）；lib 每遍 6134 通过 / 0 失败 / 42 忽略（`B\r5a-lib-full\summary.txt`）；222 通过 / 3 忽略、29 通过 / 1 忽略、133 通过 |

端到端场景与主程序的 clippy 在这一批修正之后一起跑，结果补在本节。

### 15.6 这一段里要用户过目的

1. **会话的图关掉之后会话怎么办。** 现状：会话还连着，占用不释放，别人的变更在队列里等一张不会回来的图。选项：关图即结束会话（发 close 帧，占用随之释放）；或者保持现状。倾向关图即结束——一个没有图的会话占着位号没有意义——但这是行为上的决定，没有做。
2. **图纸一旦切成只读，就不能再经接口切回去。** 现状：`POST /drawing/{id}/state` 要「授权配置」权限，而授权配置算写操作，只读状态在状态这一层拒绝全部写操作，于是对只读图纸的任何状态切换都回 403（做只读把关的测试时撞到的，`r4f-srv-test-2.log`；这是 M2 的行为，没有改）。`architecture.md` 第 726 行自己两句话对不上：「三态全互通，由授权配置权限持有者操作」与「只读状态下拒绝全部 5 个写操作：…授权配置…」。选项：状态切换这个接口不受状态这一层限制（只看角色与 PBS）；或者保持现状，只读是单向的，要改回去只能动数据库。倾向前者，否则「三态全互通」做不到；没有做。
3. **图纸离开只读时怎么告诉被降级的会话。** 现状：协议里没有这样的消息，被降级的会话要到重连才知道可以写了。选项：加一条消息（比如 `access_restored`），或者复用 `readonly_downgrade` 并加一个「已恢复」的标志。倾向加一条消息；因为上一条还没定（现在根本离不开只读），没有做。
4. **权限表之外的用户与图纸。** 现状：用户不在 `users` 表里、或图纸不在 `drawings` 表里（或由 `hello` 自动建出来、没有挂到 PBS 树上）时，服务器不限制写——没有播种的服务器什么都不限制，M1–M6 的测试与契约场景都靠这一点。选项：改成「不在表里就不能写」。倾向保持：权限表是安装时填的，没填等于没有设限；协同协议本身也没有身份认证（`--user` 是自报的）。

### 15.7 S4 是怎么做的（`be0582ea7`）

默认决定是「撤销只撤自己的改动」。难处在于：本地撤销步里存的是它碰过的实体在那一刻的整份镜像，撤销就是把镜像放回去；别人此后改过同一个实体（哪怕是另一个字段），这个改动不在镜像里，光放回去会把它一起抹掉。

做法：

- 远端变更直接写进图纸，不开撤销步，不动重做栈。写之前把本地还开着的那一步关掉并作为用户自己的一步广播（N3）。
- 每个远端变更连同它写到的实体记进一个按到达次序编号的日志；每个撤销步记下落栈时已经收过多少条。
- 在会话那张图上撤销或重做之后，把比这一步晚的远端变更按次序再写一遍（按当时写到的实体写，不按名字找——撤销可能把位号改回去了），然后照常重算下传与位号标注。
- 撤销、重做向房间说的话，是「撤销之前的图」对「现在的图」的差。自己改过、别人后来又改了的字段，撤销之后仍是别人的值，不向房间说任何话。
- 没有比这一步更晚的远端变更时（包括不在协同会话里），撤销与重做走的是原来的代码，一行不变。

可观察的结果：

| 情形 | 撤销之后 | 向房间说 |
|---|---|---|
| 自己改了 A 的位号，别人改了 B 的属性 | A 的位号回去，B 的属性还在；别人的变更不占撤销栈的一步 | 只有 A 的位号改回 |
| 同一个对象，自己改字段 F，别人改字段 G | F 回去，G 还在 | 只有 F |
| 同一个字段，自己改成 b，别人后来改成 c | 仍是 c | 什么都不说 |
| 自己撤销之后别人改了别的对象 | 重做栈还在，重做照常 | 重做的那一条 |
| 自己改了属性，别人把这个对象改了名 | 属性回去，名字是别人改的新名字 | 用新名字指这个对象 |
| 自己改了管路，别人后来改了管线的字段 | 管路自己的字段回去；管路与管段仍是管线现在的值 | 管路自己的字段 |

### 15.8 `welcome.peers` 不含连入者自己（`a0f33b354`）

用户的决定：不含。单独一个提交。

做了什么：

- 服务器 `hello`：`welcome.peers` 是图上其他活跃会话。
- 客户端：在线名单就是别人；`/api/v1/collab/status` 的 `peers` 是别人（`architecture.md` A.5 的例子就是这样），另给一个诊断字段 `online`（总人数，含本会话）；协同面板把本会话那一行排在最前、标「本机」，标题里的人数是总人数，只有自己时写「暂无其他在线用户」。连上时把名单里与自己会话号相同的一项去掉，所以把自己列进去的旧服务器读出来是一样的。
- 两条钉着「含自己」的服务器单测改了：`hello_opens_a_session_and_returns_welcome`（原来断言 peers 里有自己，现在断言第一个会话的 peers 为空）、`welcome_payload_contains_peers_and_claims`（原来长度 2，现在长度 1、是先到的那一个）。第三条 `welcome_lists_active_peers_and_outstanding_claims` 看的是注册表不是帧，没有动。

契约里要跟着改的一句（契约在 `.factory` 下只读，没有动）：

| 位置 | 原句 | 建议改成 |
|---|---|---|
| `VAL-CLI-003` 2026-11-09 裁决，条目 1 | `u2` 侧收到 `welcome` 且 `peers` 长度为 2、包含 `u1` | `u2` 侧收到 `welcome` 且 `peers` 长度为 1、包含 `u1`（`welcome.peers` 不含连入者自己，与 `VAL-CLI-013`、`VAL-CROSS-006` 一致） |
| 同上，条目 3 | `u2` 侧自己维护的在线列表计数从 2 变 1（打印 `peersBefore` / `peersAfter`） | 不用改字面：在线列表是「自己加上 `welcome.peers` 里的别人」，2 → 1 仍然成立 |

`VAL-CLI-013` 的「`welcome.peers` 为空数组（首次连入）」「`u2` 的 `welcome.peers` 长度为 1 且含 `u1`」与 `VAL-CROSS-006` 的「`welcome.peers` 为空」现在按字面成立，不用改。

端到端脚本里按旧含义写的检查。原始证据脚本在 `D:\scratch\m6-validation\` 下，没有动；`E7\m6-rerun\`（`821e11395` 那次重跑的脚本与证据）也没有动；改的是为这一批修正新建的重跑目录 `E7\m6-rerun-2\` 里的脚本，每一处都留了带日期的注释：

| 脚本 | 原来 | 现在 |
|---|---|---|
| `t10_broadcast.py`（`VAL-CLI-013`） | 注记「契约字面：`welcome.peers` 为空数组 = False（含自己，长度 1）」 | 检查：`peers` 长度 0 且 `online` = 1 |
| 同上 | 检查：u1 的 `peers` 长度 1 → 2 | 检查：`online` 1 → 2，`peers` 长度 0 → 1 |
| 同上 | 注记「契约字面：u2 的 `welcome.peers` 长度为 1 = False（含 u2 自己）」 | 检查：长度为 1 |
| `t23_stale.py`（`VAL-CLI-003`） | 检查：`welcome.peers` 长度为 2、含 u1 | 检查：长度为 1、含 u1 |
| 同上 | 在线列表 = `welcome.peers`（含自己），2 → 1 | 在线列表 = 自己 + `welcome.peers`，2 → 1 |

`t11_same_user.py`、`t20_session.py` 里的检查（「B 的 peers 含 A」「peers 是数组」）不受影响。这些脚本改完之后还没有重跑；重跑结果补在 §15.5。

### 15.9 只读是怎么把关的（`97ff6d5de`、`e8b4034de`）

审查 S1、S2 与 `VAL-CROSS-004` / `011` 要的是同一件事：不可写的会话改不了图，改不了要有据可查。做成两道，各管一头。

**服务器**（真正的把关）：`access::write_access` 给出一个会话此刻能不能写——已有的权限链（角色 → PBS ACL → 状态，含单人编辑）加上两条链不知道的事：只读图纸仍有一个写的人（持有者）；以查看者身份进来的会话不写。

- 不可写的会话发来的 `op` 被拒：`error` 帧带 `PERM_DENIED_ROLE` / `PERM_DENIED_PBS` / `PERM_DENIED_STATE` 与 `level`，图面、修订、房间都不动，`audit_log` 记一行 `write_denied`（`object_ref` 是它要改的对象，`change_text` 是它要做的改动加上拒绝的层级与原因）。
- `hello`：角色不允许编辑的用户，会话一律以「只读」模式建立（不占单人编辑的位置，不会被选作持有者）；不可写的会话在 `welcome` 之后当场收到 `readonly_downgrade`，里面带 `level` 与 `code`。
- 用户或图纸不在权限表里的（没有播种的服务器、测试里随手起的用户），不受限，和以前一样。
- 持有者：切只读的那一步里，在会话注册表的锁内选定、记下、通知。选的是「在图上时间最长的、有 socket 的活跃编辑会话」（按连入次序，不按秒、不按随机的会话号）。持有者自己也收到消息（点的是它自己的会话号）。

**客户端**（礼貌，加一道总闸）：

- 总闸设在 `push_undo_entry`——每一步编辑不管从哪个门进来（命令、属性面板、夹点、调色板拖放、REST 直接写）最后都交到这里。只读会话在会话那张图上做出来的一步不落栈，当场退回去：图纸、撤销栈、重做栈都和这一步之前一样，不向房间说话。撤销与重做不经过这里，另外拦。
- 命令入口的放行名单留着，只负责「在命令开口提问之前拒绝」；名单上放错了的动词现在也改不了图。`MEASURE`、`FIND`、`PIDGROUP`、`AUDIT` 移出名单，`PIDLEGEND` 只放行 `REPORT` / `LIST` / `EXPORT`。
- 断线不解除只读。新会话建起来时由服务器重新决定。
- 每一次拒绝都告诉服务器（新消息 `attempt`），服务器记 `write_denied`；回给调用方的是 `ok=false`、`code`、`level`，HTTP 403。`code` 在角色与 PBS 这两层是 `PERM_DENIED_ROLE` / `PERM_DENIED_PBS`，在状态这一层仍是 `COLLAB_READONLY`（`VAL-CLI-011` 一直是这个码）。

协议上多了三样（都向后兼容）：`readonly_downgrade` 的可选字段 `level`、`code`；客户端消息 `attempt { object_ref, text }`；`error` 帧在拒绝写操作时带 `level`。

## 16. M7 `cross-area-flows`（`VAL-CROSS-001`–`012`）：开工前列出的断言与取证办法

`features.json` 里这一项的说明：跨区域端到端流程，每个流程真实跨越至少两个区域（GUI + 服务器 + 持久化），证据包含截图、WS 消息序列与数据库查询三类；前置是 `collab-client-broadcast`、`conflict-resolution`、`pipe-topology-persistence`、`pid-dwg-roundtrip` 完成。

取证办法与前面一样：服务器与 GUI 用 `main` 尖端构建出来的可执行文件，GUI 起在单独的隐藏桌面上、只走 `--http` 的本机 REST 通道，每个场景前后各记一次指针位置与前台窗口；证据放在 `E7` 下，不动 `D:\scratch\m6-validation\`。WS 帧原文由脚本里的 WS 客户端落盘（`logs\ws-<名字>.jsonl`）；两个 GUI 之间的帧，给 GUI 加一个只在取证时打开的帧日志（审查 N9），不再只有摘要行。状态一律是「建议状态」。

### 16.1 十二条断言：各要什么，现在有什么，缺什么

| 断言 | 要点 | 现在有的 | 缺的 |
|---|---|---|---|
| `001` 从打开到保存的完整旅程 | `PIDINSERT VALVE 100,200` → 自动位号 → `PIDSETPROP 公称直径 DN50` → 保存 → 重开读回；`audit_log` 有 `insert_symbol` 与 `set_property`（u1）；会话全程活跃 | `PIDINSERT`、保存与重开、属性变更的广播、服务器认 `insert_symbol` | **`PIDSETPROP` 这个命令不存在**（属性现在走属性面板或 REST 的 xdata 写入）；客户端不广播新建 |
| `002` 占用排他贯穿 GUI 与服务器 | u1 占 P-1001 并改成功；u2 占用被拒，u2 在 GUI 里写这个位号被拒（`ok=false`，码表示占用冲突或只读），值不变；u1 释放后 u2 占用成功、写成功，u1 收到 `op_applied` 并读回 | 占用、拒绝、释放、着色；属性广播 | **占用不拦写**：客户端与服务器都不因为位号被别人占用而拒绝写操作 |
| `003` 管线插阀门后双方拓扑一致 | u1 画管线 L-1001（0,0 → 200,0），`PIDFITTING gate_valve 100,0`；服务器广播两条 `op_applied`；u2 本地拓扑：2 段、阀门两个连接点连到两段端点、无重叠无缺口；两侧拓扑读回的段数与端点坐标相等；`conflicts` 无 L-1001 的新行 | `PIDPIPE`、`PIDFITTING`、拓扑表（`GET /api/v1/records` 可读 `PID_AUTHOR_TOPOLOGY`）；服务器认拓扑类 op | 客户端不广播管线、管路、管件的新建，收到也不会在本图建实体 |
| `004` 无编辑权限角色被降级、写被拒且审计留痕 | u3（校对人员）连上后收到 `readonly_downgrade`，GUI 显示只读；写命令 `ok=false`、码为权限拒绝、`level` 为角色级；`audit_log` 有一行拒绝（u3、对象位号、被拒的操作）；读仍然 200 | 都有了（`97ff6d5de`、`e8b4034de`），单元测试守着 | 端到端取证 |
| `005` M3 持久化数据穿越协同会话后完整 | 离线读基线（带 `PID_AUTHOR` 的实体与 `props`、三个 XRecord 的长度）；u1、u2 做三次协同编辑（改属性、插阀门、改位号），都收到 `op_applied`；u1 保存；离线重开：每个基线实体的 `props` 键数不少于基线，三个 XRecord 存在且可解析，`PID_AUTHOR_TAG_RULES` 字节级相同 | 改属性、改位号的广播 | 插阀门的广播（同 001） |
| `006` 首次访问从空列表到两人 | 清空 `sessions`、`claims`；u1 在设置里配 `collab.server_url` 并保存（`settings.json` 的 `collab` 段出现）后连接；u1 面板 0 名其他用户（`welcome.peers` 为空）；u2 连入；u1 面板 1 名其他用户、共 2 人，`peer_joined` 的 `user_id='u2'`；u2 的 `welcome.peers` 含 u1 | `welcome.peers` 不含自己（`a0f33b354`）、面板人数、配置读写 | 要确认「在设置里配并保存」不动指针能走通（命令或 REST）；端到端取证 |
| `007` 冲突生成到裁决的闭环 | u1 经 GUI、u2 经 WS 在同一基线改 P-1001 的 `设计压力`（1.6MPa / 2.5MPa）→ 待裁决条目；u1 的 GUI 收到 `conflict` 并在面板显示条目；u4 裁决「采用乙方」；u1 收到推送后读回 2.5MPa，属性面板同值；条目已裁决；`audit_log` 有 u1、u2 的原始写入与 u4 的裁决，`object_ref` 都是 P-1001 | 服务器的并列、条目、裁决、裁决后的 `op_applied`（要带 `--tie-window-ms`） | **客户端收到 `conflict` 直接丢掉**（`Frame::Ignored`），面板没有冲突条目 |
| `008` 断线期间离线编辑，重连后对账 | 杀服务器；u1 离线改 P-1001 的 `公称直径` 为 DN80（本地成功）；重启服务器，u1 60 秒内重连；重连后这次改动被提交为 op（服务器收到，或服务器侧读回 DN80）；u2 重新连入后读回 DN80；若判为基线冲突则有待裁决条目与审计——两种结局必有其一，不许悄悄丢 | 重连阶梯；服务器的 `ops` 表与 `Store::load_ops` | **离线期间的编辑不排队**（没有会话时直接不发）；重连后也不补发；后连入的会话拿不到此前的 op |
| `009` 页间连接符跨图协同与属性传递 | u1 开 PID-1101、u2 开 PID-1102；u1 给 L-1001 建流出连接符（目标 PID-1102），u2 建对应的流入连接符，各收到 `op_applied`；u1 改 L-1001 的 `管道等级` 为 A1A，u2 那边连接符对端对象读回 A1A；u2 再建第二个流入连接符 → `ok=false`、`DUPLICATE_INFLOW_CONNECTOR` | `PIDCONNECTOR`、`PIDPAIR`、重复流入的拒绝（本地）；服务器认 `create_connector` | 连接符的新建不广播；**跨图的属性传递没有**（房间按图分，服务器不在两张图之间转发） |
| `010` 并发插入的位号唯一性由服务器保证 | 两个 GUI 间隔不到 1 秒各插一个 `VALVE`，本地都算出 V-1001；服务器只让一条以 V-1001 生效，另一条改派 V-1002（或收到冲突通知）；两边最终位号集合都是 {V-1001, V-1002}；`conflicts` 无重复位号条目 | 服务器认 `insert_symbol` | 服务器不查位号唯一、不改派；客户端不广播新建、不会按服务器改派的位号改自己的对象 |
| `011` 只读会话的截图与审计交叉验证 | u3 只读、u1 编辑；u1 改 P-1001 属性并广播；u3 收到 `op_applied` 并读回新值，属性面板显示新值但输入控件不可编辑；u3 的写入被拒并留痕；`audit_log` 有 u1 的成功行与 u3 的被拒行，`object_ref` 相同，u3 行的 `change_text` 含「只读」或原因 | 都有了，除了下一格 | **属性面板在只读会话里仍可输入**（提交之后被总闸退回）；端到端取证 |
| `012` 会话结束后本地文件可独立打开且无残留 | 各做一次编辑；u1 保存并关闭文档，两个实例退出；不带 `--collab` 重开保存的文件：`ready` 200、实体数大于 0、无协同相关错误、面板显示未配置、`PID_AUTHOR` 与 XRecord 完整；两个会话 `status='失效'` | 都有 | 端到端取证 |

### 16.2 做的次序

按「缺的东西」之间的依赖排，每一步各自带测试、各自提交，做完一步取那一步能取的证：

1. 只读与首次访问的端到端（`004`、`006`、`011`、`012`）：只差属性面板在只读会话里不可编辑这一处代码，其余是取证。
2. 客户端认 `conflict`（`007`）。
3. 占用拦写（`002`）：客户端总闸多一个理由，服务器的 `op` 多一道检查。
4. 新建与删除的广播、收到后在本图建与删（`001`、`005`、`010`、`003`；也是审查 S6 剩下的那一条）；`PIDSETPROP`；服务器保证位号唯一并改派。
5. 离线编辑排队、重连后补发，以及重连时由服务器补上错过的 op（`008`）。
6. 连接符跨图（`009`）。

## 17. 执行记录：2026-11-11 第五段（`cross-area-flows` 的前三步，N6、N9）

路径缩写同前：`B` = `D:\work\plant-code\cad\ocs-mission-backup-20261110\logs`，`E7` = `D:\scratch\m7-validation`。状态一律是「建议状态」。

### 17.1 提交（`653c934cd` 之后，都在 `main` 上，没有 push）

| 提交 | 内容 |
|---|---|
| `aa32dc8ca` | 状态文档：§16（十二条断言与取证办法），§15.5 补合并后尖端的数字 |
| `6ee15f294` | 只读会话的属性面板把对象的属性画成值（`VAL-CROSS-011`） |
| `31a2fb633` | `PIDCOLLAB SERVER / USER / DRAWING`：在程序里设置协同服务器并保存，当场连上（`VAL-CROSS-006`） |
| `c1db1a03d` | N6、N9：客户端日志分级，取证时存帧原文；地址显示时不带用户名口令 |
| `89f86a633` | 客户端认 `conflict` 帧：面板列出待裁决条目，裁决到达后清掉（`VAL-CROSS-007`） |
| `bc09302b5` | 占用拦写：服务器拒绝对别人占用位号的 op，客户端撤回对它的修改（`VAL-CROSS-002`） |
| `33e0a28e5` | M3 基线图的生成器（忽略的测试，带环境变量手工跑） |
| `a0ac5b61d` | `run QUIT / EXIT` 不需要打开着图纸（`VAL-CROSS-012` 取证时撞到的） |
| `163ae676a` | 协同面板在未配置时也上报位置与「未配置」提示（同上） |

### 17.2 上一批修正的补验（§15.5 欠的端到端）

在 `653c934cd` 构建出的可执行文件上：

- **M6 的八个场景**（`E7\m6-rerun-2\`，新建的目录；`E7\m6-rerun\` 是 `821e11395` 那次的脚本与证据，恢复原样、不再动）：`t20_session` 15、`t21_claims` 15、`t23_stale` 9、`t24_reconnect` 11、`t25_offline` 13、`t10_broadcast` 32、`t11_same_user` 7、`t22_ttl` 8，共 110 项全过，没有残留进程（`E7\m6-rerun-2\logs\run_all.summary.txt`）。汇总文件里另有两段不是通过的记录，原样留着：
  - 第一次起跑 8 个场景都在第一步退出（`0 PASS`）：新目录里还没有种子图。先跑 `t00_smoke`、`t01_seed` 之后重跑。不是产品失败。
  - `t10_broadcast` 第一遍 31 过 1 败：败的是「持有者没有被降级」那一项，脚本按旧行为查「持有者一条 `readonly_downgrade` 都收不到」，而 `97ff6d5de`（审查 S8）之后持有者也收到一条、点名自己。检查改成「`status.readonly` 为假，且收到的每一条都点名它自己」，脚本里留了带日期的注释；败的那次输出另存为 `logs\t10_broadcast.console.before-holder-check-update.txt`。单独重跑 32 项全过。
  - 指针与前台窗口：五个场景前后不变。`t23`、`t24`、`t25` 三个场景结束时指针或前台窗口与开始时不同（前台从浏览器换成 Cursor），脚手架用的是仓库里的 `hidden_desktop.py`，它同时看到这段时间里有真实的键鼠输入，判为「有人在用这台机器」。GUI 都起在隐藏桌面上，这三处我没有别的办法进一步证明不是脚手架造成的，照实记在这里。
- **M7 的三个冲突场景**：`c01_field_merge` 24、`c02_object_topology` 17、`c03_resolution` 20，共 61 项全过，与修正之前项数相同（之前那次的证据另存在 `E7\run-20261111-0307-before-review-fixes\`）。

### 17.3 `cross-area-flows`：做完的六条

每条的脚本在 `E7\` 下，证据在 `E7\logs\`（`x<编号>-*.json`、`gui-x<编号>-*.err.log` 含帧原文、`x<编号>-srv.out.log`）与 `E7\shots\`。GUI 都起在隐藏桌面上，只走 REST；每个场景前后指针与前台窗口不变（各场景输出的最后一行）。`x02`、`x04`、`x06`、`x07` 是在 `bc09302b5` 的构建上跑的，`x12` 在 `163ae676a` 的构建上；交回之前在最终尖端上全部重跑，数字写在 §18。

| 断言 | 建议状态 | 脚本与结果 | 字面上不满足或要说明的地方 |
|---|---|---|---|
| `VAL-CROSS-002` 占用排他贯穿 GUI 与服务器 | 成立 | `x02_claim_exclusive.py`，20 项全过 | 契约没有要求、多做的一项：另起一个 WS 会话直接向服务器提交对被占位号的 op，服务器回 `CLAIM_HELD`（证明不只是 GUI 自己不发）。u2 被拒的写操作回 HTTP 409、`code=CLAIM_HELD`，这是新增的错误码（§17.6 之 1） |
| `VAL-CROSS-004` 无编辑权限角色被降级、写被拒且留痕 | 成立 | `x04_readonly_role.py`，004 的 9 项全过 | 截图在 `E7\shots\cross-u3-readonly.png`，不在契约写的 `D:\scratch\` 根下（证据统一放 `E7`） |
| `VAL-CROSS-006` 首次访问从空列表到两人 | 成立，一处字面说明 | `x06_first_visit.py`，21 项全过 | 「在设置中把 `collab.server_url` 配为…并保存」是用命令做的：`PIDCOLLAB USER u1`、`PIDCOLLAB DRAWING PID-1101`、`PIDCOLLAB SERVER ws://127.0.0.1:3240/ws`，经 REST 的 `run` 发出。程序里没有一个带输入框的设置页；`settings.json` 的 `collab` 段确实当场写出（`x06-u1-settings-collab.json`）。要不要再做一个设置页，见 §17.6 之 3 |
| `VAL-CROSS-007` 冲突生成到裁决的闭环 | 成立 | `x07_conflict_closure.py`，19 项全过 | u2 用的是脚本里的 WS 客户端（契约写 Node WS）；服务器要带 `--tie-window-ms` 起（同 `VAL-CONF-004`） |
| `VAL-CROSS-011` 只读会话的截图与审计交叉验证 | 成立 | `x04_readonly_role.py`，011 的 11 项全过 | 属性面板「显示新值但输入控件不可编辑」的判据是控制通道读回的面板内容：7 组 26 行全是只读行，`设计压力` 一行的值是新值（`x04-011-u3-properties.json`）；对照里 u1 的同一行是输入控件。截图 `cross-u3-panel.png` 里能看到面板各行是只读样式、协同面板有只读标识，但 `P&ID 属性` 一组在面板下方、没有滚进截图 |
| `VAL-CROSS-012` 会话结束后本地文件可独立打开且无残留 | 成立 | `x12_after_session.py`，19 项全过 | 用的图是 M3 基线图（`E7\work\PID-1101-m3.dwg`，`33e0a28e5` 的生成器写出），因为种子图里没有图级 XRecord。前两次没有全过，原因见下 |

`x12` 的三次运行（都留着：`logs\x12_after_session.console.first-run-5-fail.txt`、`…second-run-stale-descriptor.txt`、最后一次是 `…console.txt`）：

1. 第一次 14 过 5 败。两处是产品的缺口，修了：关掉图纸之后 `run QUIT` 回 `no_document`，实例退不掉（`a0ac5b61d`）——由此连带「会话没有失效」「描述文件留下」两项；未配置的协同面板不上报任何位置（`163ae676a`）。一处是脚本读错：图级 XRecord 的条目前两条是 `kind=`、`key=` 前缀，不是 JSON。
2. 第二次 18 过 1 败：「退出的实例没有留下描述文件」数到的那 1 个是第一次运行里被杀掉的 u1 留下的（文件时间 13:41，这次运行在 13:54）。脚手架改成起实例前清掉同名实例上一次留下的描述文件。
3. 第三次 19 项全过。

### 17.4 还没做的六条

`VAL-CROSS-001`、`003`、`005`、`010`（新建与删除的广播，§16.2 第 4 步）、`008`（离线排队与重连对账，第 5 步）、`009`（连接符跨图，第 6 步）。做到哪里以 §18 为准。

### 17.5 审查的 N6、N9

| 项 | 现在 | 提交 | 守着的测试或脚本 | 还差什么 |
|---|---|---|---|---|
| **N6** 日志不分级；地址原样进日志 | 已修 | `c1db1a03d` | `a_frame_is_written_by_the_level_the_log_is_at`、`an_address_is_shown_without_its_credentials`、`an_address_carrying_credentials_is_refused_and_not_repeated` | 缺省级别仍是每帧一行摘要（M6 的契约证据读的就是这些行）；要安静用 `OCS_COLLAB_LOG=quiet` |
| **N9** 证据的卫生 | 三条里修了两条半 | `c1db1a03d`（产品）；脚本在 `E7\m6-rerun-2\` | 帧原文：`OCS_COLLAB_LOG=frames`，两个脚手架库起 GUI 时都带上，脚本用 `frames()` 读。间接的检查改读帧原文：001 的 `welcome`（`peers`、`claims` 是数组）、004 的 `claim_granted`（`session_id` 等于本会话）、013 的 `peer_joined`（`display_name` 非空）。`t21_claims` 已在 `run_all` 里 | 「各处截图只查文件存在」没有改。改过检查的三个 M6 脚本还没有在带帧日志的构建上重跑，交回之前重跑 |
| **N4** `__overrides` 不随变更走；远端改位号一律 `manual` | 没修 | — | — | |

脚手架起的实例是被杀掉的（`TerminateProcess`），会留下自动化通道的描述文件。两个脚手架库现在在起实例前和清理时删掉它们；此前留下的 27 个（`E7\m6-rerun-2\work` 16 个、`E7\work` 11 个）已删，路径与字节数记在 `D:\work\plant-code\cad\ocs-mission-backup-20261110\n8-deleted-descriptors-2.txt`，不含内容。删之前确认过没有 `OpenCADStudio` 或 `ocs_collab_server` 进程在跑。

### 17.6 这一段里要用户过目的

1. **新增错误码 `CLAIM_HELD`**，REST 回 409。契约只说「`code` 表示占用冲突或只读」。选项：沿用；或者并进现有的某个码。倾向沿用——它和只读不是一回事（释放之后同一请求就能成功），`holder` 字段也只有它带。
2. **客户端对占用的了解是不全的。** `claim_granted` 是定向消息（契约第 1866 行），别的会话只有在加入时（`welcome.claims`）或自己申请被拒时才知道某个位号被谁占着。所以客户端这一道拦不住「不知道已被占用」的修改，那种情况由服务器拒绝——而被服务器拒绝的 op，本地的修改现在还留在图里（§15.2 的 S3 一行说过，放在重连对账那一步做）。选项：把占用的获得也向房间广播（改协议，与那一行契约不一致）；或者保持定向，靠服务器拒绝加本地撤回。倾向后者，不动协议。
3. **设置页。** 协同的三项设置现在只能用命令配（`PIDCOLLAB SERVER / USER / DRAWING`），未配置的面板上有一行提示。选项：再在「选项」对话框里加一页，或者在协同面板里放输入框。没有做：那两处的输入框都要点击才能取证，而且「选项」对话框是另一条线也在改的文件。
4. **并列写入的一方，本地的值保留到裁决。** `conflict` 到达时两边的写入都没有生效，但写入方自己的图里仍是它写的值，面板上有条目、命令行有一句警告。选项：收到 `conflict` 时把本地的值退回到写之前。没有做：退回要动撤销栈，而裁决一到值就定了。
5. **`OCS_COLLAB_LOG`** 这个环境变量的名字与三级（`quiet`、缺省、`frames`）。
6. **只读会话的属性面板：没有选中对象时的图纸级设置页没有锁。** 选中对象时全部是只读行；什么都不选时显示的当前图层、线型比例这些仍可点。改它们若形成一步编辑，会被总闸退回。
7. **`run QUIT / EXIT` 在起点页放行**，改的是上游的控制通道闸口（一处条件）。

### 17.7 测试

| 提交 | 跑了什么 | 结果 |
|---|---|---|
| `6ee15f294`、`31a2fb633` | lib 全量 3 遍；协同分组 | 每遍 6140 通过 / 0 失败 / 42 忽略；139 通过。红灯 `B\r5b-step1-red.log`（4 条失败） |
| `c1db1a03d` | lib 全量 1 遍；协同分组 | 6143 / 0 / 42；142 通过 |
| `89f86a633` | lib 全量 1 遍；协同分组 | 6147 / 0 / 42；146 通过。红灯 `B\r5e-conflict-red.log`（3 条失败） |
| `bc09302b5` | 服务器三个测试程序与 clippy；lib 全量 1 遍；协同分组 | `ocs_collab` 79、`ocs_collab_server` lib 141、main 12，0 警告；6150 / 0 / 42；149 通过。红灯 `B\r5f-srv-red.log`、`B\r5g-claims-red.log` |
| `33e0a28e5` | `dwg_roundtrip` 分组；生成器带 `--ignored` 跑一次 | 2 通过 / 1 忽略；生成器通过，写出 23379 字节 |
| `a0ac5b61d`、`163ae676a` | lib 全量 1 遍 | 6153 / 0 / 43。这两处没有单独取红灯：红灯就是 `x12` 第一次运行的那两项失败。`quit_is_run_with_no_drawing_open` 第一版多查了小写的 `exit`，失败了一次（`B\r5j-lib-1-quit-lowercase-failed.log`）：命令分发本来就只认大写的命令名，测试改成只查 `QUIT`、`EXIT` |

## 18. 执行记录：2026-11-11 第六段（符号的新建进入协同，最终门禁，交回）

### 18.1 提交（`ccb41f0cf` 之后）

| 提交 | 内容 |
|---|---|
| `e403d035f` | 符号的新建与删除进入协同；服务器保证新符号的位号唯一并改派；`PIDSETPROP`（`VAL-CROSS-001`、`005`、`010`，审查 S6 剩下的那一条） |
| `8ad2e9650` | 状态文档：§15.2 的 N6、N9 两行（上一条文档提交漏进暂存区的） |
| `d8dec046d` | 合并 `upstream-merge-20261110` 到 `bcc86e1e6`（备份面板那条线的三个提交，无冲突） |
| `b4b302f4a` | 别人放的符号一律在本图新放一个（`x10` 第一次运行查出的缺陷，见 §18.3） |
| `ea7f4357b` | 去掉两处不再用到的代码（主程序 clippy 新增的两条告警） |

### 18.2 `cross-area-flows` 十二条：交回时的建议状态

| 断言 | 建议状态 | 脚本（`E7\`） | 最终尖端上的结果 | 字面上不满足或要说明的地方 |
|---|---|---|---|---|
| `001` 从打开到保存的完整旅程 | 成立 | `x01_journey.py` | 见 §18.5 | 图是 `E7\work\` 下种子图的副本，不是 `D:\scratch\PID-1101.dwg`。「重新打开该文件（新文档 id）」在同一个实例里做：关掉文档再打开保存的文件 |
| `002` 占用排他 | 成立 | `x02_claim_exclusive.py` | 同上 | §17.3 |
| `003` 管线插阀门后双方拓扑一致 | **没做** | — | — | 管线、管路、管件的新建不广播（§18.4） |
| `004` 无编辑权限角色 | 成立 | `x04_readonly_role.py` | 同上 | §17.3 |
| `005` M3 持久化数据穿越协同会话 | 成立 | `x05_m3_persistence.py` | 同上 | 基线图是 `E7\work\PID-1101-m3.dwg`（契约写的 `D:\scratch\PID-1101-m3.dwg` 不存在，用 `33e0a28e5` 的生成器写出，内容是 M3 往返测试的那份）。对比结果在 `E7\logs\cross-m3-m6.json`，不在 `D:\scratch\` 根下。「插入阀门」是独立放置的阀门（`PIDINSERT VALVE`），不是插在管路上的管件 |
| `006` 首次访问 | 成立，一处字面说明 | `x06_first_visit.py` | 同上 | 「在设置中」是用命令配的（§17.3、§17.6 之 3） |
| `007` 冲突到裁决的闭环 | 成立 | `x07_conflict_closure.py` | 同上 | §17.3 |
| `008` 离线编辑，重连后对账 | **没做** | — | — | 离线期间的编辑不排队，重连后不补发（§18.4） |
| `009` 连接符跨图 | **没做** | — | — | 连接符的新建不广播；服务器不在两张图之间传属性（§18.4） |
| `010` 并发插入的位号唯一 | 成立 | `x10_concurrent_insert.py` | 同上 | 服务器的唯一性只管它见过的对象（§18.6 之 1） |
| `011` 只读会话的截图与审计 | 成立 | `x04_readonly_role.py` | 同上 | §17.3 |
| `012` 会话结束后独立打开 | 成立 | `x12_after_session.py` | 同上 | §17.3 |

九条成立，三条没做。`cross-area-flows` 这个 feature **不能记为完成**。

### 18.3 这一步里端到端查出来的缺陷

**别人放的符号被当成自己的，没有放（`b4b302f4a`）。** `e403d035f` 里收到 `insert_symbol` 时有一条捷径：发送方的句柄在本图恰好带着同一位号，就当它是本图自己的符号被说回来。两张图从同一份文件打开时，各自新建的下一个实体拿到同一个句柄，所以两个人同时各插一个阀门，位号相同、句柄也相同，这条捷径就把对方的阀门认成了自己的。`x10` 第一次运行：u1 `['V-1002']`，u2 `['V-1001', 'V-1002']`（`E7\logs\x10_concurrent_insert.console.first-run-handle-collision.txt`）。单元测试当时没拦住，因为测试里对方的句柄是一个本图没有的值；改成相同的句柄之后先红后绿。

**`x01` 第一次运行败了一项，是脚本的问题**：我在另一个进程里重开保存的文件，然后比两个进程的文档 id——新进程的文档 id 从头编号，两边都是 2。改成在同一个实例里关掉再打开（`E7\logs\x01_journey.console.first-run-docid-check.txt` 是那一次的输出）。

### 18.4 没做的，没核实的

- **管线、管路、管件的新建与删除不广播**（`VAL-CROSS-003`）。它们是几个实体加一张拓扑表，对端要重放画法（`PIDPIPE`、`PIDFITTING` 的落点与拆段）才能得到一致的拓扑。服务器认 `create_pipe_line`、`add_pipe_run`、`insert_fitting`，客户端不发也不收。画了管线的会话，对端的图里没有那条管线；此后对它的属性修改到了对端会报「对不上对象」并把对端标成不一致。
- **离线编辑不排队，重连后不补发；重连时服务器也不补上错过的 op**（`VAL-CROSS-008`）。断线期间本地的修改只在本地；重连后若房间的修订动过，会话被标成不一致（§15.2 的 S3）。
- **自己的 op 被服务器拒绝之后，本地的修改还留着**。拒绝的原因有过期基线、被取代、对象已删、位号被占（客户端不知道的占用，§17.6 之 2）。要让 `error` 帧带上结构化的 `op_id`，客户端据此把那一步撤回；与上一条是同一块工作，没有做。
- **连接符跨图**（`VAL-CROSS-009`）。连接符的新建不广播；房间按图分，服务器不在两张图之间转发属性。
- **审查 N4**（`__overrides` 不随变更走；远端改位号一律 `manual`）没有修。服务器改派的位号例外：`tag_source` 保持原样。
- **属性溢出到 XRecord 的符号**，新建时说成「没有初始属性」，属性不随 `insert_symbol` 走。
- **一个 op 里有两个同位号的新符号**时服务器不会给第二个改派（它逐项对照的是已提交的对象）。客户端本地的位号引擎不会在一步里产生这种 op。
- 截图只查了文件存在（N9 的那一条），这一段里我看过其中两张（`cross-u3-panel.png`、`x07-u1-conflict.png`），其余没有逐张看。
- M6 的 20 条断言没有重新逐条定状态：八个场景在最终尖端上全部通过，§13.3 的「13 条成立、7 条部分成立」没有改。其中 `011`（只读的门的范围）与 `013`（`welcome.peers`）所依据的缺口已经修掉并有端到端证据，是否改回「成立」留给验证方。

### 18.5 最终尖端 `ea7f4357b` 上的结果

代码的最终尖端是 `ea7f4357b`；它之后只有这份文档的提交。下面的结果都是在 `ea7f4357b` 的构建上跑的（`B\r6e-*`）。

| 项 | 结果 |
|---|---|
| `build --workspace`、`test --workspace --no-run -j 4` | 退出码 0（63 秒、114 秒） |
| 全量 20 个测试程序 | 6789 通过 / 0 失败 / 45 忽略（`B\r6e-test.log`） |
| lib 全量 8 遍 | 每遍 6162 通过 / 0 失败 / 43 忽略（`B\r6e-lib-full\summary.txt`） |
| 分组 | `pid_backup` 226 通过 / 3 忽略；`pidbackup` 29 通过 / 1 忽略；协同 155 通过 |
| 服务器 | `ocs_collab` 80、`ocs_collab_server` lib 142、main 12（在 20 个程序之内）；`clippy -p ocs_collab -p ocs_collab_server --all-targets` 0 条 |
| `check --workspace` | 退出码 0 |
| `clippy -p OpenCADStudio` | 572 条（改前 571；多的 1 条在另一条线的文件里，§18.6 之 5）；协同的文件里 0 条。这一项是在 `ea7f4357b` 的工作区上、提交之前跑的（`B\r6d-app-clippy.log`） |
| `pytest scripts/pid-platform` | 558 通过（在 `b4b302f4a` 上跑的；此后没有动 Python） |
| M7 的十一个端到端场景（`E7\run_cross.ps1`） | `c01` 24、`c02` 17、`c03` 20、`x01` 13、`x02` 20、`x04` 26、`x05` 14、`x06` 21、`x07` 19、`x10` 15、`x12` 19，共 208 项全过；没有残留进程，没有留下描述文件；十一个场景前后指针位置与前台窗口都不变（`E7\logs\run_cross.summary.txt` 最后十一行） |
| M6 的八个端到端场景（`E7\m6-rerun-2\run_all.ps1`） | `t20` 15、`t21` 15、`t23` 9、`t24` 11、`t25` 13、`t10` 32、`t11` 7、`t22` 8，共 110 项全过；没有残留进程；八个场景前后指针位置与前台窗口都不变（`E7\m6-rerun-2\logs\run_all.summary.txt` 最后八行） |

同一批门禁在 `d8dec046d`（`B\r6a-*`）与 `b4b302f4a`（`B\r6c-*`）上也各跑过一遍，数字相同（20 个程序 6789 / 0 / 45，lib 8 遍每遍 6162 / 0 / 43）。这一段里没有出现零星失败。

各步的红灯输出：`B\r5n-create-red.log`（符号新建，4 条失败）、`B\r6b-handle-collision-red.log`（句柄相同，1 条失败）。

### 18.6 这一段里要用户过目的

1. **服务器的位号唯一性只管它见过的对象。** 它的对象表是从 op 里长出来的：图里原有、从没被任何 op 提到过的对象它不知道。两个客户端打开同一份文件各插一个符号时，本地的位号引擎都会避开图里已有的位号，然后两边算出同一个新号，服务器把后到的改派——这是 `VAL-CROSS-010` 的情形，成立。但若两份文件本来就不一样，服务器可能把新符号改派到一个只在某一方图里已经有人用的位号上。选项：会话建立时客户端把图里已有的位号报给服务器；或者保持现状。倾向前者，没有做——它要一条新消息。
2. **`PIDSETPROP <属性> <值>` 不写位号时作用于谁。** 现在：选中的带 P&ID 属性的对象；什么都没选时，本图上刚放置的那个符号（命令行或调色板放的，不含别人放的）。契约只给了 `PIDINSERT` 之后紧跟 `PIDSETPROP 公称直径 DN50` 这一种用法。
3. **`PIDINSERT` 接受类型或类别名**（`VALVE`、`EQUIPMENT`），取该类别的第一个符号。契约写的是 `PIDINSERT VALVE 100,200`，此前只认符号 id（`gate_valve`）与块名。
4. **证据日志里有前台窗口的标题。** 脚手架用仓库里的 `hidden_desktop.py` 在每个场景前后记指针位置与前台窗口，前台窗口记的是标题的前几十个字符——也就是你当时在用的窗口。其中有一行带一个服务器的 IP 地址，没有口令。这些日志只在 `E7` 下。要不要把探针改成只记窗口句柄、并把已有日志里的标题抹掉，请定；我没有动。
5. **另一条线带进来一条 clippy 告警**（`src\io\pid_backup_browse\exp_views.rs`，「large size difference between variants」），主程序的告警数因此是 572 而不是 571。那是那条线的文件，我没有动。

### 18.7 磁盘、环境、删除

- 删除：只删了脚手架实例留下的自动化通道描述文件（§17.5，27 个，路径记在 `n8-deleted-descriptors-2.txt`），以及我自己的两个临时探查脚本（`E7\probe_records.py` 与它的图）。`D:\Rust\target-ocs-mission` 里什么都没删。
- C 盘：我的构建目录、临时目录（`TEMP` 指到 `D:\work\plant-code\cad\ocs-mission-backup-20261110\tmp`）、证据都在 D 盘。C 盘上我写的只有 Cursor 自己的会话与终端记录。
- 没有 push；没有碰 `C:\Users\dpc\.factory\`、备份面板那条线的 worktree、stash、`D:\work\plant-code\cad\OpenCADStudio\target`、mission 留下的进程与 `.ocs.lock`、`.tgrep/`。
- 指针与焦点：GUI 都起在隐藏桌面上，只走 REST；没有任何一步需要点击。场景前后的探针多次记到指针或前台窗口变了，同时记到有真实的键鼠输入；不起 GUI 的 `c01`、`c02` 也记到了，所以那是机器上有人在用。

## 19. 阶段 A 结项（2026-11-20）

分支 `collab-off2`，没有推送。代码的最后一个提交是 `2710623e3`；全量复验在 `3450558e6` 上做，它与 `2710623e3` 之间、以及它之后的提交都只改文档。复验的证据在仓库外：`D:\scratch\ocs-sppid\2026-11-20-cc7`。计划是 `docs/plans/2026-11-16-pid-platform-next-steps-plan.md` 的 T2.4，任务是 `docs/plans/2026-11-16-pid-dev-spec/todo.md` 阶段 A 的 `CC-1` – `CC-8` 与 `OFF-1` – `OFF-13`。

### 19.1 `ea7f4357b` 之后做了什么

只列协同的主要提交。同一段时间里分支上还并进了别的线（`ea7f4357b..HEAD` 共 971 个提交，781 个不是合并）。

| 内容 | 任务 | 提交 | 要点 |
|---|---|---|---|
| 管线、管路、管件的新建与删除进入协同（`VAL-CROSS-003`，§18.2 缺的「管线新建的广播」） | `CT-13` | `43c2ea556`、`576ddded4`（2026-11-12） | 服务器登记管路的零件清单和阀门，接 `update_pipe_run`，认领着管线时拦它的拓扑变更；撤销带结构变化的一步，句柄起点不再退到别人的实体后面 |
| 离线的修改排队，重连后补发、取回错过的操作（`VAL-CROSS-008`） | `CT-14` | `9c627470c`、`d919afdfa`（2026-11-12） | 答 `op` 的错误点名是哪个 `op`；带结构的 `op` 全有或全无；服务器拒绝的一步在本地撤回并说明 |
| 跨图连接符（`VAL-CROSS-009`） | `CT-15` | `993e2f3e4`、`09320ca7b`（2026-11-12） | 连接符的新建与删除进入协同；对端在另一张图时由服务器配对，把流出侧线体的字段提交到流入侧那张图 |
| 建连时上报本图已有的位号（`existing_tags`，§18.6 之 1） | T2.4 ① | `1682b84c3`、`93433f7d2`、`b34bccf7e`、`ecdd8dd91`、`adb4aed54` | 服务器在握手时登记、入库，重启后仍在；为上报位号离开再加入之后，原先的占用要得回来（离开时等服务器收下关闭帧） |
| 属性变更带覆盖标记（`overrides`），改位号带真实来源（`tag_source`）（审查 N4） | T2.4 ② | `9b5327f96` | 远端改位号保留 `rule` / `manual` |
| 属性超过 2 KB 的对象 | T2.4 ③（`CC-1`、`CC-2`） | `0744f67b2`、`e24b3f947`、`68f08721e` | 符号、管线、管路、管件新建和放回时带上存在扩展字典里的全部属性；之后改属性也广播；撤销 / 重做不带走别的对象的属性 |
| 服务器 `--bind`（服务器规格 S1） | `CC-3` | `278439b5d` | 默认仍只监听本机；对外监听时用法和启动输出都提醒没有登录验证、不加密。两台机器上的协同流程还没有实际跑过 |
| 228 条契约的对照表；端到端脚本收进仓库；补齐对照表的缺口 | `CC-4` – `CC-6` | `6b1be96ac`、`84d1e0ead`、`412f10a2c`、`b3b366072`、`e6a657229`、`fa72fd400`、`a6689b486`、`130dda683` | 脚本在 `scripts/collab-e2e/`；对照表在它的 `README.md` 第 4 节，228 条的「覆盖」都是「有」 |
| 离线使用与上线后同步（`SPEC-collab-offline-sync.md`） | `OFF-1` – `OFF-13` | `bfd2bda12`（`OFF-1`）；`4bcb9020f` – `079b19a24`（`OFF-2` – `OFF-13`，其中 `OFF-12` 有两个提交）；`c399311e7`（基线图的生成器去掉同步记录） | 加入时对账；离线的修改随图保存，跨重启提交；修改编号全局唯一，服务器对收下过的编号不重复生效；不一致标记随图保存；重开后被拒的修改按反向修改撤回；未同步清单与导出；面板显示待同步的步数，退出和关图时提醒；库打不开时不接受会话（`--new-repo-id`）；端到端场景 `o01` – `o08` |
| CP-A4 时的两处修复 | — | `98bf1a894`、`2710623e3` | 切换图纸状态只过角色和 PBS 两级（`REQ-COLL-004`）；`x12_after_session` 的「没有弹出对话框」也看工具窗（`VAL-CROSS-012`） |

### 19.2 复验（`CC-7`，提交 `3450558e6`）

| 项 | 命令 | 结果 | 输出（证据目录里） |
|---|---|---|---|
| 主程序库 | `cargo test --offline --locked --lib` | 7239 条：7182 通过、55 默认不跑、2 失败（设计稿比对，§19.5 已知问题 1）；连重编测试程序 188 秒 | `unit-lib.txt` |
| 协同两个 crate | `cargo test --offline --locked -p ocs_collab -p ocs_collab_server` | `ocs_collab` 98 条，`ocs_collab_server` 的库 187 条、`main.rs` 21 条，全过 | `unit-collab.txt` |
| 端到端 | `scripts\collab-e2e\run19.ps1 -EvidenceDir <证据目录>\e2e -PortShift 300 -Label cc7`；两个可执行文件开跑前在这份检出里编（服务器 cargo 报不用重编） | 46 个场景加收尾的整轮检查，830 项全过。各组：脚手架与入口规程 4 个场景 33 项；冲突与跨区流程、桌面端客户端 19 个 357 项；后补的跨区流程 3 个 81 项；服务器进程 4 个 56 项；看图 5 个 45 项；计时 3 个 14 项；离线 8 个 239 项；收尾 5 项。残留进程 0，桌面记录 50 次都是「没变」，跑完工作区仍干净，用时 31 分钟。`t03_vanish_retry` 第一次按设计以 75 收场、重跑一次，第一次不计入 | `build-gui.txt`、`build-server.txt`；`e2e\summary-cc7-20261120-210132.txt` |
| 逐条核对 | 证据目录里的 `evidence.py`，读 `scripts/collab-e2e/README.md` 第 4 节 | 249 行（228 条契约、21 条 OS）：列的 369 个单元测试都是 `ok`；113 处场景引用，项数都与表里写的相同、都是 `PASS`；0 处对不上 | `evidence-check.txt`、`evidence.tsv` |
| 计时 | 端到端的计时一组 | `VAL-DRAW-030` 20 次放置的中位数 76.0 毫秒（上限 100）；`VAL-DRAW-032` 图里有了 200 个符号之后 61.0 毫秒，比前一组少 19.7%；`VAL-DRAW-031` 中位数 664.2 毫秒（上限 3000）；`VAL-SEM-014` 中位数 18 毫秒（上限 3000）。计时时机器上另有别的实例和编译器进程，不是契约要的干净环境；读数都在限内 | 同上 |

离线规格 §7 的成功标准 OS-01 – OS-21：README 第 4.10 节每条有证据位置。OS-20 在 `PUB-4`（阶段 D）验，不在这一遍；OS-21（原有的 38 个场景连同离线的八个一起跑、全部通过）由这一遍取证。

固定门禁（`todo.md` §1）在结项时的读数：主程序库失败的只有比对设计文件的测试（2 条），通过数 7182，比基线 6865 多；`cargo clippy --offline --locked --lib` 656 条（上限 656），加 `--profile test` 1051 条（上限 1051），是 2026-11-20 在这份检出的当前提交上跑的（代码与 `2710623e3` 相同，输出在 `D:\scratch\ocs-sppid\2026-11-20-cpa`）；协同两个 crate 全过；文档检查 `python -m pytest scripts/pid-platform -q -p no:cacheprovider` 590 项 589 过，没过的 1 项与设计稿比对同源（§19.5 已知问题 1）。

### 19.3 断言的状态：有证据的建议状态

mission 目录只读，`validation-state.json` 没有改，里面仍是 179 passed、49 pending。按证据给的建议状态是 228/228 passed：每条在 `scripts/collab-e2e/README.md` 第 4 节对到单元测试或场景（「覆盖」一栏都是「有」），`CC-7` 复验时都过了，证据位置写在那张表最后一列「复验证据」里。原来 pending 的 49 条：

| 前缀 | 条数 | 对照表 | 说明 |
|---|---:|---|---|
| `VAL-CLI` | 20 | README 第 4.7 节 | 第二轮逐条取过证（§13.3），mission 的里程碑验证没跑 |
| `VAL-CONF`（`018` 以外） | 17 | 第 4.8 节 | 功能在 §14.5 |
| `VAL-CROSS` | 12 | 第 4.9 节 | 九条在 §18.2；缺的 `003`、`008`、`009` 见 §19.1 |

§13.3 记的「7 条部分成立」现在是这样：`011`、`013` 后来修了，按字面通过；`003`、`005`、`009`、`018` 是契约写法与实现的出入，脚本按实现断言；`019` 的依赖检查有了，「冷构建不超过 20 秒」一句 CP-A 定为删掉、不验收。

契约字面与实现有出入的，记在 README 第 5.3 节，共 28 行、34 条：脚本过了，但契约原文有一句按字面不成立，或者脚本的做法与原文不完全一样。建议状态是按那里记的口径算的。其中写明要裁决的 6 行（7 条）CP-A 定了，都维持现在的实现（§19.6），建议状态不变。

### 19.4 已确认的实现选择（服务器规格待决 S2）

2026-11-16 定为四处都保持现状，在这里写成已确认：

1. 图切成只读时，编辑权归「当时在图上最久的活跃编辑会话」（§13.4 之 2）。2026-11-20 CP-A4 又定：只读时这个持有者照旧能写（`VAL-CLI-011`）；这与 `REQ-COLL-004` 迁移规则第 3 条、验收 5 不合，出入记在离线规格 §11.18。
2. 并列窗口默认 0，默认不产生冲突条目（§14.6 之 1）。要默认就出现冲突裁决，得给一个几百毫秒的默认值，代价是每一步写字段的广播都晚这么久。
3. 让路与过期分用两个错误码：`OP_SUPERSEDED`、`STALE_BASE_REVISION`（§14.6 之 3）。
4. 「被引用」只算管路端点（§14.6 之 4）。

### 19.5 已知问题与已知限制

**已知问题**

1. 设计稿比对不过：主程序库的 2 条（`app::pid_shell::pid_editor_screen_canvas::the_grid_is_drawn_the_way_the_design_draws_it`、`ui::style::pid_tokens::tests::the_code_states_nothing_the_design_does_not`），文档检查的 1 项（`test_gen_pid_tokens.py::test_the_committed_table_is_what_the_design_generates`）。原因在 `7d545f845`（2026-11-19，默认主题改成亮色）：它给代码里的主题表加了设计稿没有的 11 个变量（`canvas-grid-major` 等），网格改用变量，仓库里的设计稿 `docs/pid-platform/design/pid-platform.pen` 没有跟着改。离线同步的起点 `51b654a87` 就有，与协同无关，228 条契约没有引用它们。2026-11-20 用户定：记为阶段 A 的已知问题，归界面主题那条线修（给设计稿补上这 11 个变量并提交），不在这条分支上修。

**已知限制：客户端规格待决的 C1、C2、C4**（2026-11-16 定为本轮不做，列进阶段 4 提纲「会话管理与重新同步」）

- **C1 本图与房间对不上之后**（按离线规格 §5.5 与 §9 的 L6 改写）。离线同步做完以后，原来说的三种来源是这样：运行中标的「不一致」随图保存，`PIDCOLLAB DETACH FORCE` 能去掉；第一次加入时，图里有同步记录的核对库标识和修订号（不核对内容），没有记录的仍不检查（下面的 L1）；断线排队的修改随图保存，下次打开并连上后接着提交。不一致之后能列出、导出没同步上的修改：面板和 `PIDCOLLAB UNSYNCED` 列出还在队列里的、被拒的、没排上队的，`PIDCOLLAB EXPORT <路径>` 导出成文件；再向图纸一致的同事要他刚保存的文件，打开加入后自动补齐，对照清单在线重做。仍是限制的：没有按清单自动重放，也不能从服务器重建整张图；没有人手里有一致的图纸时，只能以其中一份为准、所有人从它重新开始（选定的那一份先导出清单，再 `PIDCOLLAB DETACH FORCE`）。
- **C2 一次运行里会话不能改、不能退**：换服务器、换图、退出协同都要重启程序。重启能解决，不挡试用。
- **C4 桌面端没有冲突裁决的入口**：面板只列出冲突。服务器默认不产生冲突条目（并列窗口为 0，§19.4 之 2），只有打开并列窗口才用得上。

**已知限制：离线规格 §9 的 L1 – L12**（照录，表里的节号是离线规格的）

| # | 限制 |
|---|---|
| L1 | **从没同步过的图。** 图里没有同步记录时，加入仍被当成与房间一致，不检查（客户端规格 §7「没有初始同步」）。改它会动到现有契约里「第一次加入」的约定，这次不动 |
| L2 | **一次运行只管一张图。** 离线时要改几张图，得分几次启动。别的图上的语义修改会被发现并打标（OS-11），但不会同步 |
| L3 | **离线后占用会释放。** 正常退出立即释放；断线到心跳超时（默认 90 秒）释放（协议规格 §5.1）。离线期间别人可以改同一个对象，回来时被拒的会比在线时多 |
| L4 | **做了又撤销也算改过。** 没排队的语义修改一旦发生就记下，撤销不清。不保存就不会写进图 |
| L5 | **补齐时别人的修改写不进本图，当场标不一致**（现状，读代码）。最常见的是我离线删了一个对象，而别人这期间改过它：不等我的删除被裁决就标了。这次不改，场景 `o05` 把它记下来。2026-11-20 起标了之后我的删除留在队列里，不再抢先发出（§11.16）。CP-A4 定为维持现状（§11.18） |
| L6 | **不一致之后只能换图重做。** 没有自动重放，也不能从服务器重建整张图 |
| L7 | **两个人离线各自发布。** 默认不许；明确放行时可能得到同一个修订号的两个包，清单里的 `reconciled: false` 可以认出来 |
| L8 | **队列跟着图纸文件走。** 文件丢了，队列也丢了 |
| L9 | **重开后的撤回只保证语义内容回去。** 放回来的对象是新建的，句柄与原来不同 |
| L10 | **用户号是自报的**（`REQ-COLL-016` 没做）。「不替别人提交」只防误操作，不防冒名 |
| L11 | **恢复备份后没换库标识。** 旧副本可能被当成一致（§6 第 4 条）。靠运行规定防，程序不拦 |
| L12 | **另存为的副本仍然记着原图号和库标识。** 没明确给图号时它会加入原图的房间。要当成新图用，先 `PIDCOLLAB DETACH` |

### 19.6 CP-A 的裁决（2026-11-20）

README 第 5.3 节写明要裁决的 6 行（7 条），用户按建议定了：都维持现在的实现，定下的口径写在 README 那一节各行末尾的「CP-A 定」之后。

| 契约 | 出入 | 定的 |
|---|---|---|
| `VAL-CLI-019` | 「冷构建不超过 20 秒」不成立，这一句没有检查；依赖树的检查已经有了 | 删掉这一句，构建用时不验收；这条要管的「不引入异步运行时」由依赖树的检查盯着 |
| `VAL-PERM-032` | `workspace` 是建会话时的正式字段，写在那里的值会被采纳；帧里多出来的 `ip`、`company` 不读的部分成立 | 口径是「`workspace` 由会话声明，`ip`、`company` 不可自报」，服务器不按名单校验；用户号本身也是自报的（L10），名单挡不住冒名 |
| `VAL-SRV-030` | 按下限（8 个会话、各 30 条）成立。规模再大时，每张图的广播队列只留最近 256 条，积压超过就丢通知：10 个会话各连发 30 条时 9 次里有 4 次丢了，其中一次有 19 条已经生效的 `op` 发送方没收到自己的 `op_applied`（库里是全的） | 维持现状。验收的规模以「每张图 8 个会话、各 30 条」为上限，规模更大时丢通知记为已知限制；「发送方的回执单独发、不走广播队列」是后续任务 |
| `VAL-INFRA-009` | 截图请求不带 `path` 时程序回 400 | `path` 写进口径，截图请求必须带 `path`；程序不在没写明的地方存文件 |
| `VAL-PIPE-017` | 应答的实体表最多 64 条，契约的那个请求不一定列得全三样；管线自己的线不会被阀门断开 | 维持现在的判法；64 条的截断记为已知限制；管线自己的线要不要随阀门断开，交给绘图那条线定（后续任务） |
| `VAL-DRAW-030`、`032` | 场景在每个计时的请求之前停 0.2 秒；一个接一个发时中位数约 113 毫秒 | 100 毫秒管「隔开发的一次放置」（现在的做法，也是契约基线的量法）；「答完之后在忙什么」是后续任务 |

其余 22 行是做法上的出入，按记下的做法认可，契约字面不改（其中 `VAL-INFRA-001`、`014` 2026-11-17 已定，`VAL-CLI-011`、`013` 后来修了，`VAL-INFRA-008`、`012` 不用裁决）。三项后续任务记在 `docs/plans/2026-11-16-pid-dev-spec/plan.md` §10 阶段 4 提纲的第 6 条。

推送：定为先把本地 `main` 合进这条分支（`main` 上另一条线当天多了 13 个提交，是检查点 C 的界面；试合并没有冲突，那边没碰协同、设计稿和依赖），在合并后的提交上重跑门禁，过了再推到 `fork/main`。合并后的复验记在 §19.7。

### 19.7 合并 `main` 之后的复验（2026-11-20）

合并提交 `3b047c7af`：把本地 `main`（`6377fe233`）合进这条分支（`27a230189`），没有冲突。`fork/main`（`144d2c643`）和 `main` 都是它的祖先，推送是快进。门禁和端到端在这个提交上接着跑完，开跑时工作区干净；输出在 `D:\scratch\ocs-sppid\2026-11-20-cpa\merged`，`status.txt` 记着各步的退出码和用时。

| 项 | 结果 | 输出 |
|---|---|---|
| 主程序库 | 7266 条：7207 通过、55 默认不跑、4 失败（见下）；连重编测试程序 256 秒 | `unit-lib.txt` |
| 协同两个 crate | `ocs_collab` 98 条，`ocs_collab_server` 的库 187 条、`main.rs` 21 条，全过，与合并前相同 | `unit-collab.txt` |
| clippy | `--lib` 656 条（上限 656），加 `--profile test` 1051 条（上限 1051） | `clippy-lib.txt`、`clippy-test.txt` |
| 文档检查 | 590 项 589 过，没过的是合并前那一项（§19.5 已知问题 1） | `docs-check.txt` |
| 端到端 | `scripts\collab-e2e\run19.ps1 -EvidenceDir <证据目录>\e2e -PortShift 300 -Label cpa`：46 个场景加收尾的整轮检查，830 项全过，各组的场景数和项数与 §19.2 相同；`t03_vanish_retry` 第一次照旧按设计以 75 收场、重跑一次，第一次不计入。残留进程 0，桌面记录 50 次都是「没变」，跑完工作区仍干净，用时 28 分钟。计时：`VAL-DRAW-030` 中位数 50.9 毫秒（上限 100）；`VAL-DRAW-032` 图里有了 200 个符号之后 60.0 毫秒，比前一组多 17.8%（增长的上限 100%）；`VAL-DRAW-031` 中位数 664.6 毫秒（上限 3000）；`VAL-SEM-014` 中位数 24 毫秒（上限 3000）；不判的一个接一个发 114.5 毫秒。机器上照旧另有 4 个别的实例 | `build-gui.txt`、`build-server.txt`；`e2e\summary-cpa-20261120-220023.txt` |

主程序库失败的 4 条，合并前是 2 条：

- 2 条是 §19.5 已知问题 1 的设计稿比对，同前。
- `app::pid_shell::pid_shell_startup::the_screens_are_the_ones_the_design_navigates_to`，也是比对设计稿：`main` 加了「备份数据包」一屏，设计稿按约定不进提交，仓库里的设计稿没有这一屏。那条线的工作树里有没提交的设计稿，所以在那里过（`docs/plans/2026-11-17-sppid-backup-publish/todo-checkpoint-c.md`）；`main` 的干净检出上一样不过，与协同无关，归那条线补设计稿时一起修。
- `app::pid_shell::pid_shell_chrome::what_the_editor_shows_is_drawn_in_the_frame_where_a_window_of_its_own_draws_it`，比对画面像素，「no drawing」一步差 26640 个像素。整套并行跑时偶发，那条线记过两次、症状相同（`docs/plans/2026-11-17-sppid-backup-publish/readings.md` §20 是 26644 个像素）。在同一个测试程序上单独跑 3 遍都过（`rerun-pixel-1.txt` – `rerun-pixel-3.txt`；直接跑测试程序要带 `.cargo/config.toml` 里的 `RUST_MIN_STACK`，不带会栈溢出）。

按 `todo.md` §1 的固定门禁，这一遍只有那条偶发不在「比对设计文件」之列；它单独跑都过，照那条线的先例不算不过。通过数 7207，比合并前（7182）多 25 条。

推送：按 CP-A 的决定推到 `fork/main`。推送前查过远端，`fork/main` 仍是 `144d2c643`，是合并提交的祖先，推送是快进；推的是本节所在的提交。本地 `main` 没有动，它是这次推送的祖先，由那条线自己快进。
