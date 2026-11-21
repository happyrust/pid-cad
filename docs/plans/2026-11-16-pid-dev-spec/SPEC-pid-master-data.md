# SPEC：项目主数据（`pid-master-data`）

> 状态：**待建；规格已确认**（2026-11-16，§9 的待决问题都按「建议」定）。基线 `main @ 9b5327f96`。
> 依据：已批准计划 T3.1；需求 `REQ-SYM-013`–`016`（`docs/pid-platform/requirements/30-symbol-library-and-project-settings.md`）。
> 共用约定见 `01-shared-conventions.md`。

## 1. 目标

给一个项目一份统一的取值表：有哪些公称直径、管道等级、绝热等级、流体介质可用，装置怎么分区，用的是哪套图例标准。有了它：

1. 规则检查能判断「这条管线的管道等级在不在项目里」，也能取到等级的扩展属性（材质、最大公称压力）来比较。
2. 发布包里能带一份主数据快照，说明这次发布是按哪份取值表校验的。
3. 材料清单和规则检查能按装置分区（PBS 节点）圈定范围。

本模块只管主数据本身：格式、存放、读取、校验、导入导出。谁来用，见 `SPEC-pid-drc.md`、`SPEC-pid-bom.md`、`SPEC-pid-publish.md`。

## 2. 现状

- 没有主数据。`公称直径`、`管道等级`、`绝热等级`、`流体介质` 是管线上的属性名（`src/pid/propagate.rs` 的 `LINE_FIELDS`），取值是自由文本，填什么都收。
- 权限模型里已经有「主数据维护」这个操作（`crates/ocs_collab/src/permission.rs` 的 `Operation::MaintainMasterData`），桌面端还没有地方用到。
- 图例标准记在符号库上（`SymbolLibrary::current_standard()`，内置值是「内置示例标准」），不随图保存；图上已有符号实例时不许切换（`LEGEND_STANDARD_LOCKED`）。
- 图级记录的读写是现成的：`graph_data::store(doc, key, items)` 的 `key` 可以是任意字符串（`SPEC-pid-author-store.md` §2.4）。
- 读 Excel 的库 `calamine` 在依赖里，本期不用（见 §8）。

## 3. 数据格式

### 3.1 主数据对象

下面的 JSON 是格式定义，数值是举例。

```json
{
  "version": 1,
  "revision": 3,
  "updated_at": 1794800000,
  "legend_standard": "内置示例标准",
  "nominal_diameters": {
    "unit": "mm",
    "items": [
      { "dn": 15, "label": "DN15", "outer_diameter": null, "hidden": false },
      { "dn": 50, "label": "DN50", "outer_diameter": 60.3, "hidden": false },
      { "dn": 65, "label": "DN65", "outer_diameter": null, "hidden": true }
    ]
  },
  "pipe_classes": [
    {
      "name": "A1A",
      "attrs": {
        "压力等级": "PN16",
        "最大公称压力": "16",
        "材质": "碳钢",
        "公称直径下限": "15",
        "公称直径上限": "600"
      }
    }
  ],
  "insulation_classes": [
    { "name": "H", "attrs": { "说明": "保温" } }
  ],
  "fluid_codes": [
    { "name": "工艺水", "attrs": { "代号": "PW" } }
  ],
  "pbs": [
    { "id": "1001", "label": "常压蒸馏装置", "parent": null },
    { "id": "1001-01", "label": "原料罐区", "parent": "1001" }
  ]
}
```

| 字段 | 类型 | 约束 |
|---|---|---|
| `version` | 整数 | 格式版本，现为 1。读到更高的报 `MASTERDATA_VERSION_UNSUPPORTED` |
| `revision` | 整数 | 每次写入加 1；发布清单里记它 |
| `updated_at` | 整数 | 最近一次写入的时间，Unix 秒 |
| `legend_standard` | 文本或 `null` | 项目锁定的图例标准名。`null` 表示没有锁定 |
| `nominal_diameters.unit` | 文本 | 本期只有 `mm` |
| `nominal_diameters.items[].dn` | 整数 | 公称直径，表内唯一 |
| `…items[].label` | 文本 | 显示标签，≤ 16 字符 |
| `…items[].outer_diameter` | 数或 `null` | 外径，可不填 |
| `…items[].hidden` | 布尔 | 为真表示本项目不许用这个直径 |
| `pipe_classes[].name` | 文本 | 管道等级名称，非空，≤ 64 字符，表内唯一 |
| `pipe_classes[].attrs` | 对象 | 扩展属性：属性名 → 文本。属性名随项目自己定 |
| `insulation_classes[]`、`fluid_codes[]` | 同上 | 与管道等级同一种结构：`name` + `attrs` |
| `pbs[].id` | 文本 | 节点号，非空，唯一 |
| `pbs[].label` | 文本 | 节点名称 |
| `pbs[].parent` | 文本或 `null` | 上级节点号；`null` 是根。允许多个根，不许成环，不许指向不存在的节点 |

内置规则会读的扩展属性名（别的名字随便起）：

| 表 | 属性名 | 谁在读 |
|---|---|---|
| 管道等级 | `最大公称压力`（数值文本，如 `16`） | 规则 `DR-007` 阀门压力等级高于管道等级 |
| 管道等级 | `材质` | 规则 `DR-021` 介质与管道等级材质不兼容 |

**默认内容**（`PIDMASTERDATA INIT` 建出来的）：公称直径序列 15、20、25、32、40、50、65、80、100、125、150、200、250、300、350、400、450、500、600（共 19 个，标签 `DN<值>`，都不隐藏，外径不填）；`legend_standard` 取符号库当前的标准名；其余三张表和 PBS 为空。

