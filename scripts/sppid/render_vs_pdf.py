#!/usr/bin/env python3
"""Compare rendered PDF(s) against reference PDF(s) or `pid_plot_compare` metrics (L0-4 / #108).

Reuses the tolerances established in `scripts/sppid/pdf_compare.py`
(`COVERAGE_WITHIN = 0.001`, `LENGTH_WITHIN_MM = 1.0`) and emits the shared
machine-parsable TSV format:

    kind\ttarget\tlocation\texpected\tactual\tdetail
    ...
    SUMMARY\tsheets=...\tcoverage=...\textra_length_mm=...\ttext_matched=...\tdiffs=...

Modes:
1. `--metrics-json <file_or_dir> [--baseline-json <file_or_dir>]`:
   Evaluates `pid_plot_compare` JSON reports against baseline records or thresholds.
2. `<reference_pdf> <rendered_pdf>`:
   Pure-stdlib PDF content comparison (page count, MediaBox, stroke segments,
   and text literals; or delegates to `--compare-bin` when given).
3. `--corpus`:
   Runs self-check on `EXPORT_PDF` in `$OCS_PID_CORPUS_DIR` (skips with exit 0
   when unset).
"""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Any, Sequence
import zlib

import corpus_index
from pdf_compare import COVERAGE_WITHIN, LENGTH_WITHIN_MM

HEADER = "kind\ttarget\tlocation\texpected\tactual\tdetail"

