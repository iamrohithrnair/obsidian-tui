//! The graph view.
//!
//! Edges are drawn on a braille canvas, which gives twice the horizontal and
//! four times the vertical resolution of text cells and so keeps long diagonal
//! links smooth. Nodes are *not*: they're single text glyphs drawn on top at
//! cell resolution.
//!
//! Splitting the two is what makes the view readable. A node rasterized into
//! braille is a blob one or two cells across, and two of those blobs sitting
//! next to each other merge into a single shape, so a cluster of notes reads as
//! one smear rather than as five notes. One glyph per node can never merge with
//! its neighbour, and the glyph itself carries the node's degree and kind.
//!
//! Labels are drawn only where there's room, because a graph with every label
//! shown is unreadable past a few dozen notes — the same reason Obsidian fades
//! labels out as you zoom away.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as CanvasLine};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use otui_core::graph::{Node, NodeKind, Vec2};
use otui_theme::Palette;

use crate::app::{App, Focus, GraphView, Regions};
use crate::ui::truncate;

/// Never draw more labels than this; past it the view is noise.
const MAX_LABELS: usize = 60;
/// Braille dots per cell, horizontally and vertically.
const DOTS_X: f64 = 2.0;
const DOTS_Y: f64 = 4.0;

/// The mark that stands for a node.
///
/// Notes get heavier as they gain links, which is how Obsidian sizes them and
/// what lets you find the hubs at a glance. A link with no note behind it is
/// hollow — Obsidian's own distinction, and the fastest way to spot the notes
/// you meant to write.
fn glyph(node: &Node) -> char {
    match node.kind {
        NodeKind::Unresolved | NodeKind::Attachment => '○',
        NodeKind::Tag => '◆',
        NodeKind::Note(_) => match node.degree {
            0 => '·',
            1..=2 => '•',
            3..=5 => '●',
            _ => '◉',
        },
    }
}

pub fn draw(
    frame: &mut Frame,
    app: &mut App,
    palette: &Palette,
    area: Rect,
    regions: &mut Regions,
) {
    let Some(graph) = app.graph.as_mut() else {
        Paragraph::new(Line::from(Span::styled(
            " press Ctrl+G to build the graph",
            Style::default().fg(palette.text_faint),
        )))
        .render(area, frame.buffer_mut());
        return;
    };

    // Keep stepping until settled; an idle graph costs nothing.
    if !graph.simulation.is_settled() {
        graph.simulation.step();
        // The layout spreads as it settles, so the extent measured when it
        // opened is too small by the time it stops. Refitting once here — and
        // not every frame — is what keeps the picture framed without making it
        // rescale under the user on every tick.
        if graph.simulation.is_settled() {
            graph.refit_span();
        }
    }

    let focused = app.focus == Focus::Graph;
    let show_labels = app.config.graph.show_labels;

    // The legend gets a row of its own rather than being painted over the
    // canvas, so a node never ends up half-hidden behind it.
    let legend_row = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: area.height.min(1),
        ..area
    };
    let canvas_area = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    if canvas_area.height == 0 || canvas_area.width == 0 {
        return;
    }

    let (x_bounds, y_bounds) = viewport(graph, canvas_area);
    regions.graph = Some((canvas_area, x_bounds, y_bounds));

    let simulation = &graph.simulation;
    let selected = graph.selected;

    let canvas = Canvas::default()
        .background_color(palette.graph_bg)
        .marker(Marker::Braille)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx: &mut Context| {
            // Only edges live on the canvas; nodes are drawn as glyphs
            // afterwards, over the top.
            for edge in &simulation.graph.edges {
                let (Some(a), Some(b)) = (
                    simulation.graph.nodes.get(edge.from),
                    simulation.graph.nodes.get(edge.to),
                ) else {
                    continue;
                };
                let touches_selection = selected == Some(edge.from) || selected == Some(edge.to);
                ctx.draw(&CanvasLine {
                    x1: f64::from(a.pos.x),
                    y1: f64::from(a.pos.y),
                    x2: f64::from(b.pos.x),
                    y2: f64::from(b.pos.y),
                    color: if touches_selection {
                        palette.graph_edge_active
                    } else {
                        palette.graph_edge
                    },
                });
            }
        });

    frame.render_widget(canvas, canvas_area);

    draw_nodes(frame, app, palette, canvas_area, x_bounds, y_bounds);

    if show_labels {
        draw_labels(frame, app, palette, canvas_area, x_bounds, y_bounds);
    }
    draw_legend(frame, app, palette, legend_row, focused);
}

