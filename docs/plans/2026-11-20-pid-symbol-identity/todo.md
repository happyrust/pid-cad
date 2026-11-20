# 任务清单：导入的图例带上类型和位号，标注文字和图例配对

> 日期：2026-11-20。状态：**执行中**。规格已确认（2026-11-20，Q1–Q5 都按推荐）；SI-1、SI-2、SI-3、SI-4、SI-5、SI-6 做完，其余没有开始。
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
  - [x] 规格 §4.5 的每一行各有一条测试。
  - [x] 样例上单次「找一个图例的标注」的耗时量出来；超过 16 毫秒就改成导入后建一次索引（规格 §4.6）。量出来 592 次平均 3.2–3.4 毫秒，最慢的一次 12.2 毫秒，没有建索引。只量了样例一张。
  - 没有验证，留给 SI-7：规格 §6 第 12 条（耗时预算）。这次量了五次，过了一次，机器一直不空闲；经过见下面「耗时预算」那一条。
- **依赖**：SI-2。
- **文件**：`src/io/pid_identity.rs`（新）、`src/scene/cache/properties.rs`、译文文件。
- **规模**：M。

**做完**（2026-11-20）

- 改了什么（产品代码 +411 −16；另有 `src/io/pid/mod.rs` +6 −4，只在测试里编译；译文 23 个文件 +185 −1）：
  - `src/io/pid_identity.rs`（新文件，产品部分 234 行）：一组只读记录、什么都不改的函数，所有编译目标都编。
    - `stated`：按整个键取值（`label` 不是 `label_of`）；同一个键写了两次取前一个；记录里不是「键=值」字符串的值跳过。
    - `symbol_identity`：只认写了 `role=symbol` 的块引用，读出图形编号、对象编号、对象种类、发布类、符号名、位号、位号来源、图纸文件里的名字；空值算没写。`kind()` 是发布类，没有就是对象种类。位号来源只在有位号时有：`file`、`database`、没有标记（`_Data.xml` 给的位号）三种，这一版不认识的标记原样带出。
    - `label_pairing`：`label_of`（非空）和 `label_on` 都写了才算一条标注；`label_on` 是 `symbol`、`line`，其余的值都算 `other`。
    - `lettering`：单行文字、多行文字的内容；别的图元没有。
    - `labels_of`：模型空间里 `label_on=symbol` 而且 `label_of` 等于这个编号的图元，按图纸里的先后；块定义里的不算。
    - `symbols_numbered`：模型空间里写着这个编号的图例块引用，按先后；复制出来的同号，都在里面。
    - `label_owner`：一条标注挂在什么上：图例（同号的取排在最前的一个）、图例不在图上、线、别的；不是标注的图元没有答案。挂在线上和别的东西上的不去找同号的图例。
  - `src/scene/cache/properties.rs`（产品 +155 −3）：`pid_semantics_section` 多认 `item_type`、`tag_source`、`label_in_file` 三个键，多出 Item type、Tag source、Name in file 三行，Type 和 Matched by 的显示按下面的规则；新加 `add_pid_pairing_row`，给 P&ID 那一节加 Labels 或 Label of 一行，要扫一遍模型空间。
  - `src/app/properties.rs`（+8）：只选中一个对象的那一页（`entity_property_sections`）调一次 `add_pid_pairing_row`。
  - `src/io/pid/metadata.rs`（+5 −13）：导入器读键用的 `pid_value` 改成转出 `pid_identity::stated`，读法只留一份；取值的办法没有变。
  - `src/io/mod.rs`（+1）：挂上新模块。
  - `src/locale_catalog.rs`（+8）和 23 个 `locales/*/opencadstudio.ftl`（各 +8，`zh-CN` 另改 1 行）：8 条新词条。
- 面板上的规则（规格 §4.5）。括号里是行在程序里的名字，测试按它找行：
  - Type（`pid_class`）：发布类；`class` 没有写或者是空值时显示对象种类。
  - Item type（`pid_item_type`，新）：`item_type`，紧跟在 Type 后面；空值没有这一行。
  - Tag source（`pid_label_source`，新）：紧跟在位号后面，有位号而且 `tag_source` 非空时才有。`file` 显示「Drawing file」，`database` 显示「Backup database」，别的值照原文。
  - Name in file（`pid_label_in_file`，新）：`label_in_file`，排在 Tag source 后面；空值没有这一行。
  - Labels（`pid_labels`，新，只在图例的块引用上）：`<条数>: <前三条的文字，用 · 隔开> +<其余的条数>`，文字去掉首尾空白；没有标注就没有这一行。
  - Label of（`pid_label_of`，新，只在标注文字上）：所属图例的位号；没有位号写「类型 · 符号名」，只有一样就写一样，都没有写「Symbol」；挂在线上写「A line」，挂在别的东西上写「Another object」；图例不在图上写「Not on the sheet」。
  - Labels、Label of 排在 Matched by、Sheet layer、Layer OID 三行的前面，这一节没有这三行时排在最后；图元原来没有 P&ID 这一节时新建一节，排在已有各节的后面。这两行只在选中一个对象时有。
  - Matched by（`pid_resolved`）：`resolved=item` 显示「Item in the drawing file」；别的值和原来一样。

  和规格的写法不完全一样、或者规格没有写到的地方记在规格 §11 的 SI-3，共 13 条（最后一条是耗时预算没有验证）。
- 简称。
  - 单元测试，`src/io/pid_identity.rs` 里新加的：I1 是 `a_key_reads_back_whole_and_first`，I2 是 `a_symbol_reference_reads_back_what_it_states`，I3 是 `the_kind_of_a_symbol_is_its_class_or_else_its_item_type`，I4 是 `a_tag_says_where_it_came_from`，I5 是 `only_a_block_reference_that_says_symbol_is_a_symbol`，I6 是 `a_label_text_reads_back_what_its_label_hangs_on`，I7 是 `the_lettering_of_a_text_is_what_it_reads`，I8 是 `the_labels_of_a_symbol_are_the_texts_that_name_its_graphic`，I9 是 `a_label_finds_the_symbol_it_hangs_on`。
  - 单元测试，`src/scene/cache/properties.rs` 里新加的：P1 是 `an_item_shows_the_kind_the_drawing_file_holds_it_as`，P2 是 `a_tag_names_where_it_came_from_in_the_row_after_it`，P3 是 `an_item_the_drawing_file_states_says_so_in_words`，P4 是 `a_symbol_lists_the_labels_the_sheet_hangs_on_it`，P5 是 `a_label_text_names_what_its_label_hangs_on`，P6 是 `the_rows_of_an_imported_symbol_stand_in_one_order`，P7 是 `the_row_of_a_pairing_joins_the_pid_group_or_makes_it`。
  - 单元测试，`src/app/pid_symbol_block_tests.rs` 里新加的：A1 是 `a_symbol_and_its_labels_name_each_other_in_the_panel`。
  - 单元测试，原有的：K 是 `src/io/pid/tests.rs` 的 `a_key_stamped_later_takes_its_place_in_the_record`（SI-1），SB6 是 `src/app/pid_symbol_block_tests.rs` 的 `a_selected_symbol_shows_its_reference_and_its_pid_group_in_the_panel`（图块计划）。
  - `tests/pid_import.rs` 里新加的：S5 是 `the_sample_symbols_and_their_labels_find_each_other`。
  - `tests/pid_panel_localization.rs` 里新加的：LOC 是 `what_the_panel_says_of_a_symbol_and_its_labels_is_said_in_every_shipped_language`。原有的：CAP 是 `pid_panel_captions_translate_in_every_shipped_language`。
- 规格 §4.5 的每一行对哪条测试（验收第 1 条）：Type、Item type 是 P1；Item tag、Tag source、Name in file 是 P2；Labels 是 P4；Matched by 是 P3；Label of 是 P5。另外：各行的先后是 P6；没有 P&ID 一节时新建是 P7；在程序里点选之后面板上有什么是 A1；译文是 LOC。
- 钉住的（I、P 都是代码里拼的数据）：
  - I1：一条记录写了 `label_of=612`、`label=GV-101`、`label=GV-102`、`note=a=b`。`label` 读出 `GV-101`（前一个），`label_of` 读出 `612`，`label_o` 读不出，`note` 读出 `a=b`（只在第一个等号处拆）；没有记录的图元什么都读不出。另一条记录里夹着一个整数和一个不带等号的字符串：跳过它们，`graphic` 照样读出 9，读成的图例只有图形编号一项。
  - I2：三个块引用逐项写了期望。图纸文件给了名字的阀（13 个键）：图形编号 612、对象编号、对象种类 `PipingComp`、发布类 `PIDPipingComponent`、符号名 `Ball Valve`、位号 `GV-101`、来源是图纸文件、没有「图纸文件里的名字」。数据库给了位号的泵：位号 `P-1001A`、来源是数据库、图纸文件里的名字 `P-1001`、没有发布类。图纸文件没有给对象的：只有图形编号和符号名。
  - I3：发布类和对象种类都有时类型是发布类；只有对象种类（`OPC`）时是对象种类；都没有时没有类型。
  - I4：有位号时，`tag_source=file` 是图纸文件，`database` 是数据库，没有标记是 `_Data.xml`，`manual` 原样带出；没有位号、或者位号是空值时没有来源。
  - I5：写了 `role=symbol` 的块引用是图例；什么都没写的块引用、图例识别认出来的块引用（带 `class`、`label`、`resolved=legend:block`）、`role=geometry` 的块引用、写了 `role=symbol` 的一条线都不是。
  - I6：`symbol`、`line`、`other` 各读对，编号读对；不认识的取值算 `other`；只写了两个键里的一个、`label_of` 是空值、没有记录，都不算标注。
  - I7：单行文字读出 `GV-101`，多行文字读出 `DN50`，线读不出。
  - I8：一张图上两个阀（612、613）和五条文字，另有块定义里一条写着 612 的文字。612 的标注是排在它前面的一条和后面的一条，按图纸里的先后；613 的是它自己的一条；把 612 当成线来挂的那条不算；块定义里的不算；614 没有标注；`61` 不按前缀对上 612。
  - I9：一条标注、两个同号的阀（612）、一条挂在 700 上的（图上写着 700 的只是一条线）、一条挂在线上的、一条挂在别的东西上的、一条不是标注的。标注找到排在前面的那个阀；700 那条是「图例不在图上」；线、别的、不是标注各自读对。按编号找图例：612 是两个，按先后；700 没有（线不算图例）；`61` 没有。
  - P1：发布类和对象种类都有：Type 是发布类，Item type 是对象种类，行的次序是 Type、Item type、Role、Matched by，两行的行名是 `Type`、`Item type` 的译文。只有对象种类（`OPC`）：两行都显示 `OPC`。`class=` 是空值而有对象种类：Type 显示对象种类。`item_type=` 是空值：没有 Item type 行，原有的行不变。只有 `class=`：Type 行照旧显示空值。
  - P2：位号 `GV-101`。`tag_source=file`：位号后面一行是 Tag source，显示「Drawing file」的译文，三行的次序是 Role、Item tag、Tag source。`database` 加 `label_in_file`：显示「Backup database」的译文，再后面一行 Name in file 显示 `GV-101-OLD`。`_Data.xml` 给的位号（带 `class`、`resolved=direct`，没有标记）：没有来源行。`manual` 照原文显示。两个键是空值：都不显示。没有位号而写了 `tag_source=file`：不显示来源行。
  - P3：`resolved=item` 显示「Item in the drawing file」的译文；`direct`、`dependency:77`、`items` 照原文；行名是 `Matched by` 的译文。
  - P4：一个阀（612）带 0、1、3、4、5 条标注，另有一条把 612 当成线来挂的文字。0 条：没有 Labels 行，行还是 Role、Symbol、Matched by、Sheet layer。1 条：`1: GV-101`，排在 Symbol 和 Matched by 之间，行名是 `Labels` 的译文。3 条：`3: GV-101 · DN50 · PN16`（第二条的文字前后各有一个空格，去掉了）。4 条：末尾多 `+1`。5 条：`+2`。
  - P5：五个图例各挂一条标注：有位号的；没有位号、有发布类和符号名的；只有符号名的；只有对象种类的（`symbol=` 是空值）；只有图形编号的。另有挂在 700（图上没有）上的、挂在线上的、挂在别的东西上的、不是标注的、没有图纸图层的各一条。Label of 依次是 `GV-101`、`PIDPipingComponent · Ball Valve`、`Ball Valve`、`OPC`、「Symbol」的译文、「Not on the sheet」的译文、「A line」的译文、「Another object」的译文；不是标注的文字和图例自己没有这一行。带图纸图层的那条，行的次序是 Role、Label of、Sheet layer，行名是 `Label of` 的译文；没有图纸图层的那条是 Role、Label of。
  - P6：一个写全了 14 个键的图例（数据库的位号 `GV-101A`，图纸文件里的名字 `GV-101`）加一条排在它前面的标注。十二行的次序：Type、Item type、Role、Symbol、Body extent、Item tag、Tag source、Name in file、Labels、Matched by、Sheet layer、Layer OID；Labels 是 `1: GV-101A`。
  - P7：三条文字各自的整页（常规一节、P&ID 一节、兜底一节）。只写了 `label_of=6`、`label_on=other` 的：原来没有 P&ID 一节，新建一节排在最后，里面只有 Label of。只写了 `role=text` 的：页面不变。写了 `role=text` 又挂在线上的：Label of 加进原有的 P&ID 一节，那一节的位置不动。
  - A1：打开新加的一张拼出来的图 `sheet_of_a_tagged_valve_and_its_labels`，它走导入器解析之后的各步生成：一个图纸文件给了名字 `GV-101` 的阀带四条标注（`GV-101`、`DN50`、`PN16`、`CS`，第一条排在阀的前面）；一个没有名字的阀带一条（`DN80`）；一段线串和挂在它上面的 `P-1001-50-CS`；挂在图上没有的编号上的 `DWG-0201`；不是标注的 `NOTE 4`。
    - 在画面上点文字，每次只选中这一条，逐行比 P&ID 一节：`DN50` 和排在阀前面的 `GV-101` 都是 Role、`Label of = GV-101`、两行图纸图层；`DN80` 是 `PIDPipingComponent · Ball Valve`；线上的是「A line」；`DWG-0201` 是「Another object」；`NOTE 4` 没有 Label of。
    - 点阀的圈。有名字的阀十一行：Type `PIDPipingComponent`、Item type `PipingComp`、Role、Symbol、Body extent、Item tag `GV-101`、Tag source「Drawing file」、Labels `4: GV-101 · DN50 · PN16 +1`、Matched by「Item in the drawing file」、两行图纸图层。没有名字的阀九行：没有位号和来源两行，Labels 是 `1: DN80`。
    - 框选阀和它的四条标注（5 个对象）：Labels、Label of 两行都没有。
    - 删掉有名字的阀，它的标注显示「Not on the sheet」；撤销一步，又显示 `GV-101`。
  - S5：见下面「样例读数」。另把每次查找的耗时打在标准错误输出上，不拿它作断言。
  - LOC：程序带的 23 种语言逐个切换。8 条新词条和 3 条复用的词条在每种语言里都有内容；8 条新词条在英文里等于原文，在其余 22 种语言里都不等于原文（荷兰语的 `Labels` 除外，它和英文同形）。简体中文 11 条、繁体中文 8 条逐条写了期望。切到简体中文后面板建出来的行：`类型 = PipingComp`、`图元类型 = PipingComp`、`位号 = GV-101A`、`位号来源 = 备份数据库`、`图纸文件里的名字 = GV-101`、`匹配方式 = 图纸文件里的对象`，来源换成 `file` 时是 `位号来源 = 图纸文件`；`标注 = 1: DN50`、`标注属于 = Ball Valve`、`标注属于 = 不在图上`、`标注属于 = 一条管线`、`标注属于 = 其他对象`。
- 先红：
  - 第一遍（17:30–17:37，`chainred.txt`）：这时 `pid_identity.rs` 里的函数是空壳，面板没有加行，词条和译文没有加。单元测试这一遍没有编过：测试代码自己有一处错（I7 里的多行文字要装箱），和要测的东西无关，改了。集成测试编过了：`tests/pid_import.rs` 66 个里 S5 红（块引用读不成图例），其余 65 个过；`tests/pid_panel_localization.rs` 2 个里 LOC 红（保加利亚语的 `Name in file` 没有翻译），CAP 过。
  - 第二遍（17:38–17:43，`chainred2.txt`）：单元测试 `io::pid_identity`、`scene::cache::properties::pid_semantics_tests`、`app::pid_symbol_block_tests` 三组 38 个：通过 21，失败 17，就是新加的 I1–I9、P1–P7、A1。
  - 实现之后、加译文之前（17:50–17:54，`chainmid.txt`）：`tests/pid_import.rs` 66 个全过；LOC 仍然红，23 个译文文件加了之后才过（18:01，`chainprobe.txt`）。
  - 这一遍里 A1 还红着，原因在测试自己：点文字的位置取在了插入点下方，文字是左下角对齐的，点空了。为了查清这一点临时加过一条探测用的测试，查完删了；改成点在字高一半的地方之后 A1 过（`chaingreen1.txt`）。产品代码没有为这件事改。
- 改了期望的原有测试 0 处。原有的测试代码动过的地方都不涉及期望值：
  - `tests/pid_panel_localization.rs` 的 CAP：开头加了一把锁（界面语言是整个进程共用的，这个文件现在有两条测试都在切语言，不能同时跑）；它用的辅助函数 `panel_caption` 改成转调新加的 `saying`。断言一条没改。
  - `src/io/pid/tests.rs`：原有的拼图代码拆出 `summarised_as_stated`、`ringed_valve` 两个辅助函数，和新加的那张图共用。
  - `src/scene/cache/properties.rs` 的测试模块：原有的辅助函数 `entity` 改成转调新加的 `saying`。
  - `src/app/pid_symbol_block_tests.rs`：文件开头的说明改了一行。
