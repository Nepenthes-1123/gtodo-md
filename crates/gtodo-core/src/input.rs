//! 利用者が入力したタスクの本文と項目を、タスク行の文法で読んで確かめる。
//!
//! CLI の `add`・`edit`・`move --wait` と TUI のフォームで共有する (同じ入力が、使うツールに
//! よって違うタスクにならないように)。文法そのものは `task.rs` だけが知っており、ここでは
//! 書いた行を読み直して確かめる。

use chrono::NaiveDate;

use crate::date;
use crate::ops::{Changes, Wait};
use crate::task::Task;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("the task text must be a single line")]
    MultiLine,
    #[error("the task text must not be empty")]
    Empty,
    /// ID や完了・キャンセルの記録は、書き込むときに付けるもの。
    #[error("{tag}: cannot be set when {action} a task")]
    Reserved { tag: &'static str, action: &'static str },
    #[error("created: cannot be changed")]
    CreatedChanged,
    #[error("invalid created date {0:?} (use YYYY-MM-DD)")]
    InvalidCreated(String),
    #[error("invalid due date {0:?} (e.g. today, tomorrow, +3d, fri, 2026-07-02)")]
    InvalidDue(String),
    #[error("the priority must be one letter (A-Z)")]
    InvalidPriority,
    #[error("{0:?} cannot be written as wait: (spaces and some punctuation such as . , ( ) are not allowed)")]
    InvalidWait(String),
    #[error("the project, context, priority or wait: value cannot be written in a task line")]
    Unwritable,
}

/// 本文以外で指定する項目。`None` は指定なし。
///
/// 指定したものは本文中のタグより優先する。編集 ([`changes`]) では空にするとその項目を消す。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub due: Option<String>,
    pub project: Option<String>,
    pub context: Option<String>,
    pub priority: Option<String>,
    pub wait: Option<String>,
}

/// 新しく加えるタスクを作る。本文をタスク行と同じ文法で読み、`fields` で上書きし、
/// 作成日が無ければ今日にする。
pub fn new_task(text: &str, fields: &Fields, today: NaiveDate) -> Result<Task, InputError> {
    let mut task = parse_text(text, today, "adding")?;
    if let Some(due) = &fields.due {
        task.due = Some(parse_due(due, today)?);
    }
    if let Some(due) = &task.due {
        check_due(due, today)?;
    }
    if let Some(project) = &fields.project {
        task.project = Some(project.strip_prefix('+').unwrap_or(project).to_string());
    }
    if let Some(context) = &fields.context {
        task.context = Some(context.strip_prefix('@').unwrap_or(context).to_string());
    }
    if let Some(priority) = &fields.priority {
        task.priority = parse_priority(priority)?.or(task.priority);
    }
    if let Some(wait) = &fields.wait {
        task.wait = match parse_wait(wait, today)? {
            Wait::Set(w) => Some(w),
            Wait::Remove | Wait::Keep => None,
        };
    }
    if task.content.is_empty() {
        return Err(InputError::Empty);
    }
    task.created.get_or_insert_with(|| today.format("%Y-%m-%d").to_string());
    check_round_trip(&task, today)?;
    Ok(task)
}

/// 既存のタスクへの変更を作る。本文に書いたタグと指定した項目だけを変え、他は残す。
/// 本文が空白だけなら本文は変えない。何も変えない指定でもエラーにはしない (呼び出し元が決める)。
pub fn changes(text: &str, fields: &Fields, today: NaiveDate) -> Result<Changes, InputError> {
    let mut changes = Changes::default();
    if !text.trim().is_empty() {
        let task = parse_text(text, today, "editing")?;
        // Neovim 版の編集でも作成日は変えられない。
        if task.created.is_some() {
            return Err(InputError::CreatedChanged);
        }
        if !task.content.is_empty() {
            changes.content = Some(task.content);
        }
        changes.priority = task.priority.map(Some);
        changes.project = task.project.map(Some);
        changes.context = task.context.map(Some);
        changes.due = task.due.map(Some);
        changes.wait = task.wait.map(Some);
    }

    // `+` だけの値は消す指定ではなく空の名前として扱い、追加と同じく書けない値としてはじく。
    let unless_empty = |value: &str, prefix: char| {
        (!value.is_empty()).then(|| value.strip_prefix(prefix).unwrap_or(value).to_string())
    };
    if let Some(due) = &fields.due {
        changes.due = Some(match due.as_str() {
            "" => None,
            due => Some(parse_due(due, today)?),
        });
    }
    if let Some(project) = &fields.project {
        changes.project = Some(unless_empty(project, '+'));
    }
    if let Some(context) = &fields.context {
        changes.context = Some(unless_empty(context, '@'));
    }
    if let Some(priority) = &fields.priority {
        changes.priority = Some(parse_priority(priority)?);
    }
    if let Some(wait) = &fields.wait {
        changes.wait = Some(match parse_wait(wait, today)? {
            Wait::Set(w) => Some(w),
            Wait::Remove | Wait::Keep => None,
        });
    }
    if let Some(Some(due)) = &changes.due {
        check_due(due, today)?;
    }

    let mut probe = Task {
        content: "x".into(),
        ..Task::default()
    };
    changes.apply(&mut probe);
    check_round_trip(&probe, today)?;
    Ok(changes)
}

