//! P&ID symbol census for a DXF / DWG: which blocks are placed, how often,
//! what lettering sits beside each placement, and where the block's
//! connection points land.
//!
//! Written for the CPECC fire-water / foam / drainage sheets exported by TWT
//! (`D:\work\plant-code\cad\0版重新处理dxf-12张`), whose symbols are blocks
//! named `$<class>$<8 digits>` carrying two `POINT` entities as connection
//! points -- but nothing here is specific to that family. Any drawing whose
//! symbols are block references gets the same report; a sheet whose symbols
//! were exploded into loose lines shows up here as "few blocks", which is
//! itself the answer (go cluster geometry instead).
//!
//! ```text
//! cargo run --example dxf_pid_probe -- <file.dxf> [more files...]
//!     [--radius-mm N]     lettering within N mm of an insertion point counts
//!                         as that placement's tags (default 15)
//!     [--units-per-mm N]  override unit detection (100 for a 1:100 sheet drawn
//!                         in model space, 1 for a sheet drawn in paper mm)
//!     [--verbose]         one line per placement: world-space connection
//!                         points and the tags nearest to it
//!     [--json]            machine-readable output instead of the text report
//! ```
//!
//! Units: a TWT sheet is an A3 drawn at 1:100 in model space, so one paper
//! millimetre is 100 drawing units; the SP02 sheets in the same folder are
//! drawn in paper millimetres. The probe guesses from the model-space extent
//! (wider than 5000 units means 1:100) and prints which it chose, so every
//! millimetre figure below can be trusted or overridden.

use std::collections::BTreeMap;
use std::path::PathBuf;

use acadrust::types::Vector3;
use acadrust::{CadDocument, EntityType};
use OpenCADStudio::io;

/// Lettering closer than this to an insertion point is reported as the
/// placement's tags. 15 mm covers a valve tag lettered beside its body and
/// the line number on the run it sits in, without reaching the next symbol
/// on a typical 25 mm pitch.
const DEFAULT_RADIUS_MM: f64 = 15.0;

/// A model-space extent wider than this is a sheet drawn at 1:100 in
/// hundredths of a millimetre; anything narrower is paper millimetres.
const MODEL_SPACE_SHEET_UNITS: f64 = 5000.0;

struct Options {
    files: Vec<PathBuf>,
    radius_mm: f64,
    units_per_mm: Option<f64>,
    verbose: bool,
    json: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        files: Vec::new(),
        radius_mm: DEFAULT_RADIUS_MM,
        units_per_mm: None,
        verbose: false,
        json: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--radius-mm" => {
                let value = args.next().ok_or("--radius-mm needs a value")?;
                options.radius_mm = value
                    .parse()
                    .map_err(|_| format!("--radius-mm: not a number: {value:?}"))?;
            }
            "--units-per-mm" => {
                let value = args.next().ok_or("--units-per-mm needs a value")?;
                let parsed: f64 = value
                    .parse()
                    .map_err(|_| format!("--units-per-mm: not a number: {value:?}"))?;
                if parsed <= 0.0 {
                    return Err("--units-per-mm must be positive".into());
                }
                options.units_per_mm = Some(parsed);
            }
            "--verbose" | "-v" => options.verbose = true,
            "--json" => options.json = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => options.files.push(PathBuf::from(other)),
        }
    }
    if options.files.is_empty() {
        return Err("usage: dxf_pid_probe <file.dxf|file.dwg> [more files...] [--radius-mm N] [--units-per-mm N] [--verbose] [--json]".into());
    }
    Ok(options)
}

/// A piece of lettering in model space, with the point a reader would take
/// as "where it is": the alignment point when the text has one, else the
/// insertion point.
struct Lettering {
    at: (f64, f64),
    value: String,
}

/// One block reference in model space.
struct Placement {
    at: (f64, f64),
    scale: (f64, f64),
    rotation_deg: f64,
    layer: String,
    attributes: Vec<(String, String)>,
}

/// What a block definition draws, measured in drawing units.
struct Body {
    kinds: BTreeMap<&'static str, usize>,
    bbox: Option<(f64, f64, f64, f64)>,
    /// `POINT` entities inside the definition, block-local. The TWT export
    /// marks a symbol's two pipe connections this way.
    connection_points: Vec<(f64, f64)>,
    base_point: (f64, f64),
}

