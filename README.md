# gtodo-md

GTD 志向の Markdown Todo 管理ツール [gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) の TUI/CLI 版です。
Neovim 版と同じファイル (`inbox.md` / `todo.md` / `done.md` / `cancelled.md` / `projects/*.md` / `templates/*.md`) を読み書きし、同じデータを同時に使えます。
コマンドラインで使う CLI と、端末の中で操作する TUI があります。

各版の変更点は [CHANGELOG.md](CHANGELOG.md)、今後の予定と決めたことは [ROADMAP.md](ROADMAP.md) にあります。

## インストール

### GitHub Releases から

[Releases](https://github.com/Nepenthes-1123/gtodo-md/releases) から OS と CPU に合うファイルを取り、中の `gtodo` (Windows は `gtodo.exe`) を `PATH` の通った場所に置きます。

| OS | ファイル |
| --- | --- |
| Linux (x86_64) | `gtodo-v<版>-x86_64-unknown-linux-musl.tar.gz` |
| Linux (ARM64) | `gtodo-v<版>-aarch64-unknown-linux-musl.tar.gz` |
| macOS (Apple シリコン) | `gtodo-v<版>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `gtodo-v<版>-x86_64-apple-darwin.tar.gz` |
| Windows (x86_64) | `gtodo-v<版>-x86_64-pc-windows-msvc.zip` |

Linux 版は静的にリンクしているので、ディストリビューションを選びません。ダウンロードしたファイルは、同じリリースの `SHA256SUMS` で確かめられます (`sha256sum -c SHA256SUMS --ignore-missing`)。
GitHub Actions でこのリポジトリから作ったものであることは、`gh attestation verify <ファイル> --repo Nepenthes-1123/gtodo-md` で確かめられます。
macOS 版は署名していないので、ブラウザで取ったファイルは開けないことがあります。そのときは `xattr -d com.apple.quarantine gtodo` で検疫の印を外してください。

### mise で

[mise](https://mise.jdx.dev/) を使っていれば、GitHub Releases から入れて `PATH` に通せます。

```sh
mise use -g github:Nepenthes-1123/gtodo-md          # 最新の版
mise use -g github:Nepenthes-1123/gtodo-md@0.1.0    # 版を固定する
```

mise は既定で、公開から 24 時間たっていない版を入れません (`minimum_release_age`)。出たばかりの版は、`@0.1.0` のように版を指定して入れてください。
入れるときに、上の attestations も mise が確かめます。

### ソースから

Rust (1.88 以降) と C のリンカがあれば、ソースから入れられます。

```sh
cargo install --locked --git https://github.com/Nepenthes-1123/gtodo-md gtodo
```

## はじめに

- Neovim 版と一緒に使うなら、gtodo-md.nvim v0.3.1 以降にしてください (データディレクトリの場所とセクション名を、同じ設定ファイルから読む版です)。
- データディレクトリは、Neovim 版と同じ規則で決まります (下記の「設定」)。Neovim 版を使っていれば、何も設定しなくても同じ場所を見ます。
- gtodo はデータディレクトリを作りません。初めて使うときは、作ってから始めてください (`mkdir -p ~/.local/share/gtodo-md`。中のファイルは最初に書き込むときに、Neovim 版と同じ雛形で作られます)。

```sh
gtodo                         # TUI を開く (? でキーの一覧)
gtodo add 牛乳を買う +home     # inbox.md へ追加
gtodo list                    # inbox.md と todo.md のタスク
```

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
gtodo split a1b2 -p launch 設計 "実装 due:fri"   # タスクをサブタスクに分割する (-p でプロジェクトへ昇格)
gtodo project list            # プロジェクトと進捗 (--archived でアーカイブしたもの)
gtodo project show site       # プロジェクトの項目・進捗・未完了のタスク
gtodo project new site        # projects/site.md を作る (archive / restore でアーカイブ・復元)
gtodo template list           # テンプレート (--archived でアーカイブしたもの)
gtodo template new weekly     # templates/weekly.md を作る (archive / restore でアーカイブ・復元)
gtodo template insert weekly --base mon --set project=site --set who=田中   # inbox.md へ足す
gtodo completions zsh         # シェルの補完 (bash | zsh | fish | powershell | elvish)
```

`add` の本文に書いた `+project`・`@context`・`due:`・`wait:`・先頭の `(A)` は、タスク行と同じくタグとして読みます。`--due` などのオプションを付けた場合はそちらを優先します。
追加したタスクの ID を表示します。inbox へ追加したタスクの期日がすでに来ていれば、Neovim 版と同じく Today へ移します (`auto_move_inbox_to_today` が `true` のとき)。

`search` の語は、本文に含まれる語 (一部だけでも当たり、大文字小文字と全角半角は区別しない)、`+project`・`@context`・`wait:誰` (タグの値と完全一致) です。すべての語に当たるタスクを出します。対象は inbox.md・todo.md・done.md で、タグや日付 (`created:` など) の文字は探しません。TUI の検索と同じ規則です。

`split` はタスクを、それを引き継いだサブタスクで置き換えます (Neovim 版の分割と同じく、元のタスクは消えて同じ位置にサブタスクが並びます)。サブタスクは 1 つずつ引数で渡し、`add` と同じく本文の `+project`・`due:` なども読みます。書かなかったプロジェクト・コンテキスト・期日・待ち先・作成日は元のタスクから引き継ぎます (優先度は引き継ぎません)。
`-p` (`--project`) でサブタスクのプロジェクトを決めます (プロジェクトへの昇格。ファイルが無ければ作ります)。省くと元のタスクのプロジェクトのまま、`-p ""` で外します。完了したタスクは分割できません。

`project` はプロジェクトファイル (`projects/<名前>.md`、Neovim 版と同じ形) を扱います。`list` は進行中のプロジェクトを、更新した順に進捗 (`[███░░░░░░░] 30% (3/10 done)`) と一緒に並べます。進捗は Neovim 版と同じく、done.md と inbox.md・todo.md の完了したタスクを完了、inbox.md・todo.md の未完了のタスクを未完了として数えます (cancelled.md は数えません)。
`new` は Neovim 版と同じ雛形でファイルを作ります (すでにあれば何もせずエラー)。`archive` / `restore` はフロントマターの `status:` を `archived` / `active` に書き換えます。プロジェクトの名前はファイル名にできるものだけです (`/` や空白を含む名前は、直した名前を添えてエラーにします)。タスクに `+名前` を書いただけではファイルは作られません (Neovim 版と同じ)。

`template` はテンプレート (`templates/<名前>.md`、Neovim 版と同じ形) を扱います。名前は英数字・`-`・`_` だけです。テンプレートに書いたタスク行を、`insert` で inbox.md の末尾へ足します (`id:`・`created:`・完了の記録は引き継がず、新しい ID を付けます)。`due:+3d` などの相対的な期日は `--base` の日付 (既定は今日) から決めます。
`{{project}}`・`{{context}}` はタグ (`+値`・`@値`) として、それ以外の `{{名前}}` は値をそのまま埋めます。テンプレートにあるプレースホルダーには、`--set 名前=値` で値を渡してください (空の値なら、`{{project}}`・`{{context}}` はタグごと消えます)。渡さなかった値は、テンプレートの先頭の `values:` (下記) に書いた値を使います。足した後は `add` と同じく、期日が来たタスクを Today へ移します。
`new` は Neovim 版と同じ記法のメモ (コメント) の入ったファイルを作ります。`archive` は `templates/archive/` へ動かし、`restore` で戻します (行き先に同じ名前があれば動かしません)。

### 定期タスク

テンプレートの先頭にフロントマターを書くと、決まった日にそのタスクを自動で inbox.md へ足します (Neovim 版と同じ書き方・同じ規則です)。

```markdown
---
type: recurring
repeat: mon, thu
values:
  project: site
---
- [ ] 週報を書く {{project}} due:+1d
```

`repeat:` には次の項目を `,` で区切って並べます。どれか 1 つに当たる日に足します。

| 項目 | 足す日 |
| --- | --- |
| `mon` 〜 `sun` | その曜日 |
| `1` 〜 `31` | 毎月その日 (その日が無い月は月末) |
| `end` | 毎月の月末 |
| `1st tue` 〜 `4th tue`、`last fri` | 毎月の第 1〜第 4・最終のその曜日 |
| `04-01` | 毎年その日 (`02-29` は閏年でない年は 2/28) |

- 足すのは日次の整理 (その日に書き込むコマンドか TUI が最初に動いたとき) です。期日が今日のタスクは、その場で Today へ移します。相対の期日は足すはずだった日から数えます。
- しばらくどちらのツールも使わなかった後は、逃した回のうち最後の 1 回分だけを足します。作ったばかり・復元したばかりのテンプレートは、それより前の回をさかのぼって足しません。
- `{{名前}}` は `values:` の値で埋めます。値の無い名前があるテンプレートは、警告して足しません。
- `template list` (と TUI のテンプレート画面) に、`repeat:` と次に足す日、読めない指定ならその理由を出します。アーカイブすると足さなくなります。
- テンプレートごとに、どの日まで確かめたかを `.state.json` の `recurring` に記録します (Neovim 版と共有します)。

`list`・`search`・`stats`・`project list`・`project show`・`template list` は `--json` で JSON を出力します。

`completions` はシェルの補完のスクリプトを出力します。コマンドとオプションを補完します (タスクのキーは補完しません)。たとえば bash なら `gtodo completions bash > ~/.local/share/bash-completion/completions/gtodo`、zsh なら `$fpath` にあるディレクトリへ `_gtodo` という名前で、fish なら `~/.config/fish/completions/gtodo.fish` に置きます。データディレクトリが無くても動きます。
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

`gtodo` を引数なしで実行すると (`gtodo tui` でも) TUI が開きます。セクションごとの一覧、タスクの操作、追加と編集、Queue、検索、カンバン、本文の markdown の表示、`$EDITOR` でファイルを直接編集する機能があります。キーは `tui.toml` で変えられます (下記の「TUI のキー」)。

| キー | 動作 |
| --- | --- |
| `j` / `k` (`↓` / `↑`) | 上下に移る。`gg` / `G` で先頭・末尾、`Ctrl-d` / `Ctrl-u` で半ページ、`Ctrl-f` / `Ctrl-b` で 1 ページ |
| `h` / `l` (`gT` / `gt`、`←` / `→`)、`1`〜`7` | タブを切り替える (Inbox・Today・Next・Waiting・Someday・Done・Cancelled) |
| `x` | 完了と未完了を切り替える |
| `d` / `n` / `w` / `s` | Today・Next・Waiting・Someday へ移す。`w` は待ち先を尋ねる (Waiting にあるタスクでは待ち先を変える。空にすると消す) |
| `c` | キャンセルする (確認あり) |
| `a` / `e` | タスクを追加する / 編集する (下記のフォーム) |
| `p` | タスクを分割する (下記) |
| `Q` | Queue を開く (下記) |
| `/` | 検索 (下記) |
| `K` | カンバン (下記) |
| `gx` | 選んでいるタスクのリンクを開く (複数あれば番号で選ぶ) |
| `o` | 期日が来たタスクを Today へ移し、並べ替える (`gtodo sort` と同じ) |
| `E` | ファイルを `$EDITOR` で開く (下記) |
| `P` | プロジェクト画面 (下記) |
| `T` | テンプレート画面 (下記) |
| `?` | ヘルプ |
| `q` / `Ctrl-c` | 終了 |
| `Esc` | 押しかけのキー (`g` の後など)・入力・確認を取り消す。終了はしない |

最下段には、選んでいるタスクで押したときに起きることを出します。Done と Cancelled のタブのタスクは操作できません (`E` でファイルを手で直すことはできます)。
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

`E` は、選んでいるタスクのファイル (タスクが無ければ開いているタブのファイル) を `$VISUAL`・`$EDITOR` の順に探したエディタで開きます。どちらも無ければ `vi` (Windows は `notepad`) です。vim・Neovim・nano・emacs・micro・kak・helix などは、選んでいるタスクの行から開きます。`code --wait` のように引数も書けます (閉じるまで待つ指定の要るエディタでは、それを付けてください)。Unix ではシェル (`sh`) で解釈します。Windows ではシェルを通さずに空白で区切り (空白を含むパスは `"` で囲む)、拡張子まで書いたものを起動します (`code.cmd --wait` のように)。Done と Cancelled のファイルも手で直せます。
開くのは一時ディレクトリに作ったコピーです。エディタを閉じると、Neovim 版で保存したときと同じ検証 (todo.md の 4 つのセクションの見出し、inbox.md・done.md・cancelled.md のトップヘッダー、開いたときにあった `## YYYY-MM` の見出し) をしてから、内容をそのまま書き戻します。並べ替えや ID の付与はしません。
検証に通らなければ書かずに、`y` でもう一度編集するか、`n` (`Esc`) で編集した内容を捨てるかを尋ねます。編集している間に Neovim 版などがそのファイルを書き換えていたら、相手の変更を潰さないよう書かずに、編集したコピーの場所を知らせます (TUI を終えた後にも標準エラーへ出します)。エディタがエラーで終わったとき (vim の `:cq` など) も書きません。

`p` はタスクを分割します。最下段でサブタスクのプロジェクトを尋ね (初期値は元のタスクのプロジェクト。空にすると外します)、サブタスクを書くファイルを `$EDITOR` で開きます (Neovim 版のスクラッチバッファの代わり)。1 行に 1 つ `- [ ] ` で書いて保存すると、`gtodo split` と同じく元のタスクを置き換えます。`<!-- -->` のコメントと空行は無視し、タスク行として読めない行があればもう一度編集するかを尋ねます。何も書かなければ分割しません。

`P` はプロジェクト画面を開きます。進行中のプロジェクトを `gtodo project list` と同じ進捗のバーと一緒に並べ、ファイルが無いが未完了のタスクに使われている名前も「ファイルなし」として後ろに出します。一覧などでタスクを選んで押すと、そのタスクのプロジェクトを選んだ状態で開きます。
`Enter` でそのプロジェクトのタスクを検索で開き (`+名前`)、`E` でプロジェクトファイルを `$EDITOR` で開きます (無ければ作ってから。保存する前に Neovim 版と同じくフロントマターの項目・`tag` と ファイル名の一致・`created` を変えていないことを確かめます)。`a` で新しいプロジェクトを作って開き、`X` でアーカイブ、`Tab` でアーカイブしたものの表示に切り替えます (そこで `X` を押すと復元)。追加・編集フォームのプロジェクトの候補には、アーカイブしたプロジェクトを出しません。

`T` はテンプレート画面を開きます。`Enter` で選んでいるテンプレートのタスクを inbox.md へ足します (プレースホルダーの値と基準日を、Neovim 版と同じく最下段で 1 つずつ尋ねます。`values:` に書いた値は初めから入っています)。定期タスクには `repeat:` と次に足す日を出します。`E` でテンプレートを `$EDITOR` で開き、`a` で新しいテンプレートを作って開きます (すでにある名前なら、それを開きます)。`X` でアーカイブし、`Tab` でアーカイブしたものの表示に切り替えます (そこで `X` を押すと復元)。

操作のたびに、書き込むコマンドと同じくロックを持ってファイルを読み直してから書くので、表示が古くても Neovim 版の変更を潰しません。
Waiting のタスクで期日が 2 日後まで (過ぎたものを含む) のものは、起動したときと日付が変わったとき、新しく該当したときに知らせます (Neovim 版と同じ条件)。
他のツールがファイルを書き換えると、1 秒以内に表示へ反映します。起動したときと日付が変わったときに、日次の整理をします (その日すでに済んでいれば何も書きません)。

## 設定

設定は Neovim 版と共有するファイルに書きます。どちらのツールで使っても同じ結果になるよう、2 つのファイルに分けています。
Neovim 版 (gtodo-md.nvim) も同じファイルを読みます。`gtodo.json` に書いた項目は Neovim 版の `setup()` より優先され、`data_dir` は `setup()` に書いた場所が優先されます (`GTODO_DATA_DIR`・`config.json` と食い違えば Neovim 版が警告します。共有設定が無いときの既定の場所とは比べません)。
キーマップや見た目など、ツールごとの設定はここには含めません (TUI のキーは下記の `tui.toml`)。

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

### TUI のキー (`tui.toml`)

TUI のキーの割り当てを変えるファイルです。TUI だけの設定なので、Neovim 版とは共有しません。`config.json` と同じディレクトリ (`$XDG_CONFIG_HOME/gtodo-md/tui.toml`、Windows は `%APPDATA%\gtodo-md\tui.toml`。`GTODO_CONFIG` を指定していればその隣) に置きます。無ければ既定のキーを使います。

```toml
[keys]
down = ["j", "<Down>", "<C-n>"]   # 複数のキーは配列で
toggle_done = "<Space>"          # 1 つなら文字列で
cancel = []                      # 空にすると割り当てを外す
```

書いた動作だけ、既定のキーを置き換えます。キーは vim と同じ書き方です: 1 文字はそのまま (`x`・`G`・`?`)、特別なキーは `<Down>`・`<Enter>`・`<Tab>`・`<S-Tab>`・`<Space>`・`<Home>`・`<PageDown>`・`<F1>` など、修飾は `<C-d>` (Ctrl)・`<A-x>` (Alt)、`<` は `<lt>`。並べると続けて押すキーになります (`gg`・`g<Down>`)。
`Esc` (取り消す・一覧へ戻る) と `Ctrl-c` (終了) は割り当てられません。入力欄の中のキーも変えられません。

同じキーを 2 つの動作に割り当てるとエラーになります。続けて押すキーは待ち時間を設けずに読むので、あるキーが別のキー列の途中になる割り当て (`g` と `gg` など) もエラーです。どちらも TUI を起動したときに、どこが誤りかを知らせて止まります。知らない名前は警告して無視します。

| 名前 | 既定のキー | 動作 |
| --- | --- | --- |
| `down` / `up` | `j` `<Down>` / `k` `<Up>` | 下へ / 上へ |
| `top` / `bottom` | `gg` `<Home>` / `G` `<End>` | 先頭へ / 末尾へ |
| `half_page_down` / `half_page_up` | `<C-d>` / `<C-u>` | 半ページ下へ / 上へ |
| `page_down` / `page_up` | `<C-f>` `<PageDown>` / `<C-b>` `<PageUp>` | 1 ページ下へ / 上へ |
| `next_tab` / `prev_tab` | `gt` `l` `<Right>` / `gT` `h` `<Left>` | 次のタブ / 前のタブ (カンバンでは列) |
| `tab_1` 〜 `tab_7` | `1` 〜 `7` | 番号のタブへ |
| `toggle_done` | `x` | 完了と未完了を切り替える |
| `move_today` / `move_next` / `move_waiting` / `move_someday` | `d` / `n` / `w` / `s` | 各セクションへ移す |
| `cancel` | `c` | キャンセルする |
| `add` / `edit` | `a` / `e` | 追加する / 編集する (プロジェクト・テンプレート画面の `add` は新しく作る) |
| `split` | `p` | 分割する |
| `sort` | `o` | 期日のチェックと並べ替え |
| `queue` / `kanban` / `search` | `Q` / `K` / `/` | Queue・カンバン・検索を開く |
| `projects` / `templates` | `P` / `T` | プロジェクト画面・テンプレート画面を開く |
| `open_link` | `gx` | リンクを開く |
| `switch_view` | `<Tab>` | 表示を切り替える (Queue は期日別と待ち先別、検索は状態) |
| `open` | `<Enter>` | Queue・検索・カンバンで選んでいるタスクを一覧で開く (プロジェクト画面ではそのプロジェクトのタスクを検索し、テンプレート画面では挿入する) |
| `edit_file` | `E` | ファイルを `$EDITOR` で開く |
| `archive` | `X` | プロジェクト・テンプレート画面でアーカイブする・復元する |
| `help` / `quit` | `?` / `q` | ヘルプ / 終了 |

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

GitHub Actions (`.github/workflows/ci.yml`) は、`main` への push と pull request のたびに Linux・macOS・Windows でテストと clippy を、Linux で fmt を実行します。

### リリース

1. `crates/gtodo/Cargo.toml` と `crates/gtodo-core/Cargo.toml` の `version` を上げ、`CHANGELOG.md` に `## v<版> (<日付>)` の節を書いて `main` へ入れる
2. GitHub の Actions の画面で Release (`.github/workflows/release.yml`) を選び、「Run workflow」で `main` を選んで、作るタグ (`v0.2.0` など) を入れて実行する。手元のクローンは要りません。ビルドがすべて通った後に、ワークフローが `main` の先頭へ `v<版>` のタグを付けます (手元から `git push origin v0.2.0` でタグを push しても同じように動きます)
3. ワークフローが各 OS のバイナリ (`gtodo-v<版>-<ターゲット>.tar.gz`、Windows は `.zip`)・`SHA256SUMS`・attestations を作り、`Release v<版>` という下書きのリリースに置く。リリースノートは `CHANGELOG.md` のその版の節。確かめてから公開する

タグと `Cargo.toml` の版が食い違うとき、画面から `main` 以外で実行したとき、そのタグが既にあるときは、ビルドの前に止まります。タグを入れずに画面から実行したときと、`release.yml` を変えた pull request では、リリースを作らずにビルドだけを試します。

## ライセンス

MIT License
