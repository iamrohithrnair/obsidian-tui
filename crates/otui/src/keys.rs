//! Keyboard handling.
//!
//! Bindings follow Obsidian where it has one (`Ctrl+O`, `Ctrl+P`, `Ctrl+E`,
//! `Ctrl+N`) and vim where Obsidian has nothing to copy (`hjkl` to move,
//! `g`/`G` for top and bottom). Global bindings are checked first, then the
//! focused pane gets what's left, so `Ctrl+P` opens the palette even mid-word.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::actions::{self, dispatch};
use crate::agent;
use crate::app::{Action, App, Focus, Mode, View};
use crate::modal::{Modal, PickerKind, Prompt, PromptIntent};
use crate::ui::panes::{SidebarTarget, sidebar_targets};

/// Columns moved per keypress when panning a wide table sideways.
const PAN_STEP: isize = 4;

/// Handles one key event.
pub fn handle(app: &mut App, key: KeyEvent) {
    // Terminals that report key releases would otherwise run every binding
    // twice.
    if key.kind == KeyEventKind::Release {
        return;
    }

    // A message from the last action is stale as soon as the user acts again.
    app.status.text.clear();

    if app.modal.is_some() {
        handle_modal(app, key);
        return;
    }

    if handle_global(app, key) {
        return;
    }

    match app.focus {
        Focus::Explorer => handle_explorer(app, key),
        Focus::Note => handle_note(app, key),
        Focus::Sidebar => handle_sidebar(app, key),
        Focus::Chat => handle_chat(app, key),
        Focus::Graph => handle_graph(app, key),
    }
}

/// Rewrites the control bytes a legacy terminal sends into the keys they mean.
///
/// Without the Kitty keyboard protocol a terminal has no way to say "Ctrl and
/// this punctuation key"; it sends a single C0 byte instead. `Ctrl+\` is `0x1C`
/// and `Ctrl+]` is `0x1D`, and crossterm decodes that range as `Ctrl` plus the
/// digits `4`–`7` — so a binding written as `Ctrl+]` can never match, which is
/// exactly what left both sidebar toggles dead.
///
/// Nothing binds `Ctrl` with a digit, and modals are dispatched before the
/// global map, so no typed input passes through here.
fn normalize_legacy_ctrl(key: KeyEvent) -> KeyEvent {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return key;
    }
    let code = match key.code {
        KeyCode::Char('4') => KeyCode::Char('\\'),
        KeyCode::Char('5') => KeyCode::Char(']'),
        other => other,
    };
    KeyEvent { code, ..key }
}

/// Bindings that work everywhere. Returns whether the key was consumed.
fn handle_global(app: &mut App, key: KeyEvent) -> bool {
    let key = normalize_legacy_ctrl(key);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let action = match (ctrl, shift, key.code) {
        (true, _, KeyCode::Char('p' | 'P')) => Some(Action::OpenPalette),
        (true, _, KeyCode::Char('o' | 'O')) => Some(Action::OpenSwitcher),
        (true, true, KeyCode::Char('f' | 'F')) => Some(Action::OpenSearch),
        (true, _, KeyCode::Char('n' | 'N')) => Some(Action::NewNote),
        (true, _, KeyCode::Char('s' | 'S')) => Some(Action::Save),
        (true, _, KeyCode::Char('d' | 'D')) => Some(Action::DailyNote),
        (true, _, KeyCode::Char('e' | 'E')) => Some(Action::ToggleMode),
        (true, _, KeyCode::Char('w' | 'W')) => Some(Action::CloseTab),
        (true, _, KeyCode::Char('t' | 'T')) => Some(Action::OpenThemePicker),
        (true, _, KeyCode::Char('l' | 'L')) => Some(Action::ToggleChat),
        (true, _, KeyCode::Char('k' | 'K')) => Some(Action::CycleSidePanel),
        (true, _, KeyCode::Char('q' | 'Q')) => Some(Action::Quit),
        (true, true, KeyCode::Char('g' | 'G')) => Some(Action::OpenLocalGraph),
        (true, false, KeyCode::Char('g' | 'G')) => Some(Action::OpenGraph),
        (true, _, KeyCode::Char('\\')) => Some(Action::ToggleLeftSidebar),
        (true, _, KeyCode::Char(']')) => Some(Action::ToggleRightSidebar),
        (true, _, KeyCode::Char('r' | 'R')) if app.focus != Focus::Chat => Some(Action::Refresh),
        (true, true, KeyCode::Tab | KeyCode::BackTab) => Some(Action::PreviousTab),
        (true, false, KeyCode::Tab) => Some(Action::NextTab),
        (_, _, KeyCode::F(2)) => Some(Action::RenameNote),
        _ => None,
    };

    if let Some(action) = action {
        dispatch(app, action);
        return true;
    }

    if alt && key.code == KeyCode::Left {
        dispatch(app, Action::Back);
        return true;
    }

    // Pane cycling skips panes that aren't on screen. The note editor and the
    // graph both bind Tab themselves — indent and next-node — so they keep it;
    // Ctrl+Tab still switches document tabs from anywhere. The chat claims Tab
    // only while a slash command is being typed, where completing it is what
    // the key obviously means.
    let completing = app.focus == Focus::Chat && crate::slash::is_command(&app.chat.input);
    let owns_tab = matches!(app.focus, Focus::Note | Focus::Graph) || completing;
    if key.code == KeyCode::Tab && !ctrl && !owns_tab {
        cycle_focus(app, 1);
        return true;
    }
    if key.code == KeyCode::BackTab && !owns_tab {
        cycle_focus(app, -1);
        return true;
    }

    false
}

