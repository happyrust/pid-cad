// What one `.pid` import drew, dropped and assumed: the summary the open pipeline carries on the document, and the log lines behind it.

use super::*;

/// What the import wants the reader to know, sized for one command-line line:
/// how much of the file became drawing, how much did not, and whether the
/// style tables were readable at all. The detail behind each number stays in
/// the log (see [`report_import`]); this is the headline.
///
/// Returned by [`load_pid`] beside the document ([`PidImport`]), and carried
/// across the format-agnostic open pipeline inside the document itself
/// ([`ImportSummary::store`] / [`ImportSummary::take`]), since the pipeline's
/// `acadrust::ReadOutcome` has no room for it. Until plan
/// 2026-09-21-load-pid-returns-its-summary it travelled through a
/// process-wide mailbox keyed by path instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSummary {
    /// Entities handed to the document.
    pub drawn: usize,
    /// Decoded source records those entities came from.
    pub decoded: usize,
    /// Source records the parser saw but could not place: no decoder, or a
    /// shape the decoder for that type refuses.
    pub missing: usize,
    /// A style table failed to read outright, so line work, lettering or
    /// fills are on their fallbacks rather than the drawing's own statement.
    pub style_tables_failed: bool,
    /// Distinct authored layers referenced by drawn entities, keyed by
    /// storage-local oid rather than merged by display name.
    pub sheet_layers: usize,
    /// Drawn CAD entities carrying an authored sheet-layer reference.
    pub layered_entities: usize,
    /// Distinct referenced layer ids whose authored name did not resolve.
    pub unresolved_sheet_layers: usize,
    /// Distinct authored sheet layer *names* drawn entities state -- the rows
    /// of the Layer Manager's sheet-layer view. Fewer than `sheet_layers`
    /// when the same name is several layer objects across storages.
    pub sheet_layer_names: usize,
    /// How many of those names the import starts switched off in the
    /// drawing's view filter.
    pub sheet_layers_off: usize,
    /// Named driving dimensions over every symbol body the drawing caches --
    /// all of them on library templates no placement draws (pid-parse
    /// `docs/analysis/2026-09-18-the-parametric-chain-closes-on-the-template-
    /// not-the-instance.md`). A dimension without a name is not counted: the
    /// panel cannot show it.
    pub driving_dimensions: usize,
    /// Cached bodies carrying at least one driving dimension: the templates.
    pub template_bodies: usize,
    /// Placements whose cached body names a template with named dimensions,
    /// so whose entities carry `driving=` -- what the panel can show library
    /// defaults for. Zero on a drawing without parametric symbols, in which
    /// case the headline says nothing about dimensions at all.
    pub parametric_placements: usize,
    /// Symbol placements drawn from the body the drawing caches for them --
    /// every placement the drawing has a drawable body for. Logged, not
    /// shown: the shape being right is what the user sees.
    pub cache_bodies: usize,
    /// Symbol placements drawn from the library's `.sym`: those the drawing
    /// caches nothing drawable for.
    pub library_bodies: usize,
    /// Strokes of cached bodies left undrawn because the file switches the
    /// symbol-internal layer they sit on off (P-D2), summed over the
    /// placements whose cache was consulted.
    pub hidden_strokes_skipped: usize,
    /// The symbol library roots the import found -- `PID_SYMBOL_LIBRARY` or
    /// the `.sym` tree above the drawing -- and drew its library bodies
    /// from. Empty when no library was found. Where a placement's body came
    /// from otherwise depends on where the file sits, so the summary says
    /// which library it was (audit 2026-09-21, item 3).
    pub symbol_library: Vec<PathBuf>,
    /// The unit the drawing's coordinates were read in, or the fact that
    /// none was decoded and the metre was assumed -- a whole-drawing factor
    /// of 25.4 if the assumption is wrong on an imperial project, so it is
    /// stated here and not only in the log (audit 2026-09-21, item 6).
    pub unit: ImportUnit,
}

