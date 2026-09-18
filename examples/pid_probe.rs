//! Temporary probe: what does the `.pid` importer actually hand the scene?
//!
//! Both sides of the import are reported, because a thin-looking sheet has
//! two possible causes and they need telling apart: the parser handed over
//! little, or the importer filtered a lot out.

use std::collections::BTreeMap;

use acadrust::EntityType;
use OpenCADStudio::io;

/// The `role=` the importer wrote into the entity's `PID_SEMANTICS` record.
fn pid_role(entity: &EntityType) -> Option<String> {
    entity
        .common()
        .extended_data
        .get_record("PID_SEMANTICS")?
        .values
        .iter()
        .find_map(|value| match value {
            acadrust::xdata::XDataValue::String(text) => {
                text.strip_prefix("role=").map(str::to_string)
            }
            _ => None,
        })
}

fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::PathBuf::from(&arg);
        if path.is_relative() && !symbol_library_is_discoverable(&path) {
            eprintln!(
                "warning: relative input {arg:?} has no discoverable symbol library; set PID_SYMBOL_LIBRARY or use an absolute drawing path"
            );
        }
        let doc = match io::load_file(&path) {
            Ok(doc) => doc,
            Err(error) => {
                println!("{arg}: FAILED {error}");
                continue;
            }
        };
        println!("{arg}");
        report_parser_side(&path);
        println!("  entities   = {}", doc.entity_count());
        println!(
            "  ext_min    = ({:.2}, {:.2})  ext_max = ({:.2}, {:.2})",
            doc.header.model_space_extents_min.x,
            doc.header.model_space_extents_min.y,
            doc.header.model_space_extents_max.x,
            doc.header.model_space_extents_max.y
        );
        println!("  ms_block   = {:?}", doc.header.model_space_block_handle);
        let vports: Vec<_> = doc.vports.iter().collect();
        println!("  vports     = {}", vports.len());
        for vp in vports {
            println!(
                "    name={:?} handle={:?} view_height={:.3} target=({:.2}, {:.2}) center=({:.2}, {:.2}) dir=({:.1},{:.1},{:.1}) ll=({:.2},{:.2}) ur=({:.2},{:.2})",
                vp.name, vp.handle, vp.view_height,
                vp.view_target.x, vp.view_target.y,
                vp.view_center.x, vp.view_center.y,
                vp.view_direction.x, vp.view_direction.y, vp.view_direction.z,
                vp.lower_left.x, vp.lower_left.y,
                vp.upper_right.x, vp.upper_right.y
            );
        }
        let mut owned = 0usize;
        let mut per_layer: BTreeMap<String, usize> = BTreeMap::new();
        // By the importer's own reading rather than the layer slot, which
        // `OCS_PID_LAYER_MODE=sheet` hands to the authored sheet layer: the
        // role is what an entity *is* in either mode.
        let mut per_role: BTreeMap<String, usize> = BTreeMap::new();
        let mut labels: BTreeMap<String, usize> = BTreeMap::new();
        let mut heights: BTreeMap<String, usize> = BTreeMap::new();
        let mut typefaces: BTreeMap<String, usize> = BTreeMap::new();
        for e in doc.entities() {
            if e.common().owner_handle == doc.header.model_space_block_handle {
                owned += 1;
            }
            *per_layer.entry(e.common().layer.clone()).or_default() += 1;
            let role = pid_role(e);
            *per_role
                .entry(role.clone().unwrap_or_else(|| "<no role>".to_string()))
                .or_default() += 1;
            if let EntityType::Text(t) = e {
                if role.as_deref() == Some("symbol-label") {
                    *labels.entry(t.value.clone()).or_default() += 1;
                } else {
                    // `rotation` is stored in radians, which reads as 0 / 2 / 3
                    // at this precision. Show degrees: a reader checking a
                    // P&ID wants to know a label stands up, not that it holds
                    // 1.5707963.
                    *heights
                        .entry(format!(
                            "{:.2}mm rot={:.0}deg",
                            t.height,
                            t.rotation.to_degrees()
                        ))
                        .or_default() += 1;
                    // What the label will actually be drawn in: the style
                    // names a typeface, and a name the machine has no font for
                    // falls back silently, so print the typeface rather than
                    // the style name.
                    let face = doc
                        .text_styles
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case(&t.style))
                        .map_or("<no such style>", |s| s.true_type_font.trim());
                    let face = if face.is_empty() { "<unstated>" } else { face };
                    *typefaces.entry(face.to_string()).or_default() += 1;
                }
            }
        }
        println!("  owned_by_model_space = {owned}");
        // The table as the Layer Manager lists it, with the state each layer
        // opens in -- under `OCS_PID_LAYER_MODE=sheet` that is the drawing's
        // own list, present-and-empty layers included.
        println!("  layer table ({}):", doc.layers.len());
        for layer in doc.layers.iter() {
            println!(
                "    {:<18} {:<3} {}",
                layer.name,
                if layer.flags.off { "off" } else { "on" },
                per_layer.get(&layer.name).copied().unwrap_or_default()
            );
        }
        println!("  layers (with entities):");
        for (layer, count) in &per_layer {
            println!("    {layer:<18} {count}");
        }
        println!("  roles (XDATA role=):");
        for (role, count) in &per_role {
            println!("    {role:<18} {count}");
        }
        println!("  symbol labels ({} distinct):", labels.len());
        for (name, count) in &labels {
            println!("    {count:>3} x {name}");
        }
        println!("  text height/rotation ({} distinct):", heights.len());
        for (key, count) in &heights {
            println!("    {count:>3} x {key}");
        }
        println!("  text typefaces ({} distinct):", typefaces.len());
        for (face, count) in &typefaces {
            println!("    {count:>3} x {face}");
        }

        // Check both axes: a vertical outlier breaks framing just as surely as
        // a horizontal one.
        println!("  entities reaching x/y>900 or x/y<0:");
        let mut outliers = 0usize;
        for e in doc.entities() {
            let pts: Vec<(f64, f64)> = match e {
                EntityType::Line(l) => vec![(l.start.x, l.start.y), (l.end.x, l.end.y)],
                EntityType::LwPolyline(p) => p
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect(),
                EntityType::Circle(c) => vec![(c.center.x, c.center.y)],
                EntityType::Arc(a) => vec![(a.center.x, a.center.y)],
                EntityType::Text(t) => vec![(t.insertion_point.x, t.insertion_point.y)],
                EntityType::Point(p) => vec![(p.location.x, p.location.y)],
                _ => Vec::new(),
            };
            if pts
                .iter()
                .any(|(x, y)| *x > 900.0 || *x < 0.0 || *y > 900.0 || *y < 0.0)
            {
                outliers += 1;
                if outliers <= 12 {
                    let kind = match e {
                        EntityType::Line(_) => "Line",
                        EntityType::LwPolyline(_) => "LwPolyline",
                        EntityType::Circle(_) => "Circle",
                        EntityType::Arc(_) => "Arc",
                        EntityType::Text(_) => "Text",
                        EntityType::Point(_) => "Point",
                        _ => "other",
                    };
                    let shown: Vec<String> = pts
                        .iter()
                        .take(4)
                        .map(|(x, y)| format!("({x:.1},{y:.1})"))
                        .collect();
                    println!("    {kind:<12} {}", shown.join(" "));
                }
            }
        }
        println!("    total outliers = {outliers}");
    }
}

