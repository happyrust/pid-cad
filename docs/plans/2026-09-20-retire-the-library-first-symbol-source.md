# 退役 `OCS_PID_SYMBOL_SOURCE=library` · 下一轮待办（2026-09-20 开单）

> 承接 `2026-09-19-draw-the-cached-body-first-and-the-library-only-when-the-drawing-carries-none.md` 的 P-D3：开关**留一轮**，
> 「下一轮若没人用就退役」。那份计划 2026-09-19 关闭（OCS `641cec3b` / `ef93f6ba` / `1ff0f4af` / `3ce6662e`，pid-parse `08fc95a` /
> `b6a70a7`），本单把退役要动的东西一次列清，到时候照单做。
> **状态：已退役（2026-09-20，OCS `fbcd321b`，会话 fable-5-1-8）。** 用户在逐笔样式计划（`2026-09-20-a-cached-body-carries-its-own-stroke-styles.md`）
> 收口后直接指示开本单；开单条件第 2 条（P-D8 截图）按用户指示**不等**，在此登记。进度见文末。

## 开单条件（两条都满足才动）

1. **一轮过去没人用**：没有用户 / 会话报告过设 `OCS_PID_SYMBOL_SOURCE=library` 才看得对的图。判据：问用户一句；日志里那行
   `OCS_PID_SYMBOL_SOURCE=library, symbol placements draw the library body first…` 一轮里没在任何交付记录出现。
2. **P-D8 的补充验收要么做了要么明确放弃**：SmartPlant 截图对 Ball Valve Type 1 / Remarks / Item Note & Label 三例的比对——那份计划说
   「有出入则回到 P-D3 的开关并登记」，开关就是为这一步留的退路。截图还没有就问用户是等还是不等；不等就在本单登记一句再动。

## 要拆的（OCS）

### `src/io/pid.rs`

| 现在 | 退役后 |
|---|---|
| `SYMBOL_SOURCE_ENV`、`PidSymbolSource { Cache, Library }` 及 `from_env` / `parse` / `strokes` | 全删。`strokes` 的 Cache 分支（`visible_primitives()`）就地内联到调用处 |
| `PidImportOptions { layer_mode, symbol_source }`、`load_pid_with_options` | 删 `symbol_source`。`PidImportOptions` 只剩一项时把 `load_pid_with_options` 折回 `load_pid_with_layer_mode`（K2 之前的口径），或保留结构体给以后的选项——到时候看还有没有第二个选项要进来 |
| `load_pid_with_options` 里 `source == Library` 的那行 info 日志 | 删 |
| `BodySources { cached, library, source }` | 删 `source`；`build_entities` 的 `match source` 只剩 Cache 分支：`cached_body_entities` → `library_body_entities` → 标记圆 |
| `cached_body_entities(body, source, …)` | 去掉 `source`，恒取 `visible_primitives()`；`hidden_strokes_skipped` 计数**保留**（仍是有意义的日志数） |
| `PlacementMeasures::of(kind, geometry, source, …)` | 去掉 `source`，恒量可见笔画 |
| `SymbolBodies` / `ImportSummary` 的 `cache_bodies` / `library_bodies` / `hidden_strokes_skipped` | **保留**——库仍是补位路径，三个数照旧进日志 |
| 单测 `the_source_decides_which_cached_strokes_are_drawn` | 改名去掉「source」，只钉 Cache 的那一半（`Default` 开 / `Construction` 关 / 无显示位的层画 / 无层条目的图元画） |

### `tests/pid_import.rs`

| 测试 / 辅助 | 处置 |
|---|---|
| `import_from` / `import_from_library` / `import_from_cache` / `import_with` | 删前两个；`import_from_cache` 折回 `import`（默认导入已是缓存）；`import_with` 视 `PidImportOptions` 去留 |
| `import_without_library(name, source)` | 去掉 `source` 参数 |
| `the_symbol_source_defaults_to_the_cache_and_names_its_two_sources` | **删** |
| `the_two_symbol_sources_differ_only_in_the_body_drawn` | **删**。它里面唯一还有价值的是 P-D7 那张「11 个放置 `extent=` 因关闭层笔画而变」的名单——退役前把 cache 侧的 11 个值挪进 `a_placement_draws_the_body_the_drawing_carries_and_skips_its_hidden_layers` 钉住（只钉现值，不再对比整体值），名单才不丢 |
| `a_placement_without_a_library_body_draws_the_body_the_drawing_carries` | 删后半段「旧序：library 源下 [1.27, 1.59, 1.59, 6.35, 7.57] / [1.27, 1.59, 6.35, 7.57] 且两路不等」；前半段「有库无库相同」保留 |
| `a_symbols_bspline_lip_reaches_the_drawing_from_either_body` | 「两路」前提消失：删 library 源下的库 vs 缓存比对，只留「默认导入画出缓存本体的 S1 曲线、采样长度 1.5～3.0 mm」；改名去掉 `from_either_body` |
| `a_symbols_lettering_follows_its_placement_colour_not_its_syms` | 语料上默认导入符号层**零文字**，颜色规则今天只能靠 library 源钉。退役后两条路：（a）把颜色规则改成 `pid.rs` 里 `apply_symbology` 的单测——合成一条 `PID-SYMBOL` 层上的 `Text` 与一个放置样式，断言文字取放置颜色、不带线宽；（b）整条删。**推荐 (a)**：`apply_symbology` 的文字分支是真实代码路径（库补位时会走到），不该失去测试 |
| `a_symbol_body_draws_in_the_style_its_placement_names` 等已按缓存重钉的 | 不动 |

