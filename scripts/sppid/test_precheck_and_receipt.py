from __future__ import annotations

import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import build_submission_round1  # noqa: E402
import precheck_and_receipt  # noqa: E402
from test_build_submission_round1 import _make_strict_vendor_sample  # noqa: E402


def _write_mock_runner(runner_py: Path, mode: str, bundle_dir: Path) -> Path:
    runner_py.write_text(
        f"""from __future__ import annotations
import json, shutil, sys
from pathlib import Path

MODE = {mode!r}
SRC = Path({str(bundle_dir)!r})

def main() -> int:
    sub = sys.argv[1]
    in_dir = Path(sys.argv[sys.argv.index("--in") + 1])
    out_dir = Path(sys.argv[sys.argv.index("--out") + 1])
    out_dir.mkdir(parents=True, exist_ok=True)
    pids = sorted(in_dir.glob("*.pid"))
    if sub == "open":
        items = []
        for p in pids:
            opened = MODE != "fail_open"
            items.append({{
                "file": p.name,
                "opened": opened,
                "message": "ok" if opened else "E_OPEN_REJECTED",
                "seconds": 0.42,
            }})
        (out_dir / "open-result.json").write_text(json.dumps({{"files": items}}), encoding="utf-8")
        return 0
    if sub == "publish":
        items = []
        for p in pids:
            stem = p.stem
            src_data = SRC / f"{{stem}}_Data.xml"
            src_meta = SRC / f"{{stem}}_Meta.xml"
            if MODE == "tamper_publish":
                raw = src_data.read_bytes().replace(b'Name="E-0201"', b'Name="E-TAMPERED-9999"')
                (out_dir / f"{{stem}}_Data.xml").write_bytes(raw)
            else:
                shutil.copy2(src_data, out_dir / f"{{stem}}_Data.xml")
            shutil.copy2(src_meta, out_dir / f"{{stem}}_Meta.xml")
            items.append({{"file": p.name, "published": True, "seconds": 0.65}})
        (out_dir / "publish-result.json").write_text(json.dumps({{"files": items}}), encoding="utf-8")
        return 0
    return 2

if __name__ == "__main__":
    raise SystemExit(main())
""",
        encoding="utf-8",
    )
    if os.name == "nt":
        cmd = runner_py.with_suffix(".cmd")
        cmd.write_text(f'@echo off\r\n"{sys.executable}" "{runner_py}" %*\r\n', encoding="utf-8")
        return cmd
    runner_py.chmod(0o755)
    return runner_py


def test_precheck_blocks_without_runner_unless_explicitly_allowed(
    tmp_path: Path, monkeypatch
) -> None:
    bundle = tmp_path / "bundle"
    _make_strict_vendor_sample(bundle, stem="SHEET-A01")
    out_report = tmp_path / "precheck.json"
    monkeypatch.delenv("OCS_PID_SPPID_RUNNER", raising=False)

    rc_strict = precheck_and_receipt.main(
        ["precheck", "--bundle-dir", str(bundle), "--plant", "P01", "--out-json", str(out_report)]
    )
    assert rc_strict == 2
    rep_strict = json.loads(out_report.read_text(encoding="utf-8"))
    assert rep_strict["gate1_xml_spec"]["passed"] is True
    assert rep_strict["sppid_c1_d_precheck_status"] == "unverified_no_sppid_runner"
    assert rep_strict["ready_for_external_submission"] is False

    rc_static = precheck_and_receipt.main(
        [
            "precheck",
            "--bundle-dir",
            str(bundle),
            "--plant",
            "P01",
            "--allow-unverified-sppid",
            "--out-json",
            str(out_report),
        ]
    )
    assert rc_static == 0
    rep_static = json.loads(out_report.read_text(encoding="utf-8"))
    assert rep_static["gate1_xml_spec"]["passed"] is True
    assert rep_static["sppid_c1_d_precheck_status"] == "unverified_no_sppid_runner"


