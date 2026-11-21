# 协同端到端脚本：验收契约对照表与运行方法

这份文件回答一个问题：228 条验收契约（`VAL-…`）里，每一条现在由哪条测试、或哪个端到端脚本的哪些检查项盯着；哪些没有，要补什么。

- 对照的基准：提交 `278439b5d`（2026-11-17）。此后为补缺口加的测试和场景，表里照常引用；补到哪一类了，见第 5.1 节。
- 契约原文：mission 目录里的 `validation-contract.md`，路径见 `docs/plans/2026-11-10-mission-p1-collab-status.md` 开头。各区条数与那份文档的第 4 节一致。
- 脚本就在本目录里（2026-11-17 收进仓库，`docs/plans/2026-11-16-pid-dev-spec/todo.md` 的 CC-5）。证据（日志、截图、数据库、图纸副本）不进仓库，写到运行时指定的目录，见第 3.2 节。

## 1. 口径

**「覆盖」一列的三种取值**

| 取值 | 意思 |
|---|---|
| 有 | 契约的每条判据，在所列的测试或脚本检查项里都有断言 |
| 部分 | 一部分判据有断言；「说明」里写了哪一部分没有 |
| 缺 | 没有能重跑的检查 |

**读这张表要记住的三件事**

1. **表里说的是「有没有能重跑的检查」，不是「这一条验过没有」。** mission 的验证记录里，前六个区的 178 条和 `VAL-CONF-018` 当时都由验证器用一次性的命令和截图取过证；后三个区的其余 49 条功能做了、脚本也过了，但没有经过 mission 的验证器。两种状态列在第 2 节的合计表里。
2. **「有」里只列了单元测试的，断言是在进程里做的。** 契约写的取证工具是黑盒的（curl、WebSocket 客户端、GUI 的 REST 接口、截图）。单元测试把判据逐条断言到了，但「经 REST 或 WebSocket 走一遍也是这个结果」这一层，只有 mission 当时的一次性取证。前六个区基本都是这种情况，合计表里单列了一栏。判据落在截图上的十五条、落在耗时上的四条、落在真实进程上的六条原来是「部分」或「缺」，2026-11-17 各补了场景（第 5.1 节（一）、（二）、（三））。
3. **「有」不等于「现在是绿的」。** 单元测试的名字取自全量测试日志（基准提交上的一份，加上补缺口时重跑的），当时都通过。端到端脚本的项数取自 2026-11-12 那次全量执行，当时都通过；脚本收进仓库后（2026-11-17）又跑过五遍：前两遍都有同一项因为后来的改动不再通过；那一项当天改了，后来的改动缺的断言也在当天补了一批（补了哪些、还没有哪些，见第 5.2 节），第三遍全部通过。第四遍是补了看图的场景之后跑的，577 项里有 3 项没有通过：都在那一步没有动过的两个场景里，原因是那一阵这台机器上起一个实例比平时慢；新写的和改过的场景在这一遍里全部通过（第 3.3 节）。第五遍是补了计时的场景之后跑的，38 个场景加收尾的整轮检查共 591 项，全部通过（第 3.3 节）。补了断言的四个场景（`t10_broadcast`、`t20_session`、`x05_m3_persistence`、`x10_concurrent_insert`），项数取自第三遍。服务器进程的四个场景是 2026-11-17 新写的，项数取自当天它们经总入口跑的那一次。脚手架与入口规程的四个场景和收尾的整轮检查，也是 2026-11-17 写的；其中 `t03_vanish_retry`、`t04_entry_rerun` 和收尾的整轮检查，项数取自第二遍。看图的五个场景（`g01`–`g05`）同样是 2026-11-17 新写的；它们、加了 3 项的 `t00_smoke` 和换了读图办法的 `t21_claims`，项数取自第四遍（第 3.3 节）。计时的三个场景（`p01`–`p03`）也是 2026-11-17 新写的；它们和这一步改了等就绪上限取法的 `t02_harness_rules`，项数取自第五遍。2026-11-20 在提交 `3450558e6` 上做了全量复验（`CC-7`）：八组 46 个场景加收尾的整轮检查 830 项全部通过，表里列的单元测试也全部通过；每条的证据位置在各表最后一列（第 3.3 节、第 4 节开头）。

**测试名怎么读**

| 写法 | 在哪里 | 怎么单独跑 |
|---|---|---|
| 不带前缀，如 `app::collab::…` | 主程序的库 | `cargo test --lib <名字>` |
| `ocs_collab::…` | `crates/ocs_collab` | `cargo test -p ocs_collab <去掉前缀的名字>` |
| `ocs_collab_server::…` | `crates/ocs_collab_server` 的库 | `cargo test -p ocs_collab_server --lib <去掉前缀的名字>` |
| `ocs_collab_server[main.rs]::…` | `crates/ocs_collab_server/src/main.rs` | `cargo test -p ocs_collab_server --bin ocs_collab_server <去掉前缀的名字>` |
| 后面带「（默认不跑）」 | 标了 `#[ignore]` | 命令末尾加 `-- --ignored` |

**脚本的检查项怎么读**

- 脚本里的检查项不编号。每一项带一个标签，标签就是契约编号；另有 `setup`、`seed`、`harness` 这样不对应契约的标签。
- 表里写「脚本名 N 项」：N 是结果文件里带该契约编号的检查项数（取的是哪一次执行，见上面的第 3 件事）。
- 结果文件是 `logs/<场景名>.result.txt`，每行依次是 `PASS` 或 `FAIL`、标签、这一项的说明。

**脚本里的截图是怎么查的**

| 查法 | 用在哪里 |
|---|---|
| 只查截图请求成功、文件写出来了 | 协同场景里的大多数截图（留作证据，不拿来判） |
| 读程序上报的界面元素位置，判断元素画出来了 | 协同面板的只读标识、编辑权持有者、占用行、冲突条目、未配置提示 |
| 在截图上量像素 | `VAL-CLI-008` 的占用色；看图的五个场景（`g01`–`g05`）；冒烟场景里标 `VAL-INFRA-009`、`010` 的三项 |

量像素用的是本目录的 `shot.py`（只用标准库读 PNG）：数一个矩形里合乎条件的像素，找出与底色不同的像素和它们的外接框，比两张图哪里不一样。看图的场景量之前先确认图是新的一帧，量在哪里由图纸和应答里的矩阵算出来（第 3.2 节的「看图那一组」）。图上的文字没有任何脚本去认；契约的判据要靠认字的几条是怎么处理的，见第 5.3 节。

## 2. 合计

| 区 | 条数 | 有 | 其中只靠单元测试 | 部分 | 缺 | mission 记录：已取证 | mission 记录：未取证 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 协同服务器（`VAL-SRV`） | 30 | 30 | 24 | 0 | 0 | 30 | 0 |
| 验证基础设施（`VAL-INFRA`） | 14 | 14 | 6 | 0 | 0 | 14 | 0 |
| 权限与审计（`VAL-PERM`） | 32 | 32 | 32 | 0 | 0 | 32 | 0 |
| 语义层（`VAL-SEM`） | 39 | 39 | 38 | 0 | 0 | 39 | 0 |
| 绘图工具（`VAL-DRAW`） | 31 | 31 | 16 | 0 | 0 | 31 | 0 |
| 管线与拓扑（`VAL-PIPE`） | 32 | 32 | 31 | 0 | 0 | 32 | 0 |
| 桌面端协同客户端（`VAL-CLI`） | 20 | 20 | 0 | 0 | 0 | 0 | 20 |
| 冲突与裁决（`VAL-CONF`） | 18 | 18 | 1 | 0 | 0 | 1 | 17 |
| 跨区流程（`VAL-CROSS`） | 12 | 12 | 0 | 0 | 0 | 0 | 12 |
| **合计** | **228** | **228** | **148** | **0** | **0** | **179** | **49** |

- 「有」228 条，「部分」0 条，「缺」0 条。对照表刚做出来时后两种共 39 条，按要做的事分五类：补单元测试的一类（7 条）、要真实服务器进程的一类（6 条）、验收规程与脚手架的一类（7 条）、判据在截图上的一类（15 条）、判据为耗时的一类（4 条）。2026-11-17 都补完了，各类补了什么见第 5.1 节。
- 「有」的 228 条里，80 条有端到端脚本，148 条只靠单元测试（第 1 节的第 2 件事）。
- `VAL-DRAW-029` 是契约里预留的空号，所以绘图区是 31 条。
- 「mission 记录」两栏照抄 mission 的状态文件：已取证是它的 `passed`，未取证是它的 `pending`。
- 离线规格（`docs/plans/2026-11-16-pid-dev-spec/SPEC-collab-offline-sync.md`）§7 的 21 条成功标准（`OS-01` – `OS-21`）不是验收契约，不在上表里。它们对到哪些检查见第 4.10 节：19 条有能重跑的检查（OS-16 只靠单元测试，其余 18 条有端到端场景）；OS-21 由 `CC-7` 的全量复验取证；OS-20 在 `PUB-4` 验。

## 3. 怎么跑

### 3.1 单元测试

```
cargo test --offline --locked --lib
cargo test --offline --locked -p ocs_collab -p ocs_collab_server
```

读数（2026-11-17，补完 7 条单元测试之后）：主程序库 6945 条（6892 通过、49 默认不跑、4 失败，失败的 4 条是设计稿比对测试，与协同无关，表里没有引用）；`ocs_collab` 95 条、`ocs_collab_server` 的库 165 条、`main.rs` 18 条，全部通过。基准提交上主程序库是 6944 条（6891 通过），`ocs_collab_server` 的库是 159 条，其余相同。

提交 `adb4aed54` 上的读数（2026-11-17，在仓库的另一份检出里跑的）：主程序库 6947 条，6898 通过、49 默认不跑、没有失败；`ocs_collab` 95 条、`ocs_collab_server` 的库 165 条、`main.rs` 18 条，全部通过。那 4 条设计稿比对测试在这里通过：它们比的是 `docs/pid-platform/design/pid-platform.pen`，前面的读数是在这份设计稿有未提交改动的工作区里取的。

提交 `3450558e6` 上的读数（2026-11-20，`CC-7` 的全量复验，在这份检出里跑的，工作区干净）：主程序库 7239 条，7182 通过、55 默认不跑、2 失败；`ocs_collab` 98 条、`ocs_collab_server` 的库 187 条、`main.rs` 21 条，全部通过。失败的 2 条是拿代码比设计稿的测试（`app::pid_shell::pid_editor_screen_canvas::the_grid_is_drawn_the_way_the_design_draws_it`、`ui::style::pid_tokens::tests::the_code_states_nothing_the_design_does_not`），表里没有引用，与协同无关。原因在 `7d545f845`（2026-11-19，默认主题改成亮色）：它给代码里的主题表加了设计稿没有的 11 个变量（`canvas-grid-major` 等），网格改用变量，仓库里的设计稿没有跟着改；离线同步的起点 `51b654a87` 上就是这两条。两条命令的输出存在那一遍的证据目录里，是 `unit-lib.txt`、`unit-collab.txt`。

### 3.2 端到端脚本

**需要什么**

- Windows，Python 3（只用标准库）。
- 编好的两个可执行文件：`OpenCADStudio.exe` 和 `ocs_collab_server.exe`（debug 构建即可）。在仓库根目录执行 `cargo build --offline --locked --bin OpenCADStudio` 和 `cargo build --offline --locked -p ocs_collab_server`。
- 一个仓库之外的目录，用来放证据。
- `cargo` 和 `git` 在路径上：总入口收尾的整轮检查要跑一次 `cargo tree`（离线，不编译）和只读的 `git status`；证据目录里没有基线图时，还要用 `cargo test` 生成它（见下）；计时那一组的 `p03_tag_rule_budget` 也用 `cargo test` 跑一条默认不跑的测试。
- `curl.exe` 在路径上（Windows 10 1803 起系统自带）：计时那一组的 `p01_insert_timing` 按契约用它计每个请求的耗时；没有它，这个场景只记一项失败，不往下跑。
- GUI 起在单独的隐藏桌面上，只走本机的 REST 通道（`--http`），不动鼠标键盘，跑的时候机器可以照常用。

**路径和端口怎么给**

脚本里没有写死的本机路径。三个入口（`run19.ps1`、`run_all.ps1`、`run_cross.ps1`）都认下面三个参数，不给参数时看环境变量；直接用 `python` 跑单个场景时只看环境变量。

