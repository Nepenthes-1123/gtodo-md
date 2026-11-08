//! 期日が来たタスクの Today への昇格 (Lua 版 `logic/due.lua` の `check_dues`)。

use chrono::NaiveDate;

use crate::document::{Document, Item, Section};
use crate::sections::{SectionKey, SectionNames};
use crate::task::Status;

/// [`promote_due`] の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DueOutcome {
    /// Today へ移したタスクの数。
    pub moved: usize,
    /// 期日が来たが、`auto_move_inbox_to_today` が無効なため inbox に残したタスク。
    pub inbox_due: Vec<DueTask>,
    pub inbox_changed: bool,
    pub todo_changed: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DueTask {
    pub content: String,
    pub due: String,
}

/// 期日が今日以前の未完了タスクを、inbox と todo の Next / Someday から Today の末尾へ移す。
///
/// inbox のタスクは `auto_move_inbox` が無効なら移さず [`DueOutcome::inbox_due`] で知らせる。
/// Waiting は対象にしない。他者の応答待ちであり、期日が来ても自分が着手できるようになった
/// わけではないため。Next / Someday から移すタスクは `wait:` を外す (Lua 版と同じく、
/// inbox から移すタスクは外さない)。
pub fn promote_due(
    inbox: &mut Document,
    todo: &mut Document,
    names: &SectionNames,
    today: NaiveDate,
    auto_move_inbox: bool,
) -> DueOutcome {
    let today = super::ymd(today);
    let mut outcome = DueOutcome::default();

    let mut to_move = Vec::new();
    for item in std::mem::take(&mut inbox.preamble) {
        match due_date(&item, &today) {
            Some(_) if auto_move_inbox => to_move.push(item),
            Some(due) => {
                if let Item::Task(t) = &item {
                    outcome.inbox_due.push(DueTask {
                        content: t.task.content.clone(),
                        due,
                    });
                }
                inbox.preamble.push(item);
            }
            None => inbox.preamble.push(item),
        }
    }
    outcome.inbox_changed = !to_move.is_empty();

    let today_name = names.name(SectionKey::Today);
    if !todo.sections.iter().any(|s| s.name == today_name) {
        todo.sections.insert(
            0,
            Section {
                name: today_name.to_string(),
                items: Vec::new(),
            },
        );
    }

    for key in [SectionKey::Next, SectionKey::Someday] {
        let Some(section) = todo.sections.iter_mut().find(|s| s.name == names.name(key)) else {
            continue;
        };
        for mut item in std::mem::take(&mut section.items) {
            if due_date(&item, &today).is_some() {
                if let Item::Task(t) = &mut item {
                    t.task.wait = None;
                }
                to_move.push(item);
            } else {
                section.items.push(item);
            }
        }
    }

    outcome.moved = to_move.len();
    outcome.todo_changed = !to_move.is_empty();
    let today_section = todo.sections.iter_mut().find(|s| s.name == today_name).unwrap();
    today_section.items.extend(to_move);
    outcome
}

/// 期日が来た未完了タスクなら、その期日。
fn due_date(item: &Item, today: &str) -> Option<String> {
    let Item::Task(t) = item else { return None };
    let due = t.task.due.as_deref().filter(|d| !d.is_empty())?;
    (t.task.status != Status::Done && due <= today).then(|| due.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Document {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        Document::parse(&lines, &SectionNames::default(), today())
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()
    }

    fn contents(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Task(t) => match &t.task.wait {
                    Some(wait) => format!("{} wait:{wait}", t.task.content),
                    None => t.task.content.clone(),
                },
                Item::Text(line) => line.clone(),
            })
            .collect()
    }

    #[test]
    fn due_tasks_move_to_the_end_of_today() {
        let mut inbox = parse(&[
            "# Inbox",
            "- [ ] 受信 due:2026-01-31 wait:手元",
            "- [ ] 未来 due:2026-02-01",
        ]);
        let mut todo = parse(&[
            "## Today",
            "- [ ] 既存",
            "## Next",
            "- [ ] 次 due:2026-01-30 wait:誰か",
            "## Waiting",
            "- [ ] 待ち due:2026-01-01",
            "## Someday",
            "- [x] 完了済み due:2026-01-01",
            "- [ ] いつか due:2026-01-31",
        ]);
        let outcome = promote_due(&mut inbox, &mut todo, &SectionNames::default(), today(), true);

        assert_eq!(outcome.moved, 3);
        assert!(outcome.inbox_changed && outcome.todo_changed);
        assert_eq!(contents(&inbox.preamble), ["未来"]);
        assert_eq!(
            contents(&todo.sections[0].items),
            ["既存", "受信 wait:手元", "次", "いつか"]
        );
        assert_eq!(contents(&todo.sections[1].items), Vec::<String>::new());
        assert_eq!(contents(&todo.sections[2].items), ["待ち"]);
        assert_eq!(contents(&todo.sections[3].items), ["完了済み"]);
    }

    #[test]
    fn inbox_tasks_stay_when_auto_move_is_disabled() {
        let mut inbox = parse(&["# Inbox", "- [ ] 受信 due:2026-01-31"]);
        let mut todo = parse(&["## Today"]);
        let outcome = promote_due(&mut inbox, &mut todo, &SectionNames::default(), today(), false);

        assert_eq!(
            outcome,
            DueOutcome {
                moved: 0,
                inbox_due: vec![DueTask {
                    content: "受信".into(),
                    due: "2026-01-31".into()
                }],
                inbox_changed: false,
                todo_changed: false,
            }
        );
        assert_eq!(inbox.preamble.len(), 1);
    }

    #[test]
    fn today_is_created_at_the_top_when_missing() {
        let mut inbox = parse(&["# Inbox", "- [ ] 受信 due:2026-01-31"]);
        let mut todo = parse(&["## Next", "- [ ] 次"]);
        promote_due(&mut inbox, &mut todo, &SectionNames::default(), today(), true);
        assert_eq!(todo.sections[0].name, "Today");
        assert_eq!(contents(&todo.sections[0].items), ["受信"]);
    }
}
