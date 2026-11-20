# 任务清单：导入的图例带上类型和位号，标注文字和图例配对

> 日期：2026-11-20。状态：**执行中**。规格已确认（2026-11-20，Q1–Q5 都按推荐）；SI-1 做完，其余没有开始。
> 配套：规格 `SPEC-pid-symbol-identity.md`（下文简称「规格」）。
> 分支 `pid-symbol-identity`，起点是图块分支验收的那个提交 `1855aaae5`；一个任务一个本地提交，不推送。

## 0. 怎么读这份清单

写法和图块计划的清单（`docs/plans/2026-11-19-pid-symbol-block/todo.md`）相同：每个任务写做什么、验收、依赖、文件、规模（S 1–2 个文件，M 3–5 个，L 更多）；做完后在任务下面加一段「做完」，写读数、没验证的、与清单不一样的地方。

每个任务都要过图块计划清单 §1 的十条固定门禁，用同一个脚本跑。与规格不一致的地方记进规格（到时加一节 §11）。

转换库不改，锁定的版本不换（`96ee00c`）。

## 1. 任务

### SI-1 导入器读对象，块引用写上类型和位号 【接线】

- **做什么**：导入时调一次 `load_publish_from_pid`，建「图形编号 → 对象」的表；每个图例的块引用写 `graphic`，有对象时写 `item`、`item_type`、`class`、`resolved=item`，按规格 §4.2 的第 2–4 级写 `label`、`tag_source`；导入摘要加 `symbol_items`、`symbol_tags`。
- **验收**：
  - [x] 规格 §6 第 1 条（样例只带 `.pid`）。
  - [x] 规格 §6 第 6 条（样例带 `_Data.xml`：原有十个键不变；两边的发布类逐个比，不同的列出来）。
  - [x] 读不出对象的图导入成功，图例上只有 `graphic`。
  - [x] 规格 §6 第 9 条的前半句和第 10、12 条（结构、图例识别、耗时预算不变）。
  - [x] 图元一级的数量出来，和规格不同的逐项说明（规格 §6 第 5 条）。
  - 没有量，留给 SI-7：规格 §6 第 9 条的后半句（出图与图块分支逐字节相同）。原先和上面第 4 条写在一起，做的时候拆开了。
- **依赖**：无。
- **文件**：`src/io/pid/items.rs`（新）、`src/io/pid/build.rs`、`src/io/pid/metadata.rs`、`src/io/pid/summary.rs`、`src/io/pid/mod.rs`、`src/io/pid/tests.rs`、`tests/pid_import.rs`。
- **规模**：L。

**做完**（2026-11-20）

- 改了什么（产品代码 +334 −2）：
  - `src/io/pid/items.rs`（新文件，产品部分 134 行）：`SheetItems::read` 调一次转换库的 `load_publish_from_pid`，建「图形编号 → 对象」的表。每个对象留四样：编号、种类、发布类、名字。名字取对象 `IObject` 接口的 `Name` 属性，去掉首尾空白，空的算没有。表示记录没写对象、或写的对象不在这张图的对象里，这个图形不进表；一个图形有几条表示记录时取第一条。读不出来（图纸不写图号、文件打不开）不算导入失败：写一行日志说原因，交回空表。
  - `src/io/pid/metadata.rs`（+147）：`attach_symbol_identity` 往块引用的记录里写下面「写进记录的规则」那几个键，交回这个引用最后有没有 `label`；`stamp_pid_pairs` 把键按固定的先后写进一条已有的记录；`pid_value` 按整个键名读一个值。
  - `src/io/pid/build.rs`（+23 −1）：`build_document_entities` 多一个参数（对象表）；每个块引用写完 `symbol=` 之后调 `attach_symbol_identity`；`Built` 多两个数。
  - `src/io/pid/mod.rs`（+14）：`load_pid_with` 在建图元之前读一次对象表；`finish` 把两个数填进摘要，另写一行日志。
  - `src/io/pid/summary.rs`（+16 −1）：`ImportSummary` 加 `symbol_items`、`symbol_tags`，排在最后；`counts()` 从 20 项变 22 项，原来 20 项的先后不动。`store`、`load`、`take` 照 `counts()` 走，没有改。
