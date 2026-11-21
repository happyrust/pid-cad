"""Tests for scripts/sppid/xml_spec_check.py and assets/pid-publish-spec.json (L4-0 / #111)."""

from __future__ import annotations

import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import xml_spec_check  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
SPEC_PATH = REPO_ROOT / "assets" / "pid-publish-spec.json"

DOC_UID = "0102030405060708090A0B0C0D0E0F10"
REP_UID = "1112131415161718191A1B1C1D1E1F20"
NOTE_UID = "2122232425262728292A2B2C2D2E2F30"


def _bom_crlf(lines: list[str]) -> bytes:
    return b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8")


def valid_data_bytes() -> bytes:
    return _bom_crlf(
        [
            '<?xml version ="1.0" encoding="UTF-8"?>',
            f'<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">',
            "   <PIDDrawing>",
            f'      <IObject UID="{DOC_UID}" Name="SYNTH-001" Description="" />',
            "      <IDocVersionComposition />",
            '      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="SYNTH" DocType="P&amp;ID" />',
            "      <IDwgRepresentationComposition />",
            "      <IPBSItem />",
            "      <IPIDDrawing />",
            "      <ISchematicDwg />",
            "   </PIDDrawing>",
            "   <PIDRepresentation>",
            f'      <IObject UID="{REP_UID}" />',
            '      <IDrawingRepresentation GraphicOID="101" />',
            "   </PIDRepresentation>",
            "   <PIDNote>",
            f'      <IObject UID="{NOTE_UID}" />',
            "      <IDocumentItem />",
            "      <IDrawingItem />",
            "      <INote />",
            "      <IPBSNote />",
            "   </PIDNote>",
            "   <Rel>",
            f'      <IObject UID="DRC-{DOC_UID}-{REP_UID}" />',
            f'      <IRel UID1="{DOC_UID}" UID2="{REP_UID}" DefUID="DwgRepresentationComposition" />',
            "   </Rel>",
            "   <Rel>",
            f'      <IObject UID="DRI-{REP_UID}-{NOTE_UID}" />',
            f'      <IRel UID1="{REP_UID}" UID2="{NOTE_UID}" DefUID="DrawingItems" />',
            "   </Rel>",
            "  </Container>",
        ]
    )


def valid_meta_bytes() -> bytes:
    dv = "A1A2A3A4A5A6A7A8A9AAABACADAEAF0"
    dr = "B1B2B3B4B5B6B7B8B9BABBBCBDBEBF00"
    fuid = "AAAD12345678-9ABC-4DEF-8000-112233445566"
    return _bom_crlf(
        [
            '<?xml version ="1.0" encoding="UTF-8"?>',
            f'<Container CompSchema="DocVersioningComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">',
            "   <DocumentVersion>",
            f'      <IObject UID="{dv}" Name="SYNTH-001" />',
            '      <IDocumentVersion DocRevision="0" DocVersion="1" DocVersionDate="2026/11/20" />',
            "      <IFileComposition />",
            "   </DocumentVersion>",
            "   <Rel>",
            '      <IObject UID="C1C2C3C4C5C6C7C8C9CACBCCCDCECF0" />',
            f'      <IRel UID1="{DOC_UID}" UID2="{dv}" DefUID="VersionedDoc" />',
            "   </Rel>",
            "   <DocumentRevision>",
            f'      <IObject UID="{dr}" Name="SYNTH-001" />',
            '      <IDocumentRevision MajorRev_ForRevise="0" MinorRev_ForRevise="" />',
            "   </DocumentRevision>",
            "   <Rel>",
            '      <IObject UID="D1D2D3D4D5D6D7D8D9DADBDCDDDEDF00" />',
            f'      <IRel UID1="{dr}" UID2="{DOC_UID}" DefUID="RevisedDocument" />',
            "   </Rel>",
            "   <File>",
            f'      <IObject UID="{fuid}" Name="SYNTH-001_Data.xml" Description="" />',
            '      <IFile FilePath="out" />',
            "   </File>",
            "   <Rel>",
            '      <IObject UID="AAAD87654321-9ABC-4DEF-8000-665544332211" />',
            f'      <IRel UID1="{fuid}" UID2="{dv}" DefUID="FileComposition" />',
            "   </Rel>",
            "  </Container>",
        ]
    )


