//! Format and structural validator for SmartPlant P&ID publish XML files
//! (`<Name>_Data.xml` and `<Name>_Meta.xml`), driven by
//! `assets/pid-publish-spec.json` (`L4-0` / `#111`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;

const EMBEDDED_SPEC_JSON: &str = include_str!("../../assets/pid-publish-spec.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishXmlKind {
    Data,
    Meta,
}

impl PublishXmlKind {
    pub fn infer_from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("_data.xml") {
            Some(Self::Data)
        } else if lower.ends_with("_meta.xml") {
            Some(Self::Meta)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckMode {
    /// Enforce both SPPID byte/formatting envelope (UTF-8 BOM, CRLF, 3-space
    /// indent, `version ="1.0"` declaration, attribute order) and XML schema.
    #[default]
    StrictVendor,
    /// Enforce XML structural and semantic rules only (suitable for internal
    /// `pid-parse` outputs that omit BOM and use 2-space LF formatting).
    SchemaOnly,
}

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub mode: CheckMode,
    /// When `Some(plant)`, `<Container Plant="...">` must equal `plant`.
    /// When `None`, `Plant` must simply be present and non-empty.
    pub expected_plant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub file: String,
    pub code: &'static str,
    pub location: String,
    pub expected: String,
    pub actual: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} [{}]: expected {}, got {}",
            self.file, self.location, self.code, self.expected, self.actual
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishSpec {
    pub schema_version: u32,
    pub format: FormatSpec,
    pub data_container: DataContainerSpec,
    pub meta_container: MetaContainerSpec,
    pub uid_rules: BTreeMap<String, String>,
    pub data_classes: BTreeMap<String, ClassSpec>,
    pub data_relations: BTreeMap<String, DataRelationSpec>,
    pub meta_classes: BTreeMap<String, ClassSpec>,
    pub meta_relations: BTreeMap<String, MetaRelationSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormatSpec {
    pub utf8_bom: bool,
    pub line_ending: String,
    pub trailing_newline: bool,
    pub xml_declaration: String,
    pub indent: IndentSpec,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndentSpec {
    pub open_container: usize,
    pub open_object: usize,
    pub interface_element: usize,
    pub close_object: usize,
    pub close_container: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataContainerSpec {
    pub attribute_order: Vec<String>,
    pub fixed_attributes: BTreeMap<String, String>,
    pub default_plant: String,
    pub first_element: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetaContainerSpec {
    pub attribute_order: Vec<String>,
    pub fixed_attributes: BTreeMap<String, String>,
    pub default_plant: String,
    pub element_sequence: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassSpec {
    pub uid_kind: String,
    pub required_interfaces: Vec<String>,
    pub allowed_interfaces: Vec<String>,
    pub interface_attributes: BTreeMap<String, InterfaceAttrSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterfaceAttrSpec {
    pub required: Vec<String>,
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataRelationSpec {
    pub prefix: String,
    pub allowed_endpoints: Vec<[String; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetaRelationSpec {
    pub uid1_target: String,
    pub uid2_target: String,
    pub rel_uid_kind: String,
}

pub fn default_publish_spec() -> &'static PublishSpec {
    static SPEC: OnceLock<PublishSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        serde_json::from_str(EMBEDDED_SPEC_JSON)
            .expect("embedded assets/pid-publish-spec.json must be valid")
    })
}

pub fn check_publish_xml_file(
    path: &Path,
    options: &CheckOptions,
) -> Result<Vec<Violation>, String> {
    let kind = PublishXmlKind::infer_from_path(path).ok_or_else(|| {
        format!(
            "cannot infer publish XML kind from filename '{}' (expected *_Data.xml or *_Meta.xml)",
            path.display()
        )
    })?;
    let bytes =
        std::fs::read(path).map_err(|err| format!("cannot read '{}': {err}", path.display()))?;
    Ok(check_publish_xml_bytes(
        &path.display().to_string(),
        kind,
        &bytes,
        default_publish_spec(),
        options,
    ))
}

fn is_upper_hex(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

fn matches_uid_kind(uid: &str, uid_kind: &str, strict: bool) -> bool {
    match uid_kind {
        "hex32" => uid.len() == 32 && is_upper_hex(uid),
        "hex32_dot_num" => {
            let Some((head, tail)) = uid.split_once('.') else {
                return false;
            };
            head.len() == 32
                && is_upper_hex(head)
                && !tail.is_empty()
                && !tail.starts_with('0')
                && tail.bytes().all(|b| b.is_ascii_digit())
        }
        "hex32_bpt" => uid
            .strip_suffix(".BPT")
            .is_some_and(|head| head.len() == 32 && is_upper_hex(head)),
        "hex32_ppt" => uid
            .strip_suffix(".PPT")
            .is_some_and(|head| head.len() == 32 && is_upper_hex(head)),
        "meta_hex" => (28..=32).contains(&uid.len()) && is_upper_hex(uid),
        "meta_file" => {
            let is_aaad = uid.strip_prefix("AAAD").is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b) || b == b'-')
            });
            if strict {
                is_aaad
            } else {
                is_aaad || ((28..=32).contains(&uid.len()) && is_upper_hex(uid))
            }
        }
        _ => false,
    }
}

fn uid_rule_description<'a>(spec: &'a PublishSpec, uid_kind: &'a str, strict: bool) -> &'a str {
    let key = if uid_kind == "meta_file" {
        if strict {
            "meta_file_strict"
        } else {
            "meta_file_schema"
        }
    } else {
        uid_kind
    };
    spec.uid_rules.get(key).map_or(uid_kind, String::as_str)
}

fn find_line_ending_issue(bytes: &[u8]) -> Option<(usize, &'static str)> {
    let mut line_no = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    line_no += 1;
                    i += 2;
                } else {
                    return Some((line_no, "bare CR (\\r)"));
                }
            }
            b'\n' => return Some((line_no, "bare LF (\\n)")),
            _ => i += 1,
        }
    }
    None
}

