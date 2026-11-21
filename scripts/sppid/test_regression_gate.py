from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import regression_gate  # noqa: E402


def _sample_summary() -> dict:
    return {
        "files_converted": 2,
        "files_failed": 0,
        "sheets": [
            {
                "drawing_name": "SENSITIVE_ALPHA_FLOW",
                "doc_uid": "AABBCCDDEEFF00112233445566778899",
                "confidence_summary": {"high": 40, "medium": 5, "low": 0},
                "total_items_written": 120,
                "total_relations_written": 110,
                "piping_synthesis": {
                    "pipelines": 4,
                    "piping_connectors": 12,
                    "piping_ports": 16,
                    "equipment_nozzle_links": 3,
                    "endpoint_connections": 20,
                },
                "pipe_network_census": {
                    "dead_end_attribution": {"open_free_end": 2},
                },
            },
            {
                "drawing_name": "SENSITIVE_BETA_FLOW",
                "doc_uid": "11223344556677889900AABBCCDDEEFF",
                "confidence_summary": {"high": 60, "medium": 8, "low": 1},
                "total_items_written": 180,
                "total_relations_written": 165,
                "piping_synthesis": {
                    "pipelines": 6,
                    "piping_connectors": 18,
                    "piping_ports": 24,
                    "equipment_nozzle_links": 5,
                    "endpoint_connections": 30,
                },
                "pipe_network_census": {
                    "dead_end_attribution": {"open_free_end": 4},
                },
            },
        ],
    }


def test_renders_sanitized_deterministic_tsv_with_totals() -> None:
    tsv = regression_gate.render_fingerprint_tsv(_sample_summary())
    assert "SENSITIVE" not in tsv
    lines = [ln for ln in tsv.splitlines() if ln.strip()]
    assert lines[0].startswith("sheet_id\t")
    assert lines[1].startswith("S01\tAABBCCDDEEFF\t")
    assert lines[2].startswith("S02\t112233445566\t")
    assert lines[3].startswith("TOTAL\t")


def test_detects_unexpected_line_diffs_and_honors_expected_column_filter() -> None:
    base_tsv = regression_gate.render_fingerprint_tsv(_sample_summary())
    cand = _sample_summary()
    cand["sheets"][0]["piping_synthesis"]["piping_ports"] = 22
    cand_tsv = regression_gate.render_fingerprint_tsv(cand)

    strict = regression_gate.diff_fingerprint_tsv(base_tsv, cand_tsv)
    assert strict["unexpected_changes"] >= 1
    assert strict["passed"] is False

    allowed = regression_gate.diff_fingerprint_tsv(
        base_tsv,
        cand_tsv,
        allowed_sheets={"S01", "TOTAL"},
        allowed_columns={"ports"},
    )
    assert allowed["unexpected_changes"] == 0
    assert allowed["passed"] is True


def test_cli_snapshot_and_diff_exit_codes(tmp_path: Path) -> None:
    sum_p = tmp_path / "summary.json"
    base_p = tmp_path / "baseline.tsv"
    sum_p.write_text(json.dumps(_sample_summary()), encoding="utf-8")

    rc_snap = regression_gate.main(
        ["snapshot", "--summary-json", str(sum_p), "--out-tsv", str(base_p)]
    )
    assert rc_snap == 0
    assert base_p.is_file()
    assert "SENSITIVE" not in base_p.read_text(encoding="utf-8")

    rc_same = regression_gate.main(
        ["diff", "--baseline-tsv", str(base_p), "--summary-json", str(sum_p)]
    )
    assert rc_same == 0

    mutated = _sample_summary()
    mutated["sheets"][1]["total_items_written"] = 999
    mut_p = tmp_path / "mutated.json"
    mut_p.write_text(json.dumps(mutated), encoding="utf-8")

    rc_diff = regression_gate.main(
        ["diff", "--baseline-tsv", str(base_p), "--summary-json", str(mut_p)]
    )
    assert rc_diff == 1
