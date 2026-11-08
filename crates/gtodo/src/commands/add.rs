use std::fmt::Write;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use clap::ValueEnum;
use gtodo_core::date;
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

pub fn run(ws: &Workspace, text: &str, to: To, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    // ロックを取る前に検証し、受け付けられない入力では何も書かない。
    let task = build(text, ws.today)?;
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

fn check_dues_and_sort(ws: &Workspace) -> Result<DueOutcome> {
    let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
    if dues.todo_changed {
        ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
    }
    Ok(dues)
}

/// 本文をタスク行と同じ文法で読む。
fn build(text: &str, today: NaiveDate) -> Result<Task> {
    if text.contains(['\n', '\r']) {
        bail!("the task text must be a single line");
    }
    let Some(mut task) = Task::parse_at(&format!("- [ ] {text}"), today) else {
        bail!("the task text must not be empty");
    };
    // ID や完了・キャンセルの記録は書き込むコマンドが付けるもので、利用者が指定すると
    // 他のタスクと ID が重なったり、完了していないのに完了の記録を持ったりする。
    for (tag, value) in [
        ("id", &task.id),
        ("completed_at", &task.completed_at),
        ("done", &task.done),
        ("cancelled", &task.cancelled),
        ("from", &task.from),
    ] {
        if value.is_some() {
            bail!("{tag}: cannot be set when adding a task");
        }
    }

    if let Some(due) = &task.due {
        // 読めなかった期日は書かれたまま残り、日付の形の表記は範囲を確かめずに桁だけ揃えられる
        // (`2026-02-30` も通る) ため、正規化した形が実在する日付かで見分ける。
        if date::parse_due_date(due, today).as_ref() != Some(due) || !is_date(due) {
            return Err(invalid_due(due));
        }
    }
    if let Some(created) = &task.created {
        if !is_date(created) {
            bail!("invalid created date {created:?} (use YYYY-MM-DD)");
        }
    }
    if task.content.is_empty() {
        bail!("the task text must not be empty");
    }
    task.created.get_or_insert_with(|| today.format("%Y-%m-%d").to_string());

    // 書いた行を読み直して同じタスクにならない値 (空白を含むプロジェクト名や小文字の優先度など) は、
    // 次に読んだときに本文へ混ざって壊れるため受け付けない。文法は task.rs だけが知っている。
    let mut written = Task {
        id: Some("000000".into()),
        ..task.clone()
    };
    let reread = Task::parse_at(&written.serialize(), today);
    if reread.as_ref().map(|t| Task { id: None, ..t.clone() }) != Some(task.clone()) {
        bail!("the project, context, priority or wait: value cannot be written in a task line");
    }
    Ok(task)
}

fn is_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn invalid_due(due: &str) -> anyhow::Error {
    anyhow::anyhow!("invalid due date {due:?} (e.g. today, tomorrow, +3d, fri, 2026-07-02)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    fn build_text(text: &str) -> Result<Task> {
        build(text, today())
    }

    #[test]
    fn tags_in_the_text_are_read_like_a_task_line() {
        let task = build_text("(A) 買い物 +home @15 due:tomorrow wait:bob").unwrap();
        assert_eq!(task.content, "買い物");
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.project.as_deref(), Some("home"));
        assert_eq!(task.context.as_deref(), Some("15"));
        assert_eq!(task.due.as_deref(), Some("2026-02-04"));
        assert_eq!(task.wait.as_deref(), Some("bob"));
        assert_eq!(task.created.as_deref(), Some("2026-02-03"));
        assert_eq!(task.id, None);

        let task = build_text("x created:2026-01-01").unwrap();
        assert_eq!(
            task.created.as_deref(),
            Some("2026-01-01"),
            "a given creation date is kept"
        );
    }

    #[test]
    fn records_written_by_commands_cannot_be_given() {
        for text in [
            "x id:abcdef",
            "x completed_at:2026-01-01",
            "x done:2026-01-01",
            "x cancelled:2026-01-01",
            "x from:today",
        ] {
            assert!(build_text(text).is_err(), "{text}");
        }
    }

    #[test]
    fn values_that_do_not_survive_being_read_back_are_rejected() {
        assert!(build_text("x due:someday").is_err());
        assert!(build_text("x +a\u{b}").is_err());
        assert!(build_text("x due:2026-99-99").is_err());
        assert!(build_text("x created:2026-13-45").is_err());
        assert!(build_text("+only-a-tag").is_err());
        assert!(build_text("two\nlines").is_err());
    }
}
