# SVG cross-renderer compatibility evidence (P5.3 / P8.3, 2026-09-09)

Third acceptance layer, compatibility half: the pages this tree writes,
rendered by the renderers people actually open SVGs with, judged against
resvg — the tree's own rasteriser, itself held to poppler by P5.2 — with
the same comparator (`src/io/raster_compare.rs`: 1 px shift window, 2/255
value tolerance, 32 px tiles with an 8-defect budget, 2 px page-edge skip,
one-way conflation excusal on the external side). A FAIL here is a
*disagreement to look at*, not a verdict on who is wrong: resvg is the
reference because it is the one engine the tree can run, and merge_lines
is the standing case where it is the one that is wrong.

Renderers (`versions.txt`; none enters the dependency tree):

| column | engine | how it was driven |
|---|---|---|
| resvg | resvg 0.45.1 (dev-dependency, the reference) | `compare_external_renders` renders in-process at the same dpi |
| chromium | Google Chrome 152.0.7977.83 | `--headless=new --screenshot`, `--force-device-scale-factor = dpi/96`, `--force-gpu-mem-available-mb=4096`, shot cropped to the page |
| inkscape | Inkscape 1.4.4 (dcaf3e7, 2026-05-05) | `inkscape.com --export-type=png --export-dpi=<dpi> --export-background=white` |
| rsvg | librsvg 2.62.91 via libvips 8.18.6 / sharp 0.35.4 (cairo 1.18.4) | `sharp(svg, { density: sqrt(72 × dpi) })`, installed under `%LOCALAPPDATA%\ocs-svg-compat\node` on first use |

Pipeline: `scripts/svg-compat.ps1` — (1) samples: the 22 corpus pages
(`dump_the_corpus_for_a_human`) plus the merge_lines minimal pair from
`../2026-09-09-svg-pdf-raster/`, and the three real P&ID sheets FF02-06 /
SP02-05 / WS02-05 through `--plot-svg --model --paper A1 --landscape
--fit` (the same plot the raster evidence took); (2) each renderer writes
`<stem>.<renderer>.png` beside `<stem>.svg`; (3) the ignored test
`compare_external_renders` (`OCS_SVG_COMPAT_DIR`, `OCS_SVG_COMPAT_DPI`)
renders the resvg reference and writes `compat-<dpi>dpi.tsv` plus a diff
map per disagreeing pair. winget has no `rsvg-convert` package; the
librsvg column is the same library reached through libvips.

## Files

- `corpus-600dpi.tsv` — 24 samples (22 corpus pages + the merge_lines
  minimal pair) × 3 renderers = 72 pairs at 600 dpi, the dpi the
  comparator is calibrated at. **66 ok, 6 FAIL**, and the six are the two
  merge_lines pages × three renderers (below). Every other pair is 0 or
  1 defect pixel out of 35 megapixels — including the samples the plan
  named for the eyeball pass: negative scale + clip (01–04), fine dashes
  (05), multiply + wipeout hatches (13), text (17), CTB pens (15–16).
- `real-sheets-254dpi.tsv` — the three real sheets × 3 renderers at
  254 dpi (10 px/mm: an A1 comes out a whole 8410 × 5940 in every engine).
  **All nine FAIL the local metric; none has a structural defect** (next
  section, and `real-sheets-254dpi-structural.tsv`).
- `real-sheets-254dpi-structural.tsv` — the same nine pairs, each defect
  pixel classified: *structural* (ink on one side, no ink within 2 px on
  the other = missing / displaced content) or *edge residue* (both sides
  have ink within 2 px = anti-aliasing coverage disagreement). Quartiles
  of the darkest channel on both sides show where the defects sit on the
  grey scale.
