//! The note pane: tab bar, reading mode and the editor.
//!
//! Reading mode renders the markdown model into styled lines the way Obsidian's
//! preview does — headings colored by level, wikilinks in the accent color and
//! unresolved ones dimmed, callouts with a tinted bar, code blocks on their own
//! background. Editing mode shows the source with a line-number gutter.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui_image::sliced::{SignedPosition, SlicedImage};

use otui_core::excalidraw;
use otui_core::index::VaultIndex;
use otui_core::markdown::{self, Align, Block, BlockKind, Marker, SpanKind, Table};
use otui_theme::Palette;

use crate::app::{App, Mode, Regions};
use crate::editor::{Cursor, Row};
use crate::images::{self, Images};
use crate::ui::{drawing, icons, scrollbar, truncate};

/// Left padding inside the note pane, matching Obsidian's generous margins.
const PADDING: u16 = 2;

/// Columns reserved for the line-number gutter when it is switched on.
///
/// Fixed rather than sized to the note, and reserved while *reading* too, where
/// it is left blank. Both modes then wrap the same prose to the same width in
/// the same column, which is what makes `Ctrl+E` restyle the page instead of
/// reflowing it. A note past 999 lines spends the separating space on a fourth
/// digit rather than moving the text.
const GUTTER: u16 = 4;

/// Widest a single table column may be drawn, in characters.
///
/// Tables are laid out at their content's width and panned across, so this is
/// only here to stop one cell of prose from pushing every column after it out
/// of reach.
const MAX_COLUMN: usize = 40;

/// Everything the markdown renderer needs beyond the blocks themselves.
///
/// Bundled because pictures made the parameter list unwieldy: laying one out
/// needs the vault to find the file, the note's own folder to resolve a
/// relative path against, and the terminal's font size to know how many rows
/// it will fill.
pub struct Ctx<'a> {
    pub palette: &'a Palette,
    pub index: &'a VaultIndex,
    /// The folder of the note being read, where a relative `![](chart.png)` is
    /// looked for first.
    pub note_dir: Option<PathBuf>,
    /// `None` renders every picture as its alt text, which is what happens in
    /// a terminal that cannot draw and in tests.
    pub images: Option<&'a mut Images>,
    /// Where each picture ended up, filled in as blocks are laid out.
    pub pictures: Vec<Picture>,
    /// Which rendered row each block's source line ended up on, filled in as
    /// blocks are laid out.
    ///
    /// A source line and a rendered row are not the same number — prose wraps,
    /// headings gain a rule, a picture takes a dozen rows — so scrolling to a
    /// heading the outline points at needs the translation written down.
    pub anchors: Vec<(usize, usize)>,
}

impl<'a> Ctx<'a> {
    /// A context that renders text only.
    #[cfg(test)]
    #[must_use]
    pub fn text(palette: &'a Palette, index: &'a VaultIndex) -> Self {
        Self {
            palette,
            index,
            note_dir: None,
            images: None,
            pictures: Vec::new(),
            anchors: Vec::new(),
        }
    }
}

/// A picture and the rows reserved for it.
pub struct Picture {
    /// Index of its first row in the rendered lines.
    pub line: usize,
    /// Columns from the left edge of the pane.
    pub indent: u16,
    pub size: Size,
    pub path: PathBuf,
}

pub fn draw(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    draw_tab_bar(frame, app, palette, rows[0], regions);

    let body = Rect {
        x: rows[1].x + PADDING,
        y: rows[1].y,
        width: rows[1].width.saturating_sub(PADDING * 2),
        height: rows[1].height,
    };

    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(palette.bg_primary)),
        rows[1],
    );

    if app.active().is_none() {
        draw_empty_state(frame, palette, body);
        return;
    }

    // Reserved in both modes so the prose column never moves between them.
    let gutter = if app.config.ui.line_numbers {
        GUTTER.min(body.width)
    } else {
        0
    };

    match app.active().map(|t| t.mode) {
        Some(Mode::Reading) => draw_reading(frame, app, palette, body, gutter, regions),
        Some(Mode::Editing) => draw_editing(frame, app, palette, body, gutter, regions),
        None => {}
    }
}

/// The prose column inside the note pane: the gutter and the scrollbar's column
/// taken off.
fn prose_area(area: Rect, gutter: u16) -> Rect {
    Rect {
        x: area.x + gutter,
        y: area.y,
        width: area.width.saturating_sub(gutter),
        height: area.height,
    }
}

/// Width and height the editor's text was last drawn at.
///
/// Key handling needs it to move between wrapped rows, and reads it from the
/// last frame rather than recomputing the layout arithmetic a second time.
#[must_use]
pub fn edit_viewport(app: &App) -> (usize, usize) {
    app.regions.editor.map_or((0, 0), |(rect, _)| {
        (rect.width as usize, rect.height as usize)
    })
}

fn draw_tab_bar(
    frame: &mut Frame,
    app: &App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(palette.tab_bar_bg)),
        area,
    );
    if app.tabs.is_empty() {
        return;
    }

    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, tab) in app.tabs.iter().enumerate() {
        let active = app.active_tab == Some(index);
        let title = app.note_title(tab.note);
        let title = truncate(&title, 22);

        // `" {title}"` plus the two-column modified marker.
        let width = title.chars().count() as u16 + 3;
        regions.tabs.push((
            Rect {
                x,
                y: area.y,
                width,
                height: 1,
            },
            index,
        ));
        x += width;

        let style = if active {
            Style::default()
                .bg(palette.tab_active_bg)
                .fg(palette.tab_active_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .bg(palette.tab_bar_bg)
                .fg(palette.tab_inactive_fg)
        };

        spans.push(Span::styled(format!(" {title}"), style));
        // Obsidian marks an unsaved tab with a dot where the close button goes.
        spans.push(Span::styled(
            if tab.is_modified() {
                format!(" {} ", icons::MODIFIED)
            } else {
                "  ".to_string()
            },
            style.fg(if tab.is_modified() {
                palette.tab_modified
            } else {
                palette.tab_inactive_fg
            }),
        ));
    }

    Paragraph::new(Line::from(spans)).render(area, frame.buffer_mut());
}

fn draw_empty_state(frame: &mut Frame, palette: &Palette, area: Rect) {
    let hints = [
        ("Ctrl+O", "open a note"),
        ("Ctrl+P", "command palette"),
        ("Ctrl+N", "new note"),
        ("Ctrl+G", "graph view"),
        ("Ctrl+L", "assistant"),
        ("?", "all shortcuts"),
    ];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "emeraldian",
            Style::default()
                .fg(palette.text_accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for (key, description) in hints {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<10}"),
                Style::default().fg(palette.text_accent),
            ),
            Span::styled(description, Style::default().fg(palette.text_muted)),
        ]));
    }

    Paragraph::new(lines).render(area, frame.buffer_mut());
}

// ---------------------------------------------------------------------------
// Reading mode
// ---------------------------------------------------------------------------

