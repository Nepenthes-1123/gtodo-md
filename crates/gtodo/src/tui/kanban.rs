//! カンバン。列と中身は Neovim 版のカンバンと同じ (Someday・Next・Today・Waiting・Done)。
//!
//! 未完了のタスクは todo.md のセクションの列へ、完了したタスクは todo.md にあるもの
//! (日次の整理を待っているもの) も done.md のものもすべて Done の列へ集める。1 つのタスクは
//! いつもどれか 1 つの列にだけある。Inbox と Cancelled は出さない。

use gtodo_core::sections::SectionKey;
use gtodo_core::store::FileKind;
use gtodo_core::task::{Status, Task};
use unicode_width::UnicodeWidthChar;

use super::app::{Row, TABS, Tab, TabView};

/// セクションの列 (表示の順)。この後に Done の列が続く。
pub const SECTION_COLUMNS: [SectionKey; 4] = [
    SectionKey::Someday,
    SectionKey::Next,
    SectionKey::Today,
    SectionKey::Waiting,
];
pub const COLUMNS: usize = 5;
/// Done の列の位置。
pub const DONE: usize = 4;
/// 列の幅の最小 (これより狭くなるなら、入らない列を左右に送る)。
pub const MIN_COLUMN_WIDTH: usize = 20;
/// カードの本文の最大の行数。
const BODY_LINES: usize = 2;

/// カード。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub row: Row,
    pub file: FileKind,
    /// todo.md のセクションにあるなら、そのキー (Done の列の、日次の整理を待っているものも)。
    pub section: Option<SectionKey>,
}

/// カンバンの状態。
#[derive(Debug, Clone, Default)]
pub struct KanbanView {
    pub columns: [Vec<Card>; COLUMNS],
    /// 選んでいる列。
    pub focus: usize,
    /// 列ごとに選んでいるカード。
    pub cursors: [usize; COLUMNS],
    /// 列ごとの、表示している最初のカード。
    pub offsets: [usize; COLUMNS],
    /// 表示している最初の列 (入らない列を左右に送るとき)。
    pub first_column: usize,
}

impl KanbanView {
    pub fn selected(&self) -> Option<&Card> {
        self.columns[self.focus].get(self.cursors[self.focus])
    }

    /// 一覧のタブの行から列を作り直す。列ごとに、選んでいたカードがまだその列にあれば
    /// 選んだまま、無ければ同じ位置 (移したカードの次のカード) を選ぶ。
    pub fn rebuild(&mut self, tabs: &[TabView]) {
        let selected: Vec<Option<String>> = (0..COLUMNS)
            .map(|c| self.columns[c].get(self.cursors[c]).map(|card| card.row.key.clone()))
            .collect();
        self.columns = build(tabs);
        for (c, key) in selected.into_iter().enumerate() {
            let column = &self.columns[c];
            match key.and_then(|k| column.iter().position(|card| card.row.key == k)) {
                Some(i) => self.cursors[c] = i,
                None => self.cursors[c] = self.cursors[c].min(column.len().saturating_sub(1)),
            }
        }
    }

    /// 選んでいる列の中でキーのカードを選ぶ。他の列へ移ったカードは追わない
    /// (列を上から順に振り分けやすくするため)。
    pub fn select_key(&mut self, key: &str) -> bool {
        let found = self.columns[self.focus].iter().position(|c| c.row.key == key);
        if let Some(i) = found {
            self.cursors[self.focus] = i;
        }
        found.is_some()
    }
}

/// 列を作る。
fn build(tabs: &[TabView]) -> [Vec<Card>; COLUMNS] {
    let mut columns: [Vec<Card>; COLUMNS] = Default::default();
    let mut done = Vec::new();
    for (c, key) in SECTION_COLUMNS.iter().enumerate() {
        let Some(tab) = TABS.iter().position(|t| *t == Tab::Section(*key)) else {
            continue;
        };
        for row in &tabs[tab].rows {
            let card = Card {
                row: row.clone(),
                file: FileKind::Todo,
                section: Some(*key),
            };
            match row.task.status {
                Status::Open => columns[c].push(card),
                Status::Done => done.push(card),
            }
        }
    }
    if let Some(tab) = TABS.iter().position(|t| *t == Tab::Done) {
        done.extend(tabs[tab].rows.iter().map(|row| Card {
            row: row.clone(),
            file: FileKind::Done,
            section: None,
        }));
    }
    // 新しく完了したものから (Neovim 版と同じく `done:`、無ければ `completed_at:` で比べる)。
    // 同じ日のものは、todo.md の完了済み・done.md の新しいものの順のまま。
    // 比べるたびに文字列を作らない (done.md の数万件を並べるため)。安定な並べ替え。
    done.sort_by(|a, b| completed_date(&b.row.task).cmp(completed_date(&a.row.task)));
    columns[DONE] = done;
    columns
}

fn completed_date(task: &Task) -> &str {
    fn value(v: &Option<String>) -> Option<&str> {
        v.as_deref().filter(|s| !s.is_empty())
    }
    value(&task.done).or(value(&task.completed_at)).unwrap_or_default()
}

