//! The agent chat panel's state and its wiring to the agent runtime.
//!
//! The panel keeps two parallel records: a **transcript** for the user, which
//! includes tool calls and errors, and a **conversation** for the model, which
//! is the exact message list replayed on the next turn. They diverge on purpose
//! — the user wants to see that a note was created, the model needs the tool
//! result blocks that say so.

use std::sync::mpsc::TryRecvError;

use otui_agent::{AgentEvent, Message, Runner, ToolRequests, Usage};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::config::AgentConfig;

/// One entry in the visible transcript.
///
/// Serializable because `/save` persists the transcript alongside the model's
/// message list — restoring only the latter would resume a conversation the
/// user can no longer read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Entry {
    User(String),
    Assistant(String),
    /// Summarized model reasoning, shown dimmed.
    Reasoning(String),
    Tool {
        name: String,
        detail: String,
        status: ToolStatus,
    },
    Error(String),
    /// A note attached as context for the next message.
    Context(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Ok,
    Failed,
}

pub struct Chat {
    /// What the user sees.
    pub transcript: Vec<Entry>,
    /// What the model sees.
    pub conversation: Vec<Message>,
    pub input: String,
    /// Cursor position within `input`, in characters.
    pub cursor: usize,
    /// Which slash command the completion list has highlighted.
    ///
    /// Reset whenever the input changes, since the list it indexes into is
    /// recomputed from what has been typed.
    pub completion: usize,
    pub scroll: usize,
    /// True while a turn is in flight.
    pub busy: bool,
    pub usage: Usage,
    /// Set when the panel should stick to the bottom as output streams in.
    pub follow: bool,

    runner: Option<Runner>,
    tools: Option<ToolRequests>,
    /// Snapshot of the agent settings this session started with.
    pub settings: AgentConfig,
}

impl Chat {
    #[must_use]
    pub fn new(settings: &AgentConfig) -> Self {
        Self {
            transcript: Vec::new(),
            conversation: Vec::new(),
            input: String::new(),
            cursor: 0,
            completion: 0,
            scroll: 0,
            busy: false,
            usage: Usage::default(),
            follow: true,
            runner: None,
            tools: None,
            settings: settings.clone(),
        }
    }

    /// Clears the session, keeping settings.
    pub fn reset(&mut self) {
        self.transcript.clear();
        self.conversation.clear();
        self.usage = Usage::default();
        self.scroll = 0;
        self.runner = None;
        self.tools = None;
        self.busy = false;
    }

    pub fn insert_char(&mut self, ch: char) {
        let byte = char_to_byte(&self.input, self.cursor);
        self.input.insert(byte, ch);
        self.cursor += 1;
        self.completion = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = char_to_byte(&self.input, self.cursor - 1);
        self.input.remove(byte);
        self.cursor -= 1;
        self.completion = 0;
    }

    /// Puts a slash command in the input, ready for its argument.
    pub fn complete_with(&mut self, name: &str) {
        self.input = format!("/{name} ");
        self.cursor = self.input.chars().count();
        self.completion = 0;
    }

    /// Moves the completion highlight, wrapping at both ends.
    ///
    /// Wrapping because the list is short and reaching the end by holding a key
    /// is a normal way to look through it.
    pub fn move_completion(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let len = len as isize;
        self.completion = ((self.completion as isize + delta).rem_euclid(len)) as usize;
    }