fn draw_reading(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    gutter: u16,
    regions: &mut Regions,
) {
    let Some(id) = app.active_note() else { return };
    let Ok(content) = app.index.read(id) else {
        Paragraph::new("could not read this note").render(area, frame.buffer_mut());
        return;
    };
    // The scrollbar stays on the pane's edge; the prose sits inside the gutter.
    let full = area;
    let area = prose_area(area, gutter);

    // An Excalidraw note is a wrapper around a scene: its Markdown is a warning
    // banner and a wall of compressed base64, and the drawing is the content.
    // Judged by the file name, so an ordinary note that happens to have a
    // `## Drawing` heading is still read as prose.
    let is_drawing = app
        .index
        .note(id)
        .is_some_and(|note| excalidraw::is_drawing(&note.meta.rel));
    if is_drawing {
        let scroll = app.active().map_or(0, |tab| tab.scroll);
        if let Some(scene) = app.scenes.get(&content) {
            let rows = drawing::draw(frame, palette, area, scene, scroll);
            let max_scroll = rows.saturating_sub(area.height as usize);
            if let Some(tab) = app.active_mut() {
                tab.scroll = tab.scroll.min(max_scroll);
            }
            return;
        }
    }

    let width = area.width.saturating_sub(1) as usize;
    let document = markdown::parse(&content);

    let note_dir = app
        .index
        .note(id)
        .map(|note| app.index.vault.path.join(&note.meta.rel))
        .and_then(|path| path.parent().map(Path::to_path_buf));
    // Pictures are capped at a share of the pane, so the layout pass has to
    // know how tall it is before it measures the first one.
    app.images.set_pane_height(area.height);
    let mut ctx = Ctx {
        palette,
        index: &app.index,
        note_dir,
        images: Some(&mut app.images),
        pictures: Vec::new(),
        anchors: Vec::new(),
    };
    let lines = render_document(&document, &mut ctx, width);
    let pictures = std::mem::take(&mut ctx.pictures);
    regions.anchors = std::mem::take(&mut ctx.anchors);

    let height = area.height as usize;
    let max_scroll = lines.len().saturating_sub(height);
    // Content can be wider than the pane — a table laid out at its natural
    // width, a long code line — and panning is the only way to reach the rest.
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    let max_hscroll = widest.saturating_sub(area.width as usize);
    if let Some(tab) = app.active_mut() {
        tab.scroll = tab.scroll.min(max_scroll);
        tab.hscroll = tab.hscroll.min(max_hscroll);
    }
    let (scroll, hscroll) = app.active().map_or((0, 0), |t| (t.scroll, t.hscroll));

    let visible: Vec<Line> = lines.iter().skip(scroll).take(height).cloned().collect();
    Paragraph::new(visible)
        .scroll((0, u16::try_from(hscroll).unwrap_or(u16::MAX)))
        .render(area, frame.buffer_mut());
    draw_pictures(
        frame,
        &mut app.images,
        palette,
        area,
        &pictures,
        scroll,
        hscroll,
    );
    scrollbar(frame, palette, full, scroll, lines.len());
}

/// Draws the pictures whose rows are on screen, over the blanks left for them.
fn draw_pictures(
    frame: &mut Frame,
    images: &mut Images,
    palette: &Palette,
    area: Rect,
    pictures: &[Picture],
    scroll: usize,
    hscroll: usize,
) {
    for picture in pictures {
        // Where the top of the picture sits relative to the pane, which is
        // above it — a negative row — once it is partly scrolled off.
        let Ok(top) = i32::try_from(picture.line).map(|line| line - scroll as i32) else {
            continue;
        };
        if top >= i32::from(area.height) || top + i32::from(picture.size.height) <= 0 {
            continue;
        }
        let Ok(top) = i16::try_from(top) else {
            continue;
        };
        // Pictures pan with the text around them, but only while they stay
        // fully inside the pane: the graphics protocols slice by row, not by
        // column, so a picture that has run off the left edge would be redrawn
        // squashed against it rather than clipped. Dropping it is the honest
        // answer, and panning is for reaching a wide table anyway — a picture
        // pinned over that table is exactly what's in the way.
        let left = i32::from(picture.indent) - hscroll as i32;
        if left < 0 || left >= i32::from(area.width) {
            continue;
        }
        let Ok(left) = i16::try_from(left) else {
            continue;
        };

        match images.get(&picture.path, picture.size) {
            Some(protocol) => {
                let position = SignedPosition::from((left, top));
                frame.render_widget(SlicedImage::new(protocol, position), area);
            }
            // Still encoding. The rows are already the right size, so the
            // picture appears without moving anything around it.
            None => {
                let row = area.y as i32 + i32::from(top).max(0);
                let column = area.x as i32 + i32::from(left).max(0);
                if let (Ok(y), Ok(x)) = (u16::try_from(row), u16::try_from(column)) {
                    let placeholder = Rect {
                        x: x.min(area.x + area.width),
                        y,
                        width: picture.size.width.min(area.width),
                        height: 1,
                    };
                    Paragraph::new(Line::from(Span::styled(
                        format!("{} …", icons::IMAGE),
                        Style::default().fg(palette.text_faint),
                    )))
                    .render(placeholder.intersection(area), frame.buffer_mut());
                }
            }
        }
    }
}

/// Turns a parsed document into styled terminal lines.
///
/// Pictures are laid out here too, as runs of blank lines with their positions
/// collected in `ctx.pictures`, so that scrolling and the scrollbar count them
/// like any other content.
pub fn render_document(
    document: &markdown::Document,
    ctx: &mut Ctx,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in &document.blocks {
        ctx.anchors.push((block.line, lines.len()));
        render_block(block, ctx, width, 0, &mut lines);
    }
    lines
}

/// The rendered row a source line was drawn on.
///
/// Only block starts are recorded, so a line inside a block resolves to the row
/// its block began on — which is what scrolling to a heading wants anyway.
#[must_use]
pub fn anchor_row(anchors: &[(usize, usize)], line: usize) -> usize {
    anchors
        .iter()
        .take_while(|(source, _)| *source <= line)
        .last()
        .map_or(line, |(_, row)| *row)
}

