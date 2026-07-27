//! Notes and YAML frontmatter.
//!
//! Obsidian notes are plain Markdown files with an optional YAML frontmatter
//! block. Frontmatter is *optional on read* — a Markdown file dropped into the
//! vault from anywhere else is still a valid note — so every accessor here
//! degrades to a sensible default rather than failing.
//!
//! The YAML parsed is deliberately the subset Obsidian actually uses in
//! frontmatter: scalars, inline lists (`[a, b]`) and block lists (`- a`). A
//! full YAML engine would be a large dependency for a format that, in practice,
//! is three keys deep.

use std::path::{Path, PathBuf};

/// A frontmatter value: either a single scalar or a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

impl FmValue {
    /// The value as a list — a scalar becomes a one-element list.
    ///
    /// Obsidian accepts `tags: foo` and `tags: [foo]` interchangeably, so
    /// callers that want tags or aliases shouldn't have to care which was used.
    #[must_use]
    pub fn as_list(&self) -> Vec<String> {
        match self {
            Self::Scalar(s) if s.is_empty() => Vec::new(),
            Self::Scalar(s) => vec![s.clone()],
            Self::List(items) => items.clone(),
        }
    }

    /// The value as a single string; a list joins with `, `.
    #[must_use]
    pub fn as_scalar(&self) -> String {
        match self {
            Self::Scalar(s) => s.clone(),
            Self::List(items) => items.join(", "),
        }
    }
}

/// Parsed YAML frontmatter, preserving key order so a round-trip through the
/// editor doesn't shuffle a user's file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    entries: Vec<(String, FmValue)>,
}

