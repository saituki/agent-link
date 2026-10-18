# 遠隔の機械で子の claude を動かす手順書

この文書は、利用者が上から順に実行する手順書である。**agent や worker は、この手順を実行しない**（Mac の `~/.ssh/authorized_keys` と遠隔の機械を書き換えるため）。

- 命令は、そのまま貼れる形で書いてある。遠隔の機械の例は `wsl-server`（WSL）。
- 決めたこと: 子タスクの claude だけを遠隔の機械で動かす。`agent-link` 本体・DB・文書・Web は Mac に置いたまま。

---

## 1. 何ができるか

親が `task create` に `"host":"wsl-server","cwd":"~/work/sai-tree"` を付けると、その子の claude は WSL で動く。画面（pane）・状態・文書・質問・報告は、Mac で動く子と同じに見える。

```
Mac                                                      遠隔の機械（wsl-server）
┌──────────────────────────────┐                  ┌────────────────────────────┐
│ tmux の pane                 │   ssh -t         │ claude（子）               │
│  └ ssh -t wsl-server '…' ────┼─────────────────▶│  cd ~/work/sai-tree        │
│                              │                  │   │ フック・agent-link の命令│
│ agent-link remote-gate（門番）│◀── ssh（鍵）─────┼───┘                        │
│  └ Mac の agent-link         │   中継              │ ~/.agent-link-remote/bin/  │
│     （state・doc・task report）│                  │   agent-link（中継）       │
└──────────────────────────────┘                  └────────────────────────────┘
```

- Mac の pane が `ssh -t wsl-server 'cd … && claude …'` を動かす。`task capture`・`task send`・`task interrupt` は、今までどおり pane に効く。
- 遠隔の claude のフックと `agent-link` の命令は、遠隔の PATH にある中継（`~/.agent-link-remote/bin/agent-link`）が ssh で Mac の門番（`agent-link remote-gate`）に送る。門番は決めた命令だけを通す（9 を見る）。
- コードは遠隔の git の写しで直し、親が `git fetch` で取り込む（5・7 を見る）。

---

## 2. 先にすること

### 2.1 Mac の `~/.ssh/config` に `IdentityFile` を足す

`Host wsl-server` に、WSL に入れる鍵を足す。

```
Host wsl-server
  ...（今ある行）
  IdentityFile ~/.ssh/id_ed25519_windows
```

確かめ方: `ssh -o BatchMode=yes wsl-server true` が、何も出さずに終わる（終了コード 0）。`Permission denied` が出るなら、`IdentityFile` が足りない。agent-link は `BatchMode=yes`（パスワードを聞かない）で ssh を呼ぶので、鍵だけで入れる必要がある。

### 2.2 Mac の「リモートログイン」を有効にする

遠隔の機械から Mac に入るため。システム設定 → 一般 → 共有 → リモートログイン をオンにする。確かめ方: WSL から `ssh <Mac のユーザー>@<Mac の住所> true`（鍵を渡す前なのでパスワードを聞かれてよい。「接続を拒否」が出なければよい）。

### 2.3 遠隔に claude を入れてログインする

```bash
ssh -t wsl-server 'claude'
```

ログインを済ませて閉じる。agent-link は `~/.local/bin/claude` と PATH の両方を探す。

### 2.4 作り直して、daemon を起こし直す

```bash
cd ~/workdir/Sai/agent-link
cargo build --release
agent-link daemon stop      # 作業中の子があれば、終わってから
agent-link daemon start
```

`agent-link remote setup` の行が `authorized_keys` に入るのは、`target/release/agent-link` の絶対パスである。作り直したあとは、3 をもう一度実行するとパスが入れ替わる（`updated`）。

---

## 3. `agent-link remote setup wsl-server`

一度だけの準備。

```bash
agent-link remote setup wsl-server
```

遠隔の `~/.agent-link-remote/` に、次が置かれる。