fn render_block(
    block: &Block,
    ctx: &mut Ctx,
    width: usize,
    indent: usize,
    out: &mut Vec<Line<'static>>,
) {
    let palette = ctx.palette;
    let pad = " ".repeat(indent);
    let inner_width = width.saturating_sub(indent).max(8);

    match &block.kind {
        BlockKind::Blank => out.push(Line::from("")),

        BlockKind::Frontmatter(entries) => {
            for (key, value) in entries {
                out.push(Line::from(vec![
                    Span::styled(
                        format!("{pad}{key}: "),
                        Style::default().fg(palette.frontmatter_key),
                    ),
                    Span::styled(
                        value.clone(),
                        Style::default().fg(palette.frontmatter_value),
                    ),
                ]));
            }
            out.push(Line::from(Span::styled(
                "─".repeat(width.min(60)),
                Style::default().fg(palette.hr),
            )));
            out.push(Line::from(""));
        }

        BlockKind::Heading { level, spans } => {
            let style = Style::default()
                .fg(palette.heading(*level))
                .add_modifier(Modifier::BOLD);
            let text: String = spans.iter().map(|s| s.text.as_str()).collect();
            out.push(Line::from(Span::styled(format!("{pad}{text}"), style)));
            // Obsidian underlines H1 and H2; a rule is the terminal equivalent
            // of the larger type it uses to separate sections.
            if *level <= 2 {
                out.push(Line::from(Span::styled(
                    format!(
                        "{pad}{}",
                        "─".repeat(inner_width.min(text.chars().count() + 8))
                    ),
                    Style::default().fg(palette.border),
                )));
            }
        }

        BlockKind::Paragraph(spans) => {
            render_spans(spans, ctx, inner_width, indent, &pad, &pad, out);
        }

        BlockKind::ListItem {
            depth,
            marker,
            spans,
        } => {
            let list_indent = " ".repeat(indent + depth * 2);
            let (glyph, glyph_style) = match marker {
                Marker::Bullet => (
                    icons::BULLET.to_string(),
                    Style::default().fg(palette.list_marker),
                ),
                Marker::Ordered(n) => (format!("{n}."), Style::default().fg(palette.list_marker)),
                Marker::Task(true) => (
                    icons::TASK_DONE.to_string(),
                    Style::default().fg(palette.checkbox_done),
                ),
                Marker::Task(false) => (
                    icons::TASK_TODO.to_string(),
                    Style::default().fg(palette.checkbox_todo),
                ),
            };

            let prefix_width = glyph.chars().count() + 1;
            let continuation = format!("{list_indent}{}", " ".repeat(prefix_width));
            let text_indent = indent + depth * 2 + prefix_width;

            let mark = ctx.pictures.len();
            let mut wrapped = Vec::new();
            render_spans(
                spans,
                ctx,
                inner_width.saturating_sub(depth * 2 + prefix_width),
                text_indent,
                "",
                "",
                &mut wrapped,
            );
            // Completed tasks are struck through, as in Obsidian.
            if matches!(marker, Marker::Task(true)) {
                for line in &mut wrapped {
                    for span in &mut line.spans {
                        span.style = span
                            .style
                            .add_modifier(Modifier::CROSSED_OUT)
                            .fg(palette.text_faint);
                    }
                }
            }
            shift(&mut ctx.pictures[mark..], out.len(), 0);

            if wrapped.is_empty() {
                wrapped.push(Line::from(""));
            }
            for (i, line) in wrapped.into_iter().enumerate() {
                let mut spans = Vec::new();
                if i == 0 {
                    spans.push(Span::raw(list_indent.clone()));
                    spans.push(Span::styled(format!("{glyph} "), glyph_style));
                } else {
                    spans.push(Span::raw(continuation.clone()));
                }
                spans.extend(line.spans);
                out.push(Line::from(spans));
            }
        }

        BlockKind::Quote(body) => {
            let mark = ctx.pictures.len();
            let mut inner = Vec::new();
            for child in body {
                render_block(child, ctx, width.saturating_sub(2), 0, &mut inner);
            }
            // Every inner line gains a quote bar and lands after what is
            // already in `out`, so anything laid out inside moves with it.
            shift(&mut ctx.pictures[mark..], out.len(), indent as u16 + 2);
            for line in inner {
                let mut spans = vec![
                    Span::raw(pad.clone()),
                    Span::styled(
                        format!("{} ", icons::QUOTE_BAR),
                        Style::default().fg(palette.quote_bar),
                    ),
                ];
                spans.extend(line.spans.into_iter().map(|mut span| {
                    span.style = span.style.fg(palette.quote_fg);
                    span
                }));
                out.push(Line::from(spans));
            }
        }

        BlockKind::Callout { kind, title, body } => {
            let color = palette.callout(kind);
            let label = if title.is_empty() {
                capitalize(kind)
            } else {
                title.iter().map(|s| s.text.as_str()).collect()
            };

            out.push(Line::from(vec![
                Span::raw(pad.clone()),
                Span::styled(format!("{} ", icons::QUOTE_BAR), Style::default().fg(color)),
                Span::styled(
                    label,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));

            let mark = ctx.pictures.len();
            let mut inner = Vec::new();
            for child in body {
                render_block(child, ctx, width.saturating_sub(2), 0, &mut inner);
            }
            shift(&mut ctx.pictures[mark..], out.len(), indent as u16 + 2);
            for line in inner {
                let mut spans = vec![
                    Span::raw(pad.clone()),
                    Span::styled(format!("{} ", icons::QUOTE_BAR), Style::default().fg(color)),
                ];
                spans.extend(line.spans);
                out.push(Line::from(spans));
            }
        }

        BlockKind::Code { lang, lines: code } => {
            let background = Style::default().bg(palette.code_bg);
            if !lang.is_empty() {
                out.push(Line::from(Span::styled(
                    format!("{pad} {lang} "),
                    background.fg(palette.text_faint),
                )));
            }
            for line in code {
                let mut spans = vec![Span::styled(format!("{pad} "), background)];
                spans.extend(highlight(line, lang, palette, background));
                // Pad to the pane width so the block reads as a filled panel.
                let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                if used < width {
                    spans.push(Span::styled(" ".repeat(width - used), background));
                }
                out.push(Line::from(spans));
            }
        }

        BlockKind::Table(table) => render_table(table, palette, &pad, out),

        BlockKind::Rule => out.push(Line::from(Span::styled(
            format!("{pad}{}", "─".repeat(inner_width)),
            Style::default().fg(palette.hr),
        ))),
    }
}

/// Lays out inline spans, lifting any picture out onto rows of its own.
///
/// A picture is a block in a line of text: the words around it wrap as usual
/// above and below, and it gets however many rows it needs in between. When it
/// can't be drawn — no support in the terminal, a missing file, a link to the
/// web — nothing is lifted out and it stays inline as its alt text.
#[allow(clippy::too_many_arguments)]
fn render_spans(
    spans: &[markdown::Span],
    ctx: &mut Ctx,
    width: usize,
    indent: usize,
    first_pad: &str,
    rest_pad: &str,
    out: &mut Vec<Line<'static>>,
) {
    let mut run: Vec<markdown::Span> = Vec::new();
    for span in spans {
        let Some((path, size)) = measure_picture(span, ctx, width) else {
            run.push(span.clone());
            continue;
        };

        if !run.is_empty() {
            let pad = if out.is_empty() { first_pad } else { rest_pad };
            let styled = style_spans(&run, ctx.palette, ctx.index);
            out.extend(wrap_spans(&styled, width, pad, rest_pad));
            run.clear();
        }

        ctx.pictures.push(Picture {
            line: out.len(),
            indent: indent as u16,
            size,
            path,
        });
        // The picture is drawn over these rows once it has been encoded. They
        // are blank so that the text below sits where it will finally sit, and
        // so a half-scrolled picture leaves no debris behind.
        out.extend(std::iter::repeat_n(Line::from(""), size.height as usize));
    }

    if !run.is_empty() || out.is_empty() {
        let pad = if out.is_empty() { first_pad } else { rest_pad };
        let styled = style_spans(&run, ctx.palette, ctx.index);
        out.extend(wrap_spans(&styled, width, pad, rest_pad));
    }
}

/// Works out where a span's picture is and how much room it needs.
fn measure_picture(span: &markdown::Span, ctx: &mut Ctx, width: usize) -> Option<(PathBuf, Size)> {
    let (target, embed) = match &span.kind {
        SpanKind::Image { src } => (src, false),
        // `![[x]]` embeds a note as often as a picture, and only the vault
        // knows which; a note embed finds no image file and stays a link.
        SpanKind::WikiLink {
            target,
            embed: true,
            ..
        } => (target, true),
        _ => return None,
    };

    let path = ctx
        .index
        .attachment_path(target, ctx.note_dir.as_deref())
        .filter(|path| images::is_image(path))?;

    let images = ctx.images.as_deref_mut()?;
    let mut room = Size::new(u16::try_from(width).unwrap_or(u16::MAX), u16::MAX);
    // Obsidian sizes an embed with `![[chart.png|400]]`, in pixels. Anything
    // else after the pipe is an alias, which is only ever alt text.
    if let Some(pixels) = embed.then(|| width_hint(&span.text)).flatten() {
        room.width = room.width.min(images.cells_wide(pixels)?);
    }

    let size = images.measure(&path, room)?;
    (size.width > 0 && size.height > 0).then_some((path, size))
}

/// Reads Obsidian's `|400` or `|400x300` embed size, in pixels.
fn width_hint(alias: &str) -> Option<u32> {
    let width = alias.split_once('x').map_or(alias, |(width, _)| width);
    width.trim().parse().ok().filter(|&pixels| pixels > 0)
}

/// Moves pictures laid out in a nested buffer to where that buffer landed.
fn shift(pictures: &mut [Picture], lines: usize, indent: u16) {
    for picture in pictures {
        picture.line += lines;
        picture.indent += indent;
    }
}

/// Applies theme colors to parsed inline spans.
fn style_spans(
    spans: &[markdown::Span],
    palette: &Palette,
    index: &otui_core::index::VaultIndex,
) -> Vec<(String, Style)> {
    spans
        .iter()
        .map(|span| {
            let mut style = Style::default().fg(palette.text_normal);

            match &span.kind {
                SpanKind::WikiLink { target, .. } => {
                    // A link to a note that doesn't exist yet is dimmed rather
                    // than hidden — that's the signal to go write it.
                    style = if index.resolve(target).is_some() {
                        style.fg(palette.link).add_modifier(Modifier::UNDERLINED)
                    } else {
                        style.fg(palette.link_unresolved)
                    };
                }
                SpanKind::Link { .. } => {
                    style = style
                        .fg(palette.link_external)
                        .add_modifier(Modifier::UNDERLINED);
                }
                // Reached only when the picture could not be drawn, so this is
                // the alt text standing in for it.
                SpanKind::Image { .. } => style = style.fg(palette.text_faint),
                SpanKind::Tag(_) => {
                    style = style.fg(palette.tag_fg).bg(palette.tag_bg);
                }
                SpanKind::Math => style = style.fg(palette.text_accent),
                SpanKind::Text => {}
            }

            (span.text.clone(), emphasis(style, span.style, palette))
        })
        .collect()
}

/// Wraps styled spans to `width`, breaking on spaces and keeping styles intact.
#[must_use]
pub fn wrap_spans(
    spans: &[(String, Style)],
    width: usize,
    first_indent: &str,
    indent: &str,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Span> = vec![Span::raw(first_indent.to_string())];
    let mut used = first_indent.chars().count();

    for (text, style) in spans {
        // Splitting inclusively keeps the space attached to the preceding word,
        // so styles don't fragment on every gap.
        for word in text.split_inclusive(' ') {
            let word_width = word.chars().count();

            if used + word_width > width && used > indent.chars().count() {
                lines.push(Line::from(std::mem::take(&mut current)));
                current.push(Span::raw(indent.to_string()));
                used = indent.chars().count();
                // A wrapped line shouldn't start with the space that ended the
                // previous one.
                let trimmed = word.trim_start();
                if trimmed.is_empty() {
                    continue;
                }
                current.push(Span::styled(trimmed.to_string(), *style));
                used += trimmed.chars().count();
                continue;
            }

            current.push(Span::styled(word.to_string(), *style));
            used += word_width;
        }
    }

    lines.push(Line::from(current));
    lines
}

fn render_table(table: &Table, palette: &Palette, pad: &str, out: &mut Vec<Line<'static>>) {
    let columns = table
        .header
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return;
    }

    // Size columns to their content.
    //
    // A table wider than the pane is left wide and scrolled across
    // horizontally, rather than squeezed to fit: dividing the pane between
    // eight columns leaves three characters each, which is not a narrow table
    // but an unreadable one. The only limit is a ceiling on a single runaway
    // column, so one essay-length cell can't push the rest off the far side.
    let mut widths = vec![0usize; columns];
    let cell_text = |cells: &Vec<Vec<markdown::Span>>, i: usize| -> String {
        cells
            .get(i)
            .map(|spans| markdown::spans_to_text(spans))
            .unwrap_or_default()
    };

    for (i, width) in widths.iter_mut().enumerate() {
        *width = (*width).max(cell_text(&table.header, i).chars().count());
        for row in &table.rows {
            *width = (*width).max(cell_text(row, i).chars().count());
        }
        *width = (*width).clamp(1, MAX_COLUMN);
    }

    let border = Style::default().fg(palette.table_border);
    let rule = |left: &str, mid: &str, right: &str| -> Line<'static> {
        let mut text = String::from(left);
        for (i, w) in widths.iter().enumerate() {
            text.push_str(&"─".repeat(w + 2));
            text.push_str(if i + 1 == widths.len() { right } else { mid });
        }
        Line::from(vec![Span::raw(pad.to_string()), Span::styled(text, border)])
    };

    let render_row = |cells: &Vec<Vec<markdown::Span>>, header: bool| -> Line<'static> {
        let mut spans = vec![Span::raw(pad.to_string()), Span::styled("│", border)];
        for (i, w) in widths.iter().enumerate() {
            let text = truncate(&cell_text(cells, i), *w);
            let padding = w.saturating_sub(text.chars().count());
            let (before, after) = match table.aligns.get(i).copied().unwrap_or(Align::Left) {
                Align::Left => (0, padding),
                Align::Right => (padding, 0),
                Align::Center => (padding / 2, padding - padding / 2),
            };
            let style = if header {
                Style::default()
                    .fg(palette.table_header)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text_normal)
            };
            spans.push(Span::raw(format!(" {}", " ".repeat(before))));
            spans.push(Span::styled(text, style));
            spans.push(Span::raw(format!("{} ", " ".repeat(after))));
            spans.push(Span::styled("│", border));
        }
        Line::from(spans)
    };

    out.push(rule("┌", "┬", "┐"));
    out.push(render_row(&table.header, true));
    out.push(rule("├", "┼", "┤"));
    for row in &table.rows {
        out.push(render_row(row, false));
    }
    out.push(rule("└", "┴", "┘"));
}

