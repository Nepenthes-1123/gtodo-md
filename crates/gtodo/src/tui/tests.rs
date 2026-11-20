//! TUI のテスト。一時ディレクトリのデータに対してキーを送り、ファイルと画面を確かめる。
//!
//! 画面は ratatui の `TestBackend` に描画して、文字だけを比べる (色や太字は見ない)。

use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::NaiveDate;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use super::app::{App, Level, Mode, Screen};
use super::draw::{self, Theme};
use super::kanban;
use super::keymap::Keymap;

const TODAY: &str = "2026-11-16";

fn today() -> NaiveDate {
    date(TODAY)
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

struct Fixture {
    dir: tempfile::TempDir,
    /// App が読む今日の日付。
    today: Rc<Cell<NaiveDate>>,
}

impl Fixture {
    fn new() -> Fixture {
        let f = Fixture {
            dir: tempfile::tempdir().unwrap(),
            today: Rc::new(Cell::new(today())),
        };
        f.write("inbox.md", "# Inbox\n\n- [ ] 受信1 id:111111\n");
        f.write(
            "todo.md",
            "# Todo\n\n## Today\n\n- [ ] (A) 今日1 +work @office due:2026-11-16 id:aaaaaa\n- [ ] 今日2 id:bbbbbb\n\n\
             ## Next\n\n- [ ] 次1 id:cccccc\n- [ ] 次2 id:dddddd\n\n## Waiting\n\n- [ ] 待ち1 wait:bob id:eeeeee\n\n## Someday\n",
        );
        f.write("done.md", "# Done\n\n## 2026-10\n\n- [x] 古い完了 completed_at:2026-10-01 id:f00001\n\n## 2026-11\n\n- [x] 新しい完了 completed_at:2026-11-02 id:f00002\n");
        f.write("cancelled.md", "# Cancelled\n");
        // 今日のロールオーバーは済んでいることにする (起動時に書き換えないため)。
        f.write(".state.json", &format!(r#"{{"last_opened":"{TODAY}"}}"#));
        f
    }

    fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    fn write(&self, name: &str, content: &str) {
        fs::write(self.dir.path().join(name), content).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }

    fn app(&self) -> App {
        let today = Rc::clone(&self.today);
        App::new(
            self.path(),
            Box::new(move || today.get()),
            Keymap::default(),
            Vec::new(),
        )
        .unwrap()
    }
}

fn press(app: &mut App, keys: &str) {
    for c in keys.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn press_code(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn press_ctrl(app: &mut App, c: char) {
    app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

/// 画面の各行 (行末の空白を除く)。
fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| draw::draw(frame, app, Theme { color: false }))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let mut line = String::new();
            let mut skip = 0;
            for x in 0..width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                skip = symbol.width().saturating_sub(1);
            }
            line.trim_end().to_string()
        })
        .collect()
}

fn footer(app: &mut App) -> String {
    screen(app, 120, 20).pop().unwrap()
}

fn selected_content(app: &App) -> String {
    app.view()
        .selected()
        .map(|r| r.task.content.clone())
        .unwrap_or_default()
}

fn last_message(app: &App) -> String {
    app.messages.last().map(|m| m.text.clone()).unwrap_or_default()
}

#[test]
fn the_screen_shows_tabs_rows_and_hints() {
    let f = Fixture::new();
    let mut app = f.app();
    assert_eq!(
        screen(&mut app, 100, 7),
        [
            " 1 Inbox 1 │ 2 Today 2 │ 3 Next 2 │ 4 Waiting 1 │ 5 Someday 0 │ 6 Done 2 │ 7 Cancelled 0",
            "────────────────────────────────────────────────────────────────────────────────────────────────────",
            "▌ [ ] (A) 今日1 +work @office  due:2026-11-16 (今日)",
            "  [ ] 今日2",
            "",
            "",
            "x 完了にする  n Next へ  w Waiting へ  s Someday へ  c キャンセル  e 編集  a 追加  Q Queue  ? ヘルプ",
        ]
    );
}

#[test]
fn long_content_is_cut_so_that_tags_stay_visible() {
    let f = Fixture::new();
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] とても長い本文がここに続いていて画面に入りきらない +work due:2026-11-20\n",
    );
    let mut app = f.app();
    let rows = screen(&mut app, 50, 5);
    assert_eq!(rows[2], "▌ [ ] とても長い本… +work  due:2026-11-20 (4日後)");
}

#[test]
fn hints_follow_the_selected_task_and_tab() {
    let f = Fixture::new();
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [x] 済み completed_at:2026-11-16 id:aaaaaa\n\n## Next\n\n## Waiting\n\n- [ ] 待ち1 wait:bob id:eeeeee\n\n## Someday\n",
    );
    let mut app = f.app();
    // 完了したタスクは移せないので、移動の案内を出さない。
    assert_eq!(
        footer(&mut app),
        "x 未完了に戻す  c キャンセル  e 編集  a 追加  o 並べ替え  Q Queue  K カンバン  P プロジェクト  ? ヘルプ  q 終了"
    );

    press(&mut app, "4");
    assert_eq!(
        footer(&mut app),
        "x 完了にする  d Today へ  n Next へ  w 待ち先を変える  s Someday へ  c キャンセル  e 編集  a 追加  Q Queue  ? ヘルプ"
    );

    // 空のタブでは、タスクの操作を出さない。
    press(&mut app, "3");
    assert_eq!(
        footer(&mut app),
        "a 追加  o 並べ替え  Q Queue  K カンバン  P プロジェクト  ? ヘルプ  q 終了"
    );

    // done.md は見るだけ。
    press(&mut app, "6");
    assert_eq!(
        footer(&mut app),
        "a 追加  Q Queue  K カンバン  P プロジェクト  ? ヘルプ  q 終了"
    );
}

#[test]
fn hints_that_do_not_fit_are_dropped_but_help_stays() {
    let f = Fixture::new();
    let mut app = f.app();
    assert_eq!(
        screen(&mut app, 60, 5)[4],
        "x 完了にする  n Next へ  w Waiting へ  a 追加  ? ヘルプ"
    );
    assert_eq!(screen(&mut app, 30, 5)[4], "x 完了にする  a 追加  ? ヘルプ");
}

#[test]
fn custom_section_names_are_used_in_tabs_and_hints() {
    let f = Fixture::new();
    f.write(
        "gtodo.json",
        r#"{"sections":{"today":"今日","next":"次","waiting":"待ち","someday":"いつか"}}"#,
    );
    f.write(
        "todo.md",
        "# Todo\n\n## 今日\n\n- [ ] 今日1 id:aaaaaa\n\n## 次\n\n## 待ち\n\n## いつか\n",
    );
    let mut app = f.app();
    let rows = screen(&mut app, 100, 5);
    assert!(
        rows[0].starts_with(" 1 Inbox 1 │ 2 今日 1 │ 3 次 0 │ 4 待ち 0 │ 5 いつか 0"),
        "{}",
        rows[0]
    );
    assert_eq!(
        rows[4],
        "x 完了にする  n 次 へ  w 待ち へ  s いつか へ  c キャンセル  e 編集  a 追加  Q Queue  ? ヘルプ"
    );
}

#[test]
fn vim_motions_move_the_cursor_and_tabs() {
    let f = Fixture::new();
    let lines: String = (1..=30).map(|i| format!("- [ ] 次{i} id:{i:06x}\n")).collect();
    f.write(
        "todo.md",
        &format!("# Todo\n\n## Today\n\n## Next\n\n{lines}\n## Waiting\n\n## Someday\n"),
    );
    let mut app = f.app();
    screen(&mut app, 80, 13); // 一覧の高さ (10 行) を決める
    assert_eq!(app.page_height, 10);

    press(&mut app, "l");
    assert_eq!(selected_content(&app), "次1");
    press(&mut app, "jj");
    assert_eq!(selected_content(&app), "次3");
    press(&mut app, "k");
    assert_eq!(selected_content(&app), "次2");
    press(&mut app, "G");
    assert_eq!(selected_content(&app), "次30");
    press(&mut app, "j");
    assert_eq!(selected_content(&app), "次30");
    press(&mut app, "gg");
    assert_eq!(selected_content(&app), "次1");
    press_ctrl(&mut app, 'd');
    assert_eq!(selected_content(&app), "次6");
    press_ctrl(&mut app, 'f');
    assert_eq!(selected_content(&app), "次16");
    press_ctrl(&mut app, 'u');
    assert_eq!(selected_content(&app), "次11");
    press_ctrl(&mut app, 'b');
    assert_eq!(selected_content(&app), "次1");
    press_code(&mut app, KeyCode::Down);
    assert_eq!(selected_content(&app), "次2");

    // 選んでいる行が見えるように表示位置が動く。
    press(&mut app, "G");
    let rows = screen(&mut app, 80, 13);
    assert_eq!(rows[11], "▌ [ ] 次30");
    assert_eq!(rows[2], "  [ ] 次21");

    // タブ: gt/gT・h/l・数字。端では反対側へ回る。
    press(&mut app, "gt");
    assert_eq!(app.current, 3);
    press(&mut app, "gTgT");
    assert_eq!(app.current, 1);
    press(&mut app, "hh");
    assert_eq!(app.current, 6);
    press(&mut app, "l");
    assert_eq!(app.current, 0);
    press(&mut app, "7");
    assert_eq!(app.current, 6);
    press_code(&mut app, KeyCode::Left);
    assert_eq!(app.current, 5);
    // タブごとに選んでいる行を覚えている。
    press(&mut app, "3");
    assert_eq!(selected_content(&app), "次30");
}

#[test]
fn a_pending_key_shows_what_can_follow_and_esc_cancels_it() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "g");
    assert_eq!(
        footer(&mut app),
        "gg 先頭へ  gt 次のタブ  gT 前のタブ  gx リンクを開く (複数あれば選ぶ)  Esc 取り消す"
    );
    press_code(&mut app, KeyCode::Esc);
    assert!(app.pending.is_empty());
    // Esc は戻る・取り消すだけで、何度押しても終了しない (lazygit と同じ)。終了は q。
    press_code(&mut app, KeyCode::Esc);
    assert!(!app.should_quit());
    press(&mut app, "q");
    assert!(app.should_quit());
}

#[test]
fn x_toggles_completion_and_keeps_the_task_selected() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "j");
    press(&mut app, "x");
    assert!(
        f.read("todo.md")
            .contains("- [x] 今日2 completed_at:2026-11-16 id:bbbbbb")
    );
    assert_eq!(last_message(&app), "completed: 今日2");
    assert_eq!(selected_content(&app), "今日2");
    assert_eq!(
        app.view().selected().unwrap().task.status,
        gtodo_core::task::Status::Done
    );

    press(&mut app, "x");
    assert!(f.read("todo.md").contains("- [ ] 今日2 id:bbbbbb"));
    assert_eq!(last_message(&app), "reopened: 今日2");
    assert_eq!(selected_content(&app), "今日2");
}

#[test]
fn a_task_without_an_id_is_found_and_selected_after_toggling() {
    let f = Fixture::new();
    f.write("inbox.md", "# Inbox\n\n- [ ] IDなし1\n- [ ] IDなし2\n");
    let mut app = f.app();
    press(&mut app, "1j");
    press(&mut app, "x");
    let inbox = f.read("inbox.md");
    assert!(inbox.contains("- [ ] IDなし1\n"), "{inbox}");
    assert!(inbox.contains("- [x] IDなし2 completed_at:2026-11-16 id:"), "{inbox}");
    assert_eq!(selected_content(&app), "IDなし2");
}

