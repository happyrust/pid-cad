//! Pipe lines, runs and what is fitted on them: how a step that made,
//! changed or erased them is said to the room, and how a peer's drawing is
//! brought to what was said (`VAL-CROSS-003`; the protocol is §1 of
//! `docs/plans/2026-11-12-collab-completion.md`).
//!
//! A pipe line is said as the stretches it has. A run is said as what it is
//! like once the step is done — its segments and its fittings, by where
//! they are — and the receiving drawing is aligned to that account: what
//! fits stays, what is missing is made, what is over is erased, and the
//! run's records in the topology table are written again. Aligning twice
//! changes nothing the second time, which is what lets one piece of code
//! follow a fitting put on a run, one erased, an insertion taken back and
//! one put on again.
//!
//! Nothing here goes by handle: an entity a peer made has whatever handle
//! the peer's drawing gave it. A segment is the one of its run with these
//! two ends; a fitting is the one of its run that stands here.

use std::collections::{BTreeMap, BTreeSet};

use document::{Document, EntityType, Handle};
use serde_json::{json, Map, Value};

use super::broadcast::{Applied, Change, KIND_DELETE_OBJECT};
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::{join_props, overrides_of, props_of, split_props};
use crate::pid::topology::{
    handle_text, parse_handle, FittingRecord, PipeSegmentRecord, SegmentEnd, TopologyTable,
    FITTING_BREAK, ROLE_FITTING, ROLE_PIPE_LINE, ROLE_PIPE_RUN, ROLE_PIPE_SEGMENT,
    ROLE_SIGNAL_LINE, TYPE_FITTING, TYPE_PIPE_LINE, TYPE_PIPE_RUN, TYPE_PIPE_SEGMENT,
    TYPE_SIGNAL_LINE, TYPE_VALVE,
};

/// A pipe line or a signal line has these stretches.
pub const KIND_CREATE_PIPE_LINE: &str = "create_pipe_line";
/// A run came into being on a line.
pub const KIND_ADD_PIPE_RUN: &str = "add_pipe_run";
/// A fitting (or a break) was put on a run.
pub const KIND_INSERT_FITTING: &str = "insert_fitting";
/// A run's parts changed in some other way.
pub const KIND_UPDATE_PIPE_RUN: &str = "update_pipe_run";

/// How far apart two coordinates may be and still be the same place.
const SAME_PLACE: f64 = 1e-6;

/// What a pipe statement carries beyond its kind and the tag it is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Pipe {
    /// `create_pipe_line`.
    Line(LineStretches),
    /// `add_pipe_run`, `insert_fitting`, `update_pipe_run`.
    Run(RunStatement),
    /// `create_connector`, `delete_connector`, `pair_connector`
    /// (`super::connectors`).
    Connector(super::connectors::ConnectorSaid),
}

// Coordinates are finite numbers; a `Change` is compared and kept in lists
// of its own, which is all this is for (as for `Placed`).
impl Eq for Pipe {}

/// The stretches a step gave a line, and what they carry.
#[derive(Clone, Debug, PartialEq)]
pub struct LineStretches {
    /// `PIPE_LINE` or `SIGNAL_LINE`.
    pub type_code: String,
    /// Each stretch as `[x1, y1, x2, y2]`.
    pub pieces: Vec<[f64; 4]>,
    pub props: BTreeMap<String, String>,
    pub tag_source: String,
}

/// A run as a step left it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunStatement {
    pub line_tag: String,
    /// For `insert_fitting`: the fitting the step put on the run, as its
    /// type, where it stands and the tag it has of its own.
    pub fitting: Option<(String, [f64; 2], Option<String>)>,
    pub run: RunState,
}

/// The account of a run: its body and its parts, in the table's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunState {
    /// Where the run's body stands.
    pub start: [f64; 2],
    /// What the body carries. Empty for a run said without an account
    /// (`architecture.md` §3.2), whose body then takes the line's values.
    pub props: BTreeMap<String, String>,
    pub overrides: BTreeSet<String>,
    pub segments: Vec<SegmentState>,
    pub fittings: Vec<FittingState>,
}

/// One segment: its two ends, and the fitting port each is on, if any, as
/// the fitting's place in [`RunState::fittings`] and the port's name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SegmentState {
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub start_peer: Option<(usize, String)>,
    pub end_peer: Option<(usize, String)>,
}

/// One fitting of a run: a symbol placed on it, or a break.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FittingState {
    /// The symbol's id, or `break`.
    pub fitting_type: String,
    /// `VALVE` for a fitting with a tag of its own, else `FITTING`.
    pub type_code: String,
    pub tag: Option<String>,
    /// The insertion point; for a break, the middle of its mark.
    pub at: [f64; 2],
    pub block: String,
    pub scale: f64,
    pub rotation: f64,
    pub props: BTreeMap<String, String>,
    pub tag_source: String,
    /// The two ends of a break's mark.
    pub mark: Option<[[f64; 2]; 2]>,
    /// Port name → the segment on it, as its place in
    /// [`RunState::segments`].
    pub ports: BTreeMap<String, Option<usize>>,
}

fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() <= SAME_PLACE && (a[1] - b[1]).abs() <= SAME_PLACE
}

// ── on the wire ──────────────────────────────────────────────────────────

fn pair(point: [f64; 2]) -> Value {
    json!([point[0], point[1]])
}

fn text_map(map: &BTreeMap<String, String>) -> Value {
    Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), json!(value)))
            .collect(),
    )
}