| 参数 | 环境变量 | 是什么 | 不给时 |
|---|---|---|---|
| `-EvidenceDir` | `OCS_E2E_EVIDENCE_DIR` | 证据目录。日志、截图、数据库、图纸副本写在它下面的 `m6\`（桌面端客户端那一组、脚手架与入口规程那一组）和 `m7\`（其余六组和收尾的整轮检查）；`t04_entry_rerun` 用的替身场景另写在 `t04\` 下 | 报错退出；给的目录在仓库里面也报错退出 |
| `-BinDir` | `OCS_E2E_BIN_DIR` | 两个可执行文件所在的目录 | 仓库的 `target\debug` |
| `-PortShift` | `OCS_E2E_PORT_SHIFT` | 加在每个端口上的平移量 | 0，即契约的端口段：GUI 3210–3239、服务器 3240 |

- 另有两个只认环境变量的：`OCS_E2E_BASE_DWG`（场景拿来复制的基础图，不给时用仓库里的 `tests/fixtures/plot/page-setups-metric.dwg`）、`OCS_E2E_EXTRA_ARGS`（追加给每个 GUI 实例的命令行参数）。
- 契约的端口段被占用时（比如同一台机器上另一个工作区也在跑），给 `-PortShift 100`，整段挪到 3310–3359。检查项的说明文字里写的仍是契约的端口号，实际用的端口看控制台输出里的 `port=`。
- 例：`powershell -File scripts\collab-e2e\run19.ps1 -EvidenceDir <仓库外的目录> -Label <这一轮的名字>`。

**46 个场景，分八组，另有一项收尾的整轮检查**

| 组 | 场景 | 入口 |
|---|---|---|
| 脚手架与入口规程（4 个） | `t00_smoke`、`t02_harness_rules`、`t03_vanish_retry`、`t04_entry_rerun` | `run_all.ps1 -Only <场景名> -New` |
| 冲突与跨区流程（11 个） | `c01_field_merge`、`c02_object_topology`、`c03_resolution`、`x01_journey`、`x02_claim_exclusive`、`x04_readonly_role`、`x05_m3_persistence`、`x06_first_visit`、`x07_conflict_closure`、`x10_concurrent_insert`、`x12_after_session` | `run_cross.ps1` |
| 桌面端客户端（8 个） | `t20_session`、`t21_claims`、`t23_stale`、`t24_reconnect`、`t25_offline`、`t10_broadcast`、`t11_same_user`、`t22_ttl` | `run_all.ps1` |
| 后补的跨区流程（3 个） | `y03_pipe_topology`、`y08_offline_reconcile`、`y09_connector_cross_drawing` | `run_cross.ps1 -Only <场景名> -New` |
| 服务器进程（4 个） | `s01_restart`、`s02_port_in_use`、`s03_disconnect`、`s04_concurrent_ops` | `run_cross.ps1 -Only <场景名> -New` |
| 看图（5 个） | `g01_palette`、`g02_insert`、`g03_preview`、`g04_tag_display`、`g05_pipe_break` | `run_cross.ps1 -Only <场景名> -New` |
| 计时（3 个） | `p01_insert_timing`、`p02_bulk_tags`、`p03_tag_rule_budget` | `run_cross.ps1 -Only <场景名> -New` |
| 离线使用与上线后同步（8 个） | `o01_restart_resubmit`、`o02_ids_and_duplicates`、`o03_align_refused`、`o04_diverged_sticks`、`o05_refused_rollback`、`o06_unsynced_exit`、`o07_no_server_dxf`、`o08_server_store` | `run_cross.ps1 -Only <场景名> -New` |
| 收尾的整轮检查（1 项） | `r01_entry_rules` | 总入口每一轮都在最后跑它 |

- **总入口 `run19.ps1`**：八组按表里的顺序依次跑，最后跑 `r01_entry_rules`。除上面三个参数外还有 `-Label`（这一轮的名字）、`-OnlyHarness`（只跑第一组）、`-Only19`（只跑第二、三组）、`-OnlyNew`（只跑第四组）、`-OnlyServer`（只跑第五组）、`-OnlyPictures`（只跑第六组）、`-OnlyTiming`（只跑第七组）、`-OnlyOffline`（只跑第八组）；几个一起给时，跑的是各自那几组合起来。不管挑了哪几组，收尾的 `r01_entry_rules` 都照跑。
- **脚手架与入口规程那一组**判的不是产品功能，是「验收是不是照规程做的」（规程见下面的「脚手架和入口照着做的规程」）：
  - `t00_smoke`：带一张真实图纸在端口 3210 起一个 GUI，查就绪（60 秒之内、响应的五个字段）和启动前查过端口；再查截图接口答的东西：写出的文件是 PNG、宽高与应答里报的一致（`VAL-INFRA-009`），应答里的坐标矩阵、实体的屏幕坐标与截出来的图对得上（`VAL-INFRA-010`，用的是场景自己画的一条线）。
  - `t02_harness_rules`：把脚手架里的三条规则逐条试一遍。它有意去要段外的端口、占住端口、起不会就绪的实例、强行结束实例，控制台输出里的「没有就绪」「消失」是它要的。
  - `t03_vanish_retry`：建实体的流程做到一半，把实例从外面结束，看入口把整个场景重跑一遍。**它的第一次尝试一定以退出码 75 收场**，汇总里它占两行，这是正常的。要经入口跑：重跑是入口做的。
  - `t04_entry_rerun`：不起 GUI，用四个只会按设定的退出码退出的替身场景，试入口的重跑规则。
- **收尾的整轮检查 `r01_entry_rules`**：总入口一开头先执行 `python r01_entry_rules.py --begin`，记下开跑的时刻、`git status --porcelain` 和脚本旁边已有的字节码缓存；最后再跑它，核对五件事：
  - 这一轮没有往仓库里写东西（`VAL-INFRA-013`）；
  - 丢过实例的场景都重跑了一次、只重跑了一次、第一次的结果留着，没丢的都只跑了一次（`VAL-INFRA-007`）；
  - 这一轮每一次 GUI 启动的端口都在段内、启动前都查过（`VAL-INFRA-008`）；
  - 桌面程序的依赖树里没有 `tokio`、`async-std`，有 `tungstenite`（`VAL-CLI-019`）；
  - 两份脚手架里这几条规则的代码一字不差。

  它跑的 `git status` 是只读的（带 `--no-optional-locks`，连索引也不刷新）。单独跑它（`run_cross.ps1 -Only r01_entry_rules -New`）之前，要先执行带 `--begin` 的那一步，否则第一项失败并说明原因。
- **服务器进程那一组**不起 GUI、不用图纸，只起 `ocs_collab_server.exe` 和几条 WebSocket 连接；入口照样要求两个可执行文件都在。
- **看图那一组**各起一个不连协同的 GUI，开一份基础图的副本，不用种子图和基线图；判据落在程序自己截的图上。五个场景共同的做法写在 `gh.py` 里：
  - 起实例之前先删掉它自己的设置目录（在证据目录里），窗口布局从程序的默认值开始。程序把面板停靠在哪一侧记在设置里；不删的话，同一个证据目录里上一轮留下的布局会带进这一轮。
  - 要量的每一张图都先确认是新的一帧。截图接口先按请求改选择集和镜头，再等窗口重画，最多等 50 毫秒，等不到就把手头的那一帧存下来——这时应答里的矩阵和位置已经是新的，图还是旧的。应答里的 `waited_for_frame` 说的就是等到了没有；没等到的，同一个请求再发，直到等到。重发不改图纸。
  - 量在哪里是算出来的：实体的坐标取自图纸和符号库，经应答里的矩阵换成像素；调色板的面板和格子在哪里，取自程序上报的矩形。命令行面板盖住的那一条不量。
  - 调色板的页签经控制通道选（`action pid_palette tab=<分类名>`），不动鼠标。
- **计时那一组**判的是耗时，读数随机器的快慢和当时忙不忙变。契约要求测的时候没有编译在跑、也没有别的实例；`p01`、`p02` 在计时前后各数一次机器上的编译器进程和别的实例，写进检查项的说明，只记不判：机器不干净时读数在限内，照样算过；超了，说明里写明要在干净的时候重测才算数。`p01`、`p02` 的读数取自 debug 构建的程序，`p03` 的取自 cargo 按测试配置编出来的测试程序（cargo 报的是 `test` profile [optimized]）；最近一遍的见第 3.3 节。
  - `p01_insert_timing`（`VAL-DRAW-030`、`032`）：起一个不连协同的 GUI，在新建的空图里建 5000 条线。一次放置是契约写的三个请求（`PIDINSERT vessel`、给点、回车），每个请求由 `curl.exe` 发出、由它自己计时（`time_total`），三个数相加是一个样本，里面没有起进程的时间。连放 20 次取中位数，要求不超过 100 毫秒；再把符号库里的符号轮着放，放到图里共有 200 个，然后再放 20 次：中位数同样不超过 100 毫秒，比第一组的增长不超过 100%。**每个计时的请求之前停 0.2 秒**（为什么，见第 5.3 节 `VAL-DRAW-030`、`032` 那一行）；不停顿、一个接一个发的第三组也量、也写进说明，不拿来判。20 个原始样本都留在证据目录的 `p01-timing.json` 里。
  - `p02_bulk_tags`（`VAL-DRAW-031`）：起一个 GUI，在新建的空图里用一个批量请求（`pid_insert_batch`）放 2000 个设备符号，做三遍，每遍换一张新的空图。按契约的口径计时：从批量请求之前最后一次 `/api/v1/ready` 答回来起，到第 2000 个实例读得到非空的位号为止，用脚本的单调时钟，取三遍的中位数，要求不超过 3000 毫秒。停表之后读回：带作者记录的实体 2000 个，位号两两不同，头一个 `P-1001`、末一个 `P-3000`。
  - `p03_tag_rule_budget`（`VAL-SEM-014`）：不起 GUI。契约指定的证据是主程序库里一条默认不跑的测试（`tag_rule_bulk_2000_under_three_seconds`），场景在脚本所在的仓库里用 `cargo test --offline --locked --lib -- --ignored --nocapture <测试名>` 跑它，判两件事：cargo 的退出码是 0、正好跑了这一条并且通过；测试打印的三轮用时，中位数不超过 3000 毫秒，每轮的末个位号是 `P-3000`、2000 个位号两两不同。cargo 先要把测试程序编出来：2026-11-17 的几次执行里它每次都重新编了一遍（控制台里有 `Compiling` 一行），连编带跑 66–184 秒，测试本身只有几十毫秒。源码没有变也重编，原因在 `build.rs`：它向 cargo 声明了「`packaging/windows/AppIcon.ico` 变了就重编」，声明之前没有看这个文件在不在，而它只在做发布构建之前才生成，平时不在，版本库里也没有；文件不在，cargo 每次都把主程序这个包当成变过的。让 cargo 自己说理由（`cargo test --offline --locked --lib --no-run -v`，连着两轮，其间什么都没有改），两轮都是其余 661 个包不用重编，只有这一个包要重编，理由是这个文件不存在，各编了一分多钟。这几次都是在仓库的另一份检出里跑的；这个文件在普通的检出里同样没有，所以应当也是这样，没有另试。这是 `build.rs` 的事，场景没有绕开它，`build.rs` 也没有改。cargo 的输出原样存在证据目录的 `p03-cargo.console.txt` 里。
- **离线那一组**（`SPEC-collab-offline-sync.md` §8；八个场景都是 2026-11-20 写的，`o01`–`o04` 在 `OFF-12`，`o05`–`o08` 在 `OFF-13`）判的是离线规格 §7 的成功标准，每个检查项标的是 OS 编号，哪一条对到哪几项见第 4.10 节；只有 `o05` 最后一项标的是需求编号 `REQ-COLL-004`（项目管理员把只读的图纸切回协同编辑，之后新的会话能提交；2026-11-20 修服务器时加的，见离线规格 §11.18）。起真实的服务器和桌面端，用 `x00_seed.py` 那张种子图的副本；杀掉、重起服务器（同一个库，或者换一个新库、换回另存的库；`o08` 另用打不开的库、带 `--new-repo-id` 起），让桌面端退出再重开同一份图。服务器进程没了、连不上时，`o08` 把请求和 WS 连接记成「没有应答」照常判下去（那几项不过），不抛异常。看的是 §8 说的几样：状态接口（`align`、`align_code`、`align_message`、`unsynced_count`、`queued_ops`，和拒绝记录 `rejections`）、服务器的操作日志（直接读库）、桌面端收发的帧（实例日志里记的）、存盘文件里的三条记录，另有 `PIDCOLLAB UNSYNCED` 在命令行上列出的和 `PIDCOLLAB EXPORT` 导出的文件。三条记录由一个不连协同的实例打开存盘文件读出：存盘的实例只把记录写进存下的那一份，不写回它接着编辑的文档。退出照人的做法：`QUIT`，问「修改未同步」时答「退出」（控制接口的 `dialog_ok`）。这些做法写在共用的 `oh.py` 里。命令行上的提示行开头带「ⓘ 」，场景比对 `PIDCOLLAB UNSYNCED` 的输出之前先去掉它。面板上的字和退出提示取不到原文，`o06` 换了读法：
  - 面板上待同步的那一行：控制接口读不到字。读的是这一行画没画出来（程序上报的界面元素里有没有 `pid-collab-sync`）和算这一行用的两个数（`connected`、`queued_ops`），另留截图；字本身由程序内的测试按同样的两个数核对（第 4.10 节的 OS-18）。
  - 退出时问「修改未同步」：对话框是工具窗，两次更新之间停在界面状态的 `tool_windows` 里，`modal` 通常是空的。场景看 `tool_windows` 里有没有 `CollabUnsynced`，答「取消」「退出」用控制接口的 `dialog_cancel`、`dialog_ok`；问的那句话由程序内的测试核对。
- **种子图和基线图**：证据目录里没有时，总入口先生成。两张种子图用 `x00_seed.py`、`t01_seed.py`；`x05_m3_persistence`、`x12_after_session` 用的基线图 `PID-1101-m3.dwg` 由一条默认不跑的测试写出（`write_the_m3_baseline_drawing`），总入口用 `cargo test` 跑它，第一次要等测试程序编出来（几分钟）。基线图是一份还没有进过任何协同房间的图，不带同步记录和排队的修改（`REQ-COLL-015`）；带着的话，桌面端一加入就被服务器按库标识不符拒绝，这两个场景不过。往返测试用的那份数据从 `5f672c831` 起带着这两条记录，生成器在写出之前把它们去掉（2026-11-18 改的）；在这之间生成的基线图是带着的，证据目录里有这样一份时，删掉它再跑一次总入口。分组入口不生成这三张图：换了新的证据目录时，先跑一次总入口。只跑第一组、第五组、第六组、第七组（`-OnlyHarness`、`-OnlyServer`、`-OnlyPictures`、`-OnlyTiming`）时总入口也不生成，这四组用不着；只跑第八组（`-OnlyOffline`）时只生成 `x00_seed.py` 那一张，这一组只用它。
- **单独跑几个场景**：`run_all.ps1 -Only <场景名>`、`run_cross.ps1 -Only <场景名>`，多个场景名用逗号隔开；第一、四、五、六、七、八组的场景和 `r01_entry_rules` 要再加 `-New`（表里「入口」一列写着）。
- **结果落在哪里**：都在证据目录里。每个场景一份控制台输出（`logs/<场景名>.console.txt`）和一份 `logs/<场景名>.result.txt`，场景结束时打印一行 `=== <场景名>: N PASS / M FAIL ===`；总入口收尾在证据目录的根上写 `summary-<Label>-<时间>.txt`，每个场景一行 `pass=`、`fail=`、`exit=`，每组一行小计。有检查项失败、或者进程的退出码不是 0 的场景，小计里都记成不干净（`not clean`）；重跑过的场景，第一次尝试单占一行，不计入小计。
- **入口还记什么**：每个场景结束后记下耗时、残留进程数（场景结束后还在跑、可执行文件在 `-BinDir` 或证据目录下的进程；`run_cross.ps1` 另记留下的自动化通道描述文件数），以及场景自己报的桌面记录——场景前后各看一次指针位置与前台窗口，写「没变」或「变了」。这几样只写进汇总，入口不拿它们判失败，要人看一眼。桌面记录分不出是场景动的还是当时用机器的人动的。总入口另在开头和结尾各记一次机器上不属于这一轮的同名进程数，出了失败好对照。
- **要多久**：七组加收尾检查一遍约 19 分钟（2026-11-17 最近的那一轮：脚手架与入口规程 1 分半，冲突与跨区流程 2 分多，桌面端客户端近 11 分钟，后补的跨区流程 40 秒，服务器进程 8 秒，看图 1 分钟，计时 2 分多；这一轮起一个实例 1.6 秒上下，上一轮偏慢，六组加收尾用了 20 分钟）。后加的离线那一组八个场景 7 分半上下（2026-11-20 `OFF-13` 的那一轮：`o01` 28 秒，`o02` 51 秒，`o03` 48 秒，`o04` 93 秒，`o05` 97 秒，`o06` 39 秒，`o07` 63 秒，`o08` 26 秒），只跑这一组（`-OnlyOffline`）连生成种子图和收尾检查 7 分 40 秒。最慢的是 `t22_ttl`（真等 300 秒的占用到期），其次是 `t23_stale`、`t25_offline`、`t20_session`、`t02_harness_rules`（各一分多钟），离线那一组里的 `o05`、`o04` 也各要一分半上下；计时那一组里 `p03_tag_rule_budget` 要等 cargo 编测试程序，这一轮连编带跑 70 秒。证据目录是新的时候，前面还要加上生成基线图的几分钟。八组全量加收尾检查一遍 31 分钟（2026-11-20 `CC-7` 那一遍，连生成种子图和基线图；那一遍 `p03_tag_rule_budget` 连编带跑 318 秒，当时机器上另有编译在跑）。
- **起实例慢的时候会红的一项**：`t25_offline` 里标 `VAL-CLI-015` 的一项，按契约要求服务器连不上时 GUI 启动后 5 秒内就绪。这台机器平时起一个实例两三秒，这一项过；起一个实例要五秒以上的时候（机器正忙，或者机器本来就慢）它会红，2026-11-17 的第四遍就是这样（第 3.3 节）。5 秒是契约的数，场景没有放宽。
- **原来也这样、现在跟着机器走的两项**：`t02_harness_rules` 里标 `VAL-INFRA-014` 的两项。场景先在一个连不上的端口上起一次实例，到上限换端口重试；一项要求重试的那一次在上限里就绪，另一项要求第一次的启动日志里已经写下连不上端口的原因。上限原来写死 6 秒：起一个实例要六七秒以上的时候，重试的那一次来不及就绪，第一次也还没走到连端口那一步就到时了，两项都红（第四遍就是这样）。2026-11-17 改成：场景前面三次正常启动里最慢的一次，取它的两倍加一秒，不少于 6 秒（契约的 60 秒走的是同一段代码）。改完拿一份启动到就绪要 8 秒多的变异程序试过：改之前的脚本红的就是这两项，改之后 13 项全过，上限取的是 19 秒（第 5.4 节）。第五遍全量里起实例快（前面三次最慢 1.8 秒），上限取的是下限 6 秒。
- **直接用 `python` 跑单个场景**：先设环境变量 `OCS_E2E_EVIDENCE_DIR`，再执行 `python scripts\collab-e2e\<场景名>.py`。这样跑没有入口的重跑和记账，还会在本目录留下 `__pycache__`（已被 `.gitignore` 忽略；先设 `PYTHONDONTWRITEBYTECODE=1` 就不会留）。经入口跑不会留，`r01_entry_rules` 会查这一点。
- **贴日志之前先看一眼**：入口记「桌面有没有被动过」时，会把前台窗口的标题写进控制台输出和汇总文件——也就是当时正在用的窗口。

**脚手架和入口照着做的规程**

验收契约里有几条管的是「怎么验」，不是产品行为。它们写在两个脚手架（`m6h.py`、`m7h.py`，两份里这部分代码相同）和三个入口共用的 `e2e_env.ps1` 里，对所有场景生效。

| 规程 | 做法 | 留下什么（都在证据目录的 `m6\logs`、`m7\logs` 里） |
|---|---|---|
| GUI 的端口（`VAL-INFRA-008`） | 端口只能在 3210–3239 里（平移时整段跟着平移），要段外的端口直接报错。每次启动前先查要的端口有没有人在监听（去连本机回环地址，连得上就是有）；有人在监听、或者同一个场景里另一个实例占着，就从 3239 往下另挑一个空的 | `gui-starts.jsonl`，每次启动一行：命令行、要的端口、实际用的端口、查过哪些端口和各自的结果 |
| 等就绪的上限（`VAL-INFRA-014`） | 到上限还没有就绪、或者没就绪进程就退出了：结束进程，留下日志尾部，换一个端口再起一次，再等一个上限；第二次仍不就绪就结束它，没有第三次。上限在冒烟场景里是契约的 60 秒，其余场景是 180 秒（第 5.3 节） | `gui-retries.jsonl`，每次重试一行：原来的端口、等了多久、轮询了几次、日志尾部、重试用的端口和结果 |
| 实例消失（`VAL-INFRA-007`），脚手架这一半 | 就绪过的实例，场景没有结束它、也没有让它退出，它却不在了：进程已经退出；或者请求被拒绝之后，隔半秒连探 3 次 `/api/v1/ready` 都被拒绝。记下现象，场景收尾后以退出码 75 退出。请求超时不算；场景自己结束的、让它退出的实例不算；还没就绪就退出的归上一条 | `vanished-<场景名>-<实例名>-<时间>.json`：怎么发现的、退出码、时刻、当时在做的请求、命令行、日志尾部 |
| 实例消失，入口这一半 | 场景以退出码 75 退出，就把它整个重跑一次：脚本从头再跑，要的还是原来的端口，图按场景开头的做法准备（`t03_vanish_retry` 接着用第一次那张图）。第二次又丢实例，记失败，不跑第三次。别的退出码不重跑 | 第一次的控制台输出和结果另存为 `<场景名>.attempt1.console.txt`、`<场景名>.attempt1.result.txt`；`<场景名>.runs.txt` 每次尝试一行（第几次、退出码、秒数、残留进程数） |
| 产物不进仓库（`VAL-INFRA-013`） | 证据目录不给、或给在仓库里面，入口和脚手架都直接退出；入口不让 Python 写字节码缓存；一轮开头和结尾各看一次 `git status`（见上面的 `r01_entry_rules`） | `m7\logs` 里的 `r01-git-status.txt`（跑完后的全文）、`r01-files-written.txt`（这一轮写下的文件清单） |

**这些脚本是从哪里来的**

脚本是 2026-11-17 从仓库外的工作目录收进来的，收的是最近执行的那一份，对照表对的也是它。那一份是从更早的原件复制出来的，复制时改过三处：可执行文件目录改成可以指定；端口整体加了 100，好和另一条开发线在同一台机器上同时跑；残留进程只看那条线的。收进来时：

- 端口改回了契约的端口段（服务器 3240、GUI 3210–3239），要错开时用 `-PortShift`。
- 写死的本机路径（证据目录、可执行文件目录、基础图、隐藏桌面库的位置）都改成由参数、环境变量和脚本自己所在的位置给出。基础图改用仓库里已有的 `tests/fixtures/plot/page-setups-metric.dwg`，它和原来仓库外的那一张逐字节相同。
- 基线图 `PID-1101-m3.dwg` 原来是从一个写死的路径复制的，改成缺的时候现生成。
- 三个入口共用的部分（解析上面三个参数、数残留进程）放在 `e2e_env.ps1`。`run_all.ps1` 的 `-Only` 也改成可以用逗号隔开多个场景名（`run_cross.ps1` 原来就可以）。
- 与原件逐个文件比过。有原件的 22 个脚本里，17 个把端口减回去之后与原件一字不差；5 个各改了一处：`t20_session.py`、`t23_stale.py`、`t25_offline.py`、`x06_first_visit.py` 里各有一处绕过脚手架直接写端口号的地方，改成跟着平移量走；`x05_m3_persistence.py` 的说明文字里有本机路径，改写了。两个脚手架（`m6h.py`、`m7h.py`）只改了路径和端口的取法。后补的三个场景和它们共用的 `yh.py` 没有更早的原件。
- 逐个文件查过内容：没有口令，没有 `127.0.0.1` 之外的地址。脚手架给每个 GUI 实例的本机 REST 通道用的访问令牌是每次运行随机生成的，经环境变量传给实例，不写进任何文件。
- 没有收的：日志、截图、数据库、可执行文件，以及当初制作副本用的一次性工具。
- 收进来时没有做的：场景的断言一处没动，对照表里记的缺口原样留着（此后补了哪些，见第 5.1 节）。
- 收进来之后新写的：服务器进程的四个场景（`s01`–`s04`），是为补第 5.1 节（三）的缺口写的，没有原件。为它们给脚手架 `m7h.py` 加了一个「不发 close 帧、直接销毁套接字」的做法，给总入口加了服务器进程这一组。
- 为补第 5.1 节（五）的缺口改的和新写的（2026-11-17）：两个脚手架加了端口、等就绪的上限、实例消失三条规则；三个入口里「跑一个场景」的那一段收进了 `e2e_env.ps1`，加了「退出码 75 重跑一次」；总入口加了第一组和收尾的整轮检查，汇总里开始记退出码。新写了四个脚本（`t02_harness_rules`、`t03_vanish_retry`、`t04_entry_rerun`、`r01_entry_rules`），没有原件。冒烟场景 `t00_smoke` 从端口 3230 改到契约写的 3210，等就绪的上限从 180 秒改成 60 秒，加了 5 项检查，挂进了总入口；`t20_session` 加了 1 项（`VAL-INFRA-012`）。原有的检查项没有改。
- 为补第 5.2 节前三行的断言改的（2026-11-17）：`t10_broadcast` 加了 11 项（位号来源 4 项、手工改写的标记 6 项、1 项前提），`x05_m3_persistence` 加了 12 项（大属性对象 11 项、1 项前提），`x10_concurrent_insert` 加了 14 项（加入时上报已有位号 7 项、重新加入前后的占用 3 项、4 项前提），`t20_session` 加了 1 项、改了 1 项（数「入站 hello」日志行的那一项，第 3.3 节）。四个脚本原有的其余 76 项，前后两遍全量的结果文件逐行比过：66 项一字不差，10 项只有文字里的数不同（引的日志行里的时间戳、进程号、用时的秒数，和一个实体句柄——新加的步骤把那个阀门删掉又放回，句柄换了），没有一项的写法变了。没有新写脚本，脚手架和入口没有动。
- 为补第 5.1 节（一）的缺口新写的和改的（2026-11-17）：新写了看图的五个场景（`g01`–`g05`）和它们共用的两个模块——`shot.py`（只用标准库读 PNG、量像素）、`gh.py`（等新的一帧、清设置目录、坐标与像素互换），都没有原件。冒烟场景 `t00_smoke` 加了 3 项（`VAL-INFRA-009` 1 项、`VAL-INFRA-010` 2 项），原有的 7 项没有动。`t21_claims` 里量占用色的那一项（`VAL-CLI-008`）原来用另装的图像库读图，改成用 `shot.py`，数哪种颜色、数哪个矩形都没有变；改的这一项前后两遍的结果一字不差，其余 14 项里 12 项一字不差、2 项只有会话号和到期时刻不同。总入口加了看图这一组和 `-OnlyPictures`；`run_cross.ps1` 只改了开头的说明。两个脚手架、`e2e_env.ps1`、`run_all.ps1` 没有动。
- 为补第 5.1 节（二）的缺口新写的和改的（2026-11-17）：新写了计时的三个场景（`p01`–`p03`）和前两个共用的模块 `ph.py`（让 `curl.exe` 发一个请求并报它自己计的用时，数机器上在跑的编译器和别的实例，新建空图），都没有原件。`curl.exe` 要发的地址、请求体和实例的访问令牌经标准输入给它，不出现在命令行上，也不写进文件。总入口加了计时这一组和 `-OnlyTiming`；`run_cross.ps1` 只改了开头的说明。`t02_harness_rules` 改了等就绪上限的取法（见上面的「原来也这样、现在跟着机器走的两项」），13 项检查判什么没有变。两个脚手架、`e2e_env.ps1`、`run_all.ps1` 没有动。
- 为离线使用与上线后同步新写的（2026-11-20，`OFF-12`）：新写了离线的四个场景（`o01`–`o04`）和它们共用的模块 `oh.py`（读状态接口、操作日志、收发的帧和存盘文件里的三条记录，导出队列，照人的做法退出，另存和换回服务器的库），都没有原件。总入口加了离线这一组和 `-OnlyOffline`。两个脚手架、`e2e_env.ps1`、`run_all.ps1`、`run_cross.ps1` 没有动。
- 为离线使用与上线后同步新写的（2026-11-20，`OFF-13`）：新写了离线的另外四个场景（`o05`–`o08`），都没有原件。第八组按文件名找场景，总入口没有改；`oh.py`、两个脚手架、`e2e_env.ps1`、`run_all.ps1`、`run_cross.ps1` 都没有动。

### 3.3 最近的执行

**2026-11-20，`CC-7` 全量复验：八组全量加收尾的整轮检查**（提交 `3450558e6`，工作区干净；两个可执行文件是开跑前在这份检出里编的 debug 构建，服务器 cargo 报不用重编；证据目录是新的，种子图和基线图由总入口现生成；端口平移 300，经总入口 `run19.ps1 -PortShift 300 -Label cc7`；之前先跑了第 3.1 节的两条单元测试命令）

- 46 个场景加收尾的整轮检查，共 830 项检查，全部通过。各场景的退出码都是 0（`t03_vanish_retry` 第一次按设计以 75 收场、重跑一次，第一次那一行不计入小计），残留进程都是 0，桌面记录 50 次都是「没变」。开头和结尾机器上各有 4 个不属于这一轮的同名进程（别的检出里开着的实例）。
- 各组：脚手架与入口规程 33 项；冲突与跨区流程、桌面端客户端 357 项（234、123）；后补的跨区流程 81 项；服务器进程 56 项；看图 45 项；计时 14 项；离线 239 项；收尾的整轮检查 5 项。前七组和收尾检查的 591 项与 2026-11-17 那一遍逐组相同；离线一组比 `OFF-13` 那一轮多 1 项，是 `o05` 最后标 `REQ-COLL-004` 的那一项（第 3.2 节「离线那一组」）。`x12_after_session` 的「没有弹出对话框」改成也看工具窗之后（离线规格 §11.18）仍是 19 项。
- 对照表逐行核对（第 4 节开头）：249 行列的 369 个单元测试都报 `ok`，113 处场景引用的项数都与表里写的相同、都是 `PASS`，0 处对不上。单元测试的读数见第 3.1 节：主程序库失败的 2 条是设计稿比对，表里没有引用。
- 计时的读数（上限都是契约的数；计时前后机器上有别的实例 4 个，`p01` 计时时另有 1–4 个编译器进程，不是契约要的干净环境；读数都在限内，按第 3.2 节的口径照样算过）：`VAL-DRAW-030` 20 次放置的中位数 76.0 毫秒（上限 100）；`VAL-DRAW-032` 图里有了 200 个符号之后 61.0 毫秒，比前一组少 19.7%；`VAL-DRAW-031` 三次依次 706.5、664.2、620.2 毫秒，中位数 664.2（上限 3000）；`VAL-SEM-014` 三轮 25、18、18 毫秒，中位数 18（上限 3000）。
- 用时：从开跑到收尾 31 分钟（含生成种子图和基线图）。`p03_tag_rule_budget` 连编带跑 318 秒，编测试程序用了 5 分 12 秒，比 2026-11-17 的 66–184 秒长，当时机器上另有编译在跑。

**2026-11-17，七组全量加收尾的整轮检查，补了计时的场景之后**（可执行文件与前两遍是同一份：提交 `adb4aed54` 的源码的 debug 构建，此后产品的源码没有改过；脚本是补了计时场景之后的；端口不平移，经总入口 `run19.ps1`）

- 38 个场景加收尾的整轮检查，共 591 项检查，全部通过。上一遍没有通过的 3 项这一遍都过了：`t02_harness_rules` 的 2 项（`VAL-INFRA-014`）和 `t25_offline` 的 1 项（`VAL-CLI-015`，这一遍启动后 1.58 秒就绪）。这一遍起实例不慢：从启动到就绪 58 次的中位数 1.6 秒、最慢 2.5 秒（上一遍 55 次的中位数 5.9 秒）。所以这一遍的全绿说明不了 `t02_harness_rules` 改得对不对，说明它的是第 5.4 节里慢启动的那一对对照；这一遍它取的上限是下限 6 秒（前面三次正常启动最慢 1.8 秒）。
- 各组：脚手架与入口规程 33 项；冲突与跨区流程 234 项；桌面端客户端 123 项；后补的跨区流程 81 项；服务器进程 56 项；看图 45 项；计时 14 项（`p01` 8、`p02` 4、`p03` 2）；收尾的整轮检查 5 项。
- 比上一遍多 14 项，就是计时的三个场景。其余 35 个场景和收尾的整轮检查，各标签的项数与上一遍逐个比过，全部相同；通过的项数只有 `t02_harness_rules`（11 → 13）和 `t25_offline`（12 → 13）不同。
- 计时的读数（上限都是契约的数）：`VAL-DRAW-030` 20 次放置的中位数 55.9 毫秒（上限 100）；`VAL-DRAW-032` 图里有了 200 个符号之后 37.0 毫秒，比前一组少 33.9%（上限 100 毫秒，增长不超过 100%）；`VAL-DRAW-031` 三次依次 478.3、439.9、534.6 毫秒，中位数 478.3（上限 3000）；`VAL-SEM-014` 三轮 16、15、15 毫秒，中位数 15（上限 3000）。不停顿的一组（不判）中位数 113.6 毫秒。
  - 契约要求量的时候机器上没有编译器在跑、没有别的实例。场景数到的是：`VAL-DRAW-030` 那一组量之前是干净的，量完时多出 1 个 `rustc`（不是这一轮起的）；`VAL-DRAW-031` 量的时候也有 1 个；`VAL-DRAW-032` 那一组前后都干净。
  - 那一轮编译停了以后，把计时那一组单独又量了一遍（`run19.ps1 -OnlyTiming`，18:01–18:03，每一组前后数到的都是 0 个编译器、0 个别的实例），14 项全过：`VAL-DRAW-030` 38.9 毫秒，`VAL-DRAW-032` 38.6 毫秒（少 0.7%），`VAL-DRAW-031` 449.9、461.9、442.4 毫秒，中位数 449.9，`VAL-SEM-014` 17、15、15 毫秒；不停顿的一组 112.6 毫秒。第 5 节各表里这三个场景的项数取自全量那一遍。
- `t03_vanish_retry` 照旧跑了两次：第一次按设计丢了实例、以退出码 75 结束（1 项通过，另存为 `attempt1`，不计入上面的数），入口重跑，第二次 5 项全过。别的场景都只跑了一次，退出码都是 0。
- 收尾检查记下的：这一轮写了 1550 个文件，都在证据目录里；`git status` 前后都是 7 行（脚本是从仓库的另一份检出里跑的，那里改过的和新加的就是这一步的 7 个脚本文件），没有多出一行；没有新的字节码缓存。GUI 启动 63 次（31 个脚本），端口都在 3210–3239 之内、启动前都查过；其中 5 次在段内换过端口，都是 `t02_harness_rules` 有意做的，别的场景没有一次换端口、没有一次就绪超时重试。依赖树 572 个包，没有 `tokio`、`async-std`，有 `tungstenite` 0.29.0。
- 没有残留进程，没有留下描述文件，开始和结束时机器上都没有别的实例。桌面记录 42 条里 22 条是「没变」、20 条是「变了」（20 条记的都是其间有人在用这台机器，系统记到了输入）。从第一个场景到收尾约 19 分钟；此前生成基线图用了约 1 分钟。
- 这一遍各项的证据没有逐条核对。正式的全量复验是任务清单里的 CC-7。

**2026-11-17，六组全量加收尾的整轮检查，补了看图的场景之后**（可执行文件与上一遍是同一份：提交 `adb4aed54` 的源码的 debug 构建，此后产品的源码没有改过；脚本是补了看图场景之后的；端口不平移，经总入口 `run19.ps1`）

- 35 个场景加收尾的整轮检查，共 577 项检查，574 项通过，3 项没有通过。新写的看图五个场景、加了检查的 `t00_smoke`、换了读图办法的 `t21_claims` 全部通过；没有通过的 3 项在这一步没有动过的两个场景里，都是因为这一遍里起一个实例比平时慢：
  - `t02_harness_rules` 2 项（`VAL-INFRA-014`）。场景把等就绪的上限缩成 6 秒，先在一个连不上的端口上起一次，到时换端口重试。这一遍重试的那一次在上限里没有就绪（上一遍 3.1 秒就绪）；第一次也还没走到连端口那一步就到时了，启动日志里没有留下原因。
  - `t25_offline` 1 项（`VAL-CLI-015`）。契约要求服务器连不上时 GUI 启动后 5 秒内就绪；这一遍用了 8.93 秒，上一遍 2.09 秒。
  - 两遍用的可执行文件是同一份，这两个场景的脚本没有改过。从启动到就绪，这一遍 55 次的中位数是 5.9 秒，上一遍 51 次是 3.5 秒。这一遍跑的时候有人在用这台机器（见下面的桌面记录），另有程序占着显卡和处理器；启动具体慢在哪里没有查清。
  - 跑完后把这两个场景单独各跑了一遍，机器还是那样（10 次启动的中位数 9.6 秒）：`t02_harness_rules` 12 项通过、1 项没过（重试的那一次 7.6 秒就绪，到时之前发出的最后一次询问等到了应答；第一次的日志里仍没有原因），`t25_offline` 12 项通过、1 项没过（8.41 秒）。这三项对起实例的快慢敏感：`t25_offline` 的一项见第 3.2 节的「起实例慢的时候会红的一项」；`t02_harness_rules` 的两项后来改了等就绪上限的取法，见同一节的「原来也这样、现在跟着机器走的两项」。
- 各组：脚手架与入口规程 33 项（2 项没过）；冲突与跨区流程 234 项；桌面端客户端 123 项（1 项没过）；后补的跨区流程 81 项；服务器进程 56 项；看图 45 项（`g01` 17、`g02` 8、`g03` 6、`g04` 9、`g05` 5）；收尾的整轮检查 5 项。
- 比上一遍多 48 项：看图的五个场景 45 项，`t00_smoke` 7 → 10。其余 29 个场景和收尾的整轮检查，各标签的项数与上一遍逐个比过，全部相同；通过的项数只有上面两个场景不同。
- `t03_vanish_retry` 照旧跑了两次：第一次按设计丢了实例、以退出码 75 结束（1 项通过，另存为 `attempt1`，不计入上面的数），入口重跑，第二次 5 项全过。别的场景都只跑了一次；退出码除 `t02_harness_rules`、`t25_offline` 是 1 之外都是 0。
- 收尾检查记下的：这一轮写了 1499 个文件，都在证据目录里；`git status` 前后都是 11 行（脚本是从仓库的另一份检出里跑的，那里改过的和新加的就是这一步的 11 个脚本文件），没有多出一行；没有新的字节码缓存。GUI 启动 61 次（29 个脚本），端口都在 3210–3239 之内、启动前都查过；其中 5 次在段内换过端口，都是 `t02_harness_rules` 有意做的，别的场景没有一次换端口、没有一次就绪超时重试。依赖树 572 个包，没有 `tokio`、`async-std`，有 `tungstenite` 0.29.0。
- 没有残留进程，没有留下描述文件，开始和结束时机器上都没有别的实例。桌面记录 39 条里 22 条是「没变」、17 条是「变了」（`x00_seed`、`t25_offline`、`t10_broadcast`、`t22_ttl`，以及从 `y03_pipe_topology` 起直到收尾的 13 个；17 条记的都是其间有人在用这台机器，系统记到了输入）。从第一个场景到收尾约 20 分钟；此前生成基线图用了约 4 分钟（要先把测试程序编出来）。
- 这一遍各项的证据没有逐条核对。正式的全量复验是任务清单里的 CC-7。

**2026-11-17，五组全量加收尾的整轮检查，补了第 5.2 节的断言之后**（可执行文件是提交 `adb4aed54` 的源码的 debug 构建，在仓库的另一份检出里编的；脚本是补了断言之后的；端口不平移，经总入口 `run19.ps1`）

- 30 个场景加收尾的整轮检查，共 529 项检查，全部通过。`t20_session` 里前两遍失败的那一项改过了（第 5.2 节），这一遍通过。
- 各组：脚手架与入口规程 30 项；冲突与跨区流程 234 项；桌面端客户端 123 项；后补的跨区流程 81 项；服务器进程 56 项；收尾的整轮检查 5 项。
- 比上一遍多 38 项，都在补了断言的四个场景里：`t10_broadcast` 32 → 43，`t20_session` 16 → 17，`x05_m3_persistence` 14 → 26，`x10_concurrent_insert` 15 → 29。其余 26 个场景和收尾的整轮检查，各标签的项数与上一遍逐个比过，全部相同。
- `t03_vanish_retry` 照旧跑了两次：第一次按设计丢了实例、以退出码 75 结束（1 项通过，另存为 `attempt1`，不计入上面的数），入口重跑，第二次 5 项全过。别的场景都只跑了一次，退出码都是 0。
- 收尾检查记下的：这一轮写了 1369 个文件，都在证据目录里；`git status` 前后都是 4 行（脚本也是从那份检出里跑的，那里改过的只有补了断言的四个脚本），没有多出一行；没有新的字节码缓存。GUI 启动 56 次（24 个脚本），端口都在 3210–3239 之内、启动前都查过；其中 5 次在段内换过端口，都是 `t02_harness_rules` 有意做的，别的场景没有一次换端口、没有一次就绪超时重试。依赖树 572 个包，没有 `tokio`、`async-std`，有 `tungstenite` 0.29.0。
- 没有残留进程，没有留下描述文件，开始和结束时机器上都没有别的实例。桌面记录 34 条里 31 条是「没变」、3 条是「变了」（`t02_harness_rules`、`t03_vanish_retry` 的第一次、`t22_ttl`；三条记的都是其间有人用过这台机器，系统记到了输入）。从第一个场景到收尾约 17 分钟；此前生成基线图用了不到 5 分钟（要先把测试程序编出来）。
- 这一遍各项的证据没有逐条核对。正式的全量复验是任务清单里的 CC-7。

**2026-11-17，五组全量加收尾的整轮检查，补完验收规程之后**（可执行文件是提交 `b3b366072` 的源码的 debug 构建，脚本是补完验收规程之后的；端口不平移，经总入口 `run19.ps1`）

- 30 个场景加收尾的整轮检查，共 491 项检查：490 项通过、1 项失败。失败的仍是 `t20_session` 里那一项（说明在下面 2026-11-17 原有 22 个场景那次执行的记录后面），其余 29 个场景和收尾检查全部通过。
- 各组：脚手架与入口规程 30 项（`t00` 7、`t02` 13、`t03` 5、`t04` 5）；冲突与跨区流程 208 项；桌面端客户端 110 项通过、1 项失败（`t20` 现在是 16 项：原有 15 项加 `VAL-INFRA-012` 一项）；后补的跨区流程 81 项；服务器进程 56 项；收尾的整轮检查 5 项。
- `t03_vanish_retry` 跑了两次：第一次按设计丢了实例、以退出码 75 结束（1 项通过，控制台和结果另存为 `attempt1`，不计入上面的数），入口重跑，第二次 5 项全过。别的场景都只跑了一次；退出码除 `t20_session` 是 1 之外都是 0。
- 收尾检查记下的：这一轮写了 1394 个文件，都在证据目录里；`git status` 前后都是 14 行，没有多出一行；没有新的字节码缓存。GUI 启动 54 次（24 个脚本），端口都在 3210–3239 之内、启动前都查过；其中 5 次在段内换过端口，都是 `t02_harness_rules` 有意做的，别的场景没有一次换端口、没有一次就绪超时重试。依赖树 572 个包，没有 `tokio`、`async-std`，有 `tungstenite` 0.29.0。
- 没有残留进程，没有留下描述文件，开始和结束时机器上都没有别的实例。桌面记录全部是「没变」。从第一个场景到收尾约 15 分钟；此前生成基线图用了不到 5 分钟（要先把测试程序编出来）。
- 原有 22 个场景和服务器进程的 4 个场景，各场景各标签的项数与此前记下的逐个比过：除 `t20_session` 多出 `VAL-INFRA-012` 一项、它过时的那一项仍然失败之外，全部相同。
- 这一遍各项的证据没有逐条核对。正式的全量复验是任务清单里的 CC-7。

**2026-11-17，只跑服务器进程那一组**（服务器是提交 `412f10a2c` 的 debug 构建，端口不平移，经总入口 `run19.ps1 -OnlyServer`）

- 4 个场景 56 项全部通过：`s01` 19、`s02` 12、`s03` 9、`s04` 16。退出码都是 0，没有残留进程，桌面记录四个都是「没变」。整组 13 秒。
- 此前各自重复跑过（`s04` 12 遍，其余各 4 遍），每一遍都全部通过。
- 其余各组这一次没有跑。

**2026-11-17，原有的 22 个场景（现在的第二、三、四组），用本目录里的脚本**（提交 `6b1be96ac` 的 debug 构建，端口不平移，经总入口 `run19.ps1`）

- 22 个场景里 21 个全部通过，`t20_session` 有 1 项失败：共 398 项通过、1 项失败。
- 每个场景的检查项、各标签下的项数，都与 2026-11-12 那次相同。
- 没有残留进程，没有留下描述文件，开始和结束时机器上都没有别的实例；跑完之后仓库里没有多出文件。桌面记录 11 个场景是「没变」、11 个是「变了」。
- 这一遍是为了确认收进来的入口能用，各项的证据没有逐条核对。正式的全量复验是任务清单里的 CC-7。

这一遍和补完验收规程之后那一遍失败的那一项在 `t20_session` 里，是标 `VAL-CLI-001` 的五项之一（「服务器日志出现一条入站 hello」）。它数服务器日志里含「入站 hello」的行，要求正好一行。加入时上报已有位号的改动（第 5.2 节表里的第一行）之后，服务器对同一次 hello 记两行：上报的结果一行，发出 `welcome` 一行，所以数出来是 2。同一标签下的另外四项（会话表正好新增一行、客户端收到的 `welcome`、状态接口、`welcome` 帧的形状）都通过。这是脚本的写法落后于改动，不是功能退了。收进仓库时只搬运、不改断言，所以当时没有动它；2026-11-17 补第 5.2 节的断言时改成只数以 `welcome` 作答的那一行，此后的那一遍它通过。

**2026-11-12，用仓库外的副本**（可执行文件构建自提交 `9bf874a34`）

22 个场景 399 项全部通过，退出码都是 0，没有残留进程，没有留下描述文件。桌面记录 6 个场景是「没变」、16 个是「变了」（分不出是谁动的，见 3.2）。对照表里原有 22 个场景中此后没有动过的 17 个，项数取自这一次；补了断言的四个取自 2026-11-17 补了断言之后的那一遍，换了读图办法的 `t21_claims` 取自补了看图场景之后的那一遍。

| 组 | 各场景的项数 | 小计 |
|---|---|---:|
| 冲突与跨区流程 | `c01` 24、`c02` 17、`c03` 20、`x01` 13、`x02` 20、`x04` 26、`x05` 14、`x06` 21、`x07` 19、`x10` 15、`x12` 19 | 208 |
| 桌面端客户端 | `t20` 15、`t21` 15、`t23` 9、`t24` 11、`t25` 13、`t10` 32、`t11` 7、`t22` 8 | 110 |
| 后补的跨区流程 | `y03` 26、`y08` 33、`y09` 22 | 81 |

这一次是在协同分支合进 `main`（`af23cffef`）和合并上游（`2278e4fba`）之前跑的。从 `9bf874a34` 到基准提交，有 10 个提交动过协同的代码（第 5.2 节）。

## 4. 对照表

各表最后一列「复验证据」是 `CC-7` 全量复验留下的文件（2026-11-20，提交 `3450558e6`，第 3.3 节的第一遍），路径相对于那一遍的证据目录（在仓库外）：

- `unit-lib.txt`、`unit-collab.txt` 是第 3.1 节两条命令的输出。「N 条」是这一行列的单元测试在里面报 `ok` 的条数，列了几条就是几条。
- `e2e/` 下是总入口的证据目录。「N 项」是那个结果文件里属于这一行的检查项数：标签是这一行的编号，「端到端脚本」（第 4.10 节是「场景与检查项」）一列另写了标签的，那个标签的也算上。都是 `PASS`，加起来与那一列写的项数相同。
- 默认不跑的那一条（`VAL-SEM-014` 的 `tag_rule_bulk_2000_under_three_seconds`）由 `p03_tag_rule_budget` 跑，证据是它存下的 cargo 输出。
- 逐行核对用的是证据目录里的 `evidence.py`：读这一节的每一行，每个测试名在输出里找 `test <名字> ... ok`，每个场景在结果文件里按标签数项数，与表里写的比，对不上的列出来。这一遍 249 行（228 条契约、21 条 OS）列出的是 0 处；各行的结果另存为同一目录里的 `evidence.tsv`。

### 4.1 协同服务器 `VAL-SRV`（30 条）

契约的取证工具是 curl 和 WebSocket 客户端，对着真实的服务器进程。多数条目由服务器自己的测试覆盖（进程内起路由和存储）；要真的重启进程、抢端口、断套接字、并发提交的六条（`024`–`028`、`030`）另有四个对着真实进程的场景 `s01`–`s04`，只起服务器和 WebSocket 连接，不起 GUI。另有三条（`008`、`013`、`016`）在桌面端的场景里有真实进程上的检查，写在「说明」里。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-SRV-001` | 健康检查返回服务就绪 | `ocs_collab_server::http::tests::health_reports_ok_with_a_working_database`<br>`ocs_collab_server::store::tests::health_check_passes_on_a_writable_database`<br>`ocs_collab_server::tests::a_memory_store_reports_a_healthy_database` | — | 有 | 各端到端场景开头的 `setup` 项也在真实进程上等 `/health` 成功 | `unit-collab.txt` 3 条 |
| `VAL-SRV-002` | 健康检查在数据库不可写时报告不健康 | `ocs_collab_server::http::tests::health_reports_failure_without_a_database`<br>`ocs_collab_server::store::tests::health_check_fails_on_a_readonly_database`<br>`ocs_collab_server[main.rs]::tests::a_database_path_that_is_a_directory_yields_an_unhealthy_state` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-SRV-003` | HTTP 建会话返回 session_id | `ocs_collab_server::http::tests::reposting_the_same_session_refreshes_instead_of_duplicating`<br>`ocs_collab_server::http::tests::a_session_created_over_http_blocks_a_second_editor` | — | 有 | 状态码与 `session_id` 有断言；`base_revision` 是整数由响应结构体的类型保证，没有单独断言 | `unit-collab.txt` 2 条 |
| `VAL-SRV-004` | WebSocket 建连并完成 hello 握手 | `ocs_collab_server::ws::tests::hello_opens_a_session_and_returns_welcome`<br>`ocs_collab::protocol::tests::hello_round_trips_through_json_with_workspace` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-005` | 未完成 hello 前的请求返回结构化错误 | `ocs_collab_server::ws::tests::a_message_before_hello_is_refused` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-SRV-006` | 未知消息类型返回结构化错误 | `ocs_collab_server::ws::tests::unparseable_frames_map_to_invalid_message`<br>`ocs_collab::protocol::tests::unknown_message_type_fails_to_deserialize` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-007` | 心跳维持会话活跃 | `ocs_collab::session::tests::heartbeat_within_the_timeout_keeps_the_session_active` | — | 有 | 时刻是注入的（3 次间隔 15 秒的心跳），不是真等 45 秒 | `unit-collab.txt` 1 条 |
| `VAL-SRV-008` | 超过超时阈值未心跳的会话判失效 | `ocs_collab::session::tests::heartbeat_after_the_timeout_marks_the_session_stale`<br>`ocs_collab::session::tests::a_heartbeat_timeout_from_config_drives_the_sweep`<br>`ocs_collab_server::ws::tests::sweeping_a_stale_session_releases_claims_and_broadcasts_peer_left`<br>`ocs_collab_server::ws::tests::a_heartbeat_on_a_stale_session_reports_expired` | — | 有 | 时刻是注入的；真实进程上等 90 秒判失效见 `VAL-CLI-003`（`t23_stale.py`） | `unit-collab.txt` 4 条 |
| `VAL-SRV-009` | 同图同模式会话可被接管 | `ocs_collab::session::tests::same_drawing_same_mode_takes_over_a_stale_session`<br>`ocs_collab_server::ws::tests::takeover_releases_the_old_sessions_claims_and_notifies_the_room` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-010` | 首次位号占用申请获批 | `ocs_collab_server::ws::tests::claim_after_hello_is_granted_and_mirrored`<br>`ocs_collab::occupancy::tests::first_claim_is_granted_and_visible` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-011` | 第二位申请者被拒且拿到占用者信息 | `ocs_collab_server::ws::tests::a_second_session_claiming_the_tag_is_denied`<br>`ocs_collab::occupancy::tests::second_claimant_is_denied_with_holder_and_expiry`<br>`ocs_collab::protocol::tests::claim_denied_reports_holder_and_expiry` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-SRV-012` | 主动释放后他人可立即占用 | `ocs_collab_server::ws::tests::release_broadcasts_claim_released_to_the_room`<br>`ocs_collab::occupancy::tests::release_frees_the_tag_for_the_next_claimant` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-013` | 占用过期自动释放并广播 | `ocs_collab_server::ws::tests::an_expired_claim_broadcasts_claim_released`<br>`ocs_collab::occupancy::tests::ttl_expiry_releases_the_tag_and_reports_it` | — | 有 | 时刻是注入的；真实进程上等占用到期见 `VAL-CLI-006`（`t22_ttl.py`） | `unit-collab.txt` 2 条 |
| `VAL-SRV-014` | 提交语义操作生效并回执 op_applied | `ocs_collab_server::ws::tests::ops_advance_the_revision_monotonically`<br>`ocs_collab_server::store::tests::commit_op_advances_the_revision_and_writes_one_audit_row` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-015` | 同一操作的 revision 单调递增 | `ocs_collab_server::ws::tests::ops_advance_the_revision_monotonically` | — | 有 | 测试里连续提交 2 次（契约写 3 次） | `unit-collab.txt` 1 条 |
| `VAL-SRV-016` | 变更广播 fanout 到同图全部会话 | `ocs_collab_server::bus::tests::a_message_reaches_every_subscriber_of_the_drawing`<br>`ocs_collab_server::bus::tests::an_async_subscriber_receives_the_published_message` | — | 有 | 真实进程上两端都收到广播见 `VAL-CLI-009`（`t10_broadcast.py`） | `unit-collab.txt` 2 条 |
| `VAL-SRV-017` | 变更不广播到其他图纸的会话 | `ocs_collab_server::bus::tests::a_message_does_not_cross_drawings` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-SRV-018` | 在线用户变化广播 peer_joined 与 peer_left | `ocs_collab_server::ws::tests::hello_opens_a_session_and_returns_welcome`<br>`ocs_collab_server::ws::tests::an_explicit_close_retires_the_session_and_tells_the_room_once`<br>`ocs_collab_server::ws::tests::own_peer_events_are_not_forwarded`<br>`ocs_collab::protocol::tests::peer_joined_carries_the_peer_inline` | — | 有 |  | `unit-collab.txt` 4 条 |
| `VAL-SRV-019` | welcome 中的 peers 与 claims 反映当前在线与占用状态 | `ocs_collab_server::ws::tests::welcome_lists_active_peers_and_outstanding_claims`<br>`ocs_collab_server::ws::tests::welcome_payload_contains_peers_and_claims`<br>`ocs_collab::occupancy::tests::claims_are_filtered_by_drawing_for_welcome` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-SRV-020` | 审计日志写入完整 9 字段 | `ocs_collab_server::ws::tests::an_op_writes_a_complete_audit_row`<br>`ocs_collab_server::store::tests::audit_rows_round_trip_with_all_nine_fields` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-021` | 审计日志不可修改与不可删除 | `ocs_collab_server::store::tests::audit_log_rejects_update_and_delete_at_the_database_level`<br>`ocs_collab_server::http::tests::audit_mutation_returns_405` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-022` | 审计按位号检索返回最近一次修改 | `ocs_collab_server::store::tests::audit_by_tag_returns_the_newest_first`<br>`ocs_collab_server::http::tests::audit_tag_filters_the_same_as_object_ref` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-SRV-023` | SQLite 落盘且启用 WAL 模式 | `ocs_collab_server::store::tests::opening_a_file_database_enables_wal`<br>`ocs_collab_server::store::tests::a_write_produces_the_wal_and_shm_sidecars`<br>`ocs_collab_server[main.rs]::tests::a_writable_path_yields_a_wal_backed_store` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-SRV-024` | 服务器重启后审计与图纸登记恢复 | `ocs_collab_server::store::tests::clearing_ephemeral_drops_sessions_and_claims_but_keeps_drawings`<br>`ocs_collab_server::ops::tests::the_drawing_is_rebuilt_from_the_op_log` | `s01_restart.py` 7 项 | 有 | 场景强行结束服务器进程，用同一条命令行、同一个库再起，逐项比对重启前后的 `/audit` 响应（含 `seq`、`change_text`）、图纸修订、操作日志和服务器重建出的图面 | `unit-collab.txt` 2 条<br>`e2e/m7/logs/s01_restart.result.txt` 7 项 |
| `VAL-SRV-025` | 服务器重启后旧会话不可继续使用 | `ocs_collab_server::store::tests::clearing_ephemeral_drops_sessions_and_claims_but_keeps_drawings`<br>`ocs_collab::session::tests::heartbeat_on_an_unknown_session_reports_unknown`<br>`ocs_collab_server::ws::tests::a_message_before_hello_is_refused`<br>`ocs_collab_server::ws::tests::a_foreign_session_id_is_rejected` | `s01_restart.py` 6 项 | 有 | 场景在重启后拿旧会话号发心跳，两种情形回的都是带 `code` 的 `error`：连接还没握手时是 `HELLO_REQUIRED`，握手之后是 `SESSION_UNKNOWN`。另查了库里旧的会话行和占用行已清、旧占用的位号可以重新占用 | `unit-collab.txt` 4 条<br>`e2e/m7/logs/s01_restart.result.txt` 6 项 |
| `VAL-SRV-026` | 端口由参数决定且默认不与其他服务冲突 | `ocs_collab_server[main.rs]::tests::port_and_db_are_taken_from_the_command_line`<br>`ocs_collab_server[main.rs]::tests::defaults_match_the_architecture_numbers`<br>`ocs_collab_server[main.rs]::tests::the_address_reported_at_startup_is_the_one_bound` | `s02_port_in_use.py` 5 项 | 有 | 场景在已被占用的端口上再起一个实例：进程以非零码退出，stderr 报「无法监听」和地址，没有改到别的端口上监听 | `unit-collab.txt` 3 条<br>`e2e/m7/logs/s02_port_in_use.result.txt` 5 项 |
| `VAL-SRV-027` | 断连不立即销毁会话 | — | `s03_disconnect.py` 7 项 | 有 | 场景直接销毁套接字（不发 close 帧），等过三轮清扫后同一用户重新握手：`welcome.claims` 里原占用还在、持有者是原来的会话，新会话去占被拒；另有发 close 帧退出的对照。原会话要到心跳超时才失效，场景不等那一段 | `e2e/m7/logs/s03_disconnect.result.txt` 7 项 |
| `VAL-SRV-028` | 端口被占用导致的启动失败不得破坏既有会话与占用数据 | — | `s02_port_in_use.py` 6 项 | 有 | 场景里第二个实例用同一端口、同一个库启动失败后，`sessions`、`claims` 两张表逐行不变，第二个实例没有打开过数据库，随后的 `POST /session` 不把原会话当成失效的 | `e2e/m7/logs/s02_port_in_use.result.txt` 6 项 |
| `VAL-SRV-029` | 会话接管通知只送达被接管的会话 | `ocs_collab_server::ws::tests::takeover_error_does_not_reach_a_third_peer` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-SRV-030` | 并发提交的 revision 分配唯一且不丢帧 | `ocs_collab_server::store::tests::commit_op_serializes_concurrent_callers`<br>`ocs_collab_server::ops::tests::an_applied_op_is_announced_before_the_drawing_is_unlocked` | `s04_concurrent_ops.py` 12 项 | 有 | 场景按契约的下限跑：8 个会话，两段各 30 条（一段抢同一修订，一段各写各的）。规模再大服务器会丢广播，见第 5.3 节 | `unit-collab.txt` 2 条<br>`e2e/m7/logs/s04_concurrent_ops.result.txt` 12 项 |

