//! Nominal diameter formatting, SPPID line-number decomposition, and composite
//! object name / segmented-attribute rules (`L4-5` / `#126`).

use super::model::ExportItem;

/// Structured SPPID line-number slots parsed from a drawing label
/// (e.g. `25-CI-0201505-B9-EH40`, `25-CI-0201503-B9-`, `6"-PG-749-B5-P40`, `1-1/2"-IA-101-A1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLineNumber {
    pub raw_size: String,
    pub nominal_diameter_attr: String,
    pub size_in_name: String,
    pub fluid_code: String,
    pub seq_no: String,
    pub materials_class: String,
    pub insulation: Option<String>,
}

impl ParsedLineNumber {
    pub fn pipeline_name(&self) -> String {
        match &self.insulation {
            Some(ins) if !ins.is_empty() => format!(
                "{}-{}-{}-{}-{}",
                self.size_in_name, self.fluid_code, self.seq_no, self.materials_class, ins
            ),
            _ => format!(
                "{}-{}-{}-{}",
                self.size_in_name, self.fluid_code, self.seq_no, self.materials_class
            ),
        }
    }

    pub fn connector_name(&self) -> String {
        format!(
            "{}-{}-{}--",
            self.pipeline_name(),
            self.size_in_name,
            self.materials_class
        )
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}

fn parse_diameter_core(raw: &str) -> Option<(String, String)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // Strip leading DN / NPS if present
    let s_clean = s
        .strip_prefix("DN")
        .or_else(|| s.strip_prefix("dn"))
        .map(str::trim)
        .unwrap_or(s);

    // Explicit `"mm"` / `"MM"` suffix
    if let Some(num) = s_clean
        .strip_suffix("mm")
        .or_else(|| s_clean.strip_suffix("MM"))
        .map(str::trim)
    {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') {
            let formatted = format!("{num} mm");
            return Some((formatted.clone(), formatted));
        }
    }

    // Explicit `"in"` / `"IN"` suffix
    if let Some(num) = s_clean
        .strip_suffix("in")
        .or_else(|| s_clean.strip_suffix("IN"))
        .map(str::trim)
    {
        if !num.is_empty()
            && num
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '/' | '.'))
        {
            return Some((format!("{num} in"), format!("{num}\"")));
        }
    }

    // Quote suffix: `6"`, `1-1/2"`, `3/4"`, `6''`
    if let Some(num) = s_clean
        .strip_suffix('"')
        .or_else(|| s_clean.strip_suffix("''"))
        .map(str::trim)
    {
        if !num.is_empty()
            && num
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '/' | '.'))
        {
            return Some((format!("{num} in"), format!("{num}\"")));
        }
    }

    // Pure metric number: `25`, `150`
    if s_clean.chars().all(|c| c.is_ascii_digit() || c == '.')
        && s_clean.chars().any(|c| c.is_ascii_digit())
    {
        let formatted = format!("{s_clean} mm");
        return Some((formatted.clone(), formatted));
    }

    None
}

/// Format a raw nominal diameter string into SPPID's `NominalDiameter` attribute
/// (`"25"` -> `"25 mm"`, `"6\""` -> `"6 in"`, `"1-1/2\""` -> `"1-1/2 in"`).
pub fn format_nominal_diameter_attr(raw: &str) -> Option<String> {
    parse_diameter_core(raw).map(|(attr, _)| attr)
}

/// Format a raw nominal diameter token as SPPID embeds it inside `PIDPipeline` /
/// `PIDPipingConnector` `IObject/@Name` (`"25"` -> `"25 mm"`, `"6\""` -> `"6\""`).
pub fn format_nominal_diameter_in_name(raw: &str) -> Option<String> {
    parse_diameter_core(raw).map(|(_, in_name)| in_name)
}

/// Format an integer SPPID diameter code per Q6 (`< 5000` -> `"{code} mm"`,
/// `>= 5000` -> reduced `(code - 5000) / 32` in inches).
pub fn format_nominal_diameter_from_code(code: u32) -> String {
    if code < 5000 {
        return format!("{code} mm");
    }
    let ticks = code - 5000;
    let whole = ticks / 32;
    let rem = ticks % 32;
    if rem == 0 {
        format!("{whole} in")
    } else {
        let g = gcd(rem, 32);
        let num = rem / g;
        let den = 32 / g;
        if whole == 0 {
            format!("{num}/{den} in")
        } else {
            format!("{whole}-{num}/{den} in")
        }
    }
}

