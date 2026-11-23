//! Turning local edits into `op` payloads, and remote `op_applied` payloads
//! back into local document edits.
//!
//! The two directions are deliberately pure functions over `Document` /
//! `PidAuthor` so both can be tested without a socket and without a GUI:
//!
//! * [`collect_changes`] reads the `(handle, before, after)` triples an undo
//!   step already carries and states what changed in P&ID semantics — a tag,
//!   a property — instead of shipping geometry. The server only ever sees
//!   semantics (`architecture.md` §3).
//! * [`apply_change`] writes those semantics onto the local document, using
//!   the same `PidAuthor` store the properties panel writes through, so a
//!   remote tag edit relabels the drawing exactly like a local one.
//!
//! An object is named by its **tag** (`object_ref`), as `architecture.md` §3.2
//! writes every payload: the tag is what the server audits and adjudicates
//! by, and what a client with no drawing of its own can name. The entity
//! handle rides along as a hint and nothing more — it tells two symbols that
//! carry one tag apart, and it is believed only when the entity it names
//! carries that tag here too. Handles are not shared identity: two clients
//! that each inserted a symbol since they opened the file gave the same handle
//! to different things.
//!
//! An object with no tag — a pipe segment, a break, a connector — therefore
//! has no name on the wire, and nothing is said about it. What it carries is
//! derived from a tagged object (a segment takes its run's values), and each
//! drawing derives it for itself when the tagged change arrives
//! (`apply_remote_changes` runs the same carry-down a local edit does).

use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::collections::BTreeSet;

use crate::pid::persist::PidAuthor;
use crate::pid::propagate::{join_props, split_props};
use crate::pid::tag_engine::TAG_SOURCE_MANUAL;
use crate::pid::topology::{
    parse_handle, ROLE_FITTING, ROLE_PIPE_LINE, ROLE_PIPE_RUN, ROLE_SIGNAL_LINE,
};

/// Op kinds this client emits and understands.
pub const KIND_SET_TAG: &str = "set_tag";
pub const KIND_SET_PROPERTY: &str = "set_property";
/// Not a wire kind: this client's note to itself that the server gave a
/// symbol it placed another tag than the one it was sent under, because
/// that one was taken (`State::apply` writes it when its own `insert_symbol`
/// comes back saying `reassigned_from`). Written like a retag, to the entity
/// the symbol was placed as.
pub const KIND_REASSIGN_TAG: &str = "reassign_tag";
/// A symbol placed: the object comes into being under the tag it was given.
pub const KIND_INSERT_SYMBOL: &str = "insert_symbol";
/// An object erased.
pub const KIND_DELETE_OBJECT: &str = "delete_object";

/// What a placed symbol is, beyond its tag: enough for a peer to place the
/// same symbol in its own drawing. This is the one place geometry travels —
/// an insertion point, a rotation and a scale, which is what a symbol's
/// placement is; the symbol's own lines come from the library on each side.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The symbol's block (`PID_SYM_<category>_<id>`).
    pub block: String,
    /// The author type code (`VALVE`, `EQUIPMENT`, …).
    pub type_code: String,
    pub position: [f64; 3],
    /// Radians, as the entity stores it.
    pub rotation: f64,
    /// Uniform, as every placement is made.
    pub scale: f64,
    pub ports: String,
    /// The author record's `props` text.
    pub props: String,
    pub tag_source: String,
}

// Placements are finite numbers; `Change` is compared and kept in sets of
// its own, which is all this is for.
impl Eq for Placed {}

/// What a change naming no tag is called when it is reported as unresolved.
pub const UNTAGGED: &str = "（无位号）";

/// One change an op carries, as this module writes and reads it.
///
/// `field` is `tag` for [`KIND_SET_TAG`] and the property key for
/// [`KIND_SET_PROPERTY`], matching the server's canonical change shape
/// (`op_changes` backfills `kind`, `object_ref`, `field` and `value`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: String,
    /// The tag the object carried when the change was made. For a retag that
    /// is the old tag: the name the peers still know it by.
    pub object_ref: String,
    /// The entities the sender changed for it, for a peer holding the same
    /// file. Usually one. A pipe line or a signal line is one object drawn
    /// as several entities that all carry its tag, and a change made to
    /// several of them is still one change to one object — stated once,
    /// with every entity it was written to.
    pub handles: Vec<Handle>,
    pub field: String,
    pub value: String,
    /// The value being replaced. The server's audit line reads it
    /// (`old_value`), which is how a change says what it overwrote.
    ///
    /// For [`KIND_INSERT_SYMBOL`] it is the tag the sender had given the
    /// symbol, when the server gave it another because that one was taken
    /// (`reassigned_from`).
    pub old: Option<String>,
    /// What was placed, for [`KIND_INSERT_SYMBOL`]; `None` for every change
    /// to an object that exists.
    pub placed: Option<Placed>,
    /// What a statement about a pipe line or a run says of it — the
    /// stretches the line has, the run as the step left it
    /// (`super::pipes`); `None` for every other change. `object_ref` is
    /// then the line's tag or the run's.
    pub pipe: Option<super::pipes::Pipe>,
    /// Where the new tag of a [`KIND_SET_TAG`] came from at the sender —
    /// typed by hand, or worked out by the tag rule
    /// (`PidAuthor::tag_source`) — so that the peer's record says the same.
    /// `None` on every other change, and on a retag from a client that does
    /// not say: that one is written as typed by hand, as every retag used
    /// to be.
    pub tag_source: Option<String>,
    /// On a [`KIND_SET_PROPERTY`] of an object that carries an override
    /// mark before the step or after it: the fields of the object that are
    /// marked once the step is done (`__overrides` — a run's field changed
    /// by hand no longer takes its line's value). The peer marks the
    /// change's own field when the list names it and takes the mark off
    /// when it does not. `None` when nothing is said about marks, and the
    /// peer leaves its own as they are.
    pub overrides: Option<BTreeSet<String>>,
}

impl Change {
    /// The JSON body one change becomes inside an `op` payload.
    pub fn to_json(&self) -> Value {
        if let Some(pipe) = &self.pipe {
            return super::pipes::to_json(self, pipe);
        }
        let mut change = json!({
            "kind": self.kind,
            "object_ref": self.object_ref,
            "field": self.field,
            "value": self.value,
        });
        if let Some(placed) = &self.placed {
            // `tag` and `type` are the keys the server reads a new symbol
            // by (`architecture.md` §3.2); `props` goes as an object, so the
            // server knows the fields the symbol starts with.
            change["tag"] = json!(self.object_ref);
            change["type"] = json!(placed.type_code);
            change["block"] = json!(placed.block);
            change["position"] = json!(placed.position);
            change["rotation"] = json!(placed.rotation);
            change["scale"] = json!(placed.scale);
            change["ports"] = json!(placed.ports);
            change["tag_source"] = json!(placed.tag_source);
            let (values, _) = split_props(&placed.props);
            change["props"] = json!(values);
        } else if let Some(old) = &self.old {
            change["old_value"] = json!(old);
        }
        if let Some(source) = &self.tag_source {
            change["tag_source"] = json!(source);
        }
        if let Some(marked) = &self.overrides {
            change["overrides"] = json!(marked);
        }
        // `handle` is the first, as it has always been written; `handles`
        // appears only when there is more than one to say.
        let text = |handle: &Handle| format!("{:X}", handle.value());
        if let Some(first) = self.handles.first() {
            change["handle"] = json!(text(first));
        }
        if self.handles.len() > 1 {
            change["handles"] = json!(self.handles.iter().map(text).collect::<Vec<_>>());
        }
        change
    }

