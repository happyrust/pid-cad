# 任务清单：`.pid` 图例导入成图块

> 日期：2026-11-19。状态：执行中。
> 配套：规格 `SPEC-pid-symbol-block.md`（下文简称「规格」）、计划 `plan.md`。
> 分支 `pid-symbol-block`，起点 `5171d197b`；一个任务一个本地提交，不推送。

## 0. 怎么读这份清单

每个任务写六项：

- **做什么**：这个任务改什么。
- **验收**：做成的标准，一条一个复选框，每条都有测试或读数对应。
- **验证**：跑什么命令，证据放在哪。
- **依赖**：要先做完哪个任务。
- **文件**：预计要动的文件。实际动的不一样时，在「做完」段写明。
- **规模**：S（1–2 个文件）、M（3–5 个文件）、L（更多，或要改很多期望值）。

类别：【纯】不碰外部状态的函数；【接线】把功能接进现有流程；【测试】只加测试；【文档】；【验收】在真实程序上量。

任务做完后，在它下面加一段「**做完**（日期）」，写读数、没验证的、与清单不一样的地方，并把复选框勾上。

## 1. 每个任务都要过的固定门禁

1. **先红后绿**。先写测试并看到它失败，再写实现。只加测试的任务没有「先红」，用第 2 条代替。
2. **故意改坏**。在这个任务的代码里放几处由环境变量 `OCS_MUTANT` 控制的改坏点，编译一次，逐个打开，每一处都要有测试变红。核对完删掉改坏点，提交里不留。
3. **全量单元测试** `cargo test --offline --locked --lib`：失败的只能是基线里就失败的那一个；通过数等于上一个提交的通过数加本任务新增的测试数。
4. **集成测试**（SB-4b 起）`cargo test --offline --locked --no-fail-fast --test pid_import --test pid_legend`：两个文件都全过，个数等于上一个提交的加本任务新增的；测试文件编译没有告警；跑完之后系统临时目录里没有名字带 `symbol` 的文件夹。样例图在同级目录 `pid-parse\test-file\` 里，没有样例时这些测试直接算通过，那样的「全过」不算数。
5. **clippy** `cargo clippy --offline --locked --lib` 和 `... --profile test`：两个告警数都不高于上一个提交。
6. **格式** `scripts\fmt-newhunks.ps1 <改前的副本> <改后的文件>`：每个动过的 `.rs` 文件 `new=0`。
7. **分层** `python scripts/arch_check.py report`：`io` 开头的行不增加。
8. **文档检查**：只在动了 `docs/pid-platform/` 时跑（SB-8）。
9. **提交**：只 `git add` 本任务的文件和 `todo.md`；说明里写读数、没验证的、门禁结果；改了既有测试的期望值的，逐处写原值、新值、原因。
10. 与规格不一致的地方记进规格 §11。

第 3–7 条用一个脚本跑（在仓库外）：`D:\scratch\ocs-symblock\tools\gates.ps1 -Out <证据目录> -Tag <标记> -Files <本任务的 .rs> -Quick <测试名过滤词>`。集成测试默认跑 `pid_import`、`pid_legend` 两个文件（`-Integ` 可以换，`-NoInteg` 不跑）。编译输出的目录是几个工作目录共用的（见下一段），所以脚本在这一步之前先把 `src\lib.rs` 的修改时间改成当时（内容不动），让测试连的库在同一次 cargo 调用里由本工作目录的源码重编，不会拿到别的工作目录留在那里的库。证据在 `D:\scratch\ocs-symblock\<任务>\`。

编译输出的位置（2026-11-19 SB-2 收尾时更正）：这台机器上的 cargo 不认 `CARGO_TARGET_DIR`，它提示 `this cargo always uses ./target`。中间产物和测试程序在 `D:\Rust\.cargo\targets\OpenCADStudio-9c2d1eeb\`（这个目录建于 11-17，早于本工作目录，可见是与别的工作目录共用的），发行版的 exe 在工作目录自己的 `target\release\`（`target` 在 `.gitignore` 里）。这里原先写的「编译输出在 `D:\scratch\ocs-ci\target`」不对，各证据文件开头 `target: D:\scratch\ocs-ci\target` 那一行也不对；测试和 clippy 的读数不受影响，证据里的 `Compiling OpenCADStudio … (D:\work\plant-code\cad\ocs-pid-symbol-block)` 一行说明编的是本工作目录的源码。

**基线读数**（`5171d197b`，2026-11-19 18:04–18:18 在本工作目录量；证据 `D:\scratch\ocs-symblock\00-baseline\`）：

| 项 | 读数 |
|---|---|
| 编译器 | rustc 1.102.0-nightly (65ce8611f 2026-11-18)，clippy 0.1.102，rustfmt 1.12.0-nightly |
| 全量单元测试 | 7,069 个：通过 7,015，忽略 53，失败 1 |
| 基线里就失败的那一个 | `app::pid_shell::pid_shell_chrome::what_the_editor_shows_is_drawn_in_the_frame_where_a_window_of_its_own_draws_it`（画面逐像素比对，26,644 个像素不同；与导入器无关，不在本计划里处理）。SB-1 的全量里它通过了，可见结果随机器当时的状态变：之后各任务里它通过或失败都不算那个任务的变化，通过数因此可能差 1 |
| 集成测试 | 起点上没有量，固定门禁原先不跑它。2026-11-20 02:17–02:25 补量（证据 `D:\scratch\ocs-symblock\integ\`）：`035046b4e` 上 `tests/pid_import.rs` 54 个，通过 43，失败 11；`tests/pid_legend.rs` 29 个全过。把分支改过的 9 个源文件换回起点的版本（分支新加的 4 个文件留着）：`tests/pid_import.rs` 通过 53，失败 1，这 1 个是系统临时目录里留下的文件夹造成的。来历和处理见 SB-4b；SB-4b 之后是 55 个、29 个，全过 |
| clippy（`--lib`） | 656 条告警 |
| clippy（`--lib --profile test`） | 1,051 条告警 |
| 分层报告里 `io` 开头的行 | `io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1` |
| 样例导入用时（只带 `.pid`） | 9 次：最短 3,836 ms，中位数 4,569 ms，最长 6,296 ms。量的时候别的窗口也在编译，读数上下抖动约一半，只能当量级看；SB-9 要在同一次运行里把基线和改后的值一起重新量。证据 `D:\scratch\ocs-symblock\sb1\baseline-measure-1.log` |
| 样例导入后的图元 | 全部 10,275 个，都在模型空间；块记录 2 条（模型空间、图纸空间）。按角色：`symbol` 3,792、`symbol-label` 592、`fill` 167、`geometry` 2,471、`text` 958、`point-ok` 1,864、`point-warning` 430、`frame` 1。按类型：LINE 4,648、POINT 1,864、TEXT 1,580、LWPOLYLINE 1,323、CIRCLE 470、ARC 223、HATCH 167 |
| 样例的导入摘要 | `drawn` 10,274、`decoded` 5,527、`missing` 0、`sheet_layers` 4、`layered_entities` 10,274、`unresolved_sheet_layers` 0、`sheet_layer_names` 4、`sheet_layers_off` 1、`driving_dimensions` 6、`template_bodies` 3、`parametric_placements` 2、`cache_bodies` 565、`library_bodies` 0、`hidden_strokes_skipped` 1,087、`label_anchors_undrawn` 27、`lettering_flattened` 0、`template_entities` 0、`template_missing` 0 |
| 另存 DWG 的大小 | SB-9 用基线程序量 |

## 2. 测试怎么写、放在哪

- 导入器的单元测试放 `src/io/pid/tests.rs`：在代码里拼 `pid_parse::PidGraphicEntity`，不依赖样例文件。`symbol_blocks.rs` 自己的纯函数测试放在该文件的 `mod tests`。
- 样例测试：本机有 `pid-test-data` 才跑，没有就打印一行并跳过；写法照仓库里现有的样例测试。
- 程序一层的测试（点选、命令）：写法照 `src/app/block_editor_tests/parameter_preview.rs` 的 `block_move_point_targets_can_be_picked_in_the_viewport`，直接设视口大小，用 `ViewportLeftPressAt` 和 `ViewportLeftRelease` 两条消息点一下。
- 改既有测试的期望值：不删测试，不把相等改成大于小于；每处在提交说明里写原值、新值、原因。

---

## 第一段：建块

### SB-1 【纯】建块的纯函数

- **做什么**：新文件 `src/io/pid/symbol_blocks.rs`，四件事：把一组图元平移到以插入点为原点；算定义内容的指纹；一张登记表，按符号名和指纹决定共用还是新建、给出块名；往文档里建定义、加引用。模块先以 `#[cfg(test)] mod symbol_blocks;` 入库（计划 §2 第 5 条）。开始前先量样例导入用时的基线。
- **验收**
  - [x] 平移：线、圆、圆弧、多段线、样条、文字（含对齐点）、填充（边界）七类，平移后每个坐标等于原坐标减插入点，误差不超过 1e-9；半径、角度、字高、凸度不变。
  - [x] 指纹：同一组图元取两次相同；坐标差 1e-8 的相同，差 1e-5 的不同；颜色、线宽、线型、图层、文字内容、字高各改一项都不同；文字内容里的数字不参加取整。
  - [x] 块名：`SPPID_<洗过的符号名>`；同一个名字的第二种画法加 `_2`，第三种加 `_3`；没有符号名时是 `SPPID_SYMBOL`；文档里已经有同名的块（不分大小写）时往后数；块名不允许的字符被洗掉。
  - [x] 共用：内容相同的第二个图例拿到同一个块名，文档里只有一个定义；内容不同的各有一个。
  - [x] 建定义：块记录、BLOCK、成员、ENDBLK 都在；成员的归属是这条块记录；ENDBLK 的句柄大于所有成员；`entities_in_block` 的个数等于成员数；模型空间不多出图元。
  - [x] 加引用：引用在模型空间，块名、插入点对，旋转 0，比例 1。
  - [x] 建好的文档用现有的存盘函数写成 DWG 再读回，定义和引用都在，成员数相同。
  - [x] 故意改坏：平移方向反了、指纹漏掉图层、取整位数改成 3、序号从 `_1` 起、共用时仍新建定义、ENDBLK 先于成员，每处都有测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb1 -Files src/io/pid/symbol_blocks.rs,src/io/pid/mod.rs -Quick symbol_blocks`。
- **依赖**：无。
- **文件**：`src/io/pid/symbol_blocks.rs`（新）、`src/io/pid/mod.rs`（一行）。
- **规模**：M。

**做完**（2026-11-19）

- 读数：新增测试 11 个，都在 `symbol_blocks.rs` 的 `mod tests`。先红：只有函数壳子时 11 个全失败；写完实现 11 个全过。全量 7,080 个：通过 7,027，忽略 53，失败 0（基线的 7,015，加新增的 11，加基线里失败的那一个这次通过）。clippy 656 / 1,051，与基线相同。两个文件都是 `new=0`。分层报告与基线逐行相同。
- 故意改坏：放了 13 处。清单要求的 6 处都在内，另加 7 处：不平移、符号名不参加共用、已占用的块名照用、引号里的数字也取整、引用放在图纸原点、块记录不记 ENDBLK、成员不归块记录。13 处都有测试变红；11 个测试每个都至少被一处改坏变红。在最终的文件上重做过一遍，结果相同。改坏点已删，删后文件的 SHA-256 与放之前相同。
- 与清单不一样的地方：
  - `place` 返回引用，不把它加进文档。引用的图层和记录由调用方（SB-2）给，给完再加。「加引用」一条的测试是把返回的引用加进模型空间之后量的。
  - 指纹用的是每个成员的 `Debug` 文本，引号外的小数取整到 6 位。成员上所有的项（包括 `role=` 记录）都参加比较，比清单列的几项多；多出来的差别只会多建定义，不会画错。
  - 样条：导入器现在不产生样条（本体里的 B 样条画成多段线），平移的测试里仍然测了。
- 没验证的：共用之后样例上有多少个定义（规格 §2.5 估 118 个，按插入点对齐的数要到 SB-2 接上导入循环才量得到）；块在程序界面里的表现（SB-3、SB-4）。
- 基线：样例导入用时、图元分布、导入摘要已量，填进了 §1 的表。量用时的临时测试没有入库，放在 `D:\scratch\ocs-symblock\tools\zz_baseline_tmp.rs`，SB-9 再用。
- 证据：`D:\scratch\ocs-symblock\sb1\`（`red-11-failed.log`、`quickgreen-1.log`、`mutants2-result.txt`、`chainfinal2.txt`）。

### SB-2 【接线】导入循环按记录建块

- **做什么**：`src/io/pid/build.rs` 的 `build_document_entities` 里，符号实例记录画出的图元不再逐个加进模型空间。改成：后处理照旧做完 → 按现规则算每个图元本来的图层 → 定引用的图层（规格 §3.1）→ 块内图元同层的改到 0 层、只写 `role=` → 平移、取块名（新定义就建）→ 加引用，引用上写完整记录加 `symbol=`。`src/io/pid/metadata.rs` 加写 `symbol=` 的函数。模块改成常规的 `mod`。变红的既有测试改期望值。
- **验收**
  - [x] 一条符号实例记录：模型空间里 1 个块引用、文档里 1 个定义；模型空间里没有 `role` 为 `symbol`、`symbol-label`、或出自这条记录的 `fill` 的散图元。
  - [x] 引用：插入点等于记录的插入点换算到图纸坐标的值；记录里 `role=symbol`，`sheet_layer`、`sheet_layer_oid`、`style`、`extent`、`driving`、`instance` 的值与改之前写在笔画上的相同；另有 `symbol=`，值是符号名原样（含块名不允许的字符）。
  - [x] 带发布数据命中时，`class`、`label`、`oid`、`resolved` 在引用上，块内图元上没有。
  - [x] 块内图元：坐标加插入点等于改之前的坐标；只带 `role=`；颜色、线宽、线型、字高、对齐与改之前相同；符号名文字在块里。
  - [x] 图层，逐种情况：

    | 记录 | 引用 | 笔画 | 填充 | 符号名 |
    |---|---|---|---|---|
    | 带图层名，显示 | 该图层 | 0 | 0 | `PID-SYMBOL-LABEL` |
    | 带图层名，隐藏 | 该图层（关） | 0 | 0 | `PID-SYMBOL-LABEL` |
    | 不带，显示 | `PID-SYMBOL` | 0 | `PID-FILL` | `PID-SYMBOL-LABEL` |
    | 不带，隐藏 | `PID-HIDDEN` | 0 | 0 | 0 |
    | 只画符号名，带图层名或不带且显示 | `PID-SYMBOL-LABEL` | — | — | 0 |
    | 只画符号名，不带且隐藏 | `PID-HIDDEN` | — | — | 0 |

  - [x] 画法相同的两条实例（只是位置不同）共用一个定义；旋转不同的不共用，块名带序号。
  - [x] 没有本体、画成标记圆点的图例成块；只画符号名的标注类图例成块；边框模板里的符号实例照旧是 `role=frame` 的散图元。
  - [x] 不是符号实例的记录（线、文字、独立的填充、点标记）照旧是散图元，记录不变。
  - [x] `drawn`、按图纸图层的分布、取景范围与改之前相同。
  - [x] 既有测试：先跑全量，列出变红的；逐个改期望值，提交说明里逐处写原值、新值、原因；不删、不放宽。
  - [x] 故意改坏：符号名文字留在块外、块内图元不改 0 层、块内图元写了 `sheet_layer=`、引用上没有 `symbol=`、引用的图层总用 `PID-SYMBOL`、只画符号名的图例不成块，每处都有测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb2 -Files <动过的 .rs> -Quick io::pid`。
- **依赖**：SB-1。
- **文件**：`src/io/pid/build.rs`、`src/io/pid/metadata.rs`、`src/io/pid/mod.rs`、`src/io/pid/tests.rs`，加上要改期望值的测试所在的文件（规格 §7 列了可能涉及的）。
- **规模**：L。代码是 M；要改多少期望值，量过才知道。

**做完**（2026-11-19）

- 读数：新增测试 12 个：`tests.rs` 的 `mod placements_as_blocks` 10 个，`symbol_blocks.rs` 的 `mod tests` 2 个。先红：建块只留壳子时，前 10 个全失败。接上实现后 8 个过、2 个失败（「画法相同的共用」「计数不变」两条），查出填充的图案原点问题（见下面「与清单不一样的地方」第 1 条），改完全过。`io::pid` 一组 267 个通过，0 个失败，5 个忽略。全量 7,092 个：通过 7,039，忽略 53，失败 0（上一个提交的 7,027 加新增的 12）。clippy 656 / 1,051，与上一个提交相同。五个文件都是 `new=0`。分层报告与上一个提交逐行相同。
- 既有测试：接上实现后第一次跑全量，失败的 2 个都是这次新增的；没有一个既有测试变红，期望值改了 0 处。
- 故意改坏：放了 15 处。清单要求的 6 处都在内，另加 9 处：实心填充留着平移写下的图案原点、带图案线的填充丢掉图案原点、块内图元也写发布数据的身份、引用上不写 `extent`/`driving`/`instance`、引用的角色写成符号名的角色、引用的插入点不换算到图纸坐标、一个图例只算 1 个 `drawn`、不与引用同层的块内图元也改到 0 层、不是符号实例的记录也只写 `role=`。15 处都有测试变红；12 个新测试每个都至少被一处改坏变红；别的测试没有变红的。在最终的文件上跑的；改坏点已删，删后文件的 SHA-256 与放之前相同。
- 与清单不一样的地方：
  - 动了 `symbol_blocks.rs`（SB-1 的文件，不在本任务的文件表里）。acadrust 平移填充时总把图案原点写进填充的扩展数据（应用名 `ACAD`），原点随位置变，画法相同、位置不同的两个图例指纹就不同，不共用。改法：平移之后，没有图案线的填充恢复平移前的那条记录（原来没有就删掉）；有图案线的保留平移后的原点。导入器现在只产生实心填充。为此加了 2 个测试。记进了规格 §11。
  - 「边框模板里的符号实例照旧是散图元」没有加新测试。边框模板走 `template.rs` 的 `overlay_template`，这次没动；既有测试 `io::pid::template::tests::pid_template_draws_the_symbols_the_template_places` 断言模板画出的 4 个图元都在 `PID-FRAME`、都写 `role=frame`，它照旧通过。
  - 画不出任何图元的符号实例不建块，和改之前一样什么都不留。
  - `Built` 多带 `symbol_blocks`、`symbol_definitions` 两个数，`finish` 里只多写一行日志。导入摘要加这两项是 SB-5 的事。
