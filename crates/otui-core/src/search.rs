//! Fuzzy name matching and full-text search.
//!
//! Two different problems, deliberately solved differently:
//!
//! - The **quick switcher** matches a short query against note names. It needs
//!   to be forgiving (`prj/ide` should find `Projects/Ideas.md`) and to rank
//!   well, so it uses a scoring subsequence matcher.
//! - **Global search** matches a phrase against note bodies. It needs to be
//!   literal — a user searching for `fn main` means those characters — so it
//!   uses substring matching and reads files on demand rather than holding
//!   every body in memory.

use crate::index::{NoteId, VaultIndex};

/// A fuzzy match: how good it is, and which characters matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    /// Byte offsets in the candidate that matched, ascending. The UI underlines
    /// these so the user can see *why* a result matched.
    pub positions: Vec<usize>,
}

const SCORE_MATCH: i32 = 16;
const SCORE_CONSECUTIVE: i32 = 12;
const SCORE_WORD_START: i32 = 10;
const SCORE_CAMEL: i32 = 8;
const SCORE_PREFIX: i32 = 16;
const PENALTY_GAP: i32 = -2;
const PENALTY_LEADING: i32 = -1;
const MAX_LEADING_PENALTY: i32 = -12;

/// Scores `query` against `candidate`, or `None` if the query isn't a
/// subsequence of it.
///
/// Matching is smart-case: an all-lowercase query matches case-insensitively,
/// while any uppercase character makes the whole query case-sensitive. That's
/// the behavior of every editor's fuzzy finder, so it needs no explaining.
#[must_use]
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }

    let case_sensitive = query.chars().any(char::is_uppercase);
    let normalize = |c: char| {
        if case_sensitive {
            c
        } else {
            c.to_ascii_lowercase()
        }
    };

    let cand: Vec<(usize, char)> = candidate.char_indices().collect();
    let mut positions = Vec::new();
    let mut score = 0;
    let mut cand_idx = 0;
    let mut last_match: Option<usize> = None;

    for qc in query.chars() {
        if qc.is_whitespace() {
            continue;
        }
        let qc = normalize(qc);

        let found = cand[cand_idx..]
            .iter()
            .position(|&(_, c)| normalize(c) == qc)
            .map(|offset| cand_idx + offset)?;

        let (byte_pos, _) = cand[found];
        positions.push(byte_pos);

        score += SCORE_MATCH;

        if last_match == Some(found.wrapping_sub(1)) {
            score += SCORE_CONSECUTIVE;
        } else if let Some(last) = last_match {
            // Distance between matches costs, so tightly-clustered matches
            // outrank ones scattered across a long name.
            let gap = (found - last - 1) as i32;
            score += (gap * PENALTY_GAP).max(-16);
        }

        if found == 0 {
            score += SCORE_PREFIX;
        } else {
            let prev = cand[found - 1].1;
            let cur = cand[found].1;
            if matches!(prev, ' ' | '/' | '-' | '_' | '.' | '\\') {
                score += SCORE_WORD_START;
            } else if prev.is_lowercase() && cur.is_uppercase() {
                score += SCORE_CAMEL;
            }
        }

        last_match = Some(found);
        cand_idx = found + 1;
    }

    // A match deep inside a long name is worth less than one near the start.
    if let Some(&first) = positions.first() {
        score += ((first as i32) * PENALTY_LEADING).max(MAX_LEADING_PENALTY);
    }

    Some(FuzzyMatch { score, positions })
}

/// A note matching a quick-switcher query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMatch {
    pub id: NoteId,
    pub score: i32,
    /// Byte offsets into the string that was matched (the note's display name).
    pub positions: Vec<usize>,
}

