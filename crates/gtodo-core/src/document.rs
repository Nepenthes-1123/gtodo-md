//! `inbox.md` / `todo.md` / `done.md` / `cancelled.md` のファイル構造。
//!
//! Lua 版 `io.parse_markdown` と `io.write_todo_file` の移植で、同じ入力に対して同じ構造・
//! 同じ書き出し結果になることを `testdata/golden/markdown_parse.json` (Lua 版から生成) で検証している。

use std::collections::HashSet;

use chrono::NaiveDate;

use crate::lua_compat;
use crate::sections::SectionNames;
use crate::task::Task;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub task: Task,
    /// 読み込んだときの行そのもの。ID が無いタスクを行の内容で同定するのに使う。
    pub line: String,
    /// 読み込んだときの行番号 (1 始まり)。表示用であり、同定には使わない。
    pub lnum: usize,
}

/// セクション内の1行。`### 見出し` もメモ行も [`Item::Text`] として元の位置に保持する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Task(Box<TaskItem>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// 最初の `##` 見出しまたは最初のタスクより前の行 (空行を含めてそのまま)。
    pub header: Vec<String>,
    /// ヘッダーの後、最初の `##` 見出しより前の行 (`inbox.md` のタスクはここに入る)。
    pub preamble: Vec<Item>,
    /// `##` 見出しごとのセクション。同名の見出しが複数あれば1つにまとめる。
    pub sections: Vec<Section>,
}

impl Document {
    /// 行のリストを解析する。セクション見出しは `names` の別名を現在の名前へ揃える。
    /// `today` はタスク行の相対的な期日の解決に使う。
    pub fn parse(lines: &[String], names: &SectionNames, today: NaiveDate) -> Document {
        let mut doc = Document::default();
        // Lua 版は preamble を "default" という名前のセクションとして持つため、`## default` という
        // 見出しがあると衝突して見出しが消える。ここでは preamble を別に持ち、衝突させない。
        let mut current: Option<usize> = None;
        let mut header_done = false;

        for (i, line) in lines.iter().enumerate() {
            if let Some(heading) = section_heading(line) {
                let name = names.canonicalize(heading);
                current = Some(match doc.sections.iter().position(|s| s.name == name) {
                    Some(idx) => idx,
                    None => {
                        doc.sections.push(Section {
                            name: name.to_string(),
                            items: Vec::new(),
                        });
                        doc.sections.len() - 1
                    }
                });
                header_done = true;
                continue;
            }

            let task = Task::parse_at(line, today);
            if task.is_some() {
                header_done = true;
            }
            if !header_done {
                doc.header.push(line.clone());
                continue;
            }

            let item = match task {
                Some(task) => Item::Task(Box::new(TaskItem {
                    task,
                    line: line.clone(),
                    lnum: i + 1,
                })),
                // 空行は書き戻すときに整形し直すため保持しない (Lua 版と同じ)。
                None if lua_compat::trim(line).is_empty() => continue,
                None => Item::Text(line.clone()),
            };
            match current {
                Some(idx) => doc.sections[idx].items.push(item),
                None => doc.preamble.push(item),
            }
        }
        doc
    }

    /// ファイルに書く行へ戻す。Lua 版 `io.write_todo_file` と同じ整形をする。
    ///
    /// - ID の無いタスクには発行する。ファイル内で既に使われた ID を持つタスク (コピー&ペーストで
    ///   複製されたもの) には振り直し、最初に現れた方が元の ID を保つ。どちらも `self` に書き戻す
    ///   ([`TaskItem::line`] は読み込んだときのまま)。
    /// - 見出しの前後に空行を入れ、連続する空行は1行にまとめ、末尾の空行は取り除く。
    ///
    /// 戻り値の各行の後ろに改行を付けて書けば ([`crate::atomic::write_lines`])、行がある限り
    /// ファイルは改行1つで終わる。
    ///
    /// [`Document::parse`] で得た構造なら、書き出した結果を読み直して書き出しても同じ行になる。
    /// 項目を動かした後の構造では、1 回目と 2 回目で行が変わりうる (Lua 版と同じ)。たとえば
    /// preamble の先頭に残ったメモ行は、次に読むときヘッダーに入る。
    pub fn serialize(&mut self) -> Vec<String> {
        let mut lines = self.header.clone();
        let mut seen_ids = HashSet::new();

        if !self.preamble.is_empty() {
            push_blank_separator(&mut lines);
            items_to_lines(&mut self.preamble, &mut lines, &mut seen_ids);
        }
        for section in &mut self.sections {
            push_blank_separator(&mut lines);
            lines.push(format!("## {}", section.name));
            lines.push(String::new());
            items_to_lines(&mut section.items, &mut lines, &mut seen_ids);
        }

        let mut lines = collapse_blank_runs(lines);
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines
    }

    /// すべてのタスクを、属するセクション名 (preamble なら `None`) とともにファイル内の順で返す。
    pub fn tasks(&self) -> impl Iterator<Item = (Option<&str>, &TaskItem)> {
        let preamble = self.preamble.iter().filter_map(|item| task_of(item).map(|t| (None, t)));
        let sections = self.sections.iter().flat_map(|s| {
            s.items
                .iter()
                .filter_map(move |item| task_of(item).map(|t| (Some(s.name.as_str()), t)))
        });
        let mut all: Vec<_> = preamble.chain(sections).collect();
        // 同名のセクションをまとめているため、セクション順に並べるとファイル内の順とずれうる。
        all.sort_by_key(|(_, t)| t.lnum);
        all.into_iter()
    }
}

