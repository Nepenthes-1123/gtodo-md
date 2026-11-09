//! 書く内容から、読んだときにあった見出しが消えていないかを確かめる。
//!
//! Lua 版 `validate.lua` が利用者の保存時に確かめる見出し (トップヘッダー・todo.md の必須
//! セクション・done.md と cancelled.md の `## YYYY-MM`) と同じものを見る。CLI は読んだ文書を
//! コードで変えて書くだけなので、ここで止まるのは gtodo 自身の不具合のときに限られる。
//! 読んだときから欠けていた見出しは問わない。問うと、書き込むコマンドの前に必ず走る日次の整理で
//! 止まり、そのデータディレクトリでは書き込むコマンドがすべて使えなくなる。

use std::collections::BTreeSet;
use std::path::Path;

use crate::atomic::WriteError;
use crate::document::section_heading;
use crate::sections::{SectionKey, SectionNames};
use crate::store::FileKind;

/// 消えてはならない見出し。
#[derive(Debug)]
pub struct Landmarks {
    header: Option<&'static str>,
    sections: Vec<SectionKey>,
    months: BTreeSet<String>,
}

impl Landmarks {
    /// `lines` にある見出しを集める。
    pub fn of(kind: FileKind, lines: &[String], names: &SectionNames) -> Landmarks {
        let title = title(kind);
        let header = lines.iter().any(|l| l.starts_with(title)).then_some(title);
        let headings = || lines.iter().filter_map(|l| section_heading(l));
        let sections = match kind {
            FileKind::Todo => SectionKey::ALL
                .into_iter()
                .filter(|&k| headings().any(|h| names.key_of(h) == Some(k)))
                .collect(),
            _ => Vec::new(),
        };
        let months = match kind {
            FileKind::Done | FileKind::Cancelled => lines.iter().filter_map(|l| month_heading(l)).collect(),
            _ => BTreeSet::new(),
        };
        Landmarks {
            header,
            sections,
            months,
        }
    }

    /// `self` (読んだとき) にあって `after` (書く内容) に無い見出しがあれば、書かずにエラーにする。
    pub fn check(&self, path: &Path, after: &Landmarks, names: &SectionNames) -> Result<(), WriteError> {
        let mut missing: Vec<String> = Vec::new();
        if let Some(header) = self.header.filter(|_| after.header.is_none()) {
            missing.push(header.to_string());
        }
        for key in self.sections.iter().filter(|k| !after.sections.contains(k)) {
            missing.push(format!("## {}", names.name(*key)));
        }
        for month in self.months.difference(&after.months) {
            missing.push(format!("## {month}"));
        }
        if missing.is_empty() {
            return Ok(());
        }
        Err(WriteError::WouldRemove {
            path: path.to_path_buf(),
            missing: missing.join(", "),
        })
    }
}

/// Lua 版は todo.md の `# Todo` を確かめないが、ここでも確かめる。gtodo の書き込みで消える理由は無く、
/// 消えたなら不具合なので、他のファイルのトップヘッダーと同じく止めてよい。
fn title(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Inbox => "# Inbox",
        FileKind::Todo => "# Todo",
        FileKind::Done => "# Done",
        FileKind::Cancelled => "# Cancelled",
    }
}

