//! Markdown parsing into a render model.
//!
//! This produces a structured document, not styled terminal output — the TUI
//! crate owns colors and layout. Keeping the split means the parser is testable
//! without a terminal, and the same document feeds the reading pane, the
//! outline and the agent's note summaries.
//!
//! The dialect is Obsidian's: CommonMark blocks plus wikilinks, embeds, tags,
//! `==highlights==` and `> [!note]` callouts. It is deliberately line-oriented.
//! A terminal renders line by line and the user edits line by line, so a
//! line-oriented model maps directly onto both, and it stays fast on large
//! notes because no line is ever visited twice.

use crate::note;

/// A parsed note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// A block together with the 0-based file line it starts on.
///
/// The line number is what lets the reading pane scroll to the same place the
/// editor is on, and lets the outline jump to a heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub line: usize,
    pub kind: BlockKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// The YAML frontmatter block, as ordered key/value text.
    Frontmatter(Vec<(String, String)>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    ListItem {
        /// Nesting depth, 0 for a top-level item.
        depth: usize,
        marker: Marker,
        spans: Vec<Span>,
    },
    /// A `> quote`. Nested quotes appear as nested blocks.
    Quote(Vec<Block>),
    /// A `> [!note] Title` callout.
    Callout {
        kind: String,
        title: Vec<Span>,
        body: Vec<Block>,
    },
    Code {
        lang: String,
        lines: Vec<String>,
    },
    Table(Table),
    Rule,
    Blank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    Ordered(u64),
    /// A `- [ ]` / `- [x]` task; `true` when checked.
    Task(bool),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<Vec<Span>>,
    pub rows: Vec<Vec<Vec<Span>>>,
    pub aligns: Vec<Align>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A run of text with uniform styling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub kind: SpanKind,
}

impl Span {
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::default(),
            kind: SpanKind::Text,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub highlight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanKind {
    Text,
    /// A `[[target]]` link. Whether it resolves is the index's business, not
    /// the parser's.
    WikiLink {
        target: String,
        anchor: Option<String>,
        embed: bool,
    },
    /// A `[text](url)` link.
    Link {
        url: String,
    },
    /// An `![alt](src)` embed. Whether `src` names something displayable is the
    /// renderer's business; the `!` only says the author wanted it shown rather
    /// than linked. `text` holds the alt text.
    Image {
        src: String,
    },
    /// An inline `#tag`, without the `#`.
    Tag(String),
    /// `$inline math$`, passed through verbatim.
    Math,
}

/// Parses a full note, frontmatter included.
#[must_use]
pub fn parse(content: &str) -> Document {
    let (fm_text, body, body_offset) = note::split_frontmatter(content);
    let mut blocks = Vec::new();

    if let Some(fm_text) = fm_text {
        let fm = note::parse_frontmatter(fm_text);
        blocks.push(Block {
            line: 0,
            kind: BlockKind::Frontmatter(
                fm.entries()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_scalar()))
                    .collect(),
            ),
        });
    }

    let lines: Vec<&str> = body.lines().collect();
    parse_blocks(&lines, body_offset, &mut blocks);
    Document { blocks }
}

/// Parses a body that has already had its frontmatter removed.
#[must_use]
pub fn parse_body(body: &str, line_offset: usize) -> Document {
    let lines: Vec<&str> = body.lines().collect();
    let mut blocks = Vec::new();
    parse_blocks(&lines, line_offset, &mut blocks);
    Document { blocks }
}