    /// Read one change back out of an `op_applied` `changes` entry. `None` for
    /// a change this client cannot act on (an unknown kind, no object named, a
    /// missing value) — applying half of it would be worse.
    pub fn from_json(value: &Value) -> Option<Self> {
        let kind = value["kind"].as_str()?.to_owned();
        if matches!(
            kind.as_str(),
            super::pipes::KIND_CREATE_PIPE_LINE
                | super::pipes::KIND_ADD_PIPE_RUN
                | super::pipes::KIND_INSERT_FITTING
                | super::pipes::KIND_UPDATE_PIPE_RUN
        ) {
            return super::pipes::from_json(&kind, value);
        }
        if matches!(
            kind.as_str(),
            super::connectors::KIND_CREATE_CONNECTOR
                | super::connectors::KIND_DELETE_CONNECTOR
                | super::connectors::KIND_PAIR_CONNECTOR
        ) {
            return super::connectors::from_json(&kind, value);
        }
        let handles: Vec<Handle> = match value["handles"].as_array() {
            Some(listed) => listed
                .iter()
                .filter_map(|entry| entry.as_str().and_then(parse_handle))
                .collect(),
            None => value["handle"]
                .as_str()
                .and_then(parse_handle)
                .into_iter()
                .collect(),
        };
        if kind == KIND_INSERT_SYMBOL {
            // The server names the symbol by the tag it has now
            // (`object_ref`), which is not the tag it was sent under when
            // that one was taken; a payload nobody has rewritten says `tag`.
            let tag = value["object_ref"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .or_else(|| value["tag"].as_str())
                .filter(|tag| !tag.is_empty())?
                .to_owned();
            let number = |entry: &Value| entry.as_f64().filter(|n| n.is_finite());
            let position = value["position"].as_array()?;
            let position = [
                number(position.first()?)?,
                number(position.get(1)?)?,
                position.get(2).and_then(number).unwrap_or(0.0),
            ];
            let props = match &value["props"] {
                Value::Object(map) => Value::Object(map.clone()).to_string(),
                Value::String(text) => text.clone(),
                _ => "{}".to_owned(),
            };
            return Some(Self {
                kind,
                object_ref: tag,
                handles,
                field: String::new(),
                value: String::new(),
                old: value["reassigned_from"].as_str().map(str::to_owned),
                placed: Some(Placed {
                    block: value["block"].as_str().filter(|block| !block.is_empty())?.to_owned(),
                    type_code: value["type"].as_str()?.to_owned(),
                    position,
                    rotation: number(&value["rotation"]).unwrap_or(0.0),
                    scale: number(&value["scale"]).filter(|scale| *scale != 0.0).unwrap_or(1.0),
                    ports: value["ports"].as_str().unwrap_or_default().to_owned(),
                    props,
                    tag_source: value["tag_source"].as_str().unwrap_or("auto").to_owned(),
                }),
                pipe: None,
                tag_source: None,
                overrides: None,
            });
        }
        if kind == KIND_DELETE_OBJECT {
            return Some(Self {
                kind,
                object_ref: value["object_ref"].as_str()?.to_owned(),
                handles,
                field: String::new(),
                value: String::new(),
                old: None,
                placed: None,
                pipe: None,
                tag_source: None,
                overrides: None,
            });
        }
        if kind != KIND_SET_TAG && kind != KIND_SET_PROPERTY && kind != KIND_REASSIGN_TAG {
            return None;
        }
        let object_ref = value["object_ref"].as_str()?.to_owned();
        let field = match value["field"].as_str() {
            Some(field) => field.to_owned(),
            // `architecture.md` writes a retag without the field.
            None if kind != KIND_SET_PROPERTY => "tag".to_owned(),
            None => return None,
        };
        let value_text = match &value["value"] {
            Value::String(text) => text.clone(),
            Value::Null => return None,
            other => other.to_string(),
        };
        // What rides with the value: each on the one kind it belongs to.
        let tag_source = value["tag_source"]
            .as_str()
            .filter(|source| kind == KIND_SET_TAG && !source.is_empty())
            .map(str::to_owned);
        let overrides = value["overrides"]
            .as_array()
            .filter(|_| kind == KIND_SET_PROPERTY)
            .map(|listed| {
                listed
                    .iter()
                    .filter_map(|field| field.as_str().map(str::to_owned))
                    .collect()
            });
        Some(Self {
            kind,
            object_ref,
            handles,
            field,
            value: value_text,
            old: value["old_value"].as_str().map(str::to_owned),
            placed: None,
            pipe: None,
            tag_source,
            overrides,
        })
    }

    /// Whether `other` says the same thing of the same object: everything
    /// but the entities it was written to.
    fn states_the_same_as(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.object_ref == other.object_ref
            && self.field == other.field
            && self.value == other.value
            && self.old == other.old
            && self.placed == other.placed
            && self.pipe == other.pipe
            && self.tag_source == other.tag_source
            && self.overrides == other.overrides
    }

    /// Whether this brings an object into the drawing or takes one out —
    /// or says what a pipe line or a run is made of — rather than changing
    /// a field of one that is there.
    pub fn is_structural(&self) -> bool {
        self.kind == KIND_INSERT_SYMBOL || self.kind == KIND_DELETE_OBJECT || self.pipe.is_some()
    }

    /// The tags this is about: the object it names, the tag a retag gives
    /// it, and for a run the line it is on and the fittings on it that
    /// have a tag of their own.
    pub fn names(&self) -> Vec<String> {
        let mut names = vec![self.object_ref.clone()];
        if self.kind == KIND_SET_TAG || self.kind == KIND_REASSIGN_TAG {
            names.push(self.value.clone());
        }
        if let Some(super::pipes::Pipe::Run(said)) = &self.pipe {
            names.push(said.line_tag.clone());
            names.extend(said.fitting.iter().filter_map(|(_, _, tag)| tag.clone()));
            names.extend(
                said.run
                    .fittings
                    .iter()
                    .filter_map(|fitting| fitting.tag.clone()),
            );
        }
        names.retain(|name| !name.is_empty());
        names
    }

    /// The tags a later step is built on when it names one of them
    /// (`SPEC-collab-offline-sync.md` §5.4): what this made, renamed or
    /// erased, the line or run it is about. A field it wrote is none: a
    /// later write to the same object stands on its own.
    pub fn built_names(&self) -> Vec<String> {
        if self.kind == KIND_SET_PROPERTY {
            return Vec::new();
        }
        self.names()
    }
}

/// Whether `doc` is still as `change` left it (`SPEC-collab-offline-sync.md`
/// §5.4): the field holds the value written, something carries the tag a
/// retag gave, the symbol placed is there, nothing carries the tag of the
/// object erased, a line or run is as stated, the connector made is there
/// and the one taken away is not. A refused step's inverse is written only
/// where it holds: what somebody else did on top of it since stays.
pub fn still_as_written(doc: &Document, change: &Change) -> bool {
    let carries = |tag: &str| {
        doc.entities()
            .any(|entity| PidAuthor::from_entity(entity).is_some_and(|author| author.tag == tag))
    };
    match &change.pipe {
        Some(super::pipes::Pipe::Connector(said)) => {
            return super::connectors::as_said(doc, change, said);
        }
        Some(_) => return super::pipes::as_said(doc, change),
        None => {}
    }
    match change.kind.as_str() {
        KIND_SET_PROPERTY => {
            let targets = resolve(doc, change);
            !targets.is_empty()
                && targets.iter().all(|handle| {
                    PidAuthor::load(doc, *handle).is_some_and(|author| {
                        let (values, _) = split_props(&author.props);
                        values
                            .get(&change.field)
                            .map(String::as_str)
                            .unwrap_or_default()
                            == change.value
                    })
                })
        }
        KIND_SET_TAG | KIND_REASSIGN_TAG => carries(&change.value),
        KIND_INSERT_SYMBOL => !resolve(doc, change).is_empty(),
        KIND_DELETE_OBJECT => !carries(&change.object_ref),
        _ => false,
    }
}

/// Whether `entity` is a symbol standing on its own: an `Insert` carrying an
/// author record with a tag, that is not part of a pipe line (a fitting on a
/// run belongs to the run; the pipe's own pieces are not `Insert`s).
///
/// Those are the objects whose coming and going is said entity by entity.
/// A pipe line, its runs and what is fitted on them are made of several
/// entities and a table between them; what a step did to those is said by
/// `super::pipes::statements`, from the drawing as the step left it
/// ([`collect_step_changes`]).
fn standalone_symbol(entity: &EntityType) -> Option<(&document::entities::Insert, PidAuthor)> {
    let EntityType::Insert(insert) = entity else {
        return None;
    };
    let author = PidAuthor::from_entity(entity)?;
    (!author.tag.is_empty() && author.owner.is_empty() && author.topology.is_empty())
        .then_some((insert, author))
}

/// A symbol that was not there before the step and is after it.
fn placed_symbol(handle: Handle, entity: &EntityType) -> Option<Change> {
    let (insert, author) = standalone_symbol(entity)?;
    Some(Change {
        kind: KIND_INSERT_SYMBOL.to_owned(),
        object_ref: author.tag.clone(),
        handles: vec![handle],
        field: String::new(),
        value: String::new(),
        old: None,
        placed: Some(Placed {
            block: insert.block_name.clone(),
            type_code: author.type_code,
            position: [
                insert.insert_point.x,
                insert.insert_point.y,
                insert.insert_point.z,
            ],
            rotation: insert.rotation,
            scale: insert.x_scale(),
            ports: author.ports,
            // Properties that overflowed into an XRecord are not in the
            // image: read off it alone, the symbol is said to start with
            // none rather than not said at all. A step's collector has the
            // drawing and puts them in (`fill_overflowed_props`).
            props: if PidAuthor::props_are_inline(entity) {
                author.props
            } else {
                "{}".to_owned()
            },
            tag_source: author.tag_source,
        }),
        pipe: None,
        tag_source: None,
        overrides: None,
    })
}

/// A symbol that was there before the step and is not after it.
fn erased_symbol(handle: Handle, entity: &EntityType) -> Option<Change> {
    let (_, author) = standalone_symbol(entity)?;
    Some(Change {
        kind: KIND_DELETE_OBJECT.to_owned(),
        object_ref: author.tag,
        handles: vec![handle],
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: None,
        tag_source: None,
        overrides: None,
    })
}

/// What one handle's before/after images amount to in P&ID semantics: a
/// retag, if the tag moved, then every property whose value moved — or
/// whose override mark did, which goes with the value (`Change::overrides`).
///
/// The retag comes first and names the object by the tag it had; the property
/// changes after it name it by the tag it has now, so a peer applying them in
/// order finds it each time.
///
/// A symbol that came into being or went out of it is said as that — where
/// it stands and what it is, or that it is gone (`placed_symbol`,
/// `erased_symbol`). Any other created or erased entity yields nothing
/// rather than a half-truth: the geometry of a pipe is not in this payload.
/// An image whose `props` overflowed into an XRecord is "not known here",
/// not "no properties": read off the images alone, the properties of such a
/// change are left out rather than broadcast as removals. A step's collector
/// knows where they are kept and reads them there ([`collect_step_changes`]).
pub fn changes_of(
    handle: Handle,
    before: Option<&EntityType>,
    after: Option<&EntityType>,
) -> Vec<Change> {
    changes_between(handle, before, after, None)
}

/// Where the properties are that an image of the side a step left only
/// points at. Properties too long for an entity's own record are kept in a
/// record of the drawing (`PidAuthor::store`); no image carries them, and the
/// drawing has gone on from that side.
pub enum Left<'a> {
    /// In the drawing's records as they were on that side: what the step
    /// keeps beside its images (`DeltaSnapshot::records_across`).
    Records(&'a Document),
    /// Read off the drawing while it was on that side, for each entity whose
    /// record only pointed at them.
    Read(&'a [(Handle, String)]),
}

impl Left<'_> {
    /// The properties `image` only points at, `image` being the entity
    /// `handle` on the side left. `None` where they are not known.
    fn props(&self, handle: Handle, image: &EntityType) -> Option<String> {
        match self {
            Self::Records(records) => PidAuthor::overflowed_props(records, image),
            Self::Read(read) => read
                .iter()
                .find(|(of, _)| *of == handle)
                .map(|(_, props)| props.clone()),
        }
    }
}

/// Where the properties of a step's two sides are kept, for the images that
/// only point at them: the drawing as it is for the side it is on, and
/// [`Left`] for the other.
struct Kept<'a> {
    now: &'a Document,
    left: &'a Left<'a>,
}

/// [`changes_of`], with the properties an image only points at read where
/// `kept` says they are. A side whose properties are known in neither place
/// is not a side without any: nothing is said of the properties then.
fn changes_between(
    handle: Handle,
    before: Option<&EntityType>,
    after: Option<&EntityType>,
    kept: Option<&Kept<'_>>,
) -> Vec<Change> {
    let (before, after) = match (before, after) {
        (Some(before), Some(after)) => (before, after),
        (None, Some(after)) => return placed_symbol(handle, after).into_iter().collect(),
        (Some(before), None) => return erased_symbol(handle, before).into_iter().collect(),
        (None, None) => return Vec::new(),
    };
    let (Some(before_author), Some(after_author)) = (
        PidAuthor::from_entity(before),
        PidAuthor::from_entity(after),
    ) else {
        return Vec::new();
    };
    // An object the peers know by no tag cannot be named to them at all (see
    // `resolve`), so nothing is stated about it — not even the tag it is
    // being given, which they would have nothing to attach to.
    if before_author.tag.is_empty() {
        return Vec::new();
    }

    let mut changes = Vec::new();
    if before_author.tag != after_author.tag {
        changes.push(Change {
            kind: KIND_SET_TAG.to_owned(),
            object_ref: before_author.tag.clone(),
            handles: vec![handle],
            field: "tag".to_owned(),
            value: after_author.tag.clone(),
            old: Some(before_author.tag.clone()),
            placed: None,
            pipe: None,
            tag_source: Some(after_author.tag_source.clone()).filter(|source| !source.is_empty()),
            overrides: None,
        });
    }

    // Retagged to nothing: from here on it has no name either.
    if after_author.tag.is_empty() {
        return changes;
    }
    // The properties of each side: in its image, or where the image points.
    let before_text = if PidAuthor::props_are_inline(before) {
        Some(before_author.props)
    } else {
        kept.and_then(|kept| kept.left.props(handle, before))
    };
    let after_text = if PidAuthor::props_are_inline(after) {
        Some(after_author.props)
    } else {
        kept.and_then(|kept| PidAuthor::overflowed_props(kept.now, after))
    };
    let (Some(before_text), Some(after_text)) = (before_text, after_text) else {
        return changes;
    };
    let (before_props, before_marks) = split_props(&before_text);
    let (after_props, after_marks) = split_props(&after_text);
    // The marks as the step left them go with every property change of an
    // object that carries one, before the step or after it; a field whose
    // mark came or went is a change of that field, whatever its value.
    let overrides =
        (!before_marks.is_empty() || !after_marks.is_empty()).then(|| after_marks.clone());
    let mark_moved = |field: &str| before_marks.contains(field) != after_marks.contains(field);
    let property = |field: &str, value: &str, old: Option<&String>| Change {
        kind: KIND_SET_PROPERTY.to_owned(),
        object_ref: after_author.tag.clone(),
        handles: vec![handle],
        field: field.to_owned(),
        value: value.to_owned(),
        old: old.cloned(),
        placed: None,
        pipe: None,
        tag_source: None,
        overrides: overrides.clone(),
    };
    for (key, value) in &after_props {
        if before_props.get(key) != Some(value) || mark_moved(key) {
            changes.push(property(key, value, before_props.get(key)));
        }
    }
    // A property that is gone travels as an empty value.
    for (key, old) in &before_props {
        if !after_props.contains_key(key) {
            changes.push(property(key, "", Some(old)));
        }
    }
    // And so does a mark that moved on a field with no value either side.
    for key in before_marks.symmetric_difference(&after_marks) {
        if !before_props.contains_key(key) && !after_props.contains_key(key) {
            changes.push(property(key, "", None));
        }
    }
    changes
}