impl RunState {
    fn to_json(&self) -> Value {
        let peer = |peer: &Option<(usize, String)>| match peer {
            Some((fitting, port)) => json!(format!("{fitting}:{port}")),
            None => Value::Null,
        };
        let segments: Vec<Value> = self
            .segments
            .iter()
            .map(|segment| {
                let mut out = json!({"start": pair(segment.start), "end": pair(segment.end)});
                if segment.start_peer.is_some() {
                    out["start_peer"] = peer(&segment.start_peer);
                }
                if segment.end_peer.is_some() {
                    out["end_peer"] = peer(&segment.end_peer);
                }
                out
            })
            .collect();
        let fittings: Vec<Value> = self
            .fittings
            .iter()
            .map(|fitting| {
                let mut out = json!({
                    "fitting_type": fitting.fitting_type,
                    "type": fitting.type_code,
                    "at": pair(fitting.at),
                    "ports": fitting.ports.iter().map(|(port, segment)| (port.clone(), json!(segment)))
                        .collect::<Map<String, Value>>(),
                });
                if let Some(tag) = &fitting.tag {
                    out["tag"] = json!(tag);
                }
                match fitting.mark {
                    Some([from, to]) => out["mark"] = json!([pair(from), pair(to)]),
                    None => {
                        out["block"] = json!(fitting.block);
                        out["scale"] = json!(fitting.scale);
                        out["rotation"] = json!(fitting.rotation);
                        out["props"] = text_map(&fitting.props);
                        out["tag_source"] = json!(fitting.tag_source);
                    }
                }
                out
            })
            .collect();
        json!({
            "start": pair(self.start),
            "props": text_map(&self.props),
            "overrides": self.overrides.iter().collect::<Vec<_>>(),
            "segments": segments,
            "fittings": fittings,
        })
    }

    /// The points of the run when its segments follow one another end to
    /// start, as `architecture.md` §3.2 writes a run.
    fn chain(&self) -> Option<Vec<[f64; 2]>> {
        let first = self.segments.first()?;
        let mut points = vec![first.start, first.end];
        for segment in &self.segments[1..] {
            if !same(*points.last()?, segment.start) {
                return None;
            }
            points.push(segment.end);
        }
        Some(points)
    }
}

/// The JSON body of a pipe statement.
pub(super) fn to_json(change: &Change, pipe: &Pipe) -> Value {
    match pipe {
        Pipe::Line(line) => {
            let mut out = json!({
                "kind": change.kind,
                "tag": change.object_ref,
                "type": line.type_code,
                "pieces": line.pieces,
                "props": text_map(&line.props),
                "tag_source": line.tag_source,
            });
            // The stretches as one polyline, when they are one.
            let mut points: Vec<[f64; 2]> = Vec::new();
            let chained = line.pieces.iter().all(|piece| {
                let (from, to) = ([piece[0], piece[1]], [piece[2], piece[3]]);
                let joins = points.last().is_none_or(|last| same(*last, from));
                if points.is_empty() {
                    points.push(from);
                }
                points.push(to);
                joins
            });
            if chained && !points.is_empty() {
                out["points"] = json!(points);
            }
            out
        }
        Pipe::Run(said) => {
            let mut out =
                json!({"kind": change.kind, "line_tag": said.line_tag, "run": said.run.to_json()});
            if change.kind == KIND_ADD_PIPE_RUN {
                out["tag"] = json!(change.object_ref);
                if let Some(points) = said.run.chain() {
                    out["points"] = json!(points);
                }
            } else {
                out["run_tag"] = json!(change.object_ref);
            }
            if let Some((fitting_type, at, tag)) = &said.fitting {
                out["fitting_type"] = json!(fitting_type);
                out["at"] = pair(*at);
                if let Some(tag) = tag {
                    out["tag"] = json!(tag);
                }
            }
            out
        }
        Pipe::Connector(said) => super::connectors::to_json(change, said),
    }
}

fn point(value: &Value) -> Option<[f64; 2]> {
    let list = value.as_array()?;
    let number = |entry: Option<&Value>| entry?.as_f64().filter(|n| n.is_finite());
    Some([number(list.first())?, number(list.get(1))?])
}

