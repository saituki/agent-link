# 実装計画書 U08: コンテナのイメージ・CLI の中継・rootless Docker の入れ方

## 目的
1. 研究プロジェクト用のコンテナのイメージ（`sai-tree:latest`）を作る Dockerfile を用意する。中に `sai-link`・`sai-paper`・`sai-sci`（Linux 版）・Claude Code・pi・Python・cc のフック・pi の質問の拡張が入る。docling と pdf2zh は入れない。
2. Web がコンテナの中の CLI を呼ぶための中継のスクリプト（`sai-in-container`）を用意する。Web の `SAI_LINK_BIN` などをこれに向けるだけで、Web の CLI の呼び出し（`cli.py`・`control.py`・`papers.py`・`sci.py`・`commands.py`）を変えずに、すべてプロジェクトのコンテナの中で動く。
3. Linux PC へのサーバの入れ方（rootless Docker・cgroup の委任・Tailscale）を `docs/server.md` に書き、systemd のひな形を置く。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。sandbox は rootless Docker で、1 プロジェクトに 1 コンテナを割り当てる（要件）。
- Web は CLI を「引数の配列・env に `SAI_PROJECT_ROOT` と `SAI_ORIGIN`・cwd に root」で呼ぶ（`ARCHITECTURE.md` §4.2、`apps/web/src/sai_web/control.py:236-247` `_phone_env`、`papers.py:136`）。実行ファイルの場所は `SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN`（`apps/web/src/sai_web/config.py:58-66`）。
- 置き場（共有契約 C4）は、ホストとコンテナで**同じパス**にする。こうすると、コンテナが書く記録のパス（`transcript_path`・`project_root`）をホストの Web がそのまま読める。コンテナに見せるのは、そのプロジェクトの root と、プロジェクト専用の HOME の 2 つだけ。
- rootless Docker では、コンテナの root がホストの利用者（Web を動かす `sai`）に写る。コンテナを root で動かすと、bind したフォルダのファイルの持ち主は `sai` のままになり、ホストの Web とコンテナが同じファイルを読み書きできる。`--user` を付けると subuid に写り、ホストの Web が書けなくなる（調査 R3 §Q2。`docs/plan/research/R3-rootless-docker.md`）。
- rootless にはホストの `docker0` が無い。`--add-host host.docker.internal:host-gateway` はホストの LAN の住所に解決されることが多く、ホストの `127.0.0.1` だけで待ち受ける Web には届かない。Web は `0.0.0.0:8200` で待ち受け、防火壁で tailnet とコンテナ以外を止める（調査 R3 §Q4）。
- rootless で `--memory`・`--cpus`・`--pids-limit` を効かせるには、cgroup v2 の委任（`Delegate=`）が要る（調査 R3 §Q3）。
- `sai-link` は cc のフックと pi の拡張の場所を、ビルドしたときのソースの場所から決める（`crates/sai-link-engine/src/cc_command.rs:386-393` `web_hooks_dir` は `SAI_WEB_HOOKS_DIR` か `CARGO_MANIFEST_DIR/../../apps/web/hooks`、`cc_command.rs:410-419` `pi_question_extension` は `CARGO_MANIFEST_DIR/../../apps/web/src/sai_web/extensions/ask_user.ts`）。イメージではビルドと実行で同じ場所（`/opt/sai-tree`）を使い、この 2 つが解決できるようにする。
- `sai-sci` は md の部品を `include_str!` で実行ファイルに埋め込む（`crates/sai-sci-cli/src/agent_assets.rs:31`）。

## 前提（着手時点のコードの状態）
- ガイド 1〜3 が終わっている（headless・Web の herdr の削除・多人数モードの置き場 C4）。
- Mac に Docker がある（arm64）。サーバの Linux PC は amd64 の見込み（要確認）。どちらでもビルドできる Dockerfile にする。

## 変更内容