### 4.2 验证基础设施 `VAL-INFRA`（14 条）

这一区一半是产品行为（GUI 的 REST 接口），一半是验收规程（端口段、超时、重试、产物放哪里）。规程类的几条没有可断言的产品行为，判的是「脚手架和入口有没有照着做」：规则写在脚手架和入口里（第 3.2 节的规程表），`t02`、`t03`、`t04` 逐条去试，`r01` 在一轮跑完后核对整轮的记录。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-INFRA-001` | GUI 实例带真实图纸启动后 ready 可用且给出 document_id | `rest::tests::ready_refuses_the_start_tab_until_a_drawing_is_active`<br>`app::automation::tests::ready_line_carries_the_session_id` | `t00_smoke.py` 3 项 | 有 | 冒烟场景带一张真实图纸在端口 3210 起 GUI，轮询 `/api/v1/ready`：从启动进程算起 60 秒内回 200；逐个核对五个字段的值和类型（`ok`、`ready` 为真，`version`、`session_id` 是非空字符串，`document_id` 是整数，并且就是命令行上那张图所在的文档）；命令行、轮询次数、首次成功的时刻、完整的响应都写在检查项的说明里。其余场景等就绪的上限仍是 180 秒（2026-11-17 的裁决，第 5.3 节） | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t00_smoke.result.txt` 3 项 |
| `VAL-INFRA-002` | REST 控制面拒绝无 token 请求 | `rest::tests::listener_answers_while_one_connection_stalls` | — | 有 | 这个测试起真实监听端口，其中断言了不带令牌、令牌不对都回 401 | `unit-lib.txt` 1 条 |
| `VAL-INFRA-003` | documents 端点返回当前文档清单 | `rest::tests::documents_carries_document_id_alongside_id`<br>`rest::tests::rest_p1_document_surface_routes_end_to_end` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-INFRA-004` | 用 run 加显式 document_id 能执行命令并使实体数增加 | `rest::tests::headless_session_runs_commands_after_start_tab_conversion`<br>`rest::tests::rest_surface_round_trips_real_entities`<br>`rest::tests::run_route_keeps_the_op_the_body_names` | — | 有 |  | `unit-lib.txt` 3 条 |
| `VAL-INFRA-005` | `POST /api/v1/commands` 带 document_id 时命令真正执行 | `app::control::http_bridge::tests::bridge_commands_with_a_document_id_draw_on_that_document`<br>`rest::tests::commands_forward_document_id`<br>`rest::tests::commands_run_forwards_document_id` | — | 有 | 第一条经界面托管的 REST 接口发一条画圆命令，断言响应 `ok`、实体数加一；后两条断言 `document_id` 被转进执行请求 | `unit-lib.txt` 3 条 |
| `VAL-INFRA-006` | 无头实例也能执行 run（缺陷 2 已修） | `rest::tests::headless_session_runs_commands_after_start_tab_conversion`<br>`app::automation::tests::envelope_mutations_work_headless_without_a_session_id` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-INFRA-007` | 实例意外消失时重试一次仍能完成测试 | — | `t03_vanish_retry.py` 4 项<br>`t02_harness_rules.py` 3 项<br>`t04_entry_rerun.py` 4 项<br>`r01_entry_rules.py` 1 项 | 有 | 分三层。脚手架：就绪过的实例进程退出了、或者请求被拒绝后连探 3 次都被拒绝，就记下现象（退出码、日志尾部、时刻）并让场景以退出码 75 收场（`t02`，连同三种不算消失的情形）。入口：见到 75 就把整个场景重跑一次、只重跑一次，第一次的控制台和结果另存（`t04` 用替身场景试；`r01` 在一轮跑完后核对每个场景各跑了几次）。`t03` 是真实的一遍：建实体的流程做到一半，实例被从外面结束；入口重跑，第二次在同一个端口上、用同一张图把流程走完，实体数加一。与契约字面的出入见第 5.3 节 | `e2e/m6/logs/t03_vanish_retry.result.txt` 4 项<br>`e2e/m6/logs/t02_harness_rules.result.txt` 3 项<br>`e2e/m6/logs/t04_entry_rerun.result.txt` 4 项<br>`e2e/m7/logs/r01_entry_rules.result.txt` 1 项 |
| `VAL-INFRA-008` | GUI 端口限定在 3210 至 3239 区间 | — | `t00_smoke.py` 1 项<br>`t02_harness_rules.py` 4 项<br>`r01_entry_rules.py` 1 项 | 有 | 规则在脚手架里：端口段之外的端口直接拒绝；启动前先查要的端口有没有人在监听，有（别的进程在监听，或同一个场景里先起的实例占着）就在段内另挑一个；每次启动的命令行、要的端口、实际端口、查端口的结果记进 `gui-starts.jsonl`（`t02` 逐条试，`t00` 查自己那一次）。`r01` 在一轮跑完后核对这一轮的每一次启动：端口都在 3210–3239 之内（平移时按平移后的段算），启动前都查过。查端口的办法与契约写的不同，见第 5.3 节 | `e2e/m6/logs/t00_smoke.result.txt` 1 项<br>`e2e/m6/logs/t02_harness_rules.result.txt` 4 项<br>`e2e/m7/logs/r01_entry_rules.result.txt` 1 项 |
| `VAL-INFRA-009` | capture 返回可用的 PNG 文件 | `app::control::tests::capture_parks_for_the_next_frame_then_finishes_with_its_result`<br>`rest::tests::capture_hoists_its_envelope_fields_to_the_top_level` | `t00_smoke.py` 2 项（标签 `harness`）<br>`t00_smoke.py` 1 项 | 有 | 冒烟场景在真实进程上发截图请求（2026-11-17 补了文件头和宽高）：按契约原样不带 `path` 发，程序拒绝（HTTP 400，`Missing path`）；带上 `path` 再发，文件存在、开头 8 个字节是 PNG 文件头，响应里的 `width`、`height` 是正整数，并且就是解码出的图的宽高。字面出入见第 5.3 节 | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t00_smoke.result.txt` 3 项 |
| `VAL-INFRA-010` | capture 返回世界到屏幕的坐标映射与实体屏幕坐标 | `app::control::vision::tests::the_capture_annotation_covers_where_a_reference_draws`<br>`app::control::vision::tests::test_compute_grounding_identifies_visible_entities` | `t00_smoke.py` 2 项 | 有 | 标注框怎么算有单测。冒烟场景（2026-11-17 加）画一条坐标已知的线，带标记截图：应答里的矩阵是 6 个数，`annotations` 每条都有屏幕坐标，线的中点经 `world_to_pixel_matrix` 换算出的像素与 `annotations` 给的 `screen_pixel` 相差不超过 20 像素；另比画线前后两张不带标记的图，图上多出来的像素的外接框与按矩阵算出的框各边相差不超过 3 像素（矩阵和画出来的图对得上） | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t00_smoke.result.txt` 2 项 |
| `VAL-INFRA-011` | state 端点给出 revision 与当前 document_id | `app::control::tests::control_retry_is_idempotent_and_stale_state_rejected`<br>`app::control::tests::geometry_revision_moves_only_when_the_drawing_changes` | — | 有 | 单测是间接断言：改图之后拿旧的 `revision` 再请求会被拒（说明版本号动了） | `unit-lib.txt` 2 条 |
| `VAL-INFRA-012` | GUI 与协同服务器可同时监听互不冲突 | — | `t20_session.py` 1 项 | 有 | `t20_session.py` 开头：GUI（3210）和服务器（3240）同时在跑，GUI 的 `/api/v1/ready` 和服务器的 `/health` 都回 200，并用 `Get-NetTCPConnection` 查到两个端口的监听方正是这两个进程。其余带服务器的场景开头也同时等两边就绪，不另带本条编号 | `e2e/m6/logs/t20_session.result.txt` 1 项 |
| `VAL-INFRA-013` | 测试产物一律写入 scratch 目录 | — | `r01_entry_rules.py` 1 项 | 有 | 总入口开跑前记一次 `git status --porcelain` 和脚本旁边的字节码缓存，收尾时再看：`git status` 没有多出一行，缓存文件没有多、也没有被重写（被忽略的文件 `git status` 看不见，所以另比）；这一轮写下的文件清单和 `git status` 的全文留在证据目录里。证据目录不给、或给在仓库里面，入口直接拒绝执行。契约写的固定路径前缀换成了「运行时指定的、仓库之外的目录」，见第 5.3 节 | `e2e/m7/logs/r01_entry_rules.result.txt` 1 项 |
| `VAL-INFRA-014` | 实例启动超时预算内未就绪即判定不可用 | — | `t00_smoke.py` 1 项<br>`t02_harness_rules.py` 4 项 | 有 | 60 秒的上限只在冒烟场景里断言（2026-11-17 的裁决）：从启动进程到 `/api/v1/ready` 首次回 200 不超过 60 秒。「到上限没有就绪就结束进程、留下日志尾部、换一个端口重试一次、不试第三次」写在脚手架里，对所有场景生效（其余场景的上限是 180 秒）；`t02` 走同一段代码、把上限缩短了去试（取本场景前面三次正常启动里最慢一次的两倍加一秒，不少于 6 秒；此前写死 6 秒，起实例慢的时候会红）：图纸不存在的实例一直不就绪，被结束、换端口重启、记进 `gui-retries.jsonl`，第二次仍不就绪就到此为止。冒烟场景里第一次到上限没有就绪、照规则重试后 60 秒内就绪的，记通过并把重试写出来 | `e2e/m6/logs/t00_smoke.result.txt` 1 项<br>`e2e/m6/logs/t02_harness_rules.result.txt` 4 项 |

