use crate::t;
use document::{Document, EntityType, Handle, Transparency};

use crate::scene::model::object::{PropSection, PropValue, Property};

pub fn general_section(entity: &EntityType) -> PropSection {
    let common = entity.common();
    let linetype_display = if common.linetype.is_empty() {
        "ByLayer".to_string()
    } else {
        common.linetype.clone()
    };
    let transp_display = match common.transparency {
        Transparency::ByLayer => "ByLayer".to_string(),
        Transparency::ByBlock => "ByBlock".to_string(),
        Transparency::Explicit(alpha) => {
            ((alpha as f64 / 255.0 * 100.0).round() as u32).to_string()
        }
    };
    let color_value = common.color_name.as_deref().map_or_else(
        || PropValue::ColorChoice(common.color),
        |identity| PropValue::NamedColorChoice {
            color: common.color,
            name: identity
                .split_once('$')
                .map(|(_, color_name)| color_name)
                .filter(|color_name| !color_name.is_empty())
                .unwrap_or(identity)
                .to_string(),
        },
    );

    let hyperlink = crate::scene::hyperlink::Link::of(entity)
        .map(|link| link.label())
        .unwrap_or_default();

    PropSection {
        title: t!("General").into_owned(),
        props: vec![
            Property {
                label: t!("Color").into_owned(),
                field: "color",
                value: color_value,
            },
            Property {
                label: t!("Layer").into_owned(),
                field: "layer",
                value: PropValue::LayerChoice(common.layer.clone()),
            },
            Property {
                label: t!("Linetype").into_owned(),
                field: "linetype",
                value: PropValue::LinetypeChoice(linetype_display),
            },
            Property {
                label: t!("Linetype scale").into_owned(),
                field: "linetype_scale",
                value: PropValue::EditText(crate::entities::common::format_decimal(
                    common.linetype_scale,
                )),
            },
            Property {
                label: t!("Plot style").into_owned(),
                field: "plot_style",
                value: PropValue::ReadOnly(
                    match common.plotstyle_flags {
                        0 => "ByLayer",
                        1 => "ByBlock",
                        _ => "ByColor",
                    }
                    .into(),
                ),
            },
            Property {
                label: t!("Lineweight").into_owned(),
                field: "lineweight",
                value: PropValue::LwChoice(common.line_weight),
            },
            Property {
                label: t!("Transparency").into_owned(),
                field: "transparency",
                value: PropValue::EditChoice {
                    value: transp_display,
                    options: crate::scene::creation_style::transparency_options(),
                },
            },
            Property {
                label: t!("Hyperlink").into_owned(),
                field: "hyperlink",
                value: PropValue::Hyperlink(hyperlink),
            },
        ],
    }
}

