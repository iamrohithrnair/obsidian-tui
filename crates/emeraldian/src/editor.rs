//! The note editor's text buffer.
//!
//! A line-vector buffer with a cursor, selection and undo. Notes are small
//! enough — a long one is a few thousand lines — that a rope would be
//! complexity without benefit, while a `Vec<String>` maps one-to-one onto how
//! the buffer is rendered and how Markdown is parsed.
//!
//! Positions are in **characters**, not bytes, so a cursor never lands inside a
//! multi-byte character.
//!
//! A long line is *shown* over several terminal rows, which is what [`Layout`]
//! works out. That mapping is deliberately kept out of the buffer: the buffer
//! stays the plain line-oriented model everything else parses and indexes, and
//! only the parts that draw or move the cursor need to know how it was wrapped.

use unicode_width::UnicodeWidthChar;

/// Display columns a character occupies.
///
/// A tab is one character but is drawn as spaces to the next stop, so measuring
/// it as one column would put the caret in the wrong place on any indented line.
#[must_use]
fn char_width(ch: char, tab_width: usize, column: usize) -> usize {
    if ch == '\t' {
        tab_width - (column % tab_width.max(1))
    } else {
        ch.width().unwrap_or(0)
    }
}

/// A position in the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    pub line: usize,
    /// Character offset within the line.
    pub col: usize,
}

/// One terminal row: the slice of a source line that is drawn on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Index of the source line this row came from.
    pub line: usize,
    /// First character of that line shown here.
    pub start: usize,
    /// One past the last character shown here.
    pub end: usize,
    /// Blank columns before the text, so a wrapped list item lines up under
    /// its own words rather than under its bullet.
    pub indent: u16,
    /// False on a soft-wrap continuation of the row above.
    pub first: bool,
}

/// How the buffer's lines were laid out across the rows of a viewport.
///
/// Rebuilt wherever it is needed rather than cached: it costs one pass over the
/// text, which is what the reading pane already spends parsing Markdown every
/// frame, and a cache here would only be a way to draw a stale note.
#[derive(Debug, Clone)]
pub struct Layout {
    rows: Vec<Row>,
    /// Index into `rows` of the first row of each source line.
    first: Vec<usize>,
    /// Columns the text was wrapped to.
    width: usize,
    wrapped: bool,
}

/// Narrowest column a wrapped line is ever squeezed into.
///
/// A deeply indented list item in a slim pane would otherwise be wrapped to
/// nothing and loop forever; below this the hanging indent is given up instead.
const MIN_WRAP: usize = 12;

impl Layout {
    /// Lays `lines` out for a viewport `width` columns wide.
    ///
    /// With `wrap` off every line gets exactly one row, however long it is, and
    /// the viewport pans sideways instead — which is what someone who turned
    /// wrapping off asked for.
    #[must_use]
    pub fn build(lines: &[String], width: usize, wrap: bool, tab_width: usize) -> Self {
        let mut rows = Vec::with_capacity(lines.len());
        let mut first = Vec::with_capacity(lines.len());

        for (index, text) in lines.iter().enumerate() {
            first.push(rows.len());
            let chars: Vec<char> = text.chars().collect();
            if !wrap || width == 0 {
                rows.push(Row {
                    line: index,
                    start: 0,
                    end: chars.len(),
                    indent: 0,
                    first: true,
                });
                continue;
            }
            wrap_line(&chars, index, width, tab_width, &mut rows);
        }

        // An empty buffer still has one line, so there is always a row to put
        // the cursor on.
        if rows.is_empty() {
            first.push(0);
            rows.push(Row {
                line: 0,
                start: 0,
                end: 0,
                indent: 0,
                first: true,
            });
        }

        Self {
            rows,
            first,
            width,
            wrapped: wrap,
        }
    }

    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn wrapped(&self) -> bool {
        self.wrapped
    }

    /// The row a cursor sits on.
    ///
    /// At a wrap boundary the cursor belongs to the start of the following row,
    /// not the end of the one before, so typing carries on where the text does.
    #[must_use]
    pub fn row_of(&self, cursor: Cursor) -> usize {
        let last = self.rows.len() - 1;
        let Some(&start) = self.first.get(cursor.line) else {
            return last;
        };
        let mut index = start.min(last);
        while index < last
            && self.rows[index + 1].line == cursor.line
            && self.rows[index + 1].start <= cursor.col
        {
            index += 1;
        }
        index
    }

    /// The row and display column a cursor is drawn at.
    #[must_use]
    pub fn position_of(&self, cursor: Cursor, lines: &[String], tab_width: usize) -> (usize, u16) {
        let index = self.row_of(cursor);
        let row = self.rows[index];
        let mut column = usize::from(row.indent);
        if let Some(text) = lines.get(row.line) {
            for ch in text.chars().take(cursor.col).skip(row.start) {
                column += char_width(ch, tab_width, column);
            }
        }
        (index, u16::try_from(column).unwrap_or(u16::MAX))
    }

    /// The cursor at a row and display column — how a click and a vertical move
    /// both land somewhere sensible.
    #[must_use]
    pub fn cursor_at(&self, row: usize, column: u16, lines: &[String], tab_width: usize) -> Cursor {
        let row = self.rows[row.min(self.rows.len() - 1)];
        let target = usize::from(column).saturating_sub(usize::from(row.indent));
        let mut col = row.start;
        let mut at = usize::from(row.indent);

        if let Some(text) = lines.get(row.line) {
            for ch in text.chars().take(row.end).skip(row.start) {
                let width = char_width(ch, tab_width, at);
                // Land on whichever character the column falls closest to, so
                // clicking the right half of a wide glyph lands after it.
                if at + width > usize::from(row.indent) + target {
                    break;
                }
                at += width;
                col += 1;
            }
        }
        Cursor {
            line: row.line,
            col,
        }
    }
}

