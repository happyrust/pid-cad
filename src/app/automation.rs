//! Headless automation server (`OpenCADStudio --serve`).
//!
//! Drives the app without a GUI over a line-based JSON protocol: one request
//! object per line on stdin, one response object per line on stdout. State (the
//! active document) persists across requests, so an external process — a script
//! or an AI agent — can act, observe, and act again.
//!
//! Operations:
//! - `{"op":"new"}`                          — start an empty document
//! - `{"op":"open","path":"file.dwg"}`       — load a drawing
//! - `{"op":"run","cmd":"LAYER Walls"}`      — run a command (the same dispatcher
//!                                             the GUI command line uses)
//! - `{"op":"entities"}`                     — summary count by entity type
//! - `{"op":"save","path":"out.dwg"}`        — write the document (path optional
//!                                             once opened/saved)

#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::OpenCADStudio;

/// Run the headless JSON server. Default transport is stdin/stdout; with
/// `--port <N>` it instead listens on `127.0.0.1:<N>` and serves one client at
/// a time (the document session persists across reconnects).
#[cfg(not(target_arch = "wasm32"))]
pub fn serve() {
    let mut app = OpenCADStudio::new();
    match port_arg() {
        Some(port) => serve_socket(&mut app, port),
        None => serve_stdio(&mut app),
    }
}

/// Headless one-shot format conversion (`--export IN OUT`). Loads `input`,
/// writes `output` (format chosen from `output`'s extension), and returns a
/// process exit code (0 on success). No window is created.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_headless(input: &std::path::Path, output: &std::path::Path) -> i32 {
    let doc = match crate::io::load_file(input) {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("export: cannot read {}: {e}", input.display());
            return 1;
        }
    };
    match crate::io::save(&doc, output) {
        Ok(()) => {
            println!("Exported {} → {}", input.display(), output.display());
            0
        }
        Err(e) => {
            eprintln!("export: cannot write {}: {e}", output.display());
            1
        }
    }
}

/// The pieces a headless plot needs from the app: a drawing, a layout, and a
/// plot style table. Kept next to the CLI entry points that use them; the GUI
/// reaches the same state through the dialog.
#[cfg(not(target_arch = "wasm32"))]
impl OpenCADStudio {
    /// Load `path` into the active tab, as the `open` operation does.
    pub(crate) fn open_drawing_headless(&mut self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self.read_drawing(path).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let document = crate::io::load_bytes(&name, bytes).map_err(|e| e.to_string())?;
        let i = self.active_tab;
        self.tabs[i].scene.document = document;
        self.tabs[i].scene.deselect_all();
        crate::app::style_ops::ensure_standard_styles(&mut self.tabs[i].scene.document);
        self.tabs[i].adopt_active_ucs_from_header();
        self.tabs[i].current_path = Some(PathBuf::from(path));
        self.tabs[i].is_start = false;
        self.tabs[i].scene.bump_geometry();
        Ok(())
    }

    /// The drawing's paper-space layouts, in the order the tabs show them.
    pub(crate) fn plottable_layouts(&self) -> Vec<String> {
        self.tabs[self.active_tab]
            .scene
            .layout_names()
            .into_iter()
            .filter(|name| name != "Model")
            .collect()
    }

    pub(crate) fn select_layout_headless(&mut self, name: &str) -> Result<(), String> {
        let i = self.active_tab;
        let available = self.tabs[i].scene.layout_names();
        if !available.iter().any(|l| l == name) {
            return Err(format!(
                "no layout named '{name}'. This drawing has: {}",
                available.join(", ")
            ));
        }
        let scene = &mut self.tabs[i].scene;
        scene.current_layout = name.to_string();
        scene.active_viewport = None;
        scene.load_current_layout_state();
        Ok(())
    }

    /// `--ctb PATH` loads that table, `--ctb none` plots without one, and no
    /// flag at all leaves whatever the page setup names. A named table that
    /// cannot be loaded is an error, never a silent fallback (D4).
    pub(crate) fn apply_headless_plot_style(&mut self, ctb: Option<&str>) -> Result<(), String> {
        let Some(choice) = ctb else { return Ok(()) };
        if choice.eq_ignore_ascii_case("none") {
            self.active_plot_style = None;
            self.plot_dialog.style_name.clear();
            self.plot_dialog.apply_plot_styles = false;
            self.plot_dialog.style_missing = false;
            return Ok(());
        }
        let path = std::path::Path::new(choice);
        let table = if path.is_file() {
            crate::io::plot_style::PlotStyleTable::load(path)?
        } else {
            crate::io::plot_style::PlotStyleTable::load_named(choice)?
        };
        self.plot_dialog.style_name = table.name.clone();
        self.plot_dialog.apply_plot_styles = true;
        self.plot_dialog.style_missing = false;
        self.active_plot_style = Some(table);
        Ok(())
    }
}

/// What `--plot-svg` was asked to do.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub struct PlotSvgRequest {
    /// Layout to plot. `None` with `model` false plots every layout.
    pub layout: Option<String>,
    /// Plot model space instead of a layout.
    pub model: bool,
    /// `Some(path)` for a CTB file, `Some("none")` for none at all, `None` to
    /// keep whatever the page setup names.
    pub ctb: Option<String>,
    /// Sheet for a model-space plot: A0…A4. Required with `model`.
    pub paper: Option<String>,
    /// Sheet orientation for a model-space plot.
    pub landscape: bool,
    /// Fit the drawing's extents to the sheet. Mutually exclusive with
    /// `scale`; one of the two is required with `model`.
    pub fit: bool,
    /// Plot scale for a model-space plot, as `1:100`, `2:1` or `0.01`.
    pub scale: Option<String>,
    /// Resolve and render, print the plan, write nothing.
    pub dry_run: bool,
    /// Replace files that already exist.
    pub force: bool,
    /// Write a page whose text the glyph atlas could not fully supply, and
    /// say how many glyphs it lost, rather than refuse it (R1). Off by
    /// default: a drawing that looks finished and is not is the worse outcome.
    pub allow_missing_glyphs: bool,
}

/// Headless plot to SVG (`--plot-svg IN OUT`). Returns a process exit code.
///
/// No window: the pages come from the same `resolve_plot_job` the GUI export
/// uses, on an app instance built without a GUI, so a plotted layout means
/// here what it means there. Which layout is never inferred (D4) — say
/// `--layout NAME` or `--model`, or get every layout in the drawing.
#[cfg(not(target_arch = "wasm32"))]
pub fn plot_svg_headless(
    input: &std::path::Path,
    output: &std::path::Path,
    request: &PlotSvgRequest,
) -> i32 {
    let mut app = OpenCADStudio::new();
    match plot_svg_with(&mut app, input, output, request) {
        Ok(batch) => {
            let verb = if batch.dry_run {
                "would write"
            } else {
                "wrote"
            };
            for page in &batch.pages {
                println!(
                    "{verb} {} ({} elements, {} bytes)",
                    page.path.display(),
                    page.report.elements,
                    page.bytes
                );
                if page.report.needs_mix_blend_mode {
                    println!(
                        "  note: merge-lines output needs a viewer that supports \
                         CSS mix-blend-mode"
                    );
                }
                if page.report.mesh_fallbacks > 0 {
                    println!(
                        "  note: {} fill(s) were not a clean triangle mesh and \
                         kept their triangles",
                        page.report.mesh_fallbacks
                    );
                }
                if page.report.missing_glyphs > 0 {
                    // Only reachable with --allow-missing-glyphs; the default
                    // refuses the page instead.
                    eprintln!(
                        "  warning: {} glyph(s) are missing from this page — its \
                         text is incomplete",
                        page.report.missing_glyphs
                    );
                }
            }
            0
        }
        Err(error) => {
            eprintln!("plot-svg: {error}");
            1
        }
    }
}

