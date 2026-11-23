# P&ID 备份包：数据清点与界面覆盖方案

> 日期：2026-11-10
> 范围：SmartPlant P&ID 备份包里到底有哪些数据、现在在界面上各自看得到看不到、看不到的原因，以及让「各种数据都能通过 UI 查看到」的做法。本文只定方案，不含界面实现。
> 基于：分支 `upstream-merge-20261110`（上游合并 `862ddb1e1`，其上两次提交 `a13e85732`、`925728489`）；`pid-parse` / `pid-plot` 钉在 `d0a58c64c413`（pid-parse 0.11.7，默认特性 `backup` = `backup-exp` + `backup-mdf`）。
> 承接：`docs/plans/2026-10-16-pid-backup-package.md`。它的 D1（SQL Server 包暂不支持）、D2（不做完整工厂树）、D5（摘除 GPL 依赖）在这次的要求下需要重新表态，见 §4。
> 证据：除标注「推断」「未验证」的句子外，本文的数字和现象都是在本分支上亲自跑出来、看到的；复现办法在 §5。

## 1. 现状

### 1.1 加载路径

```
PIDBACKUPOPEN <目录|zip>
  └ io::pid_backup::restore            解 PlantData~*.zip、拷贝包旁已解压的 Ref\ → <TEMP>\ocs-pid-backups\<hash>
  └ io::pid_backup_browse::build_index scan_backup_folder + read_backup_drawings + read_plant_groups
  └ ui::window::pid_backup_browser     树（备份文件 / 装置组 / 参考数据）+ 详情键值表，纯函数 update / visible_rows / detail_rows
  └ app::commands::pidbackup           on_pid_backup 落地 Effect；展开图纸 → load_publish_from_pid 读图元；双击 → 打开为标签页
```

整条链只占用 `Message::PidBackup(PidBackupMsg)` 一个消息和 `pid_backup_browser` 一个状态字段。

### 1.2 实测

三个包都走了两遍。先是经应用的无窗口走查（`app::commands::pidbackup::audit`）：每个节点取详情、每张图读图元、每张图过导入器，下表的数字出自它。再是真实窗口（Vulkan 后端，点击是投递到编辑器自己窗口的鼠标消息）：每一种节点各点开看过——根、各种备份文件、归档成员、装置组、图纸、图元类型、图元、读不出的图纸、参考目录与文件、搜索框、刷新与关闭、双击开图——但没有把每个节点都点一遍。

| 包 | 载入 | 备份文件 | 图纸 | 参考文件 | 图元 / 属性 | 图元读不出 | 导入不了 |
|---|---|---:|---:|---:|---|---|---|
| 全厂包 `D:\work\plant-code\cad\pid-test-data` | 首次约 0.8 s（含还原 130 MB），缓存后 0.12 s；真实窗口发命令到出树 0.21–0.24 s | 15（1 个登记但缺失） | 53 | 645 | 14 894 / 15 921（8 056 个图元没有属性） | `D03`、`D04` | `D03` |
| `DWG-0202GP06-01_p` | 0.07 s | 15 | 7 | **0** | 152 / 70 | `001` | `001` |
| `TEST02_p`（SQL Server MTF） | 0.05 s | 14 | 12（按文件夹列出） | **0** | 3 / 6 | 11 张模板图 | `CPECCHBA2-new`、`CPECCHBA2` |

全厂包 53 张图逐张读图元共 1.3 s；逐张导入共 36.5 s（最慢 `P井场` 6.2 s），149 310 个实体，9 条源记录没画出来。读不出的原因都是 pid-parse 给的 `no P&IDAttributes row carries a DrawingNo`，导入不了的是 `No decoded geometry`。

### 1.3 发现的问题

**已修（本分支，带测试）**

- 图纸行没有展开箭头，而箭头是读图元的唯一入口：真实面板里一张图的图元和属性根本点不开。基线 `2e9e9dab2` 的真实窗口同样如此（`target\ui-audit\gui-plant-baseline`）。原有单测直接发 `Toggle` 消息，绕过了画出来的行。修复 `a13e85732`：未读、在读的图纸带箭头；读取失败把原因写进详情区和命令行。

**未修，属于第二阶段的界面问题**（真实窗口截图为证）

