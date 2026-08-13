//! The P&ID property group's captions survive translation.
//!
//! `t!()` looks a caption up in `locale_catalog` and hands the Fluent id to
//! the loader. A caption missing from the catalog silently stays English,
//! and a catalog id missing from a `.ftl` renders as the raw id -- neither
//! failure shows up in a build, a lint, or an import test. This walks every
//! shipped language so the group cannot go half-translated unnoticed.

#![cfg(not(target_arch = "wasm32"))]

use OpenCADStudio::i18n::{set_language, translate, Language};

#[test]
fn pid_panel_captions_translate_in_every_shipped_language() {
    let captions = ["Type", "Item tag", "Line number", "Matched by"];
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
    }
}
