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

---

## 19. L5-6 / L5-8 / L5-9 / L6-1（#130）：DWG/DXF 一键双产物转换（`publish XML + .pid`）及 `a/b` 级往返与出图验证（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `4e115a2f8`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 新增 / 修改文件 | `src/io/pid_publish/uid.rs`、`src/io/pid_convert/{mod,export}.rs`、`examples/pid_convert.rs` |
| 证据目录子项 | `2026-11-20-issue130/`（`red.txt`、`green-fast.txt`、`real5_summary.json`、`real90_summary.json`、`real90_convert_check_ok.txt`、`arch.txt`、`pytest.txt`、`gate-cargo-test-lib.txt`、`gate-clippy-lib.txt`、`gate-clippy-test.txt`） |

### 19.1 先红后绿、跨文件 `UID`/`GraphicOID` 共享机制与三层校验闭环

- **先红后绿**：`red.txt` `2 passed; 2 failed`（退出码 `101`）→ 实现后 `green-fast.txt` **`4 passed; 0 failed`**（退出码 `0`）。
- **单次分配共享 `UID` 与 `GraphicOID`（`L5-8` / `L6-1`）**：
  1. `UidAllocator::reserve_graphic_oids` 预先合并种子 `.pid`（`SeedOidPool::from_seed`）中全部已用 OID 水位线并避开 `8,192` 段首槽位（`(oid & 0x1FFF) == 0`）；
  2. 对图幅自动从 `RefData~4~682.zip` 匹配 `A3/A2/A1/A0-W-New.pid` 空白模板种子与 `/StyleCluster` 线型/字样样式 ID，同步生成 `_Data.xml`、`_Meta.xml`、`.pid` 与 `_Coverage.json`；
  3. 同一图元在 `_Data.xml`（`PIDRepresentation.UID` / `GraphicOID` / `DrawingItems.UID2`）与 `.pid`（`/Sheet*` 图元 `oid` + `/Unclustered Dynamic Attributes` `0x0089 P&IDAttributes` 的 `described_oid` / `DrawingID` / `ModelID` + 第 2 节 `_SPID = DocUID`）中 **100% 严格相等**。

### 19.2 真实导出 DXF（5 图分位抽样 + 90 图有效全集）一键双产物与 `a/b` 级实测（`L5-6` / `L5-9` / `L6-1`）

| 批次 | 转换图数 / 失败 | 写出对象 / 关系 | 跨文件 `GraphicOID` / `ModelID` / `DrawingID` 对齐 | `_SPID == DocUID` | `a` 级索引自检 / 重读归一化实体 | `b` 级本仓重载 CAD 实体 | `b` 级 `pid-plot` PDF 图元 / 页内图元 / 总字节 | `convert_check.py` 对参考 XML | 用时 |
|---|---|---|---:|---:|---|---:|---|---|---:|
| 5 图规模分位抽样（min..max） | **`5 / 0`** | `3,572` / `2,901` | **`2,119 / 2,119`（`100%`）** | `5 / 5` | **`5 / 5` 通过** / `19,220` | `7,551` | `4,536` / `4,363` / `558,588 B` | `spec_violations=0` | `~10.6 s` |
| **90 图有效非空全集（`EXPORT_DXF`）** | **`90 / 0`** | **`77,491` / `71,415`** | **`42,861 / 42,861`（`100%`）** | **`90 / 90`** | **`90 / 90` 通过** / **`297,226`** | **`123,948`** | **`80,190` / `76,711` / `10,094,440 B`** | **`spec_violations=0`、`unexplained_diffs=0`** | **`148.9 s`（`1.65 s/图`）** |

- **关于 `c1`（SPPID 原生环境打开）状态说明**：当前机器未配置受控 SPPID 运行器（`OCS_PID_SPPID_RUNNER` 未设），按规程在每份 `_Coverage.json` 的 `pid_verification.sppid_native_c1_status` 与本账本中明确标注 **`unverified_no_sppid_runner`（SPPID 原生打开 `c1` 未验证）**，绝不冒充已过。

### 19.3 固定门禁读数

| 门禁项 | 改前读数（§18.2） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,187` 条 | **`7,189` 条**（**`7,123` 过** / **`65` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `uid.rs`、`pid_convert/{mod,export}.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `185 passed` | **`185 passed`** | **全绿** |

---

## 20. L1-2 / L1-4 / L1-5（#131）：94 张真实 `.pid` 批量打开性能、多样式字符 run 混排核查与渲染出图扩面对比（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `53ea94d76`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增文件 | `examples/pid_l1_bench.rs`、`scripts/sppid/batch_l1_eval.py`、`scripts/sppid/test_batch_l1_eval.py` |
| 证据目录子项 | `2026-11-20-issue131/`（`red.txt`、`green.txt`、`bench.json`、`bench_dev_opt0.json`、`pid_batch_report.csv`、`compare_runs.json`、`compares/`、`summary.{json,md}`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 20.1 先红后绿与评测方法说明

- **先红后绿**：`red.txt` 对骨架运行 `test_batch_l1_eval.py`（`3 failed`，退出码 `1`）→ 实现 `scripts/sppid/batch_l1_eval.py` 并补充加长图幅/符号归因单测后 `green.txt` **`4 passed`**（`pytest scripts/sppid` 全集 **`189 passed`**，退出码 `0`）。
- **评测口径**：
  1. `examples/pid_l1_bench.rs` 在单热进程中按 `examples/open_bench.rs` 相同生命周期先测量一次性字体预热（`lff + sysfont + cosmic-text`），再对 `EXPORT_SET` 全部 `94` 张真实 `.pid` 逐张独立计时 `io::load_file`（`cad_import_ms`）与编辑器场景准备（`scene_prepare_ms`：`repair_document + resolve_xrefs + build_derived_caches + prepare_open_geometry`），同步统计 `pid_parse::style_link::text_styles_for_document` 的 `TextRunStatus`（`L1-4`）并调用 `io::pid_pdf::plot_pdf` 渲染 PDF；
  2. 独立运行 `examples/pid_batch_report.rs` 交叉核验全部 `94` 张图纸导入行数与状态（`94 imported, 0 failed`）；
  3. 按 §9（`#120`）`correspondence.drawings`（`84` 张精确同名 + `10` 张近似改名候选），使用 `pid_plot_compare` 对 `94` 对渲染 PDF 与 `EXPORT_PDF` 参考图逐张测量覆盖率、多画长度、文字匹配与页面对位；所有落盘汇总均仅以 `S01..S94` 与 12 位 `pid_sha256_prefix` 标识图纸，零客户图名入库。

### 20.2 `L1-5` 94 张真实 `.pid` 批量打开与出图性能基线（`release` vs `dev`）

| 构建配置 | 图数 / 失败 | 预热 `warmup_ms` | CAD 导入总计 / 均值 / 中位数 / P95 / 最大 | 场景准备总计 / 均值 / P95 | 完整打开总计 / 均值 / 中位数 / P95 / 最大 | `plot_pdf` 出图总计 / 均值 / 中位数 / P95 | CAD 实体 / 内嵌缓存本体 / PDF 图元（页内） |
|---|---|---:|---|---|---|---|---|
| **`release`（`opt-level=3`）** | **`94 / 0`** | **`107.7 ms`** | **`153.25 s` / `1,630.3 ms` / `509.9 ms` / `7,608.7 ms` / `10,134.2 ms`** | **`5.93 s` / `63.1 ms` / `220.2 ms`** | **`159.18 s` / `1,693.4 ms` / `536.5 ms` / `7,696.0 ms` / `10,196.7 ms`** | **`92.23 s` / `981.2 ms` / `567.7 ms` / `3,371.9 ms`** | **`275,542` / `16,826` / `189,978`（`95,102`）** |
| `dev`（本仓 `opt-level=0`） | `94 / 0` | `291.0 ms` | `206.74 s` / `2,199.4 ms` / `795.9 ms` / `10,162.6 ms` / `12,972.6 ms` | `52.44 s` / `557.9 ms` / `2,101.7 ms` | `259.18 s` / `2,757.2 ms` / `1,108.2 ms` / `11,239.8 ms` / `13,175.0 ms` | `114.02 s` / `1,213.0 ms` / `783.7 ms` / `3,998.7 ms` | 同上（导出 PDF 逐项一致） |
| `pid_batch_report` 核验 | `94 / 0` | — | `180.48 s` / `1,920.0 ms` / `684.0 ms` | — | — | — | `drawn = 275,459`，`library_bodies = 0` |

- **对比 `732.1 ms/图` 基线（53 图全厂包 `38.8 s`）与热点定位**：
  1. **常规生产流程图打开速度优于基线**：`release` 下 94 张生产图完整打开**中位数为 `536.5 ms/图`**（纯导入中位数 `509.9 ms/图`），为 `732.1 ms` 基线的 **`0.733x`**（远低于 `1.5x = 1,098.2 ms` 上限）；本仓自身场景准备管线（`purge + xref + caches + prepare_open_geometry`）单图均值仅 **`63.1 ms`**（全批 `275,542` 个实体总计仅 `5.93 s`，占总打开时间 `3.7%`）。
  2. **长尾拉高算术均值的双层根因**：① 样本规模偏态——53 图全厂包中含 17 张小型测试/空壳图（总内嵌缓存 `8,757` 个，单张大型多回路图当时亦需 `4,100 ms`），而 `EXPORT_SET` 94 图全为生产交付图（总内嵌缓存 `16,826` 个，为前者的 `1.92x`），含 14 张高密度多模块图（如最慢图 `S05` `1a78323d8818` 含 `10,396` 个实体、`606` 个 `/JSite*` 子存储）；② 外部解析热点——`96.3%`（`153.25 s / 159.18 s`）的耗时位于上游依赖 `pid-parse @ a85fa10` 解析 CFB 内百量级 `/JSite*` 嵌套子存储目录项，本仓 `src/io/pid/` 与 `src/scene/` 侧无性能瓶颈。
  3. **`dev` 与 `release` 差异说明**：`Cargo.toml` 设 `[profile.dev.package] OpenCADStudio = { opt-level = 0 }` 以加速日常增量编译，导致 `dev` 下场景准备慢约 `8.8x`（`557.9 ms` vs `63.1 ms`）；评测性能一律以优化构建为准。

### 20.3 `L1-4` 多样式字符 run 混排核查结果

| 统计范围 | 文本记录总数 | `Uniform`（单样式 run） | `NoRuns`（段落默认） | `Flattened`（多样式展平） | `Unresolvable` / `LengthMismatch` | 含 `Flattened` 图纸数 |
|---|---:|---:|---:|---:|---:|---:|
| `EXPORT_SET` 全部 94 张真实 `.pid` | **`27,148`** | **`26,968`**（`99.34%`） | **`180`**（`0.66%`） | **`0`（`0.000%`）** | **`0` / `0`** | **`0 / 94`** |

- **结论**：全批 `94` 张真实生产 `.pid` 的 `27,148` 条文本记录中，不存在同一条 `TEXT` 内混合多种字符样式被展平的情形（`Flattened = 0`），结合单样式 run 早已在 `src/io/pid/styles.rs:101-136` 接通，无需额外做拆段或转 `MTEXT`。

### 20.4 `L1-2` 94 张渲染 PDF 对 `EXPORT_PDF` 参考图扩面对比与异常归因

| 子集分组 | 图纸数 | 对位通过（`within_threshold`） | 覆盖率均值 / 中位数 / P05 / 最低 | 多画长度均值 / 中位数 / P95 / 最大 (mm) | 覆盖率分桶（`=1.000` / `[0.980, 1.000)` / `<0.980`） | 文字对上 / 参考总数（比例） |
|---|---:|---:|---|---|---|---:|
| **全量 `EXPORT_SET` vs `EXPORT_PDF`** | **`94`** | **`94 / 94`（`100%`）** | **`0.995949` / `1.000000` / `0.980671` / `0.911705`** | **`5.141` / `0.000` / `21.907` / `122.764`** | **`69` / `20` / `5`**（其中 `>=0.995` 共 **`81`** 张） | **`27,062 / 27,300`（`99.13%`）** |
| 其中精确同名配对（`exact`） | `84` | `84 / 84` | `0.995566` / `0.999997` / — / `0.911705` | `5.753` / `0.000` / — / `122.764` | `62` / `17` / `5` | — |
| 其中近似改名候选（`fuzzy`） | `10` | `10 / 10` | **`0.999174` / `1.000000`** / — / **`0.995869`** | **`0.000`** / `0.000` / — / `0.000` | `7` / `3` / **`0`** | — |

- **额外验证结论**：§9（`#120`）登记的 `10` 张近似改名候选图在几何覆盖率上最低达 `0.995869`、中位数 `1.000000`、多画长度全为 `0.000 mm`，从图面几何上 100% 证实这 10 对近形字文件确系同版同图配套导出的参考 PDF。
- **`5` 张覆盖率 `< 0.980` 图纸逐张脱敏归因（全批无多画长度 `> 200 mm` 或对位失败项）**：

| 图纸编号 | `.pid` SHA-256 前缀 | 图幅 (mm) | 覆盖率 | 多画长度 (mm) | 文字匹配 | 归因层级 | 实测技术根因 |
|---|---|---|---:|---:|---|---|---|
| `S14` | `37d05c085f7e` | `1800×841` | `0.935594` | `80.581` | `782 / 818` | 参考数据 / 图框模板 | 加长 `A0+`（`1800×841 mm`）图幅：参考归档仅含标准 `A0..A3` `.igr` 模板，缺加长图框导致 `3,455.5 mm` 图框/外框线与右侧标题栏 `36` 条文字未画出且未被标准标题栏掩膜扣除；图面自身笔划配对 `9,617 / 9,642`（`99.74%`）一致 |
| `S30` | `73f4e5a08de9` | `1800×841` | `0.911705` | `0.000` | `202 / 238` | 参考数据 / 图框模板 + 嵌套 OLE | 同上缺 `1800×841 mm` 加长图框（`3,624.2 mm` 图框线 + `36` 条标题栏文字未掩膜），另有 `2` 条 `0x0013` 嵌套 OLE 框（`539.6 mm`）；图面已画笔划 `5,861 / 5,861`（**`100.00%`**）一致、`extra = 0.0 mm` |
| `S32` | `45521ea0760b` | `1800×841` | `0.942229` | `17.034` | `539 / 575` | 参考数据 / 图框模板 | 同上缺 `1800×841 mm` 加长图框（`3,624.2 mm` 图框线 + `36` 条标题栏文字未掩膜）；图面自身笔划 `8,887 / 8,889`（**`99.98%`**）一致 |
| `S74` | `d5412bd8ddfb` | `841×594` | `0.968969` | `0.000` | `144 / 150` | 标题栏副表掩膜 / 子集外符号 | 标准 `A1` 图幅左侧签署副表线（`/Sheet6` `252.7 mm` + `6` 个 2–3 字符短标注，超出标准标题栏掩膜左边界）及 `644` 子集库外 `Set Fire Point.sym`（`58.4 mm`）局部笔划差；`extra = 0.0 mm` |
| `S75` | `48338c91f1ec` | `841×594` | `0.968991` | `0.000` | `144 / 150` | 标题栏副表掩膜 / 子集外符号 | 与同系列姊妹图 `S74` 完全相同（左侧签署副表线 `252.7 mm` + `Set Fire Point.sym` `58.4 mm`，`extra = 0.0 mm`） |

### 20.5 固定门禁读数

