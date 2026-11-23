//! Page connectors: how one made or taken away is said to the room, and how
//! a peer's drawing follows what was said — including what the server says
//! of a connector whose other end is on another drawing (`VAL-CROSS-009`;
//! the protocol is §3 of `docs/plans/2026-11-12-collab-completion.md`).
//!
//! A connector has no tag. In a drawing it is the connector of its kind on
//! its line at its place; nothing here goes by handle, for the same reason
//! as in `super::pipes`.
//!
//! Pairing across drawings is not this client's to do: it has one drawing
//! in the session. It says which drawing the other end is on, the server
//! finds that end and tells both rooms (`pair_connector`), and from then on
//! commits, on the drawing the line flows in on, every write to the fields
//! that go along the connector. Those arrive here as ordinary field writes
//! on the line.

use std::collections::BTreeMap;

use document::{Document, EntityType, Handle};
use serde_json::{json, Value};

use super::broadcast::{Applied, Change};
use super::pipes::{Image, Pipe};
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{
    handle_text, ConnectorKind, ConnectorRecord, TopologyTable, ROLE_CONNECTOR_IN,
    ROLE_CONNECTOR_OUT, ROLE_SIGNAL_LINE,
};

/// A connector was put on a line.
pub const KIND_CREATE_CONNECTOR: &str = "create_connector";
/// A connector is gone from a line.
pub const KIND_DELETE_CONNECTOR: &str = "delete_connector";
/// The server: a connector has an other end on another drawing, or has
/// none any more.
pub const KIND_PAIR_CONNECTOR: &str = "pair_connector";

/// The marker a connector is drawn as: a circle of this radius (as
/// `PIDCONNECTOR` draws it).
const RADIUS: f64 = 3.0;
/// How far from a line a connector may stand and still be on it (as
/// `PIDCONNECTOR` takes its pick).
const ON_THE_LINE: f64 = 5.0;
/// How far apart two places may be and still be the same place.
const SAME_PLACE: f64 = 1e-6;

/// The fields that go along a pipe connector, and along a signal one.
const PIPE_FIELDS: [&str; 5] = ocs_collab::link::PIPE_FIELDS;
const SIGNAL_FIELDS: [&str; 2] = ocs_collab::link::SIGNAL_FIELDS;

/// What a statement about a connector says. The line it stands on is the
/// statement's `object_ref`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectorSaid {
    /// `pipe_in`, `pipe_out`, `signal_in` or `signal_out`.
    pub connector: String,
    /// Where it stands. A pairing may leave it out: an inflow is the only
    /// one of its line.
    pub at: Option<[f64; 2]>,
    /// `create_connector`: the drawing the other end is on, by the
    /// server's id for it.
    pub target_drawing: Option<String>,
    /// `create_connector`: what the line carries, of the fields that go
    /// along the connector, in the drawing of whoever made it.
    pub line_props: BTreeMap<String, String>,
    /// `pair_connector`: the other end as `(drawing, line, connector)`;
    /// `None` when it has none any more.
    pub peer: Option<(String, String, String)>,
}

fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= SAME_PLACE && (a[1] - b[1]).abs() <= SAME_PLACE
}

fn carried(kind: ConnectorKind) -> &'static [&'static str] {
    if kind.host_role() == ROLE_SIGNAL_LINE {
        &SIGNAL_FIELDS
    } else {
        &PIPE_FIELDS
    }
}

// ── on the wire ──────────────────────────────────────────────────────────

/// The JSON body of a connector statement.
pub(super) fn to_json(change: &Change, said: &ConnectorSaid) -> Value {
    let mut out = json!({
        "kind": change.kind,
        "line_tag": change.object_ref,
        "connector": said.connector,
    });
    if let Some(at) = said.at {
        out["at"] = json!(at);
    }
    match change.kind.as_str() {
        KIND_CREATE_CONNECTOR => {
            if let Some(target) = &said.target_drawing {
                out["target_drawing"] = json!(target);
            }
            if !said.line_props.is_empty() {
                out["line_props"] = json!(said.line_props);
            }
        }
        KIND_PAIR_CONNECTOR => {
            let part = |pick: fn(&(String, String, String)) -> &String| {
                said.peer.as_ref().map(pick).map_or(Value::Null, |text| json!(text))
            };
            out["peer_drawing"] = part(|peer| &peer.0);
            out["peer_line_tag"] = part(|peer| &peer.1);
            out["peer_connector"] = part(|peer| &peer.2);
        }
        _ => {}
    }
    out
}

