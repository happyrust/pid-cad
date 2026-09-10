//! Pipe topology for the block-family P&ID sheets.
//!
//! The TWT sheets draw pipe as two-point polylines on `PIPE-*` layers, each
//! valve block carries two `POINT` entities where pipe joins it, and pipe
//! ends land exactly on those points (0.00 mm on FF02-05, 106 of 112), on a
//! sheet connector's or foam interface's insertion point (blocks without a
//! `POINT`), or on the rim of an S / K circle. This module joins the
//! strokes end to end into *runs*, cutting a run wherever it meets a
//! symbol's connection point, another run (a tee) or nothing (an open end),
//! and gives each run the line number lettered along it -- so every stroke
//! of pipe belongs to a run, every run to a line number when the sheet
//! letters one, and every symbol knows the lines at its connection points.
//!
//! Distances here are drawing units unless a name says `mm`; `upm` converts.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use acadrust::{CadDocument, EntityType, Handle};
use regex::Regex;
use serde::Deserialize;

pub type Point = (f64, f64);

/// Rules for the pipe pass, `pipes` in the rules JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PipeRules {
    /// A stroke on a layer whose name starts with one of these (case
    /// insensitive) is pipe. Empty turns the pass off.
    pub layer_prefixes: Vec<String>,
    /// Ends closer than this (paper mm) meet: pipe to pipe, pipe to a
    /// connection point, pipe to a circle's rim.
    pub snap_mm: f64,
    /// A line number lettered within this (paper mm) of a run is the run's.
    pub number_mm: f64,
    /// Radius (paper mm) of the ring drawn where a run ends in the air when
    /// the runs are drawn into the sheet.
    pub open_end_mm: f64,
    /// What a line number looks like: a piece of lettering is one when a
    /// pattern matches the whole of it (the patterns are anchored; the
    /// first that matches is the one `family` reads). A pattern names the
    /// groups `family` refers to, `(?<service>...)` and `(?<seq>...)`. The
    /// default is the two spellings this corpus uses: `80-FS`, `150-FW`,
    /// `200-FS-31001-A2` with a 5-digit sequence, and the loading islands'
    /// `100-CGA-0319-A1` with 4.
    pub number_pattern: Vec<String>,
    /// The family a line number belongs to, written from the groups its
    /// pattern captured: `{service}-{seq}` makes `200-FS-31001-A2` and
    /// `150-FS-31001-A2` one line, `FS-31001` -- the same line past a reducer
    /// or a spec break. A number whose pattern has no value for a group the
    /// template names (`80-FS` has no `seq`) is its own family.
    pub family: String,
    /// `number_pattern` compiled, anchored, on first use.
    #[serde(skip)]
    compiled: OnceLock<Vec<Regex>>,
}

impl Default for PipeRules {
    fn default() -> Self {
        PipeRules {
            layer_prefixes: Vec::new(),
            snap_mm: 0.3,
            number_mm: 5.0,
            open_end_mm: 0.6,
            number_pattern: DEFAULT_NUMBER_PATTERN
                .iter()
                .map(|p| p.to_string())
                .collect(),
            family: "{service}-{seq}".to_string(),
            compiled: OnceLock::new(),
        }
    }
}

/// The line numbers of the corpus: `<size>-<service>` with an optional
/// `-<5 digits>-<class letter><digit>`, as in `80-FS`, `150-FW`,
/// `200-FS-31001-A2`; and the loading islands' `<size>-<service>-<4
/// digits>-<class>`, as in `100-CGA-0319-A1`.
pub const DEFAULT_NUMBER_PATTERN: [&str; 2] = [
    "(?<size>[0-9]+)-(?<service>[A-Z]{1,3})(?:-(?<seq>[0-9]{5})-(?<class>[A-Z][0-9]))?",
    "(?<size>[0-9]+)-(?<service>[A-Z]{2,3})-(?<seq>[0-9]{4})-(?<class>[A-Z][0-9])",
];

impl PipeRules {
    fn is_pipe_layer(&self, layer: &str) -> bool {
        self.layer_prefixes.iter().any(|p| {
            layer
                .as_bytes()
                .get(..p.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(p.as_bytes()))
        })
    }

    /// Compile `pattern` to match a whole piece of lettering.
    fn compile(pattern: &str) -> Result<Regex, regex::Error> {
        Regex::new(&format!("^(?:{pattern})$"))
    }

