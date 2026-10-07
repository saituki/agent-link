---
block: web-research
run: 445
description: 調査報告書 R3: rootless Docker でプロジェクトのコンテナを動かす注意と、コンテナの中の Claude Code のログイン
---

# 調査報告書 R3: rootless Docker でプロジェクトのコンテナを動かす注意と、コンテナの中の Claude Code のログイン

- 確かめた日: 2026-10-07
- 前提として読んだ版: Docker Engine 29.x 系（Docker 公式の rootless 文書・RootlessKit 文書）、Claude Code 2.1.x 系。Claude Code の一部の記事は v2.1.280 を基準に書かれている。記事に版が書かれていない事実には「（版不問の文書）」と付ける。
- この報告書で「（事実）」は出典を読んで確かめたこと、「（推測）」は出典から導いたが直接は書かれていないこと、「（要確認）」はこのサーバで実際に試さないと決まらないことである。

この報告書は次の構成を前提にする。研究室の Linux サーバ（Ubuntu の見込み・amd64 の見込み）で、FastAPI の Web サーバがホストで systemd のシステムサービスとして利用者 `sai` で動く。その Web が rootless Docker の CLI（`docker run`・`docker exec`・`docker inspect`）を subprocess で呼び、研究プロジェクトごとにコンテナを 1 つ作る。コンテナの中では Claude Code（`claude -p`）と pi（node）が動き、ホストのフォルダ `/srv/sai/users/<user>` をコンテナの同じパスに bind する。ホストの Web もそのフォルダの JSON・jsonl を直接読む。資源の上限は `--memory 8g --cpus 2 --pids-limit 1024`、`--cap-drop ALL --security-opt no-new-privileges`。Web は Tailscale（`tailscale serve` で HTTPS）で見せる。

---

## Q1. rootless Docker の入れ方と、システムサービスの Web から socket を使う方法

### 入れ方（Ubuntu、利用者 `sai` で入れる）

