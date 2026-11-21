# `L6-3`（[#135](https://github.com/happyrust/pid-cad/issues/135)）：智能 PID 平台 · 阶段一总验收报告（本地离线闭环与首轮送测就绪版）

> 日期：`2026-11-21`
> 所属目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`（跟踪总单：[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)）
> 编译器版本：`rustc 1.102.0-nightly (65ce8611f 2026-11-18)`；`pid-parse` 钉定提交：`a85fa10285cc5379953495fa391f374981b9e587`
> 绑定语料清单摘要：`manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038`（`1,115` 文件只读 SHA-256 绑定）
> 当前版本定位：**本地离线闭环全线达成 + 第一轮减法送测包已生成就绪**。外部数字化交付平台最低要求当前未知，本报告严格依据《智能PID平台-阶段一实施方案.md》第 3 节五条验收口径给出实测结论，凡依赖受控 SPPID 环境（`RES-06`）与外部平台回执（`RES-15`）的环节均显式标明「未执行 / 待外部回执回填」，绝不混同「本软件解析器认」与「SPPID / 外部平台认」。

---

## 1. 对照《实施方案》第 3 节五条总验收口径逐条结论

| 口径编号 | 实施方案第 3 节验收要求 | 当前判定状态 | 量化证据与账本索引 |
|---|---|---|---|
| **口径 1：代理标准（与已通过样本一致，差异可解释）** | 导出的 XML 与 `.pid` 在格式、结构、接口与属性上与已通过样本逐项对齐；差异清单必须可解释且受白名单评审守门 | **本地已通过** | 90 张有效真实导出 DXF 一键转换产出 `88,236` 个对象、`96,888` 条关系（`failed = 0`）；`180 / 180` 份 `_Data.xml` / `_Meta.xml` 通过 `#111` `StrictVendor` 零违规校验；`convert_check.py` 对参考 `_Data.xml` 实测 **`unexplained_diffs = 0`**（白名单 `assets/pid-xml-diff-allowlist.json` 逐条附技术依据）；符号召回率 **`97.79%`**（`15,605 / 15,957`）、精确率 **`99.89%`**、图面可见位号一致率 **`~94.6%`**、管口挂设备率（含设备图）**`89.86%`**、标题栏图号提取 **`92.22%`**（`readings.md` §3、§14–§16、§21–§22） |
| **口径 2：日常门禁（`.pid` 的 `a/b` 级与 XML 校验器）** | 每个任务执行 `.pid` `a` 级（解析器重读相等 + 索引无悬挂）、`b` 级（本软件加载显示与参考 PDF 一致）及 XML 格式/结构校验器 | **本地已通过** | ① **生成 `.pid` 双产物侧**：10 份空白模板 `.pid` 流级零差异无损往返（`#128`），五族记录链编码 + Q9 五步索引同步 `chain_fault_count = 0`（`#129`），90 张真实 DXF 全量双产物导出 **`42,861 / 42,861`（`100%`）** `GraphicOID`/`ModelID`/`DrawingID` 跨文件严格相等，`90 / 90` 一次通过 `a` 级（重读 `297,226` 条归一化实体 + 索引零故障）与 `b` 级（CAD 重载 `123,948` 个实体 + `pid-plot` 产出 `90` 份有效 PDF，页内图元 `76,711`）（`readings.md` §17–§19）；② **读取显示 `.pid` 侧**：`EXPORT_SET` 全部 `94 / 94` 张真实 `.pid` 零故障打开与出图，`release` 单图打开中位数 **`536.5 ms`**（`0.733x` 基线）、场景准备均值 **`63.1 ms`**，`L1-4` `27,148` 条文本 `Flattened = 0`；`L1-2` 94 图对 `EXPORT_PDF` 对位 `94 / 94` 通过，覆盖率均值 **`0.995949`**、中位数 **`1.000000`**（`69` 张达 `1.000`、`5` 张 `< 0.980` 全部定位到加长图框模板缺失或副表掩膜边界）（`readings.md` §20） |
| **口径 3：送测前预检三关** | ① XML 与已通过样本逐项对照；② `.pid` 在 SPPID 里能打开（`c1`）；③ SPPID 发布 `.pid` 得到的 XML 与直接导出 XML 一致（`d`） | **第①关已通过；第②③关待 `RES-06` 执行（SPPID 未验证）** | 第①关已在全批 90 图转换产物与第一轮减法送测包上 `100%` 通过；因当前机器未配置受控 SPPID 运行器（`$OCS_PID_SPPID_RUNNER` 未设），第②关（`c1`）与第③关（`d`）按规程标为 **`unverified_no_sppid_runner`（SPPID 未验证）**，待受控环境就绪后执行 `LX-4 / LX-5 / LX-8` |
| **口径 4：成组单变量「减法送测」** | 基于同一已知通过样本构造单变量减法变体组，随附零改动原样阳性对照，一次摸清外部平台硬门槛 | **首轮送测包已生成并通过本地预检；待投递回填 `receipt.json`** | `scripts/sppid/build_submission_round1.py`（`#135`）已在外部证据目录 `submissions/round-01-20261121/` 自动构建 `00-control` + `01-var-xml-only`..`05-var-synthetic-uids` 共 `6` 个槽位（`17` 个文件，清单摘要 `manifest_sha256 = 26eb6189e08d506a...`）及 `receipt.json` 回执模板，`precheck_passed = true`（`readings.md` §23） |
| **口径 5：严格区分「本软件解析器认」与「SPPID / 外部平台认」** | 汇报、覆盖报告与清单中明确区分本地重读/出图通过与原生 SPPID / 外部平台通过 | **严格遵守** | 每份 `_Coverage.json` 的 `pid_verification.sppid_native_c1_status`、`submission-manifest.json` 的 `sppid_c1_d_precheck_status` 及 `readings.md` §17–§23 均显式标注 `unverified_no_sppid_runner`，零越界宣称 |