    /// Whether every `number_pattern` compiles and `family` names only
    /// groups some pattern has -- checked when the rules are loaded, so a
    /// typo in an override file is reported then and not swallowed sheet by
    /// sheet.
    pub fn validate(&self) -> Result<(), String> {
        let mut groups: BTreeSet<String> = BTreeSet::new();
        for pattern in &self.number_pattern {
            let re = Self::compile(pattern)
                .map_err(|e| format!("pipes.number_pattern {pattern:?}: {e}"))?;
            groups.extend(re.capture_names().flatten().map(str::to_string));
        }
        for name in template_groups(&self.family) {
            if !groups.contains(name) {
                return Err(format!(
                    "pipes.family {:?} names a group {{{name}}} no number_pattern captures",
                    self.family
                ));
            }
        }
        Ok(())
    }

    /// The patterns, compiled once. One that does not compile is dropped
    /// with a warning; `validate` at load time is what normally catches it.
    fn compiled(&self) -> &[Regex] {
        self.compiled.get_or_init(|| {
            self.number_pattern
                .iter()
                .filter_map(|p| match Self::compile(p) {
                    Ok(re) => Some(re),
                    Err(e) => {
                        log::warn!("pipes.number_pattern {p:?} does not compile: {e}");
                        None
                    }
                })
                .collect()
        })
    }

    /// Whether `value` is a line number as the rules letter one.
    pub fn is_line_number(&self, value: &str) -> bool {
        self.compiled().iter().any(|re| re.is_match(value))
    }

    /// The family `value` belongs to by the coding rule: `family` written
    /// from the groups the first matching pattern captured
    /// (`200-FS-31001-A2` -> `FS-31001`). A value no pattern matches, or one
    /// missing a group the template names (`80-FS` has no sequence number),
    /// has nothing to family by and is its own family.
    pub fn line_family(&self, value: &str) -> String {
        self.compiled()
            .iter()
            .find_map(|re| re.captures(value))
            .and_then(|caps| {
                let mut out = String::new();
                let mut rest = self.family.as_str();
                while let Some(open) = rest.find('{') {
                    out.push_str(&rest[..open]);
                    let close = rest[open..].find('}')? + open;
                    out.push_str(caps.name(&rest[open + 1..close])?.as_str());
                    rest = &rest[close + 1..];
                }
                out.push_str(rest);
                Some(out)
            })
            .unwrap_or_else(|| value.to_string())
    }
}

/// The `{group}` names a family template refers to.
fn template_groups(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        names.push(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
    }
    names
}

/// Two arms of a tee that leave it at least this straight (cosine of the
/// angle between their directions) are one line running through; the third
/// arm is the branch.
const STRAIGHT_THROUGH_COS: f64 = -0.866;

/// Where pipe can meet a symbol, drawing units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Port {
    /// A connection point: a `POINT` of the block, placed; or the insertion
    /// point of a block that has none.
    At(Point),
    /// Anywhere on a circle's rim.
    Rim { centre: Point, r: f64 },
}

/// What a run ends at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// A symbol's connection point; the index into `Recognition::symbols`.
    Symbol(usize),
    /// Other runs: three or more strokes meet here.
    Tee,
    /// Nothing: the pipe just stops.
    Open,
    /// The run closes on itself with no symbol or tee on it.
    Loop,
}

/// A stretch of pipe between two ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// Line numbers lettered along this run itself, distinct, in lettering
    /// order.
    pub numbers: Vec<String>,
    /// The line numbers the run carries: its own, or -- when it has none --
    /// those of the lettered runs it continues, through a tee straight on or
    /// through a two-port symbol (an in-line valve). Sorted; more than one
    /// means two differently numbered lines both reach it unlettered.
    pub lines: Vec<String>,
    /// Pipe strokes it is made of (after tees split them).
    pub segments: usize,
    pub length_mm: f64,
    pub ends: [End; 2],
    /// The run's vertices from `ends[0]` to `ends[1]`, drawing units.
    pub path: Vec<Point>,
    /// Handles of the sheet entities whose strokes make the run, distinct,
    /// in handle order. A polyline a tee splits feeds every run it crosses,
    /// so two runs can share a handle.
    pub handles: Vec<Handle>,
}

