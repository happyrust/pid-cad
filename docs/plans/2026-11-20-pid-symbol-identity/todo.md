# 任务清单：导入的图例带上类型和位号，标注文字和图例配对

> 日期：2026-11-20。状态：**SI-1 到 SI-11 做完**（2026-11-21）。规格已确认（2026-11-20，Q1–Q5 都按推荐）。规格 §6 的 14 条里，第 12 条（耗时预算）一直没有在空闲的机器上验证；2026-11-21 机器空下来的时候量了，过了，是 SI-11；同一天机器再空下来时补量了 10 轮空闲的，十轮补齐了，「比图块分支慢 2 %–3 %」的说法也跟着改了（SI-11 记录后面的「补量」）。同一天把这份计划改过的代码通读了一遍（只读，代码没有改），补记了一个没有写出来的前提（最后的「通读」一段）。SI-7 留下等拍板的四处，用户在 2026-11-21 定了：第 3 条里 D03 的说法更正、第 5 条的 53 张合计数写进测试，是 SI-8；「视图一动就算改过」是 SI-9，「带空格的位号」是 SI-10。同一天在另开的本地分支 `pid-symbol-on-main` 上把 `main` 合进来走了一遍，记在最后的「合进 main 的演练」一段（这一句和那一段只在那条分支上）。演练留下的三件事用户同一天定了：缺口符号的数按新的改了，记在它后面的「演练之后」一段（也只在那条分支上）。那一段里说要跑的另外 4 个外部测试程序随后跑了，记在再后面的「演练之后：另外 4 个外部测试程序」一段（同样只在那条分支上）。
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
  - `src/io/pid/items.rs`（新文件，产品部分 134 行）：`SheetItems::read` 调一次转换库的 `load_publish_from_pid`，建「图形编号 → 对象」的表。每个对象留四样：编号、种类、发布类、名字。名字取对象 `IObject` 接口的 `Name` 属性，去掉首尾空白，空的算没有。表示记录没写对象、或写的对象不在这张图的对象里，这个图形不进表；一个图形有几条表示记录时取第一条。读不出来（图纸不写图号、文件打不开）不算导入失败：写一行日志说原因，交回空表。（这张表只按图形编号查，不分图面流：2026-11-21 通读时补记，见最后的「通读」一段和规格 §11 的 SI-1 第 9 条。）
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
- 耗时（验收第 2 条；S5 打在标准错误输出上的，`lookup-timing.txt`）：样例的模型空间 6,390 个图元，测试程序，跑了三遍，量的时候机器上有别的编译在跑。（2026-11-21 更正：这里原来写的是「优化过的测试程序」。测试程序里本程序自己的代码不开优化，只有依赖库开，见 SI-7 的记录。）
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
  - 耗时（测试程序，同一个程序连跑三遍，`sample-timing-a.txt`；三遍前后整机占用 41 %–56 %，没有别的编译在跑）：一次全选 6.5、8.3、9.7 毫秒；229 个位号逐个问合计 1.35、1.22、1.51 秒，平均每次 5.3–6.6 毫秒；958 条标注逐条点合计 5.72、5.68、7.01 秒，平均每次 5.9–7.3 毫秒（含测试自己用 `label_owner` 对答案的时间）；图例识别一遍 10.1、9.6、17.1 毫秒。只量了合计，没有量最慢的一次。（2026-11-21 更正：这里原来写的是「优化过的测试程序」。测试程序里本程序自己的代码不开优化，只有依赖库开，见 SI-7 的记录。）
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
  - [x] 规格 §6 的 14 条逐条有读数或说明，写成一张表。
  - [x] 没验证的逐条写明。
- **依赖**：SI-1 到 SI-6。
- **文件**：`docs/pid-platform/requirements/90-migration-and-interop.md`、本清单、规格。
- **规模**：M。

**做完**（2026-11-21）

- 结论：规格 §6 的 14 条里，10 条符合，其中第 8、13 条各带要说明的地方；第 3 条数都相符，但「D03 导入成功」不成立（D03 被拒绝打开）；第 12 条（耗时预算）**没有验证**；第 5 条说明了差在哪，但「写进测试」只做了样例；第 14 条是按「只动文档的提交」的先例过的。代码一行没有动。量出来三处和规格不一样、或者规格没有写到的地方，都记在规格 §11 的 SI-7 一节，等拍板：D03 打不开；视图一动图纸就算改过，所以缩放或者 `PIDTAG` 找到东西之后，关闭时它是一张没保存的图纸；带空格的位号用 `PIDTAG <位号>` 找不到。
- 改了什么（只有三份文档）：
  - `docs/pid-platform/requirements/90-migration-and-interop.md`：`REQ-MIG-008` 的验收标准加了第 7、8 两条，是规格 §10 的原文。
  - 本清单：这段记录、开头的状态行；SI-3、SI-5 的记录里「优化过的测试程序」各更正一处（原因见下面「与清单不一样的地方」）。
  - 规格：开头的状态行；§11 加 SI-7 一节；SI-3 第 9 条、SI-5 第 10 条里同一个说法各更正一处。
- 怎么量的：
  - 量的是上一个提交（SI-6，`be03c18e9`）的代码：在这个工作树上编的发布版（206,541,312 字节，sha256 开头 `704e0e7d74edf2a7`）和同一份代码的单元测试程序（306,436,608 字节，sha256 开头 `4dcbf5540fbd6717`）。编之前 `git status` 是干净的。发布版自报的版本带 `dirty`（`2026.46+531.gbe03c18e.dirty`）：编之前把 `src/lib.rs` 的修改时间改成了当时（内容没动，为的是让共用的编译目录从这个工作树重编），`build.rs` 用 `git diff-index` 判断，时间变了就算改过。
  - 两种开法。不开窗口：程序带 `--serve` 启动，一行一个请求。带窗口：真的窗口，开在另建的一个看不见的桌面上，经本机回环上的控制通道操作；读的是程序自己的状态、图元的记录、属性面板要显示的行、命令行说过的话，截图由程序自己截。用户桌面上的鼠标、键盘、前台窗口没有动，每趟跑完都核对了。
  - 带窗口时没有用真的鼠标和键盘，下面几步是替代的：从备份面板开图用面板自己的命令（`PIDBACKUPOPEN Select …` 选中一行，`PIDBACKUPOPEN Press` 按「打开」，按下去发的是双击发的那条消息）；选中图元用控制通道的选中请求；另存用控制通道的保存请求，没有走「另存为」对话框；关窗口是给窗口发关闭消息。
  - 数据：`D:\work\plant-code\cad\pid-test-data`，只读着用，每趟带窗口的运行前后各清点一遍：2,170 个文件、836,533,147 字节，没有变。样例是「L M井场 注采阀组工艺及自控流程图.pid」；53 张里图元最多的是「EF井场 注采阀组工艺及自控流程图.pid」。
  - 带窗口的运行 11 趟（01:18–01:34，整机占用 55 %–88 %），下面用字母叫它们，证据的文件夹也是这些名字。另有一趟带窗口的把 53 张逐张从备份面板打开（默认 P&ID 界面），和一组查「哪一步把图纸标成已修改」的试验（这个提交和图块分支的程序各一遍）。

  | 趟 | 界面 | 做了什么 |
  |---|---|---|
  | A | 经典 | 样例从备份面板打开 5 次：看属性面板、三种关法、`PIDTAG`、另存成 DWG 和 DXF |
  | B | 经典 | D07 和样例从备份面板打开：数据库把名字改了的图例、带空格的位号 |
  | C | 经典 | 样例只带 `.pid` 直接打开：属性面板、`PIDTAG` |
  | D | 默认 P&ID | 样例和 D07 从备份面板打开：检查栏、`PIDTAG` |
  | E | 默认 P&ID | 样例只带 `.pid` 直接打开：检查栏、`PIDTAG` |
  | F dwg、F dxf | 经典 | A 存下的 DWG、DXF 各自单独放在一个文件夹里再打开 |
  | H dwg、H dxf | 默认 P&ID | 同样两个文件在默认界面里再打开 |
  | G | 经典 | 一张直接打开的图在前面，D07 从备份面板打开在它后面，再切过去 |
  | N | 经典 | 反例：D07 从备份面板打开，删一条文字再关 |

- 规格 §6 逐条：

  | 条 | 规格要的 | 量到的 | 结论 |
  |---|---|---|---|
  | 1 | 样例只带 `.pid`：592 个块引用都有 `graphic`、`item`、`item_type`；590 个有 `class`，各类个数同 §2.2 的表；231 个有 `label`，全是 `file`；没有 `label_in_file` | 592、592、592；`class` 590，各类个数和 §2.2 的表逐行相同；`label` 231，全是 `file`；`label_in_file` 0。不开窗口量了一遍，带窗口 C、E 两趟各量一遍，数相同 | 符合 |
  | 2 | 样例从备份面板打开：354 个有 `label`，全是 `database`；没有 `label_in_file` | 带窗口从面板开了 8 次（A 5 次、B 2 次、D 1 次），命令行每次都说 354 个符号带上了数据库的位号、0 个与图纸文件里的名字不同；其中 4 次把图元逐个读出来数：354 个有 `label`，全是 `database`，`label_in_file` 0 | 符合 |
  | 3 | 53 张合计：`item_type` 9,218，`class` 8,742；只带 `.pid` 时 `label` 2,957；从面板打开时 `label` 5,027（`database` 5,006、`file` 21），`label_in_file` 32；D03、D04 导入成功，图例上只有 `graphic` | 9,218、8,742、2,957（全是 `file`）；从面板打开 5,027（5,006、21）、32。只有 `graphic` 的图例 7 个（D04 5 个，P井场、Q井场各 1 个），和 §2.2 的表相同。**D03 打不开**：程序说里面没有可画的图形，两种开法都一样；所以打开成功的是 52 张 | 数都相符；「D03 导入成功」不符（规格 §11 SI-7 第 1 条） |
  | 4 | 配对，按记录：样例 `symbol` 806、`line` 151、`other` 4，每条 `symbol` 的标注恰好对上一个块引用，383 个块引用至少一条标注；53 张 13,161、2,303、152，D04 上 95 条没有 `label_of`，6,110 个块引用至少一条标注 | 这次另写的检查程序按记录数了一遍：样例 961 条里 806、151、4；53 张 15,711 条里 13,161、2,303、152、95（都在 D04），属主 6,110 个，和规格相同。按图元的数见第 5 条 | 符合 |
  | 5 | 第 1–4 条是按记录量的；图元一级的数由任务量出来，不同的逐项说明，再写进测试 | 按图元：样例 803、151、4，383；53 张 13,035、2,303、152，95 条没有 `label_of`，至少一条标注的图例 6,091 个；每条 `symbol` 标注恰好一个属主。和记录的差全是内容空白的文字记录（不画图元）：样例 3 条，53 张 126 条，分在 17 张图上；有 19 个图例的标注全是空白的，所以 6,110 少了 19。53 张逐张对了 2,001 项，没有对不上的。样例的数 SI-2 已经写进测试；53 张的合计数没有写进测试 | 说明了；「写进测试」只做了样例 |
  | 6 | 样例带 `_Data.xml` 导入：592 个块引用上原有的键的值与图块分支验收时逐键相同；两边都有的图例上发布类相同 | 在发布版上重量：592 个块引用上原有的十二个键，值变了或者没了的 0 处，键的先后不对的 0 个；别的 5,798 个图元除了新加的两个标注键之外记录不同的 0 个。发布类两边都有的 590 个全部相同，只有 `_Data.xml` 给的 2 个（两个 OPC） | 符合，和 SI-1 量的相同 |
  | 7 | 带窗口选中一个有位号的阀门、一个没有位号的管件、一个 OPC、一条位号文字、一条线上的标注，属性面板和检查栏显示的符合 §4.5、§4.6 | 选了九种（规格的五种，另加一个仪表、一条没有位号的图例的标注、一条别的对象上的标注、一条普通的线），从面板打开和只带 `.pid` 两种开法各一遍：属性面板的行符合 §4.5（A、B、C），检查栏符合 §4.6（D、E）。D07 上数据库把名字改了的图例，位号、位号来源、图纸文件里的名字三行都在 | 符合 |
  | 8 | 样例从备份面板打开后，`PIDTAG` 加一个阀门位号，选中的恰是带这个位号的块引用和它的全部标注文字，视图缩放到它们；只带 `.pid` 时仪表位号同样能找到 | 从面板打开：`PIDTAG BV2501202A` 恰选中这个阀门和它的 3 条标注，视图移到它们上面，红框在（A、B、D）。只带 `.pid`：`PIDTAG PIA-1301104A` 恰选中 1 个仪表和它的 4 条标注（C、E）。两处要说明：找到之后视图动了，图纸就被标成已修改；带空格的位号敲不出来（5,027 个位号里 1 个） | 符合，带两处说明（规格 §11 SI-7 第 3、4 条） |
  | 9 | 样例 592 个引用、119 个定义；出图与图块分支逐字节相同 | 592、119；重出的图 14,769,401 字节，和图块分支验收时出的那一份逐字节相同 | 符合 |
  | 10 | 图例识别在样例上：认出的符号 0 个、未知块 0 个、散字 958 条 | 0、0、958 | 符合 |
  | 11 | 另存 DWG、DXF 再打开，§3 的键都在，第 7、8 条照样成立，不需要原来的 `.pid` | 从面板打开的样例另存成 DWG、DXF，各自单独放在一个文件夹里再打开：6,564 个图元的记录和另存之前逐条相同；属性面板九种选中（F）、检查栏（H）、`PIDTAG` 都和另存之前一样 | 符合 |
  | 12 | 样例的预算测试不放宽：导入 3 次，中位数不超过 3,747 毫秒 | 没有等到空闲的机器：别人的编译一直在跑，整机占用没有低过 52 %。不空闲时把这个提交和图块分支的测试程序轮流各跑了 14 轮：图块分支过 4 轮，这个提交过 1 轮。预算测试没有改 | **没有验证**（2026-11-21：机器空下来之后量了，过了，见 SI-11；同一天又补量了 10 轮空闲的，见 SI-11 的「补量」） |
  | 13 | 从备份面板打开图纸后不做任何修改就关闭，不提示保存 | 三种关法都不问：控制通道的关闭请求、`CLOSE` 命令、关窗口（A）；选中图元看属性面板之后、另存之后也不问；52 张从面板打开后都是这样关掉的。反例：删一条文字再关，会问（N）。要说明的：缩放、或者 `PIDTAG` 找到东西之后视图动了，图纸就算已修改，不带丢弃的关闭请求被拒绝；这时的弹窗没有另外去看 | 符合，带一处说明（规格 §11 SI-7 第 3 条） |
  | 14 | 图块计划的固定门禁每个提交都过 | SI-1 到 SI-6 各在自己的记录里，这次没有重跑；其中耗时预算在门禁那一遍里从 SI-2 起都超了（机器都不空闲），归在第 12 条。这个提交只动文档：文档检查改之前 590 个通过，改之后 590 个通过；测试、clippy、格式、分层没有重跑，代码和 SI-6 的提交相同 | 符合（SI-1 到 SI-6 以各自的记录为准，预算见第 12 条）；这个提交按只动文档的提交的先例 |

