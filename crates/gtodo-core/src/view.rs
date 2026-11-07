//! 読み取り専用のビュー。CLI と TUI で共有する。

use crate::document::{Document, TaskItem};
use crate::store::FileKind;
use crate::task::{Status, Task};

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

/// 検索の対象ファイル (Lua 版と同じく、キャンセル済みは含めない)。
pub const SEARCH_FILES: [FileKind; 3] = [FileKind::Inbox, FileKind::Todo, FileKind::Done];

/// AND 条件の検索。条件の無い項目は絞り込まない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// `+project` の `project` 部分。
    pub projects: Vec<String>,
    /// `@context` の `context` 部分 (`@` を含まない)。
    pub contexts: Vec<String>,
    pub status: Option<Status>,
}

impl SearchQuery {
    pub fn matches(&self, task: &Task) -> bool {
        self.projects.iter().all(|p| task.project.as_deref() == Some(p))
            && self.contexts.iter().all(|c| task.context.as_deref() == Some(c))
            && self.status.is_none_or(|s| task.status == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_requires_all_conditions() {
        let t = |line: &str| Task::parse(line).unwrap();
        let q = SearchQuery {
            projects: vec!["work".into()],
            contexts: vec!["office".into()],
            status: Some(Status::Open),
        };
        assert!(q.matches(&t("- [ ] a +work @office")));
        assert!(!q.matches(&t("- [x] a +work @office")));
        assert!(!q.matches(&t("- [ ] a +work @home")));
        assert!(!q.matches(&t("- [ ] a @office")));
        assert!(SearchQuery::default().matches(&t("- [x] 条件なしは全件")));
    }
}
