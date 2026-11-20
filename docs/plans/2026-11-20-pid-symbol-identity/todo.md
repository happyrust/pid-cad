# 任务清单：导入的图例带上类型和位号，标注文字和图例配对

> 日期：2026-11-20。状态：**执行中**。规格已确认（2026-11-20，Q1–Q5 都按推荐）；SI-1、SI-2 做完，其余没有开始。
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
  - [x] 规格 §6 第 4 条的样例部分，按导入后的文字图元数：803、151、4；383。原先写的 806 是按文字记录量的，差的 3 条就是下面第 3 条查清的那 3 条。
  - [x] 每条 `label_on=symbol` 的文字恰对上一个块引用。
  - [x] 样例上 958 个文字图元和 961 条文字记录差的 3 个查清，写进记录。
  - 没有验证，留给 SI-7：规格 §6 第 12 条（耗时预算）。这次量的几遍都超过预算，机器一直不空闲；经过和定责见下面「耗时预算」那一条。
- **依赖**：SI-1。
- **文件**：`src/io/pid/labels.rs`（新）、`src/io/pid/build.rs`、`src/io/pid/metadata.rs`、`src/io/pid/summary.rs`、`src/io/pid/tests.rs`、`tests/pid_import.rs`。
- **规模**：L。

**做完**（2026-11-20）

- 改了什么（产品代码 +278 −11）：
  - `src/io/pid/labels.rs`（新文件，产品部分 145 行）：`SheetLabels::read` 从导入时已经解析出来的记录建「文字 → 它的标注挂在哪个图形上」的表，不另读文件。每个带图形记录的图面流各建一份：文字记录取编号和父引用，标注记录取编号和第一个成员的编号，线串取编号。第一个成员的编号由 `first_member` 从两个字段拼出：`sub_type_word` 是低 16 位，`raw_reference_payload` 的头两个字节是高 16 位，低字节在前；成员个数是 0、或者尾部不够两个字节，这条标注记录不进表。父引用不是表里的标注记录的文字不进表。
  - `src/io/pid/build.rs`（+71 −5）：每条文字记录查一次表；循环里另记下建成了的块引用（图面流加图形编号）；全部图元建完之后，逐个给查到了的文字图元定 `label_on`、写两个键、计数。对象表和配对表合成一个参数 `Stated`，`build_document_entities` 还是 7 个参数；`Built` 多两个数。
  - `src/io/pid/metadata.rs`（+25 −4）：键的固定次序从 17 个加到 19 个，`label_of`、`label_on` 排在最后；`attach_label_owner` 写这两个键。
  - `src/io/pid/mod.rs`（+20 −1）：`load_pid_with` 在建图元之前读一次配对表；`finish` 把两个数填进摘要，另写一行日志。
  - `src/io/pid/summary.rs`（+17 −1）：`ImportSummary` 加 `symbol_labels`、`other_labels`，排在最后；`counts()` 从 22 项变 24 项，原来 22 项的先后不动。`store`、`load`、`take` 照 `counts()` 走，没有改。
- 写进记录的规则（散着的文字，`role=text`）：
  - 文字记录的父引用是同一个图面流里一条有成员的标注记录：写 `label_of`（那条标注记录第一个成员的编号）和 `label_on`。
  - `label_on` 的值：这个编号在同一个图面流里建成了块引用，写 `symbol`；否则它是同一个图面流里的一条线串，写 `line`；其余写 `other`。
  - 一条文字记录画成几行，每一行的图元都写；内容空白的文字记录不画图元，没有地方写。
  - 不是文字记录画出来的图元都不写：块引用、块里的成员、线。
  - 两个键排在记录的最后：固定次序的第 18、19 位。

  和规格的写法不完全一样、或者规格没有写到的地方记在规格 §11 的 SI-2，共 12 条（最后一条是耗时预算没有验证）。
