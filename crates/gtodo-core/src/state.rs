//! `data_dir/.state.json` (Neovim 版と共有する状態) の読み書き。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde_json::{Map, Value};

use crate::atomic::{self, WriteError};
use crate::sections::SectionKey;
use crate::store::Stamp;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("failed to read {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not a valid state file: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// `.state.json` の内容。
///
/// 知らないキーも含めて丸ごと保持し、書き戻すときに失わない (Neovim 版が書く due 通知の
/// 状態などを CLI が消さないため)。`last_sections` は Neovim 版が自身の設定変更を追跡するための
/// 記録なので、書き換える手段を用意しない。
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    values: Map<String, Value>,
    /// 読んだ時点の状態。書き戻す直前の照合に使う。
    stamp: Option<Stamp>,
}

impl State {
    /// ファイルが無い・空の場合は空の状態。壊れている場合はエラーを返す
    /// (Lua 版は黙って空として扱うが、それでは壊れていることに誰も気づけず、書き戻すと中身が消える)。
    pub fn read(path: &Path) -> Result<State, StateError> {
        let io_err = |source| StateError::Io {
            path: path.to_path_buf(),
            source,
        };
        let stamp = Stamp::of(path).map_err(io_err)?;
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(State {
                    values: Map::new(),
                    stamp: None,
                });
            }
            Err(source) => return Err(io_err(source)),
        };
        let invalid = |reason: String| StateError::Invalid {
            path: path.to_path_buf(),
            reason,
        };
        let text = String::from_utf8(bytes).map_err(|_| invalid("not valid UTF-8".into()))?;
        if text.trim().is_empty() {
            return Ok(State {
                values: Map::new(),
                stamp,
            });
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        let values = match value {
            Value::Object(values) => values,
            // Lua の vim.json.encode は空のテーブルを `[]` と書くため、空の状態として読む。
            Value::Array(items) if items.is_empty() => Map::new(),
            _ => return Err(invalid("top level is not an object".into())),
        };
        Ok(State { values, stamp })
    }

    /// Neovim 版が最後に起動したときに使っていたセクション名 (`last_sections`)。無いものは `None`。
    pub fn last_sections(&self) -> [Option<String>; 4] {
        let last = self.values.get("last_sections").and_then(Value::as_object);
        SectionKey::ALL.map(|k| {
            last.and_then(|l| l.get(k.state_key()))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    }

    /// 日次ロールオーバーを最後に実行した日 (`last_opened`、`YYYY-MM-DD`)。
    pub fn last_opened(&self) -> Option<&str> {
        self.values.get("last_opened").and_then(Value::as_str)
    }

    pub fn set_last_opened(&mut self, date: NaiveDate) {
        self.values
            .insert("last_opened".into(), Value::String(date.format("%Y-%m-%d").to_string()));
    }

    /// 書き戻す。読んだ後に他のプロセスが書き換えていたら、何も書かずにエラーを返す。
    pub fn write(&mut self, path: &Path) -> Result<(), StateError> {
        // Lua 版 (vim.json.encode) と同じく、改行や字下げの無い 1 行で書く。
        let data = serde_json::to_vec(&self.values).expect("a JSON object always serializes");
        self.stamp = Some(atomic::write_bytes(path, &data, self.stamp.as_ref())?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".state.json");
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn reads_names_written_by_neovim() {
        let (_d, path) = write(r#"{"last_opened":"2026-01-31","last_sections":{"TODAY":"今日","NEXT":"Next"}}"#);
        let state = State::read(&path).unwrap();
        let last = state.last_sections();
        assert_eq!(last[0].as_deref(), Some("今日"));
        assert_eq!(last[1].as_deref(), Some("Next"));
        assert_eq!(last[2], None);
        assert_eq!(state.last_opened(), Some("2026-01-31"));
    }

    #[test]
    fn missing_or_empty_file_or_key_means_no_values() {
        let dir = tempfile::tempdir().unwrap();
        let state = State::read(&dir.path().join("none")).unwrap();
        assert_eq!(state.last_sections(), <[Option<String>; 4]>::default());
        assert_eq!(state.last_opened(), None);
        for content in ["", "  \n", "[]", r#"{"last_opened":"2026-01-31"}"#] {
            let (_d, path) = write(content);
            assert_eq!(
                State::read(&path).unwrap().last_sections(),
                <[Option<String>; 4]>::default(),
                "{content:?}"
            );
        }
    }

    #[test]
    fn broken_file_is_reported() {
        for content in ["{", "[1]", "42"] {
            let (_d, path) = write(content);
            assert!(
                matches!(State::read(&path), Err(StateError::Invalid { .. })),
                "{content:?}"
            );
        }
    }

    #[test]
    fn invalid_utf8_is_reported_as_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".state.json");
        fs::write(&path, b"{\"last_opened\":\"\xff\"}").unwrap();
        assert!(matches!(State::read(&path), Err(StateError::Invalid { .. })));
    }

    #[test]
    fn writing_keeps_keys_it_does_not_know() {
        let (_d, path) = write(
            r#"{"last_opened":"2026-01-30","last_sections":{"TODAY":"今日"},"last_notify_time":1769800000,"future":{"x":[1,2]}}"#,
        );
        let mut state = State::read(&path).unwrap();
        state.set_last_opened(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        state.write(&path).unwrap();

        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "last_opened": "2026-01-31",
                "last_sections": {"TODAY": "今日"},
                "last_notify_time": 1769800000,
                "future": {"x": [1, 2]},
            })
        );
    }

    #[test]
    fn writing_creates_the_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".state.json");
        let mut state = State::read(&path).unwrap();
        state.set_last_opened(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        state.write(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"last_opened":"2026-01-31"}"#);
    }

    #[test]
    fn a_state_changed_by_neovim_after_reading_is_not_overwritten() {
        let (_d, path) = write(r#"{"last_opened":"2026-01-30"}"#);
        let mut state = State::read(&path).unwrap();
        fs::write(&path, r#"{"last_opened":"2026-01-31","last_notify_time":1}"#).unwrap();
        state.set_last_opened(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert!(matches!(
            state.write(&path),
            Err(StateError::Write(WriteError::Changed { .. }))
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"{"last_opened":"2026-01-31","last_notify_time":1}"#
        );
    }

    #[test]
    fn writing_twice_in_a_row_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".state.json");
        let mut state = State::read(&path).unwrap();
        state.set_last_opened(NaiveDate::from_ymd_opt(2026, 1, 30).unwrap());
        state.write(&path).unwrap();
        state.set_last_opened(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        state.write(&path).unwrap();
        assert_eq!(State::read(&path).unwrap().last_opened(), Some("2026-01-31"));
    }
}
