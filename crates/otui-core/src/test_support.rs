//! A throwaway vault on disk, for tests.
//!
//! Vault behavior is filesystem behavior — link resolution, renames that
//! rewrite other notes, scans that skip `.obsidian` — so the tests exercise
//! real files rather than a mock. Each vault lives in a uniquely named temp
//! directory and deletes itself on drop.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::index::VaultIndex;
use crate::vault::{ScanOptions, Vault};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TempVault {
    path: PathBuf,
}

impl TempVault {
    #[must_use]
    pub fn new(tag: &str) -> Self {
        let unique = format!(
            "otui-{tag}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create temp vault");
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a file, creating parent folders as needed.
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, content).expect("write file");
        path
    }

    #[must_use]
    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path.join(rel)).unwrap_or_default()
    }

    #[must_use]
    pub fn exists(&self, rel: &str) -> bool {
        self.path.join(rel).exists()
    }

    #[must_use]
    pub fn vault(&self) -> Vault {
        Vault::from_path(&self.path)
    }

    /// Builds a fresh index over the current contents.
    #[must_use]
    pub fn index(&self) -> VaultIndex {
        VaultIndex::build(self.vault(), ScanOptions::default()).expect("build index")
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}
