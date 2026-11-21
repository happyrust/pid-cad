# 样本分析：SPPID 导出的 XML 规格与 `.pid` 结构

> 日期：2026-11-20（同日完成全量 110 对 / 220 份真实 SPPID XML 复核与 `pid-parse @ a85fa10` Q1–Q13 源码回填）
> 所属目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`（跟踪总单：[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)）
> 范围：只读 `$OCS_PID_CORPUS_DIR` 下的 SPPID 产出（94 对 `.pid` 加 XML、11 对厂商参考数据及备份包），并结合 `Cargo.toml` 钉定的 `pid-parse @ a85fa10` 源码完成核实。
> 用途：这些样本是外部平台"已知能通过"的东西，所以**它们就是我们导出时要对齐的规格**。本文把从样本得出的规格落成机器校验器（`assets/pid-publish-spec.json` + `src/io/pid_publish_check.rs`，[#111](https://github.com/happyrust/pid-cad/issues/111)），并在第 6 节回填 Q1–Q13 的源码核实结论（全文见同目录 `03-parse-answers.md`）。
> 原始统计输出保存在外部证据目录 `$OCS_PID_EVIDENCE_DIR`（文件名见第 7 节），语料文件通过 `scripts/sppid/corpus_index.py`（[#107](https://github.com/happyrust/pid-cad/issues/107)）生成 `corpus.manifest.json` SHA-256 绑定。
>
> **状态与解决记录（可回溯日志）**：
> - `2026-11-20`（提交 `538ec0112`，[#107](https://github.com/happyrust/pid-cad/issues/107)）：落地 `scripts/sppid/corpus_index.py` 与 `corpus-layout.md`，完成 94 对样本集 + DXF/PDF + 符号库共 `1,115` 个文件的只读 SHA-256 清单绑定（见同目录 `readings.md` §2）。
> - `2026-11-20`（提交 `6fd58799c`，[#111](https://github.com/happyrust/pid-cad/issues/111)）：基于全量 110 对（220 份）真实 SPPID XML 样本生成纯结构零客户数据的 `assets/pid-publish-spec.json`，落地 `src/io/pid_publish_check.rs` 与 `scripts/sppid/xml_spec_check.py`，全批样本零违规通过（见 `readings.md` §3）。
> - `2026-11-20`（`L0-8`，[#118](https://github.com/happyrust/pid-cad/issues/118)）：对照 `pid-parse @ a85fa10` 源码与测试逐条答完第 6 节 Q1–Q13，落盘同目录 `03-parse-answers.md`。

---

## 0. 结论（一页）

1. **XML 的规格可以直接从样本得出，而且很规整。** 94 份 `_Data.xml` 里一共 21 类对象加一种 `Rel`（关系），15 种关系类型，格式细节（编码、换行、缩进、容器属性）全部一致。
2. **`.pid` 里存着 XML 的 UID。** XML 里除了管线（`PIDPipeline`）以外的对象 UID，都以 ASCII 文本形式出现在 `.pid` 的 `/PSMcluster0` 和 `/Unclustered Dynamic Attributes` 里（抽样 3 个文件，最小文件上 98 个表示的 UID 全部找到）。**所以导出 XML 和生成 `.pid` 必须共用同一套 UID，不能各生成各的。**
3. **`.pid` 里有文字，没有码表值和拼出来的名称。** 位号的组成部分、设备序号、注释文字、页间连接符文字、保温规格都在 `.pid` 里（约 100%）；所有码表引用（`@EE3F8`、`@{GUID}`、`@NewConstruction`）、公称直径的写法（`4 in`）、仪表和管线的**组合名称**，都不在（0% 到 20%）。这些要靠"符号属于哪个类"加规则生成。
4. **"类到码表值"的映射规模很小。** 94 张图一共只有约 240 种"类变体"（接口集加码表值的组合）：管件和管线段各 78 种，其余类型都在 15 种以内。管件的前 20 种覆盖 88%，前 50 种覆盖 99%。**这个映射表是有限的、可以从样本里整理出来的。**
5. **备份包里有我们要的参考数据。** 728 个 `.sym` 符号、12 个空白模板 `.pid` 和 12 个图框模板 `.igr`、94 张图的 `.pid`，另一个备份包还带完整的数据库表 XML 导出。其中的 12 个空白模板 `.pid` 可以当"种子"的候选。
6. **第 6 节列出的 Q1–Q13 已对着 `pid-parse @ a85fa10` 源码全部核实完毕**（详见同目录 `03-parse-answers.md`）：XML 侧已有 `PublishDrawing` / `PidPublishDrawing` 纯内存写出器与 58 条类型分级目录可直接复用；`.pid` 侧 `PidWriter` 仅支持 CFB 流级替换与补丁，高层图形/对象记录链生成确需新写。

---

## 1. 资料盘点

| 位置（`$OCS_PID_CORPUS_DIR` 下） | 内容 | 数量与大小 |
|---|---|---|
| `sppid导出\sppid导出格式.pid+xml\01、02、03` | SPPID 产出的 `.pid`、`_Data.xml`、`_Meta.xml`，三个子文件夹 | 94 对：01 有 56、02 有 28、03 有 10。`.pid` 332 KB 到 6.5 MB（平均 1.8 MB）；`_Data.xml` 147 KB 到 3.7 MB（平均 1.0 MB）；`_Meta.xml` 约 1.7 到 2 KB |
| `sppid导出\导出dxf pdf\导出dxf`、`导出pdf` | SPPID 导出的 DXF 和 PDF | 各 94 个 |
| `sppid导出\核对分析文件\01_脱硫剂注入装置` | 以前做过的一份核对：原 DWG 对比 SPPID 导出，含核对报告、对象清单、文字清单、渲染图 | 6 个文件 |
| `sppid备份包\SQPlant_21_p.zip` | Oracle 导出形式的备份包 | 74.6 MB；含 `Export.dmp`（135 MB）、清单、配置、`PlantData`（94 张 `.pid`）、`RefData`（符号、模板、映射、规则） |
| `sppid备份包\Copyof_SQPlant.zip` | 另一种形式：带数据库表的文本导出 | 80.1 MB；`SPPID\T_*.xml` 约 60 张表（`T_PipeRun` 154 MB、`T_Representation` 71 MB、`T_Instrument`、`T_ModelItem`、`T_Relationship`、`T_Symbol` 等），加两份 `.ddt`（表结构定义） |

**关于已有的核对报告**：它是人工转绘（原 DWG 在 SPPID 里重画）之后的质量核对。里面提到：导出的 DXF 和 PDF 不含图签字段值、设备参数表、说明栏、图例，这些要在 `.pid` 里才能看到；也列出了 SPPID 内部"模型类型与图面符号不一致"的情况（例如图面是取样阀，XML 里的 `PipModelCode` 是"截止阀"）。这说明 **XML 里的类型是 SPPID 按符号所属的类写的，不一定等于图面看起来的东西**。

---

## 2. XML 样本规格

### 2.1 两个文件

每张图两个文件：`<图名>_Data.xml`（对象和关系）和 `<图名>_Meta.xml`（文档版本信封）。

### 2.2 格式细节（94 份全部一致）

| 项 | 值 |
|---|---|
| 编码 | UTF-8，**带 BOM**（文件开头 `EF BB BF`） |
| 换行 | CRLF，没有单独的 LF；文件以换行结尾 |
| XML 声明 | `<?xml version ="1.0" encoding="UTF-8"?>`，注意 `version` 后面有一个空格再接 `=` |
| 缩进 | `<Container` 0 空格；对象与 `Rel` 开闭标签 3 空格；自闭合接口元素 `<I... />` 6 空格；**`_Data.xml` 与 `_Meta.xml` 末尾的 `</Container>` 均为 2 空格**（110 对 / 220 份实测全部一致；注：`pid-parse` 内部写出器默认输出无 BOM、LF、2 空格缩进，故 `#111` 校验器拆分为默认 `StrictVendor` 与 `SchemaOnly` 两档模式） |
| 容器属性（`Data`） | `CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID=… DocName=… Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL="."`（`Plant` 在本批样本为 `"P01"`，跨厂导出可按参数指定，校验器默认要求非空或精确匹配指定值） |
| 容器属性（`Meta`） | 同上，但 `CompSchema="DocVersioningComponent"`，没有 `LoginUser`、`LoginPWD` |
| 第一个元素 | 全部样本都是 `PIDDrawing`，其 UID 等于容器的 `DocUID` |

