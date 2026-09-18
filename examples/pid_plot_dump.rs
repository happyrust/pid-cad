//! Temporary probe: dump an imported drawing as flat CSV so it can be
//! plotted and eyeballed.
//!
//! Row shape matches `pid-parse`'s `dump_symbol_geometry`, so the same
//! plotting script draws either a single symbol body or a whole sheet:
//!
//! ```text
//! line,x1,y1,x2,y2
//! circle,cx,cy,r
//! poly,closed(0|1),x1,y1,x2,y2,...
//! text,x,y,height,rotation_rad,"value"
//! ```
//!
//! A row that draws with its own width and colour rather than the layer's
//! carries a trailing `@RRGGBB:WW` token, `WW` being the line weight in
//! hundredths of a millimetre the way DXF stores it. A line that also draws
//! dashed appends its linetype name to the same token, `@RRGGBB:WW:LT`, so a
//! plot can tell a dashed line from a solid one. The token still goes last and
//! is still the only non-numeric field a `poly` can have, so a reader that
//! only wants geometry drops it without needing to know the row's arity.
//! Without it the dump cannot show what the style table is for: a 0.13mm solid
//! instrument line and a 0.7mm dashed process header have the same coordinates
//! either way.
//!
//! Arcs are emitted as sampled polylines rather than as their own row, which
//! keeps the plotter from having to know this crate's angle convention.
//!
//! Usage: `pid_plot_dump <file> [selector,selector,...]`
//!
//! A selector is an exact layer name (`PID-SYMBOL`, or `Labels` under
//! `OCS_PID_LAYER_MODE=sheet`) or `role=<role>` for the importer's own
//! reading of the entity (`role=geometry`, `role=symbol-label`), which is the
//! same in either layer mode. Names are exact, not a prefix: `PID-SYMBOL` and
//! `PID-SYMBOL-LABEL` are different things and the latter ships hidden.

use acadrust::types::{Color, LineWeight};
use acadrust::EntityType;
use OpenCADStudio::io;

const ARC_STEPS: usize = 48;

/// One `layer` or `role=<role>` selector of the command line.
enum Selector {
    Layer(String),
    Role(String),
}

impl Selector {
    fn parse(text: &str) -> Self {
        match text.strip_prefix("role=") {
            Some(role) => Self::Role(role.to_string()),
            None => Self::Layer(text.to_string()),
        }
    }

    fn admits(&self, entity: &EntityType) -> bool {
        match self {
            Self::Layer(layer) => layer_of(entity) == Some(layer.as_str()),
            Self::Role(role) => role_of(entity).as_deref() == Some(role.as_str()),
        }
    }
}

/// The `role=` the importer wrote into the entity's `PID_SEMANTICS` record.
fn role_of(entity: &EntityType) -> Option<String> {
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
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: pid_plot_dump <file> [layer|role=<role>,...]");
        return;
    };
    let wanted: Option<Vec<Selector>> = args
        .next()
        .map(|list| list.split(',').map(Selector::parse).collect());
    let doc = match io::load_file(&std::path::PathBuf::from(&path)) {
        Ok(doc) => doc,
        Err(error) => {
            eprintln!("{path}: FAILED {error}");
            return;
        }
    };

    for entity in doc.entities() {
        if let Some(selectors) = &wanted {
            if !selectors.iter().any(|selector| selector.admits(entity)) {
                continue;
            }
        }
        let row = match entity {
            EntityType::Line(l) => {
                format!("line,{},{},{},{}", l.start.x, l.start.y, l.end.x, l.end.y)
            }
            EntityType::Circle(c) => {
                format!("circle,{},{},{}", c.center.x, c.center.y, c.radius)
            }
            EntityType::Arc(a) => {
                // Radians already -- see the angle-unit note in `io::pid`. This
                // used to call `to_radians()`, which was harmless while the
                // importer stored degrees and became a second conversion the
                // moment it stopped, flattening every arc to 1/57.3 of its
                // sweep.
                let (from, to) = (a.start_angle, a.end_angle);
                let sweep = {
                    let raw = to - from;
                    if raw <= 0.0 {
                        raw + std::f64::consts::TAU
                    } else {
                        raw
                    }
                };
                let points: Vec<String> = (0..=ARC_STEPS)
                    .flat_map(|i| {
                        let angle = from + sweep * (i as f64 / ARC_STEPS as f64);
                        let (sin, cos) = angle.sin_cos();
                        [
                            (a.center.x + a.radius * cos).to_string(),
                            (a.center.y + a.radius * sin).to_string(),
                        ]
                    })
                    .collect();
                format!("poly,0,{}", points.join(","))
            }
            EntityType::LwPolyline(p) => {
                if p.vertices.len() < 2 {
                    continue;
                }
                let points: Vec<String> = p
                    .vertices
                    .iter()
                    .flat_map(|v| [v.location.x.to_string(), v.location.y.to_string()])
                    .collect();
                format!("poly,{},{}", u8::from(p.is_closed), points.join(","))
            }
            EntityType::Text(t) => {
                if t.value.trim().is_empty() {
                    continue;
                }
                format!(
                    "text,{},{},{},{},{:?}",
                    t.insertion_point.x, t.insertion_point.y, t.height, t.rotation, t.value
                )
            }
            _ => continue,
        };
        println!("{row}{}", style_token(entity));
    }
}

/// The width, colour and dashed linetype an entity draws with, as a trailing
/// `@RRGGBB:WW` or `@RRGGBB:WW:LT` token, or nothing where it draws `ByLayer`.
///
/// `ByLayer` is what a symbol body and the diagnostic layers keep, so an
/// absent token is a statement rather than a gap: the style table had nothing
/// to say about that row. The `:LT` suffix appears only for a line drawing a
/// real linetype -- `Continuous` / `ByLayer` draw solid and add nothing.
fn style_token(entity: &EntityType) -> String {
    let common = entity.common();
    let Color::Rgb { r, g, b } = common.color else {
        return String::new();
    };
    let LineWeight::Value(weight) = common.line_weight else {
        return String::new();
    };
    let dash = match common.linetype.as_str() {
        "" | "Continuous" | "ByLayer" | "ByBlock" => String::new(),
        name => format!(":{name}"),
    };
    format!(",@{r:02X}{g:02X}{b:02X}:{weight}{dash}")
}

fn layer_of(entity: &EntityType) -> Option<&str> {
    match entity {
        EntityType::Line(l) => Some(l.common.layer.as_str()),
        EntityType::Circle(c) => Some(c.common.layer.as_str()),
        EntityType::Arc(a) => Some(a.common.layer.as_str()),
        EntityType::LwPolyline(p) => Some(p.common.layer.as_str()),
        EntityType::Text(t) => Some(t.common.layer.as_str()),
        _ => None,
    }
}
