use std::fmt::Write;

use anyhow::Result;
use gtodo_core::ops;

use crate::workspace::Workspace;
use crate::writing;

/// 完了にする。done.md へ移るのは次の日次の整理のとき (Neovim 版と同じ)。
pub fn done(ws: &Workspace, key: &str, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let task = writing::with_lock(ws, false, warn, || {
        Ok(ops::complete(&ws.data_dir, &ws.names, ws.today, key)?)
    })?;
    writeln!(out, "completed {}: {}", task.id.unwrap_or_default(), task.content)?;
    Ok(())
}