/// The "P&ID" group: identity a `.pid` import or P&ID legend recognition
/// wrote into the entity's XDATA (see
/// `crate::io::PID_SEMANTICS_XDATA_APP`).
///
/// Present when authored sheet-layer identity, published `_Data.xml` identity,
/// or a `PIDLEGEND ON` result reached the entity. Every row is read-only.
pub fn pid_semantics_section(entity: &EntityType) -> Option<PropSection> {
    let record = entity
        .common()
        .extended_data
        .get_record(crate::io::PID_SEMANTICS_XDATA_APP)?;

    let mut class = None;
    let mut item_type = None;
    let mut role = None;
    let mut symbol = None;
    let mut driving = None;
    let mut instance = None;
    let mut extent = None;
    let mut labels = Vec::new();
    let mut tag_source = None;
    let mut name_in_file = None;
    let mut lines = Vec::new();
    let mut resolved = None;
    let mut sheet_layer = None;
    let mut sheet_layer_oid = None;
    for value in &record.values {
        let document::xdata::XDataValue::String(text) = value else {
            continue;
        };
        let Some((key, val)) = text.split_once('=') else {
            continue;
        };
        match key {
            "class" => class = Some(val.to_string()),
            "item_type" if !val.is_empty() => item_type = Some(val.to_string()),
            "role" => role = Some(val.to_string()),
            "symbol" if !val.is_empty() => symbol = Some(val.to_string()),
            "driving" => driving = driving_dimensions_caption(val),
            "instance" => instance = driving_dimensions_caption(val),
            "extent" => extent = body_extent_caption(val),
            "label" if !val.is_empty() && !labels.iter().any(|old| old == val) => {
                labels.push(val.to_string());
            }
            "tag_source" if !val.is_empty() => tag_source = Some(val.to_string()),
            "label_in_file" if !val.is_empty() => name_in_file = Some(val.to_string()),
            "lines" => {
                for line in val
                    .split(',')
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                {
                    if !lines.iter().any(|old| old == line) {
                        lines.push(line.to_string());
                    }
                }
            }
            "resolved" => resolved = Some(val.to_string()),
            "sheet_layer" => sheet_layer = Some(val.to_string()),
            "sheet_layer_oid" => sheet_layer_oid = Some(val.to_string()),
            _ => {}
        }
    }

    let mut props = Vec::new();
    // What the item is: the class the published data names it as, or, where
    // nothing published names it, the kind the drawing file holds the item
    // as -- which has a row of its own beside it either way (plan
    // 2026-11-20-pid-symbol-identity, SPEC section 4.5).
    let kind = match class.as_deref() {
        Some(named) if !named.is_empty() => Some(named),
        unnamed => item_type.as_deref().or(unnamed),
    };
    if let Some(kind) = kind {
        props.push(Property {
            label: t!("Type").into_owned(),
            field: "pid_class",
            value: PropValue::ReadOnly(kind.to_string()),
        });
    }
    if let Some(item_type) = item_type.as_deref() {
        props.push(Property {
            label: t!("Item type").into_owned(),
            field: "pid_item_type",
            value: PropValue::ReadOnly(item_type.to_string()),
        });
    }
    // What the importer read the entity as -- geometry / text / symbol /
    // symbol-label / point-* / annotation / connectivity / fill / frame --
    // beside what the published data says it is. The two are different
    // questions, so both rows show when both keys are there.
    if let Some(role) = role {
        props.push(Property {
            label: t!("Role").into_owned(),
            field: "pid_role",
            value: PropValue::ReadOnly(role),
        });
    }
    // The symbol a placement's block reference places, named as the drawing
    // spells it (plan 2026-11-19-pid-symbol-block). The reference's block is
    // named after it too, but made fit for a table and numbered where one
    // symbol draws more than one way, so this row, not the block's name,
    // says which symbol it is.
    if let Some(symbol) = symbol {
        props.push(Property {
            label: t!("Symbol").into_owned(),
            field: "pid_symbol",
            value: PropValue::ReadOnly(symbol),
        });
    }
    // A symbol placement's sizes, kept apart on purpose: the driving
    // dimensions are the symbol library's template values, the instance row
    // is the parameters this drawing's placement was actually drawn with
    // (its storage's `JFlavorHolder`; plan 2026-09-20), and the extent is
    // what this drawing draws the placement at. The captions say which is
    // which so nobody reads 20.32 off a Manifold placed with Top 35.59
    // (plan 2026-09-18, K-D1 / K-D3). Every placement has an extent; only
    // one paired with its template has driving dimensions, and only a
    // parametric one has instance parameters.
    if let Some(driving) = driving {
        props.push(Property {
            label: t!("Driving dimensions (library default)").into_owned(),
            field: "pid_driving",
            value: PropValue::ReadOnly(driving),
        });
    }
    if let Some(instance) = instance {
        props.push(Property {
            label: t!("Driving dimensions (this drawing's instance)").into_owned(),
            field: "pid_instance",
            value: PropValue::ReadOnly(instance),
        });
    }
    if let Some(extent) = extent {
        props.push(Property {
            label: t!("Body extent").into_owned(),
            field: "pid_extent",
            value: PropValue::ReadOnly(extent),
        });
    }
    if let Some(first) = labels.first() {
        // A pipe's identifier is its line number; everything else carries an
        // item tag. A source handle shared by two differently numbered runs
        // carries two label values; show the first and make the extra count
        // explicit instead of silently discarding it.
        let label_caption =
            if matches!(class.as_deref(), Some("PIDPipeline" | "PIDPipingConnector")) {
                t!("Line number")
            } else {
                t!("Item tag")
            };
        let label_value = if labels.len() > 1 {
            format!("{first} +{}", labels.len() - 1)
        } else {
            first.clone()
        };
        props.push(Property {
            label: label_caption.into_owned(),
            field: "pid_label",
            value: PropValue::ReadOnly(label_value),
        });
        // Where the tag came from, as the import marks it: the drawing
        // file's own name for the item, or the plant database of a backup.
        // A tag the published data gave is marked with nothing and has no
        // such row; a mark this version does not know shows as written.
        if let Some(source) = tag_source {
            props.push(Property {
                label: t!("Tag source").into_owned(),
                field: "pid_label_source",
                value: PropValue::ReadOnly(match source.as_str() {
                    "file" => t!("Drawing file").into_owned(),
                    "database" => t!("Backup database").into_owned(),
                    _ => source,
                }),
            });
        }
    }
    // The drawing file's own name for the item, kept where a tag from
    // elsewhere took its place.
    if let Some(name) = name_in_file {
        props.push(Property {
            label: t!("Name in file").into_owned(),
            field: "pid_label_in_file",
            value: PropValue::ReadOnly(name),
        });
    }
    if !lines.is_empty() {
        props.push(Property {
            label: t!("Line number").into_owned(),
            field: "pid_lines",
            value: PropValue::ReadOnly(lines.join(", ")),
        });
    }
    if let Some(resolved_value) = resolved {
        props.push(Property {
            label: t!("Matched by").into_owned(),
            field: "pid_resolved",
            value: PropValue::ReadOnly(if resolved_value.starts_with("legend:") {
                t!("Legend recognition").into_owned()
            } else if resolved_value == "item" {
                // Nothing published resolved the entity: the drawing file's
                // own statement of the item it draws did.
                t!("Item in the drawing file").into_owned()
            } else {
                resolved_value
            }),
        });
    }
    if let Some(layer_value) = sheet_layer {
        props.push(Property {
            label: t!("Sheet layer").into_owned(),
            field: "pid_sheet_layer",
            value: PropValue::ReadOnly(layer_value),
        });
    }
    if let Some(oid_value) = sheet_layer_oid {
        props.push(Property {
            label: t!("Layer OID").into_owned(),
            field: "pid_sheet_layer_oid",
            value: PropValue::ReadOnly(oid_value),
        });
    }

    if props.is_empty() {
        return None;
    }

    Some(PropSection {
        title: "P&ID".to_string(),
        props,
    })
}

/// How many of a symbol's labels the `Labels` row spells out before it
/// counts the rest.
const LABELS_SPELT_OUT: usize = 3;

