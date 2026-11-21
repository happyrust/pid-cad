# `L0-8`（[#118](https://github.com/happyrust/pid-cad/issues/118)）：`pid-parse @ a85fa10` 源码能力核实（Q1–Q13 逐条解答）

> 日期：2026-11-20
> 状态：**已完成**（对照 `Cargo.toml` 钉定的 `pid-parse @ a85fa10285cc5379953495fa391f374981b9e587` 源码、测试与反编译分析台账逐条核实）。
> 用途：回答《智能PID平台-样本分析-XML规格与pid结构》第 6 节与《智能PID平台-阶段一实施方案》第 6 节提出的 13 个关键问题，锁定 `L4`（publish XML 导出）与 `L5`（`.pid` 生成）的技术路线。
> 写法约定：源码路径均相对于 `pid-parse` 仓库根目录（通过 `$OCS_PID_PARSE_SRC` 访问），不含盘符、用户名、主机名或客户图纸信息。

---

## 0. 核心路线决策汇总

1. **`L4-1`（统一模型 → publish XML）选定「方案 A：转换至 `pid-parse::publish` DTO + 严格厂商字节格式化包装」**：`pid-parse` 已有成熟的 `PublishDrawing` / `PidPublishDrawing` 写出器、`catalog` 21 类接口目录、`codelist_values` 58 条分级链、`pipeline` 确定性 `UUID v5` 合成与 `unit_values` `oleaut32` 数值换算；无需重写业务属性排布器，只需补齐自有模型到 DTO 的映射层，并在外层封装 UTF-8 BOM + CRLF + `0/3/6/3/2` 空格缩进以通过 `#111` 严格模式门禁。
2. **`L5-4`（`.pid` 写出器）选定「基于 `PidPackage` + `PidWriter::stream_replacements` 扩展记录链编码与索引同步层」**：`PidWriter` 已解决 CFB 容器重打包与 CLSID 保留，但没有高层「新增图形/对象记录」API；需在种子 `.pid` 上按记录族编码 `/Sheet#`、`/PSMcluster0`、`/Unclustered Dynamic Attributes` 增量并按 Q9 清单同步 `/PSMspacemap/*` 与图层计数。
3. **`L5-2` / `LX-3`（受控样本差分）直接复用现成工具**：`pid-parse` 已内置 `pid_inspect --controlled-diff-dir <dir> --json` 与采集协议，无需重造差分脚本。

---

## Q1 `.pid` 写出能力与 API 边界

- **结论（一句话）**：`pid-parse` 已实现基于 `PidPackage` 的 CFB 容器流级回写、`/TaggedTxtData/*` XML 替换、OLE 摘要属性字符串增删改、任意流整块字节替换与实验性字节区间补丁（`SheetPatch`），**尚无从高层文档模型构造图形记录或「新增一个对象并自动维护索引」的 API**。
- **证据**：
  - `src/writer/mod.rs:4-75`（`PidWriter::write_to`、`PidWriter::write_to_bytes`，保留根与子存储 CLSID，按路径字典序写出流并刷新容器时间戳）；
  - `src/writer/plan.rs`（`WritePlan { metadata_updates, stream_replacements, sheet_patches }`）；
  - `src/writer/summary_write.rs`（OLE `[MS-OLEPS]` `VT_LPSTR` / `VT_LPWSTR` UTF-8 读写与删除）；
  - `tests/writer_roundtrip.rs`、`tests/writer_real_files.rs`、`docs/writer-layer-plan.md:1-107`。
- **状态**：明确有 + 测试证明（容器/整流替换已完备；高层对象新增 API 明确未实现）。
- **对计划的影响**：
  - `L5-4` 定为在 `PidPackage` + `WritePlan::stream_replacements` 底座上构建记录链编码器，不重复实现 CFB 容器层；
  - `L5-3` 种子原样往返验收判据修正为「容器内全部流字节与存储 CLSID 100% 相同（`diff_packages` 零流差异）+ 重读相等」，而非外层 `.pid` 文件 SHA-256 相同。