impl Run {
    /// Bounding box of the run's path, `(min_x, min_y, max_x, max_y)`,
    /// drawing units; `None` for an empty path.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        self.path.iter().fold(None, |acc, p| match acc {
            None => Some((p.0, p.1, p.0, p.1)),
            Some(a) => Some((a.0.min(p.0), a.1.min(p.1), a.2.max(p.0), a.3.max(p.1))),
        })
    }
}

/// The pipe of a sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pipes {
    pub runs: Vec<Run>,
    /// Pipe strokes read from the sheet, before tees split them.
    pub segments: usize,
    /// Connection points the symbols offered, and how many a pipe end met.
    pub ports: usize,
    pub connected_ports: usize,
    pub open_ends: usize,
    /// Line number -> its family by the rules' coding pattern
    /// (`PipeRules::line_family`), for every number the runs carry; built by
    /// `trace` so that the panel and PIDLINE need no rules in hand.
    pub families: BTreeMap<String, String>,
}

impl Pipes {
    /// Line number -> the runs carrying it, in run order.
    pub fn by_line(&self) -> BTreeMap<&str, Vec<&Run>> {
        let mut out: BTreeMap<&str, Vec<&Run>> = BTreeMap::new();
        for run in &self.runs {
            for n in &run.lines {
                out.entry(n.as_str()).or_default().push(run);
            }
        }
        out
    }

    /// The family of `line` as `trace` indexed it; a number it did not see
    /// is its own family.
    pub fn family_of<'a>(&'a self, line: &'a str) -> &'a str {
        self.families.get(line).map_or(line, String::as_str)
    }

    /// (Re)build `families` for every number the runs carry.
    pub fn index_families(&mut self, rules: &PipeRules) {
        self.families = self
            .runs
            .iter()
            .flat_map(|run| run.lines.iter().chain(&run.numbers))
            .map(|line| (line.clone(), rules.line_family(line)))
            .collect();
    }

    /// The lines grouped into families by the coding rule: family key ->
    /// line number -> that line's runs. `200-FS-31001-A2` and
    /// `150-FS-31001-A2` are one family, `FS-31001` -- the same line past a
    /// reducer or a spec break. A run lettered with two families is under
    /// both.
    pub fn by_family(&self) -> BTreeMap<String, BTreeMap<&str, Vec<&Run>>> {
        let mut out: BTreeMap<String, BTreeMap<&str, Vec<&Run>>> = BTreeMap::new();
        for (line, runs) in self.by_line() {
            out.entry(self.family_of(line).to_string())
                .or_default()
                .insert(line, runs);
        }
        out
    }

    /// The symbols at the ends of the runs carrying `line`, distinct.
    pub fn symbols_on(&self, line: &str) -> BTreeSet<usize> {
        self.runs
            .iter()
            .filter(|r| r.lines.iter().any(|n| n == line))
            .flat_map(|r| r.ends)
            .filter_map(|e| match e {
                End::Symbol(i) => Some(i),
                _ => None,
            })
            .collect()
    }
}

