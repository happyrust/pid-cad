# 0202 图框外那两条文字：是导入器写的符号名，钉在了符号画不到的地方

`2026-08-22-screen-vs-assertion-crosscheck.md` §6 第 4 条留了一个尾巴：DWG-0202GP06-01
有 2 条 Text 的插入点在图框外（x = -10.5 和 -23.3），"值得单独看一眼它们属于哪个符号"。
本文把这两条认出来，排除两个本来最像的病因，并说明它们为什么落在框外。

**结论**：它们不是图纸上的字，是**导入器自己写的符号名标签**（`PID-SYMBOL-LABEL`，默认隐藏），
内容是 `.sym` 文件名 `ElecTraceLine`。标签被钉在**符号放置原点**旁 2.3mm，而
`ElecTraceLine.sym` 的本体离它自己的原点有 104mm / 155mm 远——所以标签飘到了离符号
十几个字宽以外的地方，其中五个飘出了 A2 图框的左边。**这六个符号本身一笔不缺，都画在图框里。**

> **更正**：本文 8-24 首版 §5 断言"这 6 个放置点一笔几何都没画"。**那是错的**，已重写。
> 观察（y 330..370 之间零个点）没错，错在由此推出"没画"——它默认了本体画在插入点上。

## 0. 钉住的版本与环境

| 项 | 值 |
|---|---|
| OpenCADStudio HEAD | `78192c9f`（工作区另有他人未提交的 `src/locale_catalog.rs` 等，与本文无关） |
| pid-parse | path 依赖，`v0.11.7`，工作区对 HEAD 无内容差异 |
| 构建 | `cargo build --example pid_plot_dump --example pid_probe`（OCS）、`--example dump_symbol_geometry`（pid-parse），均 debug |
| 样本 | `pid-parse/test-file/DWG-0202GP06-01.pid` 与 `test-file/symbols-full/`（只读，未开 GUI，未产生锁或 `.sv$`） |

⚠ **探针必须用绝对路径喂**。`discover_symbol_library` 从图纸路径往上走，传相对路径时
`parent()` 一步就到头，符号库找不到，23 个放置点全退化成 marker（`PID-SYMBOL` = 23）。
换成绝对路径才是真实口径（`PID-SYMBOL` = 180）。本文所有数字来自绝对路径那一次；
两次的对照本身也是 §2 的证据之一。

## 1. 这两条是什么

```
pid_probe.exe D:\...\DWG-0202GP06-01.pid
  entities reaching x>900 or x<0:
    Text         (-10.5,348.8)
    Text         (-23.3,348.9)
    total outliers = 2
```

```
pid_plot_dump.exe D:\...\DWG-0202GP06-01.pid PID-SYMBOL,PID-SYMBOL-LABEL
text,-10.533442794355608,348.8291107611703,2,0,"ElecTraceLine"
text,-23.34074305406294,348.8921901726485,2,0,"ElecTraceLine"
```

三个特征同时对上，才敢说它们是导入器生成的符号名标签，而不是图纸自己的字：

| 特征 | 实测 | 出处 |
|---|---|---|
| 字高 | 2.0（图纸自己的字是 1.50 / 2.03 / 2.46 / 2.50 / 2.54 / 3.17 / 3.50 / 6.35 这一档） | `SYMBOL_LABEL_HEIGHT_MM = 2.0`，`src/io/pid.rs:72` |
| 旋转 | 0（注释自陈"无论放置角度都摆平"） | `src/io/pid.rs:1295-1296` |
| 内容 | 恰好是 `.sym` 的文件名 | `symbol_name()` 取 `symbol_path` 的 leaf，`src/io/pid.rs:1291` |
| 图层 | `PID-SYMBOL-LABEL`（默认隐藏），全层 23 条，与 23 个 `SymbolInstance` 一一对应 | `src/io/pid.rs:1302` |

符号库里确有这个文件：`symbols-full\Design\Annotation\Graphics\ElecTraceLine.sym`（32,768 字节）。

全图 6 条 `ElecTraceLine` 标签，五条排在 y≈348 的一条横线上，一条在 (111.9, 64.2)：

| 标签 x | 标签 y | 反推的放置原点 |
|---:|---:|---|
| -23.34 | 348.89 | (-25.64, 349.89) |
| -10.53 | 348.83 | (-12.83, 349.83) |
| 3.87 | 348.14 | (1.57, 349.14) |
| 18.31 | 348.17 | (16.01, 349.17) |
| 154.40 | 348.41 | (152.10, 349.41) |
| 111.90 | 64.24 | (109.60, 65.24) |

