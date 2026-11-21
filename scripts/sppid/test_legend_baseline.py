"""Tests for scripts/sppid/legend_baseline.py (L0-5 / L3-0 / #122)."""

from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus_correspondence  # noqa: E402
import corpus_index  # noqa: E402
import legend_baseline  # noqa: E402

DOC_UID = "0102030405060708090A0B0C0D0E0F10"
V_UID = "1112131415161718191A1B1C1D1E1F20"
I_UID = "2122232425262728292A2B2C2D2E2F30"
K_UID = "3132333435363738393A3B3C3D3E3F40"


def put(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def make_ref_xml() -> bytes:
    lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        f'<Container DocUID="{DOC_UID}" DocName="S1">',
        "   <PIDDrawing>",
        f'      <IObject UID="{DOC_UID}" Name="S1" />',
        "   </PIDDrawing>",
        "   <PIDProcessVessel>",
        f'      <IObject UID="{V_UID}" Name="V-101" />',
        "   </PIDProcessVessel>",
        "   <PIDInstrument>",
        f'      <IObject UID="{I_UID}" Name="PT-201" />',
        "   </PIDInstrument>",
        "   <PIDPipingComponent>",
        f'      <IObject UID="{K_UID}" Name="BV-301" />',
        "   </PIDPipingComponent>",
        "  </Container>",
    ]
    return b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8")


def test_unset_env_exits_zero(monkeypatch, capsys):
    monkeypatch.delenv("OCS_PID_CORPUS_DIR", raising=False)
    rc = legend_baseline.main([])
    captured = capsys.readouterr()
    assert rc == 0
    assert "未设置语料目录" in (captured.out + captured.err)


def test_aggregates_baseline_metrics_and_top_gaps_reproducibly(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    put(corpus / "export-set" / "01" / "S1.pid", b"pid1")
    put(corpus / "export-set" / "01" / "S1_Data.xml", make_ref_xml())
    put(corpus / "export-set" / "01" / "S1_Meta.xml", b"meta1")
    put(corpus / "export-dxf" / "S1.dxf", b"dxf-nonempty")

    put(corpus / "export-set" / "01" / "S2_empty.pid", b"pid2")
    put(corpus / "export-set" / "01" / "S2_empty_Data.xml", make_ref_xml())
    put(corpus / "export-set" / "01" / "S2_empty_Meta.xml", b"meta2")
    put(corpus / "export-dxf" / "S2_empty.dxf", b"")

    manifest_path = tmp_path / "work" / "corpus.manifest.json"
    rec_dir = tmp_path / "work" / "rec_cache"
    out_dir = tmp_path / "work" / "report"

    assert corpus_index.main(["index", "--dir", str(corpus), "--manifest", str(manifest_path)]) == 0
    assert corpus_correspondence.main(["--dir", str(corpus), "--manifest", str(manifest_path)]) == 0
    capsys.readouterr()

    # Pre-seed recognition JSON cache for S1.dxf
    rec_dir.mkdir(parents=True, exist_ok=True)
    rec_payload = {
        "file": "S1.dxf",
        "symbols": [
            {
                "class": "BALL_VALVE",
                "label": "球阀",
                "tag": "BV-301",
                "known": True,
                "source": "block:$VALVE$1",
                "at": [0.0, 0.0],
                "bbox": [0.0, 0.0, 5.0, 5.0],
                "lines": [],
            },
            {
                "class": "CUSTOM_WIDGET",
                "label": "未知件",
                "tag": "V-101",
                "known": False,
                "source": "shape:abcd1234",
                "at": [10.0, 10.0],
                "bbox": [8.0, 8.0, 12.0, 12.0],
                "lines": [],
            },
        ],
        "unknown_blocks": {"$EQUIP$99": 4},
        "unknown_shapes": [{"id": "abcd1234", "count": 3}],
        "orphan_tags": ["PT-201"],
        "pipes": {"runs": []},
    }
    (rec_dir / "0000.json").write_text(json.dumps(rec_payload), encoding="utf-8")

    rc1 = legend_baseline.main(
        [
            "--dir",
            str(corpus),
            "--manifest",
            str(manifest_path),
            "--rec-cache",
            str(rec_dir),
            "--out-dir",
            str(out_dir),
        ]
    )
    capsys.readouterr()
    assert rc1 == 0

    summary1 = json.loads((out_dir / "baseline-summary.json").read_text(encoding="utf-8"))
    assert summary1["denominator"]["total_triples"] == 2
    assert summary1["denominator"]["skipped_empty_dxf"] == 1
    assert summary1["denominator"]["evaluated_drawings"] == 1
    assert summary1["reference"]["total_symbols"] == 3
    assert summary1["reference"]["tagged_symbols"] == 3
    # BV-301 mapped via BALL_VALVE -> PIDPipingComponent; V-101 tag found on unmapped class; PT-201 in orphan_tags
    assert summary1["tag_level"]["matched_on_symbol"] == 2
    assert summary1["tag_level"]["seen_in_orphan_tags"] == 1
    assert summary1["top_gaps"]["unknown_blocks"][0] == ["$EQUIP$99", 4]
    assert summary1["top_gaps"]["unmapped_rec_classes"][0] == ["CUSTOM_WIDGET", 1]

    # Run a second time and verify identical numbers
    rc2 = legend_baseline.main(
        [
            "--dir",
            str(corpus),
            "--manifest",
            str(manifest_path),
            "--rec-cache",
            str(rec_dir),
            "--out-dir",
            str(out_dir),
        ]
    )
    capsys.readouterr()
    assert rc2 == 0
    summary2 = json.loads((out_dir / "baseline-summary.json").read_text(encoding="utf-8"))
    assert summary1 == summary2