- 故意改坏 73 处，用环境变量打开。单元测试和两个集成测试目标各编一次，把测试程序拷出来，不开的各跑一遍，每处各跑一遍：
  - 单元测试（固定门禁快跑的那四组，133 个；18:12–18:16，`libmut-m1.txt`）：不开时通过 132、忽略 1；73 处里 70 处有测试变红。
  - `tests/pid_panel_localization.rs`（18:16–18:19，`mut-m1p.txt`）：不开时 2 个全过；73 处里 19 处变红，都红在 LOC。
  - `tests/pid_import.rs`（18:20–18:25，`mut-m1i.txt`）：只跑了改 `pid_identity.rs` 的 30 处（第 1–30 处）；不开时 66 个全过；7 处变红，都红在 S5。
  - 每一处都至少有一条测试变红。单元测试里不红的 3 处（第 71–73 处，改的是词条表）在 LOC 里红：单元测试比的是「面板上的字 = 这句话的译文」，两边走同一张词条表，词条表错了它看不出来。

  | # | 改坏 | 改在哪 | 单元测试里变红 | `tests/pid_panel_localization.rs` 里变红 | `tests/pid_import.rs` 里变红 |
  |---|---|---|---|---|---|
  | 1 | 「键=值」从最后一个等号处拆 | `pid_identity.rs` 的 `pairs` | I1 | — | — |
  | 2 | 键按前缀比 | `pid_identity.rs` 的 `stated` | I1 K | — | — |
  | 3 | 同一个键写了两次时取后一个 | 同上 | I1 | — | — |
  | 4 | 空值也算写了 | `pid_identity.rs` 的 `state` | I4 I6 P5 | — | — |
  | 5 | 不是块引用也算图例 | `pid_identity.rs` 的 `is_symbol` | I5 | — | — |
  | 6 | 不看 `role`，块引用都算图例 | 同上 | I5 | — | — |
  | 7 | 找图例时图形编号按前缀比 | `pid_identity.rs` 的 `is_numbered` | I9 | — | — |
  | 8 | 带这个编号的图元不是图例也算 | 同上 | I9 | — | — |
  | 9 | 类型先取对象种类，再取发布类 | `pid_identity.rs` 的 `kind` | I3 P5 A1 | — | — |
  | 10 | 没有来源标记的位号当成图纸文件的 | `pid_identity.rs` 的 `symbol_identity` | I4 | — | — |
  | 11 | `file` 和 `database` 对调 | 同上 | I2 I4 | — | S5 |
  | 12 | 不认识的来源标记当成图纸文件的 | 同上 | I4 | — | — |
  | 13 | 没有位号也报来源 | 同上 | I1 I2 I4 | — | S5 |
  | 14 | 对象编号读成对象种类 | 同上 | I2 | — | — |
  | 15 | 图纸文件里的名字读成位号 | 同上 | I2 | — | S5 |
  | 16 | `label_of` 是空的也算配对 | `pid_identity.rs` 的 `label_pairing` | I6 | — | — |
  | 17 | 只有 `label_of`、没有 `label_on` 也算配对，当成挂在图例上 | 同上 | I6 | — | — |
  | 18 | `line` 读成挂在图例上 | 同上 | I6 I8 I9 P4 P5 A1 | LOC | S5 |
  | 19 | `other` 读成挂在线上 | 同上 | I6 I9 P5 A1 | LOC | S5 |
  | 20 | 不认识的 `label_on` 读成挂在图例上 | 同上 | I6 | — | — |
  | 21 | 多行文字读不出内容 | `pid_identity.rs` 的 `lettering` | I7 | — | — |
  | 22 | 找标注时不看 `label_on` | `pid_identity.rs` 的 `labels_of` | I8 P4 | — | — |
  | 23 | 找标注时编号按前缀比 | 同上 | I8 | — | S5 |
  | 24 | 块定义里的文字也算 | 同上 | I8 | — | — |
  | 25 | 标注的先后倒过来 | 同上 | I8 P4 A1 | — | — |
  | 26 | 只给第一条标注 | 同上 | I8 P4 A1 | — | S5 |
  | 27 | 同号的图例只给第一个 | `pid_identity.rs` 的 `symbols_numbered` | I9 | — | — |
  | 28 | 同号的图例取最后一个 | `pid_identity.rs` 的 `label_owner` | I9 | — | — |
  | 29 | 图例不在图上时当成不是标注 | 同上 | I9 P5 A1 | LOC | — |
  | 30 | 挂在线上的标注也去找同号的图例 | 同上 | I9 P5 | — | — |
  | 31 | 空的 `item_type` 也算 | `scene/cache/properties.rs` 的 `pid_semantics_section` | P1 | — | （没跑） |
  | 32 | 空的 `tag_source` 也显示 | 同上 | P2 | — | （没跑） |
  | 33 | 空的 `label_in_file` 也显示 | 同上 | P2 | — | （没跑） |
  | 34 | 没有发布类时 Type 不显示对象种类 | 同上 | P1 | LOC | （没跑） |
  | 35 | 有发布类时 Type 也显示对象种类 | 同上 | P1 A1 | — | （没跑） |
  | 36 | `class=` 是空的、又没有对象种类时不显示 Type 行 | 同上 | P1 | — | （没跑） |
  | 37 | 不加 Item type 行 | 同上 | P1 P6 A1 | LOC | （没跑） |
  | 38 | Item type 行的行名写成 Type | 同上 | P1 | LOC | （没跑） |
  | 39 | 来源显示原文 `file`、`database` | 同上 | P2 A1 | LOC | （没跑） |
  | 40 | 两种来源的字样对调 | 同上 | P2 A1 | LOC | （没跑） |
  | 41 | 不认识的来源显示成空 | 同上 | P2 | — | （没跑） |
  | 42 | 没有位号也显示来源行 | 同上 | P2 | — | （没跑） |
  | 43 | 不加 Name in file 行 | 同上 | P2 P6 | LOC | （没跑） |
  | 44 | `resolved=item` 显示原文 | 同上 | P3 A1 | LOC | （没跑） |
  | 45 | 以 `item` 开头的值都显示成那句话 | 同上 | P3 | — | （没跑） |
  | 46 | 配对行加进第一个分组，不找 P&ID 组 | `scene/cache/properties.rs` 的 `add_pid_pairing_row` | P7 A1 | — | （没跑） |
  | 47 | 配对行排在 Matched by 后面 | 同上 | P4 P6 A1 | — | （没跑） |
  | 48 | 配对行排在组的最后 | 同上 | P4 P5 P6 A1 | — | （没跑） |
  | 49 | 配对行排在组的最前 | 同上 | P4 P5 P6 P7 A1 | — | （没跑） |
  | 50 | 没有 P&ID 组时不加行 | 同上 | P7 | — | （没跑） |
  | 51 | 新建的 P&ID 组排在最前 | 同上 | P7 | — | （没跑） |
  | 52 | 没有标注的图例也显示 Labels 行 | `scene/cache/properties.rs` 的 `pid_pairing_row` | P4 SB6 | — | （没跑） |
  | 53 | 列 4 条 | 同上 | P4 A1 | — | （没跑） |
  | 54 | 列 2 条 | 同上 | P4 A1 | — | （没跑） |
  | 55 | 文字不去首尾空白 | 同上 | P4 | — | （没跑） |
  | 56 | 各条之间用逗号隔开 | 同上 | P4 A1 | — | （没跑） |
  | 57 | 不写条数 | 同上 | P4 P6 A1 | LOC | （没跑） |
  | 58 | 不写 `+n` | 同上 | P4 A1 | — | （没跑） |
  | 59 | Labels 和 Label of 两个行名对调 | 同上 | P4 P5 | LOC | （没跑） |
  | 60 | Label of 不用位号 | 同上 | P5 A1 | — | （没跑） |
  | 61 | 有位号时写成「类型 · 位号」 | 同上 | P5 A1 | — | （没跑） |
  | 62 | 没有位号时只写类型 | 同上 | P5 A1 | LOC | （没跑） |
  | 63 | 没有位号时只写符号名 | 同上 | P5 A1 | — | （没跑） |
  | 64 | 没有位号时符号名写在类型前面 | 同上 | P5 A1 | — | （没跑） |
  | 65 | 类型和符号名都没有时显示成空 | 同上 | P5 | — | （没跑） |
  | 66 | 图例不在图上时写 Symbol | 同上 | P5 A1 | LOC | （没跑） |
  | 67 | 挂在线上的写成 Another object | 同上 | P5 A1 | LOC | （没跑） |
  | 68 | 其余的写成 A line | 同上 | P5 A1 | LOC | （没跑） |
  | 69 | 属性面板不接配对行 | `app/properties.rs` 的 `entity_property_sections` | A1 | — | （没跑） |
  | 70 | 选中几个对象时也加配对行 | `app/properties.rs` 的 `refresh_properties` | A1 | — | （没跑） |
  | 71 | 词条表里没有 Label of | `locale_catalog.rs` 的 `message_attribute` | — | LOC | （没跑） |
  | 72 | 词条表里没有 Not on the sheet | 同上 | — | LOC | （没跑） |
  | 73 | A line 指到 Another object 的译文 | 同上 | — | LOC | （没跑） |

  改坏点已删：加了开关的四个文件（`pid_identity.rs`、`scene/cache/properties.rs`、`app/properties.rs`、`locale_catalog.rs`）按副本还原，这次改动的 34 个文件的 sha256 与加开关之前逐个相同（`pre-mutant\sha256.txt`）；`src`、`tests`、`examples`、`locales`、`scripts` 里搜不到开关。固定门禁是还原之后重新编译跑的。带开关的改动留在 `with-mutants.diff`。还原之后只又改过一处：I7 的测试代码 2 行（见「读数」的第一条），所以提交的 `pid_identity.rs` 和改坏各轮跑的那一份差这 2 行；I7 的断言没有变。
- 没有红过的：
  - 新加的 19 条测试（I1–I9、P1–P7、A1、S5、LOC）每条都至少红过一次，但不是每一条断言都红过。
    - S5 在 7 处改坏下变红：3 处（第 11、13、15 处）红在图例的那组计数，1 处（第 23 处）红在「一条文字给了两个图例」，1 处（第 26 处）红在标注的合计，2 处（第 18、19 处）红在逐条文字的合计。没有红过的三条：「标注都是单行文字」「找到的图例就是文字写的那个编号」「找得到图例的文字都在那个图例的标注里」。
    - LOC 的 19 处：2 处红在「这种语言没有翻译」，1 处红在「英文等于原文」，4 处红在「面板没有这一行」，4 处红在简体中文的位号那组行，8 处红在简体中文的标注那组行。译文文件里的字没有改坏点（它们不走开关），所以这几条在 73 处下没有红过：「每种语言都有内容」、简体中文 11 条和繁体中文 8 条的逐条写法、保加利亚语之外 21 种语言各自的「不等于原文」。加译文之前 LOC 红在第一种语言（保加利亚语）的第一条上，后面的语言那时没有轮到；23 种语言各 8 条都在，是另用脚本数的（见「译文」）。
  - 改 `pid_identity.rs` 的 30 处里有 23 处只有拼出来的数据拦得住，样例测试里一条不红（第 1–10、12、14、16、17、20–22、24、25、27–30 处）。按改坏的内容看，是样例上没有那种情况（没有同号的两个图例、没有空值、没有不认识的标记、没有不在图上的图例），或者 S5 不问那一项；没有另外量，只是从改坏不红推出来的。
  - 原有的测试：K 在第 2 处下红（导入器读键现在用的是同一个函数），SB6 在第 52 处下红。CAP 在 73 处下没有红。快跑里另外的 114 个单元测试、`tests/pid_import.rs` 另外的 65 个，在各自跑过的改坏下一个没红。
  - 节点图面板（`src/app/node_graph.rs`）也走单选的那一页，那里会不会多出 Labels、Label of，没有测试看。
- 样例读数（S5 钉着；样例只带 `.pid`）：
  - 592 个块引用都读成图例：592 个有图形编号，592 个有对象编号和对象种类，592 个说得出类型；231 个有位号，来源全是图纸文件；没有一个带「图纸文件里的名字」。
  - 逐个图例问标注：合计 803 条，383 个图例至少有一条，803 条是 803 个不同的文字图元（没有一条给了两个图例），都是单行文字；一个图例最多 4 条。
  - 逐条散字问所属：958 条里挂在图例上 803（找到的图例的 `graphic` 都等于文字的 `label_of`），图例不在图上 0，挂在线上 151，挂在别的东西上 4，不是标注 0。和 SI-2 按记录钉的数相同。
- 耗时（验收第 2 条；S5 打在标准错误输出上的，`lookup-timing.txt`）：样例的模型空间 6,390 个图元，优化过的测试程序，跑了三遍，量的时候机器上有别的编译在跑。
  - 找一个图例的标注，592 次：平均 3,432、3,337、3,245 微秒；最慢的一次 6,057、7,342、12,172 微秒。
  - 找一条标注所属的图例，958 次：平均 316、283、287 微秒；最慢的一次 1,220、954、1,256 微秒。
  - 都低于 16 毫秒，没有建索引。属性面板在选中的对象变了的时候查一次，不是每画一帧查一次；记录里没有 `label_of` 的文字、不是图例的图元不扫模型空间。
- 译文：
  - 新加 8 条词条（23 种语言都加了），简体中文依次是：Name in file「图纸文件里的名字」、Labels「标注」、Label of「标注属于」、Backup database「备份数据库」、Item in the drawing file「图纸文件里的对象」、A line「一条管线」、Another object「其他对象」、Not on the sheet「不在图上」。
  - 复用词条表里已有的 3 条：Item type「图元类型」、Tag source「位号来源」、Drawing file「图纸文件」。最后一条在简体中文里原来是英文，这次改了；图纸集管理器也用这条词条，那里跟着变。
  - 词条表 7,687 条变 7,695 条；23 种语言各 9,682 个译文项变 9,690 个；词条表里的每一条在每种语言里都有对应的一项，没有重复（`locale-check-before.txt`、`locale-check-after.txt`）。
  - 仓库自带的 `scripts/test_locales.py` 不在固定门禁里，顺手跑了：是红的，上一个提交也是红的。它红在「代码里用了、词条表里没有的句子」，全量数下来是 806 处、120 个文件，这次改动前后完全相同，没有多出一处，都不在这次动的文件里（`locale-sources.txt`）。这次没有去修。
- 耗时预算（规格 §6 第 12 条；`io::pid::sample_budget`，测试没有改，没有放宽）：**这次也没有验证：量了五次，过了一次，空闲的机器上一次也没有量到。**
  - 门禁两遍各量一次，都是重新编译之后跑的，都红：第一遍导入 3 次 4,061、3,955、3,813 毫秒，中位数 3,955；第二遍 4,283、4,722、4,375，中位数 4,375。预算 3,747。第一遍量完 8 秒后记的整机占用是 100 %，有 12 个编译进程在跑。
  - 同一个测试程序不重新编译又跑三轮，每轮先等一分钟看机器会不会空下来（18:54–18:59，`budget-rounds-a.txt`）：中位数 3,939（红）、3,703（过）、5,313（红；这一轮的三次是 3,548、5,313、7,146，跑完时整机占用 100 %，35 个编译进程）。三轮前后记下的整机占用在 53 % 到 100 % 之间，编译进程 2 到 35 个，一轮也不算空闲（空闲的标准：没有编译在跑，整机占用不超过五分之一）。那些编译不是这项工作启动的，没有动。
  - 这次的改动在导入这条路上只有一处：导入器读键用的 `pid_value` 改成转出 `pid_identity::stated`，取值的办法没有变；新加的查找函数导入时不调，导入器写进图纸的东西没有变。没有像 SI-2 那样加分段计时去定责。
  - 所以「空闲的机器上中位数不超过 3,747」仍然没有验证，留给 SI-7 重量。前两个提交量的中位数：SI-1 是 3,440（过），SI-2 是 4,101（红，同样是机器不空闲）。
- 读数（固定门禁跑了两遍，下面是第二遍的数：18:43–18:54，`chainfinal2.txt`；耗时预算见上一条）：
  - 第一遍（18:27–18:42，`chainfinal.txt`）别的各项和下面相同，只有 clippy `--lib --profile test` 是 1,052 条，比上一个提交多 1 条，没过：I7 的测试代码先建一个默认值再给字段赋值（`field_reassign_with_default`）。改成一步建好，只动了测试代码 2 行，断言没有变；改完整套门禁从头重跑。
  - 全量单元测试 7,178 个：通过 7,124，忽略 54，失败 0（上一个提交的 7,107 加新加的 17）。
  - 集成测试 `tests/pid_import.rs` 66 个（上一个提交的 65 加新加的 1）、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个（原有的 1 加新加的 1）全过；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录和上一个提交的按「文件加告警内容」逐条比过（不比行号；`clippy-diff-lib.txt`、`clippy-diff-lib-test.txt`）：没有多出一条，也没有少一条；只有 `src/locale_catalog.rs` 原有的那条「函数在栈上分配 N 字节」的 N 从 870,148 变成 871,052（词条表多了 8 条）。点到这次动的文件的 17 条（测试配置 19 条）都是上一个提交就有的。
  - 格式：11 个 `.rs` 文件都是 `new=0`。
  - 分层报告与上一个提交逐行相同（`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1`）。
  - 改动：产品代码 +411 −16（`pid_identity.rs` 234，`scene/cache/properties.rs` +155 −3，`app/properties.rs` +8，`io/mod.rs` +1，`io/pid/metadata.rs` +5 −13，`locale_catalog.rs` +8）；测试 +1,514 −31（`pid_identity.rs` 389，`scene/cache/properties.rs` +433 −3，`io/pid/tests.rs` +156 −16，`io/pid/mod.rs` +6 −4，`app/pid_symbol_block_tests.rs` +154 −1，`tests/pid_import.rs` +142，`tests/pid_panel_localization.rs` +234 −7）；译文 +185 −1。
- 与清单不一样的地方：
  - 动到了清单「文件」之外的文件。产品：`src/io/mod.rs`、`src/io/pid/metadata.rs`、`src/app/properties.rs`、`src/locale_catalog.rs`；测试：`src/io/pid/tests.rs`、`src/io/pid/mod.rs`、`src/app/pid_symbol_block_tests.rs`、`tests/pid_import.rs`、`tests/pid_panel_localization.rs`。清单漏写了 `src/app/properties.rs`：要扫图纸的那两行得在拿得到整张图的地方接。按文件数这一项是 L，不是 M。
  - 清单写的是「加中文译文」，实际 23 种语言都加了：只加中文的话，别的语言里这几行会是英文，而同一节的其他行是翻译过的。
  - 验收下面加了一行不带勾的：规格 §6 第 12 条（耗时预算）这次也没有验证，留给 SI-7。SI-7 的「做什么」加了两件 SI-3 留下的事：在 53 张里图元最多的一张上量「找一个图例的标注」的耗时；另存成 DWG、DXF 再打开之后看属性面板。
