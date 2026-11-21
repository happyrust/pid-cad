#!/usr/bin/env python3
"""Build correspondence table across EXPORT_SET, EXPORT_DXF, EXPORT_PDF, and SOURCE_DWG (L0-3 / #120).

Reads `corpus.manifest.json`, pairs `<stem>.pid` + `<stem>_Data.xml` +
`<stem>_Meta.xml` with `EXPORT_DXF`, `EXPORT_PDF`, and optional `SOURCE_DWG`,
flags 0-byte files, fuzzy rename candidates, and unmatched extras, writes the
`correspondence` block back into `corpus.manifest.json` (zero absolute paths),
and optionally writes `correspondence.csv` + `correspondence-summary.md` into
`--out-dir`.

Exit codes:
- 0: correspondence table built (or `OCS_PID_CORPUS_DIR` unset when `--dir` omitted);
- 1: `--strict` passed and incomplete XML triples or unmatched items were found;
- 2: CLI usage error or unreadable manifest.
"""

from __future__ import annotations

import argparse
import csv
from difflib import SequenceMatcher
import io
import json
import os
from pathlib import Path, PurePosixPath
import sys
from typing import Any, Sequence

import corpus_index

FUZZY_THRESHOLD = 0.85


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _stem_of(posix_path: str) -> str:
    return PurePosixPath(posix_path).stem


def _match_pool(
    stems: list[str],
    pool: dict[str, dict[str, Any]],
) -> tuple[dict[str, tuple[str, dict[str, Any], float]], list[str]]:
    """Match `stems` against `pool` (stem -> file entry) via exact then fuzzy pass."""
    matched: dict[str, tuple[str, dict[str, Any], float]] = {}
    used_pool: set[str] = set()

    # Pass 1: exact stem match
    for st in stems:
        if st in pool:
            matched[st] = ("exact", pool[st], 1.0)
            used_pool.add(st)

    # Pass 2: greedy highest-similarity fuzzy match >= FUZZY_THRESHOLD
    remaining_stems = [s for s in stems if s not in matched]
    remaining_pool = [p for p in pool if p not in used_pool]
    candidates: list[tuple[float, str, str]] = []
    for st in remaining_stems:
        for pk in remaining_pool:
            ratio = SequenceMatcher(None, st, pk).ratio()
            if ratio >= FUZZY_THRESHOLD:
                candidates.append((ratio, st, pk))
    candidates.sort(key=lambda x: (-x[0], x[1], x[2]))

    assigned_stems: set[str] = set()
    for ratio, st, pk in candidates:
        if st in assigned_stems or pk in used_pool:
            continue
        assigned_stems.add(st)
        used_pool.add(pk)
        matched[st] = ("candidate_fuzzy", pool[pk], round(ratio, 4))

    unmatched_pool = sorted(pool[p]["path"] for p in pool if p not in used_pool)
    return matched, unmatched_pool


