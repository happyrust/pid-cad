// Auto-split from scene/mod.rs. Pure text-move; behaviour unchanged.
use super::*;

use crate::io::pid_legend::{self, GroupTag, Rules, TagRead};

impl Scene {
    // ── Group helpers ──────────────────────────────────────────────────────

    pub fn groups(&self) -> impl Iterator<Item = &acadrust::objects::Group> {
        self.document.objects.values().filter_map(|obj| match obj {
            ObjectType::Group(g) => Some(g),
            _ => None,
        })
    }

    /// Returns the names of all groups that contain `handle`.
    pub fn group_names_for_entity(&self, handle: Handle) -> Vec<String> {
        self.groups()
            .filter(|g| g.contains(handle))
            .map(|g| g.name.clone())
            .collect()
    }

    /// The handles of the groups that contain `handle`.
    pub fn groups_containing(&self, handle: Handle) -> Vec<Handle> {
        self.groups()
            .filter(|g| g.contains(handle))
            .map(|g| g.handle)
            .collect()
    }

    // ── P&ID tags on groups ────────────────────────────────────────────────
    //
    // A group a person made is a P&ID symbol when its description carries
    // `tagName=` (`pid_legend::GroupTag`); the tag is read from the group's
    // own lettering by the recognition's rule, or set by hand.

    /// The P&ID tag `group` carries in its description; `None` for a plain
    /// group, or no group at that handle.
    pub fn group_tag(&self, group: Handle) -> Option<GroupTag> {
        match self.document.objects.get(&group)? {
            ObjectType::Group(g) => GroupTag::parse(&g.description),
            _ => None,
        }
    }

    /// Write `tag` into `group`'s description; `None` takes the P&ID keys
    /// out and leaves a plain group. True when a group was there and its
    /// description changed. No undo of its own: callers wrap it in
    /// `begin_group_undo` / `commit_group_undo`, which record the group
    /// objects' before and after. A changed tag advances the scene epoch
    /// because stored P&ID recognition depends on group descriptions.
    pub fn set_group_tag(&mut self, group: Handle, tag: Option<&GroupTag>) -> bool {
        let Some(ObjectType::Group(g)) = self.document.objects.get_mut(&group) else {
            return false;
        };
        let description = match tag {
            Some(tag) => tag.write_into(&g.description),
            None => GroupTag::strip(&g.description),
        };
        if description == g.description {
            return false;
        }
        g.description = description;
        self.bump_geometry();
        true
    }

    /// The tag `group`'s own lettering reads as, by the rule the recognition
    /// applies to a bubble's inner text (`pid_legend::derive_group_tag`).
    pub fn derive_group_tag(&self, group: Handle, rules: &Rules) -> Option<TagRead> {
        match self.document.objects.get(&group)? {
            ObjectType::Group(g) => pid_legend::derive_group_tag(&self.document, g, rules),
            _ => None,
        }
    }

    /// The groups that are P&ID symbols, with their tags.
    pub fn tagged_groups(&self) -> impl Iterator<Item = (&acadrust::objects::Group, GroupTag)> {
        self.groups()
            .filter_map(|g| GroupTag::parse(&g.description).map(|tag| (g, tag)))
    }

    /// Creates a named group from the given handles and registers it in the group dictionary.
    pub fn create_group(&mut self, name: String, handles: Vec<Handle>) -> Handle {
        let group_dict_handle = self.document.header.acad_group_dict_handle;
        let mut group = acadrust::objects::Group::new(&name);
        group.handle = self.document.allocate_handle();
        group.owner = group_dict_handle;
        group.add_entities(handles);
        let gh = group.handle;
        self.document.objects.insert(gh, ObjectType::Group(group));
        if let Some(ObjectType::Dictionary(dict)) =
            self.document.objects.get_mut(&group_dict_handle)
        {
            dict.add_entry(&name, gh);
        }
        gh
    }