- 读数：
  - 53 张，只带 `.pid`（不开窗口；每张单独拷进一个文件夹再打开，拷贝和原件逐字节相同；一共 53.7 秒）：打开 52 张，D03 被拒绝。块引用 9,225 个，都有 `graphic`，没有两个块引用带同一个 `graphic`；有 `item`、`item_type` 的 9,218 个；有 `class` 的 8,742 个；有 `label` 的 2,957 个，全是 `file`；`label_in_file` 0 个；只有 `graphic` 的 7 个（D04 5 个；P井场 1 个，`graphic` 58352；Q井场 1 个，`graphic` 58466；后两个画的都是符号 `gap-curves`）。用到的块定义 2,857 个，模型空间图元 95,228 个。图元类型到发布类：PipingComp→PIDPipingComponent 3,473；Instrument→PIDInstrument 1,576、→PIDControlSystemFunction 1,104、→PIDInlineInstrument 1,025；ItemNote→PIDNote 793；OPC 476（没有发布类）；Nozzle→PIDNozzle 372；EquipmentOther→PIDProcessEquipment 225；Mechanical→PIDMechanicalEquipment 91；Vessel→PIDProcessVessel 68；Exchanger→PIDHeatTransferEquipment 12；PipeRun→PIDPipingConnector 3。个数和规格 §2.2 的表逐行相同。
  - 53 张，从备份面板打开（带窗口，默认 P&ID 界面，00:35 起；开始和结束时整机占用都是 100 %）：打开 52 张。有 `label` 的 5,027 个（`database` 5,006、`file` 21），`label_in_file` 32 个；`item_type`、`class`、标注配对的个数和只带 `.pid` 时相同。命令行每张说一句「有 N 个符号带上了备份数据库给其对象的位号；其中 M 个与图纸文件里的名字不同」，52 句的 N 合计 5,006、M 合计 32，没有一句和图元上数出来的不同。按每个图例在两种开法下的位号分五种：两边相同 2,904，只有数据库给了 2,070，两边不同 32，只有图纸文件有 21，都没有 4,198，和规格 §2.2 的表相同。模型空间图元 102,969 个，比只带 `.pid` 多 7,741 个，每一张多出来的都是图框上的图元（`role=frame`：从面板打开时 7,789 个，只带 `.pid` 时 48 个），其中文字 1,694 条。52 张都是不改就关：关之前没有一张被标成已修改，关的时候没有弹窗。
  - D03：两种开法都被拒绝，说的是「打开失败 : D03.pid 里没有可画的图形：解析器在里面没有找到图形」。读备份时 `PIDBACKUPOPEN` 说的是「53 张图纸，1 条警告」，D03 在面板里照样列着，点开才被拒绝。带窗口时拒绝之后程序弹出它的图纸恢复窗口（标题 Recovery，能关掉），并在设置目录里写了一份恢复报告（写的是打不开、扫到 0 个图元）。这条拒绝的规则在 `src/io/pid/mod.rs`，是 2026-11-11 的提交 `3d21e5666` 加的，在两份计划之前。
  - 记录和图元（第 4、5 条）：按记录，样例 961 条文字记录，挂在图例上 806、管线上 151、别的对象上 4；53 张 15,711 条，13,161、2,303、152，另有 95 条的上一级不是标注（都在 D04），有标注的图例 6,110 个。按图元，样例 803、151、4，有标注的图例 383 个；53 张文字 15,585 条，13,035、2,303、152，没有 `label_of` 的 95 条，有标注的图例 6,091 个；13,035 条挂在图例上的标注每条恰好一个属主，没有属主的、几个属主的都是 0；管线上和别的对象上的标注没有一条指着图例。差的是内容空白的文字记录，导入时不画：样例 3 条；53 张 126 条，全都挂在图例上，分在 17 张图上（第 2、3、4、5、6、8、9、10、11、12、13、44、46、47 张和 D01、D07、第 53 张，依次 6、4、2、7、4、4、3、4、6、6、5、6、2、16、30、15、6 条），内容是两个空格的 63 条、一个空格的 46 条、空的 13 条、三个空格的 4 条；13,161 − 126 = 13,035，15,711 − 126 = 15,585。有 19 个图例的标注全是空白的（D01 13 个、D07 6 个），6,110 − 19 = 6,091。有标注的图例各有几条：1 条的 1,093 个，2 条的 3,402 个，3 条的 1,270 个，4 条的 320 个，5 条的 3 个，11 条的 3 个。核对程序逐张比了 2,001 项（按记录的数、两种开法的数、上面说的差），对不上的 0 项。
  - 图面流：53 张里只有 Q井场一张有两个带图元的图面流：`/Sheet6`（3,388 个图元，其中图例 403 个、文字 615 条）和 `/JSite68432/Sheet6`（8 个图元，没有图例，没有文字）。D03 一个也没有，别的 51 张各一个。
  - 属性面板（经典界面；A、B、C，再打开的是 F）。从面板打开的样例上选中九种：
    - 有位号的阀门：类型 PIDPipingComponent，图元类型 PipingComp，符号 Ball Valve Type 2，位号 BV2501202A，位号来源 备份数据库，标注「3: NO · 1" · BV2501202A」，匹配方式 图纸文件里的对象。
    - 没有位号的管件（Concentric Reducer）：没有位号、位号来源两行。
    - OPC：类型 OPC，标注「2: DWG-0000集05-31 · 注气管线从集注站来」。
    - 仪表：位号 PIA-1301104A，标注「4: 1301104A · PIA · H=11.8MPa +1」。
    - 阀门的位号文字：标注属于 BV2501202A。没有位号的图例的标注：标注属于「PIDPipingComponent · Set Fire Point」。管线上的标注：标注属于 一条管线。别的对象上的标注：标注属于 其他对象。
    - 普通的一条线：没有这些行。
    - 只带 `.pid` 时：同一个阀门没有位号的几行，它的位号文字写的是标注属于「PIDPipingComponent · Ball Valve Type 2」；仪表的位号来源是 图纸文件。
    - D07 上数据库把名字改了的图例：位号 LG-Magnetic Float Gauge-0203，位号来源 备份数据库，图纸文件里的名字 LG-0203，标注「2: 0203 · LG」。D07 上只有图纸文件有名字的图例：位号 LG-003，位号来源 图纸文件。
    - 这些行是经控制通道读的（面板要显示的行），截图只看了其中几张。面板在默认宽度下长的值显示不全（看到的是「PIDPipingCompo」「3: NO · 1" · BV25」），拉宽面板没有试。
  - 检查栏（默认 P&ID 界面；D、E，再打开的是 H）：阀门，标题 BV2501202A，类型 管道元件，符号 Ball Valve Type 2，位号来源 备份数据库，标注 3 条；管件和 OPC，写的是「（没有位号）」；位号文字，「属于 BV2501202A」；没有位号的图例的标注，「属于 管道元件 · Set Fire Point」；管线上的，「管线上的标注」；别的对象上的，「其他对象上的标注」；普通的线，「Line · 不是 P&ID 对象」。D07 上改了名字的图例：类型 仪表，另有一行 图纸文件里的名字 LG-0203。只带 `.pid` 的仪表：类型 控制系统功能，位号来源 图纸文件。D、E、H dwg 三趟的截图逐张看了，H dxf 看了三张。
  - `PIDTAG`：
    - 从面板打开的样例（A、B、D）：`PIDTAG BV2501202A` 选中 4 个图元（阀门和它的 3 条标注），视图的中心落在这 4 个图元的外框里，红框在；`PIDTAG FT-833` 选中 3 个图元，是带这个位号的 3 个图例；点中阀门的位号文字再 `PIDTAG`，选中同样 4 个；点中没有位号的图例（Set Fire Point）再 `PIDTAG`，选中 2 个。
    - 改位号：`PIDTAG BV2501202A ZZ-0001` 被拒绝，说「PIDTAG: 本图上带位号 "BV2501202A" 的是 1 个导入的图例，导入的图例不能在这里改位号，未做改动。」
    - 只带 `.pid` 的样例（C、E）：`PIDTAG PIA-1301104A` 选中 1 个图例和它的 4 条标注；`PIDTAG MOV-1301102F` 选中 2 个图例和它们的 6 条标注；`PIDTAG BV2501202A` 找不到（这个位号只在数据库里）；点中一条标注再找，选中 4 个。
    - D07（B）：图纸文件里的旧名字 `LG-0203` 找不到；`PIDTAG LG-003` 选中 3 个图元；点中改了名字的那个图例再找，选中它和它的 2 条标注。敲它的位号 `LG-Magnetic Float Gauge-0203` 找不到：经控制通道发命令时只有第一个词到了命令，命令行说没有带位号 "LG-Magnetic" 的符号；按代码（`src/app/commands/pidlegend.rs` 的 `dispatch_pidtag`），参数按空白分词，一个词是查找、两个词是改位号、三个词报「改位号只接受两个参数」，所以在命令行里敲也找不到。53 张的 5,027 个位号里带空格的只有这 1 个。
    - 每次找到之后，图元和记录都没有变，修订号还是 0，但图纸被标成已修改。
  - 哪一步把图纸标成已修改（专门试的，两个程序各一遍）：这个提交上，选中图元、看属性面板，不算；`ZOOM`（全部、窗口），算；`PIDTAG` 加一个没有的位号，不算；`PIDTAG` 找到了（敲位号、点中图元），算；直接打开的和从面板打开的一样。标成已修改之后，不带丢弃的关闭请求被拒绝（`document_dirty`）；「Unsaved」窗口是在 N 那一趟看到的，缩放和 `PIDTAG` 之后没有另外去看弹窗。图块分支的程序上，选中不算，`ZOOM` 也算（同一条规则），`PIDTAG` 不算（它在那边找不到导入的图例，视图不动）。规则在 `src/app/update/viewport.rs` 的 `on_tick`：视图的位置变了就把图纸标成已修改，是 2026-05-01 的提交 `8f90a6893` 加的，只在带窗口的程序里起作用；不是这份计划改出来的，但 `PIDTAG` 能找到导入的图例是这份计划加的，所以「找一下就算改过」是新出现的。
  - 关闭（第 13 条）：从面板打开的样例，不改就关，三种关法都不问（A）：控制通道的关闭请求（图纸改过的话它会拒绝，这里没有拒绝）、`CLOSE` 命令、关窗口。选中九种图元看完属性面板之后、另存成 DWG 和 DXF 之后也不问。反例（N）：D07 从面板打开，删掉一条文字，图纸变成已修改，关闭请求被拒绝（`document_dirty`），`CLOSE` 弹出「Unsaved」窗口，关窗口关不掉。
  - 另存再打开（第 11 条；F、H 各两趟）：从面板打开的样例另存成 DWG（439,599 字节，AC1032）和 DXF（3,058,856 字节），存完图纸仍然不算改过，照样不问就关。两个文件各自单独放进一个空文件夹再打开：块引用 592 个，有 `class` 的 590 个，有 `label` 的 354 个，全是 `database`；文字 958 条，另有图框上的 34 条；标注 803、151、4，有标注的图例 383 个。和另存之前的图纸逐个图元比：6,564 个，句柄都在，记录不同的 0 个。属性面板九种选中的行和另存之前相同（F），检查栏相同（H），`PIDTAG` 敲阀门位号选中 4 个、`FT-833` 选中 3 个、点中标注选中 4 个。
  - 别的图纸后面补位号（G）：前面是一张直接打开的图（D01 的一份拷贝，不在备份里：33 个图例、5 个位号，全是 `file`），D07 从面板打开，落在它后面。D07 一打开就带着位号（8 个带上了数据库的位号，1 个和图纸文件里的名字不同）；前面那张的记录没有变。切到 D07，选中改了名字的图例，位号三行都在。两张都不问就关。
  - 「先显示、后补位号」：带窗口时没有看到过。每次从面板打开，控制通道第一次列出这张图纸的时候，命令行那一句已经在，探的那个图例已经带着数据库的位号；每个进程的第一张也是这样，53 张那一趟的 52 张也都是这样。
  - 第 6 条（发布版，不开窗口，样例单独放在文件夹里，拷贝和备份里的逐字节相同）：拿图块分支验收时存下的图元记录和这个提交的程序比，带 `_Data.xml` 和不带的各比一遍。带 `_Data.xml`：块引用两边都是 592 个，别的图元都是 5,798 个；块或者位置不同的 0 对；592 个块引用上原有的十二个键都和验收时一样，值变了或者没了的 0 处；键的先后不对、重复的 0 个；别的图元上除了两个标注键之外记录不同的 0 个；`class` 592，`label` 233，`oid` 592，`resolved` 全是 `direct`，没有 `tag_source`。不带 `_Data.xml`：同样是 0 处变了；验收时没有、现在由图纸文件给出的是 `class` 590、`label` 231、`resolved` 590。发布类按 `graphic` 对：两边都有的 590 个相同、不同的 0 个，只有 `_Data.xml` 给的 2 个（`graphic` 38445、38837，符号 `Off-Unit SQ`，类 `PIDBranchPoint`）；位号两边都有的 231 个相同、不同的 0 个，只有 `_Data.xml` 给的 2 个（`645`、`644`）。
  - 出图和图例识别（第 9、10 条，不开窗口）：样例 592 个引用、119 个定义，模型空间 6,390 个图元；重出的图 14,769,401 字节，sha256 开头 `e0b5e2e48fd157c0`，和图块分支验收时的 `branch-only.svg` 逐字节相同。图例识别：认出的符号 0 个，未知块 0 个，散字 958 条。
  - 最大的图上的耗时（EF井场，只带 `.pid`，不开窗口；模型空间 6,731 个图元，图例 635 个，文字 991 条，位号 220 个；量的时候整机占用 76 %–84 %）。每个数都是一次请求从发出到收到回答的时间，里面除了要量的查找还有拼面板的行、编码和传输，所以是上限：
    - 看一个图例的属性（里面要找它的标注）：635 个图例各一次，平均 6.96 毫秒，中位 7.0，第 95 百分位 11.3，最慢 13.3，没有一次超过 16。
    - 看一条图例上的标注的属性（里面要找它的属主）：837 条各一次，平均 1.41，最慢 6.46。
    - 所以规格 §4.6 说的「超过 16 毫秒就建索引」没有触发，索引没有建。把「选中」和「看属性」两次请求加起来算，最慢 16.7 毫秒，635 次里 5 次超过 16。
    - `PIDTAG <位号>`：220 个位号各一次。第一次 18.7 毫秒；其余 219 次平均 5.41，中位 4.85，第 95 百分位 10.3，最慢 13.0。
    - 点中一条标注再 `PIDTAG`：837 条各一次，平均 6.32，第 95 百分位 11.1，最慢 24.3，5 次超过 16；连选中那一次请求一起算，17 次超过 16，最慢 25.7。
    - 点中一个没有位号的图例再 `PIDTAG`：415 个各一次，平均 8.22，第 95 百分位 18.0，最慢 28.2，30 次超过 16。
    - 220 + 837 + 415 次，选中的图元都对。
  - 耗时预算（第 12 条）：
    - 等空闲：一个脚本从 00:56 起一直看着机器，等到没有编译在跑、整机占用不超过五分之一就跑预算测试。到 01:36 的四个十分钟里，整机占用 52 %–100 %（各段的中间值 71、80、66、64），别人的编译进程 0–37 个，一次也没有等到，预算测试一遍也没有在空闲时跑过。脚本留着跑到 06:56，记录在 `out\budget-idle.txt`。
    - 不空闲时，测试程序和图块分支的测试程序轮流跑（每轮各导入 3 次取中位数）。第一组 6 轮（00:42–00:46，整机 51 %–98 %）：图块分支 8,821、3,681、4,241、6,740、3,720、3,711，这个提交 4,043、4,146、6,234、7,497、3,844、3,926；逐轮的比值 0.458、1.126、1.47、1.112、1.033、1.058，中间值 1.085。第二组 8 轮（00:50–00:56）：图块分支最快 3,488、中位 3,899.5、最慢 4,624，这个提交 3,710、3,979.5、4,424；中位数之比 1.021，最快的一轮之比 1.064，逐轮比值的中位 0.985；处理器时间，中位数之比 1.015、最小之比 1.047。两组 14 轮的逐轮比值合起来，中位 1.046。各 14 轮里不超过 3,747 的：图块分支 4 轮，这个提交 1 轮。
    - 发布版（用户用的程序）打开样例，和图块分支的发布版轮流，量了两组，各 8 轮。第一组（00:47–00:50，整机 71 %–100 %，只量了用时）：图块分支中位 2,803 毫秒（2,051–3,958），这个提交 4,151（2,757–8,427），逐轮比值 0.92–3.79。第二组（00:50–00:56，连处理器时间一起量）：图块分支中位 2,025.5（1,836–2,470），这个提交 2,039（1,837–2,381），比值 1.007，逐轮比值 0.77–1.13；处理器时间之比 0.979。两组对不上：第一组里同一个程序自己的各轮就差一倍到两倍，机器的起伏比要量的差别大；第二组没有看出差别。
    - 这些读数能说的：在测试程序里，这个提交和图块分支的差别，各种算法从快 1.5 % 到慢 8.5 %，多数是慢 2 %–6 %，定不下来；在发布版里，机器稍静的那一组没有看出差别。图块分支在空闲的机器上是 3,488 毫秒（图块计划 SB-9 量的），预算 3,747 留给这个提交的余量是 7.4 %。拿上面的比值去乘 3,488，得到的数从 3,437（乘 0.985）到 3,784（乘 1.085），用合起来的中位 1.046 是 3,647：跨在 3,747 两边，推不出过还是不过。**第 12 条没有验证。**
    - 一个可能的解释（没有分段量过）：测试程序里本程序自己的代码不开优化，这份计划新加的代码在测试程序里比在发布版里慢得多，所以测试程序上看得到的差别在发布版上看不到。要确认，得在空闲的机器上把 SI-1 到 SI-6 逐个提交轮流量。
  - 每趟带窗口的运行都核对了：退出码 0；窗口是收到关闭消息后自己关的（N 按预期关不掉，由脚本结束）；用户桌面上的鼠标位置和前台窗口没有变；备份文件夹前后相同；控制通道的令牌没有写进任何文件；没有上网取东西（代理指到一个不通的本机端口，跑完查了设置目录里没有起始页的缓存文件）。
  - 文档检查（`python -m pytest scripts/pid-platform -q -p no:cacheprovider`）：改之前 590 个通过，改之后 590 个通过（三份文档都改完之后跑的）；检查前后工作树里改动的都只是这三份文档。
- 与清单不一样的地方：
  - 耗时预算没有在空闲的机器上重量（见上）。清单要的「仍然超过就查」只做了不空闲时能做的：和图块分支轮流量，量处理器时间，量发布版。
  - 「先显示、后补位号的第一张」没有看到：带窗口时每次都是第一次看到图纸的时候位号已经在了。每个进程的第一张和后面的各看了第 2 条和第 13 条，结果相同。
  - 「图纸在别的图纸后面补了位号、切回来之后属性面板换没换」只做到一半（G）：D07 是在别的图纸后面打开并带上位号的，切过去看是对的；但它带上位号之前没有显示过，也没有选中的图元，所以「面板上原来是没有位号的行，位号到了之后换不换」没有碰到。
  - 没有用真的鼠标、键盘（见「怎么量的」）：双击、「另存为」对话框、在命令行里敲字，都是用发同一条消息或者同一个请求代替的。
  - 规格 §6 第 5 条要把图元一级的数写进测试。样例的数 SI-2 已经写进测试；53 张的合计数没有写进任何测试，只记在这里和规格 §11：这个任务的文件清单只有文档。要不要加测试等拍板。
  - 最大的图上的耗时是在不空闲的机器上、不开窗口量的，量的是整次请求，不是单独的查找那一步。
  - 更正了 SI-3、SI-5 的记录和规格里的「优化过的测试程序」：`Cargo.toml` 的 `[profile.dev]` 开了优化，但 `[profile.dev.package]` 把本程序自己的几个包设成不优化，`[profile.test]` 只改了调试信息；编测试程序的记录里本程序那一行没有优化参数（`build-libtest-final.log`）。Cargo 最后打印的「optimized」说的是整个配置。那两处的读数没有变，只是它们是「依赖库优化、本程序自己不优化」的程序上的读数。
  - 清单没有要、这次多做的：第 7 条多选了四种（仪表、没有位号的图例的标注、别的对象上的标注、普通的线）；一趟反例（N）；一组「哪一步把图纸标成已修改」的试验；发布版的两组轮流量。
- 没验证的：
  - 第 12 条：空闲的机器上预算测试过不过。这个提交比图块分支到底慢多少、慢在哪一步，也没有定下来。
  - 带窗口时「先显示、后补位号」这条路：没有出现过，所以位号到的那一刻属性面板、检查栏换不换，没有看到。这条路只有 SI-6 的不开窗口的测试。
  - 图纸显示着、选中了图元，位号这时才到（不管它在前面还是在别的图纸后面）：没有碰到。
  - 带空格的位号在真的键盘上敲进 `PIDTAG`：没有试。经控制通道只有第一个词到了命令；「三个词会报参数个数不对」是读代码得来的，没有跑出来。
  - 真的鼠标、键盘、双击、「另存为」对话框：都没有用。
  - 缩放、`PIDTAG` 找到东西之后关闭时的弹窗：没有看。看到的是图纸被标成已修改、不带丢弃的关闭请求被拒绝；弹窗是在删了一条文字的反例（N）里看到的。平移（不缩放）没有单独试。
  - 属性面板的行是经控制通道读的，截图只看了几张；默认宽度下长的值显示不全，拉宽之后怎样没有试。检查栏的截图 D、E、H dwg 逐张看了，H dxf 只看了三张。
  - 第 6 条只在样例一张上做。
  - 最大的图上的耗时：一张图，不开窗口，机器不空闲；带窗口时从选中到面板画出来要多久，没有量。
  - 另存再打开：只做了从面板打开的样例一张；DWG 只存了一种版本（AC1032）；只带 `.pid` 打开的没有另存。
  - 从面板打开的 52 张：只数了记录、核了命令行那一句、不改就关；没有逐张看图、逐张选中、逐张 `PIDTAG`。看属性面板和检查栏的只有样例和 D07。
  - 数据库给的位号：核的是个数，和每个图例在两种开法下的关系（相同、只有数据库有、不同、只有文件有、都没有），和规格 §2.2 的分析对得上；没有把 5,006 个位号的文字逐个去和数据库里的值比。
  - D03 被拒绝之后弹出的图纸恢复窗口：只记了它出现、能关掉，和它写的报告文件；窗口里写了什么没有看。`PIDBACKUPOPEN` 说的那 1 条警告是什么，没有查。
  - Q井场的第二个图面流（8 个图元）导入之后画没画、画在哪，没有查。
  - 界面语言只看了简体中文。
  - 一台机器（Windows，看不见的桌面 2560×1369，缩放 1.0）。
  - 固定门禁里除了文档检查之外的（单元测试、集成测试、clippy、格式、分层）在这个提交上没有重跑：代码和 SI-6 的提交相同，读数见 SI-6 的记录。
  - 53 张的图元一级合计数没有写进测试。
