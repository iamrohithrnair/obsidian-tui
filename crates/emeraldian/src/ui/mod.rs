//! Rendering.
//!
//! The layout mirrors Obsidian's: a narrow icon ribbon, a file explorer, the
//! note pane with a tab bar above it, an outline/backlinks sidebar, an optional
//! agent chat panel, and a status bar. Panes collapse from the outside in as
//! the terminal narrows, so the note itself is the last thing to lose space.

pub mod chat;
pub mod drawing;
pub mod graph;
pub mod modal;
pub mod note;
pub mod panes;

use emeraldian_theme::Palette;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

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
    pub const OUTLINE: &str = "▤";
    pub const CHAT: &str = "✦";
    pub const SETTINGS: &str = "⚙";
    // Characters rather than strings: the editor swaps these in for the markers
    // they stand for, one glyph for one character, so a styled line stays
    // exactly as wide as the source it came from.
    pub const BULLET: char = '•';
    pub const QUOTE_BAR: char = '▎';
    pub const TASK_DONE: char = '☑';
    pub const TASK_TODO: char = '☐';
    /// Marks a row that continues the one above it, drawn in the gutter.
    pub const WRAP: char = '⤷';
    pub const SCROLL_THUMB: &str = "│";
    pub const IMAGE: &str = "▨";
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
    // matters more than the reminder of how to read it. It spills onto a
    // second row when the keys don't fit one, but only where there is height
    // to spare; the rows are laid out here rather than at draw time so the
    // layout and the renderer can't disagree about how many there are.
    let show_hints = app.config.ui.show_hints && area.height >= 8;
    let hint_rows = if show_hints {
        hint_lines(
            hints_for(app),
            &palette,
            area.width as usize,
            if area.height >= 10 { 2 } else { 1 },
        )
    } else {
        Vec::new()
    };
    let mut constraints = vec![
        Constraint::Length(1), // title bar
        Constraint::Min(1),    // body
    ];
    if show_hints {
        constraints.push(Constraint::Length(hint_rows.len() as u16));
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
        draw_hints(frame, &palette, rows[2], hint_rows);
    }
    draw_status_bar(frame, app, &palette, status_row);
    modal::draw(frame, app, &palette, area);

    app.regions = regions;
}

