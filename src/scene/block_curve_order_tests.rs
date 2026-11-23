//! The members of a block are stacked in the order the block stores them.
//! A circle, an arc or an ellipse is drawn by a pipeline of its own and is
//! no exception: a fill stored after it lies over it and a fill stored
//! before it lies under it, wherever in the block the two stand.

use super::Scene;
use crate::scene::model::wire_model::{TangentGeom, WireModel};
use document::entities::hatch::{BoundaryEdge, BoundaryPath, BoundaryPathFlags, LineEdge};
use document::entities::{Arc, Circle, Ellipse, Hatch, Insert, Line};
use document::types::{Vector2, Vector3};
use document::EntityType;

fn curves() -> [(&'static str, EntityType); 3] {
    [
        (
            "circle",
            EntityType::Circle(Circle::from_center_radius(Vector3::ZERO, 2.0)),
        ),
        (
            "arc",
            EntityType::Arc(Arc::from_center_radius_angles(
                Vector3::ZERO,
                2.0,
                0.0,
                std::f64::consts::PI,
            )),
        ),
        (
            "ellipse",
            EntityType::Ellipse(Ellipse::from_center_axes(
                Vector3::ZERO,
                Vector3::new(2.0, 0.0, 0.0),
                0.5,
            )),
        ),
    ]
}

fn stroke(y: f64) -> EntityType {
    EntityType::Line(Line::from_coords(-3.0, y, 0.0, 3.0, y, 0.0))
}

/// A solid fill across the middle of the curves above.
fn fill() -> EntityType {
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        path.edges.push(BoundaryEdge::Line(LineEdge {
            start: Vector2::new(from.0, from.1),
            end: Vector2::new(to.0, to.1),
        }));
    }
    let mut hatch = Hatch::new();
    hatch.paths.push(path);
    EntityType::Hatch(Box::new(hatch))
}

fn is_curve(wire: &WireModel) -> bool {
    !wire.tangent_geoms.is_empty()
        && wire
            .tangent_geoms
            .iter()
            .all(|geom| !matches!(geom, TangentGeom::Line { .. }))
}

/// The depths a plot draws the one curve and the one fill of a scene at. A
/// plot draws the lower depth first, so the higher of the two lies over the
/// other; at equal depths a plot draws the fill first and the screen leaves
/// it to the depth test.
struct Stack {
    curve: f32,
    fill: f32,
}

fn stack_of(scene: &Scene) -> Stack {
    let (wires, _) = scene.plot_wire_groups(None);
    let depths = scene.plot_wire_depths(&wires);
    let curves: Vec<f32> = wires
        .iter()
        .zip(&depths)
        .filter(|(wire, _)| is_curve(wire))
        .map(|(_, depth)| *depth)
        .collect();
    assert_eq!(curves.len(), 1, "the scene draws one curve");
    let fills = scene.paper_plot_hatches();
    assert_eq!(fills.len(), 1, "the scene draws one fill");
    Stack {
        curve: curves[0],
        fill: fills[0].draw_depth,
    }
}

/// One reference of a block that holds `members`, in that order.
fn reference_of(members: Vec<EntityType>) -> Stack {
    let mut scene = Scene::new();
    scene.define_block_raw("SYMBOL", Vector3::ZERO, members);
    scene.add_entity(EntityType::Insert(Insert::new(
        "SYMBOL",
        Vector3::new(10.0, 5.0, 0.0),
    )));
    stack_of(&scene)
}

/// One reference of a block that holds what `neighbours` makes of a
/// reference of a block that holds `members`.
fn assembly_of(
    members: Vec<EntityType>,
    neighbours: impl Fn(EntityType) -> Vec<EntityType>,
) -> Stack {
    let mut scene = Scene::new();
    scene.define_block_raw("SYMBOL", Vector3::ZERO, members);
    let nested = EntityType::Insert(Insert::new("SYMBOL", Vector3::new(1.0, 1.0, 0.0)));
    scene.define_block_raw("ASSEMBLY", Vector3::ZERO, neighbours(nested));
    scene.add_entity(EntityType::Insert(Insert::new(
        "ASSEMBLY",
        Vector3::new(10.0, 5.0, 0.0),
    )));
    stack_of(&scene)
}

