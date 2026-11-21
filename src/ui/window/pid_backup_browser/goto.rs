//! Going to a node by the labels on the way to it.
//!
//! `PIDBACKUPOPEN Select Plant groups > SQPlant > 00 > D07` names a node the
//! way the tree shows it: one label per level below the backup itself. The
//! walk unfolds each level as a click on its arrow would and selects the
//! last, so the panel ends in exactly the state the same clicks leave it in.
//! A script, or a client on the automation channel, can therefore show any
//! node without a pointer.
//!
//! A level's children may not be known yet: a drawing reads its items the
//! first time it is unfolded. The walk then stops with the read as its
//! effect and goes on from [`step`] when the items arrive.
//!
//! A segment matches the first child whose label it equals, then the first
//! whose label it equals ignoring case, then the first it names -- a row
//! adds a count or a remark to the name of its node (`SQPlantpid (82)`,
//! `T_DRAWING — 53 row(s)`), and the name alone is enough -- then the first
//! whose label starts with it ignoring case (`Instr` finds `Instrument
//! (15)`). `#3` is the third child, whatever it is called.

use super::{update, visible_rows, Effect, NodeId, PidBackupBrowser, PidBackupMsg, Row};
use crate::{t, tf};

/// The separator between the labels of a path.
pub const SEPARATOR: char = '>';

/// A walk that is under way.
#[derive(Debug, Clone)]
pub struct Goto {
    /// The node the next segment is looked for under.
    under: NodeId,
    /// The labels still to find, the next one first.
    remaining: Vec<String>,
    /// The labels found so far, as the tree spells them.
    found: Vec<String>,
}

/// How a walk ended: the labels of the node it selected, or why it stopped.
pub type Outcome = Result<String, String>;

/// Split `path` into its segments. Empty segments are dropped, so a path may
/// start or end with the separator.
pub fn segments(path: &str) -> Vec<String> {
    path.split(SEPARATOR)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

/// Begin walking to `path`, replacing a walk that was under way.
pub(super) fn start(state: &mut PidBackupBrowser, path: &str) -> Effect {
    state.goto = None;
    if state.index().is_none() {
        state.goto_outcome = Some(Err(t!("No backup is open.").into_owned()));
        return Effect::None;
    }
    state.goto = Some(Goto {
        under: NodeId::Root,
        remaining: segments(path),
        found: Vec::new(),
    });
    step(state)
}

/// Take the walk as far as the tree is known. Called again whenever something
/// the walk may have been waiting for arrived.
pub(super) fn step(state: &mut PidBackupBrowser) -> Effect {
    // Each turn finds one segment or unfolds one node; a walk is as long as
    // its path, so the bound only guards against a tree that never settles.
    for _ in 0..4096 {
        let Some(goto) = state.goto.clone() else {
            return Effect::None;
        };
        let Some(segment) = goto.remaining.first().cloned() else {
            // Nothing left to find: `under` is the node.
            state.goto = None;
            state.goto_outcome = Some(Ok(reached(state, &goto)));
            return update(state, PidBackupMsg::Select(goto.under));
        };
        let rows = visible_rows(state);
        let Some(at) = rows.iter().position(|row| row.id == goto.under) else {
            return fail(state, &goto, &segment);
        };
        let parent = &rows[at];
        if !parent.expanded {
            if !parent.expandable {
                return fail(state, &goto, &segment);
            }
            let effect = update(state, PidBackupMsg::Toggle(goto.under.clone()));
            if effect != Effect::None {
                // The node is reading what it holds: go on when it lands.
                return effect;
            }
            continue;
        }
        let children: Vec<&Row> = rows[at + 1..]
            .iter()
            .take_while(|row| row.depth > parent.depth)
            .filter(|row| row.depth == parent.depth + 1)
            .collect();
        match find(&children, &segment) {
            Some(child) => {
                let goto = state.goto.as_mut().expect("the walk is under way");
                goto.under = child.id.clone();
                goto.found.push(child.label.clone());
                goto.remaining.remove(0);
            }
            // An open node with no such child. While its content is still
            // being read the child may yet appear.
            None if is_reading(state, &goto.under) => return Effect::None,
            None => return fail(state, &goto, &segment),
        }
    }
    state.goto = None;
    Effect::None
}

/// How the last walk ended, once: the command line reports it.
pub fn take_outcome(state: &mut PidBackupBrowser) -> Option<Outcome> {
    state.goto_outcome.take()
}

/// The child `segment` names, by the rules in the module documentation.
fn find<'a>(children: &[&'a Row], segment: &str) -> Option<&'a Row> {
    if let Some(number) = segment.strip_prefix('#') {
        let number: usize = number.trim().parse().ok()?;
        return children.get(number.checked_sub(1)?).copied();
    }
    let lower = segment.to_lowercase();
    children
        .iter()
        .find(|row| row.label == segment)
        .or_else(|| {
            children
                .iter()
                .find(|row| row.label.to_lowercase() == lower)
        })
        .or_else(|| {
            children
                .iter()
                .find(|row| name_part(&row.label).to_lowercase() == lower)
        })
        .or_else(|| {
            children
                .iter()
                .find(|row| row.label.to_lowercase().starts_with(&lower))
        })
        .copied()
}