/// The DXF group name of an entity (`LINE`, `LWPOLYLINE`, `ATTDEF`, ...), so
/// a body census reads the way the file does and nothing hides as "other".
fn kind_name(entity: &EntityType) -> &'static str {
    entity.as_entity().entity_type()
}

/// Whether a coordinate is one the drawing plausibly means, rather than the
/// sentinel or garbage an unset extent reads as.
fn sane(value: f64) -> bool {
    value.is_finite() && value.abs() < 1.0e12
}

fn grow(bbox: &mut Option<(f64, f64, f64, f64)>, min: Vector3, max: Vector3) {
    if !(sane(min.x) && sane(min.y) && sane(max.x) && sane(max.y)) {
        return;
    }
    *bbox = Some(match *bbox {
        None => (min.x, min.y, max.x, max.y),
        Some((x0, y0, x1, y1)) => (x0.min(min.x), y0.min(min.y), x1.max(max.x), y1.max(max.y)),
    });
}

fn measure_body(doc: &CadDocument, block_name: &str, base_point: Vector3) -> Body {
    let mut body = Body {
        kinds: BTreeMap::new(),
        bbox: None,
        connection_points: Vec::new(),
        base_point: (base_point.x, base_point.y),
    };
    for entity in doc.entities_in_block(block_name) {
        *body.kinds.entry(kind_name(entity)).or_default() += 1;
        match entity {
            EntityType::Point(point) => {
                body.connection_points
                    .push((point.location.x, point.location.y));
            }
            // A definition marker has no box, and lettering is placed rather
            // than drawn. A nested block reference is left out for the same
            // reason `drawn_extent` leaves inserts out: acadrust boxes an
            // insert at its insertion point, which for the title block's
            // nested logo is a metre and a half from anything the block
            // draws. It still shows in the kinds census as `INSERT`.
            EntityType::Seqend(_)
            | EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::AttributeDefinition(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
            | EntityType::Insert(_) => {}
            _ => {
                let bb = entity.as_entity().bounding_box();
                grow(&mut body.bbox, bb.min, bb.max);
            }
        }
    }
    body
}

/// Where a block-local point lands for a placement: scale about the base
/// point, rotate, then move to the insertion point. Negative scales are the
/// mirrored placements the exporter writes instead of a rotation.
fn place(local: (f64, f64), body: &Body, placement: &Placement) -> (f64, f64) {
    let x = (local.0 - body.base_point.0) * placement.scale.0;
    let y = (local.1 - body.base_point.1) * placement.scale.1;
    let (sin, cos) = placement.rotation_deg.to_radians().sin_cos();
    (
        x * cos - y * sin + placement.at.0,
        x * sin + y * cos + placement.at.1,
    )
}

/// A first reading of what a run of lettering is, from its shape alone.
fn classify(value: &str) -> &'static str {
    let bytes = value.as_bytes();
    let starts_digit = bytes.first().is_some_and(u8::is_ascii_digit);
    if value.contains("DWG-") || value.starts_with('接') {
        return "sheet-ref";
    }
    if starts_digit && value.contains('-') && value.chars().any(|c| c.is_ascii_uppercase()) {
        return "line-number";
    }
    if !value.is_empty()
        && value.chars().all(|c| c.is_ascii_digit())
        && (3..=5).contains(&value.len())
    {
        return "loop-number";
    }
    let letters: String = value
        .chars()
        .take_while(|c| c.is_ascii_uppercase())
        .collect();
    if !letters.is_empty() && (1..=5).contains(&letters.len()) {
        let rest = &value[letters.len()..];
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        if rest.is_empty() {
            return "function-letters";
        }
        if rest.chars().all(|c| c.is_ascii_alphanumeric())
            && rest.chars().any(|c| c.is_ascii_digit())
        {
            return "tag";
        }
    }
    if value.contains('"') || value.contains("NPT") || value.starts_with("DN") {
        return "size";
    }
    "text"
}

fn lettering_of(doc: &CadDocument) -> Vec<Lettering> {
    let mut out = Vec::new();
    for entity in doc.model_space_entities() {
        match entity {
            EntityType::Text(text) => {
                let value = text.value.trim();
                if value.is_empty() {
                    continue;
                }
                let anchor = text.alignment_point.unwrap_or(text.insertion_point);
                out.push(Lettering {
                    at: (anchor.x, anchor.y),
                    value: value.to_string(),
                });
            }
            EntityType::MText(mtext) => {
                let value = mtext.value.replace("\\P", " ");
                let value = value.trim();
                if value.is_empty() {
                    continue;
                }
                out.push(Lettering {
                    at: (mtext.insertion_point.x, mtext.insertion_point.y),
                    value: value.to_string(),
                });
            }
            _ => {}
        }
    }
    out
}

