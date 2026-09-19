# Open CAD Studio 使用教程

本文面向第一次使用 Open CAD Studio 的用户，介绍从启动、打开图纸、绘制对象到保存文件的常用操作。界面会随着版本持续变化，以下截图基于当前开发版。

## 启动应用

如果你使用发布版，在 Windows 上直接运行 `OpenCADStudio.exe`。如果从源码启动，在项目根目录执行：

```powershell
cargo run
```

启动后会进入欢迎页。左侧是最近打开的文件列表，中间区域提供新建、打开、插件、发布说明和反馈入口。顶部是 Ribbon 功能区，常用命令按 `Draw`、`Model`、`Insert`、`Annotate`、`View`、`Manage` 等页签分组。

![Open CAD Studio 启动页](screenshots/open-cad-studio-start.png)

## 新建或打开图纸

在启动页点击 `New Drawing` 可以创建空白图纸。点击 `Open File...` 可以打开已有 DWG 或 DXF 文件。打开后的文件会以标签页形式显示，左侧 `Properties` 面板用于查看或编辑当前选中对象的属性。

也可以使用顶部菜单或快捷命令打开文件。常用文件能力包括 DWG/DXF 读写、STL/STEP/OBJ 相关导入导出、PDF 输出、外部参照和块写出。

## 认识主界面

主界面主要分为五个区域：

- 顶部 Ribbon：按任务分组放置绘图、修改、标注、图层、块和属性工具。
- 中央绘图区：显示模型空间或布局空间内容，鼠标用于选择、绘制、平移和缩放。
- 左侧属性面板：选中对象后显示对象属性；未选中对象时显示 `No Selection`。
- 文件标签：在 `Start` 和各个图纸之间切换，也可以通过 `+` 创建新图纸。
- 视图与样式控件：绘图区上方可切换 `Wireframe 2D` 等显示样式。

## 绘制基础图形

进入 `Draw` 页签后，可以直接点击图标创建常用对象：

- `Line`：绘制直线。
- `Polyline`：绘制连续折线。
- `Circle`：绘制圆。
- `Arc`：绘制圆弧。
- `Text`：插入文字。
- `Dimensions`：创建尺寸标注。
- `Leader`：创建引线标注。

基本流程通常是：选择工具，按命令提示指定第一个点，继续指定后续点或参数，最后按 `Enter`、`Esc` 或根据命令提示结束。

## 编辑对象

绘制完成后，可以使用 `Modify` 区域的工具编辑对象。常用命令包括：

- `Move`：移动对象。
- `Copy`：复制对象。
- `Rotate`：旋转对象。
- `Scale`：缩放对象。
- `Mirror`：镜像对象。
- `Trim`：修剪对象。
- `Extend`：延伸对象。
- `Offset`：偏移对象。
- `Fillet`：创建圆角。
- `Explode`：分解块、尺寸、复合线等复合对象。

先选择对象再执行命令，或先执行命令再按提示选择对象，具体取决于命令交互。使用 `Esc` 可以取消当前命令。

## 使用捕捉与精确绘图

Open CAD Studio 支持端点、中点、圆心、象限点、交点、垂足、切点、最近点、插入点等对象捕捉。启用对象捕捉后，鼠标靠近可捕捉位置时会自动吸附到精确点位。

常用精确绘图能力包括：

- `OSNAP`：对象捕捉。
- `OTRACK` / `F11`：对象捕捉追踪。
- `DYNMODE` / `F12`：动态输入。
- 极轴追踪：按角度增量辅助绘图。
- 命令历史：使用上下方向键切换历史命令。

## 管理图层和属性

Ribbon 中的 `Layers` 区域用于管理图层。你可以创建、锁定、冻结、关闭或切换当前图层。右侧属性下拉框可以快速设置对象颜色、线型和线宽，例如 `ByLayer` 表示对象继承所属图层的属性。

典型工作流是先设置当前图层，再绘制对象。这样后续可以按图层批量控制显示、打印和属性。

## 打开 Smart P&ID（.pid）图纸

Open CAD Studio 可以直接打开 SmartPlant / Smart P&ID 的 `.pid` 文件：`Open File...` 的文件类型里选 `Smart P&ID Files`（`CAD Files` 也包含 `.pid`），或者在资源管理器里右键 `.pid` →「打开方式」选 Open CAD Studio。打开就是一次导入：文件里的线、文字、符号、填充和审查标记被读成普通 CAD 实体，可以像其他图纸一样选择、量测、标注和另存。

**`.pid` 是只读来源。** 程序永远不会往 `.pid` 里写字节：按 `Ctrl+S` 会直接弹出「另存为」，默认文件名是同名的 `.dwg`；命令行或脚本里把保存目标写成 `.pid` 会被拒绝并提示 `read-only Smart P&ID source`。要保留修改，另存为 DWG 或 DXF 即可，图纸里的 P&ID 语义（见下文特性面板一段）会一起保存。

