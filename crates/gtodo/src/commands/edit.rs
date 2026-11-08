use std::fmt::Write;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Changes};
use gtodo_core::store::FileKind;
use gtodo_core::task::Task;

use crate::commands::task_input;
use crate::workspace::Workspace;
use crate::writing;

pub fn run(ws: &Workspace, key: &str, text: &str, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    // ロックを取る前に検証し、受け付けられない入力では何も書かない。
    let changes = changes(text, ws.today)?;
    let (task, dues) = writing::with_lock(ws, true, warn, || {
        let edited = ops::edit_task(&ws.data_dir, &ws.names, ws.today, key, &changes)?;
        let dues = check_dues_and_sort(ws, edited.file == FileKind::Todo);
        Ok((edited.task, dues))
    })?;

    writeln!(out, "edited {}: {}", task.id.unwrap_or_default(), task.content)?;
    match dues {
        Ok(dues) => {
            let mut notes = Vec::new();
            writing::report_dues(ws, &dues, warn, |msg| notes.push(msg));
            for note in notes {
                writeln!(out, "{note}")?;
            }
        }
        // 編集は済んでいるため、失敗として返すと利用者は編集されていないと受け取る。
        Err(e) => warn(format!("the task was edited, but checking due dates failed: {e:#}")),
    }
    Ok(())
}

fn check_dues_and_sort(ws: &Workspace, sort: bool) -> Result<DueOutcome> {
    let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
    if sort || dues.todo_changed {
        ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
    }
    Ok(dues)
}

fn changes(text: &str, today: NaiveDate) -> Result<Changes> {
    let mut changes = Changes::default();
    if !text.trim().is_empty() {
        let task = task_input::parse_text(text, today, "editing")?;
        // Neovim 版の編集でも作成日は変えられない。
        if task.created.is_some() {
            bail!("created: cannot be changed");
        }
        if !task.content.is_empty() {
            changes.content = Some(task.content);
        }
        changes.priority = task.priority.map(Some);
        changes.project = task.project.map(Some);
        changes.context = task.context.map(Some);
        changes.due = task.due.map(Some);
        changes.wait = task.wait.map(Some);
    }

    if let Some(Some(due)) = &changes.due {
        task_input::check_due(due, today)?;
    }
    if changes.is_empty() {
        bail!("nothing to change: give the new text");
    }

    let mut probe = Task {
        content: "x".into(),
        ..Task::default()
    };
    changes.apply(&mut probe);
    task_input::check_round_trip(&probe, today)?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    fn text(text: &str) -> Result<Changes> {
        changes(text, today())
    }

    #[test]
    fn only_what_the_text_mentions_is_changed() {
        assert_eq!(
            text("(A) 新しい本文 +work due:tomorrow").unwrap(),
            Changes {
                content: Some("新しい本文".into()),
                priority: Some(Some('A')),
                project: Some(Some("work".into())),
                due: Some(Some("2026-02-04".into())),
                ..Changes::default()
            }
        );
        assert_eq!(
            text("@15 wait:bob").unwrap(),
            Changes {
                context: Some(Some("15".into())),
                wait: Some(Some("bob".into())),
                ..Changes::default()
            },
            "text with tags only keeps the description"
        );
    }

    #[test]
    fn nothing_to_change_is_an_error() {
        assert!(text("").is_err());
        assert!(text("  ").is_err());
    }

    #[test]
    fn values_that_cannot_be_written_are_rejected() {
        for t in ["x id:abcdef", "x created:2026-01-01", "x due:someday", "two\nlines"] {
            assert!(text(t).is_err(), "{t:?}");
        }
    }
}
