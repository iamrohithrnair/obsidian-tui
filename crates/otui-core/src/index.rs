//! The vault index: notes, links, backlinks and tags.
//!
//! Everything the UI and the agent ask about the vault comes from here, and
//! there is exactly one index per open vault. That single-source-of-truth
//! property is what lets the agent's tools and the user's keystrokes operate on
//! the same state without a sync step between them.
//!
//! The index holds metadata only — never note bodies. A vault of ten thousand
//! notes indexes into a few megabytes, and bodies are read on demand.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::links::{self, Heading, LinkRef};
use crate::note::{self, Frontmatter, NoteMeta};
use crate::vault::{Scan, ScanOptions, Vault};

/// Index of a note within [`VaultIndex::notes`]. Stable for the lifetime of an
/// index, but invalidated by a rebuild — never persist one.
pub type NoteId = usize;

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Resolves to a note in this vault.
    Note(NoteId),
    /// Names a note that doesn't exist. Obsidian shows these dimmed and
    /// creates the note when you follow them.
    Unresolved(String),
    /// A `http(s)://` or other non-vault URL.
    External(String),
    /// A file in the vault that isn't a note (image, PDF).
    Attachment(String),
}

/// A link from one note, with its resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLink {
    pub raw: LinkRef,
    pub target: LinkTarget,
    /// The source line the link appears on, trimmed.
    ///
    /// Captured while the body is already in hand so the backlinks pane can
    /// show context without the index re-reading every file in the vault each
    /// time a note changes.
    pub context: String,
}

impl ResolvedLink {
    /// Text to display for the link, honoring `[[target|alias]]`.
    #[must_use]
    pub fn display(&self) -> &str {
        self.raw.alias.as_deref().unwrap_or(&self.raw.target)
    }
}

/// A link pointing *at* a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// The note containing the link.
    pub source: NoteId,
    /// 0-based line in the source note's body.
    pub line: usize,
    /// The source line, trimmed — shown as context in the backlinks pane so
    /// the user can see *how* they were linked without opening the note.
    pub context: String,
}

/// A fully indexed note.
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub meta: NoteMeta,
    pub frontmatter: Frontmatter,
    pub links: Vec<ResolvedLink>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub headings: Vec<Heading>,
    pub words: usize,
    /// Line the body starts on, so outline entries and link positions map back
    /// to real file lines even when frontmatter is present.
    pub body_offset: usize,
}

impl IndexedNote {
    /// Notes this one links to, deduplicated and in document order.
    #[must_use]
    pub fn outgoing(&self) -> Vec<NoteId> {
        let mut seen = Vec::new();
        for link in &self.links {
            if let LinkTarget::Note(id) = link.target {
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
        }
        seen
    }
}

/// The index over one vault.
#[derive(Debug, Clone)]
pub struct VaultIndex {
    pub vault: Vault,
    pub options: ScanOptions,
    notes: Vec<IndexedNote>,
    /// Vault-relative path → id.
    by_rel: HashMap<String, NoteId>,
    /// Lowercased link keys (stems, aliases, paths) → id.
    lookup: HashMap<String, NoteId>,
    backlinks: HashMap<NoteId, Vec<Backlink>>,
    tags: BTreeMap<String, Vec<NoteId>>,
    unresolved: BTreeMap<String, Vec<NoteId>>,
    folders: Vec<String>,
    attachments: Vec<String>,
}

impl VaultIndex {
    /// Builds an index by scanning and reading every note in the vault.
    ///
    /// A note that can't be read is skipped rather than aborting the build: one
    /// unreadable file shouldn't cost the user their whole vault.
    pub fn build(vault: Vault, options: ScanOptions) -> Result<Self, std::io::Error> {
        let scan = vault.scan(&options)?;
        Ok(Self::from_scan(vault, options, scan))
    }

    fn from_scan(vault: Vault, options: ScanOptions, scan: Scan) -> Self {
        let Scan {
            notes: metas,
            folders,
            attachments,
        } = scan;

        // Pass 1: read and parse each note. Link resolution needs the full
        // name table, so it can't happen until every note is known.
        let mut notes: Vec<IndexedNote> = Vec::with_capacity(metas.len());
        let mut raw_links: Vec<Vec<(LinkRef, String)>> = Vec::with_capacity(metas.len());

        for mut meta in metas {
            let content = fs::read_to_string(&meta.path).unwrap_or_default();
            let (fm_text, body, body_offset) = note::split_frontmatter(&content);
            let frontmatter = fm_text.map(note::parse_frontmatter).unwrap_or_default();

            if let Some(title) = frontmatter.title() {
                meta.title = title;
            }

            let mut tags = frontmatter.tags();
            for tag in links::extract_tags(body) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }

            notes.push(IndexedNote {
                meta,
                aliases: frontmatter.aliases(),
                headings: links::extract_headings(body),
                words: links::word_count(body),
                tags,
                frontmatter,
                links: Vec::new(),
                body_offset,
            });
            raw_links.push(links_with_context(body));
        }

