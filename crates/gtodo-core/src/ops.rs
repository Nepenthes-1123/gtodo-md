//! `data_dir` のファイルを読み、[`crate::logic`] の操作をして書き戻す (Lua 版 `logic/` の各ファイル操作)。
//!
//! ロックは取らない。Neovim 版の自動処理と重ならないよう、呼び出し元が
//! [`crate::lock::Lock`] を持ったまま呼ぶ。

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::atomic::{self, WriteError};
use crate::document::{Document, Item, TaskItem};
use crate::logic::{completion, due, history, sort};
use crate::sections::{SectionKey, SectionNames};
use crate::store::{self, DataDir, FileKind, ReadError, Stamp};
use crate::task::{self, Task};

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

/// 無いファイルを Lua 版 `io.ensure_files` と同じ雛形で作る。
///
/// 雛形が無いまま他の操作をすると、見出しの無いファイルができる。Neovim 版はファイルがあれば
/// 雛形を作らないため、`# Todo` やセクションの見出しが欠けたまま残る。
pub fn ensure_files(data_dir: &DataDir, names: &SectionNames) -> Result<(), WriteError> {
    for kind in [FileKind::Inbox, FileKind::Todo, FileKind::Done, FileKind::Cancelled] {
        let path = data_dir.path(kind);
        let exists = Stamp::of(&path)
            .map_err(|source| WriteError::Io {
                op: "check",
                path: path.clone(),
                source,
            })?
            .is_some();
        if !exists {
            match atomic::write_lines(&path, &template(kind, names), false, None) {
                // 確かめた後に Neovim 版が作った。Neovim 版は雛形をロックを取らずに作る。
                Ok(_) | Err(WriteError::Changed { .. }) => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

fn template(kind: FileKind, names: &SectionNames) -> Vec<String> {
    let title = |t: &str| vec![t.to_string(), String::new()];
    match kind {
        FileKind::Inbox => title("# Inbox"),
        FileKind::Done => title("# Done"),
        FileKind::Cancelled => title("# Cancelled"),
        // Lua 版はこのファイルだけ末尾に空行を付けない。
        FileKind::Todo => std::iter::once("# Todo".to_string())
            .chain(
                SectionKey::ALL
                    .into_iter()
                    .flat_map(|k| [String::new(), format!("## {}", names.name(k))]),
            )
            .collect(),
    }
}

/// タスクを inbox.md の、最初の `##` 見出しより前の末尾へ追加し、付けた ID を返す
/// (Lua 版 `init.lua` の `add_or_edit_task` の新規追加)。
pub fn add_task(data_dir: &DataDir, names: &SectionNames, today: NaiveDate, mut task: Task) -> Result<String, OpError> {
    let mut file = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    // 書き出すときにファイル内の重複として振り直されると、利用者に知らせた ID と食い違うため、
    // 使われていない ID を先に決める。inbox のタスクは期日が来ると todo.md へ移るので、両方を避ける。
    let other = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let used: HashSet<&str> = [&file.doc, &other.doc]
        .into_iter()
        .flat_map(|doc| doc.tasks().filter_map(|(_, t)| t.task.id.as_deref()))
        .collect();
    let id = loop {
        let id = task::generate_id();
        if !used.contains(id.as_str()) {
            break id;
        }
    };
    task.id = Some(id.clone());
    file.doc.preamble.push(Item::Task(Box::new(TaskItem {
        line: task.clone().serialize(),
        task,
        lnum: 0,
    })));
    file.write()?;
    Ok(id)
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

/// 完了したタスクを done.md の完了月の見出しへ移す ([`completion::take_completed`])。移した数を返す。
pub fn move_completed(data_dir: &DataDir, names: &SectionNames, today: NaiveDate) -> Result<usize, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let mut completed = completion::take_completed(&mut inbox.doc, &mut todo.doc, names, today);
    let count = completed.count();
    if count == 0 {
        return Ok(0);
    }

    let done_path = data_dir.path(FileKind::Done);
    let append = || -> Result<(), OpError> {
        let done = store::read_file(&done_path)?;
        let mut lines = done.lines;
        for (month, tasks) in &mut completed.months {
            history::insert_tasks(&mut lines, "Done", month, tasks);
        }
        atomic::write_lines(&done_path, &lines, done.crlf, done.stamp.as_ref())?;
        Ok(())
    };
    let remove = || -> Result<(), OpError> {
        if completed.inbox_changed {
            inbox.write()?;
        }
        if completed.todo_changed {
            todo.write()?;
        }
        Ok(())
    };
    append_then_remove(data_dir.root(), done_path.clone(), append, remove)?;
    Ok(count)
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
    const TODO: &str = "# Todo\n\n## Today\n\n- [x] 完了 completed_at:2026-02-01 id:bbbbbb\n";

    fn run(dir: &Path, op: fn(&DataDir) -> Result<(), OpError>) -> Result<(), OpError> {
        let result = op(&DataDir::new(dir));
        restore_permissions(dir);
        result
    }

    fn check_dues(d: &DataDir) -> Result<(), OpError> {
        super::check_dues(d, &SectionNames::default(), today(), true).map(drop)
    }

    fn move_completed(d: &DataDir) -> Result<(), OpError> {
        super::move_completed(d, &SectionNames::default(), today()).map(drop)
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
    fn a_completed_task_stays_in_todo_when_done_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("todo.md"), TODO).unwrap();
        make_unwritable(dir.path(), "done.md", "# Done\n");

        assert!(matches!(run(dir.path(), move_completed), Err(OpError::Write(_))));
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), TODO);
    }

    #[test]
    fn a_completed_task_is_left_in_both_files_when_todo_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        make_unwritable(dir.path(), "todo.md", TODO);

        let err = run(dir.path(), move_completed).unwrap_err();
        assert!(
            matches!(&err, OpError::PartiallyMoved { added, .. } if added.ends_with("done.md")),
            "{err}"
        );
        assert!(
            fs::read_to_string(dir.path().join("done.md"))
                .unwrap()
                .contains("id:bbbbbb")
        );
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), TODO);
    }

    #[test]
    fn crlf_line_endings_are_kept_in_every_written_file() {
        let dir = tempfile::tempdir().unwrap();
        let crlf = |s: &str| s.replace('\n', "\r\n");
        fs::write(dir.path().join("inbox.md"), crlf(INBOX)).unwrap();
        fs::write(dir.path().join("todo.md"), crlf(TODO)).unwrap();
        fs::write(dir.path().join("done.md"), crlf("# Done\n")).unwrap();

        let data_dir = DataDir::new(dir.path());
        check_dues(&data_dir).unwrap();
        move_completed(&data_dir).unwrap();

        for file in ["inbox.md", "todo.md", "done.md"] {
            let text = fs::read_to_string(dir.path().join(file)).unwrap();
            assert_eq!(
                text.matches('\n').count(),
                text.matches("\r\n").count(),
                "{file}: {text:?}"
            );
        }
        let todo = fs::read_to_string(dir.path().join("todo.md")).unwrap();
        assert!(todo.contains("id:aaaaaa") && !todo.contains("id:bbbbbb"), "{todo:?}");
        assert!(
            fs::read_to_string(dir.path().join("done.md"))
                .unwrap()
                .contains("id:bbbbbb")
        );
    }

    fn new_task(content: &str) -> Task {
        Task {
            content: content.into(),
            created: Some("2026-02-03".into()),
            ..Task::default()
        }
    }

    #[test]
    fn added_inbox_tasks_go_after_the_last_task_before_any_heading() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("inbox.md"),
            "# Inbox\n\n- [ ] 既存 id:aaaaaa\n\n## メモ\n\n- [ ] 見出しの下 id:bbbbbb\n",
        )
        .unwrap();
        let id = add_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            new_task("追加"),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("inbox.md")).unwrap(),
            format!(
                "# Inbox\n\n- [ ] 既存 id:aaaaaa\n- [ ] 追加 created:2026-02-03 id:{id}\n\n\
                 ## メモ\n\n- [ ] 見出しの下 id:bbbbbb\n"
            )
        );
    }

    /// 期待値は Lua 版 `io.ensure_files` が書く内容 (`title .. "\n"`)。
    #[test]
    fn missing_files_are_created_from_the_same_templates_as_lua() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("done.md"), "既存").unwrap();
        let names = SectionNames::from_last(&[Some("今日".into()), None, None, None]).unwrap();
        ensure_files(&DataDir::new(dir.path()), &names).unwrap();

        let read = |name: &str| fs::read_to_string(dir.path().join(name)).unwrap();
        assert_eq!(read("inbox.md"), "# Inbox\n\n");
        assert_eq!(
            read("todo.md"),
            "# Todo\n\n## 今日\n\n## Next\n\n## Waiting\n\n## Someday\n"
        );
        assert_eq!(read("done.md"), "既存");
        assert_eq!(read("cancelled.md"), "# Cancelled\n\n");
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
        assert_eq!(super::move_completed(&data_dir, &names, today()).unwrap(), 0);

        assert_eq!(Stamp::of(&dir.path().join("todo.md")).unwrap(), before);
        assert!(!dir.path().join("inbox.md").exists());
        assert!(!dir.path().join("done.md").exists());
    }
}
