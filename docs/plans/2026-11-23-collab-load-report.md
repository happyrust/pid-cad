# 协同服务器 20 会话负载与时延测量报告（#38）

- **日期**：2026-11-23
- **分支 / 基线**：`feat/collab-server-ready`（基于 `main @ 32ac241dd`）
- **测试套件**：`crates/ocs_collab_server/tests/load_20_sessions.rs`
- **复现命令**：
  ```powershell
  cargo test --offline --locked -p ocs_collab_server --test load_20_sessions -- --nocapture
  ```
- **机器与构建**：
  - OS：`Windows-11-10.0.26200-SP0`
  - CPU：`AMD64 Family 25 Model 97 Stepping 2, AuthenticAMD`（32 逻辑核心）
  - Cargo 配置：`test` profile（optimized），SQLite WAL 模式（本地磁盘临时库）

## 场景配置

- 1 台真实 `ocs_collab_server`（TCP WebSocket + SQLite WAL 持久化）
- 20 个并发编辑会话（`u-00` .. `u-19`）+ 1 个观察者会话（`u-20`，模拟图形客户端），共 21 个在线会话同图协同（`PID-LOAD-01`）
- 20 次位号占用（`P-1001` .. `P-1020`）
- 200 次连续属性修改提交，每次等待全部 21 个会话收到该修订号的 `op_applied` 广播后计入端到端广播时延

## 实测结果

| 指标 | 中位数（p50） | 95 分位（p95） | 最大值（max） | 说明 |
| --- | --- | --- | --- | --- |
| 加入握手耗时（`hello` → `welcome`，21 会话） | **0.82 ms** | **1.11 ms** | **1.80 ms** | 含 TCP 连接 + WS 升级 + 写库 + 广播 |
| 占用申请应答（`claim` → `claim_granted`，20 次） | **0.57 ms** | **0.62 ms** | **0.62 ms** | 含权限判定 + 写库 + 全图广播 |
| 广播扇出时延（`op` → 21 会话全部收到 `op_applied`，200 次） | **0.77 ms** | **0.92 ms** | **3.36 ms** | 含三段裁决 + SQLite WAL 事务（`ops` + `audit_log` + `drawings` + `sessions`）+ 21 路扇出 |

| 资源指标 | 实测值 | 说明 |
| --- | --- | --- |
| 数据库文件总大小（`.db` + `-wal` + `-shm`） | **4,349,600 bytes（约 4.15 MiB）** | 200 行 `ops` + 200 行 `audit_log` + 21 行 `sessions` + 20 行 `claims`（含 WAL 预分配） |
| 200 次提交全流程总耗时 | **0.24 s** | 单进程测试含建库、21 路建连、20 次占用、200 次提交与全部扇出确认 |

## 非功能目标门限说明

当前非功能目标值尚未正式敲定（`70-collaboration-and-permissions.md` 未决事项第 22 条）；本次仅如实出数作为基线。后续确定正式门限（如 `broadcast_p95_ms <= T`）后，可直接在同一测试 `load_20_sessions.rs` 中加上阈值断言，超过即失败。