fn strings(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let text = match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    (key.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn word(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn read_peer(value: &Value) -> Option<(usize, String)> {
    let (index, port) = value.as_str()?.split_once(':')?;
    Some((index.parse().ok()?, port.to_owned()))
}

fn read_run(value: &Value) -> Option<RunState> {
    let object = value.as_object()?;
    let segments = object
        .get("segments")?
        .as_array()?
        .iter()
        .map(|entry| {
            Some(SegmentState {
                start: point(&entry["start"])?,
                end: point(&entry["end"])?,
                start_peer: read_peer(&entry["start_peer"]),
                end_peer: read_peer(&entry["end_peer"]),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let fittings = object
        .get("fittings")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|entry| {
                    let fitting_type = word(&entry["fitting_type"])?;
                    let mark = match entry["mark"].as_array() {
                        Some(ends) => Some([point(ends.first()?)?, point(ends.get(1)?)?]),
                        None => None,
                    };
                    // A break is its mark; anything else is a symbol's block.
                    let block = entry["block"].as_str().unwrap_or_default().to_owned();
                    if mark.is_none() && block.is_empty() {
                        return None;
                    }
                    Some(FittingState {
                        fitting_type,
                        type_code: entry["type"].as_str().unwrap_or(TYPE_FITTING).to_owned(),
                        tag: word(&entry["tag"]),
                        at: point(&entry["at"])?,
                        block,
                        scale: entry["scale"]
                            .as_f64()
                            .filter(|scale| *scale != 0.0)
                            .unwrap_or(1.0),
                        rotation: entry["rotation"].as_f64().unwrap_or(0.0),
                        props: strings(&entry["props"]),
                        tag_source: entry["tag_source"].as_str().unwrap_or("auto").to_owned(),
                        mark,
                        ports: entry["ports"]
                            .as_object()
                            .map(|ports| {
                                ports
                                    .iter()
                                    .map(|(port, segment)| {
                                        (port.clone(), segment.as_u64().map(|index| index as usize))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_else(|| Some(Vec::new()))?;
    // Every reference stays inside the account.
    let fitting_ok = |peer: &Option<(usize, String)>| {
        peer.as_ref()
            .is_none_or(|(index, _)| *index < fittings.len())
    };
    if !segments
        .iter()
        .all(|segment| fitting_ok(&segment.start_peer) && fitting_ok(&segment.end_peer))
        || !fittings.iter().all(|fitting| {
            fitting
                .ports
                .values()
                .all(|segment| segment.is_none_or(|index| index < segments.len()))
        })
    {
        return None;
    }
    Some(RunState {
        start: object
            .get("start")
            .and_then(point)
            .or_else(|| segments.first().map(|segment| segment.start))?,
        props: object.get("props").map(strings).unwrap_or_default(),
        overrides: object
            .get("overrides")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|entry| entry.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        segments,
        fittings,
    })
}

/// Read a pipe statement out of an `op_applied` `changes` entry. `None`
/// when `kind` is not one of them, or the entry does not say enough to act
/// on. The server rewrites `object_ref` on the three that change a line's
/// connections (it becomes the line's tag), so it is never read here.
pub(super) fn from_json(kind: &str, value: &Value) -> Option<Change> {
    let statement = |object_ref: String, pipe: Pipe| Change {
        kind: kind.to_owned(),
        object_ref,
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: Some(pipe),
        tag_source: None,
        overrides: None,
    };
    match kind {
        KIND_CREATE_PIPE_LINE => {
            let tag = word(&value["tag"]).or_else(|| word(&value["object_ref"]))?;
            let pieces: Vec<[f64; 4]> = match value["pieces"].as_array() {
                Some(list) => list
                    .iter()
                    .map(|piece| {
                        let numbers = piece.as_array()?;
                        let at =
                            |index: usize| numbers.get(index)?.as_f64().filter(|n| n.is_finite());
                        Some([at(0)?, at(1)?, at(2)?, at(3)?])
                    })
                    .collect::<Option<Vec<_>>>()?,
                None => {
                    let points = value["points"]
                        .as_array()?
                        .iter()
                        .map(point)
                        .collect::<Option<Vec<_>>>()?;
                    points
                        .windows(2)
                        .map(|ends| [ends[0][0], ends[0][1], ends[1][0], ends[1][1]])
                        .collect()
                }
            };
            if pieces.is_empty() {
                return None;
            }
            Some(statement(
                tag,
                Pipe::Line(LineStretches {
                    type_code: value["type"].as_str().unwrap_or(TYPE_PIPE_LINE).to_owned(),
                    pieces,
                    props: strings(&value["props"]),
                    tag_source: value["tag_source"].as_str().unwrap_or("auto").to_owned(),
                }),
            ))
        }
        KIND_ADD_PIPE_RUN => {
            let tag = word(&value["tag"])?;
            let line_tag = word(&value["line_tag"])?;
            let run = match value.get("run").filter(|run| !run.is_null()) {
                Some(account) => read_run(account)?,
                // As `architecture.md` §3.2 writes a run: its points.
                None => {
                    let points = value["points"]
                        .as_array()?
                        .iter()
                        .map(point)
                        .collect::<Option<Vec<_>>>()?;
                    RunState {
                        start: *points.first()?,
                        segments: points
                            .windows(2)
                            .map(|ends| SegmentState {
                                start: ends[0],
                                end: ends[1],
                                ..SegmentState::default()
                            })
                            .collect(),
                        ..RunState::default()
                    }
                }
            };
            Some(statement(
                tag,
                Pipe::Run(RunStatement {
                    line_tag,
                    fitting: None,
                    run,
                }),
            ))
        }
        KIND_INSERT_FITTING | KIND_UPDATE_PIPE_RUN => {
            let tag = word(&value["run_tag"])?;
            let line_tag = word(&value["line_tag"])?;
            // Without an account of the run there is nothing to align to.
            let run = read_run(value.get("run")?)?;
            let fitting = match kind {
                KIND_INSERT_FITTING => Some((
                    word(&value["fitting_type"])?,
                    point(&value["at"])?,
                    word(&value["tag"]),
                )),
                _ => None,
            };
            Some(statement(
                tag,
                Pipe::Run(RunStatement {
                    line_tag,
                    fitting,
                    run,
                }),
            ))
        }
        _ => None,
    }
}

// ── what a step did, read off its images and the drawing it left ──────────

/// Where an entity is, as far as a pipe part's place goes.
fn place(entity: &EntityType) -> Option<[f64; 4]> {
    match entity {
        EntityType::Line(line) => Some([line.start.x, line.start.y, line.end.x, line.end.y]),
        EntityType::Insert(insert) => Some([
            insert.insert_point.x,
            insert.insert_point.y,
            insert.rotation,
            insert.x_scale(),
        ]),
        EntityType::Point(point) => Some([point.location.x, point.location.y, 0.0, 0.0]),
        _ => None,
    }
}

/// One entity's images in a step: as it was before, as it is after.
pub type Image<'a> = (Handle, Option<&'a EntityType>, Option<&'a EntityType>);

#[derive(Default)]
struct Touched {
    made: bool,
    lost: bool,
    new_fittings: Vec<Handle>,
}

/// The tag of the run a part belongs to: its owner's, in the drawing as it
/// is, else among the entities the step erased, else in the table.
fn run_of(
    doc: &Document,
    table: &TopologyTable,
    images: &[Image<'_>],
    owner: &str,
) -> Option<String> {
    let handle = parse_handle(owner)?;
    let is_run = |author: PidAuthor| {
        (author.topology == ROLE_PIPE_RUN && !author.tag.is_empty()).then_some(author.tag)
    };
    PidAuthor::load(doc, handle)
        .and_then(is_run)
        .or_else(|| {
            images
                .iter()
                .find(|(image, ..)| *image == handle)
                .and_then(|(_, before, _)| before.and_then(PidAuthor::from_entity))
                .and_then(is_run)
        })
        .or_else(|| table.run_tag_of_handle(handle))
}

fn gone(tag: &str) -> Change {
    Change {
        kind: KIND_DELETE_OBJECT.to_owned(),
        object_ref: tag.to_owned(),
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: None,
        tag_source: None,
        overrides: None,
    }
}

/// A stretch a step drew of a line: its entity, where it runs and what it
/// carries.
type Stretch = (Handle, [f64; 4], PidAuthor);

/// What one step did to the drawing's pipe lines and runs, for the room.
///
/// `images` say which lines and runs the step touched — an entity of
/// theirs came, went or moved — and `doc`, the drawing as the step left
/// it, says what each run is like now. A line's new stretches are said as
/// stretches; a line with none left, a run whose body is gone and a valve
/// that is gone are said as gone; every other touched run is said as it
/// stands.
///
/// Not said, as before: a line that lost some of its stretches and kept
/// others, and anything that only moved a line's stretch — the geometry of
/// an object that is there is not in the protocol.
pub fn statements(doc: &Document, images: &[Image<'_>]) -> Vec<Change> {
    let mut made: BTreeMap<(String, String), Vec<Stretch>> = BTreeMap::new();
    let mut lost: BTreeSet<(String, String)> = BTreeSet::new();
    let mut valves_gone: Vec<String> = Vec::new();
    let mut runs: BTreeMap<String, Touched> = BTreeMap::new();
    // Read only when the step touched a pipe part: most steps touch none.
    let mut table: Option<TopologyTable> = None;

    for (handle, before, after) in images {
        let Some(author) = (*after).or(*before).and_then(PidAuthor::from_entity) else {
            continue;
        };
        let (before, after): (Option<&EntityType>, Option<&EntityType>) = (*before, *after);
        match author.topology.as_str() {
            ROLE_PIPE_LINE | ROLE_SIGNAL_LINE if !author.tag.is_empty() => {
                let key = (author.tag.clone(), author.topology.clone());
                match (before, after) {
                    (None, Some(entity)) => {
                        if let Some(at) =
                            place(entity).filter(|_| matches!(entity, EntityType::Line(_)))
                        {
                            made.entry(key).or_default().push((*handle, at, author));
                        }
                    }
                    (Some(_), None) => {
                        lost.insert(key);
                    }
                    _ => {}
                }
            }
            ROLE_PIPE_RUN if !author.tag.is_empty() => match (before, after) {
                (None, Some(_)) => runs.entry(author.tag.clone()).or_default().made = true,
                (Some(_), None) => runs.entry(author.tag.clone()).or_default().lost = true,
                // Changed where it stands: its fields go by its tag.
                _ => {}
            },
            ROLE_PIPE_SEGMENT | ROLE_FITTING => {
                // Its fields are another matter (a segment has none of its
                // own, a valve's go by its tag): only coming, going and
                // moving change what the run is made of.
                let moved = match (before, after) {
                    (Some(before), Some(after)) => place(before) != place(after),
                    _ => true,
                };
                if !moved {
                    continue;
                }
                let table = table.get_or_insert_with(|| TopologyTable::load(doc));
                let Some(run_tag) = run_of(doc, table, images, &author.owner) else {
                    continue;
                };
                let touched = runs.entry(run_tag).or_default();
                if author.topology == ROLE_FITTING {
                    if before.is_none() {
                        touched.new_fittings.push(*handle);
                    }
                    if after.is_none() && author.type_code == TYPE_VALVE && !author.tag.is_empty() {
                        valves_gone.push(author.tag.clone());
                    }
                }
            }
            _ => {}
        }
    }
    if made.is_empty() && lost.is_empty() && runs.is_empty() {
        return Vec::new();
    }
    let table = table.unwrap_or_else(|| TopologyTable::load(doc));

    let mut out = Vec::new();
    for ((tag, _role), mut pieces) in made.clone() {
        pieces.sort_by_key(|(handle, ..)| handle.value());
        let (first, _, author) = &pieces[0];
        out.push(Change {
            kind: KIND_CREATE_PIPE_LINE.to_owned(),
            object_ref: tag,
            handles: Vec::new(),
            field: String::new(),
            value: String::new(),
            old: None,
            placed: None,
            pipe: Some(Pipe::Line(LineStretches {
                type_code: author.type_code.clone(),
                pieces: pieces.iter().map(|(_, at, _)| *at).collect(),
                // Off the drawing, not off the image: properties too long
                // for an entity's own record are kept beside it there.
                props: props_of(doc, *first),
                tag_source: author.tag_source.clone(),
            })),
            tag_source: None,
            overrides: None,
        });
    }
    valves_gone.sort();
    valves_gone.dedup();
    out.extend(valves_gone.iter().map(|tag| gone(tag)));
    for (tag, touched) in &runs {
        match run_state(doc, &table, tag) {
            Some((line_tag, run, fittings)) => {
                let fitting = touched
                    .new_fittings
                    .iter()
                    .find_map(|handle| fittings.iter().position(|known| known == handle))
                    .map(|index| &run.fittings[index])
                    .filter(|_| !touched.made)
                    .map(|fitting| {
                        (
                            fitting.fitting_type.clone(),
                            fitting.at,
                            fitting.tag.clone(),
                        )
                    });
                let kind = if touched.made {
                    KIND_ADD_PIPE_RUN
                } else if fitting.is_some() {
                    KIND_INSERT_FITTING
                } else {
                    KIND_UPDATE_PIPE_RUN
                };
                out.push(Change {
                    kind: kind.to_owned(),
                    object_ref: tag.clone(),
                    handles: Vec::new(),
                    field: String::new(),
                    value: String::new(),
                    old: None,
                    placed: None,
                    pipe: Some(Pipe::Run(RunStatement {
                        line_tag,
                        fitting,
                        run,
                    })),
                    tag_source: None,
                    overrides: None,
                });
            }
            // Not in the drawing: gone with this step, or its parts were
            // left behind by a body erased earlier — which has no name.
            None if touched.lost => out.push(gone(tag)),
            None => {}
        }
    }
    for (tag, role) in &lost {
        let key = (tag.clone(), role.clone());
        let left = made.contains_key(&key)
            || doc.entities().any(|entity| {
                PidAuthor::from_entity(entity)
                    .is_some_and(|author| author.topology == *role && author.tag == *tag)
            });
        if !left {
            out.push(gone(tag));
        }
    }
    out
}

/// The entity that is run `tag`'s body, with what it carries.
fn run_body(doc: &Document, tag: &str) -> Option<(Handle, PidAuthor, [f64; 2])> {
    doc.entities().find_map(|entity| {
        let author = PidAuthor::from_entity(entity)?;
        if author.topology != ROLE_PIPE_RUN || author.tag != tag {
            return None;
        }
        let at = match entity {
            EntityType::Point(point) => [point.location.x, point.location.y],
            _ => place(entity).map(|at| [at[0], at[1]])?,
        };
        Some((entity.common().handle, author, at))
    })
}

/// A part of a run in the drawing: a segment or a fitting.
struct Part {
    handle: Handle,
    /// A segment's two ends; a break's mark.
    ends: Option<([f64; 2], [f64; 2])>,
    /// A fitting's place.
    at: [f64; 2],
    /// A symbol fitting's block.
    block: String,
}

/// A fitting of a run in the drawing: the part, what it carries, its scale
/// and its turn.
type FittingPart = (Part, PidAuthor, f64, f64);

/// The segments and the fittings of the run whose body is `body`, as they
/// are in the drawing.
fn run_parts(doc: &Document, body: Handle) -> (Vec<Part>, Vec<FittingPart>) {
    let owner = handle_text(body);
    let (mut segments, mut fittings) = (Vec::new(), Vec::new());
    for entity in doc.entities() {
        let Some(author) = PidAuthor::from_entity(entity) else {
            continue;
        };
        if author.owner != owner {
            continue;
        }
        let handle = entity.common().handle;
        match (author.topology.as_str(), entity) {
            (ROLE_PIPE_SEGMENT, EntityType::Line(line)) => segments.push(Part {
                handle,
                ends: Some(([line.start.x, line.start.y], [line.end.x, line.end.y])),
                at: [line.start.x, line.start.y],
                block: String::new(),
            }),
            (ROLE_FITTING, EntityType::Line(line)) => {
                let (from, to) = ([line.start.x, line.start.y], [line.end.x, line.end.y]);
                fittings.push((
                    Part {
                        handle,
                        ends: Some((from, to)),
                        at: [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
                        block: String::new(),
                    },
                    author,
                    1.0,
                    0.0,
                ));
            }
            (ROLE_FITTING, EntityType::Insert(insert)) => fittings.push((
                Part {
                    handle,
                    ends: None,
                    at: [insert.insert_point.x, insert.insert_point.y],
                    block: insert.block_name.clone(),
                },
                author,
                insert.x_scale(),
                insert.rotation,
            )),
            _ => {}
        }
    }
    (segments, fittings)
}

/// The account of run `tag` as the drawing has it: its line's tag, the
/// account, and the handles of its fittings in the account's order. `None`
/// when the drawing has no such run.
fn run_state(
    doc: &Document,
    table: &TopologyTable,
    tag: &str,
) -> Option<(String, RunState, Vec<Handle>)> {
    let (body, author, start) = run_body(doc, tag)?;
    let record = table.pipe_runs.iter().find(|run| run.tag == tag);
    let line_tag = parse_handle(&author.owner)
        .and_then(|line| PidAuthor::load(doc, line))
        .map(|line| line.tag)
        .filter(|tag| !tag.is_empty())
        .or_else(|| record.map(|run| run.line_tag.clone()))
        .unwrap_or_default();
    let (mut segments, mut fittings) = run_parts(doc, body);
    // The table's order, then whatever it does not list, oldest first.
    let listed = |texts: Vec<&str>, handle: Handle| {
        let text = handle_text(handle);
        (
            texts
                .iter()
                .position(|known| *known == text)
                .unwrap_or(usize::MAX),
            handle.value(),
        )
    };
    let segment_order: Vec<&str> = record
        .map(|run| run.segments.iter().map(String::as_str).collect())
        .unwrap_or_default();
    segments.sort_by_key(|part| listed(segment_order.clone(), part.handle));
    let fitting_order: Vec<&str> = table
        .fittings
        .iter()
        .map(|fitting| fitting.handle.as_str())
        .collect();
    fittings.sort_by_key(|(part, ..)| listed(fitting_order.clone(), part.handle));

    let fitting_at = |peer: &Option<String>| -> Option<(usize, String)> {
        let (handle, port) = peer.as_deref()?.split_once(':')?;
        let index = fittings
            .iter()
            .position(|(part, ..)| handle_text(part.handle) == handle)?;
        Some((index, port.to_owned()))
    };
    let run = RunState {
        start,
        // Wherever the drawing keeps them: properties too long for an
        // entity's own record are in an XRecord beside it.
        props: props_of(doc, body),
        overrides: overrides_of(doc, body),
        segments: segments
            .iter()
            .map(|part| {
                let (start, end) = part.ends.unwrap_or((part.at, part.at));
                let known = table.segment_by_handle(part.handle);
                SegmentState {
                    start,
                    end,
                    start_peer: known.and_then(|segment| fitting_at(&segment.start.peer)),
                    end_peer: known.and_then(|segment| fitting_at(&segment.end.peer)),
                }
            })
            .collect(),
        fittings: fittings
            .iter()
            .map(|(part, author, scale, rotation)| {
                let text = handle_text(part.handle);
                let known = table.fittings.iter().find(|fitting| fitting.handle == text);
                let is_break = part.ends.is_some();
                let own_tag = author.type_code == TYPE_VALVE && !author.tag.is_empty();
                FittingState {
                    fitting_type: match known {
                        Some(fitting) => fitting.fitting_type.clone(),
                        None if is_break => FITTING_BREAK.to_owned(),
                        // `PID_SYM_<category>_<id>`: the id is what the table calls it.
                        None => part
                            .block
                            .strip_prefix(crate::pid::symbols::BLOCK_PREFIX)
                            .and_then(|rest| rest.split_once('_'))
                            .map(|(_, id)| id.to_owned())
                            .unwrap_or_else(|| part.block.clone()),
                    },
                    type_code: author.type_code.clone(),
                    tag: own_tag.then(|| author.tag.clone()),
                    at: part.at,
                    block: part.block.clone(),
                    scale: *scale,
                    rotation: *rotation,
                    props: if is_break {
                        BTreeMap::new()
                    } else {
                        props_of(doc, part.handle)
                    },
                    tag_source: author.tag_source.clone(),
                    mark: part.ends.map(|(from, to)| [from, to]),
                    ports: known
                        .map(|fitting| {
                            fitting
                                .ports
                                .iter()
                                .map(|(port, bound)| {
                                    let index = bound.as_deref().and_then(|text| {
                                        segments
                                            .iter()
                                            .position(|part| handle_text(part.handle) == text)
                                    });
                                    (port.clone(), index)
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
    };
    let handles = fittings.iter().map(|(part, ..)| part.handle).collect();
    Some((line_tag, run, handles))
}

// ── bringing a drawing to what was said ──────────────────────────────────

/// The entities that are run `tag` in `doc`: its body and everything that
/// has the body for its owner. `None` when no run carries the tag.
pub(super) fn run_with_parts(doc: &Document, tag: &str) -> Option<Vec<Handle>> {
    let (body, ..) = run_body(doc, tag)?;
    let (segments, fittings) = run_parts(doc, body);
    let mut all = vec![body];
    all.extend(segments.iter().map(|part| part.handle));
    all.extend(fittings.iter().map(|(part, ..)| part.handle));
    Some(all)
}

/// Whether the drawing's line or run is still as statement `change` says
/// it: the line has the stretches it was given, the run the account it was
/// left with (`SPEC-collab-offline-sync.md` §5.4).
pub(super) fn as_said(doc: &Document, change: &Change) -> bool {
    match &change.pipe {
        Some(Pipe::Line(said)) => {
            let pieces: Vec<[f64; 4]> = doc
                .entities()
                .filter(|entity| matches!(entity, EntityType::Line(_)))
                .filter(|entity| {
                    PidAuthor::from_entity(entity).is_some_and(|author| {
                        author.tag == change.object_ref
                            && matches!(author.topology.as_str(), ROLE_PIPE_LINE | ROLE_SIGNAL_LINE)
                    })
                })
                .filter_map(place)
                .collect();
            let same_piece = |a: &[f64; 4], b: &[f64; 4]| {
                same([a[0], a[1]], [b[0], b[1]]) && same([a[2], a[3]], [b[2], b[3]])
            };
            pieces.len() == said.pieces.len()
                && said
                    .pieces
                    .iter()
                    .all(|want| pieces.iter().any(|have| same_piece(have, want)))
        }
        Some(Pipe::Run(said)) => {
            let table = TopologyTable::load(doc);
            run_state(doc, &table, &change.object_ref).is_some_and(|(line_tag, run, _)| {
                line_tag == said.line_tag && run.to_json() == said.run.to_json()
            })
        }
        _ => false,
    }
}

fn vector(at: [f64; 2]) -> document::types::Vector3 {
    document::types::Vector3::new(at[0], at[1], 0.0)
}

impl OpenCADStudio {
    /// Bring the drawing on tab `i` to what a peer's pipe statement says.
    ///
    /// What cannot be done — a run on a line this drawing does not have, a
    /// fitting whose symbol this program's library does not have — goes
    /// into `applied.unresolved`, and nothing of that statement is written.
    pub(in crate::app) fn write_remote_pipe(
        &mut self,
        i: usize,
        change: &Change,
        applied: &mut Applied,
    ) {
        let outcome = match &change.pipe {
            Some(Pipe::Line(line)) => Ok(self.ensure_line(i, &change.object_ref, line, applied)),
            Some(Pipe::Run(said)) => self.align_run(i, &change.object_ref, said, applied),
            Some(Pipe::Connector(said)) => self.write_remote_connector(i, change, said, applied),
            None => Err(change.object_ref.clone()),
        };
        match outcome {
            Ok(handles) => applied.resolved.push((change.clone(), handles)),
            Err(name) => applied.unresolved.push(name),
        }
    }

    /// Make sure line `tag` has the stretches `said` lists: the ones the
    /// drawing lacks are drawn, and a pipe line the table does not know is
    /// entered in it. Returns the line's entities.
    fn ensure_line(
        &mut self,
        i: usize,
        tag: &str,
        said: &LineStretches,
        applied: &mut Applied,
    ) -> Vec<Handle> {
        let signal = said.type_code == TYPE_SIGNAL_LINE;
        let role = if signal {
            ROLE_SIGNAL_LINE
        } else {
            ROLE_PIPE_LINE
        };
        let mut have: Vec<(Handle, [f64; 4])> = {
            let document = &self.tabs[i].scene.document;
            document
                .entities()
                .filter_map(|entity| {
                    let author = PidAuthor::from_entity(entity)?;
                    (author.topology == role && author.tag == tag)
                        .then(|| place(entity).map(|at| (entity.common().handle, at)))?
                })
                .collect()
        };
        have.sort_by_key(|(handle, _)| handle.value());
        let missing: Vec<[f64; 4]> = said
            .pieces
            .iter()
            .filter(|piece| {
                !have.iter().any(|(_, at)| {
                    same([at[0], at[1]], [piece[0], piece[1]])
                        && same([at[2], at[3]], [piece[2], piece[3]])
                })
            })
            .copied()
            .collect();
        let drawn = self.tabs[i].scene.add_entities(
            missing
                .iter()
                .map(|piece| {
                    EntityType::Line(document::entities::Line::from_points(
                        vector([piece[0], piece[1]]),
                        vector([piece[2], piece[3]]),
                    ))
                })
                .collect(),
        );
        let props = join_props(&said.props, &BTreeSet::new());
        let document = &mut self.tabs[i].scene.document;
        for handle in &drawn {
            PidAuthor {
                type_code: said.type_code.clone(),
                tag: tag.to_owned(),
                tag_source: said.tag_source.clone(),
                tag_status: "ok".to_owned(),
                topology: role.to_owned(),
                props: props.clone(),
                ..PidAuthor::default()
            }
            .store(document, *handle);
            crate::app::commands::pidpalette::sync_pid_tag_attribute(document, *handle);
        }
        let mut all: Vec<Handle> = have.iter().map(|(handle, _)| *handle).collect();
        all.extend(&drawn);
        if !signal {
            let mut table = TopologyTable::load(document);
            table.prune(document);
            if table.line_by_tag(tag).is_none() {
                if let Some(first) = all.first() {
                    table.append_line(tag, *first, said.props.clone());
                    table.store(document);
                    self.tabs[i].dirty = true;
                }
            }
        }
        applied.handles.extend(&drawn);
        applied.retagged.extend(&drawn);
        all
    }

    /// Align run `tag` on tab `i` to the account `said` gives of it, and
    /// write its records in the topology table again. Returns the run's
    /// entities, or the name to report when the run cannot be followed.
    fn align_run(
        &mut self,
        i: usize,
        tag: &str,
        said: &RunStatement,
        applied: &mut Applied,
    ) -> Result<Vec<Handle>, String> {
        let state = &said.run;
        // Everything that can fail is looked at before anything is written.
        let line = {
            let document = &self.tabs[i].scene.document;
            let table = TopologyTable::load(document);
            table
                .line_by_tag(&said.line_tag)
                .and_then(|line| parse_handle(&line.handle))
                .filter(|handle| document.get_entity(*handle).is_some())
                .or_else(|| {
                    document
                        .entities()
                        .filter(|entity| {
                            PidAuthor::from_entity(entity).is_some_and(|author| {
                                author.topology == ROLE_PIPE_LINE && author.tag == said.line_tag
                            })
                        })
                        .map(|entity| entity.common().handle)
                        .min_by_key(|handle| handle.value())
                })
        };
        let Some(line) = line else {
            return Err(format!("{tag}（管线 {} 不在本图里）", said.line_tag));
        };
        let symbols: Vec<&FittingState> = state
            .fittings
            .iter()
            .filter(|fitting| fitting.mark.is_none())
            .collect();
        if symbols.iter().any(|fitting| {
            !self.tabs[i]
                .scene
                .document
                .block_records
                .contains(&fitting.block)
        }) {
            self.pid_symbols.instantiate(&mut self.tabs[i].scene);
        }
        if let Some(absent) = symbols.iter().find(|fitting| {
            !self.tabs[i]
                .scene
                .document
                .block_records
                .contains(&fitting.block)
        }) {
            return Err(format!("{tag}（符号 {} 不在本机的符号库里）", absent.block));
        }

        // The body.
        let body = match run_body(&self.tabs[i].scene.document, tag) {
            Some((body, ..)) => body,
            None => {
                let body = self.tabs[i].scene.add_entity(EntityType::Point(
                    document::entities::Point::at(vector(state.start)),
                ));
                let document = &mut self.tabs[i].scene.document;
                let mut values = state.props.clone();
                let stated = !values.is_empty();
                values
                    .entry("管线号".to_owned())
                    .or_insert_with(|| said.line_tag.clone());
                PidAuthor {
                    type_code: TYPE_PIPE_RUN.to_owned(),
                    tag: tag.to_owned(),
                    tag_source: "auto".to_owned(),
                    tag_status: "ok".to_owned(),
                    topology: ROLE_PIPE_RUN.to_owned(),
                    props: join_props(&values, &state.overrides),
                    owner: handle_text(line),
                    ..PidAuthor::default()
                }
                .store(document, body);
                if !stated {
                    // Said without an account: the run takes the line's
                    // values here, as a run drawn here does.
                    let batch = crate::pid::propagate::inherit_on_create(document, line, body);
                    crate::pid::propagate::apply(document, &batch);
                }
                crate::app::commands::pidpalette::sync_pid_tag_attribute(document, body);
                applied.handles.push(body);
                applied.retagged.push(body);
                body
            }
        };
        let owner = handle_text(body);
        let run_props = PidAuthor::load(&self.tabs[i].scene.document, body)
            .map(|author| author.props)
            .unwrap_or_else(|| "{}".to_owned());
        let (have_segments, have_fittings) = run_parts(&self.tabs[i].scene.document, body);

        // Segments: the ones whose two ends fit stay; one that shares its
        // start keeps its entity and takes the new end (the first piece of
        // a split keeps its handle); the rest are drawn, and what is left
        // over is erased.
        let mut free: Vec<&Part> = have_segments.iter().collect();
        let mut segment_handles: Vec<Option<Handle>> = vec![None; state.segments.len()];
        let mut moved: Vec<(Handle, [f64; 2], [f64; 2])> = Vec::new();
        for (slot, segment) in segment_handles.iter_mut().zip(&state.segments) {
            let fits = |part: &&Part| {
                part.ends.is_some_and(|(start, end)| {
                    same(start, segment.start) && same(end, segment.end)
                })
            };
            if let Some(index) = free.iter().position(fits) {
                *slot = Some(free.remove(index).handle);
            }
        }
        for by_start in [true, false] {
            for (slot, segment) in segment_handles.iter_mut().zip(&state.segments) {
                if slot.is_some() {
                    continue;
                }
                let shares = |part: &&Part| {
                    part.ends.is_some_and(|(start, end)| match by_start {
                        true => same(start, segment.start),
                        false => same(end, segment.end),
                    })
                };
                if let Some(index) = free.iter().position(shares) {
                    let handle = free.remove(index).handle;
                    moved.push((handle, segment.start, segment.end));
                    *slot = Some(handle);
                }
            }
        }
        let over: Vec<Handle> = free.iter().map(|part| part.handle).collect();
        for (handle, start, end) in &moved {
            if let Some(EntityType::Line(line)) =
                self.tabs[i].scene.document.get_entity_mut(*handle)
            {
                line.start = vector(*start);
                line.end = vector(*end);
            }
            applied.handles.push(*handle);
        }
        for (slot, segment) in segment_handles.iter_mut().zip(&state.segments) {
            if slot.is_some() {
                continue;
            }
            let handle = self.tabs[i].scene.add_entity(EntityType::Line(
                document::entities::Line::from_points(vector(segment.start), vector(segment.end)),
            ));
            PidAuthor {
                type_code: TYPE_PIPE_SEGMENT.to_owned(),
                tag: String::new(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                topology: ROLE_PIPE_SEGMENT.to_owned(),
                props: run_props.clone(),
                owner: owner.clone(),
                ..PidAuthor::default()
            }
            .store(&mut self.tabs[i].scene.document, handle);
            applied.handles.push(handle);
            *slot = Some(handle);
        }

        // Fittings: by where they stand and what they are.
        let mut free: Vec<&(Part, PidAuthor, f64, f64)> = have_fittings.iter().collect();
        let mut fitting_handles: Vec<Option<Handle>> = vec![None; state.fittings.len()];
        for (slot, fitting) in fitting_handles.iter_mut().zip(&state.fittings) {
            let fits = |known: &&(Part, PidAuthor, f64, f64)| {
                let part = &known.0;
                same(part.at, fitting.at)
                    && part.ends.is_some() == fitting.mark.is_some()
                    && (fitting.mark.is_some() || part.block == fitting.block)
            };
            if let Some(index) = free.iter().position(fits) {
                *slot = Some(free.remove(index).0.handle);
            }
        }
        let mut over_fittings: Vec<Handle> = free.iter().map(|known| known.0.handle).collect();
        for (slot, fitting) in fitting_handles.iter_mut().zip(&state.fittings) {
            if slot.is_some() {
                continue;
            }
            let handle = match fitting.mark {
                Some([from, to]) => {
                    let handle = self.tabs[i].scene.add_entity(EntityType::Line(
                        document::entities::Line::from_points(vector(from), vector(to)),
                    ));
                    PidAuthor {
                        type_code: TYPE_FITTING.to_owned(),
                        tag: String::new(),
                        tag_source: "auto".to_owned(),
                        tag_status: "ok".to_owned(),
                        topology: ROLE_FITTING.to_owned(),
                        owner: owner.clone(),
                        props: "{}".to_owned(),
                        ..PidAuthor::default()
                    }
                    .store(&mut self.tabs[i].scene.document, handle);
                    handle
                }
                None => {
                    let insert =
                        document::entities::Insert::new(fitting.block.clone(), vector(fitting.at))
                            .with_uniform_scale(fitting.scale)
                            .with_rotation(fitting.rotation);
                    let handle = self.tabs[i].scene.add_entity(EntityType::Insert(insert));
                    let ports = self.pid_symbols.symbol_ports(&fitting.block).join(",");
                    let document = &mut self.tabs[i].scene.document;
                    PidAuthor {
                        type_code: fitting.type_code.clone(),
                        // A fitting with no tag of its own shows its run's.
                        tag: fitting.tag.clone().unwrap_or_else(|| tag.to_owned()),
                        tag_source: fitting.tag_source.clone(),
                        tag_status: "ok".to_owned(),
                        ports,
                        owner: owner.clone(),
                        topology: ROLE_FITTING.to_owned(),
                        props: join_props(&fitting.props, &BTreeSet::new()),
                        ..PidAuthor::default()
                    }
                    .store(document, handle);
                    crate::app::commands::pidpalette::sync_pid_tag_attribute(document, handle);
                    if fitting.tag.is_some() {
                        applied.retagged.push(handle);
                    }
                    handle
                }
            };
            applied.handles.push(handle);
            *slot = Some(handle);
        }
        over_fittings.extend(over);
        if !over_fittings.is_empty() {
            self.tabs[i].scene.erase_entities(&over_fittings);
            applied.erased.extend(&over_fittings);
        }

        // The table: this run's records, written again from the account.
        let segment_handles: Vec<Handle> = segment_handles.into_iter().flatten().collect();
        let fitting_handles: Vec<Handle> = fitting_handles.into_iter().flatten().collect();
        let document = &mut self.tabs[i].scene.document;
        let mut table = TopologyTable::load(document);
        let before = table.clone();
        table.prune(document);
        table.pipe_segments.retain(|segment| segment.run_tag != tag);
        table.fittings.retain(|fitting| fitting.run_tag != tag);
        if !table.pipe_runs.iter().any(|run| run.tag == tag) {
            let (values, marks) = split_props(&run_props);
            table.append_run(
                &said.line_tag,
                tag,
                body,
                values,
                marks.into_iter().collect(),
            );
        }
        if let Some(record) = table.line_by_tag_mut(&said.line_tag) {
            if !record.runs.iter().any(|known| known == tag) {
                record.runs.push(tag.to_owned());
            }
            // The next run drawn here must not take this one's number.
            let ordinal = tag
                .rsplit_once('-')
                .and_then(|(_, suffix)| suffix.parse::<u32>().ok())
                .unwrap_or(0);
            record.next_run_ordinal = record.next_run_ordinal.max(ordinal.saturating_add(1));
        }
        if let Some(record) = table.pipe_runs.iter_mut().find(|run| run.tag == tag) {
            record.handle = owner.clone();
            record.line_tag = said.line_tag.clone();
            record.segments = segment_handles
                .iter()
                .map(|handle| handle_text(*handle))
                .collect();
        }
        let peer = |peer: &Option<(usize, String)>| {
            let (index, port) = peer.as_ref()?;
            Some(format!(
                "{}:{port}",
                handle_text(*fitting_handles.get(*index)?)
            ))
        };
        for (handle, segment) in segment_handles.iter().zip(&state.segments) {
            table.pipe_segments.push(PipeSegmentRecord {
                handle: handle_text(*handle),
                run_tag: tag.to_owned(),
                start: SegmentEnd {
                    point: segment.start,
                    peer: peer(&segment.start_peer),
                },
                end: SegmentEnd {
                    point: segment.end,
                    peer: peer(&segment.end_peer),
                },
            });
        }
        for (handle, fitting) in fitting_handles.iter().zip(&state.fittings) {
            table.fittings.push(FittingRecord {
                handle: handle_text(*handle),
                run_tag: tag.to_owned(),
                fitting_type: fitting.fitting_type.clone(),
                tag: fitting.tag.clone(),
                ports: fitting
                    .ports
                    .iter()
                    .map(|(port, segment)| {
                        let bound = segment
                            .and_then(|index| segment_handles.get(index))
                            .map(|handle| handle_text(*handle));
                        (port.clone(), bound)
                    })
                    .collect(),
            });
        }
        if table != before {
            table.store(document);
            self.tabs[i].dirty = true;
        }

        let mut all = vec![body];
        all.extend(&segment_handles);
        all.extend(&fitting_handles);
        all.extend(&over_fittings);
        Ok(all)
    }
}
