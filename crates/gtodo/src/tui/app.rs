//! TUI の状態とキー操作。
//!
//! 描画 (`draw.rs`) と端末 (`mod.rs`) から切り離してあり、画面を通さずに確かめられる。
//!
//! メモリ上のタスクは表示のためだけに持ち、書き戻さない。操作はロックを取ってからファイルを
//! 読み直し、タスクのキーで探して書く (CLI と同じ `ops`)。表示が古くても他のツールの変更を潰さない。

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::input;
use gtodo_core::key::task_key;
use gtodo_core::lock::Lock;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Destination, Moved, Wait};
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::{DataDir, FileKind, Stamp};
use gtodo_core::task::{Status, Task};
use gtodo_core::view::entries;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::form::{Form, Outcome, Submit};
use super::input::{Edit, LineInput};
use super::keymap::{Action, Fed, Key, Keymap, TAB_COUNT};
use super::queue;
use crate::commands::{add, edit};
use crate::render::context_name;
use crate::workspace::Workspace;
use crate::writing;

/// 一覧のタブ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Inbox,
    Section(SectionKey),
    Done,
    Cancelled,
}

pub const TABS: [Tab; TAB_COUNT] = [
    Tab::Inbox,
    Tab::Section(SectionKey::Today),
    Tab::Section(SectionKey::Next),
    Tab::Section(SectionKey::Waiting),
    Tab::Section(SectionKey::Someday),
    Tab::Done,
    Tab::Cancelled,
];

/// 起動したときに開くタブ (Today)。
const START_TAB: usize = 1;

impl Tab {
    pub fn file(self) -> FileKind {
        match self {
            Tab::Inbox => FileKind::Inbox,
            Tab::Section(_) => FileKind::Todo,
            Tab::Done => FileKind::Done,
            Tab::Cancelled => FileKind::Cancelled,
        }
    }

    /// タスクを操作できるか。done.md・cancelled.md は見るだけ (CLI も操作しない)。
    pub fn writable(self) -> bool {
        matches!(self, Tab::Inbox | Tab::Section(_))
    }

    pub fn label(self, names: &SectionNames) -> &str {
        match self {
            Tab::Inbox => "Inbox",
            Tab::Section(key) => names.name(key),
            Tab::Done => "Done",
            Tab::Cancelled => "Cancelled",
        }
    }
}

/// 一覧の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// 操作のときにタスクを探すキー (ID、無ければ行の内容から作った一時キー)。
    pub key: String,
    pub task: Task,
}

#[derive(Debug, Clone, Default)]
pub struct TabView {
    pub rows: Vec<Row>,
    pub cursor: usize,
    /// 表示している最初の行。
    pub offset: usize,
}

impl TabView {
    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    fn select_key(&mut self, key: &str) -> bool {
        match self.rows.iter().position(|r| r.key == key) {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }
}

/// キー入力の受け先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    /// Waiting へ移す前に待ち先を尋ねている。
    WaitInput {
        row: Box<Row>,
        input: LineInput,
    },
    /// キャンセルしてよいか尋ねている。
    ConfirmCancel {
        key: String,
        content: String,
    },
    /// 追加・編集のフォーム。
    Form(Box<Form>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

/// 操作のときにロックが空くまで待つ時間。待つ間は画面が止まるため、CLI (2 秒) より短くする。
/// Neovim 版の自動処理がロックを持つのは数ミリ秒で、取れなければもう一度押せばよい。
const LOCK_WAIT: Duration = Duration::from_millis(300);

/// 今の画面。Queue は一覧から開く全画面で、`Esc` で一覧へ戻る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Queue,
}

/// Queue の状態。
#[derive(Debug, Clone, Default)]
pub struct QueueView {
    /// 待ち先別 (偽なら期日別)。
    pub wait: bool,
    pub lines: Vec<queue::Line>,
    /// 選んでいるタスク (タスクの行だけを数えた位置)。
    pub cursor: usize,
    /// 表示している最初の行 (見出しも数える)。
    pub offset: usize,
}

impl QueueView {
    /// タスクの行と、その行の位置。
    pub fn items(&self) -> impl Iterator<Item = (usize, &queue::Item)> {
        self.lines.iter().enumerate().filter_map(|(i, l)| match l {
            queue::Line::Item(item) => Some((i, item.as_ref())),
            _ => None,
        })
    }