| パス | 中身 |
|---|---|
| `bin/agent-link` | 中継スクリプト（0755）。命令を ssh で Mac の門番へ送る |
| `ssh_config` | 中継が Mac へ入る ssh の設定（0600。Host の別名は `agent-link-mac`） |
| `id_ed25519`・`id_ed25519.pub` | 中継の鍵（パスフレーズ無し） |
| `known_hosts`・`cm-*` | Mac の host key・ssh の多重化の socket（中継の ssh が作る） |
| `runs/<run_id>/` | 子を起こすたびに置く、遠隔版の cc の設定と固定部分（system prompt） |

Mac の `~/.ssh/authorized_keys` には、次の 1 行が足される。

```
command="/Users/<あなた>/workdir/Sai/agent-link/target/release/agent-link remote-gate",restrict ssh-ed25519 AAAA… 
```

- この鍵で入ると、sshd は送られた命令を実行せず、必ず `agent-link remote-gate` を起こす。`restrict` で、tty・ポート転送・エージェント転送も止まる。
- 何度実行しても行は増えない（結果の `"authorized_keys"` が `unchanged`）。`agent-link` のパスが変わったときだけ `updated`（行を入れ替える）。
- 最後に 4 の 4 項目を確かめる。落ちても、足した行は残る。

WSL から Mac に届かないとき（`remote_to_mac` が落ちる）は、Mac の住所を渡す。Tailscale の住所を使う。

```bash
agent-link remote setup wsl-server --mac 100.94.91.98
```

`--mac` を省くと、ssh の接続（`SSH_CONNECTION`）から読む。

---

## 4. `agent-link remote check wsl-server`

疎通だけを確かめる（何も書かない）。4 項目を順に確かめ、落ちた項目で止める。

```bash
agent-link remote check wsl-server
```

| 項目 | 確かめること | 落ちたときの直し方 |
|---|---|---|
| `mac_to_remote` | Mac から遠隔に ssh で入れる | `~/.ssh/config` の `IdentityFile`（2.1）。`ssh -o BatchMode=yes wsl-server true` で確かめる |
| `claude` | 遠隔に `claude` がある（`claude --version`） | 2.3。`~/.local/bin` か PATH に置く |
| `relay` | 遠隔に中継と `ssh_config` がある | `agent-link remote setup wsl-server` |
| `remote_to_mac` | 遠隔の中継から Mac の門番へ通る（`agent-link-remote-ok` が返る） | Mac のリモートログイン（2.2）、`authorized_keys` の行、`--mac`（3）。遠隔で `~/.agent-link-remote/bin/agent-link remote-ping` を手で実行して、標準エラーを見る |

落ちると、標準エラーの最後の行が `Remote check failed: <項目>: <理由>`（終了コード 1）。

---

## 5. 遠隔の git の写しを用意する

遠隔の子は、遠隔の git の写しで作業する。Mac のフォルダは見えない。一度だけ作る。

```bash
ssh wsl-server 'git init -q ~/work/sai-tree'
git push wsl-server:work/sai-tree HEAD:refs/heads/al/base
ssh wsl-server 'cd ~/work/sai-tree && git switch -q al/base'
```

- 子ごとのブランチは、親が `git push wsl-server:work/sai-tree HEAD:refs/heads/al/<ID>` で送る（`<ID>` は子の ID。例 `U01`）。
- 遠隔の写しの現在のブランチへは push できない（git が断る）。`al/base` に居る間は、`al/<ID>` へ送る。

---

## 6. 親から使う

親（agent-link の main や kantoku）が `task create` に `host` と `cwd` を付ける。

```bash
git push wsl-server:work/sai-tree HEAD:refs/heads/al/U01
agent-link task create <<'EOF'
[
  { "client_id": "U01", "subject": "U01: 翻訳の失敗を直す", "block_type": "worker",
    "host": "wsl-server", "cwd": "~/work/sai-tree",
    "task_text": "ブランチ al/U01 に switch して作業し、コミットせよ。…" }
]
EOF
agent-link task execute --batch <batch_id>
agent-link task wait <task_id>
```