除 `DocUID` 和 `DocName` 以外，容器的其他属性在同一工厂批次里取值完全相同。

### 2.3 `_Meta.xml` 的结构

固定按顺序排列 6 个子元素（3 个对象交替配 3 条关系）：`DocumentVersion`（`IDocumentVersion` 带 `DocRevision`、`DocVersion`、`DocVersionDate`）→ `Rel(VersionedDoc: DocUID -> DocumentVersion)` → `DocumentRevision`（`IDocumentRevision` 带 `MajorRev_ForRevise`、`MinorRev_ForRevise`）→ `Rel(RevisedDocument: DocumentRevision -> DocUID)` → `File`（`IFile` 带 `FilePath`）→ `Rel(FileComposition: File -> DocumentVersion)`。注意：`DocumentVersion`、`DocumentRevision` 及其两条关系的 UID 在样本中为 **30–32 位大写十六进制**（原生 VB6 `Hex$()` 格式化时偶省前导零），`File` 与 `FileComposition` 的 UID 以 `AAAD` 加十六进制连字符段组成。`DocVersionDate` 是导出日期，`FilePath` 是导出时的源目录——**这两个值在不同批次里会变，不是常量。**

### 2.4 `_Data.xml` 的对象类型

94 份共 21 类对象加 `Rel`：

