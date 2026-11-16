use std::fmt::Write;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::input;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Changes};
use gtodo_core::store::FileKind;

use crate::workspace::Workspace;
use crate::writing;

/// 本文以外で指定する項目。空にするとその項目を消す。本文中のタグより優先する。
#[derive(Debug, Default)]
pub struct Fields {
    pub due: Option<String>,
    pub project: Option<String>,
    pub context: Option<String>,
    pub priority: Option<String>,
}

pub fn run(
    ws: &Workspace,
    key: &str,
    text: &str,
    fields: Fields,
    warn: &mut impl FnMut(String),
    out: &mut String,
) -> Result<()> {
    // ロックを取る前に検証し、受け付けられない入力では何も書かない。
    let changes = changes(text, fields, ws.today)?;
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

fn changes(text: &str, fields: Fields, today: NaiveDate) -> Result<Changes> {
    let fields = input::Fields {
        due: fields.due,
        project: fields.project,
        context: fields.context,
        priority: fields.priority,
        wait: None,
    };
    let changes = match input::changes(text, &fields, today) {
        // CLI ではどのオプションの値かを示す。
        Err(input::InputError::InvalidPriority) => bail!("--priority takes one letter (A-Z)"),
        other => other?,
    };
    if changes.is_empty() {
        bail!("nothing to change: give the new text or an option such as --due");
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    #[test]
    fn nothing_to_change_is_an_error() {
        for text in ["", "  "] {
            let err = changes(text, Fields::default(), today()).unwrap_err();
            assert!(err.to_string().contains("nothing to change"), "{err}");
        }
    }

    #[test]
    fn options_are_passed_as_fields() {
        let fields = Fields {
            due: Some(String::new()),
            priority: Some("B".into()),
            ..Fields::default()
        };
        assert_eq!(
            changes("", fields, today()).unwrap(),
            Changes {
                due: Some(None),
                priority: Some(Some('B')),
                ..Changes::default()
            }
        );
    }
}
