use std::fmt::Write;

use anyhow::Result;
use gtodo_core::ops;

use crate::workspace::Workspace;
use crate::writing;

pub fn run(ws: &Workspace, key: &str, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let task = writing::with_lock(ws, false, warn, || {
        Ok(ops::cancel(&ws.data_dir, &ws.names, ws.today, key)?)
    })?;
    writeln!(out, "cancelled {}: {}", task.id.unwrap_or_default(), task.content)?;
    Ok(())
}
