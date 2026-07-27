//! Configuration file handling.
//!
//! obsidian-tui runs with no config at all; the file only exists to override
//! defaults. It lives outside the vault so a vault stays a plain folder of
//! Markdown that Obsidian and git are both happy with.

use std::fs;
use std::path::{Path, PathBuf};

use otui_agent::{Effort, ProviderKind};
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
    pub agent: AgentConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Named rather than left blank so the generated file documents
            // what the default actually is.
            theme: otui_theme::presets::DEFAULT_NAME.to_string(),
            vault: None,
            ui: UiConfig::default(),
            editor: EditorConfig::default(),
            graph: GraphConfig::default(),
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
                    otui_agent::provider::anthropic::DEFAULT_MODEL.to_string()
                }
                ProviderKind::OpenAiCompatible => {
                    otui_agent::provider::openai::DEFAULT_MODEL.to_string()
                }
                ProviderKind::Offline => "offline".to_string(),
            }
        } else {
            self.model.clone()
        }
    }

    #[must_use]
    pub fn to_session_config(&self) -> otui_agent::AgentConfig {
        otui_agent::AgentConfig {
            model: self.model(),
            max_tokens: self.max_tokens,
            effort: self.effort(),
            show_reasoning: self.show_reasoning,
            max_tool_rounds: self.max_tool_rounds,
        }
    }
}

impl Config {
    /// Where the config file lives, honoring `$XDG_CONFIG_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("obsidian-tui").join("config.toml"))
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
        let dir = std::env::temp_dir().join(format!("otui-cfg-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("otui-cfg-bad-{}", std::process::id()));
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
        let (_, error) = Config::load_from(Path::new("/nonexistent/otui/config.toml"));
        assert!(error.is_none());
    }

    #[test]
    fn agent_model_falls_back_per_provider() {
        let mut agent = AgentConfig::default();
        assert_eq!(agent.model(), "claude-opus-5");

        agent.provider = "ollama".into();
        assert_eq!(agent.model(), otui_agent::provider::openai::DEFAULT_MODEL);

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
}