**导入汇总。** 打开完成后命令行给出两行信息：第一行是画出了多少实体、来自多少条已解码记录、有多少条源记录没有画出，以及带原图图层的实体数；第二行是 `P&ID sheet layers: N, of which M start switched off`——按名字数的原图图层有几个、其中几个按文件里的隐藏状态初始关闭。图里有参数化符号时还有第三行 `P&ID driving dimensions: D on T template bodies; P placed parametric bodies carry library defaults`——文件缓存的符号库模板本体上有名字的驱动尺寸共几条、分布在几个模板本体上、有几个放置能在特性面板里看到这些库默认值（每个这样的放置在面板上对应一行「驱动尺寸（库默认）」）；没有参数化符号的图不出这一行。若样式表读取失败，还会多一行错误提示，此时线宽和颜色会退回图层默认。逐条明细在日志里。

**符号从哪来。** 每个符号放置画的是这张 `.pid` 自己缓存的那份本体——SmartPlant 实际放置的那个符号版本，参数化符号则是被拉伸后的实例；符号内部处于关闭图层上的笔画（伴热线、夹套线、`NULL` 占位文字、参数化本体的构造线等，文件里标为不显示、SmartPlant 屏幕上也看不见）不画，也不进任何图层。符号库只在文件没有可画的缓存本体时补位：`.pid` 通过工程参考数据共享路径引用符号库，程序会从图纸所在目录向上查找 SmartPlant 工程的 `Ref\Symbols`；找不到时，把环境变量 `PID_SYMBOL_LIBRARY` 指向本地的符号库副本。两边都没有本体的放置只显示一个位置点。要回到旧的「先库后缓存」顺序（库本体按 `.sym` 全部 sheet 合并画出、缓存本体连关闭层一起画），把环境变量 `OCS_PID_SYMBOL_SOURCE` 设为 `library` 再启动程序；未设置、为空或值不认识时按默认的 `cache`（值不认识会在日志里提示一行）。这个开关只保留一轮。日志里另有一行说明本次导入有多少放置画的是缓存本体、多少画的是库本体、跳过了多少条关闭层上的笔画。

**图层。** 导入后的实体按「角色」放在一组 `PID-*` 合成图层上，在图层管理器里可以像普通图层一样开关：

| 图层 | 内容 | 初始 |
|---|---|---|
| `PID-GEOMETRY` / `PID-STYLE-<样式名>` | 线条；有命名样式的线按样式名各占一层 | 开 |
| `PID-TEXT` | 图纸文字 | 开 |
| `PID-SYMBOL` / `PID-SYMBOL-LABEL` | 符号本体 / 符号名标签 | 开 / 关 |
| `PID-FILL` | 填充区域 | 开 |
| `PID-POINT` / `-WARNING` / `-ERROR` / `-APPROVED` | 审查状态标记 | 开 |
| `PID-FRAME` | 按文件页面尺寸画出的图框 | 开 |
| `PID-CONNECTIVITY` / `PID-ANNOTATION` | 连通链诊断线 / 注记占位（当前为空） | 关 |
| `PID-HIDDEN` | 原图放在 `Hidden` / `HiddenObjects` / `Invisible` 图层上的内容 | 关 |

**用原图图层名作图层（`OCS_PID_LAYER_MODE`）。** 上表是默认的「分类」模式（`taxonomy`）。把环境变量 `OCS_PID_LAYER_MODE` 设为 `sheet` 再启动程序（或运行命令行工具），导入时每个实体的图层就直接是 SmartPlant 里它所在的图层名——`Default` / `Labels` / `HeatTrace` / `HiddenObjects` 等，不加前缀，同名图层跨存储合并为一层；图层表列出的是原图自己的全部图层（包括没有画出实体的），每层的开关按文件里的显示状态设定，原图隐藏的层直接为关，不再生成 `PID-HIDDEN` 与 `PID-STYLE-*`。程序自己造出的实体（图框 `PID-FRAME`、符号名标签 `PID-SYMBOL-LABEL`、连通链 `PID-CONNECTIVITY`、没有原图图层的字形线 `PID-GEOMETRY`）仍留在各自的 `PID-*` 层上。分类信息并没有丢：两种模式下实体的 `PID_SEMANTICS` 扩展数据里都写着 `role=`（角色）与 `style=`（样式名），特性面板的「角色」行和图层管理器的「图纸图层」视图两种模式下读数相同；另存为 DWG/DXF 后，第三方查看器里看到的图层名就是原图图层名。变量未设置、为空或值不认识时按默认模式导入（值不认识会在日志里提示一行）。默认值暂不翻转，等 DXF 下游消费方的用法定下来再议。