/// Breaks one line into rows at word boundaries.
fn wrap_line(chars: &[char], line: usize, width: usize, tab_width: usize, out: &mut Vec<Row>) {
    if chars.is_empty() {
        out.push(Row {
            line,
            start: 0,
            end: 0,
            indent: 0,
            first: true,
        });
        return;
    }

    let hanging = hanging_indent(chars, tab_width, width);
    let mut start = 0;
    let mut first = true;

    while start < chars.len() {
        let indent = if first { 0 } else { hanging };
        let available = width.saturating_sub(usize::from(indent)).max(1);

        let mut column = 0;
        let mut at = start;
        // One past the last space that fits, which is where the line would
        // rather break.
        let mut boundary = None;
        while at < chars.len() {
            let next = column + char_width(chars[at], tab_width, column);
            if next > available && at > start {
                break;
            }
            column = next;
            at += 1;
            if chars[at - 1] == ' ' {
                boundary = Some(at);
            }
        }

        // Breaking after the space keeps it on this row, where it is invisible,
        // instead of indenting the next one by a stray blank.
        let end = match boundary {
            Some(boundary) if at < chars.len() && boundary > start => boundary,
            _ => at,
        };

        out.push(Row {
            line,
            start,
            end,
            indent,
            first,
        });
        start = end;
        first = false;
    }
}

/// Columns a wrapped line's continuations are indented by.
///
/// A wrapped bullet reads as one item when its second row starts under the
/// first row's text, and as two items when it starts under the bullet.
fn hanging_indent(chars: &[char], tab_width: usize, width: usize) -> u16 {
    let mut column = 0;
    let mut at = 0;
    while at < chars.len() && (chars[at] == ' ' || chars[at] == '\t') {
        column += char_width(chars[at], tab_width, column);
        at += 1;
    }
    // The marker itself. Every marker is ASCII, so its length in bytes, in
    // characters and in columns are all the same number.
    let rest: String = chars[at..].iter().collect();
    column += marker_len(&rest).unwrap_or(0);

    if width.saturating_sub(column) < MIN_WRAP {
        return 0;
    }
    u16::try_from(column).unwrap_or(0)
}

/// What kind of edit was last applied, used to group undo steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    Structural,
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Cursor,
}

pub struct Editor {
    lines: Vec<String>,
    cursor: Cursor,
    /// Display column the cursor wants when moving between rows, so travelling
    /// through a short row and back out preserves the original column.
    ///
    /// Cleared by every other kind of movement, and by editing, so it only ever
    /// holds a column the user actually aimed at.
    desired_col: Option<u16>,
    selection_anchor: Option<Cursor>,
    /// Rows scrolled off the top — terminal rows, not source lines, since one
    /// long line can occupy several of them.
    pub scroll: usize,
    /// Columns scrolled off the left. Only ever non-zero with wrapping off,
    /// where reaching the end of a long line is the whole point.
    pub hscroll: usize,
    modified: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
    tab_width: usize,
    expand_tabs: bool,
}

/// Undo history depth. Deep enough to recover from a bad paste, bounded so a
/// long session can't grow without limit.
const MAX_UNDO: usize = 500;

impl Editor {
    #[must_use]
    pub fn new(text: &str, tab_width: usize, expand_tabs: bool) -> Self {
        Self {
            lines: split_lines(text),
            cursor: Cursor::default(),
            desired_col: None,
            selection_anchor: None,
            scroll: 0,
            hscroll: 0,
            modified: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            tab_width,
            expand_tabs,
        }
    }

    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// How this buffer falls across the rows of a viewport `width` columns wide.
    #[must_use]
    pub fn layout(&self, width: usize, wrap: bool) -> Layout {
        Layout::build(&self.lines, width, wrap, self.tab_width)
    }

