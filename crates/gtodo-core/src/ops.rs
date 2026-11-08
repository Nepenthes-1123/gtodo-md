//! `data_dir` のファイルを読み、[`crate::logic`] の操作をして書き戻す (Lua 版 `logic/` の各ファイル操作)。
//!
//! ロックは取らない。Neovim 版の自動処理と重ならないよう、呼び出し元が
//! [`crate::lock::Lock`] を持ったまま呼ぶ。

use std::fs::File;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::atomic::{self, WriteError};
use crate::document::Document;
use crate::logic::{due, sort};
use crate::sections::SectionNames;
use crate::store::{self, DataDir, FileKind, ReadError, Stamp};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Write(#[from] WriteError),
    /// 2 ファイルにまたがる移動で、追記した後の削除に失敗した。削除できなかったタスクは両方のファイルにある。
    #[error(
        "the tasks were added to {} but some could not be removed from where they were, so they may appear in both files; remove the duplicates by hand: {source}",
        added.display()
    )]
    PartiallyMoved { added: PathBuf, source: WriteError },
}

/// todo.md の各セクションのタスクを並べ替える。
pub fn sort_todo(data_dir: &DataDir, names: &SectionNames, today: NaiveDate) -> Result<(), OpError> {
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    sort::sort_sections(&mut todo.doc);
    todo.write()?;
    Ok(())
}

/// 期日が来たタスクを Today へ移す ([`due::promote_due`])。
pub fn check_dues(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    auto_move_inbox: bool,
) -> Result<due::DueOutcome, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let outcome = due::promote_due(&mut inbox.doc, &mut todo.doc, names, today, auto_move_inbox);

    if outcome.inbox_changed {
        let added = todo.path.clone();
        append_then_remove(data_dir.root(), added, || todo.write(), || inbox.write())?;
    } else if outcome.todo_changed {
        todo.write()?;
    }
    Ok(outcome)
}

/// 読み込んだファイルと、書き戻すために必要な読んだ時点の状態。
struct Loaded {
    path: PathBuf,
    doc: Document,
    stamp: Option<Stamp>,
    crlf: bool,
}

impl Loaded {
    fn read(data_dir: &DataDir, kind: FileKind, names: &SectionNames, today: NaiveDate) -> Result<Loaded, ReadError> {
        let path = data_dir.path(kind);
        let file = store::read_file(&path)?;
        Ok(Loaded {
            doc: Document::parse(&file.lines, names, today),
            path,
            stamp: file.stamp,
            crlf: file.crlf,
        })
    }

    fn write(&mut self) -> Result<(), WriteError> {
        let lines = self.doc.serialize();
        self.stamp = Some(atomic::write_lines(&self.path, &lines, self.crlf, self.stamp.as_ref())?);
        Ok(())
    }
}

/// 2 ファイルにまたがる移動を、追記してから削除する順で書く (Lua 版 `logic/write_pair.lua`)。
///
/// 2 ファイルを同時に置き換える手段は無く、途中での失敗は避けられない。追記が先なら、追記に
/// 失敗しても何も変わらず、削除に失敗してもタスクは両方に残る (重複は目で見て消せる)。
/// 削除が先だと、後の追記に失敗したときタスクがどちらのファイルにも残らない。
fn append_then_remove<A, R, EA, ER>(dir: &Path, added: PathBuf, append: A, remove: R) -> Result<(), OpError>
where
    A: FnOnce() -> Result<(), EA>,
    R: FnOnce() -> Result<(), ER>,
    EA: Into<OpError>,
    ER: Into<OpError>,
{
    append().map_err(Into::into)?;
    sync_dir(dir);
    remove().map_err(|e| match e.into() {
        OpError::Write(source) => OpError::PartiallyMoved { added, source },
        other => other,
    })
}

/// 追記と削除の 2 回の rename がディスクに残る順序を固定する。順序はファイルシステム任せで、
/// 削除だけが先に残った状態でクラッシュすると、タスクがどちらのファイルにも無くなる。
///
/// Windows にはディレクトリを fsync する手段が無いため何もしない (Lua 版と同じく、順序は保証されない)。
/// 失敗しても続ける (同時にクラッシュしない限り害が無いため。Lua 版と同じ)。
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    /// `name` を、書き込めないディレクトリにある実体へのリンクにする (読めるが、書くと失敗する)。
    fn make_unwritable(dir: &Path, name: &str, content: &str) {
        let ro = dir.join(format!("ro-{name}"));
        fs::create_dir(&ro).unwrap();
        fs::write(ro.join(name), content).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
        symlink(ro.join(name), dir.join(name)).unwrap();
    }

    fn restore_permissions(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() && !path.is_symlink() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
    }

    const INBOX: &str = "# Inbox\n- [ ] 受信 due:2026-02-01 id:aaaaaa\n";

    fn run(dir: &Path, op: fn(&DataDir) -> Result<(), OpError>) -> Result<(), OpError> {
        let result = op(&DataDir::new(dir));
        restore_permissions(dir);
        result
    }

    fn check_dues(d: &DataDir) -> Result<(), OpError> {
        super::check_dues(d, &SectionNames::default(), today(), true).map(drop)
    }

    #[test]
    fn a_due_task_stays_in_the_inbox_when_todo_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inbox.md"), INBOX).unwrap();
        make_unwritable(dir.path(), "todo.md", "# Todo\n");

        assert!(matches!(run(dir.path(), check_dues), Err(OpError::Write(_))));
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), INBOX);
    }

    #[test]
    fn a_due_task_is_left_in_both_files_when_the_inbox_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        make_unwritable(dir.path(), "inbox.md", INBOX);
        fs::write(dir.path().join("todo.md"), "# Todo\n").unwrap();

        let err = run(dir.path(), check_dues).unwrap_err();
        assert!(
            matches!(&err, OpError::PartiallyMoved { added, .. } if added.ends_with("todo.md")),
            "{err}"
        );
        assert!(
            fs::read_to_string(dir.path().join("todo.md"))
                .unwrap()
                .contains("id:aaaaaa")
        );
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), INBOX);
    }

    #[test]
    fn nothing_is_written_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let todo = "# Todo\n\n## Today\n\n- [ ] A id:aaaaaa\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        let before = Stamp::of(&dir.path().join("todo.md")).unwrap();

        let data_dir = DataDir::new(dir.path());
        let names = SectionNames::default();
        sort_todo(&data_dir, &names, today()).unwrap();
        assert_eq!(super::check_dues(&data_dir, &names, today(), true).unwrap().moved, 0);

        assert_eq!(Stamp::of(&dir.path().join("todo.md")).unwrap(), before);
        assert!(!dir.path().join("inbox.md").exists());
        assert!(!dir.path().join("done.md").exists());
    }
}
