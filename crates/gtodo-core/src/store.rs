//! `data_dir` 配下のファイルの読み書き。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// gtodo-md のデータディレクトリ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

/// `data_dir` 直下の管理対象ファイル。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Inbox,
    Todo,
    Done,
    Cancelled,
}

impl FileKind {
    pub fn file_name(self) -> &'static str {
        match self {
            FileKind::Inbox => "inbox.md",
            FileKind::Todo => "todo.md",
            FileKind::Done => "done.md",
            FileKind::Cancelled => "cancelled.md",
        }
    }
}

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DataDir { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, kind: FileKind) -> PathBuf {
        self.root.join(kind.file_name())
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join(".state.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("gtodo.json")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("failed to read {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not valid UTF-8", path.display())]
    InvalidUtf8 { path: PathBuf },
}

/// ファイルを行のリストとして読む。ファイルが無ければ空のリストを返す
/// (Lua 版 `io.read_lines` と同じく、まだ作られていないファイルは空として扱う)。
///
/// 改行は `\n` で区切り、各行の末尾の `\r` を1つ取り除く。末尾の改行の後ろに空行は作らない。
pub fn read_lines(path: &Path) -> Result<Vec<String>, ReadError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ReadError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let text = String::from_utf8(bytes).map_err(|_| ReadError::InvalidUtf8 {
        path: path.to_path_buf(),
    })?;
    Ok(split_lines(&text))
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_matches_lua_file_lines() {
        assert_eq!(split_lines(""), Vec::<String>::new());
        assert_eq!(split_lines("\n"), [""]);
        assert_eq!(split_lines("a"), ["a"]);
        assert_eq!(split_lines("a\n"), ["a"]);
        assert_eq!(split_lines("a\n\n"), ["a", ""]);
        assert_eq!(split_lines("a\r\nb\r\n"), ["a", "b"]);
        assert_eq!(split_lines("a\rb\n"), ["a\rb"]);
        assert_eq!(split_lines("a\r"), ["a"]);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_lines(&dir.path().join("none.md")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn invalid_utf8_is_an_error_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, b"- [ ] \xff\n").unwrap();
        assert!(matches!(read_lines(&path), Err(ReadError::InvalidUtf8 { .. })));
    }
}
