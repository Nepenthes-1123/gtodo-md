# gtodo-md

GTD 志向の Markdown Todo 管理ツール [gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) の TUI/CLI 版です。
Neovim 版と同じファイル (`inbox.md` / `todo.md` / `done.md` / `cancelled.md` / `projects/*.md`) を読み書きし、同じデータを同時に使えることを目指しています。

開発中です。進捗は [ROADMAP.md](ROADMAP.md) を参照してください。現在は読み取り系のコマンドと `add`・`done`・`undone`・`cancel`・`move`・`sort` が使えます。

## 使い方

```sh
gtodo list                    # inbox.md と todo.md のタスク
gtodo list today              # inbox | todo | today | next | waiting | someday | done | cancelled
gtodo search +work @office    # +project / @context の AND 検索 (--open / --done で絞り込み)
gtodo queue                   # 期日ごとの一覧 (--wait で待ち先ごと)
gtodo stats                   # ステータスライン向けの件数
gtodo add 牛乳を買う +home     # inbox.md へ追加 (--to today|next|waiting|someday で todo.md のセクションへ)
gtodo add 資料 --due fri -p work -c office --priority A
gtodo done a1b2               # 完了にする (done.md へ移るのは翌日の整理のとき)
gtodo undone a1b2             # 完了を取り消す (done.md へ移る前まで)
gtodo cancel ~3fa91c          # cancelled.md へ移す
gtodo move a1b2 next          # todo.md のセクションへ移す (today | next | waiting | someday)
gtodo move a1b2 waiting --wait 田中さん
gtodo sort                    # 期日が来たタスクを Today へ移し、todo.md を並べ替える
```

`add` の本文に書いた `+project`・`@context`・`due:`・`wait:`・先頭の `(A)` は、タスク行と同じくタグとして読みます。`--due` などのオプションを付けた場合はそちらを優先します。
追加したタスクの ID を表示します。inbox へ追加したタスクの期日がすでに来ていれば、Neovim 版と同じく Today へ移します (`auto_move_inbox_to_today` が `true` のとき)。

`list`・`search`・`stats` は `--json` で JSON を出力します。
各タスクの先頭に表示されるキーは、タスクの ID (`id:` タグ) です。ID の無いタスクには、行の内容から作った一時キー (`~` で始まる) を表示します。
`done` などでタスクを指定するときは、このキーを使います。ID は他のタスクと区別できれば先頭の一部だけでも構いません。一時キーは行が書き換わると使えなくなるので (Neovim 版は保存のたびに ID を付けます)、そのときは `list` し直してください。
同じ ID のタスクが複数あるとどれも指定できないので、Neovim 版で片方の `id:` を消してください (保存したときに新しい ID が付きます)。

`move` は inbox.md か todo.md のタスクを todo.md のセクションへ移します。Waiting 以外へ移すと待ち先 (`wait:`) は消えます。Waiting へ移すとき、`--wait` を省略すると今の待ち先を残し、`--wait ""` で消します。すでに Waiting にあるタスクの待ち先を変えるときも `--wait` を使います。

読み取り系のコマンドはファイルを一切書き換えません。Neovim 版で編集中のファイルがあっても安全に使えます。

書き込むコマンド (`sort` など) は、Neovim 版と同じロック (`.gtodo.lock`) を持って実行します。Neovim 版が自動処理の最中なら少し待ち、2 秒待っても空かなければエラーになります。
実行の前に、Neovim 版と同じく次のことを行います。

- 無いファイル (`inbox.md` など) を Neovim 版と同じ雛形で作る
- その日まだ行っていなければ、日次の整理 (完了したタスクを `done.md` へ移し、期日が来たタスクを Today へ移す) をする

Neovim 版で開いているファイルを書き換えた場合、Neovim 側の未保存の編集は破棄されます (Neovim 版の仕様。undo で戻せます)。

## 設定

設定は Neovim 版と共有するファイルに書きます。どちらのツールで使っても同じ結果になるよう、2 つのファイルに分けています。
ただし、Neovim 版はまだこれらのファイルを読みません ([ROADMAP.md](ROADMAP.md) の「gtodo-md.nvim 側の対応」)。それまでは Neovim 版の `setup()` にも同じ値を書いてください。
キーマップや見た目など、ツールごとの設定はここには含めません。

### データディレクトリの場所 (`config.json`)

`~/.config/gtodo-md/config.json` (Windows は `%APPDATA%\gtodo-md\config.json`)。環境変数 `GTODO_CONFIG` で変更できます。
`data_dir` には絶対パスか `~` で始まるパスを書きます。

```json
{ "data_dir": "~/Documents/gtodo-md" }
```

データディレクトリは次の順で決まります。

1. `--data-dir`
2. 環境変数 `GTODO_DATA_DIR`
3. `config.json` の `data_dir`
4. Neovim 版の既定 `stdpath("data")/gtodo-md` (通常は `~/.local/share/nvim/gtodo-md`) が存在すればそこ
5. それ以外は `~/.local/share/gtodo-md` (Windows は `%LOCALAPPDATA%\gtodo-md`)

Neovim 版がまだ 4 の場所を使うため、存在する間はそちらを優先します。

環境変数 `XDG_CONFIG_HOME` / `XDG_DATA_HOME` が設定されていれば、どの OS でも `~/.config` / `~/.local/share` の代わりに使います。

### データの設定 (`<データディレクトリ>/gtodo.json`)

セクション名など、ファイルの中身に関わる設定です。データと一緒に置くので、データを別のマシンへ同期しても設定が付いていきます。
ファイルが無ければ、すべて既定値を使います。ただしセクション名は、`sections` が無ければ Neovim 版が最後に起動したときの名前 (`.state.json` に記録されたもの) を使います。Neovim 版の `setup()` で名前を変えている場合に、見出しを既定の名前で書き換えないためです。記録された名前が他のセクションの名前と重なっているなどで使えない場合、書き込むコマンドはエラーになるので、`sections` を設定してください。

```json
{
  "sections": { "today": "Today", "next": "Next", "waiting": "Waiting", "someday": "Someday" },
  "auto_move_inbox_to_today": true
}
```

- `sections`: `todo.md` の見出しの名前。書かなかったものは既定の名前になります。既定の名前の見出しは、名前を変えた後も同じセクションとして読みます。他のセクションの名前 (既定の名前を含む) と同じ名前は使えません。キーは小文字です (Neovim 版の `setup()` では `TODAY` などの大文字)。
- `auto_move_inbox_to_today`: 期日が来た Inbox のタスクを Today へ移すか。

知らないキーは警告して無視します (バージョンの違うツール同士で同じファイルを読むため)。

## 開発

ビルドとテストは Docker 上で行います (ホストに C リンカを入れずに済ませるため)。
`scripts/cargo-docker` は `cargo` と同じ引数を受け取ります。

```sh
scripts/cargo-docker test
scripts/cargo-docker clippy --all-targets -- -D warnings
```

リンカを使わない作業 (フォーマット・エディタの補完) には、[mise](https://mise.jdx.dev/) で入れたホスト側の Rust を使います。
Rust のバージョンは `mise.toml` と `docker/dev.Dockerfile` で揃えています。

```sh
mise install
cargo fmt --all -- --check
```

ホストに C リンカ (Ubuntu では `build-essential`) があれば、通常どおり `cargo test` も使えます。

## ライセンス

MIT License