- 证据：`D:\scratch\ocs-symblock\si-7\`（仓库之外）。
  - 程序：`head\OpenCADStudio.exe`（发布版，编译记录 `build-head.log`、`build-head.done`）、`libtest-final.exe`（测试程序，编译记录 `build-libtest-final.log`）。对照用的图块分支的程序：`..\sb9\lib-final.exe`、`..\sb10\branch\OpenCADStudio.exe`。
  - 53 张：只带 `.pid` 的 `out\file\totals-01-53.json` 和每张的 `NN.json`、`NN.rows.jsonl`；从面板打开的 `out\panel\totals.json`、`out\panel\run-20261121003556.json`，D03 的 `out\panel\refused-49.json` 和恢复报告 `out\panel\appdata\OpenCADStudio\recovery_logs\`；两遍互相对、再和按记录的数对 `out\compare.json`；按记录的数 `out\records.json`；空白的文字记录和图面流 `out\textprobe.txt`、`out\textprobe.json`。
  - 带窗口的 11 趟：记录 `out\win-A.log`、`win-B.log`、`win-C.log`、`win-D.log`、`win-E.log`、`win-F-dwg.log`、`win-F-dxf.log`、`win-G.log`、`win-H-dwg.log`、`win-H-dxf.log`、`win-N.log`；结果文件夹 `out\win-A`、`win-B`、`win-C`、`win-D`、`win-E`、`win-Fdwg`、`win-Fdxf`、`win-G`、`win-Hdwg`、`win-Hdxf`、`win-N`（各有 `run.json`、`report.txt` 和截图）；另存出来的文件 `data\saved\dwg\lm-from-the-panel.dwg`、`data\saved\dxf\lm-from-the-panel.dxf`。
  - 哪一步标成已修改：`out\dirty-head\dirty.json`、`out\dirty-branch\dirty.json`。
  - 第 6 条：`out\item6\item6.txt`、`out\item6\item6.json`。出图：`out\plot\plot.json`、`out\plot\head-only.svg`。最大的图上的耗时：`out\timing\timing-05.json`。
  - 耗时预算：测试程序第一组 `out\budget-ab.txt`；测试程序第二组和发布版第二组 `out\cpu-ab.json`；发布版第一组 `out\open-ab.json`；等空闲的记录 `out\budget-idle.txt`。
  - 文档检查：`out\doccheck-before.log`、`out\doccheck-after.log`。
  - 脚本都在 `si-7\`，都不写仓库：`si7lib.py`（两种开法）、`sheets.py`、`panel.py`、`compare.py`、`records.py`、`textprobe.py`（53 张）、`win.py`（带窗口的各趟）、`dirty.py`、`item6.py`、`plot.py`、`timing.py`、`cpu-ab.py`、`open-ab.py`、`budget-ab.ps1`、`idle-watch.ps1`、`doccheck.ps1`、`build-release.ps1`、`build-libtest.ps1`。

### SI-8 53 张的合计数写进测试，D03 的说法更正 【测试】【文档】

- **做什么**：SI-7 留下等拍板的四处里有明确建议的两处（规格 §11 的 SI-7 第 1、2 条；用户 2026-11-21 定：按建议做）。一，D03：「没有可画的图形就拒绝」的规则不改，把规格里写成「导入成功」的两句更正。二，53 张按图元的合计数写进一条测试：它读仓库外面的备份，平时不跑；备份不在时说明原因，算通过。顺带把另外两处立成任务 SI-9、SI-10。
- **验收**：
  - [x] 测试钉住规格 §6 第 3 条的数、第 4 条按图元的数，和「被拒绝的恰好是 D03 一张」。
  - [x] 先红后绿；故意改坏时变红。
  - [x] 规格里 D03 的两句更正，原文留着，注明日期。
  - [x] 十条固定门禁。
- **依赖**：SI-7。
- **文件**：`src/io/pid/plant_totals.rs`（新）、`src/io/pid/mod.rs`、规格、本清单。
- **规模**：S（代码两个文件，另有两份文档）。

**做完**（2026-11-21）

- 结论：两处都做了。程序的行为没有变：加了一条平时不跑的测试（和挂上它的两行），改了两份文档。53 张按图元的合计数和 SI-7 量到的相同，现在有一条测试钉着；这条测试要单独带 `--ignored` 跑，十条固定门禁里它只是「忽略」的一条，门禁不证明那些数。耗时预算（规格 §6 第 12 条）仍然没有验证。
- 改了什么：
  - `src/io/pid/plant_totals.rs`（新，383 行，只在测试里编；2026-11-21 更正：原来写的是 384，见 SI-9 记录的「更正」）：一条测试 `the_drawings_of_the_plant_backup_come_to_the_counts_taken_of_them`，标了 `#[ignore]`。它给备份建索引，从数据库导出里读位号（8,145 个），把 53 张逐张导入，先数只带图纸文件时的图元，再补上数据库的位号数一遍；三条断言：被拒绝的恰好是 D03 一张和那句原因、只带图纸文件时的 21 行数、补了位号之后的 4 行数。钉的数逐个列在规格 §11 的 SI-8 第 2 条。备份不在、或者设了 `PID_SYMBOL_LIBRARY` 时打印一句 `skipped: …` 就结束。逐张的数和用时打印出来，不断言。
  - `src/io/pid/mod.rs`：加两行（`#[cfg(test)] mod plant_totals;`）。
  - 规格：开头的状态行；§2.2 和 §6 第 3 条里 D03 的两句更正；§6 第 5 条注明合计数由这个任务写进了测试；§6 的测试一览加一条；§11 的 SI-7 一节第 1 到 4 条末尾注明怎么定的，第 15 条更正一句（见下面「更正」）；§11 加 SI-8 一节。
  - 本清单：开头的状态行；这段记录；SI-9、SI-10 两个任务的定义。
- 先红（02:03–02:10，`libmut-red.txt`）：测试先按规格 §6 第 4 条的数写期望值（按记录的：挂在图例上的标注 13,161，有标注的图例 6,110，标注合计 15,616），跑出来是红的：实得 13,035、6,091、15,490，停在第二条断言，别的行都相同。实得的数和 SI-7 按图元量到的相同，差在哪见规格 §11 的 SI-7 第 2 条。期望值改成按图元的数之后通过。这是一条给现有行为记数的测试，没有「实现之前」可红：红的这一遍证明的是期望值写错时它会红，和规格第 4 条的数不是图元的数。
- 故意改坏（02:10–02:17，`libmut-mut.txt`；五处都改在产品代码里，用环境变量开关，跑完已经拿掉）：

  | 改坏 | 改的是什么 | 结果 |
  |---|---|---|
  | （不改） | | 通过，43 秒 |
  | `empty` | 关掉「没有可画的图形就拒绝」 | 红：被拒绝的 0 张，停在第一条断言 |
  | `forget` | 补数据库的位号时不留文件里的名字 | 红：`label_in_file` 0 个，「都有但不同」的 32 个对不上，停在第三条断言 |
  | `line` | 挂在别的对象上的标注算成线上的 | 红：线上 2,455，别的对象上没有，停在第二条断言 |
  | `vessel` | 容器不给 `class` | 红：带 `class` 的 8,674，容器 68 个没有 `class`，停在第二条断言 |
  | `astray` | 标注记的属主号写错 | 红：属主不在图上的 12,137 条，有标注的图例 395 个，停在第二条断言 |

  三条断言都停过。改坏只跑了这一条测试。
- 读数（固定门禁在最终的代码上跑了一遍，02:48–03:00，`chainfinal2.txt`）：
  - 格式：两个 `.rs` 文件都是 `new=0`（新文件整个合格；`src/io/pid/mod.rs` 原有的不合格处一处没有多）。
  - 快速筛选 `io::pid::`：99 过、2 忽略（上一个提交是 99 过、1 忽略）。
  - 全量单元测试 7,227 个：通过 7,170，忽略 57，失败 0（上一个提交是 7,226 个、忽略 56；多的一个就是这条）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录里指到这个任务的文件的各有 1 条，是同一条原有的：`src/io/pid/mod.rs` 第 612 行 `finish` 的参数太多，上一个提交的记录里同一行就有（这个任务加的两行在第 781、782 行）；新文件 0 条。
  - 分层报告与上一个提交逐行相同。
  - 文档检查没有跑：没有动 `docs/pid-platform/`。
  - 过了门禁的这份代码编出来的测试程序，再单独跑了一次这条测试：通过，46.16 秒（03:12:21–03:14:34 编译并跑完，`libmut-green2.txt`）。
- 门禁是怎么跑成的（和前几个任务不一样）：这台机器上本项目所有工作树的编译落在同一个编译目录里，这一晚另有至少两个会话在别的工作树上几乎不停地编译和跑测试。两边同时编会互相弄坏：第一遍门禁（02:18 起，`chainfinal.txt`）格式和快速筛选过了（99 过、2 忽略），全量单元测试那一步 64 秒后失败，是编译目录里的文件被另一边的编译动了（写 `invoked.timestamp` 时系统报找不到路径），不是代码的错；这一遍作废。编译目录改不成自己的（试了四种设法，这个 cargo 都不认），所以写了三个只读的脚本让门禁排队：连续三次（隔 15 秒）看不到别的会话的 cargo 才开始，跑的时候旁边每 10 秒记一次有没有人插进来。排了三次：前两次（02:25、02:34 起）等的时候发现脚本把别的项目的编译也算成了对手，改了脚本重来，门禁都没有开始；第三次 02:38 起等了 587 秒，02:48:06 开始，03:00:34 跑完。没有动别人的进程。
- 排了队也没有躲开：门禁跑的 12 分半里旁边记了 73 次，从 02:53:50 起有 27 次看到别的会话的 cargo（别的工作树上的 `cargo test --lib …`，和一次 `cargo check`）。它们当时有几次正在编译，记录里看不出来：那时的监视只认命令行里带编译目录名的编译进程（记到 02:58:39 一次），而本程序这个包的编译进程是用参数文件启动的，命令行里没有目录。这是 03:04 重跑一步时直接看进程表发现的（那个会话的 cargo 名下有编译进程在跑，监视记的是 0 个），之后监视改成把别的 cargo 名下的编译进程也数进去。那个会话每隔几分钟来一次，「没有人同时在编」等不到；各步的结果按各步自己的记录认定是这棵树的：
  - 快速筛选和全量单元测试（02:48:06–02:52:06）：这段时间监视没有看到别的会话的 cargo；忽略 2 个和 57 个、总数 7,227 个，只有加了这条测试的这棵树才有。
  - 集成测试（02:52:06–02:55:35）：先改了 `src\lib.rs` 的修改时间，记录里写着重新编的是本工作树的包；旁边的 `cargo test --lib` 编的是单元测试程序，集成测试用的库和三个测试程序是这一步自己编、自己跑的。
  - clippy 两步（02:55:35–03:00:19）：记录里写着检查的是本工作树的包；条数是 clippy 检查这些源码时打印的警告数，旁边的编译能让这一步失败，改不了条数。`--lib --profile test` 那一步在 03:03:34–03:05:07 又跑了一遍（`chainfinal3.txt`），旁边同样有别的会话在编，同样是 1,051 条。
  - 门禁之后单独跑的那一次（03:12:21–03:14:34）：跑的是从编译目录拷出来的测试程序，它里面有这条测试（跑出 1 个通过），所以是这棵树编出来的。这一次改过之后的监视记了 14 次，没有看到别的会话的 cargo（`quietfinal3b.txt`）。
- 更正（SI-7 的提交 `f6016c8b1` 的说明里有两句不准；提交不改写，记在这里）：
  - 「带窗口从面板开了 8 次，每次相同。」准确的说法：样例带窗口从面板打开了 8 次，命令行那一句每次相同；逐个读图元的记录来数的是其中 4 次。
  - 「没有用真的鼠标和键盘：开图、选中、另存、关窗口都经程序自己的命令和控制通道」准确的说法：开图、选中、另存是经程序自己的命令和控制通道做的；关窗口是给窗口发关闭消息。这份清单 SI-7 的「怎么量的」本来就是这样写的；规格 §11 的 SI-7 第 15 条同样的一句已经更正。
- 与清单不一样的地方：
  - 这个任务是做的时候立的：定义和记录一起写进来，验收一开始就是勾上的。
  - 先红不是「实现之前红」（见上面「先红」）。
  - 门禁跑了两遍，第一遍作废，记的是第二遍；clippy `--lib --profile test` 那一步多跑了一遍；排队的做法是新的，而且没有躲开别的会话（见上面两段）。
- 没验证的：
  - 耗时预算（规格 §6 第 12 条）：等空闲的脚本从 00:56 起一直在等，一轮也没有跑成，机器没有空下来过。
  - 门禁没有整遍在「没有人同时在这个编译目录里编译」的条件下跑成过：集成测试和 clippy 两步旁边都有别的会话的 cargo，结果是按上面列的理由认定的，不是按「没有人插进来」。监视只认带 `--offline` 的 cargo，别的写法启动的编译它看不到。
  - 这条测试不在固定门禁里跑：以后导入改了、数变了，门禁不会报，要有人带 `--ignored` 跑它才看得到。
  - 测试不经过备份面板、不开窗口：面板打开时补位号的那一路，53 张的合计数只有 SI-7 带窗口量的那一遍，没有测试钉着。
  - 95 条没有 `label_of` 的文字只钉了合计；逐张的数只打印，不断言，所以「一张多了、另一张少了同样多」测不出来。
  - 备份不在、设了 `PID_SYMBOL_LIBRARY` 时「说明原因、算通过」的两条路没有跑过，是读代码得来的。
  - 改坏只跑了这一条测试；全量单元测试里别的测试没有在改坏下跑。
  - 这条测试的用时（43 到 75 秒）都是在不空闲的机器上量的。
