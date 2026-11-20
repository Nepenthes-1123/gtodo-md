//! テンプレート (`templates/<name>.md`)。Lua 版 `ui/template.lua` と同じ規則。
//!
//! テンプレートに書いたタスク行を、挿入するときに inbox.md へ足す。アーカイブは
//! `templates/archive/<name>.md` へ動かすこと。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::NaiveDate;

use crate::store::DataDir;
use crate::task::Task;

/// テンプレートを置くディレクトリ (データディレクトリからの相対)。
pub const DIR: &str = "templates";
/// アーカイブしたテンプレートを置くディレクトリ (`DIR` からの相対)。
pub const ARCHIVE_DIR: &str = "archive";

/// 挿入するときにタグ全体として埋める予約のプレースホルダーと、タグの頭の記号 (Lua 版と同じ)。
const RESERVED: [(&str, char); 2] = [("project", '+'), ("context", '@')];

/// テンプレートの名前として使えるか (Lua 版と同じく、英数字・`-`・`_` だけ)。
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 名前が使えないときのエラー。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid template name {0:?}: use letters, digits, - and _ only")]
pub struct NameError(pub String);

/// 名前を確かめる。
pub fn check_name(name: &str) -> Result<&str, NameError> {
    if valid_name(name) {
        Ok(name)
    } else {
        Err(NameError(name.to_string()))
    }
}

pub fn path(data_dir: &DataDir, name: &str) -> PathBuf {
    data_dir.root().join(DIR).join(format!("{name}.md"))
}

pub fn archived_path(data_dir: &DataDir, name: &str) -> PathBuf {
    data_dir.root().join(DIR).join(ARCHIVE_DIR).join(format!("{name}.md"))
}

/// 新しいテンプレートの中身 (Lua 版 `ensure_template_file` の記法メモと同じ。定期タスクの書き方の例は
/// コメントの中にあるので、フロントマターとしては読まれない)。
pub fn seed() -> String {
    [
        "<!--",
        "記法メモ: - [ ] タスク内容 +project @context due:YYYY-MM-DD wait:理由",
        "due: は due:+3d / due:today / due:mon のような相対指定も可能(挿入時に選ぶ基準日から解決されます)",
        "+project/@context の代わりに {{project}}/{{context}} と書くと、挿入時に値を尋ねられます(複数プロジェクトで使い回すテンプレート向け)",
        "優先度は内容の先頭に (A) のように付与 (例: - [ ] (A) 重要なタスク)",
        "このコメント行は挿入時に自動で無視されます",
        "",
        "定期タスクにするには、ファイルの先頭 (このコメントより前) に次のように書きます:",
        "  ---",
        "  type: recurring",
        "  repeat: mon, thu",
        "  values:",
        "    project: site",
        "  ---",
        "repeat には mon〜sun (曜日)、1〜31 (日。その月に無い日は月末)、end (月末)、",
        "2nd tue / last fri (何週目かの曜日)、04-01 (毎年の月日) を , で区切って並べます",
        "当たる日の日次の整理で inbox.md へ足し、due:+3d はその日から数えます",
        "values: には {{名前}} に入れる値を書きます (手で挿入するときは初期値になります)",
        "-->",
        "",
    ]
    .join("\n")
        + "\n"
}

/// `dir` の `*.md` の名前を、更新時刻の新しい順に返す (Lua 版 `list_md_basenames_by_mtime` と同じ)。
/// ディレクトリが無ければ空。
pub fn list_names(dir: &std::path::Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names: Vec<(SystemTime, String)> = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".md"))
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        names.push((modified, name.to_string()));
    }
    names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(names.into_iter().map(|(_, n)| n).collect())
}

