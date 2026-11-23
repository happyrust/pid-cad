//! What a file of the backup says, when it is text.
//!
//! A backup stores its plant's rules, insulation specification, data maps
//! and export log as files the tree could only name. A click on a file --
//! one of the backup folder, a member of one of its archives, or one of the
//! `Ref` tree -- now reads it off the UI thread
//! ([`crate::io::pid_backup_browse::text_file`]) and shows it under the
//! file's own details: the encoding it was read in, how many lines it has,
//! and the lines themselves in a grid that scrolls both ways. A file that is
//! not text says so.
//!
//! `Manifest.txt` is not shown this way: its content is the manifest's own
//! branch, where its credentials are hidden field by field. The dump is not
//! text, and an archive is shown by its members. A symbol file is never
//! text, and is shown as what it draws ([`super::symbol_preview`]).
//!
//! The text of a file can be megabytes, so only the selected file's is kept
//! ([`Section::keeps_only_selected`]).

use std::path::PathBuf;
use std::sync::Arc;

use super::grid;
use super::section::{Cx, Event, EventKind, Section, SectionData, SectionLoad};
use super::{human_size, DetailLine, NodeId};
use crate::io::pid_backup_browse::manifest::HIDDEN;
use crate::io::pid_backup_browse::text_file::{
    read_text_file, read_zip_member, Readable, TextFile,
};
use crate::io::pid_backup_browse::{BackupIndex, FileKind};
use crate::{t, tf};

/// The section: it hangs no rows, and adds the text to the details of the
/// file nodes the tree already has.
pub(super) struct TextPreview;

/// What was read of a file.
enum Preview {
    /// Text: its lines as the grid shows them, and what the read found out.
    Text {
        table: Arc<grid::Table>,
        encoding: &'static str,
        line_count: usize,
        bytes: u64,
        cut_at: Option<u64>,
        hidden: usize,
    },
    /// Not text.
    Binary,
}

/// Where the content of a node is.
pub(super) enum Source {
    /// A file on disk.
    File(PathBuf),
    /// A member of a zip on disk: its position and its name.
    Member {
        archive: PathBuf,
        member: usize,
        name: String,
    },
}

/// Where the text of `id` is, when `id` is a file whose content is its own
/// to show and is not a symbol's.
fn source(index: &BackupIndex, id: &NodeId) -> Option<Source> {
    if super::symbol_preview::is_symbol(index, id) {
        None
    } else {
        content(index, id)
    }
}

/// Where the content of `id` is, when `id` is a file whose content is its
/// own to show.
pub(super) fn content(index: &BackupIndex, id: &NodeId) -> Option<Source> {
    match id {
        NodeId::File(at) => {
            let file = index.files.get(*at)?;
            let own = !file.missing
                && !matches!(file.kind, FileKind::Manifest | FileKind::ExportDump)
                && file.members.is_empty()
                && !file.format.to_ascii_lowercase().starts_with("zip");
            own.then(|| Source::File(index.backup_root.join(&file.name)))
        }
        NodeId::FileMember(file, member) => {
            let file = index.files.get(*file)?;
            let name = file.members.get(*member)?;
            // A folder of the archive is an entry too, with nothing in it.
            (!name.ends_with(['/', '\\'])).then(|| Source::Member {
                archive: index.backup_root.join(&file.name),
                member: *member,
                name: name.clone(),
            })
        }
        NodeId::RefFile(dirs, at) => index.ref_file_path(dirs, *at).map(Source::File),
        _ => None,
    }
}

/// The lines of `file` as a table: the line's number, and its text.
fn preview(file: TextFile) -> Preview {
    let rows = file
        .rows
        .into_iter()
        .map(|(number, text)| {
            vec![
                number.map(|number| number.to_string()).unwrap_or_default(),
                text,
            ]
        })
        .collect();
    let table = grid::Table::new(
        vec![
            grid::Column::number(t!("Line #")),
            grid::Column::text(t!("Text content")),
        ],
        rows,
    )
    .in_file_order();
    Preview::Text {
        table: Arc::new(table),
        encoding: file.encoding,
        line_count: file.line_count,
        bytes: file.bytes,
        cut_at: file.cut_at,
        hidden: file.hidden,
    }
}

