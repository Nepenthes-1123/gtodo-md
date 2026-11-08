use std::fmt::Write;

use anyhow::Result;
use clap::ValueEnum;
use gtodo_core::ops::{self, Moved};
use gtodo_core::sections::SectionKey;

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

pub fn run(ws: &Workspace, key: &str, to: To, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let to = to.key();
    let moved = writing::with_lock(ws, false, warn, || {
        Ok(ops::move_task(&ws.data_dir, &ws.names, ws.today, key, to)?)
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
        // 何も書いていないので、ID の無いタスクには ID が付いていない。
        Moved::AlreadyThere(task) => writeln!(out, "{} is already in {name}", task.id.as_deref().unwrap_or(key))?,
    }
    Ok(())
}