fn task_of(item: &Item) -> Option<&TaskItem> {
    match item {
        Item::Task(t) => Some(t),
        Item::Text(_) => None,
    }
}

// Lua 版と同じく、空文字列かどうかだけを見る。直前が空白だけの行でも空行を足すが、
// collapse_blank_runs が1行にまとめる。
fn push_blank_separator(lines: &mut Vec<String>) {
    if lines.last().is_some_and(|l| !l.is_empty()) {
        lines.push(String::new());
    }
}

fn items_to_lines(items: &mut [Item], lines: &mut Vec<String>, seen_ids: &mut HashSet<String>) {
    for item in items {
        match item {
            Item::Task(t) => {
                if t.task.id.as_deref().is_some_and(|id| seen_ids.contains(id)) {
                    t.task.id = None;
                }
                lines.push(t.task.serialize());
                seen_ids.extend(t.task.id.clone());
            }
            Item::Text(text) if lua_compat::trim(text).is_empty() => {
                if lines.last().is_some_and(|l| !lua_compat::trim(l).is_empty()) {
                    lines.push(text.clone());
                }
            }
            Item::Text(text) => {
                // markdown の一般的な整形規約 (見出しは空行で囲む) に合わせるためで、構造としては扱わない。
                let heading = is_heading(text);
                if heading {
                    push_blank_separator(lines);
                }
                lines.push(text.clone());
                if heading {
                    lines.push(String::new());
                }
            }
        }
    }
}

/// Lua のパターン `^#+%s`。
fn is_heading(line: &str) -> bool {
    let rest = line.trim_start_matches('#');
    rest.len() < line.len() && rest.starts_with(lua_compat::is_space)
}

/// 連続する空行 (空白だけの行を含む) を1行にまとめる。ヘッダーは手で書かれた行をそのまま持つため、
/// ここでまとめないと連続した空行が書き戻すたびに残り続ける。
fn collapse_blank_runs(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let blank = lua_compat::trim(&line).is_empty();
        if !(blank && out.last().is_some_and(|l| lua_compat::trim(l).is_empty())) {
            out.push(line);
        }
    }
    out
}

/// `## 名前` の名前部分。`##` の直後に1つ以上の空白が必要 (`###` は見出しではなく本文)。
fn section_heading(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("##")?;
    if !rest.starts_with(lua_compat::is_space) {
        return None;
    }
    Some(lua_compat::trim(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Document {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        Document::parse(
            &lines,
            &SectionNames::default(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        )
    }

    /// 期待値は Lua 版 `write_todo_file` に同じ構造を渡して得たもの。空白だけのメモ行は
    /// `Document::parse` が作らないため、ゴールデンテストでは確かめられない。
    #[test]
    fn whitespace_only_text_items_are_written_like_lua() {
        let text = |s: &str| Item::Text(s.to_string());
        let task = |s: &str| {
            Item::Task(Box::new(TaskItem {
                task: Task::parse(s).unwrap(),
                line: s.to_string(),
                lnum: 0,
            }))
        };
        let mut doc = Document {
            header: vec!["# Inbox".into()],
            preamble: vec![text("  "), task("- [ ] A id:aaaaaa"), text("\t"), text("")],
            sections: vec![Section {
                name: "Today".into(),
                items: vec![
                    text(" "),
                    text("メモ"),
                    text(""),
                    text("  "),
                    text("### 見出し"),
                    text("\t"),
                ],
            }],
        };
        assert_eq!(
            doc.serialize(),
            [
                "# Inbox",
                "",
                "- [ ] A id:aaaaaa",
                "\t",
                "## Today",
                "",
                "メモ",
                "",
                "### 見出し"
            ]
        );

        // ファイルの先頭に空白だけの行は書かない (後の空行の圧縮では消えない位置)。
        let mut doc = Document {
            preamble: vec![text("  "), task("- [ ] A id:aaaaaa")],
            ..Document::default()
        };
        assert_eq!(doc.serialize(), ["- [ ] A id:aaaaaa"]);
    }

    #[test]
    fn issued_ids_are_kept_in_the_document() {
        let mut doc = parse(&[
            "## Today",
            "- [ ] ID の無いタスク",
            "- [ ] 複製 id:aaaaaa",
            "- [ ] 複製 id:aaaaaa",
        ]);
        let first = doc.serialize();
        assert_eq!(doc.serialize(), first);

        let ids: Vec<_> = doc.tasks().map(|(_, t)| t.task.id.clone().unwrap()).collect();
        assert_eq!(ids[1], "aaaaaa");
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 3, "{ids:?}");
    }

    #[test]
    fn writing_what_was_written_changes_nothing() {
        let mut doc = parse(&[
            "# Todo",
            "",
            "",
            "- [ ] 前",
            "## Today",
            "### 小見出し",
            "- [ ] タスク",
            "メモ",
            "## Next",
            "",
            "",
        ]);
        let first = doc.serialize();
        let refs: Vec<&str> = first.iter().map(String::as_str).collect();
        assert_eq!(parse(&refs).serialize(), first);
    }
}
