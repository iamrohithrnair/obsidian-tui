//! The OpenAI-compatible chat-completions API.
//!
//! One connector covers Ollama, LM Studio, vLLM, OpenRouter and OpenAI itself,
//! because they all speak this shape. That's what makes obsidian-tui's agent
//! usable fully offline: point `base_url` at a local Ollama and no data leaves
//! the machine.
//!
//! Conversations are stored in the Anthropic block shape throughout the app, so
//! this module translates in both directions. Keeping one canonical
//! representation means switching provider mid-session doesn't corrupt history.

use std::io::Read;

use serde_json::{Map, Value, json};

use super::{Completion, Provider, Request, StopReason, StreamEvent, post_stream};
use crate::error::{Error, Result};
use crate::sse::SseReader;
use crate::types::{Role, ToolCall, Usage};

pub const ENV_VAR: &str = "OPENAI_API_KEY";
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const TIMEOUT_SECS: u64 = 600;

pub struct OpenAiCompatible {
    api_key: Option<String>,
    base_url: String,
    default_model: String,
}

impl OpenAiCompatible {
    /// Builds a connector.
    ///
    /// The API key is optional: local servers like Ollama don't authenticate,
    /// and requiring a dummy key would be a pointless obstacle.
    #[must_use]
    pub fn new(
        base_url: Option<&str>,
        api_key: Option<String>,
        default_model: Option<&str>,
    ) -> Self {
        Self {
            api_key: api_key.or_else(|| super::env_key(ENV_VAR)),
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
            default_model: default_model.unwrap_or(DEFAULT_MODEL).to_string(),
        }
    }
}

impl Provider for OpenAiCompatible {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn default_model(&self) -> &'static str {
        // The trait returns a static string, so a configured model is surfaced
        // through the session's own config rather than here.
        DEFAULT_MODEL
    }

    fn stream(
        &self,
        request: &Request<'_>,
        cancel: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion> {
        let model = if request.model.is_empty() {
            self.default_model.as_str()
        } else {
            request.model
        };
        let body = build_body(request, model);

        let authorization = self.api_key.as_ref().map(|k| format!("Bearer {k}"));
        let mut headers: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
            ("accept", "text/event-stream"),
        ];
        if let Some(auth) = authorization.as_deref() {
            headers.push(("authorization", auth));
        }

        let url = format!("{}/chat/completions", self.base_url);
        let reader = post_stream(&url, &headers, &body, TIMEOUT_SECS)?;
        decode(reader, cancel, sink)
    }
}

/// Builds the request body, translating the conversation out of block form.
pub(crate) fn build_body(request: &Request<'_>, model: &str) -> Value {
    let mut messages = Vec::new();
    if !request.system.is_empty() {
        messages.push(json!({ "role": "system", "content": request.system }));
    }
    for message in request.messages {
        messages.extend(to_openai_messages(message));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    body.insert("max_tokens".into(), json!(request.max_tokens));
    // Without this, usage is never reported on a streamed response.
    body.insert("stream_options".into(), json!({ "include_usage": true }));

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }

    Value::Object(body)
}

/// Converts one canonical message into the one or more OpenAI messages it maps
/// to. A batch of tool results becomes several `role: "tool"` messages, which
/// is how this API represents what Anthropic packs into a single user turn.
fn to_openai_messages(message: &crate::types::Message) -> Vec<Value> {
    let blocks = match message.content.as_array() {
        Some(blocks) => blocks.clone(),
        None => vec![json!({ "type": "text", "text": message.content })],
    };

    match message.role {
        Role::User => {
            let mut out = Vec::new();
            let mut text = String::new();
            for block in &blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_result") => out.push(json!({
                        "role": "tool",
                        "tool_call_id": block.get("tool_use_id").and_then(Value::as_str).unwrap_or(""),
                        "content": block.get("content").and_then(Value::as_str).unwrap_or(""),
                    })),
                    _ => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                }
            }
            if !text.is_empty() {
                out.push(json!({ "role": "user", "content": text }));
            }
            out
        }
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            for block in &blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                    Some("tool_use") => tool_calls.push(json!({
                        "id": block.get("id").and_then(Value::as_str).unwrap_or(""),
                        "type": "function",
                        "function": {
                            "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                            // Arguments travel as a JSON *string* here, not an object.
                            "arguments": block.get("input").map(ToString::to_string).unwrap_or_else(|| "{}".into()),
                        }
                    })),
                    // Thinking blocks are Anthropic-specific and have no place
                    // to go; dropping them is correct rather than lossy, since
                    // this provider never produced them.
                    _ => {}
                }
            }

            let mut msg = Map::new();
            msg.insert("role".into(), json!("assistant"));
            msg.insert(
                "content".into(),
                if text.is_empty() {
                    Value::Null
                } else {
                    json!(text)
                },
            );
            if !tool_calls.is_empty() {
                msg.insert("tool_calls".into(), Value::Array(tool_calls));
            }
            vec![Value::Object(msg)]
        }
    }
}

