# SPEC：发布包（`pid-publish`）

> 状态：**已建成；规格已确认**（2026-11-16，§9 的待决问题都按「建议」定；2026-11-22 由 `PUB-1` – `PUB-7` 建成，实现与本文的出入记在 §11）。基线 `main @ 9b5327f96`。
> 依据：已批准计划 T3.3 第 3 项；需求 `REQ-DELV-008`、`009`，参考 `006`、`010`、`011`（`docs/pid-platform/requirements/60-deliverables-and-publish.md`）。
> 共用约定见 `01-shared-conventions.md`；用到 `SPEC-pid-project-index.md`、`SPEC-pid-drc.md`、`SPEC-pid-bom.md`、`SPEC-pid-snapshot.md`、`SPEC-pid-master-data.md`。
> 2026-11-17：发布加三项协同前提（队列里有没提交的修改、标着不一致、没对过账），清单加一段 `collab`，写在 `SPEC-collab-offline-sync.md` §5.7，**已确认**（2026-11-17；定为默认拒绝，明确带上「允许未对账」才放行，见该规格 §10 第 3 条）。受影响的地方各加了一句指向它（都以「2026-11-17」开头）。

## 1. 目标

把当前这张图打成一个可以交出去的目录：图纸文件、出图、材料清单、连接关系、主数据、检查报告，外加一份带校验和的清单。两条硬规矩：

1. **有问题就不出包。** 有未豁免的错误级检查结果、或有重复位号，一个文件都不写。
2. **要么完整，要么没有。** 包先在临时目录里写齐、自检通过，再改名到位；中途失败不留半成品。

**发布不改图。** 不往图里写记录，不保存图纸；包里的图纸文件就是磁盘上那份文件的原样复制。

## 2. 现状

- 没有发布功能。
- 出图有现成的 `plot` 操作（`control_plot`，`src/app/control/actions.rs:607`）：对当前标签页同步写出 PDF 或 SVG，按扩展名定格式，可选 `layout`、`paper`、`orientation`、`scale` 等参数；默认模型空间、按图形范围、布满图纸。
- 标签页记着文件路径 `current_path`、有没有未保存修改 `dirty`、以及上次读写时的文件指纹 `disk_fingerprint`（`src/app/tab_document.rs:145`–`163`）。保存时用这个指纹发现「文件被别的程序改过」。
- 协同服务器的权限表把「发布签核」归为写操作，只读状态下拒绝（`SPEC-collab-server.md`）。

## 3. 数据格式

### 3.1 包的目录

```text
<目标目录>/                    ← 默认 <图纸所在目录>/pid-publish/
  PID-1101-r003/               ← 包名：<图号>-r<三位修订号>
    manifest.json              ← 发布清单
    PID-1101.dwg               ← 图纸文件，源文件的原样复制（扩展名跟源文件）
    PID-1101.pdf               ← 出图
    PID-1101.svg               ← 出图
    bom.csv                    ← 材料清单主表
    bom.json                   ← 材料清单（含副表）
    pipe-summary.csv           ← 管道汇总（材料清单的副表）
    topology.json              ← 连接关系
    snapshot.json              ← 发布时的快照
    master-data.json           ← 项目主数据
    drc-report.json            ← 规则检查报告
```

十一个文件，对应需求要求的六类内容：

| 需求里的类别 | 包里的文件 | 来源 |
|---|---|---|
| 图纸文件 | `<图号>.dwg`、`<图号>.pdf`、`<图号>.svg` | 源文件复制；`plot` 操作出两次图 |
| 材料清单 | `bom.csv`、`bom.json` | `SPEC-pid-bom.md` §3.2、§3.4，范围是这一张图，从发布时的快照算 |
| 报表 | `pipe-summary.csv`、`topology.json`、`snapshot.json` | 材料清单副表；§3.3；`SPEC-pid-snapshot.md` §3.2 |
| 项目主数据快照 | `master-data.json` | 主数据全文；没配置时写 `{"version":1,"configured":false}` |
| 校验结果报告 | `drc-report.json` | 发布时现做的那次检查的完整结果（`SPEC-pid-drc.md` §3.6），含已豁免的 |
| 发布清单 | `manifest.json` | §3.2 |

- 包名里的图号按快照目录同样的规则去掉不能做文件名的字符。
- 修订号是正整数。不指定时取「目标目录里这张图已有的最大修订号 + 1」，没有就从 1 起；可以指定，但必须大于已有的最大值。包目录已存在时拒绝（`PUBLISH_TARGET_EXISTS`），**从不覆盖已发布的包**。
- 项目索引不扫子目录，所以包里的 DWG 不会被当成项目图纸。
- 包写成后，里面的文件设为只读。

### 3.2 发布清单 `manifest.json`

> 2026-11-17：清单另加一段 `collab`（图里没有同步记录时是 `null`），见 `SPEC-collab-offline-sync.md` §5.7，已确认。

```json
{
  "version": 1,
  "kind": "pid-publish-manifest",
  "package_id": "PID-1101-r003",
  "project": { "id": "CDU-01", "name": "常减压装置" },
  "drawing_id": "PID-1101",
  "revision": 3,
  "published_at": 1794800000,
  "published_by": "zhangsan",
  "generator": "OpenCADStudio 0.9.0",
  "source": { "file": "PID-1101.dwg", "bytes": 482133, "sha256": "…" },
  "snapshot_seq": 7,
  "snapshot_body_sha256": "…",
  "rule_set_version": "1.0.0+2",
  "rule_set_digest": "a1b2c3d4e5f60718",
  "master_data_revision": 3,
  "gates": {
    "drc_errors": 0, "drc_warnings": 4, "drc_hints": 2, "waived": 1,
    "duplicate_tags": 0, "master_data": "configured", "index_complete": true,
    "result": "放行"
  },
  "contents": [
    { "kind": "图纸文件", "path": "PID-1101.dwg", "bytes": 482133, "sha256": "…" },
    { "kind": "图纸文件", "path": "PID-1101.pdf", "bytes": 96410, "sha256": "…" },
    { "kind": "图纸文件", "path": "PID-1101.svg", "bytes": 201877, "sha256": "…" },
    { "kind": "材料清单", "path": "bom.csv", "bytes": 2210, "sha256": "…" },
    { "kind": "材料清单", "path": "bom.json", "bytes": 9034, "sha256": "…" },
    { "kind": "报表", "path": "pipe-summary.csv", "bytes": 310, "sha256": "…" },
    { "kind": "报表", "path": "topology.json", "bytes": 15220, "sha256": "…" },
    { "kind": "报表", "path": "snapshot.json", "bytes": 512044, "sha256": "…" },
    { "kind": "项目主数据快照", "path": "master-data.json", "bytes": 4102, "sha256": "…" },
    { "kind": "校验结果报告", "path": "drc-report.json", "bytes": 7710, "sha256": "…" }
  ]
}
```

| 字段 | 说明 |
|---|---|
| `package_id` | 包名，与目录名相同 |
| `project` | 项目文件里的项目号与名称；没有项目文件时为 `null` |
| `published_at`、`published_by` | Unix 秒；发布人的取法与快照的生成人相同 |
| `source` | 被复制的那个图纸文件：文件名、字节数、SHA-256。与 `contents` 里图纸文件那一项的校验和相同——以后拿任何一份 DWG 算一下校验和，就能判断它是不是这次发布的那一份 |
| `snapshot_seq`、`snapshot_body_sha256` | 发布时存的那份快照的序号和正文校验和 |
| `rule_set_version`、`rule_set_digest` | 检查用的规则集版本与摘要值（`SPEC-pid-drc.md` §3.2） |
| `master_data_revision` | 主数据的修订号；没配置时为 `null` |
| `gates` | 门禁的计数和结论，见 §3.4。写进包里的 `result` 恒为 `放行`（不放行就没有包） |
| `contents[]` | 除清单自己以外的每个文件：类别、相对路径、字节数、SHA-256。按上表的顺序 |

### 3.3 连接关系 `topology.json`

