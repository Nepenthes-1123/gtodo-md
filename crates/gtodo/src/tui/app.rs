//! TUI の状態とキー操作。
//!
//! 描画 (`draw.rs`) と端末 (`mod.rs`) から切り離してあり、画面を通さずに確かめられる。
//!
//! メモリ上のタスクは表示のためだけに持ち、書き戻さない。操作はロックを取ってからファイルを
//! 読み直し、タスクのキーで探して書く (CLI と同じ `ops`)。表示が古くても他のツールの変更を潰さない。

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use chrono::NaiveDate;
use gtodo_core::atomic::WriteError;
use gtodo_core::config::{Platform, ProcessEnv};
use gtodo_core::document::Document;
use gtodo_core::input;
use gtodo_core::key::task_key;
use gtodo_core::lock::Lock;
use gtodo_core::logic::due::DueOutcome;
use gtodo_core::ops::{self, Destination, EditTarget, EditedError, Moved, ProjectError, Wait};
use gtodo_core::project;
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::{self, DataDir, FileKind, RawFile, Stamp};
use gtodo_core::task::{Status, Task};
use gtodo_core::view::{self, SEARCH_FILES, entries};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::editor::Editor;
use super::form::{Form, Outcome, Submit};
use super::input::{Edit, LineInput};
use super::kanban::{self, KanbanView};
use super::keymap::{Action, Fed, Key, Keymap, TAB_COUNT};
use super::markdown;
use super::projects::{self, ProjectsView};
use super::queue;
use super::search::{self, SearchView};
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

/// Done のタブの位置 (done.md のタスクを、プロジェクトの進捗を数えるのに使う)。
const DONE_TAB: usize = 5;

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
    /// 一覧のタブと検索で同じタスクを共有する (数万件を二重に持たないため)。
    pub task: Rc<Task>,
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
    /// 新しいプロジェクトの名前を尋ねている。
    NewProject {
        input: LineInput,
    },
    /// 開くリンクを選んでいる (リンクが複数あるとき)。
    ChooseLink {
        links: Vec<String>,
        cursor: usize,
    },
    /// `$EDITOR` で編集した内容が保存時の検証に通らず、もう一度編集するか尋ねている。
    FixEdit {
        /// データディレクトリからの相対パス (`todo.md`・`projects/site.md`)。
        name: String,
        problems: Vec<String>,
    },
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

/// Waiting のタスクの期日が何日後までなら知らせるか (Neovim 版の `waiting_warning_days` の既定)。
const WAITING_WARNING_DAYS: u64 = 2;

/// 操作のときにロックが空くまで待つ時間。待つ間は画面が止まるため、CLI (2 秒) より短くする。
/// Neovim 版の自動処理がロックを持つのは数ミリ秒で、取れなければもう一度押せばよい。
const LOCK_WAIT: Duration = Duration::from_millis(300);

/// リンクを開く関数。
pub type Opener = Box<dyn Fn(&str) -> std::io::Result<()>>;

/// ファイル (と 1 始まりの行) をエディタで開き、閉じるまで待つ関数。エディタが正常に終わったかを返す。
pub type EditorRunner = Box<dyn Fn(&Path, Option<usize>) -> std::io::Result<bool>>;

/// `$EDITOR` で編集しているファイルのコピー。
///
/// 本物のファイルではなく一時ディレクトリのコピーを開く。そのまま開くと、エディタが保存した時点で
/// 書かれてしまい、Neovim 版の保存時の検証で止めることも、編集している間に Neovim 版などが書いた
/// 変更を潰さないよう確かめることもできないため。閉じた後に検証し、読んだときの状態と照らして書き戻す。
struct EditSession {
    target: EditTarget,
    /// 編集を始めたときに読んだ内容と状態。
    original: RawFile,
    /// コピーを置いた一時ディレクトリ。セッションを捨てると一緒に消える。
    dir: tempfile::TempDir,
    copy: PathBuf,
    line: Option<usize>,
}

/// 今の画面。Queue は一覧から開く全画面で、`Esc` で一覧へ戻る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Queue,
    Search,
    Kanban,
    Projects,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected<'a> {
    pub row: &'a Row,
    pub file: FileKind,
    /// todo.md の管理対象のセクションにあれば、そのキー。
    pub section: Option<SectionKey>,
}