### 3.2 存在哪里，哪一份算数

| 位置 | 是什么 | 何时用 |
|---|---|---|
| 项目文件 `pid-project.json` | 放在图纸所在的目录里，同目录的图纸共用 | 文件存在就用它 |
| 图内记录 `PID_AUTHOR_MASTERDATA` | 存在 DWG 里的图级记录 | 目录里没有项目文件时用它 |
| 都没有 | — | 主数据「未配置」 |

规则只有一条：**项目文件优先；没有项目文件用图内记录；都没有就是未配置。**

- 两处都有时，图内那份不参与任何判断，`PIDMASTERDATA STATUS` 会说明。
- 项目文件存在但读不出来（JSON 坏了、校验不过）时，**不**退回去用图内那份——那样会把问题藏起来——而是报错，主数据按「读不出来」处理。
- 没保存过的图没有目录，只能用图内记录。图纸所在目录用现有的 `crate::io::drawing_dir(doc)` 取。

### 3.3 项目文件 `pid-project.json`

```json
{
  "version": 1,
  "project": { "id": "CDU-2026", "name": "常压蒸馏改造" },
  "master_data": { "version": 1, "revision": 3, "updated_at": 1794800000 },
  "drawings": [
    { "file": "PID-1101.dwg", "drawing_id": "PID-1101", "pbs": "1001-01" }
  ]
}
```

| 键 | 必填 | 说明 |
|---|---|---|
| `version` | 是 | 文件格式版本，现为 1 |
| `project.id`、`project.name` | 否 | 项目号与名称，写进发布清单 |
| `master_data` | 否 | §3.1 的整个对象（上例省略了各张表）。没有这个键等于项目文件里没有主数据，按「没有项目文件」往下找图内记录 |
| `drawings[]` | 否 | 给图纸补两样信息：`drawing_id`（图号；不写就取文件名去掉扩展名）和 `pbs`（所属 PBS 节点号）。目录里没列出的 `.dwg` 照样算项目图纸，见 `SPEC-pid-project-index.md` |

这个文件是给人直接改的：UTF-8 不带 BOM，缩进 2 格，键按上面的顺序。程序写回时保留它不认识的顶层键，不会把人手加的内容写丢。

### 3.4 图内记录 `PID_AUTHOR_MASTERDATA`

用现有的图级记录布局，只有一条数据：`graph_data::store(doc, "PID_AUTHOR_MASTERDATA", &[主数据对象])`。超过 2000 字节由那套布局自动切片。

不改 `graph_data::keys()`：那个函数返回的四个键是已建成部分的清单，有测试照着它跑。新记录名定义在本模块，往返测试也写在本模块。

### 3.5 属性值怎么与主数据比

- 属性值是文本，比之前去掉首尾空白。
- 管道等级、绝热等级、流体介质：与 `name` 逐字相等才算在表里，区分大小写。
- 公称直径：去掉开头的 `DN`（不分大小写）和空白后按整数解析，与 `dn` 比。`DN80`、`dn 80`、`80` 都算 80。解析不出来，或者那个直径 `hidden` 为真，都算不在表里。
- 主数据未配置或读不出来时，「在不在表里」的结论是「不知道」，不是「不在」。规则检查据此不报这类问题，只给一条说明（见 `SPEC-pid-drc.md`）。

属性里存的是文本的副本，不是对主数据条目的引用。所以改名、删除一条主数据不会动已有对象的取值，这正是需求要的（`REQ-SYM-014` 第 3 条：「修改与删除只对新建管线生效」）；删掉以后，还在用旧值的对象会被规则检查报出来。

### 3.6 错误码

| 码 | 何时 | HTTP |
|---|---|---|
| `MASTERDATA_SCHEMA_ERROR` | JSON 结构不对。说明里带出错位置，如 `pipe_classes[2].name` | 400 |
| `REQUIRED_FIELD_MISSING` | 名称为空（需求起的名字） | 400 |
| `MASTERDATA_DUPLICATE` | 同一张表里名称重复，或 `dn` 重复，或 PBS 节点号重复 | 409 |
| `MASTERDATA_PBS_INVALID` | PBS 的 `parent` 指向不存在的节点，或成环 | 400 |
| `MASTERDATA_VERSION_UNSUPPORTED` | `version` 比程序认识的高 | 400 |
| `MASTERDATA_CONFLICT` | 写项目文件时发现磁盘上的 `revision` 已经变了（有人先改过） | 409 |
| `MASTERDATA_EXISTS` | `INIT` 时目标位置已经有主数据；或导出的目标文件已存在 | 409 |
| `MASTERDATA_NOT_CONFIGURED` | 要导出或嵌入，但主数据未配置 | 409 |

## 4. 数据怎么传

```mermaid
flowchart TD
    A{"图纸所在目录有<br/>pid-project.json 且含 master_data？"}
    A -- 有 --> B["读项目文件并校验"]
    A -- 没有 --> C{"图内有<br/>PID_AUTHOR_MASTERDATA？"}
    B -- 通过 --> E["生效的主数据<br/>来源：project"]
    B -- 不通过 --> F["读不出来<br/>带错误码与出错位置"]
    C -- 有 --> G["生效的主数据<br/>来源：drawing"]
    C -- 没有 --> H["未配置"]
    E --> U["使用方"]
    G --> U
    F --> U
    H --> U
    U --> U1["规则检查：取值在不在表里、等级的扩展属性"]
    U --> U2["发布：整份写进发布包的 master-data.json"]
    U --> U3["材料清单、规则检查：按 PBS 节点圈图纸"]
```

