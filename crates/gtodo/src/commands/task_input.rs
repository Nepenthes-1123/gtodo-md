//! 利用者が渡したタスクの本文と項目の値を、タスク行の文法で読んで確かめる (`add` と `edit` で共有する)。

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::date;
use gtodo_core::task::Task;

/// 本文をタスク行と同じ文法で読む。本文が空でもタグだけのタスクとして返す。
/// 期日はオプションで置き換えられうるため、ここでは確かめない ([`check_due`])。
/// `action` はエラーメッセージに使う (`adding` など)。
pub fn parse_text(text: &str, today: NaiveDate, action: &str) -> Result<Task> {
    if text.contains(['\n', '\r']) {
        bail!("the task text must be a single line");
    }
    let Some(task) = Task::parse_at(&format!("- [ ] {text}"), today) else {
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
            bail!("{tag}: cannot be set when {action} a task");
        }
    }
    if let Some(created) = &task.created {
        if !is_date(created) {
            bail!("invalid created date {created:?} (use YYYY-MM-DD)");
        }
    }
    Ok(task)
}

/// オプションで渡された期日を、タスク行に書く形にする。
pub fn parse_due(input: &str, today: NaiveDate) -> Result<String> {
    let due = date::parse_due_date(input, today).ok_or_else(|| invalid_due(input))?;
    check_due(&due, today)?;
    Ok(due)
}

/// タスク行に書く期日が、実在する日付として読めるかを確かめる。
pub fn check_due(due: &str, today: NaiveDate) -> Result<()> {
    // 読めなかった期日は書かれたまま残り、日付の形の表記は範囲を確かめずに桁だけ揃えられる
    // (`2026-02-30` も通る) ため、正規化した形が実在する日付かで見分ける。
    if date::parse_due_date(due, today).as_deref() != Some(due) || !is_date(due) {
        return Err(invalid_due(due));
    }
    Ok(())
}

/// 書いた行を読み直して同じタスクにならない値 (空白を含むプロジェクト名や小文字の優先度など) は、
/// 次に読んだときに本文へ混ざって壊れるため受け付けない。文法は task.rs だけが知っている。
pub fn check_round_trip(task: &Task, today: NaiveDate) -> Result<()> {
    let mut written = Task {
        id: Some("000000".into()),
        ..task.clone()
    };
    let reread = Task::parse_at(&written.serialize(), today);
    if reread.as_ref().map(|t| Task { id: None, ..t.clone() })
        != Some(Task {
            id: None,
            ..task.clone()
        })
    {
        bail!("the project, context, priority or wait: value cannot be written in a task line");
    }
    Ok(())
}

fn is_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn invalid_due(due: &str) -> anyhow::Error {
    anyhow::anyhow!("invalid due date {due:?} (e.g. today, tomorrow, +3d, fri, 2026-07-02)")
}