impl Frontmatter {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&FmValue> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    #[must_use]
    pub fn entries(&self) -> &[(String, FmValue)] {
        &self.entries
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Tags declared in frontmatter, normalized without a leading `#`.
    #[must_use]
    pub fn tags(&self) -> Vec<String> {
        self.get("tags")
            .or_else(|| self.get("tag"))
            .map(|v| {
                v.as_list()
                    .into_iter()
                    .map(|t| t.trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Alternative names this note can be linked by.
    #[must_use]
    pub fn aliases(&self) -> Vec<String> {
        self.get("aliases")
            .or_else(|| self.get("alias"))
            .map(|v| v.as_list().into_iter().filter(|a| !a.is_empty()).collect())
            .unwrap_or_default()
    }

    /// An explicit `title:`, if the note sets one.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        self.get("title")
            .map(FmValue::as_scalar)
            .filter(|t| !t.is_empty())
    }
}

/// Splits a note into its frontmatter block and body.
///
/// Returns `(frontmatter_yaml, body, body_start_line)`. The line number lets
/// the editor and outline report positions against the original file rather
/// than against the body, so jumping to a heading lands on the right line.
///
/// A `---` on line 1 only opens frontmatter if a closing `---` exists; an
/// unterminated block is treated as body text, matching Obsidian, so a note
/// that merely starts with a horizontal rule isn't swallowed.
#[must_use]
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str, usize) {
    let Some(rest) = content.strip_prefix("---") else {
        return (None, content, 0);
    };
    // The opening fence must be alone on its line.
    let rest = match rest.strip_prefix('\n') {
        Some(r) => r,
        None => match rest.strip_prefix("\r\n") {
            Some(r) => r,
            None => return (None, content, 0),
        },
    };

    let mut offset = 0;
    let mut line_no = 1; // the opening `---`
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        line_no += 1;
        if trimmed == "---" || trimmed == "..." {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (Some(yaml), body, line_no);
        }
        offset += line.len();
    }

    (None, content, 0)
}

/// Parses the frontmatter subset Obsidian uses.
#[must_use]
pub fn parse_frontmatter(yaml: &str) -> Frontmatter {
    let mut entries: Vec<(String, FmValue)> = Vec::new();
    let mut lines = yaml.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Only top-level keys are read; nested mappings are rare in Obsidian
        // frontmatter and skipping them beats mis-parsing them.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim();

        if value.is_empty() {
            // A block list follows, indented under the key.
            let mut items = Vec::new();
            while let Some(next) = lines.peek() {
                let t = next.trim();
                if let Some(item) = t.strip_prefix("- ") {
                    items.push(unquote(item.trim()));
                    lines.next();
                } else if t == "-" || t.is_empty() {
                    // A bare dash or a blank line inside a block list carries
                    // no item; skip without ending the list.
                    lines.next();
                } else {
                    break;
                }
            }
            entries.push((
                key,
                if items.is_empty() {
                    FmValue::Scalar(String::new())
                } else {
                    FmValue::List(items)
                },
            ));
        } else if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            let items = inner
                .split(',')
                .map(|s| unquote(s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            entries.push((key, FmValue::List(items)));
        } else {
            entries.push((key, FmValue::Scalar(unquote(value))));
        }
    }

    Frontmatter { entries }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for quote in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(quote) && s.ends_with(quote) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// A note's identity and filesystem facts, without its content.
///
/// The index keeps one of these per note and loads bodies on demand, so opening
/// a 10,000-note vault doesn't mean holding 10,000 note bodies in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMeta {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Vault-relative path with `/` separators, e.g. `Projects/Ideas.md`.
    /// This is the note's stable identity and what wikilinks resolve against.
    pub rel: String,
    /// Display name: the frontmatter `title:` if set, else the file stem.
    pub title: String,
    /// File stem — what `[[Bare Name]]` links match on.
    pub stem: String,
    /// Seconds since the Unix epoch, or 0 if unavailable.
    pub modified: u64,
    pub size: u64,
}

impl NoteMeta {
    /// The vault-relative folder containing this note (`""` at the root).
    #[must_use]
    pub fn folder(&self) -> &str {
        match self.rel.rfind('/') {
            Some(idx) => &self.rel[..idx],
            None => "",
        }
    }
}

/// Converts an absolute path inside `vault_root` to a vault-relative path with
/// forward slashes, so note identity is stable across platforms.
#[must_use]
pub fn relative_path(vault_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(vault_root).ok()?;
    let mut out = String::new();
    for component in rel.components() {
        let part = component.as_os_str().to_str()?;
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter_and_reports_body_line() {
        let content = "---\ntitle: Hi\n---\n# Body\n";
        let (fm, body, line) = split_frontmatter(content);
        assert_eq!(fm, Some("title: Hi\n"));
        assert_eq!(body, "# Body\n");
        assert_eq!(line, 3, "body starts on the line after the closing fence");
    }

    #[test]
    fn unterminated_frontmatter_is_body() {
        let content = "---\ntitle: Hi\n# Body\n";
        let (fm, body, line) = split_frontmatter(content);
        assert_eq!(fm, None);
        assert_eq!(
            body, content,
            "no closing fence means it was never frontmatter"
        );
        assert_eq!(line, 0);
    }

    #[test]
    fn note_without_frontmatter_is_left_alone() {
        let content = "# Just a note\n";
        let (fm, body, _) = split_frontmatter(content);
        assert_eq!(fm, None);
        assert_eq!(body, content);
    }

    #[test]
    fn parses_scalars_inline_and_block_lists() {
        let fm = parse_frontmatter(
            "title: My Note\ntags: [alpha, beta]\naliases:\n  - First\n  - \"Second\"\ndraft: true\n",
        );
        assert_eq!(fm.title().as_deref(), Some("My Note"));
        assert_eq!(fm.tags(), vec!["alpha", "beta"]);
        assert_eq!(fm.aliases(), vec!["First", "Second"]);
        assert_eq!(
            fm.get("draft").map(FmValue::as_scalar).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn tags_accept_scalar_and_strip_hashes() {
        assert_eq!(parse_frontmatter("tags: solo\n").tags(), vec!["solo"]);
        assert_eq!(parse_frontmatter("tags: [#a, #b]\n").tags(), vec!["a", "b"]);
    }

    #[test]
    fn preserves_key_order() {
        let fm = parse_frontmatter("zeta: 1\nalpha: 2\n");
        let keys: Vec<_> = fm.entries().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec!["zeta", "alpha"],
            "order must survive a round trip"
        );
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("/vault");
        let path = Path::new("/vault/Projects/Ideas.md");
        assert_eq!(
            relative_path(root, path).as_deref(),
            Some("Projects/Ideas.md")
        );
    }

    #[test]
    fn folder_of_a_root_note_is_empty() {
        let meta = NoteMeta {
            path: PathBuf::from("/vault/Note.md"),
            rel: "Note.md".into(),
            title: "Note".into(),
            stem: "Note".into(),
            modified: 0,
            size: 0,
        };
        assert_eq!(meta.folder(), "");
    }
}