    /// Empties the input, closing the completion list with it.
    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.completion = 0;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.input.chars().count();
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize) as usize;
    }

    /// Appends streaming text to the last assistant entry, starting one if the
    /// previous entry was something else.
    fn push_text(&mut self, text: &str) {
        match self.transcript.last_mut() {
            Some(Entry::Assistant(existing)) => existing.push_str(text),
            _ => self.transcript.push(Entry::Assistant(text.to_string())),
        }
    }

    fn push_reasoning(&mut self, text: &str) {
        match self.transcript.last_mut() {
            Some(Entry::Reasoning(existing)) => existing.push_str(text),
            _ => self.transcript.push(Entry::Reasoning(text.to_string())),
        }
    }

    fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Started => self.busy = true,
            AgentEvent::Text(text) => self.push_text(&text),
            AgentEvent::Reasoning(text) => self.push_reasoning(&text),
            AgentEvent::ToolCall { name, summary, .. } => self.transcript.push(Entry::Tool {
                name,
                detail: summary,
                status: ToolStatus::Running,
            }),
            AgentEvent::ToolResult { ok, summary, .. } => {
                // Update the most recent running entry for this tool.
                if let Some(Entry::Tool { status, detail, .. }) =
                    self.transcript.iter_mut().rev().find(|e| {
                        matches!(
                            e,
                            Entry::Tool {
                                status: ToolStatus::Running,
                                ..
                            }
                        )
                    })
                {
                    *status = if ok {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Failed
                    };
                    if !summary.is_empty() {
                        *detail = summary;
                    }
                }
            }
            AgentEvent::Usage(usage) => self.usage = usage,
            AgentEvent::Failed(message) => self.transcript.push(Entry::Error(message)),
            AgentEvent::TurnEnd => {}
            AgentEvent::Done => self.busy = false,
        }
    }

    /// Asks the running turn to stop.
    pub fn cancel(&mut self) {
        if let Some(runner) = self.runner.as_ref() {
            runner.cancel();
        }
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.runner.is_some()
    }
}

fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Starts a turn with the text currently in the input box.
///
/// A free function rather than a method because sending needs the vault (for
/// the system prompt and the note context) while the chat lives inside the app.
pub fn send(app: &mut App) {
    let text = app.chat.input.trim().to_string();
    if text.is_empty() || app.chat.busy {
        return;
    }
    app.chat.input.clear();
    app.chat.cursor = 0;
    app.chat.follow = true;
    app.chat.transcript.push(Entry::User(text.clone()));

    // The open note travels with the message so "summarize this" works without
    // the user naming the note.
    let mut prompt = text;
    if app.config.agent.include_active_note {
        if let Some(id) = app.active_note() {
            if let (Some(note), Ok(body)) = (app.index.note(id), app.index.read_body(id)) {
                let rel = note.meta.rel.clone();
                let clipped: String = body.chars().take(8_000).collect();
                app.chat
                    .transcript
                    .push(Entry::Context(format!("attached: {rel}")));
                prompt =
                    format!("{prompt}\n\n<active_note path=\"{rel}\">\n{clipped}\n</active_note>");
            }
        }
    }

    let mut conversation = std::mem::take(&mut app.chat.conversation);
    conversation.push(Message::user(prompt));

    let (tool_tx, tool_rx) = std::sync::mpsc::channel();
    let specs = crate::tools::specs(app.config.agent.allow_writes);
    let host = otui_agent::ChannelToolHost::new(specs, tool_tx);

    let provider = otui_agent::build_provider_with_key(
        &app.config.agent.provider_kind(),
        app.config.agent.base_url.as_deref(),
        Some(&app.config.agent.model()),
        crate::auth::key_for(&app.config.agent.provider, &app.auth),
    );

    let runner = otui_agent::spawn(
        provider,
        app.config.agent.to_session_config(),
        system_prompt(app),
        conversation,
        Box::new(host),
    );

    app.chat.runner = Some(runner);
    app.chat.tools = Some(tool_rx);
    app.chat.busy = true;
}

/// Drains agent events and services tool calls.
///
/// Called once per frame. Both channels are taken out of the app first, because
/// running a tool needs `&mut App` and the receivers live inside it.
pub fn poll(app: &mut App) -> bool {
    let Some(runner) = app.chat.runner.take() else {
        return false;
    };
    let tools = app.chat.tools.take();
    let mut changed = false;

    // Tools first: the agent thread is blocked waiting on them, so servicing
    // them promptly is what keeps a turn moving.
    if let Some(tools) = tools.as_ref() {
        while let Ok(request) = tools.try_recv() {
            let outcome = crate::tools::execute(app, &request.call);
            request.respond(outcome);
            changed = true;
        }
    }

    let mut finished = false;
    loop {
        match runner.events.try_recv() {
            Ok(event) => {
                finished |= event == AgentEvent::Done;
                app.chat.apply(event);
                changed = true;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                finished = true;
                break;
            }
        }
    }

    if finished {
        app.chat.conversation = runner.take_history();
        app.chat.busy = false;
        // Dropping the runner joins its thread, which has already exited.
        drop(runner);
    } else {
        app.chat.runner = Some(runner);
        app.chat.tools = tools;
    }

    changed
}

