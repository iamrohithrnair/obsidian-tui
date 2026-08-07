//! Action dispatch and the command list.
//!
//! One function turns an [`Action`] into a state change. Keys, the palette,
//! sidebar clicks and the agent all go through it, so there is a single
//! definition of what each command means.

use otui_core::search;

use crate::app::{Action, App, Focus, Mode, View};
use crate::modal::{Confirm, Entry, Modal, Picker, PickerKind, Prompt, PromptIntent};

/// Every command shown in the palette, with the key that also runs it.
///
/// The palette is the discoverable surface for the whole app, so this list is
/// deliberately complete rather than a curated subset.
#[must_use]
pub fn commands() -> Vec<Entry> {
    vec![
        Entry::new("Open quick switcher", "Ctrl+O", Action::OpenSwitcher),
        Entry::new("Search all notes", "Ctrl+Shift+F", Action::OpenSearch),
        Entry::new("New note", "Ctrl+N", Action::NewNote),
        Entry::new("New folder", "", Action::NewFolder),
        Entry::new("Insert wikilink", "", Action::InsertWikiLink),
        Entry::new("Open today's daily note", "Ctrl+D", Action::DailyNote),
        Entry::new("Save", "Ctrl+S", Action::Save),
        Entry::new("Save all", "", Action::SaveAll),
        Entry::new("Rename note", "F2", Action::RenameNote),
        Entry::new("Delete note", "", Action::DeleteNote),
        Entry::new("Toggle reading / editing", "Ctrl+E", Action::ToggleMode),
        Entry::new("Close tab", "Ctrl+W", Action::CloseTab),
        Entry::new("Next tab", "Ctrl+Tab", Action::NextTab),
        Entry::new("Previous tab", "Ctrl+Shift+Tab", Action::PreviousTab),
        Entry::new("Open graph view", "Ctrl+G", Action::OpenGraph),
        Entry::new("Open local graph", "Ctrl+Shift+G", Action::OpenLocalGraph),
        Entry::new("Back to notes", "", Action::OpenNotesView),
        Entry::new("Reveal note in explorer", "", Action::RevealInExplorer),
        Entry::new("Toggle file explorer", "Ctrl+\\", Action::ToggleLeftSidebar),
        Entry::new(
            "Toggle outline sidebar",
            "Ctrl+]",
            Action::ToggleRightSidebar,
        ),
        Entry::new("Cycle sidebar panel", "Ctrl+K", Action::CycleSidePanel),
        Entry::new("Toggle assistant panel", "Ctrl+L", Action::ToggleChat),
        Entry::new(
            "Assistant: choose provider",
            "/provider",
            Action::OpenProviderPicker,
        ),
        Entry::new("Assistant: choose model", "/model", Action::OpenModelPicker),
        Entry::new("Assistant: set API key", "/key", Action::PromptApiKey),
        Entry::new("Toggle ribbon", "", Action::ToggleRibbon),
        Entry::new("Toggle shortcut hints", "", Action::ToggleHints),
        Entry::new("Change sort order", "s", Action::CycleSortOrder),
        Entry::new("Toggle line numbers", "", Action::ToggleLineNumbers),
        Entry::new("Change theme", "Ctrl+T", Action::OpenThemePicker),
        Entry::new("Open another vault", "", Action::OpenVaultPicker),
        Entry::new("Graph: toggle labels", "", Action::ToggleGraphLabels),
        Entry::new(
            "Graph: toggle unresolved notes",
            "",
            Action::ToggleGraphUnresolved,
        ),
        Entry::new("Graph: toggle tags", "", Action::ToggleGraphTags),
        Entry::new(
            "Graph: toggle attachments",
            "",
            Action::ToggleGraphAttachments,
        ),
        Entry::new("Graph: toggle orphans", "", Action::ToggleGraphOrphans),
        Entry::new("Open this note in Obsidian", "", Action::OpenInObsidian),
        Entry::new("Reload vault from disk", "", Action::Refresh),
        Entry::new("Save settings to config file", "", Action::SaveSettings),
        Entry::new("Keyboard shortcuts", "?", Action::OpenHelp),
        Entry::new("Quit", "q", Action::Quit),
    ]
}

