//! TUI (`gtodo tui`、引数なしの `gtodo`)。

mod app;
mod draw;
mod form;
mod input;
mod kanban;
mod keymap;
mod markdown;
mod queue;
mod search;

#[cfg(test)]
mod tests;

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use chrono::Local;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;

use self::app::App;
use self::draw::Theme;
use self::keymap::Keymap;

/// ファイルの変更と日付の変更を確かめる間隔。
const TICK: Duration = Duration::from_secs(1);

/// `warnings` は data_dir を決めるまでに出た警告。画面に出す (標準エラーに書くと画面が崩れるため)。
/// `keys` は `tui.toml` のパス (決められなければ `None`)。
pub fn run(data_dir: PathBuf, warnings: Vec<String>, keys: Option<PathBuf>) -> Result<()> {
    let mut warnings = warnings;
    // 端末を確かめる前に読む。書き方の誤りは、どこから実行しても知らせる。
    let keymap = match keys {
        Some(path) => load_keymap(&path, &mut warnings)?,
        None => Keymap::default(),
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the TUI needs a terminal; use the subcommands (see gtodo --help) from scripts");
    }
    // 端末を切り替える前に開く。開けないときのエラーを普通に表示するため。
    let clock = Box::new(|| Local::now().date_naive());
    let mut app = App::new(data_dir, clock, keymap, warnings)?;
    let theme = Theme::from_env();

    let mut terminal = ratatui::try_init()?;
    // 貼り付けた改行で入力欄が確定しないよう、貼り付けを 1 つの入力として受け取る。
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = event_loop(&mut terminal, &mut app, theme);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::try_restore()?;
    result
}

/// `tui.toml` を読む。無ければ既定のキー。書き方の誤りは、どこが誤りかを添えてエラーにする。
fn load_keymap(path: &Path, warnings: &mut Vec<String>) -> Result<Keymap> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Keymap::default()),
        Err(e) => bail!("failed to read {}: {e}", path.display()),
    };
    let loaded = Keymap::from_toml(&text).map_err(|e| anyhow!("invalid {}: {e}", path.display()))?;
    warnings.extend(loaded.warnings.into_iter().map(|w| format!("{}: {w}", path.display())));
    Ok(loaded.keymap)
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, theme: Theme) -> Result<()> {
    let mut next_tick = Instant::now() + TICK;
    loop {
        terminal.draw(|frame| draw::draw(frame, app, theme))?;
        if app.should_quit() {
            return Ok(());
        }
        // 入力が無い間は次の確認まで眠る。
        if event::poll(next_tick.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                // Windows は離したときにも送ってくる。
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                Event::Paste(text) => app.handle_paste(&text),
                _ => {}
            }
        }
        if Instant::now() >= next_tick {
            app.tick();
            next_tick = Instant::now() + TICK;
        }
    }
}
