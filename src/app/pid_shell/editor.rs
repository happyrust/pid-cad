//! 03 P&ID 编辑器 — screen `editor`; in the design, frame `t4QrJ` with its
//! content in `Ga1Mo`. What the design draws there and where each piece
//! comes from is in `docs/plans/2026-11-11-pid-ui-overhaul-map.md`,
//! section 3.3.
//!
//! ```text
//! toolbar (44 high)       the design's tools that have a command, and 保存
//! palette │ tabs (32)   │ inspector
//! (56)    │ the drawing │ (300)
//! ```
//!
//! Every element is on something real, and what has nothing to stand on is
//! not drawn (`evidence/ct9/differences.txt` of the task lists each):
//!
//! * a **tool** runs a command line exactly as the command line would
//!   ([`Tool::line`]): `PIDPIPE`, `PIDPIPE run`, `PIDPIPE signal`,
//!   `PIDINSERT <category>`, `PIDCONNECTOR <kind>`; 选择 ends the command
//!   that is running, and 保存 is `QSAVE`;
//! * the **palette** is the symbol library (`OpenCADStudio::pid_symbols`):
//!   one square per symbol, drawn from the symbol's own geometry, and a
//!   press starts placing it as the docked palette's cell does
//!   (`PidPaletteMsg::Insert`);
//! * the **tabs** are the open drawings — the general interface's Start tab
//!   is not one of them — with 新建 and 打开 beside them;
//! * the **drawing** is the general interface's drawing area with its own
//!   overlays, its command line and its dialogs, and none of its bars
//!   (`OpenCADStudio::view_classic` leaves them out while the module is
//!   on), in a frame of its own ([`super::rebase`]);
//! * the **inspector** is what is selected ([`inspected`]): its tag, its
//!   type, the line it is on, the fields of its kind with the values its
//!   `PID_AUTHOR` record has, its scale and rotation, and for a fitting how
//!   many of its ports are joined. A value typed there is committed by the
//!   path `PIDSETPROP` and the Properties panel's `P&ID 属性` rows take
//!   (`OpenCADStudio::commit_pid_author_property`).
//!
//! The general interface's ribbon, quick access bar, tab bar, status bar
//! and Start page are not part of this screen; every general command is
//! still there on the command line at the foot of the drawing.

use std::collections::BTreeMap;

use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{alignment, font, Background, Border, Color, Element, Fill, Length, Task};

use super::{chrome, Message as ShellMessage, ViewFn};
use crate::app::commands::pidverify::TAG_NAME;
use crate::app::{Message as AppMessage, OpenCADStudio};
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::{split_props, LINE_FIELDS};
use crate::pid::symbols::block_name;
use crate::pid::tag_engine::TagView;
use crate::pid::topology::{parse_handle, ConnectorKind, TopologyTable};
use crate::pid::type_lib::TypeLib;
use crate::ui::style::pid_fonts;
use crate::ui::style::pid_tokens::{ColorToken, NumberToken, PidTheme};
use crate::ui::window::pid_palette::{symbol_icon, PidPaletteMsg};
use crate::ui::wrap_bar::PosReport;

/// Height of the toolbar (the design's `KON2B`).
pub const TOOLBAR_HEIGHT: f32 = 44.0;
/// Width of the palette strip (the design's `Mini Palette`).
pub const PALETTE_WIDTH: f32 = 56.0;
/// Width of the inspector (the design's `Uer44`).
pub const INSPECTOR_WIDTH: f32 = 300.0;
/// Height of the strip of open drawings above the drawing. Not in the
/// design, which opens and switches drawings on a screen of its own.
pub const TABS_HEIGHT: f32 = 32.0;

/// What a part of the screen reports its rectangle under, followed by the
/// part's name (`ui::wrap_bar::window_bounds`, the automation channel's
/// `ui_bounds`).
pub const BOUNDS_PREFIX: &str = "PID_EDITOR:";

/// The id a part's rectangle is reported under.
pub fn part(name: &str) -> String {
    format!("{BOUNDS_PREFIX}{name}")
}

/// The row id of the tag in [`OpenCADStudio::commit_pid_author_property`].
const TAG_FIELD: &str = "pid_tag";
/// What a property row's id starts with there, followed by the key.
const PROPS_FIELD_PREFIX: &str = "pid_props_";

// ── Tools ────────────────────────────────────────────────────────────────

/// A tool of the toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    /// 选择: no command; ends the one that is running.
    Select,
    /// 管线: a pipe line.
    Pipe,
    /// 分支: a run under a line.
    Branch,
    /// 信号线.
    Signal,
    /// A category of the symbol library, by its id: its first symbol.
    Place(String),
    /// 页间连接, of one kind.
    Connector(ConnectorKind),
}

/// The four kinds of connector, in the order the menu lists them.
pub const CONNECTOR_KINDS: [ConnectorKind; 4] = [
    ConnectorKind::PipeIn,
    ConnectorKind::PipeOut,
    ConnectorKind::SignalIn,
    ConnectorKind::SignalOut,
];

/// What the connector menu calls a kind.
pub fn connector_label(kind: ConnectorKind) -> &'static str {
    match kind {
        ConnectorKind::PipeIn => "管道流入",
        ConnectorKind::PipeOut => "管道流出",
        ConnectorKind::SignalIn => "信号线流入",
        ConnectorKind::SignalOut => "信号线流出",
    }
}