fn symbol_library_is_discoverable(drawing: &std::path::Path) -> bool {
    if std::env::var_os("PID_SYMBOL_LIBRARY")
        .is_some_and(|value| std::env::split_paths(&value).any(|root| root.is_dir()))
    {
        return true;
    }
    let absolute = std::env::current_dir()
        .map(|cwd| cwd.join(drawing))
        .unwrap_or_else(|_| drawing.to_path_buf());
    let mut dir = absolute.parent();
    for _ in 0..5 {
        let Some(at) = dir else { break };
        for holder in [at.to_path_buf(), at.join("Ref")] {
            if std::fs::read_dir(holder).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    entry.file_type().is_ok_and(|kind| kind.is_dir())
                        && entry
                            .file_name()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains("symbol")
                })
            }) {
                return true;
            }
        }
        dir = at.parent();
    }
    false
}

/// What `pid-parse` offered, before `load_pid` decided what to draw.
///
/// The page line is the one to read first: `page_dimensions_mm` is what the
/// importer frames on, and `page_transform` says whether the parser knows the
/// sheet's coordinate space or the importer is falling back to its own
/// metre-to-millimetre constant.
fn report_parser_side(path: &std::path::Path) {
    use pid_parse::{
        build_normalized_geometry, PidCoordinateSpace, PidDrawingUnits, PidGeometryConfidence,
        PidGraphicKind, PidPageTransform, PidParser,
    };

    let parsed = match PidParser::new().parse_file(path) {
        Ok(parsed) => parsed,
        Err(error) => {
            println!("  parser    : FAILED {error}");
            return;
        }
    };
    let geometry = build_normalized_geometry(&parsed);

    println!("  page_mm    = {:?}", geometry.page_dimensions_mm);
    println!("  records    = {}", geometry.entities.len());

    let mut by_kind: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut spaces: BTreeMap<&str, usize> = BTreeMap::new();
    let mut units: BTreeMap<String, usize> = BTreeMap::new();
    let mut transforms: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &geometry.entities {
        let kind = match &entity.kind {
            PidGraphicKind::Line { .. } => "Line",
            PidGraphicKind::Polyline { .. } => "Polyline",
            PidGraphicKind::Arc { .. } => "Arc",
            PidGraphicKind::Circle { .. } => "Circle",
            PidGraphicKind::Point { .. } => "Point",
            PidGraphicKind::Text { .. } => "Text",
            PidGraphicKind::SymbolInstance { .. } => "SymbolInstance",
            PidGraphicKind::Annotation { .. } => "Annotation",
            PidGraphicKind::Unknown { .. } => "Unknown",
        };
        let confidence = match entity.confidence {
            PidGeometryConfidence::Decoded => "decoded",
            PidGeometryConfidence::Inferred => "inferred",
            PidGeometryConfidence::ProbeOnly => "probe",
        };
        *by_kind.entry((kind, confidence)).or_default() += 1;
        *spaces
            .entry(match entity.coordinate_context.coordinate_space {
                PidCoordinateSpace::SourceSheet => "source_sheet",
                PidCoordinateSpace::Model => "model",
                PidCoordinateSpace::Page => "page",
                PidCoordinateSpace::Viewport => "viewport",
                PidCoordinateSpace::Unknown => "unknown",
            })
            .or_default() += 1;
        *units
            .entry(match &entity.coordinate_context.units {
                PidDrawingUnits::Known { unit } => format!("known:{unit}"),
                PidDrawingUnits::Unknown { .. } => "unknown".into(),
            })
            .or_default() += 1;
        *transforms
            .entry(match &entity.coordinate_context.page_transform {
                PidPageTransform::Available { .. } => "available",
                PidPageTransform::Unavailable { .. } => "unavailable",
            })
            .or_default() += 1;
    }

    println!("  kind x confidence:");
    for ((kind, confidence), count) in &by_kind {
        println!("    {kind:<15} {confidence:<9} {count}");
    }
    println!("  coordinate_space = {spaces:?}");
    println!("  units            = {units:?}");
    println!("  page_transform   = {transforms:?}");
    for warning in &geometry.warnings {
        println!("  warning: {warning}");
    }
}