| 类型 | 实例数 | 出现的文件数 |
|---|---:|---:|
| `PIDDrawing` | 94 | 94 |
| `PIDRepresentation` | 46,402 | 94 |
| `PIDPipeline` | 4,118 | 92 |
| `PIDPipingConnector` | 8,550 | 94 |
| `PIDPipingComponent` | 6,891 | 94 |
| `PIDPipingPort` | 18,553 | 94 |
| `PIDPipingBranchPoint` | 5,044 | 94 |
| `PIDProcessPoint` | 8,550 | 94 |
| `PIDSignalConnector` | 1,882 | 90 |
| `PIDSignalPort` | 40,681 | 93 |
| `PIDSignalBranchPoint` | 356 | 54 |
| `PIDBranchPoint`（页间连接符） | 885 | 89 |
| `PIDInstrument` | 2,974 | 93 |
| `PIDInlineInstrument` | 1,927 | 91 |
| `PIDControlSystemFunction` | 1,988 | 89 |
| `PIDNozzle` | 797 | 84 |
| `PIDProcessVessel` | 179 | 77 |
| `PIDMechanicalEquipment` | 269 | 46 |
| `PIDProcessEquipment` | 426 | 45 |
| `PIDHeatTransferEquipment` | 54 | 29 |
| `PIDNote` | 2,026 | 92 |
| `Rel`（关系） | 204,240 | 94 |

每个对象由若干"接口"元素组成，第一个固定是 `IObject`（带 `UID`，有的还带 `Name`、`Description`）。接口集和属性集按对象类型不同，完整清单见 `资料\分析输出\xml_02_各类型接口与属性.txt`。例如：简单类型（`PIDNote` 5 个接口、`PIDRepresentation` 2 个、端口 6 到 7 个），设备类 20 到 30 多个接口。

### 2.5 UID 规则

| 对象 | UID 写法 |
|---|---|
| 主要对象（设备、管件、仪表、管线、连接、注释、表示、图纸） | 32 位十六进制大写 |
| 管道端口、信号端口 | `<对象 UID>.<序号>`，序号 1 到 10 |
| 管道分支点、信号分支点 | `<对象 UID>.BPT` |
| 工艺点 | `<对象 UID>.PPT` |
| 关系 | `<前缀>-<UID1>-<UID2>`，前缀按关系类型固定：`DRI`、`DRC`、`EQC`、`E2E`、`ITN`、`PCN`、`PE1`、`PE2`、`PPC`、`PTF`、`PRP`、`SE1`、`SE2`、`SPC`、`STF` |

**关系的 UID 完全由两端 UID 和类型决定，不需要另外生成。** 端口和分支点的 UID 也由所属对象的 UID 派生。真正需要"创造"的只有主要对象的 UID。

### 2.6 关系类型（15 种，共 204,240 条）

| 关系 | 条数 | 方向（UID1 → UID2） |
|---|---:|---|
| `DwgRepresentationComposition` | 46,402 | 图纸 → 表示 |
| `DrawingItems` | 46,402 | 表示 → 图上对象（几乎所有类型） |
| `SignalPortComposition` | 40,681 | 信号端口 → 仪表、功能块、管口 |
| `PipingTapOrFitting` | 18,670 | 管道端口、分支点 → 管线段 |
| `PipingPortComposition` | 18,553 | 管道端口 → 管件、管口、设备、在线仪表 |
| `ProcessPointCollection` | 8,550 | 工艺点 → 管线段 |
| `PipingEnd2Conn` | 7,143 | 端口、分支点 → 管线段 |
| `PipingEnd1Conn` | 6,929 | 端口、分支点 → 管线段 |
| `PipingConnectors` | 5,395 | 管线段 → 管线 |
| `SignalEnd2Conn` | 1,827 | 信号端口、分支点 → 信号线 |
| `SignalEnd1Conn` | 1,626 | 信号端口、分支点 → 信号线 |
| `SignalTapOrFitting` | 853 | 信号端口、分支点 → 信号线 |
| `EquipmentComponentComposition` | 797 | 管口 → 设备 |
| `PBSItemNotes` | 390 | 注释 → 被注释对象 |
| `EquipmentOcc_EquipmentOcc` | 22 | 设备 → 设备（驱动关系） |