/// Hands the open note to the Obsidian desktop app.
///
/// This is the one place emeraldian defers to Obsidian itself: everything
/// else it does by reading and writing Markdown directly, which is why it works
/// with the app closed. Opening in the GUI is the exception, since only the
/// running app can do it.
/// Steps the explorer to the next sort order.
///
/// The config is updated in memory, like every other setting the UI changes;
/// `/config` (or "Save settings") writes it to disk, so the choice survives a
/// restart.
fn cycle_sort_order(app: &mut App) {
    let next = app.explorer.sort().next();
    app.explorer.set_sort(next);
    app.config.ui.sort_order = next.key().to_string();
    app.explorer.rebuild(&app.index);
    // Rebuilding can move the selected row; keep it on screen.
    if let Some((area, _)) = app.regions.explorer {
        app.explorer.scroll_into_view(area.height as usize);
    }
    app.info(format!("sorted by {}; /config keeps it", next.label()));
}

/// Switches backend, bringing the endpoint and model with it.
///
/// The three settings only make sense together: an Anthropic model name sent to
/// Ollama is a 404, and Groq's address with OpenAI's key is a 401. So picking a
/// provider resets the other two to that provider's own, and the model is chosen
/// afterwards from what it actually offers.
///
/// The outcome is returned rather than announced, because a switch made from the
/// palette belongs in the status bar while one made with `/provider` belongs in
/// the transcript beside the command that caused it.
pub(crate) fn set_provider(app: &mut App, name: &str) -> Result<String, String> {
    let Some(preset) = otui_agent::catalog::find(name) else {
        return Err(format!(
            "unknown provider '{name}'; /provider with no name lists them"
        ));
    };

    app.config.agent.provider = preset.id.to_string();
    app.config.agent.base_url = preset.base_url.map(str::to_string);
    app.config.agent.model.clear();

    let key = crate::auth::key_for(preset.id, &app.auth);
    Ok(
        if otui_agent::has_credentials(&preset.kind, preset.base_url, key.as_deref()) {
            format!("{} · /model to choose one", preset.label)
        } else if let Some(env_var) = preset.env_var {
            format!(
                "{}: no key yet — /key to enter one, or export {env_var}",
                preset.label
            )
        } else {
            preset.label.to_string()
        },
    )
}

/// Asks the current provider for its models, to fill a picker with.
fn ask_for_models(app: &mut App) {
    let provider = app.config.agent.provider.clone();
    let Some(preset) = otui_agent::catalog::find(&provider) else {
        app.error(format!("unknown provider '{provider}'"));
        return;
    };
    if preset.kind == otui_agent::ProviderKind::Offline {
        app.info("no provider is set; /provider picks one");
        return;
    }
    if app.lookup.busy() {
        app.info("still asking…");
        return;
    }

    app.lookup.start(
        &provider,
        crate::auth::key_for(&provider, &app.auth),
        app.config.agent.base_url.clone(),
    );
    app.info(format!("asking {} for its models…", preset.label));
}

/// Opens the model picker on a list that has arrived.
pub(crate) fn show_models(app: &mut App, models: Vec<String>) {
    let current = app.config.agent.model();
    let entries = models
        .into_iter()
        .map(|name| {
            let detail = if name == current { "in use" } else { "" };
            Entry::new(name.clone(), detail, Action::SetModel(name))
        })
        .collect();
    app.modal = Some(Modal::Picker(Picker::new(PickerKind::Models, entries)));
}

fn open_in_obsidian(app: &mut App) {
    let Some(id) = app.active_note() else {
        app.error("no note open");
        return;
    };
    let Some(note) = app.index.note(id) else {
        app.error("no note open");
        return;
    };
    let rel = note.meta.rel.clone();
    let vault = app.index.vault.name.clone();
    match crate::obsidian::open_note(Some(&vault), &rel) {
        Ok(message) => app.info(message),
        Err(err) => app.error(err.to_string()),
    }
}

