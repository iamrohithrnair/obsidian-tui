//! UI state that outlives a run.
//!
//! Which folders you left open is not configuration — you never typed it, and
//! it would be noise in a file people edit by hand — but losing it on every
//! restart makes a deep vault tedious. It lives beside the config rather than
//! inside the vault, for the same reason sessions do: a vault should stay a
//! plain folder of Markdown that Obsidian and git are both happy with.
//!
//! What is stored is the set of folders left **open**, not the set left closed.
//! That is what keeps "collapsed by default" true for folders that appear
//! later: an unknown folder is absent from the set and so starts shut. Storing
//! the closed ones instead would make every folder added after today spring
//! open.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Everything remembered between runs, for every vault that has been opened.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Keyed by the vault's full path, so two vaults sharing a folder name
    /// don't overwrite each other.
    #[serde(default)]
    vaults: HashMap<String, VaultState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultState {
    #[serde(default)]
    pub expanded_folders: Vec<String>,
}

/// Where the state file lives, next to `config.toml`.
///
/// `OTUI_STATE_FILE` overrides it. That exists so this module can be tested
/// against a temporary file instead of reaching into the real config directory
/// — a test suite that writes there is a test suite that changes the machine it
/// runs on — and it doubles as an escape hatch for anyone who keeps their
/// dotfiles somewhere unusual.
#[must_use]
pub fn path() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("OTUI_STATE_FILE") {
        return Some(PathBuf::from(over));
    }
    Config::path().and_then(|p| p.parent().map(|dir| dir.join("state.json")))
}

fn key(vault: &Path) -> String {
    vault.to_string_lossy().into_owned()
}

impl State {
    /// Reads the state file, or an empty state when there isn't one yet.
    ///
    /// A corrupt file is treated as absent rather than fatal: the worst it
    /// costs is a set of folder positions, which is not worth refusing to
    /// start over.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// What was remembered for a vault, or `None` if it has never been opened.
    ///
    /// The distinction matters: no entry means "first run, close everything",
    /// while an entry with no folders means "the user closed them all".
    #[must_use]
    pub fn vault(&self, vault: &Path) -> Option<&VaultState> {
        self.vaults.get(&key(vault))
    }

    pub fn set_vault(&mut self, vault: &Path, state: VaultState) {
        self.vaults.insert(key(vault), state);
    }

    /// Writes the state file. Failure is silent — this is a convenience, and
    /// an unwritable config directory is not worth an error on the way out.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_that_has_never_been_opened_is_distinct_from_one_with_nothing_open() {
        let mut state = State::default();
        let fresh = Path::new("/vaults/fresh");
        let emptied = Path::new("/vaults/emptied");

        assert!(
            state.vault(fresh).is_none(),
            "an unseen vault has no entry, which is what triggers collapse-all"
        );

        state.set_vault(
            emptied,
            VaultState {
                expanded_folders: Vec::new(),
            },
        );
        assert!(
            state
                .vault(emptied)
                .is_some_and(|v| v.expanded_folders.is_empty()),
            "closing every folder is a choice, and must not read as a first run"
        );
    }

    #[test]
    fn vaults_are_kept_apart_by_full_path_not_by_name() {
        let mut state = State::default();
        let work = Path::new("/home/a/Notes");
        let personal = Path::new("/home/b/Notes");

        state.set_vault(
            work,
            VaultState {
                expanded_folders: vec!["Projects".into()],
            },
        );
        state.set_vault(
            personal,
            VaultState {
                expanded_folders: vec!["Recipes".into()],
            },
        );

        assert_eq!(
            state.vault(work).map(|v| v.expanded_folders.as_slice()),
            Some(["Projects".to_string()].as_slice()),
            "two vaults called Notes must not share a state entry"
        );
        assert_eq!(
            state.vault(personal).map(|v| v.expanded_folders.as_slice()),
            Some(["Recipes".to_string()].as_slice())
        );
    }

    #[test]
    fn a_corrupt_or_missing_file_is_not_fatal() {
        let state: State = serde_json::from_str("{ not json").unwrap_or_default();
        assert!(state.vaults.is_empty());

        // An older file that predates a field still loads.
        let state: State = serde_json::from_str("{}").expect("an empty object is valid");
        assert!(state.vaults.is_empty());
    }
}
