from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import batch_l1_eval  # noqa: E402


def _sample_bench() -> dict:
    return {
        "warmup_ms": 120,
        "sheets": [
            {
                "sheet_id": "S01",
                "stem": "SENSITIVE_STATION_ALPHA",
                "pid_sha256_prefix": "a1b2c3d4e5f6",
                "load_ok": True,
                "cad_import_ms": 310.0,
                "scene_prepare_ms": 90.0,
                "total_open_ms": 400.0,
                "cad_entities": 1450,
                "plot_ok": True,
                "plot_ms": 220.0,
                "pdf_bytes": 95000,
                "primitives": 820,
                "on_page": 810,
                "text_runs": {
                    "total_texts": 180,
                    "uniform": 160,
                    "no_runs": 18,
                    "flattened": 2,
                    "unresolvable": 0,
                },
            },
            {
                "sheet_id": "S02",
                "stem": "SENSITIVE_STATION_BETA",
                "pid_sha256_prefix": "112233445566",
                "load_ok": True,
                "cad_import_ms": 480.0,
                "scene_prepare_ms": 120.0,
                "total_open_ms": 600.0,
                "cad_entities": 2100,
                "plot_ok": True,
                "plot_ms": 340.0,
                "pdf_bytes": 112000,
                "primitives": 1120,
                "on_page": 1105,
                "text_runs": {
                    "total_texts": 240,
                    "uniform": 225,
                    "no_runs": 15,
                    "flattened": 0,
                    "unresolvable": 0,
                },
            },
            {
                "sheet_id": "S03",
                "stem": "SENSITIVE_STATION_GAMMA",
                "pid_sha256_prefix": "998877665544",
                "load_ok": True,
                "cad_import_ms": 620.0,
                "scene_prepare_ms": 180.0,
                "total_open_ms": 800.0,
                "cad_entities": 3050,
                "plot_ok": True,
                "plot_ms": 410.0,
                "pdf_bytes": 140000,
                "primitives": 1600,
                "on_page": 1580,
                "text_runs": {
                    "total_texts": 310,
                    "uniform": 300,
                    "no_runs": 9,
                    "flattened": 1,
                    "unresolvable": 0,
                },
            },
        ],
    }


def _sample_compares() -> dict:
    return {
        "SENSITIVE_STATION_ALPHA": {
            "coverage": 1.0,
            "extra_length_mm": 0.0,
            "text_matched": 180,
            "text_reference": 180,
            "fit": {"within_threshold": True},
        },
        "SENSITIVE_STATION_BETA": {
            "coverage": 0.985,
            "extra_length_mm": 42.5,
            "text_matched": 236,
            "text_reference": 240,
            "fit": {"within_threshold": True},
        },
        "SENSITIVE_STATION_GAMMA": {
            "coverage": 0.720,
            "extra_length_mm": 640.0,
            "text_matched": 210,
            "text_reference": 310,
            "fit": {"within_threshold": False},
        },
    }


def test_evaluates_timing_percentiles_and_enforces_open_budget() -> None:
    bench = _sample_bench()
    comp = _sample_compares()
    res = batch_l1_eval.evaluate_l1_batch(
        bench,
        comp,
        baseline_per_sheet_ms=732.1,
        max_open_multiple=1.5,
    )
    perf = res["l1_5_performance"]
    assert perf["sheet_count"] == 3
    assert perf["load_failures"] == 0
    assert perf["plot_failures"] == 0
    assert abs(perf["total_open_ms"] - 1800.0) < 1e-6
    assert abs(perf["mean_open_ms"] - 600.0) < 1e-6
    assert abs(perf["median_open_ms"] - 600.0) < 1e-6
    assert perf["max_open_ms"] == 800.0
    assert perf["open_budget_passed"] is True

    tight = batch_l1_eval.evaluate_l1_batch(
        bench,
        comp,
        baseline_per_sheet_ms=300.0,
        max_open_multiple=1.5,
    )
    assert tight["l1_5_performance"]["open_budget_passed"] is False


