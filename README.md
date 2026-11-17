# gtodo-md

GTD 志向の Markdown Todo 管理ツール [gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) の TUI/CLI 版です。
Neovim 版と同じファイル (`inbox.md` / `todo.md` / `done.md` / `cancelled.md` / `projects/*.md`) を読み書きし、同じデータを同時に使えることを目指しています。

開発中です。進捗は [ROADMAP.md](ROADMAP.md) を参照してください。現在は読み取り系のコマンドと `add`・`done`・`undone`・`cancel`・`move`・`edit`・`sort`、TUI の一覧とタスクの操作が使えます。

## 使い方

```sh
gtodo list                    # inbox.md と todo.md のタスク
gtodo list today              # inbox | todo | today | next | waiting | someday | done | cancelled
gtodo search 資料 +work       # 本文の語・+project・@context・wait:誰 の AND 検索 (--open / --done で絞り込み)
gtodo queue                   # 期日ごとの一覧 (--wait で待ち先ごと)
gtodo stats                   # ステータスライン向けの件数
gtodo add 牛乳を買う +home     # inbox.md へ追加 (--to today|next|waiting|someday で todo.md のセクションへ)
gtodo add 資料 --due fri -p work -c office --priority A
gtodo done a1b2               # 完了にする (done.md へ移るのは翌日の整理のとき)
gtodo undone a1b2             # 完了を取り消す (done.md へ移る前まで)
gtodo cancel ~3fa91c          # cancelled.md へ移す
gtodo move a1b2 next          # todo.md のセクションへ移す (today | next | waiting | someday)
gtodo move a1b2 waiting --wait 田中さん
gtodo edit a1b2 新しい本文 --due fri   # 指定したものだけ変える (--due "" などで消す)
gtodo sort                    # 期日が来たタスクを Today へ移し、todo.md を並べ替える
```

`add` の本文に書いた `+project`・`@context`・`due:`・`wait:`・先頭の `(A)` は、タスク行と同じくタグとして読みます。`--due` などのオプションを付けた場合はそちらを優先します。
追加したタスクの ID を表示します。inbox へ追加したタスクの期日がすでに来ていれば、Neovim 版と同じく Today へ移します (`auto_move_inbox_to_today` が `true` のとき)。

`search` の語は、本文に含まれる語 (一部だけでも当たり、大文字小文字と全角半角は区別しない)、`+project`・`@context`・`wait:誰` (タグの値と完全一致) です。すべての語に当たるタスクを出します。対象は inbox.md・todo.md・done.md で、タグや日付 (`created:` など) の文字は探しません。TUI の検索と同じ規則です。

`list`・`search`・`stats` は `--json` で JSON を出力します。
各タスクの先頭に表示されるキーは、タスクの ID (`id:` タグ) です。ID の無いタスクには、行の内容から作った一時キー (`~` で始まる) を表示します。
`done` などでタスクを指定するときは、このキーを使います。ID は他のタスクと区別できれば、先頭の 4 文字以上だけでも構いません。一時キーは行が書き換わると使えなくなるので (Neovim 版は保存のたびに ID を付けます)、そのときは `list` し直してください。
同じ ID のタスクが複数あるとどれも指定できないので、Neovim 版で片方の `id:` を消してください (保存したときに新しい ID が付きます)。

`move` は inbox.md か todo.md のタスクを todo.md のセクションへ移します。Waiting 以外へ移すと待ち先 (`wait:`) は消えます。Waiting へ移すとき、`--wait` を省略すると今の待ち先を残し、`--wait ""` で消します。すでに Waiting にあるタスクの待ち先を変えるときも `--wait` を使います。

`edit` は、本文を渡すとタスクの本文を置き換えます。本文に書いたタグとオプションで指定した項目だけを変え、指定しなかった項目は残します。タグだけを渡した場合、本文は変えません (先頭の `(A)` はタスク行と同じく後ろに本文があるときだけ優先度として読むので、優先度だけを変えるときは `--priority` を使います)。`--due ""`・`-p ""`・`-c ""`・`--priority ""` でその項目を消します。編集した後は Neovim 版と同じく期日をチェックし、todo.md のタスクなら todo.md を並べ替えます。

読み取り系のコマンドはファイルを一切書き換えません。Neovim 版で編集中のファイルがあっても安全に使えます。

書き込むコマンド (`sort` など) は、Neovim 版と同じロック (`.gtodo.lock`) を持って実行します。Neovim 版が自動処理の最中なら少し待ち、2 秒待っても空かなければエラーになります。
実行の前に、Neovim 版と同じく次のことを行います。

- 無いファイル (`inbox.md` など) を Neovim 版と同じ雛形で作る
- その日まだ行っていなければ、日次の整理 (完了したタスクを `done.md` へ移し、期日が来たタスクを Today へ移す) をする