/// Lettering within `radius` of `at`, nearest first, as (distance, value).
fn tags_near(lettering: &[Lettering], at: (f64, f64), radius: f64) -> Vec<(f64, &str)> {
    let mut hits: Vec<(f64, &str)> = lettering
        .iter()
        .filter_map(|l| {
            let d = (l.at.0 - at.0).hypot(l.at.1 - at.1);
            (d <= radius).then_some((d, l.value.as_str()))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits
}

/// The model-space extent over drawn entities. Inserts are left out because
/// a title block placed at (977675, 443948) with its body authored at
/// (-945972, -443947) has an insertion point a kilometre from anything it
/// draws.
fn drawn_extent(doc: &CadDocument) -> Option<(f64, f64, f64, f64)> {
    let mut bbox = None;
    for entity in doc.model_space_entities() {
        if matches!(
            entity,
            EntityType::Insert(_)
                | EntityType::Block(_)
                | EntityType::BlockEnd(_)
                | EntityType::AttributeDefinition(_)
                | EntityType::Seqend(_)
        ) {
            continue;
        }
        let bb = entity.as_entity().bounding_box();
        // An empty placeholder reads as a zero box at the origin.
        if bb.min.x == 0.0 && bb.min.y == 0.0 && bb.max.x == 0.0 && bb.max.y == 0.0 {
            continue;
        }
        grow(&mut bbox, bb.min, bb.max);
    }
    bbox
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let mut reports = Vec::new();
    for path in &options.files {
        let name = path.display().to_string();
        let doc = match io::load_file(path) {
            Ok(doc) => doc,
            Err(error) => {
                eprintln!("{name}: load failed: {error}");
                continue;
            }
        };
        let report = probe(&doc, &options);
        if options.json {
            reports.push(
                serde_json::json!({ "file": name, "report": report_json(&report, &options) }),
            );
        } else {
            print_report(&name, &report, &options);
        }
    }
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports)
                .unwrap_or_else(|e| format!("{{\"error\":{e:?}}}"))
        );
    }
}

struct BlockReport {
    name: String,
    body: Body,
    placements: Vec<Placement>,
    /// Tag text -> how many placements had it within the radius.
    tags: BTreeMap<String, usize>,
}

struct Report {
    model_entities: usize,
    total_entities: usize,
    units_per_mm: f64,
    units_guessed: bool,
    extent_mm: Option<(f64, f64)>,
    lettering: usize,
    blocks: Vec<BlockReport>,
    lettering_index: Vec<Lettering>,
}

fn probe(doc: &CadDocument, options: &Options) -> Report {
    let extent = drawn_extent(doc);
    let (units_per_mm, units_guessed) = match options.units_per_mm {
        Some(value) => (value, false),
        None => {
            let width = extent.map_or(0.0, |(x0, _, x1, _)| x1 - x0);
            (
                if width > MODEL_SPACE_SHEET_UNITS {
                    100.0
                } else {
                    1.0
                },
                true,
            )
        }
    };
    let radius = options.radius_mm * units_per_mm;
    let lettering = lettering_of(doc);

    let mut by_block: BTreeMap<String, Vec<Placement>> = BTreeMap::new();
    for entity in doc.model_space_entities() {
        let EntityType::Insert(insert) = entity else {
            continue;
        };
        by_block
            .entry(insert.block_name.clone())
            .or_default()
            .push(Placement {
                at: (insert.insert_point.x, insert.insert_point.y),
                scale: (insert.x_scale(), insert.y_scale()),
                rotation_deg: insert.rotation.to_degrees(),
                layer: insert.common.layer.clone(),
                attributes: insert
                    .attributes
                    .iter()
                    .map(|a| (a.tag.clone(), a.value.clone()))
                    .collect(),
            });
    }

    let mut blocks: Vec<BlockReport> = by_block
        .into_iter()
        .map(|(name, placements)| {
            let base_point = doc
                .block_records
                .get(&name)
                .map_or(Vector3::ZERO, |record| record.base_point);
            let body = measure_body(doc, &name, base_point);
            let mut tags: BTreeMap<String, usize> = BTreeMap::new();
            for placement in &placements {
                for (_, value) in tags_near(&lettering, placement.at, radius) {
                    *tags.entry(value.to_string()).or_default() += 1;
                }
            }
            BlockReport {
                name,
                body,
                placements,
                tags,
            }
        })
        .collect();
    // Most-placed first: the symbols that matter on a sheet are the ones it
    // uses a dozen times, and the title block sits at the bottom.
    blocks.sort_by(|a, b| {
        b.placements
            .len()
            .cmp(&a.placements.len())
            .then_with(|| a.name.cmp(&b.name))
    });

    Report {
        model_entities: doc.model_space_entities().count(),
        total_entities: doc.entity_count(),
        units_per_mm,
        units_guessed,
        extent_mm: extent
            .map(|(x0, y0, x1, y1)| ((x1 - x0) / units_per_mm, (y1 - y0) / units_per_mm)),
        lettering: lettering.len(),
        blocks,
        lettering_index: lettering,
    }
}