fn parse_blocks(lines: &[&str], offset: usize, out: &mut Vec<Block>) {
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let file_line = offset + i;

        if trimmed.is_empty() {
            out.push(Block {
                line: file_line,
                kind: BlockKind::Blank,
            });
            i += 1;
            continue;
        }

        if let Some((fence_char, fence_len)) = opening_fence(trimmed) {
            let lang = trimmed[fence_len..].trim().to_string();
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() {
                let candidate = lines[i].trim_start();
                if is_closing_fence(candidate, fence_char, fence_len) {
                    i += 1;
                    break;
                }
                body.push(lines[i].to_string());
                i += 1;
            }
            out.push(Block {
                line: file_line,
                kind: BlockKind::Code { lang, lines: body },
            });
            continue;
        }

        if is_rule(trimmed) {
            out.push(Block {
                line: file_line,
                kind: BlockKind::Rule,
            });
            i += 1;
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            out.push(Block {
                line: file_line,
                kind: BlockKind::Heading {
                    level,
                    spans: parse_inline(text),
                },
            });
            i += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            let start = i;
            let mut inner: Vec<String> = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let t = lines[i].trim_start();
                // Strip one `>` and at most one following space, so nested
                // quotes survive to the recursive call.
                let rest = &t[1..];
                inner.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                i += 1;
            }
            out.push(parse_quote_or_callout(&inner, offset + start));
            continue;
        }

        if let Some((depth, marker, text)) = list_item(line) {
            out.push(Block {
                line: file_line,
                kind: BlockKind::ListItem {
                    depth,
                    marker,
                    spans: parse_inline(text),
                },
            });
            i += 1;
            continue;
        }

        if let Some((table, consumed)) = parse_table(&lines[i..]) {
            out.push(Block {
                line: file_line,
                kind: BlockKind::Table(table),
            });
            i += consumed;
            continue;
        }

        // A paragraph runs until a blank line or a line that starts some other
        // block, so reading mode reflows it as one unit the way a renderer
        // would rather than showing the author's hard wraps.
        let mut text = String::new();
        while i < lines.len() {
            let candidate = lines[i];
            let ct = candidate.trim_start();
            if ct.is_empty()
                || opening_fence(ct).is_some()
                || is_rule(ct)
                || heading(ct).is_some()
                || ct.starts_with('>')
                || list_item(candidate).is_some()
            {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(candidate.trim());
            i += 1;
        }
        out.push(Block {
            line: file_line,
            kind: BlockKind::Paragraph(parse_inline(&text)),
        });
    }
}

fn parse_quote_or_callout(inner: &[String], line: usize) -> Block {
    if let Some(first) = inner.first()
        && let Some(rest) = first.trim_start().strip_prefix("[!")
        && let Some((kind, title)) = rest.split_once(']')
    {
        let kind = kind.trim().to_lowercase();
        // A trailing `+`/`-` marks a foldable callout; the marker isn't
        // part of the title.
        let title = title.trim_start_matches(['+', '-']).trim();
        let refs: Vec<&str> = inner[1..].iter().map(String::as_str).collect();
        let mut body = Vec::new();
        parse_blocks(&refs, line + 1, &mut body);
        return Block {
            line,
            kind: BlockKind::Callout {
                kind,
                title: parse_inline(title),
                body,
            },
        };
    }

    let refs: Vec<&str> = inner.iter().map(String::as_str).collect();
    let mut body = Vec::new();
    parse_blocks(&refs, line, &mut body);
    Block {
        line,
        kind: BlockKind::Quote(body),
    }
}

fn opening_fence(trimmed: &str) -> Option<(char, usize)> {
    for ch in ['`', '~'] {
        let len = trimmed.chars().take_while(|&c| c == ch).count();
        if len >= 3 {
            return Some((ch, len));
        }
    }
    None
}

fn is_closing_fence(trimmed: &str, fence_char: char, fence_len: usize) -> bool {
    let len = trimmed.chars().take_while(|&c| c == fence_char).count();
    len >= fence_len && trimmed[len..].trim().is_empty()
}

