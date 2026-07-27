//! Conversation, tool and event types shared by every provider.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One turn in the conversation.
///
/// `content` is stored as the raw JSON the provider produced, not a parsed
/// structure. That's deliberate: Anthropic's models return `thinking` blocks
/// carrying signatures that must be echoed back **unchanged** on the next turn,
/// and any lossy round-trip through our own types would break them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl Message {
    /// A plain-text user turn.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: json!([{ "type": "text", "text": text.into() }]),
        }
    }

    /// An assistant turn, holding the provider's content blocks verbatim.
    #[must_use]
    pub fn assistant(content: Value) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    /// A user turn carrying tool results.
    ///
    /// Every result for a batch goes in **one** message: splitting them teaches
    /// the model to stop issuing parallel tool calls.
    #[must_use]
    pub fn tool_results(results: &[ToolResult]) -> Self {
        let blocks: Vec<Value> = results
            .iter()
            .map(|r| {
                json!({
                    "type": "tool_result",
                    "tool_use_id": r.id,
                    "content": r.content,
                    "is_error": r.is_error,
                })
            })
            .collect();
        Self {
            role: Role::User,
            content: Value::Array(blocks),
        }
    }

    /// Concatenated plain text of this message, for display and for measuring
    /// how much of the context a conversation is using.
    #[must_use]
    pub fn text(&self) -> String {
        let Some(blocks) = self.content.as_array() else {
            return self.content.as_str().unwrap_or_default().to_string();
        };
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("")
    }
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    /// What the tool does *and when to call it*. Recent models are
    /// conservative about reaching for tools, and a description that states the
    /// trigger condition measurably raises the rate at which they're used.
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
}

impl ToolSpec {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }

    /// Builds an object schema from `(name, type, description, required)` rows.
    #[must_use]
    pub fn object_schema(fields: &[(&str, &str, &str, bool)]) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, ty, description, is_required) in fields {
            properties.insert(
                (*name).to_string(),
                json!({ "type": ty, "description": description }),
            );
            if *is_required {
                required.push(Value::String((*name).to_string()));
            }
        }
        json!({
            "type": "object",
            "properties": Value::Object(properties),
            "required": Value::Array(required),
        })
    }
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// The result of running a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub content: String,
    /// Errors are returned to the model rather than raised, so it can correct
    /// course instead of the turn dying.
    pub is_error: bool,
}

/// Token accounting for one turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}

/// Events streamed from the agent thread to the UI.
///
/// The UI never blocks on the agent: it drains these between frames, so a slow
/// model can't make the editor stutter.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// A turn began.
    Started,
    /// Incremental summarized reasoning.
    Reasoning(String),
    /// Incremental assistant text.
    Text(String),
    /// The model asked to run a tool.
    ToolCall {
        id: String,
        name: String,
        summary: String,
    },
    /// A tool finished.
    ToolResult {
        id: String,
        ok: bool,
        summary: String,
    },
    /// One assistant response completed; more may follow if tools ran.
    TurnEnd,
    Usage(Usage),
    /// The turn ended in failure. Carries a message meant for the user.
    Failed(String),
    /// The turn is over, successfully or not. Always the last event.
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_has_a_text_block() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.text(), "hello");
    }

    #[test]
    fn assistant_content_round_trips_verbatim() {
        // A thinking block's signature must survive being stored and resent.
        let content = json!([
            { "type": "thinking", "thinking": "", "signature": "abc123" },
            { "type": "text", "text": "Hi" }
        ]);
        let msg = Message::assistant(content.clone());
        assert_eq!(msg.content, content, "content must not be reshaped");
        assert_eq!(msg.text(), "Hi");
    }

    #[test]
    fn tool_results_batch_into_one_message() {
        let msg = Message::tool_results(&[
            ToolResult {
                id: "a".into(),
                content: "one".into(),
                is_error: false,
            },
            ToolResult {
                id: "b".into(),
                content: "boom".into(),
                is_error: true,
            },
        ]);
        let blocks = msg.content.as_array().expect("array");
        assert_eq!(blocks.len(), 2, "a batch is one message, not two");
        assert_eq!(blocks[0]["tool_use_id"], "a");
        assert_eq!(blocks[1]["is_error"], true);
    }

    #[test]
    fn object_schema_marks_required_fields() {
        let schema = ToolSpec::object_schema(&[
            ("name", "string", "note name", true),
            ("body", "string", "content", false),
        ]);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["name"]["type"], "string");
        assert_eq!(schema["required"], json!(["name"]));
    }

    #[test]
    fn usage_accumulates() {
        let mut total = Usage::default();
        total.add(Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
        });
        total.add(Usage {
            input_tokens: 3,
            output_tokens: 2,
            cache_read_tokens: 7,
        });
        assert_eq!(total.input_tokens, 13);
        assert_eq!(total.output_tokens, 7);
        assert_eq!(total.cache_read_tokens, 7);
    }
}
