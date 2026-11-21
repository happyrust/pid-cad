#!/usr/bin/env python3
"""Batch .pid open-performance benchmark, L1-4 text-run audit, and L1-2 rendered-PDF
comparison evaluator across the 94-drawing SmartPlant corpus (#131).

Pure Python standard-library module and CLI. Produces strictly sanitized JSON and
Markdown summaries (keyed only by `S01..S94` and 12-hex `pid_sha256_prefix`, never
emitting raw drawing stems or machine-specific paths).
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import statistics
import sys
from typing import Any, Sequence


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _percentile(sorted_vals: Sequence[float], q: float) -> float:
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return float(sorted_vals[0])
    pos = (len(sorted_vals) - 1) * q
    lo = int(math.floor(pos))
    hi = int(math.ceil(pos))
    if lo == hi:
        return float(sorted_vals[lo])
    frac = pos - lo
    return float(sorted_vals[lo] * (1.0 - frac) + sorted_vals[hi] * frac)


def _safe_uncovered_tag(label: str) -> str:
    s = label.strip()
    if s in ("图框", "图纸线"):
        return "border"
    if s.startswith("/"):
        return s.split()[0]
    if s.lower().endswith(".sym") and s.isascii():
        return s
    return "stroke"


def _classify_outlier(
    *,
    pair_status: str,
    coverage: float,
    extra_length_mm: float,
    within_threshold: bool | None,
    min_coverage: float,
    max_extra_mm: float,
    flattened_runs: int,
    comp: dict[str, Any],
) -> tuple[str, str]:
    is_fuzzy = pair_status in ("fuzzy", "candidate_fuzzy")
    if is_fuzzy:
        if within_threshold is False:
            return (
                "corpus_pairing",
                "fuzzy candidate pairing / registration mismatch between .pid and candidate reference PDF",
            )
        return (
            "corpus_pairing",
            "fuzzy candidate pairing / revision delta between .pid and candidate reference PDF",
        )
    if within_threshold is False:
        return (
            "canvas_registration",
            "page registration / border-template transform outside fit threshold",
        )

    uncovered_ref = comp.get("uncovered_reference")
    page_mm = comp.get("page_mm")
    if isinstance(uncovered_ref, list) and uncovered_ref:
        border_mm = sum(
            float(u.get("length_mm") or 0.0)
            for u in uncovered_ref
            if isinstance(u, dict) and str(u.get("label") or "").strip() in ("图框", "图纸线")
        )
        if border_mm > 0.0:
            page_desc = (
                f"{float(page_mm[0]):.0f}x{float(page_mm[1]):.0f}mm"
                if isinstance(page_mm, list) and len(page_mm) == 2
                else "non-standard"
            )
            return (
                "border_template",
                f"elongated/non-standard sheet ({page_desc}) lacks archived .igr border template "
                f"({border_mm:.1f}mm unmasked border strokes; flattened_runs={flattened_runs})",
            )
        parts = []
        for u in uncovered_ref[:3]:
            if isinstance(u, dict):
                tag = _safe_uncovered_tag(str(u.get("label") or ""))
                mm = float(u.get("length_mm") or 0.0)
                parts.append(f"{tag}:{mm:.1f}mm")
        if parts:
            return (
                "symbol_or_furniture",
                f"unmasked title-block sub-table or subset-missing .sym stroke difference "
                f"({', '.join(parts)}; flattened_runs={flattened_runs})",
            )

    if coverage < min_coverage and extra_length_mm > max_extra_mm:
        return (
            "plot_geometry",
            f"plotter stroke/symbol delta (coverage<{min_coverage:.3f} and extra>{max_extra_mm:.1f}mm; flattened_runs={flattened_runs})",
        )
    if coverage < min_coverage:
        return (
            "plot_coverage",
            f"plotter missing stroke/symbol coverage (coverage<{min_coverage:.3f}; flattened_runs={flattened_runs})",
        )
    return (
        "plot_extra_strokes",
        f"plotter extra stroke length above {max_extra_mm:.1f}mm (border furniture, hidden layer, or symbol cache strokes)",
    )


def evaluate_l1_batch(
    bench_report: dict[str, Any],
    compare_records: dict[str, dict[str, Any]],
    *,
    pair_status_by_stem: dict[str, str] | None = None,
    baseline_per_sheet_ms: float = 732.1,
    max_open_multiple: float = 1.5,
    min_coverage: float = 0.980,
    max_extra_mm: float = 200.0,
    manifest_sha256: str = "",
) -> dict[str, Any]:
    raw_sheets = list(bench_report.get("sheets") or [])
    pair_map = pair_status_by_stem or {}
    warmup_ms = float(bench_report.get("warmup_ms") or 0.0)

    sheet_count = len(raw_sheets)
    load_failures = 0
    plot_failures = 0

    cad_import_vals: list[float] = []
    scene_prepare_vals: list[float] = []
    open_vals: list[float] = []
    plot_vals: list[float] = []

    total_cad_entities = 0
    total_primitives = 0
    total_on_page = 0
    total_pdf_bytes = 0

    slowest_open_ms = -1.0
    slowest_sheet_id = ""
    slowest_sha_prefix = ""

    tr_total = 0
    tr_uniform = 0
    tr_no_runs = 0
    tr_flattened = 0
    tr_unresolvable = 0
    tr_length_mismatch = 0
    sheets_with_flattened = 0
    sheets_with_unresolvable = 0
    flattened_sheets: list[dict[str, Any]] = []

    compared_sheets = 0
    uncompared_sheets = 0
    coverages: list[float] = []
    extras: list[float] = []
    total_text_matched = 0
    total_text_reference = 0
    within_fit_count = 0

    exact_1_000 = 0
    ge_0_980 = 0
    below_0_980 = 0
    ge_0_995 = 0

    cohort_buckets: dict[str, list[tuple[float, float, bool]]] = {
        "exact": [],
        "fuzzy": [],
    }
    outliers: list[dict[str, Any]] = []
    sanitized_sheets: list[dict[str, Any]] = []

    for idx, entry in enumerate(raw_sheets, start=1):
        sheet_id = str(entry.get("sheet_id") or f"S{idx:02d}")
        stem = str(entry.get("stem") or "")
        sha_prefix = str(entry.get("pid_sha256_prefix") or "")
        raw_pair = pair_map.get(stem, str(entry.get("pair_status") or "exact"))
        norm_pair = "fuzzy" if "fuzzy" in raw_pair else "exact"

        load_ok = bool(entry.get("load_ok", False))
        plot_ok = bool(entry.get("plot_ok", False))
        if not load_ok:
            load_failures += 1
        if not plot_ok:
            plot_failures += 1

        cad_ms = float(entry.get("cad_import_ms") or 0.0)
        prep_ms = float(entry.get("scene_prepare_ms") or 0.0)
        open_ms = float(entry.get("total_open_ms") or (cad_ms + prep_ms))
        plot_ms = float(entry.get("plot_ms") or 0.0)

        if load_ok:
            cad_import_vals.append(cad_ms)
            scene_prepare_vals.append(prep_ms)
            open_vals.append(open_ms)
            if open_ms > slowest_open_ms:
                slowest_open_ms = open_ms
                slowest_sheet_id = sheet_id
                slowest_sha_prefix = sha_prefix

        if plot_ok:
            plot_vals.append(plot_ms)

        ents = int(entry.get("cad_entities") or 0)
        prims = int(entry.get("primitives") or 0)
        on_page = int(entry.get("on_page") or 0)
        pdf_bytes = int(entry.get("pdf_bytes") or 0)
        total_cad_entities += ents
        total_primitives += prims
        total_on_page += on_page
        total_pdf_bytes += pdf_bytes

        tr = entry.get("text_runs") or {}
        s_total = int(tr.get("total_texts") or 0)
        s_uniform = int(tr.get("uniform") or 0)
        s_no_runs = int(tr.get("no_runs") or 0)
        s_flat = int(tr.get("flattened") or 0)
        s_unres = int(tr.get("unresolvable") or 0)
        s_len_mis = int(tr.get("length_mismatch") or 0)

        tr_total += s_total
        tr_uniform += s_uniform
        tr_no_runs += s_no_runs
        tr_flattened += s_flat
        tr_unresolvable += s_unres
        tr_length_mismatch += s_len_mis
        if s_flat > 0:
            sheets_with_flattened += 1
        if s_unres > 0 or s_len_mis > 0:
            sheets_with_unresolvable += 1

        comp = compare_records.get(stem)
        cov_val: float | None = None
        extra_val: float | None = None
        t_match: int | None = None
        t_ref: int | None = None
        fit_ok: bool | None = None

        if isinstance(comp, dict) and "coverage" in comp and comp.get("coverage") is not None:
            compared_sheets += 1
            cov_val = float(comp["coverage"])
            extra_val = float(comp.get("extra_length_mm") or 0.0)
            t_match = int(comp.get("text_matched") or 0)
            t_ref = int(comp.get("text_reference") or 0)
            fit_obj = comp.get("fit")
            if isinstance(fit_obj, dict) and isinstance(fit_obj.get("within_threshold"), bool):
                fit_ok = fit_obj["within_threshold"]

            coverages.append(cov_val)
            extras.append(extra_val)
            total_text_matched += t_match
            total_text_reference += t_ref
            if fit_ok is True:
                within_fit_count += 1

            if cov_val >= 0.9995:
                exact_1_000 += 1
            elif cov_val >= min_coverage:
                ge_0_980 += 1
            else:
                below_0_980 += 1

            if cov_val >= 0.995:
                ge_0_995 += 1

            is_outlier = (
                cov_val < min_coverage
                or extra_val > max_extra_mm
                or fit_ok is False
            )
            cohort_buckets[norm_pair].append((cov_val, extra_val, not is_outlier))

            if is_outlier:
                layer, attribution = _classify_outlier(
                    pair_status=raw_pair,
                    coverage=cov_val,
                    extra_length_mm=extra_val,
                    within_threshold=fit_ok,
                    min_coverage=min_coverage,
                    max_extra_mm=max_extra_mm,
                    flattened_runs=s_flat,
                    comp=comp,
                )
                outliers.append(
                    {
                        "sheet_id": sheet_id,
                        "pid_sha256_prefix": sha_prefix,
                        "pair_status": norm_pair,
                        "coverage": round(cov_val, 6),
                        "extra_length_mm": round(extra_val, 3),
                        "text_matched": t_match,
                        "text_reference": t_ref,
                        "within_threshold": fit_ok,
                        "flattened_runs": s_flat,
                        "layer": layer,
                        "attribution": attribution,
                    }
                )
        else:
            uncompared_sheets += 1

        if s_flat > 0:
            flattened_sheets.append(
                {
                    "sheet_id": sheet_id,
                    "pid_sha256_prefix": sha_prefix,
                    "flattened": s_flat,
                    "total_texts": s_total,
                    "coverage": round(cov_val, 6) if cov_val is not None else None,
                    "text_matched": t_match,
                    "text_reference": t_ref,
                }
            )

        sanitized_sheets.append(
            {
                "sheet_id": sheet_id,
                "pid_sha256_prefix": sha_prefix,
                "pair_status": norm_pair,
                "load_ok": load_ok,
                "plot_ok": plot_ok,
                "cad_import_ms": round(cad_ms, 2),
                "scene_prepare_ms": round(prep_ms, 2),
                "total_open_ms": round(open_ms, 2),
                "plot_ms": round(plot_ms, 2),
                "cad_entities": ents,
                "primitives": prims,
                "on_page": on_page,
                "flattened_runs": s_flat,
                "coverage": round(cov_val, 6) if cov_val is not None else None,
                "extra_length_mm": round(extra_val, 3) if extra_val is not None else None,
                "text_matched": t_match,
                "text_reference": t_ref,
                "within_threshold": fit_ok,
            }
        )

    cad_sorted = sorted(cad_import_vals)
    prep_sorted = sorted(scene_prepare_vals)
    open_sorted = sorted(open_vals)
    plot_sorted = sorted(plot_vals)
    cov_sorted = sorted(coverages)
    extra_sorted = sorted(extras)

    total_open_ms = sum(open_vals)
    total_cad_ms = sum(cad_import_vals)
    total_prep_ms = sum(scene_prepare_vals)
    total_plot_ms = sum(plot_vals)

    mean_open_ms = statistics.fmean(open_vals) if open_vals else 0.0
    median_open_ms = statistics.median(open_vals) if open_vals else 0.0
    mean_cad_ms = statistics.fmean(cad_import_vals) if cad_import_vals else 0.0
    mean_prep_ms = statistics.fmean(scene_prepare_vals) if scene_prepare_vals else 0.0
    mean_plot_ms = statistics.fmean(plot_vals) if plot_vals else 0.0

    budget_per_sheet_ms = baseline_per_sheet_ms * max_open_multiple
    open_budget_passed = (
        sheet_count > 0
        and load_failures == 0
        and plot_failures == 0
        and mean_open_ms <= budget_per_sheet_ms
    )

    def _summarize_cohort(items: list[tuple[float, float, bool]]) -> dict[str, Any]:
        if not items:
            return {
                "count": 0,
                "mean_coverage": 0.0,
                "median_coverage": 0.0,
                "min_coverage": 0.0,
                "mean_extra_length_mm": 0.0,
                "pass_count": 0,
                "outlier_count": 0,
            }
        c_vals = sorted(x[0] for x in items)
        e_vals = [x[1] for x in items]
        passes = sum(1 for x in items if x[2])
        return {
            "count": len(items),
            "mean_coverage": round(statistics.fmean(c_vals), 6),
            "median_coverage": round(statistics.median(c_vals), 6),
            "min_coverage": round(c_vals[0], 6),
            "mean_extra_length_mm": round(statistics.fmean(e_vals), 3),
            "pass_count": passes,
            "outlier_count": len(items) - passes,
        }

    return {
        "schema_version": 1,
        "manifest_sha256": manifest_sha256,
        "debug_assertions": bool(bench_report.get("debug_assertions", False)),
        "l1_5_performance": {
            "sheet_count": sheet_count,
            "load_failures": load_failures,
            "plot_failures": plot_failures,
            "warmup_ms": round(warmup_ms, 2),
            "total_cad_import_ms": round(total_cad_ms, 2),
            "total_scene_prepare_ms": round(total_prep_ms, 2),
            "total_open_ms": round(total_open_ms, 6),
            "mean_cad_import_ms": round(mean_cad_ms, 2),
            "median_cad_import_ms": round(_percentile(cad_sorted, 0.50), 2),
            "p95_cad_import_ms": round(_percentile(cad_sorted, 0.95), 2),
            "max_cad_import_ms": round(cad_sorted[-1], 2) if cad_sorted else 0.0,
            "mean_scene_prepare_ms": round(mean_prep_ms, 2),
            "p95_scene_prepare_ms": round(_percentile(prep_sorted, 0.95), 2),
            "mean_open_ms": round(mean_open_ms, 6),
            "median_open_ms": round(median_open_ms, 6),
            "p95_open_ms": round(_percentile(open_sorted, 0.95), 2),
            "max_open_ms": round(open_sorted[-1], 6) if open_sorted else 0.0,
            "slowest_sheet_id": slowest_sheet_id,
            "slowest_pid_sha256_prefix": slowest_sha_prefix,
            "total_plot_ms": round(total_plot_ms, 2),
            "mean_plot_ms": round(mean_plot_ms, 2),
            "median_plot_ms": round(_percentile(plot_sorted, 0.50), 2),
            "p95_plot_ms": round(_percentile(plot_sorted, 0.95), 2),
            "max_plot_ms": round(plot_sorted[-1], 2) if plot_sorted else 0.0,
            "total_cad_entities": total_cad_entities,
            "total_pdf_primitives": total_primitives,
            "total_pdf_on_page": total_on_page,
            "total_pdf_bytes": total_pdf_bytes,
            "baseline_per_sheet_ms": baseline_per_sheet_ms,
            "max_open_multiple": max_open_multiple,
            "budget_per_sheet_ms": round(budget_per_sheet_ms, 2),
            "actual_vs_baseline_multiple": (
                round(mean_open_ms / baseline_per_sheet_ms, 4)
                if baseline_per_sheet_ms > 0
                else 0.0
            ),
            "cad_import_vs_baseline_multiple": (
                round(mean_cad_ms / baseline_per_sheet_ms, 4)
                if baseline_per_sheet_ms > 0
                else 0.0
            ),
            "open_budget_passed": open_budget_passed,
        },
        "l1_4_text_runs": {
            "total_texts": tr_total,
            "uniform": tr_uniform,
            "no_runs": tr_no_runs,
            "flattened": tr_flattened,
            "unresolvable": tr_unresolvable,
            "length_mismatch": tr_length_mismatch,
            "sheets_with_flattened": sheets_with_flattened,
            "sheets_with_unresolvable": sheets_with_unresolvable,
            "flattened_ratio": round(tr_flattened / tr_total, 6) if tr_total > 0 else 0.0,
            "flattened_sheets": flattened_sheets,
        },
        "l1_2_pdf_compare": {
            "compared_sheets": compared_sheets,
            "uncompared_sheets": uncompared_sheets,
            "min_coverage_threshold": min_coverage,
            "max_extra_mm_threshold": max_extra_mm,
            "mean_coverage": round(statistics.fmean(cov_sorted), 6) if cov_sorted else 0.0,
            "median_coverage": round(statistics.median(cov_sorted), 6) if cov_sorted else 0.0,
            "p05_coverage": round(_percentile(cov_sorted, 0.05), 6) if cov_sorted else 0.0,
            "min_coverage": round(cov_sorted[0], 6) if cov_sorted else 0.0,
            "mean_extra_length_mm": round(statistics.fmean(extra_sorted), 3) if extra_sorted else 0.0,
            "median_extra_length_mm": round(statistics.median(extra_sorted), 3) if extra_sorted else 0.0,
            "p95_extra_length_mm": round(_percentile(extra_sorted, 0.95), 3) if extra_sorted else 0.0,
            "max_extra_length_mm": round(extra_sorted[-1], 3) if extra_sorted else 0.0,
            "total_text_matched": total_text_matched,
            "total_text_reference": total_text_reference,
            "text_match_ratio": (
                round(total_text_matched / total_text_reference, 6)
                if total_text_reference > 0
                else 0.0
            ),
            "within_fit_threshold_count": within_fit_count,
            "count_ge_0_995": ge_0_995,
            "buckets": {
                "exact_1_000": exact_1_000,
                "ge_0_980": ge_0_980,
                "below_0_980": below_0_980,
            },
            "cohorts": {
                "exact": _summarize_cohort(cohort_buckets["exact"]),
                "fuzzy": _summarize_cohort(cohort_buckets["fuzzy"]),
            },
            "outliers": outliers,
        },
        "sheets": sanitized_sheets,
    }


def render_sanitized_markdown(summary: dict[str, Any]) -> str:
    perf = summary["l1_5_performance"]
    tr = summary["l1_4_text_runs"]
    pdf = summary["l1_2_pdf_compare"]
    buckets = pdf["buckets"]
    dbg = "开" if summary.get("debug_assertions") else "关（release）"

    lines: list[str] = [
        "# 94 张真实 `.pid` 批量打开性能与渲染出图扩面对比报告（`L1-2 / L1-4 / L1-5` · `#131`）",
        "",
        f"- 构建 `debug_assertions`：{dbg}",
    ]
    if summary.get("manifest_sha256"):
        lines.append(f"- 语料清单摘要（`manifest_sha256`）：`{summary['manifest_sha256']}`")
    lines += [
        "",
        "## 1. `L1-5` 批量打开与 PDF 渲染性能基线",
        "",
        "| 指标 | 实测值 | 口径说明 |",
        "|---|---:|---|",
        f"| 图纸总数 / 加载失败 / 出图失败 | `{perf['sheet_count']}` / `{perf['load_failures']}` / `{perf['plot_failures']}` | 全量 `.pid` 顺序热进程实测 |",
        f"| 一次性字体预热（`warmup_ms`） | `{perf['warmup_ms']:.1f} ms` | `lff + sysfont + cosmic-text` |",
        f"| CAD 导入总耗时 / 单图均值 / 中位数 / P95 / 最大 | `{perf['total_cad_import_ms']:.1f} ms` / `{perf['mean_cad_import_ms']:.1f} ms` / `{perf['median_cad_import_ms']:.1f} ms` / `{perf['p95_cad_import_ms']:.1f} ms` / `{perf['max_cad_import_ms']:.1f} ms` | `io::load_file`（解析 + 样式 + 图元落库） |",
        f"| 场景准备总耗时 / 单图均值 / P95 | `{perf['total_scene_prepare_ms']:.1f} ms` / `{perf['mean_scene_prepare_ms']:.1f} ms` / `{perf['p95_scene_prepare_ms']:.1f} ms` | `purge + xref + caches + prepare_open_geometry` |",
        f"| 完整打开总耗时 / 单图均值 / 中位数 / P95 / 最大 | `{perf['total_open_ms']:.1f} ms` / `{perf['mean_open_ms']:.1f} ms` / `{perf['median_open_ms']:.1f} ms` / `{perf['p95_open_ms']:.1f} ms` / `{perf['max_open_ms']:.1f} ms` | 最慢图纸：`{perf['slowest_sheet_id']}`（`{perf['slowest_pid_sha256_prefix']}`） |",
        f"| 对比基线（`{perf['baseline_per_sheet_ms']:.1f} ms/图` × `{perf['max_open_multiple']:.2f}x` = `{perf['budget_per_sheet_ms']:.1f} ms/图`） | `{perf['actual_vs_baseline_multiple']:.3f}x`（门禁：`{'通过' if perf['open_budget_passed'] else '未通过'}`） | 纯导入为基线的 `{perf['cad_import_vs_baseline_multiple']:.3f}x` |",
        f"| `plot_pdf` 渲染总耗时 / 单图均值 / P95 / 最大 | `{perf['total_plot_ms']:.1f} ms` / `{perf['mean_plot_ms']:.1f} ms` / `{perf['p95_plot_ms']:.1f} ms` / `{perf['max_plot_ms']:.1f} ms` | 累计 `{perf['total_pdf_primitives']}` 图元（页内 `{perf['total_pdf_on_page']}`） |",
        "",
        "## 2. `L1-4` 多样式字符 run 混排全批核查",
        "",
        "| 文本记录总数 | `Uniform`（单样式 run） | `NoRuns`（段落默认） | `Flattened`（多样式展平） | `Unresolvable` / `LengthMismatch` | 含 `Flattened` 图纸数 |",
        "|---:|---:|---:|---:|---:|---:|",
        f"| `{tr['total_texts']}` | `{tr['uniform']}` | `{tr['no_runs']}` | `{tr['flattened']}`（`{tr['flattened_ratio'] * 100:.3f}%`） | `{tr['unresolvable']}` / `{tr['length_mismatch']}` | `{tr['sheets_with_flattened']} / {perf['sheet_count']}` |",
        "",
        "## 3. `L1-2` 渲染出图对 `EXPORT_PDF` 全批对比汇总",
        "",
        "| 指标 | 全部对比图纸 | 精确同名配对（`exact`） | 近似改名候选（`fuzzy`） |",
        "|---|---:|---:|---:|",
        f"| 图纸数 | `{pdf['compared_sheets']}` | `{pdf['cohorts']['exact']['count']}` | `{pdf['cohorts']['fuzzy']['count']}` |",
        f"| 覆盖率均值 / 中位数 / 最低 | `{pdf['mean_coverage']:.6f}` / `{pdf['median_coverage']:.6f}` / `{pdf['min_coverage']:.6f}` | `{pdf['cohorts']['exact']['mean_coverage']:.6f}` / `{pdf['cohorts']['exact']['median_coverage']:.6f}` / `{pdf['cohorts']['exact']['min_coverage']:.6f}` | `{pdf['cohorts']['fuzzy']['mean_coverage']:.6f}` / `{pdf['cohorts']['fuzzy']['median_coverage']:.6f}` / `{pdf['cohorts']['fuzzy']['min_coverage']:.6f}` |",
        f"| 多画长度均值（mm） | `{pdf['mean_extra_length_mm']:.3f}` | `{pdf['cohorts']['exact']['mean_extra_length_mm']:.3f}` | `{pdf['cohorts']['fuzzy']['mean_extra_length_mm']:.3f}` |",
        f"| 覆盖率分桶（`=1.000` / `[0.980, 1.000)` / `<0.980`） | `{buckets['exact_1_000']}` / `{buckets['ge_0_980']}` / `{buckets['below_0_980']}` | — | — |",
        f"| 文字匹配总数 / 参考总数（比例） | `{pdf['total_text_matched']} / {pdf['total_text_reference']}`（`{pdf['text_match_ratio'] * 100:.2f}%`） | — | — |",
        "",
    ]

    outliers = pdf["outliers"]
    lines.append(f"## 4. 阈值外图纸逐张脱敏归因（共 `{len(outliers)}` 张）")
    lines.append("")
    if not outliers:
        lines.append("- 全部对比图纸均在覆盖率、多画长度与页面对位阈值内（无异常项）。")
    else:
        lines += [
            "| 图纸编号 | `.pid` SHA-256 前缀 | 配对方式 | 覆盖率 | 多画长度 (mm) | 文字对上 / 参考 | 对位通过 | `Flattened` | 归因层级 | 技术原因 |",
            "|---|---|---|---:|---:|---:|---|---:|---|---|",
        ]
        for item in outliers:
            t_cell = f"{item['text_matched']} / {item['text_reference']}"
            lines.append(
                f"| `{item['sheet_id']}` | `{item['pid_sha256_prefix']}` | `{item['pair_status']}` | "
                f"`{item['coverage']:.6f}` | `{item['extra_length_mm']:.3f}` | `{t_cell}` | "
                f"`{item['within_threshold']}` | `{item['flattened_runs']}` | `{item['layer']}` | {item['attribution']} |"
            )
    lines.append("")
    return "\n".join(lines)


def _load_pair_map_from_manifest(manifest_path: Path) -> tuple[str, dict[str, str]]:
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    digest = str(payload.get("manifest_sha256") or "")
    pair_map: dict[str, str] = {}
    drawings = (payload.get("correspondence") or {}).get("drawings") or []
    for row in drawings:
        stem = str(row.get("drawing_stem") or "")
        status = str(row.get("pdf_status") or "exact")
        if stem:
            pair_map[stem] = status
    return digest, pair_map


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = argparse.ArgumentParser(
        description="Evaluate 94-sheet .pid open performance, L1-4 text runs, and L1-2 PDF comparisons (#131)."
    )
    parser.add_argument("--bench-json", required=True, type=Path, help="Path to bench.json")
    parser.add_argument(
        "--compare-dir",
        required=True,
        type=Path,
        help="Directory holding <stem>.json outputs from pid_plot_compare",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=None,
        help="Optional corpus.manifest.json for correspondence status and manifest_sha256",
    )
    parser.add_argument("--out-json", type=Path, default=None, help="Sanitized summary JSON output path")
    parser.add_argument("--out-md", type=Path, default=None, help="Sanitized Markdown report output path")
    parser.add_argument("--baseline-ms", type=float, default=732.1, help="Baseline mean open ms per sheet")
    parser.add_argument("--max-multiple", type=float, default=1.5, help="Max allowed multiple over baseline")
    parser.add_argument("--min-coverage", type=float, default=0.980, help="Outlier minimum coverage threshold")
    parser.add_argument("--max-extra-mm", type=float, default=200.0, help="Outlier max extra stroke length (mm)")
    args = parser.parse_args(list(argv) if argv is not None else None)

    if not args.bench_json.is_file():
        print(f"error: bench JSON not found: {args.bench_json}", file=sys.stderr)
        return 2
    if not args.compare_dir.is_dir():
        print(f"error: compare directory not found: {args.compare_dir}", file=sys.stderr)
        return 2

    bench_report = json.loads(args.bench_json.read_text(encoding="utf-8"))
    compare_records: dict[str, dict[str, Any]] = {}
    for p in sorted(args.compare_dir.glob("*.json")):
        try:
            compare_records[p.stem] = json.loads(p.read_text(encoding="utf-8"))
        except Exception as exc:
            print(f"warning: skipping unreadable compare JSON {p.name}: {exc}", file=sys.stderr)

    manifest_sha256 = ""
    pair_map: dict[str, str] = {}
    if args.manifest is not None:
        if not args.manifest.is_file():
            print(f"error: manifest not found: {args.manifest}", file=sys.stderr)
            return 2
        manifest_sha256, pair_map = _load_pair_map_from_manifest(args.manifest)

    summary = evaluate_l1_batch(
        bench_report,
        compare_records,
        pair_status_by_stem=pair_map,
        baseline_per_sheet_ms=args.baseline_ms,
        max_open_multiple=args.max_multiple,
        min_coverage=args.min_coverage,
        max_extra_mm=args.max_extra_mm,
        manifest_sha256=manifest_sha256,
    )
    md = render_sanitized_markdown(summary)

    if args.out_json is not None:
        args.out_json.parent.mkdir(parents=True, exist_ok=True)
        args.out_json.write_text(
            json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
    if args.out_md is not None:
        args.out_md.parent.mkdir(parents=True, exist_ok=True)
        args.out_md.write_text(md, encoding="utf-8")

    perf = summary["l1_5_performance"]
    pdf = summary["l1_2_pdf_compare"]
    print(
        f"batch_l1_eval: sheets={perf['sheet_count']} mean_open_ms={perf['mean_open_ms']:.1f} "
        f"budget_passed={perf['open_budget_passed']} compared={pdf['compared_sheets']} "
        f"exact_1_000={pdf['buckets']['exact_1_000']} outliers={len(pdf['outliers'])}"
    )
    return 0 if perf["open_budget_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
