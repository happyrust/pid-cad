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
/// `extent` and `driving` are a symbol placement's measures, the same on
/// every entity the placement drew (see [`PlacementMeasures`]). The
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
    use acadrust::xdata::{ExtendedDataRecord, XDataValue};

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

/// What a symbol placement says about its size, written into the
/// `PID_SEMANTICS` record of every entity the placement drew -- body strokes
/// and the name lettered beside them alike -- so the properties panel can
/// answer for any of them (plan `2026-09-18-driving-dimensions-reach-the-
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
                    .filter_map(|primitive| shape_primitive(primitive, &placement))
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
