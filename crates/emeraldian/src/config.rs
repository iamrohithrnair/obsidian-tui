//! Configuration file handling.
//!
//! emeraldian runs with no config at all; the file only exists to override
//! defaults. It lives outside the vault so a vault stays a plain folder of
//! Markdown that Obsidian and git are both happy with.

use std::fs;
use std::path::{Path, PathBuf};

use emeraldian_agent::{Effort, ProviderKind};
use emeraldian_core::sort::SortOrder;
use serde::{Deserialize, Serialize};

/// The whole config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: String,
    /// Vault to open when none is given on the command line.
    pub vault: Option<PathBuf>,
    pub ui: UiConfig,
    pub editor: EditorConfig,
    pub graph: GraphConfig,
    pub images: ImageConfig,
    pub agent: AgentConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Named rather than left blank so the generated file documents
            // what the default actually is.
            theme: emeraldian_theme::presets::DEFAULT_NAME.to_string(),
            vault: None,
            ui: UiConfig::default(),
            editor: EditorConfig::default(),
            graph: GraphConfig::default(),
            images: ImageConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Width of the file explorer, in columns.
    pub sidebar_width: u16,
    /// Width of the outline/backlinks sidebar.
    pub right_sidebar_width: u16,
    /// Width of the agent chat panel.
    pub chat_width: u16,
    pub show_left_sidebar: bool,
    pub show_right_sidebar: bool,
    pub show_chat: bool,
    pub show_ribbon: bool,
    /// Show the shortcut hint bar above the status bar.
    pub show_hints: bool,
    /// Show `.`-prefixed files in the explorer.
    pub show_hidden: bool,
    /// Show the line-number gutter while editing.
    pub line_numbers: bool,
    /// Start notes in reading mode rather than the editor.
    pub reading_mode: bool,
    /// How the explorer orders notes within a folder. One of `modified`,
    /// `modified-asc`, `created`, `created-asc`, `name` or `name-desc`.
    ///
    /// Held as a string rather than an enum because an unrecognised value in
    /// any one field makes serde reject the whole file, and losing every other
    /// setting over a typo in this one would be a poor trade. Read it through
    /// [`UiConfig::sort_order`], which falls back to the default.
    pub sort_order: String,
}

impl UiConfig {
    /// The configured sort order, or the default if the value is unrecognised.
    #[must_use]
    pub fn sort_order(&self) -> SortOrder {
        self.sort_order.parse().unwrap_or_default()
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 30,
            right_sidebar_width: 32,
            chat_width: 46,
            show_left_sidebar: true,
            show_right_sidebar: true,
            show_chat: false,
            show_ribbon: true,
            show_hints: true,
            show_hidden: false,
            line_numbers: true,
            reading_mode: true,
            sort_order: SortOrder::default().key().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub tab_width: usize,
    /// Insert spaces rather than a tab character.
    pub expand_tabs: bool,
    /// Wrap long lines instead of scrolling horizontally.
    pub wrap: bool,
    /// Save automatically when switching away from a modified note.
    pub auto_save: bool,
    /// Folder new notes are created in, vault-relative.
    pub new_note_folder: String,
    /// Folder daily notes live in.
    pub daily_folder: String,
    /// `chrono`-free date format for daily notes: `%Y`, `%m`, `%d` only.
    pub daily_format: String,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_width: 4,
            expand_tabs: true,
            wrap: true,
            auto_save: true,
            new_note_folder: String::new(),
            daily_folder: "Daily".into(),
            daily_format: "%Y-%m-%d".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageConfig {
    /// Draw pictures in the reading pane. Turn this off to get alt text back,
    /// which is what a terminal that cannot draw them shows anyway.
    pub enabled: bool,
    /// Tallest a single picture may be drawn, as a percentage of the reading
    /// pane's height.
    ///
    /// A cap rather than a size: a picture is never scaled up, and a wide one
    /// is bounded by the pane instead. It stops a portrait photo from taking
    /// several screens on its own. Expressed as a share of the pane so that a
    /// full-screen window gets a picture worth looking at — a fixed row count
    /// small enough for a short terminal leaves a diagram unreadable on a big
    /// one. 0 turns pictures off, the same as `enabled = false`.
    pub max_height_percent: u16,
    /// How pictures are drawn, when the terminal's own answer is wrong.
    ///
    /// `auto` asks the terminal, which is right almost everywhere. The
    /// exceptions are terminals that claim a protocol they don't actually
    /// paint — a recorder such as VHS, or a multiplexer that swallows the
    /// escapes — where the picture silently leaves a blank hole. `halfblocks`
    /// is the useful override there: coarse, but drawn out of ordinary text
    /// cells, so it survives anything that can show text at all.
    ///
    /// One of `auto`, `kitty`, `iterm2`, `sixel` or `halfblocks`. Held as a
    /// string so an unrecognised value falls back to `auto` with a warning
    /// rather than refusing to start.
    pub protocol: String,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // Two thirds of the pane: big enough to read a diagram, small
            // enough to leave the prose around it visible.
            max_height_percent: 66,
            protocol: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    pub show_unresolved: bool,
    pub show_tags: bool,
    pub show_attachments: bool,
    pub show_orphans: bool,
    pub show_labels: bool,
    /// Depth used by the local-graph view.
    pub local_depth: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            show_unresolved: true,
            show_tags: false,
            show_attachments: false,
            show_orphans: true,
            show_labels: true,
            local_depth: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// `anthropic`, `openai` (any compatible server), or `offline`.
    pub provider: String,
    pub model: String,
    /// Override the API endpoint. Point this at a local server to run offline.
    pub base_url: Option<String>,
    pub max_tokens: u32,
    /// `low` | `medium` | `high` | `xhigh` | `max`.
    pub effort: String,
    /// Show the model's summarized reasoning in the panel.
    pub show_reasoning: bool,
    /// Let the agent create, edit and delete notes. When false it can still
    /// read and search — a useful setting for a vault you don't want touched.
    pub allow_writes: bool,
    /// Include the open note's content with every message.
    pub include_active_note: bool,
    pub max_tool_rounds: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: String::new(),
            base_url: None,
            max_tokens: 16_000,
            effort: "high".into(),
            show_reasoning: true,
            allow_writes: true,
            include_active_note: true,
            max_tool_rounds: 12,
        }
    }
}