fn node_loc(doc: &roxmltree::Document<'_>, node: roxmltree::Node<'_, '_>) -> String {
    let pos = doc.text_pos_at(node.range().start);
    format!(
        "line {}, col {} (<{}>)",
        pos.row,
        pos.col,
        node.tag_name().name()
    )
}

pub fn check_publish_xml_bytes(
    file_label: &str,
    kind: PublishXmlKind,
    bytes: &[u8],
    spec: &PublishSpec,
    options: &CheckOptions,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let strict = options.mode == CheckMode::StrictVendor;

    let has_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    if strict && spec.format.utf8_bom && !has_bom {
        let preview = if bytes.is_empty() {
            "<empty>".to_string()
        } else {
            bytes
                .iter()
                .take(3)
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        violations.push(Violation {
            file: file_label.to_string(),
            code: "MISSING_UTF8_BOM",
            location: "byte 0".to_string(),
            expected: "UTF-8 BOM (EF BB BF)".to_string(),
            actual: format!("leading bytes {preview}"),
        });
    }

    let body = if has_bom { &bytes[3..] } else { bytes };
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(err) => {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "INVALID_UTF8",
                location: format!("byte {}", err.valid_up_to()),
                expected: "valid UTF-8 byte sequence".to_string(),
                actual: err.to_string(),
            });
            return violations;
        }
    };

    if strict {
        if let Some((line_no, found)) = find_line_ending_issue(body) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "LINE_ENDING_NOT_CRLF",
                location: format!("line {line_no}"),
                expected: "CRLF (\\r\\n)".to_string(),
                actual: found.to_string(),
            });
        }

        if spec.format.trailing_newline && !body.ends_with(b"\r\n") {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "MISSING_TRAILING_NEWLINE",
                location: "EOF".to_string(),
                expected: "trailing CRLF (\\r\\n)".to_string(),
                actual: "missing trailing CRLF".to_string(),
            });
        }

        let mut lines_iter = text.lines();
        let Some(first_line) = lines_iter.next() else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "EMPTY_DOCUMENT",
                location: "line 1".to_string(),
                expected: spec.format.xml_declaration.clone(),
                actual: "<empty>".to_string(),
            });
            return violations;
        };

        if first_line != spec.format.xml_declaration {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "XML_DECLARATION_MISMATCH",
                location: "line 1".to_string(),
                expected: spec.format.xml_declaration.clone(),
                actual: first_line.to_string(),
            });
        }

        for (idx, line) in lines_iter.enumerate() {
            let line_no = idx + 2;
            if line.is_empty() {
                continue;
            }
            let trimmed = line.trim_start_matches(' ');
            let spaces = line.len() - trimmed.len();
            if trimmed.starts_with('\t') || line[..spaces].contains('\t') {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "INDENTATION_MISMATCH",
                    location: format!("line {line_no}"),
                    expected: "spaces only (no tabs)".to_string(),
                    actual: "tab indentation".to_string(),
                });
                break;
            }
            let expected_indent = if trimmed.starts_with("<Container") {
                Some(spec.format.indent.open_container)
            } else if trimmed.starts_with("</Container>") {
                Some(spec.format.indent.close_container)
            } else if trimmed.starts_with("</") {
                Some(spec.format.indent.close_object)
            } else if trimmed.ends_with("/>") {
                Some(spec.format.indent.interface_element)
            } else if trimmed.starts_with('<') {
                Some(spec.format.indent.open_object)
            } else {
                None
            };
            if expected_indent.is_some_and(|exp| spaces != exp) {
                let exp = expected_indent.unwrap_or(0);
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "INDENTATION_MISMATCH",
                    location: format!("line {line_no}"),
                    expected: format!("{exp} leading space(s)"),
                    actual: format!("{spaces} leading space(s)"),
                });
                break;
            }
        }
    }

    // Preserve exact byte length so roxmltree line/col positions remain identical
    // even when line 1 uses SPPID's `<?xml version ="1.0"` space-before-equals form.
    let normalized_cow;
    let parse_input = if let Some(rest) = text.strip_prefix("<?xml version =") {
        normalized_cow = format!("<?xml  version={rest}");
        normalized_cow.as_str()
    } else {
        text
    };

    let doc = match roxmltree::Document::parse(parse_input) {
        Ok(d) => d,
        Err(err) => {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "XML_PARSE_ERROR",
                location: format!("line {}, col {}", err.pos().row, err.pos().col),
                expected: "well-formed XML 1.0 document".to_string(),
                actual: err.to_string(),
            });
            return violations;
        }
    };

    let root = doc.root_element();
    if root.tag_name().name() != "Container" {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "ROOT_NOT_CONTAINER",
            location: node_loc(&doc, root),
            expected: "<Container>".to_string(),
            actual: format!("<{}>", root.tag_name().name()),
        });
        return violations;
    }

    let (attr_order, fixed_attrs) = match kind {
        PublishXmlKind::Data => (
            &spec.data_container.attribute_order,
            &spec.data_container.fixed_attributes,
        ),
        PublishXmlKind::Meta => (
            &spec.meta_container.attribute_order,
            &spec.meta_container.fixed_attributes,
        ),
    };

    let expected_attr_set: BTreeSet<&str> = attr_order.iter().map(String::as_str).collect();
    let actual_attr_order: Vec<&str> = root.attributes().map(|a| a.name()).collect();
    let actual_attr_map: BTreeMap<&str, &str> =
        root.attributes().map(|a| (a.name(), a.value())).collect();

    for req_key in attr_order {
        if !actual_attr_map.contains_key(req_key.as_str()) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "CONTAINER_ATTR_MISSING",
                location: node_loc(&doc, root),
                expected: format!("attribute '{req_key}' present"),
                actual: "missing".to_string(),
            });
        }
    }

    for &act_key in &actual_attr_order {
        if !expected_attr_set.contains(act_key) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "CONTAINER_ATTR_UNEXPECTED",
                location: format!("{} @{act_key}", node_loc(&doc, root)),
                expected: "only declared Container attributes".to_string(),
                actual: format!("unexpected attribute '{act_key}'"),
            });
        }
    }

    if strict
        && actual_attr_order.len() == attr_order.len()
        && actual_attr_order
            .iter()
            .zip(attr_order.iter())
            .any(|(a, b)| *a != b.as_str())
    {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "CONTAINER_ATTR_ORDER",
            location: node_loc(&doc, root),
            expected: attr_order.join(","),
            actual: actual_attr_order.join(","),
        });
    }

    for (key, exp_val) in fixed_attrs {
        if let Some(&act_val) = actual_attr_map.get(key.as_str()) {
            if act_val != exp_val {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "CONTAINER_ATTR_MISMATCH",
                    location: format!("{} @{key}", node_loc(&doc, root)),
                    expected: format!("{exp_val:?}"),
                    actual: format!("{act_val:?}"),
                });
            }
        }
    }

    let plant_val = actual_attr_map.get("Plant").copied().unwrap_or("");
    if let Some(exp_plant) = &options.expected_plant {
        if plant_val != exp_plant {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "CONTAINER_ATTR_MISMATCH",
                location: format!("{} @Plant", node_loc(&doc, root)),
                expected: format!("{exp_plant:?}"),
                actual: format!("{plant_val:?}"),
            });
        }
    } else if plant_val.is_empty() {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "CONTAINER_PLANT_EMPTY",
            location: format!("{} @Plant", node_loc(&doc, root)),
            expected: "non-empty Plant name".to_string(),
            actual: "\"\"".to_string(),
        });
    }

    let doc_uid = actual_attr_map.get("DocUID").copied().unwrap_or("");
    if !matches_uid_kind(doc_uid, "hex32", strict) {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "INVALID_DOC_UID",
            location: format!("{} @DocUID", node_loc(&doc, root)),
            expected: format!(
                "32-char uppercase hex ({})",
                uid_rule_description(spec, "hex32", strict)
            ),
            actual: format!("{doc_uid:?}"),
        });
    }

    let elem_children: Vec<roxmltree::Node<'_, '_>> =
        root.children().filter(|n| n.is_element()).collect();

    match kind {
        PublishXmlKind::Data => {
            check_data_elements(
                file_label,
                &doc,
                &elem_children,
                doc_uid,
                spec,
                strict,
                &mut violations,
            );
        }
        PublishXmlKind::Meta => {
            check_meta_elements(
                file_label,
                &doc,
                root,
                &elem_children,
                doc_uid,
                spec,
                strict,
                &mut violations,
            );
        }
    }

    violations
}