impl Tool {
    /// The command line the tool runs, as it would be typed; `None` for
    /// 选择, which is the Escape key.
    pub fn line(&self) -> Option<String> {
        Some(match self {
            Tool::Select => return None,
            Tool::Pipe => "PIDPIPE".to_owned(),
            Tool::Branch => "PIDPIPE run".to_owned(),
            Tool::Signal => "PIDPIPE signal".to_owned(),
            Tool::Place(category) => format!("PIDINSERT {category}"),
            Tool::Connector(kind) => format!("PIDCONNECTOR {}", kind.as_str()),
        })
    }

    /// The name its rectangle is reported under: `tool:<key>`.
    pub fn key(&self) -> String {
        match self {
            Tool::Select => "select".to_owned(),
            Tool::Pipe => "pipe".to_owned(),
            Tool::Branch => "branch".to_owned(),
            Tool::Signal => "signal".to_owned(),
            Tool::Place(category) => format!("place-{category}"),
            Tool::Connector(kind) => format!("connector-{}", kind.as_str()),
        }
    }
}

/// The symbol categories the toolbar has a tool for, `(id, label)`: the
/// library's, the three the design draws first and in its order.
pub(crate) fn place_tools(app: &OpenCADStudio) -> Vec<(String, String)> {
    const DESIGN_ORDER: [&str; 3] = ["equipment", "valve", "instrument"];
    let mut categories: Vec<(String, String)> = app
        .pid_symbols
        .categories()
        .iter()
        .filter(|category| !category.symbols.is_empty())
        .map(|category| (category.id.clone(), category.label.clone()))
        .collect();
    categories.sort_by_key(|(id, _)| {
        DESIGN_ORDER
            .iter()
            .position(|known| known == id)
            .unwrap_or(DESIGN_ORDER.len())
    });
    categories
}

// ── State and messages ───────────────────────────────────────────────────

/// What the screen remembers between frames.
#[derive(Debug, Default)]
pub struct State {
    /// Whether the list of connector kinds is open under its tool.
    pub connector_menu: bool,
    /// The row of the inspector being typed in: the object, the row's id
    /// and the text so far. Enter commits it; it is dropped as soon as the
    /// object is no longer the one selected ([`selection_refreshed`]).
    editing: Option<(document::Handle, String, String)>,
}

/// What the screen's widgets send.
#[derive(Debug, Clone)]
pub enum Message {
    /// A tool of the toolbar was pressed.
    Tool(Tool),
    /// A symbol of the palette was pressed: its block name.
    Symbol(String),
    /// Open or close the list of connector kinds.
    ConnectorMenu(bool),
    /// A row of the inspector was typed in.
    FieldInput {
        /// The object the row is of.
        handle: document::Handle,
        /// The row's id: `pid_tag`, or `pid_props_<key>`.
        field: String,
        /// The text so far.
        text: String,
    },
    /// Enter in the row being typed in.
    FieldSubmit,
}

fn send(message: Message) -> AppMessage {
    AppMessage::PidShell(ShellMessage::Editor(message))
}

/// The function that draws the screen.
pub(crate) const VIEW: Option<ViewFn> = Some(view);

/// Take one of the screen's messages.
pub(crate) fn update(app: &mut OpenCADStudio, message: Message) -> Task<AppMessage> {
    match message {
        Message::Tool(tool) => {
            app.pid_shell.editor.connector_menu = false;
            match tool.line() {
                None => app.update(AppMessage::CommandEscape),
                Some(line) => app.update(AppMessage::Command(line)),
            }
        }
        Message::Symbol(block) => {
            app.pid_shell.editor.connector_menu = false;
            app.update(AppMessage::PidPalette(PidPaletteMsg::Insert(block)))
        }
        Message::ConnectorMenu(open) => {
            app.pid_shell.editor.connector_menu = open;
            Task::none()
        }
        Message::FieldInput {
            handle,
            field,
            text,
        } => {
            app.pid_shell.editor.editing = Some((handle, field, text));
            Task::none()
        }
        Message::FieldSubmit => {
            let Some((handle, field, text)) = app.pid_shell.editor.editing.take() else {
                return Task::none();
            };
            // A read-only session is told so on the command line, in the
            // words `PIDSETPROP` would be refused with.
            if app.refuse_if_readonly("PIDSETPROP") {
                return Task::none();
            }
            let i = app.active_tab;
            if app.commit_pid_author_property(i, &[handle], &field, &text) {
                app.refresh_properties();
            }
            Task::none()
        }
    }
}

/// The command running in the showing drawing, as the command line that
/// starts it (`CadCommand::tool_line`): `None` while none runs, `Some(None)`
/// for one no tool of this screen stands for.
fn running(app: &OpenCADStudio) -> Option<Option<String>> {
    app.tabs[app.active_tab]
        .active_cmd
        .as_ref()
        .map(|command| command.tool_line())
}

/// The block the running command is placing, when it is a placement.
fn placing(app: &OpenCADStudio) -> Option<String> {
    running(app)
        .flatten()?
        .strip_prefix("PIDINSERT ")
        .map(str::to_owned)
}

