mod commands;
mod render;
mod workspace;
mod writing;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};
use gtodo_core::config::{self, Platform, ProcessEnv};
use gtodo_core::task::Status;

use crate::commands::add::{Fields, To};
use crate::commands::list::Target;
use crate::workspace::Workspace;

/// GTD-style todo manager backed by Markdown, compatible with gtodo-md.nvim.
#[derive(Debug, Parser)]
#[command(name = "gtodo", version, arg_required_else_help = true)]
struct Cli {
    /// Data directory [default: GTODO_DATA_DIR, data_dir in config.json, or $XDG_DATA_HOME/gtodo-md]
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add a task (to inbox.md unless --to is given)
    Add {
        /// Task text; +project, @context, due:, wait: and a leading (A) are read as in task lines
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// Where to add the task
        #[arg(long, value_enum, default_value_t = To::Inbox)]
        to: To,
        /// Due date (today, tomorrow, +3d, fri, 2026-07-02, ...)
        #[arg(long)]
        due: Option<String>,
        /// Project (+project)
        #[arg(short, long)]
        project: Option<String>,
        /// Context (@context)
        #[arg(short, long)]
        context: Option<String>,
        /// Priority (A-Z)
        #[arg(long)]
        priority: Option<char>,
    },
    /// Mark a task as completed (it moves to done.md on the next day)
    Done {
        /// Key shown by `gtodo list`: an ID (or its first characters) or a ~ temporary key
        task: String,
    },
    /// Mark a completed task as not completed (before it moves to done.md)
    Undone {
        /// Key shown by `gtodo list`: an ID (or its first characters) or a ~ temporary key
        task: String,
    },
    /// Move a task to cancelled.md
    Cancel {
        /// Key shown by `gtodo list`: an ID (or its first characters) or a ~ temporary key
        task: String,
    },
    /// List tasks (inbox.md and todo.md by default)
    List {
        #[arg(value_enum)]
        target: Option<Target>,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Search inbox.md, todo.md and done.md by +project and @context (all terms must match)
    Search {
        /// +project or @context
        #[arg(value_name = "TERM")]
        terms: Vec<String>,
        /// Only open tasks
        #[arg(long, conflicts_with = "done")]
        open: bool,
        /// Only completed tasks
        #[arg(long)]
        done: bool,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show open tasks grouped by due date
    Queue {
        /// Group by the person being waited for (wait:) instead
        #[arg(long)]
        wait: bool,
    },
    /// Move due tasks to Today and sort todo.md
    Sort,
    /// Show task counts for status lines
    Stats {
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut out = String::new();
    let result = run(cli, &mut out);

    // 出力先が先に閉じられた場合 (`gtodo list | head` など) は正常終了として扱う。
    match io::stdout().lock().write_all(out.as_bytes()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            eprintln!("gtodo: error: failed to write output: {e}");
            return ExitCode::FAILURE;
        }
        _ => {}
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gtodo: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, out: &mut String) -> Result<()> {
    let mut warn = |msg: String| eprintln!("gtodo: warning: {msg}");
    let data_dir = config::resolve_data_dir(cli.data_dir, &ProcessEnv, Platform::current(), &mut warn)?;
    let today = Local::now().date_naive();
    let ws = Workspace::open(data_dir, today, &mut warn)?;

    match cli.command {
        Command::Add {
            text,
            to,
            due,
            project,
            context,
            priority,
        } => {
            let fields = Fields {
                due,
                project,
                context,
                priority,
            };
            commands::add::run(&ws, &text.join(" "), to, fields, &mut warn, out)
        }
        Command::Done { task } => commands::done::done(&ws, &task, &mut warn, out),
        Command::Undone { task } => commands::done::undone(&ws, &task, &mut warn, out),
        Command::Cancel { task } => commands::cancel::run(&ws, &task, &mut warn, out),
        Command::List { target, json } => commands::list::run(&ws, target, json, out),
        Command::Search {
            terms,
            open,
            done,
            json,
        } => {
            let status = match (open, done) {
                (true, _) => Some(Status::Open),
                (_, true) => Some(Status::Done),
                _ => None,
            };
            commands::search::run(&ws, &terms, status, json, out)
        }
        Command::Queue { wait } => commands::queue::run(&ws, wait, out),
        Command::Sort => commands::sort::run(&ws, &mut warn, out),
        Command::Stats { json } => commands::stats::run(&ws, json, out),
    }
}