| 门禁项 | 改前读数（§19.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,189` 条 | **`7,189` 条**（**`7,123` 过** / **`65` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | **0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告**（`--example pid_l1_bench` 亦为 **0 新增警告**） | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `examples/pid_l1_bench.rs` **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `185 passed` | **`189 passed`** | `+4` 条测试，**全绿** |

---

## 21. L3-5 / L3-6（#132）：设备与管口组合配对、规格继承及标题栏/线号属性提取核对（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `e2f43b91c`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `src/io/pid_convert/{mod,recognize,export}.rs`、`src/io/pid_publish/relations.rs` |
| 证据目录子项 | `2026-11-20-issue132/`（`red.txt`、`green-fast.txt`、`gen90_summary.json`、`convert_check_90.json`、`full90_audit.json`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 21.1 真值规格核实与先红后绿

- **94 份真值 `_Data.xml` 规格核实**：
  1. 全批共 `797` 个 `PIDNozzle`，**`797 / 797`（`100.00%`）** 的 `IObject/@Name` 均保持空串、全部通过 `EquipmentComponentComposition`（`UID1 = PIDNozzle, UID2 = Equipment`）挂接所属设备，其中 `514` 个带 `IPipeCrossSectionItem/@NominalDiameter`、`483` 个带 `IPipingSpecifiedItem/@PipingMaterialsClass`；
  2. `PIDDrawing` 的 `IObject/@Name` 为图纸中文标题，`IDocument/@DocTitle` 保持空串。
- **先红后绿**：`red.txt` 2 条目标断言测试在骨架上变红（`0 passed; 2 failed`，退出码 `101`）→ 实现图层 `"10"` 松散容器壳体几何泛洪重构、双层容差管口挂接、管口 `DN`/相连管段规格回填及右下角标题栏 `TitleBlockInfo` 提取后，`green-fast.txt` **`18 passed; 0 failed`**（退出码 `0`）。

### 21.2 90 张有效非空真实 DXF 全批对账结果（`L3-5 / L3-6`）

| 维度 | 指标 | 改前基线（#130） | 改后实测（#132） | 对账说明 |
|---|---|---:|---:|---|
| `L3-5` 设备与管口组合 | 检出并写出 `PIDNozzle` 数 | `779` | **`779`**（真值 `784`） | 保持召回不退化 |
| `L3-5` 设备与管口组合 | `EquipmentComponentComposition` 关系数 | `379`（`48.65%`） | **`629`（`80.74%`）** | 净增 `+250` 条；在含设备符号的图纸上挂接率达 **`629 / 700`（`89.86%`）**，另有 `79` 个位于图面无独立容器符号的纯管汇图 |
| `L3-5` 管口规格回填 | 带 `NominalDiameter` 的 `PIDNozzle` 数 | `0` | **`212`** | 来自管口旁 `DN`/寸制标注与相连已标线号 `PIDPipingConnector` |
| `L3-6` 标题栏提取 | 提取标题栏图号（`drawing_number`）图数 | `0 / 90` | **`82 / 90`（`91.11%`）** | 提取右下角标题栏图号编码并写入 `_Coverage.json` |
| `L3-6` 标题栏提取 | 提取标题栏图名并与真值 `PIDDrawing/@Name` 吻合 | `0` | **`48 / 49`（精确率 `97.96%`）** | 支持单行图名及跨行装置名+流程图名拼接；其余图纸标题栏字段存于未导出块属性内 |
| `L3-6` 线号属性拆解 | 按序号对齐 `5,678` 条 `PIDPipingConnector` 属性吻合率 | — | 等级 **`90.56%`** / 保温 **`87.76%`** / 公称直径 **`71.24%`** | 支管相对主管变径处若图面未重复标支管线号则产生直径差异（已在白名单说明） |
| 整批严格对账门禁 | `convert_check.py`（`180` 份 XML + `90` 份覆盖报告） | `0` 违规 / `0` 未解释差异 | **`spec_violations = 0`、`coverage_errors = 0`、`unexplained_diffs = 0`** | 总关系数由 `71,415` 增至 **`71,665`** |

### 21.3 固定门禁读数

| 门禁项 | 改前读数（§20.5） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,189` 条 | **`7,191` 条**（**`7,125` 过** / **`65` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_convert/{mod,recognize,export}.rs`、`relations.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `189 passed` | **`189 passed`** | **全绿** |

---

## 22. L3-3 / L3-4 / L3-8（#133）：管线族成链、端口连通与跨图页间连接符配对深化及整批回归门禁（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `c2b0c35f2`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{mod,recognize,export}.rs`、`scripts/sppid/{regression_gate,test_regression_gate}.py` |
| 证据目录子项 | `2026-11-20-issue133/`（`red-py.txt`、`green-py.txt`、`red-rs.txt`、`green-rs.txt`、`gen90_summary.json`、`convert_check_90.json`、`baseline-90.tsv`、`audit_133_summary.json`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 22.1 先红后绿与机制深化说明

- **Python 回归门禁先红后绿（`L3-8`）**：`red-py.txt` `3 failed`（退出码 `1`）→ 实现 `scripts/sppid/regression_gate.py` 后 `green-py.txt` **`3 passed`**（全集 `pytest` **`192 passed`**）。
- **Rust 管网与连通深化先红后绿（`L3-3 / L3-4`）**：`red-rs.txt` `0 passed; 2 failed`（退出码 `101`）→ 实现四向边缘中点端口、共线微断口桥接、微短刺/箭头碎片过滤、无歧义交点线号传播与跨图页间连接符配对后，`green-rs.txt` **`20 passed; 0 failed`**（退出码 `0`）。

### 22.2 90 张有效非空真实 DXF 全批管网、端口连通与跨图连接符实测（`L3-3 / L3-4 / L3-8`）

| 维度 | 指标 | 改前读数（#132） | 改后实测（#133） | 技术说明 |
|---|---|---:|---:|---|
| 整批写出总规模 | 写出对象总数 / 关系总数 | `77,491` / `71,665` | **`88,236` / `96,888`** | 净增 `+10,745` 个端口对象与 `+25,223` 条连通/归属关系 |
| `L3-4` 符号端口吸附 | 合成 `PIDPipingPort` 数 | `626` | **`11,420`（`18.2x`）** | 导出 DXF 块定义不含显式 `POINT` 图元，补齐包围盒四向边缘中点后精确吸附左右/上下接管端 |
| `L3-4` 管端连通关系 | `PipingEnd1Conn + PipingEnd2Conn` | `37,221` | **`48,243`** | 净增 `+11,022` 条管端到符号端口/连接符连通关系 |
| `L3-3` 三通与成链清理 | `PIDPipingBranchPoint` / 共线桥接 / 过滤短刺 | — / `0` / `0` | **`4,907` / `124` / `744`** | 三通分支点与真值（`5,044`）高度吻合；过滤 `744` 条流向箭头与短标引杂散线段 |
| `L3-3` 线号传播与对账 | 已标线号管段 / 对参考 XML 匹配命名对象 | — / `10,671` | **`9,526` / `10,708`** | 同层无歧义交点线号传播净增 `+37` 个命名对象精确对齐 |
| `L3-3` 管端断头守恒归因 | 全部 `27,070` 条管段共 `54,140` 个端点 | — | **`54,140 / 54,140`（`100%` 守恒）** | 接符号端口 **`13,184`**、接页间连接符 **`712`**（占 `834` 个连接符的 `85.4%`）、接三通 **`34,347`**、触设备壳体 **`809`**、开放排凝/放空自由端 **`5,088`** |
| `L3-4` 跨图页间连接符配对 | 总 `PIDBranchPoint` `834` 个（带图号 `617` 个） | `0` 对 | **本批互逆成对 `38` 对（`76` 个）** | 另含本批内单向引用 **`99`** 个、指向批外关联装置图 **`442`** 个、无图号公用工程/放空连接符 `217` 个；标题栏图号提取同步升至 **`83 / 90`（`92.22%`）** |
| `L3-8` 整批逐行回归门禁 | `regression_gate.py` + `convert_check.py` | — | **`baseline-90.tsv` 落盘，自比 `0` / 旧版异动拦截 `1`** | `180` 份 XML **`spec_violations = 0`、`coverage_errors = 0`、`unexplained_diffs = 0`** |

### 22.3 固定门禁读数

| 门禁项 | 改前读数（§21.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,191` 条 | **`7,193` 条**（**`7,127` 过** / **`65` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_convert/{mod,recognize,export}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `189 passed` | **`192 passed`** | `+3` 条测试，**全绿** |

---

## 23. LX-8 / LX-9 / L6-3（#135）：第一轮「减法送测」6 套受控包自动构建预检与阶段一总验收报告（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `dd5a498d8`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `scripts/sppid/{build_submission_round1,test_build_submission_round1}.py`、`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/05-phase1-acceptance-report.md` |
| 证据目录子项 | `2026-11-20-issue135/`（`red.txt`、`green.txt`、`round1_sanitized.json`、`arch.txt`、`pytest.txt`）及 `submissions/round-01-20261121/` |

### 23.1 先红后绿与第一轮 6 槽减法送测实物包预检读数

- **先红后绿**：`red.txt` 对骨架运行 `test_build_submission_round1.py`（`1 failed`，退出码 `1`）→ 实现 `scripts/sppid/build_submission_round1.py` 后 `green.txt` **`1 passed`**（`pytest scripts/sppid` 全集 **`193 passed`**，退出码 `0`）。
- **真实语料首轮受控送测包构建（`submissions/round-01-20261121/`）**：
  - 自动从 `EXPORT_SET` 94 套已通过真值样本中选取体积最小的三件套齐全样本（`source_pid_sha256_prefix = 81a99cefae55`），产出 6 个单变量受控槽位、`submission-manifest.json`（`manifest_sha256 = 26eb6189e08d506ac4be4828b40c607f2eea6e6c587573bb2398b7bc782e9eb8`）与 `receipt.json` 标准回填模板，**`precheck_passed = true`**：

| 槽位 | 含 `.pid` | 文件数 | 总字节数 | `StrictVendor` 违规数 | `SchemaOnly` 违规数 | 对照语义差异数 | 本地预检 |
|---|---|---:|---:|---:|---:|---:|---|
| `00-control` | 是 | `3` | `548,863` | **`0`** | **`0`** | **`0`** | **通过** |
| `01-var-xml-only`（`V1-01`） | **否** | `2` | `208,895` | **`0`** | **`0`** | **`0`** | **通过** |
| `02-var-no-topology`（`V1-02`） | 是 | `3` | `447,840` | **`0`** | **`0`** | — | **通过** |
| `03-var-minimal-attrs`（`V1-03`） | 是 | `3` | `548,283` | **`0`** | **`0`** | — | **通过** |
| `04-var-plain-xml-format`（`V1-04`） | 是 | `3` | `536,770` | **`10`**（预期格式探针） | **`0`** | **`0`** | **通过** |
| `05-var-synthetic-uids`（`V1-05`） | 是 | `3` | `548,346` | **`0`** | **`0`** | — | **通过** |

- **阶段一总验收报告落盘（`L6-3`）**：`05-phase1-acceptance-report.md` 已对照《实施方案》第 3 节全部五条总口径汇总全阶段 24 个原子提交与 §1–§23 读数，明确标注 `sppid_c1_d_precheck_status = unverified_no_sppid_runner` 及待 `RES-15` 回填的 `receipt.json` 清单。

### 23.2 门禁读数（本单仅新增纯 Python 标准库脚本与 Markdown 文档，未触碰 `.rs` 代码）

| 门禁项 | 改前读数（§22.3） | 改后实测 | 结论 |
|---|---|---|---|
| Rust 库与 `clippy`（未改 `.rs` 源码） | `7,193` 条测试，`clippy` `656` / `1,051` | 同 `dd5a498d8`（未动 `src/` 与 `examples/`） | **0 变化** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `192 passed` | **`193 passed`** | `+1` 条测试，**全绿** |

---

## 24. L2-1 M1..M5 / L2-5 / L2-6 / L2-7 / L6-2（#134）：`PID_AUTHOR` 统一模型无冲突升格引擎与编辑后存回 `.pid` 全链路闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `3ba5c3c2e`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{mod,recognize,export,promote}.rs`（零改动 `src/pid/*`、`src/io/pid/*`、`src/app/*`） |
| 证据目录子项 | `2026-11-20-issue134/`（`red.txt`、`green-fast.txt`、`real134_test.log`、`real_out/real134_summary.json`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 24.1 先红后绿与契约核验

- **先红后绿**：`red.txt` 4 条核心闭环断言在骨架上全部变红（`0 passed; 4 failed`，退出码 `101`）→ 实现 `src/io/pid_convert/promote.rs` 后 `green-fast.txt` **`24 passed; 0 failed`**（退出码 `0`）：
  1. **`M1 + M2` 幂等升格与隔离契约**：重复调用 `promote_sheet_to_author` 实测 `newly_promoted = 0`、`updated_in_place = N`；`tag_source = "manual"` 手工位号与自定义扩展属性 100% 保留不被覆盖；`props` 超 `2,048` 字节自动溢出至模型空间扩展字典分片 `XRecord`（`props_ref=`）；`io::pid_legend::purge` 仅清 `PID_SEMANTICS` 中 `legend:` 记录，`PID_AUTHOR` 毫发无损；
  2. **`M3 + L2-5` 拓扑写入与增删移同步**：管线、管路、管段（带 `SegmentEnd::joined` 端口绑定）、管件与页间连接符同步写入 `PID_AUTHOR_TOPOLOGY`；删除符号后 `sync_author_after_edit` 自动修剪记录并清除管段端点的悬挂 `peer` 引用，移动符号后自动刷新管段端点坐标，重号符号自动标 `tag_status = "duplicated"`；
  3. **`L2-6` 导入 `.pid` 语义升格**：`promote_semantics_to_author` 支持按句柄集合选择性升格或全图批量升格，在旁侧无 `_Data.xml` 时仍可从 `instance` 符号路径推断类型；
  4. **`M5 + L2-7 + L6-2` 单源导出与编辑存回 `.pid`**：`save_author_document_to_dual` 直接以 `PID_AUTHOR + TopologyTable` 为唯一权威数据源同步生成 `_Data.xml + _Meta.xml + .pid + _Coverage.json`。

### 24.2 真实中位规模生产图纸「升格 → 编辑属性与新增符号 → 存回 `.pid` → `a/b` 级闭环」实测

| 环节 | 实测指标 | 结果 |
|---|---|---|
| 第 1 遍升格（`promote_pass1`） | 升格符号 **`115`** 个、管线 **`5`** 条、管路 **`5`** 条、管段 **`50`** 条、管件 **`54`** 个、页间连接符 **`16`** 个 | `unmapped_classes = 0` |
| 第 2 遍重跑（`promote_pass2`） | `newly_promoted = 0`、**`updated_in_place = 115`** | **100% 幂等** |
| 编辑与新增对象（`L6-2`） | 修改既有阀门位号为手工 `BV-EDITED-134`，新增阀门 `BV-ADDED-134` 并执行 `sync_author_after_edit` | `duplicated_tags = 0` |
| 存回真实空白模板 `.pid` + XML | 写出对象 **`236`** 个、关系 **`184`** 条；跨文件 `GraphicOID` / `ModelID` / `DrawingID` 对齐 **`166 / 166`（`100%`）**，`_SPID == DocUID` | `StrictVendor` 零违规 |
| `a` 级（重读与 Q9 索引自检） | `a_level_index_consistent = true`，重读归一化几何实体 **`2,824`** 条 | **通过（零悬挂）** |
| `b` 级（CAD 重载与 `plot_pdf` 出图） | 本仓重载 CAD 实体 **`943`** 个，写出有效 PDF `114,084 B`（图元 **`573`**、页内图元 **`509`**） | **通过**（`c1` 标「SPPID 未验证」） |

### 24.3 固定门禁读数

| 门禁项 | 改前读数（§23.2） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,193` 条 | **`7,198` 条**（**`7,131` 过** / **`66` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+5` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/io/pid_convert/{mod,recognize,export,promote}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `193 passed` | **`193 passed`** | **全绿** |

---

## 25. L2-1 M4 / L2-2 / L2-3 / L2-4 / L3-7（#136）：属性面板读写联动、来源证据只读组与三档置信度交互校核存回闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `ac6527ca1`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `src/app/commands/pidprops.rs`、`src/io/pid_convert/{mod,recognize,promote}.rs`（零触碰并行活跃分支 `pid-symbol-on-main` 与 `pid-stage-c` 的全部修改文件） |
| 证据目录子项 | `2026-11-21-issue136/`（`red.txt`、`green-fast.txt`、`real136_test.log`、`real_out/real136_summary.json`、`fmt.txt`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 25.1 先红后绿与交互契约核验

- **先红后绿**：`red.txt` 4 条应用内交互单测在骨架上全部变红（`0 passed; 4 failed`，退出码 `101`）→ 实现后 `green-fast.txt` 模块内 **`9 passed; 0 failed`**（退出码 `0`）：
  1. **`L2-1 M4 + L2-2 + L2-3` 属性面板双分组与溢出保护**：选中升格或导入对象后，属性面板呈现可编辑 `P&ID 属性`（仅列位号与非下划线前缀业务属性如 `公称直径`、`对端图号`、`说明`）与只读 `P&ID 来源证据`（来源、置信度、置信度分档、SPPID 类、变体编码、符号路径、`GraphicOID`；注：因缺 `RES-12` SPPID 界面截图，按本平台标准双分组自设计并注明未对照实物）；编辑业务属性时自动透传 `>2,048 B` XRecord 分片且完整保留内部证据键；
  2. **`L2-4` DWG/DXF 图面位号标注双向联动与单步撤销**：`recognize.rs` 与 `promote.rs` 追踪符号配对的图面文字句柄 `_tag_handles`（含单行设备/阀门位号及圈内上下双行仪表位号）；面板或 `PIDSETPROP` 修改 `pid_tag` 时同步更新图面 `TEXT` / `MTEXT`（冲突显示 `？`，双行仪表圈自动拆分上下行），`UNDO` / `REDO` 一步同时回滚 `PID_AUTHOR` 与图面文字；
  3. **`L3-7` 应用内一键升格与三档置信度人工确认/驳回**：落地 `PIDPROMOTE [ALL | SELECTED | SEMANTICS]` 与 `PIDREVIEW [SUMMARY | HIGH | MEDIUM | LOW | DUPLICATES | ACCEPT | REJECT]`，支持按档位一键画布选中、`ACCEPT` 人工确认（转 `manual` + `confirmed`）与 `REJECT` 剔除假阳性（同步清理 `TopologyTable` 管件/连接符及悬挂 `SegmentEnd.peer`），均支持单步撤销；
  4. **`L6-2` 应用内 `PIDSAVEPID` 前置重号门禁与双产物写出**：`PIDSAVEPID` 前置调用 `gate_save`（存在 `duplicated` 位号立即以 `TAG_DUPLICATED_BLOCKS_SAVE` 阻断保存且不写文件），冲突消除后直接写出 `.pid + _Data.xml + _Meta.xml + _Coverage.json`。

