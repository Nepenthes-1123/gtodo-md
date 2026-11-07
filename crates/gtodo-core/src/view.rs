//! 読み取り専用のビュー。CLI と TUI で共有する。

use crate::document::{Document, TaskItem};
use crate::store::FileKind;
use crate::task::Task;

/// どのファイル・セクションにあるタスクか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub file: FileKind,
    /// preamble (最初の `##` より前) にあるタスクは `None`。
    pub section: Option<&'a str>,
    pub item: &'a TaskItem,
}

impl<'a> Entry<'a> {
    pub fn task(&self) -> &'a Task {
        &self.item.task
    }
}

/// 文書内のすべてのタスクをファイル内の順で列挙する。
pub fn entries(file: FileKind, doc: &Document) -> impl Iterator<Item = Entry<'_>> {
    doc.tasks().map(move |(section, item)| Entry { file, section, item })
}
