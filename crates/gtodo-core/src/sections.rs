//! `todo.md` の管理対象セクション (Today / Next / Waiting / Someday) の名前と別名。

use crate::lua_compat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKey {
    Today,
    Next,
    Waiting,
    Someday,
}

impl SectionKey {
    /// `todo.md` 内での並び順。
    pub const ALL: [SectionKey; 4] = [
        SectionKey::Today,
        SectionKey::Next,
        SectionKey::Waiting,
        SectionKey::Someday,
    ];

    pub fn default_name(self) -> &'static str {
        match self {
            SectionKey::Today => "Today",
            SectionKey::Next => "Next",
            SectionKey::Waiting => "Waiting",
            SectionKey::Someday => "Someday",
        }
    }

    /// `gtodo.json` の `sections` で使うキー。
    pub fn settings_key(self) -> &'static str {
        match self {
            SectionKey::Today => "today",
            SectionKey::Next => "next",
            SectionKey::Waiting => "waiting",
            SectionKey::Someday => "someday",
        }
    }

    /// `.state.json` の `last_sections` で使われるキー。
    pub fn state_key(self) -> &'static str {
        match self {
            SectionKey::Today => "TODAY",
            SectionKey::Next => "NEXT",
            SectionKey::Waiting => "WAITING",
            SectionKey::Someday => "SOMEDAY",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SectionNameError {
    #[error("section name for {0:?} must not be empty")]
    Empty(SectionKey),
    #[error("section name {name:?} for {key:?} is already used by {other:?}")]
    Duplicate {
        name: String,
        key: SectionKey,
        other: SectionKey,
    },
}

/// セクション名の設定。
///
/// 見出しは「現在の名前」「デフォルト名」「前回の名前」のいずれでも受理し、解析時に
/// 現在の名前へ揃える (Lua 版 `config.section_aliases` と同じ)。デフォルト名を常に
/// 受理するのは、名前をカスタマイズしても既存ファイルの見出しを手で直させないため。
/// 前回の名前は Lua 版 `config.last_sections` に相当する。Rust 版では Neovim 版が
/// `.state.json` に記録した名前 (Neovim 版が最後に起動したときの名前) を入れる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionNames {
    current: [String; 4],
    last: [Option<String>; 4],
}

impl Default for SectionNames {
    fn default() -> Self {
        SectionNames {
            current: SectionKey::ALL.map(|k| k.default_name().to_string()),
            last: Default::default(),
        }
    }
}

impl SectionNames {
    /// 現在の名前を `SectionKey::ALL` の順で受け取る。前後の空白は取り除く。
    ///
    /// 他のセクションの現在の名前・デフォルト名と同じ名前はエラーにする。許すと2つのセクションの
    /// タスクが1つに合流して読まれ、書き戻すとそのまま混ざってしまう (Lua 版の `setup()` はデフォルト名へ差し戻す)。
    pub fn new(current: [String; 4]) -> Result<Self, SectionNameError> {
        let mut names = SectionNames::default();
        for (key, name) in SectionKey::ALL.into_iter().zip(current) {
            let name = lua_compat::trim(&name);
            if name.is_empty() {
                return Err(SectionNameError::Empty(key));
            }
            names.current[key.index()] = name.to_string();
        }
        for key in SectionKey::ALL {
            let name = names.name(key);
            let taken = |other: SectionKey| other != key && (names.name(other) == name || other.default_name() == name);
            if let Some(other) = SectionKey::ALL.into_iter().find(|&o| taken(o)) {
                return Err(SectionNameError::Duplicate {
                    name: name.to_string(),
                    key,
                    other,
                });
            }
        }
        Ok(names)
    }

    /// `gtodo.json` でセクション名を設定していないときの現在の名前。Neovim 版が最後に起動したときの名前
    /// (`.state.json` の `last_sections`) を使い、記録の無いキーはデフォルト名にする。
    ///
    /// デフォルト名を使うと、名前を変えている Neovim 版との間で、互いが書くたびに見出しが
    /// 自分の名前へ書き戻されて往復するため。
    pub fn from_last(last: &[Option<String>; 4]) -> Result<Self, SectionNameError> {
        SectionNames::new(SectionKey::ALL.map(|k| {
            last[k.index()]
                .clone()
                .filter(|n| !lua_compat::trim(n).is_empty())
                .unwrap_or_else(|| k.default_name().to_string())
        }))
    }

