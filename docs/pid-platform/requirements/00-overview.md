# 需求体系总纲

本文档定义 P&ID 平台需求文档体系的编号规范、文档分工、追溯矩阵与机械校验规则。需求条目按区域拆分至 `docs/pid-platform/requirements/` 目录下的九份区域文档，本文档只引用需求 ID，不定义需求。

## 需求 ID 规范

需求 ID 格式为 `REQ-<AREA>-NNN`：`REQ` 为固定前缀；`AREA` 为区域代码，恰有九类取值；`NNN` 为三位数字序号，每个区域内从 `001` 起连续编号，不跳号、不复用已删除编号。ID 全局唯一，`REQ-PLAT-*` 只能定义在 `docs/pid-platform/requirements/10-platform.md`，`REQ-TYPE-*` 只能定义在 `docs/pid-platform/requirements/20-type-library.md`，其余区域同此规则。

九个区域及其覆盖范围：

| 区域 | 名称 | 覆盖范围 |
| --- | --- | --- |
| PLAT | 平台层 | 身份与会话、角色与成员、项目空间、权限模型、审计日志、系统配置 |
| TYPE | 类型库 | 业务类型树、属性分组、数据要素与数据类型、属性级别与默认值、属性拼接、类型与符号关系、规则继承 |
| SYM | 符号与项目设置 | 图例标准、符号库、符号编辑器、项目级绘图与规则设置 |
| DRW | 图纸与绘图 | 图纸管理、画布编辑、对象放置、管路走线与拓扑连接 |
| DATA | 数据与规则检查 | 对象数据、位号唯一性、设计规则检查与检查结果模型 |
| DELV | 交付与发布 | 材料清单、报表、图纸版本快照、发布与交付物输出 |
| COLL | 权限与协同 | 协同编辑、并发冲突处理、协作态权限细化 |
| IO | 导入导出 | 各数据格式的导入导出能力矩阵 |
| MIG | 迁移与互操作 | 历史数据迁移与外部系统互操作 |

全仓库用于识别需求 ID 的锚定正则（含否定环视）为：

`(?<![A-Za-z-])REQ-(PLAT|TYPE|SYM|DRW|DATA|DELV|COLL|IO|MIG)-\d{3}(?!\d)`

`VAL-REQ-001` 这类断言 ID 不是需求 ID，锚定正则的否定环视就是为了避免误匹配：`VAL-REQ-001` 中 `REQ` 之前紧邻连字符 `-`，被前置否定环视 `(?<![A-Za-z-])` 排除；后置否定环视 `(?!\d)` 则排除三位数字之后仍跟数字的误匹配。

## 区域与文件映射

每个区域恰好对应一份需求文档：

| 区域 | 文档 | 范围 |
| --- | --- | --- |
| PLAT | `docs/pid-platform/requirements/10-platform.md` | 平台层需求，共 15 条 |
| TYPE | `docs/pid-platform/requirements/20-type-library.md` | 类型库与属性模型需求，共 15 条 |
| SYM | `docs/pid-platform/requirements/30-symbol-library-and-project-settings.md` | 符号库与项目设置需求，共 18 条 |
| DRW | `docs/pid-platform/requirements/40-drawing-and-drafting.md` | 图纸与绘图需求，共 30 条 |
| DATA | `docs/pid-platform/requirements/50-data-and-rule-engine.md` | 数据与设计规则检查需求，共 16 条 |
| DELV | `docs/pid-platform/requirements/60-deliverables-and-publish.md` | 交付与发布需求，共 11 条 |
| COLL | `docs/pid-platform/requirements/70-collaboration-and-permissions.md` | 协同与权限需求，共 16 条 |
| IO | `docs/pid-platform/requirements/80-import-export-matrix.md` | 导入导出矩阵需求，共 8 条 |
| MIG | `docs/pid-platform/requirements/90-migration-and-interop.md` | 迁移与互操作需求，共 10 条 |

`docs/pid-platform/requirements/95-reuse-mapping.md` 不属于九个区域，只引用需求 ID、不定义需求，故不列入本表。

## 需求追溯矩阵

下表汇总本批次已定义的全部需求 ID 及其界面落点；屏幕列取原型屏幕 ID，无对应屏幕时填 `无界面` 并说明理由。

