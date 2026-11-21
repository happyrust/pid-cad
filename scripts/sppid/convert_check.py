#!/usr/bin/env python3
"""End-to-end validation gate for DWG/DXF -> publish XML conversion outputs (L4-6 / L4-7 / #127).

Verifies:
1. Every `<stem>_Data.xml` and `<stem>_Meta.xml` passes `xml_spec_check` in
   `StrictVendor` mode with 0 violations;
2. Every `<stem>_Coverage.json` exists and satisfies confidence conservation;
3. Optional semantic reconciliation against reference `_Data.xml` (aligning
   named objects by `(class, Name)` so deterministic `UUID v5` conversions
   compare cleanly against transient SPPID database GUIDs), enforcing zero
   unexplained attribute/interface diffs outside `assets/pid-xml-diff-allowlist.json`.

Exit codes:
- 0: zero spec violations, zero coverage errors, and zero unexplained diffs;
- 1: one or more violations / coverage errors / unexplained diffs;
- 2: CLI usage error or unreadable input.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
from pathlib import Path
import sys
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

import xml_diff  # noqa: E402
import xml_spec_check  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ALLOWLIST_PATH = REPO_ROOT / "assets" / "pid-xml-diff-allowlist.json"


def _load_allow_rules(allowlist_path: Path | None) -> list[dict[str, Any]]:
    if allowlist_path is None:
        return []
    loaded = json.loads(allowlist_path.read_text(encoding="utf-8"))
    if isinstance(loaded, dict):
        return list(loaded.get("rules", []))
    if isinstance(loaded, list):
        return loaded
    raise ValueError("allowlist JSON must be an object with 'rules' or a list")


def _matches_rule(row: xml_diff.DiffRow, rule: dict[str, Any]) -> bool:
    for field in ("kind", "target", "location"):
        pat = rule.get(field)
        if pat is not None and not fnmatch.fnmatchcase(getattr(row, field), str(pat)):
            return False
    return True


def _compare_named_objects(
    ref_doc: xml_diff.ParsedDataDoc,
    gen_doc: xml_diff.ParsedDataDoc,
    *,
    prefix: str,
    enforce_tagged_presence: bool,
) -> tuple[list[xml_diff.DiffRow], dict[str, int]]:
    ref_by_key: dict[tuple[str, str], list[xml_diff.ParsedObject]] = {}
    gen_by_key: dict[tuple[str, str], list[xml_diff.ParsedObject]] = {}

    for obj in ref_doc.objects.values():
        if obj.cls in ("PIDDrawing", "PIDRepresentation", "PIDPipingPort", "PIDSignalPort"):
            continue
        if obj.name:
            ref_by_key.setdefault((obj.cls, obj.name), []).append(obj)

    for obj in gen_doc.objects.values():
        if obj.cls in ("PIDDrawing", "PIDRepresentation", "PIDPipingPort", "PIDSignalPort"):
            continue
        if obj.name:
            gen_by_key.setdefault((obj.cls, obj.name), []).append(obj)

    rows: list[xml_diff.DiffRow] = []
    matched_named = 0
    pfx = f"{prefix}:" if prefix else ""

    for key in sorted(set(ref_by_key.keys()) | set(gen_by_key.keys())):
        cls, name = key
        r_list = ref_by_key.get(key, [])
        g_list = gen_by_key.get(key, [])
        pair_n = min(len(r_list), len(g_list))
        for i in range(pair_n):
            matched_named += 1
            ro = r_list[i]
            go = g_list[i]
            target_id = f"{pfx}{cls}:{name}"
            for iface in sorted(set(ro.interfaces) - set(go.interfaces)):
                rows.append(
                    xml_diff.DiffRow(
                        kind="MISSING_INTERFACE",
                        target=target_id,
                        location=iface,
                        expected=f"<{iface}>",
                        actual="missing",
                        detail=f"class={cls}",
                    )
                )
            for iface in sorted(set(go.interfaces) - set(ro.interfaces)):
                rows.append(
                    xml_diff.DiffRow(
                        kind="ADDED_INTERFACE",
                        target=target_id,
                        location=iface,
                        expected="absent",
                        actual=f"<{iface}>",
                        detail=f"class={cls}",
                    )
                )
            for iface in sorted(set(ro.interfaces) & set(go.interfaces)):
                ra = {k: v for k, v in ro.interfaces[iface].items() if not (iface == "IObject" and k == "UID")}
                ga = {k: v for k, v in go.interfaces[iface].items() if not (iface == "IObject" and k == "UID")}
                for ak in sorted(set(ra) | set(ga)):
                    rv = ra.get(ak)
                    gv = ga.get(ak)
                    if rv != gv:
                        rows.append(
                            xml_diff.DiffRow(
                                kind="DIFF_ATTR",
                                target=target_id,
                                location=f"{iface}/@{ak}",
                                expected=repr(rv),
                                actual=repr(gv),
                                detail=f"class={cls}",
                            )
                        )
        if enforce_tagged_presence:
            for _ in range(len(r_list) - pair_n):
                rows.append(
                    xml_diff.DiffRow(
                        kind="MISSING_TAGGED_OBJECT",
                        target=f"{pfx}{cls}:{name}",
                        location=cls,
                        expected=f"{cls}({name!r})",
                        actual="missing",
                        detail="present in reference, absent in generated",
                    )
                )
            for _ in range(len(g_list) - pair_n):
                rows.append(
                    xml_diff.DiffRow(
                        kind="ADDED_TAGGED_OBJECT",
                        target=f"{pfx}{cls}:{name}",
                        location=cls,
                        expected="absent",
                        actual=f"{cls}({name!r})",
                        detail="extra in generated",
                    )
                )

    stats = {
        "ref_named_objects": sum(len(v) for v in ref_by_key.values()),
        "gen_named_objects": sum(len(v) for v in gen_by_key.values()),
        "matched_named_objects": matched_named,
    }
    return rows, stats


def check_converted_dir(
    gen_dir: Path,
    *,
    ref_dir: Path | None = None,
    allowlist_path: Path | None = DEFAULT_ALLOWLIST_PATH,
    expected_plant: str | None = None,
    enforce_tagged_presence: bool = False,
) -> dict[str, Any]:
    spec = xml_spec_check.load_spec()
    allow_rules = _load_allow_rules(allowlist_path) if allowlist_path and allowlist_path.is_file() else []

    data_files = sorted(gen_dir.glob("*_Data.xml"))
    if not data_files:
        return {
            "sheets_checked": 0,
            "spec_violations": 0,
            "coverage_errors": 1,
            "unexplained_diffs": 0,
            "allowlisted_diffs": 0,
            "matched_named_objects": 0,
            "ref_named_objects": 0,
            "gen_named_objects": 0,
            "errors": [f"no *_Data.xml files found in {gen_dir}"],
        }

    ref_map: dict[str, Path] = {}
    if ref_dir is not None:
        if ref_dir.is_file():
            stem = ref_dir.name[:-9] if ref_dir.name.endswith("_Data.xml") else ref_dir.stem
            ref_map[stem] = ref_dir
        elif ref_dir.is_dir():
            for rf in sorted(ref_dir.rglob("*_Data.xml")):
                ref_map[rf.name[:-9]] = rf

    spec_violations = 0
    coverage_errors = 0
    unexplained_diffs = 0
    allowlisted_diffs = 0
    tot_matched_named = 0
    tot_ref_named = 0
    tot_gen_named = 0
    errors: list[str] = []

    for df in data_files:
        stem = df.name[:-9]
        mf = gen_dir / f"{stem}_Meta.xml"
        cf = gen_dir / f"{stem}_Coverage.json"

        # 1. Strict vendor XML validation on _Data.xml and _Meta.xml
        vd = xml_spec_check.check_xml_bytes(
            str(df), "data", df.read_bytes(), spec, strict=True, expected_plant=expected_plant
        )
        for v in vd:
            errors.append(v.format_line())
        spec_violations += len(vd)

        if not mf.is_file():
            coverage_errors += 1
            errors.append(f"{stem}: missing {mf.name}")
        else:
            vm = xml_spec_check.check_xml_bytes(
                str(mf), "meta", mf.read_bytes(), spec, strict=True, expected_plant=expected_plant
            )
            for v in vm:
                errors.append(v.format_line())
            spec_violations += len(vm)

        # 2. Coverage report conservation check
        if not cf.is_file():
            coverage_errors += 1
            errors.append(f"{stem}: missing {cf.name}")
        else:
            try:
                cov = json.loads(cf.read_text(encoding="utf-8"))
                cs = cov.get("confidence_summary", {})
                conf_sum = int(cs.get("high", 0)) + int(cs.get("medium", 0)) + int(cs.get("low", 0))
                exported_syms = sum(int(v) for v in (cov.get("exported_by_class") or {}).values())
                skipped = len(cov.get("skipped_items") or [])
                if conf_sum != exported_syms + skipped:
                    coverage_errors += 1
                    errors.append(
                        f"{cf.name}: confidence sum ({conf_sum}) != exported_symbols ({exported_syms}) + skipped ({skipped})"
                    )
            except Exception as err:
                coverage_errors += 1
                errors.append(f"{cf.name}: invalid coverage JSON: {err}")

        # 3. Optional semantic reconciliation vs reference _Data.xml
        if stem in ref_map and len(vd) == 0:
            rdoc = xml_diff.parse_data_xml(ref_map[stem])
            gdoc = xml_diff.parse_data_xml(df)
            raw_rows, st = _compare_named_objects(
                rdoc,
                gdoc,
                prefix=stem if len(data_files) > 1 else "",
                enforce_tagged_presence=enforce_tagged_presence,
            )
            tot_matched_named += st["matched_named_objects"]
            tot_ref_named += st["ref_named_objects"]
            tot_gen_named += st["gen_named_objects"]
            for row in raw_rows:
                if any(_matches_rule(row, rule) for rule in allow_rules):
                    allowlisted_diffs += 1
                else:
                    unexplained_diffs += 1
                    errors.append(row.to_tsv())

    return {
        "sheets_checked": len(data_files),
        "spec_violations": spec_violations,
        "coverage_errors": coverage_errors,
        "unexplained_diffs": unexplained_diffs,
        "allowlisted_diffs": allowlisted_diffs,
        "matched_named_objects": tot_matched_named,
        "ref_named_objects": tot_ref_named,
        "gen_named_objects": tot_gen_named,
        "errors": errors,
    }


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="convert_check.py",
        description="Validate converted _Data.xml/_Meta.xml/_Coverage.json bundles and reconcile against reference XMLs.",
    )
    p.add_argument("generated_dir", help="Directory containing converted *_Data.xml, *_Meta.xml, *_Coverage.json")
    p.add_argument("--ref-dir", default=None, help="Optional reference _Data.xml file or directory")
    p.add_argument("--allowlist", default=str(DEFAULT_ALLOWLIST_PATH), help="Path to JSON diff allowlist")
    p.add_argument("--plant", default=None, help="Expected Plant attribute")
    p.add_argument("--enforce-tagged-presence", action="store_true", help="Also require every named reference object to exist or be allowlisted")
    p.add_argument("--out-json", default=None, help="Optional path to write JSON summary")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass

    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    gen_dir = Path(args.generated_dir)
    if not gen_dir.is_dir():
        print(f"error: generated directory does not exist: {gen_dir}", file=sys.stderr)
        return 2

    ref_dir = Path(args.ref_dir) if args.ref_dir else None
    allow_p = Path(args.allowlist) if args.allowlist else None
    try:
        res = check_converted_dir(
            gen_dir,
            ref_dir=ref_dir,
            allowlist_path=allow_p,
            expected_plant=args.plant,
            enforce_tagged_presence=args.enforce_tagged_presence,
        )
    except Exception as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    if args.out_json:
        Path(args.out_json).write_text(json.dumps(res, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    for e in res["errors"][:50]:
        print(e, file=sys.stderr)

    print(
        f"SUMMARY\tsheets={res['sheets_checked']}\tspec_violations={res['spec_violations']}\t"
        f"coverage_errors={res['coverage_errors']}\tunexplained_diffs={res['unexplained_diffs']}\t"
        f"allowlisted_diffs={res['allowlisted_diffs']}\tmatched_named={res['matched_named_objects']}/{res['ref_named_objects']}"
    )
    ok = (
        res["sheets_checked"] > 0
        and res["spec_violations"] == 0
        and res["coverage_errors"] == 0
        and res["unexplained_diffs"] == 0
    )
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