- `real-sheets-300dpi-superseded.tsv` — the first real-sheet pass, at
  300 dpi, kept for the record of why 254: every renderer failed ~300
  tiles in one row (tile row 214, y ≈ 580–583 mm: the sheet's bottom
  frame line), Chromium included although its page size matched resvg's
  exactly — a horizontal edge sitting on a fractional pixel row at
  11.81 px/mm, where the engines' coverage values disagree along its whole
  length; Inkscape and librsvg additionally round the page to whole pixels
  (9933 wide) where resvg and Chromium ceil (9934). At 254 dpi the sizes
  agree and that row of tiles is gone in all three.
- `calibration-150dpi.tsv` — the very first pass (corpus + real sheets)
  at 150 dpi, kept as the calibration record: at screen resolution a
  0.75 pt line is 1.5 px and the renderers' anti-aliasing quantisation
  trips the 2/255 tolerance on every thin stroke (dashes, CTB hatch,
  colour-adapted lines, the two_groups pages; `pen_widths` 08 in the two
  cairo engines only). All of those pages are 0 defects at 600 dpi, so
  none of it is the export's. The real-sheet rows in this table predate
  the Chromium tile-budget flag (see "Renderer quirks") and are not
  evidence about the export either.
- `*-4up.png` — the same region in the four engines, nearest-neighbour
  zoom, the comparator's worst tile dashed where it applies.

## The merge_lines answer: resvg alone drops the fill

`../2026-09-09-svg-pdf-raster/README.md` documents that resvg 0.45.1 drops
the one on-sheet multiply fill of `hatches, merge_lines` when three things
meet — an `isolation:isolate` layer, a layer content bbox spanning
geometry ~1.3e7 units off-sheet, and an ancestor `clip-path` — and that
poppler paints it from the equivalent PDF. This run fed the minimal pair
and the full corpus page to the other three engines:

| sample | resvg 0.45.1 | Chrome 152 | Inkscape 1.4.4 | librsvg 2.62.91 |
|---|---|---|---|---|
| `merge-lines-minimal-renders.svg` (no clip wrapper) | fill paints | paints (0 defects vs resvg) | paints (0) | paints (0) |
| `merge-lines-minimal-drops.svg` (inside `<g clip-path>`) | **fill dropped** | **paints** (9119 px differ from resvg = the fill) | **paints** (9119) | **paints** (9214) |
| `corpus-14-hatches__merge_lines.svg` (the real page) | dropped | paints (9119) | paints (9119) | paints (9214) |

`merge-lines-minimal-drops-600dpi-4up.png` and
`corpus-14-hatches-merge_lines-600dpi-4up.png` show it: the resvg panel is
blank where the other three carry the red multiply square. The "defects"
in those six FAIL rows are exactly the fill's area at 600 dpi (≈ 95 × 96
px; librsvg's 9214 adds its anti-aliased fringe). **The drop is
resvg 0.45.1's alone**; the browser, Inkscape and librsvg agree with
poppler. Consequence for the plan's pending decision: the export-side
"clip off-sheet ink" change stays un-upgraded — recorded here, to be
re-verified when the tree bumps resvg.

## The real sheets: the metric fails, the pictures agree

At 254 dpi (`real-sheets-254dpi.tsv`):

| sheet | chromium | inkscape | rsvg |
|---|---|---|---|
| FF02-06 | 270 px / 6 tiles (worst 24) | 442 / 11 (30) | 442 / 11 (30) |
| SP02-05 | 1437 / 51 (44) | 5956 / 187 (43) | 2158 / 68 (43) |
| WS02-05 | 96 / 2 (25) | 242 / 7 (30) | 242 / 7 (30) |

Against 50 megapixels per sheet that is 0.0002–0.012 %, but the metric is
local on purpose, and worst tiles of 24–44 defects are over the budget of
8. So each pair was taken apart:

- **Structural defects: 0 in all nine pairs**
  (`real-sheets-254dpi-structural.tsv`). Every defect pixel has ink within
  2 px on both sides; nothing is missing, displaced or absent.
