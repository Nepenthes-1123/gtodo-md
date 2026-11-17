//! プロジェクトファイル (`projects/<tag>.md`)。Lua 版 `ui/project.lua` と `validate.lua` のプロジェクトの部分。
//!
//! ファイルの形 (フロントマターと本文の雛形)・進捗の数え方・アーカイブ (`status:` の書き換え) は
//! Lua 版と同じにし、どちらで作ったファイルもそのまま使えるようにする。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::NaiveDate;

use crate::document::Document;
use crate::lua_compat;
use crate::store::{DataDir, read_file};
use crate::task::{Status, Task};

/// プロジェクトファイルを置くディレクトリ (データディレクトリからの相対)。
pub const DIR: &str = "projects";

/// アーカイブしたプロジェクトの `status:` の値。
pub const ARCHIVED: &str = "archived";
/// 進行中に戻したときの `status:` の値 (Lua 版の復元と同じ)。
pub const ACTIVE: &str = "active";

/// フロントマターに無ければならない項目 (Lua 版の保存時の検証と同じ)。
pub const REQUIRED_KEYS: [&str; 6] = ["title", "tag", "created", "due", "status", "members"];

/// プロジェクトファイルのパス。`tag` は [`check_tag`] を通ったものであること。
pub fn path(data_dir: &DataDir, tag: &str) -> PathBuf {
    data_dir.root().join(DIR).join(format!("{tag}.md"))
}

/// 新しいプロジェクトファイルの中身 (Lua 版 `create_project_file` の雛形と同じ)。
pub fn template(tag: &str, today: NaiveDate) -> String {
    [
        "---".to_string(),
        "title:".to_string(),
        format!("tag: {tag}"),
        format!("created: {}", today.format("%Y-%m-%d")),
        "due:".to_string(),
        "status: active".to_string(),
        "members: []".to_string(),
        "---".to_string(),
        String::new(),
        "## Overview".to_string(),
        String::new(),
        "## Notes".to_string(),
        String::new(),
        "## Reference".to_string(),
        String::new(),
    ]
    .join("\n")
        + "\n"
}

/// タグがファイル名として使えないとき。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    #[error("a project name cannot be empty")]
    Empty,
    #[error("{tag:?} cannot be used as a project file name; use {suggestion:?} instead")]
    Unsafe { tag: String, suggestion: String },
    #[error("{0:?} cannot be used as a project file name")]
    Unusable(String),
}

/// タグがプロジェクトファイルの名前として使えるか確かめる。先頭の `+` は取り除いて返す。
///
/// 使えるのは、Lua 版の分割がプロジェクト名を直す規則 ([`sanitize_tag`]) で変わらない名前。
/// Lua 版は分割のときだけ黙って直し、新しいプロジェクトの入力では直さないため、`../x` のような名前で
/// データディレクトリの外にファイルを作れてしまう。ここではどの経路でも、直した名前を添えてエラーにする。
pub fn check_tag(tag: &str) -> Result<String, TagError> {
    let tag = lua_compat::trim(tag);
    let tag = tag.strip_prefix('+').unwrap_or(tag);
    if tag.is_empty() {
        return Err(TagError::Empty);
    }
    let fixed = sanitize_tag(tag);
    if fixed == tag {
        Ok(tag.to_string())
    } else if fixed.is_empty() {
        Err(TagError::Unusable(tag.to_string()))
    } else {
        Err(TagError::Unsafe {
            tag: tag.to_string(),
            suggestion: fixed,
        })
    }
}

/// ファイル名として使えるように直す (Lua 版 `ui/split.lua` の `sanitize_project_tag` と同じ規則)。
/// 前後の `.` を除き、ファイル名に使えない記号と空白を `-` にし、Windows の予約名には `_proj` を付け、
/// 80 文字までにする。直した結果が空になることがある。
pub fn sanitize_tag(tag: &str) -> String {
    let trimmed = tag.trim_start_matches('.').trim_end_matches('.');
    let mut result = String::new();
    let mut last_dash = false;
    for c in trimmed.chars() {
        // Lua の `%s` (ASCII の空白) と、Windows でファイル名に使えない記号。
        let dash = matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '-') || lua_compat::is_space(c);
        if dash {
            if !last_dash {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    let mut result = result.trim_matches('-').to_string();
    let base = result.split('.').next().unwrap_or_default();
    const RESERVED: [&str; 11] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3",
    ];
    if !base.is_empty() && RESERVED.contains(&base.to_ascii_uppercase().as_str()) {
        result.insert_str(base.len(), "_proj");
    }
    result.chars().take(80).collect()
}

/// フロントマターの終わりの `---` の位置 (0 始まり)。1 行目が `---` で、もう 1 つの `---` があるときだけ。
fn frontmatter_end(lines: &[String]) -> Option<usize> {
    if lines.first().map(String::as_str) != Some("---") {
        return None;
    }
    lines.iter().skip(1).position(|l| l == "---").map(|i| i + 1)
}

/// フロントマターの `key: value` (Lua 版の `^(%w+):%s*(.*)$` と同じく、キーは英数字だけ)。
/// 値の前の空白は除き、後ろはそのまま。フロントマターが無ければ空。
pub fn frontmatter(lines: &[String]) -> Vec<(String, String)> {
    let Some(end) = frontmatter_end(lines) else {
        return Vec::new();
    };
    lines[1..end]
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            (!key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| {
                (
                    key.to_string(),
                    value.trim_start_matches(lua_compat::is_space).to_string(),
                )
            })
        })
        .collect()
}

