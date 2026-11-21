//! What a symbol file of the backup draws.
//!
//! A backup carries the plant's symbol library: hundreds of `.sym` files in
//! the `Ref` tree and in one of its archives, which the tree could only
//! name. A click on one -- in the backup folder, in an archive, or in the
//! `Ref` tree -- reads it off the UI thread
//! ([`crate::io::pid_backup_browse::symbol_file`]) and shows it under the
//! file's own details (issue #116): a thumbnail of what the import draws of
//! it, how it reads, and what it holds -- its primitives by kind, the layers
//! shown and the ones off, what lies on the sheets that are not drawn.
//!
//! A file named `.sym` is this section's and not the text preview's
//! ([`is_symbol`]): it is never text.
//!
//! Only the selected file's reading is kept
//! ([`Section::keeps_only_selected`]), so of a run of quick clicks the last
//! one alone is shown.

use std::borrow::Cow;
use std::f64::consts::TAU;
use std::sync::Arc;

use iced::widget::canvas;
use iced::widget::canvas::{Frame, Program};
use iced::{Element, Length, Point, Size, Theme};

use super::section::{Cx, Event, EventKind, Section, SectionData, SectionLoad};
use super::text_preview::{content, Source};
use super::{human_size, ref_at, DetailLine, NodeId};
use crate::app::Message;
use crate::io::pid_backup_browse::symbol_file::{
    read_symbol_file, read_symbol_member, Across, Body, Counts, Failure, Ink, Kind, Label, Reading,
    Sketch, SymbolFile, SymbolNotDrawn, Upright,
};
use crate::io::pid_backup_browse::BackupIndex;
use crate::{t, tf};

/// Height of the thumbnail, in pixels.
const THUMBNAIL_H: f32 = 160.0;

/// Pixels left free on every side of the symbol in its thumbnail.
const PAD: f32 = 8.0;

/// Width of the lines a symbol is drawn with, in pixels.
const LINE_W: f32 = 1.2;

/// The em of the font a label is drawn in, in heights of its capitals:
/// lettering states how tall its capitals are, a font how tall its em is.
const EM_PER_CAP: f64 = 1.4;

/// How far below the top of a line one em tall the baseline of its letters
/// lies, in ems. A face says that of itself, and a thumbnail is lettered in
/// whichever face the interface runs in: the common ones say 0.85 to 0.91.
const BASELINE_PER_EM: f64 = 0.88;

/// How far below their baseline the descenders of lettering reach, in
/// heights of its capitals. The drawing asks the face it letters in; the
/// common ones say about three tenths.
const DESCENT_PER_CAP: f64 = 0.3;

/// The smallest font size a label is drawn at, in pixels. Smaller is a
/// smudge.
const SMALLEST_LABEL: f32 = 3.0;

/// The section: it hangs no rows, and adds what a symbol file draws and
/// holds to the details of the file nodes the tree already has.
pub(super) struct SymbolPreview;

/// Whether `id` is a file named the way a symbol file is: `.sym`, in any
/// case. A file of the backup folder is one too when the path it was backed
/// up from is named so.
pub(super) fn is_symbol(index: &BackupIndex, id: &NodeId) -> bool {
    let named = |name: &str| {
        std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sym"))
    };
    match id {
        NodeId::File(at) => index.files.get(*at).is_some_and(|file| {
            named(&file.name)
                || file
                    .record
                    .as_ref()
                    .is_some_and(|record| named(&record.original))
        }),
        NodeId::FileMember(file, member) => index
            .files
            .get(*file)
            .and_then(|file| file.members.get(*member))
            .is_some_and(|name| named(name)),
        NodeId::RefFile(dirs, at) => ref_at(&index.ref_tree, dirs)
            .and_then(|dir| dir.files.get(*at))
            .is_some_and(|file| named(&file.name)),
        _ => false,
    }
}

/// Where the symbol file `id` is, when `id` is one with content to read.
fn source(index: &BackupIndex, id: &NodeId) -> Option<Source> {
    if is_symbol(index, id) {
        content(index, id)
    } else {
        None
    }
}

impl Section for SymbolPreview {
    fn details(&self, cx: &Cx<'_>, id: &NodeId, lines: &mut Vec<DetailLine>) {
        match cx.load(id) {
            None => {}
            Some(SectionLoad::Loading) => {
                lines.push(DetailLine::Marker(t!("Reading the file…").into_owned()));
            }
            Some(SectionLoad::Failed(error)) => lines.push(DetailLine::Marker(error.clone())),
            Some(SectionLoad::Ready(data)) => {
                if let Some(file) = data.get::<SymbolFile>() {
                    lines_of(file, lines);
                }
            }
        }
    }

    fn wants(&self, cx: &Cx<'_>, event: &Event<'_>) -> Option<NodeId> {
        (event.kind == EventKind::Selected && source(cx.index, event.node).is_some())
            .then(|| event.node.clone())
    }

    fn load(&self, index: &BackupIndex, node: &NodeId) -> Result<SectionData, String> {
        let file = match source(index, node) {
            Some(Source::File(path)) => read_symbol_file(&path)?,
            Some(Source::Member {
                archive,
                member,
                name,
            }) => read_symbol_member(&archive, member, &name)?,
            None => return Err(t!("this node has nothing to read").into_owned()),
        };
        Ok(SectionData::new(file))
    }

    fn keeps_only_selected(&self) -> bool {
        true
    }
}

/// The lines a symbol file adds to its details: its thumbnail, how it
/// reads, and what it holds.
fn lines_of(file: &SymbolFile, lines: &mut Vec<DetailLine>) {
    let marker = |text: Cow<'_, str>| DetailLine::Marker(text.into_owned());
    if let Some(body) = &file.body {
        if !body.sketch.is_empty() {
            lines.push(DetailLine::Sketch(Arc::clone(&body.sketch)));
        }
    }
    match &file.reading {
        Reading::Drawable => {}
        Reading::EmptyBody => {
            lines.push(marker(t!(
                "Nothing to draw: the symbol displays no primitive."
            )));
        }
        Reading::Short(None) => lines.push(marker(t!(
            "Part of the file was not read; what is shown is part of the symbol."
        ))),
        Reading::Short(Some(why)) => lines.push(marker(match why {
            SymbolNotDrawn::ClusterShort => t!(
                "Not drawn: its PSMcluster0 stream was not read whole, so the layer table and the view sets that say what draws the symbol may be incomplete."
            ),
            SymbolNotDrawn::NothingToShow => t!(
                "Not drawn: the sheet that draws the symbol was not read whole, and nothing read of it shows."
            ),
            SymbolNotDrawn::PickAmongShortSheets => t!(
                "Not drawn: the symbol is drawn from the sheet with the most strokes, and a sheet in the running was not read whole."
            ),
        })),
        Reading::Failed(Failure::TooLarge(bytes)) => lines.push(marker(tf!(
            "Too large to be a symbol file ({}); not read.",
            human_size(*bytes)
        ))),
        Reading::Failed(Failure::NotCompound(_)) => lines.push(marker(t!(
            "Not a symbol file: it does not start as an OLE compound file."
        ))),
        Reading::Failed(Failure::Decode(error)) => lines.push(marker(tf!(
            "The symbol file cannot be decoded: {}",
            error
        ))),
    }
    if let Some(body) = &file.body {
        body_lines(body, lines);
    }
}

