//! `gtodo project`: プロジェクトファイル (`projects/<tag>.md`) の一覧・表示・作成・アーカイブ。

use std::collections::HashMap;
use std::fmt::Write;

use anyhow::{Result, bail};
use clap::Subcommand;
use gtodo_core::document::Document;
use gtodo_core::ops;
use gtodo_core::project::{self, Progress, ProjectFile};
use gtodo_core::store::FileKind;
use gtodo_core::task::Status;
use gtodo_core::view::{Entry, entries};
use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use crate::render::{KEY_WIDTH, TaskJson, key_of, summary};
use crate::workspace::Workspace;
use crate::writing;

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects (projects/*.md) with their progress, most recently changed first
    List {
        /// List archived projects instead
        #[arg(long)]
        archived: bool,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a project: its fields, progress and open tasks
    Show {
        /// Project name (the +project tag, with or without +)
        tag: String,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create projects/<tag>.md from the same template as Neovim
    New {
        /// Project name (the +project tag, with or without +)
        tag: String,
    },
    /// Archive a project (sets status: archived in its file)
    Archive {
        /// Project name (the +project tag, with or without +)
        tag: String,
    },
    /// Restore an archived project (sets status: active in its file)
    Restore {
        /// Project name (the +project tag, with or without +)
        tag: String,
    },
}

pub fn run(ws: &Workspace, command: ProjectCommand, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    match command {
        ProjectCommand::List { archived, json } => list(ws, archived, json, warn, out),
        ProjectCommand::Show { tag, json } => show(ws, &tag, json, warn, out),
        ProjectCommand::New { tag } => {
            let path = writing::with_lock(ws, false, warn, || {
                Ok(ops::create_project(&ws.data_dir, &tag, ws.today)?)
            })?;
            writeln!(out, "created {}", path.display())?;
            Ok(())
        }
        ProjectCommand::Archive { tag } => set_status(ws, &tag, project::ARCHIVED, warn, out),
        ProjectCommand::Restore { tag } => set_status(ws, &tag, project::ACTIVE, warn, out),
    }
}

/// JSON での 1 件。
#[derive(Serialize)]
struct ProjectJson<'a> {
    tag: &'a str,
    title: Option<&'a str>,
    status: &'static str,
    created: Option<&'a str>,
    due: Option<&'a str>,
    members: Option<&'a str>,
    path: String,
    done: usize,
    open: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    tasks: Option<Vec<TaskJson<'a>>>,
}

impl<'a> ProjectJson<'a> {
    fn new(file: &'a ProjectFile, progress: Progress) -> ProjectJson<'a> {
        ProjectJson {
            tag: &file.tag,
            title: file.title.as_deref(),
            status: if file.archived {
                project::ARCHIVED
            } else {
                project::ACTIVE
            },
            created: file.created.as_deref(),
            due: file.due.as_deref(),
            members: file.members.as_deref(),
            path: file.path.display().to_string(),
            done: progress.done,
            open: progress.open,
            tasks: None,
        }
    }
}

/// inbox.md・todo.md・done.md を読む (進捗を数えるため)。
struct Docs {
    inbox: Document,
    todo: Document,
    done: Document,
}

impl Docs {
    fn load(ws: &Workspace) -> Result<Docs> {
        Ok(Docs {
            inbox: ws.load(FileKind::Inbox)?,
            todo: ws.load(FileKind::Todo)?,
            done: ws.load(FileKind::Done)?,
        })
    }

    fn progress(&self) -> HashMap<String, Progress> {
        project::progress(
            project::tasks(&self.inbox).chain(project::tasks(&self.todo)),
            project::tasks(&self.done),
        )
    }
}

fn list(ws: &Workspace, archived: bool, json: bool, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let mut warnings = Vec::new();
    let files: Vec<ProjectFile> = project::list(&ws.data_dir, &mut warnings)?
        .into_iter()
        .filter(|p| p.archived == archived)
        .collect();
    warnings.into_iter().for_each(&mut *warn);
    let progress = Docs::load(ws)?.progress();
    let progress_of = |tag: &str| progress.get(tag).copied().unwrap_or_default();
    if json {
        let rows: Vec<_> = files.iter().map(|f| ProjectJson::new(f, progress_of(&f.tag))).collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
        return Ok(());
    }
    if files.is_empty() {
        writeln!(
            out,
            "{}",
            if archived {
                "No archived projects."
            } else {
                "No projects."
            }
        )?;
        return Ok(());
    }
    let width = files.iter().map(|f| f.tag.width()).max().unwrap_or(0);
    for file in &files {
        let pad = " ".repeat(width - file.tag.width());
        let mut line = format!("{}{pad}  {}", file.tag, progress_of(&file.tag).bar());
        if let Some(title) = &file.title {
            line.push_str("  ");
            line.push_str(title);
        }
        if let Some(due) = &file.due {
            line.push_str(&format!("  due:{due}"));
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn show(ws: &Workspace, tag: &str, json: bool, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let tag = tag.trim().trim_start_matches('+');
    let mut warnings = Vec::new();
    let file = project::list(&ws.data_dir, &mut warnings)?
        .into_iter()
        .find(|p| p.tag == tag);
    warnings.into_iter().for_each(&mut *warn);
    let docs = Docs::load(ws)?;
    let open: Vec<Entry> = entries(FileKind::Inbox, &docs.inbox)
        .chain(entries(FileKind::Todo, &docs.todo))
        .filter(|e| e.task().status == Status::Open && e.task().project.as_deref() == Some(tag))
        .collect();
    let progress = docs.progress().get(tag).copied().unwrap_or_default();
    if file.is_none() && progress.total() == 0 {
        bail!("no project file (projects/{tag}.md) and no tasks for +{tag}");
    }

    if json {
        let mut row = match &file {
            Some(file) => ProjectJson::new(file, progress),
            None => ProjectJson {
                tag,
                title: None,
                status: project::ACTIVE,
                created: None,
                due: None,
                members: None,
                path: project::path(&ws.data_dir, tag).display().to_string(),
                done: progress.done,
                open: progress.open,
                tasks: None,
            },
        };
        row.tasks = Some(open.iter().map(TaskJson::new).collect());
        writeln!(out, "{}", serde_json::to_string_pretty(&row)?)?;
        return Ok(());
    }

    match &file {
        Some(file) => {
            match &file.title {
                Some(title) => writeln!(out, "{tag}  {title}")?,
                None => writeln!(out, "{tag}")?,
            }
            let status = if file.archived {
                project::ARCHIVED
            } else {
                project::ACTIVE
            };
            for (name, value) in [
                ("status", Some(status)),
                ("created", file.created.as_deref()),
                ("due", file.due.as_deref()),
                ("members", file.members.as_deref()),
            ] {
                if let Some(value) = value {
                    writeln!(out, "  {:<8} {value}", format!("{name}:"))?;
                }
            }
            writeln!(out, "  {:<8} {}", "file:", file.path.display())?;
        }
        None => {
            writeln!(out, "{tag}")?;
            writeln!(out, "  no project file (create it with `gtodo project new {tag}`)")?;
        }
    }
    writeln!(out, "  {}", progress.bar())?;
    if !open.is_empty() {
        writeln!(out)?;
        for entry in &open {
            let place = entry.section.unwrap_or(entry.file.file_name());
            writeln!(
                out,
                "  {:<KEY_WIDTH$}  {}  [{place}]",
                key_of(entry),
                summary(entry.task())
            )?;
        }
    }
    Ok(())
}

fn set_status(ws: &Workspace, tag: &str, status: &str, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    let changed = writing::with_lock(ws, false, warn, || {
        Ok(ops::set_project_status(&ws.data_dir, tag, status)?)
    })?;
    let tag = tag.trim().trim_start_matches('+');
    let done = if status == project::ARCHIVED {
        "archived"
    } else {
        "restored"
    };
    if changed {
        writeln!(out, "{done} {tag}")?;
    } else {
        let state = if status == project::ARCHIVED {
            "archived"
        } else {
            "active"
        };
        writeln!(out, "{tag} is already {state}")?;
    }
    Ok(())
}
