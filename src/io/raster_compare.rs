//! P5.2 — the third layer's other half: rasterise the PDF and the SVG this
//! crate writes for the same page, and ask whether they are the same picture
//! (§8 of docs/plans/2026-09-07-dxf-to-svg-export.md, P5.2 of the next-steps
//! plan).
//!
//! The PDF rasteriser is external and stays out of the dependency tree (§6
//! decision 2): `OCS_PDF_RASTERIZER` names an executable; otherwise `mutool`,
//! then `pdftoppm`, is taken from PATH, then from winget's links folder, and
//! last from the `oschwartz10612.Poppler` package folder itself — that
//! package declares no command aliases, so `winget install` links nothing
//! and the executables sit only under winget's Packages directory.
//! When none is found the comparison tests print SKIPPED and check nothing —
//! a loud skip, since `#[ignore]` cannot be decided at runtime — and the
//! evidence records which engine, at which version, the thresholds were
//! calibrated against.
//!
//! The metric is local, as the plan requires, because a global difference
//! rate answers the wrong question: a correct hairline that lands one pixel
//! over flips most of its edge pixels, while a whole missing label on a large
//! white sheet barely moves the average. Here a pixel is a **defect** when
//! its value cannot be explained by the other picture anywhere within
//! `shift_px` of it — outside the min..max of the other side's neighbourhood,
//! widened by `value_tol` — in either direction. An edge drawn one pixel over
//! under a different anti-aliasing kernel is explained; a missing line, ink
//! where a wipeout should have been, a dash out of phase or a colour that is
//! simply wrong is not. Defects are counted per tile, so a failure names
//! where to look and a page-wide veil of one-off pixels cannot hide a hole.
//! Channels are compared separately: a red line where a green one should be
//! is invisible to a grey metric and is not invisible here.

use std::path::{Path, PathBuf};
use std::process::Command;

use resvg::tiny_skia::Pixmap;

// ── The external PDF rasteriser ───────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flavor {
    MuTool,
    PdfToPpm,
}

pub struct PdfRasterizer {
    pub exe: PathBuf,
    pub flavor: Flavor,
    /// The first version line the tool printed, for the evidence.
    pub version: String,
}