/// Whether `tool` is lit. That is read off the command that is running and
/// off nothing else, so it is the same whether the command was typed or a
/// tool was pressed: 选择 while no command runs; a line tool and a connector
/// kind while the running command is the very one the tool starts; a
/// category while a symbol of that category is being placed.
pub(crate) fn tool_is_lit(app: &OpenCADStudio, tool: &Tool) -> bool {
    match tool {
        Tool::Select => running(app).is_none(),
        Tool::Place(category) => placing(app).is_some_and(|block| {
            app.pid_symbols
                .categories()
                .iter()
                .filter(|listed| listed.id == *category)
                .flat_map(|listed| &listed.symbols)
                .any(|symbol| block_name(category, &symbol.id) == block)
        }),
        Tool::Pipe | Tool::Branch | Tool::Signal | Tool::Connector(_) => {
            running(app).flatten() == tool.line()
        }
    }
}

/// Whether the palette's square of `block` is lit: while that symbol is the
/// one being placed, however the placement was started.
pub(crate) fn symbol_is_lit(app: &OpenCADStudio, block: &str) -> bool {
    placing(app).as_deref() == Some(block)
}

/// The general interface has rebuilt what it knows of the selection
/// (`OpenCADStudio::refresh_properties`: the selection, or a property of
/// what is selected, changed). A row being typed in outlives that only
/// while its object is still the one object selected.
pub(in crate::app) fn selection_refreshed(app: &mut OpenCADStudio) {
    let Some((handle, ..)) = &app.pid_shell.editor.editing else {
        return;
    };
    let selected = app.tabs[app.active_tab].scene.selected_handles_in_order();
    if selected.as_slice() != [*handle] {
        app.pid_shell.editor.editing = None;
    }
}

/// Whether there is a drawing to draw on: the showing tab is one, and not
/// the general interface's Start tab.
pub(crate) fn has_drawing(app: &OpenCADStudio) -> bool {
    !app.tabs[app.active_tab].is_start
}

/// Whether the drawing may be written to from here: not a read-only file,
/// not a read-only session.
fn writable(app: &OpenCADStudio) -> bool {
    !app.read_only && !app.collab.readonly
}

// ── What is selected ─────────────────────────────────────────────────────

/// What the inspector shows of the selection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Inspected {
    /// Nothing is selected.
    Nothing,
    /// More than one object is: how many.
    Several(usize),
    /// One object without a `PID_AUTHOR` record: what kind of entity it is.
    Plain(String),
    /// One P&ID object.
    Object(Object),
}

/// One P&ID object, as its records have it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Object {
    /// The entity.
    pub handle: document::Handle,
    /// Its tag as it is shown: `？` for a duplicate.
    pub tag: String,
    /// Its type code (`PID_AUTHOR`'s `type`).
    pub type_code: String,
    /// The label of the library symbol it is an instance of.
    pub symbol: Option<String>,
    /// The line it is on, for a run, a segment and a fitting.
    pub line: Option<String>,
    /// The run it is on, for a segment and a fitting.
    pub run: Option<String>,
    /// Its fields, `(key, value)`: the ones its kind has, in their order,
    /// then whatever else its record carries, then an empty one for each
    /// other property a clicked rule-check result names (P&ID panel spec
    /// §3.6).
    pub fields: Vec<(String, String)>,
    /// Scale and rotation in degrees, for a block reference.
    pub placement: Option<(f64, f64)>,
    /// For a fitting: how many of its ports are joined to a segment, of
    /// how many.
    pub ports: Option<(usize, usize)>,
}

/// The fields an object of `type_code` has by its kind: the five line
/// fields for a line and a run, the type library's groups for a type it
/// knows (the tag is not a field: it has its own row).
pub(crate) fn fields_of_kind(type_code: &str) -> Vec<String> {
    if matches!(type_code, "PIPE_LINE" | "PIPE_RUN") {
        return LINE_FIELDS
            .iter()
            .map(|field| (*field).to_owned())
            .collect();
    }
    TypeLib::embedded()
        .property_groups(type_code)
        .iter()
        .flat_map(|group| group.fields.iter().cloned())
        .filter(|field| field != "位号")
        .collect()
}