    pub fn selected(&self) -> Option<&queue::Item> {
        self.items().nth(self.cursor).map(|(_, item)| item)
    }

    pub fn len(&self) -> usize {
        self.items().count()
    }

    fn select_key(&mut self, key: &str) -> bool {
        let found = self.items().position(|(_, item)| item.row.key == key);
        if let Some(i) = found {
            self.cursor = i;
        }
        found.is_some()
    }
}

/// 選んでいるタスクと、それがどこにあるか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub row: Row,
    pub file: FileKind,
    /// todo.md の管理対象のセクションにあれば、そのキー。
    pub section: Option<SectionKey>,
}

impl Selected {
    /// 操作できるか (done.md・cancelled.md は見るだけ)。
    pub fn writable(&self) -> bool {
        matches!(self.file, FileKind::Inbox | FileKind::Todo)
    }
}

/// 見張るファイル。順番は [`App::stamps`] の添字。
const WATCHED_DOCS: [FileKind; 4] = [FileKind::Inbox, FileKind::Todo, FileKind::Done, FileKind::Cancelled];
const SETTINGS: usize = 4;
const STATE: usize = 5;
const WATCHED: usize = 6;

pub struct App {
    pub ws: Workspace,
    pub keymap: Keymap,
    pub tabs: Vec<TabView>,
    pub current: usize,
    pub screen: Screen,
    pub queue: QueueView,
    pub mode: Mode,
    /// 押しかけのキー列 (`g` の後など)。
    pub pending: Vec<Key>,
    pub messages: Vec<Message>,
    /// 一覧に表示できる行数。描画のたびに更新する (半ページ・1 ページの移動に使う)。
    pub page_height: usize,
    /// ヘルプの表示位置。描画するときに入りきる範囲へ収める。
    pub help_scroll: usize,
    data_dir: PathBuf,
    /// 今日の日付を返す (テストでは日付を差し替える)。
    clock: Box<dyn Fn() -> NaiveDate>,
    today: NaiveDate,
    /// 最後に設定を開こうとしたときの日付。開けなかったときに毎回開き直さないため。
    opened_date: NaiveDate,
    quit: bool,
    /// 最後に読んだときの各ファイルの状態。`None` はまだ読んでいない。
    stamps: [Option<Option<Stamp>>; WATCHED],
    /// gtodo.json などが読めず、前の設定のまま表示している理由。直るまで書き込まない。
    config_error: Option<String>,
    rollover_pending: bool,
    /// 表示した設定の警告。読み直すたびに同じ警告を出さないため。
    seen_warnings: HashSet<String>,
    /// Queue を作るために持っておく、最後に読んだ inbox.md と todo.md。
    inbox_doc: Document,
    todo_doc: Document,
}

