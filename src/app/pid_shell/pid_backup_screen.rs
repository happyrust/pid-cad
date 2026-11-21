//! Headless `iced_test` verification of the `backup` screen (`UI-4`..`UI-6`).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::{Point, Rectangle, Size};

use super::backup::{self, Message as BackupMessage, SCREEN_BOUNDS_PREFIX};
use super::*;
use crate::cli::GuiConfig;
use crate::io::pid_backup_browse::{
    BackupFile, BackupIndex, DrawingEntry, FileKind, GroupNode, ManifestSummary, RefFile, RefNode,
};
use crate::ui::style::pid_tokens::PidTheme;
use crate::ui::window::pid_backup_browser::{self, LoadState};

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
            let task = app.update(message);
            app.drive_headless_task(task)
                .expect("headless click task completes");
        }
    }
}

fn temp_scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let dir = std::env::temp_dir().join(format!(
        "ocs-pid-backup-screen-{}-{}-{}",
        tag,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_minimal_backup_zip(zip_path: &Path) {
    let file = std::fs::File::create(zip_path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("Manifest.txt", opts).expect("manifest");
    zip.write_all(
        b"[General]\r\nName=TestPlant\r\nVersion=7.02\r\nDateCreated=11/20/2026 08:00:00\r\nBackupType=2\r\n",
    )
    .expect("write manifest");
    zip.finish().expect("finish zip");
}

#[test]
fn pid_backup_screen_ui4_empty_loading_and_error_states_render_and_drive_picker() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui4-states");
    let cache_dir = scratch.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();

    let mut app = app_on_backup();
    pid_backup_browser::set_cache_folder(&mut app.pid_backup_browser, Some(cache_dir));

    // 1. Empty state.
    let bounds = drawn(&app);
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}empty")));
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}open_zip")));
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}open_folder")));
    assert!(says(&app, "打开备份 zip"));
    assert!(says(&app, "打开备份文件夹"));

    // 2. Pick an empty non-backup folder -> transitions to Error state.
    let bad_folder = scratch.join("not-a-backup");
    std::fs::create_dir_all(&bad_folder).unwrap();
    let picked_bad = bad_folder.clone();
    app.pid_shell.backup.source_picker = Some(Arc::new(move |folder| {
        assert!(folder, "open_folder asks for a folder");
        Some(Some(picked_bad.clone()))
    }));
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}open_folder")].center(),
    );
    assert!(app.pid_backup_browser.error.is_some());
    let err_bounds = drawn(&app);
    assert!(err_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}error")));
    assert!(err_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}retry_zip")));
    assert!(err_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}retry_folder")));
    assert!(says(&app, "重新选择 zip"));
    assert!(says(&app, "重新选择文件夹"));

    // 3. Loading state shows source path, loading prompt, and elapsed seconds.
    let loading_src = scratch.join("plant-loading.zip");
    app.pid_backup_browser.load = LoadState::Loading(loading_src.clone());
    app.pid_backup_browser.loading_started_at =
        Some(iced::time::Instant::now() - Duration::from_secs(6));
    let load_bounds = drawn(&app);
    assert!(load_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}loading")));
    assert!(says(&app, "正在还原并建立索引…"));

    // 4. Retry zip with a valid backup zip -> advances to Ready.
    app.pid_backup_browser.load = LoadState::Idle;
    let good_zip = scratch.join("good-backup.zip");
    write_minimal_backup_zip(&good_zip);
    let picked_good = good_zip.clone();
    app.pid_shell.backup.source_picker = Some(Arc::new(move |folder| {
        assert!(!folder, "retry_zip asks for a zip");
        Some(Some(picked_good.clone()))
    }));
    let err_bounds = drawn(&app);
    click_at(
        &mut app,
        err_bounds[&format!("{SCREEN_BOUNDS_PREFIX}retry_zip")].center(),
    );
    assert!(matches!(app.pid_backup_browser.load, LoadState::Ready(_)));
    assert!(
        !app.show_pid_backup,
        "opening from the new backup screen keeps the classic docked panel closed"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_ui4_package_header_renders_oracle_sql_server_and_loose_pid_summaries() {
    let _one = crate::headless_ui_lock();
    let mut app = app_on_backup();

    // Case A: Oracle package with ManifestSummary.
    let oracle_index = BackupIndex {
        source: PathBuf::from("D:/plant/SQPlant.zip"),
        restored: PathBuf::from("C:/temp/restored-sq"),
        backup_root: PathBuf::from("C:/temp/restored-sq"),
        read_only: false,
        manifest: Some(ManifestSummary {
            name: Some("SQPlant".to_owned()),
            version: Some("7.02".to_owned()),
            date_created: Some("07/19/2026 20:13:24".to_owned()),
            backup_type: Some("2".to_owned()),
            spid: None,
        }),
        files: vec![BackupFile {
            name: "Export.dmp".to_owned(),
            kind: FileKind::ExportDump,
            format: "Oracle exp".to_owned(),
            ..BackupFile::default()
        }],
        plant_groups: vec![GroupNode {
            name: "SQPlant".to_owned(),
            path: "SQPlant".to_owned(),
            children: Vec::new(),
            drawings: vec![0],
        }],
        drawings: vec![DrawingEntry {
            name: "A-101".to_owned(),
            pid_path: Some(PathBuf::from("C:/temp/restored-sq/A-101.pid")),
            group_path: "SQPlant".to_owned(),
            meta: Vec::new(),
        }],
        ref_tree: RefNode {
            name: "Ref".to_owned(),
            dirs: Vec::new(),
            files: vec![RefFile {
                name: "sym.sym".to_owned(),
                size: 128,
            }],
        },
        warnings: vec!["one warning".to_owned()],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(oracle_index));
    let bounds = drawn(&app);
    for key in [
        "header",
        "switch_zip",
        "switch_folder",
        "clear_cache",
        "classic_panel",
    ] {
        assert!(
            bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}{key}")),
            "missing {key}"
        );
    }
    assert!(says(&app, "清理还原缓存"));
    assert!(says(&app, "在经典面板中查看库表"));

    // Case B: SQL Server package summary.
    let sql_index = BackupIndex {
        source: PathBuf::from("D:/plant/SqlBackup"),
        restored: PathBuf::from("D:/plant/SqlBackup"),
        backup_root: PathBuf::from("D:/plant/SqlBackup"),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("SqlPlant".to_owned()),
            version: Some("8.00".to_owned()),
            date_created: Some("08/01/2026 10:00:00".to_owned()),
            backup_type: Some("1".to_owned()),
            spid: None,
        }),
        files: vec![BackupFile {
            name: "Export.dmp".to_owned(),
            kind: FileKind::ExportDump,
            format: "SQL Server MTF".to_owned(),
            ..BackupFile::default()
        }],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(sql_index));
    let _ = drawn(&app);
    assert!(says(
        &app,
        &backup::header_title_line(app.pid_backup_browser.index().unwrap())
    ));

    // Case C: Loose .pid folder without Manifest.txt.
    let loose_index = BackupIndex {
        source: PathBuf::from("D:/drawings/loose-pids"),
        restored: PathBuf::from("D:/drawings/loose-pids"),
        backup_root: PathBuf::from("D:/drawings/loose-pids"),
        read_only: true,
        manifest: None,
        drawings: vec![DrawingEntry {
            name: "L-01".to_owned(),
            pid_path: Some(PathBuf::from("D:/drawings/loose-pids/L-01.pid")),
            group_path: "loose-pids".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(loose_index));
    let _ = drawn(&app);
    assert!(
        backup::header_meta_line(app.pid_backup_browser.index().unwrap())
            .contains("直接使用目录（只读）")
    );
}

#[test]
fn pid_backup_screen_ui4_clear_cache_and_classic_panel_actions_respect_export_busy() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui4-actions");
    let cache_dir = scratch.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let stale_zip = scratch.join("stale.zip");
    write_minimal_backup_zip(&stale_zip);
    let stale_restore = crate::io::pid_backup::restore_in(&stale_zip, None, Some(&cache_dir))
        .expect("create real cached restore")
        .root;
    assert!(stale_restore.exists());

    let pkg_dir = scratch.join("pkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("Manifest.txt"), "[General]\r\nName=P1\r\n").unwrap();
    let dummy_pid = pkg_dir.join("A01.pid");
    std::fs::write(&dummy_pid, b"not-polled").unwrap();

    let mut app = app_on_backup();
    pid_backup_browser::set_cache_folder(&mut app.pid_backup_browser, Some(cache_dir.clone()));
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: pkg_dir.clone(),
        restored: pkg_dir.clone(),
        backup_root: pkg_dir.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("P1".to_owned()),
            ..ManifestSummary::default()
        }),
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(dummy_pid),
            group_path: "P1".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));

    // 1. Clear cache while idle succeeds and removes stale restore.
    let task = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ClearCache,
    )));
    app.drive_headless_task(task).unwrap();
    assert!(
        !stale_restore.exists(),
        "idle clear cache removes stale restore"
    );

    // 2. Start a PDF export without polling its background future -> running!
    let out_pdf = scratch.join("out-pdf");
    let _unpolled_export = app
        .start_pid_backup_pdf_ui(out_pdf)
        .expect("pdf export starts");
    assert!(app.pid_backup_pdf.is_running());

    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ClearCache,
    )));
    assert!(
        app.pid_shell
            .backup
            .notice
            .as_ref()
            .is_some_and(|(tone, msg)| *tone == backup::NoticeTone::Warning && msg.contains("导出")),
        "busy export blocks cache clearing with an inline warning: {:?}",
        app.pid_shell.backup.notice
    );

    // 3. OpenClassicPanel switches to Editor and reveals PanelId::PidBackup.
    assert!(!app.show_pid_backup);
    let bounds = drawn(&app);
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}classic_panel")].center(),
    );
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
    assert!(app.show_pid_backup);

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_ui4_uses_only_pid_tokens_and_renders_both_themes() {
    let _one = crate::headless_ui_lock();
    let src = include_str!("backup.rs");
    for forbidden in ["from_rgb", "from_rgba", "Color::from", "0x"] {
        assert!(
            !src.contains(forbidden),
            "backup.rs must use pid_tokens instead of literal colour `{forbidden}`"
        );
    }

    let mut app = app_on_backup();
    for theme in [PidTheme::Light, PidTheme::Dark] {
        app.pid_shell.theme = theme;
        let bounds = drawn(&app);
        assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}empty")));
    }
}

