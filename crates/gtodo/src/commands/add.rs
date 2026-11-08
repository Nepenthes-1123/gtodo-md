use std::fmt::Write;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use clap::ValueEnum;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Destination};
use gtodo_core::sections::SectionKey;
use gtodo_core::task::Task;

use crate::commands::task_input;
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

fn check_dues_and_sort(ws: &Workspace) -> Result<DueOutcome> {
    let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
    if dues.todo_changed {
        ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
    }
    Ok(dues)
}

/// 本文をタスク行と同じ文法で読み、指定された項目で上書きする。
fn build(text: &str, fields: Fields, today: NaiveDate) -> Result<Task> {
    let mut task = task_input::parse_text(text, today, "adding")?;
    if let Some(due) = fields.due {
        task.due = Some(task_input::parse_due(&due, today)?);
    }
    if let Some(due) = &task.due {
        task_input::check_due(due, today)?;
    }
    if let Some(project) = fields.project {
        task.project = Some(project.strip_prefix('+').unwrap_or(&project).to_string());
    }
    if let Some(context) = fields.context {
        task.context = Some(context.strip_prefix('@').unwrap_or(&context).to_string());
    }
    if fields.priority.is_some() {
        task.priority = fields.priority;
    }
    if task.content.is_empty() {
        bail!("the task text must not be empty");
    }
    task.created.get_or_insert_with(|| today.format("%Y-%m-%d").to_string());
    task_input::check_round_trip(&task, today)?;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    fn build_text(text: &str) -> Result<Task> {
        build(text, Fields::default(), today())
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
    fn options_override_tags_in_the_text() {
        let fields = Fields {
            due: Some("+1w".into()),
            project: Some("+work".into()),
            context: Some("@office".into()),
            priority: Some('B'),
        };
        let task = build("(A) 資料 +home @15 due:today", fields, today()).unwrap();
        assert_eq!(task.content, "資料");
        assert_eq!(task.priority, Some('B'));
        assert_eq!(task.project.as_deref(), Some("work"));
        assert_eq!(task.context.as_deref(), Some("office"));
        assert_eq!(task.due.as_deref(), Some("2026-02-10"));

        let fields = Fields {
            due: Some("today".into()),
            ..Fields::default()
        };
        let task = build("x due:someday", fields, today()).unwrap();
        assert_eq!(
            task.due.as_deref(),
            Some("2026-02-03"),
            "an unreadable due in the text is replaced"
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
        let field = |f: Fields| build("x", f, today());
        assert!(
            field(Fields {
                project: Some("two words".into()),
                ..Fields::default()
            })
            .is_err()
        );
        assert!(
            field(Fields {
                priority: Some('a'),
                ..Fields::default()
            })
            .is_err()
        );
        assert!(
            field(Fields {
                due: Some("someday".into()),
                ..Fields::default()
            })
            .is_err()
        );
        assert!(build_text("x due:someday").is_err());
        assert!(build_text("x +a\u{b}").is_err());
        assert!(
            field(Fields {
                due: Some("2026-02-30".into()),
                ..Fields::default()
            })
            .is_err()
        );
        assert!(build_text("x due:2026-99-99").is_err());
        assert!(build_text("x created:2026-13-45").is_err());
        assert!(build_text("+only-a-tag").is_err());
        assert!(build_text("two\nlines").is_err());
    }
}
