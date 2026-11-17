//! Queue ビューの行。見出しと並びは Neovim 版の Queue と同じ (core の `view::due_queue`・`wait_queue`)。

use std::rc::Rc;

use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::key::task_key;
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::FileKind;
use gtodo_core::view::{Entry, due_queue, entries, wait_queue};

use super::app::Row;
use crate::commands::queue::date_label;

/// これより先の日付は薄く出す (Neovim 版の `FAR_FUTURE_DAYS`)。
const FAR_FUTURE_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    Overdue,
    Today,
    Upcoming,
    /// [`FAR_FUTURE_DAYS`] より先。
    Far,
    Invalid,
    Person,
}

/// Queue のタスクの行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub row: Row,
    pub file: FileKind,
    /// todo.md の管理対象のセクションなら、そのキー。
    pub section: Option<SectionKey>,
    /// 右端に出す、どこにあるか (Inbox・セクション名)。
    pub place: String,
    /// 本文の後ろに付ける (`(3日超過)`・`(due:someday)`)。
    pub suffix: String,
    pub overdue: bool,
    pub far: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Blank,
    Heading(String, Heading),
    Item(Box<Item>),
    /// 対象のタスクが無いときの一文。
    Empty(&'static str),
}

/// Queue の行を作る。`wait` が真なら待ち先別、偽なら期日別。
pub fn build(wait: bool, inbox: &Document, todo: &Document, names: &SectionNames, today: NaiveDate) -> Vec<Line> {
    let all = || entries(FileKind::Inbox, inbox).chain(entries(FileKind::Todo, todo));
    let item = |e: &Entry, suffix: String, overdue: bool, far: bool| {
        let (section, place) = locate(e.file, e.section, names);
        Line::Item(Box::new(Item {
            row: Row {
                key: task_key(e.task(), &e.item.line),
                task: Rc::new(e.task().clone()),
            },
            file: e.file,
            section,
            place,
            suffix,
            overdue,
            far,
        }))
    };
    let mut lines = Vec::new();
    let heading = |lines: &mut Vec<Line>, text: String, kind| {
        if !lines.is_empty() {
            lines.push(Line::Blank);
        }
        lines.push(Line::Heading(text, kind));
    };

    if wait {
        let groups = wait_queue(all());
        if groups.is_empty() {
            return vec![Line::Empty("誰かの作業を待っているタスクはありません")];
        }
        for (person, es) in &groups {
            heading(&mut lines, format!("{person} 待ち"), Heading::Person);
            lines.extend(es.iter().map(|e| item(e, String::new(), false, false)));
        }
        return lines;
    }

    let q = due_queue(all(), today);
    if q.is_empty() {
        return vec![Line::Empty("期限付きタスクはありません")];
    }
    if !q.overdue.is_empty() {
        heading(&mut lines, "期限切れ".to_string(), Heading::Overdue);
        lines.extend(
            q.overdue
                .iter()
                .map(|(e, days)| item(e, format!(" ({days}日超過)"), true, false)),
        );
    }
    for (date, es) in &q.upcoming {
        let days = (*date - today).num_days();
        let far = days > FAR_FUTURE_DAYS;
        let kind = match days {
            0 => Heading::Today,
            _ if far => Heading::Far,
            _ => Heading::Upcoming,
        };
        heading(&mut lines, date_label(*date, today), kind);
        lines.extend(es.iter().map(|e| item(e, String::new(), false, far)));
    }
    if !q.invalid.is_empty() {
        heading(&mut lines, "期日を解釈できない".to_string(), Heading::Invalid);
        lines.extend(q.invalid.iter().map(|e| {
            let due = e.task().due.as_deref().unwrap_or_default();
            item(e, format!(" (due:{due})"), false, false)
        }));
    }
    lines
}