fn is_rule(trimmed: &str) -> bool {
    for ch in ['-', '*', '_'] {
        let stripped: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        if stripped.len() >= 3 && stripped.chars().all(|c| c == ch) {
            return true;
        }
    }
    false
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.bytes().take_while(|&b| b == b'#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    // Without a space it's a tag, not a heading.
    let text = rest.strip_prefix(' ')?;
    Some((level as u8, text.trim().trim_end_matches('#').trim()))
}

fn list_item(line: &str) -> Option<(usize, Marker, &str)> {
    let indent = line
        .chars()
        .take_while(|&c| c == ' ' || c == '\t')
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum::<usize>();
    let trimmed = line.trim_start();

    // Obsidian indents nested lists by a tab or 2–4 spaces; dividing by 2 maps
    // every common style onto the same depth ladder.
    let depth = indent / 2;

    for bullet in ['-', '*', '+'] {
        if let Some(rest) = trimmed
            .strip_prefix(bullet)
            .and_then(|r| r.strip_prefix(' '))
        {
            if let Some(task) = rest.strip_prefix('[')
                && task.len() >= 2
                && task.as_bytes()[1] == b']'
            {
                let state = task.as_bytes()[0];
                let text = task[2..].strip_prefix(' ').unwrap_or(&task[2..]);
                return Some((depth, Marker::Task(state != b' ' && state != b'\t'), text));
            }
            return Some((depth, Marker::Bullet, rest));
        }
    }

    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        let rest = &trimmed[digits..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            let n = trimmed[..digits].parse().unwrap_or(1);
            return Some((depth, Marker::Ordered(n), text));
        }
    }

    None
}

fn parse_table(lines: &[&str]) -> Option<(Table, usize)> {
    let header_line = lines.first()?.trim();
    let separator = lines.get(1)?.trim();
    if !header_line.contains('|') || !is_table_separator(separator) {
        return None;
    }

    let aligns: Vec<Align> = split_row(separator)
        .into_iter()
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            }
        })
        .collect();

    let header = split_row(header_line)
        .into_iter()
        .map(|c| parse_inline(c.trim()))
        .collect();

    let mut rows = Vec::new();
    let mut consumed = 2;
    while let Some(line) = lines.get(consumed) {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains('|') {
            break;
        }
        rows.push(
            split_row(trimmed)
                .into_iter()
                .map(|c| parse_inline(c.trim()))
                .collect(),
        );
        consumed += 1;
    }

    Some((
        Table {
            header,
            rows,
            aligns,
        },
        consumed,
    ))
}

fn is_table_separator(line: &str) -> bool {
    if !line.contains('-') || !line.contains('|') {
        return false;
    }
    line.chars()
        .all(|c| matches!(c, '-' | ':' | '|' | ' ' | '\t'))
}

fn split_row(line: &str) -> Vec<&str> {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    trimmed.split('|').collect()
}

// ---------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------

