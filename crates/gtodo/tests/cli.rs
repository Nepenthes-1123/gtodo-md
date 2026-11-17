//! `gtodo` コマンドの結合テスト。実際のバイナリを一時ディレクトリのデータに対して実行する。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

use chrono::{Days, Local};
use serde_json::Value;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let f = Fixture {
            dir: tempfile::tempdir().unwrap(),
        };
        fs::create_dir(f.data_dir()).unwrap();
        // 利用者自身の設定ファイルを読まないよう、空の設定ファイルを明示する。
        fs::write(f.config_path(), "{}").unwrap();
        f
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.path().join("config.json")
    }

    fn write(&self, name: &str, content: &str) -> &Self {
        fs::write(self.data_dir().join(name), content).unwrap();
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gtodo"))
            .arg("--data-dir")
            .arg(self.data_dir())
            .args(args)
            .env("GTODO_CONFIG", self.config_path())
            .env_remove("GTODO_DATA_DIR")
            .output()
            .unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "gtodo {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.stdout(args)).unwrap()
    }
}

fn contents(rows: &Value) -> Vec<&str> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].as_str().unwrap())
        .collect()
}

fn sample() -> Fixture {
    let f = Fixture::new();
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信1\n- [ ] 受信2 +work id:111111\n");
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] (A) 今日1 +work @office id:aaaaaa\n- [x] 完了済み id:bbbbbb\n\n\
         ## Next\n\n- [ ] 次1 @home\n\n## Waiting\n\n- [ ] 待ち1 wait:bob id:cccccc\n\n## Someday\n",
    );
    f.write(
        "done.md",
        "# Done\n\n## 2026-01\n\n- [x] 過去の完了 +work completed_at:2026-01-10 id:dddddd\n",
    );
    f.write(
        "cancelled.md",
        "# Cancelled\n\n## 2026-01\n\n- [ ] 取りやめ +work cancelled:2026-01-11 id:eeeeee\n",
    );
    f
}

#[test]
fn list_without_target_shows_inbox_and_todo_grouped_by_file_and_section() {
    let out = sample().stdout(&["list"]);
    let groups: Vec<&str> = out.lines().filter(|l| !l.starts_with(' ') && !l.is_empty()).collect();
    assert_eq!(
        groups,
        ["inbox.md", "todo.md / Today", "todo.md / Next", "todo.md / Waiting"]
    );
    assert!(out.contains("  aaaaaa   [ ] (A) 今日1 +work @office\n"), "{out}");
    assert!(!out.contains("過去の完了") && !out.contains("取りやめ"), "{out}");
}

#[test]
fn list_target_selects_one_file_or_one_section() {
    let f = sample();
    assert_eq!(contents(&f.json(&["list", "today", "--json"])), ["今日1", "完了済み"]);
    assert_eq!(
        contents(&f.json(&["list", "todo", "--json"])),
        ["今日1", "完了済み", "次1", "待ち1"]
    );
    assert_eq!(contents(&f.json(&["list", "done", "--json"])), ["過去の完了"]);
    assert_eq!(contents(&f.json(&["list", "cancelled", "--json"])), ["取りやめ"]);
    assert_eq!(f.stdout(&["list", "someday"]), "No tasks.\n");
}

#[test]
fn list_keys_are_ids_or_line_based_temporary_keys() {
    let rows = sample().json(&["list", "inbox", "--json"]);
    let keys: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert!(keys[0].starts_with('~') && keys[0].len() == 7, "{keys:?}");
    assert_eq!(keys[1], "111111");
    assert_eq!(rows[0]["id"], Value::Null);
}

