//! The file explorer and the outline/backlinks/tags sidebar.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use otui_theme::Palette;

use crate::app::{App, Focus, Regions, SidePanel};
use crate::explorer::Row;
use crate::ui::{icons, pane_block, scrollbar, truncate};

pub fn draw_explorer(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    let focused = app.focus == Focus::Explorer;
    let title = if app.explorer.filter.is_empty() {
        "Files".to_string()
    } else {
        format!("Files  ⌕ {}", app.explorer.filter)
    };

    let block = pane_block(&title, focused, palette, palette.bg_secondary);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.explorer.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            " this vault has no notes yet",
            Style::default().fg(palette.text_faint),
        )))
        .render(inner, frame.buffer_mut());
        return;
    }

    let height = inner.height as usize;
    app.explorer.scroll_into_view(height);
    let scroll = app.explorer.scroll;
    regions.explorer = Some((inner, scroll));
    let selected = app.explorer.selected;
    let active_note = app.active_note();

    let mut lines = Vec::new();
    for (index, row) in app
        .explorer
        .rows()
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
    {
        let is_cursor = index == selected;
        let indent = "  ".repeat(row.depth());

        // Folders carry a note count on the right; it's the cheapest way to
        // see the shape of a vault you don't know.
        let (icon, name, style, badge) = match row {
            Row::Folder {
                name,
                collapsed,
                count,
                ..
            } => {
                let icon = if *collapsed {
                    icons::FOLDER_CLOSED
                } else {
                    icons::FOLDER_OPEN
                };
                (
                    icon,
                    name.clone(),
                    Style::default()
                        .fg(palette.text_normal)
                        .add_modifier(Modifier::BOLD),
                    if *count > 0 {
                        count.to_string()
                    } else {
                        String::new()
                    },
                )
            }
            Row::Note { id, name, .. } => {
                // The open note is marked with the accent color, the way
                // Obsidian highlights the active file.
                let open = active_note == Some(*id);
                (
                    icons::NOTE,
                    name.clone(),
                    Style::default().fg(if open {
                        palette.text_accent
                    } else {
                        palette.text_muted
                    }),
                    String::new(),
                )
            }
        };

        let background = if is_cursor && focused {
            palette.bg_active
        } else if is_cursor {
            palette.bg_hover
        } else {
            palette.bg_secondary
        };

        let width = inner.width as usize;
        let prefix = format!("{indent}{icon} ");
        // Leave room for the count so a long folder name can't push it off.
        let reserved = prefix.chars().count() + badge.chars().count() + 2;
        let text = truncate(&name, width.saturating_sub(reserved));
        let used = prefix.chars().count() + text.chars().count() + badge.chars().count();

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(palette.text_faint).bg(background),
            ),
            Span::styled(text, style.bg(background)),
            Span::styled(
                " ".repeat(width.saturating_sub(used)),
                Style::default().bg(background),
            ),
            Span::styled(
                badge,
                Style::default().fg(palette.text_faint).bg(background),
            ),
        ]));
    }

    Paragraph::new(lines).render(inner, frame.buffer_mut());
    scrollbar(frame, palette, inner, scroll, app.explorer.len());
}

