use std::fmt::Write;

use anyhow::Result;
use gtodo_core::store::FileKind;
use gtodo_core::view::Stats;

use crate::workspace::Workspace;

pub fn run(ws: &Workspace, json: bool, out: &mut String) -> Result<()> {
    let todo = ws.load(FileKind::Todo)?;
    let inbox = ws.load(FileKind::Inbox)?;
    let stats = Stats::collect(&todo, &inbox, &ws.names);
    if json {
        let value = serde_json::json!({ "today": stats.today, "inbox": stats.inbox });
        writeln!(out, "{value}")?;
    } else {
        writeln!(out, "{}", stats.statusline())?;
    }
    Ok(())
}