/// Builds the system prompt.
///
/// It states what the vault is, what the agent may do, and — importantly — when
/// to reach for tools. Current models are conservative about tool use, so
/// naming the trigger conditions is what makes the agent actually search the
/// vault instead of guessing from the conversation.
#[must_use]
pub fn system_prompt(app: &App) -> String {
    let stats = app.index.stats();
    let vault = &app.index.vault.name;
    let writes = if app.config.agent.allow_writes {
        "You may create, edit, rename and delete notes."
    } else {
        "This vault is read-only: you may search and read, but not modify. Say so if asked to change something."
    };

    let tags: Vec<&str> = app
        .index
        .tags()
        .keys()
        .take(30)
        .map(String::as_str)
        .collect();

    format!(
        "You are the assistant built into obsidian-tui, working inside the user's \
Obsidian vault \"{vault}\" ({notes} notes, {tags_count} tags, {links} links between notes, \
{unresolved} links pointing at notes that don't exist yet).

{writes}

Use your tools rather than guessing. In particular:
- Call `search_notes` or `find_notes` whenever the answer depends on what is \
actually in the vault. Never answer from memory about the user's notes.
- Call `read_note` before summarizing, editing or quoting a note.
- Call `get_links` when asked how notes relate, what links somewhere, or what \
is orphaned.
- Call `open_note` when the user would want to see a note on screen, and \
`show_graph` when the answer is about structure.
- When you create or edit notes, connect them: a note that nothing links to is \
invisible in the graph. Use `link_notes`, or write `[[Wikilinks]]` directly in \
the content.

Write notes in Obsidian-flavored Markdown: `[[wikilinks]]`, `#tags`, `- [ ]` \
tasks, and `> [!note]` callouts. Keep replies short — this is a side panel a few \
dozen columns wide. Report what you did in one line; the user can see the notes \
themselves.

Existing tags: {tag_list}",
        vault = vault,
        notes = stats.notes,
        tags_count = stats.tags,
        links = stats.links,
        unresolved = stats.unresolved,
        writes = writes,
        tag_list = if tags.is_empty() {
            "(none yet)".to_string()
        } else {
            tags.join(", ")
        },
    )
}

/// Whether the assistant can actually reach a model.
///
/// Takes stored keys into account, not just exported ones, so a key typed in with
/// `/key` counts as being set up.
#[must_use]
pub fn ready(app: &App) -> bool {
    let provider = &app.config.agent.provider;
    let key = crate::auth::key_for(provider, &app.auth);
    otui_agent::has_credentials(
        &app.config.agent.provider_kind(),
        app.config.agent.base_url.as_deref(),
        key.as_deref(),
    )
}

/// A model list being fetched from a provider.
///
/// Asking costs a network round trip, and a local server that isn't running costs
/// the whole timeout, so it happens on a thread and the reader stays usable while
/// it does. One at a time: the answer feeds a menu that is about to open, and a
/// second request would only race the first to fill it.
#[derive(Default)]
pub struct Lookup {
    pending: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// Which provider was asked, so the picker can say so.
    pub provider: String,
}

impl Lookup {
    /// Whether an answer is still outstanding.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Asks a provider for its models.
    ///
    /// Replaces any request already in flight; the old one's answer is dropped
    /// when its channel closes.
    pub fn start(&mut self, provider: &str, api_key: Option<String>, base_url: Option<String>) {
        let Some(preset) = otui_agent::catalog::find(provider) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        self.provider = provider.to_string();

        std::thread::Builder::new()
            .name("otui-models".into())
            .spawn(move || {
                let result =
                    otui_agent::catalog::models(preset, api_key.as_deref(), base_url.as_deref())
                        .map_err(|err| err.to_string());
                // Nobody left to tell means the app moved on, which is fine.
                let _ = tx.send(result);
            })
            .ok();
    }