/// Applies an action.
pub fn dispatch(app: &mut App, action: Action) {
    // Any action beyond typing in an overlay dismisses it; leaving a stale
    // palette open over a changed view is confusing.
    let keep_modal = matches!(action, Action::OpenHelp);
    if !keep_modal {
        app.modal = None;
    }

    match action {
        // ---- navigation -------------------------------------------------
        Action::OpenNote(id) => app.open_note(id),
        Action::FollowLink(target) => app.open_or_create(&target),
        Action::RevealInExplorer => {
            if let Some(id) = app.active_note() {
                app.explorer.reveal(&app.index, id);
                app.focus = Focus::Explorer;
            }
        }
        Action::Back => {
            if let Some(previous) = app.history.pop() {
                // `open_note` would push the current note back on, undoing the
                // step; suppress that by clearing after the jump.
                let before = app.history.clone();
                app.open_note(previous);
                app.history = before;
            } else {
                app.info("no earlier note");
            }
        }

        // ---- notes ------------------------------------------------------
        Action::NewNote => {
            let folder = app.explorer.selected_folder(&app.index);
            let base = if folder.is_empty() {
                app.config.editor.new_note_folder.clone()
            } else {
                folder
            };
            let prefix = if base.is_empty() {
                String::new()
            } else {
                format!("{}/", base.trim_matches('/'))
            };
            app.modal = Some(Modal::Prompt(Prompt::new(
                "New note",
                prefix,
                PromptIntent::NewNote,
            )));
        }
        Action::NewFolder => {
            app.modal = Some(Modal::Prompt(Prompt::new(
                "New folder",
                String::new(),
                PromptIntent::NewFolder,
            )));
        }
        Action::InsertWikiLink => {
            // With text selected this wraps it, which is how you turn a phrase
            // you just typed into a link without retyping it.
            if let Some(editor) = app.editor_mut() {
                match editor.selected_text() {
                    Some(text) => {
                        editor.delete_selection();
                        editor.insert_str(&format!("[[{text}]]"));
                    }
                    None => {
                        editor.insert_str("[[]]");
                        editor.move_left(false);
                        editor.move_left(false);
                    }
                }
            } else {
                app.error("open a note in editing mode first");
            }
        }
        Action::DailyNote => {
            let name = app.daily_note_name();
            app.open_or_create(&name);
        }
        Action::Save => {
            if let Some(index) = app.active_tab {
                app.save_tab(index);
            }
        }
        Action::SaveAll => {
            for index in 0..app.tabs.len() {
                if app.tabs[index].is_modified() {
                    app.save_tab(index);
                }
            }
            app.info("saved all");
        }
        Action::RenameNote => match app.active_note() {
            Some(id) => {
                let stem = app
                    .index
                    .note(id)
                    .map(|n| n.meta.stem.clone())
                    .unwrap_or_default();
                app.modal = Some(Modal::Prompt(Prompt::new(
                    "Rename note",
                    stem,
                    PromptIntent::RenameNote,
                )));
            }
            None => app.error("no note open"),
        },
        Action::DeleteNote => match app.active_note() {
            Some(id) => {
                let title = app.note_title(id);
                app.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Move \"{title}\" to the vault trash?"),
                    // Confirmed deletion runs through the same action, with the
                    // modal already dismissed.
                    action: Action::OpenNote(id),
                }));
                // Encode the real intent separately from the picker action.
                if let Some(Modal::Confirm(confirm)) = app.modal.as_mut() {
                    confirm.action = Action::DeleteNote;
                }
            }
            None => app.error("no note open"),
        },

        // ---- tabs -------------------------------------------------------
        Action::CloseTab => app.close_tab(),
        Action::NextTab => app.cycle_tab(1),
        Action::PreviousTab => app.cycle_tab(-1),

        // ---- views ------------------------------------------------------
        Action::ToggleMode => {
            // Switching out of the editor saves, so the reading view never
            // shows something different from what's on disk.
            let should_save = app
                .active()
                .is_some_and(|t| t.mode == Mode::Editing && t.is_modified());
            if should_save
                && app.config.editor.auto_save
                && let Some(index) = app.active_tab
            {
                app.save_tab(index);
            }
            if let Some(tab) = app.active_mut() {
                tab.mode = match tab.mode {
                    Mode::Reading => Mode::Editing,
                    Mode::Editing => Mode::Reading,
                };
            }
            app.focus = Focus::Note;
        }
        Action::ToggleLeftSidebar => {
            app.config.ui.show_left_sidebar = !app.config.ui.show_left_sidebar;
            if !app.config.ui.show_left_sidebar && app.focus == Focus::Explorer {
                app.focus = Focus::Note;
            }
        }
        Action::ToggleRightSidebar => {
            app.config.ui.show_right_sidebar = !app.config.ui.show_right_sidebar;
            if !app.config.ui.show_right_sidebar && app.focus == Focus::Sidebar {
                app.focus = Focus::Note;
            }
        }
        Action::ToggleChat => {
            app.config.ui.show_chat = !app.config.ui.show_chat;
            app.focus = if app.config.ui.show_chat {
                Focus::Chat
            } else {
                Focus::Note
            };
        }
        Action::ToggleRibbon => app.config.ui.show_ribbon = !app.config.ui.show_ribbon,
        Action::ToggleHints => app.config.ui.show_hints = !app.config.ui.show_hints,
        Action::CycleSortOrder => cycle_sort_order(app),
        Action::CycleSidePanel => {
            app.side_panel = app.side_panel.next();
            app.side_selected = 0;
        }
        Action::OpenGraph => app.open_graph(None),
        Action::OpenLocalGraph => {
            let focus = app.active_note();
            if focus.is_none() {
                app.info("open a note first for a local graph");
            }
            app.open_graph(focus);
        }
        Action::OpenNotesView => {
            app.view = View::Notes;
            app.focus = if app.tabs.is_empty() {
                Focus::Explorer
            } else {
                Focus::Note
            };
        }

        // ---- modals -----------------------------------------------------
        Action::OpenPalette => {
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Commands, commands())));
        }
        Action::OpenSwitcher => {
            let entries = app
                .index
                .notes()
                .iter()
                .enumerate()
                .map(|(id, note)| {
                    Entry::new(
                        note.meta.title.clone(),
                        note.meta.rel.clone(),
                        Action::OpenNote(id),
                    )
                })
                .collect();
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Notes, entries)));
        }
        Action::OpenSearch => {
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Search, Vec::new())));
        }
        Action::OpenThemePicker => {
            let entries = app
                .themes
                .iter()
                .map(|theme| {
                    Entry::new(
                        theme.name.clone(),
                        if theme.dark { "dark" } else { "light" },
                        Action::SetTheme(theme.name.clone()),
                    )
                })
                .collect();
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Themes, entries)));
        }
        Action::OpenVaultPicker => {
            let mut entries: Vec<Entry> = otui_core::vault::discover()
                .into_iter()
                .map(|vault| {
                    Entry::new(
                        vault.name.clone(),
                        vault.path.display().to_string(),
                        Action::OpenVault(vault.path),
                    )
                })
                .collect();
            if entries.is_empty() {
                entries.push(Entry::new(
                    "No vaults registered with Obsidian",
                    "pass a folder path on the command line instead",
                    Action::OpenNotesView,
                ));
            }
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Vaults, entries)));
        }
        Action::OpenHelp => app.modal = Some(Modal::Help(0)),
        Action::OpenProviderPicker => {
            let current = app.config.agent.provider.clone();
            let entries = otui_agent::catalog::PRESETS
                .iter()
                .map(|preset| {
                    // The key's whereabouts is the thing people are actually
                    // looking for here, so it goes in the detail line.
                    let detail = match (preset.id == current, preset.env_var) {
                        (true, _) => format!("in use · {}", preset.note),
                        (false, Some(_))
                            if crate::auth::key_for(preset.id, &app.auth).is_some() =>
                        {
                            format!("key ready · {}", preset.note)
                        }
                        (false, _) => preset.note.to_string(),
                    };
                    Entry::new(preset.label, detail, Action::SetProvider(preset.id.into()))
                })
                .collect();
            app.modal = Some(Modal::Picker(Picker::new(PickerKind::Providers, entries)));
        }
        Action::OpenModelPicker => ask_for_models(app),
        Action::PromptApiKey => {
            let provider = app.config.agent.provider.clone();
            match otui_agent::catalog::find(&provider) {
                Some(preset) if preset.env_var.is_none() => app.info(format!(
                    "{} needs no API key; /base-url points at it instead",
                    preset.label
                )),
                Some(preset) => {
                    app.modal = Some(Modal::Prompt(crate::modal::Prompt::new(
                        format!("{} API key", preset.label),
                        "",
                        crate::modal::PromptIntent::ApiKey(provider),
                    )));
                }
                None => app.error(format!("unknown provider '{provider}'")),
            }
        }

        // ---- settings ---------------------------------------------------
        Action::SetTheme(name) => app.set_theme(&name),
        Action::SetProvider(name) => match set_provider(app, &name) {
            Ok(message) => app.info(message),
            Err(message) => app.error(message),
        },
        Action::SetModel(name) => {
            app.config.agent.model = name;
            app.info(format!("model: {}", app.config.agent.model()));
        }
        Action::OpenVault(path) => app.open_vault(path),
        Action::ToggleLineNumbers => {
            app.config.ui.line_numbers = !app.config.ui.line_numbers;
        }
        Action::ToggleGraphLabels => {
            app.config.graph.show_labels = !app.config.graph.show_labels;
        }
        Action::ToggleGraphUnresolved => {
            app.config.graph.show_unresolved = !app.config.graph.show_unresolved;
            app.refresh_graph();
        }
        Action::ToggleGraphTags => {
            app.config.graph.show_tags = !app.config.graph.show_tags;
            app.refresh_graph();
        }
        Action::ToggleGraphAttachments => {
            app.config.graph.show_attachments = !app.config.graph.show_attachments;
            app.refresh_graph();
        }
        Action::ToggleGraphOrphans => {
            app.config.graph.show_orphans = !app.config.graph.show_orphans;
            app.refresh_graph();
        }

        // ---- vault ------------------------------------------------------
        Action::OpenInObsidian => open_in_obsidian(app),
        Action::Refresh => {
            app.refresh();
            app.info("reloaded from disk");
        }
        Action::SaveSettings => match app.config.save() {
            Ok(path) => app.info(format!("settings saved to {}", path.display())),
            Err(err) => app.error(format!("could not save settings: {err}")),
        },
        Action::Quit => {
            // Quitting is one keystroke away from every pane, so it asks first
            // — and says what's at stake when there's unsaved work.
            let unsaved = app.tabs.iter().filter(|t| t.is_modified()).count();
            let message = match unsaved {
                0 => "Quit emeraldian?".to_string(),
                1 => "Quit emeraldian? 1 note has unsaved changes.".to_string(),
                n => format!("Quit emeraldian? {n} notes have unsaved changes."),
            };
            app.modal = Some(Modal::Confirm(Confirm {
                message,
                action: Action::ForceQuit,
            }));
        }
        Action::ForceQuit => {
            // Unsaved work is written out rather than silently dropped.
            if app.config.editor.auto_save {
                for index in 0..app.tabs.len() {
                    if app.tabs[index].is_modified() {
                        app.save_tab(index);
                    }
                }
            }
            app.quit = true;
        }
    }
}