**图纸图层视图。** 打开 `.pid` 后，图层管理器的工具栏多出 `图层` / `图纸图层`（英文界面为 `Layers` / `Sheet layers`）两个切换按钮。切到 `图纸图层`，表格列出的不再是 `PID-*` 合成层，而是 SmartPlant 自己的图层名（例如 `Default` / `Labels` / `ConsistencyChecks` / `HiddenObjects`）及每层的实体数，下方 `角色`（Roles）一段列出各角色（`geometry` / `text` / `symbol` / `symbol-label` / `point-*` / `connectivity` / `fill` / `frame`）及实体数。点击行尾的眼睛可以单独关掉或打开某个原图图层或某个角色：一个实体只要所属图层或角色任一被关就不显示。这些开关记录在图纸里（`PID_VIEW_FILTER`），另存为 DWG/DXF 后再打开仍然有效，也可以撤销。原图隐藏的图层初始为关；把它打开时，程序会连带打开承载这些实体的那个图层（默认模式下是 `PID-HIDDEN`，`OCS_PID_LAYER_MODE=sheet` 下是同名的原图图层），实体才看得见。搜索框对两种视图都有效。

**特性面板。** 选中导入的实体，左侧特性面板会多出 `P&ID` 一组只读属性：`类型`（已发布数据里的对象类，如 `PIDPipeline`）、`角色`（导入时读到的角色，即上表的分类）、`驱动尺寸（库默认）` 与 `本体尺寸`（仅符号放置的实体有，见下一段）、`位号` 或 `管线号`、`匹配方式`（若来自图例识别）、`图纸图层` 与 `图层 OID`（原图图层名及其在文件里的编号）。这些信息写在实体的 `PID_SEMANTICS` 扩展数据里，导出 DXF/DWG 时保留。

**符号的两个尺寸。** 点选一个符号放置的任一笔画或它旁边的符号名，`P&ID` 组会多出 `本体尺寸`——这张图上该符号本体的外框宽 × 高（毫米，两位小数，按放置的旋转 / 缩放量得，量的就是屏幕上画出的那些笔画——`.pid` 自己缓存的实例本体里文件显示着的那部分；`OCS_PID_SYMBOL_SOURCE=library` 下仍按整个缓存本体量）；如果它是参数化符号且文件里配得上它的库模板，还会多出 `驱动尺寸（库默认）`——模板本体上按名字列出的驱动尺寸，如 `Top 20.32 mm · Left 114.30 mm · Right 114.30 mm`。**后者是符号库的默认值，不是这张图上的实际尺寸**：SmartPlant 放置后被拉伸过的实例，文件里只存了画好的几何和一份等于库默认的变量副本，实际参数并不在文件里，所以两行并排给出，一个是「库里怎么定义的」，一个是「这张图上画了多大」。非参数化符号（阀门、仪表等）只有 `本体尺寸` 一行。两个值分别以 `extent=` / `driving=` 写在 `PID_SEMANTICS` 里，同一放置的所有实体相同，随 DWG/DXF 保留。

## 校正 P&ID 图例

打开 P&ID 图纸后，在 `Draw` 页签的 `P&ID` 区域点击 `P&ID 图例`，或运行 `PIDLEGEND LIST`。图例面板会列出当前识别到的符号和位号；点击一行可以选中并定位该符号。

面板底部的 `例外` 区域默认折叠，数字与 `PIDLEGEND REPORT` 的例外汇总使用同一口径。展开后按 `未知块`、`未命名形状`、`无主位号`、`范围标注`、`重复位号` 分组；点击任一明细会选中原始块、笔画或文字并缩放到其位置，方便直接修图或补规则。

自动识别不准确时，可以把正确的几何和位号文字组成一个明确的 P&ID 符号：

1. 选中符号的几何和它自己的位号文字。
2. 点击 `P&ID 组合`，或在视口右键菜单中选择 `组合为 P&ID 符号`。命令行也可以运行 `PIDGROUP GROUP`；普通 `GROUP` 会额外询问组名。
3. 组合会按自动识别的同一套文字规则读取位号，并把 `tagName=…;tagSource=auto` 写入原生 DXF/DWG GROUP 的描述。图例中的手动组带 `手` 角标。
4. 要调整成员时，选中组内任一对象，点击 `取消 P&ID 符号组合`，或运行 `PIDGROUP OFF` / `UNGROUP`；重新选择正确成员后再次组合。右键菜单只在当前选择属于某个组时显示取消组合入口。

选中手动组的任一成员后，左侧特性面板的 `P&ID` 区域可以直接修改 `位号 (tagName)`。输入与组内文字不同的值时来源变为 `手动`；清空，或改回文字本来能读出的值，来源恢复为 `自动`。自动来源会在下一次识别时重新读取当前文字。

