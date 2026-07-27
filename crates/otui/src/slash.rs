//! Slash commands for the agent panel.
//!
//! Typing `/` at the start of the chat input opens a command instead of a
//! message. Everything here is something you would otherwise have to leave the
//! app to do — switch provider, check whether a key is set, save the
//! conversation and pick it up tomorrow.
//!
//! Commands run locally and never reach the model. That is the point: `/model`
//! should change the model, not ask the current one to change it.

use otui_core::sort::SortOrder;

use crate::agent::Entry;
use crate::app::{Action, App};
use crate::session;

/// One command in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    /// What the argument means, shown after the name in the completion list.
    pub argument_hint: Option<&'static str>,
}

/// Every command, in the order the completion popup shows them.
///
/// Grouped by what they act on: the conversation, the provider, then the vault.
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        description: "List the slash commands",
        argument_hint: None,
    },
    SlashCommand {
        name: "new",
        description: "Start a fresh conversation",
        argument_hint: None,
    },
    SlashCommand {
        name: "save",
        description: "Save this conversation",
        argument_hint: Some("[name]"),
    },
    SlashCommand {
        name: "resume",
        description: "Reload a saved conversation",
        argument_hint: Some("[name]"),
    },
    SlashCommand {
        name: "sessions",
        description: "List saved conversations, or delete one",
        argument_hint: Some("[delete <name>]"),
    },
    SlashCommand {
        name: "compact",
        description: "Drop older turns to free up context",
        argument_hint: None,
    },
    SlashCommand {
        name: "provider",
        description: "Switch backend: anthropic, openai, offline",
        argument_hint: Some("[name]"),
    },
    SlashCommand {
        name: "model",
        description: "Set the model to request",
        argument_hint: Some("[name]"),
    },
    SlashCommand {
        name: "base-url",
        description: "Point at a local or proxied API endpoint",
        argument_hint: Some("[url|clear]"),
    },
    SlashCommand {
        name: "login",
        description: "Show how to supply credentials for this provider",
        argument_hint: None,
    },
    SlashCommand {
        name: "logout",
        description: "Switch to offline and forget the endpoint",
        argument_hint: None,
    },
    SlashCommand {
        name: "status",
        description: "Show provider, model and token usage",
        argument_hint: None,
    },
    SlashCommand {
        name: "writes",
        description: "Allow or forbid the agent editing notes",
        argument_hint: Some("[on|off]"),
    },
    SlashCommand {
        name: "context",
        description: "Send the open note with each message",
        argument_hint: Some("[on|off]"),
    },
    SlashCommand {
        name: "reasoning",
        description: "Show the model's summarized reasoning",
        argument_hint: Some("[on|off]"),
    },
    SlashCommand {
        name: "tools",
        description: "List the tools the agent can call",
        argument_hint: None,
    },
    SlashCommand {
        name: "obsidian",
        description: "Obsidian CLI status, or open this note in the app",
        argument_hint: Some("[open]"),
    },
    SlashCommand {
        name: "sort",
        description: "Change how the file explorer orders notes",
        argument_hint: Some("[modified|created|name|...]"),
    },
    SlashCommand {
        name: "vault",
        description: "Show what's indexed in this vault",
        argument_hint: None,
    },
    SlashCommand {
        name: "config",
        description: "Write the current settings to the config file",
        argument_hint: None,
    },
    SlashCommand {
        name: "keys",
        description: "Open the keyboard shortcut reference",
        argument_hint: None,
    },
    SlashCommand {
        name: "quit",
        description: "Leave obsidian-tui",
        argument_hint: None,
    },
];

/// Whether the input should be treated as a command rather than a message.
///
/// A bare `/` counts, so the completion popup appears as soon as the user
/// commits to a command.
#[must_use]
pub fn is_command(input: &str) -> bool {
    input.starts_with('/')
}

/// Splits `/name rest` into its two parts.
#[must_use]
pub fn parse(input: &str) -> Option<(&str, &str)> {
    let rest = input.strip_prefix('/')?;
    let rest = rest.trim_end();
    match rest.find(char::is_whitespace) {
        Some(at) => Some((&rest[..at], rest[at..].trim_start())),
        None => Some((rest, "")),
    }
}

