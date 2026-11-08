//! セクション内のタスクの並び順 (Lua 版 `logic/sort.lua`)。

use std::cmp::Ordering;

use crate::document::{Document, Item, TaskItem};
use crate::task::Status;

/// すべての `##` セクションのタスクを並べ替える。`##` より前 (preamble) は並べ替えない (Lua 版と同じ)。
pub fn sort_sections(doc: &mut Document) {
    for section in &mut doc.sections {
        sort_items(&mut section.items);
    }
}

/// 未完了が先、期日のあるものが先 (期日の早い順)、優先度の高い順、元の順。
///
/// タスク以外の行 (`### 見出し` やメモ) は並べ替えの境界にし、元の位置に残す。
/// 境界をまたいで並べ替えると、タスクが元いた見出しの外へ出てしまう。
pub fn sort_items(items: &mut [Item]) {
    let mut start = 0;
    while start < items.len() {
        let len = items[start..]
            .iter()
            .position(|item| !matches!(item, Item::Task(_)))
            .unwrap_or(items.len() - start);
        // sort_by は安定ソートなので、比較で決まらない場合は元の順が保たれる。
        items[start..start + len].sort_by(|a, b| match (a, b) {
            (Item::Task(a), Item::Task(b)) => compare(a, b),
            _ => unreachable!("only tasks are in a run"),
        });
        start += len + 1;
    }
}

fn compare(a: &TaskItem, b: &TaskItem) -> Ordering {
    let (a, b) = (&a.task, &b.task);
    let done = |s: Status| s == Status::Done;
    // 優先度の無いタスクは `Z` と同じ扱い (Lua 版と同じ)。
    let priority = |p: Option<char>| p.unwrap_or('Z');

    done(a.status)
        .cmp(&done(b.status))
        .then_with(|| match (due(&a.due), due(&b.due)) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| priority(a.priority).cmp(&priority(b.priority)))
}

fn due(d: &Option<String>) -> Option<&str> {
    d.as_deref().filter(|d| !d.is_empty())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::sections::SectionNames;

    fn sorted(lines: &[&str]) -> Vec<String> {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let mut doc = Document::parse(
            &lines,
            &SectionNames::default(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );
        sort_sections(&mut doc);
        doc.sections[0]
            .items
            .iter()
            .map(|item| match item {
                Item::Task(t) => t.task.content.clone(),
                Item::Text(line) => line.clone(),
            })
            .collect()
    }

    #[test]
    fn open_tasks_come_first_then_by_due_then_by_priority() {
        let order = sorted(&[
            "## Today",
            "- [x] 完了",
            "- [ ] 期日なし",
            "- [ ] (B) 期日なし B",
            "- [ ] 遅い期日 due:2026-03-01",
            "- [ ] 早い期日 due:2026-02-01",
            "- [ ] (A) 早い期日 A due:2026-02-01",
        ]);
        assert_eq!(
            order,
            ["早い期日 A", "早い期日", "遅い期日", "期日なし B", "期日なし", "完了"]
        );
    }

    #[test]
    fn equal_tasks_keep_their_order() {
        let order = sorted(&["## Today", "- [ ] 1", "- [ ] 2", "- [ ] 3"]);
        assert_eq!(order, ["1", "2", "3"]);
    }

    #[test]
    fn tasks_do_not_cross_other_lines() {
        let order = sorted(&[
            "## Today",
            "- [ ] 後 due:2026-03-01",
            "### 見出し",
            "- [ ] 先 due:2026-02-01",
        ]);
        assert_eq!(order, ["後", "### 見出し", "先"]);
    }
}