- 全部样例的比对（临时测试，没有入库）：`pid-test-data` 下 152 个 `.pid`，1 个改前改后都导入失败，其余 151 个逐图元比：每个图元写成一行（角色、画出来所在的图层、是否隐藏、去掉句柄和记录之后的全部内容，小数按 0.0001 取格），块引用就地展开成块内图元；改前改后各 436,168 行，摘要文件逐字节相同。改后 151 个文件共 28,322 个块引用、8,433 个定义；模型空间里没有 `role` 为 `symbol`、`symbol-label` 的散图元；没有引用空块的；引用都是旋转 0、比例 1、`role=symbol`。这个比对不看记录里 `role` 以外的键，那些由上面的单元测试管。
- 样例（只带 `.pid`）：592 个块引用，119 个定义，58 个符号名，其中 29 个名字不止一种画法；用得最多的五种符号各自的画法数与规格 §2.5 列的相同（2、7、4、6、5）。规格估的是 118，那个数按本体外框对齐、坐标取 4 位小数，没有复现（原脚本没有重跑）。换成更粗的比法还能合并的只有两处：`Flange Type 2` 有 1 个图例的本体和另外 19 个差 0.000027（源数据如此）；`Item Note & Label` 只画符号名，文字相对插入点有两种位置（8 个、6 个）。151 个文件里这两类共 55 个定义，占 0.65%。没有发现该共用而没共用的。记进了规格 §11。
- 没验证的：
  - 导入用时。验收线是基线的 1.1 倍（SB-9、SB-10）。上面那个比对里顺带记的单次读数（测试编译，当时机器上还有别的编译在跑，只能当量级看）：151 个文件合计 256,311 → 281,505 ms（1.10 倍），样例 3,999 → 4,545 ms（1.14 倍）。按代码看，多出来的是每个图例算一次指纹（没有做剖析）。检查点一用发行版把基线和分支交替着量；超了就先改指纹的算法，再往后做。
  - 块在程序界面里的表现（SB-3、SB-4）；发行版上无界面方式的读数（检查点一）。
- 证据：`D:\scratch\ocs-symblock\sb2\`（`chainred.txt`、`chaingreen1.txt`、`chainfinal.txt`、`mutants-result.txt`、`dump-base\`、`dump-after\`、`variants\`）。比对用的临时测试在 `D:\scratch\ocs-symblock\tools\zz_dump2_tmp.rs`。

### SB-2b 【接线】块里的圆、圆弧、椭圆按块内次序叠放

后来加的任务（计划 §9）。检查点一对图时查出来的，改的是显示代码，不是导入器。

- **做什么**：块展开时（`src/scene/cache/block_cache.rs` 的 `emit_wire`），纯圆、圆弧、椭圆成员走单独的一条路，深度只取引用自己的位置（嵌套时只取嵌套引用的位置），不带它在成员里的次序；线、多段线、填充、文字都带。结果：存在块前半段的填充总在曲线下面，后半段的总在上面，正中间的和曲线并列（并列时出图先画填充），与两者存放的先后无关。改成曲线和别的成员用同一个算式。
- **验收**
  - [x] 块里存在曲线之后的填充画在曲线上面，存在之前的画在下面；填充在块的前半段、后半段、正中间都成立；圆、圆弧、椭圆三种都成立。
  - [x] 嵌套块里同样成立；嵌套块里的曲线不越出这个嵌套块在它邻居中间的位置。
  - [x] 既有测试没有因此变红的。有的话说明旧行为是有意的：撤掉改动，只记进规格 §11，停下来问。
  - [x] 样例重新对图：`Set Fire Point` 那一处不再对调，元素和笔与基线相同。
  - [x] 故意改坏：恢复旧行为、只有嵌套块恢复旧行为、嵌套时不按比例缩，每处都有测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb2b -Tag final -Files src/scene/cache/block_cache.rs,src/scene/block_curve_order_tests.rs,src/scene/mod.rs -Quick scene::block_curve_order_tests`；样例对图用 `D:\scratch\ocs-symblock\cp1\` 下的脚本（`cp1.py`、`order.py`）。
- **依赖**：SB-2。
- **文件**：`src/scene/cache/block_cache.rs`（一处表达式）、`src/scene/block_curve_order_tests.rs`（新）、`src/scene/mod.rs`（一行）。
- **规模**：S。

**做完**（2026-11-19）

- 怎么查出来的：检查点一用 SB-2 的程序给样例出图，按清单的比法（元素个数、坐标取整后只在一边的元素）是过的。另外按画的先后比了一遍：221 段先后不同，其中 220 段全是同一支笔的线，1 处是画法不同又互相重叠的一对，`Set Fire Point` 的填充和圆的轮廓对调了。它的成员是圆、填充、符号名，填充排在正中间，和圆并列；并列时出图先画填充。基线里它们是散图元，按存放先后画。
- 读数：新增测试 5 个（`src/scene/block_curve_order_tests.rs`）。先红：改之前前 4 个失败，圆、圆弧、椭圆三种读数相同。`[曲线, 填充, 线]`：曲线 0，填充 0，并列。`[曲线, 填充, 线, 线, 线]`：曲线 0，填充 −0.0925。`[线, 线, 填充, 曲线]`：曲线 0，填充 0.0555。嵌套块：曲线 0.0925，填充 0.1036。第 5 个（嵌套块里的曲线不越出这个嵌套块的位置）改之前就是过的，不算先红，靠故意改坏证明管用。改动在 `block_cache.rs` 一处，删 5 行、加 5 行（1 行算式，4 行注释）。全量 7,097 个：通过 7,044，忽略 53，失败 0（上一个提交的 7,039 加新增的 5）。clippy 656 / 1,051，与上一个提交相同。三个文件都是 `new=0`。分层报告与上一个提交逐行相同。
- 既有测试：没有一个因此变红。所以没有证据说旧行为是有意的，改动留下。
- 故意改坏 3 处：恢复旧行为，前 4 个变红；只有嵌套块恢复旧行为，嵌套次序那 1 个变红；嵌套时不按比例缩，第 5 个变红。5 个测试每个都至少被一处改坏变红。改坏点已删。
- 样例重新对图（SB-2b 的发行版，对同一个编译器编的基线）：两边各 17,627 个元素，按画的先后逐个比 0 处不同，笔 14 种相同；SVG 逐字节相同（14,769,401 字节），连出 5 次都一样。SB-2 的程序上那 220 段同笔的先后不同也没有了。
- 全部样例对图：`pid-test-data` 下 152 个 `.pid`，1 个两边都打不开（`backup-plant-data\test\U01\D03.pid`），其余 151 个共 675,960 个元素，0 段先后不同，150 个 SVG 逐字节相同；1 个少一笔，原因在出图的窗口过滤，见 SB-2c。
- 影响面（按规则推算，不是逐个出图）：151 个文件的 8,433 个定义里，带填充的 174 个，带填充又带圆、圆弧或椭圆的 164 个。改之前先后会被换的是 38 个定义、142 个引用、35 个文件，全是 `Set Fire Point`（35 个定义、139 个引用）和 `Set Fire Point_2`（3 个、3 个），都是轮廓画到了填充上面，没有曲线被填充盖住的。推算和实际出图核对过三张图：样例 1 处，另一张 3 处，还有一张 0 处，都对得上。
- 没验证的：没有在程序界面里看。改动对所有图块生效，DWG、DXF 图纸上的块没有做改前改后的出图对比，只有「全量测试没有变红」这一条。
- 证据：`D:\scratch\ocs-symblock\sb2b\`（`red.log`、`quicka.txt`、`chainfinal.txt`）；`D:\scratch\ocs-symblock\cp1\out\`（改之前：`order-base-branch-v2.txt`、`try-old.plotcmp.jsonl`；改之后：`compare-base-branch2b.txt`、`order-base-branch2b.txt`、`count-branch2b*.txt`、`branch2b*.svg`、`plotcmp-all-2b.txt`）。对图脚本在 `D:\scratch\ocs-symblock\cp1\`（`cp1.py`、`order.py`、`plotcmp.py`）。

### SB-2c 【接线】出图按窗口筛图元时，贴在窗口边上的不丢

后来加的任务（计划 §9）。检查点一给全部样例对图时查出来的，改的是出图代码，不是导入器。

- **做什么**：出图先按窗口筛一遍图元（`src/app/update/file.rs` 的 `area_plot_job`），比的是图元的外框和窗口，两边都取成单精度。同一个块的第二个及以后的引用是照第一个（原件）挪过去的副本：笔画的点挪得准，外框却是按单精度相加挪的（`src/scene/cache/block_cache.rs` 的 `translated_prototype_wire`），会比它的笔画偏出一个舍入误差。笔画正好贴在窗口边上时（按范围出图时最靠外的那一笔，或窗口正好框在一条线上），就被判成在窗外丢掉。改成：外框够得着窗口的照旧留下；外框判在窗外的，按它真正画的坐标（笔画的点、填充的三角形、文字的字形，双精度）再量一次，真在窗外才丢。窗口的边本身也是算出来的，所以留一点余量：窗口四个数里绝对值最大的那个（不小于 1）的十亿分之一。
- **验收**
  - [x] 笔画贴在窗口的左、右、下、上四条边上，外框偏到窗外一个单精度步长：都出图。
  - [x] 笔画比窗口边偏出 1e-12：出图。点的低位算在坐标里。路径中间有断点（NaN）时照样量。
  - [x] 填充、文字够得着窗口边而外框差一步的：出图。
  - [x] 真在窗外的照旧不出：左、右、下、上各偏出 0.001 的笔画，带断点而整条在窗外的路径；不画任何东西的图元只看外框。
  - [x] 端到端：一个块两个引用，摆在出问题那张图上的坐标，按范围出图，两个引用都画出块里的两笔。
  - [x] 既有测试没有因此变红的。
  - [x] 故意改坏：只看外框（旧行为）、`area_plot_job` 不调新函数、不留余量、余量放大一百万倍、不算低位、不算笔画的点、不算填充、不算文字、四条边各少比一条、断点当成在窗外、断点当成在窗内、不画东西的当成在窗内、全留、全丢，每处都有测试变红。
  - [x] 全部样例重新对图：那张图不再少一笔；能出图的 151 张与基线逐字节相同。用这个提交编的发行版量，结果记在检查点一。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb2c -Tag final -Files src/app/update/file.rs -Quick plot_window_tests`；对图用 `D:\scratch\ocs-symblock\cp1\` 下的 `plotcmp.py`（全部样例）和 `plotwin.py`（一张图按范围和按大一圈的窗口各出一次）。
- **依赖**：SB-2b（它之后全部样例才只差这一笔）。
- **文件**：`src/app/update/file.rs`（新函数 `plot_wire_in_window`，`area_plot_job` 里一处调用，测试模块 `plot_window_tests`）。
- **规模**：S。

**做完**（2026-11-19）

- 怎么查出来的：SB-2b 之后给 `pid-test-data` 下全部样例对图，能出图的 151 张里 150 张与基线逐字节相同，1 张少一笔：`集注站 放空火炬系统工艺及自控流程图.pid`，SVG 里的 `M121.72,2411.70L121.72,2400.36`。这张图上贴着图面左边的图例有三个，少的是 `Off-Unit SQ` 那个的左边线；它的块在这张图上不止一个引用。出过的 5 次都少这一笔（SB-2b 的程序 4 次，SB-2 的程序 1 次）。把出图窗口每边放大 5 再出，就不少了（两边各 5,878 个元素，先后相同），所以是按窗口筛的那一步丢的。再往下读代码，就是上面「做什么」写的原因。
- 先红：红跑了两次，当时有 7 个测试，函数里是旧写法（只比外框）。第一次（`red.log`）6 个失败、1 个通过，但端到端那个失败的原因不对，不算数：我按名字数出图的图元，期望每个引用 2 个，实际 3 个，因为每个块引用另带一个不画东西的标记图元。改成比真正画出的笔画后重跑（`red2.log`）：6 个失败、1 个通过，端到端的读数是副本只画出底边一笔，少了左边那一竖。通过的那 1 个是「真在窗外的不出」，改前改后都该过，不算先红，靠故意改坏证明管用。「路径中间有断点」那个测试（第 8 个）和「真在窗外」里带断点的那一条，是和实现一起加的，没有在旧写法上红跑过；「只看外框」这处改坏就是旧写法，它让断点那个测试变红。
- 故意改坏 17 处（`quicka.txt`），每处都有测试变红，8 个测试每个都至少被一处改坏变红：只看外框、全丢，各 7 个变红；不算笔画的点，5 个；`area_plot_job` 不调新函数，端到端那 1 个；不算低位、不算填充、不算文字、不留余量、断点当成在窗外，各是对应的那 1 个；全留、余量放大一百万倍、断点当成在窗内、不画东西的当成在窗内、四条边各少比一条，都是「真在窗外的不出」变红，少比哪条边就报哪条边。改坏点已删，删完的文件和加改坏点之前的逐字节相同。
- 读数：新增测试 8 个。全量 7,105 个：通过 7,052，忽略 53，失败 0（上一个提交的 7,044 加新增的 8）。clippy 656 / 1,051，与上一个提交相同；点到 `file.rs` 的那几条都不在这次动的行上。`file.rs` 是 `new=0`。分层报告与上一个提交逐行相同。改动 +318 −6，其中测试 271 行。
- 既有测试：没有一个因此变红。
- 会变的和不会变的：外框够得着窗口的，判法和以前一字不差，所以以前留下的现在都留下，只会多留、不会多丢。普通图元和块的原件，外框是按自己的点算的（`block_cache.rs` 的 `Batches::finalize`、`tess.rs` 的 `set_wire_aabb`），取成单精度不改变大小次序，外框在窗外就说明点也在窗外，结果和以前一样；这一条是读代码得出的。会变的只有块的副本。多留下的若确实有一点在窗外，出图的剪裁照样裁掉。填充图案、遮罩、图片本来就不过这道筛，没动。
- 没验证的：真实图纸上按窗口出图（只按范围出过）；出 PDF（对图用的是 SVG）；没有在程序界面里看；DWG、DXF 图纸上的块没有做改前改后的出图对比，只有「全量测试没有变红」这一条。用这个提交的发行版给全部样例重新对图还没做（上面没勾的那一条），在检查点一做。
- 证据：`D:\scratch\ocs-symblock\sb2c\`（`red.log`、`red2.log`、`quicka.txt`、`chainfinal.txt`）；`D:\scratch\ocs-symblock\cp1\out\`（`plotcmp-all-2b.txt`、`plotcmp-flare.txt`、`plotcmp-flaresb2.txt`、`plotcmp-flarenostub.txt`、`plotwin-flare.txt`、`edge-flare-b.txt`）。
- 补记（2026-11-20）：上面当时没勾的那一条，用 `12ec8131c` 的发行版量过了，已勾上：那张图按范围出图，基线和分支各 5,878 个元素，只在一边出现的 0 个，先后相同；能出图的 151 张都与基线逐字节相同。读数和证据在检查点一。

### 检查点一（SB-2 之后）

- [x] SB-1、SB-2 的门禁都过。
- [x] 用分支编出的发行版，无界面方式导入样例（只带 `.pid`）：带 `role=symbol` 的块引用 592 个。
- [x] 样例出的 SVG 与基线程序（`D:\scratch\ocs-symblock\baseline\OpenCADStudio.exe`）出的相比：图形元素个数相同；坐标按 0.01 取整后，只在一边出现的元素 0 个。
- [x] 不一致就留在 SB-2 查，不往后做。结果发给用户，不等回复。

**做完**（2026-11-20）

- 经过：第一次对图用的是 SB-2 的程序，按上面两条的比法是过的。多比了一项「按画的先后逐个比」，又给 `pid-test-data` 下全部样例对了图，查出两处与基线不一致，原因都不在导入器；加了 SB-2b、SB-2c 两个任务修掉，修完才往后做。下面的读数是 11-19 23:46 到 11-20 00:01 用最后一个提交 `12ec8131c` 的发行版量的。
- 门禁：SB-1、SB-2、SB-2b、SB-2c 四个提交的门禁都过，读数在各任务的「做完」段。`12ec8131c`：全量 7,105 个，通过 7,052，忽略 53，失败 0；clippy 656 / 1,051；分层报告里 `io` 开头的行与基线相同。
- 用的程序：分支是 `12ec8131c` 的发行版（`cp1\branch\OpenCADStudio.exe`，206,088,704 字节），编的时候工作区与这个提交没有差别。基线有两个。上面写的 `baseline\OpenCADStudio.exe` 和分支不是同一个编译器编的（从 exe 里读出的编译器提交号是 `58cf8f9c…`，分支是 `65ce8611…`），拿它比用时不公平；所以用同一个编译器从分支起点 `5171d197b` 另编了一个（`cp1\base-same\OpenCADStudio.exe`）。下面没有另外说明的，都是和后一个比的。
- 样例（只带 `.pid`，无界面方式）：模型空间里块引用 592 个，全是 `role=symbol`，592 个都带 `symbol=`；它们指向 119 个定义，都叫 `SPPID_…`；模型空间里 `role` 为 `symbol`、`symbol-label` 的散图元 0 个。定义里共 971 个图元（`symbol` 828、`symbol-label` 119、`fill` 24），每个只带 `role=` 一个值。模型空间其余 5,798 个散图元：`geometry` 2,471、`text` 958、`point-ok` 1,864、`point-warning` 430、`frame` 1，个数与基线相同；`fill` 74，是基线的 167 个减去图例里的 93 个（规格 §2.2）。
- 样例对图：两边各 17,627 个图形元素；坐标按 0.01 取整后只在一边出现的 0 个；按画的先后逐个比 0 处不同；笔 14 种，相同。两个基线程序和分支出的 SVG 三份逐字节相同：都是 14,769,401 字节，SHA-256 都是 `E0B5E2E4…`。
- 全部样例对图（`pid-test-data` 下 152 个 `.pid`，按范围出图）：1 个两边都打不开（`backup-plant-data\test\U01\D03.pid`），其余 151 个共 675,960 个元素，151 个 SVG 都与基线逐字节相同。SB-2b 的程序少一笔的那张图：按范围出图，基线和分支各 5,878 个元素，只在一边出现的 0 个，先后相同。
- 导入用时（两个程序交替，各开 15 次，每次一个新进程，量的是整条 `OPEN` 命令，取中位数）：

  | 分支的程序 | 基线 | 分支 | 比值 |
  |---|---|---|---|
  | SB-2 | 2,063 ms | 2,130 ms | 1.032 |
  | SB-2b | 1,854 ms | 1,914 ms | 1.032 |
  | SB-2c，第一遍 | 1,974 ms | 2,148 ms | 1.088 |
  | SB-2c，第二遍 | 2,138 ms | 2,167 ms | 1.013 |

  四次都在 1.1 倍以内（规格 §6 第 11 条）。同一个基线程序四次的中位数从 1,854 到 2,138 ms 不等，可见读数随机器当时的状态变：别的窗口也在这台机器上编译。SB-2c 第一遍从第 10 轮起两边都变慢；第二遍开始时没有编译在跑（处理器占用 14%），结束时有（65%）。同一个程序的单次读数从 1.8 秒到 2.4 秒都有，最慢的一次 2.96 秒。SB-2c 两遍的 30 轮合起来：基线 2,040 ms，分支 2,155 ms，1.056。SB-2b、SB-2c 改的是显示和出图，没有动导入的代码，所以四个比值的差别我也归到机器状态上，这一点没有另外验证。按这个抖动，「3 次取中位数」不够稳：SB-9、SB-10 量的时候基线和分支交替着多量几轮，并记下机器当时在跑什么（记进了规格 §11）。
- 没验证的：没有在程序界面里看；带 `_Data.xml` 的导入、另存 DWG 都还没量（SB-9、SB-10）。
- 证据：`D:\scratch\ocs-symblock\cp1\out\`（`count-branch2c.txt`、`compare-basesame-branch2c.txt`、`order-basesame-branch2c.txt`、`cp1-2c.done`、`compare-base-basesame.txt`、`rustc-of-2c.txt`、`plotcmp-all-2c.txt`、`plotcmp-all-2c.done`、`plotwin-flare-2c.txt`、`time-basesame-branch-2.txt`、`time-basesame-branch2b.txt`、`time-basesame-branch2c.txt`、`time-basesame-branch2c-2.txt` 和它的 `.done`）；发行版的编译记录 `cp1\build-sb2c.done`；脚本在 `cp1\`（`cp1.py`、`order.py`、`plotcmp.py`、`plotwin.py`、`run2c.ps1`、`runtime.ps1`）。

---

## 第二段：跟着改的几处

### SB-3 【测试】选择和编辑按整个图例

- **做什么**：在程序一层加测试：用导入器建一张有几个图例的图，设好视口，点选、悬停、框选，再跑移动、复制、删除、撤销、炸开。预计不改产品代码；测试查出问题就在这个任务里修，并记进规格 §11。
- **验收**
  - [x] 点选：在图例的一条线上点一下，选择集里只有这个图例的块引用；点在它的圆上也一样；点在旁边另一个图例上选中的是另一个。
  - [x] 悬停：鼠标停在一笔上，高亮的对象是这个块引用。
  - [x] 框选：从左到右框住整个图例选到 1 个对象；从右到左只碰到一笔也选到这 1 个对象。
  - [x] 符号名图层关着时，框住图例看得见的部分就能选中，看不见的符号名不算在范围里。
  - [x] 只画符号名的图例：符号名图层关着时点不到、框不到；打开后能选中。
  - [x] 移动：引用的插入点变了，定义没变，记录还在。复制：多出一个带同样记录的块引用。删除后撤销：引用回来，记录还在。
  - [x] 炸开：变回散图元，散图元上只有 `role=`。
  - [x] 故意改坏：让导入器把笔画留成散图元，点选和框选的测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb3 -Files <测试文件> -Quick <测试名>`。