impl AgentConfig {
    #[must_use]
    pub fn provider_kind(&self) -> ProviderKind {
        ProviderKind::parse(&self.provider).unwrap_or(ProviderKind::Offline)
    }

    #[must_use]
    pub fn effort(&self) -> Effort {
        Effort::parse(&self.effort).unwrap_or(Effort::High)
    }

    /// The model to request, falling back to the provider's default.
    #[must_use]
    pub fn model(&self) -> String {
        if self.model.trim().is_empty() {
            match self.provider_kind() {
                ProviderKind::Anthropic => {
                    emeraldian_agent::provider::anthropic::DEFAULT_MODEL.to_string()
                }
                ProviderKind::OpenAiCompatible => {
                    emeraldian_agent::provider::openai::DEFAULT_MODEL.to_string()
                }
                ProviderKind::Offline => "offline".to_string(),
            }
        } else {
            self.model.clone()
        }
    }

    #[must_use]
    pub fn to_session_config(&self) -> emeraldian_agent::AgentConfig {
        emeraldian_agent::AgentConfig {
            model: self.model(),
            max_tokens: self.max_tokens,
            effort: self.effort(),
            show_reasoning: self.show_reasoning,
            max_tool_rounds: self.max_tool_rounds,
        }
    }
}

/// The directory holding the config file, themes, saved sessions, UI state and
/// stored API keys. Honors `$XDG_CONFIG_HOME`.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("emeraldian"))
}

/// Where that directory lived before the project was renamed.
#[must_use]
fn legacy_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("obsidian-tui"))
}

/// Moves a pre-rename config directory into place, once.
///
/// The project was called obsidian-tui until the rename, and everything it keeps
/// — config, themes, saved sessions, UI state and API keys — lived in a directory
/// of that name. Changing the binary's name must not quietly cost the user any of
/// it, least of all the stored keys, so the whole directory is moved across the
/// first time the new one is missing.
///
/// Call this before reading anything out of the config directory.
pub fn migrate_legacy_dir() {
    if let (Some(old), Some(new)) = (legacy_dir(), dir()) {
        migrate_dir(&old, &new);
    }
}

