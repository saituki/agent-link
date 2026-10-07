# サーバ（Linux PC）への入れ方

sai-tree を Linux の多人数サーバとして動かす手順である。対象は Ubuntu（cgroup v2）で、
rootless Docker の上に 1 プロジェクト 1 コンテナを置く。Web は 1 つのシステムサービスとして動かし、
`tailscale serve` で tailnet に出す。

- 実行するのは利用者（サーバの管理者）である。ここに書いた命令をそのまま打てばよい。
- コンテナの形は共有契約 C7、置き場は共有契約 C4（`docs/plan/unit-U08-container-image.md`）。
- 値の読み替え: `<repo>` はリポジトリの絶対パス（例 `/srv/sai/sai-tree`。更新を `deploy.sh` で行うなら `/srv/sai/app`）、`<sai-uid>` は `id -u sai` の値（例 `1001`）、`<user>` は研究プロジェクトの利用者の短い名前、`<project>` はプロジェクトの短い名前。
- 更新（Mac から反映する手順）は 15、バックアップは 16 にある。

## 1. 用意（Ubuntu・利用者 `sai`）

cgroup v2 であることを確かめる。

```bash
cat /sys/fs/cgroup/cgroup.controllers
```

`cpu memory pids` などが出れば cgroup v2 である（何も出なければ cgroup v1。Ubuntu 22.04 以降は既定で v2）。

必要な道具を入れる。

```bash
sudo apt-get update
sudo apt-get install -y git curl uidmap dbus-user-session
curl -LsSf https://astral.sh/uv/install.sh | sh
```

Tailscale を入れて `tailscale up` しておく（`docs/server.md` の 9 で使う）。

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

ログインしない利用者 `sai` を作る。Web と rootless Docker はこの利用者で動かす。

```bash
sudo adduser --disabled-password --gecos "" sai
```

`/etc/subuid`・`/etc/subgid` に `sai` の割り当て（65536 個以上）があることを確かめる。

```bash
grep '^sai:' /etc/subuid /etc/subgid
```

無ければ足す（`adduser` が作ることが多い）。

```bash
sudo usermod --add-subuids 100000-165535 --add-subgids 100000-165535 sai
```

## 2. rootless Docker（`sai` の user サービス）

Docker の deb を入れる（`docker-ce-rootless-extras` と `uidmap` が要る）。

```bash
sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-ce-rootless-extras uidmap
```

`sai` で rootless を入れる。

```bash
sudo -iu sai
dockerd-rootless-setuptool.sh install
systemctl --user enable --now docker
exit
```

root で linger を有効にする（`sai` がログインしていなくても user サービスを動かす）。

```bash
sudo loginctl enable-linger sai
```

root の dockerd を使わないなら止める。

```bash
sudo systemctl disable --now docker.service docker.socket
```

socket は `/run/user/<sai-uid>/docker.sock` になる。確かめる。

```bash
sudo -iu sai docker info | grep -i "rootless\|cgroup driver"
```

`Cgroup Driver: systemd` が出ること。

## 3. cgroup v2 の委任（root で）

rootless で `--memory`・`--cpus`・`--pids-limit` を効かせるには、cgroup v2 の委任が要る。

```bash
sudo mkdir -p /etc/systemd/system/user@.service.d
sudo tee /etc/systemd/system/user@.service.d/delegate.conf >/dev/null <<'EOF'
[Service]
Delegate=cpu cpuset io memory pids
EOF
sudo systemctl daemon-reload
sudo reboot
```

再起動したら確かめる。

```bash
sudo -iu sai cat /sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/cgroup.controllers
```

`cpu memory pids` が出ること。

## 4. リポジトリを置き、イメージを作る

リポジトリを `<repo>` に置き、`sai` が読めるようにする。

```bash
sudo mkdir -p "$(dirname <repo>)"
sudo chown sai: <repo>
sudo -iu sai git clone <リポジトリの URL> <repo>
```

その機械の上でイメージを作る（arm64 と amd64 の違いを避けるため、持ち込まずにここで作る）。
初回は Rust のビルドで時間がかかる。

```bash
cd <repo>
sudo -iu sai docker build -f apps/web/server/Dockerfile -t sai-tree:latest .
```

確かめる。

```bash
sudo -iu sai docker run --rm sai-tree:latest sai-link schema --json | head -c 200
sudo -iu sai docker run --rm sai-tree:latest claude --version
sudo -iu sai docker run --rm sai-tree:latest pi --version
sudo -iu sai docker run --rm sai-tree:latest python3 --version
```