/// How a `.pid`'s source coordinates were scaled to millimetres.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportUnit {
    /// A decoded record stated its unit; `mm_per_unit` is what it is worth.
    Stated {
        /// The label as `pid-parse` states it (`m`, `mm`).
        unit: String,
        /// Millimetres per source unit.
        mm_per_unit: f64,
    },
    /// No decoded record stated a unit this importer knows; the metre --
    /// what every fixture measures -- was assumed.
    AssumedMetre,
}

impl ImportUnit {
    /// The unit the drawing's decoded records state, read off the first one
    /// that states a unit this importer knows.
    ///
    /// `pid-parse` puts the unit on each entity's coordinate context rather
    /// than on the document, and only decoded records carry one, so those
    /// are what is read. They agree with each other by construction,
    /// because the unit comes from the sheet's single page frame. A drawing
    /// whose frame the parser could not decode says nothing about units and
    /// falls back to the metre, which is what every fixture measured before
    /// the parser could say so -- and the log then records having assumed
    /// it, at `warn`: an imperial drawing read this way is off by 25.4
    /// everywhere and looks fine.
    pub(super) fn read(geometry: &NormalizedPidGeometry, path: &Path) -> Self {
        let stated = geometry
            .entities
            .iter()
            .filter(|entity| entity.confidence == PidGeometryConfidence::Decoded)
            .find_map(|entity| {
                let PidDrawingUnits::Known { unit } = &entity.coordinate_context.units else {
                    return None;
                };
                millimetres_in(&entity.coordinate_context.units).map(|mm| (unit.clone(), mm))
            });
        match stated {
            Some((unit, mm_per_unit)) => Self::Stated { unit, mm_per_unit },
            None => {
                log::warn!(
                    "{}: no decoded coordinate unit; assuming the metre, {MM_PER_METRE}mm per source unit",
                    path.display()
                );
                Self::AssumedMetre
            }
        }
    }

    /// Millimetres per source unit under this reading.
    pub fn mm_per_unit(&self) -> f64 {
        match self {
            Self::Stated { mm_per_unit, .. } => *mm_per_unit,
            Self::AssumedMetre => MM_PER_METRE,
        }
    }

    /// Whether the unit was assumed rather than read.
    pub fn is_assumed(&self) -> bool {
        matches!(self, Self::AssumedMetre)
    }
}

/// The tag prefix the summary rides under while a document crosses the open
/// pipeline: one custom document property per field
/// (`PID_IMPORT_SUMMARY.drawn`, …) in the document's `summary_info`. Taken
/// off again by the open-completion handler ([`ImportSummary::take`]), so a
/// saved drawing never carries it: the summary describes this import, not the
/// drawing.
///
/// A property rather than an XRecord beside `PID_VIEW_FILTER` (the plan's
/// first choice) because an XRecord costs a handle the allocator never gives
/// back: taken off again, it still left `$HANDSEED` and the handle of every
/// object allocated after the import one higher, so a `.pid` saved as DWG or
/// DXF no longer matched the same save from before the record existed. A
/// property costs the document nothing, and taking it off restores the
/// document exactly.
pub const SUMMARY_PROPERTY_PREFIX: &str = "PID_IMPORT_SUMMARY.";

impl ImportSummary {
    /// Write the summary into `doc` as custom document properties, one per
    /// field under [`SUMMARY_PROPERTY_PREFIX`], replacing any already there.
    pub fn store(&self, doc: &mut CadDocument) {
        Self::remove(doc);
        let properties = &mut doc.summary_info.custom_properties;
        let mut push = |key: &str, value: String| {
            properties.push((format!("{SUMMARY_PROPERTY_PREFIX}{key}"), value));
        };
        for (key, value) in self.counts() {
            push(key, value.to_string());
        }
        push("style_tables_failed", self.style_tables_failed.to_string());
        for root in &self.symbol_library {
            push("symbol_library", root.to_string_lossy().into_owned());
        }
        match &self.unit {
            ImportUnit::Stated { unit, mm_per_unit } => {
                push("unit", unit.clone());
                push("mm_per_unit", format!("{mm_per_unit:?}"));
            }
            ImportUnit::AssumedMetre => push("unit", "assumed-metre".to_string()),
        }
    }

