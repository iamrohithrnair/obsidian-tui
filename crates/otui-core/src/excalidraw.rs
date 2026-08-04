//! Reading Excalidraw drawings.
//!
//! Two files on disk hold the same scene. A `.excalidraw` file is the plain JSON
//! that excalidraw.com writes. A `.excalidraw.md` file is what Obsidian's
//! Excalidraw plugin writes: a Markdown note whose drawing lives in a fenced
//! block under a `## Drawing` heading, either as JSON or — the plugin's default
//! now — as LZ-String compressed base64, chunked across lines.
//!
//! What comes out is geometry: shapes with positions, sizes and colours. Turning
//! that into something on screen is the renderer's problem, and deliberately not
//! this module's, because the same scene is drawn very differently at 40 columns
//! and at 200.

use serde::Deserialize;

/// A parsed scene, in Excalidraw's own coordinate space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub shape: Shape,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Rotation about the element's centre, in radians.
    pub angle: f64,
    pub stroke: Option<Rgb>,
    /// The fill, if the shape is filled at all.
    pub fill: Option<Rgb>,
    /// A dashed or dotted outline rather than a solid one.
    pub dashed: bool,
    /// Vertices of a line, arrow or freehand stroke, relative to `x`/`y`.
    pub points: Vec<(f64, f64)>,
    /// The words of a text element, empty for everything else.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rectangle,
    Ellipse,
    Diamond,
    /// An open or closed polyline.
    Line,
    /// A line with a head on it.
    Arrow,
    /// A freehand stroke.
    Freehand,
    Text,
    /// An embedded picture, or anything else with an outline and nothing known
    /// inside it.
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The smallest box holding every element, as `(x, y, width, height)`.
///
/// `None` for an empty drawing, which has no box rather than a zero-sized one at
/// the origin.
impl Drawing {
    #[must_use]
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut min = (f64::MAX, f64::MAX);
        let mut max = (f64::MIN, f64::MIN);
        for element in &self.elements {
            for (x, y) in element.corners() {
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            }
        }
        (min.0 <= max.0).then_some((min.0, min.1, max.0 - min.0, max.1 - min.1))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl Element {
    /// The extreme points of the element, for measuring the scene.
    ///
    /// A line's declared width and height are the span of its points, but its
    /// points may run backwards from the origin, so they are measured directly.
    fn corners(&self) -> Vec<(f64, f64)> {
        if self.points.is_empty() {
            return vec![
                (self.x, self.y),
                (self.x + self.width, self.y + self.height),
            ];
        }
        self.points
            .iter()
            .map(|&(dx, dy)| (self.x + dx, self.y + dy))
            .collect()
    }
}

/// Reads a drawing from the contents of a `.excalidraw` or `.excalidraw.md` file.
///
/// Returns `None` when the text holds no drawing at all, which is how a caller
/// tells an Excalidraw note from an ordinary one.
#[must_use]
pub fn parse(content: &str) -> Option<Drawing> {
    let scene = scene_json(content)?;
    let scene: Scene = serde_json::from_str(&scene).ok()?;
    Some(Drawing {
        elements: scene
            .elements
            .into_iter()
            .filter(|raw| !raw.is_deleted)
            .map(Element::from)
            .collect(),
    })
}

/// Whether a file holds an Excalidraw drawing, by name alone.
///
/// Obsidian's plugin marks its notes with `excalidraw-plugin` in the
/// frontmatter, but the name is enough and costs nothing to check.
#[must_use]
pub fn is_drawing(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    lower.ends_with(".excalidraw") || lower.ends_with(".excalidraw.md")
}

/// Pulls the scene JSON out of whichever container it arrived in.
fn scene_json(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('{') {
        return Some(trimmed.to_string());
    }

    // Obsidian's format. The heading is `# Drawing` or `## Drawing`, and the
    // plugin allows text between it and the fence.
    let after = section(content, "Drawing")?;
    if let Some(body) = fenced(after, "compressed-json") {
        // Chunked across lines for the sake of version control, and the line
        // breaks are not part of the payload.
        let packed: String = body.split_whitespace().collect();
        let wide = lz_str::decompress_from_base64(&packed)?;
        return String::from_utf16(&wide).ok();
    }
    fenced(after, "json").map(str::to_string)
}

/// The text after a `# Heading` or `## Heading` line.
fn section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    content.lines().enumerate().find_map(|(index, line)| {
        let rest = line.trim_end().trim_start_matches('#');
        (rest.trim() == heading && rest.len() < line.trim_end().len()).then(|| {
            let consumed: usize = content
                .lines()
                .take(index + 1)
                .map(|line| line.len() + 1)
                .sum();
            content
                .get(consumed.min(content.len())..)
                .unwrap_or_default()
        })
    })
}