1. 详情键值表的键列固定 96 px，长键不换行，值直接压在键上：`INamedInstrument.InstrTagSequenceNo` 与它的值重叠，`DOCUMENTCATEGORY` 也被值压住。
2. 树里看不出当前选中的是哪一行。
3. 图元行的标签是 32 位 UID 加类名，折成两行还被右边界截断；位号（`IObject.Name`，如 `MM-222`）只在详情里。
4. 归档成员是带目录项的平铺长列表（`RefData~4~681.zip` 742 行），没有大小，也看不了内容。
5. 备份文件只显示存储名：`RefData~4~680` 是 `Ref\rules.rul`、`~683` 是 `ProjectStyles.spp`、`~709` 是 `InsulationSpec.isl`、`PlantData~2~722` 是 `SPEMDataMap.xml`，界面上看不出来；`~703` 是 `exportlayer.xlsx`，却显示成「Zip，12 个成员」。
6. 「参考数据」只认包旁已解压的 `Ref\` 目录。原样的备份包（两个小包）这一支是空的；导入时也就用不上包里的符号库（对画出来的图有多大影响没有量）。
7. 关闭备份后面板里「尚未打开备份。」出现两遍；组详情的路径用 ` / `，图纸详情用 ` \ `；同组图纸按库里的行序排，不按名字。
8. 读不出、打不开的原因是 pid-parse 的英文原句，带整条缓存路径。
9. 还原缓存固定在 `%TEMP%\ocs-pid-backups`（C 盘），全厂包 130 MB，从不清理。

**与备份无关但会挡住验收**

- **DirectX 12 下模型视口不出图。** 本机自动探测选中 dx12（`[gpu] backend: dx12 (AMD Radeon RX590 GME)`），此时任何图纸（`.pid`、`.dxf`）的视口都是空的：没有实体，没有栅格，没有 ViewCube；屏幕上和 `/capture` 里一样。`--backend vulkan` 下正常。基线与合并后表现相同（`target\ui-audit\gui-backends`，视口非背景像素占比：dx12 0.28% / 0.28%，vulkan 1.51% / 3.48%，两个版本逐项相同），所以不是合并带来的。原因没有查。
- 经自动化通道执行 `PIDBACKUPOPEN` 之后，当前图纸带上修改标记（标题出现 ●），关窗口会弹「未保存的更改」，尽管这条命令不改图纸；`ZOOM E` 也一样，基线上 `PIDBACKUPOPEN` 同样。自动化通道和界面命令行走的是同一个 `run_command_line`（读代码所得），所以在命令行里敲应该也如此，这一点没有在窗口里试。

## 2. 数据清点

以全厂包为准：`Manifest.txt` 322 行、`Export.dmp` 74 489 856 B（Oracle exp V12.01.00，4 个库 154 张表 365 530 行）、`PlantData~2~711.zip`（53 张 `.pid`）、11 个 `RefData~4~*` 卷、`PlantConfig.xml`、`Export.log`。

「界面」一列：✅ 看得到；◐ 看得到一部分；❌ 看不到。「原因」一列只有三种：**无接口**（钉版 pid-parse 没有公开读法）、**未接**（有接口，界面没用）、**已接有缺**（接了，显示不全或不对）。

| # | 数据类 | 包里有什么 | 钉版 pid-parse 接口 | 界面 | 在哪里 / 原因 |
|---:|---|---|---|:-:|---|
| 1 | Manifest 概况 | 22 个单值键（Name、Version、BackupType、DateCreated、Spid、OracleVersion、Characterset、BackupCommand、ProjectType、Rootitem…） | `backup::parse_manifest_bytes` → `Manifest { lines }`、`first_field` | ◐ | 根节点详情显示 5 个；其余 17 个**未接** |
| 2 | Manifest 数据库对象清单 | Table 154（SQPlant 22 / SQPlantd 25 / SQPlantpid 82 / SQPlantpidd 25）、View 35、Right 78、Role 1 | `Manifest::tables()`、`views()`、`all(key)` | ❌ | **未接** |
| 3 | Manifest 文件登记 | File 14 行（库号、目录/文件、编号、原始 UNC 路径、状态、存储名）+ FileSize 11 行（文件数、目录数、字节数） | `Manifest::all("File")`（通用行） | ◐ | 只用来标出「登记但缺失」；原始路径、文件数**未接** |
| 4 | Manifest 连接与账号 | PlantConnInfo 4、SiteConnInfo 2、TableSpace 2、DBUids、DBPwds（混淆过的口令） | 通用行 | ❌ | **未接**。接的时候口令类字段必须遮住，见 D6 |
| 5 | 备份卷清单 | `Export.dmp` + 2 个 PlantData 卷 + 11 个 RefData 卷：格式、大小、成员数 | `backup::scan_backup_folder` | ✅ | 「备份文件」节点及各文件详情 |
| 6 | 归档成员 | 8 个归档的中央目录，共 861 项（681：742，682：20，684：10，685：3，703：12，804：10，809：4，711：60） | `backup::list_zip_entries`（`ZipListing`） | ✅ | 展开备份文件；平铺、无大小 |
| 7 | 包根目录其他文件 | `PlantConfig.xml`（ISO 15926 线型映射、伴热介质）、`Export.log`（导出日志，含每表行数） | 无（纯文本 / XML） | ❌ | `scan_backup_folder` 不列它们；**无接口**，也不需要：OCS 自己读得了 |
| 8 | 转储头 | 格式、版本行 `EXPORT:V12.01.00` | `ExportDumpScan` | ✅ | `Export.dmp` 详情 |
| 9 | 数据库表目录与行数 | 4 库 154 表 365 530 行（§5.1） | 表名：`Manifest::tables()`；行数：只在 `Export.log` 里 | ❌ | **未接**（表名）/ **无接口**（行数，OCS 自己解析日志即可） |
| 10 | 图纸登记 `T_DRAWING` | 53 行，面板取 20 列；全厂包里 10 列有值、10 列全空 | `oracle_exp::read_backup_drawings` | ✅ | 图纸详情 |
| 11 | 装置层级 `T_PLANTGROUP` | 6 行 | `read_plant_groups`、`plant_group_path` | ✅ | 「装置组」树与组详情 |
| 12 | 工厂与项目 | SQPLANT 库：`T_PLANT`、`T_PROJECT`、`T_UNIT` 3、`T_AREA` 2、`T_PLANTSETTING` 14、`T_ROLEACCESS` 12、`T_HIERARCHYLEVELS` 3、`T_DRAWINGSITE` 53… | 无 | ❌ | **无接口** |
| 13 | 格式与单位 | `T_FORMATS` 385、`T_FORMATTYPES` 41 | 无 | ❌ | **无接口** |
| 14 | 图元主表与分类表 | `T_MODELITEM` 47 124、`T_PLANTITEM` 13 687、`T_PIPERUN` 4 686、`T_INLINECOMP` 4 533、`T_INSTRUMENT` 3 718、`T_INSTRFUNCTION` 3 718、`T_PIPINGCOMP` 3 503、`T_SIGNALRUN` 1 001、`T_EQUIPMENT` 402、`T_NOZZLE` 377、`T_EQUIPMENTOTHER` 225、`T_MECHANICAL` 94、`T_VESSEL` 71、`T_EXCHANGER` 12、`T_CASE` 209、`T_CASEPROCESS` 301 | `ExpTables::pipe_runs()`（`T_PIPERUN` 的公称直径、介质代码、材料等级、顺序号、流体系统、保温规范）；`T_MODELITEM`、`T_NOZZLE`、`T_SIGNALRUN` 只在内部读 | ❌ | `pipe_runs` **未接**，其余**无接口**。面板里的图元来自 `.pid` 文件本身（#26），不是库 |
| 15 | 关系与连接 | `T_RELATIONSHIP` 24 664、`T_SIGNALPOINT` 22 057、`T_CONNECTOR` 11 757、`T_PIPINGPOINT` 9 628、`T_CONNECTORVERTEX` 5 181 | `ExpTables::port_tables()`、`connectivity_ends(图纸)`、`composition_relations()` | ❌ | **未接**；`T_RELATIONSHIP`、`T_CONNECTORVERTEX` **无接口** |
| 16 | 图形表示与标注 | `T_REPRESENTATION` 42 540、`T_SYMBOL` 18 199、`T_LABELPERSIST` 12 584、`T_LEADERVERTEX` 12 584、`T_SMARTFRAME` 12 | `T_REPRESENTATION` 只在内部读 | ❌ | **无接口** |
| 17 | 跨图连接 OPC | `T_OPC` 952 | `ExpTables::opc_points(图纸)` | ❌ | **未接** |
| 18 | 注释 | `T_NOTE` 800、`T_ITEMNOTE` 800 | 只在内部读 | ❌ | **无接口**（库侧）；图内的 ItemNote 图元经 #26 看得到 |
| 19 | 一致性与规则状态 | `T_RULEREFERENCE` 28 460、`T_PROPERTYBREAK` 14 963、`T_INCONSISTENCY` 4 624 | 无 | ❌ | **无接口** |
| 20 | 历史与版本 | `T_HISTORY` 30 931、`T_GLOBALDRAWING` 53、`T_DRAWINGPROJECT` 53、`T_DRAWINGVERSION` 15 | 无 | ❌ | **无接口** |
| 21 | 选项设置 | `T_OPTIONSYMBOLOGY` 77、`T_OPTIONSETTING` 66、`T_OPTIONAUTOGAP` 44、`T_OPTIONFORMAT` 39、`T_OPTIONHEATTRACE` 22、`T_OPTIONDISPSET` 11 | 无 | ❌ | **无接口** |
| 22 | 数据字典 | SQPLANTPIDD / SQPLANTD 两库各 25 表：`ITEMATTRIBUTIONS` 29 824 / 354、`CODELISTS` 3 407 / 130、`UNIQUEATTS` 1 527 / 131、`ATTRIBUTES` 1 004 / 85、`ENUMERATIONS` 133 / 13、`ENTITIES` 80 / 20、`FILTER*`… | `ExpTables::codelist_short_text(表号)`、`codelist_row_count()` | ❌ | 代码表**未接**，其余**无接口** |
| 23 | SQL Server 转储里的库 | `TEST02_p` 的 `Export.dmp`（MTF，19.1 MB） | `publish::mdf_load::open_mtf_dump_as_sqlite`、`sqlite_load::*`（`backup-mdf`，现在是链进来的） | ❌ | **未接**：面板提示「不读取其数据库」，图纸按文件夹列出，只有 PATH、FILE 两项 |
| 24 | 图纸文件 | `PlantData~2~711.zip` 内 53 张 `.pid`（`00\00` 43、`00\02` 4、`test\U01` 6） | 还原后按路径与 `T_DRAWING` 配对 | ✅ | 装置组下的图纸行 |
| 25 | 图纸图形 | 每张图的图形 | OCS `io::pid::load_pid` | ✅ | 双击图纸或「打开图纸」→ 新标签页（Vulkan 下看到了图；DX12 见 §1.3） |
| 26 | 图内图元与属性 | `.pid` 自带的发布数据：11 种类型，14 894 个图元 | `pid_load::load_publish_from_pid` → `PidPublishDrawing.items` | ✅ | 图纸 → 类型 → 图元 → 属性（修复之后）。**已接有缺**：标签是 UID，键值重叠 |
| 27 | 图内连接、端口、管线、OPC、图形表示、统计 | `PidPublishDrawing` 的 `process_points`、`branch_points`、`ports`、`connectivity_ends`、`exp_relations`、`opc_points`、`pipelines`、`representations`、`stats` | 同上；配合库还有 `attach_oracle_ports`、`pipeline` | ❌ | **未接**：读图元时这些字段一并读出来了，随后丢掉 |
| 28 | 解析诊断 | 读不出的原因、导入摘要（画出 / 解码 / 未画出 / 图层）、未解码记录 | `PidLoadError`、OCS `ImportSummary`、`pid_parse::inspect`、`byte_audit` | ◐ | 读不出的原因在图纸详情和命令行（修复之后）；导入摘要只在打开之后的命令行里；`inspect` **未接** |
| 29 | `SPEMDataMap.xml` | `PlantData~2~722`，711.zip 里还有一份 | 无专用接口 | ◐ | 只是一个文件行，内容看不了 |
| 30 | 符号库 | `RefData~4~681.zip`：644 个 `.sym` + `CatalogIndex.xml` | `symbol_library::read_symbol_geometry(路径)`；`CatalogIndex.xml` 的解析钉版没有 | ◐ | 包旁有解压好的 `Ref\` 时「参考数据」里是文件树（名字、大小）；原样的包是空的；没有预览。**已接有缺** |
| 31 | 图框模板 | `RefData~4~682.zip`：10 个 `.igr` + 10 个 `.pid` | `.igr` 能否用 `read_symbol_geometry` 读，未验证 | ◐ | 只有成员名 |
| 32 | 规则文件 | `RefData~4~680` = `rules.rul`（332 KB 文本） | 无 | ◐ | 一个不知其名的文件行，只有大小 |
| 33 | 样式库 | `RefData~4~683` = `ProjectStyles.spp`（OLE 容器） | 无专用接口：`style_link` 解的是图纸自带的样式表，并记下它来自哪个 `.spp`；`style_names_for_file` 能否直接读 `.spp` 未验证 | ◐ | 同上 |
| 34 | 报表模板 | `RefData~4~684.zip`：10 个 `.xlsm` | 无 | ◐ | 成员名 |
| 35 | 装配 | `RefData~4~685.zip`：3 个空目录 | — | ✅ | 成员名（本来就没有内容） |
| 36 | 图层导出表 | `RefData~4~703` = `exportlayer.xlsx` | 无 | ◐ | **已接有缺**：显示成「Zip，12 个成员」 |
| 37 | 保温规范 | `RefData~4~709` = `InsulationSpec.isl`（XML） | 无 | ◐ | 文件行，只有大小 |
| 38 | 数据映射 | `RefData~4~804.zip`：`SPPIDDataMap.xml` 及 7 个变体、`CatalogIndex.mdb` | `backup::read_datamap(路径)` → `DataMap { enum_to_enum, enum_list_names }` | ◐ | 成员名；映射内容**未接** |
| 39 | 导入映射 | `RefData~4~809.zip`：4 个 XML | 无 | ◐ | 成员名 |
| 40 | 登记但缺失的文件 | `RefData~4~824` = `JacketNDCoreND.XML`，备份时就没取到 | Manifest File 行的状态 | ✅ | 文件行标「登记但缺失」，根节点有警告 |
| 41 | 索引警告 | 缺文件、SQL Server 库未读、无法配对的图纸 | `ScanProblem` + OCS 自己的检查 | ✅ | 根节点详情的「警告」 |

合计 41 类：看得到 11 类（5、6、8、10、11、24、25、26、35、40、41），看得到一部分 13 类，看不到 17 类。看不到的 17 类里，12 类是数据库表（#9、#12–#22），另外 5 类是 Manifest 的对象清单与连接信息、根目录文件、SQL Server 库、图内连接数据。

## 3. 方案

### 3.1 做法

- **不加新面板、新窗口、新消息。** 一切都落在现有的「P&ID 备份」面板里：树上多几支，详情区多一种表格显示。停靠面板可以浮出、拖边角拉大（`src/ui/dock.rs` 的 `FloatOut`、`FloatResizeGrab`；读代码所得，没有在窗口里试），宽表靠这个。这样 `src/app/mod.rs`、`src/app/update/mod.rs`、`src/app/commands/mod.rs` 三个文件一行都不用动。
- **新代码放新文件。** `pid_backup_browser.rs` 已经 79 KB，`pid_backup_browse.rs` 1 664 行。新东西放进同名子目录：`src/ui/window/pid_backup_browser/*.rs`、`src/io/pid_backup_browse/*.rs`、`src/io/pid_backup/*.rs`、`src/app/commands/pidbackup/*.rs`（`audit.rs` 已经在这儿）。
- **先开扩展点（W0），再并行。** 现在每加一种节点都要改 `NodeId`、`visible_rows`、`detail_lines`、`update` 四处的 `match`，几个人同时加就会互相撞。W0 把「一支子树」收成一个接口，之后每个工作项只加一个文件和注册表里的一行。
- **重活在后台线程，结果缓存在面板状态里**，沿用读图元的做法（`load_items_in_background` + `ItemsState`）。
- **每个工作项都要让走查测试覆盖到。** `audit.rs` 现在断言「没有不带详情的节点、没有静默失败」；每加一类数据，走查就多走一支，报告里多一段。

### 3.2 工作项

规模：S 约 150 行以内，M 150–500 行，L 500 行以上（含测试）。

| # | 工作项 | 解决清点表的 | 依赖 | 规模 | 要动 pid-parse |
|---|---|---|---|:-:|:-:|
| W0 | 扩展点：子树接口与注册表 | —（给后面铺路） | — | M | 否 |
| W1 | 详情区：键值不重叠、选中高亮、表格显示、图元用位号做标签 | #26 的缺口；所有表格类数据的载体 | W0 | M | 否 |
| W2 | Manifest 全量 + 备份文件显示原名 | #1–#4、#32–#37 的名字、#36 的误标 | W0 | M | 否 |
| W3 | 根目录其他文件 + 文本预览 | #7、#29、#32、#37、#39 的内容 | W0 | S | 否 |
| W4 | 数据库表目录与行数 | #9 | W0 | S | 否 |
| W5 | 数据库：钉版已有的类型化视图 | #14（管线）、#15、#17、#22（代码表） | W1 | M–L | 否 |
| W6 | 图内数据补全 | #27 | W1 | M | 否 |
| W7 | 图元与图形联动 | #25、#26 之间 | W6 | M | 否 |
| W8 | 还原时把 RefData 卷放回 `Ref\` | #30、#31、#38 在原样的包里也有；导入用上包里的符号 | — | M | 否 |
| W9 | 符号与模板预览 | #30、#31 | W8 | M | 否（`CatalogIndex.xml` 的目录要换钉版） |
| W10 | 数据映射表 | #38 | W1、W8 | S | 否 |
| W11 | SQL Server 包 | #23 | W1，D4 | L | 可能（见 §3.4） |
| W12 | 解析诊断 | #28 | W1 | M | 否 |
| W13 | 搜索扩到图号、位号、文件名 | — | W6 | S–M | 否 |
| W14 | 还原缓存：位置可配、可清理 | §1.3 第 9 条 | — | S–M | 否 |
| W15 | 任意表的内容浏览 | #12、#13、#16、#18–#21，以及 #14、#15、#22 的其余表 | W4、W1 | L | **是** |
| W16 | 文案与图标 | §1.3 第 7、8 条；分支自己的三个旧图标 | — | S | 否 |

### 3.3 逐项说明

**W0 扩展点。** 新文件 `src/ui/window/pid_backup_browser/section.rs`：一支子树实现「给出自己的行」「给出某个节点的详情」「处理点到自己头上的消息并返回 `Effect`」三个函数；`NodeId` 增加一个通用变体（子树编号 + 下标路径），老变体不动。`visible_rows` / `detail_lines` / `update` 在各自 `match` 的末尾转给注册表。现有三支（备份文件、装置组、参考数据）先不搬，等后面的工作项碰到再搬。测试：一支假子树，验证行、详情、折叠、过滤都经过它。

**W1 详情区。** 改 `pid_backup_browser.rs` 里的 `kv`、`tree_row`、`details` 三个函数，新文件 `pid_backup_browser/grid.rs` 放表格。
- 键值行：键列随面板宽度取 30%–40%，键和值都换行；键比列宽长就上下排。
- 选中行加底色（照 `sheet_set.rs` 的做法：选中时用 `palette().primary.weak` 作背景）。
- `DetailLine` 增加一种「表」：列名 + 行，带横向滚动，点列名排序，行数多时只建可见的行（树现在是全量 `column`，742 行还撑得住，47 000 行不行）。
- 图元行的标签改成位号 / 名字，UID 留在详情里（D8）。
- 测试沿用 `detail_rows` 的纯函数断言，外加 `iced_test` 里按画出来的位置点击（这次修箭头时已经有先例）。

**W2 Manifest。** 新文件 `src/io/pid_backup_browse/manifest.rs`（模型：概况、按库分组的表和视图、权限、文件登记与 FileSize 合并、连接信息）和 `pid_backup_browser/manifest.rs`（`Manifest.txt` 行可以展开：概况 / 数据库对象 / 文件登记 / 连接信息）。`describe_files` 用文件登记给每个备份文件补上原始相对路径（`RefData~4~680 — Ref\rules.rul`），格式一栏按原名的扩展名说话，`exportlayer.xlsx` 不再叫 Zip。口令类字段按 D6 遮住，测试里钉死「详情区和报告里不出现原文」。

**W3 其他文件与文本预览。** 新文件 `src/io/pid_backup_browse/loose_files.rs`：包根目录里 `scan_backup_folder` 没认领的文件（`PlantConfig.xml`、`Export.log`）也列进「备份文件」。`pid_backup_browser/text_preview.rs`：文本类文件（按内容嗅探 UTF-16 / UTF-8 / GBK）在详情区显示前若干 KB，zip 成员按需解出再显示。`rules.rul`、`InsulationSpec.isl`、各个映射 XML、`Export.log` 都靠它。

**W4 表目录。** 新文件 `src/io/pid_backup_browse/export_log.rs`：解析 `Export.log` 的 `about to export <库>'s tables` 与 `exporting table <名> <n> rows exported`（清点时已经用脚本做过一遍，数字对得上 Manifest 的 154 张表）。`Export.dmp` 行可以展开：库 → 表（行数）。没有 `Export.log` 时退回 Manifest 的表名，不带行数。

**W5 类型化视图。** 新文件 `src/io/pid_backup_browse/exp_views.rs` 与 `pid_backup_browser/database.rs`。`ExpTables::read` 整个转储读一次（后台线程，结果留在会话里；74 MB 的读取耗时和内存先量再定是否常驻），然后：
- 全厂级：管线表（`pipe_runs`）、端口表（`port_tables`）、组成关系（`composition_relations`）、代码表（`codelist_short_text`）。挂在 W4 的表目录上，对应的表行点开就是表格。
- 图纸级：某张图的连接端（`connectivity_ends`）、OPC（`opc_points`）。挂在图纸节点下。
- 表里的代码值能翻成文字的翻成文字（`flow_spelling`、`nominal_spelling`、代码表）。

**W6 图内数据。** 新文件 `src/io/pid_backup_browse/drawing_data.rs`：`load_items` 现在只留 `items`，改成把 `PidPublishDrawing` 的其余集合也带回来。`pid_backup_browser/drawing_data.rs`：图纸节点下除了「图元」再加「连接点」「端口」「管线」「OPC」「图形表示」「统计」，各是一张表。包里有 Oracle 转储时用库补全端口和管线（`attach_oracle_ports`、`pipeline`），这一步依赖 W5 读好的 `ExpTables`。

**W7 联动。** 导入的实体在 `PID_SEMANTICS` XDATA 里带着 `oid`（发布的 GraphicOID），`PidPublishRepresentation` 又给出图元 UID 与 `graphic_oid` 的对应，所以数据是够的。面板里选中图元 → 如果这张图已经打开，就缩放到对应实体：`run_pid_backup_effect` 里转给现成的 `Message::PidLegendJump { min, max }`，不加新消息；要不要同时把实体选中，做的时候看现有的选择接口。反方向（在图上选实体 → 面板定位到图元）要在选择变化处挂钩，会碰 `src/app/update/mod.rs`，建议留到那边的改动收尾之后。

**W8 RefData 还原。** 新文件 `src/io/pid_backup/refdata.rs`，`restore` 里调用。按 Manifest 的 File 行把每个 RefData 卷放回它原来的相对位置：目录卷（`Symbols`、`Template Files`、`Report Files`、`SmartPlant Resources`、`Import Map Files`）解到 `<缓存>\Ref\<原目录>`，单文件卷拷成原名。包旁已经有解压好的 `Ref\` 时照旧用它。全厂包的 RefData 解开共约 66 MB（FileSize 行的合计），其中符号 34 MB 现在已经随 `Ref\` 拷进缓存，所以净增约 32 MB。这样「参考数据」在任何包里都是完整的，导入也能用上包里的符号和模板，不必换钉版。

**W9 预览。** 新文件 `pid_backup_browser/symbol_preview.rs`：选中 `.sym` 时在详情区画缩略图。画法照 `pid_palette.rs` 的 canvas 单元格，几何来自 `read_symbol_geometry`。`.igr` 先验证钉版读不读得了；读不了就只列文件，等换钉版。模板里的 `.pid` 可以双击打开。

**W9 里符号的一半做了**（2026-11-21 加的，分支 `pid-symbol-on-main`，任务 SP-1，对应 issue #116 的第一步）。选中 `.sym` 时详情区出缩略图和图元统计，还原目录 `Ref` 下的文件、包内压缩包的成员、单独登记成包内文件的三处都行；做法、读数和没验证的记在 `docs/plans/2026-11-21-pid-sym-preview/todo.md`。和上面这段写的有两处不一样：画法没有照 `pid_palette.rs` 的单元格，画的是导入器摆图例本体的那段代码给出的图元，所以实心填充和文字都在；几何来自 `read_symbol_geometry_from_bytes`，压缩包里的成员不必先解压。§2 表里第 30 行的「没有预览」对 `.sym` 不再成立；同一行的 `CatalogIndex.xml`、第 31 行的 `.igr` 和模板 `.pid` 还没有做。

**W10 数据映射。** `src/io/pid_backup_browse/datamap.rs`：`read_datamap` 读 `SPPIDDataMap.xml`，在该文件的详情里显示枚举映射表（W1 的表格）。

**W11 SQL Server 包。** `src/io/pid_backup_browse/mtf.rs`：`open_mtf_dump_as_sqlite` 把转储变成内存里的 SQLite，再查出图纸和装置组，喂给现有的配对逻辑。两件事要先定：许可（D4），以及「列出全部图纸」的查询放在哪边：钉版只有按 UID 取单张图的函数，OCS 自己写 SQL 就要直接依赖 `rusqlite`，否则请 pid-parse 加一个列表函数。

**W12 诊断。** `src/io/pid_backup_browse/diagnostics.rs`：图纸节点下加「诊断」：读图元的结果、`ImportSummary`（要导入一次，单张 0.01–6 s）、`pid_parse::inspect` 的报告文本。根节点加「检查全部图纸」：后台逐张读图元并导入，出一张表（哪张读不出、哪张导不进、各自原因），就是现在走查报告的界面版。全厂包约 40 s，所以是手动触发、带进度（D9）。

**W13 搜索。** 过滤现在只看图纸名，而且会把「备份文件」和「参考数据」两支藏起来。扩到图号、标题、备份文件与参考文件的名字，以及已经读过图元的图纸里的位号；匹配到图元时展开到那一行。

**W14 缓存。** `src/io/pid_backup.rs` 的 `restore_cache` 读一个设置项决定根目录（默认不变）；根节点详情显示缓存大小，加「清理缓存」。设置项落在 `src/app/config.rs`，选项页要不要加入口见 D5。

**W15 任意表。** 需要 pid-parse 把现在私有的 `Table` / `read_table_tolerant` 公开成「按表名读出列名和行」。有了它，W4 目录里的每张表都能点开（W1 的表格，分页或只建可见行）。

**W16 文案与图标。** 「尚未打开备份。」去重；路径分隔符统一；同组图纸按名字排；读不出、打不开的原因包一层本地化的说法，缓存路径换成图纸名；`pid_palette.svg`、`ui/collab.svg`、`ui/folder.svg` 三个图标按上游这次重画的 16 px 风格重画（现在只是补了宽高让测试通过）。

### 3.4 需要 pid-parse 配合的

| 事项 | 要什么 | 影响的工作项 |
|---|---|---|
| 任意表读取 | 公开一个按表名读表的函数（列名 + 行）。钉版和本地最新（`59e3469`，比钉版多 190 个提交）都没有 | W15 |
| SQL Server 库的图纸列表 | 一个「列出全部图纸 / 装置组」的函数，或者同意 OCS 直接用 `rusqlite` 查 | W11 |
| `CatalogIndex.xml` 目录、直接从 RefData zip 读 `.igr` / `.sym` | 本地最新已有（`publish::catalog_index`、`refdata_archive`），要换钉版才用得上 | W9 的目录部分；W8 可以因此省掉解压 |

换钉版本身是 D3。

### 3.5 顺序与并行

```
W0 ──┬── W2 ─┐
     ├── W3  │
     ├── W4 ─┼── W15（等 pid-parse）
     └── W1 ─┼── W5 ── W6 ── W7
             │         └──── W13
             ├── W10（还要 W8）
             ├── W11（等 D4）
             └── W12
W8 ── W9
W14、W16 随时
```

- W0 一个人先做完。之后 W1、W2、W3、W4、W8、W14、W16 七项互不相干，可以同时开。
- W1 落地之后 W5、W6、W10、W12 可以同时开。
- 会相撞的只有本地化：新标签都是 `pid-backup` 这一条 Fluent 消息的属性，要同时改 `src/locale_catalog.rs`、`src/i18n.rs` 的键清单和 23 个 `locales/*/opencadstudio.ftl`。建议每个工作项把自己的键集中成一段追加在末尾，合并时只是相邻追加。

### 3.6 怎么算做完

- 清点表 41 行每一行在界面上都有去处；确实做不了的（等 pid-parse 的那几类）在面板里说明「这类数据暂不能显示」而不是不出现。
- `app::commands::pidbackup::audit` 对三个包都通过，报告里每类数据有一段。
- 真实窗口里对全厂包逐类点开，截图留在 `target\ui-audit`，中文界面下没有英文原句和重叠。

## 4. 需要拍板的

| # | 问题 | 选项 | 建议 |
|---|---|---|---|
| D1 | 表格类数据放哪儿 | (a) 仍在「P&ID 备份」面板里，宽表靠浮动拉大；(b) 另开一个数据窗口或数据标签页 | **(a)**。不碰那三个正在改的文件，一个面板一套状态；(b) 看宽表更舒服，但要加窗口种类和消息，正好落在冲突区。先做 (a)，表格做成独立部件，以后要搬到窗口里只是换个容器 |
| D2 | 数据库表做到什么程度 | (a) 这一轮做表目录、行数和钉版已有的类型化视图（W4、W5），任意表等 pid-parse（W15）；(b) 等 pid-parse 出了通用读法再一起做；(c) OCS 自己再写一个 exp 解析器 | **(a)**。154 张表里只有 13 张钉版读得了；(a) 先让人看到库里有什么、多大，以及最有用的管线、连接、OPC、代码表。(c) 是把 pid-parse 的活重做一遍，不建议 |
| D3 | 换不换 pid-parse 钉版 | (a) 这一轮不换，参考数据靠 OCS 自己解压（W8）；(b) 换到最新 | **(a)**。最新比钉版多 190 个提交，换了之后 `pid_import` 54 条、`pid_legend` 29 条以及出图的对比都要重新过一遍，应该单独做、单独验。W8 之后原样的包也有完整的参考数据，不依赖换钉版 |
| D4 | SQL Server 包读不读 | (a) 读（W11），GPL-3.0 的 `oxidized-mdf` 继续链在程序里；(b) 维持「按文件夹列出」，并按 10-16 方案的 D5 把 `backup-mdf` 摘掉；(c) 维持现状，许可的事另议 | 这是许可问题，要你定。现状是 `Cargo.toml` 用默认特性，GPL 的部分已经链进来了，10-16 的 D5 没有落到 main 上。pid-parse 自己的说明是「内部使用可以，对外分发要先对齐许可」：照这个说法，只在内部用选 (a)，工作量 L；要对外分发选 (b) |
| D5 | 还原缓存 | (a) 默认位置不变，加设置项和「清理缓存」；(b) 默认改到别的盘或数据目录；(c) 不动 | **(a)**。这台机器 C 盘一度只剩 0.9 GB，全厂包一次还原约 130 MB，做了 W8 之后约 170 MB。入口先只放在面板根节点的详情里，选项页以后再加 |
| D6 | Manifest 里的口令 | (a) 一律遮住，只显示「有 n 项」；(b) 遮住，但给一个「显示」开关 | **(a)**。它们只是混淆过，不是加密；截图、报告、日志都可能带出去 |
| D7 | DX12 视口不出图 | (a) 另立一项先查；(b) 分支里先把 Windows 的默认探测顺序改成 Vulkan 在前；(c) 不管，验收时加 `--backend vulkan` | **(a)**，查清之前验收用 (c)。它不是备份功能的问题，也不是这次合并带来的，但「双击开图能看到图」是验收的一部分，本机默认就踩中。只在这一台机器（AMD RX590）、dev 配置的构建上见过，别的显卡和发布构建没试 |
| D8 | 图元行显示什么 | (a) 位号 / 名字，没有就显示类型加序号，UID 放详情；(b) 保持 UID | **(a)** |
| D9 | 全部图纸的体检 | (a) 手动触发，带进度；(b) 打开备份后自动在后台跑 | **(a)**。全厂包要 40 s 左右，占一个核 |

## 5. 附

### 5.1 全厂包的数据库表（取自 `Export.log`）

| 库 | 表数 | 行数 | 有数据的表 |
|---|---:|---:|---|
| SQPLANTPID（P&ID 应用数据） | 82 | 324 925 | 46 张：T_MODELITEM 47124、T_REPRESENTATION 42540、T_HISTORY 30931、T_RULEREFERENCE 28460、T_RELATIONSHIP 24664、T_SIGNALPOINT 22057、T_SYMBOL 18199、T_PROPERTYBREAK 14963、T_PLANTITEM 13687、T_LABELPERSIST 12584、T_LEADERVERTEX 12584、T_CONNECTOR 11757、T_PIPINGPOINT 9628、T_CONNECTORVERTEX 5181、T_PIPERUN 4686、T_INCONSISTENCY 4624、T_INLINECOMP 4533、T_INSTRFUNCTION 3718、T_INSTRUMENT 3718、T_PIPINGCOMP 3503、T_SIGNALRUN 1001、T_OPC 952、T_ITEMNOTE 800、T_NOTE 800、T_EQUIPMENT 402、T_NOZZLE 377、T_CASEPROCESS 301、T_EQUIPMENTOTHER 225、T_CASE 209、T_MECHANICAL 94、MAX_ID 80、T_OPTIONSYMBOLOGY 77、T_VESSEL 71、T_OPTIONSETTING 66、T_DRAWING 53、T_DRAWINGPROJECT 53、T_GLOBALDRAWING 53、T_OPTIONAUTOGAP 44、T_OPTIONFORMAT 39、T_OPTIONHEATTRACE 22、T_DRAWINGVERSION 15、T_EXCHANGER 12、T_SMARTFRAME 12、T_SMARTFRAMESTORAGE 12、T_OPTIONDISPSET 11、T_OPTIONDISPSETFOLDER 3 |
| SQPLANTPIDD（P&ID 数据字典） | 25 | 39 197 | 25 张：ITEMATTRIBUTIONS 29824、CODELISTS 3407、UNIQUEATTS 1527、ATTRIBUTES 1004、SPTPATTRIBUTES 700、SOURCEDESTOBJECTRELS 512、FILTERCRITERIA 401、FILTEREDRELATIONSHIP 383、FILTERINSTANCE 247、FILTERDEFINITION 242、CATEGORYFILTER 223、ENUMERATIONS 133、RELATIONSHIPS 101、SPSORTS 87、SPTPVIEWS 81、ENTITIES 80、ITEM 80、SPTPLAYOUTS 69、CORRELATTS 38、MAX_ID 24、CATEGORY 23、APPLICATIONS 5、VERSIONINFO 4、INTERSECTS 1、VERSION 1 |
| SQPLANTD（工厂数据字典） | 25 | 858 | 15 张：ITEMATTRIBUTIONS 354、UNIQUEATTS 131、CODELISTS 130、ATTRIBUTES 85、MAX_ID 24、ENTITIES 20、ITEM 20、SPTPVIEWS 20、RELATIONSHIPS 18、SOURCEDESTOBJECTRELS 16、ENUMERATIONS 13、CORRELATTS 12、VERSION 6、APPLICATIONS 5、VERSIONINFO 4 |
| SQPLANT（工厂） | 22 | 550 | 16 张：T_FORMATS 385、T_DRAWINGSITE 53、T_FORMATTYPES 41、MAX_ID 20、T_PLANTSETTING 14、T_ROLEACCESS 12、T_PLANTGROUP 6、T_PROJECTPLANTGROUP 6、T_HIERARCHYLEVELS 3、T_UNIT 3、T_AREA 2、T_ACTIVEWSSITE 1、T_APPPROJECT 1、T_PLANT 1、T_PROJECT 1、T_WSSITE 1 |
| 合计 | 154 | 365 530 | 102 张 |

钉版 `ExpTables::read` 读其中 13 张：`T_PIPINGPOINT`、`T_SIGNALPOINT`、`T_CONNECTOR`、`T_REPRESENTATION`、`T_PIPERUN`、`T_SIGNALRUN`、`T_OPC`、`T_NOZZLE`、`T_NOTE`、`T_ITEMNOTE`、`T_MODELITEM`、`CODELISTS`、`T_DRAWING`；`read_plant_groups` 另读 `T_PLANTGROUP`。

### 5.2 复现

工具链用 rustup 的 `stable`（1.99.1），不用 PATH 上那个改过的 cargo；`CARGO_TARGET_DIR` 指到本工作树的 `target`；`TEMP` / `TMP` 指到 D 盘（还原缓存写在临时目录下）。

```
rustup run stable cargo test --lib --no-run --offline -j 10
# 经应用走查一个真实备份包，写报告和面板图片
set OCS_PID_BACKUP_AUDIT=D:\work\plant-code\cad\pid-test-data
set OCS_PID_BACKUP_AUDIT_OUT=<输出目录>
set OCS_PID_BACKUP_AUDIT_LANG=zh-CN
<lib 测试可执行文件> app::commands::pidbackup::audit --ignored --nocapture
# 原有的两条真实数据测试
<lib 测试可执行文件> plant_test_backup --ignored --nocapture
```

`cargo test` 每次都会重编根 crate（`build.rs` 监视的 `packaging/windows/AppIcon.ico` 在调试构建里不存在），所以上面直接运行 `--no-run` 产出的可执行文件。

真实窗口：`OpenCADStudio.exe <图纸> --backend vulkan --http <端口>`，环境变量 `OCS_API_TOKEN` 给令牌，`POST /api/v1/commands {"cmd":"PIDBACKUPOPEN <路径>"}`，`POST /api/v1/ui_bounds {"prefix":"pid-backup-row-"}` 取行的位置，`POST /api/v1/capture` 存图。面板里的点击只能靠窗口消息：自动化通道的 `click` / `pointer` 只管绘图区。

### 5.3 这次留下的图

都在本工作树的 `target\ui-audit` 下（不入库）：

| 目录 | 内容 |
|---|---|
| `headless-plant`、`headless-plant-zh`、`headless-dwg0202`、`headless-test02` | 走查报告 `report.txt` 与面板在各状态下的图 |
| `gui-plant`、`gui-plant-search`、`gui-dwg0202`、`gui-test02` | 真实窗口逐步截图；`*-screen.png` 是屏幕上实际看到的 |
| `gui-plant-baseline` | 基线程序：图纸行没有箭头 |
| `gui-backends` | 基线与合并后、dx12 与 vulkan 各自的窗口 |
| `gui-close` | 执行过命令后关窗口弹出的「未保存的更改」 |

## 6. 执行记录

D1–D9 还没有拍板。第一批（2a）按 §4 的建议先走：D1 (a)、D2 (a)、D6 (a)、D8 (a)、D9 (a)；D3、D4、D7 不动，W11、W15 不做。每项一个提交。

| 项 | 提交 | 做了什么 | 清点表的变化 |
|---|---|---|---|
| W0 | `10c32a621`「P&ID 备份面板：子树扩展点（section）」 | `pid_backup_browser/section.rs`：一支子树一个 `Section`（挂在哪一行下面、自己的行、节点详情与给别的节点追加的详情、要在后台读的数据），`NodeId::Section(槽位, 路径)`；`visible_rows` / `detail_lines` / `update` 各自在末尾转给注册表；后台读取走 `Effect::LoadSection` → `PidBackupMsg::SectionLoaded`，带代数，换了备份之后迟到的结果丢弃。注册表此时是空的，面板没有可见变化。 | 无 |
| 附：`Select` | `4fd4e7c3b`「PIDBACKUPOPEN Select：命令行按标签路径定位节点」、`ebca0f068`「…一段路径只写名字就够」 | 方案里没有的一项，为的是不动鼠标也能看真实窗口：`PIDBACKUPOPEN Select 装置组 > SQPlant > test > U01 > D07 > #1 > #1` 逐级展开并选中，与逐级点击留下的状态相同（`pid_backup_browser/goto.rs`）；遇到要先读图元的图纸就等读完再接着走。真实窗口的检查因此可以只靠自动化通道发命令加 `/capture`。 | 无 |
| 附：修改标记 | `d93d6b909`「停靠面板不再把图纸标为已修改」 | §1.3 里「`PIDBACKUPOPEN` 之后图纸带上修改标记」的原因：面板停靠时 `dock_keep_view`（`src/app/dock/update.rs`）把相机平移半个面板宽、让图在屏幕上不动，下一个 tick（`src/app/update/viewport.rs` 的 `on_tick`）发现相机变了，写回文档并置 `dirty`。与备份无关，任何停靠面板都如此。改在 `dock_keep_view`：相机原本与文档一致时，平移后当场写回文档并记为已同步，不置 `dirty`。三个约定不动的文件没有碰。 | 无（§1.3 的一条） |
| W1 | `fc37fbaf2`「P&ID 备份面板：详情区键值不再重叠、选中行有底色、图元行显示位号、详情区表格」 | 键列取面板宽度的 36%（96–240 px），键比键列长就独占一行、值在下一行的值列，两者都在自己的框里换行；选中行用调色板的 `primary.weak` 作底色；图元行显示发布出来的名字（`IObject.Name`，注释取 `INote.NoteText`），没有名字的显示「类型 #序号」，UID 放进详情（`io/pid_backup_browse/labels.rs`）；`DetailLine::Table` 与独立的表格部件 `pid_backup_browser/grid.rs`（表头固定、列宽按内容、横向滚动、点列名排序、只建可见的行），图元类型的详情用它列出该类型的全部图元。全厂包 14 894 个图元里 3 425 个有名字。 | #26 的两处缺口（标签是 UID、键值重叠）补上，状态仍是 ✅；§1.3 的 1、2、3 条 |
| W2 | `483afcbbb`「P&ID 备份面板：Manifest.txt 全量可看，备份文件显示原名，口令一律不出现」 | `io/pid_backup_browse/manifest.rs` 把 Manifest 的每一行归位（概况、表 / 视图 / 角色 / 权限 / 数据库文件、文件登记与 FileSize 合并、连接信息），口令类字段在读入时就换成 `•••`，之后的索引、面板、日志都拿不到原文；`pid_backup_browser/manifest.rs` 是第一支 section：`Manifest.txt` 行可以展开成 概况 / 数据库对象 / 文件登记 / 连接信息。备份文件行显示「存储名 — 原始路径」（`RefData~4~680 — Ref\rules.rul`），详情里有来源路径、源内文件数与目录数、打包前大小；单个文件按原名的扩展名说格式，不再把 `exportlayer.xlsx` 叫成有 12 个成员的 Zip。 | #1 ◐→✅，#2 ❌→✅，#3 ◐→✅，#4 ❌→✅（口令按 D6 遮住）；#32、#33、#36、#37 的文件现在看得出是什么（内容仍看不了，状态不变）；§1.3 的第 5 条 |
| W3 | `3ccc72782`「P&ID 备份面板：包根目录的其他文件也列出来，文本文件点开就能读」 | `io/pid_backup_browse/loose_files.rs` 把扫描没认领的根目录文件（`Export.log`、`PlantConfig.xml`）列进「备份文件」，排在转储之后。`io/pid_backup_browse/text_file.rs` 读文本：编码按字节判断（UTF-16 / UTF-8 的 BOM，其次合法 UTF-8，再次 GBK，最后 Windows-1252），最多读 8 MiB（已见语料最大的文本是 3.7 MB 的数据映射），二进制文件只看开头 8 KB。`pid_backup_browser/text_preview.rs` 是第二支 section，不加行，只给已有的三种文件节点（备份文件、归档成员、参考文件）的详情加内容：编码、行数、逐行的表格（行号 + 文本，横竖都能滚，只建可见行）；不是文本的文件明说「不是文本文件」。读取在工作线程上，只保留当前选中文件的文本（`Section::keeps_only_selected`）。`Manifest.txt` 不走这条路（内容在它自己的子树里，口令已遮住）；别的文本里长得像口令的值同样换成 `•••` 并说明换了几处。 | #7 ❌→✅，#29 ◐→✅，#32 ◐→✅，#37 ◐→✅，#39 ◐→✅；#38 的 8 份映射 XML、#30 的 `CatalogIndex.xml` 原文可读（结构化视图仍未接，状态不变）；#6 的文本成员能看内容 |
| W4 | `46d687a30`「P&ID 备份面板：Export.dmp 展开成库和表，带行数」 | `io/pid_backup_browse/export_log.rs` 解析 `Export.log`（`about to export <库>'s tables`、`exporting table <名> <n> rows exported`，分区表按分区累加，`EXP-` / `ORA-` 消息、连接的数据库、字符集、导出结果一并取出），再与 Manifest 的 `Table` 行不分大小写地合并成 `TableCatalog`：库按 Manifest 的顺序和写法，表按名字。`pid_backup_browser/database.rs` 是第三支 section：`Export.dmp` 行可以展开成 库（表数）→ 表（行数），空表置灰；转储的详情增加库数、表数、有数据的表、总行数、导出信息和各库一览表；库的详情是它的表（可按表名或行数排序）；表的详情是库、表名、行数。没有 `Export.log` 的包（SQL Server）只列表名并说明原因。全厂包：4 库 154 表 365 530 行，与清点一致。 | #9 ❌→✅（SQL Server 包只有表名，没有行数）；#12–#22 的表现在看得到名字和行数，内容仍等 W15，状态不变 |
| 附：排版 | `f30261921`「本批新建的 12 个文件过一遍 rustfmt（只动排版）」 | CI 的 lint 一步目前只是参考，但新文件不再往上添差异。本批之前就有的文件没有动。 | 无 |

