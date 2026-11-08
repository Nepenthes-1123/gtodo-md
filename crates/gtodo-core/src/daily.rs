//! 日次ロールオーバー (Lua 版 `daily.lua` の `check_daily_rollover`)。

use chrono::NaiveDate;

use crate::logic::due::DueOutcome;
use crate::ops::{self, OpError};
use crate::sections::SectionNames;
use crate::state::{State, StateError};
use crate::store::DataDir;

/// [`rollover`] の結果。
#[derive(Debug)]
pub struct Rollover {
    /// done.md へ移した完了タスクの数。
    pub completed: usize,
    pub dues: DueOutcome,
    /// 実行したことを `.state.json` に記録できなかった理由。記録できなければ次の機会にもう一度実行される。
    pub not_recorded: Option<StateError>,
}

#[derive(Debug, thiserror::Error)]
pub enum RolloverError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Op(#[from] OpError),
}

/// 今日まだ実行していなければ、完了タスクを done.md へ移し、期日が来たタスクを Today へ移して
/// 並べ替え、`.state.json` の `last_opened` に今日を記録する。今日すでに実行していれば `None`。
///
/// ロックは取らない。Neovim 版の日次ロールオーバーと重ならないよう、呼び出し元が
/// [`crate::lock::Lock`] を持ったまま呼ぶ。途中で失敗したら記録しないので、次の機会にやり直される。
pub fn rollover(
    data_dir: &DataDir,
    names: &SectionNames,
    today: NaiveDate,
    auto_move_inbox: bool,
) -> Result<Option<Rollover>, RolloverError> {
    let path = data_dir.state_file();
    let today_str = today.format("%Y-%m-%d").to_string();
    if State::read(&path)?.last_opened() == Some(today_str.as_str()) {
        return Ok(None);
    }

    let completed = ops::move_completed(data_dir, names, today)?;
    let dues = ops::check_dues(data_dir, names, today, auto_move_inbox)?;
    if completed > 0 || dues.todo_changed {
        ops::sort_todo(data_dir, names, today)?;
    }

    // 処理の間に Neovim 版が `.state.json` を書いていると、最初に読んだ内容では照合が食い違って
    // 記録できないため、記録する直前に読み直す。
    let not_recorded = State::read(&path)
        .and_then(|mut state| {
            state.set_last_opened(today);
            state.write(&path)
        })
        .err();
    Ok(Some(Rollover {
        completed,
        dues,
        not_recorded,
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    fn run(dir: &std::path::Path) -> Option<Rollover> {
        rollover(&DataDir::new(dir), &SectionNames::default(), today(), true).unwrap()
    }

    #[test]
    fn runs_once_a_day_and_keeps_other_state() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".state.json"),
            r#"{"last_opened":"2026-02-02","last_notify_time":5}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("todo.md"),
            "# Todo\n\n## Today\n\n- [x] 完了 completed_at:2026-02-02 id:aaaaaa\n\n## Next\n\n\
             - [ ] 期日 due:2026-02-03 id:bbbbbb\n",
        )
        .unwrap();

        let first = run(dir.path()).unwrap();
        assert_eq!((first.completed, first.dues.moved), (1, 1));
        assert!(first.not_recorded.is_none());
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(".state.json")).unwrap()).unwrap();
        assert_eq!(
            state,
            serde_json::json!({"last_opened": "2026-02-03", "last_notify_time": 5})
        );

        fs::write(
            dir.path().join("todo.md"),
            "# Todo\n\n## Today\n\n- [x] 2 回目 completed_at:2026-02-03 id:cccccc\n",
        )
        .unwrap();
        assert!(run(dir.path()).is_none());
        assert!(
            fs::read_to_string(dir.path().join("todo.md"))
                .unwrap()
                .contains("id:cccccc")
        );
    }

    #[test]
    fn sorts_todo_only_when_something_moved() {
        let dir = tempfile::tempdir().unwrap();
        let unsorted = "# Todo\n\n## Today\n\n- [ ] 期日なし id:aaaaaa\n- [ ] 期日あり due:2026-03-01 id:bbbbbb\n";
        fs::write(dir.path().join("todo.md"), unsorted).unwrap();
        run(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), unsorted);

        fs::remove_file(dir.path().join(".state.json")).unwrap();
        fs::write(
            dir.path().join("inbox.md"),
            "# Inbox\n\n- [x] 完了 completed_at:2026-02-01 id:cccccc\n",
        )
        .unwrap();
        run(dir.path()).unwrap();
        let todo = fs::read_to_string(dir.path().join("todo.md")).unwrap();
        assert!(todo.find("id:bbbbbb") < todo.find("id:aaaaaa"), "{todo}");
    }

    #[test]
    fn a_failed_move_is_not_recorded_so_it_runs_again() {
        let dir = tempfile::tempdir().unwrap();
        let todo = "# Todo\n\n## Today\n\n- [x] 完了 completed_at:2026-02-02 id:aaaaaa\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        // 読めない done.md。
        fs::create_dir(dir.path().join("done.md")).unwrap();

        let err = rollover(&DataDir::new(dir.path()), &SectionNames::default(), today(), true).unwrap_err();
        assert!(matches!(err, RolloverError::Op(_)), "{err:?}");
        assert!(!dir.path().join(".state.json").exists());
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), todo);
    }

    #[test]
    fn a_broken_state_file_stops_before_anything_is_moved() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".state.json"), "{broken").unwrap();
        let todo = "# Todo\n\n## Today\n\n- [x] 完了 completed_at:2026-02-02 id:aaaaaa\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();

        let err = rollover(&DataDir::new(dir.path()), &SectionNames::default(), today(), true).unwrap_err();
        assert!(matches!(err, RolloverError::State(_)), "{err:?}");
        assert_eq!(fs::read_to_string(dir.path().join("todo.md")).unwrap(), todo);
    }
}