```json
{
  "version": 1,
  "drawing_id": "PID-1101",
  "nodes": ["PIPE_LINE|L-1001", "PIPE_RUN|L-1001-01", "PUMP|P-1001", "VALVE|V-1001", "connector|L-1001|pipe_out"],
  "edges": [["PIPE_LINE|L-1001", "PIPE_RUN|L-1001-01"], ["PIPE_RUN|L-1001-01", "PUMP|P-1001#out1"]],
  "connectors": [
    { "kind": "pipe_out", "line": "L-1001", "status": "paired",
      "peer_drawing": "PID-1102", "upstream": "L-1001", "downstream": "L-2001" }
  ]
}
```

`nodes`、`edges` 就是连通图（`SPEC-pid-project-index.md` §3.3），与 `snapshot.json` 里的 `graph` 相同，单独成一个文件是为了下游不必解析整份快照。`connectors[].status` 是项目索引给的配对结论：`paired` / `one_sided` / `target_missing` / `ambiguous` / `unpaired`。

### 3.4 前提与门禁

**前提**（不满足直接拒绝，不做检查）：

| 前提 | 不满足时 |
|---|---|
| 图纸保存过，且没有未保存的修改 | `PUBLISH_UNSAVED` |
| 磁盘上的文件就是本程序上次读写的那一份（现取的文件指纹与标签页记的 `disk_fingerprint` 相同；标签页没记到指纹时按「无法确认」处理） | `PUBLISH_FILE_CHANGED` |
| 不是协同的只读会话 | `COLLAB_READONLY` |
| 2026-11-17 加，已确认：图里有同步记录时，队列是空的、没有标不一致、正连着服务器并且已经补齐（`SPEC-collab-offline-sync.md` §5.7） | `COLLAB_UNSYNCED` / `COLLAB_DIVERGED` / `COLLAB_OFFLINE` |
| 目标目录能建、能写；包目录不存在；指定的修订号合法 | `PUBLISH_TARGET_INVALID` / `PUBLISH_TARGET_EXISTS` / `PUBLISH_REVISION_INVALID` |

要求「先保存」是为了让包里的图纸文件、清单、报告说的是同一份东西：图纸文件从磁盘复制，其余从程序里的图算，两者一致的保证就是「没有未保存的修改，文件也没被别人动过」。

**门禁**（四项全过才出包）：

| 门禁 | 怎么算 |
|---|---|
| 未豁免的错误为 0 | **现做一次**规则检查：范围是当前图，项目级规则也跑（`scope: drawing`、`cross_drawing: true`、触发时机 `发布前`）。不采信之前的检查结果。警告、提示、已豁免的不拦，但都写进报告 |
| 重复位号为 0 | **不经过规则**，直接数：本图里同一个（类型编码，位号）出现两次以上，或本图的某个（类型编码，位号）在项目的另一张图上也有。只算有自己编号的设备、阀门、仪表、其他。项目规则包把 `DR-004`、`DR-102` 停用了也照拦 |
| 主数据读得出 | 配了但读不出（文件坏了）就拦，因为此时取值合法性的规则都没跑。没配置不单独拦，由 `DR-009` 管 |
| 项目里的图都读得出 | 有图读不出来时，「别的图上有没有同号」无法确认，拦（待决问题 P3） |

任一项不过：返回 `PUBLISH_BLOCKED`，应答带 `gates`（各项计数）和完整的检查报告；目标目录里一个文件都不写，也不存快照。

```json
{
  "ok": false, "code": "PUBLISH_BLOCKED", "message": "2 条错误、1 个重复位号，没有发布",
  "gates": { "drc_errors": 2, "drc_warnings": 4, "drc_hints": 2, "waived": 1,
             "duplicate_tags": 1, "master_data": "configured", "index_complete": true,
             "result": "PUBLISH_BLOCKED" },
  "duplicates": [ { "type_code": "PUMP", "tag": "P-1001",
                    "places": [ { "drawing_id": "PID-1101", "at": [120.0, 80.0] }, { "drawing_id": "PID-1102", "at": [40.0, 60.0] } ] } ],
  "report": { "说明": "SPEC-pid-drc.md §3.6 的检查结果全文" }
}
```

`gates.master_data` 取 `configured` / `not_configured` / `unreadable`。

### 3.5 校验一个包

`verify_package(目录)` 检查：清单能解析；六类内容每类至少一个文件；清单里列的每个文件都在，字节数和校验和都对。

```json
{ "ok": true, "package_id": "PID-1101-r003", "revision": 3, "files": 11,
  "missing": [], "mismatched": [], "extra": [] }
```

`files` 是包里的文件数，含清单自己。

- 缺类别或缺文件 → `PACKAGE_INCOMPLETE`，应答列出 `missing`。
- 校验和或字节数不对 → `PACKAGE_CORRUPT`，应答列出 `mismatched`。
- 包里多出清单没列的文件：不算失败，列在 `extra` 里。

发布时在改名到位之前先对临时目录跑一遍这个校验。

### 3.6 错误码

| 码 | 何时 | HTTP |
|---|---|---|
| `PUBLISH_BLOCKED` | 门禁没过（§3.4） | 409 |
| `PUBLISH_UNSAVED` | 图没保存过，或有未保存的修改 | 409 |
| `PUBLISH_FILE_CHANGED` | 磁盘上的文件被别的程序改过 | 409 |
| `COLLAB_UNSYNCED`、`COLLAB_DIVERGED`、`COLLAB_OFFLINE` | 2026-11-17 加，已确认：协同前提不满足（`SPEC-collab-offline-sync.md` §5.7） | 409 |
| `PUBLISH_TARGET_EXISTS` | 包目录已存在 | 409 |
| `PUBLISH_REVISION_INVALID` | 指定的修订号不是正整数，或不大于已有的最大修订号 | 400 |
| `PUBLISH_TARGET_INVALID` | 目标目录建不出来或不可写 | 400 |
| `PUBLISH_FAILED` | 出包过程中某一步失败（出图、写文件、自检）；应答带 `step`；临时目录已删 | 500 |
| `COLLAB_READONLY` | 只读会话（沿用现有的码） | 403 |
| `PACKAGE_NOT_FOUND` | 要校验的目录不存在，或里面没有 `manifest.json` | 404 |
| `PACKAGE_INCOMPLETE`、`PACKAGE_CORRUPT` | 见 §3.5 | 409 |
| `INDEX_DRAWING_ID_DUPLICATED` | 项目里两个文件图号相同（项目索引报的，原样透出） | 409 |

## 4. 数据怎么传

```mermaid
flowchart TD
    A["PIDPUBLISH / pid_publish"] --> B{"前提：已保存、无未存修改、<br/>文件没被别人改、不是只读会话"}
    B -- "不满足" --> E1["PUBLISH_UNSAVED 等<br/>什么都不写"]
    B -- "满足" --> C["做当前图摘要<br/>扫描项目建索引"]
    C --> D["现做一次规则检查（发布前）<br/>数重复位号，读主数据"]
    D --> G{"门禁四项"}
    G -- "任一不过" --> E2["PUBLISH_BLOCKED + 检查报告<br/>什么都不写"]
    G -- "全过" --> S["存一份快照（序号 N）"]
    S --> T["在目标目录里建临时目录<br/>复制图纸文件、出两次图、写其余文件、写清单"]
    T --> V{"自检：六类齐全，校验和都对"}
    V -- "失败" --> E3["删临时目录<br/>PUBLISH_FAILED"]
    V -- "通过" --> R["临时目录改名为 图号-r修订号<br/>文件设为只读"]
```

- **顺序与来源**：摘要做一次，检查、快照、材料清单、连接关系都用这同一份摘要，互相对得上。
- **临时目录**建在目标目录里面（名字以 `.` 开头、带随机后缀），与最终位置在同一个盘上，改名是一步到位的。任何一步失败就删掉它。上次崩溃留下的临时目录，在下次往同一目标目录发布时清掉。
- **快照**在门禁通过后才存，原因写「发布前自动快照（修订 3）」。之后出包失败的话快照留着，它如实记了那一刻的图。
- **项目扫描**可能要几秒：命令会先扫、扫完自动接着做；操作在扫描期间返回 `{"ok":true,"status":"scanning","done":12,"total":40}`，调用方稍后用同样的请求再发一次（与 `pid_check` 相同）。
- **不改图**：发布前后图纸的「已修改」状态、撤销栈、磁盘上的文件都不变。发布不经协同服务器。（2026-11-17：仍然不经服务器；协同前提只读本机的同步状态来判，见 `SPEC-collab-offline-sync.md` §5.7，已确认。）

