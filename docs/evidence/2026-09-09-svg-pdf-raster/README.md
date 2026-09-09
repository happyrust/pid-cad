# PDF ↔ SVG raster comparison evidence (P5.2, 2026-09-09)

Third acceptance layer, second half: the same page out both doors —
`export_pdf_pages` and the SVG writer — rasterised and compared pixel by
pixel. Tools: `pdftoppm` (poppler 25.07.0, winget, `-thinlinemode shape`)
for the PDF, resvg 0.45.1 (the tree's own dev-dependency) for the SVG.
Comparator: `src/io/raster_compare.rs` — 1 px shift, 2/255 value tolerance,
32 px tiles with an 8-defect budget, 2 px page-edge skip, and a one-way
conflation excusal. The PDF door writes each glyph as its triangle mesh,
and poppler anti-aliases every triangle alone, so inside ink the SVG fills
solid the PDF raster is conflation all the way down: interiors sum 6–23%
short of full coverage, and shared edges crack to near-white — no depth
threshold separates artifact from fault (measured on FF02-06). Structure
does: every conflated pixel has fully covered triangle ink within 2 px,
so where the SVG pixel is solid ink (≤ 96/255) the PDF is excused any
value not darker than the SVG, as long as solid PDF ink sits within that
reach. The excusal never applies to the SVG side — missing, thinned or
displaced SVG ink still counts — and a lightened PDF *area* (a wrong
screen) has no solid ink within reach of its middle and still fails
(`raster_compare`'s own tests prove each direction).

- `corpus-600dpi.tsv` — the whole corpus at 600 dpi, text pages included
  (written by `dump_raster_evidence`, single-threaded so the shared glyph
  atlas holds still between the two exports of a text page).
- `real-sheets-600dpi.tsv` — three real P&ID sheets (FF02-06, SP02-05,
  WS02-05) through the whole headless plot path at 600 dpi (written by
  `app::automation::dump_real_sheet_raster_evidence`). **All three FAIL**
  (527 / 2340 / 3785 defect px; 3 / 27 / 74 tiles over budget) — tiny
  against a 278-megapixel A1, but the metric is local on purpose. The
  worst-tile crops sit next to the table (`crop-real-*.png`) and show three
  different stories: WS02-05's defects run *inside* the strokes of a large
  title (the mesh-seam pattern the conflation excusal exists for — it does
  not hold on these glyphs, why is an open question), SP02-05's cluster
  where arrows cross a red pipe line (which smells like a real draw-order
  or fill difference), FF02-06 has three tiles at a text/line junction.
  Per-tile triage is its own phase (see the 2026-09-09 plan); these rows
  are FAIL until it says otherwise. 300 dpi was tried
  first: the linework passed but every label speckled — these sheets are
  wall-to-wall 2.5–3.5 mm text, a glyph stroke is 2–3 px at 300 dpi, and
  hardly any pixel of a label counts as solid ink on either side, so the
  conflation excusal has nothing to hold on to. At 600 dpi the stroke
  interiors are real.
- `diff-*.png` — every pair with a non-zero defect count, defects marked
  on the PDF rendering.

## The merge_lines exception

`hatches, merge_lines` FAILs the comparison by design of the renderer, not
of the export. The page is the far-from-origin case (plot window at world
(500000, 4500000) mm), so all but one multiply fill sits ~1.3e7 SVG units
off-sheet, cut by the page clip. resvg 0.45.1 drops the one on-sheet
multiply fill when three things meet: an `isolation:isolate` layer, a layer
content bbox spanning the far-off geometry, and an ancestor `clip-path`.
Remove any one and the fill paints; poppler paints it from the equivalent
PDF in every variant. The minimal pair in this folder differs only by the
clip wrapper:

| file | structure | resvg 0.45.1 |
|---|---|---|
| `merge-lines-minimal-renders.svg` | isolate + far-off path + on-sheet multiply fill | fill paints |
| `merge-lines-minimal-drops.svg` | the same inside `<g clip-path="url(#page-clip)">` | fill dropped |

Cross-checks on the full corpus page: removing `isolation:isolate` paints,
removing the `clip-path` attribute paints, the untouched page drops. To
re-render any of these: `OCS_SVG_PREVIEW=<dir or file> cargo test --lib
preview_written_svgs -- --ignored --nocapture`.

The routine gate (`the_pdf_and_the_svg_rasterise_to_the_same_picture`)
excuses merge_lines pages with a comment stating the same facts, exactly as
it excuses the sub-pixel hairline pages.

## The real-sheet triage (2026-09-09, same evening)

All three FAILs were taken apart against raw renders (a temporary local
patch of `dump_real_sheet_raster_evidence` dumped `{sheet}-pdf.png` /
`{sheet}-svg.png` and the run reproduced the committed numbers exactly),
with per-pixel scanline sampling at the worst tiles. **Verdict: no export
defect — two limitations of the judge.**

- **SP02-05, and FF02-06's junction specks: the hairline policy meeting
  other ink.** The sheets carry 0.1 pt hairline linework (the arrows'
  crossbars, symbol details). Poppler pins a hairline to a full-value
  one-pixel row; resvg spreads its true ~0.8 px coverage — over bare paper
  the 1 px shift window explains the disagreement away, which is why the
  sheet is not red everywhere. Where the hairline meets the red pipe line
  or a table rule, the PDF's solid-black row has no dark counterpart
  within reach — measured at (1465, 2953): PDF `0,0,0`, SVG `208,16,16`
  (the line's red fringe over the hairline's fifth of a row) — and every
  crossing charges a few dozen pixels. The 0.25 mm stems align
  pixel-for-pixel in both renders; nothing is missing, displaced or the
  wrong colour. Same class the corpus already documents on the
  `pen widths (scale_lw=false, object_lw=false)` row.
- **WS02-05: the conflation excusal does not know coloured ink.** The
  title's ink is blue `(0,38,128)`. The excusal gates "solid ink" per
  channel at `seam_ink` = 96; blue's own channel sits at 128, so in the
  blue channel the title never qualifies, and poppler's mesh underpainting
  — measured `(33,66,144)` and `(58,87,157)` inside strokes the SVG fills
  solid `(0,38,128)` — is charged in that channel. This is precisely the
  artifact the excusal exists for, unexcused for coloured ink: "96 keeps
  coloured ink in" holds for a colour's dark channels, never its bright
  one.

Follow-up, scoped and not done here: teach the excusal what coloured ink
is (qualify a pixel as ink by its darkest channel — its distance from
paper — keep it one-way, and prove the planted faults still land), and/or
run the real-sheet evidence at 1200 dpi, where 0.1 pt clears the raster
floor (≈1.7 px) — at the price of ~4 GB pixmaps per engine. Until one of
those lands, these three rows stay FAIL with this note; nobody tunes the
budget to make them pass.