fn cycle_focus(app: &mut App, delta: isize) {
    let mut order = vec![Focus::Explorer];
    order.push(if app.view == View::Graph {
        Focus::Graph
    } else {
        Focus::Note
    });
    if app.config.ui.show_right_sidebar {
        order.push(Focus::Sidebar);
    }
    if app.config.ui.show_chat {
        order.push(Focus::Chat);
    }
    if !app.config.ui.show_left_sidebar {
        order.retain(|f| *f != Focus::Explorer);
    }
    if order.is_empty() {
        return;
    }

    let current = order.iter().position(|f| *f == app.focus).unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(order.len() as isize) as usize;
    app.focus = order[next];
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn handle_modal(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match app.modal.as_mut() {
        Some(Modal::Picker(picker)) => match key.code {
            KeyCode::Esc => app.modal = None,
            KeyCode::Enter => {
                let action = picker.selected_entry().map(|e| e.action.clone());
                if let Some(action) = action {
                    dispatch(app, action);
                } else {
                    app.modal = None;
                }
            }
            KeyCode::Up => picker.move_selection(-1),
            KeyCode::Down => picker.move_selection(1),
            KeyCode::Char('p') if ctrl => picker.move_selection(-1),
            KeyCode::Char('n') if ctrl => picker.move_selection(1),
            KeyCode::PageUp => picker.move_selection(-10),
            KeyCode::PageDown => picker.move_selection(10),
            KeyCode::Backspace => {
                picker.backspace();
                if picker.kind == PickerKind::Search {
                    actions::update_search(app);
                }
            }
            KeyCode::Char('u') if ctrl => picker.clear(),
            KeyCode::Char(ch) => {
                picker.insert(ch);
                if picker.kind == PickerKind::Search {
                    actions::update_search(app);
                }
            }
            _ => {}
        },

        Some(Modal::Prompt(prompt)) => match key.code {
            KeyCode::Esc => app.modal = None,
            KeyCode::Enter => {
                let prompt = prompt.clone();
                actions::submit_prompt(app, prompt);
            }
            KeyCode::Backspace => prompt.backspace(),
            KeyCode::Left => prompt.cursor = prompt.cursor.saturating_sub(1),
            KeyCode::Right => {
                prompt.cursor = (prompt.cursor + 1).min(prompt.value.chars().count());
            }
            KeyCode::Char(ch) => prompt.insert(ch),
            _ => {}
        },

        Some(Modal::Confirm(confirm)) => match key.code {
            // An explicit yes proceeds; anything else cancels, so a stray
            // keypress can't delete a note. Enter agrees because the dialog is
            // asking a question the user just triggered on purpose.
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                let action = confirm.action.clone();
                actions::confirm(app, action);
            }
            _ => app.modal = None,
        },

        Some(Modal::Help(scroll)) => match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => app.modal = None,
            KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
            KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
            KeyCode::PageDown => *scroll += 10,
            KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
            _ => {}
        },

        None => {}
    }
}

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

fn handle_explorer(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.explorer.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.explorer.select_previous(),
        KeyCode::Char('g') | KeyCode::Home => app.explorer.select_first(),
        KeyCode::Char('G') | KeyCode::End => app.explorer.select_last(),
        KeyCode::Char('H') => app.explorer.collapse_all(&app.index),
        KeyCode::Char('L') => app.explorer.expand_all(&app.index),
        KeyCode::PageDown => app.explorer.page(10),
        KeyCode::PageUp => app.explorer.page(-10),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            if let Some(id) = app.explorer.selected_note() {
                app.open_note(id);
            } else {
                app.explorer.toggle(&app.index);
            }
        }
        KeyCode::Char(' ') => {
            app.explorer.toggle(&app.index);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.explorer.toggle(&app.index);
        }
        KeyCode::Char('/') => {
            app.modal = Some(Modal::Prompt(Prompt::new(
                "Filter files",
                app.explorer.filter.clone(),
                PromptIntent::FilterExplorer,
            )));
        }
        KeyCode::Esc => {
            if !app.explorer.filter.is_empty() {
                app.explorer.filter.clear();
                app.explorer.rebuild(&app.index);
            }
        }
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
        KeyCode::Char('q') => dispatch(app, Action::Quit),
        KeyCode::Char('s') => dispatch(app, Action::CycleSortOrder),
        _ => {}
    }
}

fn handle_note(app: &mut App, key: KeyEvent) {
    let Some(mode) = app.active().map(|t| t.mode) else {
        // With no note open, the pane behaves like the empty state's hints.
        match key.code {
            KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
            KeyCode::Char('q') => dispatch(app, Action::Quit),
            _ => {}
        }
        return;
    };

    match mode {
        Mode::Reading => handle_reading(app, key),
        Mode::Editing => handle_editing(app, key),
    }
}

fn handle_reading(app: &mut App, key: KeyEvent) {
    let step = |app: &mut App, delta: isize| {
        if let Some(tab) = app.active_mut() {
            tab.scroll = (tab.scroll as isize + delta).max(0) as usize;
        }
    };
    // Panning across a wide table. The draw pass clamps this to the content's
    // width, which is the only place the width is known.
    let pan = |app: &mut App, delta: isize| {
        if let Some(tab) = app.active_mut() {
            tab.hscroll = (tab.hscroll as isize + delta).max(0) as usize;
        }
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => step(app, 1),
        KeyCode::Char('k') | KeyCode::Up => step(app, -1),
        KeyCode::Char('l') | KeyCode::Right => pan(app, PAN_STEP),
        KeyCode::Char('h') | KeyCode::Left => pan(app, -PAN_STEP),
        KeyCode::PageDown | KeyCode::Char(' ') => step(app, 15),
        KeyCode::PageUp => step(app, -15),
        KeyCode::Char('g') | KeyCode::Home => {
            if let Some(tab) = app.active_mut() {
                tab.scroll = 0;
                tab.hscroll = 0;
            }
        }
        KeyCode::Char('G') | KeyCode::End => step(app, 100_000),
        // Following the first link on the page is the reading-mode equivalent
        // of clicking one.
        KeyCode::Enter => follow_first_link(app),
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
        KeyCode::Char('q') => dispatch(app, Action::Quit),
        _ => {}
    }
}

