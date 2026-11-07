//! Queue ビュー。見出しの文言は Neovim 版の Queue ビューに合わせている。

use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use gtodo_core::view::{Entry, QUEUE_FILES, due_queue, entries, wait_queue};

use crate::render::{KEY_WIDTH, key_of, tags};
use crate::workspace::Workspace;

const WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

pub fn run(ws: &Workspace, wait: bool, out: &mut String) -> Result<()> {
    let docs = QUEUE_FILES
        .iter()
        .map(|&f| ws.load(f).map(|doc| (f, doc)))
        .collect::<Result<Vec<_>>>()?;
    let all = || docs.iter().flat_map(|(f, doc)| entries(*f, doc));

    if wait {
        writeln!(out, "Queue (Wait) {}", ws.today)?;
        let groups = wait_queue(all());
        if groups.is_empty() {
            writeln!(out, "\n  誰かの作業を待っているタスクはありません")?;
        }
        for (person, es) in &groups {
            writeln!(out, "\n{person} 待ち")?;
            for e in es {
                line(out, e, "▶", "")?;
            }
        }
        return Ok(());
    }

    writeln!(out, "Queue (Due)  {}", ws.today)?;
    let q = due_queue(all(), ws.today);
    if q.is_empty() {
        writeln!(out, "\n  期限付きタスクはありません")?;
        return Ok(());
    }
    if !q.overdue.is_empty() {
        writeln!(out, "\n期限切れ")?;
        for (e, days) in &q.overdue {
            line(out, e, "⚠", &format!(" ({days}日超過)"))?;
        }
    }
    for (date, es) in &q.upcoming {
        writeln!(out, "\n{}", date_label(*date, ws.today))?;
        for e in es {
            line(out, e, "▶", "")?;
        }
    }
    if !q.invalid.is_empty() {
        writeln!(out, "\n期日を解釈できない")?;
        for e in &q.invalid {
            let due = e.task().due.as_deref().unwrap_or_default();
            line(out, e, "▶", &format!(" (due:{due})"))?;
        }
    }
    Ok(())
}

fn line(out: &mut String, e: &Entry, marker: &str, suffix: &str) -> Result<()> {
    let t = e.task();
    writeln!(
        out,
        "  {:<KEY_WIDTH$}  {marker} {}{}{suffix}",
        key_of(e),
        t.content,
        tags(t)
    )?;
    Ok(())
}

fn date_label(date: NaiveDate, today: NaiveDate) -> String {
    let md = format!(
        "{}/{} {}",
        date.month(),
        date.day(),
        WEEKDAYS[date.weekday().num_days_from_sunday() as usize]
    );
    match (date - today).num_days() {
        0 => format!("今日 ({md})"),
        1 => format!("明日 ({md})"),
        n => format!("{md} ({n}日後)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_labels_name_today_and_tomorrow_and_count_days_otherwise() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let label = |d| date_label(NaiveDate::from_ymd_opt(2026, 11, d).unwrap(), today);
        assert_eq!(label(7), "今日 (11/7 土)");
        assert_eq!(label(8), "明日 (11/8 日)");
        assert_eq!(label(20), "11/20 金 (13日後)");
    }
}