/// Lightweight syntax highlighting for fenced code.
///
/// Keyword, string, comment and number recognition covers what makes code
/// readable at a glance. A full grammar per language would be a large
/// dependency for a feature used a few lines at a time.
fn highlight(line: &str, lang: &str, palette: &Palette, base: Style) -> Vec<Span<'static>> {
    let keywords = keywords_for(lang);
    let comment = comment_prefix(lang);

    if let Some(prefix) = comment
        && line.trim_start().starts_with(prefix)
    {
        return vec![Span::styled(line.to_string(), base.fg(palette.syn_comment))];
    }

    let mut spans = Vec::new();
    let mut current = String::new();
    let mut in_string: Option<char> = None;

    macro_rules! flush {
        ($style:expr) => {
            if !current.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut current), $style));
            }
        };
    }

    for ch in line.chars() {
        if let Some(quote) = in_string {
            current.push(ch);
            if ch == quote {
                flush!(base.fg(palette.syn_string));
                in_string = None;
            }
            continue;
        }

        if ch == '"' || ch == '\'' || ch == '`' {
            flush!(base.fg(palette.text_normal));
            current.push(ch);
            in_string = Some(ch);
            continue;
        }

        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
            continue;
        }

        // A word boundary: classify what we collected.
        if !current.is_empty() {
            let style = if keywords.contains(&current.as_str()) {
                base.fg(palette.syn_keyword).add_modifier(Modifier::BOLD)
            } else if current.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                base.fg(palette.syn_number)
            } else if ch == '(' {
                base.fg(palette.syn_function)
            } else if current.chars().next().is_some_and(char::is_uppercase) {
                base.fg(palette.syn_type)
            } else {
                base.fg(palette.text_normal)
            };
            flush!(style);
        }
        spans.push(Span::styled(ch.to_string(), base.fg(palette.syn_punct)));
    }

    if !current.is_empty() {
        let style = if in_string.is_some() {
            base.fg(palette.syn_string)
        } else if keywords.contains(&current.as_str()) {
            base.fg(palette.syn_keyword).add_modifier(Modifier::BOLD)
        } else if current.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            base.fg(palette.syn_number)
        } else {
            base.fg(palette.text_normal)
        };
        spans.push(Span::styled(current, style));
    }

    spans
}

fn keywords_for(lang: &str) -> &'static [&'static str] {
    const RUST: &[&str] = &[
        "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for", "while",
        "loop", "if", "else", "match", "return", "self", "Self", "const", "static", "async",
        "await", "move", "where", "type", "as", "in", "ref", "dyn", "crate", "true", "false",
    ];
    const PYTHON: &[&str] = &[
        "def", "class", "import", "from", "return", "if", "elif", "else", "for", "while", "try",
        "except", "finally", "with", "as", "lambda", "yield", "async", "await", "pass", "raise",
        "in", "not", "and", "or", "None", "True", "False", "self",
    ];
    const JS: &[&str] = &[
        "function",
        "const",
        "let",
        "var",
        "return",
        "if",
        "else",
        "for",
        "while",
        "class",
        "extends",
        "import",
        "export",
        "from",
        "async",
        "await",
        "new",
        "this",
        "try",
        "catch",
        "finally",
        "throw",
        "typeof",
        "interface",
        "type",
        "true",
        "false",
        "null",
        "undefined",
    ];
    const SHELL: &[&str] = &[
        "if", "then", "else", "fi", "for", "while", "do", "done", "case", "esac", "function",
        "export", "local", "return", "echo", "cd", "set",
    ];
    const GO: &[&str] = &[
        "func",
        "package",
        "import",
        "var",
        "const",
        "type",
        "struct",
        "interface",
        "return",
        "if",
        "else",
        "for",
        "range",
        "go",
        "defer",
        "chan",
        "select",
        "switch",
        "case",
        "map",
        "nil",
        "true",
        "false",
    ];

    match lang.to_lowercase().as_str() {
        "rust" | "rs" => RUST,
        "python" | "py" => PYTHON,
        "javascript" | "js" | "typescript" | "ts" | "tsx" | "jsx" => JS,
        "bash" | "sh" | "shell" | "zsh" => SHELL,
        "go" => GO,
        _ => &[],
    }
}

