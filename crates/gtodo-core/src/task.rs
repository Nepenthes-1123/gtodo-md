//! タスク行の文法。**ディスク上のタスク形式を知っているのはこのモジュールだけ**であり、
//! 他のモジュールが独自にタグの正規表現を組んではならない。
//!
//! Lua 版 `task.lua` の移植で、同じ入力に対して同じ結果を返すことを
//! `testdata/golden/task_parse.json` (Lua 版から生成) で検証している。

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, NaiveDate};
use regex::Regex;

use crate::{date, lua_compat, vim_class};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Open,
    Done,
}

/// 1行分のタスク。
///
/// タグの値は `None` と空文字列を区別せず、どちらも「無い」として扱う。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Task {
    pub indent: String,
    pub status: Status,
    /// 行頭の `(A)` 形式の優先度。
    pub priority: Option<char>,
    pub content: String,
    /// `+project` の `project` 部分。
    pub project: Option<String>,
    /// `@context` の `context` 部分 (先頭の `@` を含まない)。
    pub context: Option<String>,
    /// 解釈できた期日は `YYYY-MM-DD`、解釈できなければ書かれていたままの表記。
    pub due: Option<String>,
    pub created: Option<String>,
    pub wait: Option<String>,
    pub completed_at: Option<String>,
    /// `done:` タグの値。完了状態そのものは [`Task::status`]。
    pub done: Option<String>,
    pub cancelled: Option<String>,
    pub from: Option<String>,
    /// 6桁16進の一意な識別子。手編集で他の値が入っていても受け付ける。
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum TagKey {
    Id,
    CompletedAt,
    Done,
    Cancelled,
    From,
    Due,
    Created,
    Context,
    Project,
    Wait,
}

struct TagPattern {
    key: TagKey,
    /// Vim の `\<` (単語の始まり) を要求するか。
    word_start: bool,
    /// 開始位置以降、行末までに一致する正規表現 (`^` から `$` まで)。
    suffix: Regex,
}

impl TagPattern {
    /// `vim.fn.match` と同じく、一致する最も左の開始位置 (バイト) を返す。
    fn find(&self, text: &str) -> Option<usize> {
        text.char_indices()
            .map(|(i, _)| i)
            .filter(|&i| !self.word_start || is_vim_word_start(text, i))
            .find(|&i| self.suffix.is_match(&text[i..]))
    }
}

/// 行末から剥がしていく順序。Lua 版の `TAG_PATTERNS` と同じ順序でなければならない
/// (同じ行に複数の解釈がありうるとき、どれを採るかがこの順序で決まる)。
///
/// Vim の正規表現から書き換える際の対応:
/// `\s` は `[ \t]` だけを指し (Rust の `\s` は全角スペース等も含む)、`\S` はその否定、
/// `\w` は `[0-9A-Za-z_]`、`[:space:]` は `[\t\n\x0B\x0C\r ]`。
static TAG_PATTERNS: LazyLock<Vec<TagPattern>> = LazyLock::new(|| {
    let p = |key, word_start, re: &str| TagPattern {
        key,
        word_start,
        suffix: Regex::new(re).expect("tag pattern must compile"),
    };
    vec![
        p(TagKey::Id, true, r"^id:[^ \t]+[ \t]*$"),
        p(
            TagKey::CompletedAt,
            true,
            r"^completed_at:[0-9]{4}-[0-9]{2}-[0-9]{2}[ \t]*$",
        ),
        p(TagKey::Done, true, r"^done:[0-9]{4}-[0-9]{2}-[0-9]{2}[ \t]*$"),
        p(TagKey::Cancelled, true, r"^cancelled:[0-9]{4}-[0-9]{2}-[0-9]{2}[ \t]*$"),
        p(TagKey::From, true, r"^from:[0-9A-Za-z_]+[ \t]*$"),
        p(TagKey::Due, true, r"^due:[a-zA-Z0-9_/%+-]+[ \t]*$"),
        p(TagKey::Created, true, r"^created:[0-9]{4}-[0-9]{2}-[0-9]{2}[ \t]*$"),
        p(TagKey::Context, false, r"^[ \t]+@[a-zA-Z0-9_/.-]+[ \t]*$"),
        p(TagKey::Project, false, r"^[ \t]+\+[a-zA-Z0-9_/.-]+[ \t]*$"),
        p(TagKey::Wait, true, r"^wait:[^\t\n\x0B\x0C\r 　。、.,()（）]+[ \t]*$"),
    ]
});

