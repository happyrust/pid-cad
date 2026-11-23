# `L6-3`（[#135](https://github.com/happyrust/pid-cad/issues/135)）：智能 PID 平台 · 阶段一总验收报告（本地离线闭环与首轮送测就绪版）

> 日期：`2026-11-21`（**`2026-11-23` 对账更新**：`#107`–`#176` 及关联单 `#115`–`#117` 均已推送至 `pid-cad/main` 关单）
> 所属目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`（跟踪总单：[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)）
> 编译器版本：`rustc 1.102.0-nightly (65ce8611f 2026-11-18)`；`pid-parse` 当前钉定提交：`7cb03e48a564311d6b83cbde5bc1af9dc79e8e51`（含 `#117` 备份包缺口计数，兼容 `a85fa10` 基线规格）
> 绑定语料清单摘要：`manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038`（`1,115` 文件只读 SHA-256 绑定）
> 当前版本定位：**本地离线闭环全线达成 + 四套送测/真机探针实物包（22 槽 65 文件）审计就绪**。外部数字化交付平台最低要求当前未知，本报告严格依据《智能PID平台-阶段一实施方案.md》第 3 节五条验收口径给出实测结论，凡依赖受控 SPPID 环境（`RES-06`）与外部平台回执（`RES-15`）的环节均显式标明「未执行 / 待外部回执回填」，绝不混同「本软件解析器认」与「SPPID / 外部平台认」。

---

## 1. 对照《实施方案》第 3 节五条总验收口径逐条结论