Neovim 版で開いているファイルを書き換えた場合、Neovim 側の未保存の編集は破棄されます (Neovim 版の仕様。undo で戻せます)。

## TUI

`gtodo` を引数なしで実行すると (`gtodo tui` でも) TUI が開きます。今はセクションごとの一覧、タスクの操作、追加と編集、Queue、検索、カンバン、本文の markdown の表示が使えます (キーの設定ファイルと `$EDITOR` は開発中)。

| キー | 動作 |
| --- | --- |
| `j` / `k` (`↓` / `↑`) | 上下に移る。`gg` / `G` で先頭・末尾、`Ctrl-d` / `Ctrl-u` で半ページ、`Ctrl-f` / `Ctrl-b` で 1 ページ |
| `h` / `l` (`gT` / `gt`、`←` / `→`)、`1`〜`7` | タブを切り替える (Inbox・Today・Next・Waiting・Someday・Done・Cancelled) |
| `x` | 完了と未完了を切り替える |
| `d` / `n` / `w` / `s` | Today・Next・Waiting・Someday へ移す。`w` は待ち先を尋ねる (Waiting にあるタスクでは待ち先を変える。空にすると消す) |
| `c` | キャンセルする (確認あり) |
| `a` / `e` | タスクを追加する / 編集する (下記のフォーム) |
| `Q` | Queue を開く (下記) |
| `/` | 検索 (下記) |
| `K` | カンバン (下記) |
| `gx` | 選んでいるタスクのリンクを開く (複数あれば番号で選ぶ) |
| `o` | 期日が来たタスクを Today へ移し、並べ替える (`gtodo sort` と同じ) |
| `?` | ヘルプ |
| `q` / `Ctrl-c` | 終了 |
| `Esc` | 押しかけのキー (`g` の後など)・入力・確認を取り消す。終了はしない |

最下段には、選んでいるタスクで押したときに起きることを出します。Done と Cancelled のタブは見るだけです。
`NO_COLOR` を設定すると色を使いません。

`a` と `e` はフォームを開きます。項目は本文・期日・プロジェクト・コンテキスト・優先度と、追加では追加先 (初期値は開いているタブ)、Waiting では待ち先です。
本文は `gtodo add` と同じく、`+project`・`due:` などのタグも読みます (項目に入れた値が優先)。入力のたびに書き込まれる行を表示し、読めない期日などがあるうちは確定できません。
編集では、変えた項目だけを変えます。`Tab` / `↓` で次の項目、`Ctrl-n` / `Ctrl-p` でプロジェクト・コンテキストの候補 (使われているタグと `projects/*.md`)、`Enter` で確定、`Esc` で取り消します。
inbox へ追加したタスクの期日がすでに来ていれば、`gtodo add` と同じく Today へ移します。

`Q` は Queue (期日の来るタスクの一覧。Neovim 版の Queue ビューと同じ見出しと並び) を開きます。inbox.md と todo.md の未完了のタスクを、期限切れ・今日・明日・日付ごとに並べ、右端にどこにあるか (Inbox・セクション名) を出します。31 日以上先は薄く表示します。
Queue の中でも `x`・`d`/`n`/`w`/`s`・`c`・`e` でタスクを操作できます。`Tab` で期日別と待ち先別 (`wait:` ごと) を切り替え、`Enter` で一覧のそのタスクへ移り、`Esc` で一覧へ戻ります。
`gtodo.json` で `auto_move_inbox_to_today` を `false` にしているときは、起動したときと日付が変わったときに、期日が来ても inbox.md に残っているタスクの数を知らせます。

`/` は検索を開きます。打つたびに inbox.md・todo.md・done.md のタスクを絞り込み、当たった部分を太字と下線で示します。書き方は `gtodo search` と同じで、打っている途中の最後の `+project`・`@context`・`wait:` だけは前方一致になります。何も入れていなければ全部出します。
入力中も `↑` / `↓` で選べます。`Tab` で「すべて → 未完了 → 完了」を切り替え、`Enter` で結果の中へ移ると、一覧と同じキーで操作できます (done.md のタスクは見るだけ)。結果の中で `Enter` を押すと一覧のそのタスクへ移り、`/` で入力欄へ戻り、`Esc` で一覧へ戻ります。開き直すと前の検索語が残っています。

`K` はカンバン (Neovim 版のカンバンと同じ Someday・Next・Today・Waiting・Done の列) を開きます。未完了のタスクは todo.md のセクションの列に、完了したタスクは Done の列に新しい順に並びます (done.md のカードは見るだけ)。
`h` / `l` (`←` / `→`) で列を、`j` / `k` でカードを選び、一覧と同じキーでカードを操作します (`x` は確認なし)。カードを移した後も、カーソルは元の列の次のカードに残ります。`a` で選んでいる列のセクションへ追加し、`Enter` で一覧のそのタスクへ移り、`Esc` で一覧へ戻ります。画面に入らない列は左右に送ります (列の幅は 20 文字以上)。