impl Section for TextPreview {
    fn details(&self, cx: &Cx<'_>, id: &NodeId, lines: &mut Vec<DetailLine>) {
        let marker = |text: std::borrow::Cow<'_, str>| DetailLine::Marker(text.into_owned());
        match cx.load(id) {
            None => {}
            Some(SectionLoad::Loading) => lines.push(marker(t!("Reading the file…"))),
            Some(SectionLoad::Failed(error)) => lines.push(DetailLine::Marker(error.clone())),
            Some(SectionLoad::Ready(data)) => match data.get::<Preview>() {
                None => {}
                Some(Preview::Binary) => {
                    lines.push(marker(t!("Not a text file; its content is not shown.")));
                }
                Some(Preview::Text {
                    table,
                    encoding,
                    line_count,
                    bytes,
                    cut_at,
                    hidden,
                }) => {
                    lines.push(DetailLine::Field(
                        t!("Text encoding").into_owned(),
                        (*encoding).to_string(),
                    ));
                    lines.push(DetailLine::Field(
                        t!("Line count").into_owned(),
                        line_count.to_string(),
                    ));
                    if let Some(cut_at) = cut_at {
                        lines.push(marker(tf!(
                            "Only the first {} of {} is shown.",
                            human_size(*cut_at),
                            human_size(*bytes)
                        )));
                    }
                    if *hidden > 0 {
                        lines.push(marker(tf!(
                            "{} credential-like value(s) in the text are hidden and shown as {}",
                            hidden,
                            HIDDEN
                        )));
                    }
                    if !table.rows().is_empty() {
                        lines.push(DetailLine::Table(Arc::clone(table)));
                    }
                }
            },
        }
    }

    fn wants(&self, cx: &Cx<'_>, event: &Event<'_>) -> Option<NodeId> {
        (event.kind == EventKind::Selected && source(cx.index, event.node).is_some())
            .then(|| event.node.clone())
    }

    fn load(&self, index: &BackupIndex, node: &NodeId) -> Result<SectionData, String> {
        let read = match source(index, node) {
            Some(Source::File(path)) => read_text_file(&path)?,
            Some(Source::Member {
                archive,
                member,
                name,
            }) => read_zip_member(&archive, member, &name)?,
            None => return Err(t!("this node has nothing to read").into_owned()),
        };
        Ok(SectionData::new(match read {
            Readable::Text(file) => preview(file),
            Readable::Binary(_) => Preview::Binary,
        }))
    }