| 口径编号 | 实施方案第 3 节验收要求 | 当前判定状态 | 量化证据与账本索引 |
|---|---|---|---|
| **口径 1：代理标准（与已通过样本一致，差异可解释）** | 导出的 XML 与 `.pid` 在格式、结构、接口与属性上与已通过样本逐项对齐；差异清单必须可解释且受白名单评审守门 | **本地已通过** | ① **90 张有效 SPPID 导出 DXF**：一键转换产出 `88,236` 个对象、`96,888` 条关系（`failed = 0`）；`180 / 180` 份 `_Data.xml` / `_Meta.xml` 通过 `#111` `StrictVendor` 零违规校验；`convert_check.py` 对参考 `_Data.xml` 实测 **`unexplained_diffs = 0`**；符号召回率 **`97.79%`**（`15,605 / 15,957`）、精确率 **`99.89%`**、图面可见位号一致率 **`~94.6%`**、管口挂设备率（含设备图）**`89.86%`**、标题栏图号提取 **`91.11%`**（`readings.md` §3、§14–§16、§21–§22）；② **12 张真实原生 AutoCAD 存量 DXF（`SOURCE_DWG`，`L01..L12`，含天正图块+散圆仪表图与炸散散线形状图两套标准，`#138`）**：精确变体映射由 `0` 跃升至 **`1,619`**（`class_default_fallback = 0`），产出 `6,867` 对象 / `7,147` 关系，`24 / 24` 份 XML 过 `StrictVendor` 零违规（`readings.md` §27） |
| **口径 2：日常门禁（`.pid` 的 `a/b` 级与 XML 校验器）** | 每个任务执行 `.pid` `a` 级（解析器重读相等 + 索引无悬挂）、`b` 级（本软件加载显示与参考 PDF 一致）及 XML 格式/结构校验器 | **本地已通过** | ① **生成 `.pid` 双产物侧**：10 份空白模板 `.pid` 流级零差异无损往返（`#128`），五族记录链编码 + Q9 五步索引同步 `chain_fault_count = 0`（`#129`），90 张 SPPID DXF 全量双产物导出 **`42,861 / 42,861`（`100%`）** + 12 张原生 AutoCAD 存量 DXF **`3,454 / 3,454`（`100%`）** `GraphicOID`/`ModelID`/`DrawingID` 跨文件严格相等，合计 **`102 / 102`** 一次通过 `a` 级重读与 `b` 级 CAD 重载 + `pid-plot` PDF 出图验证（`readings.md` §17–§19、§27）；② **读取显示 `.pid` 侧**：`EXPORT_SET` 全部 `94 / 94` 张真实 `.pid` 零故障打开与出图，`release` 单图打开中位数 **`536.5 ms`**（`0.733x` 基线）、场景准备均值 **`63.1 ms`**，`L1-4` `27,148` 条文本 `Flattened = 0`；`L1-2` 94 图对 `EXPORT_PDF` 对位 `94 / 94` 通过，覆盖率均值 **`0.995949`**、中位数 **`1.000000`**（`69` 张达 `1.000`、`5` 张 `< 0.980` 全部定位到加长图框模板缺失或副表掩膜边界）（`readings.md` §20） |
| **口径 3：送测前预检三关** | ① XML 与已通过样本逐项对照；② `.pid` 在 SPPID 里能打开（`c1`）；③ SPPID 发布 `.pid` 得到的 XML 与直接导出 XML 一致（`d`） | **第①关已通过；第②③关待 `RES-06` 执行（SPPID 未验证）** | 第①关已在全批 90 图转换产物与第一轮减法送测包上 `100%` 通过；因当前机器未配置受控 SPPID 运行器（`$OCS_PID_SPPID_RUNNER` 未设），第②关（`c1`）与第③关（`d`）按规程标为 **`unverified_no_sppid_runner`（SPPID 未验证）**，待受控环境就绪后执行 `LX-4 / LX-5 / LX-8` |
| **口径 4：成组单变量「减法送测」** | 基于同一已知通过样本构造单变量减法变体组，随附零改动原样阳性对照，一次摸清外部平台硬门槛 | **首轮减法包 + 配套八槽全原型转换与编辑重建试点包均已生成并通过本地预检；待投递回填 `receipt.json`** | ① `build_submission_round1.py`（`#135`）已产出 `submissions/round-01-20261121/`（`00-control` + `V1-01..V1-05` 共 6 槽 17 文件，`manifest_sha256 = 26eb6189e08d506a...`，`precheck_passed = true`）；② `build_submission_converted.py`（`#140` / `#143` / `#147`）已产出 `submissions/round-01b-converted-20261121/`（`CONV-01..CONV-08` 共 8 槽 24 文件，覆盖 SPPID 拆分图、原生存量图块+散圆、原生存量炸散形状、原生存量图应用内 `PIDSAVEPID`、二进制 `AC1032` `.dwg` 直转、既有带 XML `.pid` 应用内编辑存回、外置 `.xlsx` 设备/管线表回填 `.PPT` 温压及全厂备份包纯裸无 XML `.pid` 逆向重建双产物，`manifest_sha256 = 325551c26454b781...`，`precheck_passed = true`）（`readings.md` §23、§29、§32、§36） |
| **口径 5：严格区分「本软件解析器认」与「SPPID / 外部平台认」** | 汇报、覆盖报告与清单中明确区分本地重读/出图通过与原生 SPPID / 外部平台通过 | **严格遵守** | 每份 `_Coverage.json` 的 `pid_verification.sppid_native_c1_status`、`submission-manifest.json` 的 `sppid_c1_d_precheck_status` 及 `readings.md` §17–§23 均显式标注 `unverified_no_sppid_runner`，零越界宣称 |

---

## 2. 七条工作线与检查点达成总览

