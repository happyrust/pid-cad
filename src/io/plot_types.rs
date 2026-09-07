// Paper-space plot inputs shared by every plot backend.
//
// These types describe *what* is on a plotted page — the wires, hatches and
// wipeouts already flattened to sheet coordinates, plus the page geometry and
// output options — without saying which file format receives it. They used to
// be defined in `pdf_export.rs` (hence the `Pdf…` names, kept so the existing
// call sites in `app/update/file.rs` and `io/print_to_printer.rs` do not have
// to change); `pdf_export` re-exports them.

use crate::io::plot_emit::PlotPage;
use crate::io::plot_style::PlotStyleTable;
use crate::scene::model::hatch_model::HatchModel;
use crate::scene::WireModel;

/// A wire plus the draw-order depth the plot sorts it by.
#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct PlotWire {
    pub wire: WireModel,
    pub draw_depth: f32,
}

impl std::ops::Deref for PlotWire {
    type Target = WireModel;

    fn deref(&self) -> &Self::Target {
        &self.wire
    }
}

/// Output controls shared by preview, PDF export, and printer rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfPlotOptions {
    pub object_lineweights: bool,
    pub scale_lineweights: bool,
    pub transparency: bool,
    pub stamp: bool,
    pub merge_lines: bool,
    pub group_splits: PlotGroupSplits,
}

/// End indexes of the first paper/model render group in each flat input list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlotGroupSplits {
    pub wires: usize,
    pub hatches: usize,
    pub wipeouts: usize,
}

/// Owned render data for one page in a multi-page PDF.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct PdfPageInput {
    pub wires: std::sync::Arc<Vec<PlotWire>>,
    pub hatches: Vec<HatchModel>,
    pub wipeouts: Vec<HatchModel>,
    pub paper_w: f64,
    pub paper_h: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub rotation_deg: i32,
    pub scale: f32,
    pub clip: Option<(f32, f32, f32, f32)>,
    pub options: PdfPlotOptions,
    pub plot_style: Option<PlotStyleTable>,
}

impl PdfPageInput {
    /// The page as the emitter wants it. `fallback` is the job-wide plot style
    /// table; a page that names its own overrides it, which is the priority
    /// the PDF exporter has always used and every backend has to keep.
    pub fn as_plot_page<'a>(&'a self, fallback: Option<&'a PlotStyleTable>) -> PlotPage<'a> {
        PlotPage {
            wires: &self.wires,
            hatches: &self.hatches,
            wipeouts: &self.wipeouts,
            paper_w: self.paper_w as f32,
            paper_h: self.paper_h as f32,
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            rotation_deg: self.rotation_deg,
            scale: self.scale,
            clip: self.clip,
            plot_style: self.plot_style.as_ref().or(fallback),
            options: self.options,
        }
    }
}

impl Default for PdfPlotOptions {
    fn default() -> Self {
        Self {
            object_lineweights: true,
            scale_lineweights: false,
            transparency: false,
            stamp: false,
            merge_lines: false,
            group_splits: PlotGroupSplits::default(),
        }
    }
}
