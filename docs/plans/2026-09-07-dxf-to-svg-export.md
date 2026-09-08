# 开发计划：DXF → SVG 导出（v2，已经 oracle 复核）

> 日期：2026-09-07
> 状态：**v2 已批准（2026-09-07 16:00「直接开工，同意了」）**；P0 已实施（§11）、P1 已实施（§12）、
> P2 已实施（§13；web 那一半 2026-09-08 由 `2026-09-08-svg-export-next-steps.md` 的 P4.1 补上，见其 §8）。
> v1 于同日写成；v2 是把 oracle（GPT-5.5 Pro，会话
> `ocs-dxf-svg-plan-review-2`，18 分钟）的十六条评审意见逐条并进来之后的版本，纪要见 §9。
> 结论先说：**今天不支持**。缺的表面上是「一个 SVG 后端」，但 v1 把「PDF 后端的入参」误认成
> 「已经完成语义解析、可直接序列化的纸面矢量」——真正决定纸上画什么的那一千多行
> （排序、CTB、线宽、虚线、字形展开、填充规则、混合状态）今天全写在 `append_pdf_page` 和三个
> `emit_*` 里、直接对着 `printpdf::Op`。所以 v2 的第一步不是写 SVG，是**把这一份语义展开抽成
> 只有一份实现的 emitter + 最小 sink**，PDF 与 SVG 各接一个 sink。
> 涉及（预计）：`src/io/pdf_export.rs`（机械拆分）、新增 `src/io/plot_types.rs`、
> `src/io/plot_emit.rs`、`src/io/svg_export.rs`；改 `src/app/commands/{mod,display}.rs`、
> `src/app/update/file.rs`、`src/app/automation.rs`；测试 `tests/`。

## 0. 一句话

DXF 读得进来、纸面矢量算得出来、PDF 写得出去；**只是没有人把那份纸面矢量写成 SVG**——
而「那份纸面矢量」今天还没有独立存在，它散在 PDF 后端的遍历里。先让它独立存在（一份 emitter），
再给它第二个出口（SVG sink）。**不复制遍历。**

## 1. 现状核对

### 1.1 SVG 今天只进不出

| 用到 SVG 的地方 | 方向 |
|---|---|
| `iced = { …, features = [… "svg" …] }` | **入**：UI 图标（`src/ui/icons.rs` 一批 `include_bytes!("../../assets/icons/ui/*.svg")`） |
| `resvg = "0.45"`（锁定 0.45.1） | **入**：启动时把 `assets/logo.svg` 栅格化成窗口图标 |

**没有任何一处写 SVG。**

### 1.2 导出面现有什么

| 命令 | 去向 | 实现 |
|---|---|---|
| `EXPORT` / `EXPORTPDF` | PDF | `io/pdf_export.rs`（printpdf 0.9.1 / lopdf 0.39.0） |
| `PLOT` / `PRINT` | 打印对话框 → PDF / 打印机 | 同上 + `io/print_to_printer.rs` |
| `EXPORTSTL` / `STLOUT` | STL | `io/stl.rs` |
| `EXPORTSTEP` / `STPOUT` | STEP | `io/step.rs` |
| `CUIEXPORT` | 快捷键表 | — |
| 另存为 | DWG / DXF | `io::save_as_version` |

`export_pdf` 的调用方：`app/update/file.rs` 七处、`io/print_to_printer.rs` 一处；多页走
`export_pdf_pages(&[PdfPageInput])`。

### 1.3 写 `.svg` 现在会发生什么

`--export IN OUT`（`app/automation.rs::export_headless`）把 `OUT` 交给 `crate::io::save`，
而 `io/mod.rs::validate_save_extension` 只放行 `dwg | dxf | sv$`：

```rust
const SUPPORTED_SAVE_FORMATS: &str = ".dwg, .dxf";
```

所以 `OpenCADStudio --export a.dxf b.svg` 今天**报「unsupported output format .svg」**。这一点很重要：
它说明 `--export` 走的是**文档格式**那条路，而 SVG 是**纸面输出**，两条路不同（见 D4）。

## 2. 哪些真的便宜，哪一块 v1 看错了

### 2.1 便宜的部分（v1 说对的）

`io/pdf_export.rs` 的入参**与 PDF 无关**：

```rust
pub fn export_pdf(
    wires: &[PlotWire],          // WireModel + draw_depth；点串里 NaN = 抬笔
    hatches: &[HatchModel],
    wipeouts: &[HatchModel],
    paper_w: f64, paper_h: f64,  // mm
    offset_x: f64, offset_y: f64,
    rotation_deg: i32,           // 0 | 90 | 180 | 270
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    path: &Path,
    plot_style: Option<&PlotStyleTable>,   // CTB
    options: PdfPlotOptions,     // 线宽 / 透明 / 图章 / 合并 / 分组
) -> Result<(), String>
```

纸张、偏移、旋转、缩放、裁剪、CTB、线宽策略都在这里；打印色彩口径（近白/近黄折黑、近青折深蓝、
CTB 覆盖）在 `plotted_color` / `adapt_text_color` 里统一了；PDF 在 wasm 上是桩而 SVG 只是拼字符串。
这些都成立。

### 2.2 v1 的盲点：入参不是「已解析的纸面矢量」

`export_pdf` 本身只是调 builder、写文件。**决定纸上画什么的是 `append_pdf_page`（约 440 行）+
`emit_wire_fills`（约 120 行）+ `emit_hatch`（约 240 行）+ `emit_text`（约 190 行），全部直接
对着 `printpdf::Op` 写。** 它们里面有：

- 两段 `group_splits` 分组，每组内按 `(draw_depth, 类型优先级 WireFill=0 / Hatch=1 / Wire=2 / Text=3, sequence)` 排序；
- 颜色缓存（差异 > 0.01 才重设色）、线宽缓存（差异 > 0.01 pt 才重设）、cap/join 缓存；
- 线宽的三条分支：普通笔宽（`scale_lineweights` 决定是否除以 `scale`）、`world_width > 0` 的宽多段线（几何宽度，随 CTM 缩放、覆盖 CTB 笔宽）、若干处 `0.1 pt` 兜底；
- 虚线的两条实现：`dash_array_from_pattern`（最多六项、取绝对值、四舍五入到整数 pt、1 pt 兜底）与 stationed 虚线（`pattern_stations.len() > points.len()` 时已展开成可见段，输出实线）；
- `plotted_color` 的「透明」不是 alpha，是对白**预混**：`out = 1 - (1 - rgb) × screening × alpha`；
- wipeout 由 `hatch.name == "WIPEOUT_FILL"` 识别，强制白色；`merge_lines` 开启时整页 multiply、wipeout 临时切回 Normal；
- CTB `fill_style` 会把 wire 的填充三角临时转成 hatch；gradient hatch 实际输出两色平均的纯色；
- 文字：`wire.text_verts` 是字形四边形 + UV，**不是轮廓**。`emit_text` 每次锁全局 `sdf_atlas::text_atlas()`、取 `export_table()`、按 UV key 查几何、再按 quad 的仿射基映射到世界坐标。**锁失败直接 `return`，key 查不到静默跳过**。而且它是按 `DrawItem::Text` 逐 wire 调的，「快照一次」是每个 wire 一次，不是整页一次；
- 图章 `emit_plot_stamp` **不走字形几何**：`SystemTime::now()` + `USER`/`USERNAME` + 内置 Helvetica 6 pt `ShowText`，且画在内容 CTM 与 merge 状态恢复之后。

**所以「SVG 不需要嵌字体、不存在缺字」这句 v1 的话要改成：已有从字形四边形 + 图集快照恢复矢量字形
的实现，SVG 可复用；但字体准备、图集完整性、缺字诊断仍是导出依赖，图章更是完全不同的一条路。**

v1 的 R4「SVG 后端保持与 PDF 完全相同的遍历顺序」等于默认把上面这一千多行在 SVG 模块里复制一份
再改成拼字符串。**v2 不这么做。**

## 3. 逐条映射

