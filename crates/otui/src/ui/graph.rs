//! The graph view.
//!
//! Nodes and edges are drawn on a braille canvas, which gives four times the
//! vertical resolution of text cells and makes the layout legible in a
//! terminal. Labels are drawn as text on top, only where there's room, because
//! a graph with every label shown is unreadable past a few dozen notes — the
//! same reason Obsidian fades labels out as you zoom away.
//!
//! A node is rasterized onto the canvas's own dot lattice rather than sampled
//! at a fixed number of angles. Sampling leaves gaps as soon as the node is
//! more than a couple of dots across, which is what turns a graph into a field
//! of speckle; stepping the lattice draws a solid disc at every size.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context, Line as CanvasLine, Points};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use otui_core::graph::{NodeKind, Vec2};
use otui_theme::Palette;

use crate::app::{App, Focus, GraphView, Regions};
use crate::ui::truncate;

/// Never draw more labels than this; past it the view is noise.
const MAX_LABELS: usize = 60;
/// Braille dots per cell, horizontally and vertically.
const DOTS_X: f64 = 2.0;
const DOTS_Y: f64 = 4.0;
/// Padding around the laid-out graph when the view is fitted to it.
const FIT_PADDING: f64 = 1.15;

/// A node's radius in canvas dots. Well-connected notes draw larger, as in
/// Obsidian. Held in dots rather than graph units so a node stays the same size
/// on screen however far the view is zoomed in.
fn radius_dots(degree: usize) -> f64 {
    match degree {
        0 => 1.6,
        1..=2 => 2.4,
        3..=5 => 3.2,
        6..=11 => 4.0,
        _ => 5.0,
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

    // Local mode hides everything outside the focused note's neighborhood.
    let neighborhood: Option<Vec<usize>> = graph.local_root.and_then(|root| {
        graph.simulation.graph.node_of_note(root).map(|node| {
            graph
                .simulation
                .graph
                .neighborhood(node, app.config.graph.local_depth)
        })
    });

    let (x_bounds, y_bounds, dot) = viewport(graph, canvas_area);
    regions.graph = Some((canvas_area, x_bounds, y_bounds));

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
                if !visible(edge.from) && !visible(edge.to) {
                    continue;
                }
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

                let radius = radius_dots(node.degree) * dot;

                // Notes that exist are solid; a link with no note behind it is
                // hollow, which is how Obsidian distinguishes the two and the
                // fastest way to spot the notes you meant to write.
                let coords = match node.kind {
                    NodeKind::Unresolved | NodeKind::Attachment => ring(node.pos, radius, dot),
                    NodeKind::Note(_) | NodeKind::Tag => disc(node.pos, radius, dot),
                };
                ctx.draw(&Points {
                    coords: &coords,
                    color,
                });

                if is_selected {
                    ctx.draw(&Points {
                        coords: &ring(node.pos, radius + 2.0 * dot, dot),
                        color: palette.graph_node_focused,
                    });
                }
            }
        });

    frame.render_widget(canvas, canvas_area);

    if show_labels {
        draw_labels(
            frame,
            app,
            palette,
            canvas_area,
            x_bounds,
            y_bounds,
            neighborhood.as_deref(),
        );
    }
    draw_legend(frame, app, palette, legend_row, focused);
}

/// The canvas bounds for the current pan and zoom, plus the size of one canvas
/// dot in graph units.
///
/// A braille dot is half a cell wide and a quarter of a cell tall, and terminal
/// cells are about twice as tall as they are wide, so dots come out square.
/// Scaling both axes by the same factor is what keeps the layout from being
/// stretched, and taking the larger factor is what keeps every node on screen.
pub fn viewport(graph: &GraphView, area: Rect) -> ([f64; 2], [f64; 2], f64) {
    let (min_x, min_y, max_x, max_y) = graph.simulation.graph.bounds();
    let span_x = f64::from(max_x - min_x).max(1.0) * FIT_PADDING;
    let span_y = f64::from(max_y - min_y).max(1.0) * FIT_PADDING;

    let dots_x = f64::from(area.width.max(1)) * DOTS_X;
    let dots_y = f64::from(area.height.max(1)) * DOTS_Y;
    let dot = (span_x / dots_x).max(span_y / dots_y) / f64::from(graph.zoom.max(0.01));

    let half_x = dot * dots_x / 2.0;
    let half_y = dot * dots_y / 2.0;
    let center = graph.center;

    (
        [f64::from(center.x) - half_x, f64::from(center.x) + half_x],
        [f64::from(center.y) - half_y, f64::from(center.y) + half_y],
        dot,
    )
}

/// Points filling a disc, stepped across the canvas's dot lattice so the shape
/// comes out solid at every size.
fn disc(center: Vec2, radius: f64, dot: f64) -> Vec<(f64, f64)> {
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    if dot <= 0.0 || radius <= dot {
        return vec![(cx, cy)];
    }
    // A node covering more of the screen than this means the viewport is wrong,
    // and rasterizing it point by point would only make that slow as well.
    let steps = ((radius / dot).ceil() as i32).min(64);
    let limit = radius * radius;
    let mut points = Vec::with_capacity(((2 * steps + 1) * (2 * steps + 1)) as usize);
    for gy in -steps..=steps {
        for gx in -steps..=steps {
            let dx = f64::from(gx) * dot;
            let dy = f64::from(gy) * dot;
            if dx * dx + dy * dy <= limit {
                points.push((cx + dx, cy + dy));
            }
        }
    }
    points
}

