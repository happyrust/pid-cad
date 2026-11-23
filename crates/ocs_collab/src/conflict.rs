//! What the server knows of a drawing, and the fixed three-step judgment of a
//! change against it (`architecture.md` §3.3, appendix A.4).
//!
//! The server never sees DWG bytes. What it has is the ops it applied, and
//! from them the value every field was last given, by whom and at which
//! revision. That is enough to say, of a change made against an older
//! baseline, which of three things it is:
//!
//! 1. It writes a field nobody else wrote since that baseline: it merges.
//! 2. Somebody else wrote that field since, to another value: the change is
//!    behind and yields; the value it would have put there is reported so the
//!    caller can audit it.
//! 3. Two changes made against the *same* baseline write one field to
//!    different values and neither has taken effect yet: nothing decides
//!    between them, and a conflict entry is made for a person
//!    ([`collides`]).
//!
//! Everything here is a pure function over that state: no clock, no I/O.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Op kinds the judgment understands. Anything else is opaque to it.
pub const KIND_SET_PROPERTY: &str = "set_property";
pub const KIND_SET_TAG: &str = "set_tag";
pub const KIND_DELETE_OBJECT: &str = "delete_object";
pub const KIND_INSERT_SYMBOL: &str = "insert_symbol";
pub const KIND_CREATE_PIPE_LINE: &str = "create_pipe_line";
pub const KIND_ADD_PIPE_RUN: &str = "add_pipe_run";
pub const KIND_INSERT_FITTING: &str = "insert_fitting";
pub const KIND_CREATE_CONNECTOR: &str = "create_connector";
/// A connector is gone from a line: erased, or its making taken back.
pub const KIND_DELETE_CONNECTOR: &str = "delete_connector";
/// A connector has an other end on another drawing — or has none any more.
/// Said by the server, which pairs them ([`crate::link`]); from a client it
/// is one more change the judgment has no rule for.
pub const KIND_PAIR_CONNECTOR: &str = "pair_connector";
/// A run's parts as they are after a step that changed them otherwise than
/// by adding the run or putting a fitting on it: parts erased, an insertion
/// taken back. It carries the same account of the run (`run`) the two
/// creating kinds may carry.
pub const KIND_UPDATE_PIPE_RUN: &str = "update_pipe_run";
/// The author type of a signal line: created like a pipe line
/// (`create_pipe_line`), and not one of the drawing's pipe lines.
pub const TYPE_SIGNAL_LINE: &str = "SIGNAL_LINE";

/// The field a retag writes.
pub const FIELD_TAG: &str = "tag";
/// The "field" two changes to one pipe line's connections contend for.
pub const FIELD_TOPOLOGY: &str = "topology";

/// One change an op asks for, as read off its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub kind: String,
    /// The object, by the tag the sender knows it under.
    pub object_ref: String,
    /// The field written: the property name, or [`FIELD_TAG`] for a retag.
    /// Empty for a kind that writes no single field.
    pub field: String,
    /// The value written. Empty for a kind that carries none.
    pub value: String,
    /// The value the sender replaced, when it said.
    pub old_value: Option<String>,
    /// The change as it arrived, every key the client sent.
    pub raw: Value,
}

impl Change {
    /// Every change an op payload carries, in order: the payload itself, then
    /// the ones a client batched under `changes` so that one undo step is one
    /// op. A payload that is not an object carries none.
    pub fn list(payload: &Value) -> Vec<Change> {
        let Some(first) = Change::read(payload) else {
            return Vec::new();
        };
        let mut changes = vec![first];
        if let Some(rest) = payload.get("changes").and_then(Value::as_array) {
            changes.extend(rest.iter().filter_map(Change::read));
        }
        changes
    }

    fn read(value: &Value) -> Option<Change> {
        let object = value.as_object()?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        let kind = text("kind").unwrap_or_default();
        let field = match text("field") {
            Some(field) => field,
            None if kind == KIND_SET_TAG => FIELD_TAG.to_owned(),
            None => String::new(),
        };
        // The batch rides on the first change; it is not part of that change.
        let mut raw = object.clone();
        raw.remove("changes");
        Some(Change {
            kind,
            object_ref: text("object_ref").unwrap_or_default(),
            field,
            value: object.get("value").map(value_text).unwrap_or_default(),
            old_value: object.get("old_value").map(value_text),
            raw: Value::Object(raw),
        })
    }

    /// Whether this is a write to one field of one named object — the only
    /// kind of change the three steps are defined for.
    pub fn writes_a_field(&self) -> bool {
        (self.kind == KIND_SET_PROPERTY || self.kind == KIND_SET_TAG)
            && !self.object_ref.is_empty()
            && !self.field.is_empty()
    }

    /// Whether this changes how a pipe line is connected up.
    pub fn changes_topology(&self) -> bool {
        matches!(
            self.kind.as_str(),
            KIND_ADD_PIPE_RUN
                | KIND_INSERT_FITTING
                | KIND_CREATE_CONNECTOR
                | KIND_DELETE_CONNECTOR
                | KIND_UPDATE_PIPE_RUN
        )
    }

    /// Whether the judgment has a rule for this change. One it has none for
    /// is taken or refused whole, by the op's baseline alone.
    pub fn is_understood(&self) -> bool {
        self.writes_a_field()
            || self.changes_topology()
            || (matches!(
                self.kind.as_str(),
                KIND_DELETE_OBJECT | KIND_INSERT_SYMBOL | KIND_CREATE_PIPE_LINE
            ) && !self.subject().is_empty())
    }

    /// The tag of the object the change is about, for the kinds that name it
    /// under another key: a symbol or a pipe line being created is named by
    /// its `tag`.
    pub fn subject(&self) -> String {
        if !self.object_ref.is_empty() {
            return self.object_ref.clone();
        }
        self.text("tag").unwrap_or_default()
    }

    /// A string the change carries under `key`.
    pub fn text(&self, key: &str) -> Option<String> {
        self.raw.get(key).and_then(Value::as_str).map(str::to_owned)
    }

    /// The change as it goes out in `op_applied`, naming the object by the tag
    /// it has now and backfilling the canonical keys.
    pub fn broadcast_form(&self, object_ref: &str) -> Value {
        let mut out = self.raw.as_object().cloned().unwrap_or_default();
        out.insert("object_ref".to_owned(), Value::String(object_ref.to_owned()));
        for key in ["kind", "field", "value"] {
            out.entry(key.to_owned()).or_insert(Value::Null);
        }
        Value::Object(out)
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The value a field holds and the op that put it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldState {
    pub value: String,
    /// Revision at which it became current.
    pub revision: i64,
    pub session_id: String,
    pub user_id: String,
}

/// Every field the server has seen written on one object, plus its manual
/// property override markers and `tag_source` (#35).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectState {
    pub fields: BTreeMap<String, FieldState>,
    pub overrides: BTreeSet<String>,
    pub tag_source: Option<String>,
}

/// What the three steps say about one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It takes effect. `object_ref` is the tag the object goes by now (a
    /// change made before a retag still names the old one); `replaces` is the
    /// value it overwrites, when the server knows one.
    Apply {
        object_ref: String,
        replaces: Option<String>,
    },
    /// It is behind: since its baseline another session gave the field
    /// another value, at `winner_revision`. The winner stays.
    Yield {
        object_ref: String,
        winner: String,
        winner_revision: i64,
        winner_user: String,
    },
    /// The object level: another session deleted the object after the
    /// change's baseline. The deletion stands and the change does not happen;
    /// `last_value` is what the field held when the object went.
    Gone {
        object_ref: String,
        deleted_revision: i64,
        deleted_by: String,
        last_value: Option<String>,
    },
    /// A deletion of an object other objects still hang on: refused. `by`
    /// names them.
    Referenced { object_ref: String, by: Vec<String> },
    /// A change to a pipe line's connections made before another session's
    /// change to the same line. Connections do not merge field by field: the
    /// sender has to look at the line as it now is and ask again.
    BehindTopology { line_tag: String, touched_revision: i64 },
    /// An inflow connector on a line of the drawing that has one of that
    /// family already: a line comes in from one place.
    DuplicateInflow { line_tag: String },
}

/// An object that was deleted, kept so that a change still on its way to it
/// can be told what it missed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    pub revision: i64,
    pub session_id: String,
    pub user_id: String,
    /// What it carried when it went.
    pub fields: BTreeMap<String, FieldState>,
}