两端各是什么类型，完整对应见 `资料\分析输出\xml_04_UID规则与关系两端.txt`。

**结构上的几个观察：**

- 每个"表示"对应一个图形元素：表示总数（46,402）等于 `DwgRepresentationComposition` 条数，也等于 `DrawingItems` 条数。一个管线段平均有约 2.35 个表示（`DrawingItems` 里指向 `PIDPipingConnector` 的有 20,111 条，而管线段只有 8,550 个）。
- **文件内的顺序不要求"先定义后引用"**：204,240 条关系里有 53,256 条引用了文件里更靠后才定义的对象（26%）。
- 94 份里 `PIDDrawing` 都排第一。

### 2.7 属性取值的类型

| 种类 | 例子 | 说明 |
|---|---|---|
| **码表引用** | `@EE3F8`、`@EE790`、`@NewConstruction`、`@{78398AB4-9F3D-11D6-BDA7-00104BCC2B69}` | 指向参考数据里的码表条目。出现在建造状态、设备类型、管件类型、仪表类型、流向、管线连接类型、流体系统等属性上 |
| **文本** | 位号组成（`InstrTagSequenceNo`、`MeasuredVariable`、`EquipSeqNo`）、`NoteText`、`ToFromText`、保温规格 `InsulSpec`、管道等级 `PipingMaterialsClass` | 来自图上的文字 |
| **组合文本** | 公称直径 `4 in`、`25 mm`；压力 `12 MPa`；仪表和管线的 `Name`（如 `PG-0401302`、`50 mm-CI-0602002-B2-EH40`、`-PG-0201101-F9--F9--`） | 由规则从文字拼出或换算 |
| **常量** | `IsTypical=False`、`HTraceRqmt=""`、`ProcessPoint` 的各个压力温度属性全为空 | 每个对象都写，值固定 |

属性取值的分布（前几名和不同值个数）见 `资料\分析输出\xml_05_属性取值分布.txt`。

### 2.8 类变体：映射表的规模

把每个对象的"接口集加全部码表值"当作一个变体，统计 94 份里有多少种：

| 类型 | 实例数 | 变体数 | 前 5 / 10 / 20 / 50 种的覆盖率 |
|---|---:|---:|---|
| `PIDPipingComponent`（管件） | 6,891 | **78** | 55% / 73% / 88% / 99% |
| `PIDPipingConnector`（管线段） | 8,550 | **78** | 51% / 71% / 84% / 99% |
| `PIDInlineInstrument` | 1,927 | 15 | 85% / 99% / 100% / 100% |
| `PIDProcessVessel` | 179 | 14 | 89% / 98% / 100% / 100% |
| `PIDMechanicalEquipment` | 269 | 10 | 87% / 100% / 100% / 100% |
| `PIDHeatTransferEquipment` | 54 | 9 | 83% / 100% / 100% / 100% |
| `PIDPipeline` | 4,118 | 7 | 92% / 100% / 100% / 100% |
| `PIDProcessEquipment` | 426 | 5 | 100% |
| `PIDInstrument`、`PIDNozzle` | 2,974 / 797 | 4 / 4 | 100% |
| `PIDPipingPort` | 18,553 | 3 | 100% |
| 其余 9 类（注释、各种分支点、工艺点、信号端口、信号线、图纸、页间连接符、控制功能） | — | 1 到 2 | 100% |

**合计约 240 种变体。** 这是"图例 → SPPID 类"映射要覆盖的上限。注意管件和管线段的 78 种变体里，有一部分来自保温、管道等级等组合，不一定都对应不同的符号；真正的映射工作量要在做过"符号到变体"的对应之后才知道（需要 `pid-parse` 的连接方式，见第 6 节）。

---

## 3. `.pid` 结构

### 3.1 容器

- 94 个 `.pid` 全部能按复合文件（CFB）格式读出，没有失败。
- 根的 CLSID 全部是 `16ce6023-5f5b-11d1-9777-08003655f302`。
- 流的名称和结构在 94 个文件里稳定。主要的流（括号里是大小范围）：