/// Ranks every note against a quick-switcher query.
///
/// Each note is scored against its title, its path and its aliases, keeping the
/// best of the three — so `projects/ide` and `Ideas` both find the same note.
#[must_use]
pub fn search_notes(index: &VaultIndex, query: &str, limit: usize) -> Vec<NoteMatch> {
    let mut matches: Vec<NoteMatch> = Vec::new();

    for (id, note) in index.notes().iter().enumerate() {
        let mut best: Option<FuzzyMatch> = None;
        let mut best_on_title = false;

        if let Some(m) = fuzzy_match(query, &note.meta.title) {
            best = Some(m);
            best_on_title = true;
        }
        // A path match scores slightly lower so a title hit wins a tie; the
        // title is what the user sees in the list.
        for candidate in std::iter::once(&note.meta.rel).chain(note.aliases.iter()) {
            if let Some(mut m) = fuzzy_match(query, candidate) {
                m.score -= 4;
                if best.as_ref().is_none_or(|b| m.score > b.score) {
                    best = Some(m);
                    best_on_title = false;
                }
            }
        }

        if let Some(m) = best {
            matches.push(NoteMatch {
                id,
                score: m.score,
                positions: if best_on_title {
                    m.positions
                } else {
                    Vec::new()
                },
            });
        }
    }

    matches.sort_by(|a, b| {
        b.score.cmp(&a.score).then_with(|| {
            // Ties break on name so results don't shuffle between keystrokes.
            index.notes()[a.id]
                .meta
                .rel
                .cmp(&index.notes()[b.id].meta.rel)
        })
    });
    matches.truncate(limit);
    matches
}

/// One hit inside a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHit {
    /// 0-based line within the file.
    pub line: usize,
    /// Byte offset of the match within `text`.
    pub column: usize,
    /// The matching line, trimmed for display.
    pub text: String,
}

/// All hits within one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMatch {
    pub id: NoteId,
    pub hits: Vec<ContentHit>,
}

/// Limits on a full-text search, to keep a broad query from stalling the UI.
#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    /// Maximum notes to report.
    pub max_notes: usize,
    /// Maximum hits recorded per note.
    pub max_hits_per_note: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_notes: 200,
            max_hits_per_note: 8,
        }
    }
}

/// Searches note bodies for a literal query.
///
/// Case-insensitive unless the query contains an uppercase character, matching
/// the quick switcher's smart-case rule. Notes are read from disk here rather
/// than cached, so results always reflect what's on disk — including edits made
/// by another program.
#[must_use]
pub fn search_content(index: &VaultIndex, query: &str, limits: SearchLimits) -> Vec<ContentMatch> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let case_sensitive = query.chars().any(char::is_uppercase);
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let mut results = Vec::new();

    for (id, note) in index.notes().iter().enumerate() {
        let Ok(content) = std::fs::read_to_string(&note.meta.path) else {
            continue;
        };

        let mut hits = Vec::new();
        for (line_no, line) in content.lines().enumerate() {
            let haystack = if case_sensitive {
                line.to_string()
            } else {
                line.to_lowercase()
            };
            if let Some(column) = haystack.find(&needle) {
                hits.push(ContentHit {
                    line: line_no,
                    column,
                    text: line.trim().to_string(),
                });
                if hits.len() >= limits.max_hits_per_note {
                    break;
                }
            }
        }

        if !hits.is_empty() {
            results.push(ContentMatch { id, hits });
            if results.len() >= limits.max_notes {
                break;
            }
        }
    }

    // Notes with more hits first — a note mentioning the phrase five times is
    // more likely what the user wants than one mentioning it once.
    results.sort_by(|a, b| {
        b.hits.len().cmp(&a.hits.len()).then_with(|| {
            index.notes()[a.id]
                .meta
                .rel
                .cmp(&index.notes()[b.id].meta.rel)
        })
    });
    results
}

