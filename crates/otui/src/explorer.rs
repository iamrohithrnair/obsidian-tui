//! The file explorer's tree.
//!
//! The vault is a nested folder structure but a terminal list is flat, so the
//! tree is flattened into rows on every rebuild, honoring which folders are
//! collapsed. Rebuilding rather than mutating keeps the view honest when notes
//! are created or deleted — including by the agent — at the cost of a walk over
//! a list the vault already holds in memory.

use std::collections::{BTreeMap, HashSet};

use otui_core::index::{NoteId, VaultIndex};
use otui_core::sort::SortOrder;

/// One visible line in the explorer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder {
        /// Vault-relative folder path.
        rel: String,
        name: String,
        depth: usize,
        collapsed: bool,
        /// Notes directly or indirectly inside.
        count: usize,
    },
    Note {
        id: NoteId,
        name: String,
        depth: usize,
    },
}

impl Row {
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Folder { depth, .. } | Self::Note { depth, .. } => *depth,
        }
    }

    /// The row's display text.
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "rendering destructures rows directly")
    )]
    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } | Self::Note { name, .. } => name,
        }
    }
}

#[derive(Debug, Default)]
pub struct Explorer {
    rows: Vec<Row>,
    pub selected: usize,
    pub scroll: usize,
    collapsed: HashSet<String>,
    /// Filter typed into the explorer, matched against note names.
    pub filter: String,
    /// How notes are ordered within a folder. Held here rather than passed to
    /// `rebuild` because every call site would otherwise have to thread it
    /// through, and the explorer is rebuilt from a dozen places.
    sort: SortOrder,
}

impl Explorer {
    /// The order notes are listed in.
    #[must_use]
    pub fn sort(&self) -> SortOrder {
        self.sort
    }

    /// Changes the order. The caller rebuilds to make it visible.
    pub fn set_sort(&mut self, sort: SortOrder) {
        self.sort = sort;
    }

    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// The note under the cursor, if the cursor is on a note.
    #[must_use]
    pub fn selected_note(&self) -> Option<NoteId> {
        match self.selected_row() {
            Some(Row::Note { id, .. }) => Some(*id),
            _ => None,
        }
    }