- **读**：每次要用时解析一次（`master_data::resolve`）。文件很小，不做常驻缓存；这样人在外面改了项目文件，下一次检查就能看到。
- **写项目文件**：先读磁盘上的当前内容 → 比 `revision`，与调用方拿到的不等就报 `MASTERDATA_CONFLICT` → `revision` 加 1，`updated_at` 取当前时间 → 写到临时文件 → 改名覆盖。失败不留半成品，原文件不变。
- **写图内记录**：这是一次图纸修改，与 `PIDPIPE` 写拓扑表走同一条路：先 `push_undo_snapshot`，再 `graph_data::store`。所以能撤销，图纸变为「已修改」，随下一次保存落盘。
- **协同时不传**：协同协议只传对象级的变更，图级记录不在报文里（`SPEC-collab-protocol.md`）。一个人往图里写了主数据，房间里的其他人收不到。见待决问题 M2。

## 5. 对外接口

### 5.1 模块接口（`src/pid/master_data.rs`）

```rust
pub const PID_AUTHOR_MASTERDATA: &str = "PID_AUTHOR_MASTERDATA";
pub const PROJECT_FILE: &str = "pid-project.json";

pub enum Source { Project(PathBuf), Drawing }
pub enum Resolved {
    Configured { data: MasterData, source: Source },
    Unreadable { source: Source, error: MasterDataError },
    NotConfigured,
}

/// 按 §3.2 的规则找出生效的那一份。
pub fn resolve(doc: &Document, drawing_dir: Option<&Path>) -> Resolved;
/// §3.1 的默认内容。
pub fn defaults(legend_standard: &str) -> MasterData;
/// 校验一个 JSON 值；错误里带出错位置。
pub fn validate(value: &serde_json::Value) -> Result<MasterData, MasterDataError>;
pub fn store_in_drawing(doc: &mut Document, data: &MasterData);
pub fn write_project_file(
    dir: &Path, data: &MasterData, expected_revision: Option<u64>,
) -> Result<(), MasterDataError>;

impl MasterData {
    pub fn has_diameter(&self, value: &str) -> bool;            // §3.5 的比法
    pub fn entry(&self, table: Table, name: &str) -> Option<&Entry>;
    pub fn names(&self, table: Table) -> Vec<&str>;             // 给以后的下拉候选用
    pub fn pbs_subtree(&self, id: &str) -> Vec<&str>;           // 节点自身加所有下级
}
```

`Table` 取 `PipeClass` / `InsulationClass` / `FluidCode`。`MasterDataError` 每个变体有 `code()`，返回 §3.6 的码。

### 5.2 命令

| 命令 | 做什么 | 只读会话里 |
|---|---|---|
| `PIDMASTERDATA` 或 `PIDMASTERDATA STATUS` | 说明生效的是哪一份、修订号、各表行数；读不出来时说明错在哪里；两处都有时说明图内那份没在用 | 允许 |
| `PIDMASTERDATA INIT [PROJECT\|DRAWING]` | 建一份默认主数据。不带参数时：图纸已保存就建在项目文件里，否则建在图内。目标位置已有主数据则拒绝（`MASTERDATA_EXISTS`） | 拒绝 |
| `PIDMASTERDATA EXPORT <文件>` | 把生效的主数据写成一个 JSON 文件（§3.1 的格式）。目标已存在则拒绝 | 允许 |
| `PIDMASTERDATA IMPORT <文件> [PROJECT\|DRAWING]` | 校验后整份替换。目标的默认值同 `INIT` | 拒绝 |
| `PIDMASTERDATA EMBED` | 把生效的主数据抄进图内记录。单独把一张图发给别人前用 | 拒绝 |

只读会话的处理照 `PIDLEGEND` 的写法：命令名进 `READONLY_ALLOWED`，`readonly_refuses_command`（`src/app/collab/apply.rs`）里加一条——只有不带子命令、`STATUS`、`EXPORT` 放行。

本期没有逐行增删改的命令，也没有编辑界面：改主数据就是改项目文件，或者导出、改、再导入。见待决问题 M1。

### 5.3 自动化操作与 REST

| 操作 | REST | 读写 | 请求 | 应答 |
|---|---|---|---|---|
| `pid_masterdata` | `GET /api/v1/pid/master-data` | 读 | 可选 `usage`（布尔） | 见下 |
| `pid_masterdata_set` | `PUT /api/v1/pid/master-data` | 写 | `target`（`project` / `drawing`，可省）、`master_data`（整个对象）、`expected_revision`（可省；给了就比） | `{"ok":true,"source":"project","revision":4}` |

读的应答：

```json
{
  "ok": true,
  "state": "configured",
  "source": "project",
  "path": "D:/proj/cdu/pid-project.json",
  "shadowed_in_drawing": false,
  "master_data": { "version": 1, "revision": 3 },
  "usage": {
    "pipe_classes": { "A1A": 12 },
    "insulation_classes": {},
    "fluid_codes": { "工艺水": 9 },
    "nominal_diameters": { "80": 7 }
  }
}
```

- `state` 取 `configured` / `unreadable` / `not_configured`。`unreadable` 时应答仍是 `ok: true`（读状态这件事成功了），另带 `error_code` 与 `error`，没有 `master_data`。
- `shadowed_in_drawing` 为真表示图里也有一份但没在用。
- `usage` 是需求里的「使用中管线数 / 使用中对象数」：当前图上取值等于该条目的对象个数，请求带 `usage: true` 时才算。表里没有的取值不列。

