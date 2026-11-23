# 校验与交付面板的端到端脚本

`run.py` 起真实程序，只经它的本机 REST 通道（`--http`）驱动「校验与交付」面板，核对规格书
`docs/plans/2026-11-16-pid-dev-spec/SPEC-pid-verify-deliver-panel.md` §7 里「端到端」那一行，也就是
PNL-13 的后两条验收。写法照 `scripts/collab-e2e/`：不模拟鼠标、不靠截图，程序起在单独的隐藏桌面上，
脚本里没有写死的本机路径，证据写在仓库之外。

另一条验收（2,000 条结果时重建要画的内容不超过 50 ms，每组只画前 200 行）是默认不跑的单元测试，不在这里：

```
cargo test --offline --locked --lib -- --ignored pid_verify_build_view_2000 --nocapture
```

## 1. 需要什么

- Windows，Python 3.9 以上，只用标准库。
- `cargo` 在路径上。脚本先在仓库目录里执行 `cargo build --offline --locked --bin OpenCADStudio`，再把 cargo
  报出的可执行文件（`cargo run` 起的也是它）连同 `<图> --backend vulkan --new-instance [--classic] --http <端口>`
  起起来，不用事先编好。
- REST 端口空着，默认 8090。每次起程序前先查，被占时这种界面记一项失败、不往下跑。
- 程序起在单独的桌面上（仓库的 `docs/automation/hidden_desktop.py`），窗口不出现在人正在用的桌面上；脚本不动
  鼠标键盘，跑的时候机器可以照常用。带 `--new-instance` 是为了不占单实例的监听：不然跑的这几分钟里，
  双击打开的图会跑进这个看不见的窗口。

## 2. 怎么跑

```
python scripts/pid-verify-e2e/run.py
```

| 参数 | 是什么 | 不给时 |
|---|---|---|
| `--out` | 输出目录 | 环境变量 `OCS_E2E_EVIDENCE_DIR` 下的 `pid-verify`；也没有时在系统临时目录下新建 `ocs-pid-verify-e2e-<时刻>`。给的目录在仓库里面时报错退出 |
| `--port` | 程序的 REST 端口 | 8090 |
| `--ui` | `shell`（外壳）、`classic`（经典界面）或 `both` | `both`，两种界面各起一次程序 |
| `--copies` | 检查全项目前往项目目录复制几份空白图 | 3000 |

退出码：全部通过时 0；有失败时 1；跑不起来（输出目录在仓库里、找不到 `cargo`、编译失败）时 2。

输出目录里：

- `logs/pid-verify.result.txt`：每项一行 `PASS` 或 `FAIL`，带编号和说明；缩进的行是过程记录；最后是桌面那一项和合计。
- `shell/`、`classic/`：各自的 `start.dxf`（起步的图）、`later/PID-0000.dwg`（空白图）、`project/PID-1101.dwg`
  （造的图）、导出的 `bom.csv` 和 `bom-pipe-summary.csv`、程序的设置目录 `appdata/`、程序的输出
  `editor-stderr.log` 和起停记录 `report.txt`。复制进项目目录的空白图在取消之后删掉。

## 3. 每种界面查什么

编号前面是界面（`shell/`、`classic/`），后面是规格书 §6 成功标准的条号。面板都用 `action` 操作驱动（`name` 为
`pid_verify`，§3.3），状态都从 `GET /api/v1/pid/verify-panel` 读（§3.2）。

| 编号 | 做什么 | 判据 |
|---|---|---|
| `准备` | 从仓库里的小图起步，另存 `later/PID-0000.dwg`，复制成 `project/PID-1101.dwg` 并用 `open` 打开，画泵、阀门、容器、两条管线、两个管件和一个仪表，存回原处。带路径的 `save` 只是另存一份，标签页不跟着换，所以要打开 | 空白图存下来了；面板说当前图是 `PID-1101`；八样都画上了；存回后没有未存的修改（发布前提里的 `clean` 为真） |
| `§6.1` | `show` | `ui_bounds` 按前缀 `pid-verify-` 读得到 `pid-verify-panel` 和三个页签，`visible` 为真 |
| `§6.2` | 依次 `tab=bom`、`tab=deliver`、`tab=check` | 面板状态的 `tab` 跟着变 |
| `§6.3` | `check=drawing`，再调 `pid_check`（`POST /api/v1/pid/check`） | 面板的计数等于 `pid_check` 的 `summary`；面板列出的行按（规则、对象、图号）与 `pid_check` 里没豁免的结果一致，行数等于 `total`。另一项：`pid_check` 之后面板每行的 `index` 指向它 `results[]` 里同一条 |
| `§6.8` | `tab=bom`、`bom=drawing`，再调 `pid_bom`（`GET /api/v1/pid/bom`）；然后 `bom_export=<输出目录>/<界面>/bom.csv` | 面板清单的 `total` 等于 `pid_bom` 的行数。导出后 `bom.csv` 的记录数是表头加这些行，副表 `bom-pipe-summary.csv` 是表头加管道汇总的行 |
| `§6.12` | 往项目目录复制空白图，`check=project`，读面板状态；还在读的时候再按 `check=drawing`；然后 `cancel` | 读到 `busy.done`、`busy.total`（都是整数，`done` 小于 `total`，`total` 不少于复制的份数）；再读，`done` 变大了、还没读完，说明进度在走。再按的那一下被拒：应答的 `code` 是 `PANEL_ACTION_UNAVAILABLE`、`reason` 是 `busy`（§3.7）。取消后 `busy` 为 `null`，规则检查页签显示的检查（范围、时刻、规则集版本、过期标记、计数、全部行）与之前相同；2 秒后再读仍相同 |
| `§6.13` | `hide` | `ui_bounds` 一个 `pid-verify-` 开头的位置也读不到，`visible` 为假 |
| `desktop` | 整轮 | 人正在用的桌面：指针和前台窗口跑前跑后一样。期间有人用过机器时不算失败 |

## 4. 几点说明

- 全项目检查要慢到能读到进度、来得及取消，所以项目目录里放 3,000 份空白图（做法同 CP-E3 的走查）。机器快到
  点取消之前就查完了，`§6.12` 会失败，说明里能看到 `busy`；这时用 `--copies` 加大。
- 不在隐藏桌面上直接 `cargo run`：那里的进程工作目录是输出目录，而 cargo 按工作目录找 `.cargo/config.toml`，
  读不到仓库的配置（Windows 上加大栈的编译参数），整棵依赖会换参数重编，编出的程序也不一样。所以在仓库目录里编，
  再起编出的文件。
- 脚本结束时先对程序窗口发关窗消息；没关掉（图有没存的修改时会问要不要存，没人回答）就结束进程。脚本中途被中断时，
  程序也跟着结束。
- 控件位置只有真实窗口里才有，这是这个脚本要起程序、不能只靠程序级测试的原因。
