use std::fmt::Write;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use clap::ValueEnum;
use gtodo_core::ops::{self, Moved, Wait};
use gtodo_core::sections::SectionKey;
use gtodo_core::task::Task;

use crate::workspace::Workspace;
use crate::writing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum To {
    Today,
    Next,
    Waiting,
    Someday,
}

impl To {
    fn key(self) -> SectionKey {
        match self {
            To::Today => SectionKey::Today,
            To::Next => SectionKey::Next,
            To::Waiting => SectionKey::Waiting,
            To::Someday => SectionKey::Someday,
        }
    }
}

pub fn run(
    ws: &Workspace,
    key: &str,
    to: To,
    wait: Option<String>,
    warn: &mut impl FnMut(String),
    out: &mut String,
) -> Result<()> {
    let to = to.key();
    // ロックを取る前に検証し、受け付けられない入力では何も書かない。
    let wait = match wait {
        None => Wait::Keep,
        // Waiting 以外へ移すと待ち先は消えるため、指定を黙って捨てることになる。
        Some(_) if to != SectionKey::Waiting => bail!("--wait can only be used when moving to waiting"),
        Some(w) => parse_wait(&w, ws.today)?,
    };
    let moved = writing::with_lock(ws, false, warn, || {
        Ok(ops::move_task(&ws.data_dir, &ws.names, ws.today, key, to, wait)?)
    })?;

    let name = ws.names.name(to);
    match moved {
        Moved::Moved(task) => {
            let id = task.id.as_deref().unwrap_or_default();
            match &task.wait {
                Some(w) => writeln!(out, "moved {id} to {name} (wait:{w}): {}", task.content)?,
                None => writeln!(out, "moved {id} to {name}: {}", task.content)?,
            }
        }
        Moved::WaitChanged(task) => {
            let id = task.id.as_deref().unwrap_or_default();
            match &task.wait {
                Some(w) => writeln!(out, "set wait:{w} on {id}: {}", task.content)?,
                None => writeln!(out, "removed wait: from {id}: {}", task.content)?,
            }
        }
        // 何も書いていないので、ID の無いタスクには ID が付いていない。
        Moved::AlreadyThere(task) => writeln!(out, "{} is already in {name}", task.id.as_deref().unwrap_or(key))?,
    }
    Ok(())
}

/// 空なら待ち先を消す (Neovim 版の入力欄と同じ)。
fn parse_wait(value: &str, today: NaiveDate) -> Result<Wait> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Wait::Remove);
    }
    // 書いた行を読み直して同じ待ち先にならない値 (空白や `.` を含むものなど) は、次に読んだときに
    // 本文へ混ざって壊れるため受け付けない。文法は task.rs だけが知っている。
    let mut task = Task {
        content: "x".into(),
        wait: Some(value.to_string()),
        ..Task::default()
    };
    if Task::parse_at(&task.serialize(), today).and_then(|t| t.wait).as_deref() != Some(value) {
        bail!("{value:?} cannot be written as wait: (spaces and some punctuation such as . , ( ) are not allowed)");
    }
    Ok(Wait::Set(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    #[test]
    fn an_empty_wait_removes_it() {
        assert_eq!(parse_wait("", today()).unwrap(), Wait::Remove);
        assert_eq!(parse_wait("  ", today()).unwrap(), Wait::Remove);
    }

    #[test]
    fn a_wait_is_trimmed() {
        assert_eq!(parse_wait(" 田中さん ", today()).unwrap(), Wait::Set("田中さん".into()));
    }

    #[test]
    fn waits_that_cannot_be_read_back_are_rejected() {
        for value in ["bob smith", "bob.", "a(b)", "x\ny"] {
            assert!(parse_wait(value, today()).is_err(), "{value:?}");
        }
    }
}
