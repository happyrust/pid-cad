//! The legend as entities: the `PID-LEGEND-<CLASS>` and `PID-PIPE-<line>`
//! layers, the boxes, labels, runs and rings drawn on them, and taking them
//! out again.

use std::collections::BTreeMap;

use acadrust::entities::{Circle, LwPolyline, Text};
use acadrust::types::{Color, Vector2, Vector3};
use acadrust::{CadDocument, EntityType, Handle};

use super::exploded::fnv1a;
use super::pid_pipes::{self, End};
use super::*;

/// Whether `class` is an exploded shape nothing has named yet.
pub fn is_shape_class(class: &str) -> bool {
    class.starts_with(SHAPE_CLASS_PREFIX)
}

/// The layer a class's rectangles and labels go on.
pub fn layer_for_class(class: &str) -> String {
    if is_shape_class(class) {
        return SHAPE_LAYER.to_string();
    }
    format!("{LAYER_PREFIX}{}", class.to_ascii_uppercase())
}

/// The layer the runs carrying `line` are drawn on.
pub fn pipe_layer(line: &str) -> String {
    format!("{PIPE_LAYER_PREFIX}{line}")
}

/// Whether `layer` is one [`legend_entities`] writes to: a class layer or a
/// pipe layer.
pub fn is_legend_layer(layer: &str) -> bool {
    layer.starts_with(LAYER_PREFIX) || layer.starts_with(PIPE_LAYER_PREFIX)
}

/// Layer name -> colour for every class in `recognition` and every line
/// number its pipe carries. Unnamed shapes share [`SHAPE_LAYER`], white;
/// each draws in its own colour.
pub fn legend_layers(recognition: &Recognition) -> BTreeMap<String, [u8; 3]> {
    let mut layers: BTreeMap<String, [u8; 3]> = recognition
        .symbols
        .iter()
        .map(|s| {
            if is_shape_class(&s.class) {
                (SHAPE_LAYER.to_string(), [255, 255, 255])
            } else {
                (layer_for_class(&s.class), s.color)
            }
        })
        .collect();
    layers.extend(pipe_layers(recognition));
    layers
}

/// The layers a run is drawn on: one per line number it carries, so each
/// line's layer shows the whole line; [`PIPE_NONE_LAYER`] for a run on none.
fn run_layers(run: &pid_pipes::Run) -> Vec<String> {
    if run.lines.is_empty() {
        vec![PIPE_NONE_LAYER.to_string()]
    } else {
        run.lines.iter().map(|line| pipe_layer(line)).collect()
    }
}

/// Layer name -> colour for the pipe runs: a colour of its own per line
/// number, derived from the number so the same line is the same colour on
/// every sheet; grey for the runs on no numbered line.
pub fn pipe_layers(recognition: &Recognition) -> BTreeMap<String, [u8; 3]> {
    let mut out = BTreeMap::new();
    for run in &recognition.pipes.runs {
        if run.lines.is_empty() {
            out.insert(PIPE_NONE_LAYER.to_string(), PIPE_NONE_COLOR);
        }
        for line in &run.lines {
            out.insert(pipe_layer(line), hash_color(fnv1a(line.as_bytes())));
        }
    }
    out
}

/// The pipe runs drawn in: a polyline along each run on the layer of every
/// line number it carries -- a run two lines both reach is drawn on both, so
/// either layer alone shows its whole line -- or on [`PIPE_NONE_LAYER`] when
/// it carries none, and a ring of `pipes.open_end_mm` wherever a run ends in
/// the air. All ByLayer.
pub fn pipe_entities(recognition: &Recognition, rules: &Rules) -> Vec<EntityType> {
    let radius = rules.pipes.open_end_mm * recognition.units_per_mm;
    let mut out = Vec::new();
    for run in &recognition.pipes.runs {
        let Some((&first, &last)) = run.path.first().zip(run.path.last()) else {
            continue;
        };
        for layer in run_layers(run) {
            let mut polyline = LwPolyline::from_points(
                run.path.iter().map(|&(x, y)| Vector2::new(x, y)).collect(),
            );
            polyline.common.layer = layer.clone();
            polyline.common.color = Color::ByLayer;
            out.push(EntityType::LwPolyline(polyline));
            for (end, at) in [(run.ends[0], first), (run.ends[1], last)] {
                if end != End::Open {
                    continue;
                }
                let mut ring = Circle::new();
                ring.center = Vector3::new(at.0, at.1, 0.0);
                ring.radius = radius;
                ring.common.layer = layer.clone();
                ring.common.color = Color::ByLayer;
                out.push(EntityType::Circle(ring));
            }
        }
    }
    out
}