MCP：`pid_masterdata` 进 `READ_OPS`，`pid_masterdata_set` 进 `EXECUTE_OPS`，同步更新 `TOOL_SCHEMA_DIGEST`（共用约定 §7.6）。

### 5.4 权限

- 没连协同时，桌面端不判权限。
- 连了协同且会话只读时，写主数据的命令和操作被拒，错误码与其他被拒的写操作相同（`COLLAB_READONLY` 或 `PERM_DENIED_*`，见 `SPEC-collab-client.md` §4.7）。读不受限。
- 会话可写时，本期不按「主数据维护」这个操作单独判权。见待决问题 M3。

## 6. 成功标准

1. 一份主数据写进项目文件再读回，逐项相等；写进图内记录、存成 DWG、再打开，逐项相等。
2. 两处都有时取项目文件；项目文件坏了时报错，不悄悄换成图内那份。
3. 校验失败时错误里有错误码和出错位置。
4. 写项目文件是原子的：中途失败，原文件逐字节不变，目录里不留临时文件。
5. 同样的主数据写出的项目文件逐字节相同（`updated_at` 相同的前提下）。
6. 写图内记录可以撤销，撤销后图内没有这条记录。
7. 只读会话里三个写命令被拒，状态与导出可用。
8. 2,000 行管道等级的主数据，解析加校验不超过 50 ms。

## 7. 测试

先写会失败的测试，再实现（共用约定 §5）。

| 层 | 测什么 |
|---|---|
| 单元（`src/pid/master_data.rs`） | 默认内容有 19 个直径；往返（JSON、图内记录、DWG 文件）；§3.6 每个错误码各一个反例，断言错误码与出错位置；§3.5 的比法（`DN80`、`dn 80`、`80`、`八十`、隐藏的直径）；PBS 子树；§3.2 的四种情形（只有项目文件、只有图内、两处都有、项目文件坏了） |
| 单元（写文件） | `revision` 不等时报 `MASTERDATA_CONFLICT` 且文件不变；写回后不认识的顶层键还在；目标目录不可写时不留临时文件 |
| 命令与操作 | `INIT` → `STATUS` → `EXPORT` → 改文件 → `IMPORT` 的一条龙；`pid_masterdata` 的三种 `state`；`usage` 计数；只读会话里的拒绝与放行 |
| 性能预算 | 2,000 行解析加校验 ≤ 50 ms（`#[ignore]`，写法照 `src/pid/perf_budget.rs`） |

要动的文件：新建 `src/pid/master_data.rs`、`src/app/commands/pidmasterdata.rs`；改 `src/pid/mod.rs`、`src/app/commands/mod.rs`、`src/app/command_driver/utilities.rs`（整行名单）、`src/app/collab/apply.rs`、`src/app/control/mod.rs`、`src/rest.rs`、`src/rest_openapi.json`、`src/mcp.rs`。

## 8. 不在范围

- 主数据的编辑界面（设计稿「主数据管理」屏）、逐行增删改命令、从 Excel 导入。
- 画图时属性输入框从主数据取下拉候选值。接口 `names()` 留好了，界面不接。
- 英制直径序列（需求里的「单位设置」只做 `mm`）。
- 「修改条目同步更新已有对象的取值」（设计稿里的引用影响）：属性存的是文本副本，与需求一致，不做同步。
- 经协同服务器分发主数据。

## 9. 待决问题

> **2026-11-16 已定**：本节各条都按「建议」一栏执行。汇总见 `plan.md` §3，对应的任务见 `todo.md`。

| # | 问题 | 建议 |
|---|---|---|
| M1 | **主数据要不要编辑界面。** 计划的面板三个页签（规则检查、材料清单、快照与发布）里没有主数据，本规格按「改项目文件加导入导出」写 | 本期不做界面。主数据改得少，项目文件是给人直接改的；界面和属性下拉放到面板任务之后，作为可选任务 |
| M2 | **协同时主数据怎么共用。** 图内记录不进协同报文；各人的项目文件在各自的目录里 | 本期靠项目文件，各人自己保证一致；发布包带主数据快照和修订号，可以对账。经服务器分发写进阶段 4 纲要 |
| M3 | **「主数据维护」要不要按角色判权。** 客户端现在只知道会话可写不可写，不知道角色有哪些操作权 | 本期只按会话只读与否拦。按操作判权要给 `welcome` 加一个可选键把操作权清单带下来，放到后续 |
| M4 | **默认内容预置多少。** 计划举了 `A1A`、`B1B` 作例子 | 只预置公称直径序列；管道等级、绝热等级、流体介质留空。预置的示例等级会被当成真的用 |

## 10. 需求对照

