//! The tools the agent can call.
//!
//! These are the app's own operations, exposed by name. They run on the UI
//! thread against the live [`App`], so a note the agent creates is in the
//! explorer before the model has finished its sentence, and `open_note` really
//! opens a tab.
//!
//! Descriptions state *when* to call each tool, not just what it does — current
//! models under-reach for tools given a bare capability description, and the
//! trigger condition is what fixes that.

use otui_agent::{ToolCall, ToolOutcome, ToolSpec};
use otui_core::graph::NodeKind;
use otui_core::index::{LinkTarget, NoteId};
use otui_core::search::{self, SearchLimits};
use serde_json::Value;

use crate::app::App;

/// Cap on how much note text goes back to the model in one call.
///
/// Large enough for any real note, small enough that a runaway read can't
/// blow out the context window in a single tool result.
const MAX_NOTE_CHARS: usize = 20_000;

/// The tool set, filtered by whether writes are allowed.
#[must_use]
pub fn specs(allow_writes: bool) -> Vec<ToolSpec> {
    let mut specs = vec![
        ToolSpec::new(
            "search_notes",
            "Search the full text of every note. Call this whenever the answer depends on what is written in the vault, before answering any question about the user's notes.",
            ToolSpec::object_schema(&[
                ("query", "string", "Text to look for. Case-insensitive unless it contains capitals.", true),
                ("limit", "integer", "Maximum notes to return (default 20).", false),
            ]),
        ),
        ToolSpec::new(
            "find_notes",
            "Find notes by name, fuzzily. Call this when the user refers to a note by an approximate or partial title.",
            ToolSpec::object_schema(&[
                ("query", "string", "Part of a note name or path.", true),
                ("limit", "integer", "Maximum results (default 20).", false),
            ]),
        ),
        ToolSpec::new(
            "read_note",
            "Read a note's full content. Call this before summarizing, quoting or editing a note.",
            ToolSpec::object_schema(&[(
                "name",
                "string",
                "Note name, alias, or vault-relative path.",
                true,
            )]),
        ),
        ToolSpec::new(
            "list_notes",
            "List notes, optionally in one folder. Call this to get oriented in an unfamiliar vault.",
            ToolSpec::object_schema(&[
                ("folder", "string", "Vault-relative folder; omit for the whole vault.", false),
                ("limit", "integer", "Maximum results (default 100).", false),
            ]),
        ),
        ToolSpec::new(
            "list_tags",
            "List the vault's tags with how many notes use each. Call this when asked how notes are categorized.",
            ToolSpec::object_schema(&[(
                "prefix",
                "string",
                "Only tags starting with this, e.g. `project`.",
                false,
            )]),
        ),
        ToolSpec::new(
            "get_links",
            "Show what a note links to, what links back to it, and any broken links. Call this when asked how notes relate, what references something, or what is orphaned.",
            ToolSpec::object_schema(&[(
                "name",
                "string",
                "Note name or vault-relative path.",
                true,
            )]),
        ),
        ToolSpec::new(
            "graph_neighborhood",
            "Show the local graph around a note: everything within N hops. Call this to explain how a topic connects to the rest of the vault.",
            ToolSpec::object_schema(&[
                ("name", "string", "Note at the center.", true),
                ("depth", "integer", "Hops to include (default 1, max 3).", false),
            ]),
        ),
        ToolSpec::new(
            "open_note",
            "Open a note in the editor so the user can see it. Call this whenever you reference a note the user would want to look at.",
            ToolSpec::object_schema(&[(
                "name",
                "string",
                "Note name or vault-relative path.",
                true,
            )]),
        ),
        ToolSpec::new(
            "show_graph",
            "Switch the main pane to the graph view, optionally centered on a note. Call this when the answer is about the vault's structure.",
            ToolSpec::object_schema(&[(
                "name",
                "string",
                "Note to center on; omit for the whole vault graph.",
                false,
            )]),
        ),
    ];

    if allow_writes {
        specs.extend([
            ToolSpec::new(
                "create_note",
                "Create a new note. Include `[[wikilinks]]` to related notes in the content so it isn't orphaned in the graph.",
                ToolSpec::object_schema(&[
                    ("name", "string", "Note name, optionally with a folder: `Projects/Ideas`.", true),
                    ("content", "string", "Markdown body. Include a `# Heading` and links to related notes.", true),
                ]),
            ),
            ToolSpec::new(
                "append_note",
                "Append text to the end of an existing note. Prefer this over rewriting when adding to a note.",
                ToolSpec::object_schema(&[
                    ("name", "string", "Note name or path.", true),
                    ("text", "string", "Markdown to append.", true),
                ]),
            ),
            ToolSpec::new(
                "replace_note",
                "Replace a note's entire content. Read it first; this discards whatever was there.",
                ToolSpec::object_schema(&[
                    ("name", "string", "Note name or path.", true),
                    ("content", "string", "The complete new content.", true),
                ]),
            ),
            ToolSpec::new(
                "link_notes",
                "Add a wikilink from one note to another, creating a graph edge. Call this when the user asks to connect, relate or organize notes.",
                ToolSpec::object_schema(&[
                    ("from", "string", "Note the link is added to.", true),
                    ("to", "string", "Note being linked to.", true),
                    ("context", "string", "Optional sentence to write around the link.", false),
                ]),
            ),
            ToolSpec::new(
                "rename_note",
                "Rename a note; every wikilink pointing at it is updated automatically. Call this when the user asks to rename or retitle a note, rather than creating a copy.",
                ToolSpec::object_schema(&[
                    ("name", "string", "Current note name or path.", true),
                    ("new_name", "string", "New name, without a folder or extension.", true),
                ]),
            ),
            ToolSpec::new(
                "delete_note",
                "Move a note to the vault's trash. Ask the user before calling this unless they explicitly asked for a deletion.",
                ToolSpec::object_schema(&[(
                    "name",
                    "string",
                    "Note name or path.",
                    true,
                )]),
            ),
        ]);
    }

    specs
}