- 写进记录的规则（块引用，`role=symbol`）：
  - 图形有编号的都写 `graphic`。
  - 表里有它的对象：写 `item`、`item_type`。
  - 记录里没有 `resolved`（`_Data.xml` 没有对上这个图形），而图纸文件定得出发布类：写 `class` 和 `resolved=item`。
  - 记录里没有 `label`，而图纸文件给了名字：写 `label` 和 `tag_source=file`。
  - `_Data.xml` 写的 `class`、`label`、`oid`、`resolved` 一个不动；块里的成员什么都不加。
  - 键的先后固定（一张 17 个键的表）：原有十二个键的次序不动，新键排在 `symbol` 之后；图纸文件给的 `class`、`label`、`resolved` 写在 `_Data.xml` 写它们的位置上。

  和规格的写法不完全一样的地方（键的先后、`class` 按「没有 `resolved`」判断、`_Data.xml` 对上了但没给名字的图例）记在规格 §11 的 SI-1。
- 摘要的两项：`symbol_items` 是图纸文件给得出对象的块引用数；`symbol_tags` 是最后带 `label` 的块引用数，不管 `label` 是 `_Data.xml` 给的还是图纸文件给的。两项只进日志，随文档过打开流程后取走；命令行、诊断面板、备份审计、批量报告这几处照图块计划 SB-5 的结论不动。
- 简称。
  - 单元测试，`src/io/pid/items.rs` 里新加的：I1 是 `a_graphic_is_joined_to_the_item_its_representation_names`，I2 是 `a_file_the_reader_gives_nothing_for_yields_no_items`。
  - 单元测试，`src/io/pid/tests.rs` 里新加的：R1 是 `the_reference_states_the_item_its_graphic_draws`，R2 是 `a_reference_states_only_what_the_file_decides`，R3 是 `what_data_xml_published_stands_and_the_file_adds_what_it_left_out`，R4 是 `the_summary_counts_the_references_that_state_an_item_and_a_tag`，K 是 `a_key_stamped_later_takes_its_place_in_the_record`。
  - 单元测试，原有的：RT 是 `the_summary_round_trips_through_its_properties_and_take_removes_them`；E1–E4 是下面「改了期望的原有测试」的第 2–5 条。
  - `tests/pid_import.rs` 里新加的：S1 是 `the_sample_references_state_the_items_the_file_gives_them`，S2 是 `a_sheet_whose_items_cannot_be_read_imports_with_graphics_alone`。原有的：SUM 是 `the_sample_summary_counts_what_it_counted_before_symbols_were_blocks`，RIDE 是 `the_summary_rides_the_document_once_and_is_never_saved`。