图幅是 593.75 × 419.63mm（A2），所以飘到框外的是这一排最左边的两个。

## 2. 标签钉在放置原点旁 2.3mm

```rust
label.insertion_point = Vector3::new(
    projection.mm(insertion.x) + SYMBOL_MARKER_RADIUS_MM + SYMBOL_LABEL_GAP_MM,
    projection.mm(insertion.y) - SYMBOL_LABEL_HEIGHT_MM / 2.0,
    0.0,
);
```

`1.5 + 0.8 = 2.3mm`，`2.0 / 2 = 1.0mm`，所以上表第三列 = (标签 x - 2.3, 标签 y + 1.0)。

这不是纯反推：把符号库藏起来（用相对路径跑）让它们退回 marker，marker 圆心直接印出来了——

```
    Circle       (-12.8,349.8)
    Text         (-10.5,348.8)
    Circle       (-25.6,349.9)
    Text         (-23.3,348.9)
```

`ext_min = (-25.64, 5.27)` 也是这个数：`Bounds::add` 收的是解析侧 `SymbolInstance` 的
insertion，不是画出来的实体（`src/io/pid.rs:1476`）。

## 3. 排除：不是坐标系错位

最像的病因是标签与本体走了两条路——标签用 `projection.mm(x) + 偏移`，本体与 marker 用
`projection.point(insertion)`。如果 `point()` 带页面平移而 `mm()` 只缩放，标签就会整体偏。

**不是。** `Projection::point` 就是 `mm(x), mm(y)`，纯缩放无平移（`src/io/pid.rs:483-485`），
两者同空间。这条假设死了。

## 4. 排除：不是被图幅带过滤掉的

`SheetBand::holds` 只用在两处：算初始视图范围（`Bounds::add`）与过滤连接性线段
（`on_sheet_pair`）。它自己的注释写着"实体本身照样导入"（`src/io/pid.rs:559-561`）。
没有任何一条路径会因为坐标出框而丢实体。

## 5. 这 6 个符号都画了，画在离放置原点 104mm / 155mm 的地方

`ElecTraceLine.sym` 解出 10 笔图元（8 线 + 2 弧，另有 4 条 `0x000F` 记录无解码器被跳过），
它们的坐标**全挤在 (103.1..107.7, 154.1..155.7) mm 那一小块**——离这个符号自己的原点
一百多毫米。而 `Placement::apply` 画本体时用的是 `insertion + 本体坐标`
（`src/io/pid.rs:1502-1512`），于是本体落在原点外一百多毫米处。

把 8 条本体线段按每个放置的插入点 + 旋转算出应落位置，去 `PID-SYMBOL` 层的实际输出里
逐条比对（容差 0.2mm）：

```
放置原点 (-25.64, 349.89) rot=270  8/8 命中   本体落在 (129.4, 244.9)
放置原点 (-12.83, 349.83) rot=270  8/8 命中   本体落在 (142.2, 244.8)
放置原点 (  1.57, 349.14) rot=270  8/8 命中   本体落在 (156.6, 244.1)
放置原点 ( 16.01, 349.17) rot=270  8/8 命中   本体落在 (171.0, 244.2)
放置原点 (152.10, 349.41) rot=270  8/8 命中   本体落在 (307.1, 244.4)
放置原点 (109.60,  65.24) rot=  0  8/8 命中   本体落在 (214.6, 220.2)
                                  合计 48/48
```

两条圆弧也对得上：该符号的弧半径 1.62mm，全图恰好 13 条这个半径的弧，其中 12 条两两成对，
六对的位置就是上面六个落点。剩下那 1 条属于别的符号。

层计数同样自洽：`PID-SYMBOL` 共 180 个实体 = 163 line + 15 采样弧 + 1 circle + 1 text。
那唯一一个 circle **不是 marker**：半径 6.35mm（marker 是 1.5mm），是 `DCS Field Mounted`
的仪表泡，该符号本体正好绕自身原点画，所以圆心与放置原点重合。也就是说**全图一个 marker 都没有，
23 个放置全部解出了真本体**。这与 pid-parse `2026-07-26-phase36-sym-symbol-library.md`
记的"0202：23 放置 / 23 可绘制 / 179 图元"对得上（179 + 1 条 Phase 36 之后才解出的符号内文本 = 180）。

首版之所以看反，是因为只在放置原点脚下找几何：y≈348 那一排下面确实什么都没有，但符号画在
y≈244 那一排。