        let mut index = Self {
            vault,
            options,
            by_rel: HashMap::new(),
            lookup: HashMap::new(),
            backlinks: HashMap::new(),
            tags: BTreeMap::new(),
            unresolved: BTreeMap::new(),
            notes,
            folders,
            attachments,
        };

        index.rebuild_lookup();

        // Pass 2: resolve links now that every name is known.
        for (id, raw) in raw_links.into_iter().enumerate() {
            index.notes[id].links = index.resolve_links(raw);
        }

        index.rebuild_derived();
        index
    }

    fn resolve_links(&self, raw: Vec<(LinkRef, String)>) -> Vec<ResolvedLink> {
        raw.into_iter()
            .map(|(link, context)| ResolvedLink {
                target: self.classify(&link),
                raw: link,
                context,
            })
            .collect()
    }

    /// Rebuilds the name → id tables.
    ///
    /// Insertion order encodes Obsidian's resolution precedence: a bare
    /// `[[Name]]` prefers an exact path match over a filename match over an
    /// alias, so the weaker keys go in first and the stronger ones overwrite.
    fn rebuild_lookup(&mut self) {
        self.by_rel.clear();
        self.lookup.clear();

        for (id, note) in self.notes.iter().enumerate() {
            for alias in &note.aliases {
                self.lookup.entry(alias.to_lowercase()).or_insert(id);
            }
        }
        for (id, note) in self.notes.iter().enumerate() {
            // First note wins a contested filename, matching the sorted scan
            // order so the result is deterministic rather than filesystem-dependent.
            self.lookup
                .entry(note.meta.stem.to_lowercase())
                .or_insert(id);
        }
        for (id, note) in self.notes.iter().enumerate() {
            let rel = note.meta.rel.to_lowercase();
            self.lookup.insert(rel.clone(), id);
            if let Some(stripped) = rel.strip_suffix(".md") {
                self.lookup.insert(stripped.to_string(), id);
            }
            self.by_rel.insert(note.meta.rel.clone(), id);
        }
    }

    /// Rebuilds backlinks, the tag table and the unresolved-link table.
    fn rebuild_derived(&mut self) {
        self.backlinks.clear();
        self.tags.clear();
        self.unresolved.clear();

        for id in 0..self.notes.len() {
            for tag in self.notes[id].tags.clone() {
                // Nested tags are also members of each ancestor, so `#a/b`
                // shows up when browsing `#a` — as it does in Obsidian.
                let mut prefix = String::new();
                for segment in tag.split('/') {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(segment);
                    let bucket = self.tags.entry(prefix.clone()).or_default();
                    if !bucket.contains(&id) {
                        bucket.push(id);
                    }
                }
            }

            for link in self.notes[id].links.clone() {
                match &link.target {
                    LinkTarget::Note(target) => {
                        self.backlinks.entry(*target).or_default().push(Backlink {
                            source: id,
                            line: link.raw.line,
                            context: link.context.clone(),
                        });
                    }
                    LinkTarget::Unresolved(name) => {
                        let bucket = self.unresolved.entry(name.clone()).or_default();
                        if !bucket.contains(&id) {
                            bucket.push(id);
                        }
                    }
                    LinkTarget::External(_) | LinkTarget::Attachment(_) => {}
                }
            }
        }
    }

    fn classify(&self, link: &LinkRef) -> LinkTarget {
        if is_external(&link.target) {
            return LinkTarget::External(link.target.clone());
        }
        if let Some(id) = self.resolve(&link.target) {
            return LinkTarget::Note(id);
        }
        // A link to a real non-note file is an attachment, not a broken link.
        if let Some(path) = self.find_attachment(&link.target) {
            return LinkTarget::Attachment(path);
        }
        // A Markdown link with an extension we don't handle is a file
        // reference, not a note the user meant to create.
        if link.markdown && Path::new(&link.target).extension().is_some() {
            return LinkTarget::Attachment(link.target.clone());
        }
        LinkTarget::Unresolved(link.target.clone())
    }

