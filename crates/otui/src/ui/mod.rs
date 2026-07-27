//! Rendering.
//!
//! The layout mirrors Obsidian's: a narrow icon ribbon, a file explorer, the
//! note pane with a tab bar above it, an outline/backlinks sidebar, an optional
//! agent chat panel, and a status bar. Panes collapse from the outside in as
//! the terminal narrows, so the note itself is the last thing to lose space.

pub mod chat;
pub mod graph;
pub mod modal;
pub mod note;
pub mod panes;

use otui_theme::Palette;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use ratatui::Frame;

use crate::app::{Action, App, Focus, Regions, View};
use crate::modal::Modal;

/// Glyphs used across the UI.
///
/// Restricted to characters that render in a plain terminal font — a Nerd Font
/// icon set would look closer to Obsidian for some users and like tofu for
/// everyone else.
pub mod icons {
    pub const FOLDER_OPEN: &str = "▾";
    pub const FOLDER_CLOSED: &str = "▸";
    pub const NOTE: &str = "·";
    pub const MODIFIED: &str = "●";
    pub const SEARCH: &str = "⌕";
    pub const GRAPH: &str = "◈";
    pub const FILES: &str = "≡";
    pub const CHAT: &str = "✦";
    pub const SETTINGS: &str = "⚙";
    pub const BULLET: &str = "•";
    pub const QUOTE_BAR: &str = "▎";
    pub const TASK_DONE: &str = "☑";
    pub const TASK_TODO: &str = "☐";
    pub const SCROLL_THUMB: &str = "│";
}

/// Minimum width before the left sidebar is dropped.
const MIN_WIDTH_FOR_SIDEBAR: u16 = 76;
/// Minimum width before the right sidebar is dropped.
const MIN_WIDTH_FOR_RIGHT: u16 = 110;
/// Minimum width before the chat panel is dropped.
const MIN_WIDTH_FOR_CHAT: u16 = 100;

/// Which panes are visible at the current terminal size.
pub struct Visible {
    pub ribbon: bool,
    pub left: bool,
    pub right: bool,
    pub chat: bool,
}

impl Visible {
    fn resolve(app: &App, width: u16) -> Self {
        let ui = &app.config.ui;
        // Chat outranks the outline sidebar: it was opened deliberately, while
        // the sidebar is on by default.
        let chat = ui.show_chat && width >= MIN_WIDTH_FOR_CHAT;
        let left = ui.show_left_sidebar && width >= MIN_WIDTH_FOR_SIDEBAR;
        let right = ui.show_right_sidebar
            && width >= MIN_WIDTH_FOR_RIGHT
            && !(chat && width < MIN_WIDTH_FOR_RIGHT + ui.chat_width);
        Self {
            ribbon: ui.show_ribbon && width >= MIN_WIDTH_FOR_SIDEBAR,
            left,
            right,
            chat,
        }
    }
}

