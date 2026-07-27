//! The graph view.
//!
//! Nodes and edges are drawn on a braille canvas, which gives four times the
//! vertical resolution of text cells and makes the layout legible in a
//! terminal. Labels are drawn as text on top, only where there's room, because
//! a graph with every label shown is unreadable past a few dozen notes — the
//! same reason Obsidian fades labels out as you zoom away.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as CanvasLine, Points};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use otui_core::graph::{NodeKind, Vec2};
use otui_theme::Palette;

use crate::app::{App, Focus, Regions};
use crate::ui::truncate;

/// Never draw more labels than this; past it the view is noise.
const MAX_LABELS: usize = 60;

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
    }

    let focused = app.focus == Focus::Graph;
    let show_labels = app.config.graph.show_labels;

    // Local mode dims everything outside the focused note's neighborhood.
    let neighborhood: Option<Vec<usize>> = graph.local_root.and_then(|root| {
        graph.simulation.graph.node_of_note(root).map(|node| {
            graph
                .simulation
                .graph
                .neighborhood(node, app.config.graph.local_depth)
        })
    });

    // Fit the whole graph, preserving aspect. A braille cell is 2 dots wide and
    // 4 tall, so the drawable area is far from square in graph units — scaling
    // both axes by the same factor is what keeps the layout from being
    // stretched, and taking the larger factor is what keeps every node on
    // screen.
    let (min_x, min_y, max_x, max_y) = graph.simulation.graph.bounds();
    let span_x = f64::from(max_x - min_x).max(1.0) * 1.15;
    let span_y = f64::from(max_y - min_y).max(1.0) * 1.15;

    let dots_x = f64::from(area.width.max(1)) * 2.0;
    let dots_y = f64::from(area.height.max(1)) * 4.0;
    let scale = (span_x / dots_x).max(span_y / dots_y) / f64::from(graph.zoom);

    let half_x = scale * dots_x / 2.0;
    let half_y = scale * dots_y / 2.0;
    let center = graph.center;

    let x_bounds = [f64::from(center.x) - half_x, f64::from(center.x) + half_x];
    let y_bounds = [f64::from(center.y) - half_y, f64::from(center.y) + half_y];
    // Node radius is in graph units, so it has to track the zoom level to stay
    // a constant size on screen.
    let node_scale = (scale * 3.0) as f32;

    regions.graph = Some((area, x_bounds, y_bounds));

    let simulation = &graph.simulation;
    let selected = graph.selected;

    let canvas = Canvas::default()
        .background_color(palette.graph_bg)
        .marker(Marker::Braille)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx: &mut Context| {
            let visible = |index: usize| {
                neighborhood
                    .as_ref()
                    .is_none_or(|nodes| nodes.contains(&index))
            };

            // Edges first so nodes sit on top of them.
            for edge in &simulation.graph.edges {
                let (Some(a), Some(b)) = (
                    simulation.graph.nodes.get(edge.from),
                    simulation.graph.nodes.get(edge.to),
                ) else {
                    continue;
                };
                let touches_selection = selected == Some(edge.from) || selected == Some(edge.to);
                if !visible(edge.from) && !visible(edge.to) {
                    continue;
                }
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

            ctx.layer();

            for (index, node) in simulation.graph.nodes.iter().enumerate() {
                if !visible(index) {
                    continue;
                }
                let is_selected = selected == Some(index);
                let is_neighbor =
                    selected.is_some_and(|s| simulation.graph.neighbors(s).contains(&index));

                let color = if is_selected {
                    palette.graph_node_focused
                } else if is_neighbor {
                    palette.graph_node_neighbor
                } else {
                    match node.kind {
                        NodeKind::Note(_) => palette.graph_node,
                        NodeKind::Unresolved => palette.graph_node_unresolved,
                        NodeKind::Tag => palette.graph_node_tag,
                        NodeKind::Attachment => palette.graph_node_unresolved,
                    }
                };

                // Well-connected notes draw larger, as in Obsidian.
                let radius = match node.degree {
                    0 => 0.4,
                    1..=2 => 0.9,
                    3..=5 => 1.4,
                    _ => 2.0,
                } * node_scale;

                ctx.draw(&Points {
                    coords: &blob(node.pos, radius),
                    color,
                });
            }
        });

    frame.render_widget(canvas, area);

    if show_labels {
        draw_labels(
            frame,
            app,
            palette,
            area,
            x_bounds,
            y_bounds,
            neighborhood.as_deref(),
        );
    }
    draw_legend(frame, app, palette, area, focused);
}

/// Points forming a filled disc, so nodes read as blobs rather than pixels.
fn blob(center: Vec2, radius: f32) -> Vec<(f64, f64)> {
    if radius <= 0.0 {
        return vec![(f64::from(center.x), f64::from(center.y))];
    }
    let mut points = Vec::new();
    let steps = 8;
    points.push((f64::from(center.x), f64::from(center.y)));
    for ring in 1..=2 {
        let r = radius * ring as f32 / 2.0;
        for step in 0..steps {
            let angle = std::f32::consts::TAU * step as f32 / steps as f32;
            points.push((
                f64::from(center.x + r * angle.cos()),
                f64::from(center.y + r * angle.sin()),
            ));
        }
    }
    points
}

fn draw_labels(
    frame: &mut Frame,
    app: &App,
    palette: &Palette,
    area: Rect,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    neighborhood: Option<&[usize]>,
) {
    let Some(graph) = app.graph.as_ref() else {
        return;
    };

    // Draw the best-connected nodes first so that when space runs out, the
    // labels that survive are the ones that orient the reader.
    let mut order: Vec<usize> = (0..graph.simulation.graph.nodes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(graph.simulation.graph.nodes[i].degree));

    let mut occupied: Vec<Rect> = Vec::new();
    let mut drawn = 0;

    for index in order {
        if drawn >= MAX_LABELS {
            break;
        }
        if neighborhood.is_some_and(|nodes| !nodes.contains(&index)) {
            continue;
        }
        let node = &graph.simulation.graph.nodes[index];
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
        area.x + (fx * f64::from(area.width - 1)) as u16,
        area.y + (fy * f64::from(area.height - 1)) as u16,
    ))
}

fn draw_legend(frame: &mut Frame, app: &App, palette: &Palette, area: Rect, focused: bool) {
    let Some(graph) = app.graph.as_ref() else {
        return;
    };

    let nodes = graph.simulation.graph.nodes.len();
    let edges = graph.simulation.graph.edges.len();
    let mut text = format!(" {nodes} nodes  {edges} links");
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

    let y = area.y + area.height.saturating_sub(1);
    frame.buffer_mut().set_string(
        area.x,
        y,
        truncate(&text, area.width as usize),
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
    fn blobs_scale_with_radius() {
        assert_eq!(blob(Vec2::default(), 0.0).len(), 1, "a dot for an orphan");
        assert!(blob(Vec2::default(), 2.0).len() > 8);
    }
}