**这张图发布到哪一版了**：不在图里记，由目标目录里的包推出。取这张图修订号最大的包，比它的 `source.sha256` 与当前图纸文件的校验和：

| 情况 | 显示 |
|---|---|
| 目标目录里没有这张图的包 | 未发布 |
| 校验和相同，且没有未保存的修改 | 已发布，修订 N |
| 其余 | 修订 N 之后有改动 |

## 5. 对外接口

### 5.1 模块接口（`src/pid/publish_pkg.rs`）

```rust
pub fn package_name(drawing_id: &str, revision: u32) -> String;                  // "PID-1101-r003"
pub fn next_revision(target_dir: &Path, drawing_id: &str) -> u32;
pub fn duplicate_tags(current: &DrawingSummary, index: &ProjectIndex,
                      others: &[DrawingSummary]) -> Vec<Duplicate>;
pub fn gates(report: &CheckReport, duplicates: usize, master: &Resolved, index_complete: bool) -> Gates;

pub struct PackageInput<'a> { /* 图号、修订号、项目、时间、发布人、源文件路径、快照文件与序号、
                                 材料清单、连通图与连接符状态、主数据、检查报告、门禁 */ }
/// 往 staging 里写除两张出图以外的文件（出图由调用方先放进去），最后写 manifest.json。
pub fn write_package(staging: &Path, input: &PackageInput) -> Result<Manifest, PublishError>;
/// 自检后改名到位；目标已存在则拒绝。
pub fn commit(staging: &Path, final_dir: &Path) -> Result<(), PublishError>;
pub fn verify_package(dir: &Path) -> Result<VerifyReport, PublishError>;
pub fn list_packages(target_dir: &Path, drawing_id: &str) -> Vec<PackageInfo>;
```

流程编排（前提、取摘要、扫描、检查、出图、存快照）在 `src/app/commands/pidpublish.rs`，因为出图和标签页状态只有应用层拿得到。

### 5.2 命令

| 命令 | 做什么 | 只读会话 |
|---|---|---|
| `PIDPUBLISH [<目标目录>] [REV <修订号>]` | 发布当前图。成功输出包路径、修订号、文件数；被拦输出各项门禁的计数和前若干条错误 | 拒绝 |
| `PIDPUBLISH CHECK` | 只过一遍前提和门禁，不写任何东西 | 允许 |
| `PIDPUBLISH LIST [<目标目录>]` | 列出这张图已发布的各版，以及「发布到哪一版了」 | 允许 |
| `PIDPUBLISH VERIFY <包目录>` | 校验一个包（§3.5） | 允许 |

### 5.3 自动化操作与 REST

| 操作 | REST | 读写 | 请求 | 应答 |
|---|---|---|---|---|
| `pid_publish_check` | `GET /api/v1/pid/publish/check` | 读 | `dir`（可选） | 见下 |
| `pid_publish` | `POST /api/v1/pid/publish` | 写（本机文件） | `dir`、`revision`、`plot`（都可选） | `{"ok":true,"package_id":"PID-1101-r003","path":"…","revision":3,"snapshot_seq":7,"files":11,"bytes":1331200,"gates":{…}}`，状态 201 |
| `pid_publications` | `GET /api/v1/pid/publications` | 读 | `dir`（可选） | `{"ok":true,"target_dir":"…","state":"修订 3 之后有改动","packages":[{package_id, revision, path, published_at, published_by, current}]}` |
| `pid_publish_verify` | `POST /api/v1/pid/publish/verify` | 读 | `path` | §3.5 |

`pid_publish_check` 的应答：

```json
{
  "ok": true, "can_publish": false,
  "preconditions": { "saved": true, "clean": false, "file_unchanged": true, "writable_session": true },
  "gates": { "drc_errors": 0, "drc_warnings": 4, "drc_hints": 2, "waived": 1,
             "duplicate_tags": 0, "master_data": "configured", "index_complete": true, "result": "放行" },
  "target_dir": "D:/proj/cdu/pid-publish", "next_revision": 3
}
```

- `plot`：可选对象，字段与现有 `plot` 操作相同（`layout`、`paper`、`orientation`、`scale` 等，不含 `path`），PDF 和 SVG 都用它。不给就是模型空间、按图形范围、布满图纸。本期一次只出一个布局。
- MCP：`pid_publish_check`、`pid_publications`、`pid_publish_verify` 进 `READ_OPS`；`pid_publish` 进 `EXECUTE_OPS`。

### 5.4 权限

没连协同不设限。协同中：可写会话可以发布，只读会话不能（与服务器把「发布签核」算作写操作一致）。客户端只知道会话可写不可写，不知道自己有没有「发布签核」这一项，所以本期做不到按角色细分。（2026-11-17：图里有同步记录而没连着服务器时，默认不让发布，明确带上「允许未对账」才放行，见 `SPEC-collab-offline-sync.md` §5.7，已确认。）

## 6. 成功标准

1. 图上有一条未豁免的错误 → `PUBLISH_BLOCKED`，目标目录里没有新增任何文件或目录，也没有新快照。用一组各带不同错误的图验证：**有错误的图 100% 被拦**。
2. 图上有重复位号，即使项目规则包停用了 `DR-004` → 仍是 `PUBLISH_BLOCKED`。另一张图上有同类型同位号 → 同样被拦。
3. 把错误改掉（或豁免）后再发布 → 成功；包里恰好十一个文件，`PIDPUBLISH VERIFY` 通过；警告和已豁免的都在 `drc-report.json` 里。
4. 包里的图纸文件与源文件逐字节相同；`bom.json` 的 `snapshot_seq` 等于清单里的 `snapshot_seq`；`topology.json` 的边与 `snapshot.json` 里的 `graph.edges` 相同。
5. 有未保存的修改 → `PUBLISH_UNSAVED`；只读会话 → `COLLAB_READONLY`；都不写任何东西。
6. 连发两次 → 修订号 1、2，两个包都在；往已有的包目录发布 → `PUBLISH_TARGET_EXISTS`，原包不变。
7. 出包中途失败（测试里让某一步报错）→ `PUBLISH_FAILED`，目标目录里既没有包也没有残留的临时目录。
8. 改动包里任一文件的一个字节 → `PACKAGE_CORRUPT`；删掉任一文件 → `PACKAGE_INCOMPLETE`。
9. 发布前后，图纸的「已修改」状态、撤销栈、源文件的校验和都没有变化；发布后 `PIDPUBLISH LIST` 显示「已发布，修订 N」，改图并保存后显示「修订 N 之后有改动」。
10. 性能：2,000 个对象的图，门禁部分（不含项目扫描）不超过 1 秒。整次发布不设硬指标（耗时主要在出图），应答里带各步耗时。
11. 从命令行执行 `PIDPUBLISH <目录>` 只发布一次、只发到给定的目录：默认目录里没有多出一个包（命令行不会先拿不带参数的 `PIDPUBLISH` 试跑一遍，见共用约定 §7.6）。

## 7. 测试

| 层 | 测什么 |
|---|---|
| 单元（`src/pid/publish_pkg.rs`） | 包名与修订号推算；重复位号的数法（本图内、跨图、不同类型同号不算、没有自己编号的不算）；门禁四项的组合；清单内容与顺序；校验的三种结果；改名前目标已存在；失败后不留临时目录 |
| 命令与操作（临时目录里的真实图纸） | 成功标准 1–9 的全流程：用命令建图 → 保存 → 被拦 → 整改 → 发布 → 校验；另一张图制造跨图重号；成功标准 11（带目录的命令经命令行只发布一次） |
| 一致性 | 图保存后重新读回，摘要与保存前相同（这是「从磁盘复制的图纸文件」与「从程序里算的清单」说的是同一份东西的依据） |
| REST | 四个路由、状态码（`PUBLISH_BLOCKED` → 409，`PUBLISH_FAILED` → 500）、`rest_openapi.json` 一致、MCP 名单与摘要值 |