- 摘要的两项：`symbol_labels` 是 `label_on=symbol` 的文字图元数；`other_labels` 是 `line` 和 `other` 的文字图元数合计。和 SI-1 的两项一样只进日志，随文档过打开流程后取走。
- 简称。这一段的 R1–R7 和 SI-1 那一段的 R1–R4 不是同一批测试。
  - 单元测试，`src/io/pid/labels.rs` 里新加的：L1 是 `the_first_member_of_a_label_record_is_read_across_the_fields_that_hold_it`，L2 是 `a_text_hangs_on_the_first_member_of_the_label_record_it_names`，L3 是 `the_labels_are_read_off_the_records_the_parser_decoded`。
  - 单元测试，`src/io/pid/tests.rs` 里新加的：R1 是 `a_label_text_states_the_symbol_its_label_hangs_on`，R2 是 `a_label_on_a_run_of_line_says_line_and_one_on_anything_else_other`，R3 是 `a_label_hangs_on_a_symbol_only_where_the_symbol_was_filed_as_a_reference`，R4 是 `a_record_that_is_no_text_states_no_pairing`，R5 是 `every_line_a_label_draws_states_the_pairing_and_a_blank_one_draws_nothing`，R6 是 `the_pairing_follows_what_data_xml_published_for_the_text`，R7 是 `the_summary_counts_the_label_texts_by_what_they_hang_on`，K2 是 `the_keys_of_the_pairing_have_their_place_in_the_record`。
  - 单元测试，原有的：RT 同 SI-1。
  - `tests/pid_import.rs` 里新加的：S3 是 `the_sample_label_texts_state_what_their_labels_hang_on`，S4 是 `a_text_outside_a_label_states_no_pairing`。原有的：SUM、RIDE、S2 同 SI-1；DWG 是 `the_sample_saved_as_dwg_reopens_with_its_symbols_as_they_were`，DXF 是 `the_sample_saved_as_dxf_reopens_with_its_symbols_as_they_were`；KEYS 是 `role_and_class_are_separate_keys_with_disjoint_vocabularies`。
- 钉住的（L、R、K 都是代码里拼的数据）：
  - L1：样例上 3845 号标注记录的字段（两个成员，41377 和 5984）读出 41377；编号超过 65,535 时高 16 位在尾部、低字节在前（`0x9A7C` 加尾部 `02 01` 读出 `0x01029A7C`）；尾部只有两个字节也够；成员个数是 0 读不出；尾部只有一个字节、或者没有，读不出。
  - L2：一个图面流里五条文字、三条标注记录、两条线串，另一个图面流里一条同号的文字。同一条标注的两条文字挂在同一个图形上；父引用不是标注记录的两条文字查不到；标注记录自己的编号、被挂的图形的编号、不存在的编号都查不到；同一个编号在另一个图面流里是另一条文字；表里没有的图面流什么都查不到。线串也按图面流分开。
  - L3：一份拼出来的已解析文档，三个图面流，中间一个没有图形记录。文字对上标注记录的第一个成员；编号的高 16 位读对（52304 加高位 1 是 117,840）；成员个数是 0 的标注记录不配对；父引用不是标注记录的文字不配对；没有图形记录的图面流不妨碍它后面的图面流；线串只算在自己的图面流里；没有图面流的文档读出空表。
  - R1：两个阀、三条文字。挂在 612 号阀上的同一条标注有两条文字，一条排在阀前面，一条排在后面，都写 `label_of=612`、`label_on=symbol`；父引用不是标注记录的那条文字不多写。两个块引用的记录逐条写了期望，只有自己的 `graphic`，不写标注；块里 5 个成员各只有 `role`。两个数是 2、0。
  - R2：一条线串、一条直线、三条文字。挂在线串上的写 `line`；挂在图面上没有的编号（24578，图框的属性行）上的写 `other`；挂在单独一条直线上的写 `other`；三条都写了 `label_of`。线串和直线自己的记录不变。两个数是 0、3。
  - R3：一个画不出东西的标注符号（620，没有建块引用）、另一个图面流里的阀（630）、本图面流的阀（612），各挂一条文字。块引用只有 630、612 两个。挂在 620 和 630 上的写 `other`，挂在 612 上的写 `symbol`。两个数是 1、2。
  - R4：配对表里把一条直线（702）和一个阀（612）的编号当成文字登记。导入后模型空间里没有一个图元写 `label_of` 或 `label_on`。两个数是 0、0。
  - R5：同一条标注里一条两行的文字（`PT`、`0112101`）、一条全是空格的、一条空的。画出两个文字图元，都写两个键；空白的两条什么都不画。两个数是 2、0。
  - R6：旁边有 `_Data.xml`，而且它对上了这条文字。文字的记录逐条是 `sheet_layer`、`sheet_layer_oid`、`role=text`、`class=PIDInlineComponent`、`label=GV-101`、`oid=801`、`resolved=direct`、`label_of=612`、`label_on=symbol`：`_Data.xml` 写的不动，两个新键排在后面。
  - K2：一条已有 `role` 和一个不认识的键的记录，先写 `label_on`，再写 `label_of` 和 `label_in_file`，读出来是 `role`、`label_in_file`、`label_of`、`label_on`、不认识的键。再调一次 `attach_label_owner`，两个键在原位换值，不多出一条。`label_on` 的三个取值是 `symbol`、`line`、`other`。
  - R7：一个阀、一条线串、六条文字（三条挂在阀上，一条挂在线串上，一条挂在图框的属性行上，一条不是标注）。`counts()` 第 18 项之后是 `symbol_blocks` 1、`symbol_definitions` 1、`symbol_items` 0、`symbol_tags` 0、`symbol_labels` 3、`other_labels` 2；按字段读是 3、2；存进文档的两个键是 `3`、`2`；`take` 取回来的摘要与存进去的相等。
  - S3、S4、SUM、DWG、DXF：见下面「样例读数」「D04」「另存」。