/// `--plot-svg` on an app that is already built — the part with a result to
/// look at, so the tests can.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn plot_svg_with(
    app: &mut OpenCADStudio,
    input: &std::path::Path,
    output: &std::path::Path,
    request: &PlotSvgRequest,
) -> Result<crate::io::svg_export::SvgBatch, String> {
    use crate::app::update::file::PlotRequest;

    app.open_drawing_headless(input)?;
    app.apply_headless_plot_style(request.ctb.as_deref())?;

    let plot = if request.model {
        // No implicit sheet and no implicit scale (D4): a plot nobody is
        // watching has to say what it is putting the drawing on.
        let Some(paper) = &request.paper else {
            return Err("--model needs a sheet: --paper A3 (and --fit or --scale)".to_string());
        };
        if request.fit == request.scale.is_some() {
            return Err("--model needs exactly one of --fit and --scale <1:100>".to_string());
        }
        app.select_layout_headless("Model")?;
        app.set_headless_model_page(
            paper,
            request.landscape,
            request.fit,
            request.scale.as_deref(),
        )?;
        PlotRequest::current_view()
    } else if let Some(name) = &request.layout {
        PlotRequest::layouts(vec![name.clone()])
    } else {
        let names = app.plottable_layouts();
        if names.is_empty() {
            return Err(format!(
                "{} has no paper-space layout. Use --model to plot model \
                 space, or --layout NAME.",
                input.display()
            ));
        }
        PlotRequest::layouts(names)
    };

    let job = app.resolve_plot_job(&plot)?;
    crate::io::svg_export::export_svg_pages(
        &job.pages,
        None,
        &job.assets,
        output,
        &crate::io::svg_export::SvgWriteOptions {
            svg: crate::io::svg_export::SvgOptions {
                missing_glyphs: if request.allow_missing_glyphs {
                    crate::io::svg_export::MissingGlyphs::Report
                } else {
                    crate::io::svg_export::MissingGlyphs::Refuse
                },
                ..Default::default()
            },
            force: request.force,
            dry_run: request.dry_run,
        },
    )
    .map_err(|error| error.to_string())
}

/// `--list-layouts FILE`: the layouts a drawing offers `--plot-svg`.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_layouts_headless(input: &std::path::Path) -> i32 {
    let mut app = OpenCADStudio::new();
    if let Err(error) = app.open_drawing_headless(input) {
        eprintln!("list-layouts: {error}");
        return 1;
    }
    println!("Model");
    for name in app.plottable_layouts() {
        println!("{name}");
    }
    0
}

/// `--port <N>` if present on the command line.
#[cfg(not(target_arch = "wasm32"))]
fn port_arg() -> Option<u16> {
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == "--port" {
            return args.next().and_then(|s| s.parse().ok());
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn ready() -> Value {
    json!({ "ok": true, "ready": true, "version": env!("OCS_APP_VERSION") })
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_stdio(app: &mut OpenCADStudio) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    {
        let mut o = stdout.lock();
        let _ = writeln!(o, "{}", ready());
        let _ = o.flush();
    }
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = app.automation_op(line);
        let mut o = stdout.lock();
        let _ = writeln!(o, "{resp}");
        let _ = o.flush();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_socket(app: &mut OpenCADStudio, port: u16) {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("--serve: cannot bind 127.0.0.1:{port}: {e}");
            return;
        }
    };
    eprintln!("OpenCADStudio --serve listening on 127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let Ok(read_half) = stream.try_clone() else {
            continue;
        };
        let reader = std::io::BufReader::new(read_half);
        let mut writer = stream;
        let _ = writeln!(writer, "{}", ready());
        let _ = writer.flush();
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let resp = app.automation_op(line);
            if writeln!(writer, "{resp}").is_err() {
                break;
            }
            let _ = writer.flush();
        }
    }
}

fn err(msg: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "error": msg.to_string() })
}

fn v3(v: acadrust::types::Vector3) -> Value {
    json!([v.x, v.y, v.z])
}

/// One entity as JSON. Summary mode carries identity only, geometry adds the
/// entity's defining values, and full also includes its world bounds.
fn entity_json(e: &acadrust::EntityType, detail: &str) -> Value {
    use acadrust::EntityType as E;
    let c = e.common();
    let mut obj = json!({
        "handle": format!("{:X}", c.handle.value()),
        "type": crate::entities::names::ui_name(e),
        "layer": c.layer,
    });
    if detail == "summary" {
        return obj;
    }
    let map = obj.as_object_mut().expect("json object");
    match e {
        E::Line(l) => {
            map.insert("start".into(), v3(l.start));
            map.insert("end".into(), v3(l.end));
        }
        E::Circle(cc) => {
            map.insert("center".into(), v3(cc.center));
            map.insert("radius".into(), json!(cc.radius));
        }
        E::Arc(a) => {
            map.insert("center".into(), v3(a.center));
            map.insert("radius".into(), json!(a.radius));
            map.insert("start_angle".into(), json!(a.start_angle));
            map.insert("end_angle".into(), json!(a.end_angle));
        }
        E::Point(p) => {
            map.insert("location".into(), v3(p.location));
        }
        E::Ellipse(el) => {
            map.insert("center".into(), v3(el.center));
            map.insert("major_axis".into(), v3(el.major_axis));
        }
        E::Text(t) => {
            map.insert("value".into(), json!(t.value));
            map.insert("position".into(), v3(t.insertion_point));
            map.insert("height".into(), json!(t.height));
        }
        E::MText(t) => {
            map.insert("value".into(), json!(t.value));
            map.insert("position".into(), v3(t.insertion_point));
            map.insert("height".into(), json!(t.height));
        }
        E::LwPolyline(pl) => {
            let pts: Vec<Value> = pl
                .vertices
                .iter()
                .map(|v| json!([v.location.x, v.location.y]))
                .collect();
            map.insert("vertices".into(), json!(pts));
        }
        E::Insert(ins) => {
            map.insert("block".into(), json!(ins.block_name));
        }
        _ => {}
    }
    if detail == "full" {
        let bounds = e.as_entity().bounding_box();
        map.insert("bounds".into(), json!({
            "min":[bounds.min.x,bounds.min.y,bounds.min.z],
            "max":[bounds.max.x,bounds.max.y,bounds.max.z]
        }));
    }
    obj
}