- **遗留**：SPPID 原生程序是否直接接受 `cfb::create` 字典序重打包后的种子容器，由 `LX-4` E1' 实测确认。

---

## Q2 各 CFB 流的解码状态与覆盖台账

- **结论（一句话）**：`pid-parse` 已内置动态解析覆盖台账 `inspect::coverage`（CLI：`pid_inspect --coverage --json`），全部已知顶层流与存储均具备静态分级与「解析结果为空自动降级」检查，所有 `0x6C90F544` 记录链流均可精确走链至零剩余字节。
- **证据**：`src/inspect/coverage.rs:10-55`、`:221-420`、`docs/phase10-coverage-series-summary.md:122-210`、`docs/pid-format-guide.md:62-95`。

| 流 / 存储 | 覆盖台账状态 | 解析模块与能力说明 |
|---|---|---|
| `/\x05SummaryInformation`、`/\x05DocumentSummaryInformation` | `FullyDecoded` | `src/streams/summary.rs`（含第 2 节用户自定义属性字典） |
| `/PSMroots` | `FullyDecoded` | `src/streams/psm_roots.rs`（根对象名 → 持久 ID） |
| `/DocVersion2`、`/DocVersion3` | `FullyDecoded` | `src/parsers/doc_version2.rs`（9 字节步长）、`src/streams/version_history.rs` |
| `/AppObject`、`/JTaggedTxtStgList`、`/JSitesList` | `FullyDecoded` | 应用注册表、标签文本存储表、`'OLEM'` 站点 ID 列表 |
| `/Sheet*` | 记录链 100% 分帧 + 主体图形族已解码 | `src/parsers/sheet_records.rs`（线、圆、圆弧、多段线、B 样条、椭圆、椭圆弧、矩形、边界填充、文本框、符号放置、连接点、依赖组、线性驱动尺寸） |
| `/JSite*/*` | 存储识别 + 嵌套几何与参数族已解码 | `src/streams/jsite.rs:42-98`（`JProperties`、嵌套符号缓存本体、`JFlavorHolder` 驱动参数） |
| `/PSMspacemap/0x*` | 全量走通（零剩余字节） | `src/parsers/psm_tables.rs`（`'tseg'` 段头、自由表与反向入边槽） |
| `/PSMcluster0`、`/Unclustered Dynamic Attributes` | 记录链全走通 + `0x0089` 属性行完全解码 | `src/streams/cluster.rs`、`src/parsers/attribute_rows.rs` |
| `/StyleCluster` | 记录链全走通 + 线/填充/段落/字符样式已解码 | `src/style_link.rs` |
| `/PSMclustertable`、`/PSMsegmenttable` | `PartiallyDecoded` | `src/streams/psm_tables.rs`（表结构与候选字段已解，部分原生标志位待受控样本核实） |

- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L5-1` 直接基于 `coverage_report(&doc)` 生成写入面清单，聚焦 `PSMclustertable` / `PSMsegmenttable` 在增补记录时是否需要改动。
- **遗留**：`Dynamic Attributes Metadata`（固定 28 字节）在新增属性行时是否恒定不变，在 `L5-2` 受控样本中核实一笔即可。

---

## Q3 `/Unclustered Dynamic Attributes` 行格式与「两跳」连接机制

- **结论（一句话）**：`0x0089`（`FreeFormAttrSet`）属性行已完全解码；通过 `P&IDAttributes` 行 `+12` 字段直连图元 `GraphicOID`，并通过 `0x00FA` `DependencyObject`（`+16` 挂图元、`+22` 指属性行、`0x004D` 文本框 `+4` 指该依赖）完成标注与属性值的两跳精确连接。
- **证据**：
  - `src/parsers/attribute_rows.rs:1-103`：payload `+0` 为行自身 oid，`+12`（`ATTRIBUTE_ROW_DESCRIBED_OFFSET`）为被描述记录 oid，`+20`（`ATTRIBUTE_SET_OFFSET`）为 `0x0000` 集头与 NUL 结尾 ASCII 类名（`P&IDAttributes`、`SymbolAttr`、`_TagSequenceNo` 等），后接紧凑字段帧：`0x8010` 文本（UTF-8 优先，双字节汉字块回退 GBK）、`0x0003` `i32`、`0x000B` `bool`；
  - `src/publish/pid_load.rs:9-20`、`:46-57`：`TEXT_PARENT_OFFSET = 4`、`DEPENDENCY_GRAPHIC_OFFSET = 16`、`DEPENDENCY_ROW_OFFSET = 22`；
  - 53 张全厂图实测：`9,218 / 9,225` 个符号实例唯一连到模型对象（仅 7 个在图纸与数据库中均无模型对象）。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L0-9`（逐对象对应与类变体统计）与 `L4-3` 直接复用 `pid_load` 与 `attribute_rows`，零歧义。