impl PdfRasterizer {
    /// Find a rasteriser without ever guessing silently: the env var wins,
    /// then PATH, then the winget links folder. `None` means the machine has
    /// none and the caller should say SKIPPED, not pass.
    pub fn discover() -> Option<PdfRasterizer> {
        if let Ok(named) = std::env::var("OCS_PDF_RASTERIZER") {
            let exe = PathBuf::from(named);
            let flavor = if exe
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().to_ascii_lowercase().contains("mutool"))
            {
                Flavor::MuTool
            } else {
                Flavor::PdfToPpm
            };
            return Self::probe(exe, flavor);
        }
        let links = std::env::var("LOCALAPPDATA").map(|base| {
            Path::new(&base)
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
        });
        let candidates = [
            (PathBuf::from("mutool"), Flavor::MuTool),
            (PathBuf::from("pdftoppm"), Flavor::PdfToPpm),
        ];
        for (name, flavor) in &candidates {
            if let Some(found) = Self::probe(name.clone(), *flavor) {
                return Some(found);
            }
        }
        if let Ok(links) = links {
            for (name, flavor) in &candidates {
                let exe = links.join(name).with_extension("exe");
                if exe.is_file() {
                    if let Some(found) = Self::probe(exe, *flavor) {
                        return Some(found);
                    }
                }
            }
        }
        // The winget Poppler package declares no aliases, so nothing reaches
        // Links or PATH: look inside the package's own folder, newest
        // `poppler-*` first.
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            let packages = Path::new(&base)
                .join("Microsoft")
                .join("WinGet")
                .join("Packages");
            let mut bins: Vec<PathBuf> = std::fs::read_dir(&packages)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("oschwartz10612.Poppler")
                })
                .flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten().flatten())
                .filter(|e| e.file_name().to_string_lossy().starts_with("poppler-"))
                .map(|e| e.path().join("Library").join("bin").join("pdftoppm.exe"))
                .filter(|exe| exe.is_file())
                .collect();
            bins.sort();
            if let Some(exe) = bins.pop() {
                if let Some(found) = Self::probe(exe, Flavor::PdfToPpm) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// The tool, if running it answers. The version comes from the banner —
    /// both tools put "version" on the first line or two of `-v`.
    fn probe(exe: PathBuf, flavor: Flavor) -> Option<PdfRasterizer> {
        let output = Command::new(&exe).arg("-v").output().ok()?;
        let banner = [output.stderr, output.stdout]
            .concat()
            .split(|b| *b == b'\n')
            .map(|line| String::from_utf8_lossy(line).trim().to_string())
            .find(|line| line.to_ascii_lowercase().contains("version"))
            .unwrap_or_default();
        Some(PdfRasterizer {
            exe,
            flavor,
            version: banner,
        })
    }

    /// First page of `pdf` at `dpi`, on white. The intermediate PNG lives in
    /// `work` and is removed after decoding.
    pub fn rasterize(&self, pdf: &Path, dpi: f32, work: &Path) -> Result<Pixmap, String> {
        std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
        let stem = format!(
            "rc-{}-{}",
            std::process::id(),
            pdf.file_stem().unwrap_or_default().to_string_lossy()
        );
        let png = work.join(format!("{stem}.png"));
        let output = match self.flavor {
            // `-thinlinemode shape`: Splash's default clamps a stroke thinner
            // than one device pixel to a solid pixel, while resvg draws its
            // true coverage — a 0.83 px hairline came back black from one
            // and 76% grey from the other, a policy difference at the
            // sub-pixel floor, not a drawing difference. `shape` has poppler
            // draw the coverage too, so a hairline row is judged like any
            // other ink.
            Flavor::PdfToPpm => Command::new(&self.exe)
                .args(["-png", "-r"])
                .arg(format!("{dpi}"))
                .args([
                    "-f",
                    "1",
                    "-l",
                    "1",
                    "-singlefile",
                    "-thinlinemode",
                    "shape",
                ])
                .arg(pdf)
                .arg(work.join(&stem))
                .output(),
            Flavor::MuTool => Command::new(&self.exe)
                .args(["draw", "-r"])
                .arg(format!("{dpi}"))
                .arg("-o")
                .arg(&png)
                .arg(pdf)
                .arg("1")
                .output(),
        }
        .map_err(|e| format!("{}: {e}", self.exe.display()))?;
        if !output.status.success() {
            return Err(format!(
                "{} failed on {}: {}",
                self.exe.display(),
                pdf.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let bytes = std::fs::read(&png).map_err(|e| format!("{}: {e}", png.display()))?;
        let _ = std::fs::remove_file(&png);
        Pixmap::decode_png(&bytes).map_err(|e| e.to_string())
    }
}

/// Render an SVG document at `px_per_mm`, on white — the SVG half of every
/// comparison, so both test sites (the corpus in `io::svg_export::tests`, the
/// real sheets in `app::automation`) rasterise the same way.
pub fn render_svg(svg: &str, px_per_mm: f32) -> Pixmap {
    use resvg::usvg;
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("the document parses");
    // usvg sizes the tree in CSS px at 96 dpi; scale from there.
    let factor = px_per_mm * 25.4 / 96.0;
    let w = (tree.size().width() * factor).ceil() as u32;
    let h = (tree.size().height() * factor).ceil() as u32;
    let mut pixmap = Pixmap::new(w, h).expect("a pixmap");
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(factor, factor),
        &mut pixmap.as_mut(),
    );
    pixmap
}

// ── The comparison ────────────────────────────────────────────────────────

/// The gates, as the plan first states them (calibrated numbers live with the
/// tests that use them): a defect must escape a `shift_px` neighbourhood and
/// `value_tol` channel steps to count, and a tile fails on `tile_budget`
/// defects. 1 px of shift is the plan's initial tolerance at 600 dpi; run at
/// a lower dpi the same 1 px is spatially stricter, never looser.
pub struct RasterCmp {
    pub shift_px: u32,
    /// Channel tolerance in 0..255 steps, the plan's flat-patch 1–2/255 with
    /// one step for the engines rounding differently.
    pub value_tol: u8,
    pub tile_px: u32,
    pub tile_budget: u32,
    /// Pixels ignored along the page border. The engines round the page's
    /// pixel size a step apart, so a stroke *on* the page edge — half of it
    /// already cut away — is cut at rows that differ by up to the rounding,
    /// beyond any shift a drawing defect is allowed. Two pixels is 0.17 mm at
    /// 300 dpi: content that close to the physical edge of the sheet is
    /// judged by everything but its outermost sliver.
    pub edge_skip_px: u32,
    /// The darkest a channel value may be and still count as solid ink for
    /// the conflation excusal (see `seam_reach_px`). 96 keeps coloured ink
    /// in; a 50% screen of black — 127 — is out.
    pub seam_ink: u8,
    /// The PDF door writes a glyph as its triangle mesh, and poppler
    /// anti-aliases every triangle alone, so inside a stroke the SVG fills
    /// solid the PDF raster is conflation all the way down: interiors sum
    /// 6–23% short of full coverage, and shared edges crack to near-white
    /// (measured on the real sheets — depth separates nothing). What *is*
    /// reliable is structure: every conflated pixel has a fully covered
    /// triangle interior within two pixels, while real faults — a wrong
    /// screen, missing or displaced ink — leave whole areas without solid
    /// PDF ink. So where the SVG pixel is solid ink, `a` is excused any
    /// value not darker than `b` as long as solid PDF ink sits within this
    /// radius. One-way: SVG ink is never excused by it.
    pub seam_reach_px: u32,
}

impl Default for RasterCmp {
    fn default() -> Self {
        RasterCmp {
            shift_px: 1,
            value_tol: 2,
            tile_px: 32,
            tile_budget: 8,
            edge_skip_px: 2,
            seam_ink: 96,
            seam_reach_px: 2,
        }
    }
}

#[derive(Debug)]
pub struct RasterVerdict {
    pub width: u32,
    pub height: u32,
    pub tiles_x: u32,
    pub tiles_y: u32,
    /// Defect count per tile, row-major.
    pub tile_counts: Vec<u32>,
    pub defects_total: u64,
    /// Tile x, tile y, count — the worst tile on the page.
    pub worst: (u32, u32, u32),
    /// One bit per pixel of the compared overlap, row-major, for diff maps.
    pub defects: Vec<bool>,
    pub tile_budget: u32,
}

impl RasterVerdict {
    pub fn failing_tiles(&self) -> usize {
        self.tile_counts
            .iter()
            .filter(|count| **count > self.tile_budget)
            .count()
    }

    pub fn ok(&self) -> bool {
        self.failing_tiles() == 0
    }
}

/// One channel plane of the pixmap's bottom `height` rows. The engines round
/// a page's pixel height differently — poppler a row or two short, resvg a
/// row up — and a page is anchored at its bottom-left corner (PDF's origin),
/// so rows are taken from the bottom or a rounding step becomes a page-wide
/// vertical shift of everything on it.
fn plane(pixmap: &Pixmap, channel: usize, width: u32, height: u32) -> Vec<u8> {
    let y0 = pixmap.height() - height;
    let mut out = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let p = pixmap.pixel(x, y0 + y).expect("inside the pixmap");
            out[(y * width + x) as usize] = match channel {
                0 => p.red(),
                1 => p.green(),
                _ => p.blue(),
            };
        }
    }
    out
}

