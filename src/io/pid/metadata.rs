// What rides on each entity: the `PID_SEMANTICS` XDATA, and the measures a symbol placement states there.

use super::*;

/// Write an entity's published P&ID identity into its XDATA, under
/// [`PID_SEMANTICS_XDATA_APP`] as self-describing `key=value` strings.
///
/// Four sources, four vocabularies, one record. The authored layer pair
/// (`sheet_layer`, `sheet_layer_oid`) is what the drawing files the entity
/// under, carried whenever the record declares one. `role` and `style` are
/// the importer's reading: `role` is what the entity is (see
/// [`role_of_layer`]), `style` the name the drawing gives the line style it
/// draws with -- the name `discipline_layer` derives a `PID-STYLE-*` layer
/// from, stated even when that name is an appearance and earns no layer.
/// `extent` and `driving` are a symbol placement's measures, stated on the
/// placement's block reference (see [`PlacementMeasures`]). The
/// published pairs come from the drawing's own `_Data.xml`, joined by
/// `pid-parse`'s semantic index: `class` is the owning object's XML element
/// name (`PIDPipeline`, `PIDProcessVessel`, …), `label` its `ItemTag` /
/// `Name`, `oid` the published `GraphicOID`, and `resolved` says which hop
/// found it (`direct`, or `dependency:<aggregate oid>`).
///
/// `class` and `role` are deliberately two keys: the first is the file's
/// business object, the second this importer's classification, and legend
/// recognition (`pid_legend::xdata`) writes `class` too. Records it owns are
/// marked `resolved=legend:*`; nothing written here carries that prefix, so
/// `PIDLEGEND PURGE` leaves import identity alone.
pub(super) fn attach_pid_metadata(
    entity: &mut EntityType,
    hit: Option<&PidSemanticHit<'_>>,
    source_layer: Option<&pid_parse::PidSourceLayer>,
    role: Option<&str>,
    style: Option<&str>,
    measures: Option<&PlacementMeasures>,
) {
    use document::xdata::{ExtendedDataRecord, XDataValue};

    fn push_pair(record: &mut ExtendedDataRecord, key: &str, value: &str) {
        if !value.is_empty() {
            record.add_value(XDataValue::String(format!("{key}={value}")));
        }
    }

    let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
    if let Some(layer) = source_layer {
        if let Some(name) = layer.name.as_deref() {
            push_pair(&mut record, "sheet_layer", name);
        }
        push_pair(&mut record, "sheet_layer_oid", &layer.oid.to_string());
    }
    if let Some(role) = role {
        push_pair(&mut record, "role", role);
    }
    if let Some(style) = style {
        push_pair(&mut record, "style", style);
    }
    if let Some(measures) = measures {
        if let Some(extent) = measures.extent.as_deref() {
            push_pair(&mut record, "extent", extent);
        }
        if let Some(driving) = measures.driving.as_deref() {
            push_pair(&mut record, "driving", driving);
        }
        if let Some(instance) = measures.instance.as_deref() {
            push_pair(&mut record, "instance", instance);
        }
    }
    if let Some(hit) = hit {
        let object = hit.object();
        push_pair(&mut record, "class", &object.class);
        if let Some(label) = object.label() {
            push_pair(&mut record, "label", label);
        }
        push_pair(&mut record, "oid", &object.graphic_oid.to_string());
        match hit {
            PidSemanticHit::Direct(_) => push_pair(&mut record, "resolved", "direct"),
            PidSemanticHit::ViaDependency { dependency_oid, .. } => {
                push_pair(
                    &mut record,
                    "resolved",
                    &format!("dependency:{dependency_oid}"),
                );
            }
        }
    }
    if !record.values.is_empty() {
        entity.common_mut().extended_data.add_record(record);
    }
}

/// Add `symbol=<name>` to the end of an entity's `PID_SEMANTICS` record: the
/// name of the symbol a placement places, as the drawing spells it. It goes
/// on the placement's block reference, whose block's name is that name made
/// fit for a table and numbered where one symbol draws more than one way --
/// so the key, not the block name, is what says which symbol this is.
pub(super) fn attach_symbol_name(entity: &mut EntityType, name: &str) {
    use document::xdata::{ExtendedDataRecord, XDataValue};

    if name.is_empty() {
        return;
    }
    let extended = &mut entity.common_mut().extended_data;
    let mut record = extended
        .remove_record(PID_SEMANTICS_XDATA_APP)
        .unwrap_or_else(|| ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP));
    record.add_value(XDataValue::String(format!("symbol={name}")));
    extended.add_record(record);
}

