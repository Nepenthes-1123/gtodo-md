//! `data_dir` のファイルを読み、[`crate::logic`] の操作をして書き戻す (Lua 版 `logic/` の各ファイル操作)。
//!
//! ロックは取らない。Neovim 版の自動処理と重ならないよう、呼び出し元が
//! [`crate::lock::Lock`] を持ったまま呼ぶ。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::atomic::{self, WriteError};
use crate::document::{Document, Item, Section, TaskItem};
use crate::guard::Landmarks;
use crate::key::{self, FindError};
use crate::logic::{completion, due, history, sort};
use crate::sections::{SectionKey, SectionNames};
use crate::store::{self, DataDir, FileKind, ReadError, Stamp};
use crate::task::{self, Status, Task};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Find(#[from] FindError),
    #[error("{key:?} is in {}; tasks moved there cannot be changed", file.file_name())]
    Archived { key: String, file: FileKind },
    #[error("{0} is already completed")]
    AlreadyCompleted(String),
    #[error("{0} is not completed")]
    NotCompleted(String),
    #[error("{0} is completed; completed tasks cannot be moved")]
    MoveCompleted(String),
    /// 書き換えた行を読み直すと別のタスクになる (本文の先頭の `(C)` が、優先度を消すと優先度として読まれるなど)。
    #[error("the edited task would be read back differently, so it was not written: {0}")]
    Unwritable(String),
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

/// タスクの追加先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// inbox.md の、最初の `##` 見出しより前の末尾。
    Inbox,
    /// todo.md のセクションの末尾。
    Section(SectionKey),
}

/// タスクを追加し、付けた ID を返す (Lua 版 `init.lua` の `add_or_edit_task` の新規追加)。
/// todo.md へ加えたら並べ替える。inbox.md への追加後の期日のチェックは呼び出し元が行う。
///
/// セクションの見出しが無ければ末尾に作る。Lua 版は見出しの無いセクションへ加えたタスクを
/// 書き出さずに失う。
pub fn add_task(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    mut task: Task,
    to: Destination,
) -> Result<String, OpError> {
    let kind = match to {
        Destination::Inbox => FileKind::Inbox,
        Destination::Section(_) => FileKind::Todo,
    };
    let mut file = Loaded::read(data_dir, kind, names, today)?;
    // 書き出すときにファイル内の重複として振り直されると、利用者に知らせた ID と食い違うため、
    // 使われていない ID を先に決める。inbox のタスクは期日が来ると todo.md へ移るので、両方を避ける。
    let other = match kind {
        FileKind::Inbox => FileKind::Todo,
        _ => FileKind::Inbox,
    };
    let other = Loaded::read(data_dir, other, names, today)?;
    let id = unused_id(&[&file.doc, &other.doc]);
    task.id = Some(id.clone());
    let item = Item::Task(Box::new(TaskItem {
        line: task.clone().serialize(),
        task,
        lnum: 0,
    }));

    match to {
        Destination::Inbox => file.doc.preamble.push(item),
        Destination::Section(key) => {
            section_items(&mut file.doc, names.name(key)).push(item);
            sort::sort_sections(&mut file.doc);
        }
    }
    file.write()?;
    Ok(id)
}

/// 名前のセクションの項目を返す。見出しが無ければ末尾に作る。
fn section_items<'a>(doc: &'a mut Document, name: &str) -> &'a mut Vec<Item> {
    let i = match doc.sections.iter().position(|s| s.name == name) {
        Some(i) => i,
        None => {
            doc.sections.push(Section {
                name: name.to_string(),
                items: Vec::new(),
            });
            doc.sections.len() - 1
        }
    };
    &mut doc.sections[i].items
}

/// どの文書でも使われていない ID を発行する。
fn unused_id(docs: &[&Document]) -> String {
    let used: HashSet<&str> = docs
        .iter()
        .flat_map(|doc| doc.tasks().filter_map(|(_, t)| t.task.id.as_deref()))
        .collect();
    loop {
        let id = task::generate_id();
        if !used.contains(id.as_str()) {
            return id;
        }
    }
}

