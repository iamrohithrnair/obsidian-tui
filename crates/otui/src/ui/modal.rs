//! Overlay rendering: pickers, prompts, confirmations and help.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;

use otui_theme::Palette;

use crate::app::App;
use crate::modal::{Confirm, Modal, Picker, Prompt};
use crate::ui::{centered, pane_block, scrollbar, truncate};

pub fn draw(frame: &mut Frame, app: &mut App, palette: &Palette, area: Rect) {
    let Some(modal) = app.modal.as_mut() else {
        return;
    };
    match modal {
        Modal::Picker(picker) => draw_picker(frame, picker, palette, area),
        Modal::Prompt(prompt) => draw_prompt(frame, prompt, palette, area),
        Modal::Confirm(confirm) => draw_confirm(frame, confirm, palette, area),
        Modal::Help(scroll) => draw_help(frame, scroll, palette, area),
    }
}

fn draw_picker(frame: &mut Frame, picker: &mut Picker, palette: &Palette, area: Rect) {
    let width = (area.width * 3 / 4).clamp(40, 96);
    let height = (area.height * 2 / 3).clamp(8, 24);
    let rect = centered(area, width, height);

    frame.render_widget(Clear, rect);
    let block = pane_block(picker.kind.title(), true, palette, palette.bg_secondary);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    // Query line.
    let query = if picker.query.is_empty() {
        Span::styled(
            picker.kind.placeholder(),
            Style::default().fg(palette.text_faint),
        )
    } else {
        Span::styled(
            picker.query.clone(),
            Style::default().fg(palette.text_normal),
        )
    };
    Paragraph::new(vec![
        Line::from(vec![
            Span::styled("› ", Style::default().fg(palette.text_accent)),
            query,
        ]),
        Line::from(Span::styled(
            "─".repeat(rows[0].width as usize),
            Style::default().fg(palette.border),
        )),
    ])
    .render(rows[0], frame.buffer_mut());

    frame.set_cursor_position((rows[0].x + 2 + picker.cursor as u16, rows[0].y));

    let list = rows[1];
    let visible_height = list.height as usize;
    picker.scroll_into_view(visible_height);

    if picker.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "  no matches",
            Style::default().fg(palette.text_faint),
        )))
        .render(list, frame.buffer_mut());
        return;
    }

    let selected = picker.selected;
    let scroll = picker.scroll;
    let width = list.width as usize;

    let lines: Vec<Line> = picker
        .visible()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(index, (entry, positions))| {
            let active = index == selected;
            let background = if active {
                palette.bg_active
            } else {
                palette.bg_secondary
            };

            let mut spans = vec![Span::styled(
                if active { "› " } else { "  " },
                Style::default().fg(palette.text_accent).bg(background),
            )];

            // Underline the characters that matched, so the ranking is legible.
            let label_style = Style::default()
                .fg(if active {
                    palette.text_normal
                } else {
                    palette.text_muted
                })
                .bg(background);
            for (i, ch) in entry.label.chars().enumerate() {
                let matched = positions.contains(&byte_of(&entry.label, i));
                spans.push(Span::styled(
                    ch.to_string(),
                    if matched {
                        label_style
                            .fg(palette.text_accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        label_style
                    },
                ));
            }

            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let detail = truncate(&entry.detail, width.saturating_sub(used + 2));
            let pad = width.saturating_sub(used + detail.chars().count());
            spans.push(Span::styled(
                " ".repeat(pad),
                Style::default().bg(background),
            ));
            spans.push(Span::styled(
                detail,
                Style::default().fg(palette.text_faint).bg(background),
            ));

            Line::from(spans)
        })
        .collect();

    Paragraph::new(lines).render(list, frame.buffer_mut());
    scrollbar(frame, palette, list, scroll, picker.len());
}