- 证据：`D:\scratch\ocs-symblock\si-8\`（先红：`libmut-red.txt`、`libmut-red-none.log`；绿和改坏：`libmut-mut.txt`、`libmut-mut-*.log`；固定门禁：`chainfinal2.txt` 和它列的各份记录，排队和监视 `quietfinal2c.txt`，前两次排队 `quietfinal2.txt`、`quietfinal2b.txt`，作废的第一遍 `chainfinal.txt`、`lib-testfinal.log`；重跑的那一步：`chainfinal3.txt`、`clippy-lib-testfinal3.log`、`quietfinal3.txt`；门禁之后单独跑的那一次：`libmut-green2.txt`、`libmut-green2-none.log`、`quietfinal3b.txt`）。新写的脚本在 `D:\scratch\ocs-symblock\tools\`：`quiet-run.ps1`（排队）、`foreign-watch.ps1`（跑的时候在旁边记）、`foreign-lib.ps1`（认出别的会话在这个编译目录里的编译），都只读进程表，不写仓库，不动别人的进程。

### SI-9 `.pid` 图纸上只动视图不算改过 【接线】

- **做什么**：规格 §11 的 SI-7 第 3 条：视图的位置一变，图纸就被标成已修改，所以从备份面板打开的图纸缩放一下、或者 `PIDTAG` 找到东西之后，关闭时会问保存。这条规则是 2026-05-01 的提交 `8f90a6893` 加的，为的是视图随文件保存。对写得回去的图纸（DWG、DXF、还没保存过的新图）不改。`.pid` 是只读来源，没有地方存视图：在路径是 `.pid` 的标签页上，只动视图不再把图纸标成已修改。视图仍然写进文档（另存成 DWG 时带着最后看到的视图）；已经改过的 `.pid` 图纸动了视图仍然是已修改。
- **验收**：
  - [x] `.pid` 图纸上滚轮缩放、`ZOOM`、`PIDTAG` 找到东西之后，图纸不是已修改，关闭不问保存。
  - [x] 文档里存的视图仍然是最后看到的。
  - [x] DWG 和没保存过的新图照旧：视图一动就是已修改。
  - [x] 改过的 `.pid` 图纸动了视图仍然是已修改。
  - [x] 布局里进了视口时同样。
  - [x] 先红后绿、故意改坏、十条固定门禁。
  - [x] 带窗口复核：从备份面板打开，缩放、`PIDTAG` 之后不带丢弃的关闭请求不被拒绝（做不了就写明）。
- **依赖**：SI-8。
- **文件**：`src/app/update/viewport.rs`、`src/app/pid_read_only_view_tests.rs`（新）、`src/app/mod.rs`、规格、本清单。
- **规模**：M。

**做完**（2026-11-21）

- 结论：做了。标签页的路径是 `.pid` 的图纸（直接打开的、从备份面板打开的都是），只动视图不再被标成已修改：缩放、`PIDTAG` 找到东西之后关闭，不问保存。视图照旧写进文档，另存成 DWG 时带着最后看到的视图。已经改过的 `.pid` 图纸、DWG、没保存过的新图都照旧。耗时预算（规格 §6 第 12 条）仍然没有在空闲的机器上验证。
- 改了什么（产品代码 +14 −3，只有 `src/app/update/viewport.rs` 一个文件；测试 +272）：
  - `src/app/update/viewport.rs` 的 `on_tick`：视图动过之后的下一拍，原来两处都是直接把图纸标成已修改（`dirty = true`）。现在先看这个标签页的路径是不是只读来源，用的是保存那边已有的判断 `crate::io::is_read_only_source_path`（现在只有 `.pid`）；两处都改成 `dirty |= view_is_kept`：路径不是只读来源、或者没有路径时照旧立起标记，是只读来源时不立。这一步只立标记，从不拿掉。把视图写进文档的那一步（`sync_camera_to_document`）没有动，`.pid` 图纸上照旧执行。
  - `src/app/pid_read_only_view_tests.rs`（新，270 行，只在测试里编）：5 条测试。`src/app/mod.rs` 加两行挂上它。
  - 规格：开头的状态行；§6 第 13 条注明；§7 的测试一览加一条；§11 的 SI-7 第 3 条末尾注明；§11 加 SI-9 一节。
  - 本清单：开头的状态行；SI-9 的验收勾上；这段记录；SI-8 的记录里更正一个行数（见下面「更正」）。（2026-11-21 更正：开头的状态行和验收里「先红后绿、故意改坏、十条固定门禁」一条，SI-9 的提交里漏了，是 SI-10 的提交补上的，见 SI-10 记录的「更正」。）
  - 没有动：保存、另存、关闭时的提问、`PIDTAG`、导入、备份面板。没有加设置项，没有加词条。
- 规则（怎么取舍、为什么，记在规格 §11 的 SI-9）：
  - 看的是标签页记的路径：扩展名是 `pid` 的（大小写不计）是只读来源。直接打开 `.pid` 和从备份面板打开，标签页的路径都是 `.pid`（带窗口复核的 14 个场景里都是）。
  - 这样的标签页上，视图动过之后照旧把视图写进文档，只是不把图纸标成已修改；关闭时不问保存。
  - 标记只立不拿：图纸已经是已修改的（画过、删过、改过），动了视图仍然是，关闭照旧问。
  - 路径是 DWG、DXF 的，和还没有路径的新图，照旧：视图一动就是已修改（视图要随文件保存）。
  - 布局里进了视口时是同一个判断（那里视图写在视口图元上，不经过「写进文档」那一步）。
  - 块编辑器里动视图本来就不标，没有变。
  - 另存成 DWG 之后，标签页的路径是那个 DWG，从这时起照 DWG 的规则。
- 简称（下面用）：
  - V1 `moving_the_view_of_a_read_only_source_is_no_edit`
  - V2 `the_document_of_a_read_only_source_still_holds_its_view`
  - V3 `moving_the_view_of_a_drawing_that_keeps_its_view_marks_it_modified`
  - V4 `moving_the_view_leaves_an_edited_read_only_source_modified`
  - V5 `moving_the_view_inside_a_layout_viewport_follows_the_same_rule`
- 验收的每一条对哪条测试。5 条都在 `src/app/pid_read_only_view_tests.rs`。用的图是导入器自己的那张拼出来的图（一个带位号的阀门和它的标注），照不开窗口时打开图纸的办法放进最前面的标签页；标签页的路径是手工设的（一个 `.pid` 的名字、一个 `.dwg` 的名字，或者没有），磁盘上没有这个文件，测试也不写文件。动视图用三种办法：滚轮（发 `ViewportScroll` 消息）、`ZOOM EXTENTS`、`PIDTAG GV-101`（找到阀门，视图移过去）；每动一次之后走两拍：

  | 验收 | 测试 | 钉住的 |
  |---|---|---|
  | `.pid` 图纸上滚轮缩放、`ZOOM`、`PIDTAG` 找到东西之后，不是已修改，关闭不问保存 | V1 | 路径是 `.pid`：三种办法各动一次，图纸都不是已修改；发关闭标签页的消息，程序没有记下「等着问保存」，标签页关掉了 |
  | 文档里存的视图仍然是最后看到的 | V2 | 路径是 `.pid`：三种办法各动一次之后，背着文档把相机挪到别处，再让文档把存的视图还回来（`restore_saved_camera_view_only`），得到的就是刚才看到的（中心的两个坐标和范围各差不到 0.001） |
  | DWG 和没保存过的新图照旧 | V3 | 路径是 `.dwg`、没有路径，各三种办法：视图一动图纸就是已修改，关闭时程序记下「等着问保存」，标签页还在 |
  | 改过的 `.pid` 图纸动了视图仍然是已修改 | V4 | 路径是 `.pid`：先画一条线（已修改），再动视图（三种办法各一遍）：仍然是已修改，关闭时问 |
  | 布局里进了视口时同样 | V5 | 加一个布局、一个视口，进到视口里，`ZOOM EXTENTS`：路径是 `.pid` 的不是已修改，是 `.dwg` 的是已修改 |

- 先红（07:14–07:20，`libmut-red.txt`；SI-9 和 SI-10 的测试都在树里、两件的实现都还没有写的时候跑的）：按名字筛出 31 条：这 5 条，SI-10 的 8 条，原有的 `PIDTAG` 和改位号的 13 条、只读会话的 4 条，还有 1 条是名字里带 rename 被一起筛到的线型管理器的测试。22 过 9 红；这 5 条里红的是 V1 和 V5，另外 7 条红的是 SI-10 的。V1 停在第一种办法（``the wheel: looking round a `.pid` is not an edit``）；V5 停在路径是 `.pid` 的那一半（是已修改，期望不是）。V2、V3、V4 这时就是过的：它们钉的是不该变的三样（视图照旧写进文档，DWG 和新图照旧，改过的仍然是已修改），它们会不会红要看下面的故意改坏。这一遍旁边记了 36 次，20 次看到别的 cargo（另一个项目的编译，不在这个编译目录里；07:17 起还有另一个会话的 `cargo check`，它在等编译目录）；红的 9 条的名字和断言只有这棵树才有。
- 故意改坏（07:23–07:26，`libmut-green.txt`；开关加在 `on_tick` 里，用环境变量开，跑完已经拿掉；和 SI-10 的 7 处编在同一个测试程序里，一次编译）：

  | 改坏 | 改的是什么 | 结果 |
  |---|---|---|
  | （不改） | | 31 条全过 |
  | `si9-never` | 什么图纸都算存得下视图（就是原来的规则） | 红 V1、V5（路径是 `.pid` 的那一半） |
  | `si9-every` | 什么图纸都不算（视图动了谁都不标） | 红 V3、V5（路径是 `.dwg` 的那一半） |
  | `si9-no-sync` | `.pid` 图纸的视图不写进文档 | 红 V2：刚才看到的是 [140.0, 115.0, 57.63]，文档还回来的是 [125.4, 115.0, 57.75] |
  | `si9-clears` | 标记用赋值，不是只立不拿 | 红 V4：画过线的图纸动了视图之后不再是已修改 |
  | `si9-floating` | 布局视口那一支照旧直接标 | 红 V5（路径是 `.pid` 的那一半） |

  每一处红的只有表里列的那几条，31 条里别的都过。5 条测试各至少被一处改坏变红过。这一遍旁边记了 15 次，没有看到别的会话在编。
- 带窗口复核（07:42–07:45，`recheck-both.log`；发布版，SI-9 和 SI-10 两件的改动都在里面，07:42:22 编出，206,542,336 字节；经程序自己的控制通道，窗口开在另建的看不见的桌面上，不动真实的鼠标和键盘）。同一个脚本先在改之前的程序上跑过一遍作对照（03:07 跑完，`recheck-old.log`；程序是 SI-6 的提交 `be03c18e9` 编的发布版，之后的 SI-7、SI-8 没有动产品代码）：
  - 样例只带 `.pid` 打开、从备份面板打开，各走 7 个场景，每一步之后等 0.7 秒再读：只看（列图元、选中一个图例、读属性页、程序自己截图、取消选中）；`ZOOM E`；`ZOOM W` 到一个图例；`PIDTAG` 一个没有的位号；`PIDTAG` 一个有的位号，控制通道的两种送法各一个场景；选中一个图例、`PIDTAG` 不带参数、回车。14 个场景里标签页的路径都是 `.pid`。这次的程序：14 个场景没有一个被标成已修改（其中 10 个视图动了），不带丢弃的关闭请求都通过。改之前的程序：视图动了的 10 个场景都在动视图的那一步被标成已修改，不带丢弃的关闭请求被拒绝；视图没动的 4 个和这次一样。
  - 改过再动视图：样例从面板打开，删一条文字（已修改），再 `ZOOM E`（视图动了）：仍然是已修改，不带丢弃的关闭请求被拒绝（`document_dirty`）。
  - 另存带着视图：样例从面板打开，`ZOOM W` 到一个图例（没有被标成已修改），另存成 DWG（423,183 字节），再打开这个 DWG：视图是另存前看到的那个（中心 569.122, 194.103，距离 109.792），不是 `.pid` 刚打开时的（中心 594.5, 420.5，距离 1950.703）。另存之后标签页的路径是 `.dwg`，不是已修改，不带丢弃的关闭请求通过。
  - 存得下视图的图纸照旧：SI-7 存的那份 DWG（拷了一份）打开，`ZOOM W` 到一个图例：被标成已修改，不带丢弃的关闭请求被拒绝。改之前的程序上相同。
  - 桌面没有动（指针和前台窗口都在原处）；备份文件夹前后相同；输出的文件里带控制通道令牌的 0 个；没有从网络取到东西。
  - 这一遍的第五段（D07 上带空格的位号）记在 SI-10 的记录里。
- 读数（固定门禁在只有 SI-9 的代码上跑了一遍，07:45–07:55，`chainfinal.txt`）：
  - 格式：三个 `.rs` 文件都是 `new=0`（新文件整个合格；`src/app/update/viewport.rs` 原有的 21 处不合格一处没有多；`src/app/mod.rs` 没有不合格处）。
  - 快速筛选 `pid_read_only_view_tests`：5 过。
  - 全量单元测试 7,232 个：通过 7,175，忽略 57，失败 0（上一个提交是 7,227 个、通过 7,170；多的 5 个就是这 5 条，全量的记录里 5 条都是 ok）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录里指到这个任务的文件的各有 23 行（`src/app/mod.rs` 4 行、`src/app/update/viewport.rs` 19 行），和上一个提交的记录逐行对得上，只是行号顺着挪了（`mod.rs` 第 90 行之后加 2，`viewport.rs` 第 1842 行之后加 11）；改的那一段（`viewport.rs` 第 1842–1869 行）和新文件上 0 条。
  - 分层报告与上一个提交逐行相同。
  - 文档检查没有跑：没有动 `docs/pid-platform/`。
- 门禁旁边有别的会话在编（和 SI-8 一样，排了队也没有躲开；`quietfinal.txt`）：连续三次看不到别的会话的 cargo 才开始（等了 31 秒，07:45:55 开始，07:55:11 跑完）；跑的时候旁边记了 54 次，38 次看到别的会话的 cargo，其中 37 次它名下有编译进程在跑（07:46:46 到 07:55:01：别的会话先后两次 `cargo test --lib …`，编在同一个编译目录里；最后一次是另一个项目的 `cargo build`）。没有动别人的进程。各步都是一遍就过，结果按各步自己的记录认定是这棵树的：
  - 快速筛选和全量单元测试（07:45:55–07:48:55）：筛出来的 5 条只有这棵树才有；总数 7,232 个，只有加了这 5 条、还没有 SI-10 的 8 条的这棵树才有。
  - 集成测试（07:48:55–07:51:28）：先改了 `src\lib.rs` 的修改时间，记录里写着重新编的是本工作树的包。
  - clippy 两步（07:51:28–07:55:01）：记录里写着编的、检查的是本工作树的包；条数是 clippy 检查这些源码时打印的警告数，旁边的编译能让这一步失败，改不了条数。
- 更正（SI-8 的记录）：上面 SI-8「改了什么」里 `src/io/pid/plant_totals.rs` 的行数原来写的是 384，是 383（384 是读文件的工具报的数，比换行符的个数多一；`git diff --stat` 报的是 383）。已经在原处改了，注明了日期。SI-8 的提交 `bd289be2e` 的说明和规格里都没有写这个行数。
- 与清单不一样的地方：
  - SI-9 和 SI-10 是一起做的：测试一起写，先红、绿、故意改坏是同一个测试程序（一次编两件），发布版和带窗口复核也是两件都在里面的那一份。门禁和提交是分开的：SI-9 过门禁、提交的时候，SI-10 动的三个文件先放回上一个提交的样子（改好的那一份拷在仓库外面，核过校验和，提交之后再放回来），所以这个提交的门禁读数是只有 SI-9 的代码的。
  - 先红跑了两次：第一次 03:23 起，编到一半（03:29）机器被 Windows 更新重启，没有结果；07:10 重新排队，07:14 起跑完。
  - 「只有 SI-9 的代码」的发布版没有另编：带窗口复核用的程序里也有 SI-10 的改动。
  - 带窗口对照用的程序不是上一个提交（SI-8）编的，是 SI-6 的提交编的：这两个提交之间 `src` 下只多了一个只在测试里编的文件和挂上它的两行。
- 没验证的：
  - 耗时预算（规格 §6 第 12 条）：这个任务没有动导入这条路。等空闲的脚本 07:09 重新起了（机器 03:29 重启过），到这个提交为止只在 07:26 跑成一轮，而且是「平静」不是「空闲」（跑之前整机占用 38 %、41 %、32 %，没有编译进程；空闲的标准是不超过 20 %）：中位数 3,539 毫秒（3,268、3,539、3,818），预算 3,747。它跑的是 SI-6 的提交编的测试程序。只有一轮，机器又不空闲，不算验证；规格 §11 的 SI-7 第 5 条要的是空闲时轮流各跑十轮以上。
  - 滚轮和拖动平移在带窗口的程序里没有试（不动真实的鼠标）：带窗口试的是 `ZOOM E`、`ZOOM W`、`PIDTAG`；滚轮只有单元测试里发消息的那一路；拖动平移没有测试，它和滚轮一样只是让相机动了一下（读代码得来的）。
  - 关闭时弹出的那个提问窗口本身没有看：单元测试看的是程序有没有记下「等着问保存」，带窗口看的是不带丢弃的关闭请求有没有被拒绝。
  - 另存之后再动视图，带窗口没有试；路径是 `.dwg` 的那一支在单元测试里是 V3，带窗口是「存得下视图的图纸照旧」那一段。
  - DXF 没有单独测：规则看的是「是不是只读来源」，DXF 和 DWG 走的是同一支。
  - 关整个程序（主窗口）时对几张图纸一起的提问没有试；它用的是同一个「已修改」标记（读代码得来的）。
  - 门禁没有在「没有别的会话同时在这个编译目录里编」的条件下跑成（见上面「门禁旁边有别的会话在编」）。
- 证据：`D:\scratch\ocs-symblock\si-9-10\`（先红：`libmut-red.txt`、`libmut-red-none.log`，被重启打断的那一次 `libmut-red-interrupted.txt`；绿和改坏：`libmut-green.txt`、`libmut-green-*.log`；排队和监视：`quietred.txt`、`quietgreen.txt`；发布版的排队和监视 `quietrelease.txt`；读数的摘记 `results.md`）、`D:\scratch\ocs-symblock\si-9\`（改法 `draft.md`；固定门禁 `chainfinal.txt` 和它列的各份记录，排队和监视 `quietfinal.txt`；带窗口复核的脚本 `recheck.py`，输出 `recheck-both.log`，对照用的那一遍 `recheck-old.log`）、`D:\scratch\ocs-symblock\si-7\`（发布版的记录 `build-both.done`、`build-both.log`；复核的逐步记录和截图 `out\recheck-both\`，对照的 `out\recheck-old\`）。

### SI-10 `PIDTAG` 找得到带空格的位号 【接线】

- **做什么**：规格 §11 的 SI-7 第 4 条：数据库给的位号可以带空格（53 张的 5,027 个里有 1 个，D07 的 `LG-Magnetic Float Gauge-0203`），`PIDTAG` 按空白分词，这样的位号敲不进去。改成：`PIDTAG` 后面的整串恰好是本图上某个导入图例的位号时，不管几个词都算查找（位号的大小写不计，词之间的空白不计，外面带不带引号都行）；不是的时候照旧，两个词是改位号，更多的词报错。只管导入的图例。两个词既是某个导入图例的位号、又能读成改位号时，算查找（不动数据的那个读法）。只读的协同会话按同样的判断放行这种查找。动词的写法不改（脚本和控制通道送来的一行里动词照旧要大写）。在窗口里敲的时候，这一行要以 `>` 开头（`>PIDTAG LG-Magnetic Float Gauge-0203`）：命令行里空格平时等于回车，`>` 开头的一行才把空格当字符收下；脚本、控制通道、粘贴进来的一行不用。只敲 `PIDTAG` 回车之后的那个提示照旧只收一个词，不改。
- **验收**：
  - [x] `PIDTAG LG-Magnetic Float Gauge-0203` 一行送到，选中这个图例和它的标注；没有错误，没有留下等输入的提示，什么都没有写。
  - [x] 带引号的写法同样找得到；词之间敲几个空格都一样。
  - [x] 整串不是导入图例的位号时：两个词仍然是改位号，更多的词报错并说明两种读法都不成立。
  - [x] 两个词恰好是导入图例的位号时算查找，没有改任何位号。
  - [x] 只读会话里这种查找不被拦，改位号照旧被拦。
  - [x] 在窗口里敲的那一路（`>` 开头的一行）找得到；只敲 `PIDTAG` 之后的提示照旧只收一个词。两处都有测试钉着。
  - [x] 先红后绿、故意改坏、十条固定门禁。
  - [x] 带窗口复核：D07 从备份面板打开，`PIDTAG LG-Magnetic Float Gauge-0203` 找到那个图例（做不了就写明）。
- **依赖**：SI-8。
- **文件**：`src/app/commands/pidlegend.rs`、`src/app/command_driver/utilities.rs`、`src/app/collab/apply.rs`、规格、本清单。
- **规模**：M。

**做完**（2026-11-21）

- 结论：做了。`PIDTAG` 后面的整串恰好是本图上某个导入图例的位号时，不管几个词都算查找：`PIDTAG LG-Magnetic Float Gauge-0203` 一行送到，就选中那个图例和它的标注。位号的大小写不计，词之间的空白不计，外面带不带引号都行。不是这样的位号时照旧：两个词改位号，更多的词报错。只读会话按同样的判断放行。在窗口里敲的时候这一行要以 `>` 开头；只敲 `PIDTAG` 回车之后的那个提示照旧只收一个词，带空格的位号在那里敲不进去（定下来不改的）。耗时预算（规格 §6 第 12 条）仍然没有在空闲的机器上验证。
- 改了什么（产品代码 +71 −11，三个文件；测试 +382）：
  - `src/app/commands/pidlegend.rs`（产品 +55 −6）：
    - `dispatch_pidtag`：分词之后先问一句「后面的整串是不是本图上导入图例的位号」（两个词以上时才问）。是，就走查找那一路；不是，照旧：两个词改位号，三个词以上报错。报错的句子换了：原来是「PIDTAG: 改位号只接受两个参数（旧位号 新位号），实得 "…"。」，现在是「PIDTAG: 本图上没有位号 "…" 的导入图例；改位号只接受两个参数（旧位号 新位号）。」，两种读法都不成立都说到。查找那一路要找的位号改用 `asked_tag` 算。
    - `pidtag_names_an_imported_tag`（新）：上面那一句问话。命令和只读会话的判断用的是同一个函数，两边对同一行的读法不会不一样。
    - `tag_key`、`asked_tag`（新）：位号怎么比。`tag_key` 把位号转成大写，中间每一段空白算一个空格；`asked_tag` 先去掉首尾空白和外面的一对引号，再交给 `tag_key`。
    - `imported_groups`：导入图例的位号改用 `tag_key` 比（原来只转大写）。`Asked::Tag` 的说明跟着改。
  - `src/app/command_driver/utilities.rs`（+5 −3）：命令行把一行交给命令之前要决定「整行交过去」还是「先交第一个词，别的词一个一个喂」。`PIDTAG` 原来只有恰好三个词（`PIDTAG 旧 新`）整行交；现在三个词以上都整行交。一个词、两个词的不变（两个词是 `PIDTAG <位号>`，照旧走提示那一路）。
  - `src/app/collab/apply.rs`（产品 +11 −2）：`readonly_refuses_command` 的 `PIDTAG` 一支，原来是「`PIDTAG` 后面超过一个词就拦」，现在是「后面超过一个词，而且整串不是本图上导入图例的位号，才拦」。
  - 测试：`pidlegend.rs` 加 7 条测试、一个常量、一个辅助函数（+319）；`apply.rs` 加 1 条（+63）。原有的测试一行没有动。
  - 规格：开头的状态行；§4.7 加一条；§7 的测试一览加一条；§11 的 SI-7 第 4 条末尾注明；§11 加 SI-10 一节。
  - 本清单：开头的状态行；SI-10 的验收勾上；这段记录；SI-9 的记录里注明一处、SI-9 的验收补勾一条（见下面「更正」）。
  - 没有动：改位号（`retag_by_tag`）、识别出的符号怎么找、图例清单、提示怎么收词、命令行怎么分词。没有加命令，没有加词条。
- 规则（规格 §4.7 加了一条；怎么取舍、为什么，记在规格 §11 的 SI-10）：
  - 整串怎么算：`PIDTAG` 后面的全部，去掉首尾空白；外面有一对引号的，去掉引号；转成大写，中间每一段空白（空格、制表符，不管几个）算一个空格。图上导入图例的位号同样处理之后和它相等，就是这个位号。所以记录里写着两个空格的位号，敲一个空格也找得到；回执照记录里的写法写。
  - 是位号：这一行是查找。选中带这个位号的导入图例和它们的全部标注，画红框，缩放过去；不写任何东西，不进撤销，不留提示。
  - 不是位号：一个词照旧是查找（找不到报原来那句）；两个词照旧是改位号；三个词以上报上面那句新的错，什么都不选、不写。
  - 两个词既是某个导入图例的位号、又能读成改位号（第一个词恰好是本程序里画的一个对象的位号）：算查找，不改任何位号。图上没有导入图例带这两个词时，同一行是改位号。
  - 只管导入的图例。识别出的符号（DXF、DWG 图纸上图例识别的结果）和本程序里画的对象，位号的比法没有变；带空格的位号在它们身上能不能找、能不能改，这次没有管。
  - 带引号：`PIDTAG "LG-Magnetic Float Gauge-0203"`。经命令行送的，命令行把引号里的当一个词（引号在那里就去掉了），交给 `PIDTAG` 的提示，提示再交给命令；按钮、插件、控制通道直接把整行交给命令的，引号由命令去掉。
  - 只读会话：整串是导入图例的位号时放行（带不带引号、空白多少都一样）；`PIDTAG 旧 新`、不是位号的几个词，照旧拦。这道判断只是先挡一下：真正不让只读会话改图的是每一步改动都要过的那一处（`refuses_step`），没有动。
  - 在窗口里敲：命令行里空格平时等于回车，所以带空格的一行要以 `>` 开头（`>PIDTAG LG-Magnetic Float Gauge-0203`）：`>` 开头的一行把空格当字符收下，`>` 本身不算进去。粘贴进来的一行、脚本、控制通道送来的一行本来就带着空格，不用 `>`。
  - 只敲 `PIDTAG` 回车之后的提示：照旧只收一个词。在那里敲带空格的位号，提示拿到的是第一个词，报「没有这个位号」。这是定下来不改的，有一条测试钉着（S7）。
  - 动词的写法这次没有动：带窗口复核时经控制通道送小写的 `pidtag LG-Magnetic Float Gauge-0203`，程序答的是「未知的命令」，改之前的程序上相同。
- 行为上顺带变了的：
  - `PIDTAG` 后面带三个词以上的一行（连动词四个词以上），原来命令行先开提示、把第一个词当位号去找，剩下的词到不了 `PIDTAG`；现在整行是一个回答：是位号就找到，不是就报错。
  - 三个词以上不是位号时报的句子换了（见上）。
  - 直接交给命令的带引号的位号（一个词的也算，如 `PIDTAG "GV-101"`）现在去掉引号再找，原来连引号一起比、找不到（一个词的那种是读代码得来的，见「没验证的」）。
- 简称（下面用；S1–S7 在 `src/app/commands/pidlegend.rs`，R 在 `src/app/collab/apply.rs`。证据里的 `results.md`、`draft.md` 把这 8 条叫 D1–D7 和 E，这里换了名字，免得和图纸 D07 混）：
  - S1 `pidtag_finds_an_imported_symbol_by_a_tag_that_holds_spaces`
  - S2 `two_words_that_are_the_tag_of_an_imported_symbol_find_it_and_rename_nothing`
  - S3 `pidtag_compares_a_tag_that_holds_spaces_white_space_aside`
  - S4 `pidtag_takes_a_tag_that_holds_spaces_in_quotes`
  - S5 `words_that_are_no_tag_of_an_imported_symbol_are_refused_and_say_why`
  - S6 `a_tag_that_holds_spaces_is_typed_on_a_line_that_keeps_its_spaces`
  - S7 `the_prompt_of_the_bare_verb_takes_one_word_as_it_did`
  - R `a_readonly_session_may_jump_to_an_imported_tag_that_holds_spaces`
- 验收的每一条对哪条测试。用的图是 SI-3、SI-4 那张拼出来的图，把第一个阀门的位号换成要试的那个（照数据库给位号的写法，`tag_source=database`），图纸不是已修改。S1–S5 走命令行（脚本、控制通道送来的一行走的就是它），S6、S7 走窗口里命令行的「输入」「回车」两个消息：

  | 验收 | 测试 | 钉住的 |
  |---|---|---|
  | 一行送到，选中这个图例和它的标注；没有错误，没有留下等输入的提示，什么都没有写 | S1 | `PIDTAG LG-Magnetic Float Gauge-0203`：没有报错；选中阀门和它的 4 条标注；回执 `PIDTAG: Ball Valve LG-Magnetic Float Gauge-0203 -- 1 symbol(s), 4 label(s) selected and framed in red.`；一个红框；视图动了；没有留下提示；命令行里没有剩下没人收的词；阀门的记录一个字没变；没有撤销步；图纸不是已修改 |
  | 带引号的写法同样找得到 | S4 | 经命令行送 `PIDTAG "LG-Magnetic Float Gauge-0203"`：选中的和回执同上，提示没有留下；直接交给命令的 `PIDTAG "lg-magnetic float gauge-0203"`（小写）：同样 |
  | 词之间敲几个空格都一样 | S3 | 记录里的位号在第一个词后面是两个空格。敲一个空格的、两个空格的、小写而且中间是三个空格和一个制表符、末尾还带空格的，都选中阀门和 4 条标注；回执照记录里的写法（两个空格） |
  | 整串不是导入图例的位号时：更多的词报错并说明两种读法都不成立 | S5 | 四个不是位号的：编号差一位的、位号后面多一个词的、少了第一个词开头一截的、少了最后一截的。都是那句新的错，句子里写着敲的整串；原来选中的东西不动；不画框；不留提示；阀门的记录没变；没有撤销步 |
  | 整串不是导入图例的位号时：两个词仍然是改位号 | S5、S2 | S5 的最后：`PIDTAG LG-Magnetic Float`（位号的前两个词）是改位号，图上没有位号 `LG-Magnetic` 的符号，照原来报 `tag_absent` 和原来那句。S2 的后半见下一行 |
  | 两个词恰好是导入图例的位号时算查找，没有改任何位号 | S2 | 阀门的位号是 `P-1001 P-1009`，图上另画一个位号 `P-1001` 的对象。`PIDTAG p-1001 P-1009`：选中阀门和 4 条标注，回执是查找的回执；画的那个对象位号还是 `P-1001`（`auto`）；阀门的记录没变；没有撤销步。把阀门的位号换成 `GV-101` 之后同一行：画的那个对象改成 `P-1009`（`manual`），回执是改位号的回执，一个撤销步 |
  | 只读会话里这种查找不被拦，改位号照旧被拦 | R | 只读会话里三种写法（照原样、小写加两个空格、带引号）都不被拦，都找到阀门，没有撤销步，图元一个没多没少；`PIDTAG GV-101 GV-109`、位号的前两个词、编号差一位的（不带引号、带引号）都被拦 |
  | 在窗口里敲的那一路（`>` 开头的一行）找得到 | S6 | 往命令行输入 `>PIDTAG LG-Magnetic Float Gauge-0203` 再回车，和不带 `>` 的整行（粘贴进来的样子）再回车：都没有报错，选中阀门和 4 条标注，回执同 S1，没有留下提示，图纸不是已修改 |
  | 只敲 `PIDTAG` 之后的提示照旧只收一个词 | S7 | 输入 `PIDTAG` 回车，提示开着；再输入 `>LG-Magnetic Float Gauge-0203` 回车：报 `PIDTAG: no symbol tagged "LG-Magnetic" on this sheet -- PIDLEGEND LIST shows the tags.`，什么都没选中，提示关了（它把第一个词当成了回答） |

- 先红（07:14–07:20，`libmut-red.txt`；两件的测试都在树里、实现都没有写）：筛出的 31 条里 22 过 9 红。SI-10 的 8 条里红 7 条（S1–S6 和 R）；S7 是过的，它钉的是不改的那一处，前后都该过。各停在哪：S1、S6 停在「nothing is refused」，报的是 `PIDTAG: no symbol tagged "LG-Magnetic" on this sheet`（命令行把第一个词当成了位号）；S2、S3、S4 停在「什么都没选中」；S5 停在报错的句子上（得到的是那句 `no symbol tagged`，不是新的那句）；R 停在第一种写法（被拦了）。筛到的另外 18 条原有的测试都过（是哪些、旁边有没有别的会话在编，见 SI-9 的记录）。
- 故意改坏（07:23–07:26，`libmut-green.txt`；7 处，开关在三个文件里，用环境变量开，跑完已经拿掉；和 SI-9 的 5 处编在同一个测试程序里）：

  | 改坏 | 改的是什么 | 结果 |
  |---|---|---|
  | （不改） | | 31 条全过 |
  | `si10-never` | 「整串是不是位号」永远答不是 | 红 S1、S2、S3、S4、S6、R |
  | `si10-always` | 两个词以上一律当查找 | 红 S2（停在后半：两个词又该是改位号的时候）、S5，和原有的 4 条改位号的测试 |
  | `si10-rename` | 两个词即使是位号也当改位号 | 红 S2 |
  | `si10-spaces` | 空白照敲的比，不归成一个空格 | 红 S3（停在敲一个空格的那一种）、R（停在第二种写法） |
  | `si10-quotes` | 引号算位号的一部分 | 红 S4（停在直接交给命令的那一半）、R（停在带引号的写法） |
  | `si10-driver` | 命令行照旧只把恰好三个词的整行交过去 | 红 S1、S3、S5、S6、R |
  | `si10-gate` | 只读会话照旧「后面超过一个词就拦」 | 红 R |

  S7 没有哪一处改坏让它红：它钉的是这次不改的行为，改坏的都是这次加的。别的 7 条各至少红过一次；每一处红的只有表里列的。`si10-always` 红的 4 条原有测试是 `a_rename_of_an_absent_tag_is_refused_by_code`、`a_two_word_pidtag_renames_the_tag_in_one_undo_step`、`a_rename_is_refused_where_imported_symbols_alone_carry_the_tag`、`a_rename_takes_the_objects_drawn_here_and_leaves_the_imported_symbols`。这一遍旁边记了 15 次，没有看到别的会话在编。
- 带窗口复核（07:42–07:45，`recheck-both.log` 的第五段；发布版，两件的改动都在里面；经程序自己的控制通道，窗口开在另建的看不见的桌面上，不动真实的鼠标和键盘。对照是同一个脚本 03:07 在改之前的程序上跑完的那一遍，`recheck-old.log`；那个程序是 SI-6 的提交编的发布版）：
  - D07 从备份面板打开（命令行说有 8 个符号带上了数据库的位号，其中 1 个与图纸文件里的名字不同）：图上 18 个图例，带 `LG-Magnetic Float Gauge-0203` 的 1 个（`class` 是 PIDInstrument，`tag_source` 是 `database`，`label_in_file` 是 `LG-0203`），这张图上带空格的位号只有它；它这一组是 3 个图元（图例和 2 条标注）。
  - 该找到的四行：`PIDTAG LG-Magnetic Float Gauge-0203`（控制通道的两种送法各一次）、`PIDTAG "LG-Magnetic Float Gauge-0203"`、`PIDTAG  lg-magnetic   float   gauge-0203`（小写，词之间三个空格）。每一行送之前先取消选中。四次都恰好选中这一组的 3 个图元，没有报错，没有留下等输入的命令，图纸不是已修改；回执都是 `PIDTAG: LG-Magnetic Float Gauge LG-Magnetic Float Gauge-0203 -- 1 symbol(s), 2 label(s) selected and framed in red.`。第一次之后视图从整张图（距离 975.069）移到这一组的范围里（中心 332.836, 217.262，距离 104.125）；后三次视图已经在那里，没有再动。带引号的那一次，命令行先显示 `PIDTAG` 的提示那一行，再找到。
  - 该报错的五行，都什么也没选中，图纸不是已修改：动词小写的 `pidtag LG-Magnetic Float Gauge-0203`，答「未知的命令」；位号后面多一个词的（`… X`）、编号差一位的（`…-0204`，不带引号和带引号各一行），报的都是那句新的错，句子里是敲的整串；位号的前两个词（`PIDTAG LG-Magnetic Float`）照旧是改位号，报 `PIDTAG: 本图上没有位号 "LG-Magnetic" 的符号，未做改动。`（`tag_absent`）。
  - 九行都送完之后：图上 176 个图元和它们的记录与开始时相同，图纸不是已修改，修订号是 0，不带丢弃的关闭请求通过。
  - 改之前的程序上同样九行：该找到的四行一次也没有找到（选中 0 个）。照原样的两次和小写的一次，命令行把第一个词当成了位号，报 `PIDTAG: no symbol tagged "LG-Magnetic" on this sheet -- PIDLEGEND LIST shows the tags.`（小写的那次是 `"lg-magnetic"`）；带引号的一次报原来那句「改位号只接受两个参数（旧位号 新位号），实得 "LG-Magnetic Float Gauge-0203"」。该报错的五行在改之前的程序上也都报错：动词小写的、前两个词的，说的话和现在相同；另外三行说的是原来的话（多一个词的、编号差一位的报 `no symbol tagged "LG-Magnetic"`，带引号的编号差一位的报「改位号只接受两个参数……实得」）。
  - 桌面没有动，备份文件夹前后相同，输出的文件里带控制通道令牌的 0 个，没有从网络取到东西（和 SI-9 的记录说的是同一遍）。
- 读数（固定门禁在 SI-9 提交之后、两件的代码都在的树上跑了一遍，08:08–08:20，`chainfinal.txt`）：
  - 格式：三个 `.rs` 文件都是 `new=0`（原有的不合格处一处没有多：`src/app/commands/pidlegend.rs` 5 处，`src/app/command_driver/utilities.rs` 3 处，`src/app/collab/apply.rs` 52 处）。
  - 快速筛选 `commands::pidlegend::`：41 过，另有 2 条是平时不跑的（忽略）；`collab::apply::`：40 过。S1–S7 在前一份记录里，R 在后一份记录里，都是 ok。
  - 全量单元测试 7,240 个：通过 7,183，忽略 57，失败 0（上一个提交是 7,232 个、通过 7,175；多的 8 个就是这 8 条，全量的记录里 8 条都是 ok，SI-9 的 5 条也是）。
  - 集成测试 `tests/pid_import.rs` 66 个、`tests/pid_legend.rs` 31 个、`tests/pid_panel_localization.rs` 2 个全过，个数与上一个提交相同。
  - clippy `--lib` 656 条、`--lib --profile test` 1,051 条，与上一个提交相同。两份记录里没有一行指到这个任务的三个文件，上一个提交的两份记录里也没有。两份记录各自和上一个提交的那一份比，不计行的先后，只有写耗时的那一行（`Finished …`）不同。
  - 分层报告与上一个提交逐行相同。
  - 文档检查没有跑：没有动 `docs/pid-platform/`。
- 门禁旁边有别的会话在编（和 SI-8、SI-9 一样，排了队也没有躲开；`quietfinal.txt`）：07:58:10 排上，连续三次看不到别的会话的 cargo 才开始（等了 596 秒，08:08:06 开始，08:20:22 跑完）。跑的时候旁边记了 72 次，每一次都看到别的 cargo（1 到 3 个），其中 71 次它们名下有编译或链接的进程在跑，12 次有编译进程的命令行指着这个共用的编译目录（08:10:40 到 08:19:00；最多的一次是 08:17:07，29 个）。先后看到 11 个 cargo：带 `--locked` 的 `cargo test --lib …` 四次，`cargo run --example pid_convert` 一次，`cargo clippy --lib` 和 `cargo clippy --lib --profile test` 各一次（各带着它起的一个 `cargo check`），不带 `--locked` 的 `cargo test` 两次（`-- room`、`--test increment_cases_registry`）。其中 `-- room` 那一个 08:06:13 就起了：排队的脚本最后三次都没有把它算进去（按脚本的规则，是认出它编在别的目录里），旁边监视的是另起的进程，没有这份记性，照算了；08:08:06 到 08:08:57 的头 6 次看到的只有它。从 08:09:08 起别的那些才一个接一个地来。没有动别人的进程。各步都是一遍就过，结果按各步自己的记录认定是这棵树的：
  - 快速筛选和全量单元测试（08:08:06–08:11:33）：三步跑的是同一个测试程序（08:09:55 写出）。这 8 条只有这棵树才有，在快速筛选和全量的记录里都是 ok；总数 7,240 个，只有 SI-9 的 5 条和这 8 条都在的这棵树才有。门禁跑完之后看了这个程序（只读，`programs-final.txt`）：里面有这次新加的那句报错（两处），没有原来那句。08:10:40 别的会话在同一个编译目录里也写出一个单元测试程序，是另一个文件夹、另一个文件名，里面是原来那句、没有新的：这一遍里它没有盖掉这棵树的。
  - 集成测试（08:11:33–08:16:02）：先改了 `src\lib.rs` 的修改时间，记录里写着重新编的是本工作树的包（编了 4 分 12 秒）。这一步最后写出的主程序（08:15:45）里有新的那句报错、没有原来那句，它用的库是这棵树的；三个集成测试程序是前一秒（08:15:44）写出的。这三个程序里 `PIDTAG` 的报错一句都没有（新的、原来的、这次没改的都没有），单看它们自己分不出是哪棵树的，靠的是前面两样。
  - clippy 两步（08:16:02–08:20:11）：记录里写着编的、检查的是本工作树的包；条数是 clippy 检查这些源码时打印的警告数，旁边的编译能让这一步失败，改不了条数。这两步跑的时候，别的会话也在这个编译目录里跑同样的两步（08:16:56、08:18:57 起）。
- 更正（SI-9 的提交 `de7622ffe`）：那个提交的说明和 SI-9 的记录都写着本清单改了「开头的状态行；SI-9 的验收勾上」。实际上那个提交里漏了两处：清单开头的状态行没有改（仍然写着 SI-9 还没有做）；SI-9 的验收七条里「先红后绿、故意改坏、十条固定门禁」一条没有勾上（另外六条勾了）。那一条说的事是做了的，读数就在 SI-9 的记录里；漏的只是这两处文字。这个提交里补上了，SI-9 的记录里那一句也注明了。提交不改写。规格开头的状态行在 SI-9 的提交里是改了的。
- 与清单不一样的地方：
  - 和 SI-9 一起做的（见 SI-9 的记录）：先红、绿、故意改坏、发布版、带窗口复核都是两件在一起的那一份；门禁是在 SI-9 提交之后、两件的代码都在的树上跑的。这三个文件在 SI-9 过门禁和提交的时候放在仓库外面，放回来之后核过校验和，和放出去时相同。
  - S7 是做的时候加的界线测试，前后都过，没有对应的改坏。
  - 带窗口对照用的程序是 SI-6 的提交编的，不是上一个提交编的（同 SI-9 的记录）。
- 没验证的：
  - 耗时预算（规格 §6 第 12 条）：这个任务没有动导入这条路。等空闲的脚本（07:09 起）到这个提交为止仍然只有 07:26 那一轮，而且是「平静」不是「空闲」（中位数 3,539 毫秒，预算 3,747；见 SI-9 的记录）。之后一轮也没有跑成：07:30 到 08:21 每十分钟一段，整机占用的中间值是 90 %、45 %、20 %、38 %、48 %，各段里编译进程最多的时候是 33、14、13、7、38 个。只有一轮，机器又不空闲，不算验证；规格 §11 的 SI-7 第 5 条要的是空闲时轮流各跑十轮以上。
  - 门禁没有在「没有别的会话同时在这个编译目录里编」的条件下跑成（见上面「门禁旁边有别的会话在编」）。
  - 在真的键盘上敲没有试：窗口里敲的那一路是不开窗口的测试里发「输入」「回车」两个消息（S6、S7）；带窗口复核经控制通道送的是整行。「空格等于回车」「`>` 开头把空格当字符」是命令行原有的行为，这次没有动，也没有在真的窗口里按键去看。
  - 直接交给命令的带引号的一个词（`PIDTAG "GV-101"`）没有单独测，是读代码得来的。
  - 识别出的符号（DXF、DWG 图纸上）带空格的位号：不在范围里，没有测。图上同时有带这个位号的导入图例和识别出的符号时怎么选，也没有测。
  - 改位号改成、或者改掉一个带空格的位号：不在范围里（两个词就是旧、新）。
  - 53 张里带空格的位号只有 D07 的这一个，带窗口也只在这一张图上试了；别的样子（中间连着两个空格的、两个词的）只在拼出来的图上测过。
  - 只读会话带窗口没有试，只有不开窗口的测试（R）。
- 证据：`D:\scratch\ocs-symblock\si-9-10\`（先红、绿和改坏、排队和监视，文件名同 SI-9 的「证据」；读数的摘记 `results.md`）、`D:\scratch\ocs-symblock\si-10\`（改法和改坏表 `draft.md`；固定门禁 `chainfinal.txt` 和它列的各份记录，排队和监视 `quietfinal.txt`；门禁那一段时间编译目录里写出的各个程序是哪棵树的 `programs-final.txt`，看它们的脚本 `which-programs.ps1`；过门禁的三个文件放在仓库外面时的那一份 `final\`）、`D:\scratch\ocs-symblock\si-9\`（带窗口复核的脚本 `recheck.py`，输出 `recheck-both.log`，对照用的那一遍 `recheck-old.log`）、`D:\scratch\ocs-symblock\si-7\out\recheck-both\`（复核的逐步记录 `recheck.json` 和截图）。

### SI-11 在空闲的机器上量耗时预算 【验收】【文档】

- **做什么**：规格 §6 第 12 条（耗时预算）从 SI-2 起一直没有在空闲的机器上验证；机器空下来之后怎么量，SI-7 写下了（规格 §11 的 SI-7 第 5 条：两个测试程序轮流各跑十轮以上，这个提交的中位数不超过 3,747 才算过，超过的话从 SI-1 起逐个提交轮流量）。用户 2026-11-21 08:27 的答复仍然是按推荐的做、没有明确推荐的选最合适的并说明理由；这一处没有现成的推荐，选的是接着量，不就此记成「没有验证」：它是规格 §6 的 14 条里唯一还欠的一条，量的时候只读、不碰别人的东西。量现在这棵树的测试程序，和图块分支的测试程序轮流；只量、只改文档，代码不动。
- **验收**：
  - [x] 量的是最后一个改代码的提交（SI-10）编的测试程序，不是 SI-6 那次编的；预算测试不改、不放宽。
  - [x] 每个程序跑之前、跑之后机器在做什么都记下来，空闲的轮次和不空闲的分开算。
  - [x] 过还是不过照 SI-7 第 5 条写的标准说；和那条标准对不上的地方写明。
- **依赖**：SI-10。
- **文件**：规格、本清单。
- **规模**：S。

**做完**（2026-11-21）

- 结论：**规格 §6 第 12 条过了。** 机器在 08:26 空了下来，08:33 前后别的会话又开始编。SI-10 的提交（`44d1baa9f`）编的测试程序和图块分支的测试程序轮流跑了 20 轮（08:30–08:39）：这个提交 20 轮都不超过 3,747 毫秒；空闲的 8 轮中位数 2,898（2,792–3,016），不空闲的 12 轮 3,065.5（2,876–3,567）。和图块分支比，逐轮比值的中位在空闲的 8 轮里是 1.0245，全部 20 轮里是 1.0295：慢 2 %–3 %，预算留给这份计划的余量是 7.4 %。空闲的轮次是 8 轮，不到 SI-7 第 5 条写的十轮；为什么仍然算过，见「与清单不一样的地方」。代码一行没有动。（同一天 09:03–09:10 补量了 10 轮空闲的：十轮补齐了，仍然过；「慢 2 %–3 %」只是这一次的读数，两次量出来的不一样。见后面的「补量」。）
- 改了什么（只有两份文档）：
  - 规格：开头的状态行；§6 第 12 条末尾注明；§11 的 SI-7 第 5 条末尾注明；§11 加 SI-11 一节。
  - 本清单：开头的状态行；SI-7 那张表第 12 行的末格注明；这个任务的定义和这段记录。
- 怎么量的：
  - 这个提交的程序：`si-10\libtest-head.exe`。它是 SI-10 过门禁时在编译目录里编出来、全量单元测试那一步跑的那个单元测试程序（08:09:55 写出，306,530,304 字节），原样拷出来的，两边的 sha256 相同（`14bd52bd…baad8cb`）；它带 7,240 条测试（跑预算测试时打印「7239 filtered out」），就是 SI-10 门禁的个数。08:09:55 之后 `src` 下面只有 `src\lib.rs` 的修改时间动过（门禁为了重编集成测试改的，内容没有变），提交之后工作树是干净的，所以它是提交 `44d1baa9f` 的代码编的。
  - 图块分支的程序：`sb9\lib-final.exe`（2026-11-20 09:35:14 编的，305,354,752 字节），SI-7 轮流量时用的同一个。它的产品代码是图块分支 `08d340055` 的（之后的 SB-9 `466fb083f` 只加测试），也就是预算测试的注释里记着空闲时 3,488 毫秒的那一份。
  - 测试：两个程序里是同一条 `io::pid::sample_budget::the_sample_imports_within_its_budget`，带 `--ignored --exact --nocapture` 单独跑。它从 SB-9 加进来之后没有改过：从这份计划的起点 `1855aaae5` 到现在，`src/io/pid/sample_budget.rs` 的差别是 0 行。跑一次导入样例 3 次、取中位数，每次导入都核对 592 个引用、119 个定义；中位数超过 3,747 它自己就失败。
  - 轮流：用 SI-7 的脚本 `si-7\budget-ab.ps1`，没有改。单数轮先图块分支、后这个提交，双数轮反过来；每个程序跑之前、跑之后各读两个一秒的整机处理器占用，数一遍编译进程（cargo、rustc、link、clippy-driver、rustdoc）。跑了两组：第一组 12 轮（08:30:09–08:35:16），第二组 8 轮（08:35:57–08:39:31）。什么都不编译，只写仓库外面的记录。
  - 空闲不空闲（`si-10\budget-ab-classes.ps1`，只读上面两份记录）：一轮有四次读数（两个程序各自跑之前、跑之后）。四次都没有编译进程、每次读数的平均不超过 20 %，算空闲：这是等空闲的脚本一直用的口径（那里每次读三秒，这里两秒），也就是预算测试的注释里写的「没有编译在跑、整机占用不超过五分之一」。更严的一种算法是八个一秒读数个个不超过 20 %。其余算不空闲。
- 读数：

  | | 轮数 | 图块分支：中位数（最小–最大） | 这个提交：中位数（最小–最大） | 这个提交不超过 3,747 的 | 逐轮的比值（这个提交 ÷ 图块分支）：中位（最小–最大） |
  |---|---|---|---|---|---|
  | 空闲，个个读数不超过 20 % | 6 | 2,831.5（2,753–2,959） | 2,896（2,792–3,003） | 6 轮 | 1.0245（0.985–1.045） |
  | 空闲，每次读数的平均不超过 20 %（含上一行的 6 轮） | 8 | 2,831.5（2,746–3,081） | 2,898（2,792–3,016） | 8 轮 | 1.0245（0.979–1.045） |
  | 不空闲 | 12 | 2,956.5（2,806–3,179） | 3,065.5（2,876–3,567） | 12 轮 | 1.0325（0.967–1.168） |
  | 全部 | 20 | 2,903.5（2,746–3,179） | 2,993.5（2,792–3,567） | 20 轮 | 1.0295（0.967–1.168） |

  - 第一组 12 轮，按轮次：图块分支 2,835、2,799、2,746、2,753、2,828、2,864、2,904、2,828、2,806、3,040、2,903、2,959；这个提交 2,792、2,840、2,851、2,847、2,945、2,994、2,970、2,881、2,876、3,172、3,154、3,003。空闲的是第 1、2、4、5、6、12 轮，和第 3 轮（有一个一秒读数是 23 %，那次读数的平均是 11.5 %）。第 7、9、10、11 轮旁边有 1–3 个编译进程（一秒读数最高 18 %）；第 8 轮没有编译进程，但有一个一秒读数是 70 %。
  - 第二组 8 轮，按轮次：图块分支 3,081、2,938、2,975、2,990、3,053、3,161、3,179、2,864；这个提交 3,016、3,056、3,151、2,918、3,567、3,227、3,075、2,993。空闲的只有第 1 轮（有一个一秒读数是 23 %，各次读数的平均最高 17.5 %）。第 2 到 8 轮各次读数的平均最高的到 30 %–65.5 %，编译进程 0–5 个。
  - 40 次运行的退出码都是 0。最慢的一次是第二组第 5 轮的这个提交，3,567 毫秒（那一轮一秒读数最高 48 %，有 2 个编译进程），比预算低 4.8 %。
  - 那些编译进程不是这项工作启动的（这段时间这项工作什么都没有编），没有动它们。
- 等空闲的脚本先跑的 6 轮（`si-7\out\budget-idle.txt`）：它 07:09 起一直在等，08:26:28 第一次读到空闲（整机 1 %，没有编译进程），08:26:31–08:28:01 跑了 6 轮，到了自己的上限，08:28:14 结束。它跑的是 SI-6 的提交编的测试程序（`si-7\libtest-final.exe`，一个程序，不轮流）：中位数 2,921、2,934、2,869、2,926、2,921、3,135 毫秒，6 轮都过，每轮跑完之后机器仍然空闲。量的不是现在的代码（SI-7 到 SI-10 没有动导入这条路的产品代码），只作旁证，没有算进上面的表。
- 这些读数能说的：
  - 第 12 条本身（预算测试不放宽，导入 3 次的中位数不超过 3,747）：过。空闲时这个提交用 2.8 到 3.0 秒，8 轮的中位数比预算低 22.7 %，最慢的一轮低 19.5 %。
  - 这份计划让导入慢了多少：空闲的 8 轮里逐轮比值的中位是 1.0245（8 轮里 6 轮是这个提交慢），两边中位数之比 1.023；全部 20 轮逐轮比值的中位 1.0295（20 轮里 16 轮是这个提交慢）。慢 2 %–3 %。预算是图块之前的基线加一成，图块分支用掉 2.4 %，留给这份计划 7.4 %。SI-7 在不空闲的机器上估的是「从快 1.5 % 到慢 8.5 %，多数是慢 2 %–6 %」，这次落在那个范围靠下的地方。（补量之后要改着读：补量的 10 轮里逐轮比值的中位是 1.0045，两次合起来 18 轮是 1.015。见「补量」。）
  - 今天空闲时的机器比定预算那天快。同一份图块分支的代码，2026-11-20 定预算时空闲的十二轮中位数是 3,488 毫秒（3,332–3,862），今天空闲的 8 轮是 2,831.5（2,746–3,081），少 18.8 %。原因没有查（这台机器 03:29 重启过，别的没有比）。所以 3,747 这个绝对数今天比定它的时候容易过；上面的比值是两个程序挨着量的，不受这个影响。拿那天的 3,488 乘今天的比值去推：乘 1.0245 是 3,573，乘空闲 8 轮里最大的比值 1.045 是 3,645，都在预算之内；这两个数是推出来的，不是量出来的。
- 与清单不一样的地方：
  - 这个任务是做的时候立的（同 SI-8）：定义和记录一起写进来，验收一开始就是勾上的。
  - **空闲的轮次不到十轮。** SI-7 第 5 条写的是机器空下来之后两个测试程序轮流各跑十轮以上。机器只空了六分半钟左右（08:26:28 到 08:33 前后）：头三分四十秒里，等空闲的脚本在跑它那 6 轮（跑的是 SI-6 的程序），这边在核对量法、把现在的测试程序拷出来；08:30:09 才开始轮流量。轮流的 20 轮里空闲的是 8 轮（按更严的算法 6 轮），其余 12 轮旁边有别的会话在编。没有再等下一次空闲去补够十轮。理由：十轮是结论可能就压在预算线上的时候定的（SI-7 当时推出来的范围是 3,437 到 3,784，跨在线的两边）；现在量到的离线很远，空闲的 8 轮里最慢的一轮比预算低 19.5 %，不空闲的 12 轮里最慢的也低 4.8 %，而旁边有编译只会让导入变慢；再补两轮空闲的，改变不了结论。这是做的人的判断，不是那一条字面上的标准。（后来补了：同一天 09:03–09:10 量到 10 轮空闲的。见「补量」。）
  - 量的程序不是为这次重新编的：是 SI-10 过门禁时编出来的那个单元测试程序的原样拷贝（见「怎么量的」）。
  - 十条固定门禁没有重跑：这个提交只动两份文档，代码和 SI-10 的提交相同，按只动文档的提交的先例（SI-7）。文档检查也没有跑：没有动 `docs/pid-platform/`。
- 没验证的：
  - 「空闲时轮流各跑十轮以上」字面上没有做到（见上）。（后来补量做到了。见「补量」。）
  - 今天空闲时的机器为什么比 2026-11-20 快 18.8 %，没有查；预算的绝对数是按那天的机器定的。
  - 这份计划慢的 2 %–3 % 慢在哪一步，没有逐个提交去量（SI-7 第 5 条写的是超过预算才量）。（补量之后：慢多少本身定不下来。见「补量」。）
  - 量的是测试程序，和定预算时一样的编法，不是发布版。发布版只有 SI-7 在不空闲的机器上量的两组：一组没有看出差别，一组机器起伏太大。
  - 机器的读数只在每个程序跑之前、跑之后各读两秒；程序跑着的那十来秒里机器在做什么，没有记。
  - 等空闲的脚本那 6 轮跑的是 SI-6 的程序，不是现在的代码。
- 证据：`D:\scratch\ocs-symblock\si-10\out\`（两组的记录 `budget-ab-head.txt`、`budget-ab-head-2.txt`，每次运行的输出 `budget-ab-head-*.log`，分类 `budget-ab-classes.txt`）、`D:\scratch\ocs-symblock\si-10\`（量的那个测试程序 `libtest-head.exe`，分类的脚本 `budget-ab-classes.ps1`）、`D:\scratch\ocs-symblock\si-7\`（轮流量的脚本 `budget-ab.ps1`，等空闲的脚本 `idle-watch.ps1` 和它的记录 `out\budget-idle.txt`，SI-6 的测试程序 `libtest-final.exe`）、`D:\scratch\ocs-symblock\sb9\lib-final.exe`（图块分支的测试程序）。

**补量**（2026-11-21，09:03–09:10。这一段是后加的；上面「做完」的各条保持当时写的样子，只在五处句末注了指到这里的话）

- 为什么补：「做完」里「没验证的」第一条（「空闲时轮流各跑十轮以上」字面上没有做到）是这个任务里唯一还补得上的一条。用户 2026-11-21 08:44 的答复仍然是那一句（按推荐的做；没有明确推荐的选最合适的并说明理由），这一处没有现成的推荐，选的是在后台等下一次空闲把十轮补齐，不停在「凭判断算过」上：补量只读、不编译、不碰别人的东西；机器不空就等着，等不到结论也不变。
- 结论：**十轮补齐了，第 12 条仍然过；「比图块分支慢 2 %–3 %」定不下来。** 机器 09:03 又空了下来，同一对测试程序轮流跑了 12 轮（09:03:11–09:10:06），其中 10 轮从头到尾空闲。这 10 轮里这个提交的中位数是 2,867.5 毫秒（2,777–2,961），10 轮都不超过 3,747，最慢的一轮比预算低 21.0 %：SI-7 第 5 条的标准（空闲时轮流各跑十轮以上，这个提交的中位数不超过 3,747）单凭这 10 轮就满足了。和图块分支比，这 10 轮逐轮比值的中位是 1.0045，早上那 8 轮是 1.0245，程序和量法都一样；两次合起来空闲的 18 轮是 1.015。代码一行没有动。
- 改了什么（只有两份文档）：
  - 规格：开头的状态行；§6 第 12 条、§11 的 SI-7 第 5 条、SI-11 第 2、3、4 条句末注明；SI-11 加第 6 条。
  - 本清单：开头的状态行；SI-7 那张表第 12 行的末格；SI-11「做完」里五处句末注明（结论、「这份计划让导入慢了多少」、「空闲的轮次不到十轮」、「没验证的」第一条和第三条）；加这一段。
  - 十条固定门禁没有重跑，文档检查没有跑，理由同上：只动这两份文档，没有动 `docs/pid-platform/`。
- 怎么量的：
  - 程序和测试同上：图块分支 `sb9\lib-final.exe`（sha256 `5bff22f2…236acb5`），这个提交 `si-10\libtest-head.exe`（sha256 仍是 `14bd52bd…baad8cb`）；同一条预算测试，同样的参数，每次运行导入样例 3 次取中位数。SI-11 的提交（`0aaf97fee`）只动文档，代码还是 SI-10 的。什么都不编译，只写仓库外面的记录。
  - 脚本是新写的 `si-11\idle-ab.ps1`，做的是 SI-7 那两个脚本合起来的事：等到空闲才跑一轮，一轮里两个程序轮流（单数轮先图块分支，双数轮先这个提交）。空闲的口径没有变：没有编译进程（cargo、rustc、link、clippy-driver、rustdoc），没有这项工作自己的程序在跑，整机处理器占用三个一秒读数的平均不超过 20 %，连着两次都是。每个程序跑完再读一次（三秒）；两个程序跑完之后机器都还空闲，这一轮才算数。算数的满 10 轮它自己结束。
  - 正式起之前试了三次脚本（`out\try.txt`、`try-loop.txt`、`try-wait.txt`）：单跑一轮；把占用的上限放宽、不看编译进程地连跑两轮，看循环和收尾（这样跑的轮次在记录里标成 test，不标 idle）；照正式的口径等一分多钟。08:46:51 起过一次正式的，一轮没跑就停了：空闲的判断拿四舍五入之后的平均去比 20，20.33 也会算空闲；改成拿原数比，08:54:14 重新起（那一次设的是算数的满 6 轮结束，重新起时改成满 10 轮）。正式记录开头的三行是那一次留下的。
  - 分类用 `si-11\classes.ps1`（只读记录）：口径和早上的 `si-10\budget-ab-classes.ps1` 一样（四次读数都没有编译进程、每次读数的平均不超过 20 % 算空闲；更严的是每个一秒读数都不超过 20 %）。它和量的脚本自己记的「算数、不算数」逐轮对过，12 轮没有一轮不一致；早上的 20 轮它照着早上的分类表合计，得数和上面的表相同。
- 读数：

  | | 轮数 | 图块分支：中位数（最小–最大） | 这个提交：中位数（最小–最大） | 这个提交不超过 3,747 的 | 逐轮的比值（这个提交 ÷ 图块分支）：中位（最小–最大） |
  |---|---|---|---|---|---|
  | 空闲（10 轮都是个个读数不超过 20 %） | 10 | 2,859（2,760–2,904） | 2,867.5（2,777–2,961） | 10 轮 | 1.0045（0.972–1.045） |
  | 不算数（有一个程序跑完之后机器不再空闲） | 2 | 2,790.5（2,749–2,832） | 2,831（2,811–2,851） | 2 轮 | 1.015（0.993–1.037） |
  | 全部 | 12 | 2,852.5（2,749–2,904） | 2,853.5（2,777–2,961） | 12 轮 | 1.0045（0.972–1.045） |

  - 12 轮，按轮次：图块分支 2,832、2,901、2,850、2,876、2,863、2,749、2,760、2,878、2,855、2,904、2,802、2,767；这个提交 2,811、2,845、2,777、2,856、2,784、2,851、2,883、2,879、2,918、2,961、2,888、2,793；比值 0.993、0.981、0.974、0.993、0.972、1.037、1.045、1.000、1.022、1.020、1.031、1.009。
  - 算数的是第 2、3、4、5、7、8、9、10、11、12 轮。每轮四次读数（跑之前两次，两个程序各自跑完之后一次）：这 10 轮里一秒读数最高 12 %，一次读数的平均最高 8.33 %，没有编译进程。
  - 不算数的 2 轮：第 1 轮（09:03:11）第二个程序跑完之后的读数是 4、20、43 %（平均 22.33 %）；第 6 轮（09:06:47）第一个程序跑完之后是 23、20、20 %（平均 21 %）。两轮都没有编译进程。
  - 比值的先后：算数的 10 轮里，前 4 轮（09:04–09:06）都是这个提交快（0.972–0.993），后 6 轮（09:07–09:10）是这个提交慢或者相等（1.000–1.045）。第 8 轮两边只差 1 毫秒（2,878 对 2,879），按三位小数算作相等。
  - 等的那一段：脚本 08:54:14 起，第一轮 09:03:11 才开始。头十分钟（08:54–09:04）28 次读数，整机占用 5.33 %–96.67 %（中间值 32 %），编译进程 0–6 个。那些进程不是这项工作启动的，没有动它们。
  - 24 次运行的退出码都是 0。
- 两次合起来（早上 08:30–08:39 的 20 轮，补量的 12 轮）：

  | | 轮数 | 图块分支：中位数（最小–最大） | 这个提交：中位数（最小–最大） | 这个提交不超过 3,747 的 | 逐轮的比值：中位（最小–最大） | 这个提交慢的轮数 |
  |---|---|---|---|---|---|---|
  | 空闲，个个读数不超过 20 % | 16（6 + 10） | 2,852.5（2,753–2,959） | 2,867.5（2,777–3,003） | 16 轮 | 1.015（0.972–1.045） | 10 |
  | 空闲，每次读数的平均不超过 20 %（含上一行） | 18（8 + 10） | 2,852.5（2,746–3,081） | 2,867.5（2,777–3,016） | 18 轮 | 1.015（0.972–1.045） | 11 |
  | 全部 | 32（20 + 12） | 2,864（2,746–3,179） | 2,903（2,777–3,567） | 32 轮 | 1.0215（0.967–1.168） | 22 |

  - 64 次运行的退出码都是 0。两边中位数之比：补量的 10 轮 1.003，两次合起来空闲的 18 轮 1.005。
- 这些读数改了什么：
  - 十轮：SI-7 第 5 条字面上的标准满足了。上面「与清单不一样的地方」里「空闲的轮次不到十轮」那一条当时是凭判断算过的；现在不用靠判断，结论没有变。
  - 慢了多少：定不下来。早上的 8 轮逐轮比值的中位是 1.0245（6 轮慢，2 轮快），补量的 10 轮是 1.0045（5 轮慢，4 轮快，1 轮相等），合起来 18 轮是 1.015（11 轮慢，6 轮快，1 轮相等）。两次是同一对程序、同一种轮流的量法，同一天，相隔半小时。单轮的比值两次都在 0.97 到 1.05 之间，这个摆幅比要量的差别大。上面写的「慢 2 %–3 %」只是早上那一次的读数，不是定下来的数：两次的中位一个是慢 2.45 %，一个是慢 0.45 %。预算留给这份计划的余量是 7.4 %，两个数都在它以内。
  - 机器：图块分支在补量的 10 轮里中位数是 2,859，比 2026-11-20 定预算时的 3,488 少 18.0 %，和早上看到的一样（少 18.8 %）；原因仍然没有查。
  - 推出来的数：拿 3,488 乘两次合起来的中位 1.015 是 3,540；乘两次空闲的轮次里最大的比值 1.045 仍然是 3,645。都在预算之内，仍然是推出来的，不是量出来的。
- 没验证的：
  - 两次量出来的比值为什么不一样，没有查；没有量第三次。
  - 慢的那一点慢在哪一步，仍然没有逐个提交去量：现在连慢多少都定不下来。
  - 量的仍然是测试程序，不是发布版。机器的读数仍然只在每个程序跑之前、跑之后读（这次每次三秒），程序跑着的那十来秒里机器在做什么，没有记。
  - 试脚本时照正式的口径等的那一次（`try-wait.txt`）顺带量到一轮空闲的（08:51:47 起，图块分支 2,946，这个提交 2,932，比值 0.995，两个程序跑完之后机器都还空闲）。它是试脚本的记录，没有算进上面的数。
- 证据：`D:\scratch\ocs-symblock\si-11\`（量的脚本 `idle-ab.ps1`，分类的脚本 `classes.ps1`）、`D:\scratch\ocs-symblock\si-11\out\`（正式记录 `budget-idle-ab.txt`，每次运行的输出 `budget-idle-ab-*.log`，分类和两次的合计 `classes.txt`；试脚本的记录 `try.txt`、`try-loop.txt`、`try-wait.txt`、`classes-try.txt`）。两个测试程序同上。

**通读**（2026-11-21，09:01 起，后台补量跑着的时候读的。这一段是后加的，不是一个任务：代码只读、没有动，改的只有两份文档）

- 为什么读：SI-11 提交之后等后台补量，问了用户这段时间做什么，推荐的是把这份计划改过的代码再读一遍：只读，不编译，不改代码，看到的连同建议告诉用户。用户 2026-11-21 09:01 答的是那一句（按推荐的做；没有明确推荐的选最合适的并说明理由）。
- 读了什么（起点 `1855aaae5` 到 SI-11 的提交 `0aaf97fee`：`src/` 31 个文件 +9,414 −82，`tests/` 2 个文件 +738 −10，`locales/` 23 个文件 +254 −1；文档不在内）：
  - 产品代码逐行读了：新增 2,183 行（含注释和空行），分在 26 个文件里。
  - 测试代码和词条只是略读，没有逐行读：`src/` 里的测试约 7,200 行，`tests/` 738 行，词条 254 行。测试的条数是用 git 数的：加了 103 条，没有删的；新标成平时不跑的 3 条，`src/app/commands/pidbackup/tags.rs`、`src/io/pid/plant_totals.rs`、`src/io/pid_backup_browse/exp_views.rs` 各一条，都要读仓库外面的备份。
  - `src/`、`tests/` 里改动或删掉原有代码的地方一共 44 处（差别里带删除行的块，原有的测试也在内），逐处看了：没有哪条原有的检查被放松。多看了一眼的两处：`src/app/pid_symbol_block_tests.rs` 的 `opened_with` 里「一毫米是四个像素」那条断言还在，只是把 `put_in_front` 拆了出去；`src/io/pid/tests.rs` 里 `counts[18..]` 改成 `counts[18..20]`，是因为后面又接了四个数。
  - 新增的产品代码另外用一个只读的脚本逐行筛了一遍（注释不算）：`unwrap`、`expect`、`panic!`、断言这一类 0 处，调试输出 0 处，`#[allow]` 0 处，`unsafe` 0 处，TODO 一类的备注 0 处，`as u32` 这一类类型转换 0 处；按下标取元素 7 处（`tokens[0]` 一处、`self.tabs[i]` 六处），都是所在文件原有的写法。脚本把每个文件第一个测试模块以后的都当成测试，漏了测试模块在中间的两个文件：`src/app/update/file.rs` 的 12 行、`src/app/update/viewport.rs` 的 14 行。这 26 行是另外读的，没有上面那几类写法；2,183 是脚本数的 2,157 加这 26。
