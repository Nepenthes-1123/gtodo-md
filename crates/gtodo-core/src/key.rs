//! CLI・TUI でタスクを指定するためのキー。

use crate::document::{Document, Item, TaskItem};
use crate::store::FileKind;
use crate::task::Task;

/// タスクを指定するキー。ID があれば ID、無ければ行の内容から作った一時キー (`~` + 6桁16進)。
///
/// 一時キーは行番号ではなく行の内容から作る。ソートや他のインスタンスの書き込みで行の
/// 位置がずれても同じタスクを指し、行が書き換えられていれば一致しなくなるため、
/// 別のタスクを操作してしまうことがない。
pub fn task_key(task: &Task, line: &str) -> String {
    match task.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => line_key(line),
    }
}

/// 行の内容から作る一時キー。
///
/// プロセスや Rust のバージョンをまたいで同じ値になる必要があるため、
/// 標準ライブラリのハッシュ (安定性が保証されない) ではなく FNV-1a を使う。
pub fn line_key(line: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in line.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("~{:06x}", (h ^ (h >> 32)) & 0xFF_FFFF)
}

/// [`find`] で見つけたタスクの位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub file: FileKind,
    /// [`Document::sections`] の添字。`None` は最初の `##` より前 ([`Document::preamble`])。
    pub section: Option<usize>,
    /// セクション (または preamble) の項目の添字。
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    #[error("no task in inbox.md or todo.md has the key {0:?}")]
    NotFound(String),
    #[error(
        "no task without an ID matches {0:?}; Neovim may have added an ID when it saved the file, so run `gtodo list` again"
    )]
    LineChanged(String),
    #[error("{key:?} matches more than one task:{candidates}")]
    Ambiguous { key: String, candidates: String },
    #[error("{0:?} is too short; give at least {MIN_PREFIX_LEN} characters of the ID, or the whole ID")]
    TooShort(String),
}

/// ID の先頭の一部で指定するときの最小の文字数。打ち間違いがたまたま別のタスク 1 つに一致すると、
/// そのタスクを完了・キャンセルしてしまうため。
pub const MIN_PREFIX_LEN: usize = 4;