/// キーで指定したタスクを完了にする (Lua 版 `editor.lua` の完了トグル)。完了にしたタスクを返す。
pub fn complete(data_dir: &DataDir, names: &SectionNames, today: NaiveDate, key: &str) -> Result<Task, OpError> {
    set_status(data_dir, names, today, key, Status::Done)
}

/// キーで指定した完了済みのタスクを未完了に戻す (Lua 版 `editor.lua` の完了トグル)。戻したタスクを返す。
///
/// done.md へ移った後のタスクは対象にしない (Lua 版も done.md からは戻さない)。
pub fn reopen(data_dir: &DataDir, names: &SectionNames, today: NaiveDate, key: &str) -> Result<Task, OpError> {
    set_status(data_dir, names, today, key, Status::Open)
}

/// 操作の対象を inbox.md と todo.md から探す。見つからなければ done.md と cancelled.md も探し、
/// そこにあれば、`gtodo list` し直すよう案内する代わりにその旨を伝える (日付が変わった後の
/// 日次の整理で、完了にしたタスクが done.md へ移っていることがある)。
fn locate(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    inbox: &Document,
    todo: &Document,
    key: &str,
) -> Result<key::Location, OpError> {
    match key::find(&[(FileKind::Inbox, inbox), (FileKind::Todo, todo)], key) {
        // 短すぎるキーも、手で書いた短い ID と完全に一致するタスクが移った後かもしれない。
        Err(e @ (FindError::NotFound(_) | FindError::LineChanged(_) | FindError::TooShort(_))) => {
            for kind in [FileKind::Done, FileKind::Cancelled] {
                let archive = Loaded::read(data_dir, kind, names, today)?;
                if !matches!(
                    key::find(&[(kind, &archive.doc)], key),
                    Err(FindError::NotFound(_) | FindError::LineChanged(_) | FindError::TooShort(_))
                ) {
                    return Err(OpError::Archived {
                        key: key.to_string(),
                        file: kind,
                    });
                }
            }
            Err(e.into())
        }
        other => Ok(other?),
    }
}

fn set_status(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    key: &str,
    status: Status,
) -> Result<Task, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let at = locate(data_dir, names, today, &inbox.doc, &todo.doc, key)?;
    let id = unused_id(&[&inbox.doc, &todo.doc]);
    let file = match at.file {
        FileKind::Inbox => &mut inbox,
        _ => &mut todo,
    };

    let items = file.items_mut(at.section);
    let Item::Task(item) = &mut items[at.index] else {
        unreachable!("key::find only returns tasks");
    };
    match (item.task.status, status) {
        (Status::Done, Status::Done) => return Err(OpError::AlreadyCompleted(key.to_string())),
        (Status::Open, Status::Open) => return Err(OpError::NotCompleted(key.to_string())),
        _ => {}
    }
    let task = &mut item.task;
    task.status = status;
    task.completed_at = match status {
        Status::Done => Some(today.format("%Y-%m-%d").to_string()),
        Status::Open => None,
    };
    // 書き出すときにファイル内の重複として振り直されると、利用者に知らせた ID と食い違うため、先に決める。
    task.id.get_or_insert(id);
    let result = task.clone();
    let (lnum, line) = (item.lnum, task.clone().serialize());

    match at.file {
        // Lua 版と同じく、todo.md では書き戻す前にそのセクションだけを並べ替える。
        FileKind::Todo => {
            sort::sort_items(items);
            file.write()?;
        }
        // Lua 版はバッファの 1 行だけを書き換えて保存する。ファイル全体を整形し直すと、
        // 開いている Neovim のバッファの他の行まで書き換わる。
        _ => {
            file.lines[lnum - 1] = line;
            file.write_lines()?;
        }
    }
    Ok(result)
}