/// Read a connector statement out of an `op_applied` entry of kind `kind`.
/// `None` when it does not say enough to act on. The server rewrites
/// `object_ref` to the line's tag on the two a client sends, which is what
/// `line_tag` says anyway.
pub(super) fn from_json(kind: &str, value: &Value) -> Option<Change> {
    let word = |key: &str| {
        value[key]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let line_tag = word("line_tag").or_else(|| word("object_ref"))?;
    let connector = ConnectorKind::parse(&word("connector")?)?.as_str().to_owned();
    let at = value["at"].as_array().and_then(|pair| {
        let number = |index: usize| pair.get(index)?.as_f64().filter(|n| n.is_finite());
        Some([number(0)?, number(1)?])
    });
    if at.is_none() && kind != KIND_PAIR_CONNECTOR {
        return None;
    }
    let line_props = match &value["line_props"] {
        Value::Object(map) => map
            .iter()
            .filter_map(|(field, value)| Some((field.clone(), value.as_str()?.to_owned())))
            .collect(),
        _ => BTreeMap::new(),
    };
    let peer = match (word("peer_drawing"), word("peer_line_tag"), word("peer_connector")) {
        (Some(drawing), Some(line), Some(connector)) => Some((drawing, line, connector)),
        _ => None,
    };
    Some(Change {
        kind: kind.to_owned(),
        object_ref: line_tag,
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: Some(Pipe::Connector(ConnectorSaid {
            connector,
            at,
            target_drawing: word("target_drawing"),
            line_props,
            peer,
        })),
        tag_source: None,
        overrides: None,
    })
}

// ── what a step did, read off its images and the drawing it left ──────────

/// The kind of the connector an author record is of, when it is of one.
fn kind_of(author: &PidAuthor) -> Option<ConnectorKind> {
    [
        ConnectorKind::PipeIn,
        ConnectorKind::PipeOut,
        ConnectorKind::SignalIn,
        ConnectorKind::SignalOut,
    ]
    .into_iter()
    .find(|kind| kind.role() == author.topology && kind.type_code() == author.type_code)
}

fn centre(entity: &EntityType) -> Option<[f64; 2]> {
    match entity {
        EntityType::Circle(circle) => Some([circle.center.x, circle.center.y]),
        _ => None,
    }
}

/// How far `point` is from the stretch `a`–`b`.
fn distance(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let along = if length <= 1e-12 {
        0.0
    } else {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    };
    let (x, y) = (a[0] + dx * along - point[0], a[1] + dy * along - point[1]);
    (x * x + y * y).sqrt()
}

/// The line of role `role` that passes nearest `at`, if one passes near
/// enough for a connector at `at` to be on it: its tag.
fn line_at(doc: &Document, role: &str, at: [f64; 2]) -> Option<String> {
    doc.entities()
        .filter_map(|entity| {
            let EntityType::Line(line) = entity else {
                return None;
            };
            let author = PidAuthor::from_entity(entity)?;
            (author.topology == role && !author.tag.is_empty()).then(|| {
                (
                    distance([line.start.x, line.start.y], [line.end.x, line.end.y], at),
                    author.tag,
                )
            })
        })
        .filter(|(far, _)| *far <= ON_THE_LINE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, tag)| tag)
}

/// A stretch of line `tag`, when the drawing has the line.
fn line_entity(doc: &Document, role: &str, tag: &str) -> Option<Handle> {
    doc.entities().find_map(|entity| {
        let author = PidAuthor::from_entity(entity)?;
        (matches!(entity, EntityType::Line(_)) && author.topology == role && author.tag == tag)
            .then(|| entity.common().handle)
    })
}

fn statement(kind: &str, line_tag: String, said: ConnectorSaid) -> Change {
    Change {
        kind: kind.to_owned(),
        object_ref: line_tag,
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: Some(Pipe::Connector(said)),
        tag_source: None,
        overrides: None,
    }
}