要动的文件：新建 `src/pid/publish_pkg.rs`、`src/app/commands/pidpublish.rs`；改 `src/pid/mod.rs`、`src/app/commands/mod.rs`、`src/app/command_driver/utilities.rs`（整行名单）、`src/app/collab/apply.rs`、`src/app/control/mod.rs`、`src/rest.rs`（含 `map_status`）、`src/rest_openapi.json`、`src/mcp.rs`。

已核对的两个接入点：`disk_fingerprint` 在打开文件后（`src/app/update/file.rs:2188`）和保存后（`2280`）赋值，新建的图为空；`control_plot` 是同步写文件的，它设置的操作应答只在有操作在等时生效（`set_control_result`），发布在它之后写自己的应答即可，所以可以直接调用。

## 8. 不在范围与已知限制

- **发布状态流转**（草稿、待审、已发布、已撤回，`STATE_FORBIDDEN`）和**发布后图纸转只读**不做（P1）。
- 一次只发布当前这一张图（P2）。
- 包里不含 DXF；要 DXF 用现有的另存。
- 导出任务排队、失败重试、完成通知（`REQ-DELV-011`）不做：那是平台服务端的事，桌面端的发布是当场做完的。
- 按复选框树选范围批量出图、输出到项目文档库（`REQ-DELV-007`）不做。
- 修订号是按目标目录推的：换一个空的目标目录，编号会从 1 重新开始，除非用 `REV` 指定。
- 项目规则包可以停用规则，停用的规则不会拦发布（重复位号除外）。发布清单记了规则集版本和摘要值，检查报告记了跑了多少条、跳过多少条，可以事后核对。

## 9. 待决问题

> **2026-11-16 已定**：本节各条都按「建议」一栏执行。汇总见 `plan.md` §3，对应的任务见 `todo.md`。

| # | 问题 | 建议 |
|---|---|---|
| P1 | **发布状态和发布后锁图不做。** 需求要四个状态和「已发布后图纸只读、改动走新修订」；计划里没有 | 本期不做。状态流转和锁定要有人审、有地方记，适合放在协同服务器上（它已经有图纸状态和「发布签核」权限项）。本期用「已发布，修订 N / 修订 N 之后有改动」这个推出来的状态代替 |
| P2 | **一次只发布一张图。** 整个项目或一个 PBS 节点一起发布没做 | 本期只做单图。批量发布就是逐张调用，可以以后加，不影响包的格式 |
| P3 | **项目目录里有图读不出来时不让发布。** 这样最稳，但一个无关的坏文件会挡住所有图的发布 | 拦。读不出来就确认不了「别的图上没有同号」。报错里列出是哪个文件，把它移出目录即可 |
| P4 | **发布前必须先保存。** 另一种做法是不要求保存、直接把程序里的图另存进包里 | 要求先保存。这样包里的图纸文件与工作文件校验和相同，以后能证明「这个包就是从这份文件出的」；发布也完全不碰图 |
| P5 | **包内文件比计划多三个**（`pipe-summary.csv`、`snapshot.json`、`master-data.json`），目录是平铺的 | 按 §3.1。需求要求六类内容，计划列的八个文件里没有项目主数据快照；另外两个是顺手带上的，下游用得到 |

## 10. 需求对照

| 需求 | 本期 | 说明 |
|---|---|---|
| `REQ-DELV-008` 发布包构成与发布状态 | 部分 | 六类内容、`PACKAGE_INCOMPLETE`、修订号递增已做；四个状态、`STATE_FORBIDDEN`、发布后只读不做（P1） |
| `REQ-DELV-009` 发布门禁与校验阻断 | 覆盖 | 有错误不出包、只有警告和提示放行、已豁免不拦但列入报告。需求说读「最近一次」检查结果，本规格更严：每次发布都重新检查 |
| `REQ-DELV-006` 四格式图纸输出 | 部分 | 包里有 DWG、PDF、SVG；DXF 用现有的另存，不进包 |
| `REQ-DELV-010` 导出矩阵 | 部分 | 包里用到 DWG、PDF、SVG、CSV、JSON；XLSX 在材料清单的单独导出里 |
| `REQ-DELV-007`、`011` | 不做 | 见 §8 |
| 计划 T3.3「前置执行 DRC Blocker == 0 与 重复位号 == 0 硬门禁」 | 覆盖 | §3.4 前两项；另加两项（主数据读得出、项目里的图都读得出），否则前两项的结论不可靠 |
| 计划 T3.3「在目标目录原子生成标准发布包」 | 覆盖 | 临时目录 + 自检 + 改名；计划列的八个文件都在，另多三个（P5） |
| 计划 T3.3 测试「严重违规时 100% 拒绝写出发布包、整改清零后完整产出发布包」 | 覆盖 | 成功标准 1–3 |

## 11. 实现记录

实现与本规格、任务文字有出入的地方，以及规格没写到、实现时定下的读法，记在这里（`todo.md` §1 第 7 条）。

### 11.1 `PUB-1`：包名、修订号、重复位号、门禁（2026-11-22）

**做了什么**（新文件 `src/pid/publish_pkg.rs`，`src/pid/mod.rs` 登记一行）：

- §5.1 的 `package_name`、`next_revision`、`list_packages`（每项是 `PackageInfo`：包名、修订号、路径）、`duplicate_tags`、`gates`（结果 `Gates`，字段和顺序照 §5.3 的 `gates`，另有 `passed()`；主数据那一项是 `MasterDataState`，由 `master_data::Resolved` 转来）。
- 离线规格 §5.7 的协同前提 `collab_preconditions`：输入 `CollabInput`（图里有没有同步记录、队列条数、有没有标不一致或这次连接对不上、是不是连着并且补齐、有没有带「允许未对账」），结论 `CollabVerdict` 三种：不查（没有同步记录，清单的 `collab` 写 `null`）、放行（带 `reconciled`）、拦（带码）。
- 常量 `PASSED`（`放行`）、`PUBLISH_BLOCKED`、`COLLAB_UNSYNCED`、`COLLAB_DIVERGED`、`COLLAB_OFFLINE`。写包、落位、校验是 `PUB-2` 的。

**与规格写法不同的：**

1. **重复位号沿用项目索引的 `Duplicate`。** 出现处除了 §3.4 写的 `drawing_id`、`at`，还多一个 `handle`（与项目索引的 `duplicates[].places[]` 相同），在图上定位用。

**规格没写到、这里定下的：**

1. **`duplicate_tags` 的三个参数各管什么**：本图以 `current` 为准；别的图上的出现处从项目索引 `index` 的 `duplicates` 和别的图的摘要 `others` 里都找，两处都有的同一处（图号、句柄都相同）只记一次；索引里图号与本图相同的出现处不看，因为索引可能是本图改动之前建的。所以只给索引、或者只给别的图的摘要，跨图重号都数得出来。
2. **位号去掉首尾空白再比**，只有空白的不数，与规则检查数本图重号的口径相同；列出来的是去掉空白以后的位号。项目索引的跨图重号按原样比，这里拿索引里的位号来对时也先去掉空白。
3. **包名的换字符**直接调快照目录的 `dir_for` 取最后一段，同一条规则只有一份，没有改快照模块。所以设备名（`CON` 之类）前面也加 `_`，包名其实用不着，但无害。修订号不足三位补零，超过三位照写（`r1234`）。
4. **哪些目录算这张图的包**：名字恰好等于 `package_name(图号, n)` 的目录，n 从 1 起。`r08`、`r0005`、`r+06`、`r000` 这样的写法不算，同名的文件不算，临时目录不算，别的图不算（图号是本图图号加上 `-r001` 的图也分得开）。目标目录不存在或读不了时列表是空的、`next_revision` 是 1，目录本身有问题留给写包那一步报 `PUBLISH_TARGET_INVALID`。修订号到了 `u32` 的上限不再加一。
5. **门禁**：未豁免的错误数取检查报告的 `summary.错误`（检查报告的 `blocked` 与它同真假）；`result` 只有 `放行` 和 `PUBLISH_BLOCKED` 两个值。
6. **协同前提的顺序**照离线规格 §5.7 表里的顺序查：队列、不一致、连接，几条同时不满足时报排在前面的那条。「允许未对账」只放过最后一条；连着并补齐时带不带它都是 `reconciled: true`。

