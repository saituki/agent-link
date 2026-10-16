# sai-moon と agent-link を並べて使う手順書

この文書は、利用者が上から順に実行する手順書である。**agent や worker は、この手順を実行しない**（Web を起こす・Tailscale の serve を変える・プロジェクトの設定を書き直すのは、利用者の環境を変える操作だから）。

- 命令は、そのまま貼れる形で書いてある。各段に「確かめ方」と「うまくいかないとき」を付けた。
- 決めたこと: sai-moon と agent-link は**置き換えず、並べて使う**。sai-moon は止めない。フォルダ（`~/workdir/Sai/sai-moon`）・データ（`~/.sai`）・PATH・Web（8200 番）・Tailscale の `:4444` は、そのまま使う。sai-tree の今の記録（プロジェクト内の `.sai/`）は移さない。agent-link は、新しい `.agent-link/` から始める。
- 名前は重ならないようにしてある（値は `README.md` の「sai-moon と並べて使う」の表と同じ）。

| 物 | sai-moon | agent-link |
|---|---|---|
| 実行ファイル | `sai-link`・`sai-web` | `agent-link`・`agent-link-web` |
| 環境変数 | `SAI_*` | `AGENT_LINK_*` |
| 置き場 | `~/.sai`・プロジェクトの `.sai/` | `~/.agent-link`・プロジェクトの `.agent-link/` |
| Web | 8200 番（Tailscale 4444） | 8400 番（Tailscale 4446） |
| cc の設定 | プロジェクトの `.claude/settings.json` | `.agent-link/cc-settings.json`（`--settings` で渡す） |
| 完了のコマンド | `/sai-finalize` | `/agent-link-finalize` |

> **注意（3 の先読み）**: agent-link の Web には合言葉が無い。Tailscale serve（`:4446`）で iPhone から使うのは、**tailnet が自分 1 人のときだけ**にする。tailnet に他の人（Linux のテスト利用者など）を招くなら、招く前に 6 の ACL を設定する。既定の ACL は「全員が全部の機械に届く」ため、そのまま招くと、その人が Mac の Web でエージェントを起動できてしまう。

---

## 1. agent-link を入れる

`README.md` の「入れ方」を実行する（`cargo build --release`・`(cd apps/web && uv sync)`・PATH に `target/release` と `apps/web/bin` を足す）。sai-moon の PATH の行は消さない。

```bash
cd ~/workdir/Sai/agent-link
git status --short && git log --oneline -3
ls -l target/release/agent-link apps/web/bin/agent-link-web
```

**確かめ方**: `ls` が 2 つのファイルを出す。新しいシェルで `command -v agent-link agent-link-web sai-link` が 3 つとも出る（`sai-link` は sai-moon の物）。`agent-link schema --json | python3 -c "import json,sys; print(json.load(sys.stdin)['bin'])"` が `agent-link` と出る。

**うまくいかないとき**: `cargo build --release` が失敗するなら、`rustup update stable` のあと、もう一度。`uv sync` が失敗するなら、`uv --version` を確かめる。`command -v agent-link` が何も出さないなら、`~/.zshrc` の PATH の行の綴りと `ls ~/workdir/Sai/agent-link/target/release/agent-link` を確かめる。

---

## 2. agent-link の Web を 8400 番で起こす

`README.md` の `agent-link-web-restart`（`~/.zshrc` に書く関数）を使う。ログは `~/.agent-link/web/agent-link-web.log` に出る。

```bash
agent-link-web-restart
sleep 3
lsof -nP -iTCP:8400 -sTCP:LISTEN
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8400/api/health
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8400/api/projects
```

**確かめ方**: `lsof` が `127.0.0.1:8400` だけを出す（`0.0.0.0` や `*` が出たら止める）。`curl` は 2 つとも `200` と出る（合言葉は無い）。sai-moon の Web（8200 番）は、そのまま動いている（`lsof -nP -iTCP:8200 -sTCP:LISTEN`）。