/// Draws a frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let palette = app.theme.palette.clone();
    let area = frame.area();

    // Paint the window background first so themes with a real background color
    // fill the terminal instead of leaving its default showing through.
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.bg_primary)),
        area,
    );

    // The hint bar is the first thing to go on a short terminal — the note
    // matters more than the reminder of how to read it.
    let show_hints = app.config.ui.show_hints && area.height >= 8;
    let mut constraints = vec![
        Constraint::Length(1), // title bar
        Constraint::Min(1),    // body
    ];
    if show_hints {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1)); // status bar

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let body_row = rows[1];
    let status_row = rows[rows.len() - 1];

    draw_title_bar(frame, app, &palette, rows[0]);

    let mut regions = Regions::default();

    let visible = Visible::resolve(app, area.width);
    let mut pane_constraints = Vec::new();
    if visible.ribbon {
        pane_constraints.push(Constraint::Length(3));
    }
    if visible.left {
        pane_constraints.push(Constraint::Length(app.config.ui.sidebar_width));
    }
    pane_constraints.push(Constraint::Min(24));
    if visible.right {
        pane_constraints.push(Constraint::Length(app.config.ui.right_sidebar_width));
    }
    if visible.chat {
        pane_constraints.push(Constraint::Length(app.config.ui.chat_width));
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(pane_constraints)
        .split(body_row);

    let mut column = 0;
    if visible.ribbon {
        draw_ribbon(frame, app, &palette, columns[column], &mut regions);
        column += 1;
    }
    if visible.left {
        panes::draw_explorer(frame, app, &palette, columns[column], &mut regions);
        column += 1;
    }

    let main = columns[column];
    column += 1;
    regions.main = Some(main);
    match app.view {
        View::Notes => note::draw(frame, app, &palette, main, &mut regions),
        View::Graph => graph::draw(frame, app, &palette, main, &mut regions),
    }

    if visible.right {
        panes::draw_sidebar(frame, app, &palette, columns[column], &mut regions);
        column += 1;
    }
    if visible.chat {
        regions.chat = Some(columns[column]);
        chat::draw(frame, app, &palette, columns[column]);
    }

    if show_hints {
        draw_hints(frame, app, &palette, rows[2]);
    }
    draw_status_bar(frame, app, &palette, status_row);
    modal::draw(frame, app, &palette, area);

    app.regions = regions;
}