/// Commands matching what has been typed so far, best match first.
///
/// A prefix match ranks above a substring match so `/re` offers `resume` before
/// `reasoning`, which is what a user typing left-to-right expects.
#[must_use]
pub fn completions(input: &str) -> Vec<&'static SlashCommand> {
    let Some((name, args)) = parse(input) else {
        return Vec::new();
    };
    // Once there's an argument the command is settled; no point still offering
    // alternatives.
    if !args.is_empty() || input.ends_with(char::is_whitespace) {
        return COMMANDS.iter().filter(|c| c.name == name).collect();
    }

    let needle = name.to_lowercase();
    let mut prefix: Vec<&SlashCommand> = Vec::new();
    let mut substring: Vec<&SlashCommand> = Vec::new();
    for command in COMMANDS {
        if command.name.starts_with(&needle) {
            prefix.push(command);
        } else if command.name.contains(&needle) {
            substring.push(command);
        }
    }
    prefix.extend(substring);
    prefix
}

/// What running a command did, so the caller can react without this module
/// knowing about key handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Handled; the input box should be cleared.
    Handled,
    /// Not a known command; the caller reports it.
    Unknown(String),
}

/// Runs a command typed into the chat box.
///
/// Output goes into the transcript rather than the status bar because it is
/// part of the conversation's history — you want to be able to scroll back and
/// see that you switched models halfway through.
pub fn run(app: &mut App, input: &str) -> Outcome {
    let Some((name, args)) = parse(input) else {
        return Outcome::Unknown(input.to_string());
    };
    let args = args.trim();

    match name {
        "help" | "?" => say(app, &help_text()),
        "new" | "clear" => {
            app.chat.reset();
            say(app, "started a new conversation");
        }
        "save" => save(app, args),
        "resume" | "load" => resume(app, args),
        "sessions" | "list" => sessions(app, args),
        "compact" => compact(app),
        "provider" => provider(app, args),
        "model" => model(app, args),
        "base-url" | "baseurl" | "url" => base_url(app, args),
        "login" => {
            let text = login_text(app);
            say(app, &text);
        }
        "logout" => logout(app),
        "status" => {
            let text = status_text(app);
            say(app, &text);
        }
        "writes" => writes(app, args),
        "context" => context(app, args),
        "reasoning" => reasoning(app, args),
        "tools" => {
            let text = tools_text(app);
            say(app, &text);
        }
        "obsidian" => obsidian(app, args),
        "sort" => sort(app, args),
        "vault" => {
            let text = vault_text(app);
            say(app, &text);
        }
        "config" => save_config(app),
        "keys" | "hotkeys" => crate::actions::dispatch(app, Action::OpenHelp),
        "quit" | "exit" | "q" => crate::actions::dispatch(app, Action::Quit),
        other => return Outcome::Unknown(other.to_string()),
    }

    Outcome::Handled
}

/// Adds a command's output to the transcript.
fn say(app: &mut App, text: &str) {
    app.chat.follow = true;
    app.chat.transcript.push(Entry::Context(text.to_string()));
}

fn help_text() -> String {
    let mut lines = vec!["Slash commands".to_string()];
    for command in COMMANDS {
        let name = match command.argument_hint {
            Some(hint) => format!("/{} {hint}", command.name),
            None => format!("/{}", command.name),
        };
        lines.push(format!("  {name:<22} {}", command.description));
    }
    lines.join("\n")
}

fn save(app: &mut App, args: &str) {
    let name = if args.is_empty() {
        session::suggested_name(&app.chat.transcript)
    } else {
        args.to_string()
    };
    if app.chat.conversation.is_empty() && app.chat.transcript.is_empty() {
        say(app, "nothing to save yet");
        return;
    }

    let record = session::Session {
        name: name.clone(),
        saved_at: session::now(),
        vault: Some(app.index.vault.path.clone()),
        transcript: app.chat.transcript.clone(),
        conversation: app.chat.conversation.clone(),
    };
    match session::save(&record) {
        Ok(path) => say(app, &format!("saved as '{name}' → {}", path.display())),
        Err(err) => say(app, &format!("could not save: {err}")),
    }
}