/// Parse a drawing line-number string into structured SPPID pipeline/connector slots.
pub fn parse_sppid_line_number(raw: &str) -> Option<ParsedLineNumber> {
    let trimmed = raw.trim().trim_end_matches('-').trim();
    if trimmed.is_empty() {
        return None;
    }
    let raw_parts: Vec<&str> = trimmed.split('-').map(str::trim).collect();
    if raw_parts.len() < 4 {
        return None;
    }

    // Handle fractional inch size with internal hyphen (`1-1/2"-IA-101-A1`)
    let (raw_size, idx_after_size) = if raw_parts.len() >= 5
        && !raw_parts[0].is_empty()
        && raw_parts[0].chars().all(|c| c.is_ascii_digit())
        && raw_parts[1].contains('/')
        && (raw_parts[1].ends_with('"')
            || raw_parts[1].ends_with("''")
            || raw_parts[1].to_ascii_lowercase().ends_with("in"))
    {
        (format!("{}-{}", raw_parts[0], raw_parts[1]), 2usize)
    } else {
        (raw_parts[0].to_string(), 1usize)
    };

    if raw_parts.len() < idx_after_size + 3 {
        return None;
    }
    let (nominal_diameter_attr, size_in_name) = parse_diameter_core(&raw_size)?;
    let fluid_code = raw_parts[idx_after_size].trim();
    let seq_no = raw_parts[idx_after_size + 1].trim();
    let materials_class = raw_parts[idx_after_size + 2].trim();
    if fluid_code.is_empty() || seq_no.is_empty() || materials_class.is_empty() {
        return None;
    }

    let rest: Vec<&str> = raw_parts[(idx_after_size + 3)..]
        .iter()
        .copied()
        .filter(|s| !s.is_empty())
        .collect();
    let insulation = if rest.is_empty() {
        None
    } else {
        Some(rest.join("-"))
    };

    Some(ParsedLineNumber {
        raw_size,
        nominal_diameter_attr,
        size_in_name,
        fluid_code: fluid_code.to_string(),
        seq_no: seq_no.to_string(),
        materials_class: materials_class.to_string(),
        insulation,
    })
}