#[test]
fn moving_a_task_keeps_the_cursor_on_the_same_row() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "3");
    press(&mut app, "d");
    let todo = f.read("todo.md");
    assert!(todo.find("次1").unwrap() < todo.find("## Next").unwrap(), "{todo}");
    assert_eq!(last_message(&app), "moved to Today: 次1");
    assert_eq!(selected_content(&app), "次2");
    assert_eq!(app.tabs[1].rows.len(), 3);

    // すでにそのセクションにある。
    press(&mut app, "n");
    assert_eq!(last_message(&app), "already in Next: 次2");
}

#[test]
fn completed_tasks_are_not_moved() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "x");
    let before = f.read("todo.md");
    press(&mut app, "n");
    assert_eq!(f.read("todo.md"), before);
    assert_eq!(app.messages.last().unwrap().level, Level::Warning);
    assert_eq!(last_message(&app), "completed tasks cannot be moved");
}

#[test]
fn w_asks_who_you_are_waiting_for() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "w");
    assert!(matches!(app.mode, Mode::WaitInput { .. }));
    let rows = screen(&mut app, 80, 8);
    assert_eq!(rows[6], "→ Waiting: 今日1");
    assert_eq!(rows[7], "Waiting for (empty to skip/remove):");

    // 入力中のキーは一覧の操作にならない。
    press(&mut app, "alice");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    let todo = f.read("todo.md");
    let waiting = todo.find("## Waiting").unwrap();
    assert!(
        todo[waiting..].contains("今日1 +work @office due:2026-11-16 wait:alice id:aaaaaa"),
        "{todo}"
    );
    assert_eq!(last_message(&app), "moved to Waiting: 今日1");
}

#[test]
fn w_in_waiting_changes_the_wait_with_the_current_value_filled_in() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "4w");
    let Mode::WaitInput { input, .. } = &app.mode else {
        panic!("not asking");
    };
    assert_eq!(input.text(), "bob");
    press_code(&mut app, KeyCode::Backspace);
    press_code(&mut app, KeyCode::Backspace);
    press_code(&mut app, KeyCode::Backspace);
    press(&mut app, "carol");
    press_code(&mut app, KeyCode::Enter);
    assert!(f.read("todo.md").contains("- [ ] 待ち1 wait:carol id:eeeeee"));
    assert_eq!(last_message(&app), "set wait:carol: 待ち1");
    assert_eq!(selected_content(&app), "待ち1");

    // 空にすると待ち先を消す。
    press(&mut app, "w");
    press_ctrl(&mut app, 'u');
    press_code(&mut app, KeyCode::Enter);
    assert!(f.read("todo.md").contains("- [ ] 待ち1 id:eeeeee"));
}

#[test]
fn an_unwritable_wait_keeps_the_input_open_and_esc_writes_nothing() {
    let f = Fixture::new();
    let mut app = f.app();
    let before = f.read("todo.md");
    press(&mut app, "w");
    press(&mut app, "bob smith");
    press_code(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::WaitInput { .. }));
    assert_eq!(app.messages.last().unwrap().level, Level::Error);
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(f.read("todo.md"), before);
}

#[test]
fn c_cancels_only_after_confirmation() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "c");
    assert_eq!(footer(&mut app), "Cancel this task? [y/N]");
    press(&mut app, "n");
    assert_eq!(app.mode, Mode::Normal);
    assert!(f.read("todo.md").contains("今日1"));

    press(&mut app, "c");
    press_code(&mut app, KeyCode::Enter);
    assert!(f.read("todo.md").contains("今日1"), "anything but y keeps the task");

    press(&mut app, "cy");
    assert!(!f.read("todo.md").contains("今日1"));
    assert!(f.read("cancelled.md").contains("今日1"));
    assert_eq!(last_message(&app), "cancelled: 今日1");
    assert_eq!(selected_content(&app), "今日2");
    press(&mut app, "7");
    assert_eq!(selected_content(&app), "今日1");
}

#[test]
fn done_and_cancelled_are_read_only_and_newest_first() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "6");
    let rows = screen(&mut app, 60, 6);
    assert_eq!(rows[2], "▌ [x] 新しい完了  2026-11-02");
    assert_eq!(rows[3], "  [x] 古い完了  2026-10-01");
    let before = (f.read("done.md"), f.read("todo.md"));
    for key in ["x", "d", "c", "w"] {
        press(&mut app, key);
        assert_eq!(app.mode, Mode::Normal, "{key}");
        assert_eq!(last_message(&app), "tasks in done.md cannot be changed", "{key}");
    }
    // 並べ替えは一覧のタブでだけ (案内にも出さない)。
    press(&mut app, "o");
    assert_eq!((f.read("done.md"), f.read("todo.md")), before);
}

#[test]
fn o_moves_due_tasks_and_sorts() {
    let f = Fixture::new();
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n## Next\n\n- [ ] 後 id:cccccc\n- [ ] 期日あり due:2026-11-16 id:dddddd\n\n## Waiting\n\n## Someday\n",
    );
    let mut app = f.app();
    press(&mut app, "o");
    let messages: Vec<String> = app.messages.iter().map(|m| m.text.clone()).collect();
    assert_eq!(messages, ["moved 1 due task(s) to Today", "sorted todo.md"]);
    assert_eq!(selected_content(&app), "期日あり");
}

#[test]
fn messages_are_cleared_by_the_next_action() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "x");
    assert!(!app.messages.is_empty());
    press(&mut app, "j");
    assert!(app.messages.is_empty());
}

#[test]
fn help_lists_every_key_and_closes_on_any_key() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "?");
    let rows = screen(&mut app, 100, 40).join("\n");
    for expected in [
        "j / ↓",
        "下へ",
        "gg / Home",
        "gt / l / →",
        "Ctrl-d",
        "1〜7",
        "番号のタブへ",
        "完了と未完了を切り替える",
        "Waiting へ移す (待ち先を入れる。Waiting にあれば待ち先を変える)",
        "キャンセルする (cancelled.md へ移す。確認あり)",
        "終了 (Ctrl-c でも)",
        "押しかけのキー・入力・確認を取り消す。Queue からは一覧へ戻る (終了はしない)",
        "Queue を開く (期日・待ち先ごとの一覧)",
    ] {
        assert!(rows.contains(expected), "{expected:?} not in\n{rows}");
    }
    press(&mut app, "x");
    assert_eq!(app.mode, Mode::Normal);
    assert!(
        f.read("todo.md").contains("- [ ] (A) 今日1"),
        "the key only closes the help"
    );
}

#[test]
fn ctrl_c_quits_from_anywhere() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "w");
    press_ctrl(&mut app, 'c');
    assert!(app.should_quit());
}

#[test]
fn changes_by_other_programs_are_picked_up_on_tick() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "j");
    assert_eq!(selected_content(&app), "今日2");
    // Neovim 版が先頭にタスクを足した。選んでいるタスクはそのまま。
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 新しい id:999999\n- [ ] (A) 今日1 id:aaaaaa\n- [ ] 今日2 id:bbbbbb\n\n## Next\n\n## Waiting\n\n## Someday\n",
    );
    app.tick();
    assert_eq!(app.view().rows.len(), 3);
    assert_eq!(selected_content(&app), "今日2");
    assert_eq!(app.tabs[2].rows.len(), 0);
}

#[test]
fn an_operation_on_a_task_changed_elsewhere_fails_without_writing() {
    let f = Fixture::new();
    f.write("inbox.md", "# Inbox\n\n- [ ] IDなし\n");
    let mut app = f.app();
    press(&mut app, "1");
    // 画面に出ている行を、他のプログラムが書き換えた (一時キーが一致しなくなる)。
    f.write("inbox.md", "# Inbox\n\n- [ ] IDなし (編集済み)\n");
    press(&mut app, "x");
    assert_eq!(f.read("inbox.md"), "# Inbox\n\n- [ ] IDなし (編集済み)\n");
    assert_eq!(app.messages.last().unwrap().level, Level::Error);
    // 読み直して今の内容を出している。
    assert_eq!(selected_content(&app), "IDなし (編集済み)");
}

#[test]
fn a_new_day_runs_the_daily_rollover() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));

    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(!f.read("todo.md").contains("今日1"));
    assert!(f.read("done.md").contains("今日1"));
    assert!(f.read(".state.json").contains("2026-11-17"));
    assert_eq!(last_message(&app), "moved 1 completed task(s) to done.md");
    assert_eq!(app.tabs[5].rows[0].task.content, "今日1");
}

#[test]
fn the_rollover_is_retried_while_another_process_holds_the_lock() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "x");

    // 待たずに、次の確認でやり直す。
    f.write(".gtodo.lock", "12345 0");
    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));
    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));

    fs::remove_file(f.path().join(".gtodo.lock")).unwrap();
    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(f.read("done.md").contains("今日1"));
}

#[test]
fn startup_runs_the_rollover_once_a_day() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_opened":"2026-11-15"}"#);
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [x] 昨日の完了 completed_at:2026-11-15 id:aaaaaa\n\n## Next\n\n## Waiting\n\n## Someday\n",
    );
    let app = f.app();
    assert!(f.read("done.md").contains("昨日の完了"));
    assert_eq!(app.tabs[1].rows.len(), 0);
}

#[test]
fn startup_writes_nothing_when_the_rollover_already_ran_today() {
    let f = Fixture::new();
    let snapshot = |f: &Fixture| {
        let mut files: Vec<(String, String)> = fs::read_dir(f.path())
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let before = snapshot(&f);
    let mut app = f.app();
    app.tick();
    assert_eq!(snapshot(&f), before);
}

#[test]
fn a_broken_settings_file_stops_writes_until_it_is_fixed() {
    let f = Fixture::new();
    let mut app = f.app();
    f.write("gtodo.json", "{");
    app.tick();
    assert_eq!(app.messages.last().unwrap().level, Level::Error);
    // 前に読んだ内容は出し続ける。
    assert_eq!(app.view().rows.len(), 2);

    let before = f.read("todo.md");
    press(&mut app, "x");
    assert_eq!(f.read("todo.md"), before);
    assert!(
        last_message(&app).contains("fix it before changing tasks"),
        "{}",
        last_message(&app)
    );

    f.write("gtodo.json", "{}");
    app.tick();
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));
}

#[test]
fn warnings_from_settings_are_shown_once() {
    let f = Fixture::new();
    f.write("gtodo.json", r#"{"unknown":1}"#);
    let mut app = f.app();
    assert!(last_message(&app).contains("unknown key"), "{}", last_message(&app));
    press(&mut app, "x");
    // 書き込みで .state.json などが変わって設定を読み直しても、同じ警告は出さない。
    f.write(".state.json", r#"{"last_opened":"2026-11-16","x":1}"#);
    app.tick();
    assert!(app.messages.iter().all(|m| !m.text.contains("unknown key")));
}

#[test]
fn an_operation_right_after_midnight_uses_the_new_date() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "x");
    // 完了したタスクは下へ並び替わり、選択もついていく。
    press(&mut app, "k");
    assert_eq!(selected_content(&app), "今日2");
    // 次の確認より前に押した。
    f.today.set(date("2026-11-17"));
    press(&mut app, "x");
    let todo = f.read("todo.md");
    // 前の日に完了したものは日次の整理で done.md へ移り、新しく完了したものは今日の日付になる。
    assert!(f.read("done.md").contains("今日1"), "{todo}");
    assert!(todo.contains("- [x] 今日2 completed_at:2026-11-17"), "{todo}");
}