/// The `(handle, before, after)` triples one undo delta carries, as the
/// history module stores them.
pub type EntityImages = [(
    Handle,
    Option<std::sync::Arc<EntityType>>,
    Option<std::sync::Arc<EntityType>>,
)];

/// Every semantic change one undo step contains, in handle order.
pub fn collect_changes(entities: &EntityImages) -> Vec<Change> {
    stated_once(entities.iter().flat_map(|(handle, before, after)| {
        changes_of(*handle, before.as_deref(), after.as_deref())
    }))
}

/// Everything one step says to the room: what [`collect_changes`] reads
/// off its images, then what it did to pipe lines and runs — which the
/// images only point at, and `doc`, the drawing as the step left it, says
/// (`super::pipes::statements`). The same drawing gives a symbol the step
/// placed the properties its image only points at
/// (`fill_overflowed_props`), and an object the step changed the ones it has
/// now; the ones it had are where `left` says.
pub fn collect_step_changes(
    doc: &Document,
    entities: &EntityImages,
    left: &Left<'_>,
) -> Vec<Change> {
    let images: Vec<super::pipes::Image<'_>> = entities
        .iter()
        .map(|(handle, before, after)| (*handle, before.as_deref(), after.as_deref()))
        .collect();
    step_changes(doc, &images, left)
}

/// [`collect_step_changes`] for a step that has just been taken back:
/// its images read the other way, `doc` the drawing without the step, and
/// `left` the side with it.
pub fn collect_undone_step_changes(
    doc: &Document,
    entities: &EntityImages,
    left: &Left<'_>,
) -> Vec<Change> {
    let images: Vec<super::pipes::Image<'_>> = entities
        .iter()
        .map(|(handle, before, after)| (*handle, after.as_deref(), before.as_deref()))
        .collect();
    step_changes(doc, &images, left)
}

/// What a step says, its images read from the side it left to the side
/// `doc` is on.
fn step_changes(
    doc: &Document,
    images: &[super::pipes::Image<'_>],
    left: &Left<'_>,
) -> Vec<Change> {
    let kept = Kept { now: doc, left };
    let mut changes =
        stated_once(images.iter().flat_map(|&(handle, before, after)| {
            changes_between(handle, before, after, Some(&kept))
        }));
    fill_overflowed_props(doc, &mut changes);
    changes.extend(super::pipes::statements(doc, images));
    changes.extend(super::connectors::statements(doc, images));
    changes
}

/// Gives each symbol `changes` places the properties `doc` holds for it,
/// where the image the placement was read off only points at them:
/// properties too long for an entity's own record are kept in an XRecord of
/// the drawing (`PidAuthor::store`), which no image carries. `doc` is the
/// drawing with the symbol in it — as the step left it, or as taking the
/// step back did.
///
/// A symbol the drawing holds no such properties for stays as its image said
/// it: placed, with none.
fn fill_overflowed_props(doc: &Document, changes: &mut [Change]) {
    for change in changes {
        if change.kind != KIND_INSERT_SYMBOL {
            continue;
        }
        let Some(&handle) = change.handles.first() else {
            continue;
        };
        let Some(placed) = change.placed.as_mut() else {
            continue;
        };
        let overflowed = doc
            .get_entity(handle)
            .is_some_and(|entity| !PidAuthor::props_are_inline(entity));
        if !overflowed {
            continue;
        }
        let props = PidAuthor::load(doc, handle).map(|author| author.props);
        if let Some(props) = props.filter(|props| !props.is_empty()) {
            placed.props = props;
        }
    }
}

/// One object, one statement. A pipe line renamed is every one of its
/// entities retagged, which read entity by entity is the same change said
/// once per entity; the room — and the audit log — should hear that the line
/// was renamed, once, with the entities it was written to riding along.
/// Order is kept: the first time a thing is said is where it stays.
fn stated_once(changes: impl Iterator<Item = Change>) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for change in changes {
        match out.iter_mut().find(|said| said.states_the_same_as(&change)) {
            Some(said) => {
                for handle in change.handles {
                    if !said.handles.contains(&handle) {
                        said.handles.push(handle);
                    }
                }
            }
            None => out.push(change),
        }
    }
    out
}

/// The `op` payload for a batch of changes: the first change is the op's own
/// body, so a single-change op reads exactly like the server's canonical shape
/// (`{kind, object_ref, field, value}`). Additional changes ride along in
/// `changes` for the peer to apply in order.
///
/// One step is one op on purpose. The server accepts or refuses an op as a
/// whole (`STALE_BASE_REVISION`), so a step split over several ops could land
/// in part — a retag refused and the property change behind it accepted.
pub fn payload_for(changes: &[Change]) -> Option<Value> {
    let (first, rest) = changes.split_first()?;
    let mut payload = first.to_json();
    if !rest.is_empty() {
        payload["changes"] = Value::Array(rest.iter().map(Change::to_json).collect());
    }
    Some(payload)
}

/// What an `op_applied` `changes` array says, as far as this client can
/// read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// The changes it can act on, in order.
    pub changes: Vec<Change>,
    /// The ones it cannot, each by its `kind` (or `?` when it states none).
    /// They took effect in the room all the same, so a drawing that skips
    /// them is short of it — which is for the caller to say, not to pass
    /// over because the rest of the op could be applied.
    pub unread: Vec<String>,
}

/// Every change an `op_applied` `changes` array carries, including the ones a
/// batched payload nested under `changes`.
pub fn changes_from_payload(changes: &[Value]) -> Read {
    let mut read = Read::default();
    let mut take = |entry: &Value| match Change::from_json(entry) {
        Some(change) => read.changes.push(change),
        None => read
            .unread
            .push(entry["kind"].as_str().unwrap_or("?").to_owned()),
    };
    for entry in changes {
        take(entry);
        if let Some(nested) = entry["changes"].as_array() {
            nested.iter().for_each(&mut take);
        }
    }
    read
}

/// The tags a `hello` lists (`existing_tags`): every tag an object of
/// `document` carried when the room last heard from this drawing, each once
/// and in order. `unanswered` are the payloads of the steps the room has not
/// answered yet, oldest first.
///
/// The server takes the listing to be of the drawing as it was at the
/// revision the `hello` names, which is the drawing without those steps —
/// they go out after the `welcome`, and say for themselves what they did.
/// So, for each entity such a step names, what the first of them says of it
/// stands in for what it carries now:
///
/// * placed by it (`insert_symbol`) — not listed. The room hears of the
///   symbol from the step; listed here, its tag would be taken by the time
///   the step arrives, and the server would give this client's own symbol
///   another;
/// * retagged or erased by it (`set_tag`, `delete_object`) — listed under
///   the tag the step names it by, the one the room knows. To the room the
///   object is there under that tag until the step arrives; were the tag
///   free meanwhile, a symbol a peer placed under it would be what the step
///   then renamed or erased.
///
/// A change that names no entity is not guessed at. What a step said of a
/// pipe line or a run changes nothing here either: their tags are listed as
/// the drawing carries them. The server gives a line, a run or a fitting no
/// other tag than the one it is sent under, so one listed ahead of its step
/// costs that step nothing, and is kept from a peer's symbol meanwhile.
pub fn existing_tags<'a>(
    document: &Document,
    unanswered: impl IntoIterator<Item = &'a Value>,
) -> Vec<String> {
    // Each entity an unanswered step names, with the tag the room knows it
    // by: none, for one the room has not heard of.
    let mut known_as: std::collections::HashMap<Handle, Option<String>> =
        std::collections::HashMap::new();
    for payload in unanswered {
        for change in changes_from_payload(std::slice::from_ref(payload)).changes {
            let tag = match change.kind.as_str() {
                KIND_INSERT_SYMBOL => None,
                KIND_SET_TAG | KIND_DELETE_OBJECT => Some(change.object_ref),
                _ => continue,
            };
            for handle in change.handles {
                known_as.entry(handle).or_insert_with(|| tag.clone());
            }
        }
    }
    let mut tags: std::collections::BTreeSet<String> = document
        .entities()
        .filter(|entity| !known_as.contains_key(&entity.common().handle))
        .filter_map(PidAuthor::from_entity)
        .map(|author| author.tag)
        .collect();
    tags.extend(known_as.into_values().flatten());
    tags.remove("");
    tags.into_iter().collect()
}

/// What a change list did to the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Handles whose stored record was rewritten.
    pub handles: Vec<Handle>,
    /// Handles whose tag changed, so their labels and uniqueness status need
    /// re-deriving.
    pub retagged: Vec<Handle>,
    /// Changes whose object this drawing does not have, by the name they used
    /// for it. Nothing was written for them, and the two drawings now differ.
    pub unresolved: Vec<String>,
    /// Every change that found its object, with the entities it was found to
    /// mean — whether or not a value had to be written. The caller keeps
    /// these: taking back a local step restores entity images from before
    /// the change, and the change is then written to the same entities
    /// again ([`write_to`]).
    pub resolved: Vec<(Change, Vec<Handle>)>,
    /// Entities that were erased because a peer erased their object. They
    /// are not in `handles`: nothing is left of them to re-derive from.
    pub erased: Vec<Handle>,
}

impl Applied {
    /// Nothing was written.
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty() && self.erased.is_empty()
    }
}

