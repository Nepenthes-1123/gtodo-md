//! `done.md` / `cancelled.md` の `## YYYY-MM` 見出しへの追記 (Lua 版 `logic/history.lua`)。

use crate::lua_compat;
use crate::task::Task;

/// `lines` (履歴ファイルの行) の `## {section}` の末尾へタスクを入れる。見出しが無ければ末尾に作る。
///
/// 対象の見出しが最後とは限らない (完了月で振り分けるため、月をまたいで放置された完了タスクは
/// 過去の月へ入る) ので、見出しの位置を探し、次の見出しとの間の空行より前に入れる。
/// 末尾へ足すと、実際の完了月と違う見出しの下に紛れ込む。
pub fn insert_tasks(lines: &mut Vec<String>, title: &str, section: &str, tasks: &mut [Task]) {
    if lines.is_empty() {
        lines.push(format!("# {title}"));
        lines.push(String::new());
    }

    let heading = format!("## {section}");
    let mut insert_at = lines.len();
    match lines.iter().position(|l| *l == heading) {
        Some(start) => {
            if let Some(next) = (start + 1..lines.len()).find(|&i| lines[i].starts_with("## ")) {
                insert_at = next;
                while insert_at > start + 1 && lua_compat::trim(&lines[insert_at - 1]).is_empty() {
                    insert_at -= 1;
                }
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
            lines.push(heading);
            lines.push(String::new());
            insert_at = lines.len();
        }
    }

    let new: Vec<String> = tasks.iter_mut().map(Task::serialize).collect();
    lines.splice(insert_at..insert_at, new);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(lines: &[&str], section: &str, task: &str) -> Vec<String> {
        let mut lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        insert_tasks(&mut lines, "Done", section, &mut [Task::parse(task).unwrap()]);
        lines
    }

    #[test]
    fn an_empty_file_gets_the_title_and_the_heading() {
        assert_eq!(
            insert(&[], "2026-01", "- [x] A id:aaaaaa"),
            ["# Done", "", "## 2026-01", "", "- [x] A id:aaaaaa"]
        );
    }

    #[test]
    fn tasks_go_to_the_end_of_their_own_month_even_if_it_is_not_the_last() {
        assert_eq!(
            insert(
                &[
                    "# Done",
                    "",
                    "## 2026-01",
                    "",
                    "- [x] 1月 id:aaaaaa",
                    "",
                    "## 2026-02",
                    "",
                    "- [x] 2月 id:bbbbbb"
                ],
                "2026-01",
                "- [x] 追加 id:cccccc",
            ),
            [
                "# Done",
                "",
                "## 2026-01",
                "",
                "- [x] 1月 id:aaaaaa",
                "- [x] 追加 id:cccccc",
                "",
                "## 2026-02",
                "",
                "- [x] 2月 id:bbbbbb",
            ]
        );
    }

    #[test]
    fn a_missing_month_is_added_at_the_end() {
        assert_eq!(
            insert(
                &["# Done", "", "## 2026-01", "", "- [x] 1月 id:aaaaaa"],
                "2026-02",
                "- [x] 2月 id:bbbbbb"
            ),
            [
                "# Done",
                "",
                "## 2026-01",
                "",
                "- [x] 1月 id:aaaaaa",
                "",
                "## 2026-02",
                "",
                "- [x] 2月 id:bbbbbb"
            ]
        );
    }
}