/// Runs a confirmed destructive action.
pub fn confirm(app: &mut App, action: Action) {
    app.modal = None;
    match action {
        Action::DeleteNote => {
            let Some(id) = app.active_note() else { return };
            let title = app.note_title(id);
            match app.index.delete_note(id) {
                Ok(_) => {
                    app.tabs.retain(|t| t.note != id);
                    app.active_tab = if app.tabs.is_empty() { None } else { Some(0) };
                    app.refresh();
                    app.info(format!("moved \"{title}\" to trash"));
                }
                Err(err) => app.error(format!("delete failed: {err}")),
            }
        }
        other => dispatch(app, other),
    }
}

/// Applies a prompt's answer.
pub fn submit_prompt(app: &mut App, prompt: Prompt) {
    let value = prompt.value.trim().to_string();
    app.modal = None;

    match prompt.intent {
        PromptIntent::NewNote => {
            if value.is_empty() {
                return;
            }
            let content = format!("# {}\n\n", value.rsplit('/').next().unwrap_or(&value));
            match app.index.create_note(&value, &content) {
                Ok(id) => {
                    app.explorer.rebuild(&app.index);
                    app.open_note(id);
                    if let Some(tab) = app.active_mut() {
                        tab.mode = Mode::Editing;
                    }
                    app.info(format!("created {value}"));
                }
                Err(err) => app.error(format!("could not create: {err}")),
            }
        }
        PromptIntent::NewFolder => {
            if value.is_empty() {
                return;
            }
            match app.index.create_folder(&value) {
                Ok(rel) => {
                    app.explorer.rebuild(&app.index);
                    app.info(format!("created folder {rel}"));
                }
                Err(err) => app.error(format!("could not create folder: {err}")),
            }
        }
        PromptIntent::RenameNote => {
            let Some(id) = app.active_note() else { return };
            if value.is_empty() {
                return;
            }
            match app.index.rename_note(id, &value) {
                Ok(_) => {
                    app.refresh();
                    app.explorer.rebuild(&app.index);
                    app.info(format!("renamed to {value}"));
                }
                Err(err) => app.error(format!("rename failed: {err}")),
            }
        }
        PromptIntent::FilterExplorer => {
            app.explorer.filter = value;
            app.explorer.rebuild(&app.index);
        }
        PromptIntent::ApiKey(provider) => store_key(app, &provider, &value),
    }
}