/// The entities a change is to be written to — empty when this drawing has
/// no such object, or cannot tell which of several is meant.
///
/// The tag decides. The entities the sender named are taken when they carry
/// the tag the change names here too: a peer holding the same file then
/// writes exactly what the sender wrote, and of several symbols carrying one
/// tag the handle says which is meant.
///
/// Without that — a sender with another file, or with no drawing at all —
/// the tag has to name one object:
///
/// * one entity carrying it is that object;
/// * several entities carrying it are one object when they are the pieces
///   of a line. A pipe line or a signal line is drawn as one entity per
///   stretch, each carrying the line's tag (`PIDPIPE`), so the change goes
///   to all of them — among them the one the topology table reads the
///   line's values from, which is what carries them down to its runs;
/// * several separate symbols carrying one tag are a duplicate, and name
///   nothing.
///
/// A change naming no tag names nothing. Pipe segments, breaks and connectors
/// carry no tag, and "the untagged thing under handle H" is a different thing
/// in every drawing that has had something drawn in it since it was opened.
/// The comparison is the stored text exactly — case and surrounding blanks
/// included — which is how the tag engine's own uniqueness check compares,
/// and what `canonical_tag` turns a typed tag into before it is claimed.
pub fn resolve(doc: &Document, change: &Change) -> Vec<Handle> {
    if change.object_ref.is_empty() {
        return Vec::new();
    }
    let carries = |handle: &Handle| {
        PidAuthor::load(doc, *handle).is_some_and(|author| author.tag == change.object_ref)
    };
    // All of them or none: a sender whose entities only partly match is not
    // holding this file, and its handles say nothing about this drawing.
    if !change.handles.is_empty() && change.handles.iter().all(carries) {
        return change.handles.clone();
    }
    let carrying: Vec<(Handle, PidAuthor)> = doc
        .entities()
        .filter_map(|entity| {
            let handle = entity.common().handle;
            let author = PidAuthor::load(doc, handle)?;
            (author.tag == change.object_ref).then_some((handle, author))
        })
        .collect();
    let one_line = carrying.len() > 1
        && carrying.iter().all(|(_, author)| {
            author.type_code == carrying[0].1.type_code
                && author.topology == carrying[0].1.topology
                && matches!(
                    author.topology.as_str(),
                    ROLE_PIPE_LINE | ROLE_SIGNAL_LINE
                )
        });
    if carrying.len() == 1 || one_line {
        return carrying.into_iter().map(|(handle, _)| handle).collect();
    }
    // A run and the fittings on it that have no tag of their own: those
    // show the run's tag without being the run, and the tag means the run.
    let mut runs = carrying
        .iter()
        .filter(|(_, author)| author.topology == ROLE_PIPE_RUN);
    match (runs.next(), runs.next()) {
        (Some((run, _)), None)
            if carrying.iter().all(|(_, author)| {
                matches!(author.topology.as_str(), ROLE_PIPE_RUN | ROLE_FITTING)
            }) =>
        {
            vec![*run]
        }
        _ => Vec::new(),
    }
}

/// Whether a retag is in place here already: the entities the sender wrote
/// it to carry the new tag. That is certain only of a peer holding the same
/// file, and only then is the change passed over without a word.
///
/// It used to be enough that nothing carried the old tag and *something*
/// carried the new one. That also holds when this drawing never had the
/// object and has another under the new tag — a rename that did not happen
/// here, reported as done.
fn already_retagged(doc: &Document, change: &Change) -> bool {
    (change.kind == KIND_SET_TAG || change.kind == KIND_REASSIGN_TAG)
        && !change.handles.is_empty()
        && change.handles.iter().all(|handle| {
            PidAuthor::load(doc, *handle).is_some_and(|author| author.tag == change.value)
        })
}

/// Write one change onto `doc`.
///
/// A change whose value is already in place writes nothing and is not an
/// error: re-applying an already-applied change must not fail. A change
/// naming an object this drawing does not have is recorded in
/// `applied.unresolved`.
pub fn apply_change(doc: &mut Document, change: &Change, applied: &mut Applied) {
    let targets = resolve(doc, change);
    if targets.is_empty() {
        if change.object_ref.is_empty() {
            applied.unresolved.push(UNTAGGED.to_owned());
        } else if !already_retagged(doc, change) {
            applied.unresolved.push(change.object_ref.clone());
        }
        return;
    }
    write_to(doc, &targets, change, applied);
    applied.resolved.push((change.clone(), targets));
}

