//! A small streaming agent runtime: providers, a tool-calling loop, and events.
//!
//! The design follows the shape every tool-using agent harness converges on —
//! stream a reply, run the tools it asks for, feed the results back, repeat —
//! implemented directly in Rust so obsidian-tui stays a single binary with no
//! runtime to install alongside it.
//!
//! ```no_run
//! use otui_agent::{provider::mock::Mock, session, AgentConfig, Message};
//! # use otui_agent::{ToolHost, ToolOutcome, ToolCall, ToolSpec};
//! # struct NoTools;
//! # impl ToolHost for NoTools {
//! #     fn specs(&self) -> Vec<ToolSpec> { Vec::new() }
//! #     fn call(&mut self, _: &ToolCall) -> ToolOutcome { ToolOutcome::ok("", "") }
//! # }
//! let runner = session::spawn(
//!     Box::new(Mock::replying("Hello from the vault.")),
//!     AgentConfig::default(),
//!     "You are a note-taking assistant.".into(),
//!     vec![Message::user("Hi")],
//!     Box::new(NoTools),
//! );
//!
//! for event in runner.events.iter() {
//!     println!("{event:?}");
//! }
//! ```

pub mod catalog;
pub mod error;
pub mod provider;
pub mod session;
pub mod sse;
pub mod tool;
pub mod types;

pub use error::{Error, Result};
pub use provider::{Effort, Provider};
pub use session::{run_turn, spawn, AgentConfig, Runner};
pub use tool::{ChannelToolHost, ToolHost, ToolOutcome, ToolRequest, ToolRequests};
pub use types::{AgentEvent, Message, Role, ToolCall, ToolResult, ToolSpec, Usage};

/// Which backend to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderKind {
    /// Anthropic's Messages API.
    Anthropic,
    /// Any OpenAI-compatible server, including local ones.
    OpenAiCompatible,
    /// No model; the panel explains how to configure one.
    Offline,
}

impl ProviderKind {
    /// The protocol a provider name speaks, per the [catalog].
    ///
    /// [catalog]: crate::catalog
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        catalog::find(name).map(|preset| preset.kind.clone())
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCompatible => "openai",
            Self::Offline => "offline",
        }
    }
}

/// Builds a provider, falling back to the offline notice when credentials are
/// missing.
///
/// A missing key is a configuration state, not a crash: the chat panel opens,
/// explains what to set, and the rest of the app is unaffected.
#[must_use]
pub fn build_provider(
    kind: &ProviderKind,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Box<dyn Provider> {
    build_provider_with_key(kind, base_url, model, None)
}

/// Builds a provider with a key the caller resolved itself.
///
/// The UI stores keys of its own, so it knows about credentials this crate has no
/// business reading files to find. `None` falls back to the environment, which is
/// what a library user with nothing else configured expects.
#[must_use]
pub fn build_provider_with_key(
    kind: &ProviderKind,
    base_url: Option<&str>,
    model: Option<&str>,
    api_key: Option<String>,
) -> Box<dyn Provider> {
    match kind {
        ProviderKind::Anthropic => match provider::anthropic::Anthropic::new(api_key, base_url) {
            Ok(provider) => Box::new(provider),
            Err(_) => Box::new(provider::mock::Mock::offline_notice()),
        },
        ProviderKind::OpenAiCompatible => Box::new(provider::openai::OpenAiCompatible::new(
            base_url, api_key, model,
        )),
        ProviderKind::Offline => Box::new(provider::mock::Mock::offline_notice()),
    }
}

/// Whether the configured provider has usable credentials.
///
/// Used to show a hint in the UI before the user sends a doomed first message.
#[must_use]
pub fn is_configured(kind: &ProviderKind, base_url: Option<&str>) -> bool {
    has_credentials(kind, base_url, None)
}

/// As [`is_configured`], for a key the caller resolved itself.
#[must_use]
pub fn has_credentials(kind: &ProviderKind, base_url: Option<&str>, api_key: Option<&str>) -> bool {
    let given = api_key.is_some_and(|key| !key.trim().is_empty());
    match kind {
        ProviderKind::Anthropic => {
            given || provider::anthropic::Anthropic::from_env(base_url).is_ok()
        }
        // A local server needs no key, so a configured base URL is enough.
        ProviderKind::OpenAiCompatible => {
            given || base_url.is_some() || std::env::var("OPENAI_API_KEY").is_ok()
        }
        ProviderKind::Offline => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kinds_parse_from_friendly_names() {
        assert_eq!(ProviderKind::parse("claude"), Some(ProviderKind::Anthropic));
        assert_eq!(
            ProviderKind::parse("Ollama"),
            Some(ProviderKind::OpenAiCompatible)
        );
        assert_eq!(ProviderKind::parse("none"), Some(ProviderKind::Offline));
        assert_eq!(ProviderKind::parse("gemini"), None);
    }

    #[test]
    fn provider_kind_round_trips() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Offline,
        ] {
            assert_eq!(ProviderKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn offline_kind_builds_the_notice_provider() {
        let provider = build_provider(&ProviderKind::Offline, None, None);
        assert_eq!(provider.id(), "mock");
        assert!(!is_configured(&ProviderKind::Offline, None));
    }

    #[test]
    fn a_local_base_url_counts_as_configured() {
        assert!(is_configured(
            &ProviderKind::OpenAiCompatible,
            Some("http://localhost:11434/v1")
        ));
    }
}