    /// The answer, once it arrives.
    pub fn take(&mut self) -> Option<Result<Vec<String>, String>> {
        let pending = self.pending.as_ref()?;
        match pending.try_recv() {
            Ok(result) => {
                self.pending = None;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            // The thread died without answering. Reporting that is better than
            // waiting for a reply that will never come.
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                Some(Err("the request went away".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("chat");
        vault.write("A.md", "# A\n\n#topic\nlinks [[B]] and [[Ghost]]\n");
        vault.write("B.md", "# B\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");
        (vault, app)
    }

    #[test]
    fn input_editing_is_character_aware() {
        let mut chat = Chat::new(&AgentConfig::default());
        for ch in "héllo".chars() {
            chat.insert_char(ch);
        }
        assert_eq!(chat.input, "héllo");
        assert_eq!(chat.cursor, 5);

        chat.move_cursor(-3);
        chat.insert_char('X');
        assert_eq!(chat.input, "héXllo");

        chat.backspace();
        assert_eq!(chat.input, "héllo");
    }

    #[test]
    fn cursor_movement_clamps_to_the_input() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.insert_char('a');
        chat.move_cursor(-10);
        assert_eq!(chat.cursor, 0);
        chat.move_cursor(10);
        assert_eq!(chat.cursor, 1);
    }

    #[test]
    fn streamed_text_accumulates_into_one_entry() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.apply(AgentEvent::Text("Hel".into()));
        chat.apply(AgentEvent::Text("lo".into()));

        assert_eq!(chat.transcript, vec![Entry::Assistant("Hello".into())]);
    }

    #[test]
    fn a_tool_call_updates_in_place_when_it_finishes() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.apply(AgentEvent::ToolCall {
            id: "t1".into(),
            name: "create_note".into(),
            summary: "name=Ideas".into(),
        });
        assert!(matches!(
            chat.transcript[0],
            Entry::Tool {
                status: ToolStatus::Running,
                ..
            }
        ));

        chat.apply(AgentEvent::ToolResult {
            id: "t1".into(),
            ok: true,
            summary: "created Ideas.md".into(),
        });

        assert_eq!(
            chat.transcript.len(),
            1,
            "the entry is updated, not appended"
        );
        assert!(matches!(
            &chat.transcript[0],
            Entry::Tool { status: ToolStatus::Ok, detail, .. } if detail == "created Ideas.md"
        ));
    }

    #[test]
    fn reasoning_and_text_stay_separate_entries() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.apply(AgentEvent::Reasoning("thinking".into()));
        chat.apply(AgentEvent::Text("answer".into()));
        chat.apply(AgentEvent::Reasoning("more".into()));

        assert_eq!(chat.transcript.len(), 3);
    }

    #[test]
    fn done_clears_the_busy_flag() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.apply(AgentEvent::Started);
        assert!(chat.busy);
        chat.apply(AgentEvent::Done);
        assert!(!chat.busy);
    }

    #[test]
    fn reset_clears_both_records() {
        let mut chat = Chat::new(&AgentConfig::default());
        chat.transcript.push(Entry::User("hi".into()));
        chat.conversation.push(Message::user("hi"));
        chat.reset();

        assert!(chat.transcript.is_empty());
        assert!(chat.conversation.is_empty());
    }

    #[test]
    fn the_system_prompt_describes_the_actual_vault() {
        let (_vault, app) = app();
        let prompt = system_prompt(&app);

        assert!(prompt.contains("2 notes"));
        assert!(prompt.contains("topic"), "existing tags are listed");
        assert!(prompt.contains("search_notes"), "tool triggers are stated");
        assert!(prompt.contains("create, edit, rename and delete"));
    }

    #[test]
    fn a_read_only_vault_says_so_in_the_prompt() {
        let (_vault, mut app) = app();
        app.config.agent.allow_writes = false;
        let prompt = system_prompt(&app);

        assert!(prompt.contains("read-only"));
        assert!(!prompt.contains("You may create, edit"));
    }

    #[test]
    fn sending_attaches_the_open_note_as_context() {
        let (_vault, mut app) = app();
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);

        app.chat.input = "summarize this".into();
        send(&mut app);

        // The transcript shows the attachment; the model message carries it.
        assert!(app
            .chat
            .transcript
            .iter()
            .any(|e| matches!(e, Entry::Context(c) if c.contains("A.md"))));

        app.chat.cancel();
    }

    #[test]
    fn sending_an_empty_message_does_nothing() {
        let (_vault, mut app) = app();
        app.chat.input = "   ".into();
        send(&mut app);

        assert!(app.chat.transcript.is_empty());
        assert!(!app.chat.is_running());
    }
}
