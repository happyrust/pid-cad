//! Helpers of the block and circle passes: the lettering of a sheet, the
//! units it is drawn in, where a block's stem ends, how a bubble's inner
//! text composes into a tag.

use acadrust::entities::Insert;
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

/// The placed body and pipe connection points of one block reference.
///
/// Both the ordinary block pass and a hand-made P&ID group use this exact
/// expansion, so grouping an INSERT changes who owns the symbol without
/// moving its box or its pipe ports.
pub(super) struct PlacedBlock {
    pub bbox: (f64, f64, f64, f64),
    pub at: (f64, f64),
    pub ports: Vec<Port>,
}

pub(super) fn placed_block(
    doc: &CadDocument,
    insert: &Insert,
    upm: f64,
    port_rule: Option<PortRule>,
) -> Option<PlacedBlock> {
    let record = doc.block_records.get(&insert.block_name)?;
    let base = (record.base_point.x, record.base_point.y);
    let insertion = (insert.insert_point.x, insert.insert_point.y);
    let scale = (insert.x_scale(), insert.y_scale());
    let mut bbox = None;
    let mut ports = Vec::new();
    let mut connections = 0;
    let mut local_lines: Vec<Segment> = Vec::new();
    let mut local_corners: Vec<(f64, f64)> = Vec::new();
    for member in doc.entities_in_block(&insert.block_name) {
        if let EntityType::Point(point) = member {
            let p = place(
                (point.location.x, point.location.y),
                base,
                scale,
                insert.rotation,
                insertion,
            );
            ports.push(Port::At(p));
            connections += 1;
        }
        if skip_for_box(member) {
            continue;
        }
        if port_rule == Some(PortRule::StemEnd) {
            if let EntityType::Line(line) = member {
                local_lines.push(((line.start.x, line.start.y), (line.end.x, line.end.y)));
            }
        }
        let bb = member.as_entity().bounding_box();
        for corner in [
            (bb.min.x, bb.min.y),
            (bb.min.x, bb.max.y),
            (bb.max.x, bb.min.y),
            (bb.max.x, bb.max.y),
        ] {
            if port_rule == Some(PortRule::StemEnd) {
                local_corners.push(corner);
            }
            let (x, y) = place(corner, base, scale, insert.rotation, insertion);
            grow(&mut bbox, x, y);
        }
    }
    let half = EMPTY_BODY_HALF_MM * upm;
    let bbox = bbox.unwrap_or((
        insertion.0 - half,
        insertion.1 - half,
        insertion.0 + half,
        insertion.1 + half,
    ));
    if connections == 0 {
        let port = match port_rule {
            Some(PortRule::StemEnd) => stem_end(&local_lines, &local_corners, base)
                .map(|p| place(p, base, scale, insert.rotation, insertion))
                .unwrap_or(insertion),
            Some(PortRule::Insertion) | None => insertion,
        };
        ports.push(Port::At(port));
    }
    let at = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
    Some(PlacedBlock { bbox, at, ports })
}

/// The lettering an entity is, when it is one: a TEXT's value at its
/// alignment point (else its insertion point), an MTEXT's value with its
/// paragraph breaks as spaces at its insertion point. Blank lettering, and
/// anything that is not text, is none. The one reading of text shared by
/// the sheet pass and a group's own lettering.
pub(super) fn lettering_value(entity: &EntityType) -> Option<Lettering> {
    match entity {
        EntityType::Text(text) => {
            let value = text.value.trim();
            if value.is_empty() {
                return None;
            }
            let anchor = text.alignment_point.unwrap_or(text.insertion_point);
            Some(Lettering {
                handle: text.common.handle,
                at: (anchor.x, anchor.y),
                value: value.to_string(),
            })
        }
        EntityType::MText(mtext) => {
            let value = mtext.value.replace("\\P", " ");
            let value = value.trim();
            if value.is_empty() {
                return None;
            }
            Some(Lettering {
                handle: mtext.common.handle,
                at: (mtext.insertion_point.x, mtext.insertion_point.y),
                value: value.to_string(),
            })
        }
        _ => None,
    }
}

pub(super) fn lettering_of(doc: &CadDocument) -> Vec<Lettering> {
    doc.model_space_entities()
        .filter_map(lettering_value)
        .collect()
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

    /// A bubble's two lines compose; one line, or two that are not a line of
    /// capitals and a line of digits, do not. (How the composition is used
    /// is `tags::tag_from_lettering`'s, tested there.)
    #[test]
    fn inner_lettering_composes_a_bubble_tag() {
        assert_eq!(
            compose_tag(&words(&["XV", "3201"])),
            Some("XV-3201".to_string())
        );
        assert_eq!(
            compose_tag(&words(&["HS", "0320A"])),
            Some("HS-0320A".to_string())
        );
        assert_eq!(
            compose_tag(&words(&["FQRC", "0301"])),
            Some("FQRC-0301".to_string())
        );
        assert_eq!(compose_tag(&words(&["S"])), None);
        assert_eq!(compose_tag(&[]), None);
        assert_eq!(compose_tag(&words(&["E", "H"])), None);
    }

    /// Text reads at its alignment point when it has one, MTEXT with its
    /// paragraph breaks as spaces; blank text and other entities are not
    /// lettering.
    #[test]
    fn an_entity_reads_as_lettering_or_not() {
        use acadrust::entities::{Line, MText, Text};
        use acadrust::types::Vector3;
        let plain = EntityType::Text(Text::with_value(" BUV-3101 ", Vector3::new(1.0, 2.0, 0.0)));
        let read = lettering_value(&plain).expect("text is lettering");
        assert_eq!((read.value.as_str(), read.at), ("BUV-3101", (1.0, 2.0)));
        let mut aligned = Text::with_value("XV", Vector3::new(1.0, 2.0, 0.0));
        aligned.alignment_point = Some(Vector3::new(5.0, 6.0, 0.0));
        assert_eq!(
            lettering_value(&EntityType::Text(aligned)).unwrap().at,
            (5.0, 6.0)
        );
        let mut mtext = MText::new();
        mtext.value = "XV-0301\\P防爆电动平板闸阀".to_string();
        mtext.insertion_point = Vector3::new(3.0, 4.0, 0.0);
        let read = lettering_value(&EntityType::MText(mtext)).unwrap();
        assert_eq!(
            (read.value.as_str(), read.at),
            ("XV-0301 防爆电动平板闸阀", (3.0, 4.0))
        );
        assert!(
            lettering_value(&EntityType::Text(Text::with_value("  ", Vector3::ZERO))).is_none()
        );
        assert!(lettering_value(&EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::ZERO
        )))
        .is_none());
    }
}