| 工作线 / 检查点 | 达成状态 | 原子提交范围与核心产物 |
|---|---|---|
| **`L0` 基线与语料**（`L0-1..L0-9`） | **100% 已完成** | `#107–#109, #118–#122`（`9b5faf294..833eb92e9`）：语料 SHA-256 索引、三大度量脚本、90 张有效 DXF 分母核定、`pid-parse` Q1–Q13 源码问答、237 种类变体表 `assets/pid-class-map.json`、外部送测规程 |
| **`L1` 读取与显示**（`L1-1..L1-6`） | **100% 已完成** | `#112–#115, #131`（基线 + `11c8c784b..336b7b362`, `e2f43b91c`）：显示修正五项与 `ACC-1`、新界面导出入口、不在库符号 100% 内嵌缓存核实、`644/618` `.sym` 零失败普查、94 图性能基线与 PDF 扩面对比 |
| **`L2` 内部模型与点选编辑**（`L2-1..L2-7`） | **100% 已完成（`#134` + `#136` + `#139` + `#142` + `#146` + `#159`）** | `src/io/pid_convert/promote.rs` + `src/app/commands/pidprops.rs`：零冲突完成 `M1..M5` 幂等升格、手工位号保护、`>2048B` XRecord 溢出、`L2-2..L2-4` 属性面板双分组与图面单/双行位号文字联动、多图元炸散及 `.pid` 多笔画符号 `_member_handles` 整组高亮/面板点选/成员归并 `ACCEPT/REJECT`、`PIDREVIEW DEDUP` 一键消重、多折线 `LwPolyline` 管段保真、**无伴随 `_Data.xml` 纯裸 `.pid` 放置边界归并与逆向拓扑重建**及 `L2-6 / L2-7` 应用内 `PIDSAVEPID` / `PIDCONVERT` 存回双产物（SPPID 生产 DXF `165/165` + 全部 12 张原生 AutoCAD 存量图 `1,619/1,619` 符号 `3,044/3,044` 对齐 + 3 张编辑存回真实 `.pid` `633/633` 精确变体 `897/897` 对齐 + 全部 5 张 `VENDOR_REF` 参考 `.pid` 与 3 张无 XML 纯裸 `.pid` 共 8 图 `929/929` 精确变体 `2,732/2,732` `GraphicOID` 对齐全过 `a/b` 级验证，见 `readings.md` §24–§25、§28、§31、§35），以及面向模块化多系列复制的批量智能重编号与单步可撤销命令 `PIDRENUM`（`#159`，`readings.md` §48） |
| **`L3` DWG 识别**（`L3-0..L3-8`） | **100% 已完成** | `#122, #124, #125, #132, #133, #136, #138, #145, #149, #150, #151, #152, #153, #154, #155, #156, #157, #158, #160, #161, #162, #164, #165, #166, #167, #169, #170, #172, #173, #175, #176`（`1bb06191e..dd5a498d8` + `#138..#176`）：90 张 SPPID DXF `15,852/15,852` 业务块 100% 类映射、召回 `97.79%`/精确 `99.89%`、端口吸附 `11,420` 个、跨图连接符批处理配对 + 应用内多标签页跳转命令 `PIDOPC`（`#149`）、五维设计规则体检巡检命令 `PIDDRC`（`#151`）、跨标签页位号与整条管线检索定位命令 `PIDFIND`（`#152`）、跨标签页工艺流路拓扑连通追踪与最短寻径命令 `PIDTRACE`（`#153`）、新旧版次图纸工程语义级差异比对命令 `PIDDIFF`（`#154`）、设备检修能量隔离边界 LOTO 求解命令 `PIDISOLATE`（`#155`）、ISA-5.1 自控仪表回路命令 `PIDLOOP`（`#156`）、管道等级与公称直径一致性对账命令 `PIDSPEC`（`#157`）、多标签页/整批一键数字化交付成套打包门禁命令 `PIDPACKAGE`（`#158`）、自动 HAZOP 审查节点划界与保护层底稿命令 `PIDHAZOP`（`#160`）、公用工程介质分类与装置界区外接接口条件表命令 `PIDUTILITY`（`#161`）、阀门专业材料统计表与规格自动继承命令 `PIDVALVE`（`#162`）、设备管口方位与规格条件表命令 `PIDNOZZLE`（`#164`）、多图纸总目录与标题栏文控校核命令 `PIDINDEX`（`#165`）、管道特殊件与异径大小头双管径清册命令 `PIDSPECIALTY`（`#166`）、管线一览表与跨图 `From-To` 追踪命令 `PIDLINELIST`（`#167`）、核心静/动/换热设备数据一览表与重号校核命令 `PIDEQUIP`（`#169`）、超压泄放保护装置清册与铅封开 `CSO` 合规校核命令 `PIDRELIEF`（`#170`）、SIS/ESD 安全联锁因果矩阵与完整性校核命令 `PIDCAUSE`（`#172`）、自控仪表索引与 DCS/SIS 硬件 I/O 点数核算命令 `PIDIO`（`#173`）、改扩建碰头接点与界区正隔离清册命令 `PIDTIEIN`（`#175`）、绝热与伴热管线厚度拆解及材料统计命令 `PIDINSUL`（`#176`）及 `baseline-90.tsv` 回归门禁；12 张原生 AutoCAD 存量图（`L01..L12`）多模态融合 `1,619` 个精确变体（`fallback = 0`）；以及 `L3-6` `PIDBOM` / `--export-schedule` 同构清册提取（`#150`）与 `PROPS` / `--external-props` `.PPT` 温压自动路由往返闭环（`#145`） |
| **`L4` publish XML 导出**（`L4-0..L4-7`） | **100% 已完成** | `#111, #123, #126, #127, #137, #138, #141`（`6fd58799c`, `b49ed5274`, `1afee04eb`, `477b08b61`, `1b1495c37`, `b152958c1` + `#141`）：`StrictVendor` 校验器、220 份真值零违规、共享确定性 UID 分配、公称直径 `100%` 规范化、2/3/4+ 段线号兼容与管网派生合成、90 图整批转换 `unexplained_diffs = 0`、`_Coverage.json` `VariantResolutionSummary` 可观测性（`90` 图 `exact = 15,910` + `12` 图 `exact = 1,619`，`class_default_fallback = 0`）、`--class-map` 按厂外置替换，以及应用内标准命令 `PIDCONVERT`（单图与 `BATCH`、`CLASSMAP`、`VERIFY`） |
| **`L5` `.pid` 生成**（`L5-1, L5-3..L5-6, L5-8, L5-9`） | **本地 `a/b` 级 100% 已完成** | `#128–#130`（`c851fa214..53ea94d76`）：18 类流写入面分类、10 份空白模板零差异往返、五族记录链与 Q9 索引同步、90 图一键双产物 `42,861/42,861` 跨文件对齐并通过 `a/b` 级验证（`L5-2 / L5-7` 待 `RES-06`） |
| **`L6` 整合与总验收**（`L6-1..L6-3`） | **本地全线已完成** | `examples/pid_convert.rs` 整批双产物 CLI + 应用内 `PIDCONVERT` / `PIDCONVERT BATCH` 命令（`L6-1` / `L4-6`，`#130` / `#138` / `#140` / `#141` / `#142`，覆盖 90 张 SPPID 图 + 12 张原生 AutoCAD 存量图 + 二进制 `AC1032` `.dwg` + 已编辑 `PID_AUTHOR` 模型）、`promote::save_author_document_to_dual` 与 `PIDSAVEPID` 应用内编辑存回 `.pid` 全链路 `a/b` 验证（`L6-2`，`#134` / `#136` / `#139` / `#142` 覆盖 SPPID DXF、12 张原生存量图与真实 `.pid` 原图全闭环）及一站式十一表四审全专业 EPC 数字化移交包命令 `PIDPACKAGE v5` / `--export-package`（`#158` / `#163` / `#168` / `#171` / `#174`，4 图 `48` 文件 `4,528,922` B SHA-256 与 `API 520 + IEC 61511` 双安全红线门禁闭环，见 `readings.md` §47、§52、§57、§60、§63）及本文件 `L6-3` 总验收报告（`#135`，`3ba5c3c2e`；外部回执待回填） |
| **`LX` 外部验收**（`LX-2, LX-4, LX-6..LX-9` 本机齐备） | **纯 ASCII 受控运行器模板、四套送测/真机探针实物包、总审计门禁、全系回执收敛器与合规台账已全部就绪** | `build_submission_round1.py`（`#135`）产出 `submissions/round-01-20261121/`；`build_submission_converted.py`（`#140` / `#143` / `#147`）产出 8 槽 24 文件 `submissions/round-01b-converted-20261121/`；`build_submission_round2.py` + `sppid_runner_template.ps1`（`#144`）产出 `submissions/round-02-bisect-20261121/` 与 `submissions/lx4-sppid-probes-20261121/`；`precheck_and_receipt.py` 支持三关预检、`audit-packages`（`#148`，4 套包共 22 槽 65 文件密码学哈希自洽与零泄漏总审计 `audit_passed = true`）及 `R1..R5` / `B1..B4` / `C1..C8` / `E1..E3` 自动收敛；`06-lx-compliance-and-codelist-audit.md` 已归档 `LX-6/LX-7/#117` |
| **检查点 A（能认得）** | **已达成**（2026-11-20） | 基线报告、`03-parse-answers.md` Q1–Q13 与 L4/L5 选型全部落盘 |
| **检查点 B / C / D** | **本地静态与 `a/b` 级条件已齐备** | 待受控 SPPID 环境补跑 `c1/d` 真机预检及外部平台回填首轮 `receipt.json` 后即可正式签收 |