#[test]
fn section_names_from_gtodo_json_accept_default_headings() {
    let f = sample();
    f.write("gtodo.json", r#"{"sections":{"today":"今日"}}"#);
    let rows = f.json(&["list", "today", "--json"]);
    assert_eq!(contents(&rows), ["今日1", "完了済み"]);
    assert_eq!(rows[0]["section"], "今日");
}

#[test]
fn broken_gtodo_json_stops_instead_of_misreading_sections() {
    let f = sample();
    f.write("gtodo.json", r#"{"sections":{"today":1}}"#);
    let out = f.run(&["list"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("gtodo.json"), "{out:?}");
}

#[test]
fn unknown_keys_in_shared_settings_are_warned_but_ignored() {
    let f = sample();
    f.write("gtodo.json", r#"{"added_in_a_newer_version":true}"#);
    let config = serde_json::json!({"data_dir": f.data_dir(), "also_newer": 1});
    fs::write(f.config_path(), config.to_string()).unwrap();
    // --data-dir や GTODO_DATA_DIR があると config.json は読まれないため、どちらも渡さない。
    let out = Command::new(env!("CARGO_BIN_EXE_gtodo"))
        .arg("list")
        .env("GTODO_CONFIG", f.config_path())
        .env_remove("GTODO_DATA_DIR")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"also_newer\""), "{stderr}");
    assert!(stderr.contains("\"added_in_a_newer_version\""), "{stderr}");
}

#[test]
fn section_names_neovim_used_last_are_still_recognized() {
    let f = sample();
    f.write("todo.md", "# Todo\n\n## 本日\n\n- [ ] 旧名の見出し\n");
    f.write(".state.json", r#"{"last_sections":{"TODAY":"本日"}}"#);
    let rows = f.json(&["list", "today", "--json"]);
    assert_eq!(contents(&rows), ["旧名の見出し"]);
    assert_eq!(
        rows[0]["section"], "本日",
        "gtodo.json に無いので Neovim 版の名前が現在の名前"
    );
}

#[test]
fn broken_state_file_is_reported_but_listing_continues() {
    let f = sample();
    f.write(".state.json", "{broken");
    let out = f.run(&["list", "today"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning"), "{out:?}");
}

#[test]
fn search_combines_terms_and_status_with_and_and_skips_cancelled() {
    let f = sample();
    assert_eq!(
        contents(&f.json(&["search", "+work", "--json"])),
        ["受信2", "今日1", "過去の完了"]
    );
    assert_eq!(contents(&f.json(&["search", "+work", "@office", "--json"])), ["今日1"]);
    assert_eq!(
        contents(&f.json(&["search", "+work", "--open", "--json"])),
        ["受信2", "今日1"]
    );
    assert_eq!(
        contents(&f.json(&["search", "--done", "--json"])),
        ["完了済み", "過去の完了"]
    );
    let out = f.stdout(&["search", "@home"]);
    assert!(out.contains("todo.md:10  [ ] 次1 @home"), "{out}");
}

#[test]
fn search_finds_words_in_the_description_and_wait() {
    let f = sample();
    // 本文の一部・大文字小文字と全角半角を区別しない (TUI の検索と同じ規則)。
    assert_eq!(contents(&f.json(&["search", "今日", "--json"])), ["今日1"]);
    assert_eq!(
        contents(&f.json(&["search", "＋ＷＯＲＫ", "@Office", "--json"])),
        ["今日1"]
    );
    assert_eq!(contents(&f.json(&["search", "wait:bob", "--json"])), ["待ち1"]);
    // タグは完全一致 (CLI ではすべての語が確定している)。
    assert_eq!(contents(&f.json(&["search", "+wo", "--json"])), Vec::<&str>::new());
    // キャンセル済みのタスクやタグ・日付の文字は探さない。
    assert_eq!(contents(&f.json(&["search", "id", "--json"])), Vec::<&str>::new());
}

#[test]
fn search_rejects_tags_without_a_name() {
    for term in ["+", "@", "wait:"] {
        let out = sample().run(&["search", term]);
        assert!(!out.status.success(), "{term}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("has no name after it"),
            "{term}"
        );
    }
}

#[test]
fn queue_groups_open_tasks_by_due_date_relative_to_today() {
    let today = Local::now().date_naive();
    let day = |offset: i64| {
        let d = if offset < 0 {
            today.checked_sub_days(Days::new(offset.unsigned_abs()))
        } else {
            today.checked_add_days(Days::new(offset as u64))
        };
        d.unwrap().format("%Y-%m-%d").to_string()
    };
    let f = Fixture::new();
    f.write("inbox.md", &format!("# Inbox\n- [ ] 受信の期限切れ due:{}\n", day(-2)));
    f.write(
        "todo.md",
        &format!(
            "# Todo\n## Today\n- [ ] 今日 due:{}\n- [x] 完了済み due:{}\n- [ ] いつか due:someday\n\
             ## Next\n- [ ] 明日 due:{}\n- [ ] 期日なし\n",
            day(0),
            day(-5),
            day(1)
        ),
    );
    let out = f.stdout(&["queue"]);
    let lines: Vec<&str> = out.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    assert_eq!(lines[0], format!("Queue (Due)  {today}"));
    assert_eq!(lines[1], "期限切れ");
    assert!(lines[2].ends_with("⚠ 受信の期限切れ (2日超過)"), "{out}");
    assert!(lines[3].starts_with("今日 ("), "{out}");
    assert!(lines[4].ends_with("▶ 今日"), "{out}");
    assert!(lines[5].starts_with("明日 ("), "{out}");
    assert!(lines[6].ends_with("▶ 明日"), "{out}");
    assert_eq!(lines[7], "期日を解釈できない");
    assert!(lines[8].ends_with("▶ いつか (due:someday)"), "{out}");
    assert_eq!(lines.len(), 9, "{out}");
}

#[test]
fn queue_wait_groups_by_person_and_reports_when_empty() {
    let f = sample();
    let out = f.stdout(&["queue", "--wait"]);
    assert!(out.contains("bob 待ち\n  cccccc   ▶ 待ち1\n"), "{out}");

    let empty = Fixture::new();
    assert!(
        empty
            .stdout(&["queue", "--wait"])
            .contains("誰かの作業を待っているタスクはありません")
    );
    assert!(empty.stdout(&["queue"]).contains("期限付きタスクはありません"));
}

#[test]
fn stats_counts_today_tasks_under_the_section_name_neovim_used_last() {
    let f = sample();
    f.write("todo.md", "# Todo\n\n## 本日\n\n- [ ] 旧名の見出し\n");
    f.write(".state.json", r#"{"last_sections":{"TODAY":"本日"}}"#);
    assert_eq!(f.stdout(&["stats"]), "📝 Today: 1 | 📥 Inbox: 2\n");
}

#[test]
fn stats_counts_open_today_and_inbox_tasks() {
    let f = sample();
    assert_eq!(f.stdout(&["stats"]), "📝 Today: 1 | 📥 Inbox: 2\n");
    assert_eq!(
        f.json(&["stats", "--json"]),
        serde_json::json!({"today": 1, "inbox": 2})
    );
    assert_eq!(Fixture::new().stdout(&["stats"]), "✨ All Done!\n");
}

#[test]
fn missing_data_dir_is_an_error() {
    let f = Fixture::new();
    fs::remove_dir(f.data_dir()).unwrap();
    let out = f.run(&["list"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("data directory not found"));
}

/// ディレクトリ内の全エントリ (サブディレクトリを含む) の内容と更新時刻。
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            out.extend(snapshot(&path));
            out.insert(path, (None, meta.modified().unwrap()));
        } else {
            out.insert(path.clone(), (Some(fs::read(&path).unwrap()), meta.modified().unwrap()));
        }
    }
    out
}

const READ_ONLY_COMMANDS: &[&[&str]] = &[
    &["list"],
    &["list", "today", "--json"],
    &["list", "done", "--json"],
    &["list", "cancelled"],
    &["search", "+work"],
    &["search", "@office", "--open", "--json"],
    &["queue"],
    &["queue", "--wait"],
    &["stats"],
    &["stats", "--json"],
    &["project", "list"],
    &["project", "list", "--archived", "--json"],
    &["project", "show", "work"],
    &["project", "show", "+work", "--json"],
];

/// 読むだけのコマンドが書き込むと、Neovim 側の未保存の編集が破棄されるため、一切書かないこと。
#[test]
fn read_only_commands_never_create_or_modify_files() {
    let f = sample();
    // 無いファイルを作る経路にも乗らないことを確かめるため、1 つ欠けた状態にする。
    fs::remove_file(f.data_dir().join("cancelled.md")).unwrap();
    let before = snapshot(&f.data_dir());
    for args in READ_ONLY_COMMANDS {
        f.stdout(args);
    }
    assert_eq!(
        snapshot(&f.data_dir()),
        before,
        "no file may be created, including .state.json"
    );
}

#[test]
fn read_only_commands_never_modify_existing_state_and_settings() {
    let f = sample();
    f.write(".state.json", r#"{"last_opened":"2000-01-01"}"#);
    f.write("gtodo.json", r#"{"sections":{"today":"Today"}}"#);
    let before = snapshot(&f.data_dir());
    for args in READ_ONLY_COMMANDS {
        f.stdout(args);
    }
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn the_tui_refuses_to_start_without_a_terminal_and_writes_nothing() {
    // 引数なしで TUI が開くようになった。スクリプトから誤って実行しても、端末を待って止まったり
    // 日次ロールオーバーでファイルを書き換えたりしない。
    let f = sample();
    let before = snapshot(&f.data_dir());
    for args in [&[][..], &["tui"]] {
        let out = f.run(args);
        assert!(!out.status.success(), "gtodo {args:?} should fail");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("the TUI needs a terminal"), "{stderr}");
    }
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn tui_toml_next_to_config_json_is_checked_before_starting() {
    let f = sample();
    // 書き方の誤りは、どこから実行しても (端末でなくても) どこが誤りかを添えて知らせる。
    fs::write(f.dir.path().join("tui.toml"), "[keys]\nquit = \"x\"\n").unwrap();
    let out = f.run(&[]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("tui.toml") && stderr.contains("x is bound to both toggle_done and quit"),
        "{stderr}"
    );

    // 正しければ、端末でないことで止まる。
    fs::write(f.dir.path().join("tui.toml"), "[keys]\nquit = \"Z\"\n").unwrap();
    let out = f.run(&[]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("the TUI needs a terminal"));
}

fn today_str() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

fn read(f: &Fixture, name: &str) -> String {
    fs::read_to_string(f.data_dir().join(name)).unwrap()
}

#[test]
fn sort_moves_due_tasks_to_today_and_sorts_todo() {
    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write(
        "todo.md",
        &format!(
            "# Todo\n\n## Today\n\n- [ ] 期日なし id:aaaaaa\n- [ ] (A) 優先 id:bbbbbb\n\n\
             ## Next\n\n- [ ] 期日が来た due:{} id:cccccc\n",
            today_str()
        ),
    );
    let out = f.stdout(&["sort"]);
    assert_eq!(out, "moved 1 due task(s) to Today\n");
    let todo = read(&f, "todo.md");
    let order: Vec<_> = ["cccccc", "bbbbbb", "aaaaaa"]
        .iter()
        .map(|id| todo.find(id).unwrap())
        .collect();
    assert!(order.is_sorted(), "{todo}");
}

/// 日付が変わった日にも、ロールオーバーで inbox のタスクを移さず、警告は 1 回だけ出すこと。
#[test]
fn due_inbox_tasks_are_reported_once_when_they_are_not_moved_automatically() {
    for last_opened in [today_str(), "2000-01-01".to_string()] {
        let f = Fixture::new();
        f.write(".state.json", &format!(r#"{{"last_opened":"{last_opened}"}}"#));
        f.write("gtodo.json", r#"{"auto_move_inbox_to_today":false}"#);
        f.write(
            "inbox.md",
            &format!("# Inbox\n\n- [ ] 受信 due:{} id:aaaaaa\n", today_str()),
        );
        let out = f.run(&["sort"]);
        assert!(out.status.success(), "{out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stderr.matches("受信").count(), 1, "{last_opened}: {stderr}");
        assert!(read(&f, "inbox.md").contains("id:aaaaaa"), "{last_opened}");
    }
}

#[test]
fn write_commands_create_missing_files_with_the_section_names_neovim_used_last() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_sections":{"TODAY":"本日"}}"#);
    f.stdout(&["sort"]);
    assert_eq!(read(&f, "inbox.md"), "# Inbox\n\n");
    assert_eq!(
        read(&f, "todo.md"),
        "# Todo\n\n## 本日\n\n## Next\n\n## Waiting\n\n## Someday\n"
    );
    assert_eq!(read(&f, "done.md"), "# Done\n\n");
    assert_eq!(read(&f, "cancelled.md"), "# Cancelled\n\n");
}

/// gtodo.json が無いとき、Neovim 版が使っている名前の見出しを既定の名前へ書き換えないこと
/// (書き換えると Neovim 版が書き戻し、往復が続く)。
#[test]
fn headings_keep_the_section_names_neovim_used_last_without_gtodo_json() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_sections":{"TODAY":"本日"}}"#);
    f.write("todo.md", "# Todo\n\n## Today\n\n- [ ] 既定名の見出し id:aaaaaa\n");
    f.stdout(&["sort"]);
    assert!(
        read(&f, "todo.md").starts_with("# Todo\n\n## 本日\n"),
        "{}",
        read(&f, "todo.md")
    );
}

#[test]
fn the_daily_rollover_runs_before_the_command_once_a_day() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_opened":"2000-01-01","last_notify_time":5}"#);
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [x] 完了 completed_at:2026-01-10 id:aaaaaa\n",
    );
    let out = f.run(&["sort"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("moved 1 completed task(s) to done.md"));
    assert!(
        read(&f, "done.md").contains("## 2026-01\n\n- [x] 完了"),
        "{}",
        read(&f, "done.md")
    );
    let state: Value = serde_json::from_str(&read(&f, ".state.json")).unwrap();
    assert_eq!(
        state,
        serde_json::json!({"last_opened": today_str(), "last_notify_time": 5})
    );

    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [x] 今日の完了 completed_at:2026-01-11 id:bbbbbb\n",
    );
    f.stdout(&["sort"]);
    assert!(
        read(&f, "todo.md").contains("id:bbbbbb"),
        "the rollover already ran today"
    );
}

#[test]
fn write_commands_give_up_when_another_process_holds_the_lock() {
    let f = sample();
    fs::remove_file(f.data_dir().join("cancelled.md")).unwrap();
    f.write(".gtodo.lock", "12345 0");
    let before = snapshot(&f.data_dir());
    let out = f.run(&["sort"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(".gtodo.lock"), "{out:?}");
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn write_commands_release_the_lock() {
    let f = sample();
    f.stdout(&["sort"]);
    assert!(!f.data_dir().join(".gtodo.lock").exists());
}

/// 記録された名前が他のセクションの名前と重なっていると、既定の名前で書けば Neovim 版の
/// セクションどうしが合流してしまうため、読むことはできても書くことはできない。
#[test]
fn unusable_section_names_neovim_used_last_stop_write_commands() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_sections":{"TODAY":"Now","NEXT":"Today"}}"#);
    f.write(
        "todo.md",
        "# Todo\n\n## Now\n\n- [ ] 今やる id:aaaaaa\n\n## Today\n\n- [ ] 次にやる id:bbbbbb\n",
    );
    let out = f.run(&["list"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid"), "{out:?}");

    let before = snapshot(&f.data_dir());
    let out = f.run(&["sort"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("gtodo.json"), "{out:?}");
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn write_commands_clean_up_stale_temporary_files() {
    let f = sample();
    fs::create_dir(f.data_dir().join("projects")).unwrap();
    let day_ago = SystemTime::now() - std::time::Duration::from_secs(25 * 60 * 60);
    for path in [
        f.data_dir().join(".todo.md.1.1.tmp"),
        f.data_dir().join("projects/.p.md.1.1.tmp"),
    ] {
        fs::write(&path, "").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(day_ago)
            .unwrap();
    }
    f.stdout(&["sort"]);
    assert!(!f.data_dir().join(".todo.md.1.1.tmp").exists());
    assert!(!f.data_dir().join("projects/.p.md.1.1.tmp").exists());
}

/// 記録だけ失敗してもコマンドは成功させ、次の機会にもう一度実行されることを知らせる。
#[cfg(unix)]
#[test]
fn a_rollover_that_cannot_be_recorded_is_warned_and_the_command_still_runs() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let f = sample();
    // 読めるが書けない .state.json (書き込めないディレクトリにある実体へのリンク)。
    let ro = f.dir.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::write(ro.join(".state.json"), r#"{"last_opened":"2000-01-01"}"#).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
    symlink(ro.join(".state.json"), f.data_dir().join(".state.json")).unwrap();

    let out = f.run(&["sort"]);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("will run again"), "{stderr}");
    assert!(read(&f, "done.md").contains("id:bbbbbb"), "the rollover itself ran");
}

/// `added <id> to ...` の行から ID を取り出す。
fn added_id(stdout: &str) -> String {
    stdout.split_whitespace().nth(1).unwrap().to_string()
}

#[test]
fn add_puts_a_task_in_the_inbox_with_its_creation_date() {
    let f = sample();
    let out = f.stdout(&["add", "牛乳を買う", "+home"]);
    let id = added_id(&out);
    assert_eq!(out, format!("added {id} to inbox.md\n"));
    assert!(
        read(&f, "inbox.md").ends_with(&format!("- [ ] 牛乳を買う +home created:{} id:{id}\n", today_str())),
        "{}",
        read(&f, "inbox.md")
    );
}

#[test]
fn add_to_a_section_keeps_a_future_due_date_there() {
    let f = sample();
    let out = f.stdout(&["add", "資料 due:2099-01-01", "--to", "next"]);
    let id = added_id(&out);
    assert_eq!(out, format!("added {id} to Next\n"));
    let rows = f.json(&["list", "next", "--json"]);
    assert_eq!(contents(&rows), ["資料", "次1"], "sorted within the section");
}

#[test]
fn options_override_tags_in_the_text_of_an_added_task() {
    let f = sample();
    let out = f.stdout(&[
        "add",
        "(A) 資料 +home due:today",
        "--due",
        "2099-01-01",
        "-p",
        "work",
        "-c",
        "@office",
        "--priority",
        "B",
    ]);
    let id = added_id(&out);
    assert!(read(&f, "inbox.md").contains(&format!(
        "- [ ] (B) 資料 +work @office due:2099-01-01 created:{} id:{id}",
        today_str()
    )));
}

#[test]
fn a_task_added_to_the_inbox_that_is_already_due_moves_to_today() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let out = f.stdout(&["add", "今日まで due:today"]);
    assert!(out.ends_with("moved 1 due task(s) to Today\n"), "{out}");
    let id = added_id(&out);
    assert!(!read(&f, "inbox.md").contains(&id));
    assert_eq!(
        contents(&f.json(&["list", "today", "--json"])),
        ["今日まで", "今日1", "完了済み"],
        "todo.md is sorted after the move"
    );
}

/// 日付が変わった日にも、期日が来た inbox のタスクの警告は 1 回だけ出す。
#[test]
fn due_inbox_tasks_are_reported_once_when_adding_on_a_new_day() {
    let f = Fixture::new();
    f.write(".state.json", r#"{"last_opened":"2000-01-01"}"#);
    f.write("gtodo.json", r#"{"auto_move_inbox_to_today":false}"#);
    f.write(
        "inbox.md",
        &format!("# Inbox\n\n- [ ] 受信 due:{} id:aaaaaa\n", today_str()),
    );
    let out = f.run(&["add", "別のタスク"]);
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("受信").count(), 1, "{stderr}");
}

/// 追加の後の期日のチェックに失敗しても、追加は済んでいるので成功として ID を知らせる
/// (失敗として返すと、実行し直されてタスクが二重になる)。
#[cfg(unix)]
#[test]
fn a_failure_after_the_task_was_added_is_only_a_warning() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    for name in ["inbox.md", "done.md", "cancelled.md"] {
        f.write(name, "");
    }
    let ro = f.dir.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::write(ro.join("todo.md"), "# Todo\n\n## Today\n").unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
    symlink(ro.join("todo.md"), f.data_dir().join("todo.md")).unwrap();

    let out = f.run(&["add", "今日まで due:today"]);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(out.status.success(), "{out:?}");
    let id = added_id(&String::from_utf8(out.stdout).unwrap());
    assert!(read(&f, "inbox.md").contains(&id));
    assert!(String::from_utf8_lossy(&out.stderr).contains("checking due dates failed"));
}

#[test]
fn invalid_tasks_are_rejected_before_anything_is_written() {
    let f = Fixture::new();
    for args in [
        &["add", "x due:2026-02-30"][..],
        &["add", "x id:abcdef"],
        &["add", "+only-a-tag"],
        &["add", "x", "--due", "someday"],
        &["add", "x", "-p", "two words"],
    ] {
        let out = f.run(args);
        assert!(!out.status.success(), "{args:?}");
        assert!(
            fs::read_dir(f.data_dir()).unwrap().next().is_none(),
            "{args:?} must not even create the missing files"
        );
    }
}

#[test]
fn done_takes_an_id_prefix_and_refuses_to_repeat() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(f.stdout(&["done", "aaaa"]), "completed aaaaaa: 今日1\n");
    assert!(read(&f, "todo.md").contains(&format!(
        "- [x] (A) 今日1 +work @office completed_at:{} id:aaaaaa",
        today_str()
    )));
    let again = f.run(&["done", "aaaaaa"]);
    assert!(!again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("already completed"),
        "{again:?}"
    );
}

#[test]
fn undone_reopens_a_completed_task_and_refuses_an_open_one() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.stdout(&["done", "aaaaaa"]);
    assert_eq!(f.stdout(&["undone", "aaaaaa"]), "reopened aaaaaa: 今日1\n");
    assert!(read(&f, "todo.md").contains("- [ ] (A) 今日1 +work @office id:aaaaaa"));
    let again = f.run(&["undone", "aaaaaa"]);
    assert!(!again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("not completed"),
        "{again:?}"
    );
}

/// ID の無いタスクは一時キーで指定でき、完了にしたときに付いた ID を知らせる。
/// 行が書き換わった後の一時キーは、`list` し直すよう案内する。
#[test]
fn a_task_without_an_id_is_found_by_its_temporary_key_only_until_its_line_changes() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let rows = f.json(&["list", "inbox", "--json"]);
    let key = rows[0]["key"].as_str().unwrap().to_string();
    assert!(key.starts_with('~'));

    let out = f.stdout(&["done", &key]);
    let id = out.strip_prefix("completed ").unwrap().split(':').next().unwrap();
    assert_eq!(id.len(), 6, "{out}");
    assert_eq!(out, format!("completed {id}: 受信1\n"));
    assert!(read(&f, "inbox.md").contains(&format!("id:{id}")));

    let stale = f.run(&["done", &key]);
    assert!(!stale.status.success());
    assert!(
        String::from_utf8_lossy(&stale.stderr).contains("gtodo list"),
        "{stale:?}"
    );
}

/// ID の先頭の一部は 4 文字以上が要る。ID 全体と一致するなら、手で書いた短い ID でも指定できる。
#[test]
fn short_keys_are_refused_unless_they_are_a_whole_id() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("inbox.md", "# Inbox\n\n- [ ] 手書き id:zz\n");
    let before = snapshot(&f.data_dir());
    let out = f.run(&["done", "aaa"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("too short"), "{out:?}");
    assert_eq!(snapshot(&f.data_dir()), before);

    assert_eq!(f.stdout(&["done", "zz"]), "completed zz: 手書き\n");
}

#[test]
fn a_key_matching_several_tasks_changes_nothing() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信 id:abc111\n");
    f.write("todo.md", "# Todo\n\n## Today\n\n- [ ] 今日 id:abc122\n");
    let before = snapshot(&f.data_dir());
    let out = f.run(&["done", "abc1"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("abc111") && stderr.contains("abc122"), "{stderr}");
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn cancel_moves_the_task_under_this_month_in_cancelled_md() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(f.stdout(&["cancel", "cccccc"]), "cancelled cccccc: 待ち1\n");
    assert!(!read(&f, "todo.md").contains("cccccc"));
    let month = &today_str()[..7];
    assert!(
        read(&f, "cancelled.md").ends_with(&format!(
            "## {month}\n\n- [ ] 待ち1 wait:bob cancelled:{} id:cccccc\n",
            today_str()
        )),
        "{}",
        read(&f, "cancelled.md")
    );
}

/// ID の無いタスクを操作したときに表示する ID が、書き込まれた ID と一致すること。
#[test]
fn the_reported_id_of_a_task_without_one_is_the_id_written() {
    for (op, file, line, written_to) in [
        (&["done"][..], "todo.md", "- [ ] 今日", "todo.md"),
        (&["done"], "inbox.md", "- [ ] 受信", "inbox.md"),
        (&["cancel"], "todo.md", "- [ ] 今日", "cancelled.md"),
        (&["cancel"], "inbox.md", "- [ ] 受信", "cancelled.md"),
        (&["move", "next"], "todo.md", "- [ ] 今日", "todo.md"),
        (&["move", "next"], "inbox.md", "- [ ] 受信", "todo.md"),
        (&["edit", "直した"], "todo.md", "- [ ] 今日", "todo.md"),
        (&["edit", "直した"], "inbox.md", "- [ ] 受信", "inbox.md"),
    ] {
        let f = Fixture::new();
        f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
        f.write("inbox.md", "# Inbox\n\n- [ ] 受信\n");
        f.write("todo.md", "# Todo\n\n## Today\n\n- [ ] 今日\n- [ ] 他\n");
        let key = gtodo_core::key::line_key(line);
        let args: Vec<&str> = [op[0], &key].into_iter().chain(op[1..].iter().copied()).collect();
        let out = f.stdout(&args);
        let id = out.split_whitespace().nth(1).unwrap().trim_end_matches(':');
        assert_eq!(id.len(), 6, "{op:?} {file}: {out}");
        assert!(read(&f, written_to).contains(&format!("id:{id}")), "{op:?} {file}");
    }
}

#[test]
fn move_puts_a_task_in_another_section_and_drops_its_wait_outside_waiting() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(f.stdout(&["move", "cccc", "next"]), "moved cccccc to Next: 待ち1\n");
    let todo = read(&f, "todo.md");
    let next = &todo[todo.find("## Next").unwrap()..todo.find("## Waiting").unwrap()];
    assert!(next.contains("- [ ] 待ち1 id:cccccc\n"), "{todo}");
}

#[test]
fn moving_to_waiting_keeps_the_wait() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 今日 wait:bob id:aaaaaa\n\n## Next\n\n## Waiting\n\n## Someday\n",
    );
    assert_eq!(
        f.stdout(&["move", "aaaaaa", "waiting"]),
        "moved aaaaaa to Waiting (wait:bob): 今日\n"
    );
    assert!(read(&f, "todo.md").contains("## Waiting\n\n- [ ] 今日 wait:bob id:aaaaaa\n"));
}

#[test]
fn move_takes_a_task_out_of_the_inbox() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(f.stdout(&["move", "111111", "today"]), "moved 111111 to Today: 受信2\n");
    assert!(!read(&f, "inbox.md").contains("111111"));
    assert!(read(&f, "todo.md").contains("- [ ] 受信2 +work id:111111"));
}

#[test]
fn the_wait_option_sets_or_removes_the_wait() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(
        f.stdout(&["move", "aaaaaa", "waiting", "--wait", "alice"]),
        "moved aaaaaa to Waiting (wait:alice): 今日1\n"
    );
    assert_eq!(
        f.stdout(&["move", "aaaaaa", "waiting", "--wait", "carol"]),
        "set wait:carol on aaaaaa: 今日1\n"
    );
    assert!(read(&f, "todo.md").contains("- [ ] (A) 今日1 +work @office wait:carol id:aaaaaa"));
    assert_eq!(
        f.stdout(&["move", "aaaaaa", "waiting", "--wait", ""]),
        "removed wait: from aaaaaa: 今日1\n"
    );
    assert!(read(&f, "todo.md").contains("- [ ] (A) 今日1 +work @office id:aaaaaa"));

    assert_eq!(
        f.stdout(&["move", "111111", "waiting", "--wait", "bob"]),
        "moved 111111 to Waiting (wait:bob): 受信2\n"
    );
    assert!(read(&f, "todo.md").contains("- [ ] 受信2 +work wait:bob id:111111"));
}

/// 待ち先だけを変えたときも、ID の無いタスクに付いた ID を知らせる。
#[test]
fn changing_the_wait_of_a_task_without_an_id_reports_the_id_written() {
    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("todo.md", "# Todo\n\n## Waiting\n\n- [ ] 待ち\n- [ ] 他\n");
    let key = gtodo_core::key::line_key("- [ ] 待ち");
    let out = f.stdout(&["move", &key, "waiting", "--wait", "bob"]);
    let id = out
        .strip_prefix("set wait:bob on ")
        .and_then(|rest| rest.strip_suffix(": 待ち\n"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(read(&f, "todo.md").contains(&format!("- [ ] 待ち wait:bob id:{id}\n")));
}

#[test]
fn moving_a_task_to_the_section_it_is_in_writes_nothing() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let before = snapshot(&f.data_dir());
    assert_eq!(f.stdout(&["move", "aaaaaa", "today"]), "aaaaaa is already in Today\n");
    assert_eq!(
        f.stdout(&["move", "cccccc", "waiting"]),
        "cccccc is already in Waiting\n"
    );
    assert_eq!(
        f.stdout(&["move", "cccccc", "waiting", "--wait", "bob"]),
        "cccccc is already in Waiting\n"
    );
    let key = gtodo_core::key::line_key("- [ ] 次1 @home");
    assert_eq!(f.stdout(&["move", &key, "next"]), format!("{key} is already in Next\n"));
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn moves_that_cannot_be_done_as_asked_change_nothing() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let before = snapshot(&f.data_dir());
    for (args, message) in [
        (&["move", "aaaaaa", "next", "--wait", "bob"][..], "--wait"),
        (&["move", "aaaaaa", "waiting", "--wait", "bob smith"], "wait:"),
        (&["move", "bbbbbb", "next"], "completed"),
        (&["move", "dddddd", "next"], "done.md"),
    ] {
        let out = f.run(args);
        assert!(!out.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[test]
fn edit_changes_only_what_is_given() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    assert_eq!(
        f.stdout(&["edit", "aaaa", "新しい", "本文"]),
        "edited aaaaaa: 新しい 本文\n"
    );
    assert!(read(&f, "todo.md").contains(&format!(
        "- [ ] (A) 新しい 本文 +work @office created:{} id:aaaaaa\n",
        today_str()
    )));
    f.stdout(&["edit", "aaaaaa", "@home"]);
    assert!(read(&f, "todo.md").contains(&format!(
        "- [ ] (A) 新しい 本文 +work @home created:{} id:aaaaaa\n",
        today_str()
    )));
}

#[test]
fn edit_options_change_or_remove_values() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let next_week = (Local::now().date_naive() + Days::new(7))
        .format("%Y-%m-%d")
        .to_string();
    f.stdout(&[
        "edit",
        "aaaaaa",
        "wait:bob",
        "--due",
        "+1w",
        "-p",
        "",
        "--priority",
        "B",
    ]);
    assert!(read(&f, "todo.md").contains(&format!(
        "- [ ] (B) 今日1 @office due:{next_week} created:{} wait:bob id:aaaaaa\n",
        today_str()
    )));
}

#[test]
fn a_task_edited_to_be_due_moves_to_today() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("todo.md", "# Todo\n\n## Today\n\n## Next\n\n- [ ] 次 id:aaaaaa\n");
    assert_eq!(
        f.stdout(&["edit", "aaaaaa", "due:today"]),
        "edited aaaaaa: 次\nmoved 1 due task(s) to Today\n"
    );
    let todo = read(&f, "todo.md");
    assert!(
        todo.find("id:aaaaaa").unwrap() < todo.find("## Next").unwrap(),
        "{todo}"
    );
}

#[test]
fn edits_that_cannot_be_done_as_asked_change_nothing() {
    let f = sample();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    let before = snapshot(&f.data_dir());
    for (args, message) in [
        (&["edit", "aaaaaa"][..], "nothing to change"),
        (&["edit", "aaaaaa", "x", "created:2026-01-01"], "created:"),
        (&["edit", "aaaaaa", "due:someday"], "invalid due date"),
        (&["edit", "dddddd", "x"], "done.md"),
        (&["edit", "aaaaaa", "--priority", "ab"], "--priority"),
        (&["edit", "aaaaaa", "--due", "2026-02-30"], "invalid due date"),
    ] {
        let out = f.run(args);
        assert!(!out.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    assert_eq!(snapshot(&f.data_dir()), before);
}

#[cfg(unix)]
#[test]
fn a_failure_after_the_task_was_edited_is_only_a_warning() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    for name in ["done.md", "cancelled.md"] {
        f.write(name, "");
    }
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信 id:aaaaaa\n");
    let ro = f.dir.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::write(ro.join("todo.md"), "# Todo\n\n## Today\n").unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
    symlink(ro.join("todo.md"), f.data_dir().join("todo.md")).unwrap();

    let out = f.run(&["edit", "aaaaaa", "due:today"]);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(read(&f, "inbox.md").contains(&format!("due:{}", today_str())));
    assert!(String::from_utf8_lossy(&out.stderr).contains("checking due dates failed"));
}

/// todo.md のタスクを編集すると、Neovim 版と同じく todo.md 全体を並べ替える。
#[test]
fn editing_a_todo_task_sorts_todo_md() {
    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write(
        "todo.md",
        "# Todo\n\n## Today\n\n- [ ] 今日 id:aaaaaa\n\n## Someday\n\n- [ ] (B) 後 id:bbbbbb\n- [ ] (A) 先 id:cccccc\n",
    );
    f.stdout(&["edit", "aaaaaa", "今日の分"]);
    let todo = read(&f, "todo.md");
    assert!(
        todo.find("id:cccccc").unwrap() < todo.find("id:bbbbbb").unwrap(),
        "{todo}"
    );
}

/// inbox.md のタスクは、期日で動くものが無ければ todo.md に触れない。
#[test]
fn editing_an_inbox_task_leaves_todo_md_alone() {
    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信 id:aaaaaa\n");
    let todo = "# Todo\n\n## Someday\n\n- [ ] (B) 後 id:bbbbbb\n- [ ] (A) 先 id:cccccc\n";
    f.write("todo.md", todo);
    f.stdout(&["edit", "aaaaaa", "直した"]);
    assert_eq!(read(&f, "todo.md"), todo);
}

#[test]
fn an_edited_inbox_task_that_is_due_stays_when_it_is_not_moved_automatically() {
    let f = Fixture::new();
    f.write(".state.json", &format!(r#"{{"last_opened":"{}"}}"#, today_str()));
    f.write("gtodo.json", r#"{"auto_move_inbox_to_today":false}"#);
    f.write("inbox.md", "# Inbox\n\n- [ ] 受信 id:aaaaaa\n");
    f.write("todo.md", "# Todo\n\n## Today\n");
    let out = f.run(&["edit", "aaaaaa", "due:today"]);
    assert!(out.status.success(), "{out:?}");
    assert!(read(&f, "inbox.md").contains("id:aaaaaa"));
    assert_eq!(read(&f, "todo.md"), "# Todo\n\n## Today\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("due tasks in inbox.md"));
}

#[test]
fn projects_are_created_listed_with_progress_and_archived() {
    let f = sample();
    // ファイルが無くても、タスクに使われているタグなら表示できる (作り方を添える)。
    let shown = f.stdout(&["project", "show", "work"]);
    assert!(
        shown.contains("no project file (create it with `gtodo project new work`)"),
        "{shown}"
    );
    assert_eq!(f.stdout(&["project", "list"]), "No projects.\n");

    let created = f.stdout(&["project", "new", "+work"]);
    assert!(
        created.starts_with("created ") && created.trim_end().ends_with("work.md"),
        "{created}"
    );
    let file = fs::read_to_string(f.data_dir().join("projects/work.md")).unwrap();
    assert!(file.starts_with("---\ntitle:\ntag: work\ncreated: "), "{file}");
    let out = f.run(&["project", "new", "work"]);
    assert!(!out.status.success(), "creating it again must fail");
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));

    // 完了は done.md と完了済みのタスク、未完了は inbox.md と todo.md (cancelled.md は数えない)。
    assert_eq!(f.stdout(&["project", "list"]), "work  [███░░░░░░░] 33% (1/3 done)\n");
    let shown = f.stdout(&["project", "show", "work"]);
    assert!(shown.contains("  status:  active\n"), "{shown}");
    assert!(shown.contains("[ ] 受信2 +work  [inbox.md]"), "{shown}");
    assert!(shown.contains("[ ] (A) 今日1 +work @office  [Today]"), "{shown}");

    assert_eq!(f.stdout(&["project", "archive", "work"]), "archived work\n");
    assert_eq!(f.stdout(&["project", "archive", "work"]), "work is already archived\n");
    assert_eq!(f.stdout(&["project", "list"]), "No projects.\n");
    let json: serde_json::Value =
        serde_json::from_str(&f.stdout(&["project", "list", "--archived", "--json"])).unwrap();
    assert_eq!(json[0]["tag"], "work");
    assert_eq!(json[0]["status"], "archived");
    assert_eq!((json[0]["done"].as_u64(), json[0]["open"].as_u64()), (Some(1), Some(2)));
    assert_eq!(f.stdout(&["project", "restore", "work"]), "restored work\n");
}

#[test]
fn project_names_that_cannot_be_file_names_are_refused() {
    let f = sample();
    let out = f.run(&["project", "new", "../outside"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(r#"use "outside" instead"#), "{stderr}");
    assert!(!f.data_dir().join("outside.md").exists() && !f.data_dir().join("projects").exists());
    let out = f.run(&["project", "show", "nothing"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no project file (projects/nothing.md) and no tasks"));
}