本文の markdown は、TUI で表示するときだけ整えます (ファイルの中身・フォーム・CLI の出力は元のまま)。太字・斜体・コード・取り消し線・リンクを文字の装飾にし、リンクは文字だけ、URL (`<URL>` と、そのまま書いたもの) は「ホスト名/…/末尾」に短くします。画像と HTML は書かれたまま出します。`NO_COLOR` のときはコードの `` ` `` を残します。
画面の下の詳細欄には、選んでいるタスクの全文 (URL も短くせず、リンクの行き先も添えて) とすべてのタグを出します (端末の高さが 16 行未満のときは出しません)。
`gx` はリンクを既定のブラウザなどで開きます。開くのは `http`・`https`・`mailto` のリンクだけです (本文に書かれたファイルのパスなどを、関連付けられたアプリで実行しないため)。

操作のたびに、書き込むコマンドと同じくロックを持ってファイルを読み直してから書くので、表示が古くても Neovim 版の変更を潰しません。
他のツールがファイルを書き換えると、1 秒以内に表示へ反映します。起動したときと日付が変わったときに、日次の整理をします (その日すでに済んでいれば何も書きません)。

## 設定

設定は Neovim 版と共有するファイルに書きます。どちらのツールで使っても同じ結果になるよう、2 つのファイルに分けています。
Neovim 版 (gtodo-md.nvim) も同じファイルを読みます。`gtodo.json` に書いた項目は Neovim 版の `setup()` より優先され、`data_dir` は `setup()` に書いた場所が優先されます (`GTODO_DATA_DIR`・`config.json` と食い違えば Neovim 版が警告します。共有設定が無いときの既定の場所とは比べません)。
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
4. `~/.local/share/gtodo-md` (Windows は `%LOCALAPPDATA%\gtodo-md`) が存在すればそこ
5. Neovim 版の以前の既定 `stdpath("data")/gtodo-md` (通常は `~/.local/share/nvim/gtodo-md`) が存在すればそこ。`NVIM_APPNAME` を設定していればその名前のものを先に、次に既定の名前 (`nvim`) のものを探します
6. それ以外は `~/.local/share/gtodo-md` (Windows は `%LOCALAPPDATA%\gtodo-md`)

Neovim 版も同じ順で決めます。以前の既定にデータがある場合は、移さずにそのまま使えます。4 と 5 の両方があると 4 を使い、5 が使われないことを警告します (4 が 5 へのシンボリックリンクなら同じ場所として扱います)。

環境変数 `XDG_CONFIG_HOME` / `XDG_DATA_HOME` が設定されていれば、どの OS でも `~/.config` / `~/.local/share` の代わりに使います。

### データの設定 (`<データディレクトリ>/gtodo.json`)

セクション名など、ファイルの中身に関わる設定です。データと一緒に置くので、データを別のマシンへ同期しても設定が付いていきます。
ファイルが無ければ、すべて既定値を使います。ただしセクション名は、`sections` が無ければ Neovim 版が最後に起動したときの名前 (`.state.json` に記録されたもの) を使います。Neovim 版の `setup()` で名前を変えている場合に、見出しを既定の名前で書き換えないためです。記録された名前が他のセクションの名前と重なっているなどで使えない場合、書き込むコマンドはエラーになるので、`sections` を設定してください。
CLI も Neovim 版もこのファイルを作らないので、設定を変えるときは手で作ってください。

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

Windows 向けのビルドも同じイメージで確かめられます (MinGW でリンクする `x86_64-pc-windows-gnu` ターゲット)。
Windows でだけ起きるコンパイルエラーや警告 (`#[cfg(unix)]` の付け忘れ、Windows では未使用になる import など) はこれで見つかりますが、テストは Windows 上でないと実行できません。

```sh
scripts/cargo-docker clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
scripts/cargo-docker build --target x86_64-pc-windows-gnu   # target/docker/build/x86_64-pc-windows-gnu/debug/gtodo.exe
```

リンカを使わない作業 (フォーマット・エディタの補完) には、[mise](https://mise.jdx.dev/) で入れたホスト側の Rust を使います。
Rust のバージョンは `mise.toml` と `docker/dev.Dockerfile` で揃えています。

```sh
mise install
cargo fmt --all -- --check
```

ホストに C リンカ (Ubuntu では `build-essential`) があれば、通常どおり `cargo test` も使えます。

GitHub Actions (`.github/workflows/ci.yml`) は、`main` への push と pull request のたびに Linux と Windows でテストと clippy を、Linux で fmt を実行します。

## ライセンス

MIT License
