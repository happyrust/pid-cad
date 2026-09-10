//! The tag rule, in one place: how a tag is read from a handful of lettering
//! -- the same whether the lettering is inside a bubble or is the text a
//! person put into a group with a symbol's strokes -- what class a tag's
//! shape names, and how a group carries its tag in its description.
//!
//! Reading ([`tag_from_lettering`]) goes: a piece of lettering shaped like a
//! tag (several: joined ` + `, as an assembly lists every tag it took); else
//! a line of capitals and a line of digits composed as a bubble's are
//! (`XV` / `3201` -> `XV-3201`); else the one piece there was; else the
//! pieces joined as read. The caller decides what it accepts: the bubble
//! pass takes anything long enough (a lone `S` says the symbol is lettered,
//! not tagged), a group takes a shape, a composition or a single piece and
//! never a join.
//!
//! A group made by hand is a P&ID symbol when its description carries
//! `tagName=` ([`GroupTag`]): `tagName=BUV-3101;tagSource=auto`, `key=value`
//! pieces separated by `;`, the same idiom as the `PID_SEMANTICS` XDATA. The
//! description is DXF code 300 and a DWG string, written and read by
//! acadrust as it stands, so the tag travels with the file and needs no
//! APPID. Only these two keys are ours; other text in the description is
//! left as it was.

use acadrust::objects::Group;
use acadrust::CadDocument;

use super::blocks::{compose_tag, lettering_value};
use super::*;

/// How a tag was read from a set of lettering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagHow {
    /// Lettering with the shape of a tag; several pieces joined ` + `.
    Shape,
    /// A line of capitals and a line of digits, composed as a bubble's.
    Bubble,
    /// The one piece of lettering there was.
    Single,
    /// Several pieces, none shaped like a tag, joined as read.
    Joined,
}

/// A tag read from lettering, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRead {
    pub value: String,
    pub how: TagHow,
}

/// Read a tag from `texts`, given in reading order. `shapes` are the tag
/// shapes to try first (see [`shape_matches`]); every piece that has one of
/// them is taken, joined ` + ` when there are several. With no shape
/// matched (or none given) the pieces compose as a bubble's do, else the
/// single piece is the tag, else the pieces are joined as read. `None` for
/// no lettering at all. Length is not checked here: `Rules::accepts_tag`
/// is the caller's.
pub fn tag_from_lettering(texts: &[&str], shapes: &[&str]) -> Option<TagRead> {
    let shaped: Vec<&str> = texts
        .iter()
        .copied()
        .filter(|text| shapes.iter().any(|shape| shape_matches(shape, text)))
        .collect();
    if !shaped.is_empty() {
        return Some(TagRead {
            value: shaped.join(" + "),
            how: TagHow::Shape,
        });
    }
    let owned: Vec<String> = texts.iter().map(|text| text.to_string()).collect();
    if let Some(value) = compose_tag(&owned) {
        return Some(TagRead {
            value,
            how: TagHow::Bubble,
        });
    }
    match texts {
        [] => None,
        [one] => Some(TagRead {
            value: one.to_string(),
            how: TagHow::Single,
        }),
        many => Some(TagRead {
            value: many.join(" "),
            how: TagHow::Joined,
        }),
    }
}

/// What a tag's shape says the symbol is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagClass {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
}

/// The class a tag names by its shape: the `tag_classes` (`BV9999*` is a
/// ball valve), then the block rules' tag shapes (`BUV-9999` a butterfly
/// valve), the shape dictionary's, the circle rules'. A ` + ` assembly goes
/// by its first tag. `None` when no rule has the shape.
pub fn class_for_tag(tag: &str, rules: &Rules) -> Option<TagClass> {
    let first = tag.split(" + ").next().unwrap_or(tag).trim();
    if let Some(rule) = rules
        .tag_classes
        .iter()
        .find(|rule| shape_matches(&rule.shape, first))
    {
        return Some(TagClass {
            class: rule.class.clone(),
            label: rule.label.clone(),
            color: rule.color,
        });
    }
    let block_rules = rules.blocks.values().chain(
        rules
            .shapes
            .iter()
            .flat_map(|shapes| shapes.dictionary.values()),
    );
    for rule in block_rules {
        if rule.class == IGNORE_CLASS {
            continue;
        }
        if let Some(shape) = &rule.tag.shape {
            if shape_matches(shape, first) {
                return Some(TagClass {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                });
            }
        }
    }
    for rule in &rules.circles {
        if let Some(shape) = &rule.tag.shape {
            if shape_matches(shape, first) {
                return Some(TagClass {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                });
            }
        }
    }
    None
}

