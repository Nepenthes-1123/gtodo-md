//! タスクの表示形式。

use gtodo_core::key::task_key;
use gtodo_core::task::{Status, Task};
use gtodo_core::view::Entry;
use serde::Serialize;

/// キーの表示幅 (ID は6桁、一時キーは `~` + 6桁)。
pub const KEY_WIDTH: usize = 7;

pub fn key_of(entry: &Entry) -> String {
    task_key(entry.task(), &entry.item.line)
}

/// 一覧用の1行表示。内部用のタグ (`id:`・`created:`・`done:`・`from:`) は出さない。
pub fn summary(task: &Task) -> String {
    let mut s = String::from(match task.status {
        Status::Open => "[ ]",
        Status::Done => "[x]",
    });
    if let Some(p) = task.priority {
        s.push_str(&format!(" ({p})"));
    }
    if !task.content.is_empty() {
        s.push(' ');
        s.push_str(&task.content);
    }
    s.push_str(&tags(task));
    for (name, value) in [
        ("due", &task.due),
        ("wait", &task.wait),
        ("completed_at", &task.completed_at),
        ("cancelled", &task.cancelled),
    ] {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            s.push_str(&format!(" {name}:{v}"));
        }
    }
    s
}

/// ` +project @context` (無いものは省く)。
pub fn tags(task: &Task) -> String {
    let mut s = String::new();
    for tag in [project_tag(task), context_tag(task)].into_iter().flatten() {
        s.push(' ');
        s.push_str(&tag);
    }
    s
}

/// `+project`。無ければ `None`。
pub fn project_tag(task: &Task) -> Option<String> {
    task.project
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| format!("+{p}"))
}

/// `@context`。無ければ `None`。
pub fn context_tag(task: &Task) -> Option<String> {
    task.context
        .as_deref()
        .filter(|c| !c.is_empty())
        .map(|c| format!("@{}", c.trim_start_matches('@')))
}

#[derive(Serialize)]
pub struct TaskJson<'a> {
    key: String,
    file: &'static str,
    lnum: usize,
    section: Option<&'a str>,
    status: &'static str,
    priority: Option<String>,
    content: &'a str,
    project: Option<&'a str>,
    context: Option<&'a str>,
    due: Option<&'a str>,
    created: Option<&'a str>,
    wait: Option<&'a str>,
    completed_at: Option<&'a str>,
    done: Option<&'a str>,
    cancelled: Option<&'a str>,
    from: Option<&'a str>,
    id: Option<&'a str>,
}

impl<'a> TaskJson<'a> {
    pub fn new(entry: &Entry<'a>) -> Self {
        let t = entry.task();
        let opt = |v: &'a Option<String>| v.as_deref().filter(|s| !s.is_empty());
        TaskJson {
            key: key_of(entry),
            file: entry.file.file_name(),
            lnum: entry.item.lnum,
            section: entry.section,
            status: match t.status {
                Status::Open => "open",
                Status::Done => "done",
            },
            priority: t.priority.map(String::from),
            content: &t.content,
            project: opt(&t.project),
            context: opt(&t.context),
            due: opt(&t.due),
            created: opt(&t.created),
            wait: opt(&t.wait),
            completed_at: opt(&t.completed_at),
            done: opt(&t.done),
            cancelled: opt(&t.cancelled),
            from: opt(&t.from),
            id: opt(&t.id),
        }
    }
}