pub fn draw_sidebar(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    let focused = app.focus == Focus::Sidebar;
    let block = pane_block(
        app.side_panel.title(),
        focused,
        palette,
        palette.bg_secondary,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    draw_panel_tabs(frame, app, palette, rows[0], regions);

    let lines = match app.side_panel {
        SidePanel::Outline => outline_lines(app, palette, rows[1].width as usize),
        SidePanel::Backlinks => backlink_lines(app, palette, rows[1].width as usize),
        SidePanel::Tags => tag_lines(app, palette, rows[1].width as usize),
    };

    let height = rows[1].height as usize;
    let first_visible = app.side_selected.saturating_sub(height.saturating_sub(1));
    regions.sidebar = Some((rows[1], first_visible));
    if app.side_selected >= lines.len() {
        app.side_selected = lines.len().saturating_sub(1);
    }

    // Highlight the selected row so Enter has an obvious target.
    let visible: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(app.side_selected.saturating_sub(height.saturating_sub(1)))
        .take(height)
        .map(|(index, mut line)| {
            if index == app.side_selected && focused {
                for span in &mut line.spans {
                    span.style = span.style.bg(palette.bg_active);
                }
            }
            line
        })
        .collect();

    Paragraph::new(visible).render(rows[1], frame.buffer_mut());
}

fn draw_panel_tabs(
    frame: &mut Frame,
    app: &App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    let mut spans = Vec::new();
    let mut x = area.x;
    for panel in [SidePanel::Outline, SidePanel::Backlinks, SidePanel::Tags] {
        let active = app.side_panel == panel;
        let width = panel.title().chars().count() as u16 + 2;
        regions.side_tabs.push((
            Rect {
                x,
                y: area.y,
                width,
                height: 1,
            },
            panel,
        ));
        x += width;
        spans.push(Span::styled(
            format!(" {} ", panel.title()),
            Style::default()
                .fg(if active {
                    palette.text_accent
                } else {
                    palette.text_faint
                })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    Paragraph::new(Line::from(spans)).render(area, frame.buffer_mut());
}

fn outline_lines(app: &App, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let Some(id) = app.active_note() else {
        return vec![empty_hint("No note open", palette)];
    };
    let Some(note) = app.index.note(id) else {
        return Vec::new();
    };
    if note.headings.is_empty() {
        return vec![empty_hint("No headings", palette)];
    }

    note.headings
        .iter()
        .map(|heading| {
            let indent = "  ".repeat(heading.level.saturating_sub(1) as usize);
            Line::from(Span::styled(
                truncate(&format!("{indent}{}", heading.text), width),
                Style::default().fg(palette.heading(heading.level)),
            ))
        })
        .collect()
}

fn backlink_lines(app: &App, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let Some(id) = app.active_note() else {
        return vec![empty_hint("No note open", palette)];
    };
    let backlinks = app.index.backlinks(id);
    if backlinks.is_empty() {
        return vec![empty_hint("Nothing links here yet", palette)];
    }

    let mut lines = Vec::new();
    for backlink in backlinks {
        let Some(source) = app.index.note(backlink.source) else {
            continue;
        };
        lines.push(Line::from(Span::styled(
            truncate(&source.meta.title, width),
            Style::default()
                .fg(palette.link)
                .add_modifier(Modifier::BOLD),
        )));
        // The line the link sits on is the context that makes a backlink useful.
        lines.push(Line::from(Span::styled(
            truncate(&format!("  {}", backlink.context), width),
            Style::default().fg(palette.text_faint),
        )));
    }
    lines
}

fn tag_lines(app: &App, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let tags = app.index.tags();
    if tags.is_empty() {
        return vec![empty_hint("No tags in this vault", palette)];
    }

    tags.iter()
        .map(|(tag, notes)| {
            let depth = tag.matches('/').count();
            let leaf = tag.rsplit('/').next().unwrap_or(tag);
            let label = format!("{}#{leaf}", "  ".repeat(depth));
            let count = notes.len().to_string();
            let pad = width.saturating_sub(label.chars().count() + count.chars().count() + 1);
            Line::from(vec![
                Span::styled(label, Style::default().fg(palette.tag_fg)),
                Span::raw(" ".repeat(pad)),
                Span::styled(count, Style::default().fg(palette.text_faint)),
            ])
        })
        .collect()
}

fn empty_hint(text: &str, palette: &Palette) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text}"),
        Style::default().fg(palette.text_faint),
    ))
}