/// フロントマターの項目の値 (最初に現れたもの)。前後の空白は除く。
pub fn field(lines: &[String], key: &str) -> Option<String> {
    frontmatter(lines)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| lua_compat::trim(&v).to_string())
}

/// アーカイブしたプロジェクトか (Lua 版と同じく、`status:` の値が `archived` のときだけ。
/// 値の前の空白は除き、後ろは除かない)。`status:` の無いものは進行中として扱う。
pub fn is_archived(lines: &[String]) -> bool {
    frontmatter(lines)
        .into_iter()
        .find(|(k, _)| k == "status")
        .is_some_and(|(_, v)| v == ARCHIVED)
}

/// `status:` を書き換えられないとき。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusError {
    #[error("the frontmatter has no status: field")]
    NoStatusField,
}

/// フロントマターの `status:` を `status` にした行を返す (Lua 版 `set_project_status` と同じく、その行だけを
/// `status: <status>` に置き換える)。すでにその値なら `None`。
pub fn with_status(lines: &[String], status: &str) -> Result<Option<Vec<String>>, StatusError> {
    let end = frontmatter_end(lines).ok_or(StatusError::NoStatusField)?;
    let index = (1..end)
        .find(|&i| lines[i].strip_prefix("status:").is_some())
        .ok_or(StatusError::NoStatusField)?;
    let current = lines[index]["status:".len()..].trim_start_matches(lua_compat::is_space);
    if current == status {
        return Ok(None);
    }
    let mut lines = lines.to_vec();
    lines[index] = format!("status: {status}");
    Ok(Some(lines))
}

/// 手で編集したプロジェクトファイル (TUI の `$EDITOR`) を、Neovim 版の保存時の検証
/// (`validate.lua` の `project_frontmatter_issues`) と同じ規則で確かめ、問題の説明を返す。空なら書いてよい。
///
/// フロントマターの区切りと 6 つの項目があること、`tag` がファイル名と同じこと、開いたときの
/// `created` を変えていないこと。
pub fn hand_edit_problems(tag: &str, before: &[String], after: &[String]) -> Vec<String> {
    let original_created = field(before, "created");
    let fields = frontmatter(after);
    let has_frontmatter = frontmatter_end(after).is_some();
    let created_changed = fields
        .iter()
        .any(|(k, v)| k == "created" && original_created.as_deref().is_some_and(|o| lua_compat::trim(v) != o));
    let tag_matches = fields.iter().any(|(k, v)| k == "tag" && lua_compat::trim(v) == tag);
    let missing: Vec<&str> = REQUIRED_KEYS
        .into_iter()
        .filter(|key| !fields.iter().any(|(k, _)| k == key))
        .collect();
    if has_frontmatter && missing.is_empty() && tag_matches && !created_changed {
        return Vec::new();
    }
    let mut problems = Vec::new();
    if created_changed {
        problems.push(format!(
            "created cannot be changed (it was {})",
            original_created.unwrap_or_default()
        ));
    }
    if !tag_matches {
        problems.push(format!("tag must be the file name ({tag})"));
    }
    if !missing.is_empty() {
        problems.push(format!("required fields are missing: {}", missing.join(", ")));
    }
    if problems.is_empty() {
        problems.push("the frontmatter (between the --- lines) is broken".to_string());
    }
    problems
}

/// `projects/` にあるプロジェクトファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub tag: String,
    pub path: PathBuf,
    pub archived: bool,
    pub title: Option<String>,
    pub created: Option<String>,
    pub due: Option<String>,
    pub members: Option<String>,
    pub modified: SystemTime,
}

