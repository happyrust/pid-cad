//! The exploded family: symbols drawn as loose strokes, found as connected
//! components of the geometry no block or circle claimed, cut free of the
//! pipe, signed by shape, and named by the tag beside them or by the
//! dictionary -- then a second pass over the strokes the pipe rule took.

use std::collections::{BTreeMap, HashMap, HashSet};

use acadrust::types::Vector3;
use acadrust::{CadDocument, EntityType, Handle};

use super::blocks::sane;
use super::pairing::match_pairs;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrimKind {
    Line,
    Poly,
    Circle,
    Arc,
    Ellipse,
    Solid,
}

impl PrimKind {
    fn letter(self) -> char {
        match self {
            PrimKind::Circle => 'C',
            PrimKind::Arc => 'A',
            PrimKind::Ellipse => 'E',
            PrimKind::Line | PrimKind::Poly | PrimKind::Solid => 'L',
        }
    }
}

/// One loose stroke, paper millimetres.
#[derive(Clone)]
struct Prim {
    kind: PrimKind,
    /// Polyline points; a circle's centre only.
    pts: Vec<(f64, f64)>,
    /// Radius of a circle / arc / ellipse (major), else 0.
    r: f64,
    bbox: (f64, f64, f64, f64),
    /// The entities drawn as this stroke: the one it came from and, after
    /// [`dedupe`], those drawn over it. Empty for a stroke made in a test.
    handles: Vec<Handle>,
}

impl Prim {
    fn new(kind: PrimKind, pts: Vec<(f64, f64)>, r: f64) -> Option<Prim> {
        let mut bbox = None;
        for &(x, y) in &pts {
            grow(&mut bbox, x, y);
        }
        let mut bbox = bbox?;
        if kind == PrimKind::Circle {
            bbox = (bbox.0 - r, bbox.1 - r, bbox.2 + r, bbox.3 + r);
        }
        Some(Prim {
            kind,
            pts,
            r,
            bbox,
            handles: Vec::new(),
        })
    }

    fn longest_segment(&self) -> f64 {
        self.pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .fold(0.0, f64::max)
    }

    /// This stroke as a single straight run along one axis, if it is one.
    fn axis_run(&self) -> Option<AxisRun> {
        if !matches!(self.kind, PrimKind::Line | PrimKind::Poly) || self.pts.len() != 2 {
            return None;
        }
        AxisRun::of(self.pts[0], self.pts[1])
    }
}

type Point = (f64, f64);
type BBox = (f64, f64, f64, f64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn other(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }
}

/// A straight stroke along one axis: where it sits across that axis (`at`)
/// and how far it runs along it (`lo..hi`), paper mm.
#[derive(Clone, Copy)]
struct AxisRun {
    axis: Axis,
    at: f64,
    lo: f64,
    hi: f64,
}

/// A stroke runs along an axis when it strays from it by no more than this
/// (or a thousandth of its length): a pipe snapped a hair off square is
/// still a pipe.
const AXIS_SLACK_MM: f64 = 0.05;

impl AxisRun {
    fn of(a: Point, b: Point) -> Option<AxisRun> {
        let (dx, dy) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
        let slack = AXIS_SLACK_MM.max(1e-3 * dx.max(dy));
        if dx > dy && dy <= slack {
            Some(AxisRun {
                axis: Axis::Horizontal,
                at: (a.1 + b.1) / 2.0,
                lo: a.0.min(b.0),
                hi: a.0.max(b.0),
            })
        } else if dy > dx && dx <= slack {
            Some(AxisRun {
                axis: Axis::Vertical,
                at: (a.0 + b.0) / 2.0,
                lo: a.1.min(b.1),
                hi: a.1.max(b.1),
            })
        } else {
            None
        }
    }

    fn len(&self) -> f64 {
        self.hi - self.lo
    }

    /// Whether this run sits strictly within a box's width across the axis:
    /// through the body, not along one of its edges.
    fn through(&self, bbox: BBox, eps: f64) -> bool {
        let (_, (lo, hi)) = extents(bbox, self.axis);
        self.at > lo + eps && self.at < hi - eps
    }
}

/// The extent of `bbox` along `axis`, and across it.
fn extents(bbox: BBox, axis: Axis) -> ((f64, f64), (f64, f64)) {
    match axis {
        Axis::Horizontal => ((bbox.0, bbox.2), (bbox.1, bbox.3)),
        Axis::Vertical => ((bbox.1, bbox.3), (bbox.0, bbox.2)),
    }
}

/// An axis-aligned straight stroke of the sheet, whether or not it is short
/// enough to be part of a symbol; `prim` indexes it in the loose strokes
/// when it is, `pipe` in the pipe-length strokes kept for the second pass.
#[derive(Clone, Copy)]
struct SheetRun {
    run: AxisRun,
    prim: Option<usize>,
    pipe: Option<usize>,
}

/// What [`loose_prims`] takes from a sheet.
struct Loose {
    /// Strokes short enough to be part of a symbol.
    prims: Vec<Prim>,
    /// Straight axis-aligned strokes past the pipe stub length but not past
    /// the longest a symbol stroke can be: pipe, unless the second pass finds
    /// them framed into a symbol.
    pipe_prims: Vec<Prim>,
    /// Every straight axis-aligned stroke, pipe runs included.
    runs: Vec<SheetRun>,
    /// Circles already recognised as symbols, `(centre, radius)`.
    circles: Vec<(Point, f64)>,
}

