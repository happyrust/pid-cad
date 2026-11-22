//! Headless single-sheet and batch DWG/DXF-to-SmartPlant publish XML + `.pid`
//! dual converter (`L4-6` / `L5-6` / `L5-8` / `L6-1` / `#127` / `#130`).
//!
//! ```text
//! cargo run --example pid_convert -- <input.dxf|input.dwg|dir> --out <dir>
//!     [--plant NAME]              Plant name in Container header (default: P01)
//!     [--seed-pid PATH_OR_ZIP]    Blank .pid seed, RefData~*.zip, or zip::entry for dual output
//!     [--sym-dir DIR]             Optional Ref/Symbols root for cloning real .sym JSite bodies
//!     [--class-map FILE]          Optional external SPPID class-map JSON override (#117 / #137)
//!     [--external-props FILE]     Optional external Equipment/Line schedule (.xlsx/.csv/.tsv/.json, #145)
//!     [--verify-plot]             Also run b-level CAD reload + pid-plot PDF export (<stem>.pidplot.pdf)
//!     [--summary-json FILE]       Optional path to write BatchConvertSummary JSON
//! ```

use std::path::PathBuf;

use OpenCADStudio::io::pid_convert::{
    convert_path_to_dir, diff_pid_files, export_handover_package_from_path,
    extract_bom_from_batch_dir, extract_hazop_from_path, extract_loops_from_batch_dir,
    extract_nozzles_from_path, extract_utilities_from_path, extract_valves_from_path,
    save_cad_file_as_binary_dwg, write_bom_schedule_file, write_pid_diff_file,
    write_pid_hazop_file, write_pid_loops_file, write_pid_nozzle_file, write_pid_utility_file,
    write_pid_valve_file, ConvertOptions, PackageOptions,
};