| 需求 | 本期 | 说明 |
|---|---|---|
| `REQ-SYM-013` 公称直径序列维护 | 部分 | 有序列、显示标签、外径、隐藏。「单位设置」只有 `mm`；没有编辑界面；「隐藏后不进下拉」「标签只对新建对象生效」要等属性输入接上主数据 |
| `REQ-SYM-014` 管道等级维护 | 部分 | 有名称加扩展属性、名称必填（`REQUIRED_FIELD_MISSING`）与唯一（409）、使用中管线数（`usage`）、「改删不影响已有管线」。没有编辑界面，没有下拉取值 |
| `REQ-SYM-015` 绝热等级维护 | 部分 | 同上 |
| `REQ-SYM-016` 流体介质维护 | 部分 | 同上 |
| 计划 T3.1「图例标准锁定标识」 | 覆盖 | `legend_standard` |
| 计划 T3.1「公称直径标准表 DN15–DN600」 | 覆盖 | 默认内容 |
| 计划 T3.1「管道等级表及允许公称直径范围」 | 覆盖 | 范围用扩展属性 `公称直径下限` / `公称直径上限` 表示；按它检查是一条自定义规则，写法见 `SPEC-pid-drc.md` |
| 计划 T3.1「绝热等级表、流体介质代码表、PBS 节点树」 | 覆盖 | §3.1 |
| 计划 T3.1「单图存 XRecord，项目目录可选共享 `pid-project.json`」 | 覆盖 | §3.2 |

## 11. 实现记录

实现与本规格、任务文字有出入的地方，以及规格没写到、实现时定下的读法，记在这里（`todo.md` §1 第 7 条）。

### 11.1 `MD-1`：数据对象、默认内容、校验、比法（2026-11-20）

**做了什么**（新文件 `src/pid/master_data.rs`，`src/pid/mod.rs` 登记一行）：

- 数据对象 `MasterData`、`Diameter`、`Entry`、`PbsNode` 和三张表的标识 `Table`。`Entry` 的扩展属性存成名值对，按文件里的顺序；`to_json` 按 §3.1 的键序写出。
- `defaults(legend_standard)`：19 个直径，其余为空；`revision`、`updated_at` 都是 0，表示还没写过。
- `validate(value)`：错误类型 `MasterDataError`，`code()` 是 §3.6 的码，`at()` 是出错位置（整个对象是空串，显示成「顶层」）。只有校验类的五个码；`MASTERDATA_CONFLICT`、`MASTERDATA_EXISTS`、`MASTERDATA_NOT_CONFIGURED` 属于存放和命令，留给 `MD-2`、`MD-3`。
- 比法 `has_diameter`、`entry`、`names` 和 `pbs_subtree`。§5.1 没列、另外公开的：`to_json`、`table`、`Entry::attr`、直径的读法 `parse_dn`、`MasterDataError::at`、`Table::key`。

**规格没写到、这里定下的：**

1. **哪些键可以不写。** `legend_standard`、`outer_diameter`、`parent` 不写当作 `null`，`hidden` 不写当作假，`attrs` 不写当作空对象；其余的键缺了报 `MASTERDATA_SCHEMA_ERROR`，位置是缺的那个键。缺 `name` 键也是这个码，不是 `REQUIRED_FIELD_MISSING`——后者只给「有这个键、内容为空」。
2. **认不得的键读时不理，写回时不带。** 指的是主数据对象里面的键；项目文件顶层认不得的键要保留，那是 `MD-2` 的事。
3. **取值的约束。** `version`、`revision` 要是非负整数，`version` 为 0 也不行；`updated_at` 要是整数；`dn` 要是 1 到 4294967295 的整数；`outer_diameter` 可以是任意数；`label` 不超过 16、`name` 不超过 64，都按字符数算，不按字节数。不合的都报 `MASTERDATA_SCHEMA_ERROR`。
4. **什么算空。** 名称去掉首尾空白后为空就算空（§3.5 比之前要去空白，只有空白的名称永远比不上）；PBS 节点号为空也报 `REQUIRED_FIELD_MISSING`（§3.6 只写了名称）。名称两端的空白不去掉、也不拒收，原样存；这样的名称比不上任何属性值，要不要拒收等有了编辑入口再定。
5. **重复。** 逐字比，区分大小写（`A1A` 与 `a1a` 不算重复），只在同一张表里比；报在后出现的那一个上。
6. **报哪一处。** 只报第一处：先看 `version`，比程序认识的高就不再往下读（后面的结构坏了也报 `MASTERDATA_VERSION_UNSUPPORTED`）；其余按 §3.1 的键序逐个读。PBS 先逐个读完、查重，再查上级在不在，最后查环；成环报在环上下标最小的节点的 `parent` 上，说明里写出整个环，如 `1001 → 1001-01-A → 1001-01 → 1001`。
7. **直径的读法。** 去掉 `DN` 和它后面的空白，余下的要全是 ASCII 数字：`+80`、全角的 `８０`、`DN8 0`、超出 `u32` 的都算读不出来。§3.5 只说「按整数解析」，按一般的整数解析 `+80` 是 80，这里不认。

**测试**（同文件，都以 `master_data` 开头）：10 条常规，1 条耗时（`master_data_2000_pipe_classes_parse_and_validate_within_50_ms`，默认不跑）。先只放数据对象和空壳函数，10 条都编得过、都败在断言上；实现后全过。耗时：2,000 行管道等级、每行 5 个扩展属性，文本 460,477 字节，解析加校验三次 16.46 / 16.53 / 17.25 ms，中位数 16.53 ms，预算 50 ms（测试配置下主程序包不优化，`serde_json` 按 `opt-level = 2` 编）。

### 11.2 `MD-2`：两处存放与取用顺序（2026-11-20）

**做了什么**（只改 `src/pid/master_data.rs`）：

