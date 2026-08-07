//! The agent loop.
//!
//! One turn is: send the conversation, stream the reply, run any tools the
//! model asked for, and repeat until it stops asking. That's the whole
//! contract — the same shape every tool-calling agent has — and keeping it
//! small is what makes it auditable.
//!
//! The loop runs on a background thread and communicates only through channels:
//! [`AgentEvent`]s out to the UI, tool requests out to whoever owns the vault.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::error::Error;
use crate::provider::{Effort, Provider, Request, StopReason, StreamEvent};
use crate::tool::{ToolHost, ToolOutcome};
use crate::types::{AgentEvent, Message, ToolResult, Usage};

/// Model and behavior settings for a session.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_tokens: u32,
    pub effort: Effort,
    /// Stream summarized reasoning into the panel.
    pub show_reasoning: bool,
    /// How many tool rounds a single turn may take before the loop stops.
    ///
    /// A model that keeps calling tools without concluding would otherwise
    /// spend the user's money indefinitely.
    pub max_tool_rounds: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: crate::provider::anthropic::DEFAULT_MODEL.to_string(),
            // Thinking counts against this budget on current models, so a
            // chat-sized answer still needs real headroom.
            max_tokens: 16_000,
            effort: Effort::High,
            show_reasoning: true,
            max_tool_rounds: 12,
        }
    }
}

/// Runs one turn to completion.
///
/// Appends every message it produces to `messages`, so the caller ends up with
/// a conversation that can be replayed verbatim on the next turn.
pub fn run_turn(
    provider: &dyn Provider,
    config: &AgentConfig,
    system: &str,
    messages: &mut Vec<Message>,
    tools: &mut dyn ToolHost,
    cancel: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(AgentEvent),
) {
    emit(AgentEvent::Started);

    let specs = tools.specs();
    let mut total = Usage::default();

    for round in 0..=config.max_tool_rounds {
        if cancel() {
            emit(AgentEvent::Failed("cancelled".into()));
            break;
        }

        let request = Request {
            model: &config.model,
            system,
            messages,
            tools: &specs,
            max_tokens: config.max_tokens,
            effort: config.effort,
            show_reasoning: config.show_reasoning,
        };

        let completion = match provider.stream(&request, cancel, &mut |event| match event {
            StreamEvent::Text(text) => emit(AgentEvent::Text(text)),
            StreamEvent::Reasoning(text) => emit(AgentEvent::Reasoning(text)),
        }) {
            Ok(completion) => completion,
            Err(Error::Cancelled) => {
                emit(AgentEvent::Failed("cancelled".into()));
                break;
            }
            Err(err) => {
                emit(AgentEvent::Failed(err.to_string()));
                break;
            }
        };

        total.add(completion.usage);
        emit(AgentEvent::Usage(total));

        // An assistant turn with no content at all can't be replayed — some
        // providers reject it — and there's nothing to preserve.
        let has_content = completion
            .content
            .as_array()
            .is_some_and(|blocks| !blocks.is_empty());
        if has_content {
            messages.push(Message::assistant(completion.content));
        }

        match completion.stop_reason {
            StopReason::Refusal(category) => {
                emit(AgentEvent::Failed(Error::Refused { category }.to_string()));
                break;
            }
            StopReason::MaxTokens => {
                emit(AgentEvent::TurnEnd);
                emit(AgentEvent::Failed(
                    "the reply hit the token limit; raise `max_tokens` in the agent config".into(),
                ));
                break;
            }
            StopReason::ToolUse if !completion.tool_calls.is_empty() => {
                if round == config.max_tool_rounds {
                    emit(AgentEvent::Failed(format!(
                        "stopped after {} rounds of tool calls",
                        config.max_tool_rounds
                    )));
                    break;
                }

                let mut results = Vec::with_capacity(completion.tool_calls.len());
                for call in &completion.tool_calls {
                    emit(AgentEvent::ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        summary: summarize_arguments(&call.arguments),
                    });

                    let outcome = if cancel() {
                        ToolOutcome::error("cancelled")
                    } else {
                        tools.call(call)
                    };

                    emit(AgentEvent::ToolResult {
                        id: call.id.clone(),
                        ok: !outcome.is_error,
                        summary: outcome.summary,
                    });
                    results.push(ToolResult {
                        id: call.id.clone(),
                        content: outcome.content,
                        is_error: outcome.is_error,
                    });
                }

                messages.push(Message::tool_results(&results));
                emit(AgentEvent::TurnEnd);
                continue;
            }
            _ => {
                emit(AgentEvent::TurnEnd);
                break;
            }
        }
    }

    emit(AgentEvent::Done);
}