    /// The folder under the cursor, or the folder containing the selected note.
    #[must_use]
    pub fn selected_folder(&self, index: &VaultIndex) -> String {
        match self.selected_row() {
            Some(Row::Folder { rel, .. }) => rel.clone(),
            Some(Row::Note { id, .. }) => index
                .note(*id)
                .map(|n| n.meta.folder().to_string())
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Rebuilds the flattened rows from the index.
    pub fn rebuild(&mut self, index: &VaultIndex) {
        let previous = self.selected_key();
        self.rows.clear();

        let filter = self.filter.trim().to_lowercase();
        if !filter.is_empty() {
            // While filtering, the tree gets in the way: a flat list of matches
            // with their paths is what the user is actually looking at.
            let mut hits: Vec<NoteId> = (0..index.notes().len())
                .filter(|&id| {
                    let meta = &index.notes()[id].meta;
                    meta.title.to_lowercase().contains(&filter)
                        || meta.rel.to_lowercase().contains(&filter)
                })
                .collect();
            // The flat list obeys the same order as the tree, so switching to
            // "most recently modified" doesn't silently stop applying the
            // moment you start typing.
            hits.sort_by(|&a, &b| {
                self.sort
                    .compare(&index.notes()[a].meta, &index.notes()[b].meta)
            });
            for id in hits {
                self.rows.push(Row::Note {
                    id,
                    name: index.notes()[id]
                        .meta
                        .rel
                        .trim_end_matches(".md")
                        .to_string(),
                    depth: 0,
                });
            }
            self.restore_selection(previous.as_deref());
            return;
        }

        let tree = Tree::build(index, self.sort);
        self.emit(&tree, "", 0, index);
        self.restore_selection(previous.as_deref());
    }

    /// Walks one folder, emitting subfolders before notes.
    ///
    /// Folders first is what every file tree does, Obsidian's included, and
    /// recursing rather than scanning a flat list is what makes a collapsed
    /// folder actually hide its whole subtree.
    fn emit(&mut self, tree: &Tree, folder: &str, depth: usize, index: &VaultIndex) {
        for child in tree.subfolders(folder) {
            let collapsed = self.collapsed.contains(child);
            let name = child.rsplit('/').next().unwrap_or(child).to_string();
            self.rows.push(Row::Folder {
                rel: child.clone(),
                name,
                depth,
                collapsed,
                count: count_notes(index, child),
            });
            if !collapsed {
                self.emit(tree, child, depth + 1, index);
            }
        }

        for (id, name) in tree.notes(folder) {
            self.rows.push(Row::Note {
                id: *id,
                name: name.clone(),
                depth,
            });
        }
    }

    /// A stable key for the selected row, so a rebuild keeps the cursor on the
    /// same item rather than the same index.
    fn selected_key(&self) -> Option<String> {
        match self.selected_row()? {
            Row::Folder { rel, .. } => Some(format!("f:{rel}")),
            Row::Note { name, depth, .. } => Some(format!("n:{depth}:{name}")),
        }
    }

    fn restore_selection(&mut self, previous: Option<&str>) {
        if let Some(key) = previous {
            let found = self.rows.iter().position(|row| match row {
                Row::Folder { rel, .. } => format!("f:{rel}") == key,
                Row::Note { name, depth, .. } => format!("n:{depth}:{name}") == key,
            });
            if let Some(index) = found {
                self.selected = index;
                return;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    pub fn select_next(&mut self) {
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1).min(self.rows.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    pub fn page(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let target = (self.selected as isize + delta).clamp(0, self.rows.len() as isize - 1);
        self.selected = target as usize;
    }

    /// Selects the row holding a specific note, expanding folders as needed.
    pub fn reveal(&mut self, index: &VaultIndex, note: NoteId) {
        if let Some(note) = index.note(note) {
            let folder = note.meta.folder();
            if !folder.is_empty() {
                let mut path = String::new();
                for segment in folder.split('/') {
                    if !path.is_empty() {
                        path.push('/');
                    }
                    path.push_str(segment);
                    self.collapsed.remove(&path);
                }
            }
        }
        self.rebuild(index);
        if let Some(position) = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Note { id, .. } if *id == note))
        {
            self.selected = position;
        }
    }

    /// Toggles the folder under the cursor. Returns whether anything changed.
    pub fn toggle(&mut self, index: &VaultIndex) -> bool {
        let Some(Row::Folder { rel, .. }) = self.selected_row() else {
            return false;
        };
        let rel = rel.clone();
        if !self.collapsed.remove(&rel) {
            self.collapsed.insert(rel);
        }
        self.rebuild(index);
        true
    }

    /// The folders currently open, for saving between runs.
    ///
    /// Open rather than closed, because that is the set which survives the
    /// vault changing underneath it — see [`crate::state`].
    #[must_use]
    pub fn expanded(&self, index: &VaultIndex) -> Vec<String> {
        index
            .folders()
            .iter()
            .filter(|folder| !self.collapsed.contains(*folder))
            .cloned()
            .collect()
    }

    /// Opens exactly these folders and closes every other one.
    ///
    /// A folder the saved set has never heard of stays shut, which is what
    /// makes a folder added since the last run start collapsed.
    pub fn set_expanded(&mut self, expanded: &[String], index: &VaultIndex) {
        let open: std::collections::HashSet<&String> = expanded.iter().collect();
        self.collapsed = index
            .folders()
            .iter()
            .filter(|folder| !open.contains(*folder))
            .cloned()
            .collect();
        self.rebuild(index);
    }

    pub fn collapse_all(&mut self, index: &VaultIndex) {
        self.collapsed = index.folders().iter().cloned().collect();
        self.rebuild(index);
    }

    pub fn expand_all(&mut self, index: &VaultIndex) {
        self.collapsed.clear();
        self.rebuild(index);
    }

    /// Scrolls so the selection is visible in a viewport of `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
    }
}

/// The vault's folder structure, indexed for a depth-first walk.
struct Tree {
    /// Parent folder → its immediate subfolders, sorted.
    subfolders: BTreeMap<String, Vec<String>>,
    /// Folder → the notes directly inside it, sorted by display name.
    notes: BTreeMap<String, Vec<(NoteId, String)>>,
}

impl Tree {
    fn build(index: &VaultIndex, sort: SortOrder) -> Self {
        let mut subfolders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut notes: BTreeMap<String, Vec<(NoteId, String)>> = BTreeMap::new();

        for folder in index.folders() {
            let parent = folder
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .to_string();
            subfolders.entry(parent).or_default().push(folder.clone());
        }
        // Folders stay alphabetical whatever the note order is. Obsidian sorts
        // them by name too, and a folder's own mtime tracks when a file inside
        // it was added, which makes the tree jump around for reasons that
        // aren't visible on screen.
        for children in subfolders.values_mut() {
            children.sort_by_key(|f| f.to_lowercase());
        }

        for (id, note) in index.notes().iter().enumerate() {
            notes
                .entry(note.meta.folder().to_string())
                .or_default()
                .push((id, note.meta.title.clone()));
        }
        for entries in notes.values_mut() {
            entries.sort_by(|(a, _), (b, _)| {
                sort.compare(&index.notes()[*a].meta, &index.notes()[*b].meta)
            });
        }

        Self { subfolders, notes }
    }

    fn subfolders(&self, folder: &str) -> &[String] {
        self.subfolders.get(folder).map_or(&[], Vec::as_slice)
    }

    fn notes(&self, folder: &str) -> &[(NoteId, String)] {
        self.notes.get(folder).map_or(&[], Vec::as_slice)
    }
}

fn count_notes(index: &VaultIndex, folder: &str) -> usize {
    let prefix = format!("{folder}/");
    index
        .notes()
        .iter()
        .filter(|n| n.meta.rel.starts_with(&prefix))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use otui_core::test_support::TempVault;

    fn vault() -> TempVault {
        let vault = TempVault::new("explorer");
        vault.write("Root.md", "r");
        vault.write("Projects/Alpha.md", "a");
        vault.write("Projects/Beta.md", "b");
        vault.write("Projects/Deep/Gamma.md", "g");
        vault
    }

    #[test]
    fn the_open_folders_round_trip_through_a_saved_set() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        explorer.set_expanded(&["Projects".to_string()], &index);
        assert_eq!(
            explorer.expanded(&index),
            vec!["Projects".to_string()],
            "what was opened is what comes back out"
        );
        let listed = names(&explorer);
        assert!(
            listed.iter().any(|n| n.trim() == "Alpha"),
            "Projects is open"
        );
        assert!(
            !listed.iter().any(|n| n.trim() == "Gamma"),
            "but Projects/Deep stayed shut: {listed:?}"
        );

        // Nested folders are restored independently of their parents.
        explorer.set_expanded(
            &["Projects".to_string(), "Projects/Deep".to_string()],
            &index,
        );
        assert!(names(&explorer).iter().any(|n| n.trim() == "Gamma"));
    }

    #[test]
    fn a_folder_the_saved_set_never_heard_of_starts_closed() {
        // The reason the *open* folders are stored rather than the closed
        // ones: a folder added since the last run must not spring open.
        let vault = vault();
        let index = vault.index();
        let mut first_run = explorer();
        first_run.set_expanded(&["Projects".to_string()], &index);
        // What quitting would write down, against the vault as it was then.
        let saved = first_run.expanded(&index);

        // A folder appears while the app is closed, and the next run restores.
        vault.write("Archive/Old.md", "o");
        let index = vault.index();
        let mut restored = explorer();
        restored.set_expanded(&saved, &index);

        let listed = names(&restored);
        assert!(listed.iter().any(|n| n.trim() == "Archive"), "{listed:?}");
        assert!(
            !listed.iter().any(|n| n.trim() == "Old"),
            "a new folder starts collapsed: {listed:?}"
        );
    }

    #[test]
    fn collapse_all_leaves_nothing_expanded() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.collapse_all(&index);

        assert!(explorer.expanded(&index).is_empty());
        let listed = names(&explorer);
        assert!(listed.iter().any(|n| n.trim() == "Projects"), "{listed:?}");
        assert!(
            !listed.iter().any(|n| n.trim() == "Alpha"),
            "nothing inside a folder shows: {listed:?}"
        );
    }

    /// An explorer pinned to name order.
    ///
    /// The app's default is most-recently-modified, which for files written
    /// microseconds apart in a test depends on which side of a second boundary
    /// they landed on. These tests are about tree structure, so they pin the
    /// one order that doesn't depend on the clock.
    fn explorer() -> Explorer {
        let mut explorer = Explorer::default();
        explorer.set_sort(SortOrder::NameAsc);
        explorer
    }

    fn names(explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth()), r.name()))
            .collect()
    }

    #[test]
    fn flattens_the_tree_in_path_order() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        assert_eq!(
            names(&explorer),
            vec![
                "Projects",
                "  Deep",
                "    Gamma",
                "  Alpha",
                "  Beta",
                "Root",
            ]
        );
    }

    #[test]
    fn collapsing_a_folder_hides_its_contents() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        // Put the cursor on "Projects" and collapse it.
        explorer.selected = 0;
        assert!(explorer.toggle(&index));

        assert_eq!(names(&explorer), vec!["Projects", "Root"]);

        explorer.selected = 0;
        explorer.toggle(&index);
        assert!(
            names(&explorer).len() > 2,
            "expanding restores the children"
        );
    }

    #[test]
    fn folder_counts_include_nested_notes() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        match &explorer.rows()[0] {
            Row::Folder { name, count, .. } => {
                assert_eq!(name, "Projects");
                assert_eq!(*count, 3, "Alpha, Beta and the nested Gamma");
            }
            other => panic!("expected a folder row, got {other:?}"),
        }
    }

    #[test]
    fn selection_sticks_to_the_item_across_a_rebuild() {
        let vault = vault();
        let mut index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        // Select "Root", the last row.
        explorer.select_last();
        assert_eq!(explorer.selected_row().map(Row::name), Some("Root"));

        // Adding a note earlier in the sort order shifts every index.
        index.create_note("Aaa", "x").expect("create");
        explorer.rebuild(&index);

        assert_eq!(
            explorer.selected_row().map(Row::name),
            Some("Root"),
            "the cursor follows the item, not the row number"
        );
    }

    #[test]
    fn reveal_expands_ancestors_and_selects_the_note() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);
        explorer.collapse_all(&index);
        assert_eq!(names(&explorer), vec!["Projects", "Root"]);

        let gamma = index.id_of_rel("Projects/Deep/Gamma.md").unwrap();
        explorer.reveal(&index, gamma);

        assert_eq!(explorer.selected_note(), Some(gamma));
    }

    #[test]
    fn filtering_shows_matches_even_inside_collapsed_folders() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.collapse_all(&index);

        explorer.filter = "gamma".into();
        explorer.rebuild(&index);

        assert!(
            explorer
                .rows()
                .iter()
                .any(|r| matches!(r, Row::Note { name, .. } if name.ends_with("Gamma"))),
            "a filtered match must be reachable"
        );
    }

    #[test]
    fn empty_folders_still_appear() {
        let vault = vault();
        let mut index = vault.index();
        index.create_folder("Empty").expect("create folder");

        let mut explorer = explorer();
        explorer.rebuild(&index);

        assert!(explorer
            .rows()
            .iter()
            .any(|r| matches!(r, Row::Folder { name, .. } if name == "Empty")));
    }

    #[test]
    fn navigation_stays_in_bounds() {
        let vault = vault();
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        explorer.select_previous();
        assert_eq!(explorer.selected, 0);

        for _ in 0..100 {
            explorer.select_next();
        }
        assert_eq!(explorer.selected, explorer.len() - 1);

        explorer.page(-100);
        assert_eq!(explorer.selected, 0);
    }

    #[test]
    fn an_empty_vault_has_no_rows_and_no_panic() {
        let vault = TempVault::new("explorer-empty");
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        assert!(explorer.is_empty());
        assert_eq!(explorer.selected_note(), None);
        explorer.select_next();
        explorer.scroll_into_view(10);
    }

    #[test]
    fn scroll_follows_the_selection() {
        let vault = TempVault::new("explorer-scroll");
        for i in 0..40 {
            vault.write(&format!("N{i:02}.md"), "x");
        }
        let index = vault.index();
        let mut explorer = explorer();
        explorer.rebuild(&index);

        explorer.select_last();
        explorer.scroll_into_view(10);
        assert_eq!(explorer.scroll, explorer.len() - 10);

        explorer.select_first();
        explorer.scroll_into_view(10);
        assert_eq!(explorer.scroll, 0);
    }
}

