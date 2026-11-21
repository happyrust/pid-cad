from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import precheck_and_receipt  # noqa: E402
from test_build_submission_round1 import (  # noqa: E402
    _make_strict_vendor_sample,
    _render_obj_from_spec,
)
import xml_spec_check  # noqa: E402


def _make_rich_round2_sample(src_dir: Path, stem: str = "SRC-R2") -> tuple[Path, Path]:
    _make_strict_vendor_sample(src_dir, stem=stem)
    spec = xml_spec_check.load_spec()
    data_p = src_dir / f"{stem}_Data.xml"
    raw = data_p.read_bytes()
    has_bom = raw.startswith(b"\xef\xbb\xbf")
    text = (raw[3:] if has_bom else raw).decode("utf-8")

    sport_uid = "22222222333333334444444455555555.2"
    sconn_uid = "77777777777777777777777777777777"
    srep_uid = "77777777777777777777777777777778"
    valve_uid = "88888888888888888888888888888888"
    extra_lines = [
        *_render_obj_from_spec(
            spec,
            "PIDPipingComponent",
            valve_uid,
            name="BV-2001",
            extra_attrs={
                ("IPipingComponent", "PipingComponentType1"): "@{81CD9284-BC07-11D6-BDBC-00104BCC2B69}",
                ("IPipingComponent", "PipingComponentType2"): "@{81CD92D2-BC07-11D6-BDBC-00104BCC2B69}",
                ("IPipingComponent", "PipingComponentType3"): "@{5772FBC0-A395-4FF4-9244-DCDF4809D422}",
            },
        ),
        *_render_obj_from_spec(spec, "PIDSignalPort", sport_uid, name="SP1"),
        *_render_obj_from_spec(spec, "PIDSignalConnector", sconn_uid),
        *_render_obj_from_spec(
            spec,
            "PIDRepresentation",
            srep_uid,
            extra_attrs={("IDrawingRepresentation", "GraphicOID"): "10901"},
        ),
        "   <Rel>",
        f'      <IObject UID="DRI-{srep_uid}-{sconn_uid}" />',
        f'      <IRel UID1="{srep_uid}" UID2="{sconn_uid}" DefUID="DrawingItems" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="SE1-{sport_uid}-{sconn_uid}" />',
        f'      <IRel UID1="{sport_uid}" UID2="{sconn_uid}" DefUID="SignalEnd1Conn" />',
        "   </Rel>",
    ]
    signal_snip = "\r\n".join(extra_lines) + "\r\n"
    text = text.replace("  </Container>", signal_snip + "  </Container>")
    out_bytes = (b"\xef\xbb\xbf" if has_bom else b"") + text.encode("utf-8")
    data_p.write_bytes(out_bytes)

    # Also prepare a mock single-vessel seed bundle directory for V2-02 / E3
    v2_seed_dir = src_dir.parent / "mock_v2_single_vessel"
    _make_strict_vendor_sample(v2_seed_dir, stem="SEED-VESSEL-01")
    cov = {
        "drawing_name": "SEED-VESSEL-01",
        "doc_uid": "A" * 32,
        "plant": "P01",
        "variant_resolution": {
            "exact_variant_id": 1,
            "symbol_path_match": 0,
            "structural_default": 0,
            "class_default_fallback": 0,
            "unmapped_class_variant": 0,
            "fallback_by_class": {},
        },
        "pid_verification": {
            "xml_rep_count": 1,
            "matched_graphic_oids": 1,
            "a_level_index_consistent": True,
            "b_level_pdf_bytes": 12345,
            "b_level_pdf_primitives": 48,
            "sppid_native_c1_status": "unverified_no_sppid_runner",
        },
    }
    (v2_seed_dir / "SEED-VESSEL-01_Coverage.json").write_text(
        json.dumps(cov, indent=2) + "\n", encoding="utf-8"
    )
    repacked_pid = src_dir.parent / "repacked.pid"
    repacked_pid.write_bytes(b"REPACKED_CFB_MOCK_BYTES")
    e2_pid = src_dir.parent / "e2_stamped.pid"
    e2_pid.write_bytes(b"E2_STAMPED_MOCK_BYTES")
    return v2_seed_dir, repacked_pid


