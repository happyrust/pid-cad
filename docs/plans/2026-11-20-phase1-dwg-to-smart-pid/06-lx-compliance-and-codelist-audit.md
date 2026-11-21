# `LX-6 / LX-7 / #117`（[#137](https://github.com/happyrust/pid-cad/issues/137)）：备份包分级表来源实查、`.pid` 保留流够用性台账与合规边界确认

> 日期：`2026-11-21`
> 所属目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`（跟踪总单：[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)，关联单：[happyrust/pid-cad#117](https://github.com/happyrust/pid-cad/issues/117)）
> 写法约定：全文仅使用仓库相对路径与语料逻辑名（`BACKUP_ORACLE`、`EXPORT_SET` 等），不含任何盘符、用户名、主机名、数据库连接串或客户图名。

---

## 1. `#117` 第 2 步调查结论：备份包内能否推出多级类型分级链？

### 1.1 实查范围清单（`BACKUP_ORACLE` 全厂备份包全部 16 个顶层文件）

对全厂备份包根目录及全部子归档执行了 ASCII 与 UTF-16LE 双编码全字节扫描及 XML 关系图遍历，核查范围如下：

| 类别 | 核查对象 | 规模 | 内部结构与核查结果 |
|---|---|---:|---|
| 数据库导出 | `Export.dmp` + `Export.log` | `74,489,856 B` | Oracle `exp` 导出，共 **`126` 张表**；业务表仅存末级或三列工具枚举整数（如 `T_PIPINGCOMP` 的 `PIPINGCOMPCLASS / PIPINGCOMPSUBCLASS / PIPINGCOMPTYPE`、`T_EQUIPMENT` 的 `CLASS / EQUIPMENTSUBCLASS / EQUIPMENTTYPE`），不含目标发布 Schema 的父子层级表 |
| 工厂配置 | `PlantConfig.xml` | `5,085 B` | 仅定义图面样式、颜色与线宽映射，**`0` 处**枚举或码表分级定义 |
| 符号目录 | `RefData~4~681.zip` | `742` 项 | `644` 个 `.sym` + `CatalogIndex.xml`（仅含 `(ItemType, Class, Subclass, Type) -> .sym` 路径索引，不含 `@EE*` / `@{GUID}` 发布码表） |
| 图框与种子 | `RefData~4~682.zip` | `20` 项 | `10` 对空白 `.pid` 与 `.igr` 图框模板，不含码表分级 |
| 规则与报表 | `RefData~4~680`、`683`、`684.zip`、`685.zip`、`703`、`709` | 共 `17` 项 | 分别为规则文本（`Begin Rules`）、CFB 报表模板、`10` 份 `.xlsm` 工程表模板、OOXML 工作簿与保温规格 XML（`<ProjectInsulationSpecifications>`），**`0` 处**发布枚举层级 |
| 导入映射 | `RefData~4~809.zip` | `4` 份 XML | `XOMContainer Domain="SPImportMap"` 与 `PlantGroupMap`，系外部导入映射，不含发布侧 `LEAF_LEVELS` 父链 |
| 发布数据映射 | `RefData~4~804.zip` | `10` 项 | 含 `CatalogIndex.mdb` 与 **`SPPIDDataMap.xml`**（`17,407` 个 `IObject`、`12,087` 条 `<Rel>`，见 1.2 节逐项拆解） |

---

### 1.2 五类内置内容逐项核查与可推导性定论

`pid-parse @ a85fa10` 的 `src/publish/codelist_values.rs` 中共有五处源自参考 XML 归纳的内容，实查定论如下表：