/// Tests for the sort order, which is the thing a user actually sees change.
#[cfg(test)]
mod sort_tests {
    use super::*;
    use otui_core::test_support::TempVault;
    use std::time::{Duration, SystemTime};

    /// Writes a note and stamps it with an explicit modification time.
    ///
    /// Real timestamps rather than hand-built metadata, so this exercises the
    /// whole path: the filesystem, the vault scan, and the explorer. `set_modified`
    /// avoids sleeping for a second per note, which would make this unbearable.
    fn write_at(vault: &TempVault, rel: &str, secs_ago: u64) {
        let path = vault.write(rel, "x");
        let when = SystemTime::now() - Duration::from_secs(secs_ago);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("reopen the note")
            .set_modified(when)
            .expect("stamp the note");
    }

    /// A vault where the alphabetical order and the recency order disagree, so
    /// a test can't pass by accident.
    fn staggered() -> TempVault {
        let vault = TempVault::new("explorer-sort");
        write_at(&vault, "Apple.md", 300);
        write_at(&vault, "Banana.md", 100);
        write_at(&vault, "Cherry.md", 200);
        vault
    }

    fn note_names(explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .filter(|r| matches!(r, Row::Note { .. }))
            .map(|r| r.name().to_string())
            .collect()
    }

    fn rows_for(vault: &TempVault, order: SortOrder) -> Vec<String> {
        let index = vault.index();
        let mut explorer = Explorer::default();
        explorer.set_sort(order);
        explorer.rebuild(&index);
        note_names(&explorer)
    }

