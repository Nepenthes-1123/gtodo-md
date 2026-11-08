//! タスクリストに対する操作。Lua 版 `logic/` の移植で、ファイルには触れない
//! (読み書きと、2 ファイルにまたがる書き込みの順序は [`crate::ops`] が受け持つ)。

pub mod completion;
pub mod due;
pub mod history;
pub mod sort;

// 期日の比較は Lua 版と同じく文字列のまま行う。`YYYY-MM-DD` に正規化できた期日なら日付の順と一致する。
// 正規化できなかった表記 (`due:someday` など) との順も、LuaJIT の文字列比較はバイト順なので一致する
// (PUC Lua でビルドした Neovim は `strcoll` で比べるため、ロケールによっては食い違いうる)。
fn ymd(date: chrono::NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}
