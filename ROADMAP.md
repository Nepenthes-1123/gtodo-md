# ロードマップ

[gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) と同じデータを扱う TUI/CLI を Rust で作る。
完了した項目はチェックを付け、方針を変えたら「決定事項」を更新する。

## M0: 土台

- [x] Cargo ワークスペース (`gtodo-core` ライブラリ)
- [x] タスク行の文法 (`task.rs`): parse / serialize / ID 発行
- [x] 期日表記の正規化 (`date.rs`)
- [x] Lua 版から生成するゴールデンテスト (`scripts/gen-golden-task.lua`)
- [x] Vim の単語境界判定に使う文字クラス表の生成 (`scripts/gen-vim-charclass.lua`)
- [x] ビルド・テスト・lint を通す
- [x] Docker 上でビルド・テストする手順 (`scripts/cargo-docker`)

## M1: core と CLI

core (`gtodo-core`)

- [x] ファイル構造の解析 (`parse_markdown` 相当。セクション別名を含む。Lua 版から生成するゴールデンテスト)
- [ ] ファイル構造の書き出し (`write_todo_file` 相当。ID 重複の再発行・連続空行の圧縮)
- [ ] 共有設定の読み込み: `config.json` (`data_dir` の場所)・`<data_dir>/gtodo.json` (セクション名・`auto_move_inbox_to_today`)。`GTODO_DATA_DIR`・`--data-dir`
- [ ] 書き込み: アトミック置換 (同じディレクトリの一時ファイル + fsync + rename)・内容が同じなら書かない
- [ ] 並行更新の検出 (読んだ時点と書く直前の stat の比較)。stat は読む**前に**取り、改行コード (CRLF か) と一緒に読み込み結果に持たせる (今の `read_lines` は行しか返さない)
- [ ] `.gtodo.lock` (Lua 版と同じ手順)
- [ ] `.state.json` (未知のキーを保持した読み書き)
- [ ] ソート・期日による昇格・完了タスクの履歴への移動・2ファイルにまたがる移動 (追記してから削除)
- [ ] 日次ロールオーバー
- [ ] 保存時バリデーション (必須セクション・ヘッダ・`## YYYY-MM` 見出しの削除保護)

CLI (`gtodo`)

- [ ] `gtodo add <text> [--to …] [--due …] [-p project] [-c context] [--priority X]`
- [ ] `gtodo list [inbox|todo|today|next|waiting|someday|done|cancelled] [--json]`
- [ ] `gtodo done | undone | cancel <task>`
- [ ] `gtodo move <task> <section> [--wait who]`
- [ ] `gtodo edit <task> …`
- [ ] `gtodo search [+project] [@context] [--open|--done]`
- [ ] `gtodo queue`
- [ ] `gtodo sort` (期日チェックとソート)
- [ ] `gtodo stats`

## gtodo-md.nvim 側の対応

このリポジトリとは別の作業。

- [ ] 共有設定 (`config.json`) を読む。`setup()` の同じ項目との関係 (食い違ったら警告する) と、既存の利用者の移行を決める
- [ ] 既定の `data_dir` を `$XDG_DATA_HOME/gtodo-md` にし、旧い場所 (`stdpath("data")/gtodo-md`) へ後退する。移ったら CLI/TUI 側の優先順位も入れ替える
- [ ] 決定事項に記載した Lua 版の不具合を、Lua 版でも直すか決める
- [ ] セクション名の重複を検査する (Lua 版の `sanitize_sections` は検査しない)

## M2: TUI

- [ ] セクションをタブで切り替える一覧 (`gtodo tui`、引数なしでも起動)
- [ ] Neovim 版と同じキーでの操作 (`d`/`n`/`w`/`s`/`x`/`c`)
- [ ] 追加・編集フォーム
- [ ] Queue ビュー
- [ ] 検索
- [ ] カンバン
- [ ] 外部変更の検知と再読み込み
- [ ] `$EDITOR` で生のファイルを開く

## M3 以降

- [ ] プロジェクトファイル (フロントマター・進捗)
- [ ] テンプレート
- [ ] タスクの分割・プロジェクトへの昇格
- [ ] Waiting タスクの期限警告

## 決定事項

2026-11-08 時点。