def test_precheck_executes_all_three_gates_with_runner_and_catches_open_or_publish_failures(
    tmp_path: Path,
) -> None:
    bundle = tmp_path / "bundle"
    _make_strict_vendor_sample(bundle, stem="SHEET-A02")
    out_report = tmp_path / "precheck_full.json"

    runner_ok = _write_mock_runner(tmp_path / "runner_ok.py", "ok", bundle)
    rc_ok = precheck_and_receipt.main(
        [
            "precheck",
            "--bundle-dir",
            str(bundle),
            "--plant",
            "P01",
            "--runner",
            str(runner_ok),
            "--out-json",
            str(out_report),
        ]
    )
    assert rc_ok == 0
    rep_ok = json.loads(out_report.read_text(encoding="utf-8"))
    assert rep_ok["gate1_xml_spec"]["passed"] is True
    assert rep_ok["gate2_sppid_open"]["passed"] is True
    assert rep_ok["gate3_sppid_publish_diff"]["passed"] is True
    assert rep_ok["ready_for_external_submission"] is True
    assert rep_ok["sppid_c1_d_precheck_status"] == "verified_passed"

    runner_fail_open = _write_mock_runner(tmp_path / "runner_fo.py", "fail_open", bundle)
    rc_fo = precheck_and_receipt.main(
        [
            "precheck",
            "--bundle-dir",
            str(bundle),
            "--plant",
            "P01",
            "--runner",
            str(runner_fail_open),
            "--out-json",
            str(out_report),
        ]
    )
    assert rc_fo == 1
    rep_fo = json.loads(out_report.read_text(encoding="utf-8"))
    assert rep_fo["gate2_sppid_open"]["passed"] is False

    runner_tamper = _write_mock_runner(tmp_path / "runner_tp.py", "tamper_publish", bundle)
    rc_tp = precheck_and_receipt.main(
        [
            "precheck",
            "--bundle-dir",
            str(bundle),
            "--plant",
            "P01",
            "--runner",
            str(runner_tamper),
            "--out-json",
            str(out_report),
        ]
    )
    assert rc_tp == 1
    rep_tp = json.loads(out_report.read_text(encoding="utf-8"))
    assert rep_tp["gate3_sppid_publish_diff"]["passed"] is False
    assert rep_tp["gate3_sppid_publish_diff"]["total_diffs"] > 0


def test_evaluate_receipt_validates_control_and_derives_round2_plan(tmp_path: Path) -> None:
    src_dir = tmp_path / "corpus"
    round1_dir = tmp_path / "round-01"
    _make_strict_vendor_sample(src_dir, stem="CTRL-01")
    assert (
        build_submission_round1.main(
            ["--source-dir", str(src_dir), "--out-dir", str(round1_dir), "--plant", "P01"]
        )
        == 0
    )

    manifest_path = round1_dir / "submission-manifest.json"
    receipt_path = round1_dir / "receipt.json"
    eval_out = tmp_path / "eval.json"

    # Pending receipt -> exit 2 (incomplete)
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_path),
                "--receipt",
                str(receipt_path),
                "--out-json",
                str(eval_out),
            ]
        )
        == 2
    )

    # Fill realistic Round-1 outcomes
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    outcomes = {
        "00-control": ("accepted", ""),
        "01-var-xml-only": ("accepted", ""),
        "02-var-no-topology": ("rejected", "ERR_MISSING_PIPELINE_REL"),
        "03-var-minimal-attrs": ("rejected", "ERR_ATTR_NOMINAL_DIAMETER_EMPTY"),
        "04-var-plain-xml-format": ("accepted", ""),
        "05-var-synthetic-uids": ("accepted", ""),
    }
    for row in receipt["results"]:
        st, err = outcomes[row["slot"]]
        row["status"] = st
        row["error_message"] = err
    receipt["returned_at"] = "2026-11-22T08:00:00Z"
    receipt_path.write_text(json.dumps(receipt, indent=2), encoding="utf-8")

    rc_eval = precheck_and_receipt.main(
        [
            "evaluate-receipt",
            "--manifest",
            str(manifest_path),
            "--receipt",
            str(receipt_path),
            "--out-json",
            str(eval_out),
        ]
    )
    assert rc_eval == 0
    ev = json.loads(eval_out.read_text(encoding="utf-8"))
    assert ev["channel_valid"] is True
    reqs = ev["inferred_requirements"]
    assert reqs["pid_binary_required"] is False
    assert reqs["piping_topology_required"] is True
    assert reqs["extended_attributes_required"] is True
    assert reqs["strict_vendor_whitespace_required"] is False
    assert reqs["synthetic_uids_accepted"] is True
    assert "V2-topology-split" in ev["next_round_actions"]
    assert "V2-attrs-bisect" in ev["next_round_actions"]

    # If 00-control is rejected -> channel invalid (exit 1)
    receipt["results"][0]["status"] = "rejected"
    receipt_path.write_text(json.dumps(receipt, indent=2), encoding="utf-8")
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_path),
                "--receipt",
                str(receipt_path),
                "--out-json",
                str(eval_out),
            ]
        )
        == 1
    )

    # Also supports protocol-style boolean `accepted: true/false` entries on subtraction round-1
    bool_sub_rec = {
        "round": 1,
        "manifest_sha256": json.loads(manifest_path.read_text(encoding="utf-8"))["manifest_sha256"],
        "results": [
            {"slot": "00-control", "accepted": True},
            {"slot": "01-var-xml-only", "accepted": False},
            {"slot": "02-var-no-topology", "accepted": True},
            {"slot": "03-var-minimal-attrs", "accepted": True},
            {"slot": "04-var-plain-xml-format", "accepted": True},
            {"slot": "05-var-synthetic-uids", "accepted": True},
        ],
    }
    receipt_path.write_text(json.dumps(bool_sub_rec, indent=2), encoding="utf-8")
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_path),
                "--receipt",
                str(receipt_path),
                "--out-json",
                str(eval_out),
            ]
        )
        == 0
    )
    ev_bool = json.loads(eval_out.read_text(encoding="utf-8"))
    assert ev_bool["inferred_requirements"]["pid_binary_required"] is True
    assert ev_bool["next_round_actions"] == ["proceed-to-converted-dxf-pilot-submission"]


