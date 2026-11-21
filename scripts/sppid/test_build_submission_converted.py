from __future__ import annotations

import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import precheck_and_receipt  # noqa: E402
from test_build_submission_round1 import _make_strict_vendor_sample  # noqa: E402


def _make_converted_bundle(
    slot_src: Path,
    stem: str,
    *,
    fallback: int = 0,
    matched_oids: int = 2,
    xml_reps: int = 2,
    dwg_source: Path | None = None,
) -> None:
    _make_strict_vendor_sample(slot_src, stem=stem)
    cov = {
        "drawing_name": stem,
        "doc_uid": "0" * 32,
        "plant": "P01",
        "variant_resolution": {
            "exact_variant_id": 12,
            "symbol_path_match": 0,
            "structural_default": 20,
            "class_default_fallback": fallback,
            "unmapped_class_variant": 0,
            "fallback_by_class": {},
        },
        "pid_verification": {
            "xml_rep_count": xml_reps,
            "matched_graphic_oids": matched_oids,
            "a_level_index_consistent": True,
            "b_level_pdf_bytes": 90123,
            "b_level_pdf_primitives": 412,
            "sppid_native_c1_status": "unverified_no_sppid_runner",
        },
    }
    (slot_src / f"{stem}_Coverage.json").write_text(
        json.dumps(cov, indent=2) + "\n", encoding="utf-8"
    )
    if dwg_source is not None:
        dwg_source.parent.mkdir(parents=True, exist_ok=True)
        dwg_source.write_bytes(b"AC1018\x00\x00\x00\x00\x00\x00MOCK_BINARY_DWG")


def test_build_submission_converted_packages_six_archetypes_anonymizes_and_gates_quality(
    tmp_path: Path,
) -> None:
    import build_submission_converted

    s1 = tmp_path / "src_s1"
    s2 = tmp_path / "src_s2"
    s3 = tmp_path / "src_s3"
    s4 = tmp_path / "src_s4"
    s5 = tmp_path / "src_s5"
    s6 = tmp_path / "src_s6"
    dwg_p = tmp_path / "dwg" / "SRC-LEGACY.dwg"

    _make_converted_bundle(s1, "SPPID-RAW-01")
    _make_converted_bundle(s2, "LEGACY-BLK-01")
    _make_converted_bundle(s3, "LEGACY-SHP-01")
    _make_converted_bundle(s4, "LEGACY-INAPP-01")
    _make_converted_bundle(s5, "LEGACY-DWG-01", dwg_source=dwg_p)
    _make_converted_bundle(s6, "PID-INAPP-EDIT-01")

    out_dir = tmp_path / "round-01b"
    rc = build_submission_converted.main(
        [
            "--sppid-dxf-bundle",
            str(s1),
            "--legacy-block-bundle",
            str(s2),
            "--legacy-shape-bundle",
            str(s3),
            "--legacy-inapp-bundle",
            str(s4),
            "--binary-dwg-bundle",
            str(s5),
            "--binary-dwg-source",
            str(dwg_p),
            "--pid-inapp-bundle",
            str(s6),
            "--out-dir",
            str(out_dir),
            "--plant",
            "P01",
        ]
    )
    assert rc == 0

    manifest_p = out_dir / "submission-manifest.json"
    receipt_p = out_dir / "receipt.json"
    assert manifest_p.is_file()
    assert receipt_p.is_file()

    raw_manifest = manifest_p.read_text(encoding="utf-8")
    assert not re.search(r"[A-Za-z]:\\", raw_manifest), "manifest must contain zero drive paths"
    assert "SPPID-RAW-01" not in raw_manifest
    assert "LEGACY-BLK-01" not in raw_manifest
    assert "PID-INAPP-EDIT-01" not in raw_manifest

    manifest = json.loads(raw_manifest)
    assert manifest["package_kind"] == "converted_pilot"
    assert manifest["precheck_passed"] is True
    assert manifest["sppid_c1_d_precheck_status"] == "unverified_no_sppid_runner"
    assert len(manifest["slots"]) == 6

    expected_slots = [
        ("01-sppid-dxf-direct", "CONV-01"),
        ("02-legacy-block-circle-direct", "CONV-02"),
        ("03-legacy-exploded-shape-direct", "CONV-03"),
        ("04-legacy-inapp-pidsavepid", "CONV-04"),
        ("05-binary-dwg-direct", "CONV-05"),
        ("06-pid-inapp-edit-save", "CONV-06"),
    ]
    for slot_info, (slot_name, stem) in zip(manifest["slots"], expected_slots):
        assert slot_info["slot"] == slot_name
        assert slot_info["delivery_stem"] == stem
        assert slot_info["strict_vendor_violations"] == 0
        assert slot_info["a_level_valid"] is True
        assert slot_info["class_default_fallback"] == 0
        for ext in (".pid", "_Data.xml", "_Meta.xml"):
            assert (out_dir / slot_name / f"{stem}{ext}").is_file()
    assert manifest["slots"][4]["binary_dwg_header"] == "AC1018"
    assert manifest["slots"][5]["archetype_key"] == "C6_pid_inapp_edit_save_accepted"

    # Rejects bundle with class_default_fallback > 0
    bad_s2 = tmp_path / "bad_s2"
    _make_converted_bundle(bad_s2, "BAD-02", fallback=2)
    rc_bad = build_submission_converted.main(
        [
            "--sppid-dxf-bundle",
            str(s1),
            "--legacy-block-bundle",
            str(bad_s2),
            "--legacy-shape-bundle",
            str(s3),
            "--legacy-inapp-bundle",
            str(s4),
            "--binary-dwg-bundle",
            str(s5),
            "--binary-dwg-source",
            str(dwg_p),
            "--pid-inapp-bundle",
            str(s6),
            "--out-dir",
            str(tmp_path / "round-01b-bad"),
            "--plant",
            "P01",
        ]
    )
    assert rc_bad == 1