fn check_class_element<'a>(
    file_label: &str,
    doc: &roxmltree::Document<'a>,
    elem: roxmltree::Node<'a, 'a>,
    cls_spec: &ClassSpec,
    spec: &PublishSpec,
    strict: bool,
    violations: &mut Vec<Violation>,
) -> Option<&'a str> {
    let sub_elems: Vec<roxmltree::Node<'a, 'a>> =
        elem.children().filter(|n| n.is_element()).collect();
    let loc = node_loc(doc, elem);

    let Some(&first_sub) = sub_elems.first() else {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "MISSING_IOBJECT",
            location: loc,
            expected: "first child <IObject UID=\"...\"/>".to_string(),
            actual: "empty element".to_string(),
        });
        return None;
    };

    if first_sub.tag_name().name() != "IObject" {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "FIRST_INTERFACE_NOT_IOBJECT",
            location: node_loc(doc, first_sub),
            expected: "first child <IObject>".to_string(),
            actual: format!("<{}>", first_sub.tag_name().name()),
        });
    }

    let Some(iobj) = sub_elems
        .iter()
        .copied()
        .find(|n| n.tag_name().name() == "IObject")
    else {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "MISSING_IOBJECT",
            location: loc,
            expected: "<IObject UID=\"...\"/>".to_string(),
            actual: "missing <IObject>".to_string(),
        });
        return None;
    };

    let uid = iobj.attribute("UID").unwrap_or("");
    if !matches_uid_kind(uid, &cls_spec.uid_kind, strict) {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "INVALID_OBJECT_UID",
            location: format!("{} @UID", node_loc(doc, iobj)),
            expected: format!(
                "{} ({})",
                cls_spec.uid_kind,
                uid_rule_description(spec, &cls_spec.uid_kind, strict)
            ),
            actual: format!("{uid:?}"),
        });
    }

    let present_ifaces: BTreeSet<&str> = sub_elems.iter().map(|n| n.tag_name().name()).collect();
    for req_iface in &cls_spec.required_interfaces {
        if !present_ifaces.contains(req_iface.as_str()) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "MISSING_REQUIRED_INTERFACE",
                location: format!("{loc} (UID={uid})"),
                expected: format!("required interface <{req_iface}>"),
                actual: "missing".to_string(),
            });
        }
    }

    let allowed_ifaces: BTreeSet<&str> = cls_spec
        .allowed_interfaces
        .iter()
        .map(String::as_str)
        .collect();
    for sub in sub_elems {
        let iface_name = sub.tag_name().name();
        if !allowed_ifaces.contains(iface_name) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "DISALLOWED_INTERFACE",
                location: node_loc(doc, sub),
                expected: format!("interface allowed on <{}>", elem.tag_name().name()),
                actual: format!("<{iface_name}>"),
            });
            continue;
        }
        let Some(attr_spec) = cls_spec.interface_attributes.get(iface_name) else {
            continue;
        };
        let sub_attrs: BTreeSet<&str> = sub.attributes().map(|a| a.name()).collect();
        let allowed_attrs: BTreeSet<&str> = attr_spec.allowed.iter().map(String::as_str).collect();
        for req_attr in &attr_spec.required {
            if !sub_attrs.contains(req_attr.as_str()) {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "MISSING_REQUIRED_ATTR",
                    location: node_loc(doc, sub),
                    expected: format!("required attribute '{req_attr}'"),
                    actual: "missing".to_string(),
                });
            }
        }
        for attr in sub.attributes() {
            if !allowed_attrs.contains(attr.name()) {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "DISALLOWED_ATTR",
                    location: format!("{} @{}", node_loc(doc, sub), attr.name()),
                    expected: format!(
                        "attribute allowed on <{}>/<{iface_name}>",
                        elem.tag_name().name()
                    ),
                    actual: format!("{:?}", attr.name()),
                });
            }
        }
    }

    Some(uid)
}