- **遗留**：无。

---

## Q4 管线（`PIDPipeline`）UID、名称与管线段关系的合成算法

- **结论（一句话）**：SPPID 库表 `T_PIPELINE` 本身为空，管线由非空 `TagSequenceNo` 的 `PIDPipingConnector` 按管线名聚类合成，管线 UID 采用 `DNS_NAMESPACE` → `"pid-parse/pipeline"` → `plant` → `pipeline_name` 的三级确定性 `UUID v5`（32 位大写十六进制），与管线段生成 `PipingConnectors`（前缀 `PCN`）关系。
- **证据**：`src/publish/pipeline.rs:3-25`、`:47-189`：
  - 名称优先级：若同 `SP_ID` 的 `T_PlantItem.ItemTag` 非空且含 `'-'`，直接采用 `ItemTag`（无 `'-'` 的纯数字如 `02532` 判定为非管线名跳过；7 张参考图 `629 / 629` 命中并精确排除 `2` 条未发布项）；无 `PlantItem` 时按 `{nominal}-{fluid}-{sequence}-{class}[-{insulation}]` 拼出；
  - UID 公式：`pipeline_uid(plant, &name)`（`src/publish/pipeline.rs:181-189`），同一工厂名与同一管线名永远产出同一 32 位大写十六进制 UID。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L4-4` 直接调用或复刻同一 `UUID v5` 命名空间算法，保证从自有模型导出的管线 UID 确定且可复现。
- **遗留**：SPPID 原生导出时的瞬态管线随机 GUID 与确定性 `UUID v5` 字面值不同，在 `L4-7` / `#108` `xml_diff.py` 中应按管线 `Name` 对齐管线对象而非按字面 UID 判缺失。

---

## Q5 仪表、设备、管线段、管线、控制功能的组合名称拼接规则与例外

- **结论（一句话）**：单图属性拼接规则为仪表 `{MeasuredVariable}-{InstrTagSequenceNo}`、设备 `{EquipPrefix}-{EquipSeqNo}`、管线按 Q4 五槽位连字符拼接；带备份数据库时以 `T_PlantItem.ItemTag` 为准。
- **证据**：
  - `src/publish/pid_load.rs:21-25`、`:96-102`：5 张参考图实测仪表 `236 / 236`、设备 `13 / 13` 零误差（`EFAdapter.dll` 内无硬编码拼接函数，由对象各段属性合成）；
  - `src/publish/pipeline.rs:111-138`：缺公称直径时保留前导连字符（如 `-PG-1001105-F9`），流体代码不在码表时省略该槽位并计数。
