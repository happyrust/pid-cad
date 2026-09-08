//! Paper sizes and sheet orientation for window plotting.
//!
//! This table is the one place standard sheets are named — the plot dialog's
//! dropdown, the page-setup label inference and the command line's `--paper`
//! all read it, so a size added here appears everywhere at once (P7 of
//! docs/plans/2026-09-08-svg-export-next-steps.md).

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PaperSize {
    A4,
    A3,
    A2,
    A1,
    A0,
    AnsiA,
    AnsiB,
    AnsiC,
    AnsiD,
    AnsiE,
}

impl PaperSize {
    pub const ALL: [PaperSize; 10] = [
        PaperSize::A4,
        PaperSize::A3,
        PaperSize::A2,
        PaperSize::A1,
        PaperSize::A0,
        PaperSize::AnsiA,
        PaperSize::AnsiB,
        PaperSize::AnsiC,
        PaperSize::AnsiD,
        PaperSize::AnsiE,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaperSize::A4 => "A4",
            PaperSize::A3 => "A3",
            PaperSize::A2 => "A2",
            PaperSize::A1 => "A1",
            PaperSize::A0 => "A0",
            PaperSize::AnsiA => "ANSI A",
            PaperSize::AnsiB => "ANSI B",
            PaperSize::AnsiC => "ANSI C",
            PaperSize::AnsiD => "ANSI D",
            PaperSize::AnsiE => "ANSI E",
        }
    }

    /// Portrait dimensions in mm (width, height); width < height.
    pub fn dimensions_mm(self) -> (f64, f64) {
        match self {
            PaperSize::A4 => (210.0, 297.0),
            PaperSize::A3 => (297.0, 420.0),
            PaperSize::A2 => (420.0, 594.0),
            PaperSize::A1 => (594.0, 841.0),
            PaperSize::A0 => (841.0, 1189.0),
            // ANSI sheets are defined in inches (8.5×11 doubling up to 34×44);
            // these are those figures exactly, at 25.4 mm to the inch.
            PaperSize::AnsiA => (215.9, 279.4),
            PaperSize::AnsiB => (279.4, 431.8),
            PaperSize::AnsiC => (431.8, 558.8),
            PaperSize::AnsiD => (558.8, 863.6),
            PaperSize::AnsiE => (863.6, 1117.6),
        }
    }

    /// The size a label names, spelling aside: case and the space, hyphen or
    /// underscore between "ANSI" and its letter are all accepted, so the
    /// dialog's "ANSI B" and a script's `ansi-b` are the same sheet.
    pub fn from_label(label: &str) -> Option<PaperSize> {
        let folded: String = label
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '_'))
            .map(|c| c.to_ascii_uppercase())
            .collect();
        Self::ALL.iter().copied().find(|size| {
            size.label()
                .chars()
                .filter(|c| *c != ' ')
                .eq(folded.chars())
        })
    }
}

/// A sheet the command line can name: a standard size from the table, or an
/// explicit `W×H` in portrait mm.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum PaperSpec {
    Standard(PaperSize),
    /// Width and height in mm, exactly as given.
    Custom {
        width_mm: f64,
        height_mm: f64,
    },
}

impl PaperSpec {
    /// Dimensions in mm as stated: portrait for a standard size, verbatim for
    /// a custom one. Orientation is the caller's business.
    pub fn dimensions_mm(self) -> (f64, f64) {
        match self {
            PaperSpec::Standard(size) => size.dimensions_mm(),
            PaperSpec::Custom {
                width_mm,
                height_mm,
            } => (width_mm, height_mm),
        }
    }
}

