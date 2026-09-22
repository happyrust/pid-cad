// Paper-space plot inputs shared by every plot backend.
//
// These types describe *what* is on a plotted page — the wires, hatches,
// wipeouts and raster images already flattened to sheet coordinates, plus the
// page geometry and output options — without saying which file format
// receives it. They used to be defined in `pdf_export.rs` (hence the `Pdf…`
// names, kept so the existing call sites in `app/update/file.rs` and
// `io/print_to_printer.rs` do not have to change); `pdf_export` re-exports
// them.

use crate::io::plot_emit::PlotPage;
use crate::io::plot_style::PlotStyleTable;
use crate::scene::model::hatch_model::HatchModel;
use crate::scene::model::image_model::ImageModel;
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

/// Decoded image geometry plus inherited block/viewport clip boundaries.
#[derive(Clone, Debug)]
pub struct PlotImage {
    pub image: ImageModel,
    pub clips: Vec<Vec<[f64; 2]>>,
}

/// Output controls shared by preview, PDF export, and printer rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfPlotOptions {
    pub object_lineweights: bool,
    pub scale_lineweights: bool,
    pub transparency: bool,
    pub stamp: bool,
    pub merge_lines: bool,
}

/// End indexes of the first paper/model render group in each flat input list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlotGroupSplits {
    pub wires: usize,
    pub hatches: usize,
    pub wipeouts: usize,
    pub images: usize,
}

/// Everything drawn on one page, flattened to sheet coordinates: the two
/// render groups (paper space, then the model seen through its viewports)
/// laid end to end in each list, with `group_splits` saying where the first
/// group ends.
#[derive(Default)]
pub struct PlotContent {
    pub wires: std::sync::Arc<Vec<PlotWire>>,
    pub hatches: Vec<HatchModel>,
    pub wipeouts: Vec<HatchModel>,
    pub images: Vec<PlotImage>,
    pub group_splits: PlotGroupSplits,
}

/// Owned geometry and settings for one page in a (multi-page) plot job.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct PdfPageInput {
    pub content: PlotContent,
    /// Page dimensions in mm, after any 90/270-degree rotation.
    pub paper_w: f64,
    pub paper_h: f64,
    /// Absolute-world offsets stay f64 to preserve local detail at UTM coordinates.
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
            wires: &self.content.wires,
            hatches: &self.content.hatches,
            wipeouts: &self.content.wipeouts,
            images: &self.content.images,
            group_splits: self.content.group_splits,
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
        }
    }
}