fn request_point(req: &Value, key: &str) -> Option<[f64; 2]> {
    let values = req[key].as_array()?;
    if !(2..=3).contains(&values.len()) {
        return None;
    }
    let x = values[0].as_f64()?;
    let y = values[1].as_f64()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn request_handle(value: &Value) -> Option<acadrust::Handle> {
    value.as_str()
        .and_then(|value| u64::from_str_radix(value.trim_start_matches("0x"), 16).ok())
        .map(acadrust::Handle::new)
}

fn projected_fields(mut entity: Value, fields: Option<&Vec<Value>>) -> Value {
    let Some(fields) = fields else { return entity };
    let Some(source) = entity.as_object_mut() else { return entity };
    let keep: std::collections::HashSet<&str> =
        fields.iter().filter_map(Value::as_str).collect();
    source.retain(|key, _| key == "handle" || keep.contains(key.as_str()));
    entity
}

impl OpenCADStudio {
    /// Handle one JSON request line and return the JSON response.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn automation_op(&mut self, line: &str) -> Value {
        if let Ok(req) = serde_json::from_str::<Value>(line) {
            if req["protocol"].is_number() {
                let id = req["request_id"].clone();
                let (response, task) = self.control_request(req);
                if let Err(error) = self.drive_headless_task(task) {
                    return err(error);
                }
                if matches!(response["status"].as_str(), Some("accepted" | "running")) {
                    return self.control_request(json!({"op":"operation","request_id":id})).0;
                }
                return response;
            }
        }
        let res = self.automation_op_inner(line);
        // Most automation ops mutate `Scene::selected` directly rather than
        // going through `update()` (`select` calls `deselect_all` /
        // `select_entity`, and `run` can erase the selected entities), so the
        // selection check has to run on this path too.
        #[cfg(not(target_arch = "wasm32"))]
        self.notify_plugins_selection_changed();
        res
    }

    pub(super) fn automation_op_inner(&mut self, line: &str) -> Value {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return err(format!("invalid JSON: {e}")),
        };
        match req["op"].as_str().unwrap_or("") {
            "new" => {
                let i = self.active_tab;
                self.tabs[i].scene.document = acadrust::CadDocument::new();
                self.tabs[i].scene.deselect_all();
                self.tabs[i].current_path = None;
                // The headless session starts on the welcome (Start) tab, which
                // blocks drawing commands; turn it into a real drawing.
                self.tabs[i].is_start = false;
                self.tabs[i].scene.bump_geometry();
                self.entity_summary()
            }
            #[cfg(not(target_arch = "wasm32"))]
            "open" => {
                let Some(path) = req["path"].as_str() else {
                    return err("open: missing \"path\"");
                };
                let bytes = match self.read_drawing(std::path::Path::new(path)) {
                    Ok(b) => b,
                    Err(e) => return err(format!("open: {e}")),
                };
                let name = PathBuf::from(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                match crate::io::load_bytes(&name, bytes) {
                    Ok(doc) => {
                        let i = self.active_tab;
                        self.tabs[i].scene.document = doc;
                        self.tabs[i].scene.deselect_all();
                        crate::app::style_ops::ensure_standard_styles(
                            &mut self.tabs[i].scene.document,
                        );
                        self.tabs[i].adopt_active_ucs_from_header();
                        self.tabs[i].current_path = Some(PathBuf::from(path));
                        self.tabs[i].is_start = false;
                        self.tabs[i].scene.bump_geometry();
                        self.entity_summary()
                    }
                    Err(e) => err(format!("open: {e}")),
                }
            }
            #[cfg(target_arch = "wasm32")]
            "open" => err("open: use the browser file action"),
            "run" => {
                let cmd = req["cmd"].as_str().unwrap_or("").to_string();
                if cmd.is_empty() {
                    return err("run: missing \"cmd\"");
                }
                let i = self.active_tab;
                let before = self.tabs[i].scene.document.entities().count();
                let error_revision = self.command_line.error_revision;
                if let Err(error) = self.run_headless(&cmd) { return err(error); }
                if self.command_line.error_revision != error_revision {
                    return err(self.command_line.last_error.clone().unwrap_or_default());
                }
                let after = self.tabs[i].scene.document.entities().count();
                json!({
                    "ok": true,
                    "cmd": cmd,
                    "status": if self.tabs[i].active_cmd.is_some() { "waiting_input" } else { "completed" },
                    "entities": after,
                    "added": after as i64 - before as i64,
                })
            }
            "entities" => self.entity_summary(),
            "query" => self.entity_query(&req),
            "layers" => {
                let i = self.active_tab;
                let offset = req["offset"].as_u64().unwrap_or(0) as usize;
                let limit = req["limit"].as_u64().unwrap_or(1000).min(10_000) as usize;
                let count = self.tabs[i].scene.document.layers.iter().count();
                let layers: Vec<Value> = self
                    .tabs[i]
                    .scene
                    .document
                    .layers
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|l| {
                        let mut o = json!({
                            "name": l.name,
                            "off": l.is_off(),
                            "frozen": l.is_frozen(),
                            "locked": l.is_locked(),
                        });
                        let m = o.as_object_mut().expect("json object");
                        if let Some(aci) = l.color.index() {
                            m.insert("color".into(), json!(aci));
                        }
                        if let Some((r, g, b)) = l.color.rgb() {
                            m.insert("rgb".into(), json!([r, g, b]));
                        }
                        o
                    })
                    .collect();
                json!({
                    "ok": true,
                    "current": self.tabs[i].scene.document.header.current_layer_name,
                    "count": count,
                    "next_offset": (offset + layers.len() < count).then_some(offset + layers.len()),
                    "layers": layers,
                })
            }
            "header" => {
                let h = &self.tabs[self.active_tab].scene.document.header;
                json!({
                    "ok": true,
                    "current_layer": h.current_layer_name,
                    "current_text_style": h.current_text_style_name,
                    "insertion_units": h.insertion_units,
                    "pdmode": h.point_display_mode,
                    "pdsize": h.point_display_size,
                    "ltscale": h.linetype_scale,
                    "annotation_scale_value": h.annotation_scale_value,
                })
            }
            "undo" => {
                let _ = self.update(super::Message::Undo);
                self.entity_summary()
            }
            "redo" => {
                let _ = self.update(super::Message::Redo);
                self.entity_summary()
            }
            "select" => {
                let i = self.active_tab;
                self.tabs[i].scene.deselect_all();
                if req["clear"].as_bool() != Some(true) {
                    // By explicit handles (hex, as returned by `query`).
                    if let Some(arr) = req["handles"].as_array() {
                        for h in arr.iter().filter_map(|h| h.as_str()) {
                            if let Ok(v) = u64::from_str_radix(h.trim_start_matches("0x"), 16) {
                                self.tabs[i].scene.select_entity(acadrust::Handle::new(v), false);
                            }
                        }
                    }
                    // Or by type / layer.
                    let type_filter = req["type"].as_str();
                    let layer_filter = req["layer"].as_str();
                    if type_filter.is_some() || layer_filter.is_some() {
                        let handles: Vec<acadrust::Handle> = self.tabs[i]
                            .scene
                            .document
                            .entities()
                            .filter(|e| {
                                type_filter.is_none_or(|t| {
                                    crate::entities::names::ui_name(e).eq_ignore_ascii_case(t)
                                })
                            })
                            .filter(|e| layer_filter.is_none_or(|l| e.common().layer == l))
                            .map(|e| e.common().handle)
                            .collect();
                        for h in handles {
                            self.tabs[i].scene.select_entity(h, false);
                        }
                    }
                }
                json!({ "ok": true, "selected": self.tabs[i].scene.selected_entities().len() })
            }
            "save" => {
                let i = self.active_tab;
                let path = req["path"]
                    .as_str()
                    .map(PathBuf::from)
                    .or_else(|| self.tabs[i].current_path.clone());
                let Some(path) = path else {
                    return err("save: no \"path\" and the document has none");
                };
                #[cfg(not(target_arch = "wasm32"))]
                let result =
                    self.save_tab_synchronously_protected(i, path.clone(), true);
                #[cfg(target_arch = "wasm32")]
                let result = crate::io::save(&self.tabs[i].scene.document, &path)
                    .map_err(crate::io::SaveFailure::other);
                match result {
                    Ok(()) => {
                        json!({ "ok": true, "saved": path.to_string_lossy() })
                    }
                    Err(e) => err(format!("save: {e}")),
                }
            }
            "" => err("missing \"op\""),
            other => err(format!("unknown op: {other}")),
        }
    }

    /// Run a command line headlessly. Thin wrapper over the shared
    /// [`OpenCADStudio::run_command_line`] (see `cmd_result.rs`), which the GUI
    /// command line uses too so both process `UCS Z 90` / `LINE 0,0 10,10` /
    /// `PDMODE 3` identically.
    fn run_headless(&mut self, cmd: &str) -> Result<(), String> {
        let task = self.run_command_line(cmd);
        self.drive_headless_task(task)
    }

    pub(super) fn drive_headless_task(&mut self, task: iced::Task<super::Message>) -> Result<(), String> {
        use iced::futures::StreamExt;
        let mut streams = Vec::new();
        if let Some(stream) = iced_runtime::task::into_stream(task) { streams.push(stream); }
        while let Some(stream) = streams.last_mut() {
            match iced::futures::executor::block_on(stream.next()) {
                Some(iced_runtime::Action::Output(message)) => {
                    let next = self.update(message);
                    if let Some(stream) = iced_runtime::task::into_stream(next) { streams.push(stream); }
                }
                Some(iced_runtime::Action::Widget(_)) | Some(iced_runtime::Action::Tick) | Some(iced_runtime::Action::Reload) => {},
                Some(action) => return Err(format!("GUI runtime required for {action:?}")),
                None => { streams.pop(); }
            }
        }
        self.finish_all_pending_history();
        Ok(())
    }

    /// Query entities by identity, metadata and exact plane-curve relationships.
    fn entity_query(&self, req: &Value) -> Value {
        let i = self.active_tab;
        let tab = &self.tabs[i];
        if let Some(pair) = req["intersections"].as_array() {
            if pair.len() != 2 {
                return err("query intersections expects exactly two handles");
            }
            let Some(first) = request_handle(&pair[0]) else {
                return err("query intersections contains an invalid first handle");
            };
            let Some(second) = request_handle(&pair[1]) else {
                return err("query intersections contains an invalid second handle");
            };
            let Some(first_entity) = tab.scene.document.get_entity(first) else {
                return err("query intersections first entity does not exist");
            };
            let Some(second_entity) = tab.scene.document.get_entity(second) else {
                return err("query intersections second entity does not exist");
            };
            let Some(first_curve) = crate::entities::curve::entity_curve_xy(first_entity) else {
                return err("query intersections first entity is not a planar curve");
            };
            let Some(second_curve) = crate::entities::curve::entity_curve_xy(second_entity) else {
                return err("query intersections second entity is not a planar curve");
            };
            let crossings = cadkernel::geom2d::intersect(
                &first_curve,
                &second_curve,
                cadkernel::geom2d::Tolerance::default(),
            );
            return json!({
                "ok":true,
                "document_id":tab.id,
                "geometry_revision":tab.scene.geometry_epoch,
                "handles":[format!("{:X}",first.value()),format!("{:X}",second.value())],
                "count":crossings.len(),
                "intersections":crossings.into_iter().map(|crossing|json!({
                    "point":[crossing.point[0],crossing.point[1]],
                    "parameter_first":crossing.t_a,
                    "parameter_second":crossing.t_b
                })).collect::<Vec<_>>()
            });
        }

        let type_filter = req["type"].as_str();
        let layer_filter = req["layer"].as_str();
        let handles: Option<std::collections::HashSet<u64>> = req["handles"]
            .as_array()
            .map(|values| values.iter().filter_map(request_handle).map(|h| h.value()).collect())
            .or_else(|| request_handle(&req["handle"])
                .map(|handle| std::iter::once(handle.value()).collect()));
        let near = request_point(req, "near");
        let contains = request_point(req, "contains_point");
        let bounds = req["bounds"].as_array().and_then(|values| {
            (values.len() == 4).then(|| Some([
                values[0].as_f64()?, values[1].as_f64()?,
                values[2].as_f64()?, values[3].as_f64()?,
            ])).flatten()
        });
        if req.get("handles").is_some() && handles.as_ref().is_some_and(|parsed| {
            parsed.len() != req["handles"].as_array().map_or(0, Vec::len)
        }) {
            return err("query handles contains an invalid hexadecimal handle");
        }
        if req.get("handle").is_some() && request_handle(&req["handle"]).is_none() {
            return err("query handle must be hexadecimal");
        }
        if req.get("near").is_some() && near.is_none() {
            return err("query near expects two or three finite coordinates");
        }
        if req.get("contains_point").is_some() && contains.is_none() {
            return err("query contains_point expects two or three finite coordinates");
        }
        if req.get("bounds").is_some() && bounds.is_none_or(|bounds| {
            !bounds.iter().all(|value| value.is_finite())
                || bounds[0] > bounds[2] || bounds[1] > bounds[3]
        }) {
            return err("query bounds expects finite [min_x,min_y,max_x,max_y]");
        }
        let detail = req["detail"].as_str().unwrap_or("geometry");
        if !matches!(detail, "summary" | "geometry" | "full") {
            return err("query detail must be summary, geometry or full");
        }
        let limit = req["limit"].as_u64().unwrap_or(1000).min(10000) as usize;
        let offset = req["offset"].as_u64().unwrap_or(0) as usize;

        let mut matched = Vec::new();
        for e in tab.scene.document.entities() {
            if handles.as_ref().is_some_and(|handles| {
                !handles.contains(&e.common().handle.value())
            }) {
                continue;
            }
            if type_filter.is_some_and(|value| {
                !crate::entities::names::ui_name(e).eq_ignore_ascii_case(value)
            }) || layer_filter.is_some_and(|value| e.common().layer != value) {
                continue;
            }
            if let Some(bounds) = bounds {
                let entity_bounds = e.as_entity().bounding_box();
                if entity_bounds.max.x < bounds[0] || entity_bounds.max.y < bounds[1]
                    || entity_bounds.min.x > bounds[2] || entity_bounds.min.y > bounds[3]
                {
                    continue;
                }
            }
            let curve = (near.is_some() || contains.is_some())
                .then(|| crate::entities::curve::entity_curve_xy(e)).flatten();
            if let Some(point) = contains {
                let Some(curve) = curve.as_ref().filter(|curve| curve.is_closed()) else {
                    continue;
                };
                if !cadkernel::geom2d::contains(
                    std::slice::from_ref(curve), point,
                    cadkernel::geom2d::Tolerance::default(),
                ) {
                    continue;
                }
            }
            let nearest = near.and_then(|point| curve.as_ref()
                .map(|curve| cadkernel::geom2d::closest_point(curve, point)));
            if near.is_some() && nearest.is_none() {
                continue;
            }
            let mut entity = entity_json(e, detail);
            if let Some(nearest) = nearest {
                let object = entity.as_object_mut().expect("entity JSON object");
                object.insert("distance".into(), json!(nearest.distance));
                object.insert("closest_point".into(),
                    json!([nearest.point[0], nearest.point[1]]));
                object.insert("parameter".into(), json!(nearest.t));
            }
            matched.push((nearest.map(|nearest| nearest.distance), entity));
        }
        if near.is_some() {
            matched.sort_by(|left, right| left.0.partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal));
        }
        let count = matched.len();
        let fields = req["fields"].as_array();
        let entities: Vec<Value> = matched.into_iter().skip(offset).take(limit)
            .map(|(_, entity)| projected_fields(entity, fields)).collect();
        json!({
            "ok": true,
            "document_id":tab.id,
            "geometry_revision":tab.scene.geometry_epoch,
            "count": count,
            "returned": entities.len(),
            "next_offset": (offset + entities.len() < count).then_some(offset + entities.len()),
            "entities": entities,
        })
    }

    /// Count of entities in the active document, total and by type.
    fn entity_summary(&self) -> Value {
        let i = self.active_tab;
        let mut by_type: std::collections::BTreeMap<String, u64> = Default::default();
        let mut total = 0u64;
        for e in self.tabs[i].scene.document.entities() {
            *by_type
                .entry(crate::entities::names::ui_name(e).to_string())
                .or_default() += 1;
            total += 1;
        }
        json!({ "ok": true, "total": total, "by_type": by_type })
    }
}