/// The value an entity's `PID_SEMANTICS` record states for `key`: the
/// reader every target has (`crate::io::pid_identity`), under the name the
/// importer calls it by, so what an import writes and what is read back off
/// a drawing are read one way.
pub(super) use crate::io::pid_identity::stated as pid_value;

/// The keys of a `PID_SEMANTICS` record, in the order a record states them.
/// A key written after its record was made -- what a symbol is, on its block
/// reference; what a label hangs on, on its text; later a tag the plant
/// database gives -- takes its place in this order (see
/// [`stamp_pid_pairs`]), so a record reads the same whichever way it was
/// arrived at.
const PID_KEY_ORDER: [&str; 19] = [
    "sheet_layer",
    "sheet_layer_oid",
    "role",
    "style",
    "extent",
    "driving",
    "instance",
    "class",
    "label",
    "oid",
    "resolved",
    "symbol",
    "graphic",
    "item",
    "item_type",
    "tag_source",
    "label_in_file",
    "label_of",
    "label_on",
];

/// Set `key=value` pairs in an entity's `PID_SEMANTICS` record, making the
/// record where the entity has none.
///
/// A key the record already holds has its value replaced where it stands. A
/// new key goes in after the last key that comes no later than it in
/// [`PID_KEY_ORDER`], and a key that order does not know after every key.
/// An empty value writes nothing, and what the record holds that is not a
/// `key=value` string stays where it is. The record keeps its place among
/// the entity's records.
pub(super) fn stamp_pid_pairs(entity: &mut EntityType, pairs: &[(&str, &str)]) {
    use document::xdata::{ExtendedDataRecord, XDataValue};

    fn key_of(value: &XDataValue) -> Option<&str> {
        match value {
            XDataValue::String(text) => text.split_once('=').map(|(key, _)| key),
            _ => None,
        }
    }

    fn rank(key: &str) -> usize {
        PID_KEY_ORDER
            .iter()
            .position(|known| *known == key)
            .unwrap_or(PID_KEY_ORDER.len())
    }

    if pairs.iter().all(|(_, value)| value.is_empty()) {
        return;
    }
    let extended = &mut entity.common_mut().extended_data;
    let mut record = extended
        .get_record(PID_SEMANTICS_XDATA_APP)
        .cloned()
        .unwrap_or_else(|| ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP));
    for (key, value) in pairs {
        if value.is_empty() {
            continue;
        }
        let stated = XDataValue::String(format!("{key}={value}"));
        if let Some(held) = record
            .values
            .iter_mut()
            .find(|held| key_of(held) == Some(*key))
        {
            *held = stated;
            continue;
        }
        let rank_of_key = rank(key);
        let after = record
            .values
            .iter()
            .rposition(|held| key_of(held).is_some_and(|held| rank(held) <= rank_of_key));
        record.values.insert(after.map_or(0, |at| at + 1), stated);
    }
    extended.upsert_record(record);
}

/// State on a symbol's block reference what the drawing file itself says
/// the symbol is (plan 2026-11-20-pid-symbol-identity, SPEC sections 3 and
/// 4.2), and answer whether the reference carries a tag afterwards.
///
/// `graphic` is the placement's own number in the file, and every reference
/// that has one states it: it is what a label names its symbol by, and what
/// joins the reference back to the file. `item` and `item_type` are the
/// plant item the graphic draws and the kind SmartPlant holds it as, where
/// the file states one.
///
/// `class` and `label` are the keys a published `_Data.xml` fills, in the
/// same vocabulary, and what it published stands. The file's own class is
/// written only where nothing resolved the graphic before -- the record has
/// no `resolved` -- and is marked `resolved=item`. The file's own name for
/// the item is written as the tag only where the record has none, and
/// marked `tag_source=file`: the file names fewer items than the plant
/// database tags, and a few differently, so a reader has to be able to tell
/// where a tag came from.
pub(super) fn attach_symbol_identity(
    reference: &mut EntityType,
    graphic_oid: Option<u32>,
    item: Option<&SheetItem>,
) -> bool {
    let graphic = graphic_oid.map(|graphic_oid| graphic_oid.to_string());
    let published = pid_value(reference, "resolved").is_some();
    let mut tagged = pid_value(reference, "label").is_some();
    let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(7);
    if let Some(graphic) = graphic.as_deref() {
        pairs.push(("graphic", graphic));
    }
    if let Some(item) = item {
        pairs.push(("item", &item.uid));
        pairs.push(("item_type", &item.item_type));
        if !published {
            if let Some(class) = item.class {
                pairs.push(("class", class));
                pairs.push(("resolved", "item"));
            }
        }
        if !tagged {
            if let Some(name) = item.name.as_deref() {
                pairs.push(("label", name));
                pairs.push(("tag_source", "file"));
                tagged = true;
            }
        }
    }
    stamp_pid_pairs(reference, &pairs);
    tagged
}

