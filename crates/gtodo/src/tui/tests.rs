//! TUI のテスト。一時ディレクトリのデータに対してキーを送り、ファイルと画面を確かめる。
//!
//! 画面は ratatui の `TestBackend` に描画して、文字だけを比べる (色や太字は見ない)。

use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use super::app::{App, Level, Mode};
use super::draw::{self, Theme};
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
}

impl Fixture {
    fn new() -> Fixture {
        let f = Fixture {
            dir: tempfile::tempdir().unwrap(),
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
        App::new(self.path(), today(), Keymap::default(), Vec::new()).unwrap()
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
            "x 完了にする  n Next へ  w Waiting へ  s Someday へ  c キャンセル  o 並べ替え  ? ヘルプ  q 終了",
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
        "x 未完了に戻す  c キャンセル  o 並べ替え  ? ヘルプ  q 終了"
    );

    press(&mut app, "4");
    assert_eq!(
        footer(&mut app),
        "x 完了にする  d Today へ  n Next へ  w 待ち先を変える  s Someday へ  c キャンセル  o 並べ替え  ? ヘルプ  q 終了"
    );

    // 空のタブでは、タスクの操作を出さない。
    press(&mut app, "3");
    assert_eq!(footer(&mut app), "o 並べ替え  ? ヘルプ  q 終了");

    // done.md は見るだけ。
    press(&mut app, "6");
    assert_eq!(footer(&mut app), "? ヘルプ  q 終了");
}

#[test]
fn hints_that_do_not_fit_are_dropped_but_help_stays() {
    let f = Fixture::new();
    let mut app = f.app();
    assert_eq!(
        screen(&mut app, 60, 5)[4],
        "x 完了にする  n Next へ  w Waiting へ  ? ヘルプ"
    );
    assert_eq!(screen(&mut app, 30, 5)[4], "x 完了にする  ? ヘルプ");
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
        "x 完了にする  n 次 へ  w 待ち へ  s いつか へ  c キャンセル  o 並べ替え  ? ヘルプ  q 終了"
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
    assert_eq!(footer(&mut app), "gg 先頭へ  gt 次のタブ  gT 前のタブ  Esc 取り消す");
    press_code(&mut app, KeyCode::Esc);
    assert!(app.pending.is_empty());
    assert!(!app.should_quit(), "Esc only cancels the pending key");
    press_code(&mut app, KeyCode::Esc);
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
        "q / Esc",
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
    app.tick(today());
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

    app.tick(date("2026-11-17"));
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
    app.tick(date("2026-11-17"));
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));
    app.tick(date("2026-11-17"));
    assert!(f.read("todo.md").contains("[x] (A) 今日1"));

    fs::remove_file(f.path().join(".gtodo.lock")).unwrap();
    app.tick(date("2026-11-17"));
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
    app.tick(today());
    assert_eq!(snapshot(&f), before);
}

#[test]
fn a_broken_settings_file_stops_writes_until_it_is_fixed() {
    let f = Fixture::new();
    let mut app = f.app();
    f.write("gtodo.json", "{");
    app.tick(today());
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
    app.tick(today());
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
    app.tick(today());
    assert!(app.messages.iter().all(|m| !m.text.contains("unknown key")));
}
