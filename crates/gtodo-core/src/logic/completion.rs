//! 完了したタスクの取り出し (Lua 版 `logic/completion.lua` の `move_completed_tasks`)。

use chrono::NaiveDate;

use crate::document::{Document, Item};
use crate::sections::{SectionKey, SectionNames};
use crate::task::{Status, Task};

/// [`take_completed`] の結果。
#[derive(Debug, Default)]
pub struct Completed {
    /// 完了月 (`YYYY-MM`、昇順) ごとのタスク。`done:` と `from:` を付け、`completed_at:` を外してある。
    pub months: Vec<(String, Vec<Task>)>,
    pub inbox_changed: bool,
    pub todo_changed: bool,
}

impl Completed {
    pub fn count(&self) -> usize {
        self.months.iter().map(|(_, tasks)| tasks.len()).sum()
    }
}

/// inbox と todo の管理対象セクション (Today / Next / Waiting / Someday) から完了したタスクを取り除いて返す。
///
/// 振り分ける月は、取り出した日ではなく `completed_at:` の月にする。取り出した日にすると、
/// 月をまたいで放置された完了タスクが実際に完了した月と違う見出しに記録される。
/// `completed_at:` が無ければ今日を完了日とし、日付として読めなければ今月へ寄せる。
pub fn take_completed(inbox: &mut Document, todo: &mut Document, names: &SectionNames, today: NaiveDate) -> Completed {
    let mut taken: Vec<(Task, String)> = Vec::new();

    let before = taken.len();
    take_done(&mut inbox.preamble, "inbox", &mut taken);
    let inbox_changed = taken.len() > before;

    let before = taken.len();
    for key in SectionKey::ALL {
        let name = names.name(key);
        if let Some(section) = todo.sections.iter_mut().find(|s| s.name == name) {
            // Lua 版の `string.lower` と同じく ASCII だけを小文字にする。
            take_done(&mut section.items, &name.to_ascii_lowercase(), &mut taken);
        }
    }
    let todo_changed = taken.len() > before;

    let today_ymd = super::ymd(today);
    let mut months: Vec<(String, Vec<Task>)> = Vec::new();
    for (mut task, from) in taken {
        let completed = task
            .completed_at
            .take()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| today_ymd.clone());
        let month = year_month(&completed).unwrap_or_else(|| today.format("%Y-%m").to_string());
        task.done = Some(completed);
        task.from = Some(from);
        match months.iter_mut().find(|(m, _)| *m == month) {
            Some((_, tasks)) => tasks.push(task),
            None => months.push((month, vec![task])),
        }
    }
    months.sort_by(|a, b| a.0.cmp(&b.0));

    Completed {
        months,
        inbox_changed,
        todo_changed,
    }
}

fn take_done(items: &mut Vec<Item>, from: &str, taken: &mut Vec<(Task, String)>) {
    for item in std::mem::take(items) {
        match item {
            Item::Task(t) if t.task.status == Status::Done => taken.push((t.task, from.to_string())),
            other => items.push(other),
        }
    }
}

/// Lua のパターン `^(%d%d%d%d%-%d%d)`。
fn year_month(date: &str) -> Option<String> {
    let ym = date.get(..7)?;
    let b = ym.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    (digits(0..4) && b[4] == b'-' && digits(5..7)).then(|| ym.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Document {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        Document::parse(&lines, &SectionNames::default(), today())
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    #[test]
    fn completed_tasks_are_grouped_by_the_month_they_were_completed() {
        let mut inbox = parse(&["# Inbox", "- [x] 受信 completed_at:2026-02-01", "- [ ] 未完了"]);
        let mut todo = parse(&[
            "## Today",
            "- [x] 先月 completed_at:2026-01-30",
            "## Other",
            "- [x] 管理対象外のセクション",
            "## Someday",
            "- [x] 日付なし",
        ]);
        let completed = take_completed(&mut inbox, &mut todo, &SectionNames::default(), today());

        assert!(completed.inbox_changed && completed.todo_changed);
        assert_eq!(completed.count(), 3);
        let summary: Vec<(String, Vec<String>)> = completed
            .months
            .iter()
            .map(|(m, tasks)| {
                let tasks = tasks
                    .iter()
                    .map(|t| {
                        format!(
                            "{} done:{} from:{} completed_at:{:?}",
                            t.content,
                            t.done.as_deref().unwrap(),
                            t.from.as_deref().unwrap(),
                            t.completed_at
                        )
                    })
                    .collect();
                (m.clone(), tasks)
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "2026-01".to_string(),
                    vec!["先月 done:2026-01-30 from:today completed_at:None".to_string()]
                ),
                (
                    "2026-02".to_string(),
                    vec![
                        "受信 done:2026-02-01 from:inbox completed_at:None".to_string(),
                        "日付なし done:2026-02-03 from:someday completed_at:None".to_string(),
                    ]
                ),
            ]
        );
        assert_eq!(inbox.preamble.len(), 1);
        assert_eq!(todo.sections[1].items.len(), 1, "管理対象外のセクションには触れない");
    }

    /// タスク行の文法では `completed_at:` は日付の形に限られるため、手で組んだタスクでだけ起きる。
    #[test]
    fn a_completion_date_that_is_not_a_date_goes_to_this_month() {
        let mut todo = parse(&["## Today"]);
        let mut task = Task::parse("- [x] 読めない日付").unwrap();
        task.completed_at = Some("昨日".into());
        todo.sections[0]
            .items
            .push(Item::Task(Box::new(crate::document::TaskItem {
                task,
                line: String::new(),
                lnum: 0,
            })));
        let completed = take_completed(&mut parse(&[]), &mut todo, &SectionNames::default(), today());
        assert_eq!(completed.months[0].0, "2026-02");
        assert_eq!(completed.months[0].1[0].done.as_deref(), Some("昨日"));
    }

    #[test]
    fn nothing_changes_without_completed_tasks() {
        let mut inbox = parse(&["# Inbox", "- [ ] A"]);
        let mut todo = parse(&["## Today", "- [ ] B"]);
        let completed = take_completed(&mut inbox, &mut todo, &SectionNames::default(), today());
        assert_eq!(completed.count(), 0);
        assert!(!completed.inbox_changed && !completed.todo_changed);
    }
}
