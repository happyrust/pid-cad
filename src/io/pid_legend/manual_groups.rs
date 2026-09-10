//! P&ID symbols explicitly grouped by a person.
//!
//! A GROUP whose description carries `tagName=` owns its model-space
//! members before any automatic recognition pass sees them. Its current
//! lettering supplies an `auto` tag; a `manual` tag is the stored value.

use std::collections::HashSet;

use acadrust::objects::{Group, ObjectType};
use acadrust::{CadDocument, EntityType, Handle};

use super::blocks::{grow, lettering_value, placed_block, skip_for_box};
use super::tags::derive_tag_from_texts;
use super::*;

/// The P&ID identity a marked DXF GROUP presents to recognition and the
/// Properties panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDetails {
    pub name: String,
    pub tag: Option<String>,
    pub source: TagSource,
    pub class: TagClass,
}

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
        let letter_refs: Vec<&Lettering> = letters.iter().map(|(_, letter)| *letter).collect();
        let tag = effective_group_tag(&carried, &letter_refs, rules);
        let (class, port_circles) =
            classify_group(&members, &letter_refs, tag.as_deref(), rules, upm);

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
                    if port_circles.contains(&handle) {
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
            class: class.class,
            label: class.label,
            color: class.color,
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

/// Describe one marked group independently of a stored Recognition. This is
/// cheap (only the group's members are inspected) and keeps the Properties
/// panel's displayed tag and type on the same rules as recognition.
pub fn group_details(
    doc: &CadDocument,
    group_handle: Handle,
    rules: &Rules,
) -> Option<GroupDetails> {
    let ObjectType::Group(group) = doc.objects.get(&group_handle)? else {
        return None;
    };
    let carried = GroupTag::parse(&group.description)?;
    let members: Vec<&EntityType> = group
        .entities
        .iter()
        .filter_map(|handle| doc.get_entity(*handle))
        .collect();
    let mut letters: Vec<Lettering> = members
        .iter()
        .filter_map(|entity| lettering_value(entity))
        .collect();
    letters.sort_by(|a, b| {
        b.at.1
            .total_cmp(&a.at.1)
            .then_with(|| a.at.0.total_cmp(&b.at.0))
    });
    let letter_refs: Vec<&Lettering> = letters.iter().collect();
    let tag = effective_group_tag(&carried, &letter_refs, rules);
    let upm = if members
        .iter()
        .any(|entity| matches!(entity, EntityType::Circle(_)))
    {
        guess_units_per_mm(doc)
    } else {
        1.0
    };
    let (class, _) = classify_group(&members, &letter_refs, tag.as_deref(), rules, upm);
    Some(GroupDetails {
        name: group_name(doc, group),
        tag,
        source: carried.source,
        class,
    })
}

fn effective_group_tag(
    carried: &GroupTag,
    letters: &[&Lettering],
    rules: &Rules,
) -> Option<String> {
    match carried.source {
        TagSource::Auto => {
            let values: Vec<&str> = letters.iter().map(|letter| letter.value.as_str()).collect();
            derive_tag_from_texts(&values, rules).map(|read| read.value)
        }
        TagSource::Manual => carried.name.clone(),
    }
}

/// Classify by the M3 evidence order and return the circles whose rims are
/// pipe ports. A decorative circle inside an exploded shape is not a port:
/// only a circle that matched a configured circle rule is.
fn classify_group(
    members: &[&EntityType],
    letters: &[&Lettering],
    tag: Option<&str>,
    rules: &Rules,
    upm: f64,
) -> (TagClass, HashSet<Handle>) {
    let block_class = members.iter().find_map(|entity| {
        let EntityType::Insert(insert) = entity else {
            return None;
        };
        rules
            .blocks
            .get(&insert.block_name)
            .filter(|rule| rule.class != IGNORE_CLASS)
            .map(|rule| TagClass {
                class: rule.class.clone(),
                label: rule.label.clone(),
                color: rule.color,
            })
    });
    let mut circle_class = None;
    let mut port_circles = HashSet::new();
    for entity in members {
        let EntityType::Circle(circle) = entity else {
            continue;
        };
        let inner: Vec<String> = letters
            .iter()
            .filter(|letter| {
                (letter.at.0 - circle.center.x).hypot(letter.at.1 - circle.center.y)
                    <= circle.radius * INNER_TEXT_RADIUS
            })
            .map(|letter| letter.value.clone())
            .collect();
        if let Some(rule) = rules.circle_rule(circle.radius / upm, &inner) {
            port_circles.insert(circle.common.handle);
            circle_class.get_or_insert_with(|| TagClass {
                class: rule.class.clone(),
                label: rule.label.clone(),
                color: rule.color,
            });
        }
    }
    let tag_class = tag.and_then(|tag| class_for_tag(tag, rules));
    let class = block_class
        .or(circle_class)
        .or(tag_class)
        .unwrap_or_else(|| TagClass {
            class: rules.manual_group.class.clone(),
            label: rules.manual_group.label.clone(),
            color: rules.manual_group.color,
        });
    (class, port_circles)
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
