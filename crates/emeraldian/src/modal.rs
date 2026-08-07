//! Modal overlays: the command palette, quick switcher, search and prompts.
//!
//! Every list-style overlay is the same [`Picker`] — a query, a set of entries
//! and a ranked subset — because they behave identically from the user's side:
//! type to filter, arrows to move, Enter to run. Sharing the type means they
//! also stay consistent for free.

use emeraldian_core::search::{self, FuzzyMatch};

use crate::app::Action;

/// What an overlay is for, which decides its title and how entries are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Commands,
    Notes,
    Themes,
    Vaults,
    Search,
    Providers,
    Models,
}

impl PickerKind {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Commands => "Command palette",
            Self::Notes => "Quick switcher",
            Self::Themes => "Themes",
            Self::Vaults => "Open vault",
            Self::Search => "Search",
            Self::Providers => "Assistant provider",
            Self::Models => "Model",
        }
    }

    #[must_use]
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Commands => "Type a command…",
            Self::Notes => "Find a note by name…",
            Self::Themes => "Filter themes…",
            Self::Vaults => "Filter vaults…",
            Self::Search => "Search all notes…",
            Self::Providers => "Filter providers…",
            Self::Models => "Filter models…",
        }
    }
}

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// Secondary text: a keybinding, a path, a matching line.
    pub detail: String,
    pub action: Action,
}

impl Entry {
    #[must_use]
    pub fn new(label: impl Into<String>, detail: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            action,
        }
    }
}

/// A filterable list overlay.
#[derive(Debug, Clone)]
pub struct Picker {
    pub kind: PickerKind,
    pub query: String,
    pub cursor: usize,
    entries: Vec<Entry>,
    /// Indices into `entries`, best first, with their match positions.
    matches: Vec<(usize, Vec<usize>)>,
    pub selected: usize,
    pub scroll: usize,
    /// Set for pickers whose entries are recomputed from the query rather than
    /// filtered from a fixed list — global search, which has to read files.
    pub live: bool,
}

impl Picker {
    #[must_use]
    pub fn new(kind: PickerKind, entries: Vec<Entry>) -> Self {
        let mut picker = Self {
            kind,
            query: String::new(),
            cursor: 0,
            entries,
            matches: Vec::new(),
            selected: 0,
            scroll: 0,
            live: kind == PickerKind::Search,
        };
        picker.refilter();
        picker
    }

    /// Replaces the entries, keeping the query — used by live pickers.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        if self.live {
            // Entries already reflect the query; ranking them again by name
            // would fight the search's own relevance order.
            self.matches = (0..self.entries.len()).map(|i| (i, Vec::new())).collect();
            self.selected = self.selected.min(self.matches.len().saturating_sub(1));
        } else {
            self.refilter();
        }
    }

    fn refilter(&mut self) {
        let query = self.query.trim();
        if query.is_empty() {
            self.matches = (0..self.entries.len()).map(|i| (i, Vec::new())).collect();
        } else {
            let mut scored: Vec<(usize, FuzzyMatch)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    // Match the label first; fall back to the detail so
                    // searching a path finds a note shown by title.
                    search::fuzzy_match(query, &entry.label)
                        .map(|m| (i, m))
                        .or_else(|| {
                            search::fuzzy_match(query, &entry.detail).map(|mut m| {
                                m.score -= 8;
                                m.positions.clear();
                                (i, m)
                            })
                        })
                })
                .collect();
            scored.sort_by(|a, b| {
                b.1.score
                    .cmp(&a.1.score)
                    .then_with(|| self.entries[a.0].label.cmp(&self.entries[b.0].label))
            });
            self.matches = scored.into_iter().map(|(i, m)| (i, m.positions)).collect();
        }
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Visible rows: the entry and the matched character positions.
    pub fn visible(&self) -> impl Iterator<Item = (&Entry, &[usize])> {
        self.matches
            .iter()
            .map(|(index, positions)| (&self.entries[*index], positions.as_slice()))
    }

    #[must_use]
    pub fn selected_entry(&self) -> Option<&Entry> {
        self.matches
            .get(self.selected)
            .map(|(index, _)| &self.entries[*index])
    }

    pub fn insert(&mut self, ch: char) {
        let byte = self
            .query
            .char_indices()
            .nth(self.cursor)
            .map_or(self.query.len(), |(b, _)| b);
        self.query.insert(byte, ch);
        self.cursor += 1;
        self.selected = 0;
        self.scroll = 0;
        if !self.live {
            self.refilter();
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = self
            .query
            .char_indices()
            .nth(self.cursor - 1)
            .map_or(0, |(b, _)| b);
        self.query.remove(byte);
        self.cursor -= 1;
        self.selected = 0;
        if !self.live {
            self.refilter();
        }
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        if !self.live {
            self.refilter();
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len() as isize;
        // Wrapping makes Up from the top jump to the last result, which is how
        // every launcher behaves.
        self.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
    }

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

/// A single-line text prompt, e.g. for a new note's name.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub title: String,
    pub value: String,
    pub cursor: usize,
    pub intent: PromptIntent,
}

/// What to do with a prompt's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptIntent {
    NewNote,
    NewFolder,
    RenameNote,
    /// Filter typed into the explorer.
    FilterExplorer,
    /// An API key for the named provider. Typed characters are masked.
    ApiKey(String),
}