/// A one-line rendering of tool arguments for the transcript.
fn summarize_arguments(arguments: &serde_json::Value) -> String {
    let Some(object) = arguments.as_object() else {
        return String::new();
    };
    let parts: Vec<String> = object
        .iter()
        .map(|(key, value)| {
            let text = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let text = text.replace('\n', " ");
            // Note bodies can be long; the transcript wants a hint, not a dump.
            let clipped: String = if text.chars().count() > 40 {
                text.chars().take(39).chain(std::iter::once('…')).collect()
            } else {
                text
            };
            format!("{key}={clipped}")
        })
        .collect();
    parts.join(", ")
}

/// A running turn on a background thread.
pub struct Runner {
    /// Streamed progress. Drain this each frame.
    pub events: Receiver<AgentEvent>,
    /// The conversation, readable once [`AgentEvent::Done`] arrives.
    pub history: Arc<Mutex<Vec<Message>>>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Runner {
    /// Asks the turn to stop at the next safe point.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Whether the worker thread has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Takes the conversation back once the turn is over.
    #[must_use]
    pub fn take_history(&self) -> Vec<Message> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // A dropped runner means the user closed the panel or quit; the worker
        // must not outlive it and keep spending tokens.
        self.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Starts a turn on a background thread.
///
/// The provider and tool host move onto the worker; the caller keeps the event
/// receiver and, for a [`ChannelToolHost`](crate::tool::ChannelToolHost), the
/// tool-request receiver.
pub fn spawn(
    provider: Box<dyn Provider>,
    config: AgentConfig,
    system: String,
    messages: Vec<Message>,
    mut tools: Box<dyn ToolHost>,
) -> Runner {
    let (event_tx, events): (Sender<AgentEvent>, Receiver<AgentEvent>) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let history = Arc::new(Mutex::new(messages));

    let worker_cancel = Arc::clone(&cancel);
    let worker_history = Arc::clone(&history);

    let handle = std::thread::Builder::new()
        // Linux caps a thread name at 15 bytes and refuses anything longer,
        // so this is deliberately shorter than the crate name.
        .name("emerald-agent".into())
        .spawn(move || {
            let mut conversation = worker_history.lock().map(|h| h.clone()).unwrap_or_default();

            run_turn(
                provider.as_ref(),
                &config,
                &system,
                &mut conversation,
                tools.as_mut(),
                &|| worker_cancel.load(Ordering::SeqCst),
                &mut |event| {
                    // A send failure means the UI is gone; stop the turn.
                    if event_tx.send(event).is_err() {
                        worker_cancel.store(true, Ordering::SeqCst);
                    }
                },
            );

            if let Ok(mut history) = worker_history.lock() {
                *history = conversation;
            }
        })
        .expect("spawn agent thread");

    Runner {
        events,
        history,
        cancel,
        handle: Some(handle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::mock::{Mock, Scripted};
    use crate::types::{ToolCall, ToolSpec};
    use serde_json::json;

    /// A tool host that records what it was asked to do.
    struct RecordingHost {
        specs: Vec<ToolSpec>,
        calls: Vec<ToolCall>,
        outcome: ToolOutcome,
    }

    impl RecordingHost {
        fn new(outcome: ToolOutcome) -> Self {
            Self {
                specs: vec![ToolSpec::new(
                    "list_tags",
                    "List tags",
                    json!({"type":"object"}),
                )],
                calls: Vec::new(),
                outcome,
            }
        }
    }

    impl ToolHost for RecordingHost {
        fn specs(&self) -> Vec<ToolSpec> {
            self.specs.clone()
        }
        fn call(&mut self, call: &ToolCall) -> ToolOutcome {
            self.calls.push(call.clone());
            self.outcome.clone()
        }
    }

    fn run(
        provider: &dyn Provider,
        tools: &mut dyn ToolHost,
        messages: &mut Vec<Message>,
    ) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        run_turn(
            provider,
            &AgentConfig::default(),
            "system",
            messages,
            tools,
            &|| false,
            &mut |e| events.push(e),
        );
        events
    }

    #[test]
    fn a_plain_reply_produces_text_then_done() {
        let provider = Mock::replying("Hello");
        let mut host = RecordingHost::new(ToolOutcome::ok("", ""));
        let mut messages = vec![Message::user("hi")];

        let events = run(&provider, &mut host, &mut messages);

        assert_eq!(events.first(), Some(&AgentEvent::Started));
        assert_eq!(events.last(), Some(&AgentEvent::Done));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Text(_))));
        assert!(host.calls.is_empty());

        assert_eq!(messages.len(), 2, "the assistant reply is appended");
        assert_eq!(messages[1].text(), "Hello");
    }