fn arc_points(c: (f64, f64), r: f64, start: f64, end: f64, n: usize) -> Vec<(f64, f64)> {
    let tau = std::f64::consts::TAU;
    let mut span = (end - start).rem_euclid(tau);
    if span == 0.0 {
        span = tau;
    }
    (0..=n)
        .map(|k| {
            let a = start + span * k as f64 / n as f64;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// The loose strokes of a sheet that can be part of an exploded symbol, with
/// the pipe runs and recognised circles that say where the pipe goes.
fn loose_prims(
    doc: &CadDocument,
    upm: f64,
    rules: &ShapeRules,
    used_circles: &HashSet<Handle>,
) -> Loose {
    let mm = |v: f64| v / upm;
    // Strokes short enough for a symbol, with their axis run if straight;
    // pipe runs go straight to `out.runs`.
    let mut kept: Vec<(Prim, Option<AxisRun>)> = Vec::new();
    let mut pipes: Vec<(Prim, AxisRun)> = Vec::new();
    let mut out = Loose {
        prims: Vec::new(),
        pipe_prims: Vec::new(),
        runs: Vec::new(),
        circles: Vec::new(),
    };
    for entity in doc.model_space_entities() {
        let layer = entity.common().layer.as_str();
        if is_legend_layer(layer) || rules.skip_layers.iter().any(|l| l == layer) {
            continue;
        }
        let prim = match entity {
            EntityType::Line(l) => Prim::new(
                PrimKind::Line,
                vec![(mm(l.start.x), mm(l.start.y)), (mm(l.end.x), mm(l.end.y))],
                0.0,
            ),
            EntityType::LwPolyline(p) => {
                let mut pts: Vec<(f64, f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (mm(v.location.x), mm(v.location.y)))
                    .collect();
                if p.is_closed && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                Prim::new(PrimKind::Poly, pts, 0.0)
            }
            EntityType::Polyline2D(p) => {
                let mut pts: Vec<(f64, f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (mm(v.location.x), mm(v.location.y)))
                    .collect();
                if p.flags.is_closed() && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                Prim::new(PrimKind::Poly, pts, 0.0)
            }
            EntityType::Circle(c) => {
                let r = mm(c.radius);
                let centre = (mm(c.center.x), mm(c.center.y));
                if used_circles.contains(&c.common.handle) {
                    if sane(centre.0) && sane(centre.1) && sane(r) {
                        out.circles.push((centre, r));
                    }
                    continue;
                }
                if r > rules.max_circle_mm {
                    continue;
                }
                Prim::new(PrimKind::Circle, vec![centre], r)
            }
            EntityType::Arc(a) => {
                let r = mm(a.radius);
                Prim::new(
                    PrimKind::Arc,
                    arc_points(
                        (mm(a.center.x), mm(a.center.y)),
                        r,
                        a.start_angle,
                        a.end_angle,
                        6,
                    ),
                    r,
                )
            }
            EntityType::Ellipse(e) => {
                let (cx, cy) = (mm(e.center.x), mm(e.center.y));
                let (ax, ay) = (mm(e.major_axis.x), mm(e.major_axis.y));
                let ratio = e.minor_axis_ratio;
                let pts = (0..=12)
                    .map(|k| {
                        let t = std::f64::consts::TAU * k as f64 / 12.0;
                        (
                            cx + ax * t.cos() - ay * ratio * t.sin(),
                            cy + ay * t.cos() + ax * ratio * t.sin(),
                        )
                    })
                    .collect();
                Prim::new(PrimKind::Ellipse, pts, ax.hypot(ay))
            }
            EntityType::Solid(s) => {
                let p = |v: Vector3| (mm(v.x), mm(v.y));
                let corners = [
                    p(s.first_corner),
                    p(s.second_corner),
                    p(s.fourth_corner),
                    p(s.third_corner),
                    p(s.first_corner),
                ];
                Prim::new(PrimKind::Solid, corners.to_vec(), 0.0)
            }
            _ => continue,
        };
        let Some(mut prim) = prim else {
            continue;
        };
        if !(sane(prim.bbox.0) && sane(prim.bbox.1) && sane(prim.bbox.2) && sane(prim.bbox.3)) {
            continue;
        }
        prim.handles.push(entity.common().handle);
        let run = prim.axis_run();
        // Too long for a symbol stroke, or a straight axis run past the pipe
        // stub length: pipe (or an instrument leader), not symbol. A run of
        // pipe stub length that a symbol stroke could still be is kept aside
        // for the second pass.
        let too_long =
            prim.kind != PrimKind::Circle && prim.longest_segment() > rules.max_stroke_mm;
        let stub = rules.pipe_stub_mm > 0.0 && run.is_some_and(|r| r.len() > rules.pipe_stub_mm);
        match run {
            Some(run) if too_long => out.runs.push(SheetRun {
                run,
                prim: None,
                pipe: None,
            }),
            Some(run) if stub => pipes.push((prim, run)),
            _ if too_long => {}
            _ => kept.push((prim, run)),
        }
    }
    let (prims, runs): (Vec<Prim>, Vec<Option<AxisRun>>) = kept.into_iter().unzip();
    let (prims, index) = dedupe(prims, rules.touch_mm);
    for (old, run) in runs.into_iter().enumerate() {
        if let (Some(run), Some(prim)) = (run, index[old]) {
            out.runs.push(SheetRun {
                run,
                prim: Some(prim),
                pipe: None,
            });
        }
    }
    out.prims = prims;
    let (pipe_prims, runs): (Vec<Prim>, Vec<AxisRun>) = pipes.into_iter().unzip();
    let (pipe_prims, index) = dedupe(pipe_prims, rules.touch_mm);
    for (old, run) in runs.into_iter().enumerate() {
        if let Some(pipe) = index[old] {
            out.runs.push(SheetRun {
                run,
                prim: None,
                pipe: Some(pipe),
            });
        }
    }
    out.pipe_prims = pipe_prims;
    out
}

/// Whether two strokes are the same drawing: same kind of mark, same points
/// (either way round, within `eps`), same radius.
fn same_stroke(a: &Prim, b: &Prim, eps: f64) -> bool {
    if a.kind.letter() != b.kind.letter() || a.pts.len() != b.pts.len() || (a.r - b.r).abs() > eps {
        return false;
    }
    let close = |p: Point, q: Point| (p.0 - q.0).abs() <= eps && (p.1 - q.1).abs() <= eps;
    a.pts.iter().zip(&b.pts).all(|(&p, &q)| close(p, q))
        || a.pts
            .iter()
            .zip(b.pts.iter().rev())
            .all(|(&p, &q)| close(p, q))
}

/// Drop strokes drawn twice over: a line on top of an identical polyline, a
/// diagonal repeated. A symbol with one of its lines doubled then has the id
/// of the symbol drawn once, and a doubled piece of pipe between two valves
/// is one piece, so taking it out does part them. The stroke kept takes the
/// dropped ones' entities, so the symbol still owns everything drawn for it.
/// Returns the strokes kept and, per input stroke, its new index (`None` =
/// dropped).
fn dedupe(mut prims: Vec<Prim>, eps: f64) -> (Vec<Prim>, Vec<Option<usize>>) {
    let mut order: Vec<usize> = (0..prims.len()).collect();
    order.sort_by(|&a, &b| prims[a].bbox.0.total_cmp(&prims[b].bbox.0));
    let mut dropped = vec![false; prims.len()];
    for (n, &i) in order.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        for &j in &order[n + 1..] {
            if prims[j].bbox.0 - prims[i].bbox.0 > eps {
                break;
            }
            if !dropped[j] && same_stroke(&prims[i], &prims[j], eps) {
                dropped[j] = true;
                let taken = std::mem::take(&mut prims[j].handles);
                prims[i].handles.extend(taken);
            }
        }
    }
    let mut index = vec![None; prims.len()];
    let mut out = Vec::with_capacity(prims.len());
    for (i, prim) in prims.into_iter().enumerate() {
        if !dropped[i] {
            index[i] = Some(out.len());
            out.push(prim);
        }
    }
    (out, index)
}

fn point_segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Whether segments `a`–`b` and `c`–`d` cross (proper intersection).
fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let orient = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (o1, o2) = (orient(a, b, c), orient(a, b, d));
    let (o3, o4) = (orient(c, d, a), orient(c, d, b));
    o1 * o2 < 0.0 && o3 * o4 < 0.0
}

/// Whether an end of one stroke lies on the other (either way round), two
/// strokes cross (the X of a valve drawn as two lines), or two circles meet.
fn touches(a: &Prim, b: &Prim, eps: f64) -> bool {
    for (p, q) in [(a, b), (b, a)] {
        for &e in &p.pts {
            if q.kind == PrimKind::Circle {
                let d = (e.0 - q.pts[0].0).hypot(e.1 - q.pts[0].1);
                if (d - q.r).abs() <= eps || d <= eps {
                    return true;
                }
            } else if q
                .pts
                .windows(2)
                .any(|w| point_segment_distance(e, w[0], w[1]) <= eps)
            {
                return true;
            }
        }
    }
    if a.kind == PrimKind::Circle && b.kind == PrimKind::Circle {
        let d = (a.pts[0].0 - b.pts[0].0).hypot(a.pts[0].1 - b.pts[0].1);
        if d <= eps || (d - (a.r + b.r)).abs() <= eps || (d - (a.r - b.r).abs()).abs() <= eps {
            return true;
        }
    }
    if a.kind != PrimKind::Circle && b.kind != PrimKind::Circle {
        for s in a.pts.windows(2) {
            for t in b.pts.windows(2) {
                if segments_cross(s[0], s[1], t[0], t[1]) {
                    return true;
                }
            }
        }
    }
    false
}

/// Groups of strokes that touch, transitively.
fn components(prims: &[Prim], eps: f64) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..prims.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in prims.iter().enumerate() {
        let (x0, y0, x1, y1) = p.bbox;
        let cell = |v: f64| (v / COMPONENT_CELL_MM).floor() as i64;
        for gx in cell(x0)..=cell(x1) {
            for gy in cell(y0)..=cell(y1) {
                grid.entry((gx, gy)).or_default().push(i);
            }
        }
    }
    let mut checked: HashSet<(usize, usize)> = HashSet::new();
    for items in grid.values() {
        for (n, &i) in items.iter().enumerate() {
            for &j in &items[n + 1..] {
                let key = (i.min(j), i.max(j));
                if !checked.insert(key) {
                    continue;
                }
                let (bi, bj) = (prims[i].bbox, prims[j].bbox);
                if bi.0 > bj.2 + eps || bj.0 > bi.2 + eps || bi.1 > bj.3 + eps || bj.1 > bi.3 + eps
                {
                    continue;
                }
                if touches(&prims[i], &prims[j], eps) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..prims.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    // Which stroke a component's root lands on depends on the order the
    // pairs came in, and that is a HashMap's -- a different one every run,
    // which put the symbols in a different order every run (the panel's rows
    // jumped, `End::Symbol(i)` moved). By first stroke the order is the
    // sheet's own.
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    groups.sort_by_key(|group| group[0]);
    groups
}

/// Box of a set of strokes.
fn group_bbox(prims: &[Prim], idxs: &[usize]) -> Option<BBox> {
    let mut bbox = None;
    for &i in idxs {
        let b = prims[i].bbox;
        grow(&mut bbox, b.0, b.1);
        grow(&mut bbox, b.2, b.3);
    }
    bbox
}

/// Connected parts of `idxs` when `without` is taken out, by the touch
/// relation among the remaining strokes.
fn parts_without(prims: &[Prim], idxs: &[usize], without: usize, eps: f64) -> Vec<Vec<usize>> {
    let rest: Vec<usize> = idxs.iter().copied().filter(|&i| i != without).collect();
    let mut seen = vec![false; rest.len()];
    let mut parts = Vec::new();
    for start in 0..rest.len() {
        if seen[start] {
            continue;
        }
        let mut part = vec![rest[start]];
        seen[start] = true;
        let mut queue = vec![start];
        while let Some(a) = queue.pop() {
            for b in 0..rest.len() {
                if !seen[b] && touches(&prims[rest[a]], &prims[rest[b]], eps) {
                    seen[b] = true;
                    part.push(rest[b]);
                    queue.push(b);
                }
            }
        }
        parts.push(part);
    }
    parts
}

/// A part cut off a component must be at least this wide and this tall to
/// count as a symbol rather than a bit of pipe or a tick.
const MIN_PART_SIDE_MM: f64 = 0.5;

/// A run reaching within this of a box's edge enters it.
const REACH_MM: f64 = 0.2;

/// A stub whose free end lies within this of a recognised circle's rim is a
/// stem to that circle (an actuator mark, a drain point), not pipe.
const STEM_END_MM: f64 = 0.1;

/// Whether point `p` lies on stroke `q` (within `eps`).
fn point_on_prim(p: Point, q: &Prim, eps: f64) -> bool {
    if q.kind == PrimKind::Circle {
        let d = (p.0 - q.pts[0].0).hypot(p.1 - q.pts[0].1);
        return (d - q.r).abs() <= eps;
    }
    q.pts
        .windows(2)
        .any(|w| point_segment_distance(p, w[0], w[1]) <= eps)
}

/// Whether a straight run along `axis` that is not one of the part's own
/// strokes (`own`) enters the part from outside: it lies within the part's
/// width across the axis and reaches either of its edges along the axis, or
/// passes straight through. That is a pipe (or an instrument leader) drawn
/// into the symbol. `runs` are the runs that can do so -- see
/// [`runs_for_group`].
fn threaded(
    runs: &[SheetRun],
    own: impl Fn(usize) -> bool,
    bbox: BBox,
    axis: Axis,
    eps: f64,
) -> bool {
    let ((lo, hi), _) = extents(bbox, axis);
    runs.iter().any(|r| {
        r.run.axis == axis
            && r.prim.is_none_or(|i| !own(i))
            && r.run.through(bbox, eps)
            && (((r.run.hi - lo).abs() <= REACH_MM && r.run.lo < lo)
                || ((r.run.lo - hi).abs() <= REACH_MM && r.run.hi > hi)
                || (r.run.lo <= lo + REACH_MM && r.run.hi >= hi - REACH_MM))
    })
}

/// The runs that can enter a component's parts from outside and so keep a
/// bridge from being cut: the pipe runs (no stroke of the pass) and the
/// component's own strokes -- the stubs trimmed off it are pipe too. Another
/// symbol's stroke is neither: the stem arrow drawn 0.07 mm off a gate valve
/// has a 0.2 mm base that would otherwise "enter" the valve and hold the
/// three valves of SP02-05's relief branch together. Empty when the
/// component is too small for [`split_at_bridges`] to cut at all.
fn runs_for_group(runs: &[SheetRun], group: &[usize], min_strokes: usize) -> Vec<SheetRun> {
    if min_strokes == 0 || group.len() < 2 * min_strokes + 1 {
        return Vec::new();
    }
    runs.iter()
        .filter(|r| r.prim.is_none_or(|i| group.contains(&i)))
        .copied()
        .collect()
}

/// Cut a component where a piece of pipe joins two symbols: a straight
/// axis-aligned stroke whose removal leaves at least two parts of
/// `min_strokes` strokes and some width and height, running through the
/// body of each part rather than along an edge -- unless the pipe enters one
/// of those parts the other way, in which case the stroke is a stem or a
/// branch and belongs to the symbol. The pipe piece is dropped and each part
/// is cut again the same way. `runs` as [`runs_for_group`] gives them.
fn split_at_bridges(
    prims: &[Prim],
    runs: &[SheetRun],
    idxs: Vec<usize>,
    min_strokes: usize,
    eps: f64,
) -> Vec<Vec<usize>> {
    if min_strokes == 0 || idxs.len() < 2 * min_strokes + 1 {
        return vec![idxs];
    }
    for &candidate in &idxs {
        let Some(bridge) = prims[candidate].axis_run() else {
            continue;
        };
        let parts = parts_without(prims, &idxs, candidate, eps);
        let substantial: Vec<(&Vec<usize>, BBox)> = parts
            .iter()
            .filter_map(|part| {
                let bbox = group_bbox(prims, part)?;
                let side = (bbox.2 - bbox.0).min(bbox.3 - bbox.1);
                (part.len() >= min_strokes && side >= MIN_PART_SIDE_MM).then_some((part, bbox))
            })
            .collect();
        if substantial.len() < 2 || !substantial.iter().all(|(_, b)| bridge.through(*b, eps)) {
            continue;
        }
        let crossed = substantial.iter().any(|(part, bbox)| {
            threaded(runs, |i| part.contains(&i), *bbox, bridge.axis.other(), eps)
        });
        if crossed {
            continue;
        }
        return parts
            .into_iter()
            .flat_map(|part| split_at_bridges(prims, runs, part, min_strokes, eps))
            .collect();
    }
    vec![idxs]
}

/// Drop the pipe left touching a symbol: a straight axis-aligned stroke
/// attached to the rest at one end only, whose free end sticks out past the
/// rest along its own axis while it lies within the rest's width -- the
/// pipe entering the symbol, or turning a corner at its edge -- and whose
/// free end touches no recognised circle (a stem to an actuator mark or a
/// drain point looks the same and is part of the symbol). A tick at the
/// symbol's edge ends where the body ends and so does not stick out. Done
/// until nothing changes, so a stub drawn in two pieces goes too. A symbol
/// so has one id whatever length of pipe was drawn against it.
fn trim_pipe_stubs(
    prims: &[Prim],
    mut idxs: Vec<usize>,
    circles: &[(Point, f64)],
    eps: f64,
) -> Vec<usize> {
    loop {
        if idxs.len() < 2 {
            return idxs;
        }
        let stub = idxs.iter().position(|&i| {
            let p = &prims[i];
            let Some(run) = p.axis_run() else {
                return false;
            };
            let attached = |e: Point| {
                idxs.iter()
                    .any(|&j| j != i && point_on_prim(e, &prims[j], eps))
            };
            let free = match (attached(p.pts[0]), attached(p.pts[1])) {
                (true, false) => p.pts[1],
                (false, true) => p.pts[0],
                _ => return false,
            };
            let rest: Vec<usize> = idxs.iter().copied().filter(|&j| j != i).collect();
            let Some(core) = group_bbox(prims, &rest) else {
                return false;
            };
            let ((lo, hi), (across_lo, across_hi)) = extents(core, run.axis);
            let along = match run.axis {
                Axis::Horizontal => free.0,
                Axis::Vertical => free.1,
            };
            let sticks_out = along < lo - eps || along > hi + eps;
            let within = run.at >= across_lo - eps && run.at <= across_hi + eps;
            let to_a_circle = circles
                .iter()
                .any(|&(c, r)| ((free.0 - c.0).hypot(free.1 - c.1) - r).abs() <= STEM_END_MM);
            sticks_out && within && !to_a_circle
        });
        match stub {
            Some(at) => {
                idxs.remove(at);
            }
            None => return idxs,
        }
    }
}

/// The eight symmetries of the square.
const SYMMETRIES: [fn(Point) -> Point; 8] = [
    |(x, y)| (x, y),
    |(x, y)| (-y, x),
    |(x, y)| (-x, -y),
    |(x, y)| (y, -x),
    |(x, y)| (-x, y),
    |(x, y)| (y, x),
    |(x, y)| (x, -y),
    |(x, y)| (-y, -x),
];

/// A string that is the same for every placement of a shape at any rotation
/// by 90° or mirror: strokes about the box centre, rounded to `q`, under the
/// symmetry that sorts lowest.
fn signature(prims: &[Prim], idxs: &[usize], centre: (f64, f64), q: f64) -> String {
    // (stroke kind, points about the centre, radius in quanta)
    let tokens: Vec<(char, Vec<Point>, i64)> = idxs
        .iter()
        .map(|&i| {
            let p = &prims[i];
            let rel = |pt: Point| (pt.0 - centre.0, pt.1 - centre.1);
            let pts = match p.kind {
                PrimKind::Circle => vec![rel(p.pts[0])],
                PrimKind::Arc | PrimKind::Ellipse => vec![
                    rel(p.pts[0]),
                    rel(p.pts[p.pts.len() - 1]),
                    rel(p.pts[p.pts.len() / 2]),
                ],
                _ => p.pts.iter().map(|&pt| rel(pt)).collect(),
            };
            (p.kind.letter(), pts, (p.r / q).round() as i64)
        })
        .collect();
    let mut best: Option<String> = None;
    for sym in SYMMETRIES {
        let mut rows: Vec<String> = tokens
            .iter()
            .map(|(kind, pts, r)| {
                let mut q_pts: Vec<(i64, i64)> = pts
                    .iter()
                    .map(|&pt| {
                        let (x, y) = sym(pt);
                        ((x / q).round() as i64, (y / q).round() as i64)
                    })
                    .collect();
                // A stroke reads the same from either end.
                let reversed: Vec<(i64, i64)> = q_pts.iter().rev().copied().collect();
                if reversed < q_pts {
                    q_pts = reversed;
                }
                format!("{kind}{r}:{q_pts:?}")
            })
            .collect();
        rows.sort();
        let s = rows.join("|");
        if best.as_ref().is_none_or(|b| s < *b) {
            best = Some(s);
        }
    }
    best.unwrap_or_default()
}

pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A saturated, mid-light colour that differs between shape ids.
pub fn hash_color(hash: u64) -> [u8; 3] {
    let hue = (hash % 360) as f64;
    let (s, l): (f64, f64) = (0.85, 0.55);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b)]
}

struct Component {
    /// Box in paper mm.
    bbox: (f64, f64, f64, f64),
    centre: (f64, f64),
    id: String,
    hash: u64,
    /// The strokes, as indices into the pass's stroke list.
    idxs: Vec<usize>,
}

impl Component {
    /// A component of `idxs` with its box, centre and shape id, unless its
    /// box is too small for a symbol or too big for one.
    fn of(prims: &[Prim], idxs: Vec<usize>, rules: &ShapeRules) -> Option<Component> {
        let bbox = group_bbox(prims, &idxs)?;
        let longest = (bbox.2 - bbox.0).max(bbox.3 - bbox.1);
        if longest < rules.min_box_mm || longest > rules.max_box_mm {
            return None;
        }
        let centre = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
        let sig = signature(prims, &idxs, centre, rules.quantum_mm);
        let hash = fnv1a(sig.as_bytes());
        Some(Component {
            bbox,
            centre,
            id: format!("{:08x}", (hash >> 32) as u32 ^ hash as u32),
            hash,
            idxs,
        })
    }

    fn strokes(&self) -> usize {
        self.idxs.len()
    }

    /// The entities drawn as this component's strokes.
    fn handles(&self, prims: &[Prim]) -> Vec<Handle> {
        self.idxs
            .iter()
            .flat_map(|&i| prims[i].handles.iter().copied())
            .collect()
    }

    fn size(&self) -> (f64, f64) {
        (self.bbox.2 - self.bbox.0, self.bbox.3 - self.bbox.1)
    }
}

pub(super) struct Exploded {
    pub(super) symbols: Vec<(Recognized, TagRule)>,
    pub(super) unknown: Vec<UnknownShape>,
}

/// Components of loose geometry, named by the tag beside them or by the
/// shape dictionary, or boxed as repeated unknowns.
pub(super) fn exploded_symbols(
    doc: &CadDocument,
    upm: f64,
    rules: &Rules,
    shape_rules: &ShapeRules,
    used_circles: &HashSet<Handle>,
    lettering: &[Lettering],
    taken_text: &mut [bool],
) -> Exploded {
    let loose = loose_prims(doc, upm, shape_rules, used_circles);
    let prims = loose.prims.as_slice();
    let mut comps: Vec<Component> = Vec::new();
    let eps = shape_rules.touch_mm;
    let min_strokes = shape_rules.split_min_strokes;
    // Take the pipe out: stubs off each component, then a cut wherever a
    // piece of pipe joins two symbols, then the stubs that cut left behind.
    let trim = |idxs: Vec<usize>| {
        if min_strokes > 0 {
            trim_pipe_stubs(prims, idxs, &loose.circles, eps)
        } else {
            idxs
        }
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for group in components(prims, eps) {
        let runs = runs_for_group(&loose.runs, &group, min_strokes);
        for part in split_at_bridges(prims, &runs, trim(group), min_strokes, eps) {
            groups.push(trim(part));
        }
    }
    for idxs in groups {
        if idxs.is_empty() || (idxs.len() < 2 && prims[idxs[0]].kind != PrimKind::Circle) {
            continue;
        }
        if let Some(comp) = Component::of(prims, idxs, shape_rules) {
            comps.push(comp);
        }
    }

    // ── a tag lettered beside a component names it
    let mut claims: Vec<(f64, usize, usize, usize)> = Vec::new(); // (mm, text, comp, rule)
    for (ri, rule) in rules.tag_classes.iter().enumerate() {
        let reach = rule.radius_mm.unwrap_or(rules.radius_mm);
        if reach <= 0.0 {
            continue;
        }
        for (k, l) in lettering.iter().enumerate() {
            if taken_text[k] || !shape_matches(&rule.shape, &l.value) {
                continue;
            }
            let at = (l.at.0 / upm, l.at.1 / upm);
            for (ci, c) in comps.iter().enumerate() {
                let (w, h) = c.size();
                if w < rule.min_side_mm || h < rule.min_side_mm {
                    continue;
                }
                let d = (c.centre.0 - at.0).hypot(c.centre.1 - at.1);
                if d <= reach {
                    claims.push((d, k, ci, ri));
                }
            }
        }
    }
    claims.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut text_used = vec![false; lettering.len()];
    let mut comp_used = vec![false; comps.len()];
    // Nearest free pair first, to learn which classes have a symbol on this
    // sheet at all. One none of whose tags finds one (the flame arresters,
    // drawn in pipe-length strokes and filtered out) sits out the pairing
    // below: with no symbol of its own to be had, it could only take another
    // class's.
    let mut found: HashSet<&str> = HashSet::new();
    for &(_, k, ci, ri) in &claims {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        found.insert(rules.tag_classes[ri].class.as_str());
    }
    // Then pair for real, keeping as many tags matched as the candidates
    // allow and, of such pairings, the shortest. A row of valves lettered at
    // one offset (SP02-10's GV0326A..GV0327D: 7 mm apart, each tag 6 mm left
    // of its own valve and 3 mm right of the neighbour's) defeats
    // nearest-first: the whole row shifts by one, the end tag is orphaned and
    // the end valve goes unnamed, though pairing all seven is possible. A
    // shape the dictionary names as some other class is left out here, so
    // that being matched at all never pulls a tag onto it.
    let eligible: Vec<usize> = (0..claims.len())
        .filter(|&e| {
            let (_, _, ci, ri) = claims[e];
            let rule = &rules.tag_classes[ri];
            found.contains(rule.class.as_str())
                && !shape_rules
                    .dictionary
                    .get(&comps[ci].id)
                    .is_some_and(|named| named.class != rule.class)
        })
        .collect();
    let pairs: Vec<(f64, usize, usize)> = eligible
        .iter()
        .map(|&e| (claims[e].0, claims[e].1, claims[e].2))
        .collect();
    let mut chosen: Vec<usize> = match_pairs(&pairs)
        .into_iter()
        .map(|p| eligible[p])
        .collect();
    // What that could not place takes the nearest free component as before:
    // a tag whose only candidate the dictionary names otherwise (SP02-05's
    // PSV0407A, lettered beside a valve drawn like the bleed valves) still
    // names it -- the tag is the stronger evidence.
    text_used.fill(false);
    comp_used.fill(false);
    for &e in &chosen {
        text_used[claims[e].1] = true;
        comp_used[claims[e].2] = true;
    }
    for (e, &(_, k, ci, _)) in claims.iter().enumerate() {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        chosen.push(e);
    }
    // comp -> (rule, text, mm) of its claim, then any further tags.
    let mut claimed: HashMap<usize, (usize, usize, f64, Vec<usize>)> = HashMap::new();
    for e in chosen {
        let (d, k, ci, ri) = claims[e];
        taken_text[k] = true;
        claimed.insert(ci, (ri, k, d, Vec::new()));
    }
    // Tags still free may join an assembly: a long component already claimed
    // (valves on one branch drawn touching), which then lists them all. The
    // tag has to sit beside the assembly's box, not merely within reach of
    // its centre, or the next branch's tags would drift in.
    if shape_rules.assembly_mm > 0.0 {
        let mut joins: Vec<(f64, usize, usize)> = Vec::new();
        for &(_, k, ci, _) in &claims {
            if taken_text[k] || !claimed.contains_key(&ci) {
                continue;
            }
            let c = &comps[ci];
            let longest = (c.bbox.2 - c.bbox.0).max(c.bbox.3 - c.bbox.1);
            if longest < shape_rules.assembly_mm {
                continue;
            }
            let at = (lettering[k].at.0 / upm, lettering[k].at.1 / upm);
            let dx = (c.bbox.0 - at.0).max(0.0).max(at.0 - c.bbox.2);
            let dy = (c.bbox.1 - at.1).max(0.0).max(at.1 - c.bbox.3);
            let gap = dx.hypot(dy);
            if gap <= shape_rules.assembly_mm {
                joins.push((gap, k, ci));
            }
        }
        joins.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, k, ci) in joins {
            if taken_text[k] {
                continue;
            }
            if let Some(entry) = claimed.get_mut(&ci) {
                taken_text[k] = true;
                entry.3.push(k);
            }
        }
    }

    // ── the rest: dictionary, or repeated unknowns
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (ci, c) in comps.iter().enumerate() {
        let (w, h) = c.size();
        if !claimed.contains_key(&ci)
            && !shape_rules.dictionary.contains_key(&c.id)
            && w.min(h) >= shape_rules.min_side_mm
        {
            *counts.entry(c.id.as_str()).or_default() += 1;
        }
    }
    let mut summaries: BTreeMap<&str, UnknownShape> = BTreeMap::new();
    let mut nearby: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    let mut symbols = Vec::new();
    let scale = |b: (f64, f64, f64, f64)| (b.0 * upm, b.1 * upm, b.2 * upm, b.3 * upm);
    // Strokes of the components that became symbols (named, ignored or
    // boxed): the second pass leaves them alone.
    let mut in_symbol = vec![false; prims.len()];
    for (ci, c) in comps.iter().enumerate() {
        let at = (c.centre.0 * upm, c.centre.1 * upm);
        let source = format!("shape {} ({} strokes)", c.id, c.strokes());
        let repeated = counts.get(c.id.as_str()).copied().unwrap_or(0) >= shape_rules.min_count;
        if claimed.contains_key(&ci) || shape_rules.dictionary.contains_key(&c.id) || repeated {
            for &i in &c.idxs {
                in_symbol[i] = true;
            }
        }
        if let Some((ri, k, d, more)) = claimed.get(&ci) {
            let rule = &rules.tag_classes[*ri];
            let mut tag = lettering[*k].value.clone();
            let mut tag_handles = vec![lettering[*k].handle];
            for &m in more {
                tag.push_str(" + ");
                tag.push_str(&lettering[m].value);
                tag_handles.push(lettering[m].handle);
            }
            symbols.push((
                Recognized {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: true,
                    inner_text: Vec::new(),
                    tag: Some(tag),
                    tag_distance_mm: Some(*d),
                    wants_tag: true,
                    report_untagged: rule.report_orphans,
                    lines: Vec::new(),
                    handles: c.handles(prims),
                    tag_handles,
                },
                TagRule::default(),
            ));
        } else if let Some(rule) = shape_rules.dictionary.get(&c.id) {
            if rule.class == IGNORE_CLASS {
                continue;
            }
            symbols.push((
                Recognized {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: true,
                    inner_text: Vec::new(),
                    tag: None,
                    tag_distance_mm: None,
                    wants_tag: rule.tag.wants_tag(),
                    report_untagged: rule.tag.reports_untagged(),
                    lines: Vec::new(),
                    handles: c.handles(prims),
                    tag_handles: Vec::new(),
                },
                rule.tag.clone(),
            ));
        } else if repeated {
            let (w, h) = c.size();
            symbols.push((
                Recognized {
                    class: format!("{SHAPE_CLASS_PREFIX}{}", c.id),
                    label: format!("图形 {}", c.id),
                    color: hash_color(c.hash),
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: false,
                    inner_text: Vec::new(),
                    tag: None,
                    tag_distance_mm: None,
                    wants_tag: false,
                    report_untagged: false,
                    lines: Vec::new(),
                    handles: c.handles(prims),
                    tag_handles: Vec::new(),
                },
                TagRule::default(),
            ));
            let summary = summaries
                .entry(c.id.as_str())
                .or_insert_with(|| UnknownShape {
                    id: c.id.clone(),
                    count: 0,
                    size_mm: (w, h),
                    strokes: c.strokes(),
                    example_at: at,
                    nearby: Vec::new(),
                });
            summary.count += 1;
            let reach = w.max(h) * 0.75 + 4.0;
            let near = nearby.entry(c.id.as_str()).or_default();
            for l in lettering {
                let (lx, ly) = (l.at.0 / upm, l.at.1 / upm);
                if (lx - c.centre.0).abs() <= reach && (ly - c.centre.1).abs() <= reach {
                    *near.entry(l.value.as_str()).or_default() += 1;
                }
            }
        }
    }
    let mut unknown: Vec<UnknownShape> = summaries
        .into_iter()
        .map(|(id, mut summary)| {
            let mut near: Vec<(String, usize)> = nearby
                .remove(id)
                .unwrap_or_default()
                .into_iter()
                .map(|(v, n)| (v.to_string(), n))
                .collect();
            near.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            near.truncate(4);
            summary.nearby = near;
            summary
        })
        .collect();
    unknown.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));

    // ── second pass: the tags still free, over the strokes the pipe rule took
    if shape_rules.recover_min_runs > 0 && shape_rules.pipe_stub_mm > 0.0 {
        for (comp, handles, claim) in recovered_symbols(
            &loose,
            &in_symbol,
            rules,
            shape_rules,
            lettering,
            upm,
            taken_text,
        ) {
            let at = (comp.centre.0 * upm, comp.centre.1 * upm);
            let source = format!(
                "shape {} ({} strokes, second pass)",
                comp.id,
                comp.strokes()
            );
            if let Some((ri, k, d)) = claim {
                let rule = &rules.tag_classes[ri];
                symbols.push((
                    Recognized {
                        class: rule.class.clone(),
                        label: rule.label.clone(),
                        color: rule.color,
                        at,
                        bbox: scale(comp.bbox),
                        source,
                        known: true,
                        inner_text: Vec::new(),
                        tag: Some(lettering[k].value.clone()),
                        tag_distance_mm: Some(d),
                        wants_tag: true,
                        report_untagged: rule.report_orphans,
                        lines: Vec::new(),
                        handles,
                        tag_handles: vec![lettering[k].handle],
                    },
                    TagRule::default(),
                ));
            } else if let Some(rule) = shape_rules.dictionary.get(&comp.id) {
                // Named by the dictionary, as a first-pass component would be;
                // one of these that no tag took is still the symbol it is.
                if rule.class == IGNORE_CLASS {
                    continue;
                }
                symbols.push((
                    Recognized {
                        class: rule.class.clone(),
                        label: rule.label.clone(),
                        color: rule.color,
                        at,
                        bbox: scale(comp.bbox),
                        source,
                        known: true,
                        inner_text: Vec::new(),
                        tag: None,
                        tag_distance_mm: None,
                        wants_tag: rule.tag.wants_tag(),
                        report_untagged: rule.tag.reports_untagged(),
                        lines: Vec::new(),
                        handles,
                        tag_handles: Vec::new(),
                    },
                    rule.tag.clone(),
                ));
            }
        }
    }
    Exploded { symbols, unknown }
}