### 11.2 `PUB-2`：写包、落位、校验（2026-11-22）

**做了什么**（只改 `src/pid/publish_pkg.rs`）：

- §5.1 的 `write_package`、`commit`、`verify_package`，以及它们用的 `PackageInput`、`Manifest`（字段和顺序照 §3.2）、`VerifyReport`（照 §3.5）、`PublishError`（`code()` 给错误码，`step()` 给 `PUBLISH_FAILED` 停在哪一步）。
- `plot_files(图号)`：两张出图在包里的名字（`<图号>.pdf`、`<图号>.svg`，图号换字符与包名同一规则），命令层照这两个名字往临时目录里出图。
- `topology(快照, 项目索引)`：`topology.json` 的内容（§3.3）。
- 常量 `MANIFEST_FILE`、`MANIFEST_KIND`、`MANIFEST_VERSION`、`TOPOLOGY_VERSION`，错误码 `PUBLISH_TARGET_EXISTS`、`PUBLISH_FAILED`、`PACKAGE_NOT_FOUND`、`PACKAGE_INCOMPLETE`、`PACKAGE_CORRUPT`。
- 清单的 `collab` 是 `CollabRecord`（图号、仓库号、修订、`reconciled`），由调用方从图里的同步记录和 `collab_preconditions` 的结论拼。

**与规格写法不同的：**

1. **`PackageInput` 收快照和项目索引，不收「连通图与连接符状态」**（§5.1 的注释）。连通图就在快照里，连接符状态由 `topology` 从索引查；这样 `topology.json` 与 `snapshot.json` 的 `graph` 出自同一个值，不会对不上。快照要已经存过（有序号），`snapshot.json` 就是快照目录里那份文件的字节。
2. **`bom.json`、`drc-report.json` 外面有一层 `ok: true`**，与 `PIDBOM EXPORT`、`PIDCHECK EXPORT` 导出的文件同一写法（缩进两格、末尾换行）；两份规格里的格式本身不带 `ok`。下游用读导出文件的那套办法就能读。
3. **`PUBLISH_FAILED` 的 `step` 多两个值**：§3.6 列了出图、写文件、自检（`plot`、`write`、`verify`），这里另有 `snapshot`（传进来的快照没有序号）和 `commit`（改名失败、目标也不存在）。命令层把 `step()` 原样放进应答的 `step`。

**规格没写到、这里定下的：**

1. **包里的图纸文件名**是「与包名同规则换过字符的图号 + 源文件的扩展名」（扩展名原样，不改大小写）；清单的 `source.file` 记源文件自己的文件名（§3.2「被复制的那个图纸文件」）。两个名字可以不同，校验和相同。
2. **`collab` 在清单里排在 `gates` 之后、`contents` 之前**；没有同步记录时写 `null`，`contents` 仍是十项。
3. **写包的顺序**：先查门禁（没过报 `PUBLISH_BLOCKED`）和快照序号，再读两张出图和源文件，都读得到才开始写；文件按 `contents` 的顺序写，清单最后写。前面几样不满足时临时目录里不多一个文件。写到一半失败、`commit` 失败时都不动临时目录，由调用方删（§4）。
4. **`topology.json` 的连接符**按快照摘要里的顺序；状态按句柄在项目索引里找这张图的那一个：在 `links` 里（不论是流出侧还是流入侧）是 `paired`，在 `unresolved` 里取那里的状态，都没有算 `unpaired`。索引里别的图上句柄相同的不算。
5. **`master-data.json`** 是生效主数据的全文（`MasterData::to_json()`），缩进两格加换行；没配置时是 `{"version": 1, "configured": false}`（同样缩进），清单的 `master_data_revision` 为 `null`。
6. **校验的细则**：
   - 目录不在、清单不在（或不是文件）→ `PACKAGE_NOT_FOUND`。
   - 清单解析不了、`kind` 不是 `pid-publish-manifest`、`version` 不是 1、`contents` 里有不是单段文件名的路径（带目录、`..`、空串、清单自己）→ `PACKAGE_CORRUPT`，不带 `missing`、`mismatched`。规格只说「清单能解析」，没说解析不了报哪个码。带目录的路径一律不认，校验不会去读包外的文件。
   - 清单里某一类一个文件都没有：把类别名（如 `项目主数据快照`）记进 `missing`。
   - 列的文件不在或是个目录记进 `missing`；字节数、校验和不对，或在却读不了，记进 `mismatched`。`missing` 不空就报 `PACKAGE_INCOMPLETE`（两样都有时也是它），否则 `mismatched` 不空报 `PACKAGE_CORRUPT`；错里带着完整的 `VerifyReport`。
   - `extra` 按名字排，子目录也列；`files` 只数文件，含清单。
   - `VerifyReport` 不带 `ok`，由应答那一层加（与检查报告、材料清单同一做法）。
   - 已知限制：清单自己没有校验和。只改清单里不影响解析、也不影响对账的字段（比如 `published_by`），校验查不出来。
7. **落位的细则**：目标已存在（目录、文件都算）→ `PUBLISH_TARGET_EXISTS`，先于自检；自检就是 `verify_package`，另要求清单的 `package_id` 与目标目录同名，否则也算自检不过（`PUBLISH_FAILED`、`verify`）；都过了才改名。改名失败时，目标已经有了就报 `PUBLISH_TARGET_EXISTS`，否则 `PUBLISH_FAILED`、`commit`。改名后把包里每个文件设为只读，设不成不报错，因为包已经落位了。

### 11.3 `PUB-3`：`PIDPUBLISH CHECK`，前提与门禁（2026-11-22）

**做了什么**（新文件 `src/app/commands/pidpublish.rs`；除了任务列的 `commands/mod.rs`、`command_driver/utilities.rs`、`collab/apply.rs`，还改了 `commands/piddrc.rs`、`commands/pidindex.rs`、`collab/unsynced.rs`）：

- 命令 `PIDPUBLISH [CHECK] [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]`。按 §3.4 的先后过前提；都满足后取规则和主数据，扫项目（要读盘的在后台读，读完接着做）；扫完再过一遍前提，然后在扫好的项目上算门禁。最后说出发到哪里、第几版、四项门禁的计数、前 10 条没豁免的错误、重复位号的各处、读不出来的图，以及能不能发布（不能时错误码 `PUBLISH_BLOCKED`）。`CHECK` 和发布在这一段做的事相同，都不写任何东西。
- `publish_gates(扫好的项目, 图号, 规则, 主数据)`：门禁这一段，不碰命令行和程序状态（2,000 个对象的计时量的就是它）。当前图的摘要就是这次扫描做的那一份，检查和数重复位号都用它。
- 错误码 `PUBLISH_UNSAVED`、`PUBLISH_FILE_CHANGED`、`PUBLISH_TARGET_INVALID`、`PUBLISH_REVISION_INVALID`；`PUBLISH_BLOCKED`、`PUBLISH_TARGET_EXISTS` 和三个协同码用 `publish_pkg` 的。
- `mod.rs` 登记模块、分派和命令表；`utilities.rs` 把 `PIDPUBLISH` 登记成整行命令；`apply.rs` 让只读会话按子命令词放行 `CHECK`、`LIST`、`VERIFY`，别的写法都算发布，在命令闸门拒（`COLLAB_READONLY`）。
- `unsynced.rs` 新加 `publish_collab_input`：从标签页和协同状态取 `collab_preconditions` 要的输入（有没有同步记录、队列条数、有没有标不一致或这次连接对不上、是不是连着并补齐）。「有没有同步记录」与同步状态判「离线」用同一个条件，抽成了 `has_record`。
- `piddrc.rs`：`Check` 带上触发时机，`PIDCHECK`、`pid_check` 是「按需」，新加的 `Check::before_publish` 是「发布前」；`check_project`、`pidcheck_load_rules`、`listed` 放开给发布用（任务说的「DRC-12 的入口函数」就是 `check_project`）。`pidindex.rs`：扫描排着的事多一种「扫完接着发布」。

