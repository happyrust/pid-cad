//! Headless single-sheet and batch DWG/DXF-to-SmartPlant publish XML + `.pid`
//! dual converter (`L4-6` / `L5-6` / `L5-8` / `L6-1` / `#127` / `#130`).
//!
//! ```text
//! cargo run --example pid_convert -- <input.dxf|input.dwg|dir> --out <dir>
//!     [--plant NAME]              Plant name in Container header (default: P01)
//!     [--seed-pid PATH_OR_ZIP]    Blank .pid seed, RefData~*.zip, or zip::entry for dual output
//!     [--sym-dir DIR]             Optional Ref/Symbols root for cloning real .sym JSite bodies
//!     [--verify-plot]             Also run b-level CAD reload + pid-plot PDF export (<stem>.pidplot.pdf)
//!     [--summary-json FILE]       Optional path to write BatchConvertSummary JSON
//! ```

use std::path::PathBuf;

use OpenCADStudio::io::pid_convert::{convert_path_to_dir, ConvertOptions};

struct CliArgs {
    input: PathBuf,
    out_dir: PathBuf,
    plant: String,
    seed_pid: Option<String>,
    sym_dir: Option<String>,
    verify_plot: bool,
    summary_json: Option<PathBuf>,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut plant = "P01".to_string();
    let mut seed_pid: Option<String> = None;
    let mut sym_dir: Option<String> = None;
    let mut verify_plot = false;
    let mut summary_json: Option<PathBuf> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => {
                out_dir = Some(PathBuf::from(
                    it.next().ok_or("--out requires a directory")?,
                ));
            }
            "--plant" => {
                plant = it.next().ok_or("--plant requires a value")?;
            }
            "--seed-pid" => {
                seed_pid = Some(it.next().ok_or("--seed-pid requires a path or zip spec")?);
            }
            "--sym-dir" => {
                sym_dir = Some(it.next().ok_or("--sym-dir requires a directory")?);
            }
            "--verify-plot" => {
                verify_plot = true;
            }
            "--summary-json" => {
                summary_json = Some(PathBuf::from(
                    it.next().ok_or("--summary-json requires a file path")?,
                ));
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option '{other}'"));
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected positional argument '{other}'"));
                }
                input = Some(PathBuf::from(other));
            }
        }
    }

    let input = input.ok_or(
        "usage: pid_convert <input.dxf|input.dwg|dir> --out <dir> [--plant NAME] [--seed-pid PATH_OR_ZIP] [--sym-dir DIR] [--verify-plot] [--summary-json FILE]",
    )?;
    let out_dir = out_dir.ok_or("--out <dir> is required")?;
    Ok(CliArgs {
        input,
        out_dir,
        plant,
        seed_pid,
        sym_dir,
        verify_plot,
        summary_json,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };

    let opts = ConvertOptions {
        plant: args.plant,
        seed_pid_path: args.seed_pid,
        sym_library_dir: args.sym_dir,
        verify_plot_pdf: args.verify_plot,
        ..Default::default()
    };

    match convert_path_to_dir(&args.input, &args.out_dir, &opts) {
        Ok(summary) => {
            if let Some(ref sjson) = args.summary_json {
                if let Some(parent) = sjson.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(txt) = serde_json::to_string_pretty(&summary) {
                    let _ = std::fs::write(sjson, format!("{txt}\n"));
                }
            }
            for err in &summary.errors {
                eprintln!("convert error: {err}");
            }
            println!(
                "pid_convert: converted={} failed={} pid_written={} pdf_plotted={} items={} relations={}",
                summary.files_converted,
                summary.files_failed,
                summary.pid_files_written,
                summary.pdf_files_plotted,
                summary.total_items_written,
                summary.total_relations_written
            );
            if summary.files_converted == 0 || summary.files_failed > 0 {
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
