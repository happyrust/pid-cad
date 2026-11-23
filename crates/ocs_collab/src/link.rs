//! Connectors between drawings: pairing an outflow connector on one drawing
//! with the inflow connector on another, and carrying the fields of the
//! line the outflow stands on to the line the inflow stands on
//! (`VAL-CROSS-009`; the protocol is §3 of
//! `docs/plans/2026-11-12-collab-completion.md` in the editor's repository).
//!
//! A room is one drawing, and a client is in one room. The server is the
//! one party that sees two drawings at once, so pairing and carrying are
//! its work: after an op takes effect on one drawing it commits ops of its
//! own on the drawings that follow from it ([`follow`]). Whoever keeps the
//! drawings — the server, or a test standing in for it — is reached
//! through [`Rooms`].

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::conflict::{
    Change, DrawingModel, KIND_CREATE_CONNECTOR, KIND_DELETE_CONNECTOR, KIND_DELETE_OBJECT,
    KIND_PAIR_CONNECTOR, KIND_SET_PROPERTY,
};

/// The fields of a pipe line that go along a pipe connector.
pub const PIPE_FIELDS: [&str; 5] = ["管道等级", "公称直径", "绝热等级", "流体介质", "管线类型"];
/// The fields of a signal line that go along a signal connector. Never a
/// pipe grade.
pub const SIGNAL_FIELDS: [&str; 2] = ["起点仪表", "终点仪表"];

/// How the session of an op the server made on its own account is named:
/// this, then the drawing whose op it follows from.
pub const LINK_SESSION_PREFIX: &str = "link:";

/// The other end of a paired connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub drawing: String,
    pub line_tag: String,
    pub connector: String,
}

/// One connector of a drawing, as the model keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    /// The line it stands on.
    pub line_tag: String,
    /// `pipe_in`, `pipe_out`, `signal_in` or `signal_out`.
    pub connector: String,
    /// Where on the line, when the op that made it said.
    pub at: Option<[f64; 2]>,
    /// The drawing its other end is said to be on.
    pub target_drawing: Option<String>,
    pub peer: Option<Peer>,
}

impl Connector {
    /// Read a connector out of a `create_connector` entry (or the entry the
    /// model keeps of one). `None` when it does not say which line or what
    /// kind.
    pub fn read(raw: &Value) -> Option<Self> {
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let connector = text("connector")?;
        if !matches!(
            connector.as_str(),
            "pipe_in" | "pipe_out" | "signal_in" | "signal_out"
        ) {
            return None;
        }
        let at = raw.get("at").and_then(Value::as_array).and_then(|pair| {
            let number = |index: usize| pair.get(index)?.as_f64().filter(|n| n.is_finite());
            Some([number(0)?, number(1)?])
        });
        let peer = match (text("peer_drawing"), text("peer_line_tag"), text("peer_connector")) {
            (Some(drawing), Some(line_tag), Some(connector)) => Some(Peer {
                drawing,
                line_tag,
                connector,
            }),
            _ => None,
        };
        Some(Self {
            line_tag: text("line_tag")?,
            connector,
            at,
            target_drawing: text("target_drawing"),
            peer,
        })
    }

    pub fn is_inflow(&self) -> bool {
        self.connector.ends_with("_in")
    }

    /// `pipe` or `signal`: the two do not pair with each other.
    pub fn family(&self) -> &str {
        self.connector.split('_').next().unwrap_or_default()
    }

    /// The fields that go along a connector of this family.
    pub fn carried_fields(&self) -> &'static [&'static str] {
        if self.family() == "signal" {
            &SIGNAL_FIELDS
        } else {
            &PIPE_FIELDS
        }
    }

    /// Whether `other` is this connector: the same kind on the same line —
    /// and at the same place, when both say where they are.
    pub fn is(&self, other: &Connector) -> bool {
        self.line_tag == other.line_tag
            && self.connector == other.connector
            && match (self.at, other.at) {
                (Some(a), Some(b)) => (a[0] - b[0]).abs() <= 1e-6 && (a[1] - b[1]).abs() <= 1e-6,
                _ => true,
            }
    }

    /// The `pair_connector` that tells this connector's drawing of `peer`
    /// (`None`: it has none any more).
    fn paired_with(&self, peer: Option<&Peer>) -> Value {
        let mut out = json!({
            "kind": KIND_PAIR_CONNECTOR,
            "line_tag": self.line_tag,
            "connector": self.connector,
            "peer_drawing": peer.map(|peer| peer.drawing.as_str()),
            "peer_line_tag": peer.map(|peer| peer.line_tag.as_str()),
            "peer_connector": peer.map(|peer| peer.connector.as_str()),
        });
        if let Some(at) = self.at {
            out["at"] = json!(at);
        }
        out
    }
}