    fn find_attachment(&self, target: &str) -> Option<String> {
        let needle = target.to_lowercase();
        self.attachments
            .iter()
            .find(|path| {
                let lower = path.to_lowercase();
                lower == needle
                    || lower
                        .rsplit('/')
                        .next()
                        .is_some_and(|file_name| file_name == needle)
            })
            .cloned()
    }

    // ---- queries ---------------------------------------------------------

    #[must_use]
    pub fn notes(&self) -> &[IndexedNote] {
        &self.notes
    }

    #[must_use]
    pub fn note(&self, id: NoteId) -> Option<&IndexedNote> {
        self.notes.get(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    #[must_use]
    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    #[must_use]
    pub fn attachments(&self) -> &[String] {
        &self.attachments
    }

    /// Finds the file a non-note link points at, as an absolute path.
    ///
    /// `from` is the directory of the note holding the link, which is where a
    /// relative target like `./diagram.png` or `assets/diagram.png` is looked up
    /// first. A bare filename falls back to the whole vault, because that is how
    /// Obsidian resolves `![[diagram.png]]` no matter which folder it sits in.
    #[must_use]
    pub fn attachment_path(&self, target: &str, from: Option<&Path>) -> Option<PathBuf> {
        if target.is_empty() || is_external(target) {
            return None;
        }

        // A target may be percent-encoded, since that is what an editor writes
        // when a filename contains a space.
        let decoded = percent_decode(target);
        let candidates = [decoded.as_str(), target];

        for candidate in candidates {
            if let Some(dir) = from {
                let path = dir.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
            let path = self.vault.path.join(candidate);
            if path.is_file() {
                return Some(path);
            }
            if let Some(rel) = self.find_attachment(candidate) {
                return Some(self.vault.path.join(rel));
            }
        }
        None
    }

    #[must_use]
    pub fn id_of_rel(&self, rel: &str) -> Option<NoteId> {
        self.by_rel.get(rel).copied()
    }

    /// Resolves a link target the way Obsidian does.
    ///
    /// Accepts a vault-relative path (with or without `.md`), a bare filename,
    /// or an alias, case-insensitively.
    #[must_use]
    pub fn resolve(&self, target: &str) -> Option<NoteId> {
        let key = target.trim().trim_end_matches('/').to_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Some(id) = self.lookup.get(&key) {
            return Some(*id);
        }
        // `[[folder/Note]]` written with a leading `./` or `/`.
        let trimmed = key.trim_start_matches(['.', '/']);
        self.lookup.get(trimmed).copied()
    }

    #[must_use]
    pub fn backlinks(&self, id: NoteId) -> &[Backlink] {
        self.backlinks.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Every tag in the vault with the notes carrying it, sorted by tag name.
    #[must_use]
    pub fn tags(&self) -> &BTreeMap<String, Vec<NoteId>> {
        &self.tags
    }

    /// Link targets with no note behind them, and who links to them. This is
    /// what the graph draws as hollow nodes.
    #[must_use]
    pub fn unresolved(&self) -> &BTreeMap<String, Vec<NoteId>> {
        &self.unresolved
    }

    /// Reads a note's full file content, frontmatter included.
    pub fn read(&self, id: NoteId) -> Result<String, std::io::Error> {
        let note = self.notes.get(id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no note with id {id}"),
            )
        })?;
        fs::read_to_string(&note.meta.path)
    }

    /// Reads a note's body, with frontmatter stripped.
    pub fn read_body(&self, id: NoteId) -> Result<String, std::io::Error> {
        let content = self.read(id)?;
        let (_, body, _) = note::split_frontmatter(&content);
        Ok(body.to_string())
    }

    /// Summary counts for the status bar.
    #[must_use]
    pub fn stats(&self) -> IndexStats {
        IndexStats {
            notes: self.notes.len(),
            folders: self.folders.len(),
            attachments: self.attachments.len(),
            tags: self.tags.len(),
            links: self
                .notes
                .iter()
                .map(|n| {
                    n.links
                        .iter()
                        .filter(|l| matches!(l.target, LinkTarget::Note(_)))
                        .count()
                })
                .sum(),
            unresolved: self.unresolved.len(),
            words: self.notes.iter().map(|n| n.words).sum(),
        }
    }

    // ---- mutation --------------------------------------------------------

    /// Re-reads one note and refreshes everything derived from it.
    ///
    /// Used after an edit or an external change. Re-resolving links vault-wide
    /// is necessary because creating a note can turn other notes' broken links
    /// into working ones — and it's cheap, since no file is re-read.
    pub fn refresh_note(&mut self, id: NoteId) -> Result<(), std::io::Error> {
        let path = self
            .notes
            .get(id)
            .map(|n| n.meta.path.clone())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no note with id {id}"),
                )
            })?;