### 1. `apps/web/server/Dockerfile`（新規。リポジトリの直下を文脈にしてビルドする）
```dockerfile
# ---- 1. Rust の実行ファイルを作る ----
FROM rust:1-bookworm AS build
WORKDIR /opt/sai-tree
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY waggle ./waggle
COPY sai-sci ./sai-sci
RUN cargo build --release -p sai-link-cli -p sai-paper-cli -p sai-sci-cli

# ---- 2. 実行する環境 ----
FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-venv git ca-certificates curl ripgrep jq tini \
    && rm -rf /var/lib/apt/lists/*
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv
RUN npm install -g @anthropic-ai/claude-code @earendil-works/pi-coding-agent
COPY --from=build /opt/sai-tree/target/release/sai-link /opt/sai-tree/target/release/sai-paper /opt/sai-tree/target/release/sai-sci /usr/local/bin/
# フックと拡張はビルドしたときと同じ場所に置く（cc_command.rs の web_hooks_dir・pi_question_extension）
RUN mkdir -p /opt/sai-tree/crates/sai-link-engine
COPY apps/web/hooks /opt/sai-tree/apps/web/hooks
COPY apps/web/src/sai_web/extensions /opt/sai-tree/apps/web/src/sai_web/extensions
COPY sai-sci /opt/sai-tree/sai-sci
ENV SAI_CLAUDE_BIN=claude SAI_PI_BIN=pi
ENTRYPOINT ["/usr/bin/tini", "--"]
# プロジェクトの準備（.sai）ができていれば常駐を起こし、あとは眠ってコンテナを生かしておく
CMD ["sh", "-c", "[ -d .sai ] && sai-link daemon start; exec sleep infinity"]
```
- 上は形の見本である。ビルドが通るように直してよい（たとえば `sai-paper` が要る OS のライブラリ（`pkg-config`・`libssl-dev` など）を build の段に足す、pi の npm パッケージ名を `npm view` で確かめる）。直した点は作業報告書に書く。
- `sai-sci/tools` を agent が `python3 …/sai-sci/tools/<名前>.py` の形で呼ぶか、埋め込みから書き出すかを `sai-sci` のコードで確かめ、イメージの中で動く場所に置く。
- docling・pdf2zh は入れない（サーバでは動かさない。要件）。
- `.dockerignore`（リポジトリ直下。新規）に `target`・`.sai`・`apps/web/.venv`・`apps/radio/.venv`・`apps/radio/src/data`・`apps/prep`・`.git`・`.worktrees`・`node_modules` を入れる。

### 2. `apps/web/server/sai-in-container`（新規。POSIX sh。実行権を付ける）
同じファイルを `sai-link`・`sai-paper`・`sai-sci` の名前でリンクして使う。呼ばれた名前（`basename "$0"`）をコンテナの中で動かす。
```sh
#!/bin/sh
# Web が CLI を呼ぶとき、プロジェクトのコンテナの中で動かす中継（共有契約 C7）
set -u
bin=$(basename "$0")
image=${SAI_IMAGE:-sai-tree:latest}
if [ -z "${SAI_PROJECT_ROOT:-}" ]; then
  # プロジェクトの要らない命令（schema など）は使い捨てのコンテナで動かす
  exec docker run --rm -i --network none "$image" "$bin" "$@"
fi
name=$(basename "$SAI_PROJECT_ROOT")
case "$name" in
  *[!a-z0-9-]*|-*|"") echo "プロジェクトの名前が正しくありません: $name" >&2; exit 2 ;;
esac
ctr="sai-$name"
if [ "$(docker inspect -f '{{.State.Running}}' "$ctr" 2>/dev/null)" != "true" ]; then
  echo "プロジェクトのコンテナが動いていません（Projects から起動してください）" >&2
  exit 4
fi
exec docker exec -i -w "$SAI_PROJECT_ROOT" \
  -e SAI_PROJECT_ROOT -e SAI_ORIGIN "$ctr" "$bin" "$@"
```
- Web の環境（合言葉・API キー）はコンテナへ渡さない（`-e` は 2 つだけ）。
- 終了コード・標準出力・標準エラーは `docker exec` がそのまま返す（CLI の約束がそのまま Web に届く）。
- `docker` は rootless の socket を使う（`DOCKER_HOST` は systemd のひな形で渡す）。

### 3. `apps/web/server/sai-web.service`（新規。systemd のシステムサービスのひな形）
- `User=sai`・`WorkingDirectory=<リポジトリ>`・`Restart=on-failure`。
- `Environment=`: `SAI_WEB_USERS_ROOT=/srv/sai/users`・`SAI_LINK_BIN=/opt/sai/bin/sai-link`・`SAI_PAPER_BIN=/opt/sai/bin/sai-paper`・`SAI_SCI_BIN=/opt/sai/bin/sai-sci`・`SAI_IMAGE=sai-tree:latest`・`SAI_WEB_NO_WATCH=0`・`DOCKER_HOST=unix:///run/user/<sai の uid>/docker.sock`・`XDG_RUNTIME_DIR=/run/user/<sai の uid>`。
- `ExecStart=<リポジトリ>/apps/web/bin/sai-web serve --host 0.0.0.0 --port 8200`。
- rootless の dockerd は `sai` の user サービスなので、`After=` で順番を付けられない。Web は docker が動いていないとき 503 を返す（U09）ので、起動の順番は問わない。