### 6.1 第一批（2a）之后的清点

| | 第一阶段 | 2a 之后 | 变化的行 |
|---|---:|---:|---|
| ✅ 看得到 | 11 | 21 | 新增 #1、#2、#3、#4（口令遮住）、#7、#9、#29、#32、#37、#39 |
| ◐ 看得到一部分 | 13 | 7 | 剩 #28、#30、#31、#33、#34、#36、#38 |
| ❌ 看不到 | 17 | 13 | 剩 #12–#22（表的内容，等 W15；表名和行数经 #9 看得到）、#23、#27 |

### 6.2 这一批收尾时的验证

- 全量：18 个测试可执行文件 **6406 过 / 3 失败 / 43 忽略**（第一阶段 6313 / 3 / 43）。失败的是 `app::collab::transport` 的 2 条和 `io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text`。
- `app::collab::transport` 那一组：库测试整体又跑 4 遍，每遍这一组失败 4 条（第一阶段是 3 条，多出来的是 `cancelling_the_subscription_ends_the_thread_promptly`）；这一组单独跑 3 遍，14 条全过。与机器当时的负载有关（同时还有别的仓库在编译）。
- `filled_ttf_glyphs_export_as_real_text`：5 遍库测试里失败 2 遍；单独跑 5 遍、连同所在模块跑 3 遍都过。原因是上游测试之间的竞争，与本批无关：这条测试先把字形排进进程级的字形图集、放开锁、再导出；而每一条构造应用的测试（`new_for_test` → `apply_config` → `apply_settings` → `set_textfill` → `reset_font_entries`）都会把这个图集清空。只把这条测试和 41 条上游的应用测试（`app::viewport_dimension_tests`、`app::navigation::tests`）放进同一个进程，30 遍里失败 28 遍。本批没有碰图集、字体和 PDF 导出。没有改它：修法（按图集代数重试，或给两边加一把测试锁）属于上游的测试代码。
- 第一阶段偶发过一次的两条（`quad_feed_has_no_v14_auto_tab_per_plugin`、`a_layer_switched_off_inside_a_block_patches_the_references_in_place`）：5 遍库测试里各过 5 次。
- 走查（含逐张导入）三个包都过，图纸、图元、导入的数字与第一阶段相同；口令检查三个包各 0 处命中。
- 非测试构建的告警 19 条、库测试构建 28 条，与第一阶段相同。