- **言語は Rust。** clap + ratatui。`Result` によってエラーの握りつぶしをコンパイラに防がせられることを重視した。
- **ビルドとテストは Docker 上で行う。** ホスト環境を汚さないため、C リンカ (`build-essential`) はホストに入れない。ホストの Rust (mise) はフォーマットとエディタの補完に使う。
- **Neovim 版と同じ `data_dir` を同時に共有できる互換性を持たせる。** タスク行の文法・`.gtodo.lock`・`.state.json`・アトミック置換・2ファイルにまたがる移動の順序を Lua 版に合わせる。
- **タスク行の文法は Lua 版をゴールデンテストで固定する。** Lua 版は Vim の正規表現で書かれており、単語境界 (`\<`) や `\s` の範囲が Rust の正規表現と異なるため、手で移植するだけでは細部がずれる。
- **CLI でのタスクの指定方法。**
  - ID があるタスクは ID で指定する (一意に絞れれば先頭の一部でもよい)。
  - ID が無いタスクは、行の内容から作った一時キー (例: `~3fa91c`) で指定する。操作時に読み直して一致する行を探し、見つからない・複数ある場合はエラーにする。
  - 行番号では指定しない。`list` から操作までの間に自動ソートや他インスタンスの書き込みで行がずれると、別のタスクを操作してしまうため。
  - `list` などの読むだけのコマンドは書き込まない (ID の発行もしない)。書くと Neovim 側の未保存の編集が破棄され、全インスタンスで無駄なリロードが起きるため。ID は書き込むコマンドが実行されたときに付く。
- **書き込むコマンドは、実行前に日次ロールオーバーを確認する。** Neovim 版の 60 秒タイマーの代わり。日付が変わっていればロックを取って1日1回だけ実行する。
- **Neovim 側で未保存の編集があるファイルに CLI が書き込んだ場合の挙動は、Neovim 版の仕様に従う。** 外部ツールによる変更と同じく、Neovim 側の未保存の編集は破棄される (undo で戻せる)。
- **Neovim 版と CLI/TUI は同列に扱う。どちらかを正本にはしない。** 両方が守るべき設定は、どちらのツールにも属さない共有ファイルに置き、両方が読む。
  - `config.json` (`$XDG_CONFIG_HOME/gtodo-md/`、Windows は `%APPDATA%\gtodo-md\`): `data_dir` の場所だけ。絶対パスか `~` 始まりに限る (実行したディレクトリで場所が変わらないように)。
  - キーマップ・見た目などツール固有の設定は共有しない (Neovim 版は `setup()`、CLI/TUI は今のところ無し)。
  - 形式は JSON。Neovim (`vim.json`) と Rust のどちらも追加の依存なしで読めるため。TOML は Lua 側にパーサーが要る。
  - 知らないキーはエラーにせず警告する。バージョンの違うツール同士で同じファイルを読むため。値の型が違うものはエラーにする。
- **既定の `data_dir` は `$XDG_DATA_HOME/gtodo-md` (Windows は `%LOCALAPPDATA%\gtodo-md`)。ただし Neovim 版の既定 (`stdpath("data")/gtodo-md`) が存在する間はそちらを優先する。** 新規の利用者は Neovim に依存しない場所から始められ、既存のデータは移動せずに使える。現行の Neovim 版は旧い既定を使い続けるため、新しい場所を優先すると、何かの拍子にそこが作られたとき CLI だけが別の (空かもしれない) ディレクトリを黙って見ることになる。Neovim 版が新しい既定へ移ったら、優先順位を入れ替える。
  - macOS も `~/.config`・`~/.local/share` を使う (Neovim 自身と同じ)。`XDG_*` が設定されていれば、Windows でもそちらを優先する (Neovim 自身と同じ)。
  - 相対パスの `XDG_*` は XDG の仕様どおり無視する。Neovim はそのまま使うが、カレントディレクトリで場所が変わる設定は意図したものと考えにくい。
- **セクション名の別名が他のキーの名前と衝突したときは、全キーの現在の名前 → デフォルト名 → 前回の名前の順に照合する。** Lua 版はこの順序が `pairs` 任せで定まっていない。現在の名前が他のセクションの現在の名前・デフォルト名と重複する設定は、照合以前にエラーにする (2 つのセクションが合流して読まれ、そのまま書き戻されるため)。
- **Lua 版の不具合と判断した挙動は、Rust 版では直したうえで差異として記録する。**
  - `## default` という見出し: Lua 版は最初の `##` より前の行を内部キー `"default"` で持つため衝突して見出しが消える。Rust 版は別に持つ。
