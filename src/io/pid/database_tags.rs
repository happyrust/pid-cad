// The tags a backup's plant database gives the items a sheet's symbols stand for, put on a sheet that is already imported.

use pid_parse::publish::oracle_exp::backup_uid_key;

use super::*;

/// What putting the database's tags on a sheet came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DatabaseTagFill {
    /// Symbols that carry the tag the database gives their item.
    pub(crate) tagged: usize,
    /// Of those, the symbols the drawing file names otherwise: the file's
    /// name is kept beside the tag (`label_in_file`).
    pub(crate) differing: usize,
    /// Symbols whose record was written. None on a sheet that said all of
    /// it already.
    pub(crate) changed: usize,
}

/// Put the tags a backup's plant database gives on the symbols of an
/// imported sheet (plan 2026-11-20-pid-symbol-identity, SPEC sections 4.2
/// and 4.4).
///
/// `tags` is the database's `T_PlantItem.ItemTag` by the id of the item,
/// the id folded as the dump folds it
/// (`pid_parse::publish::oracle_exp::backup_uid_key`). A symbol is a block
/// reference of model space that says `role=symbol`. The ones `drawn_here`
/// answers for are passed over: what was drawn in this program is no item
/// of that database.
///
/// A symbol whose item the database tags says the tag as its `label`,
/// marked `tag_source=database`. A tag is taken without the blanks around
/// it, and a blank one is none. What the drawing file itself calls the item
/// -- the `label` an import marked `tag_source=file`, or the
/// `label_in_file` an earlier fill kept -- stays beside the tag as
/// `label_in_file` where it is another name, and is not stated where it is
/// the same. A tag a `_Data.xml` published is replaced like any other and
/// leaves no `label_in_file`: its record does not say what the file called
/// the item.
///
/// A symbol whose item the database has no tag for is left as it is, the
/// file's name and all. Putting the same tags on a sheet twice writes
/// nothing the second time.
pub(crate) fn fill_database_tags(
    doc: &mut Document,
    tags: &HashMap<String, String>,
    drawn_here: impl Fn(&EntityType) -> bool,
) -> DatabaseTagFill {
    // What an entity states for a key, an empty value being nothing stated.
    fn stated<'a>(entity: &'a EntityType, key: &str) -> Option<&'a str> {
        pid_value(entity, key).filter(|value| !value.is_empty())
    }

    let mut fill = DatabaseTagFill::default();
    // The symbols to restate: the tag, and the file's name to keep beside it.
    let mut restated: Vec<(document::Handle, &str, Option<String>)> = Vec::new();
    for entity in doc.model_space_entities() {
        let symbol =
            matches!(entity, EntityType::Insert(_)) && stated(entity, "role") == Some("symbol");
        if !symbol || drawn_here(entity) {
            continue;
        }
        let Some(tag) = stated(entity, "item")
            .and_then(|item| tags.get(&backup_uid_key(item)))
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
        else {
            continue;
        };
        let label = stated(entity, "label");
        let source = stated(entity, "tag_source");
        let kept = stated(entity, "label_in_file");
        let in_file = match source {
            Some("file") => label,
            Some("database") => kept,
            _ => None,
        };
        let differing = in_file.filter(|name| *name != tag);
        fill.tagged += 1;
        fill.differing += usize::from(differing.is_some());
        if label != Some(tag) || source != Some("database") || kept != differing {
            restated.push((entity.common().handle, tag, differing.map(str::to_string)));
        }
    }
    for (handle, tag, in_file) in restated {
        let Some(symbol) = doc.get_entity_mut(handle) else {
            continue;
        };
        stamp_pid_pairs(symbol, &[("label", tag), ("tag_source", "database")]);
        match in_file.as_deref() {
            Some(name) => stamp_pid_pairs(symbol, &[("label_in_file", name)]),
            None => clear_pid_key(symbol, "label_in_file"),
        }
        fill.changed += 1;
    }
    fill
}