impl App {
    /// `data_dir` を開く。開けなければ (ディレクトリが無い・gtodo.json が壊れているなど) エラー。
    /// 起動時の日次ロールオーバーも試みる (ロックが取れなければ次の [`App::tick`] でやり直す)。
    pub fn new(
        data_dir: PathBuf,
        clock: Box<dyn Fn() -> NaiveDate>,
        keymap: Keymap,
        warnings: Vec<String>,
    ) -> Result<App> {
        let mut warnings = warnings;
        let today = clock();
        // 読む前に取る。読んでいる間に書き換えられても、次の確認で読み直せる。
        let dir = DataDir::new(data_dir.clone());
        let config_stamps = [stamp(&dir.settings_file()), stamp(&dir.state_file())];
        let ws = Workspace::open(data_dir.clone(), today, &mut |m| warnings.push(m))?;
        let mut stamps = [None; WATCHED];
        stamps[SETTINGS] = Some(config_stamps[0]);
        stamps[STATE] = Some(config_stamps[1]);
        let mut app = App {
            ws,
            keymap,
            tabs: vec![TabView::default(); TAB_COUNT],
            current: START_TAB,
            screen: Screen::List,
            queue: QueueView::default(),
            mode: Mode::Normal,
            pending: Vec::new(),
            messages: Vec::new(),
            page_height: 10,
            help_scroll: 0,
            data_dir,
            clock,
            today,
            opened_date: today,
            quit: false,
            stamps,
            config_error: None,
            rollover_pending: true,
            seen_warnings: HashSet::new(),
            inbox_doc: Document::default(),
            todo_doc: Document::default(),
        };
        for w in warnings {
            app.config_warning(w);
        }
        app.reload();
        app.try_rollover();
        app.notice_inbox_due();
        Ok(app)
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn tab(&self) -> Tab {
        TABS[self.current]
    }

    pub fn view(&self) -> &TabView {
        &self.tabs[self.current]
    }

    pub fn view_mut(&mut self) -> &mut TabView {
        &mut self.tabs[self.current]
    }

    pub fn today(&self) -> NaiveDate {
        self.today
    }

    /// 定期的な確認 (1 秒ごと)。日付が変わっていれば日次ロールオーバーを行い、
    /// ファイルが変わっていれば読み直す。
    pub fn tick(&mut self) {
        let new_day = self.check_date();
        if let Mode::Form(form) = &mut self.mode {
            form.set_today(self.today);
        }
        if self.rollover_pending {
            self.try_rollover();
        }
        self.reload();
        if new_day {
            self.notice_inbox_due();
        }
    }

    /// `auto_move_inbox_to_today` が偽のとき、期日が来ても inbox.md に残っているタスクを知らせる
    /// (起動したときと日付が変わったとき)。日次ロールオーバーが同じ知らせを出していれば重ねない。
    fn notice_inbox_due(&mut self) {
        if self.ws.auto_move_inbox_to_today || self.messages.iter().any(|m| m.text.starts_with(writing::INBOX_DUE)) {
            return;
        }
        let count = queue::due_in_inbox(&self.inbox_doc, self.today);
        if count > 0 {
            let queue = self.keymap.label(Action::OpenQueue).unwrap_or_default();
            self.message(
                Level::Info,
                format!(
                    "{count} task(s) in inbox.md are due; they stay there because auto_move_inbox_to_today is off ({queue} to see them)"
                ),
            );
        }
    }

    /// 日付が変わっていれば、新しい日付で設定を開き直す (日次ロールオーバーは次の確認か、
    /// 書き込む前の準備で行う)。前の日付のままだと、その日の整理は済んでいると判定されて
    /// 何もせず、完了日なども前の日付になる。
    fn check_date(&mut self) -> bool {
        let today = (self.clock)();
        if today == self.today {
            return false;
        }
        self.today = today;
        self.rollover_pending = true;
        self.reload();
        true
    }

    pub fn handle_key(&mut self, event: KeyEvent) {
        let key = Key::from_event(&event);
        // 端末の生の入力では SIGINT が来ないため、Ctrl-c はどこでも終了にする。
        if key.ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Normal => {}
            Mode::Help => return self.handle_help(key),
            Mode::WaitInput { .. } => return self.handle_wait_input(key),
            Mode::ConfirmCancel { .. } => return self.handle_confirm_cancel(key),
            Mode::Form(_) => return self.handle_form(key),
        }
        if key.code == KeyCode::Esc && !key.ctrl && !key.alt {
            // 押しかけのキーがあれば取り消し、無ければ Queue などから一覧へ戻る。知らせはどちらでも消す。
            // 終了はしない (終了は q)。
            if self.pending.is_empty() {
                self.screen = Screen::List;
            }
            self.pending.clear();
            self.messages.clear();
            return;
        }
        if let Fed::Action(action) = self.keymap.feed(&mut self.pending, key) {
            self.messages.clear();
            self.perform(action);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        match &mut self.mode {
            Mode::WaitInput { input, .. } => input.paste(text),
            Mode::Form(form) => form.paste(text),
            _ => {}
        }
    }

    fn perform(&mut self, action: Action) {
        let page = self.page_height.max(1) as isize;
        match action {
            Action::Down => self.move_cursor(1),
            Action::Up => self.move_cursor(-1),
            Action::Top => self.move_cursor(isize::MIN),
            Action::Bottom => self.move_cursor(isize::MAX),
            Action::HalfPageDown => self.move_cursor((page / 2).max(1)),
            Action::HalfPageUp => self.move_cursor(-(page / 2).max(1)),
            Action::PageDown => self.move_cursor(page),
            Action::PageUp => self.move_cursor(-page),
            // Queue から押したときは一覧へ戻ってタブを選ぶ。
            Action::NextTab => {
                self.current = (self.current + 1) % TAB_COUNT;
                self.screen = Screen::List;
            }
            Action::PrevTab => {
                self.current = (self.current + TAB_COUNT - 1) % TAB_COUNT;
                self.screen = Screen::List;
            }
            Action::GoTab(i) => {
                self.current = i.min(TAB_COUNT - 1);
                self.screen = Screen::List;
            }
            Action::OpenQueue => {
                // 開くたびに先頭 (期限切れ) から見せる。
                if self.screen != Screen::Queue {
                    self.queue.cursor = 0;
                    self.queue.offset = 0;
                }
                self.screen = Screen::Queue;
            }
            Action::QueueMode => {
                if self.screen == Screen::Queue {
                    self.queue.wait = !self.queue.wait;
                    self.queue.offset = 0;
                    // 選んでいたタスクが切り替えた先にもあれば選んだまま、無ければ先頭。
                    if !self.refresh_queue() {
                        self.queue.cursor = 0;
                    }
                }
            }
            Action::Open => self.open_in_list(),
            Action::Back => self.screen = Screen::List,
            Action::MoveToday => self.move_selected(SectionKey::Today, Wait::Keep),
            Action::MoveNext => self.move_selected(SectionKey::Next, Wait::Keep),
            Action::MoveSomeday => self.move_selected(SectionKey::Someday, Wait::Keep),
            Action::MoveWaiting => self.ask_wait(),
            Action::ToggleDone => self.toggle_done(),
            Action::Cancel => self.ask_cancel(),
            Action::Sort => self.sort(),
            Action::Add => self.open_add(),
            Action::Edit => self.open_edit(),
            Action::Help => self.mode = Mode::Help,
            Action::Quit => self.quit = true,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let (len, cursor) = match self.screen {
            Screen::List => (self.view().rows.len(), &mut self.tabs[self.current].cursor),
            Screen::Queue => (self.queue.len(), &mut self.queue.cursor),
        };
        if len == 0 {
            return;
        }
        *cursor = cursor.saturating_add_signed(delta).min(len - 1);
    }

    /// 今の画面で選んでいるタスク。
    pub fn selected(&self) -> Option<Selected> {
        match self.screen {
            Screen::List => {
                let tab = self.tab();
                self.view().selected().map(|row| Selected {
                    row: row.clone(),
                    file: tab.file(),
                    section: match tab {
                        Tab::Section(key) => Some(key),
                        _ => None,
                    },
                })
            }
            Screen::Queue => self.queue.selected().map(|item| Selected {
                row: item.row.clone(),
                file: item.file,
                section: item.section,
            }),
        }
    }

    /// 操作の対象にできる選択中のタスク。できなければ理由を知らせて `None`。
    fn target(&mut self) -> Option<Row> {
        let selected = self.selected()?;
        if !selected.writable() {
            self.message(
                Level::Warning,
                format!("tasks in {} cannot be changed", selected.file.file_name()),
            );
            return None;
        }
        Some(selected.row)
    }

    /// Queue で選んでいるタスクを、一覧のタブで選ぶ。
    fn open_in_list(&mut self) {
        if self.screen != Screen::Queue {
            return;
        }
        let Some(item) = self.queue.selected() else { return };
        let key = item.row.key.clone();
        for i in 0..TAB_COUNT {
            if self.tabs[i].select_key(&key) {
                self.current = i;
                self.screen = Screen::List;
                return;
            }
        }
        self.message(
            Level::Warning,
            "the task is under another heading in todo.md, which no tab shows".to_string(),
        );
    }

    /// inbox.md か todo.md を読み直したとき・表示を切り替えたときに、Queue を作り直す。
    /// 選んでいたタスクがまだあれば選んだままにして真を返す。無ければ同じ位置に留める。
    fn refresh_queue(&mut self) -> bool {
        let selected = self.queue.selected().map(|i| i.row.key.clone());
        self.queue.lines = queue::build(
            self.queue.wait,
            &self.inbox_doc,
            &self.todo_doc,
            &self.ws.names,
            self.today,
        );
        let kept = selected.is_some_and(|k| self.queue.select_key(&k));
        if !kept {
            self.queue.cursor = self.queue.cursor.min(self.queue.len().saturating_sub(1));
        }
        kept
    }

    fn move_selected(&mut self, to: SectionKey, wait: Wait) {
        let Some(row) = self.movable() else { return };
        self.move_row(&row, to, wait);
    }

    /// 選んでいるタスクのうち、セクションを移せるもの。
    fn movable(&mut self) -> Option<Row> {
        let row = self.target()?;
        if row.task.status == Status::Done {
            self.message(Level::Warning, "completed tasks cannot be moved".to_string());
            return None;
        }
        Some(row)
    }

    /// `row` のタスクを移す。選んでいる行ではなくキーで探すので、尋ねている間に
    /// 他のツールがタスクを別のセクションへ移していても操作できる。
    fn move_row(&mut self, row: &Row, to: SectionKey, wait: Wait) {
        let name = self.ws.names.name(to).to_string();
        let key = row.key.clone();
        let moved = self.write(false, |ws| {
            Ok(ops::move_task(&ws.data_dir, &ws.names, ws.today, &key, to, wait)?)
        });
        match moved {
            Some(Moved::Moved(task)) => self.message(Level::Info, format!("moved to {name}: {}", task.content)),
            Some(Moved::WaitChanged(task)) => {
                let text = match &task.wait {
                    Some(w) => format!("set wait:{w}: {}", task.content),
                    None => format!("removed wait: from {}", task.content),
                };
                self.message(Level::Info, text);
                self.select_task(&task);
            }
            Some(Moved::AlreadyThere(task)) => {
                self.message(Level::Info, format!("already in {name}: {}", task.content))
            }
            None => {}
        }
    }

    fn ask_wait(&mut self) {
        let Some(row) = self.movable() else { return };
        let input = LineInput::new(row.task.wait.as_deref().unwrap_or_default());
        self.mode = Mode::WaitInput {
            row: Box::new(row),
            input,
        };
    }

    fn handle_wait_input(&mut self, key: Key) {
        let Mode::WaitInput { input, .. } = &mut self.mode else {
            return;
        };
        match input.handle(key) {
            Edit::Continue => {}
            Edit::Abort => self.mode = Mode::Normal,
            Edit::Submit => match input::parse_wait(input.text(), self.today) {
                // 直せるように入力欄は閉じない。
                Err(e) => self.message(Level::Error, format!("{e:#}")),
                Ok(wait) => {
                    let Mode::WaitInput { row, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else {
                        unreachable!("checked above");
                    };
                    self.messages.clear();
                    self.move_row(&row, SectionKey::Waiting, wait);
                }
            },
        }
    }

    /// ヘルプは上下の移動でスクロールし、それ以外のキーで閉じる (端末が低いと入りきらないため)。
    fn handle_help(&mut self, key: Key) {
        let mut pending = Vec::new();
        let delta: isize = match self.keymap.feed(&mut pending, key) {
            Fed::Action(Action::Down) => 1,
            Fed::Action(Action::Up) => -1,
            Fed::Action(Action::HalfPageDown | Action::PageDown) => self.page_height.max(2) as isize / 2,
            Fed::Action(Action::HalfPageUp | Action::PageUp) => -(self.page_height.max(2) as isize / 2),
            _ => {
                self.mode = Mode::Normal;
                self.help_scroll = 0;
                return;
            }
        };
        self.help_scroll = self.help_scroll.saturating_add_signed(delta);
    }

    fn open_add(&mut self) {
        let (projects, contexts) = self.tag_candidates();
        // Queue から開いたときは inbox へ入れる (どのセクションとも決まっていないため)。
        let tab = match self.screen {
            Screen::List => self.tab(),
            Screen::Queue => Tab::Inbox,
        };
        self.mode = Mode::Form(Box::new(Form::add(tab, projects, contexts, self.today)));
    }

    fn open_edit(&mut self) {
        let Some(row) = self.target() else { return };
        let tab = match self.selected().and_then(|s| s.section) {
            Some(key) => Tab::Section(key),
            None => Tab::Inbox,
        };
        let (projects, contexts) = self.tag_candidates();
        let form = Form::edit(row, tab, projects, contexts, self.today);
        self.mode = Mode::Form(Box::new(form));
    }

    /// プロジェクトとコンテキストの候補。使われているタグと、projects/*.md の名前 (作りはしない)。
    fn tag_candidates(&self) -> (Vec<String>, Vec<String>) {
        let mut projects = BTreeSet::new();
        let mut contexts = BTreeSet::new();
        for row in self.tabs.iter().flat_map(|t| &t.rows) {
            if let Some(p) = row.task.project.as_deref().filter(|p| !p.is_empty()) {
                projects.insert(p.to_string());
            }
            if let Some(c) = context_name(&row.task) {
                contexts.insert(c.to_string());
            }
        }
        if let Ok(entries) = std::fs::read_dir(self.ws.data_dir.root().join("projects")) {
            for path in entries.flatten().map(|e| e.path()) {
                if path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    projects.insert(stem.to_string());
                }
            }
        }
        (projects.into_iter().collect(), contexts.into_iter().collect())
    }

    fn handle_form(&mut self, key: Key) {
        let Mode::Form(form) = &mut self.mode else { return };
        match form.handle(key) {
            Outcome::Continue => {}
            Outcome::Abort => self.mode = Mode::Normal,
            Outcome::Submit(submit) => {
                self.mode = Mode::Normal;
                self.messages.clear();
                self.submit_form(submit);
            }
        }
    }

    fn submit_form(&mut self, submit: Submit) {
        match submit {
            Submit::Unchanged => self.message(Level::Info, "nothing changed".to_string()),
            Submit::Add(task, destination) => {
                let content = task.content.clone();
                let to_inbox = destination == Destination::Inbox;
                let added = self.write(to_inbox, |ws| {
                    let id = ops::add_task(&ws.data_dir, &ws.names, ws.today, *task, destination)?;
                    // 期日がすでに来ていれば Today へ移す (CLI の add と同じ)。失敗しても追加は済んでいる。
                    let dues = to_inbox.then(|| add::check_dues_and_sort(ws));
                    Ok((id, dues))
                });
                let Some((id, dues)) = added else { return };
                let name = match destination {
                    Destination::Inbox => "inbox.md".to_string(),
                    Destination::Section(key) => self.ws.names.name(key).to_string(),
                };
                self.message(Level::Info, format!("added to {name}: {content}"));
                self.report_after_write(dues, "added");
                self.select_key(&id);
            }
            Submit::Edit { key, changes } => {
                let edited = self.write(true, |ws| {
                    let edited = ops::edit_task(&ws.data_dir, &ws.names, ws.today, &key, &changes)?;
                    let dues = edit::check_dues_and_sort(ws, edited.file == FileKind::Todo);
                    Ok((edited.task, dues))
                });
                let Some((task, dues)) = edited else { return };
                self.message(Level::Info, format!("edited: {}", task.content));
                self.report_after_write(Some(dues), "edited");
                self.select_task(&task);
            }
        }
    }

    /// 追加・編集の後の期日のチェックの結果を知らせる。
    fn report_after_write(&mut self, dues: Option<Result<DueOutcome>>, verb: &str) {
        match dues {
            Some(Ok(dues)) => {
                let (mut warnings, mut notes) = (Vec::new(), Vec::new());
                writing::report_dues(&self.ws, &dues, &mut |m| warnings.push(m), |m| notes.push(m));
                for n in notes {
                    self.message(Level::Info, n);
                }
                for w in warnings {
                    self.message(Level::Warning, w);
                }
            }
            // 書き込みは済んでいるため、失敗として扱うと利用者はもう一度書いてしまう。
            Some(Err(e)) => self.message(
                Level::Warning,
                format!("the task was {verb}, but checking due dates failed: {e:#}"),
            ),
            None => {}
        }
    }

    fn ask_cancel(&mut self) {
        let Some(row) = self.target() else { return };
        self.mode = Mode::ConfirmCancel {
            key: row.key,
            content: row.task.content,
        };
    }

    fn handle_confirm_cancel(&mut self, key: Key) {
        let Mode::ConfirmCancel { key: task_key, .. } = &self.mode else {
            return;
        };
        let task_key = task_key.clone();
        self.mode = Mode::Normal;
        if key.ctrl || key.alt || !matches!(key.code, KeyCode::Char('y' | 'Y')) {
            return;
        }
        self.messages.clear();
        let cancelled = self.write(false, |ws| {
            Ok(ops::cancel(&ws.data_dir, &ws.names, ws.today, &task_key)?)
        });
        if let Some(task) = cancelled {
            self.message(Level::Info, format!("cancelled: {}", task.content));
        }
    }

    fn toggle_done(&mut self) {
        let Some(row) = self.target() else { return };
        let key = row.key.clone();
        let result = match row.task.status {
            Status::Open => self
                .write(false, |ws| Ok(ops::complete(&ws.data_dir, &ws.names, ws.today, &key)?))
                .map(|t| ("completed", t)),
            Status::Done => self
                .write(false, |ws| Ok(ops::reopen(&ws.data_dir, &ws.names, ws.today, &key)?))
                .map(|t| ("reopened", t)),
        };
        if let Some((verb, task)) = result {
            self.message(Level::Info, format!("{verb}: {}", task.content));
            self.select_task(&task);
        }
    }

    fn sort(&mut self) {
        if self.screen == Screen::List && !self.tab().writable() {
            return;
        }
        let dues = self.write(true, |ws| {
            let dues = ops::check_dues(&ws.data_dir, &ws.names, ws.today, ws.auto_move_inbox_to_today)?;
            ops::sort_todo(&ws.data_dir, &ws.names, ws.today)?;
            Ok(dues)
        });
        if let Some(dues) = dues {
            let (mut warnings, mut notes) = (Vec::new(), Vec::new());
            writing::report_dues(&self.ws, &dues, &mut |m| warnings.push(m), |m| notes.push(m));
            notes.push("sorted todo.md".to_string());
            for n in notes {
                self.message(Level::Info, n);
            }
            for w in warnings {
                self.message(Level::Warning, w);
            }
        }
    }

    /// 操作したタスクを選ぶ (完了にすると ID が付いてキーが変わるため、返ってきたタスクで探す)。
    fn select_task(&mut self, task: &Task) {
        if let Some(id) = task.id.as_deref() {
            self.select_key(id);
        }
    }

    /// 今の画面でキーのタスクを選ぶ。
    fn select_key(&mut self, key: &str) {
        match self.screen {
            Screen::List => self.view_mut().select_key(key),
            Screen::Queue => self.queue.select_key(key),
        };
    }

    /// ロックを取り、CLI の書き込むコマンドと同じ準備をしてから `op` を実行し、読み直す。
    /// 失敗したら知らせて `None`。
    fn write<T>(&mut self, checks_dues: bool, op: impl FnOnce(&Workspace) -> Result<T>) -> Option<T> {
        self.check_date();
        if let Some(e) = &self.config_error {
            let text = format!("{e}; fix it before changing tasks");
            self.message(Level::Error, text);
            return None;
        }
        let (mut warnings, mut notes) = (Vec::new(), Vec::new());
        let result = writing::ensure_writable(&self.ws)
            .and_then(|()| Ok(Lock::acquire(self.ws.data_dir.root(), LOCK_WAIT)?))
            .and_then(|lock| {
                writing::run_locked(
                    &self.ws,
                    lock,
                    checks_dues,
                    &mut |m| warnings.push(m),
                    &mut |m| notes.push(m),
                    || op(&self.ws),
                )
            });
        for n in notes {
            self.message(Level::Info, n);
        }
        for w in warnings {
            self.message(Level::Warning, w);
        }
        // 失敗しても読み直す (他で書き換えられていて見つからなかった場合に、今の内容を出すため)。
        self.reload();
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.message(Level::Error, format!("{e:#}"));
                None
            }
        }
    }

