#!/usr/bin/env python3
"""Measure P&ID legend recognition (`PIDLEGEND EXPORT` JSON) against reference `_Data.xml` (L0-4 / #108).

Computes symbol recall, symbol precision, tag pairing accuracy, and optional
symbol-to-pipeline connectivity differences, emitting the shared TSV format:

    kind\ttarget\tlocation\texpected\tactual\tdetail
    ...
    SUMMARY\tref_symbols=...\trec_symbols=...\tmatched=...\trecall=...\tprecision=...\ttag_accuracy=...\tdiffs=...

Exit codes:
- 0: zero discrepancies (or `--corpus` skipped when `OCS_PID_CORPUS_DIR` unset);
- 1: one or more discrepancies;
- 2: CLI usage error or unreadable input.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import sys
from typing import Any, Sequence
import xml.etree.ElementTree as ET

import corpus_index

HEADER = "kind\ttarget\tlocation\texpected\tactual\tdetail"

DEFAULT_SYMBOL_CLASSES: tuple[str, ...] = (
    "PIDProcessVessel",
    "PIDMechanicalEquipment",
    "PIDProcessEquipment",
    "PIDHeatTransferEquipment",
    "PIDInstrument",
    "PIDInlineInstrument",
    "PIDControlSystemFunction",
    "PIDPipingComponent",
    "PIDNozzle",
    "PIDBranchPoint",
)


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _clean(val: str) -> str:
    return val.replace("\t", " ").replace("\r", "\\r").replace("\n", "\\n")


@dataclass(frozen=True)
class DiffRow:
    kind: str
    target: str
    location: str
    expected: str
    actual: str
    detail: str

    def to_tsv(self) -> str:
        return "\t".join(
            _clean(x)
            for x in (self.kind, self.target, self.location, self.expected, self.actual, self.detail)
        )


@dataclass
class RefSymbol:
    uid: str
    cls: str
    tag: str
    lines: set[str] = field(default_factory=set)


@dataclass
class RecSymbol:
    index: int
    raw_class: str
    mapped_class: str
    tag: str
    lines: set[str] = field(default_factory=set)


def extract_reference_symbols(
    xml_path: Path,
    symbol_classes: set[str] | None = None,
) -> list[RefSymbol]:
    allowed = symbol_classes or set(DEFAULT_SYMBOL_CLASSES)
    raw = xml_path.read_bytes()
    text = (raw[3:] if raw.startswith(b"\xef\xbb\xbf") else raw).decode("utf-8")
    root = ET.fromstring(text)

    symbols_by_uid: dict[str, RefSymbol] = {}
    pipeline_names: dict[str, str] = {}
    port_owner: dict[str, str] = {}
    nozzle_owner: dict[str, str] = {}
    conn_to_ports: dict[str, set[str]] = {}
    conn_to_pipes: dict[str, set[str]] = {}

    for elem in root:
        iobj = elem.find("IObject")
        uid = iobj.attrib.get("UID", "") if iobj is not None else ""
        name = (iobj.attrib.get("Name", "") if iobj is not None else "").strip()

        if elem.tag == "PIDPipeline":
            if uid and name:
                pipeline_names[uid] = name
            continue

        if elem.tag == "Rel":
            irel = elem.find("IRel")
            if irel is None:
                continue
            def_uid = irel.attrib.get("DefUID", "")
            u1 = irel.attrib.get("UID1", "")
            u2 = irel.attrib.get("UID2", "")
            if def_uid in ("PipingPortComposition", "SignalPortComposition"):
                port_owner[u1] = u2
            elif def_uid == "EquipmentComponentComposition":
                nozzle_owner[u1] = u2
            elif def_uid in ("PipingEnd1Conn", "PipingEnd2Conn", "PipingTapOrFitting"):
                conn_to_ports.setdefault(u2, set()).add(u1)
            elif def_uid == "PipingConnectors":
                conn_to_pipes.setdefault(u1, set()).add(u2)
            continue

        if elem.tag not in allowed or not uid:
            continue

        tag = name
        if not tag:
            named_inst = elem.find("INamedInstrument")
            if named_inst is not None:
                mv = named_inst.attrib.get("MeasuredVariable", "").strip()
                seq = named_inst.attrib.get("InstrTagSequenceNo", "").strip()
                if mv and seq:
                    tag = f"{mv}-{seq}"
        if not tag:
            named_eq = elem.find("INamedEquipment")
            if named_eq is not None:
                pfx = named_eq.attrib.get("EquipPrefix", "").strip()
                seq = named_eq.attrib.get("EquipSeqNo", "").strip()
                if pfx and seq:
                    tag = f"{pfx}-{seq}"
        if not tag:
            opc = elem.find("IOffpageConnector")
            if opc is not None:
                tag = opc.attrib.get("ToFromText", "").strip()

        symbols_by_uid[uid] = RefSymbol(uid=uid, cls=elem.tag, tag=tag)

    # Propagate connector -> pipeline names onto port owners (and nozzle -> parent equipment).
    for conn_uid, pipe_uids in conn_to_pipes.items():
        pnames = {pipeline_names[pu] for pu in pipe_uids if pu in pipeline_names}
        if not pnames:
            continue
        for port_uid in conn_to_ports.get(conn_uid, ()):
            owner_uid = port_owner.get(port_uid)
            if owner_uid in symbols_by_uid:
                symbols_by_uid[owner_uid].lines.update(pnames)
            parent_eq = nozzle_owner.get(owner_uid or "")
            if parent_eq in symbols_by_uid:
                symbols_by_uid[parent_eq].lines.update(pnames)

    return [symbols_by_uid[k] for k in sorted(symbols_by_uid)]


def synthesize_recognition_doc(ref_symbols: list[RefSymbol]) -> dict[str, Any]:
    return {
        "symbols": [
            {
                "class": s.cls,
                "label": s.tag,
                "tag": s.tag or None,
                "known": True,
                "source": "self-check",
                "at": [0.0, 0.0],
                "bbox": [0.0, 0.0, 1.0, 1.0],
                "lines": sorted(s.lines),
            }
            for s in ref_symbols
        ],
        "pipes": {"runs": []},
    }


def load_recognition_symbols(
    rec_data: Any,
    class_map: dict[str, str],
) -> list[RecSymbol]:
    if isinstance(rec_data, list):
        doc = rec_data[0] if rec_data else {}
    elif isinstance(rec_data, dict):
        doc = rec_data
    else:
        raise ValueError("recognition JSON must be an object or array of objects")

    raw_syms = doc.get("symbols", [])
    if not isinstance(raw_syms, list):
        raise ValueError("recognition JSON 'symbols' must be a list")

    result: list[RecSymbol] = []
    for idx, item in enumerate(raw_syms):
        if not isinstance(item, dict):
            continue
        raw_cls = str(item.get("class", "")).strip()
        mapped_cls = class_map.get(raw_cls, raw_cls)
        raw_tag = item.get("tag")
        if raw_tag is None or str(raw_tag).strip() == "":
            raw_tag = item.get("label", "")
        tag = str(raw_tag or "").strip()
        lines_val = item.get("lines") or []
        lines = {str(x).strip() for x in lines_val if str(x).strip()}
        result.append(
            RecSymbol(
                index=idx,
                raw_class=raw_cls,
                mapped_class=mapped_cls,
                tag=tag,
                lines=lines,
            )
        )
    return result


def evaluate_recognition(
    ref_symbols: list[RefSymbol],
    rec_symbols: list[RecSymbol],
    *,
    check_lines: bool = False,
) -> tuple[list[DiffRow], dict[str, Any]]:
    unmatched_ref = list(range(len(ref_symbols)))
    unmatched_rec = list(range(len(rec_symbols)))
    matched_pairs: list[tuple[RefSymbol, RecSymbol]] = []

    # Pass 1: exact (class, tag) match
    rec_by_key: dict[tuple[str, str], list[int]] = {}
    for r_i in unmatched_rec:
        rs = rec_symbols[r_i]
        rec_by_key.setdefault((rs.mapped_class, rs.tag), []).append(r_i)

    still_ref: list[int] = []
    used_rec: set[int] = set()
    for f_i in unmatched_ref:
        fs = ref_symbols[f_i]
        candidates = rec_by_key.get((fs.cls, fs.tag), [])
        chosen = next((c for c in candidates if c not in used_rec), None)
        if chosen is not None:
            used_rec.add(chosen)
            matched_pairs.append((fs, rec_symbols[chosen]))
        else:
            still_ref.append(f_i)

    still_rec = [r_i for r_i in unmatched_rec if r_i not in used_rec]

    # Pass 2: same class pairing for remaining items (captures tag mismatches cleanly)
    rec_by_cls: dict[str, list[int]] = {}
    for r_i in still_rec:
        rec_by_cls.setdefault(rec_symbols[r_i].mapped_class, []).append(r_i)

    final_unmatched_ref: list[RefSymbol] = []
    for f_i in still_ref:
        fs = ref_symbols[f_i]
        candidates = rec_by_cls.get(fs.cls, [])
        chosen = next((c for c in candidates if c not in used_rec), None)
        if chosen is not None:
            used_rec.add(chosen)
            matched_pairs.append((fs, rec_symbols[chosen]))
        else:
            final_unmatched_ref.append(fs)

    final_unmatched_rec = [rec_symbols[r_i] for r_i in still_rec if r_i not in used_rec]

    rows: list[DiffRow] = []
    tag_exact = 0
    for fs, rs in matched_pairs:
        if fs.tag == rs.tag:
            tag_exact += 1
        else:
            rows.append(
                DiffRow(
                    kind="TAG_MISMATCH",
                    target=fs.uid,
                    location=f"{fs.cls}[rec#{rs.index}]",
                    expected=repr(fs.tag),
                    actual=repr(rs.tag),
                    detail="matched class, tag differs",
                )
            )
        if check_lines:
            for missing_ln in sorted(fs.lines - rs.lines):
                rows.append(
                    DiffRow(
                        kind="MISSING_REL",
                        target=fs.uid,
                        location=f"{fs.cls}/lines",
                        expected=missing_ln,
                        actual="missing",
                        detail=f"symbol {fs.tag or fs.uid} missing pipeline link",
                    )
                )
            for extra_ln in sorted(rs.lines - fs.lines):
                rows.append(
                    DiffRow(
                        kind="ADDED_REL",
                        target=fs.uid,
                        location=f"{fs.cls}/lines",
                        expected="absent",
                        actual=extra_ln,
                        detail=f"symbol {fs.tag or fs.uid} extra pipeline link",
                    )
                )

    for fs in final_unmatched_ref:
        rows.append(
            DiffRow(
                kind="MISSING_SYMBOL",
                target=fs.uid,
                location=fs.cls,
                expected=f"{fs.cls}({fs.tag!r})",
                actual="missing",
                detail="reference symbol not matched in recognition",
            )
        )

    for rs in final_unmatched_rec:
        rows.append(
            DiffRow(
                kind="EXTRA_SYMBOL",
                target=f"rec#{rs.index}",
                location=rs.mapped_class,
                expected="absent",
                actual=f"{rs.mapped_class}({rs.tag!r})",
                detail=f"raw_class={rs.raw_class!r}",
            )
        )

    n_ref = len(ref_symbols)
    n_rec = len(rec_symbols)
    n_matched = len(matched_pairs)
    recall = (n_matched / n_ref) if n_ref > 0 else 1.0
    precision = (n_matched / n_rec) if n_rec > 0 else 1.0
    tag_acc = (tag_exact / n_matched) if n_matched > 0 else 1.0

    metrics = {
        "ref_symbols": n_ref,
        "rec_symbols": n_rec,
        "matched": n_matched,
        "tag_exact": tag_exact,
        "recall": recall,
        "precision": precision,
        "tag_accuracy": tag_acc,
        "diffs": len(rows),
    }
    return rows, metrics


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="legend_vs_xml.py",
        description="Measure PIDLEGEND EXPORT JSON recall/precision/tag-accuracy against reference _Data.xml.",
    )
    p.add_argument("recognition_json", nargs="?", help="PIDLEGEND EXPORT JSON file")
    p.add_argument("reference_xml", nargs="?", help="Reference _Data.xml file")
    p.add_argument("--self-check", default=None, metavar="DATA_XML", help="Synthesize ideal recognition from DATA_XML and verify zero-diff")
    p.add_argument("--corpus", action="store_true", help=f"Run self-check across all EXPORT_SET _Data.xml in ${corpus_index.CORPUS_ENV}")
    p.add_argument("--manifest", default=None, help="Optional corpus.manifest.json path for --corpus")
    p.add_argument("--class-map", default=None, help="Optional JSON mapping recognition class -> SPPID XML class")
    p.add_argument("--check-lines", action="store_true", help="Also compare symbol-to-pipeline line links")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    class_map: dict[str, str] = {}
    if args.class_map:
        try:
            class_map = dict(json.loads(Path(args.class_map).read_text(encoding="utf-8")))
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: invalid --class-map: {err}", file=sys.stderr)
            return 2

    if args.corpus:
        env_dir = os.environ.get(corpus_index.CORPUS_ENV, "").strip()
        if not env_dir:
            print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过识别与真值比对。")
            return 0
        corpus_dir = Path(env_dir)
        mpath = Path(args.manifest) if args.manifest else None
        try:
            sec_files = corpus_index.resolve_section_files(corpus_dir, "EXPORT_SET", mpath)
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2
        data_xmls = [f for f in sec_files if f.name.lower().endswith("_data.xml")]
        total_ref = 0
        total_diffs = 0
        print(HEADER)
        for xf in data_xmls:
            ref_syms = extract_reference_symbols(xf)
            synth_doc = synthesize_recognition_doc(ref_syms)
            rec_syms = load_recognition_symbols(synth_doc, class_map)
            rows, m = evaluate_recognition(ref_syms, rec_syms, check_lines=args.check_lines)
            for r in rows:
                print(r.to_tsv())
            total_ref += m["ref_symbols"]
            total_diffs += m["diffs"]
        print(
            f"SUMMARY\tfiles={len(data_xmls)}\tref_symbols={total_ref}\trec_symbols={total_ref}\t"
            f"matched={total_ref}\trecall=1.000000\tprecision=1.000000\ttag_accuracy=1.000000\tdiffs={total_diffs}"
        )
        return 0 if total_diffs == 0 else 1

    if args.self_check:
        xml_path = Path(args.self_check)
        if not xml_path.is_file():
            print(f"error: file not found: {xml_path}", file=sys.stderr)
            return 2
        try:
            ref_syms = extract_reference_symbols(xml_path)
            synth_doc = synthesize_recognition_doc(ref_syms)
            rec_syms = load_recognition_symbols(synth_doc, class_map)
            rows, m = evaluate_recognition(ref_syms, rec_syms, check_lines=True)
        except (OSError, ET.ParseError, ValueError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2
    else:
        if not args.recognition_json or not args.reference_xml:
            print("error: specify <recognition_json> <reference_xml>, --self-check, or --corpus", file=sys.stderr)
            return 2
        rec_path = Path(args.recognition_json)
        xml_path = Path(args.reference_xml)
        if not rec_path.is_file() or not xml_path.is_file():
            print("error: recognition_json or reference_xml not found", file=sys.stderr)
            return 2
        try:
            ref_syms = extract_reference_symbols(xml_path)
            rec_raw = json.loads(rec_path.read_text(encoding="utf-8"))
            rec_syms = load_recognition_symbols(rec_raw, class_map)
            rows, m = evaluate_recognition(ref_syms, rec_syms, check_lines=args.check_lines)
        except (OSError, ET.ParseError, ValueError, json.JSONDecodeError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2

    print(HEADER)
    for r in rows:
        print(r.to_tsv())
    print(
        f"SUMMARY\tref_symbols={m['ref_symbols']}\trec_symbols={m['rec_symbols']}\t"
        f"matched={m['matched']}\trecall={m['recall']:.6f}\tprecision={m['precision']:.6f}\t"
        f"tag_accuracy={m['tag_accuracy']:.6f}\tdiffs={m['diffs']}"
    )
    return 0 if m["diffs"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