**うまくいかないとき**: `cat ~/.agent-link/web/agent-link-web.log` を読む。`Address already in use` なら、8400 番を別の物が使っている（`lsof` で PID を見る）。`ModuleNotFoundError` なら `(cd ~/workdir/Sai/agent-link/apps/web && uv sync)` をやり直す。`agent-link not found` と出るなら、Web を起こしたシェルの PATH に `agent-link` が無い。

---

## 3. Tailscale で iPhone から使う

自分 1 人の tailnet のときだけ使う。合言葉は無いので、tailnet に届く人は誰でも全機能（エージェントの起動を含む）を使える。

- `:4444` は sai-moon（8200 番）の住所なので、**触らない**。
- `:4446` は、前に 8296 番の agent-link を指していた住所。8400 番に向け直すだけで、iPhone のアプリと通知の購読をそのまま使える。

```bash
tailscale serve status
tailscale serve --bg --https 4446 http://127.0.0.1:8400
tailscale serve status
```

**確かめ方**: `tailscale serve status` に `:4444 → http://127.0.0.1:8200` と `:4446 → http://127.0.0.1:8400` の両方が出る。iPhone と PC で `https://<Mac の名前>.<tailnet>.ts.net:4446/` を開く。タブは Home・Projects・Chats の 3 つ。Projects に agent-link の登録簿のプロジェクトが出る。Home に Claude のカード（5 時間枠・7 日枠と費用）が出る。

**うまくいかないとき**: 画面が古いまま（8296 番の時代の画面）なら、iPhone のホーム画面のアプリを一度閉じて開き直す。それでも直らなければ、Safari の設定で、このサイトのデータを消す。通知が来ないときは、Chats の 🔔 で登録し直す。

---

## 4. 両方を使うプロジェクトを整える（agent-link・sai-tree など）

### 4.1 agent-link で init する

まだ init していないプロジェクトだけ。`agent-link init` は `.agent-link/`（cc の設定 `.agent-link/cc-settings.json` を含む）・`.mcp.json`・`.claude/commands/`・`.claude/skills/`・`.serena/project.yml` を作る。`.mcp.json` などの既にある物は `.bak`（既にあれば `.bak.<日時>`）に退避される。**`.claude/settings.json` は書かない。** `.agent-link/` は `.gitignore` に足す。

```bash
cd ~/workdir/Sai/sai-tree
agent-link init
agent-link daemon start && agent-link daemon status
```

### 4.2 `.claude/settings.json` を sai-moon 版に戻す

前の agent-link（2026-10-16 より前）は、run のたびにプロジェクトの `.claude/settings.json` を agent-link 版で上書きしていた。sai-moon の run の質問が Web に届かなくなるので、**両方を使うプロジェクト（例 agent-link・sai-tree）で**、sai-moon の `init --update` を実行して sai-moon 版に戻す。

```bash
cd ~/workdir/Sai/agent-link   # sai-tree でも同じ
sai-link init --update
grep -c sai-moon .claude/settings.json
```

**確かめ方**: `grep -c sai-moon .claude/settings.json` が 1 以上を出す。agent-link の run の cc は `.agent-link/cc-settings.json` を `--settings` で受けるので、プロジェクトの `.claude/settings.json` が sai-moon 版のままで動く。

**うまくいかないとき**: `grep -c` が 0 なら、`sai-link` が sai-moon の物か（`command -v sai-link`）を確かめて、もう一度。上書き前の物は `.claude/settings.json.bak`（または `.bak.<日時>`）にある。

---

## 5. 質問の確かめ方

質問は run の持ち主の Web に届く。

- **agent-link の run の質問**: 8400 番の Web（Home・Chats）か、`agent-link-web questions`。
- **sai-moon の run の質問**: sai-moon の Web（8200 番）。

```bash
agent-link-web questions
agent-link-web answer <qid> --choice 0   # 番号は 0 から。範囲外は断られる（終了コード 2）
```