def test_audits_l1_4_flattened_runs_and_classifies_l1_2_outliers_sanitized() -> None:
    bench = _sample_bench()
    comp = _sample_compares()
    res = batch_l1_eval.evaluate_l1_batch(
        bench,
        comp,
        pair_status_by_stem={
            "SENSITIVE_STATION_ALPHA": "exact",
            "SENSITIVE_STATION_BETA": "exact",
            "SENSITIVE_STATION_GAMMA": "fuzzy",
        },
    )

    tr = res["l1_4_text_runs"]
    assert tr["total_texts"] == 730
    assert tr["flattened"] == 3
    assert tr["sheets_with_flattened"] == 2

    pdf = res["l1_2_pdf_compare"]
    assert pdf["compared_sheets"] == 3
    assert pdf["buckets"]["exact_1_000"] == 1
    assert pdf["buckets"]["ge_0_980"] == 1
    assert pdf["buckets"]["below_0_980"] == 1
    assert len(pdf["outliers"]) == 1
    outlier = pdf["outliers"][0]
    assert outlier["sheet_id"] == "S03"
    assert outlier["pid_sha256_prefix"] == "998877665544"
    assert "SENSITIVE" not in json.dumps(outlier)
    assert "fuzzy" in outlier["attribution"] or "registration" in outlier["attribution"]

    md = batch_l1_eval.render_sanitized_markdown(res)
    assert "SENSITIVE_STATION" not in md
    assert "S03" in md
    assert "998877665544" in md


def test_classifies_elongated_border_and_symbol_furniture_outliers() -> None:
    bench = _sample_bench()
    comp = _sample_compares()
    comp["SENSITIVE_STATION_BETA"] = {
        "coverage": 0.935,
        "extra_length_mm": 12.0,
        "text_matched": 782,
        "text_reference": 818,
        "page_mm": [1800.0, 841.0],
        "fit": {"within_threshold": True},
        "uncovered_reference": [
            {"label": "图框", "length_mm": 1911.4},
            {"label": "图纸线", "length_mm": 1544.2},
        ],
    }
    comp["SENSITIVE_STATION_GAMMA"] = {
        "coverage": 0.969,
        "extra_length_mm": 0.0,
        "text_matched": 144,
        "text_reference": 150,
        "page_mm": [841.0, 594.0],
        "fit": {"within_threshold": True},
        "uncovered_reference": [
            {"label": "/Sheet6", "length_mm": 252.7},
            {"label": "Set Fire Point.sym", "length_mm": 58.4},
        ],
    }
    res = batch_l1_eval.evaluate_l1_batch(bench, comp)
    outliers = {o["sheet_id"]: o for o in res["l1_2_pdf_compare"]["outliers"]}
    assert outliers["S02"]["layer"] == "border_template"
    assert "1800x841mm" in outliers["S02"]["attribution"]
    assert outliers["S03"]["layer"] == "symbol_or_furniture"
    assert "Set Fire Point.sym" in outliers["S03"]["attribution"]


def test_cli_writes_summary_json_and_checks_budget(tmp_path: Path) -> None:
    bench_p = tmp_path / "bench.json"
    comp_dir = tmp_path / "compares"
    out_json = tmp_path / "summary.json"
    out_md = tmp_path / "summary.md"
    comp_dir.mkdir()

    bench_p.write_text(json.dumps(_sample_bench()), encoding="utf-8")
    for stem, payload in _sample_compares().items():
        (comp_dir / f"{stem}.json").write_text(json.dumps(payload), encoding="utf-8")

    rc_ok = batch_l1_eval.main(
        [
            "--bench-json",
            str(bench_p),
            "--compare-dir",
            str(comp_dir),
            "--out-json",
            str(out_json),
            "--out-md",
            str(out_md),
        ]
    )
    assert rc_ok == 0
    assert out_json.is_file()
    assert out_md.is_file()
    assert "SENSITIVE_STATION" not in out_json.read_text(encoding="utf-8")
    assert "SENSITIVE_STATION" not in out_md.read_text(encoding="utf-8")

    rc_fail = batch_l1_eval.main(
        [
            "--bench-json",
            str(bench_p),
            "--compare-dir",
            str(comp_dir),
            "--baseline-ms",
            "200.0",
            "--max-multiple",
            "1.5",
        ]
    )
    assert rc_fail == 1