impl ProjectFile {
    /// 読んだ行から作る。
    pub fn from_lines(tag: &str, path: PathBuf, lines: &[String], modified: SystemTime) -> ProjectFile {
        let value = |key| field(lines, key).filter(|v| !v.is_empty());
        ProjectFile {
            tag: tag.to_string(),
            path,
            archived: is_archived(lines),
            title: value("title"),
            created: value("created"),
            due: value("due"),
            members: value("members"),
            modified,
        }
    }
}

/// `projects/*.md` を読む。Lua 版と同じく、更新時刻の新しい順。ディレクトリが無ければ空。
/// 読めないファイルは飛ばして、その理由を `warnings` に入れる。
pub fn list(data_dir: &DataDir, warnings: &mut Vec<String>) -> io::Result<Vec<ProjectFile>> {
    let dir = data_dir.root().join(DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut projects = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(tag) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".md"))
            .filter(|t| !t.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match read_file(&path) {
            Ok(file) => projects.push(ProjectFile::from_lines(&tag, path, &file.lines, modified)),
            Err(e) => warnings.push(e.to_string()),
        }
    }
    projects.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.tag.cmp(&b.tag)));
    Ok(projects)
}

/// プロジェクトの進捗 (タスクの件数)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub open: usize,
}

impl Progress {
    pub fn total(self) -> usize {
        self.done + self.open
    }

    /// 完了の割合 (切り捨て。Lua 版と同じ)。タスクが無ければ 0。
    pub fn percent(self) -> usize {
        (self.done * 100).checked_div(self.total()).unwrap_or(0)
    }

    /// Lua 版の進捗のバー (`[███░░░░░░░] 30% (3/10 done)`)。
    pub fn bar(self) -> String {
        let filled = self.percent() / 10;
        format!(
            "[{}{}] {}% ({}/{} done)",
            "█".repeat(filled),
            "░".repeat(10 - filled),
            self.percent(),
            self.done,
            self.total()
        )
    }
}

/// プロジェクトごとの進捗 (Lua 版と同じ数え方)。`active` は inbox.md と todo.md のタスク (完了済みは完了、
/// それ以外は未完了として数える)、`archive` は done.md のタスク (完了したものだけを数える。Lua 版も完了の行だけを
/// 拾う)。cancelled.md は渡さない (数えない)。
pub fn progress<'a>(
    active: impl IntoIterator<Item = &'a Task>,
    archive: impl IntoIterator<Item = &'a Task>,
) -> HashMap<String, Progress> {
    let mut map: HashMap<String, Progress> = HashMap::new();
    let mut count = |task: &Task, open_counts: bool| {
        let Some(project) = task.project.as_deref().filter(|p| !p.is_empty()) else {
            return;
        };
        match task.status {
            Status::Done => map.entry(project.to_string()).or_default().done += 1,
            Status::Open if open_counts => map.entry(project.to_string()).or_default().open += 1,
            Status::Open => {}
        }
    };
    active.into_iter().for_each(|t| count(t, true));
    archive.into_iter().for_each(|t| count(t, false));
    map
}