- 先红：
  - 单元测试（15:50–15:54，`chainred.txt`）：这时接线、键的次序和 `attach_label_owner` 已经写好，`labels.rs` 里拼编号、建表、查表的函数还是空壳。`--lib io::pid::` 94 个：通过 84，失败 9，忽略 1。失败的是 L1、L2、L3、R1、R2、R3、R5、R6、R7。K2 这一遍是过的：它管的两个函数这时已经是真的，它会变红靠下面第 1、32–36 处。R4 是这之后加的，见下面「中途改了一处」。
  - 集成测试：S3 和 SUM 的新期望是实现之后才第一次跑的（15:55–16:02，`chaingreen.txt`），一次就过：期望的数是动手之前另写程序按记录量的（`analyze-a.txt`），不是从这次的实现里读出来的。这一遍里原有的 KEYS 红了：那张图的文字多了两个它不认识的键。S4 是把期望留空跑一遍（16:05–16:11，`integgreen2.log`），从失败信息里读出 95 再填的。S3、S4、SUM 没有在缺实现的时候红过，会变红靠下面的改坏点。
- 中途改了一处：第一版在查表之前还多判了一次「图元的角色是 `text`」。文字记录画出来的图元角色都是 `text`，这个条件没有哪个测试能让它单独起作用，删了；另加 R4 钉「只有文字记录才查配对」。
- 改了期望的原有测试 3 处（没有删测试，没有把相等放宽）：

  | # | 测试 | 原来 | 现在 | 原因 |
  |---|---|---|---|---|
  | 1 | `tests.rs` 的 `the_summary_counts_the_references_that_state_an_item_and_a_tag`（SI-1 的 R4） | `counts()[18..]` 等于四项 | 等于六项，末尾多 `symbol_labels` 0、`other_labels` 0；断言的说明补了半句 | 摘要多了两项；这张拼出来的图没有标注 |
  | 2 | `tests/pid_import.rs` 的 KEYS | 认得的键 15 个 | 17 个，加 `label_of`、`label_on` | 那张图（`DWG-0202GP06-01`）的文字记录多了这两个键 |
  | 3 | `tests/pid_import.rs` 的 SUM | 22 项 | 24 项，多 `symbol_labels` 803、`other_labels` 155；前 22 项的值没有改 | 摘要多了两项 |

  另外：RT 给两个新字段填了 30、9；DWG、DXF 共用的 `check_the_saved_copy` 加了两条断言（见「另存」）；S2 改成用新加的辅助函数 `sheet_d04()` 取图，期望值没有改；`tests.rs` 的辅助函数 `import_stating` 改成转调新加的 `import_as_stated`；`src/io/pid/template.rs` 的一处测试调用改成传 `Stated`。