fn print_report(name: &str, report: &Report, options: &Options) {
    let upm = report.units_per_mm;
    println!("{name}");
    println!(
        "  entities: {} in model space ({} incl. block definitions); lettering: {}",
        report.model_entities, report.total_entities, report.lettering
    );
    match report.extent_mm {
        Some((w, h)) => println!(
            "  units: {upm} per mm ({}); drawn extent {w:.1} x {h:.1} mm",
            if report.units_guessed {
                "guessed from extent, override with --units-per-mm"
            } else {
                "given"
            }
        ),
        None => println!(
            "  units: {upm} per mm ({}); nothing drawn in model space",
            if report.units_guessed {
                "guessed"
            } else {
                "given"
            }
        ),
    }
    let inserts: usize = report.blocks.iter().map(|b| b.placements.len()).sum();
    println!(
        "  blocks placed: {} distinct, {} placements; tags counted within {} mm",
        report.blocks.len(),
        inserts,
        options.radius_mm
    );
    if report.blocks.is_empty() {
        println!("  (no block references: this sheet's symbols are loose geometry, not blocks)");
    }

    for block in &report.blocks {
        let mut layers: BTreeMap<&str, usize> = BTreeMap::new();
        let mut scales: BTreeMap<String, usize> = BTreeMap::new();
        let mut rotations: BTreeMap<String, usize> = BTreeMap::new();
        for p in &block.placements {
            *layers.entry(p.layer.as_str()).or_default() += 1;
            *scales
                .entry(format!("({}, {})", round2(p.scale.0), round2(p.scale.1)))
                .or_default() += 1;
            *rotations
                .entry(format!("{}°", round1(p.rotation_deg.rem_euclid(360.0))))
                .or_default() += 1;
        }
        println!();
        println!(
            "  {}  x{}  layers: {}",
            block.name,
            block.placements.len(),
            join_counts(&layers)
        );
        let size = match block.body.bbox {
            Some((x0, y0, x1, y1)) => format!("{:.2} x {:.2} mm", (x1 - x0) / upm, (y1 - y0) / upm),
            None => "no drawn body".to_string(),
        };
        let kinds: Vec<String> = block
            .body
            .kinds
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect();
        println!(
            "    body: {size}; {}",
            if kinds.is_empty() {
                "empty".to_string()
            } else {
                kinds.join(", ")
            }
        );
        if block.body.connection_points.is_empty() {
            println!("    connection points: none (no POINT in the definition)");
        } else {
            let points: Vec<String> = block
                .body
                .connection_points
                .iter()
                .map(|(x, y)| {
                    format!(
                        "({:.2}, {:.2})",
                        (x - block.body.base_point.0) / upm,
                        (y - block.body.base_point.1) / upm
                    )
                })
                .collect();
            println!(
                "    connection points (mm, block-local): {}",
                points.join(" ")
            );
        }
        println!(
            "    placements: scale {}; rotation {}",
            join_counts(&scales),
            join_counts(&rotations)
        );
        if block.tags.is_empty() {
            println!("    nearby tags: none within {} mm", options.radius_mm);
        } else {
            let mut tags: Vec<(&String, &usize)> = block.tags.iter().collect();
            tags.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            let shown: Vec<String> = tags
                .iter()
                .take(12)
                .map(|(value, n)| format!("{value} [{}] x{n}", classify(value)))
                .collect();
            let more = tags.len().saturating_sub(12);
            println!(
                "    nearby tags: {}{}",
                shown.join(", "),
                if more > 0 {
                    format!(", +{more} more")
                } else {
                    String::new()
                }
            );
        }
        if options.verbose {
            for (index, p) in block.placements.iter().enumerate() {
                let conn: Vec<String> = block
                    .body
                    .connection_points
                    .iter()
                    .map(|&local| {
                        let (x, y) = place(local, &block.body, p);
                        format!("({:.1}, {:.1})", x, y)
                    })
                    .collect();
                let near: Vec<String> =
                    tags_near(&report.lettering_index, p.at, options.radius_mm * upm)
                        .iter()
                        .take(6)
                        .map(|(d, v)| format!("{v} ({:.1}mm)", d / upm))
                        .collect();
                let attributes: Vec<String> = p
                    .attributes
                    .iter()
                    .map(|(t, v)| format!("{t}={v}"))
                    .collect();
                println!(
                    "      #{:<3} at ({:.1}, {:.1}) rot {}° scale ({}, {}) layer {}{}",
                    index + 1,
                    p.at.0,
                    p.at.1,
                    round1(p.rotation_deg.rem_euclid(360.0)),
                    round2(p.scale.0),
                    round2(p.scale.1),
                    p.layer,
                    if attributes.is_empty() {
                        String::new()
                    } else {
                        format!(" attrs {}", attributes.join(" "))
                    }
                );
                if !conn.is_empty() {
                    println!("           conn: {}", conn.join(" "));
                }
                if !near.is_empty() {
                    println!("           tags: {}", near.join(", "));
                }
            }
        }
    }
    println!();
}