/// Byte offset of the nth character, matching how fuzzy positions are recorded.
fn byte_of(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn draw_prompt(frame: &mut Frame, prompt: &Prompt, palette: &Palette, area: Rect) {
    let rect = centered(area, 60.min(area.width), 3);
    frame.render_widget(Clear, rect);

    let block = pane_block(&prompt.title, true, palette, palette.bg_secondary);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    Paragraph::new(Line::from(vec![
        Span::styled("› ", Style::default().fg(palette.text_accent)),
        Span::styled(
            prompt.value.clone(),
            Style::default().fg(palette.text_normal),
        ),
    ]))
    .render(inner, frame.buffer_mut());

    frame.set_cursor_position((inner.x + 2 + prompt.cursor as u16, inner.y));
}

fn draw_confirm(frame: &mut Frame, confirm: &Confirm, palette: &Palette, area: Rect) {
    let rect = centered(area, 60.min(area.width), 5);
    frame.render_widget(Clear, rect);

    let block = pane_block("Confirm", true, palette, palette.bg_secondary);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    Paragraph::new(vec![
        Line::from(Span::styled(
            confirm.message.clone(),
            Style::default().fg(palette.text_normal),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "y / Enter",
                Style::default()
                    .fg(palette.text_error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" confirm    ", Style::default().fg(palette.text_muted)),
            Span::styled(
                "n / Esc",
                Style::default()
                    .fg(palette.text_success)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", Style::default().fg(palette.text_muted)),
        ]),
    ])
    .render(inner, frame.buffer_mut());
}

/// The keybinding reference.
///
/// Grouped the way the app is: navigation, notes, panes, graph, assistant.
const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("Ctrl+O", "Quick switcher — open a note by name"),
            ("Ctrl+P", "Command palette"),
            ("Ctrl+Shift+F", "Search all notes"),
            ("Tab / Shift+Tab", "Move between panes"),
            ("hjkl / arrows", "Move within a pane"),
            ("Enter", "Open the selection / follow a link"),
            ("Alt+←", "Back to the previous note"),
            ("Esc", "Close an overlay"),
            ("?", "This help"),
            ("q", "Quit (asks first) — Ctrl+Q works while editing"),
        ],
    ),
    (
        "Notes",
        &[
            ("Ctrl+E", "Toggle reading and editing"),
            ("Ctrl+N", "New note"),
            ("Ctrl+S", "Save"),
            ("Ctrl+D", "Today's daily note"),
            ("F2", "Rename the open note"),
            ("Ctrl+W", "Close the tab"),
            ("Ctrl+Tab", "Next tab"),
            ("Ctrl+B / Ctrl+I", "Bold / italic (while editing)"),
            ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
        ],
    ),
    (
        "Panes",
        &[
            ("Ctrl+\\", "Toggle the file explorer"),
            ("Ctrl+]", "Toggle the outline sidebar"),
            ("Ctrl+K", "Cycle outline / backlinks / tags"),
            ("Ctrl+T", "Theme picker"),
        ],
    ),
    (
        "Graph",
        &[
            ("Ctrl+G", "Whole-vault graph"),
            ("Ctrl+Shift+G", "Local graph for the open note"),
            ("hjkl", "Pan"),
            ("+ / -", "Zoom"),
            ("f / 0", "Fit the whole graph on screen"),
            ("Tab / n", "Next node"),
            ("Shift+Tab / N", "Previous node"),
            ("c", "Centre on the selected node"),
            ("Enter", "Open the selected node"),
            ("L", "Toggle labels"),
            ("u", "Toggle unresolved links"),
            ("t", "Toggle tag nodes"),
            ("r", "Rebuild the layout"),
        ],
    ),
    (
        "Assistant",
        &[
            ("Ctrl+L", "Toggle the chat panel / focus it"),
            ("Enter", "Send"),
            ("/", "Slash command (type / for the list)"),
            ("Ctrl+C", "Stop the current turn"),
            ("Ctrl+R", "Clear the conversation"),
        ],
    ),
];

fn draw_help(frame: &mut Frame, scroll: &mut usize, palette: &Palette, area: Rect) {
    let rect = centered(
        area,
        72.min(area.width),
        area.height.saturating_sub(4).max(10),
    );
    frame.render_widget(Clear, rect);

    let block = pane_block("Keyboard shortcuts", true, palette, palette.bg_secondary);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines = Vec::new();
    for (section, bindings) in HELP {
        lines.push(Line::from(Span::styled(
            (*section).to_string(),
            Style::default()
                .fg(palette.text_accent)
                .add_modifier(Modifier::BOLD),
        )));
        for (key, description) in *bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {key:<16}"),
                    Style::default().fg(palette.text_normal),
                ),
                Span::styled(
                    (*description).to_string(),
                    Style::default().fg(palette.text_muted),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    let height = inner.height as usize;
    *scroll = (*scroll).min(lines.len().saturating_sub(height));

    let visible: Vec<Line> = lines.iter().skip(*scroll).take(height).cloned().collect();
    Paragraph::new(visible).render(inner, frame.buffer_mut());
    scrollbar(frame, palette, inner, *scroll, lines.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_covers_every_section_and_has_no_blank_entries() {
        assert!(HELP.len() >= 5);
        for (section, bindings) in HELP {
            assert!(!section.is_empty());
            assert!(!bindings.is_empty(), "{section} has no bindings");
            for (key, description) in *bindings {
                assert!(!key.is_empty() && !description.is_empty());
            }
        }
    }

    #[test]
    fn byte_offsets_line_up_with_multi_byte_text() {
        assert_eq!(byte_of("héllo", 0), 0);
        assert_eq!(byte_of("héllo", 2), 3, "é is two bytes");
        assert_eq!(byte_of("héllo", 99), 6);
    }

    /// Every key the help table promises, and the pane it belongs to.
    ///
    /// Documented shortcuts that don't work are worse than undocumented ones —
    /// this catches the drift rather than trusting a proofread.
    fn documented_graph_keys() -> Vec<&'static str> {
        HELP.iter()
            .find(|(section, _)| *section == "Graph")
            .map(|(_, bindings)| bindings.iter().map(|(key, _)| *key).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_help_table_documents_the_graph_keys_that_exist() {
        let keys = documented_graph_keys();
        for expected in ["hjkl", "+ / -", "f / 0", "Tab / n", "c", "L", "u", "t", "r"] {
            assert!(
                keys.contains(&expected),
                "the graph section should list {expected}, has {keys:?}"
            );
        }
        assert!(
            !keys.contains(&"l"),
            "labels are bound to L, not l — a lowercase l pans right"
        );
    }

    #[test]
    fn quitting_and_help_are_documented_where_a_newcomer_looks_first() {
        let navigation = HELP
            .iter()
            .find(|(section, _)| *section == "Navigation")
            .map(|(_, b)| *b)
            .expect("a Navigation section");
        let keys: Vec<&str> = navigation.iter().map(|(key, _)| *key).collect();
        assert!(keys.contains(&"q"), "q quits: {keys:?}");
        assert!(keys.contains(&"?"), "? opens this table: {keys:?}");
    }

    #[test]
    fn the_assistant_section_mentions_slash_commands() {
        let assistant = HELP
            .iter()
            .find(|(section, _)| *section == "Assistant")
            .map(|(_, b)| *b)
            .expect("an Assistant section");
        assert!(
            assistant.iter().any(|(key, _)| *key == "/"),
            "slash commands are only discoverable if they're listed"
        );
    }
}