/// Whoever keeps the drawings, as far as following a connector goes.
pub trait Rooms {
    /// Every drawing there is, by id.
    fn drawings(&mut self) -> Vec<String>;
    /// What is known of one drawing.
    fn model(&mut self, drawing_id: &str) -> &DrawingModel;
    /// Commit an op of the server's own on `drawing_id`, on account of an
    /// op on drawing `cause`, and tell the drawing's room. The revision it
    /// took, or `None` when it could not be committed.
    fn commit(&mut self, drawing_id: &str, op_id: &str, cause: &str, changes: Vec<Value>)
        -> Option<i64>;
}

fn set_property(line_tag: &str, field: &str, value: &str) -> Value {
    json!({"kind": KIND_SET_PROPERTY, "object_ref": line_tag, "field": field, "value": value})
}

/// What follows from an op that took effect on `drawing_id` at `revision`
/// with `changes` (as the room was told them): connectors it made are
/// paired with their other ends, a connector it took away leaves its other
/// end unpaired, and a field it wrote on a line that has a paired outflow
/// connector is written on the line at the other end — and on from there,
/// if that line flows out again.
///
/// Returns every `(drawing, revision)` it committed, in order.
pub fn follow(
    rooms: &mut dyn Rooms,
    drawing_id: &str,
    revision: i64,
    changes: &[Value],
) -> Vec<(String, i64)> {
    let mut moved = Vec::new();
    let mut written = Vec::new();
    for (index, raw) in changes.iter().enumerate() {
        let Some(change) = Change::list(raw).into_iter().next() else {
            continue;
        };
        // One op of the server's per change that calls for one; told apart
        // by where the change stood in the op.
        let stem = if index == 0 {
            format!("{drawing_id}-{revision}")
        } else {
            format!("{drawing_id}-{revision}-{index}")
        };
        match change.kind.as_str() {
            KIND_CREATE_CONNECTOR => pair(rooms, drawing_id, &stem, &change.raw, &mut moved),
            KIND_DELETE_CONNECTOR => unpair(rooms, drawing_id, &stem, &change.raw, &mut moved),
            KIND_DELETE_OBJECT => {
                unpair_line(rooms, drawing_id, &stem, &change.object_ref, &mut moved)
            }
            KIND_SET_PROPERTY => {
                written.push((change.object_ref.clone(), change.field.clone(), change.value.clone()))
            }
            _ => {}
        }
    }
    let mut seen = BTreeSet::from([drawing_id.to_owned()]);
    carry(
        rooms,
        drawing_id,
        &format!("{drawing_id}-{revision}"),
        &written,
        &mut seen,
        &mut moved,
    );
    moved
}