- 故意改坏 43 处，用环境变量打开。单元测试和 `tests/pid_import.rs` 各编一次，把测试程序拷出来，不开的各跑一遍（单元测试通过 94、忽略 1，`tests/pid_import.rs` 65 个全过），每处各跑一遍（16:14–16:25，`libmut-m1.txt`、`mut-m1.txt`）。每一处都至少有一个测试变红：42 处在单元测试里有，29 处在 `tests/pid_import.rs` 里有，其中 28 处两边都有。

  | # | 改坏 | 改在哪 | `--lib io::pid::` 里变红 | `tests/pid_import.rs` 里变红 |
  |---|---|---|---|---|
  | 1 | `symbol` 和 `line` 两个字样对调 | `labels.rs` 的 `as_str` | R1 R2 R3 R5 R6 K2 | S3 |
  | 2 | 不读文字记录 | `labels.rs` 的 `read` | L3 | S3 SUM DWG DXF |
  | 3 | 文字的编号和父引用对调 | 同上 | L3 | S3 SUM DWG DXF |
  | 4 | 不读标注记录 | 同上 | L3 | S3 SUM DWG DXF |
  | 5 | 成员个数和编号的低 16 位对调 | 同上 | L3 | S3 SUM |
  | 6 | 标注记录按它的父引用登记，不按自己的编号 | 同上 | L3 | S3 SUM DWG DXF |
  | 7 | 不读线串 | 同上 | L3 | S3 |
  | 8 | 遇到没有图形记录的图面流就不往下读 | 同上 | L3 | — |
  | 9 | 图面流按名字登记，不按路径 | 同上 | L3 | S3 SUM DWG DXF |
  | 10 | 各图面流并成一张表 | `labels.rs` 建表、查表 | L2 L3 | — |
  | 11 | 文字挂在标注记录自己的编号上 | `labels.rs` 的 `add_stream` | L2 L3 R1 R2 R3 R5 R6 R7 | S3 SUM |
  | 12 | 父引用不是标注记录的文字也配对 | 同上 | L2 L3 R1 | S4 |
  | 13 | 什么都不算线串 | `labels.rs` 的 `is_run` | L2 L3 R2 | S3 |
  | 14 | 什么都算线串 | 同上 | L2 L3 R2 R3 | S3 |
  | 15 | 成员个数是 0 也读第一个成员 | `first_member` | L1 L3 | — |
  | 16 | 只取低 16 位 | 同上 | L1 L3 | — |
  | 17 | 尾部不够时当 0 读 | 同上 | L1 | — |
  | 18 | 高 16 位按高字节在前读 | 同上 | L1 L3 | — |
  | 19 | 高低 16 位对调 | 同上 | L1 L3 | S3 SUM |
  | 20 | 不是文字记录也查配对 | `build.rs` | R4 | — |
  | 21 | 画不出东西的符号实例也算建成了块引用 | `build.rs` | R3 | — |
  | 22 | 一条文字记录只有第一行写配对 | `build.rs` | R5 | — |
  | 23 | 文字建的时候就判定，图例排在后面的算不上 `symbol` | `build.rs` | R1 | — |
  | 24 | 找块引用时不分图面流 | `build.rs` | R3 | — |
  | 25 | 一律写 `symbol` | `build.rs` | R2 R3 R7 | S3 SUM |
  | 26 | 不记建成了的块引用 | `build.rs` | R1 R3 R5 R6 R7 | S3 SUM |
  | 27 | 只计数，不往文字上写 | `build.rs` | R1 R2 R3 R5 R6 | S3 DWG DXF |
  | 28 | 两个数加反 | `build.rs` | R1 R2 R3 R5 R7 | S3 SUM |
  | 29 | 不数挂在线上的 | `build.rs` | R2 R7 | S3 SUM |
  | 30 | 不数挂在别的东西上的 | `build.rs` | R2 R3 R7 | S3 SUM |
  | 31 | 不数挂在图例上的 | `build.rs` | R1 R3 R5 R7 | S3 SUM |
  | 32 | 不写 `label_of` | `attach_label_owner` | R1 R2 R3 R5 R6 K2 | S3 DWG DXF |
  | 33 | 不写 `label_on` | 同上 | R1 R2 R3 R5 R6 K2 | S3 |
  | 34 | 两个键不在固定次序里，排到最后 | `stamp_pid_pairs` 的排位 | K2 | — |
  | 35 | 两个键的位次对调 | 同上 | R1 R2 R3 R5 R6 K2 | — |
  | 36 | 两个键排到 `label_in_file` 前面 | 同上 | K2 | — |
  | 37 | 导入时不读配对表 | `mod.rs` | — | S3 SUM DWG DXF |
  | 38 | 摘要两项填反 | `mod.rs` 的 `finish` | R7 | S3 SUM |
  | 39 | 摘要两项不填 | 同上 | R7 | S3 SUM |
  | 40 | `counts()` 里两个键名对调 | `summary.rs` | RT R7 | RIDE SUM |
  | 41 | 读回时不认 `symbol_labels` | `summary.rs` 的 `count_mut` | RT R7 | RIDE |
  | 42 | 读回时不认 `other_labels` | 同上 | RT R7 | RIDE |
  | 43 | 存的时候不存这两项 | `summary.rs` 的 `store` | RT R7 | RIDE SUM |

  改坏点已删：五个产品文件还原之后的 sha256 与加开关之前相同（`mut-run.txt`）；`src`、`tests`、`examples` 里搜不到开关。固定门禁是还原之后重新编译跑的。带开关的改动留在 `with-mutants.diff`。