### 4.3 权限与审计 `VAL-PERM`（32 条）

契约的取证工具是 curl。由 `ocs_collab` 的权限矩阵测试和服务器的 HTTP、存储测试覆盖。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-PERM-001` | 系统管理员行的 8 个操作全部允许 | `ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-002` | 项目管理员行仅「主数据维护」拒绝 | `ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-003` | 工艺设计人员行 4 允许 4 拒绝 | `ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-004` | 校对人员行 3 允许 5 拒绝 | `ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-005` | 只读查看者行 2 允许 6 拒绝 | `ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-006` | 40 格全部有确定的允许/拒绝结果，无未定义格、无 5xx | `ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells`<br>`ocs_collab::permission::tests::role_matrix_all_40_cells_are_decided` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-007` | 校对人员编辑图纸被角色级拒绝并返回稳定错误码 | `ocs_collab::permission::tests::proofreader_cannot_edit_drawing`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-008` | 矩阵封闭性——未知角色与未知操作返回 400 而非 500 | `ocs_collab_server::perm::tests::an_unknown_user_is_an_unknown_subject`<br>`ocs_collab_server::perm::tests::an_unknown_operation_is_rejected_before_the_chain`<br>`ocs_collab_server::perm::tests::each_perm_error_carries_its_contract_status_and_code`<br>`ocs_collab_server::http::tests::permission_check_maps_errors_to_their_statuses` | — | 有 |  | `unit-collab.txt` 4 条 |
| `VAL-PERM-009` | 目标节点有显式 ACL 时直接生效且不标记继承 | `ocs_collab_server::perm::tests::an_acl_on_the_target_node_is_the_effective_node`<br>`ocs_collab::permission::tests::acl_on_the_target_node_is_not_marked_inherited` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-010` | 目标节点无 ACL 时沿树向上取最近祖先 | `ocs_collab::permission::tests::acl_inherits_nearest_ancestor_not_root`<br>`ocs_collab_server::perm::tests::an_inherited_acl_reports_the_ancestor_and_a_root_first_path` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-011` | 多级祖先均命中时取最近的一条，不是最顶层 | `ocs_collab::permission::tests::acl_inherits_nearest_ancestor_not_root` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-PERM-012` | 整条路径（含根）均无配置时该级拒绝 | `ocs_collab_server::perm::tests::a_path_without_any_acl_row_denies_at_pbs`<br>`ocs_collab::permission::tests::acl_path_without_any_config_denies` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-013` | subject 为角色时按角色匹配 ACL | `ocs_collab::permission::tests::role_level_acl_applies_when_no_user_row_exists` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-PERM-014` | subject 为具体用户时按用户匹配 ACL | `ocs_collab_server::perm::tests::a_user_allow_row_wins_over_a_role_deny_row_on_the_same_node`<br>`ocs_collab::permission::tests::user_level_acl_wins_over_role_level_on_the_same_node` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-015` | 响应返回生效节点路径与继承标记 | `ocs_collab::permission::tests::acl_path_is_complete_when_the_target_node_decides`<br>`ocs_collab::permission::tests::acl_inherits_nearest_ancestor_not_root`<br>`ocs_collab_server::perm::tests::an_inherited_acl_reports_the_ancestor_and_a_root_first_path` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-PERM-016` | 只读状态下全部写操作被状态级拒绝 | `ocs_collab_server::perm::tests::a_readonly_drawing_denies_the_five_write_operations_at_state`<br>`ocs_collab::permission::tests::readonly_state_denies_all_five_write_operations` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-017` | 单人编辑状态对第二个编辑会话拒绝或降级 | `ocs_collab_server::http::tests::a_session_created_over_http_blocks_a_second_editor`<br>`ocs_collab_server::http::tests::permission_check_denies_a_second_editor_on_a_solo_drawing`<br>`ocs_collab_server::perm::tests::single_editor_denies_a_second_editors_write_with_the_holder` | — | 有 |  | `unit-collab.txt` 3 条 |
| `VAL-PERM-018` | 协同编辑状态允许多会话共享基线并放行写操作 | `ocs_collab_server::http::tests::a_coediting_drawing_takes_a_second_editing_session_and_both_may_edit`<br>`ocs_collab::session::tests::collaborative_mode_allows_two_live_sessions_on_one_drawing`<br>`ocs_collab_server::ws::tests::an_applied_op_moves_every_live_session_on_the_drawing_to_its_revision`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 | 第一条在一个测试里走完：图到第 3 版后两人各建一个会话，基线都是 3，各做一次「编辑图纸」的权限检查都放行 | `unit-collab.txt` 4 条 |
| `VAL-PERM-019` | 状态迁移到只读后，原放行的编辑操作转为状态级拒绝 | `ocs_collab_server::perm::tests::flipping_a_drawing_to_readonly_flips_the_same_request_to_a_state_denial`<br>`ocs_collab_server::http::tests::state_change_persists_a_legal_value` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-020` | 非法状态值与非法迁移被拒并返回稳定错误码 | `ocs_collab_server::http::tests::state_change_requires_configure_authorization_permission`<br>`ocs_collab_server::http::tests::state_change_with_an_illegal_value_is_rejected_without_persisting` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-021` | 三级全拒绝时返回第一级（role）的拒绝原因 | `ocs_collab_server::perm::tests::a_role_denial_short_circuits_even_when_state_would_deny_too`<br>`ocs_collab::permission::tests::role_denial_short_circuits_before_the_acl_tier` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-022` | role 允许但 pbs 拒绝时返回 pbs 级原因 | `ocs_collab_server::perm::tests::a_path_without_any_acl_row_denies_at_pbs`<br>`ocs_collab::permission::tests::acl_denial_short_circuits_before_the_state_tier` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-023` | role 与 pbs 均允许但 state 拒绝时返回 state 级原因 | `ocs_collab_server::perm::tests::role_and_acl_allow_then_the_state_tier_denies` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-PERM-024` | 三级均允许时放行且不带拒绝级别 | `ocs_collab_server::http::tests::permission_check_allows_a_permitted_write`<br>`ocs_collab_server::perm::tests::the_role_matrix_is_evaluated_for_all_40_cells` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-025` | 审计记录 9 个字段全部非空且语义正确 | `ocs_collab_server::http::tests::an_edit_leaves_an_audit_row_with_all_nine_fields_stated`<br>`ocs_collab_server::ws::tests::an_op_writes_a_complete_audit_row`<br>`ocs_collab_server::store::tests::audit_rows_round_trip_with_all_nine_fields` | — | 有 | 第一条在用户表里播种了公司，经 `GET /audit` 读回后逐字段断言，`company` 等于用户表里的值 | `unit-collab.txt` 3 条 |
| `VAL-PERM-026` | 对 /audit 的 PUT / PATCH / DELETE 均返回 405 | `ocs_collab_server::http::tests::audit_mutation_returns_405`<br>`ocs_collab_server::store::tests::audit_log_rejects_update_and_delete_at_the_database_level` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-027` | 按位号检索返回该位号最近一次修改（at DESC 排序） | `ocs_collab_server::store::tests::audit_by_tag_returns_the_newest_first`<br>`ocs_collab_server::http::tests::audit_tag_filters_the_same_as_object_ref` | — | 有 |  | `unit-collab.txt` 2 条 |
| `VAL-PERM-028` | 按用户检索只返回该用户的记录 | `ocs_collab_server::http::tests::audit_user_id_answers_with_that_users_rows_and_no_others` | — | 有 | `u-check` 是校对人员，不能编辑：他的两条是「写入被拒」的记录（`write_denied`），不是成功的修改。按用户检索各返回本人的两条，没有对方的 | `unit-collab.txt` 1 条 |
| `VAL-PERM-029` | 按时间区间检索使用闭区间边界 | `ocs_collab_server::store::tests::audit_from_and_to_are_inclusive` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-PERM-030` | change_text 含变更前后值 | `ocs_collab_server::http::tests::a_change_is_recorded_with_the_old_value_the_new_value_and_the_field`<br>`ocs_collab_server::ws::tests::an_op_writes_a_complete_audit_row` | — | 有 | 第一条测的是有旧值的情形：先写 DN50 再改 DN80，记录里旧值、新值、字段名都在 | `unit-collab.txt` 2 条 |
| `VAL-PERM-031` | 检索无匹配返回 200 与空数组，不是 404 或 500 | `ocs_collab_server::http::tests::an_audit_query_nothing_matches_answers_200_with_an_empty_list` | — | 有 |  | `unit-collab.txt` 1 条 |
| `VAL-PERM-032` | 审计的 ip / workspace / company 取自服务端会话上下文，不可由客户端自报覆盖 | `ocs_collab_server::http::tests::an_audit_row_takes_nothing_from_keys_the_client_slipped_in`<br>`ocs_collab_server::ws::tests::an_op_writes_a_complete_audit_row` | — | 有 | 帧里多出来的 `ip`、`workspace`、`company` 键都不读，第一条有断言。`workspace` 另是建会话时的正式字段，写在那里的值会被采纳，与契约字面有出入，见 5.3 | `unit-collab.txt` 2 条 |

### 4.4 语义层 `VAL-SEM`（39 条）

契约的取证工具就是单元测试（36 条）和 DWG 往返（3 条），与测试一一对应。其中 `VAL-SEM-014` 的测试默认不跑（判据是耗时），由计时的场景 `p03_tag_rule_budget` 按名字去跑。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-SEM-001` | 增量 1 的位号序列 | `pid::tag_engine::tests::tag_rule_increment_one_sequence` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-002` | 增量 5 的位号序列 | `pid::tag_engine::tests::tag_rule_increment_five_sequence` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-003` | 起始编码与增量独立生效 | `pid::tag_engine::tests::tag_rule_start_and_increment_are_independent` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-004` | 拼接字段取当前实例属性值 | `pid::tag_engine::tests::tag_rule_concat_field_uses_instance_property` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-005` | 拼接字段源属性缺失时拒绝生成且不占用序号 | `pid::tag_engine::tests::tag_rule_missing_concat_field_rejects_without_consuming_sequence` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-006` | 后缀递增生成管路位号 | `pid::tag_engine::tests::tag_rule_suffix_increment_pipe_run_sequence` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-007` | 后缀递增关闭时管路位号重复被检出 | `pid::tag_engine::tests::tag_rule_suffix_disabled_marks_duplicate` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-008` | 层级选择「当前类型依次递增」 | `pid::tag_engine::tests::tag_rule_hierarchy_self_increment` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-009` | 层级选择「父类型递增」 | `pid::tag_engine::tests::tag_rule_hierarchy_parent_increment` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-010` | 唯一校验拒绝同类型同位号 | `pid::tag_engine::tests::tag_unique_rejects_same_type_same_tag` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-011` | 唯一校验允许不同类型同位号 | `pid::tag_engine::tests::tag_unique_allows_cross_type_same_tag` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-012` | 手工位号不被规则引擎覆盖 | `pid::tag_engine::tests::tag_rule_never_overwrites_manual_tag` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-013` | 自动编码关闭时不生成位号 | `pid::tag_engine::tests::tag_rule_auto_code_disabled_leaves_tag_empty` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-014` | 2000 实例位号生成性能 | `pid::perf_budget::tests::tag_rule_bulk_2000_under_three_seconds`（默认不跑） | `p03_tag_rule_budget.py` 2 项 | 有 | 性能预算测试标了 `#[ignore]`，平时的 `cargo test --lib` 不跑。计时那一组的 `p03_tag_rule_budget`（2026-11-17 加）在脚本所在的仓库里指名跑它（`cargo test --offline --locked --lib -- --ignored --nocapture <测试名>`）：退出码 0，正好跑了这一条并且通过；再读测试打印的三轮，每轮 2000 个实例、末个位号 `P-3000`、位号互不相同，三轮耗时的中位数不超过 3000 毫秒，并且与测试自己打印的中位数相同。不起 GUI；测试程序没有编好时要先等它编出来 | `e2e/m7/logs/p03-cargo.console.txt` 1 条<br>`e2e/m7/logs/p03_tag_rule_budget.result.txt` 2 项 |
| `VAL-SEM-015` | 类型树父子关系 | `pid::type_lib::tests::type_lib_tree_ancestry_and_children` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-016` | 属性分组与组内顺序 | `pid::type_lib::tests::type_lib_property_groups_preserve_order` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-017` | 数据要素（无 UI，纯模型） | `pid::type_lib::tests::type_lib_data_element_roundtrip` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-018` | 子类型未覆盖时上溯继承父类型规则 | `pid::type_lib::tests::type_lib_child_inherits_unoverridden_rules` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-019` | 子类型显式覆盖时优先且不影响兄弟 | `pid::type_lib::tests::type_lib_child_override_wins_and_is_scoped` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-020` | 九键写入读回一致 | `pid::persist::tests::pid_author_xdata_roundtrip_all_keys` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-021` | 超过 2 KB 时转存 XRecord 并留引用键 | `pid::persist::tests::pid_author_xdata_overflow_moves_props_to_xrecord` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-022` | 坐标字段移动实体后不变（apply_transform 回归） | `pid::persist::tests::pid_author_coordinates_survive_entity_transform` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-023` | `PID_AUTHOR` 不被识别器 purge 清除 | `pid::persist::tests::pid_author_record_survives_legend_purge` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-024` | 同一 APPID 只保留一条记录 | `pid::persist::tests::pid_author_single_record_per_entity` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-025` | APPID 登记与 DWG 往返后仍可解析 | `pid::persist::tests::pid_author_app_id_registered_and_roundtrips` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-026` | 四个 key 各自写入读回 | `pid::graph_data::tests::pid_author_xrecord_four_keys_roundtrip` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-027` | 整 key 重写不残留旧内容 | `pid::graph_data::tests::pid_author_xrecord_rewrite_drops_stale_entries` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-028` | 四个 key 互不覆盖 | `pid::graph_data::tests::pid_author_xrecord_keys_are_independent` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-029` | 数据清空时删除该 key 的 XRecord | `pid::graph_data::tests::pid_author_xrecord_removed_when_empty` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-030` | 加载 JSON 后每个符号生成块定义 | `pid::symbols::tests::pid_symbols_load_generates_block_definitions` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-031` | 端口点从符号定义推导 | `pid::symbols::tests::pid_symbol_ports_derive_from_definition` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-032` | 符号与类型多对多 | `pid::symbols::tests::pid_symbol_type_many_to_many` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-033` | 图例标准项目级唯一 | `pid::symbols::tests::pid_legend_standard_locked_after_first_instance` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-034` | DWG 2018 往返位号与属性零丢失 | `pid::dwg_roundtrip::tests::pid_author_dwg2018_roundtrip_zero_loss` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-035` | 七个 DWG 版本往返丢失数为 0 | `pid::dwg_roundtrip::tests::pid_author_dwg_all_versions_zero_loss` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-036` | 管线改值下传到未覆盖的管路 | `pid::propagate::tests::pipe_line_property_propagates_to_runs` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-037` | 覆盖标记阻止后续下传，管段始终取管路当前值 | `pid::propagate::tests::run_override_blocks_inheritance_and_segment_follows_run` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-038` | 设备改位号后管口前缀重算并阻断冲突保存 | `pid::propagate::tests::equipment_tag_change_recomputes_nozzles_and_blocks_save` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-SEM-039` | 冲突保存被真实保存路径阻断且不落新文件 | `pid::propagate::tests::conflicting_save_is_blocked_on_the_real_save_path`<br>`app::automation::pid_save_gate_tests::the_save_operation_surfaces_the_pid_gate_code`<br>`app::automation::pid_save_gate_tests::the_verified_save_surfaces_the_pid_gate_code` | — | 有 |  | `unit-lib.txt` 3 条 |

### 4.5 绘图工具 `VAL-DRAW`（31 条）