    /// The summary `doc` carries, if any, leaving it in place.
    pub fn load(doc: &CadDocument) -> Option<Self> {
        let mut carried = false;
        let mut summary = Self::empty();
        let mut unit: Option<String> = None;
        let mut mm_per_unit: Option<f64> = None;
        for (tag, value) in &doc.summary_info.custom_properties {
            let Some(key) = tag.strip_prefix(SUMMARY_PROPERTY_PREFIX) else {
                continue;
            };
            carried = true;
            match key {
                "style_tables_failed" => summary.style_tables_failed = value == "true",
                "symbol_library" => summary.symbol_library.push(PathBuf::from(value)),
                "unit" => unit = Some(value.to_string()),
                "mm_per_unit" => mm_per_unit = value.parse().ok(),
                _ => {
                    if let (Ok(count), Some(slot)) =
                        (value.parse::<usize>(), summary.count_mut(key))
                    {
                        *slot = count;
                    }
                }
            }
        }
        if !carried {
            return None;
        }
        summary.unit = match (unit.as_deref(), mm_per_unit) {
            (Some("assumed-metre"), _) | (None, _) => ImportUnit::AssumedMetre,
            (Some(label), Some(mm_per_unit)) => ImportUnit::Stated {
                unit: label.to_string(),
                mm_per_unit,
            },
            (Some(label), None) => ImportUnit::Stated {
                unit: label.to_string(),
                mm_per_unit: millimetres_in(&PidDrawingUnits::Known {
                    unit: label.to_string(),
                })
                .unwrap_or(MM_PER_METRE),
            },
        };
        Some(summary)
    }

    /// The summary `doc` carries, taken off it: what the open-completion
    /// handler calls, so the headline is shown once and never saved. Leaves
    /// the document exactly as it was before [`ImportSummary::store`].
    pub fn take(doc: &mut CadDocument) -> Option<Self> {
        let summary = Self::load(doc)?;
        Self::remove(doc);
        Some(summary)
    }

    /// Drop the summary's properties from `doc`, if any are there.
    pub(super) fn remove(doc: &mut CadDocument) {
        doc.summary_info
            .custom_properties
            .retain(|(tag, _)| !tag.starts_with(SUMMARY_PROPERTY_PREFIX));
    }

    /// The headline, to the log: what a reader without a command line -- the
    /// headless export, a script -- gets instead of the editor's three lines.
    pub fn log(&self, path: &Path) {
        log::info!(
            "{}: P&ID import: {} entities from {} decoded records; {} source records not drawn; {} entities on {} authored sheet layers ({} unresolved); {} sheet layer names, {} off; {} cached / {} library bodies{}{}",
            path.display(),
            self.drawn,
            self.decoded,
            self.missing,
            self.layered_entities,
            self.sheet_layers,
            self.unresolved_sheet_layers,
            self.sheet_layer_names,
            self.sheet_layers_off,
            self.cache_bodies,
            self.library_bodies,
            if self.style_tables_failed {
                "; the style table did not read"
            } else {
                ""
            },
            if self.unit.is_assumed() {
                "; no coordinate unit stated, the metre was assumed"
            } else {
                ""
            },
        );
    }

