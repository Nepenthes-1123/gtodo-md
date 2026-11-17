//! `gtodo split`: タスクを、それを引き継いだ複数のサブタスクで置き換える (プロジェクトへの昇格)。

use std::fmt::Write;

use anyhow::{Context, Result};
use gtodo_core::input;
use gtodo_core::ops::{self, SplitProject};
use gtodo_core::store::FileKind;

use crate::commands::edit::check_dues_and_sort;
use crate::workspace::Workspace;
use crate::writing;

pub fn run(
    ws: &Workspace,
    key: &str,
    project: Option<String>,
    subtasks: &[String],
    warn: &mut impl FnMut(String),
    out: &mut String,
) -> Result<()> {
    // ロックを取る前に確かめ、受け付けられない入力では何も書かない。
    let mut tasks = Vec::new();
    for text in subtasks {
        let task = input::parse_text(text, ws.today, "splitting").with_context(|| format!("subtask {text:?}"))?;
        if let Some(due) = &task.due {
            input::check_due(due, ws.today).with_context(|| format!("subtask {text:?}"))?;
        }
        tasks.push(task);
    }
    let project = match project.as_deref().map(str::trim) {
        None => SplitProject::Keep,
        Some("") => SplitProject::Remove,
        Some(tag) => SplitProject::Set(tag.to_string()),
    };
    let (split, dues) = writing::with_lock(ws, true, warn, || {
        let split = ops::split_task(&ws.data_dir, &ws.names, ws.today, key, &project, tasks)?;
        // 編集と同じく、期日をチェックし、todo.md なら並べ替える。分割は済んでいるので、失敗は分けて返す。
        let dues = check_dues_and_sort(ws, split.file == FileKind::Todo);
        Ok((split, dues))
    })?;
    let ids: Vec<&str> = split.tasks.iter().filter_map(|t| t.id.as_deref()).collect();
    writeln!(out, "split {key} into {} task(s): {}", ids.len(), ids.join(" "))?;
    if let Some(path) = &split.created_project {
        writeln!(out, "created {}", path.display())?;
    }
    match dues {
        Ok(dues) => {
            let mut notes = Vec::new();
            writing::report_dues(ws, &dues, warn, |msg| notes.push(msg));
            for note in notes {
                writeln!(out, "{note}")?;
            }
        }
        Err(e) => warn(format!("the task was split, but checking due dates failed: {e:#}")),
    }
    Ok(())
}
