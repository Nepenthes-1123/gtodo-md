use std::fmt::Write;

use anyhow::Result;
use gtodo_core::search::{Indexed, Query};
use gtodo_core::task::Status;
use gtodo_core::view::{Entry, SEARCH_FILES, entries};

use crate::render::{KEY_WIDTH, TaskJson, key_of, summary};
use crate::workspace::Workspace;

/// 書き方と一致の規則は TUI の検索と同じ (core の `search`)。CLI ではすべての語が確定しているので、
/// タグは完全一致になる。並びはファイルの順のまま。
pub fn run(ws: &Workspace, terms: &[String], status: Option<Status>, json: bool, out: &mut String) -> Result<()> {
    let query = Query::parse(&terms.join(" "), false)?.with_status(status);

    let docs = SEARCH_FILES
        .iter()
        .map(|&f| ws.load(f).map(|doc| (f, doc)))
        .collect::<Result<Vec<_>>>()?;
    let hits: Vec<Entry> = docs
        .iter()
        .flat_map(|(f, doc)| entries(*f, doc))
        .filter(|e| query.matches(&Indexed::new(e.task())).is_some())
        .collect();

    if json {
        let rows: Vec<_> = hits.iter().map(TaskJson::new).collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
        return Ok(());
    }
    if hits.is_empty() {
        writeln!(out, "No matching tasks found.")?;
        return Ok(());
    }
    for e in &hits {
        let location = format!("{}:{}", e.file.file_name(), e.item.lnum);
        writeln!(out, "{:<KEY_WIDTH$}  {location}  {}", key_of(e), summary(e.task()))?;
    }
    Ok(())
}
