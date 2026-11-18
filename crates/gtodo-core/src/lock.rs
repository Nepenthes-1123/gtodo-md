//! `data_dir/.gtodo.lock`: Neovim 版と共有する排他ロック。Lua 版 `lock.lua` と同じ手順で取る。
//!
//! Neovim 版は自動処理 (期日のチェック・ソート・日次ロールオーバー) の間このロックを持ち、
//! 取れなければその回を諦める。CLI は書き込むコマンドの間もこのロックを持ち、
//! Neovim 版の自動処理と重ならないようにする。

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const LOCK_FILE: &str = ".gtodo.lock";

/// これより古いロックは、持っていたプロセスが異常終了して残ったものとみなして消す (Lua 版と同じ)。
const STALE_AFTER: Duration = Duration::from_secs(60);

/// 待つときの再試行の間隔。
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("{} is held by another process (Neovim may be running automatic processing); try again", path.display())]
    Busy { path: PathBuf },
    #[error(
        "{} was taken over by another process (it was held for more than 60 seconds); left it in place",
        path.display()
    )]
    TakenOver { path: PathBuf },
    #[error(
        "{} was removed by another process (it was held for more than 60 seconds)",
        path.display()
    )]
    Removed { path: PathBuf },
    #[error("failed to {op} {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

/// 取得したロック。[`Lock::release`] で解放する。解放し忘れても、自分のものである限り drop で消える。
///
/// 持つのは 60 秒未満に限り、利用者の入力や `$EDITOR` を待つ間は持たない。
/// 60 秒を超えると、Neovim 版が残骸とみなして取り直し、排他が崩れる。
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    /// ロックファイルに書いた中身。解放するときに、自分のロックかを見分ける。
    owner: String,
    released: bool,
}

/// 1 回の取得の試み。
enum Attempt {
    Acquired(Lock),
    /// 他のプロセスが持っている。
    Held,
    /// ロックファイルを作ろうとして拒まれた (Windows の `PermissionDenied`)。他のプロセスが消している途中の
    /// ファイルと同じ名前では作れず、このエラーになる (解放と取得が重なったとき)。本当に権限が無い場合と
    /// 見分けられないため、持たれているのと同じに扱い、待っても続くときだけエラーにする。
    Denied(LockError),
}

impl Lock {
    /// 1 回だけ取得を試みる。他のプロセスが持っていれば `None` (Lua 版の自動処理と同じ使い方)。
    /// Windows でロックファイルを作ろうとして拒まれたときも `None` (他のプロセスが解放している途中でありうるため)。
    pub fn try_acquire(data_dir: &Path) -> Result<Option<Lock>, LockError> {
        Ok(match Lock::attempt(data_dir)? {
            Attempt::Acquired(lock) => Some(lock),
            Attempt::Held | Attempt::Denied(_) => None,
        })
    }

