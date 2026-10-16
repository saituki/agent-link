# sai-moon から agent-link への切り替え手順書

> **古い手順書（2026-10-16）。** sai-moon を止めて agent-link に移る前提で書いた。今は sai-moon と agent-link を並べて使う（`README.md` の「sai-moon と並べて使う」）。agent-link の Web は 8400 番で、sai-moon の Web（8200 番）は止めない。下の手順の 8200 番は、sai-moon の Web を指す所と、旧い agent-link の既定を指す所が混ざっている。

この文書は、利用者が上から順に実行する手順書である。**agent や worker は、この手順を実行しない**（sai-moon を止める・PATH を変える・Tailscale の ACL を変えるのは、利用者の環境を変える操作だから）。

- 命令は、そのまま貼れる形で書いてある。各段に「確かめ方」と「うまくいかないとき」を付けた。
- 決めたこと: sai-moon は止めて、フォルダ（`~/workdir/Sai/sai-moon`）とデータ（`~/.sai`）は**残す**。sai-tree の今の記録（プロジェクト内の `.sai/`）は**移さない**。agent-link は、新しい `.agent-link/` から始める。古い記録は sai-moon の CLI で読める。
- 切り替えで変わる名前（正は `docs/plan/` の共有契約 C2）:

| 種類 | sai-moon（今） | agent-link（切り替え後） |
|---|---|---|
| 実行ファイル | `sai-link`・`sai-web` | `agent-link`・`agent-link-web` |
| 環境変数 | `SAI_*`（例 `SAI_WEB_TOKEN`・`SAI_PROJECT_ROOT`） | `AGENT_LINK_*`（例 `AGENT_LINK_WEB_PORT`・`AGENT_LINK_PROJECT_ROOT`。合言葉の変数は無い） |
| プロジェクトのフォルダ | `.sai/`（DB は `.sai/sai-engine/sai-engine.db`） | `.agent-link/`（DB は `.agent-link/engine/agent-link.db`） |
| 置き場 | `~/.sai` | `~/.agent-link` |
| Web の DB・ログ | `~/.sai/web/sai_web.db`・`sai-web.log` | `~/.agent-link/web/agent_link_web.db`・`agent-link-web.log` |
| template | `sai-base.md` など | `link-base.md`・`link-cli.md`・`link-task.md`・`link-user.md`・`link-report.md` |
| cc の命令 | `/sai-finalize` | `/agent-link-finalize` |

> **注意（7 の先読み）**: agent-link の Web には合言葉が無い。Tailscale serve（`:4444`）で iPhone から使うのは、**tailnet が自分 1 人のときだけ**にする。tailnet に他の人（Linux のテスト利用者など）を招くなら、招く前に 7 の ACL を設定する。既定の ACL は「全員が全部の機械に届く」ため、そのまま招くと、その人が Mac の Web でエージェントを起動できてしまう。

---

## 1. 前提

- agent-link の main に、複製の作業（U01〜U07）と、そのあとの簡素化（cc だけ・合言葉と Account なし・日付フォルダ・画面と CLI の文を英語に）がすべて入っている。
- release と Web の Python 環境ができている。

```bash
cd ~/workdir/Sai/agent-link
git status --short && git log --oneline -3
cargo build --release
(cd apps/web && uv sync)
ls -l target/release/agent-link apps/web/bin/agent-link-web
```

**確かめ方**: `ls` が 2 つのファイルを出す。`target/release/agent-link --help` が出る。

**うまくいかないとき**: `cargo build --release` が失敗するなら、`rustup update stable` のあと、もう一度。`uv sync` が失敗するなら、`uv --version` を確かめる。

---

## 2. 動いている run が無いことを確かめる（sai-moon の CLI で）

まだ PATH は sai-moon のままなので、`sai-link` は sai-moon の CLI である。run が動いている間に daemon を止めると、その run は失敗の扱いになる。