/// 画面の幅に入る列の数と、1 列の幅 (列の間は 1 文字空ける)。
pub fn layout(width: usize) -> (usize, usize) {
    let visible = ((width + 1) / (MIN_COLUMN_WIDTH + 1)).clamp(1, COLUMNS);
    let column_width = (width.saturating_sub(visible - 1) / visible).max(1);
    (visible, column_width)
}

/// 選んでいる列が見えるよう、表示する最初の列をずらす。
pub fn first_visible(focus: usize, visible: usize, current: usize) -> usize {
    let max = COLUMNS.saturating_sub(visible);
    let mut first = current.min(max);
    if focus < first {
        first = focus;
    } else if focus >= first + visible {
        first = focus + 1 - visible;
    }
    first.min(max)
}

/// カードの本文の行 (優先度と本文。`width` ごとに折り返し、`BODY_LINES` 行を超える分は `…`)。
pub fn body_lines(task: &Task, width: usize) -> Vec<String> {
    let mut text = String::new();
    if let Some(p) = task.priority {
        text.push_str(&format!("({p}) "));
    }
    text.push_str(&task.content);
    wrap(&text, width.max(1), BODY_LINES)
}

/// 表示幅 `width` ごとに折り返し、`max_lines` 行を超えるなら最後の行の末尾を `…` にする。
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            if lines.len() == max_lines {
                // 入りきらない。最後の行の末尾を `…` にする。
                let last = lines.last_mut().expect("never empty");
                while last.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() + 1 > width {
                    last.pop();
                }
                last.push('…');
                return lines;
            }
            lines.push(String::new());
            used = 0;
        }
        lines.last_mut().expect("never empty").push(c);
        used += w;
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    fn tabs(today: &[&str], done: &[&str]) -> Vec<TabView> {
        let mut tabs = vec![TabView::default(); TABS.len()];
        let row = |line: &str| {
            let task = Task::parse(line).unwrap();
            Row {
                key: task.id.clone().unwrap_or_default(),
                task: Rc::new(task),
            }
        };
        tabs[1].rows = today.iter().map(|l| row(l)).collect();
        tabs[5].rows = done.iter().map(|l| row(l)).collect();
        tabs
    }

    fn contents(column: &[Card]) -> Vec<&str> {
        column.iter().map(|c| c.row.task.content.as_str()).collect()
    }

    #[test]
    fn completed_tasks_go_to_done_newest_first() {
        let columns = build(&tabs(
            &[
                "- [ ] 今日 id:a00001",
                "- [x] 今日済み completed_at:2026-11-17 id:a00002",
            ],
            &[
                "- [x] 新しい completed_at:2026-11-10 id:d00002",
                "- [x] 古い completed_at:2026-10-01 id:d00001",
                "- [x] doneで記録 done:2026-11-12 completed_at:2026-01-01 id:d00003",
            ],
        ));
        assert_eq!(contents(&columns[2]), ["今日"]);
        assert_eq!(contents(&columns[DONE]), ["今日済み", "doneで記録", "新しい", "古い"]);
        assert_eq!(columns[DONE][0].file, FileKind::Todo);
        assert_eq!(columns[DONE][0].section, Some(SectionKey::Today));
        assert_eq!(columns[DONE][1].file, FileKind::Done);
    }

    #[test]
    fn the_cursor_stays_in_the_column_when_a_card_leaves() {
        let mut view = KanbanView::default();
        view.rebuild(&tabs(
            &["- [ ] a id:a00001", "- [ ] b id:a00002", "- [ ] c id:a00003"],
            &[],
        ));
        view.focus = 2;
        view.cursors[2] = 1;
        view.rebuild(&tabs(&["- [ ] a id:a00001", "- [ ] c id:a00003"], &[]));
        assert_eq!(view.selected().unwrap().row.task.content, "c");
        // 選んでいたカードが残っていれば、位置が変わっても選んだまま。
        view.rebuild(&tabs(
            &["- [ ] new id:a00009", "- [ ] a id:a00001", "- [ ] c id:a00003"],
            &[],
        ));
        assert_eq!(view.selected().unwrap().row.task.content, "c");
    }

    #[test]
    fn columns_are_at_least_twenty_wide_and_the_rest_scroll() {
        assert_eq!(layout(120), (5, 23));
        assert_eq!(layout(80), (3, 26));
        assert_eq!(layout(40), (1, 40));
        assert_eq!(layout(41), (2, 20));
        assert_eq!(layout(42), (2, 20));
        assert_eq!(layout(10), (1, 10));
        assert_eq!(first_visible(4, 3, 0), 2);
        assert_eq!(first_visible(1, 3, 2), 1);
        assert_eq!(first_visible(2, 3, 1), 1);
        assert_eq!(first_visible(0, 5, 3), 0);
    }

    #[test]
    fn bodies_wrap_to_two_lines() {
        let task = Task::parse("- [ ] (A) とても長いタスクの本文がここに続く").unwrap();
        assert_eq!(body_lines(&task, 12), ["(A) とても長", "いタスクの…"]);
        let short = Task::parse("- [ ] 短い").unwrap();
        assert_eq!(body_lines(&short, 12), ["短い"]);
        assert_eq!(wrap("abcdef", 3, 2), ["abc", "def"]);
        assert_eq!(wrap("abcdefg", 3, 2), ["abc", "de…"]);
    }
}