| PDF 侧 | SVG 侧 |
|---|---|
| `Op::DrawLine`（`LinePoint` 串） | `<path fill="none" d="M … L …">`，NaN 抬笔 = 新的 `M`；**不因首尾重合改 `Z`** |
| `Op::DrawPolygon` + `PaintMode::Fill` | `<path stroke="none" fill=…>`；wire / 字形 `fill-rule="nonzero"`，hatch / wipeout `evenodd` |
| `Op::SetOutlineThickness(Pt)` | `stroke-width`（**无单位**用户坐标，见 D2），按 §2.2 的三条分支换算，**不加 `vector-effect`** |
| `Op::SetLineDashPattern` | `stroke-dasharray`（沿用 `dash_array_from_pattern` 的整数 pt 结果换回 mm）；stationed 虚线输出实线 |
| `LineCapStyle` / `LineJoinStyle` | `stroke-linecap` / `stroke-linejoin`（默认 round，保留 CTB 的 butt/square/miter/bevel 覆盖）；显式 `stroke-miterlimit="10"`（SVG 默认 4，PDF 默认 10） |
| `SetOutlineColor` / `SetFillColor` | `stroke` / `fill`（颜色已在 `plotted_color` 预混，**不**翻成 `opacity`） |
| wipeout（`WIPEOUT_FILL`） | 白色、无描边、Normal、evenodd 的 `<path>`；这是**现有语义**，不是降级（白纸模式，页底先画白） |
| `merge_lines` 的 multiply | **叶子元素**上 `style="mix-blend-mode:multiply"`（不能只挂外层 `<g>`，`mix-blend-mode` 不继承）；整页放隔离容器 |
| `clip` | `<defs><clipPath clipPathUnits="userSpaceOnUse">` 定义在变换前坐标空间，由 CTM 组**内**的内容组引用 |
| `emit_plot_stamp`（Helvetica `ShowText`） | **首版不支持**：`stamp=true` 显式报错，GUI 置灰。不静默忽略，不临时输出依赖系统字体的 `<text>` |
| 页 = `PdfPage` | **SVG 没有页**（见 D3） |

## 4. 设计决策

### D0（新增）纸面语义展开只有一份实现；PDF / SVG 通过最小 sink 分离

这是 v2 的前置决策，其余决策都建立在它上面。

**拆法**（抽取边界在「语义展开之后、写入 printpdf 之前」）：

| 部分 | 去处 | 这次允许做什么 |
|---|---|---|
| 输入类型（`PlotWire` / `PdfPageInput` / `PdfPlotOptions` / `PlotGroupSplits`） | `io/plot_types.rs` | 移动定义；`pdf_export` 保留 re-export，调用方不动 |
| 排序、CTB、坐标恢复、线宽、虚线、hatch 段、字形展开 | `io/plot_emit.rs` | **机械抽取**：不改算法、容差、分支、状态缓存、发射顺序 |
| `PdfDocument`、资源注册、`PdfPage`、保存、文件对话框 | `io/pdf_export.rs` 里的 `PdfSink` | 保留现有实现与调用顺序 |
| SVG 元素、属性、状态栈、XML 编码 | `io/svg_export.rs` 里的 `SvgSink` | **只翻译已决定的绘制操作**，不重新解释 CTB 或排序 |

**形态**：借用型小操作枚举 + 流式 sink，不先分配 `Vec<PlotOp>`、不建场景树：

```rust
// 初次抽取保留既有 f32 排版数值；Pt 是排版单位，不是 printpdf 类型。
#[derive(Clone, Copy)]
pub struct PlotPoint { pub x_pt: f32, pub y_pt: f32 }

pub enum PlotOp<'a> {
    Save, Restore,
    Concat([f32; 6]),
    StrokeColor([f32; 3]), FillColor([f32; 3]),
    StrokeWidthPt(f32),
    LineCap(LineCap), LineJoin(LineJoin),
    DashPt { lengths: &'a [i64], phase: i64 },   // 暂时保留现有 PDF 的整数 pt 语义
    Blend(PlotBlend),
    Stroke { points: &'a [PlotPoint], closed: bool },
    Fill   { rings: &'a [PlotRing], rule: FillRule },
    Clip   { rings: &'a [PlotRing], rule: FillRule },
}

pub trait PlotSink {
    type Error;
    fn emit(&mut self, op: PlotOp<'_>) -> Result<(), Self::Error>;
}

pub fn emit_plot_content<S: PlotSink>(
    page: &PlotPageRef<'_>,
    assets: &PlotAssets,          // 图集快照等显式传入，见 R1
    sink: &mut S,
) -> Result<PlotReport, S::Error>;
```

三条约束：

1. **sink 不再接收未解析的 `WireModel` 自行决定样式。** 否则只是把两份复杂逻辑藏进两个 trait 实现。
2. **这次抽取不「顺便统一数值」。** 现有代码同时用 `Point::new(Mm(..))` 与手工 `Pt(v * MM_TO_PT)`
   （`MM_TO_PT = 2.834645`）。保持原运算次序与结果；不换成更精确的 `72.0 / 25.4`；不让 PDF 走
   `pt → mm → pt` 往返。必要时第一步保留带 `Mm/Pt` 标签的中间点，交给 PDF 适配器调原构造函数。
3. **生产用流式 sink，测试用 `RecordingSink`**（后者才克隆操作与点数组）。

P0 **不**做：高级 `set_style()`、自动状态去重、通用路径优化、图层场景树、任何「看起来等价但改变
Op 序列」的整理。

### D1 坐标系：外层 Y 翻转 + 内层原 CTM，只转换一次

v1 的比较（「逐点转换不镜像、组变换会镜像」）**不成立**：根 `translate(0,H) scale(1,-1)` 与逐点
`y' = H - y` 是同一个仿射变换。整体反射反转所有环的方向，但不改变 nonzero 的内外集合、也不改变
evenodd 的奇偶性。`emit_text` 已经用 `bl + u·(br−bl) + v·(tl−bl)` 把字形映射进 CAD 世界坐标，
SVG 只需对它套**与其他几何相同**的最终变换，**不能再对字形单独翻一次 Y**。

令 `M = [s·cosθ, s·sinθ, −s·sinθ, s·cosθ, tx, ty]` 是现有 PDF 的 CTM（mm 意义下，四个旋转分支照
`append_pdf_page` 的 `needs_state` 段），SVG 的最终矩阵是 `F·M = [a, −b, c, −d, tx, H − ty]`：

| 旋转 | SVG `matrix(a b c d e f)` |
|---|---|
| 0° | `matrix(s 0 0 -s 0 H)` |
| 90° | `matrix(0 -s -s 0 W H)` |
| 180° | `matrix(-s 0 0 s W 0)` |
| 270° | `matrix(0 s s 0 0 0)` |

**`W/H` 已由调用方按旋转交换过，后端不再交换。** 保留原来的 `needs_state` 容差分支。首版输出两层组
（外层 `F`，内层原 `M`），比合成一个矩阵更好审计。

**裁剪不能合进 matrix。** 现有 clip 在 CTM 之后建立、坐标是与 wire 对齐的变换前坐标、**不再加
`ox/oy`**。SVG 把 `<clipPath clipPathUnits="userSpaceOnUse">` 放在根 `<defs>`，由 `F/M` 内的内容组
引用。若把 `clip-path` 挂到变换外层就必须相应变换裁剪几何——决定结果的是引用处坐标系，不是
`<clipPath>` 文本落在哪个组里。

**线宽与虚线照源码分支处理，不统一加 `vector-effect="non-scaling-stroke"`。** 设 `k = MM_TO_PT`：

| 类型 | 变换前 `stroke-width`（用户单位） | 纸面宽度 |
|---|---|---|
| 普通笔宽，`scale_lineweights=false` | `p / (k·s)` | `p/k mm` |
| 普通笔宽，`scale_lineweights=true` | `p / k` | `s·p/k mm` |
| `world_width > 0` 的宽多段线 | `world_width` | `s·world_width mm` |

文字另有例外：无 CTB 覆盖时描边字用字形空间的 nominal pen 经仿射基换算后直接随 CTM 缩放；有 CTB
覆盖才按选项除以 `scale`，粗体乘 `1.7`。`0.1 pt` 是若干分支的兜底，不是统一下限。hatch 圆点的直径来自
`SCREEN_DOT_MM = 25.4/96`、PDF 用十二边形画，首版不换成按笔宽的 SVG 圆。

Y 翻转**不**要求反转 dash array 或取反 offset；危险的是优化时倒转路径、拼接子路径、改闭合——SVG 的
虚线沿子路径布局，必须保留原起点与抬笔边界。

渲染器：浏览器 / Inkscape / resvg / librsvg 对普通二维仿射变换（含负 scale）都是基线能力
（resvg 对照表覆盖 `transform`）；不引入 CSS `transform-origin`。**Illustrator 的导入 / 再保存要按
版本实测**，不承诺。

### D2 单位：根节点 mm，内部无单位

```xml
<svg xmlns="http://www.w3.org/2000/svg"
     width="297mm" height="210mm"
     viewBox="0 0 297 210"
     preserveAspectRatio="xMidYMid meet">
```

**内部坐标、笔宽、dash 长度全部无单位。** `stroke-width="0.25"` 才是 0.25 个用户单位；`"0.25mm"` 会
先按 CSS 单位换算再参与 viewBox 变换，不等于 0.25 用户单位。

「打开即原尺寸」改成：文件携带明确物理页尺寸，按原始视口、100% 打印比例输出时保持约定尺寸；
网页 CSS 缩放、应用「适合窗口」、打印适配仍可能改变最终显示 / 打印大小。