- **依赖**：SB-2。
- **文件**：程序一层的一个测试文件（放在哪个模块，开工时看现有测试的摆法再定）。
- **规模**：M。

**做完**（2026-11-20）

- 放在哪：程序一层新加 `src/app/pid_symbol_block_tests.rs`，13 个测试。测试用的图是导入器自己的步骤建的，连 `finish` 也走：`src/io/pid/tests.rs` 的 `sheet_of_two_valves_a_note_and_a_pipe`，同一个阀放两处，一个只画符号名的图例，一条不属于任何图例的管线，都在图纸图层 `Process`。它经 `src/io/pid/mod.rs` 里一行 `#[cfg(test)] pub(crate) use` 交给程序一层，`src/app/mod.rs` 加一行声明测试模块。发给程序的都是它平时收到的消息：鼠标移动、按下、松开，悬停和捕捉两个计时器到点的那一下，图层管理器的灯泡，Esc，命令行。设置是程序的默认值，只有对象捕捉那个测试把常开的捕捉设成端点和圆心。
- 产品代码没有改，测试没有查出程序的问题。
- 清单各条由哪个测试管：点选 `a_click_on_a_stroke_selects_the_symbol_and_nothing_else`；悬停 `the_rollover_highlights_the_symbol_whole`；框选 `a_selection_box_takes_the_symbol_as_one_object`（多量了清单没写的两条：窗口只框住半个图例时选不到；窗口框住整张图时每个图例算 1 个）；符号名图层关着 `a_hidden_name_is_no_part_of_what_is_selected`；只画符号名的图例 `a_symbol_that_is_only_its_name_is_selected_once_names_are_shown`；移动、复制、删除后撤销各 1 个；炸开 `an_exploded_symbol_is_loose_entities_that_say_only_their_role`。
- 比清单多的 4 个：规格 §4.1 里「旋转、镜像按整个图例」「夹点只有插入点一个」「改之前能捕到的图例笔画的端点、圆心，改之后还能捕到」这三句，清单和后面的任务里都没有对应的测试，在这里补上：旋转、镜像、夹点（`a_selected_symbol_has_one_grip_at_its_insertion_point`）、对象捕捉（`the_strokes_of_a_symbol_keep_their_object_snaps`）各 1 个。
- 第一次跑不过的有两处，都是测试写错：
  - 头 9 个测试第一次跑，8 个通过、1 个失败（`quick-q1.log`）。失败的是「符号名图层关着」：我在符号名的位置上点了一片格子，格子从 x=121 起，离图例的线头（x=120）只有 1 mm；点选够得着 8 个像素，在测试的缩放下是 2 mm，所以点到的是线头。格子改成从 x=124 起；`opened()` 里加了一条断言钉住缩放（1 mm 是 4 个像素）。
  - 后加的 4 个第一次跑，对象捕捉那个失败（`quick-q4.log`）：第二处读到的还是第一处的结果。命令进行中第一次移动鼠标当场算捕捉，之后的移动要等鼠标停住 60 ms（`SNAP_DWELL_MS`）才重算，测试没等这一下。把笔画留成散图元时同样失败（`quick-q4loose.log`），所以与成块无关。测试里把停留时间拨过去、发一次 `SnapDwellTick` 之后通过。
- 故意改坏 4 处，用环境变量打开，一次编译跑 5 遍（`quick-q6*.log`）。清单要的是第一处；后三处是为了让第一处照不到的断言也红一次。

  | 改坏 | 13 个里变红 | 红在哪 |
  |---|---|---|
  | 导入器把笔画留成散图元 | 12 | 点选、悬停、框选、符号名图层关着、夹点 5 个红在各自的断言上：点选和悬停读到的是一条线，框选读到 3 个散图元，夹点读到线的 3 个。另外 7 个红在「找不到这个图例的块引用」。没红的是对象捕捉 |
  | 块里的图元把整条记录都带上 | 1 | 炸开：散图元上多出 `sheet_layer=`、`sheet_layer_oid=`、`extent=` |
  | 符号名也放到作者的图层 | 4 | 只画符号名的图例、符号名图层关着、框选、炸开 |
  | 块里的图元不给预先算好的捕捉点（改在 `block_cache.rs` 的 `append_defn_entity`） | 1 | 对象捕捉：鼠标在圆心附近时捕不到圆心 |

  改坏点已删：动过的 `src/io/pid/build.rs`、`src/scene/cache/block_cache.rs` 与上一个提交的逐字节相同，源码里没有剩下开关。改坏的内容留在 `mutants.diff`。这一轮之后测试文件只改了文件头的注释。
- 没有红过的，和只是顺带红的：
  - 对象捕捉那个测试在第一处改坏下通过，这是它该有的结果：散图元本来就捕得到，它量的是成块前后一样。它的 6 条断言里，因为程序的原因红过的只有「鼠标在圆心附近捕到圆心」一条。端点的两条在第四处改坏下照样通过：端点不靠预先算好的点，是从画出的线上现算的。「鼠标在圆周上捕到圆心」「另一个阀的圆心」「空白处捕不到」三条排在红的那条后面，没有跑到。
  - 移动、复制、删除后撤销、旋转、镜像 5 个没有专门的改坏点，只在第一处改坏下因为找不到块引用而变红。它们钉的是程序现有的命令对带记录的块引用的做法；没有去改这些命令的代码试它们会不会红。
- 测试钉住的行为：
  - 选中一个图例，夹点 1 个，在插入点；留成散图元时点同一处，得到的是那条线的 3 个夹点。
  - 符号名图层关着：窗口框住看得见的笔画就选中；符号名的位置上点不到，交叉框也碰不到。打开之后符号名算进图例的范围：原来那个窗口框不全，选不到，放大到包住符号名才选中；在符号名上点、用交叉框碰，选中的都是这个图例。
  - 只画符号名的图例，引用在 `PID-SYMBOL-LABEL` 层：关着时点不到、交叉框碰不到，窗口框住整张图也不含它；打开后三样都选中。
  - 移动 (0, 40)：插入点跟着变，定义、记录、块定义的个数不变，旁边的阀不动。复制、镜像（不删源对象）：多 1 个引用，同一个块、同样的记录，不多出定义。绕插入点转 90°：插入点不变，引用的转角是 π/2，定义和记录不变。删除后撤销一步：引用回来，句柄和记录与原来相同，模型空间里别的东西没变。
  - 炸开：得到 2 条线、1 个圆、1 个符号名文字，位置就是图例画的位置。笔画落在引用所在的图层 `Process`（在块里是 0 层），符号名在 `PID-SYMBOL-LABEL`；每个只带 `role=`，引用和它的记录没有了。旁边的阀还是一个整体。
- 读数：新增测试 13 个。全量 7,118 个：通过 7,065，忽略 53，失败 0（上一个提交的 7,052 加新增的 13）。clippy 656 / 1,051，与上一个提交相同；点到 `src/app/mod.rs`（4 条）、`src/io/pid/mod.rs`（1 条）的都不在这次动的行上，两个测试文件上没有。4 个文件都是 `new=0`。分层报告与上一个提交逐行相同。改动 +946 −1，全是测试和只在测试里编译的两处接线。
- 没验证的：测试把图装进标签页用的是无界面打开在装入文档之后的那几步，没有走界面上打开文件的那条路（`finalize_loaded_outcome`）；没有在程序界面里看，真程序上的点选、框选留给 SB-10。测试图只有一种画法、一个图纸图层，没有带填充的图例，没有转过或镜像过的摆放。对象捕捉只量了端点和圆心；镜像只量了不删源对象；旋转只量了绕自己的插入点转 90°。规格 §4.1 里「在块编辑器里改定义，共用它的图例一起变」没有测。成功标准第 5 条里「移动图例后符号名跟着走」这里没有量，放到 SB-4。
- 证据：`D:\scratch\ocs-symblock\sb3\`（`quick-q1…q6*.log` 和 `.done`、`mutants.diff`、`chainfinal.txt`）。

### SB-4 【测试】显示和视图过滤

- **做什么**：用测试钉住规格 §4.3：图纸图层和角色的开关对引用和块内图元各设什么，图层视图的行和数字，开关之后块重画，隐藏图层上的图例，块内 0 层图元的取色。预计不改产品代码。
- **验收**
  - [x] 关掉一个图纸图层：这一层的引用是 `invisible`，块内图元不变；再打开恢复。
  - [x] 关掉角色 `symbol`：引用和块内笔画是 `invisible`。关掉 `fill`：只有块内的填充是。关掉 `symbol-label`：只有块内的符号名是。
  - [x] 图层视图（`PidViewSummary`）：图纸图层那几行一个图例算 1 个；角色那几行的数字等于引用数加各定义里的图元数；`symbol`、`symbol-label`、`fill` 三行都在。
  - [x] 打开一个隐藏的图纸图层：引用亮起来，它所在的图层（同名图层或 `PID-HIDDEN`）被打开，与现在对散图元的做法相同。
  - [x] 显示：块定义的显示缓存里没有 `invisible` 的图元，没有所在图层关着的图元；开关之后按程序现有的流程（`after_pid_view_switch`）刷新，缓存跟着变。
  - [x] 隐藏图层上的图例，打开 `PID-SYMBOL-LABEL` 图层后仍然什么都不显示。
  - [x] 取色：块内 0 层、颜色随层的笔画，画出来的颜色是引用所在图层的颜色；自己带颜色的笔画颜色不变。
  - [x] 符号名图层打开后移动一个图例，符号名跟着走（成功标准第 5 条的末句；SB-3 做完时补进来的）。
  - [x] 故意改坏：块内图元不写 `role=`、引用不写 `sheet_layer=`，对应的测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb4 -Files <测试所在的文件> -Quick pid_view_filter`。
- **依赖**：SB-2。
- **文件**：`src/io/pid_view_filter.rs`（测试）、`src/io/pid/tests.rs`（测试），显示那两条放在显示缓存或程序一层的测试里。
- **规模**：M。

**做完**（2026-11-20）

- 放在哪：18 个测试，分三层。
  - 过滤器一层 3 个，在 `src/io/pid_view_filter.rs` 的测试模块里。图在代码里直接拼：一个块，里面两笔 `role=symbol`、一个 `role=fill`、一个 `role=symbol-label`；两个引用分别在图纸图层 `Process`、`Utility`；一条管线。
  - 导入器一层 4 个，在 `src/io/pid/tests.rs` 的 `placements_as_blocks` 里。图由导入器的记录循环建：同一个阀放三处（两处 `Process`、一处 `Instrument`），一个只画符号名的图例，一条管线。隐藏图层那个测试另建一张，连 `finish` 也走。
  - 程序一层 11 个，新文件 `src/app/pid_symbol_view_tests.rs`。图是 `src/io/pid/tests.rs` 里新加的 `sheet_of_valves_on_shown_and_hidden_layers`，连 `finish` 也走：同一个阀放四处，分别在 `Process`、`Instrument`（文件显示的两个图层）、`Demolition`（文件隐藏的图层）和一个没有名字的隐藏图层上，另有一条 `Process` 上的管线。阀画四样：左边一段线（没有自己的样式，颜色随层）、一个圆（自己带颜色，黑）、右边一段线（自己带颜色，绿）、一个红色填充。发给程序的是图层管理器平时发的消息：图纸图层视图里图纸图层一行、角色一行的开关，图层表里的灯泡和取色；移动走命令行的 `MOVE`。
  - 接线：`src/io/pid/mod.rs` 那行 `#[cfg(test)] pub(crate) use` 多带一张图；`src/app/mod.rs` 加一行声明测试模块；SB-3 的 `src/app/pid_symbol_block_tests.rs` 里 3 个常量、6 个帮手改成 `pub(super)`，`opened()` 拆出 `opened_with`，两个测试文件共用。
- 产品代码没有改，测试没有查出程序的问题。原有测试的断言一条没动：`src/io/pid/tests.rs` 减掉的 12 行，是 SB-3 那张图里直接调 `finish` 的几行换成了共用的 `finished`；`src/io/pid_view_filter.rs` 减掉的 13 行，是帮手 `line_with` 拆出 `saying`、一个测试里的局部 `rows` 提到模块一级。
- 简称：
  - 过滤器一层：F1 `a_block_reference_answers_to_its_sheet_layer_and_the_members_to_their_roles`，F2 `the_summary_counts_a_reference_on_its_sheet_layer_and_the_members_under_their_roles`，F3 `lighting_a_sheet_layer_releases_the_layer_its_block_reference_is_on`。
  - 导入器一层：I1 `a_sheet_layer_switched_off_darkens_its_references_and_no_member`，I2 `a_role_switched_off_darkens_the_members_that_state_it`，I3 `the_sheet_layer_view_counts_a_symbol_once_and_its_members_by_role`，I4 `a_hidden_sheet_layer_switched_on_lights_its_reference_and_turns_its_layer_on`。
  - 程序一层：T1 `the_sheet_opens_with_the_symbols_of_its_displayed_layers_drawn`，T2 `the_layer_manager_counts_a_symbol_once_and_its_parts_by_role`，T3 `a_sheet_layer_switched_off_takes_its_symbol_out_whole`，T4 `the_role_of_fills_switched_off_leaves_the_outlines`，T5 `the_role_of_symbols_switched_off_takes_every_symbol_out`，T6 `the_names_show_with_their_layer_and_go_with_their_role`，T7 `the_names_layer_shows_nothing_of_a_symbol_on_a_hidden_layer`，T8 `a_hidden_sheet_layer_switched_on_shows_its_symbol`，T9 `a_symbol_on_the_hidden_layer_with_no_name_shows_with_the_layer_that_holds_it`，T10 `a_stroke_by_layer_takes_the_colour_of_the_layer_its_symbol_is_on`，T11 `the_name_and_the_fill_move_with_their_symbol`。