impl PromptIntent {
    /// Whether what is being typed should be hidden.
    ///
    /// Only for secrets. A note's name is not one, and hiding it would be
    /// baffling; a key on screen is a key in a screen share.
    #[must_use]
    pub fn secret(&self) -> bool {
        matches!(self, Self::ApiKey(_))
    }
}

impl Prompt {
    #[must_use]
    pub fn new(title: impl Into<String>, value: impl Into<String>, intent: PromptIntent) -> Self {
        let value = value.into();
        Self {
            cursor: value.chars().count(),
            title: title.into(),
            value,
            intent,
        }
    }

    pub fn insert(&mut self, ch: char) {
        let byte = self
            .value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(b, _)| b);
        self.value.insert(byte, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = self
            .value
            .char_indices()
            .nth(self.cursor - 1)
            .map_or(0, |(b, _)| b);
        self.value.remove(byte);
        self.cursor -= 1;
    }
}

/// A yes/no confirmation for a destructive action.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub message: String,
    pub action: Action,
}

/// The overlay currently on screen.
#[derive(Debug, Clone)]
pub enum Modal {
    Picker(Picker),
    Prompt(Prompt),
    Confirm(Confirm),
    /// The keybinding reference, with its scroll offset.
    Help(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Picker {
        Picker::new(
            PickerKind::Commands,
            vec![
                Entry::new("Open graph view", "Ctrl+G", Action::OpenGraph),
                Entry::new("New note", "Ctrl+N", Action::NewNote),
                Entry::new("Save", "Ctrl+S", Action::Save),
            ],
        )
    }

    #[test]
    fn a_new_picker_shows_everything() {
        let picker = commands();
        assert_eq!(picker.len(), 3);
        assert_eq!(
            picker.selected_entry().map(|e| e.label.as_str()),
            Some("Open graph view")
        );
    }

    #[test]
    fn typing_filters_and_ranks() {
        let mut picker = commands();
        for ch in "graph".chars() {
            picker.insert(ch);
        }
        assert_eq!(picker.len(), 1);
        assert_eq!(
            picker.selected_entry().map(|e| e.label.as_str()),
            Some("Open graph view")
        );
    }

    #[test]
    fn filtering_matches_the_detail_column_too() {
        let mut picker = commands();
        for ch in "ctrl+n".chars() {
            picker.insert(ch);
        }
        assert!(
            picker.visible().any(|(e, _)| e.label == "New note"),
            "a keybinding search should find its command"
        );
    }

    #[test]
    fn backspace_restores_earlier_results() {
        let mut picker = commands();
        for ch in "graph".chars() {
            picker.insert(ch);
        }
        assert_eq!(picker.len(), 1);
        for _ in 0..5 {
            picker.backspace();
        }
        assert_eq!(picker.len(), 3);
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        let mut picker = commands();
        picker.move_selection(-1);
        assert_eq!(picker.selected, 2, "up from the top wraps to the bottom");
        picker.move_selection(1);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn selection_stays_valid_when_results_shrink() {
        let mut picker = commands();
        picker.selected = 2;
        for ch in "graph".chars() {
            picker.insert(ch);
        }
        assert!(picker.selected < picker.len().max(1));
        assert!(picker.selected_entry().is_some());
    }

    #[test]
    fn a_query_matching_nothing_leaves_no_selection() {
        let mut picker = commands();
        for ch in "zzzz".chars() {
            picker.insert(ch);
        }
        assert!(picker.is_empty());
        assert!(picker.selected_entry().is_none());
    }

    #[test]
    fn live_pickers_keep_the_order_their_entries_arrive_in() {
        let mut picker = Picker::new(PickerKind::Search, Vec::new());
        for ch in "zzz".chars() {
            picker.insert(ch);
        }
        picker.set_entries(vec![
            Entry::new("Second.md", "line 2", Action::Save),
            Entry::new("First.md", "line 1", Action::Save),
        ]);

        assert_eq!(picker.len(), 2, "results are not re-filtered by the query");
        assert_eq!(
            picker.selected_entry().map(|e| e.label.as_str()),
            Some("Second.md"),
            "search relevance order is preserved"
        );
    }

    #[test]
    fn prompt_edits_by_character() {
        let mut prompt = Prompt::new("New note", "Draft", PromptIntent::NewNote);
        assert_eq!(prompt.cursor, 5);

        prompt.insert('!');
        assert_eq!(prompt.value, "Draft!");

        prompt.backspace();
        prompt.backspace();
        assert_eq!(prompt.value, "Draf");
    }

    #[test]
    fn scroll_follows_the_selection() {
        let entries: Vec<Entry> = (0..50)
            .map(|i| Entry::new(format!("Item {i}"), "", Action::Save))
            .collect();
        let mut picker = Picker::new(PickerKind::Commands, entries);

        picker.selected = 30;
        picker.scroll_into_view(10);
        assert_eq!(picker.scroll, 21);

        picker.selected = 2;
        picker.scroll_into_view(10);
        assert_eq!(picker.scroll, 2);
    }
}
