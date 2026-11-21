"""Tests for #108 metric toolset: xml_diff.py, legend_vs_xml.py, render_vs_pdf.py."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parent))

import legend_vs_xml  # noqa: E402
import render_vs_pdf  # noqa: E402
import xml_diff  # noqa: E402

EXPECTED_HEADER = "kind\ttarget\tlocation\texpected\tactual\tdetail"

DOC_UID = "0102030405060708090A0B0C0D0E0F10"
REP1_UID = "1112131415161718191A1B1C1D1E1F20"
REP2_UID = "1112131415161718191A1B1C1D1E1F21"
VESSEL_UID = "3132333435363738393A3B3C3D3E3F40"
INSTR_UID = "4142434445464748494A4B4C4D4E4F50"
CONN_UID = "5152535455565758595A5B5C5D5E5F60"
PIPE_UID_A = "6162636465666768696A6B6C6D6E6F70"
PIPE_UID_B = "99999999999999999999999999999999"


def make_data_xml(
    *,
    vessel_name: str = "V-101",
    include_instr: bool = True,
    include_pcn_rel: bool = True,
    pipe_uid: str = PIPE_UID_A,
) -> bytes:
    lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        f'<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">',
        "   <PIDDrawing>",
        f'      <IObject UID="{DOC_UID}" Name="SYNTH-001" Description="" />',
        '      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="SYNTH" DocType="P&amp;ID" />',
        "   </PIDDrawing>",
        "   <PIDRepresentation>",
        f'      <IObject UID="{REP1_UID}" />',
        '      <IDrawingRepresentation GraphicOID="101" />',
        "   </PIDRepresentation>",
        "   <PIDProcessVessel>",
        f'      <IObject UID="{VESSEL_UID}" Name="{vessel_name}" Description="Separator" />',
        '      <INamedEquipment EquipPrefix="V" EquipSeqNo="101" />',
        "   </PIDProcessVessel>",
    ]
    if include_instr:
        lines += [
            "   <PIDRepresentation>",
            f'      <IObject UID="{REP2_UID}" />',
            '      <IDrawingRepresentation GraphicOID="102" />',
            "   </PIDRepresentation>",
            "   <PIDInstrument>",
            f'      <IObject UID="{INSTR_UID}" Name="PT-201" />',
            '      <INamedInstrument MeasuredVariable="PT" InstrTagSequenceNo="201" />',
            "   </PIDInstrument>",
        ]
    lines += [
        "   <PIDPipingPort>",
        f'      <IObject UID="{VESSEL_UID}.1" Name="1" />',
        "   </PIDPipingPort>",
        "   <PIDPipingConnector>",
        f'      <IObject UID="{CONN_UID}" Name="" />',
        '      <IPipingConnector PipingConnectorSeqNo="1001" />',
        "   </PIDPipingConnector>",
        "   <PIDPipeline>",
        f'      <IObject UID="{pipe_uid}" Name="50 mm-PG-1001-B2" />',
        "   </PIDPipeline>",
        "   <Rel>",
        f'      <IObject UID="DRC-{DOC_UID}-{REP1_UID}" />',
        f'      <IRel UID1="{DOC_UID}" UID2="{REP1_UID}" DefUID="DwgRepresentationComposition" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="DRI-{REP1_UID}-{VESSEL_UID}" />',
        f'      <IRel UID1="{REP1_UID}" UID2="{VESSEL_UID}" DefUID="DrawingItems" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PPC-{VESSEL_UID}.1-{VESSEL_UID}" />',
        f'      <IRel UID1="{VESSEL_UID}.1" UID2="{VESSEL_UID}" DefUID="PipingPortComposition" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PE1-{VESSEL_UID}.1-{CONN_UID}" />',
        f'      <IRel UID1="{VESSEL_UID}.1" UID2="{CONN_UID}" DefUID="PipingEnd1Conn" />',
        "   </Rel>",
    ]
    if include_pcn_rel:
        lines += [
            "   <Rel>",
            f'      <IObject UID="PCN-{CONN_UID}-{pipe_uid}" />',
            f'      <IRel UID1="{CONN_UID}" UID2="{pipe_uid}" DefUID="PipingConnectors" />',
            "   </Rel>",
        ]
    lines.append("  </Container>")
    return b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8")


def make_minimal_pdf(
    *,
    media_box: str = "0 0 841.89 595.28",
    strokes: list[str] | None = None,
    texts: list[str] | None = None,
) -> bytes:
    strokes = strokes if strokes is not None else ["10 10 m 200 10 l S", "200 10 m 200 100 l S"]
    texts = texts if texts is not None else ["V-101", "PT-201"]
    ops = list(strokes)
    for idx, t in enumerate(texts):
        ops.append(f"BT /F1 10 Tf 50 {100 + idx * 20} Td ({t}) Tj ET")
    stream_raw = ("\n".join(ops) + "\n").encode("latin-1")
    compressed = zlib.compress(stream_raw)
    body = (
        b"%PDF-1.4\n"
        b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n"
        b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n"
        + f"3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [{media_box}] /Contents 4 0 R >> endobj\n".encode("ascii")
        + f"4 0 obj << /Length {len(compressed)} /Filter /FlateDecode >>\nstream\n".encode("ascii")
        + compressed
        + b"\nendstream\nendobj\n%%EOF\n"
    )
    return body


# ── 1. xml_diff.py tests ─────────────────────────────────────────────────────


def test_xml_diff_same_file_and_pipeline_name_alignment_zero_diffs(tmp_path: Path, capsys):
    ref = tmp_path / "A_Data.xml"
    gen = tmp_path / "B_Data.xml"
    ref.write_bytes(make_data_xml(pipe_uid=PIPE_UID_A))
    gen.write_bytes(make_data_xml(pipe_uid=PIPE_UID_B))

    # Same file against itself -> 0 diffs
    assert xml_diff.main([str(ref), str(ref)]) == 0
    lines = capsys.readouterr().out.strip().splitlines()
    assert lines[0] == EXPECTED_HEADER
    assert lines[-1].startswith("SUMMARY\t") and "diffs=0" in lines[-1]

    # Synthesized pipeline UID difference with identical pipeline Name -> 0 diffs by default
    assert xml_diff.main([str(ref), str(gen)]) == 0
    lines_pipe = capsys.readouterr().out.strip().splitlines()
    assert "diffs=0" in lines_pipe[-1]


def test_xml_diff_three_mutations_and_allowlist(tmp_path: Path, capsys):
    ref = tmp_path / "ref_Data.xml"
    ref.write_bytes(make_data_xml())

    # (a) Modify 1 attribute -> exact 1 ATTR_DIFF
    m_attr = tmp_path / "m_attr_Data.xml"
    m_attr.write_bytes(make_data_xml(vessel_name="V-999"))
    assert xml_diff.main([str(ref), str(m_attr)]) == 1
    out_attr = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(out_attr) == 1
    assert out_attr[0].startswith("ATTR_DIFF\t")
    assert "V-101" in out_attr[0] and "V-999" in out_attr[0]

    # Whitelist that diff via --allowlist -> exit 0
    allow = tmp_path / "allow.json"
    allow.write_text(
        json.dumps([{"kind": "ATTR_DIFF", "target": VESSEL_UID, "location": "IObject/@Name", "reason": "expected rename"}]),
        encoding="utf-8",
    )
    assert xml_diff.main([str(ref), str(m_attr), "--allowlist", str(allow)]) == 0
    capsys.readouterr()

    # (b) Delete 1 isolated object (PIDInstrument + its rep) -> MISSING_OBJECT reported
    m_obj = tmp_path / "m_obj_Data.xml"
    m_obj.write_bytes(make_data_xml(include_instr=False))
    assert xml_diff.main([str(ref), str(m_obj)]) == 1
    out_obj = capsys.readouterr().out.strip().splitlines()
    assert any(ln.startswith(f"MISSING_OBJECT\t{INSTR_UID}\t") for ln in out_obj)

    # (c) Delete 1 relation -> exact 1 MISSING_REL
    m_rel = tmp_path / "m_rel_Data.xml"
    m_rel.write_bytes(make_data_xml(include_pcn_rel=False))
    assert xml_diff.main([str(ref), str(m_rel)]) == 1
    out_rel = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(out_rel) == 1
    assert out_rel[0].startswith("MISSING_REL\t")
    assert "PipingConnectors" in out_rel[0]


# ── 2. legend_vs_xml.py tests ────────────────────────────────────────────────


def test_legend_vs_xml_self_check_and_mutations(tmp_path: Path, capsys):
    ref = tmp_path / "ref_Data.xml"
    ref.write_bytes(make_data_xml())

    # Self-check against itself -> zero diffs
    assert legend_vs_xml.main(["--self-check", str(ref)]) == 0
    self_lines = capsys.readouterr().out.strip().splitlines()
    assert self_lines[0] == EXPECTED_HEADER
    assert "diffs=0" in self_lines[-1]
    assert "recall=1.000000" in self_lines[-1]
    assert "precision=1.000000" in self_lines[-1]
    assert "tag_accuracy=1.000000" in self_lines[-1]

    # Exact matching PIDLEGEND EXPORT JSON
    rec_doc = {
        "file": "SYNTH-001.dxf",
        "symbols": [
            {
                "class": "PIDProcessVessel",
                "label": "V-101",
                "tag": "V-101",
                "known": True,
                "source": "block:VESSEL",
                "at": [10.0, 20.0],
                "bbox": [0.0, 0.0, 20.0, 40.0],
                "lines": ["50 mm-PG-1001-B2"],
            },
            {
                "class": "PIDInstrument",
                "label": "PT-201",
                "tag": "PT-201",
                "known": True,
                "source": "circle",
                "at": [50.0, 60.0],
                "bbox": [45.0, 55.0, 55.0, 65.0],
                "lines": [],
            },
        ],
        "pipes": {
            "runs": [
                {
                    "numbers": ["50 mm-PG-1001-B2"],
                    "ends": [{"symbol": 0}, "open"],
                }
            ]
        },
    }
    rec_path = tmp_path / "rec.json"
    rec_path.write_text(json.dumps([rec_doc], indent=2), encoding="utf-8")
    assert legend_vs_xml.main([str(rec_path), str(ref)]) == 0
    capsys.readouterr()

    # (a) Modify 1 attribute (tag value) -> 1 TAG_MISMATCH
    bad_tag = json.loads(json.dumps(rec_doc))
    bad_tag["symbols"][1]["tag"] = "PT-999"
    bad_tag["symbols"][1]["label"] = "PT-999"
    rec_path.write_text(json.dumps(bad_tag), encoding="utf-8")
    assert legend_vs_xml.main([str(rec_path), str(ref)]) == 1
    rows_tag = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_tag) == 1
    assert rows_tag[0].startswith("TAG_MISMATCH\t")

    # (b) Delete 1 symbol -> 1 MISSING_SYMBOL
    del_sym = json.loads(json.dumps(rec_doc))
    del_sym["symbols"].pop()
    rec_path.write_text(json.dumps(del_sym), encoding="utf-8")
    assert legend_vs_xml.main([str(rec_path), str(ref)]) == 1
    rows_del = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_del) == 1
    assert rows_del[0].startswith("MISSING_SYMBOL\t")
    assert "PT-201" in rows_del[0]

    # (c) Delete 1 line connection when --check-lines is enabled -> 1 MISSING_LINE_LINK
    del_rel = json.loads(json.dumps(rec_doc))
    del_rel["symbols"][0]["lines"] = []
    rec_path.write_text(json.dumps(del_rel), encoding="utf-8")
    assert legend_vs_xml.main([str(rec_path), str(ref), "--check-lines"]) == 1
    rows_rel = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_rel) == 1
    assert rows_rel[0].startswith("MISSING_REL\t")


# ── 3. render_vs_pdf.py tests ────────────────────────────────────────────────


def test_render_vs_pdf_self_check_and_mutations_and_compare_json(tmp_path: Path, capsys):
    ref_pdf = tmp_path / "ref.pdf"
    ref_pdf.write_bytes(make_minimal_pdf())

    # Same PDF against itself -> 0 diffs
    assert render_vs_pdf.main([str(ref_pdf), str(ref_pdf)]) == 0
    self_lines = capsys.readouterr().out.strip().splitlines()
    assert self_lines[0] == EXPECTED_HEADER
    assert "diffs=0" in self_lines[-1]

    # (a) Modify 1 attribute (text string) -> 1 TEXT_DIFF
    m_txt = tmp_path / "m_txt.pdf"
    m_txt.write_bytes(make_minimal_pdf(texts=["V-101", "PT-999"]))
    assert render_vs_pdf.main([str(ref_pdf), str(m_txt)]) == 1
    rows_txt = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_txt) == 1
    assert "PT-201" in rows_txt[0]

    # (b) Delete 1 stroke object -> 1 STROKE_DIFF / COVERAGE_LOW
    m_stroke = tmp_path / "m_stroke.pdf"
    m_stroke.write_bytes(make_minimal_pdf(strokes=["10 10 m 200 10 l S"]))
    assert render_vs_pdf.main([str(ref_pdf), str(m_stroke)]) == 1
    rows_stroke = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_stroke) == 1
    assert rows_stroke[0].startswith("COVERAGE_LOW\t")

    # (c) Modify page / fit relation in pid_plot_compare JSON mode -> exact 1 FIT_DIFF
    rec_json = tmp_path / "record.json"
    cur_json = tmp_path / "current.json"
    base_metrics = {
        "coverage": 1.0,
        "extra_length_mm": 0.0,
        "text_reference": 249,
        "text_matched": 249,
        "fit": {"within_threshold": True},
    }
    rec_json.write_text(json.dumps(base_metrics), encoding="utf-8")
    cur_json.write_text(json.dumps(base_metrics), encoding="utf-8")
    assert render_vs_pdf.main(["--metrics-json", str(cur_json), "--baseline-json", str(rec_json)]) == 0
    capsys.readouterr()

    broken_fit = dict(base_metrics, fit={"within_threshold": False})
    cur_json.write_text(json.dumps(broken_fit), encoding="utf-8")
    assert render_vs_pdf.main(["--metrics-json", str(cur_json), "--baseline-json", str(rec_json)]) == 1
    rows_fit = [ln for ln in capsys.readouterr().out.strip().splitlines() if ln and not ln.startswith(("kind\t", "SUMMARY\t"))]
    assert len(rows_fit) == 1
    assert rows_fit[0].startswith("FIT_FAILED\t")


def test_all_three_scripts_skip_cleanly_when_corpus_env_unset(monkeypatch, capsys):
    monkeypatch.delenv("OCS_PID_CORPUS_DIR", raising=False)
    for mod in (xml_diff, legend_vs_xml, render_vs_pdf):
        rc = mod.main(["--corpus"])
        captured = capsys.readouterr()
        assert rc == 0
        assert "未设置语料目录" in (captured.out + captured.err)
