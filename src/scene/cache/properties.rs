use crate::t;
use acadrust::{EntityType, Handle};

use crate::scene::model::object::{PropSection, PropValue, Property};

pub fn general_section(entity: &EntityType) -> PropSection {
    let common = entity.common();
    let linetype_display = if common.linetype.is_empty() {
        "ByLayer".to_string()
    } else {
        common.linetype.clone()
    };
    // Alpha 0 is ByLayer, Alpha 1 is ByBlock; show them by name and fall back
    // to a rounded percentage for explicit values.
    let transp_display = match common.transparency.alpha() {
        0 => "ByLayer".to_string(),
        1 => "ByBlock".to_string(),
        alpha => format!(
            "{}",
            (alpha as f64 / 255.0 * 100.0).round() as u32
        ),
    };

    // Hyperlink is stored in XDATA under the "PE_URL" application.
    let hyperlink = common
        .extended_data
        .get_record("PE_URL")
        .and_then(|r| {
            r.values.iter().find_map(|v| match v {
                acadrust::xdata::XDataValue::String(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();

    let mut section = PropSection {
        title: t!("General").into_owned(),
        props: vec![
            Property {
                label: t!("Color").into_owned(),
                field: "color",
                value: PropValue::ColorChoice(common.color),
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

/// The "P&ID" group: the published identity a `.pid` import wrote into the
/// entity's XDATA (see `crate::io::PID_SEMANTICS_XDATA_APP`).
///
/// Present when either authored sheet-layer identity or published `_Data.xml`
/// identity reached the entity. Every row is read-only.
pub fn pid_semantics_section(entity: &EntityType) -> Option<PropSection> {
    let record = entity
        .common()
        .extended_data
        .get_record(crate::io::PID_SEMANTICS_XDATA_APP)?;

    let mut class = None;
    let mut label = None;
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
            "label" => label = Some(val.to_string()),
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
    if let Some(label_value) = label {
        // A pipe's identifier is its line number; everything else carries an
        // item tag. Same value, the caption the reader expects.
        let label_caption =
            if matches!(class.as_deref(), Some("PIDPipeline" | "PIDPipingConnector")) {
                t!("Line number")
            } else {
                t!("Item tag")
            };
        props.push(Property {
            label: label_caption.into_owned(),
            field: "pid_label",
            value: PropValue::ReadOnly(label_value),
        });
    }
    if let Some(resolved_value) = resolved {
        props.push(Property {
            label: t!("Matched by").into_owned(),
            field: "pid_resolved",
            value: PropValue::ReadOnly(resolved_value),
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