- 没有红过的：
  - 新加的 13 条测试每条都至少红过一次，但不是每一条断言都红过：
    - S3 的断言依次是：958 条散字；带两个键的各有多少、三种取值各有多少；每条 `symbol` 的文字恰对上一个块引用，别的对不上任何块引用；383；不是散字的图元不写配对；块里的成员不写配对；摘要的两个数。S3 在 24 处改坏下变红：18 处红在数量那一条，6 处（第 28–31、38、39 处）红在摘要那一条。中间四段没有红过：能让它们变红的改坏都先让数量那一条红了。
    - DWG、DXF 新加的两条断言里，变红的 8 处都红在另存之前数 958 行的那一条。「另存再读回，958 行逐行相同」那一条没有红过：43 处里没有一处是改另存的。
  - 14 处改坏只有拼出来的数据拦得住，集成测试里一条不红（第 8、10、15–18、20–24、34–36 处）。按改坏的内容看，是样例图上没有那种情况：
    - 样例上直接量过的（`analyze-a.txt`）：只有一个带图形的图面流（第 10、24 处）；3,635 条标注类记录里，成员个数是 0 的 0 条，尾部不够两个字节的 0 条（第 15、17 处）；没有画成几行的文字记录（第 22 处）；没有画不出东西的符号实例（第 21 处）。
    - 没有另外量、只是从改坏不红推出来的：挂在图例和线上的标注，第一个成员的编号去掉高 16 位结果不变，也就是看来都不超过 65,535（第 16、18 处）；没有排在自己图例前面的标注文字（第 23 处）；没有排在带图形的图面流前面的空图面流（第 8 处）；没有别的种类的记录和文字记录同号（第 20 处）。
  - 记录里两个键的先后在集成测试里没有测试管：第 34–36 处下 65 条一条不红。S3 只数有没有、值是什么，DWG、DXF 比的是另存前后是否相同。
  - `finish` 里新加的那行日志没有测试管。循环之后按句柄取回文字图元时「取不到就跳过」那个分支正常走不到，没有测试，也没有对应的改坏点。图元不写图面流路径时按空路径查表，这一点没有测试。
  - KEYS 和 S2 在 43 处下没有红（KEYS 在实现之后、改期望之前红过一次，见「先红」）。其余的测试（`--lib io::pid::` 另外 82 个，`tests/pid_import.rs` 另外 57 个）在 43 处下一个没红。
