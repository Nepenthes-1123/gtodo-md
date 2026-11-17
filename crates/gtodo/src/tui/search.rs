//! TUI の検索。書き方と一致の規則は core の `search` (CLI の `gtodo search` と同じ)。
//!
//! 正規化は読み込んだときに 1 回だけ行い ([`index`])、打つたびの絞り込みでは比べるだけにする。

use gtodo_core::document::Document;
use gtodo_core::key::task_key;
use gtodo_core::search::{Hit, Indexed, Query};
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::FileKind;
use gtodo_core::task::Status;
use gtodo_core::view::entries;

use super::app::Row;
use super::input::LineInput;

/// 検索の対象のタスク。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub row: Row,
    pub file: FileKind,
    /// todo.md の管理対象のセクションなら、そのキー。
    pub section: Option<SectionKey>,
    /// 右端に出す、どこにあるか (Inbox・セクション名・Done)。
    pub place: String,
    indexed: Indexed,
}

/// 並びの順 (一覧のタブの順。todo.md の他の見出しは Someday の後)。
fn group(file: FileKind, section: Option<SectionKey>) -> u8 {
    match (file, section) {
        (FileKind::Inbox, _) => 0,
        (FileKind::Todo, Some(SectionKey::Today)) => 1,
        (FileKind::Todo, Some(SectionKey::Next)) => 2,
        (FileKind::Todo, Some(SectionKey::Waiting)) => 3,
        (FileKind::Todo, Some(SectionKey::Someday)) => 4,
        (FileKind::Todo, None) => 5,
        _ => 6,
    }
}

/// `doc` のタスクを検索の対象にする。並びは一覧と同じ順で、Done は新しいものから。
pub fn index(file: FileKind, doc: &Document, names: &SectionNames) -> Vec<Entry> {
    let mut list: Vec<Entry> = entries(file, doc)
        .map(|e| {
            let section = match file {
                FileKind::Todo => e.section.and_then(|s| names.key_of(s)),
                _ => None,
            };
            let place = match (file, e.section) {
                (FileKind::Inbox, None) => "Inbox".to_string(),
                (FileKind::Done, _) => "Done".to_string(),
                (_, Some(name)) => name.to_string(),
                (f, None) => f.file_name().to_string(),
            };
            Entry {
                row: Row {
                    key: task_key(e.task(), &e.item.line),
                    task: e.task().clone(),
                },
                file,
                section,
                place,
                indexed: Indexed::new(e.task()),
            }
        })
        .collect();
    match file {
        // ファイルの末尾が新しい。
        FileKind::Done => list.reverse(),
        // セクションの順にそろえる (同じセクションの中はファイルの順のまま)。
        _ => list.sort_by_key(|e| group(e.file, e.section)),
    }
    list
}

/// 状態の絞り込み。`Tab` で「すべて → 未完了 → 完了」と回る。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFilter {
    /// 完了済みのタスクを見落として「見つからない」とならないよう、初めはすべて。
    #[default]
    All,
    Open,
    Done,
}

impl StatusFilter {
    pub fn next(self) -> StatusFilter {
        match self {
            StatusFilter::All => StatusFilter::Open,
            StatusFilter::Open => StatusFilter::Done,
            StatusFilter::Done => StatusFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StatusFilter::All => "すべて",
            StatusFilter::Open => "未完了",
            StatusFilter::Done => "完了",
        }
    }

    fn status(self) -> Option<Status> {
        match self {
            StatusFilter::All => None,
            StatusFilter::Open => Some(Status::Open),
            StatusFilter::Done => Some(Status::Done),
        }
    }
}

/// 検索の対象のファイル (この順に並べる)。
pub const FILES: [FileKind; 3] = [FileKind::Inbox, FileKind::Todo, FileKind::Done];

/// 当たったタスク。`part` は [`FILES`] の位置、`index` はそのファイルの中の位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    part: usize,
    index: usize,
    pub hit: Hit,
}

/// 検索の画面の状態。
#[derive(Debug, Clone, Default)]
pub struct SearchView {
    pub input: LineInput,
    /// 入力欄にいる (偽なら結果の中にいて、一覧と同じキーで操作する)。
    pub typing: bool,
    pub status: StatusFilter,
    /// 検索の対象。ファイルごとに持ち、読み直したファイルの分だけ入れ替える。
    parts: [Vec<Entry>; 3],
    pub results: Vec<Found>,
    pub cursor: usize,
    pub offset: usize,
    /// 対象を入れ替える前に選んでいたタスク (入れ替えると結果の位置が古くなるため、先に覚える)。
    keep: Option<String>,
    /// 最後に絞り込んだ条件 (当たった部分を強調するのに使う)。
    pub query: Query,
}

impl SearchView {
    pub fn entry(&self, found: &Found) -> &Entry {
        &self.parts[found.part][found.index]
    }

    pub fn selected(&self) -> Option<(&Entry, &Hit)> {
        self.results.get(self.cursor).map(|f| (self.entry(f), &f.hit))
    }

