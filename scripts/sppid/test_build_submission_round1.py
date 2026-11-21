from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import build_submission_round1  # noqa: E402
import xml_spec_check  # noqa: E402


def _render_obj_from_spec(
    spec: dict,
    cls: str,
    uid: str,
    name: str | None = None,
    extra_attrs: dict[tuple[str, str], str] | None = None,
) -> list[str]:
    cls_spec = spec["data_classes"][cls]
    req_ifaces = list(cls_spec["required_interfaces"])
    iface_attrs = cls_spec["interface_attributes"]
    extra = extra_attrs or {}

    ordered = ["IObject"] + [x for x in req_ifaces if x != "IObject"]
    for iface, _ in sorted(extra.keys()):
        if iface not in ordered:
            ordered.append(iface)

    lines = [f"   <{cls}>"]
    for iface in ordered:
        attrs: dict[str, str] = {}
        req_a = iface_attrs.get(iface, {}).get("required", [])
        for k in req_a:
            attrs[k] = ""
        if iface == "IObject":
            attrs["UID"] = uid
            if name is not None:
                attrs["Name"] = name
        for (ei, ek), ev in extra.items():
            if ei == iface:
                attrs[ek] = ev
        if attrs:
            # Keep UID first on IObject
            keys = (["UID"] + [k for k in attrs if k != "UID"]) if "UID" in attrs else list(attrs.keys())
            astr = " ".join(f'{k}="{attrs[k]}"' for k in keys)
            lines.append(f"      <{iface} {astr} />")
        else:
            lines.append(f"      <{iface} />")
    lines.append(f"   </{cls}>")
    return lines