- 没验证的：
  - 没有在带窗口的程序里看（规格 §6 第 7 条，留给 SI-7）。A1 不开窗口，走的是程序的点选和属性面板的数据；面板画出来是什么样、长的一行会不会被截断，没有看。
  - 机器空闲时的耗时预算（规格 §6 第 12 条）：见上面「耗时预算」那一条。
  - 查找的耗时只量了样例一张（6,390 个图元）。图元更多的图上会不会超过 16 毫秒，没有量，留给 SI-7。
  - 英文、简体中文之外 21 种语言的 8 条译文（包括繁体中文）没有母语的人校对过。
  - 复用的词条里，Item type 在 21 种语言里、Drawing file 在 14 种语言里是英文，是原有的缺口，这次没有补：这些语言里 Item type 的行名、Tag source 的「图纸文件」会显示英文。
  - 数据库的位号（`tag_source=database`、`label_in_file`）和 `_Data.xml` 的位号在面板上的行，只有拼出来的数据在钉（I2、I4、P2、P6、LOC）。真实图纸上带数据库位号的要等 SI-6；样例带 `_Data.xml` 时的面板没有量。
  - 另存成 DWG、DXF 再打开之后面板上的行（规格 §6 第 11 条）没有测。读法只看 `PID_SEMANTICS` 记录，SI-1、SI-2 钉过另存前后记录逐行相同，但「另存、再打开、看面板」这一条线没有一条测试，留给 SI-7。
  - 复制图例之后面板上的显示，只有 I9 在读记录这一层钉「同号取排在前面的」；在程序里真的复制一个图例再看面板，没有测。
  - 节点图面板没有看（见「没有红过的」）。
  - 出图没有重出（同 SI-1、SI-2，留给 SI-7）。这次没有改导入写进图纸的任何东西。
  - 改坏只跑了快跑的 133 个单元测试和两个集成测试目标；`tests/pid_import.rs` 只跑了前 30 处。全量单元测试里别的测试、`tests/pid_legend.rs` 没有在改坏下跑。改坏各轮跑的是改 I7 那 2 行之前的测试代码，改完没有重跑改坏。
  - 用户手册没有改。