数值：默认四位小数、去尾零，**以纸面误差预算兜底**（每轴 ≤ 0.001 mm；坐标若在比例变换前格式化，
`d` 位小数的误差上界约 `s × ½ × 10⁻ᵈ`，大比例要加位数）。矩阵线性系数用足以往返的精度，不与坐标
一起截。**源码依赖 f64 恢复 high/low 坐标、抵消大世界偏移后才转 f32（`fill_tris` / `fill_tris_low`）
——禁止先量化世界坐标再平移到纸面。** 小数点固定 `.`；规范化 `-0`；NaN 只做内部哨兵不进 XML；拒绝
Infinity、非正页尺寸、非法比例、无效裁剪参数。

### D3 多页：一页一文件，多页全部编号

**单页用用户给的文件名；多页全部编号 `out-001.svg`、`out-002.svg`……**（比「第一张无后缀」更利于
脚本枚举）。编号按页面请求顺序，不按布局名重排。

写入前算出全部目标路径并做冲突检查（含 Windows 大小写冲突，布局名要清理后才能进路径）。CLI 默认
不覆盖、`--force` 才覆盖；GUI 一次确认整个输出集合。先写同目录临时文件再发布；**单文件替换成功不等于
整个集合原子提交**——中途失败要报告哪些已发布，没有批次事务就不宣称「全有或全无」，也不删上一批
多出来的页。web 多页一次打包下载。

### D4 命令行：新开 `--plot-svg`，`--export …svg` 给指路错误

`--export → io::save` 是文档格式转换；页面选择与默认值不是 SVG 序列化器能补的。至少两个入口：

```text
OpenCADStudio --plot-svg in.dxf out.svg --layout "Layout1" --ctb styles/mono.ctb
OpenCADStudio --plot-svg in.dxf out.svg --model --paper A3 --orientation landscape \
                                        --units mm --fit --margins 5mm --ctb none
```

| 参数来源 | 决策 |
|---|---|
| 布局 | 明确 `--layout` 或 `--model`；无头模式**不借用** GUI 的「当前布局」 |
| Layout 页面 | 读已保存页面与视口设置；必要设置解析不了就报错，不默默改 A4 |
| Model 页面 | 要求明确纸张与比例策略；`--fit` 与 `--scale` 互斥 |
| 无单位图纸 | 要求 `--units`，不默认当 mm |
| CTB | 明确文件 / 已保存配置 / `none`；引用的 CTB 缺失即失败，不悄悄回退 |
| 多页 CTB | 保留现有 `page.plot_style.as_ref().or(plot_style)`（页级覆盖优先、全局回退） |
| 审计 | 加 `--list-layouts` 与 `--dry-run`，打印最终纸张、比例、范围、裁剪、CTB、字体告警 |

便利默认以命名预设提供（如 `--preset preview-a3-fit`），不能让用户以为拿到了指定比例的工程出图。

其他工具的参考价值（不概括成「行业接口」）：QCAD 有专门 `dwg2svg/dxf2svg`，带 layout/block、单位、
比例、精度、覆盖、Inkscape 扩展选项，且区分视觉保真与几何保留；LibreCAD 把 SVG/SVGZ 放在图像导出、
打印另列；ODA 的 SVG 导出示例要求先设布局、设备输出矩形与渲染参数；AutoCAD 官方无原生 DWG→SVG。
**拟议的 `SVGOUT` 不必仿照谁。**

### D5 GUI 入口：复用出图设置，只切换目标格式

`EXPORTSVG` / `SVGOUT` 接入**现有出图设置界面**（纸张、比例、范围、CTB、预览、页面选择），最后只
切换目标格式；`on_svg_export_path_some` 不另造一套默认页面。两种入口最终都调同一个页面准备函数：

```rust
pub fn resolve_plot_job(document: &Document, request: &PlotRequest)
    -> Result<ResolvedPlotJob, PlotError>;

pub fn write_svg<W: std::io::Write>(page: &ResolvedPlotPage, assets: &PlotAssets, output: &mut W)
    -> Result<PlotReport, PlotError>;
```

原生包装层管路径、临时文件、对话框（现有 PDF 保存对话框特意绑父窗口处理 Wayland，SVG 复用，不用
无父窗口对话框）；web 包装层写 `Vec<u8>` 后下载。首版 `stamp` 在 SVG 上置灰。

### D6 web 构建：P0 就验 wasm 可编译与无 GUI 冷启动字形

公共 emitter 与 SVG writer 不依赖 printpdf、文件系统、窗口。**风险是实打实的**：`emit_hatch`、
`emit_text`、多个几何 helper 今天都在 `#[cfg(not(target_arch = "wasm32"))]` 下，抽出来时它们的依赖
要一起过 wasm 编译；若字形几何只有窗口绘制后才准备好，`Vec<u8>` 下载再简单也做不成真正的无头导出。
P0 加 wasm 编译检查与无 GUI 冷启动字形测试；P2 完成下载交互。PDF 的 native 限制不阻断 SVG
（注释里的依赖不兼容说明是源码事实，不升级成「printpdf 永远不支持 wasm」）。

## 5. 不做什么

- **不做 SVG 导入**（SVG → CAD 实体）。
- **不复制 plot 遍历**（D0）；也不在 P0 重写 / 优化它。
- **不嵌字体、不出 `<text>`**；图章首版不支持而不是用 `<text>` 凑。
- **不追求与 PDF 字节级等价的 SVG**，追求「同一张图」（§8）；但 **PDF 自身的字节级护栏要保**（§8 第一层）。
- **不按 CAD 图层重组 `<g>`**（首版）：`WireModel` 今天没有图层字段（有 `name`、`aci`、`world_width`、
  `pattern_stations`、`fill_tris`、`text_verts`），已用的 `wire.name` 含 `__paper_boundary__` /
  `paper_printable_area` 这类技术标记，**不能冒充图层名**。以后要图层就从准备阶段显式带 `LayerId/name`
  写到叶子元数据；Inkscape 图层扩展属性只做可选配置。
- **不引入 `<mask>`、`<pattern>`、`<linearGradient>`**（首版）：mask 只能作用于 wipeout 之前的内容、
  还要定义坐标与 luminance/alpha 语义；`<pattern>` 要另证 tile 周期闭合、多族线、相位、角度、与「先裁
  中心线再描边」的边界差异；gradient hatch 今天输出的是平均纯色，出真渐变是功能变更不是翻译。
- **不做透明底图叠加模式**：现有 PDF 就是白纸输出，透明叠加是另一种产品模式。

## 6. 分期

| 期 | 内容 | 出阶段条件 |
|---|---|---|
| **P0 冻结语义与抽取边界** | 先给旧 PDF emitter 建基准、验证导出确定性（图章的时间 / 用户名要冻结，查 printpdf 0.9.1 的资源 ID 是否随机）；机械抽 `plot_types` / `plot_emit` / `PlotSink`；保留 PDF API；补状态、分组、变换、字体依赖测试；**立即**过 wasm 编译 | 新 `PdfSink` 与旧实现**操作级回归通过**；字节一致性的适用条件写清；没有生产遍历副本 |
| **P1 完整单页 SVG** | 同一 emitter 驱动 `SvgSink`；一次覆盖文字、hatch、CTB、wipeout、宽线、station 虚线、裁剪、白底；处理字形三角接缝（R5）；定义 multiply 兼容配置；`stamp` 显式不支持 | 单页语义矩阵通过；实际 SVG **独立解析**与重点栅格比较通过；不支持的选项显式失败 |
| **P2 产品入口** | CLI `--plot-svg`、GUI `EXPORTSVG`、共享 `resolve_plot_job`；多页命名与覆盖；无头冷启动；web 下载与多页打包；输入与缺字诊断 | 原生与 web 端到端测试通过；无 GUI 预热依赖；多页失败行为可预测 |
| **P3 真实批量与受限优化** | 11 张 P&ID、`examples/dxf_legend --svg` 批量、性能与体量基线；再评估受限路径合并、字形复用、`.svgz`；特定 hatch 的 `<pattern>` 可单独推进 | 优化前后语义检查通过；记录字节数、峰值内存、解析 / 渲染耗时，**不只看文件变小** |

**不保留 v1 的「P0 先独立写基础 SVG、P1 再补另一套文字 / hatch」顺序**：一旦先写出平行遍历，
之后再抽共享层比现在抽更难审计。

## 7. 风险

**R1（主，换了）：字形几何依赖全局图集，不是「已经是几何」。** `emit_text` 锁失败 `return`、缺 key
静默跳过。对策：图集快照作为 `PlotAssets` 显式传入、保证与生成 `text_verts` 的图集一致；按页 / 任务取
一次稳定快照（改变快照时机**单独提交**、冻结输入下验证，不与机械抽取混）；SVG 严格模式遇图集不可用
/ 缺 key 返回可定位到 wire/quad 的错误，宽松模式才允许输出并报告缺失数，**不能成功返回一张悄悄少字
的图**。