#[test]
fn the_wait_prompt_moves_the_task_even_if_it_was_moved_elsewhere_meanwhile() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "w");
    // 尋ねている間に、Neovim 版が Next へ移した。
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 今日2 id:bbbbbb\n\n## Next\n\n- [ ] (A) 今日1 +work @office due:2026-11-16 id:aaaaaa\n\n## Waiting\n\n## Someday\n",
    );
    app.tick();
    press(&mut app, "bob");
    press_code(&mut app, KeyCode::Enter);
    let todo = f.read("todo.md");
    let waiting = todo.find("## Waiting").unwrap();
    assert!(
        todo[waiting..].contains("今日1 +work @office due:2026-11-16 wait:bob id:aaaaaa"),
        "{todo}"
    );
}

#[test]
fn a_held_lock_fails_the_operation_quickly() {
    let f = Fixture::new();
    let mut app = f.app();
    f.write(".gtodo.lock", "12345 0");
    let before = f.read("todo.md");
    let started = std::time::Instant::now();
    press(&mut app, "x");
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(f.read("todo.md"), before);
    assert!(
        last_message(&app).contains("held by another process"),
        "{}",
        last_message(&app)
    );
}

#[test]
fn a_state_file_change_alone_does_not_reread_the_tasks() {
    let f = Fixture::new();
    let mut app = f.app();
    // 同じ長さ・同じ更新時刻で書き換える (状態からは変化が見えない)。
    let path = f.path().join("todo.md");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let todo = f.read("todo.md").replace("今日2", "今日X");
    f.write("todo.md", &todo);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();

    f.write(
        ".state.json",
        r#"{"last_opened":"2026-11-16","last_notify_due":"2026-11-16"}"#,
    );
    app.tick();
    assert_eq!(app.view().rows[1].task.content, "今日2");

    // セクション名が変われば読み直す。
    f.write(
        "gtodo.json",
        r#"{"sections":{"today":"Today","next":"Next","waiting":"Waiting","someday":"いつか"}}"#,
    );
    app.tick();
    assert_eq!(app.view().rows[1].task.content, "今日X");
}

#[test]
fn the_help_scrolls_on_a_short_terminal() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "?");
    let first = screen(&mut app, 80, 12).join("\n");
    assert!(first.contains("j / ↓"), "{first}");
    assert!(!first.contains("終了 (Ctrl-c でも)"), "{first}");
    assert!(first.ends_with("j/k スクロール  他のキー 閉じる"), "{first}");
    press(&mut app, &"j".repeat(30));
    let last = screen(&mut app, 80, 12).join("\n");
    assert!(last.contains("終了 (Ctrl-c でも)"), "{last}");
    assert!(!last.contains("j / ↓"), "{last}");
    assert_eq!(app.mode, Mode::Help);
    press(&mut app, "x");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn the_add_form_shows_fields_and_the_line_to_be_written() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "a");
    press(&mut app, "買い物 +home");
    press_code(&mut app, KeyCode::Tab);
    press(&mut app, "fri");
    let rows = screen(&mut app, 80, 20);
    let form: Vec<&str> = rows[3..16].iter().map(|r| r.trim_end()).collect();
    assert_eq!(
        form,
        [
            "  ┌ 追加 ────────────────────────────────────────────────────────────────────┐",
            "  │                                                                          │",
            "  │  本文          買い物 +home                                              │",
            "  │▌ 期日          fri  → due:2026-11-20 (4日後)                             │",
            "  │  プロジェクト  +                                                         │",
            "  │  コンテキスト  @                                                         │",
            "  │  優先度                                                                  │",
            "  │  追加先         Inbox [Today] Next  Waiting  Someday                     │",
            "  │                                                                          │",
            "  │  書き込む行                                                              │",
            "  │  - [ ] 買い物 +home due:2026-11-20 created:2026-11-16                    │",
            "  └──────────────────────────────────────────────────────────────────────────┘",
            "",
        ]
    );
    assert_eq!(rows[19], "Tab/↓ 次の項目  S-Tab/↑ 前の項目  Enter 確定  Esc 取り消す");
}

#[test]
fn adding_puts_the_task_where_the_form_says() {
    let f = Fixture::new();
    let mut app = f.app();
    // Next のタブで開くと Next へ入れる。期日が今日でも動かさない (gtodo add --to と同じ)。
    press(&mut app, "3a");
    press(&mut app, "資料を読む due:today");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    let todo = f.read("todo.md");
    let next = todo.find("## Next").unwrap();
    let waiting = todo.find("## Waiting").unwrap();
    assert!(
        todo[next..waiting].contains("資料を読む due:2026-11-16 created:2026-11-16 id:"),
        "{todo}"
    );
    assert_eq!(last_message(&app), "added to Next: 資料を読む");
    assert_eq!(selected_content(&app), "資料を読む");
}

#[test]
fn a_due_task_added_to_the_inbox_moves_to_today() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "1a");
    press(&mut app, "期限切れ due:2026-11-10");
    press_code(&mut app, KeyCode::Enter);
    assert!(!f.read("inbox.md").contains("期限切れ"));
    assert!(f.read("todo.md").contains("期限切れ"));
    let messages: Vec<String> = app.messages.iter().map(|m| m.text.clone()).collect();
    assert_eq!(
        messages,
        ["added to inbox.md: 期限切れ", "moved 1 due task(s) to Today"]
    );
}

#[test]
fn the_destination_and_wait_can_be_chosen_in_the_form() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "1a");
    press(&mut app, "返事を待つ");
    for _ in 0..5 {
        press_code(&mut app, KeyCode::Tab);
    }
    for _ in 0..3 {
        press_code(&mut app, KeyCode::Right);
    }
    press_code(&mut app, KeyCode::Tab);
    press(&mut app, "bob");
    press_code(&mut app, KeyCode::Enter);
    let todo = f.read("todo.md");
    let waiting = todo.find("## Waiting").unwrap();
    assert!(
        todo[waiting..].contains("返事を待つ created:2026-11-16 wait:bob id:"),
        "{todo}"
    );
    assert_eq!(last_message(&app), "added to Waiting: 返事を待つ");
}

#[test]
fn an_unreadable_value_keeps_the_form_open_and_esc_writes_nothing() {
    let f = Fixture::new();
    let mut app = f.app();
    let before = (f.read("inbox.md"), f.read("todo.md"));
    press(&mut app, "a");
    press(&mut app, "x due:2/30");
    press_code(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::Form(_)));
    let rows = screen(&mut app, 80, 20).join("\n");
    assert!(rows.contains("error: invalid due date"), "{rows}");
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!((f.read("inbox.md"), f.read("todo.md")), before);
}

#[test]
fn editing_changes_only_the_edited_fields() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "e");
    let Mode::Form(form) = &app.mode else {
        panic!("no form");
    };
    assert!(!form.is_add());
    // 期日を変え、プロジェクトを消す。
    press_code(&mut app, KeyCode::Tab);
    press_ctrl(&mut app, 'u');
    press(&mut app, "+3d");
    press_code(&mut app, KeyCode::Tab);
    press_ctrl(&mut app, 'u');
    press_code(&mut app, KeyCode::Enter);
    let todo = f.read("todo.md");
    assert!(
        todo.contains("- [ ] (A) 今日1 @office due:2026-11-19 created:2026-11-16 id:aaaaaa"),
        "{todo}"
    );
    assert_eq!(last_message(&app), "edited: 今日1");
    assert_eq!(selected_content(&app), "今日1");

    // 何も変えずに確定しても書かない。
    let before = f.read("todo.md");
    press(&mut app, "e");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(f.read("todo.md"), before);
    assert_eq!(last_message(&app), "nothing changed");
}

#[test]
fn tasks_in_done_md_cannot_be_edited() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "6e");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(last_message(&app), "tasks in done.md cannot be changed");
}

#[test]
fn project_files_and_tags_in_use_are_offered_as_candidates() {
    let f = Fixture::new();
    fs::create_dir(f.path().join("projects")).unwrap();
    f.write("projects/garden.md", "---\ntitle: 庭\n---\n");
    // アーカイブしたプロジェクトは候補にしない (Neovim 版と同じ)。
    f.write("projects/old.md", "---\ntitle: 昔\nstatus: archived\n---\n");
    let mut app = f.app();
    press(&mut app, "a");
    press(&mut app, "x");
    press_code(&mut app, KeyCode::Tab);
    press_code(&mut app, KeyCode::Tab);
    let rows = screen(&mut app, 80, 20).join("\n");
    assert!(rows.contains("候補  +garden  +work"), "{rows}");
    press_ctrl(&mut app, 'n');
    press_ctrl(&mut app, 'n');
    press_code(&mut app, KeyCode::Tab);
    press_ctrl(&mut app, 'n');
    let Mode::Form(form) = &app.mode else {
        panic!("no form");
    };
    let task = form.preview().unwrap();
    assert_eq!(
        (task.project.as_deref(), task.context.as_deref()),
        (Some("work"), Some("office"))
    );
    // タスクにタグを書いただけでは projects/*.md を作らない (Neovim 版と同じ)。
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(fs::read_dir(f.path().join("projects")).unwrap().count(), 2);
}

#[test]
fn pasted_text_goes_into_the_focused_field() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "a");
    app.handle_paste("貼り付けた\n本文");
    press_code(&mut app, KeyCode::Enter);
    assert!(
        f.read("todo.md")
            .contains("- [ ] 貼り付けた 本文 created:2026-11-16 id:")
    );
}

#[test]
fn a_long_line_to_be_written_is_wrapped_not_cut() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "a");
    press(
        &mut app,
        "請求書を送る https://example.com/invoice/2026/11/very/long/path +work @office due:fri",
    );
    let rows = screen(&mut app, 60, 24);
    let start = rows.iter().position(|r| r.contains("書き込む行")).unwrap() + 1;
    let preview: String = rows[start..]
        .iter()
        .take_while(|r| !r.contains('└'))
        .map(|r| r.trim_start_matches([' ', '│']).trim_end_matches([' ', '│']))
        .collect();
    assert_eq!(
        preview,
        "- [ ] 請求書を送る https://example.com/invoice/2026/11/very/long/path +work @office due:2026-11-20 created:2026-11-16"
    );
}

/// Queue のテスト用のデータ (今日は 2026-11-16)。
fn queue_fixture() -> Fixture {
    let f = Fixture::new();
    f.write(
        "inbox.md",
        "# Inbox\n\n- [ ] 受信 due:2026-11-16 id:111111\n- [ ] 期日なし id:222222\n",
    );
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 遅れ +work due:2026-11-13 id:aaaaaa\n- [ ] 今日の作業 due:2026-11-16 id:bbbbbb\n\n\
         ## Next\n\n- [ ] 来月 due:2026-12-20 id:cccccc\n\n## Waiting\n\n- [ ] 見積もり wait:bob due:2026-11-18 id:dddddd\n\n\
         ## Someday\n\n## 買い物\n\n- [ ] 牛乳 due:2026-11-17 id:eeeeee\n",
    );
    f
}

#[test]
fn q_opens_the_queue_with_the_neovim_headings() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Q");
    assert_eq!(
        screen(&mut app, 60, 22),
        [
            " Queue (Due)  2026-11-16",
            "────────────────────────────────────────────────────────────",
            " 期限切れ",
            "▌ ⚠ 遅れ +work (3日超過)                               Today",
            "",
            " 今日 (11/16 月)",
            "  ▶ 受信                                               Inbox",
            "  ▶ 今日の作業                                         Today",
            "",
            " 明日 (11/17 火)",
            "  ▶ 牛乳                                              買い物",
            "",
            " 11/18 水 (2日後)",
            "  ▶ 見積もり                                         Waiting",
            "",
            " 12/20 日 (34日後)",
            "  ▶ 来月                                                Next",
            "",
            // 選んでいるタスクの全文とタグ (詳細欄)。
            "────────────────────────────────────────────────────────────",
            "[ ] 遅れ",
            "+work  due:2026-11-13 (3日超過)",
            "x 完了にする  Enter 一覧で開く  Tab 待ち先別  ? ヘルプ",
        ]
    );
}