/// What a symbol body holds, as fields. A count of nothing is left out,
/// but for how many primitives are displayed and how many sheets there are.
fn body_lines(body: &Body, lines: &mut Vec<DetailLine>) {
    let field = |key: Cow<'_, str>, value: String| DetailLine::Field(key.into_owned(), value);
    if body.displayed.total() > 0 && body.sketch.is_empty() {
        lines.push(DetailLine::Marker(
            t!("The import draws none of these primitives.").into_owned(),
        ));
    }
    if let Some((min_x, min_y, max_x, max_y)) = body.sketch.bounds() {
        lines.push(field(
            t!("Symbol extent"),
            format!(
                "{} × {} mm",
                millimetres(max_x - min_x),
                millimetres(max_y - min_y)
            ),
        ));
    }
    lines.push(field(t!("Displayed primitives"), counted(&body.displayed)));
    if body.not_imported.total() > 0 {
        lines.push(field(
            t!("Not drawn by the import"),
            counted(&body.not_imported),
        ));
    }
    if !body.layers_shown.is_empty() {
        lines.push(field(t!("Layers shown"), layers(&body.layers_shown)));
    }
    if !body.layers_off.is_empty() {
        lines.push(field(t!("Layers off"), layers(&body.layers_off)));
    }
    if !body.on_sheets_not_drawn.is_empty() {
        lines.push(field(
            t!("Primitives on sheets not drawn"),
            format!(
                "{} ({})",
                held(&body.on_sheets_not_drawn),
                layers(&body.on_sheets_not_drawn)
            ),
        ));
    }
    lines.push(field(t!("Sheet count"), body.sheets.to_string()));
    if body.retired > 0 {
        lines.push(field(t!("Retired copies"), body.retired.to_string()));
    }
    if !body.faults.is_empty() {
        lines.push(field(t!("Chain faults"), body.faults.len().to_string()));
        lines.extend(body.faults.iter().cloned().map(DetailLine::Marker));
    }
}

/// A length in millimetres as the details state it: to a hundredth, less
/// the zeros that say nothing.
fn millimetres(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What a kind of primitive is called.
fn kind_name(kind: Kind) -> Cow<'static, str> {
    match kind {
        Kind::Line => t!("Line"),
        Kind::Circle => t!("Circle"),
        Kind::Arc => t!("Arc"),
        Kind::Polyline => t!("Polyline"),
        Kind::Spline => t!("Spline"),
        Kind::Fill => t!("Fill"),
        Kind::Text => t!("Text"),
    }
}

/// A count by kind as a value: how many in all, then every kind there is
/// one of with how many.
fn counted(counts: &Counts) -> String {
    let total = counts.total();
    if total == 0 {
        return "0".to_string();
    }
    let kinds: Vec<String> = counts
        .listed()
        .map(|(kind, count)| format!("{} {count}", kind_name(kind)))
        .collect();
    format!("{total} ({})", kinds.join(", "))
}

/// Layers as a value, each with how many primitives it holds: the named
/// ones in the order of their names, then the primitives that name none.
fn layers(list: &[(Option<String>, usize)]) -> String {
    let named = list
        .iter()
        .filter_map(|(name, count)| name.as_ref().map(|name| format!("{name} {count}")));
    let unnamed = list
        .iter()
        .filter(|(name, _)| name.is_none())
        .map(|(_, count)| format!("{} {count}", t!("(no layer)")));
    named.chain(unnamed).collect::<Vec<_>>().join(", ")
}

/// How many primitives the layers of `list` hold together.
fn held(list: &[(Option<String>, usize)]) -> usize {
    list.iter().map(|(_, count)| count).sum()
}

/// The thumbnail of `sketch`, as a line of the details.
pub(super) fn thumbnail<'a>(sketch: Arc<Sketch>) -> Element<'a, Message> {
    canvas(Thumbnail { sketch })
        .width(Length::Fill)
        .height(Length::Fixed(THUMBNAIL_H))
        .into()
}

/// What the tests see of the thumbnail's line: a key no field has.
#[cfg(test)]
pub(super) const SKETCH_KEY: &str = "<thumbnail>";

/// What the tests see of the thumbnail's line: [`SKETCH_KEY`], and how much
/// the sketch holds.
#[cfg(test)]
pub(super) fn sketch_pair(sketch: &Sketch) -> (String, String) {
    (
        SKETCH_KEY.to_string(),
        format!(
            "{} stroke(s), {} fill(s), {} label(s)",
            sketch.strokes.len(),
            sketch.fills.len(),
            sketch.labels.len()
        ),
    )
}

/// The canvas program that draws a sketch fitted to its box.
struct Thumbnail {
    sketch: Arc<Sketch>,
}

impl Program<Message> for Thumbnail {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        paint(
            &mut frame,
            &self.sketch,
            bounds.size(),
            theme.palette().background.base.text,
        );
        vec![frame.into_geometry()]
    }
}

/// How a rectangle of the sketch is laid into the box of the thumbnail: as
/// large as it gets with its proportions kept, centred, its y turned down
/// the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fit {
    /// Pixels a millimetre takes.
    scale: f32,
    /// The left edge of the rectangle, in millimetres.
    min_x: f64,
    /// Its upper edge, in millimetres.
    max_y: f64,
    /// Where that corner goes, in pixels.
    origin: (f32, f32),
}

impl Fit {
    /// Lay `bounds` -- `(min_x, min_y, max_x, max_y)` in millimetres, y up
    /// -- into a box `size` less [`PAD`] on every side. A rectangle with no
    /// extent along one axis (a horizontal line) is fitted along the other;
    /// one with none at all (a point) takes a pixel a millimetre.
    fn new(bounds: (f64, f64, f64, f64), size: Size) -> Self {
        let (min_x, min_y, max_x, max_y) = bounds;
        let inner_w = (size.width - 2.0 * PAD).max(1.0);
        let inner_h = (size.height - 2.0 * PAD).max(1.0);
        let span_x = max_x - min_x;
        let span_y = max_y - min_y;
        let across = f64::from(inner_w) / span_x;
        let down = f64::from(inner_h) / span_y;
        let scale = match (span_x > 0.0, span_y > 0.0) {
            (true, true) => across.min(down),
            (true, false) => across,
            (false, true) => down,
            (false, false) => 1.0,
        } as f32;
        Fit {
            scale,
            min_x,
            max_y,
            origin: (
                PAD + (inner_w - span_x as f32 * scale) * 0.5,
                PAD + (inner_h - span_y as f32 * scale) * 0.5,
            ),
        }
    }

    /// Where the point `(x, y)` of the sketch goes, in pixels.
    fn point(&self, (x, y): (f64, f64)) -> Point {
        Point::new(
            self.origin.0 + (x - self.min_x) as f32 * self.scale,
            self.origin.1 + (self.max_y - y) as f32 * self.scale,
        )
    }
}

/// How far above the point a label hangs from its baseline runs, in
/// heights of its capitals. This is the drawing's own rule for a line of
/// lettering (`justification_offset` of the text entity): on the point, a
/// descender's depth above it, half a capital below it, a capital below it.
fn baseline_rise(upright: Upright) -> f64 {
    match upright {
        Upright::Baseline => 0.0,
        Upright::Bottom => DESCENT_PER_CAP,
        Upright::Middle => -0.5,
        Upright::Top => -1.0,
    }
}