| 流 | 说明 | 大小范围 |
|---|---|---|
| `/Sheet#` | 图形记录（最大的一类） | 数百字节到 790 KB |
| `/PSMcluster#` | 对象数据簇 | 25 KB 到 2 MB |
| `/Unclustered Dynamic Attributes` | 属性行表 | 0.7 KB 到 887 KB |
| `/StyleCluster` | 样式表 | 8 KB 到 24 KB |
| `/PSMspacemap/0x…` | 对象引用图 | 数十 KB |
| `/JSite#`（存储） | 每个符号一个，里面有 `JProperties`（符号路径）；符号的缓存本体是嵌套的存储（带自己的 `PSMcluster0`、`StyleCluster` 等） | 数量因图而异 |
| `/PSMroots`、`/PSMclustertable`、`/PSMsegmenttable` | 索引表 | 百字节级 |
| `/\x05SummaryInformation`、`/\x05DocumentSummaryInformation` | 摘要信息，94 个文件里前者固定 304 字节 | 304、420 到 456 |
| `/AppObject`、`/DocVersion#`、`/JSitesList`、`/JTaggedTxtStgList`、`/Dynamic Attributes Metadata` | 注册表与版本 | 几十到几百字节；`Dynamic Attributes Metadata` 固定 28 字节 |
| `/TaggedTxtData/Revision` | 大小为 0 | 0 |

**一个要注意的差别**：这 94 个文件里**没有** `/TaggedTxtData/Drawing`、`/TaggedTxtData/General` 这类流，只有大小为 0 的 `/TaggedTxtData/Revision`。图号、模板等信息在别的地方（见 3.2）。各流在 94 个文件里的出现情况见 `资料\分析输出\pid_01_94个文件的流结构.txt`，最小文件的完整目录见 `pid_02_最小文件的完整目录.txt`。

### 3.2 几个流里能直接看到的内容

| 流 | 能看到的 |
|---|---|
| `/Unclustered Dynamic Attributes` | 一张属性行表：行类名（`P&IDAttributes`、`Relationship`、`SymbolAttr` 等），属性名（`DrawingNo`、`DrawingItemType`、`DrawingID`、`ModelItemType`、`ModelID`、`SP_DrawingID`、`TypicalMode`、`PIDTemplateVersion` 等），以及 32 位十六进制的 UID；`SymbolAttr` 行里有符号的类名（例如 `Piping Connectors`、`Off-Drawing Piping Connector`、`Instrument off-line component`）和 `AABB Code` |
| `/\x05DocumentSummaryInformation` | `_PIDTemplate`、`_PlantName`（`SQPlant`）、`_SPID`（图纸 UID） |
| `/JSite#/JProperties` | 符号库路径，写成指向共享目录的 UNC 路径，形如 `\\<主机>\PID_SQProject\SQPlant\Ref\Symbols\Instrumentation\Off-Line\...\PG-Pressure Gauge.sym` |
| `/PSMcluster0` | 同样的属性行字符串（`P&IDAttributes`、`DrawingNo`、`ModelItemType`、`ModelID` 等）和 UID |

**`ModelItemType`** 是对象的类型名（我在样本里看到的有 `PipeRun`、`Connector` 等），这是 `.pid` 里和 XML 类型直接相关的字段。**`GraphicOID` 是小整数**（最小文件上样本值为 `1138`、`1092`、`661`），不是十六进制 UID，它指向 `.pid` 里图形记录的编号。

### 3.3 XML 的 UID 与值，在 `.pid` 里找得到吗

取最小文件做 UID 检查（98 个表示、其余对象若干）：

| XML 里的标识 | 在 `.pid` 里找到 | 位置 |
|---|---|---|
| 表示的 UID（`PIDRepresentation`） | **98 / 98** | `/PSMcluster0`（55 处）、`/Unclustered Dynamic Attributes`（43 处）；ASCII 文本 |
| 图纸 UID（`PIDDrawing`） | 1 / 1，ASCII 和 UTF-16 都有 | `/PSMcluster0`、`/Unclustered Dynamic Attributes`、`/\x05DocumentSummaryInformation` |
| 注释、控制功能、仪表、在线仪表、机械设备、管件、管线段、工艺设备、容器的 UID | **全部找到**（各类 2 到 22 个） | 同上 |
| 页间连接符（`PIDBranchPoint`）的 UID | 3 / 5 | 同上 |
| **管线（`PIDPipeline`）的 UID** | **0 / 9** | 找不到 |
| 二进制形式的 UID（直接 16 字节） | 0 | 都是 ASCII 文本 |

属性**值**的覆盖率，抽 3 个文件（最小、中位、最大）合并统计，有代表性的几项：

