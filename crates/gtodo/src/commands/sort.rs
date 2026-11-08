use std::fmt::Write;

use anyhow::Result;
use gtodo_core::ops;

use crate::workspace::Workspace;
use crate::writing;

/// 期日が来たタスクを Today へ移してから todo.md を並べ替える (Neovim 版の手動ソートと同じ)。
pub fn run(ws: &Workspace, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let dues = writing::with_lock(ws, true, warn, || {
        let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
        ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
        Ok(dues)
    })?;
    let mut notes = Vec::new();
    writing::report_dues(ws, &dues, warn, |msg| notes.push(msg));
    for note in notes {
        writeln!(out, "{note}")?;
    }
    Ok(())
}