/// Runs one tool call against the live app.
pub fn execute(app: &mut App, call: &ToolCall) -> ToolOutcome {
    let args = &call.arguments;
    match call.name.as_str() {
        "search_notes" => search_notes(app, args),
        "find_notes" => find_notes(app, args),
        "read_note" => read_note(app, args),
        "list_notes" => list_notes(app, args),
        "list_tags" => list_tags(app, args),
        "get_links" => get_links(app, args),
        "graph_neighborhood" => graph_neighborhood(app, args),
        "open_note" => open_note(app, args),
        "show_graph" => show_graph(app, args),
        "create_note" => create_note(app, args),
        "append_note" => append_note(app, args),
        "replace_note" => replace_note(app, args),
        "link_notes" => link_notes(app, args),
        "rename_note" => rename_note(app, args),
        "delete_note" => delete_note(app, args),
        other => ToolOutcome::error(format!("no tool named `{other}`")),
    }
}

// ---- argument helpers ------------------------------------------------------

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn required(args: &Value, key: &str) -> Result<String, ToolOutcome> {
    text(args, key).ok_or_else(|| ToolOutcome::error(format!("`{key}` is required")))
}

fn number(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map_or(default, |n| (n as usize).clamp(1, max))
}

/// Resolves a note reference the way a wikilink would.
fn resolve(app: &App, name: &str) -> Result<NoteId, ToolOutcome> {
    app.index.resolve(name).ok_or_else(|| {
        // A wrong name is by far the most common tool error, so the message
        // carries the fix. Recovering from it costs one turn instead of ending
        // the task.
        let suggestions = suggest(app, name);
        if suggestions.is_empty() {
            ToolOutcome::error(format!("no note matching `{name}`"))
        } else {
            ToolOutcome::error(format!(
                "no note named `{name}`. Did you mean: {}?",
                suggestions.join(", ")
            ))
        }
    })
}

/// Names close to `name`, for a "did you mean" hint.
///
/// Fuzzy matching handles a query that's a subsequence of a real name
/// (`prj` → `Projects`), but not a guess that's *longer* than the real name
/// (`Bee` → `B`), which is exactly what a model does when it invents a
/// plausible title. Both directions are checked.
fn suggest(app: &App, name: &str) -> Vec<String> {
    let mut names: Vec<String> = search::search_notes(&app.index, name, 3)
        .into_iter()
        .filter_map(|m| app.index.note(m.id).map(|n| n.meta.rel.clone()))
        .collect();

    if names.is_empty() {
        let needle = name.to_lowercase();
        names = app
            .index
            .notes()
            .iter()
            .filter(|note| {
                let stem = note.meta.stem.to_lowercase();
                needle.starts_with(&stem) || stem.starts_with(&needle) || stem.contains(&needle)
            })
            .take(3)
            .map(|note| note.meta.rel.clone())
            .collect();
    }

    names
}

