//! The P&ID property group's captions survive translation.
//!
//! `t!()` looks a caption up in `locale_catalog` and hands the Fluent id to
//! the loader. A caption missing from the catalog silently stays English,
//! and a catalog id missing from a `.ftl` renders as the raw id -- neither
//! failure shows up in a build, a lint, or an import test. This walks every
//! shipped language so the group cannot go half-translated unnoticed.

#![cfg(not(target_arch = "wasm32"))]

use document::entities::Line;
use document::xdata::{ExtendedDataRecord, XDataValue};
use document::EntityType;
use OpenCADStudio::i18n::{set_language, translate, Language};
use OpenCADStudio::scene::cache::properties::pid_semantics_section;

/// The caption the panel gives the row `field` of an entity whose
/// `PID_SEMANTICS` record says `values`.
fn panel_caption(values: &[&str], field: &str) -> String {
    let mut entity = EntityType::Line(Line::new());
    let mut record = ExtendedDataRecord::new("PID_SEMANTICS");
    for value in values {
        record.add_value(XDataValue::String((*value).to_string()));
    }
    entity.common_mut().extended_data.add_record(record);
    pid_semantics_section(&entity)
        .and_then(|group| group.props.into_iter().find(|row| row.field == field))
        .unwrap_or_else(|| panic!("the panel shows no {field} row"))
        .label
}

#[test]
fn pid_panel_captions_translate_in_every_shipped_language() {
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