```bash
for p in ~/workdir/Sai/sai-moon ~/workdir/Sai/sai-tree ~/workdir/Sai/agent-link; do
  echo "== $p"
  (cd "$p" && sai-link run tree | python3 -c "
import json, sys
d = json.load(sys.stdin)
print([(f['id'], len(f.get('children', []))) for f in d['items'] if f.get('type') == 'folder' and f.get('id') == 'running'])")
done
```

**確かめ方**: 3 つとも `[]` か `[('running', 0)]` と出る（running が空）。**sai-tree と agent-link の両方**で確かめる。

**うまくいかないとき**:
- `[('running', 2)]` のように 0 でない: その run が終わる（review になる）のを待つか、Web の Chats で終了（■）させる。終わってから、もう一度。
- `sai-link: command not found`: PATH がもう sai-moon を指していない。`~/workdir/Sai/sai-moon/target/release/sai-link` を絶対パスで呼ぶ。
- `プロジェクトが見つかりません`: そのフォルダに `.sai/` が無い。そのプロジェクトは飛ばしてよい。

---

## 3. sai-moon を止める

### 3.1 daemon

動いている daemon の数と場所を見る（2026-10-15 の調査では 3 つ: sai-moon・sai-tree・agent-link）。

```bash
ps aux | grep 'sai-link daemon _run' | grep -v grep
```

各プロジェクトで止める（sai-moon の CLI）。

```bash
for p in ~/workdir/Sai/sai-moon ~/workdir/Sai/sai-tree ~/workdir/Sai/agent-link; do
  echo "== $p"
  (cd "$p" && sai-link daemon status; sai-link daemon stop)
done
```

上の `ps` に、これ以外のプロジェクトの daemon が出ていたら、そのフォルダでも `sai-link daemon stop` を実行する（PID から場所を知るには `lsof -a -p <PID> -d cwd`）。

### 3.2 Web（8200 番）

```bash
lsof -nP -iTCP:8200 -sTCP:LISTEN
kill <PID>
```

`<PID>` は `lsof` の PID の列の値。

### 3.3 Radio（8201 番）

```bash
lsof -nP -iTCP:8201 -sTCP:LISTEN
kill <PID>
```

### 3.4 任意: 8299 番の確認用の Web

sai-tree の確認用の Web が 8299 番で動いていて、`tailscale serve` の `:4445` がこれを出している。使わないなら止める。

```bash
lsof -nP -iTCP:8299 -sTCP:LISTEN
kill <PID>
```

**確かめ方**:

```bash
ps aux | grep -E 'sai-moon|sai-link daemon|sai_web|sai_radio' | grep -v grep
lsof -nP -iTCP:8200 -iTCP:8201 -sTCP:LISTEN
```

どちらも何も出ない。Web の起動の行（`.venv/bin/python -m uvicorn sai_web.main:app`）は `sai-moon` という文字を含まないので、`sai_web` でも探す。**フォルダ `~/workdir/Sai/sai-moon` と `~/.sai` は消さない。**

**うまくいかないとき**:
- `sai-link daemon stop` が「実行中の作業がある」と断る: 2 に戻って、run が終わるのを待つ。
- `kill <PID>` で止まらない: 5 秒待ってから `kill -9 <PID>`。Web と Radio の中の状態は捨ててよい（記録は DB と書き出しにある）。

---

## 4. PATH

`~/.zshrc` の `sai-moon/target/release` を含む行を消し、agent-link の 2 つを足す。

```bash
grep -n 'sai-moon\|agent-link' ~/.zshrc          # 今の行を見る
```

`~/.zshrc` をエディタで開き、`…/sai-moon/target/release` を PATH から外して、次の行を足す。

```bash
export PATH="$HOME/workdir/Sai/agent-link/target/release:$HOME/workdir/Sai/agent-link/apps/web/bin:$PATH"
```

新しいシェルで確かめる。

```bash
exec zsh -l
command -v agent-link agent-link-web
command -v sai-link || echo "sai-link は PATH に無い（正しい）"
agent-link schema --json | python3 -c "import json,sys; print(json.load(sys.stdin)['bin'])"
```