/// Ranks arbitrary strings, for the command palette and theme picker.
#[must_use]
pub fn rank<'a, T>(
    query: &str,
    items: impl IntoIterator<Item = &'a T>,
    key: impl Fn(&T) -> &str,
) -> Vec<(&'a T, FuzzyMatch)>
where
    T: 'a,
{
    let mut scored: Vec<(&T, FuzzyMatch)> = items
        .into_iter()
        .filter_map(|item| fuzzy_match(query, key(item)).map(|m| (item, m)))
        .collect();
    scored.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| key(a.0).cmp(key(b.0)))
    });
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempVault;

    #[test]
    fn fuzzy_requires_a_subsequence() {
        assert!(fuzzy_match("abc", "a-b-c").is_some());
        assert!(fuzzy_match("acb", "a-b-c").is_none());
        assert!(fuzzy_match("", "anything").is_some());
    }

    #[test]
    fn fuzzy_prefers_prefix_and_consecutive_matches() {
        let prefix = fuzzy_match("proj", "Projects").unwrap().score;
        let scattered = fuzzy_match("proj", "Paper Rough Old Jam").unwrap().score;
        assert!(
            prefix > scattered,
            "prefix {prefix} should beat scattered {scattered}"
        );
    }

    #[test]
    fn fuzzy_rewards_word_and_camel_boundaries() {
        let boundary = fuzzy_match("mn", "my note").unwrap().score;
        let mid = fuzzy_match("mn", "moon").unwrap().score;
        assert!(boundary > mid, "{boundary} should beat {mid}");

        assert!(
            fuzzy_match("mn", "myNote").unwrap().score > fuzzy_match("mn", "mynote").unwrap().score
        );
    }

    #[test]
    fn fuzzy_is_smart_case() {
        assert!(fuzzy_match("note", "NOTE").is_some(), "lowercase is loose");
        assert!(
            fuzzy_match("NOTE", "note").is_none(),
            "an uppercase query is strict"
        );
    }

    #[test]
    fn fuzzy_positions_point_at_matched_bytes() {
        let m = fuzzy_match("ac", "abc").unwrap();
        assert_eq!(m.positions, vec![0, 2]);
    }

    #[test]
    fn search_notes_matches_title_path_and_alias() {
        let vault = TempVault::new("search");
        vault.write("Projects/Ideas.md", "---\naliases: [Brainstorm]\n---\n");
        vault.write("Other.md", "x");

        let index = vault.index();
        let ideas = index.id_of_rel("Projects/Ideas.md").unwrap();

        for query in ["ideas", "prj/ide", "brain"] {
            let results = search_notes(&index, query, 10);
            assert_eq!(
                results.first().map(|r| r.id),
                Some(ideas),
                "query {query:?} should find Ideas"
            );
        }
    }

    #[test]
    fn search_notes_is_stable_across_ties() {
        let vault = TempVault::new("stable");
        vault.write("A.md", "x");
        vault.write("B.md", "x");
        let index = vault.index();

        let first = search_notes(&index, "", 10);
        let second = search_notes(&index, "", 10);
        assert_eq!(first, second, "identical queries must give identical order");
    }

    #[test]
    fn content_search_finds_lines_with_context() {
        let vault = TempVault::new("content");
        vault.write("A.md", "alpha\nthe needle here\nbeta\n");
        vault.write("B.md", "nothing\n");

        let index = vault.index();
        let results = search_content(&index, "needle", SearchLimits::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, index.id_of_rel("A.md").unwrap());
        assert_eq!(results[0].hits[0].line, 1);
        assert_eq!(results[0].hits[0].text, "the needle here");
    }

    #[test]
    fn content_search_is_smart_case() {
        let vault = TempVault::new("case");
        vault.write("A.md", "Needle\n");
        let index = vault.index();

        assert_eq!(
            search_content(&index, "needle", SearchLimits::default()).len(),
            1,
            "lowercase query matches any case"
        );
        assert_eq!(
            search_content(&index, "NEEDLE", SearchLimits::default()).len(),
            0,
            "uppercase query is exact"
        );
    }

    #[test]
    fn content_search_ranks_by_hit_count() {
        let vault = TempVault::new("rank");
        vault.write("Few.md", "x\n");
        vault.write("Many.md", "x\nx\nx\n");

        let index = vault.index();
        let results = search_content(&index, "x", SearchLimits::default());
        assert_eq!(results[0].id, index.id_of_rel("Many.md").unwrap());
    }

    #[test]
    fn content_search_respects_limits() {
        let vault = TempVault::new("limits");
        vault.write("A.md", "x\nx\nx\nx\nx\n");
        let index = vault.index();

        let results = search_content(
            &index,
            "x",
            SearchLimits {
                max_notes: 10,
                max_hits_per_note: 2,
            },
        );
        assert_eq!(results[0].hits.len(), 2);
    }

    #[test]
    fn empty_content_query_returns_nothing() {
        let vault = TempVault::new("empty-query");
        vault.write("A.md", "text\n");
        let index = vault.index();
        assert!(search_content(&index, "   ", SearchLimits::default()).is_empty());
    }

    #[test]
    fn rank_orders_arbitrary_strings() {
        let items = vec!["Open Graph View".to_string(), "Close Tab".to_string()];
        let ranked = rank("graph", &items, |s| s.as_str());
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, "Open Graph View");
    }
}
