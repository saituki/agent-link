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
sudo -iu sai docker run --rm sai-tree:latest python3 --version
sudo -iu sai docker run --rm sai-tree:latest tmux -V
sudo -iu sai docker run --rm sai-tree:latest sai-ts --help
```

## 4a. 共有データ（ontology）

アプリ共通の前計算（`onto.db`・`manifest.json`）を、全プロジェクトのコンテナから**読み取り専用**で読めるようにする（要件定義書 r2 の R3。調査 E5 の案 B）。イメージも全コンテナも作り直さずに版を替えられる。

置き場（ホスト・`sai` 所有・0755）:

```
/srv/sai/shared/
  releases/<名前>/onto.db
  releases/<名前>/manifest.json
  current -> releases/<名前>      （相対の symlink）
```

- 置き場は `SAI_WEB_SHARED_DIR`（`sai-web.service` では `/srv/sai/shared`）。Web がコンテナを作るとき、フォルダが無ければ作る（0755）。作れなければ mount を付けずにコンテナを作る。
- コンテナでは `/opt/sai-shared` に `:ro` で mount し、環境変数 `SAI_SHARED_DIR=/opt/sai-shared/current` を入れる。読むのは `current` の先。`:ro` は kernel が強制するので、コンテナから書けない。
- 共有のフォルダがあるのに mount の無い止まったコンテナは、起動のときに作り直す。`deploy.sh` と `dev.sh containers` は、mount の無いコンテナを作り直す（実行中の物は作業が終わるまで飛ばす）。

**元データの取得と前計算**（`onto.db` を作る側。1 回だけ手で行い、版が上がったときにやり直す。ネットワークを使うのはここだけで、`sai-sci onto` 自体は通信しない）:

| 元 | 取得元 | 大きさ |
|---|---|---|
| MONDO（疾患） | `https://purl.obolibrary.org/obo/mondo.obo` | 約 53 MB |
| MeSH（医学用語。記述子の XML。年は版） | `https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/desc2026.xml` | 約 313 MB |

```bash
D=<作業用の一時フォルダ>
mkdir -p "$D/dl" "$D/<版の名前（日付など）>"
curl -sSL --max-time 300 -o "$D/dl/mondo.obo" https://purl.obolibrary.org/obo/mondo.obo
curl -sSL --max-time 300 -o "$D/dl/desc2026.xml" https://nlmpubs.nlm.nih.gov/projects/mesh/MESH_FILES/xmlmesh/desc2026.xml
python3 -I sai-sci/tools/onto_build.py --mondo "$D/dl/mondo.obo" --mesh "$D/dl/desc2026.xml" --mesh-version 2026 --out "$D/<版の名前>"
```

出力フォルダ（`onto.db` と `manifest.json`）が、下の更新の手順の「送り元」になる。`dev.sh shared <dir>` は出力フォルダの名前をそのまま版の名前にするので、フォルダ名を日付などにしておく（`out` のような名前だと版が区別できない）。2026-10-13 に Mac で試した結果は、MONDO `releases/2026-10-06`・MeSH 2026 で term 63,244・synonym 381,078・edge 89,251・xref 145,518（`onto.db` 約 80 MB）。

**更新の手順**（サーバで。`sai`。この節は手順を書いただけで、この作業では実行していない）:

```bash
SHARED=/srv/sai/shared
NAME=<版の名前（日付など）>
rsync -a <送り元の onto.db・manifest.json のフォルダ>/ "$SHARED/releases/$NAME.partial/"
mv "$SHARED/releases/$NAME.partial" "$SHARED/releases/$NAME"     # 完成の印。途中の版を読ませない
ln -sfn "releases/$NAME" "$SHARED/current.tmp" && mv -T "$SHARED/current.tmp" "$SHARED/current"   # 原子的に切り替え
docker exec sai-<利用者>-<プロジェクト> sh -c 'cat "$SAI_SHARED_DIR/manifest.json"'                 # 新しい版か確かめる
```

**戻し方**: `current` を前の `releases/<前の名前>` へ、同じ 2 行の手順で付け替える。版は `releases/` に残す。

**動いている run への注意**: 実行中の run は、版が切り替わったあとに `onto.db` を開き直すと、途中で版を変えて読むことがある。版は `onto.db` の `meta` と `sai-sci onto` の出力の `map_version` に出るので、記録から追える。切り替えは作業の無い時間に行う。

**表示義務**: MONDO は CC BY 4.0、MeSH は "Courtesy of the U.S. National Library of Medicine"。表示の文言は `manifest.json` の `attribution` を参照する。

