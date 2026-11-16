//! タスクの追加・編集フォーム。
//!
//! 本文と項目は `gtodo add`・`gtodo edit` と同じ規則 (core の `input`) で読む。入力のたびに
//! 書き込まれる行を作り直して見せ、読めない値があるうちは確定させない。

use chrono::NaiveDate;
use gtodo_core::input::{self, Fields, InputError};
use gtodo_core::ops::{Changes, Destination};
use gtodo_core::sections::SectionKey;
use gtodo_core::task::Task;
use ratatui::crossterm::event::KeyCode;

use super::app::{Row, Tab};
use super::input::{Edit, LineInput};
use super::keymap::Key;
use crate::render::context_name;

/// 追加先の候補 (表示の順)。
pub const DESTINATIONS: [Destination; 5] = [
    Destination::Inbox,
    Destination::Section(SectionKey::Today),
    Destination::Section(SectionKey::Next),
    Destination::Section(SectionKey::Waiting),
    Destination::Section(SectionKey::Someday),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Text,
    Due,
    Project,
    Context,
    Priority,
    Destination,
    Wait,
}

/// 文字を入れる項目 (追加先以外)。添字は [`Form::inputs`] の位置。
const TEXT_FIELDS: [Field; 6] = [
    Field::Text,
    Field::Due,
    Field::Project,
    Field::Context,
    Field::Priority,
    Field::Wait,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Add,
    /// 編集する行と、それがあったタブ。
    Edit {
        row: Box<Row>,
        tab: Tab,
    },
}

/// 確定したときの書き込み。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submit {
    Add(Box<Task>, Destination),
    Edit {
        key: String,
        changes: Changes,
    },
    /// 編集で何も変えていない。
    Unchanged,
}

/// [`Form::handle`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Abort,
    Submit(Submit),
}

/// `Ctrl-n`/`Ctrl-p` で候補を順に入れている途中。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cycle {
    /// 候補を探し始めたときに入っていた文字 (これで候補を絞る)。
    typed: String,
    index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub kind: Kind,
    pub focus: Field,
    inputs: [LineInput; 6],
    /// 開いたときの値 (編集で、変えた項目だけを変更にするため)。
    initial: [String; 6],
    destination: usize,
    projects: Vec<String>,
    contexts: Vec<String>,
    cycle: Option<Cycle>,
    today: NaiveDate,
}

impl Form {
    /// 追加のフォーム。`tab` は開いていたタブで、追加先の初期値にする。
    pub fn add(tab: Tab, projects: Vec<String>, contexts: Vec<String>, today: NaiveDate) -> Form {
        let destination = match tab {
            Tab::Section(key) => DESTINATIONS
                .iter()
                .position(|d| *d == Destination::Section(key))
                .unwrap_or(0),
            Tab::Inbox | Tab::Done | Tab::Cancelled => 0,
        };
        Form::new(Kind::Add, Default::default(), destination, projects, contexts, today)
    }

    /// 編集のフォーム。今の値を入れて開く。
    pub fn edit(row: Row, tab: Tab, projects: Vec<String>, contexts: Vec<String>, today: NaiveDate) -> Form {
        let t = &row.task;
        let value = |v: &Option<String>| v.clone().unwrap_or_default();
        let initial = [
            t.content.clone(),
            value(&t.due),
            value(&t.project),
            context_name(t).unwrap_or_default().to_string(),
            t.priority.map(String::from).unwrap_or_default(),
            value(&t.wait),
        ];
        let kind = Kind::Edit {
            row: Box::new(row),
            tab,
        };
        Form::new(kind, initial, 0, projects, contexts, today)
    }

    fn new(
        kind: Kind,
        initial: [String; 6],
        destination: usize,
        projects: Vec<String>,
        contexts: Vec<String>,
        today: NaiveDate,
    ) -> Form {
        Form {
            kind,
            focus: Field::Text,
            inputs: initial.clone().map(|v| LineInput::new(&v)),
            initial,
            destination,
            projects,
            contexts,
            cycle: None,
            today,
        }
    }

    pub fn is_add(&self) -> bool {
        self.kind == Kind::Add
    }

