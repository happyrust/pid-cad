//! Headless `iced_test` verification of `backup_export_ui` (`UI-7` & `UI-8`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::{Point, Rectangle, Size};

use super::backup::SCREEN_BOUNDS_PREFIX;
use super::backup_export_ui::{self, ExportDialog, ExportUiMessage};
use super::*;
use crate::cli::GuiConfig;
use crate::io::pid_backup_browse::{
    BackupFile, BackupIndex, DrawingEntry, FileKind, ManifestSummary,
};
use crate::ui::window::pid_backup_browser::LoadState;

const WINDOW: Size = Size::new(1600.0, 1000.0);

fn app_on_backup() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.default_assoc_prompted = true;
    app.donation_prompt_version = env!("OCS_APP_VERSION").to_owned();
    let first = app.open_first_screen(&GuiConfig::default());
    app.drive_headless_task(first)
        .expect("first screen opens headlessly");
    let _ = app.update(AppMessage::WindowResized(WINDOW.width, WINDOW.height));
    assert!(app.show_pid_screen(PidScreen::Backup));
    app
}

fn headless() -> iced::Renderer {
    use iced_test::core::renderer;
    iced_test::futures::futures::executor::block_on(<iced::Renderer as renderer::Headless>::new(
        renderer::Settings::default(),
        std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
    ))
    .expect("headless renderer")
}

fn drawn(app: &OpenCADStudio) -> BTreeMap<String, Rectangle> {
    use iced_test::core::{mouse, renderer};
    use iced_test::runtime::user_interface::{Cache, UserInterface};
    crate::ui::wrap_bar::forget_bounds("");
    let mut renderer = headless();
    let mut ui = UserInterface::build(app.view_main(), WINDOW, Cache::default(), &mut renderer);
    ui.draw(
        &mut renderer,
        &app.active_theme,
        &renderer::Style {
            text_color: iced::Color::BLACK,
        },
        mouse::Cursor::Unavailable,
    );
    crate::ui::wrap_bar::recorded_bounds().into_iter().collect()
}

fn says(app: &OpenCADStudio, needle: &str) -> bool {
    use iced_test::core::widget;
    use iced_test::runtime::user_interface::{Cache, UserInterface};
    use iced_test::Selector;
    let mut renderer = headless();
    let mut ui = UserInterface::build(app.view_main(), WINDOW, Cache::default(), &mut renderer);
    let mut find = Selector::find(needle);
    ui.operate(&renderer, &mut widget::operation::black_box(&mut find));
    matches!(
        widget::Operation::finish(&find),
        widget::operation::Outcome::Some(Some(_))
    )
}

fn click_at(app: &mut OpenCADStudio, point: Point) {
    use iced_test::core::{mouse, shell, window as core_window};
    use iced_test::runtime::user_interface::{Cache, UserInterface};
    let mut renderer = headless();
    let mut cache = Cache::default();
    let events = [
        iced::Event::Mouse(mouse::Event::CursorMoved { position: point }),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ];
    for event in events {
        let mut messages = Vec::new();
        {
            let mut ui = UserInterface::build(app.view_main(), WINDOW, cache, &mut renderer);
            let _ = ui.update(
                &core_window::Headless,
                &shell::Waker::noop(),
                &[event],
                mouse::Cursor::Available(point),
                &mut renderer,
                &mut messages,
            );
            cache = ui.into_cache();
        }
        for message in messages {
            // Do NOT drive background export tasks here when testing running state;
            // synchronous UI state transitions happen inside `app.update(message)`.
            let _ = app.update(message);
        }
    }
}

fn send_export(app: &mut OpenCADStudio, msg: ExportUiMessage) {
    let _ = app.update(AppMessage::PidShell(Message::Backup(
        super::backup::Message::Export(msg),
    )));
}

