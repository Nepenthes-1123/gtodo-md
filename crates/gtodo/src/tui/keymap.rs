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
    /// 選んでいるタスクのリンクを開く。
    OpenLink,
    Search,
    /// 表示を切り替える (Queue は期日別と待ち先別、検索は状態)。
    SwitchView,
    /// Queue で選んでいるタスクを一覧で開く。
    Open,
    /// ファイルを `$EDITOR` で開く。
    EditFile,
    /// プロジェクト画面を開く。
    OpenProjects,
    /// テンプレート画面を開く。
    OpenTemplates,
    /// プロジェクト画面で、アーカイブする (アーカイブしたものの表示では復元する)。
    Archive,
    /// Queue などから一覧へ戻る。`Esc` に固定で、割り当てを変えられない (案内に出すためだけにある)。
    Back,
    Help,
    Quit,
}

/// タブの数 (Inbox・4 つのセクション・Done・Cancelled)。
pub const TAB_COUNT: usize = 7;

impl Action {
    /// `tui.toml` で割り当てを変えられる動作 (ヘルプと README の順)。`Back` は `Esc` に固定。
    pub fn configurable() -> Vec<Action> {
        use Action::*;
        let mut actions = vec![
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
        ];
        actions.extend((0..TAB_COUNT).map(GoTab));
        actions.extend([
            ToggleDone,
            MoveToday,
            MoveNext,
            MoveWaiting,
            MoveSomeday,
            Cancel,
            Add,
            Edit,
            Sort,
            OpenQueue,
            OpenKanban,
            OpenProjects,
            OpenTemplates,
            Search,
            OpenLink,
            SwitchView,
            Open,
            EditFile,
            Archive,
            Help,
            Quit,
        ]);
        actions
    }