**要確認（本物のサーバで初回に確かめる）**: rootless Docker で、`sai` 所有の `/srv/sai/shared` が `:ro` で mount でき、相対の `current` がコンテナの中で解決されること（`docker exec` で `ls -l /opt/sai-shared/current/` を見る）。この作業では実機で確かめていない。

**Mac（開発）**: 置き場は `SAI_DEV_SHARED`（既定 `~/sai-tree-dev/shared`）。`apps/web/server/dev.sh shared <dir>`（`<dir>` は `onto.db` と `manifest.json` のあるフォルダ）で `releases/<名前>/` に入れ、`current` をそこへ向ける。mount を付けるには `dev.sh containers` で作り直す（mount が既にあるコンテナは作り直さなくてよい。2026-10-13 に Mac の Docker で、`current` を作っただけで実行中のコンテナから相対の `current` が解決され、`sai-sci onto` が引けることを確かめた）。Mac の `current` の付け替えは `ln -sfn` で、原子的ではない（`mv -T` が Mac に無い。要確認）。

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

`<machine>.<tailnet>.ts.net` は `tailscale status` の自分の名前（§1 で `tailscale up` したあとに出る）の全体に、
エディタで手で埋める（sed の段では置き換えない）。`SAI_WEB_TS_URL` の値になる（§17）。

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

sai-ts の入口（8210）は `127.0.0.1` だけで待つので、ufw の許可は要らない（§17）。

届かなければ、届く住所を書き留める。rootless では `host.docker.internal` がホストの LAN の住所に
解決されることが多く、`0.0.0.0` で待ち受けていれば届く。それでも届かない場合は
`getent hosts host.docker.internal` の値（または Tailscale の IP）を控え、U09 の
`SAI_WEB_URL_FOR_CONTAINERS` に入れる（コンテナの環境変数 `SAI_WEB_URL` になる）。

## 9. Tailscale で外に出す

```bash
sudo tailscale serve --bg --https=443 http://127.0.0.1:8200
sudo tailscale serve --bg --https=8443 http://127.0.0.1:8210
tailscale serve status
```

`--https=8443` は時系列（sai-ts）の入口。sai-web と同じホスト名の別ポートで出す。拡張の JS が sai-web の API を使えないようにするため（要件定義書 sai-ts 段階 B の D1）。sai-web の `SAI_WEB_TS_URL` を `https://<machine>.<tailnet>.ts.net:8443` にする（§7）。
入口の `127.0.0.1:8210` は sai-web と同じプロセスで待つ（`SAI_WEB_TS_PORT=8210`）。`sudo tailscale serve` の 8443 は、sai-web を起動し直しても残る。

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

`/srv/sai/skel/` に置いたファイルは、新しいプロジェクトの HOME に写る（`SAI_WEB_SKEL_DIR`。HOME に既にあるファイルは上書きしない）。

