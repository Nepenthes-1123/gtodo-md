//! gtodo-md の中核。CLI と TUI が共有する、Neovim に依存しない部分を持つ。
//!
//! データ形式は Neovim プラグイン [gtodo-md.nvim] と互換であり、同じ `data_dir` を
//! 同時に共有できることを前提とする。
//!
//! [gtodo-md.nvim]: https://github.com/Nepenthes-1123/gtodo-md.nvim

pub mod config;
pub mod date;
pub mod document;
mod lua_compat;
pub mod sections;
pub mod settings;
pub mod state;
pub mod store;
pub mod task;
pub mod view;
mod vim_class;
