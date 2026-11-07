# ビルド・テスト用の環境。ホストに C リンカを入れずに済ませるために使う (scripts/cargo-docker)。
# RUST_VERSION は mise.toml と揃えること。
ARG RUST_VERSION=1.99.1
FROM rust:${RUST_VERSION}-slim
RUN rustup component add clippy rustfmt
