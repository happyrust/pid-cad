# 阶段一读数记录（DWG 转智能 P&ID，导出 `.pid` 与 publish XML）

> 状态：**进行中**。本文件记录阶段一（[happyrust/pid-cad#106](https://github.com/happyrust/pid-cad/issues/106)）各任务的门禁基线与实测读数。
> 所属目录：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/`。
> 约定：原始日志、报告全文与导出产物保存在仓库外证据目录（`$OCS_PID_EVIDENCE_DIR`），本文件只记录可复现的数字、结论、原因与证据文件名，不包含客户数据、盘符、用户名或主机名。

---

## 1. L0-6（#109）：当前 `main` 门禁基线读数与差异说明（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 本仓库提交 | `6377fe2334da303bf390a38aae1469260f870eca`（入库提交 `9b5faf294`） |
| `pid-parse` 钉定提交 | `a85fa10285cc5379953495fa391f374981b9e587`（`Cargo.toml` 6 处 git 依赖） |
| 编译器版本（`rustc -V`） | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 证据目录子项 | `2026-11-20-issue109/`（`git-head.txt`、`rustc-version.txt`、`cargo-test-lib.txt`、`clippy-lib.txt`、`clippy-lib-test.txt`） |

### 1.1 四组核心门禁读数与配套基线

| 门禁项 | 命令 | 实测读数 | 后续任务「不退化」判据 |
|---|---|---|---|
| 编译器版本 | `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` | 每次读数同记版本；若工具链变更须在同一提交先重测基线 |
| 库单元测试 | `cargo test --offline --locked --lib` | 共 **`7,145` 条**：**`7,089` 通过**、**`55` 忽略**、**`1` 失败**（耗时 `55.91s`） | 总条数 `>= 7,145`；失败条数不增加（含未提交设计稿工作区 `<= 1` 且仅限下述字重测试，干净检出为 `0`） |
| 库 `clippy` | `cargo clippy --offline --locked --lib` | `OpenCADStudio (lib)`：**`656` 条警告**（含 `4 duplicates`） | 同一编译器下 `<= 656`（零新增警告） |
| 测试配置 `clippy` | `cargo clippy --offline --locked --lib --profile test` | `OpenCADStudio (lib test)`：**`1,051` 条警告**（含 `4 duplicates`） | 同一编译器下 `<= 1,051`（零新增警告） |
| 分层检查（配套） | `python scripts/arch_check.py report` | `33 module pairs, 183 files`；`io` 起始四行：`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1` | `io` 起始各行数字不增大 |
| `scripts/sppid` 测试（配套） | `python -m pytest scripts/sppid -q -p no:cacheprovider` | **`159 passed`** | 全绿，条数只增不减 |

补充说明：

- **唯一 1 条库测试失败归因**：`ui::style::pid_fonts::tests::the_faces_are_the_weights_the_design_uses`（工作区存在按约定不入库的本地 `docs/pid-platform/design/pid-platform.pen` 修改，设计稿中多出 `("JetBrains Mono", 500)` 一档字重；干净 `HEAD` 检出下该测试通过，全库 `0` 失败）。
- **同工作区子包 `clippy` 固定读数**：两次 `clippy` 中工作区子 crate 的既有警告恒为 `ocs_acis (lib)` `12` 条、`ocs_plugin_api (lib)` `4` 条、`iced_wgpu (lib)` `1` 条。

### 1.2 与备份包计划旧基线（2026-11-18，`64401e1ac` / `c399311e7`）的逐项差异归因

| 指标 | 旧基线（2026-11-18） | 当前基线（2026-11-20） | 变化量 | 逐项原因 |
|---|---:|---:|---:|---|
| 库测试总数 | `6,954` | `7,145` | `+191` | ① 提交 `bee5c33d1`（去掉私有依赖 `e3d-model` / `e3d-io` / `e3d-attlib` 及 `src/io/e3d/{producer,open}.rs`）移除 `9` 条测试；② 备份包读取与显示（`RD-1..14`）、PDF 导出（`PDF-2..7`）、publish 导出（`EXP-1..2`、`aba66b75a`）、自动化通道修复（`f4314f1af`）、默认亮色迁移（`6523d84d9`）及检查点 C 界面与审核修复（`UI-1..9`、`6377fe233`）合计新增 `200` 条测试（`+200 - 9 = +191`） |
| 库测试通过 / 忽略 | `6,901` 过 / `49` 忽略 | `7,089` 过 / `55` 忽略 | `+188` / `+6` | 新增的 `200` 条测试中有 `6` 条为依赖外部真实样本的 `#[ignore]` 测试；另有旧基线中 `3` 条原失败测试转为通过（`6,901 - 9 + 194 + 3 = 7,089`） |
| 库测试失败 | `4` | `1` | `-3` | 旧基线 `4` 条失败均源于工作区未提交的 `pid-platform.pen`；检查点 C（`UI-3..UI-9`）对齐外壳导航与设计变量后消除其中 `3` 条，仅剩 `pid_fonts` 字重核对 `1` 条 |
| `clippy --lib` | `630` | `656` | `+26` | ① 编译器由 `1.101.0-nightly` 升级至 `rustc 1.102.0-nightly (65ce8611f 2026-11-18)`，新 lint 规则使同一代码由 `630` 升至 `660`（`+30`）；② `bee5c33d1` 移除 `e3d` 两个模块减少 `4` 条既有警告（`660 - 4 = 656`）；此后所有提交新增警告为 `0` |
| `clippy --lib --profile test` | `965` | `1,051` | `+86` | ① 同上编译器升级使同一代码由 `965` 升至 `1,058`（`+93`）；② `bee5c33d1` 移除 `e3d` 测试减少 `7` 条既有警告（`1,058 - 7 = 1,051`）；此后所有提交新增警告为 `0` |

---

## 2. L0-2（#107）：语料索引与 SHA-256 校验工具 `corpus_index.py`（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 入库提交 | `538ec0112`（基线 `b87839522`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 新增文件 | `scripts/sppid/corpus_index.py`、`scripts/sppid/corpus-layout.md`、`scripts/sppid/test_corpus_index.py` |
| 证据目录子项 | `2026-11-20-issue107/`（`red.txt`、`green.txt`、`corpus.manifest.json`、`real-index.txt`、`real-verify.txt`、`tamper-verify.txt`、`unset-env.txt`） |

### 2.1 先红后绿与单测门禁

- **先红验证（`red.txt`）**：先落盘 `test_corpus_index.py` 8 条断言测试对最小骨架运行，`8 failed in 0.39s`（全部死在预期 `AssertionError`）。
- **实现后全绿（`green.txt`）**：`python -m pytest scripts/sppid -q -p no:cacheprovider` -> **`167 passed in 1.87s`**（原基线 `159` 条 + 新增 `8` 条全过）。

### 2.2 真实语料清单实测与全部验收项核对

| 验收项 | 实测结果 |
|---|---|
| 真实语料 `index` + `verify` | 通过外置 `--manifest` 只读扫描真实语料根目录（零改动源目录文件），索引 **`1,115` 个文件**（共 `406,863,955` 字节），`manifest_sha256` = `487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038`；紧接执行 `verify` 输出 `verify OK: 1115 file(s) matched`，退出码 `0` |
| 各逻辑段文件统计 | `EXPORT_SET`：`282` 个文件（94 套 `.pid` + `_Data.xml` + `_Meta.xml`，`280,588,766` 字节）；`EXPORT_DXF`：`94` 个文件（`83,816,462` 字节）；`EXPORT_PDF`：`94` 个文件（`8,395,931` 字节）；`SYMBOLS`：`645` 个文件（644 个 `.sym` + 1 个 `CatalogIndex.xml`，`34,062,796` 字节） |
| 单字节篡改检出（`tamper-verify.txt`） | 翻转 1 个字节后 `verify` 退出码 `1`，精确输出 `MISMATCH [EXPORT_SET] export-set/01/sample_Data.xml (expected sha256=..., actual sha256=...)` |
| 未设 `OCS_PID_CORPUS_DIR`（`unset-env.txt`） | 退出码 `0`，输出 `未设置语料目录（OCS_PID_CORPUS_DIR），跳过语料清单操作。` |
| 清单零绝对路径审计 | 扫描 `corpus.manifest.json` 全文：反斜杠 `\` 计数 `0`、盘符冒号计数 `0`、绝对根路径计数 `0`；传入 `../outside` 或带盘符映射时返回退出码 `2` |

---

## 3. L4-0（#111）：publish XML 格式与结构校验器（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `538ec0112`（入库提交 `6fd58799c`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `assets/pid-publish-spec.json`、`src/io/mod.rs`、`src/io/pid_publish_check.rs`、`scripts/sppid/xml_spec_check.py`、`scripts/sppid/test_xml_spec_check.py` |
| 证据目录子项 | `2026-11-20-issue111/`（`red-rust.txt`、`red-py.txt`、`green-rust-fast.txt`、`green-rust-real220.txt`、`green-py.txt`、`real-xml-check.txt`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-lib-test.txt`、`fmt-io-mod.txt`、`fmt-check.txt`、`arch-after.txt`） |

### 3.1 先红后绿与全批真实样本校验

| 检查项 | 实测结果 |
|---|---|
| 零客户数据审计 | `assets/pid-publish-spec.json`（`65,090` 字节）100% 纯 ASCII 模式标识，零非 ASCII 字符、零十六进制 UID 实例，覆盖 `21` 类 Data 对象、`15` 种 Data 关系、`3` 类 Meta 对象与 `3` 种 Meta 关系 |
| Rust 端先红后绿 | 先红 `red-rust.txt`：`1 passed; 9 failed; 1 ignored`（退出码 `101`，全部命中目标断言）→ 实现后 `green-rust-fast.txt`：**`10 passed; 0 failed; 1 ignored`**（退出码 `0`） |
| Python 端先红后绿 | 先红 `red-py.txt`：`3 failed, 1 passed`（退出码 `1`）→ 实现后 `green-py.txt`：`python -m pytest scripts/sppid -q -p no:cacheprovider` **`171 passed`**（原 `167` 条 + 新增 `4` 条全过，退出码 `0`） |
| 全批真实 SPPID XML 样本（`StrictVendor` 默认模式） | 对三批真实 SPPID 导出样本全量 **110 对（`220` 份 `_Data.xml` + `_Meta.xml`，含 `corpus.manifest.json` 中 `EXPORT_SET` 的 94 对 188 份 + 11 对厂商参考数据 22 份 + 5 对验证数据 10 份）**逐份校验：Rust 端 `green-rust-real220.txt` 输出 **`checked 220 XML file(s), violations=0`**，Python CLI `real-xml-check.txt` 输出 **`checked 220 file(s), 0 violation(s)`**，退出码均为 `0` |
| 7 类故意改坏精确报错 | 去掉 BOM（`MISSING_UTF8_BOM`）、改 LF（`LINE_ENDING_NOT_CRLF`）、错 UID 写法（`INVALID_OBJECT_UID`，含小写十六进制与缺端口 `.N`）、错关系前缀（`INVALID_REL_UID_PREFIX`）、关系两端类型不符（`DISALLOWED_REL_ENDPOINTS`）、容器固定属性被改（`CONTAINER_ATTR_MISMATCH`）、首元素非 `PIDDrawing`（`FIRST_ELEMENT_NOT_PID_DRAWING`）均逐条输出 `<文件>:<位置> [<错误码>]: expected <期望>, got <实际>` |
| `SchemaOnly` 与可配 `Plant` | 验证 `CheckMode::SchemaOnly`（及 `--schema-only`）兼容解析库 2 空格 LF 无 BOM 输出，且 `expected_plant` / `--plant` 精确校验工厂名 |

### 3.2 固定门禁读数

| 门禁项 | 改前基线（§1.1） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,145` 条（`7,089` 过 / `55` 忽略 / `1` `.pen` 失败） | **`7,156` 条**（**`7,099` 过** / **`56` 忽略** / **`1` 同一 `.pen` 失败**） | `+11` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/mod.rs` **`new=0`**，`src/io/pid_publish_check.rs` **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `167 passed` | **`171 passed`** | 全绿 |

---

## 4. L0-8（#118）：`pid-parse @ a85fa10` 源码能力核实与 Q1–Q13 决断（2026-11-20）

| 项 | 值 |
|---|---|
| 核实日期 | `2026-11-20` |
| 关联 Issue | [happyrust/pid-cad#118](https://github.com/happyrust/pid-cad/issues/118)（父单 [#106](https://github.com/happyrust/pid-cad/issues/106)） |
| 对照解析库修订 | `pid-parse @ a85fa10285cc5379953495fa391f374981b9e587`（`Cargo.toml` 6 处 git 依赖） |
| 详细核实文档 | `docs/plans/2026-11-20-phase1-dwg-to-smart-pid/03-parse-answers.md` |

### 4.1 关键结论与架构决断摘要

1. **Q1–Q2（`.pid` 写出与流覆盖台账）**：`PidWriter`（`src/writer/mod.rs:4-75`）支持基于 `PidPackage` 的 CFB 容器重打包（保根/子存储 CLSID，字典序重写目录并刷新时间戳）、摘要属性编辑与整流替换（`stream_replacements`），尚无高层新增图形/对象记录 API；`inspect::coverage`（`src/inspect/coverage.rs:10-420`）提供全部顶层流/存储的静态分级 + 动态降级报告。**决断：`L5-3` 按 CFB 流级字节一致验收；`L5-4` 在 `WritePlan::stream_replacements` 之上编码记录链与索引表。**
2. **Q3（两跳连接）**：`0x0089` 属性行（`src/parsers/attribute_rows.rs:1-103`）与 `0x00FA` 依赖组（`src/publish/pid_load.rs:9-57`）已完全解码，53 张全厂图 `9,218 / 9,225` 个符号实例精确命中模型对象，直接解锁 `L0-9`（[#119](https://github.com/happyrust/pid-cad/issues/119)）。
3. **Q4–Q8（publish XML 导出与目录）**：`src/publish/{model,xml_writer,pid_writer,catalog,codelist_values,pipeline,unit_values}.rs` 已提供纯内存 DTO 写出器、21 类接口规则、58 条类型分级链、三级确定性 `UUID v5` 管线合成与 `oleaut32` 15 位有效数字单位换算。**决断：`L4-1` 选定方案 A（统一模型映射至发布 DTO + 外层封装 UTF-8 BOM / CRLF / `0-3-6-3-2` 缩进以通过 `#111` 严格模式门禁）。**
4. **Q9–Q13（索引同步、摘要、流差异、符号缓存与受控差分）**：已列出新增顶层对象的 5 步同步清单（持久 ID `(segment << 13) | index`、记录链与 `record_count`、`0x0081 JSheetLayer` `+12` 对象计数、`/PSMspacemap` 反向入边槽、嵌套 `/JSitesList`）；确认常规生产图不带 `/TaggedTxtData/Drawing` 而将图号存于 `P&IDAttributes.DrawingNo`；确认 `pid_inspect --controlled-diff-dir --json` 可直接用于 `L5-2` / `LX-3` 受控样本差分。

---

## 5. L0-4（#108）：三类度量工具集（XML 比对、识别对真值、显示对比，2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `79aa6ed5b`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增文件 | `scripts/sppid/xml_diff.py`、`scripts/sppid/legend_vs_xml.py`、`scripts/sppid/render_vs_pdf.py`、`scripts/sppid/test_metrics_108.py` |
| 证据目录子项 | `2026-11-20-issue108/`（`red.txt`、`green.txt`、`real-xml-diff.txt`、`real-legend-self.txt`、`real-pdf-self.txt`、`real-vendor11-metrics.txt`） |

### 5.1 先红后绿与统一格式门禁

- **统一输出契约**：三款脚本共用固定 TSV 表头 `kind\ttarget\tlocation\texpected\tactual\tdetail`（一行一个差异）与末尾 `SUMMARY\t...` 汇总行，零外部第三方依赖（纯 Python 标准库）。
- **先红验证（`red.txt`）**：`test_metrics_108.py` 5 组测试对骨架运行，`5 failed in 0.26s`（退出码 `1`，全部命中目标断言）。
- **实现后全绿（`green.txt`）**：`python -m pytest scripts/sppid -q -p no:cacheprovider` -> **`176 passed`**（原 `171` 条 + 新增 `5` 组覆盖自检、三类故意改坏、`PIDPipeline` 按 `Name` 对齐、`--allowlist` 白名单及未设语料目录跳过的测试全过）。

### 5.2 全批真实样本自检与回归核验

| 脚本 | 实测范围 | 输出汇总（`SUMMARY`） | 退出码 |
|---|---|---|---:|
| `xml_diff.py --corpus` | `EXPORT_SET` 全量 `94` 份 `_Data.xml` 自检 | `files=94, diffs=0, allowlisted=0, raw_diffs=0` | `0` |
| `legend_vs_xml.py --corpus` | `EXPORT_SET` 全量 `94` 份 `_Data.xml`（提取 `16,390` 个可标记符号合成理想识别回环） | `files=94, ref_symbols=16390, rec_symbols=16390, matched=16390, recall=1.000000, precision=1.000000, tag_accuracy=1.000000, diffs=0` | `0` |
| `render_vs_pdf.py --corpus` | `EXPORT_PDF` 全量 `94` 份参考 PDF 自检 | `sheets=94, min_coverage=1.000000, max_extra_mm=0.000, text_matched=49997/49997, diffs=0` | `0` |
| `render_vs_pdf.py --metrics-json` | `11` 张厂商参考图 `pid_plot_compare` 验收记录核对 | `sheets=11, min_coverage=0.982943, max_extra_mm=187.091, text_matched=4007/4018, diffs=0` | `0` |

---

## 6. L1-3（#113）：符号缺失清单与内嵌缓存本体核查（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `52fbf5130`（入库提交 `11c8c784b`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/mod.rs`、`src/io/pid_sym_missing.rs` |
| 证据目录子项 | `2026-11-20-issue113/`（`red.txt`、`green-fast.txt`、`real-plant53.txt`、`real-export94.txt`、`plant53-sym-missing.{csv,md}`、`export94-sym-missing.{csv,md}`） |

### 6.1 先红后绿与全批样本扫描结果

- **先红后绿**：`red.txt` 在目标断言变红（`0 passed; 1 failed`，退出码 `101`）→ 实现后 `green-fast.txt` `1 passed; 0 failed`（退出码 `0`）；`fmt-newhunks.ps1` `new=0`，`arch_check.py` 零变化。
- **全批扫描实测（完整 CSV / Markdown 存于外部证据目录，非库内相对路径的主机/用户前缀已脱敏为 `<非标准库路径>\`）**：

| 扫描批次 | 图纸数 | 对比符号库数 | 放置总数 | 去重引用符号数 | 在库种数 | 不在库种数（涉及放置） | 不在库中「有缓存本体」 | 不在库中「无缓存本体」 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 全厂包 53 张 `.pid` vs 同包 `644` 符号库 | `53` | `644` | `9,225` | `142` | `133` | `9`（`549` 次） | **`9` 种（`549 / 549`，100%）** | **`0`** |
| `EXPORT_SET` 94 张 `.pid` vs `644` 子集符号库 | `94` | `644`（子集对照） | `18,373` | `253` | `169` | `84`（`1,319` 次） | **`84` 种（`1,319 / 1,319`，100%）** | **`0`** |

- **结论**：两批共 `147` 张真实 `.pid`（`27,598` 次符号放置）中，**不在外部 `.sym` 目录中的符号 100% 均在 `.pid` 内部自带完整的 `JSite` 内嵌缓存本体（`无缓存本体 = 0`）**，因此本软件打开与渲染显示完全不受缺库影响；仅在交互编辑需新放置该类符号时，从对应工厂备份包 `RefData~4~681.zip` 补齐即可。

---

## 7. L1-6（#114）：`.sym` 符号库全量解码状态普查与三库口径对账（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `11c8c784b`（入库提交 `336b7b362`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 新增 / 修改文件 | `src/io/mod.rs`、`src/io/pid_sym_census.rs`、`scripts/sppid/sym-census.ps1` |
| 证据目录子项 | `2026-11-20-issue114/`（`red.txt`、`green-fast.txt`、`real-census.txt`、`plant644-census.{csv,md}`、`full618-census.{csv,md}`） |

### 7.1 先红后绿与全量普查结果

- **先红后绿**：`red.txt` 在目标断言变红（`0 passed; 1 failed`，退出码 `101`）→ 实现后 `green-fast.txt` `1 passed; 0 failed`（验证非标准 CFB / 空文件分类及连续两次确定性一致）；`fmt-newhunks.ps1` `new=0`。
- **真实符号库全量解码普查（每套连续执行两轮，结果 100% 逐项相同）**：

| 符号库来源标识 | 总 `.sym` 数 | 成功（可画） | 成功（空本体） | 部分成功（未画） | 解码失败 |
|---|---:|---:|---:|---:|---:|
| `plant644`（53 图全厂包 `Ref/Symbols`） | `644` | **`643`** | **`1`** | **`0`** | **`0`** |
| `full618`（解析库参考全集 `symbols-full`） | `618` | **`618`** | **`0`** | **`0`** | **`0`** |

### 7.2 「618 vs 644 vs 728」数量口径差异书面说明

1. **`618`（`full618`）**：`pid-parse` 早期（2026-07）独立归档的参考符号库快照，共 `618` 个 `.sym`。早期快照曾记「578 个有图形、40 个空、0 失败」，原因是当时尚未解码第 3 种文本框形状、B 样条、边界填充与根簇图元合并；升级至 `pid-parse @ a85fa10` 后，`618 / 618` 全部解码出可见图元，**0 失败**。
2. **`644`（`plant644`）**：53 图全厂备份包 `RefData~4~681.zip`（解压即 `Ref/Symbols`）包含 `644` 个 `.sym`（另加 `1` 个 `CatalogIndex.xml` 合计 `645` 个文件），在 `a85fa10` 下 `643` 个解码出可见图元、`1` 个为空模板本体、**0 失败**。
3. **`728`**：94 图备份包 `RefData~4~681.zip` 包含的扩充厂级符号目录（在通用库基础上增加了该厂补充的阀门、压缩机、过滤器等符号，对应 §6 中 94 图相对 `644` 库多引用的那批符号；且该批符号在 94 张 `.pid` 内嵌 `JSite` 缓存中同样 100% 解码可画）。

---

## 8. L0-9（#119）：`.pid` 与 `_Data.xml` 逐对象对应及类变体目录 `assets/pid-class-map.json`（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `336b7b362`（入库提交 `9ac724b12`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `assets/pid-class-map.json`、`src/io/mod.rs`、`src/io/pid_variant_map.rs` |
| 证据目录子项 | `2026-11-20-issue119/`（`red.txt`、`green-fast.txt`、`real-variants.txt`、`variant-report.md`、`gate-cargo-test-lib.txt`、`gate-pixel-rerun.txt`、`gate-clippy-lib.txt`、`gate-clippy-lib-test.txt`） |

### 8.1 先红后绿、零客户数据审计与 94 对样本变体实测

- **先红后绿**：`red.txt` 在目标断言变红（`0 passed; 1 failed`，退出码 `101`）→ 实现后 `green-fast.txt` `1 passed; 0 failed`（验证两跳 `GraphicOID` 连接、派生对象分类及输出 JSON 零客户位号/UNC/UID）。
- **零客户数据审计**：生成的 `assets/pid-class-map.json`（`294,398` 字节）100% 纯 ASCII，零 UNC 前缀（`\\`）、零盘符路径（`:\`）、零客户图名/位号/十六进制实例 UID，仅包含 19 类业务对象的接口集、`@...` 码表值元组与通用符号库相对路径。
- **全批 `94` 对 `(.pid, _Data.xml)` 实测（合计 `106,150` 个业务对象，共 `237` 种类变体，与《样本分析》§2.8 完全一致）**：

| 对象类 | 实例数 | 类变体数 | 直连符号/图元数 | 派生无图形实例数 | 组记录/非根图元 `GraphicOID` 数 |
|---|---:|---:|---:|---:|---:|
| `PIDPipingComponent` | `6,891` | **`78`** | `6,856` | `0` | `35` |
| `PIDPipingConnector` | `8,550` | **`78`** | `0` | `0` | `8,550`（指向 `0x004F` 管段组记录） |
| `PIDInlineInstrument` | `1,927` | **`15`** | `1,924` | `0` | `3` |
| `PIDProcessVessel` | `179` | **`14`** | `178` | `0` | `1` |
| `PIDMechanicalEquipment` | `269` | **`10`** | `266` | `0` | `3` |
| `PIDHeatTransferEquipment` | `54` | **`9`** | `54` | `0` | `0` |
| `PIDPipeline` | `4,118` | **`7`** | `0` | `4,118` | `0` |
| `PIDProcessEquipment` | `426` | **`5`** | `426` | `0` | `0` |
| `PIDInstrument` | `2,974` | **`4`** | `2,970` | `0` | `4` |
| `PIDNozzle` | `797` | **`4`** | `792` | `0` | `5` |
| `PIDPipingPort` | `18,553` | **`3`** | `0` | `18,553` | `0` |
| `PIDBranchPoint` | `885` | **`2`** | `885` | `0` | `0` |
| `PIDControlSystemFunction` | `1,988` | **`2`** | `1,988` | `0` | `0` |
| `PIDNote` | `2,026` | **`1`** | `2,019` | `0` | `7` |
| `PIDPipingBranchPoint` | `5,044` | **`1`** | `5,044` | `0` | `0` |
| `PIDProcessPoint` | `8,550` | **`1`** | `0` | `8,550` | `0` |
| `PIDSignalBranchPoint` | `356` | **`1`** | `356` | `0` | `0` |
| `PIDSignalConnector` | `1,882` | **`1`** | `0` | `0` | `1,882`（指向 `0x004F` 信号线组记录） |
| `PIDSignalPort` | `40,681` | **`1`** | `0` | `40,681` | `0` |

- **未直连根图元的精确归因**：
  1. `derived_model_object_without_drawing_representation`（`71,902` 个）：`PIDPipeline`、`PIDPipingPort`、`PIDSignalPort`、`PIDProcessPoint` 在 SPPID 模型中本身不挂 `DrawingItems` 图形表示，由所属对象或管线段派生；
  2. `graphic_oid_not_in_decoded_sheet_entities`（`10,490` 个）：其中 `10,432` 个为 `PIDPipingConnector`（`8,550`）与 `PIDSignalConnector`（`1,882`），其 `GraphicOID` 指向 `.pid` 的管段/信号线容器组记录（子直线段通过 `parent_ref` 归属该组而非直接共用该 `GraphicOID`）；其余 `58` 个（占符号类对象的 `0.25%`）为挂在图框/嵌套站点或非几何记录上的对象。

### 8.2 固定门禁读数（覆盖 `#113`、`#114`、`#119` 累积改动）

| 门禁项 | 改前读数（§3.2） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,156` 条 | **`7,162` 条**（`+6` 条测试；并发全跑时偶发像素测试单独复跑 `gate-pixel-rerun.txt` `1 passed; 0 failed`，有效失败仍仅 `1` 条既有未提交 `.pen` 字重核对） | **0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/mod.rs`、`pid_sym_missing.rs`、`pid_sym_census.rs`、`pid_variant_map.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `176 passed` | **`176 passed`** | 全绿 |

---

## 9. L0-3（#120）：样本对应关系表与识别分母核定（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `9ac724b12`（入库提交 `d3fc99080`） |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增文件 | `scripts/sppid/corpus_correspondence.py`、`scripts/sppid/test_corpus_correspondence.py` |
| 证据目录子项 | `2026-11-20-issue120/`（`red.txt`、`green.txt`、`real-correspondence.txt`、`correspondence.csv`、`correspondence-summary.md`、`corpus.manifest.json`） |

### 9.1 先红后绿与全批 `94` 套配对核验

- **先红后绿**：`red.txt` 3 条测试全部在目标断言变红（退出码 `1`）→ 实现后 `green.txt`：`python -m pytest scripts/sppid -q -p no:cacheprovider` **`179 passed`**（退出码 `0`）；写回 `correspondence` 段后再次执行 `corpus_index.py verify` 通过（零绝对路径、原 `sections` 哈希不变）。
- **全批真实语料配对读数**：

| 指标 | 实测数量 | 说明 |
|---|---:|---|
| `EXPORT_SET` `.pid` 总数 | `94` | 三件套（`.pid` + `_Data.xml` + `_Meta.xml`）齐全 **`94 / 94`**（缺失 `0`） |
| `EXPORT_DXF` 配对情况 | `94` | 精确同名 **`84`** 张 + 同音/近形字微差候选 **`10`** 张（如 `集/及`、`撬/橇`、`储存/存储`、`聚洁/聚结`，相似度 `0.9286–0.9714`），未配上图纸 `0` 张 |
| `EXPORT_PDF` 配对情况 | `94` | 精确同名 **`84`** 张 + 同上近形字候选 **`10`** 张，未配上图纸 `0` 张，全部非空 |
| **0 字节空 DXF 剔除** | **`4`** | `EXPORT_DXF` 中有 `4` 个文件字节数为 `0`（空文件），自动标记 `exact_empty_0_bytes` 并排除在识别度量分母之外 |
| **有效非空 DXF 识别分母** | **`90`** | 其中精确同名非空 `80` 张、近形字候选非空 `10` 张（合计 `90` 张供 `L0-5` / `L3-0` 实跑识别） |

---

## 10. L0-7（#121）：外部平台验收口径与减法送测规程落盘（2026-11-20）

- **落盘文件**：`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/04-external-platform-protocol.md`
- **核定要点**：
  1. 明确「外部平台最低要求未知，当前以已通过样本严格一致（`#111` `StrictVendor` + `#108` `xml_diff`）为代理标准」；
  2. 逐条登记 `RES-15` 五项待补信息（`EXT-01`–`EXT-05`）及缺省保守策略；
  3. 固化三轮以内「减法送测」单变量变体矩阵（第一轮 `00-control` + `V1-01..V1-05` 一次探测是否读 `.pid`、是否强检管网拓扑、必填业务属性、BOM/缩进字节敏感度及自研 UID 接受度）与 `receipt.json` 标准回执格式（入库提交 `849133af1`）。

---

## 11. L0-5 / L3-0（#122）：全批有效 DXF 识别对真值现状基线报告（检查点 A 达成，2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `849133af1`（`main`） |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 识别入口程序 | `examples/dxf_legend.rs`（默认规则 `assets/pid-legend.json`） |
| 新增文件 | `scripts/sppid/legend_baseline.py`、`scripts/sppid/test_legend_baseline.py` |
| 证据目录子项 | `2026-11-20-issue122/`（`red.txt`、`green.txt`、`rec_cache/`、`run1/baseline-{summary.json,report.md}`、`run2/baseline-{summary.json,report.md}`） |

### 11.1 先红后绿与可复现性验证

- **先红后绿**：`red.txt` 2 组断言测试在骨架上变红（退出码 `1`）→ 实现后 `green.txt`：`python -m pytest scripts/sppid -q -p no:cacheprovider` **`181 passed`**（退出码 `0`）。
- **可复现性**：对 `90` 张有效非空导出 DXF（总 `94` 套剔除 §9 检出的 `4` 份 `0` 字节空 DXF，`0` 读取失败）连续执行 `run1` 与 `run2` 两遍评估，`baseline-summary.json` 逐字段 100% 相等。

### 11.2 90 张有效图纸识别对真值（`_Data.xml`）核心基线读数

| 维度 | 指标 | 实测数值 | 口径与根因说明 |
|---|---|---:|---|
| SPPID 真值分母 | 图上 10 类可标记符号总数 | **`15,957`** | 设备、管件、仪表、控制功能、在线仪表、管口、页间连接符 |
| SPPID 真值分母 | 其中带非空位号符号数 | **`9,536`** | `IObject/@Name` 或分段属性非空 |
| 现有识别器输出 | 检出图块候选总数 | **`17,185`** | 含已知规则块 **`7`** 个、未登记 `unknown` 块 **`17,178`** 个（其中含说明框 `REMARKS*` `759` 个，净业务符号块 `16,426` 个，块级几何检出覆盖已超真值总数的 97%） |
| 现有识别器输出 | 已配对位号数 / 孤立位号数 | **`0` / `197`** | `unknown` 块默认不触发位号配对（`wants_tag=false`），且旧规则位号序号写死 4 位而本批为 7 位 |
| 对真值比对指标 | 类映射后符号召回率 / 精确率 | **`0.000188` / `0.000175`** | `3 / 15,957`（旧 `assets/pid-legend.json` 仅含早期消防水系统的 15 个 `$VALVE$...` 天正块名，与本批标准图块名完全正交） |
| 对真值比对指标 | 匹配项位号准确率 | **`0.000000`** | 同上，`unknown` 块未进入位号配对且位号位数不匹配 |

### 11.3 按影响实例数排序的 Top 缺口与阶段 3/4 突破路径

1. **未召回真值对象类分布（共 `15,954` 个）**：`PIDPipingComponent`（`6,713`）、`PIDInstrument`（`2,881`）、`PIDControlSystemFunction`（`1,947`）、`PIDInlineInstrument`（`1,877`）、`PIDBranchPoint`（`834`）、`PIDNozzle`（`784`）、`PIDProcessEquipment`（`422`）、`PIDMechanicalEquipment`（`268`）、`PIDProcessVessel`（`174`）、`PIDHeatTransferEquipment`（`54`）。
2. **Top 12 未登记 DXF 块名（直接指向 `L3-1` 高收益突破口）**：
   - `DCS FUNC ACCESS IN PRIME LOC`（`1,653` 次）
   - `BALL VALVE TYPE 2`（`1,511` 次）
   - `DISC SINGLE-FUNC FIELD MOUNTED`（`924` 次）
   - `_OFF-UNIT SQ`（`831` 次）
   - `2 WAY GENERIC BODY TYPE 1`（`714` 次）
   - `FLANGE TYPE 2`（`712` 次）
   - `FLANGED NOZZLE WITH BLIND`（`638` 次）
   - `PG-PRESSURE GAUGE`（`610` 次）
   - `GLOBE VALVE TYPE 1`（`487` 次）/ `GLOBE VALVE TYPE 2`（`450` 次）
   - `CONCENTRIC REDUCER`（`389` 次）
   - `BALL VALVE TYPE 1`（`349` 次）/ `PT-PRESSURE TRANSMITTER`（`300` 次）
3. **关键发现与结论（检查点 A「能认得」正式通过）**：
   - 导出 DXF 中的块名去图层后缀后（如 `BALL VALVE TYPE 2`），正是 §8 `assets/pid-class-map.json` 中 `.sym` 文件名的大写主干（`Ball Valve Type 2.sym`）！前 15 个高频块即覆盖超过 `10,000` 个实例；
   - 因此后续 `L3-1`（图例映射）与 `L3-2`（位号增强）只需：① 将块名主干与 `assets/pid-class-map.json` 建立大小写无关自动映射（过滤 `REMARKS*` 注释锚点块），② 将位号序号模式放宽至兼容 4–7 位序号与圈内双行拼接，即可一步打通主体召回与位号配对。
   - 至此，**检查点 A 三项判据（现状基线报告已出、Q1–Q13 源码问答已完、L4/L5 技术选型已定）全部达成**（入库提交 `833eb92e9`）。

---

## 12. L4-1 / L4-2 / L4-3（#123）：自有模型导出 publish XML 底座（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `833eb92e9`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/mod.rs`、`src/io/pid_publish/{mod,uid,model,classes,write}.rs` |
| 证据目录子项 | `2026-11-20-issue123/`（`red.txt`、`green-fast.txt`、`roundtrip.txt`、`roundtrip-real.txt`、`py-spec-check.txt`、`xml-diff-real.tsv`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 12.1 模块设计与先红后绿验证

- **五子模块职责落地**：
  1. `src/io/pid_publish/uid.rs`（`L4-3`）：确定性 `UUID v5` 主对象 UID 与单调不重用 `GraphicOID` 共享分配器（供 `L4` XML 与后续 `L5` `.pid` 共用），端口 `.1/.2`、分支点 `.BPT`、工艺点 `.PPT` 派生，原生端复用 `pid_parse::publish::pipeline::pipeline_uid` 合成管线 UID，按 `assets/pid-publish-spec.json` `uid_prefix` 生成 `<PREFIX>_<UID1>_<UID2>` 关系 UID，以及严格匹配 `AAAD...` 文件标识规则的 `_Meta.xml` 信封 UID 派生；
  2. `src/io/pid_publish/model.rs`（`L4-1`）：脱离 `.pid` 与数据库的自有模型发布输入 DTO（`ExportDrawing`、`ExportItem`、`ExportRelation`、`UnmappedItem`、`ExportBundle`）；
  3. `src/io/pid_publish/classes.rs`（`L4-2`）：联合加载 `assets/pid-publish-spec.json`（21 类接口/必填属性契约）与 `assets/pid-class-map.json`（237 种类变体码表元组），支持按显式 `variant_id`、`.sym` 相对路径或类高频默认变体解析接口顺序（保证 `IObject` 排首）与 `@...` 码表默认值；
  4. `src/io/pid_publish/write.rs`（`L4-1`）：严格厂商字节格式写出器（UTF-8 BOM `EF BB BF`、CRLF、`<?xml version ="1.0" encoding="UTF-8"?>`、固定 `<Container>` 属性保序、`PIDDrawing` 首元素、`0/3/6/3/2` 空格缩进），同时输出 `_Data.xml`、`_Meta.xml` 与 `_Coverage.json`（记录写出统计与未映射跳过清单）；
  5. `src/io/pid_publish/mod.rs`：对外统一入口 `export_drawing_bundle` 与完备单元测试集。
- **先红后绿**：
  - 先红（`red.txt`）：单测落盘后对骨架运行，`1 passed; 2 failed; 0 ignored`（退出码 `101`，全部命中目标断言）；
  - 转绿（`green-fast.txt`）：实现后 `cargo test --offline --locked --lib io::pid_publish::tests` 输出 **`3 passed; 0 failed; 1 ignored`**（退出码 `0`）；未设 `OCS_PID_PUBLISH_SAMPLE_XML` 时忽略测试亦干净打印跳过提示（`roundtrip.txt`）。

### 12.2 合成复杂图纸与真实单图回环导出校验

| 验证场景 | 实测读数 | 结论 |
|---|---|---|
| 合成全链路图纸（含设备、管口、阀门、在线仪表、仪表、管线、管线段、信号线段、端口、分支点、工艺点、注释及 10 类关系 + 未映射项） | Rust `StrictVendor` 校验 `_Data.xml` 与 `_Meta.xml` 均为 **`0` 违规**；`_Coverage.json` 准确列出 `items_written=15` 与 `unmapped_items` | 通过 |
| 真实样本单图回环重导出（`1,881` 个业务对象、`1,981` 条业务关系、`772` 个图形表示，产出 `_Data.xml` `1,822,449` 字节、`_Meta.xml` `1,698` 字节） | Rust `check_publish_xml_bytes`（`StrictVendor`）**`0` 违规**；Python `scripts/sppid/xml_spec_check.py`（严格模式）输出 **`checked 2 file(s), 0 violation(s)`**（退出码 `0`） | 双端严格校验器 **0 违规** |
| 真实单图重导出对参考 `_Data.xml` 的 `xml_diff.py` 逐项比对（`xml-diff-real.tsv`） | 全部 `1,881` 个业务对象 **0 缺失 / 0 多余**；全部 `1,981` 条业务关系 **0 缺失 / 0 多余**；已有属性值 **0 不一致（`DIFF_ATTR = 0`）**；共 `4,820` 行差异 100% 可解释：① `4,632` 行（`772 × 6`）来自无业务键的 `PIDRepresentation` 重新分配确定性 `UUID v5`（`GraphicOID` 100% 保持不变）；② 其余 `188` 行为回环测试未传 `variant_id` 时回退类默认首变体 `V001` 所补齐的可选接口/默认码表项 | 差异清单 100% 结构化可解释 |

### 12.3 固定门禁读数

| 门禁项 | 改前读数（§8.2） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,162` 条 | **`7,166` 条**（**`7,105` 过** / **`60` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+4` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告**（含 `4 duplicates`） | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告**（含 `4 duplicates`） | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/mod.rs` 及 `src/io/pid_publish/*.rs` 共 6 个文件全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `181 passed` | **`181 passed`** | 全绿 |

---

## 13. L3-1（#124）：外置图例到 SPPID 类变体映射规则与解析器（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `b49ed5274`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `assets/pid-sppid-legend-map.json`、`src/io/mod.rs`、`src/io/pid_convert/{mod,class_map}.rs` |
| 证据目录子项 | `2026-11-20-issue124/`（`red.txt`、`green-fast.txt`、`real-90dxf-audit.txt`、`block-audit-90dxf.json`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 13.1 先红后绿与零冲突外置架构

- **零并行分支冲突设计**：新建独立模块 `src/io/pid_convert/` 与纯 ASCII 外置规则 `assets/pid-sppid-legend-map.json`（不改动 `assets/pid-legend.json` 或 `src/io/pid_legend/*`），联合 `assets/pid-class-map.json`（237 种类变体）完成「DXF/DWG 图块与图层 → 忽略过滤 / 归一化主干 → `(sppid_class, variant_id, symbol_path)`」解析。
- **先红后绿**：
  - 先红（`red.txt`）：`0 passed; 2 failed; 1 ignored`（退出码 `101`，全部命中目标断言）；
  - 转绿（`green-fast.txt`）：`2 passed; 0 failed; 1 ignored`（退出码 `0`），验证精确主干命中、`__MIRROREDTEXT` / 前导 `_` / 尾部副本数字归一化、`DCS_` 别名、`DrawingBorder`/`REMARKS*`/`*_DWG` 过滤、显式 `block_overrides` 及未映射图层提示。

### 13.2 全批 `90` 张有效 DXF 业务图块映射覆盖率实测（`block-audit-90dxf.json`）

| 指标 | 实测数值 | 说明 |
|---|---:|---|
| 评估有效非空 DXF 图纸数 | `90` | 与 §9 / §11 识别有效分母完全一致 |
| 模型空间 `INSERT` 总数 | `17,178` | 全批图块放置总数 |
| 过滤非业务块（`ignored_inserts`） | `1,326` | 含 `DrawingBorder`、`HiddenObjects`、`Notes` 图框层与 `REMARKS*` 说明框等 |
| 净业务图块总数（`business_inserts`） | **`15,852`** | 全部进入类变体映射分母 |
| **成功映射至 SPPID 类变体数（`mapped_inserts`）** | **`15,852`** | **映射覆盖率 `100.0000%`（`unmapped_inserts = 0`）** |

按 SPPID 目标类分布：`PIDPipingComponent` `6,678`、`PIDInstrument` `2,877`、`PIDControlSystemFunction` `1,947`、`PIDInlineInstrument` `1,870`、`PIDBranchPoint` `834`、`PIDNozzle` `779`、`PIDProcessEquipment` `341`、`PIDNote` `288`（自动与 10 类可标记符号分离，不再污染符号精确率）、`PIDMechanicalEquipment` `147`、`PIDHeatTransferEquipment` `54`、`PIDProcessVessel` `37`。

### 13.3 固定门禁读数

| 门禁项 | 改前读数（§12.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,166` 条 | **`7,169` 条**（**`7,107` 过** / **`61` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/mod.rs`、`src/io/pid_convert/{mod,class_map}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |

---

## 14. L3-2 / L3-7（#125）：存量图位号识别与配对增强、三档置信度及 90 图复跑度量（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `1bb06191e`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{mod,recognize}.rs` |
| 证据目录子项 | `2026-11-20-issue125/`（`red.txt`、`green-fast.txt`、`arch.txt`、`real-90dxf-cache.txt`、`rec_cache/`、`run1/baseline-{summary.json,report.md}`、`run2/baseline-{summary.json,report.md}`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 14.1 先红后绿与四路位号配对 + 三档置信度实现

- **先红后绿**：
  - 先红（`red.txt`）：3 组合成 fixture 测试落盘对骨架运行，`0 passed; 3 failed`（退出码 `101`，全部命中目标断言）；
  - 转绿（`green-fast.txt`）：`3 passed; 0 failed; 1 ignored`（退出码 `0`），覆盖圈内双行拼接（`PG`+`0201501A` → `PG-0201501A`、`PIA`+`0201301D` → `PIA-0201301D`、`MOV`+`0201101C` → `MOV-0201101C`）、带空格 7 位阀门位号归一化（`BV 0201501B` → `BV0201501B`）及二分图最小距离唯一配对、设备包围盒配对与图层 `10` 松散几何容器恢复（`V-02019`）、页间连接符 `DWG-*` / 走向文本配对、斜杠/波浪号范围展开（`XV-0407A/0407B`）以及高/中/低三档置信度计数守恒。
- **可复现性**：对 `90` 张有效非空 DXF 生成识别缓存后连续执行 `legend_baseline.py` 两轮（`run1` 与 `run2`），产出的 `baseline-summary.json` 字节级 100% 相等（SHA-256 `e83f2aae...`）。

### 14.2 90 张有效 DXF 对 SPPID 真值（`_Data.xml`）复跑对比（#122 基线 vs #125）

| 度量项 | #122 旧基线（§11.2） | #125 新识别器实测 | 提升幅度与说明 |
|---|---:|---:|---|
| 评估有效图纸分母 | `90` | `90` | 同分母（剔除 4 份 0 字节空 DXF，`0` 读取失败） |
| SPPID 真值可标记符号分母 | `15,957`（带位号 `9,536`） | `15,957`（带位号 `9,536`） | 10 类图面符号真值不变 |
| 检出候选符号总数 | `17,185`（`unknown` `17,178`） | **`15,622`**（已知 `15,622`） | `unknown_blocks = 0`、`unmapped_rec_classes = 0` |
| **类映射符号召回率（`recall`）** | `0.000188`（`3 / 15,957`） | **`0.977941`**（**`15,605 / 15,957`**） | **`+97.78` 个百分点**（仅剩 `352` 个松散无块图形设备/管件待深化） |
| **类映射符号精确率（`precision`）** | `0.000175`（`3 / 17,185`） | **`0.998912`**（**`15,605 / 15,622`**） | **`99.89%`** |
| **匹配项位号精确一致率（`tag_accuracy`）** | `0.000000`（`0 / 3`） | **`0.895546`**（**`13,975 / 15,605`**） | 剩余差异中约 `834` 项为 SPPID 库内页间连接符内部序号（图面仅标 `DWG-*`），图面可见位号一致率约 **`94.6%`** |
| 纯位号文本层命中（`matched_on_symbol`） | `0 / 9,536` | **`8,182 / 9,536`**（`0.858012`） | 扣除图面不印出的内部 OPC 数字序号后覆盖率约 **`94.0%`** |
| 三档置信度分布（全量含 `PIDNote`） | — | `high = 12,174`、`medium = 3,736`、`low = 0` | 三档之和严格等于全量识别对象数 `15,910` |

### 14.3 固定门禁读数

| 门禁项 | 改前读数（§13.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,169` 条 | **`7,173` 条**（**`7,110` 过** / **`62` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+4` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_convert/{mod,recognize}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |

---

## 15. L4-4 / L4-5（#126）：管网派生对象合成与组合名称、公称直径换算规则（2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `76d734140`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_publish/{mod,names,relations}.rs` |
| 证据目录子项 | `2026-11-20-issue126/`（`red.txt`、`green-fast.txt`、`arch.txt`、`real-94xml-audit.txt`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 15.1 先红后绿与管网派生合成实现

- **先红后绿**：
  - 先红（`red.txt`）：`3 passed; 4 failed; 1 ignored`（退出码 `101`，全部命中目标断言）；
  - 转绿（`green-fast.txt`）：**`7 passed; 0 failed; 2 ignored`**（退出码 `0`），覆盖：
    1. `names.rs`（`L4-5`）：公制（`25` → `25 mm`）/ 英制（`6"` → 属性 `6 in`、名称内保留 `6"`；`1-1/2"` → `1-1/2 in`）及整数编码 `< 5000` / `>= 5000` 最大公约数约分换算、线号结构化拆解（兼容带保温与尾部悬空 `-`）、以及仪表/控制功能/在线仪表（`INamedInstrument`）和四类设备（`INamedEquipment`）分段属性与 `IObject/@Name` 双向回填；
    2. `relations.rs`（`L4-4`）：管网拓扑自动合成 `PIDPipeline`（确定性 `pipeline_uid`）、`PIDPipingConnector`、`PIDProcessPoint`（`.PPT` + `ProcessPointCollection`）、`PIDPipingPort`（`.1`/`.2` + `PipingPortComposition`）、三通节点 `PIDPipingBranchPoint`（`.BPT`）、管口挂设备 `EquipmentComponentComposition`、以及两端连通关系 `PipingEnd1Conn` / `PipingEnd2Conn`，合成图纸经 `export_drawing_bundle` 导出 100% 通过 `#111` `StrictVendor` 零违规校验。

### 15.2 全批 `94` 份真实 `_Data.xml` 名称与公称直径规则吻合率实测（`real-94xml-audit.txt`）

| 规则类别 | 核对实例数 | 规则精确吻合数 | 吻合率 | 不一致项归因 |
|---|---:|---:|---:|---|
| 公称直径规范化（`NominalDiameter`） | `19,671` | **`19,671`** | **`100.00%`** | 零误差（公制 `N mm` 与英制整数/带分数 `N in` 全部吻合） |
| 仪表/控制功能/在线仪表名称拼接 | `5,184` | **`5,157`** | **`99.48%`** | 其余 `27` 处为数据库历史改造遗留 `ItemTag` 与分段属性不一致（见 `03-parse-answers.md` Q5） |
| 四类设备名称拼接 | `347` | **`344`** | **`99.14%`** | 其余 `3` 处同上为数据库手工覆盖 `ItemTag` |
| 管线完整 4/5 槽位标准名称 | `4,118` | **`3,237`** | **`78.61%`** | 其余 `881` 条（`21.39%`）为数据库中未填齐公称直径或等级槽位的缺省半成品管线名（如 `-PG-1001105-F9`，见 Q5） |

### 15.3 固定门禁读数

| 门禁项 | 改前读数（§14.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,173` 条 | **`7,178` 条**（**`7,114` 过** / **`63` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+5` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_publish/{mod,names,relations}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |

---

## 16. L4-6 / L4-7（#127）：单图与批量 DWG/DXF 转 publish XML 导出入口及对账门禁（阶段 3 最小闭环打通，2026-11-20）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-20` |
| 基线提交 | `1afee04eb`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{mod,export}.rs`、`src/io/pid_publish/{classes,relations}.rs`、`examples/pid_convert.rs`、`assets/pid-xml-diff-allowlist.json`、`scripts/sppid/{convert_check,test_convert_check}.py` |
| 证据目录子项 | `2026-11-20-issue127/`（`red-py.txt`、`green-py.txt`、`red-rust.txt`、`green-fast.txt`、`arch.txt`、`real-90dxf-convert.txt`、`batch-convert-summary.json`、`strict-check-180xml.txt`、`convert-check.txt`、`convert-check-summary.json`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 16.1 双端先红后绿与对账驱动修复

- **Python 端先红后绿**：`red-py.txt` `4 failed`（退出码 `1`）→ 实现 `convert_check.py` 后 `green-py.txt` `python -m pytest scripts/sppid -q -p no:cacheprovider` 输出 **`185 passed`**（退出码 `0`）。
- **Rust 端先红后绿**：`red-rust.txt` `0 passed; 2 failed`（退出码 `101`）→ 实现 `src/io/pid_convert/export.rs`（含 2 度管段自动串联成逻辑 `PIDPipingConnector`、线号沿管段传播、`< min_tier` 跳过登记 `_Coverage.json` 及目录批量入口）与 `examples/pid_convert.rs` 后，`green-fast.txt` **`2 passed; 0 failed`**（退出码 `0`）。
- **对账门禁真实捕获并修复 3 项代码默认值偏差**：首次对全量 90 张真实 DXF 转换产物运行 `convert_check.py` 时，门禁捕获到 `IPIDTypical/@IsTypical`、`IConnector/@RepresentationsAreAllZeroLength` 默认 `"False"` 漏填以及带线号主工艺管段 `IPipingConnector/@PipingConnectorType` 默认应取 `@EE690` 三处偏差；在 `classes.rs` 与 `relations.rs` 修复后直接消除 **`13,341` 条**原始属性差异。

### 16.2 全批 `90` 张有效 DXF 一键转换与严格对账实测

| 验收环节 | 执行命令 | 实测读数 | 退出码 |
|---|---|---|---:|
| 整批转换（`L4-6`） | `cargo run --example pid_convert -- <DXF_DIR> --out <OUT>` | **`converted=90, failed=0, items=77,491, relations=71,415`**（自动跳过 4 份 0 字节空 DXF，产出 `90` 套 `_Data.xml` + `_Meta.xml` + `_Coverage.json` 共 `270` 个文件） | `0` |
| 严格厂商格式与结构门禁（`#111` `StrictVendor`） | `python scripts/sppid/xml_spec_check.py <OUT>` | **`checked 180 file(s), 0 violation(s)`**（`90` 份 `_Data.xml` + `90` 份 `_Meta.xml` 100% 零违规） | `0` |
| 覆盖报告守恒与参考 `_Data.xml` 白名单对账门禁（`L4-7`） | `python scripts/sppid/convert_check.py <OUT> --ref-dir <REF>` | **`sheets=90, spec_violations=0, coverage_errors=0, unexplained_diffs=0, allowlisted_diffs=11845, matched_named=10671/17582`**（白名单之外 **0 未解释差异**；`assets/pid-xml-diff-allowlist.json` 逐条写明数据库专有描述、建设状态、子变体码表等技术原因） | `0` |

### 16.3 固定门禁读数

| 门禁项 | 改前读数（§15.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,178` 条 | **`7,180` 条**（**`7,116` 过** / **`63` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `classes.rs`、`relations.rs`、`pid_convert/{mod,export}.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `181 passed` | **`185 passed`** | `+4` 条全绿 |

---

## 17. L5-1 / L5-3（#128）：CFB 写入面普查与空白模板种子流级往返验证（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `477b08b61`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 新增 / 修改文件 | `src/io/mod.rs`、`src/io/pid_write/{mod,surface,seed,verify}.rs` |
| 证据目录子项 | `2026-11-20-issue128/`（`red.txt`、`green-fast.txt`、`arch.txt`、`real-10seeds-audit.txt`、`seed-audit.json`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 17.1 先红后绿与 CFB 写入面分类台账（`L5-1`）

- **先红后绿**：`red.txt` `0 passed; 3 failed; 1 ignored`（退出码 `101`）→ 实现后 `green-fast.txt` **`3 passed; 0 failed; 1 ignored`**（退出码 `0`）。
- **18 个顶层流/存储写入面完整分类（未知流 `0` 个）**：
  1. `append_records`（`3` 项）：`Sheet*`（几何/文字/符号放置记录链）、`PSMcluster0`（图层与簇记录链）、`Unclustered Dynamic Attributes`（`0x0089` 属性行与 `0x00FA` 依赖组）；
  2. `sync_index`（`1` 项）：`PSMspacemap`（`'tseg'` 段头 `m_iNext` 持久 ID 分配与 `190`/`181`/`182` 反向入边槽）；
  3. `clone_symbol_site`（`2` 项）：`JSite*`（内嵌符号定义缓存存储）与 `JSitesList`（`'OLEM'` 站点 ID 表）；
  4. `rewrite_summary`（`2` 项）：`\x05SummaryInformation` 与 `\x05DocumentSummaryInformation`（保留第 1 节标准属性原字节，重写第 2 节用户字典 `_SPID = DocUID`、`_PlantName`、`_PIDTemplate`）；
  5. `passthrough_preserve`（`10` 项）：`StyleCluster`、`PSMroots`、`PSMclustertable`、`PSMsegmenttable`、`DocVersion2`、`DocVersion3`、`AppObject`、`JTaggedTxtStgList`、`Dynamic Attributes Metadata`、`TaggedTxtData`（含 0 字节 `Revision`）一律从种子原样字节保留。

### 17.2 参考数据 `10` 份真实空白模板 `.pid` 种子选定与无损往返验证（`L5-3`）

- **主选默认空白种子**：`RefData~4~682.zip` 内的通用 A1 横幅空白模板 **`A1-W-New.pid`**（`57,344` 字节，`45` 个 CFB 流，配图框模板 `A1-W - New.igr`；另备 `A0-W-New.pid`、`A2-W-New.pid`、`A3-W-New.pid` 及 `6` 份项目图框空白模板 `28,672–118,784` 字节）。
- **全量 `10 / 10` 份空白种子实测（`seed-audit.json`）**：
  1. 经 `PidWriter::write_to_bytes(&seed, &WritePlan::default())` 原样回写并用 `PidPackage::from_bytes` 重读对比 `diff_packages`：**`10 / 10` 份全部 `lossless_passthrough = true`**（`only_in_source = 0`、`only_in_written = 0`、`modified_streams = 0`、`root_clsid_match = true`、`storage_clsid_diffs = 0`、`state_bits_diffs = 0`）；
  2. 执行 `stamp_seed_drawing_identity` 注入第 2 节 `_SPID`、`_PlantName`、`_PIDTemplate` 后重打包重读：**`10 / 10` 份 `PidParser` 均精确读回注入的 32 位大写十六进制 `_SPID`**，其余非摘要流 100% 无损保留。

### 17.3 固定门禁读数

| 门禁项 | 改前读数（§16.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,180` 条 | **`7,184` 条**（**`7,119` 过** / **`64` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+4` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/mod.rs` 及 `src/io/pid_write/*.rs` 共 5 个文件全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |

---

## 18. L5-4 / L5-5（#129）：`.pid` 五族记录链增补写出器与索引一致性自检（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `c851fa214`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 新增 / 修改文件 | `src/io/pid_write/{mod,oid,records,index}.rs` |
| 证据目录子项 | `2026-11-20-issue129/`（`red.txt`、`green-fast.txt`、`arch.txt`、`real-a1-seed-roundtrip.txt`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 18.1 先红后绿、五族记录编码与 Q9 五步索引同步

- **先红后绿**：`red.txt` `3 passed; 2 failed; 2 ignored`（退出码 `101`）→ 实现后 `green-fast.txt` **`5 passed; 0 failed; 2 ignored`**（退出码 `0`）。
- **五族记录链编码（`src/io/pid_write/records.rs`，`L5-4`）**：
  1. `0x0018 igLine2d`（50B payload，毫米转图纸米坐标，写 `Default` 层 oid）；
  2. `0x0084 igLineString2d`（`24 + vc × 16`B payload，支持任意折线管段与多段线）；
  3. `0x004D igTextBox`（shape 1：UTF-16LE 文本 + 36B 放置与方向单位向量尾块，写 `Labels` 层 oid）；
  4. `0x00CE igSymbol2d`（113B payload：支持从外部 `.sym` 符号库直接克隆完整嵌套缓存存储 `/JSite<id>/*` 或无库时自动生成自包含缓存本体，实现「无缓存缺失 = 0」）；
  5. `0x0089 FreeFormAttrSet`（`P&IDAttributes`：写入与 publish XML 完全一致的 `ModelID = UID`、`DrawingID`、`ModelItemType`、`DrawingNo`、`ItemTag`）。
- **Q9 五步索引同步与一致性自检（`src/io/pid_write/{oid,index}.rs`，`L5-5`）**：
  - 扫描种子全量既有 OID 水位线（防撞号，支持跨 `8,192` 段扩展 `/PSMspacemap/0x00002000`）；
  - 同步 `/Sheet*` 与 `/Unclustered Dynamic Attributes` 链头 `live_record_count`、原地累加 `/PSMcluster0` 中 `0x0081 JSheetLayer` `+12` 对象计数、同步 `/JSitesList`（`'OLEM'`）以及 `/PSMspacemap/*` 的 `next_free_index` 与反向入边槽（`190` / `181` / `182`）；
  - 真实 `A1-W-New.pid` 空白模板 + 真实 `Ball Valve Type 2.sym` 增补实测（`real-a1-seed-roundtrip.txt`）：`chain_fault_count = 0`、`duplicate_oids = []`、`dangling_spacemap_refs = []`、`layer_count_mismatches = []`、`missing_jsites = []`；`build_normalized_geometry` 重读实体由种子 `195` 条增至 `328` 条；故意注入越界引用 `7999` 时自检器精确报错拦截。

### 18.2 固定门禁读数

| 门禁项 | 改前读数（§17.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,184` 条 | **`7,187` 条**（**`7,121` 过** / **`65` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_write/{mod,oid,records,index}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |






