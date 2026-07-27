//! Vault mutations: creating, editing, renaming, moving and deleting notes.
//!
//! Every operation goes through [`VaultIndex`] so the in-memory index and the
//! files on disk can't drift apart. The UI and the agent's tools both call
//! these, which is what lets the agent create a note and have it appear in the
//! explorer without a refresh step.
//!
//! All paths are validated against the vault root before any filesystem call.
//! Note names arrive from user input and from a language model, and neither is
//! trusted to stay inside the vault.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::{Error, Result};
use crate::index::{NoteId, VaultIndex};
use crate::note::{self, NoteMeta};
use crate::vault::TRASH_DIR;

/// Characters that can't appear in a note name.
///
/// This is the union of what Windows, macOS and Linux reject, so a vault
/// written on one platform stays openable on the others.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];

impl VaultIndex {
    /// Resolves a vault-relative path to an absolute one inside the vault.
    ///
    /// Rejects absolute paths and any `..` that would escape the root — the
    /// check is on the *lexical* path, before touching the filesystem, so a
    /// traversal attempt never becomes a real file operation.
    pub fn resolve_path(&self, rel: &str) -> Result<PathBuf> {
        let rel = rel.trim().trim_start_matches('/');
        if rel.is_empty() {
            return Err(Error::InvalidName(rel.to_string()));
        }

        let candidate = Path::new(rel);
        let mut depth = 0i32;
        for component in candidate.components() {
            match component {
                Component::Normal(_) => depth += 1,
                Component::CurDir => {}
                Component::ParentDir => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(Error::OutsideVault(candidate.to_path_buf()));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(Error::OutsideVault(candidate.to_path_buf()));
                }
            }
        }

        Ok(self.vault.path.join(candidate))
    }

    /// Normalizes a note name into a vault-relative `.md` path.
    ///
    /// Accepts `Note`, `Note.md`, `Folder/Note` and `Folder/Note.md` alike,
    /// because that's what a user types and what a model emits.
    pub fn note_rel(&self, name: &str) -> Result<String> {
        let name = name.trim().trim_start_matches('/');
        if name.is_empty() {
            return Err(Error::InvalidName(name.to_string()));
        }

        let (folder, file) = match name.rsplit_once('/') {
            Some((folder, file)) => (folder, file),
            None => ("", name),
        };
        let stem = file.strip_suffix(".md").unwrap_or(file);
        validate_name(stem)?;

        let rel = if folder.is_empty() {
            format!("{stem}.md")
        } else {
            format!("{folder}/{stem}.md")
        };

        // Run the traversal guard over the assembled path.
        self.resolve_path(&rel)?;
        Ok(rel)
    }

    /// Returns `rel` if free, else appends ` 1`, ` 2`, … as Obsidian does.
    #[must_use]
    pub fn unique_rel(&self, rel: &str) -> String {
        if self.id_of_rel(rel).is_none() && !self.vault.path.join(rel).exists() {
            return rel.to_string();
        }
        let stem = rel.strip_suffix(".md").unwrap_or(rel);
        for n in 1..10_000 {
            let candidate = format!("{stem} {n}.md");
            if self.id_of_rel(&candidate).is_none() && !self.vault.path.join(&candidate).exists() {
                return candidate;
            }
        }
        rel.to_string()
    }

    /// Creates a note and returns its id.
    ///
    /// Fails if the note already exists; call [`Self::unique_rel`] first when
    /// the caller would rather get a fresh name than an error.
    pub fn create_note(&mut self, name: &str, content: &str) -> Result<NoteId> {
        let rel = self.note_rel(name)?;
        let path = self.resolve_path(&rel)?;
        if path.exists() {
            return Err(Error::AlreadyExists(rel));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;

        let meta = note_meta(&self.vault.path, &path, &rel)?;
        Ok(self.insert_note(meta)?)
    }

    /// Creates an empty folder so the explorer can show it before it holds
    /// notes.
    pub fn create_folder(&mut self, rel: &str) -> Result<String> {
        let rel = rel.trim().trim_matches('/').to_string();
        if rel.is_empty() {
            return Err(Error::InvalidName(rel));
        }
        for segment in rel.split('/') {
            validate_name(segment)?;
        }
        let path = self.resolve_path(&rel)?;
        if path.exists() {
            return Err(Error::AlreadyExists(rel));
        }
        fs::create_dir_all(&path)?;

        let folders = self.folders_mut();
        if !folders.contains(&rel) {
            folders.push(rel.clone());
            folders.sort();
        }
        Ok(rel)
    }

    /// Replaces a note's entire content and reindexes it.
    pub fn write_note(&mut self, id: NoteId, content: &str) -> Result<()> {
        let path = self
            .note(id)
            .map(|n| n.meta.path.clone())
            .ok_or_else(|| Error::NotFound(format!("note {id}")))?;
        fs::write(&path, content)?;
        self.refresh_note(id)?;
        Ok(())
    }

    /// Appends text to a note, inserting a newline if the file doesn't end
    /// with one — so appending twice doesn't run two lines together.
    pub fn append_note(&mut self, id: NoteId, text: &str) -> Result<()> {
        let mut content = self.read(id)?;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(text);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        self.write_note(id, &content)
    }

    /// Renames a note, rewriting every wikilink that pointed at it.
    ///
    /// Returns the note's new id — ids are positional and a rename reorders the
    /// vault, so the old id must not be reused afterwards.
    pub fn rename_note(&mut self, id: NoteId, new_name: &str) -> Result<NoteId> {
        let note = self
            .note(id)
            .ok_or_else(|| Error::NotFound(format!("note {id}")))?;
        let old_rel = note.meta.rel.clone();
        let old_stem = note.meta.stem.clone();
        let old_path = note.meta.path.clone();

        let new_stem = new_name
            .trim()
            .strip_suffix(".md")
            .unwrap_or_else(|| new_name.trim())
            .to_string();
        validate_name(&new_stem)?;
        if new_stem == old_stem {
            return Ok(id);
        }

        let folder = note.meta.folder().to_string();
        let new_rel = if folder.is_empty() {
            format!("{new_stem}.md")
        } else {
            format!("{folder}/{new_stem}.md")
        };
        let new_path = self.resolve_path(&new_rel)?;
        if new_path.exists() {
            return Err(Error::AlreadyExists(new_rel));
        }

        fs::rename(&old_path, &new_path)?;
        self.update_wikilinks(&old_stem, &new_stem, &old_rel, &new_rel)?;
        self.rebuild()?;

        self.id_of_rel(&new_rel).ok_or(Error::NotFound(new_rel))
    }

    /// Moves a note to another folder, keeping its name and its links.
    ///
    /// Wikilinks are usually written as bare filenames and don't need
    /// rewriting; the path forms that do are handled by [`Self::update_wikilinks`].
    pub fn move_note(&mut self, id: NoteId, new_folder: &str) -> Result<NoteId> {
        let note = self
            .note(id)
            .ok_or_else(|| Error::NotFound(format!("note {id}")))?;
        let old_rel = note.meta.rel.clone();
        let old_path = note.meta.path.clone();
        let stem = note.meta.stem.clone();

        let folder = new_folder.trim().trim_matches('/');
        let new_rel = if folder.is_empty() {
            format!("{stem}.md")
        } else {
            format!("{folder}/{stem}.md")
        };
        if new_rel == old_rel {
            return Ok(id);
        }

        let new_path = self.resolve_path(&new_rel)?;
        if new_path.exists() {
            return Err(Error::AlreadyExists(new_rel));
        }
        if let Some(parent) = new_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::rename(&old_path, &new_path)?;
        self.update_wikilinks(&stem, &stem, &old_rel, &new_rel)?;
        self.rebuild()?;

        self.id_of_rel(&new_rel).ok_or(Error::NotFound(new_rel))
    }

    /// Moves a note to the vault's `.trash` folder.
    ///
    /// Deletion is recoverable by design: this is a destructive operation
    /// reachable from a command palette and from an agent tool, and a wrong
    /// call should cost the user a trip to `.trash`, not their note.
    pub fn delete_note(&mut self, id: NoteId) -> Result<PathBuf> {
        let note = self
            .note(id)
            .ok_or_else(|| Error::NotFound(format!("note {id}")))?;
        let path = note.meta.path.clone();
        let rel = note.meta.rel.clone();

        let trash = self.vault.path.join(TRASH_DIR);
        fs::create_dir_all(&trash)?;

        // Flatten the path so `a/Note.md` and `b/Note.md` can both be trashed,
        // and stamp it so trashing the same note twice doesn't clobber.
        let flattened = rel.replace('/', "_");
        let stem = flattened.strip_suffix(".md").unwrap_or(&flattened);
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let target = trash.join(format!("{stem} {stamp}.md"));

        fs::rename(&path, &target)?;
        self.remove_note(id);
        Ok(target)
    }

    /// Deletes a folder and everything under it, moving it to `.trash`.
    pub fn delete_folder(&mut self, rel: &str) -> Result<PathBuf> {
        let rel = rel.trim().trim_matches('/');
        let path = self.resolve_path(rel)?;
        if !path.is_dir() {
            return Err(Error::NotFound(rel.to_string()));
        }

        let trash = self.vault.path.join(TRASH_DIR);
        fs::create_dir_all(&trash)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let target = trash.join(format!("{} {stamp}", rel.replace('/', "_")));

        fs::rename(&path, &target)?;
        self.rebuild()?;
        Ok(target)
    }

    /// Rewrites `[[old]]`-style links across the vault after a rename or move.
    ///
    /// Covers the four wikilink shapes Obsidian writes — bare, aliased,
    /// heading-anchored, and full-path — for both the old filename and the old
    /// vault-relative path. Markdown links are left alone: they carry a real
    /// relative path whose correctness depends on the linking note's own
    /// location, which this operation doesn't change.
    fn update_wikilinks(
        &self,
        old_stem: &str,
        new_stem: &str,
        old_rel: &str,
        new_rel: &str,
    ) -> Result<()> {
        let old_path_form = old_rel.strip_suffix(".md").unwrap_or(old_rel);
        let new_path_form = new_rel.strip_suffix(".md").unwrap_or(new_rel);

        let mut replacements: Vec<(String, String)> = Vec::new();
        for (old, new) in [
            (old_path_form, new_path_form),
            // The filename form is checked second so a link written as a full
            // path is rewritten as a full path.
            (old_stem, new_stem),
        ] {
            if old == new {
                continue;
            }
            for (open, close) in [("[[", "]]"), ("[[", "|"), ("[[", "#")] {
                replacements.push((format!("{open}{old}{close}"), format!("{open}{new}{close}")));
            }
        }
        if replacements.is_empty() {
            return Ok(());
        }

        for note in self.notes() {
            let Ok(content) = fs::read_to_string(&note.meta.path) else {
                continue;
            };
            let updated = replacements
                .iter()
                .fold(content.clone(), |acc, (old, new)| acc.replace(old, new));
            if updated != content {
                fs::write(&note.meta.path, updated)?;
            }
        }

        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains(FORBIDDEN)
        || trimmed.chars().any(|c| (c as u32) < 0x20)
    {
        return Err(Error::InvalidName(name.to_string()));
    }
    Ok(())
}

fn note_meta(root: &Path, path: &Path, rel: &str) -> Result<NoteMeta> {
    let metadata = fs::metadata(path)?;
    let stamp = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    };
    let modified = stamp(metadata.modified()).unwrap_or(0);
    let created = stamp(metadata.created()).unwrap_or(modified);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel)
        .to_string();

    debug_assert!(path.starts_with(root));
    let _ = note::relative_path(root, path);

    Ok(NoteMeta {
        path: path.to_path_buf(),
        rel: rel.to_string(),
        title: stem.clone(),
        stem,
        modified,
        created,
        size: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempVault;

    #[test]
    fn creates_notes_in_nested_folders() {
        let vault = TempVault::new("create");
        let mut index = vault.index();

        let id = index
            .create_note("Projects/New Idea", "# New Idea\n")
            .expect("create");
        assert_eq!(index.note(id).unwrap().meta.rel, "Projects/New Idea.md");
        assert_eq!(vault.read("Projects/New Idea.md"), "# New Idea\n");
    }

    #[test]
    fn note_names_normalize_with_or_without_extension() {
        let vault = TempVault::new("normalize");
        let index = vault.index();
        assert_eq!(index.note_rel("A").unwrap(), "A.md");
        assert_eq!(index.note_rel("A.md").unwrap(), "A.md");
        assert_eq!(index.note_rel("f/A.md").unwrap(), "f/A.md");
        assert_eq!(index.note_rel("/f/A").unwrap(), "f/A.md");
    }

    #[test]
    fn creating_an_existing_note_is_an_error() {
        let vault = TempVault::new("dup");
        vault.write("A.md", "x");
        let mut index = vault.index();

        let err = index.create_note("A", "y").expect_err("must not overwrite");
        assert!(matches!(err, Error::AlreadyExists(_)));
        assert_eq!(vault.read("A.md"), "x", "existing content is untouched");
    }

    #[test]
    fn unique_rel_suffixes_like_obsidian() {
        let vault = TempVault::new("unique");
        vault.write("A.md", "x");
        vault.write("A 1.md", "x");
        let index = vault.index();

        assert_eq!(index.unique_rel("B.md"), "B.md");
        assert_eq!(index.unique_rel("A.md"), "A 2.md");
    }

    #[test]
    fn path_traversal_is_rejected() {
        let vault = TempVault::new("traversal");
        let mut index = vault.index();

        for attempt in ["../escape", "a/../../escape", "../../../../../../tmp/x"] {
            let err = index
                .create_note(attempt, "x")
                .expect_err("must refuse to write outside the vault");
            assert!(
                matches!(err, Error::OutsideVault(_) | Error::InvalidName(_)),
                "{attempt} produced {err:?}"
            );
        }

        // A leading `/` means the vault root, as it does in Obsidian — so an
        // absolute-looking path is reinterpreted rather than escaping.
        let id = index
            .create_note("/etc/passwd", "x")
            .expect("vault-relative");
        assert_eq!(index.note(id).unwrap().meta.rel, "etc/passwd.md");
        assert!(index.note(id).unwrap().meta.path.starts_with(vault.path()));

        // A `..` that stays inside the vault is harmless.
        assert_eq!(index.note_rel("a/../B").unwrap(), "a/../B.md");
    }

    #[test]
    fn invalid_names_are_rejected() {
        let vault = TempVault::new("names");
        let index = vault.index();
        for bad in ["", "  ", "a:b", "a*b", "a?b", "a\"b"] {
            assert!(index.note_rel(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn rename_rewrites_links_across_the_vault() {
        let vault = TempVault::new("rename");
        vault.write("Old.md", "# Old\n");
        vault.write("A.md", "see [[Old]] and [[Old|alias]] and [[Old#Head]]\n");
        vault.write("B.md", "unrelated\n");

        let mut index = vault.index();
        let old_id = index.id_of_rel("Old.md").unwrap();
        let new_id = index.rename_note(old_id, "New").expect("rename");

        assert_eq!(index.note(new_id).unwrap().meta.rel, "New.md");
        assert_eq!(
            vault.read("A.md"),
            "see [[New]] and [[New|alias]] and [[New#Head]]\n"
        );
        assert_eq!(vault.read("B.md"), "unrelated\n");
        assert!(!vault.exists("Old.md"));

        // The link must actually resolve again, not just look right.
        let a = index.id_of_rel("A.md").unwrap();
        assert_eq!(index.note(a).unwrap().outgoing(), vec![new_id]);
    }

    #[test]
    fn rename_onto_an_existing_name_fails_without_touching_disk() {
        let vault = TempVault::new("rename-clash");
        vault.write("A.md", "a");
        vault.write("B.md", "b");

        let mut index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        assert!(matches!(
            index.rename_note(a, "B"),
            Err(Error::AlreadyExists(_))
        ));
        assert_eq!(vault.read("A.md"), "a");
        assert_eq!(vault.read("B.md"), "b");
    }

    #[test]
    fn move_updates_path_style_links() {
        let vault = TempVault::new("move");
        vault.write("Note.md", "# Note\n");
        vault.write("A.md", "bare [[Note]] and path [[Note]]\n");

        let mut index = vault.index();
        let id = index.id_of_rel("Note.md").unwrap();
        let moved = index.move_note(id, "Archive").expect("move");

        assert_eq!(index.note(moved).unwrap().meta.rel, "Archive/Note.md");
        let a = index.id_of_rel("A.md").unwrap();
        assert_eq!(
            index.note(a).unwrap().outgoing(),
            vec![moved],
            "bare filename links keep resolving after a move"
        );
    }

    #[test]
    fn delete_moves_to_trash_and_deindexes() {
        let vault = TempVault::new("delete");
        vault.write("Gone.md", "bye");
        let mut index = vault.index();
        let id = index.id_of_rel("Gone.md").unwrap();

        let trashed = index.delete_note(id).expect("delete");
        assert!(!vault.exists("Gone.md"));
        assert!(trashed.exists(), "the file is recoverable from .trash");
        assert!(index.id_of_rel("Gone.md").is_none());
    }

    #[test]
    fn append_keeps_lines_separate() {
        let vault = TempVault::new("append");
        vault.write("A.md", "first");
        let mut index = vault.index();
        let id = index.id_of_rel("A.md").unwrap();

        index.append_note(id, "second").expect("append");
        assert_eq!(vault.read("A.md"), "first\nsecond\n");
    }

    #[test]
    fn writing_a_note_reindexes_its_links() {
        let vault = TempVault::new("write");
        vault.write("A.md", "nothing\n");
        vault.write("B.md", "b\n");

        let mut index = vault.index();
        let a = index.id_of_rel("A.md").unwrap();
        let b = index.id_of_rel("B.md").unwrap();

        index.write_note(a, "now [[B]]\n").expect("write");
        assert_eq!(index.note(a).unwrap().outgoing(), vec![b]);
        assert_eq!(index.backlinks(b).len(), 1);
    }

    #[test]
    fn create_folder_shows_up_before_it_has_notes() {
        let vault = TempVault::new("folder");
        let mut index = vault.index();
        index.create_folder("Empty/Nested").expect("create folder");
        assert!(index.folders().contains(&"Empty/Nested".to_string()));
        assert!(vault.path().join("Empty/Nested").is_dir());
    }
}