- 证据：`D:\scratch\ocs-symblock\si-3\`（先红：`chainred.txt`、`quickred-1.log`、`integred.log`、`chainred2.txt`、`quickred2-1.log`；实现后的几遍：`chainmid.txt`、`chainprobe.txt`、`chaingreen1.txt` 和它们列的各份记录；耗时：`lookup-timing.txt`；译文：`apply-ftl-write.txt`、`locale-check-before.txt`、`locale-check-after.txt`、`test-locales-final.txt`、`locale-sources.txt`；改坏：`with-mutants.diff`、`pre-mutant\`、`libmut-m1.txt`、`libmut-m1-*.log`、`mut-m1p.txt`、`mut-m1p-*.log`、`mut-m1i.txt`、`mut-m1i-*.log`、`mutant-table-m1.md`；固定门禁：第一遍 `final-runfinal.txt`、`budgetfinal.log`、`chainfinal.txt`，第二遍 `final-runfinal2.txt`、`budgetfinal2.log`、`chainfinal2.txt`，和它们列的各份记录；clippy 记录和上一个提交的比较：`clippy-diff-lib.txt`、`clippy-diff-lib-test.txt`；耗时预算的轮次：`budget-rounds-a.txt`；改动行数：`split-lines.py` 的输出）。新加的脚本：`D:\scratch\ocs-symblock\tools\` 的 `apply-ftl.py`（把一组译文加进 23 个译文文件）、`locale-check.py`（数词条表和各语言的译文项，找缺的和重复的）；`si-3\` 的 `run-integ-mut.ps1`（两个集成测试目标各跑一轮改坏）、`mutant-table.py`（把三轮的结果合成上面那张表）、`split-lines.py`（把改动行数分成产品和测试）、`locale-sources.py`（按仓库脚本的办法全量数，比较改动前后）、`clippy-diff.py`（两份 clippy 记录按文件和告警内容比）、`apply-record.py`（把这份记录写进清单和规格）。都只读仓库，`apply-ftl.py` 和 `apply-record.py` 除外。

### SI-4 默认界面的检查栏 【接线】

- **做什么**：选中导入的图例或标注文字时，检查栏按规格 §4.6 只读显示。
- **验收**：
  - [x] 有位号的图例、没有位号的图例、没有发布类的图例、读不到对象的图例、三种标注文字、所属图例不在图上的标注，各有一条测试。
  - [x] 在本程序里画的对象、别的导入图元显示的内容不变（原有测试不改期望值）。
  - 没有验证，留给 SI-7：在带窗口的程序里看检查栏（规格 §6 第 7 条）；规格 §6 第 12 条（耗时预算），这次在最终的代码上量了四次，过了两次，机器一直不空闲，经过见下面「耗时预算」那一条。
- **依赖**：SI-3。
- **文件**：`src/app/pid_shell/editor.rs`、`src/app/pid_shell/pid_editor_screen.rs`。
- **规模**：S。

**做完**（2026-11-20）

- 改了什么（产品代码 +342 −4，只有 `src/app/pid_shell/editor.rs` 一个文件；测试 +786 −45；另有 `src/io/pid/mod.rs` +7 −5，只在测试里编译）：
  - `src/app/pid_shell/editor.rs`：选中一个没有 `PID_AUTHOR` 记录的对象时，检查栏先看它是不是 `.pid` 导入放的图例，或者一条标注的文字。是图例，显示位号、类型、符号、位号来源、图纸文件里的名字和它的标注；是标注文字，显示文字内容和它挂在什么上。都只读，没有可以输入的行。两样都不是的照原来显示「<图元种类> · 不是 P&ID 对象」。没有加面板，没有加按钮。
    - `Inspected` 多两种：`Symbol(ImportedSymbol)`、`Label(ImportedLabel)`；`inspected()` 在没有 `PID_AUTHOR` 记录的那一支先问 `imported()`。
    - `imported()`：图元自己记录里的东西现读；要扫图的那一部分（`look_up()`：图例的标注文字，或者标注的归属那句话）第一次问时查，存在 `State` 的 `paired` 里，`selection_refreshed()` 把它丢掉。
    - `class_name()` 和 11 条的名字表、`kind_stated()`、`source_shown()`、`owner_shown()`：下面「显示的规则」里的各种写法。
    - 画法：`imported_symbol()`、`imported_label()`，和它们共用的 `headline()`、`hairline()`。
  - 读记录、找标注、找所属图例，用的都是 SI-3 的 `src/io/pid_identity.rs`，这次没有动它。经典界面的属性面板没有动；导入器没有动。
  - 测试：`src/app/pid_shell/pid_editor_screen.rs` 加 7 条测试和 6 个辅助函数（+675 −1，去掉的一行是文件开头的 `use`）。`src/io/pid/tests.rs`（+89 −29）加一张拼出来的图 `sheet_of_symbols_the_file_says_less_of`：一个没有定发布类的 OPC、一个读不到对象的图例、一个发布成 `PIDBranchPoint` 的图例，前两个各带一条标注；拼图用的两段代码拆成 `lettering_at`、`valve_placed`，和 SI-3 那张图共用，那张图的数据没有变。`src/io/pid/mod.rs` 把新图转出给界面测试用。`src/app/pid_symbol_block_tests.rs`（+22 −15）把「把一张图放进最前面的标签页」拆成 `put_in_front`，和 `lettered` 一起放开给这里用。
- 显示的规则（规格 §4.6；和规格不一样的、规格没有写到的，逐条记在规格 §11 的 SI-4）：
  - 图例，标题的位置：位号；没有位号写「（没有位号）」。
  - 类型（总有这一行）：发布类在名字表里的写中文名，不在表里的照原文；没有发布类写对象种类（如 `OPC`）；两样都没有写「图纸里没有写」。
  - 名字表 11 条，就是规格 §2.2 那张表里的 11 个发布类：`PIDPipingComponent` 管道元件、`PIDInstrument` 仪表、`PIDControlSystemFunction` 控制系统功能、`PIDInlineInstrument` 在线仪表、`PIDNote` 注释、`PIDNozzle` 管口、`PIDProcessEquipment` 工艺设备、`PIDMechanicalEquipment` 机械设备、`PIDProcessVessel` 容器、`PIDHeatTransferEquipment` 换热设备、`PIDPipingConnector` 管段。按整个类名比，分大小写。
  - 符号：符号名；记录里没有符号名就没有这一行。
  - 位号来源（有位号才有这一行）：`file`「图纸文件」；`database`「备份数据库」；有位号而没有来源标记，是 `_Data.xml` 给的，写「_Data.xml」；这一版不认识的标记照原文。
  - 图纸文件里的名字：记录里有 `label_in_file` 就有这一行。
  - 标注：一行「标注」，同一行右边写条数「n 条」；下面每条文字一行，按图纸里的先后，去掉首尾空白，最多列 6 条；多出来的在最后写「另有 n 条」。一条也没有的写「0 条」。
  - 标注文字：第一行是文字内容（去掉首尾空白），第二行是归属。所属的图例有位号：「属于 <位号>」。没有位号：「属于 <类型> · <符号名>」，类型的取法同上（不写「图纸里没有写」），两样里只有一样就只写那一样，两样都没有写「属于一个没有位号的图例」。图例不在图上：「所属的图例不在图上」。挂在线上：「管线上的标注」。其余：「其他对象上的标注」。
  - 什么时候查：找一个图例的标注、找一条标注所属的图例，都要扫一遍模型空间，而检查栏每来一条消息就重画一次。所以检查栏第一次问到某个标签页上的某个图元时查一次，把结果存下（记着是哪个标签页、哪个图元）；选择刷新（`refresh_properties`：选中的对象变了，或者它的属性变了；撤销、重做也走这里）时丢掉，下次问再查。问到另一个图元、另一个标签页，存着的不用，现查，并改存新查到的。图元自己记录里的东西（位号、类型、符号、来源）每次重画现读，不存。
- 简称（下面用）：
  - E1 `the_inspector_shows_what_the_drawing_file_says_an_imported_symbol_is`
  - E2 `the_inspector_shows_a_symbol_the_file_says_less_of_by_what_it_does_say`
  - E3 `the_inspector_says_where_a_tag_came_from_and_lists_six_labels`
  - E4 `the_inspector_says_what_the_label_of_a_selected_text_hangs_on`
  - E5 `the_inspector_shows_what_else_is_selected_as_it_did`
  - E6 `what_the_sheet_says_of_the_selection_is_looked_up_once_and_kept_until_a_refresh`
  - E7 `the_inspector_names_the_classes_a_drawing_file_publishes_its_items_as`
  - 原有的三条，没有改：O `the_inspector_shows_the_selected_objects_record_and_writes_to_it`、K `the_inspector_says_what_kind_of_thing_is_selected`、R `a_row_typed_in_is_dropped_when_another_object_is_selected`
- 验收的每一种情况对哪条测试。7 条都在 `src/app/pid_shell/pid_editor_screen.rs`，用的是拼出来的图；每条都比 `inspected()` 给的数据，E1–E5 另外不开窗口把界面排一遍，按先后收检查栏里的字逐条比：

  | 情况 | 测试 | 钉住的 |
  |---|---|---|
  | 有位号的图例 | E1 | 标题 GV-101，类型「管道元件」，符号 Ball Valve，位号来源「图纸文件」，标注「4 条」和 4 条文字（按图纸里的先后，排在图例前面的那条也在）；没有可以输入的行 |
  | 没有位号的图例 | E1 | 标题「（没有位号）」，没有位号来源一行，标注「1 条」 |
  | 没有发布类的图例 | E2 | 类型写对象种类 `OPC` |
  | 读不到对象的图例 | E2 | 类型写「图纸里没有写」 |
  | 发布类不在名字表里的图例（清单没有列） | E2 | 类型照原文 `PIDBranchPoint`；它一条标注也没有，写「0 条」 |
  | 挂在图例上的标注文字 | E4 | 图例有位号：「属于 GV-101」，文字排在图例前面的也一样；没有位号：「属于 管道元件 · Ball Valve」；类型、符号名都没有：「属于一个没有位号的图例」。E2 另有两种：「属于 OPC · Ball Valve」、只有符号名的「属于 Ball Valve」 |
  | 挂在线上的标注文字 | E4 | 「管线上的标注」 |
  | 挂在别的东西上的标注文字 | E4 | 「其他对象上的标注」 |
  | 所属图例不在图上的标注 | E4 | 删掉图例（`ERASE`）之后「所属的图例不在图上」；撤销之后又是「属于 GV-101」 |
  | 位号来源的另外三种、图纸文件里的名字（清单没有列） | E3 | `database`「备份数据库」，同时有「图纸文件里的名字」一行；没有来源标记「_Data.xml」；不认识的标记照原文。这几种是改记录拼出来的：现在还没有哪一种导入会给出数据库的位号 |
  | 标注超过 6 条（清单没有列） | E3 | 6 条：全列，没有「另有」；8 条：列前 6 条，写「另有 2 条」；带首尾空白的文字去掉空白，选中那条文字时内容也去掉 |
  | 别的导入图元不变 | E5 | 不是标注的文字、管线（记录里另写上标注的两个键也一样，也不算那个图例的标注）、记录不说自己是图例的块引用、没写挂在什么上的文字：都还是「<图元种类> · 不是 P&ID 对象」；什么都不选、同时选中图例和它的一条标注，也和原来一样 |
  | 本程序里画的对象不变 | E5；O、K、R 原样通过 | 从符号库放的一个容器，记录里另外写上图例的键（`role=symbol`、`graphic`、`label`），仍按它的 `PID_AUTHOR` 记录显示 |
  | 查一次，存到选择刷新（清单没有列） | E6 | 见下 |
  | 名字表（清单没有列） | E7 | 11 个类名各自的中文名；`PIDBranchPoint`、全小写的、末尾带空格的、空的都没有名字 |

- E6 钉住的（它不画界面，只问 `inspected()`；「背着程序改」是直接改图元的记录，不通知任何东西）：
  - 选中阀门、问过一次之后，背着程序把它的一条标注改挂到另一个阀门上、把它的位号改掉：标注还是原来的 4 条，位号马上是新的。调 `refresh_properties` 之后标注是 3 条。
  - 不经刷新，直接把选择换成另一个阀门：给的是它自己的两条标注，不是存着的那一份；再背着程序改，还是那两条（新查到的也存下了）。
  - 同样换成一条标注文字：归属是现查的；再背着程序改，不变。
  - 第二个标签页里放同一张图、选中同一个编号的阀门（4 条），再把第一个标签页换到前面、不刷新：给的是第一张图的 3 条。
- 先红：
  - 实现之前（只加了测试要用到的类型和函数的空壳）：快跑 133 个，通过 126，失败 6，忽略 1（`quickred-2.log`）。红的 6 条是 E1、E2、E3、E4、E6、E7。E5 钉的是「不变」，实现之前就是绿的；它在改坏表的第 21、22、24、29、38、40 处红。再前面一遍（`quickred-1.log`）没有编过，是测试代码自己的错（一个辅助函数重名、调用多传了参数），和要测的东西无关。
  - 那一遍里 E6 还是第一版的写法（那时叫 `what_the_sheet_says_of_the_selection_is_looked_up_when_the_selection_is_refreshed`，钉的是「选择刷新时查」）。办法换成「第一次问时查」之后，E6 改了名字，后半段重写：「不经刷新换选另一个图元、另一条标注文字时现查并存下」这几条断言没有单独先红过，它们的红在改坏表里：换选另一个阀门给的是它自己的标注（第 26 处），新查到的也存下（第 31 处），换选标注文字时归属是现查的（第 3、4、7、14、15、16、25 处）；「第一次问时就存下」红在第 30 处。
  - 实现之后（第一版）：快跑 132 过、1 忽略（`quickgreen-1.log`）。最终的代码见「读数」。
- 故意改坏 48 处。开关只加在 `editor.rs`，用环境变量打开；每处跑 `pid_editor_screen` 的 22 条（新加的 7 条和原有的 15 条）。不改坏时 22 条全过；48 处每一处都有测试变红（`libmut-m4.txt`）：

  | # | 改坏 | 改在哪 | 变红的测试 |
  |---|---|---|---|
  | 1 | 选择刷新时不丢掉存着的结果 | `selection_refreshed` | E3 E4 E6 |
  | 2 | 换了选择，正在输入的行不丢 | 同上 | R |
  | 3 | 类别一律没有中文名 | `class_name` | E1 E3 E4 E6 E7 |
  | 4 | 类别拿到表里下一项的中文名 | 同上 | E1 E3 E4 E6 E7 |
  | 5 | 类别名不分大小写 | 同上 | E7 |
  | 6 | 类别名按前缀比 | 同上 | E7 |
  | 7 | 类型先取对象种类，再取发布类 | `kind_stated` | E1 E2 E3 E4 E6 |
  | 8 | 表里没有的发布类不显示，改显示对象种类 | 同上 | E2 |
  | 9 | 没有发布类时不显示对象种类 | 同上 | E2 |
  | 10 | 「图纸文件」和「备份数据库」对调 | `source_shown` | E1 E2 E3 |
  | 11 | 没有来源标记的位号写成「图纸文件」 | 同上 | E3 |
  | 12 | 不认识的来源写成「_Data.xml」 | 同上 | E3 |
  | 13 | 标注的归属不用位号 | `owner_shown` | E3 E4 |
  | 14 | 没有位号时只写类型 | 同上 | E2 E4 E6 |
  | 15 | 没有位号时只写符号名 | 同上 | E2 E4 E6 |
  | 16 | 没有位号时符号名写在类型前面 | 同上 | E2 E4 E6 |
  | 17 | 类型和符号名都没有时写成空的「属于 」 | 同上 | E4 |
  | 18 | 图例不在图上时写成「其他对象上的标注」 | 同上 | E4 |
  | 19 | 挂在线上的写成「其他对象上的标注」 | 同上 | E4 |
  | 20 | 其余的写成「管线上的标注」 | 同上 | E4 |
  | 21 | 图例的标注不看图形编号，挂在图例上的都算 | `look_up` | E1 E2 E3 E5 E6 |
  | 22 | 不是文字的图元也列进标注（空一条） | 同上 | E5 |
  | 23 | 标注列表里的文字不去首尾空白 | 同上 | E3 |
  | 24 | 标注的先后倒过来 | 同上 | E1 E3 E5 E6 |
  | 25 | 标注文字不查归属 | 同上 | E2 E3 E4 E6 |
  | 26 | 存的结果只认图纸，不认是哪个图元 | `imported` | E6 |
  | 27 | 存的结果只认图元编号，不认是哪张图 | 同上 | E6 |
  | 28 | 存的结果不用，每次现查 | 同上 | E6 |
  | 29 | 没有存的结果时不查 | 同上 | E1 E2 E3 E4 E5 E6 |
  | 30 | 查到的结果不存 | 同上 | E6 |
  | 31 | 已经存着别的图元的结果时，新查到的不存 | 同上 | E6 |
  | 32 | 图纸文件里的名字和位号对调 | 同上 | E3 |
  | 33 | 不显示位号 | 同上 | E1 E2 E3 E6 |
  | 34 | 图纸里没有写类型时显示成空 | 同上 | E2 |
  | 35 | 不显示符号名 | 同上 | E1 E2 E3 |
  | 36 | 不显示位号来源 | 同上 | E1 E2 E3 |
  | 37 | 不显示图纸文件里的名字 | 同上 | E3 |
  | 38 | 带标注键的图元不是文字也当标注显示 | 同上 | E5 |
  | 39 | 选中的标注文字不去首尾空白 | 同上 | E3 |
  | 40 | 先按导入的图例看，再看是不是本程序画的对象 | `inspected` | E5 |
  | 41 | 没有位号时标题留空 | `imported_symbol`（画法） | E1 E2 |
  | 42 | 各行画成可以输入的 | 同上 | E1 E2 E3 |
  | 43 | 标注最多列 5 条 | 同上 | E3 |
  | 44 | 标注最多列 7 条 | 同上 | E3 |
  | 45 | 条数写成列出来的条数 | 同上 | E3 |
  | 46 | 正好 6 条时也写「另有 0 条」 | 同上 | E3 |
  | 47 | 「另有 n 条」的 n 写成总条数 | 同上 | E3 |
  | 48 | 标注文字的归属写在文字内容前面 | `imported_label`（画法） | E4 |

  这是第 4 轮，跑在最终的代码上。前三轮跑在第一版上（`libmut-m1.txt`、`libmut-m2.txt`、`libmut-m3.txt`）：第 1 轮 48 处红了 47 处，没红的一处是「界面不在前面时也查」，给 E6 补了断言之后第 2 轮重跑 4 处、第 3 轮重跑 48 处，全红。第一版的办法后来换掉了（见「与清单不一样的地方」），它特有的 3 处改坏跟着去掉，换成这一版的 3 处（第 1、30、31 处），其余 45 处按原来的意思搬到新代码上重跑。

  改坏点已删：`editor.rs` 按副本还原，sha256 与加开关之前相同；`src`、`tests` 里搜不到开关。固定门禁是还原之后重新编译跑的。带开关的改动留在 `with-mutants2.diff`。
- 没有红过的：
  - 新加的 7 条每条都至少红过一次，不是每一条断言都红过（各处改坏红在哪一行：`m4-panic-lines.txt`）。
  - E5 红过的断言是三条：管线写上标注的键仍不当标注显示（第 38 处）、管线不算那个图例的标注（第 22 处；第 21、24 处也红在这条断言上，那是标注列表本身变了）、本程序画的容器仍按自己的记录显示（第 40 处）；第 29 处红在取图例显示内容的辅助函数上。别的几条（不是标注的文字、没改记录时的管线、记录不说自己是图例的块引用、没写挂在什么上的文字、什么都不选、选中两个对象）钉的是原来的行为，实现之前就成立，48 处下也没有红过：这几种判断在 SI-3 的 `pid_identity.rs` 里或者在原有的代码里，这次没有在那里加改坏点。
  - E6 里两条断言没有单独红过：「标注的归属也是存着的」（背着程序改了之后归属不变）和「第二个标签页里问到的是 4 条」。会让它们红的改坏（第 28、30 处）先红在前面的断言上，测试到那里就停了。
  - 原有的测试：R 在第 2 处红；O、K 和 `pid_editor_screen` 另外 12 条原有的测试在 48 处下一条没红。
  - 画法的细节没有测试：字号、颜色、间距、分隔线、标题前的图标。测试只按先后收检查栏里的字。
- 耗时预算（规格 §6 第 12 条；`io::pid::sample_budget`，测试没有改，没有放宽）：**这次也没有验证：在最终的代码上量了四次，过了两次，空闲的机器上一次也没有量到。**
  - 门禁那一遍（重新编译之后跑的，20:01）：导入 3 次 3,557、3,980、3,803 毫秒，中位数 3,803，预算 3,747，红。这一遍没有记整机占用。
  - 同一个测试程序不重新编译又跑三轮，每轮先等一分钟看机器会不会空下来（20:12–20:17，`budget-rounds-a.txt`）：中位数 3,365（过）、3,989（红）、3,628（过）。三轮前后记下的整机占用在 45 % 到 68 % 之间，别的编译进程 0 到 2 个（不是这项工作启动的，没有动），一轮也不算空闲（空闲的标准：没有编译在跑，整机占用不超过五分之一）。
  - 第一版的代码上也量过一次（19:47，`final-runfinal.txt`）：中位数 3,732，过。
  - 这次的产品代码只动了检查栏，导入这条路上一行没有改。
  - 「空闲的机器上中位数不超过 3,747」仍然留给 SI-7 重量。
- 读数（固定门禁，还原之后重新编译跑的：19:58–20:12，`final-runfinal2.txt`、`chainfinal2.txt`；耗时预算见上一条）：
  - 全量单元测试 7,185 个：通过 7,131，忽略 54，失败 0（上一个提交的 7,178 加新加的 7）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录和上一个提交的按「文件加告警内容」逐条比过（不比行号；`clippy-diff-lib.txt`、`clippy-diff-lib-test.txt`）：`--lib` 没有多出一条，也没有少一条；`--lib --profile test` 只有 3 条原有的「函数在栈上分配 N 字节」的 N 各大了 640（`src/app/collab/collab_large_props.rs` 两条、`src/app/collab/collab_offline_queue.rs` 一条）。应该是界面的状态多了一个字段、这几个测试函数的栈上放着几份程序的状态；没有另外去量。点到这次动的文件的只有 1 条，是 `src/io/pid/mod.rs` 原有的「参数太多」。
  - 格式：5 个 `.rs` 文件都是 `new=0`。
  - 分层报告与上一个提交逐行相同（`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1`）。
  - 第一版也开始跑过一遍门禁（19:44，`chainfinal.txt`），换办法时停掉了。那时已经出来的：格式有一处没过（`editor.rs` 里一行该折行没折，已改），快跑三组过；全量单元测试以后的各项没有跑完。上面的数都是最终代码上的这一遍。
- 与清单不一样的地方：
  - 动到了清单「文件」之外的三个文件，都是给测试用的：`src/io/pid/tests.rs`、`src/io/pid/mod.rs`、`src/app/pid_symbol_block_tests.rs`。
  - 做了两版。第一版在选择刷新时把结果查好存下，检查栏问到别的图元时现查不存。自查时发现切换标签页（`Message::TabSwitch`）不走 `refresh_properties`：切到另一张图之后存着的那一份对不上，检查栏每重画一次就要扫一遍模型空间，直到下一次选择变化。换成现在的办法：第一次问时查并存下，刷新时只丢掉。第一版的门禁停掉重跑，改坏三轮作废重跑。
  - 规格 §1.1 说程序里已经有发布类的中文名，实际没有；11 个名字是这次定的（规格 §11 SI-4 第 1 条）。
  - 清单的验收列了八种情况；另外给发布类不在名字表里的图例、位号来源的另外三种、图纸文件里的名字、标注超过 6 条、查一次存到选择刷新、名字表加了测试。
  - 验收下面加了一行不带勾的：带窗口的查看和耗时预算留给 SI-7。SI-6 的「做什么」加了一句：图纸显示之后才补位号的话，补完要调一次 `refresh_properties`。
  - 规模：清单写 S。产品代码一个文件 342 行，测试 786 行。
- 没验证的：
  - 没有在带窗口的程序里看（规格 §6 第 7 条，留给 SI-7）。测试不开窗口：比 `inspected()` 的数据，再把界面排一遍、收检查栏里的字。画出来是什么样、长的文字会不会被截断、标注多的时候能不能滚动、深浅两种主题下的样子，没有看。
  - 11 个中文名是这次自己定的写法，没有对过 SmartPlant 的中文版，也没有请用户定。
  - 真实图纸上的检查栏没有测：7 条测试用的都是拼出来的图。样例上读记录和查找的结果由 SI-3 的样例测试钉着，检查栏用的是同一组函数，但「打开样例、选中、看检查栏」这一条线没有测试，留给 SI-7 带窗口做。
  - 数据库的位号（「备份数据库」、图纸文件里的名字）和 `_Data.xml` 的位号，只有改记录拼出来的在钉。真的从备份面板打开要等 SI-6。
  - 带 `_Data.xml` 导入时发布类是 `_Data.xml` 给的，会有名字表之外的类，照原文显示。样例上是那 2 个 OPC（`PIDBranchPoint`）；别的图纸的 `_Data.xml` 还会给出哪些，没有量。
  - 查找的耗时这次没有重量，用的是 SI-3 在样例上量的（找一个图例的标注平均 3.2–3.4 毫秒，最慢 12.2 毫秒）。「重画时不再扫」是从 E6 的「背着程序改了之后显示不变」推出来的，没有数扫了几遍。图元更多的图留给 SI-7 量。
  - 图纸变了而没有走 `refresh_properties` 的地方（如果有），检查栏上的标注和归属要到下一次刷新才跟上，和属性面板上 SI-3 加的那两行是同一个时机。程序里改图纸的每一条路是不是都走它，没有逐条查；查过的只有撤销、重做（走）和切换标签页（不走，所以换了办法）。
  - 存着的结果按标签页和图元编号认，不认图纸的内容。一个标签页里的图纸整张换掉、选择又原样留着的情况（如果有）会不会走 `refresh_properties`，没有查。
  - 改坏只跑了 `pid_editor_screen` 的 22 条；全量单元测试里别的测试、三个集成测试目标没有在改坏下跑。
  - 出图没有重出（导入器没有动）。用户手册没有改。
- 证据：`D:\scratch\ocs-symblock\si-4\`（先红：`quickred-1.log`、`quickred-2.log`；实现后：`quickgreen-1.log`；改坏：第一版 `with-mutants.diff`、`pre-mutant\`、`libmut-m1.txt`、`libmut-m2.txt`、`libmut-m3.txt`、`mutant-table-m3.md`，最终 `with-mutants2.diff`、`pre-mutant2\`、`libmut-m4.txt`、`libmut-m4-*.log`、`mutant-table-m4.md`、`m4-panic-lines.txt`；固定门禁：第一版没跑完的 `final-runfinal.txt`、`chainfinal.txt`，最终 `final-runfinal2.txt`、`budgetfinal2.log`、`chainfinal2.txt` 和它列的各份记录；clippy 记录和上一个提交的比较：`clippy-diff-lib.txt`、`clippy-diff-lib-test.txt`；耗时预算的轮次：`budget-rounds-a.txt`；最终的改动：`editor-final.diff`；改动行数：`split-lines.py` 的输出）。新加的脚本都在 `si-4\`：`mutant-table.py`（把一轮改坏的结果排成上面那张表）、`remutant.py` 和 `remutant-fix.py`（把第一版的开关搬到重做后的代码上）、`split-lines.py`（改动行数分成产品和测试）、`apply-record.py`（把这份记录写进清单和规格）。都只读仓库，`remutant.py`、`remutant-fix.py`（只写带开关的那一个文件，已还原）和 `apply-record.py` 除外。

### SI-5 `PIDTAG` 认导入的图例 【接线】

- **做什么**：按规格 §4.7。
- **验收**：
  - [x] 按位号选中图例和全部标注；几个图例同一个位号时都选中；找不到时的提示不变。
  - [x] 不带参数时，点中图例或它的一条标注都能选中整组；没有位号的图例也行。
  - [x] 改位号的写法对导入的图例不生效，有一句提示。
  - [x] 在 DXF、DWG 图纸上（图例识别的结果）行为不变（原有测试不改期望值）。
  - 没有验证，留给 SI-7：在带窗口的程序里用 `PIDTAG` 找（规格 §6 第 8 条；从备份面板打开后用阀门位号找的那一半还要等 SI-6）；规格 §6 第 12 条（耗时预算），这次在最终的代码上量了四次，过了三次，机器一直不空闲，经过见下面「耗时预算」那一条。
- **依赖**：SI-3。
- **文件**：`src/app/commands/pidlegend.rs`。
- **规模**：S。

**做完**（2026-11-20）

- 改了什么（产品代码 +207 −11，只有 `src/app/commands/pidlegend.rs` 一个文件；测试 +985 −27）：
  - `src/app/commands/pidlegend.rs`：`PIDTAG` 除了图例识别认出的符号，也找 `.pid` 导入放的图例。按位号找、不带参数按点中的图元找，两条路都接上了：选中图例的块引用和它的全部标注文字，每个图例画一个红框，视图缩放过去。`PIDTAG <旧位号> <新位号>` 不改导入的图例，有一句话说明。没有加命令，没有加面板。
    - `dispatch_pidtag`：两条路各把「问的是什么」（`Asked::Tag`、`Asked::Picked`）交给 `pid_groups_select`。找不到时、点中的东西不成组时、什么都没点时的三句提示没有动。
    - `pid_groups_select`（新；原来 `pid_group_select` 里的代码搬到这里）：识别出的符号照原来成组；给了 `Asked` 时再加上 `imported_groups` 找到的导入图例。`pid_group_select` 留给图例清单的行用，不问导入的图例。
    - `imported_groups`（新）和 `Asked`、`ImportedGroup`：扫一遍模型空间，给出问到的导入图例，各带它的标注文字和回执里用的名字。
    - `retag_by_tag`：先数带旧位号的导入图例有几个，再分三种情况（下面「规则」的倒数第二条）。
    - 文件开头的说明加了一段。
  - 读记录用的是 SI-3 的 `src/io/pid_identity.rs`（`symbol_identity`、`lettering`、`label_pairing`），这次没有动它。导入器、图例识别、图例清单面板、画框和缩放的代码都没有动；没有加词条，没有动译文文件。
  - 测试：`pidlegend.rs` 加 13 条测试和 13 个辅助函数（+958，原有的测试代码一行没有动）。「背着程序改一个图元的记录」的辅助函数 `restate` 从 `src/app/pid_shell/pid_editor_screen.rs`（+1 −27）搬到 `src/app/pid_symbol_block_tests.rs`（+26），两个测试文件共用；函数体没有变。
- 规则（规格 §4.7；和规格不一样的、规格没有写到的，逐条记在规格 §11 的 SI-5）：
  - 哪些算：导入的图例是模型空间里记录写了 `role=symbol`、又没有 `PID_AUTHOR` 记录的块引用；它的标注是 `label_on=symbol` 而且 `label_of` 等于它的 `graphic` 的单行、多行文字。和属性面板的 Labels 一行、检查栏的「标注」一块是同一批文字。一组就是这个块引用加这些文字。
  - `PIDTAG <位号>`：敲的位号和记录里的 `label` 都转成大写，整个相等才算，和找识别出的符号时的比法相同。带这个位号的识别出的符号和导入的图例都选中，各画一个红框，视图缩放到全部。一个也没有时照原来报错 `PIDTAG: no symbol tagged "<位号>" on this sheet -- PIDLEGEND LIST shows the tags.`，选择不动。
  - 不带参数的 `PIDTAG`：选中的图元一个一个看。是导入图例的块引用，选这个图例；是一条挂在图例上的标注的文字，选它挂的那个图例；没有位号的图例一样。同时点中几个，各自的图例都选。点中的东西不属于任何图例（线上的标注、线、别的对象上的标注、不是标注的字、所属的图例已经不在图上的标注）时照原来提示 `PIDTAG: the picked entities belong to no recognised symbol.`，选择不动。
  - 几个图例写着同一个图形编号时（复制出来的图例，规格 §11 SI-3 第 6 条）：它们共用一批标注，每条标注只选一次、只数一次；点其中一条标注，选的是图纸里排在最前的那个图例，也就是属性面板 Label of 写的那个；点块引用选的是它自己。
  - 回执：入选的里面有导入的图例时写 `PIDTAG: <名字> + <名字> -- N symbol(s), M label(s) selected and framed in red.`。N 是入选的图例数，识别出的符号也算；M 是各组里文字的条数，同一条只数一次，识别出的符号的位号文字也算一条。入选的全是识别出的符号时照原来写 `N group(s), M entities selected and framed in red.`。先后：识别出的符号在前，导入的图例在后，各按图纸里的先后。
  - 名字：符号名加位号，如 `Ball Valve GV-101`。没有符号名时取类型（发布类的原文；没有发布类取对象种类，如 `OPC`）；两样都没有写「图例」。没有位号写 `(无位号)`，和识别出的符号原来的写法相同。
  - 本程序里画的对象（有 `PID_AUTHOR` 记录）不算导入的图例，记录里即使另外写着图例的键也一样。它仍占着它写的图形编号：它在写着同一个编号的图例里排在最前时，点这个编号的标注不选任何导入的图例。
  - 图例清单的行是识别出的符号。点一行（`Message::PidLegendPickSymbol`）只在识别出的符号里找，和原来一样。
  - `PIDTAG <旧位号> <新位号>`：改的只是本程序里画的对象，和原来一样。带旧位号的只有导入的图例：拒绝，错误码 `tag_imported`，句子是「PIDTAG: 本图上带位号 "<旧位号>" 的是 N 个导入的图例，导入的图例不能在这里改位号，未做改动。」；不写记录，不进撤销，不标成已修改。本程序里画的对象和导入的图例都带：改前者，一步撤销，回执后面加「；另有 N 个导入的图例带这个位号，导入的图例不能在这里改位号，没有改」。都不带：照原来报 `tag_absent`。
  - 什么时候查：每执行一次命令扫一遍模型空间，不建索引，不存结果。
- 简称（下面用）：
  - T1 `pidtag_finds_an_imported_symbol_by_its_tag_with_all_its_labels`
  - T2 `pidtag_selects_every_imported_symbol_that_carries_the_tag`
  - T3 `pidtag_compares_the_tag_of_an_imported_symbol_case_aside`
  - T4 `pidtag_refuses_a_tag_no_imported_symbol_carries_as_it_did`
  - T5 `pidtag_reads_a_picked_imported_symbol_or_any_one_of_its_labels`
  - T6 `pidtag_makes_no_group_of_a_pick_that_is_no_imported_symbols`
  - T7 `a_legend_list_row_goes_by_the_recognised_symbols_alone`
  - T8 `a_rename_is_refused_where_imported_symbols_alone_carry_the_tag`
  - T9 `a_rename_takes_the_objects_drawn_here_and_leaves_the_imported_symbols`
  - T10 `imported_groups_are_what_the_identity_functions_make_of_each_pick`
  - T11 `the_receipt_names_an_imported_symbol_by_its_symbol_and_its_tag`
  - T12 `on_the_sample_drawing_every_tag_and_every_label_finds_its_symbol`
  - T13 `pidtag_selects_recognised_and_imported_symbols_side_by_side`
  - 原有的五条，没有改：G `pidtag_selects_frames_and_zooms_to_the_tagged_group`、P `pidtag_reads_the_picked_symbol_and_refuses_an_unknown_tag`、L `a_legend_list_symbol_row_selects_and_frames_the_group`、W `a_two_word_pidtag_renames_the_tag_in_one_undo_step`、A `a_rename_of_an_absent_tag_is_refused_by_code`
- 验收的每一条对哪条测试。13 条都在 `src/app/commands/pidlegend.rs`。T12 用样例图纸（不在时跳过，说一句），T13 用一张现画的图，别的用 SI-3、SI-4 那两张拼出来的图。T1–T6、T8、T9、T13 走命令行，T7 走图例清单的消息，比的是选中的图元、命令行上的句子、红框的个数；T10–T12 直接问 `imported_groups`：

  | 验收 | 测试 | 钉住的 |
  |---|---|---|
  | 按位号选中图例和全部标注 | T1 | `PIDTAG gv-101`：选中阀门和它的 4 条标注，别的不选；回执 `Ball Valve GV-101 -- 1 symbol(s), 4 label(s)`；视图动了；一个红框，在阀门和标注外面 1 毫米；选中的画成红色 |
  | 几个图例同一个位号时都选中 | T2 | 两个阀门带同一个位号：2 个图例、5 条标注都选中，各一个框，第二个框的范围对。第二个阀门改成第一个的副本（同一个图形编号）之后，共用的 4 条标注只选一次、只数一次 |
  | 位号的比法不变（清单没有单列） | T3 | 记录里写的是小写 `gv-102a`：敲 `GV-102A`、`gv-102a`、`Gv-102A` 都找到；回执照记录里的写法 |
  | 找不到时的提示不变 | T4 | 六个找不到的：没有人带的 `XV-9999`；标注上的字 `DN50`、`DN80`；线上标注的字 `P-1001-50-CS`；位号的前一截 `GV-10`、比位号多一个字的 `GV-1011`。都是原来那句，选择不动，不画框 |
  | 不带参数，点中图例或它的一条标注 | T5 | 点阀门、点排在阀门前面的那条标注、点另一条标注，选中的都是阀门和 4 条标注。一个图例的一条标注加另一个图例的块引用：两组都选中，2 个图例、5 条标注、2 个框 |
  | 没有位号的图例也行 | T5 | 点没有位号的阀门、点它的标注，选中它和它的 1 条标注；回执 `Ball Valve (无位号) -- 1 symbol(s), 1 label(s)` |
  | 点中的东西不属于任何图例（清单没有列） | T6 | 线上的标注、线、不在图上的图形的标注、不是标注的字：原来那句提示，选择不动，不画框。删掉图例（`ERASE`）之后点它的标注也是那句；撤销之后又选中整组。什么都没点：原来那句「先点一个」 |
  | 改位号对导入的图例不生效，有一句提示 | T8 | 带旧位号的只有导入的图例：错误码 `tag_imported`，句子里写个数（1 个、2 个各一例），位号照敲的写法；图例的记录一个字没变，没有变成本程序的对象，没有撤销步，没有标成已修改。没有人带的位号仍是 `tag_absent` 和原来那句 |
  | 同上，本程序画的对象也带这个位号时 | T9 | 改本程序画的那个（位号、来源 `manual`），一步撤销，回执加「另有 1 个导入的图例带这个位号，导入的图例不能在这里改位号，没有改」；导入图例的记录一个字没变。没有导入的图例带这个位号时回执和原来一样 |
  | DXF、DWG 图纸上行为不变 | G、P、L、W、A 原样通过；T13；T7 | 原有的测试代码一行没有动。T13：一张图上既有识别出的符号又有导入的图例，都带 `BUV-3101`。按位号两个都选中，`2 symbol(s), 2 label(s)`，2 个框；只点识别出的符号的位号文字，仍是 `1 group(s), 2 entities`；只点导入图例的标注，是 `1 symbol(s), 1 label(s)`。T7：图例清单点行，给的是导入图例的图元编号时什么都不选，原来选中的不动，照原来说「that symbol is no longer on the sheet」 |
  | 和属性面板、检查栏说的是同一批（清单没有列） | T10 | 两张拼出来的图上，每个图元单独点一次，`imported_groups` 给的和用 `pid_identity` 那组函数一步一步查出来的相同，成组的个数是 5 和 7。再改记录拼出五种导入不会留下的情况：线上的标注写上阀门的编号（仍不算）、线自己写上标注的两个键（不是文字，不算）、一条多行文字做标注（算）、阀门改成另一个的副本（共用标注，点标注选排在前面的那个，一起点各带全部标注）、阀门加上 `PID_AUTHOR` 记录（不算；它的编号下的标注点了不选谁；按它的位号问也没有） |
  | 回执里的名字（清单没有列） | T11 | 有符号名：`Ball Valve (无位号)`、`Ball Valve ZZ-9`。去掉符号名之后：`OPC (无位号)`（对象种类）、`图例 (无位号)`（两样都没有）、`PIDBranchPoint ZZ-9`（发布类） |
  | 样例图纸（清单没有列） | T12 | 见下一条 |

- 样例上的读数（T12；只带 `.pid` 导入，模型空间 6,390 个图元）：
  - 图例 592 个，带位号的 231 个；不同的位号 229 个，同一个位号最多 2 个图例带。
  - 标注文字 958 条：挂在图例上的 803 条，线上的 151 条，别的对象上的 4 条，所属的图例不在图上的 0 条。各组里的标注合计 803 条；没有标注的图例 209 个（592 减去 SI-2 量的 383）；一个图例最多 4 条。
  - 一次把 6,390 个图元都算点中：592 个图例各成一组，和逐个图例用 `pid_identity` 那组函数查出来的相同。229 个位号逐个问：给的都是带这个位号的那几个图例的组。958 条标注逐条点：挂在图例上的给它的图例那一组，别的什么都不给。
  - 图例识别在这张图上认出的符号 0 个，没有把哪个导入的图例认成自己的。
  - 耗时（优化过的测试程序，同一个程序连跑三遍，`sample-timing-a.txt`；三遍前后整机占用 41 %–56 %，没有别的编译在跑）：一次全选 6.5、8.3、9.7 毫秒；229 个位号逐个问合计 1.35、1.22、1.51 秒，平均每次 5.3–6.6 毫秒；958 条标注逐条点合计 5.72、5.68、7.01 秒，平均每次 5.9–7.3 毫秒（含测试自己用 `label_owner` 对答案的时间）；图例识别一遍 10.1、9.6、17.1 毫秒。只量了合计，没有量最慢的一次。
- 先红：
  - 这一项是先写的实现和测试，再回头跑的先红：把新写的查找 `imported_groups` 临时换成「什么都找不到」，别的代码不动，跑这个模块的 36 条：通过 22，失败 12，忽略 2（`red-2.log`）。再前面一遍（`red-1.log`）没有编过，是测试代码自己的错（一个多行文字图元没有装箱），和要测的东西无关。带桩的那一版文件没有留副本。
  - 红的 12 条里，10 条红在各自第一条和功能有关的断言上：T1、T2、T3、T5、T13 红在选中的图元上，T8 红在错误码上，T9 红在回执上，T10 红在和 `pid_identity` 的比较上，T11 红在名字上，T12 红在「每个图例一组」上。
  - 另外 2 条（T6、T7）那一遍也红了，但红在测试自己的错上：命令行给提示句加的「ⓘ 」前缀，期望值里漏了。改了期望值，改完没有在桩上重跑。T6 的红在改坏表第 1、16、17、24、25 处，T7 的红在第 41 处。
  - T4 那一遍是绿的：它钉的是「找不到时的提示不变」。它在改坏表第 29、30 处红。
  - 那一遍之后测试还加过东西，都没有在桩上跑过：T10 的「两个同编号的图例一起点中时各带全部标注」（红在改坏表第 36、40 处）；T12 的 11 个数里的 4 个（不同的位号 229、同一个位号最多 2 个图例、没有标注的图例 209、一个图例最多 4 条）先写成 0，从失败信息里读出来再填，另外 7 个是 SI-1、SI-2、SI-3 已经钉过的数，直接写的。
  - 去掉桩之后第一遍（`green-1.log`）：通过 33，失败 1，忽略 2。失败的就是 T12 那 4 个写成 0 的数，另外 7 个一次对上。填上之后就是最终的代码。
- 故意改坏 41 处。开关只加在 `pidlegend.rs`，用环境变量打开；每处跑这个模块的 34 条（新加的 13 条和原有的 21 条；另有 2 条原有的要真实图纸，一直是忽略的）。不改坏时 34 条全过；41 处每一处都有测试变红（`libmut-m1.txt` 40 处，`libmut-m2.txt` 1 处）：

  | # | 改坏 | 改在哪 | 变红的测试 |
  |---|---|---|---|
  | 1 | 点中的那一路不找导入的图例 | `dispatch_pidtag` | T5 T6 T13 |
  | 2 | 按位号的那一路不找导入的图例 | 同上 | T1 T2 T3 T13 |
  | 3 | 按位号找导入图例时用敲进来的写法，不转大写 | 同上 | T1 T3 |
  | 4 | 改位号时不看导入的图例（数成 0 个） | `retag_by_tag` | T8 T9 |
  | 5 | 改位号时数导入图例用敲进来的写法，不转大写 | 同上 | T8 |
  | 6 | 只要有导入的图例带旧位号就整个拒绝，本程序画的也不改 | 同上 | T9 |
  | 7 | 导入的图例有两个以上才拒绝 | 同上 | T8 |
  | 8 | 拒绝的那句话里个数总写 1 | 同上 | T8 |
  | 9 | 拒绝时错误码写成 `tag_absent` | 同上 | T8 |
  | 10 | 回执里总带「另有 n 个导入的图例」那半句 | 同上 | T9 |
  | 11 | 回执里从不带那半句 | 同上 | T9 |
  | 12 | 识别符号的位号文字不计入标注数 | `pid_groups_select` | T13 |
  | 13 | 已经有识别符号入选时不找导入的图例 | 同上 | T13 |
  | 14 | 没有导入图例入选也按「几个图例、几条标注」写 | 同上 | T13 G |
  | 15 | 有导入图例入选也按「几组、几个图元」写 | 同上 | T1 T2 T3 T5 T13 |
  | 16 | 导入图例的组里只有块引用，没有标注 | 同上 | T1 T2 T3 T5 T6 T13 |
  | 17 | 导入图例的组里只有标注，没有块引用 | 同上 | T1 T2 T3 T5 T6 T13 |
  | 18 | 导入的图例并进前一组，少画一个框 | 同上 | T2 T5 T13 |
  | 19 | 回执里导入图例的名字是空的 | 同上 | T1 T2 T3 T5 T13 |
  | 20 | 标注数按各组相加，共用的标注重复计 | 同上 | T2 T13 |
  | 21 | 回执里图例数和标注数对调 | 同上 | T1 T2 T5 |
  | 22 | 不是文字的图元带着标注的键也算标注 | `imported_groups` | T10 |
  | 23 | 挂在线上、别的东西上的标注也算图例的标注 | 同上 | T10 |
  | 24 | 每条标注都当成被点中 | 同上 | T5 T6 T10 T12 T13 |
  | 25 | 点中标注不选图例 | 同上 | T5 T6 T10 T12 T13 |
  | 26 | 本程序画的对象不占图形编号 | 同上 | T10 |
  | 27 | 本程序画的对象也算导入的图例 | 同上 | T10 |
  | 28 | 图例的位号不转大写就比 | 同上 | T3 |
  | 29 | 位号以敲的字开头就算 | 同上 | T4 |
  | 30 | 敲的字以位号开头就算 | 同上 | T4 |
  | 31 | 点中块引用自己不选它 | 同上 | T5 T10 T11 T12 |
  | 32 | 点中标注时带同一编号的图例都选，不只第一个 | 同上 | T10 |
  | 33 | 名字先取类型，再取符号名 | 同上 | T1 T2 T3 T5 T11 |
  | 34 | 符号名和类型都没有时名字留空 | 同上 | T11 |
  | 35 | 没有符号名时不取类型 | 同上 | T11 |
  | 36 | 带同一编号的第二个图例不带标注 | 同上 | T10 |
  | 37 | 一组里标注的先后倒过来 | 同上 | T10 T12 |
  | 38 | 名字里不带位号 | 同上 | T1 T2 T3 T5 T11 T13 |
  | 39 | 没有位号时不写「(无位号)」 | 同上 | T5 T11 |
  | 40 | 图例的先后倒过来 | 同上 | T5 T10 T11 T12 |
  | 41 | 图例清单点行的那一路也找导入的图例（整张图的都算） | `pid_group_select` | T7 |

  前 40 处是一轮跑的（20:49–21:07）。逐处核对时发现这 40 处没有一处让 T7 变红，补了第 41 处（21:32–21:35）：让图例清单点行的那一路也去找导入的图例，T7 红，别的 33 条不红。

  改坏点已删：`pidlegend.rs` 按副本还原，sha256 与加开关之前相同；`src`、`tests` 里搜不到开关。固定门禁是前 40 处还原之后重新编译跑的。第 41 处是门禁之后跑的：跑完按同一份副本还原，sha256 与门禁时相同，门禁没有重跑。带开关的两版文件留在 `impl\pidlegend-2-mutants.rs`、`impl\pidlegend-2-mutants2.rs`。
- 没有红过的：
  - 新加的 13 条每条都至少红过一次（T7 只在补的第 41 处红过），不是每一条断言都红过。各处改坏红在哪一行：`m1-panic-lines.txt`。
  - 画框和缩放：T1 的「视图动了」「一个框」「框的范围」「选中的画成红色」，T2、T5、T13 的框的个数和 T2 的第二个框的范围，都没有单独红过。会动到它们的改坏（如第 18 处少画一个框）先红在前面的选中或回执上；画框、缩放用的是原有的代码，这次没有在那里加改坏点。
  - 钉「不变」「不动」的断言没有单独红过：T4 的选择不动、不画框；T6 的提示原文、不画框、删掉图例后它的标注不成组、什么都没点时的提示；T7 的提示原文；T8 的 1 个时的句子原文、图例的记录没变、没有变成本程序的对象、没有撤销步、没有标成已修改、`tag_absent`；T9 的导入图例的记录没变、一步撤销。其中「导入图例的记录没变」在这次改动之前就成立（那时改位号根本不看导入的图例），要让它红得另写一段去改记录的代码，开关做不到。
  - T10 红过的是两条：每个图元单独点时和 `pid_identity` 的结果相同（辅助函数里的比较，9 处）、两个同编号的图例一起点时各带全部标注（2 处）。写死的个数（5、7、7、7、8、6、1）和逐个写死的期望没有单独红过：改坏之后先停在前一条比较上。
  - T12 红过的是三条：每个图例一组（第 31 处）、一次全选和逐个查的相同（第 37、40 处）、逐条点标注（第 24、25 处）。「229 个位号逐个问」那一段在改坏下没有红过：动位号比法的五处里，第 2、3 处改在命令那一层，这条测试直接问 `imported_groups`；第 28、29、30 处在样例上不红，应该是样例上的位号本来都是大写、也没有哪个位号是另一个的开头，没有另外去查。「图例识别不把导入的图例认成自己的」和写死的 11 个数在改坏下也没有红过；11 个数里的 4 个在去掉桩之后的第一遍红过（见「先红」）。
  - T13 的框的个数、最后一句回执没有单独红过。
  - 原有的测试：G 在第 14 处红；P、L、W、A 和这个模块另外 16 条原有的测试在 41 处下一条没红。
- 耗时预算（规格 §6 第 12 条；`io::pid::sample_budget`，测试没有改，没有放宽）：**这次也没有验证：在最终的代码上量了四次，过了三次，空闲的机器上一次也没有量到。**
  - 门禁那一遍（重新编译之后跑的，21:08–21:12）：导入 3 次 4,667、4,924、4,595 毫秒，中位数 4,667，预算 3,747，红。这一遍没有记整机占用。
  - 同一个测试程序不重新编译又跑三轮，每轮先等一分钟看机器会不会空下来（21:23–21:27，`budget-rounds-a.txt`）：中位数 3,741、3,371、3,744，三轮都过，其中两轮离预算只差 6 和 3 毫秒。三轮前后记下的整机占用在 43 % 到 64 % 之间，别的编译进程 0 到 2 个（不是这项工作启动的，没有动），一轮也不算空闲（空闲的标准：没有编译在跑，整机占用不超过五分之一）。
  - 这次的产品代码只动了 `PIDTAG`，导入这条路上一行没有改。
  - 「空闲的机器上中位数不超过 3,747」仍然留给 SI-7 重量。
- 读数（固定门禁，前 40 处改坏还原之后重新编译跑的：21:12–21:23，`chainfinal.txt`；耗时预算见上一条）：
  - 全量单元测试 7,198 个：通过 7,144，忽略 54，失败 0（上一个提交的 7,185 加新加的 13）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录和上一个提交的按「文件加告警内容」逐条比过（不比行号；`clippy-diff-lib-b.txt`、`clippy-diff-lib-test-b.txt`）：都没有多出一条，也没有少一条；点到这次动的三个文件的 0 条。
  - 格式：3 个 `.rs` 文件都是 `new=0`。
  - 分层报告与上一个提交逐行相同（`io -> app 3`、`io -> modules 1`、`io -> patreon 1`、`io -> ui 1`）。
  - 这个模块单独跑：34 过，2 忽略，0 失败。
- 与清单不一样的地方：
  - 动到了清单「文件」之外的两个文件，都是给测试用的：`src/app/pid_symbol_block_tests.rs`、`src/app/pid_shell/pid_editor_screen.rs`（搬 `restate`）。
  - 清单的验收是四条；另外给位号的比法、点中的东西不属于任何图例、回执里的名字、和 `pid_identity` 逐个图元对、样例图纸、图例清单点行、两种图例在一张图上加了测试。
  - 验收下面加了一行不带勾的：带窗口的 `PIDTAG` 和耗时预算留给 SI-7。SI-7 的「做什么」加了一句：图元最多的那张图上也量 `PIDTAG` 的查找。
  - 改位号被拒绝时多了一个错误码 `tag_imported`。带旧位号的只有导入的图例这种情况，原来报的是 `tag_absent`「本图上没有位号 … 的符号」。
  - 先红是实现写好之后用桩跑的，不是先于实现；带桩的文件没有留副本（见「先红」）。
  - 第 41 处改坏是固定门禁之后补的，门禁没有重跑（见「故意改坏」）。
  - 规模：清单写 S。产品代码一个文件 207 行，测试 985 行。
- 没验证的：
  - 没有在带窗口的程序里跑 `PIDTAG`（规格 §6 第 8 条，留给 SI-7）。测试不开窗口：比选中的图元、命令行上的句子、红框的个数和范围、视图有没有动。红框和红色画出来的样子、缩放之后的画面，默认界面和经典界面各一次，都没有看。
  - 规格 §6 第 8 条的前一半是「从备份面板打开后，用一个阀门的位号找」。现在图纸上只有图纸文件给的位号（样例 231 个）；数据库的位号要等 SI-6 写进记录。`PIDTAG` 读的是记录里的 `label`，不管是哪一头写的，但「数据库的位号写进去之后找得到」没有测。
  - 样例上走命令行的那一条线没有测试：T12 直接问 `imported_groups`，不执行 `PIDTAG`；走命令行的 9 条用的都是拼出来的图。「打开样例、敲 `PIDTAG` 加位号、看选中和画面」留给 SI-7 带窗口做。
  - 带 `_Data.xml` 导入的图、另存 DWG 或 DXF 再打开的图上没有测（规格 §6 第 11 条，留给 SI-7）。
  - 样例之外的 52 张没有跑；图元最多的图上的耗时没有量（留给 SI-7）。
  - 耗时只量了查找那一步的合计和平均，机器不空闲；最慢的一次、整条命令（图例识别要不要重读、选中、画框、缩放）没有量。
  - 一张图上既有识别出的符号又有导入的图例，只有 T13 现画的一张在测；真实图纸上没有找过这样的图。
  - 新的错误码 `tag_imported`：自动化和控制通道把错误码原样交给调用方。仓库里没有按 `tag_absent`、`tag_imported` 分支的代码；仓库外面有没有按 `tag_absent` 判断的调用方，没有查。`docs/plans/2026-11-10-mission-p1-collab-status.md` 里写着「找不到旧位号时错误码 `tag_absent`」的那一行没有改。
  - 协作：`PIDTAG <旧位号> <新位号>` 是协作约定里改位号的写法。改的对象和原来一样，回执后面可能多半句；多出来的半句、被拒绝的情况在协作那一头是什么样，没有测。
  - 命令行上的句子没有进词条表（`src/locale_catalog.rs` 里原来就没有 `PIDTAG` 的句子）。新加的句子跟着各自那一路原来的语言：选中的回执是英文，改位号的两句是中文；`(无位号)`、「图例」是中文。没有译文。
  - 本程序画的对象占着图形编号的那一种情况，只有改记录拼出来的在钉；程序里有没有哪一步会给导入的图例加上 `PID_AUTHOR` 记录，没有查。
  - 改坏只跑了这个模块的 34 条；全量单元测试里别的测试、三个集成测试目标没有在改坏下跑。
  - 出图没有重出（导入器没有动）。命令的提示、自动补全、帮助没有改。仓库里没有写 `PIDTAG` 用法的用户手册，没有补。
- 证据：`D:\scratch\ocs-symblock\si-5\`（先红：`red-1.log`、`red-2.log`；去掉桩之后：`green-1.log`；实现的第一版和最终版：`impl\pidlegend-1.rs`、`impl\pidlegend-2.rs`；改坏：`impl\pidlegend-2-mutants.rs`、`libmut-m1.txt`、`libmut-m1-*.log`、`impl\pidlegend-2-mutants2.rs`、`libmut-m2.txt`、`libmut-m2-*.log`、`mutant-table-m2.md`、`m1-panic-lines.txt`；固定门禁：`final-runfinal.txt`、`chainfinal.txt` 和它们列的各份记录；clippy 记录和上一个提交的比较：`clippy-diff-lib-b.txt`、`clippy-diff-lib-test-b.txt`；耗时预算的轮次：`budget-rounds-a.txt`；样例上的耗时：`sample-timing-a.txt`、`sample-timing-a-*.log`；最终的改动：`pidlegend-final.diff`、`helpers.diff`；改动行数：`split-lines-b.txt`）。新加的脚本都在 `si-5\`：`make-mutants.py`、`make-mutants-2.py`（给 `pidlegend.rs` 加开关，前 40 处和第 41 处）、`mutant-table.py`（把两轮改坏的结果排成上面那张表）、`panic-lines.py`（各处改坏红在最终文件的哪一行）、`sample-timing.ps1`（样例那条测试连跑几遍，记耗时和整机占用）、`split-lines-b.py`（改动行数分成产品和测试）、`apply-record.py`（把这份记录写进清单和规格）。都只读仓库，`apply-record.py` 除外；带开关的文件是另外拷进工作树的，已还原。

### SI-6 从备份面板打开时用数据库补位号 【接线】

- **做什么**：备份的数据库读好后，用 `ExpTables::stored_column` 取 `T_PlantItem.ItemTag`，备成「对象编号 → 位号」的表；从面板打开的图纸按规格 §4.4 补位号。先定办法（导入时查表，还是显示后补一遍），写进记录。图纸显示之后才补的话，补完要调一次 `refresh_properties`：属性面板的行和检查栏存着的查找结果要到刷新时才换（SI-3、SI-4 留下的）。
- **验收**：
  - [x] 规格 §6 第 2 条（样例 354 个，全是 `database`）。
  - [x] 规格 §4.4 的五条各有测试或读数：数据库没读好时图纸先显示、不在界面线程上读、不标成已修改、读不全时不补、直接打开文件时不补。
  - [x] 从真实的数据库取出 8,145 个位号（备份不在时跳过）。
  - 没有验证，留给 SI-7：在带窗口的程序里从备份面板打开图纸（规格 §6 第 2、13 条在窗口里的那一遍，第 8 条的前一半），上面三条勾的都是不开窗口的测试；规格 §6 第 12 条（耗时预算），这次在最终的代码上量了四次，超了三次，机器一直不空闲，经过见下面「耗时预算」那一条。
- **依赖**：SI-1。
- **文件**：`src/io/pid_backup_browse/exp_views.rs`、`src/app/commands/pidbackup.rs`，另加办法定下来之后要动的文件。
- **规模**：M。这一项的办法还没定，是清单里最不确定的一项。

**做完**（2026-11-20）

- 办法（清单要求先定）：**图纸导入好之后再补，不在导入时查表。** 补位号只有一个函数（`io::pid::fill_database_tags`，改的是已经导入好的图纸上图例的记录），两条路都用它：
  - 这个备份的位号已经读好：图纸文件读完、显示之前补（`on_file_opened` 里加的一步）。图纸一显示，位号就是数据库的。
  - 还没读好：图纸先显示，位号是图纸文件里的名字；位号读完的消息到了，补给等着的图纸，界面不等。
  - 理由：导入不知道图纸是不是从面板打开的，也不该等数据库；「还没读好时图纸先显示」本来就要一个能在导入之后单独跑的补法，已经读好的那条路就也用它，不另写一套。导入器一行没有动。
- 改了什么（产品代码 +534 −4，12 个文件；测试 +1,895 −3；23 个语言文件各 +3 行）：
  - `src/io/pid/database_tags.rs`（新，97 行）：`fill_database_tags(图纸, 位号表, 哪些是本程序里画的)`，给三个数 `DatabaseTagFill { tagged, differing, changed }`：带着数据库位号的图例数、其中图纸文件里的名字不同的、这一遍改了记录的。
  - `src/io/pid/metadata.rs`（+23）：`clear_pid_key`，从 `PID_SEMANTICS` 记录里拿掉一个键。`src/io/pid/mod.rs`（+2）：挂上新模块。
  - `src/io/pid_backup_browse/exp_views.rs`（+85 −1，其中 3 行是只在测试里编译的计数）：`Dump::item_tags`（`T_PlantItem.ItemTag` 整列用 `ExpTables::stored_column` 取出来备成「对象编号 → 位号」的表，算一次存着）、`tags_of_cells`（单元格变位号）、`item_tags_in_background` 和 `on_tags_thread`（在名叫 `ocs-pid-backup-tags` 的线程上读）。
  - `src/app/commands/pidbackup/tags.rs`（新，产品 276 行）：`BackupTags`（面板最后要的一张图、正在读位号的备份、读到的结果、等着的图纸、等属性面板重读的图纸）和六个函数：`open_from_the_backup`（面板里双击一张图）、`tag_the_drawing_the_backup_opened`（图纸读完时）、`backup_tags_read`（位号读完时）、`show_the_tags_taken`（每条消息之后）、`put_the_backup_tags_on`（补，并在命令行说一句）、`fill_with_database_tags`（把图纸从标签页取出来补，再放回去）。
  - 接线：`src/app/commands/pidbackup.rs`（+14 −1：面板的「打开图纸」改走 `open_from_the_backup`；接 `TagsRead` 消息）、`src/ui/window/pid_backup_browser.rs`（+10：消息 `PidBackupMsg::TagsRead`）、`src/app/update/file.rs`（+12 −1：`open_id_to_come` 预告下一次打开用的号；`on_file_opened` 里的那一步）、`src/app/update/mod.rs`（+4：`update` 里每条消息之后问一次 `show_the_tags_taken`）、`src/app/mod.rs`（+7：字段 `pid_backup_tags`）、`src/app/commands/mod.rs`（+1 −1：子模块对 `app` 可见）。
  - 词条：`src/locale_catalog.rs`（+3）和 23 个 `locales/*/opencadstudio.ftl`（各 +3）。简体中文有译文，其余 22 种放的是英文原文。
  - 测试 28 条：`tags.rs` 18 条和 1 条要真实备份的（+1,217），`src/io/pid/tests.rs` 5 条（+513），`exp_views.rs` 3 条和 1 条要真实备份的（+158）；`src/io/pid/mod.rs`（+7 −3）把两个拼图纸的辅助函数交给 `tags.rs` 的测试用。
  - 没有动：导入器、图例识别、属性面板和检查栏的代码、`PIDTAG`、备份面板的界面。
- 规则（规格 §4.4、§4.2 第 1 级；和规格不一样的、规格没有写到的，逐条记在规格 §11 的 SI-6）：
  - 哪些图纸补：在备份面板里双击打开的图纸。别的打开方式都不补，文件就是备份里的那一个也不补。双击一张已经开着的图纸只是切过去，不补。
  - 什么时候读数据库：一个备份第一次从面板打开图纸时读，读一次，在工作线程上；面板别处已经把这个备份的数据库读进来时不再读文件，只取这一列。读到的表留给这个备份后面打开的图纸。
  - 补哪些图例：模型空间里记录写了 `role=symbol` 的块引用；有 `PID_AUTHOR` 记录的（本程序里画的）不补。
  - 怎么补：按记录里的 `item` 查表，对象编号按数据库的写法折过再比。查到了：`label` 写位号，`tag_source` 写 `database`；图纸文件里的名字和位号不同时写进 `label_in_file`，相同或者没有时不写，原来有的清掉。查不到：整条记录不动。
  - `_Data.xml` 给的位号也换成数据库的，不写 `label_in_file`。
  - 补不算修改：已修改标记、修改序号、撤销记录都不动。记录已经全对的图例不重写，补几遍结果一样。
  - 数据库读不全、读不了、里面没有这一列：不补，命令行一句警告，这个备份后面打开的图每张一句，不重读。备份里没有可读的数据库文件：不读，不说。
  - 补了位号，命令行一句：几个图例带着数据库的位号，其中几个和图纸文件里的名字不同。
  - 显示之后才补的图纸，属性面板和检查栏在它到前面时重读一遍；它本来就在前面的，补完当时就读。
- 简称（下面用）：
  - `src/app/commands/pidbackup/tags.rs`：
    - A1 `a_drawing_opened_from_the_panel_takes_the_tags_that_are_read`
    - A2 `a_drawing_is_shown_before_the_tags_are_read_and_takes_them_when_they_are`
    - A3 `the_tags_go_to_the_drawing_that_waits_for_them`
    - A4 `a_drawing_waits_for_the_tags_of_its_own_backup`
    - A5 `tags_that_arrive_before_the_drawing_asked_for_are_kept_for_it`
    - A6 `tags_are_not_put_on_a_tab_that_holds_another_file`
    - A7 `tags_that_find_their_drawing_closed_are_put_on_nothing`
    - A8 `a_database_that_does_not_read_leaves_the_names_of_the_file`
    - A9 `the_tags_of_one_backup_are_not_put_on_a_drawing_of_the_next`
    - A10 `a_drawing_opened_from_its_file_keeps_the_names_of_the_file`
    - A11 `an_open_that_began_before_the_ask_leaves_it_for_the_one_to_come`
    - A12 `a_backup_without_a_database_is_asked_for_no_tags`
    - A13 `an_ask_that_came_to_nothing_is_forgotten_at_the_next`
    - A14 `a_symbol_drawn_in_this_program_takes_no_tag`
    - A15 `the_id_to_come_is_the_id_the_next_open_goes_by`
    - A16 `a_drawing_tagged_behind_another_shows_its_tags_once_it_is_in_front`
    - A17 `a_drawing_tagged_behind_another_and_closed_there_is_forgotten`
    - A18 `a_drawing_given_no_tags_has_its_panel_left_as_it_is`
    - R2 `the_sample_opened_from_its_backup_carries_the_tags_of_the_database`（要真实备份，平时忽略）
  - `src/io/pid/tests.rs`：
    - F1 `a_key_taken_out_leaves_the_rest_of_its_record`
    - F2 `the_database_tag_of_an_item_is_its_symbols_tag`
    - F3 `the_same_tags_put_on_again_write_nothing`
    - F4 `only_a_symbol_of_the_file_takes_the_databases_tag`
    - F5 `the_databases_tag_replaces_the_one_data_xml_published`
  - `src/io/pid_backup_browse/exp_views.rs`：
    - D1 `a_tag_is_the_text_of_its_cell`
    - D2 `a_dump_not_read_whole_gives_no_item_tags`
    - D3 `the_item_tags_are_read_on_a_thread_of_their_own`
    - R1 `the_plant_database_gives_its_items_their_tags`（要真实备份，平时忽略）
- 测试用的图和表。A1–A18、F2–F4 用同一张拼出来的图（`sheet_for_the_database`）：六个阀门，依次是文件里叫 `GV-101` 的管件、文件里没有名字的管件、文件里叫 `PIA-0112101B` 的仪表（对象编号写成带花括号、连字符、小写的）、文件里叫 `N1` 的管口、没有对象的图形、文件里叫 `K-1` 的管件。位号表（`tags_for_that_sheet`）五条：第一个给 `GV-101`（和文件里的相同），第二个给 `BV1301102B`，第三个给 `PIA-old_0112101A`（两头带空白），第六个给的全是空白，还有一条是别的图上的对象。下面说「补对了」，指的是：前三个阀门带数据库的位号，第三个另记 `label_in_file=PIA-0112101B`，后三个不动；命令行一句 `P&ID item tags: 3 symbol(s) of A-101.pid carry the tag the backup database gives their item; 1 differ from the name in the drawing file`。A 开头的测试不读图纸文件，也不读数据库：图纸和位号表是直接交给程序的，走的是面板的消息、打开的消息和程序的 `update`。
- 验收的每一条对哪条测试：

  | 验收 | 测试 | 钉住的 |
  |---|---|---|
  | 规格 §6 第 2 条：样例从备份面板打开，354 个有 `label`，全是 `database`，没有 `label_in_file` | R2 | 见下面「真实备份上的读数」 |
  | §4.4 数据库没读好时图纸先显示 | A2、A3、A16 | 共用的准备步骤：位号正在读时面板双击，打开结束时图纸在前面，六个阀门是图纸文件里的名字，图纸进了等待名单。A2：位号到了，补对了。A3：位号到的时候图纸在别的图纸后面、面板也已经关掉备份：照样补给它，前面那张不动；位号不再留着。A16：见最后一行 |
  | §4.4 不在界面线程上读 | D3、A8 | D3：读是在名叫 `ocs-pid-backup-tags` 的线程上做的，发起的线程不叫这个名字。数据库文件打不开时，结果是「没读成」加原因，而发起的线程自己一次数据库也没读（按线程计数；同一个线程自己去读一次，计数加一）。线程中途没了算没读成，给一句原因。A8：双击刚返回时数据库还没有读（`DumpCell(not read)`），读是双击交出去的任务做的，任务跑完才变成 `unreadable` |
  | §4.4 不标成已修改 | A1、A2、A3、A5、A17 | A2：位号到的时候图纸上正开着一个撤销步；补完已修改标记没有，修改序号没变，那一步结束后撤销记录是空的。A1、A5（显示之前补）：已修改标记没有；A1 另比撤销记录是空的。A3：在后面补的那张已修改标记没有。A17：补过位号的图纸，关标签页时直接关掉了（已修改的图纸关不掉，要先问保存） |
  | §4.4 读不全时不补 | D2、A8、A18 | D2：读入器没读全的数据库，给的是为什么没读全，不给已经读到的那些表里的位号；再问一次还是这句。A8：数据库文件读不了：图纸照样打开，六个阀门保持图纸文件里的名字，已修改标记没有；命令行一句警告，写着原因和文件名，命令行的错误计数没有变；这个备份的第二张图不再读数据库，同样不补，再给一句。A18：等着的图纸等来的是「没读成」：记录不动，一句警告，不再等，属性面板不重读 |
  | §4.4 直接打开文件时不补 | A10 | 面板里装着备份、位号已经读好、文件就是备份里的图纸，直接打开：保持图纸文件里的名字，命令行不说话。另两种也不补：面板要了一张图、打开被取消，接着直接打开同一个文件；面板要了一张文件不在的图（没有开始打开），接着直接打开另一个文件。两次的要求都已经作废 |
  | 从真实的数据库取出 8,145 个位号 | R1 | 见下面「真实备份上的读数」 |
  | §4.4 数据库已经读好时，图纸一显示位号就是数据库的（清单没有单列） | A1、A5、A11 | A1：位号已经读好，面板双击：打开结束时图纸已经补对了，没有再读数据库。A5：位号比图纸文件先到，面板这时已经关掉了备份：位号留着，命令行不说话；图纸读完时补对了。A11：见「认出面板要的那一次打开」 |
  | §4.4 按对象编号到 `T_PlantItem.ItemTag` 里取（清单没有单列） | D1、F2、R1 | D1：单元格是文字才算位号，去掉两头的空白，全是空白的不算，中间的空格留着。F2：编号写成带花括号、连字符、小写的对得上；位号两头的空白去掉；全是空白的位号不算。R1：真实备份里的这一列 |
  | §3、§4.2 第 1 级：`tag_source=database`，`label_in_file` 只在不同时写（清单没有单列） | F2 | 三个阀门的整条记录逐字比：同名的不写 `label_in_file`；文件里没名字的，`label` 写在记录里位号该在的地方；不同的写 `label_in_file=PIA-0112101B`。数据库没有位号的三个整条记录不变，别的图元的记录不变。三个数是 3、1、3 |
  | §4.4 补几遍结果一样（清单没有单列） | F3、R2 | F3：第二遍三个数是 3、1、0，记录逐字相同。换一张表（第三个的位号变得和文件里的名字相同）：`label_in_file` 清掉，三个数是 1、0、1，另外五个不变。R2：样例第二遍改 0 个 |
  | 只补图纸文件自己的图例（清单没有列） | F4、A14 | F4：`role` 不是 `symbol` 的块引用、带着图例那几个键的一条线、调用方说是本程序里画的块引用，三样的记录都不变，只补了一个。A14：带 `PID_AUTHOR` 记录的阀门从面板打开时不补，另外两个照补，命令行说的是 2 个 |
  | `_Data.xml` 给的位号（清单没有列） | F5 | `_Data.xml` 给了位号的阀门：换成数据库的，标 `database`，不写 `label_in_file`，`class`、`oid`、`resolved` 不变。同一张图上位号是图纸文件给的那个：照常，写 `label_in_file=PG-7`。三个数是 2、1、2 |
  | 拿掉一个键（清单没有列） | F1 | 键名要整个相等：拿 `label_in`、`label_in_file_too`、`oid` 去清，记录不变。清 `label_in_file`：写了两次的都清掉，别的值（含一个不是字符串的）和它们的先后不变，记录在图元上的先后不变，别的应用的记录不变。没有记录的图元不会多出一条记录 |
  | 位号给对图纸（清单没有列） | A4、A6、A7、A9 | A4：别的备份的位号到了：不补，不说，接着等，那份位号不留；自己的到了才补。A6：等着的标签页里的文件换了（另存成别的名字）：不补，不说，不再等。A7：等着的图纸关了、同一个文件又直接打开：位号到了不补给它，不说。A9：上一个备份的位号还留着，面板换了备份：读的是面板里这个备份的数据库，不拿上一个的来补 |
  | 认出面板要的那一次打开（清单没有列） | A10、A11、A13、A15 | A11：一个文件正在读，这时面板要图；先开始的那个读完时不补、要求还留着，面板要的那张读完时补对了。A13：面板要了图但打开没有开始；同一个文件夹换成「没有数据库的备份」再进面板，双击同一个文件：不补，要求清掉。A15：预告的号就是下一次打开用的号，问几次都是它，问不算一次打开；计数回绕时跳过 0 |
  | 没有数据库文件的备份（清单没有列） | A12 | 图纸照常打开，保持图纸文件里的名字，不读数据库，命令行不说话 |
  | 补完之后属性面板和检查栏要换（清单「做什么」里的一句） | A2、A16、A17、A18 | A2：选中第三个阀门，属性面板的位号三行是 `PIA-0112101B`、`Drawing file`、没有第三行；位号到了之后是 `PIA-old_0112101A`、`Backup database`、`PIA-0112101B`，测试没有另外去刷新；重读名单是空的。A16：位号到的时候这张图在别的图纸后面：记录补对了，进了重读名单；切回来，三行是数据库的，名单空了。A17：在后面补完就关掉：从名单里去掉，前面那张不动。A18：没读成的不进名单 |

- 真实备份上的读数（23:43，用的是门禁编出来的那个测试程序，没有重新编译；`real-run-a.txt`。R1、R2 平时忽略，要带 `--ignored` 跑；备份不在时说一句就过）：
  - R1（验收第三条）：`T_PlantItem.ItemTag` 这一列存着 8,145 个单元格，全是文字，没有空白的，没有两头带空白的；取出 8,145 个位号。测试钉的是：数据库读全了；位号是 8,145 个，等于文字单元格数减去空白的；表里的对象编号都是折过的写法，位号两头没有空白；再问一次给的是同一张表，没有重算。读数据库文件约 2 秒，从读好的数据库备出这张表 16.8 毫秒。
  - R2（验收第一条，规格 §6 第 2 条）：样例直接打开，592 个图例都有 `item`，231 个有 `label`，`tag_source` 全是 `file`，没有 `label_in_file`。同一张图从备份面板打开（数据库这时才读，读了一次；图纸先显示，位号后到）：592 个图例，354 个有 `label`，`tag_source` 全是 `database`，没有 `label_in_file`；命令行一句 `P&ID item tags: 354 symbol(s) of <文件名> carry the tag the backup database gives their item; 0 differ from the name in the drawing file`，是提示不是警告；已修改标记没有，撤销记录是空的，命令行的错误计数没有变。从双击到补完 5.19 秒，含读数据库和导入图纸。
  - R2 另把读到的位号表直接补到直接打开的那一份上：三个数是 354、0、354，补完和从面板打开的那一份数出来相同；再补一遍是 354、0、0。354 个图例补一遍 8.07 毫秒。
  - R2 最后从面板再开这个备份的另一张图（D07；位号已经读好，显示之前补）：没有再读数据库，命令行一句提示。这张图的数只打印，没有断言：18 个图例，9 个有 `label`，其中 `database` 8 个、`file` 1 个，1 个带 `label_in_file`；命令行说的是 8 个、1 个不同。
  - 这些耗时都是在不空闲的机器上量的一次（整机占用 46 %–77 %），只能看个大概。
- 先红：
  - 先写的是测试和空壳：新函数都在，什么都不做（`fill_database_tags` 给三个 0，`clear_pid_key` 不清，`tags_of_cells` 给空表，`Dump::item_tags` 不管读没读全都给空表，`on_tags_thread` 就地执行，`open_from_the_backup` 只是照常打开，`tag_the_drawing_the_backup_opened` 和 `backup_tags_read` 是空的，`open_id_to_come` 给 0）。带空壳的 11 个文件留在 `red\`。
  - 跑当时的 16 条（22:26–22:35，`libmut-red.txt`）：红 14，绿 2，另有 1 条要真实备份的被忽略。
  - 红的 14 条都红在和功能有关的断言上：F2、F3、F4、F5 红在三个数上（都是 0），F1 红在「写了两次的都清掉」，D1 红在位号表是空的，D2 红在给了空表而不是原因，D3 红在线程名上，A15 红在预告的号上，A8 红在「正在读」没有记上，A1 红在图纸没有补对。A2、A3 和当时的「图纸不在了」一条红在共用的准备步骤上（图纸没有进等待名单），没有走到各自的断言。
  - 绿的 2 条是 A10、A12：它们钉的是「不补」「不读」，空壳也不补不读。A10 在改坏表的第 37、38、40、41、42、73、75 处红，A12 在第 28 处红。
  - 去掉空壳之后第一遍（22:40–22:43，`libmut-green.txt`；这时「图纸不在了」那一条已经拆成 A6、A7，共 17 条）：过 13，红 4。红的 4 条是测试自己的错，不是实现的：A1、A2、A8 用的读命令行的辅助函数按「行首是这句话」找，而命令行在每行前面加了种类的记号，它又按行数接着往下读，而命令行只留最后若干行，改成先清空命令行、再在行里找这句话；A3 用自动化的 `new` 新开图纸，没有把新图纸放到前面，改成发 `Message::TabNew`。
  - 那之后改过的，都没有在空壳上跑过：
    - 实现加了两处：面板要图之前就开始的打开读完时不拿走要求（配 A11）；面板换了备份、但还有一张要的图没读完时位号留着（配 A5）。
    - 加了六条测试：A4、A5、A9、A11、A13、A14。A7 加了「关掉之后同一个文件又直接打开」，A15 加了「号各不相同」，D3 加了按线程计数的那一段（产品代码里的 3 行计数只在测试里编译），F1 的几步换了先后。
    - 第一遍收尾门禁跑着的时候查出一处没做到的：图纸在别的图纸后面补了位号，切回来时属性面板还是旧的（`refresh_properties` 只读当前的图纸，切换标签页也不重读）。加了重读名单和每条消息之后的那一步（`to_show`、`show_the_tags_taken`、`update` 里的 4 行），加了 A16、A17、A18 三条，A2 加了一条断言。改坏和收尾门禁都在这之后整轮重跑。
    - 这九条（A4、A5、A9、A11、A13、A14、A16、A17、A18）没有在空壳上红过，只在改坏下红过。
- 故意改坏 78 处。开关加在七个文件里（`metadata.rs`、`database_tags.rs`、`exp_views.rs`、`tags.rs`、`update/file.rs`、`pidbackup.rs`、`update/mod.rs`），用环境变量打开；每处跑这 26 条（R1、R2 要真实备份，不在里面）。不改坏时 26 条全过；78 处每一处都有测试变红，26 条每条至少红过一次（23:14–23:17，`libmut-mut2.txt`）：

  | # | 改坏 | 改在哪 | 变红的测试 |
  |---|---|---|---|
  | 1 | 清键时连别的应用的记录也清 | `clear_pid_key` | F1 |
  | 2 | 图元没有记录时清键反而建一条空记录 | `clear_pid_key` | F1 |
  | 3 | 键名以要清的字开头就清 | `clear_pid_key` | F1 |
  | 4 | 要清的字以键名开头就清 | `clear_pid_key` | A1 A2 A3 A4 A5 A11 A14 A16 F1 F2 F3 F5 |
  | 5 | 同一个键写了两次时只清第一条 | `clear_pid_key` | F1 |
  | 6 | 记录里不是字符串的值也被清掉 | `clear_pid_key` | F1 |
  | 7 | 不是块引用的图元也补位号 | `fill_database_tags` | F4 |
  | 8 | `role` 不是 `symbol` 的块引用也补 | `fill_database_tags` | F4 |
  | 9 | 调用方说「本程序里画的」也照补 | `fill_database_tags` | A14 F4 |
  | 10 | 对象编号不按数据库的写法折叠就去查表 | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 F3 F4 F5 |
  | 11 | 位号不去掉两头的空白 | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 |
  | 12 | 全是空白的位号也当位号 | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 F3 |
  | 13 | 第二次补时不认上次留下的 `label_in_file` | `fill_database_tags` | F3 |
  | 14 | `_Data.xml` 给的名字也当成图纸文件里的名字留下 | `fill_database_tags` | F5 |
  | 15 | 文件里的名字和位号相同也写 `label_in_file` | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 F3 |
  | 16 | 文件里的名字不同也不写 `label_in_file` | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 F3 F4 F5 |
  | 17 | 记录已经全对也重写（第二遍不是 0 改动） | `fill_database_tags` | F3 |
  | 18 | 补了位号不标 `tag_source=database` | `fill_database_tags` | A1 A2 A3 A4 A5 A11 A14 A16 F2 F3 F5 |
  | 19 | 位号变得和文件里的名字相同后不清 `label_in_file` | `fill_database_tags` | F3 |
  | 20 | 读库不交给工作线程，在发起的线程上读 | `item_tags_in_background` | D3 |
  | 21 | 工作线程函数就地执行，不起线程 | `on_tags_thread` | D3 |
  | 22 | 工作线程不叫 `ocs-pid-backup-tags` | `on_tags_thread` | D3 |
  | 23 | 工作线程中途没了时给空的原因 | `on_tags_thread` | D3 |
  | 24 | 全是空白的单元格也算位号 | `tags_of_cells` | D1 |
  | 25 | 单元格文字不去掉两头的空白 | `tags_of_cells` | D1 |
  | 26 | 不是文字的单元格也算位号 | `tags_of_cells` | D1 |
  | 27 | 数据库读不全时给一张空表，不说原因 | `Dump::item_tags` | D2 |
  | 28 | 没有可读数据库的备份也去读 | `open_from_the_backup` | A12 A13 |
  | 29 | 没有可读数据库的备份打开图纸时不作废上一次的要求 | `open_from_the_backup` | A13 |
  | 30 | 面板打开图纸时不记「要补位号」 | `open_from_the_backup` | A1 A2 A3 A4 A5 A6 A7 A8 A9 A11 A13 A14 A16 A17 A18 |
  | 31 | 位号已经读好还再读一遍 | `open_from_the_backup` | A1 A8 A11 |
  | 32 | 读失败过的数据库，下一张图又去读 | `open_from_the_backup` | A8 |
  | 33 | 别的备份读好的位号也算「已读好」，不读自己的 | `open_from_the_backup` | A9 |
  | 34 | 正在读的时候再起一次读 | `open_from_the_backup` | A2 A3 A4 A6 A7 A16 A17 A18 |
  | 35 | 起了读不记「正在读」 | `open_from_the_backup` | A5 A8 A9 |
  | 36 | 双击的当时就在界面线程上读完 | `open_from_the_backup` | A8 |
  | 37 | 任何打开都当成面板要的（直接打开也补） | `tag_the_drawing_the_backup_opened` | A7 A10 |
  | 38 | 要求之后别的打开完成时不作废这次要求 | `tag_the_drawing_the_backup_opened` | A10 |
  | 39 | 要求之前就开始的打开把要求拿走 | `tag_the_drawing_the_backup_opened` | A11 |
  | 40 | 要求用过不清 | `tag_the_drawing_the_backup_opened` | A1 A2 A3 A5 A10 A11 |
  | 41 | 不核对打开号，路径对就补 | `tag_the_drawing_the_backup_opened` | A10 |
  | 42 | 不核对路径，打开号对就补 | `tag_the_drawing_the_backup_opened` | A10 |
  | 43 | 打开时拿别的备份的位号来补 | `tag_the_drawing_the_backup_opened` | A9 |
  | 44 | 位号还在读时图纸不登记等待 | `tag_the_drawing_the_backup_opened` | A2 A3 A4 A6 A7 A8 A9 A16 A17 A18 |
  | 45 | 别的备份的位号到了也把「正在读」清掉 | `backup_tags_read` | A4 |
  | 46 | 读完了不清「正在读」 | `backup_tags_read` | A2 A3 A4 A5 A6 A7 A8 |
  | 47 | 别的备份的位号到了也补给等着的图纸 | `backup_tags_read` | A4 |
  | 48 | 位号到了不补给等着的图纸 | `backup_tags_read` | A2 A3 A4 A8 A9 A16 A17 A18 |
  | 49 | 找等着的图纸只看路径、不看是哪一页 | `backup_tags_read` | A7 |
  | 50 | 找等着的图纸只看是哪一页、不看路径 | `backup_tags_read` | A6 |
  | 51 | 位号到了总补给当前页 | `backup_tags_read` | A3 A6 A7 A16 |
  | 52 | 图纸已关或换了文件时还留在等待名单里 | `backup_tags_read` | A6 A7 |
  | 53 | 图纸已关或换了文件时还往命令行写一句 | `backup_tags_read` | A6 A7 |
  | 54 | 迟到补完不登记「属性面板要重读」 | `backup_tags_read` | A2 A16 A17 |
  | 55 | 数据库没读成也登记「属性面板要重读」 | `backup_tags_read` | A18 |
  | 56 | 图纸不在前面也算重读过（清掉名单，读的是当前页） | `show_the_tags_taken` | A16 A17 |
  | 57 | 已经关掉的图纸还留在名单里 | `show_the_tags_taken` | A17 |
  | 58 | 重读之后不从名单里去掉 | `show_the_tags_taken` | A2 A16 |
  | 59 | 轮到它在前面时只去掉名单、不重读属性面板 | `show_the_tags_taken` | A2 A16 |
  | 60 | 面板已换备份时，不为还没读完文件的那张图留着位号 | `backup_tags_read` | A5 |
  | 61 | 面板已换备份、也没有图在等，还留着位号 | `backup_tags_read` | A3 A4 |
  | 62 | 读到的位号不留给后面的图纸 | `backup_tags_read` | A2 A5 A6 A7 A8 A9 |
  | 63 | 命令行那一句里写整个路径，不是文件名 | `put_the_backup_tags_on` | A1 A2 A3 A4 A5 A8 A9 A11 A14 A18 |
  | 64 | 那一句里两个个数对调 | `put_the_backup_tags_on` | A1 A2 A3 A4 A5 A11 A14 |
  | 65 | 补了位号不写那一句 | `put_the_backup_tags_on` | A1 A2 A3 A4 A5 A11 A14 |
  | 66 | 数据库没读成按错误报，不是警告 | `put_the_backup_tags_on` | A8 A9 A18 |
  | 67 | 数据库没读成什么也不说 | `put_the_backup_tags_on` | A8 A9 A18 |
  | 68 | 补位号时不把图纸从页里取出来，就地改 | `fill_with_database_tags` | A2 |
  | 69 | 不问「是不是本程序里画的」 | `fill_with_database_tags` | A14 |
  | 70 | 补完把图纸标成已修改 | `fill_with_database_tags` | A2 A3 A17 |
  | 71 | 补完把图纸的修改序号加一 | `fill_with_database_tags` | A2 |
  | 72 | 真正打开时的打开号比预告的多跳一个 | `next_open_id` | A1 A2 A3 A4 A5 A6 A7 A8 A9 A11 A14 A15 A16 A17 A18 |
  | 73 | 预告的打开号是当前这个，不是下一个 | `open_id_to_come` | A10 A11 A15 |
  | 74 | 打开号回绕时不跳过 0 | `open_id_to_come` | A15 |
  | 75 | 图纸读完时不调补位号的那一步 | `on_file_opened` | A1 A2 A3 A4 A5 A6 A7 A8 A9 A10 A11 A14 A16 A17 A18 |
  | 76 | 位号读完的消息没人接 | `on_pid_backup` | A2 A3 A4 A5 A6 A7 A8 A9 A16 A17 A18 |
  | 77 | 面板打开图纸走普通打开，不走补位号的入口 | `on_pid_backup` | A1 A2 A3 A4 A5 A6 A7 A8 A9 A11 A13 A14 A16 A17 A18 |
  | 78 | 每条消息处理完不问「有没有图纸的属性面板要重读」 | `update` | A2 A16 A17 |

  这是第二轮，跑在最终的代码上。第一轮（22:57–23:00，`libmut-run1.txt`）跑在加重读名单之前的代码上：72 处、23 条，72 处都红。第二轮去掉了其中 1 处（「补完当时不刷新属性面板」，那一行代码已经换掉），加了 7 处（第 54–59、78 处）。

  改坏点已删：七个文件按副本还原，连同 `src/io/pid/tests.rs` 共八个文件的 sha256 与加开关之前相同（`sha-before-mut2.txt`）；`src`、`tests` 里搜不到开关。固定门禁是还原之后重新编译跑的。
- 没有红过的：
  - 新加的 26 条每条都至少红过一次，不是每一条断言都红过。26 条和它们共用的准备步骤里，会让测试停下来的地方一共 202 处（断言，加上 `expect`、`unwrap` 这类准备用的），78 处改坏下停过 95 处，没有停过 107 处，逐处列在 `mut2-panic-lines.txt`。归起来是下面几类。
  - 「不算修改」里没有红过的：
    - A2 的「那一步结束后撤销记录是空的」。就地改（第 68 处）时测试先停在修改序号上，走不到这一条。门禁之后补了一次试验（不进提交）：在带开关的文件里把 A2 的这两条断言换了先后，先看撤销记录、再看修改序号，只跑 A2 一条（23:44–23:46，`libmut-exp.txt`）。不改坏时过；就地改（第 68 处）红在「撤销记录是空的」上；只动修改序号（第 71 处）红在修改序号上；只标已修改（第 70 处）红在已修改标记上。所以这一条断言是管用的，只是按提交的先后被修改序号那一条挡在后面。提交的测试没有换先后。试验完八个文件按副本还原，sha256 与门禁时相同，`src`、`tests` 里搜不到开关；门禁没有重跑。
    - A1 的「已修改标记没有」「撤销记录是空的」。显示之前补的那条路上，补完之后打开流程自己把这两样重置（`on_file_opened` 里，在补位号那一步之后），第 70 处在这条路上不红。这两条防的是有人把那一步挪到重置之后。A5、A8 的已修改标记同样没有红过。
    - 「命令行的错误计数没有变」四处（A1、A2、A8 两处）。把警告改成按错误报（第 66 处）时，测试先停在那一句的种类上。
  - 被前一条断言挡住的：
    - F4 只停过三个数那一条，后面逐样比记录的五条没有走到。
    - F2 的「文件里没名字的那个阀门的整条记录」「数据库没有位号的三个不变」「别的图元不变」没有走到，前面的三个数或者第一个阀门先红。
    - F3 的「第二遍记录逐字相同」「换表之后另外五个不变」「换表之后的三个数」没有走到。
    - A7 的「直接打开的那张保持图纸文件里的名字」没有走到：只看路径找图纸（第 49 处）时，测试先停在「命令行不说话」上。
    - A10 的 18 处里停过 4 处（三次「保持图纸文件里的名字」和一次「要求已经作废」）；三处「命令行不说话」排在后面，没有走到。
  - 钉「不读」「不说」的：A12 的 9 处里只停过 1 处（没有读数据库，第 28 处）；A4、A5、A13 里「命令行不说话」的三处没有红过。
  - 对照用的没有红过：D3 的「同一个线程自己去读一次，计数加一」（说明计数管用）；A2、A16 里补之前属性面板的三行；各条里「新图纸在前面」「备份已经换了」「打开结束」这一类准备步骤的断言。
  - 没有加开关的地方（这些地方改坏了，现有的测试不一定红）：
    - `clear_pid_key` 在记录里没有这个键时不重写记录：改成照样重写，结果一样，测试分不出来。
    - `Dump::item_tags` 成功时的结果存着不重算：只有要真实备份的 R1 在比。失败的那一路 D2 比了，但没有红过。
    - 列名 `T_PlantItem.ItemTag`：只有 R1 在钉。
    - 「数据库读全了，但里面没有这一列」那一支：读入器的表拼不出来，没有测试。
    - 位号留不留只认「同一个备份要的图」：改成「只要有一张要的图就留」，现有的测试分不出来。
    - 两处接线只测了「调没调」（第 75、78 处），没有测它们在各自函数里的先后。
- 耗时预算（`io::pid::sample_budget`，测试没有改，没有放宽）：最终的代码上量了四次，超了三次，过了一次，机器一直不空闲。**没有验证。**
  - 门禁那一遍（23:17–23:20，重新编译之后跑的）：中位数 3,821 毫秒（三次是 3,523、3,821、4,026），预算 3,747，红。这一遍没有记整机占用；十分钟后量到的是 92 %，其中别人的八个 rustc 占 47 %。
  - 同一个测试程序不重新编译又跑三轮（23:37–23:43，每轮先等一分半钟看机器会不会空下来）：3,893（红）、3,816（红）、3,671（过）。三轮前后整机占用在 48 % 到 82 % 之间，别的编译进程 1 到 4 个（不是这项工作启动的，没有动），一轮也不算空闲。
  - 作废的第一遍门禁（23:00，加重读名单之前的代码）量到 3,564，过；那一遍也没有记占用。
  - SI-3、SI-4、SI-5 各自在提交前量的三轮（共九轮，也都不空闲，整机占用 43 % 以上）中位数在 3,365 到 5,313 之间，过了六轮；这次的四个数落在这个范围里。从这些数看不出导入变慢了，也证明不了没有变慢。这次的产品代码没有动导入这条路：`metadata.rs` 加的 `clear_pid_key` 导入时不调用。
  - 「空闲的机器上中位数不超过 3,747」留给 SI-7 重量；到时候还超，从这个提交查起。
- 读数（固定门禁在最终的代码上跑了一遍，23:17–23:37，`chainfinal2.txt`）：
  - 格式：13 个 `.rs` 文件都是 `new=0`。两个新文件整个合格；另外 11 个里原有的不合格处一处没有多。
  - 快速筛选：`app::commands::pidbackup::` 60 过、2 忽略；`io::pid::` 99 过、1 忽略；`io::pid_backup_browse::` 76 过、3 忽略；`ui::window::pid_backup_browser` 139 过。
  - 全量单元测试 7,226 个：通过 7,170，忽略 56，失败 0（上一个提交的 7,198 加新加的 28；忽略的是原来的 54 加 R1、R2）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同；测试文件编译告警 0；跑完后系统临时目录里名字带 `symbol` 的文件夹 0 个。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录和上一个提交的按文件加告警内容逐条比过：没有多出新的告警，也没有少；有 4 条原有的「这个函数可能在栈上分配 N 字节」换了数。`src/locale_catalog.rs` 一条，两种配置里都有，多 339 字节（词条表多了三条）；测试配置里另有 `src/app/collab/collab_large_props.rs` 两条、`src/app/collab/collab_offline_queue.rs` 一条，各多 1,440 字节（这三个测试函数里放着整个程序，程序多了一个字段）。
  - 分层报告与上一个提交逐行相同。
  - 耗时预算不在十条固定门禁里，它是规格 §6 第 12 条，见上。
- 与清单不一样的地方：
  - 清单的「文件」写的是 `src/io/pid_backup_browse/exp_views.rs`、`src/app/commands/pidbackup.rs`，「另加办法定下来之后要动的文件」。办法定下来之后另外动了 10 个 `.rs` 文件（两个是新的）、词条表和 23 个语言文件，逐个列在规格 §11 的 SI-6 第 17 条。
  - 清单写「补完要调一次 `refresh_properties`」。做成了：显示之后才补的图纸记进一张名单，程序每处理完一条消息看一次，它在前面时调一次。直接调的话，图纸在别的图纸后面补位号时调了也白调（规格 §11 的 SI-6 第 9 条）。
  - 验收是三条；另外给「已经读好时显示出来就是数据库的」「补几遍一样」「只补图纸文件的图例」「`_Data.xml` 给的位号」「位号给对图纸」「认出面板要的那一次打开」「没有数据库的备份」「属性面板要换」加了测试。
  - 验收下面加了一行不带勾的：带窗口的那一遍和耗时预算留给 SI-7。SI-7 的「做什么」加了一句。
  - 命令行多了两句话，词条表多了三条（规格 §11 的 SI-6 第 10 条）。
  - 先红之后加了九条测试和两处实现，没有在空壳上红过；改坏跑了两轮，收尾门禁跑了两遍，记的都是第二遍（见「先红」「故意改坏」「读数」）。门禁之后另补了一次不进提交的试验（见「没有红过的」）。
  - 规模：清单写 M。产品代码 534 行、12 个文件，测试 1,895 行。
- 没验证的：
  - 没有在带窗口的程序里从备份面板打开图纸（规格 §6 第 2、8、13 条在窗口里的那一遍，留给 SI-7）。A 开头的测试不开窗口，图纸和位号表是直接交给程序的；R2 读了真实的图纸和数据库，也不开窗口。面板上双击、图纸先出来位号后到的那一下、命令行那一句、属性面板的三行、不改就关时不问保存，在窗口里都没有看。
  - 规格 §6 第 13 条只在不开窗口的测试里验了一半：已修改标记没有、修改序号没变、撤销记录是空的、关标签页没有被拦下来（A2、A17）。带窗口点关闭的那一遍留给 SI-7。
  - 规格 §6 第 8 条的前一半「从备份面板打开后，用一个阀门的位号找」没有测：`PIDTAG` 读的是记录里的 `label`，现在数据库的位号写进去了，但没有在补过位号的图上跑过 `PIDTAG`。留给 SI-7。
  - R2 在不开窗口的测试里走的是「图纸先显示、位号后到」那条路。真实数据上「位号已经读好、显示之前补」那条路只有 R2 最后打开的第二张图在走，只比了命令行有一句、是提示不是警告，没有比它的个数。
  - 样例之外只从面板开过一张（R2 顺带开的 D07，数只打印、没有断言），另外 51 张没有开过（留给 SI-7）。数据库的位号和图纸文件里的名字不同的图例，样例上一个也没有（差异是 0）；真实数据上只见过 D07 的 1 个，有断言的只有拼出来的图。
  - 真实数据上没有「读不全」的数据库：读不全、读不了都是拼出来的（D2、A8、A18）。「读全了但没有这一列」没有测试。
  - 图纸显示之后、位号到之前用户改过的图例：撤销那一步时记录回到补之前，不会再补一次（规格 §11 的 SI-6 第 7 条）。没有测试，也没有试过。
  - 双击之后、面板自己的打开开始之前插进来另一次打开；双击没有开始读文件、紧接着直接打开同一个文件（规格 §11 的 SI-6 第 13 条的两个边角）。没有测试。
  - 面板换了备份之后，上一个备份里还没读完文件的图纸不再等位号（规格 §11 的 SI-6 第 14 条）。A9 测的是换了之后读新备份的，没有测上一个备份的图纸这时是什么样。
  - 数据库读进来之后占的内存、第一次从面板打开时多出来的读库时间对界面的影响，没有量。R1、R2 打出来的耗时是在不空闲的机器上量的一次。
  - 协作、自动化和控制通道：经它们打开备份里的图纸走不走面板的那条路，没有查，也没有测。
  - 新加的两句话和一条原因在 22 种语言里是英文原文，没有译。简体中文的译文没有请人看过。
  - 改坏只跑了这 26 条；全量单元测试里别的测试、三个集成测试目标没有在改坏下跑。
  - 出图没有重出（导入器没有动）。
- 证据：`D:\scratch\ocs-symblock\si-6\`（先红：`libmut-red.txt`、`libmut-red-none.log`、带空壳的文件 `red\`；去掉空壳之后：`libmut-green.txt`、`libmut-green-none.log`；改坏：`libmut-mut2.txt`、`libmut-mut2-*.log`、`mutant-table-mut2.md`、`mut2-panic-lines.txt`、加开关之前的八个 sha256 `sha-before-mut2.txt`、七个文件的副本 `pristine\`、带开关的七个文件 `switched\`，第一轮 `libmut-run1.txt`、`mutant-table-run1.md`；固定门禁：`final-runfinal2.txt`、`chainfinal2.txt` 和它们列的各份记录，作废的第一遍 `final-runfinal.txt`、`chainfinal.txt`；clippy 记录和上一个提交的比较：`clippy-diff-lib-b.txt`、`clippy-diff-lib-test-b.txt`；耗时预算的轮次：`budget-rounds-a.txt`、`budget-rounds-a-1.log` 到 `budget-rounds-a-3.log`，门禁那一遍 `budgetfinal2.log`，十分钟后的机器占用 `machine-load-final2.txt`；真实备份上的两条：`real-run-a.txt`、`real-a-r1.log`、`real-a-r2.log`；门禁之后的那次试验：`libmut-exp.txt`、`libmut-exp-*.log`；改动行数：`split-lines-b.txt`，它按「测试模块从哪一行起」分，把 `src/io/pid/mod.rs` 里只在测试里编译的 +7 −3 算成了产品，合计写的是产品 +541 −7、测试 +1,888；上面「改了什么」的数把这几行算回了测试）。新加的脚本都在 `si-6\`：`make-mutants.py`、`make-mutants-run1.py`（给七个文件加开关，第二轮和第一轮）、`mutant-table.py`（把改坏的结果排成上面那张表）、`panic-lines.py`（各处改坏停在最终文件的哪一行，哪些断言从没停过）、`fix-tags-1.py`、`fix-tags-2.py`、`fix-tags-3.py`（先红之后的三次改动）、`split-lines.py`（改动行数分成产品和测试）、`make-ftl-plan.py`（23 个语言文件加哪几行）、`real-run.ps1`（跑真实备份上的两条）、`swap-a2.py`（试验时把 A2 的两条断言换先后）、`apply-record.py`（把这份记录写进清单和规格）。写仓库的只有 `fix-tags-*.py`、`apply-record.py`，和改坏时用的 `make-mutants.py`、`swap-a2.py`（这两个写进去的都已按副本还原）；别的只读。

### SI-7 53 张核对，带窗口的验收，需求文档 【验收】【文档】

- **做什么**：53 张图逐张导入，对规格 §6 第 3、4 条的合计数（第 4 条的配对数按文字图元量，和规格按记录量的数差在哪逐项说明；顺带查 53 张里有没有一个文件带几个有图形的图面流。这两件是 SI-2 留下的）；在带窗口的程序里做第 7、8、11、13 条（做第 11 条时，另存成 DWG、DXF 再打开之后属性面板上规格 §4.5 的行也要看，这是 SI-3 留下的），并重出样例的图，和图块分支验收时的出图逐字节比（第 9 条的后半句，SI-1 留下的）；在空闲的机器上重量样例的耗时预算（第 12 条，SI-2、SI-3 留下的：没有编译在跑、整机占用不超过五分之一时跑预算测试，仍然超过就查）；在 53 张里图元最多的一张上量「找一个图例的标注」「找一条标注所属的图例」的耗时，超过 16 毫秒就改成导入后建一次索引（规格 §4.6，SI-3 留下的：只量了样例一张）；同一张图上量 `PIDTAG` 按位号找一次、按点中的一条标注找一次的耗时（SI-5 留下的：只量了样例一张上的合计和平均，没有量最慢的一次和整条命令）；带窗口的那几条里，样例要从备份面板打开：先显示、后补位号的第一张和显示之前就补的第二张，各看一遍第 2 条和第 13 条（命令行那一句、属性面板的位号三行、不改就关时不问保存），在补过位号的图上用阀门的位号跑 `PIDTAG`（第 8 条的前一半），再看图纸在别的图纸后面补了位号、切回来之后属性面板换没换（SI-6 留下的：这些只有不开窗口的测试；耗时预算在 SI-6 的提交上量了四次、超了三次，机器都不空闲，空闲时重量仍然超过的话从那个提交查起）；`REQ-MIG-008` 加规格 §10 的两条验收标准。
- **验收**：
  - [ ] 规格 §6 的 14 条逐条有读数或说明，写成一张表。
  - [ ] 没验证的逐条写明。
- **依赖**：SI-1 到 SI-6。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`、本清单、规格。
- **规模**：M。