默认的 `PICKSTYLE` 会让点击一个成员时选中整组。需要单独编辑组内文字时，可以按住 `Ctrl` 选择，或临时运行 `PICKSTYLE 0`；完成后再恢复原设置。保存为 DXF 或 DWG 后，组、描述和位号会一起保留。导出 SVG 时，已识别的符号写成 `<g tagName="…">`，组内几何与位号文字都属于该节点。

通过 `OpenCADStudio --serve` 启动的逐行 JSON 自动化接口使用同一套语义。先用 `query` 取得十六进制句柄并通过 `select` 建立当前选择，再调用 `pid_group`：

```json
{"op":"select","handles":["1A","1B"]}
{"op":"pid_group","what":"create"}
{"op":"pid_group","what":"tag","tag":"XV-0001"}
{"op":"pid_group","what":"auto"}
{"op":"pid_group","what":"off"}
```

`create`、`tag`、`auto` 和 `off` 分别对应 `PIDGROUP GROUP`、`PIDGROUP TAG`、`PIDGROUP AUTO` 和 `PIDGROUP OFF`。响应中的 `groups_before` / `groups_after` 会返回组句柄、组名、位号来源和成员句柄，`changed` 表示本次是否改变了组状态。

要把识别结果交给其他程序，可以直接运行：

```text
PIDLEGEND EXPORT D:\output\sheet.json
PIDLEGEND EXPORT D:\output\sheet.csv
```

扩展名决定格式。JSON 与 `dxf_legend --json` 使用同一个序列化器，包含符号、位号、例外和完整管线 run；CSV 先给出 `class,label,tag,x_mm,y_mm,lines,source` 符号表，再给出 `line,family,runs,length_mm,from,to` 管线表。导出会按需重新识别，但不会绘制覆盖层或写入实体 XDATA，所以在第一次 `PIDLEGEND ON` 之前、或 `PIDLEGEND OFF` 之后都可以使用。

逐行 JSON 自动化接口提供同一份结构化结果：

```json
{"op":"pid_legend","what":"recognise"}
{"op":"pid_legend","what":"report"}
{"op":"pid_legend","what":"export","path":"D:\\output\\sheet.json"}
{"op":"pid_legend","what":"export","path":"D:\\output\\sheet.csv"}
```

三种动作都返回相同的识别字段；`report` 额外返回命令行报告数组，`export` 额外返回写入路径、格式和字节数。

## 标注、文字和表格

`Annotate` 页签集中放置文字和标注能力。常用功能包括：

- `TEXT` / `MTEXT`：创建单行或多行文字。
- `DIMLINEAR`、`DIMALIGNED`、`DIMANGULAR`、`DIMRADIUS`、`DIMDIAMETER`：创建不同类型尺寸。
- `DIMSTYLE`：管理尺寸样式。
- `MLEADER` / `MLEADERSTYLE`：创建和管理多重引线。
- `TABLE` / `TABLESTYLE`：创建表格和表格样式。

创建标注前建议确认当前图层、文字样式和尺寸样式，避免后续逐个修改。

## 切换视图和显示样式

绘图区上方的显示样式下拉框可以切换线框、着色等显示模式。`View` 页签提供缩放、视图方向、视口和布局相关工具。

常见操作包括：

- 鼠标滚轮缩放视图。
- 使用平移工具移动当前视图。
- 使用 ViewCube 切换顶视图、前视图、等轴测视图等方向。
- 在布局空间中创建或编辑视口。

## 保存和关闭图纸

图纸有未保存改动时，关闭文件或退出应用会出现保存提示。选择 `Save` 保存改动，选择 `Discard` 放弃改动，选择 `Cancel` 返回继续编辑。

![未保存改动提示](screenshots/save-changes-dialog.png)

建议在长时间绘图时定期保存。对重要文件，保存前可以另存副本，避免覆盖原始图纸。

## 使用插件

启动页和 Ribbon 中提供 `Plugins` / 插件管理入口。当前版本支持外部插件包发现、启用/禁用、安装、卸载和升级。插件通常放在项目或应用识别的插件目录中，并通过插件管理器加载。

如果插件没有出现，先确认插件包结构和清单文件是否正确，再重新打开插件管理器刷新状态。

## 常见问题

如果应用无法启动，先在项目根目录运行：

```powershell
cargo check
```

如果 `cargo check` 通过但 `cargo run` 失败，优先查看终端中的错误信息。当前开发版编译时可能会出现 warnings；warnings 不一定阻止运行，但应在发布前清理。

如果打开文件后显示异常，先确认文件格式和版本。项目支持 DWG/DXF R13 到 R2018；包含外部参照、特殊字体、复杂线型或大型 3D 实体的文件，首次打开和渲染可能需要更长时间。

如果操作过程中界面看起来没有响应，先按 `Esc` 取消当前命令，再尝试切换工具或重新选择对象。