    #[test]
    fn the_most_recently_edited_note_is_at_the_top_by_default() {
        let vault = staggered();
        let index = vault.index();
        let mut explorer = Explorer::default();
        explorer.rebuild(&index);
        assert_eq!(explorer.sort(), SortOrder::ModifiedDesc);
        assert_eq!(note_names(&explorer), ["Banana", "Cherry", "Apple"]);
    }

    #[test]
    fn each_order_lists_the_notes_differently() {
        let vault = staggered();
        assert_eq!(
            rows_for(&vault, SortOrder::ModifiedDesc),
            ["Banana", "Cherry", "Apple"]
        );
        assert_eq!(
            rows_for(&vault, SortOrder::ModifiedAsc),
            ["Apple", "Cherry", "Banana"]
        );
        assert_eq!(
            rows_for(&vault, SortOrder::NameAsc),
            ["Apple", "Banana", "Cherry"]
        );
        assert_eq!(
            rows_for(&vault, SortOrder::NameDesc),
            ["Cherry", "Banana", "Apple"]
        );
    }

    #[test]
    fn touching_a_note_moves_it_to_the_top() {
        let vault = staggered();
        let mut explorer = Explorer::default();
        explorer.rebuild(&vault.index());
        assert_eq!(note_names(&explorer)[0], "Banana");

        // The oldest note is edited; it should lead on the next rebuild.
        write_at(&vault, "Apple.md", 0);
        explorer.rebuild(&vault.index());
        assert_eq!(note_names(&explorer)[0], "Apple");
    }

