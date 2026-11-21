#!/usr/bin/env python3
"""Build Round-1 6-slot controlled subtraction test submission package (LX-8 / LX-9 / #135).

Implements the exact slot matrix and manifest/receipt contracts defined in
`docs/plans/2026-11-20-phase1-dwg-to-smart-pid/04-external-platform-protocol.md`:

- `00-control`: unmodified smallest passed SPPID triple (`.pid` + `_Data.xml` + `_Meta.xml`);
- `01-var-xml-only` (`V1-01`): `_Data.xml` + `_Meta.xml` only, `.pid` omitted;
- `02-var-no-topology` (`V1-02`): removes `PIDPipeline`, `PIDPipingPort`, `PIDSignalPort`,
  `PIDPipingBranchPoint`, `PIDSignalBranchPoint`, `PIDProcessPoint`, and connectivity `Rel`s;
- `03-var-minimal-attrs` (`V1-03`): preserves `UID`, `Name`, required envelope fields, and
  `@*` codelist / taxonomy values while clearing optional business attributes to `""`;
- `04-var-plain-xml-format` (`V1-04`): strips UTF-8 BOM, converts `CRLF` -> `LF`, `version ="1.0"`
  -> `version="1.0"`, and `0/3/6/3/2` indentation -> 2-space indentation (100% identical XML semantics);
- `05-var-synthetic-uids` (`V1-05`): deterministically remaps all primary 32-hex UIDs and derived
  port/branch/process-point/relation UIDs across `_Data.xml` and `_Meta.xml`, setting `DocName`
  to `TEST-SYNTH-UID-01`.

Automatically runs local Gate-1 prechecks (`xml_spec_check` in `StrictVendor` & `SchemaOnly` modes
and `xml_diff` semantic comparison) and writes strictly relative-path `submission-manifest.json`
plus `receipt.json` template.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
from typing import Any, Sequence

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import xml_diff  # noqa: E402
import xml_spec_check  # noqa: E402

BOM = b"\xef\xbb\xbf"
ATTR_PAIR_RE = re.compile(r'([A-Za-z_][A-Za-z0-9_.:-]*)=("([^"]*)")')
OPEN_OBJ_RE = re.compile(r"^\s*<([A-Za-z_][A-Za-z0-9_.:-]*)\s*>\s*$")
CLOSE_OBJ_RE = re.compile(r"^\s*</([A-Za-z_][A-Za-z0-9_.:-]*)\s*>\s*$")

TOPOLOGY_CLASSES_TO_DROP: frozenset[str] = frozenset(
    {
        "PIDPipeline",
        "PIDPipingPort",
        "PIDSignalPort",
        "PIDPipingBranchPoint",
        "PIDSignalBranchPoint",
        "PIDProcessPoint",
    }
)

REL_PREFIX_BY_DEF: dict[str, str] = {
    "DrawingItems": "DRI",
    "DwgRepresentationComposition": "DRC",
    "EquipmentComponentComposition": "EQC",
    "EquipmentOcc_EquipmentOcc": "E2E",
    "PBSItemNotes": "ITN",
    "PipingConnectors": "PCN",
    "PipingEnd1Conn": "PE1",
    "PipingEnd2Conn": "PE2",
    "PipingPortComposition": "PPC",
    "PipingTapOrFitting": "PTF",
    "ProcessPointCollection": "PRP",
    "SignalEnd1Conn": "SE1",
    "SignalEnd2Conn": "SE2",
    "SignalPortComposition": "SPC",
    "SignalTapOrFitting": "STF",
}

PRESERVED_ATTR_NAMES: frozenset[str] = frozenset(
    {
        "UID",
        "Name",
        "DefUID",
        "UID1",
        "UID2",
        "OrderValue",
        "GraphicOID",
        "RepresentationsAreAllZeroLength",
        "IsTypical",
        "ConstructionStatus",
        "ConstructionStatus2",
        "DocCategory",
        "DocType",
        "DocSubtype",
        "DocTitle",
        "ItemTag",
    }
)


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _synth_hex32(seed_label: str) -> str:
    return hashlib.sha256(f"ocs-round1-v1-05:{seed_label}".encode("utf-8")).hexdigest()[:32].upper()


def _parse_vendor_blocks(raw: bytes) -> tuple[str, str, list[tuple[str, list[str]]], str]:
    body = raw[3:] if raw.startswith(BOM) else raw
    lines = body.decode("utf-8").splitlines()
    decl = lines[0]
    open_cont = lines[1]
    close_cont = lines[-1]
    blocks: list[tuple[str, list[str]]] = []
    i = 2
    end_idx = len(lines) - 1
    while i < end_idx:
        m = OPEN_OBJ_RE.match(lines[i])
        if not m:
            i += 1
            continue
        tag = m.group(1)
        blk = [lines[i]]
        i += 1
        while i < end_idx:
            blk.append(lines[i])
            cm = CLOSE_OBJ_RE.match(lines[i])
            i += 1
            if cm and cm.group(1) == tag:
                break
        blocks.append((tag, blk))
    return decl, open_cont, blocks, close_cont


def _encode_vendor_blocks(
    decl: str,
    open_cont: str,
    blocks: Sequence[tuple[str, Sequence[str]]],
    close_cont: str,
) -> bytes:
    out_lines = [decl, open_cont]
    for _, blk in blocks:
        out_lines.extend(blk)
    out_lines.append(close_cont)
    return BOM + ("\r\n".join(out_lines) + "\r\n").encode("utf-8")


def _extract_block_attrs(blk: Sequence[str]) -> dict[tuple[str, str], str]:
    res: dict[tuple[str, str], str] = {}
    for ln in blk[1:-1]:
        stripped = ln.strip()
        if not stripped.startswith("<"):
            continue
        parts = stripped[1:].split(None, 1)
        if not parts:
            continue
        iface = parts[0].rstrip("/>")
        for m in ATTR_PAIR_RE.finditer(stripped):
            res[(iface, m.group(1))] = m.group(3)
    return res


def make_v1_02_no_topology(data_bytes: bytes) -> bytes:
    decl, open_cont, blocks, close_cont = _parse_vendor_blocks(data_bytes)

    dropped_uids: set[str] = set()
    for tag, blk in blocks:
        if tag in TOPOLOGY_CLASSES_TO_DROP:
            attrs = _extract_block_attrs(blk)
            uid = attrs.get(("IObject", "UID"), "")
            if uid:
                dropped_uids.add(uid)

    dropped_reps: set[str] = set()
    for tag, blk in blocks:
        if tag == "Rel":
            attrs = _extract_block_attrs(blk)
            if attrs.get(("IRel", "DefUID")) == "DrawingItems":
                u1 = attrs.get(("IRel", "UID1"), "")
                u2 = attrs.get(("IRel", "UID2"), "")
                if u2 in dropped_uids and u1:
                    dropped_reps.add(u1)
    dropped_uids |= dropped_reps

    kept_blocks: list[tuple[str, list[str]]] = []
    allowed_rels = {
        "DwgRepresentationComposition",
        "DrawingItems",
        "EquipmentComponentComposition",
        "PBSItemNotes",
    }
    for tag, blk in blocks:
        if tag in TOPOLOGY_CLASSES_TO_DROP:
            continue
        attrs = _extract_block_attrs(blk)
        uid = attrs.get(("IObject", "UID"), "")
        if uid in dropped_uids:
            continue
        if tag == "Rel":
            def_uid = attrs.get(("IRel", "DefUID"), "")
            u1 = attrs.get(("IRel", "UID1"), "")
            u2 = attrs.get(("IRel", "UID2"), "")
            if def_uid not in allowed_rels or u1 in dropped_uids or u2 in dropped_uids:
                continue
        kept_blocks.append((tag, blk))

    return _encode_vendor_blocks(decl, open_cont, kept_blocks, close_cont)


def make_v1_03_minimal_attrs(data_bytes: bytes) -> bytes:
    decl, open_cont, blocks, close_cont = _parse_vendor_blocks(data_bytes)
    new_blocks: list[tuple[str, list[str]]] = []

    def _keep_attr(name: str, val: str) -> bool:
        if name in PRESERVED_ATTR_NAMES:
            return True
        if val.startswith("@"):
            return True
        if "Type" in name or "Code" in name:
            return True
        return False

    for tag, blk in blocks:
        if tag in ("PIDDrawing", "PIDRepresentation", "Rel"):
            new_blocks.append((tag, blk))
            continue
        updated = [blk[0]]
        for ln in blk[1:-1]:
            def _repl(m: re.Match[str]) -> str:
                k, _, v = m.group(1), m.group(2), m.group(3)
                if _keep_attr(k, v):
                    return m.group(0)
                return f'{k}=""'

            updated.append(ATTR_PAIR_RE.sub(_repl, ln))
        updated.append(blk[-1])
        new_blocks.append((tag, updated))

    return _encode_vendor_blocks(decl, open_cont, new_blocks, close_cont)


def make_v1_04_plain_xml_format(xml_bytes: bytes) -> bytes:
    decl, open_cont, blocks, close_cont = _parse_vendor_blocks(xml_bytes)
    plain_decl = decl.replace('version ="1.0"', 'version="1.0"')
    out_lines = [plain_decl, open_cont.strip()]
    for _, blk in blocks:
        out_lines.append("  " + blk[0].strip())
        for ln in blk[1:-1]:
            out_lines.append("    " + ln.strip())
        out_lines.append("  " + blk[-1].strip())
    out_lines.append(close_cont.strip())
    return ("\n".join(out_lines) + "\n").encode("utf-8")


def make_v1_05_synthetic_uids(
    data_bytes: bytes,
    meta_bytes: bytes,
    *,
    test_doc_name: str = "TEST-SYNTH-UID-01",
) -> tuple[bytes, bytes]:
    d_decl, d_open, d_blocks, d_close = _parse_vendor_blocks(data_bytes)
    m_decl, m_open, m_blocks, m_close = _parse_vendor_blocks(meta_bytes)

    uid_map: dict[str, str] = {}

    def map_primary(old_uid: str) -> str:
        if old_uid not in uid_map:
            if old_uid.startswith("AAAD"):
                h = _synth_hex32(old_uid)
                uid_map[old_uid] = f"AAAD-{h[:4]}-{h[4:8]}"
            else:
                uid_map[old_uid] = _synth_hex32(old_uid)
        return uid_map[old_uid]

    def map_any_uid(old_uid: str) -> str:
        if not old_uid:
            return old_uid
        if "." in old_uid:
            base, suffix = old_uid.split(".", 1)
            return f"{map_primary(base)}.{suffix}"
        return map_primary(old_uid)

    doc_uid_m = re.search(r'DocUID="([^"]*)"', d_open)
    old_doc_uid = doc_uid_m.group(1) if doc_uid_m else ""
    new_doc_uid = map_primary(old_doc_uid) if old_doc_uid else _synth_hex32("doc")

    for _, blk in [*d_blocks, *m_blocks]:
        attrs = _extract_block_attrs(blk)
        u = attrs.get(("IObject", "UID"), "")
        if u and "-" not in u:
            map_any_uid(u)

    def rewrite_container(open_line: str) -> str:
        s = re.sub(r'DocUID="[^"]*"', f'DocUID="{new_doc_uid}"', open_line)
        s = re.sub(r'DocName="[^"]*"', f'DocName="{test_doc_name}"', s)
        return s

    new_d_blocks: list[tuple[str, list[str]]] = []
    for tag, blk in d_blocks:
        attrs = _extract_block_attrs(blk)
        if tag == "Rel":
            def_uid = attrs.get(("IRel", "DefUID"), "")
            u1 = map_any_uid(attrs.get(("IRel", "UID1"), ""))
            u2 = map_any_uid(attrs.get(("IRel", "UID2"), ""))
            pfx = REL_PREFIX_BY_DEF.get(def_uid, "REL")
            new_rel_uid = f"{pfx}-{u1}-{u2}"
            updated = [blk[0]]
            for ln in blk[1:-1]:
                if "<IObject " in ln:
                    ln = re.sub(r'UID="[^"]*"', f'UID="{new_rel_uid}"', ln)
                elif "<IRel " in ln:
                    ln = re.sub(r'UID1="[^"]*"', f'UID1="{u1}"', ln)
                    ln = re.sub(r'UID2="[^"]*"', f'UID2="{u2}"', ln)
                updated.append(ln)
            updated.append(blk[-1])
            new_d_blocks.append((tag, updated))
        else:
            old_u = attrs.get(("IObject", "UID"), "")
            new_u = map_any_uid(old_u)
            updated = [blk[0]]
            for ln in blk[1:-1]:
                if "<IObject " in ln:
                    ln = re.sub(r'UID="[^"]*"', f'UID="{new_u}"', ln)
                    if tag == "PIDDrawing":
                        ln = re.sub(r'Name="[^"]*"', f'Name="{test_doc_name}"', ln)
                updated.append(ln)
            updated.append(blk[-1])
            new_d_blocks.append((tag, updated))

    new_m_blocks: list[tuple[str, list[str]]] = []
    for tag, blk in m_blocks:
        attrs = _extract_block_attrs(blk)
        if tag == "Rel":
            def_uid = attrs.get(("IRel", "DefUID"), "")
            u1 = map_any_uid(attrs.get(("IRel", "UID1"), ""))
            u2 = map_any_uid(attrs.get(("IRel", "UID2"), ""))
            old_ru = attrs.get(("IObject", "UID"), "")
            if def_uid == "FileComposition" or old_ru.startswith("AAAD"):
                h = _synth_hex32(f"meta_rel:{def_uid}:{u1}:{u2}")
                new_ru = f"AAAD-{h[:4]}-{h[4:8]}"
            else:
                new_ru = _synth_hex32(f"meta_rel:{def_uid}:{u1}:{u2}")
            updated = [blk[0]]
            for ln in blk[1:-1]:
                if "<IObject " in ln:
                    ln = re.sub(r'UID="[^"]*"', f'UID="{new_ru}"', ln)
                elif "<IRel " in ln:
                    ln = re.sub(r'UID1="[^"]*"', f'UID1="{u1}"', ln)
                    ln = re.sub(r'UID2="[^"]*"', f'UID2="{u2}"', ln)
                updated.append(ln)
            updated.append(blk[-1])
            new_m_blocks.append((tag, updated))
        else:
            old_u = attrs.get(("IObject", "UID"), "")
            new_u = map_any_uid(old_u)
            updated = [blk[0]]
            for ln in blk[1:-1]:
                if "<IObject " in ln:
                    ln = re.sub(r'UID="[^"]*"', f'UID="{new_u}"', ln)
                    if tag == "DocumentVersion":
                        ln = re.sub(r'Name="[^"]*"', f'Name="{test_doc_name}"', ln)
                    elif tag == "File":
                        ln = re.sub(r'Name="[^"]*"', f'Name="{test_doc_name}.pid"', ln)
                elif "<IFile " in ln:
                    ln = re.sub(r'FilePath="[^"]*"', f'FilePath="{test_doc_name}.pid"', ln)
                updated.append(ln)
            updated.append(blk[-1])
            new_m_blocks.append((tag, updated))

    return (
        _encode_vendor_blocks(d_decl, rewrite_container(d_open), new_d_blocks, d_close),
        _encode_vendor_blocks(m_decl, rewrite_container(m_open), new_m_blocks, m_close),
    )


def pick_smallest_complete_triple(source_dir: Path) -> tuple[str, Path, Path, Path]:
    candidates: list[tuple[int, str, Path, Path, Path]] = []
    for data_p in sorted(source_dir.rglob("*_Data.xml")):
        stem = data_p.name[:-9]
        meta_p = data_p.with_name(f"{stem}_Meta.xml")
        pid_p = data_p.with_name(f"{stem}.pid")
        if meta_p.is_file() and pid_p.is_file():
            total_sz = data_p.stat().st_size + meta_p.stat().st_size + pid_p.stat().st_size
            candidates.append((total_sz, stem, pid_p, data_p, meta_p))
    if not candidates:
        raise FileNotFoundError(
            f"no complete (.pid + _Data.xml + _Meta.xml) triple under {source_dir}"
        )
    candidates.sort(key=lambda x: (x[0], x[1]))
    _, stem, pid_p, data_p, meta_p = candidates[0]
    return stem, pid_p, data_p, meta_p


def build_round1_package(
    source_dir: Path,
    out_dir: Path,
    *,
    plant: str = "P01",
    anonymize_control_filenames: bool = True,
) -> dict[str, Any]:
    spec = xml_spec_check.load_spec()
    stem, pid_src, data_src, meta_src = pick_smallest_complete_triple(source_dir)
    pid_raw = pid_src.read_bytes()
    data_raw = data_src.read_bytes()
    meta_raw = meta_src.read_bytes()
    source_sha_prefix = _sha256_bytes(pid_raw)[:12]

    file_stem = "CONTROL-SAMPLE-01" if anonymize_control_filenames else stem
    out_dir.mkdir(parents=True, exist_ok=True)

    v02_data = make_v1_02_no_topology(data_raw)
    v03_data = make_v1_03_minimal_attrs(data_raw)
    v04_data = make_v1_04_plain_xml_format(data_raw)
    v04_meta = make_v1_04_plain_xml_format(meta_raw)
    v05_data, v05_meta = make_v1_05_synthetic_uids(
        data_raw, meta_raw, test_doc_name="TEST-SYNTH-UID-01"
    )

    slot_defs = [
        (
            "00-control",
            file_stem,
            pid_raw,
            data_raw,
            meta_raw,
            "Unmodified passed SPPID sample triple (positive control)",
            True,
        ),
        (
            "01-var-xml-only",
            file_stem,
            None,
            data_raw,
            meta_raw,
            "Omit .pid binary container; submit _Data.xml + _Meta.xml only (V1-01)",
            True,
        ),
        (
            "02-var-no-topology",
            file_stem,
            pid_raw,
            v02_data,
            meta_raw,
            "Remove PIDPipeline, ports, branch points, process points, and connectivity Rels (V1-02)",
            True,
        ),
        (
            "03-var-minimal-attrs",
            file_stem,
            pid_raw,
            v03_data,
            meta_raw,
            "Retain UID/Name and @* codelist/taxonomy attributes; clear optional business attributes (V1-03)",
            True,
        ),
        (
            "04-var-plain-xml-format",
            file_stem,
            pid_raw,
            v04_data,
            v04_meta,
            "Strip UTF-8 BOM, use LF and 2-space indent with identical XML semantics (V1-04)",
            False,
        ),
        (
            "05-var-synthetic-uids",
            "TEST-SYNTH-UID-01",
            pid_raw,
            v05_data,
            v05_meta,
            "Replace all primary and derived UIDs with deterministic synthetic 32-hex UIDs (V1-05)",
            True,
        ),
    ]

    ctrl_data_path: Path | None = None
    slot_reports: list[dict[str, Any]] = []
    precheck_ok = True

    for slot_name, s_stem, s_pid, s_data, s_meta, desc, expect_strict_zero in slot_defs:
        s_dir = out_dir / slot_name
        s_dir.mkdir(parents=True, exist_ok=True)
        files_meta: list[dict[str, Any]] = []

        if s_pid is not None:
            p_f = s_dir / f"{s_stem}.pid"
            p_f.write_bytes(s_pid)
            files_meta.append(
                {
                    "path": f"{slot_name}/{p_f.name}",
                    "bytes": len(s_pid),
                    "sha256": _sha256_bytes(s_pid),
                }
            )

        d_f = s_dir / f"{s_stem}_Data.xml"
        m_f = s_dir / f"{s_stem}_Meta.xml"
        d_f.write_bytes(s_data)
        m_f.write_bytes(s_meta)
        if slot_name == "00-control":
            ctrl_data_path = d_f

        files_meta.append(
            {
                "path": f"{slot_name}/{d_f.name}",
                "bytes": len(s_data),
                "sha256": _sha256_bytes(s_data),
            }
        )
        files_meta.append(
            {
                "path": f"{slot_name}/{m_f.name}",
                "bytes": len(s_meta),
                "sha256": _sha256_bytes(s_meta),
            }
        )

        vd_strict = xml_spec_check.check_xml_bytes(
            d_f.name, "data", s_data, spec, strict=True, expected_plant=plant
        )
        vm_strict = xml_spec_check.check_xml_bytes(
            m_f.name, "meta", s_meta, spec, strict=True, expected_plant=plant
        )
        vd_schema = xml_spec_check.check_xml_bytes(
            d_f.name, "data", s_data, spec, strict=False, expected_plant=plant
        )
        vm_schema = xml_spec_check.check_xml_bytes(
            m_f.name, "meta", s_meta, spec, strict=False, expected_plant=plant
        )

        strict_cnt = len(vd_strict) + len(vm_strict)
        schema_cnt = len(vd_schema) + len(vm_schema)

        sem_diffs = 0
        if slot_name == "04-var-plain-xml-format" and ctrl_data_path is not None:
            d_ctrl = xml_diff.parse_data_xml(ctrl_data_path)
            d_cand = xml_diff.parse_data_xml(d_f)
            if set(d_ctrl.objects.keys()) != set(d_cand.objects.keys()) or set(
                d_ctrl.rels.keys()
            ) != set(d_cand.rels.keys()):
                sem_diffs += 1

        slot_ok = (schema_cnt == 0) and (
            (strict_cnt == 0)
            if expect_strict_zero
            else (strict_cnt > 0 and sem_diffs == 0)
        )
        if not slot_ok:
            precheck_ok = False

        slot_reports.append(
            {
                "slot": slot_name,
                "hypothesis": desc,
                "has_pid": s_pid is not None,
                "expect_strict_vendor_pass": expect_strict_zero,
                "strict_vendor_violations": strict_cnt,
                "schema_only_violations": schema_cnt,
                "semantic_diffs_vs_control": sem_diffs,
                "slot_precheck_passed": slot_ok,
                "files": files_meta,
            }
        )

    canonical_slots = json.dumps(slot_reports, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )
    manifest_sha256 = _sha256_bytes(canonical_slots)

    manifest = {
        "schema_version": 1,
        "round": 1,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "plant": plant,
        "source_pid_sha256_prefix": source_sha_prefix,
        "manifest_sha256": manifest_sha256,
        "sppid_c1_d_precheck_status": "unverified_no_sppid_runner",
        "precheck_passed": precheck_ok,
        "slots": slot_reports,
    }
    (out_dir / "submission-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )

    receipt_tpl = {
        "round": 1,
        "submitted_at": "",
        "responded_at": "",
        "manifest_sha256": manifest_sha256,
        "results": [
            {
                "slot": s["slot"],
                "accepted": None,
                "error_code": None,
                "raw_message": "",
            }
            for s in slot_reports
        ],
        "conclusions": [],
    }
    (out_dir / "receipt.json").write_text(
        json.dumps(receipt_tpl, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = argparse.ArgumentParser(
        description="Build Round-1 6-slot controlled subtraction submission package (#135)."
    )
    parser.add_argument(
        "--source-dir",
        required=True,
        type=Path,
        help="Directory holding passed SPPID triples",
    )
    parser.add_argument(
        "--out-dir",
        required=True,
        type=Path,
        help="External output directory for round-01 package",
    )
    parser.add_argument("--plant", default="P01", help="Expected Plant code (default: P01)")
    parser.add_argument(
        "--keep-original-stem",
        action="store_true",
        help="Keep original drawing filename in external submission slots instead of CONTROL-SAMPLE-01",
    )
    parser.add_argument(
        "--sanitized-summary",
        type=Path,
        default=None,
        help="Optional sanitized summary JSON path",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    if not args.source_dir.is_dir():
        print(f"error: source directory not found: {args.source_dir}", file=sys.stderr)
        return 2

    manifest = build_round1_package(
        args.source_dir,
        args.out_dir,
        plant=args.plant,
        anonymize_control_filenames=not args.keep_original_stem,
    )

    if args.sanitized_summary is not None:
        sanitized = {
            "round": manifest["round"],
            "plant": manifest["plant"],
            "source_pid_sha256_prefix": manifest["source_pid_sha256_prefix"],
            "manifest_sha256": manifest["manifest_sha256"],
            "precheck_passed": manifest["precheck_passed"],
            "sppid_c1_d_precheck_status": manifest["sppid_c1_d_precheck_status"],
            "slots": [
                {
                    "slot": s["slot"],
                    "has_pid": s["has_pid"],
                    "strict_vendor_violations": s["strict_vendor_violations"],
                    "schema_only_violations": s["schema_only_violations"],
                    "semantic_diffs_vs_control": s["semantic_diffs_vs_control"],
                    "slot_precheck_passed": s["slot_precheck_passed"],
                    "file_count": len(s["files"]),
                    "total_bytes": sum(f["bytes"] for f in s["files"]),
                }
                for s in manifest["slots"]
            ],
        }
        args.sanitized_summary.parent.mkdir(parents=True, exist_ok=True)
        args.sanitized_summary.write_text(
            json.dumps(sanitized, indent=2) + "\n", encoding="utf-8"
        )

    print(
        f"build_submission_round1: slots={len(manifest['slots'])} "
        f"precheck_passed={manifest['precheck_passed']} manifest_sha256={manifest['manifest_sha256'][:16]}..."
    )
    return 0 if manifest["precheck_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