### 其它

- user-guide「符号从哪来」段：删「要回到旧的『先库后缓存』顺序……这个开关只保留一轮」那两句；「符号的两个尺寸」段删
  「`OCS_PID_SYMBOL_SOURCE=library` 下仍按整个缓存本体量」。
- 09-19 计划：头部与 P-D3 行补一句「已于 <日期> 退役（OCS `<hash>`）」；结算「还开着的」第一条划掉。
- 共享记忆：`mem-99` 里写着开关名与「留一轮」，退役时 `remember` 一条新的替代它（`supersedesId: mem-99`）。
- `i18n`：本轮没有为开关加词条，退役无词条要删。

## 验收

`pid_import` 在 `OCS_PID_LAYER_MODE` 两值下全绿（测试数 54 减去删掉的两条、加回挪进去的名单，到时候记实数）；`--lib` `io::pid::tests`
与 `i18n` 目录守护全绿；`rg OCS_PID_SYMBOL_SOURCE|PidSymbolSource|import_from_library` 在 `src/` `tests/` `docs/user-guide.md` 零命中
（只剩计划文档里的历史记述）；`clippy --lib --test pid_import` 触碰处零告警。时间盒半个工作日。

## 登记不做

| 项 | 理由 |
|---|---|
| 把开关改成导入选项 / UI 保留下来 | 09-19 计划已登记不做：与 `OCS_PID_LAYER_MODE` 同一口径，环境变量一轮即退 |
| 顺手退役 `OCS_PID_LAYER_MODE` | 另一条线（09-07 计划遗留，等 DXF 下游消费方定用法），不混进来 |

## 进度（2026-09-20，OCS `fbcd321b`）

照「要拆的」做，两处按单上留的选择落笔：

- `PidImportOptions` / `load_pid_with_options` **删**，折回 `load_pid_with_layer_mode`（没有第二个选项要进来）；测试侧 `import_with` 换成
  `import_in_mode(name, PidLayerMode)`，`import_from_cache` 全部折回 `import`，`import_without_library` 去掉 `source`。
- `a_symbols_lettering_follows…` 走 **(a)**：颜色规则改成 `io::pid::tests::a_symbols_lettering_takes_its_placements_colour_and_nothing_else`
  （合成 `PID-SYMBOL` 上的 `Text` + `#008000` 放置样式 → 取放置颜色、不带线宽、字高字面不动；`PID-TEXT` 上的不碰）；集成测试留下缓存那一半，
  改名 `a_cached_bodys_lettering_stays_on_its_switched_off_layer`。
- P-D7 的 11 个放置名单挪进 `a_placement_draws_the_body_the_drawing_carries_and_skips_its_hidden_layers` 第 5 段（只钉现值，旧的整体值写在注释里）。
- `pid.rs`：`SYMBOL_SOURCE_ENV` / `PidSymbolSource`（含 E2 刚加的 `styled_strokes`）/ `PidImportOptions` 全删，缓存路径恒取 `visible_strokes()`、
  `PlacementMeasures` 恒量 `visible_primitives()`；`BodySources` 去 `source`；`build_entities` 只剩缓存 → 库 → 标记圆；那行 info 日志删；
  `cache_bodies` / `library_bodies` / `hidden_strokes_skipped` 保留。单测 `the_source_decides…` 改名
  `a_cached_body_draws_only_the_strokes_on_its_displayed_layers`，顺带钉 `cached_body_entities` 的计数（1 个本体、2 笔跳过）。
- user-guide 两处删；09-19 计划头部 / P-D3 行 / 结算第一条改标；`remember` 一条替代 `mem-99`。

验证：`--lib io::pid::tests` 9 → **10**；`pid_import` 55 → **53**（删 `the_symbol_source_defaults…`、`the_two_symbol_sources_differ…`
两条），`OCS_PID_LAYER_MODE` 两值全绿；`rg OCS_PID_SYMBOL_SOURCE|PidSymbolSource|import_from_library` 在 `src/` `tests/` `docs/user-guide.md`
零命中；`rustfmt --check` 两文件干净；`clippy --lib --test pid_import` 触碰处零命中。

## 门禁记录

- 2026-09-20：开单（本提交，会话 fable-5-1-30），待排期。
- 2026-09-20：用户指示直接退役（不等 P-D8 截图）；落地 OCS `fbcd321b`（会话 fable-5-1-8）。