/// 行の `{{name}}` (名前は英数字と `_`) を、現れた順に返す。
fn tokens(line: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut rest = 0;
    std::iter::from_fn(move || {
        loop {
            let start = rest + line[rest..].find("{{")?;
            let name_start = start + 2;
            let name_len = line[name_start..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            let end = name_start + name_len;
            if name_len > 0 && line[end..].starts_with("}}") {
                rest = end + 2;
                return Some((start, end + 2, &line[name_start..end]));
            }
            rest = start + 1;
        }
    })
}

/// タスク行にあるプレースホルダーの名前を、現れた順に重複なく返す (Lua 版 `list_placeholder_names` と同じ)。
/// タスク行でない行 (記法メモのコメントなど) は見ない。
pub fn placeholder_names(lines: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in lines.iter().filter(|l| Task::parse(l).is_some()) {
        for (_, _, name) in tokens(line) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// プレースホルダーを値で埋める (Lua 版 `resolve_placeholders` と同じ)。`{{project}}`・`{{context}}` は
/// 値があればタグ (`+value`・`@value`) にし、空 (または値が無い) ならタグごと消して前後の空白を 1 つにする。
/// それ以外は値をそのまま埋める (値が無ければ空)。
pub fn resolve_placeholders(line: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (start, end, name) in tokens(line) {
        let value = values.get(name).map(String::as_str).unwrap_or_default();
        match RESERVED.iter().find(|(n, _)| *n == name) {
            Some((_, prefix)) if !value.is_empty() => {
                out.push_str(&line[last..start]);
                out.push(*prefix);
                out.push_str(value);
            }
            Some(_) => {
                // 前後の空白ごと消し、空白 1 つにする (Lua 版の `%s*{{name}}%s*` → " ")。
                out.push_str(&line[last..start]);
                // 隣の空のプレースホルダーが置いた空白とも 1 つにまとめる (Lua 版は名前ごとに置き換えるため 1 つになる)。
                out.truncate(out.trim_end_matches(crate::lua_compat::is_space).len());
                out.push(' ');
                last = end + line[end..].len() - line[end..].trim_start_matches(crate::lua_compat::is_space).len();
                continue;
            }
            None => {
                out.push_str(&line[last..start]);
                out.push_str(value);
            }
        }
        last = end;
    }
    out.push_str(&line[last..]);
    out
}

/// プレースホルダーに入れる値が使えないとき。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{{{{{name}}}}}: {value:?} cannot be a +project or @context tag (it has spaces)")]
pub struct ValueError {
    pub name: String,
    pub value: String,
}

/// プレースホルダーに入れる値を整える。前後の空白を除き、`{{project}}`・`{{context}}` ではタグの頭の
/// `+`・`@` も除く (Lua 版と同じ)。タグに空白を含む値は、後ろが本文へ混ざって別の
/// タスクになるので断る。
pub fn check_value(name: &str, value: &str) -> Result<String, ValueError> {
    let value = crate::lua_compat::trim(value);
    let Some((_, prefix)) = RESERVED.iter().find(|(n, _)| *n == name) else {
        return Ok(value.to_string());
    };
    let value = value.strip_prefix(*prefix).unwrap_or(value);
    if value.contains(crate::lua_compat::is_space) {
        return Err(ValueError {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    Ok(value.to_string())
}

/// 挿入するときの基準日を読む (`today`・`+3d`・`fri`・`2026-07-02` など。実在する日付だけ)。
pub fn parse_base(text: &str, today: NaiveDate) -> Result<NaiveDate, crate::input::InputError> {
    let due = crate::input::parse_due(lua_trim(text), today)?;
    NaiveDate::parse_from_str(&due, "%Y-%m-%d").map_err(|_| crate::input::InputError::InvalidDue(text.to_string()))
}

fn lua_trim(text: &str) -> &str {
    crate::lua_compat::trim(text)
}

/// テンプレートの行から、挿入するタスクを作る (Lua 版 `extract_task_lines` と同じ)。
///
/// プレースホルダーを埋めてから、タスク行だけを読む。相対的な期日 (`due:+3d` など) は `base` (挿入するときに
/// 選んだ基準日) から決める。ID・完了の記録・作成日は引き継がない (ID は書くときに新しく付く。
/// 作成日は Lua 版と同じく付けない)。
pub fn tasks(lines: &[String], base: NaiveDate, values: &HashMap<String, String>) -> Vec<Task> {
    lines
        .iter()
        .filter_map(|line| Task::parse_at(&resolve_placeholders(line, values), base))
        .map(|mut task| {
            task.id = None;
            task.completed_at = None;
            task.done = None;
            task.cancelled = None;
            task.created = None;
            task
        })
        .collect()
}

/// 定期タスクの指定 (一覧に出す)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    /// 読めた。`next` は次に足す日 (アーカイブしたものの一覧では `None`)。
    Valid { repeat: String, next: Option<NaiveDate> },
    /// 定期タスクとして読めない (理由)。
    Invalid(String),
}

/// 一覧に出すテンプレートの情報 (CLI の `template list` と TUI のテンプレート画面で共有する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// 挿入されるタスクの数。
    pub tasks: usize,
    /// 挿入するときに尋ねるプレースホルダー。
    pub placeholders: Vec<String>,
    /// 定期タスクでなければ `None`。
    pub recurrence: Option<Recurrence>,
}

/// テンプレートの行を一覧のためにまとめる。`rollover` は次の日次の整理の日 (`recurring::next_rollover`)、
/// `recorded` は `.state.json` に記録された、どの日まで確かめたか。
pub fn summarize(lines: &[String], today: NaiveDate, rollover: NaiveDate, recorded: Option<&str>) -> Summary {
    let def = crate::recurring::read_definition(lines);
    let recurrence = match (&def.error, &def.rules) {
        (Some(error), _) => Some(Recurrence::Invalid(error.clone())),
        (None, Some(rules)) => {
            let recorded = recorded.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
            Some(Recurrence::Valid {
                repeat: def.repeat_text.clone().unwrap_or_default(),
                next: crate::recurring::next_creation(rules, recorded, rollover),
            })
        }
        (None, None) => None,
    };
    Summary {
        tasks: tasks(lines, today, &HashMap::new()).len(),
        placeholders: placeholder_names(lines),
        recurrence,
    }
}

/// フロントマターの `values:` に書いた値 (手で挿入するときの初期値。定期タスクはこの値で埋める)。
pub fn default_values(lines: &[String]) -> BTreeMap<String, String> {
    crate::recurring::read_definition(lines).values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn names_are_letters_digits_dashes_and_underscores() {
        assert!(valid_name("weekly-review_2"));
        for bad in ["", "a b", "../x", "日本語", "a.b"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn placeholders_are_listed_from_task_lines_only() {
        let t = lines(&format!(
            "{}- [ ] 準備 {{{{project}}}} {{{{who}}}}\n- [ ] 連絡 {{{{who}}}} {{{{context}}}}\n本文 {{{{note}}}}\n",
            seed()
        ));
        assert_eq!(placeholder_names(&t), ["project", "who", "context"]);
    }

    #[test]
    fn placeholders_are_filled_like_lua() {
        let line = "- [ ] 準備 {{project}} {{context}} for {{who}} {{x-y}}";
        assert_eq!(
            resolve_placeholders(
                line,
                &values(&[("project", "site"), ("context", "pc"), ("who", "田中")])
            ),
            "- [ ] 準備 +site @pc for 田中 {{x-y}}"
        );
        // 空の project・context はタグごと消し、空白を 1 つにする。
        assert_eq!(
            resolve_placeholders("- [ ] 準備  {{project}}  for {{who}}", &values(&[])),
            "- [ ] 準備 for "
        );
        assert_eq!(resolve_placeholders("- [ ] a {{context}}", &values(&[])), "- [ ] a ");
        assert_eq!(
            resolve_placeholders("- [ ] a {{project}} {{context}} b", &values(&[])),
            "- [ ] a b"
        );
    }

    #[test]
    fn summaries_show_the_repeat_and_the_next_day() {
        let t = lines("---\ntype: recurring\nrepeat: thu\nvalues:\n  who: 田中\n---\n- [ ] {{who}} に確認\n");
        let s = summarize(&t, date("2026-11-19"), date("2026-11-20"), Some("2026-11-19"));
        assert_eq!((s.tasks, s.placeholders), (1, vec!["who".to_string()]));
        assert_eq!(
            s.recurrence,
            Some(Recurrence::Valid {
                repeat: "thu".into(),
                next: Some(date("2026-11-26"))
            })
        );
        assert_eq!(default_values(&t).get("who").map(String::as_str), Some("田中"));
        let bad = lines("---\ntype: recurring\n---\n- [ ] a\n");
        assert!(matches!(
            summarize(&bad, date("2026-11-19"), date("2026-11-19"), None).recurrence,
            Some(Recurrence::Invalid(_))
        ));
        let today = date("2026-11-19");
        assert_eq!(summarize(&lines("- [ ] a\n"), today, today, None).recurrence, None);
        // 新しいテンプレートの記法メモにある例は、定期タスクとして読まれない。
        assert_eq!(summarize(&lines(&seed()), today, today, None).recurrence, None);
    }

    #[test]
    fn base_dates_are_real_dates() {
        let today = date("2026-11-16");
        assert_eq!(parse_base("today", today), Ok(today));
        assert_eq!(parse_base(" +3d ", today), Ok(date("2026-11-19")));
        assert_eq!(parse_base("2026-12-01", today), Ok(date("2026-12-01")));
        assert!(parse_base("someday", today).is_err());
        assert!(parse_base("2026-02-30", today).is_err());
    }

    #[test]
    fn reserved_values_must_be_single_tags() {
        assert_eq!(check_value("project", " +site "), Ok("site".into()));
        assert_eq!(check_value("context", "@pc"), Ok("pc".into()));
        assert_eq!(check_value("who", " 田中 さん "), Ok("田中 さん".into()));
        assert_eq!(check_value("project", ""), Ok(String::new()));
        assert!(check_value("project", "my site").is_err());
    }

    #[test]
    fn tasks_take_due_dates_from_the_base_date_and_drop_records() {
        let t = lines(&format!(
            "{}- [ ] (A) 資料 {{{{project}}}} due:+3d created:2026-01-01 id:aaaaaa\n- [x] 済み completed_at:2026-01-02\n\n## メモ\n",
            seed()
        ));
        let tasks = tasks(&t, date("2026-11-20"), &values(&[("project", "site")]));
        let lines: Vec<String> = tasks.iter().map(Task::to_line_without_id).collect();
        assert_eq!(lines, ["- [ ] (A) 資料 +site due:2026-11-23", "- [x] 済み"]);
        assert!(tasks.iter().all(|t| t.id.is_none()));
    }
}