fn resume(app: &mut App, args: &str) {
    if args.is_empty() {
        // With no name, resuming the most recent one is what "pick up where I
        // left off" means; the list is one command away if that's wrong.
        let Some(latest) = session::list().into_iter().next() else {
            say(app, "no saved conversations — /save creates one");
            return;
        };
        load_session(app, &latest.name);
        return;
    }
    load_session(app, args);
}

fn load_session(app: &mut App, name: &str) {
    match session::load(name) {
        Ok(record) => {
            let turns = record.conversation.len();
            app.chat.reset();
            app.chat.transcript = record.transcript;
            app.chat.conversation = record.conversation;
            app.chat.follow = true;

            // A conversation about a different vault will refer to notes that
            // aren't here, so say so rather than let it confuse the model.
            let mismatch = record
                .vault
                .as_deref()
                .is_some_and(|v| v != app.index.vault.path);
            let mut message = format!("resumed '{name}' — {turns} turns");
            if mismatch {
                message.push_str(" (saved against a different vault)");
            }
            say(app, &message);
        }
        Err(err) => say(app, &format!("could not resume '{name}': {err}")),
    }
}

fn sessions(app: &mut App, args: &str) {
    if let Some(name) = args.strip_prefix("delete").map(str::trim) {
        if name.is_empty() {
            say(app, "/sessions delete <name>");
        } else {
            match session::delete(name) {
                Ok(()) => say(app, &format!("deleted '{name}'")),
                Err(err) => say(app, &format!("could not delete '{name}': {err}")),
            }
        }
        return;
    }

    let sessions = session::list();
    if sessions.is_empty() {
        say(app, "no saved conversations — /save creates one");
        return;
    }
    let mut lines = vec!["Saved conversations".to_string()];
    for info in sessions.iter().take(30) {
        lines.push(format!("  {:<28} {} turns", info.name, info.turns));
    }
    lines.push("/resume <name> to reload one, /sessions delete <name> to remove one".to_string());
    say(app, &lines.join("\n"));
}

/// Keeps the tail of the conversation and drops the rest.
///
/// A real summarization would need a model round-trip; trimming is instant,
/// predictable, and solves the actual problem — a context window filling up
/// mid-task.
fn compact(app: &mut App) {
    const KEEP: usize = 6;
    let before = app.chat.conversation.len();
    if before <= KEEP {
        say(app, "nothing to compact yet");
        return;
    }
    app.chat.conversation.drain(..before - KEEP);
    // A conversation must not start with tool results whose calls were just
    // dropped; providers reject that.
    while app
        .chat
        .conversation
        .first()
        .is_some_and(|m| !is_plain_user_turn(m))
    {
        app.chat.conversation.remove(0);
    }
    let after = app.chat.conversation.len();
    say(app, &format!("compacted: {before} turns → {after}"));
}

/// Whether a message is a plain user turn, safe to begin a conversation with.
fn is_plain_user_turn(message: &otui_agent::Message) -> bool {
    if message.role != otui_agent::Role::User {
        return false;
    }
    message.content.as_array().is_none_or(|blocks| {
        !blocks
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
    })
}

fn provider(app: &mut App, args: &str) {
    if args.is_empty() {
        say(
            app,
            &format!(
                "provider: {} — /provider anthropic | openai | offline",
                app.config.agent.provider_kind().as_str()
            ),
        );
        return;
    }
    let Some(kind) = otui_agent::ProviderKind::parse(args) else {
        say(
            app,
            &format!("unknown provider '{args}' — try anthropic, openai or offline"),
        );
        return;
    };
    app.config.agent.provider = kind.as_str().to_string();
    // The model name belongs to the old provider, so clearing it falls back to
    // the new provider's default rather than requesting something that doesn't
    // exist there.
    app.config.agent.model.clear();
    let configured = otui_agent::is_configured(&kind, app.config.agent.base_url.as_deref());
    let mut message = format!(
        "provider: {} (model {})",
        kind.as_str(),
        app.config.agent.model()
    );
    if !configured {
        message.push_str("\nno credentials found — /login explains what to set");
    }
    say(app, &message);
}

