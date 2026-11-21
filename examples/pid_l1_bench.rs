//! Batch `.pid` open-performance benchmark, `L1-4` character-run audit, and
//! `PIDPLOT` PDF renderer across a SmartPlant `.pid` directory (`L1-2` / `L1-4`
//! / `L1-5` / `#131`).
//!
//! ```text
//! cargo run --example pid_l1_bench -- <dir-or-pid>... --out-json <bench.json> [--plots-dir <dir>]
//! ```

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pid_parse::style_link::TextRunStatus;
use sha2::{Digest, Sha256};
use OpenCADStudio::{io, scene};

const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

struct CliArgs {
    inputs: Vec<PathBuf>,
    out_json: PathBuf,
    plots_dir: Option<PathBuf>,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut inputs = Vec::new();
    let mut out_json: Option<PathBuf> = None;
    let mut plots_dir: Option<PathBuf> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out-json" => {
                out_json = Some(PathBuf::from(
                    it.next().ok_or("--out-json requires a file path")?,
                ));
            }
            "--plots-dir" => {
                plots_dir = Some(PathBuf::from(
                    it.next().ok_or("--plots-dir requires a directory path")?,
                ));
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option '{other}'"));
            }
            other => {
                inputs.push(PathBuf::from(other));
            }
        }
    }

    if inputs.is_empty() {
        return Err(
            "usage: pid_l1_bench <dir-or-pid>... --out-json <bench.json> [--plots-dir <dir>]"
                .to_string(),
        );
    }
    let out_json = out_json.ok_or("--out-json <bench.json> is required")?;
    Ok(CliArgs {
        inputs,
        out_json,
        plots_dir,
    })
}

fn is_compound_file(path: &Path) -> bool {
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && head == CFB_MAGIC
}

fn collect_pids(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
        children.sort();
        for child in children {
            collect_pids(&child, out);
        }
    } else if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pid"))
        && is_compound_file(path)
    {
        out.push(path.to_path_buf());
    }
}

fn sha256_prefix_12(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let full = format!("{:x}", Sha256::digest(&bytes));
    full.chars().take(12).collect()
}

#[derive(Default)]
struct TextRunCounts {
    total_texts: usize,
    uniform: usize,
    no_runs: usize,
    flattened: usize,
    unresolvable: usize,
    length_mismatch: usize,
}

fn audit_text_runs(path: &Path) -> TextRunCounts {
    let Ok(parsed) =
        pid_parse::PidParser::with_options(pid_parse::ParseOptions::geometry()).parse_file(path)
    else {
        return TextRunCounts::default();
    };
    let styles = pid_parse::style_link::text_styles_for_document(&parsed);
    let mut counts = TextRunCounts {
        total_texts: styles.len(),
        ..Default::default()
    };
    for resolved in styles.values() {
        match resolved.runs {
            TextRunStatus::Uniform => counts.uniform += 1,
            TextRunStatus::NoRuns => counts.no_runs += 1,
            TextRunStatus::Flattened { .. } => counts.flattened += 1,
            TextRunStatus::Unresolvable => counts.unresolvable += 1,
            TextRunStatus::LengthMismatch => counts.length_mismatch += 1,
        }
    }
    counts
}

