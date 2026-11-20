//! 定期タスクの定義 (テンプレートのフロントマター) の読み取りと、「その日に作るか」の判定。
//! Lua 版 `logic/recurring.lua` と同じ規則で、結果は `testdata/golden/recurring.json` で固定している。
//!
//! ```text
//! ---
//! type: recurring
//! repeat: mon, thu, 15, end, 2nd tue, last fri, 04-01
//! values:
//!   project: site
//! ---
//! ```
//!
//! `repeat` の項目はどれか 1 つに当たる日に作る (和集合)。大文字小文字は区別しない。
//! ファイルには触れない。テンプレートを読んで inbox.md へ足すのは `ops::create_recurring`。

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate, Weekday};

use crate::lua_compat::{is_space, trim};

/// 何も作られなかった期間をさかのぼって探す上限。どの規則も 1 年 (366 日) の間に必ず 1 度は当たる
/// (毎年の 02-29 も、閏年でない年は 2/28 に当たる) ので、それより前を見ても最後の回は変わらない。
const LOOKBACK_DAYS: u64 = 366;

/// 何週目か。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nth {
    /// 第 1〜第 4。
    Week(u32),
    /// 最終。
    Last,
}

/// `repeat` の 1 項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// その曜日 (`mon`)。
    Weekday(Weekday),
    /// 毎月その日 (`15`)。その日が無い月は月末。
    MonthDay(u32),
    /// 毎月の月末 (`end`)。
    MonthEnd,
    /// 毎月の何週目かの曜日 (`2nd tue`・`last fri`)。
    NthWeekday { nth: Nth, weekday: Weekday },
    /// 毎年その月日 (`04-01`)。その日が無い年 (閏年でない年の 02-29) は月末。
    YearDay { month: u32, day: u32 },
}

/// `repeat` を読めないとき。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepeatError {
    #[error("repeat is empty")]
    Empty,
    #[error("repeat {0:?} has an empty item")]
    EmptyItem(String),
    #[error("repeat: {0:?} is not a day (use mon..sun, 1..31, end, 1st..4th/last + a weekday, or MM-DD)")]
    Unknown(String),
}