    /// The row and display column the caret should be drawn at.
    #[must_use]
    pub fn caret(&self, layout: &Layout) -> (usize, u16) {
        layout.position_of(self.cursor, &self.lines, self.tab_width)
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    /// The buffer as text, always newline-terminated.
    ///
    /// Notes are text files and POSIX text files end with a newline; making it
    /// unconditional keeps diffs clean when a note is edited both here and in
    /// Obsidian.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }

    /// The current selection as an ordered pair, if any.
    #[must_use]
    pub fn selection(&self) -> Option<(Cursor, Cursor)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some(if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        })
    }

    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        Some(self.slice(start, end))
    }

    pub fn begin_selection(&mut self) {
        self.selection_anchor = Some(self.cursor);
    }

    pub fn select_all(&mut self) {
        self.selection_anchor = Some(Cursor::default());
        self.cursor = self.end_position();
    }

    // ---- movement --------------------------------------------------------

    pub fn move_left(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.col < self.line_len(self.cursor.line) {
            self.cursor.col += 1;
        } else if self.cursor.line + 1 < self.lines.len() {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
    }

    /// Moves `delta` terminal rows, following the text as it was wrapped.
    ///
    /// Moving by wrapped row rather than by source line is what makes the arrow
    /// keys agree with the screen: on a paragraph that fills four rows, four
    /// presses of `Down` cross it, as they would anywhere else.
    pub fn move_row(&mut self, layout: &Layout, delta: isize, extend: bool) {
        // Read the aimed-for column before `prepare_move` forgets it.
        let (row, column) = layout.position_of(self.cursor, &self.lines, self.tab_width);
        let want = self.desired_col.unwrap_or(column);
        self.prepare_move(extend);

        let last = layout.rows().len().saturating_sub(1) as isize;
        let target = (row as isize + delta).clamp(0, last) as usize;
        self.cursor = layout.cursor_at(target, want, &self.lines, self.tab_width);
        self.desired_col = Some(want);
    }

    /// `Home`: the start of the row on screen.
    ///
    /// On the first row of a line this keeps the behavior every editor settled
    /// on for indented text — the first non-blank, then column zero — because
    /// that is where the interesting positions are. On a wrapped continuation
    /// there is only one sensible answer: where the row begins.
    pub fn move_row_start(&mut self, layout: &Layout, extend: bool) {
        let row = layout.rows()[layout.row_of(self.cursor)];
        if !row.first {
            self.prepare_move(extend);
            self.cursor.col = row.start;
            return;
        }
        self.move_line_start(extend);
    }

    /// `End`: the end of the row on screen, which on the last row of a line is
    /// the end of the line.
    pub fn move_row_end(&mut self, layout: &Layout, extend: bool) {
        let row = layout.rows()[layout.row_of(self.cursor)];
        if row.end == self.line_len(row.line) {
            self.move_line_end(extend);
            return;
        }
        self.prepare_move(extend);
        self.cursor.col = row.end;
    }

    /// Places the cursor at a row and display column, for a click or a drag.
    pub fn goto_visual(&mut self, layout: &Layout, row: usize, column: u16, extend: bool) {
        self.prepare_move(extend);
        self.cursor = layout.cursor_at(row, column, &self.lines, self.tab_width);
    }

    pub fn move_line_start(&mut self, extend: bool) {
        self.prepare_move(extend);
        let indent = self.lines[self.cursor.line]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        self.cursor.col = if self.cursor.col == indent { 0 } else { indent };
    }

    pub fn move_line_end(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor.col = self.line_len(self.cursor.line);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.col == 0 {
            if self.cursor.line > 0 {
                self.cursor.line -= 1;
                self.cursor.col = self.line_len(self.cursor.line);
            }
        } else {
            let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
            let mut col = self.cursor.col;
            while col > 0 && !chars[col - 1].is_alphanumeric() {
                col -= 1;
            }
            while col > 0 && chars[col - 1].is_alphanumeric() {
                col -= 1;
            }
            self.cursor.col = col;
        }
    }

    pub fn move_word_right(&mut self, extend: bool) {
        self.prepare_move(extend);
        let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
        if self.cursor.col >= chars.len() {
            if self.cursor.line + 1 < self.lines.len() {
                self.cursor.line += 1;
                self.cursor.col = 0;
            }
        } else {
            let mut col = self.cursor.col;
            while col < chars.len() && chars[col].is_alphanumeric() {
                col += 1;
            }
            while col < chars.len() && !chars[col].is_alphanumeric() {
                col += 1;
            }
            self.cursor.col = col;
        }
    }

    pub fn move_document_start(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor = Cursor::default();
    }

    pub fn move_document_end(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor = self.end_position();
    }

    /// Places the cursor at a specific position, clamped into the buffer.
    pub fn goto(&mut self, line: usize, col: usize) {
        self.cursor.line = line.min(self.lines.len().saturating_sub(1));
        self.cursor.col = col.min(self.line_len(self.cursor.line));
        self.desired_col = None;
        self.selection_anchor = None;
    }

    fn prepare_move(&mut self, extend: bool) {
        // Any move that isn't between rows abandons the column the cursor was
        // aiming for; only `move_row` puts one back.
        self.desired_col = None;
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
    }

    // ---- editing ---------------------------------------------------------

    pub fn insert_char(&mut self, ch: char) {
        self.push_undo(EditKind::Insert);
        self.delete_selection_inner();

        if ch == '\t' && self.expand_tabs {
            let spaces = self.tab_width - (self.cursor.col % self.tab_width);
            let text = " ".repeat(spaces);
            self.insert_into_line(&text);
            return;
        }
        self.insert_into_line(&ch.to_string());
    }

    pub fn insert_str(&mut self, text: &str) {
        self.push_undo(EditKind::Insert);
        self.delete_selection_inner();
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.split_line();
            }
            if !part.is_empty() {
                self.insert_into_line(part.trim_end_matches('\r'));
            }
        }
    }

    /// `Enter`. Carries indentation, and continues a list.
    ///
    /// Continuing the list is what makes a terminal usable for notes at all:
    /// typing a checklist otherwise means retyping `- [ ] ` on every line.
    /// Pressing it on an item with nothing in it ends the list instead, which is
    /// how you stop — the same rule Obsidian uses.
    pub fn newline(&mut self) {
        self.push_undo(EditKind::Structural);
        self.delete_selection_inner();

        let line = self.lines[self.cursor.line].clone();
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let marker = marker(&line[indent.len()..]);

        // An item with only a marker on it: clear the line rather than adding
        // another empty one below.
        if let Some(marker) = &marker
            && line[indent.len() + marker.len..].trim().is_empty()
        {
            self.lines[self.cursor.line] = String::new();
            self.cursor.col = 0;
            self.desired_col = None;
            self.modified = true;
            return;
        }

        self.split_line();
        let carry = match &marker {
            Some(marker) => format!("{indent}{}", marker.next),
            None => indent,
        };
        if !carry.is_empty() {
            self.insert_into_line(&carry);
        }
    }

    /// `Tab` and `Shift+Tab`.
    ///
    /// Inside a list item, or with several lines selected, `Tab` nests rather
    /// than inserting a tab character — in a list that is the only thing it
    /// could reasonably mean. Anywhere else it is still a tab.
    pub fn tab(&mut self, forward: bool) {
        if !forward {
            self.indent(false);
            return;
        }
        let line = &self.lines[self.cursor.line];
        let in_list = marker(line.trim_start()).is_some();
        if in_list || self.selection().is_some() {
            self.indent(true);
        } else {
            self.insert_char('\t');
        }
    }

    /// Indents or outdents every line the cursor or the selection touches.
    pub fn indent(&mut self, forward: bool) {
        self.push_undo(EditKind::Structural);
        let (start, end) = self
            .selection()
            .map_or((self.cursor, self.cursor), |(s, e)| (s, e));

        let step = if self.expand_tabs {
            " ".repeat(self.tab_width)
        } else {
            "\t".to_string()
        };
        let mut moved = 0;
        for line in start.line..=end.line.min(self.lines.len() - 1) {
            if forward {
                self.lines[line].insert_str(0, &step);
                moved = step.chars().count();
                continue;
            }
            // Outdenting takes back a whole stop, or whatever less is there.
            let removable = self.lines[line]
                .chars()
                .take(step.chars().count())
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            self.lines[line] = self.lines[line].chars().skip(removable).collect();
            if line == self.cursor.line {
                moved = removable;
            }
        }

        self.cursor.col = if forward {
            self.cursor.col + moved
        } else {
            self.cursor.col.saturating_sub(moved)
        };
        self.desired_col = None;
        self.selection_anchor = None;
        self.modified = true;
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.push_undo(EditKind::Delete);
            self.delete_selection_inner();
            return;
        }
        if self.cursor.line == 0 && self.cursor.col == 0 {
            return;
        }
        self.push_undo(EditKind::Delete);

        if self.cursor.col > 0 {
            let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
            // Deleting through soft-tab indentation removes the whole stop.
            let mut remove = 1;
            if self.expand_tabs
                && chars[..self.cursor.col].iter().all(|c| *c == ' ')
                && self.cursor.col.is_multiple_of(self.tab_width)
            {
                remove = self.tab_width.min(self.cursor.col);
            }
            let start = self.cursor.col - remove;
            let kept: String = chars[..start]
                .iter()
                .chain(chars[self.cursor.col..].iter())
                .collect();
            self.lines[self.cursor.line] = kept;
            self.cursor.col = start;
        } else {
            let current = self.lines.remove(self.cursor.line);
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
            self.lines[self.cursor.line].push_str(&current);
        }
        self.desired_col = None;
        self.modified = true;
    }

    pub fn delete_forward(&mut self) {
        if self.selection().is_some() {
            self.push_undo(EditKind::Delete);
            self.delete_selection_inner();
            return;
        }
        let len = self.line_len(self.cursor.line);
        if self.cursor.col == len && self.cursor.line + 1 >= self.lines.len() {
            return;
        }
        self.push_undo(EditKind::Delete);

        if self.cursor.col < len {
            let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
            let kept: String = chars[..self.cursor.col]
                .iter()
                .chain(chars[self.cursor.col + 1..].iter())
                .collect();
            self.lines[self.cursor.line] = kept;
        } else {
            let next = self.lines.remove(self.cursor.line + 1);
            self.lines[self.cursor.line].push_str(&next);
        }
        self.modified = true;
    }

    /// Deletes the current line, or every line the selection touches.
    pub fn delete_line(&mut self) {
        self.push_undo(EditKind::Structural);
        let (start, end) = self
            .selection()
            .map_or((self.cursor, self.cursor), |(s, e)| (s, e));

        let first = start.line;
        let last = end.line.min(self.lines.len() - 1);
        self.lines.drain(first..=last);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor.line = first.min(self.lines.len() - 1);
        self.cursor.col = 0;
        self.selection_anchor = None;
        self.modified = true;
    }

    pub fn delete_selection(&mut self) {
        if self.selection().is_some() {
            self.push_undo(EditKind::Delete);
            self.delete_selection_inner();
        }
    }

    /// Wraps the selection (or the word at the cursor) in a marker, or removes
    /// it when already present — how `Ctrl+B` behaves in Obsidian.
    pub fn toggle_wrap(&mut self, marker: &str) {
        self.push_undo(EditKind::Structural);
        let Some((start, end)) = self.selection() else {
            // With no selection, insert the pair and place the cursor inside.
            self.insert_into_line(&format!("{marker}{marker}"));
            self.cursor.col -= marker.chars().count();
            self.modified = true;
            return;
        };

        let text = self.slice(start, end);
        let unwrapped = text
            .strip_prefix(marker)
            .and_then(|t| t.strip_suffix(marker));

        let replacement = match unwrapped {
            Some(inner) => inner.to_string(),
            None => format!("{marker}{text}{marker}"),
        };

        self.delete_selection_inner();
        for (i, part) in replacement.split('\n').enumerate() {
            if i > 0 {
                self.split_line();
            }
            self.insert_into_line(part);
        }
        self.modified = true;
    }

    fn delete_selection_inner(&mut self) {
        let Some((start, end)) = self.selection() else {
            return;
        };

        let start_chars: Vec<char> = self.lines[start.line].chars().collect();
        let end_chars: Vec<char> = self.lines[end.line].chars().collect();
        let head: String = start_chars[..start.col.min(start_chars.len())]
            .iter()
            .collect();
        let tail: String = end_chars[end.col.min(end_chars.len())..].iter().collect();

        self.lines.drain(start.line..=end.line);
        self.lines.insert(start.line, format!("{head}{tail}"));

        self.cursor = start;
        self.desired_col = None;
        self.selection_anchor = None;
        self.modified = true;
    }

    fn insert_into_line(&mut self, text: &str) {
        let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
        let col = self.cursor.col.min(chars.len());
        let mut line: String = chars[..col].iter().collect();
        line.push_str(text);
        line.extend(chars[col..].iter());
        self.lines[self.cursor.line] = line;
        self.cursor.col = col + text.chars().count();
        self.desired_col = None;
        self.modified = true;
    }

    fn split_line(&mut self) {
        let chars: Vec<char> = self.lines[self.cursor.line].chars().collect();
        let col = self.cursor.col.min(chars.len());
        let head: String = chars[..col].iter().collect();
        let tail: String = chars[col..].iter().collect();
        self.lines[self.cursor.line] = head;
        self.lines.insert(self.cursor.line + 1, tail);
        self.cursor.line += 1;
        self.cursor.col = 0;
        self.desired_col = None;
        self.modified = true;
    }

    // ---- undo ------------------------------------------------------------

    /// Records a snapshot, coalescing runs of the same kind of edit.
    ///
    /// Undo should step back by a *thought*, not a keystroke, so consecutive
    /// typing collapses into one entry while a delete after typing starts a new
    /// one.
    fn push_undo(&mut self, kind: EditKind) {
        let coalesce = self.last_edit == Some(kind) && kind != EditKind::Structural;
        self.last_edit = Some(kind);
        self.redo.clear();
        if coalesce {
            return;
        }
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    /// Ends the current undo group, so the next edit starts a new one.
    pub fn commit(&mut self) {
        self.last_edit = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = snapshot.lines;
        self.cursor = snapshot.cursor;
        self.selection_anchor = None;
        self.last_edit = None;
        self.modified = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = snapshot.lines;
        self.cursor = snapshot.cursor;
        self.selection_anchor = None;
        self.last_edit = None;
        self.modified = true;
        true
    }

    // ---- helpers ---------------------------------------------------------

    fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |l| l.chars().count())
    }

    fn end_position(&self) -> Cursor {
        let line = self.lines.len().saturating_sub(1);
        Cursor {
            line,
            col: self.line_len(line),
        }
    }

    fn slice(&self, start: Cursor, end: Cursor) -> String {
        if start.line == end.line {
            return self.lines[start.line]
                .chars()
                .skip(start.col)
                .take(end.col.saturating_sub(start.col))
                .collect();
        }
        let mut out: String = self.lines[start.line].chars().skip(start.col).collect();
        for line in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(line);
        }
        out.push('\n');
        out.extend(self.lines[end.line].chars().take(end.col));
        out
    }

    /// Scrolls so the cursor is visible in a viewport `height` rows tall.
    ///
    /// Rows, not lines: with wrapping on, a paragraph the cursor is halfway
    /// through may be taller than the viewport on its own.
    pub fn scroll_into_view(&mut self, layout: &Layout, height: usize) {
        if height == 0 {
            return;
        }
        let (row, column) = self.caret(layout);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        // Never leave blank rows below a note that would fit further up.
        self.scroll = self.scroll.min(layout.rows().len().saturating_sub(height));

        // Panning sideways only happens with wrapping off; a wrapped row always
        // fits the viewport it was wrapped to.
        if layout.wrapped() || layout.width() == 0 {
            self.hscroll = 0;
            return;
        }
        let column = usize::from(column);
        if column < self.hscroll {
            self.hscroll = column;
        } else if column >= self.hscroll + layout.width() {
            self.hscroll = column + 1 - layout.width();
        }
    }
}