/// 本文をタスク行と同じ文法で読む。本文が空でもタグだけのタスクとして返す。
/// 期日は項目で置き換えられうるため、ここでは確かめない ([`check_due`])。
/// `action` はエラーメッセージに使う (`adding` など)。
pub fn parse_text(text: &str, today: NaiveDate, action: &'static str) -> Result<Task, InputError> {
    if text.contains(['\n', '\r']) {
        return Err(InputError::MultiLine);
    }
    let Some(task) = Task::parse_at(&format!("- [ ] {text}"), today) else {
        return Err(InputError::Empty);
    };
    // ID や完了・キャンセルの記録は書き込むコマンドが付けるもので、利用者が指定すると
    // 他のタスクと ID が重なったり、完了していないのに完了の記録を持ったりする。
    for (tag, value) in [
        ("id", &task.id),
        ("completed_at", &task.completed_at),
        ("done", &task.done),
        ("cancelled", &task.cancelled),
        ("from", &task.from),
    ] {
        if value.is_some() {
            return Err(InputError::Reserved { tag, action });
        }
    }
    if let Some(created) = &task.created
        && !is_date(created)
    {
        return Err(InputError::InvalidCreated(created.clone()));
    }
    Ok(task)
}

/// 項目で渡された期日を、タスク行に書く形にする。
pub fn parse_due(input: &str, today: NaiveDate) -> Result<String, InputError> {
    let due = date::parse_due_date(input, today).ok_or_else(|| InputError::InvalidDue(input.to_string()))?;
    check_due(&due, today)?;
    Ok(due)
}

/// タスク行に書く期日が、実在する日付として読めるかを確かめる。
pub fn check_due(due: &str, today: NaiveDate) -> Result<(), InputError> {
    // 読めなかった期日は書かれたまま残り、日付の形の表記は範囲を確かめずに桁だけ揃えられる
    // (`2026-02-30` も通る) ため、正規化した形が実在する日付かで見分ける。
    if date::parse_due_date(due, today).as_deref() != Some(due) || !is_date(due) {
        return Err(InputError::InvalidDue(due.to_string()));
    }
    Ok(())
}

/// 優先度の 1 文字。空なら `None` (消す)。
pub fn parse_priority(value: &str) -> Result<Option<char>, InputError> {
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (None, _) => Ok(None),
        (Some(p), None) => Ok(Some(p)),
        _ => Err(InputError::InvalidPriority),
    }
}

/// 待ち先。前後の空白は除き、空なら消す (Neovim 版の入力欄と同じ)。
pub fn parse_wait(value: &str, today: NaiveDate) -> Result<Wait, InputError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Wait::Remove);
    }
    // 書いた行を読み直して同じ待ち先にならない値 (空白や `.` を含むものなど) は、次に読んだときに
    // 本文へ混ざって壊れるため受け付けない。
    let mut task = Task {
        content: "x".into(),
        wait: Some(value.to_string()),
        ..Task::default()
    };
    if Task::parse_at(&task.serialize(), today).and_then(|t| t.wait).as_deref() != Some(value) {
        return Err(InputError::InvalidWait(value.to_string()));
    }
    Ok(Wait::Set(value.to_string()))
}