**確かめ方**: `command -v` が `…/agent-link/target/release/agent-link` と `…/agent-link/apps/web/bin/agent-link-web` を出す。`command -v sai-link` は何も出さないか、sai-moon 以外を指す。最後の命令が `agent-link` と出る。

**うまくいかないとき**: `command -v agent-link` が何も出さない → `~/.zshrc` の行の綴りと、`ls ~/workdir/Sai/agent-link/target/release/agent-link` を確かめる。`command -v sai-link` が sai-moon を指す → 別のファイル（`~/.zprofile`・`~/.zshenv`）にも PATH の行が無いか `grep -rn 'sai-moon' ~/.zshrc ~/.zprofile ~/.zshenv` で探す。

---

## 5. プロジェクトを agent-link にする（sai-tree と agent-link の両方）

`agent-link init` は `.agent-link/`・`.mcp.json`・`.claude/settings.json`・`.claude/commands/`・`.claude/skills/`・`.serena/project.yml` を作る。`.mcp.json` と `.claude/settings.json` は書き直され、前の中身は `.bak`（既にあれば `.bak.<日時>`）に残る。**古い `.sai/` は残る**（agent-link は読まない）。

### 5.1 sai-tree

```bash
cd ~/workdir/Sai/sai-tree
agent-link init
ls -a .agent-link .agent-link/engine
```

sai-tree の `.gitignore` に `.agent-link/` の 1 行を足して commit する（**sai-tree への変更はこの 1 行だけ**）。

```bash
cd ~/workdir/Sai/sai-tree
printf '\n.agent-link/\n' >> .gitignore
git add .gitignore
git commit -m "chore: .agent-link/ を .gitignore に足す"
git status --short
```

`.mcp.json.bak`・`.claude/settings.json.bak` などの `.bak` は commit しない（戻し方 8 で使う）。

前の `.sai/blocks/` に自分で直した block があれば、違いを見て、必要な物を `.agent-link/blocks/` に写す。

```bash
diff -r .sai/blocks .agent-link/blocks
```

写すときは、中の名前を新しい名前に直す: `sai-link` → `agent-link`、`sai-web` → `agent-link-web`、`SAI_<名前>` → `AGENT_LINK_<名前>`（`SAI_LINK_BIN` は `AGENT_LINK_BIN`）、`.sai/` → `.agent-link/`、`templates:` の `sai-base`・`sai-cli`・`sai-task`・`sai-user`・`sai-report` → `link-base`・`link-cli`・`link-task`・`link-user`・`link-report`。

### 5.2 agent-link

```bash
cd ~/workdir/Sai/agent-link
agent-link init
ls -a .agent-link .agent-link/engine
```

### 5.3 daemon を起こす

それぞれのプロジェクトで起こす。

```bash
(cd ~/workdir/Sai/sai-tree && agent-link daemon start && agent-link daemon status)
(cd ~/workdir/Sai/agent-link && agent-link daemon start && agent-link daemon status)
```

**確かめ方**:

```bash
(cd ~/workdir/Sai/sai-tree && agent-link run tree)
(cd ~/workdir/Sai/agent-link && agent-link run tree)
cat ~/.agent-link/registry.json | python3 -c "import json,sys; print([e['name'] for e in json.load(sys.stdin)])"
```

`agent-link run tree` が、`todo`・`running`・`review` の 3 つの空のフォルダ（`children` が空）だけの `items` を返す。登録簿に `sai-tree` と `agent-link` がある。`.agent-link/engine/` に `agent-link.db` がある。

**うまくいかないとき**:
- `init` が `プロジェクトが見つかりません` などで止まる: そのフォルダ（`cd` した先）で実行しているか確かめる。
- `daemon start` が失敗する: `cat .agent-link/engine/daemon.log` を読む。3 で sai-moon の daemon を止め忘れていないか、`ps aux | grep 'daemon _run'` で確かめる。
- cc の設定（`.claude/settings.json`）を前に戻したい: 8 を見る。

