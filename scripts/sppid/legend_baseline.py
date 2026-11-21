#!/usr/bin/env python3
"""Batch recognition baseline evaluator against reference `_Data.xml` (L0-5 / L3-0 / #122).

Runs `dxf_legend --json` (or reads cached JSONs in `--rec-cache`) over all
usable non-empty DXF drawings from `corpus.manifest.json`, aligns each sheet
against its paired SPPID `_Data.xml` via `legend_vs_xml`, and outputs
reproducible recall/precision/tag-pairing metrics plus ranked Top N gaps.

Exit codes:
- 0: baseline report generated (or `OCS_PID_CORPUS_DIR` unset when `--dir` omitted);
- 2: CLI usage error, missing tool/files, or execution failure.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Sequence

import corpus_correspondence
import corpus_index
import legend_vs_xml

DEFAULT_CLASS_MAP: dict[str, str] = {
    # Valves, fittings, vents, specialty piping items
    "evalve": "PIDPipingComponent",
    "butterfly": "PIDPipingComponent",
    "gate": "PIDPipingComponent",
    "block-valve": "PIDPipingComponent",
    "drain-valve": "PIDPipingComponent",
    "ball-valve": "PIDPipingComponent",
    "BALL_VALVE": "PIDPipingComponent",
    "check-valve": "PIDPipingComponent",
    "relief-valve": "PIDPipingComponent",
    "breather-valve": "PIDPipingComponent",
    "flame-arrester": "PIDPipingComponent",
    "pr-valve": "PIDPipingComponent",
    "bleed-valve": "PIDPipingComponent",
    "valve-unknown": "PIDPipingComponent",
    "evalve-unknown": "PIDPipingComponent",
    "pipe-unknown": "PIDPipingComponent",
    "vent": "PIDPipingComponent",
    "funnel": "PIDPipingComponent",
    "hydrant": "PIDPipingComponent",
    "foam-port": "PIDPipingComponent",
    # Instruments & control functions
    "gauge": "PIDInstrument",
    "bubble": "PIDInstrument",
    "s-point": "PIDInstrument",
    "eh-switch": "PIDInstrument",
    "bubble-panel": "PIDControlSystemFunction",
    "flow-indicator": "PIDInlineInstrument",
    "flow-meter": "PIDInlineInstrument",
    # Equipment & off-page connectors
    "tank": "PIDProcessVessel",
    "pump": "PIDMechanicalEquipment",
    "loading-arm": "PIDProcessEquipment",
    "skid": "PIDProcessEquipment",
    "equip-unknown": "PIDProcessEquipment",
    "sheet-ref": "PIDBranchPoint",
    "flow-arrow": "PIDBranchPoint",
}

KNOWN_SPPID_CLASSES = set(legend_vs_xml.DEFAULT_SYMBOL_CLASSES)


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _populate_rec_cache(
    dxf_bin: Path,
    pending: list[tuple[int, Path]],
    rec_cache: Path,
    batch_size: int = 10,
) -> None:
    rec_cache.mkdir(parents=True, exist_ok=True)
    for start in range(0, len(pending), batch_size):
        chunk = pending[start : start + batch_size]
        cmd = [str(dxf_bin), "--json"] + [str(p) for (_idx, p) in chunk]
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"dxf_legend failed (rc={proc.returncode}): {proc.stderr[:400]}")
        docs = json.loads(proc.stdout)
        if not isinstance(docs, list) or len(docs) != len(chunk):
            # Fallback to running one-by-one if a DXF failed to load and was skipped
            for idx, dxf_path in chunk:
                single = subprocess.run(
                    [str(dxf_bin), "--json", str(dxf_path)],
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    check=False,
                )
                arr = json.loads(single.stdout) if single.returncode == 0 and single.stdout.strip() else []
                payload = arr[0] if arr else {"file": dxf_path.name, "load_failed": True, "symbols": []}
                (rec_cache / f"{idx:04d}.json").write_text(
                    json.dumps(payload, ensure_ascii=False, indent=2),
                    encoding="utf-8",
                )
        else:
            for (idx, _dxf_path), payload in zip(chunk, docs):
                (rec_cache / f"{idx:04d}.json").write_text(
                    json.dumps(payload, ensure_ascii=False, indent=2),
                    encoding="utf-8",
                )


def render_baseline_md(summary: dict[str, Any]) -> str:
    d = summary["denominator"]
    ref = summary["reference"]
    rec = summary["recognized"]
    cm = summary["class_mapped_metrics"]
    tl = summary["tag_level"]
    gaps = summary["top_gaps"]

    lines = [
        "# 存量图例识别对 SPPID 真值现状基线报告（`L0-5` / `L3-0` / `#122`）",
        "",
        f"- 语料清单摘要（`manifest_sha256`）：`{summary['manifest_sha256']}`",
        f"- 样本总套数：`{d['total_triples']}`（剔除 0 字节空 DXF：`{d['skipped_empty_dxf']}`，缺 DXF：`{d['skipped_missing_dxf']}`，DXF 读取失败：`{d['load_failed_dxf']}`，**实际评估有效图纸分母：`{d['evaluated_drawings']}`**）",
        "",
        "## 1. 核心基线指标汇总",
        "",
        "| 维度 | 指标 | 数值 | 说明 |",
        "|---|---|---:|---|",
        f"| SPPID 真值（`_Data.xml`） | 图上可标记符号总数 | `{ref['total_symbols']}` | 覆盖 10 类图面符号对象（设备/管件/仪表/管口/页间连接符） |",
        f"| SPPID 真值（`_Data.xml`） | 带非空位号符号数 | `{ref['tagged_symbols']}` | `IObject/@Name` 或分段属性非空 |",
        f"| 现有识别器（`io::pid_legend`） | 检出候选符号总数 | `{rec['total_symbols']}` | 其中已知规则符号 `{rec['known_symbols']}`，配上位号 `{rec['tagged_symbols']}` |",
        f"| 现有识别器（`io::pid_legend`） | 未配对孤立位号（`orphan_tags`） | `{rec['orphan_tags']}` | 识别出位号文本但半径内无匹配符号 |",
        f"| 类映射对齐度量（`legend_vs_xml`） | 符号召回率（`recall`） | `{cm['recall']:.6f}` | `{cm['matched']} / {ref['total_symbols']}` |",
        f"| 类映射对齐度量（`legend_vs_xml`） | 符号精确率（`precision`） | `{cm['precision']:.6f}` | `{cm['matched']} / {rec['total_symbols']}` |",
        f"| 类映射对齐度量（`legend_vs_xml`） | 匹配项位号一致率（`tag_accuracy`） | `{cm['tag_accuracy']:.6f}` | `{cm['tag_exact']} / {cm['matched']}` |",
        f"| 纯位号文本层命中拆解 | 真值位号在识别符号 `tag` 中命中 | `{tl['matched_on_symbol']}` | 占带位号真值符号的 `{tl['symbol_tag_recall']:.6f}` |",
        f"| 纯位号文本层命中拆解 | 真值位号落在 `orphan_tags` 中 | `{tl['seen_in_orphan_tags']}` | 文字已认出，仅缺符号图形规则或配对半径 |",
        "",
        "## 2. Top 缺口清单（按影响对象数降序）",
        "",
        "### 2.1 未召回的 SPPID 真值对象类分布",
        "",
        "| SPPID 对象类 | 未召回实例数 |",
        "|---|---:|",
    ]
    for cls, cnt in gaps["missed_sppid_classes"]:
        lines.append(f"| `{cls}` | {cnt} |")

    lines += [
        "",
        "### 2.2 Top 未登记 DXF 块名（`unknown_blocks`）",
        "",
        "| DXF 块名 | 出现总次数 |",
        "|---|---:|",
    ]
    for bname, cnt in gaps["unknown_blocks"]:
        lines.append(f"| `{bname}` | {cnt} |")

    lines += [
        "",
        "### 2.3 Top 未命名炸开形状指纹（`unknown_shapes`）",
        "",
        "| 形状指纹 ID | 出现总次数 |",
        "|---|---:|",
    ]
    for sid, cnt in gaps["unknown_shapes"]:
        lines.append(f"| `{sid}` | {cnt} |")

    lines += [
        "",
        "### 2.4 未映射到 SPPID 类的识别类别（`unmapped_rec_classes`）",
        "",
        "| 识别类别 | 实例数 |",
        "|---|---:|",
    ]
    for ucls, cnt in gaps["unmapped_rec_classes"]:
        lines.append(f"| `{ucls}` | {cnt} |")

    lines.append("")
    return "\n".join(lines)


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="legend_baseline.py",
        description="Run P&ID legend recognition across corpus DXFs and compute baseline metrics vs _Data.xml.",
    )
    p.add_argument("--dir", default=None, help=f"Corpus root (defaults to ${corpus_index.CORPUS_ENV})")
    p.add_argument("--manifest", default=None, help="Path to corpus.manifest.json")
    p.add_argument("--dxf-legend-bin", default=None, help="Path to compiled dxf_legend executable")
    p.add_argument("--rec-cache", default=None, help="Directory holding cached per-drawing recognition JSONs")
    p.add_argument("--class-map", default=None, help="Optional JSON overriding recognition -> SPPID class map")
    p.add_argument("--out-dir", default=None, help="Output directory for baseline-summary.json and baseline-report.md")
    p.add_argument("--top-n", type=int, default=15, help="Number of top gaps to list")
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
        print(f"未设置语料目录（{corpus_index.CORPUS_ENV}），跳过识别基线评估。")
        return 0

    corpus_dir = Path(raw_dir)
    manifest_path = Path(args.manifest) if args.manifest else (corpus_dir / corpus_index.MANIFEST_FILENAME)
    if not manifest_path.is_file():
        print(f"error: manifest not found: {manifest_path}", file=sys.stderr)
        return 2

    class_map = dict(DEFAULT_CLASS_MAP)
    if args.class_map:
        class_map.update(json.loads(Path(args.class_map).read_text(encoding="utf-8")))

    manifest = corpus_index.load_manifest(manifest_path)
    corr = manifest.get("correspondence")
    if not isinstance(corr, dict):
        corr, _ = corpus_correspondence.build_correspondence(manifest)

    export_sub = manifest["sections"]["EXPORT_SET"]["subdir"]
    dxf_sub = manifest["sections"]["EXPORT_DXF"]["subdir"]
    export_base = corpus_dir / Path(*export_sub.split("/"))
    dxf_base = corpus_dir / Path(*dxf_sub.split("/"))

    drawings = corr["drawings"]
    total_triples = sum(1 for r in drawings if r["triple_complete"])
    skipped_empty = 0
    skipped_missing = 0
    eval_items: list[tuple[int, Path, Path]] = []

    for r in drawings:
        if not r["triple_complete"]:
            continue
        if r["dxf_path"] is None:
            skipped_missing += 1
            continue
        if int(r["dxf_bytes"]) == 0:
            skipped_empty += 1
            continue
        idx = len(eval_items)
        dxf_p = dxf_base / Path(*r["dxf_path"].split("/"))
        xml_p = export_base / Path(*r["data_xml_path"].split("/"))
        eval_items.append((idx, dxf_p, xml_p))

    out_dir = Path(args.out_dir) if args.out_dir else Path(".")
    rec_cache = Path(args.rec_cache) if args.rec_cache else (out_dir / "rec_cache")
    rec_cache.mkdir(parents=True, exist_ok=True)

    missing_cache = [
        (idx, dxf_p) for (idx, dxf_p, _xml_p) in eval_items if not (rec_cache / f"{idx:04d}.json").is_file()
    ]
    if missing_cache:
        if not args.dxf_legend_bin:
            print("error: cached recognition JSONs missing and --dxf-legend-bin not provided", file=sys.stderr)
            return 2
        try:
            _populate_rec_cache(Path(args.dxf_legend_bin), missing_cache, rec_cache)
        except Exception as err:
            print(f"error: {err}", file=sys.stderr)
            return 2

    load_failed_dxf = 0
    evaluated_drawings = 0
    ref_total = 0
    ref_tagged = 0
    rec_total = 0
    rec_known = 0
    rec_tagged = 0
    rec_orphans = 0
    cm_matched = 0
    cm_tag_exact = 0
    tag_matched_on_sym = 0
    tag_seen_in_orphans = 0

    missed_sppid_classes: Counter[str] = Counter()
    unknown_blocks: Counter[str] = Counter()
    unknown_shapes: Counter[str] = Counter()
    unmapped_rec_classes: Counter[str] = Counter()

    for idx, _dxf_p, xml_p in eval_items:
        rec_json = json.loads((rec_cache / f"{idx:04d}.json").read_text(encoding="utf-8"))
        if rec_json.get("load_failed"):
            load_failed_dxf += 1
            continue
        evaluated_drawings += 1

        ref_syms = legend_vs_xml.extract_reference_symbols(xml_p)
        rec_syms = legend_vs_xml.load_recognition_symbols(rec_json, class_map)
        diff_rows, m = legend_vs_xml.evaluate_recognition(ref_syms, rec_syms)

        ref_total += len(ref_syms)
        ref_nonempty_tags = [s.tag for s in ref_syms if s.tag]
        ref_tagged += len(ref_nonempty_tags)

        raw_syms = rec_json.get("symbols", [])
        rec_total += len(raw_syms)
        rec_known += sum(1 for s in raw_syms if s.get("known"))
        sym_tags = {str(s.get("tag") or "").strip() for s in raw_syms if str(s.get("tag") or "").strip()}
        rec_tagged += sum(1 for s in raw_syms if str(s.get("tag") or "").strip())
        orphan_list = [str(t).strip() for t in rec_json.get("orphan_tags", []) if str(t).strip()]
        orphan_set = set(orphan_list)
        rec_orphans += len(orphan_list)

        cm_matched += m["matched"]
        cm_tag_exact += m["tag_exact"]

        for t in ref_nonempty_tags:
            if t in sym_tags:
                tag_matched_on_sym += 1
            elif t in orphan_set:
                tag_seen_in_orphans += 1

        for row in diff_rows:
            if row.kind == "MISSING_SYMBOL":
                missed_sppid_classes[row.location] += 1

        for s in raw_syms:
            raw_cls = str(s.get("class", "")).strip()
            mapped = class_map.get(raw_cls, raw_cls)
            if mapped not in KNOWN_SPPID_CLASSES:
                unmapped_rec_classes[raw_cls] += 1

        for bname, cnt in (rec_json.get("unknown_blocks") or {}).items():
            unknown_blocks[str(bname)] += int(cnt)
        for sh in rec_json.get("unknown_shapes") or []:
            if isinstance(sh, dict) and "id" in sh:
                unknown_shapes[str(sh["id"])] += int(sh.get("count", 0))

    recall = (cm_matched / ref_total) if ref_total > 0 else 1.0
    precision = (cm_matched / rec_total) if rec_total > 0 else 1.0
    tag_acc = (cm_tag_exact / cm_matched) if cm_matched > 0 else 1.0
    sym_tag_recall = (tag_matched_on_sym / ref_tagged) if ref_tagged > 0 else 1.0

    top_n = args.top_n
    summary = {
        "manifest_sha256": manifest.get("manifest_sha256", ""),
        "denominator": {
            "total_triples": total_triples,
            "skipped_empty_dxf": skipped_empty,
            "skipped_missing_dxf": skipped_missing,
            "load_failed_dxf": load_failed_dxf,
            "evaluated_drawings": evaluated_drawings,
        },
        "reference": {
            "total_symbols": ref_total,
            "tagged_symbols": ref_tagged,
        },
        "recognized": {
            "total_symbols": rec_total,
            "known_symbols": rec_known,
            "tagged_symbols": rec_tagged,
            "orphan_tags": rec_orphans,
        },
        "class_mapped_metrics": {
            "matched": cm_matched,
            "tag_exact": cm_tag_exact,
            "recall": round(recall, 6),
            "precision": round(precision, 6),
            "tag_accuracy": round(tag_acc, 6),
        },
        "tag_level": {
            "matched_on_symbol": tag_matched_on_sym,
            "seen_in_orphan_tags": tag_seen_in_orphans,
            "symbol_tag_recall": round(sym_tag_recall, 6),
        },
        "top_gaps": {
            "missed_sppid_classes": [[k, v] for k, v in missed_sppid_classes.most_common(top_n)],
            "unknown_blocks": [[k, v] for k, v in unknown_blocks.most_common(top_n)],
            "unknown_shapes": [[k, v] for k, v in unknown_shapes.most_common(top_n)],
            "unmapped_rec_classes": [[k, v] for k, v in unmapped_rec_classes.most_common(top_n)],
        },
    }

    if args.out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "baseline-summary.json").write_text(
            json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        (out_dir / "baseline-report.md").write_text(
            render_baseline_md(summary),
            encoding="utf-8",
        )

    print(
        f"baseline: evaluated={evaluated_drawings}/{total_triples} (empty_dxf={skipped_empty}) "
        f"ref={ref_total} rec={rec_total} matched={cm_matched} "
        f"recall={recall:.6f} precision={precision:.6f} tag_acc={tag_acc:.6f} "
        f"sym_tag_hits={tag_matched_on_sym}/{ref_tagged} orphan_tag_hits={tag_seen_in_orphans}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