/// One pipe run as the server knows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub line_tag: String,
    pub points: Value,
    /// The objects its two ends are connected to, by tag.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Each fitting as the op that inserted it described it — or, once a
    /// client has given an account of the run (`run`), as that account
    /// lists them.
    pub fittings: Vec<Value>,
    /// The run's segments as the last account of it listed them; `null`
    /// while no client has given one.
    pub segments: Value,
    /// Effective property values on this run (#35).
    pub props: BTreeMap<String, String>,
    /// Property names manually overridden on this run so pipe-line updates do
    /// not overwrite them (#35).
    pub overrides: BTreeSet<String>,
}

/// The semantic state of one drawing: its objects by tag, the retags that
/// let a change made against an older baseline still find its object, what
/// was deleted, how the pipe lines are connected up, and the tags in use in
/// a client's own copy of it that no op told of.
#[derive(Debug, Clone, Default)]
pub struct DrawingModel {
    revision: i64,
    objects: BTreeMap<String, ObjectState>,
    /// `(old tag, new tag, revision)`, oldest first.
    renames: Vec<(String, String, i64)>,
    deleted: BTreeMap<String, Deleted>,
    /// Pipe line tag → its runs' tags, in the order they were added.
    lines: BTreeMap<String, Vec<String>>,
    runs: BTreeMap<String, Run>,
    /// Each connector as the op that created it described it.
    connectors: Vec<Value>,
    /// Pipe line tag → the revision and session that last changed its
    /// connections.
    line_touched: BTreeMap<String, (i64, String)>,
    /// Tags the objects of a client's own copy of the drawing carry that no
    /// op ever told of ([`Self::report_tags`]).
    reported: BTreeSet<String>,
}

impl DrawingModel {
    /// A drawing at `revision` of which nothing else is known yet.
    pub fn at(revision: i64) -> Self {
        Self {
            revision,
            ..Self::default()
        }
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }

    /// The revision moved without a field write the model tracks (an op of a
    /// kind it does not read).
    pub fn set_revision(&mut self, revision: i64) {
        self.revision = revision;
    }

    /// The drawing's connectors, as far as the ops that made them said
    /// which line each is on and what kind it is.
    pub fn connectors(&self) -> Vec<crate::link::Connector> {
        self.connectors
            .iter()
            .filter_map(crate::link::Connector::read)
            .collect()
    }

    /// Every object the server has seen written, by current tag.
    pub fn objects(&self) -> impl Iterator<Item = (&String, &ObjectState)> {
        self.objects.iter()
    }

    pub fn object(&self, object_ref: &str) -> Option<&ObjectState> {
        self.objects.get(object_ref)
    }

    /// The value a field holds now.
    pub fn field(&self, object_ref: &str, field: &str) -> Option<&FieldState> {
        self.objects.get(object_ref)?.fields.get(field)
    }

    /// The tag an object named `object_ref` by someone at `base_revision`
    /// goes by now: every retag made after that baseline is followed.
    pub fn current_name(&self, object_ref: &str, base_revision: i64) -> String {
        let mut name = object_ref.to_owned();
        let mut after = base_revision;
        // Retags are in revision order, so one pass follows a chain.
        for (old, new, revision) in &self.renames {
            if *revision > after && *old == name {
                name = new.clone();
                after = *revision;
            }
        }
        name
    }

    /// Steps 1 and 2 for one field write made against `base_revision` by
    /// `session_id`.
    ///
    /// A session's own earlier writes are never what it is behind: it made
    /// them, so a second edit sent before the first one's `op_applied` came
    /// back is still the newer intent, not a lagging one.
    pub fn judge(&self, change: &Change, base_revision: i64, session_id: &str) -> Verdict {
        let object_ref = self.current_name(&change.subject(), base_revision);
        if change.changes_topology() {
            let line_tag = self.line_of(change);
            if change.kind == KIND_CREATE_CONNECTOR {
                // The same connector said again — a making put on again —
                // is not a second one.
                let second_inflow = crate::link::Connector::read(&change.raw).is_some_and(|said| {
                    said.is_inflow()
                        && self.connectors().iter().any(|known| {
                            known.is_inflow()
                                && known.family() == said.family()
                                && known.line_tag == line_tag
                                && !known.is(&said)
                        })
                });
                if second_inflow {
                    return Verdict::DuplicateInflow { line_tag };
                }
            }
            return match self.line_touched.get(&line_tag) {
                Some((revision, by)) if *revision > base_revision && by != session_id => {
                    Verdict::BehindTopology {
                        line_tag,
                        touched_revision: *revision,
                    }
                }
                _ => Verdict::Apply {
                    object_ref: line_tag,
                    replaces: None,
                },
            };
        }
        if change.kind == KIND_DELETE_OBJECT {
            let by = self.referenced_by(&object_ref);
            return if by.is_empty() {
                Verdict::Apply {
                    object_ref,
                    replaces: None,
                }
            } else {
                Verdict::Referenced { object_ref, by }
            };
        }
        if !change.writes_a_field() {
            // A symbol or a pipe line being created: nothing to be behind.
            return Verdict::Apply {
                object_ref,
                replaces: None,
            };
        }
        // The object level comes before the field level: a field of an
        // object that is gone has nothing to be merged into.
        if let Some(gone) = self.deleted.get(&object_ref) {
            if gone.revision > base_revision && gone.session_id != session_id {
                return Verdict::Gone {
                    object_ref,
                    deleted_revision: gone.revision,
                    deleted_by: gone.user_id.clone(),
                    last_value: gone
                        .fields
                        .get(&change.field)
                        .map(|state| state.value.clone()),
                };
            }
        }
        let current = self.field(&object_ref, &change.field);
        match current {
            Some(state)
                if state.revision > base_revision
                    && state.session_id != session_id
                    && state.value != change.value =>
            {
                Verdict::Yield {
                    object_ref,
                    winner: state.value.clone(),
                    winner_revision: state.revision,
                    winner_user: state.user_id.clone(),
                }
            }
            Some(state) => Verdict::Apply {
                object_ref,
                replaces: (state.value != change.value).then(|| state.value.clone()),
            },
            None => Verdict::Apply {
                object_ref,
                replaces: None,
            },
        }
    }

    /// Record a field write that took effect at `revision`. `object_ref` is
    /// the name [`Verdict::Apply`] gave.
    pub fn commit(
        &mut self,
        change: &Change,
        object_ref: &str,
        revision: i64,
        session_id: &str,
        user_id: &str,
    ) {
        self.revision = self.revision.max(revision);
        let state = FieldState {
            value: change.value.clone(),
            revision,
            session_id: session_id.to_owned(),
            user_id: user_id.to_owned(),
        };
        if !change.writes_a_field() {
            self.commit_structure(change, object_ref, &state);
            return;
        }
        // Written to again by someone who knows it was deleted: it is back.
        self.deleted.remove(object_ref);
        if change.kind == KIND_SET_TAG {
            if change.value == object_ref || change.value.is_empty() {
                return;
            }
            // Whoever has the object has it under the new tag from here on:
            // the old one is in use in no copy.
            self.reported.remove(object_ref);
            // The object keeps its fields under its new tag. Objects are
            // keyed by tag here, so one already carrying the new tag is the
            // same entry from now on; its fields stay where the retagged
            // object has none.
            let mut moved = self.objects.remove(object_ref).unwrap_or_default();
            moved.fields.insert(FIELD_TAG.to_owned(), state);
            if let Some(src) = change.text("tag_source") {
                moved.tag_source = Some(src);
            }
            let target = self.objects.entry(change.value.clone()).or_default();
            for (field, value) in moved.fields {
                target.fields.insert(field, value);
            }
            for ov in moved.overrides {
                target.overrides.insert(ov);
            }
            if moved.tag_source.is_some() {
                target.tag_source = moved.tag_source;
            }
            self.renames
                .push((object_ref.to_owned(), change.value.clone(), revision));
            return;
        }
        let overrides_key = change
            .raw
            .get("overrides")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<BTreeSet<String>>()
            });
        {
            let obj = self.objects.entry(object_ref.to_owned()).or_default();
            obj.fields.insert(change.field.clone(), state.clone());
            if let Some(ref ovs) = overrides_key {
                if ovs.contains(&change.field) {
                    obj.overrides.insert(change.field.clone());
                } else {
                    obj.overrides.remove(&change.field);
                }
            }
        }
        // #35: Sync property + override markers on a pipe run.
        if let Some(run) = self.runs.get_mut(object_ref) {
            run.props.insert(change.field.clone(), change.value.clone());
            if let Some(ref ovs) = overrides_key {
                if ovs.contains(&change.field) {
                    run.overrides.insert(change.field.clone());
                } else {
                    run.overrides.remove(&change.field);
                }
            }
        }
        // #35: Pipe line property change propagates to non-overridden runs of
        // that line, while runs that manually overrode `change.field` keep
        // their own value.
        if crate::link::PIPE_FIELDS.contains(&change.field.as_str()) {
            if let Some(run_tags) = self.lines.get(object_ref).cloned() {
                for run_tag in run_tags {
                    let overridden = self
                        .runs
                        .get(&run_tag)
                        .is_some_and(|r| r.overrides.contains(&change.field))
                        || self
                            .objects
                            .get(&run_tag)
                            .is_some_and(|o| o.overrides.contains(&change.field));
                    if !overridden {
                        if let Some(run) = self.runs.get_mut(&run_tag) {
                            run.props.insert(change.field.clone(), change.value.clone());
                        }
                        if let Some(obj) = self.objects.get_mut(&run_tag) {
                            obj.fields.insert(change.field.clone(), state.clone());
                        }
                    }
                }
            }
        }
    }
}