/// The part of a row's label that names its node: what stands before the
/// remark (`T_DRAWING — 53 row(s)`, `RefData~4~680 — Ref\rules.rul`) or
/// the count (`SQPlantpid (82)`) the row adds to it.
fn name_part(label: &str) -> &str {
    let label = label.split(" — ").next().unwrap_or(label);
    match label.rsplit_once(" (") {
        Some((name, count))
            if count.strip_suffix(')').is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
            }) =>
        {
            name
        }
        _ => label,
    }
}

/// Whether what `id` holds is on its way from a worker thread.
fn is_reading(state: &PidBackupBrowser, id: &NodeId) -> bool {
    match id {
        NodeId::Drawing(drawing) => {
            matches!(state.items.get(drawing), Some(super::ItemsState::Loading))
        }
        _ => state
            .section_data
            .iter()
            .any(|((_, node), load)| node == id && matches!(load, super::SectionLoad::Loading)),
    }
}

/// The node the walk got to, as the labels on the way to it; the backup's own
/// label while it is still at the root.
fn reached(state: &PidBackupBrowser, goto: &Goto) -> String {
    if goto.found.is_empty() {
        visible_rows(state)
            .first()
            .map(|row| row.label.clone())
            .unwrap_or_default()
    } else {
        goto.found.join(" > ")
    }
}