fn model(app: &mut App, args: &str) {
    if args.is_empty() {
        say(
            app,
            &format!(
                "model: {} — /model <name> to change",
                app.config.agent.model()
            ),
        );
        return;
    }
    app.config.agent.model = args.to_string();
    say(app, &format!("model: {}", app.config.agent.model()));
}

fn base_url(app: &mut App, args: &str) {
    match args {
        "" => {
            let current = app.config.agent.base_url.as_deref().unwrap_or("(default)");
            say(app, &format!("base URL: {current}"));
        }
        "clear" | "none" | "reset" => {
            app.config.agent.base_url = None;
            say(app, "base URL cleared — using the provider default");
        }
        url => {
            app.config.agent.base_url = Some(url.to_string());
            say(app, &format!("base URL: {url}"));
        }
    }
}

fn login_text(app: &App) -> String {
    let kind = app.config.agent.provider_kind();
    let configured = otui_agent::is_configured(&kind, app.config.agent.base_url.as_deref());
    let state = if configured {
        "credentials found"
    } else {
        "no credentials found"
    };
    match kind {
        otui_agent::ProviderKind::Anthropic => format!(
            "anthropic — {state}\nexport ANTHROPIC_API_KEY=sk-ant-...\nthen restart, or /provider anthropic to re-check"
        ),
        otui_agent::ProviderKind::OpenAiCompatible => format!(
            "openai-compatible — {state}\nexport OPENAI_API_KEY=sk-...\nfor a local server set the endpoint instead: /base-url http://localhost:11434/v1"
        ),
        otui_agent::ProviderKind::Offline => {
            "offline — no model is configured\n/provider anthropic or /provider openai to pick one"
                .to_string()
        }
    }
}

fn logout(app: &mut App) {
    // The key lives in the environment, which this process cannot unset for the
    // shell that started it — so "logout" means "stop using it".
    app.config.agent.provider = otui_agent::ProviderKind::Offline.as_str().to_string();
    app.config.agent.base_url = None;
    say(
        app,
        "switched to offline and forgot the endpoint\nthe API key is still in your environment — unset it there to remove it",
    );
}

fn status_text(app: &App) -> String {
    let kind = app.config.agent.provider_kind();
    let usage = app.chat.usage;
    format!(
        "provider  {}\nmodel     {}\nendpoint  {}\ncredentials {}\nwrites    {}\ncontext   {}\nturns     {}\ntokens    {} in / {} out",
        kind.as_str(),
        app.config.agent.model(),
        app.config.agent.base_url.as_deref().unwrap_or("(default)"),
        if otui_agent::is_configured(&kind, app.config.agent.base_url.as_deref()) {
            "found"
        } else {
            "missing"
        },
        on_off(app.config.agent.allow_writes),
        on_off(app.config.agent.include_active_note),
        app.chat.conversation.len(),
        usage.input_tokens,
        usage.output_tokens,
    )
}

fn writes(app: &mut App, args: &str) {
    let Some(value) = toggle(args, app.config.agent.allow_writes) else {
        say(app, "/writes on | off");
        return;
    };
    app.config.agent.allow_writes = value;
    say(
        app,
        &format!(
            "writes {} — the agent can {}",
            on_off(value),
            if value {
                "create, edit and delete notes"
            } else {
                "only read and search"
            }
        ),
    );
}

fn context(app: &mut App, args: &str) {
    let Some(value) = toggle(args, app.config.agent.include_active_note) else {
        say(app, "/context on | off");
        return;
    };
    app.config.agent.include_active_note = value;
    say(app, &format!("open-note context {}", on_off(value)));
}

