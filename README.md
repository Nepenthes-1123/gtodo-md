# gtodo-md

GTD 志向の Markdown Todo 管理ツール [gtodo-md.nvim](https://github.com/Nepenthes-1123/gtodo-md.nvim) の TUI/CLI 版です。
Neovim 版と同じファイル (`inbox.md` / `todo.md` / `done.md` / `cancelled.md` / `projects/*.md`) を読み書きし、同じデータを同時に使えることを目指しています。

開発中です。進捗は [ROADMAP.md](ROADMAP.md) を参照してください。現在は読み取り系のコマンドだけが使えます。

## 使い方

```sh
gtodo list                    # inbox.md と todo.md のタスク
gtodo list today              # inbox | todo | today | next | waiting | someday | done | cancelled
```

`list` は `--json` で JSON を出力します。

読み取り系のコマンドはファイルを一切書き換えません。Neovim 版で編集中のファイルがあっても安全に使えます。

## 設定

設定は Neovim 版と共有するファイルに書きます。
ただし、Neovim 版はまだこのファイルを読みません ([ROADMAP.md](ROADMAP.md) の「gtodo-md.nvim 側の対応」)。それまでは Neovim 版の `setup()` にも同じ値を書いてください。
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