- 清单各条由哪个测试管：
  - 关图纸图层：F1、I1（引用是 `invisible`，块内图元不变，再打开恢复），画面是 T3。
  - 关角色：F1、I2；画面上 `symbol` 是 T5，`fill` 是 T4，`symbol-label` 是 T6。
  - 图层视图：F2、I3、T2。
  - 打开隐藏的图纸图层：F3（引用在 `PID-HIDDEN` 上、在同名图层上各一例）、I4，画面是 T8。
  - 显示：没有去读缓存里存了哪些图元，读的是画出来的结果（见「程序一层读的是什么」）。「缓存里没有 `invisible` 的图元」「没有所在图层关着的图元」「开关之后刷新」各有一处改坏对应（下表第 5、6、10 行）。
  - 隐藏图层上的图例，打开 `PID-SYMBOL-LABEL` 后仍然什么都不显示：T7。
  - 取色：T10；没有名字的隐藏图层上的阀取 `PID-HIDDEN` 的颜色，在 T9。
  - 移动之后符号名跟着走：T11。
  - 故意改坏：下表第 1、2 行。
- 比清单多的：T1（刚打开时的画面）、T9（没有名字的隐藏图层上的图例，规格 §3.1 的最后一种情况；SB-2 测的是结构，这里补画面）。规格 §7 的表里过滤器一层还列了「过滤状态存取往返」，没有另加测试：`PID_VIEW_FILTER` 记录里只有图层名和角色名，与图元无关，原有的 `a_stored_filter_reads_back_and_an_empty_one_leaves_no_record` 管它；在有块的图上，T3、T8 断言的视图那一行的开关，就是从存下的记录读回来的（`PidViewSummary::of`）。
- 程序一层读的是什么：
  - 线和文字，读编辑器画图和点选用的那一份（`scene.entity_wires()`），两种读法。按对象读：一个块引用名下画了什么，同时核对出图那一份（`scene.plot_wire_groups`）取的与它相同。按位置读（`drawn_around`）：一个阀的位置周围画出来的全部，不管是哪个对象画的；这种读法没有和出图那一份核对。
  - 填充，读出图取的那一份（`scene.paper_plot_hatches()`），同时核对点选能取到的那一份（`scene.insert_hatches_for_click()`）与它相同。窗口自己拿到的那份填充清单是场景私有的，没有读。
  - 图层视图的行，读图层管理器面板自己的数据：名字、数字、开关。
- 第一次跑：
  - 过滤器、导入器两层的 7 个第一次跑就通过（`quick-e1.log`）。它们量的是程序现有的做法，会不会红看下面的改坏。
  - 程序一层的 11 个第一次跑，1 个通过、10 个失败（`quick-v1.log`），都红在帮手 `fills()` 里「点选那份填充与出图那份相同」的核对上：点选那份不带图层名，我拿它和带图层名的出图那份逐字比。改成点选那份按画它的块引用所在的图层叫名之后通过。帮手写错，不是程序的问题。
  - 改坏跑了四轮，头两轮之后各改过测试的读法：T7、T3、T11 原先只按对象读，把符号名留成散图元时，它们只红在前置条件上（阀名下没有符号名），没有跑到要钉的那一条；即使跑到，由别的对象画出来的符号名它们也看不到。加了按位置读的 `drawn_around`，「什么都没画」和移动前后的断言改用它。第三、四轮用的是同一份测试文件，第四轮多了后加的两处改坏（第 8、14 行）；下表是第四轮的结果。
- 故意改坏 14 处，用环境变量打开，一次编译跑 15 遍：开关不开的一遍，47 个全过；每处各一遍（`quick-m0.log`、`quick-m-*.log`，汇总在 `mutant-table-round4.txt`）。47 个是四组：过滤器一层 9 个、`placements_as_blocks` 14 个、这里程序一层的 11 个、SB-3 的 13 个。清单要的是头两处；其余是为了让头两处照不到的断言也红一次。18 个新测试每个至少红过一次。

  | # | 改坏 | 改在哪 | 47 个里变红 | 新 18 个里变红的 | 程序一层红在哪 |
  |---|---|---|---|---|---|
  | 1 | 块内图元不写 `role=` | `src/io/pid/build.rs` | 9 | I2 I3 T2 T4 T6 | T2 角色行的数字；T4、T6 视图里没有 `fill`、`symbol-label` 这一行 |
  | 2 | 引用不写 `sheet_layer=` | 同上 | 9 | I1 I3 I4 T2 T3 T8 | T2 图纸图层行的数字；T3 关掉之后阀还在；T8 视图里没有 `Demolition` 这一行 |
  | 3 | 没有名字的隐藏图层上的图例，符号名留在 `PID-SYMBOL-LABEL` 层 | 同上 | 3 | T2 T9 | T2 角色行的数字（两个定义并成了一个）；T9 打开 `PID-HIDDEN` 后没有符号名 |
  | 4 | 符号名不进块，留成散图元 | 同上 | 21 | I1 I2 I3 T2 T3 T5 T6 T7 T9 T11 | T3 关掉图纸图层后原处还剩符号名；T5 关掉角色 `symbol` 后还剩符号名；T6、T9 阀名下没有符号名；T7 隐藏的阀的位置上读到符号名；T11 移动后新位置周围没有符号名；T2 的数字 |
  | 5 | 块的显示缓存不跳过 `invisible` 的图元（`append_defn_entity`） | `src/scene/cache/block_cache.rs` | 1 | T6 | 关掉角色 `symbol-label` 后符号名还在 |
  | 6 | 块的显示缓存不跳过所在图层关着的图元（同一个函数） | 同上 | 7 | T1 T4 T6 T8 T10 | 符号名图层关着，符号名也画了出来 |
  | 7 | 0 层、颜色随层的块内图元不取引用所在图层的颜色（`resolve_wire_color`） | 同上 | 2 | T9 T10 | 左边那段线还是原来的颜色 |
  | 8 | 自己带颜色的块内图元也取引用所在图层的颜色（同一个函数） | 同上 | 9 | T1 T3 T4 T6 T7 T8 T9 T10 T11 | T1 右边那段线不是绿的；T10 换色后圆和右边那段线也变蓝；T3、T7、T11 红在前置条件上 |
  | 9 | 填充那条路照走 `invisible` 的块内图元 | `src/scene/render_graph.rs` | 1 | T4 | 关掉角色 `fill` 后填充还在 |
  | 10 | 开关之后不告诉场景图元变了（`after_pid_view_switch` 不调 `bump_geometry`） | `src/app/layers.rs` | 5 | T3 T4 T5 T6 T8 | 开关之后画面没有变；T4 红在填充的两份不一致 |
  | 11 | 过滤器不管不带 `sheet_layer=` 的对象 | `src/io/pid_view_filter.rs` | 6 | F1 I2 T4 T6 | 关掉角色后填充、符号名还在 |
  | 12 | 图层视图数角色时只数带 `sheet_layer=` 的对象 | 同上 | 6 | F2 I3 T2 T4 T6 | T2 角色行的数字；T4、T6 视图里没有那一行 |
  | 13 | 打开图纸图层时不开图层表里的图层 | 同上 | 4 | F3 I4 T8 | T8 图层 `Demolition` 没有被打开 |
  | 14 | 过滤器只会关：已经暗的对象不再亮 | 同上 | 12 | F1 F3 I1 I2 I4 T3 T4 T5 T6 T8 | T3、T4、T5、T6 红在「再打开，画面与之前相同」；T8 红在图层没有被打开 |

  改坏点已删：动过的 5 个产品文件里，`build.rs`、`block_cache.rs`、`render_graph.rs`、`layers.rs` 与上一个提交逐字节相同，`pid_view_filter.rs` 的改动全在测试模块里；源码里没有剩下开关。改坏的内容留在 `mutants.diff`。
- 没有单独红过的，和只是顺带红的：
  - 每个测试只红在第一条不成立的断言上，排在它后面的没有跑到。红过的：T1 的第一条（`Process` 上的阀名下画了什么）；T2 的两条；T3 的「关掉之后原处什么都不剩」和「再打开，画面与之前相同」；T4 的「填充没有了」「轮廓还在」和再打开；T5 的「每个阀的位置上什么都不剩」和再打开；T6 的打开符号名图层之后、关掉角色之后、再打开三处；T7 的「隐藏图层 `Demolition` 上的阀的位置上什么都没画」；T8 的「图层被打开」和「阀画出来了」；T9、T10 各自的第一条画面断言；T11 的「新位置周围画的是笔画加符号名」。
  - 没有红过的：T1 里隐藏的两个阀的位置上什么都没画；T7 里没有名字的隐藏图层那个阀的位置、两处点不到隐藏的符号名、点得到画出来的符号名；T8 里再关掉之后的两条；T10 里填充颜色不变的那几条（填充走另一条路，第 8 处改不到它）；T11 里移动之后的后五条（按对象读、原处什么都不剩、填充跟着走、两次点选）。
  - 第 2、3 处改坏下 T7 照样通过。原因是这个程序里，引用所在的图层关着时整个引用不画，块里在别的开着的图层上的图元也不画。隐藏的命名图层上的图例有两道保险：引用是 `invisible`，引用所在的图层也关着；去掉一道还有另一道。两道都去掉的情况没有试。
- 测试钉住的行为：
  - 关掉图纸图层 `Process`：这一层的引用和管线是 `invisible`（导入器一层那张图上 4 个对象），块里的图元一个不变，图层表不动（`released_layer` 是空，同名图层的灯泡还亮）。画面上这一层的阀整个不见——笔画、符号名、填充——管线也不见；另一层上画同一个定义的阀照常。再打开，画面与关之前相同。
  - 关掉角色 `symbol`：引用和定义里的笔画、字母变暗（导入器一层那张图上 7 个对象），填充和符号名的 `invisible` 不变；画面上每个阀整个不见，因为引用带这个角色，管线还在。关掉 `fill`：只有定义里的填充变暗（1 个），画面上填充不见、轮廓还在。关掉 `symbol-label`：只有各定义里的符号名变暗（2 个），画面上符号名不见，符号名图层的灯泡还亮着。
  - 图层视图的数字，程序一层那张图：图纸图层行是 `Demolition` 1（关）、`Instrument` 1、`Process` 2；没有名字的隐藏图层上的阀不带 `sheet_layer=`，没有行。角色行是 `geometry` 1、`symbol` 10（4 个引用，加两个定义各 3 笔）、`symbol-label` 2、`fill` 2。这张图上同一个阀有两个定义：没有名字的隐藏图层上的那个，符号名在 0 层，与别的不同（规格 §3.1）。
  - 打开隐藏的图纸图层 `Demolition`：引用亮起来，图层表里的 `Demolition` 被打开，`PID-HIDDEN` 不动；阀和填充画出来，没有名字的隐藏图层上的阀还是不见。再关掉：引用变暗，阀不见，图层表不动，灯泡还亮。
  - 没有名字的隐藏图层上的阀：引用没有被设成 `invisible`，看不见是因为它在 `PID-HIDDEN` 上。打开 `PID-HIDDEN` 的灯泡，它整个显示出来，符号名也在（符号名图层关着也一样，因为它的符号名在 0 层）；左边那段线和符号名取 `PID-HIDDEN` 的灰色，填充也在。
  - 打开 `PID-SYMBOL-LABEL` 之后，两个隐藏的阀的位置上什么都没画，在它们符号名的位置上点不到东西；在显示着的阀的符号名上点一下，选中的是这个图例。
  - 取色：把图层 `Process` 改成蓝色，这一层的阀左边那段线（0 层、随层）和管线变蓝，圆、右边那段线、填充不变；`Instrument` 上画同一个定义的阀不变。再把 `Instrument` 改成洋红，那边的左段线变洋红，`Process` 的还是蓝。
  - 移动：符号名图层开着，选中 `Process` 上的阀，`MOVE` 从 (0, 0) 到 (0, −40)。新位置周围画的是笔画加符号名，都在这个引用名下；原处什么都不剩；填充到了新位置；在新位置的符号名上点一下选中这个图例，原来符号名的位置上点不到。
- 读数：新增测试 18 个。全量 7,136 个：通过 7,083，忽略 53，失败 0（上一个提交的 7,065 加新增的 18）。clippy 656 / 1,051，与上一个提交相同；点到 `src/app/mod.rs`（4 条）、`src/io/pid/mod.rs`（1 条）的都不在这次动的行上，另外 4 个文件上没有。6 个文件都是 `new=0`。分层报告与上一个提交逐行相同。改动 +1,248 −40（不算本文件），全是测试和只在测试里编译的接线。还原产品文件之后第一次启动的全量检查没有重新编译：还原用的是复制，保留了旧的修改时间，编译工具认为不用重编，测的是带着改坏代码（开关没开）的旧程序。跑了 54 秒发现后停掉，把 5 个文件的修改时间改成当时再重跑；这里的读数是重新编译之后的。
- 没验证的：没有在程序界面里看，留给 SB-10。窗口自己拿到的那份填充清单没有读；按位置读的那种没有和出图那一份核对。测试图只有一种画法，没有转过、镜像过的摆放，没有图框，没有只画符号名的图例放在隐藏图层上的情况。图层只测了关，没有测冻结；随层只测了颜色，线宽、线型没有测。打开隐藏的图纸图层时命令行的那句提示没有断言。移动只量了 `MOVE` 平移一次。隐藏图例的两道保险都去掉的情况没有试。
- 证据：`D:\scratch\ocs-symblock\sb4\`（`quick-e1.log`、`quick-v1.log`、`quick-m0.log`、`quick-m-*.log`、`mutant-table-round4.txt`、`mutants.diff`、`chainfinal.txt`）。

### SB-4b 【测试】集成测试按新结构读图，并进固定门禁

后来加的任务（计划 §9）。固定门禁原先只跑库里的单元测试，`tests/` 下的集成测试从 SB-1 起一次没跑过；SB-4 之后补跑，`tests/pid_import.rs` 的 54 个里有 11 个失败。

- **做什么**
  1. `tests/pid_import.rs` 加一种读法「按画出来的样子读」（`drawn`）：模型空间里每个图例的块引用换回它画的图元，成员平移到插入点，0 层的换回引用所在的图层，记录换回引用上的那一份（`role` 用成员自己的，`symbol` 不带）。读「图例画了什么」的测试改用它，原来的期望值不动。另加一个测试钉住存下来的块结构。
  2. SB-1 的单元测试 `symbol_blocks_survive_a_dwg_round_trip` 不再往系统临时目录写文件；`tests/pid_import.rs` 里「临时目录上方没有符号库」的准备步骤不再受临时目录里别的文件夹影响。
  3. 固定门禁加上集成测试（§1 第 4 条），脚本 `gates.ps1` 默认跑。
- **验收**
  - [x] `tests/pid_import.rs`、`tests/pid_legend.rs` 全过。
  - [x] 原有测试不删，断言不放宽；期望值只改一处（记录的键名单加 `symbol`），提交说明里写原值、新值、原因。
  - [x] 块结构有测试钉住：每张样例图的图例数和定义数；引用带 `role=symbol`、`symbol=`、`extent=`、`style=`，在它该在的图层；成员只带 `role=`，在 0 层或导入器自己的图层；符号名在块里写一次。
  - [x] 故意改坏：引用的位置、块内坐标、少一笔、符号名不进块、成员同时留在模型空间、成员的图层和记录、引用的图层和各个键、定义共用的两个方向，每一处都有集成测试变红。
  - [x] 临时目录里有一个名字带 `symbol` 的文件夹时，「没有符号库」那一条照样通过（用一个专用的临时目录试的，系统的没有动）；跑完全部测试后系统临时目录里没有这样的文件夹。
  - [x] 固定门禁的脚本带上集成测试。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb4b -Tag final -Files src/io/pid/symbol_blocks.rs,tests/pid_import.rs -Quick symbol_blocks`；改坏用 `tools\integ-mut.ps1`（编一次，把测试程序拷出来，每处改坏跑一遍）。
- **依赖**：SB-2、SB-4。
- **文件**：`tests/pid_import.rs`、`src/io/pid/symbol_blocks.rs`（只动测试模块）。
- **规模**：M。

**做完**（2026-11-20）