/// キーで指定したタスクを cancelled.md の今月の見出しへ移す (Lua 版 `editor.lua` の
/// `cancel_current_task`)。移したタスクを返す。
pub fn cancel(data_dir: &DataDir, names: &SectionNames, today: NaiveDate, key: &str) -> Result<Task, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let at = locate(data_dir, names, today, &inbox.doc, &todo.doc, key)?;
    let id = unused_id(&[&inbox.doc, &todo.doc]);
    let file = match at.file {
        FileKind::Inbox => &mut inbox,
        _ => &mut todo,
    };
    let Item::Task(item) = file.items_mut(at.section).remove(at.index) else {
        unreachable!("key::find only returns tasks");
    };
    let mut task = item.task;
    task.cancelled = Some(today.format("%Y-%m-%d").to_string());
    task.id.get_or_insert(id);
    let result = task.clone();

    let cancelled_path = data_dir.path(FileKind::Cancelled);
    let append = || -> Result<(), OpError> {
        let cancelled = store::read_file(&cancelled_path)?;
        let mut lines = cancelled.lines.clone();
        // 完了タスクと違い、タスクの日付ではなくキャンセルした日の月へ入れる (Lua 版と同じ)。
        let month = today.format("%Y-%m").to_string();
        history::insert_tasks(&mut lines, "Cancelled", &month, &mut [task]);
        write_history(&cancelled_path, FileKind::Cancelled, &cancelled, &lines, names)
    };
    let remove = || -> Result<(), OpError> {
        match at.file {
            FileKind::Todo => file.write()?,
            // set_status で 1 行だけを書き換えるのと同じ理由。
            _ => {
                file.lines.remove(item.lnum - 1);
                file.write_lines()?;
            }
        }
        Ok(())
    };
    append_then_remove(data_dir.root(), cancelled_path.clone(), append, remove)?;
    Ok(result)
}

/// [`move_task`] で、移動先が Waiting のときの待ち先 (`wait:`) の扱い。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// 今の待ち先を残す (Neovim 版で、待ち先の入力欄を初期値のまま確定したときと同じ)。
    Keep,
    Set(String),
    /// 待ち先を消す (Neovim 版で、入力欄を空にして確定したときと同じ)。
    Remove,
}

/// [`move_task`] の結果。どれも操作したタスクを持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    /// 別のセクションへ移した。
    Moved(Task),
    /// Waiting にあるタスクの待ち先だけを変えた。
    WaitChanged(Task),
    /// すでに移動先のセクションにあり、何も書かなかった。
    AlreadyThere(Task),
}

/// キーで指定したタスクを todo.md のセクションへ移す (Lua 版 `editor.lua` の
/// `move_current_task_to`)。
///
/// 移動先が Waiting でなければ待ち先を消す (`wait` は見ない)。すでに Waiting にあるタスクは、
/// 待ち先を変えるときだけ書く。移した後は、移動元と移動先のセクションを並べ替える
/// (inbox.md からの移動では移動先だけ)。
///
/// 移動先の見出しが無ければ末尾に作る。Lua 版は inbox.md から移すとき、見出しの無いセクションへ
/// 加えたタスクを書き出さずに失う。
pub fn move_task(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    key: &str,
    to: SectionKey,
    wait: Wait,
) -> Result<Moved, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let at = locate(data_dir, names, today, &inbox.doc, &todo.doc, key)?;
    let id = unused_id(&[&inbox.doc, &todo.doc]);
    let target = names.name(to);
    let same = at.file == FileKind::Todo && at.section.map(|i| todo.doc.sections[i].name.as_str()) == Some(target);
    let file = match at.file {
        FileKind::Inbox => &mut inbox,
        _ => &mut todo,
    };

    let items = file.items_mut(at.section);
    let Item::Task(item) = &mut items[at.index] else {
        unreachable!("key::find only returns tasks");
    };
    // 完了したタスクは次の日次の整理で done.md へ移るため、セクションを変える意味が無い (Lua 版と同じ)。
    if item.task.status == Status::Done {
        return Err(OpError::MoveCompleted(key.to_string()));
    }
    let new_wait = match (to, wait) {
        (SectionKey::Waiting, Wait::Keep) => item.task.wait.clone(),
        (SectionKey::Waiting, Wait::Set(w)) => Some(w),
        _ => None,
    };
    if same {
        if to != SectionKey::Waiting || new_wait == item.task.wait {
            return Ok(Moved::AlreadyThere(item.task.clone()));
        }
        // Lua 版は待ち先だけを変えるときは並べ替えない。
        item.task.wait = new_wait;
        item.task.id.get_or_insert(id);
        let result = item.task.clone();
        file.write()?;
        return Ok(Moved::WaitChanged(result));
    }

    let Item::Task(item) = items.remove(at.index) else {
        unreachable!("key::find only returns tasks");
    };
    let lnum = item.lnum;
    let mut task = item.task;
    task.wait = new_wait;
    // 書き出すときにファイル内の重複として振り直されると、利用者に知らせた ID と食い違うため、先に決める。
    task.id.get_or_insert(id);
    let result = task.clone();
    let moved = Item::Task(Box::new(TaskItem {
        line: task.clone().serialize(),
        task,
        lnum: 0,
    }));

    match at.file {
        FileKind::Todo => {
            sort::sort_items(todo.items_mut(at.section));
            let items = section_items(&mut todo.doc, target);
            items.push(moved);
            sort::sort_items(items);
            todo.write()?;
        }
        _ => {
            // Lua 版は todo.md に見出しが 1 つも無ければ、すべてのセクションの見出しを書く。
            if todo.doc.sections.is_empty() {
                for k in SectionKey::ALL {
                    section_items(&mut todo.doc, names.name(k));
                }
            }
            let items = section_items(&mut todo.doc, target);
            items.push(moved);
            sort::sort_items(items);
            let added = todo.path.clone();
            // set_status で 1 行だけを書き換えるのと同じ理由。
            let remove = || {
                inbox.lines.remove(lnum - 1);
                inbox.write_lines()
            };
            append_then_remove(data_dir.root(), added, || todo.write(), remove)?;
        }
    }
    Ok(Moved::Moved(result))
}