/// 書いた行を読み直して同じタスクにならない値 (空白を含むプロジェクト名や小文字の優先度など) は、
/// 次に読んだときに本文へ混ざって壊れるため受け付けない。
pub fn check_round_trip(task: &Task, today: NaiveDate) -> Result<(), InputError> {
    let mut written = Task {
        id: Some("000000".into()),
        ..task.clone()
    };
    let reread = Task::parse_at(&written.serialize(), today);
    if reread.as_ref().map(|t| Task { id: None, ..t.clone() })
        != Some(Task {
            id: None,
            ..task.clone()
        })
    {
        return Err(InputError::Unwritable);
    }
    Ok(())
}

fn is_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    fn new(text: &str) -> Result<Task, InputError> {
        new_task(text, &Fields::default(), today())
    }

    fn edit(text: &str) -> Result<Changes, InputError> {
        changes(text, &Fields::default(), today())
    }

    fn fields(f: impl FnOnce(&mut Fields)) -> Fields {
        let mut fields = Fields::default();
        f(&mut fields);
        fields
    }

    #[test]
    fn tags_in_the_text_are_read_like_a_task_line() {
        let task = new("(A) 買い物 +home @15 due:tomorrow wait:bob").unwrap();
        assert_eq!(task.content, "買い物");
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.project.as_deref(), Some("home"));
        assert_eq!(task.context.as_deref(), Some("15"));
        assert_eq!(task.due.as_deref(), Some("2026-02-04"));
        assert_eq!(task.wait.as_deref(), Some("bob"));
        assert_eq!(task.created.as_deref(), Some("2026-02-03"));
        assert_eq!(task.id, None);

        let task = new("x created:2026-01-01").unwrap();
        assert_eq!(
            task.created.as_deref(),
            Some("2026-01-01"),
            "a given creation date is kept"
        );
    }

    #[test]
    fn fields_override_tags_in_the_text() {
        let f = fields(|f| {
            f.due = Some("+1w".into());
            f.project = Some("+work".into());
            f.context = Some("@office".into());
            f.priority = Some("B".into());
            f.wait = Some(" carol ".into());
        });
        let task = new_task("(A) 資料 +home @15 due:today wait:bob", &f, today()).unwrap();
        assert_eq!(task.content, "資料");
        assert_eq!(task.priority, Some('B'));
        assert_eq!(task.project.as_deref(), Some("work"));
        assert_eq!(task.context.as_deref(), Some("office"));
        assert_eq!(task.due.as_deref(), Some("2026-02-10"));
        assert_eq!(task.wait.as_deref(), Some("carol"));

        let f = fields(|f| f.due = Some("today".into()));
        let task = new_task("x due:someday", &f, today()).unwrap();
        assert_eq!(
            task.due.as_deref(),
            Some("2026-02-03"),
            "an unreadable due in the text is replaced"
        );

        // 空の優先度・待ち先は、本文の値を残す (追加では消すものが無い)。
        let f = fields(|f| {
            f.priority = Some(String::new());
            f.wait = Some(String::new());
        });
        let task = new_task("(A) x wait:bob", &f, today()).unwrap();
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.wait, None);
    }

    #[test]
    fn records_written_by_commands_cannot_be_given() {
        for text in [
            "x id:abcdef",
            "x completed_at:2026-01-01",
            "x done:2026-01-01",
            "x cancelled:2026-01-01",
            "x from:today",
        ] {
            assert!(matches!(new(text), Err(InputError::Reserved { .. })), "{text}");
            assert!(matches!(edit(text), Err(InputError::Reserved { .. })), "{text}");
        }
    }

    #[test]
    fn new_tasks_with_values_that_do_not_survive_being_read_back_are_rejected() {
        let with = |f: Fields| new_task("x", &f, today());
        assert_eq!(
            with(fields(|f| f.project = Some("two words".into()))),
            Err(InputError::Unwritable)
        );
        assert_eq!(
            with(fields(|f| f.priority = Some("a".into()))),
            Err(InputError::Unwritable)
        );
        assert_eq!(
            with(fields(|f| f.priority = Some("AB".into()))),
            Err(InputError::InvalidPriority)
        );
        assert!(matches!(
            with(fields(|f| f.due = Some("someday".into()))),
            Err(InputError::InvalidDue(_))
        ));
        assert!(matches!(
            with(fields(|f| f.due = Some("2026-02-30".into()))),
            Err(InputError::InvalidDue(_))
        ));
        assert!(matches!(
            with(fields(|f| f.wait = Some("bob smith".into()))),
            Err(InputError::InvalidWait(_))
        ));
        assert!(matches!(new("x due:someday"), Err(InputError::InvalidDue(_))));
        assert!(matches!(new("x due:2026-99-99"), Err(InputError::InvalidDue(_))));
        assert!(matches!(
            new("x created:2026-13-45"),
            Err(InputError::InvalidCreated(_))
        ));
        assert_eq!(new("x +a\u{b}"), Err(InputError::Unwritable));
        assert_eq!(new("+only-a-tag"), Err(InputError::Empty));
        assert_eq!(new(""), Err(InputError::Empty));
        assert_eq!(new("two\nlines"), Err(InputError::MultiLine));
    }

    #[test]
    fn only_what_the_text_mentions_is_changed() {
        assert_eq!(
            edit("(A) 新しい本文 +work due:tomorrow").unwrap(),
            Changes {
                content: Some("新しい本文".into()),
                priority: Some(Some('A')),
                project: Some(Some("work".into())),
                due: Some(Some("2026-02-04".into())),
                ..Changes::default()
            }
        );
        assert_eq!(
            edit("@15 wait:bob").unwrap(),
            Changes {
                context: Some(Some("15".into())),
                wait: Some(Some("bob".into())),
                ..Changes::default()
            },
            "text with tags only keeps the description"
        );
        assert!(edit("").unwrap().is_empty());
        assert!(edit("  ").unwrap().is_empty());
    }

    #[test]
    fn fields_override_the_text_and_empty_fields_remove_values() {
        let f = fields(|f| {
            f.due = Some(String::new());
            f.project = Some(String::new());
            f.context = Some("@office".into());
            f.priority = Some(String::new());
            f.wait = Some(String::new());
        });
        assert_eq!(
            changes("(A) 本文 +home due:today wait:bob", &f, today()).unwrap(),
            Changes {
                content: Some("本文".into()),
                priority: Some(None),
                project: Some(None),
                context: Some(Some("office".into())),
                due: Some(None),
                wait: Some(None),
            }
        );

        let f = fields(|f| f.due = Some("today".into()));
        assert_eq!(
            changes("x due:someday", &f, today()).unwrap().due,
            Some(Some("2026-02-03".into())),
            "a due field replaces an unreadable due in the text"
        );
    }

    #[test]
    fn changes_that_cannot_be_written_are_rejected() {
        assert_eq!(edit("x created:2026-01-01"), Err(InputError::CreatedChanged));
        assert!(matches!(edit("x due:someday"), Err(InputError::InvalidDue(_))));
        assert_eq!(edit("two\nlines"), Err(InputError::MultiLine));
        let with = |f: Fields| changes("", &f, today());
        assert_eq!(
            with(fields(|f| f.project = Some("two words".into()))),
            Err(InputError::Unwritable)
        );
        assert_eq!(
            with(fields(|f| f.project = Some("+".into()))),
            Err(InputError::Unwritable)
        );
        assert_eq!(
            with(fields(|f| f.priority = Some("a".into()))),
            Err(InputError::Unwritable)
        );
        assert_eq!(
            with(fields(|f| f.priority = Some("AB".into()))),
            Err(InputError::InvalidPriority)
        );
        assert!(matches!(
            with(fields(|f| f.due = Some("2026-02-30".into()))),
            Err(InputError::InvalidDue(_))
        ));
    }

    #[test]
    fn an_empty_wait_removes_it() {
        assert_eq!(parse_wait("", today()), Ok(Wait::Remove));
        assert_eq!(parse_wait("  ", today()), Ok(Wait::Remove));
    }

    #[test]
    fn a_wait_is_trimmed() {
        assert_eq!(parse_wait(" 田中さん ", today()), Ok(Wait::Set("田中さん".into())));
    }

    #[test]
    fn waits_that_cannot_be_read_back_are_rejected() {
        for value in ["bob smith", "bob.", "a(b)", "x\ny"] {
            assert!(parse_wait(value, today()).is_err(), "{value:?}");
        }
    }
}
