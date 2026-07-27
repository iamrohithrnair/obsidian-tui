//! The Anthropic Messages API.
//!
//! Requests follow the current API surface: adaptive thinking, `output_config`
//! effort, and **no** sampling parameters or `budget_tokens` — those are
//! rejected outright by current models. The system prompt carries a cache
//! breakpoint, which matters a lot for a chat panel: the vault instructions and
//! the tool list are identical on every turn, so caching them turns the fixed
//! prefix into a cheap cache read after the first message.

use std::io::Read;

use serde_json::{json, Map, Value};

use super::{post_stream, Completion, Provider, Request, StopReason, StreamEvent};
use crate::error::{Error, Result};
use crate::sse::SseReader;
use crate::types::{Role, ToolCall, Usage};

pub const ENV_VAR: &str = "ANTHROPIC_API_KEY";
pub const DEFAULT_MODEL: &str = "claude-opus-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const TIMEOUT_SECS: u64 = 600;

pub struct Anthropic {
    api_key: String,
    base_url: String,
}

impl Anthropic {
    /// Builds a connector from the environment.
    pub fn from_env(base_url: Option<&str>) -> Result<Self> {
        let api_key = super::env_key(ENV_VAR).ok_or(Error::MissingCredentials {
            provider: "anthropic",
            env_var: ENV_VAR,
        })?;
        Ok(Self {
            api_key,
            base_url: base_url.unwrap_or(API_URL).to_string(),
        })
    }
}

impl Provider for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    fn stream(
        &self,
        request: &Request<'_>,
        cancel: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion> {
        let body = build_body(request);
        let reader = post_stream(
            &self.base_url,
            &[
                ("x-api-key", self.api_key.as_str()),
                ("anthropic-version", API_VERSION),
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ],
            &body,
            TIMEOUT_SECS,
        )?;
        decode(reader, cancel, sink)
    }
}

/// Builds the request body.
pub(crate) fn build_body(request: &Request<'_>) -> Value {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": match m.role { Role::User => "user", Role::Assistant => "assistant" },
                "content": m.content,
            })
        })
        .collect();

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("max_tokens".into(), json!(request.max_tokens));
    body.insert("stream".into(), json!(true));
    body.insert("messages".into(), Value::Array(messages));

    if !request.system.is_empty() {
        // A cache breakpoint on the last system block covers tools + system,
        // which is the entire fixed prefix of every turn in a chat session.
        body.insert(
            "system".into(),
            json!([{
                "type": "text",
                "text": request.system,
                "cache_control": { "type": "ephemeral" },
            }]),
        );
    }

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }

    // Adaptive thinking is the only supported mode on current models; a fixed
    // token budget is rejected. `display` must be requested explicitly or the
    // reasoning arrives as empty strings.
    body.insert(
        "thinking".into(),
        if request.show_reasoning {
            json!({ "type": "adaptive", "display": "summarized" })
        } else {
            json!({ "type": "adaptive" })
        },
    );
    body.insert(
        "output_config".into(),
        json!({ "effort": request.effort.as_str() }),
    );

    Value::Object(body)
}

/// Content blocks are streamed by index, so they're accumulated separately and
/// assembled in order once the message ends.
enum BlockAcc {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    /// A block type this client doesn't model. Kept verbatim so replaying the
    /// turn doesn't corrupt it.
    Passthrough(Value),
}