- **已知例外**：改造图上偶有图面分段属性改为新序列号、而数据库 `ItemTag` 保留 `old_` 前缀旧号的情况（53 张全厂图中占 32 处），单从图面或 `.pid` 导出时会写出新拼号。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L4-5` 直接复用上述规则；在 DWG 识别路径中若识别出完整位号文本，同时回填分段属性与 `Name`，保持自洽。
- **遗留**：无。

---

## Q6 公称直径与带单位数值属性的换算规则

- **结论（一句话）**：英寸写法将 `N"`、`a/b"`、`N-a/b"` 转为 `N in`（公制 `N MM` 或纯数字转为 `N mm`）；整数编码 `< 5000` 为 `{code} mm`、`>= 5000` 按 `(code - 5000) / 32` 约分；SI 浮点属性按 `oleaut32` `VarBstrFromR8` 15 位有效数字规则拼 `{数值} {单位}`。
- **证据**：
  - `src/publish/pid_load.rs:27-34`（图面 `_NominalDiameter` 标签转换规则，对应原生 `EFAdapter.dll` `NominalDiameterConv @ 0x5F52BDE0`）；
  - `src/publish/pipeline.rs:153-179`（整数编码换算与最大公约数约分）；
  - `src/publish/unit_values.rs:14-149`（`PublishUnit::{Millimetre, Metre, Radian, CubicMetre}` 与 `automation_text` 15 位有效数字四舍五入规则）。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L4-5` 直接复用，杜绝手写浮点格式化带来的尾数/科学计数法偏差。
- **遗留**：无。

---

## Q7 码表值来源与「类目录」覆盖度

- **结论（一句话）**：`src/publish/catalog.rs` 定义了 21 类发布对象的接口顺序与属性发射条件（`always` vs `if_value`），`src/publish/codelist_values.rs` 内置了从 107 份厂商参考 XML 核对出的 58 条零冲突末级→上级类型分级链（`LEAF_LEVELS`），对当前验证厂 53 张图实现 100% 命中、对 94 套单图验证集实现 `82 / 94`（87.2%）整图一致。
- **证据**：
  - `src/publish/catalog.rs`（`CLASS_INTERFACES` 与 `PublishItemRow`）；
  - `src/publish/codelist_values.rs:85-165`（`LEAF_LEVELS` 58 条、`SPECIALTY_ITEM` 2 条、`DRAWING_CONSTANTS` 2 条、`PLANT_ADDED_INSTRUMENT_TYPE = 10_000`）；
  - `src/publish/pid_load.rs:16-20`（仪表按 `SymbolAttr` 或符号库目录 `Instrumentation\<family>\...` 分流至 `PIDInstrument` / `PIDControlSystemFunction` / `PIDInlineInstrument`）。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L0-9` 与 `L3-1` 直接提取 `catalog.rs` + `codelist_values.rs` + 94 套样本的约 240 种「接口集 + 码表元组」生成外置 `assets/pid-class-map.json`；表外未命中计数与 `#117` 对齐写进覆盖报告。
- **遗留**：跨厂扩展时需按 `#117` 支持外置替换厂级码表。

---

## Q8 XML 写出函数的输入结构与 `L4-1` 路线选择

- **结论（一句话）**：`pid-parse` 的 `write_data_xml(&PublishDrawing, ...)` 与 `write_pid_data_xml(&PidPublishDrawing, plant_name)` 均接收纯内存 Rust 结构体，完全不依赖 `.pid` 文件或数据库连接，可直接由本仓库统一模型构造调用。
- **证据**：
  - `src/publish/model.rs:1-80`（`PublishDrawing`、`PublishObject`、`PublishRelationship`、`PublishRepresentation`、`CodelistIndex`）；
  - `src/publish/xml_writer.rs:48-80`、`src/publish/pid_writer.rs:48-80`。
  - **格式注意**：两套函数内部写出为无 BOM、LF、2 空格缩进（见 `src/publish/xml_writer.rs:21-24`），而 SPPID 原始样本与 `#111` 严格模式要求 UTF-8 BOM + CRLF + `0/3/6/3/2` 空格缩进。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：**`L4-1` 确定采用「统一模型 → 发布 DTO 转换层 + 严格厂商字节后处理（BOM + CRLF + 缩进规整）」**，既完全复用 `catalog` 排序与转义规则，又 100% 满足 `#111` 严格厂商门禁。