        let content = fs::read_to_string(&path)?;
        let (fm_text, body, body_offset) = note::split_frontmatter(&content);
        let frontmatter = fm_text.map(note::parse_frontmatter).unwrap_or_default();

        let mut tags = frontmatter.tags();
        for tag in links::extract_tags(body) {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }

        let note = &mut self.notes[id];
        note.meta.title = frontmatter
            .title()
            .unwrap_or_else(|| note.meta.stem.clone());
        note.meta.size = body.len() as u64;
        note.aliases = frontmatter.aliases();
        note.headings = links::extract_headings(body);
        note.words = links::word_count(body);
        note.tags = tags;
        note.frontmatter = frontmatter;
        note.body_offset = body_offset;

        let raw = links_with_context(body);
        self.rebuild_lookup();
        self.notes[id].links = self.resolve_links(raw);
        // Other notes' links are re-resolved too: this edit may have added or
        // removed an alias, which changes what *their* links point at.
        self.resolve_all_links();
        self.rebuild_derived();
        Ok(())
    }

    /// Rebuilds the whole index from disk. Used when files change outside the
    /// app, or after an operation that moves many notes at once.
    pub fn rebuild(&mut self) -> Result<(), std::io::Error> {
        let rebuilt = Self::build(self.vault.clone(), self.options.clone())?;
        *self = rebuilt;
        Ok(())
    }

    /// Re-resolves every note's outgoing links against the current name table,
    /// keeping the already-captured line context.
    fn resolve_all_links(&mut self) {
        let raws: Vec<Vec<(LinkRef, String)>> = self
            .notes
            .iter()
            .map(|n| {
                n.links
                    .iter()
                    .map(|l| (l.raw.clone(), l.context.clone()))
                    .collect()
            })
            .collect();
        for (id, raw) in raws.into_iter().enumerate() {
            self.notes[id].links = self.resolve_links(raw);
        }
    }

    /// Registers an already-created note file without a full rebuild.
    pub(crate) fn insert_note(&mut self, meta: NoteMeta) -> Result<NoteId, std::io::Error> {
        // Keeping notes sorted by path means ids shift, so callers must use the
        // returned id rather than assuming append-at-end.
        let position = self
            .notes
            .binary_search_by(|n| n.meta.rel.cmp(&meta.rel))
            .unwrap_or_else(|pos| pos);

        self.notes.insert(
            position,
            IndexedNote {
                frontmatter: Frontmatter::default(),
                links: Vec::new(),
                tags: Vec::new(),
                aliases: Vec::new(),
                headings: Vec::new(),
                words: 0,
                body_offset: 0,
                meta,
            },
        );

        let folder = self.notes[position].meta.folder().to_string();
        if !folder.is_empty() && !self.folders.contains(&folder) {
            self.folders.push(folder);
            self.folders.sort();
        }

        self.rebuild_lookup();
        self.refresh_note(position)?;
        Ok(position)
    }

    pub(crate) fn remove_note(&mut self, id: NoteId) {
        if id < self.notes.len() {
            self.notes.remove(id);
            self.rebuild_lookup();
            self.resolve_all_links();
            self.rebuild_derived();
        }
    }

    pub(crate) fn folders_mut(&mut self) -> &mut Vec<String> {
        &mut self.folders
    }
}

/// Vault-wide counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexStats {
    pub notes: usize,
    pub folders: usize,
    pub attachments: usize,
    pub tags: usize,
    pub links: usize,
    pub unresolved: usize,
    pub words: usize,
}

/// Extracts links along with the trimmed text of the line each one sits on.
fn links_with_context(body: &str) -> Vec<(LinkRef, String)> {
    let lines: Vec<&str> = body.lines().collect();
    links::extract_links(body)
        .into_iter()
        .map(|link| {
            let context = lines
                .get(link.line)
                .map(|line| line.trim().to_string())
                .unwrap_or_default();
            (link, context)
        })
        .collect()
}

