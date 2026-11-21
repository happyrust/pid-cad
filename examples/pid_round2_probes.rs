//! Headless probe generator for Round-2 bisection (`V2-01` CFB repack, `V2-02`
//! single-vessel blank-seed bundle) and `LX-4` (`E2` summary metadata stamp) (`#144`).

use std::path::PathBuf;

use document::entities::{Block, BlockEnd, Insert, Line, Point, Text};
use document::tables::BlockRecord;
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use OpenCADStudio::io::pid_convert::{export_document_to_dir, ConvertOptions};
use OpenCADStudio::io::pid_write::{repack_pid_bytes_lossless, stamp_pid_summary_probe};

struct Args {
    source_pid: PathBuf,
    repacked_out: PathBuf,
    e2_stamped_out: PathBuf,
    seed_zip: String,
    sym_dir: Option<String>,
    single_vessel_out_dir: PathBuf,
    plant: String,
}

fn parse_args() -> Result<Args, String> {
    let mut source_pid: Option<PathBuf> = None;
    let mut repacked_out: Option<PathBuf> = None;
    let mut e2_stamped_out: Option<PathBuf> = None;
    let mut seed_zip: Option<String> = None;
    let mut sym_dir: Option<String> = None;
    let mut single_vessel_out_dir: Option<PathBuf> = None;
    let mut plant = "P01".to_string();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--source-pid" => {
                source_pid = Some(PathBuf::from(it.next().ok_or("--source-pid needs path")?));
            }
            "--repacked-out" => {
                repacked_out = Some(PathBuf::from(it.next().ok_or("--repacked-out needs path")?));
            }
            "--e2-stamped-out" => {
                e2_stamped_out = Some(PathBuf::from(
                    it.next().ok_or("--e2-stamped-out needs path")?,
                ));
            }
            "--seed-zip" => {
                seed_zip = Some(it.next().ok_or("--seed-zip needs path")?);
            }
            "--sym-dir" => {
                sym_dir = Some(it.next().ok_or("--sym-dir needs dir")?);
            }
            "--single-vessel-out-dir" => {
                single_vessel_out_dir = Some(PathBuf::from(
                    it.next().ok_or("--single-vessel-out-dir needs dir")?,
                ));
            }
            "--plant" => {
                plant = it.next().ok_or("--plant needs value")?;
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }

    Ok(Args {
        source_pid: source_pid.ok_or("--source-pid is required")?,
        repacked_out: repacked_out.ok_or("--repacked-out is required")?,
        e2_stamped_out: e2_stamped_out.ok_or("--e2-stamped-out is required")?,
        seed_zip: seed_zip.ok_or("--seed-zip is required")?,
        sym_dir,
        single_vessel_out_dir: single_vessel_out_dir
            .ok_or("--single-vessel-out-dir is required")?,
        plant,
    })
}

fn build_single_vessel_document() -> Document {
    let mut doc = Document::new();
    let blk_name = "VERTICAL DRUM";
    let next = doc.next_handle();
    let br_h = Handle::new(next);
    let b_h = Handle::new(next + 1);
    let e_h = Handle::new(next + 2);

    let mut rec = BlockRecord::new(blk_name);
    rec.handle = br_h;
    rec.block_entity_handle = b_h;
    rec.block_end_handle = e_h;
    doc.block_records.add(rec).unwrap();

    let mut blk = Block::new(blk_name, Vector3::ZERO);
    blk.common.handle = b_h;
    blk.common.owner_handle = br_h;
    doc.add_entity(EntityType::Block(blk)).unwrap();

    let mut end = BlockEnd::new();
    end.common.handle = e_h;
    end.common.owner_handle = br_h;
    doc.add_entity(EntityType::BlockEnd(end)).unwrap();

    for ((x0, y0), (x1, y1)) in [
        ((-6.0, -12.0), (6.0, -12.0)),
        ((6.0, -12.0), (6.0, 12.0)),
        ((6.0, 12.0), (-6.0, 12.0)),
        ((-6.0, 12.0), (-6.0, -12.0)),
    ] {
        let mut ln = Line::from_points(Vector3::new(x0, y0, 0.0), Vector3::new(x1, y1, 0.0));
        ln.common.owner_handle = br_h;
        doc.add_entity(EntityType::Line(ln)).unwrap();
    }
    for py in [-12.0, 12.0] {
        let mut pt = Point::new();
        pt.location = Vector3::new(0.0, py, 0.0);
        pt.common.owner_handle = br_h;
        doc.add_entity(EntityType::Point(pt)).unwrap();
    }

    let mut v_ins = Insert::new(blk_name, Vector3::new(200.0, 150.0, 0.0));
    v_ins.common.layer = "10".to_string();
    doc.add_entity(EntityType::Insert(v_ins)).unwrap();

    let mut v_txt = Text::with_value("V-1001", Vector3::new(195.0, 166.0, 0.0)).with_height(3.0);
    v_txt.common.layer = "10".to_string();
    doc.add_entity(EntityType::Text(v_txt)).unwrap();

    doc
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };

    let raw_src = match std::fs::read(&args.source_pid) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {} failed: {e}", args.source_pid.display());
            std::process::exit(1);
        }
    };

    let (repacked, rt) = match repack_pid_bytes_lossless(&raw_src) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("repack failed: {e}");
            std::process::exit(1);
        }
    };
    if let Some(p) = args.repacked_out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(&args.repacked_out, &repacked).expect("write repacked .pid");

    let e2_bytes = match stamp_pid_summary_probe(&raw_src, &format!("{}-E2", args.plant)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("stamp E2 failed: {e}");
            std::process::exit(1);
        }
    };
    if let Some(p) = args.e2_stamped_out.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(&args.e2_stamped_out, &e2_bytes).expect("write E2 .pid");

    let doc = build_single_vessel_document();
    let opts = ConvertOptions {
        plant: args.plant,
        seed_pid_path: Some(args.seed_zip),
        sym_library_dir: args.sym_dir,
        verify_plot_pdf: true,
        ..Default::default()
    };
    let rep = match export_document_to_dir(
        &doc,
        "SINGLE-VESSEL-01",
        &args.single_vessel_out_dir,
        &opts,
        true,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("single-vessel seed export failed: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "pid_round2_probes: repacked_streams={} repacked_bytes={} e2_bytes={} sv_items={} sv_exact={}",
        rt.stream_count,
        repacked.len(),
        e2_bytes.len(),
        rep.total_items_written,
        rep.variant_resolution.exact_variant_id
    );
}