/// What is selected in the showing drawing, read from its records.
pub(crate) fn inspected(app: &OpenCADStudio) -> Inspected {
    let tab = &app.tabs[app.active_tab];
    if tab.is_start {
        return Inspected::Nothing;
    }
    let selected = tab.scene.selected_handles_in_order();
    let handle = match selected.as_slice() {
        [] => return Inspected::Nothing,
        [one] => *one,
        several => return Inspected::Several(several.len()),
    };
    let document = &tab.scene.document;
    let entity = document.get_entity(handle);
    let Some(author) = PidAuthor::load(document, handle) else {
        let kind = entity
            .map(|entity| crate::entities::traits::entity_type_name(entity).to_owned())
            .unwrap_or_default();
        return Inspected::Plain(kind);
    };
    let tag = TagView::new(
        author.tag.clone(),
        author.tag_source.clone(),
        author.tag_status.clone(),
    )
    .display;

    let insert = match entity {
        Some(document::EntityType::Insert(insert)) => Some(insert),
        _ => None,
    };
    let symbol = insert
        .and_then(|insert| app.pid_symbols.symbol(&insert.block_name))
        .map(|symbol| symbol.label.clone());
    let placement = insert.map(|insert| (insert.x_scale(), insert.rotation.to_degrees()));

    let topology = TopologyTable::load(document);
    let fitting = topology
        .fittings
        .iter()
        .find(|fitting| parse_handle(&fitting.handle) == Some(handle));
    let run = fitting
        .map(|fitting| fitting.run_tag.clone())
        .or_else(|| {
            topology
                .segment_by_handle(handle)
                .map(|segment| segment.run_tag.clone())
        })
        .filter(|run| !run.is_empty());
    let line = topology.line_tag_of_handle(handle).or_else(|| {
        let run = run.as_deref()?;
        topology
            .pipe_runs
            .iter()
            .find(|record| record.tag == run)
            .map(|record| record.line_tag.clone())
    });
    let ports = fitting.map(|fitting| {
        (
            fitting.ports.values().filter(|peer| peer.is_some()).count(),
            fitting.ports.len(),
        )
    });

    let (mut values, _): (BTreeMap<String, String>, _) = split_props(&author.props);
    let mut fields: Vec<(String, String)> = fields_of_kind(&author.type_code)
        .into_iter()
        .map(|key| {
            let value = values.remove(&key).unwrap_or_default();
            (key, value)
        })
        .collect();
    fields.extend(values);
    for name in app.pid_verify_highlighted(app.active_tab, handle) {
        if name != TAG_NAME && !fields.iter().any(|(key, _)| key == name) {
            fields.push((name.clone(), String::new()));
        }
    }

    Inspected::Object(Object {
        handle,
        tag,
        type_code: author.type_code,
        symbol,
        line,
        run,
        fields,
        placement,
        ports,
    })
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn ui(theme: PidTheme, weight: font::Weight) -> iced::Font {
    pid_fonts::ui(theme, weight)
}

fn mono(theme: PidTheme, weight: font::Weight) -> iced::Font {
    pid_fonts::mono(theme, weight)
}

/// A panel of the screen: the panel colour and the design's hairline, at
/// the strength it shows at (see `chrome`).
fn panel(theme: PidTheme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
        border: Border {
            color: ColorToken::Border.color(theme).scale_alpha(0.5),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

fn filled(
    theme: PidTheme,
    fill: ColorToken,
    stroke: Option<ColorToken>,
    radius: f32,
) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill.color(theme))),
        border: Border {
            color: stroke.map_or(Color::TRANSPARENT, |stroke| stroke.color(theme)),
            width: if stroke.is_some() { 1.0 } else { 0.0 },
            radius: radius.into(),
        },
        ..container::Style::default()
    }
}

/// A button that is only its content.
fn bare(_theme: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        border: Border::default(),
        shadow: iced::Shadow::default(),
        snap: false,
        ..button::Style::default()
    }
}

fn icon<'a>(
    bytes: &'static [u8],
    size: f32,
    theme: PidTheme,
    tint: ColorToken,
) -> Element<'a, AppMessage> {
    crate::ui::icons::tinted(bytes, size, move |_| tint.color(theme))
}

/// One tool of the toolbar: the design's `Tool` — an icon and a label, lit
/// in the accent while it is the one in use.
fn tool_button<'a>(
    app: &'a OpenCADStudio,
    tool: Tool,
    label: String,
    art: &'static [u8],
    on_press: Option<AppMessage>,
    lit: bool,
) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let ink = if lit {
        ColorToken::Accent
    } else if on_press.is_some() {
        ColorToken::TextSecondary
    } else {
        ColorToken::TextMuted
    };
    let face = container(
        row![
            icon(art, 14.0, theme, ink),
            text(label)
                .size(11.5)
                .font(ui(theme, font::Weight::Normal))
                .color(ink.color(theme)),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([6, 8])
    .style(move |_| {
        if lit {
            filled(theme, ColorToken::AccentSoft, None, 3.0)
        } else {
            container::Style::default()
        }
    });
    let pressed = button(face).padding(0).style(bare).on_press_maybe(on_press);
    PosReport::owned(part(&format!("tool:{}", tool.key())), pressed).into()
}

/// A group of tools: the design's `Tool Group`.
fn tool_group<'a>(theme: PidTheme, tools: Vec<Element<'a, AppMessage>>) -> Element<'a, AppMessage> {
    container(row(tools).spacing(2).align_y(alignment::Vertical::Center))
        .padding(2)
        .style(move |_| {
            filled(
                theme,
                ColorToken::BgApp,
                None,
                NumberToken::RadiusSm.value(theme),
            )
        })
        .into()
}