- `host` は Mac の `~/.ssh/config` の Host 名。`cwd` は遠隔のフォルダで、`/` か `~/` で始める（`~` だけは不可。家のフォルダは `~/` と書く）。Mac では存在を確かめない。遠隔に無ければ、子は起動せず `The folder does not exist on wsl-server: …` で失敗する。
- `worktree` は付けない（付けるなら `""` か `"none"`）。作業用コピーは作らない。
- `task capture`（遠隔の画面）・`task send`・`task interrupt`・`task finalize` は、Mac の子と同じに使える。
- 子の文書（`agent-link doc write`）と `agent-link task report` は、中継を通って Mac の run に入る。`task wait` の `report` と `outputs` も同じに返る。
- 子の会話記録は、子が終わるときに Mac へ写される（`agent-link run md <run_id> --kind message`）。
- **拡張の CLI を名前で呼べる。** 遠隔の子の PATH の先頭に `~/.agent-link-remote/runs/<run_id>/ext-bin/` があり、そのプロジェクトで有効で `cli` を持つ拡張ごとに、同じ名前の中継（例 `hello`）が入る。中継は Mac の `agent-link ext run <name> [--stdin] -- <引数…>` を門番経由で呼ぶ（標準入力が端末でなければ `--stdin` が付く）。つまり拡張の CLI は **Mac で動き**、遠隔の機械では動かない。上限は 120 秒（超えたら終了コード 1）。拡張の CLI が読み書きするのは Mac のファイルで、遠隔のファイルではない。遠隔の子が拡張の結果を残したいときは、`doc write` を使う。

---

## 7. 直したコードを取り込む

子が終わったら、親が確かめてから取り込む。

```bash
git fetch wsl-server:work/sai-tree al/U01
git merge FETCH_HEAD
```

マージごとにビルドとテストを行う（`link-task` の「作業場所」と同じ）。

---

## 8. 実機で確かめる手順（利用者が行う）

Mac の `~/.ssh/authorized_keys` と WSL を変えるので、利用者が行う。上から順に。

1. `ssh -o BatchMode=yes wsl-server true` が通る。
2. `agent-link remote setup wsl-server` が ok を返す。もう一度実行して `unchanged` になる。
3. `agent-link remote check wsl-server` が 4 項目とも ok。
4. 次の 2 つを試す。
   - 中継の鍵で別の命令を送ると断られる: `ssh wsl-server '~/.agent-link-remote/bin/agent-link task list'` が終了コード 2。
   - 門番を通さない ssh は入れない: `ssh wsl-server 'ssh -F ~/.agent-link-remote/ssh_config agent-link-mac ls'` が断られる（`ls` は 16 進でないので `Remote gate needs hex tokens.`）。
5. 親（agent-link の main など）に、次の子を頼んで実行させる。
   ```json
   {"client_id":"T1","subject":"T1: 遠隔の試し","block_type":"sub","host":"wsl-server","cwd":"~/","task_text":"uname -a と nvidia-smi の結果を doc write で保存し、task report せよ"}
   ```
6. 確かめること:
   - `task capture` に遠隔の画面が出る。
   - 完了の `report` と `outputs` が返る。
   - `agent-link run md <子の run_id> --kind message` に会話が出る。
7. 子が作業中に `task interrupt` と `task send` を試す。子が `AskUserQuestion` を出す依頼を 1 回試し、iPhone に質問が届くことを見る。
8. 拡張の CLI を遠隔の子から呼べることを確かめる（Mac に見本の拡張 `hello` を入れて有効にしておく。`docs/for-developers.md` の `ext` の命令を見る）。次の子を頼んで実行させる。
   ```json
   {"client_id":"T2","subject":"T2: 遠隔から拡張を呼ぶ","block_type":"sub","host":"wsl-server","cwd":"~/","task_text":"hello greet x を実行し、出力を doc write で保存して task report せよ"}
   ```
   - 保存された文書に `{"hello": "x"}` が出る（拡張の CLI は Mac で動いたので、遠隔に `hello` が無くても通る）。