```bash
sudo mkdir -p /srv/sai/skel
sudo chown sai: /srv/sai/skel
# 例: 全員に同じ .env.example を配る
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

**cc の画面を見る**（対話モードの cc は、コンテナの中の tmux で動く）:

```bash
sudo -iu sai docker exec -it sai-<user>-<project> tmux ls
sudo -iu sai docker exec -it sai-<user>-<project> tmux attach -t <hl-…>
```

見たら `Ctrl-b d` で抜ける。`Ctrl-c` は押さない（cc が止まる）。

## 14. Claude のログインと規約の注意

### 利用者ごとの Claude のログイン（U10）

Account 画面の「Claude にログイン」で、その人の Claude のログインを 1 つ作る。ログインは
`<U>/<user>/claude/`（0700）に置き、その人の全プロジェクトのコンテナの `$HOME/.claude` に bind mount して共有する
（共有契約 C-MOUNT）。cc が refresh を自動で行うので、8 時間で切れない。Mac の Claude Code のログインとは別である。

- 手順: 「Claude にログイン」→ 表示された URL をブラウザで開く → 表示されたコードを画面の欄に貼って「送る」。
  成功すると、実行中でないコンテナは作り直され、実行中の物は「Projects の画面で止めて起動し直す」まで古いまま動く。
- `$HOME/.claude.json` は共有しない（複数の cc が同時に書いて壊れた報告があるため）。
- コンテナを作るとき、HOME の `.claude` の会話の記録は共有のフォルダへ一度だけ写す（元は消さない。`.credentials.json` は写さない）。
- まれに同時の refresh が衝突してログアウトしたら、Account 画面からログインし直す。止まった run は
  「Claude の認証に失敗しました（401）」で failed になる。
- `CLAUDE_CODE_OAUTH_TOKEN`（`claude setup-token`）を入れると、ファイルのログインより token が優先される。
  ログインに成功すると、token は自動で消される。

### 規約の注意

Claude のログインや長命トークンを Web に預けて使う機能は、Anthropic の規約
（開発者が資格情報を保存・仲介しない）に照らして灰色であり、使う人の責任で使うこと（要件定義書の決定）。
ログインとトークンは各自が自分のアカウントで発行し、他人と共有しない。

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

`deploy.sh` は次の 7 段を順に行い、段ごとに何をしているかを表示する（`set -e`。失敗したら止まる）。

1. `git pull --ff-only`（`/srv/sai/app`）
2. `docker build -f apps/web/server/Dockerfile -t sai-tree:latest .`（上限 60 分）
3. `(cd apps/web && uv sync)`
4. `(cd apps/prep && uv build)`（sai-prep の入れ物を `apps/prep/dist/` に作る。`GET /api/prep/download` が配る）
5. `sudo systemctl restart sai-web`
6. 古いイメージ・共有データ（§4a）の mount の無いコンテナを作り直す（作業中は飛ばす）
7. 動いているコンテナに block・template・tools・exec を配り直す（作業中は飛ばす）

**コンテナは配備で作り直さない。**（ただし 6 段の古いイメージ・共有データ（§4a）の mount が無いコンテナは、作業中で無ければ作り直す。） 実行中のエージェントがいるコンテナには触らない（止めると、動いていた headless の実行が失敗の扱いになる）。
古いイメージのコンテナを新しいイメージで作り直すには、そのコンテナが止まっていることを確かめてから、13 の「イメージを作り直したとき」の手順で `docker rm -f` する。次に Projects の「起動」を押すと、新しいイメージでコンテナが作られる。

cc のフックの設定（`.claude/settings.json`）は cc の起動ごとに sai-link が最新に書き直すので、配備のあとに各プロジェクトで `sai-link init` をやり直す必要は無い。


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

## 17. 時系列（sai-ts）

段階 B の時系列は、Projects の自分のプロジェクトの「時系列」から開く。sai-web の画面からは、そのプロジェクトのコンテナが動いているときだけ出る。
構成: ブラウザ → `tailscale serve --https=8443` → `127.0.0.1:8210`（sai-web と同じプロセスの ts 入口）→ コンテナの `127.0.0.1:<docker が選ぶ番号>` → コンテナの中の `sai-ts serve`。

**古いコンテナ**: 段階 B より前に作ったコンテナには時系列の口が無い。止めて起動し直すと、新しいイメージで作り直される（作業中は飛ばす。`deploy.sh` も作業中でないコンテナを作り直す）。

**rootless での確かめ**（未確認。サーバで行う）: コンテナが口を持つかを見る。

```bash
sudo -iu sai docker port sai-<user>-<project> 8210/tcp
```

`127.0.0.1:<番号>` が返れば口がある。何も返らなければ、古いコンテナか、rootless で口が出ていない（Mac の Docker Desktop では確かめてある）。

**データ**: 時系列のデータ（`ts/datasets/`）は agent に作らせる。例: カードで「`sai-ts demo demo --json` を実行して」と頼む。

**チャット**: 時系列の画面の「💬 Chats で開く」は、sai-web の Chats を別のタブで開き、block `ts-engineer` を選んだ状態にする。時系列の画面の中にチャットは無い。押せるのは sai-web の Projects から開いたときだけ（sai-web が入口の断片に `web` と `project` を渡す）。
- 旧の block `ts-agent` は、既存のプロジェクトの `.sai/blocks/` に残る。使わなくなったが、利用者の物なので消さない。`sai-ts init --update`（`deploy.sh` の配り直しが呼ぶ）は `ts-engineer` だけを書く。

**バックアップ**: スナップショット（`ts/snapshots/<name>/`・research-tree に結んだ元の一式）は `ts/` の中にあり、プロジェクトの置き場 `/srv/sai/users/<user>/projects/<user>-<project>/` の一部として、§16 の `users` の tar に入る（`ts/` は、`users` の中のプロジェクトの置き場の一部。`paths.ts_root()` は `<project>/ts`）。
- 確かめ（未確認。サーバで行う）: `tar -tzf ~/backups/users-*.tar.gz | grep ts/snapshots | head`。

**合言葉**: sai-web を起動し直すと、開いている時系列の画面の合言葉は無効になる。Projects から開き直す。

**受け入れた危険**（要件定義書 sai-ts 段階 B の §5・利用者が受け入れた）:
- 同じ利用者の別プロジェクトの sai-ts は同じオリジン（同じホスト名の別ポート）に載る。拡張の JS は別のプロジェクトの合言葉を読めないが、同じタブの参照の経路は完全には塞げない。
- 大量の Cookie で sai-web の Cookie を追い出して置き換える攻撃は防げない。置き換えられると、画面の右上などに別の利用者名が出る。