/// A list marker at the start of a line, and what continues it below.
struct Continuation {
    /// Bytes of the marker, including its trailing space.
    len: usize,
    /// What the next line should start with after the same indentation.
    next: String,
}

/// Reads the list marker at the start of `rest`, which must already have its
/// indentation stripped.
///
/// One place knows what a marker looks like, and three things ask it: how far
/// to indent a wrapped row, what `Enter` should carry down, and whether `Tab`
/// means "nest this item" or "insert a tab".
fn marker(rest: &str) -> Option<Continuation> {
    let bytes = rest.as_bytes();
    let bullet = matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ');

    // A task box, checked or not, always continues as an empty one: carrying
    // "done" down to a line nobody has done yet would be a lie.
    if bullet
        && bytes.get(2) == Some(&b'[')
        && matches!(bytes.get(3), Some(b' ' | b'x' | b'X'))
        && bytes.get(4) == Some(&b']')
        && bytes.get(5) == Some(&b' ')
    {
        return Some(Continuation {
            len: 6,
            next: format!("{} [ ] ", &rest[..1]),
        });
    }
    if bullet {
        return Some(Continuation {
            len: 2,
            next: rest[..2].to_string(),
        });
    }

    // `1. ` or `1) `, continuing with the next number.
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && matches!(bytes.get(digits), Some(b'.' | b')'))
        && bytes.get(digits + 1) == Some(&b' ')
    {
        let number: u64 = rest[..digits].parse().unwrap_or(0);
        return Some(Continuation {
            len: digits + 2,
            next: format!("{}{} ", number + 1, &rest[digits..=digits]),
        });
    }

    // Every level of a `> > ` quote, repeated verbatim.
    let quote = rest
        .bytes()
        .take_while(|byte| matches!(byte, b'>' | b' '))
        .count();
    if quote > 0 && rest.starts_with('>') {
        return Some(Continuation {
            len: quote,
            next: rest[..quote].to_string(),
        });
    }

    None
}