- 怎么查出来的：开始做 SB-5 时发现固定门禁只跑 `--lib`。`tests/` 下有两个文件测的是导入器，它们的样例图（同级目录 `pid-parse\test-file\`）这台机器上有，会真的跑；分支上还没有一个提交动过 `tests/`。在 `035046b4e` 上跑（02:17）：`tests/pid_legend.rs` 29 个全过；`tests/pid_import.rs` 54 个，43 个通过，11 个失败。再把分支改过的 9 个源文件换回起点 `5171d197b` 的版本跑 `tests/pid_import.rs`（分支新加的 4 个文件留着；跑完由脚本换回，工作目录的状态与之前相同）：53 个通过，1 个失败。
- 简称（按文件里的先后）：
  - 改了读法的 16 个：R1 `the_import_leaves_a_summary_the_app_can_show`，R2 `a_cached_strokes_dash_is_its_own_storages_not_its_placements`，R3 `filled_areas_come_in_as_solid_hatches`，R4 `a_symbol_name_is_lettered_beside_the_symbol_it_names`，R5 `a_placement_without_a_library_body_draws_the_body_the_drawing_carries`，R6 `a_symbols_bspline_lip_reaches_the_drawing`，R7 `a_symbol_authored_away_from_its_origin_lands_on_the_line_work_it_marks`，R8 `a_symbol_body_draws_in_the_style_its_placement_names`，R9 `the_vessel_draws_in_its_placements_maroon_not_its_syms_black`，R10 `a_cached_bodys_lettering_stays_on_its_switched_off_layer`，R11 `the_opening_view_is_framed_on_geometry_that_exists`，R12 `every_entity_is_filed_under_its_authored_layer_and_the_table_is_the_drawings_own`，R13 `a_placement_states_its_extent_and_a_parametric_one_its_library_defaults`，R14 `a_placement_draws_the_body_the_drawing_carries_and_skips_its_hidden_layers`，R15 `a_label_placement_keeps_its_anchor_cross_off_the_sheet`，R16 `paper_style_scale_scales_dash_patterns_and_the_stated_line_weights`。
  - 改了期望值的 1 个：K `role_and_class_are_separate_keys_with_disjoint_vocabularies`。
  - 新加的 1 个：S `a_placement_is_one_reference_to_a_block_of_what_it_draws`。
- 11 个失败的来历，都是这个分支带来的，没有一个指向画出来的内容变了：
  - 9 个是测试按散图元的结构读图（R1、R7、R8、R10、R12、R13、R14、R15、R16）。它们在整个文档里按 `role=` 找图例的笔画和符号名，数个数、量坐标、读记录；现在笔画在块定义里，坐标是块内的，画法相同的只存一份，`extent=` 这些键在引用上。报错例如「`role=symbol` 的图元没有写 `extent=`」「导入器建的图元在 `0` 层」「画出的加边框模板加图框是文档里的全部：328 对 341」。
  - 1 个是记录的键名单（K）：「导入写了一个不认识的键 `symbol=Cap2`」。
  - 1 个是 SB-1 留下的文件夹（R5；起点源码上失败的也是它）。`symbol_blocks_survive_a_dwg_round_trip` 把 DWG 写进系统临时目录下的 `ocs_pid_symbol_blocks_tests`，用完删了文件，没删文件夹。导入器找符号库，是从图纸所在目录往上 5 级、名字里带 `symbol` 的文件夹都算（`src/io/pid/symbols.rs` 的 `discover_symbol_library`），所以在临时目录下打开的 `.pid` 都把这个空文件夹当成了符号库，R5 的前提「临时目录上方没有符号库」不成立。文件夹建于 2026-11-19 23:19，每跑一遍全量单元测试都会再建出来；11-20 确认是空的之后删掉。从建出来到删掉这段时间，这台机器上任何工作目录跑 `tests/pid_import.rs`，R5 都会失败，与那个工作目录的代码无关。
- 改了什么：
  - 读法。`trait Entities` 有两个实现：`Document` 照存下来的读（引用和定义都在内），视图过滤、图层管理器、存盘看到的就是这个；`Drawn` 是 `drawn(&doc)` 的结果，照画出来的读。`drawn` 自己带两条断言：每个引用不转不缩；文档里的图元都在模型空间和被引用到的定义里，没有落在别处的。帮手 `on_layer`、`of_role` 改成两种都收。
  - R1–R16 改用 `drawn`，期望值一个没动。其中 10 个在 `035046b4e` 上是红的（上面 9 个加 R5）。另外 6 个（R2、R3、R4、R6、R9、R11）当时还通过，但读到的是存下来的结构：坐标是块内的，画法相同的图例只数到一次。
  - K：`KEYS` 从 10 项变 11 项，加 `symbol`。这是唯一改的期望值。
  - S：新测试，见下面「S 钉住的」。
  - 其余 37 个原有测试没有动。任务开始时把 54 个逐个读过：它们读的是管线、文字、标记点、图框，或者量的就是存下来的结构（视图过滤、图层视图、存取往返），照存下来的读是对的。
  - R5 的准备步骤 `import_without_library`：图原先直接放在临时目录下新建的一个文件夹里，查找的 5 级里就有临时目录自己和它上面三级；现在放到新建文件夹底下的 `1\2\3\4\5` 里，查找看的 5 级都是测试自己建的。
  - `symbol_blocks_survive_a_dwg_round_trip`：在内存里写、读 DWG（`DwgWriter::write_to_vec`、`DwgReader::from_stream`），断言没动。
  - 仓库外的脚本（`D:\scratch\ocs-symblock\tools\`）：`gates.ps1` 加集成测试一步；新加 `integ.ps1`（单跑集成测试）、`integ-mut.ps1`（改坏）、`integ-read.ps1`（读结果）、`integ-at-start.ps1`（在起点源码上跑）。
- S 钉住的：
  - 每张图的图例数、定义数、各样式名的图例数（引用上的 `style=`），见下表。定义数既是被引用到的定义数，也是文档里 `SPPID_` 开头的块记录数，两个都要等于表里的数。四张图 80 个图例都写了 `style=`。
  - 每个引用：`role=symbol`；有 `symbol=`；块名以 `SPPID_` 开头；有 `extent=`；定义不空。
  - 每个成员：记录里只有 `role=` 一个键，值是 `symbol`、`symbol-label`、`fill` 之一；在 `0` 层，或者在一个与引用不同的、`PID-` 开头的图层。
  - 符号名：定义里角色是 `symbol-label` 的文字正好一个，内容等于引用上的 `symbol=`。
  - 引用的图层：只画符号名的在 `PID-SYMBOL-LABEL`；其余的，写了 `sheet_layer=` 就在那个图层，没写就在 `PID-SYMBOL` 或 `PID-HIDDEN`。

  | 图 | 图例 | 定义 | 各样式名的图例数 |
  |---|---|---|---|
  | `DWG-0201GP06-01.pid` | 20 | 17 | As Drawn 5，Equipment - New 1，Nozzle - New 6，Off-Line Instrument 3，Piping Component - New 4，Piping OPC 1 |
  | `DWG-0202GP06-01.pid` | 23 | 13 | As Drawn 12，Nozzle - New 3，Off-Line Instrument 2，Piping Component - New 2，Piping OPC 4 |
  | `D06.pid` | 6 | 6 | Equipment - New 1，In-Line Instrument - New 2，Nozzle - New 1，Off-Line Instrument 1，Piping Component - New 1 |
  | `工艺管道及仪表流程-1.pid` | 31 | 10 | As Drawn 12，Equipment - New 1，Nozzle - New 8，Piping OPC 10 |

- 第一次跑：
  - S 先单独跑了一遍量定义数（`integ-measure1`），再把数写进测试。
  - 第一次全跑（`integ-green1`，02:46）`tests/pid_import.rs` 的测试程序崩了，栈溢出：`Entities for Document` 的 `entities()` 里想调文档自己的同名方法，调到的是它自己。改成 `std::ops::Deref::deref(self).entities()` 之后再跑（`integ-green2`，02:49），55 个、29 个全过。测试代码写错，不是程序的问题。
  - 这一次查出检查脚本的两个漏洞，都补上了。测试程序中途崩掉时，脚本把后一个文件的「全过」记到了前一个文件名下（`integ-green1.txt` 里 `tests\pid_import.rs … 29 passed` 那一行就是这样来的）；现在崩掉的写明没有结果。测试文件的编译告警原先不列；现在单独一行，是 0。
- 故意改坏 13 处，用环境变量打开，编一次，把测试程序拷出来跑 14 遍：不开的一遍 55 个全过，每处各一遍。跑了两轮。第一轮（`mut-m1.txt`，02:50–02:52）12 处有测试变红，第 10 处 55 个全过：没有一个集成测试钉着图例的 `style=`。量了四张图上每个引用的 `style=`（`integ-measure2.log`），在 S 里加了按样式名数图例的断言。第二轮（`mut-m2.txt`，03:04–03:06）13 处都红，下表是第二轮的结果。

  | # | 改坏 | 改在哪 | 55 个里变红 | 变红的 | S 红在哪 |
  |---|---|---|---|---|---|
  | 1 | 引用的插入点横向多偏 10 | `src/io/pid/build.rs` | 3 | R7 R15 R16 | 不红 |
  | 2 | 成员不换算成块内坐标 | `src/io/pid/symbol_blocks.rs` | 4 | R7 R15 R16 S | 定义数（DWG-0201 是 20，不是 17） |
  | 3 | 符号名不进块 | `build.rs` | 9 | R1 R4 R7 R10 R12 R13 R14 R15 S | 定义里没有符号名 |
  | 4 | 成员多于一个的图例去掉第一个成员 | `build.rs` | 10 | R1 R2 R4 R5 R8 R13 R14 R15 R16 S | 去掉一笔后只剩符号名的图例，引用不在 `PID-SYMBOL-LABEL` |
  | 5 | 成员进块的同时在模型空间也留一份 | `build.rs` | 12 | R1 R2 R5 R6 R7 R8 R9 R12 R13 R14 R15 R16 | 不红 |
  | 6 | 与引用同层的成员不改到 0 层 | `build.rs` | 1 | S | 成员在引用的图层上 |
  | 7 | 成员带整条记录 | `build.rs` | 1 | S | 成员的记录不止 `role=` |
  | 8 | 引用不放到它的图层 | `build.rs` | 2 | R12 S | 引用在 `0` 层，不在它写的图纸图层 |
  | 9 | 引用不写 `extent=`、`driving=` | `build.rs` | 4 | R1 R13 R14 S | 引用没有 `extent=` |
  | 10 | 引用不写 `style=` | `build.rs` | 1 | S | 各样式名的个数（读到 `no style 20`）；第一轮不红 |
  | 11 | 引用不写 `symbol=` | `build.rs` | 1 | S | 引用没有 `symbol=` |
  | 12 | 画法相同也不共用定义 | `symbol_blocks.rs` | 1 | S | 定义数（DWG-0201 是 20，不是 17） |
  | 13 | 同名的图例不比画法，都用第一个定义 | `symbol_blocks.rs` | 2 | R7 S | 定义数（DWG-0202 是 11，不是 13） |

  改坏点已删：`build.rs` 与上一个提交逐字节相同，`symbol_blocks.rs` 的改动只有单元测试里那 7 行；`src`、`tests` 里搜不到开关。还原之后改了两个文件的修改时间，固定门禁是重新编译之后跑的。改坏的内容留在 `mutants.diff`。
- 没有红过的：
  - R1–R16 里 14 个至少红过一次，R3、R11 没有。R3 量的两张图上（DWG-0202 有 5 个填充，D06 没有）没有一个填充属于图例，13 处改坏都碰不到它读的东西。R11 要的是开图的取景范围不超出画出来的图形，13 处里没有一处让它不成立。这两个改了读法，但没有证据说改了之后它们能查出什么。
  - K 在 13 处下都不红：它查的是「写出来的键都在名单里」，少写一个键不违反。它的「先红」是 `035046b4e` 上那一次。
  - 没改的 37 个在 13 处下一个没红。
  - 每个测试只红在第一条不成立的断言上。S 红过的断言见表的最后一列；没有红过的有：引用的 `role=symbol`、块名的前缀、定义不空、没写 `sheet_layer=` 的引用在 `PID-SYMBOL` 或 `PID-HIDDEN`。
- 临时目录里有名字带 `symbol` 的文件夹时 R5 还过不过，实际试了一次（`probe-stray.txt`，03:14）：把固定门禁编出的测试程序拷到仓库外，用 `TMP`、`TEMP` 给它指一个专用的临时目录，里面放一个空的 `ocs_pid_symbol_blocks_tests`，只跑 R5：通过。专用目录的修改时间在这一遍里变了，说明测试确实在这里建过文件夹。系统自己的临时目录没有动。旧的准备步骤在同样的情况下失败，用的是 `035046b4e` 那次的记录，没有在专用目录里重做。
- 读数（固定门禁 03:07–03:18，`chainfinal.txt`）：
  - 集成测试：`tests/pid_import.rs` 55 个全过（原有的 54 加新加的 1），`tests/pid_legend.rs` 29 个全过；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - 全量单元测试 7,136 个：通过 7,083，忽略 53，失败 0，与上一个提交相同（没有新加单元测试）。
  - clippy 656 / 1,051，与上一个提交相同，没有一条点到这两个文件。clippy 不检查 `tests/` 下的文件，这一点和以前一样。
  - 格式：`symbol_blocks.rs` 是 `before=0 after=0 new=0`；`tests/pid_import.rs` 是 `before=7 after=6 new=0`。少的那一处是 R13 里的 `measures` 闭包：它本来就和 rustfmt 的排法不一样，这次改了它里面的一行，工具就把整处算成新的，所以按 rustfmt 的排法重排了，内容没变。
  - 分层报告与上一个提交逐行相同。
  - 改动 +401 −74（两个 `.rs` 文件），全是测试；产品代码没有改。
- 与之前说的不一样的地方：进展里说过「18 个测试换成了新读法」，按文件逐个数，调用 `drawn` 的测试是 16 个。
- 没验证的：
  - S 里各样式名的个数是在分支当前的代码上量的，没有和起点提交对过。依据有三条：起点写在图例每个图元上的和现在写在引用上的是 `build.rs` 里同一个变量（读代码）；单元测试 `the_reference_stands_at_the_insertion_point_and_says_what_the_placement_said` 钉着引用上的 `style=`（代码里拼的图）；DWG-0201 上的个数与 R8 原有注释里按颜色分的个数对得上（7 = 1 + 6，5 = 4 + 1，3，5）。SB-2 的全部样例对比不看 `role` 以外的键。留给 SB-10：已在它的验收里加了一条。
  - R3、R11 改了读法但没有红过（见上）。
  - 没改的 37 个里有 24 个也读文档里的图元，照存下来的读。它们当中若有哪条断言顺带读到了图例的成员（坐标是块内的），这次没有逐条用改坏去试；分类靠的是逐个读测试。
  - 起点源码上那 1 个失败是文件夹造成的，靠的是报错的位置和文件夹的建立时间；删掉文件夹之后没有把起点源码再跑一遍。
  - 「按画出来的样子读」只在引用不转不缩时成立（`drawn` 自己断言）；导入器现在不产生转过、缩过的引用。
  - `tests/pid_legend.rs` 没有改，也没有对它做改坏。
  - 集成测试只有 `pid-parse\test-file\` 的四张图；`pid-test-data` 的 151 张不在里面（SB-9、SB-10）。没有在程序界面里看。
- 证据：`D:\scratch\ocs-symblock\integ\`（`integ-head.*`、`integ-start.*`：改之前在 `035046b4e` 和起点源码上的结果）；`D:\scratch\ocs-symblock\sb4b\`（`integ-measure1.*`、`integ-green1.*`、`integ-green2.*`、`mut-m1.txt`、`integ-measure2.*`、`mut-m2.txt`、`mut-m2-*.log`、`mutants.diff`、`probe-stray.*`、`chainfinal.txt`）。

### SB-5 【接线】导入摘要加两项

- **做什么**：`ImportSummary` 加 `symbol_blocks`、`symbol_definitions`：字段、`counts()`（18 项变 20 项）、`count_mut`、`empty`，`finish` 里填值，`Built` 里带过来。查清摘要在哪些地方显示或导出（命令行那一行、备份浏览的诊断、审计），逐处定「列出新项」还是「保持原样」并写明。
- **验收**
  - [x] 两条画法相同、一条不同的符号实例：`symbol_blocks` 是 3，`symbol_definitions` 是 2；边框模板里的不算。
  - [x] 摘要存进文档再取出（`PID_IMPORT_SUMMARY.<键>`），两项都在，值相同。
  - [x] 现有 18 项的值与改之前相同（在代码里拼的图上逐项对过；`pid-test-data` 的 151 张没有重量，SB-9 的验收里有这一条）。
  - [x] 摘要的每个显示、导出处都查过，结论写在「做完」段。
  - [x] 故意改坏：两项填反、`symbol_definitions` 按引用数填，测试变红（另加 5 处，一共 7 处）。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb5 -Files <动过的 .rs> -Quick summary`。
- **依赖**：SB-2。
- **文件**：`src/io/pid/summary.rs`、`src/io/pid/mod.rs`、`src/io/pid/build.rs`，可能还有一两处显示摘要的文件。
- **规模**：M。

**做完**（2026-11-20）

- 改了什么（产品代码 +21 −1）：
  - `src/io/pid/summary.rs`：`ImportSummary` 加 `symbol_blocks`、`symbol_definitions` 两个字段，排在 `template_missing` 之后；`counts()` 从 18 项变 20 项，新的两项排在最后，原来 18 项的先后不动；`count_mut` 加两个分支；`empty()` 加两个 0。`store`、`load`、`take` 没有改：它们照 `counts()` 和 `count_mut` 走，新的两项自动存成 `PID_IMPORT_SUMMARY.symbol_blocks`、`PID_IMPORT_SUMMARY.symbol_definitions`，自动读回、取走。`log` 没有改。
  - `src/io/pid/mod.rs`：`finish` 组摘要时把这两项填上。
  - `src/io/pid/build.rs` 没有改。任务的「文件」里列了它，但 `Built` 从 SB-2 起就带着这两个数，`finish` 原先只拿它们写一行日志。
  - 测试里加了两个小工具：`src/io/pid/tests.rs` 的 `summarised`（原来的 `finished` 改成调它，它另把 `finish` 交回的摘要带出来）；`tests/pid_import.rs` 的 `stored_count`（把摘要存进一个空文档，按键名读回一个数）。
- 两项数的是什么：`symbol_blocks` 是建出来的块引用数，也就是图纸上画出了东西的符号实例数；`symbol_definitions` 是建出来的块定义数。画法相同的实例共用一个定义；同一个符号转过、缩过、改过尺寸的，各是一种画法，各有一个定义。边框模板里的符号实例不算，也不用另外去减：模板画的东西由 `overlay_template` 全部散着放在 `PID-FRAME` 上，不经过建块的那一段。
- 简称：U 是 `src/io/pid/tests.rs` 里新加的 `the_summary_counts_the_references_and_the_definitions_they_refer_to`；T 是 `src/io/pid/template.rs` 里新加的 `pid_template_placements_are_not_among_the_symbol_blocks_the_summary_counts`；RT 是 `src/io/pid/tests.rs` 里原有的 `the_summary_round_trips_through_its_properties_and_take_removes_them`；S 是 SB-4b 加的 `a_placement_is_one_reference_to_a_block_of_what_it_draws`；RIDE 是 `tests/pid_import.rs` 里原有的 `the_summary_rides_the_document_once_and_is_never_saved`，没有改。
- 钉住的：
  - U（代码里拼的图）：同一个阀放三次，两次不转，一次转 90°，另有一条线。块引用 3 个，块名依次是 `SPPID_Ball Valve`、`SPPID_Ball Valve`、`SPPID_Ball Valve_2`，文档里多出 2 个块定义；`counts()` 的后两项是 `("symbol_blocks", 3)`、`("symbol_definitions", 2)`；存进文档后这两个键的值是 `3`、`2`；`take` 取出来的摘要与存进去的相等。原有 18 项逐项写了期望值：`drawn` 16，`decoded` 4，`sheet_layers` 1，`layered_entities` 16，`sheet_layer_names` 1，`cache_bodies` 3，其余 12 项是 0。
  - T（代码里拼的图）：图纸放两个阀、一个仪表，边框模板里放一条线和同一个阀。块引用只有图纸的 3 个，模板的阀是 `PID-FRAME` 上的 2 条散线；`drawn` 8，`template_entities` 3；两项是 3 和 2。
  - RT：两个字段填 27、19，存进文档再读回，整个摘要相等。
  - S（四张样例图）：摘要的两项等于在文档里数出来的块引用数和 `SPPID_` 块定义数，按字段读一条，按存下来的键读一条。四对数是 SB-4b 量好写在测试里的（20 / 17，23 / 13，6 / 6，31 / 10），这次没有改。