def build_correspondence(manifest: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    sections = manifest["sections"]
    export_files = sections.get("EXPORT_SET", {}).get("files", [])
    dxf_files = sections.get("EXPORT_DXF", {}).get("files", [])
    pdf_files = sections.get("EXPORT_PDF", {}).get("files", [])
    src_files = sections.get("SOURCE_DWG", {}).get("files", [])

    # Group EXPORT_SET by (parent_dir, drawing_stem)
    groups: dict[tuple[str, str], dict[str, dict[str, Any]]] = {}
    for entry in export_files:
        rel = entry["path"]
        pp = PurePosixPath(rel)
        parent = "" if str(pp.parent) == "." else pp.parent.as_posix()
        name = pp.name
        lower = name.lower()
        if lower.endswith("_data.xml"):
            stem = name[:-9]
            kind = "data_xml"
        elif lower.endswith("_meta.xml"):
            stem = name[:-9]
            kind = "meta_xml"
        elif lower.endswith(".pid"):
            stem = name[:-4]
            kind = "pid"
        else:
            continue
        groups.setdefault((parent, stem), {})[kind] = entry

    dxf_pool = {_stem_of(e["path"]): e for e in dxf_files if e["path"].lower().endswith(".dxf")}
    pdf_pool = {_stem_of(e["path"]): e for e in pdf_files if e["path"].lower().endswith(".pdf")}
    src_pool = {
        _stem_of(e["path"]): e
        for e in src_files
        if e["path"].lower().endswith((".dwg", ".dxf"))
    }

    sorted_keys = sorted(groups.keys())
    unique_stems = [stem for (_parent, stem) in sorted_keys]

    dxf_matches, unmatched_dxf = _match_pool(unique_stems, dxf_pool)
    pdf_matches, unmatched_pdf = _match_pool(unique_stems, pdf_pool)
    src_matches, unmatched_src = _match_pool(unique_stems, src_pool)

    rows: list[dict[str, Any]] = []
    problems: list[str] = []

    total_pid = 0
    complete_triples = 0
    dxf_exact = 0
    dxf_fuzzy = 0
    dxf_missing = 0
    dxf_empty = 0
    usable_dxf = 0
    pdf_exact = 0
    pdf_fuzzy = 0
    pdf_missing = 0

    for parent, stem in sorted_keys:
        g = groups[(parent, stem)]
        pid_e = g.get("pid")
        data_e = g.get("data_xml")
        meta_e = g.get("meta_xml")

        if pid_e is not None:
            total_pid += 1
        triple_ok = pid_e is not None and data_e is not None and meta_e is not None
        if triple_ok:
            complete_triples += 1
        else:
            missing_parts = [
                k for k, v in (("pid", pid_e), ("_Data.xml", data_e), ("_Meta.xml", meta_e)) if v is None
            ]
            problems.append(f"INCOMPLETE_TRIPLE: {parent}/{stem} missing {','.join(missing_parts)}")

        dxf_info = dxf_matches.get(stem)
        if dxf_info is None:
            dxf_status = "unmatched"
            dxf_path = None
            dxf_bytes = 0
            dxf_sim = 0.0
            dxf_missing += 1
        else:
            m_kind, dxf_entry, dxf_sim = dxf_info
            dxf_path = dxf_entry["path"]
            dxf_bytes = int(dxf_entry["bytes"])
            if m_kind == "exact":
                dxf_exact += 1
            else:
                dxf_fuzzy += 1
            if dxf_bytes == 0:
                dxf_status = f"{m_kind}_empty_0_bytes"
                dxf_empty += 1
            else:
                dxf_status = m_kind
                if triple_ok:
                    usable_dxf += 1

        pdf_info = pdf_matches.get(stem)
        if pdf_info is None:
            pdf_status = "unmatched"
            pdf_path = None
            pdf_sim = 0.0
            pdf_missing += 1
        else:
            p_kind, pdf_entry, pdf_sim = pdf_info
            pdf_path = pdf_entry["path"]
            if p_kind == "exact":
                pdf_exact += 1
            else:
                pdf_fuzzy += 1
            pdf_status = p_kind if int(pdf_entry["bytes"]) > 0 else f"{p_kind}_empty_0_bytes"

        src_info = src_matches.get(stem)
        if src_info is None:
            src_status = "not_provided" if not src_pool else "needs_manual_confirm"
            src_path = None
        else:
            s_kind, src_entry, _ = src_info
            src_status = s_kind if s_kind == "exact" else "candidate_needs_confirm"
            src_path = src_entry["path"]

        rows.append(
            {
                "batch": parent,
                "drawing_stem": stem,
                "pid_path": pid_e["path"] if pid_e else None,
                "data_xml_path": data_e["path"] if data_e else None,
                "meta_xml_path": meta_e["path"] if meta_e else None,
                "triple_complete": triple_ok,
                "dxf_path": dxf_path,
                "dxf_bytes": dxf_bytes,
                "dxf_status": dxf_status,
                "dxf_similarity": dxf_sim,
                "pdf_path": pdf_path,
                "pdf_status": pdf_status,
                "pdf_similarity": pdf_sim,
                "source_dwg_path": src_path,
                "source_dwg_status": src_status,
            }
        )

    for ud in unmatched_dxf:
        problems.append(f"UNMATCHED_DXF: {ud}")
    for up in unmatched_pdf:
        problems.append(f"UNMATCHED_PDF: {up}")

    summary = {
        "total_groups": len(rows),
        "total_pid": total_pid,
        "complete_xml_triples": complete_triples,
        "incomplete_triples": len(rows) - complete_triples,
        "dxf_total_files": len(dxf_pool),
        "dxf_exact": dxf_exact,
        "dxf_fuzzy": dxf_fuzzy,
        "dxf_unmatched_drawings": dxf_missing,
        "dxf_empty_zero_bytes": dxf_empty,
        "usable_dxf_for_recognition": usable_dxf,
        "unmatched_extra_dxf_files": unmatched_dxf,
        "pdf_total_files": len(pdf_pool),
        "pdf_exact": pdf_exact,
        "pdf_fuzzy": pdf_fuzzy,
        "pdf_unmatched_drawings": pdf_missing,
        "unmatched_extra_pdf_files": unmatched_pdf,
        "source_dwg_total_files": len(src_pool),
        "unmatched_source_dwg_files": unmatched_src,
    }
    return {"summary": summary, "drawings": rows}, problems


def render_csv(corr: dict[str, Any]) -> str:
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="\n")
    writer.writerow(
        [
            "batch",
            "drawing_stem",
            "triple_complete",
            "pid_path",
            "data_xml_path",
            "meta_xml_path",
            "dxf_status",
            "dxf_bytes",
            "dxf_similarity",
            "dxf_path",
            "pdf_status",
            "pdf_similarity",
            "pdf_path",
            "source_dwg_status",
            "source_dwg_path",
        ]
    )
    for r in corr["drawings"]:
        writer.writerow(
            [
                r["batch"],
                r["drawing_stem"],
                r["triple_complete"],
                r["pid_path"] or "",
                r["data_xml_path"] or "",
                r["meta_xml_path"] or "",
                r["dxf_status"],
                r["dxf_bytes"],
                r["dxf_similarity"],
                r["dxf_path"] or "",
                r["pdf_status"],
                r["pdf_similarity"],
                r["pdf_path"] or "",
                r["source_dwg_status"],
                r["source_dwg_path"] or "",
            ]
        )
    return buf.getvalue()


