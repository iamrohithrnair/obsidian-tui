//! Saving and reloading agent conversations.
//!
//! Sessions live outside the vault, next to the config file, for the same
//! reason the config does: a vault should stay a plain folder of Markdown that
//! Obsidian and git are both happy with.
//!
//! The stored record keeps both halves of the chat — the transcript the user
//! reads and the message list the model replays — so resuming restores the
//! conversation rather than just its history.

use std::fs;
use std::path::PathBuf;

use emeraldian_agent::Message;
use serde::{Deserialize, Serialize};

use crate::agent::Entry;

/// A saved conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    /// Seconds since the Unix epoch, for sorting newest-first.
    #[serde(default)]
    pub saved_at: u64,
    /// The vault the conversation was about, so a resume can warn on a mismatch.
    #[serde(default)]
    pub vault: Option<PathBuf>,
    #[serde(default)]
    pub transcript: Vec<Entry>,
    #[serde(default)]
    pub conversation: Vec<Message>,
}

/// A session on disk, without loading its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub name: String,
    pub saved_at: u64,
    pub turns: usize,
}

/// Where sessions are stored.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    crate::config::Config::path().and_then(|p| p.parent().map(|d| d.join("sessions")))
}

/// Turns a user-supplied name into something safe to use as a filename.
///
/// Slashes and dots are the dangerous part: a name like `../config` would
/// otherwise let `/save` write outside the sessions directory.
#[must_use]
pub fn slugify(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "session".to_string()
    } else {
        slug.chars().take(64).collect()
    }
}

#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes a session, overwriting any session of the same name.
pub fn save(session: &Session) -> std::io::Result<PathBuf> {
    let dir = dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory on this platform",
        )
    })?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", slugify(&session.name)));
    let text = serde_json::to_string_pretty(session)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&path, text)?;
    Ok(path)
}

/// Reads a session by name.
pub fn load(name: &str) -> std::io::Result<Session> {
    let dir = dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory on this platform",
        )
    })?;
    let path = dir.join(format!("{}.json", slugify(name)));
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Every saved session, newest first.
#[must_use]
pub fn list() -> Vec<SessionInfo> {
    let Some(dir) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut sessions: Vec<SessionInfo> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let text = fs::read_to_string(e.path()).ok()?;
            let session: Session = serde_json::from_str(&text).ok()?;
            Some(SessionInfo {
                name: session.name,
                saved_at: session.saved_at,
                turns: session.conversation.len(),
            })
        })
        .collect();

    sessions.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(a.name.cmp(&b.name)));
    sessions
}

/// Deletes a saved session.
pub fn delete(name: &str) -> std::io::Result<()> {
    let dir = dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory on this platform",
        )
    })?;
    fs::remove_file(dir.join(format!("{}.json", slugify(name))))
}

/// A name derived from the first thing the user said, so `/save` with no
/// argument still produces something recognizable in `/sessions`.
#[must_use]
pub fn suggested_name(transcript: &[Entry]) -> String {
    let first = transcript.iter().find_map(|entry| match entry {
        Entry::User(text) => Some(text.as_str()),
        _ => None,
    });
    match first {
        Some(text) => {
            let words: Vec<&str> = text.split_whitespace().take(6).collect();
            slugify(&words.join("-"))
        }
        None => format!("session-{}", now()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_strips_path_traversal() {
        // The whole point: a name can never escape the sessions directory.
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert!(!slugify("../evil").contains('/'));
        assert!(!slugify("a/b").contains('/'));
    }

    #[test]
    fn slugify_keeps_readable_names() {
        assert_eq!(slugify("Vault Cleanup"), "vault-cleanup");
        assert_eq!(slugify("notes_2024"), "notes_2024");
    }

    #[test]
    fn an_unusable_name_still_produces_a_file() {
        assert_eq!(slugify("   "), "session");
        assert_eq!(slugify("///"), "session");
    }

    #[test]
    fn long_names_are_capped() {
        assert_eq!(slugify(&"a".repeat(200)).len(), 64);
    }

    #[test]
    fn a_name_is_suggested_from_the_first_question() {
        let transcript = vec![
            Entry::Context("attached: A.md".into()),
            Entry::User("what links to my project note?".into()),
            Entry::Assistant("three notes do".into()),
        ];
        assert_eq!(suggested_name(&transcript), "what-links-to-my-project-note");
    }

    #[test]
    fn an_empty_transcript_still_gets_a_name() {
        assert!(suggested_name(&[]).starts_with("session-"));
    }

    #[test]
    fn sessions_round_trip_through_json() {
        let session = Session {
            name: "demo".into(),
            saved_at: 42,
            vault: Some(PathBuf::from("/vault")),
            transcript: vec![Entry::User("hi".into()), Entry::Assistant("hello".into())],
            conversation: vec![Message::user("hi")],
        };
        let text = serde_json::to_string(&session).expect("serialize");
        let back: Session = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back.name, "demo");
        assert_eq!(back.transcript.len(), 2);
        assert_eq!(back.conversation.len(), 1);
    }

    #[test]
    fn sessions_live_beside_the_config_not_in_the_vault() {
        if let (Some(dir), Some(config)) = (dir(), crate::config::Config::path()) {
            assert_eq!(dir.parent(), config.parent());
            assert!(dir.ends_with("sessions"));
        }
    }
}