| 属性 | 找到的比例 |
|---|---|
| 位号组成：`InstrTagSequenceNo`、`MeasuredVariable`、`EquipSeqNo`、设备 `Name` | 约 100%（`PIDPipingComponent` 的 `Name` 77%） |
| 注释文字 `NoteText`、页间连接符文字 `ToFromText`、保温规格 `InsulSpec`、管线段的 `PipingConnectorSeqNo` | 94% 到 100% |
| 图纸 `Name`、`DocType` | 100%（`DocCategory` 0%） |
| 码表引用类：`ConstructionStatus`、`EqType*`、`PipingComponentType*`、`CommoditySpecialtyType`、`InstrumentType`、`FlowDirection`、`ProcFunc`、`IsTypical` | **0%** |
| 公称直径 `NominalDiameter`（`4 in` 这类写法） | **0%** |
| 组合名称：`PIDControlSystemFunction` 的 `Name` | **0%**；`PIDInlineInstrument` 的 `Name` 0%；`PIDPipingConnector` 的 `Name` 0%；`PIDInstrument` 的 `Name` 2%；`PIDPipeline` 的 `Name` 21% |
| `PipModelCode` | 9% |
| 管线的 `FluidSystem` | 0% |

逐项数据见 `资料\分析输出\pid_03_UID在pid里的出现.txt`、`pid_04_属性值在pid里的覆盖_3个文件.txt`。

**这说明什么：**

1. **`.pid` 自带的是"图上的文字和标识"**（位号组成、序号、注释、UID）。
2. **XML 里的码表值和组合名称，是 SPPID 在导出时按"类"和规则加上去的**，`.pid` 里没有存。我们生成 XML 时必须自己补这部分，来源是上一节的"类变体目录"和名称规则。
3. **管线对象在 `.pid` 里没有对应的 UID**，是导出时合成的（这是 `pid-parse` 文档里提过的做法，需要在另一台机器上确认具体算法，见第 6 节）。

---

## 4. 备份包里的参考数据

`SQPlant_21_p.zip`（Oracle 导出形式）里的内容：

| 条目 | 内容 |
|---|---|
| `Export.dmp`、`Export.log` | 数据库导出（135 MB） |
| `Manifest.txt` | 备份清单：备份类型 2、版本 7.02、工厂名 `SQPlant`；还有数据库连接信息（已被软件编码，**不要外传**） |
| `PlantConfig.xml` | 工厂配置，目前看到的是 ISO 15926 符号对应（样式到线型） |
| `PlantData~2~711.zip` | **94 张 `.pid`**（路径 `00/00/…`）加 1 个 XML |
| `PlantData~2~722` | 1 个 XML（UTF-8 带 BOM） |
| `RefData~4~681.zip` | **728 个 `.sym`**：`Instrumentation` 290、`Piping` 229、`Equipment` 151、`Design` 35、`Equipment Components` 23；加 `CatalogIndex.xml` |
| `RefData~4~682.zip` | **12 个空白模板 `.pid`（28 KB 到 118 KB）和 12 个图框模板 `.igr`**：`A0-W`、`A1-W`、`A2-W`、`A3-W`、`CPE-SQ-A0`、`CPE-SQ-A0+`、`CPE-SQ-A1`、`CPE-SQ-A2`、`CPECCHB-SQ-A0/A1/A2`、`CPECCHBA2` |
| `RefData~4~804.zip` | 8 份 `SPPIDDataMap*.xml`（不同日期的版本）、`CatalogIndex.mdb` 和 `.ldb` |
| `RefData~4~809.zip` | 4 份 XML：两份"工厂组对应"小文件，两份**导入映射**（`SPImportMap`：条目类型映射、选择列表映射、符号映射，源和目标工厂都是 `PIPECHINAYQSTD_PID` 之类） |
| `RefData~4~684.zip` | 10 个 Excel 宏文件（设备清单、仪表阀门清单、`OPC Report` 等报表） |
| `RefData~4~680`、`~683`、`~703`、`~709` | 规则文本（以 `Begin Rules` 开头）、一个复合文件、一个压缩包、一份项目保温 XML |

**需要说清楚的一点**：`RefData~4~809.zip` 里的映射文件是 SPPID 工厂之间的导入映射，**不是外部交付平台的规格**。外部平台的规格仍然没有。

`Copyof_SQPlant.zip` 里的 `SPPID\T_*.xml` 是数据库表的文本导出，里面有 `.pid` 里没有的"数据库独有"信息（管线 `T_PipeRun`、管道点 `T_PipingPoint`、连接 `T_Connector`、关系 `T_Relationship` 等），可以用来核对 XML 里管线、端口、连通关系是怎么从表里来的。**我这次没有读这些表的内容。**

---

## 5. 对路线的影响