// ── groups ───────────────────────────────────────────────────────────────

/// Description key that carries a group's tag, and marks the group as a
/// P&ID symbol.
pub const TAG_NAME_KEY: &str = "tagName";

/// Description key that says where the tag came from ([`TagSource`]).
pub const TAG_SOURCE_KEY: &str = "tagSource";

/// Where a group's tag comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TagSource {
    /// Read from the group's own lettering by the rule, and read again from
    /// the lettering as it stands whenever the sheet is recognised: edit the
    /// text and the tag follows. What the description stores is a copy for
    /// whoever reads the file.
    #[default]
    Auto,
    /// Set by a person; the stored value is the tag whatever the lettering
    /// says.
    Manual,
}

impl TagSource {
    fn as_str(self) -> &'static str {
        match self {
            TagSource::Auto => "auto",
            TagSource::Manual => "manual",
        }
    }

    fn parse(value: &str) -> TagSource {
        if value.trim().eq_ignore_ascii_case("manual") {
            TagSource::Manual
        } else {
            TagSource::Auto
        }
    }
}

/// A group's tag as its description carries it. `name` is `None` for a
/// group marked as a P&ID symbol that has no tag (yet).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupTag {
    pub name: Option<String>,
    pub source: TagSource,
}

impl GroupTag {
    pub fn auto(name: Option<String>) -> GroupTag {
        GroupTag {
            name: clean(name),
            source: TagSource::Auto,
        }
    }

    pub fn manual(name: impl Into<String>) -> GroupTag {
        GroupTag {
            name: clean(Some(name.into())),
            source: TagSource::Manual,
        }
    }

    /// The tag in `description`, or `None` when the description carries no
    /// `tagName=` -- a group that is not a P&ID symbol. A missing or unknown
    /// `tagSource=` reads as `auto`.
    pub fn parse(description: &str) -> Option<GroupTag> {
        let mut name = None;
        let mut source = TagSource::Auto;
        let mut marked = false;
        for (key, value) in pieces(description).filter_map(key_value) {
            if key == TAG_NAME_KEY {
                marked = true;
                name = clean(Some(value.to_string()));
            } else if key == TAG_SOURCE_KEY {
                source = TagSource::parse(value);
            }
        }
        marked.then_some(GroupTag { name, source })
    }

    /// `description` with this tag written into it: our two keys replaced in
    /// place (or appended), everything else kept as it was.
    pub fn write_into(&self, description: &str) -> String {
        let name_piece = format!(
            "{TAG_NAME_KEY}={}",
            self.name.as_deref().unwrap_or_default()
        );
        let source_piece = format!("{TAG_SOURCE_KEY}={}", self.source.as_str());
        let (mut wrote_name, mut wrote_source) = (false, false);
        let mut out: Vec<String> = Vec::new();
        for piece in pieces(description) {
            match key_value(piece) {
                Some((key, _)) if key == TAG_NAME_KEY => {
                    if !wrote_name {
                        out.push(name_piece.clone());
                        wrote_name = true;
                    }
                }
                Some((key, _)) if key == TAG_SOURCE_KEY => {
                    if !wrote_source {
                        out.push(source_piece.clone());
                        wrote_source = true;
                    }
                }
                _ => out.push(piece.to_string()),
            }
        }
        if !wrote_name {
            out.push(name_piece);
        }
        if !wrote_source {
            out.push(source_piece);
        }
        out.join(";")
    }