- 结论：**没有看到必须改的。** 看到五处，都不需要现在改代码：一处记进了规格，四处记在下面。
- 记进规格的一处（规格 §11 的 SI-1 第 9 条）：一个图形编号在一张图里只指一个图形，这是没有写出来的前提。
  - 对象表（`src/io/pid/items.rs` 的 `SheetItems`）的键只有图形编号；导入器给图例查对象时（`src/io/pid/build.rs`）只拿编号去查，不看图例在哪个图面流里。标注配对在导入时是按图面流分开对的（`src/io/pid/labels.rs`）。
  - 对象这一头分不了流。核对的是锁定的那个版本的转换库（`96ee00c`，cargo 取下来的那一份，只读）的 `src/publish/pid_load.rs`：读对象的入口只读根存储下的流，`JSite…` 存储里只读 `JProperties`；读到的记录按编号并成一张表；交出来的表示记录只有三样（表示的编号、对象的编号、图形编号），没有图面流。
  - 导入之后记录里留下的也只有编号：`src/io/pid_identity.rs` 找一个图例的标注、找一条标注的属主，都是在模型空间里按编号找。
  - 前提不成立时会出什么错、53 张上为什么不会出、别的装置的图纸要先数什么，写在规格那一条里。
  - 读完汇报时建议的是「代码不改，在文档里写明」，问了用户；用户 09:18 答的仍然是那一句，这一处有明确的建议，照做。