#[test]
fn pid_backup_screen_ui5_group_tree_and_drawing_search_filter_precisely() {
    let _one = crate::headless_ui_lock();
    let mut app = app_on_backup();

    let index = BackupIndex {
        source: PathBuf::from("D:/plant/NestedPlant"),
        restored: PathBuf::from("D:/plant/NestedPlant"),
        backup_root: PathBuf::from("D:/plant/NestedPlant"),
        read_only: true,
        plant_groups: vec![
            GroupNode {
                name: "SQPlant".to_owned(),
                path: "SQPlant".to_owned(),
                drawings: vec![0],
                children: vec![GroupNode {
                    name: "00".to_owned(),
                    path: "SQPlant / 00".to_owned(),
                    drawings: vec![1],
                    children: Vec::new(),
                }],
            },
            GroupNode {
                name: crate::io::pid_backup_browse::UNREGISTERED_GROUP.to_owned(),
                path: crate::io::pid_backup_browse::UNREGISTERED_GROUP.to_owned(),
                drawings: vec![2],
                children: Vec::new(),
            },
            GroupNode {
                name: crate::io::pid_backup_browse::UNGROUPED.to_owned(),
                path: crate::io::pid_backup_browse::UNGROUPED.to_owned(),
                drawings: vec![3],
                children: Vec::new(),
            },
        ],
        drawings: vec![
            DrawingEntry {
                name: "A-100 主装置总图".to_owned(),
                pid_path: Some(PathBuf::from("D:/plant/NestedPlant/A-100.pid")),
                group_path: "SQPlant".to_owned(),
                meta: vec![("DrawingNumber".to_owned(), "A-100".to_owned())],
            },
            DrawingEntry {
                name: "B-200 压缩机流程图".to_owned(),
                pid_path: Some(PathBuf::from("D:/plant/NestedPlant/B-200.pid")),
                group_path: "SQPlant / 00".to_owned(),
                meta: vec![("DrawingNumber".to_owned(), "B-200".to_owned())],
            },
            DrawingEntry {
                name: "U-300 散落图纸".to_owned(),
                pid_path: Some(PathBuf::from("D:/plant/NestedPlant/U-300.pid")),
                group_path: crate::io::pid_backup_browse::UNREGISTERED_GROUP.to_owned(),
                meta: Vec::new(),
            },
            DrawingEntry {
                name: "M-400 缺文件图纸".to_owned(),
                pid_path: None,
                group_path: crate::io::pid_backup_browse::UNGROUPED.to_owned(),
                meta: vec![("Title".to_owned(), "Missing Sheet".to_owned())],
            },
        ],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(index));
    app.pid_shell.backup.reset_for_loaded_package(4);

    // Special groups render localized Chinese labels (S2).
    assert!(says(&app, "未登记图纸"));
    assert!(says(&app, "未分组"));
    let bounds = drawn(&app);
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:0")));
    assert!(bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:3")));

    // Selecting nested group `SQPlant / 00` shows only drawing 1.
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}group:SQPlant / 00")].center(),
    );
    let bounds_sub = drawn(&app);
    assert!(!bounds_sub.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:0")));
    assert!(bounds_sub.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:1")));
    assert_eq!(app.pid_shell.backup.selected_drawing, Some(1));

    // Selecting parent `SQPlant` includes subtree drawings 0 and 1, then keyword narrows to B-200.
    click_at(
        &mut app,
        bounds_sub[&format!("{SCREEN_BOUNDS_PREFIX}group:SQPlant")].center(),
    );
    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::DrawingFilterChanged("b-200".to_owned()),
    )));
    let bounds_kw = drawn(&app);
    assert!(!bounds_kw.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:0")));
    assert!(bounds_kw.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}drawing:1")));
}