fn follow_first_link(app: &mut App) {
    let Some(id) = app.active_note() else { return };
    let target = app.index.note(id).and_then(|note| {
        note.links.iter().find_map(|link| match &link.target {
            otui_core::index::LinkTarget::Note(target) => {
                app.index.note(*target).map(|n| n.meta.rel.clone())
            }
            otui_core::index::LinkTarget::Unresolved(name) => Some(name.clone()),
            _ => None,
        })
    });

    match target {
        Some(target) => dispatch(app, Action::FollowLink(target)),
        None => app.info("no links in this note"),
    }
}

fn handle_editing(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // Formatting shortcuts are checked before the editor sees the key.
    if ctrl {
        match key.code {
            KeyCode::Char('b' | 'B') => {
                if let Some(editor) = app.editor_mut() {
                    editor.toggle_wrap("**");
                }
                return;
            }
            KeyCode::Char('i' | 'I') => {
                if let Some(editor) = app.editor_mut() {
                    editor.toggle_wrap("*");
                }
                return;
            }
            KeyCode::Char('z' | 'Z') => {
                if let Some(editor) = app.editor_mut() {
                    editor.undo();
                }
                return;
            }
            KeyCode::Char('y' | 'Y') => {
                if let Some(editor) = app.editor_mut() {
                    editor.redo();
                }
                return;
            }
            KeyCode::Char('a' | 'A') => {
                if let Some(editor) = app.editor_mut() {
                    editor.select_all();
                }
                return;
            }
            // Emacs-style set-mark, for selecting without holding shift.
            KeyCode::Char(' ') => {
                if let Some(editor) = app.editor_mut() {
                    editor.begin_selection();
                }
                return;
            }
            KeyCode::Char('k' | 'K') if shift => {
                if let Some(editor) = app.editor_mut() {
                    editor.delete_line();
                }
                return;
            }
            _ => {}
        }
    }

    let Some(editor) = app.editor_mut() else {
        return;
    };

    match key.code {
        KeyCode::Char(ch) if !ctrl => editor.insert_char(ch),
        KeyCode::Enter => editor.newline(),
        KeyCode::Tab => editor.insert_char('\t'),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete => editor.delete_forward(),
        KeyCode::Left if ctrl => editor.move_word_left(shift),
        KeyCode::Right if ctrl => editor.move_word_right(shift),
        KeyCode::Left => editor.move_left(shift),
        KeyCode::Right => editor.move_right(shift),
        KeyCode::Up => editor.move_up(shift),
        KeyCode::Down => editor.move_down(shift),
        KeyCode::Home if ctrl => editor.move_document_start(shift),
        KeyCode::End if ctrl => editor.move_document_end(shift),
        KeyCode::Home => editor.move_line_start(shift),
        KeyCode::End => editor.move_line_end(shift),
        KeyCode::PageUp => editor.move_page(-15, shift),
        KeyCode::PageDown => editor.move_page(15, shift),
        KeyCode::Esc => {
            editor.commit();
            dispatch(app, Action::ToggleMode);
        }
        _ => {}
    }
}

fn handle_sidebar(app: &mut App, key: KeyEvent) {
    let targets = sidebar_targets(app);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.side_selected = (app.side_selected + 1).min(targets.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.side_selected = app.side_selected.saturating_sub(1);
        }
        KeyCode::Char('g') | KeyCode::Home => app.side_selected = 0,
        KeyCode::Char('G') | KeyCode::End => {
            app.side_selected = targets.len().saturating_sub(1);
        }
        KeyCode::Tab if key.modifiers.is_empty() => dispatch(app, Action::CycleSidePanel),
        KeyCode::Enter => match targets.get(app.side_selected).cloned() {
            Some(SidebarTarget::Heading(line)) => {
                // Jumping to a heading scrolls the reading view and moves the
                // editor's cursor, whichever mode is active.
                if let Some(tab) = app.active_mut() {
                    tab.scroll = line;
                }
                if let Some(editor) = app.editor_mut() {
                    editor.goto(line, 0);
                }
                app.focus = Focus::Note;
            }
            Some(SidebarTarget::Note(id)) => app.open_note(id),
            Some(SidebarTarget::Tag(tag)) => {
                // Selecting a tag searches for it, which is the useful action.
                dispatch(app, Action::OpenSearch);
                if let Some(Modal::Picker(picker)) = app.modal.as_mut() {
                    for ch in format!("#{tag}").chars() {
                        picker.insert(ch);
                    }
                }
                actions::update_search(app);
            }
            None => {}
        },
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
        KeyCode::Char('q') => dispatch(app, Action::Quit),
        _ => {}
    }
}