### 25.2 真实中位规模生产图纸在 `OpenCADStudio` 应用内全链路交互验证

| 交互步骤 | 应用内命令 / 面板操作 | 实测指标与结果 |
|---|---|---|
| 1. 一键升格 | `PIDPROMOTE` | 升格业务符号 **`115`** 个（`high = 103`、`medium = 12`、`low = 0`、`duplicated_tags = 0`） |
| 2. 筛选中置信度 | `PIDREVIEW MEDIUM` | 画布自动选中全部 **`12 / 12`** 个中置信度待校核符号 |
| 3. 批量人工确认 | `PIDREVIEW ACCEPT` | `medium` 归零（`0`），**`confirmed = 12`**、`manual_tags = 12`，三档总数 `103 + 12 = 115` 严格守恒 |
| 4. 面板校核与改位号 | 选中阀门校验 `P&ID 来源证据` 并改 `pid_tag = BV-INAPP-136` | `pid_ev_source = convert:dxf` 只读组就位；配对图面 `TEXT` 实体同步更新为新位号 |
| 5. 应用内直接存回 | `PIDSAVEPID "<out>.pid" SEED "<seed_zip>" SYMDIR "<sym_dir>"` | 写出 `.pid`（`671,744 B`）与 `_Data.xml`（`338,546 B`），跨文件 `GraphicOID` / `ModelID` / `DrawingID` **`165 / 165`（`100%`）** 对齐 |
| 6. `a / b` 级闭环校验 | `verify_dual_bundle`（`PidParser` + Q9 索引 + CAD 重载 + `plot_pdf`） | `a` 级重读归一化实体 **`2,822`** 条且 `a_level_index_consistent = true`；`b` 级重载 **`940`** 个 CAD 实体并写出有效 PDF `114,039 B`（图元 **`572`**、页内 **`508`**；`c1` 标「SPPID 未验证」） |

### 25.3 固定门禁读数

| 门禁项 | 改前读数（§24.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,198` 条 | **`7,203` 条**（**`7,135` 过** / **`67` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+5` 条测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `src/app/commands/pidprops.rs`、`src/io/pid_convert/{mod,recognize,promote}.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `193 passed` | **`193 passed`** | **全绿** |

---

## 26. LX-6..LX-9 / L4-2 / #117（#137）：送测三关预检与回执收敛 CLI、类变体兜底可观测性及分级来源调查（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `13635d1f1`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `scripts/sppid/{precheck_and_receipt,test_precheck_and_receipt}.py`、`src/io/pid_publish/{classes,write,mod}.rs`、`src/io/pid_convert/{export,promote}.rs`、`examples/pid_convert.rs`、`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/06-lx-compliance-and-codelist-audit.md` |
| 证据目录子项 | `2026-11-21-issue137/`（`probe_117_report.json`、`red-py.txt`、`green-py.txt`、`round1_control_precheck.json`、`red-rs.txt`、`green-rs.txt`、`gen90_summary.json`、`fmt.txt`、`arch.txt`、`pytest.txt`、`gate-*.txt`） |

### 26.1 双端先红后绿与三关预检 / 回执收敛验证

- **Python 端先红后绿（`LX-8 / LX-9`）**：`red-py.txt` `3 failed`（退出码 `1`）→ 实现 `scripts/sppid/precheck_and_receipt.py` 后 `green-py.txt` **`196 passed`**（退出码 `0`）：
  1. `precheck` 子命令：串联第①关（`StrictVendor` 校验）、第②关（`$OCS_PID_SPPID_RUNNER open` 解析 `open-result.json`）、第③关（`$OCS_PID_SPPID_RUNNER publish` 重发布 XML 与本方 XML 逐项 `xml_diff` 对账）；未配置运行器时默认返回退出码 `2` 阻断盲目送测（显式传入 `--allow-unverified-sppid` 时验证静态第①关并标注 `unverified_no_sppid_runner`）；已在真实首轮送测包 `round-01-20261121/00-control` 上实测 `gate1 = True`、`sppid = unverified_no_sppid_runner`；
  2. `evaluate-receipt` 子命令：核对 `manifest_sha256`、待定槽位与 `00-control` 阳性对照状态，自动推导 `R1..R5` 五项平台要求并输出下一轮减法变体清单。
- **Rust 端先红后绿（`L4-2` 衔接 `#117`）**：`red-rs.txt` `0 passed; 1 failed`（退出码 `101`）→ 实现 `VariantResolutionSummary` 与 `load_class_map_from_json` / `--class-map` 后 `green-rs.txt` 全绿（退出码 `0`）。

### 26.2 全批 90 张真实 DXF 类变体兜底可观测性实测与 `#117` 全包来源调查结论

| 指标 | 90 张真实 DXF 全批实测值 | 说明 |
|---|---:|---|
| 写出对象总数（`total_items_written`） | **`88,236`** | 与 §22.2 完全一致，零字节漂移 |
| 精确变体命中（`exact_variant_id`） | **`15,910`** | 全部已导出业务符号 `100%` 精确命中 `assets/pid-class-map.json` 变体 |
| 符号路径命中（`symbol_path_match`） | `0` | 存量 DXF 已全部先经 `class_map.rs` 解析到精确 `variant_id` |
| 结构派生默认（`structural_default`） | **`72,326`** | 管段、管线、端口、工艺点、分支点等无独立符号变体的派生结构对象（`15,910 + 72,326 = 88,236` 100% 守恒） |
| **表外类默认兜底（`class_default_fallback`）** | **`0`** | **全批 90 图零静默表外兜底**（`fallback_by_class = {}`，恒定写出至 `_Coverage.json`） |
| **无变体类计数（`unmapped_class_variant`）** | **`0`** | 零缺口 |

- **`#117` 第 2 步全厂备份包实查核心结论**（全文见同目录 `06-lx-compliance-and-codelist-audit.md` §1）：
  - 扫描 `BACKUP_ORACLE` 全部 `16` 个顶层文件（含 `Export.dmp` `126` 张表、全部 `10` 卷 `RefData~4~*`、`PlantConfig.xml` 及 `SPPIDDataMap.xml` `12,087` 条 `<Rel>`）：`SPPIDDataMap.xml` 仅含工具侧 3 层 `MapEnumMapEnum` 父子边（`1,573` 条），无法推出目标 `EFSchema.xml` 的 6 层设备分级链（`EqType0..EqType5`）及跳层管件父链，且 `SPECIALTY_ITEM` 两枚举 GUID 与仪表 `@EE3F8.1` 在包内均为 `0` 字节命中；因此分级表不能从备份包单独推出，须采用「**表外计数 100% 可观测 + 外置 JSON 按厂热替换**」方案（阶段一侧已在本单全部落地）。

### 26.3 固定门禁读数

