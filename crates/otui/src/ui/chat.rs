//! The agent chat panel.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};

use otui_theme::Palette;

use crate::agent::{Entry, ToolStatus};
use crate::app::{App, Focus};
use crate::ui::{pane_block, scrollbar, wrap};

/// Rows reserved for the input box.
const INPUT_HEIGHT: u16 = 3;

/// Tallest the slash-command list gets before it scrolls.
const MAX_COMPLETION_ROWS: u16 = 10;

pub fn draw(frame: &mut Frame, app: &mut App, palette: &Palette, area: Rect) {
    let focused = app.focus == Focus::Chat;
    let title = title(app);
    let block = pane_block(&title, focused, palette, palette.bg_secondary);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(INPUT_HEIGHT)])
        .split(inner);

    draw_transcript(frame, app, palette, rows[0]);
    draw_input(frame, app, palette, rows[1], focused);
    // Drawn last so it sits over the transcript rather than under it.
    if focused {
        draw_completions(frame, app, palette, rows[0]);
    }
}

/// What the panel calls itself.
///
/// The model answering is worth the space: "why is nothing happening" is almost
/// always a missing key or the wrong model, and neither was visible anywhere
/// before you went looking for it.
fn title(app: &App) -> String {
    if app.chat.busy {
        return "Assistant  ·  working…".to_string();
    }
    let provider = &app.config.agent.provider;
    let Some(preset) = otui_agent::catalog::find(provider) else {
        return format!("Assistant  ·  {provider}?");
    };
    if preset.kind == otui_agent::ProviderKind::Offline {
        return "Assistant  ·  no model — /provider".to_string();
    }
    if !crate::agent::ready(app) {
        return format!("Assistant  ·  {} — /key", preset.label);
    }
    format!(
        "Assistant  ·  {} {}",
        preset.label,
        app.config.agent.model()
    )
}