---

## 3. 第一轮「减法送测」就绪包摘要（`submissions/round-01-20261121/`）

- 生成命令：`python scripts/sppid/build_submission_round1.py --source-dir <EXPORT_SET> --out-dir $OCS_PID_EVIDENCE_DIR/submissions/round-01-20261121 --plant P01 --keep-original-stem`
- 选定源样本摘要：`source_pid_sha256_prefix = 81a99cefae55`（`EXPORT_SET` 内体积最小的三件套齐全已通过真值样本）
- 送测包清单摘要：`manifest_sha256 = 26eb6189e08d506ac4be4828b40c607f2eea6e6c587573bb2398b7bc782e9eb8`，`precheck_passed = true`

| 槽位 | 含 `.pid` | 文件数 | 总字节数 | `StrictVendor` 违规数 | `SchemaOnly` 违规数 | 对照语义差异数 | 本地预检 | 探测目标 |
|---|---|---:|---:|---:|---:|---:|---|---|
| `00-control` | 是 | `3` | `548,863` | `0` | `0` | `0` | **通过** | 零改动原样阳性对照，排除送测通道干扰 |
| `01-var-xml-only` | **否** | `2` | `208,895` | `0` | `0` | `0` | **通过** | 探测外部平台是否实际读取 `.pid` 二进制文件 |
| `02-var-no-topology` | 是 | `3` | `447,840` | `0` | `0` | — | **通过** | 探测外部平台是否强检管线、端口、分支点与连通关系 |
| `03-var-minimal-attrs` | 是 | `3` | `548,283` | `0` | `0` | — | **通过** | 探测除 `UID/Name` 与 `@*` 码表外的必填业务属性子集 |
| `04-var-plain-xml-format` | 是 | `3` | `536,770` | `10`（预期格式项） | `0` | `0` | **通过** | 探测外部平台对 BOM / CRLF / `0-3-6-3-2` 缩进的字节敏感度 |
| `05-var-synthetic-uids` | 是 | `3` | `548,346` | `0` | `0` | — | **通过** | 探测外部平台是否接纳本平台确定性生成的全新 32 位十六进制 UID |