    #[test]
    fn folders_stay_alphabetical_whatever_the_note_order_is() {
        let vault = TempVault::new("explorer-sort-folders");
        // Zulu is written first, so it is the *oldest*: if folders followed the
        // note order it would sort last under a recency order.
        write_at(&vault, "Zulu/Note.md", 300);
        write_at(&vault, "Alpha/Note.md", 100);
        let index = vault.index();

        for order in SortOrder::ALL {
            let mut explorer = Explorer::default();
            explorer.set_sort(order);
            explorer.rebuild(&index);
            let folders: Vec<String> = explorer
                .rows()
                .iter()
                .filter(|r| matches!(r, Row::Folder { .. }))
                .map(|r| r.name().to_string())
                .collect();
            assert_eq!(folders, ["Alpha", "Zulu"], "folders moved under {order:?}");
        }
    }

    #[test]
    fn notes_are_ordered_within_their_own_folder_not_across_the_vault() {
        let vault = TempVault::new("explorer-sort-nested");
        write_at(&vault, "Folder/Old.md", 900);
        write_at(&vault, "Folder/New.md", 10);
        write_at(&vault, "RootNote.md", 500);
        let index = vault.index();
        let mut explorer = Explorer::default();
        explorer.rebuild(&index);

        // Depth-first: the folder and its notes, then the root note. The root
        // note's timestamp sits between the two, and must not interleave.
        assert_eq!(
            names(&explorer),
            ["Folder", "  New", "  Old", "RootNote"],
            "notes should sort inside their folder only"
        );
    }

