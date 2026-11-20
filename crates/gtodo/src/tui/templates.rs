//! テンプレート画面の行 (core の `template` で `templates/*.md` を読んで作る)。

use std::collections::BTreeMap;
use std::path::Path;

use chrono::NaiveDate;
use gtodo_core::store;
use gtodo_core::template::{self, Recurrence};

use super::projects::Signature;

/// テンプレート画面の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRow {
    pub name: String,
    /// 挿入されるタスクの数。
    pub tasks: usize,
    /// 挿入するときに尋ねるプレースホルダー。
    pub placeholders: Vec<String>,
    /// 定期タスクの指定 (定期タスクでなければ `None`)。
    pub recurrence: Option<Recurrence>,
}

/// テンプレート画面の状態。
#[derive(Debug, Clone, Default)]
pub struct TemplatesView {
    /// アーカイブしたテンプレートを出している (偽なら使えるもの)。
    pub archived: bool,
    pub rows: Vec<TemplateRow>,
    pub cursor: usize,
    pub offset: usize,
    /// 最後に読んだときのディレクトリの状態。変わったときだけ読み直すため。
    pub signature: Signature,
}

impl TemplatesView {
    pub fn selected(&self) -> Option<&TemplateRow> {
        self.rows.get(self.cursor)
    }

    pub fn select_name(&mut self, name: &str) -> bool {
        match self.rows.iter().position(|r| r.name == name) {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }
}

/// `dir` のテンプレートを、更新時刻の新しい順に読む (Neovim 版の一覧と同じ順)。
/// `records` は `.state.json` の、定期タスクをどの日まで確かめたか。アーカイブしたものの一覧では `None`
/// (足さないので、次に足す日を出さない)。`rollover` は次の日次の整理の日。読めないファイルは飛ばして、
/// その理由を `warnings` に入れる。
pub fn rows(
    dir: &Path,
    today: NaiveDate,
    rollover: NaiveDate,
    records: Option<&BTreeMap<String, String>>,
    warnings: &mut Vec<String>,
) -> Vec<TemplateRow> {
    let names = match template::list_names(dir) {
        Ok(names) => names,
        Err(e) => {
            warnings.push(format!("failed to read {}: {e}", dir.display()));
            return Vec::new();
        }
    };
    names
        .into_iter()
        .filter_map(|name| match store::read_file(&dir.join(format!("{name}.md"))) {
            Ok(file) => {
                let recorded = records.and_then(|r| r.get(&name)).map(String::as_str);
                let mut summary = template::summarize(&file.lines, today, rollover, recorded);
                if records.is_none()
                    && let Some(Recurrence::Valid { next, .. }) = &mut summary.recurrence
                {
                    *next = None;
                }
                Some(TemplateRow {
                    tasks: summary.tasks,
                    placeholders: summary.placeholders,
                    recurrence: summary.recurrence,
                    name,
                })
            }
            Err(e) => {
                warnings.push(e.to_string());
                None
            }
        })
        .collect()
}