macro_rules! arg {
    ($args:expr, $key:literal) => {
        match required($args, $key) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        }
    };
}

macro_rules! note {
    ($app:expr, $name:expr) => {
        match resolve($app, &$name) {
            Ok(id) => id,
            Err(outcome) => return outcome,
        }
    };
}

// ---- read-only tools -------------------------------------------------------

fn search_notes(app: &mut App, args: &Value) -> ToolOutcome {
    let query = arg!(args, "query");
    let limit = number(args, "limit", 20, 100);

    let results = search::search_content(
        &app.index,
        &query,
        SearchLimits {
            max_notes: limit,
            max_hits_per_note: 3,
        },
    );

    if results.is_empty() {
        return ToolOutcome::ok(
            format!("no matches for \"{query}\""),
            format!("No note contains \"{query}\"."),
        );
    }

    let mut out = format!("{} notes match \"{query}\":\n", results.len());
    for result in &results {
        let Some(note) = app.index.note(result.id) else {
            continue;
        };
        out.push_str(&format!(
            "\n{} ({} hits)\n",
            note.meta.rel,
            result.hits.len()
        ));
        for hit in &result.hits {
            out.push_str(&format!(
                "  line {}: {}\n",
                hit.line + 1,
                clip(&hit.text, 160)
            ));
        }
    }

    ToolOutcome::ok(format!("{} notes match \"{query}\"", results.len()), out)
}

fn find_notes(app: &mut App, args: &Value) -> ToolOutcome {
    let query = arg!(args, "query");
    let limit = number(args, "limit", 20, 100);

    let matches = search::search_notes(&app.index, &query, limit);
    if matches.is_empty() {
        return ToolOutcome::ok(
            format!("no note named like \"{query}\""),
            format!("No note name resembles \"{query}\"."),
        );
    }

    let lines: Vec<String> = matches
        .iter()
        .filter_map(|m| app.index.note(m.id))
        .map(|n| format!("{}: {} words", n.meta.rel, n.words))
        .collect();

    ToolOutcome::ok(format!("{} name matches", lines.len()), lines.join("\n"))
}

fn read_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let id = note!(app, name);

    match app.index.read(id) {
        Ok(content) => {
            let rel = app.index.note(id).map_or(name, |n| n.meta.rel.clone());
            ToolOutcome::ok(format!("read {rel}"), clip(&content, MAX_NOTE_CHARS))
        }
        Err(err) => ToolOutcome::error(format!("could not read `{name}`: {err}")),
    }
}

fn list_notes(app: &mut App, args: &Value) -> ToolOutcome {
    let folder = text(args, "folder").unwrap_or_default();
    let limit = number(args, "limit", 100, 500);
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{}/", folder.trim_matches('/'))
    };

    let lines: Vec<String> = app
        .index
        .notes()
        .iter()
        .filter(|n| prefix.is_empty() || n.meta.rel.starts_with(&prefix))
        .take(limit)
        .map(|n| {
            let tags = if n.tags.is_empty() {
                String::new()
            } else {
                format!("  #{}", n.tags.join(" #"))
            };
            format!("{}{tags}", n.meta.rel)
        })
        .collect();

    if lines.is_empty() {
        return ToolOutcome::ok("no notes there", format!("No notes under `{folder}`."));
    }
    ToolOutcome::ok(format!("{} notes", lines.len()), lines.join("\n"))
}

fn list_tags(app: &mut App, args: &Value) -> ToolOutcome {
    let prefix = text(args, "prefix").unwrap_or_default();
    let lines: Vec<String> = app
        .index
        .tags()
        .iter()
        .filter(|(tag, _)| prefix.is_empty() || tag.starts_with(&prefix))
        .map(|(tag, notes)| format!("#{tag}: {} notes", notes.len()))
        .collect();

    if lines.is_empty() {
        return ToolOutcome::ok("no tags", "This vault has no tags yet.".to_string());
    }
    ToolOutcome::ok(format!("{} tags", lines.len()), lines.join("\n"))
}