**確かめ方**: agent-link のプロジェクトで、質問する run（cc の AskUserQuestion）を起こすと、`agent-link-web questions` に 1 件出て、Web の Home にも出る。答えると cc が続きを進める。sai-moon の run を同じプロジェクトで動かしても、その質問は 8200 番の Web に出て、8400 番には出ない。

**うまくいかないとき**:
- 質問がどちらにも出ない: プロジェクトの `.claude/settings.json` が相手の版になっている。4.2 をやり直す。
- agent-link の質問が出ない: `cat ~/.agent-link/web/agent-link-web.log` を読む。Web が止まっていても cc は普通に動く（質問は端末で答える）。agent-link の daemon が動いているか `agent-link daemon status` で見る。

---

## 6. 他の人を tailnet に招くなら（ACL）

**他の人（Linux のテスト利用者など）を tailnet に招く場合だけ必要**で、自分 1 人のままなら実行しない。招くなら、**必ず、招く前に実行する。** 順序を逆にしない。

- Tailscale の既定の ACL は「全員が全部の機械に届く」。そのまま招くと、テスト利用者が Mac の `:4446`（agent-link の Web）と `:4444`（sai-moon の Web）に届く。
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
curl -m 5 -s -o /dev/null -w '%{http_code}\n' https://<Mac の名前>.<tailnet>.ts.net:4446/api/health
curl -m 5 -s -o /dev/null -w '%{http_code}\n' https://<Linux の名前>.<tailnet>.ts.net/
```

1 つ目は届かない（接続できない・時間切れ）。2 つ目は届く（Linux の画面が返る）。管理画面の Access controls の「Preview rules」でも、`tester1@example.com` から Mac の `:4446` が拒否されることを見る。

**うまくいかないとき**: policy file の保存が JSON の文法で拒否される → 末尾のカンマと引用符を確かめる。2 つ目も届かない → Linux 側の `tailscale serve` の待ち受けが 443 番か、tag が付いているかを確かめる。1 つ目が届いてしまう → `src` が `group:testers` だけを指しているか、別の古い rule（既定の `*` → `*`）が残っていないかを確かめる。

---

## 7. 戻し方

agent-link をやめて sai-moon だけにするとき。sai-moon は止めていないので、戻す物は少ない。古い `.sai/` と `~/.sai` は手を付けていない。

1. agent-link の daemon を、使っているプロジェクトごとに止める。

   ```bash
   (cd ~/workdir/Sai/sai-tree && agent-link daemon stop)
   (cd ~/workdir/Sai/agent-link && agent-link daemon stop)
   ```

2. agent-link の Web（8400 番）を止め、Tailscale の `:4446` を閉じる。

   ```bash
   kill $(lsof -tiTCP:8400 -sTCP:LISTEN)
   tailscale serve --https=4446 off
   tailscale serve status
   ```

3. `~/.zshrc` から agent-link の PATH の行と `agent-link-web-restart` を消す（任意）。sai-moon の行は触らない。
4. プロジェクトの `.mcp.json` を init の前に戻したいときは、`.bak` から戻す（`.bak.<日時>` のときは、その名前に置き換える）。`.claude/settings.json` は 4.2 で sai-moon 版に戻してあるので、そのままでよい。

   ```bash
   cd ~/workdir/Sai/sai-tree
   ls -a .mcp.json* | grep bak
   cp .mcp.json.bak .mcp.json
   ```

**確かめ方**: `lsof -nP -iTCP:8400 -sTCP:LISTEN` が何も出さない。`lsof -nP -iTCP:8200 -sTCP:LISTEN` は sai-moon の Web を出す。`tailscale serve status` に `:4444` だけが残る。sai-moon の Web の画面に、古い記録が見える。

**うまくいかないとき**: 古い画面が iPhone に残るときは、3 の最後の手順（アプリを閉じて開き直す・サイトのデータを消す）を行う。agent-link の `.agent-link/` は、戻しても消えない。もう使わないなら、利用者の判断で消す。