- §5.1 列的 `PID_AUTHOR_MASTERDATA`、`PROJECT_FILE`、`Source`、`Resolved`、`resolve`、`store_in_drawing`、`write_project_file`。另外公开了只读一处的 `read_project_file`、`load_from_drawing`（`MD-3` 的 STATUS 要用它们说出「图内那份没在用」），和项目文件的格式版本 `PROJECT_FILE_VERSION`。
- 错误多了两个码。`MASTERDATA_CONFLICT` 是 §3.6 的。`MASTERDATA_IO_ERROR` 是新加的，§3.6 没有：读写项目文件时文件系统报的错，出错位置是文件路径。接 REST 时（`MD-4`）它按「没列的一律 400」走，要不要单列到时再定。`MASTERDATA_EXISTS`、`MASTERDATA_NOT_CONFIGURED` 属于命令，留给 `MD-3`。
- `MASTERDATA_VERSION_UNSUPPORTED` 也带出错位置了：项目文件顶层的 `version` 和主数据里的 `version` 都可能报它，位置分别是 `version` 和 `master_data.version`。
- `dwg_roundtrip.rs` 没动：空白图纸直接能存成 DWG 再读回，用不着那里的辅助函数。

**与 §5.1 的写法不同：** `write_project_file` 返回写进去的那一份（带新的 `revision` 和 `updated_at`），不是 `()`。§5.3 的写命令要回新的 `revision`，这样不用再读一遍文件。

**规格没写到、这里定下的：**

1. **新修订号。** 取磁盘上主数据的 `revision` 加 1；磁盘上没有（没有文件、没有 `master_data`、修订号读不出来）就是 1。传进来那一份自己的 `revision` 不看。给了 `expected_revision` 而磁盘上没有修订号，也算冲突。
2. **文件本身读不出来时不写。** 不是 JSON、顶层不是对象、`version` 缺了或不是正整数，报 `MASTERDATA_SCHEMA_ERROR`；`version` 比 1 高，报 `MASTERDATA_VERSION_UNSUPPORTED`。这几种都不写，文件不动，免得冲掉人手写的内容。只是 `master_data` 那一段坏了照写：整份写进去就是修好了。
3. **读项目文件。** `master_data` 为 `null` 与没有这个键一样，按「没有项目文件」往下找。开头的 UTF-8 BOM 跳过（Windows PowerShell 5.1 存 UTF-8 会带）。出错位置从文件顶层算起，如 `master_data.pipe_classes[1].name`。`project`、`drawings` 不归这里校验。
4. **写出的样子。** 键按 `version`、`project`、`master_data`、`drawings` 排，其余顶层键按原来的顺序接在后面；原来没有的 `project`、`drawings` 不补。缩进 2 格，末尾一个换行。键序靠 `serde_json` 的 `preserve_order`：主程序包没有直接打开它，是别的依赖打开的。测试里查了键序，哪天关掉了会报出来。
5. **临时文件。** 与项目文件同目录，名字是 `pid-project.json.tmp`；写或改名失败都删掉。没有加锁：修订号比对挡得住先后两次写，挡不住两处读到同一修订后同时写。
6. **图内记录。** `store_in_drawing` 用 `graph_data::store` 整条重写，不动修订号和时间，也不推撤销快照，这些都归命令（§4）。读时记录里要恰好是一个读得出的对象；多了、少了、有读不出的分段，都报 `MASTERDATA_SCHEMA_ERROR`，位置为空。
7. **「目录不可写」怎么测。** Windows 上目录的只读属性挡不住在里面写文件，换了三种做法：项目文件的位置上是个不空的目录（临时文件写出来了，改名失败）；目录不存在；项目文件被别的程序以只许读的方式开着（只在 Windows 上跑）。三种都查了原文件不变、没留下临时文件。

**测试**（同文件，都以 `master_data` 开头）：新加 11 条，其中 1 条只在 Windows 上跑。§3.2 的四种情形在 `resolve` 的三条里：只有项目文件与两处都有一条，只有图内与都没有一条，项目文件坏了一条；另有一条是图内记录坏了。其余各条：DWG 存了再打开，图内那份不变；冲突时文件逐字节不变；别的顶层键还在，键序对；同一份数据、同一时间两次写出逐字节相同；三种写失败；读不出的项目文件不覆盖。先放空壳函数，11 条都编得过、都败在断言上；实现后主数据 21 条全过（耗时那条默认不跑）。

### 11.3 `MD-3`：命令 `PIDMASTERDATA`（2026-11-20）

**做了什么**（新文件 `src/app/commands/pidmasterdata.rs`；`src/app/commands/mod.rs` 登记模块、接进分派、加进命令表；`src/app/command_driver/utilities.rs` 把它登记成整行一条命令；`src/app/collab/apply.rs` 加只读的那一条）：

- §5.2 的五种写法。子命令和目标不分大小写；不认识的子命令或目标、缺文件名，都报错并说出正确的写法。
- `MASTERDATA_EXISTS`（`INIT` 的目标位置已有主数据，`EXPORT` 的文件已存在）和 `MASTERDATA_NOT_CONFIGURED`（`EXPORT`、`EMBED` 时未配置）是命令文件里的常量，没有加进 `MasterDataError`：这两个是命令自己的判断，读写主数据的函数不会报。其余的码照 `MasterDataError::code()` 报。报错时命令行的错误码也设上。
- 写图内记录先 `push_undo_snapshot` 再写，撤销一步记录就没了；写项目文件、导出文件不进撤销。
- 只读：`readonly_refuses_command` 加一条，`PIDMASTERDATA` 只放行不带子命令、`STATUS`、`EXPORT`；命令名进 `READONLY_ALLOWED`。