/// The canvas bounds for the current pan and zoom.
///
/// A braille dot is half a cell wide and a quarter of a cell tall, and terminal
/// cells are about twice as tall as they are wide, so dots come out square.
/// Scaling both axes by the same factor is what keeps the layout from being
/// stretched, and fitting the span into whichever axis is shorter in dots is
/// what keeps every node on screen.
///
/// The span comes from [`GraphView::span`] rather than from live bounds, so a
/// resize reframes the graph but a still-settling layout doesn't rescale it.
fn viewport(graph: &GraphView, area: Rect) -> ([f64; 2], [f64; 2]) {
    let dots_x = f64::from(area.width.max(1)) * DOTS_X;
    let dots_y = f64::from(area.height.max(1)) * DOTS_Y;
    let dot = f64::from(graph.span.max(1.0)) / dots_x.min(dots_y) / f64::from(graph.zoom.max(0.01));

    let half_x = dot * dots_x / 2.0;
    let half_y = dot * dots_y / 2.0;
    let center = graph.center;

    (
        [f64::from(center.x) - half_x, f64::from(center.x) + half_x],
        [f64::from(center.y) - half_y, f64::from(center.y) + half_y],
    )
}

/// Draws one glyph per node, over the edges the canvas has already painted.
fn draw_nodes(
    frame: &mut Frame,
    app: &App,
    palette: &Palette,
    area: Rect,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) {
    let Some(graph) = app.graph.as_ref() else {
        return;
    };
    let nodes = &graph.simulation.graph.nodes;

    // Resolved once rather than per node: asking whether each of five thousand
    // nodes appears in the selection's adjacency list is a scan per node, and
    // this runs every frame while the layout settles.
    let mut is_neighbor = vec![false; nodes.len()];
    if let Some(selected) = graph.selected {
        for &neighbor in graph.simulation.graph.neighbors(selected) {
            if let Some(flag) = is_neighbor.get_mut(neighbor) {
                *flag = true;
            }
        }
    }

    // Least-connected first, so that when two nodes fall in the same cell the
    // one that better orients the reader is the one left showing.
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| nodes[i].degree);

    for index in order {
        let node = &nodes[index];
        let Some((x, y)) = project(node.pos, area, x_bounds, y_bounds) else {
            continue;
        };

        let is_selected = graph.selected == Some(index);
        let is_neighbor = is_neighbor[index];

        let color = if is_selected {
            palette.graph_node_focused
        } else if is_neighbor {
            palette.graph_node_neighbor
        } else {
            match node.kind {
                NodeKind::Note(_) => palette.graph_node,
                NodeKind::Unresolved | NodeKind::Attachment => palette.graph_node_unresolved,
                NodeKind::Tag => palette.graph_node_tag,
            }
        };
        let mut style = Style::default().fg(color);
        if is_selected {
            style = style.add_modifier(Modifier::BOLD);
        }

        frame
            .buffer_mut()
            .set_string(x, y, glyph(node).to_string(), style);
    }
}

fn draw_labels(
    frame: &mut Frame,
    app: &App,
    palette: &Palette,
    area: Rect,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) {
    let Some(graph) = app.graph.as_ref() else {
        return;
    };
    let nodes = &graph.simulation.graph.nodes;

    // Every node is reserved before any label is placed, so a label never
    // covers the thing it names — or one of its neighbours. A node is exactly
    // one cell, which is what leaves enough room for most labels to land.
    let mut occupied: Vec<Rect> = Vec::new();
    for node in nodes {
        if let Some((x, y)) = project(node.pos, area, x_bounds, y_bounds) {
            occupied.push(Rect {
                x,
                y,
                width: 1,
                height: 1,
            });
        }
    }

    // Draw the best-connected nodes first so that when space runs out, the
    // labels that survive are the ones that orient the reader.
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(nodes[i].degree));

    let mut drawn = 0;

    for index in order {
        if drawn >= MAX_LABELS {
            break;
        }
        let node = &nodes[index];
        let selected = graph.selected == Some(index);

        let Some((x, y)) = project(node.pos, area, x_bounds, y_bounds) else {
            continue;
        };

        let label = truncate(&node.label, 18);
        let width = label.chars().count() as u16;
        // Centring the label on the node can push it past the pane's left edge,
        // where it would be drawn over the neighbouring pane; clamp instead.
        let rect = Rect {
            x: x.saturating_sub(width / 2).max(area.x),
            y: y.saturating_add(1),
            width,
            height: 1,
        };
        if rect.x + rect.width > area.x + area.width || rect.y >= area.y + area.height {
            continue;
        }
        // Overlapping labels are worse than missing ones.
        if occupied.iter().any(|r| r.intersects(rect)) {
            continue;
        }

        let style = if selected {
            Style::default()
                .fg(palette.graph_label_focused)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.graph_label)
        };
        frame.buffer_mut().set_string(rect.x, rect.y, &label, style);
        occupied.push(rect);
        drawn += 1;
    }
}

