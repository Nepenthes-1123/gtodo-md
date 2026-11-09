//! ファイルのアトミックな置き換え。Lua 版 `io.lua` の `atomic_replace` と同じ手順で書く。
//!
//! 同じ `data_dir` を Neovim 版と同時に使うため、次を守る。
//! - 読んだ時点からファイルが変わっていれば、何も書かずにエラーにする (相手の変更を全行置換で潰さない)。
//! - 内容が変わらなければ書かない (mtime を動かすと、Neovim 側の全インスタンスがリロードする)。
//! - 同じディレクトリの一時ファイルに書いて fsync してから rename で置き換える (途中で壊れたファイルを見せない)。
//!
//! 変更の照合は検出であって防止ではない (Lua 版と同じ)。照合から rename までの間に他のプロセスが
//! 書いたり作ったりした内容は検出できず、上書きされる。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::store::Stamp;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{} was changed by another process after it was read; nothing was written", path.display())]
    Changed { path: PathBuf },
    #[error(
        "refusing to write {}: it would remove headings that were there when the file was read: {missing} (this is a bug in gtodo)",
        path.display()
    )]
    WouldRemove { path: PathBuf, missing: String },
    #[error("failed to {op} {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

/// 行を書き戻す。`crlf` は読んだときの改行 ([`crate::store::ReadFile::crlf`])。
///
/// `base` は読んだ時点の状態 ([`crate::store::ReadFile::stamp`]。`None` はファイルが無かったこと)。
/// 書いた後の状態を返す。同じコマンドの中で続けて書くときは、それを次の `base` にする。
pub fn write_lines(path: &Path, lines: &[String], crlf: bool, base: Option<&Stamp>) -> Result<Stamp, WriteError> {
    let nl = if crlf { "\r\n" } else { "\n" };
    let mut data = String::new();
    for line in lines {
        data.push_str(line);
        data.push_str(nl);
    }
    write_bytes(path, data.as_bytes(), base)
}

/// 任意の内容を書く。`base` と戻り値は [`write_lines`] と同じ。
pub fn write_bytes(path: &Path, data: &[u8], base: Option<&Stamp>) -> Result<Stamp, WriteError> {
    let io_err = |op, source| WriteError::Io {
        op,
        path: path.to_path_buf(),
        source,
    };
    let current = Stamp::of(path).map_err(|e| io_err("stat", e))?;
    match (base, current) {
        (Some(base), Some(current)) if *base != current => {
            return Err(WriteError::Changed {
                path: path.to_path_buf(),
            });
        }
        // 読んだ時点で無かったファイルが作られていたら、作った側の内容を潰さない。
        // Lua 版はこの場合を照合せずに上書きする。
        (None, Some(_)) => {
            return Err(WriteError::Changed {
                path: path.to_path_buf(),
            });
        }
        // 読んだ後に消されていた場合は、Lua 版と同じく新規作成として書く。
        _ => {}
    }

    let target = resolve_link(path).map_err(|e| io_err("resolve the symbolic link", e))?;
    if content_unchanged(&target, data).map_err(|e| io_err("read", e))? {
        // 照合の後に他のプロセスが同じ内容で作った場合。照合の時点で作られていた場合と同じく扱う。
        return current.ok_or_else(|| WriteError::Changed {
            path: path.to_path_buf(),
        });
    }
    replace(&target, data).map_err(|(op, e)| io_err(op, e))
}

/// シンボリックリンクは実体へ解決する。リンクそのものを rename の置換先にすると、
/// リンクが実体ファイルに置き換わって消え、利用者が作った同期や共有の仕組みが壊れる。
///
/// 実体の無いリンクはエラーにする。Lua 版はこの場合リンクを通常のファイルに置き換えるが、
/// 同期先が一時的に見えないだけかもしれず、リンクを壊すよりは書かない方が安全。
fn resolve_link(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path),
        Ok(_) => Ok(path.to_path_buf()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(e) => Err(e),
    }
}