    /// The counted fields, by the keys they are stored under.
    pub(super) fn counts(&self) -> [(&'static str, usize); 14] {
        [
            ("drawn", self.drawn),
            ("decoded", self.decoded),
            ("missing", self.missing),
            ("sheet_layers", self.sheet_layers),
            ("layered_entities", self.layered_entities),
            ("unresolved_sheet_layers", self.unresolved_sheet_layers),
            ("sheet_layer_names", self.sheet_layer_names),
            ("sheet_layers_off", self.sheet_layers_off),
            ("driving_dimensions", self.driving_dimensions),
            ("template_bodies", self.template_bodies),
            ("parametric_placements", self.parametric_placements),
            ("cache_bodies", self.cache_bodies),
            ("library_bodies", self.library_bodies),
            ("hidden_strokes_skipped", self.hidden_strokes_skipped),
        ]
    }

    /// The counted field stored under `key`, for reading a record back.
    pub(super) fn count_mut(&mut self, key: &str) -> Option<&mut usize> {
        Some(match key {
            "drawn" => &mut self.drawn,
            "decoded" => &mut self.decoded,
            "missing" => &mut self.missing,
            "sheet_layers" => &mut self.sheet_layers,
            "layered_entities" => &mut self.layered_entities,
            "unresolved_sheet_layers" => &mut self.unresolved_sheet_layers,
            "sheet_layer_names" => &mut self.sheet_layer_names,
            "sheet_layers_off" => &mut self.sheet_layers_off,
            "driving_dimensions" => &mut self.driving_dimensions,
            "template_bodies" => &mut self.template_bodies,
            "parametric_placements" => &mut self.parametric_placements,
            "cache_bodies" => &mut self.cache_bodies,
            "library_bodies" => &mut self.library_bodies,
            "hidden_strokes_skipped" => &mut self.hidden_strokes_skipped,
            _ => return None,
        })
    }

    /// Every count zero, no library, the metre assumed: what a record is
    /// read into.
    pub(super) fn empty() -> Self {
        Self {
            drawn: 0,
            decoded: 0,
            missing: 0,
            style_tables_failed: false,
            sheet_layers: 0,
            layered_entities: 0,
            unresolved_sheet_layers: 0,
            sheet_layer_names: 0,
            sheet_layers_off: 0,
            driving_dimensions: 0,
            template_bodies: 0,
            parametric_placements: 0,
            cache_bodies: 0,
            library_bodies: 0,
            hidden_strokes_skipped: 0,
            symbol_library: Vec::new(),
            unit: ImportUnit::AssumedMetre,
        }
    }
}

/// Say what the import could not draw, in the log rather than on the sheet.
///
/// The gaps a reader hits in practice are silent otherwise: evidence the
/// parser could not decode looks like a sparse drawing, an unreachable symbol
/// library looks like a sheet full of dots, and lettering the drawing gave no
/// usable height for looks like lettering the drawing sized at 2.5mm. All
/// three are recoverable -- the second by pointing [`SYMBOL_LIBRARY_ENV`] at a
/// local copy -- but only if the import says so.
pub(super) fn report_import(
    path: &Path,
    geometry: &pid_parse::NormalizedPidGeometry,
    library: Option<&SymbolLibrary>,
    drawn: usize,
    lettering_on_fallback: usize,
    symbol_bodies: SymbolBodies,
    sheet_layer_distribution: &BTreeMap<(String, u32, Option<String>), usize>,
) {
    // Where the symbol bodies came from, and how many cached strokes the
    // file itself switches off were left out (plan 2026-09-19, P-D2).
    // Information rather than a warning: by default the cached body is the
    // one SmartPlant placed, and the strokes left out are ones its screen
    // does not show either.
    if symbol_bodies.cache + symbol_bodies.library > 0 {
        log::info!(
            "{}: {} symbol placement(s) drew the body the drawing caches for them and {} the library's; {} cached stroke(s) on switched-off symbol layers were left undrawn",
            path.display(),
            symbol_bodies.cache,
            symbol_bodies.library,
            symbol_bodies.hidden_strokes_skipped
        );
    }
    for warning in &geometry.warnings {
        log::debug!("{}: {warning}", path.display());
    }

    for ((storage, oid, name), count) in sheet_layer_distribution {
        match name {
            Some(name) => log::info!(
                "{}: authored sheet layer storage={} oid={} name={:?} drawn_entities={}",
                path.display(), storage, oid, name, count
            ),
            None => log::warn!(
                "{}: authored sheet layer storage={} oid={} has no decoded name; {} entity/entities keep their synthetic PID layer",
                path.display(), storage, oid, count
            ),
        }
    }

    // Content the vendor's own graphic predicate says should draw, which
    // pid-parse has no decoder for, is a named warning rather than a debug
    // line: an igDimension / igBalloon / igLeader class silently vanishing
    // is exactly the failure a reader cannot notice on their own. Kept
    // separate from the inferred-or-probe-only aggregate below so the drop
    // is called by name (Phase 38 S2).
    for dropped in &geometry.dropped_graphic_records {
        let class_name = dropped
            .rad_class_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        log::warn!(
            "{}: {} record(s) of graphic type 0x{:04X}{} in {} have no decoder; that content is missing from the drawing",
            path.display(),
            dropped.count,
            dropped.type_code,
            class_name,
            dropped.stream_path
        );
    }

    // The other way content goes missing, and on the reference corpus the
    // larger one: a record whose type pid-parse does decode, in a shape its
    // decoder refuses. Refusing beats guessing -- a decoder that read an
    // unknown framing would draw fiction -- but the reader still has a
    // drawing with strokes missing, so it is named the same way. Worded
    // apart from the no-decoder case because the two ask for different work.
    for refused in &geometry.refused_graphic_records {
        let class_name = refused
            .rad_class_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        log::warn!(
            "{}: {} record(s) of graphic type 0x{:04X}{} in {} are a shape pid-parse's decoder for that type refuses; that content is missing from the drawing",
            path.display(),
            refused.count,
            refused.type_code,
            class_name,
            refused.stream_path
        );
    }

    // The headline number a thin-looking sheet is read against: how much of
    // the file reached the drawing, and how much the parser saw but could not
    // place. Counting the evidence rather than the entities keeps it
    // comparable with `pid-parse`'s own coverage reporting, so the two agree
    // on how much of the file is understood.
    let mut placed = 0usize;
    let mut undrawn = 0usize;
    for entity in &geometry.entities {
        match entity.confidence {
            PidGeometryConfidence::Decoded => placed += 1,
            PidGeometryConfidence::Inferred | PidGeometryConfidence::ProbeOnly => undrawn += 1,
        }
    }
    log::info!(
        "{}: drew {drawn} entit(ies) from {placed} decoded record(s); {undrawn} further evidence item(s) are inferred or probe-only and are hidden or dropped",
        path.display()
    );

    // Lettering the drawing states no usable height for. The style chain
    // itself resolves -- across the reference corpus all 184 text records
    // reach a character style -- but 25 of them reach one storing 0.254mm
    // (0.01"), which no drawing letters at, so `style_link` refuses it and
    // this importer keeps `TEXT_HEIGHT_MM`. Measured in `pid-parse`'s
    // `docs/analysis/2026-08-10-text-height-residue-is-one-sentinel-not-version-2.md`.
    // Silent, this reads as lettering the drawing sized at 2.5mm.
    if lettering_on_fallback > 0 {
        log::warn!(
            "{}: {lettering_on_fallback} text record(s) state no usable character height; they are lettered at the {TEXT_HEIGHT_MM}mm ISO 3098 fallback rather than a height read off the drawing",
            path.display()
        );
    }

    // A marker dot is a placement neither body reached: the drawing caches
    // nothing drawable for it and the library has no `.sym` for it (or was
    // not found). On the corpus every placement has a cached body, so this
    // is the line that says why a dot appeared when one does.
    if symbol_bodies.markers > 0 {
        log::warn!(
            "{}: {} symbol placement(s) drew as a marker dot, the drawing caching no drawable body for them and the library having none{}",
            path.display(),
            symbol_bodies.markers,
            if library.is_none() {
                format!(
                    ". Set {SYMBOL_LIBRARY_ENV} to a local copy of the project's reference-data Symbols share"
                )
            } else {
                String::new()
            }
        );
    }
    // The library is the stand-in for a placement the drawing caches no body
    // for, so its absence is information: the bodies on screen are the
    // drawing's own.
    let Some(library) = library else {
        log::info!(
            "{}: no symbol library found; placements draw the body the drawing caches for them, and a marker where it caches none",
            path.display()
        );
        return;
    };
    let missing = library.missing();
    if missing.is_empty() {
        return;
    }
    log::info!(
        "{}: {} of {} symbol(s) looked up are not in the library at {:?}; the drawing's own cached body stands in where it has one. First missing: {}",
        path.display(),
        missing.len(),
        library.lookups(),
        library.roots(),
        missing
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    );
}
