# 项目约定（OpenCADStudio / 智能 P&ID 平台）

## 默认仓库

- **本项目默认仓库是 `https://github.com/happyrust/pid-cad.git`**（远程名 `pid-cad`）。
- 任务单、Issue、阶段跟踪、验收记录一律在 `happyrust/pid-cad` 上，不在 `happyrust/OpenCADStudio` 上。
- 其余两个远程只作代码来源，不要当成本项目的任务仓库：
  - `origin` = `https://github.com/HakanSeven12/OpenCADStudio.git`（上游原作者）
  - `fork` = `https://github.com/happyrust/OpenCADStudio.git`（派生，仅 `main` 跟踪）
- `git config --local remote.pushDefault` 已设为 `pid-cad`；不带参数的 `git push` 走它。

## 分支与推送

- 主线在本地 `main` 上以原子提交推进，一个 Issue 一个提交。
- **阶段验收通过之前不推送**；推送时机由用户明确指示。
- 并行工作树（`ocs-pid-symbol-block`、`ocs-collab-off2` 等）各自分支推进，主会话改动不得跨树覆盖。

## 阶段一

- 权威文档目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`
- 跟踪总单：`happyrust/pid-cad#106`
- 门禁六项：`cargo test --offline --locked --lib`、双 `clippy`、`scripts\fmt-newhunks.ps1`、`python scripts/arch_check.py report`、`python -m pytest scripts/sppid`
- 读数账本：`readings.md`（逐单一节，含先红后绿、真实图实测与门禁读数）