/// The move itself, taking both paths so it can be tested without touching the
/// real config directory or the environment.
///
/// Every failure is deliberately silent. The worst case is that the user starts
/// from defaults, which is also what a first-ever run does; losing the old
/// directory would be far worse, so nothing here deletes or overwrites. An
/// existing `new` always wins — it means the user has already run a renamed
/// build, and their current settings are not to be clobbered by older ones.
fn migrate_dir(old: &Path, new: &Path) {
    if new.exists() || !old.is_dir() {
        return;
    }
    if let Some(parent) = new.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Both sit under the same config directory, so this is a rename within one
    // filesystem: atomic, and cheap regardless of how many sessions are stored.
    let _ = fs::rename(old, new);
}

impl Config {
    /// Where the config file lives, honoring `$XDG_CONFIG_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        dir().map(|dir| dir.join("config.toml"))
    }

    /// Directory scanned for user theme files.
    #[must_use]
    pub fn themes_dir() -> Option<PathBuf> {
        Self::path().and_then(|p| p.parent().map(|d| d.join("themes")))
    }

    /// Loads the config, or defaults if it's absent.
    ///
    /// A malformed file returns defaults plus the parse error, so the app still
    /// starts and can tell the user what's wrong — losing access to your notes
    /// over a stray bracket would be a poor trade.
    #[must_use]
    pub fn load() -> (Self, Option<String>) {
        let Some(path) = Self::path() else {
            return (Self::default(), None);
        };
        Self::load_from(&path)
    }

    #[must_use]
    pub fn load_from(path: &Path) -> (Self, Option<String>) {
        let Ok(text) = fs::read_to_string(path) else {
            return (Self::default(), None);
        };
        match toml::from_str::<Self>(&text) {
            Ok(config) => (config, None),
            Err(err) => (Self::default(), Some(format!("{}: {err}", path.display()))),
        }
    }

    /// Writes the config, creating the directory if needed.
    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no config directory on this platform",
            )
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(&path, text)?;
        Ok(path)
    }

    /// Creates the config file with defaults if it doesn't exist yet.
    pub fn ensure_exists(&self) -> std::io::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory")
        })?;
        if !path.exists() {
            self.save()?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable_without_a_file() {
        let config = Config::default();
        assert_eq!(
            config.theme, "obsidian-dark",
            "the default is named, not blank"
        );
        assert!(config.ui.show_left_sidebar);
        assert_eq!(config.ui.sidebar_width, 30);
        assert!(config.editor.wrap);
        assert_eq!(config.agent.provider_kind(), ProviderKind::Anthropic);
    }

    #[test]
    fn partial_config_keeps_defaults_for_the_rest() {
        let dir = std::env::temp_dir().join(format!("emeraldian-cfg-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        fs::write(&path, "theme = \"nord\"\n\n[ui]\nsidebar_width = 20\n").expect("write");

        let (config, error) = Config::load_from(&path);
        fs::remove_dir_all(&dir).ok();

        assert!(error.is_none());
        assert_eq!(config.theme, "nord");
        assert_eq!(config.ui.sidebar_width, 20);
        assert!(
            config.ui.show_left_sidebar,
            "unspecified keys keep their defaults"
        );
        assert_eq!(config.editor.tab_width, 4);
    }

    #[test]
    fn a_broken_config_reports_the_error_and_still_starts() {
        let dir = std::env::temp_dir().join(format!("emeraldian-cfg-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        fs::write(&path, "theme = [unclosed").expect("write");

        let (config, error) = Config::load_from(&path);
        fs::remove_dir_all(&dir).ok();

        assert!(error.is_some(), "the user must be told what's wrong");
        assert_eq!(config.ui.sidebar_width, UiConfig::default().sidebar_width);
    }

    #[test]
    fn missing_config_is_not_an_error() {
        let (_, error) = Config::load_from(Path::new("/nonexistent/emeraldian/config.toml"));
        assert!(error.is_none());
    }

    #[test]
    fn agent_model_falls_back_per_provider() {
        let mut agent = AgentConfig::default();
        assert_eq!(agent.model(), "claude-opus-5");

        agent.provider = "ollama".into();
        assert_eq!(
            agent.model(),
            emeraldian_agent::provider::openai::DEFAULT_MODEL
        );

        agent.model = "llama3.1".into();
        assert_eq!(agent.model(), "llama3.1");
    }

    #[test]
    fn agent_effort_falls_back_when_misspelled() {
        let agent = AgentConfig {
            effort: "enormous".into(),
            ..Default::default()
        };
        assert_eq!(agent.effort(), Effort::High);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let config = Config {
            theme: "gruvbox-dark".into(),
            ..Default::default()
        };
        let text = toml::to_string_pretty(&config).expect("serialize");
        let parsed: Config = toml::from_str(&text).expect("parse");
        assert_eq!(parsed.theme, "gruvbox-dark");
        assert_eq!(parsed.ui.chat_width, config.ui.chat_width);
    }

    #[test]
    fn an_older_config_without_an_images_section_still_gets_pictures() {
        // Upgrading shouldn't silently turn a feature off, and a file written
        // before the section existed is exactly that case.
        let config: Config = toml::from_str("theme = \"gruvbox-dark\"\n").expect("parse");
        assert!(config.images.enabled);
        assert!(config.images.max_height_percent > 0);
        assert_eq!(
            config.images.protocol, "auto",
            "and the terminal is still the one asked"
        );
    }

    #[test]
    fn the_default_sort_order_is_most_recently_modified_first() {
        assert_eq!(Config::default().ui.sort_order(), SortOrder::ModifiedDesc);
    }

    /// A temp directory holding an `old` and a `new` path, neither created.
    fn migration_paths(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("emeraldian-mig-{tag}-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(&root).expect("temp root");
        (
            root.clone(),
            root.join("obsidian-tui"),
            root.join("emeraldian"),
        )
    }

    #[test]
    fn a_pre_rename_config_directory_is_carried_across_whole() {
        let (root, old, new) = migration_paths("move");
        fs::create_dir_all(old.join("themes")).expect("legacy dir");
        fs::write(old.join("config.toml"), "theme = \"nord\"\n").expect("write config");
        // The keys matter most: re-typing an API key is the one loss a user
        // would actually notice.
        fs::write(
            old.join("auth.json"),
            "{\"keys\":{\"anthropic\":\"sk-test\"}}",
        )
        .expect("write auth");
        fs::write(old.join("themes/mine.toml"), "name = \"mine\"\n").expect("write theme");

        migrate_dir(&old, &new);

        assert!(!old.exists(), "the old directory is moved, not copied");
        assert_eq!(
            fs::read_to_string(new.join("config.toml")).expect("config moved"),
            "theme = \"nord\"\n"
        );
        assert!(
            fs::read_to_string(new.join("auth.json"))
                .expect("keys moved")
                .contains("sk-test")
        );
        assert!(new.join("themes/mine.toml").exists(), "themes come too");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_existing_directory_is_never_clobbered_by_an_older_one() {
        let (root, old, new) = migration_paths("keep");
        fs::create_dir_all(&old).expect("legacy dir");
        fs::write(old.join("config.toml"), "theme = \"stale\"\n").expect("write old");
        fs::create_dir_all(&new).expect("current dir");
        fs::write(new.join("config.toml"), "theme = \"current\"\n").expect("write new");

        migrate_dir(&old, &new);

        assert_eq!(
            fs::read_to_string(new.join("config.toml")).expect("read"),
            "theme = \"current\"\n",
            "having already run a renamed build, the current settings win"
        );
        assert!(old.exists(), "and nothing is deleted either");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_first_ever_run_has_nothing_to_migrate() {
        let (root, old, new) = migration_paths("fresh");

        migrate_dir(&old, &new);

        assert!(!new.exists(), "no directory is conjured out of nothing");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_configured_sort_order_is_read_back() {
        let toml = "[ui]\nsort_order = \"name\"\n";
        let config: Config = toml::from_str(toml).expect("parse");
        assert_eq!(config.ui.sort_order(), SortOrder::NameAsc);
    }

    #[test]
    fn a_nonsense_sort_order_falls_back_without_discarding_the_file() {
        // The whole point of holding this as a string: one bad value must not
        // cost the user every other setting in the file.
        let toml = "[ui]\nsort_order = \"sideways\"\nchat_width = 77\n";
        let config: Config = toml::from_str(toml).expect("a bad value still parses");
        assert_eq!(config.ui.sort_order(), SortOrder::ModifiedDesc);
        assert_eq!(config.ui.chat_width, 77, "the rest of the file survives");
    }

    #[test]
    fn every_sort_order_survives_a_write_and_read() {
        for order in SortOrder::ALL {
            let mut config = Config::default();
            config.ui.sort_order = order.key().to_string();
            let text = toml::to_string_pretty(&config).expect("serialise");
            let parsed: Config = toml::from_str(&text).expect("parse");
            assert_eq!(parsed.ui.sort_order(), order, "{order:?} did not survive");
        }
    }
}