fn content_unchanged(target: &Path, data: &[u8]) -> io::Result<bool> {
    match fs::metadata(target) {
        Ok(meta) if meta.len() != data.len() as u64 => return Ok(false),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    Ok(fs::read(target)? == data)
}

/// 置き換えた後の状態を返す。rename の後に置換先を取り直すと、その間に他のプロセスが書いた結果を
/// 自分の書き込みとして返してしまうため、rename の前に一時ファイルから取る (rename では変わらない)。
fn replace(target: &Path, data: &[u8]) -> Result<Stamp, (&'static str, io::Error)> {
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let base = target.file_name().ok_or((
        "write",
        io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"),
    ))?;
    // 一時ファイルは置換先と同じディレクトリに作る。別のファイルシステムだと rename が失敗するため。
    let (file, tmp) = create_tmp(dir, &base.to_string_lossy()).map_err(|e| ("create a temporary file for", e))?;
    let result = write_and_stamp(file, data, target, &tmp).and_then(|stamp| {
        rename_with_retry(&tmp, target)
            .map(|()| stamp)
            .map_err(|e| ("rename", e))
    });
    if result.is_err() {
        // 元のエラーを返すことが優先。一時ファイルの削除に失敗しても、残骸は sweep_stale_tmp が後で消す。
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 一時ファイルに書き、権限を整えて閉じる。rename の前に閉じるのは Lua 版と同じ
/// (Windows で開いたままのファイルの置き換えが拒まれる余地を残さない)。
fn write_and_stamp(mut file: File, data: &[u8], target: &Path, tmp: &Path) -> Result<Stamp, (&'static str, io::Error)> {
    write_and_sync(&mut file, data).map_err(|e| ("write", e))?;
    inherit_permissions(target, tmp).map_err(|e| ("stat", e))?;
    // 閉じてからパスで取る。Windows では書き込んだハンドルを閉じるときに更新日時が動きうるため、
    // 閉じる前の値を返すと、次の書き込みが自分の書き込みを他人の変更と取り違える。
    drop(file);
    let meta = fs::metadata(tmp).map_err(|e| ("stat", e))?;
    Stamp::from_metadata(&meta).map_err(|e| ("stat", e))
}

/// 一時ファイル名の連番。プロセス ID と組み合わせて衝突を減らすためのもので、
/// 正しさは排他作成 (`create_new`) が担う。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn create_tmp(dir: &Path, base: &str) -> io::Result<(File, PathBuf)> {
    let mut last_err = None;
    for _ in 0..3 {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
        let path = dir.join(format!(".{base}.{}.{seq}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()))
}

/// fsync を挟まないと、ファイルシステムによっては rename だけが先に永続化され、
/// 直後のクラッシュで中身が空のファイルが残る。
fn write_and_sync(file: &mut File, data: &[u8]) -> io::Result<()> {
    file.write_all(data)?;
    file.sync_all()
}

/// 一時ファイルは 0600 で作るため、既存ファイルの権限を引き継がないと置き換えで権限が落ちる。
/// 「ファイルが無い (新規作成)」以外の stat の失敗は、権限を落としたまま確定させないよう失敗にする。
fn inherit_permissions(target: &Path, tmp: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(meta) => {
            // chmod を受け付けないファイルシステム (一部の FUSE など) では失敗するが、Lua 版と同じく書き込みは続ける。
            // 止めると、Neovim 版では書けるのに CLI だけ書けなくなる。
            let _ = fs::set_permissions(tmp, meta.permissions());
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// rename が一時的に弾かれたときの再試行間隔 (Lua 版と同じ。合計 630ms)。
/// Windows では他のプロセスがファイルを開いている間、置き換えが共有違反で失敗しうる。
const RENAME_BACKOFF: [Duration; 6] = [
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(160),
    Duration::from_millis(320),
];

fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut backoff = RENAME_BACKOFF.iter();
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if is_retryable(&e) => match backoff.next() {
                Some(wait) => thread::sleep(*wait),
                None => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
}

/// 待てば結果が変わりうるエラーだけを再試行する (Lua 版の EACCES / EBUSY / EPERM)。
/// 容量不足などは何度試しても同じなので、すぐに失敗させる。
fn is_retryable(e: &io::Error) -> bool {
    // Windows の ERROR_SHARING_VIOLATION (32) と ERROR_LOCK_VIOLATION (33) は ErrorKind に対応が無い。
    const WINDOWS_SHARING_ERRORS: [i32; 2] = [32, 33];
    matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ResourceBusy)
        || (cfg!(windows) && e.raw_os_error().is_some_and(|c| WINDOWS_SHARING_ERRORS.contains(&c)))
}

/// 残骸とみなすまでの猶予。書き込み中の一時ファイルの寿命は通常ミリ秒単位なので、
/// 短くすると、止まっている他のプロセスの書き込み途中のファイルを消してしまう。
const TMP_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// クラッシュなどで残った一時ファイル (`.<名前>.tmp`) を消す。
///
/// 判定は更新時刻だけで行い、名前に含まれるプロセス ID は見ない
/// (プロセス ID は再利用されるため、持ち主の識別に使えない)。
/// ディレクトリが無ければ何もしない。個々のエントリを読めなくても、残りの掃除は続ける (Lua 版と同じ)。
pub fn sweep_stale_tmp(dir: &Path, now: SystemTime) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with('.') && name.ends_with(".tmp") && name.len() > ".tmp".len() + 1) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > TMP_TTL);
        if meta.is_file() && stale {
            // 他のプロセスが同時に消した場合などは失敗してよい。次の機会にまた掃除される。
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::read_file;

    fn lines(ls: &[&str]) -> Vec<String> {
        ls.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn writes_lines_with_the_original_line_ending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "a\r\n").unwrap();
        let file = read_file(&path).unwrap();
        write_lines(&path, &lines(&["a", "b"]), file.crlf, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"a\r\nb\r\n");

        let file = read_file(&path).unwrap();
        write_lines(&path, &[], file.crlf, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"");
    }

    #[test]
    fn creates_a_file_that_did_not_exist_when_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("done.md");
        let file = read_file(&path).unwrap();
        let stamp = write_lines(&path, &lines(&["# Done"]), file.crlf, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Done\n");
        assert_eq!(Some(stamp), Stamp::of(&path).unwrap());
    }

    #[test]
    fn a_file_changed_after_reading_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "original\n").unwrap();
        let file = read_file(&path).unwrap();
        fs::write(&path, "changed by neovim, longer\n").unwrap();

        let err = write_lines(&path, &lines(&["mine"]), false, file.stamp.as_ref()).unwrap_err();
        assert!(matches!(err, WriteError::Changed { .. }), "{err:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed by neovim, longer\n");
    }

    #[test]
    fn a_change_that_keeps_the_size_is_still_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "- [ ] a\n").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let file = read_file(&path).unwrap();
        // Neovim 側でチェックボックスを切り替えた場合。サイズは変わらない。
        fs::write(&path, "- [x] a\n").unwrap();

        let err = write_lines(&path, &lines(&["- [ ] b"]), false, file.stamp.as_ref()).unwrap_err();
        assert!(matches!(err, WriteError::Changed { .. }), "{err:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "- [x] a\n");
    }

    #[test]
    fn a_file_created_after_reading_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("done.md");
        let file = read_file(&path).unwrap();
        fs::write(&path, "# Done\nby neovim\n").unwrap();

        let err = write_lines(&path, &lines(&["# Done"]), false, file.stamp.as_ref()).unwrap_err();
        assert!(matches!(err, WriteError::Changed { .. }), "{err:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Done\nby neovim\n");
    }

    #[test]
    fn a_file_deleted_after_reading_is_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "a\n").unwrap();
        let file = read_file(&path).unwrap();
        fs::remove_file(&path).unwrap();
        write_lines(&path, &lines(&["b"]), false, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "b\n");
    }

    #[test]
    fn unchanged_content_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "a\n").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let file = read_file(&path).unwrap();

        let stamp = write_lines(&path, &lines(&["a"]), false, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old);
        assert_eq!(Some(stamp), file.stamp);
    }

    #[test]
    fn the_returned_stamp_allows_writing_again_in_the_same_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        let file = read_file(&path).unwrap();
        let stamp = write_lines(&path, &lines(&["1"]), false, file.stamp.as_ref()).unwrap();
        write_lines(&path, &lines(&["2"]), false, Some(&stamp)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "2\n");
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        write_lines(&path, &lines(&["a"]), false, None).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["todo.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "a\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let file = read_file(&path).unwrap();
        write_lines(&path, &lines(&["b"]), false, file.stamp.as_ref()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_kept_and_its_target_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.md");
        let link = dir.path().join("todo.md");
        fs::write(&real, "a\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let file = read_file(&link).unwrap();
        write_lines(&link, &lines(&["b"]), false, file.stamp.as_ref()).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "b\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_change_to_the_target_of_a_symlink_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.md");
        let link = dir.path().join("todo.md");
        fs::write(&real, "a\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let file = read_file(&link).unwrap();
        fs::write(&real, "changed\n").unwrap();
        let err = write_lines(&link, &lines(&["b"]), false, file.stamp.as_ref()).unwrap_err();
        assert!(matches!(err, WriteError::Changed { .. }), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_without_a_target_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("done.md");
        std::os::unix::fs::symlink(dir.path().join("missing/real.md"), &link).unwrap();
        let file = read_file(&link).unwrap();
        let err = write_lines(&link, &lines(&["# Done"]), false, file.stamp.as_ref()).unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    #[test]
    fn only_transient_errors_are_retried() {
        assert!(is_retryable(&io::ErrorKind::PermissionDenied.into()));
        assert!(is_retryable(&io::ErrorKind::ResourceBusy.into()));
        assert!(!is_retryable(&io::ErrorKind::StorageFull.into()));
        assert!(!is_retryable(&io::ErrorKind::NotFound.into()));
    }

    #[test]
    fn a_failed_replacement_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // ファイルで置き換えられない (空でないディレクトリ) ため、一時ファイルを書いた後の rename で失敗する。
        let target = dir.path().join("todo.md");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), "a\n").unwrap();
        let base = Stamp::of(&target).unwrap();
        let result = write_bytes(&target, b"b\n", base.as_ref());
        assert!(matches!(result, Err(WriteError::Io { op: "rename", .. })), "{result:?}");
        assert_eq!(fs::read_to_string(target.join("keep")).unwrap(), "a\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_in_a_read_only_directory_is_left_as_is() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        fs::write(&path, "a\n").unwrap();
        let file = read_file(&path).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_lines(&path, &lines(&["b"]), false, file.stamp.as_ref());
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        // root で実行すると権限が無視されて書けてしまうため、その環境では検証できない。
        if result.is_ok() {
            return;
        }
        assert!(matches!(result, Err(WriteError::Io { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn sweeping_leaves_directories_alone_and_ignores_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".dir.tmp")).unwrap();
        let later = SystemTime::now() + Duration::from_secs(48 * 60 * 60);
        sweep_stale_tmp(dir.path(), later).unwrap();
        assert!(dir.path().join(".dir.tmp").is_dir());
        sweep_stale_tmp(&dir.path().join("missing"), later).unwrap();
    }

    #[test]
    fn only_stale_temporary_files_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for (name, age_hours) in [
            (".todo.md.1.1.tmp", 25),
            (".todo.md.1.2.tmp", 1),
            ("todo.md", 48),
            (".tmp", 48),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, "").unwrap();
            let mtime = now - Duration::from_secs(age_hours * 60 * 60);
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }
        sweep_stale_tmp(dir.path(), now).unwrap();
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [".tmp", ".todo.md.1.2.tmp", "todo.md"]);
    }
}