- **遗留**：无。

---

## Q9 `GraphicOID` 分配规则、跨存储作用域与新增对象同步清单

- **结论（一句话）**：`GraphicOID` 即存储内持久 ID `(segment << 13) | index`（每段上限 `8,192`，顶层与各 `JSiteN` 存储各自独立编号），新增一个对象必须同步记录链流、所属 `JSheetLayer` 对象计数与 `/PSMspacemap` 反向入边表。
- **证据**：`docs/pid-format-guide.md:97-340`（源自 `radsrvitem.dll` `Segment::Load @ sub_5647A180`、`Segment::Save @ sub_5647B550` 与 `shlyhp.dll` `SheetLayer::IJPersistImp::Save` 反编译及四图 `1,948 / 1,948` 条目、`3,988 / 3,989` 活边实测）。
- **新增一个顶层图上对象的完整同步清单**：
  1. **分配持久 ID**：从顶层 `/PSMspacemap/0x00000000`（或下一段）取自由表槽位或递增 `m_iNext`（`< 0x2000`），分别给几何记录、标注文本框、`0x00FA` 依赖组、`0x0089` 属性行分配唯一 oid；
  2. **写记录链并更新链头**：将几何/文本/符号放置记录追加至 `/Sheet#`（`+8` `aux_hi` 填所属 `JSheetLayer` oid），将 `0x0089` 属性行与 `0x00FA` 依赖组追加至 `/PSMcluster0` 或 `/Unclustered Dynamic Attributes`，同步流头 `+4` `record_count`；
  3. **更新图层对象计数**：将目标 `0x0081 JSheetLayer` payload `+12` 的 `u32` 对象计数加 `1`（样本 `319 / 320` 图层精确守恒）；
  4. **更新 `/PSMspacemap` 入边槽**：`PSMspacemap` 是**反向引用表（「谁引用我」）**——为被 `0x0089` 描述的记录登记 `(attr_row_oid, tag 190)` 入边，为被 `0x00FA` 指向的两端登记 `(dep_oid, tag 249)` 入边，若放置符号引用 `JSiteK` 则在 `JSiteK` 条目与站点列表登记 `181` / `182` 对偶入边；
  5. **若新增嵌套符号定义存储 `JSiteK`**：同步创建 `/JSiteK/*` 子流，并将 `K` 写入 `/JSitesList`（`'OLEM'` 表）与顶层站点列表对象 `2`。
- **状态**：明确有（`native-reader` 反编译级证据）。
- **对计划的影响**：直接作为 `L4-3` OID 分配器与 `L5-5` 索引一致性自检器的逐项实现规格。
- **遗留**：`PSMclustertable` 与 `PSMsegmenttable` 在不新开段（`m_iNext < 8192`）时是否可原样保留种子值，在 `L5-2` 单一加线受控样本中做最终核对。

---

## Q10 摘要流中的 `_PIDTemplate`、`_PlantName`、`_SPID`

- **结论（一句话）**：三者存储在 `/\x05DocumentSummaryInformation` 第 2 节用户自定义属性字典中，其中 `_SPID` 等于图纸 `PIDDrawing` 的 32 位大写十六进制 `DocUID`，`_PlantName` 为工厂名，`_PIDTemplate` 为图框模板路径/名称。
- **证据**：`src/streams/summary.rs:17-58`（`FMTID_DOC_SUMMARY_SECTION_2 = {D5CDD505-2E9C-101B-9397-08002B2CF9AE}`，`parse_doc_summary_section_2` 读入 `SummaryInfo::user_properties`）。
- **状态**：结构与读取明确有；原生 SPPID 打开时是否强校验第 2 节 `_SPID` / `_PlantName` 与 plant 库匹配由 `LX-4` E2 实验确认。
- **对计划的影响**：`L5-3` 从种子生成新图纸时，需同步重写第 2 节字典里的 `_SPID`（与 XML `DocUID` 保持同一值）和 `_PlantName`（当前 `writer::summary_write` 只写第 1 节标准属性，需补一个小辅助函数改写第 2 节字典）。
- **遗留**：`LX-4` E2 验证修改 `_SPID` 后 SPPID 的打开行为。