/// The keys worth showing for wherever the user currently is.
///
/// Discoverability in a TUI comes from the screen, not the manual — a user who
/// never presses `?` should still learn the app by using it.
fn hints_for(app: &App) -> &'static [(&'static str, &'static str)] {
    if app.modal.is_some() {
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
            ("↑↓←→", "walk"),
            ("hjkl", "pan"),
            ("+/-", "zoom"),
            ("f", "fit"),
            ("Enter", "open"),
            ("drag", "move"),
            ("L", "labels"),
            ("?", "help"),
        ]
    } else {
        match app.focus {
            Focus::Explorer => &[
                ("Enter", "open"),
                ("Space", "fold"),
                ("/", "filter"),
                ("s", "sort"),
                ("^N", "new"),
                ("^W", "close tab"),
                ("^\\", "files"),
                ("^]", "outline"),
                ("^L", "chat"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Focus::Note => match app.active().map(|t| t.mode) {
                Some(crate::app::Mode::Editing) => &[
                    ("^S", "save"),
                    ("Esc", "read"),
                    ("^B/^I", "bold/italic"),
                    ("Tab", "indent list"),
                    ("^Z", "undo"),
                    ("click", "place cursor"),
                    ("^W", "close tab"),
                    ("^\\", "files"),
                    ("^]", "outline"),
                    ("^L", "chat"),
                    ("^P", "palette"),
                ],
                _ => &[
                    ("^E", "edit"),
                    ("Enter", "follow link"),
                    ("←→", "pan wide"),
                    ("^O", "switcher"),
                    ("^G", "graph"),
                    ("^W", "close tab"),
                    ("^\\", "files"),
                    ("^]", "outline"),
                    ("^L", "chat"),
                    ("?", "help"),
                    ("q", "quit"),
                ],
            },
            Focus::Sidebar => &[
                ("Enter", "jump"),
                ("^K", "next panel"),
                ("Tab", "panes"),
                ("^W", "close tab"),
                ("^\\", "files"),
                ("^]", "outline"),
                ("^L", "chat"),
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
    }
}

/// The separator between two hints on the same row.
const HINT_GAP: &str = "  ·  ";

/// Lays hints out across at most `max_rows` rows of `width` columns.
///
/// There are more keys worth showing than fit on one row, so a row that fills
/// up continues onto the next. Anything still left over once the last row is
/// full is dropped rather than half-drawn — the bar is a reminder, and a
/// truncated one is worse than a shorter one.
fn hint_lines(
    hints: &[(&str, &str)],
    palette: &Palette,
    width: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    if width == 0 || max_rows == 0 {
        return Vec::new();
    }

    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;

    for (key, label) in hints {
        let hint = key.chars().count() + 1 + label.chars().count();
        let gap = if used > 1 {
            HINT_GAP.chars().count()
        } else {
            0
        };

        if used + gap + hint > width {
            if rows.len() + 1 >= max_rows {
                break;
            }
            rows.push(Line::from(std::mem::replace(
                &mut spans,
                vec![Span::raw(" ")],
            )));
            used = 1;
        } else if gap > 0 {
            spans.push(Span::styled(
                HINT_GAP,
                Style::default().fg(palette.text_faint),
            ));
            used += gap;
        }

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
        used += hint;
    }

    if spans.len() > 1 {
        rows.push(Line::from(spans));
    }
    rows
}

fn draw_hints(frame: &mut Frame, palette: &Palette, area: Rect, lines: Vec<Line<'static>>) {
    Paragraph::new(lines)
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
        // After the chat icon rather than beside the file one: both are panels
        // you open and close, and it keeps the earlier icons at their indices.
        (
            icons::OUTLINE,
            app.config.ui.show_right_sidebar,
            Action::ToggleRightSidebar,
        ),
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
            format!("{}{words} words  {backlinks} backlinks  ", position(app))
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

/// Where the cursor is, while editing. Empty the rest of the time.
///
/// A writer wants to know how far down a note they are, and it's the one number
/// that tells you the caret you can see is the caret the buffer thinks it has.
fn position(app: &App) -> String {
    let Some(editor) = app.active().and_then(|tab| {
        (tab.mode == crate::app::Mode::Editing)
            .then_some(tab.editor.as_ref())
            .flatten()
    }) else {
        return String::new();
    };
    let cursor = editor.cursor();
    let selected = editor.selected_text().map_or(String::new(), |text| {
        format!("{} selected  ", text.chars().count())
    });
    format!(
        "{selected}Ln {}/{}, Col {}  ",
        cursor.line + 1,
        editor.line_count(),
        cursor.col + 1
    )
}

fn mode_label(app: &App) -> String {
    match app.view {
        View::Graph => "GRAPH".into(),
        View::Notes => match app.active() {
            Some(tab) => match tab.mode {
                crate::app::Mode::Reading => "READING".into(),
                crate::app::Mode::Editing => "EDITING".into(),
            },
            None => "emeraldian  ·  Ctrl+O to open a note, ? for help".into(),
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
    use emeraldian_core::graph::Vec2;
    use emeraldian_core::test_support::TempVault;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
    fn the_hint_bar_shows_how_to_close_things() {
        let (_vault, mut app) = demo_app();
        let screen = render(&mut app, 140, 40).join("\n");

        // Closing a tab or a pane was reachable but unadvertised, so the only
        // way to find it was the `?` overlay.
        for hint in ["^W close tab", "^\\ files", "^] outline", "^L chat"] {
            assert!(screen.contains(hint), "{hint:?} is missing from the bar");
        }
    }

    #[test]
    fn the_hint_bar_wraps_rather_than_dropping_keys() {
        let palette = crate::app::App::new(
            TempVault::new("hint-width").vault(),
            crate::config::Config::default(),
        )
        .expect("app")
        .theme
        .palette
        .clone();

        let hints = [
            ("^E", "edit"),
            ("Enter", "follow link"),
            ("^W", "close tab"),
            ("^]", "outline"),
        ];

        // Wide enough for everything: one row, nothing lost.
        let one = hint_lines(&hints, &palette, 120, 2);
        assert_eq!(one.len(), 1);
        assert_eq!(
            one[0]
                .spans
                .iter()
                .filter(|s| s.content == HINT_GAP)
                .count(),
            3,
            "all four hints on the row"
        );

        // Too narrow for one row: continues onto a second rather than
        // silently losing the last keys.
        let two = hint_lines(&hints, &palette, 30, 2);
        assert_eq!(two.len(), 2, "wrapped");
        let text: String = two
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("outline"), "the last hint survived: {text:?}");
        for line in &two {
            let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(used <= 30, "a row overflowed its width: {used}");
        }

        // A terminal with only one row to give keeps the old behaviour.
        let capped = hint_lines(&hints, &palette, 30, 1);
        assert_eq!(capped.len(), 1);
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
        assert_eq!(regions.ribbon.len(), 6, "every ribbon icon is a button");
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
    fn panning_reveals_the_far_side_of_a_wide_table() {
        let vault = TempVault::new("pan-wide-table");
        let header: Vec<String> = (0..10).map(|i| format!("column {i}")).collect();
        let cells: Vec<String> = (0..10).map(|i| format!("cell {i}")).collect();
        vault.write(
            "Wide.md",
            &format!(
                "# Wide\n\n| {} |\n|{}|\n| {} |\n",
                header.join(" | "),
                "---|".repeat(10),
                cells.join(" | ")
            ),
        );

        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let wide = app.index.id_of_rel("Wide.md").expect("indexed");
        app.open_note(wide);

        let before = render(&mut app, 100, 30).join("\n");
        assert!(before.contains("column 0"), "the near side is on screen");
        assert!(
            !before.contains("column 9"),
            "the far side is off the right edge, which is the whole problem"
        );

        app.active_mut().expect("tab").hscroll = 90;
        let after = render(&mut app, 100, 30).join("\n");
        assert!(
            after.contains("column 9"),
            "panning brings the far side into view: {after}"
        );
    }

    #[test]
    fn panning_is_clamped_to_the_widest_line() {
        let (_vault, mut app) = demo_app();
        // Far past the end of any line in the note.
        app.active_mut().expect("tab").hscroll = 10_000;
        render(&mut app, 140, 40);
        assert!(
            app.active().expect("tab").hscroll < 140,
            "panning past the content would leave a blank pane with no way back"
        );
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

    /// The foreground colours of every braille cell on screen.
    fn edge_colours(terminal: &Terminal<TestBackend>) -> Vec<ratatui::style::Color> {
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .filter(|cell| {
                cell.symbol()
                    .chars()
                    .next()
                    .is_some_and(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
            })
            .filter_map(|cell| cell.fg.into())
            .collect()
    }

    #[test]
    fn selecting_a_node_lights_up_its_links() {
        let (_vault, mut app) = demo_app();
        app.open_graph(None);
        let graph = app.graph.as_mut().expect("graph");
        graph.simulation.run(4000);
        graph.fit();

        let palette = app.theme.palette.clone();

        // Nothing selected: every link is drawn in the ordinary edge colour,
        // and that colour has to be something other than the background or the
        // graph reads as a scatter plot.
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let plain = edge_colours(&terminal);
        assert!(!plain.is_empty(), "no links were drawn at all");
        assert!(
            plain.iter().all(|c| *c != palette.graph_bg),
            "links are painted in the background colour"
        );

        // Select the best-connected node; its links must now carry the accent,
        // and must survive the ordinary edges drawn around them.
        let graph = app.graph.as_mut().expect("graph");
        graph.cycle_selection(0);
        let selected = graph.selected.expect("a selection");
        let links = graph.simulation.graph.neighbors(selected).len();
        assert!(links > 0, "the test needs a node with links");

        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let highlighted = edge_colours(&terminal);
        assert!(
            highlighted.contains(&palette.graph_edge_active),
            "the selected node's links are not highlighted"
        );
    }

    #[test]
    fn a_highlighted_link_survives_an_ordinary_one_crossing_it() {
        // A braille cell holds one colour — whichever was written last — so
        // drawing every edge in one pass let an ordinary link rub out the
        // highlight wherever the two crossed. That is exactly the middle of the
        // picture, where the reader is looking.
        let vault = TempVault::new("crossing-links");
        vault.write("A.md", "[[B]]\n");
        vault.write("B.md", "b\n");
        vault.write("C.md", "[[D]]\n");
        vault.write("D.md", "d\n");

        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        app.open_graph(None);
        let palette = app.theme.palette.clone();

        // Two diagonals crossing at the origin. Dragging pins them, so the
        // layout cannot drift and the crossing point is exactly (0, 0).
        let graph = app.graph.as_mut().expect("graph");
        let node = |label: &str| {
            graph
                .simulation
                .graph
                .nodes
                .iter()
                .position(|n| n.label == label)
                .unwrap_or_else(|| panic!("{label} is in the graph"))
        };
        let (a, b, c, d) = (node("A"), node("B"), node("C"), node("D"));
        // A-B is listed before C-D, so C-D is the one that used to win.
        graph.simulation.drag(a, Vec2::new(-10.0, -10.0));
        graph.simulation.drag(b, Vec2::new(10.0, 10.0));
        graph.simulation.drag(c, Vec2::new(-10.0, 10.0));
        graph.simulation.drag(d, Vec2::new(10.0, -10.0));
        graph.fit();
        graph.selected = Some(a);

        let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");

        let (rect, x_bounds, y_bounds) = app.regions.graph.expect("graph region");
        let (x, y) = graph::project(Vec2::new(0.0, 0.0), rect, x_bounds, y_bounds)
            .expect("the crossing is on screen");

        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(x, y)].fg,
            palette.graph_edge_active,
            "the crossing at ({x}, {y}) lost the selection's colour to the link crossing it"
        );
    }

    #[test]
    fn the_graph_draws_distinct_nodes_edges_and_labels() {
        let (_vault, mut app) = demo_app();
        app.open_graph(None);
        // Settle before looking: an unsettled layout is still a spiral.
        let graph = app.graph.as_mut().expect("graph");
        graph.simulation.run(4000);
        graph.fit();

        let screen = render(&mut app, 140, 40).join("\n");

        // A note reads as a filled mark and a link with no note behind it as a
        // hollow one — the distinction that tells you what you have yet to
        // write.
        assert!(screen.contains('•'), "a linked note is a filled mark");
        assert!(
            screen.contains('○'),
            "the unresolved [[Nowhere]] link is hollow"
        );
        // Links have to be visible, or the picture is a scatter plot.
        assert!(
            screen
                .chars()
                .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
            "edges are drawn as braille"
        );
        // Every node in a graph this small should find room for its label.
        for label in ["Welcome", "Ideas", "Nowhere", "Deep"] {
            assert!(screen.contains(label), "{label} is labelled");
        }
    }

    #[test]
    fn overlays_render_over_the_app() {
        let (_vault, mut app) = demo_app();

        crate::actions::dispatch(&mut app, crate::app::Action::OpenPalette);
        assert!(
            render(&mut app, 140, 40)
                .join("\n")
                .contains("Command palette")
        );

        crate::actions::dispatch(&mut app, crate::app::Action::OpenHelp);
        assert!(
            render(&mut app, 140, 40)
                .join("\n")
                .contains("Keyboard shortcuts")
        );
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

    /// The note pane, without the chrome around it, as the user sees it.
    fn note_pane(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let rows = render(app, width, height);
        let left = app.regions.main.map_or(0, |rect| rect.x) as usize;
        rows.iter()
            .map(|row| row.chars().skip(left).collect::<String>())
            .collect()
    }

    #[test]
    fn a_long_line_wraps_instead_of_being_cut_off() {
        // The bug this fixes: a line wider than the pane was clipped at the edge
        // and, with no way to scroll sideways either, simply unreachable.
        let vault = TempVault::new("render-wrap");
        let sentence = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
        vault.write("Long.md", &format!("{sentence}\n"));

        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let long = app.index.id_of_rel("Long.md").expect("indexed");
        app.open_note(long);
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);

        let rows = note_pane(&mut app, 60, 20);
        let shown: String = rows.join(" ");
        for word in sentence.split(' ') {
            assert!(shown.contains(word), "{word:?} was cut off: {rows:?}");
        }
        assert!(
            rows.iter().filter(|row| row.contains("alpha")).count() == 1,
            "and it is drawn once, not repeated"
        );
    }

    #[test]
    fn switching_modes_does_not_reflow_the_prose() {
        // The prose column has to sit in the same place with the same width in
        // both modes, or Ctrl+E rewraps the paragraph under the reader's eyes.
        let vault = TempVault::new("render-reflow");
        let sentence = "one two three four five six seven eight nine ten eleven twelve";
        vault.write("Prose.md", &format!("{sentence}\n"));

        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let prose = app.index.id_of_rel("Prose.md").expect("indexed");
        app.open_note(prose);

        let reading = note_pane(&mut app, 46, 12);
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);
        let editing = note_pane(&mut app, 46, 12);

        // Keep only the prose: the gutter carries a line number or a wrap marker
        // while editing, and the hint and status bars legitimately differ.
        let broke_as = |rows: &[String]| -> Vec<String> {
            rows.iter()
                .map(|row| {
                    row.split_whitespace()
                        .filter(|word| sentence.split(' ').any(|prose| prose == *word))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|row| !row.is_empty())
                .collect()
        };
        let reading = broke_as(&reading);
        assert!(reading.len() > 1, "the sentence should wrap at this width");
        assert_eq!(
            reading,
            broke_as(&editing),
            "the same words broke in different places"
        );
    }

    /// Draws a frame and reports where the terminal caret ended up.
    ///
    /// A frame that asks for no caret leaves the backend's position alone, so
    /// comparing across two draws says whether one was drawn at all.
    fn caret_after(terminal: &mut Terminal<TestBackend>, app: &mut App) -> (u16, u16) {
        use ratatui::backend::Backend;

        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let position = terminal
            .backend_mut()
            .get_cursor_position()
            .expect("cursor position");
        (position.x, position.y)
    }

    #[test]
    fn the_caret_follows_the_cursor_onto_a_wrapped_row() {
        // Counting characters put the caret past the pane on a long line; it has
        // to follow the row the character was actually drawn on.
        let vault = TempVault::new("caret-wrap");
        vault.write("Long.md", &format!("{}\n", "word ".repeat(40)));
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let long = app.index.id_of_rel("Long.md").expect("indexed");
        app.open_note(long);
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);

        let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
        let (text, _) = app.regions.editor.expect("the editor was drawn");

        // Character 150 of one long line is well past the pane's width.
        app.editor_mut().expect("editor").goto(0, 150);
        let (x, y) = caret_after(&mut terminal, &mut app);

        assert!(
            x < text.x + text.width,
            "the caret sat at column {x}, outside a pane {} wide",
            text.width
        );
        assert!(y > text.y, "and on a later row, since the line wrapped");
    }

    #[test]
    fn no_caret_is_drawn_when_the_note_pane_is_not_focused() {
        // Otherwise the note claims a caret that belongs to whatever pane is
        // actually taking the keys.
        let (_vault, mut app) = demo_app();
        app.config.ui.show_chat = false;
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
        let focused = caret_after(&mut terminal, &mut app);

        // Move the cursor somewhere else, then hand the keys to another pane. A
        // note pane still drawing a caret would move it; one that isn't cannot.
        app.editor_mut().expect("editor").goto(4, 3);
        app.focus = Focus::Explorer;
        assert_eq!(
            caret_after(&mut terminal, &mut app),
            focused,
            "the note pane drew a caret while the explorer had the keys"
        );

        app.focus = Focus::Note;
        assert_ne!(
            caret_after(&mut terminal, &mut app),
            focused,
            "and draws one again when it gets them back"
        );
    }

    #[test]
    fn editing_shows_markdown_styled_without_moving_it() {
        // Live preview in a character grid: the bullet is drawn as one, but it
        // still occupies the single column the `-` did, so the caret can't drift.
        let (_vault, mut app) = demo_app();
        crate::actions::dispatch(&mut app, crate::app::Action::ToggleMode);
        let rows = note_pane(&mut app, 120, 30);

        let task = rows
            .iter()
            .find(|row| row.contains("a task"))
            .expect("the task line");
        assert!(
            task.contains(&format!("{} [ ] a task", icons::BULLET)),
            "the marker is styled in place: {task:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains(&format!(
                "{} [{}] a done task",
                icons::BULLET,
                icons::TASK_DONE
            ))),
            "a done task shows a ticked box: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("```rust")),
            "and a code fence is still literally what it says"
        );
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