    pub fn destination(&self) -> Destination {
        DESTINATIONS[self.destination]
    }

    /// 表示する項目 (上から順)。
    pub fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Text, Field::Due, Field::Project, Field::Context, Field::Priority];
        match &self.kind {
            Kind::Add => {
                fields.push(Field::Destination);
                // 待ち先は Waiting へ入れるときだけ (他のセクションでは Neovim 版も待ち先を消す)。
                if self.destination() == Destination::Section(SectionKey::Waiting) {
                    fields.push(Field::Wait);
                }
            }
            Kind::Edit { row, tab } => {
                if *tab == Tab::Section(SectionKey::Waiting) || row.task.wait.is_some() {
                    fields.push(Field::Wait);
                }
            }
        }
        fields
    }

    pub fn input(&self, field: Field) -> Option<&LineInput> {
        TEXT_FIELDS.iter().position(|f| *f == field).map(|i| &self.inputs[i])
    }

    fn input_mut(&mut self, field: Field) -> Option<&mut LineInput> {
        TEXT_FIELDS
            .iter()
            .position(|f| *f == field)
            .map(|i| &mut self.inputs[i])
    }

    fn value(&self, field: Field) -> &str {
        self.input(field).map_or("", LineInput::text)
    }

    /// 編集で、開いたときから変えたか。
    fn changed(&self, field: Field) -> bool {
        TEXT_FIELDS
            .iter()
            .position(|f| *f == field)
            .is_some_and(|i| self.inputs[i].text() != self.initial[i])
    }

    /// 入力から作った書き込みの内容。読めない値があればそのエラー。
    pub fn evaluate(&self) -> Result<Submit, InputError> {
        let visible = self.fields();
        match &self.kind {
            Kind::Add => {
                // 追加では、空の項目は指定しなかったものとして本文のタグに任せる。
                let given = |field: Field| {
                    let v = self.value(field).trim();
                    (visible.contains(&field) && !v.is_empty()).then(|| v.to_string())
                };
                let fields = Fields {
                    due: given(Field::Due),
                    project: given(Field::Project),
                    context: given(Field::Context),
                    priority: given(Field::Priority),
                    wait: given(Field::Wait),
                };
                let task = input::new_task(self.value(Field::Text), &fields, self.today)?;
                Ok(Submit::Add(Box::new(task), self.destination()))
            }
            Kind::Edit { row, .. } => {
                if self.value(Field::Text).trim().is_empty() {
                    return Err(InputError::Empty);
                }
                // タグだけの本文は、CLI の edit では本文を変えない指定だが、フォームでは本文の欄が
                // タグだけに見えたまま元の本文が残り、紛らわしい。本文の欄には本文を入れてもらう。
                if self.changed(Field::Text)
                    && input::parse_text(self.value(Field::Text), self.today, "editing")?
                        .content
                        .is_empty()
                {
                    return Err(InputError::Empty);
                }
                // 変えた項目だけを渡す。変えていない本文を読み直すと、本文の先頭の `(A)` が
                // 優先度として読まれるなど、意図しない変更になりうる。
                let given = |field: Field| {
                    (visible.contains(&field) && self.changed(field)).then(|| self.value(field).trim().to_string())
                };
                let fields = Fields {
                    due: given(Field::Due),
                    project: given(Field::Project),
                    context: given(Field::Context),
                    priority: given(Field::Priority),
                    wait: given(Field::Wait),
                };
                let text = if self.changed(Field::Text) {
                    self.value(Field::Text)
                } else {
                    ""
                };
                let changes = input::changes(text, &fields, self.today)?;
                if changes.is_empty() {
                    return Ok(Submit::Unchanged);
                }
                Ok(Submit::Edit {
                    key: row.key.clone(),
                    changes,
                })
            }
        }
    }

    /// 書き込まれるタスク (確定前のプレビュー)。
    pub fn preview(&self) -> Result<Task, InputError> {
        match (self.evaluate()?, &self.kind) {
            (Submit::Add(task, _), _) => Ok(*task),
            (Submit::Edit { changes, .. }, Kind::Edit { row, .. }) => {
                let mut task = row.task.clone();
                changes.apply(&mut task);
                Ok(task)
            }
            (Submit::Unchanged, Kind::Edit { row, .. }) => Ok(row.task.clone()),
            (Submit::Edit { .. } | Submit::Unchanged, Kind::Add) => unreachable!("adding always gives Submit::Add"),
        }
    }

    /// 入っている期日の解釈 (期日の欄の横に出す)。空なら `None`。
    pub fn due(&self) -> Option<Result<String, InputError>> {
        let value = self.value(Field::Due).trim();
        (!value.is_empty()).then(|| input::parse_due(value, self.today))
    }

    /// 今の項目に出す候補と、`Ctrl-n`/`Ctrl-p` で選んでいるものの位置。
    pub fn candidates(&self) -> (Vec<&str>, Option<usize>) {
        let (list, prefix) = match self.focus {
            Field::Project => (&self.projects, '+'),
            Field::Context => (&self.contexts, '@'),
            _ => return (Vec::new(), None),
        };
        let typed = match &self.cycle {
            Some(c) => c.typed.clone(),
            None => self.value(self.focus).to_string(),
        };
        let typed = typed.trim().trim_start_matches(prefix).to_lowercase();
        let matches: Vec<&str> = list
            .iter()
            .map(String::as_str)
            .filter(|c| c.to_lowercase().starts_with(&typed))
            .collect();
        (matches, self.cycle.as_ref().map(|c| c.index))
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        match (key.code, key.ctrl) {
            (KeyCode::Esc, _) => return Outcome::Abort,
            (KeyCode::Enter, _) => {
                return match self.evaluate() {
                    Ok(submit) => Outcome::Submit(submit),
                    // 理由はフォームに出ている。
                    Err(_) => Outcome::Continue,
                };
            }
            (KeyCode::Tab | KeyCode::Down, false) => self.move_focus(1),
            (KeyCode::BackTab | KeyCode::Up, false) => self.move_focus(-1),
            (KeyCode::Char('n'), true) => self.cycle_candidate(1),
            (KeyCode::Char('p'), true) => self.cycle_candidate(-1),
            _ if self.focus == Field::Destination => match key.code {
                KeyCode::Left => self.destination = (self.destination + DESTINATIONS.len() - 1) % DESTINATIONS.len(),
                KeyCode::Right | KeyCode::Char(' ') => self.destination = (self.destination + 1) % DESTINATIONS.len(),
                _ => {}
            },
            _ => {
                self.cycle = None;
                let key = match (self.focus, key.code) {
                    // 優先度は大文字で書く (小文字は優先度として読まれない)。
                    (Field::Priority, KeyCode::Char(c)) if !key.ctrl => Key {
                        code: KeyCode::Char(c.to_ascii_uppercase()),
                        ..key
                    },
                    _ => key,
                };
                if let Some(input) = self.input_mut(self.focus) {
                    // Enter と Esc は上で受けている。
                    let _: Edit = input.handle(key);
                }
            }
        }
        Outcome::Continue
    }

    pub fn paste(&mut self, text: &str) {
        self.cycle = None;
        let focus = self.focus;
        // 打ったときと同じく、優先度は大文字にする。
        let text = match focus {
            Field::Priority => text.to_ascii_uppercase(),
            _ => text.to_string(),
        };
        if let Some(input) = self.input_mut(focus) {
            input.paste(&text);
        }
    }

    /// 日付が変わったら呼ぶ。相対的な期日 (`tomorrow` など) を新しい日付から読み直す。
    pub fn set_today(&mut self, today: NaiveDate) {
        self.today = today;
    }

    fn move_focus(&mut self, delta: isize) {
        self.cycle = None;
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = (at as isize + delta).rem_euclid(fields.len() as isize) as usize;
        self.focus = fields[next];
    }

    fn cycle_candidate(&mut self, delta: isize) {
        if !matches!(self.focus, Field::Project | Field::Context) {
            return;
        }
        let (matches, index) = self.candidates();
        if matches.is_empty() {
            return;
        }
        let len = matches.len() as isize;
        let next = match index {
            Some(i) => (i as isize + delta).rem_euclid(len) as usize,
            None if delta > 0 => 0,
            None => (len - 1) as usize,
        };
        let chosen = matches[next].to_string();
        let typed = match &self.cycle {
            Some(c) => c.typed.clone(),
            None => self.value(self.focus).to_string(),
        };
        let focus = self.focus;
        if let Some(input) = self.input_mut(focus) {
            *input = LineInput::new(&chosen);
        }
        self.cycle = Some(Cycle { typed, index: next });
    }
}