def test_build_submission_round2_and_lx4_probes_and_evaluate_receipts(tmp_path: Path) -> None:
    import build_submission_round2

    src_dir = tmp_path / "src_ctrl"
    v2_seed_dir, repacked_pid = _make_rich_round2_sample(src_dir, stem="RAW-CUSTOMER-NAME")
    e2_pid = tmp_path / "e2_stamped.pid"

    r2_out = tmp_path / "round-02"
    lx4_out = tmp_path / "lx4-probes"

    rc = build_submission_round2.main(
        [
            "--source-dir",
            str(src_dir),
            "--repacked-pid",
            str(repacked_pid),
            "--single-vessel-bundle",
            str(v2_seed_dir),
            "--e2-stamped-pid",
            str(e2_pid),
            "--out-dir",
            str(r2_out),
            "--lx4-out-dir",
            str(lx4_out),
            "--plant",
            "P01",
        ]
    )
    assert rc == 0

    r2_manifest_raw = (r2_out / "submission-manifest.json").read_text(encoding="utf-8")
    assert not re.search(r"[A-Za-z]:\\", r2_manifest_raw)
    assert "RAW-CUSTOMER-NAME" not in r2_manifest_raw
    r2_manifest = json.loads(r2_manifest_raw)
    assert r2_manifest["package_kind"] == "round2_bisect"
    assert r2_manifest["precheck_passed"] is True
    assert len(r2_manifest["slots"]) == 5

    # Verify V2-03 stripped signal items/rels while keeping piping
    v203_xml = (r2_out / "03-v2-piping-no-signal" / "R2-V2-03_Data.xml").read_text(encoding="utf-8")
    assert "PIDSignalConnector" not in v203_xml
    assert "PIDSignalPort" not in v203_xml
    assert "SignalEnd1Conn" not in v203_xml
    assert "77777777777777777777777777777778" not in v203_xml
    assert "PIDPipeline" in v203_xml

    # Verify V2-04 cleared upper codelist levels while keeping leaf PipingComponentType3
    v204_xml = (r2_out / "04-v2-leaf-codelist-only" / "R2-V2-04_Data.xml").read_text(encoding="utf-8")
    assert 'PipingComponentType1=""' in v204_xml
    assert 'PipingComponentType2=""' in v204_xml
    assert 'PipingComponentType3="@{5772FBC0-A395-4FF4-9244-DCDF4809D422}"' in v204_xml

    # Evaluate Round-2 receipt -> derives B1..B4
    r2_rec_p = r2_out / "receipt.json"
    r2_rec = json.loads(r2_rec_p.read_text(encoding="utf-8"))
    outcomes = {
        "00-control": "accepted",
        "01-v2-cfb-repack": "accepted",
        "02-v2-single-vessel-seed": "accepted",
        "03-v2-piping-no-signal": "accepted",
        "04-v2-leaf-codelist-only": "rejected",
    }
    for row in r2_rec["results"]:
        row["status"] = outcomes[row["slot"]]
    r2_rec_p.write_text(json.dumps(r2_rec, indent=2), encoding="utf-8")

    eval_r2_p = tmp_path / "eval_r2.json"
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(r2_out / "submission-manifest.json"),
                "--receipt",
                str(r2_rec_p),
                "--out-json",
                str(eval_r2_p),
            ]
        )
        == 0
    )
    ev_r2 = json.loads(eval_r2_p.read_text(encoding="utf-8"))
    assert ev_r2["package_kind"] == "round2_bisect"
    b = ev_r2["bisect_conclusions"]
    assert b["B1_cfb_repack_accepted"] is True
    assert b["B2_seed_plus_one_vessel_accepted"] is True
    assert b["B3_signal_topology_required"] is False
    assert b["B4_upper_leaf_levels_required"] is True

    # Verify LX-4 E1..E3 package and receipt evaluation
    lx4_manifest = json.loads((lx4_out / "submission-manifest.json").read_text(encoding="utf-8"))
    assert lx4_manifest["package_kind"] == "lx4_acceptance"
    assert len(lx4_manifest["slots"]) == 3

    lx4_rec_p = lx4_out / "receipt.json"
    lx4_rec = json.loads(lx4_rec_p.read_text(encoding="utf-8"))
    for row in lx4_rec["results"]:
        row["status"] = "accepted" if row["slot"] != "E3-foreign-single-vessel" else "rejected"
    lx4_rec_p.write_text(json.dumps(lx4_rec, indent=2), encoding="utf-8")

    eval_lx4_p = tmp_path / "eval_lx4.json"
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(lx4_out / "submission-manifest.json"),
                "--receipt",
                str(lx4_rec_p),
                "--out-json",
                str(eval_lx4_p),
            ]
        )
        == 0
    )
    ev_lx4 = json.loads(eval_lx4_p.read_text(encoding="utf-8"))
    assert ev_lx4["lx4_conclusions"]["c1_foreign_pid_accepted"] is False
    assert ev_lx4["lx4_conclusions"]["plant_db_registration_required"] is True


def test_sppid_runner_template_ps1_is_pure_ascii_and_passes_dry_run_precheck_contract(
    tmp_path: Path,
) -> None:
    runner_ps1 = Path(__file__).resolve().parent / "sppid_runner_template.ps1"
    assert runner_ps1.is_file(), "sppid_runner_template.ps1 must exist"
    raw_bytes = runner_ps1.read_bytes()
    assert all(b < 128 for b in raw_bytes), "PowerShell runner template must be 100% ASCII"

    bundle_dir = tmp_path / "bundle_check"
    _make_strict_vendor_sample(bundle_dir, stem="DRY-CHECK-01")

    # Create a lightweight wrapper `.cmd` invoking powershell -File sppid_runner_template.ps1 --dry-run
    wrapper = tmp_path / "runner_dry.cmd"
    wrapper.write_text(
        f'@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File "{runner_ps1}" --dry-run %*\r\n',
        encoding="ascii",
    )

    out_json = tmp_path / "precheck_dry.json"
    rc = precheck_and_receipt.main(
        [
            "precheck",
            "--bundle-dir",
            str(bundle_dir),
            "--plant",
            "P01",
            "--runner",
            str(wrapper),
            "--out-json",
            str(out_json),
        ]
    )
    assert rc == 0
    rep = json.loads(out_json.read_text(encoding="utf-8"))
    assert rep["ready_for_external_submission"] is True
    assert rep["sppid_c1_d_precheck_status"] == "verified_passed"

    # Also test `restore` subcommand directly
    restore_out = tmp_path / "restore_out"
    proc = subprocess.run(
        [
            "powershell",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(runner_ps1),
            "--dry-run",
            "restore",
            "--snapshot",
            "baseline-clean",
            "--out",
            str(restore_out),
        ],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    assert proc.returncode == 0
    assert (restore_out / "restore-result.json").is_file()
