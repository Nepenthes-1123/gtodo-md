# 変更履歴

各版の変更点です。版の付け方は [Semantic Versioning](https://semver.org/lang/ja/) に従います。
共有設定 (`config.json`・`gtodo.json`) は v0.1.0 から、前の版と互換のある変え方 (項目を足す) だけにします。

## v0.1.0 (未リリース)

最初のリリースです。[gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) v0.3.0 以降と同じデータディレクトリを、同時に使えます。

### CLI

- タスクの一覧・検索・期日の Queue・件数 (`list`・`search`・`queue`・`stats`。`--json` も)。読むだけのコマンドはファイルを一切書き換えません
- タスクの追加・完了・キャンセル・移動・編集・並べ替え (`add`・`done`・`undone`・`cancel`・`move`・`edit`・`sort`)
- タスクの分割とプロジェクトへの昇格 (`split`)
- プロジェクトファイル (`project list`・`show`・`new`・`archive`・`restore`)
- テンプレート (`template list`・`new`・`insert`・`archive`・`restore`)
- シェルの補完 (`completions`)

### TUI (`gtodo`・`gtodo tui`)

- セクションごとのタブの一覧と、1 打鍵での操作 (Neovim 版と同じキー)
- 追加・編集のフォーム、Queue、検索、カンバン
- 本文の markdown の表示、全文の詳細欄、`gx` でリンクを開く
- `E` でファイルを `$EDITOR` で開く (保存する前に Neovim 版と同じ検証をする)
- プロジェクト画面 (`P`)・テンプレート画面 (`T`)・分割 (`p`)
- 期日の近い Waiting のタスクの知らせ
- 外部の変更の反映と、日付が変わったときの日次の整理
- `tui.toml` でキーを変えられる

### Neovim 版と同時に使うために

- ファイルの形・タスク行の文法・自動処理の結果を Neovim 版と同じにしています (Neovim 版から作った期待値でテストしています)
- 同じロック (`.gtodo.lock`) を使い、読んだ後に他のツールがファイルを書き換えていたら書かずに止まります
- Neovim 版と挙動を変えたところ (Neovim 版の不具合と判断したもの) は、[ROADMAP.md](ROADMAP.md) の決定事項に記録しています

### 対応 OS

Linux (x86_64・aarch64)、macOS (Apple シリコン・Intel)、Windows (x86_64)。
macOS はこの版の時点では、CI でテストを走らせていません (リポジトリを公開した後に足します)。