fn handle_chat(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // While a slash command is being typed, the arrow keys belong to the list of
    // commands rather than to the transcript — a menu on screen is what the keys
    // obviously act on, and it is the only way to read the list without knowing
    // the names already.
    let completions = if app.chat.busy {
        Vec::new()
    } else {
        crate::slash::completions(&app.chat.input)
    };
    if !completions.is_empty() && handle_completions(app, key, &completions) {
        return;
    }

    match key.code {
        KeyCode::Enter if !app.chat.busy && crate::slash::is_command(&app.chat.input) => {
            run_command(app);
        }
        KeyCode::Enter if !app.chat.busy => agent::send(app),
        KeyCode::Char('c' | 'C') if ctrl => {
            app.chat.cancel();
            app.info("stopping…");
        }
        KeyCode::Char('r' | 'R') if ctrl => {
            app.chat.reset();
            app.info("conversation cleared");
        }
        KeyCode::Char(ch) if !ctrl => app.chat.insert_char(ch),
        KeyCode::Backspace => app.chat.backspace(),
        KeyCode::Left => app.chat.move_cursor(-1),
        KeyCode::Right => app.chat.move_cursor(1),
        KeyCode::Up | KeyCode::PageUp => {
            // Scrolling up detaches from the bottom so streaming output doesn't
            // yank the view back.
            app.chat.follow = false;
            app.chat.scroll = app.chat.scroll.saturating_sub(3);
        }
        KeyCode::Down | KeyCode::PageDown => {
            app.chat.scroll += 3;
            app.chat.follow = true;
        }
        KeyCode::Esc => app.focus = Focus::Note,
        _ => {}
    }
}