/// Bytes of the list marker at the start of `rest`, if any.
fn marker_len(rest: &str) -> Option<usize> {
    marker(rest).map(|marker| marker.len)
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // A trailing newline produces an empty final element that isn't a real line.
    if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
impl Editor {
    /// Test helper: move the cursor while extending the selection.
    fn goto_extend(&mut self, line: usize, col: usize) {
        let anchor = self.selection_anchor;
        self.goto(line, col);
        self.selection_anchor = anchor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> Editor {
        Editor::new(text, 4, true)
    }

    #[test]
    fn round_trips_text_with_a_trailing_newline() {
        assert_eq!(editor("a\nb\n").text(), "a\nb\n");
        assert_eq!(editor("a\nb").text(), "a\nb\n", "a newline is added");
        assert_eq!(editor("").text(), "\n");
    }

    #[test]
    fn typing_inserts_and_marks_modified() {
        let mut ed = editor("");
        assert!(!ed.is_modified());
        for ch in "hello".chars() {
            ed.insert_char(ch);
        }
        assert_eq!(ed.text(), "hello\n");
        assert!(ed.is_modified());
        assert_eq!(ed.cursor(), Cursor { line: 0, col: 5 });
    }

    #[test]
    fn newline_carries_indentation() {
        let mut ed = editor("    indented");
        ed.move_line_end(false);
        ed.newline();
        ed.insert_char('x');
        assert_eq!(ed.text(), "    indented\n    x\n");
    }

    #[test]
    fn tab_expands_to_the_next_tab_stop() {
        let mut ed = editor("ab");
        ed.move_line_end(false);
        ed.insert_char('\t');
        assert_eq!(ed.text(), "ab  \n", "2 spaces reach column 4");
    }

    #[test]
    fn backspace_removes_a_whole_soft_tab() {
        let mut ed = editor("    x");
        ed.goto(0, 4);
        ed.backspace();
        assert_eq!(ed.text(), "x\n");
    }

    #[test]
    fn backspace_at_line_start_joins_lines() {
        let mut ed = editor("ab\ncd");
        ed.goto(1, 0);
        ed.backspace();
        assert_eq!(ed.text(), "abcd\n");
        assert_eq!(ed.cursor(), Cursor { line: 0, col: 2 });
    }

    #[test]
    fn delete_forward_joins_the_next_line() {
        let mut ed = editor("ab\ncd");
        ed.move_line_end(false);
        ed.delete_forward();
        assert_eq!(ed.text(), "abcd\n");
    }

    /// A layout wide enough that nothing wraps, so a row is a line.
    fn unwrapped(ed: &Editor) -> Layout {
        ed.layout(200, true)
    }

    #[test]
    fn vertical_movement_remembers_the_desired_column() {
        let mut ed = editor("longer line\nx\nanother long line");
        let layout = unwrapped(&ed);
        ed.goto(0, 9);
        ed.move_row(&layout, 1, false);
        assert_eq!(ed.cursor().col, 1, "clamped to the short line");
        ed.move_row(&layout, 1, false);
        assert_eq!(ed.cursor().col, 9, "restored on the longer line");
    }

    #[test]
    fn typing_forgets_the_column_a_vertical_move_was_aiming_for() {
        // Otherwise the cursor jumps back to a column the user left behind.
        let mut ed = editor("longer line\nx\nanother long line");
        let layout = unwrapped(&ed);
        ed.goto(0, 9);
        ed.move_row(&layout, 1, false);
        ed.insert_char('!');

        let layout = unwrapped(&ed);
        ed.move_row(&layout, 1, false);
        assert_eq!(
            ed.cursor().col,
            2,
            "the column comes from where typing left it"
        );
    }

    #[test]
    fn home_toggles_between_indent_and_column_zero() {
        let mut ed = editor("    text");
        ed.move_line_end(false);
        ed.move_line_start(false);
        assert_eq!(ed.cursor().col, 4, "first stop is the indent");
        ed.move_line_start(false);
        assert_eq!(ed.cursor().col, 0);
    }

    #[test]
    fn word_movement_crosses_words_and_lines() {
        let mut ed = editor("alpha beta\ngamma");
        ed.move_word_right(false);
        assert_eq!(ed.cursor().col, 6, "lands at the start of the next word");
        ed.move_word_right(false);
        assert_eq!(ed.cursor().col, 10);
        ed.move_word_right(false);
        assert_eq!(ed.cursor(), Cursor { line: 1, col: 0 });
    }

    #[test]
    fn selection_spans_lines_and_deletes_cleanly() {
        let mut ed = editor("one\ntwo\nthree");
        ed.goto(0, 1);
        ed.begin_selection();
        ed.goto_extend(2, 2);
        assert_eq!(ed.selected_text().as_deref(), Some("ne\ntwo\nth"));

        ed.delete_selection();
        assert_eq!(ed.text(), "oree\n");
        assert_eq!(ed.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut ed = editor("hello world");
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(0, 5);
        ed.insert_char('X');
        assert_eq!(ed.text(), "X world\n");
    }

    #[test]
    fn toggle_wrap_adds_and_removes_markers() {
        let mut ed = editor("bold me");
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(0, 4);
        ed.toggle_wrap("**");
        assert_eq!(ed.text(), "**bold** me\n");

        // Re-selecting the wrapped text removes the markers again.
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(0, 8);
        ed.toggle_wrap("**");
        assert_eq!(ed.text(), "bold me\n");
    }

    #[test]
    fn delete_line_removes_the_whole_line() {
        let mut ed = editor("one\ntwo\nthree");
        ed.goto(1, 1);
        ed.delete_line();
        assert_eq!(ed.text(), "one\nthree\n");
        assert_eq!(ed.cursor().line, 1);
    }

    #[test]
    fn deleting_the_last_line_leaves_an_empty_buffer() {
        let mut ed = editor("only");
        ed.delete_line();
        assert_eq!(ed.text(), "\n");
        assert_eq!(ed.line_count(), 1);
    }

    #[test]
    fn undo_groups_a_run_of_typing() {
        let mut ed = editor("");
        for ch in "hello".chars() {
            ed.insert_char(ch);
        }
        assert!(ed.undo());
        assert_eq!(ed.text(), "\n", "one undo removes the whole run");
        assert!(ed.redo());
        assert_eq!(ed.text(), "hello\n");
    }

    #[test]
    fn undo_separates_typing_from_deleting() {
        let mut ed = editor("");
        ed.insert_str("word");
        ed.backspace();
        assert_eq!(ed.text(), "wor\n");

        ed.undo();
        assert_eq!(ed.text(), "word\n", "the delete is its own step");
        ed.undo();
        assert_eq!(ed.text(), "\n");
    }

    #[test]
    fn commit_forces_a_new_undo_group() {
        let mut ed = editor("");
        ed.insert_char('a');
        ed.commit();
        ed.insert_char('b');
        ed.undo();
        assert_eq!(ed.text(), "a\n");
    }

    #[test]
    fn undo_on_an_empty_history_is_a_no_op() {
        let mut ed = editor("text");
        assert!(!ed.undo());
        assert!(!ed.redo());
        assert_eq!(ed.text(), "text\n");
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut ed = editor("");
        ed.insert_str("one");
        ed.undo();
        ed.insert_str("two");
        assert!(!ed.redo(), "redo is invalidated by a divergent edit");
        assert_eq!(ed.text(), "two\n");
    }

    #[test]
    fn multi_byte_characters_are_handled_by_character_not_byte() {
        let mut ed = editor("héllo → wörld");
        ed.move_line_end(false);
        assert_eq!(ed.cursor().col, 13);

        ed.goto(0, 6);
        ed.insert_char('!');
        assert_eq!(ed.text(), "héllo !→ wörld\n");

        ed.goto(0, 1);
        ed.delete_forward();
        assert_eq!(ed.text(), "hllo !→ wörld\n");
    }

    #[test]
    fn insert_str_handles_multi_line_paste() {
        let mut ed = editor("start");
        ed.move_line_end(false);
        ed.insert_str("\nmiddle\nend");
        assert_eq!(ed.text(), "start\nmiddle\nend\n");
    }

    #[test]
    fn scroll_follows_the_cursor_both_ways() {
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let mut ed = editor(&text);
        let layout = unwrapped(&ed);

        ed.goto(50, 0);
        ed.scroll_into_view(&layout, 10);
        assert_eq!(ed.scroll, 41, "cursor sits on the last visible row");

        ed.goto(5, 0);
        ed.scroll_into_view(&layout, 10);
        assert_eq!(ed.scroll, 5);
    }

    #[test]
    fn scroll_counts_wrapped_rows_not_lines() {
        // Three lines that each take four rows: the cursor on the last one is 12
        // rows down, not 3, and scrolling by lines would leave it off screen.
        let mut ed = editor("aaa bbb ccc ddd\neee fff ggg hhh\niii jjj kkk lll");
        let layout = ed.layout(4, true);
        assert_eq!(layout.rows().len(), 12);

        ed.goto(2, 15);
        ed.scroll_into_view(&layout, 5);
        let (row, _) = ed.caret(&layout);
        assert!(
            row >= ed.scroll && row < ed.scroll + 5,
            "row {row} is off a viewport starting at {}",
            ed.scroll
        );
    }

    #[test]
    fn a_short_note_never_scrolls_past_its_own_end() {
        let mut ed = editor("one\ntwo\nthree");
        let layout = unwrapped(&ed);
        ed.scroll = 99;
        ed.goto(0, 0);
        ed.scroll_into_view(&layout, 10);
        assert_eq!(ed.scroll, 0, "no blank rows above a note that fits");
    }

    #[test]
    fn wrapping_off_pans_sideways_to_reach_the_end_of_a_long_line() {
        let mut ed = editor(&"x".repeat(200));
        let layout = ed.layout(40, false);
        assert_eq!(layout.rows().len(), 1, "one row, however long the line");

        ed.move_line_end(false);
        ed.scroll_into_view(&layout, 10);
        assert_eq!(ed.hscroll, 161, "the caret is at the right edge");
    }

    #[test]
    fn a_long_line_wraps_at_word_boundaries() {
        let ed = editor("the quick brown fox jumps");
        let layout = ed.layout(10, true);
        let rows: Vec<&str> = layout
            .rows()
            .iter()
            .map(|row| &ed.lines()[row.line][row.start..row.end])
            .collect();

        assert_eq!(rows, vec!["the quick ", "brown fox ", "jumps"]);
        for row in layout.rows() {
            assert!(row.end - row.start <= 10, "no row is wider than the pane");
        }
    }

    #[test]
    fn a_word_longer_than_the_pane_is_broken_rather_than_lost() {
        let ed = editor("supercalifragilistic");
        let layout = ed.layout(8, true);
        assert!(layout.rows().len() > 1);
        assert_eq!(
            layout
                .rows()
                .iter()
                .map(|row| row.end - row.start)
                .sum::<usize>(),
            20,
            "every character is on some row"
        );
    }

    #[test]
    fn a_wrapped_list_item_lines_up_under_its_own_text() {
        let ed = editor("- alpha beta gamma delta epsilon");
        let layout = ed.layout(20, true);
        let rows = layout.rows();

        assert!(rows.len() > 1, "should wrap");
        assert_eq!(rows[0].indent, 0);
        assert_eq!(rows[1].indent, 2, "indented past the bullet, not under it");
    }

    #[test]
    fn the_caret_and_a_click_agree_on_where_a_character_is() {
        // Wide characters occupy two columns, so a column is not a character.
        let mut ed = editor("héllo 日本語 world text here");
        let layout = ed.layout(12, true);

        for col in 0..ed.lines()[0].chars().count() {
            ed.goto(0, col);
            let (row, column) = ed.caret(&layout);
            let back = layout.cursor_at(row, column, ed.lines(), 4);
            assert_eq!(
                back,
                Cursor { line: 0, col },
                "column {column} on row {row} should map back to character {col}"
            );
        }
    }

    #[test]
    fn moving_down_a_wrapped_paragraph_walks_it_row_by_row() {
        let mut ed = editor("aaa bbb ccc ddd eee fff");
        let layout = ed.layout(8, true);
        ed.goto(0, 0);

        let mut rows = vec![ed.caret(&layout).0];
        for _ in 0..2 {
            ed.move_row(&layout, 1, false);
            rows.push(ed.caret(&layout).0);
        }
        assert_eq!(rows, vec![0, 1, 2], "one press, one row");
    }

    #[test]
    fn home_and_end_work_on_the_row_not_the_line() {
        let mut ed = editor("aaa bbb ccc ddd");
        let layout = ed.layout(8, true);
        // Second row: "ccc ddd".
        ed.goto(0, 10);

        ed.move_row_end(&layout, false);
        assert_eq!(ed.cursor().col, 15);
        ed.move_row_start(&layout, false);
        assert_eq!(ed.cursor().col, 8, "the start of what is on screen");
    }

    #[test]
    fn enter_continues_a_list_and_an_empty_item_ends_it() {
        let mut ed = editor("- first");
        ed.move_line_end(false);
        ed.newline();
        assert_eq!(ed.text(), "- first\n- \n");

        ed.insert_char('x');
        ed.newline();
        assert_eq!(ed.text(), "- first\n- x\n- \n");

        // Nothing typed on the new item: Enter ends the list.
        ed.newline();
        assert_eq!(ed.text(), "- first\n- x\n\n");
    }

    #[test]
    fn enter_numbers_the_next_item_and_leaves_a_task_unchecked() {
        let mut ed = editor("3. third");
        ed.move_line_end(false);
        ed.newline();
        assert_eq!(ed.text(), "3. third\n4. \n");

        let mut ed = editor("  - [x] done");
        ed.move_line_end(false);
        ed.newline();
        assert_eq!(
            ed.text(),
            "  - [x] done\n  - [ ] \n",
            "indentation carries and the box starts empty"
        );
    }

    #[test]
    fn enter_carries_a_quote_and_splits_text_mid_item() {
        let mut ed = editor("> quoted");
        ed.move_line_end(false);
        ed.newline();
        assert_eq!(ed.text(), "> quoted\n> \n");

        let mut ed = editor("- alphabeta");
        ed.goto(0, 7);
        ed.newline();
        assert_eq!(ed.text(), "- alpha\n- beta\n");
    }

    #[test]
    fn tab_nests_a_list_item_but_is_still_a_tab_in_prose() {
        let mut ed = editor("- item");
        ed.move_line_end(false);
        ed.tab(true);
        assert_eq!(ed.text(), "    - item\n");
        ed.tab(false);
        assert_eq!(ed.text(), "- item\n");

        let mut ed = editor("prose");
        ed.move_line_end(false);
        ed.tab(true);
        assert_eq!(ed.text(), "prose   \n", "a tab to the next stop");
    }

    #[test]
    fn tab_indents_every_line_of_a_selection() {
        let mut ed = editor("one\ntwo\nthree");
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(1, 1);
        ed.tab(true);
        assert_eq!(ed.text(), "    one\n    two\nthree\n");
    }

    #[test]
    fn outdenting_takes_back_only_what_is_there() {
        let mut ed = editor("  two spaces");
        ed.tab(false);
        assert_eq!(ed.text(), "two spaces\n");
        ed.tab(false);
        assert_eq!(ed.text(), "two spaces\n", "and stops at the margin");
    }

    #[test]
    fn select_all_covers_the_buffer() {
        let mut ed = editor("a\nb\nc");
        ed.select_all();
        assert_eq!(ed.selected_text().as_deref(), Some("a\nb\nc"));
    }
}
