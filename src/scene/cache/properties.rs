use crate::t;
use acadrust::{EntityType, Handle, Transparency};

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

    let hyperlink = crate::scene::pe_url_of(entity)
        .unwrap_or_default()
        .to_owned();

    let mut section = PropSection {
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
                value: PropValue::EditText(format!("{:.4}", common.linetype_scale)),
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
                    options: vec!["ByLayer".to_string(), "ByBlock".to_string()],
                },
            },
            Property {
                label: t!("Hyperlink").into_owned(),
                field: "hyperlink",
                value: PropValue::PlainText(hyperlink),
            },
        ],
    };

    // Thickness (DXF 39) is a General-group property, but only the entity
    // types that carry an extrusion thickness expose it (line, circle, arc,
    // polyline, text, 2D solid, …). Show it right after Hyperlink for those.
    if let Some(t) = crate::scene::view::dispatch::entity_thickness(entity) {
        section.props.push(crate::entities::common::edit_prop(
            t!("Thickness").as_ref(),
            "thickness",
            t,
        ));
    }

    section
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
    let mut role = None;
    let mut driving = None;
    let mut instance = None;
    let mut extent = None;
    let mut labels = Vec::new();
    let mut lines = Vec::new();
    let mut resolved = None;
    let mut sheet_layer = None;
    let mut sheet_layer_oid = None;
    for value in &record.values {
        let acadrust::xdata::XDataValue::String(text) = value else {
            continue;
        };
        let Some((key, val)) = text.split_once('=') else {
            continue;
        };
        match key {
            "class" => class = Some(val.to_string()),
            "role" => role = Some(val.to_string()),
            "driving" => driving = driving_dimensions_caption(val),
            "instance" => instance = driving_dimensions_caption(val),
            "extent" => extent = body_extent_caption(val),
            "label" if !val.is_empty() && !labels.iter().any(|old| old == val) => {
                labels.push(val.to_string());
            }
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
    if let Some(class) = class.as_deref() {
        props.push(Property {
            label: t!("Type").into_owned(),
            field: "pid_class",
            value: PropValue::ReadOnly(class.to_string()),
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
            | EntityType::Unknown(_)
            // Non-plotting drawing-view border: never rendered, no properties.
            | EntityType::ViewBorder(_)
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
    use acadrust::entities::Line;
    use acadrust::xdata::{ExtendedDataRecord, XDataValue};

    fn entity(values: &[&str]) -> EntityType {
        let mut entity = EntityType::Line(Line::new());
        let mut record = ExtendedDataRecord::new(crate::io::PID_SEMANTICS_XDATA_APP);
        for value in values {
            record.add_value(XDataValue::String((*value).to_string()));
        }
        entity.common_mut().extended_data.add_record(record);
        entity
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
}
