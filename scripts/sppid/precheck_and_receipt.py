#!/usr/bin/env python3
"""Three-gate pre-submission precheck orchestrator (LX-8) and subtraction receipt evaluator (LX-9).

Subcommands:
1. `precheck`:
   - Gate 1 (Static XML spec + optional reference diff): runs `xml_spec_check` (`StrictVendor`)
     on every `<stem>_Data.xml` and `<stem>_Meta.xml` in `--bundle-dir`.
   - Gate 2 (SPPID native `c1` open): invokes `<runner> open --in <bundle> --out <work/open> --timeout <sec>`
     and verifies `open-result.json` reports `opened == true` for every `.pid`.
   - Gate 3 (SPPID `d` re-publish reconciliation): invokes `<runner> publish --in <bundle> --out <work/publish> --timeout <sec>`
     and reconciles re-published `<stem>_Data.xml` against our exported `<stem>_Data.xml` via `xml_diff`.
   - When no runner is configured (`--runner` omitted and `OCS_PID_SPPID_RUNNER` unset), records
     `sppid_c1_d_precheck_status = "unverified_no_sppid_runner"`, sets `ready_for_external_submission = False`,
     and exits with code `2` unless `--allow-unverified-sppid` is explicitly passed.

2. `evaluate-receipt`:
   - Verifies `receipt.json` against `submission-manifest.json` (`manifest_sha256`, slot completeness,
     and `00-control` positive control acceptance).
   - Deterministically infers external platform requirements `R1..R5` and generates the Round-2
     narrowing action list (`04-external-platform-protocol.md` §4).
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

import xml_diff  # noqa: E402
import xml_spec_check  # noqa: E402


def _extract_runner_files(payload: Any) -> list[dict[str, Any]]:
    if isinstance(payload, list):
        return [x for x in payload if isinstance(x, dict)]
    if isinstance(payload, dict) and isinstance(payload.get("files"), list):
        return [x for x in payload["files"] if isinstance(x, dict)]
    return []


def run_precheck(
    bundle_dir: Path,
    *,
    plant: str | None = None,
    runner_path: str | None = None,
    timeout_sec: int = 180,
    allow_unverified_sppid: bool = False,
) -> tuple[int, dict[str, Any]]:
    spec = xml_spec_check.load_spec()
    data_files = sorted(bundle_dir.glob("*_Data.xml"))
    pid_files = sorted(bundle_dir.glob("*.pid"))

    gate1_errors: list[str] = []
    if not data_files:
        gate1_errors.append("no *_Data.xml files found in bundle_dir")
    for df in data_files:
        stem = df.name[:-9]
        mf = bundle_dir / f"{stem}_Meta.xml"
        vd = xml_spec_check.check_xml_bytes(
            df.name, "data", df.read_bytes(), spec, strict=True, expected_plant=plant
        )
        gate1_errors.extend(v.format_line() for v in vd)
        if not mf.is_file():
            gate1_errors.append(f"{stem}: missing {mf.name}")
        else:
            vm = xml_spec_check.check_xml_bytes(
                mf.name, "meta", mf.read_bytes(), spec, strict=True, expected_plant=plant
            )
            gate1_errors.extend(v.format_line() for v in vm)

    gate1_passed = len(data_files) > 0 and len(gate1_errors) == 0
    gate1 = {
        "sheets_checked": len(data_files),
        "violations": len(gate1_errors),
        "passed": gate1_passed,
        "errors": gate1_errors[:50],
    }

    eff_runner = (runner_path or os.environ.get("OCS_PID_SPPID_RUNNER") or "").strip()
    if not eff_runner:
        report = {
            "gate1_xml_spec": gate1,
            "gate2_sppid_open": {
                "status": "unverified_no_sppid_runner",
                "passed": None,
                "pid_count": len(pid_files),
            },
            "gate3_sppid_publish_diff": {
                "status": "unverified_no_sppid_runner",
                "passed": None,
                "total_diffs": None,
            },
            "sppid_c1_d_precheck_status": "unverified_no_sppid_runner",
            "ready_for_external_submission": False,
        }
        if not gate1_passed:
            return 1, report
        return (0 if allow_unverified_sppid else 2), report

    with tempfile.TemporaryDirectory(prefix="ocs_sppid_precheck_") as tmp_str:
        tmp_root = Path(tmp_str)
        open_out = tmp_root / "open"
        pub_out = tmp_root / "publish"
        open_out.mkdir(parents=True, exist_ok=True)
        pub_out.mkdir(parents=True, exist_ok=True)

        # Gate 2: <runner> open --in <bundle> --out <open_out> --timeout <sec>
        open_cmd = [
            eff_runner,
            "open",
            "--in",
            str(bundle_dir),
            "--out",
            str(open_out),
            "--timeout",
            str(timeout_sec),
        ]
        open_proc = subprocess.run(open_cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        open_res_file = open_out / "open-result.json"
        open_errors: list[str] = []
        opened_ok = 0
        if open_proc.returncode != 0:
            open_errors.append(f"runner open exited with {open_proc.returncode}: {open_proc.stderr.strip()}")
        if not open_res_file.is_file():
            open_errors.append("runner open produced no open-result.json")
        else:
            entries = _extract_runner_files(json.loads(open_res_file.read_text(encoding="utf-8")))
            by_name = {str(e.get("file")): e for e in entries}
            for pf in pid_files:
                rec = by_name.get(pf.name)
                if rec and rec.get("opened") is True:
                    opened_ok += 1
                else:
                    msg = rec.get("message", "missing entry") if rec else "missing entry"
                    open_errors.append(f"{pf.name}: open failed ({msg})")

        gate2_passed = len(open_errors) == 0 and opened_ok == len(pid_files)
        gate2 = {
            "status": "executed",
            "pid_count": len(pid_files),
            "opened_count": opened_ok,
            "passed": gate2_passed,
            "errors": open_errors,
        }

        # Gate 3: <runner> publish --in <bundle> --out <pub_out> --timeout <sec>
        pub_cmd = [
            eff_runner,
            "publish",
            "--in",
            str(bundle_dir),
            "--out",
            str(pub_out),
            "--timeout",
            str(timeout_sec),
        ]
        pub_proc = subprocess.run(pub_cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        pub_errors: list[str] = []
        total_diffs = 0
        if pub_proc.returncode != 0:
            pub_errors.append(f"runner publish exited with {pub_proc.returncode}: {pub_proc.stderr.strip()}")
        for pf in pid_files:
            stem = pf.stem
            our_data = bundle_dir / f"{stem}_Data.xml"
            sppid_data = pub_out / f"{stem}_Data.xml"
            if not sppid_data.is_file():
                pub_errors.append(f"{stem}: runner publish produced no {sppid_data.name}")
                continue
            if our_data.is_file():
                diffs = xml_diff.diff_data_docs(
                    xml_diff.parse_data_xml(sppid_data),
                    xml_diff.parse_data_xml(our_data),
                )
                total_diffs += len(diffs)
                for d in diffs[:25]:
                    pub_errors.append(d.to_tsv())

        gate3_passed = len(pub_errors) == 0 and total_diffs == 0
        gate3 = {
            "status": "executed",
            "total_diffs": total_diffs,
            "passed": gate3_passed,
            "errors": pub_errors[:50],
        }

    all_passed = gate1_passed and gate2_passed and gate3_passed
    report = {
        "gate1_xml_spec": gate1,
        "gate2_sppid_open": gate2,
        "gate3_sppid_publish_diff": gate3,
        "sppid_c1_d_precheck_status": "verified_passed" if (gate2_passed and gate3_passed) else "verified_failed",
        "ready_for_external_submission": all_passed,
    }
    return (0 if all_passed else 1), report


def _slot_status_lower(row: dict[str, Any] | None) -> str:
    if not row:
        return "pending"
    if "status" in row and row["status"] is not None and str(row["status"]).strip():
        return str(row["status"]).strip().lower()
    if isinstance(row.get("accepted"), bool):
        return "accepted" if row["accepted"] else "rejected"
    return "pending"


def evaluate_receipt(manifest_path: Path, receipt_path: Path) -> tuple[int, dict[str, Any]]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))

    expected_sha = manifest.get("manifest_sha256")
    actual_sha = receipt.get("manifest_sha256")
    if expected_sha and actual_sha != expected_sha:
        return 1, {
            "channel_valid": False,
            "reason": f"manifest_sha256 mismatch: expected {expected_sha}, got {actual_sha}",
        }

    results = {str(r.get("slot")): r for r in (receipt.get("results") or []) if isinstance(r, dict)}
    expected_slots = [str(s["slot"]) for s in (manifest.get("slots") or [])]

    pending_slots = [sl for sl in expected_slots if _slot_status_lower(results.get(sl)) == "pending"]
    if pending_slots:
        return 2, {
            "channel_valid": None,
            "pending_slots": pending_slots,
            "reason": "receipt still has pending slots",
        }

    if manifest.get("package_kind") == "converted_pilot":
        arch_acc: dict[str, bool] = {}
        slot_ok_map: dict[str, bool] = {}
        next_actions: list[str] = []
        for s in manifest.get("slots") or []:
            sl = str(s["slot"])
            akey = str(s.get("archetype_key", sl))
            ok_slot = _slot_status_lower(results.get(sl)) == "accepted"
            arch_acc[akey] = ok_slot
            slot_ok_map[sl] = ok_slot
            if not ok_slot:
                next_actions.append(f"investigate-{sl}")
        diag_hints: list[str] = []
        if (
            "01-sppid-dxf-direct" in slot_ok_map
            and "07-sppid-dxf-external-props" in slot_ok_map
            and (not slot_ok_map["01-sppid-dxf-direct"])
            and slot_ok_map["07-sppid-dxf-external-props"]
        ):
            diag_hints.append("external_schedule_props_required_for_dxf")
        if (
            "06-pid-inapp-edit-save" in slot_ok_map
            and "08-bare-pid-inapp-rebuild" in slot_ok_map
            and slot_ok_map["06-pid-inapp-edit-save"]
            and (not slot_ok_map["08-bare-pid-inapp-rebuild"])
        ):
            diag_hints.append("companion_xml_or_schedule_needed_for_bare_pid")
        if (
            "02-legacy-block-circle-direct" in slot_ok_map
            and "05-binary-dwg-direct" in slot_ok_map
            and slot_ok_map["02-legacy-block-circle-direct"]
            and (not slot_ok_map["05-binary-dwg-direct"])
        ):
            diag_hints.append("binary_dwg_entity_decode_divergence")
        all_ok = bool(arch_acc) and all(arch_acc.values())
        if all_ok:
            next_actions.append("phase1-external-acceptance-complete")
        return 0, {
            "channel_valid": True,
            "package_kind": "converted_pilot",
            "manifest_sha256": expected_sha,
            "all_archetypes_accepted": all_ok,
            "archetype_acceptance": arch_acc,
            "diagnostic_hints": diag_hints,
            "next_round_actions": next_actions,
            "slot_statuses": {sl: _slot_status_lower(results.get(sl)) for sl in expected_slots if sl in results},
        }

    if manifest.get("package_kind") == "round2_bisect":
        ctrl_st = _slot_status_lower(results.get("00-control"))
        if ctrl_st != "accepted":
            return 1, {
                "channel_valid": False,
                "package_kind": "round2_bisect",
                "control_status": ctrl_st,
                "reason": "00-control positive baseline was not accepted in Round-2",
            }
        b_conc = {
            "B1_cfb_repack_accepted": _slot_status_lower(results.get("01-v2-cfb-repack")) == "accepted",
            "B2_seed_plus_one_vessel_accepted": _slot_status_lower(results.get("02-v2-single-vessel-seed")) == "accepted",
            "B3_signal_topology_required": _slot_status_lower(results.get("03-v2-piping-no-signal")) != "accepted",
            "B4_upper_leaf_levels_required": _slot_status_lower(results.get("04-v2-leaf-codelist-only")) != "accepted",
        }
        r2_next: list[str] = []
        if not b_conc["B1_cfb_repack_accepted"]:
            r2_next.append("investigate-cfb-directory-order")
        if not b_conc["B2_seed_plus_one_vessel_accepted"]:
            r2_next.append("investigate-seed-increment-streams")
        if not r2_next:
            r2_next.append("proceed-to-converted-pilot-submission")
        return 0, {
            "channel_valid": True,
            "package_kind": "round2_bisect",
            "manifest_sha256": expected_sha,
            "bisect_conclusions": b_conc,
            "next_round_actions": r2_next,
            "slot_statuses": {sl: _slot_status_lower(results.get(sl)) for sl in expected_slots if sl in results},
        }

    if manifest.get("package_kind") == "lx4_acceptance":
        e1_ok = _slot_status_lower(results.get("E1-byte-identical")) == "accepted"
        e2_ok = _slot_status_lower(results.get("E2-metadata-stamp")) == "accepted"
        e3_ok = _slot_status_lower(results.get("E3-foreign-single-vessel")) == "accepted"
        if not e1_ok:
            return 1, {
                "channel_valid": False,
                "package_kind": "lx4_acceptance",
                "reason": "E1 byte-identical copy failed to open; SPPID runner or project environment invalid",
            }
        lx4_conc = {
            "e1_byte_identical_opened": e1_ok,
            "e2_modified_metadata_opened": e2_ok,
            "c1_foreign_pid_accepted": e3_ok,
            "checksum_or_stamp_enforced": e1_ok and (not e2_ok),
            "plant_db_registration_required": e2_ok and (not e3_ok),
        }
        return 0, {
            "channel_valid": True,
            "package_kind": "lx4_acceptance",
            "manifest_sha256": expected_sha,
            "lx4_conclusions": lx4_conc,
            "slot_statuses": {sl: _slot_status_lower(results.get(sl)) for sl in expected_slots if sl in results},
        }

    ctrl_status = _slot_status_lower(results.get("00-control"))
    if ctrl_status != "accepted":
        return 1, {
            "channel_valid": False,
            "control_status": ctrl_status,
            "reason": "00-control positive baseline was not accepted; submission channel/environment invalid",
        }

    def is_accepted(slot_name: str) -> bool:
        return _slot_status_lower(results.get(slot_name)) == "accepted"

    inferred = {
        "pid_binary_required": not is_accepted("01-var-xml-only"),
        "piping_topology_required": not is_accepted("02-var-no-topology"),
        "extended_attributes_required": not is_accepted("03-var-minimal-attrs"),
        "strict_vendor_whitespace_required": not is_accepted("04-var-plain-xml-format"),
        "synthetic_uids_accepted": is_accepted("05-var-synthetic-uids"),
    }

    next_actions: list[str] = []
    if inferred["piping_topology_required"]:
        next_actions.append("V2-topology-split")
    if inferred["extended_attributes_required"]:
        next_actions.append("V2-attrs-bisect")
    if not inferred["synthetic_uids_accepted"]:
        next_actions.append("V2-uid-format-probe")
    if not next_actions:
        next_actions.append("proceed-to-converted-dxf-pilot-submission")

    summary = {
        "channel_valid": True,
        "manifest_sha256": expected_sha,
        "inferred_requirements": inferred,
        "next_round_actions": next_actions,
        "slot_statuses": {sl: _slot_status_lower(results.get(sl)) for sl in expected_slots if sl in results},
    }
    return 0, summary


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="precheck_and_receipt.py",
        description="Run three-gate SPPID prechecks (LX-8) or evaluate external platform receipts (LX-9).",
    )
    sub = p.add_subparsers(dest="subcmd", required=True)

    p_pre = sub.add_parser("precheck", help="Run Gate 1 (XML) + Gate 2 (SPPID open) + Gate 3 (SPPID publish diff)")
    p_pre.add_argument("--bundle-dir", required=True, help="Directory holding .pid + _Data.xml + _Meta.xml")
    p_pre.add_argument("--plant", default=None, help="Expected Plant attribute")
    p_pre.add_argument("--runner", default=None, help="Override OCS_PID_SPPID_RUNNER executable")
    p_pre.add_argument("--timeout", type=int, default=180, help="Per-step runner timeout in seconds")
    p_pre.add_argument(
        "--allow-unverified-sppid",
        action="store_true",
        help="Exit 0 on Gate 1 pass when no SPPID runner is configured (marks unverified_no_sppid_runner)",
    )
    p_pre.add_argument("--out-json", default=None, help="Optional JSON output path")

    p_ev = sub.add_parser("evaluate-receipt", help="Validate receipt.json and derive R1..R5 + Round-2 plan")
    p_ev.add_argument("--manifest", required=True, help="Path to submission-manifest.json")
    p_ev.add_argument("--receipt", required=True, help="Path to receipt.json")
    p_ev.add_argument("--out-json", default=None, help="Optional JSON output path")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    if args.subcmd == "precheck":
        bdir = Path(args.bundle_dir)
        if not bdir.is_dir():
            print(f"error: bundle-dir does not exist: {bdir}", file=sys.stderr)
            return 2
        rc, rep = run_precheck(
            bdir,
            plant=args.plant,
            runner_path=args.runner,
            timeout_sec=args.timeout,
            allow_unverified_sppid=args.allow_unverified_sppid,
        )
        if args.out_json:
            Path(args.out_json).write_text(json.dumps(rep, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(
            f"PRECHECK\tgate1={rep['gate1_xml_spec']['passed']}\tsppid={rep['sppid_c1_d_precheck_status']}\tready={rep['ready_for_external_submission']}"
        )
        return rc

    if args.subcmd == "evaluate-receipt":
        mp = Path(args.manifest)
        rp = Path(args.receipt)
        if not mp.is_file() or not rp.is_file():
            print("error: --manifest and --receipt must point to existing JSON files", file=sys.stderr)
            return 2
        rc, ev = evaluate_receipt(mp, rp)
        if args.out_json:
            Path(args.out_json).write_text(json.dumps(ev, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"RECEIPT_EVAL\trc={rc}\tchannel_valid={ev.get('channel_valid')}")
        return rc

    return 2


if __name__ == "__main__":
    raise SystemExit(main())