**R2 合成语义。** wipeout 白填充是现有语义；`merge_lines` 的 multiply 要挂在叶子上（`style=` 而非同名
XML 属性——librsvg 只认 CSS 属性；resvg 自 0.26 支持 mix-blend-mode/isolation，0.45.1 不应默认当不
支持）；`transparency` 是预混色不是 alpha；两个本应分别 multiply 的路径合成一个 `<path>` 后交叠处
不再按两次绘制混色，所以**样式合并必须受限**。目标配置不支持 multiply 时 `merge_lines=true` 报错，
或仅在显式允许降级时输出并报告。

**R3 体量（量级估计，非实测）。** 假设一页 5 万条两端点线段 + 20 万个三角形，仅坐标就有
50,000×4 + 200,000×6 = 1.4M 个数值，每个 6–10 字节 → 坐标部分 8–14 MB，朴素输出 20–40 MB 量级不意外。
次序：先流式输出、样式复用、合理精度、字形填充边界缓存；再受限路径合并（不越过绘制顺序边界、不跨
wipeout/clip/blend 变化、不倒转线段、不拼接独立子路径、multiply 下有重叠的独立绘制不合并）；最后评估
`.svgz`（必须是显式选择的格式、测接收工具，不把 gzip 字节写进 `.svg`）。「PDF 有流压缩」只从
`PdfSaveOptions::default()` 推不出压缩率，要看锁定版产物。

**R4 顺序与状态（被 D0 吸收，但要写成测试）。** 必须保存「分组边界 + 排序键 + 绘图状态缓存」：两组
不能拼回去做全局深度排序；同深度同类型时 wipeout 与 hatch 的顺序来自 `wipeouts.iter().chain(hatches)`；
颜色 / 线宽的 0.01 缓存阈值是**输出语义的一部分**，SVG 若把每根 wire 的理论值直接写到属性里，会比
PDF「更精确」但不再是同一输出。**顺手发现一个现有状态风险**：每组开始 `last_cap/last_join` 被置为
`Some(Round)`，但组间没有对应的实际 PDF 状态重置——若第一组末尾留下 CTB 的 Butt/Miter、第二组第一根
要 Round，缓存会抑制本该发出的恢复操作。这一条**单独用反例锁定、另行修复**，不能在 SVG sink 里无意
修好还声称与现有 PDF 一致。禁止全局「按样式分桶」「按图层分桶」。
（→ 2026-09-08 已修：`2026-09-08-svg-export-next-steps.md` P5.1 / §9，反例进语料，冻结文件同步。）

**R5（新）：填充字形是三角网，抗锯齿留缝。** 源码确实逐三角 `DrawPolygon`（普通 wire 填充也如此），
所以 PDF 也可能有 stitching，但**没有实测证据说这 11 张 PDF 已经出现**。CAD 出图最稳的方向是消除
内部三角边界而不是扩大墨迹：首选字形原始闭合轮廓（要确认 atlas 烘焙前是否保留，附件证明不了）；
推荐从一致的三角网提取外边界与孔洞（内部共享边抵消，再组环）；中间方案是同一字形所有三角子路径
合为一次 nonzero 填充（要保证方向一致、拓扑正确并实测）；`shape-rendering` 只是提示、`crispEdges`
不适合细字曲线；**同色细描边 / 微量重叠不作默认**（扩张轮廓、缩小字腔、multiply 下重叠变暗）。边界
提取在**字形局部坐标**做并缓存，不在大世界坐标靠粗 epsilon 焊接；要检测非流形边、T 接点、不一致顶点，
不能为消缝吞掉细小孔洞。这不破坏 PDF 护栏：P1 加带明确语义边界的字形填充批次，`PdfSink` 仍按原顺序
输出原三角，`SvgSink` 用等价边界。**不把任意相邻同色对象都当 mesh 合并。**

**R6（新）：现有 PDF 导出可能本来就不确定。** `emit_plot_stamp` 读当前秒与用户名；公开的 printpdf
源码里图形状态 ID 用随机字符串（0.9.1 是否如此要查锁定版）。不先证明旧实现对同一冻结输入可重复，
就不能要求重构前后字节相同。

**R7（新）：hatch 的边界语义。** `emit_hatch` 注释写 “rasterise” 但实际是 `pattern_segments_for_plot()`
拿到世界坐标线段逐条描边、边界本身不描边、先清 dash 状态；现有方案是**先裁中心线再描边**，若换成
无限图案先描边再 clip，边界处的圆帽与笔宽溢出会变。首版直接复用这些线段。

## 8. 验收

### 第一层：PDF 重构的精确操作回归（P0 出口）

| 护栏 | 要求 |
|---|---|
| **操作级，必须过** | 旧 emitter 与新 `PdfSink` 的原生 `Op` 顺序、几何、状态切换相同；浮点用 `to_bits()` 比；资源 ID 用一致的符号映射比 |
| **PDF 结构级，必须过** | 页面尺寸、资源定义、解码后的内容操作顺序一致；只归一化**已证明会变**的时间 / 随机 ID，不排序绘制操作、不放宽坐标 |
| **完整字节级，条件满足时必须过** | 先让旧实现对同一冻结输入重复导出证明自身稳定；再要求重构前后 hash 一致 |

冻结：字体 / 图集、时钟、用户名、保存配置、依赖版本。**不通过删掉「看起来不重要」的 PDF 字段把真实
回归一并抹掉。** 现有测试只查 `%PDF` 头、体量、「有字比无字大」，远不够。

### 第二层：SVG 序列化后的独立结构检查（P1 出口）

解析**实际写出**的 SVG，把 transform、clip、样式解析到统一纸面坐标，再检查几何、填充规则、笔宽、
dash、绘制顺序、混合状态。**不能只比较送给两个 sink 的同一个 trace**——那只证明共享输入相同，
证明不了 SVG 写对了。

### 第三层：受控栅格比较 + 重点图人工检查（P1/P3）

全页逐像素差异率**不能**当唯一判据：`0.1 pt` 线在 300 dpi 约 0.417 px、600 dpi 约 0.833 px，同一条
正确细线落在不同采样位置、不同抗锯齿核就会翻掉大部分边缘像素；而一整段文字消失在大幅白纸上的
全局差异率仍可能很低。反过来只比路径数 / 包围盒 / 总长度又太弱（镜像、次序错、缺孔、颜色错都保持
这些统计不变）。

固定一套 PDF 基准引擎与 resvg 版本；统一页框、精确像素尺寸、白底、色彩处理、抗锯齿参数（MuPDF
暴露抗锯齿精度与最小描边宽度参数，测试必须记录，不让隐含笔宽增强参与比较）；不为通过测试切换
`pdftoppm / mutool / pdfium`。初始工程门限（随后用实图校准）：

| 检查 | 初始要求 |
|---|---|
| SVG 数值序列化 | 纸面每轴误差 ≤ 0.001 mm |
| 边缘位置 | 600 dpi 下局部边缘距离，1 px 作初始容忍尺度，**不是**允许整体平移 |
| 平坦色块 | 排除约 1 px 边缘带后查颜色偏差；普通不混合色块先试 1–2/255 通道误差 |
| 缺失内容 | 按局部着墨区域、字形、关键对象查；缺字、缺整段线、wipeout 顺序错**直接失败** |
| multiply 场景 | 独立用例、独立色彩基准，不与普通线图共用宽松阈值 |

先测「同一 PDF 在两个引擎中的差异」建立栅格化噪声参考；再注入删字、错比例、错 dash phase、交换
wipeout 顺序等故障，证明判据真能检出。**阈值必须同时「不误报正常栅格差异」与「不漏报真实缺陷」，
不能只调到 11 张全绿。**

### 全量清单

| 类别 | 必须覆盖 |
|---|---|
| **PDF 不退化** | 冻结输入的旧 / 新 Op 对照；浮点逐位；资源、页尺寸、解码内容；原实现可重复时完整字节 hash 一致 |
| **坐标与物理尺寸** | 四种旋转；比例 <1 / =1 / >1；非零偏移；局部 clip；大世界 high/low 坐标；已知 100 mm 标尺；纸张不重复交换 |
| **状态与顺序** | 两个 `group_splits` 均非空；跨组深度交叉；同深度四类对象；CTB cap/join；颜色与线宽缓存阈值；恢复状态后的后续对象 |
| **线与字** | 普通虚线、station 正反方向、六项以上 pattern、NaN 抬笔、退化点、宽多段线；TrueType、SHX/LFF、粗体、装饰线、旋转 / 斜切字；冷图集、缺失 key |
| **填充与合成** | evenodd 孔洞、多孤岛；实体 / 图案 / 平均色 gradient；CTB 填充样式；多个 wipeout；wipeout 后再绘制；multiply 下交叠与对白预混色 |
| **输出文件** | XML 转义、非法数值、唯一 ID、页尺寸、零外部依赖；文件名冲突、大小写冲突、拒绝覆盖、写入失败、多页部分发布 |
| **兼容性** | 锁定 resvg 与 PDF 栅格引擎；浏览器、Inkscape、Illustrator、librsvg 重点样本；至少查负 scale + clip + 文字、细虚线、multiply + wipeout |
| **真实性与性能** | 11 张真图的局部差异图与人工抽查；故障注入验证阈值；原始 / gzip 大小、峰值内存、解析与渲染时间；优化不得以漏画换体量 |

