#!/usr/bin/env python3
"""Compare two SPPID `_Data.xml` files or directories item-by-item (L0-4 / #108).

Aligns objects by `UID` (with optional `PIDPipeline` alignment by unique
`IObject/@Name` so deterministic `UUID v5` pipelines match transient SPPID
pipeline GUIDs cleanly), compares object classes, interface sets, attribute
sets/values, and relation sets `(DefUID, UID1, UID2)`, and prints a fixed
machine-parsable TSV table:

    kind\ttarget\tlocation\texpected\tactual\tdetail
    ...
    SUMMARY\tfiles=N\tdiffs=D\tallowlisted=A\t...

Exit codes:
- 0: zero unexplained diffs (or `--corpus` skipped when `OCS_PID_CORPUS_DIR` unset);
- 1: one or more unexplained diffs;
- 2: CLI usage error or unreadable XML/JSON.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import fnmatch
import json
import os
from pathlib import Path
import sys
from typing import Any, Sequence
import xml.etree.ElementTree as ET

import corpus_index

HEADER = "kind\ttarget\tlocation\texpected\tactual\tdetail"


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _clean_cell(val: str) -> str:
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
            _clean_cell(x)
            for x in (self.kind, self.target, self.location, self.expected, self.actual, self.detail)
        )


@dataclass
class ParsedObject:
    uid: str
    cls: str
    name: str
    interfaces: dict[str, dict[str, str]]


@dataclass
class ParsedRel:
    rel_uid: str
    def_uid: str
    uid1: str
    uid2: str
    extra_attrs: dict[str, str]


@dataclass
class ParsedDataDoc:
    doc_uid: str
    doc_name: str
    objects: dict[str, ParsedObject]
    rels: dict[tuple[str, str, str], ParsedRel]


def parse_data_xml(path: Path) -> ParsedDataDoc:
    raw = path.read_bytes()
    text = (raw[3:] if raw.startswith(b"\xef\xbb\xbf") else raw).decode("utf-8")
    root = ET.fromstring(text)
    doc_uid = root.attrib.get("DocUID", "")
    doc_name = root.attrib.get("DocName", "")
    objects: dict[str, ParsedObject] = {}
    rels: dict[tuple[str, str, str], ParsedRel] = {}

    for elem in root:
        iobj = elem.find("IObject")
        uid = iobj.attrib.get("UID", "") if iobj is not None else ""
        if elem.tag == "Rel":
            irel = elem.find("IRel")
            if irel is None:
                continue
            def_uid = irel.attrib.get("DefUID", "")
            u1 = irel.attrib.get("UID1", "")
            u2 = irel.attrib.get("UID2", "")
            extra = {k: v for k, v in irel.attrib.items() if k not in ("DefUID", "UID1", "UID2")}
            rels[(def_uid, u1, u2)] = ParsedRel(
                rel_uid=uid,
                def_uid=def_uid,
                uid1=u1,
                uid2=u2,
                extra_attrs=extra,
            )
        else:
            ifaces: dict[str, dict[str, str]] = {}
            for sub in elem:
                attrs = dict(sub.attrib)
                if sub.tag == "IObject":
                    attrs.pop("UID", None)
                ifaces[sub.tag] = attrs
            name = iobj.attrib.get("Name", "") if iobj is not None else ""
            objects[uid] = ParsedObject(uid=uid, cls=elem.tag, name=name, interfaces=ifaces)

    return ParsedDataDoc(doc_uid=doc_uid, doc_name=doc_name, objects=objects, rels=rels)


def _align_pipeline_uids(ref_doc: ParsedDataDoc, gen_doc: ParsedDataDoc) -> dict[str, str]:
    """Map generated PIDPipeline UID -> reference PIDPipeline UID when Name uniquely matches."""
    ref_pipes_by_name: dict[str, list[str]] = {}
    for uid, obj in ref_doc.objects.items():
        if obj.cls == "PIDPipeline" and obj.name and uid not in gen_doc.objects:
            ref_pipes_by_name.setdefault(obj.name, []).append(uid)

    gen_pipes_by_name: dict[str, list[str]] = {}
    for uid, obj in gen_doc.objects.items():
        if obj.cls == "PIDPipeline" and obj.name and uid not in ref_doc.objects:
            gen_pipes_by_name.setdefault(obj.name, []).append(uid)

    remap: dict[str, str] = {}
    for name, ref_uids in ref_pipes_by_name.items():
        gen_uids = gen_pipes_by_name.get(name, [])
        if len(ref_uids) == 1 and len(gen_uids) == 1:
            remap[gen_uids[0]] = ref_uids[0]
    return remap


def diff_data_docs(
    ref_doc: ParsedDataDoc,
    gen_doc: ParsedDataDoc,
    *,
    file_prefix: str = "",
    strict_pipeline_uid: bool = False,
) -> list[DiffRow]:
    uid_remap: dict[str, str] = {} if strict_pipeline_uid else _align_pipeline_uids(ref_doc, gen_doc)

    gen_objects: dict[str, ParsedObject] = {}
    for uid, obj in gen_doc.objects.items():
        mapped = uid_remap.get(uid, uid)
        gen_objects[mapped] = obj

    gen_rels: dict[tuple[str, str, str], ParsedRel] = {}
    for (def_uid, u1, u2), r in gen_doc.rels.items():
        mu1 = uid_remap.get(u1, u1)
        mu2 = uid_remap.get(u2, u2)
        gen_rels[(def_uid, mu1, mu2)] = r

    rows: list[DiffRow] = []
    pfx = f"{file_prefix}:" if file_prefix else ""

    for uid in sorted(ref_doc.objects.keys() - gen_objects.keys()):
        ro = ref_doc.objects[uid]
        rows.append(
            DiffRow(
                kind="MISSING_OBJECT",
                target=f"{pfx}{uid}",
                location=ro.cls,
                expected=f"{ro.cls} (Name={ro.name!r})",
                actual="missing",
                detail="present in reference, absent in generated",
            )
        )

    for uid in sorted(gen_objects.keys() - ref_doc.objects.keys()):
        go = gen_objects[uid]
        rows.append(
            DiffRow(
                kind="ADDED_OBJECT",
                target=f"{pfx}{uid}",
                location=go.cls,
                expected="absent",
                actual=f"{go.cls} (Name={go.name!r})",
                detail="absent in reference, added in generated",
            )
        )

    for uid in sorted(ref_doc.objects.keys() & gen_objects.keys()):
        ro = ref_doc.objects[uid]
        go = gen_objects[uid]
        if ro.cls != go.cls:
            rows.append(
                DiffRow(
                    kind="CLASS_MISMATCH",
                    target=f"{pfx}{uid}",
                    location="class",
                    expected=ro.cls,
                    actual=go.cls,
                    detail="object class differs for same UID",
                )
            )
            continue

        for iface in sorted(ro.interfaces.keys() - go.interfaces.keys()):
            rows.append(
                DiffRow(
                    kind="MISSING_INTERFACE",
                    target=f"{pfx}{uid}",
                    location=iface,
                    expected=f"<{iface}>",
                    actual="missing",
                    detail=f"class={ro.cls}",
                )
            )

        for iface in sorted(go.interfaces.keys() - ro.interfaces.keys()):
            rows.append(
                DiffRow(
                    kind="ADDED_INTERFACE",
                    target=f"{pfx}{uid}",
                    location=iface,
                    expected="absent",
                    actual=f"<{iface}>",
                    detail=f"class={go.cls}",
                )
            )

        for iface in sorted(ro.interfaces.keys() & go.interfaces.keys()):
            ra = ro.interfaces[iface]
            ga = go.interfaces[iface]
            for attr in sorted(ra.keys() - ga.keys()):
                rows.append(
                    DiffRow(
                        kind="MISSING_ATTR",
                        target=f"{pfx}{uid}",
                        location=f"{iface}/@{attr}",
                        expected=repr(ra[attr]),
                        actual="missing",
                        detail=f"class={ro.cls}",
                    )
                )
            for attr in sorted(ga.keys() - ra.keys()):
                rows.append(
                    DiffRow(
                        kind="ADDED_ATTR",
                        target=f"{pfx}{uid}",
                        location=f"{iface}/@{attr}",
                        expected="absent",
                        actual=repr(ga[attr]),
                        detail=f"class={go.cls}",
                    )
                )
            for attr in sorted(ra.keys() & ga.keys()):
                if ra[attr] != ga[attr]:
                    rows.append(
                        DiffRow(
                            kind="ATTR_DIFF",
                            target=f"{pfx}{uid}",
                            location=f"{iface}/@{attr}",
                            expected=repr(ra[attr]),
                            actual=repr(ga[attr]),
                            detail=f"class={ro.cls}",
                        )
                    )

    for rel_key in sorted(ref_doc.rels.keys() - gen_rels.keys()):
        def_uid, u1, u2 = rel_key
        rows.append(
            DiffRow(
                kind="MISSING_REL",
                target=f"{pfx}{def_uid}",
                location=f"{u1}->{u2}",
                expected=f"{def_uid}({u1}->{u2})",
                actual="missing",
                detail="relation missing in generated",
            )
        )

    for rel_key in sorted(gen_rels.keys() - ref_doc.rels.keys()):
        def_uid, u1, u2 = rel_key
        rows.append(
            DiffRow(
                kind="ADDED_REL",
                target=f"{pfx}{def_uid}",
                location=f"{u1}->{u2}",
                expected="absent",
                actual=f"{def_uid}({u1}->{u2})",
                detail="extra relation in generated",
            )
        )

    for rel_key in sorted(ref_doc.rels.keys() & gen_rels.keys()):
        rr = ref_doc.rels[rel_key]
        gr = gen_rels[rel_key]
        if rr.extra_attrs != gr.extra_attrs:
            def_uid, u1, u2 = rel_key
            rows.append(
                DiffRow(
                    kind="REL_DIFF",
                    target=f"{pfx}{def_uid}",
                    location=f"{u1}->{u2}",
                    expected=json.dumps(rr.extra_attrs, sort_keys=True),
                    actual=json.dumps(gr.extra_attrs, sort_keys=True),
                    detail="relation extra attributes differ",
                )
            )

    return rows


def _matches_allow_rule(row: DiffRow, rule: dict[str, Any]) -> bool:
    for field in ("kind", "target", "location"):
        pat = rule.get(field)
        if pat is not None and not fnmatch.fnmatchcase(getattr(row, field), str(pat)):
            return False
    return True


def _apply_allowlist(rows: list[DiffRow], rules: list[dict[str, Any]]) -> tuple[list[DiffRow], int]:
    if not rules:
        return rows, 0
    kept: list[DiffRow] = []
    allowed_count = 0
    for r in rows:
        if any(_matches_allow_rule(r, rule) for rule in rules):
            allowed_count += 1
        else:
            kept.append(r)
    return kept, allowed_count


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="xml_diff.py",
        description="Compare reference and generated _Data.xml files or folders.",
    )
    p.add_argument("reference", nargs="?", help="Reference _Data.xml file or folder")
    p.add_argument("generated", nargs="?", help="Generated _Data.xml file or folder")
    p.add_argument("--corpus", action="store_true", help=f"Run self-check (or vs --generated-dir) on ${corpus_index.CORPUS_ENV}")
    p.add_argument("--manifest", default=None, help="Optional corpus.manifest.json path for --corpus")
    p.add_argument("--generated-dir", default=None, help="Optional generated folder to compare against corpus EXPORT_SET")
    p.add_argument("--allowlist", default=None, help="JSON file of explained/whitelisted diff rules")
    p.add_argument("--strict-pipeline-uid", action="store_true", help="Do not align PIDPipeline by Name when UID differs")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    allow_rules: list[dict[str, Any]] = []
    if args.allowlist:
        try:
            loaded = json.loads(Path(args.allowlist).read_text(encoding="utf-8"))
            if isinstance(loaded, dict):
                allow_rules = list(loaded.get("rules", []))
            elif isinstance(loaded, list):
                allow_rules = loaded
            else:
                raise ValueError("allowlist JSON must be a list or {'rules': [...]}")
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: invalid allowlist: {err}", file=sys.stderr)
            return 2

    pairs: list[tuple[str, Path, Path]] = []
    if args.corpus:
        env_dir = os.environ.get(corpus_index.CORPUS_ENV, "").strip()
        if not env_dir:
            print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过 XML 比对。")
            return 0
        corpus_dir = Path(env_dir)
        mpath = Path(args.manifest) if args.manifest else None
        try:
            sec_files = corpus_index.resolve_section_files(corpus_dir, "EXPORT_SET", mpath)
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2
        ref_xmls = [f for f in sec_files if f.name.lower().endswith("_data.xml")]
        if args.generated_dir:
            gdir = Path(args.generated_dir)
            gen_by_name = {f.name: f for f in gdir.rglob("*_Data.xml")}
            for rf in ref_xmls:
                gf = gen_by_name.get(rf.name)
                if gf is not None:
                    pairs.append((rf.stem[:-5], rf, gf))
        else:
            for rf in ref_xmls:
                pairs.append((rf.stem[:-5], rf, rf))
    else:
        if not args.reference or not args.generated:
            print("error: specify <reference> <generated> or --corpus", file=sys.stderr)
            return 2
        ref_path = Path(args.reference)
        gen_path = Path(args.generated)
        if not ref_path.exists() or not gen_path.exists():
            print("error: reference or generated path does not exist", file=sys.stderr)
            return 2
        if ref_path.is_file() and gen_path.is_file():
            pairs.append(("", ref_path, gen_path))
        elif ref_path.is_dir() and gen_path.is_dir():
            ref_map = {p.name: p for p in sorted(ref_path.rglob("*_Data.xml"))}
            gen_map = {p.name: p for p in sorted(gen_path.rglob("*_Data.xml"))}
            for name in sorted(ref_map.keys() & gen_map.keys()):
                pairs.append((name[:-9], ref_map[name], gen_map[name]))
        else:
            print("error: reference and generated must both be files or both be directories", file=sys.stderr)
            return 2

    if not pairs:
        print("error: no _Data.xml pairs found", file=sys.stderr)
        return 2

    all_rows: list[DiffRow] = []
    try:
        for label, rf, gf in pairs:
            rdoc = parse_data_xml(rf)
            gdoc = parse_data_xml(gf)
            all_rows.extend(
                diff_data_docs(
                    rdoc,
                    gdoc,
                    file_prefix=label,
                    strict_pipeline_uid=args.strict_pipeline_uid,
                )
            )
    except (OSError, UnicodeDecodeError, ET.ParseError) as err:
        print(f"error: XML parse failure: {err}", file=sys.stderr)
        return 2

    unexplained, allowlisted = _apply_allowlist(all_rows, allow_rules)
    print(HEADER)
    for r in unexplained:
        print(r.to_tsv())
    print(
        f"SUMMARY\tfiles={len(pairs)}\tdiffs={len(unexplained)}\t"
        f"allowlisted={allowlisted}\traw_diffs={len(all_rows)}"
    )
    return 0 if len(unexplained) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
