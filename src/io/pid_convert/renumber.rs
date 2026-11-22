//! Multi-sheet batch P&ID tag & pipeline renumbering engine with pre-flight collision
//! blocking and paired single/2-line canvas text synchronization
//! (`PIDRENUM`, `L2-3` / `L2-4` / `L3-6` / `L6-2` / `#159`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::external_props::normalize_lookup_key;
use super::promote::{
    has_native_pid_semantics, promote_semantics_to_author, promote_sheet_to_author,
    resolve_owning_author_handles, sync_author_after_edit, sync_paired_canvas_tag_texts,
};
use super::ConvertOptions;

/// Canonical 6-column header order shared by CSV and TSV renumber cross-reference schedules.
pub const RENUMBER_CSV_HEADERS: [&str; 6] = [
    "DrawingCode",
    "EntityKind",
    "SPPIDClass",
    "OldTagOrLine",
    "NewTagOrLine",
    "HandleHex",
];

/// One planned or applied old-to-new tag/pipeline mapping row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidRenumberItem {
    pub drawing_name: String,
    pub entity_kind: String,
    pub sppid_class: String,
    pub old_tag: String,
    pub new_tag: String,
    pub handle_hex: String,
}

/// One pre-flight duplicate-tag collision blocking non-forced renumbering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidRenumberConflict {
    pub drawing_name: String,
    pub handle_hex: String,
    pub old_tag: String,
    pub proposed_tag: String,
    pub sppid_class: String,
    pub reason: String,
}

/// Summary statistics for `PIDRENUM`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidRenumberSummary {
    pub from_pattern: String,
    pub to_pattern: String,
    pub dry_run: bool,
    pub applied: bool,
    pub sheets_processed: usize,
    pub symbols_renamed: usize,
    pub pipelines_renamed: usize,
    pub canvas_texts_synced: usize,
    pub conflict_count: usize,
}

/// Complete `PIDRENUM` plan / execution report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidRenumberReport {
    pub summary: PidRenumberSummary,
    pub items: Vec<PidRenumberItem>,
    pub conflicts: Vec<PidRenumberConflict>,
}

/// Options controlling `plan_or_apply_sheet_renumber`.
#[derive(Debug, Clone)]
pub struct RenumberOptions {
    pub from_pattern: String,
    pub to_pattern: String,
    pub dry_run: bool,
    pub force: bool,
}

fn replace_case_insensitive(input: &str, from_pat: &str, to_pat: &str) -> Option<String> {
    if from_pat.is_empty() {
        return None;
    }
    let in_up = input.to_ascii_uppercase();
    let pat_up = from_pat.to_ascii_uppercase();
    if !in_up.contains(&pat_up) {
        return None;
    }

    let mut out = String::with_capacity(input.len() + to_pat.len());
    let mut cursor = 0usize;
    let pat_len = pat_up.len();
    while let Some(rel) = in_up[cursor..].find(&pat_up) {
        let match_start = cursor + rel;
        let match_end = match_start + pat_len;
        if !input.is_char_boundary(match_start) || !input.is_char_boundary(match_end) {
            return None;
        }
        out.push_str(&input[cursor..match_start]);
        out.push_str(to_pat);
        cursor = match_end;
    }
    out.push_str(&input[cursor..]);
    (out != input).then_some(out)
}

fn ensure_sheet_authored(doc: &mut Document) {
    let has_author = doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some());
    if has_author {
        return;
    }
    if has_native_pid_semantics(doc) {
        let _ = promote_semantics_to_author(doc, None);
    } else {
        let _ = promote_sheet_to_author(doc, &ConvertOptions::default());
    }
}

struct SymPlan {
    handle: Handle,
    group_cls: String,
    cur_tag: String,
    final_tag: String,
    changed: bool,
}

/// Outcome returned by `plan_or_apply_sheet_renumber` for a single sheet.
#[derive(Debug, Clone, Default)]
pub struct SheetRenumberOutcome {
    pub items: Vec<PidRenumberItem>,
    pub conflicts: Vec<PidRenumberConflict>,
    pub pipelines_renamed: usize,
    pub modified_handles: Vec<Handle>,
}