/// Sliding window minimum and maximum, radius `r`, two separable passes.
fn window_min_max(plane: &[u8], width: u32, height: u32, r: u32) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    let r = r as usize;
    let mut row_min = vec![0u8; w * h];
    let mut row_max = vec![0u8; w * h];
    for y in 0..h {
        let row = &plane[y * w..(y + 1) * w];
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(w);
            let mut min = u8::MAX;
            let mut max = u8::MIN;
            for &v in &row[lo..hi] {
                min = min.min(v);
                max = max.max(v);
            }
            row_min[y * w + x] = min;
            row_max[y * w + x] = max;
        }
    }
    let mut out_min = vec![0u8; w * h];
    let mut out_max = vec![0u8; w * h];
    for y in 0..h {
        let lo = y.saturating_sub(r);
        let hi = (y + r + 1).min(h);
        for x in 0..w {
            let mut min = u8::MAX;
            let mut max = u8::MIN;
            for row in lo..hi {
                min = min.min(row_min[row * w + x]);
                max = max.max(row_max[row * w + x]);
            }
            out_min[y * w + x] = min;
            out_max[y * w + x] = max;
        }
    }
    (out_min, out_max)
}

/// Compare two renderings of what should be the same page. The engines are
/// allowed to disagree about a page's exact pixel count by a rounding step;
/// more than `2` px of size difference is its own failure, not a comparison.
pub fn compare(a: &Pixmap, b: &Pixmap, cfg: &RasterCmp) -> Result<RasterVerdict, String> {
    if a.width().abs_diff(b.width()) > 2 || a.height().abs_diff(b.height()) > 2 {
        return Err(format!(
            "the pages are different sizes: {}×{} vs {}×{}",
            a.width(),
            a.height(),
            b.width(),
            b.height()
        ));
    }
    let width = a.width().min(b.width());
    let height = a.height().min(b.height());
    let tiles_x = width.div_ceil(cfg.tile_px);
    let tiles_y = height.div_ceil(cfg.tile_px);
    let mut tile_counts = vec![0u32; (tiles_x * tiles_y) as usize];
    let mut defects = vec![false; (width * height) as usize];

    for channel in 0..3 {
        let plane_a = plane(a, channel, width, height);
        let plane_b = plane(b, channel, width, height);
        let (min_a, max_a) = window_min_max(&plane_a, width, height, cfg.shift_px);
        let (min_b, max_b) = window_min_max(&plane_b, width, height, cfg.shift_px);
        // The conflation excusal's reach (see `seam_reach_px`): the nearest
        // fully covered PDF ink may sit a step beyond the shift window.
        let (reach_min_a, _) = window_min_max(&plane_a, width, height, cfg.seam_reach_px);
        let tol = cfg.value_tol;
        let ink = cfg.seam_ink;
        for i in 0..(width * height) as usize {
            let unexplained_a = plane_a[i] < min_b[i].saturating_sub(tol)
                || plane_a[i] > max_b[i].saturating_add(tol);
            let unexplained_b = plane_b[i] < min_a[i].saturating_sub(tol)
                || plane_b[i] > max_a[i].saturating_add(tol);
            // Poppler's conflation (see `seam_reach_px`): inside ink the SVG
            // lays solid, the PDF raster's value is unreliable wholesale —
            // underpainted interiors, cracks to near-white — so only the
            // presence of solid PDF ink within reach is judged there. `a`
            // darker than `b` is still real (extra ink is not conflation),
            // and `b` is never excused.
            let conflated = plane_b[i] <= ink
                && reach_min_a[i] <= ink
                && plane_a[i] >= plane_b[i].saturating_sub(tol);
            if (unexplained_a || unexplained_b) && !conflated {
                defects[i] = true;
            }
        }
    }

    // The border band is not judged (`edge_skip_px`); it is cleared in the
    // bitmap too, so a diff map shows what was judged, not what was excused.
    let skip = cfg.edge_skip_px;
    let mut defects_total = 0u64;
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) as usize;
            if x < skip || x >= width - skip || y < skip || y >= height - skip {
                defects[i] = false;
                continue;
            }
            if defects[i] {
                defects_total += 1;
                let tile = (y / cfg.tile_px) * tiles_x + (x / cfg.tile_px);
                tile_counts[tile as usize] += 1;
            }
        }
    }
    let worst = tile_counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .map(|(i, count)| (i as u32 % tiles_x, i as u32 / tiles_x, *count))
        .unwrap_or((0, 0, 0));

    Ok(RasterVerdict {
        width,
        height,
        tiles_x,
        tiles_y,
        tile_counts,
        defects_total,
        worst,
        defects,
        tile_budget: cfg.tile_budget,
    })
}