#[test]
fn the_queue_cursor_skips_headings_and_tab_switches_to_waits() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Q");
    let selected = |app: &App| app.selected().map(|s| s.row.task.content.clone()).unwrap_or_default();
    assert_eq!(selected(&app), "遅れ");
    press(&mut app, "j");
    assert_eq!(selected(&app), "受信");
    press(&mut app, "jj");
    assert_eq!(selected(&app), "牛乳");
    press(&mut app, "G");
    assert_eq!(selected(&app), "来月");
    press(&mut app, "gg");
    assert_eq!(selected(&app), "遅れ");

    press_code(&mut app, KeyCode::Tab);
    let rows = screen(&mut app, 60, 8);
    assert_eq!(rows[0], " Queue (Wait) 2026-11-16");
    assert_eq!(rows[2], " bob 待ち");
    assert_eq!(rows[3], "▌ ▶ 見積もり                                         Waiting");
    // 選んでいたタスクが切り替えた先にもあれば、選んだまま。
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(selected(&app), "見積もり");
    press(&mut app, "gg");
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(
        selected(&app),
        "見積もり",
        "遅れ has no wait:, so the first task is selected"
    );
}

#[test]
fn the_queue_footer_follows_the_selected_task() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Qj");
    // inbox のタスクは、どのセクションへも移せる。
    assert_eq!(
        footer(&mut app),
        "x 完了にする  d Today へ  n Next へ  w Waiting へ  s Someday へ  a 追加  Enter 一覧で開く  Tab 待ち先別  ? ヘルプ"
    );
}

#[test]
fn enter_opens_the_task_in_its_tab() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Qjjjjj");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::List);
    assert_eq!(app.current, 2);
    assert_eq!(selected_content(&app), "来月");

    // todo.md の他の見出しのタスクは、どのタブにも出ていない。
    press(&mut app, "Qjjj");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Queue);
    assert_eq!(app.messages.last().unwrap().level, Level::Warning);
}

#[test]
fn tasks_can_be_changed_from_the_queue() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Qj");
    // inbox のタスクを Today へ。Queue に残り、右端の場所が変わる。
    press(&mut app, "d");
    assert!(!f.read("inbox.md").contains("受信"));
    let item = app.queue.selected().unwrap();
    assert_eq!((item.row.task.content.as_str(), item.place.as_str()), ("受信", "Today"));
    // 完了にすると Queue から消え、次のタスクを選ぶ。
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("- [x] 受信"));
    assert!(app.queue.items().all(|(_, i)| i.row.task.content != "受信"));
    assert_eq!(app.screen, Screen::Queue);

    // Queue から追加すると inbox へ入れる。
    press(&mut app, "a");
    let Mode::Form(form) = &app.mode else {
        panic!("no form");
    };
    assert_eq!(form.destination(), gtodo_core::ops::Destination::Inbox);
}

#[test]
fn esc_goes_back_to_the_list_and_q_quits_from_the_queue() {
    let f = queue_fixture();
    let mut app = f.app();
    // 押しかけのキーがあれば、Esc はそれを取り消すだけ。
    press(&mut app, "Qg");
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Queue);
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::List);
    assert!(!app.should_quit());
    press(&mut app, "Q4");
    assert_eq!((app.screen, app.current), (Screen::List, 3));
    press(&mut app, "Qq");
    assert!(app.should_quit());
}

#[test]
fn the_queue_follows_changes_by_other_programs() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Q");
    f.write("inbox.md", "# Inbox\n\n- [ ] 新しく来た due:2026-11-15 id:333333\n");
    app.tick();
    let contents: Vec<&str> = app.queue.items().map(|(_, i)| i.row.task.content.as_str()).collect();
    // 期限切れは超過した日数の多い順 (Neovim 版と同じ)。
    assert_eq!(
        contents,
        ["遅れ", "新しく来た", "今日の作業", "牛乳", "見積もり", "来月"]
    );
}

#[test]
fn due_inbox_tasks_are_noted_when_they_are_not_moved_automatically() {
    let f = queue_fixture();
    f.write("gtodo.json", r#"{"auto_move_inbox_to_today": false}"#);
    let mut app = f.app();
    // Waiting の期日の近いタスクの知らせも一緒に出る。
    let note = "1 task(s) in inbox.md are due; they stay there because auto_move_inbox_to_today is off (Q to see them)";
    assert!(app.messages.iter().any(|m| m.text == note), "{:?}", app.messages);
    press(&mut app, "j");
    assert!(app.messages.is_empty());
    // 日付が変わったときにも出す (日次の整理が警告していれば重ねない)。
    f.today.set(date("2026-11-17"));
    app.tick();
    let texts: Vec<&str> = app.messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts.iter().filter(|t| t.contains("inbox.md")).count(), 1, "{texts:?}");
}

#[test]
fn no_note_when_due_inbox_tasks_move_automatically() {
    let f = queue_fixture();
    let app = f.app();
    assert!(app.messages.iter().all(|m| !m.text.contains("auto_move")));
}

#[test]
fn the_inbox_note_waits_for_the_rollover_and_is_not_repeated() {
    let f = queue_fixture();
    f.write("gtodo.json", r#"{"auto_move_inbox_to_today": false}"#);
    let mut app = f.app();
    press(&mut app, "j");
    // 日付が変わったとき、Neovim 版がロックを持っていれば日次の整理を待つ。
    f.write(".gtodo.lock", "12345 0");
    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(app.messages.iter().all(|m| !m.text.contains("inbox.md")));
    fs::remove_file(f.path().join(".gtodo.lock")).unwrap();
    app.tick();
    app.tick();
    let about_inbox = app.messages.iter().filter(|m| m.text.contains("inbox.md")).count();
    assert_eq!(about_inbox, 1, "{:?}", app.messages);
}

#[test]
fn the_inbox_note_is_given_once_even_if_an_operation_notices_the_new_day() {
    let f = queue_fixture();
    f.write("gtodo.json", r#"{"auto_move_inbox_to_today": false}"#);
    let mut app = f.app();
    press(&mut app, "j");
    f.today.set(date("2026-11-17"));
    // 次の確認より前に操作した。日次の整理はその準備で行われ、inbox.md のタスクを警告する。
    press(&mut app, "x");
    app.tick();
    app.tick();
    let about_inbox = app.messages.iter().filter(|m| m.text.contains("inbox.md")).count();
    assert_eq!(about_inbox, 1, "{:?}", app.messages);
}

#[test]
fn changes_made_while_the_queue_is_closed_show_when_it_opens() {
    let f = queue_fixture();
    let mut app = f.app();
    f.write("inbox.md", "# Inbox\n\n- [ ] 新しく来た due:2026-11-16 id:333333\n");
    app.tick();
    press(&mut app, "Q");
    let contents: Vec<&str> = app.queue.items().map(|(_, i)| i.row.task.content.as_str()).collect();
    assert_eq!(
        contents,
        ["遅れ", "新しく来た", "今日の作業", "牛乳", "見積もり", "来月"]
    );
}

#[test]
fn the_place_stays_visible_on_a_narrow_terminal() {
    let f = queue_fixture();
    let mut app = f.app();
    press(&mut app, "Q");
    let rows = screen(&mut app, 32, 8);
    assert_eq!(rows[3], "▌ ⚠ 遅れ +work (3日超過)   Today");
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n## Next\n\n## Waiting\n\n- [ ] とても長い本文のタスク +work @office due:2026-11-13 id:aaaaaa\n\n## Someday\n",
    );
    app.tick();
    let rows = screen(&mut app, 40, 8);
    assert!(rows[3].ends_with("Waiting"), "{}", rows[3]);
}

/// 検索のテスト用のデータ (今日は 2026-11-16)。
fn search_fixture() -> Fixture {
    let f = Fixture::new();
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信した資料 id:111111\n");
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 会議の資料を作る +work @office id:aaaaaa\n- [ ] 牛乳 id:bbbbbb\n\n\
         ## Next\n\n## Waiting\n\n- [ ] 見積もり wait:田中 id:cccccc\n\n## Someday\n",
    );
    f.write(
        "done.md",
        "# Done\n\n## 2026-11\n\n- [x] 先月の資料 completed_at:2026-11-02 id:f00001\n",
    );
    f
}

fn search_results(app: &App) -> Vec<String> {
    app.search
        .results
        .iter()
        .map(|r| app.search.entry(r).row.task.content.clone())
        .collect()
}

#[test]
fn slash_opens_the_search_and_typing_narrows_the_results() {
    let f = search_fixture();
    let mut app = f.app();
    press(&mut app, "/");
    assert_eq!(app.screen, Screen::Search);
    // 何も入れていなければ全部出す。
    assert_eq!(search_results(&app).len(), 5);
    press(&mut app, "資料");
    assert_eq!(
        screen(&mut app, 70, 8),
        [
            " / 資料                                   [すべて] 未完了  完了  3 件",
            "──────────────────────────────────────────────────────────────────────",
            "▌ [ ] 受信した資料                                               Inbox",
            "  [ ] 会議の資料を作る +work @office                             Today",
            "  [x] 先月の資料  2026-11-02                                      Done",
            "",
            "",
            "Enter 結果へ  ↑/↓ 選ぶ  Tab 状態  Esc 一覧へ",
        ]
    );
    // 当たった部分の位置 (強調して見せる)。
    let (entry, _) = app.search.selected().unwrap();
    let ranges = app.search.query.highlight(&entry.row.task.content);
    assert_eq!(&entry.row.task.content[ranges[0].clone()], "資料");
    // 入力欄で打った q は文字。
    press(&mut app, " q");
    assert!(!app.should_quit());
    assert_eq!(app.search.input.text(), "資料 q");
}

#[test]
fn tags_and_status_narrow_the_search() {
    let f = search_fixture();
    let mut app = f.app();
    press(&mut app, "/+wo");
    assert_eq!(search_results(&app), ["会議の資料を作る"]);
    press_ctrl(&mut app, 'u');
    press(&mut app, "wait:田");
    assert_eq!(search_results(&app), ["見積もり"]);
    press_ctrl(&mut app, 'u');
    press(&mut app, "資料");
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(search_results(&app), ["受信した資料", "会議の資料を作る"]);
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(search_results(&app), ["先月の資料"]);
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(search_results(&app).len(), 3);
}

#[test]
fn results_can_be_changed_and_opened_in_the_list() {
    let f = search_fixture();
    let mut app = f.app();
    press(&mut app, "/資料");
    // 入力欄にいるまま、矢印で選ぶ。
    press_code(&mut app, KeyCode::Down);
    press_code(&mut app, KeyCode::Enter);
    assert!(!app.search.typing);
    assert_eq!(
        footer(&mut app),
        "x 完了にする  n Next へ  w Waiting へ  s Someday へ  e 編集  a 追加  Enter 一覧で開く  Tab 状態  Esc 一覧へ  ? ヘルプ"
    );
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("- [x] 会議の資料を作る"));
    // 完了したタスクも「すべて」なので残り、選んだまま。
    assert_eq!(app.selected().unwrap().row.task.content, "会議の資料を作る");

    // done.md のタスクは見るだけ。
    press(&mut app, "j");
    press(&mut app, "x");
    assert_eq!(last_message(&app), "tasks in done.md cannot be changed");

    // Enter で一覧のそのタスクへ。
    press(&mut app, "k");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!((app.screen, app.current), (Screen::List, 1));
    assert_eq!(selected_content(&app), "会議の資料を作る");
}

