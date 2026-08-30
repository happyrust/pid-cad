//! Evidence probe for placement-time text assignment.

use pid_parse::symbol_library::{SymbolLibrary, SymbolPrimitive};
use pid_parse::{build_normalized_geometry, PidGraphicKind, PidParser, PidSemanticIndex};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let pid = std::path::PathBuf::from(args.next().expect("pid path"));
    let symbols = std::path::PathBuf::from(args.next().expect("symbol-library root"));
    let parsed = PidParser::new().parse_file(&pid).expect("parse pid");
    let geometry = build_normalized_geometry(&parsed);
    let semantics = PidSemanticIndex::load_beside(&pid, &parsed).expect("matching _Data.xml");
    let mut library = SymbolLibrary::new(symbols);

    let mut assigned = 0usize;
    for entity in &geometry.entities {
        let PidGraphicKind::SymbolInstance { symbol_path, .. } = &entity.kind else {
            continue;
        };
        let Some(path) = symbol_path.as_deref() else { continue };
        let oid = entity.graphic_oid.unwrap_or(0);
        let hit = semantics.resolve(oid);
        let object = hit.as_ref().map(|hit| hit.object());
        let label = object.and_then(|object| object.label());
        let texts: Vec<String> = library
            .resolve(path)
            .into_iter()
            .flat_map(|body| &body.primitives)
            .filter_map(|styled| match &styled.primitive {
                SymbolPrimitive::Text { text, .. } if text.contains("NULL") => Some(text.clone()),
                _ => None,
            })
            .collect();
        if label.is_some() && !texts.is_empty() {
            assigned += 1;
        }
        println!(
            "oid={oid:<6} class={:<24} label={:?} null_templates={:?} symbol={}",
            object.map_or("<unresolved>", |object| object.class.as_str()),
            label,
            texts,
            path.rsplit(['\\', '/']).next().unwrap_or(path)
        );
    }
    println!("assignment_candidates={assigned}");
}
