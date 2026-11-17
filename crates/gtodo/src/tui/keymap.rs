//! キーと動作の対応。
//!
//! 最下段の案内と `?` のヘルプは、ここに登録したキーから作る (キーを変えたときに表示と食い違わないため)。
//! 複数打鍵 (`gg`) は待ち時間を設けない。そのため、あるキー列を別のキー列の途中にしてはならない
//! (`g` と `gg` の両方を登録すると、`g` を押した時点でどちらか決められない)。

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 1 回のキー入力。文字キーの Shift は文字そのもの (`G`) に表れるため持たない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub fn from_event(event: &KeyEvent) -> Key {
        Key {
            code: event.code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
        }
    }

    pub const fn plain(code: KeyCode) -> Key {
        Key {
            code,
            ctrl: false,
            alt: false,
        }
    }

    pub const fn char(c: char) -> Key {
        Key::plain(KeyCode::Char(c))
    }

    pub const fn ctrl(c: char) -> Key {
        Key {
            code: KeyCode::Char(c),
            ctrl: true,
            alt: false,
        }
    }

    /// 表示用の名前 (`x`・`Ctrl-d`・`↓` など)。
    pub fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "S-Tab".to_string(),
            KeyCode::Backspace => "BS".to_string(),
            KeyCode::Delete => "Del".to_string(),
            KeyCode::Insert => "Ins".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        };
        let mut label = String::new();
        if self.ctrl {
            label.push_str("Ctrl-");
        }
        if self.alt {
            label.push_str("Alt-");
        }
        label.push_str(&base);
        label
    }
}

/// キー列の表示用の名前 (`gg`・`Ctrl-d` など)。
pub fn sequence_label(keys: &[Key]) -> String {
    keys.iter().map(Key::label).collect()
}

/// キーに割り当てられる動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Down,
    Up,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    NextTab,
    PrevTab,
    /// 0 から数えたタブの位置。
    GoTab(usize),
    MoveToday,
    MoveNext,
    MoveWaiting,
    MoveSomeday,
    ToggleDone,
    Cancel,
    Sort,
    Add,
    Edit,
    OpenQueue,
    OpenKanban,
    Search,
    /// 表示を切り替える (Queue は期日別と待ち先別、検索は状態)。
    SwitchView,
    /// Queue で選んでいるタスクを一覧で開く。
    Open,
    /// Queue などから一覧へ戻る。`Esc` に固定で、割り当てを変えられない (案内に出すためだけにある)。
    Back,
    Help,
    Quit,
}

/// タブの数 (Inbox・4 つのセクション・Done・Cancelled)。
pub const TAB_COUNT: usize = 7;

/// 登録したキーの対応。先に登録したキーが最下段の案内に出る。
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<(Vec<Key>, Action)>,
}

impl Default for Keymap {
    fn default() -> Self {
        use Action::*;
        let mut bindings: Vec<(Vec<Key>, Action)> = vec![
            (vec![Key::char('j')], Down),
            (vec![Key::plain(KeyCode::Down)], Down),
            (vec![Key::char('k')], Up),
            (vec![Key::plain(KeyCode::Up)], Up),
            (vec![Key::char('g'), Key::char('g')], Top),
            (vec![Key::plain(KeyCode::Home)], Top),
            (vec![Key::char('G')], Bottom),
            (vec![Key::plain(KeyCode::End)], Bottom),
            (vec![Key::ctrl('d')], HalfPageDown),
            (vec![Key::ctrl('u')], HalfPageUp),
            (vec![Key::ctrl('f')], PageDown),
            (vec![Key::plain(KeyCode::PageDown)], PageDown),
            (vec![Key::ctrl('b')], PageUp),
            (vec![Key::plain(KeyCode::PageUp)], PageUp),
            (vec![Key::char('g'), Key::char('t')], NextTab),
            (vec![Key::char('l')], NextTab),
            (vec![Key::plain(KeyCode::Right)], NextTab),
            (vec![Key::char('g'), Key::char('T')], PrevTab),
            (vec![Key::char('h')], PrevTab),
            (vec![Key::plain(KeyCode::Left)], PrevTab),
            (vec![Key::char('x')], ToggleDone),
            (vec![Key::char('d')], MoveToday),
            (vec![Key::char('n')], MoveNext),
            (vec![Key::char('w')], MoveWaiting),
            (vec![Key::char('s')], MoveSomeday),
            (vec![Key::char('c')], Cancel),
            (vec![Key::char('o')], Sort),
            (vec![Key::char('a')], Add),
            (vec![Key::char('e')], Edit),
            (vec![Key::char('Q')], OpenQueue),
            (vec![Key::char('K')], OpenKanban),
            (vec![Key::char('/')], Search),
            (vec![Key::plain(KeyCode::Tab)], SwitchView),
            (vec![Key::plain(KeyCode::Enter)], Open),
            (vec![Key::char('?')], Help),
            // Esc は終了に割り当てない。戻る・取り消すだけにする (lazygit・lazydocker と同じ)。
            // vim を使う人は癖で押すため、一覧で押して終了すると困る。
            (vec![Key::char('q')], Quit),
        ];
        for (i, c) in ('1'..='7').enumerate() {
            bindings.push((vec![Key::char(c)], GoTab(i)));
        }
        Keymap { bindings }
    }
}