未確認（実機で見る）: `ssh -t` の中の claude に、確定キー（`M-Enter`）と貼り付けが届くか。届かないと、子は `The agent did not report that it started working within 15 seconds …` で失敗する。

---

## 9. 安全のこと

### 門番が通す命令

遠隔の鍵で入ると、`agent-link remote-gate` だけが起きる。通す命令は `crates/agent-link-engine/src/remote.rs` の `gate_plan` が決める。

| 引数 | 門番がすること |
|---|---|
| `cc-state-hook <working\|idle\|blocked\|tool>`・`cc-prompt-hook`・`cc-permission-hook`・`cc-statusline` | Mac の `agent-link` をその引数で実行する |
| `doc write …`・`doc read …`・`doc list …`・`task report …` | 同上 |
| `web-hook cc_register`・`web-hook cc_question` | Mac の `apps/web/hooks/<名>.py` を実行する |
| `ext run <name> [--stdin] -- …`（`<name>` は拡張の名前の形 `^[a-z][a-z0-9-]{0,31}$`。`--stdin` は `--` の前だけ） | Mac の `agent-link` をその引数で実行する（有効かどうかは `ext run` が確かめる。上限 120 秒） |
| `remote-ping` | `agent-link-remote-ok` を出して終わる |

- ほかの命令（`task create`・`run`・`card`・`daemon` など。`ext` の `run` 以外の `install`・`enable`・`config` なども含む）は、終了コード 2・`Remote gate does not allow this command: …` で断る。
- 環境変数は 7 つ（`HERDR_PANE_ID`・`AGENT_LINK_PROJECT_ROOT`・`AGENT_LINK_PARENT_RUN_ID`・`AGENT_LINK_BLOCK_NAME`・`AGENT_LINK_IS_SUBTASK`・`AGENT_LINK_WEB_URL`・`AGENT_LINK_HOME`）だけを、値の形を検査して通す。
- 命令はシェルを通さず、引数の配列で起こす。

つまり、遠隔の機械が乗っ取られても、できるのは「Mac の run に文書と報告を書く」「状態のフックを呼ぶ」「有効な拡張の CLI を名前で呼ぶ」ことまでである。Mac のファイルを自由に読む・書く・任意の命令を走らせることはできない（拡張の CLI は、拡張が決めた処理だけを Mac で行う。信用できない拡張を有効にしない）。ただし `AGENT_LINK_PROJECT_ROOT` を変えれば、Mac の別のプロジェクト（`.agent-link/` を持つフォルダ）の run に文書を書ける。

### 鍵を外す

1. Mac の `~/.ssh/authorized_keys` から、`remote-gate` を含む行を消す。
2. 遠隔の `~/.agent-link-remote` を消す（`ssh wsl-server 'rm -rf ~/.agent-link-remote'`）。

### 共有しない

中継の鍵（`~/.agent-link-remote/id_ed25519`）は、遠隔の家のフォルダに、パスフレーズ無しで置かれる。その機械に入れる人は、門番を通る命令を Mac へ送れる。遠隔の機械を他の人と共有しない。

---

## 10. できないこと（範囲外）

- カードの run（`card run`）を遠隔で動かすこと。`task create` の子だけ。
- 孫の子タスク。遠隔の子は `task create` を使えない（門番が断る）。
- MCP と `.agent-link/settings.json` の `env` を渡すこと（Mac のパスが入りうるため、渡さない）。
- 失敗した子の会話記録を Mac に写すこと。写すのは、終わるときの記録だけ。
- 遠隔のファイルを Mac から読むこと。親に渡す物は、文書か git のコミットにする。
- dgx は、claude を入れれば同じ手順で使える見込み（推測。未確認）。