fn comment_prefix(lang: &str) -> Option<&'static str> {
    match lang.to_lowercase().as_str() {
        "rust" | "rs" | "javascript" | "js" | "typescript" | "ts" | "go" | "c" | "cpp" | "java" => {
            Some("//")
        }
        "python" | "py" | "bash" | "sh" | "shell" | "zsh" | "yaml" | "yml" | "toml" | "ruby"
        | "rb" => Some("#"),
        "sql" | "lua" | "haskell" => Some("--"),
        _ => None,
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Editing mode
// ---------------------------------------------------------------------------

fn draw_editing(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    gutter: u16,
    regions: &mut Regions,
) {
    let wrap = app.config.editor.wrap;
    // A caret in an unfocused pane claims input that would go somewhere else.
    let focused = app.focus == crate::app::Focus::Note;
    let prose = prose_area(area, gutter);
    // One column is left for the scrollbar, which also gives the caret a place
    // to sit at the end of a full row.
    let text = Rect {
        width: prose.width.saturating_sub(1),
        ..prose
    };
    let height = text.height as usize;

    let Some(editor) = app.editor_mut() else {
        return;
    };
    let layout = editor.layout(text.width as usize, wrap);
    editor.scroll_into_view(&layout, height);

    let scroll = editor.scroll;
    let hscroll = editor.hscroll;
    let (caret_row, caret_column) = editor.caret(&layout);
    let selection = editor.selection();
    let cursor_line = editor.cursor().line;
    let fenced = fenced_lines(editor.lines());

    // Painting a whole source line at once and slicing it per row keeps one
    // decision — what each character is — in one place, however it wrapped.
    let mut painted: Option<(usize, Vec<(char, Style)>)> = None;
    let mut numbers: Vec<Line> = Vec::new();
    let mut body: Vec<Line> = Vec::new();

    for row in layout.rows().iter().skip(scroll).take(height) {
        let on_cursor_line = row.line == cursor_line;
        numbers.push(gutter_line(row, on_cursor_line, gutter, palette));

        if painted.as_ref().is_none_or(|(line, _)| *line != row.line) {
            let source = &editor.lines()[row.line];
            painted = Some((
                row.line,
                paint_line(source, on_cursor_line, fenced[row.line], palette),
            ));
        }
        let Some((_, chars)) = &painted else { continue };

        body.push(row_line(
            row,
            &chars[row.start.min(chars.len())..row.end.min(chars.len())],
            RowStyle {
                background: if fenced[row.line] {
                    palette.code_bg
                } else if on_cursor_line {
                    palette.cursor_line_bg
                } else {
                    palette.bg_primary
                },
                selection: palette.bg_selection,
            },
            selection,
            text.width as usize + hscroll,
        ));
    }

    if gutter > 0 {
        Paragraph::new(numbers).render(
            Rect {
                width: gutter,
                ..area
            },
            frame.buffer_mut(),
        );
    }
    Paragraph::new(body)
        .scroll((0, u16::try_from(hscroll).unwrap_or(u16::MAX)))
        .render(text, frame.buffer_mut());
    scrollbar(frame, palette, area, scroll, layout.rows().len());

    regions.editor = Some((text, scroll));

    // Place the terminal cursor so the user sees a real caret.
    let column = usize::from(caret_column);
    if focused && caret_row >= scroll && caret_row - scroll < height && column >= hscroll {
        let x = text.x + u16::try_from(column - hscroll).unwrap_or(u16::MAX);
        if x < prose.x + prose.width {
            frame.set_cursor_position((x, text.y + (caret_row - scroll) as u16));
        }
    }
}

/// Colours for the row a line is drawn on, behind whatever it says.
struct RowStyle {
    background: ratatui::style::Color,
    selection: ratatui::style::Color,
}

/// The gutter cell for one row: a line number, or the wrap marker on a row that
/// is the continuation of the one above.
///
/// The marker lives here rather than in the text so that distinguishing a soft
/// wrap from a real line break costs no reading width at all.
fn gutter_line(row: &Row, on_cursor_line: bool, gutter: u16, palette: &Palette) -> Line<'static> {
    if gutter == 0 {
        return Line::from("");
    }
    let width = usize::from(gutter) - 1;
    let (text, color) = if row.first {
        (
            format!("{:>width$} ", row.line + 1),
            if on_cursor_line {
                palette.line_number_active
            } else {
                palette.line_number
            },
        )
    } else {
        (format!("{:>width$} ", icons::WRAP), palette.text_faint)
    };
    Line::from(Span::styled(text, Style::default().fg(color)))
}

/// Builds one terminal row from the painted characters it shows.
///
/// The row is padded to the full width so a code block's background and the
/// cursor line's highlight reach the edge of the pane rather than stopping at
/// the end of the text.
fn row_line(
    row: &Row,
    chars: &[(char, Style)],
    colors: RowStyle,
    selection: Option<(Cursor, Cursor)>,
    width: usize,
) -> Line<'static> {
    let selected = |col: usize| {
        selection.is_some_and(|(start, end)| {
            let at = (row.line, col);
            at >= (start.line, start.col) && at < (end.line, end.col)
        })
    };
    // A background already on the character — inline code — outranks the row's,
    // and a selection outranks both.
    let resolve = |style: Style, col: usize| {
        if selected(col) {
            return style.bg(colors.selection);
        }
        match style.bg {
            Some(_) => style,
            None => style.bg(colors.background),
        }
    };

    let mut spans = vec![Span::styled(
        " ".repeat(usize::from(row.indent)),
        Style::default().bg(colors.background),
    )];
    let mut run = String::new();
    let mut current: Option<Style> = None;

    for (offset, (ch, style)) in chars.iter().enumerate() {
        let style = resolve(*style, row.start + offset);
        if current.is_some_and(|open| open != style) {
            spans.push(Span::styled(std::mem::take(&mut run), current.unwrap()));
        }
        current = Some(style);
        run.push(*ch);
    }
    if let Some(style) = current {
        spans.push(Span::styled(run, style));
    }

    // Trailing fill, including the selected newline at the end of a line the
    // selection runs through.
    let used: usize = spans.iter().map(Span::width).sum();
    let end = selected(row.end).then_some(colors.selection);
    if let Some(color) = end {
        spans.push(Span::styled(" ", Style::default().bg(color)));
    }
    let pad = width.saturating_sub(used + usize::from(end.is_some()));
    spans.push(Span::styled(
        " ".repeat(pad),
        Style::default().bg(colors.background),
    ));
    Line::from(spans)
}

/// Which lines sit inside a fenced code block, so their content is never read
/// as Markdown while it is being edited.
fn fenced_lines(lines: &[String]) -> Vec<bool> {
    let mut inside = false;
    lines
        .iter()
        .map(|line| {
            let trimmed = line.trim_start();
            let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
            // The fence lines belong to the block they open and close.
            let fenced = inside || fence;
            inside ^= fence;
            fenced
        })
        .collect()
}

