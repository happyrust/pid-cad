from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import convert_check  # noqa: E402


def _strict_data_xml(
    doc_uid: str,
    inst_uid: str,
    inst_name: str,
    mv: str,
    seq: str,
    desc: str | None = None,
) -> bytes:
    desc_attr = f' Description="{desc}"' if desc is not None else ""
    lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        (
            f'<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" '
            f'IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" '
            f'Project="" DocUID="{doc_uid}" DocName="S01" Version="" ToolID="SMARTPLANTPID" '
            f'ToolSignature="AAAD" SDECIMAL=".">'
        ),
        "   <PIDDrawing>",
        f'      <IObject UID="{doc_uid}" Name="S01" Description="" />',
        "      <IDocVersionComposition />",
        '      <IDocument DocCategory="P&amp;ID Documents" DocSubtype="" DocTitle="S01" DocType="P&amp;ID" />',
        "      <IDwgRepresentationComposition />",
        "      <IPBSItem />",
        "      <IPIDDrawing />",
        "      <ISchematicDwg />",
        "   </PIDDrawing>",
        "   <PIDInstrument>",
        f'      <IObject UID="{inst_uid}" Name="{inst_name}"{desc_attr} />',
        "      <IDocumentItem />",
        "      <IDrawingItem />",
        "      <IElecPowerConsumer />",
        "      <IExpandableThing />",
        '      <IHeatTracedItem HTraceRqmt="" />',
        "      <IInlineComponentOcc />",
        "      <IInstrument />",
        "      <IInstrumentOcc />",
        "      <ILoopMember />",
        f'      <INamedInstrument InstrFuncModifier="" InstrLoopSuffix="" InstrTagPrefix="" InstrTagSequenceNo="{seq}" InstrTagSuffix="" MeasuredVariable="{mv}" />',
        "      <INoteCollection />",
        '      <IPBSItem ConstructionStatus="@NewConstruction" ConstructionStatus2="@{78398AB4-9F3D-11D6-BDA7-00104BCC2B69}" />',
        '      <IPIDTypical IsTypical="False" />',
        "      <IPart />",
        "      <IPartOcc />",
        "      <IPipingPortComposition />",
        "      <IPlannedMatl />",
        "      <IPressureReliefItem />",
        "      <IProcessPointCollection />",
        "      <ISignalPortComposition />",
        "   </PIDInstrument>",
        "  </Container>",
    ]
    return b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8")


def _strict_meta_xml(doc_uid: str) -> bytes:
    dv = "11111111111111111111111111111111"
    dr = "22222222222222222222222222222222"
    fu = "AAAD33333333-3333-3333-3333-333333333333"
    rvd = "44444444444444444444444444444444"
    rrd = "55555555555555555555555555555555"
    rfc = "AAAD66666666-6666-6666-6666-666666666666"
    lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        (
            f'<Container CompSchema="DocVersioningComponent" Scope="Data" SoftwareVersion="10.00.31.0023" '
            f'IsValidated="False" SchemaVersion="04.02.17.01" Plant="P01" '
            f'Project="" DocUID="{doc_uid}" DocName="S01" Version="" ToolID="SMARTPLANTPID" '
            f'ToolSignature="AAAD" SDECIMAL=".">'
        ),
        "   <DocumentVersion>",
        f'      <IObject UID="{dv}" Name="S01" />',
        '      <IDocumentVersion DocRevision="0" DocVersion="1" DocVersionDate="2026/11/20" />',
        "      <IFileComposition />",
        "   </DocumentVersion>",
        "   <Rel>",
        f'      <IObject UID="{rvd}" />',
        f'      <IRel UID1="{doc_uid}" UID2="{dv}" DefUID="VersionedDoc" />',
        "   </Rel>",
        "   <DocumentRevision>",
        f'      <IObject UID="{dr}" Name="S01" />',
        '      <IDocumentRevision MajorRev_ForRevise="0" MinorRev_ForRevise="" />',
        "   </DocumentRevision>",
        "   <Rel>",
        f'      <IObject UID="{rrd}" />',
        f'      <IRel UID1="{dr}" UID2="{doc_uid}" DefUID="RevisedDocument" />',
        "   </Rel>",
        "   <File>",
        f'      <IObject UID="{fu}" Name="S01_Data.xml" Description="" />',
        '      <IFile FilePath="S01_Data.xml" />',
        "   </File>",
        "   <Rel>",
        f'      <IObject UID="{rfc}" />',
        f'      <IRel UID1="{fu}" UID2="{dv}" DefUID="FileComposition" />',
        "   </Rel>",
        "  </Container>",
    ]
    return b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8")


