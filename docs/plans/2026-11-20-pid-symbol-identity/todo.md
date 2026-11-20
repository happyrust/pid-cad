# 任务清单：导入的图例带上类型和位号，标注文字和图例配对

> 日期：2026-11-20。状态：**执行中**。规格已确认（2026-11-20，Q1–Q5 都按推荐）；SI-1、SI-2、SI-3 做完，其余没有开始。
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

- **做什么**：53 张图逐张导入，对规格 §6 第 3、4 条的合计数（第 4 条的配对数按文字图元量，和规格按记录量的数差在哪逐项说明；顺带查 53 张里有没有一个文件带几个有图形的图面流。这两件是 SI-2 留下的）；在带窗口的程序里做第 7、8、11、13 条（做第 11 条时，另存成 DWG、DXF 再打开之后属性面板上规格 §4.5 的行也要看，这是 SI-3 留下的），并重出样例的图，和图块分支验收时的出图逐字节比（第 9 条的后半句，SI-1 留下的）；在空闲的机器上重量样例的耗时预算（第 12 条，SI-2、SI-3 留下的：没有编译在跑、整机占用不超过五分之一时跑预算测试，仍然超过就查）；在 53 张里图元最多的一张上量「找一个图例的标注」「找一条标注所属的图例」的耗时，超过 16 毫秒就改成导入后建一次索引（规格 §4.6，SI-3 留下的：只量了样例一张）；`REQ-MIG-008` 加规格 §10 的两条验收标准。
- **验收**：
  - [ ] 规格 §6 的 14 条逐条有读数或说明，写成一张表。
  - [ ] 没验证的逐条写明。
- **依赖**：SI-1 到 SI-6。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`、本清单、规格。
- **规模**：M。