/// A tag's claim on a component: `(tag rule, text, mm)`.
type Claim = (usize, usize, f64);

/// The candidate components of the second pass, each with the entities its
/// strokes came from and the claim a tag no first-pass component took makes
/// on it, or none. A candidate nobody claims is the caller's to name by the
/// dictionary.
///
/// A symbol drawn in pipe-length strokes -- the flame arrester's frame of
/// 2.7 and 3.2 mm lines, the flow indicator's 3.7 x 4.9 mm body -- loses all
/// of them to `pipe_stub_mm` and leaves nothing for its tag to claim. Here
/// the strokes not yet part of a symbol are clustered again together with
/// those pipe-length runs (`Loose::pipe_prims`) that are *framed*: held at
/// two or more points by other strokes of this pool, not counting a run
/// along the same line. A frame's side has the two neighbouring sides on
/// it, a T's bar has its stem and a side; a pipe leading off the symbol is
/// held at one end, by the symbol, and the pipe between two valves is held
/// by the valves, which are symbols already and so not in the pool. The
/// pipe is then taken out of each component as in the first pass, and a
/// component of at least three strokes, `recover_min_runs` of them such
/// runs, is a candidate. Tags pair with candidates as in the first pass:
/// most pairs, then shortest -- a candidate the dictionary names as another
/// class left out of that -- then nearest free for the rest.
fn recovered_symbols(
    loose: &Loose,
    in_symbol: &[bool],
    rules: &Rules,
    shape_rules: &ShapeRules,
    lettering: &[Lettering],
    upm: f64,
    taken_text: &mut [bool],
) -> Vec<(Component, Vec<Handle>, Option<Claim>)> {
    let eps = shape_rules.touch_mm;
    let free: Vec<usize> = (0..loose.prims.len()).filter(|&i| !in_symbol[i]).collect();
    if loose.pipe_prims.is_empty() {
        return Vec::new();
    }
    // The pool: the free strokes first, then every pipe-length run.
    let mut pool: Vec<Prim> = free.iter().map(|&i| loose.prims[i].clone()).collect();
    pool.extend(loose.pipe_prims.iter().cloned());
    let n_free = free.len();
    let collinear = |a: &Prim, b: &Prim| match (a.axis_run(), b.axis_run()) {
        (Some(ra), Some(rb)) => ra.axis == rb.axis && (ra.at - rb.at).abs() <= eps,
        _ => false,
    };
    let framed: Vec<bool> = (0..pool.len())
        .map(|i| {
            if i < n_free {
                return true;
            }
            let (run, bi) = (&pool[i], pool[i].bbox);
            let held = pool
                .iter()
                .enumerate()
                .filter(|&(j, other)| {
                    let bj = other.bbox;
                    j != i
                        && !(bi.0 > bj.2 + eps
                            || bj.0 > bi.2 + eps
                            || bi.1 > bj.3 + eps
                            || bj.1 > bi.3 + eps)
                        && !collinear(run, other)
                        && touches(run, other, eps)
                })
                .count();
            held >= 2
        })
        .collect();
    // Renumber to the strokes of the pass: the free strokes (all of them, so
    // they keep the indices 0..n_free), then the framed runs.
    let mut prims: Vec<Prim> = Vec::new();
    let mut of_prim: HashMap<usize, usize> = HashMap::new(); // Loose::prims index -> pass index
    let mut of_pipe: HashMap<usize, usize> = HashMap::new(); // Loose::pipe_prims index -> pass index
    for (p, prim) in pool.into_iter().enumerate() {
        if !framed[p] {
            continue;
        }
        if p < n_free {
            of_prim.insert(free[p], prims.len());
        } else {
            of_pipe.insert(p - n_free, prims.len());
        }
        prims.push(prim);
    }
    if prims.len() == n_free {
        return Vec::new();
    }
    // The sheet's runs in this pass's numbering: a symbol's stroke is gone
    // (it can no more thread a component here than in the first pass), a
    // pipe-length run is a stroke of the pass when framed and pipe when not.
    let runs: Vec<SheetRun> = loose
        .runs
        .iter()
        .filter_map(|r| {
            let prim = match (r.prim, r.pipe) {
                (Some(i), _) => Some(*of_prim.get(&i)?),
                (None, Some(p)) => of_pipe.get(&p).copied(),
                (None, None) => None,
            };
            Some(SheetRun {
                run: r.run,
                prim,
                pipe: None,
            })
        })
        .collect();
    let min_strokes = shape_rules.split_min_strokes;
    let trim = |idxs: Vec<usize>| {
        if min_strokes > 0 {
            trim_pipe_stubs(&prims, idxs, &loose.circles, eps)
        } else {
            idxs
        }
    };
    let mut comps: Vec<Component> = Vec::new();
    for group in components(&prims, eps) {
        let runs = runs_for_group(&runs, &group, min_strokes);
        for part in split_at_bridges(&prims, &runs, trim(group), min_strokes, eps) {
            let idxs = trim(part);
            let held_runs = idxs.iter().filter(|&&i| i >= n_free).count();
            if idxs.len() < 3 || held_runs < shape_rules.recover_min_runs {
                continue;
            }
            if let Some(comp) = Component::of(&prims, idxs, shape_rules) {
                comps.push(comp);
            }
        }
    }
    // Pair as the first pass does.
    let mut claims: Vec<(f64, usize, usize, usize)> = Vec::new(); // (mm, text, comp, rule)
    for (ri, rule) in rules.tag_classes.iter().enumerate() {
        let reach = rule.radius_mm.unwrap_or(rules.radius_mm);
        if reach <= 0.0 {
            continue;
        }
        for (k, l) in lettering.iter().enumerate() {
            if taken_text[k] || !shape_matches(&rule.shape, &l.value) {
                continue;
            }
            let at = (l.at.0 / upm, l.at.1 / upm);
            for (ci, c) in comps.iter().enumerate() {
                let (w, h) = c.size();
                if w < rule.min_side_mm || h < rule.min_side_mm {
                    continue;
                }
                let d = (c.centre.0 - at.0).hypot(c.centre.1 - at.1);
                if d <= reach {
                    claims.push((d, k, ci, ri));
                }
            }
        }
    }
    claims.sort_by(|a, b| a.0.total_cmp(&b.0));
    let eligible: Vec<usize> = (0..claims.len())
        .filter(|&e| {
            let (_, _, ci, ri) = claims[e];
            !shape_rules
                .dictionary
                .get(&comps[ci].id)
                .is_some_and(|named| named.class != rules.tag_classes[ri].class)
        })
        .collect();
    let pairs: Vec<(f64, usize, usize)> = eligible
        .iter()
        .map(|&e| (claims[e].0, claims[e].1, claims[e].2))
        .collect();
    let mut chosen: Vec<usize> = match_pairs(&pairs)
        .into_iter()
        .map(|p| eligible[p])
        .collect();
    let mut text_used = vec![false; lettering.len()];
    let mut comp_used = vec![false; comps.len()];
    for &e in &chosen {
        text_used[claims[e].1] = true;
        comp_used[claims[e].2] = true;
    }
    for (e, &(_, k, ci, _)) in claims.iter().enumerate() {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        chosen.push(e);
    }
    let mut taken_comp: Vec<Option<Claim>> = vec![None; comps.len()];
    for e in chosen {
        let (d, k, ci, ri) = claims[e];
        taken_text[k] = true;
        taken_comp[ci] = Some((ri, k, d));
    }
    comps
        .into_iter()
        .zip(taken_comp)
        .map(|(comp, claim)| {
            let handles = comp.handles(&prims);
            (comp, handles, claim)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bowtie(x: f64, y: f64, turn: fn((f64, f64)) -> (f64, f64)) -> Vec<Prim> {
        // Two triangles tip to tip, 2.4 x 1.2 mm, placed with `turn`.
        let p = |dx: f64, dy: f64| {
            let (tx, ty) = turn((dx, dy));
            (x + tx, y + ty)
        };
        vec![
            Prim::new(PrimKind::Line, vec![p(-1.2, -0.6), p(-1.2, 0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(-1.2, 0.6), p(1.2, -0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(1.2, -0.6), p(1.2, 0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(1.2, 0.6), p(-1.2, -0.6)], 0.0).unwrap(),
        ]
    }

    #[test]
    fn a_shape_has_the_same_id_however_it_is_turned_or_mirrored() {
        let upright = bowtie(10.0, 10.0, |(x, y)| (x, y));
        let quarter = bowtie(50.0, 80.0, |(x, y)| (-y, x));
        let mirrored = bowtie(90.0, 20.0, |(x, y)| (-x, y));
        let other = bowtie(90.0, 20.0, |(x, y)| (x * 1.5, y));
        let id = |prims: &[Prim]| {
            let idxs: Vec<usize> = (0..prims.len()).collect();
            let mut bbox = None;
            for p in prims {
                grow(&mut bbox, p.bbox.0, p.bbox.1);
                grow(&mut bbox, p.bbox.2, p.bbox.3);
            }
            let b = bbox.unwrap();
            signature(prims, &idxs, ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0), 0.1)
        };
        assert_eq!(id(&upright), id(&quarter));
        assert_eq!(id(&upright), id(&mirrored));
        assert_ne!(id(&upright), id(&other), "a wider valve is another shape");
    }

    #[test]
    fn a_stroke_a_hair_off_square_is_still_an_axis_run() {
        let run = AxisRun::of((275.47, 241.07), (277.35, 241.03)).expect("0.04 mm off over 1.9 mm");
        assert_eq!(run.axis, Axis::Horizontal);
        assert!((run.len() - 1.88).abs() < 1e-9);
        assert!((run.at - 241.05).abs() < 1e-9);
        assert!(
            AxisRun::of((0.0, 0.0), (1.0, 0.3)).is_none(),
            "a slope is not a run"
        );
        assert!(
            AxisRun::of((1.0, 1.0), (1.0, 1.0)).is_none(),
            "nor is a point"
        );
        let v = AxisRun::of((5.0, 9.0), (5.02, 1.0)).unwrap();
        assert_eq!(v.axis, Axis::Vertical);
        assert!((v.at - 5.01).abs() < 1e-9 && v.lo == 1.0 && v.hi == 9.0);
        // Through a box's body, not along its edge.
        assert!(v.through((4.0, 0.0, 6.0, 10.0), 0.05));
        assert!(!v.through((5.0, 0.0, 6.0, 10.0), 0.05));
    }

    #[test]
    fn touching_strokes_form_one_component_and_a_gap_breaks_it() {
        let mut prims = bowtie(0.0, 0.0, |(x, y)| (x, y));
        prims.extend(bowtie(20.0, 0.0, |(x, y)| (x, y)));
        // A leader that ends on the first valve's right edge joins it.
        prims.push(Prim::new(PrimKind::Line, vec![(1.2, 0.0), (1.2, 4.0)], 0.0).unwrap());
        let groups = components(&prims, 0.05);
        let mut sizes: Vec<usize> = groups.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [4, 5]);
    }

    #[test]
    fn hash_colours_are_bright() {
        for h in [0u64, 1, 12345, u64::MAX, 0xdead_beef] {
            let [r, g, b] = hash_color(h);
            assert!(r.max(g).max(b) >= 200, "{h}: {r} {g} {b}");
        }
        assert_ne!(hash_color(1), hash_color(100));
    }
}