- 钉住的（I、R、K 都是代码里拼的数据）：
  - I1：三个对象、七条表示记录。图形 612 对上阀门，编号、种类、发布类照原样，名字 `"  GV-101 "` 去掉空白是 `GV-101`，别的接口的 `Name` 和 `IObject` 的别的属性都不当名字；613 对上 OPC，没有发布类，全是空白的名字算没有；614 的对象没有 `Name` 属性，没有名字；没写对象的 615、对象不在表里的 616 不进表；612 的第二条表示记录不算；同一个对象画在两个图形上，两个图形都查得到；表里一共 4 个图形。
  - I2：读一个不存在的文件，交回空表，不报错。
  - R1：一个阀，图纸文件给了对象、发布类、名字。块引用的记录逐条是 `sheet_layer`、`sheet_layer_oid`、`role=symbol`、`style`、`extent`、`class=PIDPipingComponent`、`label=GV-101`、`resolved=item`、`symbol=Ball Valve`、`graphic=612`、`item=…`、`item_type=PipingComp`、`tag_source=file`，共 13 条；块里 5 个成员各只有 `role`；两个数是 1、1。
  - R2：四个阀——一个 OPC（没有发布类、没有名字），一个有发布类没有名字的管件，一个表里没有的图形，一个没有图形编号的记录。四条记录逐条写了期望：OPC 只多 `graphic`、`item`、`item_type`；管件另有 `class` 和 `resolved=item`，没有 `label`、`tag_source`；第三个只多 `graphic`；第四个什么都不多。两个数是 2、0。
  - R3：旁边有一份 `_Data.xml` 的三个阀。`_Data.xml` 给了类和位号的：`class`、`label`、`oid`、`resolved=direct` 照旧，图纸文件里不一样的名字不用，没有 `tag_source`，只多 `graphic`、`item`、`item_type`。`_Data.xml` 给了类没给位号的：`class`、`oid`、`resolved=direct` 照旧，`label` 用图纸文件的名字，带 `tag_source=file`。`_Data.xml` 里没有的：`class` 加 `resolved=item`，`label` 加 `tag_source=file`，没有 `oid`。两个数是 3、3。
  - R4：三个阀，两个有对象，其中一个有名字。`counts()` 的后四项是 `symbol_blocks` 3、`symbol_definitions` 1、`symbol_items` 2、`symbol_tags` 1；按字段读是 2、1；存进文档的两个键是 `2`、`1`；`take` 取回来的摘要与存进去的相等。
  - K：一条已有三个键的记录，分两次写进七个键，读出来是固定的先后；再写一次数据库的位号——`label` 在原位换值，`tag_source` 在原位换值，`label_in_file` 排在它后面，空值的 `oid` 不写，不认识的两个键按来的先后排在最后。按整个键名读值：`label` 读不到 `label_in_file` 的值，`label_in` 读不到东西。这条记录在实体的几条记录里位置不变，别的程序的记录不动。记录里不是 `键=值` 的值留在原位，新键绕着它落位。没有东西可写时不建记录；实体没有记录时建一条。
  - S1、S2、SUM：见下面「样例读数」和「读不出对象的图」。
- 先红：
  - 单元测试（14:42–14:48，`chainred.txt`）：这时接线已经写好，对象表和 `metadata.rs` 的两个函数还是空壳（不读、不写）。`--lib io::pid::` 84 个：通过 76，失败 7，忽略 1。失败的是 I1、K、R1、R2、R3、R4，和原有的 `the_summary_counts_the_references_and_the_definitions_they_refer_to`（它读 `counts()` 第 18 项之后的全部，这时多出了两项）。I2 这一遍是过的：空壳本来就交回空表。
  - 样例测试（15:00–15:04，`sample-red.log`）：照规格 §7 的做法，实现之后把期望的数留空跑一遍，从失败信息里读出数再填。`sample_sheet` 8 个：通过 5，失败 3，是 S1、S2、SUM。这三个没有在缺实现的时候红过，会变红靠下面的改坏点。
- 改了期望的原有测试 8 处（没有删测试，没有把相等放宽）：

  | # | 测试 | 原来 | 现在 | 原因 |
  |---|---|---|---|---|
  | 1 | `tests.rs` 的 `the_summary_counts_the_references_and_the_definitions_they_refer_to` | `counts()[18..]` 等于那两项 | `counts()[18..20]` 等于同样的两项 | 后面多了两项 |
  | 2 | `tests.rs` 的 `the_reference_stands_at_the_insertion_point_and_says_what_the_placement_said`（E1） | 块引用的整条记录，末尾没有 `graphic` | 末尾多 `graphic=612`；断言的说明补了半句 | 规格 §3：有编号的图例都写 `graphic` |
  | 3 | `tests.rs` 的 `the_published_identity_is_on_the_reference_and_not_in_the_block`（E2） | 同上 | 末尾多 `graphic=612` | 同上 |
  | 4 | `tests.rs` 的 `a_placement_with_no_body_is_a_block_of_its_marker_dot`（E3） | 同上，两个图例 | 末尾各多 `graphic=701`、`graphic=702` | 同上 |
  | 5 | `tests.rs` 的 `a_label_that_draws_only_its_name_is_a_block_of_that_name`（E4） | 同上 | 末尾多 `graphic=801` | 同上 |
  | 6 | `src/app/pid_symbol_block_tests.rs` 的 `legend_recognition_leaves_the_symbols_of_an_import_alone` | 同上，第一个阀 | 末尾多 `graphic=701` | 同上 |
  | 7 | `tests/pid_import.rs` 的 `role_and_class_are_separate_keys_with_disjoint_vocabularies` | 认得的键 11 个 | 15 个，加 `graphic`、`item`、`item_type`、`tag_source` | 记录里多了这四个键 |
  | 8 | `tests/pid_import.rs` 的 SUM | 20 项 | 22 项，多 `symbol_items` 592、`symbol_tags` 231；前 20 项的值没有改 | 摘要多了两项 |

  另外，`tests.rs` 开头那条摘要存取往返的测试（RT）给两个新字段填了 25、11；`src/io/pid/template.rs` 的一处测试调用多传一个空的对象表。