/// タスクがどこにあるか。todo.md の管理対象のセクションならそのキーと、右端に出す名前
/// (Inbox・セクション名・Done、見出しの無いところならファイル名)。Queue と検索で共有する。
pub fn locate(file: FileKind, section: Option<&str>, names: &SectionNames) -> (Option<SectionKey>, String) {
    let key = match file {
        FileKind::Todo => section.and_then(|s| names.key_of(s)),
        _ => None,
    };
    let place = match (file, section) {
        (FileKind::Inbox, None) => "Inbox".to_string(),
        (FileKind::Done, _) => "Done".to_string(),
        (_, Some(name)) => name.to_string(),
        (file, None) => file.file_name().to_string(),
    };
    (key, place)
}

/// 期日が来ている (今日か過去の) inbox.md の未完了のタスクの数。
pub fn due_in_inbox(inbox: &Document, today: NaiveDate) -> usize {
    let q = due_queue(entries(FileKind::Inbox, inbox), today);
    q.overdue.len()
        + q.upcoming
            .iter()
            .filter(|(date, _)| *date <= today)
            .map(|(_, es)| es.len())
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
    }

    fn doc(text: &str) -> Document {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        Document::parse(&lines, &SectionNames::default(), today())
    }

    fn render(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                Line::Blank => String::new(),
                Line::Heading(text, kind) => format!("# {text} {kind:?}"),
                Line::Item(i) => format!(
                    "{} {}{} [{}]{}",
                    if i.overdue { "⚠" } else { "▶" },
                    i.row.task.content,
                    i.suffix,
                    i.place,
                    if i.far { " far" } else { "" }
                ),
                Line::Empty(text) => format!("({text})"),
            })
            .collect()
    }

    #[test]
    fn due_groups_follow_the_neovim_queue() {
        let inbox = doc("# Inbox\n\n- [ ] 受信 due:2026-11-17\n- [ ] 期日なし\n");
        let todo = doc(
            "# Todo\n\n## Today\n\n- [ ] 遅れ due:2026-11-14\n- [x] 済み due:2026-11-01\n\n## Next\n\n- [ ] 来月 due:2026-12-18\n- [ ] 変 due:someday\n\n## 買い物\n\n- [ ] 明日 due:2026-11-18\n\n## Waiting\n\n## Someday\n",
        );
        let names = SectionNames::default();
        assert_eq!(
            render(&build(false, &inbox, &todo, &names, today())),
            [
                "# 期限切れ Overdue",
                "⚠ 遅れ (3日超過) [Today]",
                "",
                "# 今日 (11/17 火) Today",
                "▶ 受信 [Inbox]",
                "",
                "# 明日 (11/18 水) Upcoming",
                "▶ 明日 [買い物]",
                "",
                "# 12/18 金 (31日後) Far",
                "▶ 来月 [Next] far",
                "",
                "# 期日を解釈できない Invalid",
                "▶ 変 (due:someday) [Next]",
            ]
        );
        let Line::Item(item) = &build(false, &inbox, &todo, &names, today())[1] else {
            panic!("not an item");
        };
        assert_eq!((item.file, item.section), (FileKind::Todo, Some(SectionKey::Today)));
        assert_eq!(due_in_inbox(&inbox, today()), 1);
    }

    #[test]
    fn wait_groups_are_by_person() {
        let inbox = doc("# Inbox\n");
        let todo = doc(
            "# Todo\n\n## Today\n\n## Next\n\n## Waiting\n\n- [ ] 見積もり wait:田中\n- [ ] 返事 wait:bob\n- [x] 済み wait:bob\n\n## Someday\n",
        );
        let names = SectionNames::default();
        assert_eq!(
            render(&build(true, &inbox, &todo, &names, today())),
            [
                "# bob 待ち Person",
                "▶ 返事 [Waiting]",
                "",
                "# 田中 待ち Person",
                "▶ 見積もり [Waiting]"
            ]
        );
    }

    #[test]
    fn empty_queues_say_so() {
        let empty = doc("# Todo\n");
        let names = SectionNames::default();
        assert_eq!(
            build(false, &empty, &empty, &names, today()),
            [Line::Empty("期限付きタスクはありません")]
        );
        assert_eq!(
            build(true, &empty, &empty, &names, today()),
            [Line::Empty("誰かの作業を待っているタスクはありません")]
        );
    }
}