    fn keeps_only_selected(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Message;
    use crate::io::pid_backup_browse::build_index;
    use crate::ui::dock::Chrome;
    use crate::ui::window::pid_backup_browser::section::load_in_background;
    use crate::ui::window::pid_backup_browser::{
        detail_rows, update, view, visible_rows, Effect, PidBackupBrowser, PidBackupMsg,
    };
    use std::io::Write;
    use std::path::Path;

    /// What a credential looks like, made up for the tests.
    const OBFUSCATED: &str = "Qz######rt######uv#######wx#######yz######ab######cd######ef####";
    const PASSWORD: &str = "a-made-up-password";

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ocs-pid-backup-preview-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in members {
            if name.ends_with('/') {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    /// A backup folder on disk with every kind of file the preview meets,
    /// indexed the way the application indexes one.
    fn backup(name: &str) -> (PathBuf, PidBackupBrowser) {
        let root = scratch(name);
        let write = |name: &str, bytes: &[u8]| std::fs::write(root.join(name), bytes).unwrap();
        write(
            "Manifest.txt",
            format!(
                "Name<<|>>SQPlant\n\
                 DBPwds<<|>>{OBFUSCATED}\n\
                 BackupCommand<<|>>exp system/{PASSWORD}@orcl file=Export.dmp\n\
                 File<<|>>4<<|>>2<<|>>680<<|>>20680<<|>>\\\\host\\P\\Ref\\rules.rul<<|>>1<<|>>RefData~4~680\n\
                 File<<|>>4<<|>>2<<|>>683<<|>>20683<<|>>\\\\host\\P\\Ref\\ProjectStyles.spp<<|>>1<<|>>RefData~4~683\n\
                 File<<|>>4<<|>>1<<|>>809<<|>>20809<<|>>\\\\host\\P\\Ref\\Maps<<|>>1<<|>>RefData~4~809.zip\n\
                 File<<|>>4<<|>>2<<|>>824<<|>>20824<<|>>\\\\host\\P\\Ref\\Jacket.XML<<|>>2<<|>>Failed to access file.\n"
            )
            .as_bytes(),
        );
        write("Export.dmp", b"\x03\x03iEXPORT:V12.02.00\nrest-of-dump");
        write(
            "Export.log",
            format!(
                "Connected to: Oracle Database\nabout to export SQPLANT's tables via Conventional Path ...\n. . exporting table                      T_DRAWING         53 rows exported\nexp system/{PASSWORD}@orcl\nkey {OBFUSCATED}\n"
            )
            .as_bytes(),
        );
        write(
            "PlantConfig.xml",
            "\u{feff}<PlantConfig>\r\n  <伴热 介质=\"蒸汽\"/>\r\n</PlantConfig>\r\n".as_bytes(),
        );
        write(
            "RefData~4~680",
            b"\"Begin Rules\",2\n\"Rule\",\"Pipe to Nozzle\"\n\"End Rules\"\n",
        );
        let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        ole.extend([0u8; 256]);
        write("RefData~4~683", &ole);
        write_zip(
            &root.join("RefData~4~809.zip"),
            &[
                ("Maps/", b""),
                (
                    "Maps/import.xml",
                    b"<Map>\n  <Row from=\"A\" to=\"B\"/>\n</Map>\n",
                ),
                ("Maps/valve.sym", &ole),
            ],
        );
        std::fs::create_dir_all(root.join("Ref").join("Symbols")).unwrap();
        std::fs::write(
            root.join("Ref").join("Symbols").join("CatalogIndex.xml"),
            b"<Catalog>\n</Catalog>\n",
        )
        .unwrap();
        std::fs::write(root.join("Ref").join("empty.txt"), b"").unwrap();
        let index = build_index(&root, None).expect("the folder indexes");
        let mut state = PidBackupBrowser::default();
        update(&mut state, PidBackupMsg::Loaded(Ok(Arc::new(index))));
        (root, state)
    }

    fn file(state: &PidBackupBrowser, name: &str) -> NodeId {
        let index = state.index().expect("a ready index");
        NodeId::File(
            index
                .files
                .iter()
                .position(|file| file.name == name)
                .unwrap_or_else(|| panic!("{name} is a file of the backup")),
        )
    }

    fn member(state: &PidBackupBrowser, archive: &str, name: &str) -> NodeId {
        let NodeId::File(at) = file(state, archive) else {
            unreachable!()
        };
        let index = state.index().expect("a ready index");
        NodeId::FileMember(
            at,
            index.files[at]
                .members
                .iter()
                .position(|member| member == name)
                .unwrap_or_else(|| panic!("{name} is a member of {archive}")),
        )
    }

    /// Select `node`, run the read it starts the way the application does,
    /// and hand the result back. Returns whether a read was started.
    fn select(state: &mut PidBackupBrowser, node: &NodeId) -> bool {
        match update(state, PidBackupMsg::Select(node.clone())) {
            // The backup's own row has the restore cache measured, which
            // is no read of a file.
            Effect::None | Effect::MeasureCache { .. } => false,
            Effect::LoadSection {
                generation,
                section,
                node: asked,
            } => {
                let index = state.index().cloned().expect("a ready index");
                let result = iced::futures::executor::block_on(load_in_background(
                    state.sections,
                    index,
                    section,
                    asked.clone(),
                ));
                update(
                    state,
                    PidBackupMsg::SectionLoaded {
                        generation,
                        section,
                        node: asked,
                        result,
                    },
                );
                true
            }
            other => panic!("selecting a file starts one read at most: {other:?}"),
        }
    }

    fn value<'a>(rows: &'a [(String, String)], key: &str) -> Option<&'a str> {
        rows.iter()
            .find(|(found, _)| found == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_files_the_export_left_beside_the_backup_are_listed_after_the_dump() {
        let (root, state) = backup("listed");
        let index = state.index().expect("a ready index");
        assert_eq!(
            index
                .files
                .iter()
                .map(|file| (file.name.as_str(), file.kind, file.format.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Manifest.txt", FileKind::Manifest, "Manifest"),
                (
                    "Export.dmp",
                    FileKind::ExportDump,
                    index.files[1].format.as_str()
                ),
                ("Export.log", FileKind::Other, "LOG"),
                ("PlantConfig.xml", FileKind::Other, "XML"),
                ("RefData~4~680", FileKind::RefData, "RUL"),
                ("RefData~4~683", FileKind::RefData, "SPP"),
                ("RefData~4~809.zip", FileKind::RefData, "Zip"),
                (
                    "RefData~4~824",
                    FileKind::RefData,
                    index.files[7].format.as_str()
                ),
            ]
        );
        // They are rows of the tree like the others.
        let mut state = state;
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));
        let labels: Vec<String> = visible_rows(&state)
            .into_iter()
            .filter(|row| row.depth == 2)
            .map(|row| row.label)
            .collect();
        assert_eq!(labels[2..4], ["Export.log", "PlantConfig.xml"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_text_file_shows_its_encoding_its_line_count_and_its_lines() {
        let (root, mut state) = backup("text");
        let rules = file(&state, "RefData~4~680");
        assert!(select(&mut state, &rules));
        let rows = detail_rows(&state, &rules);
        // The file's own details first, then what the read found.
        assert_eq!(
            rows[0],
            (
                t!("Backup file name").into_owned(),
                "RefData~4~680".to_string()
            )
        );
        assert_eq!(value(&rows, &t!("Original path")), Some("Ref\\rules.rul"));
        assert_eq!(value(&rows, &t!("Text encoding")), Some("UTF-8"));
        assert_eq!(value(&rows, &t!("Line count")), Some("3"));
        let table = rows
            .iter()
            .position(|(key, _)| *key == format!("{} | {}", t!("Line #"), t!("Text content")))
            .expect("the lines are a table");
        assert_eq!(
            rows[table + 1..],
            [
                ("1".to_string(), "\"Begin Rules\",2".to_string()),
                ("2".to_string(), "\"Rule\",\"Pipe to Nozzle\"".to_string()),
                ("3".to_string(), "\"End Rules\"".to_string()),
            ]
        );

        // A byte-order mark, CRLF and Chinese: read as what they are.
        let config = file(&state, "PlantConfig.xml");
        assert!(select(&mut state, &config));
        let rows = detail_rows(&state, &config);
        assert_eq!(value(&rows, &t!("Text encoding")), Some("UTF-8"));
        assert_eq!(value(&rows, "2"), Some("  <伴热 介质=\"蒸汽\"/>"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_member_of_an_archive_and_a_reference_file_are_read_where_they_are() {
        let (root, mut state) = backup("places");
        let map = member(&state, "RefData~4~809.zip", "Maps/import.xml");
        assert!(select(&mut state, &map));
        let rows = detail_rows(&state, &map);
        assert_eq!(value(&rows, &t!("Line count")), Some("3"));
        assert_eq!(value(&rows, "2"), Some("  <Row from=\"A\" to=\"B\"/>"));

        // `Ref/Symbols/CatalogIndex.xml`: the first directory, its first file.
        let catalog = NodeId::RefFile(vec![0], 0);
        assert!(select(&mut state, &catalog));
        let rows = detail_rows(&state, &catalog);
        assert_eq!(
            value(&rows, &t!("Reference file name")),
            Some("CatalogIndex.xml")
        );
        assert_eq!(value(&rows, "1"), Some("<Catalog>"));

        // An empty file is text with no line, and no table.
        let empty = NodeId::RefFile(vec![], 0);
        assert!(select(&mut state, &empty));
        let rows = detail_rows(&state, &empty);
        assert_eq!(value(&rows, &t!("Line count")), Some("0"));
        assert!(!rows
            .iter()
            .any(|(key, _)| key.contains(t!("Text content").as_ref())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_is_not_text_says_so() {
        let (root, mut state) = backup("binary");
        let not_text = (
            t!("Not a text file; its content is not shown.").into_owned(),
            String::new(),
        );
        let styles = file(&state, "RefData~4~683");
        assert!(select(&mut state, &styles));
        let rows = detail_rows(&state, &styles);
        assert!(rows.contains(&not_text), "{rows:?}");
        assert_eq!(value(&rows, &t!("Text encoding")), None);

        // A symbol file is read once, as what it draws: it is never text,
        // and does not say that it is none.
        let symbol = member(&state, "RefData~4~809.zip", "Maps/valve.sym");
        assert!(select(&mut state, &symbol));
        let rows = detail_rows(&state, &symbol);
        assert!(!rows.contains(&not_text), "{rows:?}");
        assert_eq!(value(&rows, &t!("Text encoding")), None);
        // What the symbol preview says of it is a plain line of its own.
        assert!(rows.iter().any(|(_, value)| value.is_empty()), "{rows:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_has_no_text_of_its_own_is_not_read() {
        let (root, mut state) = backup("not-read");
        for node in [
            // Its content is the manifest's branch; the dump is not text; an
            // archive is its members; a file the backup never got is absent;
            // a folder of an archive holds nothing.
            file(&state, "Manifest.txt"),
            file(&state, "Export.dmp"),
            file(&state, "RefData~4~809.zip"),
            file(&state, "RefData~4~824"),
            member(&state, "RefData~4~809.zip", "Maps/"),
            NodeId::Files,
            NodeId::RefDir(vec![0]),
            NodeId::Root,
        ] {
            assert!(!select(&mut state, &node), "{node:?}");
            let rows = detail_rows(&state, &node);
            assert_eq!(value(&rows, &t!("Text encoding")), None, "{node:?}");
        }
        assert!(state.section_data.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_cannot_be_read_says_why_and_the_read_is_shown_while_it_runs() {
        let (root, mut state) = backup("failed");
        let log = file(&state, "Export.log");
        // While the read is in flight the details say so.
        assert!(matches!(
            update(&mut state, PidBackupMsg::Select(log.clone())),
            Effect::LoadSection { .. }
        ));
        assert!(detail_rows(&state, &log)
            .contains(&(t!("Reading the file…").into_owned(), String::new())));

        // The file went away after the backup was indexed.
        std::fs::remove_file(root.join("PlantConfig.xml")).unwrap();
        let config = file(&state, "PlantConfig.xml");
        assert!(select(&mut state, &config));
        let rows = detail_rows(&state, &config);
        let reason = rows
            .iter()
            .find(|(key, value)| key.contains("PlantConfig.xml") && value.is_empty())
            .unwrap_or_else(|| panic!("the reason is shown: {rows:?}"));
        assert!(reason.0.contains(&root.display().to_string()), "{reason:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_selected_files_text_is_kept() {
        let (root, mut state) = backup("kept");
        let rules = file(&state, "RefData~4~680");
        let log = file(&state, "Export.log");
        assert!(select(&mut state, &rules));
        assert!(select(&mut state, &log));
        assert_eq!(state.section_data.len(), 1);
        assert_eq!(
            value(&detail_rows(&state, &rules), &t!("Text encoding")),
            None
        );
        assert!(value(&detail_rows(&state, &log), &t!("Text encoding")).is_some());
        // Back on the first file: read again.
        assert!(select(&mut state, &rules));
        assert_eq!(
            value(&detail_rows(&state, &rules), &t!("Line count")),
            Some("3")
        );
        // A node with no text lets go of the last one.
        assert!(!select(&mut state, &NodeId::Root));
        assert!(state.section_data.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The export log of the test backup carries what a careless export
    /// could leave in one: the command with its login, and an obfuscated
    /// value. Neither reaches the details or the panel's state.
    #[test]
    fn a_credential_in_a_text_file_is_not_shown() {
        let (root, mut state) = backup("credentials");
        let log = file(&state, "Export.log");
        assert!(select(&mut state, &log));
        let rows = detail_rows(&state, &log);
        assert_eq!(
            value(&rows, "4"),
            Some(format!("exp system/{HIDDEN}@orcl").as_str())
        );
        assert_eq!(value(&rows, "5"), Some(format!("key {HIDDEN}").as_str()));
        assert!(rows.contains(&(
            tf!(
                "{} credential-like value(s) in the text are hidden and shown as {}",
                2,
                HIDDEN
            )
            .into_owned(),
            String::new()
        )));
        for text in [format!("{rows:?}"), format!("{state:?}")] {
            assert!(!text.contains(PASSWORD) && !text.contains(OBFUSCATED));
        }
        // The manifest holds both in earnest, and is never read as text.
        let manifest = file(&state, "Manifest.txt");
        assert!(!select(&mut state, &manifest));
        let rows = format!("{:?}", detail_rows(&state, &manifest));
        assert!(!rows.contains(PASSWORD) && !rows.contains(OBFUSCATED));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A backup with no `Ref` folder beside it, as SmartPlant leaves one:
    /// its reference data volumes are the reference tree. The arrow of
    /// "Reference data" is where a press lands, and a click on a file of the
    /// tree reads it.
    #[test]
    fn the_volumes_of_a_backup_are_its_reference_tree_drawn_and_clicked() {
        let _guard = crate::headless_ui_lock();
        let root = scratch("volumes");
        let backup = root.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "Name<<|>>SQPlant\r\n\
             File<<|>>2<<|>>1<<|>>711<<|>>1<<|>>\\\\host\\P\\Plant<<|>>1<<|>>PlantData~2~711.zip\r\n\
             File<<|>>4<<|>>1<<|>>681<<|>>1<<|>>\\\\host\\P\\Ref\\Symbols<<|>>1<<|>>RefData~4~681.zip\r\n\
             File<<|>>4<<|>>1<<|>>682<<|>>1<<|>>\\\\host\\P\\Ref\\Template Files<<|>>1<<|>>RefData~4~682.zip\r\n\
             File<<|>>4<<|>>2<<|>>680<<|>>1<<|>>\\\\host\\P\\Ref\\rules.rul<<|>>1<<|>>RefData~4~680\r\n",
        )
        .unwrap();
        write_zip(&backup.join("PlantData~2~711.zip"), &[("00/00/A.pid", b"a")]);
        write_zip(
            &backup.join("RefData~4~681.zip"),
            &[("Piping/gate.sym", &[0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0])],
        );
        write_zip(&backup.join("RefData~4~682.zip"), &[("A1.igr", &[0xD0, 0xCF, 0, 0])]);
        std::fs::write(
            backup.join("RefData~4~680"),
            b"\"Begin Rules\",2\n\"End Rules\"\n",
        )
        .unwrap();
        let index = build_index(&backup, Some(&root.join("out"))).expect("the folder indexes");
        let mut state = PidBackupBrowser::default();
        update(&mut state, PidBackupMsg::Loaded(Ok(Arc::new(index))));

        // The details of the group say where the tree was read from.
        let rows = detail_rows(&state, &NodeId::Ref);
        assert_eq!(value(&rows, &t!("Reference file count")), Some("3"));
        assert_eq!(
            value(&rows, &t!("Reference folder")),
            Some(root.join("out").join("Ref").display().to_string().as_str())
        );

        fn press(state: &PidBackupBrowser, label: &str, left_of: bool) -> Vec<PidBackupMsg> {
            let mut ui = iced_test::simulator::Simulator::with_size(
                iced::Settings::default(),
                (340.0, 700.0),
                view(state, 340.0, Chrome::default()),
            );
            let bounds = ui
                .find(label)
                .unwrap_or_else(|error| panic!("the row of {label} is drawn: {error:?}"))
                .visible_bounds()
                .unwrap_or_else(|| panic!("the row of {label} is on screen"));
            let x = if left_of {
                bounds.x - 7.0
            } else {
                bounds.center_x()
            };
            ui.point_at(iced::Point::new(x, bounds.center_y()));
            let _ = ui.simulate(iced_test::simulator::click());
            ui.into_messages()
                .filter_map(|message| match message {
                    Message::PidBackup(message) => Some(message),
                    _ => None,
                })
                .collect()
        }

        let opened = press(&state, t!("Reference data").as_ref(), true);
        assert!(
            opened
                .iter()
                .any(|message| matches!(message, PidBackupMsg::Toggle(NodeId::Ref))),
            "{opened:?}"
        );
        for message in opened {
            update(&mut state, message);
        }
        let labels: Vec<String> = visible_rows(&state)
            .into_iter()
            .filter(|row| matches!(row.id, NodeId::RefDir(_) | NodeId::RefFile(..)))
            .map(|row| row.label)
            .collect();
        assert_eq!(labels, ["Symbols", "Template Files", "rules.rul"]);

        let clicked = press(&state, "rules.rul", false);
        let rules = NodeId::RefFile(vec![], 0);
        assert!(
            clicked
                .iter()
                .any(|message| matches!(message, PidBackupMsg::Select(node) if *node == rules)),
            "{clicked:?}"
        );
        assert!(select(&mut state, &rules));
        let rows = detail_rows(&state, &rules);
        assert_eq!(value(&rows, "1"), Some("\"Begin Rules\",2"));
        assert_eq!(value(&rows, &t!("Line count")), Some("2"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a manifest's credentials look like in the panel: the row of
    /// `Manifest.txt` itself is never read as text (its content is the
    /// branch under it, hidden field by field), and a copy of the file is a
    /// file like any other wherever it lies -- beside the backup under
    /// another name, inside an archive, among the reference files. None of
    /// them, and no settings file with a password in it, shows a credential:
    /// not in the details, not in what the panel keeps.
    #[test]
    fn the_manifest_its_copies_and_a_settings_file_show_no_credential() {
        const SECRETS: [&str; 6] = [
            "copy-site-secret",
            "copy-plant-secret",
            "copy-dbpwds-secret",
            "copy-command-secret",
            "ini-secret",
            "xml-secret",
        ];
        let manifest = |line_end: &str| -> Vec<u8> {
            let text = [
                "Name<<|>>SQPlant",
                "SiteConnInfo<<|>>copy-site-secret<<|>>1<<|>>ORCL",
                "PlantConnInfo<<|>>SQPlant<<|>>2<<|>>ORCL<<|>>SQPlant<<|>>copy-plant-secret",
                "DBPwds<<|>>copy-dbpwds-secret",
                "BackupCommand<<|>>Exp.exe system/copy-command-secret@ORCL FILE=Export.dmp",
                "",
            ]
            .join(line_end);
            // As SmartPlant writes a manifest: UTF-16 with a byte-order mark.
            let mut bytes = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                bytes.extend(unit.to_le_bytes());
            }
            bytes
        };
        let root = scratch("manifest-copies");
        std::fs::write(root.join("Manifest.txt"), manifest("\r\n")).unwrap();
        std::fs::write(root.join("Manifest - copy.txt"), manifest("\r\n")).unwrap();
        // The same lines with nothing but carriage returns between them.
        std::fs::write(root.join("Manifest.mac.txt"), manifest("\r")).unwrap();
        std::fs::write(
            root.join("Connection.ini"),
            b"[Database]\r\nUser=plant\r\nPassword=ini-secret\r\n",
        )
        .unwrap();
        write_zip(
            &root.join("RefData~4~900.zip"),
            &[
                ("Old/Manifest.txt", &manifest("\n")),
                (
                    "Old/site.config",
                    b"<add key=\"SitePassword\" value=\"xml-secret\"/>\n",
                ),
            ],
        );
        std::fs::create_dir_all(root.join("Ref")).unwrap();
        std::fs::write(root.join("Ref").join("Manifest.bak"), manifest("\r\n")).unwrap();
        let index = build_index(&root, None).expect("the folder indexes");
        let mut state = PidBackupBrowser::default();
        update(&mut state, PidBackupMsg::Loaded(Ok(Arc::new(index))));

        // The manifest's own row: nothing is read, and its details are the
        // file's alone.
        let own = file(&state, "Manifest.txt");
        assert!(!select(&mut state, &own));
        assert_eq!(
            value(&detail_rows(&state, &own), &t!("Text encoding")),
            None
        );

        let places = [
            (
                "a copy beside the backup",
                file(&state, "Manifest - copy.txt"),
            ),
            (
                "a copy with carriage returns alone",
                file(&state, "Manifest.mac.txt"),
            ),
            ("a settings file", file(&state, "Connection.ini")),
            (
                "a copy inside an archive",
                member(&state, "RefData~4~900.zip", "Old/Manifest.txt"),
            ),
            (
                "a configuration inside an archive",
                member(&state, "RefData~4~900.zip", "Old/site.config"),
            ),
            (
                "a copy among the reference files",
                NodeId::RefFile(vec![], 0),
            ),
        ];
        for (what, node) in &places {
            assert!(select(&mut state, node), "{what} is read as text");
            let rows = detail_rows(&state, node);
            assert!(
                value(&rows, &t!("Text encoding")).is_some(),
                "{what} is shown as text: {} rows",
                rows.len()
            );
            // Where a credential shows is said; the credential is not.
            for (at, secret) in SECRETS.iter().enumerate() {
                assert!(
                    !format!("{rows:?}").contains(secret),
                    "{what}: credential {at} of SECRETS is in the details"
                );
                assert!(
                    !format!("{state:?}").contains(secret),
                    "{what}: credential {at} of SECRETS is in the panel's state"
                );
            }
            // And the panel says that it hid something.
            assert!(
                rows.iter()
                    .any(|(key, value)| value.is_empty() && key.contains(HIDDEN)),
                "{what}: the details do not say that anything was hidden"
            );
        }
        // What is not a credential is still there to read.
        let copy = &places[0].1;
        assert!(select(&mut state, copy));
        let rows = detail_rows(&state, copy);
        assert_eq!(value(&rows, "1"), Some("Name<<|>>SQPlant"));
        assert_eq!(
            value(&rows, "5"),
            Some(
                format!("BackupCommand<<|>>Exp.exe system/{HIDDEN}@ORCL FILE=Export.dmp").as_str()
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The preview as the panel draws it: the row of a file is where a press
    /// lands, and the lines of the file it selects are drawn under its
    /// details, the header of the grid above them.
    #[test]
    fn a_click_on_a_drawn_file_row_selects_it_and_its_lines_are_drawn() {
        let _guard = crate::headless_ui_lock();
        let (root, mut state) = backup("drawn");
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));

        fn draw(state: &PidBackupBrowser) -> iced_test::simulator::Simulator<'_, Message> {
            iced_test::simulator::Simulator::with_size(
                iced::Settings::default(),
                (340.0, 700.0),
                view(state, 340.0, Chrome::default()),
            )
        }
        let mut ui = draw(&state);
        let row = ui
            .find("RefData~4~680 — Ref\\rules.rul")
            .expect("the row of the rules file is drawn")
            .visible_bounds()
            .expect("the row is on screen");
        ui.point_at(row.center());
        let _ = ui.simulate(iced_test::simulator::click());
        let clicked: Vec<PidBackupMsg> = ui
            .into_messages()
            .filter_map(|message| match message {
                Message::PidBackup(message) => Some(message),
                _ => None,
            })
            .collect();
        let rules = file(&state, "RefData~4~680");
        assert!(
            clicked
                .iter()
                .any(|message| matches!(message, PidBackupMsg::Select(node) if *node == rules)),
            "{clicked:?}"
        );
        assert!(select(&mut state, &rules));

        let mut ui = draw(&state);
        let top = |ui: &mut iced_test::simulator::Simulator<'_, Message>, label: &str| -> f32 {
            ui.find(label)
                .unwrap_or_else(|error| panic!("{label} is drawn: {error:?}"))
                .visible_bounds()
                .unwrap_or_else(|| panic!("{label} is on screen"))
                .y
        };
        let encoding = top(&mut ui, t!("Text encoding").as_ref());
        let header = top(&mut ui, t!("Text content").as_ref());
        let first = top(&mut ui, "\"Begin Rules\",2");
        let last = top(&mut ui, "\"End Rules\"");
        assert!(encoding < header && header < first && first < last);
        // The lines keep the height of a grid row: 1, 2, 3 in order.
        assert_eq!(last - first, 2.0 * grid::ROW_H);
        let _ = std::fs::remove_dir_all(&root);
    }
}