- 故意改坏 37 处，用环境变量打开。单元测试和 `tests/pid_import.rs` 各编一次，把测试程序拷出来，不开的各跑一遍（83 个、63 个全过），每处各跑一遍（15:05–15:14，`libmut-m1.txt`、`mut-m1.txt`）。每一处都至少有一个测试变红。

  | # | 改坏 | 改在哪 | `--lib io::pid::` 83 个里变红 | `tests/pid_import.rs` 63 个里变红 |
  |---|---|---|---|---|
  | 1 | 名字从任何接口的 `Name` 取 | `items.rs` | I1 | — |
  | 2 | 名字取 `IObject` 的第一个属性，不管叫什么 | `items.rs` | I1 | — |
  | 3 | 名字不去首尾空白 | `items.rs` | I1 | — |
  | 4 | 全是空白的名字也算名字 | `items.rs` | I1 | — |
  | 5 | 读到了对象却交回空表 | `items.rs` 的 `read` | — | S1 SUM |
  | 6 | 读不出来时崩溃，不交回空表 | `items.rs` 的 `read` | I2 | S2 |
  | 7 | 建表时什么都不放 | `items.rs` 的 `of` | I1 | S1 SUM |
  | 8 | 没写对象的表示记录算到第一个对象头上 | `items.rs` 的 `of` | I1 | — |
  | 9 | 对象不在本图里的表示记录算到另一个对象头上 | `items.rs` 的 `of` | I1 | — |
  | 10 | 一个图形有几条表示记录时取最后一条 | `items.rs` 的 `of` | I1 | — |
  | 11 | 导入时不读对象表 | `mod.rs` | — | S1 SUM |
  | 12 | 摘要两项填反 | `mod.rs` 的 `finish` | R4 | SUM |
  | 13 | 摘要两项不填 | `mod.rs` 的 `finish` | R4 | SUM |
  | 14 | 不认识的键排到最前 | `stamp_pid_pairs` | K | — |
  | 15 | 没有东西可写时也建一条空记录 | `stamp_pid_pairs` | K | — |
  | 16 | 记录挪到实体各条记录的最后 | `stamp_pid_pairs` | K | — |
  | 17 | 空值也写 | `stamp_pid_pairs` | K | — |
  | 18 | 已有的键不换值，再写一条 | `stamp_pid_pairs` | K | — |
  | 19 | 新键一律追加在末尾 | `stamp_pid_pairs` | K R1 R2 R3 | — |
  | 20 | 不是 `键=值` 的值也参与排位 | `stamp_pid_pairs` | K | — |
  | 21 | 新键排到同位次的键前面 | `stamp_pid_pairs` | K | — |
  | 22 | `_Data.xml` 给了类，也写图纸文件的类 | `attach_symbol_identity` | R3 | — |
  | 23 | `_Data.xml` 给了位号，也写图纸文件的名字 | `attach_symbol_identity` | R3 | — |
  | 24 | 不写 `graphic` | `attach_symbol_identity` | R1 R2 R3 E1 E2 E3 E4 | S1 S2 |
  | 25 | 不写 `item` | `attach_symbol_identity` | R1 R2 R3 | S1 |
  | 26 | 不写 `item_type` | `attach_symbol_identity` | R1 R2 R3 | S1 |
  | 27 | 写 `class` 时不写 `resolved=item` | `attach_symbol_identity` | R1 R2 R3 | S1 |
  | 28 | 写 `label` 时不写 `tag_source=file` | `attach_symbol_identity` | R1 R3 | S1 |
  | 29 | 总是答「没有位号」 | `attach_symbol_identity` | R1 R3 R4 | SUM |
  | 30 | 有对象就答「有位号」 | `attach_symbol_identity` | R2 R4 | SUM |
  | 31 | `_Data.xml` 给的位号不算数 | `attach_symbol_identity` | R3 | — |
  | 32 | 读值时只比键名的开头 | `pid_value` | K | — |
  | 33 | 不数有对象的引用 | `build.rs` | R1 R2 R3 R4 | SUM |
  | 34 | 每个引用都算有对象 | `build.rs` | R2 R4 | S2 |
  | 35 | 不数带位号的引用 | `build.rs` | R1 R3 R4 | SUM |
  | 36 | `counts()` 里两个键名对调 | `summary.rs` | RT R4 | RIDE SUM |
  | 37 | 读回时不认 `symbol_tags` | `summary.rs` 的 `count_mut` | RT R4 | RIDE |

  改坏点已删：五个产品文件还原之后的 sha256 与加开关之前相同（`mut-run.txt`）；`src`、`tests`、`examples` 里搜不到开关。固定门禁是还原之后重新编译跑的。带开关的改动留在 `with-mutants.diff`。
