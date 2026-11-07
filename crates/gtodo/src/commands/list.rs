use std::fmt::Write;

use anyhow::Result;
use clap::ValueEnum;
use gtodo_core::sections::SectionKey;
use gtodo_core::store::FileKind;
use gtodo_core::view::{Entry, entries};

use crate::render::{TaskJson, summary};
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Target {
    Inbox,
    /// All sections of todo.md
    Todo,
    Today,
    Next,
    Waiting,
    Someday,
    Done,
    Cancelled,
}

impl Target {
    fn file(self) -> FileKind {
        match self {
            Target::Inbox => FileKind::Inbox,
            Target::Done => FileKind::Done,
            Target::Cancelled => FileKind::Cancelled,
            Target::Todo | Target::Today | Target::Next | Target::Waiting | Target::Someday => FileKind::Todo,
        }
    }

    fn section(self) -> Option<SectionKey> {
        match self {
            Target::Today => Some(SectionKey::Today),
            Target::Next => Some(SectionKey::Next),
            Target::Waiting => Some(SectionKey::Waiting),
            Target::Someday => Some(SectionKey::Someday),
            _ => None,
        }
    }
}

pub fn run(ws: &Workspace, target: Option<Target>, json: bool, out: &mut String) -> Result<()> {
    let targets = match target {
        Some(t) => vec![t],
        None => vec![Target::Inbox, Target::Todo],
    };

    let docs = targets
        .iter()
        .map(|t| ws.load(t.file()).map(|doc| (*t, doc)))
        .collect::<Result<Vec<_>>>()?;
    let mut selected: Vec<Entry> = Vec::new();
    for (target, doc) in &docs {
        let section = target.section().map(|k| ws.names.name(k));
        selected.extend(entries(target.file(), doc).filter(|e| section.is_none() || e.section == section));
    }

    if json {
        let rows: Vec<_> = selected.iter().map(TaskJson::new).collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
        return Ok(());
    }

    if selected.is_empty() {
        writeln!(out, "No tasks.")?;
        return Ok(());
    }
    let mut current_group: Option<(FileKind, Option<&str>)> = None;
    for entry in &selected {
        let group = (entry.file, entry.section);
        if current_group != Some(group) {
            if current_group.is_some() {
                writeln!(out)?;
            }
            match entry.section {
                Some(section) => writeln!(out, "{} / {section}", entry.file.file_name())?,
                None => writeln!(out, "{}", entry.file.file_name())?,
            }
            current_group = Some(group);
        }
        writeln!(out, "  {}", summary(entry.task()))?;
    }
    Ok(())
}