---

## 6. Web を起こす

### 6.1 起こす

```bash
mkdir -p ~/.agent-link/web
cd ~/workdir/Sai/agent-link/apps/web && nohup .venv/bin/python -m uvicorn agent_link_web.main:app --host 127.0.0.1 --port 8200 --app-dir src > ~/.agent-link/web/uvicorn-8200.log 2>&1 &
```

（`README.md` の「最初の一歩」の起動の命令と同じ物。`--host 127.0.0.1` を変えない。）

**確かめ方**:

```bash
sleep 3
lsof -nP -iTCP:8200 -sTCP:LISTEN
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8200/api/health
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8200/api/projects
```

`lsof` が `127.0.0.1:8200` だけを出す。`curl` は 2 つとも `200` と出る（合言葉は無い）。

**うまくいかないとき**: `cat ~/.agent-link/web/uvicorn-8200.log` を読む。`Address already in use` なら 3.2 の Web が残っている（`lsof` で PID を見て `kill`）。`ModuleNotFoundError` なら `(cd ~/workdir/Sai/agent-link/apps/web && uv sync)` をやり直す。

### 6.2 tailnet への出し方

自分 1 人の tailnet のときだけ使う。合言葉は無いので、tailnet に届く人は誰でも全機能（エージェントの起動を含む）を使える。`tailscale serve` の `:4444 → 8200` は、そのまま使える。使っていない `:4443 → 8100` は閉じる。

```bash
tailscale serve status
tailscale serve --https=4443 off
tailscale serve status
```

**確かめ方**: `tailscale serve status` に `:4444 → http://127.0.0.1:8200` が残り、`:4443` が無い。`:4445 → 8299` は、3.4 で止めたなら `tailscale serve --https=4445 off` で閉じてよい。

iPhone と PC で `https://<Mac の名前>.<tailnet>.ts.net:4444/` を開く。タブは Home・Projects・Chats の 3 つ。Projects に `sai-tree` と `agent-link` が出る。Home に Claude のカード（5 時間枠・7 日枠と費用）が出る。

通知を使うなら、置き場が変わって VAPID の鍵が新しくなる（`~/.agent-link/web/vapid.json`）ので、iPhone で通知を登録し直す（Chats の 🔔）。

**うまくいかないとき**: 画面が古いまま（sai-moon の画面）なら、iPhone のホーム画面のアプリを一度閉じて開き直す（新しい画面は古い倉庫 `sai-web-v*` を自動で捨てる）。それでも直らなければ、Safari の設定で、このサイトのデータを消す。

---

## 7. 他の人を tailnet に招くなら（ACL）

**他の人（Linux のテスト利用者など）を tailnet に招く場合だけ必要**で、自分 1 人のままなら実行しない。招くなら、**必ず、招く前に実行する。** 順序を逆にしない。

- Tailscale の既定の ACL は「全員が全部の機械に届く」。そのまま招くと、テスト利用者が Mac の `:4444`（agent-link の Web）と `:4445` に届く。
- agent-link の Web には合言葉が無い。ACL で届く機械を絞らないと、招いた人は誰でも Mac の Web を使える。

やること: Linux の機械に tag（例 `tag:sai-server`）を付け、テスト利用者を group にまとめ、group には Linux の 443 番と 8443 番だけを許す。Tailscale の管理画面（Access controls）の policy file を、次の形にする（`tester1@example.com` は実際の利用者に置き換える）。

```json
{
  "groups":    { "group:testers": ["tester1@example.com"] },
  "tagOwners": { "tag:sai-server": ["autogroup:admin"] },
  "acls": [
    { "action": "accept", "src": ["autogroup:admin"], "dst": ["*:*"] },
    { "action": "accept", "src": ["group:testers"],  "dst": ["tag:sai-server:443,8443"] }
  ]
}
```