/// The body of the first ```` ```lang ```` fence, if that is the fence that
/// opens.
///
/// Only the first fence counts: a note whose drawing is compressed must not have
/// a later plain-JSON block read as the scene.
fn fenced<'a>(content: &'a str, lang: &str) -> Option<&'a str> {
    let opening = content.find("```")?;
    let after = &content[opening + 3..];
    let (tag, body) = after.split_once('\n')?;
    if tag.trim() != lang {
        return None;
    }
    let end = body.find("```")?;
    Some(&body[..end])
}

// ---------------------------------------------------------------------------
// The JSON shape, kept private so the model above can outlive Excalidraw's
// schema changes.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Scene {
    #[serde(default)]
    elements: Vec<Raw>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    angle: f64,
    #[serde(default)]
    stroke_color: String,
    #[serde(default)]
    background_color: String,
    #[serde(default)]
    fill_style: String,
    #[serde(default)]
    stroke_style: String,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    points: Vec<Vec<f64>>,
    #[serde(default)]
    text: String,
}

impl From<Raw> for Element {
    fn from(raw: Raw) -> Self {
        let shape = match raw.r#type.as_str() {
            "rectangle" => Shape::Rectangle,
            "ellipse" => Shape::Ellipse,
            "diamond" => Shape::Diamond,
            "line" => Shape::Line,
            "arrow" => Shape::Arrow,
            "freedraw" => Shape::Freehand,
            "text" => Shape::Text,
            // Images, frames, embedded pages and anything a later Excalidraw
            // adds: an outline in the right place says more than nothing does.
            _ => Shape::Frame,
        };

        Element {
            shape,
            x: raw.x,
            y: raw.y,
            width: raw.width,
            height: raw.height,
            angle: raw.angle,
            stroke: color(&raw.stroke_color),
            // `fillStyle` describes how to fill, and Excalidraw keeps whatever
            // was last chosen even for a shape left transparent.
            fill: (raw.fill_style != "none")
                .then(|| color(&raw.background_color))
                .flatten(),
            dashed: !raw.stroke_style.is_empty() && raw.stroke_style != "solid",
            points: raw
                .points
                .iter()
                .filter_map(|point| match point.as_slice() {
                    [x, y, ..] => Some((*x, *y)),
                    _ => None,
                })
                .collect(),
            text: raw.text,
        }
    }
}