| 编号 | 内置内容 | 包内是否存在？ | 实查技术证据 | 能否完全从备份包推出？ |
|---|---|---|---|---|
| 1 | **`LEAF_LEVELS`（26 条末级到根分级链，共 58 个层级值）** | **部分节点在 `SPPIDDataMap.xml`，完整链不在包内** | ① `SPPIDDataMap.xml` 含 `1,573` 条 `DefUID="MapEnumMapEnum"` 关系，但它是**工具侧 3 层树**（`SP_27->SP_26->SP_25` 共 `17+272` 条、`SP_24->SP_23->SP_22` 共 `58+604` 条、`SP_29->SP_28` 共 `265` 条），而非发布目标 **`EFSchema.xml` 的 `EnumEnum` 多级树**；<br>② 实测反例 A（跳层）：工具链 `SP_27_6 -> SP_26_1 -> SP_25_70` 经 `MapEnumToEnum` 只给出第 1 层 `@{81CD9284-...}` 与第 3 层 `@{81CD934A-...}`，**中间第 2 层 `@{81CD92C0-...}` 挂在旁支 `SP_25_54` 上，工具树本身不包含 `81CD9284 -> 81CD92C0 -> 81CD934A` 三级父子边**；<br>③ 实测反例 B（层数不足）：设备 `IEquipment` 发布层级深达 **6 层**（`EqType0..EqType5`：`47BF0267 -> EE791 -> EE7A1 -> EE7B9 -> EE7C2 -> EE7CA`），而工具侧仅有 `CLASS / SUBCLASS / TYPE` **3 列**，数学上无法编码 6 层目标树；<br>④ 实测反例 C（根枚举缺失）：仪表第 1 层 `@EE3F8.1` 在整个备份包全部文件（含 `Export.dmp` 与 `SPPIDDataMap.xml`）中出现次数为 **`0`** | **不能从备份包单独推出**（完整树定义在目标机器的 SmartPlant Schema 文件 `EFSchema.xml` 中，不在备份包内） |
| 2 | **`SPECIALTY_ITEM`（`1 -> @{5F7F8F6E-...}`，`2 -> @{5F7F8F68-...}`）** | **不在包内（`0` 字节命中）** | `SPPIDDataMap.xml` 将 `CommoditySpecialtyType` 绑定到转换器 `EFAdapter.BoolToEnumConv`，两个目标 GUID 硬编码在厂商适配器 DLL / `EFSchema.xml` 内，备份包全量扫描命中 **`0` 次** | **不能从备份包推出**（属 SmartPlant Schema 固定布尔枚举 GUID） |
| 3 | **`DRAWING_CONSTANTS`（`DocCategory = "P&ID Documents"`、`DocType = "P&ID"`）** | **不在包内（`0` 字节命中）** | `Export.dmp` 自身代码表存的是 `"Piping Documents"` 与 `"P&IDs"`，`SPPIDDataMap.xml` 未映射这两个属性；发布值由 SPPID 发布器固定写入，且 110 对真实样本 `100%` 恒为此二值 | **不能从备份包推出**（属发布器固定常量） |
| 4 | **`PLANT_ADDED_INSTRUMENT_TYPE = 10_000`** | 数据列在 `Export.dmp`，阈值规则不在包内 | `SP_28` 中 `< 10000` 为厂商标准条目、`>= 10000` 为建厂自定义扩展条目；该分段约定是 SPPID 参考数据建库惯例，未在 XML 中显式声明阈值 | **不能从包内元数据自动推出**（需保留为可配置阈值并统计命中次数） |

### 1.3 对 `#117` 与阶段一 `L4-2` 的工程处置落地

1. **调查定论**：备份包本身不包含 `EFSchema.xml` 的目标枚举父子图，因此**不能靠解析备份包替代分级表**；正确的工程路径是 `#117` 提出的「**第 1 步（让表外兜底/少写 100% 可观测，为 0 也输出）+ 第 3 步（数据外置成独立 JSON，支持按厂替换）**」。
2. **本仓阶段一导出侧（`src/io/pid_publish/` + `src/io/pid_convert/`）已在本单直接落地这两项机制**：
   - **可观测性（`VariantResolutionSummary`）**：`resolve_variant_tracked` 与 `_Coverage.json` 对每张转换图纸显式输出四项计数（`exact_variant_id`、`symbol_path_match`、`class_default_fallback`、`unmapped_class_variant`）及按类兜底明细 `fallback_by_class`（即使全为 `0` 也恒定写出，绝不静默省掉）；
   - **按厂热替换（`--class-map <path>`）**：外置文件 `assets/pid-class-map.json` 已独立存放全部 237 种变体与分级码表元组，`ConvertOptions::class_map_path` 与 `examples/pid_convert.rs --class-map <file.json>` 支持在不重编译程序的情况下加载其他工厂或从 `EFSchema.xml` 导出的类变体表。
3. **备份包导出侧（`pid-parse` 仓）衔接建议**：待后续需要升级 `Cargo.toml` 钉定的 `pid-parse` 版本时，可在 `pid-parse` 的 `BackupPublishReport` 中按本报告第 1.2 节口径增加 `unmapped_leaf_levels`、`unmapped_specialty_codes`、`plant_added_instruments_skipped` 三个计数字段。

---

## 2. `LX-6`：`.pid` 写出器未逐字段重编码流的够用性核查台账

对照 `#128`（`src/io/pid_write/surface.rs`）建立的 CFB 18 类顶层流分类表，写出器对 `8` 类核心业务与索引流执行结构化增补/同步，对剩余 **`10` 类容器与环境流采用从空白模板种子（`SEED`）原样字节继承（`passthrough_preserve`）**。逐项够用性评估如下：

