//! 読み取り専用のビュー。CLI と TUI で共有する。

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::document::{Document, TaskItem};
use crate::store::FileKind;
use crate::task::{Status, Task};

/// どのファイル・セクションにあるタスクか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub file: FileKind,
    /// preamble (最初の `##` より前) にあるタスクは `None`。
    pub section: Option<&'a str>,
    pub item: &'a TaskItem,
}

impl<'a> Entry<'a> {
    pub fn task(&self) -> &'a Task {
        &self.item.task
    }
}

/// 文書内のすべてのタスクをファイル内の順で列挙する。
pub fn entries(file: FileKind, doc: &Document) -> impl Iterator<Item = Entry<'_>> {
    doc.tasks().map(move |(section, item)| Entry { file, section, item })
}

/// Queue の対象ファイル (Lua 版と同じ)。
pub const QUEUE_FILES: [FileKind; 2] = [FileKind::Inbox, FileKind::Todo];

/// 期日別の Queue。対象は期日のある未完了タスク。
#[derive(Debug, Default)]
pub struct DueQueue<'a> {
    /// 期日を過ぎたタスクと超過日数。期日の早い順。
    pub overdue: Vec<(Entry<'a>, i64)>,
    /// 今日以降の期日ごとのタスク。日付の早い順、同じ日付の中は入力の順。
    pub upcoming: Vec<(NaiveDate, Vec<Entry<'a>>)>,
    /// 期日を日付として解釈できないタスク (例: `due:someday`・存在しない日付 `due:2026-02-30`)。
    ///
    /// Lua 版は前者があると例外で Queue 自体を開けなくなり、後者は補正した別の日付で日数や期限切れを判定する。
    /// どちらも利用者が書いた期日とは違う結果になるため、落とさず別のグループとして見せる。
    pub invalid: Vec<Entry<'a>>,
}

impl DueQueue<'_> {
    pub fn is_empty(&self) -> bool {
        self.overdue.is_empty() && self.upcoming.is_empty() && self.invalid.is_empty()
    }
}

pub fn due_queue<'a>(entries: impl IntoIterator<Item = Entry<'a>>, today: NaiveDate) -> DueQueue<'a> {
    let mut queue = DueQueue::default();
    let mut by_date: BTreeMap<NaiveDate, Vec<Entry<'a>>> = BTreeMap::new();
    for entry in entries {
        let task = entry.task();
        if task.status == Status::Done {
            continue;
        }
        let Some(due) = task.due.as_deref().filter(|d| !d.is_empty()) else {
            continue;
        };
        match NaiveDate::parse_from_str(due, "%Y-%m-%d") {
            Ok(date) if date < today => queue.overdue.push((entry, (today - date).num_days())),
            Ok(date) => by_date.entry(date).or_default().push(entry),
            Err(_) => queue.invalid.push(entry),
        }
    }
    queue.overdue.sort_by_key(|(_, days)| std::cmp::Reverse(*days));
    queue.upcoming = by_date.into_iter().collect();
    queue
}

/// 待ち先 (`wait:`) ごとの Queue。対象は待ち先のある未完了タスク。待ち先の名前順。
pub fn wait_queue<'a>(entries: impl IntoIterator<Item = Entry<'a>>) -> Vec<(&'a str, Vec<Entry<'a>>)> {
    let mut by_person: BTreeMap<&'a str, Vec<Entry<'a>>> = BTreeMap::new();
    for entry in entries {
        let task = entry.task();
        if task.status == Status::Done {
            continue;
        }
        if let Some(person) = task.wait.as_deref().filter(|w| !w.is_empty()) {
            by_person.entry(person).or_default().push(entry);
        }
    }
    by_person.into_iter().collect()
}

/// 検索の対象ファイル (Lua 版と同じく、キャンセル済みは含めない)。
pub const SEARCH_FILES: [FileKind; 3] = [FileKind::Inbox, FileKind::Todo, FileKind::Done];

/// AND 条件の検索。条件の無い項目は絞り込まない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// `+project` の `project` 部分。
    pub projects: Vec<String>,
    /// `@context` の `context` 部分 (`@` を含まない)。
    pub contexts: Vec<String>,
    pub status: Option<Status>,
}

impl SearchQuery {
    pub fn matches(&self, task: &Task) -> bool {
        self.projects.iter().all(|p| task.project.as_deref() == Some(p))
            && self.contexts.iter().all(|c| task.context.as_deref() == Some(c))
            && self.status.is_none_or(|s| task.status == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn doc(text: &str) -> Document {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        Document::parse(&lines, &crate::sections::SectionNames::default(), date("2026-01-31"))
    }

    fn contents<'a>(entries: impl IntoIterator<Item = &'a Entry<'a>>) -> Vec<&'a str> {
        entries.into_iter().map(|e| e.task().content.as_str()).collect()
    }

    #[test]
    fn due_queue_groups_overdue_upcoming_and_invalid_and_skips_done_or_undated() {
        let d = doc("## Today
- [ ] 期限切れ1日 due:2026-01-30
- [ ] 期限切れ3日 due:2026-01-28
- [ ] 今日 due:2026-01-31
- [ ] 来月A due:2026-02-10
- [ ] 来月B due:2026-02-10
- [ ] 明日 due:2026-02-01
- [ ] いつか due:someday
- [ ] 存在しない日付 due:2026-13-45
- [x] 完了済み due:2026-01-01
- [ ] 期日なし");
        let q = due_queue(entries(FileKind::Todo, &d), date("2026-01-31"));

        assert_eq!(
            contents(q.overdue.iter().map(|(e, _)| e)),
            ["期限切れ3日", "期限切れ1日"]
        );
        assert_eq!(q.overdue.iter().map(|(_, d)| *d).collect::<Vec<_>>(), [3, 1]);
        let upcoming: Vec<_> = q.upcoming.iter().map(|(d, es)| (d.to_string(), contents(es))).collect();
        assert_eq!(
            upcoming,
            [
                ("2026-01-31".to_string(), vec!["今日"]),
                ("2026-02-01".to_string(), vec!["明日"]),
                ("2026-02-10".to_string(), vec!["来月A", "来月B"]),
            ]
        );
        assert_eq!(contents(&q.invalid), ["いつか", "存在しない日付"]);
    }

    #[test]
    fn wait_queue_groups_open_tasks_by_person_in_name_order() {
        let d = doc("## Waiting
- [ ] 2 wait:bob
- [ ] 1 wait:alice
- [ ] 3 wait:bob
- [x] 完了済み wait:alice
- [ ] 待ちなし");
        let q = wait_queue(entries(FileKind::Todo, &d));
        let grouped: Vec<_> = q.iter().map(|(p, es)| (*p, contents(es))).collect();
        assert_eq!(grouped, [("alice", vec!["1"]), ("bob", vec!["2", "3"])]);
    }

    #[test]
    fn search_query_requires_all_conditions() {
        let t = |line: &str| Task::parse(line).unwrap();
        let q = SearchQuery {
            projects: vec!["work".into()],
            contexts: vec!["office".into()],
            status: Some(Status::Open),
        };
        assert!(q.matches(&t("- [ ] a +work @office")));
        assert!(!q.matches(&t("- [x] a +work @office")));
        assert!(!q.matches(&t("- [ ] a +work @home")));
        assert!(!q.matches(&t("- [ ] a @office")));
        assert!(SearchQuery::default().matches(&t("- [x] 条件なしは全件")));
    }
}