| 门禁项 | 改前读数（§25.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,203` 条 | **`7,204` 条**（**`7,136` 过** / **`67` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+1` 条 Rust 测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `classes.rs`、`write.rs`、`mod.rs`、`export.rs`、`promote.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `193 passed` | **`196 passed`** | `+3` 条 Python 测试，**全绿** |

---

## 27. L3-1..L3-4 / L4-5 / L6-1（#138）：原生存量 AutoCAD DWG/DXF 多模态符号识别融合与 2/3 段存量线号转换闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `1b1495c37`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `assets/pid-sppid-legend-map.json`、`src/io/pid_convert/{class_map,recognize,export}.rs`、`src/io/pid_publish/names.rs` |
| 证据目录子项 | `2026-11-21-issue138/`（`red.txt`、`green-fast.txt`、`legacy12_summary.json`、`legacy12_check.json`、`baseline-legacy-12.tsv`、`gen90_reg_summary.json`、`current-90.tsv`、`sanitized_138_report.json`、`cargo_test.txt`、`clippy_lib2.txt`、`clippy_test2.txt`） |

### 27.1 先红后绿与设计摘要

- **问题根因**：阶段一前序 `L3/L4/L5` 在 90 张 SPPID 导出 DXF（图块名直接对应英文 `.sym` 主干、线号为 4–6 段 SPPID 格式）上闭环，但实测 `SOURCE_DWG` 批次 12 张真实原生 AutoCAD 存量图（涵盖两套非 SPPID 图例标准：`L01..L04, L11..L12` 为天正 `$VALVE$*` 图块 + 散圆仪表圈/容器/泵；`L05..L10` 为完全炸散成散线的 49 种形状指纹图）时，改前 `pid_convert` 仅扫描 `INSERT` 图块匹配英文 `.sym` 名，导致 **12 图精确变体映射数为 `0`（`variant_exact = 0`）**，且两段式（如 `80-FS`、`150-FW`）与三段式（如 `150-FW-31001`）存量线号因段数不足 4 段被 `parse_sppid_line_number` 拒绝。
- **先红阶段**（`red.txt`，退出码 `101`）：新增 `parses_two_part_and_three_part_legacy_line_numbers` 与 `converts_legacy_autocad_blocks_circles_and_two_part_line_numbers_to_valid_dual_bundle` 两条单测，按预期因断言失败变红。
- **转绿阶段**（`green-fast.txt`，`PUB_RC=0`、`CONV_RC=0`）：
  1. 在 `assets/pid-sppid-legend-map.json` 新增纯 ASCII `legacy_ignore_classes`（过滤非 P&ID 电器辅助类 `ignore / motor / eh-switch`）与 `legacy_class_map`（将 `io::pid_legend` 产出的全部 45 个业务类别映射到 `assets/pid-class-map.json` 中带真实库内 `.sym` 的精确 `variant_id`）；
  2. 在 `src/io/pid_convert/recognize.rs` 新增 `fuse_legacy_autocad_recognition`：仅当图纸无任何命中 SPPID `.sym` 的图块（`!symbols.iter().any(|s| s.known)`）时，只读复用 `crate::io::pid_legend::recognise_with_units`（零改动 `src/io/pid_legend/*`）融合块、圆与散线形状符号、位号句柄 `tag_handles`、页间连接符对端图号及 `legacy.pipes.runs` 管段成链；并在 `export.rs` 支持无预置线号图层时的 `"PIPE"` 管段兜底采集；
  3. 在 `src/io/pid_publish/names.rs` 扩展 `parse_sppid_line_number` 与 `ParsedLineNumber::pipeline_name()` 兼容 2 段（`<公称直径>-<介质>`）与 3 段（`<公称直径>-<介质>-<序号>`）存量线号，4+ 段 SPPID 线号行为保持不变。

### 27.2 12 张真实原生 AutoCAD 存量 DXF（`L01..L12`）双产物实测与 90 张 SPPID 基准零回归验证

| 汇总指标 | 改前（`1b1495c37`） | 改后（#138） | 说明 |
|---|---:|---:|---|
| 转换成功 / 失败图数 | `12 / 0`（空壳） | **`12 / 0`** | 全部 12 张原生存量 DXF 一次产出 `_Data.xml` + `_Meta.xml` + `_Coverage.json` + `.pid` + `.pidplot.pdf` |
| **精确变体映射数（`exact_variant_id`）** | **`0`** | **`1,619`** | 覆盖 8 大 SPPID 业务类：`PIDPipingComponent=610`、`PIDInstrument=308`、`PIDInlineInstrument=250`、`PIDBranchPoint=174`、`PIDNote=136`、`PIDControlSystemFunction=101`、`PIDProcessVessel=30`、`PIDMechanicalEquipment=10` |
| **表外类默认兜底（`class_default_fallback`）** | `0` | **`0`** | `unmapped_class_variant = 0`，100% 精确命中库内 `.sym` 变体 |
| 合成管线 / 端口 / 管端连通关系 | — | **`54` / `1,124` / `3,302`** | 两段与四段存量线号全部进入 `PIDPipeline` 合成；跨图页间连接符 `174` 个（本批内互逆成对 **`12` 对 = `24` 个**，`one_way_in_batch = 0`） |
| 写出总对象数 / 总关系数 | — | **`6,867` / `7,147`** | `convert_check.py` 实测 `24 / 24` 份 XML **`spec_violations = 0`、`coverage_errors = 0`** |
| 跨文件 `GraphicOID` 对齐（`matched / xml_reps`） | — | **`3,454 / 3,454`（`100%`）** | `12 / 12` 张 `.pid` 全部通过 `a` 级（重读 `20,808` 条归一化实体 + Q9 索引零故障）与 `b` 级（CAD 重载 `10,505` 实体 + `pid-plot` PDF 页内图元 `6,972`）；`c1` 标 `unverified_no_sppid_runner` |
| **90 张 SPPID DXF 回归门禁（`baseline-90.tsv`）** | 基准 | **`passed=True, unexpected=0, expected=0`** | `items=88,236`、`relations=96,888`、`exact_variant_id=15,910` 逐图逐列 **100% 零差异**（退出码 `0`） |

#### 12 张原生 AutoCAD 存量图纸脱敏逐图明细（`L01..L12`）

| 代号 | 源 DXF SHA-256 前缀 | 高置信度 | 中置信度 | 精确变体数 | 合成管线 | 端口数 | 对象总数 | 关系总数 | `GraphicOID` 对齐 | `a/b` 级验证 | PDF 图元数 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|
| `L01` | `a0f05f81a655` | `107` | `1` | `108` | `7` | `96` | `483` | `554` | `227 / 227` | `true / true` | `560` |
| `L02` | `66913e5a4dcb` | `145` | `0` | `145` | `7` | `130` | `686` | `789` | `319 / 319` | `true / true` | `648` |
| `L03` | `28ed389e23cb` | `118` | `0` | `118` | `5` | `114` | `493` | `562` | `236 / 236` | `true / true` | `579` |
| `L04` | `c43bd94ee217` | `155` | `0` | `155` | `6` | `150` | `745` | `880` | `347 / 347` | `true / true` | `736` |
| `L05` | `4f2200b7c055` | `209` | `13` | `222` | `0` | `67` | `836` | `700` | `449 / 449` | `true / true` | `720` |
| `L06` | `135b6bf6b0a6` | `90` | `37` | `127` | `5` | `73` | `673` | `747` | `345 / 345` | `true / true` | `642` |
| `L07` | `4051ce7189a6` | `168` | `40` | `208` | `4` | `147` | `850` | `844` | `437 / 437` | `true / true` | `727` |
| `L08` | `7553b30957f1` | `180` | `39` | `219` | `5` | `151` | `849` | `854` | `444 / 444` | `true / true` | `734` |
| `L09` | `8379ca9156bb` | `143` | `30` | `173` | `5` | `123` | `692` | `694` | `356 / 356` | `true / true` | `642` |
| `L10` | `5a2218c7924d` | `68` | `44` | `112` | `2` | `49` | `415` | `382` | `225 / 225` | `true / true` | `483` |
| `L11` | `fc7451dd86cb` | `8` | `3` | `11` | `3` | `0` | `49` | `41` | `27 / 27` | `true / true` | `233` |
| `L12` | `2e16d71c2b6f` | `18` | `3` | `21` | `5` | `24` | `96` | `100` | `42 / 42` | `true / true` | `268` |

### 27.3 固定门禁读数

| 门禁项 | 改前读数（§26.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,204` 条 | **`7,206` 条**（**`7,138` 过** / **`67` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `class_map.rs`、`recognize.rs`、`export.rs`、`names.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `196 passed` | **`196 passed`** | **全绿** |

---

## 28. L2-1..L2-7 / L3-7 / L6-2（#139）：原生存量多图元/散圆/多段线图纸 `PID_AUTHOR` 升格、整组校核与 `PIDSAVEPID` 12 图全批闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `b152958c1`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_write/{oid,index}.rs`、`src/io/pid_convert/{recognize,promote,mod}.rs`、`src/app/commands/pidprops.rs` |
| 证据目录子项 | `2026-11-21-issue139/`（`red.txt`、`green_pidwrite2.txt`、`green_convert2.txt`、`green_pidprops2.txt`、`real12_inapp/real139_legacy12_inapp_summary.json`、`cargo_test.txt`、`clippy_lib.txt`、`clippy_test.txt`、`gen90_reg_summary.json`） |

### 28.1 先红后绿与四项真实断链修复

- **先红阶段**（`red.txt`，`RED139_RC=101`）：构造含散圆储罐（`Circle`）、圈内双行散圆仪表（`Circle` + 双行 `Text`）、无 `Insert` 炸散散线阀门（`Line` 簇）与多顶点 `LwPolyline` 两段式线号（`80-FS`）管线的合成原生存量图纸，运行 `promote_sheet_to_author` + `save_author_document_to_dual` 按预期因 `pipe_segments_promoted == 0` 及非 `Insert` 符号存回断链变红。
- **转绿与实测捕获修复**：
  1. **多图元符号与多折线管段保真升格（`recognize.rs` + `promote.rs`）**：`ConvertedSymbol` 新增 `member_handles`；`promote_sheet_to_author` 在 `PidAuthor.props` 以字符串保留键记录 `_at`、`_bbox`、`_member_handles`（兼容 `split_props` 属性编辑写回），屏蔽全部符号成员图元免受管段覆盖，并将 `rec.pipe_segments` 中的 `Line` / `LwPolyline` 折线段写入 `_segments` 与 `TopologyTable`；
  2. **非 `Insert` 符号与世界坐标等比映射存回（`promote.rs`）**：`save_author_document_to_dual` 优先读取 `_at` / `_bbox` 及 `Circle` / `Arc` / `LwPolyline` / `Line` 世界锚点，并在图纸世界坐标超出种子图幅时自动按包围盒等比居中映射至图框内，消除硬钳位零长线；
  3. **`SeedOidPool` 显式 `preferred_oid` 水位预留（`pid_write/{oid,index}.rs`）**：在 `apply_increments_to_seed` 入口调用 `advance_watermark_past(max_preferred)`，消除克隆 `/JSite*` 或辅助文字 `alloc_oid(None)` 提前占用后续符号 `preferred_oid` 的碰撞隐患（实测 `L01` `144 / 144` `GraphicOID` 恢复 100% 命中）；
  4. **整组图元高亮、成员散线归并确认与一键消重（`promote.rs` + `pidprops.rs`）**：`PIDREVIEW HIGH/MEDIUM/LOW/DUPLICATES` 自动展开 `_member_handles` 整组高亮，点选任一成员散线执行 `ACCEPT / REJECT` 自动归并主句柄，并新增单步可撤销 `PIDREVIEW DEDUP` 自动消解原生存量图历史同名位号冲突。

### 28.2 12 张真实原生 AutoCAD 存量图（`L01..L12`）应用内交互全批闭环实测

在 `OpenCADStudio` 应用内逐张加载全部 12 张真实原生 AutoCAD 存量图，依次执行真实命令流 `PIDPROMOTE` → `PIDREVIEW MEDIUM` → `PIDREVIEW ACCEPT` → `PIDREVIEW DEDUP` → `PIDSAVEPID`（写出 `.pid` + `_Data.xml` + `_Meta.xml` + `_Coverage.json`），并逐张执行 `#111` `StrictVendor` 校验与 `#130` `a/b` 级重读/索引/PDF 出图校验：

| 汇总指标 | 全批 12 图应用内实测值 | 说明 |
|---|---:|---|
| 跑通应用内闭环图数 | **`12 / 12`** | 含 6 张天正图块+散圆仪表图（`L01..L04, L11..L12`）与 6 张全散线形状图（`L05..L10`） |
| 升格业务符号数 / 存回精确变体数 | **`1,619` / `1,619`（`100.00%`）** | 散圆仪表/容器/泵与炸散散线阀门/仪表/DCS 功能块 **零丢失**，**`class_default_fallback = 0`** |
| `PIDREVIEW MEDIUM` 人工确认数 | **`210` 个符号（高亮选中 `914` 个成员图元）** | 炸散形状符号按整组散线在画布同步高亮选中并一次确认转 `confirmed` |
| `PIDREVIEW DEDUP` 消解历史重号数 | **`18` 个**（`L02=2, L07=12, L08=4`） | 消解后全批 `12 / 12` 通过 `gate_save` 重号门禁 |
| `PIDSAVEPID` 写出对象总数 / 关系总数 | **`5,197` / `5,076`** | `24 / 24` 份 XML 通过 `StrictVendor` **0 违规** |
| 跨文件 `GraphicOID` 对齐（`matched / xml_reps`） | **`3,044 / 3,044`（`100.00%`）** | `12 / 12` 通过 `a` 级（Q9 索引零故障）与 `b` 级（`pid-plot` PDF 页内图元合计 `6,037`）；`c1` 标 `unverified_no_sppid_runner` |
| 90 张 SPPID 基准图回归门禁（`baseline-90.tsv`） | **`passed=True, unexpected=0, expected=0`** | 逐图逐列 **100% 零差异**（退出码 `0`） |

#### 12 张原生 AutoCAD 存量图应用内 `PIDPROMOTE → PIDREVIEW → PIDSAVEPID` 脱敏逐图明细

| 代号 | 源 DXF SHA-256 前缀 | 升格符号数 | `MEDIUM` 确认数（选中图元数） | `DEDUP` 消重数 | 存回精确变体 | 写出对象 / 关系 | `GraphicOID` 对齐 | `a/b` 级验证 | PDF 图元数 |
|---|---|---:|---:|---:|---:|---:|---:|---|---:|
| `L01` | `a0f05f81a655` | `108` | `1`（`1`） | `0` | `108` | `201 / 126` | `144 / 144` | `true / true` | `380` |
| `L02` | `66913e5a4dcb` | `145` | `0`（`0`） | `2` | `145` | `316 / 241` | `211 / 211` | `true / true` | `450` |
| `L03` | `28ed389e23cb` | `118` | `0`（`0`） | `0` | `118` | `241 / 168` | `166 / 166` | `true / true` | `407` |
| `L04` | `c43bd94ee217` | `155` | `0`（`0`） | `0` | `155` | `266 / 148` | `199 / 199` | `true / true` | `428` |
| `L05` | `4f2200b7c055` | `222` | `13`（`41`） | `0` | `222` | `1,075 / 1,124` | `603 / 603` | `true / true` | `874` |
| `L06` | `135b6bf6b0a6` | `127` | `37`（`150`） | `0` | `127` | `496 / 557` | `275 / 275` | `true / true` | `558` |
| `L07` | `4051ce7189a6` | `208` | `40`（`160`） | `12` | `208` | `637 / 644` | `365 / 365` | `true / true` | `642` |
| `L08` | `7553b30957f1` | `219` | `39`（`168`） | `4` | `219` | `693 / 701` | `392 / 392` | `true / true` | `667` |
| `L09` | `8379ca9156bb` | `143` -> `173` | `30`（`130`） | `0` | `173` | `556 / 546` | `315 / 315` | `true / true` | `588` |
| `L10` | `5a2218c7924d` | `112` | `44`（`258`） | `0` | `112` | `557 / 657` | `288 / 288` | `true / true` | `538` |
| `L11` | `fc7451dd86cb` | `11` | `3`（`3`） | `0` | `11` | `53 / 52` | `28 / 28` | `true / true` | `230` |
| `L12` | `2e16d71c2b6f` | `21` | `3`（`3`） | `0` | `21` | `106 / 112` | `58 / 58` | `true / true` | `275` |

### 28.3 固定门禁读数

| 门禁项 | 改前读数（§27.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,206` 条 | **`7,209` 条**（**`7,140` 过** / **`68` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条 Rust 测试（`+2` 常驻 + `+1` 12 图实测套件），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `oid.rs`、`index.rs`、`recognize.rs`、`promote.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `196 passed` | **`196 passed`** | **全绿** |

---

## 29. LX-8 / LX-9 / L6-1 / L6-3（#140）：五类真实存量图转换件配套送测包构建器、二进制 `.dwg` 直转核验与转换回执收敛（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `5c8fdbe65`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `scripts/sppid/{build_submission_converted,test_build_submission_converted,precheck_and_receipt}.py`、`src/io/pid_convert/{export,mod}.rs`、`examples/pid_convert.rs` |
| 外部送测包与证据目录 | `submissions/round-01b-converted-20261121/`（`manifest_sha256 = e40be5b2d915fafb...`）、`2026-11-21-issue140/`（`red-py.txt`、`green-py.txt`、`red-rs.txt`、`round1b_final.txt`、`cargo_test.txt`、`clippy_lib.txt`、`clippy_test.txt`） |

### 29.1 双端先红后绿与二进制 `.dwg`（`AC1032`）无损直转核验

- **Python 端先红后绿**：`red-py.txt` `2 failed`（退出码 `1`）→ 实现 `build_submission_converted.py` 与扩展 `precheck_and_receipt.py`（支持 `package_kind = "converted_pilot"` 推导 `C1..C5` 五类转换原型验收矩阵）后 `green-py.txt` **`198 passed`**（退出码 `0`）。
- **Rust 端先红后绿**：`red-rs.txt` `0 passed; 1 failed`（退出码 `101`）→ 实现 `save_cad_file_as_binary_dwg`（校验 `AC10*` 二进制头魔数）与 `examples/pid_convert.rs --save-dwg` 后 `converts_binary_dwg_file_directly_to_valid_dual_bundle` 一次通过。
- **真实二进制 `.dwg`（`AC1032`）对原 `.dxf` 转换一致性核验**：将原生 AutoCAD 存量图 `L01` 存为二进制 `AC1032` `.dwg`（`binary_dwg_sha256_prefix = e605ccf6d6f0`）后经 `pid_convert` 直转双产物，产出 **`483` 个对象、`554` 条关系、`108` 个精确变体、`0` 表外兜底、`_Data.xml` `588,255` 字节、`227 / 227` `GraphicOID` 100% 对齐**——与原 `.dxf` 转换结果逐项完全相等。

### 29.2 五槽「真实存量图转换件」送测就绪包（`submissions/round-01b-converted-20261121/`）实测表

- 送测包清单摘要：`manifest_sha256 = e40be5b2d915fafbef97bff9f2e7e6debc442850184418fde73045c84a46e9b9`，`package_kind = "converted_pilot"`，`precheck_passed = true`，`sppid_c1_d_precheck_status = "unverified_no_sppid_runner"`（全文零绝对路径、零客户图名，统一脱敏主干 `CONV-01..CONV-05`）。

| 槽位 | 交付主干 | 源 `.pid` SHA-256 前缀 | 精确变体数 | 表外兜底 | `StrictVendor` 违规数 | `GraphicOID` 对齐 | `a/b` 级验证 | 覆盖转换原型 |
|---|---|---|---:|---:|---:|---:|---|---|
| `01-sppid-dxf-direct` | `CONV-01` | `4fa01b17fe81` | `43` | `0` | `0` | `80 / 80` | `true / true` | SPPID 导出拆分 DXF → `pid_convert` 直出双产物 |
| `02-legacy-block-circle-direct` | `CONV-02` | `6bd7bed9d245` | `108` | `0` | `0` | `227 / 227` | `true / true` | 原生 AutoCAD 图块+散圆存量图 → `pid_convert` 直出双产物 |
| `03-legacy-exploded-shape-direct` | `CONV-03` | `972d3ddb6ec0` | `127` | `0` | `0` | `345 / 345` | `true / true` | 原生 AutoCAD 全散线形状存量图 → `pid_convert` 直出双产物 |
| `04-legacy-inapp-pidsavepid` | `CONV-04` | `f6f0720170a0` | `127` | `0` | `0` | `275 / 275` | `true / true` | 原生 AutoCAD 存量图 → 应用内 `PIDPROMOTE → PIDREVIEW → PIDSAVEPID` 存回双产物 |
| `05-binary-dwg-direct` | `CONV-05` | `f5d544e5aa52` | `108` | `0` | `0` | `227 / 227` | `true / true` | 二进制 `.dwg`（`AC1032`，`e605ccf6d6f0`）→ `pid_convert` 直转双产物 |

### 29.3 固定门禁读数

| 门禁项 | 改前读数（§28.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,209` 条 | **`7,210` 条**（**`7,141` 过** / **`68` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+1` 条 Rust 测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `export.rs`、`mod.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `196 passed` | **`198 passed`** | `+2` 条 Python 测试，**全绿** |

---

## 30. L4-6 / L5-8 / L6-1（#141）：应用内标准转换命令 `PIDCONVERT`（单图/整批双产物导出、`CLASSMAP` 覆盖与 `VERIFY` 自检）闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `6e0037a59`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{export,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据目录 | `$OCS_PID_EVIDENCE_DIR/2026-11-21-issue141/`（`red.txt`、`green.txt`、`real141_test.log`、`real_out/real141_summary.json`、`cargo_test.txt`、`clippy_lib.txt`、`clippy_test.txt`、`fmt_arch_pytest.txt`） |

### 30.1 先红后绿与命令契约核验

- **先红阶段（`red.txt`）**：新增 `l4_6_pidconvert_exports_current_unpromoted_sheet_xml_and_dual_verify_without_mutating_canvas` 与 `l4_6_pidconvert_batch_converts_mixed_dxf_and_binary_dwg_directory_with_summary_and_verify` 两条单测，实测 `0 passed; 2 failed`（`RED_RC=101`）。
- **落地实现与转绿（`green.txt`）**：
  1. `src/io/pid_convert/{export,mod}.rs`：新增 `verify_published_xml_pair`、`export_document_to_dir` 与 `verify_batch_dir_strict`，统一封装内存 `Document` 单图落盘（`_Data.xml` + `_Meta.xml` + `_Coverage.json` + 可选 `.pid` + `.pidplot.pdf`）及 `#111` `StrictVendor` + `a/b` 级双端核验；转换或映射校验失败时在创建目录前立即阻断，零半成品残留。
  2. `src/app/commands/pidprops.rs`：零触碰并行工作树（未改 `utilities.rs` 与 `commands/mod.rs`），注册并实现 `PIDCONVERT`：
     - 单图模式：`PIDCONVERT "<输出目录 | 输出.pid | HERE>" [STEM <主干>] [PLANT <装置>] [SEED "<种子.pid|zip>"] [DUAL] [SYMDIR "<目录>"] [CLASSMAP "<映射.json>"] [VERIFY]`（只读转换当前活动画布 `Document`，不写入 `PID_AUTHOR` XDATA、不置 `dirty`、不污染撤销栈）；
     - 批量模式：`PIDCONVERT BATCH "<输入文件或目录>" "<输出目录>" [PLANT <装置>] [SEED "<种子.pid|zip>"] [DUAL] [SYMDIR "<目录>"] [CLASSMAP "<映射.json>"] [VERIFY] [SUMMARY "<汇总.json>"]`（自动递归遍历非空 `.dxf` 与二进制 `.dwg`，跳过 `0` 字节空文件，输出跨图连接符配对与汇总 JSON）。
  3. 复跑 `app::commands::pidprops::tests::`：**`12 passed; 0 failed`**（退出码 `0`）。

### 30.2 真实多源图纸应用内 `PIDCONVERT` 单图与 `PIDCONVERT BATCH` 端到端实测表

在 `OpenCADStudio` 应用内直接对未升格真实图纸运行 `PIDCONVERT ... VERIFY` 与 `PIDCONVERT BATCH ... VERIFY SUMMARY`（全文采用纯 ASCII 脱敏主干，`sppid_native_c1_status = "unverified_no_sppid_runner"`）：

| 执行模式 | 脱敏主干 | 源图 SHA-256 前缀 | 精确变体数 | 表外兜底 | 写出对象 / 关系 | `StrictVendor` 违规数 | `GraphicOID` 对齐 | `a` 级 / `b` 级 PDF 图元 | 原型说明 |
|---|---|---|---:|---:|---:|---:|---:|---|---|
| 单图 `PIDCONVERT` | `INAPP-SINGLE-01` | `c0dd2cc4c75d` | `115` | `0` | `645 / 685` | `0` | `302 / 302` | `true` / `727` | 中位规模真实 SPPID 拆分 DXF（未升格画布零侵入直出双产物） |
| 批量 `PIDCONVERT BATCH` | `BATCH-01-SPPID` | `75d6f761fd13` | `43` | `0` | `160 / 153` | `0` | `80 / 80` | `true` / `406` | SPPID 导出拆分 DXF |
| 批量 `PIDCONVERT BATCH` | `BATCH-02-LEG-BLK` | `a0f05f81a655` | `108` | `0` | `483 / 554` | `0` | `227 / 227` | `true` / `560` | 原生 AutoCAD 图块+散圆存量图（`.dxf`） |
| 批量 `PIDCONVERT BATCH` | `BATCH-03-LEG-EXP` | `135b6bf6b0a6` | `127` | `0` | `673 / 747` | `0` | `345 / 345` | `true` / `642` | 原生 AutoCAD 全散线形状存量图（`.dxf`） |
| 批量 `PIDCONVERT BATCH` | `BATCH-04-BIN-DWG` | `e605ccf6d6f0` | `108` | `0` | `483 / 554` | `0` | `227 / 227` | `true` / `560` | 二进制 `AC1032` `.dwg`（与原 `.dxf` 转换结果 100% 相等） |
| **批量合计（含跳过 1 份 `0 B` 空文件）** | **`4 / 4` 成功（`failed = 0`）** | — | **`386`** | **`0`** | **`1,799 / 2,008`** | **`0`（`8 / 8` XML）** | **`879 / 879`** | **`4 / 4` 通过（`2,168` 图元）** | 单条应用内命令完成 `.dxf` + `.dwg` 混合整批双产物转换与自检 |

### 30.3 固定门禁读数

| 门禁项 | 改前读数（§29.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,210` 条（`7,141` 过 / `68` 忽略 / `1` 既有 `.pen`） | **`7,213` 条**（**`7,143` 过** / **`69` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条 Rust 测试（`+2` 常规 + `+1` 真实多源集成），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `pidprops.rs`、`export.rs`、`mod.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `198 passed` | **`198 passed`** | **全绿** |

---

## 31. L2-2 / L2-6 / L2-7 / L6-2（#142）：真实 `.pid` 原图应用内 `PIDPROMOTE` 按 `OID` 归并升格、面板/`PIDINSERT` 编辑与 `PIDSAVEPID`/`PIDCONVERT` 存回双产物闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `b6ade3c73`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{promote,export,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据目录 | `$OCS_PID_EVIDENCE_DIR/2026-11-21-issue142/`（`red.txt`、`green_promote.txt`、`green_pidprops.txt`、`real142_test.log`、`real_out/real142_summary.json`、`cargo_test.txt`、`clippy_lib_final.txt`、`clippy_test_final.txt`、`fmt_arch_pytest.txt`） |

### 31.1 先红后绿与 4 处真实 `.pid` 交互断链修复

- **先红阶段（`red.txt`）**：
  - `l2_6_real_pid_stroke_group_merges_by_oid_resolves_exact_variant_and_preserves_polyline_pipes`：`RED1_RC=101`（改前 2 个多笔画符号 + 1 条 `LwPolyline` 管线被裂变为 `10` 个符号对象、`9` 个假重号冲突、`0` 条管段）；
  - `l6_2_in_app_pid_import_promote_panel_edit_pidinsert_and_pidconvert_roundtrip`：`RED2_RC=101`。
- **落地修复与转绿**：
  1. **按 `oid` 多笔画归并、精确变体映射与 `LwPolyline` 管网构建（`src/io/pid_convert/promote.rs`，零改动 `src/io/pid/*`）**：
     - 对携带 `oid` 的原生 `.pid` 导入图元按 `oid` 聚合成单个主句柄（收集 `_member_handles`、联合 `_bbox` 与中心 `_at`），过滤纯派生连接点对象（`PIDPipingPort` / `PIDPipingBranchPoint` / `PIDProcessPoint`），并将独立位号文字组自动关联进对应符号 `_tag_handles`；
     - 利用 `PID-SYMBOL-LABEL` 图层上的 `.sym` 主干名匹配 `default_legend_class_map()` 与 `default_class_map()`，获得精确 `(sppid_class, variant_id, symbol_path)`（无 `oid` 单实体保持既有行为向后兼容）；
     - 将 `.pid` 工艺管线（`Line` 与多折线 `LwPolyline`，自动剔除复杂线型短刻度）升格为 `PIPE_SEGMENT` 并写入 `TopologyTable`；扩展 `save_author_document_to_dual` 原生支持 `LwPolyline` 折线管段与新建 `PIDINSERT` 符号的目录 `.sym` 默认路径。
  2. **成员笔画面板联动、原生 `.pid` 自动路由与 `PIDCONVERT` 已编辑模型优先导出（`src/app/commands/pidprops.rs` + `src/io/pid_convert/export.rs`）**：
     - `pid_author_section` 与 `commit_pid_author_property` 通过 `resolve_owning_author_handles` 反查主句柄，使点选多笔画符号任意一根笔画均可读写其 `PID_AUTHOR`；
     - 默认 `PIDPROMOTE` 检测到 `has_native_pid_semantics` 时直接执行 `promote_semantics_to_author`；
     - 单图 `PIDCONVERT` 当画布存在已校核/手建 `PID_AUTHOR` 对象（且未指定 `RAW`）时前置执行 `gate_save` 并调用 `export_author_document_to_dir`（在克隆副本上同步，保持活动画布只读零污染）。
  3. 复跑单测：`promote::tests` **`6 passed; 0 failed`**，`pidprops::tests` **`13 passed; 0 failed`**。

### 31.2 3 张真实 `.pid` 原图应用内「打开 → `PIDPROMOTE` → 面板改位号 + `PIDINSERT` → `PIDCONVERT` & `PIDSAVEPID`」实测表

对 `EXPORT_SET` 生产 `.pid`（`P01`、`P02`）与 `VENDOR_REF` 厂商参考 `.pid`（`P03`）逐张在 `OpenCADStudio` 内执行完整交互与存回核验（`sppid_native_c1_status = "unverified_no_sppid_runner"`）：

| 脱敏编号 | 源 `.pid` SHA-256 前缀 | 原始展开图元数 | 按 `oid` 归并符号数 | 面板改位号 + `PIDINSERT` 后精确变体数 | 表外兜底 | 写出对象 / 关系 | `StrictVendor` 违规数 | `GraphicOID` 对齐 | `a` 级 / `b` 级 PDF 图元 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `P01`（`EXPORT_SET`） | `81a99cefae55` | `519` | `119`（重号 `0`） | `120` | `0` | `259 / 197` | `0` | `164 / 164` | `true` / `367` |
| `P02`（`EXPORT_SET`） | `a7c3cfe616cb` | `1,661` | `298`（历史重号 `3` 经 `DEDUP` 消解） | `299` | `0` | `711 / 559` | `0` | `442 / 442` | `true` / `713` |
| `P03`（`VENDOR_REF`） | `c568d23f3992` | `973` | `213`（历史重号 `2` 经 `DEDUP` 消解） | `214` | `0` | `438 / 320` | `0` | `291 / 291` | `true` / `534` |
| **合计（`3 / 3` 通过）** | — | **`3,153`** | **`630`** | **`633 / 633`（`100%`）** | **`0`** | **`1,408 / 1,076`** | **`0`（`12 / 12` XML）** | **`897 / 897`（`100%`）** | **`3 / 3` 通过（`1,614` 图元）** |

- **90 张 SPPID 基准图回归门禁（`baseline-90.tsv`）**：`regression_gate.py diff` **`passed=True, unexpected=0, expected=0`**（退出码 `0`）。

### 31.3 固定门禁读数

| 门禁项 | 改前读数（§30.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,213` 条（`7,143` 过 / `69` 忽略 / `1` 既有 `.pen`） | **`7,216` 条**（**`7,145` 过** / **`70` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条 Rust 测试（`+2` 常规 + `+1` 真实 `.pid` 集成），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `promote.rs`、`export.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `198 passed` | **`198 passed`** | **全绿** |

---

## 32. LX-8 / LX-9 / L6-2 / L6-3（#143）：六槽全原型转换与编辑存回试点送测包（新增 `CONV-06`）及双形态回执收敛加固（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `39f6bd939`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `scripts/sppid/{build_submission_converted,test_build_submission_converted,precheck_and_receipt,test_precheck_and_receipt}.py` |
| 外部送测包与证据目录 | `submissions/round-01b-converted-20261121/`（`manifest_sha256 = bb39b67f93d8c64d...`）、`$OCS_PID_EVIDENCE_DIR/2026-11-21-issue143/`（`red-py.txt`、`green-py.txt`、`round1b_6slots.txt`） |

### 32.1 先红后绿与双形态回执兼容核验

- **先红阶段（`red-py.txt`）**：`3 failed, 195 passed`（`RED_PY_RC=1`，验证未接入第 6 槽 `--pid-inapp-bundle` 及布尔 `"accepted": true/false` 回执字段被判为 `pending`）。
- **落地实现与转绿（`green-py.txt`）**：
  1. `scripts/sppid/build_submission_converted.py`：新增第 6 槽 `06-pid-inapp-edit-save`（脱敏主干 `CONV-06`，`archetype_key = "C6_pid_inapp_edit_save_accepted"`，CLI 参数 `--pid-inapp-bundle`），向后兼容 5 槽与完整 6 槽打包；
  2. `scripts/sppid/precheck_and_receipt.py`：新增 `_slot_status_lower`，使减法包（`R1..R5`）与转换试点包（`C1..C6`）在 `evaluate-receipt` 下同时兼容 `"status": "accepted"/"rejected"` 与协议文档布尔 `"accepted": true/false` 两种回执回填写法；
  3. 复跑 `pytest scripts/sppid`：**`198 passed`**（退出码 `0`）。

### 32.2 六槽「真实存量图转换与交互编辑件」送测就绪包（`submissions/round-01b-converted-20261121/`）实测表

- 送测包清单摘要：`manifest_sha256 = bb39b67f93d8c64daa90b06b4377ce19b5cee55efa3ea29a4e92fcdf815f8397`，`package_kind = "converted_pilot"`，`precheck_passed = true`，`sppid_c1_d_precheck_status = "unverified_no_sppid_runner"`（共 6 槽 18 个交付文件，全文零绝对路径、零客户图名）。

| 槽位 | 交付主干 | 源 `.pid` SHA-256 前缀 | 精确变体数 | 表外兜底 | `StrictVendor` 违规数 | `GraphicOID` 对齐 | `a/b` 级验证 | 覆盖转换 / 编辑原型 |
|---|---|---|---:|---:|---:|---:|---|---|
| `01-sppid-dxf-direct` | `CONV-01` | `92c09b250f80` | `43` | `0` | `0` | `80 / 80` | `true / true` | SPPID 导出拆分 DXF → `pid_convert` 直出双产物 |
| `02-legacy-block-circle-direct` | `CONV-02` | `7a817a6626a5` | `108` | `0` | `0` | `227 / 227` | `true / true` | 原生 AutoCAD 图块+散圆存量图 → `pid_convert` 直出双产物 |
| `03-legacy-exploded-shape-direct` | `CONV-03` | `17b4c5bcba05` | `127` | `0` | `0` | `345 / 345` | `true / true` | 原生 AutoCAD 全散线形状存量图 → `pid_convert` 直出双产物 |
| `04-legacy-inapp-pidsavepid` | `CONV-04` | `f6f0720170a0` | `127` | `0` | `0` | `275 / 275` | `true / true` | 原生 AutoCAD 存量图 → 应用内 `PIDPROMOTE → PIDREVIEW → PIDSAVEPID` 存回双产物 |
| `05-binary-dwg-direct` | `CONV-05` | `83d282d79557` | `108` | `0` | `0` | `227 / 227` | `true / true` | 二进制 `.dwg`（`AC1032`，`e605ccf6d6f0`）→ `pid_convert` 直转双产物 |
| `06-pid-inapp-edit-save` | `CONV-06` | `fc7b3bfb080d` | `299` | `0` | `0` | `442 / 442` | `true / true` | 既有真实 `.pid` 原图 → 应用内 `PIDPROMOTE → 面板改位号 + PIDINSERT → PIDSAVEPID` 存回双产物 |
| **合计（`6 / 6` 槽通过）** | **`CONV-01..06`** | — | **`812`** | **`0`** | **`0`（`12 / 12` XML）** | **`1,596 / 1,596`** | **`6 / 6` 通过** | 逐槽执行 `precheck_and_receipt.py precheck --allow-unverified-sppid` 全部 `gate1=True` |

### 32.3 固定门禁读数

| 门禁项 | 改前读数（§31.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,216` 条（`7,145` 过 / `70` 忽略 / `1` 既有 `.pen`） | **`7,216` 条**（本单未改动 `.rs` 源码，与 `39f6bd939` 一致） | **0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 无 `.rs` 修改（`new=0`） | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `198 passed` | **`198 passed`** | **全绿** |

---

## 33. LX-2 / LX-4 / LX-9 / L6-3（#144）：第二轮二分送测包构建器（`V2-01..04`）、`LX-2` 运行器脚手架与 `LX-4` `E1..E3` 探针及回执收敛闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `2098e5ff8`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_write/{verify,mod}.rs`、`examples/pid_round2_probes.rs`、`scripts/sppid/{build_submission_round2,test_build_submission_round2,sppid_runner_template,precheck_and_receipt}.*` |
| 外部送测包与证据目录 | `submissions/round-02-bisect-20261121/`（`manifest_sha256 = 905ad0bd5c0ed0c8...`）、`submissions/lx4-sppid-probes-20261121/`（`manifest_sha256 = 7b6d314670908336...`）、`$OCS_PID_EVIDENCE_DIR/2026-11-21-issue144/` |

### 33.1 双端先红后绿与 `LX-2` 运行器契约核验

- **Python 端先红后绿**：`red-py.txt` `2 failed, 198 passed`（`RED_PY_RC=1`）→ 落地 `sppid_runner_template.ps1`（100% 7-bit ASCII Windows PowerShell 5.1 脚手架，支持 `open / publish / restore` 与无 BOM JSON 输出）、`build_submission_round2.py` 与 `precheck_and_receipt.py` 扩展（支持 `round2_bisect` `B1..B4` 与 `lx4_acceptance` `E1..E3` 收敛）后 `green-py.txt` **`200 passed`**（退出码 `0`）。
- **Rust 端先红后绿**：`red-rs.txt` `0 passed; 1 failed`（`RED_RS_RC=101`）→ 落地 `repack_pid_bytes_lossless`、`stamp_pid_summary_probe` 与 `examples/pid_round2_probes.rs` 后 `io::pid_write::` **`6 passed; 0 failed`**（退出码 `0`）。

### 33.2 第二轮定向二分送测包（`submissions/round-02-bisect-20261121/`）实测表

- 清单摘要：`manifest_sha256 = 905ad0bd5c0ed0c88c9abb743f9f6a0e71e4a5b7a6c77cd185149760f53dc676`，`package_kind = "round2_bisect"`，`precheck_passed = true`（5 槽 15 文件，全文零绝对路径、零客户图名）。

| 槽位 | 交付主干 | `.pid` 字节数（SHA-256 前缀） | `_Data.xml` 字节数 | `StrictVendor` 违规数 | 本地预检 | 二分探测目标 |
|---|---|---|---:|---:|---|---|
| `00-control` | `R2-CTRL-00` | `339,968`（`81a99cefae55`） | `206,753` | `0` | **通过** | 零改动原样阳性对照 |
| `01-v2-cfb-repack` | `R2-V2-01` | `339,968`（`74dbb9e6181b`） | `206,749` | `0` | **通过** | 流级无损 `PidWriter` 重打包 `.pid`（探测平台对 `cfb` 目录重排接受度 `B1`） |
| `02-v2-single-vessel-seed` | `R2-V2-02` | `57,344`（`26077035e5d6`） | `2,309` | `0` | **通过**（`1 / 1` `GraphicOID` 对齐） | 空白模板 `A1-W-New.pid` + 单台容器 `V-1001` 最小双产物（`B2` / `RES-14`） |
| `03-v2-piping-no-signal` | `R2-V2-03` | `339,968`（`81a99cefae55`） | `168,535` | `0` | **通过** | 保留全部工艺管网，单变量剥离仪表信号拓扑（`B3`） |
| `04-v2-leaf-codelist-only` | `R2-V2-04` | `339,968`（`81a99cefae55`） | `205,588` | `0` | **通过** | 保留末级类型码表，单变量置空上级分类父链属性（`B4` / `#117`） |

### 33.3 `LX-4` SPPID 接受度递进实验探针包（`submissions/lx4-sppid-probes-20261121/`）实测表

- 清单摘要：`manifest_sha256 = 7b6d3146709083361a82c4c2dab595dcb5ab45809f9a7ff2cf2ff809ec4ca013`，`package_kind = "lx4_acceptance"`，`precheck_passed = true`（3 槽 9 文件，全部通过 `sppid_runner_template.ps1 --dry-run` 三关契约验证）。

| 递进实验槽位 | 交付主干 | `.pid` 字节数（SHA-256 前缀） | `StrictVendor` 违规数 | 实验目标 |
|---|---|---|---:|---|
| `E1-byte-identical` | `LX4-E1` | `339,968`（`81a99cefae55`） | `0` | 字节完全相同副本放回，验证受控运行器与项目环境基线 |
| `E2-metadata-stamp` | `LX4-E2` | `339,968`（`0bf5ac880bcc`） | `0` | 仅改第 2 节 `_PlantName` 单点摘要属性，探测 SPPID 是否强检文件校验和/修改戳 |
| `E3-foreign-single-vessel` | `LX4-E3` | `57,344`（`26077035e5d6`） | `0` | 从未在数据库登记的空白模板 + 单容器 `.pid`，直接验证 `c1` 外来文件可打开性 |

### 33.4 固定门禁读数

| 门禁项 | 改前读数（§32.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,216` 条（`7,145` 过 / `70` 忽略 / `1` 既有 `.pen`） | **`7,217` 条**（**`7,146` 过** / **`70` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+1` 条 Rust 测试，**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `verify.rs`、`mod.rs`、`examples/pid_round2_probes.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `198 passed` | **`200 passed`** | `+2` 条 Python 测试，**全绿** |

---

## 34. L3-6 / RES-11 / V1-03（#145）：外置设备表/管线表（`XLSX/CSV/TSV/JSON`）属性回填接口与 `PIDProcessPoint` 温压自动路由闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `f3b2c556a`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/external_props.rs`、`src/io/pid_convert/{mod,export,promote}.rs`、`examples/pid_convert.rs`、`src/app/commands/pidprops.rs` |
| 外部证据目录 | `$OCS_PID_EVIDENCE_DIR/2026-11-21-issue145/`（`red.txt`、`green_extprops.txt`、`green_pidprops.txt`、`real145_test.log`、`real_out/real145_summary.json`、`reg90_diff.txt`、`cargo_test.txt`、`clippy_lib.txt`、`clippy_test.txt`、`fmt_arch_pytest.txt`） |

### 34.1 先红后绿与多格式外置工程表 + `.PPT` 温压自动路由设计

- **先红阶段（`red.txt`）**：`loads_multisheet_xlsx_csv_and_json_routes_process_point_temp_pressure_and_keeps_strict_vendor` `0 passed; 1 failed`（`RED_RC=101`）。
- **落地实现与转绿**：
  1. `src/io/pid_convert/external_props.rs`：零新增外部 crate（复用 `calamine` 与 `serde_json`），原生支持 `.xlsx` / `.xlsm` / `.xls`（自动扫描全部工作表）、`.csv`、`.tsv`、`.json`；同时支持精确 `<Interface>.<Attribute>` 表头与中英双语工程表头（`位号/ItemTag/LineNumber`、`设备名称/Description`、`公称直径/DN`、`管道等级/Spec`、`保温规格/InsulSpec`、`介质/FluidCode`、`设计压力/PressureMax`、`设计温度/PhaseTemperatureMax`、`操作压力/PressureBase`、`操作温度/PhaseTemperatureBase`）；
  2. **SPPID `PIDProcessPoint` 温压路由与 `StrictVendor` 白名单守门**：管线表命中 `PIDPipeline` 时，自动沿 `PipingConnectors` + `ProcessPointCollection` 关系链将温压属性写入 `<conn_uid>.PPT`（`IBulkMaxProcessPoint` / `IBulkBaseProcessPoint` / `IBulkMinProcessPoint`），将管道等级/保温规格传播至所属 `PIDPipingConnector`，并由 `default_publish_spec()` 严格校验类/接口允许集，自动过滤无关杂列；
  3. **全入口接线**：`ConvertOptions::external_props_path`、`examples/pid_convert.rs --external-props <FILE>` 以及应用内 `PIDCONVERT`（单图与 `BATCH`）和 `PIDSAVEPID` 的 `PROPS "<FILE>"`，结果统计写入 `_Coverage.json` `external_props_applied`；
  4. 复跑单测：`external_props::tests` **`1 passed; 0 failed`**，`pidprops::tests` **`14 passed; 0 failed`**。

### 34.2 真实生产 DXF + 双工作表 `.xlsx`（设备表 + 管线表）CLI 与应用内双路径实测

| 验证项 | 未加 `PROPS` 基线 | 加双工作表 `.xlsx`（CLI `--external-props` / 应用内 `PIDCONVERT ... PROPS`） | 说明 |
|---|---:|---:|---|
| 源图 SHA-256 前缀 | `c0dd2cc4c75d` | `c0dd2cc4c75d` | 中位规模真实 SPPID 导出拆分 DXF |
| 工程表加载行数 / 匹配图面对象数 | `0 / 0` | **`127` 行 / `183` 个对象** | 同时覆盖 `EquipmentAndValves` 与 `LineSchedule` 两个工作表 |
| 更新 `<conn_uid>.PPT` 工艺点数 | `0` | **`59` 个** | 非空 `PressureMax` 工艺点由 **`0` 升至 `59` 个** |
| 合规注入属性数 / 安全过滤杂列次数 | `0 / 0` | **`153` 项 / `135` 次** | 全部输出 XML 通过 `#111` `StrictVendor` **0 违规** |
| 双产物对齐与 `a/b` 级验证 | `302 / 302` | **`302 / 302`（`100%`）** | `a_valid = true`，`b` 级 PDF 页内图元 `727`，`class_default_fallback = 0` |
| 默认配置 90 张基准图回归门禁 | `baseline-90.tsv` | **`passed=True, unexpected=0, expected=0`** | 未传 `external_props_path` 时逐图逐列 **100% 零差异**（退出码 `0`） |

### 34.3 固定门禁读数

| 门禁项 | 改前读数（§33.4） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,217` 条（`7,146` 过 / `70` 忽略 / `1` 既有 `.pen`） | **`7,220` 条**（**`7,148` 过** / **`71` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+3` 条 Rust 测试（`+2` 常规 + `+1` 真实 `.xlsx` 集成），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 6 个新增与修改 `.rs` 文件全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `200 passed` | **`200 passed`** | **全绿** |

---

## 35. L2-6 / L2-7 / L6-2（#146）：无伴随 XML 纯裸 `.pid` 放置边界归并升格与 `VENDOR_REF` + `BACKUP_ORACLE` 8 图应用内全批核验闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `d3de3b0cd`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `src/io/pid_convert/promote.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-21-issue146/`（`red.txt`、`green_promote.txt`、`green_pidprops.txt`、`real_vendor_and_bare_pids.txt`、`real_out/real146_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 35.1 先红后绿与设计摘要

- **问题根因**：`#142` 已打通带伴随 `<stem>_Data.xml` 的真实 `.pid` 原图按 `oid` 归并升格，但《智能PID平台-阶段一实施方案.md》`L2-6` 明确要求「`_Data.xml` 缺省时仍能显示基本身份并升格编辑存回」（典型场景为全厂备份包 `BACKUP_ORACLE` 解包出的纯裸 `.pid` 文件）。在无伴随 `_Data.xml` 时，`load_pid` 产出的 `PID_SEMANTICS` 全部 `oid == None`，原回退分支逐笔画按 `"ent:<handle>"` 裂变（例如 1 台 4 笔画阀门裂变为 4 个重号符号）、未按 `LegendClassMap` 过滤忽略类符号、未就近配对散落位号文字，也未将无 `oid` 的管线实体配对进入 `TopologyTable`。
- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/io/pid_convert/promote.rs` 新增 `l2_6_xml_free_bare_pid_merges_placement_runs_pairs_nearby_tags_and_rebuilds_dual`（构造无 `oid` 纯裸 `.pid` 展开实体：容器 5 笔画 + 结尾 `role=symbol-label` + 邻近位号文字 `V-3301`、阀门 3 笔画 + 邻近位号 `XV-3302`、忽略类标题栏符号 `TITLEBLOCK` 及带线号 `100-PW-3301-CS150` 的管线段），改前因符号裂变为 `8` 个（预期 `2` 个）断言失败变红。
- **转绿阶段**（`green_promote.txt` `7 passed; 0 failed`、`green_pidprops.txt` `14 passed; 0 failed`）：
  1. **纯裸 `.pid` 放置边界归并**：在 `promote_semantics_to_author` 中识别无 `oid` 纯裸 `.pid` 实体——`load_pid` 将每次 `.sym` 放置展开为连续带相同 `extent=(x1,y1,x2,y2)` 的几何笔画并在末尾紧跟 `role=symbol-label` 隐藏 `Text`（携带 `.sym` 主干名），按该放置边界将整组笔画归并为单一 `AuthorSymbol`（`group_key = "bare_sym:..."`，写入完整 `_member_handles`）；
  2. **忽略类过滤与精确变体映射**：利用结尾 `symbol-label` `.sym` 主干名查询 `LegendClassMap`，自动跳过 `is_ignored` 非业务符号并获得精确 `variant_id`（`class_default_fallback = 0`）；
  3. **可见位号/线号文字配对与逆向拓扑重建**：对无 `extent`、无 `role` 的散落 `Text`/`MText`（`BareFreeText`），在无预设位号时按包围盒距离贪心一对一绑定可见设备/仪表位号并写入 `_tag_handles`，同时将无 `extent` 的 `Line`/`LwPolyline` 管线段配对邻近线号文字升格进入 `TopologyTable`，支持应用内 `PIDSAVEPID` 逆向重建完整 `_Data.xml` + `_Meta.xml` + `.pid` 双产物。

### 35.2 全批 5 张厂商验收参考 `.pid`（`V01..V05`）+ 3 张无 XML 纯裸备份 `.pid`（`B01..B03`）应用内实测

| 代号 | 样本类别 | 源 `.pid` SHA-256 前缀 | 展开模型实体数 | 归并升格符号数 | 精确变体数 / 兜底 | 写出对象数 | 写出关系数 | `GraphicOID` 对齐 | `a/b` 级验证 | PDF 图元数 |
|---|---|---|---:|---:|---:|---:|---:|---:|---|---:|
| `V01` | `VENDOR_REF`（含超大型全集） | `c568d23f3992` | `973` | `62` | `62 / 0` | `374` | `452` | `178 / 178` | `true / true` | `570` |
| `V02` | `VENDOR_REF`（含超大型全集） | `69d8bdff3aaa` | `1,182` | `72` | `72 / 0` | `494` | `600` | `227 / 227` | `true / true` | `659` |
| `V03` | `VENDOR_REF`（含超大型全集） | `778c9395d439` | `1,331` | `83` | `83 / 0` | `477` | `534` | `226 / 226` | `true / true` | `679` |
| `V04` | `VENDOR_REF`（含超大型全集） | `655e7281f51a` | `2,167` | `154` | `154 / 0` | `845` | `978` | `392 / 392` | `true / true` | `886` |
| `V05` | `VENDOR_REF`（超大型 `5,576` 实体） | `ecc602494c4d` | `5,576` | `325` | `325 / 0` | `2,120` | `2,504` | `977 / 977` | `true / true` | `1,638` |
| `B01` | `BACKUP_ORACLE`（**纯裸无 XML**） | `837d439e2e52` | `899` | `61` | `61 / 0` | `343` | `385` | `157 / 157` | `true / true` | `493` |
| `B02` | `BACKUP_ORACLE`（**纯裸无 XML**） | `6f83a78c5445` | `1,028` | `62` | `62 / 0` | `499` | `642` | `231 / 231` | `true / true` | `578` |
| `B03` | `BACKUP_ORACLE`（**纯裸无 XML**） | `a7c3cfe616cb` | `1,661` | `110` | `110 / 0` | `706` | `848` | `344 / 344` | `true / true` | `711` |
| **合计（8 图）** | **`5` 参考 + `3` 纯裸无 XML** | — | **`14,817`** | **`929`** | **`929 / 0`** | **`5,858`** | **`6,943`** | **`2,732 / 2,732`（`100%`）** | **`8/8` 全过** | **`6,214`** |

- **`StrictVendor` 合规性**：全部 `16 / 16` 份生成的 `_Data.xml` 与 `_Meta.xml`（含由纯裸 `.pid` 逆向重建的 `B01..B03` 6 份 XML）通过 `#111` `StrictVendor` **0 违规**（`c1` 按规程标 `unverified_no_sppid_runner`）。

### 35.3 固定门禁读数

| 门禁项 | 改前读数（§34.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,220` 条（`7,148` 过 / `71` 忽略 / `1` 既有 `.pen`） | **`7,222` 条**（**`7,149` 过** / **`72` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 8 图真实 `.pid` 集成），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `promote.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `200 passed` | **`200 passed`** | **全绿** |

---

## 36. LX-8 / LX-9 / L3-6 / L2-6 / L6-3（#147）：八槽全原型转换试点送测包升级（新增 `CONV-07` 外置工程表温压回填件与 `CONV-08` 纯裸 `.pid` 逆向双产物件）及 `C1..C8` 跨槽诊断收敛闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `f7ad35999`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `scripts/sppid/{build_submission_converted,test_build_submission_converted,precheck_and_receipt}.py` |
| 外部证据子项 | `2026-11-21-issue147/`（`red-py.txt`、`green-py.txt`、`round1b_8slots.txt`、`cargo_test.txt`）与 `submissions/round-01b-converted-20261121/`（8 槽 24 文件） |

### 36.1 先红后绿与设计摘要

- **先红阶段**（`red-py.txt`，`RED_RC=1`）：在 `scripts/sppid/test_build_submission_converted.py` 新增 `test_build_submission_converted_supports_eight_slots_gates_external_props_and_derives_cross_slot_diagnostics`，验证 `--external-props-bundle`（`CONV-07`）与 `--bare-pid-bundle`（`CONV-08`）打包、外置属性注入强检及跨槽对比诊断归因，运行按预期失败变红。
- **转绿阶段**（`green-py.txt`，`201 passed`）：
  1. 扩展 `scripts/sppid/build_submission_converted.py`（向后兼容 5/6/7/8 槽）：新增第 7 槽 `07-sppid-dxf-external-props`（`CONV-07`，`C7_external_props_schedule_accepted`，强检 `_Coverage.json` `external_props_applied.attributes_injected > 0` 并记录 `external_props_process_points_updated`）与第 8 槽 `08-bare-pid-inapp-rebuild`（`CONV-08`，`C8_bare_pid_rebuild_accepted`，记录 `bare_pid_rebuilt_xml = true`）；
  2. 扩展 `scripts/sppid/precheck_and_receipt.py` `evaluate-receipt`：对 `converted_pilot` 包输出 `C1..C8` 全量判定并自动派生跨槽对比 `diagnostic_hints`（例如当 `01-sppid-dxf-direct` 被拒而 `07-sppid-dxf-external-props` 通过时给出 `"external_schedule_props_required_for_dxf"`；当 `06-pid-inapp-edit-save` 通过而 `08-bare-pid-inapp-rebuild` 被拒时给出 `"companion_xml_or_schedule_needed_for_bare_pid"`）。

### 36.2 重建八槽全原型转换试点送测包（`submissions/round-01b-converted-20261121/`，8 槽 24 文件）实测

- 清单摘要：**`manifest_sha256 = 325551c26454b7810fde673ad67639dd0259368caf5c5b3512a5ed3a6fa5a854`**，`precheck_passed = true`（零本机盘符路径、零客户图名，全部 `8 / 8` 槽通过 `precheck --allow-unverified-sppid` Gate 1 零违规）。

| 槽位 | 交付主干 | 原型验收键 | 精确变体数 / 兜底 | `StrictVendor` 违规 | `GraphicOID` 对齐 | `a/b` 级验证 | 新增专属核验读数 |
|---|---|---|---:|---:|---:|---|---|
| `01-sppid-dxf-direct` | `CONV-01` | `C1_sppid_dxf_accepted` | `43 / 0` | `0` | `80 / 80` | `true / true` | 与 `CONV-07` 构成外置表对照基线 |
| `02-legacy-block-circle-direct` | `CONV-02` | `C2_legacy_block_circle_accepted` | `108 / 0` | `0` | `227 / 227` | `true / true` | 与 `CONV-05` 构成 `.dxf`/`.dwg` 对照 |
| `03-legacy-exploded-shape-direct` | `CONV-03` | `C3_legacy_exploded_shape_accepted` | `127 / 0` | `0` | `345 / 345` | `true / true` | 炸散多散线形状直转 |
| `04-legacy-inapp-pidsavepid` | `CONV-04` | `C4_inapp_pidsavepid_accepted` | `127 / 0` | `0` | `275 / 275` | `true / true` | 原生存量图应用内交互存回 |
| `05-binary-dwg-direct` | `CONV-05` | `C5_binary_dwg_accepted` | `108 / 0` | `0` | `227 / 227` | `true / true` | `binary_dwg_header = "AC1032"` |
| `06-pid-inapp-edit-save` | `CONV-06` | `C6_pid_inapp_edit_save_accepted` | `299 / 0` | `0` | `442 / 442` | `true / true` | 既有带 XML `.pid` 应用内编辑存回 |
| **`07-sppid-dxf-external-props`** | **`CONV-07`** | **`C7_external_props_schedule_accepted`** | **`115 / 0`** | **`0`** | **`302 / 302`** | **`true / true`** | **`attributes_injected = 153`、`.PPT = 59`** |
| **`08-bare-pid-inapp-rebuild`** | **`CONV-08`** | **`C8_bare_pid_rebuild_accepted`** | **`110 / 0`** | **`0`** | **`344 / 344`** | **`true / true`** | **`bare_pid_rebuilt_xml = true`** |
| **合计（8 槽 24 文件）** | `CONV-01..08` | `C1..C8` | **`1,037 / 0`** | **`0`（`16/16` XML）** | **`2,242 / 2,242`（`100%`）** | **`8/8` 全过** | **覆盖全部 8 类真实工程输入形态** |

### 36.3 固定门禁读数

| 门禁项 | 改前读数（§35.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,222` 条（`7,149` 过 / `72` 忽略 / `1` 既有 `.pen`） | **`7,222` 条**（**`7,149` 过** / **`72` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | **0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告**（零 `.rs` 改动） | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告**（零 `.rs` 改动） | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 本单零 `.rs` 改动（`new=0`） | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `200 passed` | **`201 passed`** | `+1` 条 Python 测试，**全绿** |

---

## 37. LX-8 / LX-9 / L6-3（#148）：四套送测与探针实物包一键完整性、密码学哈希自洽及脱敏零泄漏总审计 CLI（`audit-packages`）闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `2672a54d6`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 修改文件 | `scripts/sppid/{precheck_and_receipt,test_precheck_and_receipt}.py` |
| 外部证据子项 | `2026-11-21-issue148/`（`red-py.txt`、`green-py.txt`）与 `submissions/delivery-readiness-summary.json` |

### 37.1 先红后绿与设计摘要

- **先红阶段**（`red-py.txt`，`RED_RC=1`）：在 `scripts/sppid/test_precheck_and_receipt.py` 新增 `test_audit_packages_verifies_multi_package_integrity_and_catches_tampering_or_leaks`（覆盖多包批量审计、单字节交付物篡改拦截与 `receipt.json` 主机路径泄漏拦截），按预期失败变红。
- **转绿阶段**（`green-py.txt`，`202 passed`）：
  1. 在 `scripts/sppid/precheck_and_receipt.py` 落地 `audit_submission_packages` 与 CLI 子命令 `audit-packages --submissions-root <DIR> [--out-json <FILE>]`；
  2. 自动完成四重独立审计：① 重算 `slots` 规范化 JSON 的 SHA-256 并与 `manifest_sha256` 及 `receipt.json` 指纹、槽位列表交叉核对；② 逐槽逐文件核验相对路径安全性（禁盘符、禁 `..`）、磁盘存在性、精确字节长度与 SHA-256 哈希；③ 对 `submission-manifest.json`、`receipt.json` 与 XML（豁免 SPPID 标准 `<IFile FilePath="..."/>` 规格属性）执行主机路径零泄漏扫描；④ 对全部 `_Data.xml` / `_Meta.xml` 独立重跑 `SchemaOnly` 与 `StrictVendor`（按槽位 `expect_strict_vendor_pass` 契约）零违规校验，并输出纯 ASCII 脱敏总交接单 `delivery-readiness-summary.json`。

### 37.2 阶段一全部 4 套送测与探针实物包（`submissions/`，22 槽 65 文件）总审计实测

| 包目录名 | `package_kind` | `manifest_sha256` 前 16 位 | 槽位数 | 文件数 | 总字节数 | 回执状态 | 独立审计结果 |
|---|---|---|---:|---:|---:|---|---|
| `round-01-20261121` | `round1_subtraction` | `26eb6189e08d506a` | `6` | `17` | `2,838,997` | `pending` | **`audit_passed = true`** |
| `round-01b-converted-20261121` | `converted_pilot` | `325551c26454b781` | `8` | `24` | `9,973,718` | `pending` | **`audit_passed = true`** |
| `round-02-bisect-20261121` | `round2_bisect` | `905ad0bd5c0ed0c8` | `5` | `15` | `2,215,756` | `pending` | **`audit_passed = true`** |
| `lx4-sppid-probes-20261121` | `lx4_acceptance` | `7b6d314670908336` | `3` | `9` | `1,158,135` | `pending` | **`audit_passed = true`** |
| **合计（4 套实物包）** | — | — | **`22`** | **`65`** | **`16,186,606`** | `pending` | **`errors = []`（`100%` 通过）** |

### 37.3 固定门禁读数

| 门禁项 | 改前读数（§36.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,222` 条（`7,149` 过 / `72` 忽略 / `1` 既有 `.pen`） | **`7,222` 条**（**`7,149` 过** / **`72` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | **0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告**（零 `.rs` 改动） | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告**（零 `.rs` 改动） | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | 本单零 `.rs` 改动（`new=0`） | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `201 passed` | **`202 passed`** | `+1` 条 Python 测试，**全绿** |

---

## 38. L3-4 / L2-2 / L6-2（#149）：应用内多标签页跨图页间连接符（`Off-Page Connector`）互逆配对校核、断链定位与一键跳转聚焦命令 `PIDOPC` 闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `0fbe04710`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{opc,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-21-issue149/`（`red.txt`、`green_pidprops.txt`、`real_multi_tab_opc.txt`、`real_out/real149_summary.json`、`fmt_arch_pytest.txt`、`clippy_counts.json`） |

### 38.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidopc_pairs_multi_tab_connectors_selects_broken_and_jumps_camera_to_peer`（构造双标签页图纸 `DWG-2101-01` 与 `DWG-2101-02`，覆盖互逆配对、单向断链、外部未打开及升格前后双向跨标签页跳转与相机聚焦），运行按预期在 `PIDOPC EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`15 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/opc.rs`（`analyze_open_sheets_opc`）：跨所有打开图纸标签页自动提取标题栏图号与文件名前缀候选码，统一兼容**未升格存量 `.dxf`/`.dwg`**（只读调用 `recognise_sppid_sheet`）与**已升格 `PID_AUTHOR` + `TopologyTable` 模型**，将全工作区连接符分类为 `Paired`（互逆配对）、`OneWayOpen`（目标图纸已打开但缺反向连接符）、`ExternalUnopened`（目标图纸未打开）、`MissingTarget`（缺目标图号）；
  2. 在 `src/app/commands/pidprops.rs` 注册并分发 `PIDOPC`：支持 `SUMMARY`、`ALL`、`PAIRED`、`UNPAIRED`（整组高亮选中本图单向断链/缺目标连接符）、`JUMP`（切换至对端标签页、整组选中对端连接符并将视口相机精准聚焦至对端坐标）及 `EXPORT "<json>"`。

### 38.2 真实双标准成对图纸（SPPID 生产图对 + 原生 AutoCAD 存量图对）多标签页跳转核验

| 标签页代号 | 样本类别 | 源图 SHA-256 前缀 | 跨标签页实测行为与核验结果 |
|---|---|---|---|
| `SP01` | `EXPORT_DXF`（SPPID 拆分生产图） | `2164f88bbb57` | `PIDOPC PAIRED -> PIDOPC JUMP` 精准切换至 `SP02` 并选中聚焦对端连接符 |
| `SP02` | `EXPORT_DXF`（SPPID 拆分生产图） | `fc2c812dc182` | 执行 `PIDPROMOTE` 升格进 `PID_AUTHOR` 后反向 `PIDOPC JUMP` 精准返回 `SP01` |
| `LG01` | `SOURCE_DWG`（原生 AutoCAD 存量图） | `a0f05f81a655` | `PIDOPC PAIRED -> PIDOPC JUMP` 精准切换至 `LG02` 并选中聚焦对端连接符 |
| `LG02` | `SOURCE_DWG`（原生 AutoCAD 存量图） | `28ed389e23cb` | 4 图同开共识别 `43` 个连接符，自动锁定 **`8` 对（`16` 个）互逆连接符**，`one_way_open = 0` |

### 38.3 固定门禁读数

| 门禁项 | 改前读数（§37.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,222` 条（`7,149` 过 / `72` 忽略 / `1` 既有 `.pen`） | **`7,224` 条**（**`7,150` 过** / **`73` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 4 图多标签页跳转），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `opc.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 39. L3-6 / L4-6 / L6-1 / L6-2（#150）：应用内设备/仪表/管线清册一键提取导出命令 `PIDBOM` 与 CLI `--export-schedule`（与 `PROPS` 外置工程表同构往返闭环）（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `0fdad0d35`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{bom,mod}.rs`、`src/app/commands/pidprops.rs`、`examples/pid_convert.rs` |
| 外部证据子项 | `2026-11-21-issue150/`（`red.txt`、`green_pidprops.txt`、`real_multi_source_bom.txt`、`cli_export_schedule.txt`、`real_out/real150_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 39.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidbom_exports_isomorphic_schedule_and_roundtrips_through_pidconvert_props`（测试双标签页 `PIDBOM ALLTABS` 合并导出 `.csv` / `.json`、直接兼容 `load_external_property_table` 加载、补填温压/描述后经 `PIDCONVERT ... PROPS ... VERIFY` 回填通过 `StrictVendor` 零违规），运行按预期在 `PIDBOM must write CSV schedule` 变红。
- **转绿阶段**（`green_pidprops.txt`，`16 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/bom.rs`：零新增外部依赖，支持从单张或多张打开图纸（兼容未升格 `.dxf`/`.dwg` 与已升格 `PID_AUTHOR` 模型）以及批处理输出目录 `_Data.xml` 提取 `Equipment`、`Pipeline`、`Instrument`、`Valve` 四大类带位号对象，自动沿 `PIDPipeline -> PIDPipingConnector -> PIDProcessPoint` 汇总公称直径、管道等级、流体代号、保温规格与温压，跨多图按规范化位号去重归并所属图号与出现频次；
  2. 支持导出 14 列 UTF-8 BOM `.csv`、`.tsv` 与带 `"items"` 映射的 `.json`，列头与 `#145` `external_props` 解析器 **100% 同构兼容**；
  3. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDBOM [SUMMARY | "<输出.csv|.tsv|.json>" [ALLTABS]]`，并在 `examples/pid_convert.rs` 接入 `--export-schedule <FILE>`（最小真实 SPPID 图实测 `csv_rows = 29`，退出码 `0`）。

### 39.2 真实多源图纸「`PIDBOM ALLTABS` 提取清册 → 补填温压 → `PIDCONVERT PROPS` 双产物」往返实测

| 验证项 | 实测读数 | 说明 |
|---|---:|---|
| 扫描打开图纸 | `2` 张（`c0dd2cc4c75d` + `a0f05f81a655`） | 同时覆盖 SPPID 中位规模拆分图与原生 AutoCAD 存量图 |
| 去重清册总条目（`total_unique_rows`） | **`122` 条** | `Equipment = 7`、`Pipeline = 25`、`Instrument = 55`、`Valve = 35` |
| 工艺补填管线温压行数 | **`25` 条** | 在导出 CSV 的管线行填入保温规格、设计压力与设计温度 |
| `PROPS` 回填结果 | **`122` 行加载 / `85` 对象命中** | 自动更新 **`59` 个 `.PPT` 工艺点**、合规注入 **`103` 项属性**，安全过滤只读汇总列 `345` 次 |
| `StrictVendor` 与 `a/b` 级双产物验证 | **`302 / 302`（`100%`）** | `StrictVendor` **0 违规**、`class_default_fallback = 0`、`a_valid = true`、`b` 级 PDF 页内图元 `727` |

### 39.3 固定门禁读数

| 门禁项 | 改前读数（§38.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,224` 条（`7,150` 过 / `73` 忽略 / `1` 既有 `.pen`） | **`7,226` 条**（**`7,151` 过** / **`74` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 真实多源清册往返），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `bom.rs`、`mod.rs`、`pidprops.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 40. L3-3 / L3-5 / L3-7 / L6-2（#151）：应用内 P&ID 交付前设计规则一键体检、分类高亮与逐项巡检聚焦命令 `PIDDRC` 闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `f0c7199f0`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{drc,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-21-issue151/`（`red.txt`、`green_pidprops.txt`、`real_multi_source_drc.txt`、`real_out/real151_summary.json`、`fmt_arch_pytest.txt`、`clippy_clean_counts.json`） |

### 40.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `piddrc_detects_duplicates_orphan_nozzles_deadends_steps_camera_and_clears_after_fix`（构造重复位号、无位号阀门、孤立管口与悬空开放管段四类典型问题，校验 `PIDDRC` 分类选中、`PIDDRC NEXT` 视口聚焦以及 `PIDREVIEW DEDUP + ACCEPT` 后告警自动归零），运行按预期在 `PIDDRC EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`17 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/drc.rs`（`audit_sheet_drc`）：统一兼容未升格 `.dxf`/`.dwg` 与已升格 `PID_AUTHOR` 模型，串联五大工程体检项：`DuplicateTag`（`Error`）、`OneWayOpc`（`Error`）、`OrphanNozzle`（`Warning`）、`OpenPipeDeadEnd`（`Warning`）与 `UntaggedComponent`（`Warning`），给出 `ready_for_export = (error_count == 0)` 判定；
  2. 在 `src/app/commands/pidprops.rs` 注册并分发 `PIDDRC`：支持 `SUMMARY`、`ALL / ERRORS / WARNINGS / DUPLICATES / ONEWAYOPC / ORPHANNOZZLES / DEADENDS / UNTAGGED` 分类整组高亮选中、无状态游标 `PIDDRC NEXT [分类]`（逐项推进选中并将视口相机精准聚焦至问题图元坐标）及 `EXPORT "<json>"`。

### 40.2 真实双源工程图纸应用内 `PIDDRC` 体检与巡检实测

| 代号 | 样本类别 | 源图 SHA-256 前缀 | `ready_for_export` | `Errors` | `orphan_nozzles` | `open_pipe_dead_ends` | `untagged_components` | `PIDDRC NEXT` 巡检验证 |
|---|---|---|---:|---:|---:|---:|---:|---|
| `SP01` | `EXPORT_DXF`（SPPID 中位规模生产图） | `c0dd2cc4c75d` | `true` | `0` | `10` | `1` | `12`（与 §25 中置信度阀门数精确一致） | **通过** |
| `LG01` | `SOURCE_DWG`（原生 AutoCAD 散线存量图） | `135b6bf6b0a6` | `true` | `0` | `0` | `11` | `28` | **通过** |

### 40.3 固定门禁读数

| 门禁项 | 改前读数（§39.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,226` 条（`7,151` 过 / `74` 忽略 / `1` 既有 `.pen`） | **`7,228` 条**（**`7,152` 过** / **`75` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 真实多源 DRC 巡检），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `drc.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 41. L2-2 / L3-2 / L3-3 / L6-2（#152）：应用内跨标签页按位号/线号智能检索、整条管线全段高亮与视口秒级定位命令 `PIDFIND` 闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `7ac42d319`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{find,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-21-issue152/`（`red.txt`、`green_pidprops.txt`、`real_multi_source_find.txt`、`real_out/real152_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 41.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidfind_searches_tags_and_full_pipelines_across_tabs_and_focuses_camera`（构造未升格 DXF 与已升格 `PID_AUTHOR` 混合双标签页，验证跨标签页去连字符位号检索跳转、双段管线子串全段高亮选中与双行仪表圈跳转聚焦），运行按预期在 `PIDFIND ALLTABS` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`18 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/find.rs`（`search_pid_entities`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图与已升格 `PID_AUTHOR` 模型；支持四级精度排序（`0`=完全一致、`1`=忽略连字符/空格一致、`2`=前缀一致、`3`=子串命中）以及 `parse_sppid_line_number` 英制/公制规范化管线号双向等价匹配（如用清册公制线号 `150-PG-...` 精准命中图面英寸标注 `6"-PG-...`）；
  2. **整条管线连通 BFS 全段聚拢**：当命中管线号时，从标注种子管段出发按 `6.5 mm` 端点连通容差向相连管段自动传播，一键聚拢整条管线的全部管段图元并计算联合重心；
  3. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDFIND "<位号或线号关键字>" [ALLTABS] [NEXT]`，支持跨标签页自动切换、成员整组选中、视口相机平移聚焦及多匹配项无状态循环跳转。

### 41.2 真实三源异构工程图纸（SPPID DXF + 原生 AutoCAD DXF + 超大型 `.pid` 原图）多标签页跳转实测

| 目标标签页 | 样本形态 | 源图 SHA-256 前缀 | 跨标签页检索类型 | 自动切换与整组选中图元数 |
|---|---|---|---|---|
| `SP01` | 未升格 SPPID 拆分生产图 | `c0dd2cc4c75d` | 规范化工艺管线号 | **自动切换并全选 `6` 段相连管段** |
| `LG01` | 未升格原生 AutoCAD 存量图 | `a0f05f81a655` | 散圆仪表位号 | **自动切换并整组选中 `2` 个成员图元** |
| `PD01` | 已升格超大型真实 `.pid`（`5,576` 实体） | `ecc602494c4d` | 工艺设备位号 | **自动切换并整组选中 `4` 个多笔画成员图元** |

### 41.3 固定门禁读数

| 门禁项 | 改前读数（§40.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,228` 条（`7,152` 过 / `75` 忽略 / `1` 既有 `.pen`） | **`7,230` 条**（**`7,153` 过** / **`76` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 三源跨标签页检索），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `find.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 42. L3-3 / L3-4 / L2-2 / L6-2（#153）：应用内跨符号/跨标签页工艺流路拓扑连通追踪与起止点路径溯源命令 `PIDTRACE` 闭环（2026-11-21）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-21` |
| 基线提交 | `c12f5e348`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{trace,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-21-issue153/`（`red.txt`、`green_pidprops.txt`、`real_multi_tab_trace.txt`、`real_out/real153_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 42.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidtrace_traces_connected_subnet_and_cross_tab_opc_shortest_path`（构造双标签页跨图工艺流程 `P-1531A -> BV153101 -> 互逆 OPC -> BV153201 -> V-1532` 与同图孤立隔离阀 `BV999999`，验证单起点跨图连通子网审查、双位号跨图最短流路寻径、孤立支路不可达拦截及 `PIDPROMOTE` 升格前后一致性），运行按预期在 `PIDTRACE EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`19 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/trace.rs`（`trace_subnet_from_handles`、`trace_subnet_by_query`、`trace_shortest_path_by_query`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图与已升格 `PID_AUTHOR` 模型，串联管段端点/三通交点、行内阀门/仪表包围盒与端口、设备壳体/管口及多标签页互逆 `PIDOPC`（`Paired`）连接符构建无向拓扑图；
  2. **行内阀门 `6 mm` 端口间隙桥接避让与未裁剪贯通管段增强**：严格采用 `< 0.0` 非共线交叉判据，对 `2.0..=6.5 mm` 微断口桥接避开行内符号包围盒以确保最短流路必经在线切断阀，并同步捕获 AutoCAD 未裁剪直通管段上骑跨的在线阀门/仪表；
  3. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDTRACE [<起点位号> [TO] [<终点位号>]] [ALLTABS] [EXPORT "<json>"]`，支持跨标签页高亮流路图元、自动切换目标图并聚焦视口相机。

### 42.2 真实双标准成对工程图纸（SPPID 生产图对 + 原生 AutoCAD 存量图对）多标签页连通追踪实测

| 图纸组 | 样本形态 | 源图 SHA-256 前缀对 | 跨标签页数 / 互逆 `OPC` 桥 | 连通子网访问节点与管段 | 可达设备 / 阀门 / 仪表 / 管线 | 跨图双位号最短寻径 |
|---|---|---|---|---|---|---|
| `SP01 <-> SP02` | 未升格 SPPID 拆分生产图对 | `2164f88bbb57` + `fc2c812dc182` | `2` 图 / `2` 桥 | **`966` 节点**（`704` 管段） | 设备 `6` / 阀门 `59` / 仪表 `87` / 管线 `37` | **通过（`12` 步跨图序列）** |
| `LG01 <-> LG02` | 未升格原生 AutoCAD 存量图对 | `a0f05f81a655` + `28ed389e23cb` | `2` 图 / `6` 桥 | **`508` 节点**（`282` 管段） | 设备 `14` / 阀门 `0` / 仪表 `56` / 管线 `11` | **连通桥全通** |

### 42.3 固定门禁读数

| 门禁项 | 改前读数（§41.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,230` 条（`7,153` 过 / `76` 忽略 / `1` 既有 `.pen`） | **`7,232` 条**（**`7,154` 过** / **`77` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 四图跨标签页流路追踪），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `trace.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 43. L3-8 / L2-2 / L3-6 / L6-2（#154）：应用内 P&ID 新旧版次图纸语义级差异比对、分类巡检聚焦与变更清册导出命令 `PIDDIFF` 及 CLI `--diff-with` 闭环（2026-11-22）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-22` |
| 基线提交 | `ea4d276ea`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{diff,mod}.rs`、`src/app/commands/pidprops.rs`、`examples/pid_convert.rs` |
| 外部证据子项 | `2026-11-22-issue154/`（`red.txt`、`green_pidprops.txt`、`real_multi_source_diff.txt`、`cli_diff_with.txt`、`real_out/real154_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 43.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_154_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `piddiff_ignores_layout_shifts_detects_added_removed_modified_and_steps_camera`（构造 `R0` 基准版与 `RA` 当前版双标签页：验证泵符号纯排版平移零误报、删除阀门 `BV154102`、新增容器 `V-1549`、同序号管线等级由 `CS150` 升级至 `SS300`、页间连接符目标图号变更、`PIDDIFF ADDED / MODIFIED / NEXT` 视口聚焦以及 `PIDPROMOTE` 跨形态一致性），运行按预期在 `PIDDIFF EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`20 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/diff.rs`（`diff_pid_documents`、`diff_pid_files`、`write_pid_diff_file`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图与已升格 `PID_AUTHOR` 模型（含 `PidAuthor` 面板属性叠加），统一抽取主干名消除版次文件名后缀假差异，按精确规范化位号 + 同 `(fluid_code, seq_no)` 管线主干 + 同槽位页间连接符三级配对，精准归类 `Added`、`Modified`（逐字段 `old -> new` 明细）与 `Removed`，支持导出 `.json` 与 UTF-8 BOM `.csv` / `.tsv` 设计变更清册；
  2. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDDIFF "<基准图纸路径或已打开标签页>" [SUMMARY | ALL | ADDED | MODIFIED | NEXT] [EXPORT "<.json|.csv|.tsv>"]`，并在 `examples/pid_convert.rs` 新增 `--diff-with` 与 `--diff-out`（最小真实 SPPID 图自比对校验 `total_changes = 0`、`unchanged = 38`，退出码 `0`）。

### 43.2 真实双源工程图纸纯排版平移零误报与模拟升版校审实测

| 代号 | 样本类别 | 源图 SHA-256 前缀 | 纯排版微移误报数（基线未变对象数） | 模拟升版 `Added / Modified / Removed` | `.csv` 变更单与 `PIDDIFF NEXT` 巡检 |
|---|---|---|---:|---|---|
| `SP01` | `EXPORT_DXF`（SPPID 中位规模生产图） | `c0dd2cc4c75d` | **`0`**（`101` 项未变） | `1` 新增 / `1` 修改 / `14` 删除（`86` 未变） | **通过** |
| `LG01` | `SOURCE_DWG`（原生 AutoCAD 存量图） | `a0f05f81a655` | **`0`**（`47` 项未变） | `4` 新增 / `8` 修改 / `7` 删除（`32` 未变） | **通过** |

### 43.3 固定门禁读数

| 门禁项 | 改前读数（§42.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,232` 条（`7,154` 过 / `77` 忽略 / `1` 既有 `.pen`） | **`7,234` 条**（**`7,155` 过** / **`78` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 真实双源升版差异比对），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `diff.rs`、`mod.rs`、`pidprops.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 44. L3-3 / L3-4 / L2-2 / L6-2（#155）：应用内 P&ID 设备检修能量隔离边界（LOTO 首道关断阀清单与无阀直连预警）求解与隔离单导出命令 `PIDISOLATE` 闭环（2026-11-22）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-22` |
| 基线提交 | `f1c34a0d5`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{isolate,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-22-issue155/`（`red.txt`、`green_pidprops.txt`、`real_multi_tab_isolate.txt`、`real_out/real155_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 44.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_155_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidisolate_solves_loto_boundary_valves_across_tabs_and_flags_unvalved_hazards`（构造含本图入口阀 + 二级阀、跨标签页互逆 `OPC` 对端出口阀及无阀直连相邻容器的三支路泵系统，验证首道关断阀精准截断不误纳二级阀、无阀直连风险预警、补加切断阀并 `PIDPROMOTE` 后 `fully_isolatable = true` 收敛及跨标签页 `PIDISOLATE NEXT` 聚焦），运行按预期在 `PIDISOLATE EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`21 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/isolate.rs`（`solve_pid_isolation_from_handles`、`solve_pid_isolation_by_query`、`write_pid_isolation_file`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图与已升格 `PID_AUTHOR` 模型；自动区分可关断在线阀门（`IsoKind::Valve`）与被动导通管件/纯检测变送器（`REDUCER / FLANGE / STRAINER / PT` 等），沿相连管段及跨标签页互逆 `PIDOPC` 向外传播并在首道隔离阀处精准截断（同步支持 AutoCAD 未裁剪直通管段上骑跨阀门截断）；
  2. 自动检测三类隔离失效风险：`UnvalvedPeerEquipment`（无阀直连相邻设备）、`UnvalvedExternalOpc`（无阀直通外部未配对页间连接符）与 `UnvalvedOpenDeadEnd`（包络内无阀敞口管端），支持导出 `.json` 与 UTF-8 BOM `.csv` / `.tsv` 9 列 LOTO 检修能量隔离挂牌清册；
  3. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDISOLATE [<设备位号>] [ALLTABS] [SUMMARY | VALVES | ENVELOPE | HAZARDS | ALL | NEXT] [EXPORT "<.json|.csv|.tsv>"]`。

### 44.2 真实双标准成对工程图纸（SPPID 生产图对 + 原生 AutoCAD 存量图对）多标签页设备隔离审查实测

| 图纸组 | 样本形态 | 源图 SHA-256 前缀对 | 跨标签页数 | 首道关断阀数（`boundary_valves`） | 包络管段数 | 无阀风险（相邻设备 / 外部 OPC / 敞口） |
|---|---|---|---:|---:|---:|---|
| `SP01 <-> SP02` | 未升格 SPPID 拆分生产图对 | `2164f88bbb57` + `fc2c812dc182` | `2` 图 | **`80` 台** | `568` 段 | `13` 处（设备 `5` / 外部 OPC `8` / 敞口 `0`） |
| `LG01 <-> LG02` | 未升格原生 AutoCAD 存量图对 | `a0f05f81a655` + `28ed389e23cb` | `1` 图 | **`8` 台** | `8` 段 | `2` 处（设备 `2` / 外部 OPC `0` / 敞口 `0`） |

### 44.3 固定门禁读数

| 门禁项 | 改前读数（§43.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,234` 条（`7,155` 过 / `78` 忽略 / `1` 既有 `.pen`） | **`7,236` 条**（**`7,156` 过** / **`79` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 四图跨标签页设备隔离审查），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `isolate.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 45. L3-2 / L3-6 / L2-2 / L6-2（#156）：应用内 ISA-5.1 自控仪表回路跨标签页自动聚合、信号链校核与回路索引表导出命令 `PIDLOOP` 及 CLI `--export-loops` 闭环（2026-11-22）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-22` |
| 基线提交 | `38b905764`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{loops,mod}.rs`、`src/app/commands/pidprops.rs`、`examples/pid_convert.rs` |
| 外部证据子项 | `2026-11-22-issue156/`（`red.txt`、`green_pidprops.txt`、`real_multi_tab_loops.txt`、`cli_export_loops.txt`、`real_out/real156_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 45.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_156_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidloop_aggregates_isa_loops_across_tabs_detects_incomplete_and_steps_camera`（构造跨双标签页的完整闭环回路 `02013` = `PT-02013 + PIC-02013 + PV-02013`、缺主变送元件的残缺回路 `02099` = `LIC-02099 + LV-02099` 以及手动球阀干扰项 `BV0201399`，验证跨图回路聚合、手动阀过滤、`INCOMPLETE` 残缺回路识别、跨标签页 `PIDLOOP NEXT` 视口跳转聚焦及补齐 `LT-02099` + `PIDPROMOTE` 后清零残缺项），运行按预期在 `PIDLOOP EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`22 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/loops.rs`（`parse_isa_instrument_tag`、`analyze_pid_loops`、`extract_loops_from_batch_dir`、`write_pid_loops_file`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图、已升格 `PID_AUTHOR` 模型与 `_Data.xml` 批处理目录；按 ISA-5.1 拆解被测变量、三类功能角色（`Sensor / Controller / FinalElement`，严格排除手动工艺阀门）与回路号，自动校核信号链完整性（`CompleteClosedLoop / MonitoringLoop / Incomplete`），导出 `.json` 与 UTF-8 BOM `.csv` / `.tsv` 10 列自控仪表回路索引表；
  2. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDLOOP [<回路号或仪表位号>] [ALLTABS] [SUMMARY | INCOMPLETE | COMPLETE | ALL | NEXT] [EXPORT "<.json|.csv|.tsv>"]`，并在 `examples/pid_convert.rs` 新增 `--export-loops <FILE>`（最小真实 SPPID 图实测导出 `4` 行回路索引表，退出码 `0`）。

### 45.2 真实双标准成对工程图纸（SPPID 生产图对 + 原生 AutoCAD 存量图对）多标签页回路聚合实测

| 图纸组 | 样本形态 | 源图 SHA-256 前缀对 | 归组仪表总数 | 聚合 ISA-5.1 回路数 | 完整闭环回路 / 监测回路 / 残缺误报数 | `.csv` 索引表与 `PIDLOOP NEXT` |
|---|---|---|---:|---:|---|---|
| `SP01 + SP02` | 未升格 SPPID 拆分生产图对 | `2164f88bbb57` + `fc2c812dc182` | **`93` 台** | **`17` 个** | `5` 闭环 / `12` 监测 / **`0` 残缺** | **通过** |
| `LG01 + LG02` | 未升格原生 AutoCAD 存量图对 | `a0f05f81a655` + `28ed389e23cb` | **`84` 台** | **`28` 个** | `0` 闭环 / `28` 监测 / **`0` 残缺** | **通过** |

### 45.3 固定门禁读数

| 门禁项 | 改前读数（§44.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,236` 条（`7,156` 过 / `79` 忽略 / `1` 既有 `.pen`） | **`7,238` 条**（**`7,157` 过** / **`80` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 四图跨标签页回路聚合），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `loops.rs`、`mod.rs`、`pidprops.rs`、`examples/pid_convert.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |

---

## 46. L3-3 / L3-4 / L3-6 / L6-2（#157）：应用内 P&ID 管道等级与公称直径一致性校核（漏画变径器 / 跨等级无分界直连 / 跨图 OPC 规格失配）及巡检命令 `PIDSPEC` 闭环（2026-11-22）

| 项 | 值 |
|---|---|
| 测量日期 | `2026-11-22` |
| 基线提交 | `0df907b00`（`main`） |
| `rustc -V` | `rustc 1.102.0-nightly (65ce8611f 2026-11-18)` |
| 语料清单摘要 | `manifest_sha256 = 487b320319a9dfad20b1f9b70c8647ad2885ece4d2f8d3a217f71c88b9112038` |
| 新增 / 修改文件 | `src/io/pid_convert/{spec_check,mod}.rs`、`src/app/commands/pidprops.rs` |
| 外部证据子项 | `2026-11-22-issue157/`（`red.txt`、`green_pidprops.txt`、`real_multi_tab_spec.txt`、`real_out/real157_summary.json`、`fmt_arch_pytest.txt`、`rust_gates_status.json`） |

### 46.1 先红后绿与设计摘要

- **先红阶段**（`red.txt`，`RED_157_RC=101`）：在 `src/app/commands/pidprops.rs` 新增 `pidspec_detects_missing_reducers_spec_breaks_and_opc_mismatches_then_clears_after_fix`（构造合法三通异径支管豁免对照、`DN100->DN50` 1-对-1 直连缺变径器、`CS150->SS300` 无分界跨等级直连以及跨双标签页互逆 `OPC` 两端规格失配四类场景，验证精准检出、分类高亮、`PIDSPEC NEXT` 聚焦以及补齐 `CONCENTRIC REDUCER` / 分界阀门 / 修正对端线号并 `PIDPROMOTE` 后告警全部清零），运行按预期在 `PIDSPEC EXPORT` 断言处变红。
- **转绿阶段**（`green_pidprops.txt`，`23 passed; 0 failed`）：
  1. 新增 `src/io/pid_convert/spec_check.rs`（`audit_pid_spec_consistency`、`write_pid_spec_file`）：统一兼容未升格 `.dxf`/`.dwg`、未升格 `.pid` 原图与已升格 `PID_AUTHOR` 模型，基于 `parse_sppid_line_number` 公制/英制规范化直径与管道材质等级拆解，串联检测 `MissingReducer`（自动豁免 `>=3` 管段汇交或主管内插三通支管的合法异径三通）、`UnmarkedSpecBreak`（区分仅变径的大小头与可分界等级的阀门/法兰/设备）与 `OpcSpecMismatch`（多标签页互逆 `PIDOPC` 两端接续管径/等级对账），支持导出 `.json` 与 UTF-8 BOM `.csv` / `.tsv` 配管规格一致性校核报告；
  2. 在 `src/app/commands/pidprops.rs` 注册实现 `PIDSPEC [SUMMARY | ALL | REDUCERS | SPECBREAKS | OPCMISMATCH | NEXT] [ALLTABS] [EXPORT "<.json|.csv|.tsv>"]`。

### 46.2 真实双标准成对工程图纸（SPPID 生产图对 + 原生 AutoCAD 存量图对）多标签页规格对账实测

| 图纸组 | 样本形态 | 源图 SHA-256 前缀对 | 带线号管段数 | `missing_reducers` | `unmarked_spec_breaks` | `opc_spec_mismatches` | `.csv` 报告与 `PIDSPEC NEXT` |
|---|---|---|---:|---:|---:|---:|---|
| `SP01 + SP02` | 未升格 SPPID 拆分生产图对 | `2164f88bbb57` + `fc2c812dc182` | **`390` 段** | **`0`** | **`0`** | `1` | **通过** |
| `LG01 + LG02` | 未升格原生 AutoCAD 存量图对 | `a0f05f81a655` + `28ed389e23cb` | **`282` 段** | `1` | **`0`** | `6` | **通过** |

### 46.3 固定门禁读数

| 门禁项 | 改前读数（§45.3） | 改后实测 | 结论 |
|---|---|---|---|
| `cargo test --offline --locked --lib` | `7,238` 条（`7,157` 过 / `80` 忽略 / `1` 既有 `.pen`） | **`7,240` 条**（**`7,158` 过** / **`81` 忽略** / **`1` 同一既有 `.pen` 字重失败**） | `+2` 条 Rust 测试（`+1` 常规 + `+1` 四图跨标签页配管规格对账），**0 新增失败** |
| `cargo clippy --offline --locked --lib` | `656` 条警告 | **`656` 条警告** | **0 新增警告** |
| `cargo clippy --offline --locked --lib --profile test` | `1,051` 条警告 | **`1,051` 条警告** | **0 新增警告** |
| `scripts\fmt-newhunks.ps1` | — | `spec_check.rs`、`mod.rs`、`pidprops.rs` 全部 **`new=0`** | **0 格式偏差** |
| `python scripts/arch_check.py report` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | `33 module pairs, 183 files`，`io` 四行 `3, 1, 1, 1` | **0 新增向上依赖** |
| `python -m pytest scripts/sppid -q -p no:cacheprovider` | `202 passed` | **`202 passed`** | **全绿** |
