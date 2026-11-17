//! タスクの分割 (Lua 版 `ui/split.lua`)。選んだタスクを、それを引き継いだ複数のタスクで置き換える。
//!
//! Lua 版は親の行の文字列から `key:value` と `#語` を拾ってサブタスクの行に足すが、ここではタスクの項目
//! (プロジェクト・コンテキスト・期日・待ち先・作成日・`from:`) だけを引き継ぐ。行の文字列から拾うと、
//! 本文中の `ratio:3` などもタグとして複製される (文法を知っているのは `task.rs` だけという決め事に合わせる)。

use chrono::NaiveDate;

use crate::task::Task;

/// サブタスクを書くファイルの中身 (TUI で `$EDITOR` に渡す)。カーソルを置く行も返す (1 始まり)。
///
/// Neovim 版は空のスクラッチバッファに `- [ ] ` を置く。ここでは何を分割しているかと書き方を、
/// 読むときに無視するコメントとして添える。
pub fn seed(parent: &Task) -> (String, usize) {
    let text = format!(
        "<!-- 分割するタスク: {} -->\n\
         <!-- 1 行に 1 つ \"- [ ] \" で始めて書く。書かなかった +project・@context・due:・wait: などは元のタスクから引き継ぐ。\n\
         タスク行を 1 つも書かずに保存すると分割しない。 -->\n\
         - [ ] \n",
        parent.content
    );
    (text, 4)
}

/// サブタスクを書いたファイルを読む。空行と `<!-- -->` のコメントは飛ばし、本文の無い `- [ ]` (書かなかった行) も
/// 飛ばす。タスク行として読めない行があれば、その説明を返す。
pub fn read_subtasks(lines: &[String], today: NaiveDate) -> Result<Vec<Task>, Vec<String>> {
    let mut tasks = Vec::new();
    let mut problems = Vec::new();
    let mut in_comment = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = crate::lua_compat::trim(line);
        if in_comment || trimmed.starts_with("<!--") {
            in_comment = !trimmed.contains("-->");
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        match Task::parse_at(line, today) {
            Some(task) if task.content.is_empty() && task.to_line_without_id().trim() == "- [ ]" => {}
            Some(task) => tasks.push(task),
            None => problems.push(format!(
                "line {} is not a task line (write it as - [ ] ...): {line}",
                i + 1
            )),
        }
    }
    if problems.is_empty() { Ok(tasks) } else { Err(problems) }
}

/// サブタスクに、書かれていない項目を元のタスクから引き継ぐ。`project` はサブタスクのプロジェクト
/// (分割で決めたもの。サブタスクに書いたプロジェクトが優先)。インデントは元のタスクに合わせる。
/// ID は引き継がない (書くときに新しく付ける)。優先度と完了の記録も引き継がない (Lua 版と同じ)。
pub fn inherit(parent: &Task, mut child: Task, project: Option<&str>) -> Task {
    child.indent = format!("{}{}", parent.indent, child.indent);
    child.id = None;
    if child.project.is_none() {
        child.project = project.map(str::to_string);
    }
    for (mine, theirs) in [
        (&mut child.context, &parent.context),
        (&mut child.due, &parent.due),
        (&mut child.wait, &parent.wait),
        (&mut child.created, &parent.created),
        (&mut child.from, &parent.from),
    ] {
        if mine.is_none() {
            mine.clone_from(theirs);
        }
    }
    child
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn task(line: &str) -> Task {
        Task::parse_at(line, date("2026-11-16")).unwrap()
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn subtasks_inherit_what_they_do_not_say() {
        let parent = task(
            "  - [ ] (A) 発表 ratio:3 +talk @office due:2026-12-01 wait:bob created:2026-11-01 from:mail id:aaaaaa",
        );
        let child = inherit(&parent, task("- [ ] 資料 @home"), Some("site"));
        assert_eq!(
            child.to_line_without_id(),
            "  - [ ] 資料 +site @home due:2026-12-01 created:2026-11-01 wait:bob from:mail"
        );
        // 優先度・ID・本文中の key:value は引き継がない。サブタスクに書いた値が優先。
        let child = inherit(&parent, task("- [ ] (B) 練習 +own due:2026-11-20"), None);
        assert_eq!(
            child.to_line_without_id(),
            "  - [ ] (B) 練習 +own @office due:2026-11-20 created:2026-11-01 wait:bob from:mail"
        );
        let child = inherit(&parent, task("- [ ] 予約"), None);
        assert_eq!(child.project, None, "the project can be removed");
        assert_eq!(child.id, None);
    }

    #[test]
    fn the_subtask_file_skips_comments_blank_and_unwritten_lines() {
        let (seed, line) = seed(&task("- [ ] 発表 id:aaaaaa"));
        let seeded = lines(&seed);
        assert_eq!(seeded[line - 1], "- [ ] ");
        assert_eq!(read_subtasks(&seeded, date("2026-11-16")), Ok(Vec::new()));
        let mut edited = seeded.clone();
        edited[line - 1] = "- [ ] 資料".into();
        edited.push(String::new());
        edited.push("  - [ ] 印刷 due:+1d".into());
        let tasks = read_subtasks(&edited, date("2026-11-16")).unwrap();
        let written: Vec<String> = tasks.iter().map(Task::to_line_without_id).collect();
        assert_eq!(written, ["- [ ] 資料", "  - [ ] 印刷 due:2026-11-17"]);
        edited.push("メモ".into());
        assert_eq!(
            read_subtasks(&edited, date("2026-11-16")),
            Err(vec![format!(
                "line {} is not a task line (write it as - [ ] ...): メモ",
                edited.len()
            )])
        );
    }
}
