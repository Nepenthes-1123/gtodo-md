# ビルド・テスト用の環境。ホストに C リンカを入れずに済ませるために使う (scripts/cargo-docker)。
# RUST_VERSION は mise.toml と揃えること。
ARG RUST_VERSION=1.99.1
FROM rust:${RUST_VERSION}-slim
RUN rustup component add clippy rustfmt
# Windows 向けのクロスコンパイル用。MSVC ターゲットは Microsoft の SDK を要するため、MinGW でリンクする GNU ターゲットを使う。
# MinGW はスレッドモデル違いの 2 版を持つ。Rust の標準ライブラリはスレッドに Windows API を直接使うので、
# リンクに使うだけならどちらでもよく、両方を入れるメタパッケージ (gcc-mingw-w64-x86-64) より約 200MB 小さい win32 版だけを入れる。
RUN rustup target add x86_64-pc-windows-gnu
RUN apt-get update \
	&& apt-get install --yes --no-install-recommends gcc-mingw-w64-x86-64-win32 \
	&& rm -rf /var/lib/apt/lists/*