/// How far above the point a label hangs from the top of its first line
/// lies, in millimetres: its baseline where the drawing puts it, and on
/// that a line one em tall.
fn line_top(label: &Label) -> f64 {
    let em = label.height * EM_PER_CAP;
    baseline_rise(label.upright) * label.height + BASELINE_PER_EM * em
}

/// The corners of the box a label takes, as near as that is told without
/// the font it is drawn in: a letter six tenths of the em wide, a wide
/// character the whole of it, every line an em tall.
fn label_corners(label: &Label) -> [(f64, f64); 4] {
    let em = label.height * EM_PER_CAP;
    let widest = label
        .text
        .lines()
        .map(|line| {
            line.chars()
                .map(|letter| if letter.is_ascii() { 0.6 } else { 1.0 })
                .sum::<f64>()
        })
        .fold(0.0, f64::max);
    let width = widest * em;
    let (left, right) = match label.across {
        Across::Left => (0.0, width),
        Across::Center => (-width / 2.0, width / 2.0),
        Across::Right => (-width, 0.0),
    };
    let top = line_top(label);
    let bottom = top - em * label.text.lines().count().max(1) as f64;
    let (sin, cos) = label.rotation.sin_cos();
    [(left, bottom), (right, bottom), (right, top), (left, top)].map(|(x, y)| {
        (
            label.at.0 + x * cos - y * sin,
            label.at.1 + x * sin + y * cos,
        )
    })
}

/// The rectangle the thumbnail fits into its box: what the sketch covers
/// ([`Sketch::bounds`]) and how far its lettering reaches, so a label at
/// the edge is not cut off and a symbol that is lettering alone has a size.
fn reach(sketch: &Sketch) -> Option<(f64, f64, f64, f64)> {
    sketch
        .labels
        .iter()
        .flat_map(label_corners)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .fold(sketch.bounds(), |bounds, (x, y)| {
            Some(match bounds {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
            })
        })
}

/// The path through `points` as `fit` lays them, closed when `closed`. None
/// when there are fewer than two of them, or one of them is no number.
fn outline(fit: &Fit, points: &[(f64, f64)], closed: bool) -> Option<canvas::Path> {
    let (first, rest) = points.split_first()?;
    if rest.is_empty()
        || points
            .iter()
            .any(|(x, y)| !(x.is_finite() && y.is_finite()))
    {
        return None;
    }
    Some(canvas::Path::new(|builder| {
        builder.move_to(fit.point(*first));
        for point in rest {
            builder.line_to(fit.point(*point));
        }
        if closed {
            builder.close();
        }
    }))
}

/// Draw `sketch` fitted into a box of `size`: its fills, its strokes over
/// them, its lettering over both. What the file gives no colour of its own
/// is drawn in `foreground`.
fn paint(frame: &mut Frame, sketch: &Sketch, size: Size, foreground: iced::Color) {
    // The box the symbol is shown in.
    frame.stroke(
        &canvas::Path::rectangle(
            Point::new(0.5, 0.5),
            Size::new((size.width - 1.0).max(0.0), (size.height - 1.0).max(0.0)),
        ),
        canvas::Stroke::default()
            .with_color(foreground.scale_alpha(0.2))
            .with_width(1.0),
    );
    let Some(fit) = reach(sketch).map(|reach| Fit::new(reach, size)) else {
        return;
    };
    let color = |ink: Ink| match ink {
        Ink::Foreground => foreground,
        Ink::Rgb([r, g, b]) => iced::Color::from_rgb8(r, g, b),
    };
    for fill in &sketch.fills {
        if let Some(path) = outline(&fit, &fill.points, true) {
            frame.fill(&path, color(fill.ink));
        }
    }
    for stroke in &sketch.strokes {
        if let Some(path) = outline(&fit, &stroke.points, stroke.closed) {
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(color(stroke.ink))
                    .with_width(LINE_W),
            );
        }
    }
    for label in &sketch.labels {
        let text = lettering(label, fit.scale, color(label.ink));
        let anchored = label.at.0.is_finite() && label.at.1.is_finite();
        let legible = text.size.0.is_finite() && text.size.0 >= SMALLEST_LABEL;
        if !(anchored && legible && text.position.y.is_finite()) {
            continue;
        }
        let at = fit.point(label.at);
        frame.with_save(|frame| {
            frame.translate(iced::Vector::new(at.x, at.y));
            // The sketch turns counter-clockwise with its y up; the
            // screen's y is down. A label that is not turned stays text:
            // a turned one is drawn as the outlines of its letters.
            let turn = label.rotation.rem_euclid(TAU);
            if turn.is_finite() && turn > 1.0e-6 && TAU - turn > 1.0e-6 {
                frame.rotate(-(turn as f32));
            }
            frame.fill_text(text);
        });
    }
}