| 影响 | 对应的路线项 |
|---|---|
| XML 规格从样本直接得出，格式细节（BOM、CRLF、声明里的空格）要原样复刻，成本很低 | L4 |
| 码表值要靠"类变体目录"（约 240 种），目录可以从 94 对样本里整理 | L3-1、L4-2 |
| 仪表、管线等的组合名称要实现规则；需要和样本逐项对照 | L4-5 |
| 管线对象在 `.pid` 里没有，要合成：UID、名称、与管线段的关系 | L4-4 |
| UID 必须同时写进 `.pid` 的 `/PSMcluster0` 和 `/Unclustered Dynamic Attributes` | L5、L4-3 |
| 12 个空白模板 `.pid` 是现成的"种子"候选；资料清单第 14 项里"空白种子"这一半已经有了，"放了一个设备的图"还没有 | L5-3 |
| 94 对样本加备份包，构成真值；可以直接做 L0-3 的对应关系表（图名一一对应） | L0 |
| 外部平台的规格仍缺，目前只有"这些样本能通过" | L0-7 |

---

## 6. 对 `pid-parse @ a85fa10` 源码回答的 13 个问题（`L0-8` 已完成）

> **解决记录（2026-11-20，`L0-8`）**：已在 `$OCS_PID_PARSE_SRC` 对照 `Cargo.toml` 钉定的 `pid-parse @ a85fa10285cc5379953495fa391f374981b9e587` 源码、测试与原生 DLL 反编译分析文档逐条核实完毕。下表给出每题结论摘要，带精确源码行号与影响分析的完整报告见同目录 **`03-parse-answers.md`**。

| # | 问题 | 源码核实结论（详见 `03-parse-answers.md`） |
|---|---|---|
| Q1 | `pid-parse` 有没有从"文档模型"构造 `.pid` 的写出能力？现有写出器能写哪些流？ | **`src/writer/mod.rs:4-75`**：仅支持基于 `PidPackage` 的 CFB 容器重打包（保根/子存储 CLSID，字典序重写目录并刷新时间戳）、`/TaggedTxtData/*` 替换、摘要属性编辑、整流替换（`stream_replacements`）与字节补丁（`sheet_patches`），**无高层新增对象 API**；`L5-4` 在整流替换之上新写记录链与索引编码器 |
| Q2 | 第 3.1 节各流的解码状态与覆盖台账？ | **`src/inspect/coverage.rs:10-420`**（`pid_inspect --coverage --json`）：摘要、`PSMroots`、`DocVersion2/3`、`AppObject`、`JTaggedTxtStgList`、`JSitesList` 为 `FullyDecoded`；`Sheet*`、`JSite*`、`PSMspacemap/*`、`PSMcluster0`、`Unclustered Dynamic Attributes`、`StyleCluster` 记录链 100% 走通至零剩余且核心族已解码 |
| Q3 | `/Unclustered Dynamic Attributes` 行格式与两跳连接？ | **`src/parsers/attribute_rows.rs:1-103` + `src/publish/pid_load.rs:9-57`**：`0x0089` 完全解码（`+0` 行 oid、`+12` 被描述图元 oid、`+20` 类名与 `0x8010`/`0x0003`/`0x000B` 字段）；标注经 `0x004D`（`+4`）→ `0x00FA`（`+16` 图元、`+22` 属性行）两跳直达，53 图 `9,218 / 9,225` 命中 |
| Q4 | 管线（`PIDPipeline`）如何合成 UID、名称与关系？ | **`src/publish/pipeline.rs:3-189`**：按非空 `TagSequenceNo` 管线段聚合；优先取含 `'-'` 的 `ItemTag`（7 参考图 `629 / 629` 命中），否则按五槽位拼接；UID 为 `DNS_NAMESPACE` → `"pid-parse/pipeline"` → `plant` → `name` 三级确定性 `UUID v5` |
| Q5 | 仪表、设备、管线段、管线组合名称规则与例外？ | **`src/publish/pid_load.rs:21-25`、`pipeline.rs:111-138`**：仪表 `{MeasuredVariable}-{InstrTagSequenceNo}`（`236 / 236`），设备 `{EquipPrefix}-{EquipSeqNo}`（`13 / 13`）；改造图偶有库内 `ItemTag` 留 `old_` 旧号而图面分段写新号的已知差异 |
| Q6 | 公称直径与带单位数值换算规则？ | **`src/publish/pid_load.rs:27-34`、`pipeline.rs:153-179`、`unit_values.rs:14-149`**：英寸号转 `N in`、公制转 `N mm`；整数码 `<5000` 为 `mm`、`>=5000` 按 `(code-5000)/32` 约分；SI 浮点按 `oleaut32` 15 位有效数字 `automation_text` 格式化 |
| Q7 | 码表值与"类目录"位置、规模、覆盖率？ | **`src/publish/catalog.rs` + `codelist_values.rs:85-165`**：内置 21 类接口规则表与 107 份厂商 XML 核对的 58 条零冲突类型分级链（`LEAF_LEVELS`），53 张全厂包表外未命中 `0`、94 套单图验证集 `82 / 94`（87.2%）整图一致 |
| Q8 | XML 写出函数输入结构与 `L4-1` 选型？ | **`src/publish/model.rs`、`xml_writer.rs`、`pid_writer.rs`**：纯内存 `PublishDrawing` / `PidPublishDrawing` 可脱离文件与库直接调用；**`L4-1` 锁定方案 A（模型转 DTO + 外层加 BOM/CRLF/缩进规整以过 `#111` 严格门禁）** |
| Q9 | `GraphicOID` 分配、跨存储唯一性与新增对象同步流清单？ | **`docs/pid-format-guide.md:97-340`**：持久 ID `(segment << 13) | index` 按存储独立（顶层与各 `JSiteN` 隔离）；新增对象需同步记录链及 `record_count`、`0x0081 JSheetLayer` `+12` 对象计数、`/PSMspacemap` 反向入边槽（及新增嵌套站点时的 `/JSitesList`） |
| Q10 | `DocumentSummaryInformation` 中 `_PIDTemplate`、`_PlantName`、`_SPID`？ | **`src/streams/summary.rs:17-58`**：位于第 2 节用户字典，`_SPID` 即图纸 32 位 `DocUID`；新建图纸时需与 XML `DocUID` 同步写入，SPPID 打开期强校验程度由 `LX-4` E2 实测 |
| Q11 | 为何 94 个文件无 `/TaggedTxtData/Drawing`、`/General`？ | **`src/streams/tagged_text.rs:16-36`**：二者仅见于旧版/特定配置；常规生产 `.pid` 只留空 `/Revision`，图号存于 `0x0089` `P&IDAttributes.DrawingNo`，写出时严禁依赖 `TaggedTxtData` 填图号 |
| Q12 | `/JSite#/JProperties` 符号 UNC 路径要求？ | **`src/streams/jsite.rs:24-98`**：每个 `JSite#` 自带完整内嵌符号缓存本体，离线解析与渲染不访问 UNC；生成新 `.pid` 时须同时写入内嵌缓存本体与规范化相对符号路径 |
| Q13 | 现有"受控样本"证据与差分工具？ | **`docs/analysis/2026-05-09-controlled-pid-diff-protocol.md`**：已有完整 7 类原子操作采集协议与自动化命令 `pid_inspect --controlled-diff-dir <dir> --json`，真实受控 `.pid` 样本对在 `LX-3` 采集后直接接入 |