/// The shortcut bar: the handful of keys that matter wherever you are.
///
/// Discoverability in a TUI comes from the screen, not the manual — a user who
/// never presses `?` should still learn the app by using it.
fn draw_hints(frame: &mut Frame, app: &App, palette: &Palette, area: Rect) {
    let hints: &[(&str, &str)] = if app.modal.is_some() {
        match app.modal.as_ref() {
            Some(Modal::Confirm(_)) => &[("y/Enter", "confirm"), ("Esc", "cancel")],
            Some(Modal::Help(_)) => &[("j/k", "scroll"), ("Esc", "close")],
            Some(Modal::Prompt(_)) => &[("Enter", "confirm"), ("Esc", "cancel")],
            _ => &[
                ("Enter", "select"),
                ("↑↓", "move"),
                ("^U", "clear"),
                ("Esc", "cancel"),
            ],
        }
    } else if app.view == View::Graph {
        &[
            ("hjkl", "pan"),
            ("+/-", "zoom"),
            ("f", "fit"),
            ("Tab", "next node"),
            ("Enter", "open"),
            ("L", "labels"),
            ("?", "help"),
            ("q", "quit"),
        ]
    } else {
        match app.focus {
            Focus::Explorer => &[
                ("Enter", "open"),
                ("Space", "fold"),
                ("/", "filter"),
                ("s", "sort"),
                ("^N", "new"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Focus::Note => match app.active().map(|t| t.mode) {
                Some(crate::app::Mode::Editing) => &[
                    ("^S", "save"),
                    ("Esc", "read"),
                    ("^B/^I", "bold/italic"),
                    ("^Z", "undo"),
                    ("^P", "palette"),
                ],
                _ => &[
                    ("^E", "edit"),
                    ("Enter", "follow link"),
                    ("^O", "switcher"),
                    ("^G", "graph"),
                    ("?", "help"),
                    ("q", "quit"),
                ],
            },
            Focus::Sidebar => &[
                ("Enter", "jump"),
                ("^K", "next panel"),
                ("Tab", "panes"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Focus::Chat => &[
                ("Enter", "send"),
                ("/", "commands"),
                ("^C", "stop"),
                ("^R", "clear"),
                ("Esc", "leave"),
            ],
            // Focus lands here only while the graph is not the active view,
            // which the branch above already handles.
            Focus::Graph => &[("Esc", "back"), ("?", "help"), ("q", "quit")],
        }
    };

    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default()
                .fg(palette.text_accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(palette.text_muted),
        ));
        spans.push(Span::styled(
            "  ·  ",
            Style::default().fg(palette.text_faint),
        ));
    }
    spans.pop();

    // Drop trailing hints rather than wrapping onto a second row.
    let mut used = 0usize;
    let width = area.width as usize;
    spans.retain(|span| {
        used += span.content.chars().count();
        used <= width
    });

    Paragraph::new(Line::from(spans))
        .style(Style::default().bg(palette.bg_secondary))
        .render(area, frame.buffer_mut());
}

fn draw_title_bar(frame: &mut Frame, app: &App, palette: &Palette, area: Rect) {
    let mut spans = vec![
        Span::styled(
            format!(" {} ", app.index.vault.name),
            Style::default()
                .fg(palette.text_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(palette.border)),
    ];

    match app.active_note() {
        Some(id) => {
            let note = app.index.note(id);
            let path = note.map_or_else(String::new, |n| n.meta.rel.clone());
            spans.push(Span::styled(path, Style::default().fg(palette.titlebar_fg)));
        }
        None => spans.push(Span::styled(
            "No note open",
            Style::default().fg(palette.text_faint),
        )),
    }

    let right = format!("{} ", app.theme.name());
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + right.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, Style::default().fg(palette.text_faint)));

    Paragraph::new(Line::from(spans))
        .style(Style::default().bg(palette.titlebar_bg))
        .render(area, frame.buffer_mut());
}

/// Obsidian's left ribbon: a vertical strip of mode buttons.
///
/// Each icon is a real button — the rect and its action are recorded so a click
/// runs the same command the keyboard shortcut does.
fn draw_ribbon(frame: &mut Frame, app: &App, palette: &Palette, area: Rect, regions: &mut Regions) {
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.ribbon_bg)),
        area,
    );

    let entries = [
        (
            icons::FILES,
            matches!(app.focus, Focus::Explorer),
            Action::ToggleLeftSidebar,
        ),
        (icons::SEARCH, false, Action::OpenSearch),
        (icons::GRAPH, app.view == View::Graph, Action::OpenGraph),
        (icons::CHAT, app.config.ui.show_chat, Action::ToggleChat),
        (icons::SETTINGS, false, Action::OpenPalette),
    ];

    for (row, (icon, active, action)) in entries.into_iter().enumerate() {
        let y = area.y + 1 + row as u16 * 2;
        if y >= area.y + area.height {
            break;
        }
        let rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        let style = Style::default().bg(palette.ribbon_bg).fg(if active {
            palette.ribbon_icon_active
        } else {
            palette.ribbon_icon
        });
        Paragraph::new(Line::from(Span::styled(format!(" {icon} "), style)))
            .render(rect, frame.buffer_mut());
        regions.ribbon.push((rect, action));
    }
}

fn draw_status_bar(frame: &mut Frame, app: &App, palette: &Palette, area: Rect) {
    let mut left = Vec::new();

    if !app.status.text.is_empty() {
        left.push(Span::styled(
            format!(" {} ", app.status.text),
            Style::default().fg(if app.status.is_error {
                palette.text_error
            } else {
                palette.text_success
            }),
        ));
    } else {
        left.push(Span::styled(
            format!(" {} ", mode_label(app)),
            Style::default().fg(palette.text_accent),
        ));
    }

    // Obsidian keeps word and character counts bottom-right; backlink count is
    // the other number a note's author actually looks at.
    let right = match app.active_note() {
        Some(id) => {
            let words = app.index.note(id).map_or(0, |n| n.words);
            let backlinks = app.index.backlinks(id).len();
            format!("{words} words  {backlinks} backlinks  ")
        }
        None => {
            let stats = app.index.stats();
            format!("{} notes  {} tags  ", stats.notes, stats.tags)
        }
    };

    let used: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + right.chars().count());
    left.push(Span::raw(" ".repeat(pad)));
    left.push(Span::styled(
        right,
        Style::default().fg(palette.statusbar_fg),
    ));

    Paragraph::new(Line::from(left))
        .style(Style::default().bg(palette.statusbar_bg))
        .render(area, frame.buffer_mut());
}