fn weekday(text: &str) -> Option<Weekday> {
    Some(match text {
        "sun" => Weekday::Sun,
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        _ => return None,
    })
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if NaiveDate::from_ymd_opt(year, 2, 29).is_some() => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// 1〜2 桁の ASCII の数字 (Lua の `%d%d?`)。
fn small_number(text: &str) -> Option<u32> {
    (matches!(text.len(), 1 | 2) && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// 1 項目を読む (Lua 版 `parse_item` と同じ順で照合する)。
fn parse_item(item: &str) -> Option<Rule> {
    let text = item.to_ascii_lowercase();
    if let Some(w) = weekday(&text) {
        return Some(Rule::Weekday(w));
    }
    if text == "end" {
        return Some(Rule::MonthEnd);
    }
    if let Some(n) = small_number(&text) {
        return (1..=31).contains(&n).then_some(Rule::MonthDay(n));
    }
    // `^(%S+)%s+(%a+)$`: 空白でない語、空白、英字だけの語。
    if let Some(split) = text.find(is_space) {
        let (ord, rest) = text.split_at(split);
        let wd = rest.trim_start_matches(is_space);
        let nth = match ord {
            "1st" => Some(Nth::Week(1)),
            "2nd" => Some(Nth::Week(2)),
            "3rd" => Some(Nth::Week(3)),
            "4th" => Some(Nth::Week(4)),
            "last" => Some(Nth::Last),
            _ => None,
        };
        if let (Some(nth), Some(weekday)) = (nth, weekday(wd)) {
            return Some(Rule::NthWeekday { nth, weekday });
        }
    }
    if let Some((m, d)) = text.split_once('-')
        && let (Some(month), Some(day)) = (small_number(m), small_number(d))
        // 閏年にある日 (02-29) までは受け付ける。その月に無い日 (04-31 など) は書き間違いとみなす。
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(2000, month)
    {
        return Some(Rule::YearDay { month, day });
    }
    None
}

/// `repeat` の値を規則のリストにする。
pub fn parse_repeat(spec: &str) -> Result<Vec<Rule>, RepeatError> {
    if trim(spec).is_empty() {
        return Err(RepeatError::Empty);
    }
    spec.split(',')
        .map(|item| {
            let text = trim(item);
            if text.is_empty() {
                return Err(RepeatError::EmptyItem(spec.to_string()));
            }
            parse_item(text).ok_or_else(|| RepeatError::Unknown(text.to_string()))
        })
        .collect()
}

/// テンプレートのフロントマター。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    /// 行頭から書いた `key: value`。
    pub fields: BTreeMap<String, String>,
    /// `values:` の後の字下げした `name: value`。
    pub values: BTreeMap<String, String>,
}

/// 値を囲む引用符 (`"..."`・`'...'`) を外す (エスケープは解釈しない)。
fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// `name: value` (Lua の `([%w_]+)%s*:%s*(.-)%s*$`)。名前は英数字と `_`。
fn key_value(text: &str) -> Option<(&str, &str)> {
    let name_len = text
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    if name_len == 0 {
        return None;
    }
    let rest = text[name_len..].trim_start_matches(is_space).strip_prefix(':')?;
    Some((&text[..name_len], trim(rest)))
}

fn without_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// 先頭の行が `---` で、閉じる `---` があるときだけフロントマターとして読む (閉じていなければ無いものとする)。
/// 本文の最初の行の位置 (0 始まり) と一緒に返す。空行と `#` で始まる行は読み飛ばす。
pub fn parse_front_matter(lines: &[String]) -> (Option<FrontMatter>, usize) {
    let is_fence = |line: &String| trim(without_cr(line)) == "---";
    if !lines.first().is_some_and(is_fence) {
        return (None, 0);
    }
    let Some(close) = lines.iter().skip(1).position(is_fence).map(|i| i + 1) else {
        return (None, 0);
    };
    let mut fm = FrontMatter::default();
    let mut in_values = false;
    for line in &lines[1..close] {
        let line = without_cr(line);
        let skip = trim(line).is_empty() || line.trim_start_matches(is_space).starts_with('#');
        if skip {
            continue;
        }
        if line.starts_with(is_space) {
            if in_values && let Some((k, v)) = key_value(line.trim_start_matches(is_space)) {
                fm.values.insert(k.to_string(), unquote(v).to_string());
            }
        } else if let Some((k, v)) = key_value(line) {
            fm.fields.insert(k.to_string(), unquote(v).to_string());
            in_values = k == "values" && v.is_empty();
        } else {
            in_values = false;
        }
    }
    (Some(fm), close + 1)
}

/// テンプレートの定義。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Definition {
    /// 定期タスクか (`type: recurring`)。
    pub recurring: bool,
    /// 規則 (定期タスクとして読めたとき)。
    pub rules: Option<Vec<Rule>>,
    /// `repeat` に書かれた値。
    pub repeat_text: Option<String>,
    /// `values:` に書かれた値 (定期タスクでなくても、手で挿入するときの初期値に使う)。
    pub values: BTreeMap<String, String>,
    /// フロントマターの後の行。
    pub body: Vec<String>,
    /// 定期タスクとして読めない理由。
    pub error: Option<String>,
}

/// テンプレートの行から定義を読む。
pub fn read_definition(lines: &[String]) -> Definition {
    let (fm, body_start) = parse_front_matter(lines);
    let mut def = Definition {
        body: lines[body_start..].to_vec(),
        ..Definition::default()
    };
    let Some(fm) = fm else {
        return def;
    };
    def.values = fm.values;
    def.repeat_text = fm.fields.get("repeat").cloned();
    let repeat = def.repeat_text.as_deref().unwrap_or_default();
    match fm.fields.get("type").map(String::as_str).unwrap_or_default() {
        "" => {
            if !repeat.is_empty() {
                def.error = Some("repeat needs 'type: recurring'".into());
            }
        }
        "recurring" => {
            def.recurring = true;
            if repeat.is_empty() {
                def.error = Some("a recurring template needs 'repeat:'".into());
            } else {
                match parse_repeat(repeat) {
                    Ok(rules) => def.rules = Some(rules),
                    Err(e) => def.error = Some(e.to_string()),
                }
            }
        }
        other => def.error = Some(format!("unknown type {other:?} (only 'recurring' is supported)")),
    }
    def
}

fn rule_matches(rule: &Rule, date: NaiveDate) -> bool {
    let (y, m, d) = (date.year(), date.month(), date.day());
    let last = days_in_month(y, m);
    let wd = date.weekday();
    match *rule {
        Rule::Weekday(w) => wd == w,
        // その月に無い日 (11 月の 31 など) は月末に作る。
        Rule::MonthDay(day) => d == day || (day > last && d == last),
        Rule::MonthEnd => d == last,
        Rule::NthWeekday { nth, weekday } => {
            wd == weekday
                && match nth {
                    Nth::Last => d + 7 > last,
                    Nth::Week(n) => (d - 1) / 7 + 1 == n,
                }
        }
        Rule::YearDay { month, day } => m == month && (d == day || (day > last && d == last)),
    }
}

/// `date` が規則のどれかに当たるか。
pub fn occurs_on(rules: &[Rule], date: NaiveDate) -> bool {
    rules.iter().any(|r| rule_matches(r, date))
}

/// `after` より後で `today` までに当たる日のうち、最後の日。`after` が `None` (まだ確かめていない
/// テンプレート) なら、過去の回をさかのぼって作らないよう `today` の分だけを見る。
pub fn latest_occurrence(rules: &[Rule], after: Option<NaiveDate>, today: NaiveDate) -> Option<NaiveDate> {
    let lower = after.or_else(|| today.pred_opt())?;
    let first = lower
        .succ_opt()?
        .max(today.checked_sub_days(Days::new(LOOKBACK_DAYS - 1))?);
    let mut day = today;
    while day >= first {
        if occurs_on(rules, day) {
            return Some(day);
        }
        day = day.pred_opt()?;
    }
    None
}

/// `from` より後で、最初に当たる日 (一覧に「次に作る日」として出す)。
pub fn next_occurrence(rules: &[Rule], from: NaiveDate) -> Option<NaiveDate> {
    (1..=LOOKBACK_DAYS)
        .filter_map(|n| from.checked_add_days(Days::new(n)))
        .find(|d| occurs_on(rules, *d))
}

/// 次の日次の整理がいつか。`last_opened` (最後に整理した日) が今日なら明日、そうでなければ今日。
pub fn next_rollover(last_opened: Option<&str>, today: NaiveDate) -> NaiveDate {
    let ran_today = last_opened.is_some_and(|d| d == today.format("%Y-%m-%d").to_string());
    if ran_today {
        today.succ_opt().unwrap_or(today)
    } else {
        today
    }
}

/// 次に足す日 (一覧に出す)。`recorded` はどの日まで確かめたか、`rollover` は次の日次の整理の日
/// ([`next_rollover`])。その整理で足す回 (逃した回を含む) があればその整理の日、無ければその後に当たる
/// 最初の日 (以後は毎日整理される前提)。記録の無いテンプレートは、整理が初めて見る日の分から数える。
pub fn next_creation(rules: &[Rule], recorded: Option<NaiveDate>, rollover: NaiveDate) -> Option<NaiveDate> {
    let after = match recorded {
        Some(r) => r,
        None => rollover.pred_opt()?,
    };
    if latest_occurrence(rules, Some(after), rollover).is_some() {
        return Some(rollover);
    }
    next_occurrence(rules, after.max(rollover))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_kind_of_item_is_read() {
        assert_eq!(
            parse_repeat("Mon, 15 ,end, 2nd TUE, last fri, 04-01").unwrap(),
            [
                Rule::Weekday(Weekday::Mon),
                Rule::MonthDay(15),
                Rule::MonthEnd,
                Rule::NthWeekday {
                    nth: Nth::Week(2),
                    weekday: Weekday::Tue
                },
                Rule::NthWeekday {
                    nth: Nth::Last,
                    weekday: Weekday::Fri
                },
                Rule::YearDay { month: 4, day: 1 },
            ]
        );
        assert_eq!(parse_repeat(" "), Err(RepeatError::Empty));
        assert_eq!(parse_repeat("mon,"), Err(RepeatError::EmptyItem("mon,".into())));
        assert_eq!(parse_repeat("5th mon"), Err(RepeatError::Unknown("5th mon".into())));
    }

    #[test]
    fn a_day_the_month_does_not_have_falls_on_its_last_day() {
        let rules = parse_repeat("31").unwrap();
        assert!(occurs_on(&rules, date("2026-11-30")));
        assert!(!occurs_on(&rules, date("2026-11-29")));
        assert!(occurs_on(&parse_repeat("02-29").unwrap(), date("2026-02-28")));
        assert!(!occurs_on(&parse_repeat("02-29").unwrap(), date("2028-02-28")));
    }

    #[test]
    fn only_today_is_seen_before_the_first_check() {
        let rules = parse_repeat("mon, wed").unwrap();
        assert_eq!(
            latest_occurrence(&rules, None, date("2026-11-18")),
            Some(date("2026-11-18"))
        );
        assert_eq!(latest_occurrence(&rules, None, date("2026-11-19")), None);
        assert_eq!(
            latest_occurrence(&rules, Some(date("2026-11-10")), date("2026-11-19")),
            Some(date("2026-11-18"))
        );
    }

    #[test]
    fn the_next_creation_day_follows_the_next_rollover() {
        let rules = parse_repeat("thu").unwrap();
        let thu = date("2026-11-19");
        // 今日 (木) の整理がまだなら今日。済んでいれば (記録が無くても) 来週。
        assert_eq!(next_rollover(Some("2026-11-18"), thu), thu);
        assert_eq!(next_rollover(Some("2026-11-19"), thu), date("2026-11-20"));
        assert_eq!(next_creation(&rules, None, thu), Some(thu));
        assert_eq!(
            next_creation(&rules, None, date("2026-11-20")),
            Some(date("2026-11-26"))
        );
        assert_eq!(
            next_creation(&rules, Some(thu), date("2026-11-20")),
            Some(date("2026-11-26"))
        );
        // 逃した回は、次の整理で足す。
        assert_eq!(
            next_creation(&rules, Some(date("2026-11-12")), date("2026-11-21")),
            Some(date("2026-11-21"))
        );
        // 記録が先の日付 (時計が戻った) なら、その後の回。
        assert_eq!(
            next_creation(&rules, Some(date("2026-11-26")), thu),
            Some(date("2026-12-03"))
        );
    }

    #[test]
    fn front_matter_needs_both_fences() {
        let (fm, body) = parse_front_matter(&lines(&["---", "type: recurring", "- [ ] a"]));
        assert_eq!((fm, body), (None, 0));
        let def = read_definition(&lines(&["---", "type: recurring", "repeat: thu", "---", "- [ ] a"]));
        assert!(def.recurring && def.error.is_none());
        assert_eq!(def.body, ["- [ ] a"]);
    }
}