    /// `description` with our two keys taken out: the group is a plain
    /// group again.
    pub fn strip(description: &str) -> String {
        pieces(description)
            .filter(|piece| {
                !matches!(key_value(piece), Some((key, _)) if key == TAG_NAME_KEY || key == TAG_SOURCE_KEY)
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// A tag value fit for the description: trimmed, never empty, and holding
/// no `;` (the piece separator) -- a `;` becomes its full-width `；`.
fn clean(name: Option<String>) -> Option<String> {
    let name = name?;
    let name = name.trim().replace(';', "；");
    (!name.is_empty()).then_some(name)
}

fn pieces(description: &str) -> impl Iterator<Item = &str> {
    description
        .split(';')
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
}

fn key_value(piece: &str) -> Option<(&str, &str)> {
    let (key, value) = piece.split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// The lettering among `group`'s members, in reading order: top to bottom,
/// left to right.
pub(super) fn group_lettering(doc: &CadDocument, group: &Group) -> Vec<Lettering> {
    let mut out: Vec<Lettering> = group
        .entities
        .iter()
        .filter_map(|handle| doc.get_entity(*handle))
        .filter_map(lettering_value)
        .collect();
    out.sort_by(|a, b| {
        b.at.1
            .total_cmp(&a.at.1)
            .then_with(|| a.at.0.total_cmp(&b.at.0))
    });
    out
}

/// The tag `group`'s own lettering reads as, by the rule the recognition
/// applies everywhere else. Range annotations (`XV-0407A～0409A` names
/// several symbols, not one) are left out first; then the pieces shaped like
/// a tag under any rule are the tag -- else a bubble's composition of a line
/// of capitals and a line of digits -- else the one piece long enough to be
/// a tag (`tag_min_chars`). Several such pieces read as nothing: a group
/// that letters `DN100` and `1.6MPa` has no tag, not a tag of both. A tag too
/// short to be one (`S`) is none either.
pub fn derive_group_tag(doc: &CadDocument, group: &Group, rules: &Rules) -> Option<TagRead> {
    let lettering = group_lettering(doc, group);
    let texts: Vec<&str> = lettering
        .iter()
        .map(|l| l.value.as_str())
        .filter(|value| expand_range(value).is_none())
        .collect();
    let shapes = rules.tag_shapes();
    let read = match tag_from_lettering(&texts, &shapes) {
        Some(read) if matches!(read.how, TagHow::Shape | TagHow::Bubble) => read,
        // No shape, no composition: the single piece long enough, if there
        // is exactly one (a short `S` or `NO` beside it does not count).
        _ => {
            let long: Vec<&str> = texts
                .iter()
                .copied()
                .filter(|value| rules.accepts_tag(value))
                .collect();
            match long[..] {
                [one] => TagRead {
                    value: one.to_string(),
                    how: TagHow::Single,
                },
                _ => return None,
            }
        }
    };
    rules.accepts_tag(&read.value).then_some(read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::{Line, Text};
    use acadrust::objects::ObjectType;
    use acadrust::types::Vector3;
    use acadrust::{EntityType, Handle};

    fn read(texts: &[&str], shapes: &[&str]) -> Option<(String, TagHow)> {
        tag_from_lettering(texts, shapes).map(|r| (r.value, r.how))
    }

    /// The four ways lettering reads, in order of preference; a bubble's
    /// two lines compose whatever else is there, and one lone piece is
    /// itself.
    #[test]
    fn lettering_reads_as_a_shape_a_composition_a_single_piece_or_a_join() {
        let shapes = ["BUV-9999", "BV9999*", "*DWG-*"];
        assert_eq!(
            read(&["BUV-3101"], &shapes),
            Some(("BUV-3101".into(), TagHow::Shape))
        );
        // Shaped pieces win over everything else and keep their tails.
        assert_eq!(
            read(&["1/2\"NPT", "BV0301A 1/2\""], &shapes),
            Some(("BV0301A 1/2\"".into(), TagHow::Shape))
        );
        // Several shaped pieces are an assembly, in the order given.
        assert_eq!(
            read(&["GV0326A", "GV0326B", "DN50"], &["GV9999*"]),
            Some(("GV0326A + GV0326B".into(), TagHow::Shape))
        );
        // A bubble's two lines compose, shapes or none.
        assert_eq!(
            read(&["XV", "3201"], &shapes),
            Some(("XV-3201".into(), TagHow::Bubble))
        );
        assert_eq!(
            read(&["HS", "0320A"], &[]),
            Some(("HS-0320A".into(), TagHow::Bubble))
        );
        assert_eq!(
            read(&["FQRC", "0301"], &[]),
            Some(("FQRC-0301".into(), TagHow::Bubble))
        );
        // One piece is itself, several unshaped ones a join, none nothing.
        assert_eq!(read(&["S"], &[]), Some(("S".into(), TagHow::Single)));
        assert_eq!(
            read(&["P-0407"], &shapes),
            Some(("P-0407".into(), TagHow::Single))
        );
        assert_eq!(read(&["E", "H"], &[]), Some(("E H".into(), TagHow::Joined)));
        assert_eq!(read(&[], &shapes), None);
    }

    /// A tag's shape names its class under whichever rule table has it; an
    /// assembly goes by its first tag; a shape no rule has names nothing.
    #[test]
    fn a_tag_names_its_class_by_shape() {
        let rules = Rules::builtin();
        let class = |tag: &str| class_for_tag(tag, &rules).map(|c| c.class);
        assert_eq!(class("BV0301"), Some("ball-valve".into()));
        assert_eq!(class("GV0326A + GV0326B"), Some("gate".into()));
        assert_eq!(class("BUV-3101"), Some("butterfly".into()));
        assert_eq!(class("LA-0302"), Some("loading-arm".into()));
        assert_eq!(class("TG-0101"), Some("tank".into()));
        assert_eq!(class("接 DWG-0100FF02-05"), Some("sheet-ref".into()));
        assert_eq!(class("P-0407"), Some("pump".into()));
        assert_eq!(class("HELLO"), None);
        assert_eq!(class(""), None);
        let ball = class_for_tag("BV0301", &rules).unwrap();
        assert_eq!((ball.label.as_str(), ball.color), ("球阀", [255, 170, 0]));
    }

    /// The rules' tag shapes come from every table that has one, each once.
    #[test]
    fn the_rules_list_every_tag_shape_once() {
        let rules = Rules::builtin();
        let shapes = rules.tag_shapes();
        for shape in [
            "BV9999*", "BUV-9999", "*DWG-*", "T?-9999", "P-9999*", "LA-9999*",
        ] {
            assert_eq!(
                shapes.iter().filter(|s| **s == shape).count(),
                1,
                "{shape} in {shapes:?}"
            );
        }
        assert_eq!(rules.manual_group.class, "manual");
    }

    /// The description carries the tag as `key=value` pieces; parsing and
    /// writing round-trip, foreign text is kept, and a group without the
    /// key is not a P&ID group.
    #[test]
    fn a_group_tag_is_written_into_and_read_from_the_description() {
        assert_eq!(GroupTag::parse(""), None);
        assert_eq!(GroupTag::parse("a collection of related entities"), None);
        assert_eq!(
            GroupTag::parse("tagName=BUV-3101;tagSource=auto"),
            Some(GroupTag::auto(Some("BUV-3101".into())))
        );
        assert_eq!(
            GroupTag::parse("tagName=BUV-3101"),
            Some(GroupTag::auto(Some("BUV-3101".into()))),
            "no source reads as auto"
        );
        assert_eq!(
            GroupTag::parse(" tagName = XV-0001 ; tagSource = MANUAL "),
            Some(GroupTag::manual("XV-0001")),
            "spacing and case of the source do not matter"
        );
        assert_eq!(
            GroupTag::parse("tagName=;tagSource=auto"),
            Some(GroupTag::auto(None)),
            "marked, no tag"
        );

        // Written into an empty description, then over itself.
        let tag = GroupTag::auto(Some("BUV-3101".into()));
        let written = tag.write_into("");
        assert_eq!(written, "tagName=BUV-3101;tagSource=auto");
        assert_eq!(GroupTag::parse(&written), Some(tag.clone()));
        let manual = GroupTag::manual("BUV-3199");
        assert_eq!(
            manual.write_into(&written),
            "tagName=BUV-3199;tagSource=manual"
        );

        // Foreign text stays where it was, our keys are replaced in place.
        let foreign = "备注=阀门 A=B;tagName=OLD;color=red;tagSource=manual;tagName=DUP";
        assert_eq!(
            tag.write_into(foreign),
            "备注=阀门 A=B;tagName=BUV-3101;color=red;tagSource=auto"
        );
        assert_eq!(GroupTag::strip(foreign), "备注=阀门 A=B;color=red");
        assert_eq!(GroupTag::parse(&GroupTag::strip(foreign)), None);

        // A value is trimmed and cannot hold the separator.
        assert_eq!(
            GroupTag::manual("  BV0301;BV0302 ").name,
            Some("BV0301；BV0302".into())
        );
        assert_eq!(GroupTag::manual("   ").name, None);
        assert_eq!(
            GroupTag::auto(None).write_into(""),
            "tagName=;tagSource=auto"
        );
    }

    fn text(value: &str, x: f64, y: f64) -> EntityType {
        EntityType::Text(Text::with_value(value, Vector3::new(x, y, 0.0)).with_height(2.0))
    }

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> EntityType {
        EntityType::Line(Line::from_points(
            Vector3::new(x0, y0, 0.0),
            Vector3::new(x1, y1, 0.0),
        ))
    }

    /// `members` added to a fresh document and grouped under `name` with
    /// `description`, the way `Scene::create_group` does it; returns the
    /// document and the group's handle.
    fn grouped(name: &str, description: &str, members: Vec<EntityType>) -> (CadDocument, Handle) {
        let mut doc = CadDocument::new();
        let handles: Vec<Handle> = members
            .into_iter()
            .map(|member| doc.add_entity(member).unwrap())
            .collect();
        let dictionary = doc.header.acad_group_dict_handle;
        let mut group = Group::new(name);
        group.handle = doc.allocate_handle();
        group.owner = dictionary;
        group.add_entities(handles);
        group.description = description.to_string();
        let gh = group.handle;
        doc.objects.insert(gh, ObjectType::Group(group));
        if let Some(ObjectType::Dictionary(dict)) = doc.objects.get_mut(&dictionary) {
            dict.add_entry(name, gh);
        }
        (doc, gh)
    }

    fn group_of(doc: &CadDocument, gh: Handle) -> &Group {
        match doc.objects.get(&gh) {
            Some(ObjectType::Group(group)) => group,
            other => panic!("no group at {gh:?}: {other:?}"),
        }
    }

    /// A group's lettering reads by the same rule as a bubble's, over the
    /// pieces long enough to be a tag: a valve's strokes with `BUV-3101`
    /// beside them are `BUV-3101`; a size lettered too is not part of it; two
    /// bubble lines compose; a lone tagless piece is the tag; a range
    /// annotation, a short piece and several unshaped pieces are not.
    #[test]
    fn a_group_reads_its_tag_from_its_own_lettering() {
        let rules = Rules::builtin();
        let derive = |members: Vec<EntityType>| {
            let (doc, gh) = grouped("G", "", members);
            derive_group_tag(&doc, group_of(&doc, gh), &rules).map(|r| (r.value, r.how))
        };
        let body = || {
            vec![
                line(0.0, 0.0, 0.0, 3.0),
                line(0.0, 3.0, 6.0, 0.0),
                line(6.0, 0.0, 6.0, 3.0),
                line(6.0, 3.0, 0.0, 0.0),
            ]
        };
        let with = |extra: Vec<EntityType>| {
            let mut members = body();
            members.extend(extra);
            members
        };
        assert_eq!(
            derive(with(vec![text("BUV-3101", 0.0, 4.0)])),
            Some(("BUV-3101".into(), TagHow::Shape))
        );
        assert_eq!(
            derive(with(vec![
                text("1/2\"NPT", 0.0, -3.0),
                text("BUV-3101", 0.0, 4.0)
            ])),
            Some(("BUV-3101".into(), TagHow::Shape)),
            "the size is not the tag, wherever it is lettered"
        );
        // Two shaped pieces, an assembly in reading order: top first.
        assert_eq!(
            derive(with(vec![
                text("GV0326B", 0.0, -3.0),
                text("GV0326A", 0.0, 4.0)
            ])),
            Some(("GV0326A + GV0326B".into(), TagHow::Shape))
        );
        assert_eq!(
            derive(with(vec![text("3201", 3.0, 0.5), text("XV", 3.0, 2.0)])),
            Some(("XV-3201".into(), TagHow::Bubble))
        );
        assert_eq!(
            derive(with(vec![text("ANY-THING", 0.0, 4.0)])),
            Some(("ANY-THING".into(), TagHow::Single))
        );
        assert_eq!(
            derive(with(vec![
                text("NO", 3.0, -1.0),
                text("ANY-THING", 0.0, 4.0)
            ])),
            Some(("ANY-THING".into(), TagHow::Single)),
            "a short piece beside the one long one does not make a join"
        );
        assert_eq!(derive(body()), None, "no lettering, no tag");
        assert_eq!(derive(with(vec![text("S", 3.0, 1.5)])), None, "too short");
        assert_eq!(
            derive(with(vec![text("XV-0407A～0409A", 0.0, 4.0)])),
            None,
            "a range annotation names several symbols"
        );
        assert_eq!(
            derive(with(vec![
                text("DN100", 0.0, 4.0),
                text("1.6MPa", 0.0, -3.0)
            ])),
            None,
            "two unshaped pieces are not a tag of both"
        );
        // A range annotation beside a real tag does not get in its way.
        assert_eq!(
            derive(with(vec![
                text("XV-0407A～0409A", 0.0, 6.0),
                text("BV0301", 0.0, 4.0)
            ])),
            Some(("BV0301".into(), TagHow::Shape))
        );
    }

    /// The description survives a save and a reload in both formats, with
    /// the group's members: the tag travels with the file. (M0 of the
    /// 2026-09-10 plan: the DWG path had never been exercised.) The group is
    /// found by its marker, not its name: acadrust's DXF reader gives a GROUP
    /// object an empty name -- the name is the ACAD_GROUP dictionary's key,
    /// which the DWG builder resolves and the DXF reader does not -- so the
    /// dictionary entry is what names it after a DXF reload.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_group_description_survives_dxf_and_dwg_round_trips() {
        let description = "tagName=BUV-3101;tagSource=manual;备注=阀门 A=B";
        let (doc, _) = grouped(
            "VALVE-1",
            description,
            vec![line(0.0, 0.0, 6.0, 3.0), text("BUV-3101", 0.0, 4.0)],
        );
        for ext in ["dxf", "dwg"] {
            let bytes = crate::io::save_to_bytes(&doc, ext, doc.version)
                .unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = crate::io::load_bytes(&format!("round-trip.{ext}"), bytes)
                .unwrap_or_else(|e| panic!("{ext}: {e}"));
            let groups: Vec<&Group> = back
                .objects
                .values()
                .filter_map(|object| match object {
                    ObjectType::Group(group) => Some(group),
                    _ => None,
                })
                .collect();
            assert_eq!(groups.len(), 1, "{ext}: one group comes back");
            let group = groups[0];
            assert_eq!(group.description, description, "{ext}");
            assert_eq!(
                GroupTag::parse(&group.description),
                Some(GroupTag::manual("BUV-3101")),
                "{ext}"
            );
            assert_eq!(group.entities.len(), 2, "{ext}: both members");
            assert!(
                group.entities.iter().all(|h| back.get_entity(*h).is_some()),
                "{ext}: the members resolve"
            );
            // Named by the dictionary, whichever reader filled `name` in.
            let dictionary = back.header.acad_group_dict_handle;
            let keyed = match back.objects.get(&dictionary) {
                Some(ObjectType::Dictionary(dict)) => dict
                    .entries
                    .iter()
                    .any(|(key, handle)| key == "VALVE-1" && *handle == group.handle),
                _ => false,
            };
            assert!(
                keyed || group.name == "VALVE-1",
                "{ext}: the group keeps its name (name {:?}, dictionary entry {keyed})",
                group.name
            );
        }
    }
}