### 序列化契约（写进代码注释与测试）

| 项目 | 要求 |
|---|---|
| XML | UTF-8；带 XML 声明；根 `xmlns` 正确；无 DTD / 脚本 / 外部依赖 |
| 动态文本 | 转义文本 / 属性中的特殊字符；非法 XML 控制字符拒绝或报告后清理；不把任意输入直接拼进注释 |
| 描边路径 | 显式 `fill="none"`；保留原 `closed=false` |
| 填充路径 | 显式 `stroke="none"`；wire / 字形 nonzero，hatch / wipeout evenodd |
| 裁剪 | 显式坐标单位与 clip rule；白底不进内容裁剪 |
| 页面 | 明确页边界裁剪或 overflow 策略，避免页外墨迹在嵌入场景显示 |
| ID | 确定、唯一、与原始文件名 / 图层名解耦；多页内独立生成 |
| 描边默认 | cap/join round，保留 CTB 覆盖；`stroke-miterlimit="10"` |

## 9. Oracle 评审纪要

会话 `ocs-dxf-svg-plan-review-2`（GPT-5.5 Pro，浏览器模式，17 min 57 s，↑25.5k ↓6.2k tokens），
附件：本计划 v1 全文 + `src/io/pdf_export.rs` 全文。**它没看到**：`WireModel` 定义、图集实现、页面
输入构建代码、`Cargo.lock`、11 张真图——所以它对这些的判断标成「需实测」，本文件里我按仓里实际
内容做了核对（`WireModel` 字段、printpdf 0.9.1 / resvg 0.45.1 / lopdf 0.39.0、图章代码、缓存阈值、
虚线 helper、`plotted_color` 公式、`pattern_segments_for_plot`、页级 CTB 优先级，全部对得上）。

总体判断：**「修订后通过」。架构方向对；最大盲点不是 Y 翻转，而是把可复用的出图输入误认成已完成
语义解析的纸面矢量。** 十六条意见与落点：

| # | 严重度 | 意见 | 落到 |
|---|---|---|---|
| 1 | 必须改 | 不复制遍历；抽最小流式 `PlotSink`，边界在语义展开后、写 printpdf 前；不顺便统一数值 | D0 |
| 2 | 必须改 | 「PDF 字节不变」拆成操作级 / 结构级 / 条件字节级三层；先证旧实现自身可重复 | §8 第一层、R6 |
| 3 | 必须改 | 「文字已是几何、不缺字」不成立；图集是导出依赖；图章走 Helvetica `ShowText`，首版不支持 | §2.2、R1、§3 |
| 4 | 必须改 | R4 要保存分组边界 + 排序键 + 状态缓存；发现组间 cap/join 缓存风险 | R4 |
| 5 | 必须改 | D1 结论可保留但理由重写；给出四个旋转的合成矩阵；clip 不能合进 matrix | D1 |
| 6 | 必须改 | 不统一加 `non-scaling-stroke`；线宽三分支、文字例外、两种虚线实现照源码 | D1、§3 |
| 7 | 必须改 | D2 内部无单位；误差预算；f64 high/low 不能先量化 | D2 |
| 8 | 必须改 | 白 wipeout 是现有语义非降级；multiply 挂叶子；`transparency` 是预混不是 alpha；本期不引入 mask | §3、R2、§5 |
| 9 | 必须改 | 字形三角接缝当首版质量问题；提边界而非细描边 | R5 |
| 10 | 必须改 | hatch 首版复用显式线段；不换 `<pattern>`；gradient 保持平均色 | R7、§5 |
| 11 | 必须改 | XML / 默认绘图属性 / 图层策略写成契约；miterlimit 10；首版不按图层分组 | §8 契约、§5 |
| 12 | 应该改 | R3 区分文本体量优化与改语义的路径合并；量级估计非实测 | R3 |
| 13 | 必须改 | D4 独立命令正确，补页面解析协议、`--list-layouts` / `--dry-run`；其他工具不概括成行业接口 | D4 |
| 14 | 应该改 | D3 多页全编号、预检冲突、默认不覆盖、报告部分发布、web 打包 | D3 |
| 15 | 必须改 | D5/D6 共享 `resolve_plot_job`；wasm 在 P0 验 | D5、D6 |
| 16 | 必须改 | 验收以语义结构 + 独立解码为主；给初始门限与故障注入 | §8 第二 / 三层 |

## 10. 待拍板

1. **D0 的范围**：v2 把任务从「加一个后端」变成「先机械拆 `pdf_export.rs` 再加后端」。P0 只动结构
   不动行为，但它会碰 `app/update/file.rs` 七处与 `print_to_printer.rs` 一处的 import（re-export 可以
   让它们零改动）。**要不要接受这个范围**——不接受就回到 v1 的复制路线，代价是两套出图引擎。
2. **D4 的参数面**：上面那张表是建议，哪些首版必须有（`--layout`/`--model`、`--ctb`、`--dry-run`）、
   哪些可后补（`--preset`、`--margins`）。
3. **D3 的编号格式**：`out-001.svg` 三位定宽，还是按页数自适应。
4. **R5 的取舍**：字形接缝首版就做边界提取（P1 多一块工作），还是先出三角、P3 再做。
5. **图章**：首版不支持可以接受吗？还是需要一个固定字体的矢量图章协议（那是另一件事）。

计划文件目前**未跟踪**；这个仓里另一个会话在持续提交，评审通过前是否先提一笔占住，请说一声。

> 2026-09-07 16:00 拍板：五条全按上面的建议走（接受 D0；D4 首版 `--layout/--model`、`--ctb`、
> `--dry-run` 必有；D3 三位定宽；R5 在 P1 做边界提取；图章首版不支持）。

## 11. P0 实施记录（2026-09-07）

**做了什么**（一个提交，不改任何调用方）：

| 文件 | 内容 |
|---|---|
| `src/io/plot_types.rs`（新） | `PlotWire` / `PdfPlotOptions` / `PlotGroupSplits` / `PdfPageInput` 原样搬来；`pdf_export` `pub use` 回去，`app/update/file.rs` 七处与 `print_to_printer.rs` 零改动 |
| `src/io/plot_emit.rs`（新） | `PlotOp`（15 个变体）、`PlotSink`、`RecordingSink`、`PlotPage`、`PlotAssets`、`emit_plot_content`：`append_pdf_page` + `emit_wire_fills` / `emit_hatch` / `emit_text` / `emit_plot_stamp` + 五个 helper 的机械抽取，发 `PlotOp` 而不是 `printpdf::Op` |
| `src/io/pdf_export.rs`（改） | 只剩 PDF 专有的东西：`PdfSink`（`PlotOp` → `Op` 一对一，`BuiltinText` 展开成图章那六个文本 op）、文档 / 页 / 图形状态注册、保存、对话框 |
| `src/io/pdf_export/legacy_reference.rs`（新，test-only） | 旧 `append_pdf_page` 与全部 helper **逐字冻结**，只把 `PdfPage` 入栈改成返回 `Vec<Op>` |

**坐标契约**：`PlotPoint` 用 PDF 点。旧代码几何走 `Point::new(Mm(v))`（printpdf 的 `From<Mm> for Pt`
是 `v * 2.834_646`），而笔宽 / 裁剪 / CTM 平移 / 虚线用 `MM_TO_PT = 2.834645`——两个常量差在第七位。
抽取**两个都保留**（`GEOMETRY_MM_TO_PT` 与 `MM_TO_PT`），PDF sink 直接包 `Pt`，结果逐位相同；统一它们是
行为变更，另提。CTM 平移里的 `tx * 2.834645` 是 f64 字面量参与运算再转 f32，也照原样。

**验收（第一层，全过）**，`cargo test --lib io::pdf_export` 9/9：

