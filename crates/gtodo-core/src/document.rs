//! `inbox.md` / `todo.md` / `done.md` / `cancelled.md` のファイル構造。
//!
//! Lua 版 `io.parse_markdown` の移植で、同じ入力に対して同じ構造になることを
//! `testdata/golden/markdown_parse.json` (Lua 版から生成) で検証している。

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

/// `## 名前` の名前部分。`##` の直後に1つ以上の空白が必要 (`###` は見出しではなく本文)。
fn section_heading(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("##")?;
    if !rest.starts_with(lua_compat::is_space) {
        return None;
    }
    Some(lua_compat::trim(rest))
}