/// Vim の `\<` が `text` のバイト位置 `i` で成立するか。
///
/// Vim は ASCII の単語文字かどうかではなく、前後の文字の「文字クラス」が変わるかで
/// 判定する。そのため `買い物id:` の `id` は単語の始まりになり、`caféid:` はならない。
/// Rust の `\b` は Unicode の単語文字で判定するため、どちらも単語の途中と見なしてしまう。
fn is_vim_word_start(text: &str, i: usize) -> bool {
    let Some(cur) = text[i..].chars().next() else {
        return false;
    };
    if !vim_class::is_word_char(cur) {
        return false;
    }
    match text[..i].chars().next_back() {
        Some(prev) => !vim_class::is_word_char(prev),
        None => true,
    }
}

/// 行頭の `- [ ]` / `- [x]` を分解して `(indent, status, rest)` を返す。
fn split_checkbox(line: &str) -> Option<(&str, Status, &str)> {
    let body = line.trim_start_matches(lua_compat::is_space);
    let indent = &line[..line.len() - body.len()];
    let body = body.strip_prefix('-')?.trim_start_matches(lua_compat::is_space);
    let body = body.strip_prefix('[')?;
    let mut chars = body.chars();
    let status = match chars.next()? {
        ' ' => Status::Open,
        'x' | 'X' => Status::Done,
        _ => return None,
    };
    let rest = chars.as_str().strip_prefix(']')?;
    Some((indent, status, rest.trim_start_matches(lua_compat::is_space)))
}

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// 新しいタスク ID (6桁16進) を発行する。
///
/// 一意性は保証しない。ファイル内での重複は書き込み時に検出して再発行する。
pub fn generate_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let seed = nanos ^ (u64::from(std::process::id()) << 32) ^ ID_SEQ.fetch_add(1, Ordering::Relaxed);
    // 時刻の下位ビットは OS によって粒度が粗い (Windows は 100ns 単位) ため、そのまま
    // 24 ビットに切り詰めると偏る。splitmix64 で全ビットを混ぜてから切り詰める。
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    format!("{:06x}", z & 0xFF_FFFF)
}

fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

impl Task {
    /// タスク行を解析する。タスク行でなければ `None`。相対的な期日は今日から解決する。
    pub fn parse(line: &str) -> Option<Task> {
        Self::parse_at(line, Local::now().date_naive())
    }

    /// [`Task::parse`] の「今日」を指定できる版。
    pub fn parse_at(line: &str, today: NaiveDate) -> Option<Task> {
        let (indent, status, rest) = split_checkbox(line)?;
        let mut task = Task {
            indent: indent.to_string(),
            status,
            ..Task::default()
        };

        // 先頭のタグも「直前が空白」の条件を満たせるよう、番兵の空白を置く。
        let mut text = format!(" {rest}");
        while let Some((key, pos)) = TAG_PATTERNS.iter().find_map(|p| p.find(&text).map(|pos| (p.key, pos))) {
            let matched = lua_compat::trim(&text[pos..]).to_string();
            text.truncate(pos);
            // 同じタグが複数あれば、左にあるもの (後から剥がされたもの) が残る。Lua 版と同じ。
            let after_colon = || matched.split_once(':').map(|(_, v)| v.to_string());
            match key {
                TagKey::Project => task.project = Some(matched[1..].to_string()),
                TagKey::Context => task.context = Some(matched[1..].to_string()),
                TagKey::Due => {
                    let raw = &matched["due:".len()..];
                    task.due = Some(date::parse_due_date(raw, today).unwrap_or_else(|| raw.to_string()));
                }
                TagKey::Id => task.id = after_colon(),
                TagKey::CompletedAt => task.completed_at = after_colon(),
                TagKey::Done => task.done = after_colon(),
                TagKey::Cancelled => task.cancelled = after_colon(),
                TagKey::From => task.from = after_colon(),
                TagKey::Created => task.created = after_colon(),
                TagKey::Wait => task.wait = after_colon(),
            }
        }

        let cleaned = lua_compat::collapse_spaces(lua_compat::trim(&text));
        let cleaned = lua_compat::trim(&cleaned);
        match split_priority(cleaned) {
            Some((pri, content)) => {
                task.priority = Some(pri);
                task.content = content.to_string();
            }
            None => task.content = cleaned.to_string(),
        }
        Some(task)
    }