def test_spec_json_has_zero_customer_data():
    raw = SPEC_PATH.read_text(encoding="utf-8")
    assert raw.isascii()
    assert re.search(r'"[0-9A-F]{28,32}"', raw) is None
    spec = json.loads(raw)
    assert spec["schema_version"] == 1
    assert len(spec["data_classes"]) == 21
    assert len(spec["data_relations"]) == 15


def test_valid_pair_passes_with_zero_violations(tmp_path: Path, capsys):
    (tmp_path / "SYNTH_Data.xml").write_bytes(valid_data_bytes())
    (tmp_path / "SYNTH_Meta.xml").write_bytes(valid_meta_bytes())

    rc = xml_spec_check.main([str(tmp_path)])
    out = capsys.readouterr()
    assert rc == 0, out.out + out.err
    assert "0 violation(s)" in (out.out + out.err)


def test_seven_intentional_mutations_each_report_exact_details(tmp_path: Path, capsys):
    base = valid_data_bytes()
    base_str = base[3:].decode("utf-8")

    cases = [
        ("no_bom_Data.xml", base[3:], "MISSING_UTF8_BOM", "EF BB BF"),
        ("lf_only_Data.xml", b"\xef\xbb\xbf" + base_str.replace("\r\n", "\n").encode("utf-8"), "LINE_ENDING_NOT_CRLF", "CRLF"),
        (
            "bad_uid_Data.xml",
            b"\xef\xbb\xbf" + base_str.replace(NOTE_UID, NOTE_UID.lower()).encode("utf-8"),
            "INVALID_OBJECT_UID",
            NOTE_UID.lower(),
        ),
        (
            "bad_prefix_Data.xml",
            b"\xef\xbb\xbf"
            + base_str.replace(f"DRC-{DOC_UID}-{REP_UID}", f"DRI-{DOC_UID}-{REP_UID}").encode("utf-8"),
            "INVALID_REL_UID_PREFIX",
            f"DRC-{DOC_UID}-{REP_UID}",
        ),
        (
            "bad_endpoints_Data.xml",
            b"\xef\xbb\xbf"
            + base_str.replace(f"DRC-{DOC_UID}-{REP_UID}", f"DRC-{DOC_UID}-{NOTE_UID}")
            .replace(
                f'UID1="{DOC_UID}" UID2="{REP_UID}" DefUID="DwgRepresentationComposition"',
                f'UID1="{DOC_UID}" UID2="{NOTE_UID}" DefUID="DwgRepresentationComposition"',
            )
            .encode("utf-8"),
            "DISALLOWED_REL_ENDPOINTS",
            "PIDDrawing -> PIDNote",
        ),
        (
            "bad_container_Data.xml",
            b"\xef\xbb\xbf" + base_str.replace('ToolSignature="AAAD"', 'ToolSignature="XXXX"').encode("utf-8"),
            "CONTAINER_ATTR_MISMATCH",
            "XXXX",
        ),
        (
            "first_not_drawing_Data.xml",
            _bom_crlf(
                [
                    '<?xml version ="1.0" encoding="UTF-8"?>',
                    f'<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" Project="" DocUID="{DOC_UID}" DocName="SYNTH-001" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">',
                    "   <PIDRepresentation>",
                    f'      <IObject UID="{REP_UID}" />',
                    '      <IDrawingRepresentation GraphicOID="101" />',
                    "   </PIDRepresentation>",
                    "  </Container>",
                ]
            ),
            "FIRST_ELEMENT_NOT_PID_DRAWING",
            "PIDDrawing",
        ),
    ]

    for fname, payload, expected_code, expected_token in cases:
        fpath = tmp_path / fname
        fpath.write_bytes(payload)
        rc = xml_spec_check.main([str(fpath)])
        captured = capsys.readouterr()
        combined = captured.out + captured.err
        assert rc == 1, f"{fname} should fail"
        assert fname in combined, f"{fname} missing filename in: {combined}"
        assert expected_code in combined, f"{fname} missing {expected_code} in: {combined}"
        assert "expected" in combined and "got" in combined, f"{fname} missing expected/got in: {combined}"
        assert expected_token in combined, f"{fname} missing {expected_token} in: {combined}"


def test_unset_corpus_env_exits_zero(monkeypatch, capsys):
    monkeypatch.delenv("OCS_PID_CORPUS_DIR", raising=False)
    rc = xml_spec_check.main(["--corpus"])
    captured = capsys.readouterr()
    assert rc == 0
    assert "未设置语料目录" in (captured.out + captured.err)
