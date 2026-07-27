//! Integration with Obsidian's official command line interface.
//!
//! Obsidian ships a CLI (<https://obsidian.md/cli>) that drives the desktop
//! app. It is opt-in — the user enables it under Settings → General →
//! "Command line interface", which symlinks the binary onto their `PATH`.
//!
//! Two consequences shape this module:
//!
//! 1. The CLI may simply not be there, so every entry point degrades to a
//!    message explaining how to enable it rather than an error.
//! 2. It is a remote control for the app, not a backend: every command runs
//!    against an Obsidian instance, launching one if none is running.
//!    Everything obsidian-tui does to a vault it does by reading and writing
//!    Markdown directly; the CLI is only used for the things that need the app
//!    itself — listing the vaults it knows about, and opening a note in the GUI.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a CLI call could not be completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The binary was not found in any of the known locations.
    NotInstalled,
    /// The binary ran but reported a failure — most often "app not running".
    Failed(String),
    /// The binary could not be executed at all.
    Spawn(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(
                f,
                "the Obsidian CLI isn't on PATH; enable it in Obsidian under \
                 Settings → General → Command line interface"
            ),
            Self::Failed(message) => write!(f, "obsidian: {message}"),
            Self::Spawn(message) => write!(f, "could not run the Obsidian CLI: {message}"),
        }
    }
}

/// Locations the CLI is installed to, in the order the docs describe them.
///
/// The symlink Obsidian creates is the first hit; the in-bundle binary is the
/// last resort, so a user who never enabled the setting still gets a working
/// "open in Obsidian" on macOS.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(explicit) = std::env::var_os("OBSIDIAN_CLI") {
        paths.push(PathBuf::from(explicit));
    }
    paths.push(PathBuf::from("/usr/local/bin/obsidian"));
    paths.push(PathBuf::from("/opt/homebrew/bin/obsidian"));
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".local/bin/obsidian"));
    }
    paths.push(PathBuf::from(
        "/Applications/Obsidian.app/Contents/MacOS/obsidian-cli",
    ));
    paths
}

/// Finds the CLI, preferring anything already on `PATH`.
#[must_use]
pub fn locate() -> Option<PathBuf> {
    // `PATH` first: if the user enabled the setting, that symlink is the one
    // matching their installed app version.
    if let Some(found) = on_path("obsidian") {
        return Some(found);
    }
    candidates().into_iter().find(|p| is_executable(p))
}

/// Searches `PATH` for an executable, the way a shell would.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs a CLI subcommand and returns its stdout.
///
/// Arguments are passed through verbatim: the CLI's own syntax is
/// `obsidian <command> key=value`, and mirroring it keeps the slash command
/// honest about what it is really doing.
pub fn run(args: &[String]) -> Result<String, Error> {
    let binary = locate().ok_or(Error::NotInstalled)?;
    let output = Command::new(&binary)
        .args(args)
        .output()
        .map_err(|e| Error::Spawn(e.to_string()))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    // Whatever the CLI put on stderr is more useful than any wording we could
    // invent, so it is surfaced verbatim.
    let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.is_empty() {
        message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    if message.is_empty() {
        message = format!("exited with {}", output.status);
    }
    Err(Error::Failed(first_line(&message)))
}

/// The vaults Obsidian knows about.
pub fn vaults() -> Result<Vec<String>, Error> {
    let out = run(&["vaults".to_string()])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Opens a vault-relative note in the Obsidian desktop app.
///
/// `path=` rather than `file=` because obsidian-tui always knows the exact
/// relative path, and `file=` would resolve by name and could land elsewhere.
pub fn open_note(vault: Option<&str>, rel: &str) -> Result<String, Error> {
    run(&open_args(vault, rel))?;
    Ok(format!("opened {rel} in Obsidian"))
}

fn open_args(vault: Option<&str>, rel: &str) -> Vec<String> {
    let mut args = vec!["open".to_string(), format!("path={rel}")];
    if let Some(vault) = vault {
        args.push(format!("vault={vault}"));
    }
    args
}

/// A one-line description of the integration's state, for `/obsidian` and the
/// command palette.
#[must_use]
pub fn status() -> String {
    match locate() {
        None => format!("Obsidian CLI: not found. {}", Error::NotInstalled),
        Some(path) => match vaults() {
            Ok(vaults) if vaults.is_empty() => {
                format!("Obsidian CLI: {} (no vaults reported)", path.display())
            }
            Ok(vaults) => format!(
                "Obsidian CLI: {} / vaults: {}",
                path.display(),
                vaults.join(", ")
            ),
            // The CLI starts Obsidian itself when nothing is running, so a
            // failure here isn't simply "the app is closed". Report what it
            // said rather than guessing at a cause.
            Err(err) => format!("Obsidian CLI: {} / {err}", path.display()),
        },
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_cli_explains_how_to_enable_it() {
        let message = Error::NotInstalled.to_string();
        assert!(message.contains("Settings"), "{message}");
        assert!(message.contains("Command line interface"), "{message}");
    }

    #[test]
    fn failures_are_reported_verbatim() {
        let err = Error::Failed("app is not running".into());
        assert!(err.to_string().contains("app is not running"));
    }

    #[test]
    fn the_bundled_binary_is_the_last_candidate() {
        let candidates = candidates();
        let last = candidates.last().expect("candidates are never empty");
        assert!(
            last.ends_with("obsidian-cli"),
            "the in-bundle binary is a fallback, not a preference: {last:?}"
        );
    }

    #[test]
    fn an_explicit_override_wins() {
        // Set for this test only; `candidates` reads the environment directly
        // so the override has to come first in the list.
        unsafe { std::env::set_var("OBSIDIAN_CLI", "/tmp/obsidian-test-binary") };
        let candidates = candidates();
        assert_eq!(candidates[0], PathBuf::from("/tmp/obsidian-test-binary"));
        unsafe { std::env::remove_var("OBSIDIAN_CLI") };
    }

    #[test]
    fn only_the_first_line_of_an_error_is_kept() {
        assert_eq!(first_line("bad thing\nstack trace\nmore"), "bad thing");
    }

    #[test]
    fn a_directory_is_not_executable() {
        assert!(!is_executable(Path::new("/tmp")));
    }

    #[test]
    fn open_targets_the_exact_path_not_the_name() {
        // Guards the `path=` vs `file=` choice: `file=` resolves by name and
        // could open a different note that happens to share a title.
        assert_eq!(
            open_args(None, "Folder/A.md"),
            vec!["open".to_string(), "path=Folder/A.md".to_string()]
        );
    }

    #[test]
    fn open_can_target_a_named_vault() {
        assert_eq!(
            open_args(Some("Holocron"), "A.md"),
            vec![
                "open".to_string(),
                "path=A.md".to_string(),
                "vault=Holocron".to_string()
            ]
        );
    }
}