/// Plan or apply batch tag/pipeline renumbering on a single `Document` (`#159`).
///
/// When `opts.dry_run` is `true` or conflicts occur (without `opts.force`), `doc` is NOT mutated.
pub fn plan_or_apply_sheet_renumber(
    drawing_name: &str,
    doc: &mut Document,
    selected_filter: Option<&[Handle]>,
    opts: &RenumberOptions,
) -> SheetRenumberOutcome {
    let already_authored = doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some());

    // Borrow `doc` directly when authored; otherwise plan on a scratch clone so dry-run/conflict leaves `doc` untouched
    let scratch_doc: Document;
    let plan_doc: &Document = if already_authored {
        doc
    } else {
        let mut cloned = doc.clone();
        ensure_sheet_authored(&mut cloned);
        scratch_doc = cloned;
        &scratch_doc
    };

    let scope_owners: Option<BTreeSet<Handle>> = selected_filter.map(|sel| {
        let mut s: BTreeSet<Handle> = resolve_owning_author_handles(plan_doc, sel)
            .into_iter()
            .collect();
        for &h in sel {
            s.insert(h);
        }
        s
    });

    let mut sym_plans: Vec<SymPlan> = Vec::new();
    for ent in plan_doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(plan_doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            continue;
        }
        if auth.tag.trim().is_empty() {
            continue;
        }
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();
        let cls = pmap
            .get("_sppid_class")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&auth.type_code)
            .to_string();
        let in_scope = scope_owners.as_ref().is_none_or(|set| set.contains(&h));
        let cur_tag = auth.tag.trim().to_string();
        let replaced = if in_scope {
            replace_case_insensitive(&cur_tag, &opts.from_pattern, &opts.to_pattern)
        } else {
            None
        };
        let (final_tag, changed) = match replaced {
            Some(nt) => (nt, true),
            None => (cur_tag.clone(), false),
        };
        sym_plans.push(SymPlan {
            handle: h,
            group_cls: cls,
            cur_tag,
            final_tag,
            changed,
        });
    }

    let mut final_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for sp in &sym_plans {
        let nk = normalize_lookup_key(&sp.final_tag);
        if !nk.is_empty() {
            *final_counts.entry((sp.group_cls.clone(), nk)).or_default() += 1;
        }
    }

    let mut items = Vec::new();
    let mut conflicts = Vec::new();
    for sp in &sym_plans {
        if !sp.changed {
            continue;
        }
        let nk = normalize_lookup_key(&sp.final_tag);
        let occ = final_counts
            .get(&(sp.group_cls.clone(), nk))
            .copied()
            .unwrap_or(0);
        if occ > 1 {
            conflicts.push(PidRenumberConflict {
                drawing_name: drawing_name.to_string(),
                handle_hex: format!("{:X}", sp.handle.value()),
                old_tag: sp.cur_tag.clone(),
                proposed_tag: sp.final_tag.clone(),
                sppid_class: sp.group_cls.clone(),
                reason: format!(
                    "目标位号 {} 在同类别 {} 中将产生重复冲突（共 {} 处）",
                    sp.final_tag, sp.group_cls, occ
                ),
            });
        }
        items.push(PidRenumberItem {
            drawing_name: drawing_name.to_string(),
            entity_kind: "symbol".to_string(),
            sppid_class: sp.group_cls.clone(),
            old_tag: sp.cur_tag.clone(),
            new_tag: sp.final_tag.clone(),
            handle_hex: format!("{:X}", sp.handle.value()),
        });
    }

    // Plan pipeline label renames (whole-sheet when selected_filter is None, or selected text/pipe handles)
    let mut pipe_seen_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut pipe_text_updates: Vec<(Handle, String)> = Vec::new();
    for ent in plan_doc.model_space_entities() {
        let eh = ent.common().handle;
        let in_scope = scope_owners.as_ref().is_none_or(|set| set.contains(&eh));
        if !in_scope {
            continue;
        }
        let raw_val = match ent {
            EntityType::Text(t) => Some(t.value.as_str()),
            EntityType::MText(mt) => Some(mt.value.as_str()),
            _ => None,
        };
        let Some(val) = raw_val else {
            continue;
        };
        if parse_sppid_line_number(val.trim()).is_none() {
            continue;
        }
        let Some(new_val) = replace_case_insensitive(val, &opts.from_pattern, &opts.to_pattern)
        else {
            continue;
        };
        pipe_text_updates.push((eh, new_val.clone()));
        let old_clean = val.trim().trim_end_matches('-').to_string();
        let new_clean = new_val.trim().trim_end_matches('-').to_string();
        if pipe_seen_pairs.insert((old_clean.clone(), new_clean.clone())) {
            items.push(PidRenumberItem {
                drawing_name: drawing_name.to_string(),
                entity_kind: "pipeline".to_string(),
                sppid_class: "PIDPipeline".to_string(),
                old_tag: old_clean,
                new_tag: new_clean,
                handle_hex: format!("{:X}", eh.value()),
            });
        }
    }

    let topo = TopologyTable::load(plan_doc);
    if scope_owners.is_none() {
        for pl in &topo.pipe_lines {
            if let Some(nt) =
                replace_case_insensitive(&pl.tag, &opts.from_pattern, &opts.to_pattern)
            {
                let oc = pl.tag.trim_end_matches('-').to_string();
                let nc = nt.trim_end_matches('-').to_string();
                if pipe_seen_pairs.insert((oc.clone(), nc.clone())) {
                    items.push(PidRenumberItem {
                        drawing_name: drawing_name.to_string(),
                        entity_kind: "pipeline".to_string(),
                        sppid_class: "PIDPipeline".to_string(),
                        old_tag: oc,
                        new_tag: nc,
                        handle_hex: pl.handle.clone(),
                    });
                }
            }
        }
    }

    let pipelines_renamed = pipe_seen_pairs.len();

    if opts.dry_run || (!conflicts.is_empty() && !opts.force) || items.is_empty() {
        return SheetRenumberOutcome {
            items,
            conflicts,
            pipelines_renamed,
            modified_handles: Vec::new(),
        };
    }

    // Apply mutations in-place on `doc` so active undo recorders capture every entity/XRecord change
    if !already_authored {
        ensure_sheet_authored(doc);
    }
    let mut modified_handles: BTreeSet<Handle> = BTreeSet::new();
    for sp in &sym_plans {
        if !sp.changed {
            continue;
        }
        let Some(mut auth) = PidAuthor::load(doc, sp.handle) else {
            continue;
        };
        auth.tag = sp.final_tag.clone();
        auth.tag_source = "manual".to_string();
        auth.store(doc, sp.handle);
        if let Some(EntityType::Insert(ins)) = doc.get_entity_mut(sp.handle) {
            for attr in &mut ins.attributes {
                if attr.tag.eq_ignore_ascii_case("PID_TAG") {
                    attr.value = sp.final_tag.clone();
                }
            }
        }
        modified_handles.insert(sp.handle);
    }

    // Update pipeline canvas texts
    for (th, new_val) in pipe_text_updates {
        match doc.get_entity_mut(th) {
            Some(EntityType::Text(t)) => {
                t.value = new_val;
                modified_handles.insert(th);
            }
            Some(EntityType::MText(mt)) => {
                mt.value = new_val;
                modified_handles.insert(th);
            }
            _ => {}
        }
    }

    // Update `_line_label` on pipe segments and `TopologyTable` when whole-sheet renumbering
    if scope_owners.is_none() {
        let all_hs: Vec<Handle> = doc
            .model_space_entities()
            .map(|e| e.common().handle)
            .collect();
        for h in all_hs {
            let Some(mut auth) = PidAuthor::load(doc, h) else {
                continue;
            };
            if auth.topology != ROLE_PIPE_SEGMENT && auth.type_code != TYPE_PIPE_SEGMENT {
                continue;
            }
            let Ok(mut pmap) = serde_json::from_str::<serde_json::Map<String, Value>>(&auth.props)
            else {
                continue;
            };
            let Some(cur_l) = pmap
                .get("_line_label")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            if let Some(nl) = replace_case_insensitive(&cur_l, &opts.from_pattern, &opts.to_pattern)
            {
                pmap.insert("_line_label".to_string(), Value::String(nl));
                auth.props = Value::Object(pmap).to_string();
                auth.store(doc, h);
                modified_handles.insert(h);
            }
        }

        let mut table = TopologyTable::load(doc);
        let mut topo_changed = false;
        for pl in &mut table.pipe_lines {
            if let Some(nt) =
                replace_case_insensitive(&pl.tag, &opts.from_pattern, &opts.to_pattern)
            {
                pl.tag = nt;
                topo_changed = true;
            }
        }
        for pr in &mut table.pipe_runs {
            if let Some(nt) =
                replace_case_insensitive(&pr.tag, &opts.from_pattern, &opts.to_pattern)
            {
                pr.tag = nt;
                topo_changed = true;
            }
            if let Some(nlt) =
                replace_case_insensitive(&pr.line_tag, &opts.from_pattern, &opts.to_pattern)
            {
                pr.line_tag = nlt;
                topo_changed = true;
            }
        }
        for ps in &mut table.pipe_segments {
            if let Some(nrt) =
                replace_case_insensitive(&ps.run_tag, &opts.from_pattern, &opts.to_pattern)
            {
                ps.run_tag = nrt;
                topo_changed = true;
            }
        }
        if topo_changed {
            table.store(doc);
        }
    }

    let _ = sync_author_after_edit(doc);
    let synced_th = sync_paired_canvas_tag_texts(doc);
    for th in synced_th {
        modified_handles.insert(th);
    }

    SheetRenumberOutcome {
        items,
        conflicts,
        pipelines_renamed,
        modified_handles: modified_handles.into_iter().collect(),
    }
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidRenumberReport` as delimited old-to-new cross-reference schedule text with UTF-8 BOM.
pub fn format_pid_renumber_delimited(report: &PidRenumberReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = RENUMBER_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for it in &report.items {
        let cells = [
            it.drawing_name.as_str(),
            it.entity_kind.as_str(),
            it.sppid_class.as_str(),
            it.old_tag.as_str(),
            it.new_tag.as_str(),
            it.handle_hex.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidRenumberReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#159`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_renumber_file(report: &PidRenumberReport, out_path: &Path) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let ext = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("json")
        .to_ascii_lowercase();
    let payload = match ext.as_str() {
        "json" => format!(
            "{}\n",
            serde_json::to_string_pretty(report)
                .map_err(|e| format!("serialize renumber JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_renumber_delimited(report, '\t'),
        "csv" => format_pid_renumber_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported renumber report extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}