| 流 / 存储路径 | 当前处置策略 | 在 10 份空白模板与 94 份真值 `.pid` 中的特征 | 对阶段一 `a/b` 级与 `c1` 级的够用性结论 |
|---|---|---|---|
| `/PSMroots` | 原样继承种子 | 固定根簇入口表，指向 `/PSMcluster0`、`/StyleCluster` 等根对象 | **够用**：增补图元均挂入既有 `/Sheet6` 与 `/PSMcluster0` 链内，根入口表不变 |
| `/PSMclustertable` | 原样继承种子 | 簇目录描述表；新增记录追加在既有簇流内，未新建 `/PSMclusterN` 流 | **够用**：簇集合未增删时表项恒定不变 |
| `/PSMsegmenttable` | 原样继承种子 | 空间段目录表；`#129` 已在既有 `/PSMspacemap/0x00000000` 段内更新反向引用槽 | **够用**：单段容量足以容纳整张流程图全部图元与符号引用 |
| `/StyleCluster` | 原样继承种子 | 存放线型、颜色、字体等样式记录；写出器通过 `pick_seed_style_ids` 直接引用种子既有样式 OID | **够用**：复用同项目标准空白模板时样式集与目标项目完全一致 |
| `/DocVersion#` | 原样继承种子 | OLE 文档版本头，同版本 SPPID（`10.00.31.0023`）空白模板与交付图一致 | **够用**：无需修改 |
| `/JTaggedTxtStgList` + `/TaggedTxtData/Revision` | 原样继承种子 | 94 份生产样本中 `/TaggedTxtData/Revision` 全为 `0` 字节空流（见 `03-parse-answers.md` Q11） | **够用**：与真实生产样本完全一致 |
| `/AppObject` | 原样继承种子 | 应用级全局配置流，同项目模板与生产图结构相同 | **够用**（若后续 `LX-4` `E3` 实验发现项目级时间戳/登记号校验，仅需针对该单流做受控差分） |
| `/Dynamic Attributes Metadata` | 原样继承种子 | 定义 `P&IDAttributes`（`0x0089`）列结构 schema；同厂空白模板已内置相同列定义 | **够用**：`#129/#130` 写入的 `0x0089` 属性行在 `PidParser` 重读中 `100%` 解码对齐 |
| 根存储 `CLSID` 与流顺序 | `PidWriter` 保持根 `CLSID` 与流字节无损 | `#128` 实测 10 份空白模板流字节与存储 `CLSID` `100%` 零差异往返 | **够用**：无需全面逆向未变流 |

**`LX-6` 结论**：阶段一写出所需的全部流与记录字段均已具备（`a/b` 级已在 90 张真实转换图与交互编辑存回图上 `100%` 验证通过），当前不存在阻塞阶段一的未知流；仅在后续 `RES-06` 真机执行 `LX-4`（`E1..E3` 接受度实验）出现具体报错码时，才需对照受控样本做单点字段核查。

---

## 3. `LX-7`：合规、版权与客户数据安全边界书面确认

阶段一全流程严格执行以下四条合规与保密红线（已通过自动化检索核验）：

1. **不分发厂商二进制与专有资源**：仓库内绝不提交或分发任何 SmartPlant P&ID 可执行程序、DLL、`.sym` 符号文件、`.igr` 图框模板、`EFSchema.xml` 或 `SPPIDDataMap.xml` 原文；所有规格文件（`assets/pid-publish-spec.json`、`assets/pid-class-map.json`、`assets/pid-sppid-legend-map.json`、`assets/pid-xml-diff-allowlist.json`）均为纯 ASCII 结构摘要，不含厂商受版权保护的图形字节或客户业务数据。
2. **客户图纸、语料与连接凭据零入库**：
   - `Manifest.txt` 中编码的数据库连接信息、客户工程名称、位号实例与图名关键字严禁进入代码、测试夹具、文档、提交说明、日志或线上 Issue；
   - 仓库内所有单元与集成测试均使用内存中合成的最小几何/XML/`.pid` 夹具；依赖真实语料的脚本与 `#[ignore]` 测试一律通过环境变量（`OCS_PID_CORPUS_DIR` 等）在外部临时/证据目录只读运行，证据报告仅使用脱敏编号（`S01..S94`）与 SHA-256 前缀。
3. **本机环境信息脱敏**：仓库文件与对外沟通记录不写入本机盘符路径、操作系统用户名或主机名。
4. **受控验收环境隔离（`RES-06` 执行准则）**：后续接入受控 SPPID 环境运行 `precheck_and_receipt.py precheck` 时，必须先建立基线虚拟机快照、使用低权限专用访客账号（凭据不落盘不入库）、关闭虚拟机对宿主机的写共享，仅通过 `--in` / `--out` 单向中转目录交换待检包裹。