- 别的四处（没有改，记在这里备查）：
  - 换一个数据库再补一次位号，图纸文件自己的名字可能查不到了（`src/io/pid/database_tags.rs` 的 `fill_database_tags`）。第一次补的位号和文件里的名字相同时不留 `label_in_file`；再拿另一个数据库的位号补同一张图，记录里已经没有文件的名字可比，新位号旁边不会有 `label_in_file`。另外，第二个数据库没有给位号的图例留着第一个数据库的位号。程序里走不到：补位号只有一处调用，每张图在从备份面板打开时补一次，用的是打开它的那个备份。建议不改。
  - 从备份面板打开时，只认紧接着的那一次打开（`src/app/commands/pidbackup/tags.rs` 的 `tag_the_drawing_the_backup_opened`）。面板记下「下一次打开的编号」；要是别的打开抢在前面用了这个编号，面板要开的那张图就不补位号，图例上只有图纸文件里的名字，也没有提示。这是有意的，代码里的注释写着只认紧接着的那一次：为的是不把位号补到别的图上。建议不改。
  - `PIDTAG` 找不到时，提示里的位号是照敲的原样放进一对引号里的（`src/app/commands/pidlegend.rs` 的 `dispatch_pidtag`）：位号带着引号敲的时候，提示里引号成了两层。查找本身不受影响（比的时候去掉了引号）。只是外观；建议下次动 `PIDTAG` 时顺手改。
  - 导入比图块分支多做的事，读代码看到三件：为了拿到对象，把图纸文件用转换库的另一个入口又读了一遍（`SheetItems::read`）；用已经解析好的记录建标注表（不另读文件）；给每个图例和每条标注文字写几个键。没有看到哪一步随图元数的平方增长。这是读出来的，没有分段量；和「补量」里「慢多少定不下来」放在一起看。