    #[test]
    fn tool_calls_run_and_the_loop_continues() {
        let provider = Mock::scripted(vec![
            Scripted::ToolCall {
                name: "list_tags".into(),
                arguments: json!({ "prefix": "project" }),
            },
            Scripted::Text("You have 3 tags.".into()),
        ]);
        let mut host = RecordingHost::new(ToolOutcome::ok("3 tags", "a, b, c"));
        let mut messages = vec![Message::user("what tags?")];

        let events = run(&provider, &mut host, &mut messages);

        assert_eq!(host.calls.len(), 1);
        assert_eq!(host.calls[0].name, "list_tags");

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCall { name, summary, .. }
                if name == "list_tags" && summary.contains("prefix=project")))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolResult { ok: true, .. }))
        );

        // user, assistant(tool_use), user(tool_result), assistant(text)
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[3].text(), "You have 3 tags.");
    }

    #[test]
    fn tool_errors_go_back_to_the_model_rather_than_ending_the_turn() {
        let provider = Mock::scripted(vec![
            Scripted::ToolCall {
                name: "list_tags".into(),
                arguments: json!({}),
            },
            Scripted::Text("I'll try another way.".into()),
        ]);
        let mut host = RecordingHost::new(ToolOutcome::error("no such note"));
        let mut messages = vec![Message::user("go")];

        let events = run(&provider, &mut host, &mut messages);

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolResult { ok: false, .. }))
        );
        assert_eq!(events.last(), Some(&AgentEvent::Done));

        let results = messages[2].content.as_array().unwrap();
        assert_eq!(results[0]["is_error"], true);
        assert_eq!(messages[3].text(), "I'll try another way.");
    }

    #[test]
    fn runaway_tool_loops_are_capped() {
        // A provider that only ever asks for tools would loop forever.
        let script = (0..50)
            .map(|_| Scripted::ToolCall {
                name: "list_tags".into(),
                arguments: json!({}),
            })
            .collect();
        let provider = Mock::scripted(script);
        let mut host = RecordingHost::new(ToolOutcome::ok("ok", "ok"));
        let mut messages = vec![Message::user("go")];

        let config = AgentConfig {
            max_tool_rounds: 3,
            ..Default::default()
        };
        let mut events = Vec::new();
        run_turn(
            &provider,
            &config,
            "system",
            &mut messages,
            &mut host,
            &|| false,
            &mut |e| events.push(e),
        );

        assert_eq!(host.calls.len(), 3, "capped at max_tool_rounds");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Failed(m) if m.contains("3 rounds")))
        );
        assert_eq!(events.last(), Some(&AgentEvent::Done));
    }

    #[test]
    fn provider_errors_are_reported_and_end_the_turn() {
        let provider = Mock::scripted(vec![Scripted::Error("rate limited".into())]);
        let mut host = RecordingHost::new(ToolOutcome::ok("", ""));
        let mut messages = vec![Message::user("hi")];

        let events = run(&provider, &mut host, &mut messages);

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Failed(m) if m.contains("rate limited")))
        );
        assert_eq!(events.last(), Some(&AgentEvent::Done));
    }

    #[test]
    fn cancellation_ends_the_turn_promptly() {
        let provider = Mock::replying("never seen");
        let mut host = RecordingHost::new(ToolOutcome::ok("", ""));
        let mut messages = vec![Message::user("hi")];

        let mut events = Vec::new();
        run_turn(
            &provider,
            &AgentConfig::default(),
            "system",
            &mut messages,
            &mut host,
            &|| true,
            &mut |e| events.push(e),
        );

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Failed(m) if m == "cancelled"))
        );
        assert_eq!(messages.len(), 1, "nothing was appended");
    }

    #[test]
    fn usage_accumulates_across_tool_rounds() {
        let provider = Mock::scripted(vec![
            Scripted::ToolCall {
                name: "list_tags".into(),
                arguments: json!({}),
            },
            Scripted::Text("done now".into()),
        ]);
        let mut host = RecordingHost::new(ToolOutcome::ok("ok", "ok"));
        let mut messages = vec![Message::user("go")];

        let events = run(&provider, &mut host, &mut messages);
        let last_usage = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Usage(u) => Some(*u),
                _ => None,
            })
            .next_back()
            .expect("usage reported");
        assert!(last_usage.output_tokens > 0);
    }

    #[test]
    fn spawn_runs_a_turn_and_returns_history() {
        let runner = spawn(
            Box::new(Mock::replying("threaded hello")),
            AgentConfig::default(),
            "system".into(),
            vec![Message::user("hi")],
            Box::new(RecordingHost::new(ToolOutcome::ok("", ""))),
        );

        let mut saw_done = false;
        while let Ok(event) = runner.events.recv() {
            if event == AgentEvent::Done {
                saw_done = true;
                break;
            }
        }
        assert!(saw_done);

        // Wait for the worker to publish the final history.
        while !runner.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let history = runner.take_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].text(), "threaded hello");
    }

    #[test]
    fn argument_summaries_are_clipped() {
        let long = "x".repeat(200);
        let summary = summarize_arguments(&json!({ "body": long }));
        assert!(summary.chars().count() < 60, "got {summary:?}");
        assert!(summary.ends_with('…'));
    }
}