---

## 2. 七条工作线与检查点达成总览

| 工作线 / 检查点 | 达成状态 | 原子提交范围与核心产物 |
|---|---|---|
| **`L0` 基线与语料**（`L0-1..L0-9`） | **100% 已完成** | `#107–#109, #118–#122`（`9b5faf294..833eb92e9`）：语料 SHA-256 索引、三大度量脚本、90 张有效 DXF 分母核定、`pid-parse` Q1–Q13 源码问答、237 种类变体表 `assets/pid-class-map.json`、外部送测规程 |
| **`L1` 读取与显示**（`L1-1..L1-6`） | **100% 已完成** | `#112–#115, #131`（基线 + `11c8c784b..336b7b362`, `e2f43b91c`）：显示修正五项与 `ACC-1`、新界面导出入口、不在库符号 100% 内嵌缓存核实、`644/618` `.sym` 零失败普查、94 图性能基线与 PDF 扩面对比 |
| **`L2` 内部模型与点选编辑**（`L2-1..L2-7`） | **100% 已完成（`#134` + `#136`）** | `src/io/pid_convert/promote.rs` + `src/app/commands/pidprops.rs`：零冲突完成 `M1..M5` 幂等升格、手工位号保护、`>2048B` XRecord 溢出、`L2-2..L2-4` 属性面板可编辑 `P&ID 属性` + 只读 `P&ID 来源证据` 双分组与 DWG 图面单/双行位号文字联动、`L2-5` 删改清悬挂拓扑、`L2-6` `.pid` 语义升格及 `L2-7` 应用内 `PIDSAVEPID` 存回 `.pid`（真实生产图应用内闭环 `165/165` 对齐过 `a/b` 级验证，见 `readings.md` §24–§25） |
| **`L3` DWG 识别**（`L3-0..L3-8`） | **100% 已完成** | `#122, #124, #125, #132, #133, #136`（`1bb06191e`, `76d734140`, `c2b0c35f2`, `dd5a498d8`）：`15,852/15,852` 业务块 100% 类映射、召回 `97.79%`/精确 `99.89%`、三档置信度守恒与 `PIDREVIEW` 人工确认/驳回交互闭环、管口挂设备 `89.86%`、标题栏图号 `92.22%`、端口吸附 `11,420` 个、跨图连接符配对与 `regression_gate.py` 逐行 diff 门禁 |
| **`L4` publish XML 导出**（`L4-0..L4-7`） | **100% 已完成** | `#111, #123, #126, #127, #137`（`6fd58799c`, `b49ed5274`, `1afee04eb`, `477b08b61`）：`StrictVendor` 校验器、220 份真值零违规、共享确定性 UID 分配、公称直径 `100%` 规范化、管网派生合成、90 图整批转换 `unexplained_diffs = 0`、`_Coverage.json` `VariantResolutionSummary` 可观测性（`exact_variant_id = 15,910`、`class_default_fallback = 0`）与 `--class-map` 按厂外置替换 |
| **`L5` `.pid` 生成**（`L5-1, L5-3..L5-6, L5-8, L5-9`） | **本地 `a/b` 级 100% 已完成** | `#128–#130`（`c851fa214..53ea94d76`）：18 类流写入面分类、10 份空白模板零差异往返、五族记录链与 Q9 索引同步、90 图一键双产物 `42,861/42,861` 跨文件对齐并通过 `a/b` 级验证（`L5-2 / L5-7` 待 `RES-06`） |
| **`L6` 整合与总验收**（`L6-1..L6-3`） | **本地全线已完成** | `examples/pid_convert.rs` 整批双产物命令（`L6-1`）、`promote::save_author_document_to_dual` 与 `PIDSAVEPID` 编辑后存回 `.pid` 全链路 `a/b` 验证（`L6-2`，`#134` / `#136`）及本文件 `L6-3` 总验收报告（`#135`，`3ba5c3c2e`；外部回执待回填） |
| **`LX` 外部验收**（`LX-6..LX-9` 本机齐备） | **三关预检编排 CLI、首轮 6 套减法送测包、回执收敛器与合规台账已全部就绪** | `build_submission_round1.py`（`#135`）已产出 `submissions/round-01-20261121/`；`precheck_and_receipt.py`（`#137`）已支持串联三关预检与 `receipt.json` `R1..R5` 收敛；`06-lx-compliance-and-codelist-audit.md` 已归档 `LX-6` 保留流够用性、`LX-7` 合规边界与 `#117` 分级来源调查 |
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