/// Add to the "P&ID" group the one row that takes the sheet around the
/// entity to fill (plan 2026-11-20-pid-symbol-identity, SPEC section 4.5):
/// for the block reference of an imported symbol, the labels the sheet
/// hangs on it; for the text of a label, what the label hangs on.
///
/// The row stands before the ones that say how the entity was matched and
/// which sheet layer it is on, in the group [`pid_semantics_section`] made;
/// where the entity states nothing that makes one, the row makes the group,
/// after the sections there are. An entity that pairs with nothing leaves
/// `sections` as they were.
///
/// It looks through model space once, so it is for the page of one selected
/// object, not for every object of a selection.
pub fn add_pid_pairing_row(
    sections: &mut Vec<PropSection>,
    document: &Document,
    entity: &EntityType,
) {
    let Some(row) = pid_pairing_row(document, entity) else {
        return;
    };
    match sections.iter_mut().find(|section| section.title == "P&ID") {
        Some(group) => {
            let at = group
                .props
                .iter()
                .position(|property| {
                    matches!(
                        property.field,
                        "pid_resolved" | "pid_sheet_layer" | "pid_sheet_layer_oid"
                    )
                })
                .unwrap_or(group.props.len());
            group.props.insert(at, row);
        }
        None => sections.push(PropSection {
            title: "P&ID".to_string(),
            props: vec![row],
        }),
    }
}

/// The row [`add_pid_pairing_row`] adds for `entity`, where it pairs with
/// anything on the sheet.
fn pid_pairing_row(document: &Document, entity: &EntityType) -> Option<Property> {
    use crate::io::pid_identity::{label_owner, labels_of, lettering, symbol_identity, LabelOwner};

    if let Some(symbol) = symbol_identity(entity) {
        // How many labels, and what the first few read; `+n` for the rest.
        let labels = labels_of(document, symbol.graphic?);
        if labels.is_empty() {
            return None;
        }
        let spelt: Vec<&str> = labels
            .iter()
            .copied()
            .take(LABELS_SPELT_OUT)
            .filter_map(lettering)
            .map(str::trim)
            .collect();
        let mut value = format!("{}: {}", labels.len(), spelt.join(" · "));
        if labels.len() > LABELS_SPELT_OUT {
            value.push_str(&format!(" +{}", labels.len() - LABELS_SPELT_OUT));
        }
        return Some(Property {
            label: t!("Labels").into_owned(),
            field: "pid_labels",
            value: PropValue::ReadOnly(value),
        });
    }

    let value = match label_owner(document, entity)? {
        // Its symbol by its tag; one with no tag by what kind of thing it
        // is and the name of its symbol, as far as it states them.
        LabelOwner::Symbol(reference) => {
            let symbol = symbol_identity(reference).unwrap_or_default();
            match symbol.tag {
                Some(tag) => tag.to_string(),
                None => {
                    let named: Vec<&str> = symbol.kind().into_iter().chain(symbol.symbol).collect();
                    if named.is_empty() {
                        t!("Symbol").into_owned()
                    } else {
                        named.join(" · ")
                    }
                }
            }
        }
        LabelOwner::Missing => t!("Not on the sheet").into_owned(),
        LabelOwner::Line => t!("A line").into_owned(),
        LabelOwner::Other => t!("Another object").into_owned(),
    };
    Some(Property {
        label: t!("Label of").into_owned(),
        field: "pid_label_of",
        value: PropValue::ReadOnly(value),
    })
}