fn toolbar(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    // A tool needs a drawing to work on and leave to write to it.
    let usable = has_drawing(app) && writable(app);
    let press = |tool: &Tool| usable.then(|| send(Message::Tool(tool.clone())));
    let tool = |tool: Tool, label: &str, art: &'static [u8]| {
        let lit = has_drawing(app) && tool_is_lit(app, &tool);
        let on_press = press(&tool);
        tool_button(app, tool, label.to_owned(), art, on_press, lit)
    };

    let select = {
        let lit = has_drawing(app) && tool_is_lit(app, &Tool::Select);
        // 选择 only ends a command: it works in a read-only session too.
        let on_press = has_drawing(app).then(|| send(Message::Tool(Tool::Select)));
        tool_button(
            app,
            Tool::Select,
            "选择".to_owned(),
            include_bytes!("../../../assets/icons/ui/pick_point.svg"),
            on_press,
            lit,
        )
    };
    let lines = vec![
        tool(
            Tool::Pipe,
            "管线",
            include_bytes!("../../../assets/icons/line.svg"),
        ),
        tool(
            Tool::Branch,
            "分支",
            include_bytes!("../../../assets/icons/polyline.svg"),
        ),
        tool(
            Tool::Signal,
            "信号线",
            include_bytes!("../../../assets/icons/ui/bolt.svg"),
        ),
    ];
    let mut objects: Vec<Element<'_, AppMessage>> = place_tools(app)
        .into_iter()
        .map(|(id, label)| {
            tool(
                Tool::Place(id),
                &label,
                include_bytes!("../../../assets/icons/pid_palette.svg"),
            )
        })
        .collect();
    objects.push(connector_tool(app, usable));

    let save = PosReport::owned(
        part("save"),
        button(
            container(
                row![
                    icon(
                        include_bytes!("../../../assets/icons/ui/save.svg"),
                        14.0,
                        theme,
                        ColorToken::TextOnAccent
                    ),
                    text("保存")
                        .size(12)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ColorToken::TextOnAccent.color(theme)),
                ]
                .spacing(6)
                .align_y(alignment::Vertical::Center),
            )
            .padding([7, 12])
            .style(move |_| {
                filled(
                    theme,
                    ColorToken::Accent,
                    None,
                    NumberToken::RadiusSm.value(theme),
                )
            }),
        )
        .padding(0)
        .style(bare)
        .on_press_maybe(usable.then(|| AppMessage::Command("QSAVE".to_owned()))),
    );

    container(
        row![
            tool_group(theme, vec![select]),
            tool_group(theme, lines),
            tool_group(theme, objects),
            Space::new().width(Fill),
            save,
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .width(Fill)
    .height(TOOLBAR_HEIGHT)
    .padding([0, 12])
    .align_y(alignment::Vertical::Center)
    .style(move |_| panel(theme))
    .into()
}

/// 页间连接: the tool, and under it while it is open the four kinds a
/// connector comes in.
fn connector_tool(app: &OpenCADStudio, usable: bool) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let open = app.pid_shell.editor.connector_menu && usable;
    let lit = has_drawing(app)
        && (open
            || CONNECTOR_KINDS
                .iter()
                .any(|kind| tool_is_lit(app, &Tool::Connector(*kind))));
    let ink = if lit {
        ColorToken::Accent
    } else if usable {
        ColorToken::TextSecondary
    } else {
        ColorToken::TextMuted
    };
    let face = container(
        row![
            icon(
                include_bytes!("../../../assets/icons/ui/arrow_long_right.svg"),
                14.0,
                theme,
                ink
            ),
            text("页间连接")
                .size(11.5)
                .font(ui(theme, font::Weight::Normal))
                .color(ink.color(theme)),
            icon(
                include_bytes!("../../../assets/icons/ui/tri_down.svg"),
                8.0,
                theme,
                ink
            ),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([6, 8])
    .style(move |_| {
        if lit {
            filled(theme, ColorToken::AccentSoft, None, 3.0)
        } else {
            container::Style::default()
        }
    });
    let head = PosReport::owned(
        part("tool:connector"),
        button(face)
            .padding(0)
            .style(bare)
            .on_press_maybe(usable.then(|| send(Message::ConnectorMenu(!open)))),
    );

    let mut kinds = column![].spacing(2);
    for kind in CONNECTOR_KINDS {
        let tool = Tool::Connector(kind);
        let row_of = container(
            text(connector_label(kind))
                .size(12)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::TextPrimary.color(theme)),
        )
        .width(Fill)
        .padding([6, 10]);
        kinds = kinds.push(PosReport::owned(
            part(&format!("tool:{}", tool.key())),
            button(row_of)
                .width(Fill)
                .padding(0)
                .style(move |_, status| button::Style {
                    background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                        .then(|| Background::Color(ColorToken::BgHover.color(theme))),
                    border: Border {
                        radius: 3.0.into(),
                        ..Border::default()
                    },
                    snap: false,
                    ..button::Style::default()
                })
                .on_press(send(Message::Tool(tool))),
        ));
    }
    let list = container(kinds).padding(4).style(move |_| {
        filled(
            theme,
            ColorToken::BgRaised,
            Some(ColorToken::BorderStrong),
            NumberToken::RadiusSm.value(theme),
        )
    });
    iced_aw::DropDown::new(head, list, open)
        .width(Length::Fixed(132.0))
        .alignment(iced_aw::drop_down::Alignment::Bottom)
        .offset(4.0)
        .on_dismiss(send(Message::ConnectorMenu(false)))
        .into()
}

/// The palette strip: one square per symbol of the library, drawn from the
/// symbol's own geometry.
fn palette(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let usable = has_drawing(app) && writable(app);
    let mut squares = column![].spacing(6).align_x(alignment::Horizontal::Center);
    for category in app.pid_symbols.categories() {
        for symbol in &category.symbols {
            let block = block_name(&category.id, &symbol.id);
            let lit = has_drawing(app) && symbol_is_lit(app, &block);
            let ink = if lit {
                ColorToken::Accent
            } else if usable {
                ColorToken::TextSecondary
            } else {
                ColorToken::TextMuted
            };
            let square = container(symbol_icon(&symbol.geometry, 26.0, ink.color(theme)))
                .width(40)
                .height(40)
                .center_x(40)
                .center_y(40)
                .style(move |_| {
                    if lit {
                        filled(
                            theme,
                            ColorToken::AccentSoft,
                            Some(ColorToken::Accent),
                            NumberToken::RadiusSm.value(theme),
                        )
                    } else {
                        filled(
                            theme,
                            ColorToken::BgRaised,
                            None,
                            NumberToken::RadiusSm.value(theme),
                        )
                    }
                });
            let pressed = button(square)
                .padding(0)
                .style(bare)
                .on_press_maybe(usable.then(|| send(Message::Symbol(block.clone()))));
            squares = squares.push(PosReport::owned(
                part(&format!("symbol:{block}")),
                iced::widget::tooltip(
                    pressed,
                    container(
                        text(format!("{} · {}", category.label, symbol.label))
                            .size(11)
                            .font(ui(theme, font::Weight::Normal))
                            .color(ColorToken::TextPrimary.color(theme)),
                    )
                    .padding([4, 8])
                    .style(move |_| {
                        filled(
                            theme,
                            ColorToken::BgRaised,
                            Some(ColorToken::BorderStrong),
                            NumberToken::RadiusSm.value(theme),
                        )
                    }),
                    iced::widget::tooltip::Position::Right,
                )
                .gap(6),
            ));
        }
    }
    container(scrollable(squares).height(Fill))
        .width(PALETTE_WIDTH)
        .height(Fill)
        .padding([10, 8])
        .align_x(alignment::Horizontal::Center)
        .style(move |_| panel(theme))
        .into()
}

/// The open drawings, with 新建 and 打开, and the switch of the command
/// line at the far end.
fn tabs(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let mut strip = row![].spacing(2).align_y(alignment::Vertical::Center);
    for (index, tab) in app.tabs.iter().enumerate() {
        if tab.is_start {
            continue;
        }
        let showing = index == app.active_tab;
        let ink = if showing {
            ColorToken::TextPrimary
        } else {
            ColorToken::TextSecondary
        };
        let mut name = row![].spacing(6).align_y(alignment::Vertical::Center);
        if tab.dirty {
            name = name.push(
                container(Space::new())
                    .width(6)
                    .height(6)
                    .style(move |_| filled(theme, ColorToken::Amber, None, 3.0)),
            );
        }
        name = name.push(
            text(tab.tab_display_name())
                .size(12)
                .font(ui(
                    theme,
                    if showing {
                        font::Weight::Medium
                    } else {
                        font::Weight::Normal
                    },
                ))
                .color(ink.color(theme)),
        );
        let switch = PosReport::owned(
            part(&format!("tab:{}", tab.id)),
            button(container(name).padding([0, 4]))
                .padding(0)
                .style(bare)
                .on_press(AppMessage::TabSwitch(index)),
        );
        let close = PosReport::owned(
            part(&format!("tab-close:{}", tab.id)),
            button(icon(
                include_bytes!("../../../assets/icons/ui/close.svg"),
                10.0,
                theme,
                ColorToken::TextMuted,
            ))
            .padding(3)
            .style(bare)
            .on_press(AppMessage::TabClose(tab.id)),
        );
        strip = strip.push(
            container(
                row![switch, close]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
            )
            .padding([5, 8])
            .style(move |_| {
                if showing {
                    filled(
                        theme,
                        ColorToken::BgRaised,
                        Some(ColorToken::Border),
                        NumberToken::RadiusSm.value(theme),
                    )
                } else {
                    container::Style::default()
                }
            }),
        );
    }
    let small = |name: &str, label: &'static str, art: &'static [u8], message: AppMessage| {
        PosReport::owned(
            part(name),
            button(
                container(
                    row![
                        icon(art, 12.0, theme, ColorToken::TextSecondary),
                        text(label)
                            .size(11.5)
                            .font(ui(theme, font::Weight::Normal))
                            .color(ColorToken::TextSecondary.color(theme)),
                    ]
                    .spacing(5)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([5, 8]),
            )
            .padding(0)
            .style(bare)
            .on_press(message),
        )
    };
    strip = strip.push(small(
        "new",
        "新建",
        include_bytes!("../../../assets/icons/ui/plus.svg"),
        AppMessage::TabNew,
    ));
    strip = strip.push(small(
        "open",
        "打开",
        include_bytes!("../../../assets/icons/ui/folder_open.svg"),
        AppMessage::OpenFile,
    ));
    strip = strip.push(Space::new().width(Fill));
    if has_drawing(app) {
        let (label, art): (&'static str, &'static [u8]) = if app.command_line_hidden {
            (
                "显示命令行",
                include_bytes!("../../../assets/icons/ui/tri_up.svg").as_slice(),
            )
        } else {
            (
                "收起命令行",
                include_bytes!("../../../assets/icons/ui/tri_down.svg").as_slice(),
            )
        };
        strip = strip.push(small(
            "command-line",
            label,
            art,
            AppMessage::Command("COMMANDLINETOGGLE".to_owned()),
        ));
    }
    container(strip)
        .width(Fill)
        .height(TABS_HEIGHT)
        .padding([0, 8])
        .align_y(alignment::Vertical::Center)
        .style(move |_| panel(theme))
        .into()
}

/// What stands where the drawing would while no drawing is open — in the
/// place of the general interface's Start page, which this screen does not
/// show. `OpenCADStudio::view_classic` draws it there.
pub(in crate::app) fn no_drawing(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let action = |name: &str,
                  label: &'static str,
                  message: AppMessage,
                  fill: ColorToken,
                  stroke: Option<ColorToken>,
                  ink: ColorToken| {
        PosReport::owned(
            part(name),
            button(
                container(
                    text(label)
                        .size(12)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ink.color(theme)),
                )
                .padding([7, 14])
                .style(move |_| filled(theme, fill, stroke, NumberToken::RadiusSm.value(theme))),
            )
            .padding(0)
            .style(bare)
            .on_press(message),
        )
    };
    container(
        column![
            text("没有打开的图纸")
                .size(14)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextPrimary.color(theme)),
            text("新建一张，或打开已有的图纸文件。")
                .size(12)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme)),
            row![
                action(
                    "empty-new",
                    "新建图纸",
                    AppMessage::TabNew,
                    ColorToken::Accent,
                    None,
                    ColorToken::TextOnAccent
                ),
                action(
                    "empty-open",
                    "打开图纸",
                    AppMessage::OpenFile,
                    ColorToken::BgRaised,
                    Some(ColorToken::BorderStrong),
                    ColorToken::TextPrimary
                ),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Center),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(ColorToken::BgCanvas.color(theme))),
        ..container::Style::default()
    })
    .into()
}