/// Points forming an unfilled circle, sampled densely enough that neighbouring
/// samples land on adjacent dots and the outline reads as continuous.
fn ring(center: Vec2, radius: f64, dot: f64) -> Vec<(f64, f64)> {
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    if dot <= 0.0 || radius <= dot {
        return vec![(cx, cy)];
    }
    let steps = ((std::f64::consts::TAU * radius / dot).ceil() as usize).clamp(8, 512);
    (0..steps)
        .map(|i| {
            let angle = std::f64::consts::TAU * i as f64 / steps as f64;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect()
}

/// A node's on-screen half-extent in cells: columns each side, then rows each
/// side. Rounded up, because a disc that isn't aligned to the cell grid bleeds
/// into the next row or column.
fn label_offsets(degree: usize) -> (u16, u16) {
    let radius = radius_dots(degree);
    (
        (radius / DOTS_X).ceil() as u16,
        (radius / DOTS_Y).ceil() as u16,
    )
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
    let nodes = &graph.simulation.graph.nodes;

    let shown = |index: usize| neighborhood.is_none_or(|set| set.contains(&index));

    // Every node is reserved before any label is placed, so a label never
    // covers the thing it names — or one of its neighbours.
    let mut occupied: Vec<Rect> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if !shown(index) {
            continue;
        }
        if let Some((x, y)) = project(node.pos, area, x_bounds, y_bounds) {
            let (half_x, half_y) = label_offsets(node.degree);
            occupied.push(Rect {
                x: x.saturating_sub(half_x),
                y: y.saturating_sub(half_y),
                width: half_x * 2 + 1,
                height: half_y * 2 + 1,
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
        if !shown(index) {
            continue;
        }
        let node = &nodes[index];
        let selected = graph.selected == Some(index);

        let Some((x, y)) = project(node.pos, area, x_bounds, y_bounds) else {
            continue;
        };

        let label = truncate(&node.label, 18);
        let width = label.chars().count() as u16;
        let (_, half_y) = label_offsets(node.degree);
        // Centring the label on the node can push it past the pane's left edge,
        // where it would be drawn over the neighbouring pane; clamp instead.
        let rect = Rect {
            x: x.saturating_sub(width / 2).max(area.x),
            y: y.saturating_add(half_y + 1),
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
    fn a_disc_is_solid_on_the_dot_lattice() {
        let points = disc(Vec2::default(), 4.0, 1.0);
        assert!(points.contains(&(0.0, 0.0)));
        assert!(points.contains(&(3.0, 0.0)));
        assert!(points.contains(&(2.0, 2.0)));
        assert!(
            !points.iter().any(|(x, y)| x * x + y * y > 16.0),
            "nothing outside the radius"
        );
        assert!(points.len() > 40, "a radius-4 disc covers ~49 lattice dots");
    }

    #[test]
    fn a_tiny_node_is_a_single_dot() {
        assert_eq!(disc(Vec2::default(), 0.5, 1.0).len(), 1);
        assert_eq!(ring(Vec2::default(), 0.5, 1.0).len(), 1);
    }

    #[test]
    fn a_ring_is_hollow_and_densely_sampled() {
        let radius = 4.0;
        let points = ring(Vec2::default(), radius, 1.0);
        assert!(
            points.len() >= 25,
            "samples must land at most a dot apart, got {}",
            points.len()
        );
        for (x, y) in &points {
            assert!(
                (x.hypot(*y) - radius).abs() < 1e-6,
                "every point sits on the circle"
            );
        }
    }

    #[test]
    fn rasterizing_a_huge_node_stays_bounded() {
        // A degenerate viewport must not turn into a multi-million-point loop.
        assert!(disc(Vec2::default(), 1e6, 1.0).len() <= 129 * 129);
    }

    #[test]
    fn node_size_grows_with_connectedness() {
        assert!(radius_dots(0) < radius_dots(3));
        assert!(radius_dots(3) < radius_dots(30));
    }
}

#[cfg(test)]
mod label_layout_tests {
    use super::{label_offsets, radius_dots, DOTS_X, DOTS_Y};

    #[test]
    fn a_label_clears_the_whole_node_it_names() {
        for degree in [0, 1, 3, 8, 30] {
            let radius = radius_dots(degree);
            let (half_x, half_y) = label_offsets(degree);
            // The reserved box has to contain the disc, or the label lands on
            // top of the node — the bug this guards against.
            assert!(
                f64::from(half_x) * DOTS_X >= radius,
                "degree {degree}: {half_x} columns is too narrow for radius {radius}"
            );
            assert!(
                f64::from(half_y) * DOTS_Y >= radius,
                "degree {degree}: {half_y} rows is too short for radius {radius}"
            );
        }
    }

    #[test]
    fn even_the_smallest_node_reserves_a_cell() {
        let (half_x, half_y) = label_offsets(0);
        assert!(half_x >= 1 && half_y >= 1, "a node always occupies a cell");
    }
}
