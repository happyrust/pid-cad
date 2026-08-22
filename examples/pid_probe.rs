//! Temporary probe: what does the `.pid` importer actually hand the scene?
//!
//! Both sides of the import are reported, because a thin-looking sheet has
//! two possible causes and they need telling apart: the parser handed over
//! little, or the importer filtered a lot out.

use std::collections::BTreeMap;

use acadrust::EntityType;
use OpenCADStudio::io;

fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::PathBuf::from(&arg);
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
        let mut labels: BTreeMap<String, usize> = BTreeMap::new();
        let mut heights: BTreeMap<String, usize> = BTreeMap::new();
        for e in doc.entities() {
            if e.common().owner_handle == doc.header.model_space_block_handle {
                owned += 1;
            }
            *per_layer.entry(e.common().layer.clone()).or_default() += 1;
            if let EntityType::Text(t) = e {
                if t.common.layer == "PID-SYMBOL-LABEL" {
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
                }
            }
        }
        println!("  owned_by_model_space = {owned}");
        println!("  layers:");
        for (layer, count) in &per_layer {
            println!("    {layer:<18} {count}");
        }
        println!("  symbol labels ({} distinct):", labels.len());
        for (name, count) in &labels {
            println!("    {count:>3} x {name}");
        }
        println!("  text height/rotation ({} distinct):", heights.len());
        for (key, count) in &heights {
            println!("    {count:>3} x {key}");
        }

        // A sheet is at most ~1189mm (A0) wide; anything reaching past 900 or
        // behind 0 either is the border or is the reason the view is wrong.
        println!("  entities reaching x>900 or x<0:");
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
            if pts.iter().any(|(x, _)| *x > 900.0 || *x < 0.0) {
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