**任务没列、多改的文件：** `src/pid/master_data.rs` 加了 `write_drawing_record(doc, data, expected_revision)`，写图内记录，规矩同 `write_project_file`：修订号取记录上原有的加 1，没有记录或读不出修订号是 1；给了 `expected_revision` 而对不上报 `MASTERDATA_CONFLICT`，什么也不写；`updated_at` 取当前时间；原记录坏了照样盖掉；返回写进去的那一份。撤销快照仍由调用方先推。`INIT`、`IMPORT` 写图内时用它，`MD-4` 的写操作写图内时也用它。`MASTERDATA_CONFLICT` 的说明和显示改成不提项目文件，两处都会报它。

**规格没写到、这里定下的：**

1. **「图纸已保存」怎么算。** 看图纸有没有文件路径（从文件打开的，或存过的），有没有没存的改动不管。项目文件在那个路径所在的目录。
2. **写项目文件而图纸没有路径。** 写了 `PROJECT` 而图纸还没存过：报错、不写，提示先存图或写进图内。§3.6 没有对应的码，这条和其他写法上的错（不认识的子命令或目标、缺文件名）都不带码。
3. **`INIT` 遇到读不出来的主数据。** 目标位置有主数据但读不出来（项目文件坏了、图内记录坏了），`INIT` 不动它，报读的时候的码。要换掉它用 `IMPORT`；项目文件整个读不出来时 `IMPORT` 也不写（§11.2 第 2 条），得先手工修好或挪走。
4. **文件名。** 是这一行剩下的部分，可以带空格，也可以加一对双引号；不是完整路径时从图纸所在目录算，没存过的图从当前目录算。`IMPORT` 末尾一个词是 `PROJECT` 或 `DRAWING`（不分大小写）就是目标，所以文件名本身以这两个词之一结尾时要加引号。
5. **导出与导入的格式。** `EXPORT` 写出 §3.1 的主数据对象，`revision`、`updated_at` 照生效的那份；缩进 2 格、不带 BOM、末尾一个换行，同项目文件。只新建文件：已存在报 `MASTERDATA_EXISTS`，不覆盖；写到一半失败就删掉。`IMPORT` 读同样的形状，开头的 BOM 跳过；不是 JSON 报 `MASTERDATA_SCHEMA_ERROR`，校验不过报校验的码和位置（从主数据对象算起，如 `pipe_classes[1].name`），什么也不写。文件里的 `revision`、`updated_at` 不看，修订号按目标位置原有的加 1。
6. **`EMBED`。** 只在生效的是项目文件时有事做：把那一份原样抄进图内记录，修订号和时间照抄——抄的是同一份，单独拿到图纸的人看修订号就知道是项目的第几版。图内已是一样的就不写，也不推撤销步。生效的就是图内记录时只说一声；未配置报 `MASTERDATA_NOT_CONFIGURED`；项目文件读不出来报它的码，不拿图内那份顶替。
7. **`STATUS` 怎么说。** 读不出来用警告说（读状态这件事本身成了，同 §5.3 的 `unreadable`），未配置用提示说，都不设错误码。项目文件生效而图内也有一份时，加一行说图内那份没在用；项目文件读不出来而图内有一份时，说不拿它顶替。写进图内而生效的是项目文件时（`INIT DRAWING`、`IMPORT … DRAWING`），写完加一条警告：图内这份不生效。
8. **只读挡在哪里。** 只读会话拦编辑的那道锁（`refuses_step`）只挡图纸修改；项目文件写在图纸外面，挡它的只有命令入口这一道。`MD-4` 的写操作写项目文件时要自己查只读。

**测试**（`pidmasterdata.rs` 6 条，都以 `pidmasterdata` 开头；`master_data.rs` 新加 2 条；`apply.rs` 的两条只读测试，名单里各加了几种写法）：

- 一条龙 `INIT` → `STATUS` → `EXPORT` → 改文件 → `IMPORT`，查每一步的输出和落盘内容；再 `INIT`、再 `EXPORT` 到同一个文件都报 `MASTERDATA_EXISTS`，文件逐字节不变；导入有重复项的文件报 `MASTERDATA_DUPLICATE`，项目文件不变。
- 没存过的图写进图内：一个撤销步，撤销后没有这条记录；写 `PROJECT` 报错。
- 导入到图内；`EMBED` 抄进图内、撤销后没有记录；未配置时 `EMBED`、`EXPORT` 报 `MASTERDATA_NOT_CONFIGURED`。
- 只读会话：`INIT`、`INIT DRAWING`、两种目标的 `IMPORT`、`EMBED` 被拒，项目文件、图内记录、撤销步都没变；`STATUS`、不带子命令、`EXPORT` 可用。
- 从命令行执行 `PIDMASTERDATA INIT DRAWING` 只执行一次：一个撤销步，只建一次，没有先跑一遍 `STATUS`。
- 命令表里有它；几种写错的写法都报错、以命令名开头，什么也没写。
- `write_drawing_record`：修订号接着记录上的数；对不上时报冲突、记录不变；空白图纸给了 `expected_revision` 也算冲突；坏了的记录照样盖掉。

先放空壳（命令什么也不做，`write_drawing_record` 原样返回传进来的那份、不写），8 条新测试编得过、都败在断言上；实现后全过。

### 11.4 `MD-4`：主数据的操作与接口（2026-11-20）

**做了什么**（`src/app/control/mod.rs` 登记两个操作：读进查询名单，写进动作分派，能力清单加 `pid_master_data`；`src/rest.rs` 加 `GET` / `PUT /api/v1/pid/master-data` 和状态码；`src/rest_openapi.json` 写这两条；`src/mcp.rs` 读写各进一个名单，读的参数加 `usage`，`TOOL_SCHEMA_DIGEST` 换成新值）：