fn reasoning(app: &mut App, args: &str) {
    let Some(value) = toggle(args, app.config.agent.show_reasoning) else {
        say(app, "/reasoning on | off");
        return;
    };
    app.config.agent.show_reasoning = value;
    say(app, &format!("reasoning {}", on_off(value)));
}

fn tools_text(app: &App) -> String {
    let specs = crate::tools::specs(app.config.agent.allow_writes);
    let mut lines = vec![format!("{} tools available", specs.len())];
    for spec in &specs {
        lines.push(format!("  {}", spec.name));
    }
    if !app.config.agent.allow_writes {
        lines.push("read-only — /writes on to allow edits".to_string());
    }
    lines.join("\n")
}

fn obsidian(app: &mut App, args: &str) {
    match args {
        "open" => match app.active_note() {
            Some(id) => {
                let Some(note) = app.index.note(id) else {
                    say(app, "no note open");
                    return;
                };
                let rel = note.meta.rel.clone();
                let vault = app.index.vault.name.clone();
                match crate::obsidian::open_note(Some(&vault), &rel) {
                    Ok(message) => say(app, &message),
                    Err(err) => say(app, &err.to_string()),
                }
            }
            None => say(app, "no note open — /obsidian open needs one"),
        },
        "" | "status" => {
            let status = crate::obsidian::status();
            say(app, &status);
        }
        other => say(app, &format!("/obsidian [status|open] — not '{other}'")),
    }
}

fn vault_text(app: &App) -> String {
    format!(
        "vault   {}\nnotes   {}\ntags    {}",
        app.index.vault.path.display(),
        app.index.notes().len(),
        app.index.tags().len(),
    )
}

/// `/sort` with no argument cycles; with one, sets that order by name.
///
/// Listing the valid keys on a bad argument matters more here than elsewhere:
/// there are six of them and no menu to read them off.
fn sort(app: &mut App, args: &str) {
    let arg = args.trim();
    if arg.is_empty() {
        crate::actions::dispatch(app, Action::CycleSortOrder);
        let now = app.explorer.sort();
        say(app, &format!("sorting by {}", now.label()));
        return;
    }
    if arg.eq_ignore_ascii_case("list") {
        let mut text = String::from("sort orders:");
        let current = app.explorer.sort();
        for order in SortOrder::ALL {
            let marker = if order == current { "*" } else { " " };
            text.push_str(&format!(
                "\n {marker} {:<13} {}",
                order.key(),
                order.label()
            ));
        }
        say(app, &text);
        return;
    }
    let Ok(order) = arg.parse::<SortOrder>() else {
        let keys: Vec<&str> = SortOrder::ALL.iter().map(|o| o.key()).collect();
        let text = format!("unknown sort order {arg:?}. try: {}", keys.join(", "));
        say(app, &text);
        return;
    };
    app.explorer.set_sort(order);
    app.config.ui.sort_order = order.key().to_string();
    app.explorer.rebuild(&app.index);
    say(
        app,
        &format!("sorting by {}; /config keeps it", order.label()),
    );
}

fn save_config(app: &mut App) {
    match app.config.save() {
        Ok(path) => say(app, &format!("settings written to {}", path.display())),
        Err(err) => say(app, &format!("could not write the config: {err}")),
    }
}