    /// `tui.toml` での名前。
    pub fn name(self) -> String {
        use Action::*;
        match self {
            Down => "down".into(),
            Up => "up".into(),
            Top => "top".into(),
            Bottom => "bottom".into(),
            HalfPageDown => "half_page_down".into(),
            HalfPageUp => "half_page_up".into(),
            PageDown => "page_down".into(),
            PageUp => "page_up".into(),
            NextTab => "next_tab".into(),
            PrevTab => "prev_tab".into(),
            GoTab(i) => format!("tab_{}", i + 1),
            MoveToday => "move_today".into(),
            MoveNext => "move_next".into(),
            MoveWaiting => "move_waiting".into(),
            MoveSomeday => "move_someday".into(),
            ToggleDone => "toggle_done".into(),
            Cancel => "cancel".into(),
            Sort => "sort".into(),
            Add => "add".into(),
            Edit => "edit".into(),
            OpenQueue => "queue".into(),
            OpenKanban => "kanban".into(),
            OpenLink => "open_link".into(),
            Search => "search".into(),
            SwitchView => "switch_view".into(),
            Open => "open".into(),
            EditFile => "edit_file".into(),
            OpenProjects => "projects".into(),
            OpenTemplates => "templates".into(),
            Archive => "archive".into(),
            Back => "back".into(),
            Help => "help".into(),
            Quit => "quit".into(),
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::configurable().into_iter().find(|a| a.name() == name)
    }
}

/// キーの書き方 (vim と同じ)。1 文字はそのまま (`x`・`G`・`?`)、特別なキーと修飾は `<...>`
/// (`<Down>`・`<C-d>`・`<A-x>`・`<S-Tab>`・`<Space>`・`<lt>` は `<`)。並べると続けて押すキー (`gg`・`g<Down>`)。
pub fn parse_keys(text: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        // `<` の後に `>` が無い、または `>` までにもう 1 つ `<` があれば、その `<` は文字 (vim と同じ)。
        if c == '<'
            && let Some(end) = rest.find('>')
            && !rest[1..end].contains('<')
        {
            keys.push(parse_special(&rest[1..end]).ok_or_else(|| format!("unknown key {:?}", &rest[..=end]))?);
            rest = &rest[end + 1..];
        } else {
            keys.push(Key::char(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    if keys.is_empty() {
        return Err("an empty key".into());
    }
    for key in &keys {
        // 取り消しと終了は、どの画面でも同じに使えるよう変えさせない。
        if key.code == KeyCode::Esc || (key.ctrl && key.code == KeyCode::Char('c')) {
            return Err(format!(
                "{} is reserved (Esc cancels and goes back, Ctrl-c quits)",
                key.label()
            ));
        }
    }
    Ok(keys)
}

/// `<...>` の中身。
fn parse_special(inner: &str) -> Option<Key> {
    let mut key = Key::plain(KeyCode::Null);
    let mut shift = false;
    let mut name = inner;
    while name.len() > 2 && name.as_bytes()[1] == b'-' {
        match name.as_bytes()[0].to_ascii_lowercase() {
            b'c' => key.ctrl = true,
            b'a' | b'm' => key.alt = true,
            b's' => shift = true,
            _ => return None,
        }
        name = &name[2..];
    }
    let lower = name.to_ascii_lowercase();
    key.code = match lower.as_str() {
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "enter" | "cr" | "return" => KeyCode::Enter,
        "tab" if shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "ins" | "insert" => KeyCode::Insert,
        "space" => KeyCode::Char(' '),
        "esc" => KeyCode::Esc,
        "lt" => KeyCode::Char('<'),
        _ if lower.len() > 1 && lower.starts_with('f') => {
            KeyCode::F(lower[1..].parse().ok().filter(|n| (1..=24).contains(n))?)
        }
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if shift {
                // Shift は大文字そのもので表す (`<S-a>` は `A`)。端末は Ctrl と Shift の付いた英字を
                // Ctrl だけの英字と区別して送らず、Shift の付いた記号も記号そのもので送るため、
                // それ以外の組み合わせは押せないキーとして断る (別のキーとして読まないように)。
                if key.ctrl || !c.is_ascii_alphabetic() {
                    return None;
                }
                KeyCode::Char(c.to_ascii_uppercase())
            } else if key.ctrl {
                // 端末は Ctrl と英字を小文字で送る。
                KeyCode::Char(c.to_ascii_lowercase())
            } else {
                KeyCode::Char(c)
            }
        }
    };
    if shift && !matches!(key.code, KeyCode::BackTab | KeyCode::Char(_)) {
        return None;
    }
    Some(key)
}

/// `tui.toml` を読んだ結果。
#[derive(Debug)]
pub struct Loaded {
    pub keymap: Keymap,
    /// 続けられるが知らせる問題 (知らない名前など)。
    pub warnings: Vec<String>,
}

impl Keymap {
    /// `tui.toml` の中身から作る。既定の割り当てを、書かれた動作ごとに置き換える。
    ///
    /// 知らない名前は警告して無視する (バージョンの違う gtodo で同じファイルを使うため。
    /// 共有の設定ファイルと同じ方針)。同じキーが 2 つの動作に付いている・あるキー列が別の
    /// キー列の途中になっている (待ち時間を設けないので長い方を押せない) ときはエラー。
    pub fn from_toml(text: &str) -> Result<Loaded, String> {
        let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut keymap = Keymap::default();
        let mut warnings = Vec::new();
        for (key, value) in &table {
            if key != "keys" {
                warnings.push(format!("unknown section {key:?} is ignored"));
                continue;
            }
            let Some(keys) = value.as_table() else {
                return Err("[keys] must be a table".into());
            };
            for (name, value) in keys {
                let Some(action) = Action::from_name(name) else {
                    warnings.push(if name == "back" {
                        format!("keys.{name}: Esc always goes back and cannot be changed; ignored")
                    } else {
                        format!("keys.{name}: unknown action is ignored")
                    });
                    continue;
                };
                let texts: Vec<&str> = match value {
                    toml::Value::String(s) => vec![s.as_str()],
                    toml::Value::Array(items) => items
                        .iter()
                        .map(|v| v.as_str().ok_or_else(|| format!("keys.{name}: keys must be strings")))
                        .collect::<Result<_, _>>()?,
                    _ => {
                        return Err(format!(
                            "keys.{name}: write a key as a string, or several keys as an array"
                        ));
                    }
                };
                let sequences = texts
                    .into_iter()
                    .map(|t| parse_keys(t).map_err(|e| format!("keys.{name}: {e}")))
                    .collect::<Result<Vec<_>, _>>()?;
                keymap.bindings.retain(|(_, a)| *a != action);
                for seq in sequences {
                    if !keymap.bindings.iter().any(|(k, a)| *k == seq && *a == action) {
                        keymap.bindings.push((seq, action));
                    }
                }
            }
        }
        keymap.validate()?;
        Ok(Loaded { keymap, warnings })
    }

    /// 同じキー列が 2 つの動作に付いていないか、あるキー列が別のキー列の途中になっていないか。
    fn validate(&self) -> Result<(), String> {
        for (i, (a, action_a)) in self.bindings.iter().enumerate() {
            for (b, action_b) in &self.bindings[i + 1..] {
                if a == b {
                    return Err(format!(
                        "{} is bound to both {} and {}",
                        sequence_label(a),
                        action_a.name(),
                        action_b.name()
                    ));
                }
                let (short, long, short_action, long_action) = if a.len() < b.len() {
                    (a, b, action_a, action_b)
                } else {
                    (b, a, action_b, action_a)
                };
                if long.starts_with(short) {
                    return Err(format!(
                        "{} ({}) is the start of {} ({}); keys are read without waiting, so {} could not be typed",
                        sequence_label(short),
                        short_action.name(),
                        sequence_label(long),
                        long_action.name(),
                        sequence_label(long)
                    ));
                }
            }
        }
        Ok(())
    }
}

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
            (vec![Key::char('g'), Key::char('x')], OpenLink),
            (vec![Key::char('/')], Search),
            (vec![Key::plain(KeyCode::Tab)], SwitchView),
            (vec![Key::plain(KeyCode::Enter)], Open),
            (vec![Key::char('E')], EditFile),
            (vec![Key::char('P')], OpenProjects),
            (vec![Key::char('T')], OpenTemplates),
            (vec![Key::char('X')], Archive),
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
    fn key_notation_follows_vim() {
        let one = |t: &str| parse_keys(t).unwrap();
        assert_eq!(one("x"), [Key::char('x')]);
        assert_eq!(one("gg"), [Key::char('g'), Key::char('g')]);
        assert_eq!(one("<C-d>"), [Key::ctrl('d')]);
        assert_eq!(one("<c-D>"), [Key::ctrl('d')], "case is ignored in <...>");
        assert_eq!(one("<Down>"), [Key::plain(KeyCode::Down)]);
        assert_eq!(one("g<Down>"), [Key::char('g'), Key::plain(KeyCode::Down)]);
        assert_eq!(one("<S-Tab>"), [Key::plain(KeyCode::BackTab)]);
        assert_eq!(one("<Space>"), [Key::char(' ')]);
        assert_eq!(one("<lt>"), [Key::char('<')]);
        assert_eq!(one("<"), [Key::char('<')]);
        assert_eq!(one("<F5>"), [Key::plain(KeyCode::F(5))]);
        assert_eq!(one("<S-a>"), [Key::char('A')]);
        assert_eq!(
            one("<<Down>"),
            [Key::char('<'), Key::plain(KeyCode::Down)],
            "a < before <...> is a letter"
        );
        assert_eq!(one("a<b"), [Key::char('a'), Key::char('<'), Key::char('b')]);
        assert_eq!(
            one("<A-x>"),
            [Key {
                code: KeyCode::Char('x'),
                ctrl: false,
                alt: true
            }]
        );
        assert_eq!(one("<CR>"), [Key::plain(KeyCode::Enter)]);
        // `<C-S-a>` と `<S-1>` は端末から区別して届かないので、別のキーとして読まずに断る。
        for bad in [
            "", "<Dwn>", "<F0>", "<S-Down>", "<X-a>", "<Esc>", "<C-c>", "<C-S-a>", "<S-1>",
        ] {
            assert!(parse_keys(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn every_configurable_action_has_a_name_that_reads_back() {
        for action in Action::configurable() {
            assert_eq!(Action::from_name(&action.name()), Some(action), "{action:?}");
        }
        assert_eq!(Action::from_name("back"), None, "Esc is not configurable");
    }

    #[test]
    fn the_default_keys_are_valid() {
        Keymap::default().validate().unwrap();
        let loaded = Keymap::from_toml("").unwrap();
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.keymap.label(Action::Down).as_deref(), Some("j"));
    }

    #[test]
    fn listed_actions_replace_their_default_keys() {
        let loaded = Keymap::from_toml(
            r#"
            # 下へは Ctrl-n と矢印
            [keys]
            down = ["<C-n>", "<Down>"]
            toggle_done = "<Space>"
            cancel = []
            "#,
        )
        .unwrap();
        let map = loaded.keymap;
        let keys = |a| map.keys_for(a).map(sequence_label).collect::<Vec<_>>();
        assert_eq!(keys(Action::Down), ["Ctrl-n", "↓"]);
        assert_eq!(keys(Action::ToggleDone), ["Space"]);
        assert!(keys(Action::Cancel).is_empty(), "an empty list unbinds the action");
        assert_eq!(keys(Action::Up), ["k", "↑"], "other actions keep their defaults");
        let mut pending = Vec::new();
        assert_eq!(map.feed(&mut pending, Key::char('j')), Fed::Unbound);
        assert_eq!(map.feed(&mut pending, Key::ctrl('n')), Fed::Action(Action::Down));
    }

    #[test]
    fn unknown_names_are_warned_about_and_ignored() {
        let loaded = Keymap::from_toml(
            r#"
            [keys]
            fly = "f"
            back = "<BS>"
            down = "j"

            [colors]
            done = "grey"
            "#,
        )
        .unwrap();
        assert_eq!(
            loaded.warnings,
            [
                "unknown section \"colors\" is ignored",
                "keys.back: Esc always goes back and cannot be changed; ignored",
                "keys.fly: unknown action is ignored"
            ]
        );
    }

    #[test]
    fn clashing_keys_are_errors() {
        let err = |t: &str| Keymap::from_toml(t).unwrap_err();
        assert_eq!(err("[keys]\nquit = \"x\""), "x is bound to both toggle_done and quit");
        assert_eq!(
            err("[keys]\nquit = \"g\""),
            "g (quit) is the start of gg (top); keys are read without waiting, so gg could not be typed"
        );
        assert_eq!(
            err("[keys]\nup = [\"<Up>\", \"<Up>x\"]"),
            "↑ (up) is the start of ↑x (up); keys are read without waiting, so ↑x could not be typed"
        );
        assert!(err("[keys]\nquit = 1").contains("keys.quit: write a key as a string"));
        assert!(err("[keys]\nquit = [1]").contains("keys.quit: keys must be strings"));
        assert!(err("[keys]\nquit = \"<Esc>\"").contains("keys.quit: Esc is reserved"));
        assert!(err("keys = 1").contains("[keys] must be a table"));
        // TOML の書き方の誤りは、何行目かが分かる。
        assert!(
            err("[keys]\nquit = \"q").contains("line 2"),
            "{}",
            err("[keys]\nquit = \"q")
        );
    }

    #[test]
    fn the_readme_lists_every_configurable_action() {
        // `add` などは CLI の説明にも出るので、tui.toml の節の表の行だけを見る。
        let readme = include_str!("../../../../README.md");
        let (_, section) = readme.split_once("### TUI のキー (`tui.toml`)").unwrap();
        let section = section.split("\n#").next().unwrap();
        let table: Vec<&str> = section.lines().filter(|l| l.starts_with("| `")).collect();
        let listed = |text: &str| table.iter().any(|row| row.contains(text));
        for action in Action::configurable() {
            let name = action.name();
            // `tab_1`〜`tab_7` は 1 行にまとめてある。
            let listed = listed(&format!("`{name}`")) || name.starts_with("tab_");
            assert!(listed, "README.md does not list `{name}` in the tui.toml table");
        }
        assert!(listed("`tab_1` 〜 `tab_7`"));
    }

    #[test]
    fn continuations_list_what_can_follow() {
        let map = Keymap::default();
        let pending = [Key::char('g')];
        let labels: Vec<String> = map.continuations(&pending).map(|(k, _)| sequence_label(k)).collect();
        assert_eq!(labels, ["gg", "gt", "gT", "gx"]);
    }
}