契约的取证工具是 GUI 的 REST 接口加截图。逻辑由主程序的库测试覆盖；判据落在截图上的 12 条另有看图的场景 `g01`–`g04`，各起一个 GUI，在它自己截的图上量；判据是耗时的 3 条另有计时的场景 `p01_insert_timing`、`p02_bulk_tags`。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-DRAW-001` | 调色板面板可打开，且分类页签与符号库一致 | `ui::window::pid_palette::tests::tabs_come_from_the_symbol_library_categories`<br>`app::commands::pidpalette::tests::pidpalette_toggles_one_panel` | `g01_palette.py` 4 项<br>`g01_palette.py` 4 项（标签 `VAL-DRAW-003`） | 有 | 页签集合与面板开关有单测。调色板场景（2026-11-17 加）在整窗截图上按程序上报的矩形量：面板停靠在视口右边让出来的那一条里，矩形里八成以上是同一种颜色、与打开前相比九成以上的像素变了；页签条里的色块个数等于符号库的分类数，每个色块上都写着东西、写的东西两两不同；打开、关闭前后实体数不变。按每一类的 `label` 选页签，点亮的是该类所在位置的色块、列出的是该类的符号（标签 `VAL-DRAW-003` 的几项）。色块上写的是哪几个字不从图上认，见第 5.3 节 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/g01_palette.result.txt` 8 项 |
| `VAL-DRAW-002` | 调色板面板可关闭，且重复打开不叠加 | `app::commands::pidpalette::tests::pidpalette_toggles_one_panel` | `g01_palette.py` 3 项 | 有 | 开关与不叠加有单测。调色板场景：再执行一次 `PIDPALETTE`，程序不再上报面板和格子，整窗截图与打开之前那张（命令行条除外）一个像素不差；第三次执行，上报的面板恰好一个、位置与第一次打开时相同，截图与第一次打开的那张一个像素不差；实体数始终不变 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g01_palette.result.txt` 3 项 |
| `VAL-DRAW-003` | 分类页签可切换，网格内容随页签变化 | `ui::window::pid_palette::tests::a_categorys_grid_holds_exactly_its_symbols`<br>`ui::window::pid_palette::tests::the_active_tab_is_clamped_to_the_library` | `g01_palette.py` 4 项 | 有 | 格数等于该类符号数有单测。调色板场景：依次选每一类的页签，截图里点亮的色块换到该类的位置，程序上报的格子正是该类的符号，每个格子里画着图标，格子以外的网格区域什么也没画；「设备」「阀门」两张图第一个格位里画的东西不同。选页签走控制通道，不动鼠标，见第 5.3 节 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/g01_palette.result.txt` 4 项 |
| `VAL-DRAW-004` | 网格每个符号都有可见图标，无空单元格 | `ui::window::pid_palette::tests::every_symbol_has_a_drawable_bounds`<br>`app::commands::pidpalette::tests::every_palette_symbol_resolves` | `g01_palette.py` 1 项 | 有 | 每个符号都有可画的外包框有单测。调色板场景：「设备」页签下程序上报的格子数等于该类的符号数，每个格子放图标的方块里都有不是面板底色的像素，各格画的不一样 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/g01_palette.result.txt` 1 项 |
| `VAL-DRAW-005` | 点击符号进入插入模式，命令行为 `PIDINSERT <符号>` | `app::commands::pidpalette::tests::a_cell_click_starts_pidinsert_and_the_tab_survives_a_document_switch`<br>`app::commands::pidpalette::tests::the_command_names_itself_and_asks_for_a_point` | — | 有 | 单测里「点击」是直接发面板消息，不是真实鼠标 | `unit-lib.txt` 2 条 |
| `VAL-DRAW-006` | 面板状态在文档切换时保持 | `app::commands::pidpalette::tests::a_cell_click_starts_pidinsert_and_the_tab_survives_a_document_switch` | `g01_palette.py` 3 项 | 有 | 页签跨文档保持有单测。调色板场景：`POST /documents` 新建第二张图，切过去、再切回来，每次用 `state` 确认当前文档，截图里面板都开着、点亮的仍是「阀门」页签、格子仍是阀门类的符号。文档列表的字面出入见第 5.3 节 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g01_palette.result.txt` 3 项 |
| `VAL-DRAW-007` | `PIDINSERT` 插入设备符号后实体数 +1 | `app::commands::pidpalette::tests::pidinsert_commits_an_insert_carrying_its_author_record` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-008` | 新实体是 `Insert` 且指向 `PID_SYM_<category>_<id>` 块 | `app::commands::pidpalette::tests::pidinsert_commits_an_insert_carrying_its_author_record`<br>`app::commands::pidpalette::tests::the_command_carries_the_block_and_its_type` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-DRAW-009` | 插入位置与喂入坐标一致（插入点读回 + 矩阵反算） | `app::commands::pidpalette::tests::pidinsert_commits_an_insert_carrying_its_author_record` | `g02_insert.py` 3 项 | 有 | 插入点读回（偏差 ≤ 0.5）有单测。插入场景（2026-11-17 加）：`PIDINSERT vessel` 给点后读回的 `position` 与给的点相差不超过 0.5；带 `focus_handles` 截图，该符号的 `screen_pixel` 经 `pixel_to_world_matrix` 反算，与 `world_center` 相距不超过 0.5 个单位；按插入点和符号库算出的外框四边中点换成像素，那些地方画着线，框内空白处是底色（符号画在该在的地方） | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g02_insert.result.txt` 3 项 |
| `VAL-DRAW-010` | 插入后符号在截图里可见且能被 `highlight_handles` 圈出 | — | `g02_insert.py` 4 项 | 有 | 插入场景：同一个镜头不带、带 `highlight_handles` 各截一张，符号都在 `visible_entities` 里，`selected` 依次为假、真；不带高亮的那张里符号画在该在的地方；两张图的差别全在该符号的 `screen_bounds`（放宽 40 像素）以内，外框上的线换了颜色，框外一个像素不差；取消选择后再截一张，与第一张相同。高亮是哪一种颜色没有断言，只断言线的颜色变了 | `e2e/m7/logs/g02_insert.result.txt` 4 项 |
| `VAL-DRAW-011` | 插入的实体带 XDATA `PID_AUTHOR` 的 `type` 与 `tag` | `app::commands::pidpalette::tests::pidinsert_commits_an_insert_carrying_its_author_record` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-012` | `PIDINSERT` 未知符号名被拒绝且不建实体 | `app::commands::pidpalette::tests::pidinsert_refuses_an_unknown_symbol_with_a_code_and_no_entity`<br>`app::commands::pidpalette::tests::pidinsert_resolves_the_bare_id_and_refuses_an_unknown_one` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-DRAW-013` | 同一符号连续插入两次得到两个独立实体 | `app::commands::pidpalette::tests::two_consecutive_inserts_are_two_independent_entities` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-014` | 预览跟随鼠标移动 | `app::commands::pidpalette::tests::the_preview_follows_the_cursor` | `g03_preview.py` 2 项 | 有 | 预览图元随光标变化有单测。预览场景（2026-11-17 加，视口截图）：`PIDINSERT` 等待给点时先后给两个点各截一张，图上比命令前多出来的那团像素（预览）跟着点走，位移与按矩阵算的相差不超过 8 像素，实体数不变；取消命令后两处都回到命令前的样子，实体数仍不变 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g03_preview.result.txt` 2 项 |
| `VAL-DRAW-015` | `空格` 使预览旋转 90° | `app::commands::pidpalette::tests::space_rotates_the_preview_a_quarter_turn_about_its_centre`<br>`app::commands::pidpalette::tests::a_placement_commits_the_rotation_the_preview_showed` | `g03_preview.py` 1 项 | 有 | 预览旋转与落点角度有单测。预览场景：输入 `SPACE` 后预览那团像素的宽高互换（各容 3 像素）、中心不动（每轴容 8 像素），实体数不变 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/g03_preview.result.txt` 1 项 |
| `VAL-DRAW-016` | `Ctrl+空格` 切换插入点 | `app::commands::pidpalette::tests::ctrl_space_switches_the_insertion_point` | `g03_preview.py` 1 项 | 有 | 插入基准点切换有单测。预览场景：输入 `CTRL+SPACE` 后给的点没有变，预览相对它移动了不少于 3 像素，宽高不变，实体数不变 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g03_preview.result.txt` 1 项 |
| `VAL-DRAW-017` | `Esc` 取消插入模式，实体数不变 | `app::commands::pidpalette::tests::escape_cancels_without_a_placement_and_a_later_insert_still_commits`<br>`rest::tests::run_cancel_escapes_a_live_command` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-DRAW-018` | 取消后再次插入仍能正常提交 | `app::commands::pidpalette::tests::escape_cancels_without_a_placement_and_a_later_insert_still_commits` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-019` | 首个设备位号符合「前缀 + 序号」规则 | `app::commands::pidpalette::tests::consecutive_placements_increment_and_each_category_starts_at_its_own_origin` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-020` | 连续插入同类型位号递增 | `app::commands::pidpalette::tests::consecutive_placements_increment_and_each_category_starts_at_its_own_origin` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-021` | 位号唯一冲突时显示 `？` | `app::commands::pidpalette::tests::a_manual_duplicate_shows_the_conflict_mark_and_undoes`<br>`app::commands::pidprops::tests::a_conflicting_tag_edit_shows_the_conflict_mark_in_the_group` | `g04_tag_display.py` 4 项 | 有 | 状态、保留原位号、显示属性变 `？` 有单测。位号场景（2026-11-17 加）：把第二个符号的位号手工改成已被占用的那个，读回记录里位号保留、来源 `manual`、状态 `duplicated`，块属性的值是 `？`，属性面板「位号」行也是 `？`；截图上该符号上方位号所在的地方仍画着东西，宽度由四个多字高变成半个字高（一个字符宽），另两个符号的没有变。图上的字不认，见第 5.3 节 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/g04_tag_display.result.txt` 4 项 |
| `VAL-DRAW-022` | 位号在截图中可见（文字渲染） | `app::commands::pidpalette::tests::each_placement_shows_its_tag_above_the_symbol` | `g04_tag_display.py` 3 项 | 有 | 每个落点带一个显示位号的属性、位置在符号上方有单测。位号场景：放三个符号，带作者记录的实体正好三个；带 `focus_handles` 的截图与同一镜头下关掉属性显示的一张相比，每个符号的 `screen_bounds`（放宽 40 像素）里多出一块像素，在本体上方、对着符号中线、高约一个字高、宽四个多字高，三块互不相交。图上的字不认，见第 5.3 节 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/g04_tag_display.result.txt` 3 项 |
| `VAL-DRAW-023` | 不同类型各自独立编号 | `app::commands::pidpalette::tests::consecutive_placements_increment_and_each_category_starts_at_its_own_origin`<br>`app::commands::pidpalette::tests::a_valve_placement_records_the_valve_category_code` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-DRAW-024` | 选中带 `PID_AUTHOR` 的实体后属性面板出现 `P&ID 属性` 分组 | `app::commands::pidprops::tests::the_group_shows_the_author_record_and_only_for_authored_entities` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-025` | `P&ID 属性` 分组显示 `type`/`tag`/`tag_source`/`tag_status` 与 `props` 键值对 | `app::commands::pidprops::tests::the_group_shows_the_author_record_and_only_for_authored_entities` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-026` | 编辑属性值后提交，XDATA 读回已更新 | `app::commands::pidprops::tests::a_props_edit_lands_in_xdata_and_leaves_the_other_keys` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-027` | 编辑位号后 `tag_source` 变为 `manual` | `app::commands::pidprops::tests::a_tag_edit_is_manual_and_undoes_with_its_label` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-028` | 属性面板编辑走 undo，可回滚到旧值 | `app::commands::pidprops::tests::a_tag_edit_is_manual_and_undoes_with_its_label` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-DRAW-030` | 5000 实体画布上符号插入 ≤ 100 ms（中位数） | `app::commands::pidpalette::tests::an_automated_insert_settles_each_step_without_an_async_task` | `p01_insert_timing.py` 3 项 | 有 | 三步都同步结算的机制前提有单测。计时场景 `p01_insert_timing`（2026-11-17 加）：新建一张空图，用 `entities_create` 建 5000 条线，读回正好 5000 个实体；连做 20 次 `PIDINSERT vessel`、给点、回车，三个请求都由 `curl.exe` 发，样本是三次往返各自的 `time_total` 之和（不含程序和 curl 的启动）；20 次的每个请求都答 200 且 ok、带作者记录的实体多出 20 个；中位数不超过 100 毫秒。每个请求发出前停 0.2 秒；量的时候机器上有没有编译进程、有没有别的实例，写在检查项的说明里。这两处见第 5.3 节 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/p01_insert_timing.result.txt` 3 项 |
| `VAL-DRAW-031` | 2000 实例位号生成 ≤ 3 秒 | `app::commands::pidbulk::tests::a_batch_of_two_thousand_tags_runs_to_p_3000` | `p02_bulk_tags.py` 3 项 | 有 | 单测按契约规模跑 2000 个并核对位号，耗时只打印。计时场景 `p02_bulk_tags`（2026-11-17 加）：三次完整重复，各在一张新建的空图里用一个 `pid_insert_batch` 请求放 2000 个 `vessel`；从提交前最后一次 `/ready` 答回，到读到第 2000 个实例的非空位号，三次用时的中位数不超过 3000 毫秒（脚本的单调时钟，不含程序的启动）；停表后把每张图读回来：带作者记录的实体 2000 个，位号 2000 个互不相同，第一个 `P-1001`、第 2000 个 `P-3000`。量的时候的环境同样写在说明里 | `unit-lib.txt` 1 条<br>`e2e/m7/logs/p02_bulk_tags.result.txt` 3 项 |
| `VAL-DRAW-032` | 插入性能不受既有 P&ID 实体数量线性拖垮 | `app::commands::pidpalette::tests::an_automated_insert_settles_each_step_without_an_async_task` | `p01_insert_timing.py` 4 项 | 有 | 机制前提同 030。同一个计时场景 `p01_insert_timing`：第一组 20 次之后再插入 180 个符号（符号库的每一种轮着来），图里带作者记录的实体正好 200 个、每个分类都有、底图的线仍是 5000 条；再连做 20 次插入，20 次都成了，中位数不超过 100 毫秒，并且比第一组的中位数增长不超过 100% | `unit-lib.txt` 1 条<br>`e2e/m7/logs/p01_insert_timing.result.txt` 4 项 |

### 4.6 管线与拓扑 `VAL-PIPE`（32 条）

契约的取证工具是 GUI 的 REST 接口。判据几乎都是坐标、拓扑表和属性，库测试能逐条断言到；只有一条（`VAL-PIPE-017`）判据全在截图上，由看图的场景 `g05_pipe_break` 盯着。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-PIPE-001` | 创建管线产生独立位号 `L-1001` | `app::commands::pidpipe::tests::pidpipe_creates_a_pipe_line_tagged_l_1001`<br>`app::commands::pidpipe::tests::the_run_response_reports_how_many_entities_were_added` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-002` | 管线首条管路位号是 `L-1001-01` | `app::commands::pidpipe::tests::a_run_on_the_line_is_tagged_l_1001_01` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-003` | 同一管线第二条管路位号递增为 `L-1001-02`，且不复用已删编号 | `app::commands::pidpipe::tests::run_suffix_increments_and_deleted_numbers_are_not_reused` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-004` | 不同管线的管路位号互不干扰 | `app::commands::pidpipe::tests::runs_of_different_lines_do_not_interfere` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-005` | 管段无独立位号 | `app::commands::pidpipe::tests::a_segment_has_no_tag_of_its_own`<br>`pid::topology::tests::a_segment_record_carries_no_tag_field` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-006` | 管件继承宿主管路位号，阀门另有独立位号 | `app::commands::pidpipe::tests::a_valve_has_its_own_tag_and_an_owner_pointing_at_the_run` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-007` | 管路创建时全量继承管线五个字段 | `app::commands::pidpipe::tests::a_new_run_inherits_all_five_line_fields` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-008` | 管路手工改字段后打覆盖标记 | `app::commands::pidpipe::tests::a_manual_run_edit_marks_the_field_as_overridden`<br>`app::commands::pidprops::tests::a_run_edit_from_the_property_panel_marks_the_override_and_cascades` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-009` | 已打覆盖标记的字段不再被管线改值覆盖 | `app::commands::pidpipe::tests::an_overridden_field_stops_taking_the_line_value` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-010` | 管段始终取管路当前值（管段无覆盖标记） | `app::commands::pidpipe::tests::a_segment_follows_its_run_and_carries_no_override_mark` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-011` | 管段不直连管线——改管线只经管路下传 | `app::commands::pidpipe::tests::a_line_edit_never_reaches_a_segment_behind_an_overridden_run` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-012` | 新建管路继承的是管线**当前**值，不是创建管线时的初值 | `app::commands::pidpipe::tests::a_late_run_inherits_the_lines_current_value` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-013` | 管段上插入阀门后管段数从 1 变 2 | `app::commands::pidpipe::tests::a_fitting_at_the_midpoint_splits_the_segment_in_two` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-014` | 阀门端口为「连接点1」「连接点2」 | `app::commands::pidpipe::tests::a_fitting_at_the_midpoint_splits_the_segment_in_two`<br>`app::commands::pidpipe::tests::a_valve_has_its_own_tag_and_an_owner_pointing_at_the_run` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-015` | 两个管段各连阀门一个端口（连接关系写进拓扑表） | `app::commands::pidpipe::tests::a_fitting_at_the_midpoint_splits_the_segment_in_two` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-016` | 断开后几何端点分列阀门两侧且外侧端点不动 | `app::commands::pidpipe::tests::a_fitting_at_the_midpoint_splits_the_segment_in_two` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-017` | 截图可见断开——管线不再贯穿阀门 | — | `g05_pipe_break.py` 3 项 | 有 | 管段场景（2026-11-17 加）：管路上插闸阀后按契约的请求带标记截图，阀门在 `annotations` 里且是选中的；段 A、阀门、段 B 的 `screen_bounds` 从左到右依次排开、互不越过（各容 2 像素）；不带标记的图上沿管路那条线正好画着三段，中间两道缝是底色，缝的两头对着两个管段的端点（各容 3 像素）。几何上的不重叠另由 016 的单测保证。`annotations` 最多 64 条等出入见第 5.3 节 | `e2e/m7/logs/g05_pipe_break.result.txt` 3 项 |
| `VAL-PIPE-018` | 在管段端点插阀门不拆分，阀门绑定该端点 | `app::commands::pidpipe::tests::a_fitting_on_a_segment_endpoint_does_not_split` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-019` | 移动阀门后两侧管段端点跟随 | `app::commands::pidmove::tests::moving_the_valve_drags_the_inner_endpoints_onto_its_ports` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-020` | 补位后仍首尾相接，无重叠无缺口 | `app::commands::pidmove::tests::the_refilled_segments_stay_joined_without_overlap_or_gap` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-021` | 移动阀门不改变管段数与端口绑定 | `app::commands::pidmove::tests::moving_the_valve_keeps_the_segment_count_and_port_bindings` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-022` | 反向移动与越界移动的稳定性 | `app::commands::pidmove::tests::a_reverse_move_keeps_the_topology_intact`<br>`app::commands::pidmove::tests::an_out_of_range_move_fails_without_partial_writes` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-023` | 四种连接符各自可创建，且类型可区分 | `app::commands::pidconnector::tests::four_connector_kinds_are_created_and_distinguishable` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-024` | 同一线号同图第二个流入连接符被拒，错误码 `DUPLICATE_INFLOW_CONNECTOR` | `app::commands::pidconnector::tests::a_second_inflow_connector_on_the_same_line_is_refused` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-025` | 管道连接符选到信号线上被拒，错误码 `CONNECTOR_KIND_MISMATCH` | `app::commands::pidconnector::tests::a_connector_on_the_wrong_line_kind_is_refused` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-026` | 连接符对端 XDATA `connector_peer` 格式为 `<drawing_id>:<handle>` | `app::commands::pidconnector::tests::pairing_writes_cross_drawing_peers_on_both_sides` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-027` | 一对流入/流出连接符建立后属性可传递；信号线不含 `管道等级` | `app::commands::pidconnector::tests::pairing_transfers_pipe_properties_to_the_inflow_side`<br>`app::commands::pidconnector::tests::signal_connectors_transfer_instruments_and_never_a_pipe_grade` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-028` | `PID_AUTHOR_TOPOLOGY` XRecord 承载拓扑表且结构完整 | `app::commands::pidpipe::tests::the_topology_lives_in_the_xrecord_and_not_on_the_segment_entity`<br>`pid::topology::tests::the_table_roundtrips_through_the_xrecord` | — | 有 |  | `unit-lib.txt` 2 条 |
| `VAL-PIPE-029` | 保存重开后拓扑表读回一致 | `pid::topology::tests::topology_and_entity_xdata_survive_a_dwg_roundtrip` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-030` | 重开后实体 XDATA 语义零丢失 | `pid::topology::tests::topology_and_entity_xdata_survive_a_dwg_roundtrip` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-031` | 打断符插入管段后管段被分段且拓扑表更新 | `app::commands::pidpipe::tests::pidbreak_splits_the_segment_into_two_non_overlapping_pieces` | — | 有 |  | `unit-lib.txt` 1 条 |
| `VAL-PIPE-032` | 打断符两侧不连通 | `app::commands::pidpipe::tests::pidbreak_leaves_both_inner_ends_disconnected` | — | 有 |  | `unit-lib.txt` 1 条 |

### 4.7 桌面端协同客户端 `VAL-CLI`（20 条）

脚本起真实的服务器和桌面端，从 REST 接口、服务器的库和两边的日志取证，计时类的判据（心跳周期、启动 5 秒、60 秒无连接、断开 3 秒内发现、退避递增、占用到期）都有断言。这一区有七条的契约原文与实现有字面出入，列在第 5.3 节。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-CLI-001` | 客户端连服务器后收到 welcome 并拿到会话 | — | `t20_session.py` 5 项<br>`t20_session.py` 1 项（标签 `T2.4-existing-tags`） | 有 | 标本条编号的五项里，数服务器日志行的那一项在 2026-11-17 改了：只数以 `welcome` 作答的入站 `hello`，要求正好一条，用户和图号都对。原来数的是所有含「入站 hello」的行；加入时上报已有位号之后，服务器对同一次 `hello` 多记一行上报结果，那一项就不通过了（第 3.3 节）。上报的那一行另写了一项（标签 `T2.4-existing-tags`）：客户端的 `hello` 带着图里已有的位号，服务器在 `welcome` 之前记下上报了几个、新登记几个，回读到的登记正是这些位号 | `e2e/m6/logs/t20_session.result.txt` 6 项 |
| `VAL-CLI-002` | 客户端发送 heartbeat 的周期为 15 秒 | `app::collab::transport::tests::the_session_emits_heartbeats_carrying_its_session_id`<br>`app::collab::transport::tests::the_heartbeat_cadence_lands_inside_the_contract_window` | `t20_session.py` 3 项 | 有 |  | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t20_session.result.txt` 3 项 |
| `VAL-CLI-003` | 服务器在 90 秒无心跳后判会话失效并广播 peer_left | — | `t23_stale.py` 6 项 | 有 | 第二个观测端用的是普通 WebSocket 客户端（契约已登记的口径）。字面出入见第 5.3 节 | `e2e/m6/logs/t23_stale.result.txt` 6 项 |
| `VAL-CLI-004` | 占用申请获批后返回 claim_granted 并落库 | `app::collab::tests::a_granted_claim_publishes_this_sessions_user_id`<br>`app::collab::transport::tests::queued_claims_and_releases_reach_the_wire_with_the_session_id` | `t21_claims.py` 2 项 | 有 | 占用用的是命令 `PIDCLAIM`，面板上的按钮没有端到端点过 | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t21_claims.result.txt` 2 项 |
| `VAL-CLI-005` | 占用被拒时返回 claim_denied 并携带 holder 与 expires_at | `app::collab::tests::a_denied_claim_records_the_holder_and_never_claims_it_for_us` | `t21_claims.py` 4 项 | 有 | 字面出入见第 5.3 节 | `unit-lib.txt` 1 条<br>`e2e/m6/logs/t21_claims.result.txt` 4 项 |
| `VAL-CLI-006` | 占用在 300 秒 TTL 到期后自动释放并广播 claim_released | `app::collab::tests::a_release_clears_the_tag_and_keeps_its_reason` | `t22_ttl.py` 4 项 | 有 | 真等默认的 300 秒到期，这个场景最慢 | `unit-lib.txt` 1 条<br>`e2e/m6/logs/t22_ttl.result.txt` 4 项 |
| `VAL-CLI-007` | 主动释放占用返回 claim_released 且 reason 为主动 | — | `t21_claims.py` 2 项 | 有 | 释放用的是命令 `PIDRELEASE`，面板上的按钮没有端到端点过 | `e2e/m6/logs/t21_claims.result.txt` 2 项 |
| `VAL-CLI-008` | 占用状态在 GUI 面板可见且可被 REST 读回 | `app::collab::claims::tests::a_claimed_tag_colours_the_entities_that_carry_it`<br>`app::collab::claims::tests::this_sessions_own_claim_is_coloured_as_occupied`<br>`app::collab::claims::tests::a_released_claim_row_stops_reporting_its_rectangle` | `t21_claims.py` 3 项 | 有 | 占用色像素在视口截图上量（契约已登记的口径）。2026-11-17 起读图用脚本目录里的 `shot.py`，不再要另装图像库 | `unit-lib.txt` 3 条<br>`e2e/m6/logs/t21_claims.result.txt` 3 项 |
| `VAL-CLI-009` | 实例 A 的位号变更在实例 B 本地文档生效 | `app::collab::apply::tests::a_remote_tag_change_reaches_the_local_document`<br>`app::collab::broadcast::tests::a_set_tag_change_rewrites_the_local_tag` | `t10_broadcast.py` 6 项<br>`t10_broadcast.py` 4 项（标签 `T2.4-tag-source`）<br>`t10_broadcast.py` 6 项（标签 `T2.4-overrides`） | 有 | 标本条编号的几项是原有的。2026-11-17 另加了两组。位号来源（标签 `T2.4-tag-source`）：自动编的位号、手工改、撤销、重做，两端每次读回的位号和 `tag_source` 都相同，后三步发出的变更带着来源。属性的手改标记（标签 `T2.4-overrides`）：改管线的属性时没有标记的管路跟着变；手改管路的字段、撤销、重做，发出的变更带着标记（撤销时是把标记去掉），两端的取值和标记每次都相同；之后再改管线，带标记的管路不跟着变。字面出入见第 5.3 节 | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t10_broadcast.result.txt` 16 项 |
| `VAL-CLI-010` | 远端变更后双方 base_revision 同步刷新 | `app::collab::apply::tests::a_remote_op_moves_the_baseline_to_the_server_revision`<br>`ocs_collab_server::ws::tests::an_applied_op_moves_every_live_session_on_the_drawing_to_its_revision` | `t10_broadcast.py` 4 项 | 有 |  | `unit-lib.txt` 1 条<br>`unit-collab.txt` 1 条<br>`e2e/m6/logs/t10_broadcast.result.txt` 4 项 |
| `VAL-CLI-011` | readonly_downgrade 后 GUI 拒绝写操作 | `app::collab::apply::tests::a_readonly_session_refuses_writes_and_keeps_inspection`<br>`app::collab::apply::tests::commands_that_write_are_refused_and_the_forms_that_read_are_not`<br>`app::collab::apply::tests::every_command_a_readonly_session_may_run_leaves_the_drawing_as_it_was`<br>`app::collab::apply::tests::a_write_that_gets_past_the_command_gate_is_taken_back`<br>`ocs_collab_server::http::tests::switching_to_readonly_downgrades_everyone_but_the_holder` | `t10_broadcast.py` 5 项 | 有 | 脚本只查了契约给的那一条写命令（`PIDTAG`）；「写命令作为一类被拒」由单测盯着：两条盯放行名单，一条盯名单之外的总闸（漏过名单的写入会被收回）。字面出入见第 5.3 节 | `unit-lib.txt` 4 条<br>`unit-collab.txt` 1 条<br>`e2e/m6/logs/t10_broadcast.result.txt` 5 项 |
| `VAL-CLI-012` | 只读降级在 GUI 上可见 | `app::collab::claims::tests::a_downgrade_marks_the_holding_session_in_the_panel`<br>`ui::window::pid_collab::tests::a_downgraded_session_draws_the_badge_and_marks_the_holders_row` | `t10_broadcast.py` 4 项 | 有 |  | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t10_broadcast.result.txt` 4 项 |
| `VAL-CLI-013` | 在线用户列表随 peer_joined 增长 | `app::collab::tests::the_roster_is_the_others_and_the_head_count_includes_this_session`<br>`ocs_collab_server::ws::tests::hello_opens_a_session_and_returns_welcome` | `t10_broadcast.py` 7 项 | 有 | 字面出入见第 5.3 节 | `unit-lib.txt` 1 条<br>`unit-collab.txt` 1 条<br>`e2e/m6/logs/t10_broadcast.result.txt` 7 项 |
| `VAL-CLI-014` | 服务器重启后客户端自动重连且退避递增 | — | `t24_reconnect.py` 4 项 | 有 | 重连间隔是从客户端日志读的（服务器停着时没有服务器日志可读） | `e2e/m6/logs/t24_reconnect.result.txt` 4 项 |
| `VAL-CLI-015` | 服务器不可达时客户端进入离线态且不阻塞 UI | — | `t25_offline.py` 6 项 | 有 |  | `e2e/m6/logs/t25_offline.result.txt` 6 项 |
| `VAL-CLI-016` | 客户端优雅退出后服务器侧会话判失效且无线程泄漏 | `app::collab::transport::tests::quitting_sends_a_close_frame_and_joins_the_thread`<br>`ocs_collab_server::ws::tests::an_explicit_close_retires_the_session_and_tells_the_room_once` | `t20_session.py` 4 项 | 有 | 退出用的是命令 `QUIT`，窗口的关闭按钮没有端到端点过 | `unit-lib.txt` 1 条<br>`unit-collab.txt` 1 条<br>`e2e/m6/logs/t20_session.result.txt` 4 项 |
| `VAL-CLI-017` | 客户端以 --collab 缺省时不建立任何连接 | `app::collab::collab_offline_queue::a_client_that_never_joined_queues_nothing`<br>`ui::window::pid_collab::tests::a_panel_with_no_server_says_so_and_how_to_set_one` | `t25_offline.py` 4 项 | 有 |  | `unit-lib.txt` 2 条<br>`e2e/m6/logs/t25_offline.result.txt` 4 项 |
| `VAL-CLI-018` | 同一用户重复连入同一图纸产生两个独立会话 | — | `t11_same_user.py` 4 项 | 有 | 字面出入见第 5.3 节 | `e2e/m6/logs/t11_same_user.result.txt` 4 项 |
| `VAL-CLI-019` | 客户端协同传输不引入新的异步运行时 | `app::collab::tests::nothing_under_this_module_is_asynchronous_code` | `r01_entry_rules.py` 1 项 | 有 | 协同目录下没有异步代码由单测钉住。依赖树由 `r01_entry_rules.py` 检查：`cargo tree --offline --locked -p OpenCADStudio --edges normal` 的输出里没有 `tokio`、`async-std`，有 `tungstenite`。「冷构建不超过 20 秒」这一句没有检查：按字面不成立，CP-A 定为删掉这一句、不验收（第 5.3 节） | `unit-lib.txt` 1 条<br>`e2e/m7/logs/r01_entry_rules.result.txt` 1 项 |
| `VAL-CLI-020` | 服务器强杀后客户端按错误分支重连而非误判正常关闭 | — | `t24_reconnect.py` 4 项 | 有 |  | `e2e/m6/logs/t24_reconnect.result.txt` 4 项 |