- `emitter_through_pdf_sink_matches_the_frozen_exporter_op_for_op`：22 个用例（四种旋转 + 比例 + 偏移 + 裁剪；
  普通 / 超六项 / stationed 正反向虚线；三种笔宽选项组合 + 宽多段线；颜色适配 + 透明；视口点阵圆点；
  wire 填充 + 低位残差；solid / pattern / gradient / 岛 / ACI-7 白 / wipeout / 退化环 / UTM 量级 `world_origin`；
  CTB 颜色 / 笔 / 加网 / cap-join / 灰度策略 / fill_style 转 pattern；图集文字 + 缺 key + 装饰条；
  两个 render group 交叉深度 + 越界 split + merge_lines + stamp；空页），旧 emitter 与新
  `PdfSink` 的 `Op` 流**逐 op、逐位**相同。**比较不能用 printpdf 自带的 `PartialEq`**：它的 `Pt::eq`
  先四舍五入到 1/1000，`Point::eq` 要求四个分量 `is_normal()`——**坐标为 0 的点跟自己都不相等**。
  改用每个 op 的 `Debug` 文本（Rust 浮点 `Debug` 是最短往返表示，文本相同 ⇔ 逐位相同）。
- `the_corpus_exercises_every_op_kind`：语料覆盖旧 exporter 能发出的全部 18 种 `Op`。
- `geometry_points_match_printpdf_mm_to_pt_bit_for_bit`：`GEOMETRY_MM_TO_PT` 对库逐位钉死（6000+ 个采样）。
- `saved_pdf_bytes_match_the_frozen_exporter_apart_from_the_trailer_id`：**结构级 / 条件字节级**——非
  merge_lines、非 stamp 的 20 个用例，旧 op 与新 op 各自存成 PDF 后，除 trailer `/ID` 外**字节相同**。
- `pdf_bytes_repeat_except_for_the_random_trailer_id`：R6 的实测结论——printpdf 0.9.1 的「随机」是**进程级
  xorshift 计数器**（`RAND_SEED` 从 2100 起、每次 `+21`），每次 `save` 抽两串进 `/ID`，`add_graphics_state`
  也从同一计数器取名。所以同进程两次导出永不逐字节相同、但除 `/ID` 外全同；跨进程第 N 次导出可重复。
  `merge_lines` 时图形状态名进内容流，字节级不适用，由操作级覆盖。
- **变异检验**：把 `GEOMETRY_MM_TO_PT` 改动第七位，钉死 / 操作级 / 字节级三条同时红。

**wasm**：`cargo check --lib --target wasm32-unknown-unknown` 通过，`plot_emit` 与 `plot_types` 全部进 web 构建
（唯一的平台分叉是图章的 `SystemTime::now`，wasm 上给 0）。

**其它**：全库 `cargo test --lib` 635 过 / 1 失败——`app::update::free_text_entry_tests::normal_commands_still_uppercase_and_submit_on_space`
（「Space submitted the line」），与本改动无关，HEAD 干净工作树上同样失败。clippy 对新文件的 9 条告警全是旧代码
原有的写法（`chunks_exact(3)`、`Option<RenderInstance>.clone()`、`&format!`），P0 不改语句；`legacy_reference`
显式 `allow`。

**P0 出口条件对照**：新 `PdfSink` 与旧实现操作级回归通过 ✓；字节一致性的适用条件写清（上面）✓；
没有生产遍历副本 ✓（`legacy_reference` 是 test-only）。R4 里那个**组间 cap/join 缓存**的现有问题在抽取里
原样保留（两边一致），单独修（2026-09-08 修了，见 `2026-09-08-svg-export-next-steps.md` §9）。

## 12. P1 实施记录（2026-09-07）

**做了什么**（一个提交，PDF 侧仍逐位不变）：

| 文件 | 内容 |
|---|---|
| `src/io/svg_export.rs`（新） | `SvgSink`（`PlotOp` → 元素 + 表现属性）、`write_svg_page` / `svg_page_to_string`、`SvgOptions` / `SvgError` / `SvgReport`、R5 的三角网提边界 |
| `src/io/svg_export/tests.rs`（新，test-only） | 第二层与第三层验收，见下 |
| `src/io/plot_emit.rs` | 新增 `PlotOp::FillMesh { tris }`：**一个字形 / 一个 wire 填充是一个形状**，`emit_text` 与 `emit_wire_fills` 各发一条 |
| `src/io/pdf_export.rs` | `PdfSink` 把 `FillMesh` **按原顺序展开成一三角一个 `DrawPolygon`**，PDF 的 `Op` 流一位不动；测试语料搬走后只留 PDF 专有的护栏 |
| `src/io/plot_corpus.rs`（新，test-only） | 22 个用例的语料从 `pdf_export::tests` 搬出来，PDF 与 SVG 两侧共用——为一边加的用例自动护住另一边 |

**坐标（D1/D2 落地）**：外层 `matrix(1/k 0 0 −1/k 0 H)`（k = `GEOMETRY_MM_TO_PT`）+ 内层原 CTM 两层组；
**根 mm、内部无单位**，`viewBox` 就是纸面 mm，所以 `stroke-width="0.25"` 正好是 0.25 mm 纸面。
内层坐标仍是 emitter 的**点**——它们在 F 里跟几何一起缩放，和 PDF 的 CTM 一样，所以笔宽 / dash 不用换算。
小数位按比例自适应：`½·10⁻ᵈ · s · pt→mm ≤ 0.001 mm`，下限 4 位、上限 9 位（D2 的「大比例要加位数」）。
裁剪 `<clipPath clipPathUnits="userSpaceOnUse">` 放 `<defs>`，由 CTM 组**内**的内容组引用；页面另有
`page-clip`（等价 PDF 的 media box），白底在内容裁剪之外。cap/join/miterlimit=10 挂页组继承，只有 CTB
覆盖时才写在叶子上；multiply 用 `style="mix-blend-mode:multiply"` 挂**叶子**，页组 `isolation:isolate`
（且只在真用到时才写）。图章 `BuiltinText` **显式报错**。

**R5**：`FillMesh` 先提边界——按顶点位模式配对有向边，内部边成对抵消，剩下的串成环（外轮廓与孔各一环、
保持原绕向，nonzero 填充孔就还是孔）。**不是干净流形就不猜**（同向重边、顶点出度 > 1 的夹点 / T 接点、
退化三角形都判失败），退回「同一个 `<path>` 里一三角一子路径」——仍是一次栅格化，接缝一样没有，只是
路径大一点。不用同色细描边、不靠 epsilon 焊接：一个字形的三角全过同一个仿射映射，共享顶点的 f32
**逐位相同**，所以位比较是可靠的身份判据。

**验收**：`cargo test --lib io::svg_export` **18/18**（含 1 个 `#[ignore]` 的人工看图导出）。

- **第二层（主）`the_written_svg_draws_what_the_emitter_asked_for`**：22 页语料每页写出 SVG，再用
  **usvg 0.45.1 解析写出的文件**（第三方解析器，不是本模块），把 transform / clip / 继承样式解析回
  纸面 mm，与 `RecordingSink` 的操作流逐个形状比：几何、填充规则、笔宽、dash、cap/join、颜色、
  绘制顺序、multiply、裁剪层数。容差 0.002 mm；**离纸面很远的点按 f32 精度放宽**（语料里有故意放在
  UTM 世界原点、落在纸外 500 m 的图元，SVG 消费者用 f32，那儿的精度是格式给的，不是写入器给的）。
- **故障注入 `the_comparison_catches_a_broken_file`**：删一个 `<path>`、挪一个点、去掉
  `fill-rule="evenodd"`、把页矩阵的 Y 翻转符号改回去、笔宽乘 9 —— 五种都必须被上面那条判据抓到。
- **第三层（栅格，resvg 0.45.1）**：100 mm 标尺（同时锁单位、页原点、Y 方向：CAD y=160 在 210 mm 纸上
  离顶 50 mm）；wipeout 真的遮住底下的墨；两三角方块的**对角线上最淡的像素 > 0.9 墨**（分开填充时那里
  会留一条抗锯齿缝）；缺字用例证明 §8 那句话——删掉一个字形，全页差异率 < 0.1 % 但结构比较直接红、
  局部差异率高出两个数量级。
- 其它：序列化契约（XML 声明 / `xmlns` / mm 尺寸 / 无 DTD / 无脚本 / 无外部引用 / 无 `<text>` /
  描边路径显式 `fill="none"` / id 唯一）；数字格式（无指数、`-0` 归一、去尾零）；不可能的页面
  （零高、负比例、NaN 裁剪、无穷坐标）显式失败；`mesh_outline` 的单元测试（两三角方块、方环的孔
  保持反向绕、领结退回兜底）。
- **PDF 没退化**：`cargo test --lib io::pdf_export` 9/9，`FillMesh` 展开后与冻结旧 exporter 仍逐 op 逐位相同、
  存盘字节除 `/ID` 外相同。**wasm**：`cargo check --lib --target wasm32-unknown-unknown` 通过，
  `svg_export` 进 web 构建（不碰 printpdf、不碰文件系统）。rustfmt 0 diff；clippy 对两个新文件 0 告警。
  全库 `cargo test --lib` 655 过 / 1 失败——`free_text_entry_tests::normal_commands_still_uppercase_and_submit_on_space`，
  P0 那趟就红着，在 `src/app/update/mod.rs`，与本改动无关。

