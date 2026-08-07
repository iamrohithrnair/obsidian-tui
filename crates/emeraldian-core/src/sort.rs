//! How the file explorer orders notes.
//!
//! The six orders are the same six Obsidian offers, under the same names, so
//! the setting reads the way the menu people already know reads. The choice is
//! written to the config file, so it survives a restart.

use std::fmt;
use std::str::FromStr;

use crate::note::NoteMeta;

/// The order notes appear in within a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// Alphabetical, A to Z. Obsidian's own default.
    NameAsc,
    /// Alphabetical, Z to A.
    NameDesc,
    /// Most recently edited first. The default here.
    #[default]
    ModifiedDesc,
    /// Least recently edited first.
    ModifiedAsc,
    /// Newest note first.
    CreatedDesc,
    /// Oldest note first.
    CreatedAsc,
}

impl SortOrder {
    /// Every order, in the sequence [`SortOrder::next`] walks them.
    pub const ALL: [SortOrder; 6] = [
        SortOrder::ModifiedDesc,
        SortOrder::ModifiedAsc,
        SortOrder::CreatedDesc,
        SortOrder::CreatedAsc,
        SortOrder::NameAsc,
        SortOrder::NameDesc,
    ];

    /// The label shown in the UI, phrased as Obsidian phrases it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            SortOrder::NameAsc => "File name (A to Z)",
            SortOrder::NameDesc => "File name (Z to A)",
            SortOrder::ModifiedDesc => "Modified time (new to old)",
            SortOrder::ModifiedAsc => "Modified time (old to new)",
            SortOrder::CreatedDesc => "Created time (new to old)",
            SortOrder::CreatedAsc => "Created time (old to new)",
        }
    }

    /// The value written to the config file.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            SortOrder::NameAsc => "name",
            SortOrder::NameDesc => "name-desc",
            SortOrder::ModifiedDesc => "modified",
            SortOrder::ModifiedAsc => "modified-asc",
            SortOrder::CreatedDesc => "created",
            SortOrder::CreatedAsc => "created-asc",
        }
    }

    /// The next order when cycling through them from the UI.
    #[must_use]
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|&o| o == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// Orders two notes.
    ///
    /// Every comparison falls back to the lowercased title, so the order is
    /// total and the tree can't reshuffle between rebuilds: two notes saved in
    /// the same second would otherwise be free to swap places on every
    /// keystroke.
    #[must_use]
    pub fn compare(self, a: &NoteMeta, b: &NoteMeta) -> std::cmp::Ordering {
        let by_name = || a.title.to_lowercase().cmp(&b.title.to_lowercase());
        match self {
            SortOrder::NameAsc => by_name(),
            SortOrder::NameDesc => by_name().reverse(),
            SortOrder::ModifiedDesc => b.modified.cmp(&a.modified).then_with(by_name),
            SortOrder::ModifiedAsc => a.modified.cmp(&b.modified).then_with(by_name),
            SortOrder::CreatedDesc => b.created.cmp(&a.created).then_with(by_name),
            SortOrder::CreatedAsc => a.created.cmp(&b.created).then_with(by_name),
        }
    }

    /// Whether this order puts recently touched notes at the top.
    #[must_use]
    pub const fn is_by_time(self) -> bool {
        !matches!(self, SortOrder::NameAsc | SortOrder::NameDesc)
    }
}

impl fmt::Display for SortOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for SortOrder {
    type Err = ();

