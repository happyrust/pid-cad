//! Stable machine-readable exports for P&ID recognition.
//!
//! The `dxf_legend` example, `PIDLEGEND EXPORT`, and the line-based JSON
//! automation API all call this module. Keeping one serializer prevents the
//! editor and headless tooling from quietly growing different schemas.

use std::path::Path;

use serde_json::{json, Value};

use super::{Recognition, Recognized};
use crate::io::pid_pipes::{End, Pipes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
}

impl ExportFormat {
    pub fn for_path(path: &Path) -> Result<Self, String> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("json") => Ok(Self::Json),
            Some("csv") => Ok(Self::Csv),
            _ => Err(format!(
                "P&ID export path must end in .json or .csv: {}",
                path.display()
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportReceipt {
    pub format: ExportFormat,
    pub bytes: usize,
}

fn symbol_json(symbol: &Recognized) -> Value {
    json!({
        "class": symbol.class,
        "label": symbol.label,
        "color": symbol.color,
        "at": [symbol.at.0, symbol.at.1],
        "bbox": [symbol.bbox.0, symbol.bbox.1, symbol.bbox.2, symbol.bbox.3],
        "source": symbol.source,
        "known": symbol.known,
        "inner_text": symbol.inner_text,
        "tag": symbol.tag,
        "tag_distance_mm": symbol.tag_distance_mm,
        "lines": symbol.lines,
    })
}

fn end_json(end: End) -> Value {
    match end {
        End::Symbol(index) => json!({ "symbol": index }),
        End::Tee => json!("tee"),
        End::Open => json!("open"),
        End::Loop => json!("loop"),
    }
}

fn pipes_json(pipes: &Pipes) -> Value {
    json!({
        "segments": pipes.segments,
        "ports": pipes.ports,
        "connected_ports": pipes.connected_ports,
        "open_ends": pipes.open_ends,
        "families": pipes.families,
        "runs": pipes.runs.iter().map(|run| json!({
            "numbers": run.numbers,
            "lines": run.lines,
            "segments": run.segments,
            "length_mm": run.length_mm,
            "ends": [end_json(run.ends[0]), end_json(run.ends[1])],
            "path": run.path.iter().map(|point| [point.0, point.1]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Recognition as the object historically emitted for one file by
/// `dxf_legend --json`, minus that command's outer `file` field.
pub fn to_json(recognition: &Recognition) -> Value {
    json!({
        "units_per_mm": recognition.units_per_mm,
        "lettering": recognition.lettering,
        "symbols": recognition.symbols.iter().map(symbol_json).collect::<Vec<_>>(),
        "unknown_blocks": recognition.unknown_blocks,
        "unknown_shapes": recognition.unknown_shapes.iter().map(|shape| json!({
            "id": shape.id,
            "count": shape.count,
            "size_mm": [shape.size_mm.0, shape.size_mm.1],
            "strokes": shape.strokes,
            "example_at": [shape.example_at.0, shape.example_at.1],
            "nearby": shape.nearby,
        })).collect::<Vec<_>>(),
        "orphan_tags": recognition.orphan_tags,
        "range_annotations": recognition.range_annotations.iter().map(|(label, annotations)| {
            (label.clone(), Value::Array(annotations.iter().map(|annotation| json!({
                "value": annotation.value,
                "members": annotation.members,
                "missing": annotation.missing,
            })).collect()))
        }).collect::<serde_json::Map<String, Value>>(),
        "duplicate_tags": recognition.duplicate_tags,
        "pipes": pipes_json(&recognition.pipes),
    })
}

/// One source drawing in the `dxf_legend --json` document shape.
pub fn to_json_document(source: &str, recognition: &Recognition) -> Value {
    let mut value = to_json(recognition);
    value
        .as_object_mut()
        .expect("recognition JSON is an object")
        .insert("file".to_string(), Value::String(source.to_string()));
    value
}

/// Pretty JSON for one drawing. The outer array deliberately matches
/// `dxf_legend --json`, which accepts several files in one invocation.
pub fn to_json_pretty(source: &str, recognition: &Recognition) -> Result<String, String> {
    json_documents_pretty(&[to_json_document(source, recognition)])
}

/// Pretty-print one or more [`to_json_document`] values.
pub fn json_documents_pretty(documents: &[Value]) -> Result<String, String> {
    let mut output = serde_json::to_string_pretty(documents).map_err(|error| error.to_string())?;
    output.push('\n');
    Ok(output)
}

fn csv_number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let mut text = format!("{value:.6}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_row(output: &mut String, values: impl IntoIterator<Item = String>) {
    let mut first = true;
    for value in values {
        if !first {
            output.push(',');
        }
        first = false;
        output.push_str(&csv_cell(&value));
    }
    output.push('\n');
}

fn end_csv(end: End, symbols: &[Recognized]) -> String {
    match end {
        End::Symbol(index) => symbols
            .get(index)
            .map(|symbol| {
                symbol
                    .tag
                    .clone()
                    .unwrap_or_else(|| format!("{} #{}", symbol.label, index + 1))
            })
            .unwrap_or_else(|| format!("symbol #{}", index + 1)),
        End::Tee => "tee".to_string(),
        End::Open => "open".to_string(),
        End::Loop => "loop".to_string(),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

/// Two CSV tables in one UTF-8 document: symbols first, then pipe lines.
///
/// The pipe table has one row per line number. `from` and `to` list the
/// distinct endpoints of every run carrying that number, so an in-line chain
/// retains all of its intervening tagged valves rather than only its two
/// outermost ends.
pub fn to_csv(recognition: &Recognition) -> String {
    let mut output = String::new();
    csv_row(
        &mut output,
        ["class", "label", "tag", "x_mm", "y_mm", "lines", "source"]
            .into_iter()
            .map(str::to_string),
    );
    let units_per_mm =
        if recognition.units_per_mm.is_finite() && recognition.units_per_mm > f64::EPSILON {
            recognition.units_per_mm
        } else {
            1.0
        };
    for symbol in &recognition.symbols {
        csv_row(
            &mut output,
            [
                symbol.class.clone(),
                symbol.label.clone(),
                symbol.tag.clone().unwrap_or_default(),
                csv_number(symbol.at.0 / units_per_mm),
                csv_number(symbol.at.1 / units_per_mm),
                symbol.lines.join(" + "),
                symbol.source.clone(),
            ],
        );
    }

    output.push('\n');
    csv_row(
        &mut output,
        ["line", "family", "runs", "length_mm", "from", "to"]
            .into_iter()
            .map(str::to_string),
    );
    for (line, runs) in recognition.pipes.by_line() {
        let mut from = Vec::new();
        let mut to = Vec::new();
        for run in &runs {
            push_unique(&mut from, end_csv(run.ends[0], &recognition.symbols));
            push_unique(&mut to, end_csv(run.ends[1], &recognition.symbols));
        }
        csv_row(
            &mut output,
            [
                line.to_string(),
                recognition.pipes.family_of(line).to_string(),
                runs.len().to_string(),
                csv_number(runs.iter().map(|run| run.length_mm).sum()),
                from.join(" | "),
                to.join(" | "),
            ],
        );
    }
    output
}

/// Render according to `path`'s extension.
pub fn render(path: &Path, source: &str, recognition: &Recognition) -> Result<String, String> {
    match ExportFormat::for_path(path)? {
        ExportFormat::Json => to_json_pretty(source, recognition),
        ExportFormat::Csv => Ok(to_csv(recognition)),
    }
}

/// Render and write an export, creating its parent directory when necessary.
#[cfg(not(target_arch = "wasm32"))]
pub fn write(
    path: &Path,
    source: &str,
    recognition: &Recognition,
) -> Result<ExportReceipt, String> {
    let format = ExportFormat::for_path(path)?;
    let body = render(path, source, recognition)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, body.as_bytes()).map_err(|error| error.to_string())?;
    Ok(ExportReceipt {
        format,
        bytes: body.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_legend::{RangeAnnotation, UnknownShape};
    use crate::io::pid_pipes::Run;
    use acadrust::Handle;
    use std::collections::BTreeMap;

    fn symbol(index: u64, tag: &str, at: (f64, f64)) -> Recognized {
        Recognized {
            class: "butterfly".to_string(),
            label: "蝶阀".to_string(),
            color: [255, 140, 0],
            at,
            bbox: (at.0 - 1.0, at.1 - 1.0, at.0 + 1.0, at.1 + 1.0),
            source: "$VALVE$00000316".to_string(),
            group: None,
            known: true,
            inner_text: Vec::new(),
            tag: Some(tag.to_string()),
            tag_distance_mm: Some(2.0),
            wants_tag: true,
            report_untagged: true,
            lines: vec!["100-FW".to_string()],
            handles: vec![Handle::new(index)],
            tag_handles: Vec::new(),
        }
    }

    fn recognition() -> Recognition {
        let symbols = vec![
            symbol(1, "BUV-1", (20.0, 40.0)),
            symbol(2, "BUV-2", (40.0, 40.0)),
            symbol(3, "BUV-3", (60.0, 40.0)),
        ];
        let run = |from, to, length| Run {
            numbers: vec!["100-FW".to_string()],
            lines: vec!["100-FW".to_string()],
            segments: 1,
            length_mm: length,
            ends: [from, to],
            path: vec![(0.0, 0.0), (length, 0.0)],
            handles: Vec::new(),
        };
        Recognition {
            units_per_mm: 2.0,
            symbols,
            unknown_blocks: BTreeMap::from([("MYSTERY on 0".to_string(), 2)]),
            unknown_shapes: vec![UnknownShape {
                id: "abc123".to_string(),
                count: 2,
                size_mm: (2.0, 3.0),
                strokes: 4,
                example_at: (5.0, 6.0),
                nearby: vec![("NOTE".to_string(), 1)],
            }],
            orphan_tags: BTreeMap::from([("蝶阀".to_string(), vec!["BUV-9".to_string()])]),
            range_annotations: BTreeMap::from([(
                "蝶阀".to_string(),
                vec![RangeAnnotation {
                    value: "BUV-1/2".to_string(),
                    members: vec!["BUV-1".to_string(), "BUV-2".to_string()],
                    missing: Vec::new(),
                }],
            )]),
            duplicate_tags: BTreeMap::from([("蝶阀".to_string(), vec!["BUV-1".to_string()])]),
            lettering: 4,
            pipes: Pipes {
                runs: vec![
                    run(End::Open, End::Symbol(0), 10.0),
                    run(End::Symbol(0), End::Symbol(1), 20.0),
                    run(End::Symbol(1), End::Symbol(2), 30.0),
                    run(End::Symbol(2), End::Open, 40.0),
                ],
                segments: 4,
                ports: 6,
                connected_ports: 6,
                open_ends: 2,
                families: BTreeMap::from([("100-FW".to_string(), "100-FW".to_string())]),
            },
        }
    }

    #[test]
    fn json_keeps_the_dxf_legend_machine_schema() {
        let value = to_json_document("sheet.dxf", &recognition());
        assert_eq!(value["file"], "sheet.dxf");
        assert_eq!(value["symbols"][0]["tag"], "BUV-1");
        assert_eq!(value["unknown_shapes"][0]["id"], "abc123");
        assert_eq!(
            value["range_annotations"]["蝶阀"][0]["members"],
            json!(["BUV-1", "BUV-2"])
        );
        assert_eq!(value["pipes"]["runs"][0]["ends"][0], "open");
        assert_eq!(value["pipes"]["runs"][0]["ends"][1]["symbol"], 0);
        let pretty = to_json_pretty("sheet.dxf", &recognition()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&pretty).unwrap()[0], value);
    }

    #[test]
    fn csv_lists_symbols_and_every_endpoint_on_a_numbered_line() {
        let mut recognition = recognition();
        recognition.symbols[0].tag = Some("BUV-1, \"north\"".to_string());
        let csv = to_csv(&recognition);
        assert!(csv.starts_with("class,label,tag,x_mm,y_mm,lines,source\n"));
        assert!(csv.contains("\"BUV-1, \"\"north\"\"\""));
        assert!(csv.contains("butterfly,蝶阀,\"BUV-1, \"\"north\"\"\",10,20,100-FW"));
        assert!(csv.contains("\nline,family,runs,length_mm,from,to\n"));
        let line = csv
            .lines()
            .find(|line| line.starts_with("100-FW,"))
            .expect("pipe line row");
        assert!(line.contains(",4,100,"));
        for endpoint in ["BUV-1, \"\"north\"\"", "BUV-2", "BUV-3", "open"] {
            assert!(line.contains(endpoint), "{endpoint}: {line}");
        }
    }

    #[test]
    fn export_format_is_selected_from_a_case_insensitive_extension() {
        assert_eq!(
            ExportFormat::for_path(Path::new("legend.JSON")).unwrap(),
            ExportFormat::Json
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("legend.csv")).unwrap(),
            ExportFormat::Csv
        );
        assert!(ExportFormat::for_path(Path::new("legend.txt")).is_err());
    }
}
