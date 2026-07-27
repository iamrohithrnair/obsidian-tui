//! The file explorer's tree.
//!
//! The vault is a nested folder structure but a terminal list is flat, so the
//! tree is flattened into rows on every rebuild, honoring which folders are
//! collapsed. Rebuilding rather than mutating keeps the view honest when notes
//! are created or deleted — including by the agent — at the cost of a walk over
//! a list the vault already holds in memory.

use std::collections::{BTreeMap, HashSet};

use otui_core::index::{NoteId, VaultIndex};

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
}

impl Explorer {
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
            for (id, note) in index.notes().iter().enumerate() {
                if note.meta.title.to_lowercase().contains(&filter)
                    || note.meta.rel.to_lowercase().contains(&filter)
                {
                    self.rows.push(Row::Note {
                        id,
                        name: note.meta.rel.trim_end_matches(".md").to_string(),
                        depth: 0,
                    });
                }
            }
            self.restore_selection(previous.as_deref());
            return;
        }

        let tree = Tree::build(index);
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
    fn build(index: &VaultIndex) -> Self {
        let mut subfolders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut notes: BTreeMap<String, Vec<(NoteId, String)>> = BTreeMap::new();

        for folder in index.folders() {
            let parent = folder
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .to_string();
            subfolders.entry(parent).or_default().push(folder.clone());
        }
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
            entries.sort_by_key(|(_, name)| name.to_lowercase());
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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

        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
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
        let mut explorer = Explorer::default();
        explorer.rebuild(&index);

        explorer.select_last();
        explorer.scroll_into_view(10);
        assert_eq!(explorer.scroll, explorer.len() - 10);

        explorer.select_first();
        explorer.scroll_into_view(10);
        assert_eq!(explorer.scroll, 0);
    }
}