    /// Recreate every *live* group whose full membership was copied.
    ///
    /// Partial group copies intentionally remain ungrouped: copying one member
    /// out of a group should not create a new one-member fragment. When the
    /// whole source group is in `handle_map`, the new handles get their own
    /// Group object so later selection/editing treats the copy as a group too.
    ///
    /// The in-drawing COPY / ARRAY path: the source groups still live in this
    /// document, so gather them here and hand them to [`Scene::recreate_groups`].
    /// The clipboard paste path snapshots its groups instead (they may come from
    /// another drawing) and calls `recreate_groups` directly — one shared body
    /// so all copy routes preserve groups identically.
    pub fn copy_complete_groups(
        &mut self,
        handle_map: &rustc_hash::FxHashMap<Handle, Handle>,
    ) -> usize {
        if handle_map.is_empty() {
            return 0;
        }
        let sources: Vec<_> = self
            .document
            .objects
            .values()
            .filter_map(|obj| match obj {
                ObjectType::Group(g)
                    if !g.entities.is_empty()
                        && g.entities.iter().all(|h| handle_map.contains_key(h)) =>
                {
                    Some(g.clone())
                }
                _ => None,
            })
            .collect();
        self.recreate_groups(sources, handle_map)
    }

    /// Recreate each group in `sources` in this document, remapping its member
    /// handles through `handle_map` (source → new). Each recreated group gets a
    /// fresh handle, a unique `NAME_COPYn`, and a group-dictionary entry.
    /// Members absent from the map are dropped; a group left with no members is
    /// skipped.
    ///
    /// Shared by the in-drawing COPY/ARRAY path ([`Scene::copy_complete_groups`],
    /// live source groups) and the clipboard paste path (groups snapshotted into
    /// the clipboard at copy time), so a fully-copied group stays grouped whether
    /// the copy lands in the same drawing or a different file.
    pub fn recreate_groups(
        &mut self,
        sources: Vec<acadrust::objects::Group>,
        handle_map: &rustc_hash::FxHashMap<Handle, Handle>,
    ) -> usize {
        let group_dict_handle = self.document.header.acad_group_dict_handle;
        let mut copied = 0;
        let mut dictionary_recorded = false;
        for source in sources {
            let entities: Vec<Handle> = source
                .entities
                .iter()
                .filter_map(|h| handle_map.get(h).copied())
                .collect();
            if entities.is_empty() {
                continue;
            }
            let name = self.unique_group_copy_name(&source.name);
            let mut group = source;
            group.handle = self.document.allocate_handle();
            group.owner = group_dict_handle;
            group.name = name.clone();
            group.entities = entities;
            let gh = group.handle;
            if self.is_recording_undo() {
                if !dictionary_recorded {
                    let before = self.document.objects.get(&group_dict_handle).cloned();
                    self.record_undo_object_before(group_dict_handle, before);
                    dictionary_recorded = true;
                }
                self.record_undo_object_before(gh, None);
            }
            self.document.objects.insert(gh, ObjectType::Group(group));
            if let Some(ObjectType::Dictionary(dict)) =
                self.document.objects.get_mut(&group_dict_handle)
            {
                dict.add_entry(&name, gh);
            }
            copied += 1;
        }
        copied
    }

    fn unique_group_copy_name(&self, source: &str) -> String {
        let group_dict_handle = self.document.header.acad_group_dict_handle;
        let exists = |name: &str| {
            self.document
                .objects
                .get(&group_dict_handle)
                .and_then(|obj| match obj {
                    ObjectType::Dictionary(dict) => Some(
                        dict.entries
                            .iter()
                            .any(|(entry, _)| entry.eq_ignore_ascii_case(name)),
                    ),
                    _ => None,
                })
                .unwrap_or(false)
        };
        for n in 1.. {
            let candidate = format!("{source}_COPY{n}");
            if !exists(&candidate) {
                return candidate;
            }
        }
        unreachable!()
    }

    /// Dissolves all groups that contain any of the given handles.
    /// Returns the number of groups removed.
    pub fn delete_groups_containing(&mut self, handles: &[Handle]) -> usize {
        let group_dict_handle = self.document.header.acad_group_dict_handle;
        let to_delete: Vec<Handle> = self
            .document
            .objects
            .values()
            .filter_map(|obj| match obj {
                ObjectType::Group(g) if handles.iter().any(|h| g.contains(*h)) => Some(g.handle),
                _ => None,
            })
            .collect();
        let count = to_delete.len();
        for gh in &to_delete {
            if let Some(ObjectType::Dictionary(dict)) =
                self.document.objects.get_mut(&group_dict_handle)
            {
                dict.entries.retain(|(_, h)| h != gh);
            }
            self.document.objects.remove(gh);
        }
        count
    }