- 没有红过的：
  - 「`_Data.xml` 给的不覆盖」这条规则（第 22、23、31 处）只有 R3 在钉，集成测试里没有一条会因此变红：`tests/pid_import.rs` 里带 `_Data.xml` 的那几条只查有没有，不逐键比。样例带 `_Data.xml` 的逐键核对是一次性量的，见下面。
  - S1 里排在数量后面的三段断言——没有两个图例共用一个 `graphic`；有 `class` 的恰是 `resolved=item` 的、有 `label` 的恰是 `tag_source=file` 的；块里的成员不带身份——在 37 处下都没有自己红过：前两段是数量那一条先红，块内成员那一段没有对应的改坏点。
  - 其余的测试（`--lib io::pid::` 另外 71 个，`tests/pid_import.rs` 另外 59 个）在 37 处下一个没红。
- 样例读数（验收第 1、5 条；样例只带 `.pid`，S1 和 SUM 钉着）：592 个块引用都有 `graphic`、`item`、`item_type`，592 个 `graphic` 各不相同；590 个有 `class`，都带 `resolved=item`；231 个有 `label`，都带 `tag_source=file`；没有 `label_in_file`，没有 `oid`。按种类：`EquipmentOther` 32、`Instrument` 269、`ItemNote` 49、`Mechanical` 4、`Nozzle` 9、`OPC` 2、`PipingComp` 225、`Vessel` 2。按发布类：`PIDControlSystemFunction` 87、`PIDInlineInstrument` 78、`PIDInstrument` 104、`PIDMechanicalEquipment` 4、`PIDNote` 49、`PIDNozzle` 9、`PIDPipingComponent` 225、`PIDProcessEquipment` 32、`PIDProcessVessel` 2；没有发布类的 2 个是 OPC。摘要 `symbol_items` 592、`symbol_tags` 231。这些是导入之后在图元上数的，和规格 §6 第 1 条按图纸里的记录量的数逐项相同，没有要说明的差别。
- 样例带 `_Data.xml`（验收第 2 条，规格 §6 第 6 条）：把样例的全部图元记录导出来（临时测试，没有入库；带和不带 `_Data.xml` 各 592 个块引用、5,798 个散图元），和图块分支验收时从程序里取的记录比（`cmp66.py`、`cmp66.txt`）。
  - 592 个块引用按先后配对，块名、位置不同的 0 个。
  - 原有的键是十二个（规格写的是十个）：`sheet_layer`、`sheet_layer_oid`、`role`、`style`、`extent`、`driving`、`instance`、`class`、`label`、`oid`、`resolved`、`symbol`。带 `_Data.xml` 时值改变或者没了的 0 处；只带 `.pid` 时也是 0 处，原来没有、现在由图纸文件给出的是 `class` 590、`label` 231、`resolved` 590。
  - 5,798 个散图元的记录不同的 0 个。键的先后不合固定次序或者重复的 0 个。
  - 带 `_Data.xml` 时：`class` 592、`label` 233、`oid` 592、`resolved` 592（全是 `direct`）、`graphic`、`item`、`item_type` 各 592，没有 `tag_source`。
  - 两边的发布类按图形逐个比：相同 590，不同 0，只有 `_Data.xml` 有的 2 个。这 2 个是 OPC（符号 `Off-Unit SQ`，图形 38445、38837）：`_Data.xml` 发布成 `PIDBranchPoint`，图纸文件不定发布类。位号：相同 231，不同 0，只有 `_Data.xml` 有的 2 个（还是这两个 OPC，`645`、`644`）。没有要用户定的。
  - 这次核对最初用的是简化代码之前的导出；简化之后用最终的代码重新导出了一遍，两份导出除了耗时那一行都与之前的逐行相同。留着的 `cmp66.txt` 是用最终代码的导出重跑出来的，上面的数取自它。