- 两个操作的逻辑写在命令文件里（`control_pid_masterdata`、`control_pid_masterdata_set`），和命令共用「写到哪里」的判断和写入函数。
- 读：应答照 §5.3。未配置时 `source`、`path` 是 `null`；生效的是图内记录时 `path` 是 `null`；读不出来时带 `error_code`、`error`，没有 `master_data`。生效的（或读不出来的）是项目文件而图内也有一份时，`shadowed_in_drawing` 为真。
- 写：依次查只读、`target`、`expected_revision`、`master_data`，都过了才写。写图内记录是一个撤销步；修订号对不上时连撤销步也不留。
- REST 的 `GET` 用查询参数 `usage=true`（或 `1`）；`PUT` 的请求体就是操作的参数。

**任务没列、多改的文件：**

- `src/app/commands/pidmasterdata.rs`：两个操作的入口。命令里「写到哪里」的判断提成 `target()`，命令和操作共用；写入函数加 `expected_revision` 参数，命令传 `None`。
- `src/pid/master_data.rs`：加 `usage(doc, data)` 和 `Usage`，数各条目在图上有几个对象用着。
- `src/mcp_ops.rs`：写操作的参数表 `PID_MASTERDATA_SET`（不认识的键拒收）。执行门面的参数说明和调用前的校验从这里来，`EXECUTE_OPS` 里的操作都要在这里登记。

**规格没写到、这里定下的：**

1. **写成功的应答。** 同别的写操作，结果放在 `result` 里：`{"source":"project","path":"…/pid-project.json","revision":4,"in_effect":true}`。§5.3 表里的 `source`、`revision` 就是这里的两个键。`path` 是项目文件的路径，写图内时是 `null`。`in_effect` 说写进去的这份是不是生效的：写进图内而目录里的项目文件有主数据时是 `false`，同命令的那条警告（§11.3 第 7 条）。
2. **`usage` 数什么。** 数 P&ID 对象，不数图元：有位号的按「类型 + 位号」算一个（一条管线画成几段，几个图元是同一个位号，算一个），没有位号的按图元算。管路、管段的属性是从上级抄下来的，没打覆盖标记的字段就是上级的值，不再算一次；打了标记的算它自己的。看四个字段：`管道等级`、`绝热等级`、`流体介质` 比名称，`公称直径` 读出 `dn` 再比，比法同 §3.5。键是条目名称，直径是 `dn` 的十进制写法；没人用的条目不列，四个键总在。只在 `state` 是 `configured` 时才算。
3. **HTTP 状态码。** 照 §3.6 的表；表里没有的 `MASTERDATA_IO_ERROR`（§11.2 新加的码）是 500：请求没错，是桌面端读写文件出了错。
4. **请求写错了。** `target` 不是 `project` / `drawing`、`expected_revision` 不是非负整数、图纸没存过却写 `project`：都报 `invalid_request`（400），§3.6 没有对应的码。缺 `master_data` 报校验的 `MASTERDATA_SCHEMA_ERROR`。`target` 不分大小写，同命令；MCP 的参数表只列小写的两个。
5. **只读。** 写操作被拒的条件同 `PIDMASTERDATA IMPORT`（`readonly_refuses_command`），码取会话的拒绝层级（§5.4），应答带 `level`，同别的被拒的写；也报给服务器。控制面没有统一的只读关口，写项目文件又在图纸外面，所以操作自己查（§11.3 第 8 条）。读不受限。
6. **名字和归类。** 能力名 `pid_master_data`。写操作能写后台标签里的图（同 `pid_insert_batch`）。MCP 里写操作可以放进批处理。
7. **起始页。** 没有打开的图纸时，两个操作都报 `no_document`。

**测试**（`pidmasterdata.rs` 4 条，都以 `pid_masterdata` 开头；`master_data.rs` 1 条；`rest.rs` 新加 1 条、2 条加了断言；`mcp.rs` 1 条）：

- 读：三种 `state`；项目文件坏了而图内有一份时 `shadowed_in_drawing` 为真、不拿图内那份顶替；不带 `usage` 时没有它。
- 使用计数：操作层，表里没有的取值不列，`usage: false` 时没有；函数层，一条管线画成两段算一个，管路抄来的不算，管段改过的直径算，名称带空白、直径写法不同都认，大小写不同、隐藏、读不出的不算，没有 P&ID 记录的图元不算，空图四个键都是空对象。
- 写：不写目标时写项目文件；修订号对上才写，对不上报 `MASTERDATA_CONFLICT`、文件逐字节不变；写图内是一个撤销步、`in_effect` 为假，对不上时撤销步不留；重名、缺数据、目标或修订号写错都不写；没存过的图写 `project` 报 `invalid_request`。
- 只读会话：三种目标都报 `COLLAB_READONLY`、`level` 是 `state`，什么也没写，服务器收到了这次尝试；读照常。
- REST：读写一轮（含 `usage=true`），409 两种、400 两种，被拒的都没写；状态码的映射；OpenAPI 有这条路径和写的 403、409、500。
- MCP：两个名单里有这两个操作，读的参数有 `usage`；写的参数校验收对的，拒缺 `master_data`、`target` 写错、多了不认识的键。

先放空壳（读只回 `{"ok":false}`，写什么也不做，`usage` 返回空；路由、状态码、OpenAPI、MCP 名单还没加），7 条新测试和 2 条加了断言的测试编得过、都败在断言上；实现后全过。工具面摘要那条在实现后败，核对过变的只是这两个操作和 `usage`，换成新值。
