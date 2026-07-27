//! Server-Sent Events parsing.
//!
//! Both Anthropic's Messages API and every OpenAI-compatible server stream with
//! SSE, so one small parser covers all providers. It follows the WHATWG rules
//! that matter here: `field: value` lines, an optional single leading space
//! after the colon, `data:` accumulating across lines, comment lines starting
//! with `:`, and a blank line dispatching the event.

use std::io::{BufRead, BufReader, Read};

/// One dispatched SSE event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// The `event:` field. Empty when the stream doesn't name events, as
    /// OpenAI-compatible servers don't.
    pub event: String,
    /// The accumulated `data:` payload.
    pub data: String,
}

/// Streams SSE events from a reader.
pub struct SseReader<R: Read> {
    lines: BufReader<R>,
    buffer: String,
}

impl<R: Read> SseReader<R> {
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self {
            lines: BufReader::new(reader),
            buffer: String::new(),
        }
    }

    /// Reads the next event, or `None` at end of stream.
    ///
    /// Blocks until an event is complete, which is what makes the caller's
    /// cancellation check land between events rather than mid-parse.
    pub fn next_event(&mut self) -> std::io::Result<Option<SseEvent>> {
        let mut event = SseEvent::default();
        let mut saw_field = false;

        loop {
            self.buffer.clear();
            let read = self.lines.read_line(&mut self.buffer)?;
            if read == 0 {
                // End of stream: dispatch anything already accumulated so a
                // final event without a trailing blank line isn't lost.
                return Ok(if saw_field { Some(event) } else { None });
            }

            let line = self.buffer.trim_end_matches(['\n', '\r']);

            if line.is_empty() {
                if saw_field {
                    return Ok(Some(event));
                }
                // Blank lines between events (and keep-alive newlines) are skipped.
                continue;
            }

            // A line beginning with `:` is a comment, used for keep-alives.
            if line.starts_with(':') {
                continue;
            }

            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };

            match field {
                "event" => {
                    event.event = value.to_string();
                    saw_field = true;
                }
                "data" => {
                    if !event.data.is_empty() {
                        event.data.push('\n');
                    }
                    event.data.push_str(value);
                    saw_field = true;
                }
                // `id` and `retry` carry no meaning for a one-shot completion.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &str) -> Vec<SseEvent> {
        let mut reader = SseReader::new(input.as_bytes());
        let mut events = Vec::new();
        while let Some(event) = reader.next_event().expect("read") {
            events.push(event);
        }
        events
    }

    #[test]
    fn parses_named_events() {
        let events = collect("event: message_start\ndata: {\"a\":1}\n\nevent: done\ndata: {}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "message_start");
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[1].event, "done");
    }

    #[test]
    fn parses_unnamed_data_only_events() {
        let events = collect("data: {\"x\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(events.len(), 2);
        assert!(events[0].event.is_empty());
        assert_eq!(events[1].data, "[DONE]");
    }

    #[test]
    fn concatenates_multiline_data() {
        let events = collect("data: line one\ndata: line two\n\n");
        assert_eq!(events[0].data, "line one\nline two");
    }

    #[test]
    fn skips_comments_and_blank_padding() {
        let events = collect(": keep-alive\n\n\ndata: real\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "real");
    }

    #[test]
    fn handles_crlf_and_absent_space_after_colon() {
        let events = collect("event:ping\r\ndata:{\"v\":2}\r\n\r\n");
        assert_eq!(events[0].event, "ping");
        assert_eq!(events[0].data, "{\"v\":2}");
    }

    #[test]
    fn dispatches_a_trailing_event_without_a_blank_line() {
        let events = collect("data: last");
        assert_eq!(events.len(), 1, "a truncated stream still yields its event");
        assert_eq!(events[0].data, "last");
    }

    #[test]
    fn empty_stream_yields_nothing() {
        assert!(collect("").is_empty());
    }
}
