//! Errors from vault operations.

use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// No note, folder or id matching what was asked for.
    NotFound(String),
    /// The target path is already taken.
    AlreadyExists(String),
    /// A note or folder name that can't be written to disk.
    InvalidName(String),
    /// A path that resolves outside the vault root.
    ///
    /// This is the guard that keeps agent tools — which take names from a
    /// language model — from reading or writing anywhere on the filesystem.
    OutsideVault(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::NotFound(what) => write!(f, "not found: {what}"),
            Self::AlreadyExists(what) => write!(f, "already exists: {what}"),
            Self::InvalidName(name) => write!(f, "invalid name: {name}"),
            Self::OutsideVault(path) => {
                write!(f, "path is outside the vault: {}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