/// What one step did to the drawing's connectors, for the room: each one
/// that came is said as made — on which line, where, of what kind, towards
/// which drawing, and what the line carries — and each one that went as
/// gone. `doc` is the drawing as the step left it.
///
/// A connector that only changed (a pairing written on it) is not said:
/// across drawings pairing is the server's, and between two drawings of one
/// editor it is that editor's own business.
pub fn statements(doc: &Document, images: &[Image<'_>]) -> Vec<Change> {
    let mut out = Vec::new();
    let mut table: Option<TopologyTable> = None;
    for (handle, before, after) in images {
        let (entity, made) = match (before, after) {
            (None, Some(entity)) => (*entity, true),
            (Some(entity), None) => (*entity, false),
            _ => continue,
        };
        let Some(author) = PidAuthor::from_entity(entity) else {
            continue;
        };
        if author.topology != ROLE_CONNECTOR_IN && author.topology != ROLE_CONNECTOR_OUT {
            continue;
        }
        let (Some(kind), Some(at)) = (kind_of(&author), centre(entity)) else {
            continue;
        };
        if !made {
            // Its record went with it; the line it stood on is the one
            // under it — unless that went in the same step, and then the
            // line's going says all there is to say.
            if let Some(line_tag) = line_at(doc, kind.host_role(), at) {
                out.push(statement(
                    KIND_DELETE_CONNECTOR,
                    line_tag,
                    ConnectorSaid {
                        connector: kind.as_str().to_owned(),
                        at: Some(at),
                        ..ConnectorSaid::default()
                    },
                ));
            }
            continue;
        }
        let table = table.get_or_insert_with(|| TopologyTable::load(doc));
        let record = table.connector_by_handle(*handle);
        let Some(line_tag) = record
            .map(|record| record.line_tag.clone())
            .or_else(|| line_at(doc, kind.host_role(), at))
        else {
            continue;
        };
        // The drawing the other end is on, when the connector was made
        // towards one. A connector paired inside this editor names a
        // document of this process (`peer`), which means nothing to anyone
        // else.
        let target_drawing = record
            .filter(|record| record.peer.is_none())
            .and_then(|record| record.peer_drawing.clone())
            .filter(|drawing| !drawing.is_empty());
        let line_props = line_entity(doc, kind.host_role(), &line_tag)
            .map(|line| crate::pid::propagate::props_of(doc, line))
            .map(|props| {
                carried(kind)
                    .iter()
                    .filter_map(|field| {
                        let value = props.get(*field)?;
                        (!value.is_empty()).then(|| ((*field).to_owned(), value.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(statement(
            KIND_CREATE_CONNECTOR,
            line_tag,
            ConnectorSaid {
                connector: kind.as_str().to_owned(),
                at: Some(at),
                target_drawing,
                line_props,
                peer: None,
            },
        ));
    }
    out
}

// ── following what was said ──────────────────────────────────────────────

/// The connector of `kind` on line `line_tag` — at `at`, when a place is
/// given — in the drawing as it is.
///
/// By the entities, not by the topology table: taking a step of one's own
/// back puts the table back as it was when the step was made, without the
/// record of a connector a peer has made since, while the connector itself
/// stays. A record is told apart by its handle where there is one, and the
/// line under the connector says the rest.
fn find(
    doc: &Document,
    kind: ConnectorKind,
    line_tag: &str,
    at: Option<[f64; 2]>,
) -> Option<Handle> {
    let table = TopologyTable::load(doc);
    doc.entities().find_map(|entity| {
        let author = PidAuthor::from_entity(entity)?;
        let centre = centre(entity)?;
        if kind_of(&author).map(ConnectorKind::as_str) != Some(kind.as_str())
            || !at.is_none_or(|at| same(centre, at))
        {
            return None;
        }
        let handle = entity.common().handle;
        let on_line = match table.connector_by_handle(handle) {
            Some(record) => record.line_tag == line_tag,
            None => line_at(doc, kind.host_role(), centre).as_deref() == Some(line_tag),
        };
        on_line.then_some(handle)
    })
}

/// Whether the drawing is still as statement `change` left it: the
/// connector it made is there, the one it took away is not
/// (`SPEC-collab-offline-sync.md` §5.4).
pub(super) fn as_said(doc: &Document, change: &Change, said: &ConnectorSaid) -> bool {
    let Some(kind) = ConnectorKind::parse(&said.connector) else {
        return false;
    };
    let there = find(doc, kind, &change.object_ref, said.at).is_some();
    match change.kind.as_str() {
        KIND_CREATE_CONNECTOR => there,
        KIND_DELETE_CONNECTOR => !there,
        _ => false,
    }
}

/// The table's record of connector `handle`, entered if it is not there.
fn record_of<'a>(
    table: &'a mut TopologyTable,
    handle: Handle,
    kind: ConnectorKind,
    line_tag: &str,
) -> Option<&'a mut ConnectorRecord> {
    if table.connector_by_handle(handle).is_none() {
        let mut record = ConnectorRecord::new(handle_text(handle), kind, line_tag.to_owned());
        record.upstream = Some(line_tag.to_owned());
        table.upsert_connector(record);
    }
    table.connector_by_handle_mut(handle)
}

impl OpenCADStudio {
    /// Bring the drawing on tab `i` to what a statement about a connector
    /// says. Returns the connector's entity (none, for one that is gone),
    /// or the name to report when the statement cannot be followed.
    pub(in crate::app) fn write_remote_connector(
        &mut self,
        i: usize,
        change: &Change,
        said: &ConnectorSaid,
        applied: &mut Applied,
    ) -> Result<Vec<Handle>, String> {
        let line_tag = change.object_ref.as_str();
        let name = || format!("{line_tag} 上的连接符 {}", said.connector);
        let Some(kind) = ConnectorKind::parse(&said.connector) else {
            return Err(name());
        };
        let existing = find(&self.tabs[i].scene.document, kind, line_tag, said.at);
        match change.kind.as_str() {
            KIND_CREATE_CONNECTOR => {
                if let Some(handle) = existing {
                    // Said again: it is there. Its record is made sure of.
                    let document = &mut self.tabs[i].scene.document;
                    let mut table = TopologyTable::load(document);
                    if table.connector_by_handle(handle).is_none() {
                        if let Some(record) = record_of(&mut table, handle, kind, line_tag) {
                            record.peer_drawing = said.target_drawing.clone();
                        }
                        table.store(document);
                        self.tabs[i].dirty = true;
                    }
                    return Ok(vec![handle]);
                }
                let at = said.at.ok_or_else(name)?;
                if line_entity(&self.tabs[i].scene.document, kind.host_role(), line_tag).is_none() {
                    return Err(format!("{line_tag}（连接符所在的线不在本图里）"));
                }
                let mut circle = document::entities::Circle::new();
                circle.center = document::types::Vector3::new(at[0], at[1], 0.0);
                circle.radius = RADIUS;
                let handle = self.tabs[i].scene.add_entity(EntityType::Circle(circle));
                if handle.is_null() {
                    return Err(name());
                }
                let document = &mut self.tabs[i].scene.document;
                PidAuthor {
                    type_code: kind.type_code().to_owned(),
                    tag: String::new(),
                    tag_source: "auto".to_owned(),
                    tag_status: "ok".to_owned(),
                    topology: kind.role().to_owned(),
                    props: "{}".to_owned(),
                    ..PidAuthor::default()
                }
                .store(document, handle);
                let mut table = TopologyTable::load(document);
                table.prune(document);
                let mut record =
                    ConnectorRecord::new(handle_text(handle), kind, line_tag.to_owned());
                record.peer_drawing = said.target_drawing.clone();
                record.upstream = Some(line_tag.to_owned());
                table.upsert_connector(record);
                table.store(document);
                self.tabs[i].dirty = true;
                applied.handles.push(handle);
                Ok(vec![handle])
            }
            KIND_DELETE_CONNECTOR => {
                let Some(handle) = existing else {
                    // Gone already.
                    return Ok(Vec::new());
                };
                self.tabs[i].scene.erase_entities(&[handle]);
                let document = &mut self.tabs[i].scene.document;
                let mut table = TopologyTable::load(document);
                table.prune(document);
                table.store(document);
                self.tabs[i].dirty = true;
                self.tabs[i].edit_revision = self.tabs[i].edit_revision.wrapping_add(1);
                applied.erased.push(handle);
                Ok(vec![handle])
            }
            KIND_PAIR_CONNECTOR => {
                let handle = existing.ok_or_else(name)?;
                let document = &mut self.tabs[i].scene.document;
                let mut table = TopologyTable::load(document);
                if let Some(record) = record_of(&mut table, handle, kind, line_tag) {
                    match &said.peer {
                        Some((drawing, peer_line, _)) => {
                            record.peer_drawing = Some(drawing.clone());
                            // Head and tail: the line that flows out, then
                            // the line it flows in on.
                            let (up, down) = if kind.is_inflow() {
                                (peer_line.clone(), line_tag.to_owned())
                            } else {
                                (line_tag.to_owned(), peer_line.clone())
                            };
                            record.upstream = Some(up);
                            record.downstream = Some(down);
                        }
                        None => {
                            record.downstream = None;
                            record.upstream = Some(line_tag.to_owned());
                        }
                    }
                }
                table.store(document);
                // On the entity: which drawing, which line, which end. (Not
                // a handle, as between two drawings of one editor: the same
                // connector has another handle in every client's drawing.)
                let peer = said
                    .peer
                    .as_ref()
                    .map(|(drawing, line, connector)| format!("{drawing}:{line}/{connector}"))
                    .unwrap_or_default();
                if let Some(mut author) = PidAuthor::load(document, handle) {
                    if author.connector_peer != peer {
                        author.connector_peer = peer;
                        author.store(document, handle);
                    }
                }
                self.tabs[i].dirty = true;
                applied.handles.push(handle);
                Ok(vec![handle])
            }
            _ => Err(name()),
        }
    }
}
