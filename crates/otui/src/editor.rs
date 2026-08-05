//! The note editor's text buffer.
//!
//! A line-vector buffer with a cursor, selection and undo. Notes are small
//! enough — a long one is a few thousand lines — that a rope would be
//! complexity without benefit, while a `Vec<String>` maps one-to-one onto how
//! the buffer is rendered and how Markdown is parsed.
//!
//! Positions are in **characters**, not bytes, so a cursor never lands inside a
//! multi-byte character.

/// A position in the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    pub line: usize,
    /// Character offset within the line.
    pub col: usize,
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
    /// Column the cursor wants when moving vertically, so travelling through a
    /// short line and back out preserves the original column.
    desired_col: usize,
    selection_anchor: Option<Cursor>,
    pub scroll: usize,
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
            desired_col: 0,
            selection_anchor: None,
            scroll: 0,
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
        self.desired_col = self.cursor.col;
    }

    pub fn move_right(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.col < self.line_len(self.cursor.line) {
            self.cursor.col += 1;
        } else if self.cursor.line + 1 < self.lines.len() {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
        self.desired_col = self.cursor.col;
    }

    pub fn move_up(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.desired_col.min(self.line_len(self.cursor.line));
        } else {
            self.cursor.col = 0;
        }
    }

    pub fn move_down(&mut self, extend: bool) {
        self.prepare_move(extend);
        if self.cursor.line + 1 < self.lines.len() {
            self.cursor.line += 1;
            self.cursor.col = self.desired_col.min(self.line_len(self.cursor.line));
        } else {
            self.cursor.col = self.line_len(self.cursor.line);
        }
    }

    pub fn move_line_start(&mut self, extend: bool) {
        self.prepare_move(extend);
        // Home goes to the first non-blank first, then to column zero — the
        // behavior every editor settled on for indented text.
        let indent = self.lines[self.cursor.line]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        self.cursor.col = if self.cursor.col == indent { 0 } else { indent };
        self.desired_col = self.cursor.col;
    }

    pub fn move_line_end(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor.col = self.line_len(self.cursor.line);
        self.desired_col = self.cursor.col;
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
        self.desired_col = self.cursor.col;
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
        self.desired_col = self.cursor.col;
    }

    pub fn move_page(&mut self, lines: isize, extend: bool) {
        self.prepare_move(extend);
        let target = (self.cursor.line as isize + lines).clamp(0, self.lines.len() as isize - 1);
        self.cursor.line = target as usize;
        self.cursor.col = self.desired_col.min(self.line_len(self.cursor.line));
    }

    pub fn move_document_start(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor = Cursor::default();
        self.desired_col = 0;
    }

    pub fn move_document_end(&mut self, extend: bool) {
        self.prepare_move(extend);
        self.cursor = self.end_position();
        self.desired_col = self.cursor.col;
    }

    /// Places the cursor at a specific position, clamped into the buffer.
    pub fn goto(&mut self, line: usize, col: usize) {
        self.cursor.line = line.min(self.lines.len().saturating_sub(1));
        self.cursor.col = col.min(self.line_len(self.cursor.line));
        self.desired_col = self.cursor.col;
        self.selection_anchor = None;
    }

    fn prepare_move(&mut self, extend: bool) {
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

    pub fn newline(&mut self) {
        self.push_undo(EditKind::Structural);
        self.delete_selection_inner();

        // Carry the current line's indentation, which is what makes editing
        // nested lists bearable.
        let indent: String = self.lines[self.cursor.line]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.split_line();
        if !indent.is_empty() {
            self.insert_into_line(&indent);
        }
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
        self.desired_col = self.cursor.col;
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

    /// Applies `f` to the status char of every task line in the selection (or the
    /// cursor line when there's no selection). Returns true if any line changed.
    pub fn transform_tasks(&mut self, mut f: impl FnMut(char) -> char) -> bool {
        let (start, end) = match self.selection() {
            Some((a, b)) => (a.line, b.line),
            None => (self.cursor.line, self.cursor.line),
        };
        let mut changed = false;
        for line in start..=end {
            if let Some((open, status, close)) = task_marker(&self.lines[line]) {
                let next = f(status);
                if next == status {
                    continue;
                }
                if !changed {
                    self.push_undo(EditKind::Structural);
                    changed = true;
                }
                let mut updated = self.lines[line].clone();
                updated.replace_range(open..close, &format!("[{next}]"));
                self.lines[line] = updated;
            }
        }
        if changed {
            self.modified = true;
        }
        changed
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
        self.desired_col = start.col;
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
        self.desired_col = self.cursor.col;
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
        self.desired_col = 0;
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

    /// Scrolls so the cursor is visible in a viewport of `height` lines.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor.line < self.scroll {
            self.scroll = self.cursor.line;
        } else if self.cursor.line >= self.scroll + height {
            self.scroll = self.cursor.line + 1 - height;
        }
    }
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

/// Returns `(open, status, close)` byte indices covering `[<status>]` if the
/// line is a task line: optional ASCII whitespace, `-`/`*`/`+`, a space, then
/// `[c]`. `open` points at `[`, `close` is one past `]`. A single status char
/// is required; `[]`, `[ab]`, or a tab after the bullet are not tasks.
fn task_marker(line: &str) -> Option<(usize, char, usize)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    // Skip ASCII whitespace (space/tab) ahead of the bullet.
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    if i >= bytes.len() || !matches!(bytes[i], b'-' | b'*' | b'+') {
        return None;
    }
    // Mirror the core parser: exactly one space after the bullet, never a tab.
    let open = i + 2;
    if open >= bytes.len() || bytes[open - 1] != b' ' || bytes[open] != b'[' {
        return None;
    }
    let close_rel = line[open..].find(']')?;
    let close = open + close_rel + 1; // one past `]`
    let status_slice = &line[open + 1..close - 1];
    if status_slice.is_empty() {
        return None;
    }
    let mut chars = status_slice.chars();
    let status = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some((open, status, close))
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

    #[test]
    fn vertical_movement_remembers_the_desired_column() {
        let mut ed = editor("longer line\nx\nanother long line");
        ed.goto(0, 9);
        ed.move_down(false);
        assert_eq!(ed.cursor().col, 1, "clamped to the short line");
        ed.move_down(false);
        assert_eq!(ed.cursor().col, 9, "restored on the longer line");
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
    fn transform_tasks_cycles_status_forward_with_wrap() {
        let mut ed = editor("- [ ] one\n- [x] two\n");
        let advanced = ed.transform_tasks(|c| match c {
            ' ' => '/',
            '/' => 'x',
            'x' => ' ',
            other => other,
        });
        assert!(advanced);
        assert_eq!(ed.text(), "- [/] one\n- [x] two\n", "cursor line only");
    }

    #[test]
    fn transform_tasks_cycles_status_backward_with_wrap() {
        let mut ed = editor("- [ ] one");
        let advanced = ed.transform_tasks(|c| match c {
            ' ' => 'x',
            '/' => ' ',
            'x' => '/',
            other => other,
        });
        assert!(advanced);
        assert_eq!(ed.text(), "- [x] one\n", "wraps back to the last state");
    }

    #[test]
    fn transform_tasks_sets_status_directly() {
        let mut ed = editor("- [x] done");
        assert!(ed.transform_tasks(|_| ' '));
        assert_eq!(ed.text(), "- [ ] done\n");
    }

    #[test]
    fn transform_tasks_skips_plain_bullet_lines_in_a_selection() {
        let mut ed = editor("- [ ] a\n- plain\n");
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(1, 3);
        assert!(ed.transform_tasks(|c| if c == ' ' { 'x' } else { c }));
        assert_eq!(ed.text(), "- [x] a\n- plain\n", "plain line untouched");
    }

    #[test]
    fn transform_tasks_on_a_non_task_line_is_a_no_op() {
        let mut ed = editor("just text\n");
        assert!(!ed.transform_tasks(|c| c));
        assert_eq!(ed.text(), "just text\n");
    }

    #[test]
    fn transform_tasks_is_a_single_undo_step() {
        let mut ed = editor("- [ ] one\n- [x] two\n");
        ed.goto(0, 0);
        ed.begin_selection();
        ed.goto_extend(1, 0);
        ed.transform_tasks(|c| if c == ' ' { 'x' } else { ' ' });
        assert_eq!(ed.text(), "- [x] one\n- [ ] two\n");

        assert!(ed.undo());
        assert_eq!(
            ed.text(),
            "- [ ] one\n- [x] two\n",
            "one undo restores every line in the operation"
        );
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

        ed.goto(50, 0);
        ed.scroll_into_view(10);
        assert_eq!(ed.scroll, 41, "cursor sits on the last visible row");

        ed.goto(5, 0);
        ed.scroll_into_view(10);
        assert_eq!(ed.scroll, 5);
    }

    #[test]
    fn select_all_covers_the_buffer() {
        let mut ed = editor("a\nb\nc");
        ed.select_all();
        assert_eq!(ed.selected_text().as_deref(), Some("a\nb\nc"));
    }
}
