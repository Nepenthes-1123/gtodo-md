//! Lua 版の文字列処理と同じ結果を得るための補助関数。
//!
//! Lua のパターン `%s` は C ロケールの `isspace` であり、Rust の `char::is_whitespace`
//! (全角スペース等を含む) とは範囲が異なる。ここを取り違えると、本文の空白の畳み込みや
//! トリムの結果が Lua 版と食い違う。

/// Lua の `%s` に相当する(空白・タブ・改行・垂直タブ・改ページ・復帰)。
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

/// `vim.trim` 相当。前後の `%s` を取り除く。
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `s:gsub("%s+", " ")` 相当。連続する `%s` を半角空白1つに畳む。
pub(crate) fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if is_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}