/// [`Keymap::feed`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fed {
    Action(Action),
    /// 続きのキーを待っている。
    Pending,
    /// どのキー列にも当たらない。
    Unbound,
}

impl Keymap {
    /// `action` に割り当てたキー列 (登録した順)。
    pub fn keys_for(&self, action: Action) -> impl Iterator<Item = &[Key]> {
        self.bindings
            .iter()
            .filter(move |(_, a)| *a == action)
            .map(|(keys, _)| keys.as_slice())
    }

    /// `action` に最初に割り当てたキー列の表示名。割り当てが無ければ `None`。
    pub fn label(&self, action: Action) -> Option<String> {
        self.keys_for(action).next().map(sequence_label)
    }

    /// `pending` (押しかけのキー列) に `key` を足して、動作が決まれば返す。
    ///
    /// 決まったとき・どれにも当たらなかったときは `pending` を空にする。押しかけの途中で
    /// 当たらないキーが来たら、そのキーだけで始め直す (`g` の後の `j` は下へ移る)。
    pub fn feed(&self, pending: &mut Vec<Key>, key: Key) -> Fed {
        pending.push(key);
        if let Some(action) = self.exact(pending) {
            pending.clear();
            return Fed::Action(action);
        }
        if self.is_prefix(pending) {
            return Fed::Pending;
        }
        let restart = pending.len() > 1;
        pending.clear();
        if restart {
            return self.feed(pending, key);
        }
        Fed::Unbound
    }

    /// 押しかけのキー列の続きになるキー列と動作 (最下段に候補として出す)。
    pub fn continuations<'a>(&'a self, pending: &'a [Key]) -> impl Iterator<Item = (&'a [Key], Action)> + 'a {
        self.bindings
            .iter()
            .filter(move |(keys, _)| keys.len() > pending.len() && keys.starts_with(pending))
            .map(|(keys, action)| (keys.as_slice(), *action))
    }

    fn exact(&self, keys: &[Key]) -> Option<Action> {
        self.bindings.iter().find(|(k, _)| k == keys).map(|(_, a)| *a)
    }

    fn is_prefix(&self, keys: &[Key]) -> bool {
        self.bindings
            .iter()
            .any(|(k, _)| k.len() > keys.len() && k.starts_with(keys))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(map: &Keymap, keys: &[Key]) -> Vec<Fed> {
        let mut pending = Vec::new();
        keys.iter().map(|k| map.feed(&mut pending, *k)).collect()
    }

    #[test]
    fn single_keys_resolve_immediately() {
        let map = Keymap::default();
        assert_eq!(feed_all(&map, &[Key::char('j')]), [Fed::Action(Action::Down)]);
        assert_eq!(feed_all(&map, &[Key::ctrl('d')]), [Fed::Action(Action::HalfPageDown)]);
        assert_eq!(feed_all(&map, &[Key::char('G')]), [Fed::Action(Action::Bottom)]);
    }

    #[test]
    fn sequences_wait_for_the_next_key() {
        let map = Keymap::default();
        let g = Key::char('g');
        assert_eq!(feed_all(&map, &[g, g]), [Fed::Pending, Fed::Action(Action::Top)]);
        assert_eq!(
            feed_all(&map, &[g, Key::char('T')]),
            [Fed::Pending, Fed::Action(Action::PrevTab)]
        );
    }

    #[test]
    fn an_unbound_key_after_a_prefix_starts_over() {
        let map = Keymap::default();
        let mut pending = Vec::new();
        assert_eq!(map.feed(&mut pending, Key::char('g')), Fed::Pending);
        assert_eq!(map.feed(&mut pending, Key::char('j')), Fed::Action(Action::Down));
        assert!(pending.is_empty());
        assert_eq!(map.feed(&mut pending, Key::char('g')), Fed::Pending);
        assert_eq!(map.feed(&mut pending, Key::char('z')), Fed::Unbound);
        assert!(pending.is_empty());
    }

    #[test]
    fn no_default_sequence_is_a_prefix_of_another() {
        // 待ち時間を設けないため、途中で決まるキー列があると長い方を押せない。
        let map = Keymap::default();
        for (a, _) in &map.bindings {
            for (b, _) in &map.bindings {
                assert!(
                    !(b.len() > a.len() && b.starts_with(a)),
                    "{} is a prefix of {}",
                    sequence_label(a),
                    sequence_label(b)
                );
            }
        }
    }

    #[test]
    fn labels() {
        let map = Keymap::default();
        assert_eq!(map.label(Action::Top).as_deref(), Some("gg"));
        assert_eq!(map.label(Action::HalfPageUp).as_deref(), Some("Ctrl-u"));
        assert_eq!(Key::plain(KeyCode::Down).label(), "↓");
    }

    #[test]
    fn continuations_list_what_can_follow() {
        let map = Keymap::default();
        let pending = [Key::char('g')];
        let labels: Vec<String> = map.continuations(&pending).map(|(k, _)| sequence_label(k)).collect();
        assert_eq!(labels, ["gg", "gt", "gT"]);
    }
}