### 3B. 配套八槽「全原型存量图转换、外置表回填与交互重建件」试点就绪包摘要（`submissions/round-01b-converted-20261121/`，#140 / #143 / #147）

- 送测包清单摘要：`manifest_sha256 = 325551c26454b7810fde673ad67639dd0259368caf5c5b3512a5ed3a6fa5a854`，`package_kind = "converted_pilot"`，`precheck_passed = true`（默认纯 ASCII 脱敏主干 `CONV-01..CONV-08`，零客户图名、零盘符路径）

| 槽位 | 交付主干 | 精确变体数 | 表外兜底 | `StrictVendor` 违规数 | `GraphicOID` 对齐 | `a/b` 级验证 | 验证目标 |
|---|---|---:|---:|---:|---:|---|---|
| `01-sppid-dxf-direct` | `CONV-01` | `43` | `0` | `0` | `80 / 80` | **通过 / 通过** | SPPID 导出拆分 DXF → `pid_convert` 双产物 |
| `02-legacy-block-circle-direct` | `CONV-02` | `108` | `0` | `0` | `227 / 227` | **通过 / 通过** | 原生 AutoCAD 图块+散圆存量图 → `pid_convert` 双产物 |
| `03-legacy-exploded-shape-direct` | `CONV-03` | `127` | `0` | `0` | `345 / 345` | **通过 / 通过** | 原生 AutoCAD 全散线形状存量图 → `pid_convert` 双产物 |
| `04-legacy-inapp-pidsavepid` | `CONV-04` | `127` | `0` | `0` | `275 / 275` | **通过 / 通过** | 原生 AutoCAD 存量图 → 应用内 `PIDPROMOTE → PIDREVIEW → PIDSAVEPID` 存回 |
| `05-binary-dwg-direct` | `CONV-05` | `108` | `0` | `0` | `227 / 227` | **通过 / 通过** | 二进制 `AC1032` `.dwg`（`e605ccf6d6f0`）→ `pid_convert` 直转双产物 |
| `06-pid-inapp-edit-save` | `CONV-06` | `299` | `0` | `0` | `442 / 442` | **通过 / 通过** | 既有带 XML `.pid` 原图 → 应用内 `PIDPROMOTE → 面板改位号 + PIDINSERT → PIDSAVEPID` 存回 |
| `07-sppid-dxf-external-props` | `CONV-07` | `115` | `0` | `0` | `302 / 302` | **通过 / 通过** | SPPID 拆分 DXF + 双工作表 `.xlsx` 回填 `153` 项属性与 `59` 个 `.PPT` 工艺点温压 |
| `08-bare-pid-inapp-rebuild` | `CONV-08` | `110` | `0` | `0` | `344 / 344` | **通过 / 通过** | 全厂备份包无伴随 `_Data.xml` 纯裸 `.pid` → 应用内放置边界归并逆向重建双产物 |