impl Selected<'_> {
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
    pub search: SearchView,
    pub kanban: KanbanView,
    pub projects: ProjectsView,
    pub mode: Mode,
    /// 押しかけのキー列 (`g` の後など)。
    pub pending: Vec<Key>,
    pub messages: Vec<Message>,
    /// 一覧に表示できる行数。描画のたびに更新する (半ページ・1 ページの移動に使う)。
    pub page_height: usize,
    /// ヘルプの表示位置。描画するときに入りきる範囲へ収める。
    pub help_scroll: usize,
    data_dir: PathBuf,
    /// リンクを開く (テストでは差し替えて、開いたものを記録する)。
    opener: Opener,
    /// ファイルをエディタで開く (テストでは差し替えて、編集したことにする)。
    editor: EditorRunner,
    /// 知らせに出すエディタの名前。
    editor_name: String,
    /// `$EDITOR` で編集しているファイル (もう一度編集するかを尋ねている間も持つ)。
    edit: Option<EditSession>,
    /// [`App::run_editor`] でエディタを開く。端末を明け渡す必要があるので、呼ぶのは `mod.rs`。
    editor_requested: bool,
    /// 書き戻せずに残した編集のコピー。終了した後にも標準エラーへ知らせる (画面の知らせは消えるため)。
    kept_copies: Vec<PathBuf>,
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
    /// 期日が来ても inbox.md に残っているタスクを知らせた (日次の整理が警告した場合を含む) 日。
    inbox_noticed: Option<NaiveDate>,
    /// 期日の近い Waiting のタスクとして知らせたタスク (本文と期日) と、その日。同じタスクを何度も知らせないため。
    waiting_noticed: (Option<NaiveDate>, HashSet<String>),
    /// Queue を開いていない間に inbox.md・todo.md が変わり、作り直しが要る。
    queue_stale: bool,
    /// 検索を開いていない間に対象のファイルが変わり、絞り込み直しが要る。
    search_stale: bool,
    /// カンバンを開いていない間に todo.md・done.md が変わり、作り直しが要る。
    kanban_stale: bool,
    /// inbox.md・todo.md・done.md が変わり、プロジェクトの進捗を数え直す必要がある。
    projects_stale: bool,
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
        let editor = Editor::from_env(&ProcessEnv, Platform::current());
        let editor_name = editor.name().to_string();
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
            search: SearchView::default(),
            kanban: KanbanView::default(),
            projects: ProjectsView::default(),
            mode: Mode::Normal,
            pending: Vec::new(),
            messages: Vec::new(),
            page_height: 10,
            help_scroll: 0,
            data_dir,
            opener: Box::new(|url| open::that_detached(url)),
            editor: Box::new(move |file, line| editor.command(file, line).status().map(|s| s.success())),
            editor_name,
            edit: None,
            editor_requested: false,
            kept_copies: Vec::new(),
            clock,
            today,
            opened_date: today,
            quit: false,
            stamps,
            config_error: None,
            rollover_pending: true,
            seen_warnings: HashSet::new(),
            inbox_noticed: None,
            waiting_noticed: (None, HashSet::new()),
            queue_stale: true,
            search_stale: true,
            kanban_stale: true,
            projects_stale: true,
            inbox_doc: Document::default(),
            todo_doc: Document::default(),
        };
        for w in warnings {
            app.config_warning(w);
        }
        app.reload();
        app.try_rollover();
        app.notice_inbox_due();
        app.notice_waiting_due();
        Ok(app)
    }

    /// リンクを開く方法を差し替える (テスト用)。
    #[cfg(test)]
    pub fn set_opener(&mut self, opener: Opener) {
        self.opener = opener;
    }

    /// エディタを差し替える (テスト用)。
    #[cfg(test)]
    pub fn set_editor(&mut self, editor: EditorRunner) {
        self.editor = editor;
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// 書き戻せずに残した編集のコピー。終わるときに呼ぶ。編集の途中 (検証に通らず尋ねている間に
    /// `Ctrl-c` で終えたときなど) のものもコピーを残して返す (編集した内容を黙って消さないため)。
    pub fn finish(&mut self) -> Vec<PathBuf> {
        if let Some(session) = self.edit.take() {
            let name = session.target.file_name();
            self.kept_copies.push(session.dir.keep().join(name));
        }
        std::mem::take(&mut self.kept_copies)
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
        self.check_date();
        if let Mode::Form(form) = &mut self.mode {
            form.set_today(self.today);
        }
        if self.rollover_pending {
            self.try_rollover();
        }
        self.reload();
        self.notice_inbox_due();
        self.notice_waiting_due();
        if self.screen == Screen::Projects
            && (self.projects_stale || self.projects.signature != projects::signature(&self.projects_dir()))
        {
            self.refresh_projects();
        }
    }

    /// 期日が近い (2 日後まで。過ぎたものを含む) Waiting のタスクを知らせる (Neovim 版の警告と同じ条件と文言)。
    /// 起動したとき・日付が変わったときと、まだ知らせていないタスクが加わったときに、該当するものをすべて出す。
    ///
    /// Neovim 版は 1 時間ごとに知らせ直すが、TUI の知らせは次のキー操作まで残るので、新しく該当したときだけにする。
    fn notice_waiting_due(&mut self) {
        let (day, noticed) = &mut self.waiting_noticed;
        if *day != Some(self.today) {
            *day = Some(self.today);
            noticed.clear();
        }
        let soon = view::waiting_due_soon(&self.todo_doc, &self.ws.names, self.today, WAITING_WARNING_DAYS);
        // タスクのキーではなく本文と期日で覚える。ID の無いタスクは、Neovim 版が ID を付けるとキーが変わり、
        // 同じタスクを知らせ直してしまうため。期日を変えたときは知らせ直す。
        let keys: Vec<String> = soon
            .iter()
            .map(|e| format!("{}\0{}", e.task().content, e.task().due.as_deref().unwrap_or_default()))
            .collect();
        if keys.iter().all(|k| noticed.contains(k)) {
            return;
        }
        noticed.extend(keys);
        let lines: Vec<String> = soon
            .iter()
            .map(|e| {
                format!(
                    "- {} (due: {})",
                    e.task().content,
                    e.task().due.as_deref().unwrap_or_default()
                )
            })
            .collect();
        self.message(Level::Warning, format!("Waiting tasks due soon:\n{}", lines.join("\n")));
    }

    /// `auto_move_inbox_to_today` が偽のとき、期日が来ても inbox.md に残っているタスクを知らせる。
    /// 1 日に 1 回 (起動したときと日付が変わったとき)、その日の日次の整理が済んでから確かめる。
    /// 日次の整理が同じタスクを警告していれば重ねない。
    fn notice_inbox_due(&mut self) {
        if self.ws.auto_move_inbox_to_today || self.rollover_pending || self.inbox_noticed == Some(self.today) {
            return;
        }
        self.inbox_noticed = Some(self.today);
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
    fn check_date(&mut self) {
        let today = (self.clock)();
        if today != self.today {
            self.today = today;
            self.rollover_pending = true;
            self.reload();
        }
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
            Mode::ChooseLink { .. } => return self.handle_choose_link(key),
            Mode::FixEdit { .. } => return self.handle_fix_edit(key),
            Mode::NewProject { .. } => return self.handle_new_project(key),
        }
        if self.screen == Screen::Search && self.search.typing {
            return self.handle_search_input(key);
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
            Mode::WaitInput { input, .. } | Mode::NewProject { input } => input.paste(text),
            Mode::Form(form) => form.paste(text),
            Mode::Normal if self.screen == Screen::Search && self.search.typing => {
                self.search.input.paste(text);
                self.search.run(false);
            }
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
            // カンバンでは列を移る。Queue などから押したときは一覧へ戻ってタブを選ぶ。
            Action::NextTab if self.screen == Screen::Kanban => {
                self.kanban.focus = (self.kanban.focus + 1).min(kanban::COLUMNS - 1);
            }
            Action::PrevTab if self.screen == Screen::Kanban => {
                self.kanban.focus = self.kanban.focus.saturating_sub(1);
            }
            Action::NextTab => {
                self.current = (self.current + 1) % TAB_COUNT;
                self.screen = Screen::List;
            }
            Action::PrevTab => {
                self.current = (self.current + TAB_COUNT - 1) % TAB_COUNT;
                self.screen = Screen::List;
            }
            Action::OpenKanban => {
                if self.screen != Screen::Kanban && self.kanban_stale {
                    self.kanban.rebuild(&self.tabs);
                    self.kanban_stale = false;
                }
                self.screen = Screen::Kanban;
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
                    self.screen = Screen::Queue;
                    if self.queue_stale {
                        self.refresh_queue();
                    }
                }
            }
            Action::SwitchView => match self.screen {
                Screen::Queue => {
                    self.queue.wait = !self.queue.wait;
                    self.queue.offset = 0;
                    // 選んでいたタスクが切り替えた先にもあれば選んだまま、無ければ先頭。
                    if !self.refresh_queue() {
                        self.queue.cursor = 0;
                    }
                }
                Screen::Search => {
                    self.search.status = self.search.status.next();
                    self.search.run(true);
                }
                Screen::Projects => {
                    self.projects.archived = !self.projects.archived;
                    self.projects.cursor = 0;
                    self.projects.offset = 0;
                    self.refresh_projects();
                }
                Screen::List | Screen::Kanban => {}
            },
            Action::Search => self.open_search(),
            Action::OpenLink => self.open_link(),
            Action::Open if self.screen == Screen::Projects => self.search_project(),
            Action::Open => self.open_in_list(),
            Action::OpenProjects => self.open_projects(),
            Action::Archive => self.archive_project(),
            Action::EditFile if self.screen == Screen::Projects => self.edit_project(),
            Action::Add if self.screen == Screen::Projects => {
                self.mode = Mode::NewProject {
                    input: LineInput::new(""),
                };
            }
            Action::EditFile => self.start_edit(),
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
            Screen::Search => (self.search.results.len(), &mut self.search.cursor),
            Screen::Kanban => {
                let focus = self.kanban.focus;
                (self.kanban.columns[focus].len(), &mut self.kanban.cursors[focus])
            }
            Screen::Projects => (self.projects.rows.len(), &mut self.projects.cursor),
        };
        if len == 0 {
            return;
        }
        *cursor = cursor.saturating_add_signed(delta).min(len - 1);
    }

    /// 今の画面で選んでいるタスク。
    pub fn selected(&self) -> Option<Selected<'_>> {
        match self.screen {
            Screen::List => {
                let tab = self.tab();
                self.view().selected().map(|row| Selected {
                    row,
                    file: tab.file(),
                    section: match tab {
                        Tab::Section(key) => Some(key),
                        _ => None,
                    },
                })
            }
            Screen::Queue => self.queue.selected().map(|item| Selected {
                row: &item.row,
                file: item.file,
                section: item.section,
            }),
            Screen::Search => self.search.selected().map(|(entry, _)| Selected {
                row: &entry.row,
                file: entry.file,
                section: entry.section,
            }),
            Screen::Kanban => self.kanban.selected().map(|card| Selected {
                row: &card.row,
                file: card.file,
                section: card.section,
            }),
            // 並んでいるのはタスクではなくプロジェクト。
            Screen::Projects => None,
        }
    }

    /// 操作の対象にできる選択中のタスク。できなければ理由を知らせて `None`。
    fn target(&mut self) -> Option<Row> {
        let selected = self.selected()?;
        if !selected.writable() {
            let text = format!("tasks in {} cannot be changed", selected.file.file_name());
            self.message(Level::Warning, text);
            return None;
        }
        Some(selected.row.clone())
    }

    /// Queue で選んでいるタスクを、一覧のタブで選ぶ。
    /// Queue・検索で選んでいるタスクを、一覧のタブで選ぶ。
    fn open_in_list(&mut self) {
        if self.screen == Screen::List {
            return;
        }
        let Some(key) = self.selected().map(|s| s.row.key.clone()) else {
            return;
        };
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
        self.queue_stale = false;
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
        // 続けて押すキー (tui.toml で `down = "gj"` など) も使えるよう、押しかけのキーは一覧と共有する。
        let delta: isize = match self.keymap.feed(&mut self.pending, key) {
            Fed::Pending => return,
            Fed::Action(Action::Down) => 1,
            Fed::Action(Action::Up) => -1,
            Fed::Action(Action::HalfPageDown | Action::PageDown) => self.page_height.max(2) as isize / 2,
            Fed::Action(Action::HalfPageUp | Action::PageUp) => -(self.page_height.max(2) as isize / 2),
            _ => {
                self.mode = Mode::Normal;
                self.help_scroll = 0;
                self.pending.clear();
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
            Screen::Queue | Screen::Search | Screen::Projects => Tab::Inbox,
            // 選んでいる列のセクションへ入れる (Done の列からは inbox へ)。
            Screen::Kanban => kanban::SECTION_COLUMNS
                .get(self.kanban.focus)
                .map_or(Tab::Inbox, |key| Tab::Section(*key)),
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
        // アーカイブしたプロジェクトは候補から外す (Neovim 版と同じ)。読めないファイルは候補にしないだけ。
        let files = project::list(&self.ws.data_dir, &mut Vec::new()).unwrap_or_default();
        for file in &files {
            if file.archived {
                projects.remove(&file.tag);
            } else {
                projects.insert(file.tag.clone());
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
            content: row.task.content.clone(),
        };
    }

    /// 選んでいるタスクのファイル (選んでいなければ開いているタブ・列のファイル) のコピーを作り、
    /// エディタで開くよう求める。カーソルはそのタスクの行に置く。
    fn start_edit(&mut self) {
        let (file, key) = match self.selected() {
            Some(selected) => (selected.file, Some(selected.row.key.clone())),
            None => match self.screen {
                Screen::List => (self.tab().file(), None),
                Screen::Kanban if self.kanban.focus >= kanban::SECTION_COLUMNS.len() => (FileKind::Done, None),
                _ => (FileKind::Todo, None),
            },
        };
        self.edit_file(EditTarget::Data(file), key);
    }

    /// `target` のコピーを作り、エディタで開くよう求める。`key` のタスクがあれば、カーソルをその行に置く。
    /// 無いファイルは雛形で作ってから開く (書き戻すときに、作られたファイルと食い違わないように)。
    fn edit_file(&mut self, target: EditTarget, key: Option<String>) {
        let path = target.path(&self.ws.data_dir);
        let mut original = store::read_raw(&path);
        if matches!(&original, Ok(raw) if raw.stamp.is_none()) {
            let created = self.write(false, |ws| match &target {
                // 無いデータファイルは、書き込む前の準備 (`ensure_files`) で作られる。
                EditTarget::Data(_) => Ok(()),
                EditTarget::Project(tag) => match ops::create_project(&ws.data_dir, tag, ws.today) {
                    Ok(_) | Err(ProjectError::Exists(_)) => Ok(()),
                    Err(e) => Err(e.into()),
                },
            });
            if created.is_none() {
                return;
            }
            original = store::read_raw(&path);
        }
        let original = match original {
            Ok(raw) => raw,
            Err(e) => return self.message(Level::Error, e.to_string()),
        };
        let Ok(text) = std::str::from_utf8(&original.bytes) else {
            let text = format!("{} is not valid UTF-8", target.name());
            return self.message(Level::Error, text);
        };
        let line = key.and_then(|key| {
            store::split_lines(text)
                .iter()
                .position(|l| Task::parse(l).is_some_and(|t| task_key(&t, l) == key))
                .map(|i| i + 1)
        });
        // ファイル名は元と同じにする (エディタの色分けが効き、どのファイルか分かるように)。
        let session = tempfile::Builder::new()
            .prefix("gtodo-edit-")
            .tempdir()
            .and_then(|dir| {
                let copy = dir.path().join(target.file_name());
                fs::write(&copy, &original.bytes)?;
                Ok(EditSession {
                    target,
                    original,
                    dir,
                    copy,
                    line,
                })
            });
        match session {
            Ok(session) => {
                self.edit = Some(session);
                self.editor_requested = true;
            }
            Err(e) => self.message(Level::Error, format!("failed to make a copy to edit: {e}")),
        }
    }

    /// エディタで開くよう求められているか。
    pub fn editor_requested(&self) -> bool {
        self.editor_requested
    }

    /// エディタでコピーを開いて閉じるまで待ち、編集した内容を確かめて書き戻す。端末は呼び出し元が明け渡す。
    ///
    /// エディタを待つ間はロックを持たない (Neovim 版は 60 秒より古いロックを残骸とみなすため)。
    pub fn run_editor(&mut self) {
        self.editor_requested = false;
        let Some(session) = self.edit.take() else { return };
        let name = session.target.name();
        self.messages.clear();
        match (self.editor)(&session.copy, session.line) {
            Ok(true) => {}
            Ok(false) => {
                let text = format!(
                    "the editor ({}) exited with an error; {name} was not changed",
                    self.editor_name
                );
                self.message(Level::Warning, text);
                return self.reload();
            }
            Err(e) => {
                let text = format!("failed to start the editor ({}): {e}", self.editor_name);
                self.message(Level::Error, text);
                return self.reload();
            }
        }
        let edited = match fs::read(&session.copy) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.message(Level::Error, format!("failed to read the edited copy: {e}"));
                return self.reload();
            }
        };
        if edited == session.original.bytes {
            self.message(Level::Info, format!("{name} was not changed"));
            return self.reload();
        }
        enum Saved {
            Yes,
            Invalid(Vec<String>),
            Changed,
        }
        let saved = self.write(false, |ws| {
            match ops::write_edited(&ws.data_dir, &ws.names, &session.target, &session.original, &edited) {
                Ok(()) => Ok(Saved::Yes),
                Err(EditedError::Invalid(problems)) => Ok(Saved::Invalid(problems)),
                Err(EditedError::Write(WriteError::Changed { .. })) => Ok(Saved::Changed),
                Err(e) => Err(e.into()),
            }
        });
        match saved {
            Some(Saved::Yes) => self.message(Level::Info, format!("saved {name}")),
            Some(Saved::Invalid(problems)) => {
                self.message(Level::Error, format!("{name} was not saved: {}", problems.join("; ")));
                self.mode = Mode::FixEdit {
                    name: name.clone(),
                    problems,
                };
                self.edit = Some(session);
            }
            // 書けなかった。編集した内容を失わないよう、コピーを残して場所を知らせる。
            // 場所は切り詰められないよう、行を分けて出す。
            Some(Saved::Changed) => {
                let kept = session.dir.keep().join(session.target.file_name());
                let text = format!(
                    "{name} was changed while you were editing and was not saved. Your copy:\n{}",
                    kept.display()
                );
                self.message(Level::Error, text);
                self.kept_copies.push(kept);
            }
            None => {
                let kept = session.dir.keep().join(session.target.file_name());
                let text = format!("{name} was not saved. Your copy:\n{}", kept.display());
                self.message(Level::Warning, text);
                self.kept_copies.push(kept);
            }
        }
    }

    /// 保存時の検証に通らなかったとき。`y` でもう一度編集し、`n`・`Esc` で編集した内容を捨てる。
    fn handle_fix_edit(&mut self, key: Key) {
        let Mode::FixEdit { name, .. } = &self.mode else {
            return;
        };
        let name = name.clone();
        if key.ctrl || key.alt {
            return;
        }
        match key.code {
            KeyCode::Char('y' | 'Y') => {
                self.mode = Mode::Normal;
                self.messages.clear();
                self.editor_requested = true;
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.edit = None;
                self.messages.clear();
                self.message(Level::Info, format!("discarded the changes to {name}"));
            }
            _ => {}
        }
    }

    fn projects_dir(&self) -> PathBuf {
        self.ws.data_dir.root().join(project::DIR)
    }

    /// プロジェクト画面を開く。選んでいるタスクにプロジェクトがあれば、それを選ぶ
    /// (アーカイブしたプロジェクトなら、アーカイブしたものの表示で)。
    fn open_projects(&mut self) {
        let wanted = self
            .selected()
            .and_then(|s| s.row.task.project.clone())
            .filter(|p| !p.is_empty());
        let reopen = self.screen != Screen::Projects;
        self.screen = Screen::Projects;
        match wanted {
            Some(tag) => self.open_projects_at(&tag),
            None => {
                self.refresh_projects();
                if reopen {
                    self.projects.cursor = 0;
                    self.projects.offset = 0;
                }
            }
        }
    }

    /// `tag` のプロジェクトを選ぶ (アーカイブしたものなら、アーカイブしたものの表示で)。
    fn open_projects_at(&mut self, tag: &str) {
        self.refresh_projects();
        if !self.projects.select_tag(tag) {
            self.projects.archived = !self.projects.archived;
            self.refresh_projects();
            if !self.projects.select_tag(tag) {
                self.projects.archived = !self.projects.archived;
                self.refresh_projects();
            }
        }
    }

    /// プロジェクトの一覧と進捗を作り直す。選んでいたプロジェクトは選んだまま。
    fn refresh_projects(&mut self) {
        let mut warnings = Vec::new();
        let files = project::list(&self.ws.data_dir, &mut warnings).unwrap_or_else(|e| {
            warnings.push(format!("failed to read {}: {e}", self.projects_dir().display()));
            Vec::new()
        });
        let done = &self.tabs[DONE_TAB].rows;
        let progress = project::progress(
            project::tasks(&self.inbox_doc).chain(project::tasks(&self.todo_doc)),
            done.iter().map(|r| r.task.as_ref()),
        );
        let keep = self.projects.selected().map(|r| r.tag.clone());
        self.projects.rows = projects::rows(&files, &progress, self.projects.archived);
        self.projects.signature = projects::signature(&self.projects_dir());
        self.projects_stale = false;
        if !keep.is_some_and(|tag| self.projects.select_tag(&tag)) {
            self.projects.cursor = self.projects.cursor.min(self.projects.rows.len().saturating_sub(1));
        }
        for w in warnings {
            self.config_warning(w);
        }
    }

    /// 選んでいるプロジェクトのタスクを検索で開く (`+tag`)。
    fn search_project(&mut self) {
        let Some(tag) = self.projects.selected().map(|r| r.tag.clone()) else {
            return;
        };
        self.open_search();
        self.search.input = LineInput::new(&format!("+{tag} "));
        self.search.run(false);
        // 結果があれば、一覧と同じキーで操作できるよう結果の中へ移る。
        self.search.typing = self.search.results.is_empty();
    }

    /// 選んでいるプロジェクトのファイルを `$EDITOR` で開く (無ければ作ってから)。
    fn edit_project(&mut self) {
        let Some(row) = self.projects.selected() else {
            return;
        };
        // 作るときだけ、ファイル名として使えるかを確かめる (Neovim 版で作ったファイルは名前を直していないことがある)。
        let checked = if row.has_file {
            project::check_existing(&row.tag)
        } else {
            project::check_tag(&row.tag)
        };
        match checked {
            Ok(tag) => {
                self.projects_stale = true;
                self.edit_file(EditTarget::Project(tag), None);
            }
            Err(e) => self.message(Level::Error, e.to_string()),
        }
    }

    /// 新しいプロジェクトの名前の入力。`Enter` で作って `$EDITOR` で開き、`Esc` で取り消す。
    fn handle_new_project(&mut self, key: Key) {
        let Mode::NewProject { input } = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let tag = match project::check_tag(input.text()) {
                    Ok(tag) => tag,
                    // 直せるように、入力は残す。
                    Err(e) => return self.message(Level::Error, e.to_string()),
                };
                self.mode = Mode::Normal;
                self.messages.clear();
                if project::path(&self.ws.data_dir, &tag).exists() {
                    // すでにあるなら、作らずにそれを選ぶ。
                    self.message(Level::Info, format!("{}/{tag}.md already exists", project::DIR));
                    self.open_projects_at(&tag);
                    return;
                }
                let created = self.write(false, |ws| Ok(ops::create_project(&ws.data_dir, &tag, ws.today)?));
                if created.is_some() {
                    self.message(Level::Info, format!("created {}/{tag}.md", project::DIR));
                    self.projects.archived = false;
                    self.refresh_projects();
                    self.projects.select_tag(&tag);
                    self.edit_file(EditTarget::Project(tag), None);
                }
            }
            _ => {
                input.handle(key);
            }
        }
    }

    /// 選んでいるプロジェクトをアーカイブする (アーカイブしたものの表示では復元する)。
    fn archive_project(&mut self) {
        if self.screen != Screen::Projects {
            return;
        }
        let Some(row) = self.projects.selected() else { return };
        let tag = row.tag.clone();
        if !row.has_file {
            let text = format!(
                "{}/{tag}.md does not exist, so there is nothing to archive",
                project::DIR
            );
            return self.message(Level::Warning, text);
        }
        let (status, done, already) = if self.projects.archived {
            (project::ACTIVE, "restored", "active")
        } else {
            (project::ARCHIVED, "archived", "archived")
        };
        self.messages.clear();
        let changed = self.write(false, |ws| Ok(ops::set_project_status(&ws.data_dir, &tag, status)?));
        match changed {
            Some(true) => self.message(Level::Info, format!("{done} {tag}")),
            Some(false) => self.message(Level::Info, format!("{tag} is already {already}")),
            None => {}
        }
        self.refresh_projects();
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

    /// 選んでいるタスクのリンクを開く。複数あれば選ばせる。
    fn open_link(&mut self) {
        let Some(selected) = self.selected() else { return };
        let mut links = markdown::links(&selected.row.task.content);
        match links.len() {
            0 => self.message(Level::Warning, "the task has no links".to_string()),
            1 => self.open_url(&links.remove(0)),
            _ => self.mode = Mode::ChooseLink { links, cursor: 0 },
        }
    }

    /// 開くリンクを選ぶ。矢印 (と下へ・上への動作のキー) で選んで `Enter`、または番号。
    /// それ以外のキーで取り消す。
    fn handle_choose_link(&mut self, key: Key) {
        let Mode::ChooseLink { links, cursor } = &mut self.mode else {
            return;
        };
        let step = match key.code {
            KeyCode::Down => Some(1),
            KeyCode::Up => Some(-1),
            KeyCode::Enter | KeyCode::Char('1'..='9') => None,
            _ => match self.keymap.feed(&mut self.pending, key) {
                Fed::Pending => return,
                Fed::Action(Action::Down) => Some(1),
                Fed::Action(Action::Up) => Some(-1),
                _ => None,
            },
        };
        if let Some(step) = step {
            *cursor = cursor.saturating_add_signed(step).min(links.len() - 1);
            return;
        }
        let chosen = match key.code {
            KeyCode::Enter => links.get(*cursor).cloned(),
            KeyCode::Char(c @ '1'..='9') => links.get(c as usize - '1' as usize).cloned(),
            _ => None,
        };
        self.pending.clear();
        self.mode = Mode::Normal;
        if let Some(url) = chosen {
            self.open_url(&url);
        }
    }

    /// ブラウザなどで開く。開くのは http(s) と mailto だけ (本文に書かれたファイルのパスなどを、
    /// 関連付けられたアプリで実行しないため)。
    fn open_url(&mut self, url: &str) {
        let lower = url.to_ascii_lowercase();
        if !["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
            self.message(
                Level::Warning,
                format!("only http(s) and mailto links are opened: {url}"),
            );
            return;
        }
        match (self.opener)(url) {
            Ok(()) => self.message(Level::Info, format!("opened {url}")),
            Err(e) => self.message(Level::Error, format!("failed to open {url}: {e}")),
        }
    }

    /// 検索を開く。前の検索語と状態は残す (続きから絞り込めるように)。
    fn open_search(&mut self) {
        if self.screen != Screen::Search && self.search_stale {
            self.search.run(true);
            self.search_stale = false;
        }
        self.screen = Screen::Search;
        self.search.typing = true;
    }

    /// 検索の入力欄にいる間のキー。文字は入力欄へ入れ、打つたびに絞り込む。
    fn handle_search_input(&mut self, key: Key) {
        match (key.code, key.ctrl) {
            (KeyCode::Esc, _) => {
                self.screen = Screen::List;
                self.messages.clear();
            }
            // 結果の中へ移り、一覧と同じキーで操作する。
            (KeyCode::Enter, _) => {
                if !self.search.results.is_empty() {
                    self.search.typing = false;
                }
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => self.move_cursor(1),
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => self.move_cursor(-1),
            (KeyCode::Tab, _) => self.perform(Action::SwitchView),
            _ => {
                let before = self.search.input.text().to_string();
                self.search.input.handle(key);
                if self.search.input.text() != before {
                    self.search.run(false);
                }
            }
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
            Screen::Search => self.search.select_key(key),
            Screen::Kanban => self.kanban.select_key(key),
            Screen::Projects => false,
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
            Ok((value, prepared)) => {
                self.note_prepared(prepared);
                Some(value)
            }
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
        match result {
            Ok(((), prepared)) => self.note_prepared(prepared),
            Err(e) => self.message(Level::Error, format!("{e:#}")),
        }
        self.reload();
    }

    /// 書き込む前の準備で日次の整理が inbox.md のタスクを警告したら、その日はもう知らせない。
    fn note_prepared(&mut self, prepared: writing::Prepared) {
        if prepared.inbox_due_warned {
            self.inbox_noticed = Some(self.today);
        }
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
        let mut search_changed = false;
        let mut kanban_changed = false;
        for (i, kind) in WATCHED_DOCS.iter().enumerate() {
            if reparse || now[i] != self.stamps[i] {
                self.load(*kind);
                queue_changed |= matches!(kind, FileKind::Inbox | FileKind::Todo);
                search_changed |= SEARCH_FILES.contains(kind);
                // カンバンに出るのは todo.md と done.md (inbox.md は出さない)。
                kanban_changed |= matches!(kind, FileKind::Todo | FileKind::Done);
                self.projects_stale |= matches!(kind, FileKind::Inbox | FileKind::Todo | FileKind::Done);
            }
        }
        if kanban_changed {
            if self.screen == Screen::Kanban {
                self.kanban.rebuild(&self.tabs);
            } else {
                self.kanban_stale = true;
            }
        }
        if search_changed {
            if self.screen == Screen::Search {
                self.search.run(true);
            } else {
                self.search_stale = true;
            }
        }
        // inbox.md と todo.md を読み終えてから作り直す。片方ずつ作り直すと、inbox.md から
        // todo.md へ移したタスクが途中でどちらにも無くなり、選んでいたタスクを見失う。
        if queue_changed || reparse {
            // 開いていない間は作り直さない (開くときに作る)。
            if self.screen == Screen::Queue {
                self.refresh_queue();
            } else {
                self.queue_stale = true;
            }
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
        // 検索のための正規化は、読み込んだときに 1 回だけ行う (打つたびには行わない)。
        // 一覧のタブの行は検索の対象から作り、タスクを共有する。
        let index = SEARCH_FILES
            .contains(&kind)
            .then(|| search::index(kind, &doc, &self.ws.names));
        for (i, tab) in TABS.iter().enumerate() {
            if tab.file() != kind {
                continue;
            }
            let view = &mut self.tabs[i];
            let selected = view.selected().map(|r| r.key.clone());
            view.rows = match &index {
                Some(entries) => rows_from(*tab, entries),
                None => rows(*tab, &doc),
            };
            if !selected.is_some_and(|k| view.select_key(&k)) {
                view.clamp();
            }
        }
        if let Some(entries) = index {
            self.search.set_part(kind, entries);
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

/// 検索の対象でないファイル (cancelled.md) のタブに出すタスク。新しいものから (ファイルの末尾が新しい)。
fn rows(tab: Tab, doc: &Document) -> Vec<Row> {
    let mut rows: Vec<Row> = entries(tab.file(), doc)
        .map(|e| Row {
            key: task_key(e.task(), &e.item.line),
            task: Rc::new(e.task().clone()),
        })
        .collect();
    rows.reverse();
    rows
}

/// 検索の対象から、タブに出す行を作る (検索の対象は、一覧と同じ順に並べてある)。
fn rows_from(tab: Tab, entries: &[search::Entry]) -> Vec<Row> {
    entries
        .iter()
        .filter(|e| match tab {
            Tab::Section(key) => e.section == Some(key),
            _ => true,
        })
        .map(|e| e.row.clone())
        .collect()
}