- 改了什么（只有两份文档）：
  - 规格：开头的状态行；§11 的 SI-1 加第 9 条；SI-2 第 3 条、SI-7 第 12 条句末注明。
  - 本清单：开头的状态行；SI-1「做完」里 `items.rs` 那一条句末注明；加这一段。
  - 十条固定门禁没有重跑，文档检查没有跑，理由同「补量」：只动这两份文档，没有动 `docs/pid-platform/`。
- 没验证的：
  - 测试代码和词条没有逐行读。
  - 前提不成立时会出的错是读代码推出来的，没有拼一份两个图面流同号的数据去试。
  - 别的装置的图纸没有；「拿到那样的图纸要先数」的那一项没有写成检查或测试。
  - 转换库只读了读对象的入口里和这件事有关的几段，没有通读。
  - 别的四处都是读代码看到的，没有一处去试过。
- 证据：`D:\scratch\ocs-symblock\si-11\review\`（逐文件的差别 `<路径>.diff`，筛新增产品代码的脚本 `added-risky.ps1` 和它的输出 `added-risky.txt`，改动原有代码的 44 处 `removed-lines.txt`）。

**合进 main 的演练**（2026-11-21，09:40 起。这一段是后加的，不是一个任务：在另开的本地分支 `pid-symbol-on-main` 上做，这份计划的两条分支和 `main` 都没有动，没有推送；这一段只在那条分支上）

- 为什么做：通读之后把 `main` 试合了一遍（只算，不落地），看到 7 处文字冲突，还看到 `main` 钉的转换库换了版本，会让 3 个缺口符号的数变。问了用户下一步，推荐的是另开一条本地分支把 `main` 合进来走一遍：合并、过门禁、带着备份重数全厂的数、和 `main` 的程序对一次出图。用户 2026-11-21 09:39 答的是那一句。
- 合的是什么：这份计划的头 `18569a5f1`（和 `main` 共同的起点 `5171d197b` 之后 32 个提交，73 个文件，+20,791 −176）和 `main` 当时的头 `4e115a2f8`（2026-11-21 09:35:16；起点之后 54 个提交，110 个文件，+48,588 −419）。两边都改过的文件 13 个：7 个自动合上；6 个有冲突，共 7 处，都是两边在同一处各加了东西，两边都留。合并提交 `16ba6fc27`，逐处怎么解的写在它的说明里。
- 文字上合得上、要编译和跑测试才看得出的两处（也在合并提交里）：
  - `main` 的 RD-13（`69e0e3146`，图框按图纸里记的位置放）给 `overlay_template` 和 `finish` 加了一个参数。这边的测试里有 5 处调用，补上 `FramePlacement::NONE`（不挪），和 `main` 自己的测试写法相同。
  - `main` 把默认主题改成了亮色（`7d545f845`），前景色从白变成黑。`src/app/pid_symbol_view_tests.rs` 里写着颜色的三个常量从 `#FFFFFF` 改成 `#000000`；这些测试查的东西（看不看得见、颜色随不随图层）没有变。
- 转换库：这条分支用的是 `main` 钉的 `a85fa10`。这份计划的两条分支仍然锁 `96ee00c`，本清单 §0 的那一句说的是它们。
- 门禁（合并提交上，离线；`chaing1.txt`）：
  - 程序内测试：7,286 条过，3 条不过，69 条平时不跑。
  - 外部测试三组：`pid_legend` 31 条过，`pid_panel_localization` 2 条过；`pid_import` 66 条里 64 条过，2 条不过。
  - clippy 两遍：`--lib` 656 条、`--lib --profile test` 1,051 条，和这份计划最后一次（SI-10）的数相同。两份记录和那一次的按「文件加告警内容」逐条比过（不比行号；`clippy-cmp-lib.txt`、`clippy-cmp-lib-test.txt`）：没有多出一条，也没有少一条，也就是合进 `main` 的 54 个提交之后没有新的告警；只有原有的告警换了数，两种配置里各 2 条、测试配置另有 3 条：`src/io/pid/mod.rs` 的 `finish` 参数太多，从 8 个变成 9 个（RD-13 加的那一个）；「这个函数可能在栈上分配 N 字节」换了数的 4 条（`src/app/update/mod.rs` 1 条，测试配置里 `src/app/collab/` 下 3 条）。`main` 自己的 clippy 没有单独跑。
  - 架构报告（往上层依赖的有 33 对模块、183 个文件）：和这份计划最后一次的逐行相同，也就是合进 `main` 的 54 个提交之后，每一对模块的文件数都没有变。
- 不过的 5 条是 `main` 自己的。在干净的 `4e115a2f8` 上（这个工作目录临时检出，一个文件没改；`chainn1.txt`）跑了同样的测试：程序内 7,119 条过、3 条不过、65 条平时不跑；外部测试 `pid_import` 54 条里 52 条过、2 条不过，`pid_legend` 29 条、`pid_panel_localization` 1 条都过。不过的是同样 5 条，两条外部测试的失败信息和合并后逐字相同。数对得上：7,286 = 7,119 + 这份计划平时跑的 167 条；66 = 54 + 12。
  - 3 条设计稿测试：`app::pid_shell::pid_editor_screen_canvas::the_grid_is_drawn_the_way_the_design_draws_it`、`app::pid_shell::pid_shell_startup::the_screens_are_the_ones_the_design_navigates_to`、`ui::style::pid_tokens::tests::the_code_states_nothing_the_design_does_not`。它们拿代码和设计稿 `docs/pid-platform/design/pid-platform.pen` 比。提交在库里的设计稿从起点到 `4e115a2f8` 没有变过（1,769,868 字节）；`main` 的工作目录里有一份没有提交的（2,912,176 字节，2026-11-20 13:44），`main` 的 `7d545f845`、`acfabf006` 是照那一份写的代码。设计稿不是这份计划的文件，没有动。
  - 2 条外部测试：`tests/pid_import.rs` 的 `the_sheet_border_is_drawn_at_the_page_the_drawing_states`（要求纸面从原点开始；现在样例图的纸面角在 (−1.10, 1.84) mm）和 `lettering_starts_from_the_side_the_drawing_states`（要求每段标注文字都带对齐点；现在靠左的 28 段没有）。两条的断言都是起点以前写的；`main` 在起点之后没有动过 `tests/` 下任何文件，而 RD-13 有意把纸面放到图纸记的位置，RD-14（`7922f0960`，标注文字的竖直位置）有意把竖直对齐换成基线，靠左又在基线上的字不带对齐点。没有改，等用户定。
- 全厂重数（规格 §6 第 3 到 5 条，`src/io/pid/plant_totals.rs` 的那条测试；用合并提交编出来的测试程序带 `--ignored` 单独跑，备份只读；`totals-mrg.txt`）：和合并之前预计的一样，变了的只有 3 个缺口符号。
  - 是什么：管线交叉处的缺口（`Piping\Gaps\gap-curves.sym`），对象是管段（`PipeRun`）。`a85fa10` 的 `src/publish/pid_load.rs` 不再给挂在管段上的图例交出表示记录，所以导入器查不到它们的对象；`96ee00c` 交。图例本身还在图上。
  - 在哪：P井场、Q井场、O井场的注采阀组流程图各一个；这三张图上数据库给的位号各少一个（209 → 208、214 → 213、240 → 239）。另外 49 张图的读数逐行相同（`cmp-totals-mrg.txt`，比的是 SI-8 时这条测试通过的一次的日志 `si-8\libmut-green2-none.log`）。
  - 只带图纸文件时：带 `item` 的 9,218 → 9,215，带 `class` 的 8,742 → 8,739；按种类分的 12 行里 `PipeRun PIDPipingConnector 3` 这一行没有了，剩 11 行。
  - 补上数据库的位号之后：带 `label` 的 5,027 → 5,024（`database` 5,006 → 5,003，`file` 21 不变）；只有数据库有的 2,070 → 2,067；都没有的 4,198 → 4,201。
  - 没有变的：图例 9,225 个；只带图纸文件时带 `label` 的 2,957；标注 15,490 条（图例上 13,035、线上 2,303、别的对象上 152）；有标注的图例 6,091 个；图框之外没有 `label_of` 的文字 95 条；其余 11 行种类的个数；都有且相同的 2,904、都有但不同的 32、只有文件有的 21；数据库的位号 8,145 个；被拒绝的仍然只有 D03 一张。
  - 这条测试在合并提交上不过（`plant_totals.rs:381`，期望值是 `96ee00c` 时的数）。期望值和规格都没有改，等用户定。
  - 另外两条读备份的测试都过（`real-run-mrg.txt`）：`the_plant_database_gives_its_items_their_tags`（8,145 个位号）；`the_sample_opened_from_its_backup_carries_the_tags_of_the_database`（样例 592 个图例、354 个带数据库的位号；D07 18 个图例、9 个带位号）。
- 样例预算（`the_sample_imports_within_its_budget`，同一个测试程序单独跑了一次；`totals-budget.txt`）：过。三次 2,963、3,136、3,330 毫秒，中位数 3,136，预算 3,747；592 个引用、119 个定义，和钉的相同。读的时候有别的会话的两个编译进程在，所以这个数只说明没有超，不能拿来比快慢。
- 出图（`main` 的程序和合并后的程序各出一遍图，逐张比；不开窗口，不联网，图纸只读；`plotcmp-mm.txt`、`plotcmp-mm-sum.txt`）：
  - 两个程序：`main` 的是在干净的 `4e115a2f8` 上编的（这个工作目录临时检出，编完切回），合并后的是在 `16ba6fc27` 上编的。都是平时的开发配置，不是发布版。两个程序自己报的版本对得上：它们拒绝 D03 时各写了一份报告（写在这次自己的设置目录里），里面的修订号一个是 `4e115a2f80e3`，一个是 `16ba6fc271ea`。
  - 结果：`pid-test-data` 下 152 个 `.pid`。两个程序都出了图的 151 张，151 张的 SVG 逐字节相同，共 675,960 个图元、692,358,484 字节。两个程序都拒绝的 1 张：`backup-plant-data\test\U01\D03.pid`，和以前一样。用时 843 秒。
  - 两个程序确实不同：打开同一张图，图元数 151 张没有一张相同，合计 436,168 对 338,072。148 张是合并后的少（图例成了块引用；第一张 4,883 对 3,901）；3 张是合并后的多（`test\U01` 的 D04、D06、D07，都是小图：470 对 475、85 对 94、118 对 130）。
  - `main` 的改动确实在两个程序里：和 11-19 用这两份计划之前的程序出的图比（`D:\scratch\ocs-symblock\cp1\out\all-2c.plotcmp.jsonl`，同样 151 张），`main` 现在出的图 151 张都不一样（图元数、打开时的图元数都相同，内容不同），而合并后的程序和 `main` 现在的逐字节相同。也就是这两份计划「画出来不变」这一条，在 `main` 的 54 个提交之上仍然成立。
  - 3 个缺口符号的图形还在图上：那三张图（测试图纸里各有两份，共 6 个文件）出的图元数、打开时的图元数，两个程序都和换转换库之前相同。少的只是「它是哪个对象的图形」。
  - 没有上网：代理三个变量指着 `127.0.0.1:9`，设置目录是这次自己的，跑完里面没有起始页的文件（多出来的只有上面说的两份 D03 的报告）；两个程序的错误输出里各只有两行显卡探测的话。
