//! プロジェクト画面の行 (core の `project::list` と進捗から作る)。

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use gtodo_core::project::{Progress, ProjectFile};

/// プロジェクト画面の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub tag: String,
    pub title: Option<String>,
    pub due: Option<String>,
    /// `projects/<tag>.md` があるか (無いものは、未完了のタスクに使われているタグ)。
    pub has_file: bool,
    pub progress: Progress,
}

/// プロジェクト画面の状態。
#[derive(Debug, Clone, Default)]
pub struct ProjectsView {
    /// アーカイブしたプロジェクトを出している (偽なら進行中)。
    pub archived: bool,
    pub rows: Vec<ProjectRow>,
    pub cursor: usize,
    pub offset: usize,
    /// 最後に読んだときの `projects/` の状態。変わったときだけ読み直すため。
    pub signature: Signature,
}

impl ProjectsView {
    pub fn selected(&self) -> Option<&ProjectRow> {
        self.rows.get(self.cursor)
    }

    pub fn select_tag(&mut self, tag: &str) -> bool {
        match self.rows.iter().position(|r| r.tag == tag) {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }
}

/// `projects/` の中身の状態 (名前・更新時刻・サイズ)。中身を読まずに変わったかを確かめるため。
pub type Signature = Vec<(String, Option<SystemTime>, u64)>;

/// `dir` の状態。無ければ空。
pub fn signature(dir: &Path) -> Signature {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Signature = entries
        .filter_map(Result::ok)
        .map(|e| {
            let meta = e.metadata().ok();
            (
                e.file_name().to_string_lossy().into_owned(),
                meta.as_ref().and_then(|m| m.modified().ok()),
                meta.map_or(0, |m| m.len()),
            )
        })
        .collect();
    items.sort();
    items
}

/// 行を作る。`files` は更新時刻の新しい順 (Neovim 版の一覧と同じ)。進行中の表示では、ファイルが無いが
/// 未完了のタスクに使われているタグを、後ろに名前順で足す (`E` で作れるように)。
pub fn rows(files: &[ProjectFile], progress: &HashMap<String, Progress>, archived: bool) -> Vec<ProjectRow> {
    let progress_of = |tag: &str| progress.get(tag).copied().unwrap_or_default();
    let mut rows: Vec<ProjectRow> = files
        .iter()
        .filter(|f| f.archived == archived)
        .map(|f| ProjectRow {
            tag: f.tag.clone(),
            title: f.title.clone(),
            due: f.due.clone(),
            has_file: true,
            progress: progress_of(&f.tag),
        })
        .collect();
    if !archived {
        let with_file: BTreeSet<&str> = files.iter().map(|f| f.tag.as_str()).collect();
        let mut without: Vec<&String> = progress
            .iter()
            .filter(|(tag, p)| p.open > 0 && !with_file.contains(tag.as_str()))
            .map(|(tag, _)| tag)
            .collect();
        without.sort();
        rows.extend(without.into_iter().map(|tag| ProjectRow {
            tag: tag.clone(),
            title: None,
            due: None,
            has_file: false,
            progress: progress_of(tag),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    fn file(tag: &str, archived: bool, age: u64) -> ProjectFile {
        ProjectFile {
            tag: tag.into(),
            path: PathBuf::from(format!("projects/{tag}.md")),
            archived,
            title: Some(format!("{tag} title")),
            created: None,
            due: None,
            members: None,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1000 - age),
        }
    }

    #[test]
    fn files_come_first_and_tags_without_files_follow_in_the_active_view() {
        let files = [file("new", false, 1), file("old", true, 5), file("mid", false, 3)];
        let progress = HashMap::from([
            ("new".to_string(), Progress { done: 1, open: 1 }),
            ("zeta".to_string(), Progress { done: 0, open: 2 }),
            ("alpha".to_string(), Progress { done: 0, open: 1 }),
            // 未完了のタスクが無いタグは、ファイルが無ければ出さない。
            ("finished".to_string(), Progress { done: 3, open: 0 }),
        ]);
        let active = rows(&files, &progress, false);
        let tags: Vec<(&str, bool)> = active.iter().map(|r| (r.tag.as_str(), r.has_file)).collect();
        assert_eq!(tags, [("new", true), ("mid", true), ("alpha", false), ("zeta", false)]);
        assert_eq!(active[0].progress, Progress { done: 1, open: 1 });
        assert_eq!(active[1].progress, Progress::default());
        let archived = rows(&files, &progress, true);
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].tag, "old");
    }
}