- 样例读数（验收第 1、2 条；样例只带 `.pid`，S3 和 SUM 钉着）：958 个散着的文字（`role=text`）全部带 `label_of` 和 `label_on`；按 `label_on` 是 `symbol` 803、`line` 151、`other` 4。803 条 `symbol` 的文字，每条的 `label_of` 恰等于一个块引用的 `graphic`；另外 155 条的 `label_of` 不等于任何块引用的 `graphic`。至少有一条标注的块引用 383 个（共 592 个）。不是散字的图元和块里的成员没有一个写这两个键。摘要 `symbol_labels` 803、`other_labels` 155。
- 和规格的数差在哪（验收第 3 条）：规格 §2.3 的 806、151、4 是按图纸文件里的 961 条文字记录量的。其中 3 条内容是空白（编号 39551 是空的，40241、49691 各是两个空格；都在挂在图例上的标注里），导入时不画图元。所以图元是 958 个，`symbol` 是 803 不是 806；`line`、`other` 和 383 不变。这 3 条是动手之前另写程序按记录查出来的（`analyze-a.txt`）；测试钉的是 958 和 803，不钉这三个编号。另外两点也是那次量的：样例只有一个带图形的图面流（`/Sheet6`）；958 条文字记录每条恰画一个图元。
- D04（S4 钉着）：`pid-test-data` 的 `backup-plant-data\test\U01\D04.pid`，95 个散着的文字，没有一个带 `label_of` 或 `label_on`，摘要两项都是 0。和规格 §2.3 按记录量的「父记录不是标注记录的 95 条」相同。
- 另存（DWG、DXF 钉着）：样例打开之后带配对的文字是 958 行；另存成 DWG、DXF 再读回，这 958 行逐行相同，每行比的是文字内容、图层和整条记录。
- 结构、图例识别（样例上原有的、期望值没有改的测试都过）：592 个引用、119 个定义、块引用上 SI-1 写的那些键；读两遍结果相同；图例识别认出的符号 0 个、散字 958 条。
- 耗时预算（规格 §6 第 12 条；`io::pid::sample_budget`，测试没有改）：**这次没有过，空闲的机器上也没有量到。**
  - 固定门禁那一遍（16:30，紧接在编译之后，`budgetfinal.log`）：导入样例 3 次，3,971、4,101、4,288 毫秒，中位数 4,101，预算 3,747，红。上一个提交量的中位数是 3,440。
  - 同一个测试程序不重新编译又跑两轮（16:41、16:43，`budget-rounds-a.txt`）：中位数 3,973、3,792，都红。
  - 机器：下面两组不编译的轮次（16:40–16:50，共 10 轮）每轮前后各记 3 个整机处理器占用的读数，60 个读数在 54 % 到 99 % 之间，中位数 72.5 %；头两轮前后数到的编译进程是 0、0、0、1 个（占用 67–91 %），后 8 轮是 4–6 个，那些编译不是这项工作的。16:30 量预算的那一刻没有记占用。16:52 另量了一次（`machine-load.txt`）：整机 56 %，其中一个浏览器进程（chrome，15:19 启动，不是这项工作启动的，没有动它）4.1 秒里用掉 55.4 秒处理器时间，占整机 32 个逻辑处理器的 42 %。预算是按机器空闲定的（测试文件里写的口径：没有编译在跑，整机占用不超过五分之一），这样的轮次今天一轮也没有等到。
  - 定责。临时给 `mod.rs`、`build.rs` 加了分段计时和一个「不读配对表」的开关（`with-timing.diff`），编了一份测试程序，先跑一遍（16:47，中位数 4,984，`timing-build.log`），再交替跑 8 轮（16:47–16:49，单数轮做这一步，双数轮不做，`budget-rounds-ab.txt`），共 27 次导入，各段的最小、中位、最大值算在 `timing-stats.txt`：
    - 解析图纸文件这一段（转换库的 `parse_file`，这次没有动，锁定的版本也没有换）用 3.10–5.54 秒，占一次导入的 88–94 %；其余各段合计 0.26–0.59 秒。
    - 这次加的三段（做这一步的 15 次导入）：读配对表 1.7–2.8 毫秒，主循环里查表和记账 1.7–3.3 毫秒，收尾写配对 3.1–6.8 毫秒；三段合计 7.0–12.9 毫秒，中位数 8.1 毫秒，占那一次导入的千分之 1.8–2.8。不做这一步的 12 次里主循环那一项仍有 1.2–1.9 毫秒：计时自己的开销和查空表都算在里面，没有再分。
    - 交替的 8 轮，整机占用 54–99 %：做这一步的 4 轮中位数 4,693、3,832、3,697、3,992，不做的 4 轮 3,721、4,305、3,440、3,869。两边都有过预算的，也都有不过的。
    - 量完按副本还原了这两个文件，sha256 与固定门禁跑的那一份相同，仓库里搜不到计时代码；还原后重新编译，`--lib io::pid::` 通过 94、忽略 1（16:52，`quick-restored.log`）。
  - 结论：这次加的一步约占一次导入的千分之二；去掉这一步，同样的机器状态下预算测试也有红的。所以今天的超预算算不到这次的改动头上。但「空闲的机器上中位数不超过 3,747」今天没有验证，写进了 SI-7 的「做什么」，机器空下来就重量。