/// One reference of a block whose only content beside a stroke is a
/// reference of a block that holds `members`.
fn nested_reference_of(members: Vec<EntityType>) -> Stack {
    assembly_of(members, |nested| vec![stroke(4.0), nested])
}

/// Every curve for which `right` does not hold, with the depths it and the
/// fill were drawn at.
fn wrong(
    place: impl Fn(Vec<EntityType>) -> Stack,
    members: impl Fn(EntityType) -> Vec<EntityType>,
    right: impl Fn(&Stack) -> bool,
) -> Vec<String> {
    curves()
        .into_iter()
        .filter_map(|(name, curve)| {
            let stack = place(members(curve));
            (!right(&stack))
                .then(|| format!("{name}: curve at {}, fill at {}", stack.curve, stack.fill))
        })
        .collect()
}

/// A fill stored after a curve lies over it. The fill is the middle one of
/// three members here, the place where a fill and a curve used to tie.
#[test]
fn a_fill_stored_after_a_curve_is_drawn_over_it() {
    let wrong = wrong(
        reference_of,
        |curve| vec![curve, fill(), stroke(2.5)],
        |stack| stack.curve < stack.fill,
    );
    assert!(
        wrong.is_empty(),
        "the fill must lie over the curve: {wrong:?}"
    );
}

/// A curve stored after a fill lies over it, also when the fill stands in
/// the second half of the block.
#[test]
fn a_curve_stored_after_a_fill_is_drawn_over_it() {
    let wrong = wrong(
        reference_of,
        |curve| vec![stroke(2.5), stroke(-2.5), fill(), curve],
        |stack| stack.fill < stack.curve,
    );
    assert!(
        wrong.is_empty(),
        "the curve must lie over the fill: {wrong:?}"
    );
}

/// A fill in the first half of the block still lies over a curve stored
/// before it.
#[test]
fn a_fill_early_in_the_block_is_drawn_over_the_curve_before_it() {
    let wrong = wrong(
        reference_of,
        |curve| vec![curve, fill(), stroke(2.5), stroke(-2.5), stroke(3.5)],
        |stack| stack.curve < stack.fill,
    );
    assert!(
        wrong.is_empty(),
        "the fill must lie over the curve: {wrong:?}"
    );
}

/// The same holds for the members of a block placed inside another block.
#[test]
fn a_curve_in_a_nested_block_keeps_its_place_too() {
    let over = wrong(
        nested_reference_of,
        |curve| vec![stroke(2.5), stroke(-2.5), fill(), curve],
        |stack| stack.fill < stack.curve,
    );
    assert!(
        over.is_empty(),
        "the curve must lie over the fill: {over:?}"
    );
    let under = wrong(
        nested_reference_of,
        |curve| vec![curve, fill(), stroke(2.5)],
        |stack| stack.curve < stack.fill,
    );
    assert!(
        under.is_empty(),
        "the fill must lie over the curve: {under:?}"
    );
}

/// The curves of a block placed inside another block stay where that block
/// stands among its neighbours: the first member of a block stored after a
/// fill still lies over the fill, and the last member of a block stored
/// before a fill still lies under it.
#[test]
fn a_curve_in_a_nested_block_stays_where_that_block_stands() {
    let strokes = || (0..6).map(|row| stroke(f64::from(row) - 2.5));
    let over = wrong(
        |members| assembly_of(members, |nested| vec![fill(), nested]),
        |curve| std::iter::once(curve).chain(strokes()).collect(),
        |stack| stack.fill < stack.curve,
    );
    assert!(
        over.is_empty(),
        "the curve must lie over the fill: {over:?}"
    );
    let under = wrong(
        |members| assembly_of(members, |nested| vec![nested, fill()]),
        |curve| strokes().chain(std::iter::once(curve)).collect(),
        |stack| stack.curve < stack.fill,
    );
    assert!(
        under.is_empty(),
        "the fill must lie over the curve: {under:?}"
    );
}