#[derive(Default)]
struct ToolAcc {
    id: String,
    name: String,
    arguments: String,
}

/// Decodes an OpenAI-compatible SSE stream into a completion.
pub(crate) fn decode(
    reader: impl Read,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(StreamEvent),
) -> Result<Completion> {
    let mut sse = SseReader::new(reader);
    let mut text = String::new();
    let mut reasoning_seen = false;
    let mut tools: Vec<ToolAcc> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason = StopReason::EndTurn;

    loop {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let Some(event) = sse
            .next_event()
            .map_err(|e| Error::Transport(e.to_string()))?
        else {
            break;
        };

        // This API signals completion with a sentinel rather than an event.
        if event.data.trim() == "[DONE]" {
            break;
        }

        let Ok(payload) = serde_json::from_str::<Value>(&event.data) else {
            continue;
        };

        if let Some(message) = payload.pointer("/error/message").and_then(Value::as_str) {
            return Err(Error::Protocol(message.to_string()));
        }

        if let Some(u) = payload.get("usage").filter(|u| !u.is_null()) {
            usage.input_tokens = u
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(usage.input_tokens);
            usage.output_tokens = u
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(usage.output_tokens);
        }

        let Some(choice) = payload.pointer("/choices/0") else {
            continue;
        };

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            stop_reason = StopReason::parse(reason);
        }

        let Some(delta) = choice.get("delta") else {
            continue;
        };

        if let Some(chunk) = delta.get("content").and_then(Value::as_str) {
            if !chunk.is_empty() {
                text.push_str(chunk);
                sink(StreamEvent::Text(chunk.to_string()));
            }
        }

        // Reasoning models on this API use one of two field names depending on
        // the server, so both are accepted.
        for field in ["reasoning_content", "reasoning"] {
            if let Some(chunk) = delta.get(field).and_then(Value::as_str) {
                if !chunk.is_empty() {
                    reasoning_seen = true;
                    sink(StreamEvent::Reasoning(chunk.to_string()));
                }
            }
        }

        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                // Chunks are keyed by index, and each field may arrive in any
                // chunk — only `arguments` is guaranteed to be split.
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if tools.len() <= index {
                    tools.resize_with(index + 1, ToolAcc::default);
                }
                let acc = &mut tools[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        acc.id = id.to_string();
                    }
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    if !name.is_empty() {
                        acc.name = name.to_string();
                    }
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    acc.arguments.push_str(args);
                }
            }
        }
    }

    let _ = reasoning_seen;

    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }

    let mut tool_calls = Vec::new();
    for (index, acc) in tools.into_iter().enumerate() {
        if acc.name.is_empty() {
            continue;
        }
        let arguments: Value = if acc.arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&acc.arguments).unwrap_or_else(|_| json!({}))
        };
        // Some servers omit the id entirely; synthesizing one keeps the
        // tool-result round trip well-formed.
        let id = if acc.id.is_empty() {
            format!("call_{index}")
        } else {
            acc.id
        };
        content.push(json!({
            "type": "tool_use",
            "id": id,
            "name": acc.name,
            "input": arguments,
        }));
        tool_calls.push(ToolCall {
            id,
            name: acc.name,
            arguments,
        });
    }

    // Some servers report `stop` even when they emitted tool calls.
    if !tool_calls.is_empty() {
        stop_reason = StopReason::ToolUse;
    }

    Ok(Completion {
        content: Value::Array(content),
        tool_calls,
        stop_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Effort;
    use crate::types::{Message, ToolResult, ToolSpec};

    fn request<'a>(messages: &'a [Message], tools: &'a [ToolSpec]) -> Request<'a> {
        Request {
            model: "local-model",
            system: "You help with a vault.",
            messages,
            tools,
            max_tokens: 2048,
            effort: Effort::High,
            show_reasoning: true,
        }
    }

    #[test]
    fn system_prompt_becomes_the_first_message() {
        let messages = [Message::user("hi")];
        let body = build_body(&request(&messages, &[]), "local-model");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "hi");
    }

    #[test]
    fn assistant_tool_calls_translate_to_function_calls() {
        let messages = [Message::assistant(json!([
            { "type": "thinking", "thinking": "hmm", "signature": "s" },
            { "type": "text", "text": "Creating it." },
            { "type": "tool_use", "id": "t1", "name": "create_note", "input": { "name": "Ideas" } }
        ]))];
        let body = build_body(&request(&messages, &[]), "local-model");
        let assistant = &body["messages"][1];

        assert_eq!(assistant["content"], "Creating it.");
        assert_eq!(assistant["tool_calls"][0]["id"], "t1");
        assert_eq!(
            assistant["tool_calls"][0]["function"]["name"],
            "create_note"
        );
        assert_eq!(
            assistant["tool_calls"][0]["function"]["arguments"], "{\"name\":\"Ideas\"}",
            "arguments travel as a JSON string on this API"
        );
    }

    #[test]
    fn tool_results_become_separate_tool_messages() {
        let messages = [Message::tool_results(&[
            ToolResult {
                id: "t1".into(),
                content: "created".into(),
                is_error: false,
            },
            ToolResult {
                id: "t2".into(),
                content: "also created".into(),
                is_error: false,
            },
        ])];
        let body = build_body(&request(&messages, &[]), "local-model");
        let msgs = body["messages"].as_array().unwrap();

        assert_eq!(msgs.len(), 3, "system plus one message per tool result");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["tool_call_id"], "t1");
        assert_eq!(msgs[2]["tool_call_id"], "t2");
    }

    #[test]
    fn tools_use_the_function_wrapper() {
        let messages = [Message::user("hi")];
        let tools = [ToolSpec::new(
            "search_notes",
            "Search",
            ToolSpec::object_schema(&[("query", "string", "text", true)]),
        )];
        let body = build_body(&request(&messages, &tools), "local-model");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "search_notes");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    fn decode_str(input: &str) -> Result<(Completion, Vec<StreamEvent>)> {
        let mut events = Vec::new();
        let completion = decode(input.as_bytes(), &|| false, &mut |e| events.push(e))?;
        Ok((completion, events))
    }

    #[test]
    fn decodes_streamed_text_and_usage() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let (completion, events) = decode_str(stream).expect("decode");

        assert_eq!(
            events,
            vec![
                StreamEvent::Text("Hel".into()),
                StreamEvent::Text("lo".into())
            ]
        );
        assert_eq!(completion.content[0]["text"], "Hello");
        assert_eq!(completion.stop_reason, StopReason::EndTurn);
        assert_eq!(completion.usage.input_tokens, 9);
        assert_eq!(completion.usage.output_tokens, 2);
    }

    #[test]
    fn decodes_tool_calls_split_across_chunks() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"create_note\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"name\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Ideas\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");

        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "call_1");
        assert_eq!(completion.tool_calls[0].name, "create_note");
        assert_eq!(completion.tool_calls[0].arguments["name"], "Ideas");
    }

    #[test]
    fn tool_calls_force_a_tool_use_stop_even_when_the_server_says_stop() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"list_tags\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn reasoning_fields_are_streamed_under_either_name() {
        for field in ["reasoning_content", "reasoning"] {
            let stream = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"{field}\":\"thinking...\"}}}}]}}\n\ndata: [DONE]\n\n"
            );
            let (_, events) = decode_str(&stream).expect("decode");
            assert_eq!(events, vec![StreamEvent::Reasoning("thinking...".into())]);
        }
    }

    #[test]
    fn missing_tool_call_id_is_synthesized() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"list_tags\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(completion.tool_calls[0].id, "call_0");
    }

    #[test]
    fn errors_in_the_stream_are_surfaced() {
        let stream = "data: {\"error\":{\"message\":\"model not found\"}}\n\n";
        let err = decode_str(stream).expect_err("should fail");
        assert!(err.to_string().contains("model not found"));
    }
}