- 读数（固定门禁 16:30–16:39，`chainfinal.txt`；耗时预算见上一条）：
  - 全量单元测试 7,161 个：通过 7,107，忽略 54，失败 0（上一个提交的 7,096 加新加的 11）。
  - 集成测试 `tests/pid_import.rs` 65 个（上一个提交的 63 加新加的 2）、`tests/pid_legend.rs` 31 个全过；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同；点到这次八个文件的只有 `src/io/pid/mod.rs` 的 `finish` 参数个数（8/7）那 1 条，上一个提交就有，这次没有给它加参数。
  - 格式：八个文件都是 `new=0`。
  - 分层报告与上一个提交逐行相同（`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1`）。
  - 改动：产品代码 +278 −11（`labels.rs` 145，`build.rs` +71 −5，`metadata.rs` +25 −4，`mod.rs` +20 −1，`summary.rs` +17 −1）；测试 +974 −17（`labels.rs` 276，`tests.rs` +510 −2，`template.rs` +4 −1，`tests/pid_import.rs` +184 −14）。
- 与清单不一样的地方：
  - 验收第 1 条的数改了：清单原来写的是 806、151、4，那是按文字记录量的；图元上是 803、151、4，测试钉的是后一组。差的 3 条就是验收第 3 条要查清的那 3 条。
  - 动到了清单「文件」之外的两个文件：`src/io/pid/mod.rs`（读配对表、填摘要、一行日志）和 `src/io/pid/template.rs`（一处测试调用改了传参）。清单漏写了 `mod.rs`：配对表要在导入入口读。
  - 规格 §4.3 的判断细化了三处：`symbol` 要真建成了块引用，`line` 只认线串，没有成员的标注记录不配对。样例上这三处都不改变结果。
  - 验收下面加了一行不带勾的：规格 §6 第 12 条（耗时预算）没有验证，留给 SI-7。SI-7 的「做什么」加了三件 SI-2 留下的事：配对的合计数按文字图元量、查有没有一个文件带几个有图形的图面流、在空闲的机器上重量耗时预算。