### 6.3 这一批留下的已知不足

- 详情区在面板下半截（树 3 : 详情 2）。全厂包 `rules.rul` 在 889 px 高的窗口里只露出 4 行文本，82 张表的库露出 8 行。能滚动，但读长文件吃力。属于 D1 的范围：面板里加可拖动的分隔条，或把表格弹到独立窗口。
- `Select` 走到的节点、以及点选靠下的行之后，树不会自动滚到选中行。
- 同组图纸仍按库里的行序排；关闭备份后的重复提示、路径分隔符不一致（§1.3 第 7 条）没有动。

### 6.4 第二批（2b）

D1 按 (a)（仍在现有面板里，表格保持独立部件），D5 按 (a)（默认位置不变，加设置项和「清理缓存」）。D3、D4、D7、W11、W15 仍然不做；W9、W7、W13 留给下一批。

| 项 | 提交 | 做了什么 | 清点表的变化 |
|---|---|---|---|
| 0 真实窗口工具 | 「docs/automation：隐藏桌面上驱动真实编辑器窗口的脚本与说明」 | 2a 里用来「不动鼠标、不抢焦点地看真实窗口」的办法入库：`docs/automation/hidden_desktop.py`（库）、`hidden_steps.py`（按 JSON 步骤驱动一个或几个实例）、`hidden_steps_example.json`、`guides/hidden-desktop.md`（一页说明：起实例、发命令、截图、确认指针和前台窗口没变、收尾、同时起几个实例、做不到的事）。相比 2a 的草稿：路径一律转成绝对路径；编辑器挂在作业对象上，脚本怎么结束它都跟着结束；能同时起多个实例并给每个实例不同的命令行；自己核对运行前后的指针与前台窗口。 | 无 |
| 1 测试之间的竞争 | 「并行测试共用进程级状态造成的偶发失败：六条测试各自按根因修」 | 库测试并行跑时偶发失败的六条上游测试，根因是四种进程级状态被别的测试动了：字形图集（每条构造应用的测试都清空它）、语言代数（同样每次构造都加一）、quad 面板存储（两条测试共用插件名 `PLUGIN`）、插件错误队列（每个应用的 tick 都清空它）。另有一条是时间门限（一次指针移动处理超过 8 ms，下一次移动被合并）。只动测试代码；`sdf_atlas.rs` 加的是 `#[cfg(test)]` 的辅助函数，`plugin/mod.rs` 把错误队列的取用收成一个函数、测试构建里按线程各一份，产品构建的行为不变。修前修后的失败次数见 §6.5。 | 无 |
| 口令：文本预览 | 「文本预览：Manifest 的副本与别的文件里的口令也遮住」 | 先确认：`Manifest.txt` 那一行从不被读成文本（内容只在它展开的子树里，口令已遮住），三个包的走查各有一条断言。再补两处漏洞（先红后绿）：一份行尾只有回车的 Manifest 副本，整份被当成一行，按位置认定的口令字段露了原文——现在回车、换行、回车加换行都算行尾；别的文件里叫作口令的值（`Password=…`、`<Pwd>…</Pwd>`、`"password": "…"`、`key="DbPassword" value="…"`、连接串里的 `Pwd=…;`）原样显示——现在一律换成 `•••` 并计数。三个包里能被预览打开的文本逐个扫过（脚本只报位置和长度）：除 `Manifest.txt` 本身外没有一处口令，新规则在真实文件上一处都没有遮（`ObjectAttrValidation.xml` 的 `PassValue` 不算口令）。 | 无 |
| 2 详情区 | 「P&ID 备份面板：树与详情之间的分隔条可拖动，选中行自动滚入视野」 | 树和详情改成一个 pane grid 的两格，中间 7 px 的缝就是可拖的分隔条（悬停时高亮，详情顶上另有一条细线标出位置）；比例记在面板状态里（`split::Split`，默认仍是 3 : 2，范围 12%–88%，每一半不小于 48 px），打开、刷新、关闭备份都不改它。`PIDBACKUPOPEN Split <百分比>` 从命令行设比例，脚本和隐藏桌面上的真实窗口靠它。选中行滚入视野用的是一个控件操作（`split::reveal_selected`）：在排好版的面板里找到树的滚动区和选中行，行不在视野里才滚，滚到刚好露出整行；它读的是实际布局，折行的长标签也算得对。点选、`Select` 走到的节点都会触发。全厂包 `rules.rul`：889 px 高的真实窗口里默认比例露 4 行，`Split 30` 之后 14 行。 | 无（§6.3 的前两条） |
| W8 | 「P&ID 备份：还原时把参考数据卷放回 Ref，原样的包也有参考数据、图框也画得出来」 | 新文件 `src/io/pid_backup/refdata.rs`，`restore` 里调用：按 Manifest 的 File 行，把备份自带的每个 RefData 卷放回它在服务器上的相对位置——目录卷解到 `<还原目录>\Ref\<原目录>`，单文件卷拷成原名。相对位置按各卷所在目录的共同部分算（那个共同目录在还原里一律叫 `Ref`，导入器认这个名字），只有一个卷时也保留卷自己的名字；路径越出 `Ref` 的卷不还原。包旁已有的 `Ref\` 先拷过来，再由卷覆盖同名文件。还原出的图框模板也是 `.pid`，它们属于参考数据，不算图纸。缓存键加入版本号、Manifest 与各卷的大小时间，旧缓存自动重做。「参考数据」节点的详情多一行「参考数据目录」。 | #30、#31、#38 在原样的包里也有了（状态不变，预览与结构化视图仍未接）；§1.3 第 6 条。导入的变化：全厂包带图框的图纸 0 → 45 张（4 366 个图框图元），DWG-0202 包 0 → 4 张（309 个），TEST02 包 1 → 1 张；三个包图纸自身的图元数不变（149 310 / 1 697 / 920）；参考文件 645 → 695、0 → 677、0 → 662；还原缓存 130 → 160 MB、1 → 69 MB、67 → 133 MB |
| W6 | 「P&ID 备份面板：图纸下面多出工艺点、分支点、图形表示和读取统计」 | 读图元时一并读出来、随后丢掉的那些集合现在留下了：`io/pid_backup_browse/drawing_data.rs` 的 `DrawingData`（工艺点、分支点、图形表示、读取统计）随 `DrawingItems` 一起回来，凡是指向图元的地方都带上树里的叫法（`PT-101`、`PipeRun #3`）。`pid_backup_browser/drawing_data.rs` 是第四支 section，不读任何东西：图纸读完之后，它的图元类型下面多五行——「工艺点 (n)」「分支点 (n)」「图形表示 (n)」各是一张表（工艺点：所在连接器与取值依据；分支点：类与所画的图形表示；图形表示：所画的图元与图形记录的 OID，也就是导入的实体带着的那个 `oid`），「读取统计」是文件读出了多少（属性记录、未解码的记录、标签值的去向等八项），「端口、连接、OPC 与管线」是一行说明：这几类记在备份的数据库里，面板还没有读。空的列表置灰并明说「这张图没有」。文件里除图元外什么都没读出来的图纸不多出这些行。全厂包：51 张图纸共 4 678 个工艺点（全部落在图纸点了名的连接器上）、2 757 个分支点（管道 2 582、信号 175）、23 732 个图形表示（20 975 个画的是图元，2 757 个是管线上的分支点），153 726 条属性记录没有一条未解码。 | #27 ❌→◐：工艺点、分支点、图形表示、统计看得到；端口、连接端、组成关系、OPC、管线要读数据库转储，留给 W5 |
| W5 | 「P&ID 备份面板：读数据库转储——管段、端口、管嘴与注释、代码表，以及每张图纸在库里的那一面」 | 先量后做：全厂包的转储 74 489 856 字节，读文件加钉版认得的 13 张表用 1.0–1.7 秒（调试构建，随机器负载），读完私有内存多 123 MiB（转储原文已放掉，留下的是表），各视图算出来之后 145 MiB 上下；据此定为第一次用到时在工作线程上读、读完留在索引里直到备份关闭（`BackupIndex::dump`）。`io/pid_backup_browse/exp_views.rs` 把钉版的类型化投影收成视图。全库的挂在 W4 的表目录上，选中表就读：`T_PIPERUN`（管段：管线名、公称直径、介质、材料等级、顺序号、流体系统代码、保温规范；4 686 行，3 135 行属于某条管线）、`T_PIPINGPOINT` / `T_SIGNALPOINT`（端口 9 628 / 22 057 行）、`T_NOZZLE` / `T_ITEMNOTE`（管嘴 377、注释 114 条关系）、`CODELISTS`（83 张代码表 1 994 条短文字；两个库都有这张表，读取器读的是导出时排在前面的那个库，面板据 `Export.log` 的顺序认它）。单张图纸的挂在 W6 留的那一行下面，展开就读：端口、管段连接（哪个端口连在哪条管段的哪一端）、管嘴与注释、跨图连接符、管线（按钉版的规则合成，并说明多少连接器没有顺序号）；全厂包 51 张图共 31 578 / 19 374 / 486 / 476 / 2 395 行，单张最慢 0.2–0.5 秒。钉版对表是「全有或全无」：DWG-0202 包的 `T_PIPERUN` 第 0 行有一个不是整数的 NUMBER，整读失败——这种转储改为留下原文、按视图各读各的，读得出的照常显示（端口 93 + 151、管嘴 10、注释 2、跨图连接符 7），读不出的（管段、代码表、管段连接、管线）写明读取器停在哪里。SQL Server 的转储（TEST02）不读，每一处都说明原因。视图只有读取器认得的那几列，面板上明说「不是整张表」。 | #14 ❌→◐（管段）；#15 ❌→◐（端口两张表、每张图的管段连接；`T_RELATIONSHIP`、`T_CONNECTORVERTEX` 仍无接口）；#17 ❌→✅；#18 ❌→◐（注释与图元的关系；注释正文仍无接口）；#22 ❌→◐（代码表的短文字）；#27 ◐→✅（九个集合都有去处；读取器读不下去的转储和 SQL Server 包写明原因） |
| W10 | 「P&ID 备份面板：数据映射文件点开是枚举映射表」 | 参考数据里的 `SPPIDDataMap.xml` 和它旁边带日期的副本（全厂包 8 份，各 3.3–3.7 MB）此前只能当文本看。`io/pid_backup_browse/datamap.rs` 用钉版的 `read_datamap` 读出它的枚举部分：每个枚举条目（`SP_20_3`）发布出来对应的值（`@{…}`），连同条目所属枚举表的名字，按表名、条目序号排。`pid_backup_browser/datamap.rs` 是第五支 section：参考树里名字是 `SPPIDDataMap*.xml` 的文件多一个箭头，展开时在工作线程上读，下面多一行「枚举映射 (n)」，详情是枚举表数和一张表（枚举表、条目 UID、条目序号、发布值）；文件自己的文本预览不变。读不了的文件在那一行写明原因，没有枚举的写「这个文件里没有枚举映射」。三个包带的是同一套 8 份：现行那份 102 张枚举表 1 921 条，最旧的一份 97 张 1 809 条，每份读 15 毫秒上下。钉版只认枚举这一部分；文件里的类映射、属性映射（31 个 `SPMapClassDef`、1 818 个 `SPMapPropertyDef`）仍只有文本。 | #38 ◐→✅（枚举映射成表；类与属性映射仍是文本） |
| W12 | 「P&ID 备份面板：图纸体检、单张图纸的导入诊断与解析器报告」 | 第六支 section（`pid_backup_browser/diagnostics.rs` 与 `io/pid_backup_browse/diagnostics.rs`），树上不加行，两处入口都是详情里的按钮。**图纸体检**在备份根节点详情的末尾：「检查全部图纸」在工作线程上把每张图读一遍图元、导入一遍；进行中显示「正在检查：53 张里已完成 n 张…」，靠面板自己的一只 250 毫秒的钟刷新（`Effect::Tick` / `PidBackupMsg::Progress`，`Section::in_progress`）——应用平时不重画，没有这只钟数字就停着不动；做完是四行汇总和一张可排序的表（图纸、出了什么问题、图元数、画出的实体、未画出的源记录、用时、组路径）。**单张图纸**：有文件的图纸在「打开图纸」下面多一个「导入并诊断」，不打开图纸就按打开时的做法导入一遍，原地显示画出 / 解码 / 未画出的记录数、图层、图框模板及其图元、符号本体取自缓存还是符号库、找到的符号库、单位、样式表是否读失败、用时；详情末尾是 `pid_parse::inspect` 的解析报告全文（逐行的表，按文本预览同一套规则遮蔽口令样的值）。图元读不出的图纸也有这个按钮。`PIDBACKUPOPEN Press [n]` 按下选中节点详情里的第 n 个按钮，脚本和隐藏桌面靠它。顺带修了详情区的一处布局缺陷（先红后绿）：表格上方的行一多，表格就被挤出面板且上方不能滚动；现在上方超过 10 行时自带滚动条，最后一个小标题和它下面的几行留在表格正上方；6 行以内的短表只占自己的高度，长的数据表占余下的三分之二，文本的行表占五分之二。全厂包：53 张里 48 张完全读出，D03、D04 图元读不出，D03 导不进，149 310 个实体、9 条源记录未画出，35–62 秒；DWG-0202 包 7 张里 6 张完全读出；TEST02 包 12 张里 11 张图元读不出、2 张导不进——那 11 张都是它的 PlantData 里带的图框模板 `.pid`（`ZGSY P&ID Reference Data\Template Files`），SQL Server 包按文件夹列图纸，所以它们也在图纸之列。 | #28 ◐→✅ |
| W14 | 「P&ID 备份面板：还原缓存的位置可设、可清理，占用多少看得到」 | 还原缓存原先固定在 `%TEMP%\ocs-pid-backups`，占多少看不到，也从不清理（§1.3 第 9 条）。默认位置不变（D5 (a)）。新文件 `src/io/pid_backup/cache.rs`：缓存目录可以另指（`restore_cache_in` / `restore_in` / `build_index_in`），能量（`usage`：正在用的那份还原与其余各占多少）、能清（`clear`：除正在用的那一份外全部删掉）；只认还原自己放进去的东西——名字是 16 位十六进制键、里面有完成标记的目录，以及十分钟没人动过的半截暂存目录——缓存目录里别的文件和目录一概不动。备份根节点的详情在警告之后多出「还原缓存」一段：缓存位置（默认位置有标注）、本备份的缓存（大小与文件数；直接读文件夹的备份写明没有缓存；不在当前缓存位置的写出它在哪）、其他备份的缓存（几个、共多少），按钮「清理其他备份的缓存」「更改缓存位置…」「恢复默认位置」；清理之后原地写「已清理 n 个备份的缓存，释放 x」。量和清都在工作线程上：选中根节点时量一次，清理和改位置之后再量；每次请求带序号，只显示最新一次的结果（位置改了、备份换了之后才到的旧结果不显示）。位置是用户设置 `AppConfig.pid_backup.cache_dir`（默认位置不写进 settings.json），改动当即保存，下次启动照用。命令行：`PIDBACKUPOPEN Cache`（报告位置与占用）、`Cache=<文件夹完整路径>`、`Cache=Default`、`Cache=Clear`。`Press` 现在按详情里画出来的顺序数全部按钮，也可以写按钮的标签（`Press 检查全部图纸`）——根节点上多了缓存的按钮，序号会随「有没有可清理的」变，脚本用标签才稳；在命令行上按「更改缓存位置…」不弹对话框，改为提示 `Cache=<文件夹>` 的写法（脚本运行中、FILEDIA 0 时点这个按钮也一样）。三个包在默认位置：全厂包本备份 160.3 MB / 750 个文件，DWG-0202 包 68.5 MB / 686 个，TEST02 包 133.1 MB / 1 339 个。真实窗口里走了一遍：把位置指到另一个目录 → 还原两个包 → 详情显示「其他备份的缓存 1 个，共 68.5 MB」→ 清理（释放 68.5 MB）→ 关掉重开，设置还在、还原直接复用 → 恢复默认位置（settings.json 里那一项随之消失）。 | §1.3 第 9 条；清点表无变化 |
| W16 之一：滚动条 | 「P&ID 备份面板：树和详情的滚动条不再盖住行尾」 | 做 W14 时在真实窗口的图上看到的旧缺陷（先红后绿）：详情的行一多就出现滚动条，滚动条是叠在内容上画的，每行顶到右边的最后几个字被它盖住——长路径的末尾、SPID 的最后一位（`W14-real-cache-with-another-restore.png` 上能看到）；树的行多了也一样，选中行的底色伸到滚动条下面。现在两处的滚动条都在内容旁边占自己的一条（10 px 加 2 px 间隔），只在出现时才占，内容随之收窄、照常折行；内容放得下时不占位置，布局不变。表格（grid）自己的两条滚动条没有动：iced 的这个机制只支持单方向滚动。 | 无 |
| W16 | 「P&ID 备份面板：提示不重复，组路径一种写法，同组图纸按名字排，读不出的原因说清楚，三个图标重画」 | §1.3 第 7、8 条和三个图标，五件事，都先红后绿。**提示**：没有备份时「尚未打开备份。」上下两半各说一遍，载入中下半还在说「尚未打开备份」；现在只有上半说，下半是节点的详情，没有节点就不说话。**组路径**：组的详情写 `SQPlant / test / U01`，图纸的详情照搬图纸表的 `SQPlant \ test \ U01`；现在索引建图纸条目时就按树的写法写（`filed_under`），未登记、未分组两个组在「组路径」里也和树上一样翻译，体检表的组路径一列同样。**排序**：同组图纸原先按图纸表的行序（没有图纸表时按路径），现在按名字、照人读的顺序排（不分大小写，数字按数值，`labels::natural_cmp`）；全厂包 6 个组里 2 个的次序因此变了，另两个包各 4 个组里 1 个。**原因**：读不出图元的原因原先是「无法读取 <还原缓存里的整条路径>：no P&IDAttributes row carries a DrawingNo」，导不进的是导入器的英文句子，同样带整条路径。现在只写文件名（整条路径在图纸详情的「文件路径」里）；读取器自己的那一条理由译成「图纸里没有图号（P&IDAttributes 里没有带 DrawingNo 的行）」；导入器的失败是「<文件> 里没有可画的图形：解析器在里面没有找到图形」（有图形但没有一个是从记录里解码出来的，另有一句带个数）和「解析器读不了 <文件>：<原因>」。三个包里 14 张读不出的图纸、4 张导不进的图纸现在都是这几句。导入器的两句经 `io::pid::load_pid` 出去，用「打开」命令打开 `.pid` 失败时也是这个说法。**图标**：`pid_palette.svg`、`ui/collab.svg`、`ui/folder.svg` 原是 24 px、圆头线条的画法，不走其余图标的像素网格拟合；重画成 16 px 网格图（1 px 线、半像素坐标、只用 path、白色主线加 `#4a9eff` 强调色）：带拉链的文件夹（备份面板）、并排的两个人（协同面板）、带标题栏的面板里一个仪表圈和一个阀门（符号面板）。没有画 32 px 的版本（现有图标里 `ui/pick_point.svg` 也没有）。 | §1.3 第 7、8 条 |