/// Decodes an Anthropic SSE stream into a completion.
pub(crate) fn decode(
    reader: impl Read,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(StreamEvent),
) -> Result<Completion> {
    let mut sse = SseReader::new(reader);
    let mut blocks: Vec<Option<BlockAcc>> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason = StopReason::EndTurn;
    let mut refusal_category: Option<String> = None;

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

        let payload: Value = match serde_json::from_str(&event.data) {
            Ok(value) => value,
            // A malformed chunk shouldn't kill a turn that is otherwise fine.
            Err(_) => continue,
        };

        match event.event.as_str() {
            "message_start" => {
                if let Some(u) = payload.pointer("/message/usage") {
                    usage.input_tokens = field_u64(u, "input_tokens");
                    usage.cache_read_tokens = field_u64(u, "cache_read_input_tokens");
                }
            }
            "content_block_start" => {
                let index = field_u64(&payload, "index") as usize;
                let block = payload.get("content_block").cloned().unwrap_or(Value::Null);
                let acc = match block.get("type").and_then(Value::as_str) {
                    Some("text") => BlockAcc::Text(
                        block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    Some("thinking") => BlockAcc::Thinking {
                        text: block
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        signature: String::new(),
                    },
                    Some("tool_use") => BlockAcc::ToolUse {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        json: String::new(),
                    },
                    _ => BlockAcc::Passthrough(block),
                };
                set_at(&mut blocks, index, acc);
            }
            "content_block_delta" => {
                let index = field_u64(&payload, "index") as usize;
                let delta = payload.get("delta").cloned().unwrap_or(Value::Null);
                let kind = delta.get("type").and_then(Value::as_str).unwrap_or("");

                match (kind, blocks.get_mut(index).and_then(Option::as_mut)) {
                    ("text_delta", Some(BlockAcc::Text(buf))) => {
                        let chunk = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        buf.push_str(chunk);
                        sink(StreamEvent::Text(chunk.to_string()));
                    }
                    ("thinking_delta", Some(BlockAcc::Thinking { text, .. })) => {
                        let chunk = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                        text.push_str(chunk);
                        if !chunk.is_empty() {
                            sink(StreamEvent::Reasoning(chunk.to_string()));
                        }
                    }
                    ("signature_delta", Some(BlockAcc::Thinking { signature, .. })) => {
                        signature
                            .push_str(delta.get("signature").and_then(Value::as_str).unwrap_or(""));
                    }
                    ("input_json_delta", Some(BlockAcc::ToolUse { json, .. })) => {
                        json.push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        );
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = payload
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                {
                    stop_reason = StopReason::parse(reason);
                }
                if let Some(category) = payload
                    .pointer("/delta/stop_details/category")
                    .and_then(Value::as_str)
                {
                    refusal_category = Some(category.to_string());
                }
                if let Some(u) = payload.get("usage") {
                    usage.output_tokens = field_u64(u, "output_tokens");
                }
            }
            "error" => {
                let message = payload
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("stream error");
                return Err(Error::Protocol(message.to_string()));
            }
            "message_stop" => break,
            _ => {}
        }
    }

    if let StopReason::Refusal(_) = stop_reason {
        stop_reason = StopReason::Refusal(refusal_category);
    }

    let (content, tool_calls) = assemble(blocks);
    Ok(Completion {
        content,
        tool_calls,
        stop_reason,
        usage,
    })
}

fn assemble(blocks: Vec<Option<BlockAcc>>) -> (Value, Vec<ToolCall>) {
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();

    for block in blocks.into_iter().flatten() {
        match block {
            BlockAcc::Text(text) => {
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
            }
            BlockAcc::Thinking { text, signature } => {
                // Thinking blocks are replayed verbatim; the signature is what
                // makes them valid on the next request, so an empty-text block
                // is still worth keeping.
                let mut block = Map::new();
                block.insert("type".into(), json!("thinking"));
                block.insert("thinking".into(), json!(text));
                if !signature.is_empty() {
                    block.insert("signature".into(), json!(signature));
                }
                content.push(Value::Object(block));
            }
            BlockAcc::ToolUse { id, name, json } => {
                // An empty argument stream means a no-argument tool, which is
                // `{}` rather than a parse failure.
                let arguments = if json.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&json).unwrap_or_else(|_| json!({}))
                };
                content.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": arguments,
                }));
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            BlockAcc::Passthrough(value) => {
                if !value.is_null() {
                    content.push(value);
                }
            }
        }
    }

    (Value::Array(content), tool_calls)
}

fn set_at(blocks: &mut Vec<Option<BlockAcc>>, index: usize, value: BlockAcc) {
    if blocks.len() <= index {
        blocks.resize_with(index + 1, || None);
    }
    blocks[index] = Some(value);
}