- 没验证的：
  - 机器空闲时的耗时预算（规格 §6 第 12 条）：见上面「耗时预算」那一条。
  - 53 张合计的数（规格 §6 第 4 条的后半句：13,161、2,303、152，6,110）没有量，留给 SI-7。那组数也是按记录量的，图元上会少掉内容空白的记录，少多少没有量。
  - 一个文件有几个带图形的图面流时按图面流分开对不对，只有拼出来的数据在钉（L2、L3、R3）。53 张里有没有这样的图没有查，留给 SI-7。
  - 「挂在一个画不出东西的符号实例上的标注写 `other`」「挂在单独一条直线上的标注写 `other`」在真实图纸上有没有出现，没有量；样例上都没有。
  - 带 `_Data.xml` 的样例没有量配对。配对只取自 `.pid`；带 `_Data.xml` 时键的先后只有 R6 一条拼出来的测试在钉。
  - 出图没有重出（同 SI-1，留给 SI-7）。文字图元多了两个键，出图认不认，没有查。
  - 没有在程序界面里看。选中一条标注文字现在显示什么没有看；那是 SI-3、SI-4 的事。
  - 改坏只跑了 `--lib io::pid::` 和 `tests/pid_import.rs`。全量单元测试里别的测试、`tests/pid_legend.rs` 没有在改坏下跑。
  - 用户手册没有改。
- 证据：`D:\scratch\ocs-symblock\si-2\`（动手前按记录量的数：`texts-run.ps1`、`analyze.py`、`analyze-a.txt`；先红：`chainred.txt`、`quickred-1.log`；实现后的两遍：`chaingreen.txt`、`integgreen.log`、`chaingreen2.txt`、`integgreen2.log`；改坏：`with-mutants.diff`、`pre-mutant\`、`mut-run.txt`、`libmut-m1.txt`、`libmut-m1-*.log`、`mut-m1.txt`、`mut-m1-*.log`；固定门禁：`final-runfinal.txt`、`budgetfinal.log`、`chainfinal.txt` 和它列的各份记录；耗时定责：`budget-rounds-a.txt`、`with-timing.diff`、`timing-build.log`、`budget-rounds-ab.txt`、`budget-rounds-ab-*.log`、`timing-stats.txt`、`machine-load.txt`、`quick-restored.log`）。新加的脚本在 `D:\scratch\ocs-symblock\tools\`：`budget-rounds.ps1` 不编译，把预算测试跑若干轮，每轮记下前后的整机占用和编译进程的个数；`timing-stats.ps1` 从计时记录算各段的最小、中位、最大值；`machine-load.ps1` 记下哪些进程在占处理器（只读，只记进程名和用量）。

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

- **做什么**：53 张图逐张导入，对规格 §6 第 3、4 条的合计数（第 4 条的配对数按文字图元量，和规格按记录量的数差在哪逐项说明；顺带查 53 张里有没有一个文件带几个有图形的图面流。这两件是 SI-2 留下的）；在带窗口的程序里做第 7、8、11、13 条，并重出样例的图，和图块分支验收时的出图逐字节比（第 9 条的后半句，SI-1 留下的）；在空闲的机器上重量样例的耗时预算（第 12 条，SI-2 留下的：没有编译在跑、整机占用不超过五分之一时跑预算测试，仍然超过就查）；`REQ-MIG-008` 加规格 §10 的两条验收标准。
- **验收**：
  - [ ] 规格 §6 的 14 条逐条有读数或说明，写成一张表。
  - [ ] 没验证的逐条写明。
- **依赖**：SI-1 到 SI-6。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`、本清单、规格。
- **规模**：M。