/// Parses inline markup into styled spans.
///
/// Delimiters are matched greedily left to right; an unclosed delimiter is
/// emitted as literal text rather than swallowing the rest of the line, which
/// matters because a note is read while it's being typed.
#[must_use]
pub fn parse_inline(text: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut buffer = String::new();
    let mut i = 0;

    while i < text.len() {
        let rest = &text[i..];

        // Escapes are handled here rather than in `match_inline` because they
        // contribute a character to the plain-text run instead of a span.
        if let Some(escaped) = rest.strip_prefix('\\').and_then(|r| r.chars().next()) {
            buffer.push(escaped);
            i += 1 + escaped.len_utf8();
            continue;
        }

        // A `#tag` must not sit inside a word, so `C#` and `100#2` stay text.
        let after_word = text[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-'));

        if let Some((consumed, produced)) = match_inline(rest, !after_word) {
            if !buffer.is_empty() {
                spans.push(Span::plain(std::mem::take(&mut buffer)));
            }
            spans.extend(produced);
            i += consumed;
            continue;
        }

        // No construct starts here: take one character as literal text. This is
        // also what makes an unclosed `**` render as itself instead of eating
        // the rest of the line.
        let ch = rest.chars().next().unwrap_or('\0');
        buffer.push(ch);
        i += ch.len_utf8();
    }

    if !buffer.is_empty() {
        spans.push(Span::plain(buffer));
    }
    spans
}

/// A delimiter and the style it applies to whatever it wraps.
type Delimiter = (&'static str, fn(Style) -> Style);

/// Style transforms for the two-character emphasis delimiters.
const DOUBLE_DELIMITERS: &[Delimiter] = &[
    ("**", |s| Style { bold: true, ..s }),
    ("__", |s| Style { bold: true, ..s }),
    ("==", |s| Style {
        highlight: true,
        ..s
    }),
    ("~~", |s| Style {
        strikethrough: true,
        ..s
    }),
];

/// Tries every inline construct at the start of `rest`.
///
/// Returns the bytes consumed and the spans produced, or `None` if no
/// construct starts here. Every branch requires a closing delimiter before it
/// commits, so an unterminated one falls through to literal text.
fn match_inline(rest: &str, tag_boundary: bool) -> Option<(usize, Vec<Span>)> {
    // Embeds and wikilinks are checked before Markdown links, since `![[x]]`
    // and `[[x]]` both start with characters a Markdown link also uses.
    if let Some(after) = rest.strip_prefix("![[") {
        let end = after.find("]]")?;
        return Some((3 + end + 2, vec![wikilink_span(&after[..end], true)]));
    }
    if let Some(after) = rest.strip_prefix("[[") {
        if let Some(end) = after.find("]]") {
            return Some((2 + end + 2, vec![wikilink_span(&after[..end], false)]));
        }
        return None;
    }
    if (rest.starts_with('[') || rest.starts_with("!["))
        && let Some((consumed, span)) = markdown_link(rest)
    {
        return Some((consumed, vec![span]));
    }

    // Inline code wins over emphasis: backticks suppress markup inside them.
    if rest.starts_with('`') {
        let (consumed, inner) = delimited_text(rest, "`")?;
        return Some((
            consumed,
            vec![Span {
                text: inner.to_string(),
                style: Style {
                    code: true,
                    ..Style::default()
                },
                kind: SpanKind::Text,
            }],
        ));
    }

    for (delim, apply) in DOUBLE_DELIMITERS {
        if rest.starts_with(delim)
            && let Some((consumed, inner)) = delimited_text(rest, delim)
        {
            // The inside is parsed too, so `**bold `code`**` keeps both.
            let spans = parse_inline(inner)
                .into_iter()
                .map(|mut span| {
                    span.style = apply(span.style);
                    span
                })
                .collect();
            return Some((consumed, spans));
        }
    }

    // Single-character emphasis is tried after the doubles so `**` is never
    // read as two nested italics.
    for delim in ["*", "_"] {
        let doubled = rest.starts_with(&delim.repeat(2));
        if rest.starts_with(delim)
            && !doubled
            && let Some((consumed, inner)) = delimited_text(rest, delim)
        {
            let spans = parse_inline(inner)
                .into_iter()
                .map(|mut span| {
                    span.style.italic = true;
                    span
                })
                .collect();
            return Some((consumed, spans));
        }
    }

    if rest.starts_with('$')
        && let Some((consumed, inner)) = delimited_text(rest, "$")
    {
        return Some((
            consumed,
            vec![Span {
                text: inner.to_string(),
                style: Style::default(),
                kind: SpanKind::Math,
            }],
        ));
    }

    if tag_boundary && rest.starts_with('#') {
        let raw: String = rest[1..]
            .chars()
            .take_while(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))
            .collect();
        let tag = raw.trim_end_matches('/');
        // A purely numeric `#1` is an issue reference, not a tag.
        if !tag.is_empty() && !tag.bytes().all(|b| b.is_ascii_digit()) {
            return Some((
                1 + raw.len(),
                vec![Span {
                    text: format!("#{tag}"),
                    style: Style::default(),
                    kind: SpanKind::Tag(tag.to_string()),
                }],
            ));
        }
    }

    None
}

fn wikilink_span(inner: &str, embed: bool) -> Span {
    let (target_part, alias) = match inner.split_once('|') {
        Some((t, a)) => (t, Some(a.trim().to_string())),
        None => (inner, None),
    };
    let (target, anchor) = match target_part.split_once('#') {
        Some((t, a)) => (t.trim(), Some(a.trim().to_string())),
        None => (target_part.trim(), None),
    };

    let display = alias.unwrap_or_else(|| match &anchor {
        Some(anchor) if target.is_empty() => anchor.clone(),
        Some(anchor) => format!("{target} › {anchor}"),
        None => target.to_string(),
    });

    Span {
        text: display,
        style: Style::default(),
        kind: SpanKind::WikiLink {
            target: target.to_string(),
            anchor,
            embed,
        },
    }
}

/// Matches `[text](url)` or `![alt](src)`, returning the bytes consumed.
fn markdown_link(rest: &str) -> Option<(usize, Span)> {
    let embed = rest.starts_with('!');
    let open = usize::from(embed);
    let text_end = rest.find(']')?;
    if !rest[text_end + 1..].starts_with('(') {
        return None;
    }
    let url_end = rest[text_end + 2..].find(')')?;
    let label = &rest[open + 1..text_end];
    let url = rest[text_end + 2..text_end + 2 + url_end].trim();

    Some((
        text_end + 2 + url_end + 1,
        Span {
            // An embed keeps its alt text even when empty: the renderer shows
            // the picture, and only falls back to text when it cannot.
            text: if label.is_empty() && !embed {
                url.to_string()
            } else {
                label.to_string()
            },
            style: Style::default(),
            kind: if embed {
                SpanKind::Image {
                    src: url.to_string(),
                }
            } else {
                SpanKind::Link {
                    url: url.to_string(),
                }
            },
        },
    ))
}

/// Finds `delim…delim` and returns the total consumed length plus the inner text.
fn delimited_text<'a>(rest: &'a str, delim: &str) -> Option<(usize, &'a str)> {
    let after = &rest[delim.len()..];
    let end = after.find(delim)?;
    if end == 0 {
        return None; // `**` with nothing between is literal
    }
    Some((delim.len() * 2 + end, &after[..end]))
}

