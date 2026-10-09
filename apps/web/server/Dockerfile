# sai-tree の研究プロジェクト用イメージ（共有契約 C7）。
#
# 中に入る物: sai-link・sai-paper・sai-sci（Linux 版）・sai-ts（時系列の道具・段階 B）・
# Claude Code・Python・cc のフック。docling と pdf2zh は入れない（サーバでは動かさない）。
#
# ビルドはリポジトリの直下を文脈にする（その機械の上で作る。arm64 と amd64 の違いを避ける）:
#   docker build -f apps/web/server/Dockerfile -t sai-tree:latest .
#
# sai-link は cc のフックの場所を、ビルドしたときのソースの場所
# （`CARGO_MANIFEST_DIR` = /opt/sai-tree/crates/sai-link-engine）から決める
# （crates/sai-link-engine/src/cc_command.rs の web_hooks_dir）。
# ビルドと実行で同じ /opt/sai-tree を使い、この 2 つが解決できるようにする。

# ---- 1. Rust の実行ファイルを作る ----
FROM rust:1-bookworm AS build
WORKDIR /opt/sai-tree
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY waggle ./waggle
COPY sai-sci ./sai-sci
# 依存のダウンロードとビルド生成物は BuildKit のキャッシュに残す（2 回目から差分だけ）。
# キャッシュの中の target はイメージに入らないので、実行ファイルは同じ RUN の中で取り出す。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/opt/sai-tree/target \
    cargo build --release -p sai-link-cli -p sai-paper-cli -p sai-sci-cli \
    && mkdir -p /out \
    && cp target/release/sai-link target/release/sai-paper target/release/sai-sci /out/

# ---- 2. 実行する環境 ----
FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-venv git ca-certificates curl ripgrep jq tini procps tmux \
    && rm -rf /var/lib/apt/lists/*
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv
RUN npm install -g @anthropic-ai/claude-code
COPY --from=build /out/sai-link /out/sai-paper /out/sai-sci /usr/local/bin/
# cc のフックを、ビルドしたときと同じ場所（/opt/sai-tree）に置く。
# sai-link は `CARGO_MANIFEST_DIR`（= /opt/sai-tree/crates/sai-link-engine）から
# `../../apps/web/...` をたどって場所を決める（cc_command.rs の web_hooks_dir）。
# `canonicalize()` が通るよう、その crate のフォルダを実在させる。
RUN mkdir -p /opt/sai-tree/crates/sai-link-engine
COPY apps/web/hooks /opt/sai-tree/apps/web/hooks
# sai_web の全体を置く（sai-web の CLI が `sai_web.webcli` を読むため）。
COPY apps/web/src/sai_web /opt/sai-tree/apps/web/src/sai_web
# コンテナの agent が質問を読んで答えるための sai-web（内部の合言葉で `/api/questions/mine` などを呼ぶ）。
# fastapi を入れない python3 で動くよう、標準ライブラリだけの webcli を PYTHONPATH で読む。
RUN printf '#!/bin/sh\nPYTHONPATH=/opt/sai-tree/apps/web/src exec python3 -m sai_web.webcli "$@"\n' > /usr/local/bin/sai-web \
    && chmod 755 /usr/local/bin/sai-web
# sai-sci の研究の部品（tools・exec・blocks・template）は実行ファイルに埋め込まれている
# （crates/sai-sci-cli/src/agent_assets.rs の include_str!）。元のフォルダは参照用に置く。
COPY sai-sci /opt/sai-tree/sai-sci
# 時系列の道具 sai-ts（段階 B）。プロジェクトのコンテナの中で動き、sai-web の ts 入口が中継する。
# 仮想環境はイメージの中で作る（.dockerignore で持ち込まない）。Python は bookworm の python3 を使う。
COPY apps/ts /opt/sai-tree/apps/ts
RUN cd /opt/sai-tree/apps/ts && UV_PYTHON_DOWNLOADS=never uv sync --frozen --no-dev \
    && ln -s /opt/sai-tree/apps/ts/bin/sai-ts /usr/local/bin/sai-ts
ENV SAI_CLAUDE_BIN=claude
ENTRYPOINT ["/usr/bin/tini", "--"]
# プロジェクトの準備（.sai）ができていれば常駐を起こし、あとは眠ってコンテナを生かしておく。
CMD ["sh", "-c", "[ -d .sai ] && sai-link daemon start; exec sleep infinity"]