    fn attempt(data_dir: &Path) -> Result<Attempt, LockError> {
        let path = data_dir.join(LOCK_FILE);
        let io_err = |op, source| LockError::Io {
            op,
            path: path.clone(),
            source,
        };
        remove_if_stale(&path, SystemTime::now()).map_err(|e| io_err("check", e))?;

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(Attempt::Held),
            Err(e) if may_be_being_removed(&e) => return Ok(Attempt::Denied(io_err("create", e))),
            Err(e) => return Err(io_err("create", e)),
        };
        let owner = owner_token();
        if let Err(e) = file.write_all(owner.as_bytes()) {
            drop(file);
            // 中身が持ち主の印になっていないため、drop での持ち主の確認では消せない。
            // 消せなくても元のエラーを返すことが優先 (残ったロックは 60 秒後に残骸として消える)。
            let _ = fs::remove_file(&path);
            return Err(io_err("write", e));
        }
        Ok(Attempt::Acquired(Lock {
            path,
            owner,
            released: false,
        }))
    }

    /// 取れるまで最大 `wait` だけ待つ。利用者が実行したコマンドを黙って諦めないため。
    /// 待ち終えたとき、最後の試みでロックファイルを作ろうとして拒まれていれば、そのエラーを返す
    /// (書き込めないディレクトリを「他のプロセスが持っている」と知らせないため)。
    pub fn acquire(data_dir: &Path, wait: Duration) -> Result<Lock, LockError> {
        // 表せないほど長い待ち時間は、期限なしとして扱う。
        let deadline = Instant::now().checked_add(wait);
        loop {
            let denied = match Lock::attempt(data_dir)? {
                Attempt::Acquired(lock) => return Ok(lock),
                Attempt::Held => None,
                Attempt::Denied(e) => Some(e),
            };
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(denied.unwrap_or_else(|| LockError::Busy {
                    path: data_dir.join(LOCK_FILE),
                }));
            }
            thread::sleep(RETRY_INTERVAL);
        }
    }

    /// 解放する。消せなかったロックが残ると、古いとみなされるまでの 60 秒間、
    /// Neovim 版の自動処理が黙って止まるため、失敗は呼び出し元へ返す。
    ///
    /// ロックが既に他のプロセスのものになっていれば消さずに [`LockError::TakenOver`] を、
    /// 既に消されていれば [`LockError::Removed`] を返す (60 秒を超えて持ち、残骸とみなされた場合)。
    /// 確かめずに消すと、取り直した相手のロックを消して排他を崩す (Lua 版も確かめる)。
    /// 確かめてから消すまでの間に取り直されることは防げない。
    pub fn release(mut self) -> Result<(), LockError> {
        self.released = true;
        self.remove_if_ours()
    }

    fn remove_if_ours(&self) -> Result<(), LockError> {
        let io_err = |op, source| LockError::Io {
            op,
            path: self.path.clone(),
            source,
        };
        match fs::read_to_string(&self.path) {
            Ok(owner) if owner == self.owner => fs::remove_file(&self.path).map_err(|e| io_err("remove", e)),
            Ok(_) => Err(LockError::TakenOver {
                path: self.path.clone(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(LockError::Removed {
                path: self.path.clone(),
            }),
            Err(e) => Err(io_err("read", e)),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if !self.released {
            // エラーや panic で抜けた経路。報告する先が無いが、残すよりは消すことを試みる。
            let _ = self.remove_if_ours();
        }
    }
}

/// ロックファイルの中身。先頭のプロセス ID は Lua 版と同じく、保持しているプロセスを調べる手がかり。
/// プロセス ID だけでは、Docker や WSL と Windows のように PID の空間が別のプロセスと一致しうるため、
/// 取得した時刻を添えて自分のロックかを見分ける。Lua 版も同じ形で書き、解放するときに見分ける。
fn owner_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{} {nanos}", std::process::id())
}

/// Windows では、他のプロセスが消している途中 (閉じられるまで名前が残る削除待ち) のファイルは、
/// 開くことも同じ名前で作ることも `PermissionDenied` で拒まれる。解放と取得が重なると起こる。
fn may_be_being_removed(e: &io::Error) -> bool {
    cfg!(windows) && e.kind() == io::ErrorKind::PermissionDenied
}

fn remove_if_stale(path: &Path, now: SystemTime) -> io::Result<()> {
    let modified = match fs::metadata(path) {
        Ok(meta) => meta.modified()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        // 消している途中なら古いかどうかは関係ない。続く作成で、取れたかどうかが決まる。
        Err(e) if may_be_being_removed(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    // 時計がずれて未来の時刻になっているロックは、古いとは判断できないので残す。
    if now.duration_since(modified).is_ok_and(|age| age > STALE_AFTER) {
        match fs::remove_file(path) {
            // 他のプロセスが先に消した場合。
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            other => other?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    #[test]
    fn only_one_holder_at_a_time_and_release_frees_it() {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lock::try_acquire(dir.path()).unwrap().unwrap();
        let content = fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
        assert_eq!(content.split(' ').next(), Some(std::process::id().to_string().as_str()));
        assert!(Lock::try_acquire(dir.path()).unwrap().is_none());

        lock.release().unwrap();
        assert!(!dir.path().join(LOCK_FILE).exists());
        assert!(Lock::try_acquire(dir.path()).unwrap().is_some());
    }

    /// Windows の旧来の削除方式では、他のプロセス (解放前に中身を確かめる別の gtodo、ウイルス対策、同期クライアントなど)
    /// が開いている間に消したファイルは、閉じられるまで名前が残り、作り直しが拒まれる。取得はこれを持たれているのと
    /// 同じに扱って待つが (下の `a_lock_file_being_removed_counts_as_held`)、NTFS の今の Windows では名前がすぐ空き、
    /// 待たずに取れることを CI の Windows で確かめる。
    #[test]
    fn a_lock_released_while_another_process_has_it_open_can_be_taken_again() {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lock::try_acquire(dir.path()).unwrap().unwrap();
        let reader = File::open(dir.path().join(LOCK_FILE)).unwrap();
        lock.release().unwrap();
        let again = Lock::try_acquire(dir.path());
        drop(reader);
        assert!(matches!(again, Ok(Some(_))), "{again:?}");
    }

    /// 他のハンドルが開いている間に、閉じたら消す印 (`FILE_FLAG_DELETE_ON_CLOSE`) のハンドルを閉じて、
    /// 名前が残ったまま削除を待つ状態を作る。解放と取得が重なったときと同じく、作り直しが `PermissionDenied` で拒まれる。
    #[cfg(windows)]
    #[test]
    fn a_lock_file_being_removed_counts_as_held() {
        use std::os::windows::fs::OpenOptionsExt;
        const DELETE: u32 = 0x0001_0000;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        fs::write(&path, "12345").unwrap();
        let other = File::open(&path).unwrap();
        let removing = OpenOptions::new()
            .access_mode(DELETE)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path)
            .unwrap();
        drop(removing);

        assert!(Lock::try_acquire(dir.path()).unwrap().is_none());
        // 待っても続けば、他のプロセスが持っているのではなく、作れなかったと知らせる。
        let err = Lock::acquire(dir.path(), Duration::from_millis(100)).unwrap_err();
        assert!(matches!(err, LockError::Io { op: "create", .. }), "{err:?}");

        drop(other);
        assert!(Lock::try_acquire(dir.path()).unwrap().is_some());
    }

    #[test]
    fn dropping_without_release_still_frees_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        drop(Lock::try_acquire(dir.path()).unwrap().unwrap());
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn a_lock_older_than_a_minute_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        fs::write(&path, "12345").unwrap();
        let old = SystemTime::now() - Duration::from_secs(61);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(Lock::try_acquire(dir.path()).unwrap().is_some());
    }

    #[test]
    fn a_lock_dated_in_the_future_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        fs::write(&path, "12345").unwrap();
        let future = SystemTime::now() + Duration::from_secs(3600);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(future)
            .unwrap();
        assert!(Lock::try_acquire(dir.path()).unwrap().is_none());
    }

    /// 60 秒を超えて持ち、他のプロセスに取り直された状態を作る。
    /// 相手のプロセス ID は自分と同じにする (PID の空間が別のプロセスとは一致しうる)。
    fn taken_over() -> (tempfile::TempDir, Lock) {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lock::try_acquire(dir.path()).unwrap().unwrap();
        fs::write(dir.path().join(LOCK_FILE), std::process::id().to_string()).unwrap();
        (dir, lock)
    }

    #[test]
    fn releasing_a_lock_taken_over_by_another_process_keeps_theirs() {
        let (dir, lock) = taken_over();
        assert!(matches!(lock.release(), Err(LockError::TakenOver { .. })));
        assert_eq!(
            fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap(),
            std::process::id().to_string()
        );
    }

    #[test]
    fn dropping_a_lock_taken_over_by_another_process_keeps_theirs() {
        let (dir, lock) = taken_over();
        drop(lock);
        assert_eq!(
            fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap(),
            std::process::id().to_string()
        );
    }

    #[test]
    fn a_lock_removed_by_another_process_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lock::try_acquire(dir.path()).unwrap().unwrap();
        fs::remove_file(dir.path().join(LOCK_FILE)).unwrap();
        assert!(matches!(lock.release(), Err(LockError::Removed { .. })));
    }

    #[test]
    fn a_recent_lock_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOCK_FILE), "12345").unwrap();
        assert!(Lock::try_acquire(dir.path()).unwrap().is_none());
    }

    #[test]
    fn acquire_waits_and_then_reports_busy() {
        let dir = tempfile::tempdir().unwrap();
        let _held = Lock::try_acquire(dir.path()).unwrap().unwrap();
        let started = Instant::now();
        let err = Lock::acquire(dir.path(), Duration::from_millis(200)).unwrap_err();
        assert!(matches!(err, LockError::Busy { .. }), "{err:?}");
        assert!(started.elapsed() >= Duration::from_millis(200));
    }

    #[test]
    fn acquire_succeeds_once_the_holder_releases() {
        let dir = tempfile::tempdir().unwrap();
        let held = Lock::try_acquire(dir.path()).unwrap().unwrap();
        let path = dir.path().to_path_buf();
        let waiter = thread::spawn(move || Lock::acquire(&path, Duration::from_secs(5)).map(|l| l.release()));
        thread::sleep(Duration::from_millis(100));
        held.release().unwrap();
        waiter.join().unwrap().unwrap().unwrap();
    }
}