fn mode_label(app: &App) -> String {
    match app.view {
        View::Graph => "GRAPH".into(),
        View::Notes => match app.active() {
            Some(tab) => match tab.mode {
                crate::app::Mode::Reading => "READING".into(),
                crate::app::Mode::Editing => "EDITING".into(),
            },
            None => "obsidian-tui  ·  Ctrl+O to open a note, ? for help".into(),
        },
    }
}

/// A bordered block in the app's style, brighter when focused.
#[must_use]
pub fn pane_block<'a>(
    title: &'a str,
    focused: bool,
    palette: &Palette,
    background: ratatui::style::Color,
) -> Block<'a> {
    Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(if focused {
            palette.border_focus
        } else {
            palette.border
        }))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused {
                    palette.text_accent
                } else {
                    palette.text_muted
                })
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(background))
}

/// Truncates to `width` display columns, adding an ellipsis when it doesn't fit.
#[must_use]
pub fn truncate(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;

    if width == 0 {
        return String::new();
    }
    let total: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= width {
        return text.to_string();
    }

    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Wraps text to `width` columns on word boundaries.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;

    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0;

    for word in text.split(' ') {
        let word_width: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();

        if used > 0 && used + 1 + word_width > width {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }

        // A single word longer than the line has to be broken somewhere.
        if word_width > width {
            for ch in word.chars() {
                let w = ch.width().unwrap_or(0);
                if used + w > width {
                    lines.push(std::mem::take(&mut current));
                    used = 0;
                }
                current.push(ch);
                used += w;
            }
            continue;
        }

        if used > 0 {
            current.push(' ');
            used += 1;
        }
        current.push_str(word);
        used += word_width;
    }

    lines.push(current);
    lines
}

/// Draws a vertical scrollbar on the right edge of `area`.
pub fn scrollbar(frame: &mut Frame, palette: &Palette, area: Rect, offset: usize, total: usize) {
    if area.height == 0 || total <= area.height as usize {
        return;
    }
    let height = area.height as usize;
    let thumb = ((height * height) / total).max(1);
    let max_offset = total.saturating_sub(height);
    // A track shorter than the thumb has nowhere to travel, so the division
    // simply doesn't apply.
    let position = (offset * (height - thumb))
        .checked_div(max_offset)
        .unwrap_or(0);

    let x = area.x + area.width.saturating_sub(1);
    for row in 0..height {
        let inside = row >= position && row < position + thumb;
        let style = Style::default().fg(if inside {
            palette.scrollbar_thumb
        } else {
            palette.scrollbar_track
        });
        frame
            .buffer_mut()
            .set_string(x, area.y + row as u16, icons::SCROLL_THUMB, style);
    }
}

