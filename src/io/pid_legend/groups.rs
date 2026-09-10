//! The tagged symbols as plot groups: what the SVG export wraps in one
//! `<g tagName="…">` each, so a consumer of the file can find a valve by its
//! tag.

use std::collections::HashMap;

use acadrust::Handle;

use crate::io::plot_emit::PlotGroup;

use super::{Recognition, Recognized};

/// One [`PlotGroup`] per symbol that carries a tag, named by the tag, over
/// the wires of the symbol's own entities and of the lettering the tag was
/// read from -- `WireModel::name` is the entity handle in decimal, for a
/// block reference's expansion as for a loose stroke. A symbol without a tag
/// is not a group: there is nothing to name it by. Two symbols may share a
/// tag (a motorised valve and the `XV` bubble it takes its tag from) and
/// then make two groups of the same name, as they are two symbols.
///
/// The groups are disjoint: an entity two symbols both count as theirs -- the
/// `S` lettered in a spray point sitting on a tank's rim is inside both
/// circles -- goes with the smaller symbol, the one it is more nearly inside
/// of, and the same one every time.
pub fn plot_groups(recognition: &Recognition) -> Vec<PlotGroup> {
    let tagged: Vec<&Recognized> = recognition
        .symbols
        .iter()
        .filter(|symbol| {
            symbol
                .tag
                .as_deref()
                .is_some_and(|tag| !tag.trim().is_empty())
        })
        .collect();
    let area = |symbol: &Recognized| {
        let (x0, y0, x1, y1) = symbol.bbox;
        (x1 - x0).abs() * (y1 - y0).abs()
    };
    // Each handle's owner: the smallest symbol that lists it, first on a tie.
    let mut owner: HashMap<Handle, usize> = HashMap::new();
    for (i, symbol) in tagged.iter().enumerate() {
        for handle in symbol.handles.iter().chain(&symbol.tag_handles) {
            if handle.is_null() {
                continue;
            }
            let claim = owner.entry(*handle).or_insert(i);
            if area(tagged[*claim]) > area(symbol) {
                *claim = i;
            }
        }
    }
    tagged
        .iter()
        .enumerate()
        .filter_map(|(i, symbol)| {
            let mut members: Vec<String> = symbol
                .handles
                .iter()
                .chain(&symbol.tag_handles)
                .filter(|handle| owner.get(handle) == Some(&i))
                .map(|handle| handle.value().to_string())
                .collect();
            members.sort();
            members.dedup();
            (!members.is_empty()).then(|| PlotGroup {
                tag: symbol.tag.as_deref().unwrap_or_default().trim().to_string(),
                members,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(tag: Option<&str>, handles: &[u64], tag_handles: &[u64]) -> Recognized {
        sized(tag, handles, tag_handles, 1.0)
    }

    fn sized(tag: Option<&str>, handles: &[u64], tag_handles: &[u64], side: f64) -> Recognized {
        Recognized {
            class: "butterfly".into(),
            label: "蝶阀".into(),
            color: [255, 140, 0],
            at: (0.0, 0.0),
            bbox: (0.0, 0.0, side, side),
            source: "$VALVE$01".into(),
            known: true,
            inner_text: Vec::new(),
            tag: tag.map(str::to_string),
            tag_distance_mm: tag.map(|_| 3.0),
            wants_tag: true,
            report_untagged: true,
            lines: Vec::new(),
            handles: handles.iter().map(|&h| Handle::new(h)).collect(),
            tag_handles: tag_handles.iter().map(|&h| Handle::new(h)).collect(),
        }
    }

    /// A tagged symbol is a group over its own entities and its tag's, by
    /// handle value; an untagged one, or one nothing was drawn for, is not.
    #[test]
    fn a_tagged_symbol_is_a_group_over_its_entities_and_its_tag() {
        let recognition = Recognition {
            symbols: vec![
                symbol(Some("BUV-3101"), &[0x2A], &[0x2B]),
                symbol(None, &[0x30], &[]),
                symbol(Some("XV-0407"), &[0x40, 0x41, 0x41], &[]),
                symbol(Some(" "), &[0x50], &[]),
                symbol(Some("PSV0407A"), &[], &[]),
                symbol(Some("GV0326A + GV0326B"), &[0x60, 0], &[0x61, 0x62]),
            ],
            ..Recognition::default()
        };
        let groups = plot_groups(&recognition);
        let names: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|g| {
                (
                    g.tag.as_str(),
                    g.members.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("BUV-3101", vec!["42", "43"]),
                ("XV-0407", vec!["64", "65"]),
                ("GV0326A + GV0326B", vec!["96", "97", "98"]),
            ]
        );
    }

    /// Lettering inside two circles at once -- the `S` of a spray point on
    /// a tank's rim is within the tank's inner radius too -- goes with the
    /// smaller symbol, whichever was recognised first; a symbol left with
    /// nothing of its own is no group.
    #[test]
    fn an_entity_two_symbols_claim_goes_with_the_smaller_one() {
        let recognition = Recognition {
            symbols: vec![
                sized(Some("TD-0208"), &[0x10, 0x11, 0x20], &[], 40.0),
                sized(Some("S"), &[0x21, 0x20], &[], 3.0),
                sized(Some("S"), &[0x20], &[], 30.0),
            ],
            ..Recognition::default()
        };
        let groups = plot_groups(&recognition);
        let names: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|g| {
                (
                    g.tag.as_str(),
                    g.members.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [("TD-0208", vec!["16", "17"]), ("S", vec!["32", "33"])]
        );
        // The same handle in two symbols of one size stays with the first.
        let tie = Recognition {
            symbols: vec![
                sized(Some("A"), &[0x10, 0x20], &[], 5.0),
                sized(Some("B"), &[0x11, 0x20], &[], 5.0),
            ],
            ..Recognition::default()
        };
        let groups = plot_groups(&tie);
        assert_eq!(groups[0].members, ["16", "32"]);
        assert_eq!(groups[1].members, ["17"]);
    }
}