    /// Return `handles` plus every member of a selectable group containing one
    /// of them. Selection and rollover highlighting share this expansion so the
    /// preview matches what a click will select.
    pub fn handles_expanded_for_selectable_groups(
        &self,
        handles: &[Handle],
    ) -> HashSet<Handle> {
        let mut expanded: HashSet<Handle> = handles.iter().copied().collect();
        expanded.extend(
            self.document
                .objects
                .values()
                .filter_map(|obj| match obj {
                    ObjectType::Group(g)
                        if g.selectable
                            && handles.iter().any(|handle| g.contains(*handle)) =>
                    {
                        Some(g.entities.clone())
                    }
                    _ => None,
                })
                .flatten(),
        );
        expanded
    }

    /// If any handle belongs to a selectable group, also select every member.
    pub fn expand_selection_for_groups(&mut self, handles: &[Handle]) {
        let previous_len = self.selected.len();
        self.selected
            .extend(self.handles_expanded_for_selectable_groups(handles));
        if self.selected.len() != previous_len {
            self.bump_selection_set();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_legend::{TagHow, TagSource};
    use acadrust::entities::{Line, Text};
    use acadrust::types::Vector3;

    /// A group of a valve's strokes and the tag lettered beside them reads
    /// its tag by the recognition's rule; written into the description it
    /// is a P&ID group, stripped it is a plain group again.
    #[test]
    fn a_group_reads_carries_and_drops_its_pid_tag() {
        let mut scene = Scene::new();
        let strokes: Vec<Handle> = [
            ((0.0, 0.0), (0.0, 3.0)),
            ((0.0, 3.0), (6.0, 0.0)),
            ((6.0, 0.0), (6.0, 3.0)),
            ((6.0, 3.0), (0.0, 0.0)),
        ]
        .into_iter()
        .map(|(a, b)| {
            scene.add_entity(EntityType::Line(Line::from_points(
                Vector3::new(a.0, a.1, 0.0),
                Vector3::new(b.0, b.1, 0.0),
            )))
        })
        .collect();
        let tag = scene.add_entity(EntityType::Text(
            Text::with_value("BUV-3101", Vector3::new(0.0, 4.0, 0.0)).with_height(2.5),
        ));
        let mut members = strokes.clone();
        members.push(tag);
        let gh = scene.create_group("*A1".to_string(), members);
        assert_eq!(scene.groups_containing(tag), vec![gh]);

        // A plain group until a tag is written into it.
        assert_eq!(scene.group_tag(gh), None);
        assert_eq!(scene.tagged_groups().count(), 0);
        let rules = Rules::builtin();
        let read = scene
            .derive_group_tag(gh, &rules)
            .expect("the lettering reads");
        assert_eq!((read.value.as_str(), read.how), ("BUV-3101", TagHow::Shape));

        assert!(scene.set_group_tag(gh, Some(&GroupTag::auto(Some(read.value.clone())))));
        let carried = scene.group_tag(gh).expect("a P&ID group now");
        assert_eq!(carried.name.as_deref(), Some("BUV-3101"));
        assert_eq!(carried.source, TagSource::Auto);
        assert_eq!(scene.tagged_groups().count(), 1);
        assert!(
            !scene.set_group_tag(gh, Some(&carried)),
            "writing the same tag again changes nothing"
        );

        // By hand, then back to a plain group.
        assert!(scene.set_group_tag(gh, Some(&GroupTag::manual("BUV-3199"))));
        assert_eq!(
            scene.group_tag(gh),
            Some(GroupTag::manual("BUV-3199")),
            "the hand-set value is what the description says"
        );
        assert_eq!(
            scene.derive_group_tag(gh, &rules).map(|r| r.value),
            Some("BUV-3101".to_string()),
            "the lettering still reads as it did"
        );
        assert!(scene.set_group_tag(gh, None));
        assert_eq!(scene.group_tag(gh), None);
        assert!(!scene.set_group_tag(gh, None));
        assert!(!scene.set_group_tag(Handle::new(0xFFFF), Some(&GroupTag::manual("X"))));
    }
}