### 这不是个例：库里有一类符号是按页面坐标画的

用 `dump_symbol_geometry` 量本体相对自己原点的范围（毫米）：

| 符号 | 本体范围 | 库内路径 |
|---|---|---|
| Cap2 | x[-0.0 .. 1.9] y[-3.2 .. 1.9] | `Piping\Fittings\End Components\` |
| Flanged Nozzle | x[0.0 .. 3.2] y[-3.2 .. 1.9] | `Equipment Components\Nozzles\` |
| Gauge Hatch | x[0.0 .. 3.2] y[-3.1 .. 2.9] | `Equipment Components\Nozzles\` |
| DCS Field Mounted | x[-6.3 .. 6.3] y[-6.3 .. 6.3] | `Instrumentation\System Functions\D C S\` |
| Off-Drawing | x[-45.0 .. 0.0] y[-2.5 .. 2.5] | `Instrumentation\Instrument OPC's\`（另一份在 `Piping\Piping OPC's\`，x[-45.0..-1.0]） |
| **ElecTraceLine** | **x[103.1 .. 107.7] y[154.1 .. 155.7]** | `Design\Annotation\Graphics\` |
| **Remarks** | **x[98.8 .. 111.9] y[123.3 .. 140.4]** | 三份（`Instrumentation\Labels - General Instrument\`、`Piping\Labels - Piping Components\`、`Piping\Labels - Piping Segments\`），三份同值 |
| **Item Note & Label** | **x[101.6 .. 120.7] y[138.4 .. 152.4]** | `Design\Annotation\Labels\` |
| **Drawing Description** | **x[108.0 .. 162.4] y[154.3 .. 168.9]** | `Design\` |
| **Wastewater Pit** | **x[82.5 .. 196.9] y[120.6 .. 152.4]** | `Equipment\Labels - Equipment\Descriptions new\`（`Equipment\Vessels\Tanks\` 那份是 x[25.4..177.8] y[127.0..177.8]，同样远） |

这张图用到的 11 个符号里，有 6 个是"远离原点"这一类。**它们不是零星异常，是半数。**

### 全库普查：三分之一的符号都不在自己的原点上

把 `symbols-full` 的 618 个 `.sym` 全量 dump（613 个有几何，5 个空），按"所有坐标是否都在
原点 ±20mm 内"分两堆：

| | 数量 |
|---|---:|
| 绕原点画（\|坐标\| ≤ 20mm） | 402 |
| **离原点远** | **211（34%）** |

按顶层目录分，差异是结构性的，不是随机的：

| 顶层目录 | 绕原点 | 离原点远 | 远的占比 |
|---|---:|---:|---:|
| Equipment | 5 | 89 | 95% |
| Design | 1 | 36 | 97% |
| Equipment Components | 16 | 5 | 24% |
| Piping | 145 | 41 | 22% |
| Instrumentation | 235 | 40 | 15% |

换个切法：路径里带 `Label` / `Annotation` 的，113 远 / 18 近；其余 98 远 / 384 近。
**标注类与设备类基本都远，管件与仪表元件基本都近。**

而且那些远的坐标常常是**整英寸**，一眼就是"画在定义图纸上的某个位置"，不是几何本身的形状：

```
Design\Annotation\Graphics\Line.sym     x[101.6 .. 127.0] y[152.4 .. 152.4]   = (4",6") → (5",6")
Design\Annotation\Graphics\Circle.sym   x[101.6 .. 101.6] y[127.0 .. 127.0]   = (4",5")，r=3.81mm=0.15"
Design\Annotation\Graphics\Break.sym    x[-203.2 .. -177.8] y[73.7 .. 77.5]   = (-8"..-7", ~2.9")
```

`Circle.sym` 的这个值 pid-parse 自己的单测 `circle_symbol_reads_as_a_single_circle` 就写死着
（`center.0 == 0.101_600`、`center.1 == 0.127_000`），只是没人把它和"放置原点在哪"联系起来。

值得记一笔：pid-parse 的模型说的是另一回事——`symbol_library.rs` 的
`valve_symbol_reads_as_lines_and_circles_around_its_own_origin` 断言本体在原点 ±20mm 内。
那条断言对 402 个符号成立，对另外 211 个不成立，而 `read_symbol_geometry` 对两者一视同仁。

### 首版漏掉的第三处出框

`pid_probe` 的出界检查只测 `x > 900 或 x < 0`，所以它一直没报 y 方向的。同一张图里
`Item Note & Label` 有 4 个放置，其中一个的标签在 **(430.74, -127.13)**——掉在图框**下面**。
同样的比对，4 个放置 **4/4、8/8 线全中**，那个放置原点 (428.44, -126.13) 的本体落在
(530 .. 549, 12 .. 26)，正是图幅右下角，**在框内**。`PID-SYMBOL` 层的全局包围盒
x 最大 549.09、y 最小 12.30 就是它顶出来的。

**四张 fixture 里，所有出框的东西都是符号名标签，没有一条是图纸自己的字，也没有一笔是几何。**

## 6. "SmartPlant 里本来就该溢出图框吗"

**不该，也没有。答案是甲：`.sym` 本来就带偏移，导入器的 `insertion + 本体` 是忠实的。**
出框的只有我们自己写的标签；本体一个都没出去。

（本节 8-24 首版写成"待定的甲乙二选一"，而且写着"天平明显偏向乙"。下面三条证据把它定成甲，
乙被推翻。原文的甲乙表述保留在本节末尾，因为后面几条建议是按它写的。）

### 定论的那一条：电伴热符号落在管子上，误差 0.02–0.20mm

`ElecTraceLine` 是电伴热标记，它**必须**画在管道上——这是一条与解码无关的领域约束，
所以它能当外部裁判。DWG-0202 的六个放置，本体落点对 `PID-GEOMETRY` 最近线段的距离：

```
本体落点                最近的图纸自身线段
(129.4, 244.9)          0.10 mm
(142.2, 244.8)          0.09 mm
(156.6, 244.1)          0.09 mm
(171.0, 244.2)          0.05 mm
(307.1, 244.4)          0.20 mm
(214.6, 220.2)          0.02 mm
```

`PID-GEOMETRY` 全部来自图纸自己的 `igLine2d` / `igLineString2d` 记录，**一笔都不来自符号库**，
所以这不是自证。六个全部压在管中心线上，最大偏离 0.20mm。

同一批放置按乙读（本体归一化到放置原点）：

```
放置原点                最近的图纸自身线段
(-25.6, 349.9)          74.30 mm    ← 图框外
(-12.8, 349.8)          70.97 mm    ← 图框外
(  1.6, 349.1)          70.49 mm
( 16.0, 349.2)          70.46 mm
(152.1, 349.4)          70.22 mm
(109.6,  65.2)          31.85 mm
```

六个全部悬空，两个在纸外。**甲 0.09mm 平均 vs 乙 64.71mm 平均**，不是个需要权衡的判断。

代数上也就此闭合：设 SmartPlant 减去手柄 `H`，则 `insertion = 目标 − 本体 + H`，
而我们画的是 `insertion + 本体 = 目标 + H`。实测 `≈ 目标`，故 `H ≈ 0`。
`ElecTraceLine` 是一个本体在 (103..108, 154..156)mm 的"远"符号，`H = 0` 就是"不该减"。

### 佐证一：库里根本没有承载手柄的地方

乙唯一的硬支撑是"`ElecTraceLine.sym` 有 4 条 `0x000F` 记录无解码器被跳过"。
把全库 618 个 `.sym` 的跳过记录按类型普查，再按"远 / 近"交叉：

| 被跳过的类型 | 出现在多少个远符号 | 多少个近符号 |
|---|---:|---:|
| `0x0019` | 114 / 211（54%） | 168 / 402（42%） |
| `0x005E` | 28 / 211（13%） | 364 / 402（91%） |
| `0x0015` | 33 / 211（16%） | 8 / 402（2%） |
| **`0x000F`** | **2 / 211（0.9%）** | **0 / 402** |

`0x000F` 全库只出现在 **2 个文件、共 6 条记录**里。一个"每个符号都有的放置手柄"不可能只在
2/618 个文件里。而且**没有任何一个类型**满足"几乎所有远符号都有、近符号都没有"——
最偏向远的 `0x0015` 也只盖住 211 个里的 33 个。

手柄也不在别的流里：用 olefile 把 618 个 `.sym` 的 CFB 流 / 存储清单也做了同样的交叉，
没有任何一个流是"远符号普遍有、近符号没有"。`SymbolInformationCluster` 远 87.2% / 近 100%，
方向还是反的。

### 佐证二：放置原点本来就不是"符号该在哪"

五个 `ElecTraceLine` 的放置原点 x = −25.64 / −12.83 / 1.57 / 16.01，`Item Note & Label`
有一个在 y = −126.13。**放置原点落在纸外是常态**，它本来就不是页面坐标，
就是一个"从这里加上库坐标"的锚。反过来，本体落点 129.4 / 142.2 / 156.6 / 171.0
是一排均匀间距 12.8–14.4mm 的点——那才是页面上该有的样子。

### 这条已被回归测试钉住

`tests/pid_import.rs::a_symbol_authored_away_from_its_origin_lands_on_the_line_work_it_marks`：
DWG-0202 的六个 `ElecTraceLine` 必须落在图纸自身线工作（`PID-GEOMETRY` 加
`PID-STYLE-*` 各专业层）的 `TRACE_REACH_MM = 6mm` 以内。
验过退回乙会红——在 `Placement::apply` 里减掉 `ElecTraceLine` 的本体原点
`(0.1031916, 0.1542723)` 后，六条读数变成 29–172mm，两条落到负 x。

### 首版的甲乙表述（存档）

- **甲：`.sym` 本来就带偏移，导入器的 `insertion + 本体` 是忠实的。**（← 成立）
- **乙：`.sym` 存的是它自己定义图的页面坐标，SmartPlant 拿某个原点记录归一化，
  而 reader 把那条记录丢了。**（← 推翻）

首版偏向乙的理由是全库普查："偏移是 211 个符号（34%）的常态，按目录成建制分布，
坐标还常常是整英寸。"这几条观察本身没错，**错在从它们推出结论**：
坐标像整英寸只说明作者当年在定义图纸上按英寸网格画，不说明 SmartPlant 会把它减掉。
这正是本项目栽过三次的那类推理（A01 那 18 条、GLine2d 锚点、JStyleOverride 锚点），
这次是第四次——只不过这回反过来了，"看着规整"支持的是错的那一边。
定论靠的不是更多的规律，是一条与解码无关的领域约束（电伴热必须在管上）加一次实测。

## 7. 已落地与仍未决

**已落地**（`fix(io): name a placed .pid symbol beside the symbol, and frame on what it
drew`）。这两条无论 §6 定成甲还是乙都该改，所以没等定论：

1. **标签改挂本体**。`symbol_label_anchor` 读 `built` 的包围盒，把符号名放在右边缘外
   `SYMBOL_LABEL_GAP_MM`、与本体中线齐平处。marker 回退路径**逐位不变**：1.5mm 的点
   以插入点为心，右边缘正好是老公式的 `+ 半径 + 间隙`，中线正好是插入点。
2. **取景改收本体**。`accumulate_bounds` 的 `SymbolInstance` 分支改成 `add_drawn(built)`。

四张 fixture 的效果：`pid_probe` 报的出界实体 2 / 0 / 9 / 0 **全部归零**；`ext_min` 回到纸面上
（0202 由 -25.64 → 40.79，工艺管道由 -8.03 → 63.37）。三条回归测试钉住它，全部验过"退回旧写法就会红"：
`io::pid::tests` 两条（marker 位置逐位不变、本体远离锚点时标签跟本体走）与
`tests/pid_import.rs` 两条（符号名不得离最近笔画超过 20mm；取景不得越出实际画出的东西 1mm）。

**后来也定了**：

3. **§6 的甲乙定成甲**，本体落位是对的，`0x000F` 与位置无关（全库 2 个文件 6 条记录）。
   带一条回归测试 `a_symbol_authored_away_from_its_origin_lands_on_the_line_work_it_marks`。
   这条原本是"比那两条大得多的一件事"——现在它不再是待办，是已钉住的不变量。

**仍未决**：

4. 顺带：`pid_probe` 的出界检查只看 x，漏了 y 方向的 `Item Note & Label`（§5 末）；
   `pid_probe` / `pid_plot_dump` 传相对路径会静默失去符号库，普查数字差一大截（180 → 23）。
   这两个 example 该在找不到符号库时明说一句，出界检查也该测 y。
5. `pid_plot_dump` 的 `poly` 行不带闭合标志，所以从 dump 看不出一条折线是开是闭
   （图幅边框在 DXF 里 `is_closed = true`，dump 里只剩三段）。它只影响读 dump 的人，
   不影响导入结果，但会让人误判缺了一条边。
6. `place_primitive` 的 `SymbolPrimitive::Polyline` 分支不设 `is_closed`——
   `pid-parse` 那侧的 `SymbolPrimitive::Polyline { vertices }` 也没有这个字段，
   所以符号里若有闭合折线，会画成开口的。尚未证实库里有这种符号。