fn check_data_elements<'a>(
    file_label: &str,
    doc: &roxmltree::Document<'a>,
    children: &[roxmltree::Node<'a, 'a>],
    doc_uid: &str,
    spec: &PublishSpec,
    strict: bool,
    violations: &mut Vec<Violation>,
) {
    let Some(&first) = children.first() else {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "EMPTY_CONTAINER",
            location: "Container".to_string(),
            expected: format!("first element <{}>", spec.data_container.first_element),
            actual: "0 child elements".to_string(),
        });
        return;
    };

    if first.tag_name().name() != spec.data_container.first_element {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "FIRST_ELEMENT_NOT_PID_DRAWING",
            location: node_loc(doc, first),
            expected: format!("<{}>", spec.data_container.first_element),
            actual: format!("<{}>", first.tag_name().name()),
        });
    } else {
        let first_uid = first
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "IObject")
            .and_then(|n| n.attribute("UID"))
            .unwrap_or("");
        if first_uid != doc_uid {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "DRAWING_UID_MISMATCH",
                location: format!("{} @UID", node_loc(doc, first)),
                expected: format!("DocUID {doc_uid:?}"),
                actual: format!("{first_uid:?}"),
            });
        }
    }

    let mut uid_to_class: BTreeMap<&'a str, &'a str> = BTreeMap::new();
    let mut rels: Vec<roxmltree::Node<'a, 'a>> = Vec::new();

    for &elem in children {
        let tag = elem.tag_name().name();
        if tag == "Rel" {
            rels.push(elem);
            continue;
        }
        let Some(cls_spec) = spec.data_classes.get(tag) else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "UNKNOWN_OBJECT_CLASS",
                location: node_loc(doc, elem),
                expected: "one of the 21 declared PID object classes or <Rel>".to_string(),
                actual: format!("<{tag}>"),
            });
            continue;
        };
        if let Some(uid) =
            check_class_element(file_label, doc, elem, cls_spec, spec, strict, violations)
        {
            if !uid.is_empty() && uid_to_class.insert(uid, tag).is_some() {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "DUPLICATE_OBJECT_UID",
                    location: node_loc(doc, elem),
                    expected: "unique object UID within document".to_string(),
                    actual: format!("{uid:?}"),
                });
            }
        }
    }

    for rel in rels {
        let sub: Vec<roxmltree::Node<'a, 'a>> = rel.children().filter(|n| n.is_element()).collect();
        if sub.len() != 2
            || sub[0].tag_name().name() != "IObject"
            || sub[1].tag_name().name() != "IRel"
        {
            let actual_tags = sub
                .iter()
                .map(|n| format!("<{}>", n.tag_name().name()))
                .collect::<Vec<_>>()
                .join(", ");
            violations.push(Violation {
                file: file_label.to_string(),
                code: "MISSING_IREL",
                location: node_loc(doc, rel),
                expected: "[<IObject>, <IRel>]".to_string(),
                actual: format!("[{actual_tags}]"),
            });
            continue;
        }
        let iobj = sub[0];
        let irel = sub[1];
        for req_k in ["UID1", "UID2", "DefUID"] {
            if irel.attribute(req_k).is_none() {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "MISSING_REQUIRED_ATTR",
                    location: node_loc(doc, irel),
                    expected: format!("required attribute '{req_k}'"),
                    actual: "missing".to_string(),
                });
            }
        }
        for attr in irel.attributes() {
            if !matches!(attr.name(), "UID1" | "UID2" | "DefUID" | "OrderValue") {
                violations.push(Violation {
                    file: file_label.to_string(),
                    code: "DISALLOWED_ATTR",
                    location: format!("{} @{}", node_loc(doc, irel), attr.name()),
                    expected: "UID1, UID2, DefUID, or OrderValue".to_string(),
                    actual: format!("{:?}", attr.name()),
                });
            }
        }

        let rel_uid = iobj.attribute("UID").unwrap_or("");
        let u1 = irel.attribute("UID1").unwrap_or("");
        let u2 = irel.attribute("UID2").unwrap_or("");
        let def_uid = irel.attribute("DefUID").unwrap_or("");

        let Some(rel_spec) = spec.data_relations.get(def_uid) else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "UNKNOWN_REL_TYPE",
                location: format!("{} @DefUID", node_loc(doc, irel)),
                expected: "one of the 15 declared DefUID relation types".to_string(),
                actual: format!("{def_uid:?}"),
            });
            continue;
        };

        let expected_rel_uid = format!("{}-{u1}-{u2}", rel_spec.prefix);
        if rel_uid != expected_rel_uid {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "INVALID_REL_UID_PREFIX",
                location: format!("{} @UID", node_loc(doc, iobj)),
                expected: format!("{expected_rel_uid:?}"),
                actual: format!("{rel_uid:?}"),
            });
        }

        let c1 = uid_to_class.get(u1).copied();
        let c2 = uid_to_class.get(u2).copied();
        let (Some(class1), Some(class2)) = (c1, c2) else {
            let missing_side = if c1.is_none() {
                format!("UID1={u1:?}")
            } else {
                format!("UID2={u2:?}")
            };
            violations.push(Violation {
                file: file_label.to_string(),
                code: "DANGLING_REL_ENDPOINT",
                location: node_loc(doc, irel),
                expected: "both UID1 and UID2 defined in document".to_string(),
                actual: format!("unresolved {missing_side}"),
            });
            continue;
        };

        let allowed = rel_spec
            .allowed_endpoints
            .iter()
            .any(|pair| pair[0] == class1 && pair[1] == class2);
        if !allowed {
            let allowed_desc = rel_spec
                .allowed_endpoints
                .iter()
                .map(|p| format!("{} -> {}", p[0], p[1]))
                .collect::<Vec<_>>()
                .join(", ");
            violations.push(Violation {
                file: file_label.to_string(),
                code: "DISALLOWED_REL_ENDPOINTS",
                location: format!("{} ({def_uid})", node_loc(doc, irel)),
                expected: format!("one of [{allowed_desc}]"),
                actual: format!("{class1} -> {class2}"),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_meta_elements<'a>(
    file_label: &str,
    doc: &roxmltree::Document<'a>,
    root: roxmltree::Node<'a, 'a>,
    children: &[roxmltree::Node<'a, 'a>],
    doc_uid: &'a str,
    spec: &PublishSpec,
    strict: bool,
    violations: &mut Vec<Violation>,
) {
    let actual_seq: Vec<&str> = children.iter().map(|n| n.tag_name().name()).collect();
    let expected_seq = &spec.meta_container.element_sequence;
    if actual_seq.len() != expected_seq.len()
        || actual_seq
            .iter()
            .zip(expected_seq.iter())
            .any(|(a, b)| *a != b.as_str())
    {
        violations.push(Violation {
            file: file_label.to_string(),
            code: "META_SEQUENCE_MISMATCH",
            location: node_loc(doc, root),
            expected: format!("{expected_seq:?}"),
            actual: format!("{actual_seq:?}"),
        });
    }

    let mut uid_by_class: BTreeMap<&str, &str> = BTreeMap::new();
    uid_by_class.insert("DocUID", doc_uid);
    let mut rels: Vec<roxmltree::Node<'a, 'a>> = Vec::new();

    for &elem in children {
        let tag = elem.tag_name().name();
        if tag == "Rel" {
            rels.push(elem);
            continue;
        }
        let Some(cls_spec) = spec.meta_classes.get(tag) else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "UNKNOWN_OBJECT_CLASS",
                location: node_loc(doc, elem),
                expected: "DocumentVersion, DocumentRevision, File, or Rel".to_string(),
                actual: format!("<{tag}>"),
            });
            continue;
        };
        if let Some(uid) =
            check_class_element(file_label, doc, elem, cls_spec, spec, strict, violations)
        {
            if !uid.is_empty() {
                uid_by_class.insert(tag, uid);
            }
        }
    }

    for rel in rels {
        let sub: Vec<roxmltree::Node<'a, 'a>> = rel.children().filter(|n| n.is_element()).collect();
        let iobj = sub
            .iter()
            .copied()
            .find(|n| n.tag_name().name() == "IObject");
        let irel = sub.iter().copied().find(|n| n.tag_name().name() == "IRel");
        let (Some(iobj), Some(irel)) = (iobj, irel) else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "MISSING_IREL",
                location: node_loc(doc, rel),
                expected: "both <IObject> and <IRel>".to_string(),
                actual: "missing child".to_string(),
            });
            continue;
        };

        let def_uid = irel.attribute("DefUID").unwrap_or("");
        let Some(rel_spec) = spec.meta_relations.get(def_uid) else {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "UNKNOWN_REL_TYPE",
                location: format!("{} @DefUID", node_loc(doc, irel)),
                expected: "VersionedDoc, RevisedDocument, or FileComposition".to_string(),
                actual: format!("{def_uid:?}"),
            });
            continue;
        };

        let rel_uid = iobj.attribute("UID").unwrap_or("");
        if !matches_uid_kind(rel_uid, &rel_spec.rel_uid_kind, strict) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "INVALID_OBJECT_UID",
                location: format!("{} @UID", node_loc(doc, iobj)),
                expected: format!(
                    "{} ({})",
                    rel_spec.rel_uid_kind,
                    uid_rule_description(spec, &rel_spec.rel_uid_kind, strict)
                ),
                actual: format!("{rel_uid:?}"),
            });
        }

        let exp_u1 = uid_by_class
            .get(rel_spec.uid1_target.as_str())
            .copied()
            .unwrap_or("");
        let exp_u2 = uid_by_class
            .get(rel_spec.uid2_target.as_str())
            .copied()
            .unwrap_or("");
        let act_u1 = irel.attribute("UID1").unwrap_or("");
        let act_u2 = irel.attribute("UID2").unwrap_or("");
        if !exp_u1.is_empty() && !exp_u2.is_empty() && (act_u1 != exp_u1 || act_u2 != exp_u2) {
            violations.push(Violation {
                file: file_label.to_string(),
                code: "META_REL_TARGET_MISMATCH",
                location: format!("{} ({def_uid})", node_loc(doc, irel)),
                expected: format!(
                    "UID1={exp_u1:?} ({}), UID2={exp_u2:?} ({})",
                    rel_spec.uid1_target, rel_spec.uid2_target
                ),
                actual: format!("UID1={act_u1:?}, UID2={act_u2:?}"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC_UID: &str = "0102030405060708090A0B0C0D0E0F10";
    const REP_UID: &str = "1112131415161718191A1B1C1D1E1F20";
    const NOTE_UID: &str = "2122232425262728292A2B2C2D2E2F30";
    const VESSEL_UID: &str = "3132333435363738393A3B3C3D3E3F40";
    const NOZZLE_UID: &str = "4142434445464748494A4B4C4D4E4F50";
    const PORT_UID: &str = "4142434445464748494A4B4C4D4E4F50.1";
    const CONN_UID: &str = "5152535455565758595A5B5C5D5E5F60";
    const PPT_UID: &str = "5152535455565758595A5B5C5D5E5F60.PPT";
    const BPT_UID: &str = "6162636465666768696A6B6C6D6E6F70.BPT";

    fn with_bom_crlf(lines: &[&str]) -> Vec<u8> {
        let mut out = vec![0xEF, 0xBB, 0xBF];
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out
    }

    fn valid_synthetic_data_xml() -> Vec<u8> {
        with_bom_crlf(&[
            r#"<?xml version ="1.0" encoding="UTF-8"?>"#,
            &format!(
                r#"<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">"#
            ),
            "   <PIDDrawing>",
            &format!(r#"      <IObject UID="{DOC_UID}" Name="SYNTH-001" Description="" />"#),
            r#"      <IDocVersionComposition />"#,
            r#"      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="SYNTH" DocType="P&amp;ID" />"#,
            r#"      <IDwgRepresentationComposition />"#,
            r#"      <IPBSItem />"#,
            r#"      <IPIDDrawing />"#,
            r#"      <ISchematicDwg />"#,
            "   </PIDDrawing>",
            "   <PIDRepresentation>",
            &format!(r#"      <IObject UID="{REP_UID}" />"#),
            r#"      <IDrawingRepresentation GraphicOID="101" />"#,
            "   </PIDRepresentation>",
            "   <PIDNote>",
            &format!(r#"      <IObject UID="{NOTE_UID}" />"#),
            r#"      <IDocumentItem />"#,
            r#"      <IDrawingItem />"#,
            r#"      <INote />"#,
            r#"      <IPBSNote />"#,
            "   </PIDNote>",
            "   <Rel>",
            &format!(r#"      <IObject UID="DRC-{DOC_UID}-{REP_UID}" />"#),
            &format!(
                r#"      <IRel UID1="{DOC_UID}" UID2="{REP_UID}" DefUID="DwgRepresentationComposition" />"#
            ),
            "   </Rel>",
            "   <Rel>",
            &format!(r#"      <IObject UID="DRI-{REP_UID}-{NOTE_UID}" />"#),
            &format!(r#"      <IRel UID1="{REP_UID}" UID2="{NOTE_UID}" DefUID="DrawingItems" />"#),
            "   </Rel>",
            "  </Container>",
        ])
    }

    fn valid_synthetic_meta_xml() -> Vec<u8> {
        let dv = "A1A2A3A4A5A6A7A8A9AAABACADAEAF0";
        let dr = "B1B2B3B4B5B6B7B8B9BABBBCBDBEBF00";
        let file = "AAAD12345678-9ABC-4DEF-8000-112233445566";
        with_bom_crlf(&[
            r#"<?xml version ="1.0" encoding="UTF-8"?>"#,
            &format!(
                r#"<Container CompSchema="DocVersioningComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">"#
            ),
            "   <DocumentVersion>",
            &format!(r#"      <IObject UID="{dv}" Name="SYNTH-001" />"#),
            r#"      <IDocumentVersion DocRevision="0" DocVersion="1" DocVersionDate="2026/11/20" />"#,
            r#"      <IFileComposition />"#,
            "   </DocumentVersion>",
            "   <Rel>",
            r#"      <IObject UID="C1C2C3C4C5C6C7C8C9CACBCCCDCECF0" />"#,
            &format!(r#"      <IRel UID1="{DOC_UID}" UID2="{dv}" DefUID="VersionedDoc" />"#),
            "   </Rel>",
            "   <DocumentRevision>",
            &format!(r#"      <IObject UID="{dr}" Name="SYNTH-001" />"#),
            r#"      <IDocumentRevision MajorRev_ForRevise="0" MinorRev_ForRevise="" />"#,
            "   </DocumentRevision>",
            "   <Rel>",
            r#"      <IObject UID="D1D2D3D4D5D6D7D8D9DADBDCDDDEDF00" />"#,
            &format!(r#"      <IRel UID1="{dr}" UID2="{DOC_UID}" DefUID="RevisedDocument" />"#),
            "   </Rel>",
            "   <File>",
            &format!(r#"      <IObject UID="{file}" Name="SYNTH-001_Data.xml" Description="" />"#),
            r#"      <IFile FilePath="out" />"#,
            "   </File>",
            "   <Rel>",
            r#"      <IObject UID="AAAD87654321-9ABC-4DEF-8000-665544332211" />"#,
            &format!(r#"      <IRel UID1="{file}" UID2="{dv}" DefUID="FileComposition" />"#),
            "   </Rel>",
            "  </Container>",
        ])
    }

    #[test]
    fn valid_synthetic_data_and_meta_have_zero_violations() {
        let spec = default_publish_spec();
        let opts = CheckOptions::default();
        let v_data = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &valid_synthetic_data_xml(),
            spec,
            &opts,
        );
        assert!(v_data.is_empty(), "unexpected data violations: {v_data:?}");

        let v_meta = check_publish_xml_bytes(
            "SYNTH_Meta.xml",
            PublishXmlKind::Meta,
            &valid_synthetic_meta_xml(),
            spec,
            &opts,
        );
        assert!(v_meta.is_empty(), "unexpected meta violations: {v_meta:?}");
    }

    #[test]
    fn mutation_1_missing_bom_reports_file_location_expected_actual() {
        let raw = &valid_synthetic_data_xml()[3..];
        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "MISSING_UTF8_BOM");
        assert_eq!(v[0].file, "SYNTH_Data.xml");
        assert!(!v[0].location.is_empty());
        assert!(v[0].expected.contains("EF BB BF"));
        assert!(!v[0].actual.is_empty());
    }

    #[test]
    fn mutation_2_lf_line_ending_reports_violation() {
        let crlf = valid_synthetic_data_xml();
        let mut lf = Vec::with_capacity(crlf.len());
        let mut i = 0;
        while i < crlf.len() {
            if i + 1 < crlf.len() && crlf[i] == b'\r' && crlf[i + 1] == b'\n' {
                lf.push(b'\n');
                i += 2;
            } else {
                lf.push(crlf[i]);
                i += 1;
            }
        }
        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &lf,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert!(v.iter().any(|x| x.code == "LINE_ENDING_NOT_CRLF"), "{v:?}");
    }

    #[test]
    fn mutation_3_invalid_uid_syntax_reports_violation() {
        let text = String::from_utf8(valid_synthetic_data_xml()[3..].to_vec()).unwrap();
        let bad_uid = "2122232425262728292a2b2c2d2e2f30";
        let mutated = text.replace(NOTE_UID, bad_uid);
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(mutated.as_bytes());

        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert!(
            v.iter()
                .any(|x| x.code == "INVALID_OBJECT_UID" && x.actual.contains(bad_uid)),
            "{v:?}"
        );
    }

    #[test]
    fn mutation_4_wrong_relation_prefix_reports_violation() {
        let text = String::from_utf8(valid_synthetic_data_xml()[3..].to_vec()).unwrap();
        let mutated = text.replace(
            &format!("DRC-{DOC_UID}-{REP_UID}"),
            &format!("DRI-{DOC_UID}-{REP_UID}"),
        );
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(mutated.as_bytes());

        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "INVALID_REL_UID_PREFIX");
        assert!(v[0].expected.contains(&format!("DRC-{DOC_UID}-{REP_UID}")));
    }

    #[test]
    fn mutation_5_disallowed_relation_endpoints_report_violation() {
        let text = String::from_utf8(valid_synthetic_data_xml()[3..].to_vec()).unwrap();
        let mutated = text
            .replace(
                &format!("DRC-{DOC_UID}-{REP_UID}"),
                &format!("DRC-{DOC_UID}-{NOTE_UID}"),
            )
            .replace(
                &format!(
                    r#"UID1="{DOC_UID}" UID2="{REP_UID}" DefUID="DwgRepresentationComposition""#
                ),
                &format!(
                    r#"UID1="{DOC_UID}" UID2="{NOTE_UID}" DefUID="DwgRepresentationComposition""#
                ),
            );
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(mutated.as_bytes());

        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "DISALLOWED_REL_ENDPOINTS");
        assert!(v[0].actual.contains("PIDDrawing -> PIDNote"));
    }

    #[test]
    fn mutation_6_modified_container_attribute_reports_violation() {
        let text = String::from_utf8(valid_synthetic_data_xml()[3..].to_vec()).unwrap();
        let mutated = text.replace(r#"ToolSignature="AAAD""#, r#"ToolSignature="ZZZZ""#);
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(mutated.as_bytes());

        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "CONTAINER_ATTR_MISMATCH");
        assert!(v[0].expected.contains("AAAD"));
        assert!(v[0].actual.contains("ZZZZ"));
    }

    #[test]
    fn mutation_7_first_element_not_pid_drawing_reports_violation() {
        let raw = with_bom_crlf(&[
            r#"<?xml version ="1.0" encoding="UTF-8"?>"#,
            &format!(
                r#"<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">"#
            ),
            "   <PIDRepresentation>",
            &format!(r#"      <IObject UID="{REP_UID}" />"#),
            r#"      <IDrawingRepresentation GraphicOID="101" />"#,
            "   </PIDRepresentation>",
            "   <PIDDrawing>",
            &format!(r#"      <IObject UID="{DOC_UID}" Name="SYNTH-001" Description="" />"#),
            r#"      <IDocVersionComposition />"#,
            r#"      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="SYNTH" DocType="P&amp;ID" />"#,
            r#"      <IDwgRepresentationComposition />"#,
            r#"      <IPBSItem />"#,
            r#"      <IPIDDrawing />"#,
            r#"      <ISchematicDwg />"#,
            "   </PIDDrawing>",
            "  </Container>",
        ]);

        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "FIRST_ELEMENT_NOT_PID_DRAWING");
    }

    #[test]
    fn schema_only_mode_accepts_no_bom_and_lf_and_custom_plant() {
        let crlf = valid_synthetic_data_xml();
        let text = String::from_utf8(crlf[3..].to_vec())
            .unwrap()
            .replace("\r\n", "\n")
            .replace("   <", "  <")
            .replace("      <", "    <")
            .replace(r#"Plant="P01""#, r#"Plant="SQPlant""#);

        let opts = CheckOptions {
            mode: CheckMode::SchemaOnly,
            expected_plant: Some("SQPlant".to_string()),
        };
        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            text.as_bytes(),
            default_publish_spec(),
            &opts,
        );
        assert!(
            v.is_empty(),
            "schema-only should accept 2-space LF without BOM: {v:?}"
        );

        let wrong_plant = CheckOptions {
            mode: CheckMode::SchemaOnly,
            expected_plant: Some("OTHER".to_string()),
        };
        let v_plant = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            text.as_bytes(),
            default_publish_spec(),
            &wrong_plant,
        );
        assert_eq!(v_plant.len(), 1, "{v_plant:?}");
        assert_eq!(v_plant[0].code, "CONTAINER_ATTR_MISMATCH");
    }

    #[test]
    fn port_bpt_ppt_uid_rules_are_enforced() {
        let _ = (VESSEL_UID, NOZZLE_UID, PORT_UID, CONN_UID, PPT_UID, BPT_UID);
        let raw = with_bom_crlf(&[
            r#"<?xml version ="1.0" encoding="UTF-8"?>"#,
            &format!(
                r#"<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">"#
            ),
            "   <PIDDrawing>",
            &format!(r#"      <IObject UID="{DOC_UID}" Name="SYNTH-001" Description="" />"#),
            r#"      <IDocVersionComposition />"#,
            r#"      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="SYNTH" DocType="P&amp;ID" />"#,
            r#"      <IDwgRepresentationComposition />"#,
            r#"      <IPBSItem />"#,
            r#"      <IPIDDrawing />"#,
            r#"      <ISchematicDwg />"#,
            "   </PIDDrawing>",
            "   <PIDPipingPort>",
            &format!(r#"      <IObject UID="{NOZZLE_UID}" Name="1" />"#),
            r#"      <IConnection />"#,
            r#"      <IPipeCrossSectionItem />"#,
            r#"      <IPipingConnection />"#,
            r#"      <IPipingPort />"#,
            r#"      <IPort />"#,
            "   </PIDPipingPort>",
            "  </Container>",
        ]);
        let v = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &raw,
            default_publish_spec(),
            &CheckOptions::default(),
        );
        assert!(v.iter().any(|x| x.code == "INVALID_OBJECT_UID"), "{v:?}");
    }

    #[test]
    #[ignore = "reads external SPPID XML sample directories via OCS_PID_XML_CHECK_DIRS or OCS_PID_CORPUS_DIR"]
    fn real_sppid_xml_samples_have_zero_violations() {
        let raw_dirs = std::env::var("OCS_PID_XML_CHECK_DIRS")
            .or_else(|_| std::env::var("OCS_PID_CORPUS_DIR"))
            .unwrap_or_default();
        if raw_dirs.trim().is_empty() {
            println!(
                "未设置语料目录（OCS_PID_CORPUS_DIR / OCS_PID_XML_CHECK_DIRS），跳过样本检查。"
            );
            return;
        }
        let opts = CheckOptions::default();
        let mut checked = 0usize;
        let mut all_violations = Vec::new();
        for part in raw_dirs.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let root = Path::new(part);
            let mut stack = vec![root.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = std::fs::read_dir(&dir)
                    .unwrap_or_else(|e| panic!("cannot read dir {}: {e}", dir.display()));
                for entry in entries {
                    let entry = entry.unwrap();
                    let p = entry.path();
                    if p.is_dir() {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if name.eq_ignore_ascii_case("plot") || name.eq_ignore_ascii_case("publish")
                        {
                            continue;
                        }
                        stack.push(p);
                    } else if PublishXmlKind::infer_from_path(&p).is_some() {
                        checked += 1;
                        let vs = check_publish_xml_file(&p, &opts).unwrap();
                        all_violations.extend(vs);
                    }
                }
            }
        }
        println!(
            "checked {checked} XML file(s), violations={}",
            all_violations.len()
        );
        assert!(checked > 0, "expected at least one XML file in {raw_dirs}");
        assert!(
            all_violations.is_empty(),
            "expected 0 violations across {checked} files, got {}:\n{}",
            all_violations.len(),
            all_violations
                .iter()
                .take(20)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