---

## 7. 本文用到的原始输出（`资料\分析输出\`）

| 文件 | 内容 |
|---|---|
| `xml_01_类型与关系统计.txt` | 94 份 `_Data.xml` 的容器属性、对象类型实例数、关系类型条数 |
| `xml_02_各类型接口与属性.txt` | 每类对象的接口和属性清单（含出现次数） |
| `xml_03_Meta统计.txt` | `_Meta.xml` 的统计 |
| `xml_04_UID规则与关系两端.txt` | UID 写法、关系 UID 前缀、关系两端的类型 |
| `xml_05_属性取值分布.txt` | 每个属性的不同值个数和最常见取值 |
| `pid_01_94个文件的流结构.txt` | 94 个 `.pid` 的流清单和大小范围 |
| `pid_02_最小文件的完整目录.txt` | 最小 `.pid` 的完整目录 |
| `pid_03_UID在pid里的出现.txt` | UID 在 `.pid` 里的出现情况 |
| `pid_04_属性值在pid里的覆盖_3个文件.txt` | 属性值在 `.pid` 里的覆盖（3 个文件） |
| `pid_05_部分流里的字符串样例.txt` | 部分流里的字符串（其中带 `U:` 的行是我按错误编码解读出的乱码，**请忽略**） |

---

## 8. 没有做、没有核实的地方

- **没有读** `Copyof_SQPlant.zip` 里的 `T_*.xml` 数据库表，也没有读 `Export.dmp`（Oracle 导出，本机没有读取它的工具）。
- **没有把 `.pid` 里的对象和 XML 里的对象逐个对上**（那需要 `pid-parse` 的连接方式，见 Q3）。所以"符号到类变体"的对应关系我没有建，只统计了变体的规模。
- `.pid` 的值覆盖率只抽了 3 个文件，不是 94 个。
- 94 个 `.pid` 与备份包里 `PlantData` 的 94 张图，是否是同一批文件，我没有逐个比对。
- `资料\sppid导出\导出dxf pdf` 里的 DXF 和 PDF 没有分析。
- `.pid` 里图形记录（`/Sheet#`）的内部结构没有看；那是 `pid-parse` 的工作。
- 没有验证"这 94 对样本都已通过外部平台"，这是你的说明。