def test_audit_packages_verifies_multi_package_integrity_and_catches_tampering_or_leaks(
    tmp_path: Path,
) -> None:
    import re
    import build_submission_converted
    from test_build_submission_converted import _make_converted_bundle

    src_dir = tmp_path / "src_passed"
    _make_strict_vendor_sample(src_dir, stem="PASSED-01")

    subm_root = tmp_path / "submissions"
    r1_dir = subm_root / "round-01-test"
    assert (
        build_submission_round1.main(
            ["--source-dir", str(src_dir), "--out-dir", str(r1_dir), "--plant", "P01"]
        )
        == 0
    )

    conv_srcs = []
    for idx in range(1, 7):
        b = tmp_path / f"cb_{idx}"
        _make_converted_bundle(b, f"CONV-SRC-{idx:02d}")
        conv_srcs.append(b)
    dwg_p = tmp_path / "CONV-SRC-05.dwg"
    dwg_p.write_bytes(b"AC1032\x00\x00\x00\x00MOCK")
    r1b_dir = subm_root / "round-01b-test"
    assert (
        build_submission_converted.main(
            [
                "--sppid-dxf-bundle",
                str(conv_srcs[0]),
                "--legacy-block-bundle",
                str(conv_srcs[1]),
                "--legacy-shape-bundle",
                str(conv_srcs[2]),
                "--legacy-inapp-bundle",
                str(conv_srcs[3]),
                "--binary-dwg-bundle",
                str(conv_srcs[4]),
                "--binary-dwg-source",
                str(dwg_p),
                "--pid-inapp-bundle",
                str(conv_srcs[5]),
                "--out-dir",
                str(r1b_dir),
                "--plant",
                "P01",
            ]
        )
        == 0
    )

    summary_p = tmp_path / "delivery-readiness-summary.json"
    rc = precheck_and_receipt.main(
        [
            "audit-packages",
            "--submissions-root",
            str(subm_root),
            "--out-json",
            str(summary_p),
        ]
    )
    assert rc == 0
    raw_sum = summary_p.read_text(encoding="utf-8")
    assert not re.search(r"[A-Za-z]:\\", raw_sum)
    summary = json.loads(raw_sum)
    assert summary["audit_passed"] is True
    assert summary["total_packages"] == 2
    assert summary["total_slots"] == 12
    assert summary["total_files"] == 35
    assert summary["errors"] == []

    # 1-byte tampering on a deliverable file must fail SHA-256 verification
    target_pid = r1b_dir / "01-sppid-dxf-direct" / "CONV-01.pid"
    orig_bytes = target_pid.read_bytes()
    target_pid.write_bytes(orig_bytes[:-1] + bytes([orig_bytes[-1] ^ 0xFF]))
    rc_tamper = precheck_and_receipt.main(
        [
            "audit-packages",
            "--submissions-root",
            str(subm_root),
            "--out-json",
            str(summary_p),
        ]
    )
    assert rc_tamper == 1
    sum_tamper = json.loads(summary_p.read_text(encoding="utf-8"))
    assert sum_tamper["audit_passed"] is False
    assert any("sha256 mismatch" in e.lower() for e in sum_tamper["errors"])
    target_pid.write_bytes(orig_bytes)

    # Host drive path leak inside receipt.json must fail leak check
    rec_p = r1b_dir / "receipt.json"
    orig_rec = rec_p.read_text(encoding="utf-8")
    rec_obj = json.loads(orig_rec)
    rec_obj["results"][0]["error_message"] = "failed reading /home/mockuser/file.pid"
    rec_p.write_text(json.dumps(rec_obj, indent=2), encoding="utf-8")
    rc_leak = precheck_and_receipt.main(
        [
            "audit-packages",
            "--submissions-root",
            str(subm_root),
            "--out-json",
            str(summary_p),
        ]
    )
    assert rc_leak == 1
    sum_leak = json.loads(summary_p.read_text(encoding="utf-8"))
    assert sum_leak["audit_passed"] is False
    assert any("path leak" in e.lower() for e in sum_leak["errors"])
    rec_p.write_text(orig_rec, encoding="utf-8")