MEDIABOX_RE = re.compile(rb"/MediaBox\s*\[\s*([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\s*\]")
STREAM_RE = re.compile(rb"stream\r?\n(.*?)\r?\nendstream", re.DOTALL)
PAREN_TEXT_RE = re.compile(r"\(([^()\\]*(?:\\.[^()\\]*)*)\)\s*Tj")
HEX_TEXT_RE = re.compile(r"<([0-9A-Fa-f]+)>\s*Tj")
LINE_OP_RE = re.compile(
    r"([-\d.]+)\s+([-\d.]+)\s+m\s+([-\d.]+)\s+([-\d.]+)\s+l"
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
class PdfDigest:
    media_boxes: list[tuple[float, float, float, float]]
    strokes: Counter[str]
    texts: list[str]


def parse_pdf_stdlib(path: Path) -> PdfDigest:
    raw = path.read_bytes()
    boxes = [
        (float(m.group(1)), float(m.group(2)), float(m.group(3)), float(m.group(4)))
        for m in MEDIABOX_RE.finditer(raw)
    ]
    strokes: Counter[str] = Counter()
    texts: list[str] = []

    for sm in STREAM_RE.finditer(raw):
        chunk = sm.group(1)
        decoded_bytes = chunk
        for wbits in (zlib.MAX_WBITS, -zlib.MAX_WBITS):
            try:
                decoded_bytes = zlib.decompress(chunk, wbits)
                break
            except zlib.error:
                continue
        content = decoded_bytes.decode("latin-1", errors="replace")
        for lm in LINE_OP_RE.finditer(content):
            seg = f"{float(lm.group(1)):.3f},{float(lm.group(2)):.3f}->{float(lm.group(3)):.3f},{float(lm.group(4)):.3f}"
            strokes[seg] += 1
        for pm in PAREN_TEXT_RE.finditer(content):
            texts.append(pm.group(1))
        for hm in HEX_TEXT_RE.finditer(content):
            texts.append(f"<{hm.group(1).upper()}>")

    return PdfDigest(media_boxes=boxes, strokes=strokes, texts=texts)


def _segment_len_mm(seg_key: str) -> float:
    p1, p2 = seg_key.split("->", 1)
    x1, y1 = (float(v) for v in p1.split(","))
    x2, y2 = (float(v) for v in p2.split(","))
    pts = math.hypot(x2 - x1, y2 - y1)
    return pts * 25.4 / 72.0


def compare_pdfs_stdlib(
    label: str,
    ref_path: Path,
    gen_path: Path,
) -> tuple[list[DiffRow], dict[str, Any]]:
    rows: list[DiffRow] = []
    if ref_path.read_bytes() == gen_path.read_bytes():
        ref_d = parse_pdf_stdlib(ref_path)
        n_txt = len(ref_d.texts)
        return [], {
            "coverage": 1.0,
            "extra_length_mm": 0.0,
            "text_matched": n_txt,
            "text_reference": n_txt,
        }

    ref_d = parse_pdf_stdlib(ref_path)
    gen_d = parse_pdf_stdlib(gen_path)

    if len(ref_d.media_boxes) != len(gen_d.media_boxes) or any(
        any(abs(a - b) > 0.05 for a, b in zip(rb, gb))
        for rb, gb in zip(ref_d.media_boxes, gen_d.media_boxes)
    ):
        rows.append(
            DiffRow(
                kind="PAGE_SIZE_DIFF",
                target=label,
                location="MediaBox",
                expected=repr(ref_d.media_boxes),
                actual=repr(gen_d.media_boxes),
                detail="page count or MediaBox differs",
            )
        )

    # Stroke coverage & extra length
    ref_total_mm = sum(_segment_len_mm(k) * c for k, c in ref_d.strokes.items())
    covered_mm = sum(
        _segment_len_mm(k) * min(c, gen_d.strokes.get(k, 0))
        for k, c in ref_d.strokes.items()
    )
    extra_mm = sum(
        _segment_len_mm(k) * max(0, c - ref_d.strokes.get(k, 0))
        for k, c in gen_d.strokes.items()
    )
    coverage = (covered_mm / ref_total_mm) if ref_total_mm > 0 else (1.0 if not gen_d.strokes else 0.0)

    if coverage < 1.0 - COVERAGE_WITHIN:
        rows.append(
            DiffRow(
                kind="COVERAGE_LOW",
                target=label,
                location="strokes",
                expected=">= 0.999000",
                actual=f"{coverage:.6f}",
                detail=f"covered_mm={covered_mm:.3f}/{ref_total_mm:.3f}",
            )
        )
    if extra_mm > LENGTH_WITHIN_MM:
        rows.append(
            DiffRow(
                kind="EXTRA_INK_HIGH",
                target=label,
                location="strokes",
                expected=f"<= {LENGTH_WITHIN_MM:.3f} mm",
                actual=f"{extra_mm:.3f} mm",
                detail="extra stroke length above tolerance",
            )
        )

    ref_tc = Counter(ref_d.texts)
    gen_tc = Counter(gen_d.texts)
    matched_text = sum(min(c, gen_tc.get(k, 0)) for k, c in ref_tc.items())
    if ref_tc != gen_tc:
        missing_t = list((ref_tc - gen_tc).elements())
        extra_t = list((gen_tc - ref_tc).elements())
        rows.append(
            DiffRow(
                kind="TEXT_DIFF",
                target=label,
                location="text",
                expected=repr(missing_t) if missing_t else "no extra text",
                actual=repr(extra_t) if extra_t else "missing",
                detail=f"matched={matched_text}/{len(ref_d.texts)}",
            )
        )

    return rows, {
        "coverage": coverage,
        "extra_length_mm": extra_mm,
        "text_matched": matched_text,
        "text_reference": len(ref_d.texts),
    }


def evaluate_compare_json(
    label: str,
    cur: dict[str, Any],
    base: dict[str, Any] | None,
    *,
    min_coverage: float = 0.98,
    max_extra_mm: float = 200.0,
) -> tuple[list[DiffRow], dict[str, Any]]:
    rows: list[DiffRow] = []
    cov = float(cur["coverage"])
    extra = float(cur["extra_length_mm"])
    t_match = int(cur["text_matched"])
    t_ref = int(cur["text_reference"])
    fit_ok = cur.get("fit", {}).get("within_threshold") if isinstance(cur.get("fit"), dict) else None

    if base is not None:
        b_cov = float(base["coverage"])
        b_extra = float(base["extra_length_mm"])
        b_tmatch = int(base["text_matched"])
        b_fit = base.get("fit", {}).get("within_threshold") if isinstance(base.get("fit"), dict) else None

        if cov < b_cov - COVERAGE_WITHIN:
            rows.append(
                DiffRow(
                    kind="COVERAGE_LOW",
                    target=label,
                    location="coverage",
                    expected=f">= {b_cov - COVERAGE_WITHIN:.6f} (record {b_cov:.6f})",
                    actual=f"{cov:.6f}",
                    detail="coverage regressed below baseline tolerance",
                )
            )
        if extra > b_extra + LENGTH_WITHIN_MM:
            rows.append(
                DiffRow(
                    kind="EXTRA_INK_HIGH",
                    target=label,
                    location="extra_length_mm",
                    expected=f"<= {b_extra + LENGTH_WITHIN_MM:.3f} (record {b_extra:.3f})",
                    actual=f"{extra:.3f}",
                    detail="extra ink length increased above baseline tolerance",
                )
            )
        if t_match < b_tmatch:
            rows.append(
                DiffRow(
                    kind="TEXT_MISSING",
                    target=label,
                    location="text_matched",
                    expected=f">= {b_tmatch}",
                    actual=str(t_match),
                    detail=f"text_reference={t_ref}",
                )
            )
        if b_fit is True and fit_ok is not True:
            rows.append(
                DiffRow(
                    kind="FIT_FAILED",
                    target=label,
                    location="fit.within_threshold",
                    expected="True",
                    actual=repr(fit_ok),
                    detail="page registration fit regressed",
                )
            )
    else:
        if cov < min_coverage:
            rows.append(
                DiffRow(
                    kind="COVERAGE_LOW",
                    target=label,
                    location="coverage",
                    expected=f">= {min_coverage:.6f}",
                    actual=f"{cov:.6f}",
                    detail="below minimum coverage threshold",
                )
            )
        if extra > max_extra_mm:
            rows.append(
                DiffRow(
                    kind="EXTRA_INK_HIGH",
                    target=label,
                    location="extra_length_mm",
                    expected=f"<= {max_extra_mm:.3f}",
                    actual=f"{extra:.3f}",
                    detail="above maximum extra ink threshold",
                )
            )
        if fit_ok is False:
            rows.append(
                DiffRow(
                    kind="FIT_FAILED",
                    target=label,
                    location="fit.within_threshold",
                    expected="True",
                    actual="False",
                    detail="page registration fit outside threshold",
                )
            )

    return rows, {
        "coverage": cov,
        "extra_length_mm": extra,
        "text_matched": t_match,
        "text_reference": t_ref,
    }


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="render_vs_pdf.py",
        description="Compare rendered PDF(s) against reference PDF(s) or pid_plot_compare metrics.",
    )
    p.add_argument("reference_pdf", nargs="?", help="Reference PDF file or folder")
    p.add_argument("rendered_pdf", nargs="?", help="Rendered PDF file or folder")
    p.add_argument("--metrics-json", default=None, help="pid_plot_compare JSON file or directory")
    p.add_argument("--baseline-json", default=None, help="Baseline pid_plot_compare JSON file or directory")
    p.add_argument("--compare-bin", default=None, help="Optional pid_plot_compare executable")
    p.add_argument("--pid", default=None, help="Drawing .pid path when invoking --compare-bin")
    p.add_argument("--corpus", action="store_true", help=f"Run self-check on EXPORT_PDF in ${corpus_index.CORPUS_ENV}")
    p.add_argument("--manifest", default=None, help="Optional corpus.manifest.json path for --corpus")
    p.add_argument("--min-coverage", type=float, default=0.98)
    p.add_argument("--max-extra-mm", type=float, default=200.0)
    return p


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    all_rows: list[DiffRow] = []
    stats_list: list[dict[str, Any]] = []

    if args.corpus:
        env_dir = os.environ.get(corpus_index.CORPUS_ENV, "").strip()
        if not env_dir:
            print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过显示 PDF 比对。")
            return 0
        corpus_dir = Path(env_dir)
        mpath = Path(args.manifest) if args.manifest else None
        try:
            pdfs = [
                f for f in corpus_index.resolve_section_files(corpus_dir, "EXPORT_PDF", mpath)
                if f.suffix.lower() == ".pdf"
            ]
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2
        for pf in pdfs:
            rows, st = compare_pdfs_stdlib(pf.stem, pf, pf)
            all_rows.extend(rows)
            stats_list.append(st)
    elif args.metrics_json:
        mpath = Path(args.metrics_json)
        bpath = Path(args.baseline_json) if args.baseline_json else None
        if not mpath.exists() or (bpath is not None and not bpath.exists()):
            print("error: metrics-json or baseline-json does not exist", file=sys.stderr)
            return 2
        try:
            if mpath.is_file():
                cur_j = json.loads(mpath.read_text(encoding="utf-8"))
                base_j = json.loads(bpath.read_text(encoding="utf-8")) if bpath and bpath.is_file() else None
                rows, st = evaluate_compare_json(
                    mpath.stem,
                    cur_j,
                    base_j,
                    min_coverage=args.min_coverage,
                    max_extra_mm=args.max_extra_mm,
                )
                all_rows.extend(rows)
                stats_list.append(st)
            else:
                cur_files = {p.name: p for p in sorted(mpath.glob("*.json"))}
                base_files = {p.name: p for p in sorted(bpath.glob("*.json"))} if bpath and bpath.is_dir() else {}
                for name, cf in cur_files.items():
                    cur_j = json.loads(cf.read_text(encoding="utf-8"))
                    base_j = json.loads(base_files[name].read_text(encoding="utf-8")) if name in base_files else None
                    rows, st = evaluate_compare_json(
                        cf.stem,
                        cur_j,
                        base_j,
                        min_coverage=args.min_coverage,
                        max_extra_mm=args.max_extra_mm,
                    )
                    all_rows.extend(rows)
                    stats_list.append(st)
        except (OSError, KeyError, ValueError, json.JSONDecodeError) as err:
            print(f"error: invalid metrics JSON: {err}", file=sys.stderr)
            return 2
    else:
        if not args.reference_pdf or not args.rendered_pdf:
            print("error: specify <reference_pdf> <rendered_pdf>, --metrics-json, or --corpus", file=sys.stderr)
            return 2
        ref_p = Path(args.reference_pdf)
        gen_p = Path(args.rendered_pdf)
        if not ref_p.exists() or not gen_p.exists():
            print("error: reference_pdf or rendered_pdf does not exist", file=sys.stderr)
            return 2

        if args.compare_bin and ref_p.is_file() and gen_p.is_file():
            if not args.pid:
                print("error: --pid is required when using --compare-bin", file=sys.stderr)
                return 2
            with tempfile.TemporaryDirectory() as tmpd:
                cmd = [args.compare_bin, str(ref_p), str(gen_p), tmpd, args.pid]
                proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
                if proc.returncode != 0:
                    print(proc.stderr, file=sys.stderr)
                    return 2
                jsons = list(Path(tmpd).glob("*.json"))
                if not jsons:
                    print("error: pid_plot_compare produced no JSON report", file=sys.stderr)
                    return 2
                cur_j = json.loads(jsons[0].read_text(encoding="utf-8"))
                rows, st = evaluate_compare_json(
                    ref_p.stem,
                    cur_j,
                    None,
                    min_coverage=args.min_coverage,
                    max_extra_mm=args.max_extra_mm,
                )
                all_rows.extend(rows)
                stats_list.append(st)
        elif ref_p.is_file() and gen_p.is_file():
            rows, st = compare_pdfs_stdlib(ref_p.stem, ref_p, gen_p)
            all_rows.extend(rows)
            stats_list.append(st)
        elif ref_p.is_dir() and gen_p.is_dir():
            rmap = {p.name: p for p in sorted(ref_p.glob("*.pdf"))}
            gmap = {p.name: p for p in sorted(gen_p.glob("*.pdf"))}
            for name in sorted(rmap.keys() & gmap.keys()):
                rows, st = compare_pdfs_stdlib(name[:-4], rmap[name], gmap[name])
                all_rows.extend(rows)
                stats_list.append(st)
        else:
            print("error: reference_pdf and rendered_pdf must both be files or both be directories", file=sys.stderr)
            return 2

    if not stats_list:
        print("error: no PDF comparisons executed", file=sys.stderr)
        return 2

    min_cov = min(s["coverage"] for s in stats_list)
    max_ext = max(s["extra_length_mm"] for s in stats_list)
    tot_match = sum(s["text_matched"] for s in stats_list)
    tot_ref = sum(s["text_reference"] for s in stats_list)

    print(HEADER)
    for r in all_rows:
        print(r.to_tsv())
    print(
        f"SUMMARY\tsheets={len(stats_list)}\tmin_coverage={min_cov:.6f}\t"
        f"max_extra_mm={max_ext:.3f}\ttext_matched={tot_match}/{tot_ref}\tdiffs={len(all_rows)}"
    )
    return 0 if len(all_rows) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