/// Styles one source line character by character, ready to be sliced across
/// however many rows it wrapped onto.
///
/// Markers are dimmed and given the glyph the reading pane would use, but only
/// where that glyph is exactly as wide as what it replaces — a `-` becomes a
/// `•`, an `x` inside `[x]` becomes a `☑`. Nothing is hidden and nothing moves,
/// so the caret is always where the character is, and `on_cursor_line` can show
/// the line exactly as typed without the text shifting underneath.
fn paint_line(
    text: &str,
    on_cursor_line: bool,
    fenced: bool,
    palette: &Palette,
) -> Vec<(char, Style)> {
    let chars: Vec<char> = text.chars().collect();
    if fenced {
        let style = Style::default().fg(palette.code_fg).bg(palette.code_bg);
        return chars.into_iter().map(|ch| (ch, style)).collect();
    }

    let mut out: Vec<(char, Style)> = markdown::scan_inline(text)
        .into_iter()
        .zip(&chars)
        .map(|(paint, ch)| (*ch, paint_style(paint, on_cursor_line, palette)))
        .collect();

    decorate_markers(&chars, on_cursor_line, palette, &mut out);
    out
}

/// Theme colors for one inline character of source.
fn paint_style(paint: markdown::Paint, on_cursor_line: bool, palette: &Palette) -> Style {
    use markdown::Ink;

    let mut style = Style::default().fg(match paint.ink {
        // On the cursor's own line the syntax is what you are editing, so it is
        // shown at full contrast rather than pushed into the background.
        Ink::Marker if on_cursor_line => palette.text_muted,
        Ink::Marker => palette.text_faint,
        // Whether a link resolves is the reading pane's business: it has the
        // link target, and this only has the characters.
        Ink::WikiLink => palette.link,
        Ink::Link => palette.link_external,
        Ink::Tag => palette.tag_fg,
        Ink::Math => palette.text_accent,
        Ink::Text => palette.text_normal,
    });
    if paint.ink == Ink::Tag {
        style = style.bg(palette.tag_bg);
    }
    emphasis(style, paint.style, palette)
}

/// Applies bold, italic, strikethrough, inline code and highlight colors.
///
/// Shared with the reading pane so a bold word can't end up a different color
/// depending on which mode you are looking at it in.
fn emphasis(mut style: Style, md: markdown::Style, palette: &Palette) -> Style {
    if md.code {
        style = style.fg(palette.code_fg).bg(palette.code_bg);
    }
    if md.bold {
        style = style.fg(palette.bold).add_modifier(Modifier::BOLD);
    }
    if md.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if md.strikethrough {
        style = style
            .fg(palette.strikethrough)
            .add_modifier(Modifier::CROSSED_OUT);
    }
    if md.highlight {
        style = style
            .bg(palette.text_highlight_bg)
            .fg(palette.text_highlight_fg);
    }
    style
}

/// Restyles a line's leading Markdown marker, and swaps in the glyph the
/// reading pane uses wherever it is exactly as wide.
fn decorate_markers(
    chars: &[char],
    on_cursor_line: bool,
    palette: &Palette,
    out: &mut [(char, Style)],
) {
    let indent = chars
        .iter()
        .take_while(|ch| **ch == ' ' || **ch == '\t')
        .count();
    let rest: String = chars[indent..].iter().collect();
    let at = |offset: usize| indent + offset;

    // A heading keeps its hashes, dimmed, and colors its title.
    let hashes = rest.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && rest.chars().nth(hashes) == Some(' ') {
        let level = u8::try_from(hashes).unwrap_or(6);
        let title = Style::default()
            .fg(palette.heading(level))
            .add_modifier(Modifier::BOLD);
        for entry in out.iter_mut().skip(at(hashes + 1)) {
            entry.1 = title;
        }
        return;
    }

    // `- [x] `: color the box, tick it, and strike the item through.
    if let Some(checked) = task(&rest) {
        let color = if checked {
            palette.checkbox_done
        } else {
            palette.checkbox_todo
        };
        substitute(out, at(0), bullet_glyph(on_cursor_line), color);
        for offset in [2, 4] {
            if let Some(entry) = out.get_mut(at(offset)) {
                entry.1 = Style::default().fg(palette.text_faint);
            }
        }
        let tick = if checked { icons::TASK_DONE } else { ' ' };
        substitute(out, at(3), (!on_cursor_line).then_some(tick), color);
        if checked {
            for entry in out.iter_mut().skip(at(6)) {
                entry.1 = entry
                    .1
                    .fg(palette.text_faint)
                    .add_modifier(Modifier::CROSSED_OUT);
            }
        }
        return;
    }

    if matches!(rest.as_bytes().first(), Some(b'-' | b'*' | b'+'))
        && rest.as_bytes().get(1) == Some(&b' ')
    {
        substitute(
            out,
            at(0),
            bullet_glyph(on_cursor_line),
            palette.list_marker,
        );
        return;
    }

    // `1. ` keeps its number; only the color says it is a marker.
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
        && rest.as_bytes().get(digits + 1) == Some(&b' ')
    {
        for entry in out.iter_mut().skip(indent).take(digits + 1) {
            entry.1 = Style::default().fg(palette.list_marker);
        }
        return;
    }

    // Every `>` of a quote, at every level, becomes the bar the reading pane
    // draws — and the body it introduces is tinted to match.
    if rest.starts_with('>') {
        let prefix = rest
            .bytes()
            .take_while(|b| matches!(b, b'>' | b' '))
            .count();
        for offset in 0..prefix {
            if chars.get(at(offset)) == Some(&'>') {
                substitute(
                    out,
                    at(offset),
                    (!on_cursor_line).then_some(icons::QUOTE_BAR),
                    palette.quote_bar,
                );
            }
        }
        for entry in out.iter_mut().skip(at(prefix)) {
            if entry.1.fg == Some(palette.text_normal) {
                entry.1 = entry.1.fg(palette.quote_fg);
            }
        }
    }
}

/// The glyph a list bullet is drawn as, or nothing on the cursor's own line
/// where the character as typed is what matters.
fn bullet_glyph(on_cursor_line: bool) -> Option<char> {
    (!on_cursor_line).then_some(icons::BULLET)
}

/// Recolors one character, optionally drawing a different glyph in its place.
///
/// Only ever called with a replacement the same width as the original, so the
/// column a character occupies never depends on how it is styled.
fn substitute(
    out: &mut [(char, Style)],
    at: usize,
    glyph: Option<char>,
    color: ratatui::style::Color,
) {
    if let Some(entry) = out.get_mut(at) {
        if let Some(glyph) = glyph {
            entry.0 = glyph;
        }
        entry.1 = Style::default().fg(color);
    }
}

