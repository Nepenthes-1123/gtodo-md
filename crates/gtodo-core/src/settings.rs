//! `data_dir/gtodo.json` (データの形と自動処理に関する設定) の読み取り。
//!
//! Neovim 版と CLI/TUI が同じ値を使わなければならない設定を、データと一緒に置く。
//! セクション名は見出しとしてファイルに書かれるため、データを別のマシンへ同期しても
//! 設定が付いていくようにする。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::sections::{SectionKey, SectionNames};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("failed to read {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not a valid settings file: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSettings {
    /// 現在のセクション名 (`.state.json` の前回の名前は含まない)。`sections` が無ければ `None`。
    pub sections: Option<SectionNames>,
    /// 期日が来た Inbox のタスクを Today へ移すか。
    pub auto_move_inbox_to_today: bool,
}

impl Default for DataSettings {
    fn default() -> Self {
        DataSettings {
            sections: None,
            auto_move_inbox_to_today: true,
        }
    }
}

/// `gtodo.json` を読む。ファイルが無ければ既定値を返す。
///
/// 未知のキーはエラーにせず `warn` に渡す。Neovim 版と CLI/TUI のバージョンがずれ、
/// 片方だけが知っている項目が増えても、もう片方が動き続けるようにするため。
pub fn read(path: &Path, warn: &mut impl FnMut(String)) -> Result<DataSettings, SettingsError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DataSettings::default()),
        Err(source) => {
            return Err(SettingsError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let invalid = |reason: String| SettingsError::Invalid {
        path: path.to_path_buf(),
        reason,
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    let Value::Object(top) = value else {
        return Err(invalid("top level is not an object".into()));
    };

    let mut settings = DataSettings::default();
    for (key, value) in &top {
        match key.as_str() {
            "sections" => {
                let Value::Object(sections) = value else {
                    return Err(invalid("\"sections\" is not an object".into()));
                };
                settings.sections = Some(section_names(sections, path, warn).map_err(invalid)?);
            }
            "auto_move_inbox_to_today" => {
                settings.auto_move_inbox_to_today = value
                    .as_bool()
                    .ok_or_else(|| invalid("\"auto_move_inbox_to_today\" is not a boolean".into()))?;
            }
            _ => warn(format!("{}: unknown key {key:?} is ignored", path.display())),
        }
    }
    Ok(settings)
}

fn section_names(
    sections: &Map<String, Value>,
    path: &Path,
    warn: &mut impl FnMut(String),
) -> Result<SectionNames, String> {
    for key in sections.keys() {
        if !SectionKey::ALL.iter().any(|k| k.settings_key() == key) {
            warn(format!("{}: unknown section {key:?} is ignored", path.display()));
        }
    }
    let mut current = SectionKey::ALL.map(|k| k.default_name().to_string());
    for key in SectionKey::ALL {
        match sections.get(key.settings_key()) {
            None => {}
            Some(Value::String(name)) => current[key as usize] = name.clone(),
            Some(_) => return Err(format!("section name for {:?} is not a string", key.settings_key())),
        }
    }
    SectionNames::new(current).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_str(content: &str) -> (Result<DataSettings, SettingsError>, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gtodo.json");
        fs::write(&path, content).unwrap();
        let mut warnings = Vec::new();
        let result = read(&path, &mut |w| warnings.push(w));
        (result, warnings)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = read(&dir.path().join("gtodo.json"), &mut |w| panic!("{w}")).unwrap();
        assert_eq!(settings, DataSettings::default());
        assert_eq!(settings.sections, None);
        assert!(settings.auto_move_inbox_to_today);
    }

    #[test]
    fn section_names_override_defaults_per_key() {
        let (settings, warnings) = read_str(r#"{"sections":{"today":"今日"},"auto_move_inbox_to_today":false}"#);
        let settings = settings.unwrap();
        let sections = settings.sections.unwrap();
        assert_eq!(sections.name(SectionKey::Today), "今日");
        assert_eq!(sections.name(SectionKey::Next), "Next");
        assert!(!settings.auto_move_inbox_to_today);
        assert!(warnings.is_empty());
    }

    #[test]
    fn unknown_keys_are_warned_and_ignored() {
        let (settings, warnings) = read_str(r#"{"future_option":1,"sections":{"tomorrow":"明日"}}"#);
        assert_eq!(settings.unwrap().sections, Some(SectionNames::default()));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("\"future_option\""));
        assert!(warnings[1].contains("\"tomorrow\""));
    }

    #[test]
    fn malformed_values_are_errors() {
        for content in [
            "",
            "{broken",
            "[]",
            r#"{"sections":[]}"#,
            r#"{"sections":{"today":1}}"#,
            r#"{"sections":{"today":"  "}}"#,
            r#"{"sections":{"today":"Next"}}"#,
            r#"{"auto_move_inbox_to_today":"yes"}"#,
        ] {
            let (result, _) = read_str(content);
            assert!(
                matches!(result, Err(SettingsError::Invalid { .. })),
                "{content:?}: {result:?}"
            );
        }
    }
}
