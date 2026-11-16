use std::fmt::Write;

use anyhow::Result;
use chrono::NaiveDate;
use clap::ValueEnum;
use gtodo_core::input;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Destination};
use gtodo_core::sections::SectionKey;
use gtodo_core::task::Task;

use crate::workspace::Workspace;
use crate::writing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum To {
    Inbox,
    Today,
    Next,
    Waiting,
    Someday,
}

impl To {
    fn destination(self) -> Destination {
        match self {
            To::Inbox => Destination::Inbox,
            To::Today => Destination::Section(SectionKey::Today),
            To::Next => Destination::Section(SectionKey::Next),
            To::Waiting => Destination::Section(SectionKey::Waiting),
            To::Someday => Destination::Section(SectionKey::Someday),
        }
    }
}

/// 本文以外で指定する項目。指定されたものは本文中のタグより優先する。
#[derive(Debug, Default)]
pub struct Fields {
    pub due: Option<String>,
    pub project: Option<String>,
    pub context: Option<String>,
    pub priority: Option<char>,
}

pub fn run(
    ws: &Workspace,
    text: &str,
    to: To,
    fields: Fields,
    warn: &mut impl FnMut(String),
    out: &mut String,
) -> Result<()> {
    // ロックを取る前に検証し、受け付けられない入力では何も書かない。
    let task = build(text, fields, ws.today)?;
    let destination = to.destination();
    let (id, dues) = writing::with_lock(ws, destination == Destination::Inbox, warn, || {
        let id = ops::add_task(&ws.data_dir, &ws.names, ws.today, task, destination)?;
        // 期日がすでに来ていれば、Neovim 版と同じくこの場で Today へ移し、次の日次の整理まで
        // inbox に残さない。ここでの失敗は追加の成否と分けて返す。
        let dues = match destination {
            Destination::Inbox => Some(check_dues_and_sort(ws)),
            Destination::Section(_) => None,
        };
        Ok((id, dues))
    })?;

    match destination {
        Destination::Inbox => writeln!(out, "added {id} to inbox.md")?,
        Destination::Section(key) => writeln!(out, "added {id} to {}", ws.names.name(key))?,
    }
    match dues {
        Some(Ok(dues)) => {
            let mut notes = Vec::new();
            writing::report_dues(ws, &dues, warn, |msg| notes.push(msg));
            for note in notes {
                writeln!(out, "{note}")?;
            }
        }
        // 追加は済んでいるため、失敗として返すと実行し直されてタスクが二重になる。
        Some(Err(e)) => warn(format!("the task was added, but checking due dates failed: {e:#}")),
        None => {}
    }
    Ok(())
}

/// inbox.md へ加えた後の期日のチェック (TUI のフォームも使う)。
pub fn check_dues_and_sort(ws: &Workspace) -> Result<DueOutcome> {
    let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
    if dues.todo_changed {
        ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
    }
    Ok(dues)
}

/// 本文をタスク行と同じ文法で読み、指定された項目で上書きする (TUI のフォームと同じ規則)。
fn build(text: &str, fields: Fields, today: NaiveDate) -> Result<Task> {
    let fields = input::Fields {
        due: fields.due,
        project: fields.project,
        context: fields.context,
        priority: fields.priority.map(String::from),
        wait: None,
    };
    Ok(input::new_task(text, &fields, today)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_passed_as_fields() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 3).unwrap();
        let fields = Fields {
            due: Some("+1w".into()),
            project: Some("+work".into()),
            context: Some("@office".into()),
            priority: Some('B'),
        };
        let task = build("(A) 資料 +home @15 due:today", fields, today).unwrap();
        assert_eq!(task.content, "資料");
        assert_eq!(task.priority, Some('B'));
        assert_eq!(task.project.as_deref(), Some("work"));
        assert_eq!(task.context.as_deref(), Some("office"));
        assert_eq!(task.due.as_deref(), Some("2026-02-10"));
        assert_eq!(task.created.as_deref(), Some("2026-02-03"));
    }
}
