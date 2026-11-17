//! 検索の書き方と一致の規則。CLI の `gtodo search` と TUI の検索で共有する。
//!
//! - 空白 (全角を含む) で区切った語をすべて含むタスクを出す (AND)。
//! - 本文の語は部分一致 (日本語には単語の区切りが無いため)。本文だけを見て、`created:` や
//!   `id:` などのタグは見ない (`2026` でほぼすべてが当たるため)。
//! - `+project`・`@context`・`wait:` はタグの値と比べる。確定した語は完全一致、打っている途中の
//!   最後の語 (TUI で、後ろに空白が無いもの) だけは前方一致 (打つたびに絞り込むため)。
//! - 大文字小文字と全角半角は区別しない (NFKC と小文字にそろえてから比べる)。ひらがなと
//!   カタカナは区別する (同じとみなすと意図しないものまで当たるため)。

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::{canonical_combining_class, decompose_compatible};

use crate::task::{Status, Task};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error("{0:?} has no name after it (write +project, @context or wait:who)")]
    EmptyTag(String),
}

/// 比べるための形 (NFKC と小文字) と、元の文字列への対応。
///
/// 元の文字列を「基底の文字と、それに続く結合する文字」のまとまりごとに正規化する。
/// 半角カナの濁点 (`ﾞ`) のように、正規化すると前の文字と合わさるものを分けないため。
/// まとまりごとに正規化しても、文字列全体を正規化したものとほぼ同じになる
/// (違うのは、結合文字を挟まずに合わさる、ハングルの字母などの並び)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folded {
    text: String,
    /// まとまりごとの (`text` での開始位置, 元の文字列での範囲)。
    spans: Vec<(usize, Range<usize>)>,
}

impl Folded {
    pub fn new(s: &str) -> Folded {
        let mut folded = Folded::default();
        let mut start = 0;
        for (i, c) in s.char_indices().skip(1) {
            if !combines(c) {
                folded.push(&s[start..i], start..i);
                start = i;
            }
        }
        if start < s.len() {
            folded.push(&s[start..], start..s.len());
        }
        folded
    }

    fn push(&mut self, segment: &str, range: Range<usize>) {
        self.spans.push((self.text.len(), range));
        self.text.extend(segment.nfkc().flat_map(char::to_lowercase));
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// `needle` (正規化したもの) が現れる元の文字列の範囲。重なる範囲はまとめる。
    fn find_all(&self, needle: &str) -> Vec<Range<usize>> {
        if needle.is_empty() {
            return Vec::new();
        }
        self.text
            .match_indices(needle)
            .map(|(at, m)| self.original(at)..self.original_end(at + m.len()))
            .collect()
    }

    /// `text` の位置 `at` を含むまとまりの、元の文字列での開始位置。
    fn original(&self, at: usize) -> usize {
        let i = self.spans.partition_point(|(start, _)| *start <= at).saturating_sub(1);
        self.spans.get(i).map_or(0, |(_, r)| r.start)
    }

    /// `text` の位置 `end` (排他) の直前の文字を含むまとまりの、元の文字列での終わり。
    fn original_end(&self, end: usize) -> usize {
        let i = self.spans.partition_point(|(start, _)| *start < end).saturating_sub(1);
        self.spans.get(i).map_or(0, |(_, r)| r.end)
    }
}

/// 前の文字と合わさる文字か (互換分解の先頭が結合文字)。
fn combines(c: char) -> bool {
    let mut first = None;
    decompose_compatible(c, |d| {
        first.get_or_insert(d);
    });
    first.is_some_and(|d| canonical_combining_class(d) != 0)
}

/// 正規化した文字列 (比べる相手の値用)。
fn fold(s: &str) -> String {
    Folded::new(s).text
}

/// タグの条件。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Term {
    /// 正規化した値 (`+`・`@`・`wait:` を除く)。
    value: String,
    /// 前方一致で比べるか (打っている途中の最後の語)。
    prefix: bool,
}

impl Term {
    /// `value` は正規化したもの。
    fn matches(&self, value: Option<&str>) -> bool {
        value.is_some_and(|v| {
            if self.prefix {
                v.starts_with(&self.value)
            } else {
                v == self.value
            }
        })
    }
}

/// 検索の条件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// 本文に含まれる語 (正規化したもの)。
    words: Vec<String>,
    projects: Vec<Term>,
    contexts: Vec<Term>,
    waits: Vec<Term>,
    pub status: Option<Status>,
}