### 6.5 第 1 项：每条测试的根因与修前修后

「定向重现」是把这条测试和会干扰它的测试放进同一个进程跑 30 遍（`target\stage1-logs\repro-item1.ps1`，干扰方是 41 条构造应用的上游测试；quad 那条是它和它旁边那条两条一起跑）。「全套」是库测试整套并行连跑 8 遍。修前的全套是 2a 尖端 `f4fe14604`，修后是本项的构建。

| 测试 | 根因（进程级状态） | 修法（都在测试里） | 定向重现 修前 → 修后 | 全套 8 遍 修前 → 修后 |
|---|---|---|---|---|
| `io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text` | 字形图集。排好字到导出读回之间，别的测试构造应用（`apply_settings` → `set_textfill` → 清空图集），文字从 PDF 里消失 | `sdf_atlas::undisturbed`：失败且图集代数动过才重跑，图集没动的失败照常报 | 29 → 0 | 4 → 0 |
| `ui::ribbon::qat::tests::a_command_items_look_is_worked_out_once_and_forgotten_with_the_tabs` | 语言代数。`apply_settings` 每次都 `set_language`，代数加一，快速访问栏的缓存按设计清空，计数就不对了 | 取数的那一段在语言代数没动的一轮里完成 | 24 → 0 | 0 → 0（主线 8 遍里 1 遍） |
| `scene::resident_patch_shorter_runs_tests::a_layer_switched_off_inside_a_block_patches_the_references_in_place` | 字形图集的代数。镶嵌备忘以它为键，一动就整套重建：顺序与「就地修补」的不同，修补计数也不加 | `sdf_atlas::undisturbed` | 9 → 0 | 1 → 0（第一阶段也出现过 1 次） |
| `app::plugin_host::tests::quad_feed_has_no_v14_auto_tab_per_plugin` | quad 面板存储是进程级的，哪些插件活着却是线程级的；旁边那条注册测试在 `PLUGIN` 名下放着一个面板的那一刻，这条测试的 `PLUGIN` 也看得见它 | 这条测试用自己的插件名 | 8 → 0 | 0 → 0（主线 1 遍，第一阶段 1 次） |
| `app::automation::tests::the_job_carries_one_glyph_snapshot_that_covers_its_pages` | 字形图集。产品代码自己会重排最多 4 次，旁边测试多的时候 4 次可能都被打断 | 取作业、出 SVG 两处各自重做，直到有一次图集没动 | **没重现**（0 → 0） | 0 → 0（主线 1 遍） |
| `app::update::plugin_grips_tests::third_settle_strike_tears_the_connection_down_and_arms_the_rearm` | 插件错误队列是进程级的，每个应用的 tick 都把它倒进自己的命令行。失败那次队列里少的正是第一击的几行，而这条测试中途自己不清队列 | `plugin/mod.rs`：测试构建里队列按线程各一份 | **没重现**（两种干扰组合各 0 / 30） | 1 → 0 |
| `app::update::viewport::rapid_drawing_tests::drawing_preview_keeps_the_new_object_colour_on_full_and_fast_passes` | 不是共享状态，是时间门限：一次指针移动处理 ≥ 8 ms（`MOVE_COALESCE_FROM_MS`），下一次移动就被合并掉不处理；机器忙的时候会超 | 每次移动前把门限打开（同文件别的测试也这样控制它） | **没重现**（单独 0 / 20；同时压三套全量时 0 / 20） | 修后那 8 遍里出现 1 遍（当时正在编译）；加这一行之后没有再量 |

