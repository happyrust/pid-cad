//! The per-class summary for the command line and `dxf_legend`.

use super::*;

/// A per-class summary, one line per class plus the unknown-block,
/// unknown-shape and orphan-tag lines, for the command line or a terminal.
pub fn report(recognition: &Recognition) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} symbols recognised, {} pieces of lettering, {} units/mm",
        recognition.symbols.len(),
        recognition.lettering,
        recognition.units_per_mm
    ));
    let groups: Vec<&Recognized> = recognition
        .symbols
        .iter()
        .filter(|symbol| symbol.group.is_some())
        .collect();
    if !groups.is_empty() {
        let tagged = groups.iter().filter(|symbol| symbol.tag.is_some()).count();
        let manual_tags = groups
            .iter()
            .filter(|symbol| {
                symbol.tag.is_some()
                    && symbol
                        .group
                        .as_ref()
                        .is_some_and(|group| group.tag_source == TagSource::Manual)
            })
            .count();
        lines.push(format!(
            "  GROUP {} manual symbols, {tagged} tagged ({manual_tags} manual tags)",
            groups.len()
        ));
    }
    for ((class, label), items) in recognition.by_class() {
        if is_shape_class(&class) {
            // Summarised below, one line per shape id.
            continue;
        }
        let mut line = format!("  {label} ({class}) x{}", items.len());
        if items[0].wants_tag {
            let tagged: Vec<&&Recognized> = items.iter().filter(|s| s.tag.is_some()).collect();
            line.push_str(&format!("  tagged {}/{}", tagged.len(), items.len()));
            let mut distances: Vec<f64> = tagged.iter().filter_map(|s| s.tag_distance_mm).collect();
            distances.retain(|d| *d > 0.0);
            if let (Some(min), Some(max)) = (
                distances.iter().copied().reduce(f64::min),
                distances.iter().copied().reduce(f64::max),
            ) {
                line.push_str(&format!("  at {min:.1}..{max:.1} mm"));
            }
            let mut tags: Vec<&str> = tagged.iter().filter_map(|s| s.tag.as_deref()).collect();
            tags.sort_unstable();
            if !tags.is_empty() {
                line.push_str(&format!(": {}", tags.join(", ")));
            }
            // Where the untagged ones stand -- unless the class says its tags
            // are not worth chasing (the flow arrows): then the count says it.
            let untagged: Vec<String> = items
                .iter()
                .filter(|s| s.tag.is_none() && s.report_untagged)
                .map(|s| format!("({:.0}, {:.0})", s.at.0, s.at.1))
                .collect();
            if !untagged.is_empty() {
                line.push_str(&format!("  UNTAGGED at {}", untagged.join(" ")));
            }
        }
        lines.push(line);
    }
    for (what, n) in &recognition.unknown_blocks {
        lines.push(format!(
            "  UNKNOWN BLOCK {what} x{n} -- add it to the rules"
        ));
    }
    for shape in recognition.unknown_shapes.iter().take(REPORTED_SHAPES) {
        let near: Vec<String> = shape
            .nearby
            .iter()
            .map(|(v, n)| {
                if *n > 1 {
                    format!("{v} x{n}")
                } else {
                    v.clone()
                }
            })
            .collect();
        lines.push(format!(
            "  UNKNOWN SHAPE {} x{}  {:.1}x{:.1} mm  {} strokes  e.g. at ({:.0}, {:.0}){} -- name it under shapes.dictionary",
            shape.id,
            shape.count,
            shape.size_mm.0,
            shape.size_mm.1,
            shape.strokes,
            shape.example_at.0,
            shape.example_at.1,
            if near.is_empty() {
                String::new()
            } else {
                format!("  near: {}", near.join(", "))
            }
        ));
    }
    if recognition.unknown_shapes.len() > REPORTED_SHAPES {
        lines.push(format!(
            "  ... and {} more unknown shapes",
            recognition.unknown_shapes.len() - REPORTED_SHAPES
        ));
    }
    for (label, tags) in &recognition.orphan_tags {
        lines.push(format!(
            "  ORPHAN {label} tags (no symbol claimed them): {}",
            tags.join(", ")
        ));
    }
    for (label, annotations) in &recognition.range_annotations {
        let spelt: Vec<String> = annotations
            .iter()
            .map(|a| {
                if a.missing.is_empty() {
                    a.value.clone()
                } else {
                    format!("{} (missing {})", a.value, a.missing.join(", "))
                }
            })
            .collect();
        let complete = annotations.iter().filter(|a| a.missing.is_empty()).count();
        lines.push(format!(
            "  RANGE {label} annotations, {complete}/{} with every member on a symbol: {}",
            annotations.len(),
            spelt.join(", ")
        ));
    }
    for (label, tags) in &recognition.duplicate_tags {
        lines.push(format!(
            "  DUPLICATE {label} tags (a symbol already carries them): {}",
            tags.join(", ")
        ));
    }
    let pipes = &recognition.pipes;
    if pipes.segments > 0 {
        let lettered = pipes.runs.iter().filter(|r| !r.numbers.is_empty()).count();
        let on_a_line = pipes.runs.iter().filter(|r| !r.lines.is_empty()).count();
        let ambiguous = pipes.runs.iter().filter(|r| r.lines.len() > 1).count();
        lines.push(format!(
            "  PIPE {} strokes -> {} runs: {lettered} lettered, {on_a_line} on a line ({ambiguous} on two), {} on none; {}/{} connection points on a pipe, {} open ends",
            pipes.segments,
            pipes.runs.len(),
            pipes.runs.len() - on_a_line,
            pipes.connected_ports,
            pipes.ports,
            pipes.open_ends
        ));
        for (line, runs) in pipes.by_line() {
            let length: f64 = runs.iter().map(|r| r.length_mm).sum();
            let mut on: Vec<String> = pipes
                .symbols_on(line)
                .into_iter()
                .map(|i| {
                    let s = &recognition.symbols[i];
                    s.tag
                        .clone()
                        .unwrap_or_else(|| format!("{} ({:.0}, {:.0})", s.label, s.at.0, s.at.1))
                })
                .collect();
            on.sort();
            on.dedup();
            lines.push(format!(
                "  LINE {line}: {} runs, {length:.0} mm, symbols: {}",
                runs.len(),
                if on.is_empty() {
                    "-".to_string()
                } else {
                    on.join(", ")
                }
            ));
        }
        let off_line: Vec<&pid_pipes::Run> =
            pipes.runs.iter().filter(|r| r.lines.is_empty()).collect();
        if !off_line.is_empty() {
            let length: f64 = off_line.iter().map(|r| r.length_mm).sum();
            lines.push(format!(
                "  LINE (none): {} runs, {length:.0} mm on no numbered line",
                off_line.len()
            ));
        }
    }
    lines
}