    fn try_rollover(&mut self) {
        if self.config_error.is_some() || writing::ensure_writable(&self.ws).is_err() {
            // 書き込めない理由は、読み込んだときに警告してある。
            self.rollover_pending = false;
            return;
        }
        let lock = match Lock::try_acquire(self.ws.data_dir.root()) {
            Ok(Some(lock)) => lock,
            // Neovim 版などが持っている。次の確認でやり直す。
            Ok(None) => return,
            Err(e) => {
                self.rollover_pending = false;
                self.message(Level::Error, e.to_string());
                return;
            }
        };
        self.rollover_pending = false;
        let (mut warnings, mut notes) = (Vec::new(), Vec::new());
        let result = writing::run_locked(
            &self.ws,
            lock,
            false,
            &mut |m| warnings.push(m),
            &mut |m| notes.push(m),
            || Ok(()),
        );
        for n in notes {
            self.message(Level::Info, n);
        }
        for w in warnings {
            self.message(Level::Warning, w);
        }
        if let Err(e) = result {
            self.message(Level::Error, format!("{e:#}"));
        }
        self.reload();
    }

    /// 変わったファイルだけを読み直す。gtodo.json か .state.json が変わったとき (と日付が
    /// 変わったとき) は、セクション名などの設定から読み直す。
    fn reload(&mut self) {
        let mut now = [None; WATCHED];
        for (i, kind) in WATCHED_DOCS.iter().enumerate() {
            now[i] = Some(stamp(&self.ws.data_dir.path(*kind)));
        }
        now[SETTINGS] = Some(stamp(&self.ws.data_dir.settings_file()));
        now[STATE] = Some(stamp(&self.ws.data_dir.state_file()));

        let reopen = self.today != self.opened_date
            || now[SETTINGS] != self.stamps[SETTINGS]
            || now[STATE] != self.stamps[STATE];
        // 文書の解析は、セクション名と (相対的な期日を解決する) 日付で決まる。.state.json は
        // Neovim 版が通知のたびなどに書き換えるので、それだけで全部を読み直さない。
        let mut reparse = false;
        if reopen {
            self.opened_date = self.today;
            let mut warnings = Vec::new();
            match Workspace::open(self.data_dir.clone(), self.today, &mut |m| warnings.push(m)) {
                Ok(ws) => {
                    reparse = ws.names != self.ws.names || ws.today != self.ws.today;
                    self.ws = ws;
                    self.config_error = None;
                }
                Err(e) => {
                    let text = format!("{e:#}");
                    if self.config_error.as_ref() != Some(&text) {
                        self.message(Level::Error, format!("{text}; showing the tasks as read before"));
                    }
                    self.config_error = Some(text);
                }
            }
            for w in warnings {
                self.config_warning(w);
            }
        }
        let mut queue_changed = false;
        for (i, kind) in WATCHED_DOCS.iter().enumerate() {
            if reparse || now[i] != self.stamps[i] {
                self.load(*kind);
                queue_changed |= matches!(kind, FileKind::Inbox | FileKind::Todo);
            }
        }
        // inbox.md と todo.md を読み終えてから作り直す。片方ずつ作り直すと、inbox.md から
        // todo.md へ移したタスクが途中でどちらにも無くなり、選んでいたタスクを見失う。
        if queue_changed || reparse {
            self.refresh_queue();
        }
        self.stamps = now;
    }