#[test]
fn pid_backup_screen_ui5_details_show_db_fields_filter_items_and_surface_unreadable_or_missing() {
    use crate::io::pid_backup_browse::{DrawingItem, DrawingItems, ItemAttribute, ItemGroup};
    use crate::ui::window::pid_backup_browser::ItemsState;

    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui5-details");
    let bad_pid = scratch.join("D04.pid");
    std::fs::write(&bad_pid, b"not-an-ole-compound-pid").unwrap();

    let mut app = app_on_backup();
    let index = BackupIndex {
        source: scratch.clone(),
        restored: scratch.clone(),
        backup_root: scratch.clone(),
        read_only: true,
        drawings: vec![
            DrawingEntry {
                name: "D01 可读工艺图".to_owned(),
                pid_path: Some(scratch.join("D01.pid")),
                group_path: "SQPlant / 00".to_owned(),
                meta: vec![
                    ("DrawingNumber".to_owned(), "D01-A".to_owned()),
                    ("PlantArea".to_owned(), "Area-00".to_owned()),
                ],
            },
            DrawingEntry {
                name: "D04 损坏对象图".to_owned(),
                pid_path: Some(bad_pid),
                group_path: "SQPlant / 00".to_owned(),
                meta: vec![("DrawingNumber".to_owned(), "D04-ERR".to_owned())],
            },
            DrawingEntry {
                name: "D05 登记缺文件图".to_owned(),
                pid_path: None,
                group_path: crate::io::pid_backup_browse::UNGROUPED.to_owned(),
                meta: vec![("DrawingNumber".to_owned(), "D05-MISS".to_owned())],
            },
        ],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(index));
    app.pid_backup_browser.items.insert(
        0,
        ItemsState::Ready(Arc::new(DrawingItems {
            drawing_uid: "DOC-001".to_owned(),
            drawing_name: "D01 可读工艺图".to_owned(),
            item_count: 2,
            groups: vec![
                ItemGroup {
                    item_type: "Equipment".to_owned(),
                    items: vec![DrawingItem {
                        uid: "EQ-1".to_owned(),
                        label: "V-1001".to_owned(),
                        item_type: "Equipment".to_owned(),
                        class: Some("Vessel".to_owned()),
                        attributes: vec![ItemAttribute {
                            interface: "IJNamedItem".to_owned(),
                            name: "Name".to_owned(),
                            value: "V-1001".to_owned(),
                        }],
                    }],
                },
                ItemGroup {
                    item_type: "Instrument".to_owned(),
                    items: vec![DrawingItem {
                        uid: "IN-2".to_owned(),
                        label: "PT-1001".to_owned(),
                        item_type: "Instrument".to_owned(),
                        class: Some("PressureTransmitter".to_owned()),
                        attributes: Vec::new(),
                    }],
                },
            ],
            data: Default::default(),
        })),
    );
    app.pid_shell.backup.reset_for_loaded_package(3);

    // 1. Readable drawing shows DB fields, type chips, and filters by type + keyword + expand.
    let bounds0 = drawn(&app);
    assert!(bounds0.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}db_fields")));
    assert!(bounds0.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:EQ-1")));
    assert!(bounds0.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:IN-2")));

    click_at(
        &mut app,
        bounds0[&format!("{SCREEN_BOUNDS_PREFIX}item_type:Instrument")].center(),
    );
    let bounds_inst = drawn(&app);
    assert!(!bounds_inst.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:EQ-1")));
    assert!(bounds_inst.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:IN-2")));

    click_at(
        &mut app,
        bounds_inst[&format!("{SCREEN_BOUNDS_PREFIX}item_type:all")].center(),
    );
    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ItemQueryChanged("v-1001".to_owned()),
    )));
    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ToggleItemExpanded("EQ-1".to_owned()),
    )));
    let bounds_q = drawn(&app);
    assert!(bounds_q.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:EQ-1")));
    assert!(!bounds_q.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}item:IN-2")));
    assert!(says(&app, "  IJNamedItem.Name = V-1001"));

    // 2. Selecting D04 automatically triggers background item load -> Failed(reason) shown clearly.
    click_at(
        &mut app,
        bounds_q[&format!("{SCREEN_BOUNDS_PREFIX}drawing:1")].center(),
    );
    assert!(matches!(
        app.pid_backup_browser.items.get(&1),
        Some(ItemsState::Failed(_))
    ));
    let bounds_d04 = drawn(&app);
    assert!(bounds_d04.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}items_error")));
    assert!(says(&app, "无法读取图内对象"));
    assert!(says(&app, "D04-ERR"));

    // 3. Selecting missing-file drawing D05 disables open button and shows missing-file notice.
    click_at(
        &mut app,
        bounds_d04[&format!("{SCREEN_BOUNDS_PREFIX}drawing:2")].center(),
    );
    let bounds_d05 = drawn(&app);
    assert!(bounds_d05.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}missing_file_notice")));
    assert!(says(&app, "D05-MISS"));

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_ui6_open_drawing_switches_to_editor_reuses_open_tab_and_preserves_state() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui6-open");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plot/page-setups-metric.dwg");
    let valid_dxf = scratch.join("SheetA.dwg");
    std::fs::copy(&fixture, &valid_dxf).unwrap();
    let second_dxf = scratch.join("SheetB.dwg");
    std::fs::copy(&fixture, &second_dxf).unwrap();

    let mut app = app_on_backup();
    let index = BackupIndex {
        source: scratch.clone(),
        restored: scratch.clone(),
        backup_root: scratch.clone(),
        read_only: true,
        plant_groups: vec![GroupNode {
            name: "Area01".to_owned(),
            path: "Area01".to_owned(),
            drawings: vec![0, 1],
            children: Vec::new(),
        }],
        drawings: vec![
            DrawingEntry {
                name: "SheetA".to_owned(),
                pid_path: Some(valid_dxf.clone()),
                group_path: "Area01".to_owned(),
                meta: Vec::new(),
            },
            DrawingEntry {
                name: "SheetB".to_owned(),
                pid_path: Some(second_dxf.clone()),
                group_path: "Area01".to_owned(),
                meta: Vec::new(),
            },
        ],
        ..BackupIndex::default()
    };
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(index));
    app.pid_shell.backup.selected_group = Some("Area01".to_owned());
    app.pid_shell.backup.drawing_filter = "Sheet".to_owned();
    app.pid_shell.backup.selected_drawing = Some(0);
    app.pid_shell.backup.item_type_filter = Some("Equipment".to_owned());
    app.pid_shell.backup.item_query = "V-1001".to_owned();

    // 1. Click "在编辑器中打开" opens SheetA and switches to Editor.
    let bounds = drawn(&app);
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}open_drawing")].center(),
    );
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
    let sheeta_tab = app.tab_showing(&valid_dxf).expect("SheetA opened in a tab");

    //Open SheetB in another tab so SheetA is not the front tab, then switch back to Backup.
    let open_b = app.update(AppMessage::OpenRecent(second_dxf.clone()));
    app.drive_headless_task(open_b).unwrap();
    let tabs_count = app.tabs.len();
    assert_ne!(app.active_tab, sheeta_tab);

    // 2. Round-trip Backup -> Editor -> Backup preserves every filter & selection field.
    assert!(app.show_pid_screen(PidScreen::Backup));
    assert_eq!(
        app.pid_shell.backup.selected_group.as_deref(),
        Some("Area01")
    );
    assert_eq!(app.pid_shell.backup.drawing_filter, "Sheet");
    assert_eq!(app.pid_shell.backup.selected_drawing, Some(0));
    assert_eq!(
        app.pid_shell.backup.item_type_filter.as_deref(),
        Some("Equipment")
    );
    assert_eq!(app.pid_shell.backup.item_query, "V-1001");

    // 3. Double-clicking drawing:0 (already open!) reuses its existing tab without adding a tab.
    let bounds_back = drawn(&app);
    let row0 = bounds_back[&format!("{SCREEN_BOUNDS_PREFIX}drawing:0")].center();
    click_at(&mut app, row0);
    click_at(&mut app, row0);
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
    assert_eq!(
        app.tabs.len(),
        tabs_count,
        "already-open drawing is not reopened"
    );
    assert_eq!(app.active_tab, sheeta_tab);

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_ui6_corrupt_drawing_stays_on_backup_without_recovery_modal() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("ui6-corrupt");
    let bad_pid = scratch.join("D03.pid");
    std::fs::write(&bad_pid, b"corrupt-non-cfb-stream-d03").unwrap();

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: scratch.clone(),
        restored: scratch.clone(),
        backup_root: scratch.clone(),
        read_only: true,
        drawings: vec![DrawingEntry {
            name: "D03 坏图样本".to_owned(),
            pid_path: Some(bad_pid.clone()),
            group_path: "SQPlant".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));
    app.pid_shell.backup.selected_drawing = Some(0);
    let tabs_before = app.tabs.len();

    // 1. Opening D03 from Backup screen fails in-place: stays on Backup, no Recovery modal!
    let bounds = drawn(&app);
    click_at(
        &mut app,
        bounds[&format!("{SCREEN_BOUNDS_PREFIX}open_drawing")].center(),
    );
    assert_eq!(
        app.pid_shell.current(),
        Some(PidScreen::Backup),
        "failed open from backup stays on Backup screen"
    );
    assert_eq!(
        app.active_modal, None,
        "S4: failed open from backup does not stack ModalKind::Recovery"
    );
    assert_eq!(app.tabs.len(), tabs_before, "no broken tab added");
    assert!(app.pid_shell.backup.open_failures.contains_key(&bad_pid));

    let bounds_after = drawn(&app);
    assert!(bounds_after.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}open_error")));

    // 2. Opening the same corrupt file from the Editor still shows ModalKind::Recovery as before.
    assert!(app.show_pid_screen(PidScreen::Editor));
    let editor_open = app.update(AppMessage::OpenRecent(bad_pid));
    app.drive_headless_task(editor_open).unwrap();
    assert_eq!(
        app.active_modal,
        Some(crate::app::ModalKind::Recovery),
        "editor open failure keeps its existing recovery modal"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_review_switch_to_backup_loads_items_tabs_and_quoted_paths() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("review-switch-quotes");
    let zip_path = scratch.join("pkg.zip");
    write_minimal_backup_zip(&zip_path);
    let dummy_pid = scratch.join("D04.pid");
    std::fs::write(&dummy_pid, b"not-a-valid-cfb").unwrap();

    // 1. Load a package while standing on Editor, then switch to Backup via left nav:
    //    selected drawing 0 must automatically start loading items (not sit at None!).
    let mut app = app_on_backup();
    assert!(app.show_pid_screen(PidScreen::Editor));
    let load_task = app.on_pid_backup(pid_backup_browser::PidBackupMsg::Loaded(Ok(Arc::new(
        BackupIndex {
            source: scratch.clone(),
            restored: scratch.clone(),
            backup_root: scratch.clone(),
            read_only: true,
            drawings: vec![DrawingEntry {
                name: "D04".to_owned(),
                pid_path: Some(dummy_pid),
                group_path: "SQPlant".to_owned(),
                meta: vec![("NAME".to_owned(), "D04".to_owned())],
            }],
            ..BackupIndex::default()
        },
    ))));
    app.drive_headless_task(load_task).unwrap();
    assert!(
        !app.pid_backup_browser.items.contains_key(&0),
        "items are not loaded while still on Editor"
    );

    let editor_bounds = drawn(&app);
    click_at(
        &mut app,
        editor_bounds[&chrome::nav_bounds_id(PidScreen::Backup)].center(),
    );
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Backup));
    assert!(
        app.pid_backup_browser.items.contains_key(&0),
        "switching to Backup screen must trigger items loading for selected drawing 0"
    );

    // 2. Detail-pane segmented tabs (`detail_tab:all`, `detail_tab:props`, `detail_tab:items`) work.
    let bk_bounds = drawn(&app);
    assert!(bk_bounds.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}detail_tab:props")));
    click_at(
        &mut app,
        bk_bounds[&format!("{SCREEN_BOUNDS_PREFIX}detail_tab:props")].center(),
    );
    let props_only = drawn(&app);
    assert!(props_only.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}db_fields")));
    assert!(!props_only.contains_key(&format!("{SCREEN_BOUNDS_PREFIX}items_card")));

    // 3. Windows Explorer "Copy as path" surrounding double-quotes are stripped cleanly.
    let _ = app.on_pid_backup(pid_backup_browser::PidBackupMsg::Close);
    let quoted = format!("  \"{}\"  ", zip_path.display());
    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::SourceInputChanged(quoted),
    )));
    let open_task = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::OpenTypedSource,
    )));
    app.drive_headless_task(open_task).unwrap();
    assert!(
        matches!(app.pid_backup_browser.load, LoadState::Ready(_)),
        "double-quoted backup path must strip quotes and load cleanly, err={:?}",
        app.pid_backup_browser.error
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pid_backup_screen_review_busy_blocks_package_switch_and_cache_error_surfaces() {
    let _one = crate::headless_ui_lock();
    let scratch = temp_scratch("review-busy-cache");
    let pkg_dir = scratch.join("pkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("Manifest.txt"), "[General]\r\nName=P1\r\n").unwrap();
    let dummy_pid = pkg_dir.join("A01.pid");
    std::fs::write(&dummy_pid, b"pid").unwrap();

    let other_zip = scratch.join("other.zip");
    write_minimal_backup_zip(&other_zip);

    let mut app = app_on_backup();
    app.pid_backup_browser.load = LoadState::Ready(Arc::new(BackupIndex {
        source: pkg_dir.clone(),
        restored: pkg_dir.clone(),
        backup_root: pkg_dir.clone(),
        read_only: true,
        manifest: Some(ManifestSummary {
            name: Some("P1".to_owned()),
            ..ManifestSummary::default()
        }),
        drawings: vec![DrawingEntry {
            name: "A01".to_owned(),
            pid_path: Some(dummy_pid),
            group_path: "P1".to_owned(),
            meta: Vec::new(),
        }],
        ..BackupIndex::default()
    }));

    // 1. When cache clearing fails (e.g. cache root is a regular file, not a directory),
    //    `backup.notice` must surface `NoticeTone::Warning` instead of claiming success!
    let file_not_dir = scratch.join("cache-is-a-file");
    std::fs::write(&file_not_dir, b"blocking-file").unwrap();
    let bad_sub = file_not_dir.join("nested");
    pid_backup_browser::set_cache_folder(&mut app.pid_backup_browser, Some(bad_sub));
    // Trigger two ClearCache messages back-to-back before driving the first so the second
    // hits `state.cache.clearing == true` -> `Err("the restore cache is being cleared already")`.
    let _first_clear = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ClearCache,
    )));
    let _second_clear = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::ClearCache,
    )));
    assert!(
        app.pid_shell
            .backup
            .notice
            .as_ref()
            .is_some_and(|(tone, _)| *tone == backup::NoticeTone::Warning),
        "cache clear failure must report Warning instead of Info success, got {:?}",
        app.pid_shell.backup.notice
    );

    // 2. While PDF export is running, switching package via OpenTypedSource or PickSource is blocked!
    let _unpolled_pdf = app
        .start_pid_backup_pdf_ui(scratch.join("pdf-out"))
        .expect("start pdf");
    assert!(app.pid_backup_pdf.is_running());

    let _ = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::SourceInputChanged(other_zip.display().to_string()),
    )));
    let blocked_task = app.update(AppMessage::PidShell(Message::Backup(
        BackupMessage::OpenTypedSource,
    )));
    app.drive_headless_task(blocked_task).unwrap();
    assert_eq!(
        app.pid_backup_browser
            .index()
            .map(|idx| idx.backup_root.as_path()),
        Some(pkg_dir.as_path()),
        "running export must block switching to another backup package"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
