//! The P&ID property group's captions survive translation.
//!
//! `t!()` looks a caption up in `locale_catalog` and hands the Fluent id to
//! the loader. A caption missing from the catalog silently stays English,
//! and a catalog id missing from a `.ftl` renders as the raw id -- neither
//! failure shows up in a build, a lint, or an import test. This walks every
//! shipped language so the group cannot go half-translated unnoticed.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Mutex, MutexGuard};

use document::entities::{Insert, Line, Text};
use document::types::Vector3;
use document::xdata::{ExtendedDataRecord, XDataValue};
use document::{Document, EntityType};
use OpenCADStudio::i18n::{set_language, translate, Language};
use OpenCADStudio::scene::cache::properties::{add_pid_pairing_row, pid_semantics_section};
use OpenCADStudio::scene::model::object::{PropSection, PropValue};

/// The language is the whole process's, and each test of this file walks
/// through several: one test at a time.
fn one_test_at_a_time() -> MutexGuard<'static, ()> {
    static LANGUAGE: Mutex<()> = Mutex::new(());
    LANGUAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `entity` with `values` as its `PID_SEMANTICS` record.
fn saying(mut entity: EntityType, values: &[&str]) -> EntityType {
    let mut record = ExtendedDataRecord::new("PID_SEMANTICS");
    for value in values {
        record.add_value(XDataValue::String((*value).to_string()));
    }
    entity.common_mut().extended_data.add_record(record);
    entity
}

/// The caption the panel gives the row `field` of an entity whose
/// `PID_SEMANTICS` record says `values`.
fn panel_caption(values: &[&str], field: &str) -> String {
    let entity = saying(EntityType::Line(Line::new()), values);
    pid_semantics_section(&entity)
        .and_then(|group| group.props.into_iter().find(|row| row.field == field))
        .unwrap_or_else(|| panic!("the panel shows no {field} row"))
        .label
}

/// The row `field` of a group as `caption = what it reads`.
fn panel_row(group: &PropSection, field: &str) -> String {
    let row = group
        .props
        .iter()
        .find(|row| row.field == field)
        .unwrap_or_else(|| panic!("the panel shows no {field} row"));
    match &row.value {
        PropValue::ReadOnly(read) => format!("{} = {read}", row.label),
        other => panic!("{field} is not read-only: {other:?}"),
    }
}

/// The row `field` of the `P&ID` group of the `at`-th entity of a sheet,
/// as the page of one selected object shows it.
fn sheet_row(entities: &[EntityType], at: usize, field: &str) -> String {
    let mut doc = Document::new();
    let filed: Vec<_> = entities
        .iter()
        .map(|entity| {
            doc.add_entity(entity.clone())
                .expect("model space takes the entity")
        })
        .collect();
    let entity = doc.get_entity(filed[at]).expect("filed on the sheet");
    let mut sections: Vec<PropSection> = pid_semantics_section(entity).into_iter().collect();
    add_pid_pairing_row(&mut sections, &doc, entity);
    let group = sections
        .iter()
        .find(|section| section.title == "P&ID")
        .expect("a P&ID group");
    panel_row(group, field)
}

#[test]
fn pid_panel_captions_translate_in_every_shipped_language() {
    let _one_at_a_time = one_test_at_a_time();
    let captions = [
        "Type",
        "Item tag",
        "Line number",
        "Matched by",
        "Sheet layer",
        "Layer OID",
        "Symbol",
    ];
    for language in [
        Language::EnUs,
        Language::ZhCn,
        Language::DeDe,
        Language::FrFr,
        Language::HiIn,
        Language::NlNl,
        Language::RuRu,
        Language::TrTr,
        Language::ArSa,
        Language::EsEs,
        Language::JaJp,
        Language::PtBr,
    ] {
        set_language(language).expect("every shipped language loads");
        for caption in captions {
            let rendered = translate(caption);
            assert!(
                !rendered.starts_with("catalog-"),
                "{language:?}: {caption} rendered as a raw message id ({rendered}), \
                 so its catalog entry has no line in that language's .ftl"
            );
            assert!(
                !rendered.is_empty(),
                "{language:?}: {caption} rendered empty"
            );
        }
        if language != Language::EnUs {
            assert_ne!(
                translate("Item tag"),
                "Item tag",
                "{language:?}: the P&ID tag caption is not in the catalog, so it \
                 falls through to English while its neighbours translate"
            );
        }
        if language == Language::ZhCn {
            assert_eq!(translate("Sheet layer"), "图纸图层");
            assert_eq!(translate("Layer OID"), "图层 OID");
            assert_eq!(translate("Symbol"), "符号");
            // The row the panel builds is captioned through the catalog too.
            assert_eq!(
                panel_caption(&["role=symbol", "symbol=Ball Valve"], "pid_symbol"),
                "符号"
            );
        }
    }
}