    /// Parses a config value. Unknown values are rejected so the caller can
    /// warn and fall back rather than silently reordering someone's vault.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept the hyphenated key and a couple of spellings people
        // reasonably reach for, since this is hand-edited in a TOML file.
        match s.trim().to_lowercase().replace('_', "-").as_str() {
            "name" | "name-asc" | "a-z" => Ok(SortOrder::NameAsc),
            "name-desc" | "z-a" => Ok(SortOrder::NameDesc),
            "modified" | "modified-desc" | "recent" => Ok(SortOrder::ModifiedDesc),
            "modified-asc" => Ok(SortOrder::ModifiedAsc),
            "created" | "created-desc" => Ok(SortOrder::CreatedDesc),
            "created-asc" => Ok(SortOrder::CreatedAsc),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn note(title: &str, modified: u64, created: u64) -> NoteMeta {
        NoteMeta {
            path: PathBuf::from(format!("/v/{title}.md")),
            rel: format!("{title}.md"),
            title: title.to_string(),
            stem: title.to_string(),
            modified,
            created,
            size: 0,
        }
    }

    fn sorted(order: SortOrder, notes: &mut [NoteMeta]) -> Vec<String> {
        notes.sort_by(|a, b| order.compare(a, b));
        notes.iter().map(|n| n.title.clone()).collect()
    }

    #[test]
    fn modified_desc_puts_the_newest_note_first() {
        let mut notes = vec![
            note("old", 100, 0),
            note("new", 300, 0),
            note("mid", 200, 0),
        ];
        assert_eq!(
            sorted(SortOrder::ModifiedDesc, &mut notes),
            ["new", "mid", "old"]
        );
    }

    #[test]
    fn modified_asc_is_the_exact_reverse() {
        let mut notes = vec![
            note("old", 100, 0),
            note("new", 300, 0),
            note("mid", 200, 0),
        ];
        assert_eq!(
            sorted(SortOrder::ModifiedAsc, &mut notes),
            ["old", "mid", "new"]
        );
    }

    #[test]
    fn created_order_uses_the_creation_stamp_not_the_modified_one() {
        // Deliberately inverted: modified says one thing, created another.
        let mut notes = vec![note("a", 300, 100), note("b", 100, 300)];
        assert_eq!(sorted(SortOrder::CreatedDesc, &mut notes), ["b", "a"]);
        assert_eq!(sorted(SortOrder::ModifiedDesc, &mut notes), ["a", "b"]);
    }

    #[test]
    fn name_order_ignores_case() {
        let mut notes = vec![
            note("banana", 0, 0),
            note("Apple", 0, 0),
            note("cherry", 0, 0),
        ];
        assert_eq!(
            sorted(SortOrder::NameAsc, &mut notes),
            ["Apple", "banana", "cherry"]
        );
    }

    #[test]
    fn name_desc_reverses_it() {
        let mut notes = vec![
            note("banana", 0, 0),
            note("Apple", 0, 0),
            note("cherry", 0, 0),
        ];
        assert_eq!(
            sorted(SortOrder::NameDesc, &mut notes),
            ["cherry", "banana", "Apple"]
        );
    }

    #[test]
    fn ties_fall_back_to_the_name_so_the_order_is_stable() {
        // Same timestamp: without the tiebreak these could come back in any
        // order, and the tree would visibly reshuffle between rebuilds.
        let mut notes = vec![note("zebra", 100, 0), note("apple", 100, 0)];
        assert_eq!(
            sorted(SortOrder::ModifiedDesc, &mut notes),
            ["apple", "zebra"]
        );
        assert_eq!(
            sorted(SortOrder::CreatedDesc, &mut notes),
            ["apple", "zebra"]
        );
    }

    #[test]
    fn the_default_is_most_recently_modified_first() {
        assert_eq!(SortOrder::default(), SortOrder::ModifiedDesc);
    }

    #[test]
    fn next_visits_every_order_and_comes_back_round() {
        let mut seen = vec![SortOrder::default()];
        let mut at = SortOrder::default();
        for _ in 0..SortOrder::ALL.len() - 1 {
            at = at.next();
            assert!(
                !seen.contains(&at),
                "{at:?} repeated before the cycle closed"
            );
            seen.push(at);
        }
        assert_eq!(at.next(), SortOrder::default(), "cycle should wrap");
        assert_eq!(seen.len(), SortOrder::ALL.len());
    }

    #[test]
    fn every_key_round_trips_through_parsing() {
        for order in SortOrder::ALL {
            assert_eq!(order.key().parse(), Ok(order), "{order:?}");
        }
    }

    #[test]
    fn parsing_is_forgiving_about_case_and_underscores() {
        assert_eq!("MODIFIED".parse(), Ok(SortOrder::ModifiedDesc));
        assert_eq!("modified_asc".parse(), Ok(SortOrder::ModifiedAsc));
        assert_eq!("  name  ".parse(), Ok(SortOrder::NameAsc));
        assert_eq!("a-z".parse(), Ok(SortOrder::NameAsc));
    }

    #[test]
    fn an_unknown_value_is_an_error_rather_than_a_silent_default() {
        assert_eq!("sideways".parse::<SortOrder>(), Err(()));
        assert_eq!("".parse::<SortOrder>(), Err(()));
    }

    #[test]
    fn every_order_has_a_distinct_label_and_key() {
        let labels: std::collections::HashSet<_> =
            SortOrder::ALL.iter().map(|o| o.label()).collect();
        let keys: std::collections::HashSet<_> = SortOrder::ALL.iter().map(|o| o.key()).collect();
        assert_eq!(labels.len(), SortOrder::ALL.len());
        assert_eq!(keys.len(), SortOrder::ALL.len());
    }

    #[test]
    fn only_the_name_orders_are_not_time_based() {
        assert!(!SortOrder::NameAsc.is_by_time());
        assert!(!SortOrder::NameDesc.is_by_time());
        assert!(SortOrder::ModifiedDesc.is_by_time());
        assert!(SortOrder::CreatedAsc.is_by_time());
    }
}