同一机制、没见过失败、也没重现出来、一并套了同样保护的两条：`io::pdf_export::tests::text_grows_the_pdf_vs_no_text`（定向 0 → 0）、`scene::resident_patch_shorter_runs_tests::references_drawn_as_stubs_are_patched_when_a_layer_inside_switches`（定向 0 → 0）。

没有处理的：`rapid_drawing_tests::throttle_axis_command_matches_authoritative_move` 在同时压三套全量时 20 遍里失败 1 遍（单独 0 / 20），同样是时间门限一类，全套里没出现过。协同传输那一组照旧失败，等主线那边的修正。

### 6.6 交接记录（第二批）

写于 2026-11-11 01:10，W8 提交之后。给接手这个分支的人或会话看：做到哪了、怎么接着做、哪里会绊脚。之后每做完一项，在本节末尾的「进度」里补一两行，随那一项一起提交。

工作树 `D:\work\plant-code\cad\ocs-upstream-20261110`，分支 `upstream-merge-20261110`，只做本地提交、不改写已有提交（主线已经并入过这个分支）。`src/app/mod.rs`、`src/app/update/mod.rs`、`src/app/commands/mod.rs` 三个文件和 `src/app/collab/*` 不动。

**已完成（第二批）**

| 项 | 提交 |
|---|---|
| 0 真实窗口工具（隐藏桌面） | `c2a7db496` |
| 1 测试之间的竞争（§6.5） | `2271924a0` |
| 口令：文本预览 | `36eeb97fe` |
| 2 详情区：可拖的分隔条、选中行滚入视野 | `23eae5d00` |
| W8 还原时把 RefData 卷放回 `Ref\` | `4d389009b` |
| 交接记录（本节） | `d92c68ca9` |
| W6 图内数据：工艺点、分支点、图形表示、读取统计 | `8200f9b88` |
| W5 数据库转储：管段、端口、管嘴与注释、代码表，图纸在库里的那一面 | `d6edd4d27` |
| W10 数据映射文件点开是枚举映射表 | `1fb8423f3` |
| W12 图纸体检、单张图纸的导入诊断与解析器报告 | `76312cc76` |
| W14 还原缓存：位置可设、可清理，占用看得到 | `e46c35f62` |
| W16 之一：滚动条不再盖住行尾 | `d9931ad61` |
| W16 提示、组路径、排序、原因、三个图标 | `3d21e5666` |
| 测试不在临时目录里留下文件夹 | `20619d8e4` |

**还剩**：第二批的清单做完了（清点与收尾验证见 §6.7）。留给下一批的是 W9（符号与模板预览）、W7（图元与图形联动）、W13（搜索扩到图号、位号、文件名）；按决定不做的是 D3（换 pid-parse 钉版）、D4 / W11（SQL Server 包）、D7（DX12 视口）、W15（任意表，等 pid-parse 公开读法）。

**后来**（2026-11-21 加的）：W9 里符号的一半做了，见 §3.3 的 W9 下面加的那一段；W9 还剩模板（`.igr`、模板 `.pid`）和 `CatalogIndex.xml` 的目录。

**怎么编译**

- 工具链用 `rustup run stable cargo …`（1.99.1）；PATH 上那个 cargo 是改过的，不认 `CARGO_TARGET_DIR`。
- 环境变量：`CARGO_TARGET_DIR=<工作树>\target`，`TEMP` 与 `TMP` 都指到 `<工作树>\target\tmp`（还原缓存和测试的临时目录都写在临时目录下，不能落到 C 盘），`LC_ALL=C`。
- 平时改一轮跑一轮，只编库测试：`rustup run stable cargo test --workspace --lib --no-run --offline -j 4`。只有根 crate 变了的时候约 2 分钟（实测 80–135 s），产物是 `target\debug\deps\OpenCADStudio-<哈希>.exe`（约 268 MB），输出里 `Executable unittests src\lib.rs (…)` 那一行给出它的路径。
- 真实窗口和全量测试要全部目标（`target\debug\OpenCADStudio.exe`、集成测试、示例）：`rustup run stable cargo test --workspace --no-run --offline -j 4`，在库测试编过之后再编约 2 分钟。
- 不要用 `cargo build --workspace --lib`：那是另一套特性组合（不带开发依赖），iced 的各个 crate 会再编一遍，多出一份 832 MB 的 rlib 和约 2 GB 的增量目录。
- cargo 每次构建都会改写 `Cargo.lock`，提交前 `git restore Cargo.lock`。
- 每次提交之后的第一次构建会重编根 crate（`build.rs` 监视 HEAD 和分支引用），不是出了问题。

**怎么跑测试**

- 直接运行 `--no-run` 产出的可执行文件，不要再走 `cargo test`（每次都重编根 crate，见 §5.2）。工作目录是工作树根；环境变量 `RUST_MIN_STACK=67108864`、`ICED_TEST_BACKEND=tiny-skia`、`LC_ALL=C`、`CARGO_MANIFEST_DIR=<工作树>`（workspace 里其他 crate 的测试程序要指到各自的目录，工作目录也换过去），`TEMP` / `TMP` 同上。
- 备份面板这一块：`<库测试程序> pid_backup`（226 条，不到 1 秒）、`pidbackup`（29 条）、`i18n::tests`（18 条，2 秒；新加的本地化键漏了任何一处它会失败）。
- 走查三个真实包：环境变量见 §5.2；`OCS_PID_BACKUP_AUDIT_SKIP_IMPORT=1` 跳过逐张导入。带导入时全厂包约 65 秒（其中导入 40 秒上下），另外两个包各 15–20 秒。报告在输出目录的 `report.txt`，面板图片在旁边。
- 全量：`--no-run` 的输出里列出 18 个测试可执行文件，逐个运行。
- 本机的便利脚本在 `target\stage1-logs\`（不入库；丢了按上面的命令重写即可）：`b.ps1 [-All] -Jobs 4` 编译，`t.ps1 <过滤词…>` 跑库测试，`a.ps1 -Tag <名> [-Import] [-Only plant|dwg0202|test02]` 走查三个包（输出到 `target\ui-audit\stage2b\<名>\<包>\`），`run-tests.ps1 -Tag <名>` 全量，`fullruns.ps1 -Exe <库测试程序> -Tag <名> -Runs 8` 库测试连跑并统计每条失败几遍，`add_locale_keys.py <键.json>` 把新键加进 23 个 ftl 和两处键清单，`custom\plan_2b_row.py <行文件>` 往 §6.4 的表加一行。

**怎么在隐藏桌面上驱动真实窗口**

- 说明是 `docs/automation/guides/hidden-desktop.md`，脚本是 `docs/automation/hidden_steps.py`，步骤文件的例子是 `docs/automation/hidden_steps_example.json`。先编出 `target\debug\OpenCADStudio.exe`，然后：`python docs\automation\hidden_steps.py target\debug\OpenCADStudio.exe <输出目录> <步骤.json> --backend vulkan`。
- 步骤里发 `PIDBACKUPOPEN <包路径>`、`PIDBACKUPOPEN Select 参考数据 > rules.rul`（按标签路径展开并选中）、`PIDBACKUPOPEN Split 30`（树占 30%），再 `capture`（整窗）和 `crop`（只截面板）。起窗口、五六步、截图，一轮约 35 秒。
- 编辑器跑在另一个桌面上，不动指针、不抢前台，脚本自己核对运行前后的指针位置和前台窗口。代价是做不了指针操作：拖分隔条、悬停高亮、点面板里的行，这些靠 `iced_test` 按画出来的位置点击来验。
- 这台机器上 DX12 视口不出图（D7），所以带 `--backend vulkan`。
- 面板里还没有备份时，`PIDBACKUPOPEN` 后面带空格的内容会被命令行拆开逐词喂给提问：先发一条只带路径的 `PIDBACKUPOPEN <路径>`（路径里没有空格），之后的 `Select …`、`Split …` 才整行生效。要改得动 `src/app/command_driver/utilities.rs` 的整行命令清单，没有动。
- 执行过 `ZOOM E` 的图纸会被标成已修改，收尾时脚本报「did not close … was ended」，是预期的。

**踩过的坑**

- **偶发失败。** 库测试并行跑，共用进程级状态：字形图集（每条构造应用的测试都清空它）、语言代数、quad 面板存储、插件错误队列，另有按时钟合并指针移动的门限。本分支修过的七条见 §6.5。主线后来又各见过一次的两条不在其中，同属字形图集一族，按约定由主线处理、本分支不修：`app::automation::tests::a_job_whose_text_the_atlas_no_longer_holds_is_laid_out_again`（自己清空图集之后断言「现场图集里一定没有这些键」，而键是瓦片位置，别的测试紧跟着烘进去的字形会占回同样的位置）和 `io::svg_export::tests::a_page_style_table_wins_over_the_job_wide_one`（比较两页的输出，两页各自在写出时从现场图集取快照，中间图集被清空或重排两页就不一样）。本分支修前修后各 8 遍全套里这两条都没有失败过。
- **协同传输那一组**（`app::collab::transport`）在本分支上照旧失败 2–4 条，修正在主线上。
- **内存。** 同一台机器上常有别的仓库在编译。rustc 或链接器报 `memory allocation of … failed` 的那一遍不算通过也不算失败，等一等重跑。构建用 `-j 4`；大构建之前看一眼可提交内存（`Win32_OperatingSystem.FreeVirtualMemory`），低于 20 GB 先等。
- **磁盘。** 本工作树的 `target` 约 18 GB，其中增量缓存约 10 GB。D 盘低于 15 GB 就停。C 盘只有 8 GB 上下，`TEMP` / `TMP` 没指到 D 盘的话，每个包一次还原 70–160 MB 会落在 C 盘。`target\tmp\ocs-pid-backups\` 里 W8 之前的三个旧缓存（`ec348b5bfefd6425`、`c5fc2b87d4a3ade3`、`f9243fc51d4a5325`，约 198 MB）不会再被用到。
- **两个会话同时改这棵树**（2026-11-11 00:28–00:58 出过一次）。上级看到一轮「中断」时，断的可能只是它那边的事件流，那一轮自己还在跑（这次是被一条后台任务结束的通知重新叫起来的）；上级再启动新的一轮，就有两个会话同时改文件、共用一个 `target`。接手时先看有没有不是自己起的 `cargo`、`rustc`、测试进程（按命令行里的工作树路径找），`src` 和 `target\stage1-logs` 里有没有刚被写过的文件。有的话先不要动手：这次是在 `target\stage1-logs` 里放了一份说明，并让 `b.ps1`、`t.ps1`、`a.ps1` 打印它的路径，对方读到后把手上的 W8 收完提交、写下交接就停了。
- **用词与输出。** 上级猜测前两次中断与请求里的用词有关。代码、测试名、注释、提交说明用中性的词（「路径越出目标目录」「带盘符的路径」，测试用的文件名 `outside.txt`）；口令的原值、Manifest 的原文不要打到终端、日志、报告和截图里，要验证遮蔽就断言「原值不出现」或比较条数。
- **PowerShell。** here-string 和带 `\"` 的参数容易被截断或改写，补丁脚本和 JSON 先写成文件再执行；双引号字符串里变量后面紧跟冒号要写成 `${名字}`。
- **本地化。** 新标签都是 `pid-backup` 这条 Fluent 消息的属性，23 个 `locales/*/opencadstudio.ftl`、`src/locale_catalog.rs` 和 `src/i18n.rs` 的键清单要一起改，新键追加在各自的末尾。

