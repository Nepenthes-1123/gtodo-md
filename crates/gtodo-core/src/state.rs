//! `data_dir/.state.json` (Neovim 版と共有する状態) の読み取り。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::sections::SectionKey;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("failed to read {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not a valid state file: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
}

/// Neovim 版が前回の起動時に使っていたセクション名 (`last_sections`)。
///
/// ファイルが無い・空・キーが無い場合はすべて `None`。壊れている場合はエラーを返す
/// (Lua 版は黙って空として扱うが、それでは壊れていることに誰も気づけない)。
/// CLI はこの値を書き換えない。これは Neovim 版の設定変更を追跡するための記録だから。
pub fn read_last_sections(path: &Path) -> Result<[Option<String>; 4], StateError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(source) => {
            return Err(StateError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if text.trim().is_empty() {
        return Ok(Default::default());
    }
    let invalid = |reason: String| StateError::Invalid {
        path: path.to_path_buf(),
        reason,
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    let Value::Object(state) = value else {
        return Err(invalid("top level is not an object".into()));
    };
    let Some(last) = state.get("last_sections").and_then(Value::as_object) else {
        return Ok(Default::default());
    };
    Ok(SectionKey::ALL.map(|k| last.get(k.state_key()).and_then(Value::as_str).map(str::to_string)))
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
        let last = read_last_sections(&path).unwrap();
        assert_eq!(last[0].as_deref(), Some("今日"));
        assert_eq!(last[1].as_deref(), Some("Next"));
        assert_eq!(last[2], None);
    }

    #[test]
    fn missing_or_empty_file_or_key_means_no_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_last_sections(&dir.path().join("none")).unwrap(),
            <[Option<String>; 4]>::default()
        );
        for content in ["", "  \n", r#"{"last_opened":"2026-01-31"}"#] {
            let (_d, path) = write(content);
            assert_eq!(
                read_last_sections(&path).unwrap(),
                <[Option<String>; 4]>::default(),
                "{content:?}"
            );
        }
    }

    #[test]
    fn broken_file_is_reported() {
        for content in ["{", "[]", "42"] {
            let (_d, path) = write(content);
            assert!(
                matches!(read_last_sections(&path), Err(StateError::Invalid { .. })),
                "{content:?}"
            );
        }
    }
}