impl Query {
    /// 書かれた検索語を読む。`live` は打っている途中の入力 (TUI) で、最後の語が空白で
    /// 終わっていなければタグを前方一致にし、値の無いタグ (`+` だけ) は無視する。
    /// そうでなければ (CLI)、値の無いタグはエラー。
    pub fn parse(input: &str, live: bool) -> Result<Query, SearchError> {
        let mut query = Query::default();
        let open_end = live && !input.ends_with(char::is_whitespace);
        let tokens: Vec<&str> = input.split(char::is_whitespace).filter(|t| !t.is_empty()).collect();
        for (i, token) in tokens.iter().enumerate() {
            let typing = open_end && i + 1 == tokens.len();
            let folded = fold(token);
            let (list, value) = if let Some(v) = folded.strip_prefix('+') {
                (&mut query.projects, v)
            } else if let Some(v) = folded.strip_prefix('@') {
                (&mut query.contexts, v)
            } else if let Some(v) = folded.strip_prefix("wait:") {
                (&mut query.waits, v)
            } else {
                query.words.push(folded);
                continue;
            };
            if value.is_empty() && !typing {
                if live {
                    continue;
                }
                return Err(SearchError::EmptyTag(token.to_string()));
            }
            list.push(Term {
                value: value.to_string(),
                prefix: typing,
            });
        }
        Ok(query)
    }

    /// 状態だけを指定した検索 (CLI の `--open`・`--done` だけのとき)。
    pub fn with_status(mut self, status: Option<Status>) -> Query {
        self.status = status;
        self
    }

    /// `task` が当たれば、どこが当たったかを返す。
    pub fn matches(&self, task: &Indexed) -> Option<Hit> {
        if self.status.is_some_and(|s| s != task.status) {
            return None;
        }
        let mut hit = Hit::default();
        for word in &self.words {
            let found = task.content.find_all(word);
            if found.is_empty() {
                return None;
            }
            hit.content.extend(found);
        }
        for (terms, value, flag) in [
            (&self.projects, task.project.as_deref(), &mut hit.project),
            (&self.contexts, task.context.as_deref(), &mut hit.context),
            (&self.waits, task.wait.as_deref(), &mut hit.wait),
        ] {
            if !terms.iter().all(|t| t.matches(value)) {
                return None;
            }
            *flag = !terms.is_empty();
        }
        hit.content.sort_by_key(|r| (r.start, r.end));
        hit.content.dedup_by(|b, a| {
            // 重なる範囲はまとめる (並べてあるので、後ろの方が始まりは同じか後)。
            if b.start <= a.end {
                a.end = a.end.max(b.end);
                true
            } else {
                false
            }
        });
        Some(hit)
    }
}

/// 検索のために正規化しておいたタスク。読み込んだときに 1 回だけ作る。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    content: Folded,
    /// タグの値は正規化したものを持つ (`@` は除く)。
    project: Option<String>,
    context: Option<String>,
    wait: Option<String>,
    status: Status,
}

impl Indexed {
    pub fn new(task: &Task) -> Indexed {
        let value = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(fold);
        Indexed {
            content: Folded::new(&task.content),
            project: value(&task.project),
            context: value(&task.context).map(|c| c.trim_start_matches('@').to_string()),
            wait: value(&task.wait),
            status: task.status,
        }
    }
}