**与规格、任务写法不同的：**

1. **发布的写法这一步还不出包。** 前提和门禁都过时报一行错误「出包还没接上，没有发布」，不带错误码，什么也不写；`PUB-4` 接上。`LIST`、`VERIFY` 这一步报「还没有这一项」，由 `PUB-5` 做。
2. **`CHECK` 也收目标目录、`REV` 和 `ALLOW_OFFLINE`。** §5.2 的 `PIDPUBLISH CHECK` 没写参数；这里与发布同一套写法，查的是「照这样发布能不能成」，目标目录和修订号也按写的算。`ALLOW_OFFLINE` 是离线规格 §5.7 定的写法，§5.2 的发布写法里还没有它。
3. **只读会话里 `CHECK` 照查门禁，最后说这个会话发布不了。** 门禁都过时，错误码是只读的码（`COLLAB_READONLY`）；门禁没过时仍是 `PUBLISH_BLOCKED`，前面加一行提示说会话只读。§3.4 把「不是只读会话」列为前提、不满足不做检查，而 §5.2 又说只读会话允许 `CHECK`；这里取后者。发布的写法在命令闸门就被拒，不扫项目也不做检查。
4. **前提多一条：当前图要在项目的图纸里。** 图号、项目目录都从这里来，门禁也要跨图查。项目只认目录里的 `.dwg`，不在里面的报 `INDEX_NO_PROJECT_DIR`；列图纸出错时报项目索引给的码，原样透出（与 §3.6 末行同一做法）。
5. **「目标目录能建、能写」只看路径。** 目标在的话要是目录；不在的话，往上找到的第一个在的要是目录。检查不写任何东西（§5.2），所以不建也不试写；建不出来、写不了的（权限、只读盘）留给 `PUB-4` 真去建临时目录时报。

**规格没写到、这里定下的：**

1. **前提的先后**：没保存过、有没保存的修改（`PUBLISH_UNSAVED`）→ 磁盘上的文件变了（`PUBLISH_FILE_CHANGED`）→ 协同前提 → 当前图在项目里 → 目标目录（`PUBLISH_TARGET_INVALID`）→ 修订号（`PUBLISH_REVISION_INVALID`）→ 这一版的包已经有了（`PUBLISH_TARGET_EXISTS`），报第一条不满足的。只读会话不在这个序列里（见上第 3 条）。
2. **文件变没变**：现取的指纹与标签页记的 `disk_fingerprint` 相等才算没变；没记到、取不到都算变了（§3.4「无法确认」）。网页版没有本机文件，一律算变了。
3. **扫完再过一遍前提。** 扫项目要读盘时在后台读，这期间图可能被改、被存、被关。扫完先按标签页的 `id` 找回那张图（关掉了就说一声），再从头过一遍前提，不满足就照前提报，不算门禁。规则和主数据仍用起头时取的那份（与 `PIDCHECK` 相同）。
4. **命令行的写法**：子命令词只认紧跟在 `PIDPUBLISH` 后面、没加引号的 `CHECK`、`LIST`、`VERIFY`；`REV`、`ALLOW_OFFLINE` 不分大小写、不加引号，先后不论；别的词是目标目录，加了双引号的一律算目录（目录名有空格就加引号）。`REV` 没写值或写了两次、`ALLOW_OFFLINE` 写了两次、目录写了两个或写成空串、引号没配对，都报写法错（不带错误码）。目标目录的相对路径从图纸所在目录算，不写时是 `pid-publish`。
5. **修订号**：`REV` 后面要是正整数（`u32`），且不小于下一版（已有的最大修订号加一），否则 `PUBLISH_REVISION_INVALID`；可以跳号（已有 r002 时写 `REV 7` 就是 r007）。
6. **命令行上说的**：一行「发到哪里、第几版」；按 `ALLOW_OFFLINE` 放行时一行提示；一行计数（错误、警告、提示、已豁免的条数，重复位号个数，主数据状态，项目里的图读没读全）；没豁免的错误按检查报告的顺序列前 10 条，多的说还有几条；每个重复位号一行，列出各处（图号和坐标）；读不出来的图各一行；最后一句是结论。`PIDCHECK` 记着的结果不用也不改。

### 11.4 `PUB-4`：`PIDPUBLISH` 出包（2026-11-22）

**做了什么**（`src/app/commands/pidpublish.rs`；`src/app/control/actions.rs` 拆出程序内部用的出图）：

- 发布的写法过完前提和门禁以后接着出包，照 §4：存发布前的快照 → 在目标目录里建临时目录 → 出 PDF 和 SVG → 从刚存的快照算材料清单 → `write_package` → `commit`（里面先自检，再改名、设只读）。成功时说包在哪里、几个文件、共多少字节、发布前的快照是第几份，再用一行说各步用时。失败时先说已做的几步的用时，再一行带错误码的错误；`PUBLISH_FAILED` 带上停在哪一步（`step …`）、为什么，以及留下了什么（临时目录已删，发布前的快照第 N 份留着）。
- `actions.rs`：`control_plot` 的主体拆成 `plot_current(请求)`。它返回 `plot` 操作的应答对象，不存成控制结果，也不给出图日期字段盖时间。`control_plot` 照旧先盖时间，再调它，再存结果，行为不变。判扩展名的一段抽成 `plot_target`。
- 测试钩子：测试里给一个线程局部变量设上步名（`snapshot`、`plot`、`bom`、`write`、`verify`、`commit`），走到那一步就失败。`verify` 是往临时目录里的 `bom.csv` 末尾多写一个换行，让 `commit` 的自检真的对不上。不是测试时，钩子什么都不做。

**与规格、任务写法不同的：**

1. **出图不盖出图日期。** 任务说用现有的出图操作 `control_plot` 出图。可它出图前会给图里的出图日期字段写上现在的时间，并把图标成已修改，还会把结果存成自动化的控制结果。发布不许改图，所以走拆出来的 `plot_current`：出图方式相同（模型空间、按图形范围、布满图纸，即 `plot` 操作的默认值），只是不盖时间、不碰控制结果。包里 PDF、SVG 上的出图日期字段显示的是图里原来存的值。
2. **`step` 多一个 `bom`。** 从快照算材料清单出错时（快照读不回来、主数据读不出来等）报 `step: "bom"`。§3.6 列了出图、写文件、自检，`PUB-2` 加了 `snapshot`、`commit`（§11.2 第 3 条）。
3. **临时目录建不了时报 `PUBLISH_TARGET_INVALID`，不带 `step`。** §3.6 给这个码的说明正是「目标目录建不出来或不可写」。这时快照已经存了，留着，错误里会说。`commit` 时包目录被别人抢先建了，报 `PUBLISH_TARGET_EXISTS`，同样删临时目录、留快照。
4. **失败时，这次为出包新建的目标目录也删掉。** 目标目录原来不在、是出包时建出来的（连同建出来的上级目录），失败后从里往外删，只删空的。验收只要求目标目录里不留包和临时目录；这里连空目录也不留，图纸目录看上去和发布前一样。
5. **临时目录的后缀是时间加进程号。** §4 说带随机后缀；仓库没有随机数库，照任务用纳秒数和进程号。

**规格没写到、这里定下的：**