fn join_counts<K: std::fmt::Display>(counts: &BTreeMap<K, usize>) -> String {
    let mut items: Vec<(&K, &usize)> = counts.iter().collect();
    items.sort_by(|a, b| b.1.cmp(a.1));
    items
        .iter()
        .map(|(k, n)| format!("{k}:{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn report_json(report: &Report, options: &Options) -> serde_json::Value {
    let upm = report.units_per_mm;
    let blocks: Vec<serde_json::Value> = report
        .blocks
        .iter()
        .map(|block| {
            let placements: Vec<serde_json::Value> = block
                .placements
                .iter()
                .map(|p| {
                    let conn: Vec<serde_json::Value> = block
                        .body
                        .connection_points
                        .iter()
                        .map(|&local| {
                            let (x, y) = place(local, &block.body, p);
                            serde_json::json!([x, y])
                        })
                        .collect();
                    let tags: Vec<serde_json::Value> = tags_near(&report.lettering_index, p.at, options.radius_mm * upm)
                        .iter()
                        .map(|(d, v)| serde_json::json!({ "text": v, "kind": classify(v), "distance_mm": round2(d / upm) }))
                        .collect();
                    serde_json::json!({
                        "at": [p.at.0, p.at.1],
                        "scale": [p.scale.0, p.scale.1],
                        "rotation_deg": p.rotation_deg,
                        "layer": p.layer,
                        "attributes": p.attributes.iter().map(|(t, v)| serde_json::json!({ "tag": t, "value": v })).collect::<Vec<_>>(),
                        "connection_points": conn,
                        "tags": tags,
                    })
                })
                .collect();
            serde_json::json!({
                "name": block.name,
                "count": block.placements.len(),
                "body": {
                    "size_mm": block.body.bbox.map(|(x0, y0, x1, y1)| [round2((x1 - x0) / upm), round2((y1 - y0) / upm)]),
                    "kinds": block.body.kinds,
                    "connection_points_mm": block.body.connection_points.iter().map(|(x, y)| [round2((x - block.body.base_point.0) / upm), round2((y - block.body.base_point.1) / upm)]).collect::<Vec<_>>(),
                },
                "tags": block.tags,
                "placements": placements,
            })
        })
        .collect();
    serde_json::json!({
        "model_entities": report.model_entities,
        "total_entities": report.total_entities,
        "units_per_mm": upm,
        "units_guessed": report.units_guessed,
        "drawn_extent_mm": report.extent_mm.map(|(w, h)| [round1(w), round1(h)]),
        "lettering": report.lettering,
        "radius_mm": options.radius_mm,
        "blocks": blocks,
    })
}