def render_summary_md(corr: dict[str, Any], manifest_sha: str) -> str:
    s = corr["summary"]
    lines = [
        "# 语料对应关系核对报告（`L0-3` / `#120`）",
        "",
        f"- 清单摘要（`manifest_sha256`）：`{manifest_sha}`",
        f"- `.pid` 图纸总数：`{s['total_pid']}`（三件套 `.pid` + `_Data.xml` + `_Meta.xml` 齐全：`{s['complete_xml_triples']}`，缺失：`{s['incomplete_triples']}`）",
        f"- `EXPORT_DXF` 文件数：`{s['dxf_total_files']}`（精确同名：`{s['dxf_exact']}`，近似改名候选：`{s['dxf_fuzzy']}`，未配上图纸：`{s['dxf_unmatched_drawings']}`，其中 **0 字节空 DXF：`{s['dxf_empty_zero_bytes']}`**，可用于识别度量的有效非空 DXF 分母：**`{s['usable_dxf_for_recognition']}`**）",
        f"- `EXPORT_PDF` 文件数：`{s['pdf_total_files']}`（精确同名：`{s['pdf_exact']}`，近似改名候选：`{s['pdf_fuzzy']}`，未配上图纸：`{s['pdf_unmatched_drawings']}`）",
        f"- `SOURCE_DWG` 原始图文件数：`{s['source_dwg_total_files']}`",
        "",
    ]
    fuzzy_rows = [r for r in corr["drawings"] if "fuzzy" in r["dxf_status"] or "fuzzy" in r["pdf_status"]]
    if fuzzy_rows:
        lines += [
            "## 近似改名候选（需人工复核确认项）",
            "",
            "| 批次 | `.pid` / XML 图名 | DXF 状态与相似度 | PDF 状态与相似度 |",
            "|---|---|---|---|",
        ]
        for r in fuzzy_rows:
            lines.append(
                f"| `{r['batch']}` | `{r['drawing_stem']}` | `{r['dxf_status']}` ({r['dxf_similarity']}) | `{r['pdf_status']}` ({r['pdf_similarity']}) |"
            )
        lines.append("")

    empty_rows = [r for r in corr["drawings"] if "empty_0_bytes" in r["dxf_status"]]
    if empty_rows:
        lines += [
            f"## 0 字节空导出 DXF（共 {len(empty_rows)} 份，不计入识别分母）",
            "",
        ]
        for r in empty_rows:
            lines.append(f"- `{r['dxf_path']}`")
        lines.append("")

    return "\n".join(lines) + "\n"


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="corpus_correspondence.py",
        description="Build correspondence table across EXPORT_SET, EXPORT_DXF, EXPORT_PDF, and SOURCE_DWG.",
    )
    p.add_argument("--dir", default=None, help=f"Corpus root (defaults to ${corpus_index.CORPUS_ENV})")
    p.add_argument("--manifest", default=None, help="Path to corpus.manifest.json")
    p.add_argument("--out-dir", default=None, help="Directory to write correspondence.csv and correspondence-summary.md")
    p.add_argument("--strict", action="store_true", help="Exit 1 when any XML triple is incomplete")
    return p


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    raw_dir = args.dir.strip() if args.dir is not None else os.environ.get(corpus_index.CORPUS_ENV, "").strip()
    if not raw_dir:
        print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过对应关系表生成。")
        return 0

    corpus_dir = Path(raw_dir)
    manifest_path = Path(args.manifest) if args.manifest else (corpus_dir / corpus_index.MANIFEST_FILENAME)
    if not manifest_path.is_file():
        print(f"error: manifest not found: {manifest_path}", file=sys.stderr)
        return 2

    try:
        manifest = corpus_index.load_manifest(manifest_path)
        corr, problems = build_correspondence(manifest)
        manifest["correspondence"] = corr
        manifest_path.write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        if args.out_dir:
            odir = Path(args.out_dir)
            odir.mkdir(parents=True, exist_ok=True)
            (odir / "correspondence.csv").write_text(render_csv(corr), encoding="utf-8")
            (odir / "correspondence-summary.md").write_text(
                render_summary_md(corr, manifest.get("manifest_sha256", "")),
                encoding="utf-8",
            )
    except (OSError, ValueError, json.JSONDecodeError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    s = corr["summary"]
    print(
        f"correspondence: pid={s['total_pid']} triples={s['complete_xml_triples']} "
        f"dxf_exact={s['dxf_exact']} dxf_fuzzy={s['dxf_fuzzy']} "
        f"dxf_empty={s['dxf_empty_zero_bytes']} usable_dxf={s['usable_dxf_for_recognition']} "
        f"pdf_exact={s['pdf_exact']} pdf_fuzzy={s['pdf_fuzzy']}"
    )
    for line in problems:
        print(line, file=sys.stderr)

    if args.strict and s["incomplete_triples"] > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