fn dist(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// The pipe strokes of the sheet as two-point segments with the handle of
/// the entity each was read from, drawing units.
fn pipe_segments(doc: &CadDocument, rules: &PipeRules) -> Vec<(Handle, Point, Point)> {
    let mut segments = Vec::new();
    for entity in doc.model_space_entities() {
        if !rules.is_pipe_layer(&entity.common().layer) {
            continue;
        }
        let handle = entity.common().handle;
        let mut push = |a: Point, b: Point| {
            if a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite() && a != b {
                segments.push((handle, a, b));
            }
        };
        match entity {
            EntityType::Line(l) => push((l.start.x, l.start.y), (l.end.x, l.end.y)),
            EntityType::LwPolyline(p) => {
                let pts: Vec<Point> = p
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect();
                for w in pts.windows(2) {
                    push(w[0], w[1]);
                }
                if p.is_closed && pts.len() > 2 {
                    push(pts[pts.len() - 1], pts[0]);
                }
            }
            EntityType::Polyline2D(p) => {
                let pts: Vec<Point> = p
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect();
                for w in pts.windows(2) {
                    push(w[0], w[1]);
                }
                if p.flags.is_closed() && pts.len() > 2 {
                    push(pts[pts.len() - 1], pts[0]);
                }
            }
            _ => {}
        }
    }
    segments
}

/// Vertices of the pipe graph: ends within `snap` of each other are one
/// vertex, placed at their mean. Returns each vertex's position and, per
/// segment, its two vertices.
fn snap_vertices(segments: &[(Point, Point)], snap: f64) -> (Vec<Point>, Vec<(usize, usize)>) {
    let ends: Vec<Point> = segments.iter().flat_map(|&(a, b)| [a, b]).collect();
    let mut parent: Vec<usize> = (0..ends.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let cell = |v: f64| (v / snap.max(1e-9)).floor() as i64;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, e) in ends.iter().enumerate() {
        grid.entry((cell(e.0), cell(e.1))).or_default().push(i);
    }
    for (i, e) in ends.iter().enumerate() {
        let (cx, cy) = (cell(e.0), cell(e.1));
        for gx in cx - 1..=cx + 1 {
            for gy in cy - 1..=cy + 1 {
                let Some(items) = grid.get(&(gx, gy)) else {
                    continue;
                };
                for &j in items {
                    if j > i && dist(*e, ends[j]) <= snap {
                        let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                        if ri != rj {
                            parent[ri] = rj;
                        }
                    }
                }
            }
        }
    }
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut sums: Vec<(f64, f64, usize)> = Vec::new();
    let mut of_end = vec![0; ends.len()];
    for (i, e) in ends.iter().enumerate() {
        let root = find(&mut parent, i);
        let v = *index.entry(root).or_insert_with(|| {
            sums.push((0.0, 0.0, 0));
            sums.len() - 1
        });
        sums[v].0 += e.0;
        sums[v].1 += e.1;
        sums[v].2 += 1;
        of_end[i] = v;
    }
    let vertices: Vec<Point> = sums
        .iter()
        .map(|&(x, y, n)| (x / n as f64, y / n as f64))
        .collect();
    let of_segment: Vec<(usize, usize)> = (0..segments.len())
        .map(|s| (of_end[2 * s], of_end[2 * s + 1]))
        .collect();
    (vertices, of_segment)
}

/// Pipe strokes joined into runs, cut at the symbols' connection points and
/// at tees, with the line numbers lettered along them. `ports` are the
/// symbols' connection points by symbol index; `lettering` is the sheet's
/// text with its anchor.
pub fn trace(
    doc: &CadDocument,
    upm: f64,
    rules: &PipeRules,
    ports: &[(usize, Port)],
    lettering: &[(Point, &str)],
) -> Pipes {
    if rules.layer_prefixes.is_empty() {
        return Pipes::default();
    }
    let snap = rules.snap_mm * upm;
    let mut segments = pipe_segments(doc, rules);
    let read = segments.len();
    if segments.is_empty() {
        return Pipes {
            ports: ports.len(),
            ..Pipes::default()
        };
    }

    // Tees: an end lying on another stroke's interior splits that stroke.
    // The ends are found first; splitting adds strokes but no new ends.
    let ends: Vec<Point> = segments.iter().flat_map(|&(_, a, b)| [a, b]).collect();
    let mut s = 0;
    while s < segments.len() {
        let (h, a, b) = segments[s];
        let hit = ends.iter().copied().find(|&e| {
            dist(e, a) > snap && dist(e, b) > snap && point_segment_distance(e, a, b) <= snap
        });
        match hit {
            Some(e) => {
                segments[s] = (h, a, e);
                segments.push((h, e, b));
                // `segments[s]` may be split again by another end.
            }
            None => s += 1,
        }
    }

    let geometry: Vec<(Point, Point)> = segments.iter().map(|&(_, a, b)| (a, b)).collect();
    let (vertices, of_segment) = snap_vertices(&geometry, snap);
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); vertices.len()];
    for (si, &(a, b)) in of_segment.iter().enumerate() {
        if a == b {
            continue;
        }
        incident[a].push(si);
        incident[b].push(si);
    }

    // Symbols' connection points onto vertices.
    let mut port_of: Vec<Option<usize>> = vec![None; vertices.len()];
    let mut connected_ports = 0;
    for &(symbol, port) in ports {
        let mut met = false;
        match port {
            Port::At(p) => {
                let nearest = vertices
                    .iter()
                    .enumerate()
                    .filter(|(v, q)| !incident[*v].is_empty() && dist(p, **q) <= snap)
                    .min_by(|a, b| dist(p, *a.1).total_cmp(&dist(p, *b.1)));
                if let Some((v, _)) = nearest {
                    port_of[v].get_or_insert(symbol);
                    met = true;
                }
            }
            Port::Rim { centre, r } => {
                for (v, q) in vertices.iter().enumerate() {
                    if !incident[v].is_empty() && (dist(centre, *q) - r).abs() <= snap {
                        port_of[v].get_or_insert(symbol);
                        met = true;
                    }
                }
            }
        }
        if met {
            connected_ports += 1;
        }
    }

    // A vertex that ends a run: a symbol's, a tee's, an open end's.
    let end_at = |v: usize| -> Option<End> {
        if let Some(symbol) = port_of[v] {
            return Some(End::Symbol(symbol));
        }
        match incident[v].len() {
            0 | 1 => Some(End::Open),
            2 => None,
            _ => Some(End::Tee),
        }
    };

    // Walk each stroke out to both ends.
    let mut used = vec![false; segments.len()];
    let mut runs: Vec<Run> = Vec::new();
    // The vertex at each end of each run (none for a loop).
    let mut end_vertex: Vec<[Option<usize>; 2]> = Vec::new();
    for start in 0..segments.len() {
        if used[start] || of_segment[start].0 == of_segment[start].1 {
            continue;
        }
        used[start] = true;
        let (mut path_v, mut count) = (vec![of_segment[start].0, of_segment[start].1], 1usize);
        let mut handles = vec![segments[start].0];
        let mut ends = [None, None];
        let mut closed = false;
        // Extend from the tail (side 1), then from the head (side 0).
        for side in [1usize, 0] {
            loop {
                let v = if side == 1 {
                    *path_v.last().unwrap()
                } else {
                    path_v[0]
                };
                if let Some(end) = end_at(v) {
                    ends[side] = Some(end);
                    break;
                }
                let next = incident[v].iter().copied().find(|&s| !used[s]);
                let Some(s) = next else {
                    // Both strokes at this vertex are used: the run came
                    // back to where it began.
                    closed = true;
                    break;
                };
                used[s] = true;
                count += 1;
                handles.push(segments[s].0);
                let (a, b) = of_segment[s];
                let w = if a == v { b } else { a };
                if side == 1 {
                    path_v.push(w);
                } else {
                    path_v.insert(0, w);
                }
            }
            if closed {
                break;
            }
        }
        let ends = if closed {
            end_vertex.push([None, None]);
            [End::Loop, End::Loop]
        } else {
            end_vertex.push([Some(path_v[0]), Some(*path_v.last().unwrap())]);
            [ends[0].unwrap_or(End::Open), ends[1].unwrap_or(End::Open)]
        };
        let path: Vec<Point> = path_v.iter().map(|&v| vertices[v]).collect();
        let length_mm = path.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>() / upm;
        handles.sort_unstable_by_key(|h| h.value());
        handles.dedup();
        runs.push(Run {
            numbers: Vec::new(),
            lines: Vec::new(),
            segments: count,
            length_mm,
            ends,
            path,
            handles,
        });
    }

    // Line numbers onto the nearest run.
    let reach = rules.number_mm * upm;
    for &(at, value) in lettering {
        if !rules.is_line_number(value) {
            continue;
        }
        let nearest = runs
            .iter_mut()
            .map(|run| {
                let d = run
                    .path
                    .windows(2)
                    .map(|w| point_segment_distance(at, w[0], w[1]))
                    .fold(f64::INFINITY, f64::min);
                (d, run)
            })
            .filter(|(d, _)| *d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, run)) = nearest {
            if !run.numbers.iter().any(|n| n == value) {
                run.numbers.push(value.to_string());
            }
        }
    }

    // A run continues into another through a tee it goes straight across,
    // or through a symbol with two connection points (an in-line valve):
    // those neighbours carry the same line. A lettered run keeps its own
    // number; an unlettered one takes the numbers of every lettered run it
    // continues -- the header's on one side of a stub, the branch's on the
    // other, if the sheet letters both.
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); runs.len()];
    // The runs ending at each vertex, with the direction each leaves it in.
    let mut arms: HashMap<usize, Vec<(usize, Point)>> = HashMap::new();
    for (ri, (run, ends)) in runs.iter().zip(&end_vertex).enumerate() {
        let n = run.path.len();
        let leaving = [
            (ends[0], run.path[0], run.path[1]),
            (ends[1], run.path[n - 1], run.path[n - 2]),
        ];
        for (v, from, to) in leaving {
            let Some(v) = v else {
                continue;
            };
            let len = dist(from, to);
            if len > 0.0 {
                let direction = ((to.0 - from.0) / len, (to.1 - from.1) / len);
                arms.entry(v).or_default().push((ri, direction));
            }
        }
    }
    for (v, at_v) in &arms {
        if port_of[*v].is_some() {
            continue;
        }
        for (n, &(a, da)) in at_v.iter().enumerate() {
            for &(b, db) in &at_v[n + 1..] {
                if a != b && da.0 * db.0 + da.1 * db.1 <= STRAIGHT_THROUGH_COS {
                    next[a].push(b);
                    next[b].push(a);
                }
            }
        }
    }
    let mut ports_of_symbol: HashMap<usize, Vec<usize>> = HashMap::new();
    for (v, symbol) in port_of.iter().enumerate() {
        if let Some(symbol) = symbol {
            ports_of_symbol.entry(*symbol).or_default().push(v);
        }
    }
    let two_point: HashSet<usize> = ports
        .iter()
        .filter(|(_, p)| matches!(p, Port::At(_)))
        .fold(HashMap::<usize, usize>::new(), |mut n, (s, _)| {
            *n.entry(*s).or_default() += 1;
            n
        })
        .into_iter()
        .filter(|&(_, n)| n == 2)
        .map(|(s, _)| s)
        .collect();
    for (symbol, vs) in &ports_of_symbol {
        if !two_point.contains(symbol) || vs.len() != 2 {
            continue;
        }
        let at = |v: usize| {
            arms.get(&v)
                .map(|a| a.iter().map(|(r, _)| *r).collect::<Vec<_>>())
        };
        if let (Some(ra), Some(rb)) = (at(vs[0]), at(vs[1])) {
            for &a in &ra {
                for &b in &rb {
                    if a != b {
                        next[a].push(b);
                        next[b].push(a);
                    }
                }
            }
        }
    }
    let mut lines: Vec<BTreeSet<String>> = runs
        .iter()
        .map(|r| r.numbers.iter().cloned().collect())
        .collect();
    for (source, run) in runs.iter().enumerate() {
        if run.numbers.is_empty() {
            continue;
        }
        let mut seen = vec![false; runs.len()];
        seen[source] = true;
        let mut queue = vec![source];
        while let Some(r) = queue.pop() {
            for &n in &next[r] {
                if seen[n] || !runs[n].numbers.is_empty() {
                    continue;
                }
                seen[n] = true;
                lines[n].extend(run.numbers.iter().cloned());
                queue.push(n);
            }
        }
    }
    for (run, lines) in runs.iter_mut().zip(lines) {
        run.lines = lines.into_iter().collect();
    }

    let open_ends = runs
        .iter()
        .flat_map(|r| r.ends)
        .filter(|e| *e == End::Open)
        .count();
    let mut pipes = Pipes {
        runs,
        segments: read,
        ports: ports.len(),
        connected_ports,
        open_ends,
        families: BTreeMap::new(),
    };
    pipes.index_families(rules);
    pipes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_have_a_size_a_service_and_maybe_a_number_and_class() {
        let rules = PipeRules::default();
        for good in [
            "80-FS",
            "150-FW",
            "200-FS-31001-A2",
            "100-FW-32002-B1",
            // The loading islands' spelling: a 4-digit sequence number.
            "100-CGA-0319-A1",
            "200-FS-3100-A2",
        ] {
            assert!(rules.is_line_number(good), "{good}");
        }
        for bad in [
            "BUV-3101",
            "XV-3201",
            "DWG-0100FF02-04",
            "80-",
            "-FS",
            "80-fs",
            "80-FSXX",
            "200-FS-310-A2",
            "200-FS-31001-AA",
            "200-FS-31001",
            "100-CGA-0319",
            "1/2\"NPT",
            "5000m",
        ] {
            assert!(!rules.is_line_number(bad), "{bad}");
        }
    }

    #[test]
    fn a_line_family_is_the_service_and_the_sequence_number() {
        let rules = PipeRules::default();
        assert_eq!(rules.line_family("200-FS-31001-A2"), "FS-31001");
        assert_eq!(rules.line_family("150-FS-31001-B1"), "FS-31001");
        assert_eq!(rules.line_family("100-FW-32002-B1"), "FW-32002");
        assert_eq!(rules.line_family("100-CGA-0319-A1"), "CGA-0319");
        assert_eq!(rules.line_family("80-CGA-0319-B2"), "CGA-0319");
        // No sequence number: nothing to family by, the code stands alone.
        assert_eq!(rules.line_family("80-FS"), "80-FS");
        assert_eq!(rules.line_family("150-FW"), "150-FW");
        // Not a line number at all: its own family too.
        assert_eq!(rules.line_family("BUV-3101"), "BUV-3101");
    }

    #[test]
    fn the_rules_file_can_respell_the_line_numbers_and_their_family() {
        let rules = PipeRules {
            number_pattern: vec!["(?<unit>[0-9]{2})-(?<fluid>[A-Z]+)-(?<no>[0-9]+)".to_string()],
            family: "{fluid}{no}".to_string(),
            ..PipeRules::default()
        };
        assert!(rules.is_line_number("12-CW-7"));
        assert!(!rules.is_line_number("200-FS-31001-A2"));
        assert_eq!(rules.line_family("12-CW-7"), "CW7");
        assert_eq!(rules.line_family("200-FS-31001-A2"), "200-FS-31001-A2");
    }

    #[test]
    fn validate_rejects_a_pattern_that_does_not_compile_or_a_family_group_no_pattern_has() {
        assert_eq!(PipeRules::default().validate(), Ok(()));
        let broken = PipeRules {
            number_pattern: vec!["(?<size>[0-9]+".to_string()],
            ..PipeRules::default()
        };
        let err = broken.validate().unwrap_err();
        assert!(err.contains("pipes.number_pattern"), "{err}");
        let unnamed = PipeRules {
            family: "{service}-{area}".to_string(),
            ..PipeRules::default()
        };
        let err = unnamed.validate().unwrap_err();
        assert!(err.contains("{area}"), "{err}");
        // A pattern that does not compile is dropped at use, not fatal.
        assert!(!broken.is_line_number("200-FS-31001-A2"));
    }

    #[test]
    fn by_family_unites_the_sizes_of_one_line_and_leaves_short_codes_alone() {
        let run = |lines: &[&str]| Run {
            numbers: Vec::new(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
            segments: 1,
            length_mm: 1.0,
            ends: [End::Open, End::Open],
            path: vec![(0.0, 0.0), (1.0, 0.0)],
            handles: Vec::new(),
        };
        let mut pipes = Pipes {
            runs: vec![
                run(&["200-FS-31001-A2"]),
                run(&["150-FS-31001-A2"]),
                run(&["80-FS"]),
                run(&[]),
            ],
            ..Pipes::default()
        };
        pipes.index_families(&PipeRules::default());
        assert_eq!(pipes.family_of("150-FS-31001-A2"), "FS-31001");
        assert_eq!(pipes.family_of("80-FS"), "80-FS");
        // A number the runs do not carry was never indexed: its own family.
        assert_eq!(pipes.family_of("300-FS-31001-A2"), "300-FS-31001-A2");
        let families = pipes.by_family();
        assert_eq!(
            families.keys().collect::<Vec<_>>(),
            ["80-FS", "FS-31001"],
            "one family per short code, one per service-number"
        );
        let fs = &families["FS-31001"];
        assert_eq!(
            fs.keys().copied().collect::<Vec<_>>(),
            ["150-FS-31001-A2", "200-FS-31001-A2"],
            "both sizes of the line sit in its family"
        );
        assert_eq!(families["80-FS"].len(), 1);
    }

    #[test]
    fn a_pipe_layer_is_matched_by_prefix_regardless_of_case() {
        let rules = PipeRules {
            layer_prefixes: vec!["PIPE".to_string()],
            ..PipeRules::default()
        };
        assert!(rules.is_pipe_layer("PIPE-消防"));
        assert!(rules.is_pipe_layer("pipe"));
        assert!(!rules.is_pipe_layer("VALVE_消防"));
        assert!(!rules.is_pipe_layer("PIP"));
    }
}
