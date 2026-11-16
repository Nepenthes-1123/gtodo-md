//! 1 行の入力欄。

use ratatui::crossterm::event::KeyCode;
use unicode_width::UnicodeWidthStr;

use super::keymap::Key;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    text: String,
    /// カーソルの位置 (文字境界のバイト位置)。
    cursor: usize,
}

/// [`LineInput::handle`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Submit,
    Abort,
    /// 内容かカーソルが変わった (または何も起きなかった)。
    Continue,
}

impl LineInput {
    pub fn new(text: &str) -> LineInput {
        LineInput {
            text: text.to_string(),
            cursor: text.len(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// カーソルより前の表示幅 (端末のカーソルを置く位置)。
    pub fn cursor_width(&self) -> usize {
        self.text[..self.cursor].width()
    }

    pub fn handle(&mut self, key: Key) -> Edit {
        match (key.code, key.ctrl) {
            (KeyCode::Enter, _) => return Edit::Submit,
            (KeyCode::Esc, _) => return Edit::Abort,
            (KeyCode::Char('u'), true) => {
                self.text.replace_range(..self.cursor, "");
                self.cursor = 0;
            }
            (KeyCode::Char('a'), true) | (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::Char('e'), true) | (KeyCode::End, _) => self.cursor = self.text.len(),
            (KeyCode::Char(c), false) if !key.alt => self.insert(&c.to_string()),
            (KeyCode::Backspace, _) => {
                if let Some(prev) = self.prev_boundary() {
                    self.text.replace_range(prev..self.cursor, "");
                    self.cursor = prev;
                }
            }
            (KeyCode::Delete, _) => {
                if let Some(next) = self.next_boundary() {
                    self.text.replace_range(self.cursor..next, "");
                }
            }
            (KeyCode::Left, _) => {
                if let Some(prev) = self.prev_boundary() {
                    self.cursor = prev;
                }
            }
            (KeyCode::Right, _) => {
                if let Some(next) = self.next_boundary() {
                    self.cursor = next;
                }
            }
            _ => {}
        }
        Edit::Continue
    }

    /// 貼り付けた文字列を入れる。1 行の欄なので改行は空白にする。
    pub fn paste(&mut self, text: &str) {
        let line: String = text
            .chars()
            .filter(|c| *c != '\r')
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        self.insert(&line);
    }

    fn insert(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor].char_indices().next_back().map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(input: &mut LineInput, keys: &[Key]) {
        for k in keys {
            input.handle(*k);
        }
    }

    #[test]
    fn edits_at_the_cursor_by_character() {
        let mut input = LineInput::new("田中");
        assert_eq!(input.cursor_width(), 4);
        press(&mut input, &[Key::plain(KeyCode::Left), Key::char('x')]);
        assert_eq!(input.text(), "田x中");
        assert_eq!(input.cursor_width(), 3);
        press(
            &mut input,
            &[Key::plain(KeyCode::Backspace), Key::plain(KeyCode::Backspace)],
        );
        assert_eq!(input.text(), "中");
        press(&mut input, &[Key::plain(KeyCode::Delete)]);
        assert_eq!(input.text(), "");
        press(&mut input, &[Key::plain(KeyCode::Backspace), Key::plain(KeyCode::Left)]);
        assert_eq!(input.text(), "");
    }

    #[test]
    fn ctrl_u_clears_before_the_cursor() {
        let mut input = LineInput::new("bob");
        press(&mut input, &[Key::plain(KeyCode::Left), Key::ctrl('u')]);
        assert_eq!(input.text(), "b");
        assert_eq!(input.cursor_width(), 0);
    }

    #[test]
    fn enter_and_esc_finish() {
        let mut input = LineInput::default();
        assert_eq!(input.handle(Key::plain(KeyCode::Enter)), Edit::Submit);
        assert_eq!(input.handle(Key::plain(KeyCode::Esc)), Edit::Abort);
    }

    #[test]
    fn pasted_newlines_become_spaces() {
        let mut input = LineInput::default();
        input.paste("a\r\nb");
        assert_eq!(input.text(), "a b");
    }
}