- 先红（03:36–03:44，`chainred.txt`；产品代码没有动）：这时字段还不存在，测试只按键名读这两项。`--lib summary` 17 个，通过 15，失败 2，是 U 和 T：U 的原有 18 项、块名、定义数先过，红在 `counts()` 的后两项上（读到的是空的）；T 的块引用、散线、`drawn`、`template_entities` 先过，红在两项上（读到 `(None, None)`，要 `(Some(3), Some(2))`）。`tests/pid_import.rs` 55 个，通过 54，失败 1，是 S，红在 DWG-0201 存下来的键上（读到 `(None, None)`，要 `(Some(20), Some(17))`）。实现之后才加了按字段读的断言（U 一条，S 一条）和 RT 里两个字段的值。
- 现有 18 项没有变，依据三条：产品代码的改动只有加法，`finish` 里算原来 18 项的代码一行没动；U 里 18 项的期望值在改产品代码之前（03:41）和之后都过，期望值没有改过；原有测试里读摘要的断言没有改，全过。`pid-test-data` 的 151 张上没有重量，见「没验证的」。
- 摘要用在哪里，逐处的结论。按类型名 `ImportSummary`、属性前缀 `PID_IMPORT_SUMMARY` 和各字段名搜的 `src`、`tests`、`examples`；导入器以外用到摘要的就是下表这几处，再加测试。

  | # | 在哪 | 现在用的 | 结论 | 理由 |
  |---|---|---|---|---|
  | 1 | 打开图纸后的命令行（`src/app/update/file.rs`） | 固定两行：画了多少、解出多少记录、多少没画、多少图元在几个图纸图层上；图层名的个数和开始时关着的个数。有事才出的三行：驱动尺寸，样式表没读出来，单位是假定的 | 保持原样 | 这几行让人看出图进来得全不全、有什么要留意。图例存成块不影响这些，在图上点一下图例就看得到。摘要里 `cache_bodies`、`library_bodies`、`hidden_strokes_skipped`、`label_anchors_undrawn`、`lettering_flattened` 也都只进日志 |
  | 2 | 不经界面的读图（`src/io/mod.rs` 的 `load_file`）：取出摘要，调 `ImportSummary::log` 写日志 | `log` 写的那几行 | 保持原样 | 这两个数已经在日志里：`finish` 自己写一行「N symbol placement(s) are block references to M definition(s)」（SB-2 加的），经界面和不经界面的两条路都过 `finish` |
  | 3 | 随文档过打开流程（`store`、`load`、`take`） | 全部计数 | 自动带上 | 照 `counts()` 走。打开完成后和其他各条一起取走，存盘的图里不留（RIDE 钉着） |
  | 4 | 备份浏览的诊断面板（`src/io/pid_backup_browse/diagnostics.rs` 的 `Imported`，`src/ui/window/pid_backup_browser/diagnostics.rs` 显示） | 画了多少、解出多少、多少没画、图纸图层、边框模板、图例取自图纸自带的还是符号库、符号库的位置、单位、样式表 | 保持原样 | `Imported` 只留看面板的人能据此做点什么的那一部分；这两个数不指向任何要人处理的事 |
  | 5 | 备份审计（`src/app/commands/pidbackup/audit.rs`） | 每张图一行（图元数、解出数、没画数、图纸图层、用时），合计，参考数据的用量 | 保持原样 | 审计查的是备份里的图读得全不全、参考数据够不够，与图例怎么存无关 |
  | 6 | 批量报告（`examples/pid_batch_report.rs`，CSV 21 列） | 摘要里的 13 项 | 这次不加列 | 加列要改表头，仓库里已提交的那份基线就得重签；重签是单独一件事，见规格 §11 的 SB-5 第 2 条 |

  用户手册 `docs/user-guide.md` 的「导入汇总」讲的是第 1 处那几行，不用改。`docs` 下除了计划和证据，没有别的文件列举摘要的各项。
- 故意改坏 7 处，用环境变量打开。单元测试和 `tests/pid_import.rs` 各编一次，把测试程序拷出来，不开的各跑一遍（17 个、55 个全过），每处各跑一遍（03:47–03:52，`libmut-m1.txt`、`mut-m1.txt`）。前两处是验收要的，后五处是加的。

  | # | 改坏 | 改在哪 | `--lib summary` 17 个里变红 | `tests/pid_import.rs` 55 个里变红 | 读到的 |
  |---|---|---|---|---|---|
  | 1 | 两项填反 | `mod.rs` 的 `finish` | 2：U T | 1：S | U、T 读到 2 / 3；S 在 DWG-0201 读到 17 / 20 |
  | 2 | `symbol_definitions` 按引用数填 | `finish` | 2：U T | 1：S | U、T 读到 3 / 3；S 读到 20 / 20 |
  | 3 | `counts()` 里两个键名对调 | `summary.rs` | 3：RT U T | 2：S RIDE | S 按字段读的一条过，红在存下来的键上（17 / 20） |
  | 4 | 读回时不认 `symbol_definitions` | `summary.rs` 的 `count_mut` | 2：RT U | 1：RIDE | U 红在最后的 `take` 上：取出来的与存进去的不相等 |
  | 5 | 边框模板画的图元也算进 `symbol_blocks` | `finish` | 1：T | 1：S | T 读到 6 / 2；S 红在 `D06.pid`，读到 122 / 6（这张图套了边框模板，模板画了 116 个图元；排在它前面的两张照过） |
  | 6 | 两项不填 | `finish` | 2：U T | 1：S | 0 / 0 |
  | 7 | 存的时候跳过这两个键 | `summary.rs` 的 `store` | 2：RT U | 2：S RIDE | U、S 红在存下来的键上（读不到） |

  改坏点已删：`summary.rs`、`mod.rs` 的 sha256 与加开关之前相同；`src`、`tests`、`examples` 里搜不到开关。固定门禁是删掉之后重新编译跑的。带开关的改动留在 `with-mutants.diff`。
- 没有红过的：
  - U、T 里交代前提的几条（U 的原有 18 项、块名、定义数；T 的块引用、散线、`drawn` 和 `template_entities`）在 7 处下都不红：7 处都只碰新的两项。
  - U 里按字段读的那一条排在 `counts()` 那一条后面，字段不对时前一条先红，它自己没有红过。
  - 其余的测试（`--lib summary` 另外 14 个，`tests/pid_import.rs` 另外 53 个）在 7 处下一个没红。
- 读数（固定门禁 03:53–04:01，`chainfinal.txt`）：
  - 全量单元测试 7,138 个：通过 7,085，忽略 53，失败 0（上一个提交的 7,136 加新加的 U、T）。`--lib summary` 17 个全过。
  - 集成测试：`tests/pid_import.rs` 55 个、`tests/pid_legend.rs` 29 个全过，个数与上一个提交相同（没有新加集成测试，S 里加了两条断言）；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy 656 / 1,051，与上一个提交相同。点到这五个文件的只有 1 条：`src/io/pid/mod.rs` 第 596 行，`finish` 的参数有 8 个（上限 7）。上一个提交就有，位置相同；这次动的是第 708、709 行。
  - 格式：五个文件都是 `new=0`（`summary.rs` 0 → 0，`mod.rs` 1 → 1，`tests.rs` 2 → 2，`template.rs` 0 → 0，`tests/pid_import.rs` 6 → 6）。先红那一遍 `template.rs` 是 `new=1`：T 里有一处和 rustfmt 的排法不一样，红完之后按 rustfmt 的排法改了，内容没变；`red.diff` 是改过排法之后存的。
  - 分层报告与上一个提交逐行相同。
  - 改动：产品代码 +21 −1（`summary.rs` +19 −1，`mod.rs` +2）；测试 +276 −3（`tests.rs` +110 −2，`template.rs` +130，`tests/pid_import.rs` +36 −1）。
- 查显示、导出处时发现的另一件事：仓库里有一份已提交的批量导入基线 `docs/evidence/2026-09-24-pid-batch-baseline/report.csv`，带另存 DXF 的哈希，这个分支会让它过时，计划没有提。这次没有重跑、没有重签、没有加列，记进了规格 §11 的 SB-5 第 2 条，检查点二列出来请用户定。
- 没验证的：
  - 现有 18 项在 `pid-test-data` 的 151 张上没有重量；SB-9 的验收里有这一条。
  - 没有在程序界面里看。命令行那几行没变是读代码得出的（那一段没有改），没有打开一张图看。
  - `pid_batch_report` 没有跑，那份基线现在差在哪些列没有量。
  - 边框模板里的符号不算，在样例图上只有 `D06.pid` 一张查得出来（第 5 处改坏）。
  - `tests/pid_legend.rs` 没有改，也没有对它做改坏。
- 证据：`D:\scratch\ocs-symblock\sb5\`（先红：`chainred.txt`、`quickred-1.log`、`integred.log`、`red.diff`；改坏：`with-mutants.diff`、`libmut-m1.txt`、`libmut-m1-*.log`、`mut-m1.txt`、`mut-m1-*.log`；固定门禁：`chainfinal.txt` 和它列的各份记录）。新加的脚本 `D:\scratch\ocs-symblock\tools\lib-mut.ps1`：单元测试编一次，把测试程序拷出来，每处改坏跑一遍。

### SB-6 【接线】属性面板加「符号」一行

- **做什么**：`src/scene/cache/properties.rs` 的 `pid_semantics_section` 认 `symbol=` 这个键，在「角色」一行之后加一行。新词条按现有词条的做法进词条目录。
- **验收**
  - [x] 记录里有 `symbol=` 时，`P&ID` 一栏在「角色」之后有一行，值是符号名原样，只读，字段名 `pid_symbol`。
  - [x] 记录里没有 `symbol=` 时，各行与改之前相同。
  - [x] 选中一个导入的块引用，面板里既有块引用自己的几栏，也有 `P&ID` 一栏，没有 `*VARIES*`。
  - [x] 新词条在词条目录里，词条相关的现有测试通过（两个测试 C、L；另有一个脚本 `scripts/test_locales.py` 在改之前就不通过，改后它缺的名单不变，见「做完」）。
  - [x] 故意改坏：不认 `symbol` 键、行加在「角色」之前，测试变红（另加 6 处，一共 8 处）。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb6 -Files <动过的 .rs> -Quick pid_semantics`。
- **依赖**：SB-2。
- **文件**：`src/scene/cache/properties.rs`、`src/locale_catalog.rs`、`locales/` 下的词条文件。
- **规模**：S。

**做完**（2026-11-20）

- 改了什么（产品代码 +38：`properties.rs` +14，词条目录 +1，23 个词条文件各 +1；用户手册改一句）：
  - `src/scene/cache/properties.rs` 的 `pid_semantics_section`：认 `symbol` 这个键。值不空时，在「角色」一行之后、三个尺寸行之前加一行：行名 `t!("Symbol")`，字段名 `pid_symbol`，只读，值是记录里 `=` 后面的全部内容，不去空格、不改写。值是空的不出行，和 `label` 键的做法一样；导入器不写空名字（`src/io/pid/metadata.rs` 的 `attach_symbol_name` 遇到空名字什么都不写），这一条是给别处写出来的记录留的。没有这个键的记录走的代码一行没变。
  - `src/locale_catalog.rs`：加一条 `"Symbol" => Some(("properties", "symbol"))`，排在 `"Role"` 那一条后面。
  - `locales/` 下 23 种语言的 `opencadstudio.ftl`：`properties` 这条消息里各加一行 `.symbol = …`，排在 `.role` 后面。只加英文和中文不够，见下面「中间一轮」。
  - `docs/user-guide.md`：「特性面板」一段逐项列 `P&ID` 各行的那一句，在「角色」后面补上「符号」。这个文件不在任务的「文件」里；手册这一节另外几句的事见规格 §11 的 SB-6 第 1 条和新加的任务 SB-8b。
- 各语言用的词都取自这种语言自己的词条文件里已有的条目，没有新造。取的是各语言的 `mleader-style.symbol`；它也是这种语言在 P&ID 的两个菜单项（`common.group-as-p-id-symbol`、`common.ungroup-p-id-symbol`）里说「符号」用的词。两处例外：阿拉伯语那两个菜单项里的词不带冠词，新行带，和它「角色」一行（`الدور`）的写法一致；韩语的 `mleader-style.symbol` 是 `기호`，P&ID 的两个菜单项里却是 `심볼`，新行跟 P&ID 的走。日语、荷兰语、土耳其语的 `table-cells.symbol`（`記号`、`Teken`、`Simge`）是「字符」的意思，没有用。加行用的脚本 `add-symbol.ps1` 从文件里读出这些值再写，脚本里没有写死任何译文。

  | 语言 | 词 | 语言 | 词 | 语言 | 词 | 语言 | 词 |
  |---|---|---|---|---|---|---|---|
  | ar-SA | الرمز | es-ES | Símbolo | it-IT | Simbolo | pt-BR | Símbolo |
  | bg-BG | Символ | fi-FI | Symboli | ja-JP | シンボル | ru-RU | Символ |
  | cs-CZ | Symbol | fr-FR | Symbole | ko-KR | 심볼 | tr-TR | Sembol |
  | de-DE | Symbol | hi-IN | प्रतीक | lo-LA | ສັນຍາລັກ | vi-VN | Ký hiệu |
  | el-GR | Σύμβολο | hu-HU | Szimbólum | nl-NL | Symbool | zh-CN | 符号 |
  | en-US | Symbol | | | pl-PL | Symbol | zh-TW | 符號 |

- 简称：N1 是 `properties.rs` 测试模块里新加的 `a_symbol_reference_names_its_symbol_right_after_its_role`；N2 是同一处新加的 `the_name_of_a_symbol_is_shown_as_the_record_spells_it`；K 是同一处原有的 `a_placed_symbol_shows_its_library_defaults_and_its_extent_as_two_rows`，没有改；P 是 `src/app/pid_symbol_block_tests.rs` 里新加的 `a_selected_symbol_shows_its_reference_and_its_pid_group_in_the_panel`；L 是 `tests/pid_panel_localization.rs` 里原有的 `pid_panel_captions_translate_in_every_shipped_language`，加了三处；C 是原有的 `i18n::tests::every_catalog_covers_and_formats_the_source_catalog`，没有改。P、L 所在的两个文件不在任务的「文件」里。
- 钉住的：
  - N1（拼出来的记录）：一条带齐各键的记录（图纸图层、角色、样式、三个尺寸、类型、位号、`oid`、匹配方式）。不带 `symbol=` 时没有 `pid_symbol` 行。把 `symbol=Parametric Manifold` 加在最后（导入器就写在最后）：这一行的值是 `Parametric Manifold`；各行的字段名依次是 `pid_class`、`pid_role`、`pid_symbol`、`pid_driving`、`pid_instance`、`pid_extent`、`pid_label`、`pid_resolved`、`pid_sheet_layer`、`pid_sheet_layer_oid`；行名等于 `t!("Symbol")`；去掉这一行之后，其余各行与不带键时整行相等（行名、字段名、值）。读值用的是这个测试模块原有的 `value`，它只认只读的行。
  - N2：名字 ` 3" Gate=Valve (蝶阀) `（两头有空格，中间有引号、等号、汉字）原样显示；记录里只有 `symbol=Ball Valve` 一个值时也出这一行，一共 1 行；`symbol=` 后面是空的不出行，只有 `pid_role`。
  - P（程序一层，打开 SB-3 那张图：两个阀（同一个符号 `Ball Valve`）、一个只画自己名字的标注符号（`Item Note`）、一条管线，都在图纸图层 `Process`，不带 `_Data.xml`）：
    - 在第一个阀的一笔上点一下：面板的 `P&ID` 一栏正好五行，`pid_role = symbol`、`pid_symbol = Ball Valve`、`pid_extent = 40.00 × 20.00 mm`、`pid_sheet_layer = Process`、`pid_sheet_layer_oid = 12`；块引用自己的行都在，`ins_x = 100.0000`、`ins_y = 100.0000`、`ins_z = 0.0000`、`u_scale = 1.0000`、`rotation = 0`，`block` 一行里是它的块名；整个面板没有一行是 `*VARIES*`。
    - 用窗口框住两个阀：`P&ID` 一栏还是那五行（两个是同一个符号）；`ins_x` 是 `*VARIES*`，`ins_y` 是 `100.0000`。
    - 打开符号名图层，用交叉框选第二个阀和那个标注符号：`pid_symbol` 是 `*VARIES*`。
  - L：`Symbol` 加进它逐语言查的行名单（12 种语言下不是空的、不是原始编号）；中文下 `Symbol` 翻成「符号」；中文下面板自己拼出来的 `pid_symbol` 行，行名是「符号」（最后这一条是转绿之后加的，见第 7 处改坏）。
  - C：23 种语言的词条文件都有词条目录里的每一条。
- 先红（04:19–04:22，`chainred.txt`；这时只有三个测试文件有改动，产品代码、词条都没动）：
  - `--lib pid_semantics` 6 个：通过 4（原有的），失败 2，是 N1、N2，都是「missing pid_symbol」。
  - `--lib a_selected_symbol` 2 个：通过 1（SB-3 的夹点测试），失败 1，是 P：`P&ID` 一栏读到四行，没有 `pid_symbol = Ball Valve`。
  - `tests/pid_panel_localization.rs` 1 个：失败，中文下 `Symbol` 读到 `Symbol`，要「符号」。
  - C 通过：目录还没动。