#[test]
fn slash_returns_to_the_input_and_esc_to_the_list_keeping_the_query() {
    let f = search_fixture();
    let mut app = f.app();
    press(&mut app, "/資料");
    press_code(&mut app, KeyCode::Enter);
    press(&mut app, "/");
    assert!(app.search.typing);
    press(&mut app, "を");
    assert_eq!(search_results(&app), ["会議の資料を作る"]);
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::List);
    // 開き直すと前の検索語のまま (続きから絞り込める)。
    press(&mut app, "/");
    assert_eq!(app.search.input.text(), "資料を");
    // 当たるものが無ければ、Enter しても入力欄に留まる。
    press(&mut app, "ん");
    press_code(&mut app, KeyCode::Enter);
    assert!(app.search.typing);
    let rows = screen(&mut app, 60, 6);
    assert_eq!(rows[2], "  一致するタスクはありません");
}

#[test]
fn the_search_follows_changes_by_other_programs() {
    let f = search_fixture();
    let mut app = f.app();
    press(&mut app, "/資料");
    f.write(
        "inbox.md",
        "# Inbox\n\n- [ ] 新しい資料 id:222222\n- [ ] 受信した資料 id:111111\n",
    );
    app.tick();
    assert_eq!(
        search_results(&app),
        ["新しい資料", "受信した資料", "会議の資料を作る", "先月の資料"]
    );
    assert_eq!(app.selected().unwrap().row.task.content, "受信した資料");
}

fn column_contents(app: &App, column: usize) -> Vec<String> {
    app.kanban.columns[column]
        .iter()
        .map(|c| c.row.task.content.clone())
        .collect()
}

#[test]
fn k_opens_the_kanban_with_the_neovim_columns() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kll");
    assert_eq!(app.screen, Screen::Kanban);
    assert_eq!(
        screen(&mut app, 60, 12),
        [
            " Kanban   ◀ Someday (0)   Waiting (1) · Done (2) ▶",
            "────────────────────────────────────────────────────────────",
            "          Next (2)                      Today (2)",
            "┌───────────────────────────┐ ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━┓",
            "│次1                        │ ┃(A) 今日1                  ┃",
            "└───────────────────────────┘ ┃due:2026-11-16 (今日) +work┃",
            "┌───────────────────────────┐ ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━┛",
            "│次2                        │ ┌───────────────────────────┐",
            "└───────────────────────────┘ │今日2                      │",
            "                              └───────────────────────────┘",
            "",
            "x 完了にする  a 追加  Enter 一覧で開く  h/l 列  ? ヘルプ",
        ]
    );
    // 5 列。Done は todo.md の完了済みと done.md のものを新しい順に。
    assert_eq!(column_contents(&app, 0), Vec::<String>::new());
    assert_eq!(column_contents(&app, 3), ["待ち1"]);
    assert_eq!(column_contents(&app, kanban::DONE), ["新しい完了", "古い完了"]);
    // 端の列から先へは進まない。
    press(&mut app, "llll");
    assert_eq!(app.kanban.focus, kanban::DONE);
    press(&mut app, "hhhhhh");
    assert_eq!(app.kanban.focus, 0);
}

#[test]
fn completing_a_card_moves_it_to_done_and_keeps_the_cursor_in_the_column() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kll");
    // 確認なしで完了にする (一覧と同じ。押す前に最下段に結果が出ている)。
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("- [x] (A) 今日1"));
    assert_eq!(column_contents(&app, 2), ["今日2"]);
    assert_eq!(column_contents(&app, kanban::DONE)[0], "今日1");
    assert_eq!(app.kanban.focus, 2);
    assert_eq!(app.selected().unwrap().row.task.content, "今日2");

    // Done の列の、日次の整理を待っているカードは未完了に戻せる。done.md のカードは見るだけ。
    // 他の列で選んでいたカードは、上にカードが加わっても選んだまま。
    press(&mut app, "ll");
    assert_eq!(app.selected().unwrap().row.task.content, "新しい完了");
    press(&mut app, "gg");
    assert_eq!(app.selected().unwrap().row.task.content, "今日1");
    press(&mut app, "x");
    assert!(f.read("todo.md").contains("- [ ] (A) 今日1"));
    assert_eq!(column_contents(&app, 2), ["今日1", "今日2"]);
    assert_eq!(app.selected().unwrap().row.task.content, "新しい完了");
    press(&mut app, "x");
    assert_eq!(last_message(&app), "tasks in done.md cannot be changed");
    press(&mut app, "d");
    assert_eq!(last_message(&app), "tasks in done.md cannot be changed");
}

#[test]
fn cards_move_between_columns_and_the_cursor_stays() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kl");
    assert_eq!(app.selected().unwrap().row.task.content, "次1");
    press(&mut app, "d");
    assert_eq!(column_contents(&app, 1), ["次2"]);
    assert!(column_contents(&app, 2).contains(&"次1".to_string()));
    assert_eq!(app.selected().unwrap().row.task.content, "次2");
    // w は待ち先を尋ねる。
    press(&mut app, "w");
    press(&mut app, "carol");
    press_code(&mut app, KeyCode::Enter);
    assert!(f.read("todo.md").contains("- [ ] 次2 wait:carol id:dddddd"));
    assert_eq!(column_contents(&app, 1), Vec::<String>::new());
    assert!(app.selected().is_none());
    let rows = screen(&mut app, 110, 8);
    assert!(rows[3].contains("(no tasks)"), "{}", rows[3]);
}

#[test]
fn enter_opens_a_card_in_its_tab_and_esc_returns() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Klll");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!((app.screen, app.current), (Screen::List, 3));
    assert_eq!(selected_content(&app), "待ち1");
    // 開き直すと、前に選んでいた列のまま。
    press(&mut app, "K");
    assert_eq!(app.kanban.focus, 3);
    press(&mut app, "l");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!((app.screen, app.current), (Screen::List, 5));
    assert_eq!(selected_content(&app), "新しい完了");
    press(&mut app, "K");
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::List);
}

#[test]
fn adding_from_a_column_puts_the_task_in_its_section() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kl");
    press(&mut app, "a");
    press(&mut app, "カンバンから");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(last_message(&app), "added to Next: カンバンから");
    assert!(column_contents(&app, 1).contains(&"カンバンから".to_string()));
    // Done の列からは inbox へ。
    press(&mut app, "lll");
    press(&mut app, "a");
    let Mode::Form(form) = &app.mode else {
        panic!("no form");
    };
    assert_eq!(form.destination(), gtodo_core::ops::Destination::Inbox);
}

#[test]
fn the_kanban_follows_changes_by_other_programs() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kll");
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 新しい id:999999\n- [ ] (A) 今日1 id:aaaaaa\n\n## Next\n\n## Waiting\n\n## Someday\n",
    );
    app.tick();
    assert_eq!(column_contents(&app, 2), ["新しい", "今日1"]);
    assert_eq!(app.selected().unwrap().row.task.content, "今日1");
    assert_eq!(column_contents(&app, 1), Vec::<String>::new());
}

#[test]
fn a_long_column_scrolls_to_keep_the_selected_card_visible() {
    let f = Fixture::new();
    let cards: String = (1..=30).map(|i| format!("- [ ] 次{i} id:{i:06x}\n")).collect();
    f.write(
        "todo.md",
        &format!("# Todo\n\n## Today\n\n## Next\n\n{cards}\n## Waiting\n\n## Someday\n"),
    );
    let mut app = f.app();
    press(&mut app, "Kl");
    // 高さ 14: 1 行目・区切り・列の見出し・最下段を除いた 10 行に、3 行のカードが 3 枚入る。
    let rows = screen(&mut app, 40, 14).join("\n");
    assert!(
        rows.contains("次1") && rows.contains("次3") && !rows.contains("次4"),
        "{rows}"
    );
    press(&mut app, "G");
    let rows = screen(&mut app, 40, 14).join("\n");
    assert!(
        rows.contains("┃次30") && rows.contains("次28") && !rows.contains("次27"),
        "{rows}"
    );
    // 上へ戻ると、選んでいるカードが一番上になるところまでだけ戻す。
    press(&mut app, "kkk");
    let rows = screen(&mut app, 40, 14).join("\n");
    assert!(
        rows.contains("┃次27") && rows.contains("次29") && !rows.contains("次30"),
        "{rows}"
    );
    // 半ページ (見えている枚数の半分) ずつ動く。
    press_ctrl(&mut app, 'u');
    assert_eq!(app.selected().unwrap().row.task.content, "次26");
    press(&mut app, "gg");
    let rows = screen(&mut app, 40, 14).join("\n");
    assert!(rows.contains("┃次1 "), "{rows}");
}

#[test]
fn a_card_taller_than_the_column_is_drawn_as_far_as_it_fits() {
    let f = Fixture::new();
    let mut app = f.app();
    press(&mut app, "Kll");
    // 本文と期日の行がある「今日1」は 4 行。列には 2 行しか入らない。
    let rows = screen(&mut app, 40, 6);
    assert_eq!(rows[3], "┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓");
    assert_eq!(rows[4], "┃(A) 今日1                             ┃");
}

/// markdown のテスト用のデータ。
fn markdown_fixture() -> Fixture {
    let f = Fixture::new();
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] **請求書** を [経理](https://example.com/finance/2026/invoice) へ `送る` id:aaaaaa\n\
         - [ ] 参考 https://a.example/x と https://b.example/y/z id:bbbbbb\n- [ ] リンクなし id:cccccc\n\
         - [ ] ファイル [手順](docs/howto.md) id:dddddd\n\n## Next\n\n## Waiting\n\n## Someday\n",
    );
    f
}

/// 開いたリンクを記録する。
fn record_opened(app: &mut App) -> Rc<std::cell::RefCell<Vec<String>>> {
    let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = Rc::clone(&opened);
    app.set_opener(Box::new(move |url| {
        sink.borrow_mut().push(url.to_string());
        Ok(())
    }));
    opened
}

#[test]
fn rows_show_markdown_and_the_detail_shows_full_urls() {
    let f = markdown_fixture();
    let mut app = f.app();
    let rows = screen(&mut app, 80, 18);
    // 色を使わないときはコードの記号を残す。リンクは文字だけ、URL は短くする。
    assert_eq!(rows[2], "▌ [ ] 請求書 を 経理 へ `送る`");
    assert_eq!(rows[3], "  [ ] 参考 a.example/x と b.example/…/z");
    // 詳細欄には全文 (行き先も短くせずに)。
    assert!(
        rows.iter()
            .any(|r| r == "[ ] 請求書 を 経理 (https://example.com/finance/2026/invoice) へ `送る`"),
        "{rows:#?}"
    );
    // 低い端末では詳細欄を出さない。
    let rows = screen(&mut app, 80, 12);
    assert!(!rows.iter().any(|r| r.contains("https://")), "{rows:#?}");
}

#[test]
fn gx_opens_the_link_or_asks_which_one() {
    let f = markdown_fixture();
    let mut app = f.app();
    let opened = record_opened(&mut app);
    // 1 つならすぐ開く。
    press(&mut app, "gx");
    assert_eq!(*opened.borrow(), ["https://example.com/finance/2026/invoice"]);
    assert_eq!(last_message(&app), "opened https://example.com/finance/2026/invoice");

    // 複数なら選ぶ。番号でも、選んで Enter でも開ける。
    press(&mut app, "j");
    press(&mut app, "gx");
    assert!(matches!(app.mode, Mode::ChooseLink { .. }));
    let rows = screen(&mut app, 60, 12).join("\n");
    assert!(
        rows.contains("▌1 https://a.example/x") && rows.contains(" 2 https://b.example/y/z"),
        "{rows}"
    );
    press(&mut app, "2");
    assert_eq!(opened.borrow().last().unwrap(), "https://b.example/y/z");
    press(&mut app, "gx");
    press_code(&mut app, KeyCode::Down);
    press_code(&mut app, KeyCode::Up);
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(opened.borrow().last().unwrap(), "https://a.example/x");
    // それ以外のキーで取り消す。
    press(&mut app, "gx");
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(opened.borrow().len(), 3);

    // リンクが無い・http(s) でないリンクは開かない。
    press(&mut app, "j");
    press(&mut app, "gx");
    assert_eq!(last_message(&app), "the task has no links");
    press(&mut app, "j");
    press(&mut app, "gx");
    assert_eq!(
        last_message(&app),
        "only http(s) and mailto links are opened: docs/howto.md"
    );
    assert_eq!(opened.borrow().len(), 3);
}