1. **快照用哪份摘要**：用这次扫描做的当前图摘要（门禁查的就是这一份）和由它算的连通图，跟 `PIDSNAPSHOT` 存进同一个快照目录。原因写「发布前自动快照（修订 N）」，`dirty` 照标签页（前提已保证是假），作者与 `PIDCHECK` 的豁免人同一个来源。存好拿到序号后，按序号读回来交给写包，所以包里的 `snapshot.json` 与快照目录里那份逐字节相同。材料清单走 `PIDBOM` 从快照算的入口，`bom.json` 的来源是快照、`snapshot_seq` 就是这个序号。
2. **出图换页**：出图只认当前标签页，出包时临时把要发布的那一页设为当前页，出完换回来。整个过程在一次命令处理里做完，界面上看不到切换。命令一律用默认的出图方式；§5.3 `pid_publish` 的 `plot` 参数由 `PUB-6` 接。
3. **清残留**：临时目录叫 `.publishing-<包名>-<纳秒>-<进程号>`。清残留只认 `.publishing-` 开头的目录，不跟符号链接；删不掉的留着，下次再清。数修订号、列包只认恰好是包名写法的目录，看不到它。已知限制：要是另一个程序正往同一目标目录发布，它的临时目录也会被当成残留删掉，让它出包失败；一个人在一台机器上用时不会遇到。
4. **清单的 `project`**：取项目目录里项目文件 `pid-project.json` 的 `project.id`、`project.name`（主数据规格 §3.3）。没有这个文件、文件里没有 `project`、或读不出来时为 `null`；缺其中一项时，那一项是空串。
5. **清单的 `collab`**：协同前提放行时，取图里的同步记录（图号、仓库、修订号）加上放行方式：连着并补齐时 `reconciled` 为 `true`，带 `ALLOW_OFFLINE` 放行时为 `false`。图里没有同步记录时为 `null`。
6. **时间与出处**：`published_at` 和快照的 `created_at` 都是门禁算完、开始出包那一刻的 Unix 秒。`published_by` 与快照作者相同，`generator` 是「OpenCADStudio <版本>」。
7. **用时**按步记毫秒：快照、出图、材料清单、写包、自检落位，最后是合计。建临时目录（含清残留）的时间不单列。

### 11.5 `PUB-5`：`PIDPUBLISH LIST` / `VERIFY`，发布到哪一版了（2026-11-22）

**做了什么**（`src/app/commands/pidpublish.rs`；`src/app/commands/pidsnapshot.rs` 只放开了一个函数的可见性）：

- `PIDPUBLISH LIST [<目标目录>]`：不扫项目，只读目标目录里这张图的包。第一行说图号、发布到哪一版了（§4 末尾的表）、目标目录和里面有几个包；之后新的在前，一行一个包：包名、出包时间、发布人，与磁盘上的图纸文件一致的那个包标「当前」。目标目录里没有这张图的包时只有一行，说「未发布」。
- `publications(目标目录, 图号, 图纸文件, 有没有未保存的修改)`：推出状态和各包，不碰命令行和程序状态；`pidpublish_listed(标签页, 目录)` 从标签页取图纸文件、算出目标目录再调它。`PUB-6` 的 `pid_publications` 接这两个；应答里各包的 `path` 由目标目录和包名拼出。
- `PIDPUBLISH VERIFY <包目录>`：调 `verify_package`（§3.5）。通过时一行说包名、第几版、几个文件，清单没列的文件另起一行（不算失败）；不通过时是它的错误码（`PACKAGE_NOT_FOUND`、`PACKAGE_INCOMPLETE`、`PACKAGE_CORRUPT`）和原因。
- 两个子命令都不写任何东西。只读会话里本来就放行（`PUB-3` 在 `apply.rs` 按子命令词放行），闸门没改。
- 发布的前提里「项目目录、图号、目标目录」这一段抽成 `placed`，`LIST` 与发布共用，两者找的是同一个目标目录。

**与规格、任务写法不同的：**

1. **多改了 `pidsnapshot.rs` 的一处可见性。** 任务的「文件」只列了 `pidpublish.rs`。列表里的时间要与 `PIDSNAPSHOT LIST` 同一种写法，就把那边的 `utc_said` 改成 `pub(super)` 拿来用，函数体没动。

**规格没写到、这里定下的：**

1. **状态只看修订号最大的包**（§4 末尾的表）：它的清单里 `source.sha256` 与磁盘上图纸文件现在的校验和相同（不分大小写）、图也没有未保存的修改，是「已发布，修订 N」；否则是「修订 N 之后有改动」，包括图被改回了某个旧版的样子。
2. **「当前」标哪个**：源文件与磁盘上图纸文件一致、图也没有未保存的修改的包里，修订号最大的那一个；几版源文件相同（没改图连发几次）时只标最新的。状态是「已发布，修订 N」时它就是第 N 版；图改回旧版的样子时，状态是「之后有改动」，而旧的那版标「当前」。
3. **清单读不出来的包**：包是按目录名列出来的（`list_packages`），清单读不出来、不是 JSON 的照列，那一行写「发布清单读不出来」，不算一致；它要是修订号最大的，状态就是「之后有改动」。`LIST` 不校验包，要校验用 `VERIFY`。
4. **图没保存过时 `LIST` 报 `PUBLISH_UNSAVED`**：没有图纸文件，推不出图号和目标目录。§3.6 这个码本来是给发布的，这里沿用。图不在项目的图纸里时与发布一样报 `INDEX_NO_PROJECT_DIR`（图号从项目的图纸列表来）。有未保存的修改不算错，照表显示「之后有改动」。
5. **目录怎么算**：`LIST` 的目标目录与发布同一个算法：相对路径从图纸所在目录算，不写是 `pid-publish`；目录不在就是「未发布」，不建目录。`VERIFY` 的相对路径从图纸所在目录算，图没保存过时从程序的当前目录算，写完整路径也行。
6. **时间**：清单的 `published_at`（Unix 秒）写成 UTC 的日期和时刻，到分，标明 UTC，与快照列表相同（标准库拿不到本机时区）。
7. **命令行的写法**：子命令词不分大小写；`LIST` 后面至多一个目录，`VERIFY` 后面恰好一个，有空格的加双引号。目录写了两个、写成空串、引号没配对、`VERIFY` 没写目录，都报写法错（不带错误码）并说正确写法。

### 11.6 `PUB-6`：发布的四个自动化操作，REST 与 MCP（2026-11-22）

**做了什么**（`src/app/commands/pidpublish.rs`、`src/app/control/mod.rs`、`src/app/control/http_bridge.rs`、`src/rest.rs`、`src/rest_openapi.json`、`src/mcp.rs`、`src/mcp_ops.rs`）：

- `pid_publish_check`（查询，自动化停着也能用）：前提逐条照实答，不在第一条不满足的地方停下；再答四项门禁（同 `PIDPUBLISH CHECK`，现做一次检查）、目标目录和这次会用的修订号。要读的图缓存里没有时答 `scanning`（带 `done`、`total`）并在后台读，调用方稍后用同样的请求再发。
- `pid_publish`（写，本机文件）：同 `PIDPUBLISH`。`dir`、`rev`、`allow_offline` 就是命令行上的目标目录、`REV`、`ALLOW_OFFLINE`；`plot` 交给 PDF 和 SVG 的出图。请求、会话、前提、规则包和扫描都在进写操作流程之前的等待一步里看（同 `pid_bom_export`），扫描没做完时答 `scanning`，什么也没写。出包一段从命令里抽成 `pidpublish_pack`，命令与操作共用，命令行上的说法没变。不能放进 `batch`。
- `pid_publications`、`pid_publish_verify`（查询）：同 `PIDPUBLISH LIST`、`PIDPUBLISH VERIFY`，接 `PUB-5` 的 `pidpublish_listed` 和 `verify_package`。
- REST：§5.3 的四条路由和 OpenAPI。`map_status`：§3.6 里请求的问题是 400，只读会话 403，`PACKAGE_NOT_FOUND` 404；图和磁盘的状态不让发的——`PUBLISH_BLOCKED`、`PUBLISH_UNSAVED`、`PUBLISH_FILE_CHANGED`、`COLLAB_UNSYNCED`、`COLLAB_DIVERGED`、`COLLAB_OFFLINE`、`PUBLISH_TARGET_EXISTS`、`PACKAGE_INCOMPLETE`、`PACKAGE_CORRUPT`——是 409；`PUBLISH_FAILED` 500。
- MCP：三个查询进 `READ_OPS`，`ocs_read` 多了 `dir`、`rev`、`allow_offline` 三个参数，`path` 的说明加上 `pid_publish_verify`；`pid_publish` 进 `EXECUTE_OPS`。工具清单摘要已更新。