/// Write `change` to the entities `targets`, whatever tag they carry now.
///
/// This is the second half of [`apply_change`], and what applies a peer's
/// change a second time after a local undo or redo put back entity images
/// from before it: the entities are the ones it was resolved to when it
/// arrived, and by then the object may be under another name here (the undo
/// took back a retag). An entity that is gone, or has no record, is skipped.
pub fn write_to(doc: &mut Document, targets: &[Handle], change: &Change, applied: &mut Applied) {
    for &handle in targets {
        let Some(mut author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        match change.kind.as_str() {
            KIND_SET_TAG => {
                if author.tag == change.value {
                    continue;
                }
                author.tag = change.value.clone();
                // Where the sender says the tag came from. One that does
                // not say (an older client) is taken to have typed it.
                author.tag_source = change
                    .tag_source
                    .clone()
                    .unwrap_or_else(|| TAG_SOURCE_MANUAL.to_owned());
                author.store(doc, handle);
                applied.retagged.push(handle);
            }
            // The server's number in place of the one this drawing worked
            // out: still a tag nobody typed, so its source stays what it was.
            KIND_REASSIGN_TAG => {
                if author.tag == change.value {
                    continue;
                }
                author.tag = change.value.clone();
                author.store(doc, handle);
                applied.retagged.push(handle);
            }
            KIND_SET_PROPERTY => {
                let (mut values, mut marks) = split_props(&author.props);
                let current = values.get(&change.field).cloned().unwrap_or_default();
                // Whether the sender holds this field marked, when it says
                // what it holds marked. The other fields' marks are not
                // this change's to move: each is said with its own field.
                let marked = change
                    .overrides
                    .as_ref()
                    .map(|listed| listed.contains(&change.field));
                let mark_moves =
                    marked.is_some_and(|marked| marked != marks.contains(&change.field));
                if current == change.value && !mark_moves {
                    continue;
                }
                if change.value.is_empty() {
                    values.remove(&change.field);
                } else {
                    values.insert(change.field.clone(), change.value.clone());
                }
                match marked {
                    Some(true) => {
                        marks.insert(change.field.clone());
                    }
                    Some(false) => {
                        marks.remove(&change.field);
                    }
                    None => {}
                }
                author.props = join_props(&values, &marks);
                author.store(doc, handle);
            }
            _ => continue,
        }
        if !applied.handles.contains(&handle) {
            applied.handles.push(handle);
        }
    }
}

/// Write a whole change list onto `doc`, in order. For the tests of this
/// module: the app goes change by change (`write_remote_changes`), since a
/// symbol placed or an object erased is its business, not the document's.
#[cfg(test)]
pub fn apply_changes(doc: &mut Document, changes: &[Change]) -> Applied {
    let mut applied = Applied::default();
    for change in changes {
        apply_change(doc, change, &mut applied);
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::persist::PidAuthor;
    use document::entities::Insert;
    use document::types::Vector3;

    /// A symbol that was not in the drawing before a step and is after it
    /// is said as placed — what, where, under which tag — and one that was
    /// and is not, as erased. What a peer needs to place it survives the
    /// wire. A fitting on a run, and a symbol with no tag, are not said.
    #[test]
    fn a_symbol_that_came_or_went_is_said_as_that() {
        let mut doc = Document::new();
        let handle = add_author(&mut doc, "V-1001", r#"{"公称直径":"DN50"}"#);
        let entity = doc.get_entity(handle).cloned().expect("entity");

        let placed = changes_of(handle, None, Some(&entity));
        assert_eq!(placed.len(), 1, "{placed:?}");
        assert_eq!(placed[0].kind, KIND_INSERT_SYMBOL);
        assert_eq!(placed[0].object_ref, "V-1001");
        assert_eq!(placed[0].handles, [handle]);
        let what = placed[0].placed.clone().expect("what was placed");
        assert_eq!(what.block, "PID_SYM_equipment_vessel");
        assert_eq!(what.type_code, "EQUIPMENT");
        assert_eq!(what.position, [0.0, 0.0, 0.0]);

        let wire = placed[0].to_json();
        assert_eq!(wire["tag"], "V-1001");
        assert_eq!(wire["type"], "EQUIPMENT");
        assert_eq!(wire["props"]["公称直径"], "DN50");
        let back = Change::from_json(&wire).expect("readable");
        assert_eq!(back.object_ref, "V-1001");
        assert_eq!(back.handles, [handle]);
        let read = back.placed.expect("what was placed");
        assert_eq!((read.block, read.type_code, read.position), (what.block, what.type_code, what.position));
        assert_eq!(split_props(&read.props).0.get("公称直径").map(String::as_str), Some("DN50"));

        // As the server sends one it took under another tag.
        let mut renumbered = wire.clone();
        renumbered["object_ref"] = json!("V-1002");
        renumbered["tag"] = json!("V-1002");
        renumbered["reassigned_from"] = json!("V-1001");
        let back = Change::from_json(&renumbered).expect("readable");
        assert_eq!((back.object_ref.as_str(), back.old.as_deref()), ("V-1002", Some("V-1001")));

        let erased = changes_of(handle, Some(&entity), None);
        assert_eq!(erased.len(), 1, "{erased:?}");
        assert_eq!((erased[0].kind.as_str(), erased[0].object_ref.as_str()), (KIND_DELETE_OBJECT, "V-1001"));
        assert!(Change::from_json(&erased[0].to_json()).is_some_and(|change| change.is_structural()));

        // Part of a pipe run: the run's business, not a symbol of its own.
        let fitting = add_author(&mut doc, "V-2001", "{}");
        let mut record = PidAuthor::load(&doc, fitting).expect("author");
        record.owner = "1F".to_owned();
        record.store(&mut doc, fitting);
        let entity = doc.get_entity(fitting).cloned().expect("entity");
        assert!(changes_of(fitting, None, Some(&entity)).is_empty());
        // No tag: nothing a peer could call it.
        let nameless = add_author(&mut doc, "", "{}");
        let entity = doc.get_entity(nameless).cloned().expect("entity");
        assert!(changes_of(nameless, None, Some(&entity)).is_empty());
    }

    fn add_author(doc: &mut Document, tag: &str, props: &str) -> Handle {
        let handle = doc
            .add_entity(EntityType::Insert(Insert::new(
                "PID_SYM_equipment_vessel",
                Vector3::ZERO,
            )))
            .expect("entity");
        PidAuthor {
            type_code: "EQUIPMENT".to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            props: props.to_owned(),
            ..PidAuthor::default()
        }
        .store(doc, handle);
        handle
    }

    fn doc_with_author(tag: &str, props: &str) -> (Document, Handle) {
        let mut doc = Document::new();
        let handle = add_author(&mut doc, tag, props);
        (doc, handle)
    }

    /// A change as a peer with the same file would send it.
    fn change(
        kind: &str,
        object_ref: &str,
        handle: Option<Handle>,
        field: &str,
        value: &str,
    ) -> Change {
        Change {
            kind: kind.to_owned(),
            object_ref: object_ref.to_owned(),
            handles: handle.into_iter().collect(),
            field: field.to_owned(),
            value: value.to_owned(),
            old: None,
            placed: None,
            pipe: None,
            tag_source: None,
            overrides: None,
        }
    }

    fn tag_of(doc: &Document, handle: Handle) -> String {
        PidAuthor::load(doc, handle).expect("author").tag
    }

    #[test]
    fn a_tag_edit_becomes_one_set_tag_change_naming_the_old_tag() {
        let (mut doc, handle) = doc_with_author("P-1001", "");
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.tag = "P-1009".to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = changes_of(handle, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        let change = &changes[0];
        assert_eq!(change.kind, KIND_SET_TAG);
        assert_eq!(change.object_ref, "P-1001");
        assert_eq!(change.handles, [handle]);
        assert_eq!(change.field, "tag");
        assert_eq!(change.value, "P-1009");
        assert_eq!(change.old.as_deref(), Some("P-1001"));
    }

    #[test]
    fn a_property_edit_becomes_one_set_property_change_naming_the_tag() {
        let (mut doc, handle) = doc_with_author("P-1001", r#"{"设计压力":"1.6MPa"}"#);
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.props = r#"{"设计压力":"2.5MPa"}"#.to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = changes_of(handle, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        let change = &changes[0];
        assert_eq!(change.kind, KIND_SET_PROPERTY);
        assert_eq!(change.object_ref, "P-1001");
        assert_eq!(change.field, "设计压力");
        assert_eq!(change.value, "2.5MPa");
        assert_eq!(change.old.as_deref(), Some("1.6MPa"));
    }

    /// One step can move the tag and several properties of one symbol. Every
    /// one of them is stated: the retag first, by the old tag; the properties
    /// after it, by the new one; a property that is gone as an empty value.
    #[test]
    fn a_step_that_moves_the_tag_and_properties_states_all_of_them_in_order() {
        let (mut doc, handle) = doc_with_author(
            "P-1001",
            r#"{"设计压力":"1.6MPa","介质":"水","设计温度":"80℃"}"#,
        );
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.tag = "P-1009".to_owned();
        author.props = r#"{"设计压力":"2.5MPa","设计温度":"80℃","公称直径":"DN50"}"#.to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = changes_of(handle, Some(&before), Some(&after));
        let stated: Vec<(&str, &str, &str, &str)> = changes
            .iter()
            .map(|change| {
                (
                    change.kind.as_str(),
                    change.object_ref.as_str(),
                    change.field.as_str(),
                    change.value.as_str(),
                )
            })
            .collect();
        assert_eq!(
            stated,
            [
                (KIND_SET_TAG, "P-1001", "tag", "P-1009"),
                (KIND_SET_PROPERTY, "P-1009", "公称直径", "DN50"),
                (KIND_SET_PROPERTY, "P-1009", "设计压力", "2.5MPa"),
                (KIND_SET_PROPERTY, "P-1009", "介质", ""),
            ]
        );
        assert_eq!(changes[3].old.as_deref(), Some("水"));
    }

    #[test]
    fn an_unchanged_entity_yields_no_change() {
        let (doc, handle) = doc_with_author("P-1001", r#"{"设计压力":"1.6MPa"}"#);
        let image = doc.get_entity_arc(handle).expect("image");
        assert!(changes_of(handle, Some(&image), Some(&image)).is_empty());
    }

    /// A created or erased entity the room has no name for is skipped: there
    /// is nothing a peer could call it. (A symbol with a tag is said — see
    /// `a_symbol_that_came_or_went_is_said_as_that`; this used to be the
    /// rule for every created or erased entity.)
    #[test]
    fn a_created_or_erased_entity_with_no_tag_is_skipped() {
        let (doc, handle) = doc_with_author("", "");
        let image = doc.get_entity_arc(handle).expect("image");
        assert!(changes_of(handle, None, Some(&image)).is_empty());
        assert!(changes_of(handle, Some(&image), None).is_empty());
    }

    /// A record whose `props` overflowed into an XRecord reads back from a
    /// snapshot with an empty `props`. That is "not known here", not "every
    /// property removed", and no removal may be broadcast for it.
    #[test]
    fn an_overflowed_props_record_is_not_read_as_every_property_removed() {
        let (mut doc, handle) = doc_with_author("P-1001", r#"{"设计压力":"1.6MPa"}"#);
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        // Past the soft limit, so `store` moves `props` out of the record.
        let long = "x".repeat(crate::pid::persist::XDATA_SOFT_LIMIT);
        author.props = format!(r#"{{"设计压力":"1.6MPa","备注":"{long}"}}"#);
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");
        assert!(
            !PidAuthor::props_are_inline(&after),
            "the fixture must overflow"
        );

        let changes = collect_changes(&[(handle, Some(before), Some(after))]);
        assert!(changes.is_empty(), "{changes:?}");
    }

    /// The tag of such a step is still in the record itself, so a retag made
    /// in it is still stated.
    #[test]
    fn a_retag_is_still_stated_when_the_props_overflowed() {
        let long = "x".repeat(crate::pid::persist::XDATA_SOFT_LIMIT);
        let (mut doc, handle) = doc_with_author("P-1001", &format!(r#"{{"备注":"{long}"}}"#));
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.tag = "P-1009".to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = collect_changes(&[(handle, Some(before), Some(after))]);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, KIND_SET_TAG);
        assert_eq!(changes[0].value, "P-1009");
    }

    /// Undoing a step is the same images read the other way.
    #[test]
    fn an_undone_step_states_the_change_backwards() {
        let (mut doc, handle) = doc_with_author("P-1001", "");
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.tag = "P-1009".to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");
        // Taken back, the drawing is as it was before the step.
        author.tag = "P-1001".to_owned();
        author.store(&mut doc, handle);

        let undone = collect_undone_step_changes(
            &doc,
            &[(handle, Some(before), Some(after))],
            &Left::Read(&[]),
        );
        assert_eq!(undone.len(), 1, "{undone:?}");
        assert_eq!(undone[0].object_ref, "P-1009");
        assert_eq!(undone[0].value, "P-1001");
        assert_eq!(undone[0].old.as_deref(), Some("P-1009"));
    }

    /// The payload for one change reads exactly like `architecture.md` §3.2
    /// writes it, plus what the audit and a same-file peer want: the value it
    /// replaced, and the entity.
    #[test]
    fn a_single_change_payload_carries_the_canonical_fields() {
        let change = Change {
            kind: KIND_SET_TAG.to_owned(),
            object_ref: "P-1001".to_owned(),
            handles: vec![Handle::new(0x1A)],
            field: "tag".to_owned(),
            value: "P-1009".to_owned(),
            old: Some("P-1001".to_owned()),
            placed: None,
            pipe: None,
            tag_source: None,
            overrides: None,
        };
        let payload = payload_for(std::slice::from_ref(&change)).expect("payload");
        assert_eq!(payload["kind"], KIND_SET_TAG);
        assert_eq!(payload["object_ref"], "P-1001");
        assert_eq!(payload["field"], "tag");
        assert_eq!(payload["value"], "P-1009");
        assert_eq!(payload["old_value"], "P-1001");
        assert_eq!(payload["handle"], "1A");
        assert!(payload.get("changes").is_none());
    }

    #[test]
    fn extra_changes_ride_along_under_changes() {
        let a = change(
            KIND_SET_TAG,
            "P-1001",
            Some(Handle::new(0x1A)),
            "tag",
            "P-1009",
        );
        let b = change(
            KIND_SET_PROPERTY,
            "V-1001",
            Some(Handle::new(0x1B)),
            "设计压力",
            "2.5MPa",
        );
        let payload = payload_for(&[a, b]).expect("payload");
        assert_eq!(payload["object_ref"], "P-1001");
        assert_eq!(payload["changes"].as_array().unwrap().len(), 1);
        assert_eq!(payload["changes"][0]["object_ref"], "V-1001");
    }

    /// VAL-CLI-009: the receiving side writes the new tag onto its own copy.
    #[test]
    fn a_set_tag_change_rewrites_the_local_tag() {
        let (mut doc, handle) = doc_with_author("P-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(
                KIND_SET_TAG,
                "P-1001",
                Some(handle),
                "tag",
                "P-1009",
            )],
        );
        assert_eq!(applied.handles, vec![handle]);
        assert_eq!(applied.retagged, vec![handle]);
        assert!(applied.unresolved.is_empty());
        let author = PidAuthor::load(&doc, handle).expect("author");
        assert_eq!(author.tag, "P-1009");
        assert_eq!(author.tag_source, "manual");
    }

    #[test]
    fn a_set_property_change_rewrites_only_that_key() {
        let (mut doc, handle) =
            doc_with_author("P-1001", r#"{"设计压力":"1.6MPa","设计温度":"120℃"}"#);
        apply_changes(
            &mut doc,
            &[change(
                KIND_SET_PROPERTY,
                "P-1001",
                Some(handle),
                "设计压力",
                "2.5MPa",
            )],
        );
        let (values, _) = split_props(&PidAuthor::load(&doc, handle).expect("author").props);
        assert_eq!(values.get("设计压力").map(String::as_str), Some("2.5MPa"));
        assert_eq!(values.get("设计温度").map(String::as_str), Some("120℃"));
    }

    /// A change that names only the tag — a client with no drawing of its own
    /// has nothing else to name — finds the one symbol carrying it.
    #[test]
    fn a_change_naming_only_the_tag_finds_the_symbol_carrying_it() {
        let mut doc = Document::new();
        let pump = add_author(&mut doc, "P-1001", "");
        let vessel = add_author(&mut doc, "V-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "V-1001", None, "tag", "V-1002")],
        );
        assert_eq!(applied.handles, vec![vessel]);
        assert_eq!(tag_of(&doc, vessel), "V-1002");
        assert_eq!(tag_of(&doc, pump), "P-1001");
    }

    /// A handle is not shared identity. Two clients that each inserted a
    /// symbol since they opened the file gave the same handle to different
    /// things, so a handle whose entity carries another tag is not believed.
    #[test]
    fn a_handle_whose_entity_carries_another_tag_is_not_believed() {
        let mut doc = Document::new();
        let pump = add_author(&mut doc, "P-1003", "");
        // The sender means its own vessel V-1002, which sits under the handle
        // this drawing gave to a pump.
        let applied = apply_changes(
            &mut doc,
            &[change(
                KIND_SET_PROPERTY,
                "V-1002",
                Some(pump),
                "设计压力",
                "2.5MPa",
            )],
        );
        assert!(applied.is_empty());
        assert_eq!(applied.unresolved, ["V-1002"]);
        assert_eq!(
            PidAuthor::load(&doc, pump).expect("author").props,
            "",
            "the pump must be left alone"
        );
    }

    /// Two symbols carrying one tag: the handle says which is meant, and
    /// without it there is no answer.
    #[test]
    fn the_handle_tells_same_tagged_symbols_apart() {
        let mut doc = Document::new();
        let first = add_author(&mut doc, "P-1001", "");
        let second = add_author(&mut doc, "P-1001", "");

        let applied = apply_changes(
            &mut doc,
            &[change(
                KIND_SET_TAG,
                "P-1001",
                Some(second),
                "tag",
                "P-1002",
            )],
        );
        assert_eq!(applied.handles, vec![second]);
        assert_eq!(tag_of(&doc, first), "P-1001");
        assert_eq!(tag_of(&doc, second), "P-1002");

        let mut doc = Document::new();
        let first = add_author(&mut doc, "P-1001", "");
        let second = add_author(&mut doc, "P-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "P-1001", None, "tag", "P-1002")],
        );
        assert!(applied.is_empty());
        assert_eq!(applied.unresolved, ["P-1001"]);
        assert_eq!(tag_of(&doc, first), "P-1001");
        assert_eq!(tag_of(&doc, second), "P-1001");
    }

    /// Re-applying the same change is a no-op, not a failure: a client that
    /// reconnects and re-reads the room must not report an error.
    #[test]
    fn reapplying_a_change_writes_nothing_and_reports_nothing() {
        let (mut doc, handle) = doc_with_author("P-1009", r#"{"设计压力":"2.5MPa"}"#);
        let applied = apply_changes(
            &mut doc,
            &[
                // The retag from P-1001 has been applied: P-1001 is gone and
                // P-1009 is here.
                change(KIND_SET_TAG, "P-1001", Some(handle), "tag", "P-1009"),
                change(
                    KIND_SET_PROPERTY,
                    "P-1009",
                    Some(handle),
                    "设计压力",
                    "2.5MPa",
                ),
            ],
        );
        assert!(applied.is_empty());
        assert!(applied.unresolved.is_empty(), "{:?}", applied.unresolved);
    }

    /// A change naming an object this drawing lacks writes nothing, and says
    /// so: the two drawings differ from here on.
    #[test]
    fn a_change_for_an_absent_object_is_reported_as_unresolved() {
        let (mut doc, _) = doc_with_author("P-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(
                KIND_SET_PROPERTY,
                "P-9001",
                Some(Handle::new(0xBEEF)),
                "设计压力",
                "2.5MPa",
            )],
        );
        assert!(applied.is_empty());
        assert_eq!(applied.unresolved, ["P-9001"]);
    }

    /// The `changes` array on the wire round-trips through `from_json`, and an
    /// unknown kind is dropped rather than half-applied.
    #[test]
    fn the_wire_shape_round_trips_and_unknown_kinds_are_dropped() {
        let sent = Change {
            kind: KIND_SET_TAG.to_owned(),
            object_ref: "P-1001".to_owned(),
            handles: vec![Handle::new(0x1A)],
            field: "tag".to_owned(),
            value: "P-1009".to_owned(),
            old: Some("P-1001".to_owned()),
            placed: None,
            pipe: None,
            tag_source: None,
            overrides: None,
        };
        assert_eq!(Change::from_json(&sent.to_json()), Some(sent));

        // As `architecture.md` §3.2 writes a retag: no field, no handle.
        let bare = json!({"kind":"set_tag","object_ref":"V-1001","value":"V-1002"});
        let change = Change::from_json(&bare).expect("parsed");
        assert_eq!(change.object_ref, "V-1001");
        assert_eq!(change.field, "tag");
        assert!(change.handles.is_empty());

        // The server backfills absent canonical keys with null.
        let backfilled = json!({
            "kind":"set_tag","object_ref":"V-1001","field":null,"value":"V-1002"
        });
        assert_eq!(
            Change::from_json(&backfilled).map(|change| change.field),
            Some("tag".to_owned())
        );

        // A kind this client has no way to act on. (`delete_object` stood
        // here until this client learned to erase, and `create_pipe_line`
        // until it learned to draw the line — see `super::pipes`.)
        let unknown = json!({"kind":"rename_drawing","tag":"L-1001","object_ref":"L-1001"});
        assert!(Change::from_json(&unknown).is_none());
        // A pipe line that does not say where it runs is still not one it
        // can draw.
        let nowhere = json!({"kind":"create_pipe_line","tag":"L-1001","object_ref":"L-1001"});
        assert!(Change::from_json(&nowhere).is_none());
        // And a symbol that says neither where it stands nor what it is.
        let placeless = json!({"kind":"insert_symbol","tag":"V-1001","type":"VALVE"});
        assert!(Change::from_json(&placeless).is_none());
        let nameless = json!({"kind":"set_tag","value":"x"});
        assert!(Change::from_json(&nameless).is_none());
        let valueless = json!({"kind":"set_property","object_ref":"P-1001","field":"设计压力"});
        assert!(Change::from_json(&valueless).is_none());
        let fieldless = json!({"kind":"set_property","object_ref":"P-1001","value":"x"});
        assert!(Change::from_json(&fieldless).is_none());
    }

    /// A batched payload nests the rest under `changes`; the receiver flattens
    /// it so every change is applied.
    #[test]
    fn a_batched_payload_flattens_on_the_receiving_side() {
        let changes = vec![json!({
            "kind":"set_tag","object_ref":"P-1001","field":"tag","value":"P-1009",
            "changes":[{"kind":"set_property","object_ref":"P-1009","field":"设计压力","value":"2.5MPa"}]
        })];
        let read = changes_from_payload(&changes);
        let parsed = read.changes;
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].object_ref, "P-1001");
        assert_eq!(parsed[1].object_ref, "P-1009");
        assert_eq!(parsed[1].field, "设计压力");
        assert!(read.unread.is_empty());
    }

    /// An op that carries changes this client can act on beside ones it
    /// cannot is not "applied": the ones it cannot read are named, so the
    /// caller can say the drawing is short of them. They used to be passed
    /// over as long as anything else in the op could be read.
    #[test]
    fn changes_this_client_cannot_read_are_named_not_dropped() {
        let changes = vec![
            json!({"kind":"set_tag","object_ref":"P-1001","field":"tag","value":"P-1009"}),
            json!({"kind":"insert_symbol","object_ref":"V-1001","symbol":"gate_valve"}),
            json!({
                "kind":"set_property","object_ref":"P-1009","field":"设计压力","value":"2.5MPa",
                "changes":[{"object_ref":"X-1"}]
            }),
        ];
        let read = changes_from_payload(&changes);
        assert_eq!(read.changes.len(), 2, "{read:?}");
        assert_eq!(read.unread, ["insert_symbol", "?"]);
    }

    fn add_line_piece(doc: &mut Document, tag: &str, role: &str, type_code: &str) -> Handle {
        let handle = doc
            .add_entity(EntityType::Line(document::entities::Line::from_points(
                Vector3::ZERO,
                Vector3::new(10.0, 0.0, 0.0),
            )))
            .expect("entity");
        PidAuthor {
            type_code: type_code.to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            topology: role.to_owned(),
            props: "{}".to_owned(),
            ..PidAuthor::default()
        }
        .store(doc, handle);
        handle
    }

    /// A pipe line is one object drawn as one entity per stretch, every one
    /// carrying its tag. A change that names only the tag — from a client
    /// with no drawing, or with another file — means the line, and goes to
    /// all of it; it used to be "several carry it, no answer".
    #[test]
    fn a_line_drawn_as_several_entities_is_one_object_to_a_change_naming_its_tag() {
        let mut doc = Document::new();
        let pieces: Vec<Handle> = (0..3)
            .map(|_| add_line_piece(&mut doc, "L-1001", ROLE_PIPE_LINE, "PIPE_LINE"))
            .collect();
        let pump = add_author(&mut doc, "P-1001", "");

        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_PROPERTY, "L-1001", None, "管道等级", "A1A")],
        );
        assert!(applied.unresolved.is_empty(), "{applied:?}");
        assert_eq!(applied.handles, pieces);
        for piece in &pieces {
            let props = PidAuthor::load(&doc, *piece).expect("author").props;
            assert!(props.contains("A1A"), "{props}");
        }

        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "L-1001", None, "tag", "L-1002")],
        );
        assert!(applied.unresolved.is_empty(), "{applied:?}");
        assert_eq!(applied.retagged, pieces);
        for piece in &pieces {
            assert_eq!(tag_of(&doc, *piece), "L-1002");
        }
        assert_eq!(tag_of(&doc, pump), "P-1001");
    }

    /// A signal line is the same; two separate symbols that happen to carry
    /// one tag are not — a line's pieces share a role, a duplicate does not
    /// become one object by being one.
    #[test]
    fn only_the_pieces_of_one_line_count_as_one_object() {
        let mut doc = Document::new();
        let signal: Vec<Handle> = (0..2)
            .map(|_| add_line_piece(&mut doc, "S-1", ROLE_SIGNAL_LINE, "SIGNAL_LINE"))
            .collect();
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_PROPERTY, "S-1", None, "信号类型", "4-20mA")],
        );
        assert_eq!(applied.handles, signal);

        // A line piece and a symbol under one tag: not one line.
        let mut doc = Document::new();
        add_line_piece(&mut doc, "L-1001", ROLE_PIPE_LINE, "PIPE_LINE");
        add_author(&mut doc, "L-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_PROPERTY, "L-1001", None, "管道等级", "A1A")],
        );
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(applied.unresolved, ["L-1001"]);
    }

    /// What was done to several entities of one object is said once, with
    /// the entities it was written to — and a peer holding the same file
    /// writes exactly those.
    #[test]
    fn a_change_made_to_several_entities_of_one_object_is_stated_once() {
        let mut doc = Document::new();
        let pieces: Vec<Handle> = (0..3)
            .map(|_| add_line_piece(&mut doc, "L-1001", ROLE_PIPE_LINE, "PIPE_LINE"))
            .collect();
        let mut images = Vec::new();
        // The first two pieces are retagged; the third is left as it is.
        for piece in &pieces[..2] {
            let before = doc.get_entity_arc(*piece);
            let mut author = PidAuthor::load(&doc, *piece).expect("author");
            author.tag = "L-1002".to_owned();
            author.store(&mut doc, *piece);
            images.push((*piece, before, doc.get_entity_arc(*piece)));
        }

        let stated = collect_changes(&images);
        assert_eq!(stated.len(), 1, "{stated:?}");
        assert_eq!(stated[0].kind, KIND_SET_TAG);
        assert_eq!(stated[0].object_ref, "L-1001");
        assert_eq!(stated[0].handles, pieces[..2]);
        let wire = stated[0].to_json();
        assert_eq!(wire["handle"], format!("{:X}", pieces[0].value()));
        assert_eq!(wire["handles"].as_array().map(Vec::len), Some(2));
        assert_eq!(Change::from_json(&wire).as_ref(), Some(&stated[0]));

        // A peer with the same file, before the change: only those two.
        let mut peer = Document::new();
        let theirs: Vec<Handle> = (0..3)
            .map(|_| add_line_piece(&mut peer, "L-1001", ROLE_PIPE_LINE, "PIPE_LINE"))
            .collect();
        assert_eq!(theirs, pieces, "the same file gives the same handles");
        let applied = apply_changes(&mut peer, &stated);
        assert_eq!(applied.retagged, pieces[..2]);
        assert_eq!(tag_of(&peer, pieces[2]), "L-1001");
    }

    /// Handles are believed all together or not at all. A sender whose
    /// entities only partly carry the tag here is not holding this file, and
    /// one of its handles matching by chance does not pick an object out of
    /// a duplicate.
    #[test]
    fn handles_that_only_partly_match_are_not_believed() {
        let mut doc = Document::new();
        let first = add_author(&mut doc, "P-1001", "");
        let second = add_author(&mut doc, "P-1001", "");
        let other = add_author(&mut doc, "V-1001", "");
        let mut both = change(KIND_SET_PROPERTY, "P-1001", None, "设计压力", "2.5MPa");
        both.handles = vec![first, other];
        let applied = apply_changes(&mut doc, &[both]);
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(applied.unresolved, ["P-1001"]);
        assert_eq!(PidAuthor::load(&doc, first).expect("author").props, "");
        assert_eq!(PidAuthor::load(&doc, second).expect("author").props, "");
    }

    /// A rename of an object this drawing never had did not happen here,
    /// whatever else carries the new tag. Only the sender's own entities
    /// carrying it already say "done".
    #[test]
    fn a_retag_this_drawing_never_had_is_reported_though_the_new_tag_is_here() {
        let mut doc = Document::new();
        let other = add_author(&mut doc, "P-1009", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "P-1001", None, "tag", "P-1009")],
        );
        assert!(applied.is_empty());
        assert_eq!(
            applied.unresolved,
            ["P-1001"],
            "nothing was renamed here, and that has to be said"
        );

        // The sender's entity is another one, carrying something else.
        let stranger = add_author(&mut doc, "V-1001", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "P-1001", Some(stranger), "tag", "P-1009")],
        );
        assert_eq!(applied.unresolved, ["P-1001"]);

        // The sender's entity carries the new tag: the same file, renamed
        // already.
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "P-1001", Some(other), "tag", "P-1009")],
        );
        assert!(applied.unresolved.is_empty(), "{applied:?}");
        assert!(applied.is_empty());
    }

    /// An object without a tag — a pipe segment, a break, a connector — has
    /// no name a peer can check, and its handle is not one: a peer that drew
    /// a pipe of its own since it opened the file has another untagged
    /// segment under that very handle. A change naming no tag is applied to
    /// nothing, whatever handle rides along.
    #[test]
    fn a_change_naming_no_tag_is_not_applied_by_its_handle_alone() {
        let mut doc = Document::new();
        let segment = add_author(&mut doc, "", r#"{"公称直径":"DN50"}"#);
        let applied = apply_changes(
            &mut doc,
            &[change(
                KIND_SET_PROPERTY,
                "",
                Some(segment),
                "公称直径",
                "DN80",
            )],
        );
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(applied.unresolved, [UNTAGGED]);
        assert_eq!(
            PidAuthor::load(&doc, segment).expect("author").props,
            r#"{"公称直径":"DN50"}"#,
            "this drawing's own segment must be left alone"
        );
    }

    /// So the sender does not state such a change at all. What an untagged
    /// object carries is derived — a segment takes its run's values — and the
    /// peer derives it again from the tagged change that caused it.
    #[test]
    fn a_change_to_an_untagged_object_is_not_stated() {
        let (mut doc, handle) = doc_with_author("", r#"{"公称直径":"DN50"}"#);
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.props = r#"{"公称直径":"DN80"}"#.to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = changes_of(handle, Some(&before), Some(&after));
        assert!(changes.is_empty(), "{changes:?}");
    }

    /// A retag whose old tag fits several symbols here names none of them.
    /// That the new tag is already carried by something else does not make it
    /// "applied already": the report must still be made.
    #[test]
    fn a_retag_fitting_several_symbols_is_reported_even_when_the_new_tag_exists() {
        let mut doc = Document::new();
        let first = add_author(&mut doc, "P-1001", "");
        let second = add_author(&mut doc, "P-1001", "");
        let other = add_author(&mut doc, "P-1009", "");
        let applied = apply_changes(
            &mut doc,
            &[change(KIND_SET_TAG, "P-1001", None, "tag", "P-1009")],
        );
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(applied.unresolved, ["P-1001"]);
        assert_eq!(tag_of(&doc, first), "P-1001");
        assert_eq!(tag_of(&doc, second), "P-1001");
        assert_eq!(tag_of(&doc, other), "P-1009");
    }

    // ── Override marks, and where a tag came from (review N4) ────────────

    fn marks(fields: &[&str]) -> BTreeSet<String> {
        fields.iter().map(|field| (*field).to_owned()).collect()
    }

    /// A run carrying `tag` and `props`.
    fn add_run(doc: &mut Document, tag: &str, props: &str) -> Handle {
        let run = add_line_piece(doc, tag, ROLE_PIPE_RUN, "PIPE_RUN");
        let mut author = PidAuthor::load(doc, run).expect("author");
        author.props = props.to_owned();
        author.store(doc, run);
        run
    }

    /// A run's field changed by hand carries an override mark, and from
    /// then on does not take its line's value. The change says which fields
    /// of the run are marked once the step is done, so that a peer holds
    /// the run the same way: it used to say the value alone, and the peer's
    /// run went on following its line (review N4). Taken back, it says the
    /// old value, and that the field is marked no more.
    #[test]
    fn collab_override_marks_a_hand_edit_says_which_fields_are_marked() {
        let mut doc = Document::new();
        let run = add_run(
            &mut doc,
            "L-1001-01",
            r#"{"公称直径":"DN80","管道等级":"A1A"}"#,
        );
        let before = doc.get_entity_arc(run).expect("before");
        crate::pid::propagate::set_prop_manual(&mut doc, run, "管道等级", "B2B").expect("a run");
        let after = doc.get_entity_arc(run).expect("after");

        let changes = changes_of(run, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        let change = &changes[0];
        assert_eq!(
            (
                change.kind.as_str(),
                change.object_ref.as_str(),
                change.field.as_str(),
                change.value.as_str(),
                change.old.as_deref(),
            ),
            (
                KIND_SET_PROPERTY,
                "L-1001-01",
                "管道等级",
                "B2B",
                Some("A1A")
            )
        );
        assert_eq!(change.overrides, Some(marks(&["管道等级"])));
        let wire = change.to_json();
        assert_eq!(wire["overrides"], json!(["管道等级"]), "{wire}");
        assert_eq!(Change::from_json(&wire).as_ref(), Some(change));

        let undone = changes_of(run, Some(&after), Some(&before));
        assert_eq!(undone.len(), 1, "{undone:?}");
        assert_eq!(undone[0].value, "A1A");
        assert_eq!(undone[0].overrides, Some(BTreeSet::new()));
        let wire = undone[0].to_json();
        assert_eq!(wire["overrides"], json!([]), "{wire}");
        assert_eq!(Change::from_json(&wire).as_ref(), Some(&undone[0]));
    }

    /// Every property change of an object that carries a mark lists the
    /// marks — a field the run took from its line too, which is how the
    /// peer hears that this one is not marked. An object that carries none,
    /// before the step or after it, says nothing about marks: its changes
    /// are written as they always were.
    #[test]
    fn collab_override_marks_only_an_object_that_carries_marks_lists_them() {
        let mut doc = Document::new();
        let run = add_run(
            &mut doc,
            "L-1001-01",
            r#"{"公称直径":"DN80","管道等级":"A1A","__overrides":["公称直径"]}"#,
        );
        let before = doc.get_entity_arc(run).expect("before");
        // As the carry-down from the line writes it: the value, no mark.
        let mut author = PidAuthor::load(&doc, run).expect("author");
        author.props =
            r#"{"公称直径":"DN80","管道等级":"C3C","__overrides":["公称直径"]}"#.to_owned();
        author.store(&mut doc, run);
        let after = doc.get_entity_arc(run).expect("after");

        let changes = changes_of(run, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(
            (changes[0].field.as_str(), changes[0].value.as_str()),
            ("管道等级", "C3C")
        );
        assert_eq!(changes[0].overrides, Some(marks(&["公称直径"])));

        let (mut doc, pump) = doc_with_author("P-1001", r#"{"设计压力":"1.6MPa"}"#);
        let before = doc.get_entity_arc(pump).expect("before");
        let mut author = PidAuthor::load(&doc, pump).expect("author");
        author.props = r#"{"设计压力":"2.5MPa"}"#.to_owned();
        author.store(&mut doc, pump);
        let after = doc.get_entity_arc(pump).expect("after");

        let changes = changes_of(pump, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].overrides, None);
        let wire = changes[0].to_json();
        assert!(wire.get("overrides").is_none(), "{wire}");
    }

    /// A mark that came or went while the value stayed is a change of the
    /// field all the same, said with the value the field has.
    #[test]
    fn collab_override_marks_a_mark_that_moved_alone_is_still_said() {
        let mut doc = Document::new();
        let run = add_run(&mut doc, "L-1001-01", r#"{"管道等级":"A1A"}"#);
        let before = doc.get_entity_arc(run).expect("before");
        crate::pid::propagate::set_prop_manual(&mut doc, run, "管道等级", "A1A").expect("a run");
        let after = doc.get_entity_arc(run).expect("after");

        let changes = changes_of(run, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(
            (
                changes[0].kind.as_str(),
                changes[0].field.as_str(),
                changes[0].value.as_str(),
                changes[0].old.as_deref(),
            ),
            (KIND_SET_PROPERTY, "管道等级", "A1A", Some("A1A"))
        );
        assert_eq!(changes[0].overrides, Some(marks(&["管道等级"])));

        let undone = changes_of(run, Some(&after), Some(&before));
        assert_eq!(undone.len(), 1, "{undone:?}");
        assert_eq!(undone[0].value, "A1A");
        assert_eq!(undone[0].overrides, Some(BTreeSet::new()));
    }

    /// The receiving side marks the change's own field when the sender
    /// lists it, and takes the mark off when the list leaves it out — also
    /// when the value is in place already. Marks on other fields are not
    /// this change's to move, and a change that says nothing about marks
    /// (an older client's) moves none.
    #[test]
    fn collab_override_marks_a_property_change_marks_or_unmarks_its_own_field() {
        let mut doc = Document::new();
        let run = add_run(
            &mut doc,
            "L-1001-01",
            r#"{"公称直径":"DN80","管道等级":"A1A","__overrides":["公称直径"]}"#,
        );
        let grade = |value: &str, listed: Option<BTreeSet<String>>| {
            let mut said = change(KIND_SET_PROPERTY, "L-1001-01", Some(run), "管道等级", value);
            said.overrides = listed;
            said
        };
        // The run's grade, and the fields it holds marked.
        let carried = |doc: &Document| {
            let (values, marked) = split_props(&PidAuthor::load(doc, run).expect("author").props);
            (values.get("管道等级").cloned(), marked)
        };
        let grade_of = |value: &str| Some(value.to_owned());

        // Changed by hand at the sender.
        let applied = apply_changes(&mut doc, &[grade("B2B", Some(marks(&["管道等级"])))]);
        assert_eq!(applied.handles, vec![run]);
        assert_eq!(
            carried(&doc),
            (grade_of("B2B"), marks(&["公称直径", "管道等级"]))
        );

        // Taken back there.
        apply_changes(&mut doc, &[grade("A1A", Some(marks(&[])))]);
        assert_eq!(carried(&doc), (grade_of("A1A"), marks(&["公称直径"])));

        // The value in place already, the mark not: still a write.
        let applied = apply_changes(&mut doc, &[grade("A1A", Some(marks(&["管道等级"])))]);
        assert_eq!(applied.handles, vec![run]);
        assert_eq!(
            carried(&doc),
            (grade_of("A1A"), marks(&["公称直径", "管道等级"]))
        );
        // Said again, there is nothing left to write.
        let applied = apply_changes(&mut doc, &[grade("A1A", Some(marks(&["管道等级"])))]);
        assert!(applied.is_empty(), "{applied:?}");

        // Nothing said about marks: the value, and the marks as they are.
        apply_changes(&mut doc, &[grade("C3C", None)]);
        assert_eq!(
            carried(&doc),
            (grade_of("C3C"), marks(&["公称直径", "管道等级"]))
        );
    }

    /// A retag says where the new tag came from — typed by hand, or worked
    /// out by the rule — so that the peer's record says the same. Taken
    /// back, it is the old tag, from where that one had come.
    #[test]
    fn collab_tag_source_a_retag_says_where_the_new_tag_came_from() {
        let (mut doc, handle) = doc_with_author("P-1001", "");
        let before = doc.get_entity_arc(handle).expect("before");
        let mut author = PidAuthor::load(&doc, handle).expect("author");
        author.tag = "P-1009".to_owned();
        author.tag_source = "manual".to_owned();
        author.store(&mut doc, handle);
        let after = doc.get_entity_arc(handle).expect("after");

        let changes = changes_of(handle, Some(&before), Some(&after));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(
            (changes[0].kind.as_str(), changes[0].value.as_str()),
            (KIND_SET_TAG, "P-1009")
        );
        assert_eq!(changes[0].tag_source.as_deref(), Some("manual"));
        let wire = changes[0].to_json();
        assert_eq!(wire["tag_source"], "manual", "{wire}");
        assert_eq!(Change::from_json(&wire).as_ref(), Some(&changes[0]));

        let undone = changes_of(handle, Some(&after), Some(&before));
        assert_eq!(undone.len(), 1, "{undone:?}");
        assert_eq!(
            (undone[0].value.as_str(), undone[0].tag_source.as_deref()),
            ("P-1001", Some("auto"))
        );
    }

    /// The receiving side writes the source the sender names: a tag the
    /// rule gave there — a nozzle renamed with its equipment, a retag taken
    /// back — is not one somebody typed here. It used to be written as
    /// typed whatever it was (review N4), and still is when the sender
    /// names no source.
    #[test]
    fn collab_tag_source_a_retag_is_written_with_the_source_the_sender_names() {
        let (mut doc, handle) = doc_with_author("P-1001", "");
        let retag = |from: &str, to: &str, source: Option<&str>| {
            let mut said = change(KIND_SET_TAG, from, Some(handle), "tag", to);
            said.tag_source = source.map(str::to_owned);
            said
        };
        let stored = |doc: &Document| {
            let author = PidAuthor::load(doc, handle).expect("author");
            (author.tag, author.tag_source)
        };
        let is = |tag: &str, source: &str| (tag.to_owned(), source.to_owned());

        apply_changes(&mut doc, &[retag("P-1001", "P-1005", Some("auto"))]);
        assert_eq!(stored(&doc), is("P-1005", "auto"));
        apply_changes(&mut doc, &[retag("P-1005", "P-1009", Some("manual"))]);
        assert_eq!(stored(&doc), is("P-1009", "manual"));
        // Taken back at the sender.
        apply_changes(&mut doc, &[retag("P-1009", "P-1005", Some("auto"))]);
        assert_eq!(stored(&doc), is("P-1005", "auto"));
        // A sender that names no source.
        apply_changes(&mut doc, &[retag("P-1005", "P-1001", None)]);
        assert_eq!(stored(&doc), is("P-1001", "manual"));
    }

    // ── The drawing's own tags (`hello.existing_tags`) ───────────────────

    /// The payload of one unanswered step that made `changes`.
    fn step(changes: &[Change]) -> Value {
        payload_for(changes).expect("a step that says something")
    }

    /// What `doc` lists with every step answered.
    fn carried(doc: &Document) -> Vec<String> {
        existing_tags(doc, std::iter::empty())
    }

    fn retag(doc: &mut Document, handle: Handle, tag: &str) {
        let mut author = PidAuthor::load(doc, handle).expect("author");
        author.tag = tag.to_owned();
        author.store(doc, handle);
    }

    /// What a `hello` lists: the tag of every object of the drawing that has
    /// one — a symbol, a pipe line (one tag, however many pieces it is drawn
    /// as), a valve fitted on a run — each once, in order. An object with no
    /// tag has nothing to list, and neither has plain geometry.
    #[test]
    fn collab_existing_tags_every_tag_the_drawing_carries_is_listed_once() {
        let mut doc = Document::new();
        add_author(&mut doc, "V-1001", "{}");
        add_author(&mut doc, "P-1001", "{}");
        add_author(&mut doc, "", "{}");
        for _ in 0..3 {
            add_line_piece(&mut doc, "L-1001", ROLE_PIPE_LINE, "PIPE_LINE");
        }
        add_line_piece(&mut doc, "", "", "PIPE_SEGMENT");
        let fitting = add_author(&mut doc, "V-2001", "{}");
        let mut record = PidAuthor::load(&doc, fitting).expect("author");
        record.owner = "1F".to_owned();
        record.topology = ROLE_FITTING.to_owned();
        record.store(&mut doc, fitting);
        doc.add_entity(EntityType::Line(document::entities::Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 0.0, 0.0),
        )))
        .expect("entity");

        assert_eq!(carried(&doc), ["L-1001", "P-1001", "V-1001", "V-2001"]);
        assert!(carried(&Document::new()).is_empty());
    }

    /// A symbol placed in a step the room has not answered yet is not
    /// listed, whatever it has been called since: the room hears of it from
    /// that step. Listed with the `hello`, its tag would be taken when the
    /// step arrives, and the server would give this client's own symbol
    /// another.
    #[test]
    fn collab_existing_tags_a_symbol_the_room_has_not_heard_of_yet_is_left_out() {
        let mut doc = Document::new();
        add_author(&mut doc, "P-1001", "{}");
        let new = add_author(&mut doc, "P-1002", "{}");
        let entity = doc.get_entity(new).cloned().expect("entity");
        let placing = step(&changes_of(new, None, Some(&entity)));
        assert_eq!(placing["kind"], KIND_INSERT_SYMBOL);

        assert_eq!(existing_tags(&doc, [&placing]), ["P-1001"]);

        // Retagged in a later step: neither tag is one the room knows.
        retag(&mut doc, new, "P-1003");
        let retagging = step(&[change(KIND_SET_TAG, "P-1002", Some(new), "tag", "P-1003")]);
        assert_eq!(existing_tags(&doc, [&placing, &retagging]), ["P-1001"]);

        // Erased again in a third: there is nothing to list, and nothing
        // the room is owed.
        let entity = doc.get_entity(new).cloned().expect("entity");
        let erasing = step(&changes_of(new, Some(&entity), None));
        doc.remove_entity(new);
        assert_eq!(
            existing_tags(&doc, [&placing, &retagging, &erasing]),
            ["P-1001"]
        );
    }

    /// Once the room has answered the step, the symbol is an object of the
    /// drawing like any other.
    #[test]
    fn collab_existing_tags_a_symbol_whose_step_was_answered_is_listed() {
        let mut doc = Document::new();
        add_author(&mut doc, "P-1001", "{}");
        add_author(&mut doc, "P-1002", "{}");
        assert_eq!(carried(&doc), ["P-1001", "P-1002"]);
    }

    /// An object retagged in a step the room has not answered yet is listed
    /// by the tag the room knows it by — the one it had before the first
    /// such step — and one erased in such a step is listed still. To the
    /// room it is there, under that tag, until the step arrives; were the
    /// tag free meanwhile, a symbol a peer placed under it would be what the
    /// step then renamed or erased.
    #[test]
    fn collab_existing_tags_what_an_unanswered_step_renamed_or_erased_is_listed_by_its_old_tag() {
        let mut doc = Document::new();
        let pump = add_author(&mut doc, "P-1001", "{}");
        let vessel = add_author(&mut doc, "V-1001", "{}");
        add_author(&mut doc, "E-1001", "{}");
        // P-1001 → P-1005 → P-1009 in two steps, a property with the second.
        retag(&mut doc, pump, "P-1009");
        let first = step(&[change(KIND_SET_TAG, "P-1001", Some(pump), "tag", "P-1005")]);
        let second = step(&[
            change(KIND_SET_PROPERTY, "P-1005", Some(pump), "公称直径", "DN50"),
            change(KIND_SET_TAG, "P-1005", Some(pump), "tag", "P-1009"),
        ]);
        // V-1001 erased.
        let entity = doc.get_entity(vessel).cloned().expect("entity");
        let erasing = step(&changes_of(vessel, Some(&entity), None));
        doc.remove_entity(vessel);

        assert_eq!(carried(&doc), ["E-1001", "P-1009"]);
        assert_eq!(
            existing_tags(&doc, [&first, &second, &erasing]),
            ["E-1001", "P-1001", "V-1001"]
        );
    }

    /// Erased in one unanswered step and put back in the next — an erase
    /// taken back — the object is what the room has known all along: listed,
    /// once. A step that only wrote a property changes nothing of the
    /// listing.
    #[test]
    fn collab_existing_tags_an_erase_taken_back_leaves_the_object_listed() {
        let mut doc = Document::new();
        let pump = add_author(&mut doc, "P-1001", "{}");
        let entity = doc.get_entity(pump).cloned().expect("entity");
        let erasing = step(&changes_of(pump, Some(&entity), None));
        let placing = step(&changes_of(pump, None, Some(&entity)));
        let write = change(KIND_SET_PROPERTY, "P-1001", Some(pump), "公称直径", "DN50");
        let writing = step(&[write]);

        assert_eq!(existing_tags(&doc, [&erasing, &placing]), ["P-1001"]);
        assert_eq!(existing_tags(&doc, [&writing]), ["P-1001"]);
    }

    /// A pipe line renamed in an unanswered step is every one of its pieces
    /// retagged, said once with all of them: the line is listed by the tag
    /// the room knows, once.
    #[test]
    fn collab_existing_tags_a_line_renamed_in_an_unanswered_step_is_listed_by_its_old_tag() {
        let mut doc = Document::new();
        let pieces: Vec<Handle> = (0..3)
            .map(|_| add_line_piece(&mut doc, "L-1002", ROLE_PIPE_LINE, "PIPE_LINE"))
            .collect();
        add_author(&mut doc, "P-1001", "{}");
        let mut renaming = change(KIND_SET_TAG, "L-1001", None, "tag", "L-1002");
        renaming.handles = pieces;

        assert_eq!(
            existing_tags(&doc, [&step(&[renaming])]),
            ["L-1001", "P-1001"]
        );
    }

    /// A change that names no entity — written by hand, or read off another
    /// drawing — does not say which object here it is about, and the listing
    /// does not guess: the drawing is listed as it is.
    #[test]
    fn collab_existing_tags_a_change_naming_no_entity_is_not_guessed_at() {
        let mut doc = Document::new();
        add_author(&mut doc, "P-1009", "{}");
        let waiting = [
            step(&[change(KIND_SET_TAG, "P-1001", None, "tag", "P-1009")]),
            step(&[change(KIND_DELETE_OBJECT, "V-1001", None, "", "")]),
        ];

        assert_eq!(existing_tags(&doc, &waiting), ["P-1009"]);
    }
}