fn field_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Anthropic model ids offered in the model picker.
pub const MODELS: &[&str] = &[
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-haiku-4-5",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Effort;
    use crate::types::{Message, ToolSpec};

    fn request<'a>(
        messages: &'a [Message],
        tools: &'a [ToolSpec],
        show_reasoning: bool,
    ) -> Request<'a> {
        Request {
            model: DEFAULT_MODEL,
            system: "You help with a vault.",
            messages,
            tools,
            max_tokens: 4096,
            effort: Effort::High,
            show_reasoning,
        }
    }

    #[test]
    fn body_omits_parameters_current_models_reject() {
        let messages = [Message::user("hi")];
        let body = build_body(&request(&messages, &[], true));

        for rejected in ["temperature", "top_p", "top_k"] {
            assert!(
                body.get(rejected).is_none(),
                "{rejected} is rejected with a 400 on current models"
            );
        }
        assert!(
            body.pointer("/thinking/budget_tokens").is_none(),
            "budget_tokens was removed in favor of effort"
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn reasoning_display_is_opt_in() {
        let messages = [Message::user("hi")];
        let shown = build_body(&request(&messages, &[], true));
        assert_eq!(shown["thinking"]["display"], "summarized");

        let hidden = build_body(&request(&messages, &[], false));
        assert!(hidden["thinking"].get("display").is_none());
    }

    #[test]
    fn system_prompt_carries_a_cache_breakpoint() {
        let messages = [Message::user("hi")];
        let body = build_body(&request(&messages, &[], true));
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn tools_are_sent_in_the_documented_shape() {
        let messages = [Message::user("hi")];
        let tools = [ToolSpec::new(
            "search_notes",
            "Search the vault",
            ToolSpec::object_schema(&[("query", "string", "text to find", true)]),
        )];
        let body = build_body(&request(&messages, &tools, true));

        assert_eq!(body["tools"][0]["name"], "search_notes");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    const TEXT_STREAM: &str = concat!(
        "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":11,\"cache_read_input_tokens\":4}}}\n\n",
        "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\n",
        "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
        "event: content_block_stop\ndata: {\"index\":0}\n\n",
        "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
        "event: message_stop\ndata: {}\n\n",
    );

    fn decode_str(input: &str) -> Result<(Completion, Vec<StreamEvent>)> {
        let mut events = Vec::new();
        let completion = decode(input.as_bytes(), &|| false, &mut |e| events.push(e))?;
        Ok((completion, events))
    }

    #[test]
    fn decodes_streamed_text_and_usage() {
        let (completion, events) = decode_str(TEXT_STREAM).expect("decode");

        assert_eq!(
            events,
            vec![
                StreamEvent::Text("Hello ".into()),
                StreamEvent::Text("world".into())
            ],
            "text must stream incrementally, not arrive at the end"
        );
        assert_eq!(completion.content[0]["text"], "Hello world");
        assert_eq!(completion.stop_reason, StopReason::EndTurn);
        assert_eq!(completion.usage.input_tokens, 11);
        assert_eq!(completion.usage.output_tokens, 7);
        assert_eq!(completion.usage.cache_read_tokens, 4);
    }

    #[test]
    fn decodes_tool_calls_with_streamed_arguments() {
        let stream = concat!(
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"create_note\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"name\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"Ideas\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");

        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].name, "create_note");
        assert_eq!(completion.tool_calls[0].arguments["name"], "Ideas");
        // The same call must also be in the replayable content.
        assert_eq!(completion.content[0]["type"], "tool_use");
        assert_eq!(completion.content[0]["input"]["name"], "Ideas");
    }

    #[test]
    fn tool_call_with_no_arguments_becomes_an_empty_object() {
        let stream = concat!(
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"list_tags\"}}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(completion.tool_calls[0].arguments, json!({}));
    }

    #[test]
    fn thinking_blocks_keep_their_signature_for_replay() {
        let stream = concat!(
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"weighing options\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-xyz\"}}\n\n",
            "event: content_block_start\ndata: {\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Done\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, events) = decode_str(stream).expect("decode");

        assert!(events.contains(&StreamEvent::Reasoning("weighing options".into())));
        assert_eq!(completion.content[0]["type"], "thinking");
        assert_eq!(
            completion.content[0]["signature"], "sig-xyz",
            "the signature must survive to be replayed"
        );
        assert_eq!(completion.content[1]["text"], "Done");
    }

    #[test]
    fn unknown_block_types_pass_through_untouched() {
        let stream = concat!(
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"opaque\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(completion.content[0]["type"], "redacted_thinking");
        assert_eq!(completion.content[0]["data"], "opaque");
    }

    #[test]
    fn refusal_is_reported_with_its_category() {
        let stream = concat!(
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"category\":\"cyber\"}}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(
            completion.stop_reason,
            StopReason::Refusal(Some("cyber".into()))
        );
    }

    #[test]
    fn stream_errors_become_protocol_errors() {
        let stream = "event: error\ndata: {\"error\":{\"message\":\"overloaded\"}}\n\n";
        let err = decode_str(stream).expect_err("should fail");
        assert!(err.to_string().contains("overloaded"));
    }

    #[test]
    fn malformed_chunks_are_skipped_not_fatal() {
        let stream = concat!(
            "event: content_block_start\ndata: not json at all\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let (completion, _) = decode_str(stream).expect("decode");
        assert_eq!(completion.content[0]["text"], "ok");
    }

    #[test]
    fn cancellation_stops_the_stream() {
        let mut events = Vec::new();
        let err = decode(TEXT_STREAM.as_bytes(), &|| true, &mut |e| events.push(e))
            .expect_err("should cancel");
        assert!(matches!(err, Error::Cancelled));
        assert!(events.is_empty(), "nothing should be emitted after cancel");
    }
}