/// State on a loose text what its label hangs on (plan
/// 2026-11-20-pid-symbol-identity, SPEC sections 3 and 4.3).
///
/// `label_of` is the number of the graphic the drawing file hangs the label
/// on, and `label_on` says what that graphic is on this sheet: `symbol`, a
/// symbol whose block reference states the same number as its `graphic`;
/// `line`, a run of line; `other`, anything else. A text that is no label's
/// states neither.
pub(super) fn attach_label_owner(text: &mut EntityType, graphic: u32, on: LabelOn) {
    stamp_pid_pairs(
        text,
        &[
            ("label_of", &graphic.to_string()),
            ("label_on", on.as_str()),
        ],
    );
}

/// What a symbol placement says about its size, written into the
/// `PID_SEMANTICS` record of the placement's block reference for the
/// properties panel to show (plan `2026-09-18-driving-dimensions-reach-the-
/// panel-as-library-defaults`, K2).
///
/// The two are different facts and the panel shows them as two rows. The
/// extent is what this drawing actually draws the placement at; the driving
/// dimensions are what the symbol library's template was authored with. They
/// agree only for an instance nobody resized: `DWG-0201`'s Parametric
/// Manifold is placed 172.21 x 71.18 mm while its template's `Left` / `Right`
/// / `Top` say 228.6 x 40.64, and a placed instance carries no dimension
/// values of its own (pid-parse `docs/analysis/2026-09-18-the-parametric-
/// chain-closes-on-the-template-not-the-instance.md`). So the second row is
/// captioned as the library default, and never as the instance's size.
pub(super) struct PlacementMeasures {
    /// `<W>x<H>`, millimetres to two places: the rectangle the placement's
    /// body covers on the sheet, rotation, scale and mirror applied. Measured
    /// on the body the drawing itself caches for the placement -- the
    /// instance as SmartPlant last drew it -- over the strokes on the layers
    /// the file displays, so it is the size of the very strokes on screen
    /// (plan 2026-09-19, P-D7: ` Line2` reads `25.40x0.00` once its
    /// construction tick on a switched-off layer is left out). A placement
    /// the drawing carries no drawable body for is measured on what was
    /// drawn instead, which is the library body or the marker dot. Every
    /// placement writes it (K-D3).
    pub(super) extent: Option<String>,
    /// `<name>:<mm>;<name>:<mm>;…`, two places, in the template's on-disk
    /// order: the named driving dimensions of the library template the
    /// cached body names (`PidSymbolDefinition::template`). `None` for a
    /// placement whose body names no template -- every symbol that is not
    /// parametric, and a parametric one pid-parse could not pair -- so the
    /// panel shows no default it cannot vouch for. A dimension without a name
    /// (a derived one, or one no relation writes) is left out.
    pub(super) driving: Option<String>,
    /// `<name>:<mm>;<name>:<mm>;…`, two places, in the symbol's variable
    /// order: the parameters this drawing's instance was actually placed
    /// with, read off the instance storage's `JFlavorHolder`
    /// (`PidSymbolVariable::instance_value_m`; pid-parse
    /// `docs/analysis/2026-09-20-jflavorholder-carries-the-placed-instances-parameters.md`).
    /// DWG-0201's Manifold reads `Left:57.91;Right:114.30;Top:35.59` here
    /// against `Top:20.32;Left:114.30;Right:114.30` in [`Self::driving`]. An
    /// unstretched instance repeats the defaults. `None` for a placement
    /// whose body carries no variables with an instance value.
    pub(super) instance: Option<String>,
}