- `main` 之后又动了：合并之后 `main` 加了一个提交 `53ea94d76`（2026-11-21 10:16:12；8 个文件：`src/io/pid_convert/` 下 2 个、`src/io/pid_publish/uid.rs`、`examples/pid_convert.rs`、文档 4 个），都不是这边改过的文件。拿它和合并提交试合（`git merge-tree`，只算不落地）没有冲突。这次演练没有再追它：门禁、重数、出图都是 `4e115a2f8` 这一版的。
- 出过的差错：09:40 第一次合并没成。那条命令写法不当，PowerShell 中途把 git 打断了，合并做到一半退了回去，留下三样东西：
  - 工作目录里 66 个 `main` 新加的文件的拷贝。逐个核对过和 `main` 里的逐字节相同、没有一个是别人的，才删掉，然后重新合并，成了。
  - 这个工作目录自己的 git 目录里一个空的 `AUTO_MERGE.lock`。当时就看到并删了。
  - 公共 git 目录里一个空的 `packed-refs.lock`。漏看了，10:04 提交合并时 git 报错才发现（提交本身成了）。它从 09:40:09 留到 10:04:11：这 24 分钟里，这个仓库任何一个工作目录里要动 `packed-refs` 的操作（删分支或引用、整理引用、自动 gc）都会失败，报这个文件已存在；普通的提交不受影响，`main` 这段时间没有新提交。确认是自己留下的之后删掉了（0 字节；创建时间和 git 记录里那次切分支是同一秒；当时没有 git 进程；`packed-refs` 本身没有变）。
  - 两次都当时告诉了用户；第一次说的「已经收拾干净」不对，第二次更正了。合并提交的说明里没有写这件事（第二个锁文件是提交时才发现的），补在这里。此后 git 命令一律经 `cmd /c` 跑，每次切换、提交之后查整个 git 目录有没有锁文件：都没有。
- 留给用户定的（都没有动）：
  - 3 个缺口符号：接受新的数（跟着转换库，改规格和测试里钉的数），还是想办法让它们留着管段这个对象，还是先不定。
  - `main` 自己不过的 5 条测试：2 条外部测试要不要在这条分支上跟着 RD-13、RD-14 改；3 条设计稿测试要等 `main` 那边把设计稿提交进来。
  - 推送。
- 没验证的：
  - 外部测试 35 组里只运行了 3 组（`pid_import`、`pid_legend`、`pid_panel_localization`）。其余 32 组在合并的树上只做过类型检查（`cargo check --tests`，0 个错误，合并提交之前跑的，树和提交相同），没有运行。`main` 在起点之后没有动过它们；默认主题改成亮色会不会让其中查颜色的测试不过，没有试。
  - 示例程序 9 个只做了类型检查（`cargo check --examples`，合并提交上，0 个错误；`check-examples-e1.done`），没有运行。
  - 带窗口的程序没有开；发布版没有编。出图比的是 SVG，不是屏幕。
  - 预算只读了一次，机器不空闲。
  - `main` 的 `53ea94d76` 没有合进来，只试合过。
  - 3 个缺口符号在图上是什么样子，没有去看（只知道图元数没有变）。规格 §2.2 的表里「数据库给得出对象」那一行（9,218 个，含这 3 个，也就是数据库里它们挂在管段上）是当时另外数的，这次没有重数。
  - 编译目录是几个工作目录共用的。每次都等别的会话的编译停下来才开始，但编两个出图程序的时候各有一个别的会话的 `cargo test --lib` 同时在编（它编的是测试程序，和出图程序不是同一个产物）。两个程序是各自那一版代码编的，依据是三件事：编译记录里编的是这个工作目录、工作区和提交没有差别；两个程序自己报的修订号各是各的；打开同一张图，两个程序的图元数不同（都见「出图」）。测试程序（门禁、重数用的）靠的是测试的条数和编译记录，没有别的核对。
- 证据：`D:\scratch\ocs-symblock\si-11\merge\`（重数的脚本 `totals-run.ps1`、比两份重数日志的 `cmp-totals.ps1`、比两份 clippy 记录的 `clippy-cmp.ps1`、编出图程序的 `build-prog.ps1`、检查示例的 `check-examples.ps1`；试合的记录 `summary.txt`、`hunks-identity-4e115a2f8.txt`）、`D:\scratch\ocs-symblock\si-11\merge\out\`（门禁 `chaing1.txt` 和各步的日志；干净的 `main` 上的 `chainn1.txt`；重数 `totals-mrg.txt`、`totals-mrg.log`、`cmp-totals-mrg.txt`；另两条读备份的测试 `real-run-mrg.txt`；预算 `totals-budget.txt`；两次编程序的记录 `build-prog-a.done`、`build-prog-b.done`；测试程序 `libtest-mrg.exe` 和 `build-libtest-mrg.done`；等别人编译的记录 `quiet*.txt`）、`D:\scratch\ocs-symblock\si-11\merge\cmp\`（出图的脚本 `run-plotcmp.ps1`、`plotcmp.py`、算合计的 `sum-plotcmp.ps1`；两个程序 `a\`、`b\`；结果 `out\plotcmp-mm.txt`、`out\plotcmp-mm-sum.txt`、`out\mm.plotcmp.jsonl`；D03 的两份报告在 `appdata\OpenCADStudio\recovery_logs\`）。

**演练之后：缺口符号按新的数改**（2026-11-21，11:11 起。接着上一段，也只在分支 `pid-symbol-on-main` 上；这份计划的两条分支和 `main` 没有动，没有推送）

- 用户怎么定的：上一段末尾留了三件事，报告里各给了做法。缺口符号，推荐的是接受新的数；`main` 自己不过的 5 条测试，推荐的是不动；推送，没有给推荐，只说要用户明说。用户 2026-11-21 11:11 答的是：「按你推荐的方案做。没有明确推荐的，你结合当前情况选最合适的继续，并简要说明理由。」照这一句：缺口符号按新的数改（这一段）；5 条测试不动；推送不在这一句里，没有推，仍然等用户明说。
- 没有推荐、自己选的两处，和理由（都先告诉了用户）：
  - 没有运行过的外部测试跑一遍。上一段把它们列在「没验证的」里；跑一遍只读、不联网，有不过的再到干净的 `main` 上跑同一组，分清是谁的。放在这个提交之后做，另记。
  - `main` 后来加的 `53ea94d76` 不追。它改的 8 个文件这边都没有改过，试合没有冲突；追一次要把门禁、重数、出图重做一遍，而 `main` 还在动，真要合的时候对着当时的 `main` 做一次就够。
- 更正上一段的一句话：「外部测试 35 组里只运行了 3 组…其余 32 组…」说得不准。`tests\` 下是 35 个文件，cargo 的测试目标是 7 个（`Cargo.toml` 里 `autotests = false`，目标逐个列出）：运行过的 3 个 `pid_*`；没有运行的 4 个，`integration`（`tests/integration.rs` 把另外 28 个文件当模块收进一个测试程序）、`bevy_composite_shader`、`dwg_acadsharp_samples`、`text_width_variants`。「没有运行、只做过类型检查」这一点不变，变的是数法：是 4 个测试程序，不是 32 组。上一个提交（`4eed11733`）的说明里也有这一句，提交不改，在这里和这个提交的说明里更正；对用户也说过同样的话，11:25 更正了。
- 改了什么：
  - `src/io/pid/plant_totals.rs`：钉的两段文字里 6 行不同。`of an item 9218` → `9215`；`of a class 8742` → `8739`；`PipeRun PIDPipingConnector 3` 一行删掉；`tagged 5027: database 5006, file 21` → `tagged 5024: database 5003, file 21`；`the database's tag put on 5006` → `5003`；最后一行里 `in the database alone 2070` → `2067`、`in neither 4198` → `4201`。新的数是从合并提交那一次重数的输出（`totals-mrg.log`）里照抄的，不是推算的。两个常量的注释写明这些数是 `a85fa10` 的，`96ee00c` 时是多少也留在注释里。
  - 另有两处注释引用了原来的数，一并改了：`src/io/pid/items.rs`（`SheetItem` 的说明里「9,218 of 9,225」，注明是 `96ee00c` 量的，现在是 9,215）；`src/app/commands/pidlegend.rs`（一条测试的说明里 5,027 → 5,024）。
  - 程序的行为没有变：三个文件里改的是一条平时不跑的测试钉的文字，和注释。
  - 规格：原来的数都没有动。带日期的说明加在五处（状态一句、§2.2 末尾一段、§6 第 3 条、§11 的 SI-8 第 2 条和第 4 条）；§11 最后加了一节「合进 main 之后」：差在哪里、变了和没变的数、三种做法和选这一种的理由、看得见的后果、改了什么、没有重数的。
- 验证（三个 `.rs` 改完之后的树；规格和这份清单是之后才改的，不参加编译）：
  - 测试程序：改完之后重新编（11:15:56 起，253 秒，离线），编完当即拷出来（`libtest-pin.exe`，310,726,144 字节，sha256 `cd7e25af…`）。它是这棵树编的：钉着新数的那条测试用它跑过了，而新的数只在这棵树里。
  - 那条测试带 `--ignored` 单独跑（11:20:10，备份只读；`totals-pin.txt`）：过了，51 秒。53 张，被拒绝的 1 张（D03）；数据库的位号 8,145 个；打印出来的两段数和钉的逐行相同。改之前它在合并提交上不过（上一段）。
  - 另两条读备份的测试，同一个测试程序（`real-run-pin.txt`）：都过。`the_plant_database_gives_its_items_their_tags`：8,145 个位号。`the_sample_opened_from_its_backup_carries_the_tags_of_the_database`：样例只带图纸文件 592 个图例、592 个带对象、231 个带位号；从面板打开 354 个带数据库的位号；D07 18 个图例、9 个带位号（数据库 8、文件 1）。
  - 门禁（11:21:01–11:34:08，离线；`chaing2.txt`），和合并提交那一次（`chaing1.txt`）逐项相同：
    - 程序内测试 7,286 条过、3 条不过、69 条平时不跑；不过的是 `main` 自己的那 3 条设计稿测试。
    - 外部测试三组：`pid_import` 64 条过、2 条不过（`main` 自己的那 2 条）；`pid_legend` 31 条、`pid_panel_localization` 2 条都过。
    - clippy 两遍 656 条、1,051 条；两份记录和合并提交那一次的逐条比过（`clippy-cmp-lib-g2.txt`、`clippy-cmp-lib-test-g2.txt`），没有多一条，也没有少一条；点到这三个文件的一条也没有。
    - 架构报告和合并提交那一次的逐字节相同（33 对模块、183 个文件）。
    - 格式：三个文件都没有新的 rustfmt 块（`pidlegend.rs` 原有的 5 块照旧）。
  - 共用的编译目录：启动前等到没有别的编译（等了 77 秒）。跑的 18 分钟里别的会话照常在编：先后起了 9 次 cargo（`test --lib` 4 次、`clippy` 4 次、另一个项目的 `build` 1 次），104 次观察里 10 次看到它们的编译器在共用目录里（`quietpin.txt`）。门禁的每一步都是从这个工作目录编的（各步日志里的 `Compiling` 或 `Checking OpenCADStudio … (D:\work\plant-code\cad\ocs-pid-symbol-block)` 一行），测试的条数和合并的树对得上（7,286 + 3 + 69）。程序内测试那一步本来不用重编（测试程序刚编过），它连同 5 个依赖又编了一遍；应当是别的会话的编译在中间动过共用目录，没有去查。没有碰别人的进程。
- 没验证的：
  - 3 个缺口图例在带窗口的程序里是什么样子没有看（换转换库之前也没有看过）。规格「合进 main 之后」第 5 条写的后果是照 §4.6、§4.7 的规则推出来的。
  - 规格 §2.2 第一张表里数据库的三行没有重数（当时那个一次性的核对程序没有重跑）；「数据库给得出对象仍是 9,218」是推出来的。
  - 那 4 个外部测试程序这时还没有运行。
  - 上一段「没验证的」里别的各条照旧：示例程序只做了类型检查；没有开带窗口的程序，没有编发布版；`53ea94d76` 没有合进来；样例预算只读过一次。
- 证据：`D:\scratch\ocs-symblock\si-11\merge\`（这一次的脚本 `pin-chain.ps1`：编测试程序、拷出来、重数、门禁连着跑；等别人编译和跑的时候看到的 `quietpin.txt`）、`D:\scratch\ocs-symblock\si-11\merge\out\`（`build-libtest-pin.done`、`libtest-pin.exe`；重数 `totals-pin.txt`、`totals-pin.log`；另两条 `real-run-pin.txt`；门禁 `chaing2.txt` 和各步的日志 `lib-testg2.log`、`integg2.log`、`clippy-libg2.log`、`clippy-lib-testg2.log`、`archg2.txt`、`fmtg2.txt`；两份 clippy 对比）。

**演练之后：另外 4 个外部测试程序**（2026-11-21，11:37 起。接着上一段，也只在分支 `pid-symbol-on-main` 上，跑的是提交 `b8d3a6d18`；程序没有改，没有推送）

- 为什么跑：上一段「自己选的两处」的第一处。到那时为止 7 个外部测试程序只运行过 3 个 `pid_*`，另外 4 个在合并的树上只做过类型检查。
- 结果（`chaini4.txt`、`integi4.log`）：4 个测试程序共 155 条，都过；没有不过的，没有平时不跑的，没有被筛掉的。
  - `integration` 150 条（`tests/integration.rs` 收进来的 28 个模块）。
  - `dwg_acadsharp_samples` 3 条。
  - `bevy_composite_shader` 1 条。
  - `text_width_variants` 1 条。
  - 条数和源文件里 `#[test]` 的个数逐个对得上（150、3、1、1）。
- 「过」的里面有 4 条在这台机器上其实什么也没有查。这些测试找不到要用的东西时自己打印一句话就返回，算作通过，而通过的测试打印的话 cargo 平时不显示；所以把这次编出来的测试程序带 `--nocapture` 各再跑了一遍看输出（三份 `nocapture-…-i4.txt`；跑之前核对过程序文件的写入时间 11:39:10、11:39:11 落在这次编译之内）：
  - `annotative_context_roundtrip` 的 3 条（这个模块一共就这 3 条：`annotative_leaf_contexts_survive_dwg_roundtrip`、`annotative_leaf_type_breakdown`、`dump_one_blkref_leaf`）：要读的参照图纸写的是另一台机器上的绝对路径（`tests/annotative_context_roundtrip.rs` 第 15 行，一个 Linux 用户目录下的文件），这台机器上没有，三条都打印 `SKIP: golden file … not present` 就返回了。
  - `text_font_rendering::block_nested_mtext_fill_tris_keep_paired_low_half_at_utm`（查块里文字的填充三角形高低两半成对）：它点名的字体 `DejaVu Sans` 这台机器没有装（装着的 DejaVu 只有 `DejaVu Math TeX Gyre`），打印 `no TTF fills resolvable on this host; skipping fill-pairing assertions` 就返回了。
  - 这 4 条要查的东西在这台机器上没有被这些测试查过。前 3 条跳不跳只看那份图纸在不在，和代码无关，换成 `main` 也一样。第 4 条照它自己的注释，是没有解析到 TTF 字体、填充是空的时候才跳过；这台机器没有装它点名的字体，文字那部分代码（`src/scene/text/`）这条分支一个文件也没有改，所以认为换成 `main` 它也一样跳过。这一句是推出来的：没有在 `main` 上跑过，装上那个字体之后它会不会真去查也没有试。它走的是 `scene::cache::block_cache::expand_insert`；那个文件这条分支改过一处（`emit_wire` 里解析曲线的深度，5 行换 5 行），不在填充三角形那一段。
  - `integration` 打印的话只有这 4 句。这 4 个测试程序的源文件里别的提前返回只有一处，`tests/entity_transform_regression.rs` 第 867 行，是设了 `OCS_UPDATE_GOLDEN` 才走的改写参照文件的那一支，这次没有设。
  - 另外两个也会自己跳过的测试程序这次是真做了的。`dwg_acadsharp_samples`：样例图纸在（工作目录旁边的 `ACadSharp\samples`，它点名的 14 个文件都在），没有打印一句跳过。`text_width_variants`：量了两种窄体，`Arial Narrow` 的宽高比 6.567（`Arial` 8.007），`Bodoni MT Condensed` 5.313（`Bodoni MT` 8.262）。
- 是谁的：这 4 个测试程序的源文件和 `main` 的相同（这条分支相对合进来的 `4e115a2f8`，`tests\` 下只改了 3 个 `pid_*` 文件），不同的是它们连着的库。没有不过的，所以没有到干净的 `main` 上再跑同一组。
- 怎么跑的：`gates.ps1 -NoFull -NoClippy -NoArch -Integ integration,bevy_composite_shader,dwg_acadsharp_samples,text_width_variants`，经 `quiet-run.ps1` 等到没有别的编译才开始（等了 31 秒）；离线、依赖锁定、有不过的也接着跑完（`cargo test --offline --locked --no-fail-fast --test …`）。11:38:08–11:39:19，70 秒，其中编译 62 秒。是从这个工作目录编的（日志里 `Compiling OpenCADStudio … (D:\work\plant-code\cad\ocs-pid-symbol-block)` 一行）；这 70 秒里观察了 7 次，没有别的会话在编（`quieti4.txt`）。
- 没有留下东西：`OCS_UPDATE_GOLDEN` 没有设；跑完（连同带 `--nocapture` 再跑的那几次）工作区没有改动，整个 git 目录没有锁文件；系统临时目录里 `ocs` 开头的条目前后都是 2,769 个，没有多一个，也没有少一个。
- 顺带看到的：那 2,769 个是程序内测试留下的，每跑一遍留 17、18 个（`ocs-m071-missing-<进程号>` 这样的目录、`ocs_ortho_<进程号>.bak` 这样的文件），最早的是 11-18 15:05 的，各个会话都在留；这条分支上跑门禁的那几次也一样（今天 11:23–11:24 那一批 18 个，和上一段门禁里程序内测试那一步的时间对得上）。名字在源码里找得到出处的 9 种出自 7 个文件，都不是这条分支改过的。没有清。
- 用时：事先对用户估的是「大约 40 分钟」，那是按「32 组」估的（上一段更正过的那个数法）；实际从开始等到跑完 1 分 42 秒。
- 到这里，7 个外部测试程序在这条分支上都运行过了：252 条过、2 条不过，不过的是 `pid_import` 里 `main` 自己的那 2 条（上一段）。
- `main` 又动了：现在是 `e2f43b91c`（2026-11-21 11:28:43），在合进来的 `4e115a2f8` 之后有 2 个提交、12 个文件，第一个就是上一段说的 `53ea94d76`。12 个文件里没有一个是这边改过的（这边相对 `4e115a2f8` 改了 73 个文件），`Cargo.toml`、`Cargo.lock` 不在其中。照上一段定的不追；`e2f43b91c` 没有试合过。
- 没验证的：
  - 上面跳过的 4 条所查的东西。
  - 这 4 个测试程序在干净的 `main` 上没有跑过；这里只知道它们在这条分支上都过。
  - 上一段「没验证的」里别的各条照旧：3 个缺口图例在带窗口的程序里的样子；规格 §2.2 第一张表里数据库的三行；示例程序只做了类型检查；没有开带窗口的程序，没有编发布版；`main` 后来的 2 个提交没有合进来；样例预算只读过一次。
- 证据：`D:\scratch\ocs-symblock\si-11\merge\quieti4.txt`；`D:\scratch\ocs-symblock\si-11\merge\out\`（`chaini4.txt`、`integi4.log`、`donei4.txt`；带 `--nocapture` 再跑的三份 `nocapture-integration-i4.txt`、`nocapture-dwg_acadsharp_samples-i4.txt`、`nocapture-text_width_variants-i4.txt`；跑之前临时目录的清单 `temp-ocs-before-i4.txt`）。