/// A closed rectangle and a one-line label per recognised symbol, each on
/// its class layer with colour ByLayer (an unnamed shape carries its own
/// colour), then the pipe runs ([`pipe_entities`]). The label reads
/// `<label> <tag>`, or just the label when the symbol carries no tag.
pub fn legend_entities(recognition: &Recognition, rules: &Rules) -> Vec<EntityType> {
    let upm = recognition.units_per_mm;
    let pad = rules.pad_mm * upm;
    let height = rules.label_mm * upm;
    let mut out = Vec::with_capacity(recognition.symbols.len() * 2);
    for symbol in &recognition.symbols {
        let layer = layer_for_class(&symbol.class);
        let color = if is_shape_class(&symbol.class) {
            Color::Rgb {
                r: symbol.color[0],
                g: symbol.color[1],
                b: symbol.color[2],
            }
        } else {
            Color::ByLayer
        };
        let (x0, y0, x1, y1) = symbol.bbox;
        let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
        let mut rectangle = LwPolyline::from_points(vec![
            Vector2::new(x0, y0),
            Vector2::new(x1, y0),
            Vector2::new(x1, y1),
            Vector2::new(x0, y1),
        ]);
        rectangle.is_closed = true;
        rectangle.common.layer = layer.clone();
        rectangle.common.color = color;
        out.push(EntityType::LwPolyline(rectangle));

        let text = match &symbol.tag {
            Some(tag) => format!("{} {tag}", symbol.label),
            None => symbol.label.clone(),
        };
        let mut label =
            Text::with_value(text, Vector3::new(x0, y1 + 0.3 * height, 0.0)).with_height(height);
        label.common.layer = layer;
        label.common.color = color;
        out.push(EntityType::Text(label));
    }
    out.extend(pipe_entities(recognition, rules));
    out
}

fn ensure_layer(doc: &mut CadDocument, name: &str, color: [u8; 3]) {
    let rgb = Color::Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
    };
    if let Some(layer) = doc.layers.get_mut(name) {
        layer.color = rgb;
        return;
    }
    let mut layer = acadrust::tables::Layer::new(name);
    layer.handle = doc.allocate_handle();
    layer.color = rgb;
    let _ = doc.layers.add(layer);
}

/// Draw the legend into `doc` (headless path): create the class and pipe
/// layers in their colours and add the rectangles, labels and runs to model
/// space. Returns how many entities were added.
pub fn apply(doc: &mut CadDocument, recognition: &Recognition, rules: &Rules) -> usize {
    for (layer, color) in legend_layers(recognition) {
        ensure_layer(doc, &layer, color);
    }
    let mut added = 0;
    for entity in legend_entities(recognition, rules) {
        if doc.add_entity(entity).is_ok() {
            added += 1;
        }
    }
    added
}

/// Handles of every entity on a legend layer.
pub fn legend_handles(doc: &CadDocument) -> Vec<Handle> {
    doc.entities()
        .filter(|e| is_legend_layer(&e.common().layer))
        .map(|e| e.common().handle)
        .collect()
}