/// A label as the thumbnail letters it at `scale` pixels a millimetre, in
/// the frame laid at the point it hangs from and turned with it: the top of
/// its line where [`line_top`] puts it, so that its baseline runs where the
/// drawing's does.
fn lettering(label: &Label, scale: f32, color: iced::Color) -> canvas::Text {
    canvas::Text {
        content: label.text.clone(),
        position: Point::new(0.0, -(line_top(label) as f32) * scale),
        color,
        size: iced::Pixels((label.height * EM_PER_CAP) as f32 * scale),
        line_height: iced::advanced::text::LineHeight::Relative(1.0),
        align_x: match label.across {
            Across::Left => iced::advanced::text::Alignment::Left,
            Across::Center => iced::advanced::text::Alignment::Center,
            Across::Right => iced::advanced::text::Alignment::Right,
        },
        align_y: iced::alignment::Vertical::Top,
        shaping: iced::advanced::text::Shaping::Advanced,
        ..canvas::Text::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_backup_browse::symbol_file::{digest, made_up};
    use crate::io::pid_backup_browse::{build_index, build_index_in, RefNode};
    use crate::io::pid_sym_census::{inspect_sym_bytes, CFB_MAGIC};
    use crate::ui::dock::Chrome;
    use crate::ui::window::pid_backup_browser::section::load_in_background;
    use crate::ui::window::pid_backup_browser::{
        detail_rows, update, view, Effect, PidBackupBrowser, PidBackupMsg,
    };
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// What does not open the way a compound file does.
    const PLAIN: &[u8] = b"PLAIN-ASCII-NOT-CFB";

    /// What opens the way a compound file does and holds none.
    fn hollow() -> Vec<u8> {
        let mut bytes = CFB_MAGIC.to_vec();
        bytes.extend([0u8; 256]);
        bytes
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ocs-pid-backup-symbol-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in members {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    /// A backup folder on disk with a symbol file in every place the tree
    /// shows one, indexed the way the application indexes one.
    fn backup(name: &str) -> (PathBuf, PidBackupBrowser) {
        let root = scratch(name);
        let write = |name: &str, bytes: &[u8]| std::fs::write(root.join(name), bytes).unwrap();
        write(
            "Manifest.txt",
            b"Name<<|>>SQPlant\n\
              File<<|>>4<<|>>2<<|>>810<<|>>20810<<|>>\\\\host\\P\\Ref\\Loose.sym<<|>>1<<|>>RefData~4~810\n\
              File<<|>>4<<|>>1<<|>>809<<|>>20809<<|>>\\\\host\\P\\Ref\\Symbols<<|>>1<<|>>RefData~4~809.zip\n",
        );
        write("Export.dmp", b"\x03\x03iEXPORT:V12.02.00\nrest-of-dump");
        write("RefData~4~810", PLAIN);
        let hollow = hollow();
        write_zip(
            &root.join("RefData~4~809.zip"),
            &[
                ("Symbols/", b""),
                ("Symbols/hollow.sym", &hollow),
                ("Symbols/plain.SYM", PLAIN),
                ("Symbols/notes.txt", b"notes\n"),
            ],
        );
        std::fs::create_dir_all(root.join("Ref").join("Symbols")).unwrap();
        std::fs::write(root.join("Ref").join("Symbols").join("Gate.sym"), &hollow).unwrap();
        let index = build_index(&root, None).expect("the folder indexes");
        let mut state = PidBackupBrowser::default();
        update(&mut state, PidBackupMsg::Loaded(Ok(Arc::new(index))));
        (root, state)
    }

    fn file(state: &PidBackupBrowser, name: &str) -> NodeId {
        let index = state.index().expect("a ready index");
        NodeId::File(
            index
                .files
                .iter()
                .position(|file| file.name == name)
                .unwrap_or_else(|| panic!("{name} is a file of the backup")),
        )
    }

    fn member(state: &PidBackupBrowser, archive: &str, name: &str) -> NodeId {
        let NodeId::File(at) = file(state, archive) else {
            unreachable!()
        };
        let index = state.index().expect("a ready index");
        NodeId::FileMember(
            at,
            index.files[at]
                .members
                .iter()
                .position(|member| member == name)
                .unwrap_or_else(|| panic!("{name} is a member of {archive}")),
        )
    }

    /// Select `node` and answer the read that starts: the generation it is
    /// of and the section that asked. A file with content starts one.
    fn start(state: &mut PidBackupBrowser, node: &NodeId) -> (u64, usize) {
        match update(state, PidBackupMsg::Select(node.clone())) {
            Effect::LoadSection {
                generation,
                section,
                node: asked,
            } => {
                assert_eq!(&asked, node);
                (generation, section)
            }
            other => panic!("selecting a file starts one read: {other:?}"),
        }
    }

    /// Hand `result` back as what the read `read` of `node` gave.
    fn finish(
        state: &mut PidBackupBrowser,
        read: (u64, usize),
        node: &NodeId,
        result: Result<SectionData, String>,
    ) {
        update(
            state,
            PidBackupMsg::SectionLoaded {
                generation: read.0,
                section: read.1,
                node: node.clone(),
                result,
            },
        );
    }

    /// Select `node` and run the read it starts the way the application
    /// does.
    fn select(state: &mut PidBackupBrowser, node: &NodeId) {
        let read = start(state, node);
        let index = state.index().cloned().expect("a ready index");
        let result = iced::futures::executor::block_on(load_in_background(
            state.sections,
            index,
            read.1,
            node.clone(),
        ));
        finish(state, read, node, result);
    }

    /// Select `node` and hand `file` back as what was read of it.
    fn show(state: &mut PidBackupBrowser, node: &NodeId, file: SymbolFile) {
        let read = start(state, node);
        finish(state, read, node, Ok(SectionData::new(file)));
    }

    fn value<'a>(rows: &'a [(String, String)], key: &str) -> Option<&'a str> {
        rows.iter()
            .find(|(found, _)| found == key)
            .map(|(_, value)| value.as_str())
    }

    /// Whether `rows` hold the plain line `text`.
    fn says(rows: &[(String, String)], text: &str) -> bool {
        rows.iter()
            .any(|(key, value)| key == text && value.is_empty())
    }

    #[test]
    fn a_symbol_file_is_read_as_a_symbol_wherever_it_lies() {
        let (root, mut state) = backup("places");
        let not_text = t!("Not a text file; its content is not shown.");
        let not_compound = t!("Not a symbol file: it does not start as an OLE compound file.");
        let undecoded = tf!("The symbol file cannot be decoded: {}", "").into_owned();
        // A member of an archive; one named in capitals; a file of the
        // backup folder that was a symbol where it was backed up from; a
        // file of the `Ref` tree.
        let nodes = [
            (
                member(&state, "RefData~4~809.zip", "Symbols/hollow.sym"),
                false,
            ),
            (
                member(&state, "RefData~4~809.zip", "Symbols/plain.SYM"),
                true,
            ),
            (file(&state, "RefData~4~810"), true),
            (NodeId::RefFile(vec![0], 0), false),
        ];
        for (node, plain) in &nodes {
            assert!(
                is_symbol(state.index().expect("a ready index"), node),
                "{node:?}"
            );
            select(&mut state, node);
            let rows = detail_rows(&state, node);
            assert!(!says(&rows, &not_text), "{node:?}: {rows:?}");
            let said = if *plain {
                says(&rows, &not_compound)
            } else {
                // The conversion library's own words follow the colon.
                rows.iter().any(|(key, value)| {
                    key.starts_with(&undecoded) && key.len() > undecoded.len() && value.is_empty()
                })
            };
            assert!(said, "{node:?}: {rows:?}");
            // A file that does not decode has no thumbnail and no fields.
            assert_eq!(value(&rows, SKETCH_KEY), None);
            assert_eq!(value(&rows, &t!("Displayed primitives")), None);
        }

        // What is not named as a symbol is the text preview's as before.
        let notes = member(&state, "RefData~4~809.zip", "Symbols/notes.txt");
        assert!(!is_symbol(state.index().expect("a ready index"), &notes));
        select(&mut state, &notes);
        assert_eq!(
            value(&detail_rows(&state, &notes), &t!("Line count")),
            Some("1")
        );
        // Neither a folder of an archive nor the archive is read at all.
        for node in [
            member(&state, "RefData~4~809.zip", "Symbols/"),
            file(&state, "RefData~4~809.zip"),
            NodeId::RefDir(vec![0]),
        ] {
            assert!(
                matches!(
                    update(&mut state, PidBackupMsg::Select(node.clone())),
                    Effect::None | Effect::MeasureCache { .. }
                ),
                "{node:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_read_is_shown_while_it_runs_and_an_archive_that_went_away_says_why() {
        let (root, mut state) = backup("failed");
        let node = member(&state, "RefData~4~809.zip", "Symbols/hollow.sym");
        let read = start(&mut state, &node);
        assert!(says(&detail_rows(&state, &node), &t!("Reading the file…")));

        // The archive went away after the backup was indexed.
        std::fs::remove_file(root.join("RefData~4~809.zip")).unwrap();
        let index = state.index().cloned().expect("a ready index");
        let result = iced::futures::executor::block_on(load_in_background(
            state.sections,
            index,
            read.1,
            node.clone(),
        ));
        assert!(result.is_err());
        finish(&mut state, read, &node, result);
        let rows = detail_rows(&state, &node);
        let reason = rows
            .iter()
            .find(|(key, value)| key.contains("RefData~4~809.zip") && value.is_empty())
            .unwrap_or_else(|| panic!("the reason is shown: {rows:?}"));
        assert!(reason.0.contains(&root.display().to_string()), "{reason:?}");
        assert!(!says(&rows, &t!("Reading the file…")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_a_symbol_holds_is_listed_under_its_thumbnail() {
        let (root, mut state) = backup("listed");
        let node = member(&state, "RefData~4~809.zip", "Symbols/hollow.sym");
        show(&mut state, &node, digest(&made_up::valve()));
        let rows = detail_rows(&state, &node);
        let at = |key: &str| {
            rows.iter()
                .position(|(found, _)| found == key)
                .unwrap_or_else(|| panic!("{key} is shown: {rows:?}"))
        };
        // The file's own details, the thumbnail, then what the file holds.
        assert_eq!(at(&t!("Backup file name")), 0);
        assert!(at(&t!("Member")) < at(SKETCH_KEY));
        assert!(at(SKETCH_KEY) < at(&t!("Symbol extent")));
        assert_eq!(
            value(&rows, SKETCH_KEY),
            Some("5 stroke(s), 1 fill(s), 1 label(s)")
        );
        assert_eq!(value(&rows, &t!("Symbol extent")), Some("10 × 6 mm"));
        assert_eq!(
            value(&rows, &t!("Displayed primitives")),
            Some(
                format!(
                    "8 ({} 1, {} 1, {} 1, {} 1, {} 1, {} 1, {} 2)",
                    t!("Line"),
                    t!("Circle"),
                    t!("Arc"),
                    t!("Polyline"),
                    t!("Spline"),
                    t!("Fill"),
                    t!("Text")
                )
                .as_str()
            )
        );
        assert_eq!(
            value(&rows, &t!("Not drawn by the import")),
            Some(format!("1 ({} 1)", t!("Text")).as_str())
        );
        assert_eq!(
            value(&rows, &t!("Layers shown")),
            Some(format!("Default 6, {} 2", t!("(no layer)")).as_str())
        );
        assert_eq!(value(&rows, &t!("Layers off")), Some("Label 1"));
        assert_eq!(
            value(&rows, &t!("Primitives on sheets not drawn")),
            Some("2 (Default 2)")
        );
        assert_eq!(value(&rows, &t!("Sheet count")), Some("2"));
        assert_eq!(value(&rows, &t!("Retired copies")), Some("3"));
        assert_eq!(value(&rows, &t!("Chain faults")), None);
        // A symbol that reads whole and is drawn has no more to say: every
        // line is a field or the thumbnail.
        assert!(rows.iter().all(|(_, value)| !value.is_empty()), "{rows:?}");

        // A stream whose framing does not hold is said in the conversion
        // library's words, under how many there are. (The node is left and
        // come back to: selecting the node it is on again starts no read.)
        update(&mut state, PidBackupMsg::Select(NodeId::Root));
        show(&mut state, &node, digest(&made_up::faulty()));
        let rows = detail_rows(&state, &node);
        assert_eq!(value(&rows, &t!("Chain faults")), Some("1"));
        let wrong = format!("0x{:08X}", made_up::WRONG_MAGIC);
        assert!(
            rows.iter()
                .any(|(key, value)| key.contains(&wrong) && value.is_empty()),
            "{rows:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_symbol_that_is_not_drawn_says_why_and_has_no_thumbnail() {
        let (root, mut state) = backup("not-drawn");
        let node = member(&state, "RefData~4~809.zip", "Symbols/hollow.sym");
        // Selecting the node it is on again starts no read: the node is
        // left and come back to between the cases.
        let shown = |state: &mut PidBackupBrowser, file: SymbolFile| {
            update(state, PidBackupMsg::Select(NodeId::Root));
            show(state, &node, file);
            detail_rows(state, &node)
        };

        // Nothing in it.
        let rows = shown(&mut state, digest(&made_up::empty()));
        assert!(
            says(
                &rows,
                &t!("Nothing to draw: the symbol displays no primitive.")
            ),
            "{rows:?}"
        );
        assert_eq!(value(&rows, SKETCH_KEY), None);
        assert_eq!(value(&rows, &t!("Symbol extent")), None);
        assert_eq!(value(&rows, &t!("Displayed primitives")), Some("0"));
        assert_eq!(value(&rows, &t!("Sheet count")), Some("0"));
        for absent in [
            t!("Not drawn by the import"),
            t!("Layers shown"),
            t!("Layers off"),
            t!("Primitives on sheets not drawn"),
            t!("Retired copies"),
        ] {
            assert_eq!(value(&rows, &absent), None, "{absent}");
        }

        // A primitive is displayed, and the import draws nothing for it.
        let rows = shown(&mut state, digest(&made_up::placeholder()));
        assert!(
            says(&rows, &t!("The import draws none of these primitives.")),
            "{rows:?}"
        );
        assert_eq!(value(&rows, SKETCH_KEY), None);
        let one_text = format!("1 ({} 1)", t!("Text"));
        assert_eq!(
            value(&rows, &t!("Displayed primitives")),
            Some(one_text.as_str())
        );
        assert_eq!(
            value(&rows, &t!("Not drawn by the import")),
            Some(one_text.as_str())
        );

        // The stream that says what draws the symbol was read short.
        let rows = shown(
            &mut state,
            digest(&made_up::valve_short(&["PSMcluster0"], &[])),
        );
        assert!(
            rows.iter()
                .any(|(key, value)| { key.contains("PSMcluster0") && value.is_empty() }),
            "{rows:?}"
        );
        assert_eq!(value(&rows, SKETCH_KEY), None);
        assert_eq!(value(&rows, &t!("Displayed primitives")), Some("0"));
        // No sheet is drawn, so the whole body is listed as on one that is
        // not, by its layers.
        assert_eq!(
            value(&rows, &t!("Primitives on sheets not drawn")),
            Some("11 (Default 8, Label 1, (no layer) 2)")
        );

        // The sheet that draws it was read short: what was read is drawn,
        // and said to be a part.
        let rows = shown(
            &mut state,
            digest(&made_up::valve_short(&[], &[made_up::NORMAL])),
        );
        assert!(
            says(
                &rows,
                &t!("Part of the file was not read; what is shown is part of the symbol.")
            ),
            "{rows:?}"
        );
        assert!(value(&rows, SKETCH_KEY).is_some());

        // Too long to be a symbol.
        let rows = shown(
            &mut state,
            SymbolFile {
                reading: Reading::Failed(Failure::TooLarge(17 << 20)),
                body: None,
            },
        );
        assert!(
            says(
                &rows,
                &tf!(
                    "Too large to be a symbol file ({}); not read.",
                    human_size(17 << 20)
                )
            ),
            "{rows:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn of_a_run_of_quick_clicks_only_the_last_symbol_is_shown() {
        let (root, mut state) = backup("quick");
        let first = member(&state, "RefData~4~809.zip", "Symbols/hollow.sym");
        let second = member(&state, "RefData~4~809.zip", "Symbols/plain.SYM");
        // Two clicks before the first read comes back.
        let first_read = start(&mut state, &first);
        let second_read = start(&mut state, &second);
        assert_eq!(state.section_data.len(), 1);
        // It comes back late: nobody waits for it, and it is not kept.
        finish(
            &mut state,
            first_read,
            &first,
            Ok(SectionData::new(digest(&made_up::valve()))),
        );
        assert_eq!(state.section_data.len(), 1);
        assert_eq!(value(&detail_rows(&state, &first), SKETCH_KEY), None);
        assert!(says(
            &detail_rows(&state, &second),
            &t!("Reading the file…")
        ));
        finish(
            &mut state,
            second_read,
            &second,
            Ok(SectionData::new(digest(&made_up::valve()))),
        );
        assert!(value(&detail_rows(&state, &second), SKETCH_KEY).is_some());
        assert_eq!(value(&detail_rows(&state, &first), SKETCH_KEY), None);

        // Back on the first: it is read again, and the second let go of.
        show(&mut state, &first, digest(&made_up::empty()));
        assert_eq!(state.section_data.len(), 1);
        assert_eq!(value(&detail_rows(&state, &second), SKETCH_KEY), None);
        assert_eq!(
            value(&detail_rows(&state, &first), &t!("Displayed primitives")),
            Some("0")
        );
        // A node that is no file lets go of the last one.
        update(&mut state, PidBackupMsg::Select(NodeId::Root));
        assert!(state.section_data.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn close(found: Point, expected: (f32, f32)) -> bool {
        (found.x - expected.0).abs() < 1.0e-3 && (found.y - expected.1).abs() < 1.0e-3
    }

    #[test]
    fn a_sketch_is_fitted_into_its_box_with_its_proportions_kept() {
        // A box 200 by 160 leaves 184 by 144 inside its margins.
        let size = Size::new(200.0, 160.0);

        // Wider than the box is: the width decides, and it is centred
        // down the box. The upper left corner of the sketch is the upper
        // left of what is drawn.
        let fit = Fit::new((0.0, 0.0, 10.0, 4.0), size);
        assert!((fit.scale - 18.4).abs() < 1.0e-4, "{fit:?}");
        assert!(close(fit.point((0.0, 4.0)), (8.0, 43.2)), "{fit:?}");
        assert!(close(fit.point((10.0, 0.0)), (192.0, 116.8)), "{fit:?}");

        // Taller: the height decides.
        let fit = Fit::new((0.0, 0.0, 1.0, 100.0), size);
        assert!((fit.scale - 1.44).abs() < 1.0e-4, "{fit:?}");
        assert!(close(fit.point((0.0, 100.0)), (99.28, 8.0)), "{fit:?}");
        assert!(close(fit.point((1.0, 0.0)), (100.72, 152.0)), "{fit:?}");

        // Where the sketch is on its sheet does not matter.
        let moved = Fit::new((-500.0, 200.0, -490.0, 204.0), size);
        assert!(
            close(moved.point((-500.0, 204.0)), (8.0, 43.2)),
            "{moved:?}"
        );

        // A horizontal line has no height: it runs across the middle.
        let fit = Fit::new((0.0, 3.0, 10.0, 3.0), size);
        assert!(close(fit.point((0.0, 3.0)), (8.0, 80.0)), "{fit:?}");
        assert!(close(fit.point((10.0, 3.0)), (192.0, 80.0)), "{fit:?}");
        // A vertical one no width.
        let fit = Fit::new((2.0, 0.0, 2.0, 10.0), size);
        assert!(close(fit.point((2.0, 10.0)), (100.0, 8.0)), "{fit:?}");
        assert!(close(fit.point((2.0, 0.0)), (100.0, 152.0)), "{fit:?}");
        // A point is the middle of the box.
        let fit = Fit::new((5.0, 5.0, 5.0, 5.0), size);
        assert!((fit.scale - 1.0).abs() < 1.0e-6, "{fit:?}");
        assert!(close(fit.point((5.0, 5.0)), (100.0, 80.0)), "{fit:?}");

        // A box smaller than its margins still gives numbers.
        let fit = Fit::new((0.0, 0.0, 10.0, 4.0), Size::new(4.0, 4.0));
        assert!(fit.scale.is_finite() && fit.scale > 0.0, "{fit:?}");
    }

    #[test]
    fn the_lettering_of_a_sketch_is_inside_what_is_fitted() {
        use crate::io::pid_backup_browse::symbol_file::Stroke;
        let label = |rotation: f64, across: Across, upright: Upright| Label {
            text: "PG".to_string(),
            at: (0.0, 0.0),
            height: 2.5,
            rotation,
            across,
            upright,
            ink: Ink::Foreground,
        };
        let reach_of = |label: Label| {
            reach(&Sketch {
                labels: vec![label],
                ..Sketch::default()
            })
            .expect("a label has a reach")
        };
        let near = |found: (f64, f64, f64, f64), expected: (f64, f64, f64, f64)| {
            (found.0 - expected.0).abs() < 1.0e-9
                && (found.1 - expected.1).abs() < 1.0e-9
                && (found.2 - expected.2).abs() < 1.0e-9
                && (found.3 - expected.3).abs() < 1.0e-9
        };
        // Two letters of 2.5 mm capitals: an em of 3.5 mm, 4.2 mm wide, on
        // a line whose top is 3.08 mm above its baseline. A symbol that is
        // lettering alone has a size. Standing on the point it hangs from:
        let found = reach_of(label(0.0, Across::Left, Upright::Baseline));
        assert!(near(found, (0.0, -0.42, 4.2, 3.08)), "{found:?}");
        // Hung from the foot of its descenders it is three tenths of a
        // capital higher, and the point is kept in what is fitted.
        let found = reach_of(label(0.0, Across::Left, Upright::Bottom));
        assert!(near(found, (0.0, 0.0, 4.2, 3.83)), "{found:?}");
        // From the middle of its capitals, and from their top.
        let found = reach_of(label(0.0, Across::Center, Upright::Middle));
        assert!(near(found, (-2.1, -1.67, 2.1, 1.83)), "{found:?}");
        let found = reach_of(label(0.0, Across::Right, Upright::Top));
        assert!(near(found, (-4.2, -2.92, 0.0, 0.58)), "{found:?}");
        // Turned a quarter, it runs up the sheet.
        let found = reach_of(label(
            std::f64::consts::FRAC_PI_2,
            Across::Left,
            Upright::Bottom,
        ));
        assert!(near(found, (-3.83, 0.0, 0.0, 4.2)), "{found:?}");
        // A label of two lines takes two of them, down from its first and
        // as wide as its longer one; a wide character takes the whole of
        // its em.
        let worded = |text: &str| Label {
            text: text.to_string(),
            ..label(0.0, Across::Left, Upright::Baseline)
        };
        let found = reach_of(worded("X\nPG"));
        assert!(near(found, (0.0, -3.92, 4.2, 3.08)), "{found:?}");
        let found = reach_of(worded("阀"));
        assert!(near(found, (0.0, -0.42, 3.5, 3.08)), "{found:?}");

        // Beside a stroke, the two together.
        let sketch = Sketch {
            strokes: vec![Stroke {
                points: vec![(-10.0, -1.0), (1.0, 1.0)],
                closed: false,
                ink: Ink::Foreground,
            }],
            labels: vec![label(0.0, Across::Left, Upright::Bottom)],
            ..Sketch::default()
        };
        assert_eq!(sketch.bounds(), Some((-10.0, -1.0, 1.0, 1.0)));
        let found = reach(&sketch).expect("a reach");
        assert!(near(found, (-10.0, -1.0, 4.2, 3.83)), "{found:?}");

        // Nothing drawn, nothing to fit.
        assert_eq!(reach(&Sketch::default()), None);
    }

    #[test]
    fn a_label_is_lettered_with_its_baseline_where_the_drawing_puts_it() {
        let label = |upright: Upright| Label {
            text: "PG".to_string(),
            at: (0.0, 0.0),
            height: 2.5,
            rotation: 0.0,
            across: Across::Center,
            upright,
            ink: Ink::Foreground,
        };
        let drawn = |upright: Upright| lettering(&label(upright), 2.0, iced::Color::BLACK);
        // At two pixels a millimetre: capitals of 5 px, an em of 7 px, and
        // a baseline 6.16 px under the top of its line.
        let text = drawn(Upright::Baseline);
        assert_eq!(text.content, "PG");
        assert!((text.size.0 - 7.0).abs() < 1.0e-4, "{:?}", text.size);
        assert!(matches!(text.align_y, iced::alignment::Vertical::Top));
        assert!(matches!(
            text.align_x,
            iced::advanced::text::Alignment::Center
        ));
        // Standing on the point, the top of its line is that far above it.
        let top = |upright: Upright| drawn(upright).position.y;
        assert!(text.position.x.abs() < 1.0e-6);
        assert!((top(Upright::Baseline) + 6.16).abs() < 1.0e-4);
        // Hung from the foot of its descenders it is 1.5 px higher; from
        // the middle of its capitals half of them lower, from their top
        // all of them.
        assert!((top(Upright::Bottom) + 7.66).abs() < 1.0e-4);
        assert!((top(Upright::Middle) + 3.66).abs() < 1.0e-4);
        assert!((top(Upright::Top) + 1.16).abs() < 1.0e-4);
    }

    /// The thumbnail takes its room among the drawn lines: what the file
    /// holds starts its height below the file's own details.
    #[test]
    fn the_thumbnail_is_drawn_between_the_files_own_details_and_what_it_holds() {
        let _guard = crate::headless_ui_lock();
        let (root, mut state) = backup("drawn");
        let node = member(&state, "RefData~4~809.zip", "Symbols/hollow.sym");
        show(&mut state, &node, digest(&made_up::valve()));

        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (340.0, 700.0),
            view(&state, 340.0, Chrome::default()),
        );
        let mut top = |label: &str| -> f32 {
            ui.find(label)
                .unwrap_or_else(|error| panic!("{label} is drawn: {error:?}"))
                .visible_bounds()
                .unwrap_or_else(|| panic!("{label} is on screen"))
                .y
        };
        let own = top(t!("Member").as_ref());
        let extent = top(t!("Symbol extent").as_ref());
        let displayed = top(t!("Displayed primitives").as_ref());
        assert!(extent - own >= THUMBNAIL_H, "{own} {extent}");
        assert!(displayed > extent && displayed - extent < THUMBNAIL_H);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The files of the `Ref` tree under `node`, `dirs` being the way to it.
    fn ref_files(node: &RefNode, dirs: &mut Vec<usize>, found: &mut Vec<NodeId>) {
        for at in 0..node.files.len() {
            found.push(NodeId::RefFile(dirs.clone(), at));
        }
        for (at, dir) in node.dirs.iter().enumerate() {
            dirs.push(at);
            ref_files(dir, dirs, found);
            dirs.pop();
        }
    }

    /// What the symbol files of one place of a backup read as.
    #[derive(Default)]
    struct Tally {
        files: usize,
        drawable: usize,
        empty: usize,
        short: usize,
        failed: usize,
        thumbnails: usize,
        displayed: [usize; Kind::ALL.len()],
        not_imported: [usize; Kind::ALL.len()],
        with_not_imported: usize,
        with_layers_off: usize,
        with_other_sheets: usize,
        with_retired: usize,
        with_faults: usize,
        strokes: usize,
        fills: usize,
        labels: usize,
        widest: f64,
        tallest: f64,
        slowest: Option<(std::time::Duration, String)>,
        /// The files that are not plainly drawn, each with how it reads.
        notable: Vec<String>,
        /// The files the import draws nothing for a primitive of, each with
        /// which.
        not_imported_in: Vec<String>,
        /// How many primitives the sheets that are not drawn hold, by the
        /// layer they are on.
        other_sheet_layers: BTreeMap<String, usize>,
        /// The files one of whose sheets that are not drawn holds
        /// primitives on a layer the drawn sheet shows, each with how many.
        other_sheet_on_shown: Vec<(usize, String)>,
    }

    fn add(into: &mut [usize; Kind::ALL.len()], counts: &Counts) {
        for (slot, kind) in into.iter_mut().zip(Kind::ALL) {
            *slot += counts.of(kind);
        }
    }

    fn by_kind(counts: &[usize; Kind::ALL.len()]) -> String {
        let listed: Vec<String> = Kind::ALL
            .iter()
            .zip(counts)
            .filter(|(_, count)| **count > 0)
            .map(|(kind, count)| format!("{kind:?} {count}"))
            .collect();
        if listed.is_empty() {
            "none".to_string()
        } else {
            listed.join(", ")
        }
    }

    /// Every symbol file of a real backup through the read the panel makes
    /// of one, each against what the symbol census (issue #114) says of the
    /// same bytes, with the totals printed.
    ///
    /// The backup is the folder or the zip `OCS_PID_SYM_PREVIEW_BACKUP`
    /// names, or the plant package beside the repository, whose archive of
    /// symbols must read as the census counted it: 644 files, 643 drawn and
    /// one with nothing in it.
    ///
    /// ```text
    /// cargo test --lib every_symbol_of_a_real_backup_is_previewed -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "reads a real backup: ../pid-test-data, or the one OCS_PID_SYM_PREVIEW_BACKUP names"]
    fn every_symbol_of_a_real_backup_is_previewed() {
        let named = std::env::var_os("OCS_PID_SYM_PREVIEW_BACKUP").map(PathBuf::from);
        let plant = named.is_none();
        let Some(source_path) = named.or_else(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()?
                .join("pid-test-data");
            root.join("Export.dmp").is_file().then_some(root)
        }) else {
            println!("skipped: there is no backup to read");
            return;
        };
        // Whatever has to be unpacked goes where the test clears it away.
        let cache = scratch("real");
        let index = build_index_in(&source_path, None, Some(&cache)).expect("the backup indexes");

        let mut nodes: Vec<(String, String, NodeId)> = Vec::new();
        for (at, file) in index.files.iter().enumerate() {
            nodes.push((
                "backup folder".to_string(),
                file.name.clone(),
                NodeId::File(at),
            ));
            for (member, name) in file.members.iter().enumerate() {
                nodes.push((
                    file.name.clone(),
                    name.clone(),
                    NodeId::FileMember(at, member),
                ));
            }
        }
        let mut in_ref = Vec::new();
        ref_files(&index.ref_tree, &mut Vec::new(), &mut in_ref);
        for node in in_ref {
            let NodeId::RefFile(dirs, at) = &node else {
                unreachable!()
            };
            let name = index
                .ref_file_path(dirs, *at)
                .and_then(|path| {
                    path.strip_prefix(&index.ref_root)
                        .ok()
                        .map(|path| path.display().to_string())
                })
                .unwrap_or_default();
            nodes.push(("Ref tree".to_string(), name, node));
        }
        nodes.retain(|(_, _, node)| source(&index, node).is_some());

        let box_size = Size::new(340.0, THUMBNAIL_H);
        let started = std::time::Instant::now();
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        for (place, name, node) in &nodes {
            let asked = std::time::Instant::now();
            let data = SymbolPreview
                .load(&index, node)
                .unwrap_or_else(|error| panic!("{place}: {name}: {error}"));
            let took = asked.elapsed();
            let file = data.get::<SymbolFile>().expect("a symbol file was read");

            // The same bytes, read here, as the census counts them.
            let bytes = match content(&index, node).expect("the node has content") {
                Source::File(path) => std::fs::read(&path).expect("the file reads"),
                Source::Member {
                    archive, member, ..
                } => {
                    let archive = std::fs::File::open(&archive).expect("the archive opens");
                    let mut zip = zip::ZipArchive::new(archive).expect("the archive reads");
                    let mut entry = zip.by_index(member).expect("the member is there");
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut bytes).expect("the member reads");
                    bytes
                }
            };
            let census = inspect_sym_bytes(place, name, &bytes, None);
            assert_eq!(
                file.reading.census_status(),
                census.status,
                "{place}: {name}"
            );
            let (displayed, total) = file.body.as_ref().map_or((0, 0), |body| {
                let off: usize = body.layers_off.iter().map(|(_, count)| count).sum();
                let displayed = body.displayed.total();
                (displayed, displayed + off + held(&body.on_sheets_not_drawn))
            });
            assert_eq!(
                (displayed, total),
                (census.displayed_primitives, census.total_primitives),
                "{place}: {name}"
            );

            // The lines the details get, and the thumbnail's every point
            // inside its box.
            let mut lines = Vec::new();
            lines_of(file, &mut lines);
            let thumbnails = lines
                .iter()
                .filter(|line| matches!(line, DetailLine::Sketch(_)))
                .count();
            assert!(thumbnails <= 1, "{place}: {name}");
            assert!(lines.len() > thumbnails, "{place}: {name}");

            let tally = tallies.entry(place.clone()).or_default();
            tally.files += 1;
            tally.thumbnails += thumbnails;
            match &file.reading {
                Reading::Drawable => tally.drawable += 1,
                Reading::EmptyBody => tally.empty += 1,
                Reading::Short(_) => tally.short += 1,
                Reading::Failed(_) => tally.failed += 1,
            }
            if file.reading != Reading::Drawable || thumbnails == 0 {
                tally.notable.push(format!(
                    "{name}: {:?}, {thumbnails} thumbnail(s)",
                    file.reading
                ));
            }
            if tally.slowest.as_ref().is_none_or(|(most, _)| took > *most) {
                tally.slowest = Some((took, name.clone()));
            }
            let Some(body) = &file.body else {
                continue;
            };
            add(&mut tally.displayed, &body.displayed);
            add(&mut tally.not_imported, &body.not_imported);
            if body.not_imported.total() > 0 {
                let mut which = [0; Kind::ALL.len()];
                add(&mut which, &body.not_imported);
                tally.with_not_imported += 1;
                tally
                    .not_imported_in
                    .push(format!("{name}: {}", by_kind(&which)));
            }
            tally.with_layers_off += usize::from(!body.layers_off.is_empty());
            tally.with_other_sheets += usize::from(!body.on_sheets_not_drawn.is_empty());
            for (layer, count) in &body.on_sheets_not_drawn {
                let layer = layer.clone().unwrap_or_else(|| "(no layer)".to_string());
                *tally.other_sheet_layers.entry(layer).or_default() += count;
            }
            let on_shown: usize = body
                .on_sheets_not_drawn
                .iter()
                .filter(|(layer, _)| body.layers_shown.iter().any(|(shown, _)| shown == layer))
                .map(|(_, count)| count)
                .sum();
            if on_shown > 0 {
                tally.other_sheet_on_shown.push((
                    on_shown,
                    format!(
                        "{name}: {on_shown} of {} not drawn; {} displayed",
                        held(&body.on_sheets_not_drawn),
                        body.displayed.total()
                    ),
                ));
            }
            tally.with_retired += usize::from(body.retired > 0);
            tally.with_faults += usize::from(!body.faults.is_empty());
            tally.strokes += body.sketch.strokes.len();
            tally.fills += body.sketch.fills.len();
            tally.labels += body.sketch.labels.len();
            if let Some((min_x, min_y, max_x, max_y)) = body.sketch.bounds() {
                tally.widest = tally.widest.max(max_x - min_x);
                tally.tallest = tally.tallest.max(max_y - min_y);
            }
            if let Some(bounds) = reach(&body.sketch) {
                let fit = Fit::new(bounds, box_size);
                assert!(fit.scale.is_finite() && fit.scale > 0.0, "{place}: {name}");
                let strokes = body.sketch.strokes.iter().flat_map(|stroke| &stroke.points);
                let fills = body.sketch.fills.iter().flat_map(|fill| &fill.points);
                for point in strokes.chain(fills) {
                    let at = fit.point(*point);
                    assert!(
                        (-0.5..=box_size.width + 0.5).contains(&at.x)
                            && (-0.5..=box_size.height + 0.5).contains(&at.y),
                        "{place}: {name}: {point:?} is drawn at {at:?}"
                    );
                }
            }
        }
        let took = started.elapsed();

        let mut files = 0;
        for (place, tally) in &tallies {
            files += tally.files;
            println!(
                "{place}: {} symbol file(s): {} drawable, {} empty, {} read short, {} failed; {} thumbnail(s)",
                tally.files, tally.drawable, tally.empty, tally.short, tally.failed, tally.thumbnails
            );
            println!("  displayed primitives: {}", by_kind(&tally.displayed));
            println!(
                "  the import draws nothing for: {} (in {} file(s))",
                by_kind(&tally.not_imported),
                tally.with_not_imported
            );
            for line in &tally.not_imported_in {
                println!("    {line}");
            }
            println!(
                "  file(s) with layers off: {}; with primitives on sheets not drawn: {}; with retired copies: {}; with chain faults: {}",
                tally.with_layers_off, tally.with_other_sheets, tally.with_retired, tally.with_faults
            );
            let by_layer: Vec<String> = tally
                .other_sheet_layers
                .iter()
                .map(|(layer, count)| format!("{layer} {count}"))
                .collect();
            println!(
                "  primitives on sheets not drawn, by layer: {}",
                if by_layer.is_empty() {
                    "none".to_string()
                } else {
                    by_layer.join(", ")
                }
            );
            let mut most = tally.other_sheet_on_shown.clone();
            most.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            println!(
                "  file(s) with primitives on a sheet not drawn, on a layer the drawn sheet shows: {} ({} primitive(s))",
                most.len(),
                most.iter().map(|(count, _)| count).sum::<usize>()
            );
            for (_, line) in most.iter().take(60) {
                println!("    {line}");
            }
            println!(
                "  drawn: {} stroke(s), {} fill(s), {} label(s); the widest {:.2} mm, the tallest {:.2} mm",
                tally.strokes, tally.fills, tally.labels, tally.widest, tally.tallest
            );
            if let Some((most, name)) = &tally.slowest {
                println!(
                    "  the slowest read: {:.1} ms ({name})",
                    most.as_secs_f64() * 1000.0
                );
            }
            for line in &tally.notable {
                println!("  not plainly drawn: {line}");
            }
        }
        println!(
            "{files} symbol file(s) read and checked against the census in {:.1} s",
            took.as_secs_f64()
        );
        let _ = std::fs::remove_dir_all(&cache);

        assert!(files > 0, "the backup holds symbol files");
        if plant {
            let archive = tallies
                .get("RefData~4~681.zip")
                .expect("the plant package's archive of symbols");
            assert_eq!(
                (
                    archive.files,
                    archive.drawable,
                    archive.empty,
                    archive.short,
                    archive.failed
                ),
                (644, 643, 1, 0, 0)
            );
        }
    }
}