    #[test]
    fn the_filtered_list_obeys_the_sort_order_too() {
        let vault = staggered();
        let index = vault.index();
        let mut explorer = Explorer::default();
        explorer.set_sort(SortOrder::NameDesc);
        // Matches every note through its path, so all three are in play and the
        // two orders below are genuinely different lists.
        explorer.filter = "md".into();
        explorer.rebuild(&index);
        assert_eq!(note_names(&explorer), ["Cherry", "Banana", "Apple"]);

        explorer.set_sort(SortOrder::ModifiedDesc);
        explorer.rebuild(&index);
        assert_eq!(note_names(&explorer), ["Banana", "Cherry", "Apple"]);
    }

    #[test]
    fn changing_the_order_keeps_the_selection_on_the_same_note() {
        let vault = staggered();
        let index = vault.index();
        let mut explorer = Explorer::default();
        explorer.set_sort(SortOrder::NameAsc);
        explorer.rebuild(&index);
        explorer.selected = 0;
        let before = explorer.selected_note();
        assert_eq!(note_names(&explorer)[0], "Apple");

        explorer.set_sort(SortOrder::ModifiedDesc);
        explorer.rebuild(&index);
        // Apple is now last, and the cursor should have followed it there
        // rather than staying on row 0.
        assert_eq!(explorer.selected_note(), before, "selection should follow");
        assert_eq!(explorer.selected, 2);
    }

    /// The helper from the sibling module, duplicated rather than shared so
    /// each module's tests stand on their own.
    fn names(explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth()), r.name()))
            .collect()
    }
}
