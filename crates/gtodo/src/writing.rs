//! 書き込むコマンドに共通の手順。

use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use gtodo_core::atomic;
use gtodo_core::daily;
use gtodo_core::lock::Lock;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops;
use gtodo_core::sections::SectionKey;

use crate::workspace::Workspace;

/// ロックが空くまで待つ時間。Neovim 版の自動処理がロックを持つのは数ミリ秒なので、
/// それと重なっただけなら待てば取れる。長く待っても取れないのは他の CLI が実行中の場合。
pub const LOCK_WAIT: Duration = Duration::from_secs(2);

/// ロックを持ったまま `command` を実行する。その前に、無いファイルを作り、
/// 日付が変わっていれば日次ロールオーバーを実行する (Neovim 版の 60 秒タイマーの代わり)。
///
/// `command` が自分で期日をチェックして知らせるなら `checks_dues` を真にする。ロールオーバーでの
/// 期日の警告と同じ内容を 2 回出さないため。
pub fn with_lock<T>(
    ws: &Workspace,
    checks_dues: bool,
    warn: &mut impl FnMut(String),
    command: impl FnOnce() -> Result<T>,
) -> Result<T> {
    ensure_writable(ws)?;
    let lock = Lock::acquire(ws.data_dir.root(), LOCK_WAIT)?;
    run_locked(
        ws,
        lock,
        checks_dues,
        warn,
        &mut |msg| eprintln!("gtodo: {msg}"),
        command,
    )
}

/// 書き込めない設定なら理由を返す。ロックを取る前に確かめる。
pub fn ensure_writable(ws: &Workspace) -> Result<()> {
    if let Some(e) = &ws.unusable_last_names {
        // 既定の名前で書くと、Neovim 版の見出しを別のセクションとして書き換えてしまう。
        bail!(
            "the section names Neovim used last are invalid ({e}); set \"sections\" in gtodo.json before running commands that write"
        );
    }
    Ok(())
}

/// 取得済みの `lock` を持ったまま、[`with_lock`] と同じ準備をしてから `command` を実行し、ロックを解放する。
///
/// ロックの取り方 (待つか、1 回だけ試すか) を呼び出し元が決めるために分けてある。TUI は
/// 利用者の操作では待ち、定期的な確認では待たない。`note` には、警告ではない知らせ
/// (完了タスクを done.md へ移したことなど) が渡される。
pub fn run_locked<T>(
    ws: &Workspace,
    lock: Lock,
    checks_dues: bool,
    warn: &mut impl FnMut(String),
    note: &mut impl FnMut(String),
    command: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let result = prepare(ws, checks_dues, warn, note).and_then(|()| command());
    // コマンドの書き込みは済んでいるため、失敗として返すと実行し直されて二重に書かれうる。
    if let Err(e) = lock.release() {
        warn(e.to_string());
    }
    result
}

/// 無いファイルを作り、古い一時ファイルを片付け、日次ロールオーバーを実行する。ロックを持ったまま呼ぶ。
fn prepare(
    ws: &Workspace,
    checks_dues: bool,
    warn: &mut impl FnMut(String),
    note: &mut impl FnMut(String),
) -> Result<()> {
    let root = ws.data_dir.root();
    ops::ensure_files(&ws.data_dir, &ws.names)?;
    for dir in [root.to_path_buf(), root.join("projects")] {
        if let Err(e) = atomic::sweep_stale_tmp(&dir, SystemTime::now()) {
            warn(format!("failed to clean up temporary files in {}: {e}", dir.display()));
        }
    }
    if let Some(r) = daily::rollover(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)? {
        if r.completed > 0 {
            note(format!("moved {} completed task(s) to done.md", r.completed));
        }
        let mut dues = r.dues;
        if checks_dues {
            dues.inbox_due.clear();
        }
        report_dues(ws, &dues, warn, &mut *note);
        if let Some(e) = r.not_recorded {
            warn(format!("{e}; the daily rollover will run again next time"));
        }
    }
    Ok(())
}

/// 期日が来たのに inbox.md に残したタスクの警告の書き出し (TUI が同じ知らせを重ねて出さないために使う)。
pub const INBOX_DUE: &str = "due tasks in inbox.md";

/// 期日が来たタスクを Today へ移したことと、移さなかった inbox のタスクを知らせる。
pub fn report_dues(ws: &Workspace, dues: &DueOutcome, warn: &mut impl FnMut(String), mut note: impl FnMut(String)) {
    if dues.moved > 0 {
        note(format!(
            "moved {} due task(s) to {}",
            dues.moved,
            ws.names.name(SectionKey::Today)
        ));
    }
    if !dues.inbox_due.is_empty() {
        let tasks: Vec<String> = dues
            .inbox_due
            .iter()
            .map(|t| format!("\n  {} (due: {})", t.content, t.due))
            .collect();
        warn(format!("{INBOX_DUE}:{}", tasks.concat()));
    }
}