---

## 4. 固定六门禁全阶段演进核验表

| 门禁项 | 初始基线（#109，`9b5faf294`） | 当前收官节点（#137） | 演进结论 |
|---|---|---|---|
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` | 同一编译器 |
| `cargo test --offline --locked --lib` | `7,145` 条（`7,089` 过 / `55` 忽略 / `1` `.pen`） | **`7,204` 条**（**`7,136` 过** / **`67` 忽略** / **`1` 同一既有 `.pen`**） | 净增 **`+59` 条测试**，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 全阶段全部新增与修改 `.rs` 文件 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `159 passed` | **`196 passed`** | 净增 **`+37` 条测试**，**全绿** |

---

## 5. 待外部条件或并行分支就绪后的收尾交接清单

1. **并行分支关联单衔接**：`L0–L6` 本机功能与交互命令已全部在 `main` 零冲突闭环；关联单 `#116`（`.sym` 图形预览）正由 `pid-symbol-on-main` 工作树推进，待该分支合入后直接复用即可。
2. **待受控 SPPID 环境（`RES-06`）配置 `$OCS_PID_SPPID_RUNNER` 后**：对 `#130` 产出的 `.pid` 执行 `LX-4 / LX-5 / L5-7` 原生打开（`c1`）与发布一致（`d`）真机预检，更新 `_Coverage.json` 与本报告口径 3 状态。
3. **待外部数字化交付平台渠道（`RES-15`）开放后**：直接投递 `submissions/round-01-20261121/` 下的 6 套受控包，将返回结果填入 `receipt.json` 并按 `04-external-platform-protocol.md` 第 4 节二分规则收敛最终交付子集。
