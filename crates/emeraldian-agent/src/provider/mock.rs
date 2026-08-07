//! An offline provider.
//!
//! Two jobs. It backs the agent tests, so the tool loop can be exercised
//! deterministically without a network or an API key. And it's what the chat
//! panel falls back to when no credentials are configured, where it explains
//! how to set them up instead of failing with a bare 401 — the panel stays
//! usable and self-documenting on a fresh install.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::{Completion, Provider, Request, StopReason, StreamEvent};
use crate::error::{Error, Result};
use crate::types::{ToolCall, Usage};

/// One scripted assistant turn.
#[derive(Debug, Clone)]
pub enum Scripted {
    /// Reply with text and end the turn.
    Text(String),
    /// Ask for a tool, then continue to the next scripted turn.
    ToolCall { name: String, arguments: Value },
    /// Fail the turn.
    Error(String),
}

pub struct Mock {
    script: Vec<Scripted>,
    cursor: AtomicUsize,
    /// Reply used once the script runs out.
    fallback: String,
}

impl Mock {
    /// A provider that replays `script`, one entry per assistant turn.
    #[must_use]
    pub fn scripted(script: Vec<Scripted>) -> Self {
        Self {
            script,
            cursor: AtomicUsize::new(0),
            fallback: "Done.".to_string(),
        }
    }

    /// A provider that always answers with the same text.
    #[must_use]
    pub fn replying(text: impl Into<String>) -> Self {
        Self {
            script: Vec::new(),
            cursor: AtomicUsize::new(0),
            fallback: text.into(),
        }
    }

    /// The fallback used when no provider is configured.
    ///
    /// It names the exact environment variable and the config alternative, so
    /// the first thing a user sees in the chat panel is how to make it work.
    #[must_use]
    pub fn offline_notice() -> Self {
        Self::replying(concat!(
            "The assistant isn't connected to a model yet.\n\n",
            "To use Claude, set an API key and restart:\n",
            "    export ANTHROPIC_API_KEY=sk-ant-...\n\n",
            "To run fully offline against a local model, add this to your config\n",
            "(`:config` opens it):\n",
            "    [agent]\n",
            "    provider = \"openai\"\n",
            "    base_url = \"http://localhost:11434/v1\"\n",
            "    model = \"llama3.1\"\n\n",
            "Everything else in emeraldian works without a model.",
        ))
    }
}

impl Provider for Mock {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn default_model(&self) -> &'static str {
        "mock"
    }

    fn stream(
        &self,
        _request: &Request<'_>,
        cancel: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion> {
        if cancel() {
            return Err(Error::Cancelled);
        }

        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        let turn = self
            .script
            .get(index)
            .cloned()
            .unwrap_or_else(|| Scripted::Text(self.fallback.clone()));

        match turn {
            Scripted::Text(text) => {
                // Emit in chunks so consumers exercise their streaming path.
                for chunk in chunks(&text) {
                    if cancel() {
                        return Err(Error::Cancelled);
                    }
                    sink(StreamEvent::Text(chunk.clone()));
                }
                Ok(Completion {
                    content: json!([{ "type": "text", "text": text }]),
                    tool_calls: Vec::new(),
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 0,
                        output_tokens: text.len() as u64 / 4,
                        cache_read_tokens: 0,
                    },
                })
            }
            Scripted::ToolCall { name, arguments } => {
                let id = format!("mock_{index}");
                Ok(Completion {
                    content: json!([{
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": arguments,
                    }]),
                    tool_calls: vec![ToolCall {
                        id,
                        name,
                        arguments,
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                })
            }
            Scripted::Error(message) => Err(Error::Protocol(message)),
        }
    }
}

/// Splits text into word-sized chunks, mimicking token streaming.
fn chunks(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive(' ').map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Effort;
    use crate::types::Message;

    fn request<'a>(messages: &'a [Message]) -> Request<'a> {
        Request {
            model: "mock",
            system: "",
            messages,
            tools: &[],
            max_tokens: 1024,
            effort: Effort::Low,
            show_reasoning: false,
        }
    }

    #[test]
    fn replies_with_streamed_text() {
        let provider = Mock::replying("hello there friend");
        let messages = [Message::user("hi")];
        let mut events = Vec::new();

        let completion = provider
            .stream(&request(&messages), &|| false, &mut |e| events.push(e))
            .expect("stream");

        assert!(events.len() > 1, "text should arrive in chunks");
        assert_eq!(completion.content[0]["text"], "hello there friend");
        assert_eq!(completion.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn scripted_turns_advance_in_order() {
        let provider = Mock::scripted(vec![
            Scripted::ToolCall {
                name: "list_tags".into(),
                arguments: json!({}),
            },
            Scripted::Text("Found them.".into()),
        ]);
        let messages = [Message::user("go")];

        let first = provider
            .stream(&request(&messages), &|| false, &mut |_| {})
            .expect("first turn");
        assert_eq!(first.stop_reason, StopReason::ToolUse);
        assert_eq!(first.tool_calls[0].name, "list_tags");

        let second = provider
            .stream(&request(&messages), &|| false, &mut |_| {})
            .expect("second turn");
        assert_eq!(second.stop_reason, StopReason::EndTurn);
        assert_eq!(second.content[0]["text"], "Found them.");
    }

    #[test]
    fn scripted_errors_propagate() {
        let provider = Mock::scripted(vec![Scripted::Error("boom".into())]);
        let messages = [Message::user("go")];
        let err = provider
            .stream(&request(&messages), &|| false, &mut |_| {})
            .expect_err("should fail");
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn cancellation_is_honored() {
        let provider = Mock::replying("text");
        let messages = [Message::user("hi")];
        let err = provider
            .stream(&request(&messages), &|| true, &mut |_| {})
            .expect_err("should cancel");
        assert!(matches!(err, Error::Cancelled));
    }

    #[test]
    fn offline_notice_tells_the_user_what_to_do() {
        let provider = Mock::offline_notice();
        let messages = [Message::user("hi")];
        let completion = provider
            .stream(&request(&messages), &|| false, &mut |_| {})
            .expect("stream");
        let text = completion.content[0]["text"].as_str().unwrap();

        assert!(text.contains("ANTHROPIC_API_KEY"));
        assert!(
            text.contains("localhost:11434"),
            "offline path is documented"
        );
    }
}