/// Centers a box of the given size inside `area`.
#[must_use]
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 3,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use otui_core::test_support::TempVault;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Renders a full frame and returns it as text, one string per row.
    ///
    /// This is the closest thing to running the app that a test can do: it
    /// exercises the real layout and every pane's draw path, and catches
    /// panics from arithmetic on small terminals — the failure mode that only
    /// shows up when someone resizes their window.
    fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("draw a frame");

        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn demo_app() -> (TempVault, App) {
        let vault = TempVault::new("render-smoke");
        vault.write(
            "Welcome.md",
            "---\ntags: [start]\n---\n# Welcome\n\nA note that links to [[Ideas]] and [[Nowhere]].\n\n- [ ] a task\n- [x] a done task\n\n> [!tip] Callout\n> body text\n\n```rust\nlet x = 1;\n```\n",
        );
        vault.write("Ideas.md", "# Ideas\n\nBack to [[Welcome]].\n");
        vault.write("Projects/Deep.md", "# Deep\n");

        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let welcome = app.index.id_of_rel("Welcome.md").expect("indexed");
        app.open_note(welcome);
        (vault, app)
    }

    #[test]
    fn the_hint_bar_shows_context_appropriate_keys() {
        let (_vault, mut app) = demo_app();

        let reading = render(&mut app, 140, 40).join("\n");
        assert!(reading.contains("edit"), "reading mode offers ^E");
        assert!(reading.contains("follow link"));

        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);
        let editing = render(&mut app, 140, 40).join("\n");
        assert!(editing.contains("save"), "editing offers ^S");

        app.focus = crate::app::Focus::Explorer;
        let explorer = render(&mut app, 140, 40).join("\n");
        assert!(explorer.contains("filter"), "the explorer offers /");
    }

    #[test]
    fn the_hint_bar_can_be_turned_off_and_yields_on_short_terminals() {
        let (_vault, mut app) = demo_app();
        assert!(render(&mut app, 140, 40).join("\n").contains("help"));

        crate::actions::dispatch(&mut app, crate::app::Action::ToggleHints);
        assert!(!render(&mut app, 140, 40).join("\n").contains("· ? help"));

        // Re-enabled, but the terminal is too short to spare a row.
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleHints);
        render(&mut app, 140, 6);
    }

    #[test]
    fn drawing_records_clickable_regions() {
        let (_vault, mut app) = demo_app();
        app.config.ui.show_chat = true;
        render(&mut app, 160, 40);

        let regions = &app.regions;
        assert_eq!(regions.ribbon.len(), 5, "every ribbon icon is a button");
        assert_eq!(regions.tabs.len(), app.tabs.len());
        assert_eq!(regions.side_tabs.len(), 3);
        assert!(regions.explorer.is_some());
        assert!(regions.sidebar.is_some());
        assert!(regions.chat.is_some());
        assert!(regions.main.is_some());

        // Regions must not overlap the pane next door.
        let (explorer, _) = regions.explorer.unwrap();
        let main = regions.main.unwrap();
        assert!(explorer.x + explorer.width <= main.x);
    }

    #[test]
    fn the_graph_records_its_canvas_bounds_for_hit_testing() {
        let (_vault, mut app) = demo_app();
        app.open_graph(None);
        render(&mut app, 140, 40);

        let (rect, x_bounds, y_bounds) = app.regions.graph.expect("graph region");
        assert!(rect.width > 0 && rect.height > 0);
        assert!(x_bounds[1] > x_bounds[0]);
        assert!(y_bounds[1] > y_bounds[0]);
    }

    #[test]
    fn a_full_frame_renders_every_pane() {
        let (_vault, mut app) = demo_app();
        let rows = render(&mut app, 140, 40);
        let screen = rows.join("\n");

        // Chrome.
        assert!(
            screen.contains("Welcome.md"),
            "title bar shows the open note"
        );
        assert!(screen.contains("Files"), "explorer pane");
        assert!(screen.contains("Outline"), "right sidebar");
        assert!(screen.contains("words"), "status bar counts");

        // Content.
        assert!(screen.contains("Ideas"), "explorer lists notes");
        assert!(screen.contains("Projects"), "and folders");
        assert!(
            screen.contains(icons::TASK_TODO),
            "tasks render as checkboxes"
        );
        assert!(screen.contains("Callout"), "callouts render");
    }

    #[test]
    fn the_chat_panel_renders_when_enabled() {
        let (_vault, mut app) = demo_app();
        app.config.ui.show_chat = true;
        app.chat
            .transcript
            .push(crate::agent::Entry::User("hello".into()));

        let screen = render(&mut app, 160, 40).join("\n");
        assert!(screen.contains("Assistant"));
        assert!(screen.contains("hello"));
    }

    #[test]
    fn the_graph_view_renders() {
        let (_vault, mut app) = demo_app();
        app.open_graph(None);

        let screen = render(&mut app, 140, 40).join("\n");
        assert!(
            screen.contains("nodes"),
            "the legend reports the graph size"
        );
        assert!(screen.contains("links"));
    }

    #[test]
    fn overlays_render_over_the_app() {
        let (_vault, mut app) = demo_app();

        crate::actions::dispatch(&mut app, crate::app::Action::OpenPalette);
        assert!(render(&mut app, 140, 40)
            .join("\n")
            .contains("Command palette"));

        crate::actions::dispatch(&mut app, crate::app::Action::OpenHelp);
        assert!(render(&mut app, 140, 40)
            .join("\n")
            .contains("Keyboard shortcuts"));
    }

    #[test]
    fn narrow_terminals_drop_panes_instead_of_panicking() {
        let (_vault, mut app) = demo_app();
        app.config.ui.show_chat = true;

        // The note pane is the last thing to lose space.
        let wide = render(&mut app, 160, 40).join("\n");
        assert!(wide.contains("Files") && wide.contains("Assistant"));

        let narrow = render(&mut app, 70, 24).join("\n");
        assert!(!narrow.contains("Assistant"), "the chat panel drops first");
        assert!(narrow.contains("Welcome"), "the note always survives");
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let (_vault, mut app) = demo_app();
        // Every pane and overlay, at sizes where naive layout arithmetic
        // underflows.
        for (width, height) in [(20u16, 5u16), (10, 3), (40, 10), (1, 1)] {
            render(&mut app, width, height);
        }

        app.open_graph(None);
        render(&mut app, 12, 6);

        crate::actions::dispatch(&mut app, crate::app::Action::OpenPalette);
        render(&mut app, 12, 6);
    }

    #[test]
    fn every_theme_renders() {
        let (_vault, mut app) = demo_app();
        let names: Vec<String> = app.themes.iter().map(|t| t.name.clone()).collect();

        for name in names {
            app.set_theme(&name);
            let screen = render(&mut app, 100, 30).join("\n");
            assert!(screen.contains("Welcome"), "theme {name} broke rendering");
        }
    }

    #[test]
    fn editing_mode_renders_with_a_gutter() {
        let (_vault, mut app) = demo_app();
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);

        let screen = render(&mut app, 120, 30).join("\n");
        assert!(screen.contains("# Welcome"), "editing shows the source");
        assert!(screen.contains(" 1 "), "line numbers");
    }

    #[test]
    fn truncate_respects_display_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_handles_wide_characters() {
        // CJK characters occupy two columns each.
        let text = "日本語テキスト";
        let out = truncate(text, 6);
        let width: usize = out
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        assert!(width <= 6, "{out:?} is {width} columns");
    }

    #[test]
    fn wrap_breaks_on_word_boundaries() {
        assert_eq!(
            wrap("the quick brown fox", 10),
            vec!["the quick", "brown fox"]
        );
    }

    #[test]
    fn wrap_splits_words_longer_than_the_line() {
        let lines = wrap("supercalifragilistic", 8);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| l.chars().count() <= 8));
    }

    #[test]
    fn wrap_of_short_text_is_one_line() {
        assert_eq!(wrap("short", 40), vec!["short"]);
        assert_eq!(wrap("", 40), vec![""]);
    }

    #[test]
    fn centered_box_fits_inside_the_area() {
        let area = Rect::new(0, 0, 100, 40);
        let inner = centered(area, 60, 20);
        assert_eq!(inner.width, 60);
        assert!(inner.x + inner.width <= area.width);
        assert!(inner.y + inner.height <= area.height);
    }

    #[test]
    fn centered_box_clamps_to_a_small_terminal() {
        let area = Rect::new(0, 0, 20, 10);
        let inner = centered(area, 60, 20);
        assert_eq!(inner.width, 20);
        assert_eq!(inner.height, 10);
    }
}