/// Remove everything [`apply`] added. Returns how many entities went. The
/// layers stay: empty, they purge like any other.
pub fn clear(doc: &mut CadDocument) -> usize {
    let handles = legend_handles(doc);
    handles
        .into_iter()
        .filter(|h| doc.remove_entity(*h).is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_layer_names() {
        assert_eq!(layer_for_class("butterfly"), "PID-LEGEND-BUTTERFLY");
        assert_eq!(layer_for_class("shape-1a2b3c4d"), SHAPE_LAYER);
        assert_eq!(pipe_layer("100-FW"), "PID-PIPE-100-FW");
        assert!(is_legend_layer("PID-LEGEND-TANK"));
        assert!(is_legend_layer(SHAPE_LAYER));
        assert!(is_legend_layer("PID-PIPE-200-FS-31001-A2"));
        assert!(is_legend_layer(PIPE_NONE_LAYER));
        assert!(!is_legend_layer("VALVE_消防"));
        assert!(!is_legend_layer("PIPE-消防"));
    }

    #[test]
    fn pipe_runs_draw_on_a_layer_per_line_and_ring_their_open_ends() {
        use super::pid_pipes::{End, Run};
        let run = |lines: &[&str], ends: [End; 2], path: &[(f64, f64)]| Run {
            numbers: Vec::new(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
            segments: path.len() - 1,
            length_mm: 0.0,
            ends,
            path: path.to_vec(),
            handles: Vec::new(),
        };
        let recognition = Recognition {
            units_per_mm: 100.0,
            pipes: Pipes {
                runs: vec![
                    run(
                        &["100-FW"],
                        [End::Open, End::Tee],
                        &[(0.0, 0.0), (500.0, 0.0)],
                    ),
                    run(&[], [End::Tee, End::Open], &[(500.0, 0.0), (500.0, 300.0)]),
                    run(
                        &["100-FW", "80-FW"],
                        [End::Tee, End::Symbol(0)],
                        &[(500.0, 0.0), (900.0, 0.0), (900.0, -200.0)],
                    ),
                ],
                ..Pipes::default()
            },
            ..Recognition::default()
        };
        let layers = pipe_layers(&recognition);
        assert_eq!(
            layers.keys().collect::<Vec<_>>(),
            ["PID-PIPE-100-FW", "PID-PIPE-80-FW", PIPE_NONE_LAYER]
        );
        assert_eq!(layers[PIPE_NONE_LAYER], PIPE_NONE_COLOR);
        assert_ne!(layers["PID-PIPE-100-FW"], layers["PID-PIPE-80-FW"]);
        assert_eq!(
            layers["PID-PIPE-100-FW"],
            hash_color(fnv1a(b"100-FW")),
            "a line's colour comes from its number, the same on every sheet"
        );

        let rules = Rules::default();
        let entities = pipe_entities(&recognition, &rules);
        let on = |layer: &str| -> Vec<&EntityType> {
            entities
                .iter()
                .filter(|e| e.common().layer == layer)
                .collect()
        };
        // The header piece and the two-line piece; one ring at the open end.
        let header = on("PID-PIPE-100-FW");
        assert_eq!(header.len(), 3, "{header:?}");
        assert!(
            matches!(header[0], EntityType::LwPolyline(p) if p.vertices.len() == 2 && !p.is_closed)
        );
        assert!(matches!(header[1], EntityType::Circle(c)
            if c.center.x == 0.0 && c.center.y == 0.0 && (c.radius - 60.0).abs() < 1e-9));
        assert!(matches!(header[2], EntityType::LwPolyline(p) if p.vertices.len() == 3));
        // The two-line piece is on the second line's layer too, whole.
        let branch = on("PID-PIPE-80-FW");
        assert_eq!(branch.len(), 1);
        assert!(matches!(branch[0], EntityType::LwPolyline(p) if p.vertices.len() == 3));
        // The unnumbered stub, grey, ringed where it stops.
        let none = on(PIPE_NONE_LAYER);
        assert_eq!(none.len(), 2);
        assert!(matches!(none[1], EntityType::Circle(c) if c.center.y == 300.0));
        assert_eq!(entities.len(), 6);
        assert!(entities.iter().all(|e| e.common().color == Color::ByLayer));
        assert!(entities.iter().all(|e| is_legend_layer(&e.common().layer)));
    }
}