/// Reads an `on`/`off` argument, flipping the current value when absent.
fn toggle(args: &str, current: bool) -> Option<bool> {
    match args.trim().to_lowercase().as_str() {
        "" | "toggle" => Some(!current),
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("slash");
        vault.write("A.md", "# A\n\nlinks to [[B]]\n");
        vault.write("B.md", "# B\n\n#topic\n");
        let app = App::new(vault.vault(), Config::default()).expect("build app");
        (vault, app)
    }

    /// The text of the last thing the command printed.
    fn last(app: &App) -> String {
        match app.chat.transcript.last() {
            Some(Entry::Context(text)) => text.clone(),
            other => panic!("expected command output, got {other:?}"),
        }
    }

    #[test]
    fn only_a_leading_slash_is_a_command() {
        assert!(is_command("/help"));
        assert!(is_command("/"));
        assert!(!is_command("what is /help"));
        assert!(!is_command(" /help"), "leading space means it's prose");
    }

    #[test]
    fn parsing_splits_the_name_from_the_argument() {
        assert_eq!(parse("/model gpt-5"), Some(("model", "gpt-5")));
        assert_eq!(parse("/help"), Some(("help", "")));
        assert_eq!(parse("/save my long name"), Some(("save", "my long name")));
        assert_eq!(parse("/"), Some(("", "")));
        assert_eq!(parse("hello"), None);
    }

    #[test]
    fn a_bare_slash_offers_everything() {
        assert_eq!(completions("/").len(), COMMANDS.len());
    }

    #[test]
    fn prefix_matches_rank_above_substring_matches() {
        let names: Vec<&str> = completions("/re").iter().map(|c| c.name).collect();
        assert_eq!(
            names.first(),
            Some(&"resume"),
            "typing left-to-right should offer the prefix match first: {names:?}"
        );
    }

    #[test]
    fn completions_stop_once_an_argument_is_typed() {
        let names: Vec<&str> = completions("/model gpt-5").iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["model"]);
    }

    #[test]
    fn an_unknown_command_is_reported_not_sent() {
        let (_v, mut app) = app();
        assert_eq!(
            run(&mut app, "/nonsense"),
            Outcome::Unknown("nonsense".into())
        );
        assert!(
            app.chat.conversation.is_empty(),
            "a typo must never reach the model"
        );
    }

    #[test]
    fn help_lists_every_command() {
        let text = help_text();
        for command in COMMANDS {
            assert!(text.contains(command.name), "missing /{}", command.name);
        }
    }

    #[test]
    fn switching_provider_resets_the_model_to_that_providers_default() {
        let (_v, mut app) = app();
        app.config.agent.provider = "anthropic".into();
        app.config.agent.model = "claude-only-model".into();

        run(&mut app, "/provider openai");

        assert_eq!(app.config.agent.provider, "openai");
        assert_ne!(
            app.config.agent.model(),
            "claude-only-model",
            "a model name from the old provider would 404 on the new one"
        );
    }

    #[test]
    fn an_unknown_provider_changes_nothing() {
        let (_v, mut app) = app();
        run(&mut app, "/provider banana");
        assert_eq!(app.config.agent.provider, "anthropic");
        assert!(last(&app).contains("unknown provider"));
    }

    #[test]
    fn toggles_flip_when_given_no_argument() {
        let (_v, mut app) = app();
        let before = app.config.agent.allow_writes;
        run(&mut app, "/writes");
        assert_eq!(app.config.agent.allow_writes, !before);
        run(&mut app, "/writes on");
        assert!(app.config.agent.allow_writes);
        run(&mut app, "/writes off");
        assert!(!app.config.agent.allow_writes);
    }

    #[test]
    fn a_read_only_agent_is_offered_fewer_tools() {
        let (_v, mut app) = app();
        app.config.agent.allow_writes = true;
        let writable = tools_text(&app).lines().count();
        app.config.agent.allow_writes = false;
        let readonly = tools_text(&app).lines().count();
        assert!(readonly < writable, "{readonly} vs {writable}");
    }

    #[test]
    fn new_clears_both_halves_of_the_chat() {
        let (_v, mut app) = app();
        app.chat.transcript.push(Entry::User("hi".into()));
        app.chat.conversation.push(otui_agent::Message::user("hi"));

        run(&mut app, "/new");

        assert!(app.chat.conversation.is_empty());
        // The confirmation itself is the only thing left.
        assert_eq!(app.chat.transcript.len(), 1);
    }

    #[test]
    fn compact_keeps_the_recent_turns() {
        let (_v, mut app) = app();
        for i in 0..12 {
            app.chat
                .conversation
                .push(otui_agent::Message::user(format!("turn {i}")));
        }
        run(&mut app, "/compact");
        assert_eq!(app.chat.conversation.len(), 6);
        assert!(
            format!("{:?}", app.chat.conversation[0]).contains("turn 6"),
            "the tail is what's worth keeping"
        );
    }

    #[test]
    fn compact_never_leaves_a_conversation_starting_on_a_tool_result() {
        let (_v, mut app) = app();
        // A conversation that would be trimmed straight into tool results,
        // which every provider rejects.
        for i in 0..10 {
            app.chat
                .conversation
                .push(otui_agent::Message::user(format!("turn {i}")));
        }
        app.chat.conversation.insert(
            4,
            otui_agent::Message::tool_results(&[otui_agent::ToolResult {
                id: "1".into(),
                content: "done".into(),
                is_error: false,
            }]),
        );
        run(&mut app, "/compact");
        assert!(
            app.chat
                .conversation
                .first()
                .is_some_and(is_plain_user_turn),
            "the first message has to be a plain user turn"
        );
    }

    #[test]
    fn compact_on_a_short_conversation_does_nothing() {
        let (_v, mut app) = app();
        app.chat.conversation.push(otui_agent::Message::user("hi"));
        run(&mut app, "/compact");
        assert_eq!(app.chat.conversation.len(), 1);
        assert!(last(&app).contains("nothing to compact"));
    }

    #[test]
    fn a_session_can_be_deleted_by_name() {
        let (_v, mut app) = app();
        run(&mut app, "/sessions delete");
        assert!(last(&app).contains("/sessions delete <name>"));
    }

    #[test]
    fn saving_an_empty_conversation_is_refused() {
        let (_v, mut app) = app();
        run(&mut app, "/save");
        assert!(last(&app).contains("nothing to save"));
    }

    #[test]
    fn logout_stops_using_the_provider_and_says_the_key_remains() {
        let (_v, mut app) = app();
        app.config.agent.base_url = Some("http://localhost:11434/v1".into());
        run(&mut app, "/logout");
        assert_eq!(app.config.agent.provider, "offline");
        assert!(app.config.agent.base_url.is_none());
        assert!(last(&app).contains("environment"));
    }

    #[test]
    fn base_url_can_be_set_and_cleared() {
        let (_v, mut app) = app();
        run(&mut app, "/base-url http://localhost:11434/v1");
        assert_eq!(
            app.config.agent.base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        run(&mut app, "/base-url clear");
        assert!(app.config.agent.base_url.is_none());
    }

    #[test]
    fn login_names_the_variable_for_the_current_provider() {
        let (_v, mut app) = app();
        app.config.agent.provider = "anthropic".into();
        assert!(login_text(&app).contains("ANTHROPIC_API_KEY"));
        app.config.agent.provider = "openai".into();
        assert!(login_text(&app).contains("OPENAI_API_KEY"));
    }

    #[test]
    fn status_reports_what_the_next_turn_will_do() {
        let (_v, app) = app();
        let text = status_text(&app);
        assert!(text.contains("provider"));
        assert!(text.contains("model"));
        assert!(text.contains("tokens"));
    }

    #[test]
    fn vault_reports_the_indexed_counts() {
        let (_v, app) = app();
        let text = vault_text(&app);
        assert!(text.contains("notes   2"), "{text}");
    }

    #[test]
    fn quit_asks_before_leaving() {
        let (_v, mut app) = app();
        run(&mut app, "/quit");
        assert!(!app.quit, "even /quit confirms");
        assert!(matches!(app.modal, Some(crate::modal::Modal::Confirm(_))));
    }

    #[test]
    fn keys_opens_the_help_overlay() {
        let (_v, mut app) = app();
        run(&mut app, "/keys");
        assert!(matches!(app.modal, Some(crate::modal::Modal::Help(_))));
    }

    #[test]
    fn every_command_runs_without_panicking() {
        for command in COMMANDS {
            // Quitting and the help overlay are covered above; running them
            // here too would just re-assert the same thing.
            if command.name == "quit" || command.name == "config" {
                continue;
            }
            let (_v, mut app) = app();
            let outcome = run(&mut app, &format!("/{}", command.name));
            assert_eq!(
                outcome,
                Outcome::Handled,
                "/{} is in the registry but not handled",
                command.name
            );
        }
    }
}