/// `Top:20.32;Left:114.30;Right:114.30` as the panel shows it:
/// `Top 20.32 mm · Left 114.30 mm · Right 114.30 mm`. The import wrote the
/// values in millimetres already, so nothing is converted here; a value that
/// is not `name:number` is shown as written rather than dropped, and an
/// empty key shows nothing.
fn driving_dimensions_caption(value: &str) -> Option<String> {
    let parts: Vec<String> = value
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once(':') {
            Some((name, millimetres)) if millimetres.parse::<f64>().is_ok() => {
                format!("{} {} mm", name.trim(), millimetres.trim())
            }
            _ => part.to_string(),
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `172.21x71.18` as the panel shows it: `172.21 × 71.18 mm`. Anything that
/// is not two numbers around an `x` is shown as written.
fn body_extent_caption(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(match value.split_once('x') {
        Some((width, height))
            if width.trim().parse::<f64>().is_ok() && height.trim().parse::<f64>().is_ok() =>
        {
            format!("{} × {} mm", width.trim(), height.trim())
        }
        _ => value.to_string(),
    })
}

/// The "3D Visualization" group (Material), common to every graphical object.
/// Material source is flag-based; a custom material handle is shown as "Custom"
/// (name resolution needs the doc).
pub fn visualization_section(entity: &EntityType) -> Option<PropSection> {
    if matches!(
        entity,
        EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::Leader(_)
            | EntityType::Wipeout(_)
            // The reference lists no 3D group for an underlay; a point
            // cloud's is its own (stylization and colour scheme).
            | EntityType::Underlay(_)
            // Nor for an OLE object or a multileader.
            | EntityType::Ole2Frame(_)
            | EntityType::MultiLeader(_)
            | EntityType::Unknown(_)
            // Non-plotting drawing-view border: never rendered, no properties.
            | EntityType::ViewBorder(_)
    ) || matches!(
        entity,
        EntityType::Extended(extended)
            if matches!(extended.data, document::entities::ExtendedEntityData::PointCloudEx(_))
    ) {
        return None;
    }
    let common = entity.common();
    let material = match common.material_flags {
        0 => "ByLayer",
        1 => "ByBlock",
        2 => "Global",
        _ => "Custom",
    };
    let mut options = vec![
        "ByLayer".to_string(),
        "ByBlock".to_string(),
        "Global".to_string(),
    ];
    if !options.iter().any(|o| o == material) && !material.is_empty() {
        options.push(material.to_string());
    }
    Some(PropSection {
        title: t!("3D Visualization").into_owned(),
        props: vec![Property {
            label: t!("Material").into_owned(),
            field: "material",
            value: PropValue::Choice {
                selected: material.to_string(),
                options,
            },
        }],
    })
}

pub fn fallback_properties(_handle: Handle, entity: &EntityType) -> PropSection {
    PropSection {
        title: t!("Geometry").into_owned(),
        props: vec![Property {
            label: t!("Type").into_owned(),
            field: "type",
            value: PropValue::ReadOnly(
                crate::t!(crate::entities::names::ui_name_or_class(entity)).into_owned(),
            ),
        }],
    }
}

#[cfg(test)]
mod pid_semantics_tests {
    use super::*;
    use document::entities::{Insert, Line, Text};
    use document::types::Vector3;
    use document::xdata::{ExtendedDataRecord, XDataValue};

    /// `entity` with `values` as its `PID_SEMANTICS` record.
    fn saying(mut entity: EntityType, values: &[&str]) -> EntityType {
        let mut record = ExtendedDataRecord::new(crate::io::PID_SEMANTICS_XDATA_APP);
        for value in values {
            record.add_value(XDataValue::String((*value).to_string()));
        }
        entity.common_mut().extended_data.add_record(record);
        entity
    }

    fn entity(values: &[&str]) -> EntityType {
        saying(EntityType::Line(Line::new()), values)
    }

    /// A block reference that says `values`.
    fn reference(values: &[&str]) -> EntityType {
        saying(
            EntityType::Insert(Insert::new("SPPID_Ball Valve", Vector3::new(0.0, 0.0, 0.0))),
            values,
        )
    }

    /// A text that reads `lettering` and says `values`.
    fn text(lettering: &str, values: &[&str]) -> EntityType {
        let mut text = Text::new();
        text.value = lettering.to_string();
        saying(EntityType::Text(text), values)
    }

    /// A sheet with `entities` in model space, and their handles in the
    /// order they were filed.
    fn sheet(entities: Vec<EntityType>) -> (Document, Vec<Handle>) {
        let mut doc = Document::new();
        let filed = entities
            .into_iter()
            .map(|entity| {
                doc.add_entity(entity)
                    .expect("model space takes the entity")
            })
            .collect();
        (doc, filed)
    }

    /// The `P&ID` group of the entity filed under `handle` as the page of
    /// one selected object shows it: what the entity states, and the row
    /// that takes the sheet around it.
    fn on_sheet(doc: &Document, handle: Handle) -> Option<PropSection> {
        let entity = doc.get_entity(handle).expect("filed on the sheet");
        let mut sections: Vec<PropSection> = pid_semantics_section(entity).into_iter().collect();
        add_pid_pairing_row(&mut sections, doc, entity);
        assert!(sections.len() <= 1, "one P&ID group at most");
        sections.pop()
    }

    /// The fields of a group's rows, in the order the panel shows them.
    fn fields(section: &PropSection) -> Vec<&'static str> {
        section.props.iter().map(|row| row.field).collect()
    }

    /// What a row is captioned.
    fn caption<'a>(section: &'a PropSection, field: &str) -> &'a str {
        &section
            .props
            .iter()
            .find(|property| property.field == field)
            .unwrap_or_else(|| panic!("missing {field}"))
            .label
    }

    /// What a row reads, or `None` where the group has no such row.
    fn reads(section: &PropSection, field: &str) -> Option<String> {
        section
            .props
            .iter()
            .any(|property| property.field == field)
            .then(|| value(section, field).to_string())
    }

    fn value<'a>(section: &'a PropSection, field: &str) -> &'a str {
        match &section
            .props
            .iter()
            .find(|property| property.field == field)
            .unwrap_or_else(|| panic!("missing {field}"))
            .value
        {
            PropValue::ReadOnly(value) => value,
            other => panic!("{field} is not read-only: {other:?}"),
        }
    }

    #[test]
    fn legend_identity_shows_connected_lines_and_a_friendly_match_source() {
        let section = pid_semantics_section(&entity(&[
            "class=蝶阀",
            "label=BUV-3201",
            "lines=100-FW, 150-FW",
            "resolved=legend:block",
        ]))
        .expect("P&ID section");

        assert_eq!(value(&section, "pid_class"), "蝶阀");
        assert_eq!(value(&section, "pid_label"), "BUV-3201");
        assert_eq!(value(&section, "pid_lines"), "100-FW, 150-FW");
        assert_eq!(
            value(&section, "pid_resolved"),
            t!("Legend recognition").as_ref()
        );
    }

    #[test]
    fn a_pipeline_with_two_labels_shows_the_first_and_the_extra_count() {
        let section = pid_semantics_section(&entity(&[
            "class=PIDPipeline",
            "label=100-FW",
            "label=150-FW",
            "resolved=legend:pipe",
        ]))
        .expect("P&ID section");

        assert_eq!(value(&section, "pid_label"), "100-FW +1");
        assert!(section
            .props
            .iter()
            .find(|property| property.field == "pid_label")
            .is_some_and(|property| property.label == t!("Line number")));
    }

    /// A `.pid` import writes what it read the entity as (`role=`) beside the
    /// published object's class (`class=`); both show, as two rows, with the
    /// role right after the type (plan 2026-09-07, L2 step 4). An entity the
    /// importer drew itself has a role and nothing else, and still gets its
    /// P&ID group.
    #[test]
    fn an_imported_entity_shows_its_role_beside_its_type() {
        let section = pid_semantics_section(&entity(&[
            "class=PIDProcessVessel",
            "label=V-101",
            "sheet_layer=Default",
            "sheet_layer_oid=42",
            "role=symbol",
            "style=Equipment",
        ]))
        .expect("P&ID section");
        assert_eq!(value(&section, "pid_class"), "PIDProcessVessel");
        assert_eq!(value(&section, "pid_role"), "symbol");
        let fields: Vec<&str> = section.props.iter().map(|p| p.field).collect();
        assert_eq!(fields[..2], ["pid_class", "pid_role"]);
        assert!(section
            .props
            .iter()
            .find(|property| property.field == "pid_role")
            .is_some_and(|property| property.label == t!("Role")));

        let frame = pid_semantics_section(&entity(&["role=frame"])).expect("P&ID section");
        assert_eq!(value(&frame, "pid_role"), "frame");
        assert!(frame.props.iter().all(|p| p.field != "pid_class"));
    }

    /// A placed parametric symbol shows two sizes, right after its role: the
    /// library template's driving dimensions, captioned as the library
    /// default, and the extent this drawing draws it at (plan 2026-09-18, K2).
    /// A symbol that is not parametric has the extent row alone (K-D3).
    #[test]
    fn a_placed_symbol_shows_its_library_defaults_and_its_extent_as_two_rows() {
        let manifold = pid_semantics_section(&entity(&[
            "sheet_layer=Default",
            "role=symbol",
            "style=Equipment - New",
            "extent=172.21x71.18",
            "driving=Top:20.32;Left:114.30;Right:114.30",
            "class=PIDProcessVessel",
            "label=V-101",
        ]))
        .expect("P&ID section");
        assert_eq!(
            value(&manifold, "pid_driving"),
            "Top 20.32 mm · Left 114.30 mm · Right 114.30 mm"
        );
        assert_eq!(value(&manifold, "pid_extent"), "172.21 × 71.18 mm");
        let fields: Vec<&str> = manifold.props.iter().map(|p| p.field).collect();
        assert_eq!(
            fields[..4],
            ["pid_class", "pid_role", "pid_driving", "pid_extent"]
        );
        assert!(manifold
            .props
            .iter()
            .find(|property| property.field == "pid_driving")
            .is_some_and(|property| {
                property.label == t!("Driving dimensions (library default)")
            }));
        assert!(manifold
            .props
            .iter()
            .find(|property| property.field == "pid_extent")
            .is_some_and(|property| property.label == t!("Body extent")));
        assert!(
            manifold.props.iter().all(|p| p.field != "pid_instance"),
            "no instance= pair, no instance row"
        );

        // With the instance's own parameters (plan 2026-09-20): a third
        // row between the library default and the extent, captioned as this
        // drawing's instance.
        let stretched = pid_semantics_section(&entity(&[
            "role=symbol",
            "extent=172.21x71.18",
            "driving=Top:20.32;Left:114.30;Right:114.30",
            "instance=Left:57.91;Right:114.30;Top:35.59",
        ]))
        .expect("P&ID section");
        assert_eq!(
            value(&stretched, "pid_instance"),
            "Left 57.91 mm · Right 114.30 mm · Top 35.59 mm"
        );
        let fields: Vec<&str> = stretched.props.iter().map(|p| p.field).collect();
        assert_eq!(
            fields[..4],
            ["pid_role", "pid_driving", "pid_instance", "pid_extent"]
        );
        assert!(stretched
            .props
            .iter()
            .find(|property| property.field == "pid_instance")
            .is_some_and(|property| {
                property.label == t!("Driving dimensions (this drawing's instance)")
            }));

        let valve = pid_semantics_section(&entity(&["role=symbol", "extent=12.70x8.00"]))
            .expect("P&ID section");
        assert_eq!(value(&valve, "pid_extent"), "12.70 × 8.00 mm");
        assert!(valve
            .props
            .iter()
            .all(|p| p.field != "pid_driving" && p.field != "pid_instance"));

        // Whatever the import wrote is shown rather than dropped when it is
        // not in the shape the panel formats.
        assert_eq!(
            driving_dimensions_caption("Right:25.40;odd"),
            Some("Right 25.40 mm · odd".to_string())
        );
        assert_eq!(driving_dimensions_caption(""), None);
        assert_eq!(body_extent_caption("wide"), Some("wide".to_string()));
        assert_eq!(body_extent_caption(" "), None);
    }

    /// The block reference of a placed symbol names the symbol it places
    /// (`symbol=`; plan 2026-11-19-pid-symbol-block, SB-6): one read-only
    /// row right after the role, wherever the key stands in the record, and
    /// the rest of the group is what it is without the key.
    #[test]
    fn a_symbol_reference_names_its_symbol_right_after_its_role() {
        let said = [
            "sheet_layer=Default",
            "sheet_layer_oid=42",
            "role=symbol",
            "style=Equipment - New",
            "extent=172.21x71.18",
            "driving=Top:20.32;Left:114.30;Right:114.30",
            "instance=Left:57.91;Right:114.30;Top:35.59",
            "class=PIDProcessVessel",
            "label=V-101",
            "oid=77",
            "resolved=direct",
        ];
        let unnamed = pid_semantics_section(&entity(&said)).expect("P&ID section");
        assert!(unnamed.props.iter().all(|p| p.field != "pid_symbol"));

        // The import writes the key last.
        let mut named = said.to_vec();
        named.push("symbol=Parametric Manifold");
        let section = pid_semantics_section(&entity(&named)).expect("P&ID section");
        assert_eq!(value(&section, "pid_symbol"), "Parametric Manifold");
        let fields: Vec<&str> = section.props.iter().map(|p| p.field).collect();
        assert_eq!(
            fields,
            [
                "pid_class",
                "pid_role",
                "pid_symbol",
                "pid_driving",
                "pid_instance",
                "pid_extent",
                "pid_label",
                "pid_resolved",
                "pid_sheet_layer",
                "pid_sheet_layer_oid",
            ]
        );
        assert!(section
            .props
            .iter()
            .find(|property| property.field == "pid_symbol")
            .is_some_and(|property| property.label == t!("Symbol")));
        let others: Vec<&Property> = section
            .props
            .iter()
            .filter(|property| property.field != "pid_symbol")
            .collect();
        assert_eq!(
            others,
            unnamed.props.iter().collect::<Vec<_>>(),
            "the rows beside the name"
        );
    }

    /// The name is shown as the record spells it: all that follows the first
    /// `=`, the spaces at either end included. A reference that says nothing
    /// else still names its symbol, and an empty name is no row.
    #[test]
    fn the_name_of_a_symbol_is_shown_as_the_record_spells_it() {
        let odd =
            pid_semantics_section(&entity(&["role=symbol", "symbol= 3\" Gate=Valve (蝶阀) "]))
                .expect("P&ID section");
        assert_eq!(value(&odd, "pid_symbol"), " 3\" Gate=Valve (蝶阀) ");

        let alone = pid_semantics_section(&entity(&["symbol=Ball Valve"])).expect("P&ID section");
        assert_eq!(value(&alone, "pid_symbol"), "Ball Valve");
        assert_eq!(alone.props.len(), 1);

        let nameless =
            pid_semantics_section(&entity(&["role=symbol", "symbol="])).expect("P&ID section");
        let fields: Vec<&str> = nameless.props.iter().map(|p| p.field).collect();
        assert_eq!(fields, ["pid_role"]);
    }

    /// What an import states of the item a symbol draws (plan
    /// 2026-11-20-pid-symbol-identity, SPEC section 4.5): the kind the
    /// drawing file holds the item as has a row of its own, right after the
    /// type; and where nothing published says what the item is, the type
    /// reads that kind too. A record that states no kind shows what it
    /// showed.
    #[test]
    fn an_item_shows_the_kind_the_drawing_file_holds_it_as() {
        let published = pid_semantics_section(&entity(&[
            "role=symbol",
            "class=PIDPipingComponent",
            "resolved=direct",
            "item_type=PipingComp",
        ]))
        .expect("P&ID section");
        assert_eq!(value(&published, "pid_class"), "PIDPipingComponent");
        assert_eq!(value(&published, "pid_item_type"), "PipingComp");
        assert_eq!(
            fields(&published),
            ["pid_class", "pid_item_type", "pid_role", "pid_resolved"]
        );
        assert_eq!(caption(&published, "pid_class"), t!("Type"));
        assert_eq!(caption(&published, "pid_item_type"), t!("Item type"));

        let unpublished = pid_semantics_section(&entity(&["role=symbol", "item_type=OPC"]))
            .expect("P&ID section");
        assert_eq!(value(&unpublished, "pid_class"), "OPC");
        assert_eq!(value(&unpublished, "pid_item_type"), "OPC");
        assert_eq!(
            fields(&unpublished),
            ["pid_class", "pid_item_type", "pid_role"]
        );
        assert_eq!(caption(&unpublished, "pid_class"), t!("Type"));

        // A class stated empty names nothing: the kind stands in for it.
        let blank =
            pid_semantics_section(&entity(&["class=", "item_type=OPC"])).expect("P&ID section");
        assert_eq!(value(&blank, "pid_class"), "OPC");

        // No kind, or an empty one: the rows there were, as they were.
        let unkind =
            pid_semantics_section(&entity(&["role=symbol", "item_type="])).expect("P&ID section");
        assert_eq!(fields(&unkind), ["pid_role"]);
        let empty = pid_semantics_section(&entity(&["class="])).expect("P&ID section");
        assert_eq!(fields(&empty), ["pid_class"]);
        assert_eq!(value(&empty, "pid_class"), "");
    }

    /// A tag says where it came from, in the row after it (SPEC section
    /// 4.5): the drawing file, or the plant database of a backup -- and
    /// then the name the drawing file gave the item follows, where the
    /// record states one. A tag the published data gave is marked with
    /// nothing and has no such row; a mark this version does not know is
    /// shown as written; and where there is no tag there is no source to
    /// show.
    #[test]
    fn a_tag_names_where_it_came_from_in_the_row_after_it() {
        let tagged = |more: &[&str]| {
            let mut said = vec!["role=symbol", "label=GV-101"];
            said.extend_from_slice(more);
            pid_semantics_section(&entity(&said)).expect("P&ID section")
        };

        let by_the_file = tagged(&["tag_source=file"]);
        assert_eq!(value(&by_the_file, "pid_label"), "GV-101");
        assert_eq!(caption(&by_the_file, "pid_label"), t!("Item tag"));
        assert_eq!(
            value(&by_the_file, "pid_label_source"),
            t!("Drawing file").as_ref()
        );
        assert_eq!(caption(&by_the_file, "pid_label_source"), t!("Tag source"));
        assert_eq!(
            fields(&by_the_file),
            ["pid_role", "pid_label", "pid_label_source"]
        );

        let by_the_database = tagged(&["tag_source=database", "label_in_file=GV-101-OLD"]);
        assert_eq!(
            value(&by_the_database, "pid_label_source"),
            t!("Backup database").as_ref()
        );
        assert_eq!(value(&by_the_database, "pid_label_in_file"), "GV-101-OLD");
        assert_eq!(
            caption(&by_the_database, "pid_label_in_file"),
            t!("Name in file")
        );
        assert_eq!(
            fields(&by_the_database),
            [
                "pid_role",
                "pid_label",
                "pid_label_source",
                "pid_label_in_file"
            ]
        );

        let published = tagged(&["class=PIDPipingComponent", "resolved=direct"]);
        assert_eq!(
            fields(&published),
            ["pid_class", "pid_role", "pid_label", "pid_resolved"]
        );

        assert_eq!(
            value(&tagged(&["tag_source=manual"]), "pid_label_source"),
            "manual"
        );
        assert_eq!(
            fields(&tagged(&["tag_source=", "label_in_file="])),
            ["pid_role", "pid_label"],
            "an empty mark and an empty name show nothing"
        );

        let untagged = pid_semantics_section(&entity(&["role=symbol", "tag_source=file"]))
            .expect("P&ID section");
        assert_eq!(fields(&untagged), ["pid_role"]);
    }

    /// An item nothing published resolves is matched by the drawing file's
    /// own statement of it, and the row says so in words (SPEC section
    /// 4.5). The other ways of matching read as the record spells them.
    #[test]
    fn an_item_the_drawing_file_states_says_so_in_words() {
        let matched_by = |how: &str| {
            let resolved = format!("resolved={how}");
            let section = pid_semantics_section(&entity(&["role=symbol", resolved.as_str()]))
                .expect("P&ID section");
            assert_eq!(caption(&section, "pid_resolved"), t!("Matched by"));
            value(&section, "pid_resolved").to_string()
        };
        assert_eq!(matched_by("item"), t!("Item in the drawing file"));
        assert_eq!(matched_by("direct"), "direct");
        assert_eq!(matched_by("dependency:77"), "dependency:77");
        assert_eq!(matched_by("items"), "items");
    }

    /// The block reference of an imported symbol lists the labels the sheet
    /// hangs on it (SPEC section 4.5): how many there are, and what the
    /// first three read; `+n` stands for the rest. The row comes before the
    /// one that says how the item was matched. A symbol with no label has
    /// no such row, and a label that names the number as a line's is not
    /// the symbol's.
    #[test]
    fn a_symbol_lists_the_labels_the_sheet_hangs_on_it() {
        let labelled = |count: usize| {
            let mut entities = vec![reference(&[
                "sheet_layer=Process",
                "role=symbol",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
            ])];
            for lettering in ["GV-101", " DN50 ", "PN16", "CS", "FO"]
                .into_iter()
                .take(count)
            {
                entities.push(text(
                    lettering,
                    &["role=text", "label_of=612", "label_on=symbol"],
                ));
            }
            entities.push(text(
                "L-1001",
                &["role=text", "label_of=612", "label_on=line"],
            ));
            let (doc, filed) = sheet(entities);
            on_sheet(&doc, filed[0]).expect("P&ID section")
        };

        let unlabelled = labelled(0);
        assert_eq!(
            fields(&unlabelled),
            ["pid_role", "pid_symbol", "pid_resolved", "pid_sheet_layer"]
        );

        let once = labelled(1);
        assert_eq!(value(&once, "pid_labels"), "1: GV-101");
        assert_eq!(caption(&once, "pid_labels"), t!("Labels"));
        assert_eq!(
            fields(&once),
            [
                "pid_role",
                "pid_symbol",
                "pid_labels",
                "pid_resolved",
                "pid_sheet_layer"
            ]
        );

        assert_eq!(value(&labelled(3), "pid_labels"), "3: GV-101 · DN50 · PN16");
        assert_eq!(
            value(&labelled(4), "pid_labels"),
            "4: GV-101 · DN50 · PN16 +1"
        );
        assert_eq!(
            value(&labelled(5), "pid_labels"),
            "5: GV-101 · DN50 · PN16 +2"
        );
    }

    /// The text of a label names what its label hangs on (SPEC section
    /// 4.5): the tag of its symbol; for a symbol with no tag, what kind of
    /// thing it is and the name of its symbol, as far as the reference
    /// states them; that it is a line, or another object; or that its
    /// symbol is not on the sheet. The row comes before the sheet layer's,
    /// or last where the group has none. A text that is no label's has no
    /// such row, and neither has a symbol.
    #[test]
    fn a_label_text_names_what_its_label_hangs_on() {
        let label = |graphic: &str, on: &str| {
            let of = format!("label_of={graphic}");
            let on = format!("label_on={on}");
            text(
                "LETTERING",
                &["sheet_layer=Process", "role=text", of.as_str(), on.as_str()],
            )
        };
        let (doc, filed) = sheet(vec![
            reference(&[
                "role=symbol",
                "class=PIDPipingComponent",
                "label=GV-101",
                "symbol=Ball Valve",
                "graphic=612",
                "item_type=PipingComp",
            ]),
            reference(&[
                "role=symbol",
                "class=PIDPipingComponent",
                "symbol=Ball Valve",
                "graphic=613",
                "item_type=PipingComp",
            ]),
            reference(&["role=symbol", "symbol=Ball Valve", "graphic=614"]),
            reference(&["role=symbol", "symbol=", "graphic=615", "item_type=OPC"]),
            reference(&["role=symbol", "graphic=616"]),
            label("612", "symbol"),
            label("613", "symbol"),
            label("614", "symbol"),
            label("615", "symbol"),
            label("616", "symbol"),
            label("700", "symbol"),
            label("612", "line"),
            label("6", "other"),
            text("NOTE 4", &["sheet_layer=Process", "role=text"]),
            text("BARE", &["role=text", "label_of=612", "label_on=symbol"]),
        ]);
        let hangs_on = |at: usize| {
            on_sheet(&doc, filed[at]).and_then(|section| reads(&section, "pid_label_of"))
        };

        assert_eq!(hangs_on(5).as_deref(), Some("GV-101"));
        assert_eq!(
            hangs_on(6).as_deref(),
            Some("PIDPipingComponent · Ball Valve")
        );
        assert_eq!(hangs_on(7).as_deref(), Some("Ball Valve"));
        assert_eq!(hangs_on(8).as_deref(), Some("OPC"));
        assert_eq!(hangs_on(9), Some(t!("Symbol").into_owned()));
        assert_eq!(hangs_on(10), Some(t!("Not on the sheet").into_owned()));
        assert_eq!(hangs_on(11), Some(t!("A line").into_owned()));
        assert_eq!(hangs_on(12), Some(t!("Another object").into_owned()));
        assert_eq!(hangs_on(13), None, "no label's");
        assert_eq!(hangs_on(0), None, "a symbol");

        let on_a_layer = on_sheet(&doc, filed[5]).expect("P&ID section");
        assert_eq!(
            fields(&on_a_layer),
            ["pid_role", "pid_label_of", "pid_sheet_layer"]
        );
        assert_eq!(caption(&on_a_layer, "pid_label_of"), t!("Label of"));
        let on_none = on_sheet(&doc, filed[14]).expect("P&ID section");
        assert_eq!(fields(&on_none), ["pid_role", "pid_label_of"]);
        assert_eq!(reads(&on_none, "pid_label_of").as_deref(), Some("GV-101"));
    }

    /// The rows of an imported symbol stand in one order (SPEC section
    /// 4.5): what it is and the kind the file holds it as; its role, its
    /// symbol and its extent; its tag, where the tag came from and the name
    /// the file gave; its labels; how it was matched; its sheet layer.
    #[test]
    fn the_rows_of_an_imported_symbol_stand_in_one_order() {
        let (doc, filed) = sheet(vec![
            text("GV-101A", &["role=text", "label_of=612", "label_on=symbol"]),
            reference(&[
                "sheet_layer=Process",
                "sheet_layer_oid=12",
                "role=symbol",
                "style=Equipment - New",
                "extent=4.00x3.00",
                "class=PIDPipingComponent",
                "label=GV-101A",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
                "item=4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                "item_type=PipingComp",
                "tag_source=database",
                "label_in_file=GV-101",
            ]),
        ]);
        let section = on_sheet(&doc, filed[1]).expect("P&ID section");
        assert_eq!(
            fields(&section),
            [
                "pid_class",
                "pid_item_type",
                "pid_role",
                "pid_symbol",
                "pid_extent",
                "pid_label",
                "pid_label_source",
                "pid_label_in_file",
                "pid_labels",
                "pid_resolved",
                "pid_sheet_layer",
                "pid_sheet_layer_oid",
            ]
        );
        assert_eq!(value(&section, "pid_labels"), "1: GV-101A");
    }

    /// The row that takes the sheet joins the `P&ID` group the entity
    /// already has, wherever that group stands among the others; where the
    /// entity states nothing else the panel shows, the row makes the group.
    /// An entity that pairs with nothing leaves the panel as it was.
    #[test]
    fn the_row_of_a_pairing_joins_the_pid_group_or_makes_it() {
        let (doc, filed) = sheet(vec![
            text("DWG-0201", &["label_of=6", "label_on=other"]),
            text("NOTE 4", &["role=text"]),
            text("P-1001", &["role=text", "label_of=704", "label_on=line"]),
        ]);
        let page = |at: usize| {
            let entity = doc.get_entity(filed[at]).expect("filed on the sheet");
            let mut sections = vec![general_section(entity)];
            sections.extend(pid_semantics_section(entity));
            sections.push(fallback_properties(filed[at], entity));
            add_pid_pairing_row(&mut sections, &doc, entity);
            sections
                .iter()
                .map(|section| (section.title == "P&ID").then(|| fields(section)))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            page(0),
            [None, None, Some(vec!["pid_label_of"])],
            "the group is made, after the others"
        );
        assert_eq!(
            page(1),
            [None, Some(vec!["pid_role"]), None],
            "no pairing, no row"
        );
        assert_eq!(
            page(2),
            [None, Some(vec!["pid_role", "pid_label_of"]), None],
            "the row joins the group where it stands"
        );
    }
}