def test_evaluate_receipt_supports_converted_pilot_archetypes_and_boolean_accepted_fields(
    tmp_path: Path,
) -> None:
    import build_submission_converted

    bundles = []
    for idx in range(1, 7):
        b = tmp_path / f"b{idx}"
        _make_converted_bundle(b, f"SRC-{idx:02d}")
        bundles.append(b)
    dwg_p = tmp_path / "SRC-05.dwg"
    dwg_p.write_bytes(b"AC1032\x00\x00\x00\x00MOCK")

    out_dir = tmp_path / "conv_pkg"
    assert (
        build_submission_converted.main(
            [
                "--sppid-dxf-bundle",
                str(bundles[0]),
                "--legacy-block-bundle",
                str(bundles[1]),
                "--legacy-shape-bundle",
                str(bundles[2]),
                "--legacy-inapp-bundle",
                str(bundles[3]),
                "--binary-dwg-bundle",
                str(bundles[4]),
                "--binary-dwg-source",
                str(dwg_p),
                "--pid-inapp-bundle",
                str(bundles[5]),
                "--out-dir",
                str(out_dir),
                "--plant",
                "P01",
            ]
        )
        == 0
    )

    manifest_p = out_dir / "submission-manifest.json"
    receipt_p = out_dir / "receipt.json"
    eval_p = tmp_path / "conv_eval.json"

    # Pending -> exit 2
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_p),
                "--receipt",
                str(receipt_p),
                "--out-json",
                str(eval_p),
            ]
        )
        == 2
    )

    # Partial failure using boolean `accepted: true/false` protocol schema -> isolates C3 and C6
    bool_rec = {
        "package_kind": "converted_pilot",
        "round": "1B",
        "submitted_at": "2026-11-21",
        "responded_at": "2026-11-22",
        "manifest_sha256": json.loads(manifest_p.read_text(encoding="utf-8"))["manifest_sha256"],
        "results": [
            {"slot": "01-sppid-dxf-direct", "accepted": True, "error_code": None, "raw_message": ""},
            {"slot": "02-legacy-block-circle-direct", "accepted": True, "error_code": None, "raw_message": ""},
            {"slot": "03-legacy-exploded-shape-direct", "accepted": False, "error_code": "E_PORT", "raw_message": "ERR_VALVE_PORT"},
            {"slot": "04-legacy-inapp-pidsavepid", "accepted": True, "error_code": None, "raw_message": ""},
            {"slot": "05-binary-dwg-direct", "accepted": True, "error_code": None, "raw_message": ""},
            {"slot": "06-pid-inapp-edit-save", "accepted": False, "error_code": "E_SAVE", "raw_message": "ERR_SAVE"},
        ],
    }
    receipt_p.write_text(json.dumps(bool_rec, indent=2), encoding="utf-8")

    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_p),
                "--receipt",
                str(receipt_p),
                "--out-json",
                str(eval_p),
            ]
        )
        == 0
    )
    ev1 = json.loads(eval_p.read_text(encoding="utf-8"))
    assert ev1["package_kind"] == "converted_pilot"
    assert ev1["all_archetypes_accepted"] is False
    assert ev1["archetype_acceptance"]["C3_legacy_exploded_shape_accepted"] is False
    assert ev1["archetype_acceptance"]["C6_pid_inapp_edit_save_accepted"] is False
    assert "investigate-03-legacy-exploded-shape-direct" in ev1["next_round_actions"]
    assert "investigate-06-pid-inapp-edit-save" in ev1["next_round_actions"]

    # All 6 accepted -> phase1-external-acceptance-complete
    for row in bool_rec["results"]:
        row["accepted"] = True
        row["error_code"] = None
        row["raw_message"] = ""
    receipt_p.write_text(json.dumps(bool_rec, indent=2), encoding="utf-8")
    assert (
        precheck_and_receipt.main(
            [
                "evaluate-receipt",
                "--manifest",
                str(manifest_p),
                "--receipt",
                str(receipt_p),
                "--out-json",
                str(eval_p),
            ]
        )
        == 0
    )
    ev2 = json.loads(eval_p.read_text(encoding="utf-8"))
    assert ev2["all_archetypes_accepted"] is True
    assert ev2["archetype_acceptance"]["C6_pid_inapp_edit_save_accepted"] is True
    assert ev2["next_round_actions"] == ["phase1-external-acceptance-complete"]