### 3C. 第二轮定向二分减法送测包（`submissions/round-02-bisect-20261121/`）与 `LX-4` `E1..E3` 探针包摘要（#144）

- **第二轮二分包**：`manifest_sha256 = 905ad0bd5c0ed0c88c9abb743f9f6a0e71e4a5b7a6c77cd185149760f53dc676`，`package_kind = "round2_bisect"`，`precheck_passed = true`（5 槽 15 文件，`10 / 10` XML `StrictVendor` 零违规）：
  - `00-control`（`R2-CTRL-00`）：零改动阳性对照（`81a99cefae55`）；
  - `01-v2-cfb-repack`（`R2-V2-01`）：流级无损 `PidWriter` 重打包 `.pid`（`74dbb9e6181b`，`339,968` B）；
  - `02-v2-single-vessel-seed`（`R2-V2-02`）：空白模板 `A1-W-New.pid` + 单台容器 `V-1001` 最小双产物（`26077035e5d6`，`57,344` B，`1 / 1` `GraphicOID` 对齐）；
  - `03-v2-piping-no-signal`（`R2-V2-03`）：保留完整工艺管网，单变量剥离仪表信号拓扑（`_Data.xml` `168,535` B）；
  - `04-v2-leaf-codelist-only`（`R2-V2-04`）：保留末级类型码表，单变量置空上级分类父链属性（`_Data.xml` `205,588` B）。
- **`LX-4` SPPID 接受度探针包**：`submissions/lx4-sppid-probes-20261121/`（`manifest_sha256 = 7b6d3146709083361a82c4c2dab595dcb5ab45809f9a7ff2cf2ff809ec4ca013`，`E1-byte-identical` / `E2-metadata-stamp` / `E3-foreign-single-vessel` 共 3 槽 9 文件），已配齐纯 ASCII 受控运行器模板 `scripts/sppid/sppid_runner_template.ps1`。

---

## 4. 固定六门禁全阶段演进核验表

| 门禁项 | 初始基线（#109，`9b5faf294`） | 当前收官节点（#176） | 演进结论 |
|---|---|---|---|
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` | 同一编译器 |
| `cargo test --offline --locked --lib` | `7,145` 条（`7,089` 过 / `55` 忽略 / `1` `.pen`） | **`7,278` 条**（**`7,177` 过** / **`100` 忽略** / **`1` 同一既有 `.pen`**） | 净增 **`+133` 条测试**，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 全阶段全部新增与修改 `.rs` 文件 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `159 passed` | **`202 passed`** | 净增 **`+43` 条测试**，**全绿** |

---

## 5. 待外部条件就绪后的收尾交接清单（2026-11-23 更新）

1. **并行分支关联单均已合入收官**：`L0–L6` 本机功能、EPC 清册命令群（`#149–#176`）以及关联单 `#116`（`.sym` 图形预览，`32ac241dd`）与 `#117`（备份包导出侧码表缺口可观测性，`7a43bf4bf` + `pid-parse @ 7cb03e48a`）均已合入 `main` 并推送至 `pid-cad/main` 关单；阶段一已无任何未合并代码依赖。
2. **待受控 SPPID 环境（`RES-06`）配置 `$OCS_PID_SPPID_RUNNER` 后**：先执行 `submissions/lx4-sppid-probes-20261121/`（`E1..E3` 探针）验证 `c1` 接受度，再对自主导出 `.pid` 执行 `LX-5 / L5-7 / LX-8` 原生打开（`c1`）与发布一致（`d`）真机预检，更新 `_Coverage.json` 与本报告口径 3 状态。
3. **待外部数字化交付平台渠道（`RES-15`）开放后**：投递 `submissions/round-01-20261121/`（6 槽减法包）及配套 `submissions/round-01b-converted-20261121/`（8 槽全原型转换试点包），将返回结果填入 `receipt.json`，运行 `precheck_and_receipt.py evaluate-receipt` 收敛结果；如遇被拒则按 `04-external-platform-protocol.md` 投递预构建好的 `submissions/round-02-bisect-20261121/` 二分定位。