- 1 行目の rule は、あなた（管理者）が今までどおり全部の機械に届くための物。
- Linux の機械に tag を付ける: 管理画面の Machines で、その機械の「Edit ACL tags」から `tag:sai-server` を選ぶ。または Linux 側で `sudo tailscale up --advertise-tags=tag:sai-server`。
- （要確認）tag の付け方と、利用者の数の上限は、Tailscale の今の文書（https://tailscale.com/kb/1068/acl-tags と、使っているプランの利用者の上限）で確かめる。

**確かめ方**: テスト利用者の端末（招いたあと）から、次を試す。

```bash
curl -m 5 -s -o /dev/null -w '%{http_code}\n' https://<Mac の名前>.<tailnet>.ts.net:4444/api/health
curl -m 5 -s -o /dev/null -w '%{http_code}\n' https://<Linux の名前>.<tailnet>.ts.net/
```

1 つ目は届かない（接続できない・時間切れ）。2 つ目は届く（Linux の画面が返る）。管理画面の Access controls の「Preview rules」でも、`tester1@example.com` から Mac の `:4444` が拒否されることを見る。

**うまくいかないとき**: policy file の保存が JSON の文法で拒否される → 末尾のカンマと引用符を確かめる。2 つ目も届かない → Linux 側の `tailscale serve` の待ち受けが 443 番か、tag が付いているかを確かめる。1 つ目が届いてしまう → `src` が `group:testers` だけを指しているか、別の古い rule（既定の `*` → `*`）が残っていないかを確かめる。

---

## 8. 戻し方

agent-link に問題があって sai-moon に戻すとき。古い `.sai/` と `~/.sai` は手を付けていないので、そのまま使える。

1. agent-link の daemon と Web を止める。

   ```bash
   (cd ~/workdir/Sai/sai-tree && agent-link daemon stop)
   (cd ~/workdir/Sai/agent-link && agent-link daemon stop)
   lsof -nP -iTCP:8200 -sTCP:LISTEN
   kill <PID>
   ```

2. PATH を sai-moon に戻す: `~/.zshrc` から agent-link の行を消し、`$HOME/workdir/Sai/sai-moon/target/release` を足して、`exec zsh -l`。`command -v sai-link` が sai-moon を指すことを確かめる。
3. sai-tree の cc の設定を戻す。`.bak` は `init` が書き直す前の物（既にあれば `.bak.<日時>` の一番新しい物）。

   ```bash
   cd ~/workdir/Sai/sai-tree
   ls -a .claude .mcp.json* | grep bak
   cp .claude/settings.json.bak .claude/settings.json
   cp .mcp.json.bak .mcp.json
   ```

   `.bak` の名前が `.bak.<日時>` のときは、その名前に置き換える。

4. sai-moon の daemon と Web を、前と同じ命令で起こす。

   ```bash
   for p in ~/workdir/Sai/sai-moon ~/workdir/Sai/sai-tree ~/workdir/Sai/agent-link; do
     (cd "$p" && ~/workdir/Sai/sai-moon/target/release/sai-link daemon start)
   done
   cd ~/workdir/Sai/sai-moon/apps/web && nohup .venv/bin/python -m uvicorn sai_web.main:app --host 127.0.0.1 --port 8200 &
   ```

   （sai-moon の Web の起動の命令と、`SAI_WEB_TOKEN` の設定は、sai-moon の README に従う。）
5. Radio（8201 番）が要るなら、sai-moon の `apps/radio/README.md` の命令で起こす。

**確かめ方**: `command -v sai-link` が sai-moon を指す。`lsof -nP -iTCP:8200 -sTCP:LISTEN` が sai-moon の Web を出す。Web の画面に、古い記録（run 1〜289）が見える。

**うまくいかないとき**: 古い画面が iPhone に残るときは、6.2 の最後の手順（アプリを閉じて開き直す・サイトのデータを消す）を行う。agent-link の `.agent-link/` は、戻しても消えない。もう使わないなら、利用者の判断で消す。