/// Keys that belong to the slash-command list. Returns whether one was used.
fn handle_completions(
    app: &mut App,
    key: KeyEvent,
    completions: &[&'static crate::slash::SlashCommand],
) -> bool {
    let selected = || {
        completions
            .get(app.chat.completion)
            .or(completions.first())
            .map(|command| command.name)
    };

    match key.code {
        // Only when there is something to choose between: with the command
        // already named, the arrows still belong to the transcript.
        KeyCode::Up if completions.len() > 1 => app.chat.move_completion(-1, completions.len()),
        KeyCode::Down if completions.len() > 1 => app.chat.move_completion(1, completions.len()),
        // Tab fills in the highlighted command, the way a shell would.
        KeyCode::Tab => {
            if let Some(name) = selected() {
                app.chat.complete_with(name);
            }
        }
        // Enter takes the highlight when the name is still being typed, and runs
        // the command once it is settled. So `/` then arrows then Enter fills it
        // in, and Enter again runs it — while typing `/help` and pressing Enter
        // runs it outright, without a detour through the menu.
        KeyCode::Enter => match selected().filter(|name| !settled(&app.chat.input, name)) {
            Some(name) => app.chat.complete_with(name),
            None => run_command(app),
        },
        // Abandoning what you were typing is what Escape means here; the list
        // closes with it, and a second Escape leaves the panel.
        KeyCode::Esc => app.chat.clear_input(),
        _ => return false,
    }
    true
}

/// Whether the input already names this command, rather than a prefix of it.
fn settled(input: &str, name: &str) -> bool {
    crate::slash::parse(input).is_some_and(|(typed, _)| typed.eq_ignore_ascii_case(name))
}

fn run_command(app: &mut App) {
    let input = app.chat.input.trim().to_string();
    app.chat.clear_input();
    if let crate::slash::Outcome::Unknown(name) = crate::slash::run(app, &input) {
        app.error(format!("unknown command '/{name}'; /help lists them"));
    }
}

fn handle_graph(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(graph) = app.graph.as_mut() else {
        return;
    };
    // Pan speed scales with zoom so it feels constant on screen.
    let step = 20.0 / graph.zoom;

    match key.code {
        KeyCode::Char('h') => graph.center.x -= step,
        KeyCode::Char('l') => graph.center.x += step,
        KeyCode::Char('k') => graph.center.y += step,
        KeyCode::Char('j') => graph.center.y -= step,
        // Arrows walk the selection from node to node, which is how you read a
        // graph; `hjkl` pans the camera. Binding both to panning left no way to
        // step through the picture except Tab, which jumps by link count and so
        // teleports across the vault.
        KeyCode::Left => graph.select_in_direction(-1.0, 0.0),
        KeyCode::Right => graph.select_in_direction(1.0, 0.0),
        KeyCode::Up => graph.select_in_direction(0.0, 1.0),
        KeyCode::Down => graph.select_in_direction(0.0, -1.0),
        KeyCode::Char('+' | '=') => graph.zoom_by(1.25),
        KeyCode::Char('-' | '_') => graph.zoom_by(1.0 / 1.25),
        // `f` to fit and `0` to reset are the two conventions graph and image
        // viewers use; both frame the whole layout.
        KeyCode::Char('f' | '0') => graph.fit(),
        KeyCode::Tab | KeyCode::Char('n') => graph.cycle_selection(1),
        KeyCode::BackTab | KeyCode::Char('N') => graph.cycle_selection(-1),
        // Recentre on the selection without changing zoom.
        KeyCode::Char('c') => {
            if let Some(selected) = graph.selected {
                graph.focus_node(selected);
            }
        }
        KeyCode::Enter => {
            let target = graph.selected.and_then(|index| {
                graph
                    .simulation
                    .graph
                    .nodes
                    .get(index)
                    .map(|node| (node.kind.clone(), node.label.clone()))
            });
            match target {
                Some((otui_core::graph::NodeKind::Note(id), _)) => {
                    dispatch(app, Action::OpenNote(id));
                }
                // Opening an unresolved node creates the note it stands for,
                // which is the whole point of showing them.
                Some((otui_core::graph::NodeKind::Unresolved, label)) => {
                    dispatch(app, Action::FollowLink(label));
                }
                _ => app.info("select a node first; Tab cycles through them"),
            }
        }
        KeyCode::Char('L') => dispatch(app, Action::ToggleGraphLabels),
        KeyCode::Char('u') => dispatch(app, Action::ToggleGraphUnresolved),
        KeyCode::Char('t') => dispatch(app, Action::ToggleGraphTags),
        KeyCode::Char('a') => dispatch(app, Action::ToggleGraphAttachments),
        KeyCode::Char('r') if !ctrl => {
            app.refresh_graph();
            app.info("graph rebuilt");
        }
        KeyCode::Esc => dispatch(app, Action::OpenNotesView),
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
        KeyCode::Char('q') => dispatch(app, Action::Quit),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("keys");
        vault.write("A.md", "# A\n\n[[B]]\n");
        vault.write("B.md", "# B\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");
        (vault, app)
    }

    #[test]
    fn key_releases_are_ignored() {
        let (_v, mut app) = app();
        let mut event = ctrl(KeyCode::Char('g'));
        event.kind = KeyEventKind::Release;

        handle(&mut app, event);
        assert_eq!(
            app.view,
            View::Notes,
            "a release must not re-run the binding"
        );
    }

    #[test]
    fn global_bindings_work_from_any_pane() {
        let (_v, mut app) = app();
        app.focus = Focus::Explorer;
        handle(&mut app, ctrl(KeyCode::Char('p')));
        assert!(matches!(app.modal, Some(Modal::Picker(_))));
    }

    #[test]
    fn typing_in_the_editor_does_not_trigger_commands() {
        let (_v, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        app.active_mut().unwrap().mode = Mode::Editing;

        handle(&mut app, key(KeyCode::Char('n')));
        handle(&mut app, key(KeyCode::Char('g')));

        assert!(app.modal.is_none(), "plain letters are text, not commands");
        assert_eq!(app.view, View::Notes);
        assert!(app.editor_mut().unwrap().text().contains("ng"));
    }

    #[test]
    fn explorer_navigation_and_opening() {
        let (_v, mut app) = app();
        app.focus = Focus::Explorer;

        handle(&mut app, key(KeyCode::Char('j')));
        handle(&mut app, key(KeyCode::Enter));

        assert!(app.active_note().is_some(), "Enter opens the selected note");
        assert_eq!(app.focus, Focus::Note);
    }

    #[test]
    fn escape_in_a_picker_closes_it() {
        let (_v, mut app) = app();
        handle(&mut app, ctrl(KeyCode::Char('o')));
        assert!(app.modal.is_some());

        handle(&mut app, key(KeyCode::Esc));
        assert!(app.modal.is_none());
    }

    #[test]
    fn the_quick_switcher_opens_the_chosen_note() {
        let (_v, mut app) = app();
        handle(&mut app, ctrl(KeyCode::Char('o')));
        for ch in "B".chars() {
            handle(&mut app, key(KeyCode::Char(ch)));
        }
        handle(&mut app, key(KeyCode::Enter));

        assert_eq!(app.note_title(app.active_note().expect("open")), "B");
        assert!(app.modal.is_none());
    }

    #[test]
    fn confirmation_requires_an_explicit_yes() {
        let (vault, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);

        dispatch(&mut app, Action::DeleteNote);
        handle(&mut app, key(KeyCode::Char('x')));
        assert!(vault.exists("B.md"), "anything but y cancels");
        assert!(app.modal.is_none());

        dispatch(&mut app, Action::DeleteNote);
        handle(&mut app, key(KeyCode::Char('y')));
        assert!(!vault.exists("B.md"));
    }

    #[test]
    fn reading_mode_scrolls_and_clamps_at_the_top() {
        let (_v, mut app) = app();
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);

        handle(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.active().unwrap().scroll, 1);

        handle(&mut app, key(KeyCode::Char('k')));
        handle(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.active().unwrap().scroll, 0, "must not go negative");
    }

    #[test]
    fn the_pane_toggles_respond_to_the_bytes_a_terminal_actually_sends() {
        let (_v, mut app) = app();

        // A terminal without the Kitty keyboard protocol sends 0x1C for Ctrl+\
        // and 0x1D for Ctrl+], which crossterm reports as Ctrl+4 and Ctrl+5.
        // Binding the punctuation alone left both toggles doing nothing.
        let right = app.config.ui.show_right_sidebar;
        handle(&mut app, ctrl(KeyCode::Char('5')));
        assert_ne!(
            app.config.ui.show_right_sidebar, right,
            "Ctrl+] must toggle the outline sidebar"
        );

        let left = app.config.ui.show_left_sidebar;
        handle(&mut app, ctrl(KeyCode::Char('4')));
        assert_ne!(
            app.config.ui.show_left_sidebar, left,
            "Ctrl+\\ must toggle the file explorer"
        );

        // The punctuation form still works, for terminals that do send it.
        handle(&mut app, ctrl(KeyCode::Char(']')));
        assert_eq!(app.config.ui.show_right_sidebar, right);
        handle(&mut app, ctrl(KeyCode::Char('\\')));
        assert_eq!(app.config.ui.show_left_sidebar, left);
    }

    #[test]
    fn a_plain_digit_is_not_mistaken_for_a_pane_toggle() {
        let (_v, mut app) = app();
        let before = (
            app.config.ui.show_left_sidebar,
            app.config.ui.show_right_sidebar,
        );
        handle(&mut app, key(KeyCode::Char('4')));
        handle(&mut app, key(KeyCode::Char('5')));
        assert_eq!(
            (
                app.config.ui.show_left_sidebar,
                app.config.ui.show_right_sidebar
            ),
            before,
            "only the control form is rewritten"
        );
    }

    #[test]
    fn reading_mode_pans_sideways_and_clamps_at_the_left() {
        let (_v, mut app) = app();
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);

        handle(&mut app, key(KeyCode::Right));
        assert_eq!(app.active().unwrap().hscroll, PAN_STEP as usize);

        handle(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.active().unwrap().hscroll, PAN_STEP as usize * 2);

        for _ in 0..5 {
            handle(&mut app, key(KeyCode::Left));
        }
        assert_eq!(app.active().unwrap().hscroll, 0, "must not go negative");

        // Jumping to the top comes back to the left edge too; leaving a note
        // scrolled sideways when you asked for the start of it is disorienting.
        handle(&mut app, key(KeyCode::Right));
        handle(&mut app, key(KeyCode::Char('g')));
        assert_eq!(app.active().unwrap().hscroll, 0);
    }

    #[test]
    fn enter_in_reading_mode_follows_a_link() {
        let (_v, mut app) = app();
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);

        handle(&mut app, key(KeyCode::Enter));
        assert_eq!(app.note_title(app.active_note().expect("open")), "B");
    }

    #[test]
    fn editing_shortcuts_wrap_the_selection() {
        let (_v, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        app.active_mut().unwrap().mode = Mode::Editing;

        let editor = app.editor_mut().unwrap();
        editor.goto(0, 2);
        editor.begin_selection();
        editor.move_line_end(true);

        handle(&mut app, ctrl(KeyCode::Char('b')));
        assert!(app.editor_mut().unwrap().text().contains("**B**"));
    }

    #[test]
    fn escape_leaves_the_editor_for_reading_mode() {
        let (_v, mut app) = app();
        let b = app.index.id_of_rel("B.md").unwrap();
        app.open_note(b);
        app.active_mut().unwrap().mode = Mode::Editing;

        handle(&mut app, key(KeyCode::Esc));
        assert_eq!(app.active().unwrap().mode, Mode::Reading);
    }

    #[test]
    fn graph_keys_pan_zoom_and_select() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenGraph);

        let start = app.graph.as_ref().unwrap().center.x;
        handle(&mut app, key(KeyCode::Char('l')));
        assert!(app.graph.as_ref().unwrap().center.x > start);

        handle(&mut app, key(KeyCode::Char('+')));
        assert!(app.graph.as_ref().unwrap().zoom > 1.0);

        handle(&mut app, key(KeyCode::Tab));
        assert!(app.graph.as_ref().unwrap().selected.is_some());
    }

    #[test]
    fn enter_on_a_graph_node_opens_the_note() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenGraph);
        handle(&mut app, key(KeyCode::Tab));
        handle(&mut app, key(KeyCode::Enter));

        assert_eq!(app.view, View::Notes);
        assert!(app.active_note().is_some());
    }

    #[test]
    fn escape_leaves_the_graph() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::OpenGraph);
        handle(&mut app, key(KeyCode::Esc));
        assert_eq!(app.view, View::Notes);
    }

    #[test]
    fn chat_input_accepts_text_and_ctrl_r_clears() {
        let (_v, mut app) = app();
        dispatch(&mut app, Action::ToggleChat);
        assert_eq!(app.focus, Focus::Chat);

        for ch in "hi".chars() {
            handle(&mut app, key(KeyCode::Char(ch)));
        }
        assert_eq!(app.chat.input, "hi");

        app.chat
            .transcript
            .push(crate::agent::Entry::User("x".into()));
        handle(&mut app, ctrl(KeyCode::Char('r')));
        assert!(app.chat.transcript.is_empty());
    }

    #[test]
    fn tab_cycles_only_through_visible_panes() {
        let (_v, mut app) = app();
        app.config.ui.show_right_sidebar = false;
        app.config.ui.show_chat = false;
        app.focus = Focus::Explorer;

        cycle_focus(&mut app, 1);
        assert_eq!(app.focus, Focus::Note);
        cycle_focus(&mut app, 1);
        assert_eq!(app.focus, Focus::Explorer, "wraps past the hidden panes");
    }

    #[test]
    fn the_help_overlay_scrolls_and_closes() {
        let (_v, mut app) = app();
        handle(&mut app, key(KeyCode::Char('?')));
        assert!(matches!(app.modal, Some(Modal::Help(0))));

        handle(&mut app, key(KeyCode::Char('j')));
        assert!(matches!(app.modal, Some(Modal::Help(1))));

        handle(&mut app, key(KeyCode::Esc));
        assert!(app.modal.is_none());
    }

    #[test]
    fn the_explorer_filter_prompt_applies_on_enter() {
        let (_v, mut app) = app();
        app.focus = Focus::Explorer;

        handle(&mut app, key(KeyCode::Char('/')));
        for ch in "B".chars() {
            handle(&mut app, key(KeyCode::Char(ch)));
        }
        handle(&mut app, key(KeyCode::Enter));

        assert_eq!(app.explorer.filter, "B");
        assert_eq!(app.explorer.len(), 1);
    }
}

