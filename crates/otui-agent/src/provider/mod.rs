//! Provider connectors.
//!
//! A provider turns a [`Request`] into a streamed [`Completion`]. Two are
//! built in — Anthropic's Messages API and the OpenAI-compatible chat
//! completions shape, which covers Ollama, LM Studio, vLLM and OpenRouter —
//! plus an offline [`mock`] provider so the chat panel, its tools and its tests
//! all work without network or credentials.
//!
//! Each connector splits into a pure decoder over a byte stream and a thin HTTP
//! wrapper. The decoders are where the protocol complexity lives, and keeping
//! them independent of the network is what makes them testable against captured
//! fixtures.

pub mod anthropic;
pub mod mock;
pub mod openai;

use std::io::Read;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::{Message, ToolCall, ToolSpec, Usage};

/// How much thinking and token budget to spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
    XHigh,
    Max,
}

impl Effort {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

/// One completion request.
pub struct Request<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
    pub max_tokens: u32,
    pub effort: Effort,
    /// Whether to ask for visible reasoning. Off keeps the panel terse.
    pub show_reasoning: bool,
}

/// Incremental output during a stream.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    /// The model wants tools run; the loop should execute them and continue.
    ToolUse,
    MaxTokens,
    /// Safety classifiers declined. Arrives as HTTP 200, so it must be checked
    /// before the content is read.
    Refusal(Option<String>),
    Other(String),
}

impl StopReason {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "end_turn" | "stop" => Self::EndTurn,
            "tool_use" | "tool_calls" => Self::ToolUse,
            "max_tokens" | "length" => Self::MaxTokens,
            "refusal" => Self::Refusal(None),
            other => Self::Other(other.to_string()),
        }
    }
}

/// A finished assistant turn.
#[derive(Debug)]
pub struct Completion {
    /// The assistant content blocks exactly as the provider produced them, to
    /// be replayed on the next request without modification.
    pub content: Value,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

/// A connector to one model backend.
pub trait Provider: Send {
    /// Stable identifier used in config.
    fn id(&self) -> &'static str;

    /// The model used when config doesn't name one.
    fn default_model(&self) -> &'static str;

    /// Runs one request, calling `sink` for each incremental chunk.
    ///
    /// `cancel` is polled between stream events; returning `true` aborts with
    /// [`Error::Cancelled`] rather than waiting for the model to finish.
    fn stream(
        &self,
        request: &Request<'_>,
        cancel: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion>;
}

/// Issues a streaming POST and returns the response body reader.
///
/// HTTP error statuses are turned into [`Error::Http`] with the body attached,
/// because a provider's own error text is the only actionable diagnostic the
/// user gets for a bad model name, an expired key or a rate limit.
pub(crate) fn post_stream(
    url: &str,
    headers: &[(&str, &str)],
    body: &Value,
    timeout_secs: u64,
) -> Result<Box<dyn Read + Send>> {
    let mut request = ureq::post(url)
        .config()
        // Read the body on failure instead of getting a bare status code.
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .build();

    for (key, value) in headers {
        request = request.header(*key, *value);
    }

    let response = request
        .send_json(body)
        .map_err(|err| Error::Transport(err.to_string()))?;

    let status = response.status().as_u16();
    let mut response_body = response.into_body();

    if !(200..300).contains(&status) {
        let text = response_body.read_to_string().unwrap_or_default();
        return Err(Error::Http { status, body: text });
    }

    Ok(Box::new(response_body.into_reader()))
}

/// Reads an environment variable, treating blank as absent.
///
/// An exported-but-empty key is a common shell accident, and failing with
/// "missing key" is far clearer than the 401 it would otherwise cause.
pub(crate) fn env_key(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_round_trips() {
        for effort in [
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::XHigh,
            Effort::Max,
        ] {
            assert_eq!(Effort::parse(effort.as_str()), Some(effort));
        }
        assert_eq!(Effort::parse("nonsense"), None);
    }

    #[test]
    fn stop_reasons_cover_both_provider_vocabularies() {
        assert_eq!(StopReason::parse("end_turn"), StopReason::EndTurn);
        assert_eq!(StopReason::parse("stop"), StopReason::EndTurn);
        assert_eq!(StopReason::parse("tool_use"), StopReason::ToolUse);
        assert_eq!(StopReason::parse("tool_calls"), StopReason::ToolUse);
        assert_eq!(StopReason::parse("length"), StopReason::MaxTokens);
        assert_eq!(StopReason::parse("refusal"), StopReason::Refusal(None));
        assert!(matches!(StopReason::parse("weird"), StopReason::Other(_)));
    }

    #[test]
    fn blank_env_keys_count_as_missing() {
        // SAFETY: single-threaded test process, restored immediately.
        unsafe {
            std::env::set_var("OTUI_TEST_KEY", "   ");
        }
        assert_eq!(env_key("OTUI_TEST_KEY"), None);
        unsafe {
            std::env::set_var("OTUI_TEST_KEY", "real");
        }
        assert_eq!(env_key("OTUI_TEST_KEY").as_deref(), Some("real"));
        unsafe {
            std::env::remove_var("OTUI_TEST_KEY");
        }
    }
}