**P1 出口条件对照**：单页语义矩阵通过 ✓；实际 SVG 独立解析通过 ✓；不支持的选项显式失败 ✓（图章）；
重点栅格比较 **部分** ✓——做了受控栅格与故障注入，但**没有做 PDF ↔ SVG 的栅格对比**：那需要一个锁定版
PDF 栅格引擎（pdftoppm / mutool / pdfium），这棵树里没有，装它是另一个决定。§8 第三层的这一半留给 P3，
连同 11 张真图的人工抽查（`#[ignore]` 的 `dump_the_corpus_for_a_human` 是那个抽查的入口）。

**留给后面的**：body 目前**先在内存里拼**再写（`<defs>` 要在引用之前），大页面这是实打实的内存代价，
R3 的体量工作里再解（两趟，或对能接受的消费者把 defs 放最后）；每个叶子都写全属性，样式复用没做；
字形边界没做缓存（一个字形出现 N 次就提 N 次边界）；浏览器 / Inkscape / Illustrator / librsvg 的实测
没做（§8「兼容性」那一行还欠着）。

## 13. P2 实施记录（2026-09-07）

**做了什么**（一个提交）：

| 层 | 内容 |
|---|---|
| `src/io/svg_export.rs` | 文件层（D3）：`export_svg_pages` / `page_paths` / `SvgWriteOptions{force,dry_run}` / `SvgBatch` / `SvgPageOutcome`、`SvgError::{Page,Partial}`、`pick_svg_path_owned`（带父窗口，同 PDF 的 Wayland 处理） |
| `src/io/plot_types.rs` | `PdfPageInput::as_plot_page(fallback)`：页级 CTB 优先、任务级回退这条规则**只有一份**，PDF 与 SVG 共用（`build_pdf_pages` 改用它） |
| `src/app/update/file.rs` | `PlotRequest` + **`resolve_plot_job`**：GUI 的 SVG 导出与无头 `--plot-svg` 都从这里拿页；`set_headless_model_page`（模型空间的纸张 / 比例）；`on_svg_export_path_some` |
| `src/app/automation.rs` | `plot_svg_headless` / `plot_svg_with` / `list_layouts_headless` + 无头四件套（开图、列布局、切布局、装 CTB） |
| `src/cli.rs` / `src/main.rs` | `--plot-svg IN OUT`、`--layout` / `--model`（+`--paper` / `--landscape` / `--fit` / `--scale`）、`--ctb`、`--dry-run`、`--force`、`--list-layouts FILE` |
| `src/app/{mod,update/mod,commands/display,commands/mod}.rs` | `EXPORTSVG` / `SVGOUT` 命令 → 保存对话框 → 同一个 `resolve_plot_job` |

**D3 的文件语义**：单页用给的名字，多页**全部**编号 `out-001.svg`（三位定宽、按请求顺序）。写之前先
把全部目标路径算出来查一遍——批内撞名（含只差大小写，Windows 上是同一个文件）、以及**默认不覆盖**
（`--force` 才覆盖）；然后**先把每一页都渲染出来**再开始发布，所以一页写不出来（图章、坏数值）时
磁盘上一个文件都不会动。真到了发布阶段还失败（比如目标是个非空目录），先写同目录临时文件再改名，
错误里**点名已经落盘的是哪几个**——一页一文件不是事务，不假装回滚，也不删上一批多出来的页。

**D4 的命令行**：`--layout` / `--model` 二选一，都不给就是**这张图的每个布局各一页**；无头**从不**借用
编辑器的「当前布局」。`--model` 必须自己说清纸张与比例：`--paper A3`（A0…A4）加 `--fit` 或
`--scale 1:2` 之一，缺一个就报错——比例串是**严格解析**的，不走 GUI 那个「读不懂就当 1:1」的
`parse_plot_scale`（那对着人看的输入框是对的，对着脚本是错的）。`--ctb` 收文件路径、样式库里的名字、
或 `none`；装不上就是错误，不悄悄回退。`--dry-run` **照样渲染**（所以不支持的选项一样会失败），
只是不落盘，并把要写的路径打出来。`--list-layouts FILE` 单独一条路，不用凑 IN/OUT。

**验收**：`cargo test --lib app::automation` **20/20**（新增 5 条），`io::svg_export` **25/25**（新增 8 条），
全库 `cargo test --lib` **667 过 / 1 失败**（还是那条无关的 `free_text_entry_tests::normal_commands_…`）。

- 无头端到端：临时目录里造一张有边框、圆和 `TEXT` 的图 → 存 DXF → `plot_svg_with` → SVG 落盘、
  usvg 解析通过、**纸张就是命令行要的那张**（A3 横 = 420×297）。**字形是冷图集里出来的**：同一张图
  去掉标签再画一遍，元素数必须少——这条正面回答 D6 的「无 GUI 冷启动字形」，也说明没有 `<text>`
  在替字形挡枪。
- 拒绝的路径都有用例：不存在的布局、装不上的 CTB、`--model` 缺纸张 / 缺比例 / 两个都给 / 纸张名不认识 /
  比例串读不懂——都必须报错**且不落盘**；`--ctb none` 是合法答案。`--scale 1:2` 与 `--fit` 画出来必须不同。
- 文件层：编号、拒绝覆盖 / `--force`、已存在的文件挡下整批（页一也不许写）、发布中途失败点名已落盘的页、
  dry-run 不落盘但照样拒绝图章、页级 CTB 与任务级 CTB 走同一张表画出同一页。
- 真跑了一遍二进制：`--list-layouts` 列出 `Model` / `Layout1`；`--plot-svg … --model --paper A3 --landscape --fit`
  写出 420×297、CTM `matrix(4,0,0,4,0,0)`；`--paper A4 --scale 1:2` 写出 297×210、CTM `matrix(0.5,…)`；
  重复一次不带 `--force` 被拒。
- GUI 侧：`EXPORTSVG` / `SVGOUT` 进命令表并能派发（无头下没有窗口，路径选择直接返回「取消」，
  但命令必须被认出来且不 panic）。clippy 对新代码 0 告警，wasm 编译通过。

**P2 出口条件对照**：原生端到端 ✓；无 GUI 预热依赖 ✓；多页失败行为可预测 ✓；**web 端到端 ✗**。

**这一期没做的**（下一步的料）：

- **web 下载与多页打包**：web 构建上 `EXPORTSVG` 明确报「暂不可用」，`Vec<u8>` 下载与多页打包没写。
  写入器本身是跨平台的（wasm 编译通过），缺的是包装层。
  → 2026-09-08 P4.1 补上了单页下载（`svg_job_to_string` + `sys::download_bytes`）；多页打包仍待 G4。
- **缺字诊断**（R1 的严格 / 宽松模式）：`emit_text` 仍沿用 PDF 一直以来的行为——图集锁不上就整段不画、
  key 查不到就静默跳过。SVG 这一侧既没有严格模式的可定位错误，`SvgReport` 里也没有缺失计数。
  R1 点名要的东西，现在只在计划里，不在代码里。
- **D4 表里剩下的**：`--units`（无单位图纸不该默认当 mm）、`--preset`、`--margins`、
  `--orientation` 作为取值（现在是 `--landscape` 开关）。
- **GUI 的多页 SVG**：编辑器里 SVG 导出目前是当前这一页；`PRINTALL` 那条「所有布局」的路只通 PDF。
  CLI 那边不带 `--layout` 就是全部布局，两边不对称。
- **图章置灰**：对话框没有按目标格式变灰，SVG 导出遇到 `stamp=true` 是明确报错。

## 14. 真图基线与 R3 第一轮（2026-09-07）

### 14.1 先量再改

11 张真图（`cad/0版重新处理dxf-12张`，12 张里有 1 张被别的程序锁着打不开——**「11 张」大概就是这么来的**）。
**这些图的布局是空的**：整张图纸（图框、标题栏、流程）都画在模型空间，纸空间只有 4 个实体、
**一个 VIEWPORT 都没有**，所以 `--layout 布局1` 画出来只有白纸是对的，不是导出的毛病（PDF 走同一个
`resolve_plot_job`，一样空）。下面都是 `--model --paper A1 --landscape --fit`。

改之前先量了一张（SP02-06，2.2 MB）的字节都花在哪：

| | 字节 | 占比 |
|---|---|---|
| 路径数据 `d="…"` | 2.02 MB | **91 %** |
| 其余属性 | 198 KB | 9 % |

3586 条路径，**连续同样式的段只有 404 段**（全页只有 49 种不同样式）；坐标里 20 万个数用了 4 位小数、
2 万个用 3 位。结论很直白：**样式复用最多省 9 %，精度才是大头**。

### 14.2 做了两件事

