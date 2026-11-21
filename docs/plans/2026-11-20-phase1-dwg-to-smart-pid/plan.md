# 阶段一总目录索引与可回溯推进台账（`2026-11-20-phase1-dwg-to-smart-pid`）

> 权威目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`
> 线上跟踪总单：[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)
> 维护规则：阶段一所有正在推进与已完成的任务，一律在本台账、同目录对应专项文档以及 `readings.md` 同步登记**日期、提交号、线上 Issue 编号与实测结论**，确保全链路可回溯。

---

## 1. 本目录权威文档清单

| 文件名 | 定位与职责 | 当前状态 |
|---|---|---|
| `智能PID平台-阶段一实施方案.md` | 阶段一主执行方案（L0–L6 + LX 逐任务做法、门禁、验收与逐项解决记录） | **已确认并执行中**（已对齐 `main @ 6377fe233` 与 `pid-parse @ a85fa10`） |
| `智能PID平台-阶段一Issue清单.md` | 阶段一拆单说明、线上 GitHub Issue 映射与逐单解决日志 | **已同步线上 `#106`–`#131`** |
| `智能PID平台-阶段一路线图-DWG转智能PID与pid导出.md` | 阶段一总体路线、取舍依据、预检三关与减法送测设计 | **已对齐当前仓库进度并脱敏** |
| `智能PID平台-样本分析-XML规格与pid结构.md` | 110 对真实 SPPID 样本格式/结构实测规格与 Q1–Q13 答案回填 | **已完成全批 220 份复核与 Q1–Q13 回填** |
| `03-parse-answers.md` | `L0-8`（[#118](https://github.com/happyrust/pid-cad/issues/118)）对照 `pid-parse @ a85fa10` 源码逐条回答 Q1–Q13（含精确源码路径与行号） | **已完成** |
| `04-external-platform-protocol.md` | `L0-7`（[#121](https://github.com/happyrust/pid-cad/issues/121)）外部数字化交付平台验收口径、三轮减法送测变体矩阵与回执标准 | **已完成** |
| `智能PID平台-内部模型合一分析.md` | `PID_SEMANTICS` 与 `PID_AUTHOR` 升格合一方案及与并行分支 `pid-symbol-identity` 的衔接边界 | **已对齐并行分支边界** |
| `智能PID平台-资料清单.md` | `RES-01`–`RES-17` 资料清点、环境变量约定与缺口降级策略（`L0-1`） | **已完成清点登记** |
| `readings.md` | 阶段一门禁基线与各单先红后绿、真实样本实测读数账本 | **持续更新（已记 §1–§18）** |

---

## 2. 阶段一任务推进与解决日志总表（实时回溯）

| 任务编号 | 线上 Issue | 状态 | 解决记录（提交号 / 产物 / 读数索引） |
|---|---|---|---|
| `L0-1` 资料清点与缺口登记 | 父单 [#106](https://github.com/happyrust/pid-cad/issues/106) | **已完成**（2026-11-20） | 完成 `RES-01`–`RES-17` 状态核实（见《智能PID平台-资料清单.md》） |
| `L0-6` 当前 `main` 门禁基线 | [#109](https://github.com/happyrust/pid-cad/issues/109) | **已完成**（2026-11-20） | 提交 **`9b5faf294`**：`7,145` 条库测试（`7,089` 过 / `55` 忽略 / `1` `.pen`），`clippy` `656` / `1,051`，`rustc 1.102.0-nightly`（见 `readings.md` §1） |
| `L0-2` 语料索引与 SHA-256 校验工具 | [#107](https://github.com/happyrust/pid-cad/issues/107) | **已完成**（2026-11-20） | 提交 **`538ec0112`**：落地 `scripts/sppid/corpus_index.py`、`corpus-layout.md`、`test_corpus_index.py`；只读索引真实语料 `1,115` 文件（`manifest_sha256=487b3203...`），见 `readings.md` §2 |
| `L4-0` XML 格式与结构校验器 | [#111](https://github.com/happyrust/pid-cad/issues/111) | **已完成**（2026-11-20） | 提交 **`6fd58799c`**：落地 `assets/pid-publish-spec.json`、`src/io/pid_publish_check.rs`、`scripts/sppid/xml_spec_check.py`；全量 `220` 份真实 SPPID XML 零违规，7 类改坏全检出，见 `readings.md` §3 |
| `L0-8` `pid-parse @ a85fa10` 能力核实 | [#118](https://github.com/happyrust/pid-cad/issues/118) | **已完成**（2026-11-20） | 落盘 `03-parse-answers.md` 与 `readings.md` §4，逐条给出源码路径行号，锁定 `L4-1` 方案 A 与 `L5-4` 整流替换记录链方案 |
| `L1-1` 显示修正五项与总验收 | [#112](https://github.com/happyrust/pid-cad/issues/112) | **已在基线完成** | 提交 `dd96a50f7..51b654a87`（含 `RD-7/11/8/14/9/13` + `ACC-1`），见 `2026-11-17-sppid-backup-publish/readings.md` §22–§29 |
| 备份包新版浏览与导出界面（关联） | [#115](https://github.com/happyrust/pid-cad/issues/115) | **已在基线完成** | 提交 `0f9f1f859..6377fe233`（检查点 C `UI-1..UI-9` + 审核加固），见 `2026-11-17-sppid-backup-publish/readings.md` §32–§41 |
| `L0-4` 度量工具集（XML/识别/PDF） | [#108](https://github.com/happyrust/pid-cad/issues/108) | **已完成**（2026-11-20） | 提交 **`52fbf5130`**：落地 `scripts/sppid/{xml_diff,legend_vs_xml,render_vs_pdf,test_metrics_108}.py`；94 套 XML/PDF 自检与 11 套厂商 PDF 验收记录核对全过，见 `readings.md` §5 |
| `L1-3` 符号缺失清单 | [#113](https://github.com/happyrust/pid-cad/issues/113) | **已完成**（2026-11-20） | 提交 **`11c8c784b`**：落地 `src/io/pid_sym_missing.rs`；全厂包 53 图（9 种不在库）与导出集 94 图（对比子集库差 84 种）100% 自带内嵌缓存本体（无缓存缺失 `0`），见 `readings.md` §6 |
| `L1-6` `.sym` 解码状态普查 | [#114](https://github.com/happyrust/pid-cad/issues/114) | **已完成**（2026-11-20） | 提交 **`336b7b362`**：落地 `src/io/pid_sym_census.rs` + `scripts/sppid/sym-census.ps1`；`plant644`（643 可画 + 1 空 / 0 失败）与 `full618`（618 可画 / 0 失败）两轮一致并澄清 618/644/728 口径，见 `readings.md` §7 |
| `L0-9` 符号与 SPPID 类变体对应表 | [#119](https://github.com/happyrust/pid-cad/issues/119) | **已完成**（2026-11-20） | 提交 **`9ac724b12`**：落地 `src/io/pid_variant_map.rs` 与零客户数据的 `assets/pid-class-map.json`（94 对样本 `106,150` 对象共 `237` 种变体，管件/管线段各 `78` 种），见 `readings.md` §8 |
| `L0-3` 样本对应关系表 | [#120](https://github.com/happyrust/pid-cad/issues/120) | **已完成**（2026-11-20） | 提交 **`d3fc99080`**：落地 `scripts/sppid/{corpus_correspondence,test_corpus_correspondence}.py`；核实 94 套三件套齐全、84 精确 + 10 近形字候选配对、剔除 4 份 0 字节空 DXF 锁定 90 张有效分母，见 `readings.md` §9 |
| `L0-7` 外部平台验收口径与减法送测预案 | [#121](https://github.com/happyrust/pid-cad/issues/121) | **已完成**（2026-11-20） | 提交 **`849133af1`**：落盘 `04-external-platform-protocol.md`（代理标准、`RES-15` 待补清单、三轮减法矩阵与回执模板），见 `readings.md` §10 |
| `L0-5 / L3-0` 94 套样本识别与真值基线报告 | [#122](https://github.com/happyrust/pid-cad/issues/122) | **已完成 · 检查点 A 达成**（2026-11-20） | 提交 **`833eb92e9`**：落地 `scripts/sppid/{legend_baseline,test_legend_baseline}.py`；实跑 90 张有效 DXF（真值 `15,957` 符号 vs 检出 `17,185` 块，查明块名主干与 `.sym` 名一一对应及 7 位序号缺口），见 `readings.md` §11 |
| `L4-1 / L4-2 / L4-3` 自有模型导出 publish XML 底座 | [#123](https://github.com/happyrust/pid-cad/issues/123) | **已完成**（2026-11-20） | 提交 **`b49ed5274`**：落地 `src/io/pid_publish/{mod,uid,model,classes,write}.rs`（共享确定性 UID/`GraphicOID` 分配、237 种变体映射、严格字节格式写出器）；合成与 1,881 对象真实单图回环双端 `StrictVendor` 零违规、差异 100% 可解释，见 `readings.md` §12 |
| `L3-1` 图例到 SPPID 类变体映射规则与解析器 | [#124](https://github.com/happyrust/pid-cad/issues/124) | **已完成**（2026-11-20） | 提交 **`1bb06191e`**：落地 `assets/pid-sppid-legend-map.json` 与 `src/io/pid_convert/class_map.rs`；90 张有效 DXF 净业务图块 **`15,852 / 15,852`（100%）** 映射至 SPPID 类变体（未映射 `0`），见 `readings.md` §13 |
| `L3-2 / L3-7` 存量图位号识别增强与三档置信度及 90 图复跑 | [#125](https://github.com/happyrust/pid-cad/issues/125) | **已完成**（2026-11-20） | 提交 **`76d734140`**：落地 `src/io/pid_convert/recognize.rs`；90 张有效 DXF 两轮复跑 100% 一致，符号召回率由 `0.000188` 跃升至 **`0.977941`（`15,605 / 15,957`）**、精确率 **`0.998912`**、位号一致率 **`0.895546`**（图面可见位号约 `94.6%`），三档置信度守恒，见 `readings.md` §14 |
| `L4-4 / L4-5` 管网派生对象合成与名称、公称直径规则 | [#126](https://github.com/happyrust/pid-cad/issues/126) | **已完成**（2026-11-20） | 提交 **`1afee04eb`**：落地 `src/io/pid_publish/{names,relations}.rs`；94 份真实 XML 核对公称直径 `19,671 / 19,671`（`100%`）、仪表 `99.48%`、设备 `99.14%`，管网拓扑合成 100% 过 `StrictVendor` 零违规，见 `readings.md` §15 |
| `L4-6 / L4-7` 单图与批量 DWG/DXF 转 publish XML 入口与对账 | [#127](https://github.com/happyrust/pid-cad/issues/127) | **已完成 · L4 全线收官**（2026-11-20） | 提交 **`477b08b61`**：落地 `src/io/pid_convert/export.rs`、`examples/pid_convert.rs`、`assets/pid-xml-diff-allowlist.json` 与 `scripts/sppid/convert_check.py`；90 张真实 DXF 一键转换（`77,491` 对象 / `71,415` 关系），`180 / 180` 份 XML `StrictVendor` **0 违规**、白名单之外 **0 未解释差异**，见 `readings.md` §16 |
| `L5-1 / L5-3` 写入面普查与空白模板种子流级往返验证 | [#128](https://github.com/happyrust/pid-cad/issues/128) | **已完成**（2026-11-21） | 提交 **`c851fa214`**：落地 `src/io/pid_write/{mod,surface,seed,verify}.rs`；18 类顶层流写入面完整分类，`RefData~4~682.zip` 内 **`10 / 10` 份空白模板 `.pid` 流级零差异无损往返** + 第 2 节 `_SPID` 注入重读全过，见 `readings.md` §17 |
| `L5-4 / L5-5` `.pid` 记录链按族增补写出器与索引一致性自检 | [#129](https://github.com/happyrust/pid-cad/issues/129) | **已完成**（2026-11-21） | 落地 `src/io/pid_write/{oid,records,index}.rs`；五族记录编码 + `.sym` 内嵌缓存克隆 + Q9 五步索引同步在真实 `A1-W-New.pid` 种子上 `chain_fault_count=0`、图层计数与空间表 100% 守恒，见 `readings.md` §18 |
| `L5-6 / L5-8 / L6-1` DWG/DXF 一键双产物（XML + `.pid`）及 `a/b` 级验证 | [#130](https://github.com/happyrust/pid-cad/issues/130) | **进行中** | 扩展 `src/io/pid_convert/export.rs` 与 `examples/pid_convert.rs --seed-pid` 打通双产物导出（共享 `UID`/`GraphicOID`）与 `a/b` 级往返出图验证 |
| `L1-2 / L1-5` 94 张真实 `.pid` 批量打开耗时与渲染出图扩面对比 | [#131](https://github.com/happyrust/pid-cad/issues/131) | **待推进（已建单）** | 94 张 `.pid` 批量性能基线与对 `EXPORT_PDF` 全批覆盖率/多画/文字对比 |
| `L2-1..L2-6` 内部模型合一与编辑 | 待拆单 | **暂缓（避让并行分支）** | 等活跃分支 `pid-symbol-identity` 合入 `main` 后承接其底座实施 |
| `.sym` 参考数据预览（关联） | [#116](https://github.com/happyrust/pid-cad/issues/116) | **暂缓（避让并行分支）** | 等 `pid-symbol-identity` 合入 `main` 后实施，避免撞 `pid_backup_browser.rs` 与 `src/io/pid/` |