/// The design's zoom control, at the drawing's top left: out, in, fit. The
/// design's percentage is not drawn — the viewport has no 100 %.
/// `OpenCADStudio::view_classic` puts it over the drawing.
pub(in crate::app) fn zoom_control(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let step = |name: &str, art: &'static [u8], line: &str| {
        PosReport::owned(
            part(name),
            button(icon(art, 14.0, theme, ColorToken::TextSecondary))
                .padding(0)
                .style(bare)
                .on_press(AppMessage::Command(line.to_owned())),
        )
    };
    container(
        row![
            step(
                "zoom:out",
                include_bytes!("../../../assets/icons/ui/minus.svg"),
                "ZOOM OUT"
            ),
            step(
                "zoom:in",
                include_bytes!("../../../assets/icons/ui/plus.svg"),
                "ZOOM IN"
            ),
            step(
                "zoom:fit",
                include_bytes!("../../../assets/icons/zoom_ext.svg"),
                "ZOOM EXTENTS"
            ),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center),
    )
    .padding([5, 8])
    .style(move |_| {
        filled(
            theme,
            ColorToken::BgPanel,
            Some(ColorToken::Border),
            NumberToken::RadiusSm.value(theme),
        )
    })
    .into()
}

/// One row of the inspector: a label over a box. `field` is the row's id
/// when the value can be typed over. The box is on the warning colours
/// while a clicked rule-check result names the row (P&ID panel spec §3.6).
fn field<'a>(
    app: &'a OpenCADStudio,
    handle: document::Handle,
    label: String,
    value: String,
    field: Option<String>,
    required: bool,
) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let mut heading = row![text(label.clone())
        .size(11)
        .font(ui(theme, font::Weight::Normal))
        .color(ColorToken::TextSecondary.color(theme))]
    .spacing(4);
    if required {
        heading = heading.push(
            text("*")
                .size(11)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::Danger.color(theme)),
        );
    }
    let lit = field
        .as_deref()
        .is_some_and(|id| app.pid_verify_row_highlighted(app.active_tab, id));
    let (ground, edge) = if lit {
        (ColorToken::AmberSoft, ColorToken::Amber)
    } else {
        (ColorToken::BgInput, ColorToken::BorderControl)
    };
    let boxed = |inside: Element<'a, AppMessage>| {
        container(inside)
            .width(Fill)
            .padding([7, 10])
            .style(move |_| {
                filled(
                    theme,
                    ground,
                    Some(edge),
                    NumberToken::RadiusSm.value(theme),
                )
            })
    };
    let body: Element<'a, AppMessage> = match field.filter(|_| writable(app)) {
        Some(field) => {
            let typed = app
                .pid_shell
                .editor
                .editing
                .as_ref()
                .filter(|(of, row, _)| *of == handle && *row == field)
                .map(|(_, _, text)| text.clone());
            let row_id = field.clone();
            let input = text_input("", &typed.unwrap_or(value))
                .size(12)
                .font(mono(theme, font::Weight::Normal))
                .padding(0)
                .style(move |general: &iced::Theme, status| text_input::Style {
                    // The box round it is the row's; the field is flat.
                    background: Background::Color(Color::TRANSPARENT),
                    border: Border::default(),
                    value: ColorToken::TextPrimary.color(theme),
                    selection: ColorToken::AccentSoft.color(theme),
                    ..text_input::default(general, status)
                })
                .on_input(move |text| {
                    send(Message::FieldInput {
                        handle,
                        field: row_id.clone(),
                        text,
                    })
                })
                .on_submit(send(Message::FieldSubmit));
            PosReport::owned(part(&format!("field:{field}")), boxed(input.into())).into()
        }
        None => boxed(
            text(value)
                .size(12)
                .font(mono(theme, font::Weight::Normal))
                .color(ColorToken::TextPrimary.color(theme))
                .into(),
        )
        .into(),
    };
    column![heading, body].spacing(6).width(Fill).into()
}