#[cfg(test)]
mod tests {
    use crate::app::OpenCADStudio;

    /// A drawing on disk with a little geometry in model space, built through
    /// the same dispatcher the editor uses.
    #[cfg(not(target_arch = "wasm32"))]
    fn drawing_with_geometry(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        drawing_with(name, true)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn drawing_with(name: &str, label: bool) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("ocs-plot-svg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("drawing.dxf");

        let mut app = OpenCADStudio::new_for_test();
        assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
        assert_eq!(
            app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 100,0 100,60 0,60 0,0"}"#)["ok"],
            true
        );
        assert_eq!(
            app.automation_op(r#"{"op":"run","cmd":"CIRCLE 50,30 20"}"#)["ok"],
            true
        );
        // Text, because glyph geometry is the one thing a headless plot could
        // silently lose: `emit_text` reads the process-wide SDF atlas, which
        // in the editor is warm from drawing the canvas (D6/R1).
        if label {
            let text = acadrust::entities::Text::with_value(
                "PLOT".to_string(),
                acadrust::types::Vector3::new(20.0, 20.0, 0.0),
            )
            .with_height(8.0);
            app.tabs[app.active_tab]
                .scene
                .add_entity(acadrust::EntityType::Text(text));
        }
        let save = format!(
            r#"{{"op":"save","path":{}}}"#,
            serde_json::to_string(&path.to_string_lossy()).unwrap()
        );
        assert_eq!(app.automation_op(&save)["ok"], true, "saved the fixture");
        (dir, path)
    }

    // The headless entry point end to end: open a drawing with no window in
    // sight, resolve the page through the same `resolve_plot_job` the GUI
    // export uses, and write an SVG that parses.
    /// A model-space plot with everything D4 requires stated.
    #[cfg(not(target_arch = "wasm32"))]
    fn model_on_a3() -> super::PlotSvgRequest {
        super::PlotSvgRequest {
            model: true,
            paper: Some("A3".into()),
            landscape: true,
            fit: true,
            ..Default::default()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn plot_svg_writes_model_space_without_a_gui() {
        let (dir, drawing) = drawing_with_geometry("model");
        let out = dir.join("plot.svg");
        let mut app = OpenCADStudio::new_for_test();
        let batch = super::plot_svg_with(&mut app, &drawing, &out, &model_on_a3())
            .expect("model space plots");

        assert_eq!(batch.pages.len(), 1);
        assert_eq!(batch.pages[0].path, out);
        assert!(batch.pages[0].report.elements > 1, "nothing was drawn");
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("<?xml"));
        assert!(
            text.contains("width=\"420mm\" height=\"297mm\""),
            "the sheet the command line asked for is not the sheet it got"
        );
        // The label's glyphs came out as geometry from a cold atlas — no
        // window ever drew this drawing (D6) — and as geometry, not a <text>
        // element pointing at a font this file does not carry.
        assert!(!text.contains("<text"), "the label is not vector geometry");
        let (bare_dir, bare_drawing) = drawing_with("unlabelled", false);
        let bare_out = bare_dir.join("plot.svg");
        let mut app = OpenCADStudio::new_for_test();
        let bare = super::plot_svg_with(&mut app, &bare_drawing, &bare_out, &model_on_a3())
            .expect("the same drawing without its label plots");
        assert!(
            batch.pages[0].report.elements > bare.pages[0].report.elements,
            "the label added nothing: {} elements with it, {} without",
            batch.pages[0].report.elements,
            bare.pages[0].report.elements
        );
        let _ = std::fs::remove_dir_all(&bare_dir);
        resvg::usvg::Tree::from_str(&text, &resvg::usvg::Options::default())
            .expect("the written file is a valid SVG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // R1: the job carries the glyph snapshot its pages are drawn from, taken
    // by `resolve_plot_job` itself and already checked against the pages — so
    // a backend on another thread, or a lenient one, cannot be handed text the
    // snapshot does not cover without the check having said so first.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_job_carries_one_glyph_snapshot_that_covers_its_pages() {
        use crate::app::update::file::PlotRequest;

        let (dir, drawing) = drawing_with_geometry("job-snapshot");
        let mut app = OpenCADStudio::new_for_test();
        app.open_drawing_headless(&drawing).unwrap();
        app.select_layout_headless("Model").unwrap();
        app.set_headless_model_page("A3", true, true, None).unwrap();
        let job = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        assert_eq!(job.pages.len(), 1);
        let snapshot = job
            .assets
            .glyphs
            .as_ref()
            .expect("a page with a label gets a snapshot");
        assert!(
            snapshot.glyphs() >= 4,
            "PLOT is four glyphs: {}",
            snapshot.glyphs()
        );
        assert_eq!(
            job.assets.stale_glyphs(&job.pages),
            0,
            "the snapshot the job carries does not cover the job's own text"
        );
        // And the strict default plots it without a word about missing text.
        let out = dir.join("plot.svg");
        let batch = super::plot_svg_with(&mut app, &drawing, &out, &model_on_a3()).unwrap();
        assert_eq!(batch.pages[0].report.missing_glyphs, 0);
        let _ = std::fs::remove_dir_all(&dir);

        // A drawing with no text takes no snapshot: nothing to draw from it,
        // and the export table is not free.
        let (bare_dir, bare_drawing) = drawing_with("job-no-text", false);
        let mut app = OpenCADStudio::new_for_test();
        app.open_drawing_headless(&bare_drawing).unwrap();
        app.select_layout_headless("Model").unwrap();
        app.set_headless_model_page("A3", true, true, None).unwrap();
        let job = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        assert!(job.assets.glyphs.is_none(), "no text, no snapshot");
        let _ = std::fs::remove_dir_all(&bare_dir);
    }

    // R1, the recovery: the scene keeps its laid-out wires until the geometry
    // changes, and the atlas can be rewound or re-scaled behind them — every
    // key those quads carry then points at a tile that is gone. A job resolved
    // from such wires would be short its text, and `resolve_plot_job` is the
    // one place that can notice: it checks the snapshot against the pages and
    // lays the drawing out again when they disagree.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_job_whose_text_the_atlas_no_longer_holds_is_laid_out_again() {
        use crate::app::update::file::PlotRequest;
        use crate::io::plot_emit::{GlyphSnapshot, PlotAssets};
        use crate::io::svg_export::{export_svg_pages, SvgWriteOptions};

        let (dir, drawing) = drawing_with_geometry("job-rewound");
        let mut app = OpenCADStudio::new_for_test();
        app.open_drawing_headless(&drawing).unwrap();
        app.select_layout_headless("Model").unwrap();
        app.set_headless_model_page("A3", true, true, None).unwrap();
        let first = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        assert_eq!(first.assets.stale_glyphs(&first.pages), 0);

        // Rewind the atlas behind the job's back, as TEXTFILL does.
        crate::scene::text::sdf_atlas::text_atlas()
            .lock()
            .unwrap()
            .reset();
        let live = PlotAssets {
            glyphs: GlyphSnapshot::capture(),
            ..Default::default()
        };
        assert!(
            live.stale_glyphs(&first.pages) > 0,
            "the reset should have orphaned every glyph the first job laid out"
        );
        // The first job is unaffected: it draws from the snapshot it carries,
        // and the strict default writes its page whole.
        assert_eq!(first.assets.stale_glyphs(&first.pages), 0);
        let strict = SvgWriteOptions {
            force: true,
            ..Default::default()
        };
        let before = export_svg_pages(
            &first.pages,
            None,
            &first.assets,
            &dir.join("before.svg"),
            &strict,
        )
        .expect("a job carries its own glyphs, whatever happened to the atlas since");
        assert_eq!(before.pages[0].report.missing_glyphs, 0);

        // A job resolved now starts from the same cached wires, whose keys the
        // atlas no longer has; it must come back laid out afresh and covered.
        let second = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        assert_eq!(
            second.assets.stale_glyphs(&second.pages),
            0,
            "the job was handed pages its snapshot cannot letter"
        );
        let after = export_svg_pages(
            &second.pages,
            None,
            &second.assets,
            &dir.join("after.svg"),
            &strict,
        )
        .expect("the re-laid-out page is whole under the strict default");
        assert_eq!(after.pages[0].report.missing_glyphs, 0);
        assert_eq!(
            after.pages[0].report.elements, before.pages[0].report.elements,
            "the same drawing laid out twice should draw the same number of elements"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // D4: headless never picks a sheet or a scale on the user's behalf.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_model_space_plot_must_state_its_sheet_and_scale() {
        let (dir, drawing) = drawing_with_geometry("explicit");
        let out = dir.join("plot.svg");
        let refuse = |request: super::PlotSvgRequest| {
            let mut app = OpenCADStudio::new_for_test();
            super::plot_svg_with(&mut app, &drawing, &out, &request)
                .expect_err("the request is under-specified")
        };
        assert!(refuse(super::PlotSvgRequest {
            model: true,
            fit: true,
            ..Default::default()
        })
        .contains("--paper"));
        assert!(refuse(super::PlotSvgRequest {
            model: true,
            paper: Some("A3".into()),
            ..Default::default()
        })
        .contains("--fit"));
        assert!(refuse(super::PlotSvgRequest {
            model: true,
            paper: Some("A3".into()),
            fit: true,
            scale: Some("1:100".into()),
            ..Default::default()
        })
        .contains("--fit"));
        assert!(refuse(super::PlotSvgRequest {
            model: true,
            paper: Some("A9".into()),
            fit: true,
            ..Default::default()
        })
        .contains("A9"));
        assert!(refuse(super::PlotSvgRequest {
            model: true,
            paper: Some("A3".into()),
            scale: Some("one to a hundred".into()),
            ..Default::default()
        })
        .contains("scale"));
        assert!(!out.exists(), "a refused plot wrote a file");

        // An explicit ratio is honoured: 1:2 puts a 100 mm line on 50 mm of
        // paper, which a fitted plot would not.
        let mut app = OpenCADStudio::new_for_test();
        super::plot_svg_with(
            &mut app,
            &drawing,
            &out,
            &super::PlotSvgRequest {
                model: true,
                paper: Some("A3".into()),
                landscape: true,
                scale: Some("1:2".into()),
                ..Default::default()
            },
        )
        .expect("an explicit scale plots");
        let scaled = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        let mut app = OpenCADStudio::new_for_test();
        super::plot_svg_with(&mut app, &drawing, &out, &model_on_a3()).expect("a fitted plot");
        let fitted = std::fs::read_to_string(&out).unwrap();
        assert_ne!(scaled, fitted, "--scale drew the same as --fit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A rehearsal renders the page — so it still fails on a page it could not
    // write — but leaves the directory alone.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_dry_run_plot_writes_nothing() {
        let (dir, drawing) = drawing_with_geometry("dry");
        let out = dir.join("plot.svg");
        let mut app = OpenCADStudio::new_for_test();
        let batch = super::plot_svg_with(
            &mut app,
            &drawing,
            &out,
            &super::PlotSvgRequest {
                dry_run: true,
                ..model_on_a3()
            },
        )
        .expect("the rehearsal succeeds");
        assert!(batch.dry_run);
        assert_eq!(batch.pages.len(), 1);
        assert!(batch.pages[0].bytes > 0, "the page was not rendered");
        assert!(!out.exists(), "a dry run wrote a file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Headless never guesses which layout to plot (D4), and a plot style table
    // that cannot be loaded is an error rather than a quiet fallback.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_headless_plot_refuses_what_it_cannot_resolve() {
        let (dir, drawing) = drawing_with_geometry("refuse");
        let out = dir.join("plot.svg");

        let mut app = OpenCADStudio::new_for_test();
        let error = super::plot_svg_with(
            &mut app,
            &drawing,
            &out,
            &super::PlotSvgRequest {
                layout: Some("Nonexistent".into()),
                ..Default::default()
            },
        )
        .expect_err("there is no such layout");
        assert!(error.contains("Nonexistent"), "{error}");

        let mut app = OpenCADStudio::new_for_test();
        let error = super::plot_svg_with(
            &mut app,
            &drawing,
            &out,
            &super::PlotSvgRequest {
                ctb: Some("no-such-table.ctb".into()),
                ..model_on_a3()
            },
        )
        .expect_err("the plot style table is missing");
        assert!(error.to_lowercase().contains("no-such-table"), "{error}");
        assert!(!out.exists(), "a refused plot wrote a file");

        // And `none` is a valid answer: plot with no table at all.
        let mut app = OpenCADStudio::new_for_test();
        super::plot_svg_with(
            &mut app,
            &drawing,
            &out,
            &super::PlotSvgRequest {
                ctb: Some("none".into()),
                ..model_on_a3()
            },
        )
        .expect("no plot style table is fine");
        assert!(out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The editor's side of the same feature: the command exists, dispatches,
    // and asks for a path. With no window there is nothing to pick, so it
    // stops there — but an unrecognised command would fail here.
    #[test]
    fn the_svg_export_command_is_wired_up() {
        let mut app = OpenCADStudio::new_for_test();
        assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
        for command in ["EXPORTSVG", "SVGOUT"] {
            let request = format!(r#"{{"op":"run","cmd":"{command}"}}"#);
            let response = app.automation_op(&request);
            assert_eq!(response["ok"], true, "{command}: {response}");
        }
    }

    #[test]
    fn layout_notice_skips_grid_camera_and_scene_builds() {
        let mut app = OpenCADStudio::new_for_test();
        assert_eq!(app.automation_op(r#"{"op":"new"}"#)["ok"], true);
        assert_eq!(app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 10,10"}"#)["ok"], true);
        let i = app.active_tab;
        let scene = &mut app.tabs[i].scene;
        scene.document.add_layout("Review").unwrap();
        scene.set_current_layout("Review".to_string());
        let mut viewport = acadrust::entities::Viewport::new();
        viewport.id = 2;
        viewport.width = 100.0;
        viewport.height = 50.0;
        viewport.status.is_on = true;
        scene.add_entity(acadrust::EntityType::Viewport(viewport));
        for entity in scene.document.entities_mut() {
            if let acadrust::EntityType::Viewport(viewport) = entity {
                viewport.status.grid_on = true;
            }
        }
        let before = scene.last_tess_wires.get();
        app.layout_settling = true;
        drop(app.view_main());
        assert_eq!(app.tabs[i].scene.last_tess_wires.get(), before);
        let _ = app.update(crate::app::Message::LayoutSettled);
        assert!(!app.layout_settling);
        drop(app.view_main());
        assert!(app.tabs[i].scene.last_tess_wires.get() > before);
    }

    #[test]
    fn automation_ops_round_trip() {
        let mut app = OpenCADStudio::new_for_test();

        let r = app.automation_op(r#"{"op":"new"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["total"], 0);

        // A synchronous command runs through the real dispatcher.
        let r = app.automation_op(r#"{"op":"run","cmd":"PDMODE 3"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["cmd"], "PDMODE 3");

        // A draw command with coordinates creates real geometry.
        let r = app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 10,10 10,20"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["added"], 2); // two segments → two Line entities
        let r = app.automation_op(r#"{"op":"run","cmd":"CIRCLE 5,5 3"}"#);
        assert_eq!(r["added"], 1);

        let r = app.automation_op(r#"{"op":"entities"}"#);
        assert_eq!(r["ok"], true);
        assert_eq!(r["total"], 3);
        assert_eq!(r["by_type"]["Line"], 2);
        assert_eq!(r["by_type"]["Circle"], 1);

        // query returns per-entity detail and honours a type filter.
        let r = app.automation_op(r#"{"op":"query","type":"Circle"}"#);
        assert_eq!(r["count"], 1);
        assert_eq!(r["entities"][0]["type"], "Circle");
        assert_eq!(r["entities"][0]["radius"], 3.0);

        // select by type, then a selection command acts on it.
        let r = app.automation_op(r#"{"op":"select","type":"Line"}"#);
        assert_eq!(r["selected"], 2);
        app.automation_op(r#"{"op":"run","cmd":"ERASE"}"#);
        let r = app.automation_op(r#"{"op":"entities"}"#);
        assert_eq!(r["total"], 1); // only the Circle remains

        // undo restores the erased lines.
        let r = app.automation_op(r#"{"op":"undo"}"#);
        assert_eq!(r["total"], 3);

        // move a selected entity by a displacement.
        app.automation_op(r#"{"op":"select","type":"Circle"}"#);
        app.automation_op(r#"{"op":"run","cmd":"MOVE 0,0 100,0"}"#);
        let r = app.automation_op(r#"{"op":"query","type":"Circle"}"#);
        assert_eq!(r["entities"][0]["center"][0], 105.0); // 5 + 100

        // Errors are reported, never panics.
        assert_eq!(app.automation_op(r#"{"op":"bogus"}"#)["ok"], false);
        assert_eq!(app.automation_op("not json")["ok"], false);
        assert_eq!(app.automation_op(r#"{"op":"run"}"#)["ok"], false);
    }

    #[test]
    fn ucs_interactive_inline_args() {
        // `UCS Z 90` must drive the interactive UCS command step-by-step (option
        // "Z" then value "90") and rotate the active UCS 90° about Z. (#169)
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"UCS Z 90"}"#);
        let i = app.active_tab;
        let ucs = app.tabs[i]
            .active_ucs
            .as_ref()
            .expect("UCS Z 90 should set an active UCS");
        // 90° about Z sends the X axis (1,0,0) → (0,1,0).
        assert!(
            ucs.x_axis.x.abs() < 1e-6 && (ucs.x_axis.y - 1.0).abs() < 1e-6,
            "x_axis after UCS Z 90 = ({}, {})",
            ucs.x_axis.x,
            ucs.x_axis.y
        );
    }

    #[test]
    fn translated_and_rotated_ucs_resolves_absolute_relative_and_polar_input() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"UCS ORIGIN 100,200,300"}"#);
        app.automation_op(r#"{"op":"run","cmd":"UCS Z 90"}"#);
        app.automation_op(r#"{"op":"run","cmd":"LINE 2,3 @5<0"}"#);

        let line = app.tabs[app.active_tab]
            .scene
            .document
            .entities()
            .find_map(|entity| match entity {
                acadrust::EntityType::Line(line) => Some(line),
                _ => None,
            })
            .expect("LINE should create one segment");
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(line.start.x, 97.0));
        assert!(close(line.start.y, 202.0));
        assert!(close(line.start.z, 300.0));
        assert!(close(line.end.x, 97.0));
        assert!(close(line.end.y, 207.0));
        assert!(close(line.end.z, 300.0));
    }

    #[test]
    fn tilted_ucs_places_planar_entities_with_the_plane_normal() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(
            r#"{"op":"run","cmd":"UCS 3POINT 0,0,0 1,0,0 0,0,1"}"#,
        );
        app.automation_op(r#"{"op":"run","cmd":"CIRCLE 2,3 1"}"#);

        let circle = app.tabs[app.active_tab]
            .scene
            .document
            .entities()
            .find_map(|entity| match entity {
                acadrust::EntityType::Circle(circle) => Some(circle),
                _ => None,
            })
            .expect("CIRCLE should create one entity");
        let center = circle.center_wcs();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(center.x, 2.0));
        assert!(close(center.y, 0.0));
        assert!(close(center.z, 3.0));
        assert!(close(circle.normal.x, 0.0));
        assert!(close(circle.normal.y, -1.0));
        assert!(close(circle.normal.z, 0.0));
    }

    #[test]
    fn value_prompt_commands_inline_args() {
        // A single-value setting command entered with its value on one line
        // drives the interactive front-end (start + value step) and applies via
        // the inline handler. (F4)
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"PDMODE 3"}"#);
        app.automation_op(r#"{"op":"run","cmd":"LTSCALE 2.5"}"#);
        let i = app.active_tab;
        let h = &app.tabs[i].scene.document.header;
        assert_eq!(h.point_display_mode, 3, "PDMODE 3 should set point mode");
        assert!(
            (h.linetype_scale - 2.5).abs() < 1e-9,
            "LTSCALE 2.5 should set scale, got {}",
            h.linetype_scale
        );
        // No command should be left dangling.
        assert!(app.tabs[i].active_cmd.is_none(), "command must have finished");
    }

    #[test]
    fn rotate_by_typed_angle_after_center() {
        // ROTATE: after picking the centre, typing the angle directly must
        // rotate the selection (the reference point is optional, as the prompt
        // says). Before the fix this did nothing and the command cancelled, so
        // the objects never rotated. Regression for #159.
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 10,0"}"#);
        app.automation_op(r#"{"op":"select","type":"Line"}"#);
        // Centre (0,0) then 90° — no reference point.
        app.automation_op(r#"{"op":"run","cmd":"ROTATE 0,0 90"}"#);
        let q = app.automation_op(r#"{"op":"query","type":"Line"}"#);
        assert_eq!(q["count"], 1, "the line must survive the rotate");
        let ex = q["entities"][0]["end"][0].as_f64().unwrap();
        let ey = q["entities"][0]["end"][1].as_f64().unwrap();
        // (10,0) rotated 90° about the origin → (0,10).
        assert!(
            ex.abs() < 1e-3 && (ey - 10.0).abs() < 1e-3,
            "line end after ROTATE 90 = ({ex}, {ey})"
        );
    }

    #[test]
    fn start_page_runs_tools_that_need_no_drawing_but_still_refuses_the_rest() {
        // App-wide commands remain available on the welcome page; drawing
        // commands and scene tools do not. (#388, #389)
        use crate::app::Message;
        use crate::modules::ModuleEvent;
        use crate::ui::command_line::EntryKind;

        // Fresh app = welcome tab, no drawing.
        let mut app = OpenCADStudio::new_for_test();
        assert!(
            app.tabs[app.active_tab].is_start,
            "test needs the welcome tab"
        );

        // ABOUT schedules its modal; it must pass the welcome-page gate.
        let start = app.command_line.history.len();
        let _ = app.update(Message::RibbonToolClick {
            tool_id: "ABOUT".to_string(),
            event: ModuleEvent::Command("ABOUT".to_string()),
        });
        assert_eq!(
            app.command_line.history.len(),
            start,
            "ABOUT must not be refused on the welcome page"
        );

        // …but a tool that does need a drawing is still turned away (#299).
        let start = app.command_line.history.len();
        let _ = app.update(Message::RibbonToolClick {
            tool_id: "LINE".to_string(),
            event: ModuleEvent::Command("LINE".to_string()),
        });
        let refusal = &app.command_line.history[start..];
        assert_eq!(refusal.len(), 1, "LINE must emit one refusal");
        assert_eq!(refusal[0].kind, EntryKind::Info);
        assert!(
            app.tabs[app.active_tab].active_cmd.is_none(),
            "LINE must not have started"
        );

        // A non-command tool event touches the scene, so it stays inert too.
        let start = app.command_line.history.len();
        let _ = app.update(Message::RibbonToolClick {
            tool_id: "LAYERS".to_string(),
            event: ModuleEvent::ToggleLayers,
        });
        let refusal = &app.command_line.history[start..];
        assert_eq!(refusal.len(), 1, "scene tools must emit one refusal");
        assert_eq!(refusal[0].kind, EntryKind::Info);

        // Check link commands in source without launching them.
        let dispatch_src = include_str!("commands/mod.rs");
        // Extract the `start_allowed` match body.
        let gate = dispatch_src
            .split("pub fn start_allowed")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("the start_allowed gate moved — re-point this test");
        // Welcome-page links plus app-wide configuration commands.
        let standalone = [
            "DONATE",
            "REPORT",
            "WEBVERSION",
            "ABOUT",
            "CHANGELOG",
            "CUI",
            "ALIASEDIT",
        ];
        for cmd in standalone {
            assert!(
                gate.contains(&format!("\"{cmd}\"")),
                "{cmd} needs no drawing but is missing from dispatch's standalone \
                 list, so it is refused on the welcome page"
            );
        }
        // Every allowed command needs a dispatch arm.
        let view_src = include_str!("commands/view.rs");
        for cmd in standalone {
            assert!(
                view_src.contains(&format!("\"{cmd}\" =>"))
                    || view_src.contains(&format!("\"{cmd}\" |")),
                "{cmd} has no dispatch arm"
            );
        }
    }

    /// PICKADD / PICKDRAG (#226): the command flips the live flag both via the
    /// inline form and the two-step ValuePrompt flow.
    #[test]
    fn lasso_press_drag_selects() {
        // Press-drag lasso must select crossed entities — regression probe
        // for the #226 PICKDRAG work (both PICKDRAG modes complete through
        // the poly path).
        use crate::app::Message;
        for (add, rect) in [(true, false), (true, true), (false, false), (false, true)] {
            let mut app = OpenCADStudio::new_for_test();
            app.automation_op(r#"{"op":"new"}"#);
            let i = app.active_tab;
            app.pick_add = add;
            app.pick_drag_rect = rect;
            let _ = app.run_command_line("LINE 0,0 10,10");
            app.tabs[i].scene.selection.borrow_mut().vp_size = (800.0, 600.0);
            let _ = app.run_command_line("ZOOM EXTENTS");
            // Both directions. Crossing = a right → left diagonal sweep (the
            // freeform path may be degenerate — crossing counts hits).
            // Window = a left → right perimeter walk so the freeform ring
            // actually ENCLOSES the line (a diagonal has no area).
            let path: Vec<(f32, f32)> = if !add && rect {
                // Rectangle window: a simple left → right diagonal spans it.
                (0..=10)
                    .map(|k| {
                        let t = k as f32 / 10.0;
                        (20.0 + t * 760.0, 20.0 + t * 560.0)
                    })
                    .collect()
            } else if add {
                let (sx, sy, ex, ey) = (780.0f32, 580.0f32, 20.0f32, 20.0f32);
                (0..=10)
                    .map(|k| {
                        let t = k as f32 / 10.0;
                        (sx + t * (ex - sx), sy + t * (ey - sy))
                    })
                    .collect()
            } else {
                vec![
                    (20.0, 20.0),
                    (400.0, 20.0),
                    (780.0, 20.0),
                    (780.0, 300.0),
                    (780.0, 580.0),
                    (400.0, 580.0),
                    (20.0, 580.0),
                    (20.0, 300.0),
                ]
            };
            let _ = app.update(Message::ViewportMove(iced::Point::new(path[0].0, path[0].1)));
            let _ = app.update(Message::ViewportLeftPress);
            std::thread::sleep(std::time::Duration::from_millis(180));
            for &(x, y) in &path {
                let _ = app.update(Message::ViewportMove(iced::Point::new(x, y)));
            }
            {
                let sel = app.tabs[i].scene.selection.borrow();
                assert!(sel.left_dragging, "drag must start (add={add} rect={rect})");
                if rect {
                    // Rectangle mode drives the box machinery, not the lasso.
                    assert!(
                        sel.box_anchor.is_some() && sel.box_current.is_some() && !sel.poly_active,
                        "rect marquee must arm the box (add={add})"
                    );
                } else {
                    assert!(sel.poly_active, "lasso must start (add={add})");
                    assert!(sel.poly_points.len() >= 3, "lasso points (add={add})");
                }
            }
            let _ = app.update(Message::ViewportLeftRelease);
            assert!(
                !app.tabs[i].scene.selected.is_empty(),
                "marquee must select the line (add={add} rect={rect})"
            );
        }
    }

    #[test]
    fn pickadd_command_flips_flag() {
        let mut app = OpenCADStudio::new_for_test();
        // The Start tab blocks drawing commands — open a drawing first.
        app.automation_op(r#"{"op":"new"}"#);
        // The boot path may have restored a persisted value — normalize.
        app.pick_add = true;
        app.pick_drag_rect = false;
        let _ = app.run_command_line("PICKADD 0");
        assert!(!app.pick_add, "PICKADD 0 must switch to replace mode");
        let _ = app.run_command_line("PICKADD 1");
        assert!(app.pick_add);
        // Two-step: bare command then the value, like typing 1 + Enter
        // (feed_active_cmd is the same path the GUI submit offers first).
        let _ = app.run_command_line("PICKDRAG");
        assert!(app.tabs[app.active_tab].active_cmd.is_some(), "prompt must open");
        let _ = app.feed_active_cmd("1");
        assert!(app.pick_drag_rect, "PICKDRAG 1 via the prompt must switch");
    }

    #[test]
    fn matchprop_matches_text_style_and_height() {
        // MATCHPROP between text objects must carry the text-specific
        // properties (style, height) to TEXT and MTEXT destinations, not just
        // the generic layer/color/linetype set. Regression for #361.
        use crate::command::StepInput;
        use acadrust::{EntityType, MText, Text};

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;

        let mut src = Text::new();
        src.value = "SRC".into();
        src.height = 5.0;
        src.style = "BIG".into();
        let src_h = app.tabs[i].scene.add_entity(EntityType::Text(src));

        let mut dst_text = Text::new();
        dst_text.value = "DST".into();
        dst_text.height = 1.0;
        let dst_text_h = app.tabs[i].scene.add_entity(EntityType::Text(dst_text));

        let mut dst_mtext = MText::new();
        dst_mtext.value = "DSTM".into();
        dst_mtext.height = 2.0;
        let dst_mtext_h = app.tabs[i].scene.add_entity(EntityType::MText(dst_mtext));

        // Drive the interactive command exactly as the viewport does:
        // phase 1 source pick, phase 2 destination selection.
        let _ = app.run_command_line("MATCHPROP");
        assert!(app.tabs[i].active_cmd.is_some(), "MATCHPROP must start");
        let _ = app.feed_command(StepInput::EntityPick(src_h, glam::DVec3::ZERO));
        let _ = app.feed_command(StepInput::SelectionComplete(vec![
            dst_text_h,
            dst_mtext_h,
        ]));

        let doc = &app.tabs[i].scene.document;
        match doc.get_entity(dst_text_h) {
            Some(EntityType::Text(t)) => {
                assert_eq!(t.style, "BIG", "TEXT destination must take source style");
                assert!(
                    (t.height - 5.0).abs() < 1e-9,
                    "TEXT destination must take source height, got {}",
                    t.height
                );
            }
            other => panic!("dest TEXT missing: {other:?}"),
        }
        match doc.get_entity(dst_mtext_h) {
            Some(EntityType::MText(m)) => {
                assert_eq!(m.style, "BIG", "MTEXT destination must take source style");
                assert!(
                    (m.height - 5.0).abs() < 1e-9,
                    "MTEXT destination must take source height, got {}",
                    m.height
                );
            }
            other => panic!("dest MTEXT missing: {other:?}"),
        }
    }

    #[test]
    fn handing_over_an_already_open_drawing_switches_to_its_tab() {
        // Double-clicking a drawing that is already open should land on the tab
        // showing it, not load a second copy of the same file.
        use crate::app::Message;
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);

        let path = std::env::temp_dir().join("ocs_already_open.dwg");
        std::fs::write(&path, b"x").unwrap();
        let canon = std::fs::canonicalize(&path).unwrap();

        // Two tabs, the second holding the drawing; leave the first active.
        app.tabs
            .push(crate::app::document::DocumentTab::new_drawing(99));
        let target = app.tabs.len() - 1;
        app.tabs[target].current_path = Some(canon.clone());
        app.active_tab = 0;

        let _ = app.update(Message::OpenExternal(canon.clone()));
        assert_eq!(app.active_tab, target, "should have switched to the tab");
        assert!(
            app.opening.is_none(),
            "an already-open drawing must not start a load"
        );
        assert!(app.pending_opens.is_empty(), "and must not queue one either");

        // The same file spelled differently (a `..` hop) is still the same file.
        let indirect = canon.parent().unwrap().join("..").join(
            canon
                .strip_prefix(canon.parent().unwrap().parent().unwrap())
                .unwrap(),
        );
        app.active_tab = 0;
        let _ = app.update(Message::OpenExternal(indirect));
        assert_eq!(
            app.active_tab, target,
            "an unresolved spelling of the same path must still match the tab"
        );
        assert!(app.opening.is_none(), "still no second load");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_second_handoff_queues_instead_of_displacing_the_first() {
        // `opening` is one slot, and `on_file_opened` drops any result that
        // arrives once it is clear — so without the queue, two drawings handed
        // over at the same moment (select several files in a file manager: one
        // process each, all arriving together) would leave one tab and silently
        // lose the rest.
        use crate::app::Message;
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);

        // Any existing file will do: OpenRecent only stats it, and the actual
        // load is an async Task this test drops.
        let dir = std::env::temp_dir();
        let (a, b) = (dir.join("ocs_si_a.dwg"), dir.join("ocs_si_b.dwg"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();

        let _ = app.update(Message::OpenExternal(a.clone()));
        assert!(
            app.opening.is_some(),
            "first handoff should start an open, not queue"
        );
        assert!(app.pending_opens.is_empty(), "nothing to queue yet");

        let _ = app.update(Message::OpenExternal(b.clone()));
        assert_eq!(
            app.pending_opens.len(),
            1,
            "second handoff arriving mid-open must queue, not be dropped"
        );
        assert_eq!(app.pending_opens.front(), Some(&b));

        // A failed drawing pauses the queue while its recovery report is shown.
        let open_id = app.opening.as_ref().map(|opening| opening.id).unwrap();
        let _ = app.update(Message::FileOpened(open_id, Err("boom".into())));
        assert!(
            app.active_modal == Some(crate::app::ModalKind::Recovery),
            "failed open should show its recovery report"
        );
        assert_eq!(
            app.pending_opens.len(),
            1,
            "queued drawing should wait until the report is acknowledged"
        );
        let _ = app.update(Message::RecoveryClose);
        assert!(
            app.pending_opens.is_empty(),
            "closing the report must release the queued drawing"
        );

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn saving_over_an_existing_drawing_succeeds() {
        for (label, pre_existing) in [("new path", false), ("existing drawing", true)] {
            let path = std::env::temp_dir().join(format!(
                "ocs_save_over_{}_{}.dxf",
                std::process::id(),
                pre_existing,
            ));
            let _ = std::fs::remove_file(&path);
            if pre_existing {
                std::fs::write(&path, b"a previous drawing").unwrap();
            }

            let mut app = OpenCADStudio::new_for_test();
            app.automation_op(r#"{"op":"new"}"#);
            let p = path.to_string_lossy().replace('\\', "\\\\");
            let saved = app.automation_op(&format!(r#"{{"op":"save","path":"{p}"}}"#));
            assert_eq!(saved["ok"], true, "{label}: {}", saved["error"]);
            let saved_again = app.automation_op(r#"{"op":"save"}"#);
            assert_eq!(
                saved_again["ok"],
                true,
                "normal save: {}",
                saved_again["error"]
            );

            drop(app);
            let sidecar = path.with_file_name(format!(
                ".{}.ocs.lock",
                path.file_name().unwrap().to_string_lossy()
            ));
            let _ = std::fs::remove_file(sidecar);
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn save_then_open_round_trips() {
        let mut app = OpenCADStudio::new_for_test();
        let path = std::env::temp_dir().join(format!(
            "ocs_automation_test_{}.dxf",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().replace('\\', "\\\\");
        app.automation_op(r#"{"op":"new"}"#);
        assert_eq!(
            app.automation_op(&format!(r#"{{"op":"save","path":"{p}"}}"#))["ok"],
            true
        );
        assert_eq!(
            app.automation_op(&format!(r#"{{"op":"open","path":"{p}"}}"#))["ok"],
            true
        );
        drop(app);
        let sidecar = path.with_file_name(format!(
            ".{}.ocs.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_file(sidecar);
        let _ = std::fs::remove_file(&path);
    }
}
