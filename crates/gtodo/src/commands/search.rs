use std::fmt::Write;

use anyhow::{Result, bail};
use gtodo_core::task::Status;
use gtodo_core::view::{Entry, SEARCH_FILES, SearchQuery, entries};

use crate::render::{KEY_WIDTH, TaskJson, key_of, summary};
use crate::workspace::Workspace;

pub fn run(ws: &Workspace, terms: &[String], status: Option<Status>, json: bool, out: &mut String) -> Result<()> {
    let query = parse_query(terms, status)?;

    let docs = SEARCH_FILES
        .iter()
        .map(|&f| ws.load(f).map(|doc| (f, doc)))
        .collect::<Result<Vec<_>>>()?;
    let hits: Vec<Entry> = docs
        .iter()
        .flat_map(|(f, doc)| entries(*f, doc))
        .filter(|e| query.matches(e.task()))
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

fn parse_query(terms: &[String], status: Option<Status>) -> Result<SearchQuery> {
    let mut query = SearchQuery {
        status,
        ..SearchQuery::default()
    };
    for term in terms {
        if let Some(p) = term.strip_prefix('+').filter(|p| !p.is_empty()) {
            query.projects.push(p.to_string());
        } else if let Some(c) = term.strip_prefix('@').filter(|c| !c.is_empty()) {
            query.contexts.push(c.to_string());
        } else {
            bail!("unsupported search term {term:?}: use +project or @context");
        }
    }
    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(ts: &[&str]) -> Vec<String> {
        ts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn terms_become_project_and_context_conditions() {
        let q = parse_query(&terms(&["+work", "@office", "@home"]), Some(Status::Open)).unwrap();
        assert_eq!(q.projects, ["work"]);
        assert_eq!(q.contexts, ["office", "home"]);
        assert_eq!(q.status, Some(Status::Open));
    }

    #[test]
    fn bare_words_and_empty_tags_are_rejected() {
        for t in ["word", "+", "@"] {
            assert!(parse_query(&terms(&[t]), None).is_err(), "{t:?}");
        }
    }
}