/// A line of the inspector's last block: a name and what it is.
fn key_value<'a>(theme: PidTheme, key: &'static str, value: String) -> Element<'a, AppMessage> {
    row![
        text(key)
            .size(12)
            .font(ui(theme, font::Weight::Normal))
            .color(ColorToken::TextSecondary.color(theme)),
        Space::new().width(Fill),
        text(value)
            .size(12)
            .font(mono(theme, font::Weight::Normal))
            .color(ColorToken::TextPrimary.color(theme)),
    ]
    .width(Fill)
    .into()
}

/// What the inspector says when it has no object to show.
fn note<'a>(theme: PidTheme, line: String) -> Element<'a, AppMessage> {
    container(
        text(line)
            .size(12)
            .font(ui(theme, font::Weight::Normal))
            .color(ColorToken::TextMuted.color(theme)),
    )
    .padding(16)
    .into()
}

fn inspector(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let body: Element<'_, AppMessage> = match inspected(app) {
        Inspected::Nothing => note(theme, "没有选中对象".to_owned()),
        Inspected::Several(count) => note(theme, format!("选中了 {count} 个对象")),
        Inspected::Plain(kind) => note(theme, format!("{kind} · 不是 P&ID 对象")),
        Inspected::Object(object) => {
            let mut title = row![
                icon(
                    include_bytes!("../../../assets/icons/pid_palette.svg"),
                    16.0,
                    theme,
                    ColorToken::Accent
                ),
                text(object.tag.clone())
                    .size(15)
                    .font(mono(theme, font::Weight::Semibold))
                    .color(ColorToken::TextPrimary.color(theme)),
                Space::new().width(Fill),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center);
            // The design's badge, for the one kind whose joins are recorded.
            if let Some((joined, total)) = object.ports {
                if total > 0 && joined == total {
                    title = title.push(
                        container(
                            row![
                                container(Space::new()).width(5).height(5).style(move |_| {
                                    filled(theme, ColorToken::Success, None, 2.5)
                                }),
                                text("已连接")
                                    .size(11)
                                    .font(ui(theme, font::Weight::Medium))
                                    .color(ColorToken::Success.color(theme)),
                            ]
                            .spacing(5)
                            .align_y(alignment::Vertical::Center),
                        )
                        .padding([2, 8])
                        .style(move |_| filled(theme, ColorToken::SuccessSoft, None, 10.0)),
                    );
                }
            }
            let kind = match &object.symbol {
                Some(symbol) => format!("{} / {symbol}", object.type_code),
                None => object.type_code.clone(),
            };
            let mut head = column![
                title,
                text(kind)
                    .size(11)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextMuted.color(theme)),
            ]
            .spacing(8);
            if let Some(line) = &object.line {
                head = head.push(
                    text(format!("所在管线 {line}"))
                        .size(11)
                        .font(mono(theme, font::Weight::Normal))
                        .color(ColorToken::TextSecondary.color(theme)),
                );
            }

            let mut fields = column![field(
                app,
                object.handle,
                "位号".to_owned(),
                object.tag.clone(),
                Some(TAG_FIELD.to_owned()),
                true,
            )]
            .spacing(14);
            for (key, value) in &object.fields {
                fields = fields.push(field(
                    app,
                    object.handle,
                    key.clone(),
                    value.clone(),
                    Some(format!("{PROPS_FIELD_PREFIX}{key}")),
                    false,
                ));
            }
            if let Some((scale, rotation)) = object.placement {
                fields = fields.push(
                    row![
                        field(
                            app,
                            object.handle,
                            "缩放比例".to_owned(),
                            format!("{scale:.2}"),
                            None,
                            false
                        ),
                        field(
                            app,
                            object.handle,
                            "旋转".to_owned(),
                            format!("{rotation:.0}°"),
                            None,
                            false
                        ),
                    ]
                    .spacing(10),
                );
            }

            let mut blocks = column![
                container(head).width(Fill).padding(16),
                container(Space::new())
                    .width(Fill)
                    .height(1)
                    .style(move |_| {
                        container::Style {
                            background: Some(Background::Color(
                                ColorToken::Border.color(theme).scale_alpha(0.5),
                            )),
                            ..container::Style::default()
                        }
                    }),
                container(fields).width(Fill).padding(16),
            ];
            // 拓扑: only what the topology table records.
            let mut topology = column![text("拓扑")
                .size(11)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextMuted.color(theme))]
            .spacing(10);
            let mut recorded = false;
            if let Some(run) = &object.run {
                topology = topology.push(key_value(theme, "所在管路", run.clone()));
                recorded = true;
            }
            if let Some((joined, total)) = object.ports {
                topology = topology.push(key_value(
                    theme,
                    "连接点",
                    format!("{joined} / {total} 已吸附"),
                ));
                recorded = true;
            }
            if recorded {
                blocks = blocks.push(container(Space::new()).width(Fill).height(1).style(
                    move |_| container::Style {
                        background: Some(Background::Color(
                            ColorToken::Border.color(theme).scale_alpha(0.5),
                        )),
                        ..container::Style::default()
                    },
                ));
                blocks = blocks.push(container(topology).width(Fill).padding(16));
            }
            scrollable(blocks).height(Fill).into()
        }
    };
    container(body)
        .width(INSPECTOR_WIDTH)
        .height(Fill)
        .style(move |_| panel(theme))
        .into()
}

/// Draw the editor.
pub(crate) fn view(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let drawing = PosReport::owned(
        part("drawing"),
        super::rebase::Rebase::new(app.view_classic()),
    );
    column![
        PosReport::owned(part("toolbar"), chrome::shielded(app, toolbar(app))),
        row![
            PosReport::owned(part("palette"), chrome::shielded(app, palette(app))),
            column![
                PosReport::owned(part("tabs"), chrome::shielded(app, tabs(app))),
                drawing,
            ]
            .width(Fill)
            .height(Fill),
            PosReport::owned(part("inspector"), chrome::shielded(app, inspector(app))),
        ]
        .width(Fill)
        .height(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