| 需求 ID | 区域 | 屏幕 ID 或 无界面 | 说明 |
| --- | --- | --- | --- |
| REQ-PLAT-001 | PLAT | 无界面 | 登录失败锁定由平台认证服务承载，14 屏原型不含登录屏 |
| REQ-PLAT-002 | PLAT | 无界面 | 密码有效期校验在认证侧执行，14 屏原型不含登录屏 |
| REQ-PLAT-003 | PLAT | 无界面 | 找回密码流程在认证侧完成，14 屏原型不含登录屏 |
| REQ-PLAT-004 | PLAT | 无界面 | 会话超时与并发控制在认证服务执行，14 屏原型不含会话屏 |
| REQ-PLAT-005 | PLAT | permissions | 预置角色在权限屏角色列表呈现并受删除保护 |
| REQ-PLAT-006 | PLAT | permissions | 自定义角色与成员继承在权限屏角色管理区维护 |
| REQ-PLAT-007 | PLAT | permissions | 角色删除保护与引用检查在权限屏操作 |
| REQ-PLAT-008 | PLAT | workbench | 项目空间创建入口与项目列表落在工作台屏 |
| REQ-PLAT-009 | PLAT | workbench | 项目空间状态展示与只读降级在工作台屏生效 |
| REQ-PLAT-010 | PLAT | permissions | 三级授权链的权限配置落在权限屏 |
| REQ-PLAT-011 | PLAT | permissions | 功能权限勾选与菜单可见性在权限屏配置 |
| REQ-PLAT-012 | PLAT | 无界面 | 审计日志在平台管理端呈现，14 屏原型不含审计屏 |
| REQ-PLAT-013 | PLAT | 无界面 | 日志保留与清理在平台管理端配置，14 屏原型不含审计屏 |
| REQ-PLAT-014 | PLAT | 无界面 | 运行参数在平台管理端配置，14 屏原型不含参数屏 |
| REQ-PLAT-015 | PLAT | 无界面 | 配置包发布在部署侧完成，14 屏原型不含发布屏 |
| REQ-TYPE-001 | TYPE | typelib | 类型树与类型编码编辑在类型库屏呈现 |
| REQ-TYPE-002 | TYPE | typelib | 类型删除与受影响节点提示在类型库屏操作 |
| REQ-TYPE-003 | TYPE | typelib | 属性分组创建与拖拽排序在类型库屏属性区完成 |
| REQ-TYPE-004 | TYPE | typelib | 属性定义移除操作在类型库屏属性表执行 |
| REQ-TYPE-005 | TYPE | typelib | 数据要素名称与标签维护在类型库屏完成 |
| REQ-TYPE-006 | TYPE | typelib | 数据定义的类型选择在类型库屏数据要素区配置 |
| REQ-TYPE-007 | TYPE | typelib | 使用情况查询与删除保护在类型库屏呈现 |
| REQ-TYPE-008 | TYPE | typelib | 属性级别下拉在类型库屏属性表编辑 |
| REQ-TYPE-009 | TYPE | typelib | 默认值上溯结果在类型库屏属性表与实例面板展示 |
| REQ-TYPE-010 | TYPE | typelib | 拼接表达式编辑与校验在类型库屏完成 |
| REQ-TYPE-011 | TYPE | typelib | 拼接求值与循环引用错误提示在类型库屏显示 |
| REQ-TYPE-012 | TYPE | symbols | 类型与符号映射及符号标签配置在符号屏呈现 |
| REQ-TYPE-013 | TYPE | typelib | 类型变更影响提示在类型库屏保存时给出 |
| REQ-TYPE-014 | TYPE | tag-rules | 位号规则继承来源与层级在位号规则屏预览 |
| REQ-TYPE-015 | TYPE | color-rules | 子类型覆盖标记与预览在颜色规则屏维护 |
| REQ-SYM-001 | SYM | symbols | 图例标准选定与不可切换规则在符号库屏呈现 |
| REQ-SYM-002 | SYM | symbols | 符号分类树与选项板分组同步在符号库屏维护 |
| REQ-SYM-003 | SYM | symbols | 符号属性七项在符号库屏属性对话框编辑 |
| REQ-SYM-004 | SYM | symbol-editor | 符号编辑器绘图与常用选项卡在符号编辑器屏呈现 |
| REQ-SYM-005 | SYM | symbol-editor | 连接点三类型与三方向组合在符号编辑器屏连接点表定义 |
| REQ-SYM-006 | SYM | symbol-editor | 符号标签四字段在符号编辑器屏属性标签表编辑 |
| REQ-SYM-007 | SYM | tag-rules | 位号规则八字段与格式拼接在位号规则屏定义 |
| REQ-SYM-008 | SYM | tag-rules | 自动编码、层级选择与后缀递增在位号规则屏配置 |
| REQ-SYM-009 | SYM | tag-rules | 位号唯一校验与自动弹窗开关在位号规则屏设置 |
| REQ-SYM-010 | SYM | tag-rules | 位号规则继承来源与导入导出在位号规则屏呈现 |
| REQ-SYM-011 | SYM | color-rules | 颜色规则条件表达式在颜色规则屏条件编辑区配置 |
| REQ-SYM-012 | SYM | color-rules | 颜色规则继承与启动规则开关在颜色规则屏维护 |
| REQ-SYM-013 | SYM | masterdata | 公称直径序列在项目主数据屏公称直径页签维护 |
| REQ-SYM-014 | SYM | masterdata | 管道等级在项目主数据屏管道等级页签维护 |
| REQ-SYM-015 | SYM | masterdata | 绝热等级在项目主数据屏绝热等级页签维护 |
| REQ-SYM-016 | SYM | masterdata | 流体介质在项目主数据屏流体介质页签维护 |
| REQ-SYM-017 | SYM | 无界面 | 相交断线权重属 P&ID 基础设置，14 屏原型未含基础设置屏 |
| REQ-SYM-018 | SYM | 无界面 | 打断符样式与高度属 P&ID 基础设置，14 屏原型未含基础设置屏 |
| REQ-DRW-001 | DRW | drawings | PBS 层级树与图纸列表在图纸管理屏呈现，图纸挂接与层级检索在该屏完成 |
| REQ-DRW-002 | DRW | drawings | PBS 节点重命名与删除守卫在图纸管理屏操作 |
| REQ-DRW-003 | DRW | drawings | 图纸新建与属性编辑在图纸管理屏完成 |
| REQ-DRW-004 | DRW | drawings | 图纸导入入口与格式选择在图纸管理屏操作 |
| REQ-DRW-005 | DRW | drawings | 图纸复制与位号冲突标记在图纸管理屏与编辑器屏可见 |
| REQ-DRW-006 | DRW | drawings | 图纸移动与父节点重选在图纸管理屏完成 |
| REQ-DRW-007 | DRW | permissions | 图纸权限的主体与权限集在权限管理屏维护，图纸屏只读展示 |
| REQ-DRW-008 | DRW | drawings | 平铺视图预览与位号检索在图纸管理屏呈现 |
| REQ-DRW-009 | DRW | drawings | 版本快照列表与导出入口在图纸管理屏的版本区呈现 |
| REQ-DRW-010 | DRW | drawings | 两版本勾选与比对结果在图纸管理屏的版本区呈现 |
| REQ-DRW-011 | DRW | editor | 新建管线与管路分支绘制在编辑器画布完成，管线号与该屏位号生成联动 |
| REQ-DRW-012 | DRW | editor | 管线管路管段三级层级在编辑器目录树与特性面板呈现 |
| REQ-DRW-013 | DRW | editor | 管路拆分合并与层级拖动在编辑器目录树与画布完成 |
| REQ-DRW-014 | DRW | editor | 阀门插入断开与拖动补位在编辑器画布直接可见 |
| REQ-DRW-015 | DRW | editor | 自动旋转与连接点切换在编辑器画布以空格与 Ctrl+空格触发 |
| REQ-DRW-016 | DRW | editor | 设备放置与 XY 缩放比例在编辑器特性面板编辑 |
| REQ-DRW-017 | DRW | editor | 管口插入与位号前缀在编辑器属性表呈现 |
| REQ-DRW-018 | DRW | editor | 默认管口与逻辑连接点的生成差异在编辑器目录树可辨 |
| REQ-DRW-019 | DRW | editor | 设备关联对话框与属性同步在编辑器画布触发 |
| REQ-DRW-020 | DRW | editor | 仪表放置与属性标记在编辑器画布呈现 |
| REQ-DRW-021 | DRW | editor | 新建回路、添加仪表、移除仪表在编辑器常用选项卡操作 |
| REQ-DRW-022 | DRW | editor | 信号线绘制与信号线型连接点吸附在编辑器画布完成 |
| REQ-DRW-023 | DRW | editor | 管道连接符流入流出与跳转在编辑器画布与目录树呈现 |
| REQ-DRW-024 | DRW | editor | 信号线连接符流入流出与跨图跳转在编辑器画布呈现 |
| REQ-DRW-025 | DRW | editor | 打断符插入与间距拖动在编辑器画布完成 |
| REQ-DRW-026 | DRW | editor | 手动标注的落位与属性提取在编辑器画布完成 |
| REQ-DRW-027 | DRW | editor | 自动标注与提示标注在编辑器插入符号时呈现 |
| REQ-DRW-028 | DRW | editor | 对象复制粘贴与落点在编辑器目录树与画布完成 |
| REQ-DRW-029 | DRW | 无界面 | 草图设置为编辑器内的模态对话框，14 屏原型未单列该屏幕，仅在编辑器屏以入口示意 |
| REQ-DRW-030 | DRW | 无界面 | 用户环境为按用户保存的客户端偏好对话框，14 屏原型未单列该屏幕 |
| REQ-DATA-001 | DATA | properties | 位号作为实例列表首列在属性管理器屏呈现 |
| REQ-DATA-002 | DATA | tag-rules | 位号唯一校验开关在位号规则屏配置 |
| REQ-DATA-003 | DATA | validation | 重复位号在校验结果屏以错误级结果列出 |
| REQ-DATA-004 | DATA | properties | 冲突解决在属性管理器屏改位号并回写 |
| REQ-DATA-005 | DATA | properties | 类型树与实例列表在属性管理器屏呈现 |
| REQ-DATA-006 | DATA | properties | 批量编辑与表格回写在属性管理器屏完成 |
| REQ-DATA-007 | DATA | properties | 必填项为空在属性表标红并阻止保存 |
| REQ-DATA-008 | DATA | drawings | 图纸预览的位号检索在图纸管理屏完成 |
| REQ-DATA-009 | DATA | properties | 多条件查询结果列表在属性管理器屏呈现 |
| REQ-DATA-010 | DATA | 无界面 | 规则集配置属配置端功能，14 屏原型只呈现检查结果 |
| REQ-DATA-011 | DATA | 无界面 | 规则作用域与规则包为配置端对象，原型不单列屏幕 |
| REQ-DATA-012 | DATA | 无界面 | DSL 编辑器为配置端文本控件，原型不单列屏幕 |
| REQ-DATA-013 | DATA | validation | 三种触发时机的结果汇入校验结果屏 |
| REQ-DATA-014 | DATA | validation | 结果字段与严重度在校验结果屏按严重度分组呈现 |
| REQ-DATA-015 | DATA | validation | 结果分组定位与豁免在校验结果屏操作 |
| REQ-DATA-016 | DATA | validation | 内置规则清单与启停状态在校验结果屏规则列表呈现 |
| REQ-DELV-001 | DELV | bom | 材料清单表头与位号唯一性在 BOM 屏呈现 |
| REQ-DELV-002 | DELV | bom | 数量公式结果在 BOM 屏数量列呈现 |
| REQ-DELV-003 | DELV | bom | 聚合范围切换在 BOM 屏范围选择器呈现 |
| REQ-DELV-004 | DELV | bom | 报表模板字段映射在 BOM 屏预览表头可见 |
| REQ-DELV-005 | DELV | bom | 模板与导出格式选择在 BOM 屏导出区完成 |
| REQ-DELV-006 | DELV | drawings | 四格式导出入口在图纸管理屏操作菜单 |
| REQ-DELV-007 | DELV | drawings | 导出范围与出图参数在图纸管理屏导出对话框 |
| REQ-DELV-008 | DELV | 无界面 | 发布包与发布状态在服务端执行，原型不含发布屏 |
| REQ-DELV-009 | DELV | validation | 发布门禁读取校验结果屏的错误级结果 |
| REQ-DELV-010 | DELV | bom | 导出矩阵的格式清单在 BOM 屏导出格式下拉呈现 |
| REQ-DELV-011 | DELV | 无界面 | 导出任务队列在服务端运行，原型不单列任务屏 |
| REQ-COLL-001 | COLL | permissions | 三级授权链的判定顺序与拒绝级别在权限屏呈现 |
| REQ-COLL-002 | COLL | permissions | PBS 节点的显式 ACL 与继承标记在权限屏节点树维护 |
| REQ-COLL-003 | COLL | permissions | 单图授权行与 `继承自` 切换在权限屏图纸页签维护 |
| REQ-COLL-004 | COLL | drawings | 图纸状态在图纸管理屏列表以状态列显示，切换入口在该屏操作菜单 |
| REQ-COLL-005 | COLL | editor | 并发处置的结果（被拒绝、让位、只读降级）在编辑器屏的协同区与提示里呈现；仲裁由协同服务器执行 |
| REQ-COLL-006 | COLL | editor | 位号占用列表与「占用 / 释放」在编辑器屏检视面板下半的协同区呈现，占用标记画在图面上；有效期与到期释放由协同服务器执行 |
| REQ-COLL-007 | COLL | editor | 只读降级在编辑器屏以只读标记、原因与被拒绝的提示呈现 |
| REQ-COLL-008 | COLL | editor | 待裁决的冲突条目列在编辑器屏的协同区；冲突在提交链路上由协同服务器检测 |
| REQ-COLL-009 | COLL | editor | 这一轮裁决经编辑器屏协同区的冲突行与命令提交；裁决列表与裁决对话框留到下一轮 |
| REQ-COLL-010 | COLL | editor | 连接状态与在线成员名单在编辑器屏的协同区呈现；心跳与超时由客户端与协同服务器维持 |
| REQ-COLL-011 | COLL | permissions | 角色 × 操作矩阵在权限屏以角色行与操作列勾选呈现 |
| REQ-COLL-012 | COLL | permissions | 拒绝优先与最小权限在权限屏按格取值判定 |
| REQ-COLL-013 | COLL | editor | 每一步编辑在编辑器屏作为一次提交发出，排队的个数在协同区呈现 |
| REQ-COLL-014 | COLL | editor | 断线、重连与排队的步数在编辑器屏的连接状态与协同区呈现 |
| REQ-COLL-015 | COLL | 无界面 | 图号、服务器库的标识与已应用的修订随图保存，补发在入会时完成，没有专门的屏幕 |
| REQ-COLL-016 | COLL | 无界面 | 身份核对与登记在入会时由协同服务器执行，监听地址是服务器启动参数，14 屏原型不含登录屏 |
| REQ-IO-001 | IO | 无界面 | 导入导出能力矩阵属规格层清单，14 屏原型不含导入导出配置屏 |
| REQ-IO-002 | IO | 无界面 | 图纸导入的格式校验与文件名唯一在服务端执行，入口屏由 `REQ-DRW-004` 承载 |
| REQ-IO-003 | IO | 无界面 | 四格式导出的格式契约在服务端执行，出口屏由 `REQ-DELV-006` 承载 |
| REQ-IO-004 | IO | 无界面 | 属性表格的行级错误定位在服务端产出，编辑面由 `REQ-DATA-006` 承载 |
| REQ-IO-005 | IO | 无界面 | JSON 交换 schema 与版本号是无界面文件契约 |
| REQ-IO-006 | IO | 无界面 | 符号库导入的类型依赖校验在服务端执行，符号维护面由 `REQ-SYM-002` 承载 |
| REQ-IO-007 | IO | 无界面 | 配置包导入导出为管理端功能，14 屏原型不含配置包屏 |
| REQ-IO-008 | IO | 无界面 | 错误文件与重导任务在服务端产出，无可视屏幕 |
| REQ-MIG-001 | MIG | 无界面 | 存量图纸识别在导入服务执行，14 屏原型不含识别确认屏 |
| REQ-MIG-002 | MIG | 无界面 | 人工确认队列属独立迁移工具界面，不在 14 屏原型范围 |
| REQ-MIG-003 | MIG | 无界面 | 参考产品迁移按批在服务端执行，无对应屏幕 |
| REQ-MIG-004 | MIG | 无界面 | 三维模型关联表在服务端维护，14 屏原型不含关联屏 |
| REQ-MIG-005 | MIG | 无界面 | 自动化接口以进程参数启动，无图形交互面 |
| REQ-MIG-006 | MIG | 无界面 | 插件是带外进程，加载与清单校验在进程启动时完成 |
| REQ-MIG-007 | MIG | 无界面 | 数据交换标准的范围声明是口径文档，无界面 |
| REQ-MIG-008 | MIG | backup | 备份数据包打开、按装置组过滤图纸与查看属性和图内对象落在备份数据包屏 |
| REQ-MIG-009 | MIG | backup | 整包导出 publish 文件、目录预检与结果比对查看落在备份数据包屏 |
| REQ-MIG-010 | MIG | backup | 整包导出 PDF、张数进度与缺符号明细查看落在备份数据包屏 |

## 校验规则

需求文档接受以下四条机械校验：

1. **锚定正则**：全仓库需求 ID 必须匹配 `(?<![A-Za-z-])REQ-(PLAT|TYPE|SYM|DRW|DATA|DELV|COLL|IO|MIG)-\d{3}(?!\d)`；正则带否定环视，避免把 `VAL-REQ-001` 误判为需求。
2. **唯一性**：同一 ID 在全仓库只允许定义一次；重复定义报错并指出两处位置。
3. **每区下限**：九个区域中每一个已存在的区域文档至少定义 5 条需求。
4. **区域与文件一一对应**：`REQ-<AREA>-*` 只能出现在该区域对应的文件中；`00-overview.md` 与 `95-reuse-mapping.md` 只引用、不定义。

需求追溯矩阵与各区域文档之间要求双向闭合：矩阵列出的 ID 集合必须与各区域文档定义的 ID 集合完全相等，无遗漏、无孤儿；新增或删除需求时矩阵须同步更新。
