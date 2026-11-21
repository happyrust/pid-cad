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

