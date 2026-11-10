//! A one-line text field's editing model: the text, a caret and a selection, all as byte offsets
//! on char boundaries. No drawing and no OS calls; the Tasks panel feeds it keys and draws it.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextField {
    text: String,
    caret: usize,
    /// The other end of the selection; equal to `caret` when nothing is selected.
    anchor: usize,
    /// A UTF-16 high surrogate waiting for its pair (WM_CHAR sends them one at a time).
    pending_high: Option<u16>,
    /// An input method's text being composed, shown in place until it is committed (macOS
    /// marked text; Windows never sets it).
    marked: Option<std::ops::Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
}

impl TextField {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The selected byte range, start <= end (empty when nothing is selected).
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn clear(&mut self) {
        *self = TextField::default();
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Puts the caret at byte `at` (snapped back to a char boundary), extending the selection
    /// when `extend`.
    pub fn place(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
    }

    /// Types `s` over the selection. Line breaks and other control characters are dropped,
    /// so a pasted paragraph becomes one line.
    pub fn insert(&mut self, s: &str) {
        let clean: String = s
            .chars()
            .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
            .filter(|c| !c.is_control())
            .collect();
        let sel = self.selection();
        self.text.replace_range(sel.clone(), &clean);
        self.caret = sel.start + clean.len();
        self.anchor = self.caret;
    }

    /// The selected text.
    #[cfg_attr(windows, allow(dead_code))] // the Mac's Cmd+C and Cmd+X
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// An input method's composition: `s` takes the place of the text composed so far (at
    /// first, of the selection) and the caret follows it; an empty `s` takes it away, before
    /// the committed text arrives through `insert`.
    #[cfg_attr(windows, allow(dead_code))] // macOS marked text
    pub fn compose(&mut self, s: &str) {
        if let Some(r) = self.marked.take() {
            self.anchor = r.start;
            self.caret = r.end.min(self.text.len());
        }
        let start = self.selection().start;
        self.insert(s);
        if !s.is_empty() {
            self.marked = Some(start..self.caret);
        }
    }

    /// One UTF-16 unit from WM_CHAR. Returns whether the text changed.
    pub fn type_utf16(&mut self, unit: u16) -> bool {
        if (0xD800..0xDC00).contains(&unit) {
            self.pending_high = Some(unit);
            return false;
        }
        let units: Vec<u16> = match self.pending_high.take() {
            Some(high) if (0xDC00..0xE000).contains(&unit) => vec![high, unit],
            _ => vec![unit],
        };
        let s = String::from_utf16_lossy(&units);
        if s.chars().all(|c| c.is_control()) {
            return false;
        }
        self.insert(&s);
        true
    }

    /// A navigation or delete key. `word` is Ctrl held, `extend` is Shift held.
    pub fn key(&mut self, key: Key, word: bool, extend: bool) {
        let sel = self.selection();
        match key {
            Key::Left | Key::Right if !extend && !sel.is_empty() => {
                let to = if key == Key::Left { sel.start } else { sel.end };
                self.place(to, false);
            }
            Key::Left => self.place(
                if word {
                    self.word_left()
                } else {
                    self.char_left()
                },
                extend,
            ),
            Key::Right => self.place(
                if word {
                    self.word_right()
                } else {
                    self.char_right()
                },
                extend,
            ),
            Key::Home => self.place(0, extend),
            Key::End => self.place(self.text.len(), extend),
            Key::Backspace | Key::Delete if !sel.is_empty() => self.insert(""),
            Key::Backspace => {
                let from = if word {
                    self.word_left()
                } else {
                    self.char_left()
                };
                self.text.replace_range(from..self.caret, "");
                self.place(from, false);
            }
            Key::Delete => {
                let to = if word {
                    self.word_right()
                } else {
                    self.char_right()
                };
                self.text.replace_range(self.caret..to, "");
                self.anchor = self.caret;
            }
        }
    }

    fn char_left(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn char_right(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map_or(self.caret, |c| self.caret + c.len_utf8())
    }

    /// The start of the word before the caret (skipping the spaces right before it).
    fn word_left(&self) -> usize {
        let before = &self.text[..self.caret];
        let trimmed = before.trim_end();
        trimmed.rfind(char::is_whitespace).map_or(0, |i| {
            i + trimmed[i..].chars().next().map_or(1, char::len_utf8)
        })
    }

    /// The end of the word after the caret (and the spaces after it).
    fn word_right(&self) -> usize {
        let after = &self.text[self.caret..];
        let word_end = after.find(char::is_whitespace).unwrap_or(after.len());
        let rest = &after[word_end..];
        let spaces = rest.len() - rest.trim_start().len();
        self.caret + word_end + spaces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> TextField {
        let mut f = TextField::default();
        for u in s.encode_utf16() {
            f.type_utf16(u);
        }
        f
    }

    #[test]
    fn typing_builds_the_text_and_moves_the_caret() {
        let f = typed("beli galon besok 9am");
        assert_eq!(f.text(), "beli galon besok 9am");
        assert_eq!(f.caret(), f.text().len());
    }

    #[test]
    fn a_surrogate_pair_arrives_as_one_character() {
        let f = typed("ok 👍");
        assert_eq!(f.text(), "ok 👍");
        let mut f = f;
        f.key(Key::Backspace, false, false);
        assert_eq!(f.text(), "ok ");
    }

    #[test]
    fn control_characters_are_not_typed() {
        let mut f = typed("a");
        assert!(!f.type_utf16(8)); // backspace arrives as WM_CHAR too
        assert!(!f.type_utf16(13));
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn arrows_and_deletes_work_on_characters_and_words() {
        let mut f = typed("rapat lusa jam 3");
        f.key(Key::Backspace, true, false);
        assert_eq!(f.text(), "rapat lusa jam ");
        f.key(Key::Backspace, true, false);
        assert_eq!(f.text(), "rapat lusa ");
        f.key(Key::Home, false, false);
        f.key(Key::Right, true, false);
        assert_eq!(f.caret(), "rapat ".len());
        f.key(Key::Delete, false, false);
        assert_eq!(f.text(), "rapat usa ");
        f.key(Key::Left, false, false);
        f.insert("X");
        assert_eq!(f.text(), "rapatX usa ");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut f = typed("Kirim invoice");
        f.key(Key::Left, true, true);
        assert_eq!(&f.text()[f.selection()], "invoice");
        f.insert("faktur");
        assert_eq!(f.text(), "Kirim faktur");
        f.select_all();
        f.key(Key::Backspace, false, false);
        assert!(f.is_empty());
    }

    #[test]
    fn a_pasted_paragraph_becomes_one_line() {
        let mut f = TextField::default();
        f.insert("beli\r\ngalon\tbesok");
        assert_eq!(f.text(), "beli galon besok");
    }

    #[test]
    fn a_composition_shows_in_place_then_gives_way_to_the_commit() {
        let mut f = typed("beli  besok");
        f.place("beli ".len(), false);
        f.compose("k");
        f.compose("ka");
        assert_eq!(f.text(), "beli ka besok");
        assert_eq!(f.caret(), "beli ka".len());
        f.compose("");
        assert_eq!(f.text(), "beli  besok");
        f.insert("kopi");
        assert_eq!(f.text(), "beli kopi besok");
        f.select_all();
        assert_eq!(f.selected_text(), "beli kopi besok");
    }

    #[test]
    fn placing_the_caret_snaps_to_a_character() {
        let mut f = typed("tgl 15 nov é");
        f.place(f.text().len() - 1, false); // inside the two-byte é
        assert_eq!(f.caret(), f.text().len() - 2);
    }
}