/// The slash-command list, shown while the user is typing one.
///
/// It grows upward from the input box so the command being typed stays put —
/// the list moving under a fixed cursor is easier to read than the reverse.
fn draw_completions(frame: &mut Frame, app: &App, palette: &Palette, area: Rect) {
    if app.chat.busy || !crate::slash::is_command(&app.chat.input) {
        return;
    }
    let matches = crate::slash::completions(&app.chat.input);
    // One exact match with nothing left to choose is not worth a popup.
    if matches.is_empty() || area.height < 2 {
        return;
    }

    let rows = (matches.len() as u16)
        .min(area.height)
        .min(MAX_COMPLETION_ROWS);
    let popup = Rect {
        x: area.x,
        y: area.y + area.height - rows,
        width: area.width,
        height: rows,
    };
    frame.render_widget(Clear, popup);

    // The list is longer than the popup for a bare `/`, so it scrolls to keep
    // the highlight in view rather than letting the arrows walk off the edge.
    let selected = app.chat.completion.min(matches.len() - 1);
    let visible = rows as usize;
    let first = selected
        .saturating_sub(visible - 1)
        .min(matches.len() - visible.min(matches.len()));

    let width = popup.width as usize;
    for (row, command) in matches.iter().skip(first).take(visible).enumerate() {
        let name = match command.argument_hint {
            Some(hint) => format!(" /{} {hint}", command.name),
            None => format!(" /{}", command.name),
        };
        let line = format!("{name:<24}{}", command.description);
        let style = if first + row == selected {
            Style::default()
                .fg(palette.text_accent)
                .bg(palette.bg_active)
        } else {
            Style::default()
                .fg(palette.text_muted)
                .bg(palette.bg_secondary)
        };
        let padded = format!("{line:<width$}");
        frame.buffer_mut().set_string(
            popup.x,
            popup.y + row as u16,
            crate::ui::truncate(&padded, width),
            style,
        );
    }

    if matches.len() > visible {
        let more = format!(" {}/{} ", selected + 1, matches.len());
        let x = popup.x + popup.width.saturating_sub(more.chars().count() as u16 + 1);
        frame.buffer_mut().set_string(
            x,
            popup.y,
            &more,
            Style::default()
                .fg(palette.text_faint)
                .bg(palette.bg_active),
        );
    }
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
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with('✓') && l.contains("create_note"))
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with('✗') && l.contains("read_note"))
        );
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

    /// The rows of the completion popup, top to bottom, as plain text.
    fn popup_rows(app: &App, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, 60, height);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, height))
                .expect("terminal");
        terminal
            .draw(|frame| draw_completions(frame, app, &palette(), area))
            .expect("drawn");

        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .filter(|row| !row.is_empty())
            .collect()
    }

    #[test]
    fn the_panel_says_which_model_is_answering_or_what_is_missing() {
        let (vault, mut app) = app();
        app.auth = crate::auth::Auth::at(vault.path().join("auth.json"));

        crate::actions::set_provider(&mut app, "offline").expect("a known provider");
        assert!(
            title(&app).contains("/provider"),
            "with nothing set up, the title is the instructions: {}",
            title(&app)
        );

        crate::actions::set_provider(&mut app, "anthropic").expect("a known provider");
        if crate::auth::key_for("anthropic", &app.auth).is_none() {
            assert!(
                title(&app).contains("/key"),
                "a provider with no key says so: {}",
                title(&app)
            );
        }

        // A local server needs no key, so it is ready as soon as it is chosen.
        crate::actions::set_provider(&mut app, "ollama").expect("a known provider");
        let ready = title(&app);
        assert!(ready.contains("Ollama"), "{ready}");
        assert!(
            ready.contains(&app.config.agent.model()),
            "and names the model, which is the other half of 'why did that fail': {ready}"
        );

        app.chat.busy = true;
        assert!(
            title(&app).contains("working"),
            "while a turn is running, that is the more useful thing to say"
        );
    }

    #[test]
    fn the_command_list_scrolls_to_keep_the_highlight_in_view() {
        let (_v, mut app) = app();
        app.chat.input = "/".into();
        let total = crate::slash::completions("/").len();
        assert!(
            total > MAX_COMPLETION_ROWS as usize,
            "this test only means something if the list is too long to show at once"
        );

        let first_page = popup_rows(&app, 20);
        assert_eq!(first_page.len(), MAX_COMPLETION_ROWS as usize);
        assert!(
            first_page[0].contains("/help"),
            "starts at the top: {first_page:?}"
        );
        assert!(first_page[0].contains("1/"), "and counts: {first_page:?}");

        // Down to the very last command.
        app.chat.completion = total - 1;
        let last_page = popup_rows(&app, 20);
        assert!(
            last_page.last().is_some_and(|row| row.contains("/quit")),
            "the last command is reachable rather than off the bottom: {last_page:?}"
        );
        assert!(last_page[0].contains(&format!("{total}/{total}")));
    }

    #[test]
    fn the_highlight_is_the_row_the_arrows_landed_on() {
        let (_v, mut app) = app();
        app.chat.input = "/".into();
        app.chat.completion = 2;

        let area = Rect::new(0, 0, 60, 20);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| draw_completions(frame, &app, &palette(), area))
            .expect("drawn");

        let buffer = terminal.backend().buffer().clone();
        let popup_top = 20 - MAX_COMPLETION_ROWS;
        let accent = palette().text_accent;
        let highlighted: Vec<u16> = (popup_top..20)
            .filter(|&y| buffer[(1, y)].style().fg == Some(accent))
            .collect();
        assert_eq!(
            highlighted,
            vec![popup_top + 2],
            "exactly the third row, and only it"
        );
    }

    #[test]
    fn a_pane_with_no_room_draws_no_popup_rather_than_panicking() {
        let (_v, mut app) = app();
        app.chat.input = "/".into();
        assert!(popup_rows(&app, 1).is_empty());
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