/// Flattens spans back to plain text, for search snippets and note previews.
#[must_use]
pub fn spans_to_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// What one character of a source line is, for colouring it where it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ink {
    #[default]
    Text,
    /// Syntax rather than content: a `**`, a `[[`, the `#` of a heading.
    Marker,
    WikiLink,
    Link,
    Tag,
    Math,
}

impl Ink {
    fn of(kind: &SpanKind) -> Self {
        match kind {
            SpanKind::Text => Self::Text,
            SpanKind::WikiLink { .. } => Self::WikiLink,
            SpanKind::Link { .. } => Self::Link,
            // Alt text standing in for a picture reads as a link to it.
            SpanKind::Image { .. } => Self::Link,
            SpanKind::Tag(_) => Self::Tag,
            SpanKind::Math => Self::Math,
        }
    }
}

/// One character of a source line, and what to make of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Paint {
    pub style: Style,
    pub ink: Ink,
}

/// Describes a source line character by character, without moving any of it.
///
/// The editor shows Markdown as it was typed, so it needs to know what each
/// character *is* rather than what the reading pane would draw in its place.
/// The styles come from [`parse_inline`] — one definition of the dialect, not
/// two — and every character the parser dropped on the way is a delimiter, so
/// it is reported as [`Ink::Marker`] for the editor to dim.
///
/// The result always has one entry per character of `line`.
#[must_use]
pub fn scan_inline(line: &str) -> Vec<Paint> {
    let chars: Vec<char> = line.chars().collect();
    // Syntax until the parser claims it as content.
    let mut out = vec![
        Paint {
            style: Style::default(),
            ink: Ink::Marker,
        };
        chars.len()
    ];

    let mut at = 0;
    for span in parse_inline(line) {
        let paint = Paint {
            style: span.style,
            ink: Ink::of(&span.kind),
        };
        // Span text is always a subsequence of the line: the parser either
        // copies a character through or drops it.
        for want in span.text.chars() {
            while at < chars.len() && chars[at] != want {
                at += 1;
            }
            if at == chars.len() {
                return out;
            }
            out[at] = paint;
            at += 1;
        }
    }
    out
}

