#!/usr/bin/env python3
"""Validate SPPID publish `_Data.xml` and `_Meta.xml` files against `assets/pid-publish-spec.json` (L4-0 / #111).

Supports both:
- Strict vendor mode (default): checks UTF-8 BOM, CRLF line endings, trailing
  newline, exact XML declaration (`version ="1.0"`), exact space indentation,
  `<Container>` attribute order, and all structural/semantic rules.
- Schema-only mode (`--schema-only`): skips byte/whitespace formatting checks
  while enforcing all XML structural and semantic rules.

Exit codes:
- 0: zero violations across checked files (or `--corpus` skipped when `OCS_PID_CORPUS_DIR` unset);
- 1: one or more violations found (or no XML files found when explicit paths given);
- 2: CLI usage error, unreadable spec, or invalid target path.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Sequence
import xml.etree.ElementTree as ET

import corpus_index

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SPEC_PATH = REPO_ROOT / "assets" / "pid-publish-spec.json"

ATTR_NAME_RE = re.compile(r'([A-Za-z_][A-Za-z0-9_.:-]*)\s*=\s*"[^"]*"')


@dataclass(frozen=True)
class Violation:
    file: str
    code: str
    location: str
    expected: str
    actual: str

    def format_line(self) -> str:
        return (
            f"{self.file}:{self.location} [{self.code}]: "
            f"expected {self.expected}, got {self.actual}"
        )


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def infer_xml_kind(path: Path) -> str | None:
    lower = path.name.lower()
    if lower.endswith("_data.xml"):
        return "data"
    if lower.endswith("_meta.xml"):
        return "meta"
    return None


def load_spec(spec_path: Path = DEFAULT_SPEC_PATH) -> dict[str, Any]:
    return json.loads(spec_path.read_text(encoding="utf-8"))


def _extract_open_container_attr_order(text: str) -> list[str] | None:
    idx = text.find("<Container")
    if idx < 0:
        return None
    end = text.find(">", idx)
    if end < 0:
        return None
    tag_slice = text[idx:end]
    return ATTR_NAME_RE.findall(tag_slice)


def _uid_regex(spec: dict[str, Any], uid_kind: str, strict: bool) -> tuple[str, re.Pattern[str]]:
    rules = spec["uid_rules"]
    if uid_kind == "meta_file":
        key = "meta_file_strict" if strict else "meta_file_schema"
    else:
        key = uid_kind
    pattern_str = rules[key]
    return pattern_str, re.compile(pattern_str)


def check_xml_bytes(
    file_label: str,
    kind: str,
    raw: bytes,
    spec: dict[str, Any],
    *,
    strict: bool = True,
    expected_plant: str | None = None,
) -> list[Violation]:
    violations: list[Violation] = []
    fmt = spec["format"]
    indent_cfg = fmt["indent"]

    has_bom = raw.startswith(b"\xef\xbb\xbf")
    if strict and fmt["utf8_bom"] and not has_bom:
        preview = " ".join(f"{b:02X}" for b in raw[:3]) or "<empty>"
        violations.append(
            Violation(
                file=file_label,
                code="MISSING_UTF8_BOM",
                location="byte 0",
                expected="UTF-8 BOM (EF BB BF)",
                actual=f"leading bytes {preview}",
            )
        )

    body = raw[3:] if has_bom else raw
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError as err:
        violations.append(
            Violation(
                file=file_label,
                code="INVALID_UTF8",
                location=f"byte {err.start}",
                expected="valid UTF-8 byte sequence",
                actual=str(err),
            )
        )
        return violations

    if strict:
        # Check CRLF line endings and trailing newline.
        bare_lf_match = re.search(r"(?<!\r)\n", text)
        bare_cr_match = re.search(r"\r(?!\n)", text)
        if bare_lf_match is not None:
            line_no = text.count("\n", 0, bare_lf_match.start()) + 1
            violations.append(
                Violation(
                    file=file_label,
                    code="LINE_ENDING_NOT_CRLF",
                    location=f"line {line_no}",
                    expected="CRLF (\\r\\n)",
                    actual="bare LF (\\n)",
                )
            )
        elif bare_cr_match is not None:
            line_no = text.count("\r", 0, bare_cr_match.start()) + 1
            violations.append(
                Violation(
                    file=file_label,
                    code="LINE_ENDING_NOT_CRLF",
                    location=f"line {line_no}",
                    expected="CRLF (\\r\\n)",
                    actual="bare CR (\\r)",
                )
            )

        if fmt["trailing_newline"] and not text.endswith("\r\n"):
            violations.append(
                Violation(
                    file=file_label,
                    code="MISSING_TRAILING_NEWLINE",
                    location="EOF",
                    expected="trailing CRLF (\\r\\n)",
                    actual="missing trailing CRLF",
                )
            )

        raw_lines = text.splitlines()
        if not raw_lines:
            violations.append(
                Violation(
                    file=file_label,
                    code="EMPTY_DOCUMENT",
                    location="line 1",
                    expected=fmt["xml_declaration"],
                    actual="<empty>",
                )
            )
            return violations

        if raw_lines[0] != fmt["xml_declaration"]:
            violations.append(
                Violation(
                    file=file_label,
                    code="XML_DECLARATION_MISMATCH",
                    location="line 1",
                    expected=fmt["xml_declaration"],
                    actual=raw_lines[0],
                )
            )

        for idx, ln in enumerate(raw_lines[1:], start=2):
            if not ln:
                continue
            stripped = ln.lstrip(" ")
            spaces = len(ln) - len(stripped)
            if stripped.startswith("\t") or "\t" in ln[: spaces + 1]:
                violations.append(
                    Violation(
                        file=file_label,
                        code="INDENTATION_MISMATCH",
                        location=f"line {idx}",
                        expected="spaces only (no tabs)",
                        actual="tab indentation",
                    )
                )
                break
            if stripped.startswith("<Container"):
                exp_indent = indent_cfg["open_container"]
            elif stripped.startswith("</Container>"):
                exp_indent = indent_cfg["close_container"]
            elif stripped.startswith("</"):
                exp_indent = indent_cfg["close_object"]
            elif stripped.endswith("/>"):
                exp_indent = indent_cfg["interface_element"]
            elif stripped.startswith("<"):
                exp_indent = indent_cfg["open_object"]
            else:
                continue
            if spaces != exp_indent:
                violations.append(
                    Violation(
                        file=file_label,
                        code="INDENTATION_MISMATCH",
                        location=f"line {idx}",
                        expected=f"{exp_indent} leading space(s)",
                        actual=f"{spaces} leading space(s)",
                    )
                )
                break

    try:
        root = ET.fromstring(text)
    except ET.ParseError as err:
        line_no, col_no = err.position
        violations.append(
            Violation(
                file=file_label,
                code="XML_PARSE_ERROR",
                location=f"line {line_no}, col {col_no}",
                expected="well-formed XML 1.0 document",
                actual=str(err),
            )
        )
        return violations

    if root.tag != "Container":
        violations.append(
            Violation(
                file=file_label,
                code="ROOT_NOT_CONTAINER",
                location="root",
                expected="<Container>",
                actual=f"<{root.tag}>",
            )
        )
        return violations

    c_spec = spec["data_container"] if kind == "data" else spec["meta_container"]
    expected_order: list[str] = c_spec["attribute_order"]
    expected_set = set(expected_order)
    actual_attrs = root.attrib

    for key in expected_order:
        if key not in actual_attrs:
            violations.append(
                Violation(
                    file=file_label,
                    code="CONTAINER_ATTR_MISSING",
                    location="Container",
                    expected=f"attribute '{key}' present",
                    actual="missing",
                )
            )
    for key in actual_attrs:
        if key not in expected_set:
            violations.append(
                Violation(
                    file=file_label,
                    code="CONTAINER_ATTR_UNEXPECTED",
                    location=f"Container/@{key}",
                    expected="only declared Container attributes",
                    actual=f"unexpected attribute '{key}'",
                )
            )

    if strict:
        raw_order = _extract_open_container_attr_order(text)
        if raw_order is not None and raw_order != expected_order:
            violations.append(
                Violation(
                    file=file_label,
                    code="CONTAINER_ATTR_ORDER",
                    location="Container",
                    expected=",".join(expected_order),
                    actual=",".join(raw_order),
                )
            )

    for key, exp_val in c_spec["fixed_attributes"].items():
        if key in actual_attrs and actual_attrs[key] != exp_val:
            violations.append(
                Violation(
                    file=file_label,
                    code="CONTAINER_ATTR_MISMATCH",
                    location=f"Container/@{key}",
                    expected=repr(exp_val),
                    actual=repr(actual_attrs[key]),
                )
            )

    plant_val = actual_attrs.get("Plant", "")
    if expected_plant is not None:
        if plant_val != expected_plant:
            violations.append(
                Violation(
                    file=file_label,
                    code="CONTAINER_ATTR_MISMATCH",
                    location="Container/@Plant",
                    expected=repr(expected_plant),
                    actual=repr(plant_val),
                )
            )
    elif not plant_val:
        violations.append(
            Violation(
                file=file_label,
                code="CONTAINER_PLANT_EMPTY",
                location="Container/@Plant",
                expected="non-empty Plant name",
                actual=repr(plant_val),
            )
        )

    doc_uid = actual_attrs.get("DocUID", "")
    hex32_pat_str, hex32_re = _uid_regex(spec, "hex32", strict)
    if not hex32_re.match(doc_uid):
        violations.append(
            Violation(
                file=file_label,
                code="INVALID_DOC_UID",
                location="Container/@DocUID",
                expected=f"32-char uppercase hex ({hex32_pat_str})",
                actual=repr(doc_uid),
            )
        )

    children = list(root)
    if kind == "data":
        _check_data_elements(file_label, children, doc_uid, spec, strict, violations)
    else:
        _check_meta_elements(file_label, children, doc_uid, spec, strict, violations)

    return violations


def _check_class_interfaces(
    file_label: str,
    elem: ET.Element,
    elem_loc: str,
    cls_spec: dict[str, Any],
    strict: bool,
    spec: dict[str, Any],
    violations: list[Violation],
) -> str | None:
    sub_elems = list(elem)
    if not sub_elems:
        violations.append(
            Violation(
                file=file_label,
                code="MISSING_IOBJECT",
                location=elem_loc,
                expected="first child <IObject UID=\"...\"/>",
                actual="empty element",
            )
        )
        return None

    if sub_elems[0].tag != "IObject":
        violations.append(
            Violation(
                file=file_label,
                code="FIRST_INTERFACE_NOT_IOBJECT",
                location=elem_loc,
                expected="first child <IObject>",
                actual=f"<{sub_elems[0].tag}>",
            )
        )

    iobj = elem.find("IObject")
    if iobj is None:
        violations.append(
            Violation(
                file=file_label,
                code="MISSING_IOBJECT",
                location=elem_loc,
                expected="<IObject UID=\"...\"/>",
                actual="missing <IObject>",
            )
        )
        return None

    uid = iobj.attrib.get("UID", "")
    pat_str, uid_re = _uid_regex(spec, cls_spec["uid_kind"], strict)
    if not uid_re.match(uid):
        violations.append(
            Violation(
                file=file_label,
                code="INVALID_OBJECT_UID",
                location=f"{elem_loc}/IObject/@UID",
                expected=f"{cls_spec['uid_kind']} ({pat_str})",
                actual=repr(uid),
            )
        )

    present_ifaces = {c.tag for c in sub_elems}
    for req_iface in cls_spec["required_interfaces"]:
        if req_iface not in present_ifaces:
            violations.append(
                Violation(
                    file=file_label,
                    code="MISSING_REQUIRED_INTERFACE",
                    location=f"{elem_loc} (UID={uid})",
                    expected=f"required interface <{req_iface}>",
                    actual="missing",
                )
            )

    allowed_ifaces = set(cls_spec["allowed_interfaces"])
    iface_attr_specs = cls_spec["interface_attributes"]
    for child in sub_elems:
        if child.tag not in allowed_ifaces:
            violations.append(
                Violation(
                    file=file_label,
                    code="DISALLOWED_INTERFACE",
                    location=f"{elem_loc}/{child.tag}",
                    expected=f"interface allowed on <{elem.tag}>",
                    actual=f"<{child.tag}>",
                )
            )
            continue
        attr_spec = iface_attr_specs.get(child.tag)
        if attr_spec is None:
            continue
        req_attrs = attr_spec["required"]
        allowed_attrs = set(attr_spec["allowed"])
        for req_a in req_attrs:
            if req_a not in child.attrib:
                violations.append(
                    Violation(
                        file=file_label,
                        code="MISSING_REQUIRED_ATTR",
                        location=f"{elem_loc}/{child.tag}",
                        expected=f"required attribute '{req_a}'",
                        actual="missing",
                    )
                )
        for act_a in child.attrib:
            if act_a not in allowed_attrs:
                violations.append(
                    Violation(
                        file=file_label,
                        code="DISALLOWED_ATTR",
                        location=f"{elem_loc}/{child.tag}/@{act_a}",
                        expected=f"attribute allowed on <{elem.tag}>/<{child.tag}>",
                        actual=repr(act_a),
                    )
                )

    return uid


def _check_data_elements(
    file_label: str,
    children: list[ET.Element],
    doc_uid: str,
    spec: dict[str, Any],
    strict: bool,
    violations: list[Violation],
) -> None:
    if not children:
        violations.append(
            Violation(
                file=file_label,
                code="EMPTY_CONTAINER",
                location="Container",
                expected="first element <PIDDrawing>",
                actual="0 child elements",
            )
        )
        return

    first_expected = spec["data_container"]["first_element"]
    if children[0].tag != first_expected:
        violations.append(
            Violation(
                file=file_label,
                code="FIRST_ELEMENT_NOT_PID_DRAWING",
                location="Container/element[1]",
                expected=f"<{first_expected}>",
                actual=f"<{children[0].tag}>",
            )
        )
    else:
        first_iobj = children[0].find("IObject")
        first_uid = first_iobj.attrib.get("UID", "") if first_iobj is not None else ""
        if first_uid != doc_uid:
            violations.append(
                Violation(
                    file=file_label,
                    code="DRAWING_UID_MISMATCH",
                    location="Container/PIDDrawing[1]/IObject/@UID",
                    expected=f"DocUID {doc_uid!r}",
                    actual=repr(first_uid),
                )
            )

    data_classes = spec["data_classes"]
    data_relations = spec["data_relations"]
    uid_to_class: dict[str, str] = {}
    rels: list[tuple[int, ET.Element]] = []

    for idx, elem in enumerate(children, start=1):
        loc = f"element[{idx}]<{elem.tag}>"
        if elem.tag == "Rel":
            rels.append((idx, elem))
            continue
        cls_spec = data_classes.get(elem.tag)
        if cls_spec is None:
            violations.append(
                Violation(
                    file=file_label,
                    code="UNKNOWN_OBJECT_CLASS",
                    location=loc,
                    expected="one of the 21 declared PID object classes or <Rel>",
                    actual=f"<{elem.tag}>",
                )
            )
            continue
        uid = _check_class_interfaces(file_label, elem, loc, cls_spec, strict, spec, violations)
        if uid:
            if uid in uid_to_class:
                violations.append(
                    Violation(
                        file=file_label,
                        code="DUPLICATE_OBJECT_UID",
                        location=f"{loc}/IObject/@UID",
                        expected="unique object UID within document",
                        actual=repr(uid),
                    )
                )
            else:
                uid_to_class[uid] = elem.tag

    allowed_irel_attrs = {"UID1", "UID2", "DefUID", "OrderValue"}
    for idx, rel in rels:
        loc = f"element[{idx}]<Rel>"
        sub = list(rel)
        if len(sub) != 2 or sub[0].tag != "IObject" or sub[1].tag != "IRel":
            violations.append(
                Violation(
                    file=file_label,
                    code="MISSING_IREL",
                    location=loc,
                    expected="exact child sequence [<IObject>, <IRel>]",
                    actual="[" + ", ".join(f"<{c.tag}>" for c in sub) + "]",
                )
            )
            continue
        iobj, irel = sub[0], sub[1]
        rel_uid = iobj.attrib.get("UID", "")
        for req_k in ("UID1", "UID2", "DefUID"):
            if req_k not in irel.attrib:
                violations.append(
                    Violation(
                        file=file_label,
                        code="MISSING_REQUIRED_ATTR",
                        location=f"{loc}/IRel",
                        expected=f"required attribute '{req_k}'",
                        actual="missing",
                    )
                )
        for act_k in irel.attrib:
            if act_k not in allowed_irel_attrs:
                violations.append(
                    Violation(
                        file=file_label,
                        code="DISALLOWED_ATTR",
                        location=f"{loc}/IRel/@{act_k}",
                        expected="UID1, UID2, DefUID, or OrderValue",
                        actual=repr(act_k),
                    )
                )
        u1 = irel.attrib.get("UID1", "")
        u2 = irel.attrib.get("UID2", "")
        defuid = irel.attrib.get("DefUID", "")
        rel_spec = data_relations.get(defuid)
        if rel_spec is None:
            violations.append(
                Violation(
                    file=file_label,
                    code="UNKNOWN_REL_TYPE",
                    location=f"{loc}/IRel/@DefUID",
                    expected="one of the 15 declared DefUID relation types",
                    actual=repr(defuid),
                )
            )
            continue
        expected_rel_uid = f"{rel_spec['prefix']}-{u1}-{u2}"
        if rel_uid != expected_rel_uid:
            violations.append(
                Violation(
                    file=file_label,
                    code="INVALID_REL_UID_PREFIX",
                    location=f"{loc}/IObject/@UID",
                    expected=repr(expected_rel_uid),
                    actual=repr(rel_uid),
                )
            )
        c1 = uid_to_class.get(u1)
        c2 = uid_to_class.get(u2)
        if c1 is None or c2 is None:
            missing_side = f"UID1={u1!r}" if c1 is None else f"UID2={u2!r}"
            violations.append(
                Violation(
                    file=file_label,
                    code="DANGLING_REL_ENDPOINT",
                    location=f"{loc}/IRel",
                    expected="both UID1 and UID2 defined in document",
                    actual=f"unresolved {missing_side}",
                )
            )
            continue
        if [c1, c2] not in rel_spec["allowed_endpoints"]:
            allowed_desc = ", ".join(f"{p[0]} -> {p[1]}" for p in rel_spec["allowed_endpoints"])
            violations.append(
                Violation(
                    file=file_label,
                    code="DISALLOWED_REL_ENDPOINTS",
                    location=f"{loc}/IRel ({defuid})",
                    expected=f"one of [{allowed_desc}]",
                    actual=f"{c1} -> {c2}",
                )
            )


def _check_meta_elements(
    file_label: str,
    children: list[ET.Element],
    doc_uid: str,
    spec: dict[str, Any],
    strict: bool,
    violations: list[Violation],
) -> None:
    expected_seq = spec["meta_container"]["element_sequence"]
    actual_seq = [c.tag for c in children]
    if actual_seq != expected_seq:
        violations.append(
            Violation(
                file=file_label,
                code="META_SEQUENCE_MISMATCH",
                location="Container",
                expected=repr(expected_seq),
                actual=repr(actual_seq),
            )
        )

    meta_classes = spec["meta_classes"]
    meta_relations = spec["meta_relations"]
    uid_by_class: dict[str, str] = {"DocUID": doc_uid}
    rels: list[tuple[int, ET.Element]] = []

    for idx, elem in enumerate(children, start=1):
        loc = f"element[{idx}]<{elem.tag}>"
        if elem.tag == "Rel":
            rels.append((idx, elem))
            continue
        cls_spec = meta_classes.get(elem.tag)
        if cls_spec is None:
            violations.append(
                Violation(
                    file=file_label,
                    code="UNKNOWN_OBJECT_CLASS",
                    location=loc,
                    expected="DocumentVersion, DocumentRevision, File, or Rel",
                    actual=f"<{elem.tag}>",
                )
            )
            continue
        uid = _check_class_interfaces(file_label, elem, loc, cls_spec, strict, spec, violations)
        if uid:
            uid_by_class[elem.tag] = uid

    for idx, rel in rels:
        loc = f"element[{idx}]<Rel>"
        iobj = rel.find("IObject")
        irel = rel.find("IRel")
        if iobj is None or irel is None:
            violations.append(
                Violation(
                    file=file_label,
                    code="MISSING_IREL",
                    location=loc,
                    expected="both <IObject> and <IRel>",
                    actual="missing child",
                )
            )
            continue
        defuid = irel.attrib.get("DefUID", "")
        rel_spec = meta_relations.get(defuid)
        if rel_spec is None:
            violations.append(
                Violation(
                    file=file_label,
                    code="UNKNOWN_REL_TYPE",
                    location=f"{loc}/IRel/@DefUID",
                    expected="VersionedDoc, RevisedDocument, or FileComposition",
                    actual=repr(defuid),
                )
            )
            continue
        rel_uid = iobj.attrib.get("UID", "")
        pat_str, rel_re = _uid_regex(spec, rel_spec["rel_uid_kind"], strict)
        if not rel_re.match(rel_uid):
            violations.append(
                Violation(
                    file=file_label,
                    code="INVALID_OBJECT_UID",
                    location=f"{loc}/IObject/@UID",
                    expected=f"{rel_spec['rel_uid_kind']} ({pat_str})",
                    actual=repr(rel_uid),
                )
            )
        exp_u1 = uid_by_class.get(rel_spec["uid1_target"], "")
        exp_u2 = uid_by_class.get(rel_spec["uid2_target"], "")
        act_u1 = irel.attrib.get("UID1", "")
        act_u2 = irel.attrib.get("UID2", "")
        if exp_u1 and exp_u2 and (act_u1 != exp_u1 or act_u2 != exp_u2):
            violations.append(
                Violation(
                    file=file_label,
                    code="META_REL_TARGET_MISMATCH",
                    location=f"{loc}/IRel ({defuid})",
                    expected=f"UID1={exp_u1!r} ({rel_spec['uid1_target']}), UID2={exp_u2!r} ({rel_spec['uid2_target']})",
                    actual=f"UID1={act_u1!r}, UID2={act_u2!r}",
                )
            )


def _collect_target_xml_files(targets: Sequence[str]) -> list[Path]:
    collected: list[Path] = []
    for raw in targets:
        p = Path(raw)
        if not p.exists():
            raise FileNotFoundError(f"path does not exist: {p}")
        if p.is_file():
            if infer_xml_kind(p) is None:
                raise ValueError(f"unsupported filename {p.name!r} (expected *_Data.xml or *_Meta.xml)")
            collected.append(p)
        else:
            for candidate in sorted(p.rglob("*.xml")):
                if infer_xml_kind(candidate) is not None:
                    collected.append(candidate)
    return collected


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="xml_spec_check.py",
        description="Validate SPPID publish _Data.xml and _Meta.xml files against assets/pid-publish-spec.json.",
    )
    parser.add_argument("paths", nargs="*", help="XML files or directories to scan")
    parser.add_argument(
        "--corpus",
        action="store_true",
        help=f"Validate EXPORT_SET files resolved via ${corpus_index.CORPUS_ENV}",
    )
    parser.add_argument("--manifest", default=None, help="Optional corpus.manifest.json path when using --corpus")
    parser.add_argument("--spec", default=str(DEFAULT_SPEC_PATH), help="Path to pid-publish-spec.json")
    parser.add_argument(
        "--schema-only",
        action="store_true",
        help="Skip UTF-8 BOM / CRLF / space-indent / attribute-order checks",
    )
    parser.add_argument(
        "--plant",
        default=None,
        help="Require Container/@Plant to match this exact value (default: any non-empty Plant)",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    try:
        spec = load_spec(Path(args.spec))
    except (OSError, json.JSONDecodeError) as err:
        print(f"error: cannot load spec: {err}", file=sys.stderr)
        return 2

    files: list[Path] = []
    if args.corpus:
        env_dir = os.environ.get(corpus_index.CORPUS_ENV, "").strip()
        if not env_dir:
            print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过 XML 规格检查。")
            return 0
        corpus_dir = Path(env_dir)
        if not corpus_dir.is_dir():
            print(f"error: corpus directory does not exist: {corpus_dir}", file=sys.stderr)
            return 2
        mpath = Path(args.manifest) if args.manifest else None
        try:
            section_files = corpus_index.resolve_section_files(corpus_dir, "EXPORT_SET", mpath)
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: cannot read corpus manifest: {err}", file=sys.stderr)
            return 2
        files.extend(p for p in section_files if infer_xml_kind(p) is not None)

    if args.paths:
        try:
            files.extend(_collect_target_xml_files(args.paths))
        except (OSError, ValueError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2

    if not args.corpus and not args.paths:
        print("error: specify one or more paths or --corpus", file=sys.stderr)
        return 2

    if not files:
        print("error: no *_Data.xml or *_Meta.xml files found to check", file=sys.stderr)
        return 1

    strict = not args.schema_only
    total_violations = 0
    for fpath in files:
        kind = infer_xml_kind(fpath)
        if kind is None:
            continue
        try:
            raw = fpath.read_bytes()
        except OSError as err:
            print(f"error: cannot read {fpath}: {err}", file=sys.stderr)
            return 2
        vs = check_xml_bytes(
            str(fpath),
            kind,
            raw,
            spec,
            strict=strict,
            expected_plant=args.plant,
        )
        for v in vs:
            print(v.format_line(), file=sys.stderr)
        total_violations += len(vs)

    if total_violations > 0:
        print(
            f"checked {len(files)} file(s), {total_violations} violation(s)",
            file=sys.stderr,
        )
        return 1

    print(f"checked {len(files)} file(s), 0 violation(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