    /// 読み直したファイルの対象を入れ替える。絞り込み直すのは呼び出し元 ([`SearchView::run`])。
    pub fn set_part(&mut self, file: FileKind, entries: Vec<Entry>) {
        let Some(i) = FILES.iter().position(|f| *f == file) else {
            return;
        };
        if self.keep.is_none() {
            self.keep = self.selected().map(|(e, _)| e.row.key.clone());
        }
        self.parts[i] = entries;
        // 古い結果は入れ替えた対象の位置を指さないので捨てる。
        self.results.clear();
    }

    /// 検索語か状態か対象が変わったときに絞り込み直す。`keep` なら選んでいたタスクを
    /// 選んだまま (無ければ同じ位置)、そうでなければ先頭を選ぶ。
    pub fn run(&mut self, keep: bool) {
        let remembered = self.keep.take();
        let selected = keep
            .then(|| remembered.or_else(|| self.selected().map(|(e, _)| e.row.key.clone())))
            .flatten();
        // 打っている途中の入力として読むので、エラーにはならない。
        self.query = Query::parse(self.input.text(), true)
            .unwrap_or_default()
            .with_status(self.status.status());
        let query = &self.query;
        self.results = self
            .parts
            .iter()
            .enumerate()
            .flat_map(|(part, list)| list.iter().enumerate().map(move |(index, e)| (part, index, e)))
            .filter_map(|(part, index, e)| query.matches(&e.indexed).map(|hit| Found { part, index, hit }))
            .collect();
        let at = selected.and_then(|k| self.position(&k));
        self.cursor = at
            .unwrap_or(if keep { self.cursor } else { 0 })
            .min(self.results.len().saturating_sub(1));
        if !keep {
            self.offset = 0;
        }
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.results.iter().position(|f| self.entry(f).row.key == key)
    }

    pub fn select_key(&mut self, key: &str) -> bool {
        let found = self.position(key);
        if let Some(i) = found {
            self.cursor = i;
        }
        found.is_some()
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn doc(text: &str) -> Document {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        Document::parse(
            &lines,
            &SectionNames::default(),
            NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(),
        )
    }

    fn view(input: &str) -> SearchView {
        let names = SectionNames::default();
        let mut v = SearchView {
            input: LineInput::new(input),
            ..SearchView::default()
        };
        v.set_part(
            FileKind::Inbox,
            index(FileKind::Inbox, &doc("# Inbox\n\n- [ ] 受信の資料\n"), &names),
        );
        v.set_part(FileKind::Todo, index(
            FileKind::Todo,
            &doc("# Todo\n\n## 買い物\n\n- [ ] 資料用の紙\n\n## Someday\n\n- [ ] いつか資料\n\n## Today\n\n- [ ] 今日の資料 +work\n- [x] 済んだ資料\n\n## Next\n\n## Waiting\n"),
            &names,
        ));
        v.set_part(
            FileKind::Done,
            index(
                FileKind::Done,
                &doc("# Done\n\n## 2026-10\n\n- [x] 古い資料\n\n## 2026-11\n\n- [x] 新しい資料\n"),
                &names,
            ),
        );
        v.run(false);
        v
    }

    fn found(v: &SearchView) -> Vec<String> {
        v.results
            .iter()
            .map(|r| {
                let e = v.entry(r);
                format!("{} [{}]", e.row.task.content, e.place)
            })
            .collect()
    }

    #[test]
    fn results_follow_the_tab_order_and_done_is_newest_first() {
        assert_eq!(
            found(&view("資料")),
            [
                "受信の資料 [Inbox]",
                "今日の資料 [Today]",
                "済んだ資料 [Today]",
                "いつか資料 [Someday]",
                "資料用の紙 [買い物]",
                "新しい資料 [Done]",
                "古い資料 [Done]",
            ]
        );
    }

    #[test]
    fn the_status_filter_cycles_all_open_done() {
        let mut v = view("資料");
        v.status = v.status.next();
        assert_eq!(v.status, StatusFilter::Open);
        v.run(false);
        assert_eq!(found(&v).len(), 4);
        v.status = v.status.next();
        v.run(false);
        assert_eq!(
            found(&v),
            ["済んだ資料 [Today]", "新しい資料 [Done]", "古い資料 [Done]"]
        );
        assert_eq!(v.status.next(), StatusFilter::All);
    }

    #[test]
    fn an_empty_query_shows_everything_and_tags_narrow() {
        assert_eq!(found(&view("")).len(), 7);
        assert_eq!(found(&view("+wo")), ["今日の資料 [Today]"]);
    }

    #[test]
    fn the_selection_stays_on_the_same_task_when_entries_change() {
        let mut v = view("資料");
        v.cursor = 3;
        let key = v.selected().unwrap().0.row.key.clone();
        // inbox を読み直して受信の資料が無くなっても、選んでいたタスクを選んだまま。
        v.set_part(FileKind::Inbox, Vec::new());
        v.run(true);
        assert_eq!(v.selected().unwrap().0.row.key, key);
    }
}