- 读不出对象的图（验收第 3 条，S2 钉着）：`pid-test-data` 的 `backup-plant-data\test\U01\D04.pid` 不写图号，对象读不出来。导入成功，5 个块引用都只多 `graphic`，没有 `item`、`item_type`、`class`、`label`、`tag_source`、`resolved`；摘要 `symbol_blocks` 5、`symbol_items` 0、`symbol_tags` 0。
- 结构、图例识别、耗时（验收第 4 条）：
  - 样例的另外五条测试期望值没有改，都过：592 个引用、119 个定义、散图元和块内成员按角色的个数；读两遍结果相同；另存 DWG、DXF 再打开；图例识别认出的符号 0 个、散字 958 条。
  - 耗时预算（`io::pid::sample_budget`，没有改；15:22，`budgetfinal.log`）：导入样例 3 次，3,385、3,708、3,440 毫秒，中位数 3,440，预算 3,747，过。测试里写的基线是 3,406。量的时候机器上另有别的程序在占 CPU。
  - 「读对象」这一步占多少：在集成测试的程序里导入样例各 3 次，只带 `.pid` 时带着这一步 2,692、1,943、1,298 毫秒，去掉这一步（第 11 处改坏）1,233、1,318、1,329 毫秒；前两个数偏高，当时机器上另有别人的编译在跑，这组数分不清这一步占多少。立项时单独量这一步是 92 毫秒（规格 §2.5 的核对程序）。
  - 带 `_Data.xml` 导入样例要 9 秒多，不是这次改动带来的：带着这一步 9,202、8,356、9,366 毫秒，去掉 9,322、9,645、9,423 毫秒。慢在读 `_Data.xml` 那一步，这次没有动它，也没有查它为什么慢。
- 读数（固定门禁 15:19–15:30，`chainfinal.txt`）：
  - 全量单元测试 7,150 个：通过 7,096，忽略 54，失败 0（上一个提交的 7,089 加新加的 7）。基线里那条不稳的窗口测试这一遍是过的。
  - 集成测试 `tests/pid_import.rs` 63 个（上一个提交的 61 加新加的 2）、`tests/pid_legend.rs` 31 个全过；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同；点到这九个文件的只有 `src/io/pid/mod.rs` 的 `finish` 参数个数那 1 条（8 个，上限 7 个；现在在第 602 行，上一个提交在第 596 行），上一个提交就有，这次没有给它加参数。
  - 格式：九个文件都是 `new=0`。
  - 分层报告与上一个提交逐行相同（`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1`）。
  - 改动：产品代码 +334 −2（`items.rs` 134，`metadata.rs` +147，`build.rs` +23 −1，`mod.rs` +14，`summary.rs` +16 −1）；测试 +866 −7（`items.rs` 128，`tests.rs` +545 −5，`template.rs` +1，`pid_symbol_block_tests.rs` +1，`tests/pid_import.rs` +191 −2）。
