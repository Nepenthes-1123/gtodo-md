//! CLI・TUI でタスクを指定するためのキー。

use crate::task::Task;

/// タスクを指定するキー。ID があれば ID、無ければ行の内容から作った一時キー (`~` + 6桁16進)。
///
/// 一時キーは行番号ではなく行の内容から作る。ソートや他のインスタンスの書き込みで行の
/// 位置がずれても同じタスクを指し、行が書き換えられていれば一致しなくなるため、
/// 別のタスクを操作してしまうことがない。
pub fn task_key(task: &Task, line: &str) -> String {
    match task.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => line_key(line),
    }
}

/// 行の内容から作る一時キー。
///
/// プロセスや Rust のバージョンをまたいで同じ値になる必要があるため、
/// 標準ライブラリのハッシュ (安定性が保証されない) ではなく FNV-1a を使う。
pub fn line_key(line: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in line.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("~{:06x}", (h ^ (h >> 32)) & 0xFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_with_id_is_keyed_by_id() {
        let task = Task::parse("- [ ] x id:a1b2c3").unwrap();
        assert_eq!(task_key(&task, "- [ ] x id:a1b2c3"), "a1b2c3");
    }

    #[test]
    fn task_without_id_is_keyed_by_line_content() {
        let line = "- [ ] IDの無いタスク";
        let task = Task::parse(line).unwrap();
        let key = task_key(&task, line);
        assert!(key.starts_with('~') && key.len() == 7, "{key}");
        assert_eq!(key, line_key(line));
    }

    #[test]
    fn line_key_is_stable_across_releases() {
        // 期待値は独立した実装 (Python) で計算したもの。変わると既存の一時キーが無効になる。
        assert_eq!(line_key(""), "~d0bfc1");
        assert_eq!(line_key("- [ ] 牛乳を買う"), "~387609");
    }

    #[test]
    fn line_key_distinguishes_trailing_whitespace() {
        assert_ne!(line_key("- [ ] 牛乳を買う"), line_key("- [ ] 牛乳を買う "));
    }
}