/// Synchronize `IObject/@Name` and segmented tag attributes (`INamedInstrument`,
/// `INamedEquipment`) on `item` according to SPPID Q5 rules.
pub fn populate_item_tag_attributes(item: &mut ExportItem) {
    match item.class.as_str() {
        "PIDInstrument" | "PIDControlSystemFunction" | "PIDInlineInstrument" => {
            let mv_key = (
                "INamedInstrument".to_string(),
                "MeasuredVariable".to_string(),
            );
            let seq_key = (
                "INamedInstrument".to_string(),
                "InstrTagSequenceNo".to_string(),
            );
            if let Some(name) = item.name.clone() {
                if let Some((mv, seq)) = name.trim().split_once('-') {
                    if !mv.is_empty() && !seq.is_empty() {
                        item.attributes
                            .entry(mv_key)
                            .or_insert_with(|| mv.trim().to_string());
                        item.attributes
                            .entry(seq_key)
                            .or_insert_with(|| seq.trim().to_string());
                    }
                }
            } else {
                let mv = item.attributes.get(&mv_key).map(|s| s.trim()).unwrap_or("");
                let seq = item
                    .attributes
                    .get(&seq_key)
                    .map(|s| s.trim())
                    .unwrap_or("");
                if !mv.is_empty() && !seq.is_empty() {
                    item.name = Some(format!("{mv}-{seq}"));
                }
            }
        }
        "PIDProcessVessel"
        | "PIDMechanicalEquipment"
        | "PIDProcessEquipment"
        | "PIDHeatTransferEquipment" => {
            let pfx_key = ("INamedEquipment".to_string(), "EquipPrefix".to_string());
            let seq_key = ("INamedEquipment".to_string(), "EquipSeqNo".to_string());
            let suf_key = ("INamedEquipment".to_string(), "EquipSuff".to_string());
            if let Some(name) = item.name.clone() {
                if let Some((pfx, seq)) = name.trim().split_once('-') {
                    if !pfx.is_empty() && !seq.is_empty() {
                        item.attributes
                            .entry(pfx_key)
                            .or_insert_with(|| pfx.trim().to_string());
                        item.attributes
                            .entry(seq_key)
                            .or_insert_with(|| seq.trim().to_string());
                        item.attributes.entry(suf_key).or_default();
                    }
                }
            } else {
                let pfx = item
                    .attributes
                    .get(&pfx_key)
                    .map(|s| s.trim())
                    .unwrap_or("");
                let seq = item
                    .attributes
                    .get(&seq_key)
                    .map(|s| s.trim())
                    .unwrap_or("");
                let suf = item
                    .attributes
                    .get(&suf_key)
                    .map(|s| s.trim())
                    .unwrap_or("");
                if !pfx.is_empty() && !seq.is_empty() {
                    item.name = Some(format!("{pfx}-{seq}{suf}"));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_metric_and_imperial_nominal_diameters_and_integer_codes() {
        assert_eq!(format_nominal_diameter_attr("25").as_deref(), Some("25 mm"));
        assert_eq!(
            format_nominal_diameter_attr("150 MM").as_deref(),
            Some("150 mm")
        );
        assert_eq!(
            format_nominal_diameter_attr("DN50").as_deref(),
            Some("50 mm")
        );
        assert_eq!(format_nominal_diameter_attr("6\"").as_deref(), Some("6 in"));
        assert_eq!(
            format_nominal_diameter_attr("1-1/2\"").as_deref(),
            Some("1-1/2 in")
        );
        assert_eq!(
            format_nominal_diameter_attr("3/4\"").as_deref(),
            Some("3/4 in")
        );

        assert_eq!(
            format_nominal_diameter_in_name("25").as_deref(),
            Some("25 mm")
        );
        assert_eq!(
            format_nominal_diameter_in_name("6\"").as_deref(),
            Some("6\"")
        );

        assert_eq!(format_nominal_diameter_from_code(100), "100 mm");
        assert_eq!(format_nominal_diameter_from_code(5000 + 4 * 32), "4 in");
        assert_eq!(
            format_nominal_diameter_from_code(5000 + 32 + 16),
            "1-1/2 in"
        );
        assert_eq!(format_nominal_diameter_from_code(5000 + 24), "3/4 in");
    }

    #[test]
    fn parses_sppid_line_numbers_and_builds_pipeline_and_connector_names() {
        let p1 = parse_sppid_line_number("25-CI-0201505-B9-EH40").expect("metric insulated");
        assert_eq!(p1.nominal_diameter_attr, "25 mm");
        assert_eq!(p1.pipeline_name(), "25 mm-CI-0201505-B9-EH40");
        assert_eq!(p1.connector_name(), "25 mm-CI-0201505-B9-EH40-25 mm-B9--");

        let p2 = parse_sppid_line_number("25-CI-0201503-B9-").expect("trailing hyphen uninsulated");
        assert_eq!(p2.insulation, None);
        assert_eq!(p2.pipeline_name(), "25 mm-CI-0201503-B9");
        assert_eq!(p2.connector_name(), "25 mm-CI-0201503-B9-25 mm-B9--");

        let p3 = parse_sppid_line_number("6\"-PG-749-B5-P40").expect("imperial insulated");
        assert_eq!(p3.nominal_diameter_attr, "6 in");
        assert_eq!(p3.size_in_name, "6\"");
        assert_eq!(p3.pipeline_name(), "6\"-PG-749-B5-P40");
        assert_eq!(p3.connector_name(), "6\"-PG-749-B5-P40-6\"-B5--");

        let p4 = parse_sppid_line_number("1-1/2\"-IA-101-A1").expect("fractional inch");
        assert_eq!(p4.nominal_diameter_attr, "1-1/2 in");
        assert_eq!(p4.pipeline_name(), "1-1/2\"-IA-101-A1");
    }

    #[test]
    fn populates_segmented_instrument_and_equipment_attributes_bidirectionally() {
        let mut inst = ExportItem {
            uid: "1569C79E599040069D038B74D16A03CA".to_string(),
            class: "PIDInstrument".to_string(),
            name: Some("PG-0201501A".to_string()),
            ..Default::default()
        };
        populate_item_tag_attributes(&mut inst);
        assert_eq!(
            inst.attributes
                .get(&(
                    "INamedInstrument".to_string(),
                    "MeasuredVariable".to_string()
                ))
                .map(String::as_str),
            Some("PG")
        );
        assert_eq!(
            inst.attributes
                .get(&(
                    "INamedInstrument".to_string(),
                    "InstrTagSequenceNo".to_string()
                ))
                .map(String::as_str),
            Some("0201501A")
        );

        let mut eq = ExportItem {
            uid: "B381617DF61B47F2BE9493BFDDC44619".to_string(),
            class: "PIDMechanicalEquipment".to_string(),
            name: Some("P-02015A".to_string()),
            ..Default::default()
        };
        populate_item_tag_attributes(&mut eq);
        assert_eq!(
            eq.attributes
                .get(&("INamedEquipment".to_string(), "EquipPrefix".to_string()))
                .map(String::as_str),
            Some("P")
        );
        assert_eq!(
            eq.attributes
                .get(&("INamedEquipment".to_string(), "EquipSeqNo".to_string()))
                .map(String::as_str),
            Some("02015A")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "audits Q5/Q6 name & diameter composition rules across all 94 real _Data.xml files"]
    fn real_94_xml_name_and_diameter_rule_audit() {
        let Ok(xml_dir) = std::env::var("OCS_PID_XML_DIR") else {
            println!("未设置 XML 目录（OCS_PID_XML_DIR），跳过全批名称与公称直径规则核对。");
            return;
        };
        let mut xmls = Vec::new();
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            v.sort();
            for p in v {
                if p.is_dir() {
                    walk(&p, out);
                } else if p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n.ends_with("_Data.xml"))
                {
                    out.push(p);
                }
            }
        }
        walk(std::path::Path::new(&xml_dir), &mut xmls);

        let mut inst_tot = 0usize;
        let mut inst_match = 0usize;
        let mut eq_tot = 0usize;
        let mut eq_match = 0usize;
        let mut pipe_tot = 0usize;
        let mut pipe_parsed = 0usize;
        let mut diam_tot = 0usize;
        let mut diam_valid = 0usize;

        for f in &xmls {
            let raw = std::fs::read(f).unwrap();
            let body = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
                &raw[3..]
            } else {
                &raw[..]
            };
            let txt = std::str::from_utf8(body).unwrap();
            let parse_s = if let Some(rest) = txt.strip_prefix("<?xml version =") {
                format!("<?xml  version={rest}")
            } else {
                txt.to_string()
            };
            let doc = roxmltree::Document::parse(&parse_s).unwrap();
            for el in doc.root_element().children().filter(|n| n.is_element()) {
                let cls = el.tag_name().name();
                let iobj = el
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "IObject");
                let name = iobj.and_then(|c| c.attribute("Name")).unwrap_or("");

                if let Some(pcs) = el
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "IPipeCrossSectionItem")
                {
                    if let Some(nd) = pcs.attribute("NominalDiameter") {
                        if !nd.is_empty() {
                            diam_tot += 1;
                            if format_nominal_diameter_attr(nd).as_deref() == Some(nd) {
                                diam_valid += 1;
                            }
                        }
                    }
                }

                match cls {
                    "PIDInstrument" | "PIDControlSystemFunction" | "PIDInlineInstrument" => {
                        if let Some(ni) = el
                            .children()
                            .find(|c| c.is_element() && c.tag_name().name() == "INamedInstrument")
                        {
                            let mv = ni.attribute("MeasuredVariable").unwrap_or("");
                            let seq = ni.attribute("InstrTagSequenceNo").unwrap_or("");
                            let suf = ni.attribute("InstrTagSuffix").unwrap_or("");
                            if !name.is_empty() && !mv.is_empty() && !seq.is_empty() {
                                inst_tot += 1;
                                let composed = format!("{mv}-{seq}{suf}");
                                if composed == name {
                                    inst_match += 1;
                                }
                            }
                        }
                    }
                    "PIDProcessVessel"
                    | "PIDMechanicalEquipment"
                    | "PIDProcessEquipment"
                    | "PIDHeatTransferEquipment" => {
                        if let Some(ne) = el
                            .children()
                            .find(|c| c.is_element() && c.tag_name().name() == "INamedEquipment")
                        {
                            let pfx = ne.attribute("EquipPrefix").unwrap_or("");
                            let seq = ne.attribute("EquipSeqNo").unwrap_or("");
                            let suf = ne.attribute("EquipSuff").unwrap_or("");
                            if !name.is_empty() && !pfx.is_empty() && !seq.is_empty() {
                                eq_tot += 1;
                                let composed = format!("{pfx}-{seq}{suf}");
                                if composed == name {
                                    eq_match += 1;
                                }
                            }
                        }
                    }
                    "PIDPipeline" if !name.is_empty() => {
                        pipe_tot += 1;
                        if parse_sppid_line_number(name).is_some_and(|p| p.pipeline_name() == name)
                        {
                            pipe_parsed += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        println!(
            "xml_files={} inst={}/{} ({:.4}) eq={}/{} ({:.4}) pipeline_std={}/{} ({:.4}) diam={}/{} ({:.4})",
            xmls.len(),
            inst_match,
            inst_tot,
            inst_match as f64 / inst_tot.max(1) as f64,
            eq_match,
            eq_tot,
            eq_match as f64 / eq_tot.max(1) as f64,
            pipe_parsed,
            pipe_tot,
            pipe_parsed as f64 / pipe_tot.max(1) as f64,
            diam_valid,
            diam_tot,
            diam_valid as f64 / diam_tot.max(1) as f64,
        );
        assert_eq!(xmls.len(), 94);
        assert!(inst_match as f64 / inst_tot as f64 >= 0.98);
        assert!(eq_match as f64 / eq_tot as f64 >= 0.98);
        assert_eq!(diam_valid, diam_tot);
    }
}