fn round2(ms: f64) -> f64 {
    (ms * 100.0).round() / 100.0
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };

    let mut pid_files = Vec::new();
    for input in &args.inputs {
        collect_pids(input, &mut pid_files);
    }
    if pid_files.is_empty() {
        eprintln!("error: no .pid files found under inputs");
        std::process::exit(2);
    }

    if let Some(ref plots_dir) = args.plots_dir {
        if let Err(err) = std::fs::create_dir_all(plots_dir) {
            eprintln!(
                "error: cannot create plots directory {}: {err}",
                plots_dir.display()
            );
            std::process::exit(2);
        }
    }

    let t_warm = Instant::now();
    scene::text::lff::warm();
    let _ = scene::text::sysfont::families();
    scene::text::ttf_glyph::warm_font_system();
    let warmup_ms = round2(t_warm.elapsed().as_secs_f64() * 1000.0);

    let mut sheets = Vec::with_capacity(pid_files.len());
    let mut load_failures = 0usize;
    let mut plot_failures = 0usize;

    for (idx, path) in pid_files.iter().enumerate() {
        let sheet_id = format!("S{:02}", idx + 1);
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let pid_sha256_prefix = sha256_prefix_12(path);

        let t_total = Instant::now();
        let t_import = Instant::now();
        let loaded = io::load_file(path);
        let cad_import_ms = round2(t_import.elapsed().as_secs_f64() * 1000.0);

        let (load_ok, scene_prepare_ms, total_open_ms, cad_entities, load_error) = match loaded {
            Ok(mut doc) => {
                let t_prep = Instant::now();
                io::linetypes::repair_document(&mut doc);
                if let Some(base_dir) = path.parent() {
                    let _ = io::xref::resolve_xrefs_with_progress(&mut doc, base_dir, None);
                }
                let mut caches =
                    scene::build_derived_caches_with_progress(&doc, &|_: u16| {}, path.parent());
                let entities = doc.entity_count();
                let (_doc, _geometry, _prepare) =
                    scene::prepare_open_geometry(doc, &mut caches, [0.0; 4]);
                let prep_ms = round2(t_prep.elapsed().as_secs_f64() * 1000.0);
                let tot_ms = round2(t_total.elapsed().as_secs_f64() * 1000.0);
                (true, prep_ms, tot_ms, entities, String::new())
            }
            Err(err) => {
                load_failures += 1;
                (false, 0.0, cad_import_ms, 0, err)
            }
        };

        let tr = audit_text_runs(path);

        let (plot_ok, plot_ms, pdf_bytes, primitives, on_page, plot_error) =
            if let Some(ref plots_dir) = args.plots_dir {
                let out_pdf = plots_dir.join(format!("{stem}.pdf"));
                let t_plot = Instant::now();
                match io::pid_pdf::plot_pdf(path, &out_pdf) {
                    Ok(receipt) => (
                        true,
                        round2(t_plot.elapsed().as_secs_f64() * 1000.0),
                        receipt.bytes,
                        receipt.primitives,
                        receipt.ledger.on_page,
                        String::new(),
                    ),
                    Err(err) => {
                        plot_failures += 1;
                        (
                            false,
                            round2(t_plot.elapsed().as_secs_f64() * 1000.0),
                            0,
                            0,
                            0,
                            err,
                        )
                    }
                }
            } else {
                (true, 0.0, 0, 0, 0, String::new())
            };

        eprintln!(
            "[{sheet_id}/{total:02}] load={load_ok} open={total_open_ms:.1}ms (import={cad_import_ms:.1}ms prep={scene_prepare_ms:.1}ms) plot={plot_ok} ({plot_ms:.1}ms) flat={flat}",
            total = pid_files.len(),
            flat = tr.flattened,
        );

        sheets.push(serde_json::json!({
            "sheet_id": sheet_id,
            "stem": stem,
            "pid_sha256_prefix": pid_sha256_prefix,
            "load_ok": load_ok,
            "load_error": load_error,
            "cad_import_ms": cad_import_ms,
            "scene_prepare_ms": scene_prepare_ms,
            "total_open_ms": total_open_ms,
            "cad_entities": cad_entities,
            "plot_ok": plot_ok,
            "plot_error": plot_error,
            "plot_ms": plot_ms,
            "pdf_bytes": pdf_bytes,
            "primitives": primitives,
            "on_page": on_page,
            "text_runs": {
                "total_texts": tr.total_texts,
                "uniform": tr.uniform,
                "no_runs": tr.no_runs,
                "flattened": tr.flattened,
                "unresolvable": tr.unresolvable,
                "length_mismatch": tr.length_mismatch,
            },
        }));
    }

    let report = serde_json::json!({
        "debug_assertions": cfg!(debug_assertions),
        "warmup_ms": warmup_ms,
        "sheets": sheets,
    });

    if let Some(parent) = args.out_json.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = serde_json::to_string_pretty(&report).expect("serialize bench report");
    std::fs::write(&args.out_json, format!("{payload}\n")).expect("write bench report");

    println!(
        "pid_l1_bench: sheets={} load_failures={} plot_failures={} warmup_ms={warmup_ms:.1}",
        pid_files.len(),
        load_failures,
        plot_failures
    );
    if load_failures > 0 || plot_failures > 0 {
        std::process::exit(1);
    }
}