#[cfg(test)]
mod chat_command_tests {
    use super::*;
    use crate::agent::Entry;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("chat-keys");
        vault.write("A.md", "# A\n");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        app.focus = Focus::Chat;
        (vault, app)
    }

    fn press(app: &mut App, code: KeyCode) {
        handle(app, KeyEvent::new(code, KeyModifiers::empty()));
    }

    fn type_str(app: &mut App, text: &str) {
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }

    #[test]
    fn a_slash_command_runs_locally_instead_of_being_sent() {
        let (_v, mut app) = app();
        type_str(&mut app, "/writes off");
        press(&mut app, KeyCode::Enter);

        assert!(!app.config.agent.allow_writes, "the command took effect");
        assert!(
            app.chat.conversation.is_empty(),
            "a command must never reach the model"
        );
        assert!(app.chat.input.is_empty(), "the input box is cleared");
    }

    #[test]
    fn a_message_mentioning_a_slash_is_still_a_message() {
        let (_v, mut app) = app();
        // Only a leading slash is a command; prose about paths is not.
        type_str(&mut app, "what is in /tmp");
        assert!(!crate::slash::is_command(&app.chat.input));
    }

    #[test]
    fn tab_completes_the_command_being_typed() {
        let (_v, mut app) = app();
        type_str(&mut app, "/resu");
        press(&mut app, KeyCode::Tab);

        assert_eq!(app.chat.input, "/resume ");
        assert_eq!(
            app.chat.cursor,
            app.chat.input.chars().count(),
            "the cursor follows the completion"
        );
    }

    #[test]
    fn arrows_walk_the_command_list_and_enter_takes_the_highlight() {
        let (_v, mut app) = app();
        type_str(&mut app, "/");

        let names: Vec<&str> = crate::slash::completions("/")
            .iter()
            .map(|command| command.name)
            .collect();
        assert!(names.len() > 3, "a bare slash offers everything");

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.chat.completion, 2, "the highlight moved");

        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.chat.input,
            format!("/{} ", names[2]),
            "Enter takes the highlighted command rather than running the first"
        );

        press(&mut app, KeyCode::Enter);
        assert!(
            app.chat.input.is_empty(),
            "and Enter again runs it, since the name is settled"
        );
    }

    #[test]
    fn the_highlight_wraps_and_resets_as_the_command_is_typed() {
        let (_v, mut app) = app();
        type_str(&mut app, "/");

        press(&mut app, KeyCode::Up);
        assert_eq!(
            app.chat.completion,
            crate::slash::completions("/").len() - 1,
            "up from the top wraps to the bottom"
        );

        type_str(&mut app, "s");
        assert_eq!(
            app.chat.completion, 0,
            "a keystroke changes the list, so the highlight starts over"
        );
    }

    #[test]
    fn a_named_command_leaves_the_arrows_to_the_transcript() {
        let (_v, mut app) = app();
        app.chat.scroll = 9;
        app.chat.follow = true;
        type_str(&mut app, "/model ");

        press(&mut app, KeyCode::Up);
        assert_eq!(app.chat.completion, 0, "there is nothing left to choose");
        assert!(
            !app.chat.follow && app.chat.scroll < 9,
            "so Up scrolls the conversation, as it does while writing a message"
        );
    }

    #[test]
    fn escape_abandons_a_half_typed_command_before_leaving_the_panel() {
        let (_v, mut app) = app();
        type_str(&mut app, "/sess");

        press(&mut app, KeyCode::Esc);
        assert!(app.chat.input.is_empty(), "the command is abandoned");
        assert_eq!(app.focus, Focus::Chat, "but the panel keeps focus");

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Note, "a second Escape leaves");
    }

    #[test]
    fn tab_in_a_normal_message_does_not_complete() {
        let (_v, mut app) = app();
        type_str(&mut app, "hello");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.chat.input, "hello");
    }

    #[test]
    fn an_unknown_command_is_reported_and_not_sent() {
        let (_v, mut app) = app();
        type_str(&mut app, "/nope");
        press(&mut app, KeyCode::Enter);

        assert!(app.chat.conversation.is_empty());
        assert!(
            app.status.text.contains("unknown command"),
            "the user is told, rather than the typo silently vanishing"
        );
    }

    #[test]
    fn command_output_lands_in_the_transcript() {
        let (_v, mut app) = app();
        type_str(&mut app, "/vault");
        press(&mut app, KeyCode::Enter);

        assert!(
            matches!(app.chat.transcript.last(), Some(Entry::Context(_))),
            "output is part of the conversation's history"
        );
    }

    #[test]
    fn q_in_the_chat_box_types_rather_than_quits() {
        let (_v, mut app) = app();
        type_str(&mut app, "q");
        assert_eq!(app.chat.input, "q");
        assert!(app.modal.is_none(), "no quit prompt while typing");
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;

    fn graph_app() -> (TempVault, App) {
        let vault = TempVault::new("bindings");
        vault.write("A.md", "# A\n\n[[B]] [[C]]\n");
        vault.write("B.md", "# B\n\n[[C]]\n");
        vault.write("C.md", "# C\n");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        dispatch(&mut app, Action::OpenGraph);
        (vault, app)
    }

    fn press(app: &mut App, ch: char) {
        handle(app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()));
    }

    #[test]
    fn fit_frames_the_graph_rather_than_the_origin() {
        let (_v, mut app) = graph_app();
        {
            let graph = app.graph.as_mut().expect("graph");
            graph.center = otui_core::graph::Vec2::new(9_999.0, 9_999.0);
            graph.zoom = 8.0;
        }

        press(&mut app, 'f');

        let graph = app.graph.as_ref().expect("graph");
        let (min_x, min_y, max_x, max_y) = graph.simulation.graph.bounds();
        assert!((graph.zoom - 1.0).abs() < f32::EPSILON);
        assert!(
            graph.center.x >= min_x && graph.center.x <= max_x,
            "fit must land inside the layout, not back at the origin"
        );
        assert!(graph.center.y >= min_y && graph.center.y <= max_y);
    }

    #[test]
    fn zoom_is_clamped_at_both_ends() {
        let (_v, mut app) = graph_app();
        for _ in 0..60 {
            press(&mut app, '+');
        }
        assert_eq!(app.graph.as_ref().unwrap().zoom, crate::app::MAX_ZOOM);
        for _ in 0..120 {
            press(&mut app, '-');
        }
        assert_eq!(app.graph.as_ref().unwrap().zoom, crate::app::MIN_ZOOM);
    }

    #[test]
    fn the_first_tab_selects_the_best_connected_node() {
        let (_v, mut app) = graph_app();
        handle(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));

        let graph = app.graph.as_ref().expect("graph");
        let selected = graph.selected.expect("something is selected");
        let best = graph
            .simulation
            .graph
            .nodes
            .iter()
            .map(|n| n.degree)
            .max()
            .unwrap_or(0);
        assert_eq!(
            graph.simulation.graph.nodes[selected].degree, best,
            "the first Tab should land somewhere worth looking at"
        );
    }

    #[test]
    fn n_and_shift_n_step_the_selection_both_ways() {
        let (_v, mut app) = graph_app();
        press(&mut app, 'n');
        let first = app.graph.as_ref().unwrap().selected;
        press(&mut app, 'n');
        let second = app.graph.as_ref().unwrap().selected;
        assert_ne!(first, second);
        press(&mut app, 'N');
        assert_eq!(app.graph.as_ref().unwrap().selected, first, "N goes back");
    }

    #[test]
    fn c_recentres_on_the_selection() {
        let (_v, mut app) = graph_app();
        press(&mut app, 'n');
        let node = {
            let graph = app.graph.as_ref().unwrap();
            graph.simulation.graph.nodes[graph.selected.unwrap()].pos
        };
        app.graph.as_mut().unwrap().center = otui_core::graph::Vec2::new(500.0, 500.0);

        press(&mut app, 'c');

        assert_eq!(app.graph.as_ref().unwrap().center, node);
    }

    #[test]
    fn uppercase_l_toggles_labels_and_lowercase_l_pans() {
        let (_v, mut app) = graph_app();
        let before = app.config.graph.show_labels;
        let x = app.graph.as_ref().unwrap().center.x;

        press(&mut app, 'L');
        assert_ne!(app.config.graph.show_labels, before, "L toggles labels");
        assert_eq!(app.graph.as_ref().unwrap().center.x, x, "L does not pan");

        press(&mut app, 'l');
        assert!(app.graph.as_ref().unwrap().center.x > x, "l pans right");
    }

    #[test]
    fn r_rebuilds_the_graph_without_leaving_the_view() {
        let (_v, mut app) = graph_app();
        press(&mut app, 'r');
        assert_eq!(app.view, View::Graph);
        assert!(app.graph.is_some());
    }

    #[test]
    fn arrows_move_the_selection_and_hjkl_moves_the_camera() {
        let (_v, mut app) = graph_app();
        {
            // Two nodes side by side, so "right" has an unambiguous answer.
            let graph = app.graph.as_mut().unwrap();
            graph
                .simulation
                .drag(0, otui_core::graph::Vec2::new(0.0, 0.0));
            graph
                .simulation
                .drag(1, otui_core::graph::Vec2::new(30.0, 0.0));
            graph.selected = Some(0);
        }
        let center = app.graph.as_ref().unwrap().center;

        handle(
            &mut app,
            KeyEvent::new(KeyCode::Right, KeyModifiers::empty()),
        );
        let graph = app.graph.as_ref().unwrap();
        assert_eq!(graph.selected, Some(1), "→ walks to the next node");
        assert_eq!(graph.center, center, "→ does not pan");

        handle(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::empty()),
        );
        let graph = app.graph.as_ref().unwrap();
        assert!(graph.center.x > center.x, "l still pans");
        assert_eq!(
            graph.selected,
            Some(1),
            "panning does not move the selection"
        );
    }

    #[test]
    fn a_toggles_attachments() {
        let (_v, mut app) = graph_app();
        let before = app.config.graph.show_attachments;
        press(&mut app, 'a');
        assert_ne!(app.config.graph.show_attachments, before);
    }

    #[test]
    fn q_asks_before_quitting_from_the_graph() {
        let (_v, mut app) = graph_app();
        press(&mut app, 'q');
        assert!(!app.quit);
        assert!(matches!(app.modal, Some(Modal::Confirm(_))));
    }

    #[test]
    fn enter_with_nothing_selected_says_so_instead_of_doing_nothing() {
        let (_v, mut app) = graph_app();
        app.graph.as_mut().unwrap().selected = None;
        handle(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert!(!app.status.text.is_empty(), "silence looks like a bug");
    }

    #[test]
    fn q_while_editing_types_rather_than_quits() {
        let (vault, mut app) = graph_app();
        let _ = &vault;
        dispatch(&mut app, Action::OpenNotesView);
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);
        dispatch(&mut app, Action::ToggleMode);
        assert_eq!(app.active().map(|t| t.mode), Some(Mode::Editing));

        press(&mut app, 'q');

        assert!(app.modal.is_none(), "q is a letter while editing");
        assert!(app.editor_mut().expect("editor").text().contains('q'));
    }

    #[test]
    fn s_cycles_the_sort_order_in_the_explorer() {
        let vault = TempVault::new("sort-key");
        vault.write("A.md", "a");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        app.focus = Focus::Explorer;

        let before = app.explorer.sort();
        press(&mut app, 's');
        let after = app.explorer.sort();

        assert_ne!(before, after, "s should change the order");
        assert_eq!(after, before.next());
        // The config follows, so "Save settings" keeps it.
        assert_eq!(app.config.ui.sort_order, after.key());
    }

    #[test]
    fn s_is_a_letter_when_typing_rather_than_a_sort_command() {
        let vault = TempVault::new("sort-key-editing");
        vault.write("A.md", "a");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        app.open_note(0);
        dispatch(&mut app, Action::ToggleMode);
        app.focus = Focus::Note;

        let before = app.explorer.sort();
        press(&mut app, 's');

        assert_eq!(app.explorer.sort(), before, "s must not sort while editing");
        assert!(app.editor_mut().expect("editor").text().contains('s'));
    }

    #[test]
    fn cycling_the_sort_order_returns_to_where_it_started() {
        let vault = TempVault::new("sort-cycle");
        vault.write("A.md", "a");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        app.focus = Focus::Explorer;

        let start = app.explorer.sort();
        for _ in 0..otui_core::sort::SortOrder::ALL.len() {
            press(&mut app, 's');
        }
        assert_eq!(app.explorer.sort(), start, "a full cycle should come home");
    }
}