struct CliArgs {
    input: PathBuf,
    out_dir: Option<PathBuf>,
    diff_with: Option<PathBuf>,
    diff_out: Option<PathBuf>,
    plant: String,
    seed_pid: Option<String>,
    sym_dir: Option<String>,
    class_map: Option<String>,
    external_props: Option<String>,
    export_schedule: Option<PathBuf>,
    export_loops: Option<PathBuf>,
    export_hazop: Option<PathBuf>,
    export_utilities: Option<PathBuf>,
    export_valves: Option<PathBuf>,
    export_nozzles: Option<PathBuf>,
    export_package: Option<PathBuf>,
    strict_gate: bool,
    save_dwg: Option<PathBuf>,
    verify_plot: bool,
    summary_json: Option<PathBuf>,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut plant = "P01".to_string();
    let mut seed_pid: Option<String> = None;
    let mut sym_dir: Option<String> = None;
    let mut class_map: Option<String> = None;
    let mut external_props: Option<String> = None;
    let mut export_schedule: Option<PathBuf> = None;
    let mut export_loops: Option<PathBuf> = None;
    let mut export_hazop: Option<PathBuf> = None;
    let mut export_utilities: Option<PathBuf> = None;
    let mut export_valves: Option<PathBuf> = None;
    let mut export_nozzles: Option<PathBuf> = None;
    let mut export_package: Option<PathBuf> = None;
    let mut strict_gate = false;
    let mut save_dwg: Option<PathBuf> = None;
    let mut verify_plot = false;
    let mut summary_json: Option<PathBuf> = None;
    let mut diff_with: Option<PathBuf> = None;
    let mut diff_out: Option<PathBuf> = None;

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
            "--class-map" => {
                class_map = Some(it.next().ok_or("--class-map requires a JSON file path")?);
            }
            "--external-props" => {
                external_props = Some(it.next().ok_or(
                    "--external-props requires a schedule file path (.xlsx/.csv/.tsv/.json)",
                )?);
            }
            "--export-schedule" => {
                export_schedule = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-schedule requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-loops" => {
                export_loops = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-loops requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-hazop" => {
                export_hazop = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-hazop requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-utilities" => {
                export_utilities = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-utilities requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-valves" => {
                export_valves = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-valves requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-nozzles" => {
                export_nozzles = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-nozzles requires an output path (.csv/.tsv/.json)")?,
                ));
            }
            "--export-package" => {
                export_package = Some(PathBuf::from(
                    it.next()
                        .ok_or("--export-package requires an output directory")?,
                ));
            }
            "--strict-gate" => {
                strict_gate = true;
            }
            "--save-dwg" => {
                save_dwg = Some(PathBuf::from(
                    it.next().ok_or("--save-dwg requires an output .dwg path")?,
                ));
            }
            "--verify-plot" => {
                verify_plot = true;
            }
            "--summary-json" => {
                summary_json = Some(PathBuf::from(
                    it.next().ok_or("--summary-json requires a file path")?,
                ));
            }
            "--diff-with" => {
                diff_with = Some(PathBuf::from(
                    it.next()
                        .ok_or("--diff-with requires a baseline CAD/PID file path")?,
                ));
            }
            "--diff-out" => {
                diff_out = Some(PathBuf::from(
                    it.next()
                        .ok_or("--diff-out requires an output .json/.csv/.tsv path")?,
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
    if out_dir.is_none()
        && diff_with.is_none()
        && export_package.is_none()
        && export_hazop.is_none()
        && export_utilities.is_none()
        && export_valves.is_none()
        && export_nozzles.is_none()
    {
        return Err(
            "--out <dir> (or --diff-with <baseline> or --export-package <dir>) is required"
                .to_string(),
        );
    }
    Ok(CliArgs {
        input,
        out_dir,
        diff_with,
        diff_out,
        plant,
        seed_pid,
        sym_dir,
        class_map,
        external_props,
        export_schedule,
        export_loops,
        export_hazop,
        export_utilities,
        export_valves,
        export_nozzles,
        export_package,
        strict_gate,
        save_dwg,
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

    let effective_input = if let Some(ref dwg_out) = args.save_dwg {
        if let Err(err) = save_cad_file_as_binary_dwg(&args.input, dwg_out) {
            eprintln!("error: save-dwg failed: {err}");
            std::process::exit(1);
        }
        dwg_out.clone()
    } else {
        args.input.clone()
    };

    let opts = ConvertOptions {
        plant: args.plant,
        seed_pid_path: args.seed_pid,
        sym_library_dir: args.sym_dir,
        class_map_path: args.class_map,
        external_props_path: args.external_props,
        verify_plot_pdf: args.verify_plot,
        ..Default::default()
    };

    if let Some(ref nz_out) = args.export_nozzles {
        match extract_nozzles_from_path(&effective_input)
            .and_then(|rep| write_pid_nozzle_file(&rep, nz_out).map(|()| rep))
        {
            Ok(rep) => {
                println!(
                    "pid_nozzle: sheets={} equipment={} total_nozzles={} connected={} spare={} orphan={}",
                    rep.summary.sheets_scanned,
                    rep.summary.equipment_with_nozzles,
                    rep.summary.total_nozzles,
                    rep.summary.connected_nozzles,
                    rep.summary.spare_nozzles,
                    rep.summary.orphan_nozzles,
                );
                if args.out_dir.is_none()
                    && args.diff_with.is_none()
                    && args.export_package.is_none()
                    && args.export_hazop.is_none()
                    && args.export_utilities.is_none()
                    && args.export_valves.is_none()
                {
                    return;
                }
            }
            Err(err) => {
                eprintln!("error: export-nozzles failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref vm_out) = args.export_valves {
        match extract_valves_from_path(&effective_input)
            .and_then(|rep| write_pid_valve_file(&rep, vm_out).map(|()| rep))
        {
            Ok(rep) => {
                println!(
                    "pid_valve: sheets={} total_valves={} resolved={} unresolved={} mto_groups={}",
                    rep.summary.sheets_scanned,
                    rep.summary.total_valves,
                    rep.summary.spec_resolved_valves,
                    rep.summary.unresolved_valves,
                    rep.summary.mto_groups_count,
                );
                if args.out_dir.is_none()
                    && args.diff_with.is_none()
                    && args.export_package.is_none()
                    && args.export_hazop.is_none()
                    && args.export_utilities.is_none()
                {
                    return;
                }
            }
            Err(err) => {
                eprintln!("error: export-valves failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref ut_out) = args.export_utilities {
        match extract_utilities_from_path(&effective_input)
            .and_then(|rep| write_pid_utility_file(&rep, ut_out).map(|()| rep))
        {
            Ok(rep) => {
                println!(
                    "pid_utility: sheets={} services={} utility_services={} pipelines={} battery_limits={}",
                    rep.summary.sheets_scanned,
                    rep.summary.total_services,
                    rep.summary.utility_services_count,
                    rep.summary.labeled_pipelines_count,
                    rep.summary.battery_limit_count,
                );
                if args.out_dir.is_none()
                    && args.diff_with.is_none()
                    && args.export_package.is_none()
                    && args.export_hazop.is_none()
                {
                    return;
                }
            }
            Err(err) => {
                eprintln!("error: export-utilities failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref hz_out) = args.export_hazop {
        match extract_hazop_from_path(&effective_input)
            .and_then(|rep| write_pid_hazop_file(&rep, hz_out).map(|()| rep))
        {
            Ok(rep) => {
                println!(
                    "pid_hazop: sheets={} nodes={} multi_sheet={} protected={} unprotected={} pipelines={} safeguards={}",
                    rep.summary.sheets_scanned,
                    rep.summary.total_nodes,
                    rep.summary.multi_sheet_nodes,
                    rep.summary.protected_nodes,
                    rep.summary.unprotected_nodes,
                    rep.summary.total_linked_pipelines,
                    rep.summary.total_linked_safeguards,
                );
                if args.out_dir.is_none()
                    && args.diff_with.is_none()
                    && args.export_package.is_none()
                {
                    return;
                }
            }
            Err(err) => {
                eprintln!("error: export-hazop failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref pkg_dir) = args.export_package {
        let pkg_opts = PackageOptions {
            convert: opts.clone(),
            verify_strict: true,
            strict_gate: args.strict_gate,
        };
        match export_handover_package_from_path(&effective_input, pkg_dir, &pkg_opts) {
            Ok(man) => {
                println!(
                    "pid_package: schema=v{} handover_ready={} sheets={} pid={} pdf={} valves={} bl={} hazop={} files={} manifest_sha256={}",
                    man.schema_version,
                    man.handover_ready,
                    man.sheets_exported,
                    man.pid_files_written,
                    man.pdf_files_plotted,
                    man.total_valves,
                    man.battery_limit_interfaces,
                    man.hazop_nodes_count,
                    man.files.len(),
                    man.manifest_sha256,
                );
                if args.out_dir.is_none() && args.diff_with.is_none() {
                    return;
                }
            }
            Err(err) => {
                eprintln!("error: export-package failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if let Some(ref base_p) = args.diff_with {
        let rep = match diff_pid_files(base_p, &effective_input, &opts) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("error: diff-with failed: {err}");
                std::process::exit(1);
            }
        };
        if let Some(ref do_p) = args.diff_out {
            if let Err(err) = write_pid_diff_file(&rep, do_p) {
                eprintln!("error: diff-out failed: {err}");
                std::process::exit(1);
            }
        }
        println!(
            "pid_diff: total_changes={} added={} modified={} removed={} unchanged={}",
            rep.summary.total_changes,
            rep.summary.added_count,
            rep.summary.modified_count,
            rep.summary.removed_count,
            rep.summary.unchanged_count,
        );
        if args.out_dir.is_none() {
            return;
        }
    }

    let Some(ref out_dir) = args.out_dir else {
        return;
    };

    match convert_path_to_dir(&effective_input, out_dir, &opts) {
        Ok(summary) => {
            if let Some(ref sjson) = args.summary_json {
                if let Some(parent) = sjson.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(txt) = serde_json::to_string_pretty(&summary) {
                    let _ = std::fs::write(sjson, format!("{txt}\n"));
                }
            }
            if let Some(ref sched_p) = args.export_schedule {
                match extract_bom_from_batch_dir(out_dir)
                    .and_then(|bom| write_bom_schedule_file(&bom, sched_p))
                {
                    Ok(()) => {}
                    Err(err) => {
                        eprintln!("error: export-schedule failed: {err}");
                        std::process::exit(1);
                    }
                }
            }
            if let Some(ref loops_p) = args.export_loops {
                match extract_loops_from_batch_dir(out_dir)
                    .and_then(|lp| write_pid_loops_file(&lp, loops_p))
                {
                    Ok(()) => {}
                    Err(err) => {
                        eprintln!("error: export-loops failed: {err}");
                        std::process::exit(1);
                    }
                }
            }
            for err in &summary.errors {
                eprintln!("convert error: {err}");
            }
            println!(
                "pid_convert: converted={} failed={} pid_written={} pdf_plotted={} items={} relations={} variant_exact={} variant_fallback={}",
                summary.files_converted,
                summary.files_failed,
                summary.pid_files_written,
                summary.pdf_files_plotted,
                summary.total_items_written,
                summary.total_relations_written,
                summary.variant_resolution.exact_variant_id + summary.variant_resolution.symbol_path_match,
                summary.variant_resolution.class_default_fallback + summary.variant_resolution.unmapped_class_variant,
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