### 4. `docs/server.md`（新規）— サーバの入れ方の手順書
次の順に、打つ命令をそのまま書く。**エージェントは手順書を書くだけで、利用者の環境を変える命令は実行しない**（AGENTS.md）。出典は調査 R3 の「推奨の設定」。
1. 用意: Ubuntu（cgroup v2。`/sys/fs/cgroup/cgroup.controllers` があること）・git・uv・Tailscale。利用者 `sai` を作る（ログインしない利用者でよい）。`/etc/subuid`・`/etc/subgid` に `sai` の 65536 個以上の割り当てがあることを確かめる。
2. rootless Docker: Docker の deb（`docker-ce`・`docker-ce-rootless-extras`・`uidmap`）を入れ、`sai` で `dockerd-rootless-setuptool.sh install`・`systemctl --user enable --now docker`、root で `loginctl enable-linger sai`。root の dockerd を使わないなら `systemctl disable --now docker.service docker.socket`。
3. cgroup v2 の委任: `/etc/systemd/system/user@.service.d/delegate.conf` に `[Service]` `Delegate=cpu cpuset io memory pids` を書き、`systemctl daemon-reload` して再起動する。確かめ方: `sai` で `cat /sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/cgroup.controllers` に `cpu memory pids` が出る。
4. リポジトリを置き、`sai` で `docker build -f apps/web/server/Dockerfile -t sai-tree:latest .`（その機械の上でビルドする。arm64 と amd64 の違いを避けるため）。
5. `apps/web` で `uv sync`。
6. `sudo mkdir -p /srv/sai/users /opt/sai/bin && sudo chown sai: /srv/sai/users /opt/sai/bin`。`sai-in-container` を `/opt/sai/bin/sai-link`・`sai-paper`・`sai-sci` の 3 つの名前でリンクする。
7. `sai-web.service` の `<…>` を埋めて `/etc/systemd/system/` に置き、`systemctl enable --now sai-web`。
8. 防火壁（ufw）: 8200 番は `tailscale0` と、コンテナから来る通信だけを許し、LAN からは止める。コンテナから届くか確かめる命令: `docker run --rm --add-host host.docker.internal:host-gateway sai-tree:latest curl -s http://host.docker.internal:8200/api/health`。届かなければ、届く住所（`getent hosts host.docker.internal` の値など）を書き留め、U09 の `SAI_WEB_URL_FOR_CONTAINERS` に入れる。
9. `tailscale serve --bg --https=443 http://127.0.0.1:8200`。
10. 最初の管理者: `SAI_WEB_USERS_ROOT=/srv/sai/users apps/web/bin/sai-web user add <短い名前> --email <大学のメールアドレス> --admin`。
11. Radio・MLflow を使うなら、`apps/radio/README.md`・`apps/mlflow-server/README.md` のとおりに同じ機械で動かす（全員で共有する）。
12. 全員に配るひな形: `/srv/sai/skel/` に `.pi/agent/models.json` などを置くと、新しいプロジェクトの HOME に写る（U09 の `SAI_WEB_SKEL_DIR`）。
13. 運用: イメージを作り直したときの入れ替え（`docker rm -f sai-<…>` → 画面から起動し直すとイメージが新しくなる。U09 の作り直し）、プロジェクトを消す手順（`docker rm -f sai-<user>-<name>` と `projects/<…>`・`homes/<…>` の削除）、バックアップする場所（`/srv/sai/users` と Web の置き場 `~sai/.sai/web/`）。
14. 規約の注意: Claude の長命トークン（`claude setup-token`）を Web に預けて使う機能は、Anthropic の規約（開発者が資格情報を保存・仲介しない）に照らして灰色であり、使う人の責任で使うこと（要件定義書の決定）。トークンは各自が自分のアカウントで発行し、他人と共有しない。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C4 多人数モードの置き場**（U06 が決めた。全文）
- `<U>` = `SAI_WEB_USERS_ROOT`。`<U>/<user>/home/`＝その人のひな形の HOME（Web だけが使う）。`<U>/<user>/projects/<user>-<project>/`＝プロジェクトの root。`<U>/<user>/homes/<user>-<project>/`＝そのプロジェクトのコンテナの HOME（`SAI_HOME` は `<HOME>/.sai`）。`<U>/<user>/secrets.env`＝API キーと Claude のトークン（0600。`--env-file` で渡し、ファイルは見せない）。
- 利用者の短い名前 `^[a-z][a-z0-9]{1,15}$`、プロジェクトの短い名前 `^[a-z0-9][a-z0-9-]{0,29}$`。
- コンテナに見せるのは、そのプロジェクトの root と HOME の 2 つだけ（同じパスに）。

**C7 コンテナと中継**（U09 が同じ形でコンテナを作る）
- コンテナの名前は `sai-<プロジェクトの root のフォルダ名>`（＝`sai-<user>-<project>`）。ラベル `sai.owner=<user>`・`sai.project=<user>-<project>`。
- イメージの名前は環境変数 `SAI_IMAGE`（既定 `sai-tree:latest`）。
- コンテナは root で動かす（`--user` を付けない）。作業の場所（`-w`）はプロジェクトの root。
- コンテナの主プロセスは `tini` の下の `sh -c "[ -d .sai ] && sai-link daemon start; exec sleep infinity"`。
- 中継 `sai-in-container` は `SAI_PROJECT_ROOT` と `SAI_ORIGIN` だけを渡し、コンテナが動いていなければ終了コード 4「プロジェクトのコンテナが動いていません（Projects から起動してください）」。