/// Whether a line is a task, and whether it is done.
fn task(rest: &str) -> Option<bool> {
    let bytes = rest.as_bytes();
    let valid = matches!(bytes.first(), Some(b'-' | b'*' | b'+'))
        && bytes.get(1) == Some(&b' ')
        && bytes.get(2) == Some(&b'[')
        && bytes.get(4) == Some(&b']')
        && bytes.get(5) == Some(&b' ');
    valid.then(|| matches!(bytes.get(3), Some(b'x' | b'X')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otui_core::test_support::TempVault;
    use otui_theme::{Palette, presets};

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
    fn wrap_spans_preserves_styles_across_lines() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let spans = vec![
            ("hello ".to_string(), bold),
            ("world again".to_string(), bold),
        ];
        let lines = wrap_spans(&spans, 11, "", "");

        assert!(lines.len() > 1, "should wrap");
        for line in &lines {
            for span in &line.spans {
                if !span.content.trim().is_empty() {
                    assert!(span.style.add_modifier.contains(Modifier::BOLD));
                }
            }
        }
    }

    #[test]
    fn wrap_spans_applies_a_hanging_indent() {
        let spans = vec![("one two three four".to_string(), Style::default())];
        let lines = wrap_spans(&spans, 10, "", "    ");
        let rendered = text_of(&lines);

        assert!(rendered.len() > 1);
        assert!(
            rendered[1].starts_with("    "),
            "continuation lines are indented: {rendered:?}"
        );
    }

    /// A real PNG of the given size, since the layout reads its header.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    /// One cell is 10x20 pixels, so a 400x200 picture is 40x10 cells.
    ///
    /// The cap is a share of the pane, so a pane of `max_rows` rows with the
    /// share set to all of it caps pictures at exactly that many rows.
    fn images(max_rows: u16) -> Images {
        let mut images = Images::halfblocks(100);
        images.set_pane_height(max_rows);
        images
    }

    fn drawing<'a>(palette: &'a Palette, index: &'a VaultIndex, images: &'a mut Images) -> Ctx<'a> {
        Ctx {
            palette,
            index,
            note_dir: None,
            images: Some(images),
            pictures: Vec::new(),
            anchors: Vec::new(),
        }
    }

    #[test]
    fn a_picture_reserves_its_rows_where_the_text_leaves_off() {
        let vault = TempVault::new("render-image");
        vault.write_bytes("chart.png", &png(400, 200));
        let index = vault.index();
        let palette = palette();
        let mut images = images(20);

        let document = markdown::parse("Before\n\n![a chart](chart.png)\n\nAfter\n");
        let mut ctx = drawing(&palette, &index, &mut images);
        let lines = render_document(&document, &mut ctx, 60);

        let picture = match ctx.pictures.as_slice() {
            [picture] => picture,
            other => panic!("expected one picture, got {}", other.len()),
        };
        assert_eq!(
            (picture.size.width, picture.size.height),
            (40, 10),
            "400x200 pixels is 40x10 cells at this font size"
        );

        let rendered = text_of(&lines);
        assert!(
            rendered[picture.line..picture.line + 10]
                .iter()
                .all(|line| line.is_empty()),
            "the rows it will be drawn over are left blank: {rendered:?}"
        );
        let after = rendered
            .iter()
            .position(|line| line.contains("After"))
            .expect("text after the picture");
        assert!(
            after >= picture.line + 10,
            "text below it is pushed past its rows, so nothing moves when it loads"
        );
    }

    #[test]
    fn a_picture_too_wide_for_the_pane_is_scaled_to_fit() {
        let vault = TempVault::new("render-image-wide");
        vault.write_bytes("wide.png", &png(4000, 1000));
        let index = vault.index();
        let palette = palette();
        let mut images = images(20);

        let mut ctx = drawing(&palette, &index, &mut images);
        let document = markdown::parse("![](wide.png)\n");
        render_document(&document, &mut ctx, 30);

        assert_eq!(ctx.pictures[0].size.width, 30, "bounded by the pane");
        assert_eq!(ctx.pictures[0].size.height, 4, "and keeps its proportions");
    }

    #[test]
    fn a_tall_picture_stops_at_the_configured_row_cap() {
        let vault = TempVault::new("render-image-tall");
        vault.write_bytes("tall.png", &png(400, 4000));
        let index = vault.index();
        let palette = palette();
        let mut images = images(8);

        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![](tall.png)\n"), &mut ctx, 60);

        assert_eq!(
            ctx.pictures[0].size.height, 8,
            "a portrait photo shouldn't take several screens on its own"
        );
    }

    #[test]
    fn a_picture_grows_with_the_pane_rather_than_stopping_at_a_fixed_height() {
        // A cap in rows that suits an 80x24 terminal leaves a diagram
        // unreadable on a full-screen one, which is what made pictures look
        // like mush regardless of how well they were resampled.
        let vault = TempVault::new("render-image-share");
        vault.write_bytes("tall.png", &png(400, 4000));
        let index = vault.index();
        let palette = palette();

        let height_in = |rows: u16| {
            let mut images = Images::halfblocks(50);
            images.set_pane_height(rows);
            let mut ctx = drawing(&palette, &index, &mut images);
            render_document(&markdown::parse("![](tall.png)\n"), &mut ctx, 60);
            ctx.pictures[0].size.height
        };

        assert_eq!(height_in(24), 12, "half of a short pane");
        assert_eq!(height_in(60), 30, "and half of a tall one");
    }

    #[test]
    fn an_obsidian_embed_of_an_image_is_a_picture_but_of_a_note_is_not() {
        let vault = TempVault::new("render-embed");
        vault.write_bytes("assets/chart.png", &png(200, 100));
        vault.write("Other.md", "body\n");
        let index = vault.index();
        let palette = palette();
        let mut images = images(20);

        let mut ctx = drawing(&palette, &index, &mut images);
        // The picture is found by bare filename, from a subfolder, as Obsidian
        // resolves it.
        render_document(&markdown::parse("![[chart.png]]\n"), &mut ctx, 60);
        assert_eq!(ctx.pictures.len(), 1, "an image embed draws");

        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![[Other]]\n"), &mut ctx, 60);
        assert!(ctx.pictures.is_empty(), "a note embed is not a picture");
    }

    #[test]
    fn an_obsidian_width_hint_shrinks_the_picture() {
        let vault = TempVault::new("render-image-hint");
        vault.write_bytes("chart.png", &png(400, 200));
        let index = vault.index();
        let palette = palette();
        let mut images = images(20);

        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![[chart.png]]\n"), &mut ctx, 60);
        assert_eq!(ctx.pictures[0].size.width, 40, "40 cells at its own size");

        // 200 pixels is 20 cells at this font.
        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![[chart.png|200]]\n"), &mut ctx, 60);
        assert_eq!(ctx.pictures[0].size.width, 20, "the hint wins");
        assert_eq!(ctx.pictures[0].size.height, 5, "and it keeps its shape");

        // A hint wider than the pane is still bounded by the pane.
        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![[chart.png|9000]]\n"), &mut ctx, 30);
        assert_eq!(ctx.pictures[0].size.width, 30);

        // Everything else after the pipe is an alias, not a size.
        let mut ctx = drawing(&palette, &index, &mut images);
        render_document(&markdown::parse("![[chart.png|a diagram]]\n"), &mut ctx, 60);
        assert_eq!(ctx.pictures[0].size.width, 40);
    }

    #[test]
    fn a_picture_that_cannot_be_drawn_falls_back_to_its_alt_text() {
        let vault = TempVault::new("render-image-missing");
        vault.write_bytes("chart.png", &png(200, 100));
        let index = vault.index();
        let palette = palette();

        // Every reason a picture might not be drawable, and all of them should
        // leave readable text behind rather than a hole.
        let cases = [
            ("![a chart](chart.png)", "a terminal that cannot draw"),
            ("![gone](missing.png)", "a file that isn't there"),
            ("![remote](https://e.com/x.png)", "a picture on the web"),
        ];
        for (source, reason) in cases {
            let mut images = match reason {
                "a terminal that cannot draw" => Images::disabled(),
                _ => images(20),
            };
            let mut ctx = drawing(&palette, &index, &mut images);
            let lines = render_document(&markdown::parse(source), &mut ctx, 60);

            assert!(ctx.pictures.is_empty(), "nothing is reserved for {reason}");
            let rendered = text_of(&lines).join(" ");
            let alt = source
                .split_once('[')
                .and_then(|(_, rest)| rest.split_once(']'))
                .map(|(alt, _)| alt)
                .unwrap_or_default();
            assert!(
                rendered.contains(alt),
                "{reason} still reads as {alt:?}: {rendered:?}"
            );
        }
    }

    #[test]
    fn a_picture_inside_a_quote_moves_with_the_quote() {
        let vault = TempVault::new("render-image-quote");
        vault.write_bytes("chart.png", &png(200, 100));
        let index = vault.index();
        let palette = palette();
        let mut images = images(20);

        let mut ctx = drawing(&palette, &index, &mut images);
        let lines = render_document(
            &markdown::parse("Intro\n\n> quoted\n> ![](chart.png)\n"),
            &mut ctx,
            60,
        );

        let picture = &ctx.pictures[0];
        assert!(
            picture.indent >= 2,
            "it sits to the right of the quote bar, not under it"
        );
        let rendered = text_of(&lines);
        assert!(
            rendered[picture.line]
                .trim_end()
                .ends_with(icons::QUOTE_BAR),
            "its first row is a quote bar and then blank: {:?}",
            rendered[picture.line]
        );
    }

    #[test]
    fn headings_are_colored_by_level_and_underlined() {
        let vault = TempVault::new("render-heading");
        let index = vault.index();
        let palette = palette();
        let document = markdown::parse("# Title\n\n### Deep\n");

        let lines = render_document(&document, &mut Ctx::text(&palette, &index), 40);
        let rendered = text_of(&lines);

        assert!(rendered.iter().any(|l| l.contains("Title")));
        assert!(
            rendered.iter().any(|l| l.starts_with('─')),
            "H1 gets a rule under it: {rendered:?}"
        );
        assert!(rendered.iter().any(|l| l.contains("Deep")));
    }

    #[test]
    fn unresolved_wikilinks_render_dimmed() {
        let vault = TempVault::new("render-link");
        vault.write("Real.md", "x");
        let index = vault.index();
        let palette = palette();

        let document = markdown::parse("See [[Real]] and [[Missing]].\n");
        let lines = render_document(&document, &mut Ctx::text(&palette, &index), 60);

        let styles: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.content.contains("Real") || s.content.contains("Missing"))
            .map(|s| (s.content.to_string(), s.style.fg))
            .collect();

        let real = styles
            .iter()
            .find(|(t, _)| t.contains("Real"))
            .expect("Real");
        let missing = styles
            .iter()
            .find(|(t, _)| t.contains("Missing"))
            .expect("Missing");

        assert_eq!(real.1, Some(palette.link));
        assert_eq!(missing.1, Some(palette.link_unresolved));
    }

    #[test]
    fn tasks_render_with_checkboxes_and_strike_completed_ones() {
        let vault = TempVault::new("render-task");
        let index = vault.index();
        let palette = palette();

        let document = markdown::parse("- [ ] todo\n- [x] done\n");
        let lines = render_document(&document, &mut Ctx::text(&palette, &index), 40);
        let rendered = text_of(&lines);

        assert!(rendered.iter().any(|l| l.contains(icons::TASK_TODO)));
        assert!(rendered.iter().any(|l| l.contains(icons::TASK_DONE)));

        let done_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("done")))
            .expect("done line");
        assert!(
            done_line
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::CROSSED_OUT))
        );
    }

    #[test]
    fn callouts_render_with_a_colored_bar_and_title() {
        let vault = TempVault::new("render-callout");
        let index = vault.index();
        let palette = palette();

        let document = markdown::parse("> [!warning] Careful\n> body\n");
        let lines = render_document(&document, &mut Ctx::text(&palette, &index), 40);
        let rendered = text_of(&lines);

        assert!(rendered[0].contains(icons::QUOTE_BAR));
        assert!(rendered[0].contains("Careful"));
        assert!(rendered.len() > 1, "the body renders too");
    }

    #[test]
    fn a_callout_without_a_title_uses_its_kind() {
        let vault = TempVault::new("render-callout-2");
        let index = vault.index();
        let document = markdown::parse("> [!tip]\n> body\n");
        let lines = render_document(&document, &mut Ctx::text(&palette(), &index), 40);

        assert!(text_of(&lines)[0].contains("Tip"));
    }

    #[test]
    fn tables_render_with_borders_and_fit_the_width() {
        let vault = TempVault::new("render-table");
        let index = vault.index();
        let document = markdown::parse("| A | B |\n|---|---|\n| 1 | 2 |\n");
        let lines = render_document(&document, &mut Ctx::text(&palette(), &index), 40);
        let rendered = text_of(&lines);

        assert!(rendered.iter().any(|l| l.contains('┌')));
        assert!(rendered.iter().any(|l| l.contains('│')));
        for line in &rendered {
            assert!(line.chars().count() <= 44, "{line:?} overflows");
        }
    }

    #[test]
    fn a_wide_table_keeps_its_columns_rather_than_being_crushed() {
        // Eight columns divided between a pane this narrow used to leave three
        // characters each — a table that fits and says nothing. It is laid out
        // at its own width now and panned across instead.
        let vault = TempVault::new("render-wide-table");
        let index = vault.index();

        let header: Vec<String> = (0..8).map(|i| format!("column {i}")).collect();
        let cells: Vec<String> = (0..8).map(|i| format!("value number {i}")).collect();
        let source = format!(
            "| {} |\n|{}|\n| {} |\n",
            header.join(" | "),
            "---|".repeat(8),
            cells.join(" | ")
        );

        let document = markdown::parse(&source);
        let lines = render_document(&document, &mut Ctx::text(&palette(), &index), 40);
        let rendered = text_of(&lines);

        let row = rendered
            .iter()
            .find(|line| line.contains("value number 0"))
            .expect("the body row");
        for cell in &cells {
            assert!(
                row.contains(cell.as_str()),
                "{cell:?} was truncated: {row:?}"
            );
        }
        assert!(
            row.chars().count() > 40,
            "the table is wider than the pane, which is what makes panning necessary"
        );
    }

    #[test]
    fn a_runaway_column_cannot_push_the_others_out_of_reach() {
        let vault = TempVault::new("render-essay-table");
        let index = vault.index();
        let essay = "e".repeat(300);
        let source = format!("| A | B |\n|---|---|\n| {essay} | end |\n");

        let lines = render_document(
            &markdown::parse(&source),
            &mut Ctx::text(&palette(), &index),
            40,
        );
        let row = text_of(&lines)
            .into_iter()
            .find(|line| line.contains('e'))
            .expect("the body row");
        assert!(
            row.contains("end"),
            "the last column is still reachable: {row:?}"
        );
        assert!(row.chars().count() < 300, "the essay cell was capped");
    }

    #[test]
    fn code_blocks_highlight_keywords() {
        let palette = palette();
        let spans = highlight("let x = 1;", "rust", &palette, Style::default());

        let keyword = spans
            .iter()
            .find(|s| s.content == "let")
            .expect("keyword span");
        assert_eq!(keyword.style.fg, Some(palette.syn_keyword));

        let number = spans
            .iter()
            .find(|s| s.content == "1")
            .expect("number span");
        assert_eq!(number.style.fg, Some(palette.syn_number));
    }

    #[test]
    fn comments_and_strings_are_highlighted() {
        let palette = palette();

        let comment = highlight("# a note", "python", &palette, Style::default());
        assert_eq!(comment[0].style.fg, Some(palette.syn_comment));

        let string = highlight("x = \"hi\"", "python", &palette, Style::default());
        assert!(
            string
                .iter()
                .any(|s| s.style.fg == Some(palette.syn_string) && s.content.contains("hi"))
        );
    }

    #[test]
    fn an_unknown_language_still_renders() {
        let palette = palette();
        let spans = highlight("some text", "brainfuck", &palette, Style::default());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "some text");
    }

    #[test]
    fn frontmatter_renders_as_a_metadata_header() {
        let vault = TempVault::new("render-fm");
        let index = vault.index();
        let document = markdown::parse("---\ntitle: T\ntags: [a]\n---\nbody\n");
        let lines = render_document(&document, &mut Ctx::text(&palette(), &index), 40);
        let rendered = text_of(&lines);

        assert!(rendered.iter().any(|l| l.contains("title: T")));
        assert!(rendered.iter().any(|l| l.contains("tags: a")));
        assert!(rendered.iter().any(|l| l.contains("body")));
    }

    #[test]
    fn rendering_never_produces_lines_wider_than_the_pane() {
        let vault = TempVault::new("render-width");
        let index = vault.index();
        let long = "word ".repeat(60);
        let document = markdown::parse(&format!("{long}\n\n- {long}\n\n> {long}\n"));

        for width in [20usize, 40, 80] {
            let lines = render_document(&document, &mut Ctx::text(&palette(), &index), width);
            for line in text_of(&lines) {
                assert!(
                    line.chars().count() <= width + 4,
                    "width {width}: {:?} is {} chars",
                    line,
                    line.chars().count()
                );
            }
        }
    }
}