## 5. `apps/web` の仮想環境

```bash
cd <repo>/apps/web
sudo -iu sai uv sync
```

## 6. CLI の中継を置く

`/srv/sai/users`（利用者の置き場）と `/opt/sai/bin`（中継のリンク）を用意する。

```bash
sudo mkdir -p /srv/sai/users /opt/sai/bin
sudo chown sai: /srv/sai/users /opt/sai/bin
```

中継 `sai-in-container` を `sai-link`・`sai-paper`・`sai-sci` の 3 つの名前でリンクする。
Web は `SAI_*_BIN` をこのリンクに向ける（7 のひな形）。

```bash
sudo -iu sai ln -sf <repo>/apps/web/server/sai-in-container /opt/sai/bin/sai-link
sudo -iu sai ln -sf <repo>/apps/web/server/sai-in-container /opt/sai/bin/sai-paper
sudo -iu sai ln -sf <repo>/apps/web/server/sai-in-container /opt/sai/bin/sai-sci
```

## 7. Web のシステムサービス

`apps/web/server/sai-web.service` の `<repo>`・`<sai-uid>` を埋めて `/etc/systemd/system/` に置く。

```bash
sed -e 's|<repo>|<repo>|g' -e "s|<sai-uid>|$(id -u sai)|g" \
  <repo>/apps/web/server/sai-web.service | sudo tee /etc/systemd/system/sai-web.service >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --now sai-web
systemctl status sai-web --no-pager
```

rootless の dockerd は `sai` の user サービスなので `After=` では順番を付けられない。Web は
docker が動いていないとき 503 を返す（U09）ので、起動の順番は問わない。

## 8. 防火壁（ufw）

8200 番は `tailscale0` と、コンテナから来る通信だけを許し、LAN からは止める。
rootless のコンテナからはホストの `127.0.0.1` に届かないので、Web は `0.0.0.0:8200` で待ち受ける。

```bash
sudo ufw allow in on tailscale0 to any port 8200 proto tcp
sudo ufw deny 8200/tcp
sudo ufw status verbose
```

コンテナから届くか確かめる（`--add-host host.docker.internal:host-gateway`）。

```bash
sudo -iu sai docker run --rm --add-host host.docker.internal:host-gateway \
  sai-tree:latest curl -s http://host.docker.internal:8200/api/health
```

届かなければ、届く住所を書き留める。rootless では `host.docker.internal` がホストの LAN の住所に
解決されることが多く、`0.0.0.0` で待ち受けていれば届く。それでも届かない場合は
`getent hosts host.docker.internal` の値（または Tailscale の IP）を控え、U09 の
`SAI_WEB_URL_FOR_CONTAINERS` に入れる（コンテナの環境変数 `SAI_WEB_URL` になる）。

## 9. Tailscale で外に出す

```bash
sudo tailscale serve --bg --https=443 http://127.0.0.1:8200
tailscale serve status
```

## 10. 最初の管理者

```bash
cd <repo>
sudo -iu sai env SAI_WEB_USERS_ROOT=/srv/sai/users \
  apps/web/bin/sai-web user add <短い名前> --email <大学のメールアドレス> --admin
```

## 11. Radio・MLflow（使う場合）

全員で共有するので、同じ機械で動かす。手順は各 README のとおり。

- `apps/radio/README.md`
- `apps/mlflow-server/README.md`

## 12. 全員に配るひな形

`/srv/sai/skel/` に `.pi/agent/models.json` などを置くと、新しいプロジェクトの HOME に写る
（U09 の `SAI_WEB_SKEL_DIR`）。

```bash
sudo mkdir -p /srv/sai/skel
sudo chown sai: /srv/sai/skel
# 例: 全員に同じモデルの設定を配る
sudo -iu sai mkdir -p /srv/sai/skel/.pi/agent
sudo -iu sai cp <repo>/apps/web/.env.example /srv/sai/skel/.env.example
```

## 13. 運用

**イメージを作り直したとき**（`sai` で `docker build` のあと）:

```bash
sudo -iu sai docker rm -f sai-<user>-<project>
```

画面（Projects）から起動し直すと、新しいイメージでコンテナが作られる（U09 の作り直し）。

**プロジェクトを消すとき**:

```bash
sudo -iu sai docker rm -f sai-<user>-<project>
sudo rm -rf /srv/sai/users/<user>/projects/<user>-<project>
sudo rm -rf /srv/sai/users/<user>/homes/<user>-<project>
```

**バックアップする場所**: `/srv/sai/users` と、Web の置き場 `~sai/.sai/web/`（手順は 16）。

## 14. 規約の注意（Claude のトークン）

Claude の長命トークン（`claude setup-token`）を Web に預けて使う機能は、Anthropic の規約
（開発者が資格情報を保存・仲介しない）に照らして灰色であり、使う人の責任で使うこと（要件定義書の決定）。
トークンは各自が自分のアカウントで発行し、他人と共有しない。

## 15. 更新のしかた（Mac から `deploy.sh` で反映する）

サーバの上に bare リポジトリを 1 つ置き、Mac からそこへ push して、サーバで `deploy.sh` を走らせる。
外部の git の remote（GitHub など）は使わない。

**初回だけ**。順番は次のとおり。

1. サーバで（root）: 置き場を作り、`sai` に渡す。

   ```bash
   sudo mkdir -p /srv/sai && sudo chown sai: /srv/sai
   ```

2. サーバで（`sai`）: bare リポジトリを作る。

   ```bash
   sudo -iu sai git init --bare /srv/sai/repo.git
   ```

3. Mac で（Tailscale の ssh）: remote を足して、最初の push をする。`sai` が ssh でログインできるように、`/home/sai/.ssh/authorized_keys` に Mac の公開鍵を入れておく。

   ```bash
   cd ~/workdir/Sai/sai-tree
   git remote add server ssh://sai@<マシン名>/srv/sai/repo.git
   git push server main
   ```

4. サーバで（`sai`）: 作業用のリポジトリを clone する（push の後でないと空になる）。

   ```bash
   sudo -iu sai git clone /srv/sai/repo.git /srv/sai/app
   ```

`/srv/sai/app` が配備の作業用のリポジトリである（`deploy.sh` が使う）。`<repo>` を `/srv/sai/app` にすれば、4 以降の手順と `sai-web.service` の `WorkingDirectory` が同じ場所を見る。

**`deploy.sh` が `sudo` で許す行**（root で。`systemctl restart sai-web` の 1 つだけを `sai` に許す）:

```bash
sudo visudo -f /etc/sudoers.d/sai-web
```

開いたファイルに次の 1 行を書く。

```
sai ALL=(root) NOPASSWD: /usr/bin/systemctl restart sai-web
```

**毎回**（Mac で）:

1. テストを全部通す（AGENTS.md の「テストと確認」。Linux の CI でも回る）。
2. `git push server main`
3. `ssh sai@<マシン名> /srv/sai/app/apps/web/server/deploy.sh`

`deploy.sh` は次の 5 段を順に行い、段ごとに何をしているかを表示する（`set -e`。失敗したら止まる）。

1. `git pull --ff-only`（`/srv/sai/app`）
2. `docker build -f apps/web/server/Dockerfile -t sai-tree:latest .`（上限 60 分）
3. `(cd apps/web && uv sync)`
4. `(cd apps/prep && uv build)`（sai-prep の入れ物を `apps/prep/dist/` に作る。`GET /api/prep/download` が配る）
5. `sudo systemctl restart sai-web`

**コンテナは配備で作り直さない。** 実行中のエージェントがいるコンテナには触らない（止めると、動いていた headless の実行が失敗の扱いになる）。
古いイメージのコンテナを新しいイメージで作り直すには、そのコンテナが止まっていることを確かめてから、13 の「イメージを作り直したとき」の手順で `docker rm -f` する。次に Projects の「起動」を押すと、新しいイメージでコンテナが作られる。


## 16. バックアップ（配備の前に）

配備の前に、利用者の置き場と Web の置き場を `sai` の home に取る。

```bash
mkdir -p ~/backups
tar -C /srv/sai -czf ~/backups/users-$(date +%Y%m%d-%H%M%S).tar.gz users
tar -C ~ -czf ~/backups/web-$(date +%Y%m%d-%H%M%S).tar.gz .sai/web
```

- `users` は各プロジェクトの `.sai`・HOME・記録（Chats・論文の Library）を含む。
- 取った物は別の機械にも置く（同じ機械の中だけに置かない）。
- 取り出すときは `tar -C /srv/sai -xzf <file>`（サービスを止めてから行う）。