/// キーに一致するタスクを探す。ID は先頭の [`MIN_PREFIX_LEN`] 文字以上でもよい。一時キー (`~` 始まり) は
/// ID の無いタスクの行の内容と照らし合わせる。
///
/// ID はファイルの中でしか重複が直されないため、ファイルをまたいで同じ ID がありうる。
/// 複数のタスクに一致したら、どれかを選ばずにエラーにする。ただし ID 全体が一致するタスクが
/// あればそちらだけを候補にする。手で書いた短い ID が、他の ID の先頭の一部になっていても指定できるように。
pub fn find(docs: &[(FileKind, &Document)], key: &str) -> Result<Location, FindError> {
    let long_enough = key.chars().count() >= MIN_PREFIX_LEN;
    let matches = |item: &TaskItem| {
        let id = item.task.id.as_deref().filter(|id| !id.is_empty());
        match key.strip_prefix('~') {
            Some(_) => id.is_none() && line_key(&item.line) == key,
            None => id.is_some_and(|id| id == key || (long_enough && id.starts_with(key))),
        }
    };
    let mut found = Vec::new();
    for &(file, doc) in docs {
        let groups = std::iter::once((None, &doc.preamble))
            .chain(doc.sections.iter().enumerate().map(|(i, s)| (Some(i), &s.items)));
        for (section, items) in groups {
            for (index, item) in items.iter().enumerate() {
                if let Item::Task(t) = item
                    && matches(t)
                {
                    found.push((Location { file, section, index }, t.as_ref()));
                }
            }
        }
    }
    if found.iter().any(|(_, t)| t.task.id.as_deref() == Some(key)) {
        found.retain(|(_, t)| t.task.id.as_deref() == Some(key));
    }
    match found.as_slice() {
        [(location, _)] => Ok(*location),
        [] if key.starts_with('~') => Err(FindError::LineChanged(key.to_string())),
        [] if !long_enough => Err(FindError::TooShort(key.to_string())),
        [] => Err(FindError::NotFound(key.to_string())),
        many => Err(FindError::Ambiguous {
            key: key.to_string(),
            candidates: many
                .iter()
                .map(|(l, t)| {
                    format!(
                        "\n  {} {}: {}",
                        l.file.file_name(),
                        task_key(&t.task, &t.line),
                        t.task.content
                    )
                })
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_with_id_is_keyed_by_id() {
        let task = Task::parse("- [ ] x id:a1b2c3").unwrap();
        assert_eq!(task_key(&task, "- [ ] x id:a1b2c3"), "a1b2c3");
    }

    #[test]
    fn task_without_id_is_keyed_by_line_content() {
        let line = "- [ ] IDの無いタスク";
        let task = Task::parse(line).unwrap();
        let key = task_key(&task, line);
        assert!(key.starts_with('~') && key.len() == 7, "{key}");
        assert_eq!(key, line_key(line));
    }

    #[test]
    fn line_key_is_stable_across_releases() {
        // 期待値は独立した実装 (Python) で計算したもの。変わると既存の一時キーが無効になる。
        assert_eq!(line_key(""), "~d0bfc1");
        assert_eq!(line_key("- [ ] 牛乳を買う"), "~387609");
    }

    fn docs(inbox: &[&str], todo: &[&str]) -> (Document, Document) {
        let parse = |lines: &[&str]| {
            let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            Document::parse(
                &lines,
                &crate::sections::SectionNames::default(),
                chrono::NaiveDate::MIN,
            )
        };
        (parse(inbox), parse(todo))
    }

    fn find_in(inbox: &Document, todo: &Document, key: &str) -> Result<Location, FindError> {
        find(&[(FileKind::Inbox, inbox), (FileKind::Todo, todo)], key)
    }

    #[test]
    fn tasks_are_found_by_id_prefix_or_temporary_key() {
        let (inbox, todo) = docs(
            &["# Inbox", "- [ ] IDの無いタスク"],
            &[
                "# Todo",
                "## Today",
                "- [ ] 今日 id:a1b2c3",
                "メモ",
                "- [ ] 別 id:ffffff",
            ],
        );
        let at = |file, section, index| Location { file, section, index };
        assert_eq!(find_in(&inbox, &todo, "a1b2c3"), Ok(at(FileKind::Todo, Some(0), 0)));
        assert_eq!(find_in(&inbox, &todo, "ffff"), Ok(at(FileKind::Todo, Some(0), 2)));
        assert_eq!(
            find_in(&inbox, &todo, &line_key("- [ ] IDの無いタスク")),
            Ok(at(FileKind::Inbox, None, 0))
        );
    }

    #[test]
    fn keys_matching_no_task_or_several_tasks_are_errors() {
        let (inbox, todo) = docs(
            &["# Inbox", "- [ ] 受信 id:abc111", "- [ ] 同じ行", "- [ ] 同じ行"],
            &["# Todo", "## Today", "- [ ] 今日 id:abc111", "- [ ] 次 id:abc122"],
        );
        assert!(
            matches!(find_in(&inbox, &todo, "abc111"), Err(FindError::Ambiguous { .. })),
            "across files"
        );
        assert!(matches!(
            find_in(&inbox, &todo, "abc1"),
            Err(FindError::Ambiguous { .. })
        ));
        assert!(matches!(
            find_in(&inbox, &todo, &line_key("- [ ] 同じ行")),
            Err(FindError::Ambiguous { .. })
        ));
        assert_eq!(find_in(&inbox, &todo, "9999"), Err(FindError::NotFound("9999".into())));
        let changed = line_key("- [ ] 受信");
        assert_eq!(
            find_in(&inbox, &todo, &changed),
            Err(FindError::LineChanged(changed.clone()))
        );
    }

    #[test]
    fn prefixes_shorter_than_the_minimum_match_nothing() {
        let (inbox, todo) = docs(
            &["# Inbox", "- [ ] 手書き id:zz", "- [ ] IDなし"],
            &["# Todo", "- [ ] 唯一 id:a1b2c3", "- [ ] 日本語 id:日本語の仕事"],
        );
        for key in ["", "a", "a1b", "z", "日本語"] {
            assert_eq!(
                find_in(&inbox, &todo, key),
                Err(FindError::TooShort(key.into())),
                "{key:?}"
            );
        }
        assert!(find_in(&inbox, &todo, "a1b2").is_ok());
        assert!(
            find_in(&inbox, &todo, "日本語の").is_ok(),
            "the minimum counts characters, not bytes"
        );
        assert!(
            find_in(&inbox, &todo, "zz").is_ok(),
            "a whole hand-written ID is never too short"
        );
        assert_eq!(
            find_in(&inbox, &todo, "~ab"),
            Err(FindError::LineChanged("~ab".into())),
            "temporary keys are not prefixes"
        );
    }

    #[test]
    fn a_whole_id_wins_over_ids_that_merely_start_with_it() {
        let (inbox, todo) = docs(
            &["# Inbox", "- [ ] 手書き id:abcd"],
            &["# Todo", "- [ ] 発行 id:abcdef"],
        );
        assert_eq!(
            find_in(&inbox, &todo, "abcd"),
            Ok(Location {
                file: FileKind::Inbox,
                section: None,
                index: 0
            })
        );
    }

    #[test]
    fn a_temporary_key_never_matches_a_task_that_has_an_id() {
        let line = "- [ ] 今日 id:a1b2c3";
        let (inbox, todo) = docs(&["# Inbox"], &["# Todo", line]);
        assert!(find_in(&inbox, &todo, &line_key(line)).is_err());
    }

    #[test]
    fn line_key_distinguishes_trailing_whitespace() {
        assert_ne!(line_key("- [ ] 牛乳を買う"), line_key("- [ ] 牛乳を買う "));
    }
}