fn temp_scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let dir = std::env::temp_dir().join(format!(
        "ocs-pid-backup-export-ui-{}-{}-{}",
        tag,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn pid_backup_export_ui_ui7_loose_pid_disables_publish_but_allows_pdf() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui7-loose");
    let pid_file = scratch.join("L01.pid");
    std::fs::write(&pid_file, b"loose-pid").unwrap();

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: scratch.clone(),
        restored: scratch.clone(),
        backup_root: scratch.clone(),
        read_only: true,
        manifest: None,
        drawings: vec![DrawingEntry {
            name: "L01".to_owned(),
            pid_path: Some(pid_file),
            group_path: "loose".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));

    assert!(backup_export_ui::publish_disabled_reason(&app)
        .is_some_and(|why| why.contains("Manifest.txt")));
    assert_eq!(backup_export_ui::pdf_disabled_reason(&app), None);

    let bounds = drawn(&app);
    assert!(bounds.contains_key(&format!(
        "{SCREEN_BOUNDS_PREFIX}export_publish_disabled_hint"
    )));
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}export_publish")].center(),
    );
    assert!(app.pid_shell.backup.export_ui.dialog.is_none());

    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}export_pdf")].center(),
    );
    assert!(matches!(
        app.pid_shell.backup.export_ui.dialog,
        Some(ExportDialog::Pdf(_))
    ));

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_export_ui_ui7_publish_dialog_prechecks_all_invalid_inputs_and_sql_warning() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui7-pub-check");
    let pkg = scratch.join("pkg");
    let restore = scratch.join("restore");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::create_dir_all(&restore).unwrap();
    std::fs::write(pkg.join("Manifest.txt"), "[General]\r\nName=SqlP\r\n").unwrap();
    let pid_file = restore.join("A01.pid");
    std::fs::write(&pid_file, b"pid").unwrap();

    let non_empty = scratch.join("non-empty");
    std::fs::create_dir_all(&non_empty).unwrap();
    std::fs::write(non_empty.join("blocking.txt"), b"x").unwrap();

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: pkg.clone(),
        restored: restore.clone(),
        backup_root: pkg.clone(),
        read_only: false,
        manifest: Some(ManifestSummary {
            name: Some("SqlP".to_owned()),
            ..ManifestSummary::default()
        }),
        files: vec![BackupFile {
            name: "Export.dmp".to_owned(),
            kind: FileKind::ExportDump,
            format: "SQL Server MTF".to_owned(),
            ..BackupFile::default()
        }],
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(pid_file),
            group_path: "SqlP".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));

    let bounds = drawn(&app);
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}export_publish")].center(),
    );
    let dlg_bounds = drawn(&app);
    assert!(dlg_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}publish_dialog")));
    assert!(dlg_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}sql_warning")));

    // 1. Non-empty folder -> SPPID_EXPORT_TARGET_NOT_EMPTY + blocking.txt, submit disabled.
    send_export(
        &mut app,
        ExportUiMessage::PublishOutChanged(non_empty.display().to_string()),
    );
    assert!(drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")));
    send_export(&mut app, ExportUiMessage::SubmitPublish);
    assert!(!app.pid_backup_export.is_running());

    // 2. Folder inside package or restore -> SPPID_EXPORT_TARGET_INVALID.
    for inside in [pkg.join("sub"), restore.join("sub")] {
        send_export(
            &mut app,
            ExportUiMessage::PublishOutChanged(inside.display().to_string()),
        );
        assert!(drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")));
        send_export(&mut app, ExportUiMessage::SubmitPublish);
        assert!(!app.pid_backup_export.is_running());
    }

    // 3. Empty plant name -> blocked.
    let valid_out = scratch.join("clean-out");
    send_export(
        &mut app,
        ExportUiMessage::PublishOutChanged(valid_out.display().to_string()),
    );
    send_export(
        &mut app,
        ExportUiMessage::PublishPlantChanged("   ".to_owned()),
    );
    assert!(drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")));
    send_export(&mut app, ExportUiMessage::SubmitPublish);
    assert!(!app.pid_backup_export.is_running());

    // 4. Non-existent reference folder -> SPPID_EXPORT_REFERENCE_INVALID.
    send_export(
        &mut app,
        ExportUiMessage::PublishPlantChanged("P01".to_owned()),
    );
    send_export(
        &mut app,
        ExportUiMessage::PublishRefChanged(scratch.join("missing-ref").display().to_string()),
    );
    assert!(drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")));
    send_export(&mut app, ExportUiMessage::SubmitPublish);
    assert!(!app.pid_backup_export.is_running());

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_export_ui_ui7_pdf_dialog_blocks_conflicts_and_pairs_with_last_publish_dir() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui7-pdf-pair");
    let pkg = scratch.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("Manifest.txt"), "[General]\r\nName=P1\r\n").unwrap();
    let pid_file = pkg.join("A01.pid");
    std::fs::write(&pid_file, b"pid").unwrap();

    // Directory with conflicting _PdfIndex.json:
    let conflict_dir = scratch.join("conflict");
    std::fs::create_dir_all(&conflict_dir).unwrap();
    std::fs::write(conflict_dir.join("_PdfIndex.json"), b"{}").unwrap();

    // Directory with publish files only (paired delivery target!):
    let pair_dir = scratch.join("paired-deliver");
    std::fs::create_dir_all(&pair_dir).unwrap();
    std::fs::write(pair_dir.join("_Index.json"), b"{}").unwrap();
    std::fs::write(pair_dir.join("A01_Data.xml"), b"<xml/>").unwrap();

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: pkg.clone(),
        restored: pkg.clone(),
        backup_root: pkg.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("P1".to_owned()),
            ..ManifestSummary::default()
        }),
        files: vec![BackupFile {
            name: "Export.dmp".to_owned(),
            kind: FileKind::ExportDump,
            format: "SQL Server MTF".to_owned(),
            ..BackupFile::default()
        }],
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(pid_file),
            group_path: "P1".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));
    app.pid_shell.backup.export_ui.last_publish_out_dir = Some(pair_dir.clone());

    let bounds = drawn(&app);
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}export_pdf")].center(),
    );
    let dlg = drawn(&app);
    assert!(dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_dialog")));
    assert!(dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}sql_warning")));
    assert!(dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_use_last_publish")));

    // Conflict folder is rejected.
    send_export(
        &mut app,
        ExportUiMessage::PdfOutChanged(conflict_dir.display().to_string()),
    );
    assert!(drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_error")));
    send_export(&mut app, ExportUiMessage::SubmitPdf);
    assert!(!app.pid_backup_pdf.is_running());

    // Clicking "使用上次 publish 输出目录" fills `pair_dir`, shows pairing badge, and allows start!
    let dlg = drawn(&app);
    click_at(
        &mut app,
        dlg[&format!("{SCREEN_BOUNDS_PREFIX}pdf_use_last_publish")].center(),
    );
    let paired_dlg = drawn(&app);
    assert!(paired_dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_pair_ready")));
    assert!(!paired_dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_error")));
    click_at(
        &mut app,
        paired_dlg[&format!("{SCREEN_BOUNDS_PREFIX}pdf_submit")].center(),
    );
    assert!(app.pid_backup_pdf.is_running());
    assert!(app.pid_shell.backup.export_ui.dialog.is_none());

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_export_ui_ui8_running_exports_show_dual_banners_and_chrome_status_switch() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui8-running");
    let pkg = scratch.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("Manifest.txt"), "[General]\r\nName=P1\r\n").unwrap();
    let pid_file = pkg.join("A01.pid");
    std::fs::write(&pid_file, b"pid").unwrap();

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: pkg.clone(),
        restored: pkg.clone(),
        backup_root: pkg.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("P1".to_owned()),
            ..ManifestSummary::default()
        }),
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(pid_file),
            group_path: "P1".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));

    // Start both exports concurrently into separate folders without polling their futures.
    let _pub_task = app
        .start_pid_backup_export_ui(scratch.join("out-pub"), "P01".to_owned(), None)
        .expect("start publish");
    let _pdf_task = app
        .start_pid_backup_pdf_ui(scratch.join("out-pdf"))
        .expect("start pdf");
    assert!(app.pid_backup_export.is_running());
    assert!(app.pid_backup_pdf.is_running());

    // 1. Both running banners appear simultaneously inside Backup screen.
    let bounds = drawn(&app);
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}running_publish")));
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}running_pdf")));

    // 2. Switch to Editor -> bottom chrome status bar shows both running indicators and clicks back!
    assert!(app.show_pid_screen(PidScreen::Editor));
    let status = chrome::status_text(&app, PidScreen::Editor);
    assert!(status.contains("正在导出 publish"), "{status}");
    assert!(status.contains("正在导出 PDF 0/1"), "{status}");

    let ed_bounds = drawn(&app);
    assert!(ed_bounds.contains_key("PID_CHROME:status:backup_export"));
    assert!(ed_bounds.contains_key("PID_CHROME:status:backup_pdf"));
    click_at(&mut app, ed_bounds["PID_CHROME:status:backup_pdf"].center());
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Backup));

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_export_ui_ui8_three_outcome_forms_details_and_path_opener_work_in_both_themes() {
    use crate::io::pid_backup_export::{DrawingNote, ExportError, ExportOutcome, GateSummary};
    use crate::io::pid_backup_pdf::{PdfError, PdfOutcome, PdfPage};
    use crate::ui::style::pid_tokens::PidTheme;

    let _one = crate::headless_ui_lock();
    let src = include_str!("backup_export_ui.rs");
    for forbidden in ["from_rgb", "from_rgba", "Color::from", "0x"] {
        assert!(
            !src.contains(forbidden),
            "backup_export_ui.rs must use pid_tokens instead of literal `{forbidden}`"
        );
    }

    let scratch = temp_scratch("ui8-outcomes");
    let out_dir = scratch.join("deliver");
    let pub_index = out_dir.join("_Index.json");
    let pdf_index = out_dir.join("_PdfIndex.json");

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: scratch.clone(),
        restored: scratch.clone(),
        backup_root: scratch.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("P1".to_owned()),
            ..ManifestSummary::default()
        }),
        files: vec![BackupFile {
            name: "Export.dmp".to_owned(),
            kind: FileKind::ExportDump,
            format: "SQL Server MTF".to_owned(),
            ..BackupFile::default()
        }],
        ..BackupIndex::default()
    }));

    let opened_paths: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let opened_clone = opened_paths.clone();
    app.pid_shell.backup.export_ui.path_opener = Some(Arc::new(move |path| {
        opened_clone.lock().unwrap().push(path.to_path_buf());
    }));

    // Form 1: Clean success + open directory / open index buttons.
    app.on_pid_backup_export_finished(Ok(ExportOutcome {
        written: 7,
        failed: 0,
        pid_entries: 7,
        templates: 0,
        unmatched: Vec::new(),
        without_drawing_no: 0,
        unwritten: Vec::new(),
        noted: Vec::new(),
        problems: Vec::new(),
        other_problems: Vec::new(),
        warnings: Vec::new(),
        gate: None,
        export_kind: "OracleExp".to_owned(),
        files: 22,
        index_path: pub_index.clone(),
        elapsed: Duration::from_millis(4200),
    }));
    app.on_pid_backup_pdf_finished(Ok(PdfOutcome {
        pages: vec![PdfPage {
            name: "A01".to_owned(),
            pdf: Some("A01.pdf".to_owned()),
            ..PdfPage::default()
        }],
        written: 7,
        failed: 0,
        without_file: 0,
        index_path: pdf_index.clone(),
        elapsed: Duration::from_millis(8100),
    }));

    let b1 = drawn(&app);
    assert!(b1.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_outcome_ok")));
    assert!(b1.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_outcome_ok")));
    for key in [
        "pub_open_dir",
        "pub_open_index",
        "pdf_open_dir",
        "pdf_open_index",
    ] {
        let cur = drawn(&app);
        click_at(
            &mut app,
            cur[&format!("{SCREEN_BOUNDS_PREFIX}{key}")].center(),
        );
    }
    assert_eq!(
        *opened_paths.lock().unwrap(),
        vec![
            out_dir.clone(),
            pub_index.clone(),
            out_dir.clone(),
            pdf_index.clone()
        ]
    );

    // Form 2: Warnings, unwritten drawings, GateSummary 4 counts + lines, missing symbols, SQL Server.
    app.on_pid_backup_export_finished(Ok(ExportOutcome {
        written: 52,
        failed: 1,
        pid_entries: 53,
        templates: 0,
        unmatched: Vec::new(),
        without_drawing_no: 2,
        unwritten: vec![DrawingNote {
            name: "D03".to_owned(),
            notes: vec!["corrupt stream".to_owned()],
        }],
        noted: vec![DrawingNote {
            name: "D04".to_owned(),
            notes: vec!["fallback number".to_owned()],
        }],
        problems: Vec::new(),
        other_problems: vec!["archive warning".to_owned()],
        warnings: vec!["minor warning".to_owned()],
        gate: Some(GateSummary {
            clean: 15,
            dirty: 32,
            revision_mismatch: 0,
            unpaired: 6,
            lines: vec!["A01: 2 attribute diffs".to_owned()],
        }),
        export_kind: "SqlServerMtf".to_owned(),
        files: 160,
        index_path: pub_index,
        elapsed: Duration::from_secs(28),
    }));
    app.on_pid_backup_pdf_finished(Ok(PdfOutcome {
        pages: vec![
            PdfPage {
                name: "A01".to_owned(),
                pdf: Some("A01.pdf".to_owned()),
                missing_symbols: vec!["ValveA.sym".to_owned(), "PumpB.sym".to_owned()],
                ..PdfPage::default()
            },
            PdfPage {
                name: "D03".to_owned(),
                pdf: None,
                error: Some("cannot read OLE".to_owned()),
                ..PdfPage::default()
            },
        ],
        written: 52,
        failed: 1,
        without_file: 0,
        index_path: pdf_index,
        elapsed: Duration::from_secs(52),
    }));

    let b2 = drawn(&app);
    click_at(
        &mut app,
        b2[&format!("{SCREEN_BOUNDS_PREFIX}pub_toggle_details")].center(),
    );
    let b2 = drawn(&app);
    click_at(
        &mut app,
        b2[&format!("{SCREEN_BOUNDS_PREFIX}pdf_toggle_details")].center(),
    );
    let b2_open = drawn(&app);
    assert!(b2_open.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_details")));
    assert!(b2_open.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_details")));
    assert!(says(&app, "未写出 [D03]: corrupt stream"));
    assert!(says(&app, "比对: A01: 2 attribute diffs"));
    assert!(says(&app, "缺符号 [A01]: ValveA.sym, PumpB.sym"));

    // Form 3: Whole-run failure with SPPID_* error codes in both Light and Dark themes.
    app.on_pid_backup_export_finished(Err(ExportError::Unsupported("bad dump".to_owned())));
    app.on_pid_backup_pdf_finished(Err(PdfError::Failed("disk full".to_owned())));
    for theme in [PidTheme::Light, PidTheme::Dark] {
        app.pid_shell.theme = theme;
        let b3 = drawn(&app);
        assert!(b3.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_outcome_err")));
        assert!(b3.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_outcome_err")));
        assert!(says(
            &app,
            "publish 文件导出失败 [SPPID_EXPORT_UNSUPPORTED]"
        ));
        assert!(says(&app, "PDF 导出失败 [SPPID_PDF_FAILED]"));
    }

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_export_ui_review_cross_package_isolation_quoted_paths_and_cached_validation() {
    use crate::io::pid_backup_export::{ExportOutcome, GateSummary};
    use crate::io::pid_backup_pdf::{PdfOutcome, PdfPage};
    use crate::ui::window::pid_backup_browser::PidBackupMsg;

    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("review-export-iso");
    let pkg_a = scratch.join("pkg-a");
    let pkg_b = scratch.join("pkg-b");
    std::fs::create_dir_all(&pkg_a).unwrap();
    std::fs::create_dir_all(&pkg_b).unwrap();
    std::fs::write(pkg_a.join("Manifest.txt"), "[General]\r\nName=PA\r\n").unwrap();
    std::fs::write(pkg_b.join("Manifest.txt"), "[General]\r\nName=PB\r\n").unwrap();
    let pid_a = pkg_a.join("A01.pid");
    let pid_b = pkg_b.join("B01.pid");
    std::fs::write(&pid_a, b"pid-a").unwrap();
    std::fs::write(&pid_b, b"pid-b").unwrap();

    let mut app = app_on_backup();
    let load_a = app.on_pid_backup(PidBackupMsg::Loaded(Ok(Arc::new(BackupIndex {
        source: pkg_a.clone(),
        restored: pkg_a.clone(),
        backup_root: pkg_a.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("PA".to_owned()),
            ..ManifestSummary::default()
        }),
        files: vec![BackupFile {
            name: "Manifest.txt".to_owned(),
            ..BackupFile::default()
        }],
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(pid_a),
            group_path: "PA".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }))));
    app.drive_headless_task(load_a).unwrap();

    // 1. Finish publish (>120 gate lines -> footer shown) and PDF on Package A.
    let out_a = scratch.join("deliver-a");
    std::fs::create_dir_all(&out_a).unwrap();
    std::fs::write(out_a.join("_Index.json"), b"{}").unwrap();
    app.pid_shell.backup.export_ui.last_publish_out_dir = Some(out_a.clone());
    app.on_pid_backup_export_finished(Ok(ExportOutcome {
        written: 1,
        failed: 0,
        pid_entries: 1,
        templates: 0,
        unmatched: Vec::new(),
        without_drawing_no: 0,
        unwritten: Vec::new(),
        noted: Vec::new(),
        problems: Vec::new(),
        other_problems: Vec::new(),
        warnings: Vec::new(),
        gate: Some(GateSummary {
            clean: 125,
            dirty: 0,
            revision_mismatch: 0,
            unpaired: 0,
            lines: (0..125).map(|i| format!("D{i:03}: clean")).collect(),
        }),
        export_kind: "OracleExp".to_owned(),
        files: 4,
        index_path: out_a.join("_Index.json"),
        elapsed: Duration::from_secs(1),
    }));
    app.on_pid_backup_pdf_finished(Ok(PdfOutcome {
        pages: vec![PdfPage {
            name: "A01".to_owned(),
            pdf: Some("A01.pdf".to_owned()),
            ..PdfPage::default()
        }],
        written: 1,
        failed: 0,
        without_file: 0,
        index_path: out_a.join("_PdfIndex.json"),
        elapsed: Duration::from_secs(1),
    }));

    send_export(&mut app, ExportUiMessage::TogglePublishDetails);
    assert!(
        says(
            &app,
            "已显示前 120 条（共 125 条），完整比对明细见清单文件。"
        ),
        "gate lines > 120 must display truncation footer"
    );

    // 2. Switch to Package B: Package A's outcome banners and last_publish_out_dir must NOT leak!
    let load_b = app.on_pid_backup(PidBackupMsg::Loaded(Ok(Arc::new(BackupIndex {
        source: pkg_b.clone(),
        restored: pkg_b.clone(),
        backup_root: pkg_b.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("PB".to_owned()),
            ..ManifestSummary::default()
        }),
        files: vec![BackupFile {
            name: "Manifest.txt".to_owned(),
            ..BackupFile::default()
        }],
        drawings: vec![DrawingEntry {
            name: "B01".to_owned(),
            pid_path: Some(pid_b),
            group_path: "PB".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }))));
    app.drive_headless_task(load_b).unwrap();

    let b_bounds = drawn(&app);
    assert!(
        !b_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_outcome_ok")),
        "Package A publish outcome must not show under Package B"
    );
    assert!(
        !b_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_outcome_ok")),
        "Package A PDF outcome must not show under Package B"
    );
    send_export(&mut app, ExportUiMessage::OpenPdfDialog);
    let pdf_dlg = drawn(&app);
    assert!(
        !pdf_dlg.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pdf_use_last_publish")),
        "Package A publish folder must not be offered for pairing on Package B"
    );
    send_export(&mut app, ExportUiMessage::CloseDialog);

    // 3. Double-quoted output path validates in update(), and view() caches validation (zero per-frame disk I/O).
    send_export(&mut app, ExportUiMessage::OpenPublishDialog);
    let out_b = scratch.join("deliver-b");
    send_export(
        &mut app,
        ExportUiMessage::PublishOutChanged(format!("  \"{}\"  ", out_b.display())),
    );
    assert!(
        !drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")),
        "double-quoted valid output folder must pass precheck"
    );
    // Create a file behind the dialog's back without triggering update(): pure view() must NOT re-scan disk!
    std::fs::create_dir_all(&out_b).unwrap();
    std::fs::write(out_b.join("sneaky.txt"), b"x").unwrap();
    assert!(
        !drawn(&app).contains_key(&format!("{SCREEN_BOUNDS_PREFIX}pub_error")),
        "view() must read cached validation rather than running synchronous fs::read_dir every frame"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
