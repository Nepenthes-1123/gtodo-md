//! ゴールデンテストで共有する補助関数。

use std::collections::HashSet;

use chrono::NaiveDate;
use gtodo_core::task::Task;

/// 新しく発行された ID (入力に無い ID) は乱数なので、出現順の番号に置き換える。
/// どのタスクが元の ID を保ち、どのタスクが振り直されたかに加え、新しい ID どうしが
/// 重なっていないことも比べられる。置き換えるのはタスク行の ID だけ。
pub fn mask_new_ids(lines: &[String], known: &HashSet<String>, today: NaiveDate) -> Vec<String> {
    let mut issued: Vec<String> = Vec::new();
    lines
        .iter()
        .map(|line| {
            let Some(id) = Task::parse_at(line, today).and_then(|t| t.id) else {
                return line.clone();
            };
            if known.contains(&id) {
                return line.clone();
            }
            let n = issued.iter().position(|i| *i == id).unwrap_or_else(|| {
                issued.push(id.clone());
                issued.len() - 1
            });
            let rest = line.strip_suffix(&format!(" id:{id}")).unwrap();
            format!("{rest} id:NEW{n}")
        })
        .collect()
}