/// [`edit_task`] で変える項目。`None` の項目は変えない。`Some(None)` はその項目を消す。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub content: Option<String>,
    pub priority: Option<Option<char>>,
    pub project: Option<Option<String>>,
    pub context: Option<Option<String>>,
    pub due: Option<Option<String>>,
    pub wait: Option<Option<String>>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }

    pub fn apply(&self, task: &mut Task) {
        fn set<T: Clone>(field: &mut T, change: &Option<T>) {
            if let Some(value) = change {
                *field = value.clone();
            }
        }
        set(&mut task.content, &self.content);
        set(&mut task.priority, &self.priority);
        set(&mut task.project, &self.project);
        set(&mut task.context, &self.context);
        set(&mut task.due, &self.due);
        set(&mut task.wait, &self.wait);
    }
}

/// [`edit_task`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub task: Task,
    /// タスクのあるファイル (inbox.md か todo.md)。
    pub file: FileKind,
}

/// キーで指定したタスクを書き換える (Lua 版 `init.lua` の `add_or_edit_task` の編集)。
/// 作成日の無いタスクには今日を付ける (Lua 版と同じ)。
///
/// 書き換えた後の期日のチェックと並べ替えは呼び出し元が行う。
pub fn edit_task(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    key: &str,
    changes: &Changes,
) -> Result<Edited, OpError> {
    let mut inbox = Loaded::read(data_dir, FileKind::Inbox, names, today)?;
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    let at = locate(data_dir, names, today, &inbox.doc, &todo.doc, key)?;
    let id = unused_id(&[&inbox.doc, &todo.doc]);
    let file = match at.file {
        FileKind::Inbox => &mut inbox,
        _ => &mut todo,
    };

    let Item::Task(item) = &file.items_mut(at.section)[at.index] else {
        unreachable!("key::find only returns tasks");
    };
    let lnum = item.lnum;
    let mut task = item.task.clone();
    changes.apply(&mut task);
    task.created.get_or_insert_with(|| today.format("%Y-%m-%d").to_string());
    // serialize は ID の無いタスクに発行するが、clone に付くため返すタスクに残らない。
    task.id.get_or_insert(id);
    let line = task.clone().serialize();
    if Task::parse_at(&line, today).as_ref() != Some(&task) {
        return Err(OpError::Unwritable(line));
    }
    // Lua 版は todo.md でもその 1 行だけを書き換える。並べ替えは後の期日のチェックと合わせて行う。
    file.lines[lnum - 1] = line;
    file.write_lines()?;
    Ok(Edited { task, file: at.file })
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
        let mut lines = done.lines.clone();
        for (month, tasks) in &mut completed.months {
            history::insert_tasks(&mut lines, "Done", month, tasks);
        }
        write_history(&done_path, FileKind::Done, &done, &lines, names)
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

/// done.md・cancelled.md へ追記した `lines` を書く。
fn write_history(
    path: &Path,
    kind: FileKind,
    read: &store::ReadFile,
    lines: &[String],
    names: &SectionNames,
) -> Result<(), OpError> {
    let before = Landmarks::of(kind, &read.lines, names);
    before.check(path, &Landmarks::of(kind, lines, names), names)?;
    atomic::write_lines(path, lines, read.crlf, read.stamp.as_ref())?;
    Ok(())
}

/// 読み込んだファイルと、書き戻すために必要な読んだ時点の状態。
struct Loaded {
    path: PathBuf,
    /// 読んだときの行。1 行だけを書き換えるときに使う。
    lines: Vec<String>,
    doc: Document,
    stamp: Option<Stamp>,
    crlf: bool,
    kind: FileKind,
    names: SectionNames,
    /// 読んだときにあった見出し。[`Loaded::lines`] は書き換えるため、読んだ時点で集めておく。
    landmarks: Landmarks,
}

impl Loaded {
    fn read(data_dir: &DataDir, kind: FileKind, names: &SectionNames, today: NaiveDate) -> Result<Loaded, ReadError> {
        let path = data_dir.path(kind);
        let file = store::read_file(&path)?;
        Ok(Loaded {
            doc: Document::parse(&file.lines, names, today),
            landmarks: Landmarks::of(kind, &file.lines, names),
            lines: file.lines,
            path,
            stamp: file.stamp,
            crlf: file.crlf,
            kind,
            names: names.clone(),
        })
    }

    /// 文書を整形し直して書く。
    fn write(&mut self) -> Result<(), WriteError> {
        let lines = self.doc.serialize();
        self.write_checked(&lines)
    }

    /// [`Loaded::lines`] をそのまま書く。
    fn write_lines(&mut self) -> Result<(), WriteError> {
        let lines = std::mem::take(&mut self.lines);
        let result = self.write_checked(&lines);
        self.lines = lines;
        result
    }

    fn write_checked(&mut self, lines: &[String]) -> Result<(), WriteError> {
        let after = Landmarks::of(self.kind, lines, &self.names);
        self.landmarks.check(&self.path, &after, &self.names)?;
        self.stamp = Some(atomic::write_lines(&self.path, lines, self.crlf, self.stamp.as_ref())?);
        Ok(())
    }

    fn items_mut(&mut self, section: Option<usize>) -> &mut Vec<Item> {
        match section {
            Some(i) => &mut self.doc.sections[i].items,
            None => &mut self.doc.preamble,
        }
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
    if let Ok(d) = std::fs::File::open(dir) {
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
    fn tasks_already_moved_to_done_or_cancelled_are_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("done.md"),
            "# Done\n\n## 2026-02\n\n- [x] 済み id:dddddd\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("cancelled.md"),
            "# Cancelled\n\n## 2026-02\n\n- [ ] 取りやめ id:cccccc\n- [ ] 手書き id:zz\n",
        )
        .unwrap();
        let data_dir = DataDir::new(dir.path());
        let complete = |key| complete(&data_dir, &SectionNames::default(), today(), key);
        for (key, file) in [
            ("dddddd", FileKind::Done),
            ("cccccc", FileKind::Cancelled),
            ("zz", FileKind::Cancelled),
        ] {
            let err = complete(key).unwrap_err();
            assert!(matches!(&err, OpError::Archived { file: f, .. } if *f == file), "{err}");
        }
        let err = complete("eeeeee").unwrap_err();
        assert!(matches!(err, OpError::Find(FindError::NotFound(_))), "{err}");
        let err = complete("ccc").unwrap_err();
        assert!(
            matches!(err, OpError::Find(FindError::TooShort(_))),
            "a short prefix of an archived ID is not taken as that task: {err}"
        );
    }

    /// Lua 版は見出しが別名 (現在の名前でない) のセクションのタスクを見つけられず、何も書かない。
    #[test]
    fn tasks_under_an_alias_heading_can_be_completed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("todo.md"),
            "# Todo\n\n## Today\n\n- [ ] 今日 id:aaaaaa\n",
        )
        .unwrap();
        let names = SectionNames::from_last(&[Some("今日".into()), None, None, None]).unwrap();
        complete(&DataDir::new(dir.path()), &names, today(), "aaaaaa").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            "# Todo\n\n## 今日\n\n- [x] 今日 completed_at:2026-02-03 id:aaaaaa\n"
        );
    }

    fn assert_crlf(path: &Path) {
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(text.matches('\n').count(), text.matches("\r\n").count(), "{text:?}");
    }

    #[test]
    fn crlf_is_kept_when_one_inbox_line_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inbox.md"), "# Inbox\r\n\r\n- [ ] 受信 id:aaaaaa\r\n").unwrap();
        complete(&DataDir::new(dir.path()), &SectionNames::default(), today(), "aaaaaa").unwrap();
        assert_crlf(&dir.path().join("inbox.md"));
    }

    #[test]
    fn crlf_is_kept_in_cancelled_md() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("todo.md"), TODO).unwrap();
        fs::write(dir.path().join("cancelled.md"), "# Cancelled\r\n").unwrap();
        super::cancel(&DataDir::new(dir.path()), &SectionNames::default(), today(), "bbbbbb").unwrap();
        assert_crlf(&dir.path().join("cancelled.md"));
    }

    fn cancel(d: &DataDir) -> Result<(), OpError> {
        super::cancel(d, &SectionNames::default(), today(), "bbbbbb").map(drop)
    }

    #[test]
    fn a_cancelled_task_stays_in_todo_when_cancelled_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("todo.md"), TODO).unwrap();
        make_unwritable(dir.path(), "cancelled.md", "# Cancelled\n");

        assert!(matches!(run(dir.path(), cancel), Err(OpError::Write(_))));
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), TODO);
    }

    #[test]
    fn a_cancelled_task_is_left_in_both_files_when_todo_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        make_unwritable(dir.path(), "todo.md", TODO);

        let err = run(dir.path(), cancel).unwrap_err();
        assert!(
            matches!(&err, OpError::PartiallyMoved { added, .. } if added.ends_with("cancelled.md")),
            "{err}"
        );
        assert!(
            fs::read_to_string(dir.path().join("cancelled.md"))
                .unwrap()
                .contains("id:bbbbbb")
        );
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), TODO);
    }

    const MOVE_INBOX: &str = "# Inbox\n- [ ] 受信 id:aaaaaa\n";

    fn move_to_next(d: &DataDir) -> Result<(), OpError> {
        move_task(
            d,
            &SectionNames::default(),
            today(),
            "aaaaaa",
            SectionKey::Next,
            Wait::Keep,
        )
        .map(drop)
    }

    #[test]
    fn a_moved_inbox_task_stays_in_the_inbox_when_todo_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inbox.md"), MOVE_INBOX).unwrap();
        make_unwritable(dir.path(), "todo.md", "# Todo\n\n## Next\n");

        assert!(matches!(run(dir.path(), move_to_next), Err(OpError::Write(_))));
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), MOVE_INBOX);
    }

    #[test]
    fn a_moved_inbox_task_is_left_in_both_files_when_the_inbox_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        make_unwritable(dir.path(), "inbox.md", MOVE_INBOX);
        fs::write(dir.path().join("todo.md"), "# Todo\n\n## Next\n").unwrap();

        let err = run(dir.path(), move_to_next).unwrap_err();
        assert!(
            matches!(&err, OpError::PartiallyMoved { added, .. } if added.ends_with("todo.md")),
            "{err}"
        );
        assert!(
            fs::read_to_string(dir.path().join("todo.md"))
                .unwrap()
                .contains("id:aaaaaa")
        );
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), MOVE_INBOX);
    }

    /// Lua 版は todo.md に書き出さないまま inbox.md から消すため、タスクが無くなる。
    #[test]
    fn a_task_moved_from_the_inbox_to_a_missing_section_is_kept_under_a_new_heading() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inbox.md"), MOVE_INBOX).unwrap();
        fs::write(dir.path().join("todo.md"), "# Todo\n\n## Today\n").unwrap();
        move_to_next(&DataDir::new(dir.path())).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            "# Todo\n\n## Today\n\n## Next\n\n- [ ] 受信 id:aaaaaa\n"
        );
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), "# Inbox\n");
    }

    /// Lua 版は見出しが別名 (現在の名前でない) のセクションのタスクを見つけられず、何も書かない。
    #[test]
    fn tasks_under_an_alias_heading_can_be_moved() {
        let dir = tempfile::tempdir().unwrap();
        let todo = "# Todo\n\n## Today\n\n- [ ] 今日 id:aaaaaa\n\n## Next\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        let names = SectionNames::from_last(&[Some("今日".into()), None, None, None]).unwrap();
        let data_dir = DataDir::new(dir.path());
        let move_to = |to| move_task(&data_dir, &names, today(), "aaaaaa", to, Wait::Keep).unwrap();

        assert!(matches!(move_to(SectionKey::Today), Moved::AlreadyThere(_)));
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), todo);
        assert!(matches!(move_to(SectionKey::Next), Moved::Moved(_)));
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            "# Todo\n\n## 今日\n\n## Next\n\n- [ ] 今日 id:aaaaaa\n"
        );
    }

    #[test]
    fn completed_tasks_are_not_moved() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("todo.md"), TODO).unwrap();
        let err = move_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            "bbbbbb",
            SectionKey::Next,
            Wait::Keep,
        )
        .unwrap_err();
        assert!(matches!(err, OpError::MoveCompleted(_)), "{err}");
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), TODO);
    }

    #[test]
    fn crlf_is_kept_when_a_task_is_moved_out_of_the_inbox() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("inbox.md"), MOVE_INBOX.replace('\n', "\r\n")).unwrap();
        fs::write(dir.path().join("todo.md"), "# Todo\r\n\r\n## Next\r\n").unwrap();
        move_to_next(&DataDir::new(dir.path())).unwrap();
        assert_crlf(&dir.path().join("inbox.md"));
        assert_crlf(&dir.path().join("todo.md"));
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
            Destination::Inbox,
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

    #[test]
    fn added_todo_tasks_are_sorted_into_their_section() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("todo.md"),
            "# Todo\n\n## Today\n\n## Next\n\n- [ ] 期日なし id:aaaaaa\n",
        )
        .unwrap();
        let task = Task {
            due: Some("2026-03-01".into()),
            ..new_task("期日あり")
        };
        let id = add_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            task,
            Destination::Section(SectionKey::Next),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            format!(
                "# Todo\n\n## Today\n\n## Next\n\n\
                 - [ ] 期日あり due:2026-03-01 created:2026-02-03 id:{id}\n- [ ] 期日なし id:aaaaaa\n"
            )
        );
    }

    /// Lua 版は見出しの無いセクションへ加えたタスクを書き出さずに失う。
    #[test]
    fn a_missing_section_is_created_at_the_end_instead_of_losing_the_task() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("todo.md"), "# Todo\n\n## Today\n").unwrap();
        let id = add_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            new_task("待ち"),
            Destination::Section(SectionKey::Waiting),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            format!("# Todo\n\n## Today\n\n## Waiting\n\n- [ ] 待ち created:2026-02-03 id:{id}\n")
        );
    }

    /// Lua 版はバッファのその 1 行だけを書き換えて保存するため、他の行は整形し直さない。
    #[test]
    fn editing_rewrites_only_the_line_of_the_task() {
        let dir = tempfile::tempdir().unwrap();
        let todo = "# Todo\r\n## Next\r\n- [ ] (B) 後 id:aaaaaa\r\n- [ ] 直す id:bbbbbb\r\n\r\n\r\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        let changes = Changes {
            content: Some("直した".into()),
            ..Changes::default()
        };
        let edited = edit_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            "bbbbbb",
            &changes,
        )
        .unwrap();
        assert_eq!(edited.file, FileKind::Todo);
        assert_eq!(
            fs::read_to_string(dir.path().join("todo.md")).unwrap(),
            "# Todo\r\n## Next\r\n- [ ] (B) 後 id:aaaaaa\r\n- [ ] 直した created:2026-02-03 id:bbbbbb\r\n\r\n\r\n"
        );
    }

    #[test]
    fn an_edit_that_would_be_read_back_differently_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = "# Inbox\n\n- [ ] (B) (C) 本文 created:2026-01-01 id:aaaaaa\n";
        fs::write(dir.path().join("inbox.md"), inbox).unwrap();
        let changes = Changes {
            priority: Some(None),
            ..Changes::default()
        };
        let err = edit_task(
            &DataDir::new(dir.path()),
            &SectionNames::default(),
            today(),
            "aaaaaa",
            &changes,
        )
        .unwrap_err();
        assert!(matches!(err, OpError::Unwritable(_)), "{err}");
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), inbox);
    }

    fn assert_would_remove<T: std::fmt::Debug>(result: Result<T, impl Into<OpError>>, missing: &str) {
        match result.map_err(Into::into) {
            Err(OpError::Write(WriteError::WouldRemove { missing: m, .. })) => assert_eq!(m, missing),
            other => panic!("{other:?}"),
        }
    }

    /// gtodo の不具合で見出しを落とした内容になっても、ファイルには書かない。
    #[test]
    fn writes_that_would_remove_a_heading_that_was_read_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = DataDir::new(dir.path());
        let names = SectionNames::default();
        let todo = "# Todo\n\n## Today\n\n## Next\n\n- [ ] 次 id:aaaaaa\n";
        let inbox = "# Inbox\n\n- [ ] 受信 id:bbbbbb\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        fs::write(dir.path().join("inbox.md"), inbox).unwrap();

        let mut file = Loaded::read(&data_dir, FileKind::Todo, &names, today()).unwrap();
        file.doc.sections.retain(|s| s.name != "Next");
        assert_would_remove(file.write(), "## Next");
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), todo);

        let mut file = Loaded::read(&data_dir, FileKind::Inbox, &names, today()).unwrap();
        file.lines.remove(0);
        assert_would_remove(file.write_lines(), "# Inbox");
        assert_eq!(fs::read_to_string(dir.path().join("inbox.md")).unwrap(), inbox);

        let done = "# Done\n\n## 2026-01\n\n- [x] 済み id:cccccc\n";
        fs::write(dir.path().join("done.md"), done).unwrap();
        let path = data_dir.path(FileKind::Done);
        let read = store::read_file(&path).unwrap();
        let lines = vec!["# Done".to_string()];
        assert_would_remove(
            write_history(&path, FileKind::Done, &read, &lines, &names),
            "## 2026-01",
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), done);
    }

    /// 見出しを落とす書き込みが追記の側で止まれば何も変わらず、削除の側で止まれば、追記は済んで
    /// 重複が残ったことを知らせる。
    #[test]
    fn a_refused_write_in_a_two_file_move_is_reported_like_other_write_errors() {
        let dir = tempfile::tempdir().unwrap();
        let refused = || WriteError::WouldRemove {
            path: dir.path().join("x.md"),
            missing: "## Next".into(),
        };
        let added = dir.path().join("todo.md");
        let remove = || -> Result<(), WriteError> { panic!("nothing is removed when nothing was added") };
        let err = append_then_remove(dir.path(), added.clone(), || Err(refused()), remove).unwrap_err();
        assert!(matches!(err, OpError::Write(WriteError::WouldRemove { .. })), "{err}");

        let err = append_then_remove(dir.path(), added, || Ok::<_, WriteError>(()), || Err(refused())).unwrap_err();
        assert!(
            matches!(
                err,
                OpError::PartiallyMoved {
                    source: WriteError::WouldRemove { .. },
                    ..
                }
            ),
            "{err}"
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
