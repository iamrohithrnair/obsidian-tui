//! API keys on disk.
//!
//! Environment variables are the usual way to hand a key to a program, and they
//! stay the first place looked. But they mean exporting something in every shell
//! that ever starts the reader, which is a poor fit for an app you leave running
//! for days, so a key can also be typed in once and kept.
//!
//! Deliberately separate from `config.toml`: that file is worth committing to a
//! dotfiles repo, and a secret in it is a secret published. This one lives beside
//! it, is written `0600`, and is what the config's comments point people away
//! from.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Stored keys, by provider id.
///
/// Carries the file it came from, so a store can only ever write back to where it
/// was read. That is what keeps the test suite off the developer's own keys.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auth {
    #[serde(default)]
    keys: BTreeMap<String, String>,
    /// `None` for a store with nowhere to live: a system without a config
    /// directory, or a test. Its keys last as long as the process does.
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Auth {
    /// Where the keys live, beside the config file.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        crate::config::Config::path()
            .and_then(|path| path.parent().map(|parent| parent.join(Self::FILE_NAME)))
    }

    const FILE_NAME: &'static str = "auth.json";

    /// Reads the stored keys.
    ///
    /// A missing file is an empty store rather than an error: not having typed a
    /// key in yet is the normal state, and most people never will.
    ///
    /// Under test this deliberately reads nothing and writes nowhere. Otherwise
    /// every test that builds an `App` would depend on whether the machine
    /// running it happens to have keys stored, and a test that logs out would
    /// delete them.
    #[must_use]
    pub fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        Self::path().map_or_else(Self::default, Self::at)
    }

    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        let mut auth: Self = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            // A corrupt file is treated as empty. The alternative is refusing to
            // start over a file the user has probably never seen.
            .unwrap_or_default();
        auth.path = Some(path.to_path_buf());
        auth
    }

    /// A store that will write to `path`, whether or not it exists yet.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::load_from(&path.into())
    }

    /// Whether the keys will outlive the process.
    #[must_use]
    pub fn persists(&self) -> bool {
        self.path.is_some()
    }

    /// The key stored for a provider, if there is one.
    #[must_use]
    pub fn get(&self, provider: &str) -> Option<&str> {
        self.keys
            .get(provider)
            .map(String::as_str)
            .filter(|key| !key.is_empty())
    }

    /// Stores a key, or forgets it if `key` is blank.
    pub fn set(&mut self, provider: &str, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.keys.remove(provider);
        } else {
            self.keys.insert(provider.to_string(), key.to_string());
        }
    }

    /// Writes the keys back where they came from, readable only by their owner.
    ///
    /// A store with nowhere to live succeeds without writing: there is no file to
    /// update, the keys are already held in memory, and [`persists`] is how a
    /// caller tells the user they won't survive a restart.
    ///
    /// [`persists`]: Self::persists
    pub fn save(&self) -> io::Result<()> {
        match &self.path {
            Some(path) => self.save_to(path),
            None => Ok(()),
        }
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        restrict(path);
        Ok(())
    }
}

/// Takes a file's permissions down to owner-only.
///
/// Best-effort: a filesystem that doesn't carry Unix modes — a mounted share, or
/// Windows — leaves the file at whatever the umask gave it, which is no worse
/// than the environment variable it replaces.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The key to use for a provider: the environment first, then the store.
///
/// The environment wins so a key exported for one run — a different account, a
/// colleague's terminal, CI — takes effect without editing a file, and so the
/// behaviour of everything that worked before a key was ever stored is unchanged.
#[must_use]
pub fn key_for(provider: &str, auth: &Auth) -> Option<String> {
    let from_env = emeraldian_agent::catalog::env_var(provider)
        .and_then(|name| std::env::var(name).ok())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    from_env.or_else(|| auth.get(provider).map(str::to_string))
}

/// Where a provider's key is coming from, for `/status` to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An exported variable, named here so it can be reported.
    Env(&'static str),
    /// The `auth.json` store.
    Stored,
    /// No key anywhere. Fine for a local server, fatal for a hosted one.
    Missing,
}

#[must_use]
pub fn source(provider: &str, auth: &Auth) -> Source {
    let name = emeraldian_agent::catalog::env_var(provider);
    match name.filter(|name| std::env::var(name).is_ok_and(|key| !key.trim().is_empty())) {
        Some(name) => Source::Env(name),
        None if auth.get(provider).is_some() => Source::Stored,
        None => Source::Missing,
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Env(name) => write!(f, "${name}"),
            Source::Stored => write!(f, "auth.json"),
            Source::Missing => write!(f, "not set"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emeraldian_core::test_support::TempVault;

    #[test]
    fn a_key_survives_a_round_trip_to_disk() {
        let dir = TempVault::new("auth-roundtrip");
        let path = dir.path().join("nested").join("auth.json");

        let mut auth = Auth::at(&path);
        auth.set("anthropic", "sk-ant-secret");
        auth.save().expect("saved");

        let read = Auth::load_from(&path);
        assert_eq!(read.get("anthropic"), Some("sk-ant-secret"));
        assert_eq!(read.get("openai"), None);
        assert!(read.persists(), "and it knows where it came from");
    }

    #[test]
    fn a_store_with_nowhere_to_live_keeps_its_keys_in_memory() {
        let mut auth = Auth::default();
        auth.set("openai", "sk-session-only");
        assert!(
            auth.save().is_ok(),
            "not being able to write is not an error"
        );
        assert_eq!(auth.get("openai"), Some("sk-session-only"));
        assert!(
            !auth.persists(),
            "but it says so, so the user can be told the key won't last"
        );
    }

    #[test]
    fn the_test_suite_never_reads_or_writes_the_real_keys() {
        // The guarantee that lets a test log out without deleting the keys of
        // whoever is running it.
        assert!(
            !Auth::load().persists(),
            "Auth::load must be inert under cfg(test)"
        );
    }

    #[test]
    fn a_blank_key_forgets_rather_than_storing_an_empty_one() {
        let mut auth = Auth::default();
        auth.set("openai", "sk-old");
        auth.set("openai", "   ");
        assert_eq!(
            auth.get("openai"),
            None,
            "clearing a key is how you go back to the environment"
        );
        assert_eq!(auth.keys.len(), 0, "and leaves nothing behind");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_off_a_pasted_key() {
        let mut auth = Auth::default();
        // A key copied out of a browser often brings a newline with it, and a
        // header with a newline in it is rejected by the server.
        auth.set("openai", " sk-pasted\n");
        assert_eq!(auth.get("openai"), Some("sk-pasted"));
    }

    #[test]
    fn a_file_that_is_not_json_reads_as_no_keys_at_all() {
        let dir = TempVault::new("auth-corrupt");
        let path = dir.path().join("auth.json");
        fs::write(&path, "this is not json").expect("written");
        assert_eq!(
            Auth::load_from(&path).keys.len(),
            0,
            "a corrupt store must not stop the app from opening notes"
        );
        assert_eq!(
            Auth::load_from(&dir.path().join("absent.json")).keys.len(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_not_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempVault::new("auth-mode");
        let path = dir.path().join("auth.json");

        let mut auth = Auth::at(&path);
        auth.set("openai", "sk-secret");
        auth.save().expect("saved");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "a secret is the owner's business");
    }
}