/// Find the other end of the connector `raw` made on `drawing_id`, and
/// pair the two.
fn pair(
    rooms: &mut dyn Rooms,
    drawing_id: &str,
    stem: &str,
    raw: &Value,
    moved: &mut Vec<(String, i64)>,
) {
    let Some(said) = Connector::read(raw) else {
        return;
    };
    // As the model has it: there, and not paired yet.
    let Some(mine) = rooms
        .model(drawing_id)
        .connectors()
        .into_iter()
        .find(|known| known.is(&said) && known.peer.is_none())
    else {
        return;
    };
    let known_drawings: BTreeSet<String> = rooms.drawings().into_iter().collect();
    let candidates: Vec<String> = match (&mine.target_drawing, mine.is_inflow()) {
        (Some(target), _) => {
            if known_drawings.contains(target) {
                vec![target.clone()]
            } else {
                Vec::new()
            }
        }
        // An inflow that does not say where it comes from is looked for
        // from every drawing; the lowest id that flows out to here has it.
        (None, true) => known_drawings.into_iter().collect(),
        // An outflow to nowhere in particular pairs with nothing.
        (None, false) => Vec::new(),
    };
    for other in candidates {
        if other == drawing_id {
            continue;
        }
        let theirs = rooms.model(&other).connectors().into_iter().find(|known| {
            known.peer.is_none()
                && known.family() == mine.family()
                && known.is_inflow() != mine.is_inflow()
                && known.line_tag == mine.line_tag
                && if known.is_inflow() {
                    // An inflow may say where it expects to come from.
                    known.target_drawing.as_deref().is_none_or(|from| from == drawing_id)
                } else {
                    known.target_drawing.as_deref() == Some(drawing_id)
                }
        });
        let Some(theirs) = theirs else {
            continue;
        };
        let to_them = Peer {
            drawing: other.clone(),
            line_tag: theirs.line_tag.clone(),
            connector: theirs.connector.clone(),
        };
        let to_me = Peer {
            drawing: drawing_id.to_owned(),
            line_tag: mine.line_tag.clone(),
            connector: mine.connector.clone(),
        };
        // What the line flowing out carries now goes to the line flowing in.
        let (source, source_line, target_line) = if mine.is_inflow() {
            (other.as_str(), theirs.line_tag.as_str(), mine.line_tag.as_str())
        } else {
            (drawing_id, mine.line_tag.as_str(), theirs.line_tag.as_str())
        };
        let target_drawing = if mine.is_inflow() {
            drawing_id
        } else {
            other.as_str()
        };
        let carried: Vec<Written> = {
            let model = rooms.model(source);
            mine.carried_fields()
                .iter()
                .filter_map(|field| {
                    let value = model.field(source_line, field)?.value.clone();
                    (!value.is_empty()).then(|| (target_line.to_owned(), (*field).to_owned(), value))
                })
                .collect()
        };
        // #90: outflow still wins, but a different value already on the
        // inflow line is named on the change so the audit can keep it.
        let values: Vec<Value> = {
            let model = rooms.model(target_drawing);
            carried
                .iter()
                .map(|(line, field, value)| {
                    let mut entry = set_property(line, field, value);
                    if let Some(old) = model.field(line, field).map(|known| known.value.clone()) {
                        if !old.is_empty() && old != *value {
                            entry["replaced"] = json!(old);
                        }
                    }
                    entry
                })
                .collect()
        };
        let mut here = vec![mine.paired_with(Some(&to_them))];
        let mut there = vec![theirs.paired_with(Some(&to_me))];
        if mine.is_inflow() {
            here.extend(values);
        } else {
            there.extend(values);
        }
        let op_id = format!("pair-{stem}");
        let mine_at = rooms.commit(drawing_id, &op_id, drawing_id, here);
        let theirs_at = rooms.commit(&other, &op_id, drawing_id, there);
        moved.extend(mine_at.map(|revision| (drawing_id.to_owned(), revision)));
        moved.extend(theirs_at.map(|revision| (other.clone(), revision)));
        // On from the drawing the values arrived on, if its line flows out
        // again.
        let (arrived, arrived_at) = if mine.is_inflow() {
            (drawing_id, mine_at)
        } else {
            (other.as_str(), theirs_at)
        };
        if let Some(revision) = arrived_at {
            let mut seen = BTreeSet::from([source.to_owned(), arrived.to_owned()]);
            carry(
                rooms,
                arrived,
                &format!("{arrived}-{revision}"),
                &carried,
                &mut seen,
                moved,
            );
        }
        return;
    }
}

/// The connector `raw` names is gone from `drawing_id`: whichever
/// connector was paired with it is told so.
fn unpair(
    rooms: &mut dyn Rooms,
    drawing_id: &str,
    stem: &str,
    raw: &Value,
    moved: &mut Vec<(String, i64)>,
) {
    let Some(gone) = Connector::read(raw) else {
        return;
    };
    let mut all = rooms.drawings();
    all.sort();
    all.dedup();
    for other in all {
        if other == drawing_id {
            continue;
        }
        let left = rooms.model(&other).connectors().into_iter().find(|known| {
            known.peer.as_ref().is_some_and(|peer| {
                peer.drawing == drawing_id
                    && peer.line_tag == gone.line_tag
                    && peer.connector == gone.connector
            })
        });
        if let Some(left) = left {
            let op_id = format!("pair-{stem}");
            let at = rooms.commit(&other, &op_id, drawing_id, vec![left.paired_with(None)]);
            moved.extend(at.map(|revision| (other.clone(), revision)));
        }
    }
}

/// #31: An entire pipe/signal line was deleted on `drawing_id`: every connector
/// on another drawing that was paired with any connector on `line_tag` of
/// `drawing_id` returns to unpaired.
fn unpair_line(
    rooms: &mut dyn Rooms,
    drawing_id: &str,
    stem: &str,
    line_tag: &str,
    moved: &mut Vec<(String, i64)>,
) {
    if line_tag.is_empty() {
        return;
    }
    let mut all = rooms.drawings();
    all.sort();
    all.dedup();
    for other in all {
        if other == drawing_id {
            continue;
        }
        let affected: Vec<Connector> = rooms
            .model(&other)
            .connectors()
            .into_iter()
            .filter(|known| {
                known.peer.as_ref().is_some_and(|peer| {
                    peer.drawing == drawing_id && peer.line_tag == line_tag
                })
            })
            .collect();
        if !affected.is_empty() {
            let changes: Vec<Value> = affected.iter().map(|c| c.paired_with(None)).collect();
            let op_id = format!("pair-{stem}");
            let at = rooms.commit(&other, &op_id, drawing_id, changes);
            moved.extend(at.map(|revision| (other.clone(), revision)));
        }
    }
}