#[test]
fn the_footer_offers_gx_only_for_tasks_with_links() {
    let f = markdown_fixture();
    let mut app = f.app();
    assert!(screen(&mut app, 200, 12)[11].contains("gx リンクを開く"));
    press(&mut app, "jj");
    assert!(!screen(&mut app, 200, 12)[11].contains("gx"));
}

#[test]
fn failing_to_open_a_link_is_reported() {
    let f = markdown_fixture();
    let mut app = f.app();
    app.set_opener(Box::new(|_| Err(std::io::Error::other("no browser"))));
    press(&mut app, "gx");
    assert_eq!(app.messages.last().unwrap().level, Level::Error);
    assert!(last_message(&app).contains("no browser"));
}

#[test]
fn cards_show_markdown_too() {
    let f = markdown_fixture();
    let mut app = f.app();
    press(&mut app, "Kll");
    let rows = screen(&mut app, 60, 12).join("\n");
    assert!(rows.contains("┃請求書 を 経理 へ `送る`"), "{rows}");
    assert!(rows.contains("│参考 a.example/x と"), "{rows}");
}

#[test]
fn gx_works_and_is_offered_for_read_only_tasks_too() {
    let f = Fixture::new();
    f.write(
        "done.md",
        "# Done\n\n## 2026-11\n\n- [x] 記録 https://example.com/report completed_at:2026-11-02 id:f00001\n",
    );
    let mut app = f.app();
    let opened = record_opened(&mut app);
    press(&mut app, "6");
    assert!(screen(&mut app, 120, 12)[11].contains("gx リンクを開く"));
    press(&mut app, "gx");
    assert_eq!(*opened.borrow(), ["https://example.com/report"]);
}

#[test]
fn keys_from_tui_toml_are_used_and_shown() {
    let f = Fixture::new();
    let keymap = Keymap::from_toml("[keys]\ntoggle_done = \"<Space>\"\ndown = [\"<C-n>\"]\ncancel = []\n")
        .unwrap()
        .keymap;
    let today = Rc::clone(&f.today);
    let mut app = App::new(f.path(), Box::new(move || today.get()), keymap, Vec::new()).unwrap();
    // 案内とヘルプは、設定したキーから作る。外した動作は出さない。
    let footer = screen(&mut app, 120, 12).pop().unwrap();
    assert!(footer.starts_with("Space 完了にする"), "{footer}");
    assert!(!footer.contains("キャンセル"), "{footer}");
    press(&mut app, "?");
    let help = screen(&mut app, 100, 40).join("\n");
    assert!(help.contains("Ctrl-n") && help.contains("Space"), "{help}");
    press_code(&mut app, KeyCode::Esc);
    // 既定の j は効かず、Ctrl-n で下へ。
    press(&mut app, "j");
    assert_eq!(selected_content(&app), "今日1");
    press_ctrl(&mut app, 'n');
    assert_eq!(selected_content(&app), "今日2");
    press(&mut app, " ");
    assert!(f.read("todo.md").contains("- [x] 今日2"));
    press(&mut app, "c");
    assert_eq!(app.mode, Mode::Normal, "c is unbound");
}

#[test]
fn keys_from_tui_toml_also_move_in_the_help_and_the_link_choice() {
    let f = markdown_fixture();
    let keymap = Keymap::from_toml("[keys]\ndown = \"<C-n>\"\nup = \"<C-p>\"\nhalf_page_down = \"zj\"\n")
        .unwrap()
        .keymap;
    let today = Rc::clone(&f.today);
    let mut app = App::new(f.path(), Box::new(move || today.get()), keymap, Vec::new()).unwrap();
    // ヘルプは続けて押すキーでもスクロールし、途中のキーで閉じない。
    press(&mut app, "?");
    screen(&mut app, 80, 12);
    press(&mut app, "z");
    assert_eq!(app.mode, Mode::Help);
    press(&mut app, "j");
    assert_eq!(app.mode, Mode::Help);
    assert!(app.help_scroll > 0);
    press(&mut app, "q");
    assert_eq!(app.mode, Mode::Normal);
    // リンクを選ぶときも、設定した下へ・上へのキーで選ぶ。
    let opened = record_opened(&mut app);
    press_ctrl(&mut app, 'n');
    press(&mut app, "gx");
    assert!(matches!(app.mode, Mode::ChooseLink { .. }));
    press_ctrl(&mut app, 'n');
    press_ctrl(&mut app, 'p');
    press_ctrl(&mut app, 'n');
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(opened.borrow().last().unwrap(), "https://b.example/y/z");
    // 既定の j は割り当てていないので、取り消しになる。
    press(&mut app, "gx");
    press(&mut app, "j");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(opened.borrow().len(), 1);
}

/// エディタで開かれたもの (コピーのパス・行・開いたときの内容)。
type Opened = Rc<RefCell<Vec<(PathBuf, Option<usize>, String)>>>;

/// エディタの代わりに、開かれたものを記録し、`edit(何回目か, 内容)` でコピーを書き換える。
fn fake_editor(app: &mut App, edit: impl Fn(usize, &str) -> String + 'static) -> Opened {
    let opened: Opened = Rc::default();
    let sink = Rc::clone(&opened);
    app.set_editor(Box::new(move |path, line| {
        let text = fs::read_to_string(path).unwrap();
        let n = sink.borrow().len();
        sink.borrow_mut().push((path.to_path_buf(), line, text.clone()));
        fs::write(path, edit(n, &text)).unwrap();
        Ok(true)
    }));
    opened
}

/// `E` を押してエディタを開く (端末を明け渡すのは mod.rs なので、ここでは直接呼ぶ)。
fn edit_file(app: &mut App) {
    press(app, "E");
    assert!(app.editor_requested());
    app.run_editor();
}

#[test]
fn edit_file_opens_a_copy_at_the_selected_task_and_saves_it() {
    let f = Fixture::new();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| text.replace("今日2", "今日2 を直した"));
    press(&mut app, "j");
    edit_file(&mut app);
    let (copy, line, text) = opened.borrow()[0].clone();
    // 本物ではなく、同じ名前のコピーを開く。カーソルは選んでいたタスクの行。
    assert_eq!(copy.file_name().unwrap(), "todo.md");
    assert_ne!(copy.parent().unwrap(), f.dir.path());
    assert_eq!(text.lines().nth(5), Some("- [ ] 今日2 id:bbbbbb"));
    assert_eq!(line, Some(6));
    assert!(!copy.exists(), "the copy is removed once saved");
    assert!(f.read("todo.md").contains("- [ ] 今日2 を直した id:bbbbbb"));
    assert_eq!(last_message(&app), "saved todo.md");
    assert!(app.view().rows.iter().any(|r| r.task.content == "今日2 を直した"));
}

#[test]
fn edit_file_opens_the_file_of_the_selected_task_or_the_tab() {
    let f = Fixture::new();
    fs::remove_file(f.dir.path().join("cancelled.md")).unwrap();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| text.to_string());
    let last = |opened: &Opened| {
        let (path, line, text) = opened.borrow().last().unwrap().clone();
        (path.file_name().unwrap().to_string_lossy().into_owned(), line, text)
    };
    // 見るだけのタブでも、ファイルは手で直せる。
    press(&mut app, "6");
    let selected = selected_content(&app);
    edit_file(&mut app);
    let (name, line, text) = last(&opened);
    assert_eq!(name, "done.md");
    assert!(text.lines().nth(line.unwrap() - 1).unwrap().contains(&selected));
    // タスクが無ければ行は指定しない。無いファイルは雛形で作ってから開く。
    press(&mut app, "7");
    edit_file(&mut app);
    let (name, line, text) = last(&opened);
    assert_eq!((name.as_str(), line), ("cancelled.md", None));
    assert!(text.starts_with("# Cancelled\n"), "{text:?}");
    assert_eq!(f.read("cancelled.md"), text);
    // Queue では選んでいるタスクのファイル。
    press(&mut app, "Q");
    edit_file(&mut app);
    let (name, line, _) = last(&opened);
    assert_eq!((name.as_str(), line), ("todo.md", Some(5)));
    assert_eq!(last_message(&app), "todo.md was not changed");
}