fn get_links(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let id = note!(app, name);

    let Some(note) = app.index.note(id) else {
        return ToolOutcome::error("note vanished");
    };
    let rel = note.meta.rel.clone();

    let mut outgoing = Vec::new();
    let mut broken = Vec::new();
    for link in &note.links {
        match &link.target {
            LinkTarget::Note(target) => {
                if let Some(t) = app.index.note(*target) {
                    outgoing.push(t.meta.rel.clone());
                }
            }
            LinkTarget::Unresolved(missing) => broken.push(missing.clone()),
            _ => {}
        }
    }
    outgoing.dedup();
    broken.dedup();

    let backlinks: Vec<String> = app
        .index
        .backlinks(id)
        .iter()
        .filter_map(|b| app.index.note(b.source).map(|n| n.meta.rel.clone()))
        .collect();

    let mut out = format!("Links for {rel}:\n");
    out.push_str(&format!(
        "\nLinks to ({}): {}\n",
        outgoing.len(),
        or_none(&outgoing)
    ));
    out.push_str(&format!(
        "Linked from ({}): {}\n",
        backlinks.len(),
        or_none(&backlinks)
    ));
    if !broken.is_empty() {
        out.push_str(&format!(
            "Broken links ({}): {}\n",
            broken.len(),
            broken.join(", ")
        ));
    }

    ToolOutcome::ok(
        format!("{} out, {} in", outgoing.len(), backlinks.len()),
        out,
    )
}

fn graph_neighborhood(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let id = note!(app, name);
    let depth = number(args, "depth", 1, 3);

    let graph = otui_core::graph::Graph::build(&app.index, &Default::default());
    let Some(origin) = graph.node_of_note(id) else {
        return ToolOutcome::ok(
            "not in the graph",
            format!("`{name}` has no links, so it doesn't appear in the graph."),
        );
    };

    let nodes = graph.neighborhood(origin, depth);
    let mut lines = Vec::new();
    for node_index in &nodes {
        let Some(node) = graph.nodes.get(*node_index) else {
            continue;
        };
        let kind = match node.kind {
            NodeKind::Note(_) => "note",
            NodeKind::Unresolved => "missing note",
            NodeKind::Tag => "tag",
            NodeKind::Attachment => "attachment",
        };
        lines.push(format!("{} ({kind}, {} links)", node.label, node.degree));
    }

    ToolOutcome::ok(
        format!("{} nodes within {depth} hop(s)", nodes.len()),
        format!("Around {name} within {depth} hop(s):\n{}", lines.join("\n")),
    )
}

// ---- UI actions ------------------------------------------------------------

fn open_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let id = note!(app, name);
    app.open_note(id);
    app.explorer.reveal(&app.index, id);
    let rel = app.index.note(id).map_or(name, |n| n.meta.rel.clone());
    ToolOutcome::ok(
        format!("opened {rel}"),
        format!("`{rel}` is now open in the editor."),
    )
}

fn show_graph(app: &mut App, args: &Value) -> ToolOutcome {
    let focus = match text(args, "name") {
        Some(name) => Some(note!(app, name)),
        None => None,
    };
    app.open_graph(focus);

    let summary = match focus.and_then(|id| app.index.note(id)) {
        Some(note) => format!("graph centered on {}", note.meta.rel),
        None => "graph view".to_string(),
    };
    ToolOutcome::ok(summary.clone(), format!("Showing the {summary}."))
}

// ---- write tools -----------------------------------------------------------