/// Maps a graph position to a terminal cell, or `None` if off-screen.
fn project(pos: Vec2, area: Rect, x_bounds: [f64; 2], y_bounds: [f64; 2]) -> Option<(u16, u16)> {
    let x_span = x_bounds[1] - x_bounds[0];
    let y_span = y_bounds[1] - y_bounds[0];
    if x_span <= 0.0 || y_span <= 0.0 {
        return None;
    }

    let fx = (f64::from(pos.x) - x_bounds[0]) / x_span;
    // Canvas y grows upward, terminal rows grow downward.
    let fy = 1.0 - (f64::from(pos.y) - y_bounds[0]) / y_span;
    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return None;
    }

    Some((
        area.x + (fx * f64::from(area.width.saturating_sub(1))) as u16,
        area.y + (fy * f64::from(area.height.saturating_sub(1))) as u16,
    ))
}

fn draw_legend(frame: &mut Frame, app: &App, palette: &Palette, area: Rect, focused: bool) {
    let Some(graph) = app.graph.as_ref() else {
        return;
    };
    if area.height == 0 || area.width == 0 {
        return;
    }

    let nodes = graph.simulation.graph.nodes.len();
    let edges = graph.simulation.graph.edges.len();
    let mut text = format!(
        " {nodes} nodes  {edges} links  ·  {zoom:.0}%",
        zoom = graph.zoom * 100.0
    );
    if let Some(selected) = graph
        .selected
        .and_then(|i| graph.simulation.graph.nodes.get(i))
    {
        text.push_str(&format!("  ·  {}", truncate(&selected.label, 24)));
    }
    if graph.local_root.is_some() {
        text.push_str("  ·  local");
    }
    if !graph.simulation.is_settled() {
        text.push_str("  ·  settling…");
    }
    // The key list lives in the hint bar; repeating it here just crowds the
    // canvas.
    let _ = focused;

    let width = area.width as usize;
    let padded = format!("{text:<width$}");
    frame.buffer_mut().set_string(
        area.x,
        area.y,
        truncate(&padded, width),
        Style::default().fg(palette.text_faint).bg(palette.graph_bg),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_maps_bounds_corners_to_screen_corners() {
        let area = Rect::new(0, 0, 100, 50);
        let x = [-10.0, 10.0];
        let y = [-5.0, 5.0];

        let top_left = project(Vec2::new(-10.0, 5.0), area, x, y).expect("in bounds");
        assert_eq!(top_left, (0, 0));

        let bottom_right = project(Vec2::new(10.0, -5.0), area, x, y).expect("in bounds");
        assert_eq!(bottom_right, (99, 49));
    }

    #[test]
    fn projection_rejects_off_screen_points() {
        let area = Rect::new(0, 0, 100, 50);
        assert!(project(Vec2::new(999.0, 0.0), area, [-10.0, 10.0], [-5.0, 5.0]).is_none());
    }

    #[test]
    fn projection_handles_a_degenerate_viewport() {
        let area = Rect::new(0, 0, 100, 50);
        assert!(project(Vec2::default(), area, [0.0, 0.0], [0.0, 0.0]).is_none());
    }

    #[test]
    fn a_note_gets_heavier_as_it_gains_links() {
        let note = |degree| Node {
            label: "n".into(),
            kind: NodeKind::Note(0),
            pos: Vec2::default(),
            vel: Vec2::default(),
            degree,
            pinned: false,
        };
        // The ramp has to be strictly increasing, or degree stops being
        // readable off the glyph.
        let marks: Vec<char> = [0, 1, 3, 30].into_iter().map(|d| glyph(&note(d))).collect();
        assert_eq!(marks, vec!['\u{b7}', '\u{2022}', '\u{25cf}', '\u{25c9}']);
    }

    #[test]
    fn a_link_with_no_note_behind_it_is_hollow() {
        let unresolved = Node {
            label: "Ghost".into(),
            kind: NodeKind::Unresolved,
            pos: Vec2::default(),
            vel: Vec2::default(),
            degree: 1,
            pinned: false,
        };
        assert_eq!(glyph(&unresolved), '\u{25cb}');
    }
}