def _coverage_json(high: int = 1, medium: int = 0, low: int = 0) -> str:
    return json.dumps(
        {
            "drawing_name": "S01",
            "plant": "P01",
            "confidence_summary": {
                "high": high,
                "medium": medium,
                "low": low,
                "ignored_blocks": 0,
                "unmapped_blocks": low,
            },
            "exported_by_class": {"PIDInstrument": high + medium},
            "skipped_items": [{"source_id": f"u{i}"} for i in range(low)],
        },
        indent=2,
    )


def test_valid_bundle_and_allowlisted_db_description_pass(tmp_path: Path) -> None:
    gen = tmp_path / "gen"
    ref = tmp_path / "ref"
    gen.mkdir()
    ref.mkdir()

    doc_uid_gen = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    doc_uid_ref = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    (gen / "S01_Data.xml").write_bytes(
        _strict_data_xml(doc_uid_gen, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC", "PG-0201501A", "PG", "0201501A")
    )
    (gen / "S01_Meta.xml").write_bytes(_strict_meta_xml(doc_uid_gen))
    (gen / "S01_Coverage.json").write_text(_coverage_json(high=1), encoding="utf-8")

    (ref / "S01_Data.xml").write_bytes(
        _strict_data_xml(
            doc_uid_ref,
            "DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD",
            "PG-0201501A",
            "PG",
            "0201501A",
            desc="DB-only description",
        )
    )

    res = convert_check.check_converted_dir(gen, ref_dir=ref, expected_plant="P01")
    assert res["sheets_checked"] == 1
    assert res["spec_violations"] == 0, res["errors"]
    assert res["coverage_errors"] == 0, res["errors"]
    assert res["unexplained_diffs"] == 0, res["errors"]
    assert res["allowlisted_diffs"] >= 1
    assert convert_check.main([str(gen), "--ref-dir", str(ref), "--plant", "P01"]) == 0


def test_missing_bom_fails_strict_spec_gate(tmp_path: Path) -> None:
    gen = tmp_path / "gen"
    gen.mkdir()
    doc_uid = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    raw = _strict_data_xml(doc_uid, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC", "PG-0201501A", "PG", "0201501A")
    (gen / "S01_Data.xml").write_bytes(raw[3:])
    (gen / "S01_Meta.xml").write_bytes(_strict_meta_xml(doc_uid))
    (gen / "S01_Coverage.json").write_text(_coverage_json(high=1), encoding="utf-8")

    res = convert_check.check_converted_dir(gen)
    assert res["spec_violations"] >= 1
    assert convert_check.main([str(gen)]) == 1


def test_unallowlisted_segmented_attr_corruption_fails_gate(tmp_path: Path) -> None:
    gen = tmp_path / "gen"
    ref = tmp_path / "ref"
    gen.mkdir()
    ref.mkdir()
    doc_uid = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    (gen / "S01_Data.xml").write_bytes(
        _strict_data_xml(doc_uid, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC", "PG-0201501A", "LT", "0201501A")
    )
    (gen / "S01_Meta.xml").write_bytes(_strict_meta_xml(doc_uid))
    (gen / "S01_Coverage.json").write_text(_coverage_json(high=1), encoding="utf-8")
    (ref / "S01_Data.xml").write_bytes(
        _strict_data_xml(doc_uid, "DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD", "PG-0201501A", "PG", "0201501A")
    )

    res = convert_check.check_converted_dir(gen, ref_dir=ref)
    assert res["spec_violations"] == 0, res["errors"]
    assert res["unexplained_diffs"] >= 1
    assert convert_check.main([str(gen), "--ref-dir", str(ref)]) == 1


def test_broken_coverage_conservation_fails_gate(tmp_path: Path) -> None:
    gen = tmp_path / "gen"
    gen.mkdir()
    doc_uid = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    (gen / "S01_Data.xml").write_bytes(
        _strict_data_xml(doc_uid, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC", "PG-0201501A", "PG", "0201501A")
    )
    (gen / "S01_Meta.xml").write_bytes(_strict_meta_xml(doc_uid))
    bad_cov = json.loads(_coverage_json(high=1))
    bad_cov["confidence_summary"]["high"] = 5
    (gen / "S01_Coverage.json").write_text(json.dumps(bad_cov), encoding="utf-8")

    res = convert_check.check_converted_dir(gen)
    assert res["coverage_errors"] >= 1
    assert convert_check.main([str(gen)]) == 1