/// What the panel says of an imported symbol and of its labels (plan
/// 2026-11-20-pid-symbol-identity, SI-3; SPEC section 4.5) is said in every
/// language the program ships.
///
/// A sentence missing from the catalog stays English in every language, and
/// one whose catalog entry has no line in a language's `.ftl` falls back to
/// English in that language; either way the panel goes half-translated and
/// nothing else notices. So each sentence added for these rows must read
/// otherwise than its English in every other language -- but for Dutch,
/// which spells `Labels` as English does. The sentences these rows share
/// with other parts of the program are checked for being there. In Chinese
/// every one of them is spelt out, and the rows the panel builds read so.
#[test]
fn what_the_panel_says_of_a_symbol_and_its_labels_is_said_in_every_shipped_language() {
    let _one_at_a_time = one_test_at_a_time();
    let added = [
        "Name in file",
        "Labels",
        "Label of",
        "Backup database",
        "Item in the drawing file",
        "A line",
        "Another object",
        "Not on the sheet",
    ];
    let shared = ["Item type", "Tag source", "Drawing file"];
    let spelt_as_english = [(Language::NlNl, "Labels")];

    let languages: Vec<Language> = Language::ALL
        .into_iter()
        .filter(|language| *language != Language::System)
        .collect();
    assert_eq!(languages.len(), 23, "every shipped language");
    for language in languages {
        set_language(language).expect("every shipped language loads");
        for sentence in added.iter().chain(&shared) {
            let rendered = translate(sentence);
            assert!(
                !rendered.trim().is_empty() && !rendered.starts_with("No localization"),
                "{language:?}: {sentence:?} renders as {rendered:?}"
            );
        }
        for sentence in added {
            let rendered = translate(sentence);
            if language == Language::EnUs || spelt_as_english.contains(&(language, sentence)) {
                assert_eq!(rendered, sentence, "{language:?}");
            } else {
                assert_ne!(
                    rendered, sentence,
                    "{language:?}: {sentence:?} is not translated"
                );
            }
        }
    }

    let spelt = |language: Language, sentences: &[&str]| -> Vec<String> {
        set_language(language).expect("every shipped language loads");
        sentences
            .iter()
            .map(|sentence| translate(sentence).into_owned())
            .collect()
    };
    assert_eq!(
        spelt(Language::ZhCn, &added),
        [
            "图纸文件里的名字",
            "标注",
            "标注属于",
            "备份数据库",
            "图纸文件里的对象",
            "一条管线",
            "其他对象",
            "不在图上",
        ]
    );
    assert_eq!(
        spelt(Language::ZhCn, &shared),
        ["图元类型", "位号来源", "图纸文件"]
    );
    assert_eq!(
        spelt(Language::ZhTw, &added),
        [
            "圖面檔案裡的名稱",
            "標註",
            "標註屬於",
            "備份資料庫",
            "圖面檔案裡的物件",
            "一條管線",
            "其他物件",
            "不在圖面上",
        ]
    );

    // The rows the panel builds are captioned, and read, through the
    // catalog.
    set_language(Language::ZhCn).expect("every shipped language loads");
    let tagged = |source: &str| {
        let source = format!("tag_source={source}");
        let entity = saying(
            EntityType::Line(Line::new()),
            &[
                "role=symbol",
                "label=GV-101A",
                "resolved=item",
                "item_type=PipingComp",
                source.as_str(),
                "label_in_file=GV-101",
            ],
        );
        pid_semantics_section(&entity).expect("a P&ID group")
    };
    let by_the_database = tagged("database");
    assert_eq!(
        [
            panel_row(&by_the_database, "pid_class"),
            panel_row(&by_the_database, "pid_item_type"),
            panel_row(&by_the_database, "pid_label"),
            panel_row(&by_the_database, "pid_label_source"),
            panel_row(&by_the_database, "pid_label_in_file"),
            panel_row(&by_the_database, "pid_resolved"),
        ],
        [
            "类型 = PipingComp",
            "图元类型 = PipingComp",
            "位号 = GV-101A",
            "位号来源 = 备份数据库",
            "图纸文件里的名字 = GV-101",
            "匹配方式 = 图纸文件里的对象",
        ]
    );
    assert_eq!(
        panel_row(&tagged("file"), "pid_label_source"),
        "位号来源 = 图纸文件"
    );

    let reference = saying(
        EntityType::Insert(Insert::new("SPPID_Ball Valve", Vector3::new(0.0, 0.0, 0.0))),
        &["role=symbol", "symbol=Ball Valve", "graphic=612"],
    );
    let label = |graphic: &str, on: &str| {
        let mut text = Text::new();
        text.value = "DN50".to_string();
        let of = format!("label_of={graphic}");
        let on = format!("label_on={on}");
        saying(
            EntityType::Text(text),
            &["role=text", of.as_str(), on.as_str()],
        )
    };
    let sheet = [
        reference,
        label("612", "symbol"),
        label("700", "symbol"),
        label("704", "line"),
        label("6", "other"),
    ];
    assert_eq!(
        [
            sheet_row(&sheet, 0, "pid_labels"),
            sheet_row(&sheet, 1, "pid_label_of"),
            sheet_row(&sheet, 2, "pid_label_of"),
            sheet_row(&sheet, 3, "pid_label_of"),
            sheet_row(&sheet, 4, "pid_label_of"),
        ],
        [
            "标注 = 1: DN50",
            "标注属于 = Ball Valve",
            "标注属于 = 不在图上",
            "标注属于 = 一条管线",
            "标注属于 = 其他对象",
        ]
    );
}