/// 文書のタスク ([`progress`] に渡すため)。
pub fn tasks(doc: &Document) -> impl Iterator<Item = &Task> {
    doc.tasks().map(|(_, item)| &item.task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sections::SectionNames;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn doc(text: &str) -> Document {
        Document::parse(&lines(text), &SectionNames::default(), date("2026-01-31"))
    }

    #[test]
    fn the_template_is_the_same_as_lua() {
        assert_eq!(
            template("site", date("2026-11-16")),
            "---\ntitle:\ntag: site\ncreated: 2026-11-16\ndue:\nstatus: active\nmembers: []\n---\n\n\
             ## Overview\n\n## Notes\n\n## Reference\n\n"
        );
        // 作った直後のファイルは、手で編集したときの検証に通る。
        let t = lines(&template("site", date("2026-11-16")));
        assert!(hand_edit_problems("site", &t, &t).is_empty());
    }

    #[test]
    fn tags_must_be_usable_as_file_names() {
        assert_eq!(check_tag(" +site-renewal "), Ok("site-renewal".into()));
        assert_eq!(check_tag("日本語.v2"), Ok("日本語.v2".into()));
        // `../etc` で外に作れないように (Lua 版の分割と同じく、前の `.` を除いて記号を `-` にする)。
        assert_eq!(
            check_tag("../etc"),
            Err(TagError::Unsafe {
                tag: "../etc".into(),
                suggestion: "etc".into()
            })
        );
        assert_eq!(
            check_tag("a/b c"),
            Err(TagError::Unsafe {
                tag: "a/b c".into(),
                suggestion: "a-b-c".into()
            })
        );
        assert_eq!(
            check_tag("con"),
            Err(TagError::Unsafe {
                tag: "con".into(),
                suggestion: "con_proj".into()
            })
        );
        assert_eq!(check_tag("+"), Err(TagError::Empty));
        assert_eq!(check_tag("..."), Err(TagError::Unusable("...".into())));
        assert_eq!(sanitize_tag(&"x".repeat(100)).len(), 80);
        assert_eq!(sanitize_tag("--a--b--"), "a-b");
        assert_eq!(sanitize_tag("lpt1.txt"), "lpt1_proj.txt");
    }

    #[test]
    fn frontmatter_fields_are_read_like_lua() {
        let l = lines("---\ntitle:  サイト改修 \ntag: site\ndue_date: x\nstatus:archived\n---\nstatus: active\n");
        assert_eq!(field(&l, "title").as_deref(), Some("サイト改修"));
        assert_eq!(field(&l, "due_date"), None, "keys are letters and digits only");
        assert!(is_archived(&l));
        assert!(
            !is_archived(&lines("---\nstatus: archived \n---\n")),
            "Lua keeps trailing spaces"
        );
        assert!(!is_archived(&lines("status: archived\n")), "outside the frontmatter");
    }

    #[test]
    fn the_status_line_alone_is_rewritten() {
        let l = lines("---\ntitle: a\nstatus: active\n---\nstatus: active\n");
        assert_eq!(
            with_status(&l, ARCHIVED).unwrap(),
            Some(lines("---\ntitle: a\nstatus: archived\n---\nstatus: active\n"))
        );
        assert_eq!(with_status(&l, ACTIVE).unwrap(), None);
        assert_eq!(
            with_status(&lines("---\ntitle: a\n---\nstatus: x\n"), ARCHIVED),
            Err(StatusError::NoStatusField)
        );
        assert_eq!(with_status(&lines("# x\n"), ARCHIVED), Err(StatusError::NoStatusField));
    }

    #[test]
    fn hand_edits_follow_the_neovim_save_checks() {
        let before = lines(&template("site", date("2026-11-16")));
        let problems = |after: &str| hand_edit_problems("site", &before, &lines(after));
        let ok = "---\ntitle: 改修\ntag: site\ncreated: 2026-11-16\ndue: 2027-01-01\nstatus: active\nmembers: [a]\n---\n本文\n";
        assert!(problems(ok).is_empty());
        assert_eq!(
            problems(&ok.replace("created: 2026-11-16", "created: 2026-01-01")),
            ["created cannot be changed (it was 2026-11-16)"]
        );
        assert_eq!(
            problems(&ok.replace("tag: site", "tag: other")),
            ["tag must be the file name (site)"]
        );
        assert_eq!(
            problems(&ok.replace("due: 2027-01-01\n", "").replace("members: [a]\n", "")),
            ["required fields are missing: due, members"]
        );
        assert_eq!(
            problems(&ok.replacen("---\n", "", 1)),
            [
                "tag must be the file name (site)",
                "required fields are missing: title, tag, created, due, status, members"
            ]
        );
        // 開いたときに created が無ければ、何を書いてもよい。
        let no_created = lines("---\ntitle:\ntag: site\n---\n");
        assert!(hand_edit_problems("site", &no_created, &lines(ok)).is_empty());
    }

    #[test]
    fn progress_counts_like_lua() {
        let inbox = doc("# Inbox\n- [ ] 受信 +site\n- [x] 受信済み +site\n");
        let todo = doc("# Todo\n## Today\n- [ ] 作業 +site\n- [ ] 他 +other\n## 買い物\n- [ ] 紙 +site\n");
        let done = doc("# Done\n## 2026-01\n- [x] 済み +site\n- [x] 済み2 +site\n- [ ] 残り +site\n");
        let map = progress(tasks(&inbox).chain(tasks(&todo)), tasks(&done));
        assert_eq!(map["site"], Progress { done: 3, open: 3 });
        assert_eq!(map["other"], Progress { done: 0, open: 1 });
        assert_eq!(map["site"].bar(), "[█████░░░░░] 50% (3/6 done)");
        assert_eq!(Progress { done: 1, open: 2 }.bar(), "[███░░░░░░░] 33% (1/3 done)");
        assert_eq!(Progress::default().bar(), "[░░░░░░░░░░] 0% (0/0 done)");
    }
}