/// 当たった場所。強調して見せるのに使う。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hit {
    /// 本文 (元の文字列) の中で当たった範囲。
    pub content: Vec<Range<usize>>,
    pub project: bool,
    pub context: bool,
    pub wait: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(line: &str) -> Indexed {
        Indexed::new(&Task::parse(line).unwrap())
    }

    fn live(input: &str) -> Query {
        Query::parse(input, true).unwrap()
    }

    fn hit(query: &str, line: &str) -> Option<Vec<String>> {
        let t = Task::parse(line).unwrap();
        live(query)
            .matches(&Indexed::new(&t))
            .map(|h| h.content.iter().map(|r| t.content[r.clone()].to_string()).collect())
    }

    #[test]
    fn words_match_parts_of_the_content_ignoring_case_and_width() {
        assert_eq!(hit("議事", "- [ ] 会議の議事録を書く"), Some(vec!["議事".into()]));
        assert_eq!(hit("readme", "- [ ] README を直す"), Some(vec!["README".into()]));
        assert_eq!(hit("ＡＢＣ", "- [ ] abcの件"), Some(vec!["abc".into()]));
        assert_eq!(hit("abc", "- [ ] ＡＢＣの件"), Some(vec!["ＡＢＣ".into()]));
        // 半角カナと濁点は、全角の 1 文字として当たり、元の 2 文字を強調する。
        assert_eq!(hit("ガス", "- [ ] ｶﾞｽ代を払う"), Some(vec!["ｶﾞｽ".into()]));
        // ひらがなとカタカナは区別する。
        assert_eq!(hit("がす", "- [ ] ガス代を払う"), None);
    }

    #[test]
    fn all_words_must_match_and_full_width_spaces_separate_them() {
        assert!(hit("会議　資料", "- [ ] 会議の資料を作る").is_some());
        assert_eq!(hit("会議 予算", "- [ ] 会議の資料を作る"), None);
        assert_eq!(
            hit("資料 会議", "- [ ] 会議の資料を作る"),
            Some(vec!["会議".into(), "資料".into()])
        );
    }

    #[test]
    fn tags_and_dates_in_the_line_are_not_searched_as_words() {
        assert_eq!(
            hit("2026", "- [ ] 本文 due:2026-01-02 created:2026-01-01 id:abcdef"),
            None
        );
        assert_eq!(hit("work", "- [ ] 本文 +work"), None);
    }

    #[test]
    fn tags_match_exactly_unless_still_being_typed() {
        let line = "- [ ] 本文 +Work @office wait:田中さん";
        assert!(hit("+work ", line).is_some(), "case is ignored");
        assert_eq!(hit("+wo ", line), None, "a finished tag must match exactly");
        assert!(hit("+wo", line).is_some(), "the tag being typed matches as a prefix");
        assert!(hit("@off", line).is_some());
        assert!(hit("wait:田中", line).is_some());
        assert_eq!(hit("wait:田中 ", line), None);
        assert!(
            hit("＋ｗｏｒｋ ＠ｏｆｆｉｃｅ", line).is_some(),
            "full-width tags fold too"
        );
        assert_eq!(hit("+home", line), None);
        assert_eq!(hit("+work", "- [ ] タグなし"), None);
        // 打ち始めの `+` だけでも、プロジェクトのあるタスクに絞る。
        assert!(hit("+", line).is_some());
        assert_eq!(hit("+", "- [ ] タグなし"), None);
        assert!(
            hit("+ ", "- [ ] タグなし").is_some(),
            "a finished empty tag is ignored while typing"
        );
    }

    #[test]
    fn the_cli_reads_every_term_as_finished() {
        let q = Query::parse("+wo", false).unwrap();
        assert!(q.matches(&task("- [ ] x +work")).is_none());
        assert!(
            Query::parse("+work", false)
                .unwrap()
                .matches(&task("- [ ] x +work"))
                .is_some()
        );
        assert_eq!(Query::parse("+", false), Err(SearchError::EmptyTag("+".into())));
        assert_eq!(Query::parse("wait:", false), Err(SearchError::EmptyTag("wait:".into())));
    }

    #[test]
    fn an_empty_query_matches_everything_and_status_filters() {
        let open = task("- [ ] 未完了");
        let done = task("- [x] 完了");
        assert!(live("").matches(&open).is_some());
        let q = live("").with_status(Some(Status::Done));
        assert!(q.matches(&open).is_none());
        assert!(q.matches(&done).is_some());
    }

    #[test]
    fn hits_name_the_matched_tags() {
        let t = task("- [ ] 本文 +work @office wait:bob");
        let h = live("本 +work @office wait:b").matches(&t).unwrap();
        assert_eq!((h.project, h.context, h.wait), (true, true, true));
        let h = live("本").matches(&t).unwrap();
        assert_eq!((h.project, h.context, h.wait), (false, false, false));
    }

    #[test]
    fn overlapping_and_repeated_matches_are_merged() {
        assert_eq!(hit("ab abc", "- [ ] xabcx"), Some(vec!["abc".into()]));
        assert_eq!(hit("a", "- [ ] aXa"), Some(vec!["a".into(), "a".into()]));
    }

    #[test]
    fn folding_keeps_the_way_back_to_the_original() {
        let f = Folded::new("ｱｲｳｴｵﾞ");
        assert_eq!(f.as_str(), "アイウエオ\u{3099}");
        let f = Folded::new("Ⅻ月");
        assert_eq!(f.as_str(), "xii月");
        assert_eq!(f.find_all("ii"), vec![Range { start: 0, end: 3 }]);
    }
}