/// `## YYYY-MM` の年月。Lua 版 (`^##%s+(%d%d%d%d%-%d%d)$`) と同じく、後ろに何かあれば年月の見出しとしない。
fn month_heading(line: &str) -> Option<String> {
    let rest = line.strip_prefix("##")?;
    let month = rest.trim_start_matches(crate::lua_compat::is_space);
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let (y, m) = month.split_once('-')?;
    (month.len() < rest.len() && y.len() == 4 && m.len() == 2 && digits(y) && digits(m)).then(|| month.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn check(kind: FileKind, before: &str, after: &str, names: &SectionNames) -> Result<(), String> {
        let of = |text| Landmarks::of(kind, &lines(text), names);
        of(before)
            .check(Path::new("f.md"), &of(after), names)
            .map_err(|e| match e {
                WriteError::WouldRemove { missing, .. } => missing,
                other => panic!("{other}"),
            })
    }

    #[test]
    fn removing_a_heading_that_was_read_is_refused() {
        let names = SectionNames::default();
        let todo = "# Todo\n## Today\n## Next\n## Waiting\n## Someday\n";
        assert_eq!(
            check(FileKind::Todo, todo, "# Todo\n## Today\n## Next\n## Someday\n", &names),
            Err("## Waiting".into())
        );
        assert_eq!(
            check(FileKind::Inbox, "# Inbox\n- [ ] x\n", "- [ ] x\n", &names),
            Err("# Inbox".into())
        );
        assert_eq!(
            check(
                FileKind::Done,
                "# Done\n## 2026-01\n## 2026-02\n",
                "# Done\n## 2026-02\n",
                &names
            ),
            Err("## 2026-01".into())
        );
    }

    #[test]
    fn every_top_header_is_kept_and_matched_by_prefix_as_in_lua() {
        let names = SectionNames::default();
        assert_eq!(
            check(FileKind::Todo, "# Todo\n## Today\n", "## Today\n", &names),
            Err("# Todo".into())
        );
        assert_eq!(
            check(FileKind::Cancelled, "# Cancelled\n", "", &names),
            Err("# Cancelled".into())
        );
        assert_eq!(
            check(FileKind::Inbox, "# Inbox (仕事)\n", "# Inbox (仕事)\n", &names),
            Ok(())
        );
        assert_eq!(
            check(FileKind::Inbox, "# Inbox (仕事)\n", "# 受信\n", &names),
            Err("# Inbox".into())
        );
    }

    #[test]
    fn headings_missing_since_reading_and_renamed_aliases_are_allowed() {
        let names = SectionNames::from_last(&[Some("今日".into()), None, None, None]).unwrap();
        assert_eq!(
            check(
                FileKind::Todo,
                "## Today\n## Next\n",
                "# Todo\n## 今日\n## Next\n",
                &names
            ),
            Ok(()),
            "the default name is rewritten to the current name"
        );
        assert_eq!(
            check(
                FileKind::Cancelled,
                "# Cancelled\n",
                "# Cancelled\n## 2026-02\n",
                &names
            ),
            Ok(())
        );
        assert_eq!(
            check(FileKind::Todo, "## Today\n## 今日\n## Today\n", "## 今日\n", &names),
            Ok(()),
            "duplicate headings are merged into one"
        );
    }

    /// 書き出すと見出しを整形し直す (別名を現在の名前へ、同じ名前の見出しを 1 つへ) が、
    /// それで止まらないことを、ゴールデンテストの入力のすべてで確かめる。
    #[test]
    fn rewriting_any_golden_input_keeps_its_landmarks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/golden");
        let mut files = Vec::new();
        for dir in [root.join("markdown")].into_iter().chain(
            std::fs::read_dir(root.join("logic"))
                .unwrap()
                .map(|e| e.unwrap().path()),
        ) {
            if dir.is_dir() {
                files.extend(std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()));
            }
        }
        files.retain(|p| p.extension().is_some_and(|e| e == "md"));
        assert!(files.len() > 50, "{}", files.len());

        let aliased = SectionNames::from_last(&[Some("今日".into()), Some("次".into()), None, None]).unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 2, 3).unwrap();
        for names in [SectionNames::default(), aliased] {
            for path in &files {
                let lines = crate::store::read_file(path).unwrap().lines;
                let written = crate::document::Document::parse(&lines, &names, today).serialize();
                for kind in [FileKind::Inbox, FileKind::Todo, FileKind::Done, FileKind::Cancelled] {
                    let after = Landmarks::of(kind, &written, &names);
                    let result = Landmarks::of(kind, &lines, &names).check(path, &after, &names);
                    assert!(result.is_ok(), "{}: {result:?}", path.display());
                }
            }
        }
    }

    #[test]
    fn month_headings_are_recognized_as_in_lua() {
        assert_eq!(month_heading("## 2026-02").as_deref(), Some("2026-02"));
        assert_eq!(month_heading("##\t2026-02").as_deref(), Some("2026-02"));
        for line in [
            "##2026-02",
            "## 2026-02 ",
            "## 2026-2",
            "## 26-02",
            "### 2026-02",
            "## 2026-02 x",
        ] {
            assert_eq!(month_heading(line), None, "{line:?}");
        }
    }
}
