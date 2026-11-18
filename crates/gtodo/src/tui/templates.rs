//! テンプレート画面の行 (core の `template` で `templates/*.md` を読んで作る)。

use std::collections::HashMap;
use std::path::Path;

use chrono::NaiveDate;
use gtodo_core::store;
use gtodo_core::template;

use super::projects::Signature;

/// テンプレート画面の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRow {
    pub name: String,
    /// 挿入されるタスクの数。
    pub tasks: usize,
    /// 挿入するときに尋ねるプレースホルダー。
    pub placeholders: Vec<String>,
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
/// 読めないファイルは飛ばして、その理由を `warnings` に入れる。
pub fn rows(dir: &Path, today: NaiveDate, warnings: &mut Vec<String>) -> Vec<TemplateRow> {
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
            Ok(file) => Some(TemplateRow {
                tasks: template::tasks(&file.lines, today, &HashMap::new()).len(),
                placeholders: template::placeholder_names(&file.lines),
                name,
            }),
            Err(e) => {
                warnings.push(e.to_string());
                None
            }
        })
        .collect()
}