def _make_strict_vendor_sample(sample_dir: Path, stem: str = "SENSITIVE_SAMPLE_UNIT") -> None:
    sample_dir.mkdir(parents=True, exist_ok=True)
    spec = xml_spec_check.load_spec()

    doc_uid = "11111111222222223333333344444444"
    rep_v_uid = "AAAAAAAABBBBBBBBCCCCCCCCDDDDDDDD"
    vessel_uid = "22222222333333334444444455555555"
    rep_p_uid = "EEEEEEEEFFFFFFFF0000000011111111"
    conn_uid = "33333333444444445555555566666666"
    pipe_uid = "44444444555555556666666677777777"
    port_uid = f"{vessel_uid}.1"
    ppt_uid = f"{conn_uid}.PPT"

    data_lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        f'<Container CompSchema="PIDComponent" Scope="Data" SoftwareVersion="10.00.31.0023" '
        f'IsValidated="False" SchemaVersion="04.02.17.01" LoginUser="" LoginPWD="" Plant="P01" '
        f'Project="" DocUID="{doc_uid}" DocName="{stem}" Version="" ToolID="SMARTPLANTPID" '
        f'ToolSignature="AAAD" SDECIMAL=".">',
        *_render_obj_from_spec(
            spec,
            "PIDDrawing",
            doc_uid,
            name=stem,
            extra_attrs={
                ("IDocument", "DocCategory"): "P&amp;ID Documents",
                ("IDocument", "DocType"): "P&amp;ID",
            },
        ),
        *_render_obj_from_spec(
            spec,
            "PIDRepresentation",
            rep_v_uid,
            extra_attrs={("IDrawingRepresentation", "GraphicOID"): "10241"},
        ),
        *_render_obj_from_spec(
            spec,
            "PIDRepresentation",
            rep_p_uid,
            extra_attrs={("IDrawingRepresentation", "GraphicOID"): "10242"},
        ),
        *_render_obj_from_spec(
            spec,
            "PIDProcessEquipment",
            vessel_uid,
            name="E-0201",
            extra_attrs={
                ("IObject", "Description"): "Separator Skid",
                ("IEquipment", "EqType0"): "@EE100",
                ("IPBSItem", "ConstructionStatus"): "@NewConstruction",
            },
        ),
        *_render_obj_from_spec(
            spec,
            "PIDPipeline",
            pipe_uid,
            name="50-PG-02011-B9",
        ),
        *_render_obj_from_spec(
            spec,
            "PIDPipingConnector",
            conn_uid,
            name="50-PG-02011-B9-50-B9--",
            extra_attrs={
                ("IInsulatedItem", "InsulSpec"): "EH40",
                ("IPipeCrossSectionItem", "NominalDiameter"): "50 mm",
                ("IPipingConnector", "PipingConnectorType"): "@EE690",
            },
        ),
        *_render_obj_from_spec(spec, "PIDPipingPort", port_uid, name="1"),
        *_render_obj_from_spec(spec, "PIDProcessPoint", ppt_uid),
        "   <Rel>",
        f'      <IObject UID="DRC-{doc_uid}-{rep_v_uid}" />',
        f'      <IRel UID1="{doc_uid}" UID2="{rep_v_uid}" DefUID="DwgRepresentationComposition" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="DRI-{rep_v_uid}-{vessel_uid}" />',
        f'      <IRel UID1="{rep_v_uid}" UID2="{vessel_uid}" DefUID="DrawingItems" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="DRC-{doc_uid}-{rep_p_uid}" />',
        f'      <IRel UID1="{doc_uid}" UID2="{rep_p_uid}" DefUID="DwgRepresentationComposition" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="DRI-{rep_p_uid}-{conn_uid}" />',
        f'      <IRel UID1="{rep_p_uid}" UID2="{conn_uid}" DefUID="DrawingItems" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PCN-{conn_uid}-{pipe_uid}" />',
        f'      <IRel UID1="{conn_uid}" UID2="{pipe_uid}" DefUID="PipingConnectors" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PPC-{port_uid}-{vessel_uid}" />',
        f'      <IRel UID1="{port_uid}" UID2="{vessel_uid}" DefUID="PipingPortComposition" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PRP-{ppt_uid}-{conn_uid}" />',
        f'      <IRel UID1="{ppt_uid}" UID2="{conn_uid}" DefUID="ProcessPointCollection" />',
        "   </Rel>",
        "   <Rel>",
        f'      <IObject UID="PE1-{port_uid}-{conn_uid}" />',
        f'      <IRel UID1="{port_uid}" UID2="{conn_uid}" DefUID="PipingEnd1Conn" />',
        "   </Rel>",
        "  </Container>",
    ]
    data_bytes = b"\xef\xbb\xbf" + ("\r\n".join(data_lines) + "\r\n").encode("utf-8")

    ver_uid = "55555555666666667777777788888888"
    rev_uid = "66666666777777778888888899999999"
    file_uid = "AAAD12345678-9ABC-4DEF-8000-112233445566"
    meta_lines = [
        '<?xml version ="1.0" encoding="UTF-8"?>',
        f'<Container CompSchema="DocVersioningComponent" Scope="Data" SoftwareVersion="10.00.31.0023" '
        f'IsValidated="False" SchemaVersion="04.02.17.01" Plant="P01" Project="" DocUID="{doc_uid}" '
        f'DocName="{stem}" Version="" ToolID="SMARTPLANTPID" ToolSignature="AAAD" SDECIMAL=".">',
        "   <DocumentVersion>",
        f'      <IObject UID="{ver_uid}" Name="{stem}" />',
        '      <IDocumentVersion DocRevision="0" DocVersion="1" DocVersionDate="2026/11/20" />',
        "      <IFileComposition />",
        "   </DocumentVersion>",
        "   <Rel>",
        '      <IObject UID="777777778888888899999999AAAAAAAA" />',
        f'      <IRel UID1="{doc_uid}" UID2="{ver_uid}" DefUID="VersionedDoc" />',
        "   </Rel>",
        "   <DocumentRevision>",
        f'      <IObject UID="{rev_uid}" Name="0" />',
        '      <IDocumentRevision MajorRev_ForRevise="0" MinorRev_ForRevise="" />',
        "   </DocumentRevision>",
        "   <Rel>",
        '      <IObject UID="8888888899999999AAAAAAAABBBBBBBB" />',
        f'      <IRel UID1="{rev_uid}" UID2="{doc_uid}" DefUID="RevisedDocument" />',
        "   </Rel>",
        "   <File>",
        f'      <IObject UID="{file_uid}" Name="{stem}.pid" Description="" />',
        f'      <IFile FilePath="export/{stem}.pid" />',
        "   </File>",
        "   <Rel>",
        '      <IObject UID="AAAD00001111-2222-3333-4444-555566667777" />',
        f'      <IRel UID1="{file_uid}" UID2="{ver_uid}" DefUID="FileComposition" />',
        "   </Rel>",
        "  </Container>",
    ]
    meta_bytes = b"\xef\xbb\xbf" + ("\r\n".join(meta_lines) + "\r\n").encode("utf-8")

    (sample_dir / f"{stem}.pid").write_bytes(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\x00" * 504)
    (sample_dir / f"{stem}_Data.xml").write_bytes(data_bytes)
    (sample_dir / f"{stem}_Meta.xml").write_bytes(meta_bytes)


def test_builds_six_slots_and_passes_local_prechecks(tmp_path: Path) -> None:
    src_dir = tmp_path / "corpus"
    out_dir = tmp_path / "round-01"
    san_json = tmp_path / "round1_sanitized.json"
    _make_strict_vendor_sample(src_dir)

    rc = build_submission_round1.main(
        [
            "--source-dir",
            str(src_dir),
            "--out-dir",
            str(out_dir),
            "--plant",
            "P01",
            "--sanitized-summary",
            str(san_json),
        ]
    )
    assert rc == 0

    manifest = json.loads((out_dir / "submission-manifest.json").read_text(encoding="utf-8"))
    receipt_tpl = json.loads((out_dir / "receipt.json").read_text(encoding="utf-8"))
    sanitized = json.loads(san_json.read_text(encoding="utf-8"))

    assert manifest["precheck_passed"] is True
    assert len(manifest["slots"]) == 6
    assert len(receipt_tpl["results"]) == 6
    assert "SENSITIVE" not in json.dumps(sanitized)
    for slot_item in manifest["slots"]:
        for f_entry in slot_item["files"]:
            assert ":" not in f_entry["path"] and "\\" not in f_entry["path"]

    by_slot = {s["slot"]: s for s in manifest["slots"]}

    assert by_slot["00-control"]["strict_vendor_violations"] == 0
    assert by_slot["00-control"]["has_pid"] is True

    assert by_slot["01-var-xml-only"]["has_pid"] is False
    assert by_slot["01-var-xml-only"]["strict_vendor_violations"] == 0

    assert by_slot["02-var-no-topology"]["strict_vendor_violations"] == 0
    data_02 = next((out_dir / "02-var-no-topology").glob("*_Data.xml")).read_text(encoding="utf-8-sig")
    assert "<PIDPipeline>" not in data_02
    assert "<PIDPipingPort>" not in data_02
    assert "<PIDProcessEquipment>" in data_02

    assert by_slot["03-var-minimal-attrs"]["strict_vendor_violations"] == 0
    data_03 = next((out_dir / "03-var-minimal-attrs").glob("*_Data.xml")).read_text(encoding="utf-8-sig")
    assert 'EqType0="@EE100"' in data_03
    assert 'InsulSpec=""' in data_03

    assert by_slot["04-var-plain-xml-format"]["schema_only_violations"] == 0
    assert by_slot["04-var-plain-xml-format"]["semantic_diffs_vs_control"] == 0
    assert by_slot["04-var-plain-xml-format"]["strict_vendor_violations"] > 0

    assert by_slot["05-var-synthetic-uids"]["strict_vendor_violations"] == 0
    data_05 = next((out_dir / "05-var-synthetic-uids").glob("*_Data.xml")).read_text(encoding="utf-8-sig")
    assert "11111111222222223333333344444444" not in data_05
    assert 'DocName="TEST-SYNTH-UID-01"' in data_05