**与规格、任务写法不同的：**

1. **修订号的参数叫 `rev`**，不是 §5.3 表里的 `revision`：与命令行的 `REV` 一致，也不和应答里的 `revision`（这次用的修订号）混。
2. **`pid_publish` 的结果在 `result` 里**，不像 §5.3 表里那样平铺：控制协议的写操作都这样（`pid_snapshot_save`、`pid_bom_export` 也是），REST 照转。字段与表里的一致。
3. **失败的说明在 `error` 里**，不是 §3.4 例子里的 `message`：控制面的失败应答都是 `{"ok":false,"status":"failed","code":…,"error":…}`。
4. **`preconditions` 有七条**：§5.3 例子的四条之外，加上协同的 `collab_synced`、`collab_consistent`、`collab_online`（任务的验收要「这三条」），分别对应 `COLLAB_UNSYNCED`、`COLLAB_DIVERGED`、`COLLAB_OFFLINE`；图没有同步记录时三条都为真。`allow_offline` 写了 `true` 时 `collab_online` 仍照实答，但没连着不再挡 `can_publish`。
5. **扫描中不是 201**：`POST /api/v1/pid/publish` 在项目还在读时答 200 和 `scanning`，出了包才是 201。`map_status` 改成成功的应答只有不是 `scanning` 时才给 201，`rest.rs` 和控制层的 HTTP 桥两处写操作都改用它。
6. **只读会话里 `pid_publish` 自己拒绝**：自动化操作不走命令行，碰不到命令闸门，所以在操作里照闸门的做法查一次：答协同给的码（`COLLAB_READONLY` 等，403），带 `level`，也记下这次尝试。
7. **改的文件多了三个**：任务的「文件」一栏是四个，另外改了 `pidpublish.rs`（四个操作写在发布模块里）、`http_bridge.rs`（上一条的 201）、`mcp_ops.rs`（`pid_publish` 的定义，和下面第 10 条的说明压缩）。

**规格没写到、这里定下的：**

1. **图没保存过时的 `pid_publish_check`** 不报错：`saved` 为假，`can_publish` 为假，`gates`、`target_dir`、`next_revision` 都是 `null`——没有图纸文件就推不出图号和目标目录，门禁也无从算起。
2. **`can_publish`**：七条前提都放行（协同三条照 `collab_preconditions` 判，见上面第 4 条）并且门禁「放行」。**`next_revision`**：这次会用的修订号，写了 `rev` 就是它，不写是下一版。`dir` 写的目录用不了、`rev` 不对、这一版的包已经有了，`pid_publish_check` 与 `pid_publish` 一样答相应的错误码。
3. **`plot` 的限制**：要是对象，`null` 等于没写；不能有 `path`（文件名由包定）；`layout` 不能是 `all`（本期一次只出一个布局）；这几种都是 `invalid_request`，什么也没写。其余字段由出图判断，例如纸张不认得时出包在「出图」一步失败：`PUBLISH_FAILED`，`step` 是 `plot`。
4. **门禁没过**：`PUBLISH_BLOCKED` 的应答带 `gates`、`duplicates`（每个重复位号出现的各处）和 `report`（这次检查的报告），调用方不必再查一次。
5. **出包失败**：`PUBLISH_FAILED` 带 `step`；`error` 里还说留下了什么（快照那一步就失败时什么都没写；之后的步骤失败时临时目录已删、发布前的快照留着）。
6. **`pid_publications`** 的应答顶层多一个 `drawing_id`；各包的 `path` 由目标目录和包名拼出（§11.5）。
7. **`pid_publish_verify` 不通过时**也带 §3.5 报告的各字段（缺了哪些、哪些对不上），`error` 是去掉打头错误码的原因。相对路径的算法同 `VERIFY`（§11.5 第 5 条）。
8. **扫出的项目里找不到当前这张图**时答 `INDEX_IO_ERROR`（500），不发布。
9. **REST 查询参数**：`allow_offline` 的 `true`/`1`、`false`/`0` 换成布尔，别的值原样交给操作、答 `invalid_request`（400）；`rev` 不是整数时也原样交给操作，答 `PUBLISH_REVISION_INVALID`（400）。
10. **MCP 执行工具的说明压缩**：加上 `pid_publish` 后执行工具的 schema 是 50,736 字节，超过测试定的 50,000 上限。上限没放宽，把几条 P&ID 操作（BOM 导出、规则包、快照、定位、豁免、主数据、发布）和 `grip_drag`、`node_graph`、`pointer`、`click` 的说明写短，意思不变，省下 950 字节，现在约 49,786。

### 11.7 `PUB-7`：拦截率与一致性的验收测试（2026-11-22）

**做了什么**（新建 `src/app/commands/pidpublish_acceptance.rs`，由 `pidpublish.rs` 末尾的 `#[cfg(test)] #[path = …] mod acceptance;` 挂进去）：只加三条测试，程序没改。

- **拦截率**：错误级规则从内置规则表里取，去掉本期不出结果的 `DR-020`、`DR-109`，断言剩 18 条（表里增减了错误级规则，测试先败在这里）。每条规则用一个新的项目目录：项目规则包里错误级的只留这一条，照 DRC 验收里它命中的例子造一张图，经 `save` 操作存成 `PID-1101.dwg`，再发 `pid_publish`。三样都对才算拦下：答 `PUBLISH_BLOCKED`；报告里有这条规则没豁免的错误级结果；目录里的文件和大小与发布前一样。读数打在测试输出里：`PUB-7 blocked 18 / 18`。
- **重复位号**：本图两台泵同位号、另一张图上有同类型同位号的泵，各发布一次，都是 `PUBLISH_BLOCKED`：`gates` 里检查错误是 0、重复位号是 1，`duplicates` 列的两处图号对得上（本图的两处都是 `PID-1101`，跨图的是 `PID-1101`、`PID-1102`），目录里什么也没多。
- **一致性**：与项目索引往返测试（IX-4）同一张小图，经 `save` 操作存盘。存之前程序里的图、存完程序里的图、从文件读回的图，三份摘要相同（`source`、`fingerprint` 和各处的句柄除外）。

**与任务写法不同的：**

1. **重复位号一条停了全部错误级规则**，不只 `DR-004`、`DR-102`：检查本身没有错误，拦下的就只能是门禁里不经规则的重复位号这一项，测试也就断言得了 `drc_errors` 是 0。
2. **拦截率每张图只开一条错误级规则**，任务只说「造一张能让它命中的图」：图上难免顺带犯别的错误级规则（画一根空的管线就同时犯 `DR-001`、`DR-002`、`DR-014`），只开这一条，拦下才说得清是因为它。警告级、提示级照常开着，它们不挡发布。

**规格没写到、这里定下的：**

1. **命令造不出的违规直接改图上的记录**：位号空着（`DR-003`、`DR-101`）、同一张图两台泵同位号（`DR-004`）、管路号与管线号相同而没有后缀（`DR-011`）改 P&ID 记录，`DR-116` 改拓扑表；其余都用命令画（`PIDINSERT`、`PIDPIPE`、`PIDFITTING`、`PIDCONNECTOR`、`PIDSETPROP`）。
2. **`DR-116` 要协同服务器配过的对**：`PIDCONNECTOR` 写了对端图号的连接符只记本侧的上游线号，下游线号要配上对才补；按项目索引规格 §3.1 的判法，这是「只指了目标图」，不去找对面，算 `unpaired`，`DR-116` 不报。测试照项目索引测试的做法，在拓扑表里给连接符写上对端图号 `PID-1102` 和下游线号，就是服务器配过对的样子；`PID-1102` 在项目里，上面却没有回来的连接符，算 `one_sided`，`DR-116` 报错。
