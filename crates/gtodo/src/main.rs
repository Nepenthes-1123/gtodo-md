mod commands;
mod render;
mod workspace;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};
use gtodo_core::config::{self, Platform, ProcessEnv};

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
    /// List tasks (inbox.md and todo.md by default)
    List {
        #[arg(value_enum)]
        target: Option<Target>,
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
        Command::List { target, json } => commands::list::run(&ws, target, json, out),
    }
}