    /// タスク行の文字列へ戻す。ID が無ければここで発行し、`self` にも書き戻す
    /// (以降の呼び出しで別の ID が振られないようにするため)。
    ///
    /// タグの順序は Lua 版と同じで、`id:` は常に末尾に付く。
    pub fn serialize(&mut self) -> String {
        if present(&self.id).is_none() {
            self.id = Some(generate_id());
        }

        let status = match self.status {
            Status::Open => ' ',
            Status::Done => 'x',
        };
        let priority = self.priority.map(|p| format!("({p}) ")).unwrap_or_default();
        let mut parts = vec![format!("{}- [{status}] {priority}{}", self.indent, self.content)];

        if let Some(p) = present(&self.project) {
            parts.push(format!("+{p}"));
        }
        if let Some(c) = present(&self.context) {
            parts.push(if c.starts_with('@') {
                c.to_string()
            } else {
                format!("@{c}")
            });
        }
        for (name, value) in [
            ("due", &self.due),
            ("created", &self.created),
            ("wait", &self.wait),
            ("completed_at", &self.completed_at),
            ("done", &self.done),
            ("cancelled", &self.cancelled),
            ("from", &self.from),
            ("id", &self.id),
        ] {
            if let Some(v) = present(value) {
                parts.push(format!("{name}:{v}"));
            }
        }
        parts.join(" ")
    }
}

/// `(A) 本文` を `('A', "本文")` に分ける。後ろの空白を必須にして、
/// `(A)本文` のような本文の書きぶりを優先度と取り違えないようにする。
fn split_priority(s: &str) -> Option<(char, &str)> {
    let b = s.as_bytes();
    if b.len() < 4 || b[0] != b'(' || !b[1].is_ascii_uppercase() || b[2] != b')' {
        return None;
    }
    let after = &s[3..];
    let content = after.trim_start_matches(lua_compat::is_space);
    if content.len() == after.len() {
        return None;
    }
    Some((b[1] as char, lua_compat::trim(content)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_assigns_id_and_keeps_it_on_the_task() {
        let mut task = Task::parse("- [ ] 新しいタスク").unwrap();
        assert_eq!(task.id, None);

        let first = task.serialize();
        let id = task.id.clone().expect("serialize must store the issued id");
        assert!(first.ends_with(&format!(" id:{id}")));
        assert_eq!(task.serialize(), first, "a second serialize must not reissue the id");
    }

    #[test]
    fn serialize_keeps_existing_id() {
        let mut task = Task::parse("- [ ] 既存 id:abc123").unwrap();
        assert_eq!(task.serialize(), "- [ ] 既存 id:abc123");
    }

    #[test]
    fn serialize_treats_empty_tag_values_as_absent() {
        let mut task = Task {
            content: "本文".into(),
            project: Some(String::new()),
            due: Some(String::new()),
            id: Some("abc123".into()),
            ..Task::default()
        };
        assert_eq!(task.serialize(), "- [ ] 本文 id:abc123");
    }

    #[test]
    fn serialize_accepts_context_with_or_without_at_sign() {
        for ctx in ["home", "@home"] {
            let mut task = Task {
                content: "本文".into(),
                context: Some(ctx.into()),
                id: Some("abc123".into()),
                ..Task::default()
            };
            assert_eq!(task.serialize(), "- [ ] 本文 @home id:abc123", "{ctx:?}");
        }
    }

    #[test]
    fn generated_ids_are_six_lowercase_hex_digits() {
        for _ in 0..100 {
            let id = generate_id();
            assert_eq!(id.len(), 6, "{id}");
            assert!(
                id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{id}"
            );
        }
    }

    #[test]
    fn consecutive_generated_ids_differ() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| generate_id()).collect();
        // 24 ビット空間で 1000 個なら誕生日衝突の期待値は約 0.03 個。
        assert!(ids.len() >= 998, "too many collisions: {}", 1000 - ids.len());
    }
}
