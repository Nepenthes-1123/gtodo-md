//! 期日 (`due:`) の表記を `YYYY-MM-DD` へ正規化する。

use chrono::{Datelike, Days, Months, NaiveDate};

use crate::lua_compat;

/// 期日の表記を `YYYY-MM-DD` へ正規化する。解釈できなければ `None`。
///
/// 受け付ける表記は Lua 版 `utils.parse_due_date` と同じ:
///
/// - `today` / `t`、`tomorrow` / `tm`
/// - `+N` / `+Nd` / `+Nw` / `+Nm` / `+Ny` (`m`/`y` は暦算で、存在しない日は月末に丸める)
/// - 曜日 (先頭3文字が `sun`〜`sat`。当日と同じ曜日なら翌週)
/// - `YYYY-MM-DD` / `YYYY/M/D` / `YYYYMMDD` / `M-D` / `M/D` (年は `today` の年)
///
/// 日付の形をした表記は範囲を検証せずに桁だけ揃える (`2026-13-45` はそのまま返る)。
pub fn parse_due_date(s: &str, today: NaiveDate) -> Option<String> {
    let s = lua_compat::trim(s).to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }

    match s.as_str() {
        "today" | "t" => return Some(format_date(today)),
        "tomorrow" | "tm" => return today.checked_add_days(Days::new(1)).map(format_date),
        _ => {}
    }

    if let Some(spec) = s.strip_prefix('+') {
        let (num, unit) = match spec.find(|c: char| !c.is_ascii_digit()) {
            Some(i) => spec.split_at(i),
            None => (spec, ""),
        };
        if !num.is_empty() && matches!(unit, "" | "d" | "w" | "m" | "y") {
            // Lua 版は数値の桁あふれを考慮していない。ここでは解釈できない値として扱い、
            // 呼び出し元に元の表記を残させる。
            let n: u32 = num.parse().ok()?;
            // Lua 版は「今の時刻 + N×86400 秒」で日付を進めるため、夏時間のある
            // タイムゾーンでは切り替わりの日に1日ずれうる。こちらは暦の日数で進める。
            return match unit {
                "" | "d" => today.checked_add_days(Days::new(n.into())),
                "w" => today.checked_add_days(Days::new(u64::from(n) * 7)),
                "m" => today.checked_add_months(Months::new(n)),
                "y" => n.checked_mul(12).and_then(|m| today.checked_add_months(Months::new(m))),
                _ => unreachable!(),
            }
            .map(format_date);
        }
    }

    if let Some(target) = s.get(..3).and_then(weekday_from_sunday) {
        let current = today.weekday().num_days_from_sunday();
        let diff = (target + 7 - current) % 7;
        let diff = if diff == 0 { 7 } else { diff };
        return today.checked_add_days(Days::new(diff.into())).map(format_date);
    }

    if let Some((y, m, d)) = split_ymd(&s) {
        return Some(format!("{y:04}-{m:02}-{d:02}"));
    }
    if let Some((m, d)) = split_md(&s) {
        return Some(format!("{:04}-{m:02}-{d:02}", today.year()));
    }

    None
}

fn format_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn weekday_from_sunday(prefix: &str) -> Option<u32> {
    ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
        .iter()
        .position(|w| *w == prefix)
        .map(|i| i as u32)
}

fn digits(s: &str, min: usize, max: usize) -> Option<u32> {
    if (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// `YYYY[-/]M[M][-/]D[D]` または `YYYYMMDD`。
fn split_ymd(s: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = s.split(['-', '/']).collect();
    if let [y, m, d] = parts.as_slice() {
        return Some((digits(y, 4, 4)?, digits(m, 1, 2)?, digits(d, 1, 2)?));
    }
    if s.len() == 8 {
        return Some((digits(&s[..4], 4, 4)?, digits(&s[4..6], 2, 2)?, digits(&s[6..], 2, 2)?));
    }
    None
}

/// `M[M][-/]D[D]`。
fn split_md(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split(['-', '/']).collect();
    if let [m, d] = parts.as_slice() {
        return Some((digits(m, 1, 2)?, digits(d, 1, 2)?));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn plus_one_year_from_leap_day_clamps_to_end_of_february() {
        assert_eq!(parse_due_date("+1y", date(2024, 2, 29)).as_deref(), Some("2025-02-28"));
    }

    #[test]
    fn plus_one_month_into_leap_february_clamps_to_29th() {
        assert_eq!(parse_due_date("+1m", date(2024, 1, 31)).as_deref(), Some("2024-02-29"));
    }

    #[test]
    fn relative_days_cross_year_boundary() {
        assert_eq!(parse_due_date("+2d", date(2026, 12, 31)).as_deref(), Some("2027-01-02"));
    }

    #[test]
    fn month_day_uses_year_of_today() {
        assert_eq!(parse_due_date("2/3", date(2030, 6, 1)).as_deref(), Some("2030-02-03"));
    }

    #[test]
    fn surrounding_whitespace_and_case_are_ignored() {
        assert_eq!(
            parse_due_date("  ToMoRRoW\t", date(2026, 1, 31)).as_deref(),
            Some("2026-02-01")
        );
    }

    #[test]
    fn unrecognized_or_overflowing_specs_are_rejected() {
        let today = date(2026, 1, 31);
        for spec in [
            "",
            " ",
            "someday",
            "+",
            "+3x",
            "+d",
            "2026-02",
            "2026-02-03-04",
            "1/2/3",
            "+99999999999d",
        ] {
            assert_eq!(parse_due_date(spec, today), None, "{spec:?}");
        }
    }
}