- **What the defects are** (most common (external, resvg) colour pairs,
  from the raw renders):
  - a sub-pixel vertical hairline at x = 507 mm on FF02-06, rendered
    `(127,127,127)` (50 % coverage) by resvg and `(97..99)` (~61 %) by
    Chrome, Inkscape and librsvg — 82 of Chrome's 270 defects and the
    same rows in the cairo engines; the hairline policy P5.2 met with
    poppler, now between resvg and everyone else;
  - the 75 %-coverage edge row of thin table rules on SP02-05's right-hand
    material table (x 720–821 mm), `(63,63,63)` in resvg and `(60,60,60)`
    in Inkscape — one level past the tolerance, along ~380 mm of rules:
    3798 of Inkscape's 5956 defects, and why Inkscape's count is the
    outlier;
  - Skia's supersampled coverage on the same edge rows in Chrome
    (`(39..47)` where resvg has `63`) and faint anti-aliasing fringes
    beyond strokes (`(224..245)` where resvg has paper);
  - the edge row of a red pipe line at y = 125 mm on SP02-05
    (`(194,92,92)` in the cairo engines, `(159,95,95)` in resvg);
  - glyph-stroke edges inside the dense 2.5–3.5 mm labels (the worst
    tiles of all nine pairs sit on text or on a thick frame line; the
    `*-worst-tile-*-4up.png` crops show the same geometry in all four
    engines).

Verdict: **no compatibility defect in the export on the real sheets.**
The disagreements are anti-aliasing coverage policies on features ≤ 2 px
wide, the same class the P5.2 evidence documents for poppler vs resvg,
here between four engines. The rows stay FAIL with this note; nobody
tunes the budget to make them pass. A comparator that wants to pass real
sheets across engines would need either a coverage-aware tolerance for
sub-2-px features or a 1200 dpi run — both belong to the "teach the judge"
follow-up already scoped in the 2026-09-09 plan, not to the export.

## Renderer quirks met on the way (none is the export's)

- **Chromium headless tile memory budget.** An A1 at 300 dpi
  (9934 × 7016, 279 MB) came back white below row 6351, no error: headless
  Chrome does not paint raster tiles beyond its default GPU memory budget.
  `--force-gpu-mem-available-mb=4096` lifts it; the script carries the
  flag and the comment.
- **Page size rounding.** Inkscape and librsvg round the page to whole
  pixels; resvg and Chromium ceil. At 150 dpi that is 1754 × 1240 vs
  1754 × 1241, at 300 dpi 9933 vs 9934. The comparator aligns a one-pixel
  size difference bottom-left; a dpi at which the page is a whole number
  of pixels (254 = 10 px/mm for an A-series sheet) removes the question.
- **librsvg through libvips applies `density` twice** to a page stated in
  mm (librsvg lays the mm out at `density` px/in, libvips scales the
  picture by density/72 again; measured 72 → 72 dpi, 96 → 128, 150 →
  312.5); `sqrt(72 × dpi)` lands on `dpi` device pixels per inch
  (1754 px wide for an A4 landscape page at 150 dpi, as resvg).
- **Cairo hairlines at low dpi.** `pen_widths` 08 at 150 dpi: 585 defects
  in Inkscape and librsvg, 1 in Chrome, 0 in all three at 600 dpi — the
  cairo engines put a 1.5 px stroke on the grid differently from the Skia
  family; not a matter for the export.

## Follow-ups

1. merge_lines: re-run `scripts/svg-compat.ps1 -Only corpus` when the
   tree bumps resvg; the export-side clip of off-sheet ink is not
   upgraded (only resvg drops the fill).
2. Real-sheet cross-engine FAIL rows: fold into the "teach the judge
   coloured ink / sub-2-px coverage or 1200 dpi" phase scoped in the
   2026-09-09 plan (§6.2); prove the planted faults still land before any
   tolerance moves.
3. Nothing to fix in the export from this run.