### 4.8 冲突与裁决 `VAL-CONF`（18 条）

脚本起真实的服务器，用 WebSocket 客户端和 HTTP 请求取证，不起 GUI。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-CONF-001` | 不同字段并发修改自动合并且不产生冲突条目 | `ocs_collab::conflict::tests::a_field_nobody_else_wrote_since_the_baseline_merges`<br>`ocs_collab_server::ops::tests::a_write_behind_the_revision_to_a_field_nobody_else_wrote_merges`<br>`ocs_collab_server::ops::tests::inside_the_window_writes_to_different_fields_both_commit` | `c01_field_merge.py` 6 项 | 有 |  | `unit-collab.txt` 3 条<br>`e2e/m7/logs/c01_field_merge.result.txt` 6 项 |
| `VAL-CONF-002` | 同字段并发修改按 base_revision 判定落后方让位且无冲突条目 | `ocs_collab::conflict::tests::a_field_someone_else_wrote_since_the_baseline_is_kept_and_the_change_yields`<br>`ocs_collab_server::ops::tests::a_write_behind_the_revision_to_a_field_someone_else_wrote_yields` | `c01_field_merge.py` 5 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c01_field_merge.result.txt` 5 项 |
| `VAL-CONF-003` | 被覆盖的原值写入审计日志 | `ocs_collab_server::ops::tests::a_write_behind_the_revision_to_a_field_someone_else_wrote_yields` | `c01_field_merge.py` 4 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c01_field_merge.result.txt` 4 项 |
| `VAL-CONF-004` | base_revision 相同且值不同时生成冲突条目 | `ocs_collab::conflict::tests::two_writes_collide_only_on_one_field_of_one_object_with_different_values`<br>`ocs_collab_server::ops::tests::two_writes_against_one_revision_inside_the_window_make_a_conflict_entry` | `c01_field_merge.py` 5 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c01_field_merge.result.txt` 5 项 |
| `VAL-CONF-005` | 属性级落后方让位时不生成冲突条目 | `ocs_collab_server::ops::tests::a_write_two_revisions_behind_yields` | `c01_field_merge.py` 4 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c01_field_merge.result.txt` 4 项 |
| `VAL-CONF-006` | 对象级冲突——一方删除另一方修改，删除生效 | `ocs_collab::conflict::tests::a_change_to_an_object_someone_else_deleted_since_the_baseline_does_not_happen`<br>`ocs_collab_server::ops::tests::a_change_to_an_object_deleted_meanwhile_does_not_happen` | `c02_object_topology.py` 4 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c02_object_topology.result.txt` 4 项 |
| `VAL-CONF-007` | 对象级冲突不生成冲突条目而记入审计 | `ocs_collab_server::ops::tests::a_change_to_an_object_deleted_meanwhile_does_not_happen` | `c02_object_topology.py` 4 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c02_object_topology.result.txt` 4 项 |
| `VAL-CONF-008` | 拓扑级冲突生成待裁决条目 | `ocs_collab::conflict::tests::changes_to_one_lines_connections_contend_for_the_line`<br>`ocs_collab_server::ops::tests::two_changes_to_one_lines_connections_inside_the_window_make_a_conflict_entry` | `c02_object_topology.py` 3 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c02_object_topology.result.txt` 3 项 |
| `VAL-CONF-009` | 拓扑级冲突在裁决前不改变任何一方的拓扑 | — | `c02_object_topology.py` 2 项 | 有 |  | `e2e/m7/logs/c02_object_topology.result.txt` 2 项 |
| `VAL-CONF-010` | 管理员裁决 resolution=采用甲方 | `ocs_collab_server::http::tests::a_ruling_writes_the_value_it_picks_and_is_on_record` | `c03_resolution.py` 4 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c03_resolution.result.txt` 4 项 |
| `VAL-CONF-011` | 管理员裁决 resolution=采用乙方 | `ocs_collab_server::http::tests::a_ruling_writes_the_value_it_picks_and_is_on_record`<br>`ocs_collab_server::http::tests::a_ruling_on_a_topology_entry_applies_the_change_it_picks` | `c03_resolution.py` 3 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c03_resolution.result.txt` 3 项 |
| `VAL-CONF-012` | 管理员裁决 resolution=人工改值 | `ocs_collab_server::http::tests::a_ruling_writes_the_value_it_picks_and_is_on_record`<br>`ocs_collab_server::store::tests::resolving_with_a_manual_value_stores_it` | `c03_resolution.py` 3 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c03_resolution.result.txt` 3 项 |
| `VAL-CONF-013` | 裁决结果与双方原值写入审计 | `ocs_collab_server::http::tests::a_ruling_writes_the_value_it_picks_and_is_on_record` | `c03_resolution.py` 4 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c03_resolution.result.txt` 4 项 |
| `VAL-CONF-014` | 裁决后 status 变更且不可重复裁决 | `ocs_collab_server::http::tests::a_second_ruling_is_refused_and_changes_nothing`<br>`ocs_collab_server::store::tests::conflicts_are_created_pending_and_resolved_once` | `c03_resolution.py` 2 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c03_resolution.result.txt` 2 项 |
| `VAL-CONF-015` | 非管理员角色裁决被拒绝 | `ocs_collab_server::http::tests::a_ruling_by_someone_who_may_not_configure_is_refused_at_the_role_tier` | `c03_resolution.py` 2 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c03_resolution.result.txt` 2 项 |
| `VAL-CONF-016` | 冲突条目可按图纸与状态检索 | `ocs_collab_server::http::tests::conflicts_are_listed_by_drawing_and_status` | `c03_resolution.py` 2 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/c03_resolution.result.txt` 2 项 |
| `VAL-CONF-017` | 删除被引用对象被拒绝 | `ocs_collab::conflict::tests::an_object_at_the_end_of_a_pipe_run_cannot_be_deleted`<br>`ocs_collab_server::ops::tests::deleting_an_object_a_pipe_run_ends_on_is_refused` | `c02_object_topology.py` 4 项 | 有 |  | `unit-collab.txt` 2 条<br>`e2e/m7/logs/c02_object_topology.result.txt` 4 项 |
| `VAL-CONF-018` | 审计日志改删请求返回 405 | `ocs_collab_server::http::tests::audit_seq_requests_return_405_and_leave_the_log_untouched`<br>`ocs_collab_server::http::tests::audit_mutation_returns_405` | — | 有 | 没有脚本；`audit_seq_requests_return_405_and_leave_the_log_untouched` 这个单测起真实端口发请求。mission 记录里这一条已是通过 | `unit-collab.txt` 2 条 |

### 4.9 跨区流程 `VAL-CROSS`（12 条）

脚本起真实的服务器和两三个桌面端。`003`、`008`、`009` 三条在 mission 交回时还没有做，后来补做（管线拓扑进入协同、离线排队与重连对账、连接符跨图），脚本是 `y03`、`y08`、`y09`，已经在 2026-11-12 那次全量执行里通过。任务清单里「`VAL-CROSS-003`、`008` 没有脚本」的说法已经过时。

| 编号 | 契约标题 | 单元测试 | 端到端脚本 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|---|
| `VAL-CROSS-001` | 设计师从打开到保存的完整协同旅程零丢失 | `app::collab::apply::tests::placing_a_symbol_and_setting_its_property_are_said_to_the_room` | `x01_journey.py` 11 项 | 有 | 图是种子图的副本；「重新打开该文件」在同一个实例里做（关掉文档再打开）。脚本早于大属性对象和「改位号带真实来源」（见第 5.2 节） | `unit-lib.txt` 1 条<br>`e2e/m7/logs/x01_journey.result.txt` 11 项 |
| `VAL-CROSS-002` | 占用排他性贯穿 GUI 与服务器 | `app::collab::apply::tests::a_write_to_a_tag_somebody_else_holds_is_taken_back`<br>`ocs_collab_server::http::tests::an_op_on_a_tag_another_session_holds_is_refused_until_it_is_released` | `x02_claim_exclusive.py` 16 项 | 有 |  | `unit-lib.txt` 1 条<br>`unit-collab.txt` 1 条<br>`e2e/m7/logs/x02_claim_exclusive.result.txt` 16 项 |
| `VAL-CROSS-003` | 管线插阀门后双方拓扑一致 | `app::collab::collab_pipe_ops::a_peer_follows_a_line_a_run_and_a_valve`<br>`ocs_collab::conflict::tests::collab_pipe_ops_an_account_of_a_run_is_what_the_run_is_now`<br>`ocs_collab_server::http::tests::collab_pipe_ops_a_held_line_refuses_changes_to_its_connections` | `y03_pipe_topology.py` 17 项 | 有 | 契约写「两条 `op_applied`」，实现是三条（管线、管路、管件各一条）。同一个脚本还查了删除（标签 `CT-13-delete`，5 项） | `unit-lib.txt` 1 条<br>`unit-collab.txt` 2 条<br>`e2e/m7/logs/y03_pipe_topology.result.txt` 17 项 |
| `VAL-CROSS-004` | 无编辑权限角色被降级、写被拒且审计留痕 | `ocs_collab_server::http::tests::a_role_that_does_not_edit_joins_as_a_viewer_and_is_told` | `x04_readonly_role.py` 9 项 | 有 |  | `unit-collab.txt` 1 条<br>`e2e/m7/logs/x04_readonly_role.result.txt` 9 项 |
| `VAL-CROSS-005` | M3 持久化数据穿越 M6 协同会话后完整 | — | `x05_m3_persistence.py` 11 项<br>`x05_m3_persistence.py` 9 项（标签 `CC-2-large-props`）<br>`x05_m3_persistence.py` 2 项（标签 `CC-1-large-props`） | 有 | 基线图由仓库里一条默认不跑的测试写出（`write_the_m3_baseline_drawing`）；「插入阀门」是独立放置的阀门，不是插在管路上的管件。2026-11-17 加了大属性对象的两组（实体自己的记录过了 2 KB、属性另存进扩展字典里一条 XRecord 的符号；脚本的输出里叫「溢出记录」）。标签 `CC-2-large-props`：逐项写到溢出、一步写到溢出、改一项、撤销、重做、隔着对方的改动撤销，两端每次逐项相同，对方写的属性一项不少。标签 `CC-1-large-props`：撤销删除时放回去的对象带着全部属性；接收的一方保存的文件不连服务器重开，属性都在。删除和撤销删除用的是会话里新放的那个阀门：基线图里原有的阀 V-1001（M3 时期写的，拓扑角色是 `valve`，现在的命令不再写这种角色）在会话里被删时没有变更发给房间，另一方的图上还在，两端都不报分叉——试跑时看到的，没有改、没有断言 | `e2e/m7/logs/x05_m3_persistence.result.txt` 22 项 |
| `VAL-CROSS-006` | 首次访问流程从空列表到两人 | `app::collab::claims::tests::naming_the_server_saves_it_and_starts_the_session`<br>`ui::window::pid_collab::tests::the_roster_shows_this_session_and_says_when_nobody_else_is_there` | `x06_first_visit.py` 17 项 | 有 | 「在设置中」配服务器地址是用命令做的 | `unit-lib.txt` 2 条<br>`e2e/m7/logs/x06_first_visit.result.txt` 17 项 |
| `VAL-CROSS-007` | 冲突生成到裁决的跨区域闭环 | `app::collab::apply::tests::a_ruling_reaches_the_drawing_and_takes_the_entry_off_the_panel`<br>`app::collab::tests::a_conflict_is_kept_until_its_ruling_arrives`<br>`ui::window::pid_collab::tests::a_conflict_awaiting_a_ruling_is_listed_with_what_is_contended` | `x07_conflict_closure.py` 14 项 | 有 |  | `unit-lib.txt` 3 条<br>`e2e/m7/logs/x07_conflict_closure.result.txt` 14 项 |
| `VAL-CROSS-008` | 断线期间离线编辑，重连后与服务器对账 | `app::collab::collab_offline_queue::steps_made_with_the_session_away_wait_and_go_out_in_order_one_at_a_time`<br>`app::collab::collab_offline_queue::what_the_room_did_meanwhile_arrives_before_anything_is_sent`<br>`app::collab::collab_offline_queue::a_step_the_server_refuses_is_taken_back` | `y08_offline_reconcile.py` 10 项 | 有 | 脚本里两边各离线改一个字段（设计压力、设计温度），契约写的是一边改公称直径。同一个脚本还查了被拒的修改撤回（标签 `CT-14-rejected`，18 项） | `unit-lib.txt` 3 条<br>`e2e/m7/logs/y08_offline_reconcile.result.txt` 10 项 |
| `VAL-CROSS-009` | 页间连接符跨图协同与属性传递 | `app::collab::collab_connector_cross_drawing::two_drawings_are_paired_by_the_server_and_the_lines_field_arrives_on_the_other`<br>`ocs_collab::link::tests::collab_connector_cross_drawing_an_outflow_and_the_inflow_it_names_are_paired`<br>`ocs_collab_server::ops::tests::collab_connector_cross_drawing_the_server_pairs_an_outflow_with_the_inflow_on_the_drawing_it_names` | `y09_connector_cross_drawing.py` 18 项 | 有 |  | `unit-lib.txt` 1 条<br>`unit-collab.txt` 2 条<br>`e2e/m7/logs/y09_connector_cross_drawing.result.txt` 18 项 |
| `VAL-CROSS-010` | 双实例并发插入的位号唯一性由服务器保证 | `app::collab::apply::tests::a_symbol_the_server_renumbered_is_renamed_where_it_was_placed`<br>`app::collab::collab_existing_tags::a_peers_symbol_placed_under_a_listed_tag_is_given_another`<br>`ocs_collab::conflict::tests::a_symbol_placed_under_a_taken_tag_is_given_the_next_free_one`<br>`ocs_collab_server::http::tests::two_symbols_placed_under_one_tag_end_up_with_two_tags` | `x10_concurrent_insert.py` 11 项<br>`x10_concurrent_insert.py` 7 项（标签 `T2.4-existing-tags`）<br>`x10_concurrent_insert.py` 3 项（标签 `T2.4-claim-after-rejoin`） | 有 | 标本条编号的几项是原有的（两份相同的图上并发放置）。2026-11-17 加了两份图本来就不一样的情形。标签 `T2.4-existing-tags`：加入时的 `hello` 带着图里已有的位号，服务器登记，后到的不重复登记；启动时没有图的会话打开图之后重新加入去上报；别人新放的符号在本地算出的位号正好是第三份图里已有的那个，服务器不让它生效、改派下一个，三方读回没有重号。标签 `T2.4-claim-after-rejoin`：为上报而重新加入之前申请的占用，重新加入后还在、记在新会话名下、释放得掉（这一组盯的缺陷写在 `SPEC-collab-client.md` §10.3） | `unit-lib.txt` 2 条<br>`unit-collab.txt` 2 条<br>`e2e/m7/logs/x10_concurrent_insert.result.txt` 21 项 |
| `VAL-CROSS-011` | 只读会话的截图与审计交叉验证 | — | `x04_readonly_role.py` 11 项 | 有 | 只读标识和属性面板的行是从程序上报的界面状态读的，截图本身只查了写出来 | `e2e/m7/logs/x04_readonly_role.result.txt` 11 项 |
| `VAL-CROSS-012` | 协同会话结束后本地文件可独立打开且无协同残留 | `ui::window::pid_collab::tests::a_panel_with_no_server_says_so_and_how_to_set_one` | `x12_after_session.py` 15 项 | 有 |  | `unit-lib.txt` 1 条<br>`e2e/m7/logs/x12_after_session.result.txt` 15 项 |

### 4.10 离线使用与上线后同步 `OS`（21 条）

离线规格（`docs/plans/2026-11-16-pid-dev-spec/SPEC-collab-offline-sync.md`）§7 的 21 条成功标准。它们不是验收契约，不计入第 2 节的合计。场景是第八组的 `o01`–`o08`（第 3.2 节的「离线那一组」），检查项的标签就是 OS 编号，前提标 `setup`；同一个场景里同一个编号的几项，按出现的次序记作 a、b、c（离线规格 §11.15、§11.17 的对照运行也这样叫）。「N 项」取自 2026-11-20 `OFF-13` 的那一轮 `run19.ps1 -OnlyOffline`，八个场景都没有失败的项。「覆盖」的取值同第 1 节；OS-20 不在这一组验，记「—」；OS-21 由 `CC-7` 的全量复验取证（第 3.3 节）。单元测试只在它替代或补足端到端检查的地方写出（OS-16、OS-18、OS-19），各任务另加的单元测试见离线规格 §11 的各节。

| 编号 | 成功标准 | 场景与检查项 | 覆盖 | 说明 | 复验证据 |
|---|---|---|---|---|---|
| `OS-01` | 没有服务器：带同步记录的图打开、绘制、保存、重开，三条记录都在，失败步骤数为 0 | `o07_no_server_dxf.py` 4 项 | 有 | 图里的三条记录是场景先连着服务器造出来的：同步记录、排队的一步、被拒并撤回的一条清单。服务器不在时带协同参数打开，画直线、画圆、放阀门、改属性、保存，失败的步骤 0（01a）；退出重开，画的都在（01b）；不连协同的实例读文件，同步记录和清单与打开前相同，队列是原来那条加新排的两条（01c）；不连协同的实例打开、画、保存，三条记录不变（01d） | `e2e/m7/logs/o07_no_server_dxf.result.txt` 4 项 |
| `OS-02` | 三条记录经 DWG 和 DXF 存盘再读回，内容不变 | `o07_no_server_dxf.py` 2 项 | 有 | 带协同参数的实例经控制接口另存 DWG 和 DXF，不连协同的实例分别读回，三条记录与原图逐项相同（02a）；带协同参数打开那份 DXF，队列和清单的条数相同、按离线对待（02b）。七个版本的 DWG 和 DXF 另有往返测试（`dwg_roundtrip.rs`，离线规格 §11.10） | `e2e/m7/logs/o07_no_server_dxf.result.txt` 2 项 |
| `OS-03` | 跨重启：断线、改 n 步、保存、退出、重开、连上，补齐别人的修改，n 步按原次序提交，两端相同 | `o01_restart_resubmit.py` 4 项 | 有 | n 是 3，其中同一个字段改两次（次序一反结果就不同）。房间在这期间往前走一步；重开后 `hello` 带 `since` 和库标识，补发的先写进图、再发自己的 | `e2e/m7/logs/o01_restart_resubmit.result.txt` 4 项 |
| `OS-04` | 开程序时没有服务器：图里有记录就排队；服务器起来后提交，两端相同 | `o01_restart_resubmit.py` 2 项 | 有 |  | `e2e/m7/logs/o01_restart_resubmit.result.txt` 2 项 |
| `OS-05` | 连续两次离线运行：两次排的修改编号互不相同，全部提交，没有一条被吞掉 | `o02_ids_and_duplicates.py` 2 项 | 有 |  | `e2e/m7/logs/o02_ids_and_duplicates.result.txt` 2 项 |
| `OS-06` | 两个人同时离线各改各的，先后回来：各自的修改都到对方图上，两端相同 | `o02_ids_and_duplicates.py` 2 项 | 有 |  | `e2e/m7/logs/o02_ids_and_duplicates.result.txt` 2 项 |
| `OS-07` | 同一份带队列的图在两台机器上先后打开：每条修改在房间里只生效一次 | `o02_ids_and_duplicates.py` 2 项 | 有 | 「第二台机器」是同一台机器上的第二个实例（自己的 APPDATA，用户仍是 u1）。副本在补齐时自己认出了重号、一条没发，服务器认编号这一层由第二项测：一个 WS 会话把日志里已有的编号再发一次，只回当初那条 `op_applied`、不广播 | `e2e/m7/logs/o02_ids_and_duplicates.result.txt` 2 项 |
| `OS-08` | 三种对不上（库标识不符、本地修订领先、日志不够）：显示原因和「需要取得最新图纸」；一步都没有发出去；房间的修改没有写进这份图；保存后图里的库标识和修订不变 | `o03_align_refused.py` 9 项 | 有 | 三种（`REPO_MISMATCH`、`REVISION_AHEAD`、`REPLAY_GAP`）各 3 项：说了什么；发没发、写没写；保存后读回 | `e2e/m7/logs/o03_align_refused.result.txt` 9 项 |
| `OS-09` | 没对上的会话硬发 `op`：服务器拒绝，码是 `NOT_ALIGNED`，房间里没有这一步 | `o03_align_refused.py` 2 项 | 有 | 硬发的是一个 WS 会话：桌面端对不上时不发 | `e2e/m7/logs/o03_align_refused.result.txt` 2 项 |
| `OS-10` | 标了不一致的图保存、重开、加入：仍按不一致处理，不上报位号，不发队列 | `o04_diverged_sticks.py` 4 项 | 有 | 补齐时写不进而标的（`REMOTE_UNAPPLIED`）保存、重开；带着队列补齐时写不进的（2026-11-20 修了那处缺陷后加的，离线规格 §11.16）；带着队列、又在不连协同时改过的（`UNQUEUED_EDITS`）打开、再重开。重开时的 `hello` 里没有库标识、`since` 和已有位号 | `e2e/m7/logs/o04_diverged_sticks.result.txt` 4 项 |
| `OS-11` | 带记录的图在没连协同的运行里做了语义修改并保存：下次加入按不一致处理并说明原因；只动位置的修改不触发 | `o04_diverged_sticks.py` 2 项 | 有 |  | `e2e/m7/logs/o04_diverged_sticks.result.txt` 2 项 |
| `OS-12` | 明确给的图号与图里记录的不同：不在这张图上起会话，图里的记录没有被改写；没给图号时取图里记录的；`PIDCOLLAB DETACH` 并保存之后三条记录都没有了，加入时按没同步过的图对待 | `o04_diverged_sticks.py` 3 项 | 有 |  | `e2e/m7/logs/o04_diverged_sticks.result.txt` 3 项 |
| `OS-13` | 当前用户不是排队的人：不代发，按对不上处理 | `o04_diverged_sticks.py` 2 项 | 有 | 对不上的码是客户端自己的 `QUEUED_BY_OTHERS` | `e2e/m7/logs/o04_diverged_sticks.result.txt` 2 项 |
| `OS-14` | 重开后被拒并撤回，四种原因各一例（占用、没有写权限、删除仍被引用的对象、别人动过同一条线）：被拒的那一步从本图撤回，后面无关的照常提交，两端相同，拒绝记录和未同步清单各有一条 | `o05_refused_rollback.py` 8 项 | 有 | 一份图里排三种（`CLAIM_HELD`、`OBJECT_REFERENCED`、`STALE_BASE_REVISION`）加一步无关的：码和撤回的说法、三步在本图撤回、无关的提交、两端相同、记录与清单的编号对上（14a – e）。没有写权限另用一份图，图纸状态改成只读之后重开（`PERM_DENIED_STATE`，14f – h）：写权限按会话判，这份图里没有能提交的一步，「后面无关的照常提交」在前一份里验。被拒的是从文件里恢复的步骤，按反向修改撤回，拒绝记录里说「已按反向修改撤回」。被引用的对象是一个 WS 会话先加一条带 `from` 的管路造出来的：服务器只按管路的 `from`、`to` 认引用，桌面端画管路时不带这两个字段 | `e2e/m7/logs/o05_refused_rollback.result.txt` 8 项 |
| `OS-15` | 重开后让位（别人改了同一个字段、别人删了对象）：本图是房间的样子，不标不一致 | `o05_refused_rollback.py` 3 项 | 有 | 两步让位（`OP_SUPERSEDED`、`OBJECT_DELETED`），同一次重连里无关的一步照常提交 | `e2e/m7/logs/o05_refused_rollback.result.txt` 3 项 |
| `OS-16` | 客户端会发的十种变更逐种：做一步、按反向修改撤回，本图的位号、属性、拓扑记录与做这一步之前相同 | `app::collab::collab_inverse::a_property_changed`<br>`app::collab::collab_inverse::a_retag`<br>`app::collab::collab_inverse::a_symbol_placed`<br>`app::collab::collab_inverse::a_symbol_erased`<br>`app::collab::collab_inverse::a_connector_made`<br>`app::collab::collab_inverse::a_connector_erased`<br>`app::collab::collab_inverse::a_pipe_line_drawn`<br>`app::collab::collab_inverse::a_run_drawn`<br>`app::collab::collab_inverse::a_fitting_put_on_a_run`<br>`app::collab::collab_inverse::a_run_changed`<br>`app::collab::collab_inverse::a_pipe_line_erased`<br>`app::collab::collab_inverse::a_run_erased_with_its_segments`<br>`app::collab::collab_inverse::a_tagged_fitting_erased`<br>`app::collab::collab_inverse::a_line_property_carried_to_its_run`<br>`app::collab::collab_inverse::a_step_taken_back`<br>`app::collab::collab_inverse::one_of_many_properties_changed`<br>`app::collab::collab_inverse::a_symbol_with_many_properties_erased` | 有 | 只靠单元测试，不起进程（离线规格 §8 定的）。前 13 条是十种变更逐种（删除分符号、管线、管段、带位号的阀门），后四条是改管线属性带着管段、撤回的一步、属性超过 2 KB 的两例。撤不回的 0 种；管段号不退是记下的差异（离线规格 §11.10） | `unit-lib.txt` 17 条 |
| `OS-17` | 未同步清单列出队列里的、被拒的、没排上队的；导出的文件条数对得上；清单随图保存 | `o06_unsynced_exit.py` 4 项 | 有 | 只有排队的两步时，`PIDCOLLAB UNSYNCED` 的第一行是「排队 2 条，被拒 0 条，没排上队 0 条」（17a）；三种各一条时列出三行（17b），导出的文件按状态各 1 条，与状态接口的数对得上（17c）；保存后不连协同的实例读出的队列和清单与导出的同号（17d）。没排上队的那条是不连协同的实例在副本上改出来的 | `e2e/m7/logs/o06_unsynced_exit.result.txt` 4 项 |
| `OS-18` | 离线时面板显示「离线，n 步待同步」，提交完数字归零；队列不空时退出提示一次 | `o06_unsynced_exit.py` 6 项<br>`ui::window::pid_collab::tests::the_sync_line_and_the_prompt_read_as_the_spec_has_them`<br>`ui::window::pid_collab::tests::the_sync_line_is_drawn_under_the_banner_while_steps_wait`<br>`app::collab::collab_exit_prompt::pid_collab::offline_the_line_says_how_many_steps_wait`<br>`app::collab::collab_exit_prompt::quitting_with_steps_queued_asks_and_cancel_stays` | 有 | 场景读不到面板上的字：读的是这一行在不在和 `connected`、`queued_ops` 两个数，另留截图；字由单元测试按同样的两个数核对（第 3.2 节的「离线那一组」）。在线、队列空时没有这一行（18a），离线两步时有（18b）；服务器回来后被拒的一步撤回、另一步提交，队列为 0，这一行没了（18e，归零就是这一行消失，`OFF-10` 的做法）。退出：队列两步时 `QUIT` 问一次，答「取消」留下、队列不变（18c），再 `QUIT` 再问、答「退出」就退出（18d）；队列空时不问（18f） | `unit-lib.txt` 4 条<br>`e2e/m7/logs/o06_unsynced_exit.result.txt` 6 项 |
| `OS-19` | 服务器的库打不开：不接受会话；用 `--new-repo-id` 启动后，旧图纸得到 `REPO_MISMATCH` | `o08_server_store.py` 6 项<br>`ocs_collab_server[main.rs]::tests::store_unavailable_a_database_that_cannot_be_opened_takes_no_session`<br>`ocs_collab_server[main.rs]::tests::new_repo_id_at_startup_changes_the_id_once_and_it_stays`<br>`ocs_collab_server::ws::tests::store_unavailable_a_hello_is_refused_and_the_connection_closed`<br>`ocs_collab_server::ws::tests::new_repo_id_a_drawing_that_remembers_the_old_id_gets_repo_mismatch` | 有 | 库打不开是用 `--db` 指向一个目录造的：进程在跑，`/health` 和 `POST /session` 是 503（19a）；`hello` 得到 `STORE_UNAVAILABLE`，接着连接关闭（19b）；桌面端没有会话、一步不发（19c）。带 `--new-repo-id` 启动：输出新标识（19d）；旧图纸对不上（`REPO_MISMATCH`）、一步不发、队列还在（19e）；再不带参数启动，标识还是新的（19f） | `unit-collab.txt` 4 条<br>`e2e/m7/logs/o08_server_store.result.txt` 6 项 |
| `OS-20` | 发布：队列不空、不一致、没对账三种情形各被拦，码对得上；放行时清单写 `reconciled: false` | 在 `PUB-4` 验 | — | `PUB-4` 验收的最后一条（「协同前提」）；这一组不跑 | —（在 `PUB-4` 验，这一轮不跑） |
| `OS-21` | 原有 228 条契约的场景全部照过 | `run19.ps1` 全量（八组加 `r01_entry_rules`） | 有 | `CC-7` 的全量复验（第 3.3 节 2026-11-20 那一遍）：原有的 38 个场景连同这八个场景一起跑，全部通过 | `e2e/summary-cc7-20261120-210132.txt`：八组 46 个场景加 `r01_entry_rules`，830 项全过 |

## 5. 缺口

### 5.1 没有覆盖或只覆盖一部分的（0 条）

对照表刚做出来时有 39 条，按要做的事分五类。2026-11-17 都补完了，每一类留一段话，写补了什么。

**（一）判据在截图上，要起 GUI 的场景（原 15 条，2026-11-17 补完）**

`VAL-DRAW-001`–`004`、`006`、`009`、`010`、`014`–`016`、`021`、`022`，`VAL-PIPE-017`，`VAL-INFRA-009`、`010`。新写了五个看图的场景：`g01_palette`（调色板的开合、页签、图标、跨文档）、`g02_insert`（插入的位置和高亮）、`g03_preview`（放置时的预览、转向、换基准点）、`g04_tag_display`（位号的显示和重号时的 `？`）、`g05_pipe_break`（阀门把管路断开）；冒烟场景加了 3 项。都挂在总入口里（看图的五个是第六组），对照表里已改成「有」。

判的是程序自己截的图上的像素，量在哪里由图纸和应答里的矩阵算出来（做法见第 3.2 节的「看图那一组」）。图上的字不认：契约的判据要靠认字的，换成了量位置、量大小，加上读记录里的值；这几处和别的字面出入列在第 5.3 节。各项检查试过会不会红，做法和结果见第 5.4 节。

**（二）判据是耗时（原 4 条，2026-11-17 补完）**

`VAL-DRAW-030`、`031`、`032` 和 `VAL-SEM-014`。新写了三个计时的场景：`p01_insert_timing`（一次放置要多久，图里有了 200 个符号之后慢了多少）、`p02_bulk_tags`（一个批量请求放 2000 个带位号的实例要多久）、`p03_tag_rule_budget`（去跑标签规则那条默认不跑的性能测试，核对它量到的数）。都挂在总入口里（第七组），对照表里已改成「有」。

判的是耗时：2026-11-17 第五遍全量里，在 5000 条线的图上放一个符号（三个请求）20 次的中位数是 55.9 毫秒，图里再有 200 个符号之后是 37.0 毫秒（上限都是 100 毫秒，后一组比前一组的增长不许超过 100%）；一个批量请求放 2000 个带位号的实例，三次的中位数 478.3 毫秒（上限 3000）；标签规则那条测试三轮的中位数 15 毫秒（上限 3000）。其中 55.9 和 478.3 这两个数是在机器上有一个别人起的编译进程的时候量的；它停了以后单独再量的一遍，四个数依次是 38.9、38.6、449.9、15 毫秒（第 3.3 节）。换一台机器、或者机器正忙，读数会变；怎么计时见第 3.2 节的「计时那一组」。做法与契约字面不一样的地方列在第 5.3 节。各项检查试过会不会红，做法和结果见第 5.4 节。

**（三）要真实的服务器进程或套接字（原 6 条，2026-11-17 补完）**

`VAL-SRV-024`–`028`、`030` 补了四个不起 GUI 的场景：`s01_restart`（重启）、`s02_port_in_use`（端口被占）、`s03_disconnect`（断线）、`s04_concurrent_ops`（并发提交），挂在总入口里（现在是第五组），对照表里已改成「有」。各项检查试过会不会红，做法和结果见第 5.4 节。做并发场景时发现服务器在更大的规模下会丢广播，列在第 5.3 节。

**（四）补或改一条单元测试就够（原 7 条，2026-11-17 补完）**

`VAL-INFRA-005`、`VAL-PERM-018`、`025`、`028`、`030`、`031`、`032` 各补了一条单元测试，对照表里已改成「有」，测试名见第 4.2、4.3 节。每一条都试过会不会红：把它盯着的那处代码临时改坏，测试在对应的断言上失败，改回去才通过。`VAL-PERM-032` 与契约字面有一处出入，列在第 5.3 节。

**（五）验收规程与脚手架（原 7 条，2026-11-17 补完）**

`VAL-INFRA-001`、`007`、`008`、`012`、`013`、`014` 和 `VAL-CLI-019`。动手前先裁决了三处（2026-11-17）：「60 秒内就绪」只在冒烟场景里断言，其余场景等就绪的上限仍是 180 秒，超时后结束进程、记下日志、换一个端口重试一次，契约的字面不改；「实例消失后重试一次」做进总入口，两次的结果都留；冷构建加一步依赖检查，「不超过 20 秒」那一句留到 CP-A 裁决。

做法：规则写进脚手架和入口（第 3.2 节的规程表）；新写四个脚本去试（`t02_harness_rules`、`t03_vanish_retry`、`t04_entry_rerun`、`r01_entry_rules`），冒烟场景和 `t20_session` 各加了检查项；都挂在总入口里。对照表里已改成「有」。各项检查试过会不会红，做法和结果见第 5.4 节；做法与契约字面不一样的地方列在第 5.3 节。`VAL-CLI-019` 里「冷构建不超过 20 秒」这一句没有检查。

### 5.2 脚本早于后来的改动

2026-11-12 那次全量执行之后合进来的改动，单元测试都有，脚本里原来没有对应的断言（在脚本目录里检索过关键字），相关的几条契约在端到端这一层只验到了改动之前的行为；其中第一行的改动还让一项已有的断言不再通过（第 3.3 节）。

前三行的断言 2026-11-17 补了，不再通过的那一项也改了；第三行只补了符号这一种，每一行还没有的写在最后一列。新加的检查项挂在相关契约的那几行上（`VAL-CLI-001`、`VAL-CLI-009`、`VAL-CROSS-005`、`VAL-CROSS-010`），对照表里的取值没有变；它们会不会红，逐项试过（第 5.4 节）。后两行不是改脚本能补的。

| 改动 | 提交 | 脚本的现状 | 还没有的 |
|---|---|---|---|
| 加入时上报图里已有的位号，服务器据此避开 | `1682b84c3`、`93433f7d2`、`b34bccf7e`、`ecdd8dd91` | 已补。`x10_concurrent_insert.py` 加了两组。标 `T2.4-existing-tags` 的 7 项：两个实例加入时都上报了图里已有的位号，服务器登记下来；第三个实例启动时不带图，加入之后才打开一张带位号的图，它重新加入、把位号报上去；此后另一方新放的阀门，本机算出的位号正好是那张图里已有的，服务器改派到下一个，三方读回没有重号。标 `T2.4-claim-after-rejoin` 的 3 项：第三个实例在重新加入的前一刻申请的占用，落在新会话名下，能释放。`t20_session.py` 里过时的那一项改成只数以 `welcome` 作答的那一行，另加一项（`T2.4-existing-tags`）查上报结果的那一行。写这几项时发现一处产品缺陷（重新加入之前刚发出的占用申请，之后要不回来），在提交 `adb4aed54` 里修了，`T2.4-claim-after-rejoin` 盯的就是它 | 退出程序时的离开与修掉的那一处是同样的写法，没有复现、没有改、没有断言。只读的会话先把启动时的空白图认作本会话的图，之后打开的图里的位号不上报（试跑时看到的，没有往下查，没有改、没有断言） |
| 属性变更带覆盖标记，改位号带真实来源 | `9b5327f96` | 已补。`t10_broadcast.py` 加了两组。标 `T2.4-tag-source` 的 4 项：放一个阀门（位号是自动编的）、手工改位号、撤销、重做，每一步两端读回的位号和来源都相同；后三步发出的变更带着来源（手工、自动、手工）。标 `T2.4-overrides` 的 6 项：改管线的公称直径，没有手改标记的管路跟着变；手工改管路，发出的帧带覆盖标记，两端都记下标记；撤销去掉标记，重做又标上；再改管线，标了的管路不跟着变；最后数一遍，每个动作正好一条变更，两端没有分叉 | `x01_journey.py` 没有扩。覆盖标记只试了公称直径这一个属性 |
| 大属性对象（属性多到存进扩展字典的符号、管线、管路、管件）新建、改属性、撤销、重做都带全属性 | `0744f67b2`、`e24b3f947`、`68f08721e` | 符号这一种已补。`x05_m3_persistence.py` 加了两组。标 `CC-2-large-props` 的 9 项：会话里新放的一个阀门逐项写 9 项属性（写到实体自己的记录过了 2 KB，属性另存进扩展字典），基线图里的设备 P-1001 一步写进一项 2 KB 以上的属性；改属性、撤销、重做，两方隔着对方的改动各自撤销，每一步两端读回一致，最后数一遍变更条数、两端没有分叉。标 `CC-1-large-props` 的 2 项：删掉那个阀门再撤销，重建它的帧带全 9 个属性；接收方存盘后不连服务器重新打开，9 个属性都在 | 管线、管路、管件的大属性没有端到端的断言，仍只有单元测试。`x01_journey.py` 没有扩 |
| 服务器的 `--bind`（对外监听） | `278439b5d` | 不在契约里；脚本都在本机回环地址上跑 | 两台机器的流程没有跑过，记作已知未验证；脚本做不了，要人在两台机器上试 |
| 协同分支合进 `main`、合并上游 | `af23cffef`、`2278e4fba`、`5b2c2a51f` | 合并之后跑过三遍（2026-11-17：脚本收进仓库时跑的原有 22 个场景，补完验收规程后、补了上面三行的断言后各跑的一遍五组全量；第 3.3 节），证据都没有逐条核对；2026-11-20 的 `CC-7` 全量复验（提交 `3450558e6`）八组全过，各条的证据逐条核对过（第 3.3 节、第 4 节开头） | 没有了 |

### 5.3 契约字面与实现有出入的（不算覆盖缺口，结项前要裁决）

脚本过了，但契约原文里有一句按字面不成立，或者脚本的做法与原文不完全一样。前七条来自状态文档 §13.3（当时记作「13 条成立，7 条部分成立」），其余来自 §18.2 和这次对照；`VAL-INFRA` 的四行是 2026-11-17 补验收规程的缺口时重写或新加的，写的是脚手架和入口的做法与契约字面不一样的地方（`VAL-CLI-019` 那一行也在那时改过）；`VAL-PERM-032`、`VAL-SRV-030` 两条是补缺口时发现的（前一条在补单元测试时，后一条在补并发场景时）；`VAL-INFRA-009` 起的七行是 2026-11-17 补看图的场景时加的，最后两行是同一天补计时的场景时加的，写的都是场景的做法与契约字面不一样的地方。

**2026-11-20 在 CP-A 裁决了**（用户按建议定）：写明要裁决的 6 行（`VAL-CLI-019`、`VAL-PERM-032`、`VAL-SRV-030`、`VAL-INFRA-009`、`VAL-PIPE-017`、`VAL-DRAW-030` / `032`）都维持现在的实现，定下的口径写在各行末尾「CP-A 定」之后；其余各行按记下的做法认可。契约原文在 mission 目录里，只读，没有改；这一节就是口径。要后续再做的三件事记在 `docs/plans/2026-11-16-pid-dev-spec/plan.md` §10。

| 编号 | 出入 |
|---|---|
| `VAL-CLI-003` | 原文「后加入的一方收到关于先加入者的 `peer_joined`」不成立：后加入者是从 `welcome.peers` 里知道先加入者的。脚本按 2026-11-11 定的口径断言。第二个观测端用普通 WebSocket 客户端，是契约已登记的口径 |
| `VAL-CLI-005` | `claim_denied` 帧里的 `holder` 是会话号，不是原文写的「显示名或用户号」；桌面端再把它解析成用户显示 |
| `VAL-CLI-009` | 原文「双方均确认收到对方的 `peer_joined`」不成立，与 `003` 是同一件事 |
| `VAL-CLI-011` | 当时只读会话的放行名单里混有会改图的命令，「写命令被拒」作为一类不成立。后来修了（状态文档 §15、§18.4），现在由两条单元测试盯着放行名单、一条盯着名单之外的总闸；脚本只查了契约给的那一条命令（`PIDTAG`）。记录里的「部分成立」没有改回 |
| `VAL-CLI-013` | 当时服务器把加入者自己也列进 `peers`，「首次连入为空数组」「长度为 1」两句不成立。后来修了，脚本现在按这两句的字面断言并通过；记录里的「部分成立」没有改回 |
| `VAL-CLI-018` | 原文「两个实例互相收到 `peer_joined`」只对先连入的一方成立，与 `003` 是同一件事 |
| `VAL-CLI-019` | 「冷构建不超过 20 秒」不成立，这一句没有检查，留到 CP-A 裁决（改字面，还是删掉这一句）。依赖树的检查已经有了（`r01_entry_rules.py`）。`smol` 按 2026-11-09 的裁决不计入（它是界面库自带的依赖），检查里只把它记下来。**CP-A 定**：删掉这一句，构建用时不验收；这条要管的「不引入异步运行时」由依赖树的检查盯着 |
| `VAL-CROSS-001` | 图是种子图的副本，不是契约写的那个路径；「重新打开该文件」在同一个实例里做（关掉文档再打开） |
| `VAL-CROSS-003` | 原文写「两条 `op_applied`」，实现是三条：管线、管路、管件各一条 |
| `VAL-CROSS-005` | 基线图由生成器写出，不是契约写的那个路径上的文件；「插入阀门」是独立放置的阀门，不是插在管路上的管件 |
| `VAL-CROSS-006` | 「在设置中」配服务器地址，脚本是用命令配的 |
| `VAL-CROSS-008` | 原文是一方离线改公称直径；脚本是两方各离线改一个字段（设计压力、设计温度）。判的是同一件事：离线的修改重连后提交、对方读回、不静默丢弃 |
| `VAL-CLI-004`、`007`、`016` | 契约允许「点面板或用等价命令」，脚本用的都是命令；面板上的占用、释放按钮和窗口的关闭按钮没有端到端点过 |
| `VAL-INFRA-001`、`014` | 契约写 60 秒内就绪。按 2026-11-17 的裁决，60 秒只在冒烟场景里断言，其余场景等就绪的上限仍是 180 秒；超时后的处置（结束进程、留日志、换端口重试一次）所有场景一样。契约的字面不改。另有一处要知道：冒烟场景里第一次启动到 60 秒没有就绪、照规则重试后 60 秒内就绪的，记通过（说明里写出重试）；两次都不就绪才记失败 |
| `VAL-INFRA-007` | 契约写「验证器记下现象、同端口重启同一张图、整个流程重跑一次」。脚本的做法：现象记在证据目录里，不是契约写的那个固定路径；「重跑」是入口把整个场景脚本再跑一遍；「`/ready` 连续 3 次拒绝」是在某个请求被拒绝之后才去连探 3 次，平时不轮询。`t03` 重跑时用的是第一次那张图（不重新复制，只删掉被结束的实例留在旁边的锁文件和恢复文件）；别的场景没有为重跑另写代码，重跑就是从头再跑一遍，图按各自开头的做法重新准备 |
| `VAL-INFRA-008`、`012` | 2026-11-12 那次执行用的副本端口整体加了 100（3310–3359）。收进仓库时已改回契约的端口段（见 3.2 末），不用裁决。`008` 还有一处做法上的出入：启动前查端口，契约写的是 `Get-NetTCPConnection`，脚手架用的是去连本机回环地址（连得上就是有人在监听）；只绑定还没有开始监听的、或只监听在别的地址上的端口，会被当成空闲。`012` 的监听方是用 `Get-NetTCPConnection` 查的 |
| `VAL-INFRA-013` | 契约写产物都放在一个固定的本机路径下。脚本里没有写死的路径，产物写在运行时指定的证据目录里，只要求它在仓库之外。`git status` 比的是「比开跑前多出来的行」，开跑前工作区里已有的改动不算；开跑前就已经列在改动里的文件再被改一次，这样比看不出来 |
| `VAL-PERM-032` | 原文要求建会话时塞进载荷的伪造 `workspace` 不被采纳。实现里 `workspace` 是 `hello` 和 `POST /session` 的正式字段，会话就是在那里声明自己的工作区的（`VAL-PERM-025` 的 `ws-alpha` 也是这样来的），写在那里的值原样记进审计，服务器分不出「声明」和「伪造」。成立的部分：帧里多出来的 `ip`、`company` 键，以及提交修改的帧里的 `workspace` 键，服务器都不读；`ip` 取连接的对端地址，`company` 取用户表。单元测试断言的是成立的这部分。要裁决：把原文改成「`workspace` 由会话声明，另两项不可自报」，还是让服务器按一张工作区名单校验。**CP-A 定**：口径是「`workspace` 由会话声明，`ip`、`company` 不可自报」，服务器不按名单校验；用户号本身也是自报的（离线规格 §9 的 L10），名单挡不住冒名 |
| `VAL-SRV-030` | 契约写「不少于 8 个会话、各不少于 30 个 op」。按下限（8 个会话、每批各 30 条）成立，场景就是这样跑的。规模再大就不一定：服务器给每张图的广播队列只留最近 256 条，某条连接积压超过这个数，最早的那些广播就被丢掉（服务器在 stderr 记一行 `subscriber lagged`；代码注释写明这是有意的，丢了的靠客户端重新对齐）。连着发 op 的会话转发广播慢，积压会接近整整一批。10 个会话各连发 30 条（一批 300 条）时，这台机器上 9 次里有 4 次丢了广播，其中一次有 19 条已经生效的 op、发送方自己没有收到 `op_applied`。丢的只是通知：那一次库里的图纸修订、操作日志、审计行都是全的。要裁决：维持现状、把规模上限写进契约，还是改服务器（发送方的回执单独发、不走广播队列；或转发时先把积压的发完；或加大队列）。复现：把 `s04_concurrent_ops.py` 里的 `N` 改成 10，多跑几次。**CP-A 定**：维持现状。验收的规模以「每张图 8 个会话、各 30 条」为上限，规模更大时丢广播通知记为已知限制；「发送方的回执单独发、不走广播队列」是后续任务 |
| `VAL-INFRA-009` | 契约给的请求体只有 `document_id` 和 `annotate`。程序对不带 `path` 的截图请求回 HTTP 400（`Missing path`）：存到哪里要请求方写明。冒烟场景先按契约原样发一次，把这个应答记在检查项的说明里，再带上 `path` 发，判的是后一次。要裁决：把 `path` 写进契约的请求体，还是让程序在不给时自己找地方存。**CP-A 定**：`path` 写进口径，截图请求必须带 `path`；程序不在没写明的地方存文件 |
| `VAL-DRAW-001` | 契约写「页签文本集合」与符号库的分类名逐项相同。页签上写的是哪几个字，场景不从图上认；查的是页签的个数等于分类数，每个页签上都写着东西、写的东西两两不同，以及按每一类的名字选页签时，亮起来的是那一类位置上的页签、列出的是那一类的符号。页签的字取自分类的名字，这一点是读源码知道的（`src/ui/window/pid_palette.rs`）。另：契约只写「停靠面板」，没有写停在哪一侧；场景从程序的默认设置起，查的是面板在视口右边让出来的那一条里，换过停靠位置之后的样子不管 |
| `VAL-DRAW-003` | 契约写「点击」页签。场景经控制通道选页签（`action pid_palette tab=<分类名>`），不移动指针；用鼠标点页签这一下没有端到端点过 |
| `VAL-DRAW-006` | 契约写新建文档之后文档列表「返回 2 个文档」。列表里还有起始页那一条（`start=true`），实际是 3 条；场景数的是不算起始页的图纸，由 1 张变成 2 张 |
| `VAL-DRAW-010` | 契约说带高亮与不带高亮的两张图，差别在该实体的 `screen_bounds` 之内。高亮时位号的字画得比上报的矩形略宽，场景把矩形向外放宽 40 像素再比（`VAL-DRAW-022` 的契约自己写的就是 40 像素）：差别全在放宽后的框里，框外一个像素不差。高亮用的是什么颜色不查，只查外框上的线换了颜色 |
| `VAL-DRAW-021`、`022` | 两条的判据里都有要认字的：`021` 写画出来的位号文本是 `？`，`022` 写三个位号都能辨识。场景不认字。位号是哪几个字符，读自记录、块属性和属性面板；图上量的是位号那一块像素在哪里、多宽多高：正常的位号宽四个多字高，换成 `？` 之后只有半个字高宽，三个符号的三块互不相交。「画出来的就是那几个字」只有这样间接的证据 |
| `VAL-PIPE-017` | 契约的那一个请求（`view:"extents"`、只高亮阀门、`annotate:true`）不一定列得全三样：应答里的实体表最多 64 条，选中的排在前面，其余按屏幕上的面积从大到小排；水平的管段面积是 0，基础图里的东西一多它就排不进表。场景照契约的请求只判阀门在表里并且是选中的；段 A、阀门、段 B 的先后次序读自同一个视图的第二张图，那一张把两段管路也一起高亮。另有一处摆法不同：管路所属的管线自己的那条线不会被阀门断开，画在管路的位置上会从两道缝里透出来，场景把它画在 60 个单位之外；画在同一处时图是什么样，场景结尾记了一笔，不算检查。要裁决：契约改成三样都高亮，还是让实体表不截断；管线自己的线要不要跟着断开。**CP-A 定**：维持现在的判法；实体表最多 64 条的截断记为已知限制；管线自己的线要不要随阀门断开，交给绘图那条线定（后续任务） |
| `VAL-DRAW-030`、`032` | 契约写每次放置用 `curl.exe` 记三次 HTTP 往返的总时长，没有写两个请求之间隔多久。场景在每个计时的请求之前停 0.2 秒。原因：程序答完一个请求之后还要忙几十毫秒（写场景时量到约 40 毫秒，与请求是什么、图有多大无关），紧接着发来的请求要等它忙完，量到的用时里就多出这一段。不停顿、一个接一个发的一组场景也量：第五遍全量里中位数 113.6 毫秒，单独再量的那一遍 112.6 毫秒，超过 100 毫秒；这一组只写进说明，不判。契约记的交付基线（28.7 毫秒、31.7 毫秒）是用 PowerShell 逐个起 `curl.exe` 量的，起进程本身就隔开了两个请求，只是契约没有写出来。要裁决：100 毫秒管的是「隔开发的一次放置」（现在的做法），还是「一个接一个发时的每一次」（那样现在不过，要先查程序答完之后在忙什么）。另：契约要求测之前确认没有编译在跑、没有别的实例，读数明显高于基线时先在干净的环境里重测再判；场景只把当时的情况记下来，不拦着不测，也不自己重测（第 3.2 节的「计时那一组」）。**CP-A 定**：100 毫秒管「隔开发的一次放置」（现在的做法，也是契约基线的量法）；一个接一个发的那组照样只写进说明，「答完之后在忙什么」是后续任务 |
| `VAL-DRAW-031` | 契约写用 PowerShell 的秒表计时，澄清里把批量请求写成 `{"op":"run","cmd":"pid_insert_batch",…}`。场景用 Python 的单调时钟，请求和别的场景一样经脚手架发，请求体里写的是 `"op":"pid_insert_batch"`；契约的那种写法程序不认：试过一次，答 HTTP 400「未知的命令」，图里什么也没有放；写成操作（`op`）才认。起点、终点、三遍取中位数和终态的三项校验都照契约 |

### 5.4 这张表是怎么核对的

- 228 个编号各出现一次，各区条数与状态文档第 4 节相同；mission 记录的两栏合计 179 与 49，也与那一节相同。
- 表里引用了 287 个单元测试，每个名字都在全量测试日志里唯一地找到，并且是通过的（标「默认不跑」的一条除外：它在全量测试里不跑，由 `p03_tag_rule_budget` 按名字去跑，过没过看那个场景）。日志是基准提交上的一份；补缺口加的 7 条取自 2026-11-17 补完它们之后重跑的那一次。
- 表里引用了 39 个脚本（38 个场景加收尾的整轮检查），都存在。原有 22 个场景里此后没有动过的 17 个，项数取自 2026-11-12 那次执行的结果文件；补了断言的四个（`t10_broadcast`、`t20_session`、`x05_m3_persistence`、`x10_concurrent_insert`）取自 2026-11-17 补了断言之后的那一遍五组全量（第 3.3 节）；服务器进程的四个场景取自 2026-11-17 它们经总入口跑的那一次；脚手架与入口规程里的 `t03_vanish_retry`、`t04_entry_rerun` 和收尾的整轮检查，取自 2026-11-17 补完验收规程之后的那一遍五组全量；看图的五个场景、加了检查的 `t00_smoke` 和换了读图办法的 `t21_claims`，取自 2026-11-17 补了看图场景之后的那一遍六组全量；计时的三个场景和改了等就绪上限取法的 `t02_harness_rules`，取自 2026-11-17 补了计时场景之后的那一遍七组全量。每个场景在它取数的那一次执行里，项都没有失败的；补了看图场景之后的那一遍里没过的 3 项在 `t02_harness_rules`、`t25_offline`，这两个场景的项数不取自那一遍。
- 服务器进程的四个场景，逐项试过会不会红：把服务器在某一处临时改坏、重新编出来（共 19 处，一次只改一处；改坏的可执行文件只放在仓库外，源码改完即还原），让场景对着它跑，共 21 次；另有一次不重启服务器的对照。22 次运行每一次都有检查失败。带契约编号的 43 项检查里，41 项至少在一次这样的运行里失败过；剩下 2 项没有做出让它失败的改法：`s02` 的「原来的实例照常服务、原来的连接没有收到 `error`」和 `s03` 的「重新握手成功，且落在销毁之后 3 到 30 秒之间」。
- 脚手架与入口规程的四个场景、收尾的整轮检查和 `t20_session` 里后加的那一项，也逐项试过会不会红（2026-11-17）。做了 62 次对照运行，一次只改一处，改的都是放在仓库外的副本：30 次改脚手架（把某一条规则拿掉或改错），10 次改入口的重跑规则，5 次改场景自己（重跑时换一个端口、换一张图，流程里的命令不建实体；检查之前先把服务器结束、把 GUI 冻结），1 次不改脚本、让真实例连丢两次，6 次针对收尾的整轮检查（一轮中间往仓库里放一个文件、放一份字节码缓存，跑完即删；不做开头的记录；换成另外两个包的依赖树；让两份脚手架差一行），10 次让冒烟场景对着一个改过的 GUI 跑（`/api/v1/ready` 的字段给错 7 种、在哪个端口上都要过 65 秒才就绪、只在 3210 上这样、什么都不改；改过的可执行文件只放在仓库外，源码改完即逐字节还原）。62 次的结果都与事先写下的预期一致：60 次有检查失败；另 2 次是应当全部通过的对照（改过的 GUI 什么都不改；只在 3210 上过 65 秒才就绪，第一次到上限被结束、换端口重试后 60 秒内就绪），确实全部通过。这几个脚本一遍跑下来是 36 项检查（`t00` 7、`t02` 13、`t03` 5、`t04` 5、`r01` 5，加 `t20` 的 1 项），其中 29 项带契约编号：这 29 项都至少在一次对照里失败过；没有失败过的 5 项都是不带契约编号的前提项（实例就绪三项、替身场景跑完、截图接口回 200）。另有一项只在失败时才会出现的检查（`t03` 的「实例已经被结束，脚手架却没有发现」），没有一次对照走到它：把脚手架的探测去掉之后，请求直接报连接错误，场景记的是「没有跑完」，红的是另一行。
- 补第 5.2 节前三行的断言时（2026-11-17）：这些检查盯的是已经交付的行为，写完就是绿的，所以也做了对照运行，共 26 次，事先写下每一次应当红在哪一组、或者应当全绿。产品这一侧，在 GUI 和服务器的源码里加了 11 个用环境变量挑选的故障开关（13 处改动），一起编成一份 GUI 和一份服务器，只放在仓库外，一次只开一个；改过的源文件已逐字节还原。8 次应当全绿的确实全绿：四个场景各对着按 `adb4aed54` 编出的程序跑一遍，再对着带开关、一个都不开的那一份跑一遍。13 次各开一个故障，都红在事先写下的那一组：`t10_broadcast` 4 次（发出的变更不带位号来源；接收方不认带来的来源；发出的变更不带手改标记；接收方不认带来的标记），`x05_m3_persistence` 4 次（发送方不读另存的属性；重建对象时不带另存的属性；隔着对方的改动撤销的那条路上两处：比较撤销前后时不读另存的属性，换回撤销步里留的记录时不保留对方写的属性），`x10_concurrent_insert` 3 次（加入时不上报已有位号；重新加入后不把占用要回来；服务器不登记上报的位号），`t20_session` 2 次（不上报；服务器不登记）。4 次换的是场景：`t20_session` 用补断言之前的那一版（过时的那一项红），再用一份换了用户启动的副本（改过的那一项、新加的那一项都红，还连带着红了原有的一项）；`t10_broadcast` 用两份副本，一份把改管线那一步改成别的属性（`T2.4-overrides` 红 4 项），一份在计数之前多发一条变更（只红数变更条数的那一项）。1 次是修 `adb4aed54` 那处缺陷之前的 GUI 留下的输出（`T2.4-claim-after-rejoin` 里 2 项红）。新加和改过的 39 项里 33 项至少红过一次；没有红过的 6 项都是前提（实例就绪、打开图、先画好或删掉要用的对象）。`T2.4-overrides` 的「没有标记的管路跟着管线变」和「11 件事正好 11 条变更」两项、`t20_session` 改过的那一项，产品这一侧的 11 个故障没有一个让它们红，只在换场景的对照里红过；`T2.4-claim-after-rejoin` 的「释放得掉」一项只在修缺陷之前的那份输出里红过。有两次连带着红了别的组：换回记录时不保留对方写的属性的那一次，同时红了原有的 `VAL-CROSS-005` 两项；不上报已有位号时没有重新加入这一步，`T2.4-claim-after-rejoin` 也红了两项。
- 补看图的场景时（2026-11-17）：这些检查盯的也是已经交付的行为，写完就是绿的，所以同样做了对照运行，共 62 次，事先写下每一次应当红在哪几项、或者应当全绿，结果都如所写。产品这一侧，在 GUI 的源码里加了 34 个用环境变量挑选的故障开关（29 处改动，分在 7 个文件里），编成一份 GUI，只放在仓库外，一次只开一个；改过的源文件已逐字节还原。15 次应当全绿的确实全绿：先对着正常的程序造一遍 `t00_smoke`、`t21_claims` 要打开的那张种子图（`t01_seed`），再让七个脚本（`g01`–`g05`、`t00_smoke`、`t21_claims`）各对着正常的程序跑一遍、各对着带开关而一个都不开的那一份跑一遍。43 次各开一个故障，都红在事先写下的那几项上：`g01_palette` 15 次（面板停到左边；第二次 `PIDPALETTE` 不关面板；从第二次打开起停到左边；打开面板时往图里加一条线；换页签不生效；点亮的是下一个页签；最后一个页签不画；页签上不写字；每个页签都写第一个分类的名字；每格都画同一个图标；不画图标；切换文档时把面板关掉；切换文档后回到第一个页签；新建文档的请求什么都不做；截图每次都交出第一次截的那一张），`g02_insert` 6 次（符号落点偏出 (30, 20)；截图应答里的矩阵横向偏出画面宽度的 5%；标注的 `screen_pixel` 横向偏 40 像素；不画高亮；清空选择不生效；截图交出旧图），`g03_preview` 5 次（预览停在第一个点不跟着走；空格不转；Ctrl+空格不换锚点；取消不生效；截图交出旧图），`g04_tag_display` 6 次（位号属性不可见；位号画在符号本体上；重号的符号仍显示原来的位号；属性面板的「位号」行在重号时仍显示原来的位号；每放一个符号多写一个带作者记录的实体；截图交出旧图），`g05_pipe_break` 5 次（不画高亮；不画编号标记；放管件时管路不截断；截断处又被一条线接上；截图交出旧图），`t00_smoke` 4 次（应答里的 `width` 多 1；矩阵偏；`screen_pixel` 偏；截图交出旧图），`t21_claims` 2 次（被占用的对象不上色；截图交出旧图）。4 次换的是场景（副本只放在仓库外，对着正常的程序跑），针对的是没有哪个故障够得着的四项前提：`g01_palette` 在该取「面板关着」的那一张之前先把面板打开，`g03_preview` 用符号库里没有的名字启动 `PIDINSERT`，`g04_tag_display` 只放两个符号，`g05_pipe_break` 不放阀门；四次都红在那一项前提上（后三份到这一项就停了；`g01_palette` 那一份接着往下跑，后面拿这张图当「关着的样子」来比的 4 项也跟着红了）。新写和改过的 49 项（五个看图场景的 45 项、`t00_smoke` 新加的 3 项、`t21_claims` 改过的 1 项）里 44 项至少红过一次；没有红过的 5 项是五个看图场景各自的第一项（实例就绪）。截图交出旧图的那个故障七个脚本都试了，每个脚本里都有靠图判的项因此失败。
- 计时的三个场景和 `t02_harness_rules` 改过的四项（2026-11-17）。哪个对照该红在哪几项，跑之前写下，跑完逐个对：24 个对照，该绿的 8 个全绿，该红的 16 个都红在写下的那几项上，别的项不红。变异程序（在产品源码里临时加 8 处故障，各由环境变量里的一个开关打开，编出一份放在仓库外，源码随即逐字节恢复、重新编译）：不开开关，`p01`、`p02` 全绿；每次插入多停 80 毫秒，`p01` 的两个中位数红；图里已有的 P&ID 实体每多一个、插入多停 0.4 毫秒，第二组的中位数和增长红；插入不写作者记录，数作者记录的 3 项红；批量少放一个，`p02` 的放置、计时、读回 3 项都红；批量编号变慢，计时红；有一个位号重复、编号错一位，读回红。`VAL-SEM-014` 的测试是从源码编的，所以把故障留在源码里跑 `p03`：开开关 2 项红（三轮都是四千多毫秒），不开全绿，跑完恢复。场景的副本只改一处：底图只建 4000 条线、符号只补到 150 个、装作找不到 curl，`p01` 各红在该红的那一项；`p03` 把测试名写错、或者去掉 `--ignored`，cargo 的退出码是 0 但一条测试也没有跑，2 项都红；`t02` 把上限写成 0 秒红 3 项，把「图纸不存在」换成一张存在的图红 1 项。慢启动的一对：让实例开始监听之前先停 8 秒（启动到就绪 8.3–8.6 秒），改之前的 `t02`（上限固定 6 秒）红的正是第四遍全量里红的那两项，改之后的 13 项全绿，上限取的是 19 秒。新增和改过的 18 项里 16 项至少被一个对照打红，剩下的 2 项是两个场景里「实例就绪」的准备项。两处要交代：`t02` 的第一个对照是设计错了——副本改了文件名（脚手架按脚本的文件名给记录文件起名，按名字找记录的两项跟着红），上限写的 1 秒也没有让 1.2 秒就绪的实例超时，结果红的不是写下的那两项；改了对照、重新写下预期再跑，才是上面的结果。「批量少放一个」的预期起先写的是 2 项，跑之前改成了 3 项。
- 2026-11-17 的前两遍（收进仓库时跑的原有 22 个场景、补完验收规程后跑的五组全量），原有 22 个场景的结果文件都与 2026-11-12 的逐个场景、逐个标签比过通过和失败的项数：除 `t20_session` 之外全部相同。`t20_session` 里过时的那一项两遍都是失败（2026-11-12 是通过），第二遍另多出后加的 `VAL-INFRA-012` 一项。
- 第三遍（补了第 5.2 节的断言后跑的五组全量）与第二遍逐个场景、逐个标签比过：除补了断言的四个场景之外，其余 26 个场景和收尾的整轮检查各标签通过和失败的项数全部相同；四个场景里多出来的正是新加的 38 项，另有 `t20_session` 的 `VAL-CLI-001` 从 4 项通过、1 项失败变成 5 项通过。
- 第四遍（补了看图的场景后跑的六组全量）与第三遍逐个场景、逐个标签比过：两遍都有的 34 份结果（31 个场景、两张种子图、`t03_vanish_retry` 的第一次）里，31 份各标签的通过数和失败数完全相同。不同的 3 份：`t00_smoke` 多了 3 项（`VAL-INFRA-009` 1 项、`VAL-INFRA-010` 2 项），原有的 7 项不变；`t02_harness_rules` 的 `VAL-INFRA-014` 由 4 项通过变成 2 项通过、2 项失败；`t25_offline` 的 `VAL-CLI-015` 由 6 项通过变成 5 项通过、1 项失败。后两处的原因见第 3.3 节。看图的五个场景只在第四遍里有。
- 第五遍（补了计时的场景后跑的七组全量）与第四遍逐个场景、逐个标签比过：两遍都有的结果文件 39 个（含两个种子脚本和 `t03_vanish_retry` 留下的第一次尝试），37 个各标签通过、没过的项数全都相同；不同的 2 个是 `t02_harness_rules`（`VAL-INFRA-014` 通过 2 → 4）和 `t25_offline`（`VAL-CLI-015` 通过 5 → 6），项数没有变，都是上一遍没过的这一遍过了（第 3.3 节）。计时的三个场景只在第五遍里有。
- 离线那一节（第 4.10 节，2026-11-20 `OFF-13`）：OS-01 – OS-21 各出现一次。项数是逐个场景、逐个标签从 `OFF-13` 那一轮 `run19.ps1 -OnlyOffline` 的结果文件里数的：八个场景 238 项全过，带 OS 编号的 67 项与表里写的逐条相同；`o05` 后来加的那一项 `REQ-COLL-004`（离线规格 §11.18）不在这 238 项里。表里引用的 25 个单元测试（OS-16 的 17 个，OS-18、OS-19 各 4 个）都在源码里唯一地找到；它们在离线规格 §11.16 那次全量测试里是通过的，`OFF-13` 没有动 Rust 代码。这 67 项每一项都至少在一次对照运行里失败过：`o01` – `o04` 的 34 项见离线规格 §11.15（`o04` 后加的那一项见 §11.16），`o05` – `o08` 的 33 项见 §11.17。
- 「有」「部分」「缺」是逐条对着契约正文和测试、脚本的代码判断的，判断时没有重新执行端到端脚本。前六个区逐条读了契约正文和测试代码；后三个区以状态文档当时的逐条结论为准，对带计时、截图判据的和后补的几条另读了脚本的检查项。
