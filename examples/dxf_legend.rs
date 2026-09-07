//! Headless P&ID legend recognition: what `PIDLEGEND` does in the editor,
//! from a terminal, so a new sheet can be checked without opening it.
//!
//! ```text
//! cargo run --example dxf_legend -- <file.dxf|dwg> [more files...]
//!     [--rules FILE]        rules JSON (default: assets/pid-legend.json, or
//!                           the file OCS_PID_LEGEND_RULES names)
//!     [--units-per-mm N]    override unit detection
//!     [--verbose]           one line per recognised symbol
//!     [--write DIR]         also write <DIR>/<name>.legend.dxf with the
//!                           coloured rectangles and labels drawn in
//!     [--json]              machine-readable output instead of the report
//! ```

use std::path::{Path, PathBuf};

use OpenCADStudio::io;
use OpenCADStudio::io::pid_legend::{self, Rules};
use OpenCADStudio::io::pid_pipes;

struct Options {
    files: Vec<PathBuf>,
    rules: Option<PathBuf>,
    units_per_mm: Option<f64>,
    verbose: bool,
    write: Option<PathBuf>,
    json: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        files: Vec::new(),
        rules: None,
        units_per_mm: None,
        verbose: false,
        write: None,
        json: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rules" => {
                options.rules = Some(PathBuf::from(args.next().ok_or("--rules needs a file")?))
            }
            "--write" => {
                options.write = Some(PathBuf::from(
                    args.next().ok_or("--write needs a directory")?,
                ))
            }
            "--units-per-mm" => {
                let value = args.next().ok_or("--units-per-mm needs a value")?;
                let parsed: f64 = value
                    .parse()
                    .map_err(|_| format!("--units-per-mm: not a number: {value:?}"))?;
                if parsed <= 0.0 {
                    return Err("--units-per-mm must be positive".into());
                }
                options.units_per_mm = Some(parsed);
            }
            "--verbose" | "-v" => options.verbose = true,
            "--json" => options.json = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => options.files.push(PathBuf::from(other)),
        }
    }
    if options.files.is_empty() {
        return Err("usage: dxf_legend <file.dxf|file.dwg> [more files...] [--rules FILE] [--units-per-mm N] [--verbose] [--write DIR] [--json]".into());
    }
    Ok(options)
}

fn symbol_json(symbol: &pid_legend::Recognized) -> serde_json::Value {
    serde_json::json!({
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

fn end_json(end: pid_pipes::End) -> serde_json::Value {
    match end {
        pid_pipes::End::Symbol(i) => serde_json::json!({ "symbol": i }),
        pid_pipes::End::Tee => serde_json::json!("tee"),
        pid_pipes::End::Open => serde_json::json!("open"),
        pid_pipes::End::Loop => serde_json::json!("loop"),
    }
}

fn pipes_json(pipes: &pid_pipes::Pipes) -> serde_json::Value {
    serde_json::json!({
        "segments": pipes.segments,
        "ports": pipes.ports,
        "connected_ports": pipes.connected_ports,
        "open_ends": pipes.open_ends,
        "runs": pipes.runs.iter().map(|r| serde_json::json!({
            "numbers": r.numbers,
            "lines": r.lines,
            "segments": r.segments,
            "length_mm": r.length_mm,
            "ends": [end_json(r.ends[0]), end_json(r.ends[1])],
            "path": r.path.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let rules = match &options.rules {
        Some(path) => match Rules::from_path(path) {
            Ok(rules) => rules,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        },
        None => Rules::load(),
    };

    let mut reports = Vec::new();
    for path in &options.files {
        let name = path.display().to_string();
        let mut doc = match io::load_file(path) {
            Ok(doc) => doc,
            Err(error) => {
                eprintln!("{name}: load failed: {error}");
                continue;
            }
        };
        let recognition = match options.units_per_mm {
            Some(upm) => pid_legend::recognise_with_units(&doc, &rules, upm),
            None => pid_legend::recognise(&doc, &rules),
        };

        if options.json {
            reports.push(serde_json::json!({
                "file": name,
                "units_per_mm": recognition.units_per_mm,
                "lettering": recognition.lettering,
                "symbols": recognition.symbols.iter().map(symbol_json).collect::<Vec<_>>(),
                "unknown_blocks": recognition.unknown_blocks,
                "unknown_shapes": recognition.unknown_shapes.iter().map(|s| serde_json::json!({
                    "id": s.id,
                    "count": s.count,
                    "size_mm": [s.size_mm.0, s.size_mm.1],
                    "strokes": s.strokes,
                    "example_at": [s.example_at.0, s.example_at.1],
                    "nearby": s.nearby,
                })).collect::<Vec<_>>(),
                "orphan_tags": recognition.orphan_tags,
                "pipes": pipes_json(&recognition.pipes),
            }));
        } else {
            println!("{name}");
            for line in pid_legend::report(&recognition) {
                println!("{line}");
            }
            if options.verbose {
                for (index, symbol) in recognition.symbols.iter().enumerate() {
                    println!(
                        "    #{:<4} {:<14} {:<10} at ({:.1}, {:.1}) box ({:.1}, {:.1})-({:.1}, {:.1}) {}{}",
                        index + 1,
                        symbol.class,
                        symbol.label,
                        symbol.at.0,
                        symbol.at.1,
                        symbol.bbox.0,
                        symbol.bbox.1,
                        symbol.bbox.2,
                        symbol.bbox.3,
                        symbol.source,
                        match (&symbol.tag, symbol.tag_distance_mm) {
                            (Some(tag), Some(d)) => format!(" -> {tag} ({d:.1} mm)"),
                            (Some(tag), None) => format!(" -> {tag}"),
                            (None, _) if symbol.wants_tag => " -> (no tag)".to_string(),
                            _ => String::new(),
                        }
                    );
                }
            }
        }

        if let Some(dir) = &options.write {
            let added = pid_legend::apply(&mut doc, &recognition, &rules);
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sheet".to_string());
            let out = dir.join(format!("{stem}.legend.dxf"));
            match write_dxf(&doc, &out) {
                Ok(()) => {
                    if !options.json {
                        println!("  wrote {} ({added} legend entities)", out.display());
                    }
                }
                Err(error) => eprintln!("{}: write failed: {error}", out.display()),
            }
        }
        if !options.json {
            println!();
        }
    }
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports)
                .unwrap_or_else(|e| format!("{{\"error\":{e:?}}}"))
        );
    }
}

fn write_dxf(doc: &acadrust::CadDocument, out: &Path) -> Result<(), String> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    io::save(doc, out)
}