- 与清单不一样的地方：
  - 动到了清单「文件」之外的两个文件，`src/io/pid/template.rs`（一处测试调用多传一个参数）和 `src/app/pid_symbol_block_tests.rs`（一条期望值多 `graphic=701`）。记在规格 §11 的 SI-1。
  - 验收第 4 条拆开了：结构、图例识别、耗时预算量过，勾上；规格 §6 第 9 条的后半句（出图逐字节相同）没有量，没有勾，写进了 SI-7 的「做什么」。
  - 规格说「原有十个键」，实际是十二个，比的是十二个（规格 §11 的 SI-1 第 6 条）。
- 没验证的：
  - 出图没有重出，验收第 4 条里这半句没有勾。规格 §6 第 9 条的后半句「出图与图块分支逐字节相同」要编正式版的程序、走控制通道出图，这次没有做，留给 SI-7 带窗口的验收。现在只有一条间接的依据：图块分支验收时，带和不带 `_Data.xml` 的两份出图逐字节相同（`sb10\out\branch-only.svg`、`branch-xml.svg`，都是 14,769,401 字节，哈希相同），而那两种情况下 592 个块引用的记录差着 `class`、`label`、`oid`、`resolved` 四个键，可见出图不随这几个键变。新加的四个键出图认不认，没有查。
  - 没有在程序界面里看。属性面板现在对导入的图例显示什么没有看；那是 SI-3 的事。
  - 53 张合计的数（规格 §6 第 3 条）没有量，留给 SI-7。除了样例和 D04，`tests/pid_import.rs` 用到的另外几张图上只有原有的断言在管。
  - 「一个图形有几条表示记录时取第一条」在真实图纸上有没有用到，没有量：样例上换成取最后一条，S1 的数不变（第 10 处只有 I1 变红）。
  - 带 `_Data.xml` 的逐键核对只在样例一张图上做过，而且没有写成仓库里的测试。
  - `src/app/pid_symbol_block_tests.rs` 不在改坏时跑的那个范围里（`io::pid::`）；它在全量单元测试里是过的。`tests/pid_legend.rs` 没有改，也没有对它做改坏。
  - 用户手册没有改。