/// Keeps a typed-in API key, or forgets it when the prompt was left empty.
fn store_key(app: &mut App, provider: &str, key: &str) {
    let label = otui_agent::catalog::find(provider).map_or(provider, |preset| preset.label);
    app.auth.set(provider, key);

    if let Err(err) = app.auth.save() {
        app.error(format!("could not save the key: {err}"));
        return;
    }
    if key.trim().is_empty() {
        app.info(format!("forgot the {label} key"));
        return;
    }

    // Worth saying, because a key in the environment silently wins over the one
    // just typed, and the resulting "but I entered it" is hard to debug.
    match crate::auth::source(provider, &app.auth) {
        crate::auth::Source::Env(name) => app.info(format!(
            "saved, but ${name} is set and takes precedence — unset it to use this key"
        )),
        _ if !app.auth.persists() => app.info(format!(
            "{label} key set for this session; there is no config directory to keep it in"
        )),
        _ => app.info(format!("{label} key saved · /model to choose one")),
    }
}

/// Recomputes a live search picker's entries.
pub fn update_search(app: &mut App) {
    let Some(Modal::Picker(picker)) = app.modal.as_ref() else {
        return;
    };
    if picker.kind != PickerKind::Search {
        return;
    }
    let query = picker.query.trim().to_string();
    if query.len() < 2 {
        if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
            picker.set_entries(Vec::new());
        }
        return;
    }

    let results = search::search_content(&app.index, &query, Default::default());
    let entries: Vec<Entry> = results
        .iter()
        .filter_map(|result| {
            let note = app.index.note(result.id)?;
            let first = result.hits.first()?;
            Some(Entry::new(
                note.meta.title.clone(),
                format!("{}  ·  {}", note.meta.rel, first.text),
                Action::OpenNote(result.id),
            ))
        })
        .collect();

    if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
        picker.set_entries(entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("actions");
        vault.write("A.md", "# A\n\n[[B]] and [[Ghost]]\n");
        vault.write("B.md", "# B\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");
        (vault, app)
    }

    #[test]
    fn every_command_has_a_label() {
        for entry in commands() {
            assert!(!entry.label.is_empty());
        }
    }

    #[test]
    fn the_palette_lists_every_command() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenPalette);

        match app.modal.as_ref() {
            Some(Modal::Picker(picker)) => {
                assert_eq!(picker.kind, PickerKind::Commands);
                assert_eq!(picker.len(), commands().len());
            }
            other => panic!("expected a picker, got {other:?}"),
        }
    }

    #[test]
    fn dispatching_dismisses_an_open_overlay() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenPalette);
        assert!(app.modal.is_some());

        dispatch(&mut app, Action::OpenGraph);
        assert!(app.modal.is_none());
    }

    #[test]
    fn toggling_a_pane_moves_focus_out_of_it() {
        let (_v, mut app) = app();
        app.focus = Focus::Explorer;

        dispatch(&mut app, Action::ToggleLeftSidebar);
        assert!(!app.config.ui.show_left_sidebar);
        assert_eq!(app.focus, Focus::Note, "focus can't stay in a hidden pane");
    }

    #[test]
    fn toggling_the_chat_panel_focuses_it() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::ToggleChat);
        assert!(app.config.ui.show_chat);
        assert_eq!(app.focus, Focus::Chat);

        dispatch(&mut app, Action::ToggleChat);
        assert_eq!(app.focus, Focus::Note);
    }

    #[test]
    fn switching_out_of_the_editor_saves() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        dispatch(&mut app, Action::ToggleMode);
        app.editor_mut().unwrap().insert_str("edited ");

        dispatch(&mut app, Action::ToggleMode);
        assert!(vault.read("B.md").contains("edited"));
    }

    #[test]
    fn back_returns_to_the_previous_note_only_once() {
        let (_v, mut app) = app();
        let a = app.index.id_of_rel("A.md").unwrap();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(a);
        app.open_note(b);

        dispatch(&mut app, Action::Back);
        assert_eq!(app.active_note(), Some(a));

        dispatch(&mut app, Action::Back);
        assert!(app.status.text.contains("no earlier note"));
    }

    #[test]
    fn creating_a_note_from_a_prompt_opens_it_for_editing() {
        let (vault, mut app) = app();
        submit_prompt(
            &mut app,
            Prompt::new("New note", "Projects/Fresh", PromptIntent::NewNote),
        );

        assert!(vault.exists("Projects/Fresh.md"));
        assert_eq!(app.active().map(|t| t.mode), Some(Mode::Editing));
        assert!(vault.read("Projects/Fresh.md").starts_with("# Fresh"));
    }

    #[test]
    fn an_empty_prompt_creates_nothing() {
        let (_v, mut app) = app();
        let before = app.index.len();
        submit_prompt(
            &mut app,
            Prompt::new("New note", "   ", PromptIntent::NewNote),
        );
        assert_eq!(app.index.len(), before);
    }

    #[test]
    fn renaming_through_a_prompt_updates_links() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        submit_prompt(
            &mut app,
            Prompt::new("Rename", "Bravo", PromptIntent::RenameNote),
        );

        assert!(vault.exists("Bravo.md"));
        assert!(vault.read("A.md").contains("[[Bravo]]"));
    }

    #[test]
    fn deleting_asks_before_it_acts() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        dispatch(&mut app, Action::DeleteNote);
        assert!(
            matches!(app.modal, Some(Modal::Confirm(_))),
            "a destructive action must be confirmed"
        );
        assert!(vault.exists("B.md"), "nothing happened yet");

        confirm(&mut app, Action::DeleteNote);
        assert!(!vault.exists("B.md"));
    }

    #[test]
    fn quitting_asks_first_and_names_the_unsaved_work() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        app.editor_mut().unwrap().insert_str("unsaved ");

        dispatch(&mut app, Action::Quit);
        assert!(!app.quit, "the prompt has to be answered first");
        let Some(Modal::Confirm(confirm)) = app.modal.as_ref() else {
            panic!("expected a confirmation, got {:?}", app.modal);
        };
        assert!(confirm.message.contains("1 note"), "{}", confirm.message);

        confirm_modal(&mut app);
        assert!(app.quit);
        assert!(vault.read("B.md").contains("unsaved"));
    }

    /// Answers whatever confirmation is on screen with a yes.
    fn confirm_modal(app: &mut App) {
        let Some(Modal::Confirm(c)) = app.modal.as_ref() else {
            panic!("no confirmation on screen");
        };
        let action = c.action.clone();
        confirm(app, action);
    }

    #[test]
    fn search_updates_a_live_picker() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenSearch);
        if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
            for ch in "# B".chars() {
                picker.insert(ch);
            }
        }
        update_search(&mut app);

        match app.modal.as_ref() {
            Some(Modal::Picker(picker)) => assert_eq!(picker.len(), 1),
            other => panic!("expected picker, got {other:?}"),
        }
    }

    #[test]
    fn a_one_character_search_query_returns_nothing() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenSearch);
        if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
            picker.insert('a');
        }
        update_search(&mut app);

        match app.modal.as_ref() {
            Some(Modal::Picker(picker)) => assert!(picker.is_empty()),
            other => panic!("expected picker, got {other:?}"),
        }
    }

    #[test]
    fn graph_filters_rebuild_the_graph() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenGraph);
        let before = app.graph.as_ref().unwrap().simulation.graph.nodes.len();

        dispatch(&mut app, Action::ToggleGraphUnresolved);
        let after = app.graph.as_ref().unwrap().simulation.graph.nodes.len();
        assert_ne!(before, after, "hiding unresolved notes changes the graph");
    }
}
