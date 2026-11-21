#!/usr/bin/env python3
"""Whole-batch deterministic TSV fingerprint & line-by-line regression diff gate (L3-8 / #133).

Pure Python standard-library CLI and helper module:
- `snapshot`: renders a strictly sanitized, deterministic line-per-sheet TSV
  (`S01..SNN` + 12-char `doc_uid` prefix, zero customer drawing names or paths)
  plus a `TOTAL` conservation row;
- `diff`: compares candidate TSV or batch summary JSON line-by-line against a
  baseline TSV, rejecting any unexpected row/column change with exit code `1`.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any, Sequence

COLUMNS: tuple[str, ...] = (
    "sheet_id",
    "doc_uid_prefix",
    "high",
    "medium",
    "low",
    "items",
    "rels",
    "pipelines",
    "connectors",
    "ports",
    "end_conns",
    "nozzle_links",
    "dead_ends_open",
)


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def render_fingerprint_tsv(summary: dict[str, Any]) -> str:
    sheets = list(summary.get("sheets") or [])
    lines: list[str] = ["\t".join(COLUMNS)]
    totals = [0] * (len(COLUMNS) - 2)

    for idx, sh in enumerate(sheets, start=1):
        sheet_id = f"S{idx:02d}"
        uid_pfx = str(sh.get("doc_uid") or "")[:12]
        cs = sh.get("confidence_summary") or {}
        ps = sh.get("piping_synthesis") or {}
        pnc = sh.get("pipe_network_census") or {}
        dea = pnc.get("dead_end_attribution") or {}

        vals = [
            int(cs.get("high") or 0),
            int(cs.get("medium") or 0),
            int(cs.get("low") or 0),
            int(sh.get("total_items_written") or 0),
            int(sh.get("total_relations_written") or 0),
            int(ps.get("pipelines") or 0),
            int(ps.get("piping_connectors") or 0),
            int(ps.get("piping_ports") or 0),
            int(ps.get("endpoint_connections") or 0),
            int(ps.get("equipment_nozzle_links") or 0),
            int(dea.get("open_free_end") or 0),
        ]
        for i, v in enumerate(vals):
            totals[i] += v
        lines.append("\t".join([sheet_id, uid_pfx, *(str(v) for v in vals)]))

    lines.append("\t".join(["TOTAL", f"N={len(sheets)}", *(str(v) for v in totals)]))
    return "\n".join(lines) + "\n"


def _parse_tsv(tsv_text: str) -> tuple[list[str], dict[str, list[str]]]:
    raw_lines = [ln.rstrip("\r") for ln in tsv_text.splitlines() if ln.strip()]
    if not raw_lines:
        return [], {}
    header = raw_lines[0].split("\t")
    rows: dict[str, list[str]] = {}
    for ln in raw_lines[1:]:
        cells = ln.split("\t")
        if cells:
            rows[cells[0]] = cells
    return header, rows


def diff_fingerprint_tsv(
    baseline_tsv: str,
    candidate_tsv: str,
    *,
    allowed_sheets: set[str] | None = None,
    allowed_columns: set[str] | None = None,
) -> dict[str, Any]:
    b_hdr, b_rows = _parse_tsv(baseline_tsv)
    c_hdr, c_rows = _parse_tsv(candidate_tsv)
    allow_sh = allowed_sheets or set()
    allow_col = allowed_columns or set()

    expected_diffs: list[dict[str, Any]] = []
    unexpected_diffs: list[dict[str, Any]] = []

    if b_hdr != c_hdr:
        unexpected_diffs.append(
            {
                "sheet_id": "HEADER",
                "column": "*",
                "expected": "\t".join(b_hdr),
                "actual": "\t".join(c_hdr),
            }
        )
        return {
            "passed": False,
            "expected_changes": 0,
            "unexpected_changes": len(unexpected_diffs),
            "unexpected_diffs": unexpected_diffs,
            "expected_diffs": expected_diffs,
        }

    all_keys = list(dict.fromkeys([*b_rows.keys(), *c_rows.keys()]))
    for key in all_keys:
        br = b_rows.get(key)
        cr = c_rows.get(key)
        if br is None or cr is None or len(br) != len(cr):
            entry = {
                "sheet_id": key,
                "column": "ROW",
                "expected": "\t".join(br) if br else "<absent>",
                "actual": "\t".join(cr) if cr else "<absent>",
            }
            if key in allow_sh and ("ROW" in allow_col or "*" in allow_col):
                expected_diffs.append(entry)
            else:
                unexpected_diffs.append(entry)
            continue

        for ci, col_name in enumerate(b_hdr):
            if br[ci] != cr[ci]:
                entry = {
                    "sheet_id": key,
                    "column": col_name,
                    "expected": br[ci],
                    "actual": cr[ci],
                }
                sheet_ok = ("*" in allow_sh) or (key in allow_sh) or (key == "TOTAL" and bool(allow_sh))
                col_ok = ("*" in allow_col) or (col_name in allow_col)
                if sheet_ok and col_ok:
                    expected_diffs.append(entry)
                else:
                    unexpected_diffs.append(entry)

    return {
        "passed": len(unexpected_diffs) == 0,
        "expected_changes": len(expected_diffs),
        "unexpected_changes": len(unexpected_diffs),
        "unexpected_diffs": unexpected_diffs,
        "expected_diffs": expected_diffs,
    }


def _load_candidate_tsv(summary_json: Path | None, candidate_tsv: Path | None) -> str:
    if candidate_tsv is not None:
        return candidate_tsv.read_text(encoding="utf-8")
    if summary_json is not None:
        payload = json.loads(summary_json.read_text(encoding="utf-8"))
        return render_fingerprint_tsv(payload)
    raise ValueError("either --summary-json or --candidate-tsv is required")


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = argparse.ArgumentParser(
        description="Whole-batch recognition & conversion TSV regression gate (L3-8 / #133)."
    )
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_snap = sub.add_parser("snapshot", help="Write sanitized TSV fingerprint from summary JSON")
    p_snap.add_argument("--summary-json", required=True, type=Path)
    p_snap.add_argument("--out-tsv", required=True, type=Path)

    p_diff = sub.add_parser("diff", help="Compare summary JSON or TSV against baseline TSV")
    p_diff.add_argument("--baseline-tsv", required=True, type=Path)
    p_diff.add_argument("--summary-json", type=Path, default=None)
    p_diff.add_argument("--candidate-tsv", type=Path, default=None)
    p_diff.add_argument("--allow-sheet", action="append", default=[])
    p_diff.add_argument("--allow-col", action="append", default=[])
    p_diff.add_argument("--out-json", type=Path, default=None)

    args = parser.parse_args(list(argv) if argv is not None else None)

    if args.cmd == "snapshot":
        if not args.summary_json.is_file():
            print(f"error: summary JSON not found: {args.summary_json}", file=sys.stderr)
            return 2
        payload = json.loads(args.summary_json.read_text(encoding="utf-8"))
        tsv = render_fingerprint_tsv(payload)
        args.out_tsv.parent.mkdir(parents=True, exist_ok=True)
        args.out_tsv.write_text(tsv, encoding="utf-8")
        print(f"regression_gate snapshot: sheets={len(payload.get('sheets') or [])} -> {args.out_tsv.name}")
        return 0

    if not args.baseline_tsv.is_file():
        print(f"error: baseline TSV not found: {args.baseline_tsv}", file=sys.stderr)
        return 2
    try:
        cand_tsv = _load_candidate_tsv(args.summary_json, args.candidate_tsv)
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    base_tsv = args.baseline_tsv.read_text(encoding="utf-8")
    res = diff_fingerprint_tsv(
        base_tsv,
        cand_tsv,
        allowed_sheets=set(args.allow_sheet),
        allowed_columns=set(args.allow_col),
    )
    if args.out_json is not None:
        args.out_json.parent.mkdir(parents=True, exist_ok=True)
        args.out_json.write_text(json.dumps(res, indent=2) + "\n", encoding="utf-8")

    for row in res["unexpected_diffs"][:30]:
        print(
            f"UNEXPECTED_DIFF\t{row['sheet_id']}\t{row['column']}\texpected={row['expected']}\tactual={row['actual']}"
        )
    print(
        f"regression_gate diff: passed={res['passed']} unexpected={res['unexpected_changes']} expected={res['expected_changes']}"
    )
    return 0 if res["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