/// Why an off-page connector is currently unpaired (#32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpairedReason {
    /// Target drawing is not registered on the server.
    TargetDrawingUnregistered,
    /// Target drawing exists, but has no opposite-direction same-family
    /// connector on the same line tag.
    NoMatchingConnector,
    /// Target drawing has matching opposite-direction connector(s) on the same
    /// line tag, but all of them are already paired with another connector.
    TargetAlreadyPaired,
}

impl UnpairedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnpairedReason::TargetDrawingUnregistered => "target_drawing_unregistered",
            UnpairedReason::NoMatchingConnector => "no_matching_connector",
            UnpairedReason::TargetAlreadyPaired => "target_already_paired",
        }
    }

    pub fn message(self, is_inflow: bool) -> &'static str {
        match self {
            UnpairedReason::TargetDrawingUnregistered => "目标图纸没有登记",
            UnpairedReason::NoMatchingConnector => {
                if is_inflow {
                    "目标图纸上没有同一线号的流出连接符"
                } else {
                    "目标图纸上没有同一线号的流入连接符"
                }
            }
            UnpairedReason::TargetAlreadyPaired => {
                if is_inflow {
                    "目标图纸上同一线号的流出连接符已经另有配对"
                } else {
                    "目标图纸上同一线号的流入连接符已经另有配对"
                }
            }
        }
    }
}

/// One unpaired off-page connector on a drawing, with the reason it failed to
/// pair (#32).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnpairedConnector {
    pub line_tag: String,
    pub connector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_drawing: Option<String>,
    pub reason_code: UnpairedReason,
    pub reason: String,
}

/// Lists every unpaired off-page connector on `drawing_id` along with the exact
/// reason it has not paired (#32). Paired connectors do not appear.
pub fn unpaired_connectors(rooms: &mut dyn Rooms, drawing_id: &str) -> Vec<UnpairedConnector> {
    let known_drawings: BTreeSet<String> = rooms.drawings().into_iter().collect();
    let mine: Vec<Connector> = rooms
        .model(drawing_id)
        .connectors()
        .into_iter()
        .filter(|c| c.peer.is_none())
        .collect();
    let mut out = Vec::with_capacity(mine.len());
    for conn in mine {
        let reason_code = match &conn.target_drawing {
            Some(target) => {
                if !known_drawings.contains(target) {
                    UnpairedReason::TargetDrawingUnregistered
                } else {
                    let matches: Vec<Connector> = rooms
                        .model(target)
                        .connectors()
                        .into_iter()
                        .filter(|other| {
                            other.family() == conn.family()
                                && other.is_inflow() != conn.is_inflow()
                                && other.line_tag == conn.line_tag
                        })
                        .collect();
                    if matches.is_empty() {
                        UnpairedReason::NoMatchingConnector
                    } else if matches.iter().all(|m| m.peer.is_some()) {
                        UnpairedReason::TargetAlreadyPaired
                    } else {
                        UnpairedReason::NoMatchingConnector
                    }
                }
            }
            None => {
                if conn.is_inflow() {
                    // An inflow without explicit target_drawing scans all other known drawings.
                    let mut saw_paired = false;
                    for other in &known_drawings {
                        if other == drawing_id {
                            continue;
                        }
                        for candidate in rooms.model(other).connectors() {
                            if candidate.family() == conn.family()
                                && !candidate.is_inflow()
                                && candidate.line_tag == conn.line_tag
                                && candidate.target_drawing.as_deref() == Some(drawing_id)
                                && candidate.peer.is_some()
                            {
                                saw_paired = true;
                            }
                        }
                    }
                    if saw_paired {
                        UnpairedReason::TargetAlreadyPaired
                    } else {
                        UnpairedReason::NoMatchingConnector
                    }
                } else {
                    UnpairedReason::TargetDrawingUnregistered
                }
            }
        };
        let is_inflow = conn.is_inflow();
        out.push(UnpairedConnector {
            line_tag: conn.line_tag,
            connector: conn.connector,
            at: conn.at,
            target_drawing: conn.target_drawing,
            reason_code,
            reason: reason_code.message(is_inflow).to_owned(),
        });
    }
    out
}

/// A field written on a line: `(line, field, value)`.
type Written = (String, String, String);