1. **精度按预算给，不再有「至少四位」的地板**。`auto_decimals` 现在纯按 D2 的
   `½·10⁻ᵈ·s·pt→mm ≤ 0.001 mm` 算：1:1 出图是 3 位，比例大了才加位。测试里同时钉了两头——
   **既不能超预算，也不能比预算多花一位**。
2. **样式复用**：`stroke` / `fill` / `stroke-width` / cap / join / dasharray 都是可继承属性，
   一段共用同一套画笔的元素外面包一个 `<g>` 写一次，叶子只写它要**反驳**的那一半
   （描边写 `fill="none"`，填充写 `stroke="none"`）。样式段一遇到 Save / Restore / Concat / Clip
   就收口，绝不跨结构边界。

另外把 **`.svgz`** 接上：**只有目标文件名就叫 `.svgz` 时才压**（D3/R3 的要求——不能把 gzip 字节塞进
`.svg`，接收方没法知道）。测试里验了 gzip 魔数、`.svg` 一定不是 gzip、以及**这棵树里唯一的消费者
usvg 能直接读**。

### 14.3 量出来的结果（11 张，KB）

| 图号 | 改前 | 改后 | 省 | .svgz | 相对改后 |
|---|---|---|---|---|---|
| FF02-05 | 2309 | 2065 | 11 % | 735 | 64 % |
| FF02-06 | 2275 | 2024 | 11 % | 722 | 64 % |
| FF02-07 | 2734 | 2449 | 10 % | 870 | 64 % |
| SP02-05 | 3457 | 2894 | 16 % | 845 | 71 % |
| SP02-06 | 2171 | 1857 | 14 % | 604 | 67 % |
| SP02-07 | 1960 | 1648 | 16 % | 503 | 69 % |
| SP02-08 | 1970 | 1658 | 16 % | 506 | 69 % |
| SP02-09 | 1696 | 1437 | 15 % | 448 | 69 % |
| SP02-10 | 1825 | 1545 | 15 % | 466 | 70 % |
| WS02-05 | 1074 | 905 | 16 % | 299 | 67 % |
| WS02-06 | 1162 | 1045 | 10 % | 385 | 63 % |
| **合计** | **22.1 MB** | **19.1 MB（−14 %）** | | **6.2 MB（−72 %）** | |

值得记一笔：改之前那批 gzip 是 7.0 MB，改之后 6.2 MB——**文本层的优化压缩后仍然剩下 11 %**，
不是白做（原先担心「重复属性本来就压得掉」，事实是精度那一刀压完照样在）。

**渲染没退**：SP02-06 在 600 dpi 下重渲，标题栏（填充式中文字形 + CPECC 多色徽标，都是 `FillMesh`
提出来的轮廓）与改前逐处对照看不出差别；第二层的 22 页结构比较在新精度下全绿——0.002 mm 的容差
本来就是按 0.001 mm 预算定的。

**耗时**：每张 8–12 s，**debug 构建**，含进程启动 + DXF 解析 + 场景构建 + 出图。不是能拿去说事的
数字，release 基线还欠着。峰值内存也没量。

### 14.4 R3 还剩下的

- **受限路径合并**（R3 的第二梯队）没做：这一轮只动了文本层，没动绘制顺序，也没合并任何路径。
- **流式输出**没做：body 仍先在内存里拼（`<defs>` 要在引用之前）。
- **字形边界缓存**没做：一个字形出现 N 次就提 N 次边界。
- release 的耗时 / 峰值内存基线、以及浏览器 / Inkscape / Illustrator 的实测，都还欠着。

**顺手撞见的 R1 实证**：`a_missing_glyph…` 那条用例在并行跑全库时会偶发拿不到字形——另一条用例
同时排版自己的文字、把图集撑大重排，我这边 quad 的 UV key 就失效了，`emit_text` 于是**静默少画**。
这正是 R1 描述的失效模式，第一次在测试里现形（用例改成重试，并把原因写在注释里）。真要治，
得按 R1 做「快照进 `PlotAssets`」加严格模式。

## 15. R1 实施记录（2026-09-08）

**做了什么**（一个提交；P0 的机械抽取早已单独提交，这一笔只动快照的时机与缺字的下场）：

| 层 | 内容 |
|---|---|
| `src/io/plot_emit.rs` | `GlyphSnapshot`（`capture` / `of(&atlas)` / `missing_in`）；`PlotAssets.glyphs` + `for_pages` / `stale_glyphs`；`PlotReport { text_items, missing_glyphs, first_missing, atlas_unavailable }` + `MissingGlyph { wire, quad }`；`emit_plot_content` 按 D0 的签名返回 `PlotReport`；`emit_text` 从**页的**快照画，缺 key 计数并点名第一个，锁不上记 `atlas_unavailable` |
| `src/io/svg_export.rs` | `SvgOptions.missing_glyphs: MissingGlyphs::{Refuse, Report}`（**默认 `Refuse`**）；`SvgError::MissingGlyphs { count, atlas_unavailable, first }`；`SvgReport.missing_glyphs`；`write_svg_page` 在**任何字节到达调用方之前**拒绝——sink 看到的是绘制操作，看不见没画的字，这是唯一能拒的地方 |
| `src/io/pdf_export.rs` | 收下报告、不看：PDF 历史上一直发布缺字页，把它改成拒绝是 PDF 的决定，不混进这一笔；9 条冻结比较逐位不变 |
| `src/io/plot_corpus.rs` | `text_wire_with_snapshot`：排版与快照**在同一把锁下**，快照一定持有 quad 携带的每个 key |
| `src/app/update/file.rs` | `PlotJob { pages, assets }`；`resolve_plot_job` 排完版**当场**取一份任务级快照并对页检查，发现失配就 `bump_geometry` 重排一次再取；`EXPORTSVG` 走 `job.assets`，严格默认，错误进命令行 |
| `src/app/automation.rs` / `cli.rs` / `main.rs` | `--allow-missing-glyphs`：默认拒绝，给了才写页并在 stderr 打 `warning: N glyph(s) are missing` |

**快照的时机**：任务级——`resolve_plot_job` 在排版的那条线程上、排完版立刻取，所以后台线程出图时
编辑器继续烘焙字形也动不到它。`PlotAssets.glyphs` 为 `None` 时 emitter 在页的**第一个文本项**处自取
一次、整页共用（无文字的页不付 `export_table` 的代价——它在图集锁下按字体族重新解析字体）。
两处都是「一页一份」，不再是原来 `emit_text` 每个渲染组各取一份。

**三种下场**：图集锁不上 → `atlas_unavailable`，严格模式报「the glyph atlas could not be read」；
key 缺 → 计数 + 第一个的 `wire` 名与 quad 序号，严格模式报「N glyph(s) are missing from the page,
first on 'wire' at quad k」；宽松（只有 `--allow-missing-glyphs`）→ 照写，`SvgReport.missing_glyphs > 0`，
CLI 打 warning。**任何入口都不会成功返回一张悄悄少字的图**。

**恢复**：`stale_glyphs > 0` 说明排版期间图集长过一次、早排的 quad 指向已不存在的 tile
（冷图集上第一张大图正是这种情形），于是重排一次——此时图集已含全部字形，不会再长——再取快照。
只重排一次；若之后仍失配（理论上不该），任务按严格规则被拒，不循环。

**验收**：

- `io::svg_export` **30/30**（+4）、`app::automation` **22/22**（+2）、`io::pdf_export` **9/9** 逐位不变。
  全库 `cargo test --lib` 并行**跑三遍**，每遍 676 过 / 1 失败（还是那条无关的
  `free_text_entry_tests::normal_commands_…`）；§14.4 那条 `a_missing_glyph…` 的重试拆掉了，三遍没有 flake。
- 新用例钉的是：默认拒绝并点名 wire（`the-label`）；宽松拿到页和计数；**快照说了算而不是活图集**
  （空快照 + 活图集里有字 → 拒绝）；同锁快照下别的用例并行烘焙不影响（严格通过）；任务携带一份快照且
  覆盖自己的页、无文字的图不取快照；图集被 `reset`（TEXTFILL 的路）后老任务照样整页、新任务重排后覆盖、
  两次元素数相同。
- **11 张真图**在严格默认下全部出图，**11/11 与 §14 的 R3 批次 SHA-256 逐字节相同**——快照只改变从哪个
  图集状态画，不改变画什么；`--allow-missing-glyphs` 接受且输出与严格相同。
- clippy 对 R1 的行 0 告警（本工具链新增的 `chunks_exact_to_as_chunks` 在旧行上还响着，没动），
  wasm 编译过，touched 文件 fmt 过（`automation.rs` 只格式化新代码，同 P2 的口径）。

**没做的**：PDF 仍旧宽松（历史行为）；GUI 没有宽松开关（缺字就是错误，用户只能改图重试），对话框也没有
相应的 UI；`PlotReport.text_items` 只记录没展示；§14.4 的 R3 剩余项不变。