/// 書き込まれる行 (ID は付ける前なので出さない)。
pub fn preview_line(task: &Task) -> String {
    task.to_line_without_id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()
    }

    fn typing(form: &mut Form, text: &str) {
        for c in text.chars() {
            form.handle(Key::char(c));
        }
    }

    fn row(line: &str) -> Row {
        let task = Task::parse_at(line, today()).unwrap();
        Row {
            key: task.id.clone().unwrap_or_default(),
            task,
        }
    }

    fn add_form(tab: Tab) -> Form {
        Form::add(
            tab,
            vec!["home".into(), "work".into(), "writing".into()],
            vec!["office".into()],
            today(),
        )
    }

    #[test]
    fn the_destination_starts_at_the_open_tab() {
        assert_eq!(add_form(Tab::Inbox).destination(), Destination::Inbox);
        assert_eq!(
            add_form(Tab::Section(SectionKey::Next)).destination(),
            Destination::Section(SectionKey::Next)
        );
        assert_eq!(add_form(Tab::Done).destination(), Destination::Inbox);
    }

    #[test]
    fn tags_in_the_text_and_fields_make_the_task() {
        let mut form = add_form(Tab::Inbox);
        typing(&mut form, "買い物 +home due:tomorrow");
        assert_eq!(
            preview_line(&form.preview().unwrap()),
            "- [ ] 買い物 +home due:2026-11-17 created:2026-11-16"
        );
        // 項目は本文のタグより優先する。
        form.handle(Key::plain(KeyCode::Tab));
        typing(&mut form, "fri");
        assert_eq!(form.due(), Some(Ok("2026-11-20".to_string())));
        form.handle(Key::plain(KeyCode::Tab));
        typing(&mut form, "+work");
        form.handle(Key::plain(KeyCode::Tab));
        form.handle(Key::plain(KeyCode::Tab));
        typing(&mut form, "a");
        assert_eq!(
            preview_line(&form.preview().unwrap()),
            "- [ ] (A) 買い物 +work due:2026-11-20 created:2026-11-16"
        );
        assert!(matches!(
            form.handle(Key::plain(KeyCode::Enter)),
            Outcome::Submit(Submit::Add(..))
        ));
    }

    #[test]
    fn unreadable_values_block_submitting() {
        let mut form = add_form(Tab::Inbox);
        assert_eq!(form.evaluate(), Err(InputError::Empty));
        assert_eq!(form.handle(Key::plain(KeyCode::Enter)), Outcome::Continue);
        typing(&mut form, "x");
        form.handle(Key::plain(KeyCode::Tab));
        typing(&mut form, "2/30");
        assert!(matches!(form.due(), Some(Err(InputError::InvalidDue(_)))));
        assert_eq!(form.handle(Key::plain(KeyCode::Enter)), Outcome::Continue);
    }

    #[test]
    fn the_wait_field_appears_only_for_waiting() {
        let mut form = add_form(Tab::Inbox);
        assert!(!form.fields().contains(&Field::Wait));
        for _ in 0..5 {
            form.handle(Key::plain(KeyCode::Tab));
        }
        assert_eq!(form.focus, Field::Destination);
        for _ in 0..3 {
            form.handle(Key::plain(KeyCode::Right));
        }
        assert_eq!(form.destination(), Destination::Section(SectionKey::Waiting));
        assert!(form.fields().contains(&Field::Wait));
        form.handle(Key::plain(KeyCode::Down));
        typing(&mut form, "bob");
        form.handle(Key::plain(KeyCode::BackTab));
        form.handle(Key::plain(KeyCode::Left));
        // 他の追加先では、入れた待ち先は使わない。
        form.focus = Field::Text;
        typing(&mut form, "x");
        assert_eq!(form.preview().unwrap().wait, None);
    }

    #[test]
    fn ctrl_n_cycles_through_matching_candidates() {
        let mut form = add_form(Tab::Inbox);
        form.focus = Field::Project;
        typing(&mut form, "w");
        assert_eq!(form.candidates().0, ["work", "writing"]);
        form.handle(Key::ctrl('n'));
        assert_eq!(form.value(Field::Project), "work");
        form.handle(Key::ctrl('n'));
        assert_eq!(form.value(Field::Project), "writing");
        form.handle(Key::ctrl('n'));
        assert_eq!(form.value(Field::Project), "work");
        form.handle(Key::ctrl('p'));
        assert_eq!(form.value(Field::Project), "writing");
        // 文字を打つと、そこから絞り直す。
        form.handle(Key::plain(KeyCode::Backspace));
        assert_eq!(form.candidates().0, ["writing"]);
        // 先頭の + は無視して絞る。
        form.focus = Field::Context;
        typing(&mut form, "@O");
        assert_eq!(form.candidates().0, ["office"]);
    }

    #[test]
    fn editing_changes_only_what_was_changed() {
        let r = row("- [ ] (B) (A) 本文の先頭が括弧 +home due:2026-11-20 wait:bob id:aaaaaa");
        assert_eq!(r.task.content, "(A) 本文の先頭が括弧");
        let mut form = Form::edit(r.clone(), Tab::Section(SectionKey::Waiting), vec![], vec![], today());
        assert_eq!(form.value(Field::Project), "home");
        assert_eq!(form.value(Field::Wait), "bob");
        // 何も変えなければ書かない (本文を読み直して優先度が変わったりしない)。
        assert_eq!(form.evaluate(), Ok(Submit::Unchanged));

        form.focus = Field::Due;
        form.handle(Key::ctrl('u'));
        typing(&mut form, "+1d");
        form.focus = Field::Project;
        form.handle(Key::ctrl('u'));
        assert_eq!(
            form.evaluate(),
            Ok(Submit::Edit {
                key: "aaaaaa".into(),
                changes: Changes {
                    due: Some(Some("2026-11-17".into())),
                    project: Some(None),
                    ..Changes::default()
                },
            })
        );
        assert_eq!(
            preview_line(&form.preview().unwrap()),
            "- [ ] (B) (A) 本文の先頭が括弧 due:2026-11-17 wait:bob"
        );

        // 本文を空にはできない。
        form.focus = Field::Text;
        form.handle(Key::ctrl('u'));
        assert_eq!(form.evaluate(), Err(InputError::Empty));
    }

    #[test]
    fn priority_is_written_in_upper_case() {
        let mut form = add_form(Tab::Inbox);
        typing(&mut form, "x");
        form.focus = Field::Priority;
        typing(&mut form, "b");
        assert_eq!(form.value(Field::Priority), "B");
        assert_eq!(form.preview().unwrap().priority, Some('B'));
        form.handle(Key::plain(KeyCode::Backspace));
        form.paste("c");
        assert_eq!(form.value(Field::Priority), "C");
    }

    #[test]
    fn a_body_of_only_tags_is_refused_when_editing() {
        let r = row("- [ ] 本文 +work id:aaaaaa");
        let mut form = Form::edit(r, Tab::Inbox, vec![], vec![], today());
        form.handle(Key::ctrl('u'));
        typing(&mut form, "+home");
        assert_eq!(form.evaluate(), Err(InputError::Empty));
        typing(&mut form, " 新しい本文");
        assert!(matches!(form.evaluate(), Ok(Submit::Edit { .. })));
    }

    #[test]
    fn relative_dues_follow_a_date_change() {
        let mut form = add_form(Tab::Inbox);
        typing(&mut form, "x due:tomorrow");
        assert_eq!(form.preview().unwrap().due.as_deref(), Some("2026-11-17"));
        form.set_today(NaiveDate::from_ymd_opt(2026, 11, 17).unwrap());
        assert_eq!(form.preview().unwrap().due.as_deref(), Some("2026-11-18"));
    }
}
