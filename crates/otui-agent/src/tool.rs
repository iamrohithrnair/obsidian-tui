//! Tool execution.
//!
//! Tools run on the **UI thread**, not the agent thread. The agent runs in the
//! background so a slow model never blocks rendering, but its tools operate on
//! the live vault index and can open tabs and move the graph — state that has
//! exactly one owner. Rather than wrapping that state in locks, the agent sends
//! a request down a channel and waits for the UI to run the tool and reply.
//!
//! The result is that the agent and the user act on the same state, in a
//! defined order, with no shared mutability at all.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::types::{ToolCall, ToolSpec};

/// What a tool produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Text returned to the model.
    pub content: String,
    /// Errors go back to the model rather than aborting the turn, so it can
    /// correct itself — a wrong note name should cost one retry, not the turn.
    pub is_error: bool,
    /// A one-line description for the chat transcript, e.g. `created Ideas.md`.
    pub summary: String,
}

impl ToolOutcome {
    #[must_use]
    pub fn ok(summary: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            summary: summary.into(),
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            summary: message.clone(),
            content: message,
            is_error: true,
        }
    }
}

/// Something that can run the agent's tools.
pub trait ToolHost: Send {
    fn specs(&self) -> Vec<ToolSpec>;
    fn call(&mut self, call: &ToolCall) -> ToolOutcome;
}

/// A tool call awaiting execution on the UI thread.
pub struct ToolRequest {
    pub call: ToolCall,
    reply: Sender<ToolOutcome>,
}

impl ToolRequest {
    /// Answers the waiting agent thread.
    ///
    /// A dropped reply channel means the agent gave up (cancelled or timed
    /// out), which isn't an error worth reporting.
    pub fn respond(self, outcome: ToolOutcome) {
        let _ = self.reply.send(outcome);
    }
}

/// A [`ToolHost`] that forwards calls to another thread.
pub struct ChannelToolHost {
    specs: Vec<ToolSpec>,
    requests: Sender<ToolRequest>,
    timeout: Duration,
}

/// How long the agent waits for the UI to run a tool.
///
/// Generous, because a tool may legitimately prompt the user; but bounded, so a
/// UI that stops servicing requests doesn't leave the agent thread parked
/// forever.
const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(120);

impl ChannelToolHost {
    #[must_use]
    pub fn new(specs: Vec<ToolSpec>, requests: Sender<ToolRequest>) -> Self {
        Self {
            specs,
            requests,
            timeout: DEFAULT_TOOL_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl ToolHost for ChannelToolHost {
    fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    fn call(&mut self, call: &ToolCall) -> ToolOutcome {
        let (reply, response) = std::sync::mpsc::channel();
        let request = ToolRequest {
            call: call.clone(),
            reply,
        };

        if self.requests.send(request).is_err() {
            return ToolOutcome::error("the application stopped accepting tool calls");
        }

        match response.recv_timeout(self.timeout) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                ToolOutcome::error(format!("tool `{}` timed out", call.name))
            }
            Err(RecvTimeoutError::Disconnected) => {
                ToolOutcome::error(format!("tool `{}` was abandoned", call.name))
            }
        }
    }
}

/// Receiving end of a [`ChannelToolHost`], held by the UI thread.
pub type ToolRequests = Receiver<ToolRequest>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: json!({}),
        }
    }

    #[test]
    fn forwards_calls_and_returns_the_reply() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut host = ChannelToolHost::new(Vec::new(), tx);

        // Stand in for the UI thread.
        let worker = std::thread::spawn(move || {
            let request = rx.recv().expect("request");
            assert_eq!(request.call.name, "list_tags");
            request.respond(ToolOutcome::ok("3 tags", "a, b, c"));
        });

        let outcome = host.call(&call("list_tags"));
        worker.join().expect("worker");

        assert!(!outcome.is_error);
        assert_eq!(outcome.content, "a, b, c");
        assert_eq!(outcome.summary, "3 tags");
    }

    #[test]
    fn a_gone_ui_yields_an_error_not_a_hang() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let mut host = ChannelToolHost::new(Vec::new(), tx);

        let outcome = host.call(&call("anything"));
        assert!(outcome.is_error);
    }

    #[test]
    fn an_unanswered_call_times_out() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut host = ChannelToolHost::new(Vec::new(), tx).with_timeout(Duration::from_millis(20));

        // Hold the receiver open but never respond.
        let outcome = host.call(&call("slow"));
        drop(rx);

        assert!(outcome.is_error);
        assert!(outcome.content.contains("timed out"));
    }

    #[test]
    fn error_outcomes_carry_the_message_both_ways() {
        let outcome = ToolOutcome::error("no such note");
        assert!(outcome.is_error);
        assert_eq!(outcome.content, "no such note");
        assert_eq!(outcome.summary, "no such note");
    }
}
