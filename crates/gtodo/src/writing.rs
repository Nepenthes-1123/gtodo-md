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
const LOCK_WAIT: Duration = Duration::from_secs(2);

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
    if let Some(e) = &ws.unusable_last_names {
        // 既定の名前で書くと、Neovim 版の見出しを別のセクションとして書き換えてしまう。
        bail!(
            "the section names Neovim used last are invalid ({e}); set \"sections\" in gtodo.json before running commands that write"
        );
    }
    let root = ws.data_dir.root();
    let lock = Lock::acquire(root, LOCK_WAIT)?;

    ops::ensure_files(&ws.data_dir, &ws.names)?;
    for dir in [root.to_path_buf(), root.join("projects")] {
        if let Err(e) = atomic::sweep_stale_tmp(&dir, SystemTime::now()) {
            warn(format!("failed to clean up temporary files in {}: {e}", dir.display()));
        }
    }
    if let Some(r) = daily::rollover(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)? {
        if r.completed > 0 {
            eprintln!("gtodo: moved {} completed task(s) to done.md", r.completed);
        }
        let mut dues = r.dues;
        if checks_dues {
            dues.inbox_due.clear();
        }
        report_dues(ws, &dues, warn, |msg| eprintln!("gtodo: {msg}"));
        if let Some(e) = r.not_recorded {
            warn(format!("{e}; the daily rollover will run again next time"));
        }
    }

    let result = command();
    // コマンドの書き込みは済んでいるため、失敗として返すと実行し直されて二重に書かれうる。
    if let Err(e) = lock.release() {
        warn(e.to_string());
    }
    result
}

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
        warn(format!("due tasks in inbox.md:{}", tasks.concat()));
    }
}
