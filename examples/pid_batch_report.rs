//! Import every `.pid` under the given directories and report, one CSV row
//! per drawing, what the import drew, what it could not draw, and what it
//! assumed -- the numbers a thin-looking sheet is read against, for a whole
//! corpus at once (plan 2026-09-24-pid-import-status-and-next-steps, B1).
//!
//! ```text
//! cargo run --example pid_batch_report -- <dir-or-file>... [--export-dir <dir>] > report.csv
//! ```
//!
//! The summary columns are `load_pid`'s own summary and the refusal columns
//! the parse the importer runs, so a row says what the editor's command line
//! and log would. With `--export-dir` each drawing is also opened the way the
//! editor opens it (`io::load_file`) and saved as DXF the way `--export`
//! saves it, so `dxf_sha256` compares with a headless export byte for byte.
//! A `.pid` that is not a compound document -- a process-id file beside a
//! server -- is skipped, and a drawing that fails or panics gets a row saying
//! so instead of stopping the run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};
use OpenCADStudio::io;
use OpenCADStudio::io::pid::{load_pid, ImportUnit};

const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

const HEADER: [&str; 21] = [
    "file",
    "status",
    "import_ms",
    "drawn",
    "decoded",
    "missing",
    "refused",
    "undecoded",
    "unit",
    "page_mm",
    "cache_bodies",
    "library_bodies",
    "hidden_strokes",
    "sheet_layer_names",
    "sheet_layers_off",
    "layered_entities",
    "unresolved_layers",
    "lettering_flattened",
    "style_tables_failed",
    "dxf_sha256",
    "error",
];

fn is_compound_file(path: &Path) -> bool {
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && head == CFB_MAGIC
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(|entry| Some(entry.ok()?.path()))
            .collect();
        children.sort();
        for child in children {
            collect(&child, out);
        }
    } else if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pid"))
        && is_compound_file(path)
    {
        out.push(path.to_path_buf());
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic without a message".to_string())
}

/// `type:count` per graphic type code, the census's two lists.
fn by_type<'a>(records: impl Iterator<Item = (u16, usize)> + 'a) -> String {
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for (type_code, count) in records {
        *counts.entry(type_code).or_default() += count;
    }
    counts
        .iter()
        .map(|(type_code, count)| format!("0x{type_code:04X}:{count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Refused and undecoded graphic records by type, and the page size, off the
/// same parse `load_pid` runs.
fn breakdown(path: &Path) -> (String, String, String) {
    let Ok(parsed) =
        pid_parse::PidParser::with_options(pid_parse::ParseOptions::geometry()).parse_file(path)
    else {
        return ("?".into(), "?".into(), "?".into());
    };
    let geometry = pid_parse::build_normalized_geometry(&parsed);
    let refused = by_type(
        geometry
            .refused_graphic_records
            .iter()
            .map(|record| (record.type_code, record.count)),
    );
    let undecoded = by_type(
        geometry
            .dropped_graphic_records
            .iter()
            .map(|record| (record.type_code, record.count)),
    );
    let page = geometry
        .page_dimensions_mm
        .map_or_else(|| "-".to_string(), |(w, h)| format!("{w:.0}x{h:.0}"));
    (refused, undecoded, page)
}

/// Open the drawing as the editor does, save it as DXF as `--export` does,
/// and hash what was written.
fn export(path: &Path, index: usize, dir: &Path) -> String {
    let doc = match io::load_file(path) {
        Ok(doc) => doc,
        Err(_) => return "load-failed".to_string(),
    };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let out = dir.join(format!("{index:03}-{stem}.dxf"));
    if io::save(&doc, &out).is_err() {
        return "save-failed".to_string();
    }
    std::fs::read(&out)
        .map(|bytes| format!("{:X}", Sha256::digest(&bytes)))
        .unwrap_or_else(|_| "read-failed".to_string())
}

fn report(path: &Path, name: &str, index: usize, export_dir: Option<&Path>) -> Vec<String> {
    let started = Instant::now();
    let outcome = std::panic::catch_unwind(|| load_pid(path));
    let import_ms = started.elapsed().as_millis().to_string();
    let failure = |status: &str, error: String| {
        let mut row = vec![String::new(); HEADER.len()];
        row[0] = name.to_string();
        row[1] = status.to_string();
        row[2] = import_ms.clone();
        row[HEADER.len() - 1] = error;
        row
    };
    let summary = match outcome {
        Ok(Ok(import)) => import.summary,
        Ok(Err(error)) => return failure("failed", error),
        Err(panic) => return failure("panicked", panic_message(&*panic)),
    };
    let (refused, undecoded, page) = breakdown(path);
    let dxf_sha256 = export_dir
        .map(|dir| export(path, index, dir))
        .unwrap_or_default();
    let unit = match &summary.unit {
        ImportUnit::Stated { unit, .. } => unit.clone(),
        ImportUnit::AssumedMetre => "assumed-metre".to_string(),
    };
    vec![
        name.to_string(),
        "ok".to_string(),
        import_ms,
        summary.drawn.to_string(),
        summary.decoded.to_string(),
        summary.missing.to_string(),
        refused,
        undecoded,
        unit,
        page,
        summary.cache_bodies.to_string(),
        summary.library_bodies.to_string(),
        summary.hidden_strokes_skipped.to_string(),
        summary.sheet_layer_names.to_string(),
        summary.sheet_layers_off.to_string(),
        summary.layered_entities.to_string(),
        summary.unresolved_sheet_layers.to_string(),
        summary.lettering_flattened.to_string(),
        summary.style_tables_failed.to_string(),
        dxf_sha256,
        String::new(),
    ]
}

fn main() {
    let mut inputs = Vec::new();
    let mut export_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--export-dir" {
            export_dir = args.next().map(PathBuf::from);
            continue;
        }
        inputs.push(PathBuf::from(arg));
    }
    if inputs.is_empty() {
        eprintln!("usage: pid_batch_report <dir-or-file>... [--export-dir <dir>]");
        std::process::exit(2);
    }
    // Each row names its drawing relative to the input it was found under, so
    // a report compares across machines and checkouts.
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for input in &inputs {
        let mut found = Vec::new();
        collect(input, &mut found);
        for path in found {
            let name = path
                .strip_prefix(input)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(path.file_name().unwrap_or_default()))
                .to_string_lossy()
                .replace('\\', "/");
            files.push((path, name));
        }
    }
    if let Some(dir) = &export_dir {
        if let Err(error) = std::fs::create_dir_all(dir) {
            eprintln!("cannot create {}: {error}", dir.display());
            std::process::exit(2);
        }
    }

    println!("{}", HEADER.join(","));
    let mut failed = 0usize;
    for (index, (path, name)) in files.iter().enumerate() {
        let row = report(path, name, index, export_dir.as_deref());
        if row[1] != "ok" {
            failed += 1;
        }
        println!(
            "{}",
            row.iter()
                .map(|cell| csv_field(cell))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    eprintln!(
        "{} drawing(s): {} imported, {failed} failed",
        files.len(),
        files.len() - failed
    );
}