- 中间一轮（04:24–04:28，`chainmid.txt`）：写了实现和目录的一条，词条只加了英文、中文两个文件。N1、N2 过，L 过，C 红：「1 missing keys in BgBg: [("properties", "symbol")]」。也就是说少了某种语言的那一行，L 照过（它对各语言只查「不是空的、不是原始编号」，只在中文下对译文），拦住的是 C。于是 23 个文件都加。这一轮 P 也红，是测试自己写错了：我以为块引用有 `x_scale`、`y_scale`、`z_scale` 三行，面板对三个方向比例相同的引用只出 `u_scale` 一行（原有的做法）；改的是测试的期望，产品代码没动。
- 转绿（04:30–04:36，`chaingreen.txt`）：`pid_semantics` 6 个、`a_selected_symbol` 2 个、C、L 全过。
- 故意改坏 8 处，用环境变量打开。`--lib` 的 8 个（`pid_semantics` 6 个加 `a_selected_symbol` 2 个）和 L 各编一次，把测试程序拷出来，不开的各跑一遍（全过），每处各跑一遍。前两处是验收要的，后六处是加的。

  | # | 改坏 | `--lib` 8 个里变红 | L | 读到的 |
  |---|---|---|---|---|
  | 1 | 不认 `symbol` 键 | 3：N1 N2 P | 红 | N1、N2「missing pid_symbol」；P 的 `P&ID` 一栏只有四行；L「the panel shows no pid_symbol row」 |
  | 2 | 行加在「角色」之前 | 2：N1 P | 过 | 次序成了 `pid_symbol`、`pid_role` |
  | 3 | 行加在三个尺寸之后 | 2：N1 P | 过 | `pid_extent` 之后才是 `pid_symbol` |
  | 4 | 名字去掉首尾空格 | 1：N2 | 过 | 读到 `3" Gate=Valve (蝶阀)`，两头的空格没了 |
  | 5 | 做成可编辑的文本框 | 2：N1 N2 | 过 | 「pid_symbol is not read-only: EditText(…)」 |
  | 6 | 空名字也出一行 | 1：N2 | 过 | `pid_role` 之后多出一行 `pid_symbol` |
  | 7 | 行名写死成英文，不经词条目录 | 0 | 红 | 中文下读到 `Symbol`，要「符号」 |
  | 8 | 记录里没有 `symbol=` 也出一行 | 3：N1 N2 K | 没跑 | N1 红在「不带键时没有这一行」上；K 的前四行成了 `pid_class`、`pid_role`、`pid_symbol`、`pid_driving` |

  - 第 1–7 处：`--lib` 是 04:42–04:44 跑的（`libmut-m3.txt`），L 是 04:44–04:46（`mut-i1.txt`）。第 8 处对应验收第 2 条，是固定门禁第一遍之后补的（05:02–05:04，`libmut-m4.txt`），只跑了 `--lib`。
  - 第 7 处是怎么来的：行名写死成英文时 `--lib` 的 8 个全过（测试在英文下跑），L 原先只查词条翻得出来，也发现不了；而中文界面下这一行叫不叫「符号」正是用户看得见的那一点。所以在 L 里加了一条：切到中文后，面板自己拼出来的这一行，行名是「符号」。这一条是实现之后加的，没有先红，靠第 7 处证明它会红。
  - 第一遍（04:37–04:39，`libmut-m1.txt`）没跑成：`lib-mut.ps1` 把测试程序拷出来直接跑，没带上仓库 `.cargo/config.toml` 里 `[env]` 给测试线程的 64 MB 栈（`RUST_MIN_STACK`），程序一层的两个测试栈溢出，测试程序中途退出。SB-5 用这个脚本只跑过名字带 `summary` 的 17 个测试，那一次没有出这个问题。脚本改成照仓库的 `[env]` 带上环境变量，重跑（04:40，`libmut-m2.txt`，当时 6 处）和上表的结果一致。
  - 改坏点已删：`src`、`tests`、`examples` 里搜不到开关。第 8 处删掉之后，`properties.rs` 的 sha256 与加它之前相同（`324c1532…`）；固定门禁第二遍是在这之后跑的。带开关的改动留在 `with-mutants.diff`（前 7 处）、`with-mutant-8.diff`。
- 没有红过的：
  - P 的后两段（框住两个阀之后 `P&ID` 一栏不变、`ins_x` 是 `*VARIES*`；交叉框选阀和标注符号之后「符号」是 `*VARIES*`）：先红时和改坏时 P 都红在第一段，后面的没走到。这两段钉的是多选时各行怎么合并，那是面板原有的代码，这次没有改，也没有对它做改坏。
  - P 里块引用自己的那几行和「没有一行是 `*VARIES*`」：同样是原有的代码。其中列块引用各行的那一条在中间一轮红过一次，是测试写错了（见上）。
  - 程序一层分不出只读和可编辑：第 5 处下 P 照过，它读的是行里的字，不看是哪种控件。只读由 N1、N2 管。
  - N1 里「其余各行与不带键时相同」那一条：8 处里 N1 红的 5 处都红在它前面的断言上。
  - L 里把 `Symbol` 加进逐语言名单的那一处：目录里没有这一条时 `translate` 原样交回英文，名单查的「不是空的、不是原始编号」照过。中文下对译文的那一条在先红时红过。
  - C 没有改，也没有对词条目录、词条文件做改坏；它在中间一轮红过。
- 读数（固定门禁第二遍 05:05–05:12，`chainfinal2.txt`）：
  - 全量单元测试 7,141 个：通过 7,088，忽略 53，失败 0（上一个提交的 7,138 加新加的 N1、N2、P）。`--lib pid_semantics` 6 个全过。
  - 集成测试：`tests/pid_import.rs` 55 个、`tests/pid_legend.rs` 29 个、`tests/pid_panel_localization.rs` 1 个，全过；前两个的个数与上一个提交相同；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。`tests/pid_panel_localization.rs` 不在固定门禁的两个集成测试里，这次带上一起跑。
  - clippy 656 / 1,051，与上一个提交相同。点到这四个文件的 2 条都是上一个提交就有的：`src/scene/cache/properties.rs` 第 20 行（这次动的是第 110 行以后）；`src/locale_catalog.rs` 第 4 行「这个函数可能在栈上分配 870,148 字节」，上一个提交是 870,035，目录多一条多了 113 字节。
  - 格式：四个文件都是 `new=0`（都是 0 → 0）。第一遍（04:49–04:59，`chainfinal.txt`）有两个文件是 `new=1`：N2 和 P 里各有一处和 rustfmt 的排法不一样，按 rustfmt 的排法改了，内容没变。第一遍其余各项的读数与第二遍相同。
  - 分层报告与上一个提交逐行相同。
  - 改动：产品代码 +38（`properties.rs` +14，`locale_catalog.rs` +1，23 个词条文件 +23）；测试 +238 −2（`properties.rs` +80，`pid_symbol_block_tests.rs` +132 −2，`tests/pid_panel_localization.rs` +26）；用户手册 +1 −1。
- `scripts/test_locales.py`（不在固定门禁里，加词条时跑到的）：它在这次改动之前就不通过。最后一项检查要求 `src` 里 `t!`、`tf!`、`HistoryLabel::Message` 里直接写的每一处文字都在词条目录里，现在有 806 处不在；脚本自己只显示前 30 处。整份名单用 `locales_full.py` 量：它拿仓库脚本的原文来跑，只去掉最后那条断言，把名单全写出来。四份名单：计划起点 `5171d197b` 806 处；上一个提交 806 处，和起点是同样的 806 处，其中 28 处行号移动了；先红时 807 处，多的一处是新测试里的 `t!("Symbol")`；改完之后 806 处，与上一个提交的逐字节相同。目录从 7,686 条变成 7,687 条，每条在英文词条文件里都有对应的一行。这 806 处没有动，记进了规格 §11 的 SB-6 第 2 条。
- 没验证的：
  - 没有在程序界面里看。面板上这一行画出来的样子（名字很长时显示得全不全）没有看；P 读的是面板的数据，不是画出来的画面。SB-10 在真实程序上验收时看。
  - 译文没有请懂这些语言的人看过。23 个词都取自各语言词条文件里已有的条目；韩语、阿拉伯语的取法见上。
  - 面板这一行的行名只在中文下对过（L）。其余 22 种语言有这一条词条（C 管），面板显示出来的是不是它没有逐个对。
  - 带 `_Data.xml` 的图上选中图例（`P&ID` 一栏另有类型、位号、匹配方式），只在 N1 里用拼出来的记录测了；P 用的图不带 `_Data.xml`。
  - 样例图上没有打开看面板。四张真实图纸上每个引用都带 `symbol=`、值等于块里的符号名文字，是 SB-4b 的集成测试钉的。
  - 炸开之后的散图元没有这一行，没有单独测：SB-3 的测试钉的是炸开后每个图元的记录里只有 `role=`，N1 钉的是没有 `symbol=` 就没有这一行，是两条合起来推出的。
  - `scripts/test_locales.py` 不通过，所以「`t!` 用到的文字都在目录里」没有一个通过的检查在管；这次靠的是量整份名单，前后相同。
- 证据：`D:\scratch\ocs-symblock\sb6\`（先红：`chainred.txt`、`quickred-*.log`、`integred.log`、`localesred.txt`；中间一轮：`chainmid.txt` 和它列的各份记录；转绿：`chaingreen.txt`；改坏：`with-mutants.diff`、`with-mutant-8.diff`、`libmut-m1.txt`（没跑成的那一遍）、`libmut-m2.txt`、`libmut-m3.txt`、`mut-i1.txt`、`libmut-m4.txt` 和各自的 `*.log`；固定门禁：`chainfinal.txt`（第一遍）、`chainfinal2.txt` 和它列的各份记录；词条：`survey2.txt`、`add-symbol.ps1`、`add-symbol-check.txt`、`add-symbol-stage1.txt`、`add-symbol-stage2.txt`；词条脚本的名单：`locales_full.py`、`locales-missing-start.txt`、`locales-missing-head.txt`、`locales-missing-red.txt`、`locales-missing-final2.txt`）。`D:\scratch\ocs-symblock\tools\lib-mut.ps1` 改过：拷出来的测试程序带上仓库 `.cargo/config.toml` 的 `[env]`。

### SB-7 【接线】图例识别跳过导入的图块

- **做什么**：`src/io/pid_legend/mod.rs` 的 `recognise_with_units` 里，块符号那一遍遇到带 `PID_SEMANTICS` 记录且 `role=symbol` 的块引用就跳过。
- **验收**
  - [x] 这样的块引用不进认出的符号，不进未知块，不进异常清单。
  - [x] 不带记录的块引用、记录里没有 `role=symbol` 的块引用，结果与改之前相同。
  - [x] 块里的文字不算散字。
  - [x] 打开、关闭图例识别（写记录、清记录）前后，这些引用上的记录逐字不变。
  - [x] 故意改坏：去掉跳过、跳过所有带记录的块引用，测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb7 -Files <动过的 .rs> -Quick pid_legend`。
- **依赖**：SB-2。
- **文件**：`src/io/pid_legend/mod.rs`，可能还有 `src/io/pid_legend/xdata.rs` 和该模块的测试文件。
- **规模**：S。

**做完**（2026-11-20）

- 改了什么（产品代码 +34：`xdata.rs` +25，`mod.rs` +9；其中代码 16 行，其余是说明）：
  - `src/io/pid_legend/xdata.rs`：加 `is_imported_symbol`。块引用的 `PID_SEMANTICS` 记录里有一项恰好是 `role=symbol`，它就是导入器放的图例。
  - `src/io/pid_legend/mod.rs`：块符号那一遍，在排除手工编组的成员之后问这一句，是就跳过。
  - 认的是记录，不是块名：同一个 `SPPID_` 块的引用，不带记录、或者记录里说的是别的，照旧识别。
  - 图例识别自己写的记录只有 `class=`、`label=`、`lines=`、`resolved=legend:…`，没有 `role=`，所以它认出并写过记录的符号，下一次识别不会被跳过。
  - 没有动的：手工编组那一遍（人把导入的图块编进 P&ID 组合是明确的指定）；写记录的 `attach`、清记录的 `purge`、读散字的 `lettering_of`。
  - 用户手册「校正 P&ID 图例」一节没有讲 `.pid` 导入的图，这次的改动没有让它哪一句变成错的，没有改。
- 钉住的（3 个新测试）：
  - C（`tests/pid_legend.rs` 的 `a_reference_that_is_not_an_imports_symbol_is_read_as_ever`）：一张拼出来的 A3 图，一个照导入器的样子做的阀块（两段短线、一个圆、`PID-SYMBOL-LABEL` 层上的符号名文字）放了三次，另有一处散字和两条框线（把图幅撑成 A3）。三个引用都不是导入器放的：第一个不带记录；第二个带发布数据给的 `class=`、`label=`、`oid=`、`resolved=direct`，没有 `role=`；第三个的 `role=` 是 `symbol-label`。三个都照旧认成未知块：进符号清单，进未知块计数（`SPPID_Ball Valve on Process` 3 个），进异常清单，各给管线追踪一个连接点；散字 1 处。
  - D（同一文件的 `a_symbol_a_pid_import_placed_is_left_out_of_recognition`）：同一张图再加两个带 `role=symbol` 的引用，记录照导入器写的两种样子：`role=symbol` 排在第一项的（图纸没给图层的图例，在 `PID-SYMBOL` 层）；排在 `sheet_layer=`、`sheet_layer_oid=` 之后、后面还有发布数据的四项和 `symbol=` 的。C 的各条断言照样成立；整个识别结果打印出来，与不加这两个引用时逐字相同。
  - P（`src/app/pid_symbol_block_tests.rs` 的 `legend_recognition_leaves_the_symbols_of_an_import_alone`）：程序一层。用的是这个测试文件一直用的那张图（导入器测试用的 `sheet_of_two_valves_a_note_and_a_pipe`：两个阀、一个标注类图例、一条管线，不带 `_Data.xml`），先记下每个带记录的图元说了什么。这张图上是 9 个：三个引用、块里的 5 个图元、管线；测试没有断言这个数，它断言的是第一个阀的记录恰好是那五项、另外两个引用也带记录。在旁边画一个图例规则认识的阀（块名 `$VALVE$00000316`），好让 `PIDLEGEND` 有符号可认、有记录可写可清。然后依次 `PIDLEGEND REPORT`、`ON`、`OFF`、`ON`、`PURGE`：报告里没有 `UNKNOWN BLOCK`；`ON` 认出的只有画的那个阀，没有未知块，画出一个框一个标签，记录只写在那个阀上；`OFF` 把画出的图例拿掉；再 `ON`，记录又写在那个阀上；`PURGE` 把图例和它写的记录都清掉。`REPORT`、第一次 `ON`、`OFF`、`PURGE` 之后各核对一次：那 9 个图元的记录与开始时逐字相同（第二次 `ON` 之后没有单独核对，紧接着的 `PURGE` 之后核对）。
- 先红（05:31–05:40，`chainred.txt`，实现之前）：
  - P 红在 `REPORT` 之后的那一条：报告第一行是「4 symbols recognised」，后面有 `UNKNOWN BLOCK SPPID_Ball Valve on Process x2` 和 `UNKNOWN BLOCK SPPID_Item Note on PID-SYMBOL-LABEL x1`。
  - D 红在第一条断言：认出 5 个符号，要 3 个。
  - C 按设计在实现之前就过：它钉的是验收第 2 条「结果与改之前相同」，同样的断言改前改后都成立。
  - 原有的图例识别测试全过：`--lib pid_legend` 37 个，`--lib pidlegend` 21 个（另有 2 个默认不跑），`tests/pid_legend.rs` 原有的 29 个。
  - `chainred.txt` 里集成测试那一项写的「compile errors: 1」，数到的是 cargo 的 `error: 1 target failed` 那一行，不是编译错误：测试程序编出来了，31 个都跑了。
- 转绿（05:42–05:48，`libmut-m1.txt`、`mut-i1.txt` 里不设变量的那一遍，代码里带着改坏开关）：`--lib` 的 59 个（P，加上 `pid_legend` 的 37 个、`pidlegend` 的 21 个）全过，`tests/pid_legend.rs` 31 个全过。去掉开关之后的读数是下面的固定门禁。
- P 在先红之后加了一条断言：第二次 `ON` 之后，记录又写在画的那个阀上。它排在先红时红的那一条后面，先红的读数不受影响；它自己没有先红，靠第 2 处改坏证明会红。
- 故意改坏 11 处（用环境变量 `OCS_MUTANT` 切换；测试程序各编一次，每处跑一遍。「库」是 `--lib` 的那 59 个，「集成」是 `tests/pid_legend.rs` 的 31 个）：

  | # | 改坏 | 库：红的 | 集成：红的 | 红在哪 |
  |---|---|---|---|---|
  | 1 | 不跳过 | 1：P | 1：D | P：报告里有 `UNKNOWN BLOCK`；D：认出 5 个 |
  | 2 | 带记录的块引用都跳过 | 4：P 和原有的 3 个 | 2：C D | P：第二次 `ON` 没认出画的阀（它带着第一次 `ON` 写的记录）；C、D：只认出不带记录的那 1 个 |
  | 3 | `role=symbol` 按开头匹配 | 0 | 2：C D | `role=symbol-label` 的那个也被跳过，只认出 2 个 |
  | 4 | 只看记录的第一项 | 1：P | 1：D | P：同第 1 处；D：认出 4 个，`role=symbol` 排在 `sheet_layer=` 后面的那个没跳过 |
  | 5 | 按块名以 `SPPID_` 开头判断，不看记录 | 0 | 2：C D | 三个都被跳过，认出 0 个 |
  | 6 | 散字从全部图元里读，块里的也算 | 0 | 2：C D | 散字 2 处，要 1 处 |
  | 7 | 写记录时不避开导入的记录 | 2：P 和原有的 1 个 | 0 | P：`ON` 之后那 9 个图元的记录全空了 |
  | 8 | 清记录时连导入的一起清 | 2：P 和原有的 1 个 | 0 | P：`PURGE` 之后那 9 个图元的记录全空了 |
  | 9 | 跳过挪到后面，仍计进未知块 | 1：P | 1：D | P：同第 1 处；D：未知块成了 `… on PID-SYMBOL` 1 个、`… on Process` 4 个 |
  | 10 | 跳过挪到后面，仍进异常清单 | 0 | 1：D | 异常清单里 5 个，要 3 个 |
  | 11 | 跳过挪到后面，仍给连接点 | 0 | 1：D | 连接点 5 个，要 3 个 |

  - 第 1–8 处是 05:42–05:48 跑的（`libmut-m1.txt`、`mut-i1.txt`）。第 9–11 处是看了前 8 处的结果之后补的（05:49–05:53，`libmut-m2.txt`、`mut-i2.txt`）：验收第 1 条说的是三样（认出的符号、未知块、异常清单），而前 8 处里 D 的「未知块」「异常清单」「连接点」三条断言没有一处单独红过：第 1–5 处 D 先红在「认出的符号」那一条，第 6–8 处这三条都成立。
  - 第 2 处红的原有 3 个，都在 `app::commands::pidlegend::tests`：`a_copied_group_keeps_its_tag_and_the_legend_follows_group_changes`、`pidlegend_on_publishes_xdata_off_keeps_it_and_purge_removes_everything`、`pidlegend_on_draws_off_removes_and_on_twice_does_not_stack`。第 7、8 处红的原有 1 个是 `io::pid_legend::xdata::tests::recognition_xdata_round_trips_and_purge_keeps_authored_identity_and_groups`。
  - 第 6、7、8 处改坏的是原有的代码（`lettering_of`、`attach`、`purge`），不是这次加的那几行：验收第 3、4 条说的行为本来就成立（见下面「没有红过的」），改坏它们是为了看新加的断言会不会红。
  - 改坏点已删：开关加在 `mod.rs`、`xdata.rs`、`blocks.rs` 三个文件里，05:53 从加开关之前留的副本拷回；连同两个测试文件，五个文件的 sha256 都与加开关之前相同（`blocks.rs` 回到提交里的样子，这次不改它），`src`、`tests`、`examples` 里搜不到开关（`restore-final.txt`）。固定门禁是在这之后跑的。带开关的改动留在 `with-mutants.diff`（前 8 处）、`with-mutants-late.diff`（11 处）。