/// The defect map over a faded copy of `a`, for the evidence folder: what
/// differs is red, everything else is the page at quarter strength.
pub fn diff_map(a: &Pixmap, verdict: &RasterVerdict) -> Pixmap {
    let y0 = a.height() - verdict.height;
    let mut out = Pixmap::new(verdict.width, verdict.height).expect("a diff pixmap");
    let data = out.data_mut();
    for y in 0..verdict.height {
        for x in 0..verdict.width {
            let i = (y * verdict.width + x) as usize;
            let p = a.pixel(x, y0 + y).expect("inside the pixmap");
            let (r, g, b) = if verdict.defects[i] {
                (255, 0, 0)
            } else {
                let fade = |v: u8| 255 - (255 - v) / 4;
                (fade(p.red()), fade(p.green()), fade(p.blue()))
            };
            data[i * 4] = r;
            data[i * 4 + 1] = g;
            data[i * 4 + 2] = b;
            data[i * 4 + 3] = 255;
        }
    }
    out
}

// ── The metric proves itself before it judges anything ────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 3]) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).expect("a pixmap");
        let data = pixmap.data_mut();
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = paint(x, y);
                let i = ((y * width + x) * 4) as usize;
                data[i] = r;
                data[i + 1] = g;
                data[i + 2] = b;
                data[i + 3] = 255;
            }
        }
        pixmap
    }

    fn white() -> [u8; 3] {
        [255, 255, 255]
    }

    /// A vertical dark line from `x0` to `x1`, softened one pixel each side
    /// the way an anti-aliasing kernel would.
    fn line(x: u32, x0: u32, x1: u32) -> [u8; 3] {
        if (x0..=x1).contains(&x) {
            [20, 20, 20]
        } else if x + 1 == x0 || x == x1 + 1 {
            [140, 140, 140]
        } else {
            white()
        }
    }

    #[test]
    fn an_edge_one_pixel_over_is_not_a_defect() {
        let a = page(96, 96, |x, _| line(x, 40, 42));
        let b = page(96, 96, |x, _| line(x, 41, 43));
        let verdict = compare(&a, &b, &RasterCmp::default()).unwrap();
        assert_eq!(
            verdict.defects_total, 0,
            "a one-pixel shift under a different kernel is rasterisation \
             noise, not a defect"
        );
    }

    #[test]
    fn a_missing_line_is_caught_where_it_is_missing() {
        let a = page(96, 96, |x, _| line(x, 40, 42));
        let b = page(96, 96, |_, _| white());
        let verdict = compare(&a, &b, &RasterCmp::default()).unwrap();
        assert!(!verdict.ok(), "a whole missing line passed");
        // And it is caught in the line's own column of tiles, not smeared.
        assert!(
            verdict.worst.0 == 40 / 32,
            "worst tile at {:?}",
            verdict.worst
        );
    }

    #[test]
    fn a_flat_patch_of_the_wrong_shade_is_caught() {
        let a = page(96, 96, |_, _| [200, 200, 200]);
        let b = page(96, 96, |_, _| [195, 195, 195]);
        let verdict = compare(&a, &b, &RasterCmp::default()).unwrap();
        assert!(
            !verdict.ok(),
            "five grey steps across a flat patch is a wrong colour, \
             not noise"
        );
        // Two steps, though, is the engines rounding differently.
        let c = page(96, 96, |_, _| [198, 198, 198]);
        let verdict = compare(&a, &c, &RasterCmp::default()).unwrap();
        assert_eq!(verdict.defects_total, 0);
    }

    #[test]
    fn a_wrong_hue_hides_from_grey_but_not_from_this() {
        // Same luminance, different colour: a grey metric calls these equal.
        let a = page(96, 96, |_, _| [200, 100, 100]);
        let b = page(96, 96, |_, _| [100, 200, 100]);
        let verdict = compare(&a, &b, &RasterCmp::default()).unwrap();
        assert!(!verdict.ok());
    }

    #[test]
    fn the_border_band_is_excused_and_the_interior_is_not() {
        // The same wrong ink: on the outermost row it is the engines
        // disagreeing where the page ends, two rows in it is a defect.
        let with_row = |row: u32| {
            page(
                96,
                96,
                move |_, y| {
                    if y == row {
                        [20, 20, 20]
                    } else {
                        white()
                    }
                },
            )
        };
        let blank = page(96, 96, |_, _| white());
        let at_edge = compare(&with_row(0), &blank, &RasterCmp::default()).unwrap();
        assert_eq!(at_edge.defects_total, 0, "the border band is not judged");
        let inside = compare(&with_row(4), &blank, &RasterCmp::default()).unwrap();
        assert!(!inside.ok(), "a missing row four pixels in is a defect");
    }

    #[test]
    fn pages_align_at_the_bottom_left_where_the_pdf_anchors_them() {
        // The same horizontal rule, drawn the same distance from the BOTTOM,
        // in two pixmaps whose heights round a step apart. Aligned at the
        // top, everything on the page would shift; aligned at the origin,
        // nothing has moved.
        let rule = |height: u32| {
            move |_: u32, y: u32| {
                if height - y == 40 {
                    [20, 20, 20]
                } else {
                    [255, 255, 255]
                }
            }
        };
        let a = page(96, 94, rule(94));
        let b = page(96, 96, rule(96));
        let verdict = compare(&a, &b, &RasterCmp::default()).unwrap();
        assert_eq!(
            verdict.defects_total, 0,
            "a two-row rounding difference read as a page-wide shift"
        );
    }

    /// A wide solid bar, optionally with one column underpainted the way
    /// poppler leaves a conflation seam across a triangle-meshed glyph.
    fn bar(seam: Option<(u32, u8)>) -> impl Fn(u32, u32) -> [u8; 3] {
        move |x, _| {
            if let Some((at, value)) = seam {
                if x == at {
                    return [value; 3];
                }
            }
            if (30..=60).contains(&x) {
                [20, 20, 20]
            } else if x + 1 == 30 || x == 61 {
                [140, 140, 140]
            } else {
                [255, 255, 255]
            }
        }
    }

    #[test]
    fn poppler_may_underpaint_a_seam_inside_solid_ink_but_the_svg_may_not() {
        let solid = page(96, 96, bar(None));
        let seamed = page(96, 96, bar(Some((45, 70))));
        // In `a`, the PDF side, a 50-step seam inside solid ink is the
        // rasteriser's conflation, excused.
        let verdict = compare(&seamed, &solid, &RasterCmp::default()).unwrap();
        assert_eq!(verdict.defects_total, 0, "a conflation seam was charged");
        // The same seam in `b` is the SVG thinning real ink: caught.
        let verdict = compare(&solid, &seamed, &RasterCmp::default()).unwrap();
        assert!(!verdict.ok(), "an SVG-side seam was excused");
    }

    #[test]
    fn a_deep_crack_is_conflation_but_a_broad_light_patch_is_not() {
        let solid = page(96, 96, bar(None));
        // One column nearly white inside the bar: a junction crack. Poppler's
        // conflation runs to near-white at the fan of a glyph junction, and
        // fully covered triangle interiors sit either side, so depth
        // separates nothing — structure does. Excused.
        let cracked = page(96, 96, bar(Some((45, 220))));
        let verdict = compare(&cracked, &solid, &RasterCmp::default()).unwrap();
        assert_eq!(verdict.defects_total, 0, "a deep thin crack was charged");
        // The same lightness thirteen columns wide reads as a wrong screen:
        // the middle of it has no solid PDF ink within reach.
        let base = bar(None);
        let patched = page(96, 96, move |x, y| {
            if (40..=52).contains(&x) {
                [150, 150, 150]
            } else {
                base(x, y)
            }
        });
        let verdict = compare(&patched, &solid, &RasterCmp::default()).unwrap();
        assert!(
            !verdict.ok(),
            "a broad light patch was excused as conflation"
        );
    }

    #[test]
    fn the_pdf_floor_inside_ink_is_excused_and_the_svg_floor_is_not() {
        // Poppler under-covers whole mesh interiors (6–23% on the real
        // sheets): the bar a shade light everywhere, structure intact, is
        // conflation. The same floor on the SVG side is thinned ink: caught.
        let solid = page(96, 96, bar(None));
        let base = bar(None);
        let floored = page(96, 96, move |x, y| {
            let v = base(x, y);
            if v == [20, 20, 20] {
                [45, 45, 45]
            } else {
                v
            }
        });
        let verdict = compare(&floored, &solid, &RasterCmp::default()).unwrap();
        assert_eq!(
            verdict.defects_total, 0,
            "the conflated interior was charged"
        );
        let verdict = compare(&solid, &floored, &RasterCmp::default()).unwrap();
        assert!(!verdict.ok(), "an SVG-side floor was excused");
    }

    #[test]
    fn sizes_a_rounding_step_apart_compare_and_worse_do_not() {
        let a = page(96, 96, |x, _| line(x, 40, 42));
        let b = page(95, 96, |x, _| line(x, 40, 42));
        assert!(compare(&a, &b, &RasterCmp::default()).unwrap().ok());
        let c = page(80, 96, |x, _| line(x, 40, 42));
        let error = compare(&a, &c, &RasterCmp::default()).expect_err("16 px apart");
        assert!(error.contains("different sizes"), "{error}");
    }
}