/// Fields written on lines of `drawing_id` — `(line, field, value)` — go
/// to the line at the other end of every paired outflow connector on those
/// lines, one op per drawing they arrive on, and on from there. `seen` is
/// the drawings the values have been to: a ring of connectors is gone round
/// once.
fn carry(
    rooms: &mut dyn Rooms,
    drawing_id: &str,
    stem: &str,
    written: &[Written],
    seen: &mut BTreeSet<String>,
    moved: &mut Vec<(String, i64)>,
) {
    if written.is_empty() {
        return;
    }
    let outflows: Vec<Connector> = rooms
        .model(drawing_id)
        .connectors()
        .into_iter()
        .filter(|known| !known.is_inflow() && known.peer.is_some())
        .collect();
    // drawing → what arrives on it.
    let mut arriving: Vec<(String, Vec<Written>)> = Vec::new();
    for (line, field, value) in written {
        for outflow in outflows.iter().filter(|outflow| &outflow.line_tag == line) {
            if !outflow.carried_fields().contains(&field.as_str()) {
                continue;
            }
            let Some(peer) = &outflow.peer else {
                continue;
            };
            if seen.contains(&peer.drawing) {
                continue;
            }
            let entry = (peer.line_tag.clone(), field.clone(), value.clone());
            match arriving.iter_mut().find(|(drawing, _)| drawing == &peer.drawing) {
                Some((_, list)) if !list.contains(&entry) => list.push(entry),
                Some(_) => {}
                None => arriving.push((peer.drawing.clone(), vec![entry])),
            }
        }
    }
    for (drawing, _) in &arriving {
        seen.insert(drawing.clone());
    }
    for (drawing, list) in arriving {
        let changes: Vec<Value> = list
            .iter()
            .map(|(line, field, value)| set_property(line, field, value))
            .collect();
        let op_id = format!("carry-{stem}");
        if let Some(revision) = rooms.commit(&drawing, &op_id, drawing_id, changes) {
            moved.push((drawing.clone(), revision));
            carry(
                rooms,
                &drawing,
                &format!("{drawing}-{revision}"),
                &list,
                seen,
                moved,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Drawings kept in memory: each a model and the ops committed on it.
    #[derive(Default)]
    struct Desk {
        models: BTreeMap<String, DrawingModel>,
        /// `(drawing, op_id, session, changes)` of every op of the desk's own.
        made: Vec<(String, String, String, Vec<Value>)>,
    }

    impl Desk {
        /// An op of a client's takes effect on `drawing`, and what follows
        /// from it is followed.
        fn take(&mut self, drawing: &str, session: &str, changes: Vec<Value>) -> Vec<(String, i64)> {
            let model = self.models.entry(drawing.to_owned()).or_default();
            let revision = model.revision() + 1;
            let mut told = Vec::new();
            for raw in &changes {
                let change = Change::list(raw).into_iter().next().expect("a change");
                let object_ref = match model.judge(&change, revision - 1, session) {
                    crate::conflict::Verdict::Apply { object_ref, .. } => object_ref,
                    other => panic!("refused: {other:?}"),
                };
                model.commit(&change, &object_ref, revision, session, "u");
                told.push(change.broadcast_form(&object_ref));
            }
            follow(self, drawing, revision, &told)
        }

        fn value(&self, drawing: &str, line: &str, field: &str) -> Option<String> {
            Some(self.models.get(drawing)?.field(line, field)?.value.clone())
        }

        fn connector(&self, drawing: &str, index: usize) -> Connector {
            self.models[drawing].connectors()[index].clone()
        }
    }

    impl Rooms for Desk {
        fn drawings(&mut self) -> Vec<String> {
            self.models.keys().cloned().collect()
        }

        fn model(&mut self, drawing_id: &str) -> &DrawingModel {
            self.models.entry(drawing_id.to_owned()).or_default()
        }

        fn commit(
            &mut self,
            drawing_id: &str,
            op_id: &str,
            cause: &str,
            changes: Vec<Value>,
        ) -> Option<i64> {
            let session = format!("{LINK_SESSION_PREFIX}{cause}");
            let model = self.models.entry(drawing_id.to_owned()).or_default();
            let revision = model.revision() + 1;
            for raw in &changes {
                let change = Change::list(raw).into_iter().next().expect("a change");
                let name = if change.object_ref.is_empty() {
                    change.text("line_tag").unwrap_or_default()
                } else {
                    change.object_ref.clone()
                };
                model.commit(&change, &name, revision, &session, "u");
            }
            self.made
                .push((drawing_id.to_owned(), op_id.to_owned(), session, changes));
            Some(revision)
        }
    }

    fn outflow(line: &str, to: &str) -> Value {
        json!({"kind": KIND_CREATE_CONNECTOR, "line_tag": line, "connector": "pipe_out",
               "at": [120.0, 0.0], "target_drawing": to})
    }

    fn inflow(line: &str) -> Value {
        json!({"kind": KIND_CREATE_CONNECTOR, "line_tag": line, "connector": "pipe_in", "at": [20.0, 0.0]})
    }

    fn write(line: &str, field: &str, value: &str) -> Value {
        set_property(line, field, value)
    }

    /// `VAL-CROSS-009`: an outflow on one drawing that names another, and
    /// the inflow on that other for the same line — whichever comes second
    /// pairs the two, and each drawing's room is told of its own end.
    #[test]
    fn collab_connector_cross_drawing_an_outflow_and_the_inflow_it_names_are_paired() {
        for outflow_first in [true, false] {
            let mut desk = Desk::default();
            let moved = if outflow_first {
                assert!(desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")]).is_empty());
                desk.take("PID-1102", "s-2", vec![inflow("L-1001")])
            } else {
                assert!(desk.take("PID-1102", "s-2", vec![inflow("L-1001")]).is_empty());
                desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")])
            };
            let mut drawings: Vec<&str> = moved.iter().map(|(drawing, _)| drawing.as_str()).collect();
            drawings.sort();
            assert_eq!(drawings, ["PID-1101", "PID-1102"], "one op on each drawing");
            let out = desk.connector("PID-1101", 0);
            let inn = desk.connector("PID-1102", 0);
            assert_eq!(
                out.peer,
                Some(Peer {
                    drawing: "PID-1102".into(),
                    line_tag: "L-1001".into(),
                    connector: "pipe_in".into()
                })
            );
            assert_eq!(
                inn.peer,
                Some(Peer {
                    drawing: "PID-1101".into(),
                    line_tag: "L-1001".into(),
                    connector: "pipe_out".into()
                })
            );
            assert_eq!(desk.made.len(), 2);
            assert!(desk.made.iter().all(|(_, op_id, session, changes)| {
                op_id.starts_with("pair-")
                    && session.starts_with(LINK_SESSION_PREFIX)
                    && changes[0]["kind"] == KIND_PAIR_CONNECTOR
            }));
        }
    }

    /// #90: pairing still copies the outflow onto the inflow, and names the
    /// inflow value it replaced.
    #[test]
    fn pairing_names_the_inflow_value_it_replaces() {
        let mut desk = Desk::default();
        desk.take("PID-1102", "s-2", vec![write("L-1001", "公称直径", "DN80")]);
        desk.take("PID-1101", "s-1", vec![write("L-1001", "公称直径", "DN50")]);
        desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")]);
        desk.take("PID-1102", "s-2", vec![inflow("L-1001")]);
        assert_eq!(desk.value("PID-1102", "L-1001", "公称直径").as_deref(), Some("DN50"));
        let replaced = desk.made.iter().find_map(|(_, _, _, changes)| {
            changes.iter().find(|change| change.get("replaced").is_some()).cloned()
        });
        let replaced = replaced.expect("the carried change names what it replaced");
        assert_eq!(replaced["replaced"], "DN80");
        assert_eq!(replaced["value"], "DN50");
        assert_eq!(replaced["object_ref"], "L-1001");
    }

    /// What the line flowing out carries when the two are paired goes to
    /// the line flowing in with the pairing; what is written on it later
    /// goes after — and only the fields that go along a connector.
    #[test]
    fn collab_connector_cross_drawing_the_outflow_lines_fields_arrive_on_the_inflow_line() {
        let mut desk = Desk::default();
        desk.take("PID-1101", "s-1", vec![write("L-1001", "公称直径", "DN50")]);
        desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")]);
        assert_eq!(desk.value("PID-1102", "L-1001", "公称直径"), None, "not paired yet");
        desk.take("PID-1102", "s-2", vec![inflow("L-1001")]);
        assert_eq!(
            desk.value("PID-1102", "L-1001", "公称直径").as_deref(),
            Some("DN50"),
            "carried with the pairing"
        );

        let moved = desk.take("PID-1101", "s-1", vec![write("L-1001", "管道等级", "A1A")]);
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].0, "PID-1102");
        assert_eq!(desk.value("PID-1102", "L-1001", "管道等级").as_deref(), Some("A1A"));
        let (drawing, op_id, session, changes) = desk.made.last().expect("an op");
        assert_eq!(drawing, "PID-1102");
        assert!(op_id.starts_with("carry-PID-1101-"), "{op_id}");
        assert_eq!(session, "link:PID-1101");
        assert_eq!(changes, &[write("L-1001", "管道等级", "A1A")]);

        // A field that does not go along a connector stays where it is, and
        // so does a field of another line.
        let made = desk.made.len();
        assert!(desk.take("PID-1101", "s-1", vec![write("L-1001", "备注", "x")]).is_empty());
        assert!(desk.take("PID-1101", "s-1", vec![write("L-2002", "管道等级", "B1")]).is_empty());
        assert_eq!(desk.made.len(), made);
        // And nothing flows back from the inflow side.
        assert!(desk.take("PID-1102", "s-2", vec![write("L-1001", "管道等级", "C1")]).is_empty());
        assert_eq!(desk.value("PID-1101", "L-1001", "管道等级").as_deref(), Some("A1A"));
    }

    /// The fields the creator's drawing has for the line (`line_props`) are
    /// what the server knows of a line it never heard a write to.
    #[test]
    fn collab_connector_cross_drawing_a_line_the_server_never_heard_of_is_known_by_what_its_connector_says() {
        let mut desk = Desk::default();
        let mut made = outflow("L-1001", "PID-1102");
        made["line_props"] = json!({"管道等级": "A1A", "备注": "x"});
        desk.take("PID-1101", "s-1", vec![made]);
        desk.take("PID-1102", "s-2", vec![inflow("L-1001")]);
        assert_eq!(desk.value("PID-1102", "L-1001", "管道等级").as_deref(), Some("A1A"));
        assert_eq!(desk.value("PID-1102", "L-1001", "备注"), None);
    }

    /// Pipe pairs with pipe and signal with signal; a signal connector
    /// carries the two instruments and never a pipe grade.
    #[test]
    fn collab_connector_cross_drawing_signal_connectors_carry_instruments_only() {
        let mut desk = Desk::default();
        let signal = |connector: &str, to: Option<&str>| {
            let mut made = json!({"kind": KIND_CREATE_CONNECTOR, "line_tag": "S-2001",
                                  "connector": connector, "at": [5.0, 5.0]});
            if let Some(to) = to {
                made["target_drawing"] = json!(to);
            }
            made
        };
        desk.take("A", "s-1", vec![signal("signal_out", Some("B"))]);
        // A pipe inflow on a line of the same tag is not its other end.
        let mut pipe = inflow("S-2001");
        pipe["at"] = json!([9.0, 9.0]);
        assert!(desk.take("B", "s-2", vec![pipe]).is_empty());
        assert_eq!(desk.take("B", "s-2", vec![signal("signal_in", None)]).len(), 2);

        desk.take("A", "s-1", vec![write("S-2001", "起点仪表", "FT-101")]);
        assert_eq!(desk.value("B", "S-2001", "起点仪表").as_deref(), Some("FT-101"));
        assert!(desk.take("A", "s-1", vec![write("S-2001", "管道等级", "A1A")]).is_empty());
        assert_eq!(desk.value("B", "S-2001", "管道等级"), None);
    }

    /// A line that flows in on a drawing and out of it again passes the
    /// value on; a ring of drawings is gone round once.
    #[test]
    fn collab_connector_cross_drawing_a_value_goes_on_through_a_drawing_and_round_a_ring_once() {
        let mut desk = Desk::default();
        let out = |to: &str, x: f64| {
            let mut made = outflow("L-1", to);
            made["at"] = json!([x, 0.0]);
            made
        };
        desk.take("A", "s-a", vec![out("B", 100.0)]);
        desk.take("B", "s-b", vec![inflow("L-1")]);
        desk.take("B", "s-b", vec![out("C", 100.0)]);
        desk.take("C", "s-c", vec![inflow("L-1")]);
        desk.take("C", "s-c", vec![out("A", 100.0)]);
        desk.take("A", "s-a", vec![inflow("L-1")]);

        let moved = desk.take("A", "s-a", vec![write("L-1", "管道等级", "A1A")]);
        let drawings: Vec<&str> = moved.iter().map(|(drawing, _)| drawing.as_str()).collect();
        assert_eq!(drawings, ["B", "C"], "on to B, on to C, and not back to A");
        assert_eq!(desk.value("C", "L-1", "管道等级").as_deref(), Some("A1A"));
    }

    /// A connector taken away leaves its other end unpaired, and nothing is
    /// carried any more; made again, it is paired again.
    #[test]
    fn collab_connector_cross_drawing_a_connector_taken_away_unpairs_its_other_end() {
        let mut desk = Desk::default();
        desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")]);
        desk.take("PID-1102", "s-2", vec![inflow("L-1001")]);
        let gone = json!({"kind": KIND_DELETE_CONNECTOR, "line_tag": "L-1001",
                          "connector": "pipe_out", "at": [120.0, 0.0]});
        let moved = desk.take("PID-1101", "s-1", vec![gone]);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].0, "PID-1102");
        assert!(desk.models["PID-1101"].connectors().is_empty());
        assert_eq!(desk.connector("PID-1102", 0).peer, None);
        assert!(desk.take("PID-1101", "s-1", vec![write("L-1001", "管道等级", "A1A")]).is_empty());

        assert_eq!(desk.take("PID-1101", "s-1", vec![outflow("L-1001", "PID-1102")]).len(), 2);
        assert!(desk.connector("PID-1102", 0).peer.is_some());
    }

    /// A second inflow of the same family on the same line of a drawing is
    /// refused by the judgment; an outflow, or an inflow on another line,
    /// is not.
    #[test]
    fn collab_connector_cross_drawing_a_second_inflow_on_a_line_is_refused() {
        let mut model = DrawingModel::default();
        let first = Change::list(&inflow("L-1001")).remove(0);
        model.commit(&first, "L-1001", 1, "s-1", "u1");
        let mut second = inflow("L-1001");
        second["at"] = json!([60.0, 0.0]);
        let second = Change::list(&second).remove(0);
        assert!(matches!(
            model.judge(&second, 1, "s-2"),
            crate::conflict::Verdict::DuplicateInflow { ref line_tag } if line_tag == "L-1001"
        ));
        let elsewhere = Change::list(&inflow("L-1002")).remove(0);
        assert!(matches!(
            model.judge(&elsewhere, 1, "s-2"),
            crate::conflict::Verdict::Apply { .. }
        ));
        let out = Change::list(&outflow("L-1001", "PID-1102")).remove(0);
        assert!(matches!(model.judge(&out, 1, "s-1"), crate::conflict::Verdict::Apply { .. }));
    }

    /// #31: Deleting an entire pipe line on A removes its connectors and
    /// unpairs the downstream inflow connector on B; recreating the line +
    /// outflow connector re-pairs cleanly.
    #[test]
    fn deleting_pipe_line_unpairs_downstream_connector_and_allows_repairing() {
        let mut desk = Desk::default();
        desk.take(
            "PID-A",
            "s-1",
            vec![json!({"kind": "create_pipe_line", "tag": "L-1001", "type": "PIPE_LINE"})],
        );
        desk.take("PID-A", "s-1", vec![outflow("L-1001", "PID-B")]);
        desk.take("PID-B", "s-2", vec![inflow("L-1001")]);
        assert!(desk.connector("PID-B", 0).peer.is_some());

        // Delete entire pipe line L-1001 on PID-A.
        let moved = desk.take(
            "PID-A",
            "s-1",
            vec![json!({"kind": KIND_DELETE_OBJECT, "object_ref": "L-1001"})],
        );
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].0, "PID-B");
        assert!(desk.models["PID-A"].connectors().is_empty());
        assert_eq!(desk.connector("PID-B", 0).peer, None);

        // Recreate L-1001 and outflow connector on PID-A -> pairs again!
        desk.take(
            "PID-A",
            "s-1",
            vec![json!({"kind": "create_pipe_line", "tag": "L-1001", "type": "PIPE_LINE"})],
        );
        let repaired = desk.take("PID-A", "s-1", vec![outflow("L-1001", "PID-B")]);
        assert_eq!(repaired.len(), 2);
        assert!(desk.connector("PID-B", 0).peer.is_some());
    }

    /// #32: Unpaired connectors report each of the 3 reasons and disappear once paired.
    #[test]
    fn unpaired_connectors_report_all_three_reasons_and_clear_when_paired() {
        let mut desk = Desk::default();
        // Register PID-A, PID-B, PID-C in models; PID-MISSING is unregistered.
        desk.models.entry("PID-B".into()).or_default();
        desk.models.entry("PID-C".into()).or_default();

        // 1. Target drawing not registered.
        desk.take("PID-A", "s-1", vec![outflow("L-UNREG", "PID-MISSING")]);
        // 2. Target drawing registered (PID-B), but has no inflow connector on L-NO-MATCH.
        desk.take("PID-A", "s-1", vec![outflow("L-NO-MATCH", "PID-B")]);
        // 3. Target drawing PID-B has inflow on L-TAKEN already paired with PID-C.
        desk.take("PID-C", "s-3", vec![outflow("L-TAKEN", "PID-B")]);
        desk.take("PID-B", "s-2", vec![inflow("L-TAKEN")]);
        desk.take("PID-A", "s-1", vec![outflow("L-TAKEN", "PID-B")]);

        let list = unpaired_connectors(&mut desk, "PID-A");
        assert_eq!(list.len(), 3, "{list:?}");
        let reason_of = |line: &str| {
            list.iter()
                .find(|u| u.line_tag == line)
                .map(|u| u.reason_code)
        };
        assert_eq!(reason_of("L-UNREG"), Some(UnpairedReason::TargetDrawingUnregistered));
        assert_eq!(reason_of("L-NO-MATCH"), Some(UnpairedReason::NoMatchingConnector));
        assert_eq!(reason_of("L-TAKEN"), Some(UnpairedReason::TargetAlreadyPaired));

        // Once L-NO-MATCH gets its inflow on PID-B, it pairs and disappears from unpaired list!
        desk.take("PID-B", "s-2", vec![inflow("L-NO-MATCH")]);
        let after = unpaired_connectors(&mut desk, "PID-A");
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|u| u.line_tag != "L-NO-MATCH"));
    }
}