---

## Q11 为何 94 个样本无 `/TaggedTxtData/Drawing`、`/General` 而只有 0 字节 `/Revision`

- **结论（一句话）**：`/TaggedTxtData/Drawing` 与 `/General` 仅存在于特定旧版或附加导出配置中；常规 SPPID 生产图纸本身只保留 0 字节占位流 `/TaggedTxtData/Revision`，图号实际存于顶层 `0x0089` `P&IDAttributes.DrawingNo`，图框模板信息存于 `DocumentSummaryInformation` 第 2 节 `_PIDTemplate` 与图纸 `0x003D` `igSmartFrame2d` 记录中。
- **证据**：`src/streams/tagged_text.rs:16-36`（缺失时静默置 `None`）、`src/publish/pid_load.rs:9-13`、`docs/analysis/2026-10-29-the-two-a1-pages-name-their-template.md`。
- **状态**：明确有 + 测试证明。
- **对计划的影响**：`L5-1` / `L5-3` / `L5-4` 绝不靠写 `/TaggedTxtData/Drawing` 填图号，统一写 `P&IDAttributes` 行（含 `DrawingNo`、`DrawingID`、`ModelID`、`ModelItemType`）并原样保留种子的 0 字节 `/TaggedTxtData/Revision`。
- **遗留**：无。

---

## Q12 `/JSite#/JProperties` 符号 UNC 路径对打开的影响

- **结论（一句话）**：每个 `JSite#` 存储除在 `JProperties` 记录源 `.sym` 的 UNC/本地路径外，已在自身子流内完整嵌入了符号图形与样式缓存本体；离线渲染与解析完全依赖内嵌缓存本体而不访问 UNC 路径，新生成 `.pid` 时应同时写入内嵌符号本体与规范化工厂相对符号路径。
- **证据**：`src/streams/jsite.rs:24-98`、`docs/analysis/2026-09-07-nested-site-curves-are-embedded-symbol-bodies.md`（全厂包 `9,225` 个符号放置 100% 命中图纸内嵌缓存本体）。
- **状态**：内嵌缓存机制明确有；原生 SPPID 在原 UNC 不可达时的容错表现随 `LX-4` E1/E3 一并观察。
- **对计划的影响**：`L5-4` 写入符号放置时，从符号库 `.sym` 提取本体克隆进新建的 `/JSiteK/*` 存储，并将 `JProperties` 路径写成与种子同构的符号库路径格式。
- **遗留**：`LX-4` 顺带记录 SPPID 打开离线副本时对不可达 UNC 的提示情况。

---

## Q13 现有「受控样本」证据与工具链

- **结论（一句话）**：`pid-parse` 已有定型的受控原子编辑差分协议与自动化解析命令 `pid_inspect --controlled-diff-dir <dir> --json`，但仓库内未提交专有 `.pid` 样本对（在本地无 `test-file/controlled-diff` 时自动软跳过）。
- **证据**：`docs/analysis/2026-05-09-controlled-pid-diff-protocol.md:1-118`（定义了画单线、折线、圆、弧、文本、单符号 7 类原子操作、`before/` + `after/` + `metadata/*.json` 目录契约及晋升判据）。
- **状态**：协议与工具明确有；真实受控 `.pid` 样本对需在受控 SPPID 环境补采。
- **对计划的影响**：`L5-2` 与 `LX-3` 直接按该协议采集样本放入语料 `CONTROLLED` 段，并用 `pid_inspect --controlled-diff-dir --json` 直接产出流级与字节区间变更报告，省去新写差分工具的工作量。
- **遗留**：待 `RES-06`（受控 SPPID 环境快照就绪）后手工执行原子编辑采集。
