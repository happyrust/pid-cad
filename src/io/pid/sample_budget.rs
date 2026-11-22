//! What importing a real sheet costs (plan 2026-11-19-pid-symbol-block,
//! SB-9).
//!
//! One test, run only when asked for. It times three imports of the sheet
//! the plan measured on -- 592 symbol placements among some ten thousand
//! entities -- against what they took before symbols were filed as blocks.
//! What the import makes of that sheet is pinned by tests that run by
//! default, in a test program of their own: `tests/pid_import.rs`,
//! `sample_sheet`.
//!
//! The sheet is read where it lies in the plant test data, a folder beside
//! this repository, with nothing published beside it. The symbol library
//! of that test data is in reach of it there, as it was when the baseline
//! was taken, and the sheet takes no body from it. Where the folder is
//! missing, or `PID_SYMBOL_LIBRARY` names libraries of its own, the test
//! says so and passes -- which proves nothing: the budget is held only on
//! a machine that has the drawing.

use std::time::{Duration, Instant};

use super::*;

/// Symbol placements the sample draws. Each timed import is checked to have
/// filed them; `tests/pid_import.rs` is where this number and the next are
/// pinned.
const PLACEMENTS: usize = 592;

/// The block definitions the import files for them.
const DEFINITIONS: usize = 119;

/// Imports the budget is the median of (the convention of
/// `src/pid/perf_budget.rs`).
const RUNS: usize = 3;

/// What the median of three imports of the sample took before symbols were
/// filed as blocks, in milliseconds: the unit tests of `5171d197b`, built
/// as these are, on an AMD Ryzen 9 7950X under Windows 11 on 2026-11-20.
/// Taken turn and turn about with this branch's (`08d340055`), a process of
/// three imports each, over the twelve rounds in which nothing was being
/// built on the machine and no more than a fifth of it was in use: the
/// median of the twelve medians, which ran from 3,196 to 3,762. The
/// branch's was 3,488, from 3,332 to 3,862.
const BASELINE_MS: u128 = 3_406;

/// What the median import of the sample may take, in milliseconds: three
/// tenths over the baseline. It is a coarse screen, and not the tenth that
/// SPEC-pid-symbol-block §6, 11 allows. What else runs on the machine moves
/// a turn by more than a tenth -- of those twelve rounds, two turns of the
/// branch and one of the baseline itself were a tenth over -- so the tenth
/// is judged by timing two programs turn and turn about, as the baseline
/// was taken. This number lies above every turn of the twenty rounds timed
/// that morning, the eight that were not quiet among them: the slowest of
/// the baseline took 4,176, of the branch 4,273. It is the number of one
/// machine on one day. A build running beside the test can still put a
/// turn over it; and where the machine has grown quicker -- the day after,
/// the branch's turns at rest came to some 2,830 -- an import has to lose
/// more than three tenths before it shows. Read a red run against what the
/// machine was doing before reading it against the code.
const BUDGET_MS: u128 = (BASELINE_MS * 13).div_ceil(10);

/// Where the sample lies -- the sheet of the well sites L and M, in the
/// plant test data beside this repository -- or `None`, and a line saying
/// why, where it cannot be read the way the baseline was taken.
fn sample_path() -> Option<PathBuf> {
    if std::env::var_os(SYMBOL_LIBRARY_ENV).is_some() {
        eprintln!("skipped: {SYMBOL_LIBRARY_ENV} is set; the baseline was taken without it");
        return None;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pid-test-data")
        .join("backup-plant-data")
        .join("00")
        .join("00")
        .join("L M井场 注采阀组工艺及自控流程图.pid");
    if !path.is_file() {
        eprintln!("skipped: no sample drawing at {}", path.display());
        return None;
    }
    Some(path)
}

/// The median of three imports of the sample stays within the budget: the
/// coarse screen kept beside SPEC-pid-symbol-block §6, 11, not the tenth
/// that criterion allows.
#[test]
#[ignore = "times three imports of the sample against a budget taken on one machine; run alone with --ignored --nocapture"]
fn the_sample_imports_within_its_budget() {
    let Some(path) = sample_path() else {
        return;
    };
    let mut samples = Vec::with_capacity(RUNS);
    for run in 1..=RUNS {
        let started = Instant::now();
        let import = load_pid(&path).expect("the sample imports");
        let elapsed = started.elapsed();
        println!(
            "sample import {run}/{RUNS}: {} ms, {} references to {} definitions",
            elapsed.as_millis(),
            import.summary.symbol_blocks,
            import.summary.symbol_definitions
        );
        assert_eq!(
            (
                import.summary.symbol_blocks,
                import.summary.symbol_definitions
            ),
            (PLACEMENTS, DEFINITIONS),
            "run {run}"
        );
        samples.push(elapsed);
    }

    samples.sort();
    let median = samples[RUNS / 2];
    let all: Vec<u128> = samples.iter().map(Duration::as_millis).collect();
    println!(
        "sample import: median {} ms of {all:?}, budget {BUDGET_MS} ms",
        median.as_millis()
    );
    assert!(
        median.as_millis() <= BUDGET_MS,
        "over budget: median {} ms of {all:?}, budget {BUDGET_MS} ms",
        median.as_millis()
    );
}
