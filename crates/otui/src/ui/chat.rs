//! The agent chat panel.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use otui_theme::Palette;

use crate::agent::{Entry, ToolStatus};
use crate::app::{App, Focus};
use crate::ui::{pane_block, scrollbar, wrap};

/// Rows reserved for the input box.
const INPUT_HEIGHT: u16 = 3;

pub fn draw(frame: &mut Frame, app: &mut App, palette: &Palette, area: Rect) {
    let focused = app.focus == Focus::Chat;
    let title = if app.chat.busy {
        "Assistant  ·  working…"
    } else {
        "Assistant"
    };

    let block = pane_block(title, focused, palette, palette.bg_secondary);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(INPUT_HEIGHT)])
        .split(inner);

    draw_transcript(frame, app, palette, rows[0]);
    draw_input(frame, app, palette, rows[1], focused);
}

fn draw_transcript(frame: &mut Frame, app: &mut App, palette: &Palette, area: Rect) {
    let width = area.width.saturating_sub(1) as usize;
    let lines = transcript_lines(app, palette, width);

    let height = area.height as usize;
    let max_scroll = lines.len().saturating_sub(height);

    // Following means new output stays visible; scrolling up stops it so the
    // user can read without being yanked to the bottom.
    if app.chat.follow {
        app.chat.scroll = max_scroll;
    } else {
        app.chat.scroll = app.chat.scroll.min(max_scroll);
    }

    if lines.is_empty() {
        let hint = if app.chat.settings.allow_writes {
            "Ask about your notes. The assistant can search, read, create and link them."
        } else {
            "Ask about your notes. The assistant can search and read them (writes are off)."
        };
        let hint_lines: Vec<Line> = wrap(hint, width)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, Style::default().fg(palette.text_faint))))
            .collect();
        Paragraph::new(hint_lines).render(area, frame.buffer_mut());
        return;
    }

    let visible: Vec<Line> = lines
        .iter()
        .skip(app.chat.scroll)
        .take(height)
        .cloned()
        .collect();
    Paragraph::new(visible).render(area, frame.buffer_mut());
    scrollbar(frame, palette, area, app.chat.scroll, lines.len());
}

/// Renders the transcript into wrapped, styled lines.
#[must_use]
pub fn transcript_lines(app: &App, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for entry in &app.chat.transcript {
        match entry {
            Entry::User(text) => {
                lines.push(Line::from(Span::styled(
                    "You",
                    Style::default()
                        .fg(palette.text_accent)
                        .add_modifier(Modifier::BOLD),
                )));
                for line in wrap(text, width) {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(palette.text_normal),
                    )));
                }
                lines.push(Line::from(""));
            }

            Entry::Assistant(text) => {
                for paragraph in text.split('\n') {
                    for line in wrap(paragraph, width) {
                        lines.push(Line::from(Span::styled(
                            line,
                            Style::default().fg(palette.text_normal),
                        )));
                    }
                }
                lines.push(Line::from(""));
            }

            Entry::Reasoning(text) => {
                for line in wrap(text, width.saturating_sub(2)) {
                    lines.push(Line::from(Span::styled(
                        format!("  {line}"),
                        Style::default()
                            .fg(palette.text_faint)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
            }

            Entry::Tool {
                name,
                detail,
                status,
            } => {
                // Tool calls are shown so the user can see what the agent
                // actually did to their vault — not just what it says it did.
                let (glyph, color) = match status {
                    ToolStatus::Running => ("◌", palette.text_muted),
                    ToolStatus::Ok => ("✓", palette.text_success),
                    ToolStatus::Failed => ("✗", palette.text_error),
                };
                let text = if detail.is_empty() {
                    name.clone()
                } else {
                    format!("{name}  {detail}")
                };
                for (i, line) in wrap(&text, width.saturating_sub(2)).into_iter().enumerate() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            if i == 0 {
                                format!("{glyph} ")
                            } else {
                                "  ".into()
                            },
                            Style::default().fg(color),
                        ),
                        Span::styled(line, Style::default().fg(palette.text_muted)),
                    ]));
                }
            }

            Entry::Context(text) => {
                lines.push(Line::from(Span::styled(
                    format!("⎘ {text}"),
                    Style::default().fg(palette.text_faint),
                )));
            }

            Entry::Error(text) => {
                for line in wrap(text, width) {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(palette.text_error),
                    )));
                }
                lines.push(Line::from(""));
            }
        }
    }

    lines
}