/// Read `--paper`: a standard name (`A3`, `ansi-b`) or `WxH` in mm
/// (`300x600`, `297.5x420`). A size that is not positive and finite is
/// refused here, before any drawing is opened.
pub fn parse_paper(input: &str) -> Result<PaperSpec, String> {
    let input = input.trim();
    if let Some(size) = PaperSize::from_label(input) {
        return Ok(PaperSpec::Standard(size));
    }
    let custom = input
        .split_once(['x', 'X', '×'])
        .and_then(|(w, h)| Some((w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?)))
        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0);
    match custom {
        Some((width_mm, height_mm)) => Ok(PaperSpec::Custom {
            width_mm,
            height_mm,
        }),
        None => Err(format!(
            "unknown paper size '{input}'. Known sizes: {}; or say WxH in mm, like 300x600",
            PaperSize::ALL.map(PaperSize::label).join(", ")
        )),
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// Sheet dimensions in mm for the given size and orientation.
pub fn sheet_mm(size: PaperSize, o: Orientation) -> (f64, f64) {
    let (w, h) = size.dimensions_mm();
    match o {
        Orientation::Portrait => (w, h),
        Orientation::Landscape => (h, w),
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum PlotScale {
    /// Scale so the window fills the sheet minus a 5% margin — or exactly
    /// into the margin box when margins are stated (`--margins`).
    Fit,
    /// Exact scale factor (mm per drawing unit).
    Ratio(f64),
}

/// Map a world-space window onto a sheet. Returns (scale, offset_x, offset_y),
/// where the scaled window is centered on the sheet and (offset_x, offset_y) is
/// the sheet-mm position of the window's min corner. The caller turns that into
/// its own coordinate offset.
pub fn window_to_sheet(
    window_wh: (f64, f64),
    sheet_mm: (f64, f64),
    scale: PlotScale,
) -> (f64, f64, f64) {
    window_to_sheet_margins(window_wh, sheet_mm, None, scale)
}

/// [`window_to_sheet`], kept inside stated margins.
///
/// `margins_mm` is `[left, bottom, right, top]` on the un-rotated sheet.
/// With margins, `Fit` fills the margin box exactly — the margins are the
/// breathing room, so the historical 5% slack would double-count — and the
/// window is centered in that box rather than on the sheet. Without them,
/// this is `window_to_sheet` to the letter: fit keeps its 5% slack so every
/// plot made before margins existed keeps its bytes. The caller has checked
/// the margins leave some sheet; a margin box squeezed to nothing here would
/// only divide by it.
pub fn window_to_sheet_margins(
    window_wh: (f64, f64),
    sheet_mm: (f64, f64),
    margins_mm: Option<[f64; 4]>,
    scale: PlotScale,
) -> (f64, f64, f64) {
    let (ww, wh) = (window_wh.0.max(1e-9), window_wh.1.max(1e-9));
    let ([left, bottom, right, top], slack) = match margins_mm {
        Some(margins) => (margins, 1.0),
        None => ([0.0; 4], 1.05),
    };
    let usable = (
        (sheet_mm.0 - left - right).max(1e-9),
        (sheet_mm.1 - bottom - top).max(1e-9),
    );
    let scale = match scale {
        PlotScale::Ratio(r) => r.max(1e-9),
        PlotScale::Fit => {
            let sx = (usable.0 / slack) / ww;
            let sy = (usable.1 / slack) / wh;
            sx.min(sy)
        }
    };
    let scaled_w = ww * scale;
    let scaled_h = wh * scale;
    let offset_x = left + (usable.0 - scaled_w) / 2.0;
    let offset_y = bottom + (usable.1 - scaled_h) / 2.0;
    (scale, offset_x, offset_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dimensions_and_orientation() {
        assert_eq!(PaperSize::A4.dimensions_mm(), (210.0, 297.0));
        assert_eq!(PaperSize::A0.dimensions_mm(), (841.0, 1189.0));
        assert_eq!(PaperSize::ALL.len(), 10);
        assert_eq!(PaperSize::A3.label(), "A3");
        // Portrait keeps (w,h); landscape swaps.
        assert_eq!(
            sheet_mm(PaperSize::A4, Orientation::Portrait),
            (210.0, 297.0)
        );
        assert_eq!(
            sheet_mm(PaperSize::A4, Orientation::Landscape),
            (297.0, 210.0)
        );
    }

    #[test]
    fn ansi_sheets_are_the_inch_sizes_in_mm() {
        // 8.5×11 in up to 34×44 in, at exactly 25.4 mm/in.
        assert_eq!(PaperSize::AnsiA.dimensions_mm(), (215.9, 279.4));
        assert_eq!(PaperSize::AnsiB.dimensions_mm(), (279.4, 431.8));
        assert_eq!(PaperSize::AnsiE.dimensions_mm(), (863.6, 1117.6));
        assert_eq!(PaperSize::AnsiC.label(), "ANSI C");
        assert_eq!(
            sheet_mm(PaperSize::AnsiB, Orientation::Landscape),
            (431.8, 279.4)
        );
    }

    #[test]
    fn a_label_names_its_size_whatever_the_spelling() {
        assert_eq!(PaperSize::from_label("A3"), Some(PaperSize::A3));
        assert_eq!(PaperSize::from_label("a0"), Some(PaperSize::A0));
        for spelling in ["ANSI B", "ansi b", "ansi-b", "ANSI_B", "AnsiB"] {
            assert_eq!(
                PaperSize::from_label(spelling),
                Some(PaperSize::AnsiB),
                "{spelling}"
            );
        }
        assert_eq!(PaperSize::from_label("A9"), None);
        assert_eq!(PaperSize::from_label("300x600"), None);
        assert_eq!(PaperSize::from_label(""), None);
    }

    #[test]
    fn paper_parses_standard_names_and_custom_mm() {
        assert_eq!(
            parse_paper(" ansi-c "),
            Ok(PaperSpec::Standard(PaperSize::AnsiC))
        );
        assert_eq!(
            parse_paper("300x600"),
            Ok(PaperSpec::Custom {
                width_mm: 300.0,
                height_mm: 600.0
            })
        );
        assert_eq!(
            parse_paper("297.5 X 420"),
            Ok(PaperSpec::Custom {
                width_mm: 297.5,
                height_mm: 420.0
            })
        );
        for bad in ["A9", "300", "0x600", "-1x5", "AxB", "300x", "nanx5"] {
            let error = parse_paper(bad).expect_err(bad);
            assert!(error.contains("ANSI E"), "{bad}: {error}");
            assert!(error.contains("300x600"), "{bad}: {error}");
        }
    }

    #[test]
    fn margins_bound_the_fit_and_recenter_the_window() {
        // 100×50 window on a 297×210 sheet with 20/10/17/25 mm margins:
        // the box is 260×175, fit is exact (no 5% slack), limited by width.
        let (s, ox, oy) = window_to_sheet_margins(
            (100.0, 50.0),
            (297.0, 210.0),
            Some([20.0, 10.0, 17.0, 25.0]),
            PlotScale::Fit,
        );
        assert!((s - 2.6).abs() < 1e-9, "scale {s}");
        assert!(
            (ox - 20.0).abs() < 1e-9,
            "flush against the left margin: {ox}"
        );
        assert!(
            (oy - (10.0 + (175.0 - 130.0) / 2.0)).abs() < 1e-9,
            "oy {oy}"
        );

        // An exact ratio is untouched; only the centering moves into the box.
        let (s, ox, oy) = window_to_sheet_margins(
            (100.0, 50.0),
            (297.0, 210.0),
            Some([20.0, 10.0, 17.0, 25.0]),
            PlotScale::Ratio(1.0),
        );
        assert!((s - 1.0).abs() < 1e-9);
        assert!(
            (ox - (20.0 + (260.0 - 100.0) / 2.0)).abs() < 1e-9,
            "ox {ox}"
        );
        assert!((oy - (10.0 + (175.0 - 50.0) / 2.0)).abs() < 1e-9, "oy {oy}");

        // No margins = the old behaviour, byte for byte: 5% slack, sheet
        // centering. (The wrapper and the original must agree.)
        assert_eq!(
            window_to_sheet_margins((100.0, 100.0), (210.0, 297.0), None, PlotScale::Fit),
            window_to_sheet((100.0, 100.0), (210.0, 297.0), PlotScale::Fit)
        );
    }

    #[test]
    fn fit_centers_and_scales_within_margin() {
        // 100×100 window onto a 210×297 sheet, Fit. Limiting axis is width:
        // usable = 210/1.05 = 200; scale = 200/100 = 2.0.
        let (s, ox, oy) = window_to_sheet((100.0, 100.0), (210.0, 297.0), PlotScale::Fit);
        assert!((s - 2.0).abs() < 1e-9, "scale {s}");
        // Window is 100*2 = 200 wide/tall; centered on 210×297.
        assert!((ox - (210.0 - 200.0) / 2.0).abs() < 1e-9, "ox {ox}");
        assert!((oy - (297.0 - 200.0) / 2.0).abs() < 1e-9, "oy {oy}");
    }

    #[test]
    fn ratio_applies_exact_scale_and_centers() {
        // Ratio(0.5): scale is exactly 0.5; the scaled window is centered and
        // (ox, oy) is the sheet-mm position of its min corner.
        let (s, ox, oy) = window_to_sheet((100.0, 80.0), (210.0, 297.0), PlotScale::Ratio(0.5));
        assert!((s - 0.5).abs() < 1e-9);
        // scaled window = 50×40; centered box origin = ((210-50)/2,(297-40)/2).
        assert!((ox - (210.0 - 50.0) / 2.0).abs() < 1e-9, "ox {ox}");
        assert!((oy - (297.0 - 40.0) / 2.0).abs() < 1e-9, "oy {oy}");
    }
}
