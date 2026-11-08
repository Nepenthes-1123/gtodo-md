//! `data_dir` のファイルを読み、[`crate::logic`] の操作をして書き戻す (Lua 版 `logic/` の各ファイル操作)。
//!
//! ロックは取らない。Neovim 版の自動処理と重ならないよう、呼び出し元が
//! [`crate::lock::Lock`] を持ったまま呼ぶ。

use std::path::PathBuf;

use chrono::NaiveDate;

use crate::atomic::{self, WriteError};
use crate::document::Document;
use crate::logic::sort;
use crate::sections::SectionNames;
use crate::store::{self, DataDir, FileKind, ReadError, Stamp};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// todo.md の各セクションのタスクを並べ替える。
pub fn sort_todo(data_dir: &DataDir, names: &SectionNames, today: NaiveDate) -> Result<(), OpError> {
    let mut todo = Loaded::read(data_dir, FileKind::Todo, names, today)?;
    sort::sort_sections(&mut todo.doc);
    todo.write()?;
    Ok(())
}

/// 読み込んだファイルと、書き戻すために必要な読んだ時点の状態。
struct Loaded {
    path: PathBuf,
    doc: Document,
    stamp: Option<Stamp>,
    crlf: bool,
}

impl Loaded {
    fn read(data_dir: &DataDir, kind: FileKind, names: &SectionNames, today: NaiveDate) -> Result<Loaded, ReadError> {
        let path = data_dir.path(kind);
        let file = store::read_file(&path)?;
        Ok(Loaded {
            doc: Document::parse(&file.lines, names, today),
            path,
            stamp: file.stamp,
            crlf: file.crlf,
        })
    }

    fn write(&mut self) -> Result<(), WriteError> {
        let lines = self.doc.serialize();
        self.stamp = Some(atomic::write_lines(&self.path, &lines, self.crlf, self.stamp.as_ref())?);
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
    }

    #[test]
    fn nothing_is_written_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let todo = "# Todo\n\n## Today\n\n- [ ] A id:aaaaaa\n";
        fs::write(dir.path().join("todo.md"), todo).unwrap();
        let before = Stamp::of(&dir.path().join("todo.md")).unwrap();

        let data_dir = DataDir::new(dir.path());
        let names = SectionNames::default();
        sort_todo(&data_dir, &names, today()).unwrap();

        assert_eq!(Stamp::of(&dir.path().join("todo.md")).unwrap(), before);
        assert!(!dir.path().join("inbox.md").exists());
        assert!(!dir.path().join("done.md").exists());
    }
}
