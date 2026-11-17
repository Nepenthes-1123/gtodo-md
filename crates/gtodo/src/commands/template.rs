//! `gtodo template`: テンプレート (`templates/<name>.md`) の一覧・作成・挿入・アーカイブ。

use std::collections::HashMap;
use std::fmt::Write;

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use gtodo_core::ops;
use gtodo_core::store;
use gtodo_core::template;
use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use crate::commands::add::check_dues_and_sort;
use crate::workspace::Workspace;
use crate::writing;

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List templates (templates/*.md), most recently changed first
    List {
        /// List archived templates (templates/archive/*.md) instead
        #[arg(long)]
        archived: bool,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create templates/<name>.md with the same notes as Neovim, and print its path
    New {
        /// Template name (letters, digits, - and _)
        name: String,
    },
    /// Add the tasks of a template to inbox.md
    Insert {
        /// Template name
        name: String,
        /// Date that relative due dates (due:+3d, ...) count from (today, +3d, fri, 2026-07-02, ...)
        #[arg(long, default_value = "today")]
        base: String,
        /// Value for a {{placeholder}} (every placeholder in the template must be given; an empty value omits it)
        #[arg(long = "set", value_name = "NAME=VALUE")]
        set: Vec<String>,
    },
    /// Archive a template (moves it to templates/archive/)
    Archive {
        /// Template name
        name: String,
    },
    /// Restore an archived template
    Restore {
        /// Template name
        name: String,
    },
}

pub fn run(ws: &Workspace, command: TemplateCommand, warn: &mut impl FnMut(String), out: &mut String) -> Result<()> {
    match command {
        TemplateCommand::List { archived, json } => list(ws, archived, json, out),
        TemplateCommand::New { name } => {
            let path = writing::with_lock(ws, false, warn, || Ok(ops::create_template(&ws.data_dir, &name)?))?;
            writeln!(out, "created {}", path.display())?;
            Ok(())
        }
        TemplateCommand::Insert { name, base, set } => insert(ws, &name, &base, &set, warn, out),
        TemplateCommand::Archive { name } => {
            writing::with_lock(ws, false, warn, || Ok(ops::move_template(&ws.data_dir, &name, false)?))?;
            writeln!(out, "archived {name}")?;
            Ok(())
        }
        TemplateCommand::Restore { name } => {
            writing::with_lock(ws, false, warn, || Ok(ops::move_template(&ws.data_dir, &name, true)?))?;
            writeln!(out, "restored {name}")?;
            Ok(())
        }
    }
}

/// テンプレートの行を読む。
fn read(ws: &Workspace, name: &str, archived: bool) -> Result<Vec<String>> {
    template::check_name(name)?;
    let path = if archived {
        template::archived_path(&ws.data_dir, name)
    } else {
        template::path(&ws.data_dir, name)
    };
    let file = store::read_file(&path)?;
    if file.stamp.is_none() {
        bail!("no template named {name:?} ({})", path.display());
    }
    Ok(file.lines)
}

#[derive(Serialize)]
struct TemplateJson {
    name: String,
    path: String,
    tasks: usize,
    placeholders: Vec<String>,
}

fn list(ws: &Workspace, archived: bool, json: bool, out: &mut String) -> Result<()> {
    let dir = ws.data_dir.root().join(template::DIR);
    let dir = if archived { dir.join(template::ARCHIVE_DIR) } else { dir };
    let names = template::list_names(&dir).with_context(|| format!("failed to read {}", dir.display()))?;
    let mut rows = Vec::new();
    for name in names {
        let lines = store::read_file(&dir.join(format!("{name}.md")))?.lines;
        rows.push(TemplateJson {
            path: dir.join(format!("{name}.md")).display().to_string(),
            tasks: template::tasks(&lines, ws.today, &HashMap::new()).len(),
            placeholders: template::placeholder_names(&lines),
            name,
        });
    }
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
        return Ok(());
    }
    if rows.is_empty() {
        writeln!(
            out,
            "{}",
            if archived {
                "No archived templates."
            } else {
                "No templates."
            }
        )?;
        return Ok(());
    }
    let width = rows.iter().map(|r| r.name.width()).max().unwrap_or(0);
    for row in &rows {
        let pad = " ".repeat(width - row.name.width());
        let mut line = format!("{}{pad}  {} task(s)", row.name, row.tasks);
        for name in &row.placeholders {
            line.push_str(&format!("  {{{{{name}}}}}"));
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn insert(
    ws: &Workspace,
    name: &str,
    base: &str,
    set: &[String],
    warn: &mut impl FnMut(String),
    out: &mut String,
) -> Result<()> {
    // ロックを取る前に確かめ、受け付けられない入力では何も書かない。
    let lines = read(ws, name, false)?;
    let base = template::parse_base(base, ws.today).map_err(|e| anyhow!("--base: {e}"))?;
    let mut values = HashMap::new();
    for item in set {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--set {item:?}: write it as NAME=VALUE"))?;
        values.insert(key.to_string(), template::check_value(key, value)?);
    }
    let wanted = template::placeholder_names(&lines);
    let missing: Vec<String> = wanted.iter().filter(|n| !values.contains_key(*n)).cloned().collect();
    if !missing.is_empty() {
        let sets: Vec<String> = missing.iter().map(|n| format!("--set {n}=...")).collect();
        bail!(
            "give a value for every placeholder: {} (an empty value such as --set {}= omits it)",
            sets.join(" "),
            missing[0]
        );
    }
    for key in values.keys().filter(|k| !wanted.contains(k)) {
        warn(format!("the template has no {{{{{key}}}}}; --set {key} is ignored"));
    }
    let tasks = template::tasks(&lines, base, &values);
    if tasks.is_empty() {
        bail!("template {name:?} has no task lines");
    }
    let (ids, dues) = writing::with_lock(ws, true, warn, || {
        let ids = ops::insert_tasks(&ws.data_dir, &ws.names, ws.today, tasks)?;
        // gtodo add と同じく、期日が来ていればこの場で Today へ移す。失敗は挿入の成否と分けて返す。
        Ok((ids, check_dues_and_sort(ws)))
    })?;
    writeln!(
        out,
        "added {} task(s) from {name} to inbox.md: {}",
        ids.len(),
        ids.join(" ")
    )?;
    match dues {
        Ok(dues) => {
            let mut notes = Vec::new();
            writing::report_dues(ws, &dues, warn, |msg| notes.push(msg));
            for note in notes {
                writeln!(out, "{note}")?;
            }
        }
        Err(e) => warn(format!("the tasks were added, but checking due dates failed: {e:#}")),
    }
    Ok(())
}