    fn load(&mut self, kind: FileKind) {
        let doc = match self.ws.load(kind) {
            Ok(doc) => doc,
            Err(e) => {
                self.message(Level::Error, format!("{e:#}"));
                Document::default()
            }
        };
        for (i, tab) in TABS.iter().enumerate() {
            if tab.file() != kind {
                continue;
            }
            let view = &mut self.tabs[i];
            let selected = view.selected().map(|r| r.key.clone());
            view.rows = rows(*tab, &doc, &self.ws.names);
            if !selected.is_some_and(|k| view.select_key(&k)) {
                view.clamp();
            }
        }
        match kind {
            FileKind::Inbox => self.inbox_doc = doc,
            FileKind::Todo => self.todo_doc = doc,
            _ => {}
        }
    }

    fn config_warning(&mut self, text: String) {
        if self.seen_warnings.insert(text.clone()) {
            self.message(Level::Warning, text);
        }
    }

    fn message(&mut self, level: Level, text: String) {
        let message = Message { level, text };
        if self.messages.last() != Some(&message) {
            self.messages.push(message);
        }
    }
}

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    // 状態を取れないファイルは無いものとして扱う。取れるようになれば変わったとみなして読み直す。
    Stamp::of(path).ok().flatten()
}

/// タブに出すタスク。Done・Cancelled は新しいものから (ファイルの末尾が新しい)。
fn rows(tab: Tab, doc: &Document, names: &SectionNames) -> Vec<Row> {
    let row = |e: gtodo_core::view::Entry| Row {
        key: task_key(e.task(), &e.item.line),
        task: e.task().clone(),
    };
    let file = tab.file();
    match tab {
        Tab::Inbox => entries(file, doc).map(row).collect(),
        Tab::Section(key) => {
            let name = names.name(key);
            entries(file, doc)
                .filter(|e| e.section == Some(name))
                .map(row)
                .collect()
        }
        Tab::Done | Tab::Cancelled => {
            let mut rows: Vec<Row> = entries(file, doc).map(row).collect();
            rows.reverse();
            rows
        }
    }
}