- 没有红过的：
  - 验收第 3 条（块里的文字不算散字）和第 4 条（导入的记录不被改写）靠的是原有的代码，不是这次加的：`lettering_of` 只读模型空间的文字；`attach` 只动图例识别自己的记录；`purge` 只清图例识别自己的记录。「散字 1 处」和「记录逐字相同」这几条断言因此没有先红，只在第 6、7、8 处改坏下红过（分别是散字、`ON` 之后、`PURGE` 之后）；P 里 `REPORT` 之后和 `OFF` 之后的「记录逐字相同」两条没有任何一处让它们红。
  - C 整个测试在实现之前就过（按设计），只在第 2、3、5、6 处改坏下红过；它开头的「每毫米 1 个图形单位」没有红过。
  - C 和 D 共用的「三个都算未知的、来源是那个块名」一条没有单独红过。
  - D 最后「整个结果与不加那两个引用时逐字相同」一条，没有一处改坏走到它：每一处都红在它前面的断言。
  - P 第一次 `ON` 之后的四条（认出的只有画的阀、没有未知块、一个框一个标签、记录只写在那个阀上）、「`OFF` 拿掉图例」「`PURGE` 拿掉图例」「`PURGE` 清掉 `ON` 写的记录」没有红过：第 1、4、9 处 P 先红在 `REPORT` 那一条，没有走到它们；其余各处这几条都成立。它们说的是图例识别本来就有的行为，原有的测试也在钉。
  - P 开头的两条前提（第一个阀的记录恰好是那五项；另外两个引用也带记录）没有红过。
- 读数（固定门禁，05:53–06:03，`chainfinal.txt`，去掉开关之后的代码）：
  - 格式：四个 `.rs` 文件 `new=0`（改前 0，改后 0）。
  - 快测：`--lib pid_legend` 37，`--lib pidlegend` 21（另有 2 个默认不跑），`--lib pid_symbol_block` 15，全过。
  - 全量库测试：7,142 个，7,089 过，53 个默认不跑，0 失败（SB-6 时 7,141 个，多的一个是 P）。
  - 集成测试：`tests/pid_import.rs` 55，`tests/pid_legend.rs` 31（原有 29，加 C、D），全过；测试目标的编译警告 0；临时目录里名字带 `symbol` 的文件夹 0。
  - clippy：656 / 1,051，与 SB-6 相同；点到这次四个文件的行 0 / 0。
  - 分层报告：36 行，与 SB-5 留的那份相同。
  - 语言文件检查脚本的整份清单：806 条，与 SB-6 提交时逐字节相同（这次没有加界面文字）。
  - 改动量：产品代码 +34；测试 +313 −2（`tests/pid_legend.rs` +160，`src/app/pid_symbol_block_tests.rs` +153 −2）。
- 没验证的：
  - 没有在真实图纸上跑图例识别，三个测试用的都是拼出来的图；规格 §4.5 说的散字数量（958）没有重量。已加进 SB-9 的验收，SB-10 在程序里再看一遍。
  - 带发布数据的那种记录（`class=`、`label=`、`oid=`、`resolved=` 排在 `role=symbol` 后面）只在 D 里手写过；导入器在带 `_Data.xml` 的图上写出来的记录，没有拿来跑识别。
  - 另存为 DWG / DXF 再打开之后还跳不跳过，没有测；是从「记录存取前后不变」的原有测试推出来的。
  - 手工编组里带着导入的图块时怎么认，没有测（那一遍的代码没有改）。
  - `PIDLEGEND LIST`、`EXPORT` 和自动化接口的 `pid_legend` 没有单独测：它们用的是同一个识别结果。
  - 连接点只钉了个数；图例识别在 `.pid` 导入的图上能不能追出管线，没有量（规格 §11 的 SB-7 第 1 条）。
  - 只有导入图块、没有别的符号的图上 `PIDLEGEND ON` 说什么，没有测；读代码是「nothing recognised -- no known block or circle symbol on this sheet」那一句，不画东西，也不写记录；图上留着以前画的图例或图例记录时，它把那些清掉。这句话对一张全是导入图块的图合不合适，这次没有动，检查点二里列出来。
  - 没有在程序界面里看过（SB-10 做）。
- 证据：`D:\scratch\ocs-symblock\sb7\`（先红：`chainred.txt`、`quickred-*.log`、`integred.log`；转绿和改坏：`libmut-m1.txt`、`mut-i1.txt`、`libmut-m2.txt`、`mut-i2.txt` 和各处的 `*.log`，`with-mutants.diff`、`with-mutants-late.diff`，加开关之前的文件在 `final-tree\`，拷回之后的核对在 `restore-final.txt`；固定门禁：`chainfinal.txt` 和它列的各份记录，`locales-missing-final.txt`；另有只量格式的一遍 `fmtcheck-a.txt`，是 05:41 加开关和 P 的那条断言之前量的，四个文件 `new=0`）。

### 检查点二（SB-7 之后）

- [x] SB-3 到 SB-7 的门禁都过；全量单元测试、集成测试、两个 clippy 读数、分层报告各再量一遍。
- [x] 记进规格 §11 的偏差列给用户。不等回复。

**做完**（2026-11-20）

- 门禁：SB-3、SB-4、SB-4b、SB-5、SB-6、SB-7 六个提交的门禁都过，读数在各任务的「做完」段。
- 再量一遍（06:12–06:19，在 SB-7 的提交 `3eb3e81de` 上，工作区与这个提交没有差别；`chainhead.txt`）：
  - 全量库测试 7,142 个：7,089 过，53 个默认不跑，0 失败。
  - 集成测试：`tests/pid_import.rs` 55，`tests/pid_legend.rs` 31，`tests/pid_panel_localization.rs` 1，全过；测试目标的编译警告 0；临时目录里名字带 `symbol` 的文件夹 0。
  - 快测五组：`pid_symbol_block` 15，`pid_legend` 37，`pidlegend` 21（另有 2 个默认不跑），`pid_semantics` 6，`io::pid::` 76，全过。
  - clippy：656 / 1,051，与计划起点的读数相同。两份记录另与起点的两份（`00-baseline\`）按「文件 + 告警内容」逐条比了一遍，不看行号：只差一条，`src/locale_catalog.rs` 的栈大小提示，数字从 870,035 变成 870,148（SB-6 加了一条词条）。比的脚本把 cargo 的一行汇总（`ocs_acis` 的告警条数）记到了一个文件名下，`--lib` 那一份因此多数出一条，那不是告警。
  - 格式：从计划起点 `5171d197b` 到现在动过的 22 个 `.rs` 文件，逐个与起点的那一份比，`new=0`。
  - 分层报告：36 行，与 SB-5 留的那份相同。
  - 语言文件检查脚本的整份清单：806 条，与 SB-6 提交时逐字节相同。
- 分支现在的样子：比起点多 13 个提交（两个文档提交、SB-1 到 SB-7 的 10 个、检查点一），49 个文件，+7,345 −131：22 个 `.rs` 文件 +6,314 −110，23 个词条文件各 +1，4 个文档 +1,008 −21。没有推送。
- 列给用户的（2026-11-20 发出，不等回复）：规格 §11 里 SB-4b 的 4 条、SB-5 的 2 条、SB-6 的 2 条、SB-7 的 3 条；SB-3、SB-4 没有记偏差。其中请用户定的四件，和回复到来之前按哪一种做：
  1. 仓库里那份批量导入基线（`docs/evidence/2026-09-24-pid-batch-baseline/report.csv`）会过时，要不要重签、要不要加两列。建议本计划里不重签、不加列，合回之后单独做：重签要先分清哪些差别是起点之前带来的。按这个继续。
  2. `scripts/test_locales.py` 从起点就不通过（806 处文字不在词条目录里）。建议另立一件事，本计划不处理。按这个继续。
  3. 用户手册讲 `.pid` 的一节跟上图块（SB-8b，做 SB-6 时加的任务）。建议做，排在 SB-8 之后。按这个继续。
  4. 一张全是导入图块的图上 `PIDLEGEND ON` 说「nothing recognised -- no known block or circle symbol on this sheet」（读代码得出，没有测）。建议本期不动：要改就和「把导入的图例按符号名列成认出的符号」（规格 §9 的 Q3）一起定。按这个继续。
- 没验证的：这一遍只是把门禁重跑了一遍，没有在程序上量，那张 592 个图例的样例也没有量（SB-9、SB-10）。「六个提交的门禁都过」指各任务做完时各自量的读数，没有回到每个提交上重跑。
- 证据：`D:\scratch\ocs-symblock\cp2\`（`chainhead.txt` 和它列的各份记录、`fileshead.txt`、`locales-missing-head.txt`、`clippy-diff-lib.txt`、`clippy-diff-lib-test.txt`；脚本 `rungates.ps1`、`clippy-diff.ps1`）。

---

## 第三段：收尾

### SB-8 【文档】`REQ-MIG-008` 补一条验收标准

- **做什么**：在 `docs/pid-platform/requirements/90-migration-and-interop.md` 的 `REQ-MIG-008` 里补一条验收标准，编号和写法照该文档现有的。意思是：打开的图纸上，文件里的每个符号实例是一个能整体选中的对象，在它的任意一笔上点一下就选中整个符号，不依赖图纸旁边的发布数据。
- **验收**
  - [ ] 验收标准加上了，编号接着现有的，写法一致。
  - [ ] 文档检查 `python -m pytest scripts/pid-platform -q -p no:cacheprovider` 的结果不比改之前差（改之前先量一遍）。
  - [ ] 文档检查要求同步的其它文件（索引、对照表）一并改。
- **验证**：上面的文档检查命令，改前改后各一遍，输出存 `D:\scratch\ocs-symblock\sb8\`。
- **依赖**：SB-3（验收标准说的行为先得有测试）。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`，可能还有文档检查要求同步的文件。
- **规模**：S。

### SB-8b 【文档】用户手册讲 `.pid` 的一节跟上图块

做 SB-6 时加的，原计划没有：见规格 §11 的 SB-6 第 1 条、`plan.md` §9。

- **做什么**：`docs/user-guide.md`「打开 Smart P&ID（.pid）图纸」一节还按「图例是一组散笔画」讲的几句，改成现在的样子；另加一段讲图例是图块：在任意一笔上点一下选中整个图例；块名是 `SPPID_` 加符号名，画法不同的带序号，认符号看特性面板的「符号」一行；炸开变回散图元；`PID_SEMANTICS` 记录里哪些键在引用上、哪些在块里的图元上；图纸图层视图各行的数字怎么数。只写测试钉住的或在程序上量过的。已知要改的几句（做的时候把这一节从头到尾再对一遍）：
  - 开头「文件里的线、文字、符号、填充和审查标记被读成普通 CAD 实体」：符号现在是块引用。
  - 「图层」一段「导入后每个实体的图层就是 …… 图纸图层名」：图例的引用在图纸图层上，块里的笔画在 `0` 层。
  - 「分类信息没有丢：每个实体的 `PID_SEMANTICS` 扩展数据里写着 `role=` …… 与 `style=`」和后面给第三方的那句粗体字：图例的 `style=` 在引用上，块里的图元只带 `role=`。
  - 「图纸图层视图」一段「列出每层的实体数」「各角色 …… 及实体数」：数法变了（规格 §4.3）。
  - 「符号的三个尺寸」一段「点选一个符号放置的任一笔画或它旁边的符号名」「同一放置的所有实体相同」：选中的是这个图例的块引用，三个值只在引用上。
- **验收**
  - [ ] 这一节里讲图例结构的每一句都与现在的行为一致；改了哪几句、每句依据哪条测试或哪份读数，列成表写进「做完」段。
  - [ ] 另存 DWG / DXF 之后，第三方怎么取图例的分类、样式和符号名写清楚。
  - [ ] 这一节里没讲错的不动；别的章节不动。
- **验证**：SB-8 的文档检查改前改后各一遍（它管不管这个文件，做的时候查清）；逐句对照表存 `D:\scratch\ocs-symblock\sb8b\`。
- **依赖**：SB-3、SB-4、SB-6（讲的行为先得有测试）。
- **文件**：`docs/user-guide.md`。
- **规模**：S。

### SB-9 【测试】样例测试和性能预算

- **做什么**：用样例图（本机有 `pid-test-data` 才跑）加测试：结构、摘要、导入两次结果相同、存 DWG 再读回；加一个默认不跑的性能预算测试（`#[ignore]`，写法照 `src/pid/perf_budget.rs`）。
- **验收**
  - [ ] 样例只带 `.pid`：模型空间里带 `role=symbol` 的块引用 592 个；没有 `role` 为 `symbol`、`symbol-label` 的散图元；`symbol_blocks` 是 592；`symbol_definitions` 等于文档里 `SPPID_` 开头的块定义数，数值量出来后写进测试。
  - [ ] 现有 18 项摘要计数与基线相同（基线值在 SB-1 开始前量好）。
  - [ ] 同一个文件导入两次：块名的集合相同，每个引用的块名、插入点、记录相同。
  - [ ] 存成 DWG 再读回：块引用数、定义数、每个引用的记录与存之前相同。
  - [ ] 样例上跑图例识别（`pid_legend::recognise`）：认出的符号、未知块、异常清单里都没有导入的图块；散字数量出来写进测试（规格 §4.5 写的是 958，试验时量的）。做 SB-7 时加的：SB-7 的测试用的是拼出来的图，没有在真实图纸上跑（规格 §11 的 SB-7 第 2 条）。
  - [ ] 性能预算：样例导入 3 次取中位数，不超过基线中位数的 1.1 倍；基线值和预算值写在测试里并注明是哪台机器、哪个提交量的。
  - [ ] 故意改坏：关掉共用（每个图例一个定义），定义数的测试变红。
- **验证**：`gates.ps1 -Out D:\scratch\ocs-symblock\sb9 -Files <测试文件> -Quick <测试名>`；预算测试另用 `cargo test --offline --locked --lib <名字> -- --ignored` 跑三遍。
- **依赖**：SB-2、SB-5。
- **文件**：`src/io/pid/` 下的测试文件，预算测试放在现有预算测试旁边。
- **规模**：M。

### SB-10 【验收】在真实程序上验收

- **做什么**：编分支头的发行版，用无界面方式（`--serve`）在样例上逐条量规格 §6；起一个打开了样例图的程序给用户；写验收报告。不改代码；量出问题就回到对应的任务。
- **验收**
  - [ ] 只带 `.pid`：块引用 592 个，定义数与 SB-9 的测试一致。
  - [ ] 出图：SVG 与基线程序出的相比，图形元素个数相同，坐标按 0.01 取整后只在一边出现的 0 个。
  - [ ] 另存 DWG：大小不超过基线（499,568 字节）的 1.7 倍；再打开，块引用数和每个引用的记录与保存前相同，再出的图与保存前相同。
  - [ ] 用时：只带 `.pid` 导入 3 次的中位数不超过基线程序同机中位数的 1.1 倍。
  - [ ] 带 `_Data.xml`：引用上的 `class`、`label`、`oid`、`resolved` 与基线程序写在笔画上的相同（按 `oid` 归并后比）。
  - [ ] 引用上的 `style=` 与基线程序写在该图例笔画上的相同（SB-4b 留下的：集成测试里各样式名的图例个数是在分支上量的，没有和起点对过）。
  - [ ] 图例识别：导入的图块没有一个被报成未知块。
  - [ ] 起好一个打开了样例图的程序，路径和用法告诉用户。
  - [ ] 验收报告：规格 §6 的 12 条逐条写结果和证据位置，没验证到的单列。
- **验证**：脚本和输出在 `D:\scratch\ocs-symblock\sb10\`。
- **依赖**：SB-1 到 SB-9。
- **文件**：只有 `todo.md`。
- **规模**：M。

### 检查点三（SB-10 之后，等用户）

- [ ] 验收报告发给用户。
- [ ] 用户看过之后：合回本地 `main`（有冲突先问），删掉工作目录，清理 `D:\scratch\ocs-symblock`。
