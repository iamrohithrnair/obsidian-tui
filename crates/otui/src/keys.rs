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
use crate::ui::panes::{sidebar_targets, SidebarTarget};

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

/// Bindings that work everywhere. Returns whether the key was consumed.
fn handle_global(app: &mut App, key: KeyEvent) -> bool {
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
    // Ctrl+Tab still switches document tabs from anywhere.
    let owns_tab = matches!(app.focus, Focus::Note | Focus::Graph);
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
            // Only an explicit yes proceeds; anything else cancels, so a stray
            // keypress can't delete a note.
            KeyCode::Char('y' | 'Y') => {
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
        _ => {}
    }
}

fn handle_note(app: &mut App, key: KeyEvent) {
    let Some(mode) = app.active().map(|t| t.mode) else {
        // With no note open, the pane behaves like the empty state's hints.
        if key.code == KeyCode::Char('?') {
            dispatch(app, Action::OpenHelp);
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

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => step(app, 1),
        KeyCode::Char('k') | KeyCode::Up => step(app, -1),
        KeyCode::PageDown | KeyCode::Char(' ') => step(app, 15),
        KeyCode::PageUp => step(app, -15),
        KeyCode::Char('g') | KeyCode::Home => {
            if let Some(tab) = app.active_mut() {
                tab.scroll = 0;
            }
        }
        KeyCode::Char('G') | KeyCode::End => step(app, 100_000),
        // Following the first link on the page is the reading-mode equivalent
        // of clicking one.
        KeyCode::Enter => follow_first_link(app),
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
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
        _ => {}
    }
}

fn handle_chat(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
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

fn handle_graph(app: &mut App, key: KeyEvent) {
    let Some(graph) = app.graph.as_mut() else {
        return;
    };
    // Pan speed scales with zoom so it feels constant on screen.
    let step = 20.0 / graph.zoom;

    match key.code {
        KeyCode::Char('h') | KeyCode::Left => graph.center.x -= step,
        KeyCode::Char('l') | KeyCode::Right => graph.center.x += step,
        KeyCode::Char('k') | KeyCode::Up => graph.center.y += step,
        KeyCode::Char('j') | KeyCode::Down => graph.center.y -= step,
        KeyCode::Char('+' | '=') => graph.zoom = (graph.zoom * 1.25).min(20.0),
        KeyCode::Char('-' | '_') => graph.zoom = (graph.zoom / 1.25).max(0.1),
        KeyCode::Char('0') => {
            graph.zoom = 1.0;
            graph.center = otui_core::graph::Vec2::default();
        }
        KeyCode::Tab => {
            let count = graph.simulation.graph.nodes.len();
            if count > 0 {
                let next = graph.selected.map_or(0, |s| (s + 1) % count);
                graph.selected = Some(next);
                // Centering on the selection keeps Tab usable as a tour.
                graph.center = graph.simulation.graph.nodes[next].pos;
            }
        }
        KeyCode::BackTab => {
            let count = graph.simulation.graph.nodes.len();
            if count > 0 {
                let next = graph.selected.map_or(0, |s| (s + count - 1) % count);
                graph.selected = Some(next);
                graph.center = graph.simulation.graph.nodes[next].pos;
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
                _ => {}
            }
        }
        KeyCode::Char('L') => dispatch(app, Action::ToggleGraphLabels),
        KeyCode::Char('u') => dispatch(app, Action::ToggleGraphUnresolved),
        KeyCode::Char('t') => dispatch(app, Action::ToggleGraphTags),
        KeyCode::Esc => dispatch(app, Action::OpenNotesView),
        KeyCode::Char('?') => dispatch(app, Action::OpenHelp),
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