fn draw_input(frame: &mut Frame, app: &App, palette: &Palette, area: Rect, focused: bool) {
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = inner.width as usize;
    let prompt = if app.chat.busy { "…" } else { ">" };

    let content = if app.chat.input.is_empty() && !focused {
        Span::styled("Ctrl+L to focus", Style::default().fg(palette.text_faint))
    } else {
        Span::styled(
            app.chat.input.clone(),
            Style::default().fg(palette.text_normal),
        )
    };

    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{prompt} "),
            Style::default().fg(palette.text_accent),
        ),
        content,
    ]))
    .render(inner, frame.buffer_mut());

    if focused && !app.chat.busy {
        let x = inner.x + 2 + app.chat.cursor.min(width.saturating_sub(3)) as u16;
        frame.set_cursor_position((x, inner.y));
    }

    // A second line shows token usage once a turn has run.
    if inner.height > 1 && app.chat.usage.output_tokens > 0 {
        let usage = &app.chat.usage;
        let text = format!(
            "{} in · {} out{}",
            usage.input_tokens,
            usage.output_tokens,
            if usage.cache_read_tokens > 0 {
                format!(" · {} cached", usage.cache_read_tokens)
            } else {
                String::new()
            }
        );
        frame.buffer_mut().set_string(
            inner.x,
            inner.y + 1,
            crate::ui::truncate(&text, width),
            Style::default().fg(palette.text_faint),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;
    use otui_theme::presets;

    fn app() -> (TempVault, App) {
        let vault = TempVault::new("ui-chat");
        vault.write("A.md", "# A\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");
        (vault, app)
    }

    fn palette() -> Palette {
        Palette::from(&presets::default_theme())
    }

    fn text_of(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn user_and_assistant_turns_are_labeled_and_wrapped() {
        let (_v, mut app) = app();
        app.chat.transcript.push(Entry::User("a question".into()));
        app.chat.transcript.push(Entry::Assistant(
            "a fairly long answer that needs wrapping".into(),
        ));

        let lines = text_of(&transcript_lines(&app, &palette(), 20));

        assert_eq!(lines[0], "You");
        assert!(lines.iter().any(|l| l.contains("a question")));
        for line in &lines {
            assert!(line.chars().count() <= 20, "{line:?} overflows");
        }
    }

    #[test]
    fn tool_calls_show_their_status() {
        let (_v, mut app) = app();
        app.chat.transcript.push(Entry::Tool {
            name: "create_note".into(),
            detail: "created Ideas.md".into(),
            status: ToolStatus::Ok,
        });
        app.chat.transcript.push(Entry::Tool {
            name: "read_note".into(),
            detail: "no such note".into(),
            status: ToolStatus::Failed,
        });

        let lines = text_of(&transcript_lines(&app, &palette(), 40));
        assert!(lines
            .iter()
            .any(|l| l.starts_with('✓') && l.contains("create_note")));
        assert!(lines
            .iter()
            .any(|l| l.starts_with('✗') && l.contains("read_note")));
    }

    #[test]
    fn errors_render_in_the_error_color() {
        let (_v, mut app) = app();
        app.chat.transcript.push(Entry::Error("no API key".into()));

        let lines = transcript_lines(&app, &palette(), 40);
        assert_eq!(lines[0].spans[0].style.fg, Some(palette().text_error));
    }

    #[test]
    fn an_empty_transcript_renders_nothing() {
        let (_v, app) = app();
        assert!(transcript_lines(&app, &palette(), 40).is_empty());
    }

    #[test]
    fn multi_line_assistant_text_keeps_its_line_breaks() {
        let (_v, mut app) = app();
        app.chat
            .transcript
            .push(Entry::Assistant("first\nsecond".into()));

        let lines = text_of(&transcript_lines(&app, &palette(), 40));
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1], "second");
    }
}
