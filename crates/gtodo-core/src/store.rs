//! `data_dir` 配下のファイルの読み書き。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// ファイルの状態 (更新時刻とサイズ)。読んだ時点のものを書き込む直前と比べ、
/// 他のプロセスによる変更を検出する (Lua 版の read stamp と同じ項目)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    /// ファイルが無ければ `None`。シンボリックリンクは実体の状態を返す。
    pub fn of(path: &Path) -> io::Result<Option<Stamp>> {
        match fs::metadata(path) {
            Ok(meta) => Stamp::from_metadata(&meta).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(crate) fn from_metadata(meta: &fs::Metadata) -> io::Result<Stamp> {
        Ok(Stamp {
            modified: meta.modified()?,
            len: meta.len(),
        })
    }
}

/// 読み込んだファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFile {
    pub lines: Vec<String>,
    /// 読む前に取った状態。ファイルが無ければ `None`。
    pub stamp: Option<Stamp>,
    /// 改行が CRLF か。書き戻すときに同じ改行を使う。
    pub crlf: bool,
}

/// ファイルを読む。ファイルが無ければ空として扱う
/// (Lua 版 `io.read_lines` と同じく、まだ作られていないファイルは空として扱う)。
///
/// 改行は `\n` で区切り、各行の末尾の `\r` を1つ取り除く。末尾の改行の後ろに空行は作らない。
pub fn read_file(path: &Path) -> Result<ReadFile, ReadError> {
    let io_err = |source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    };
    // 読む前に取る。読んでいる最中に書き換えられた場合も、書き込み前の照合で食い違いとして検出できる。
    let stamp = Stamp::of(path).map_err(io_err)?;
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(ReadFile {
                lines: Vec::new(),
                stamp: None,
                crlf: false,
            });
        }
        Err(source) => return Err(io_err(source)),
    };
    let crlf = is_crlf(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| ReadError::InvalidUtf8 {
        path: path.to_path_buf(),
    })?;
    Ok(ReadFile {
        lines: split_lines(&text),
        stamp,
        crlf,
    })
}

/// Lua 版 (`io.lua` の `detect_crlf`) と同じく、先頭 2048 バイトに `\r\n` があれば CRLF とみなす。
fn is_crlf(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(2048)].windows(2).any(|w| w == b"\r\n")
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
    fn missing_file_reads_as_empty_without_a_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let file = read_file(&dir.path().join("none.md")).unwrap();
        assert_eq!(file.lines, Vec::<String>::new());
        assert_eq!(file.stamp, None);
    }

    #[test]
    fn stamp_and_line_ending_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let file = read_file(&path).unwrap();
        assert_eq!(file.lines, ["a", "b"]);
        assert!(file.crlf);
        assert_eq!(file.stamp, Stamp::of(&path).unwrap());
        assert!(file.stamp.is_some());

        fs::write(&path, "a\nb\n").unwrap();
        assert!(!read_file(&path).unwrap().crlf);
    }

    #[test]
    fn crlf_is_detected_from_the_first_2048_bytes_like_lua() {
        let mut late = vec![b'a'; 2048];
        late.extend_from_slice(b"\r\n");
        assert!(!is_crlf(&late));
        assert!(is_crlf(b"# Todo\r\n"));
        assert!(!is_crlf(b"a\rb\n"));
    }

    #[test]
    fn invalid_utf8_is_an_error_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, b"- [ ] \xff\n").unwrap();
        assert!(matches!(read_file(&path), Err(ReadError::InvalidUtf8 { .. })));
    }
}