- 证据：`D:\scratch\ocs-symblock\si-1\`（先红：`chainred.txt`、`quickred-1.log`、`sample-red.log`；改坏：`with-mutants.diff`、`mut-run.txt`、`libmut-m1.txt`、`libmut-m1-*.log`、`mut-m1.txt`、`mut-m1-*.log`；样例的记录和核对：`dump-only.tsv`、`dump-xml.tsv`、`cmp66.py`、`cmp66.txt`、`xmlonly.txt`、`sample-final.log`；固定门禁：`final-runfinal.txt`、`budgetfinal.log`、`chainfinal.txt` 和它列的各份记录）。新加的脚本 `D:\scratch\ocs-symblock\tools\final-run.ps1`：先跑耗时预算，再跑固定门禁。

### SI-2 导入器给标注文字写配对 【接线】

- **做什么**：从已解析的文字记录和标注记录建「文字 → 挂在哪个图形上」的表（规格 §2.3 的拼法）；散着的文字写 `label_of`、`label_on`；导入摘要加 `symbol_labels`、`other_labels`。
- **验收**：
  - [ ] 规格 §6 第 4 条的样例部分（806、151、4；383）。
  - [ ] 每条 `label_on=symbol` 的文字恰对上一个块引用。
  - [ ] 样例上 958 个文字图元和 961 条文字记录差的 3 个查清，写进记录。
- **依赖**：SI-1。
- **文件**：`src/io/pid/labels.rs`（新）、`src/io/pid/build.rs`、`src/io/pid/metadata.rs`、`src/io/pid/summary.rs`、`src/io/pid/tests.rs`、`tests/pid_import.rs`。
- **规模**：L。

### SI-3 读记录的一组函数，属性面板 【纯】【接线】

- **做什么**：一组不碰外部状态的函数：从一个图元读出它的类型、位号、来源；找一个图例的全部标注；找一条标注所属的图例。经典界面的属性面板按规格 §4.5 加行，加中文译文。
- **验收**：
  - [ ] 规格 §4.5 的每一行各有一条测试。
  - [ ] 样例上单次「找一个图例的标注」的耗时量出来；超过 16 毫秒就改成导入后建一次索引（规格 §4.6）。
- **依赖**：SI-2。
- **文件**：`src/io/pid_identity.rs`（新）、`src/scene/cache/properties.rs`、译文文件。
- **规模**：M。

### SI-4 默认界面的检查栏 【接线】

- **做什么**：选中导入的图例或标注文字时，检查栏按规格 §4.6 只读显示。
- **验收**：
  - [ ] 有位号的图例、没有位号的图例、没有发布类的图例、读不到对象的图例、三种标注文字、所属图例不在图上的标注，各有一条测试。
  - [ ] 在本程序里画的对象、别的导入图元显示的内容不变（原有测试不改期望值）。
- **依赖**：SI-3。
- **文件**：`src/app/pid_shell/editor.rs`、`src/app/pid_shell/pid_editor_screen.rs`。
- **规模**：S。

### SI-5 `PIDTAG` 认导入的图例 【接线】

- **做什么**：按规格 §4.7。
- **验收**：
  - [ ] 按位号选中图例和全部标注；几个图例同一个位号时都选中；找不到时的提示不变。
  - [ ] 不带参数时，点中图例或它的一条标注都能选中整组；没有位号的图例也行。
  - [ ] 改位号的写法对导入的图例不生效，有一句提示。
  - [ ] 在 DXF、DWG 图纸上（图例识别的结果）行为不变（原有测试不改期望值）。
- **依赖**：SI-3。
- **文件**：`src/app/commands/pidlegend.rs`。
- **规模**：S。

### SI-6 从备份面板打开时用数据库补位号 【接线】

- **做什么**：备份的数据库读好后，用 `ExpTables::stored_column` 取 `T_PlantItem.ItemTag`，备成「对象编号 → 位号」的表；从面板打开的图纸按规格 §4.4 补位号。先定办法（导入时查表，还是显示后补一遍），写进记录。
- **验收**：
  - [ ] 规格 §6 第 2 条（样例 354 个，全是 `database`）。
  - [ ] 规格 §4.4 的五条各有测试或读数：数据库没读好时图纸先显示、不在界面线程上读、不标成已修改、读不全时不补、直接打开文件时不补。
  - [ ] 从真实的数据库取出 8,145 个位号（备份不在时跳过）。
- **依赖**：SI-1。
- **文件**：`src/io/pid_backup_browse/exp_views.rs`、`src/app/commands/pidbackup.rs`，另加办法定下来之后要动的文件。
- **规模**：M。这一项的办法还没定，是清单里最不确定的一项。

### SI-7 53 张核对，带窗口的验收，需求文档 【验收】【文档】

- **做什么**：53 张图逐张导入，对规格 §6 第 3、4 条的合计数；在带窗口的程序里做第 7、8、11、13 条，并重出样例的图，和图块分支验收时的出图逐字节比（第 9 条的后半句，SI-1 留下的）；`REQ-MIG-008` 加规格 §10 的两条验收标准。
- **验收**：
  - [ ] 规格 §6 的 14 条逐条有读数或说明，写成一张表。
  - [ ] 没验证的逐条写明。
- **依赖**：SI-1 到 SI-6。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`、本清单、规格。
- **规模**：M。