## ファイルの所有権
- 書いてよい: `apps/web/server/`（新規のフォルダ。`Dockerfile`・`sai-in-container`・`sai-web.service`・`test_sai_in_container.sh`）・`.dockerignore`（新規）・`docs/server.md`（新規）
- 読むだけ: `crates/`・`apps/web/src/`・`apps/web/hooks/`
- 触らない: `apps/web/src/sai_web/*.py`（U09・U10 が書く）・`README.md`・`ARCHITECTURE.md`（U15 が書く）

## 完了条件
- [ ] AC1: Mac で `perl -e 'alarm 3600; exec @ARGV' docker build -f apps/web/server/Dockerfile -t sai-tree:check .` が通る。`docker run --rm sai-tree:check sai-link schema --json`・`sai-paper schema --json`・`sai-sci schema --json`・`claude --version`・`pi --version`・`python3 --version` が動く。`docling`・`pdf2zh` は入っていない。
- [ ] AC2: `apps/web/server/test_sai_in_container.sh`（偽の `docker` を PATH の先頭に置き、呼ばれた引数を記録する）: `SAI_PROJECT_ROOT=/srv/sai/users/alice/projects/alice-x` で `sai-link run tree --json` を呼ぶと `docker exec -i -w /srv/sai/users/alice/projects/alice-x -e SAI_PROJECT_ROOT -e SAI_ORIGIN sai-alice-x sai-link run tree --json` になる。名前が `../x` などなら終了コード 2。コンテナが動いていなければ終了コード 4。
- [ ] AC3: 実物の確認（Mac の Docker。ホーム直下の一時フォルダ `~/sai-container-check/` を使い、最後に必ず消す）: 一時フォルダの root と HOME だけを同じパスで bind したコンテナ `sai-check-x`（`sai-check`・`sai-tree:check` の名前は確認用で、最後に `docker rm -f` する）を C7 の形で作り、中継から `sai-link init --name check-x` → `sai-link daemon start` → `sai-link daemon status` が動く。cc のフックが `.claude/settings.json` に入り、そのパスがコンテナの中に実在する。pi の拡張のパスが `.pi/settings.json` に入る。コンテナの中から、一時フォルダの別のフォルダ（bind していない物）が見えない。
- [ ] AC4: `docs/server.md` が上の 14 項目を含み、命令がそのまま打てる形で書いてある。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
sh apps/web/server/test_sai_in_container.sh
perl -e 'alarm 3600; exec @ARGV' docker build -f apps/web/server/Dockerfile -t sai-tree:check .
docker run --rm sai-tree:check sai-link schema --json | head -c 200
```
確認が終わったら `docker rm -f sai-check-x`・`docker rmi sai-tree:check`・`rm -rf ~/sai-container-check` を行う。`apps/mlflow-server` の compose には触らない。

## 決定事項と理由
- CLI の呼び出しは、Web を変えずに実行ファイルの場所（`SAI_*_BIN`）を中継のスクリプトに向けて切り替える — 理由: Web の CLI の呼び出しは 15 か所以上に散っている。中継なら 1 か所で済み、単独モードは何も変わらない。
- ホストとコンテナで同じパスを使う — 理由: 記録のパスを読み替えずに Web が読める。
- コンテナは root で動かす — 理由: rootless では root がホストの `sai` に写り、ファイルの持ち主がそろう（調査 R3）。Claude Code の root の制限は、U01 の許可の一覧で避ける。
- Web は `0.0.0.0:8200` で待ち受け、防火壁で止める — 理由: rootless のコンテナから、ホストの `127.0.0.1` には届かない（調査 R3 §Q4）。`disable_host_loopback` は既定（true）のままにして、コンテナからホストの 127.0.0.1 の全部の口に届かないようにする。
- 使い捨ての `schema` のコンテナは `--network none` — 理由: ネットワークが要らない命令に外への口を与えない。
- フックと拡張はビルドと同じ場所（`/opt/sai-tree`）に置く — 理由: `sai-link` のコードを変えずに場所が解決する。

## 要確認
- サーバの Linux PC の CPU（amd64 か arm64）。どちらでもビルドできるようにしておき、手順書に「その機械の上でビルドする」と書く。
- pi の npm のパッケージ名（`@earendil-works/pi-coding-agent` の見込み。Mac の `/opt/homebrew/lib/node_modules/` の名前で確かめる）。
- rootless のコンテナから `host.docker.internal:8200` に届くか（Linux の実機で利用者が確かめる。手順書の 8）。