/// Parses `#rgb`, `#rrggbb` or `transparent`.
fn color(value: &str) -> Option<Rgb> {
    let hex = value.trim().strip_prefix('#')?;
    let pair = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
    let single = |index: usize| {
        u8::from_str_radix(hex.get(index..index + 1)?, 16)
            .ok()
            // `#f00` means `#ff0000`, not `#0f0000`.
            .map(|value| value * 17)
    };
    match hex.len() {
        3 => Some(Rgb {
            r: single(0)?,
            g: single(1)?,
            b: single(2)?,
        }),
        6 | 8 => Some(Rgb {
            r: pair(0)?,
            g: pair(2)?,
            b: pair(4)?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = r##"{
      "type": "excalidraw",
      "version": 2,
      "elements": [
        {"type": "rectangle", "x": 10, "y": 20, "width": 100, "height": 50,
         "strokeColor": "#1e1e1e", "backgroundColor": "#ffc9c9",
         "fillStyle": "solid", "strokeStyle": "solid"},
        {"type": "text", "x": 20, "y": 30, "width": 40, "height": 25,
         "text": "hello", "strokeColor": "#1971c2", "fillStyle": "hachure",
         "backgroundColor": "transparent"},
        {"type": "arrow", "x": 200, "y": 200, "width": 50, "height": 0,
         "points": [[0, 0], [50, 0]], "strokeColor": "#2f9e44",
         "strokeStyle": "dashed"},
        {"type": "ellipse", "x": 0, "y": 0, "width": 10, "height": 10,
         "isDeleted": true}
      ]
    }"##;

    #[test]
    fn a_plain_excalidraw_file_gives_up_its_shapes() {
        let drawing = parse(PLAIN).expect("parsed");

        assert_eq!(
            drawing.elements.len(),
            3,
            "a deleted element is history, not part of the picture"
        );

        let rect = &drawing.elements[0];
        assert_eq!(rect.shape, Shape::Rectangle);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (10.0, 20.0, 100.0, 50.0)
        );
        assert_eq!(
            rect.stroke,
            Some(Rgb {
                r: 30,
                g: 30,
                b: 30
            })
        );
        assert_eq!(
            rect.fill,
            Some(Rgb {
                r: 255,
                g: 201,
                b: 201
            })
        );

        let text = &drawing.elements[1];
        assert_eq!(text.shape, Shape::Text);
        assert_eq!(text.text, "hello");
        assert_eq!(text.fill, None, "transparent is not a fill");

        let arrow = &drawing.elements[2];
        assert_eq!(arrow.shape, Shape::Arrow);
        assert_eq!(arrow.points, vec![(0.0, 0.0), (50.0, 0.0)]);
        assert!(arrow.dashed);
    }

    #[test]
    fn bounds_cover_every_shape_and_follow_lines_that_run_backwards() {
        let drawing = parse(PLAIN).expect("parsed");
        let (x, y, width, height) = drawing.bounds().expect("bounds");
        assert_eq!((x, y), (10.0, 20.0));
        assert_eq!((x + width, y + height), (250.0, 200.0));

        // A line's points may go left and up from its origin, which a naive
        // x + width would miss entirely.
        let back = parse(
            r##"{"elements": [{"type": "line", "x": 100, "y": 100,
                 "width": 40, "height": 40, "points": [[0, 0], [-40, -40]]}]}"##,
        )
        .expect("parsed");
        assert_eq!(back.bounds(), Some((60.0, 60.0, 40.0, 40.0)));
    }

    #[test]
    fn an_empty_drawing_has_no_bounds_at_all() {
        let drawing = parse(r##"{"elements": []}"##).expect("parsed");
        assert!(drawing.is_empty());
        assert_eq!(
            drawing.bounds(),
            None,
            "not a zero-sized box at the origin, which would scale to nothing"
        );
    }

    #[test]
    fn an_obsidian_note_gives_up_the_drawing_under_its_heading() {
        let note = format!(
            "---\nexcalidraw-plugin: parsed\n---\n\n\
             # Excalidraw Data\n\n## Text Elements\nhello ^abc123\n\n\
             ## Drawing\n```json\n{PLAIN}\n```\n%%\n"
        );
        let drawing = parse(&note).expect("parsed");
        assert_eq!(drawing.elements.len(), 3);
    }

    #[test]
    fn an_obsidian_note_decompresses_its_drawing() {
        // What the plugin writes by default: compressed base64, wrapped across
        // lines for the sake of version control.
        let packed = lz_str::compress_to_base64(PLAIN);
        let wrapped: String = packed
            .as_bytes()
            .chunks(64)
            .map(|chunk| String::from_utf8_lossy(chunk).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let note = format!("---\nexcalidraw-plugin: parsed\n---\n\n## Drawing\n```compressed-json\n{wrapped}\n```\n%%\n");

        let drawing = parse(&note).expect("parsed");
        assert_eq!(drawing.elements.len(), 3);
        assert_eq!(drawing.elements[1].text, "hello");
    }

    #[test]
    fn a_note_that_only_talks_about_drawings_is_not_one() {
        assert!(parse("# My notes\n\nI should learn Excalidraw.\n").is_none());
        assert!(
            parse("## Drawing\n\nJust a heading, no data.\n").is_none(),
            "a heading alone is not a drawing"
        );
        assert!(
            parse("## Drawing\n```json\nnot json at all\n```\n").is_none(),
            "a fence full of nonsense is not a drawing"
        );
    }

    #[test]
    fn drawings_are_recognised_by_name() {
        assert!(is_drawing("diagrams/Flow.excalidraw.md"));
        assert!(is_drawing("Legacy.excalidraw"));
        assert!(is_drawing("SHOUTING.EXCALIDRAW.MD"));
        assert!(!is_drawing("notes/About excalidraw.md"));
        assert!(!is_drawing("chart.png"));
    }

    #[test]
    fn colors_are_read_in_every_form_excalidraw_writes() {
        assert_eq!(
            color("#ff8800"),
            Some(Rgb {
                r: 255,
                g: 136,
                b: 0
            })
        );
        assert_eq!(
            color("#f80"),
            Some(Rgb {
                r: 255,
                g: 136,
                b: 0
            }),
            "the short form doubles each digit"
        );
        assert_eq!(
            color("#ff8800ff"),
            Some(Rgb {
                r: 255,
                g: 136,
                b: 0
            }),
            "an alpha channel is ignored rather than rejected"
        );
        assert_eq!(color("transparent"), None);
        assert_eq!(color(""), None);
    }
}