/// Rows the sidebar currently shows, so key handling can act on the selection.
#[must_use]
pub fn sidebar_targets(app: &App) -> Vec<SidebarTarget> {
    match app.side_panel {
        SidePanel::Outline => app
            .active_note()
            .and_then(|id| app.index.note(id))
            .map(|note| {
                note.headings
                    .iter()
                    .map(|h| SidebarTarget::Heading(note.body_offset + h.line))
                    .collect()
            })
            .unwrap_or_default(),
        SidePanel::Backlinks => app
            .active_note()
            .map(|id| {
                app.index
                    .backlinks(id)
                    .iter()
                    // Two rendered lines per backlink: title then context.
                    .flat_map(|b| [SidebarTarget::Note(b.source), SidebarTarget::Note(b.source)])
                    .collect()
            })
            .unwrap_or_default(),
        SidePanel::Tags => app
            .index
            .tags()
            .keys()
            .map(|tag| SidebarTarget::Tag(tag.clone()))
            .collect(),
    }
}

/// What activating a sidebar row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarTarget {
    /// Scroll the note to a file line.
    Heading(usize),
    Note(otui_core::index::NoteId),
    Tag(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use otui_core::test_support::TempVault;
    use otui_theme::presets;

    fn app_with(content: &str) -> (TempVault, App) {
        let vault = TempVault::new("panes");
        vault.write("A.md", content);
        vault.write("B.md", "links to [[A]]\n");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let a = app.index.id_of_rel("A.md").unwrap();
        app.open_note(a);
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
    fn outline_lists_headings_indented_by_level() {
        let (_v, app) = app_with("# One\n\n## Two\n\n### Three\n");
        let lines = text_of(&outline_lines(&app, &palette(), 40));

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "One");
        assert_eq!(lines[1], "  Two");
        assert_eq!(lines[2], "    Three");
    }

    #[test]
    fn outline_targets_point_at_real_file_lines() {
        let (_v, app) = app_with("---\ntitle: T\n---\n# One\n\n## Two\n");
        let targets = sidebar_targets(&app);

        // Frontmatter occupies lines 0-2, so "# One" is file line 3.
        assert_eq!(targets[0], SidebarTarget::Heading(3));
        assert_eq!(targets[1], SidebarTarget::Heading(5));
    }

    #[test]
    fn backlinks_show_the_source_and_its_context() {
        let (_v, app) = app_with("# A\n");
        let lines = text_of(&backlink_lines(&app, &palette(), 60));

        assert_eq!(lines[0], "B");
        assert!(lines[1].contains("links to [[A]]"));
    }

    #[test]
    fn empty_panels_explain_themselves() {
        let (_v, app) = app_with("just prose, no headings\n");
        let lines = text_of(&outline_lines(&app, &palette(), 40));
        assert!(lines[0].contains("No headings"), "got {lines:?}");

        let vault = TempVault::new("panes-empty");
        vault.write("Solo.md", "no links\n");
        let mut app = App::new(vault.vault(), Config::default()).expect("app");
        let solo = app.index.id_of_rel("Solo.md").unwrap();
        app.open_note(solo);

        let lines = text_of(&backlink_lines(&app, &palette(), 40));
        assert!(lines[0].contains("Nothing links here"));
    }

    #[test]
    fn tags_are_listed_with_counts() {
        let vault = TempVault::new("panes-tags");
        vault.write("A.md", "#project/alpha\n");
        vault.write("B.md", "#project/alpha\n");
        let app = App::new(vault.vault(), Config::default()).expect("app");

        let lines = text_of(&tag_lines(&app, &palette(), 30));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#project") && l.contains('2'))
        );
        assert!(lines.iter().any(|l| l.contains("#alpha")));
    }

    #[test]
    fn sidebar_targets_line_up_with_rendered_backlink_rows() {
        let (_v, mut app) = app_with("# A\n");
        app.side_panel = SidePanel::Backlinks;
        let rendered = backlink_lines(&app, &palette(), 60);
        let targets = sidebar_targets(&app);

        assert_eq!(
            rendered.len(),
            targets.len(),
            "every rendered row needs a target or Enter picks the wrong one"
        );
    }
}