    /// 前回の名前 (`.state.json` の `last_sections`) を設定する。空の名前は無視する。
    pub fn with_last(mut self, key: SectionKey, name: Option<String>) -> Self {
        self.last[key.index()] = name.filter(|n| !n.is_empty());
        self
    }

    pub fn name(&self, key: SectionKey) -> &str {
        &self.current[key.index()]
    }

    /// 見出しの名前がどの管理対象セクションを指すか。
    ///
    /// 別名が他のキーの名前と衝突したときは、全キーの現在の名前 → デフォルト名 → 前回の名前の順に照合する。
    /// キーごとに別名をまとめて照合すると、Today の前回の名前が Next の現在の名前を奪い、
    /// 見出しが別のセクションへ移ってしまう (Lua 版も同じ順で照合する)。
    pub fn key_of(&self, heading: &str) -> Option<SectionKey> {
        let find = |names: [Option<&str>; 4]| SectionKey::ALL.into_iter().find(|&k| names[k.index()] == Some(heading));
        find(SectionKey::ALL.map(|k| Some(self.name(k))))
            .or_else(|| find(SectionKey::ALL.map(|k| Some(k.default_name()))))
            .or_else(|| find(SectionKey::ALL.map(|k| self.last[k.index()].as_deref())))
    }

    /// 見出しの名前を現在の名前へ揃える。管理対象外の見出しはそのまま返す。
    pub fn canonicalize<'a>(&'a self, heading: &'a str) -> &'a str {
        match self.key_of(heading) {
            Some(key) => self.name(key),
            None => heading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(today: &str) -> SectionNames {
        SectionNames::new([today.into(), "Next".into(), "Waiting".into(), "Someday".into()]).unwrap()
    }

    #[test]
    fn default_and_last_names_canonicalize_to_current_name() {
        let n = names("今日").with_last(SectionKey::Today, Some("本日".into()));
        for heading in ["今日", "Today", "本日"] {
            assert_eq!(n.canonicalize(heading), "今日", "{heading}");
        }
        assert_eq!(n.canonicalize("独自セクション"), "独自セクション");
    }

    #[test]
    fn last_names_never_take_another_sections_current_or_default_name() {
        let n = SectionNames::new(["B".into(), "A".into(), "Waiting".into(), "Someday".into()])
            .unwrap()
            .with_last(SectionKey::Today, Some("A".into()))
            .with_last(SectionKey::Next, Some("B".into()))
            .with_last(SectionKey::Waiting, Some("Someday".into()));
        assert_eq!(n.key_of("A"), Some(SectionKey::Next));
        assert_eq!(n.key_of("B"), Some(SectionKey::Today));
        assert_eq!(n.key_of("Someday"), Some(SectionKey::Someday));
    }

    #[test]
    fn empty_last_name_is_ignored() {
        let n = SectionNames::default().with_last(SectionKey::Today, Some(String::new()));
        assert_eq!(n.key_of(""), None);
    }

    #[test]
    fn current_names_are_trimmed_and_must_not_be_blank() {
        assert_eq!(names("  今日\t").name(SectionKey::Today), "今日");
        let err = SectionNames::new([" ".into(), "Next".into(), "Waiting".into(), "Someday".into()]).unwrap_err();
        assert_eq!(err, SectionNameError::Empty(SectionKey::Today));
    }

    #[test]
    fn names_neovim_used_last_become_current_names_and_the_rest_stay_default() {
        let n = SectionNames::from_last(&[Some("今日".into()), Some(" ".into()), None, Some("いつか".into())]).unwrap();
        assert_eq!(
            SectionKey::ALL.map(|k| n.name(k)),
            ["今日", "Next", "Waiting", "いつか"]
        );

        let err = SectionNames::from_last(&[Some("Next".into()), None, None, None]).unwrap_err();
        assert!(matches!(err, SectionNameError::Duplicate { .. }), "{err:?}");
    }

    #[test]
    fn a_name_used_by_another_section_is_rejected() {
        for (current, other) in [
            (["Next", "Next", "Waiting", "Someday"], SectionKey::Next),
            (["今日", "今日", "Waiting", "Someday"], SectionKey::Next),
            (["今日", "Today", "Waiting", "Someday"], SectionKey::Today),
        ] {
            let err = SectionNames::new(current.map(String::from)).unwrap_err();
            assert!(
                matches!(&err, SectionNameError::Duplicate { other: o, .. } if *o == other),
                "{current:?}: {err:?}"
            );
        }
    }
}
