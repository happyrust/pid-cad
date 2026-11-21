#!/usr/bin/env python3
"""Build Round-1B 5-slot converted DWG/DXF pilot submission package (LX-8 / LX-9 / #140).

Packages five representative real converted SmartPlant P&ID bundles produced by
OpenCADStudio's Phase-1 pipeline so external digital delivery testing validates
all source archetypes alongside Round-1 subtraction probes:

- `01-sppid-dxf-direct` (`CONV-01`): SPPID-exported DXF -> `pid_convert` dual bundle;
- `02-legacy-block-circle-direct` (`CONV-02`): Native AutoCAD block + circle-bubble DXF -> `pid_convert`;
- `03-legacy-exploded-shape-direct` (`CONV-03`): Native AutoCAD exploded stroke-shape DXF -> `pid_convert`;
- `04-legacy-inapp-pidsavepid` (`CONV-04`): Native AutoCAD sheet -> `PIDPROMOTE -> PIDREVIEW -> PIDSAVEPID`;
- `05-binary-dwg-direct` (`CONV-05`): Genuine binary `.dwg` (`AC10*` magic verified) -> `pid_convert`.

Validates every slot against `StrictVendor` XML spec (`0` violations), `.pid` `a/b`-level
`_Coverage.json` verification (`matched_graphic_oids == xml_rep_count`, `a_level_index_consistent`),
and zero unmapped fallback (`class_default_fallback == 0`), emitting strictly relative-path,
customer-name-free `submission-manifest.json` and `receipt.json`.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from typing import Any, Sequence

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import xml_spec_check  # noqa: E402

BOM = b"\xef\xbb\xbf"

SLOT_DEFS: tuple[tuple[str, str, str, str], ...] = (
    (
        "01-sppid-dxf-direct",
        "CONV-01",
        "C1_sppid_dxf_accepted",
        "SPPID-exported split DXF converted via pid_convert dual exporter",
    ),
    (
        "02-legacy-block-circle-direct",
        "CONV-02",
        "C2_legacy_block_circle_accepted",
        "Native AutoCAD TWT-block + circle-bubble legacy sheet converted via pid_convert",
    ),
    (
        "03-legacy-exploded-shape-direct",
        "CONV-03",
        "C3_legacy_exploded_shape_accepted",
        "Native AutoCAD exploded stroke-shape legacy sheet converted via pid_convert",
    ),
    (
        "04-legacy-inapp-pidsavepid",
        "CONV-04",
        "C4_inapp_pidsavepid_accepted",
        "Native AutoCAD sheet promoted/reviewed/saved in-app via PIDPROMOTE -> PIDREVIEW -> PIDSAVEPID",
    ),
    (
        "05-binary-dwg-direct",
        "CONV-05",
        "C5_binary_dwg_accepted",
        "Binary AutoCAD .dwg (AC10*) converted directly via pid_convert dual exporter",
    ),
)


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _find_single_converted_bundle(src_dir: Path) -> tuple[str, bytes, bytes, bytes, dict[str, Any]]:
    pids = sorted(src_dir.glob("*.pid"))
    if not pids:
        raise ValueError(f"no .pid file found in {src_dir}")
    pid_p = pids[0]
    stem = pid_p.stem
    data_p = src_dir / f"{stem}_Data.xml"
    meta_p = src_dir / f"{stem}_Meta.xml"
    cov_p = src_dir / f"{stem}_Coverage.json"
    for req in (data_p, meta_p, cov_p):
        if not req.is_file():
            raise ValueError(f"missing required companion file {req.name} in {src_dir}")
    cov = json.loads(cov_p.read_text(encoding="utf-8"))
    return stem, pid_p.read_bytes(), data_p.read_bytes(), meta_p.read_bytes(), cov


def _anonymize_xml_stem(xml_bytes: bytes, old_stem: str, new_stem: str) -> bytes:
    if not old_stem or old_stem == new_stem:
        return xml_bytes
    has_bom = xml_bytes.startswith(BOM)
    body = xml_bytes[3:] if has_bom else xml_bytes
    text = body.decode("utf-8")
    for suffix in ("_Data.xml", "_Meta.xml", ".pid", ""):
        for attr in ("Name", "DocTitle", "DocName", "Description"):
            text = text.replace(f'{attr}="{old_stem}{suffix}"', f'{attr}="{new_stem}{suffix}"')
    encoded = text.encode("utf-8")
    return (BOM + encoded) if has_bom else encoded


def _inspect_binary_dwg_header(dwg_path: Path) -> str:
    if not dwg_path.is_file():
        raise ValueError(f"binary .dwg source does not exist: {dwg_path}")
    head = dwg_path.read_bytes()[:6]
    if len(head) < 6 or not head.startswith(b"AC10"):
        raise ValueError(f"file is not a valid binary AutoCAD .dwg (magic={head!r})")
    return head.decode("ascii", errors="replace")


def build_converted_submission_package(
    bundle_dirs: Sequence[Path],
    *,
    binary_dwg_source: Path,
    out_dir: Path,
    plant: str = "P01",
    keep_original_stem: bool = False,
) -> tuple[int, dict[str, Any]]:
    if len(bundle_dirs) != len(SLOT_DEFS):
        raise ValueError(f"expected {len(SLOT_DEFS)} bundle directories, got {len(bundle_dirs)}")

    dwg_magic = _inspect_binary_dwg_header(binary_dwg_source)
    dwg_sha_pfx = _sha256_bytes(binary_dwg_source.read_bytes())[:12]

    spec = xml_spec_check.load_spec()
    out_dir.mkdir(parents=True, exist_ok=True)

    slot_reports: list[dict[str, Any]] = []
    precheck_ok = True

    for idx, ((slot_name, anon_stem, arch_key, desc), bdir) in enumerate(
        zip(SLOT_DEFS, bundle_dirs)
    ):
        orig_stem, pid_raw, data_raw, meta_raw, cov = _find_single_converted_bundle(bdir)
        deliv_stem = orig_stem if keep_original_stem else anon_stem
        if not keep_original_stem:
            data_raw = _anonymize_xml_stem(data_raw, orig_stem, deliv_stem)
            meta_raw = _anonymize_xml_stem(meta_raw, orig_stem, deliv_stem)

        s_dir = out_dir / slot_name
        s_dir.mkdir(parents=True, exist_ok=True)

        pid_f = s_dir / f"{deliv_stem}.pid"
        data_f = s_dir / f"{deliv_stem}_Data.xml"
        meta_f = s_dir / f"{deliv_stem}_Meta.xml"
        pid_f.write_bytes(pid_raw)
        data_f.write_bytes(data_raw)
        meta_f.write_bytes(meta_raw)

        vd_strict = xml_spec_check.check_xml_bytes(
            data_f.name, "data", data_raw, spec, strict=True, expected_plant=plant
        )
        vm_strict = xml_spec_check.check_xml_bytes(
            meta_f.name, "meta", meta_raw, spec, strict=True, expected_plant=plant
        )
        strict_cnt = len(vd_strict) + len(vm_strict)

        var_res = cov.get("variant_resolution") or {}
        fallback_cnt = int(var_res.get("class_default_fallback", 0)) + int(
            var_res.get("unmapped_class_variant", 0)
        )
        exact_cnt = int(var_res.get("exact_variant_id", 0)) + int(
            var_res.get("symbol_path_match", 0)
        )

        pver = cov.get("pid_verification") or {}
        xml_reps = int(pver.get("xml_rep_count", 0))
        matched_oids = int(pver.get("matched_graphic_oids", 0))
        idx_ok = bool(pver.get("a_level_index_consistent", False))
        a_valid = idx_ok and xml_reps > 0 and (xml_reps == matched_oids)
        b_valid = (int(pver.get("b_level_pdf_bytes") or 0) > 0) or (
            bdir / f"{orig_stem}.pidplot.pdf"
        ).is_file()

        slot_ok = (strict_cnt == 0) and (fallback_cnt == 0) and a_valid
        if not slot_ok:
            precheck_ok = False

        entry: dict[str, Any] = {
            "slot": slot_name,
            "archetype_key": arch_key,
            "description": desc,
            "delivery_stem": deliv_stem,
            "source_pid_sha256_prefix": _sha256_bytes(pid_raw)[:12],
            "strict_vendor_violations": strict_cnt,
            "exact_variants": exact_cnt,
            "class_default_fallback": fallback_cnt,
            "xml_rep_count": xml_reps,
            "matched_graphic_oids": matched_oids,
            "a_level_valid": a_valid,
            "b_level_plot_valid": b_valid,
            "slot_precheck_passed": slot_ok,
            "files": [
                {
                    "path": f"{slot_name}/{pid_f.name}",
                    "bytes": len(pid_raw),
                    "sha256": _sha256_bytes(pid_raw),
                },
                {
                    "path": f"{slot_name}/{data_f.name}",
                    "bytes": len(data_raw),
                    "sha256": _sha256_bytes(data_raw),
                },
                {
                    "path": f"{slot_name}/{meta_f.name}",
                    "bytes": len(meta_raw),
                    "sha256": _sha256_bytes(meta_raw),
                },
            ],
        }
        if idx == 4:
            entry["binary_dwg_header"] = dwg_magic
            entry["binary_dwg_sha256_prefix"] = dwg_sha_pfx
        slot_reports.append(entry)

    canonical_slots = json.dumps(slot_reports, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )
    manifest_sha256 = _sha256_bytes(canonical_slots)

    manifest = {
        "schema_version": 1,
        "package_kind": "converted_pilot",
        "round": "1B",
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "plant": plant,
        "manifest_sha256": manifest_sha256,
        "sppid_c1_d_precheck_status": "unverified_no_sppid_runner",
        "precheck_passed": precheck_ok,
        "slots": slot_reports,
    }
    (out_dir / "submission-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )

    receipt_tpl = {
        "package_kind": "converted_pilot",
        "round": "1B",
        "submitted_at": "",
        "returned_at": "",
        "manifest_sha256": manifest_sha256,
        "results": [
            {
                "slot": s["slot"],
                "status": "pending",
                "error_message": "",
            }
            for s in slot_reports
        ],
    }
    (out_dir / "receipt.json").write_text(
        json.dumps(receipt_tpl, indent=2) + "\n", encoding="utf-8"
    )
    return (0 if precheck_ok else 1), manifest


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="build_submission_converted.py",
        description="Build 5-slot converted DWG/DXF pilot submission package (LX-8 / LX-9 / #140).",
    )
    p.add_argument("--sppid-dxf-bundle", required=True, type=Path)
    p.add_argument("--legacy-block-bundle", required=True, type=Path)
    p.add_argument("--legacy-shape-bundle", required=True, type=Path)
    p.add_argument("--legacy-inapp-bundle", required=True, type=Path)
    p.add_argument("--binary-dwg-bundle", required=True, type=Path)
    p.add_argument("--binary-dwg-source", required=True, type=Path)
    p.add_argument("--out-dir", required=True, type=Path)
    p.add_argument("--plant", default="P01")
    p.add_argument("--keep-original-stem", action="store_true")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    try:
        rc, manifest = build_converted_submission_package(
            [
                args.sppid_dxf_bundle,
                args.legacy_block_bundle,
                args.legacy_shape_bundle,
                args.legacy_inapp_bundle,
                args.binary_dwg_bundle,
            ],
            binary_dwg_source=args.binary_dwg_source,
            out_dir=args.out_dir,
            plant=args.plant,
            keep_original_stem=args.keep_original_stem,
        )
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    print(
        f"CONVERTED_SUBMISSION\tslots={len(manifest['slots'])}\tprecheck_passed={manifest['precheck_passed']}\tmanifest_sha256={manifest['manifest_sha256'][:16]}"
    )
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