/// A short plain-text preview of a note body, as shown in list views.
#[must_use]
pub fn preview(body: &str, max_chars: usize) -> String {
    let doc = parse_body(body, 0);
    let mut out = String::new();

    for block in &doc.blocks {
        let text = match &block.kind {
            BlockKind::Paragraph(spans) | BlockKind::Heading { spans, .. } => spans_to_text(spans),
            BlockKind::ListItem { spans, .. } => spans_to_text(spans),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(text.trim());
        if out.chars().count() >= max_chars {
            break;
        }
    }

    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Document) -> Vec<&BlockKind> {
        doc.blocks.iter().map(|b| &b.kind).collect()
    }

    #[test]
    fn parses_headings_with_source_lines() {
        let doc = parse("---\ntitle: T\n---\n# One\n\n## Two\n");
        let headings: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, spans } => Some((*level, spans_to_text(spans), b.line)),
                _ => None,
            })
            .collect();
        assert_eq!(
            headings,
            vec![(1, "One".to_string(), 3), (2, "Two".to_string(), 5)],
            "lines must be file lines, counting the frontmatter"
        );
    }

    #[test]
    fn paragraph_lines_reflow_into_one_block() {
        let doc = parse_body("first line\nsecond line\n\nnew para\n", 0);
        let paragraphs: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Paragraph(spans) => Some(spans_to_text(spans)),
                _ => None,
            })
            .collect();
        assert_eq!(paragraphs, vec!["first line second line", "new para"]);
    }

    #[test]
    fn parses_code_fences_verbatim() {
        let doc = parse_body("```rust\nlet x = 1;\n# not a heading\n```\n", 0);
        match &doc.blocks[0].kind {
            BlockKind::Code { lang, lines } => {
                assert_eq!(lang, "rust");
                assert_eq!(lines, &["let x = 1;", "# not a heading"]);
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_fence_runs_to_end_of_note() {
        let doc = parse_body("```\nstill code\n", 0);
        assert!(matches!(doc.blocks[0].kind, BlockKind::Code { .. }));
    }

    #[test]
    fn parses_list_markers_and_depth() {
        let doc = parse_body("- one\n  - nested\n1. first\n- [ ] todo\n- [x] done\n", 0);
        let items: Vec<(usize, Marker, String)> = doc
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::ListItem {
                    depth,
                    marker,
                    spans,
                } => Some((*depth, *marker, spans_to_text(spans))),
                _ => None,
            })
            .collect();

        assert_eq!(items[0], (0, Marker::Bullet, "one".into()));
        assert_eq!(items[1], (1, Marker::Bullet, "nested".into()));
        assert_eq!(items[2], (0, Marker::Ordered(1), "first".into()));
        assert_eq!(items[3], (0, Marker::Task(false), "todo".into()));
        assert_eq!(items[4], (0, Marker::Task(true), "done".into()));
    }

    #[test]
    fn parses_callouts_with_body() {
        let doc = parse_body("> [!warning] Careful\n> body text\n", 0);
        match &doc.blocks[0].kind {
            BlockKind::Callout { kind, title, body } => {
                assert_eq!(kind, "warning");
                assert_eq!(spans_to_text(title), "Careful");
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected callout, got {other:?}"),
        }
    }

    #[test]
    fn foldable_callout_marker_is_not_part_of_the_title() {
        let doc = parse_body("> [!note]- Collapsed\n", 0);
        match &doc.blocks[0].kind {
            BlockKind::Callout { title, .. } => assert_eq!(spans_to_text(title), "Collapsed"),
            other => panic!("expected callout, got {other:?}"),
        }
    }

    #[test]
    fn plain_quotes_stay_quotes_and_nest() {
        let doc = parse_body("> outer\n> > inner\n", 0);
        match &doc.blocks[0].kind {
            BlockKind::Quote(body) => {
                assert!(body.iter().any(|b| matches!(b.kind, BlockKind::Quote(_))));
            }
            other => panic!("expected quote, got {other:?}"),
        }
    }

    #[test]
    fn parses_tables_with_alignment() {
        let doc = parse_body("| A | B |\n|:--|--:|\n| 1 | 2 |\n", 0);
        match &doc.blocks[0].kind {
            BlockKind::Table(table) => {
                assert_eq!(table.aligns, vec![Align::Left, Align::Right]);
                assert_eq!(spans_to_text(&table.header[0]), "A");
                assert_eq!(table.rows.len(), 1);
                assert_eq!(spans_to_text(&table.rows[0][1]), "2");
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn a_pipe_line_without_a_separator_is_a_paragraph() {
        let doc = parse_body("a | b\nnot a table\n", 0);
        assert!(matches!(kinds(&doc)[0], BlockKind::Paragraph(_)));
    }

    #[test]
    fn parses_inline_emphasis_and_code() {
        let spans = parse_inline("**bold** *it* `code` ==mark== ~~gone~~");
        let bold = spans.iter().find(|s| s.text == "bold").unwrap();
        assert!(bold.style.bold);
        assert!(spans.iter().find(|s| s.text == "it").unwrap().style.italic);
        assert!(spans.iter().find(|s| s.text == "code").unwrap().style.code);
        assert!(
            spans
                .iter()
                .find(|s| s.text == "mark")
                .unwrap()
                .style
                .highlight
        );
        assert!(
            spans
                .iter()
                .find(|s| s.text == "gone")
                .unwrap()
                .style
                .strikethrough
        );
    }

    #[test]
    fn nested_emphasis_combines_styles() {
        let spans = parse_inline("**bold `code`**");
        let code = spans.iter().find(|s| s.text == "code").unwrap();
        assert!(code.style.bold && code.style.code);
    }

    #[test]
    fn wikilinks_display_alias_and_keep_target() {
        let spans = parse_inline("[[Note A|Display]] and [[B#Head]] and ![[img.png]]");

        let display = &spans[0];
        assert_eq!(display.text, "Display");
        assert!(matches!(
            &display.kind,
            SpanKind::WikiLink { target, embed, .. } if target == "Note A" && !embed
        ));

        let anchored = spans.iter().find(|s| s.text.contains("Head")).unwrap();
        assert!(matches!(
            &anchored.kind,
            SpanKind::WikiLink { anchor: Some(a), .. } if a == "Head"
        ));

        let embed = spans
            .iter()
            .find(|s| matches!(&s.kind, SpanKind::WikiLink { embed: true, .. }))
            .expect("embed span");
        assert!(matches!(&embed.kind, SpanKind::WikiLink { target, .. } if target == "img.png"));
    }

    #[test]
    fn markdown_links_and_tags() {
        let spans = parse_inline("see [docs](https://e.com) #project/alpha");
        assert!(spans.iter().any(|s| s.text == "docs"
            && matches!(&s.kind, SpanKind::Link { url } if url == "https://e.com")));
        assert!(
            spans
                .iter()
                .any(|s| matches!(&s.kind, SpanKind::Tag(t) if t == "project/alpha"))
        );
    }

    #[test]
    fn a_bang_makes_an_image_rather_than_a_link() {
        let spans = parse_inline("![a chart](assets/chart.png) vs [a chart](assets/chart.png)");

        assert!(
            matches!(&spans[0].kind, SpanKind::Image { src } if src == "assets/chart.png"),
            "the bang is what says show it, not link to it"
        );
        assert_eq!(
            spans[0].text, "a chart",
            "alt text is kept for the fallback"
        );
        assert!(
            spans
                .iter()
                .any(|s| matches!(&s.kind, SpanKind::Link { url } if url == "assets/chart.png"))
        );
    }

    #[test]
    fn an_image_without_alt_text_stays_empty_rather_than_showing_its_path() {
        let spans = parse_inline("![](a.png)");
        assert_eq!(
            spans[0].text, "",
            "a link falls back to its URL as a label, but an image has a picture to show"
        );
    }

    #[test]
    fn unclosed_delimiters_stay_literal() {
        assert_eq!(spans_to_text(&parse_inline("a ** b")), "a ** b");
        assert_eq!(spans_to_text(&parse_inline("[[unclosed")), "[[unclosed");
        assert_eq!(spans_to_text(&parse_inline("` open")), "` open");
    }

    #[test]
    fn escapes_are_honored() {
        let spans = parse_inline(r"\*not italic\*");
        assert_eq!(spans_to_text(&spans), "*not italic*");
        assert!(!spans.iter().any(|s| s.style.italic));
    }

    /// A one-character sketch of a scan: `.` content, `#` a delimiter.
    fn shape(line: &str) -> String {
        scan_inline(line)
            .iter()
            .map(|paint| if paint.ink == Ink::Marker { '#' } else { '.' })
            .collect()
    }

    #[test]
    fn scan_inline_marks_delimiters_where_they_sit() {
        // The editor draws source, so it needs one entry per character, with the
        // syntax told apart from the content rather than removed.
        assert_eq!(shape("**bold** text"), "##....##.....");
        assert_eq!(shape("a `code` b"), "..#....#..");
        assert_eq!(shape("see [[Note]]"), "....##....##");
        assert_eq!(shape("==mark=="), "##....##");
    }

    #[test]
    fn scan_inline_always_describes_every_character() {
        // Anything shorter would be indexed out of bounds while drawing.
        for line in [
            "",
            "plain",
            "a ** b",
            "[[unclosed",
            "` open",
            r"\*escaped\*",
            "héllo 日本語 **wide**",
            "![](a.png)",
            "[docs](https://e.com) #tag/deep",
            "**nested `code` here**",
        ] {
            assert_eq!(
                scan_inline(line).len(),
                line.chars().count(),
                "{line:?} was described by the wrong number of entries"
            );
        }
    }

    #[test]
    fn scan_inline_carries_the_style_of_the_content_it_describes() {
        let scan = scan_inline("**bold** and `code`");
        assert!(scan[2].style.bold, "the b of bold");
        assert!(!scan[0].style.bold, "not the delimiter");
        assert!(scan[14].style.code, "the c of code");

        let scan = scan_inline("a [[Target]] and a #tag");
        assert_eq!(scan[4].ink, Ink::WikiLink);
        assert_eq!(scan[20].ink, Ink::Tag);
    }

    #[test]
    fn scan_inline_treats_an_unclosed_delimiter_as_content() {
        // It renders as itself, so colouring it as syntax would be a lie about
        // what the line is going to look like.
        assert_eq!(shape("a ** b"), "......");
        assert_eq!(shape("[[unclosed"), "..........");
    }

    #[test]
    fn preview_flattens_and_truncates() {
        let body = "# Title\n\nSome body text that goes on.\n\n- item\n";
        assert_eq!(
            preview(body, 100),
            "Title Some body text that goes on. item"
        );
        let short = preview(body, 10);
        assert!(short.chars().count() <= 10, "got {short:?}");
        assert!(short.ends_with('…'));
    }

    #[test]
    fn horizontal_rules_are_not_frontmatter() {
        let doc = parse_body("text\n\n---\n\nmore\n", 0);
        assert!(kinds(&doc).iter().any(|k| matches!(k, BlockKind::Rule)));
    }
}