#[test]
fn edit_file_writes_nothing_when_the_editor_fails() {
    let f = Fixture::new();
    let before = f.read("todo.md");
    let mut app = f.app();
    // `:cq` などで終わったときは、保存していても書かない (git のコミットメッセージと同じ)。
    app.set_editor(Box::new(|path, _| {
        fs::write(path, "broken").unwrap();
        Ok(false)
    }));
    edit_file(&mut app);
    assert!(last_message(&app).ends_with("exited with an error; todo.md was not changed"));
    app.set_editor(Box::new(|_, _| {
        Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
    }));
    edit_file(&mut app);
    assert!(last_message(&app).starts_with("failed to start the editor ("));
    assert_eq!(f.read("todo.md"), before);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn an_edit_that_fails_the_neovim_checks_can_be_edited_again() {
    let f = Fixture::new();
    let before = f.read("todo.md");
    let mut app = f.app();
    // 1 回目は Waiting の見出しを消してしまい、2 回目で戻す。
    let opened = fake_editor(&mut app, |n, text| match n {
        0 => text.replace("## Waiting\n", "").replace("次1", "次1 直した"),
        _ => text.replace("## Someday", "## Waiting\n\n## Someday"),
    });
    edit_file(&mut app);
    assert_eq!(f.read("todo.md"), before, "nothing is written");
    assert_eq!(
        app.mode,
        Mode::FixEdit {
            name: "todo.md".into(),
            problems: vec!["required sections are missing: ## Waiting".into()]
        }
    );
    let rows = screen(&mut app, 80, 12);
    assert_eq!(
        rows[10..],
        [
            "todo.md: required sections are missing: ## Waiting",
            "Edit again? [y/n] (n discards your changes)"
        ]
    );
    // y か n (Esc) で答えるまでは他のキーを受け付けない。
    press(&mut app, "xq");
    assert!(matches!(app.mode, Mode::FixEdit { .. }) && !app.should_quit());
    assert!(!app.editor_requested());
    press(&mut app, "y");
    assert!(app.editor_requested());
    app.run_editor();
    // 前に編集したコピーの続きを開く。
    let opened = opened.borrow();
    assert_eq!(opened[0].0, opened[1].0);
    assert!(opened[1].2.contains("次1 直した"));
    let todo = f.read("todo.md");
    assert!(todo.contains("次1 直した") && todo.contains("## Waiting"), "{todo}");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(last_message(&app), "saved todo.md");
}

#[test]
fn an_edit_that_fails_the_neovim_checks_can_be_discarded() {
    let f = Fixture::new();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| text.replace("# Inbox", "Inbox"));
    press(&mut app, "1");
    edit_file(&mut app);
    assert!(matches!(&app.mode, Mode::FixEdit { name, .. } if name == "inbox.md"));
    let copy = opened.borrow()[0].0.clone();
    assert!(copy.exists());
    press_code(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert!(!copy.exists(), "the copy is removed");
    assert_eq!(f.read("inbox.md"), "# Inbox\n\n- [ ] 受信1 id:111111\n");
    assert_eq!(last_message(&app), "discarded the changes to inbox.md");
}

#[test]
fn an_edit_is_not_saved_over_changes_made_while_editing() {
    let f = Fixture::new();
    let mut app = f.app();
    let real = f.dir.path().join("todo.md");
    app.set_editor(Box::new(move |path, _| {
        // 編集している間に Neovim 版が書いた。
        let theirs = fs::read_to_string(&real).unwrap().replace("次2", "次2 Neovim");
        fs::write(&real, theirs).unwrap();
        let mine = fs::read_to_string(path).unwrap().replace("次1", "次1 TUI");
        fs::write(path, mine).unwrap();
        Ok(true)
    }));
    edit_file(&mut app);
    let todo = f.read("todo.md");
    assert!(todo.contains("次2 Neovim") && !todo.contains("次1 TUI"), "{todo}");
    // 編集した内容は失わず、場所を知らせる。
    let message = last_message(&app);
    // 80 桁の端末でも切れないように短くしてある。
    let first = format!("error: {}", message.lines().next().unwrap());
    assert_eq!(
        first,
        "error: todo.md was changed while you were editing and was not saved. Your copy:"
    );
    assert!(first.width() <= 80);
    // 場所は切り詰められないよう、別の行に出す。
    let kept = Path::new(message.lines().last().unwrap());
    assert!(fs::read_to_string(kept).unwrap().contains("次1 TUI"));
    // 終わった後にも知らせるため、覚えておく。
    assert_eq!(app.finish(), [kept.to_path_buf()]);
    fs::remove_dir_all(kept.parent().unwrap()).unwrap();
}

#[test]
fn quitting_while_asked_to_fix_an_edit_keeps_the_copy() {
    let f = Fixture::new();
    let mut app = f.app();
    fake_editor(&mut app, |_, text| text.replace("## Next", "## Nxt"));
    edit_file(&mut app);
    assert!(matches!(app.mode, Mode::FixEdit { .. }));
    // Ctrl-c はどこでも終了する。編集した内容は消さずに残す。
    press_ctrl(&mut app, 'c');
    assert!(app.should_quit());
    let kept = app.finish();
    assert_eq!(kept.len(), 1);
    assert!(fs::read_to_string(&kept[0]).unwrap().contains("## Nxt"));
    fs::remove_dir_all(kept[0].parent().unwrap()).unwrap();
}

#[test]
fn edit_file_on_the_kanban_and_in_search_opens_the_file_of_the_column_or_task() {
    let f = Fixture::new();
    fs::write(f.dir.path().join("done.md"), "# Done\n").unwrap();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| text.to_string());
    let last = |opened: &Opened| {
        let (path, line, _) = opened.borrow().last().unwrap().clone();
        (path.file_name().unwrap().to_string_lossy().into_owned(), line)
    };
    // 空の Done の列では done.md。
    press(&mut app, "K");
    for _ in 0..kanban::COLUMNS {
        press(&mut app, "l");
    }
    assert!(app.selected().is_none());
    edit_file(&mut app);
    assert_eq!(last(&opened), ("done.md".into(), None));
    // 検索では選んでいるタスクのファイル。
    press_code(&mut app, KeyCode::Esc);
    press(&mut app, "/受信");
    press_code(&mut app, KeyCode::Enter);
    edit_file(&mut app);
    assert_eq!(last(&opened), ("inbox.md".into(), Some(3)));
}

#[test]
fn waiting_tasks_due_soon_are_noticed_once_per_task_and_day() {
    let f = Fixture::new();
    let todo = "# Todo\n\n## Today\n\n## Next\n\n## Waiting\n\n- [ ] 見積もり wait:bob due:2026-11-17 id:aaaaaa\n\
                - [ ] 来週 wait:bob due:2026-11-23 id:bbbbbb\n\n## Someday\n";
    f.write("todo.md", todo);
    let mut app = f.app();
    assert_eq!(
        last_message(&app),
        "Waiting tasks due soon:\n- 見積もり (due: 2026-11-17)"
    );
    let rows = screen(&mut app, 80, 14);
    assert!(
        rows.iter().any(|r| r == "warning: Waiting tasks due soon:"),
        "{rows:#?}"
    );
    // 同じタスクは知らせ直さない (Neovim 版が ID を付けても)。
    press_code(&mut app, KeyCode::Esc);
    app.tick();
    f.write("todo.md", &todo.replace("id:aaaaaa", "id:ffffff"));
    app.tick();
    assert!(app.messages.is_empty());
    // 新しく該当したタスクがあれば、該当するものをすべて出す。
    f.write(
        "todo.md",
        &todo.replace(
            "## Someday",
            "- [ ] 返事 wait:alice due:2026-11-15 id:cccccc\n\n## Someday",
        ),
    );
    app.tick();
    assert_eq!(
        last_message(&app),
        "Waiting tasks due soon:\n- 見積もり (due: 2026-11-17)\n- 返事 (due: 2026-11-15)"
    );
    // 日付が変わったら知らせ直す (2 日後までに入った「来週」は、まだ入らない)。
    press_code(&mut app, KeyCode::Esc);
    f.today.set(date("2026-11-17"));
    app.tick();
    assert!(
        app.messages
            .iter()
            .any(|m| m.level == Level::Warning && m.text.starts_with("Waiting tasks due soon:\n- 見積もり")),
        "{:?}",
        app.messages
    );
}

/// プロジェクトの画面を試すためのデータ。site はファイルがあり、work はタスクにだけ使われている。
fn project_fixture() -> Fixture {
    let f = Fixture::new();
    fs::create_dir(f.path().join("projects")).unwrap();
    f.write(
        "projects/site.md",
        "---\ntitle: サイト改修\ntag: site\ncreated: 2026-11-01\ndue: 2026-12-01\nstatus: active\nmembers: []\n---\n\n## Overview\n",
    );
    f.write("inbox.md", "# Inbox\n\n- [ ] 原稿 +site id:111111\n");
    f.write(
        "done.md",
        "# Done\n\n## 2026-11\n\n- [x] 要件 +site completed_at:2026-11-02 id:f00001\n",
    );
    f
}

#[test]
fn the_projects_screen_shows_progress_and_tags_without_files() {
    let f = project_fixture();
    let mut app = f.app();
    // 今日1 (+work) を選んで開くと、そのプロジェクトを選ぶ。
    press(&mut app, "P");
    assert_eq!(app.screen, Screen::Projects);
    let rows = screen(&mut app, 90, 12);
    assert_eq!(rows[0], " Projects 2");
    assert_eq!(
        rows[2..4],
        [
            "  [█████░░░░░] 50% (1/2 done)  site  サイト改修  due:2026-12-01",
            "▌ [░░░░░░░░░░] 0% (0/1 done)  work  (ファイルなし)"
        ]
    );
    let footer = screen(&mut app, 120, 12).pop().unwrap();
    assert!(
        footer.starts_with("Enter タスクを検索  E ファイルを作る  a 追加  Tab アーカイブ済み  Esc 一覧へ"),
        "{footer}"
    );
    // Enter でそのプロジェクトのタスクを検索する。
    press(&mut app, "k");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Search);
    assert_eq!(app.search.input.text(), "+site ");
    assert!(!app.search.typing);
    assert_eq!(app.search.results.len(), 2, "inbox.md and done.md tasks with +site");
}

#[test]
fn projects_are_archived_and_restored_from_the_projects_screen() {
    let f = project_fixture();
    let mut app = f.app();
    press(&mut app, "1P");
    assert_eq!(app.projects.selected().unwrap().tag, "site");
    press(&mut app, "X");
    assert_eq!(last_message(&app), "archived site");
    assert!(f.read("projects/site.md").contains("\nstatus: archived\n"));
    assert!(app.projects.rows.iter().all(|r| r.tag != "site"));
    // Tab でアーカイブしたものを見て、X で戻す。
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(screen(&mut app, 80, 10)[0], " Projects (archived) 1");
    press(&mut app, "X");
    assert_eq!(last_message(&app), "restored site");
    assert!(f.read("projects/site.md").contains("\nstatus: active\n"));
    // ファイルの無いプロジェクトはアーカイブできない。
    press_code(&mut app, KeyCode::Tab);
    press(&mut app, "G");
    press(&mut app, "X");
    assert!(last_message(&app).contains("projects/work.md does not exist"));
}

#[test]
fn new_projects_are_created_and_opened_in_the_editor() {
    let f = project_fixture();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| text.replace("title:\n", "title: 新規\n"));
    press(&mut app, "Pa");
    assert!(matches!(app.mode, Mode::NewName { .. }));
    // すでにある名前なら、作らずにそれを選ぶ。
    press(&mut app, "site");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(last_message(&app), "projects/site.md already exists");
    assert_eq!(app.projects.selected().unwrap().tag, "site");
    assert!(!app.editor_requested());
    press(&mut app, "a");
    // ファイル名にできない名前は、直した名前を添えて断る (入力は残す)。
    press(&mut app, "a/b");
    press_code(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::NewName { .. }));
    assert!(
        last_message(&app).contains(r#"use "a-b" instead"#),
        "{}",
        last_message(&app)
    );
    for _ in 0..3 {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "launch");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.editor_requested());
    app.run_editor();
    let (copy, _, text) = opened.borrow()[0].clone();
    assert_eq!(copy.file_name().unwrap(), "launch.md");
    assert!(
        text.starts_with("---\ntitle:\ntag: launch\ncreated: 2026-11-16\n"),
        "{text}"
    );
    assert!(f.read("projects/launch.md").contains("title: 新規\n"));
    app.tick();
    assert_eq!(app.projects.selected().unwrap().tag, "launch");
    assert_eq!(app.projects.selected().unwrap().title.as_deref(), Some("新規"));
}

#[test]
fn project_files_are_checked_like_neovim_when_edited_by_hand() {
    let f = project_fixture();
    let mut app = f.app();
    // ファイルの無い work は、E で作ってから開く。
    let opened = fake_editor(&mut app, |n, text| match n {
        0 => text.replace("title:", "title: 仕事"),
        _ => text.replace("tag: site", "tag: other"),
    });
    press(&mut app, "PG");
    press(&mut app, "E");
    app.run_editor();
    assert!(f.read("projects/work.md").contains("title: 仕事\ntag: work\n"));
    assert_eq!(opened.borrow().len(), 1);
    // tag を変えると、Neovim 版と同じく保存しない。
    press(&mut app, "gg");
    assert_eq!(app.projects.selected().unwrap().tag, "site");
    press(&mut app, "E");
    app.run_editor();
    assert_eq!(
        app.mode,
        Mode::FixEdit {
            name: "projects/site.md".into(),
            problems: vec!["tag must be the file name (site)".into()]
        }
    );
    assert!(f.read("projects/site.md").contains("tag: site\n"));
    press(&mut app, "n");
    assert_eq!(last_message(&app), "discarded the changes to projects/site.md");
}

/// テンプレートの画面を試すためのデータ。weekly はプレースホルダーを使い、plain は使わない。
fn template_fixture() -> Fixture {
    let f = Fixture::new();
    fs::create_dir_all(f.path().join("templates/archive")).unwrap();
    f.write(
        "templates/weekly.md",
        "<!-- メモ {{project}} -->\n- [ ] 振り返り {{project}} due:+2d\n- [ ] {{who}} に連絡 {{context}}\n",
    );
    f.write("templates/plain.md", "- [ ] 掃除 due:+1d\n");
    f.write("templates/archive/old.md", "- [ ] 古い作業\n");
    f
}

