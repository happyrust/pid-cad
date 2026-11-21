#!/usr/bin/env python3
"""Build Round-2 5-slot bisection submission package (V2-01..V2-04) and LX-4 E1..E3 SPPID probes (#144).

Implements the Round-2 narrowing matrix in `04-external-platform-protocol.md` §4
and the `LX-4` SPPID acceptance probe suite in `智能PID平台-阶段一Issue清单.md`:

Round-2 (`package_kind = "round2_bisect"`):
- `00-control` (`R2-CTRL-00`): unmodified positive control triple;
- `01-v2-cfb-repack` (`R2-V2-01`): original XML + stream-lossless `PidWriter` CFB-repacked `.pid`;
- `02-v2-single-vessel-seed` (`R2-V2-02`): blank template seed + 1 vessel (`V-1001`) minimal dual bundle;
- `03-v2-piping-no-signal` (`R2-V2-03`): retains full `Piping*` network while stripping `Signal*` items/rels;
- `04-v2-leaf-codelist-only` (`R2-V2-04`): retains leaf type codelist while clearing upper `LEAF_LEVELS` parents.

LX-4 (`package_kind = "lx4_acceptance"`, optional `--lx4-out-dir`):
- `E1-byte-identical` (`LX4-E1`): byte-for-byte identical copy of passed `.pid`;
- `E2-metadata-stamp` (`LX4-E2`): single summary metadata property modified on passed `.pid`;
- `E3-foreign-single-vessel` (`LX4-E3`): brand-new un-registered single-vessel `.pid` from blank template.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
from typing import Any, Sequence

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from build_submission_converted import (  # noqa: E402
    _anonymize_xml_stem,
    _find_single_converted_bundle,
)
from build_submission_round1 import (  # noqa: E402
    ATTR_PAIR_RE,
    _encode_vendor_blocks,
    _extract_block_attrs,
    _parse_vendor_blocks,
    pick_smallest_complete_triple,
)
import xml_spec_check  # noqa: E402

SIGNAL_CLASSES_TO_DROP: frozenset[str] = frozenset(
    {
        "PIDSignalConnector",
        "PIDSignalPort",
        "PIDSignalBranchPoint",
    }
)

LEVEL_KEY_RE = re.compile(r"^(EqType|PipingComponentType|InstrumentType)(\d+)$")


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


def make_v2_03_piping_no_signal(data_bytes: bytes) -> bytes:
    decl, open_cont, blocks, close_cont = _parse_vendor_blocks(data_bytes)

    dropped_signal_uids: set[str] = set()
    for tag, blk in blocks:
        if tag in SIGNAL_CLASSES_TO_DROP:
            attrs = _extract_block_attrs(blk)
            uid = attrs.get(("IObject", "UID"), "")
            if uid:
                dropped_signal_uids.add(uid)

    dropped_rep_uids: set[str] = set()
    for tag, blk in blocks:
        if tag == "Rel":
            attrs = _extract_block_attrs(blk)
            if (
                attrs.get(("IRel", "DefUID")) == "DrawingItems"
                and attrs.get(("IRel", "UID2"), "") in dropped_signal_uids
            ):
                rep_uid = attrs.get(("IRel", "UID1"), "")
                if rep_uid:
                    dropped_rep_uids.add(rep_uid)

    all_dropped = dropped_signal_uids | dropped_rep_uids
    kept_blocks: list[tuple[str, list[str]]] = []
    for tag, blk in blocks:
        if tag in SIGNAL_CLASSES_TO_DROP:
            continue
        attrs = _extract_block_attrs(blk)
        uid = attrs.get(("IObject", "UID"), "")
        if uid in all_dropped:
            continue
        if tag == "Rel":
            def_uid = attrs.get(("IRel", "DefUID"), "")
            u1 = attrs.get(("IRel", "UID1"), "")
            u2 = attrs.get(("IRel", "UID2"), "")
            if def_uid.startswith("Signal") or u1 in all_dropped or u2 in all_dropped:
                continue
        kept_blocks.append((tag, blk))

    return _encode_vendor_blocks(decl, open_cont, kept_blocks, close_cont)


def make_v2_04_leaf_codelist_only(data_bytes: bytes) -> bytes:
    decl, open_cont, blocks, close_cont = _parse_vendor_blocks(data_bytes)
    new_blocks: list[tuple[str, list[str]]] = []

    for tag, blk in blocks:
        if tag in ("PIDDrawing", "PIDRepresentation", "Rel"):
            new_blocks.append((tag, blk))
            continue

        updated = [blk[0]]
        for ln in blk[1:-1]:
            max_by_pfx: dict[str, int] = {}
            for m in ATTR_PAIR_RE.finditer(ln):
                k, val = m.group(1), m.group(3)
                if not val:
                    continue
                km = LEVEL_KEY_RE.match(k)
                if km:
                    pfx, idx = km.group(1), int(km.group(2))
                    if idx > max_by_pfx.get(pfx, -1):
                        max_by_pfx[pfx] = idx

            if not max_by_pfx:
                updated.append(ln)
                continue

            def _clear_upper(m: re.Match[str], _max_map: dict[str, int] = max_by_pfx) -> str:
                k, val = m.group(1), m.group(3)
                km = LEVEL_KEY_RE.match(k)
                if km and val:
                    pfx, idx = km.group(1), int(km.group(2))
                    if idx < _max_map.get(pfx, -1):
                        return f'{k}=""'
                return m.group(0)

            updated.append(ATTR_PAIR_RE.sub(_clear_upper, ln))
        updated.append(blk[-1])
        new_blocks.append((tag, updated))

    return _encode_vendor_blocks(decl, open_cont, new_blocks, close_cont)


def _write_slot_triple(
    out_dir: Path,
    slot_name: str,
    deliv_stem: str,
    orig_stem: str,
    pid_raw: bytes,
    data_raw: bytes,
    meta_raw: bytes,
    spec: dict[str, Any],
    plant: str,
    keep_original_stem: bool,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
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

    vd = xml_spec_check.check_xml_bytes(
        data_f.name, "data", data_raw, spec, strict=True, expected_plant=plant
    )
    vm = xml_spec_check.check_xml_bytes(
        meta_f.name, "meta", meta_raw, spec, strict=True, expected_plant=plant
    )
    strict_cnt = len(vd) + len(vm)

    entry: dict[str, Any] = {
        "slot": slot_name,
        "delivery_stem": deliv_stem,
        "pid_sha256_prefix": _sha256_bytes(pid_raw)[:12],
        "strict_vendor_violations": strict_cnt,
        "slot_precheck_passed": strict_cnt == 0,
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
    if extra:
        entry.update(extra)
    return entry


def build_round2_and_lx4_packages(
    *,
    source_dir: Path,
    repacked_pid: Path,
    single_vessel_bundle: Path,
    out_dir: Path,
    lx4_out_dir: Path | None = None,
    e2_stamped_pid: Path | None = None,
    plant: str = "P01",
    keep_original_stem: bool = False,
) -> tuple[int, dict[str, Any], dict[str, Any] | None]:
    spec = xml_spec_check.load_spec()
    ctrl_stem, pid_src, data_src, meta_src = pick_smallest_complete_triple(source_dir)
    ctrl_pid = pid_src.read_bytes()
    ctrl_data = data_src.read_bytes()
    ctrl_meta = meta_src.read_bytes()

    if not repacked_pid.is_file():
        raise FileNotFoundError(f"repacked .pid not found: {repacked_pid}")
    v201_pid = repacked_pid.read_bytes()

    sv_stem, sv_pid, sv_data, sv_meta, sv_cov = _find_single_converted_bundle(single_vessel_bundle)
    sv_pver = sv_cov.get("pid_verification") or {}
    sv_a_ok = bool(sv_pver.get("a_level_index_consistent", False)) and (
        int(sv_pver.get("matched_graphic_oids", 0)) == int(sv_pver.get("xml_rep_count", 0)) > 0
    )

    v203_data = make_v2_03_piping_no_signal(ctrl_data)
    v204_data = make_v2_04_leaf_codelist_only(ctrl_data)

    out_dir.mkdir(parents=True, exist_ok=True)
    slots: list[dict[str, Any]] = []

    slots.append(
        _write_slot_triple(
            out_dir,
            "00-control",
            ctrl_stem if keep_original_stem else "R2-CTRL-00",
            ctrl_stem,
            ctrl_pid,
            ctrl_data,
            ctrl_meta,
            spec,
            plant,
            keep_original_stem,
            {"probe": "positive_control"},
        )
    )
    slots.append(
        _write_slot_triple(
            out_dir,
            "01-v2-cfb-repack",
            ctrl_stem if keep_original_stem else "R2-V2-01",
            ctrl_stem,
            v201_pid,
            ctrl_data,
            ctrl_meta,
            spec,
            plant,
            keep_original_stem,
            {"probe": "B1_cfb_repack_accepted"},
        )
    )
    s_v202 = _write_slot_triple(
        out_dir,
        "02-v2-single-vessel-seed",
        sv_stem if keep_original_stem else "R2-V2-02",
        sv_stem,
        sv_pid,
        sv_data,
        sv_meta,
        spec,
        plant,
        keep_original_stem,
        {
            "probe": "B2_seed_plus_one_vessel_accepted",
            "a_level_valid": sv_a_ok,
        },
    )
    if not sv_a_ok:
        s_v202["slot_precheck_passed"] = False
    slots.append(s_v202)

    slots.append(
        _write_slot_triple(
            out_dir,
            "03-v2-piping-no-signal",
            ctrl_stem if keep_original_stem else "R2-V2-03",
            ctrl_stem,
            ctrl_pid,
            v203_data,
            ctrl_meta,
            spec,
            plant,
            keep_original_stem,
            {"probe": "B3_signal_topology_required"},
        )
    )
    slots.append(
        _write_slot_triple(
            out_dir,
            "04-v2-leaf-codelist-only",
            ctrl_stem if keep_original_stem else "R2-V2-04",
            ctrl_stem,
            ctrl_pid,
            v204_data,
            ctrl_meta,
            spec,
            plant,
            keep_original_stem,
            {"probe": "B4_upper_leaf_levels_required"},
        )
    )

    r2_ok = all(bool(s["slot_precheck_passed"]) for s in slots)
    r2_sha = _sha256_bytes(
        json.dumps(slots, sort_keys=True, separators=(",", ":")).encode("utf-8")
    )
    r2_manifest = {
        "schema_version": 1,
        "package_kind": "round2_bisect",
        "round": 2,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "plant": plant,
        "source_control_pid_sha256_prefix": _sha256_bytes(ctrl_pid)[:12],
        "manifest_sha256": r2_sha,
        "sppid_c1_d_precheck_status": "unverified_no_sppid_runner",
        "precheck_passed": r2_ok,
        "slots": slots,
    }
    (out_dir / "submission-manifest.json").write_text(
        json.dumps(r2_manifest, indent=2) + "\n", encoding="utf-8"
    )
    r2_receipt = {
        "package_kind": "round2_bisect",
        "round": 2,
        "submitted_at": "",
        "returned_at": "",
        "manifest_sha256": r2_sha,
        "results": [{"slot": s["slot"], "status": "pending", "error_message": ""} for s in slots],
    }
    (out_dir / "receipt.json").write_text(
        json.dumps(r2_receipt, indent=2) + "\n", encoding="utf-8"
    )

    lx4_manifest: dict[str, Any] | None = None
    if lx4_out_dir is not None:
        if e2_stamped_pid is None or not e2_stamped_pid.is_file():
            raise FileNotFoundError("e2_stamped_pid is required when --lx4-out-dir is specified")
        e2_raw = e2_stamped_pid.read_bytes()
        lx4_out_dir.mkdir(parents=True, exist_ok=True)
        lx4_slots = [
            _write_slot_triple(
                lx4_out_dir,
                "E1-byte-identical",
                ctrl_stem if keep_original_stem else "LX4-E1",
                ctrl_stem,
                ctrl_pid,
                ctrl_data,
                ctrl_meta,
                spec,
                plant,
                keep_original_stem,
                {"experiment": "E1_byte_identical_copy"},
            ),
            _write_slot_triple(
                lx4_out_dir,
                "E2-metadata-stamp",
                ctrl_stem if keep_original_stem else "LX4-E2",
                ctrl_stem,
                e2_raw,
                ctrl_data,
                ctrl_meta,
                spec,
                plant,
                keep_original_stem,
                {"experiment": "E2_single_metadata_modified"},
            ),
            _write_slot_triple(
                lx4_out_dir,
                "E3-foreign-single-vessel",
                sv_stem if keep_original_stem else "LX4-E3",
                sv_stem,
                sv_pid,
                sv_data,
                sv_meta,
                spec,
                plant,
                keep_original_stem,
                {"experiment": "E3_foreign_unregistered_pid"},
            ),
        ]
        lx4_ok = all(bool(s["slot_precheck_passed"]) for s in lx4_slots)
        lx4_sha = _sha256_bytes(
            json.dumps(lx4_slots, sort_keys=True, separators=(",", ":")).encode("utf-8")
        )
        lx4_manifest = {
            "schema_version": 1,
            "package_kind": "lx4_acceptance",
            "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "plant": plant,
            "manifest_sha256": lx4_sha,
            "precheck_passed": lx4_ok,
            "slots": lx4_slots,
        }
        (lx4_out_dir / "submission-manifest.json").write_text(
            json.dumps(lx4_manifest, indent=2) + "\n", encoding="utf-8"
        )
        lx4_receipt = {
            "package_kind": "lx4_acceptance",
            "submitted_at": "",
            "returned_at": "",
            "manifest_sha256": lx4_sha,
            "results": [
                {"slot": s["slot"], "status": "pending", "error_message": ""} for s in lx4_slots
            ],
        }
        (lx4_out_dir / "receipt.json").write_text(
            json.dumps(lx4_receipt, indent=2) + "\n", encoding="utf-8"
        )
        r2_ok = r2_ok and lx4_ok

    return (0 if r2_ok else 1), r2_manifest, lx4_manifest


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="build_submission_round2.py",
        description="Build Round-2 bisection package (V2-01..04) and LX-4 E1..E3 SPPID probes (#144).",
    )
    p.add_argument("--source-dir", required=True, type=Path)
    p.add_argument("--repacked-pid", required=True, type=Path)
    p.add_argument("--single-vessel-bundle", required=True, type=Path)
    p.add_argument("--out-dir", required=True, type=Path)
    p.add_argument("--lx4-out-dir", default=None, type=Path)
    p.add_argument("--e2-stamped-pid", default=None, type=Path)
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
        rc, r2_manifest, lx4_manifest = build_round2_and_lx4_packages(
            source_dir=args.source_dir,
            repacked_pid=args.repacked_pid,
            single_vessel_bundle=args.single_vessel_bundle,
            out_dir=args.out_dir,
            lx4_out_dir=args.lx4_out_dir,
            e2_stamped_pid=args.e2_stamped_pid,
            plant=args.plant,
            keep_original_stem=args.keep_original_stem,
        )
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    lx4_info = (
        f"\tlx4_sha256={lx4_manifest['manifest_sha256'][:16]}" if lx4_manifest is not None else ""
    )
    print(
        f"ROUND2_SUBMISSION\tslots={len(r2_manifest['slots'])}\tprecheck_passed={r2_manifest['precheck_passed']}\tmanifest_sha256={r2_manifest['manifest_sha256'][:16]}{lx4_info}"
    )
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
