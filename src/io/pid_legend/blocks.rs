//! Helpers of the block and circle passes: the lettering of a sheet, the
//! units it is drawn in, where a block's stem ends, how a bubble's inner
//! text composes into a tag.

use acadrust::{CadDocument, EntityType};

use super::*;

pub(super) fn sane(value: f64) -> bool {
    value.is_finite() && value.abs() < 1.0e12
}

pub(super) fn grow(bbox: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
    if !(sane(x) && sane(y)) {
        return;
    }
    *bbox = Some(match *bbox {
        None => (x, y, x, y),
        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    });
}

/// Entities whose bounding box says nothing about what a block draws.
pub(super) fn skip_for_box(entity: &EntityType) -> bool {
    matches!(
        entity,
        // acadrust boxes an insert at its insertion point (see dxf_pid_probe).
        EntityType::Insert(_)
            | EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::AttributeDefinition(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
    )
}

/// Where a block-local point lands for a placement.
pub(super) fn place(
    local: (f64, f64),
    base: (f64, f64),
    scale: (f64, f64),
    rotation: f64,
    at: (f64, f64),
) -> (f64, f64) {
    let x = (local.0 - base.0) * scale.0;
    let y = (local.1 - base.1) * scale.1;
    let (sin, cos) = rotation.sin_cos();
    (x * cos - y * sin + at.0, x * sin + y * cos + at.1)
}

/// A block's line, block coordinates.
pub(super) type Segment = ((f64, f64), (f64, f64));

/// The far end of a block along its stem, block coordinates
/// ([`PortRule::StemEnd`]). The stem is the block's line that starts at the
/// base point (the one nearest it, if none quite does -- within a twentieth
/// of the block's reach); the far end is the point on the stem's axis where
/// the block's geometry (`corners`, its members' boxes) stops. None when the
/// block has no line at its base.
pub(super) fn stem_end(
    lines: &[Segment],
    corners: &[(f64, f64)],
    base: (f64, f64),
) -> Option<(f64, f64)> {
    let dist = |p: (f64, f64)| (p.0 - base.0).hypot(p.1 - base.1);
    let reach = corners.iter().map(|&c| dist(c)).fold(0.0, f64::max);
    let (near, far) = lines
        .iter()
        .map(|&(a, b)| if dist(a) <= dist(b) { (a, b) } else { (b, a) })
        .min_by(|x, y| dist(x.0).total_cmp(&dist(y.0)))?;
    if reach <= 0.0 || dist(near) > reach / 20.0 {
        return None;
    }
    let length = (far.0 - near.0).hypot(far.1 - near.1);
    if length <= 0.0 {
        return None;
    }
    let direction = ((far.0 - near.0) / length, (far.1 - near.1) / length);
    let along = corners
        .iter()
        .map(|c| (c.0 - base.0) * direction.0 + (c.1 - base.1) * direction.1)
        .fold(f64::NEG_INFINITY, f64::max);
    if !along.is_finite() {
        return None;
    }
    Some((base.0 + direction.0 * along, base.1 + direction.1 * along))
}

pub(super) fn lettering_of(doc: &CadDocument) -> Vec<Lettering> {
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

/// The model-space extent over drawn entities, inserts left out (their box
/// is just the insertion point, which for a title block is nowhere near what
/// it draws).
fn drawn_extent(doc: &CadDocument) -> Option<(f64, f64, f64, f64)> {
    let mut bbox = None;
    for entity in doc.model_space_entities() {
        if skip_for_box(entity) {
            continue;
        }
        let bb = entity.as_entity().bounding_box();
        if bb.min.x == 0.0 && bb.min.y == 0.0 && bb.max.x == 0.0 && bb.max.y == 0.0 {
            continue;
        }
        grow(&mut bbox, bb.min.x, bb.min.y);
        grow(&mut bbox, bb.max.x, bb.max.y);
    }
    bbox
}

/// Drawing units per paper millimetre, from the sheet's drawn extent.
pub fn guess_units_per_mm(doc: &CadDocument) -> f64 {
    let width = drawn_extent(doc).map_or(0.0, |(x0, _, x1, _)| x1 - x0);
    if width > MODEL_SPACE_SHEET_UNITS {
        100.0
    } else {
        1.0
    }
}

/// A circle's inner lettering as a tag, when it has the two parts of one: a
/// line of capitals and a line of digits (optionally suffixed, `0320A`)
/// become `HS-0320A`.
pub(super) fn compose_tag(inner: &[String]) -> Option<String> {
    let letters = inner
        .iter()
        .find(|v| (1..=5).contains(&v.len()) && v.chars().all(|c| c.is_ascii_uppercase()));
    let digits = inner.iter().find(|v| {
        let body = v.trim_end_matches(|c: char| c.is_ascii_uppercase());
        (3..=5).contains(&body.len())
            && body.chars().all(|c| c.is_ascii_digit())
            && v.len() - body.len() <= 1
    });
    match (letters, digits) {
        (Some(l), Some(d)) => Some(format!("{l}-{d}")),
        _ => None,
    }
}

/// [`compose_tag`], or the inner lettering joined as read when it is not a
/// tag (`S`, `K`, `M`).
pub(super) fn inner_tag(inner: &[String]) -> Option<String> {
    compose_tag(inner).or_else(|| (!inner.is_empty()).then(|| inner.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_stem_ends_where_the_block_stops_along_it() {
        // The vent stub as TWT draws it: a stem from the base point along -x
        // and a cup at its end, closed 245 units past the stem's own end.
        let lines: [Segment; 4] = [
            ((-2430.8, 0.0), (0.0, 0.0)),
            ((-2675.7, 252.7), (-2675.7, -252.7)),
            ((-2675.7, -252.7), (-2430.8, -252.7)),
            ((-2675.7, 252.7), (-2430.8, 252.7)),
        ];
        let corners: Vec<(f64, f64)> = lines
            .iter()
            .flat_map(|&(a, b)| {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                [(x0, y0), (x0, y1), (x1, y0), (x1, y1)]
            })
            .collect();
        let end = stem_end(&lines, &corners, (0.0, 0.0)).unwrap();
        assert!(
            (end.0 + 2675.7).abs() < 1e-9 && end.1.abs() < 1e-9,
            "{end:?}"
        );
        // A block whose lines are all far from its base has no stem.
        assert_eq!(stem_end(&lines[1..], &corners, (0.0, 0.0)), None);
        // A stem pointing the other way, base off the origin.
        let up: [Segment; 2] = [((10.0, 5.0), (10.0, 20.0)), ((8.0, 20.0), (12.0, 22.0))];
        let corners = [(10.0, 5.0), (10.0, 20.0), (8.0, 20.0), (12.0, 22.0)];
        let end = stem_end(&up, &corners, (10.0, 5.0)).unwrap();
        assert!(
            (end.0 - 10.0).abs() < 1e-9 && (end.1 - 22.0).abs() < 1e-9,
            "{end:?}"
        );
    }

    #[test]
    fn inner_lettering_composes_a_bubble_tag() {
        assert_eq!(
            inner_tag(&words(&["XV", "3201"])),
            Some("XV-3201".to_string())
        );
        assert_eq!(
            inner_tag(&words(&["HS", "0320A"])),
            Some("HS-0320A".to_string())
        );
        assert_eq!(
            inner_tag(&words(&["FQRC", "0301"])),
            Some("FQRC-0301".to_string())
        );
        assert_eq!(inner_tag(&words(&["S"])), Some("S".to_string()));
        assert_eq!(inner_tag(&[]), None);
        assert_eq!(compose_tag(&words(&["S"])), None);
        assert_eq!(compose_tag(&words(&["E", "H"])), None);
    }
}