（事実）rootless Docker は `uidmap` パッケージの `newuidmap`・`newgidmap` と、`/etc/subuid`・`/etc/subgid` の 65,536 個以上の subordinate UID/GID が要る。Docker の deb を入れると `docker-ce-rootless-extras` が入り、`/usr/bin/dockerd-rootless-setuptool.sh` が使える。出典: [Docker Rootless mode](https://docs.docker.com/engine/security/rootless/)（2026-10-07 閲覧）。

（事実）root でない利用者として次を実行すると、`~/.config/systemd/user/docker.service` が作られ、CLI の context `rootless` が設定される。出典: 同上。

```
dockerd-rootless-setuptool.sh install
systemctl --user start docker
systemctl --user enable docker
sudo loginctl enable-linger sai
```

（事実）`loginctl enable-linger sai` をしないと、ログアウト時に user サービスが止まり、起動時に自動で立ち上がらない。このとき `systemctl --user` はログイン方法によっては "Failed to connect to bus" で失敗する。`sudo -iu sai` ではなく `pam_systemd` 経由（`machinectl shell sai@` など）でログインする。出典: [Docker Rootless mode Tips](https://docs.docker.com/engine/security/rootless/tips/)、[Docker Rootless mode Troubleshooting](https://docs.docker.com/engine/security/rootless/troubleshoot/)（2026-10-07 閲覧）。

（事実）socket は既定で `$XDG_RUNTIME_DIR/docker.sock`、すなわち `/run/user/<uid>/docker.sock`。rootless の data dir は `~/.local/share/docker`、daemon の設定 dir は `~/.config/docker`（client の `~/.docker` とは別）。出典: Docker Rootless mode Tips（同上）。

（事実）rootless Docker を systemd の**システム**サービス（`/etc/systemd/system/docker.service`）として起動することは `User=` を付けても**サポートされない**。daemon はあくまで `sai` の systemd **user** サービス（`systemctl --user`）で動かす。出典: Docker Rootless mode Tips（同上、原文 "Starting Rootless Docker as a systemd-wide service (/etc/systemd/system/docker.service) is not supported, even with the User= directive."）。

### システムサービスの Web からその socket を使う

（事実）Web は `sai` と同じ利用者で動くので、`/run/user/<uid>/docker.sock` にアクセスできる。Web のシステムサービスに次を環境変数として渡せばよい。

```
[Service]
User=sai
Environment=DOCKER_HOST=unix:///run/user/<uid>/docker.sock
Environment=PATH=/usr/local/bin:/usr/bin:/bin
```

（事実）Docker Engine v23 以降は `dockerd-rootless-setuptool.sh install` が CLI context `rootless` を自動設定するので、`docker context use rootless` でも同じ socket につながる。ただし context は `~/.docker/contexts`（利用者のホーム）に依存するため、**システムサービスには環境変数 `DOCKER_HOST` のほうが確実**（推測）。出典: Docker Rootless mode、[Docker/Moby on Rootless Containers](https://rootlesscontaine.rs/getting-started/docker/)（2026-10-07 閲覧）。

（事実）`DOCKER_HOST` は既定で `unix:///run/user/$UID/docker.sock` と案内されている。出典: Docker Rootless mode（同上）。

（注意・要確認）`dockerd-rootless-setuptool.sh` が作るのは `sai` の user サービスである。システムサービスの Web を起動する前に、`sai` の user manager（`user@<uid>.service`）と `docker.service` が linger で上がっている必要がある。順序は systemd の `After=`/`Requires=` では素直に表せないので、Web 側で socket の有無を待つか、起動順を運用で担保する（要確認）。

---

## Q2. bind mount の所有者と `--user`、`claude` の root 制限

### rootless の UID/GID mapping（事実）

（事実）rootless では、コンテナの UID 0 は「rootless Docker を動かしている利用者のホスト UID」（=`id -u sai`）に写る。コンテナの UID n（n>=1）は `subuid + (n-1)` に写る。GID も同じ。つまり「ホストの `sai` が所有するファイルは、コンテナの中では root が所有しているように見える」。出典: [Docker UID/GID mapping](https://docs.docker.com/engine/security/rootless/uid-gid-mapping/)（2026-10-07 閲覧）。

### コンテナを root（既定）で動かす場合

（事実）コンテナを UID 0 で動かすと、bind mount の中で作るファイルはホストから見て `sai` の所有になる。出典: 同上。→ ホストの Web（`sai` で動く）は読めるし書ける。

（推測）この構成では、`/srv/sai/users/<user>` を `sai` の所有のままにして、コンテナの中のエージェントもホストの Web も同じ実体のファイルを読み書きできる。もっとも素直な構成である。

### `--user <uid>`（非 0）を付ける場合

（事実）`--user <n>`（n>=1）を付けると、そのプロセスのホスト側 UID は `subuid + (n-1)` になる。出典: 同上。つまり、コンテナの中で作ったファイルのホストでの所有者は `sai` ではなく subordinate UID（例: 231072 番台）になる。

（事実）Docker の Forum でも、rootless では「ホストの利用者がコンテナの root になり、コンテナ内のほかの利用者は shifted user id になる」と説明されている。出典: [Bind mounting permissions with USER broken using rootless docker](https://forums.docker.com/t/bind-mounting-permissions-with-user-broken-using-rootless-docker/88257)（2026-10-07 閲覧）。

（推測）`--user 1000` を付けると、`/srv/sai/users/<user>` のディレクトリはホスト `sai` の所有（コンテナの root）のままなので、subuid に写ったプロセスはそのディレクトリに書けない（既定モード 0755 のため）。書けるようにするには、ホスト側で対象ディレクトリを subuid に chown するか、setfacl で subuid に rw を与える必要がある。subuid は `/etc/subuid` の割り当てで変わりうるので、この方法は壊れやすい。

（事実）Docker の `--userns` は `host` しか受け付けない（`--userns=keep-id` は無い）。`--userns=host` は「daemon で user namespace remap を有効にしているときに、そのコンテナだけ remap を切る」ためのもので、rootless では rootless の user namespace を使うことに変わりはない。出典: [docker container run](https://docs.docker.com/reference/cli/docker/container/run/)（2026-10-07 閲覧、原文 "host is the only valid value for the --userns flag."）。

（推測）したがって Docker には Podman の `--userns=keep-id` に当たる「ホストの利用者 UID をそのままコンテナに持って入る」手段が無い。bind mount の所有者を `sai` に保ちたいなら、コンテナを root（UID 0）で動かすのが唯一素直な道である。

### `claude` は root で `--dangerously-skip-permissions` を使えない

（事実）Claude Code は Linux と macOS で、root または sudo で動いていると `--dangerously-skip-permissions`（`--permission-mode bypassPermissions`）を拒否し、`--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons` で終了する。公式の dev container 文書も「`remoteUser` が root でないことを確認せよ」と書く。出典: [Choose a sandbox environment](https://code.claude.com/docs/en/sandbox-environments)、[Development containers](https://code.claude.com/docs/en/devcontainer)（2026-10-07 閲覧）。

（事実）不具合票 #9184（v2.0.10、2025-10-08）でも同じエラーが報告されている。出典: [anthropics/claude-code#9184](https://github.com/anthropics/claude-code/issues/9184)（2026-10-07 閲覧）。

（事実・非公式）不具合票 #58150 は `cli.js` を読んだとして、root 判定が `process.getuid() === 0 && process.env.IS_SANDBOX !== "1" && process.env.CLAUDE_CODE_BUBBLEWRAP !== "1"` であると書いている。つまり `IS_SANDBOX=1` を付けると root でも bypassPermissions で起動できる。出典: [anthropics/claude-code#58150](https://github.com/anthropics/claude-code/issues/58150)（2026-10-07 閲覧）。

（事実・非公式）`IS_SANDBOX=1` は公式文書には無い、版で変わりうる逃げ道である。v2.1.280 を基準にした第三者記事も、同じ判定コードと「未文書・どの版で消えても不思議ではない」と書いている。出典: [claude --dangerously-skip-permissions (2026)](https://www.morphllm.com/claude-code-dangerously-skip-permissions)（2026-10-07 閲覧、一部は非公式）。

（推測）この構成では、Q2 の所有者の話と Q7 の root 制限が正面から衝突する。コンテナを root で動かせば bind mount はきれいに `sai` の所有になるが、`claude -p` を完全自動で回すには `IS_SANDBOX=1`（非公式）に頼ることになる。逆にコンテナを非 root にすれば `claude` は公式の想定どおり動くが、bind mount に書けなくなる。次章の「推奨の設定」で両案を示す。

（代替・事実）root でも `--permission-mode bypassPermissions` **以外**は拒否されない。`--permission-mode acceptEdits` や `auto`、あるいは `--allowedTools` と `permissions.allow` の組み合わせなら root のチェックに当たらない（推測を含む）。公式文書は「root チェックは bypass モードにだけ掛かる」とは明記していないが、dev container 文書は「`--dangerously-skip-permissions` を root で使うな」とだけ書く。出典: [Development containers](https://code.claude.com/docs/en/devcontainer)（同上）。

---

## Q3. 資源の上限（`--memory`・`--cpus`・`--pids-limit`）と cgroup v2 の委任

（事実）rootless で `--cpus`・`--memory`・`--pids-limit` などの cgroup 系の `docker run` フラグが効くのは、次のすべてを満たすときだけである。

- Docker/Moby 20.10 以降、containerd 1.4 以降、runc 1.0-rc91 以降
- ホストが cgroup v2 で動いている
- ホストが systemd で動いている

出典: [Docker/Moby on Rootless Containers](https://rootlesscontaine.rs/getting-started/docker/)、[Docker Rootless mode Tips](https://docs.docker.com/engine/security/rootless/tips/)（2026-10-07 閲覧）。

（事実）`docker info` の `Cgroup Driver` が `systemd` なら条件を満たす。`none` なら満たさず、cgroup 系のフラグは**黙って無視される**。出典: Docker Rootless mode Tips（同上、原文 "rootless mode ignores the cgroup-related docker run flags"）。

（事実）Ubuntu は 21.10 以降 cgroup v2 が既定。cgroup v2 かどうかは `/sys/fs/cgroup/cgroup.controllers` の有無で分かる。委任には systemd 244 以降が推奨。出典: [cgroup v2 on Rootless Containers](https://rootlesscontaine.rs/getting-started/common/cgroup2/)（2026-10-07 閲覧）。

（事実）既定で非 root 利用者に委任されるのは `memory` と `pids` だけである。`cpu`・`cpuset`・`io` も委任するには、root で次のファイルを作る。

```
# /etc/systemd/system/user@.service.d/delegate.conf
[Service]
Delegate=cpu cpuset io memory pids
```

そのあと `sudo systemctl daemon-reload` を実行し、**再ログインまたは再起動**する（再起動が推奨）。`cpuset` の委任には systemd 244 以降が要る。出典: Docker Rootless mode Tips、[cgroup v2 on Rootless Containers](https://rootlesscontaine.rs/getting-started/common/cgroup2/)（同上）。

（推測）`--memory 8g` と `--pids-limit 1024` は既定の委任でも効く。`--cpus 2` を本当に効かせるには上の `Delegate=` が要る。`delegate.conf` を入れずに `--cpus 2` を付けても、Docker は受け付けるが CPU 使用は制限されない。

（事実）`docker info` が `none` のときの代わり（`ulimit`・`cpulimit`）はプロセス単位でしか効かず、コンテナの中から無効化できる。出典: Docker Rootless mode Tips（同上）。

（注意）cgroup v2 でも `dbus-user-session` が無いと `docker run` が `read unix @->/run/systemd/private` で失敗することがある。`sudo apt-get install -y dbus-user-session` を入れて再ログイン。出典: Docker Rootless mode Troubleshooting（同上）。

---

## Q4. ネットワーク（コンテナ→ホストの Web、外向き）

### コンテナ→ホスト

（事実）rootless にはホスト側の `docker0` が無い。Docker は RootlessKit の network driver（既定は slirp4netns。無ければ gvisor-tap-vsock）を使う。slirp4netns の既定のネットワークは IP `10.0.2.100/24`、gateway `10.0.2.2`、DNS `10.0.2.3`。出典: [RootlessKit Network Drivers](https://github.com/rootless-containers/rootlesskit/blob/master/docs/network.md)（2026-10-07 閲覧）。

（事実）RootlessKit は既定で `--disable-host-loopback` を付けることが「強く推奨」されている。付いていないと、ホストの `127.0.0.1` で待ち受けるポートに、RootlessKit の network namespace から `10.0.2.2` として届いてしまう。セキュリティ境界なので既定で無効になっている。Docker の `DOCKERD_ROOTLESS_ROOTLESSKIT_DISABLE_HOST_LOOPBACK` の既定は `true`。出典: RootlessKit Network Drivers（同上）、[Rootless Docker: Tips, Gotchas & Fixes](https://sumguy.com/rootless-docker-tips-gotchas-fixes/)（2026-10-07 閲覧、後者は二次情報）.

（事実）`host.docker.internal` は Linux では自動では用意されない（Docker Desktop は用意する）。`docker run` の `--add-host` は特別な値 `host-gateway` を受け付け、これは「ホストの内部 IP アドレス」に解決される。慣例として `host.docker.internal` を `host-gateway` に向ける。出典: [docker container run](https://docs.docker.com/reference/cli/docker/container/run/)（2026-10-07 閲覧）。

```
docker run --add-host host.docker.internal:host-gateway ...
```

（事実）rootless では `host-gateway` が期待どおりに解決されないという報告がある。docker/cli の Discussion #7114 では、`host-gateway` がホストの LAN IP（例 192.168.x.x）に解決される一方で、ホストのサービスが `127.0.0.1` だけで待ち受けているため届かない、という説明がされる。回避策として (1) ホストのサービスを `0.0.0.0` で待ち受ける、(2) `10.0.2.2` に直接向ける、(3) `network_mode: host` を使う、が挙げられている。出典: [docker/cli Discussion #7114](https://github.com/docker/cli/discussions/7114)（2026-10-07 閲覧）。

（事実）Docker の Forum でも「`--add-host` 自体は効く。ただし private な内部 IP ではなく、その機械が起動時に持つ "external" IP を指す必要がある」と書かれている。出典: [Linux rootless docker host.docker.internal IP](https://forums.docker.com/t/linux-rootless-docker-host-docker-internal-ip/141683)（2026-10-07 閲覧）。

（推測）この構成での素直な答えは次のとおり。**ホストの Web を `127.0.0.1` だけで待ち受けさせない。** `0.0.0.0:8200`（または Tailscale のインターフェース IP）で待ち受け、コンテナからは `--add-host host.docker.internal:host-gateway` で届かせる。既定の `disable_host_loopback=true` はそのままにする（`false` にすると、コンテナからホストの `127.0.0.1:*` すべてに届いてしまい、境界が緩む）。`tailscale serve` は `127.0.0.1:8200` でも `0.0.0.0:8200` でも待ち受けた先へ流せる。

（事実）`10.0.2.2` を直接使う方法は RootlessKit の文書にも出てくる（slirp4netns の gateway）。ただしこれは `disable_host_loopback` が `false` のときだけホストの `127.0.0.1` に届く。出典: RootlessKit Network Drivers（同上）。

（事実）`--net=host` は Docker Engine v29.5 以降の rootless で使え、ホストの network namespace を共有するので `127.0.0.1:8200` に直接届く。ただしネットワークの隔離が無くなる。出典: [Docker Rootless mode Troubleshooting](https://docs.docker.com/engine/security/rootless/troubleshoot/)（2026-10-07 閲覧）。→ 複数利用者の隔離を考えると推奨しない（推測）。

### 外向き（Anthropic・OpenRouter・npm・pip）

（事実）slirp4netns は既定で外向きの TCP・UDP を通す。RootlessKit の文書にも `curl https://www.google.com` が通る例がある。DNS は既定で `10.0.2.3` に転送される。出典: RootlessKit Network Drivers（同上）。

（推測）`api.anthropic.com`・`openrouter.ai`・`registry.npmjs.org`・`pypi.org` などへの HTTPS は特に設定なしで通る。企業のプロキシが要る網なら `HTTP_PROXY`/`HTTPS_PROXY` をコンテナに渡す。

（事実）slirp4netns は TCP・UDP・ICMP Echo のみ対応。UDP/QUIC（HTTP/3）や ICMP の一部は制限がある。`ping` は `net.ipv4.ping_group_range` の設定が要る。出典: RootlessKit Network Drivers、Docker Rootless mode Tips（同上）。→ 実用上は TCP の HTTPS だけ通れば足りる（推測）。

---

## Q5. `--cap-drop ALL` と `--security-opt no-new-privileges` は rootless で問題になるか

（事実）rootless では daemon もコンテナも root でない利用者の user namespace の中で動く。`--cap-add` で足した capability も、そのコンテナの user namespace が支配する資源にしか効かず、ホストや全体の資源に対する権限にはならない。出典: [Docker Rootless mode Troubleshooting](https://docs.docker.com/engine/security/rootless/troubleshoot/)（2026-10-07 閲覧）。

（推測）Claude Code（`claude -p`）と pi（node）は、ネットワークの bind や raw socket を必要としない。`--cap-drop ALL` で落ちるのは `CAP_NET_RAW` や `CAP_NET_BIND_SERVICE` などだが、rootless の時点でそれらは意味を持たないので、**問題にならない**。逆に `--cap-drop ALL` を付けても付けなくても安全性はほとんど変わらない（rootless の user namespace が既に効いている）。

（推測）`no-new-privileges` は setuid/setgid による権限昇格を止める。コンテナの中の `sudo`・`su`・`passwd` のような setuid 道具は動かなくなるが、この用途では使わない。npm・pip のインストールにも影響しない。**問題にならない**（要確認: 使用するベースイメージが setuid の補助道具に依存していないか、実際に `docker run` して確かめる）。

（事実）Claude Code の内蔵サンドボックス（`/sandbox`）は Linux/WSL2 で `bubblewrap` と `socat` を使い、非特権コンテナの中では bubblewrap が新しい `/proc` を mount できず `bwrap` エラーになることがある。そのときは `enableWeakerNestedSandbox` を `true` にする。ただしこの設定は隔離を弱める。出典: [Configure the sandboxed Bash tool](https://code.claude.com/docs/en/sandboxing)（2026-10-07 閲覧）。

（推測）この構成では、外側が rootless Docker のコンテナという隔離境界なので、その中でさらに Claude Code の内蔵サンドボックスを重ねる必要は薄い。`--dangerously-skip-permissions` を使うなら内蔵サンドボックスは既定でオフ。もし内蔵サンドボックスを使うなら `bubblewrap`/`socat` の導入と上記の `enableWeakerNestedSandbox` を検討する。

（事実）rootless の既知の制限として AppArmor はサポートされない。`--security-opt apparmor=...` は期待どおり動かない。出典: Docker Rootless mode Troubleshooting（同上）。→ `no-new-privileges` と `cap-drop` で代替する（推測）。

（注意）`no-new-privileges` はコンテナの**中**のプロセスに掛かる。rootless Docker が namespce を作るときにホスト側で使う `newuidmap`/`newgidmap` には掛からないので、コンテナの起動自体は壊れない（推測）。

---

## Q6. Podman rootless との比較（この用途での欠点を 1 段落）

（事実）Podman は rootless が既定で、`--userns=keep-id`（ホストの利用者 UID:GID をコンテナ内の同じ値に写す）を持つ。bind mount の所有者問題に対しては `:U`（コンテナの UID/GID に合わせて再帰的に chown）や idmapped mount も使える。`host.containers.internal` と `host.docker.internal` を `/etc/hosts` に自動で足し、Podman 5.3 以降は pasta の `--map-guest-addr`（既定 169.254.1.2）で rootless でもホストに届く。ただし idmap は rootful のみ、pasta のホスト到達は Podman 5.0〜5.2 で壊れていた時期がある。出典: [podman-run](https://docs.podman.io/en/latest/markdown/podman-run.1.html)、[podman Discussion #24133](https://github.com/podman-container-tools/podman/discussions/24133)（2026-10-07 閲覧）。

（推測）この用途で rootless Docker に明らかな欠点があるとすれば、**bind mount の所有者を保つ公式手段が無い**ことである。Docker の `--userns` は `host` しか取らず、Podman の `keep-id` に当たるものが無い。そのため「コンテナを root で動かしてホスト `sai` の所有を保つ」か「非 root で動かして subuid に写る」かの二択になり、後者ではホストの Web が読みにくくなる。加えて `host.docker.internal` が自動で入らない。一方で、Web が `docker` CLI を subprocess で叩く今の設計には、Docker の常駐 daemon と安定した socket（`/run/user/<uid>/docker.sock`）は都合がよい。Podman でも `podman system service` で API socket は作れるが、socket の寿命が daemon ではなく利用者セッションに結び付く点が違う。総合的には、bind mount の所有権だけを見れば Podman に分があり、既存の Docker 前提の運用（Compose、CLI、ecosystem）をそのまま使いたいなら Docker のままでよい。

---

## Q7. コンテナの中の Claude Code のログイン（ブラウザ無し・Team・認証情報・HOME 共有）

### ブラウザの無い環境でのログイン

（事実）初回に `claude` を起動するとブラウザが開き、ログイン後に `Login successful` と出る。ブラウザが Claude Code のローカル callback サーバに届かないとき（WSL2・SSH・コンテナでよく起きる）は、ブラウザに出たログインコードをターミナルの `Paste code here if prompted` に貼る。出典: [Authentication](https://code.claude.com/docs/en/authentication)（2026-10-07 閲覧）。

（事実）非対話のログイン用に `claude auth login` がある。`--email` でメールを先に入れ、`--sso` で SSO を強制し、`--console` で Console（API 課金）を選ぶ。`claude auth status` は JSON で状態を返す（`configDirectory` を含む。v2.1.268 以降）。`claude auth logout` でログアウト。出典: [CLI reference](https://code.claude.com/docs/en/cli-reference)（2026-10-07 閲覧）。

（推測）コンテナの中では `claude auth login`（または `claude /login`）を実行し、表示された URL をホスト側のブラウザで開き、返ってきたコードをコンテナのターミナルに貼る、という流れになる。返ってきたコードを貼る方式は「ブラウザが callback に届かないとき」の公式の逃げ道なので、コンテナでも同じである。

（事実）CI やスクリプト向けに `claude setup-token` がある。ブラウザ認可のあとトークンをターミナルに**印字するだけで保存しない**。`CLAUDE_CODE_OAUTH_TOKEN` として使う。このトークンは Claude のサブスクリプション（Pro・Max・Team・Enterprise）で認証し、**モデルへのリクエストだけ**ができ、Remote Control や claude.ai コネクタは使えない。出典: Authentication（同上）。

### Team プランでも同じか

（事実）Team/Enterprise のメンバーは、自分の claude.ai アカウントでログインする。ログイン方法の選択で「Claude account with subscription」を選び、自分の Team/Enterprise プランを選んで Authorize する。出典: [Use Claude Code with your Team or Enterprise plan](https://support.claude.com/en/articles/11845131-use-claude-code-with-your-team-or-enterprise-plan)（2026-10-07 閲覧）。

（事実）`claude setup-token` はサブスクリプションで認証するので、Team/Enterprise でも使える（前述のとおり Pro・Max・Team・Enterprise が対象）。出典: Authentication（同上）。

（推測）ブラウザ無しの流れ（URL → コードを貼る）は Team でも個人プランでも同じである。違いは Authorize の画面で選ぶプラン/組織だけ。

（事実・規約上の注意）Claude Code の Legal and compliance は、**各エンドユーザーが自分の Anthropic API キー・Claude サブスクリプションの資格情報・3P 推論プロバイダの資格情報で認証しなければならない**と書く。開発者は Claude.ai の資格情報やセッショントークンを収集・保存・仲介してはならず、Claude アカウントへのログインは Anthropic 自身のフローで完結しなければならない。OAuth 認証は Free・Pro・Max・Team・Enterprise のサブスクリプション購入者向けで、普通の利用を支えるためのもの。出典: [Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)（2026-10-07 閲覧）。

（推測）したがって「サーバに 1 つの Team アカウントでログインし、その資格情報を複数人のコンテナへ配る」ことは規約に反する。**利用者ごとに自分のアカウントでログイン**させ、その利用者のコンテナにその利用者の資格情報（またはその利用者が発行した `claude setup-token`）を入れる。

### 認証情報の置き場と Linux での保存

（事実）Linux では認証情報は `~/.claude/.credentials.json`（モード 0600）に保存される。macOS は Keychain、Windows は `%USERPROFILE%\.claude\.credentials.json`。Linux にはキーチェーンが無いので平文ファイルになる。出典: Authentication（同上）。

（事実）`CLAUDE_CONFIG_DIR` を設定すると、`.credentials.json` はそのディレクトリの下になり、設定・セッション履歴もそちらに入る。macOS の Keychain のエントリもそのディレクトリに対応づけられる。出典: Authentication（同上）。

（事実）OAuth のアカウント、個人の MCP サーバ、プロジェクトごとの trust は `~/.claude.json` に入る（`~/.claude` の外）。`CLAUDE_CONFIG_DIR` を設定すると `.claude.json` もその下に入る。出典: [Development containers](https://code.claude.com/docs/en/devcontainer)（2026-10-07 閲覧）。

（事実）Console のログインを API キー無しで行うと「Anthropic profile」になり、これは設定ディレクトリの外に保存される。つまり `CLAUDE_CONFIG_DIR` を分けても Console のキー無しログインは分離しきれない。出典: Authentication、Development containers（同上）。

### 同じ HOME を複数のコンテナで同時に使ってよいか

（事実）`~/.claude/.credentials.json` には access token（短命）と refresh token が入る。複数の Claude Code プロセスが同時に動くと、このファイルに file lock が無いため、1 つのプロセスが refresh して新しいトークンを書くと、他のプロセスが古い refresh token で更新しようとして失敗し、ログアウト状態に落ちる。これが連鎖する。不具合票 #56339（2.1.116、macOS）と #24317（2.1.37、Team Max 5x、7〜12 同時セッション）に再現手順つきで報告されている。出典: [anthropics/claude-code#56339](https://github.com/anthropics/claude-code/issues/56339)、[anthropics/claude-code#24317](https://github.com/anthropics/claude-code/issues/24317)（2026-10-07 閲覧）。

（推測）したがって **同じ `~/.claude/.credentials.json` を複数のコンテナで同時に読み書きさせるのは避ける**。プロジェクトごとにコンテナを 1 つ作る設計では、同じ利用者の複数プロジェクトが同時に走るとこの競合が起きる。

（推測・対策）この競合を避ける現実的な方法は次のいずれか。

1. `CLAUDE_CODE_OAUTH_TOKEN`（`claude setup-token` で発行した長命トークン）を、その利用者の各コンテナに環境変数で渡す。このトークンはファイルに書かれず、refresh も無いので競合しない。利用者ごとに 1 つ発行する。（事実: トークンは印字されるだけで保存されない。Authentication より）
2. コンテナごとに `CLAUDE_CONFIG_DIR` を分ける（例: `/srv/sai/users/<user>/.claude-<project>`）。ただし各ディレクトリで個別にログインが要る。Console のキー無しログインは設定ディレクトリの外に残る点に注意。
3. 同じ利用者の複数コンテナを同時に走らせない（1 利用者 1 コンテナにし、プロジェクトは中で切り替える）。
4. `ANTHROPIC_API_KEY`（Console の API 課金）を使う。OAuth の競合は無いが、サブスクリプションではなく API 課金になる。

（推測）1 がこの用途に最も合う。規約上も「各エンドユーザーが自分の資格情報で認証する」に沿う（利用者ごとに setup-token を発行する）。ただし 1 つのトークンを同時に複数コンテナから使うことが Anthropic 側で問題にならないかは公式に明記が無い（要確認）。安全側では「1 利用者につき同時 1 コンテナ」に寄せる。

（注意）プロジェクトの JSON・jsonl をホストの Web とコンテナの両方が読み書きするなら、アプリ側で同時書き込みを直列化する（ロックや追記専用）。Claude Code の資格情報とは別の問題として、自分のデータでも競合を避ける（推測）。

---

## 推奨の設定

以下は上の調査からの提案である。`<uid>` は `sai` の `id -u`（例 1001）、`<user>` は研究プロジェクトの利用者名、`<project>` はプロジェクト名に読み替える。

### 1. rootless Docker（`sai` の user サービスとして）

```
# sai として
dockerd-rootless-setuptool.sh install
systemctl --user enable --now docker
sudo loginctl enable-linger sai
```

- systemd の**システム**サービスとして rootless Docker を動かさない。daemon は `sai` の user サービス。
- socket: `/run/user/<uid>/docker.sock`。
- `~/.config/docker/daemon.json` と `~/.config/systemd/user/docker.service.d/override.conf` を使う（`~/.docker` ではない）。

### 2. cgroup v2 の委任（root で）

```
# /etc/systemd/system/user@.service.d/delegate.conf
[Service]
Delegate=cpu cpuset io memory pids
```

```
sudo systemctl daemon-reload
sudo reboot        # 再起動が推奨
sudo apt-get install -y dbus-user-session   # 必要なら
```

確認: `docker info | grep "Cgroup Driver"` が `systemd` であること。

### 3. Web のシステムサービス（rootless socket を使う）

```
[Service]
User=sai
Environment=DOCKER_HOST=unix:///run/user/<uid>/docker.sock
Environment=PATH=/usr/local/bin:/usr/bin:/bin
```

### 4. Web の待ち受け住所

- `127.0.0.1` だけで待ち受けない。`0.0.0.0:8200`（または Tailscale のインターフェース IP `100.x.y.z:8200`）で待ち受ける。
- `disable_host_loopback` は既定の `true` のままにする。
- ファイアウォールで 8200 への到達を `tailscale0` と必要な範囲に絞る。
- `tailscale serve` はそのアドレスへ流す。
- Tailscale の IP で待ち受けてコンテナから届くかは実際に試す（要確認）。届かなければ `0.0.0.0` にする。

### 5. `docker run` の引数（案 A: コンテナを root で動かし、bind mount を `sai` の所有に保つ）

```
docker run -d --name sai-<project> \
  --memory 8g --cpus 2 --pids-limit 1024 \
  --cap-drop ALL --security-opt no-new-privileges \
  -e IS_SANDBOX=1 \
  --add-host host.docker.internal:host-gateway \
  -v /srv/sai/users/<user>:/srv/sai/users/<user> \
  -e CLAUDE_CODE_OAUTH_TOKEN=<その利用者の setup-token> \
  -e SAI_WEB_URL=http://host.docker.internal:8200 \
  <image> \
  claude -p --dangerously-skip-permissions ...
```

- 狙い: コンテナ root はホスト `sai` に写るので、bind mount のファイルは `sai` の所有のまま。ホストの Web がそのまま読める。
- 代償: `--dangerously-skip-permissions` を root で使うために `IS_SANDBOX=1`（**非公式・版で変わりうる**）に頼る。使えると確かめた版を記録し、`claude` を更新したら再確認する。
- `IS_SANDBOX=1` を使いたくない場合は、`--dangerously-skip-permissions` をやめ、`--permission-mode acceptEdits` と `permissions.allow` / `--allowedTools` で必要な道具を事前に許可する（root チェックに当たらない）。`claude -p` は非対話なので、必要な道具を事前許可しておく。

### 5'. `docker run` の引数（案 B: コンテナを非 root で動かす）

```
docker run -d --name sai-<project> \
  --memory 8g --cpus 2 --pids-limit 1024 \
  --cap-drop ALL --security-opt no-new-privileges \
  --user <container-uid> \
  --add-host host.docker.internal:host-gateway \
  -v /srv/sai/users/<user>:/srv/sai/users/<user> \
  -e CLAUDE_CODE_OAUTH_TOKEN=<その利用者の setup-token> \
  <image> ...
```

- 狙い: `claude --dangerously-skip-permissions` を公式の想定どおり非 root で使える。
- 代償: bind mount に書くには、ホスト側でディレクトリを subuid（`subuid + <container-uid> - 1`）に chown するか setfacl で rw を与える必要がある。Web が読むだけなら 0644/0755 で足りるが、`/etc/subuid` の割り当てが変わると壊れる。所有者が `sai` でなくなる点を許容できるかで選ぶ。
- 推奨は案 A（所有者が素直）。`IS_SANDBOX=1` を嫌うなら案 B か、bypass をやめる。

### 6. ログインと資格情報（Q7 のまとめ）

- 利用者ごとに自分の Team/Enterprise アカウントでログインする。1 つのアカウントの資格情報を複数人へ配らない（規約）。
- 長命トークンはその利用者が `claude setup-token` で発行し、`CLAUDE_CODE_OAUTH_TOKEN` として**その利用者のコンテナだけ**に渡す。ファイルに書かないので refresh の競合が無い。
- どうしてもファイル方式にするなら、同じ `~/.claude/.credentials.json` を複数コンテナで共有しない。`CLAUDE_CONFIG_DIR` を分けるか、同時 1 コンテナにする。Console のキー無しログインは設定ディレクトリの外に残る点に注意。
- ブラウザが無いので、ログインは URL をホストのブラウザで開き、コードをコンテナに貼る。

---

## 分からなかったこと

- rootless の `host.docker.internal:host-gateway` が、このサーバの slirp4netns から実際にどの IP に解決されるか。公式文書は「ホストの内部 IP」とだけ書く。Docker の Forum と Discussion は「ホストの external/LAN IP になり、`127.0.0.1` だけのサービスには届かない」と述べる。**要確認**: コンテナで `getent hosts host.docker.internal` と `curl http://host.docker.internal:8200/` を実行して確かめる。届かなければ Web を `0.0.0.0` で待ち受ける。
- `IS_SANDBOX=1` と `CLAUDE_CODE_BUBBLEWRAP=1` が今の版でも有効か。公式文書には無い。確かめた版（v2.1.139 の bundle という報告）と、実際に使う版で再確認する。
- `CLAUDE_CODE_OAUTH_TOKEN` を複数のコンテナから同時に使って問題が無いか。公式文書は同時利用を明示していない。可能なら「1 利用者につき同時 1 コンテナ」に寄せる。
- Team/Enterprise の `claude setup-token` が組織のポリシー（`forceLoginMethod`・`forceLoginOrgUUID`）下でどう振る舞うか。公式文書は「`claude setup-token` は `forceLoginMethod` だけを強制し、`forceLoginOrgUUID` は強制しない」と書く。出典: Authentication（同上）。組織側の設定を管理者に確認する。
- システムサービスの Web と `sai` の user サービスの起動順。socket が無いときの Web の再試行の方針は運用で決める。