/// Decodes `%20`-style escapes, which editors write for spaces in filenames.
///
/// Anything that isn't a valid escape is passed through, so a literal `%` in a
/// filename survives.
fn percent_decode(target: &str) -> String {
    if !target.contains('%') {
        return target.to_string();
    }
    let bytes = target.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escape = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| target.to_string())
}

fn is_external(target: &str) -> bool {
    // A scheme-prefixed target is a URL. `obsidian://` is included so a link
    // into the desktop app isn't mistaken for a missing note.
    const SCHEMES: &[&str] = &[
        "http://",
        "https://",
        "mailto:",
        "obsidian://",
        "ftp://",
        "file://",
    ];
    let lower = target.to_lowercase();
    SCHEMES.iter().any(|s| lower.starts_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempVault;

    #[test]
    fn resolves_links_and_records_backlinks() {
        let vault = TempVault::new("index");
        vault.write("A.md", "Link to [[B]] here.\n");
        vault.write("B.md", "# B\n");

        let index = vault.index();
        let a = index.id_of_rel("A.md").expect("A indexed");
        let b = index.id_of_rel("B.md").expect("B indexed");

        assert_eq!(index.note(a).unwrap().outgoing(), vec![b]);

        let backlinks = index.backlinks(b);
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].source, a);
        assert_eq!(backlinks[0].context, "Link to [[B]] here.");
    }

    #[test]
    fn link_resolution_prefers_path_then_filename_then_alias() {
        let vault = TempVault::new("resolve");
        vault.write("Target.md", "---\naliases: [Nickname]\n---\n");
        vault.write("Folder/Other.md", "x");

        let index = vault.index();
        let target = index.id_of_rel("Target.md").unwrap();
        let other = index.id_of_rel("Folder/Other.md").unwrap();

        assert_eq!(index.resolve("Target"), Some(target), "bare filename");
        assert_eq!(
            index.resolve("target.md"),
            Some(target),
            "case-insensitive path"
        );
        assert_eq!(index.resolve("Nickname"), Some(target), "alias");
        assert_eq!(
            index.resolve("Folder/Other"),
            Some(other),
            "path with folder"
        );
        assert_eq!(index.resolve("Nope"), None);
    }

    #[test]
    fn unresolved_links_are_tracked_not_dropped() {
        let vault = TempVault::new("unresolved");
        vault.write("A.md", "[[Ghost]] and [[Ghost]] again\n");

        let index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();

        assert_eq!(
            index.unresolved().get("Ghost").map(Vec::as_slice),
            Some([a].as_slice()),
            "a broken link is a real graph node, listed once per source note"
        );
    }

    #[test]
    fn external_links_are_not_broken_links() {
        let vault = TempVault::new("external");
        vault.write("A.md", "[site](https://example.com)\n");

        let index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        assert!(matches!(
            index.note(a).unwrap().links[0].target,
            LinkTarget::External(_)
        ));
        assert!(index.unresolved().is_empty());
    }

    #[test]
    fn attachments_resolve_by_filename() {
        let vault = TempVault::new("attach-link");
        vault.write("A.md", "![[diagram.png]]\n");
        vault.write("assets/diagram.png", "binary");

        let index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        assert_eq!(
            index.note(a).unwrap().links[0].target,
            LinkTarget::Attachment("assets/diagram.png".into())
        );
    }

    #[test]
    fn attachment_paths_prefer_the_note_s_own_folder() {
        let vault = TempVault::new("attach-path");
        vault.write("notes/A.md", "x\n");
        vault.write("notes/diagram.png", "near");
        vault.write("diagram.png", "far");

        let index = vault.index();
        let from = index.vault.path.join("notes");

        assert_eq!(
            index.attachment_path("diagram.png", Some(&from)),
            Some(from.join("diagram.png")),
            "a sibling file wins over one of the same name at the vault root"
        );
        assert_eq!(
            index.attachment_path("diagram.png", None),
            Some(index.vault.path.join("diagram.png")),
            "without a note to sit next to, the vault is searched"
        );
    }

    #[test]
    fn attachment_paths_survive_encoded_spaces_and_ignore_urls() {
        let vault = TempVault::new("attach-encoded");
        vault.write("my diagram.png", "binary");

        let index = vault.index();

        assert_eq!(
            index.attachment_path("my%20diagram.png", None),
            Some(index.vault.path.join("my diagram.png")),
            "editors percent-encode spaces when they write a link"
        );
        assert_eq!(
            index.attachment_path("https://example.com/a.png", None),
            None
        );
        assert_eq!(index.attachment_path("nothing.png", None), None);
    }

    #[test]
    fn nested_tags_roll_up_to_ancestors() {
        let vault = TempVault::new("tags");
        vault.write("A.md", "#project/alpha\n");

        let index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        assert_eq!(index.tags().get("project"), Some(&vec![a]));
        assert_eq!(index.tags().get("project/alpha"), Some(&vec![a]));
    }

    #[test]
    fn frontmatter_title_overrides_filename() {
        let vault = TempVault::new("title");
        vault.write("raw-slug.md", "---\ntitle: Real Title\n---\nbody\n");

        let index = vault.index();
        let id = index.id_of_rel("raw-slug.md").unwrap();
        assert_eq!(index.note(id).unwrap().meta.title, "Real Title");
        assert_eq!(
            index.resolve("raw-slug"),
            Some(id),
            "links still use the filename"
        );
    }

    #[test]
    fn refreshing_a_note_updates_links_both_ways() {
        let vault = TempVault::new("refresh");
        vault.write("A.md", "no links yet\n");
        vault.write("B.md", "# B\n");

        let mut index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        let b = index.id_of_rel("B.md").unwrap();
        assert!(index.backlinks(b).is_empty());

        vault.write("A.md", "now [[B]]\n");
        index.refresh_note(a).expect("refresh");

        assert_eq!(index.note(a).unwrap().outgoing(), vec![b]);
        assert_eq!(index.backlinks(b).len(), 1);
    }

    #[test]
    fn stats_count_only_resolved_links() {
        let vault = TempVault::new("stats");
        vault.write("A.md", "[[B]] [[Ghost]] [x](https://e.com)\n");
        vault.write("B.md", "b\n");

        let stats = vault.index().stats();
        assert_eq!(stats.notes, 2);
        assert_eq!(stats.links, 1);
        assert_eq!(stats.unresolved, 1);
    }

    #[test]
    fn hostile_unicode_does_not_crash_the_pipeline() {
        // One byte-vs-char bug crashed a real vault, so the whole pipeline gets
        // exercised against the characters that actually appear in notes people
        // paste from the web.
        let vault = TempVault::new("unicode");
        let nasty = concat!(
            "#\u{a0}Find all Python processes using port 8000\n",
            "\u{a0}#tag after a non-breaking space\n",
            "“smart quotes” and em—dashes and … ellipses\n",
            "日本語のノート #日本語タグ\n",
            "emoji 🎉🇯🇵👨‍👩‍👧‍👦 in [[Ünïcödé Note]]\n",
            "RTL: مرحبا [[עברית]]\n",
            "combining: e\u{301}cole #café\n",
            "zero width\u{200b}space #zw\n",
            "`#code` and ```\n#fenced\n```\n",
            "| tablé | ✓ |\n|---|---|\n| ünïcödé | ✗ |\n",
            "> [!nöte] Cällout\n> bödy\n",
        );
        vault.write("Ünïcödé Note.md", nasty);
        vault.write("עברית.md", "# עברית\n\n[[Ünïcödé Note]]\n");

        let index = vault.index();
        assert_eq!(index.len(), 2);

        // Every read path over every note.
        for (id, note) in index.notes().iter().enumerate() {
            let content = index.read(id).expect("read");
            let _ = crate::markdown::parse(&content);
            let _ = crate::markdown::preview(&content, 80);
            let _ = index.backlinks(id);
            let _ = note.words;
        }

        // Links across scripts must still resolve.
        let hebrew = index.id_of_rel("עברית.md").expect("indexed");
        let unicode = index.id_of_rel("Ünïcödé Note.md").expect("indexed");
        assert!(index.note(hebrew).unwrap().outgoing().contains(&unicode));

        // Search and the graph walk the same text.
        let _ = crate::search::search_content(&index, "ünïcödé", Default::default());
        let _ = crate::search::search_notes(&index, "ünï", 10);
        let mut sim =
            crate::graph::Simulation::new(crate::graph::Graph::build(&index, &Default::default()));
        sim.run(50);
    }

    #[test]
    fn empty_vault_indexes_cleanly() {
        let vault = TempVault::new("empty");
        let index = vault.index();
        assert!(index.is_empty());
        assert_eq!(index.stats(), IndexStats::default());
    }
}