impl DrawingModel {
    /// The pipe line a topology change is about: the one it names, or the
    /// line of the run it names. A run the server has not seen is taken to
    /// belong to the line its tag is built from (`{line}-{suffix}`).
    pub fn line_of(&self, change: &Change) -> String {
        if let Some(line) = change.text("line_tag") {
            return line;
        }
        let run = change.text("run_tag").unwrap_or_default();
        if let Some(known) = self.runs.get(&run) {
            return known.line_tag.clone();
        }
        match run.rsplit_once('-') {
            Some((line, suffix)) if !line.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) => {
                line.to_owned()
            }
            _ => run,
        }
    }

    /// The pipe lines that have `object_ref` at an end of one of their runs.
    pub fn referenced_by(&self, object_ref: &str) -> Vec<String> {
        let mut lines: Vec<String> = self
            .runs
            .values()
            .filter(|run| {
                run.from.as_deref() == Some(object_ref) || run.to.as_deref() == Some(object_ref)
            })
            .map(|run| run.line_tag.clone())
            .collect();
        lines.sort();
        lines.dedup();
        lines
    }

    /// What two changes contend for, and with what: `(object, field, value)`.
    /// Field writes contend for the field; changes to one pipe line's
    /// connections contend for the line's topology, each with its own
    /// description. Anything else contends for nothing.
    pub fn stake(&self, change: &Change) -> Option<(String, String, String)> {
        if change.writes_a_field() {
            return Some((
                change.object_ref.clone(),
                change.field.clone(),
                change.value.clone(),
            ));
        }
        change.changes_topology().then(|| {
            (
                self.line_of(change),
                FIELD_TOPOLOGY.to_owned(),
                change.raw.to_string(),
            )
        })
    }

    /// The change with what the server decides filled in: a pipe run that
    /// came without a tag gets the line's next one (`{line}-{NN}`), so that
    /// every client — and a replay of the log — calls it the same.
    ///
    /// And a symbol placed under a tag an object of this drawing already
    /// has gets the next free one. Two people who place a symbol at the same
    /// moment both work the same number out — neither has seen the other's
    /// yet — and it is here, where the two meet, that one of them is told
    /// another (`VAL-CROSS-010`). The tag it came with rides along as
    /// `reassigned_from`, which is how the session that placed it knows to
    /// rename its own.
    ///
    /// A tag in use in a client's own copy of the drawing
    /// ([`Self::report_tags`]) is taken the same way: a symbol placed under
    /// it gets the next free one, and neither a symbol nor a run is given
    /// it.
    pub fn completed(&self, change: &Change) -> Change {
        let mut change = change.clone();
        if change.kind == KIND_ADD_PIPE_RUN && change.text("tag").is_none() {
            let line = self.line_of(&change);
            let taken = self.lines.get(&line).map_or(0, Vec::len);
            let tag = (taken + 1..)
                .map(|n| format!("{line}-{n:02}"))
                .find(|tag| !self.runs.contains_key(tag) && !self.reported.contains(tag))
                .unwrap_or_default();
            change.raw["tag"] = Value::String(tag);
        }
        if change.kind == KIND_INSERT_SYMBOL {
            let tag = change.subject();
            if !tag.is_empty() && self.is_taken(&tag) {
                let free = self.free_tag_after(&tag);
                change.raw["reassigned_from"] = Value::String(tag);
                change.raw["tag"] = Value::String(free.clone());
                change.raw["object_ref"] = Value::String(free.clone());
                change.object_ref = free;
            }
        }
        change
    }

    /// The first tag after `taken` that is free ([`Self::is_taken`]): the
    /// number `taken` ends in counted up, in as many digits (`V-1001` →
    /// `V-1002`), or `-2`, `-3`, … appended to one that ends in no number.
    fn free_tag_after(&self, taken: &str) -> String {
        let digits = taken
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .count();
        let (stem, number) = taken.split_at(taken.len() - digits);
        let free = |tag: &String| !self.is_taken(tag);
        match number.parse::<u64>() {
            Ok(from) => (from + 1..)
                .map(|n| format!("{stem}{n:0digits$}"))
                .find(free),
            Err(_) => (2u64..).map(|n| format!("{taken}-{n}")).find(free),
        }
        .unwrap_or_else(|| taken.to_owned())
    }

    /// An object that was deleted, by the tag it had.
    pub fn deleted(&self, object_ref: &str) -> Option<&Deleted> {
        self.deleted.get(object_ref)
    }

    /// A client that joins lists the tags the objects of its own copy of
    /// the drawing carry (`hello.existing_tags`), as that copy was at
    /// `revision` of this room. Two people who each opened a file of their
    /// own need not have the same objects in it, and all the server knows
    /// of a drawing is the ops it applied: this is how it learns of a tag
    /// that is in use in a copy it never saw. Such a tag is taken from here
    /// on — [`Self::completed`] gives a symbol placed under it another, and
    /// gives it to nothing — until an op deletes the object or gives it
    /// another tag, which takes it away from everyone who has it.
    ///
    /// A tag an op has told of is known already. And one the room deleted
    /// or renamed away after `revision` is about to leave that copy too —
    /// the replay behind the `welcome` says so — and is not taken on its
    /// account.
    ///
    /// Returns the tags that are news, in the order they were listed.
    pub fn report_tags<I, S>(&mut self, tags: I, revision: i64) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut news = Vec::new();
        for tag in tags {
            let tag = tag.as_ref();
            if tag.is_empty() || self.knows(tag) || self.let_go_after(tag, revision) {
                continue;
            }
            if self.reported.insert(tag.to_owned()) {
                news.push(tag.to_owned());
            }
        }
        news
    }

    /// The tags only a client's listing tells of ([`Self::report_tags`]),
    /// in order.
    pub fn reported_tags(&self) -> impl Iterator<Item = &String> {
        self.reported.iter().filter(|tag| !self.knows(tag))
    }

    /// Whether an op told of something that goes by `tag`.
    fn knows(&self, tag: &str) -> bool {
        self.objects.contains_key(tag)
            || self.runs.contains_key(tag)
            || self.lines.contains_key(tag)
    }

    /// Whether the room deleted what went by `tag`, or gave it another tag,
    /// after `revision`.
    fn let_go_after(&self, tag: &str, revision: i64) -> bool {
        let deleted = self.deleted.get(tag);
        let renamed = |(old, _, at): &(String, String, i64)| old == tag && *at > revision;
        deleted.is_some_and(|gone| gone.revision > revision) || self.renames.iter().any(renamed)
    }

    /// Whether a symbol cannot go by `tag`: an object of the drawing has
    /// it, or one in a client's own copy does.
    fn is_taken(&self, tag: &str) -> bool {
        self.objects.contains_key(tag) || self.reported.contains(tag)
    }

    /// The pipe topology, for the read-back endpoint.
    pub fn topology(&self) -> Value {
        let lines: Vec<Value> = self
            .lines
            .iter()
            .map(|(tag, runs)| serde_json::json!({"tag": tag, "runs": runs}))
            .collect();
        let runs: Vec<Value> = self
            .runs
            .iter()
            .map(|(tag, run)| {
                serde_json::json!({
                    "tag": tag,
                    "line_tag": run.line_tag,
                    "points": run.points,
                    "from": run.from,
                    "to": run.to,
                    "fittings": run.fittings,
                    "segments": run.segments,
                    "props": run.props,
                    "overrides": run.overrides.iter().collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::json!({
            "pipe_lines": lines,
            "pipe_runs": runs,
            "connectors": self.connectors,
        })
    }

    /// Take the account of a run a change carries (`run`: its segments and
    /// what is fitted on it, as they are once the change has been made) as
    /// what the run is now. Returns whether the change carried one.
    ///
    /// A fitting with a tag of its own — a valve — is an object of the
    /// drawing from here on: a symbol placed under that tag later is given
    /// another, and a deletion finds it.
    fn take_run_account(&mut self, run_tag: &str, change: &Change, state: &FieldState) -> bool {
        let Some(account) = change.raw.get("run").and_then(Value::as_object) else {
            return false;
        };
        let fittings: Vec<Value> = account
            .get("fittings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for fitting in &fittings {
            let tag = fitting.get("tag").and_then(Value::as_str).unwrap_or_default();
            if !tag.is_empty() && tag != run_tag {
                let type_code = fitting.get("type").and_then(Value::as_str);
                self.know_fitting(tag, type_code, state);
            }
        }
        if let Some(run) = self.runs.get_mut(run_tag) {
            run.fittings = fittings;
            run.segments = account.get("segments").cloned().unwrap_or(Value::Null);
            if let Some(props) = account.get("props").and_then(Value::as_object) {
                run.props.clear();
                for (k, v) in props {
                    run.props.insert(k.clone(), value_text(v));
                }
            }
            if let Some(ovs) = account.get("overrides").and_then(Value::as_array) {
                run.overrides = ovs
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
        }
        true
    }

    /// A fitting that has a tag of its own is an object under that tag.
    fn know_fitting(&mut self, tag: &str, type_code: Option<&str>, state: &FieldState) {
        self.deleted.remove(tag);
        let object = self.objects.entry(tag.to_owned()).or_default();
        object
            .fields
            .entry("type".to_owned())
            .or_insert_with(|| FieldState {
                value: type_code.unwrap_or("VALVE").to_owned(),
                ..state.clone()
            });
    }

    /// Record a change that took effect and is not a field write: an object
    /// created or deleted, or a pipe line's connections changed.
    fn commit_structure(&mut self, change: &Change, object_ref: &str, state: &FieldState) {
        let stamp = |value: &str| FieldState {
            value: value.to_owned(),
            ..state.clone()
        };
        match change.kind.as_str() {
            KIND_DELETE_OBJECT => {
                // Gone from every copy that had it.
                self.reported.remove(object_ref);
                let fields = self
                    .objects
                    .remove(object_ref)
                    .map(|object| object.fields)
                    .unwrap_or_default();
                self.deleted.insert(
                    object_ref.to_owned(),
                    Deleted {
                        revision: state.revision,
                        session_id: state.session_id.clone(),
                        user_id: state.user_id.clone(),
                        fields,
                    },
                );
                // A pipe line goes with its runs; a run leaves its line.
                if let Some(runs) = self.lines.remove(object_ref) {
                    for run in runs {
                        self.runs.remove(&run);
                    }
                }
                if let Some(run) = self.runs.remove(object_ref) {
                    if let Some(runs) = self.lines.get_mut(&run.line_tag) {
                        runs.retain(|tag| tag != object_ref);
                    }
                }
                // A fitting that had a tag of its own leaves the run it was on.
                for run in self.runs.values_mut() {
                    run.fittings.retain(|fitting| {
                        fitting.get("tag").and_then(Value::as_str) != Some(object_ref)
                    });
                }
                // #31: Connectors standing on a deleted pipe/signal line go with it.
                self.connectors.retain(|known| {
                    crate::link::Connector::read(known)
                        .is_none_or(|c| c.line_tag != object_ref)
                });
            }
            KIND_INSERT_SYMBOL | KIND_CREATE_PIPE_LINE => {
                self.deleted.remove(object_ref);
                let object = self.objects.entry(object_ref.to_owned()).or_default();
                if let Some(src) = change.text("tag_source") {
                    object.tag_source = Some(src);
                }
                if let Some(type_code) = change.text("type") {
                    object.fields.insert("type".to_owned(), stamp(&type_code));
                }
                if let Some(props) = change.raw.get("props").and_then(Value::as_object) {
                    for (field, value) in props {
                        object.fields.insert(field.clone(), stamp(&value_text(value)));
                    }
                }
                // A signal line comes into being the same way and is known by
                // its tag like any object, but it is not a pipe line: it has
                // no runs, and the topology does not list it as one.
                let signal = change.text("type").as_deref() == Some(TYPE_SIGNAL_LINE);
                if change.kind == KIND_CREATE_PIPE_LINE && !signal {
                    self.lines.entry(object_ref.to_owned()).or_default();
                }
            }
            KIND_ADD_PIPE_RUN => {
                let Some(tag) = change.text("tag") else {
                    return;
                };
                let runs = self.lines.entry(object_ref.to_owned()).or_default();
                if !runs.contains(&tag) {
                    runs.push(tag.clone());
                }
                self.runs.insert(
                    tag.clone(),
                    Run {
                        line_tag: object_ref.to_owned(),
                        points: change.raw.get("points").cloned().unwrap_or(Value::Null),
                        from: change.text("from"),
                        to: change.text("to"),
                        fittings: Vec::new(),
                        segments: Value::Null,
                        props: BTreeMap::new(),
                        overrides: BTreeSet::new(),
                    },
                );
                self.take_run_account(&tag, change, state);
            }
            KIND_INSERT_FITTING | KIND_UPDATE_PIPE_RUN => {
                let run = change.text("run_tag").unwrap_or_default();
                let runs = self.lines.entry(object_ref.to_owned()).or_default();
                if !runs.contains(&run) {
                    runs.push(run.clone());
                }
                self.runs.entry(run.clone()).or_insert_with(|| Run {
                    line_tag: object_ref.to_owned(),
                    ..Run::default()
                });
                // With an account of the run, the account is what the run is
                // now. Without one — a client that sends the bare op of
                // `architecture.md` §3.2 — a fitting is one more on the run.
                if !self.take_run_account(&run, change, state)
                    && change.kind == KIND_INSERT_FITTING
                {
                    if let Some(tag) = change.text("tag").filter(|tag| *tag != run) {
                        self.know_fitting(&tag, change.text("type").as_deref(), state);
                    }
                    if let Some(entry) = self.runs.get_mut(&run) {
                        entry.fittings.push(change.raw.clone());
                    }
                }
            }
            KIND_CREATE_CONNECTOR => {
                let said = crate::link::Connector::read(&change.raw);
                // What the drawing of whoever made it has for the fields
                // that go along it: all the server knows of a line nobody
                // has written a field of while it was listening. A value it
                // has heard written stands.
                if let (Some(said), Some(props)) = (
                    &said,
                    change.raw.get("line_props").and_then(Value::as_object),
                ) {
                    let object = self.objects.entry(object_ref.to_owned()).or_default();
                    for (field, value) in props {
                        if said.carried_fields().contains(&field.as_str()) {
                            object
                                .fields
                                .entry(field.clone())
                                .or_insert_with(|| stamp(&value_text(value)));
                        }
                    }
                }
                // Said again (its making put on again), it is the one
                // connector it was.
                if let Some(said) = &said {
                    self.connectors.retain(|known| {
                        crate::link::Connector::read(known).is_none_or(|known| !known.is(said))
                    });
                }
                let mut entry = change.raw.clone();
                if let Some(entry) = entry.as_object_mut() {
                    entry.remove("line_props");
                }
                self.connectors.push(entry);
            }
            KIND_DELETE_CONNECTOR => {
                if let Some(gone) = crate::link::Connector::read(&change.raw) {
                    self.connectors.retain(|known| {
                        crate::link::Connector::read(known).is_none_or(|known| !known.is(&gone))
                    });
                }
            }
            KIND_PAIR_CONNECTOR => {
                if let Some(said) = crate::link::Connector::read(&change.raw) {
                    for known in &mut self.connectors {
                        if crate::link::Connector::read(known).is_some_and(|known| known.is(&said)) {
                            for key in ["peer_drawing", "peer_line_tag", "peer_connector"] {
                                known[key] = change.raw.get(key).cloned().unwrap_or(Value::Null);
                            }
                        }
                    }
                }
                return;
            }
            _ => return,
        }
        if change.changes_topology() {
            self.line_touched.insert(
                object_ref.to_owned(),
                (state.revision, state.session_id.clone()),
            );
        }
    }
}

/// Step 3's condition between two field writes made against one baseline,
/// neither of which has taken effect: the same field of the same object, to
/// different values. Which baseline, and that both are still pending, is the
/// caller's to establish.
pub fn collides(a: &Change, b: &Change) -> bool {
    a.writes_a_field()
        && b.writes_a_field()
        && a.object_ref == b.object_ref
        && a.field == b.field
        && a.value != b.value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(object_ref: &str, field: &str, value: &str) -> Change {
        Change::list(&json!({
            "kind": KIND_SET_PROPERTY, "object_ref": object_ref, "field": field, "value": value,
        }))
        .remove(0)
    }

    fn retag(old: &str, new: &str) -> Change {
        Change::list(&json!({"kind": KIND_SET_TAG, "object_ref": old, "value": new})).remove(0)
    }

    /// A model in which `session` wrote `field` of P-1001 at `revision`.
    fn written(field: &str, value: &str, revision: i64, session: &str) -> DrawingModel {
        let mut model = DrawingModel::default();
        model.commit(&set("P-1001", field, value), "P-1001", revision, session, "u1");
        model
    }

    fn applies(verdict: &Verdict) -> bool {
        matches!(verdict, Verdict::Apply { .. })
    }

    // Step 1 (`VAL-CONF-001`): s1 wrote 设计压力 at revision 1; s2, still at
    // baseline 0, writes 公称直径 of the same object. Nothing to decide.
    #[test]
    fn a_field_nobody_else_wrote_since_the_baseline_merges() {
        let model = written("设计压力", "1.6MPa", 1, "s1");
        let verdict = model.judge(&set("P-1001", "公称直径", "DN50"), 0, "s2");
        assert_eq!(
            verdict,
            Verdict::Apply {
                object_ref: "P-1001".into(),
                replaces: None
            }
        );
    }

    // Step 2 (`VAL-CONF-002`, `-005`): the same field, written by another
    // session after the change's baseline.
    #[test]
    fn a_field_someone_else_wrote_since_the_baseline_is_kept_and_the_change_yields() {
        let model = written("设计压力", "1.6MPa", 1, "s1");
        assert_eq!(
            model.judge(&set("P-1001", "设计压力", "2.5MPa"), 0, "s2"),
            Verdict::Yield {
                object_ref: "P-1001".into(),
                winner: "1.6MPa".into(),
                winner_revision: 1,
                winner_user: "u1".into(),
            }
        );
        // `VAL-CONF-005`: written at revision 3, the change carries baseline 1.
        let model = written("公称直径", "DN50", 3, "s1");
        assert!(!applies(&model.judge(&set("P-1001", "公称直径", "DN80"), 1, "s2")));
    }

    // A change that has seen the current value simply replaces it — the
    // ordinary edit — and says what it replaced.
    #[test]
    fn a_change_made_against_the_current_value_replaces_it() {
        let model = written("设计压力", "1.6MPa", 3, "s1");
        assert_eq!(
            model.judge(&set("P-1001", "设计压力", "2.5MPa"), 3, "s2"),
            Verdict::Apply {
                object_ref: "P-1001".into(),
                replaces: Some("1.6MPa".into())
            }
        );
    }

    // Two people arriving at the same value disagree about nothing.
    #[test]
    fn the_same_value_never_yields() {
        let model = written("设计压力", "1.6MPa", 3, "s1");
        assert_eq!(
            model.judge(&set("P-1001", "设计压力", "1.6MPa"), 0, "s2"),
            Verdict::Apply {
                object_ref: "P-1001".into(),
                replaces: None
            }
        );
    }

    // A session is not behind its own writes: a second edit sent before the
    // first came back as `op_applied` carries the older baseline and is still
    // the newer intent.
    #[test]
    fn a_session_does_not_yield_to_its_own_earlier_write() {
        let model = written("设计压力", "1.6MPa", 1, "s1");
        assert_eq!(
            model.judge(&set("P-1001", "设计压力", "2.5MPa"), 0, "s1"),
            Verdict::Apply {
                object_ref: "P-1001".into(),
                replaces: Some("1.6MPa".into())
            }
        );
    }

    #[test]
    fn an_object_never_written_takes_every_field() {
        let model = DrawingModel::default();
        for field in ["设计压力", "公称直径", "绝热等级"] {
            assert!(applies(&model.judge(&set("P-1001", field, "x"), 0, "s1")));
        }
    }

    #[test]
    fn committing_replaces_the_value_and_keeps_one_entry_per_field() {
        let mut model = written("设计压力", "1.6MPa", 1, "s1");
        model.commit(&set("P-1001", "设计压力", "2.5MPa"), "P-1001", 5, "s2", "u2");
        let state = model.field("P-1001", "设计压力").expect("the field");
        assert_eq!(
            (state.value.as_str(), state.revision, state.user_id.as_str()),
            ("2.5MPa", 5, "u2")
        );
        assert_eq!(model.object("P-1001").expect("object").fields.len(), 1);
        assert_eq!(model.revision(), 5);
    }

    // The question a retag raises: the op names the tag the object *had*.
    // Afterwards it is known by the new one — and a change made by someone
    // who had not seen the retag still names the old one. It must reach the
    // same object, and go out naming it as everyone now knows it.
    #[test]
    fn a_change_made_before_a_retag_still_finds_the_object() {
        let mut model = written("设计压力", "1.6MPa", 1, "s1");
        model.commit(&retag("P-1001", "P-1009"), "P-1001", 2, "s1", "u1");
        assert!(model.object("P-1001").is_none(), "known by the new tag only");
        assert_eq!(
            model.field("P-1009", "设计压力").map(|f| f.value.as_str()),
            Some("1.6MPa"),
            "its fields moved with it"
        );

        // s2 had seen revision 1, not the retag at 2.
        assert_eq!(
            model.judge(&set("P-1001", "公称直径", "DN50"), 1, "s2"),
            Verdict::Apply {
                object_ref: "P-1009".into(),
                replaces: None
            }
        );
        // Someone who has seen the retag and says P-1001 means whatever is
        // called P-1001 now — a different object.
        assert_eq!(
            model.judge(&set("P-1001", "公称直径", "DN50"), 2, "s2"),
            Verdict::Apply {
                object_ref: "P-1001".into(),
                replaces: None
            }
        );
        // And a chain is followed to its end.
        model.commit(&retag("P-1009", "P-1010"), "P-1009", 3, "s1", "u1");
        assert_eq!(model.current_name("P-1001", 1), "P-1010");
        assert_eq!(model.current_name("P-1009", 2), "P-1010");
        assert_eq!(model.current_name("P-1010", 3), "P-1010");
    }

    // Two retags of one object made against the same baseline: the tag is a
    // field like any other, and the later one yields.
    #[test]
    fn a_retag_behind_another_retag_yields() {
        let mut model = DrawingModel::default();
        model.commit(&retag("P-1001", "P-1009"), "P-1001", 1, "s1", "u1");
        assert_eq!(
            model.judge(&retag("P-1001", "P-1005"), 0, "s2"),
            Verdict::Yield {
                object_ref: "P-1009".into(),
                winner: "P-1009".into(),
                winner_revision: 1,
                winner_user: "u1".into(),
            }
        );
    }

    // Step 3's condition (`VAL-CONF-004`).
    #[test]
    fn two_writes_collide_only_on_one_field_of_one_object_with_different_values() {
        let a = set("P-1001", "设计压力", "1.6MPa");
        assert!(collides(&a, &set("P-1001", "设计压力", "2.5MPa")));
        assert!(!collides(&a, &set("P-1001", "设计压力", "1.6MPa")), "same value");
        assert!(!collides(&a, &set("P-1001", "公称直径", "DN50")), "another field");
        assert!(!collides(&a, &set("P-1002", "设计压力", "2.5MPa")), "another object");
        assert!(collides(&retag("P-1001", "P-1005"), &retag("P-1001", "P-1009")));
    }

    // One undo step is one op: the first change is the payload, the rest ride
    // under `changes`. Every one of them is read; the batch is not part of
    // the first change.
    #[test]
    fn a_payload_lists_its_own_change_then_the_batched_ones() {
        let payload = json!({
            "kind": "set_tag", "object_ref": "P-1001", "value": "P-1009", "old_value": "P-1001",
            "handle": "1A",
            "changes": [
                {"kind": "set_property", "object_ref": "P-1009", "field": "设计压力",
                 "value": "2.5MPa", "old_value": "1.6MPa"},
                {"kind": "set_property", "object_ref": "P-1009", "field": "公称直径", "value": 50},
            ],
        });
        let changes = Change::list(&payload);
        assert_eq!(changes.len(), 3);
        assert_eq!(
            (changes[0].kind.as_str(), changes[0].field.as_str(), changes[0].value.as_str()),
            ("set_tag", "tag", "P-1009")
        );
        assert!(changes[0].raw.get("changes").is_none());
        assert_eq!(changes[0].raw["handle"], "1A");
        assert_eq!(changes[1].old_value.as_deref(), Some("1.6MPa"));
        assert_eq!(changes[2].value, "50", "a number is read as its text");
        assert!(changes.iter().all(Change::writes_a_field));

        assert!(Change::list(&json!("not an object")).is_empty());
        let opaque = Change::list(&json!({"kind": "insert_symbol", "tag": "V-1001"}));
        assert_eq!(opaque.len(), 1);
        assert!(!opaque[0].writes_a_field());
    }

    #[test]
    fn the_broadcast_form_names_the_object_as_it_is_known_now() {
        let change = set("P-1001", "设计压力", "2.5MPa");
        let out = change.broadcast_form("P-1009");
        assert_eq!(out["object_ref"], "P-1009");
        assert_eq!(out["field"], "设计压力");
        assert_eq!(out["value"], "2.5MPa");
        let bare = Change::list(&json!({"kind": "set_tag", "object_ref": "V-1", "value": "V-2"}))
            .remove(0);
        assert_eq!(bare.broadcast_form("V-1")["field"], Value::Null, "backfilled");
    }

    fn one(payload: Value) -> Change {
        Change::list(&payload).remove(0)
    }

    fn delete(object_ref: &str) -> Change {
        one(json!({"kind": KIND_DELETE_OBJECT, "object_ref": object_ref}))
    }

    /// Judge and, when it applies, commit — as the server does.
    fn take(model: &mut DrawingModel, change: &Change, base: i64, revision: i64, session: &str) -> Verdict {
        let change = model.completed(change);
        let verdict = model.judge(&change, base, session);
        if let Verdict::Apply { object_ref, .. } = &verdict {
            model.commit(&change, object_ref, revision, session, session);
        }
        verdict
    }

    // The object level (`VAL-CONF-006`): s1 deletes P-1002 at revision 2; s2,
    // who has not seen that, changes one of its fields. The deletion stands,
    // and s2 is told what the field held.
    #[test]
    fn a_change_to_an_object_someone_else_deleted_since_the_baseline_does_not_happen() {
        let mut model = DrawingModel::default();
        model.commit(&set("P-1002", "设计压力", "1.0MPa"), "P-1002", 1, "s1", "u1");
        assert!(applies(&take(&mut model, &delete("P-1002"), 1, 2, "s1")));
        assert!(model.object("P-1002").is_none(), "gone from the drawing");
        assert_eq!(model.deleted("P-1002").map(|gone| gone.revision), Some(2));

        assert_eq!(
            model.judge(&set("P-1002", "设计压力", "2.5MPa"), 1, "s2"),
            Verdict::Gone {
                object_ref: "P-1002".into(),
                deleted_revision: 2,
                deleted_by: "s1".into(),
                last_value: Some("1.0MPa".into()),
            }
        );
        // A field it never carried: still gone, with nothing to report.
        match model.judge(&set("P-1002", "公称直径", "DN50"), 1, "s2") {
            Verdict::Gone { last_value, .. } => assert_eq!(last_value, None),
            other => panic!("expected Gone, got {other:?}"),
        }
        // Someone who has seen the deletion and writes P-1002 anyway is
        // making it anew.
        assert!(applies(&take(&mut model, &set("P-1002", "设计压力", "0.6MPa"), 2, 3, "s2")));
        assert!(model.deleted("P-1002").is_none());
        assert!(model.object("P-1002").is_some());
    }

    // `VAL-CONF-017`: P-1003 sits at an end of one of L-1001's runs. It
    // cannot be deleted, and the refusal names what holds it.
    #[test]
    fn an_object_at_the_end_of_a_pipe_run_cannot_be_deleted() {
        let mut model = DrawingModel::default();
        let run = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001",
            "points": [[0, 0], [100, 0]], "from": "P-1003", "to": "V-1002"}));
        assert!(applies(&take(&mut model, &run, 0, 1, "s1")));
        assert_eq!(
            model.judge(&delete("P-1003"), 1, "s2"),
            Verdict::Referenced {
                object_ref: "P-1003".into(),
                by: vec!["L-1001".into()]
            }
        );
        assert!(applies(&model.judge(&delete("P-1004"), 1, "s2")), "nothing holds it");
        // With the line gone nothing holds P-1003 any more.
        assert!(applies(&take(&mut model, &delete("L-1001"), 1, 2, "s1")));
        assert!(applies(&model.judge(&delete("P-1003"), 2, "s2")));
        assert_eq!(model.topology()["pipe_runs"], json!([]));
    }

    // A run that comes without a tag is given the line's next one, and the
    // topology the server reports is what the ops built.
    #[test]
    fn pipe_runs_are_numbered_under_their_line_and_show_in_the_topology() {
        let mut model = DrawingModel::default();
        let line = one(json!({"kind": KIND_CREATE_PIPE_LINE, "tag": "L-1001",
            "props": {"管道等级": "A1A"}}));
        assert!(applies(&take(&mut model, &line, 0, 1, "s1")));
        assert_eq!(model.field("L-1001", "管道等级").map(|f| f.value.as_str()), Some("A1A"));

        let run = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "points": [[0, 0], [100, 0]]}));
        assert_eq!(model.completed(&run).text("tag").as_deref(), Some("L-1001-01"));
        take(&mut model, &run, 1, 2, "s1");
        assert_eq!(model.completed(&run).text("tag").as_deref(), Some("L-1001-02"));
        let named = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "tag": "L-1001-07", "points": []}));
        assert_eq!(model.completed(&named).text("tag").as_deref(), Some("L-1001-07"), "a given tag is kept");

        let fitting = one(json!({"kind": KIND_INSERT_FITTING, "run_tag": "L-1001-01",
            "fitting_type": "gate_valve", "at": [50, 0]}));
        assert_eq!(model.line_of(&fitting), "L-1001");
        take(&mut model, &fitting, 2, 3, "s1");
        let topology = model.topology();
        assert_eq!(topology["pipe_lines"], json!([{"tag": "L-1001", "runs": ["L-1001-01"]}]));
        assert_eq!(topology["pipe_runs"][0]["fittings"][0]["fitting_type"], "gate_valve");
        // A run the server never saw belongs to the line its tag is built on.
        let unseen = one(json!({"kind": KIND_INSERT_FITTING, "run_tag": "L-2002-03", "at": [1, 1]}));
        assert_eq!(model.line_of(&unseen), "L-2002");
    }

    // The topology level. Connections do not merge field by field: a change
    // made before another session's change to the same line is behind it,
    // and two made against one baseline contend for the line as a whole
    // (`VAL-CONF-008`).
    #[test]
    fn changes_to_one_lines_connections_contend_for_the_line() {
        let mut model = DrawingModel::default();
        let run = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "points": [[0, 0], [100, 0]]}));
        let fitting = one(json!({"kind": KIND_INSERT_FITTING, "run_tag": "L-1001-01", "at": [50, 0]}));

        let (a, b) = (model.stake(&run).expect("stake"), model.stake(&fitting).expect("stake"));
        assert_eq!((a.0.as_str(), a.1.as_str()), ("L-1001", FIELD_TOPOLOGY));
        assert_eq!((b.0.as_str(), b.1.as_str()), ("L-1001", FIELD_TOPOLOGY));
        assert_ne!(a.2, b.2, "each with its own description");
        assert!(!a.2.is_empty() && !b.2.is_empty());
        let elsewhere = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1002", "points": []}));
        assert_ne!(model.stake(&elsewhere).expect("stake").0, a.0);
        assert_eq!(model.stake(&delete("P-1")), None, "a deletion contends for nothing");

        take(&mut model, &run, 0, 1, "s1");
        assert_eq!(
            model.judge(&fitting, 0, "s2"),
            Verdict::BehindTopology {
                line_tag: "L-1001".into(),
                touched_revision: 1
            }
        );
        assert!(applies(&model.judge(&fitting, 1, "s2")), "having seen it");
        assert!(applies(&model.judge(&fitting, 0, "s1")), "its own change");
        assert!(applies(&model.judge(&elsewhere, 0, "s2")), "another line");
    }

    #[test]
    fn a_symbol_inserted_is_known_by_its_tag_with_its_type_and_props() {
        let mut model = DrawingModel::default();
        let insert = one(json!({"kind": KIND_INSERT_SYMBOL, "type": "VALVE", "tag": "V-1001",
            "position": [100, 200], "props": {"公称直径": "DN50"}}));
        assert!(insert.is_understood());
        assert_eq!(insert.subject(), "V-1001");
        assert!(applies(&take(&mut model, &insert, 0, 1, "s1")));
        assert_eq!(model.field("V-1001", "type").map(|f| f.value.as_str()), Some("VALVE"));
        assert_eq!(model.field("V-1001", "公称直径").map(|f| f.value.as_str()), Some("DN50"));
        assert!(!one(json!({"kind": "something_else", "tag": "X"})).is_understood());
        assert!(!one(json!({"kind": KIND_INSERT_SYMBOL})).is_understood(), "names nothing");
    }

    /// `VAL-CROSS-010`: two sessions place a symbol under the same tag —
    /// each worked the number out before it had seen the other's. The first
    /// keeps it; the second is given the next free one, and its change says
    /// which tag it came with, so the session that placed it can rename its
    /// own. A tag that is free is left alone, and one freed by a deletion
    /// can be taken again.
    #[test]
    fn a_symbol_placed_under_a_taken_tag_is_given_the_next_free_one() {
        let mut model = DrawingModel::default();
        let place = |tag: &str| {
            one(json!({"kind": KIND_INSERT_SYMBOL, "type": "VALVE", "tag": tag,
                "object_ref": tag, "handle": "3A1", "position": [100, 200]}))
        };
        let first = model.completed(&place("V-1001"));
        assert_eq!(first.subject(), "V-1001");
        assert!(first.raw.get("reassigned_from").is_none());
        assert!(applies(&take(&mut model, &first, 0, 1, "s1")));

        let second = model.completed(&place("V-1001"));
        assert_eq!(second.subject(), "V-1002");
        assert_eq!(second.raw["tag"], "V-1002");
        assert_eq!(second.raw["object_ref"], "V-1002");
        assert_eq!(second.raw["reassigned_from"], "V-1001");
        assert_eq!(second.raw["handle"], "3A1", "the sender's entity still rides along");
        assert!(applies(&take(&mut model, &second, 0, 2, "s2")));
        assert!(model.object("V-1001").is_some() && model.object("V-1002").is_some());

        // The next one counts past both; the digits keep their width.
        assert_eq!(model.completed(&place("V-1001")).subject(), "V-1003");
        let _ = take(&mut model, &place("P-0099"), 2, 3, "s1");
        assert_eq!(model.completed(&place("P-0099")).subject(), "P-0100");
        // A tag that ends in no number gets one appended.
        let _ = take(&mut model, &place("PUMP"), 3, 4, "s1");
        assert_eq!(model.completed(&place("PUMP")).subject(), "PUMP-2");

        // Deleted, the tag is free again.
        let _ = take(&mut model, &delete("V-1001"), 4, 5, "s1");
        let again = model.completed(&place("V-1001"));
        assert_eq!(again.subject(), "V-1001");
        assert!(again.raw.get("reassigned_from").is_none());
    }

    /// A symbol placed under `tag`.
    fn place(tag: &str) -> Change {
        one(json!({"kind": KIND_INSERT_SYMBOL, "type": "VALVE", "tag": tag, "position": [0, 0]}))
    }

    /// The tag a symbol placed under `tag` would be given.
    fn placed(model: &DrawingModel, tag: &str) -> String {
        model.completed(&place(tag)).subject()
    }

    /// The server knows a drawing by the ops it applied, and two people who
    /// each opened a file of their own need not have the same objects in
    /// it. A client that joins lists the tags its copy carries
    /// (`hello.existing_tags`), and from then on they are taken like an
    /// object's: a symbol somebody places under one is given the next free
    /// tag, and no tag the server gives out is one of them. It used to give
    /// a symbol a tag that one side's drawing had in use already.
    #[test]
    fn collab_existing_tags_a_tag_only_a_clients_copy_has_is_taken() {
        let mut model = DrawingModel::default();
        // Nobody has said anything: the tag is free, as it always was.
        assert_eq!(placed(&model, "V-1002"), "V-1002");

        // One client's file has V-1002 and V-1003 in it. No tag is nobody's.
        let news = model.report_tags(["V-1002", "V-1003", ""], 0);
        assert_eq!(news, ["V-1002", "V-1003"]);
        let listed: Vec<&String> = model.reported_tags().collect();
        assert_eq!(listed, ["V-1002", "V-1003"]);
        assert!(model.object("V-1002").is_none(), "nothing else is known");

        // Placed under one of them: the next tag that neither an object nor
        // a copy has.
        let moved = model.completed(&place("V-1002"));
        assert_eq!(moved.subject(), "V-1004");
        assert_eq!(moved.raw["reassigned_from"], "V-1002");
        // One that runs into an object counts past them as well.
        assert!(applies(&take(&mut model, &place("V-1001"), 0, 1, "s1")));
        assert_eq!(placed(&model, "V-1001"), "V-1004");

        // Listed again, and with it a tag an op has told of: nothing new.
        assert!(model.report_tags(["V-1003", "V-1001"], 1).is_empty());
        assert_eq!(model.reported_tags().count(), 2);

        // A run the server names is not given one of them either.
        let run = json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "points": []});
        let named = |model: &DrawingModel| model.completed(&one(run.clone())).text("tag");
        assert_eq!(named(&model).as_deref(), Some("L-1001-01"));
        model.report_tags(["L-1001-01", "L-1001-02"], 1);
        assert_eq!(named(&model).as_deref(), Some("L-1001-03"));
    }

    /// A listed tag is let go of the way an object's is: an op that deletes
    /// the object, or gives it another tag, takes it away from everyone who
    /// has it, and the tag is free again.
    #[test]
    fn collab_existing_tags_a_listed_tag_is_free_again_once_an_op_takes_it_away() {
        let mut model = DrawingModel::default();
        model.report_tags(["V-1001", "V-1002"], 0);
        assert_eq!(placed(&model, "V-1001"), "V-1003", "both are taken");

        assert!(applies(&take(&mut model, &delete("V-1001"), 0, 1, "s1")));
        assert_eq!(placed(&model, "V-1001"), "V-1001", "deleted");
        let renamed = retag("V-1002", "V-2002");
        assert!(applies(&take(&mut model, &renamed, 1, 2, "s1")));
        assert_eq!(placed(&model, "V-1002"), "V-1002", "renamed away");
        assert_eq!(placed(&model, "V-2002"), "V-2003", "the object's now");
        assert_eq!(model.reported_tags().count(), 0);
    }

    /// What a client lists is its copy as it was at the revision it says it
    /// had reached (`hello.since`). A tag the room deleted or renamed away
    /// after that is about to leave that copy too — the replay says so —
    /// and is not taken on its account. One the room let go of by that
    /// revision the client has all the same: its copy is another, and the
    /// tag is taken.
    #[test]
    fn collab_existing_tags_what_the_room_took_away_since_is_not_taken_again() {
        let mut model = DrawingModel::default();
        assert!(applies(&take(&mut model, &place("V-1001"), 0, 1, "s1")));
        assert!(applies(&take(&mut model, &place("V-1002"), 1, 2, "s1")));
        assert!(applies(&take(&mut model, &delete("V-1001"), 2, 3, "s1")));
        let renamed = retag("V-1002", "V-2002");
        assert!(applies(&take(&mut model, &renamed, 3, 4, "s1")));

        // Away since revision 2: it has both, and is about to lose both.
        assert!(model.report_tags(["V-1001", "V-1002"], 2).is_empty());
        // Having seen the deletion (3), and not the retag (4).
        assert_eq!(model.report_tags(["V-1001", "V-1002"], 3), ["V-1001"]);
        // Having seen both, and with the old tag in its copy still.
        assert_eq!(model.report_tags(["V-1001", "V-1002"], 4), ["V-1002"]);
        assert_eq!(placed(&model, "V-1001"), "V-1003");
    }

    /// The account of a run a desktop client sends with a change to it.
    fn account(fittings: Value) -> Value {
        json!({"start": [0, 0], "segments": [{"start": [0, 0], "end": [84, 0]}, {"start": [116, 0], "end": [200, 0]}],
            "fittings": fittings})
    }

    /// `VAL-CROSS-003`: a change that says what the run is like afterwards
    /// (`run`) makes that the run — its fittings are the ones listed, not
    /// one more on top of what was there — and a valve among them is an
    /// object under its own tag: the next symbol placed under that tag is
    /// given another, and deleting it takes it off the run.
    #[test]
    fn collab_pipe_ops_an_account_of_a_run_is_what_the_run_is_now() {
        let mut model = DrawingModel::default();
        let valve = json!({"fitting_type": "gate_valve", "type": "VALVE", "tag": "V-1001", "at": [100, 0]});
        let run = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "tag": "L-1001-01",
            "points": [[0, 0], [200, 0]], "run": account(json!([]))}));
        assert!(applies(&take(&mut model, &run, 0, 1, "s1")));
        let fitting = one(json!({"kind": KIND_INSERT_FITTING, "line_tag": "L-1001", "run_tag": "L-1001-01",
            "fitting_type": "gate_valve", "at": [100, 0], "tag": "V-1001", "run": account(json!([valve]))}));
        assert!(applies(&take(&mut model, &fitting, 1, 2, "s1")));
        // Said again (a step put on again): still one fitting.
        assert!(applies(&take(&mut model, &fitting, 2, 3, "s1")));
        let topology = model.topology();
        assert_eq!(topology["pipe_runs"][0]["fittings"], json!([valve]));
        assert_eq!(topology["pipe_runs"][0]["segments"].as_array().map(Vec::len), Some(2));
        assert_eq!(model.field("V-1001", "type").map(|f| f.value.as_str()), Some("VALVE"));
        let symbol = one(json!({"kind": KIND_INSERT_SYMBOL, "type": "VALVE", "tag": "V-1001", "position": [5, 5]}));
        assert_eq!(model.completed(&symbol).subject(), "V-1002", "the fitting has that tag");

        // The valve erased: its tag gone, and the run as it is now.
        assert!(applies(&take(&mut model, &delete("V-1001"), 3, 4, "s1")));
        assert_eq!(model.topology()["pipe_runs"][0]["fittings"], json!([]), "off the run with its tag");
        assert!(model.object("V-1001").is_none() && model.deleted("V-1001").is_some());
        let after = one(json!({"kind": KIND_UPDATE_PIPE_RUN, "line_tag": "L-1001", "run_tag": "L-1001-01",
            "run": account(json!([]))}));
        assert!(after.is_understood() && after.changes_topology());
        assert_eq!(model.line_of(&after), "L-1001");
        assert!(applies(&take(&mut model, &after, 4, 5, "s1")));
        // And put back: the object is there again.
        let back = one(json!({"kind": KIND_UPDATE_PIPE_RUN, "line_tag": "L-1001", "run_tag": "L-1001-01",
            "run": account(json!([valve]))}));
        assert!(applies(&take(&mut model, &back, 5, 6, "s1")));
        assert!(model.object("V-1001").is_some() && model.deleted("V-1001").is_none());
    }

    /// A change to a run's parts is a change to its line's connections like
    /// the two that create them: made before another session's change to
    /// the same line, it is behind it.
    #[test]
    fn collab_pipe_ops_a_change_to_a_runs_parts_contends_for_the_line() {
        let mut model = DrawingModel::default();
        let run = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "tag": "L-1001-01", "points": []}));
        take(&mut model, &run, 0, 1, "s1");
        let update = one(json!({"kind": KIND_UPDATE_PIPE_RUN, "line_tag": "L-1001", "run_tag": "L-1001-01",
            "run": account(json!([]))}));
        assert_eq!(
            model.judge(&update, 0, "s2"),
            Verdict::BehindTopology { line_tag: "L-1001".into(), touched_revision: 1 }
        );
        assert!(applies(&model.judge(&update, 1, "s2")));
        let stake = model.stake(&update).expect("a stake");
        assert_eq!((stake.0.as_str(), stake.1.as_str()), ("L-1001", FIELD_TOPOLOGY));
    }

    /// A signal line is created like a pipe line and known by its tag, but
    /// the topology does not list it among the pipe lines.
    #[test]
    fn collab_pipe_ops_a_signal_line_is_not_a_pipe_line() {
        let mut model = DrawingModel::default();
        let signal = one(json!({"kind": KIND_CREATE_PIPE_LINE, "tag": "S-2001", "type": TYPE_SIGNAL_LINE,
            "points": [[0, 50], [80, 50]]}));
        assert!(applies(&take(&mut model, &signal, 0, 1, "s1")));
        assert_eq!(model.field("S-2001", "type").map(|f| f.value.as_str()), Some("SIGNAL_LINE"));
        assert_eq!(model.topology()["pipe_lines"], json!([]));
        let pipe = one(json!({"kind": KIND_CREATE_PIPE_LINE, "tag": "L-1001", "type": "PIPE_LINE", "points": []}));
        take(&mut model, &pipe, 1, 2, "s1");
        assert_eq!(model.topology()["pipe_lines"], json!([{"tag": "L-1001", "runs": []}]));
    }

    /// #35: Manual overrides on a pipe run survive subsequent pipe-line
    /// property changes while non-overridden runs follow the line; `tag_source`
    /// is preserved on retag instead of being forced to manual.
    #[test]
    fn pipe_run_overrides_and_tag_source_follow_modifications() {
        let mut model = DrawingModel::default();
        let line = one(json!({"kind": KIND_CREATE_PIPE_LINE, "tag": "L-1001", "type": "PIPE_LINE", "tag_source": "auto"}));
        take(&mut model, &line, 0, 1, "s1");
        let r1 = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "tag": "L-1001-01", "points": []}));
        let r2 = one(json!({"kind": KIND_ADD_PIPE_RUN, "line_tag": "L-1001", "tag": "L-1001-02", "points": []}));
        take(&mut model, &r1, 1, 2, "s1");
        take(&mut model, &r2, 2, 3, "s1");

        // User manually overrides 公称直径 on run L-1001-01 to DN80.
        let manual_override = one(json!({
            "kind": KIND_SET_PROPERTY,
            "object_ref": "L-1001-01",
            "field": "公称直径",
            "value": "DN80",
            "overrides": ["公称直径"]
        }));
        take(&mut model, &manual_override, 3, 4, "s1");

        // Line L-1001 updates 公称直径 to DN100 -> L-1001-01 stays DN80 (overridden), L-1001-02 becomes DN100.
        let line_change = one(json!({
            "kind": KIND_SET_PROPERTY,
            "object_ref": "L-1001",
            "field": "公称直径",
            "value": "DN100"
        }));
        take(&mut model, &line_change, 4, 5, "s1");

        let topo = model.topology();
        let runs = topo["pipe_runs"].as_array().expect("runs");
        let run1 = runs.iter().find(|r| r["tag"] == "L-1001-01").expect("r1");
        let run2 = runs.iter().find(|r| r["tag"] == "L-1001-02").expect("r2");
        assert_eq!(run1["props"]["公称直径"], "DN80");
        assert_eq!(run1["overrides"], json!(["公称直径"]));
        assert_eq!(run2["props"]["公称直径"], "DN100");
        assert_eq!(run2["overrides"], json!([]));

        // Retagging with tag_source="auto" preserves "auto" rather than forcing "manual".
        let auto_retag = one(json!({
            "kind": KIND_SET_TAG,
            "object_ref": "P-1001",
            "value": "P-1002",
            "tag_source": "auto"
        }));
        take(&mut model, &auto_retag, 5, 6, "s2");
        assert_eq!(
            model.object("P-1002").and_then(|o| o.tag_source.as_deref()),
            Some("auto")
        );
    }
}
