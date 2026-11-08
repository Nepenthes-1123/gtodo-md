# CLAUDE.md

## プロジェクト概要

gtodo-md は Neovim プラグイン [gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) の TUI/CLI 版 (Rust)。
Neovim 版と同じ `data_dir` を**同時に**共有できることを前提とする。進捗と決定事項は `ROADMAP.md` で管理する。

リポジトリ構成: 親ディレクトリの bare リポジトリ (`.git`) から作ったブランチごとの git worktree の1つ (`.main` が `main`)。

## コマンド

ホストには C リンカが無い (意図的に入れていない) ため、ビルド・テスト・clippy は Docker 上で実行する。
`cargo fmt` はリンカ不要なのでホストの cargo (mise) で実行してよい。

```sh
scripts/cargo-docker test
scripts/cargo-docker clippy --all-targets -- -D warnings
cargo fmt --all            # 列幅は 120 (rustfmt.toml)

# 生成物の再生成 (Neovim と gtodo-md.nvim のチェックアウトが必要)
nvim --clean --headless -l scripts/gen-golden-task.lua <path/to/gtodo-md.nvim>
nvim --clean --headless -l scripts/gen-golden-markdown.lua <path/to/gtodo-md.nvim>
nvim --clean --headless -l scripts/gen-golden-logic.lua <path/to/gtodo-md.nvim>
nvim --clean --headless -l scripts/gen-vim-charclass.lua > crates/gtodo-core/src/vim_class.rs
```

生成スクリプトは必ず `--clean` で実行すること。Vim の `\<` と `charclass()` はカレントバッファの `'iskeyword'` に依存するため、ユーザー設定が混ざると既定と異なる結果が記録される。

## 構成

- `crates/gtodo-core` — Neovim に依存しない中核 (ライブラリ)。
  - `task.rs` — タスク行の文法。**ディスク上のタスク形式を知っているのはこのモジュールだけ**。他のモジュールがタグの正規表現を組んではならない (Lua 版ではこれを破って3件の静かな破損が起きた)。
  - `date.rs` — 期日表記の正規化。
  - `document.rs` — ファイル構造 (`header` / `preamble` / `sections`) の解析と書き出し。Lua 版 `io.parse_markdown` / `io.write_todo_file` の移植。
  - `sections.rs` — 管理対象セクションの名前と別名 (現在の名前・デフォルト名・Neovim 版が最後に起動したときの名前)。
  - `store.rs` — `data_dir` のファイル配置と読み込み。読む前の stat (`Stamp`) と改行コードを一緒に返す。
  - `atomic.rs` — ファイルのアトミックな置き換え (Lua 版 `atomic_replace` と同じ手順)。
  - `config.rs` — `config.json` と `data_dir` の場所の解決 (Neovim 版と共有する規則)。OS ごとの場所の規則は `Platform` で受け取り、どの OS 上でもテストする (パスの区切りや絶対パスの判定はホスト OS の規則になる)。
  - `settings.rs` — `data_dir/gtodo.json` (セクション名など、データの形と自動処理の設定。Neovim 版と共有する)。
  - `state.rs` — `.state.json` の読み書き。知らないキーも保ったまま書き戻す。`last_sections` は Neovim 版の記録であり、CLI から書き換えてはならない。
  - `lock.rs` — `.gtodo.lock` (Neovim 版と共有する排他ロック)。
  - `logic/` — タスクリストに対する操作 (並べ替え)。Lua 版 `logic/` の移植で、ファイルには触れない。
  - `ops.rs` — `data_dir` のファイルを読み、`logic/` の操作をして書き戻す。ロックは呼び出し元が持つ。
  - `key.rs` — タスクを指定するキー (ID、または行の内容から作る一時キー `~xxxxxx`)。一時キーのハッシュ (FNV-1a) を変えると既存のキーが無効になる。
  - `view.rs` — 読み取り専用のビュー (一覧・検索・Queue・件数)。CLI と TUI で共有する。
  - `lua_compat.rs` — Lua の `%s` / `vim.trim` と同じ範囲で空白を扱う補助。
  - `vim_class.rs` — `scripts/gen-vim-charclass.lua` の生成物。手で編集しない。
- `crates/gtodo` — CLI (`gtodo` コマンド)。`workspace.rs` が `data_dir` とその設定を読む。
  **読むだけのコマンドはファイルを一切書き換えない** (書くと Neovim 側の未保存の編集が破棄されるため)。
  `tests/cli.rs` の `read_only_commands_never_create_or_modify_files` で固定している。
  結合テストは利用者の設定を読まないよう、`GTODO_CONFIG` に空の `config.json` を指定して実行する。

## 互換性の不変条件

- タスク行の解析結果・文字列化、ファイル構造の解析・書き出し、`logic/` の操作の結果は Lua 版と一致させる。`testdata/golden/*.json` は Lua 版から生成した期待値であり、Rust 側に合わせて手で書き換えてはならない。文法に関わる不一致は Lua 版を正として直す。ケースを増やすときは `task_cases.json` / `markdown/cases.json` / `logic/cases.json` に足して再生成する。
- Lua 版の不具合と判断して挙動を変えた箇所は、ゴールデンテストに含めず、Rust 側の個別のテストと `ROADMAP.md` の決定事項に記録する。
- Vim の正規表現を移植するときは、`\s` は `[ \t]`、`\w` は `[0-9A-Za-z_]`、`[:space:]` は `[\t\n\x0B\x0C\r ]` に置き換える。Rust の `\s`/`\w`/`\b` は Unicode の範囲で判定するため、そのまま使うと全角スペースや日本語の直後で結果が変わる。`\<` は `task.rs` の `is_vim_word_start` を使う。
- 意図して Lua 版と挙動を変えた箇所は、コードコメントに理由を書く (例: `date.rs` の夏時間の扱い)。
- `data_dir` のファイルを書くときは必ず `atomic.rs` を通し、読んだときの `Stamp` を渡す (`fs::write` で直接書かない)。並行更新の検出とアトミックな置き換えが効かなくなり、Neovim 版の変更を潰す。
- `config.json` と `gtodo.json` は Neovim 版と共有する。知らないキーで止めてはならない (警告して無視する)。項目を足すときは ROADMAP の決定事項と README を更新し、Neovim 版でも同じ意味で読めるようにする。

## ローカライズ

コードコメントとドキュメントは日本語、識別子とエラー・警告のメッセージは英語。
Neovim 版の画面に対応する表示 (Queue の見出しなど) は Neovim 版の文言に合わせる。