fn create_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Creating over an existing note would silently destroy it; picking a free
    // name is what Obsidian does and is always recoverable.
    let rel = match app.index.note_rel(&name) {
        Ok(rel) => app.index.unique_rel(&rel),
        Err(err) => return ToolOutcome::error(format!("invalid name `{name}`: {err}")),
    };

    match app.index.create_note(&rel, content) {
        Ok(id) => {
            app.explorer.rebuild(&app.index);
            app.open_note(id);
            app.explorer.reveal(&app.index, id);
            if app.graph.is_some() {
                app.refresh_graph();
            }
            ToolOutcome::ok(
                format!("created {rel}"),
                format!("Created `{rel}` and opened it."),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not create `{rel}`: {err}")),
    }
}

fn append_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let addition = arg!(args, "text");
    let id = note!(app, name);

    match app.index.append_note(id, &addition) {
        Ok(()) => {
            reload_open_tab(app, id);
            let rel = app.index.note(id).map_or(name, |n| n.meta.rel.clone());
            ToolOutcome::ok(
                format!("appended to {rel}"),
                format!("Appended {} characters to `{rel}`.", addition.len()),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not append: {err}")),
    }
}

fn replace_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let content = arg!(args, "content");
    let id = note!(app, name);

    match app.index.write_note(id, &content) {
        Ok(()) => {
            reload_open_tab(app, id);
            let rel = app.index.note(id).map_or(name, |n| n.meta.rel.clone());
            ToolOutcome::ok(
                format!("rewrote {rel}"),
                format!("Replaced the contents of `{rel}`."),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not write: {err}")),
    }
}

fn link_notes(app: &mut App, args: &Value) -> ToolOutcome {
    let from = arg!(args, "from");
    let to = arg!(args, "to");
    let from_id = note!(app, from);

    // The target need not exist: an unresolved link is a real graph node and a
    // legitimate way to plan a note that isn't written yet.
    let target_name = app
        .index
        .resolve(&to)
        .and_then(|id| app.index.note(id).map(|n| n.meta.stem.clone()))
        .unwrap_or_else(|| to.clone());

    let line = match text(args, "context") {
        Some(context) if context.contains("[[") => context,
        Some(context) => format!("{context} [[{target_name}]]"),
        None => format!("[[{target_name}]]"),
    };

    match app.index.append_note(from_id, &line) {
        Ok(()) => {
            reload_open_tab(app, from_id);
            if app.graph.is_some() {
                app.refresh_graph();
            }
            let rel = app.index.note(from_id).map_or(from, |n| n.meta.rel.clone());
            ToolOutcome::ok(
                format!("linked {rel} → {target_name}"),
                format!("Added a link from `{rel}` to `{target_name}`."),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not link: {err}")),
    }
}

fn rename_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let new_name = arg!(args, "new_name");
    let id = note!(app, name);

    match app.index.rename_note(id, &new_name) {
        Ok(new_id) => {
            // A rename renumbers every note, so open tabs must be re-resolved.
            app.refresh();
            app.explorer.rebuild(&app.index);
            let rel = app
                .index
                .note(new_id)
                .map_or(new_name, |n| n.meta.rel.clone());
            ToolOutcome::ok(
                format!("renamed to {rel}"),
                format!("Renamed `{name}` to `{rel}` and updated every wikilink to it."),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not rename: {err}")),
    }
}

fn delete_note(app: &mut App, args: &Value) -> ToolOutcome {
    let name = arg!(args, "name");
    let id = note!(app, name);
    let rel = app
        .index
        .note(id)
        .map_or_else(|| name.clone(), |n| n.meta.rel.clone());

    match app.index.delete_note(id) {
        Ok(trashed) => {
            app.tabs.retain(|t| t.note != id);
            app.active_tab = if app.tabs.is_empty() { None } else { Some(0) };
            app.refresh();
            ToolOutcome::ok(
                format!("deleted {rel}"),
                format!(
                    "Moved `{rel}` to the vault trash ({}). It can be restored from there.",
                    trashed.display()
                ),
            )
        }
        Err(err) => ToolOutcome::error(format!("could not delete: {err}")),
    }
}

/// Reloads an open tab after the agent changed the file underneath it.
///
/// Without this the user would keep editing stale text and silently overwrite
/// the agent's change on the next save.
fn reload_open_tab(app: &mut App, id: NoteId) {
    let Some(position) = app.tabs.iter().position(|t| t.note == id) else {
        return;
    };
    if app.tabs[position]
        .editor
        .as_ref()
        .is_some_and(|e| e.is_modified())
    {
        // Unsaved user edits win; dropping them to show the agent's version
        // would be data loss.
        app.error("the assistant changed a note you have unsaved edits in");
        return;
    }
    app.tabs[position].editor = None;
    app.explorer.rebuild(&app.index);
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n…(truncated)");
    out
}

fn or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;
    use serde_json::json;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("tools");
        vault.write("A.md", "# A\n\n#topic\nSee [[B]] and [[Ghost]].\n");
        vault.write("B.md", "# B\n\nThe needle is here.\n");
        vault.write("Projects/C.md", "# C\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");
        (vault, app)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "t".into(),
            name: name.into(),
            arguments: args,
        }
    }

    fn run(app: &mut App, name: &str, args: Value) -> ToolOutcome {
        execute(app, &call(name, args))
    }

    #[test]
    fn write_tools_are_hidden_when_writes_are_disabled() {
        let read_only: Vec<String> = specs(false).into_iter().map(|s| s.name).collect();
        let writable: Vec<String> = specs(true).into_iter().map(|s| s.name).collect();

        assert!(read_only.contains(&"search_notes".to_string()));
        assert!(!read_only.contains(&"create_note".to_string()));
        assert!(writable.contains(&"create_note".to_string()));
        assert!(writable.contains(&"delete_note".to_string()));
    }

    #[test]
    fn every_tool_description_states_when_to_call_it() {
        for spec in specs(true) {
            assert!(
                spec.description.contains("Call this")
                    || spec.description.contains("Prefer this")
                    || spec.description.contains("Include")
                    || spec.description.contains("Ask the user")
                    || spec.description.contains("Read it first"),
                "`{}` needs a trigger condition, not just a capability",
                spec.name
            );
        }
    }

    #[test]
    fn search_returns_matching_lines() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "search_notes", json!({ "query": "needle" }));

        assert!(!outcome.is_error);
        assert!(outcome.content.contains("B.md"));
        assert!(outcome.content.contains("The needle is here."));
    }

    #[test]
    fn an_unknown_note_name_suggests_alternatives() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "read_note", json!({ "name": "Bee" }));

        assert!(outcome.is_error);
        assert!(
            outcome.content.contains("Did you mean"),
            "got {:?}",
            outcome.content
        );
    }

    #[test]
    fn missing_required_arguments_are_reported_clearly() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "read_note", json!({}));
        assert!(outcome.is_error);
        assert!(outcome.content.contains("`name` is required"));
    }

    #[test]
    fn unknown_tools_error_rather_than_panic() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "launch_missiles", json!({}));
        assert!(outcome.is_error);
    }

    #[test]
    fn get_links_reports_both_directions_and_broken_links() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "get_links", json!({ "name": "A" }));

        assert!(outcome.content.contains("Links to (1): B.md"));
        assert!(outcome.content.contains("Broken links (1): Ghost"));
    }

    #[test]
    fn create_note_writes_opens_and_indexes() {
        let (vault, mut app) = app();
        let outcome = run(
            &mut app,
            "create_note",
            json!({ "name": "Projects/New", "content": "# New\n\nSee [[A]].\n" }),
        );

        assert!(!outcome.is_error, "{:?}", outcome.content);
        assert!(vault.exists("Projects/New.md"));

        let id = app.index.id_of_rel("Projects/New.md").expect("indexed");
        assert_eq!(app.active_note(), Some(id), "the new note is opened");

        let a = app.index.id_of_rel("A.md").unwrap();
        assert!(
            app.index.backlinks(a).iter().any(|b| b.source == id),
            "its links are live in the index"
        );
    }

    #[test]
    fn create_note_never_overwrites_an_existing_note() {
        let (vault, mut app) = app();
        run(
            &mut app,
            "create_note",
            json!({ "name": "B", "content": "replacement" }),
        );

        assert!(
            vault.read("B.md").contains("The needle is here."),
            "the original must survive"
        );
        assert!(vault.exists("B 1.md"), "a free name is chosen instead");
    }

    #[test]
    fn link_notes_creates_a_real_graph_edge() {
        let (_v, mut app) = app();
        let outcome = run(
            &mut app,
            "link_notes",
            json!({ "from": "B", "to": "Projects/C", "context": "Related:" }),
        );

        assert!(!outcome.is_error, "{:?}", outcome.content);

        let b = app.index.id_of_rel("B.md").unwrap();
        let c = app.index.id_of_rel("Projects/C.md").unwrap();
        assert!(app.index.note(b).unwrap().outgoing().contains(&c));
    }

    #[test]
    fn link_notes_accepts_a_target_that_does_not_exist_yet() {
        let (_v, mut app) = app();
        let outcome = run(
            &mut app,
            "link_notes",
            json!({ "from": "B", "to": "Future Note" }),
        );

        assert!(!outcome.is_error);
        assert!(app.index.unresolved().contains_key("Future Note"));
    }

    #[test]
    fn rename_updates_links_and_keeps_tabs_valid() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        let outcome = run(
            &mut app,
            "rename_note",
            json!({ "name": "B", "new_name": "Bravo" }),
        );
        assert!(!outcome.is_error, "{:?}", outcome.content);

        assert!(vault.exists("Bravo.md"));
        assert!(vault.read("A.md").contains("[[Bravo]]"));

        let title = app.note_title(app.active_note().expect("tab survives"));
        assert_eq!(title, "Bravo");
    }

    #[test]
    fn delete_moves_to_trash_and_closes_the_tab() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        let outcome = run(&mut app, "delete_note", json!({ "name": "B" }));

        assert!(!outcome.is_error);
        assert!(!vault.exists("B.md"));
        assert!(
            outcome.content.contains("restored"),
            "recovery is explained"
        );
        assert!(app.tabs.is_empty());
    }

    #[test]
    fn open_note_switches_the_editor_and_reveals_it() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "open_note", json!({ "name": "Projects/C" }));

        assert!(!outcome.is_error);
        let c = app.index.id_of_rel("Projects/C.md").unwrap();
        assert_eq!(app.active_note(), Some(c));
        assert_eq!(app.explorer.selected_note(), Some(c));
    }

    #[test]
    fn show_graph_switches_the_view() {
        let (_v, mut app) = app();
        let outcome = run(&mut app, "show_graph", json!({ "name": "A" }));

        assert!(!outcome.is_error);
        assert_eq!(app.view, crate::app::View::Graph);
        assert!(app.graph.as_ref().unwrap().selected.is_some());
    }

    #[test]
    fn appending_refreshes_an_open_unmodified_tab() {
        let (_v, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        // Force the editor to exist and hold the old text.
        app.editor_mut();

        run(
            &mut app,
            "append_note",
            json!({ "name": "B", "text": "from the agent" }),
        );

        let text = app.editor_mut().expect("editor").text();
        assert!(
            text.contains("from the agent"),
            "the open tab must show the change"
        );
    }

    #[test]
    fn appending_will_not_discard_unsaved_edits() {
        let (_v, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        app.editor_mut().unwrap().insert_str("my unsaved work");

        run(
            &mut app,
            "append_note",
            json!({ "name": "B", "text": "agent text" }),
        );

        let text = app.editor_mut().expect("editor").text();
        assert!(text.contains("my unsaved work"), "user edits are preserved");
        assert!(app.status.is_error, "and the conflict is reported");
    }

    #[test]
    fn list_tools_summarize_the_vault() {
        let (_v, mut app) = app();

        let tags = run(&mut app, "list_tags", json!({}));
        assert!(tags.content.contains("#topic"));

        let notes = run(&mut app, "list_notes", json!({ "folder": "Projects" }));
        assert!(notes.content.contains("Projects/C.md"));
        assert!(!notes.content.contains("A.md"));
    }

    #[test]
    fn graph_neighborhood_lists_connected_nodes() {
        let (_v, mut app) = app();
        let outcome = run(
            &mut app,
            "graph_neighborhood",
            json!({ "name": "A", "depth": 1 }),
        );

        assert!(!outcome.is_error);
        assert!(outcome.content.contains("B"));
        assert!(outcome.content.contains("Ghost"), "missing notes show too");
    }

    #[test]
    fn long_notes_are_truncated_rather_than_flooding_the_context() {
        let vault = TempVault::new("tools-big");
        vault.write("Big.md", &"x".repeat(MAX_NOTE_CHARS * 2));
        let mut app = App::new(vault.vault(), Config::default()).expect("app");

        let outcome = run(&mut app, "read_note", json!({ "name": "Big" }));
        assert!(outcome.content.contains("(truncated)"));
        assert!(outcome.content.chars().count() < MAX_NOTE_CHARS + 100);
    }
}