/// Stop the walk: `segment` was not found under the node it got to.
fn fail(state: &mut PidBackupBrowser, goto: &Goto, segment: &str) -> Effect {
    state.goto = None;
    let reached = reached(state, goto);
    state.goto_outcome = Some(Err(tf!(
        "there is no \"{}\" under \"{}\"",
        segment,
        reached
    )
    .into_owned()));
    Effect::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_backup_browse::DrawingItems;
    use crate::ui::window::pid_backup_browser::tests::{items_of, ready};
    use std::sync::Arc;

    fn selected_label(state: &PidBackupBrowser) -> Option<String> {
        let selected = state.selected.clone()?;
        visible_rows(state)
            .into_iter()
            .find(|row| row.id == selected)
            .map(|row| row.label)
    }

    #[test]
    fn a_path_is_split_at_the_separator_and_trimmed() {
        assert_eq!(
            segments(" Plant groups >SQPlant>  00 > "),
            ["Plant groups", "SQPlant", "00"]
        );
        assert!(segments("  ").is_empty());
        // A label keeps its own spaces and slashes.
        assert_eq!(segments("a b/c > d"), ["a b/c", "d"]);
    }

    #[test]
    fn a_walk_unfolds_every_level_and_selects_the_last() {
        let mut state = ready();
        let groups = t!("Plant groups").into_owned();
        let effect = start(&mut state, &format!("{groups} > SQPlant > 00 > 01"));
        assert_eq!(effect, Effect::None);
        assert_eq!(
            take_outcome(&mut state),
            Some(Ok(format!("{groups} > SQPlant > 00 > 01")))
        );
        assert_eq!(state.selected, Some(NodeId::Group(vec![0, 0, 0])));
        // Every level on the way is open, as the same clicks would leave it;
        // the node itself is selected, not unfolded.
        for id in [
            NodeId::Groups,
            NodeId::Group(vec![0]),
            NodeId::Group(vec![0, 0]),
        ] {
            assert!(state.expanded.contains(&id), "{id:?}");
        }
        assert!(!state.expanded.contains(&NodeId::Group(vec![0, 0, 0])));
        // Reported once.
        assert_eq!(take_outcome(&mut state), None);
    }

    #[test]
    fn a_segment_matches_by_case_then_by_prefix_then_by_number() {
        let mut state = ready();
        let groups = t!("Plant groups").into_owned();
        // `sqplant` differs in case only; `0` is the start of `00`; `#2` is
        // the second child of `00`, the drawing A-101 (its group comes first).
        start(&mut state, &format!("{groups} > sqplant > 0 > #2"));
        assert_eq!(
            take_outcome(&mut state),
            Some(Ok(format!("{groups} > SQPlant > 00 > A-101")))
        );
        assert_eq!(state.selected, Some(NodeId::Drawing(0)));
        // An exact label wins over one it is the start of.
        let mut state = ready();
        start(&mut state, &format!("{groups} > SQPlant > 00 > 01"));
        assert_eq!(selected_label(&state).as_deref(), Some("01"));
    }

    #[test]
    fn a_segment_names_a_row_without_its_count_or_its_remark() {
        let row = |label: &str| Row {
            id: NodeId::Files,
            depth: 2,
            label: label.to_string(),
            expandable: false,
            expanded: false,
            disabled: false,
        };
        let rows = [
            row("SQPlantpidd (25)"),
            row("SQPlantpid (82)"),
            row("RefData~4~680 — Ref\\rules.rul"),
            row("T_DRAWING — 53 row(s)"),
            row("Group (old) (3)"),
        ];
        let children: Vec<&Row> = rows.iter().collect();
        let found = |segment: &str| find(&children, segment).map(|row| row.label.as_str());
        // The name alone, not the longer name it is the start of.
        assert_eq!(found("SQPlantpid"), Some("SQPlantpid (82)"));
        assert_eq!(found("sqplantpidd"), Some("SQPlantpidd (25)"));
        assert_eq!(
            found("RefData~4~680"),
            Some("RefData~4~680 — Ref\\rules.rul")
        );
        assert_eq!(found("t_drawing"), Some("T_DRAWING — 53 row(s)"));
        // Only a count in brackets is taken off.
        assert_eq!(found("Group (old)"), Some("Group (old) (3)"));
        assert_eq!(name_part("Unit (north)"), "Unit (north)");
        assert_eq!(name_part("Empty ()"), "Empty ()");
        // The start of a label still finds it, after the names.
        assert_eq!(found("SQPl"), Some("SQPlantpidd (25)"));
        assert_eq!(found("nothing"), None);
    }

    #[test]
    fn a_walk_into_a_drawing_waits_for_its_items() {
        let mut state = ready();
        let groups = t!("Plant groups").into_owned();
        let effect = start(
            &mut state,
            &format!("{groups} > SQPlant > 00 > A-101 > Instrument > #1"),
        );
        // The drawing's first unfold reads its items: the walk hands the read
        // over and has not ended yet.
        assert_eq!(
            effect,
            Effect::LoadItems(0, std::path::PathBuf::from("D:/tmp/restore/00/00/a.pid"))
        );
        assert_eq!(take_outcome(&mut state), None);
        assert_eq!(state.selected, None);

        // The items land, and the walk goes on by itself.
        let effect = update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(items_of("UID-1", "Instrument")))),
        );
        assert_eq!(effect, Effect::None);
        assert_eq!(state.selected, Some(NodeId::Item(0, 0, 0)));
        assert!(take_outcome(&mut state).is_some_and(|outcome| outcome.is_ok()));
    }

    #[test]
    fn a_walk_that_finds_no_such_child_says_where_it_stopped() {
        let mut state = ready();
        let groups = t!("Plant groups").into_owned();
        start(&mut state, &format!("{groups} > SQPlant > 99"));
        let outcome = take_outcome(&mut state).expect("the walk ended");
        let error = outcome.expect_err("there is no group 99");
        assert!(error.contains("99") && error.contains("SQPlant"), "{error}");
        // What it did find stays open; nothing is selected by a failed walk.
        assert!(state.expanded.contains(&NodeId::Group(vec![0])));
        assert_eq!(state.selected, None);

        // A drawing whose items hold no such type: the read has landed, so
        // the walk does not wait any longer.
        let mut state = ready();
        start(
            &mut state,
            &format!("{groups} > SQPlant > 00 > A-101 > Vessel"),
        );
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(DrawingItems::default()))),
        );
        let error = take_outcome(&mut state)
            .expect("the walk ended")
            .expect_err("the drawing holds no vessel");
        assert!(
            error.contains("Vessel") && error.contains("A-101"),
            "{error}"
        );
    }

    #[test]
    fn under_a_search_a_walk_reaches_what_the_search_keeps_and_nothing_else() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Filter("valve".to_string()));
        let reference = t!("Reference data").into_owned();
        let effect = start(&mut state, &format!("{reference} > Symbols > valve.svg"));
        assert_eq!(
            take_outcome(&mut state),
            Some(Ok(format!("{reference} > Symbols > valve.svg")))
        );
        let file = NodeId::RefFile(vec![0], 0);
        assert_eq!(state.selected, Some(file.clone()));
        // The walk ends as a click on the file does.
        let mut clicked = ready();
        assert_eq!(
            effect,
            update(&mut clicked, PidBackupMsg::Select(file.clone()))
        );
        // The search holds the levels open: the walk unfolded none of them.
        assert!(!state.expanded.contains(&NodeId::Ref));
        assert!(!state.expanded.contains(&NodeId::RefDir(vec![0])));

        // A file the search hides is not there to walk to, and the
        // selection stays.
        start(&mut state, &format!("{reference} > Symbols > pump.svg"));
        let error = take_outcome(&mut state)
            .expect("the walk ended")
            .expect_err("the search hides pump.svg");
        assert!(
            error.contains("pump.svg") && error.contains("Symbols"),
            "{error}"
        );
        assert_eq!(state.selected, Some(file));

        // Nor is the reference data itself, when the search names none of
        // its files.
        update(&mut state, PidBackupMsg::Filter("b-2".to_string()));
        start(&mut state, &reference);
        let error = take_outcome(&mut state)
            .expect("the walk ended")
            .expect_err("no reference file is called b-2");
        assert!(error.contains(&reference), "{error}");
    }

    #[test]
    fn an_empty_path_selects_the_backup_and_an_idle_panel_refuses() {
        let mut state = ready();
        start(&mut state, "");
        assert_eq!(state.selected, Some(NodeId::Root));
        assert_eq!(
            take_outcome(&mut state),
            Some(Ok("Plant backup".to_string()))
        );

        let mut idle = PidBackupBrowser::default();
        assert_eq!(start(&mut idle, "anything"), Effect::None);
        assert_eq!(
            take_outcome(&mut idle),
            Some(Err(t!("No backup is open.").into_owned()))
        );
    }
}
