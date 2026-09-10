//! P&ID symbols explicitly grouped by a person.
//!
//! A GROUP whose description carries `tagName=` owns its model-space
//! members before any automatic recognition pass sees them. Its current
//! lettering supplies an `auto` tag; a `manual` tag is the stored value.

use std::collections::HashSet;

use acadrust::objects::{Group, ObjectType};
use acadrust::{CadDocument, EntityType, Handle};

use super::blocks::{grow, placed_block, skip_for_box};
use super::tags::derive_tag_from_texts;
use super::*;

pub(super) struct ManualGroups {
    pub symbols: Vec<Recognized>,
    pub excluded_handles: HashSet<Handle>,
    pub circle_handles: HashSet<Handle>,
    pub ports: Vec<(usize, Port)>,
}

/// Recognise every marked GROUP, and claim its model-space members before
/// the block, circle, exploded-shape and lettering passes run.
pub(super) fn recognise_manual_groups(
    doc: &CadDocument,
    rules: &Rules,
    upm: f64,
    lettering: &[Lettering],
    taken_text: &mut [bool],
) -> ManualGroups {
    let model_handles: HashSet<Handle> = doc
        .model_space_entities()
        .map(|entity| entity.common().handle)
        .collect();
    let mut marked: Vec<(&Group, GroupTag)> = doc
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::Group(group) => GroupTag::parse(&group.description).map(|tag| (group, tag)),
            _ => None,
        })
        .collect();
    marked.sort_by_key(|(group, _)| group.handle.value());

    let mut symbols = Vec::new();
    let mut excluded_handles = HashSet::new();
    let mut circle_handles = HashSet::new();
    let mut ports = Vec::new();

    for (group, carried) in marked {
        let member_handles: HashSet<Handle> = group
            .entities
            .iter()
            .copied()
            .filter(|handle| model_handles.contains(handle))
            .collect();
        if member_handles.is_empty() {
            continue;
        }
        let members: Vec<&EntityType> = group
            .entities
            .iter()
            .filter(|handle| member_handles.contains(handle))
            .filter_map(|handle| doc.get_entity(*handle))
            .collect();
        if members.is_empty() {
            continue;
        }
        excluded_handles.extend(member_handles.iter().copied());

        let mut letters: Vec<(usize, &Lettering)> = lettering
            .iter()
            .enumerate()
            .filter(|(_, letter)| member_handles.contains(&letter.handle))
            .collect();
        letters.sort_by(|(_, a), (_, b)| {
            b.at.1
                .total_cmp(&a.at.1)
                .then_with(|| a.at.0.total_cmp(&b.at.0))
        });
        for &(index, _) in &letters {
            taken_text[index] = true;
        }
        let text_values: Vec<&str> = letters
            .iter()
            .map(|(_, letter)| letter.value.as_str())
            .collect();
        let derived = derive_tag_from_texts(&text_values, rules);
        let tag = match carried.source {
            TagSource::Auto => derived.map(|read| read.value),
            TagSource::Manual => carried.name.clone(),
        };

        // Evidence order is deliberate: a known INSERT, then a circle rule,
        // then the effective tag's shape, then the configured manual class.
        let block_class = members.iter().find_map(|entity| {
            let EntityType::Insert(insert) = entity else {
                return None;
            };
            rules
                .blocks
                .get(&insert.block_name)
                .filter(|rule| rule.class != IGNORE_CLASS)
                .map(|rule| (rule.class.clone(), rule.label.clone(), rule.color))
        });
        let circle_matches: Vec<(Handle, &CircleRule)> = members
            .iter()
            .filter_map(|entity| {
                let EntityType::Circle(circle) = entity else {
                    return None;
                };
                let inner: Vec<String> = letters
                    .iter()
                    .filter(|(_, letter)| {
                        (letter.at.0 - circle.center.x).hypot(letter.at.1 - circle.center.y)
                            <= circle.radius * INNER_TEXT_RADIUS
                    })
                    .map(|(_, letter)| letter.value.clone())
                    .collect();
                rules
                    .circle_rule(circle.radius / upm, &inner)
                    .map(|rule| (circle.common.handle, rule))
            })
            .collect();
        let circle_class = circle_matches
            .first()
            .map(|(_, rule)| (rule.class.clone(), rule.label.clone(), rule.color));
        let tag_class = tag.as_deref().and_then(|tag| {
            class_for_tag(tag, rules).map(|class| (class.class, class.label, class.color))
        });
        let (class, label, color) =
            block_class
                .or(circle_class)
                .or(tag_class)
                .unwrap_or_else(|| {
                    (
                        rules.manual_group.class.clone(),
                        rules.manual_group.label.clone(),
                        rules.manual_group.color,
                    )
                });

        let mut bbox = None;
        let mut handles = Vec::new();
        let mut tag_handles = Vec::new();
        let mut group_ports = Vec::new();
        for entity in &members {
            let handle = entity.common().handle;
            if matches!(entity, EntityType::Text(_) | EntityType::MText(_)) {
                tag_handles.push(handle);
                continue;
            }
            handles.push(handle);
            match entity {
                EntityType::Insert(insert) => {
                    let port_rule = rules
                        .blocks
                        .get(&insert.block_name)
                        .and_then(|rule| rule.port);
                    if let Some(placed) = placed_block(doc, insert, upm, port_rule) {
                        grow(&mut bbox, placed.bbox.0, placed.bbox.1);
                        grow(&mut bbox, placed.bbox.2, placed.bbox.3);
                        group_ports.extend(placed.ports);
                    } else {
                        grow(&mut bbox, insert.insert_point.x, insert.insert_point.y);
                    }
                }
                EntityType::Circle(circle) => {
                    circle_handles.insert(handle);
                    grow(
                        &mut bbox,
                        circle.center.x - circle.radius,
                        circle.center.y - circle.radius,
                    );
                    grow(
                        &mut bbox,
                        circle.center.x + circle.radius,
                        circle.center.y + circle.radius,
                    );
                    if circle_matches
                        .iter()
                        .any(|(circle_handle, _)| *circle_handle == handle)
                    {
                        group_ports.push(Port::Rim {
                            centre: (circle.center.x, circle.center.y),
                            r: circle.radius,
                        });
                    }
                }
                _ if !skip_for_box(entity) => {
                    let bb = entity.as_entity().bounding_box();
                    grow(&mut bbox, bb.min.x, bb.min.y);
                    grow(&mut bbox, bb.max.x, bb.max.y);
                }
                _ => {}
            }
        }
        let fallback = letters.first().map_or((0.0, 0.0), |(_, letter)| letter.at);
        let half = EMPTY_BODY_HALF_MM * upm;
        let bbox = bbox.unwrap_or((
            fallback.0 - half,
            fallback.1 - half,
            fallback.0 + half,
            fallback.1 + half,
        ));
        let at = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
        let name = group_name(doc, group);
        let symbol_index = symbols.len();
        ports.extend(group_ports.into_iter().map(|port| (symbol_index, port)));
        symbols.push(Recognized {
            class,
            label,
            color,
            at,
            bbox,
            source: format!("group {name}"),
            known: true,
            inner_text: Vec::new(),
            tag_distance_mm: tag.as_ref().map(|_| 0.0),
            tag,
            wants_tag: true,
            report_untagged: true,
            lines: Vec::new(),
            handles,
            tag_handles,
            group: Some(GroupOrigin {
                name,
                tag_source: carried.source,
            }),
        });
    }

    ManualGroups {
        symbols,
        excluded_handles,
        circle_handles,
        ports,
    }
}

/// DXF keeps a GROUP's name on the ACAD_GROUP dictionary key but does not
/// currently copy it into `Group::name`; DWG and newly-created groups do.
fn group_name(doc: &CadDocument, group: &Group) -> String {
    if !group.name.trim().is_empty() {
        return group.name.clone();
    }
    let dictionary = doc.header.acad_group_dict_handle;
    if let Some(ObjectType::Dictionary(dict)) = doc.objects.get(&dictionary) {
        if let Some((name, _)) = dict
            .entries
            .iter()
            .find(|(_, handle)| *handle == group.handle)
        {
            return name.clone();
        }
    }
    format!("#{}", group.handle.value())
}