#[test]
fn templates_are_inserted_after_asking_placeholders_and_the_base_date() {
    let f = template_fixture();
    let mut app = f.app();
    press(&mut app, "T");
    assert_eq!(app.screen, Screen::Templates);
    let rows = screen(&mut app, 90, 12);
    assert_eq!(rows[0], " Templates 2");
    let weekly = rows
        .iter()
        .find(|r| r.contains("weekly"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(
        weekly.ends_with("weekly  2 task(s)  {{project}}  {{who}}  {{context}}"),
        "{weekly}"
    );
    let i = app.templates.rows.iter().position(|r| r.name == "weekly").unwrap();
    app.templates.cursor = i;

    press_code(&mut app, KeyCode::Enter);
    assert_eq!(
        screen(&mut app, 90, 12).pop().unwrap(),
        "Value for {{project}} (blank to omit):"
    );
    // タグに空白を含む値は断る (入力は残す)。
    press(&mut app, "my site");
    press_code(&mut app, KeyCode::Enter);
    assert!(last_message(&app).contains("cannot be a +project or @context tag"));
    for _ in 0.."my site".chars().count() {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "+site");
    press_code(&mut app, KeyCode::Enter);
    press(&mut app, "田中");
    press_code(&mut app, KeyCode::Enter);
    press_code(&mut app, KeyCode::Enter); // context は空
    // 基準日の初期値は today。
    assert_eq!(
        screen(&mut app, 90, 12).pop().unwrap(),
        "Base Date (today, +3d, 2026-08-20, ...): today"
    );
    for _ in 0..5 {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "2026-12-01");
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(last_message(&app), "added 2 task(s) from weekly to inbox.md");
    let inbox = f.read("inbox.md");
    assert!(inbox.contains("- [ ] 振り返り +site due:2026-12-03 id:"), "{inbox}");
    assert!(inbox.contains("- [ ] 田中 に連絡 id:"), "{inbox}");
}

/// 定期タスクは一覧に指定と次に足す日を出し、日付が変わった整理で inbox.md へ足す。手で挿入するときは
/// values: の値を初めから入れて尋ねる。
#[test]
fn recurring_templates_show_their_days_are_added_on_a_new_day_and_prefill_values() {
    let f = Fixture::new();
    fs::create_dir_all(f.path().join("templates")).unwrap();
    f.write(
        "templates/standup.md",
        "---\ntype: recurring\nrepeat: mon, thu\nvalues:\n  who: 田中\n---\n- [ ] {{who}} と朝会\n",
    );
    f.write("templates/broken.md", "---\ntype: recurring\n---\n- [ ] x\n");
    let mut app = f.app();
    press(&mut app, "T");
    // 今日 (2026-11-16 月) の整理は済んでいるので、次は木曜。
    let rows = screen(&mut app, 100, 12);
    let row = |rows: &[String], name: &str| rows.iter().find(|r| r.contains(name)).cloned().unwrap();
    assert!(
        row(&rows, "standup").ends_with("standup  1 task(s)  {{who}}  repeat: mon, thu (next 2026-11-19)"),
        "{rows:#?}"
    );
    assert!(
        row(&rows, "broken").ends_with("broken   1 task(s)  recurring: a recurring template needs 'repeat:'"),
        "{rows:#?}"
    );

    f.today.set(date("2026-11-19"));
    app.tick();
    assert!(
        f.read("inbox.md").contains("- [ ] 田中 と朝会 id:"),
        "{}",
        f.read("inbox.md")
    );
    assert!(
        app.messages
            .iter()
            .any(|m| m.text == "added 1 recurring task(s) to inbox.md: standup (2026-11-19)"),
        "{:?}",
        app.messages
    );
    assert!(
        f.read(".state.json")
            .contains(r#""recurring":{"standup":"2026-11-19"}"#)
    );
    press(&mut app, "T");
    let rows = screen(&mut app, 100, 12);
    assert!(row(&rows, "standup").ends_with("(next 2026-11-23)"), "{rows:#?}");

    let i = app.templates.rows.iter().position(|r| r.name == "standup").unwrap();
    app.templates.cursor = i;
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(
        screen(&mut app, 100, 12).pop().unwrap(),
        "Value for {{who}} (blank to omit): 田中"
    );
    press_code(&mut app, KeyCode::Enter);
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(f.read("inbox.md").matches("- [ ] 田中 と朝会 id:").count(), 2);
}

#[test]
fn due_template_tasks_move_to_today_like_add() {
    let f = template_fixture();
    let mut app = f.app();
    press(&mut app, "T");
    let i = app.templates.rows.iter().position(|r| r.name == "plain").unwrap();
    app.templates.cursor = i;
    // プレースホルダーが無ければ、すぐ基準日を尋ねる。前日を基準にすると due:+1d は今日になる。
    press_code(&mut app, KeyCode::Enter);
    for _ in 0..5 {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "2026-11-15");
    press_code(&mut app, KeyCode::Enter);
    assert!(
        f.read("todo.md").contains("- [ ] 掃除 due:2026-11-16 id:"),
        "{}",
        f.read("todo.md")
    );
    assert!(!f.read("inbox.md").contains("掃除"));
}

#[test]
fn templates_are_created_edited_archived_and_restored() {
    let f = template_fixture();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| format!("{text}- [ ] 新しい作業\n"));
    press(&mut app, "Ta");
    press(&mut app, "a b");
    press_code(&mut app, KeyCode::Enter);
    assert!(last_message(&app).contains("invalid template name"));
    for _ in 0..3 {
        press_code(&mut app, KeyCode::Backspace);
    }
    // アーカイブした名前では作らない (入力は残す)。
    press(&mut app, "old");
    press_code(&mut app, KeyCode::Enter);
    assert!(last_message(&app).contains("is archived"));
    assert!(matches!(app.mode, Mode::NewName { .. }));
    for _ in 0..3 {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "daily");
    press_code(&mut app, KeyCode::Enter);
    assert!(app.editor_requested());
    app.run_editor();
    let (copy, _, text) = opened.borrow()[0].clone();
    assert_eq!(copy.file_name().unwrap(), "daily.md");
    assert!(text.starts_with("<!--\n記法メモ:"), "{text}");
    assert!(f.read("templates/daily.md").ends_with("- [ ] 新しい作業\n"));
    app.tick();
    assert_eq!(app.templates.selected().unwrap().name, "daily");
    assert_eq!(app.templates.selected().unwrap().tasks, 1);

    // タスク行の無いテンプレートは、尋ねる前に知らせる。
    f.write("templates/empty.md", &template_seed());
    app.tick();
    app.templates.cursor = app.templates.rows.iter().position(|r| r.name == "empty").unwrap();
    press_code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert!(last_message(&app).contains("has no task lines"));
    app.templates.cursor = app.templates.rows.iter().position(|r| r.name == "daily").unwrap();
    // X でアーカイブし、Tab でアーカイブしたものを見て、X で戻す。
    press(&mut app, "X");
    assert_eq!(last_message(&app), "archived daily");
    assert!(f.path().join("templates/archive/daily.md").exists());
    press_code(&mut app, KeyCode::Tab);
    assert_eq!(screen(&mut app, 80, 10)[0], " Templates (archived) 2");
    // アーカイブしたものは挿入も編集もしない。
    press(&mut app, "gg");
    press_code(&mut app, KeyCode::Enter);
    assert!(last_message(&app).contains("is archived; restore it"));
    assert_eq!(app.mode, Mode::Normal);
    app.templates.cursor = app.templates.rows.iter().position(|r| r.name == "daily").unwrap();
    press(&mut app, "X");
    assert_eq!(last_message(&app), "restored daily");
    assert!(f.path().join("templates/daily.md").exists());
}

fn template_seed() -> String {
    gtodo_core::template::seed()
}

#[test]
fn p_splits_a_task_into_subtasks_written_in_the_editor() {
    let f = Fixture::new();
    let mut app = f.app();
    let opened = fake_editor(&mut app, |_, text| {
        text.replace("- [ ] \n", "- [ ] ワイヤー\n- [ ] 実装 @pc due:2026-11-20\n")
    });
    // 初期値は元のタスクのプロジェクト。
    press(&mut app, "p");
    assert_eq!(
        screen(&mut app, 80, 12).pop().unwrap(),
        "Project tag (empty for plain split): work"
    );
    for _ in 0..4 {
        press_code(&mut app, KeyCode::Backspace);
    }
    press(&mut app, "launch");
    press_code(&mut app, KeyCode::Enter);
    assert!(app.editor_requested());
    app.run_editor();
    let (copy, line, text) = opened.borrow()[0].clone();
    assert!(text.starts_with("<!-- 分割するタスク: 今日1 -->"), "{text}");
    assert_eq!(text.lines().nth(line.unwrap() - 1), Some("- [ ] "));
    assert!(!copy.exists(), "the file is removed once used");
    let todo = f.read("todo.md");
    assert!(!todo.contains("今日1"), "{todo}");
    // 優先度は引き継がず、書かなかった項目 (コンテキスト・期日) は引き継ぐ。
    assert!(
        todo.contains("- [ ] ワイヤー +launch @office due:2026-11-16 id:"),
        "{todo}"
    );
    assert!(todo.contains("- [ ] 実装 +launch @pc due:2026-11-20 id:"), "{todo}");
    assert!(app.messages.iter().any(|m| m.text == "split into 2 task(s)"));
    assert!(app.messages.iter().any(|m| m.text == "created projects/launch.md"));
    assert!(f.path().join("projects/launch.md").exists());
}

#[test]
fn split_asks_again_when_a_line_is_not_a_task_and_does_nothing_when_nothing_is_written() {
    let f = Fixture::new();
    let mut app = f.app();
    let before = f.read("todo.md");
    fake_editor(&mut app, |n, text| match n {
        0 => text.replace("- [ ] \n", "メモ\n"),
        1 => text.replace("メモ\n", "- [ ] 書き直した\n"),
        _ => text.to_string(),
    });
    press(&mut app, "j");
    press(&mut app, "p");
    press_code(&mut app, KeyCode::Enter);
    app.run_editor();
    assert!(matches!(&app.mode, Mode::FixEdit { name, .. } if name == "the subtasks"));
    assert_eq!(f.read("todo.md"), before);
    press(&mut app, "y");
    app.run_editor();
    assert!(f.read("todo.md").contains("- [ ] 書き直した id:"));
    assert!(!f.read("todo.md").contains("今日2"));
    // 何も書かなければ分割しない。
    let before = f.read("todo.md");
    press(&mut app, "p");
    press_code(&mut app, KeyCode::Enter);
    app.run_editor();
    assert_eq!(last_message(&app), "no subtasks were written, so nothing was split");
    assert_eq!(f.read("todo.md"), before);
    // 完了したタスクは分割しない。
    press(&mut app, "x");
    press(&mut app, "p");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(last_message(&app), "completed tasks cannot be split");
}

#[test]
fn a_task_keeps_a_project_whose_name_cannot_be_a_file_name_when_split() {
    let f = Fixture::new();
    f.write("inbox.md", "# Inbox\n\n- [ ] 手書き +a/b id:111111\n");
    let mut app = f.app();
    fake_editor(&mut app, |_, text| text.replace("- [ ] \n", "- [ ] 続き\n"));
    press(&mut app, "1p");
    press_code(&mut app, KeyCode::Enter);
    app.run_editor();
    assert!(
        f.read("inbox.md").contains("- [ ] 続き +a/b id:"),
        "{}",
        f.read("inbox.md")
    );
    assert!(
        !f.path().join("projects").exists(),
        "no file for a name that cannot be a file name"
    );
}