impl PlacementMeasures {
    /// The measures of one placement, or `None` for any other kind of record.
    pub(super) fn of(
        kind: &PidGraphicKind,
        geometry: &NormalizedPidGeometry,
        projection: Projection,
        drawn: &[EntityType],
    ) -> Option<Self> {
        let PidGraphicKind::SymbolInstance {
            insertion,
            rotation,
            scale,
            definition,
            ..
        } = kind
        else {
            return None;
        };
        let placement = Placement {
            insertion,
            rotation: *rotation,
            scale: *scale,
            projection,
        };
        let cached = definition
            .and_then(|reference| geometry.symbol_definition(reference))
            .filter(|body| !body.primitives.is_empty());
        let extent = cached
            .and_then(|body| {
                body.visible_primitives()
                    .filter_map(|primitive| {
                        shape_primitive(primitive, None, &placement, &HashMap::new())
                    })
                    .filter_map(|entity| stroke_extent(&entity))
                    .reduce(union_extent)
            })
            .or_else(|| drawn.iter().filter_map(stroke_extent).reduce(union_extent))
            .map(|(min_x, min_y, max_x, max_y)| {
                format!("{:.2}x{:.2}", max_x - min_x, max_y - min_y)
            });
        let driving = cached
            .and_then(|body| body.template)
            .and_then(|template| geometry.symbol_definition(template))
            .map(|template| {
                template
                    .dimensions
                    .iter()
                    .filter_map(|dimension| {
                        dimension
                            .name
                            .as_deref()
                            .map(|name| format!("{name}:{:.2}", projection.mm(dimension.value_m)))
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .filter(|driving| !driving.is_empty());
        let instance = cached
            .map(|body| {
                body.variables
                    .iter()
                    .filter_map(|variable| {
                        variable.instance_value_m.map(|value_m| {
                            format!("{}:{:.2}", variable.name, projection.mm(value_m))
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .filter(|instance| !instance.is_empty());
        Some(Self {
            extent,
            driving,
            instance,
        })
    }
}

/// The rectangle one stroke of a body covers, in millimetres, as
/// `(min_x, min_y, max_x, max_y)`; `None` for lettering and for anything
/// that is not a stroke.
///
/// Unlike [`drawn_extent`], which hangs labels and frames views and may take
/// an arc as its whole circle, this is a number a user reads off the panel,
/// so an arc contributes exactly the sweep it draws: its two ends and
/// whichever quadrant points the sweep passes.
pub(super) fn stroke_extent(entity: &EntityType) -> Option<(f64, f64, f64, f64)> {
    match entity {
        EntityType::Line(_) | EntityType::Circle(_) | EntityType::LwPolyline(_) => {
            drawn_extent(entity)
        }
        EntityType::Arc(arc) => Some(arc_extent(
            (arc.center.x, arc.center.y),
            arc.radius,
            arc.start_angle,
            arc.end_angle,
        )),
        _ => None,
    }
}

/// The rectangle an arc sweeping counter-clockwise from `start_angle` to
/// `end_angle` (radians) covers. Equal angles are read as the whole circle,
/// which is what a DXF arc with coincident ends draws.
pub(super) fn arc_extent(
    center: (f64, f64),
    radius: f64,
    start_angle: f64,
    end_angle: f64,
) -> (f64, f64, f64, f64) {
    use std::f64::consts::{FRAC_PI_2, TAU};
    let start = start_angle.rem_euclid(TAU);
    let mut sweep = (end_angle - start_angle).rem_euclid(TAU);
    if sweep == 0.0 {
        sweep = TAU;
    }
    let end = start + sweep;
    let mut angles = vec![start, end];
    let mut quadrant = (start / FRAC_PI_2).ceil() * FRAC_PI_2;
    while quadrant <= end + 1e-12 {
        angles.push(quadrant);
        quadrant += FRAC_PI_2;
    }
    angles
        .into_iter()
        .map(|angle| {
            let (sin, cos) = angle.sin_cos();
            let (x, y) = (center.0 + radius * cos, center.1 + radius * sin);
            (x, y, x, y)
        })
        .reduce(union_extent)
        .expect("two ends at least")
}