**进度**

- W8（`4d389009b`）：三个包导入前后——带图框的图纸 0 → 45 张、0 → 4 张、1 → 1 张，图纸自身的图元数不变（149 310 / 1 697 / 920），未画出的源记录（9 / 0 / 49）和导不进的图纸（1 / 1 / 2）不变；修改前的数是拿 W8 之前的还原目录重量的（`target\ui-audit\stage2b\W8-before\`）。
- W6：图纸下面多出工艺点、分支点、图形表示、读取统计四类数据和一行「数据库里的那几类还没读」的说明（见 §6.4）。这一项没有碰导入。顺带量到的：三个包不带导入的走查各 33 / 24 / 44 秒；机器忙的时候库测试的编译从 75 秒到 450 秒不等。
- W5：数据库转储读进来了（数字见 §6.4；量的办法是 `<库测试程序> the_dumps_of_the_test_backups_are_read_and_measured --ignored --nocapture`，约 11 秒）。两件事接手时要知道：钉版的 `ExpTables::read` 对表全有或全无，DWG-0202 包的转储它读不全，面板按视图各读各的；单张图纸的数据库一面要把那张 `.pid` 再读一遍（面板自己读到的图元留在界面线程上）。磁盘：rustc 给根 crate 的增量缓存留两份会话（库测试 2.4 GB、库 2 GB 各两份），每次构建成功后删掉旧的那份能腾出约 4 GB（`target\stage1-logs\prune-incremental.ps1`）。
- W10：数据映射文件展开是枚举映射表。这一项里遇到一次「同一条命令被执行了两遍」：第二遍立刻报日志文件被占用而退出，第一遍的 cargo 还在后台编；看到 `The process cannot access the file` 时先查进程表里有没有自己这条命令的 cargo，等它编完再接着做，不要再起一遍。
- W12：图纸体检、单张图纸的导入诊断与解析器报告（见 §6.4）。这一项中途在 03:13 断过一次（五条线同一分钟一起停，不是拦截），09:17 续上；断的时候未提交的东西是完整的，只是隐藏桌面那一步被切在半截。续做时改了三处设计，接手的人不必再走一遍：（1）诊断不在树上加行——在每张图纸下加行会让「没有图元的图纸没有箭头」这类既有行为跟着变，在根下加行会让八条按整棵树断言的旧测试跟着变；现在两处入口都是详情里的按钮。（2）自动化通道（和 `drive_headless_task`）是把一条命令引出的任务逐个跑到底才返回的，所以真实窗口里截不到「进行中」的画面；自己不停续下去的任务（进度的钟）必须不靠别的消息就能停，否则这条通道会卡死——钟看的是体检线程自己置的「跑完了」，不是结果有没有送达。（3）详情区带表格时上方的行原先不能滚动，行一多表格就被挤出面板；判断方法是在面板图上看，`detail_rows` 的断言看不出来，要有按画面断言的测试。另：同一条命令被执行两遍的事又遇到一次，补丁脚本都按「原文只出现一次才替换」写，第二遍会在断言上停下，不会改坏文件。
- W14：还原缓存的位置可设、可清理（见 §6.4）。接手时要知道的三件事：（1）根节点详情里现在有不止一个按钮，`PIDBACKUPOPEN Press` 不带参数按下的是第一个（缓存的按钮），体检要写成 `PIDBACKUPOPEN Press 检查全部图纸`（按标签，英文界面是 `Press Check every drawing`）；W12 那份隐藏桌面步骤里的 `Press` 要照此改。（2）选中根节点现在会起一次后台测量（`Effect::MeasureCache`），断言「选中根节点什么也不读」的旧测试改成了「只起这一次测量」。（3）走查和测试都不清理默认缓存（三个包的还原在里面，清了只是下一遍多花一分钟，但别的会话可能正用着）；清理是拿临时目录当缓存位置验的，真实窗口那一遍用的是 `target\tmp\w14-cache`，验完已删。另外在真实窗口的图上看到一处旧缺陷：详情区出现滚动条时，滚动条盖住每行最右边的几个字（长路径、SPID 的末尾被遮住），W14 没有动它，留给下一项。
- W16 之一（滚动条）：上一行说的那处旧缺陷修了，单独一个提交。判断这类问题要看图：`detail_rows` 的断言里行的文字一个不少，只有画出来才看得到最后几个字在滚动条底下；红测量的是「有滚动条时一行的右端」与「没有滚动条时」之差。
- W16：文案与图标（见 §6.4）。接手时要知道的：（1）图纸的 `group_path` 现在是树的写法（` / `），不再是图纸表的原样；要原样的话读 `meta` 里的 `SP_PLANTGROUPID` 再查组表。（2）`GroupNode.drawings` 不再是索引的顺序，按图纸名排过；按下标遍历图纸的代码不受影响。（3）导入失败的两句话改在 `src/io/pid/mod.rs`（导入器），这是它头一次用 `tf!`；键仍挂在 `pid-backup` 这条消息下。（4）看图标用 `<库测试程序> dump_fitted_icons --ignored`（环境变量 `OCS_ICON_DUMP=<目录>`、`OCS_ICON_DUMP_NAMES=ui/folder.svg,…`），它写出各缩放比例下拟合后的 SVG；`target\stage1-logs\custom\icon_sheet.py` 用 PyMuPDF 把它们排成一张放大的对照图。（5）数屏幕上某句话出现几次：`iced_test` 的 `find` 只找第一个，给它一个从不选中的闭包选择器，它会把每个控件都递给闭包（`drawn_tests.rs` 的 `count_of`）。

### 6.7 第二批（2b）之后的清点与收尾验证

**清点表**

| | 2a 之后 | 2b 之后 | 变化的行 |
|---|---:|---:|---|
| ✅ 看得到 | 21 | 25 | 新增 #17（W5）、#27（W6 加 W5）、#28（W12）、#38（W10） |
| ◐ 看得到一部分 | 7 | 9 | #28、#38 升为 ✅；#14、#15、#18、#22 由 ❌ 升上来（W5）；#30、#31、#33、#34、#36 不变（#30、#31 在原样的包里也有了，W8） |
| ❌ 看不到 | 13 | 7 | 剩 #12、#13、#16、#19、#20、#21（钉版 pid-parse 没有读法，等 W15）和 #23（SQL Server 库，D4） |

**收尾时的验证**（最终的树，`20619d8e4`）

- 全量：18 个测试可执行文件跑了两遍（`3d21e5666` 上一遍，加上测试清理之后再一遍），两遍都是 **6496 过 / 1 失败 / 44 忽略**（2a 收尾是 6406 / 3 / 43）。失败的那一条两遍都是 `app::collab::transport::tests::the_session_emits_heartbeats_carrying_its_session_id`。多出来的一条忽略是 W5 量转储的那条测试（要真实备份包才跑）。
- 库测试另外连跑 8 遍：每遍 5927–5929 过、2–4 条失败、42 忽略，失败的全部在 `app::collab::transport::tests`：`quitting_sends_a_close_frame_and_joins_the_thread` 8 遍、`the_session_emits_heartbeats_carrying_its_session_id` 6 遍、`queued_claims_and_releases_reach_the_wire_with_the_session_id` 5 遍、`cancelling_the_subscription_ends_the_thread_promptly` 3 遍。连同上面两遍，库测试共 10 遍，这一组之外没有一条失败过；没有因内存不足而不计的遍次。2a 收尾时失败过的 `io::pdf_export::tests::filled_ttf_glyphs_export_as_real_text` 在第 1 项里修了，这 10 遍里都过。
- 走查三个包（带逐张导入）在 W16 的提交上都过：图纸自身的图元 149 310 / 1 697 / 920，未画出的源记录 9 / 0 / 49，导不进的图纸 1 / 1 / 2，读不出图元的图纸 2 / 1 / 11，与 W8 之后相同。口令检查三个包各 0 处命中（19 416 / 3 509 / 4 776 个节点的详情，加报告、命令行、索引、面板状态）。
- 备份面板这一块：`pid_backup` 226 过 / 3 忽略，`pidbackup` 29 过 / 1 忽略，`i18n::tests` 18 过。
- 非测试构建的告警 19 条、库测试构建 28 条，与第一阶段、2a 相同。

**真实窗口里没有动过手的**（要动指针）：拖动树与详情之间的分隔条、点树的行、点详情里的按钮——这三样有 `iced_test` 按画出来的位置拖和点的用例，真实窗口里是用 `PIDBACKUPOPEN Split`、`Select`、`Press` 代替的；悬停时分隔条的高亮——没有用例，也没有在真实窗口里看过；「更改缓存位置…」弹出的选文件夹对话框——只验到「脚本运行中不弹」和点击发出的消息，对话框本身没有打开过。另有一项真实窗口里截不到：体检进行中的进度行——自动化通道等一条命令引出的任务全部跑完才返回，这一段只有面板状态的测试。

**这一批留下的已知不足**

- 面板里还没有备份时，`PIDBACKUPOPEN` 后面带空格的内容被逐词交给提问（见 §6.6）；要改得动 `src/app/command_driver/utilities.rs` 的整行命令清单。
- TEST02 包的 PlantData 里带着图框模板的 `.pid`（`ZGSY P&ID Reference Data\Template Files`），SQL Server 包按文件夹列图纸，它们也被列成图纸：12 张里 11 张是模板，读不出图元。
- 钉版的读取器读不了 DWG-0202 包转储里的 `T_PIPERUN`（W5）：这个包的管段、代码表、管段连接、管线几个视图写明读不出的原因。
- 表格（grid）自己的两条滚动条仍叠在内容上：iced 给滚动条留位置的机制只支持单方向滚动的区域。
- 改了缓存位置之后，旧位置里的缓存留在原地，要切回去才能清；「清理」不删正在用的那一份（关掉备份后 `PIDBACKUPOPEN Cache=Clear` 全清）。选项页里没有缓存位置的入口（D5 定的是先只放在根节点详情里）。
- 三个重画的图标没有 32 px 的版本。
- 打开 `.pid` 失败时命令行那一行的前缀是上游的写法（`✕ 无效: 打开失败 : …`），没有动。
- 备份面板的测试只在通过时删掉自己建的临时文件夹，失败的那一遍会留下。

**仍待拍板**：D3、D4（许可）、D7 没有变化。新增三件可以顺手定的：上面第一条（整行命令）要不要动 `command_driver/utilities.rs`；TEST02 那一类包里的模板 `.pid` 要不要从图纸里剔出去（按目录名认，还是按「读不出图号」认）；缓存位置要不要在选项页里也放一个入口。
