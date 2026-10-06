# 切り替えの手順書（古い名前・置き場から新しい名前・置き場へ）

利用者が **1 回だけ** 行う手順です。上から順に、シェル（zsh）へコピーして実行します。
エージェントは実行しません（利用者の環境を変える操作です）。

## 何が変わるか

| | 古い | 新しい |
|---|---|---|
| 実行ファイル | `sai-moon-link`・`sai-moon-paper`・`*-tui`・`sai-moon-paper-cite` | `sai-link`・`sai-paper`・`sai-sci`（`target/release/`）と `sai-web`（`apps/web/bin/`） |
| Web のフォルダ | `apps/phone`（`sai_phone`） | `apps/web`（`sai_web`） |
| 利用者全体の置き場 | `~/.sai-moon`（環境変数 `SAI_SAI_HOME`） | `~/.sai`（環境変数 `SAI_HOME`） |
| Web のデータ | `apps/phone/src/data/`（DB・通知の鍵）と `<置き場>/phone/` | `~/.sai/web/`（`sai_web.db`・`vapid.json`・`runs/` など） |
| Web の環境変数 | `SAI_PHONE_*` | `SAI_WEB_*` |
| launchd | `com.sai-moon`（今は使っていない） | `com.sai.web` |
| エージェントが sai を呼ぶ道 | MCP の `sai`（`sai_write` など） | Bash で CLI（`sai-link doc write` など） |

古い `~/.sai`（古い sai-link の置き場）は `~/.sai-old` に移して残します。

## 前提

- 実装の統合（`apps/phone` → `apps/web` の改名・CLI の改名・MCP の廃止）が本体のフォルダにマージ済みであること。
- 本体のフォルダで `cargo build --release` を済ませ、`target/release/` に `sai-link`・`sai-paper`・`sai-sci` があること。
  古い `sai-moon-link` などは **消えずに残る**（片付けは最後の段）。

```bash
ls "$HOME/workdir/Sai/sai-moon/target/release/" | grep -E '^sai-'
```

同じシェルで最後まで続けます。変数を 2 つ置きます（`export` しない。`SAI_HOME` を export すると、
途中で呼ぶ古い命令と混ざるため）。

```bash
REPO="$HOME/workdir/Sai/sai-moon"        # 本体のフォルダ
SAI_HOME="$HOME/.sai"                    # 新しい置き場（コマンドの行では $SAI_HOME と書く）
OLD="$REPO/target/release/sai-moon-link" # 古い実行ファイル（段 0 で使う）
env | grep '^SAI_'                       # 何も出ないのが正しい（出たら unset する）
```

`env | grep '^SAI_'` に `SAI_PROJECT_ROOT`・`SAI_PARENT_RUN_ID`・`SAI_BLOCK_NAME` が出るときは、
エージェントの中のシェルです。普通のターミナルで開き直してください。

---

## 0. 動いている作業を止める

**何をするか**: 登録済みのプロジェクトで、動いている run（review を含む）がすべて終わっていることを確かめ、
daemon・Web・エージェントのプロセスを止める。Radio（8201・8202）と MLflow は **そのまま**。
旧 sai-phone（8100）・旧 sai-radio（8000）も触らない。

### 0-1. プロジェクトの一覧

```bash
python3 -c 'import json,os; [print(p["root"]) for p in json.load(open(os.path.expanduser("~/.sai-moon/registry.json")))]'
```

出た root を、以降の段で `$root` として使う（段 6 でも同じ一覧を使う）。

### 0-2. 動いている run が無いことを確かめる

`running` と `review` に run が 1 件も無いのが正しい。

```bash
python3 -c 'import json,os; [print(p["root"]) for p in json.load(open(os.path.expanduser("~/.sai-moon/registry.json")))]' |
while IFS= read -r root; do
  echo "== $root"
  (cd "$root" && SAI_PROJECT_ROOT="$root" "$OLD" phone tree | python3 -c '
import json, sys
def walk(items):
    for i in items:
        if i.get("type") == "run" and i.get("lane") in ("running", "review"):
            print("  残り:", i["lane"], i["run_id"], i.get("label"))
        walk(i.get("children", []))
walk(json.load(sys.stdin)["items"])')
done
```

- `running` が残る: その run が終わるのを待つ（または Web・TUI から終了させる）。
- `review` が残る: 内容を確かめて done へ進める（Web の「→ done」、または `(cd "$root" && SAI_PROJECT_ROOT="$root" "$OLD" phone advance <run_id>)`）。

**確かめ方**: 上の命令が「`== <root>`」の行だけを出し、「残り:」が 1 行も出ない。

### 0-3. daemon を止める

```bash
python3 -c 'import json,os; [print(p["root"]) for p in json.load(open(os.path.expanduser("~/.sai-moon/registry.json")))]' |
while IFS= read -r root; do
  echo "== $root"
  (cd "$root" && SAI_PROJECT_ROOT="$root" "$OLD" daemon stop)
done
```

実行中の作業があると `daemon stop` は止めてくれない。その場合は 0-2 に戻る。

### 0-4. エージェントのプロセスを閉じる

エージェントの pane ごとに `sai-moon-link mcp serve` が動いている。残ったままだと、置き場を移したあとに古い場所を見る。

```bash
pgrep -fl 'sai-moon-(link|paper)'
```

`daemon _run` が残っていれば 0-3 をやり直す。`mcp serve` が出たら、そのエージェント（cc・pi）の pane を閉じる。
全部閉じたら、同じ命令が何も出さない（この手順を読んでいるエージェントの pane は最後に閉じる）。

### 0-5. Web（8200）を止める

launchd で動いているときと、手で起こしたときの 2 通りがある。

```bash
launchctl list | grep -iE 'sai|com\.sai-moon'          # launchd に載っているか
ls ~/Library/LaunchAgents | grep -i sai                # 入っている plist
lsof -nP -iTCP:8200 -sTCP:LISTEN                       # 8200 を待ち受けているプロセス
```

- launchd で動いている（`com.sai-moon` が出る）:
  `launchctl unload ~/Library/LaunchAgents/com.sai-moon.plist`
- 手で起こした（`uvicorn sai_phone.main:app --port 8200` の行が `lsof` に出る。ログは `~/.sai-moon/phone/uvicorn-8200.log`）:
  `lsof` の PID を確かめて `kill <PID>` する。

**確かめ方**: `lsof -nP -iTCP:8200 -sTCP:LISTEN` が何も出さない。
`lsof -nP -iTCP:8201 -iTCP:8202 -sTCP:LISTEN` は Radio が残っているので出てよい。

---

## 1. 古い実行ファイルを PATH から外す

**何をするか**: 古い sai-link・sai-paper のリポジトリが入れた `sai-link`・`sai-paper` が、新しい物より先に見つからないようにする。
`sai-moon-*`（今の本体を指すリンク）は、戻せるように段 10 まで残す。

### 1-1. 場所を確かめる

```bash
which -a sai-link sai-paper sai-sci sai-web sai-moon-link sai-moon-paper
ls -l ~/.local/bin | grep -i sai
cargo install --list | grep -iE '^sai-(link|paper)'
grep -n 'sai' ~/.zshrc ~/.zprofile ~/.zshenv ~/.bashrc 2>/dev/null
```

見る所:

- `~/.local/bin/sai-paper`・`sai-paper-tui` が、古い `~/workdir/Sai/sai-paper/target/release/` を指すリンクなら外す。
- `~/.cargo/bin/sai-link` など `cargo install` が入れた物が出たら外す。
- `~/.zshrc` などに、古いリポジトリの `target/release` を PATH に足す行（例 `export PATH="$HOME/workdir/Sai/sai-paper/target/release:$PATH"`）があれば消す。

### 1-2. 外す

```bash
# リンクは名前を変えて外す（段 10 まで残して戻せるようにする）
[ -L ~/.local/bin/sai-paper ] && mv ~/.local/bin/sai-paper ~/.local/bin/sai-paper-old
[ -L ~/.local/bin/sai-paper-tui ] && mv ~/.local/bin/sai-paper-tui ~/.local/bin/sai-paper-tui-old

# cargo install が入れた物（1-1 で出たときだけ）
# cargo uninstall sai-link
# cargo uninstall sai-paper
```

シェルの設定（`~/.zshrc` など）から古い PATH の行を、エディタで消す。

**確かめ方**: 新しいシェルを開き（または `hash -r`）、`which -a sai-link sai-paper` が **何も出さない**（段 3 で新しい物を置く前）。

---

## 2. 置き場を移す（古い `~/.sai` を `~/.sai-old` へ、`~/.sai-moon` を `~/.sai` へ）

**何をするか**: 古い sai-link の置き場を退避し、今の置き場を新しい名前に移し、`phone/` を `web/` に直す。

```bash
ls -ld ~/.sai ~/.sai-moon ~/.sai-old
```

`~/.sai-moon` があり、`~/.sai-old` が **無い** のが正しい。`~/.sai-old` が既にあるときは、中身を確かめて名前を変えてから進める。

```bash
[ -d ~/.sai ] && mv ~/.sai ~/.sai-old   # 古い sai-link の置き場（無ければ飛ばす）
mv ~/.sai-moon ~/.sai
mv ~/.sai/phone ~/.sai/web              # browser-profile・browser-downloads・runs・cc-rate-limits.json・ログごと
```

古い `~/.sai-old` の `blocks/`（古い sai-link の共通 block）を使い続けたいときは、必要な物だけ `~/.sai/blocks/` へ手で写す。

**確かめ方**:

```bash
ls ~/.sai          # registry.json・template・sci・exec・paper.env・web … が見える
ls ~/.sai/web      # runs・cc-rate-limits.json・browser-profile … が見える
ls -d ~/.sai-old ~/.sai-moon   # ~/.sai-old はある。~/.sai-moon は「No such file」
```

`SAI_SAI_HOME` をシェルの設定（`~/.zshrc` など）で設定していれば、行ごと消すか `SAI_HOME` に替える（段 3 でまとめて直す）。

---

## 3. 新しい実行ファイルを置く

**何をするか**: PATH に新しい `target/release` と `apps/web/bin` を足す。

```bash
cat >> ~/.zshrc <<EOF

# sai（sai-link・sai-paper・sai-sci・sai-web）
export PATH="$REPO/target/release:$REPO/apps/web/bin:\$PATH"
EOF
exec zsh -l          # 設定を読み直す（開き直しでもよい。変数 REPO・SAI_HOME は置き直す）
```

シェルを開き直したら、前提で置いた 3 つの変数（`REPO`・`SAI_HOME`・`OLD`）を置き直す。

**確かめ方**:

```bash
which sai-link sai-paper sai-sci sai-web
# 4 つとも $REPO/target/release/ か $REPO/apps/web/bin/ の下を指す
for b in sai-link sai-paper sai-sci sai-web; do "$b" --help >/dev/null 2>&1 && echo "ok: $b"; done   # 4 行出る
grep -nE 'SAI_SAI_HOME|SAI_PHONE_' ~/.zshrc ~/.zprofile ~/.zshenv 2>/dev/null   # 出なければよい
```

最後の `grep` に出たら、`SAI_SAI_HOME` は `SAI_HOME` に、`SAI_PHONE_` は `SAI_WEB_` に書き直す。

---

## 4. Web のデータを移す

**何をするか**: DB・通知の鍵・設定（`.env`）を新しい場所へ **写す**。`apps/phone/` は段 10 まで消さない（戻せるようにする）。

```bash
ls -l "$REPO/apps/phone/src/data" "$REPO/apps/phone/.env"
ls -d "$REPO/apps/web"
```

`sai_phone.db`・`vapid.json`・`.env` があるのが正しい。Web が止まっている（段 0-5）ことを確かめてから写す。

```bash
mkdir -p ~/.sai/web
cp -p "$REPO/apps/phone/src/data/sai_phone.db" ~/.sai/web/sai_web.db
for s in -wal -shm; do
  [ -f "$REPO/apps/phone/src/data/sai_phone.db$s" ] && cp -p "$REPO/apps/phone/src/data/sai_phone.db$s" "$HOME/.sai/web/sai_web.db$s"
done
cp -p "$REPO/apps/phone/src/data/vapid.json" ~/.sai/web/vapid.json   # 通知の鍵。移さないと iPhone の通知を登録し直すことになる
```

`.env`（合言葉などが入っている）は、名前を新しい物に直しながら写す。**中身を画面に出さない**。

```bash
[ -e "$REPO/apps/web/.env" ] && echo "既にある。上書きしない" || {
  sed -e 's/SAI_SAI_HOME/SAI_HOME/g' \
      -e 's/SAI_PHONE_/SAI_WEB_/g' \
      -e 's/sai-moon-link/sai-link/g' \
      -e 's/sai-moon-paper/sai-paper/g' \
      "$REPO/apps/phone/.env" > "$REPO/apps/web/.env"
  chmod 600 "$REPO/apps/web/.env"
}
```

仮想環境を作る。

```bash
(cd "$REPO/apps/web" && uv sync)
```

**確かめ方**（合言葉の値は出さない）:

```bash
ls -l ~/.sai/web/sai_web.db ~/.sai/web/vapid.json
sed -E 's/=.*/=<値>/' "$REPO/apps/web/.env" | grep -v '^#' | grep -v '^$'   # 項目名だけ見る
grep -c -E 'SAI_PHONE_|SAI_SAI_HOME|sai-moon|\.sai-moon' "$REPO/apps/web/.env"   # 0 が正しい
ls "$REPO/apps/web/.venv/bin/uvicorn"
```

- 項目名が `SAI_WEB_TOKEN`・`SAI_WEB_PORT`・`SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_WEB_OPENROUTER_ACCOUNTS` などになっている。
- 数えた結果が 0 でないときは、`.env` の値に古い名前（例: 絶対パスの `…/.sai-moon/…`）が残っている。エディタで直す。
- `SAI_HOME` を `.env` に書いていれば、値は `~/.sai` の絶対パスにする（書いていなければ既定で `~/.sai`）。

---

## 5. launchd の plist を入れ替える

**何をするか**: 新しい plist（ラベル `com.sai.web`）を作る。古い `com.sai-moon.plist` があれば退避する。

```bash
ls ~/Library/LaunchAgents | grep -i sai    # 古い com.sai-moon.plist があるか（今は手で起こしているなら無い）
ls "$REPO/apps/web/launchd/"
```

```bash
mkdir -p ~/.sai/web
sed -e "s|__REPO__|$REPO|g" -e "s|__SAI_HOME__|$HOME/.sai|g" \
  "$REPO/apps/web/launchd/com.sai.web.plist.template" > ~/Library/LaunchAgents/com.sai.web.plist

# 古い plist があれば、戻せるように ~/.sai-old へ退避する
[ -f ~/Library/LaunchAgents/com.sai-moon.plist ] && mv ~/Library/LaunchAgents/com.sai-moon.plist ~/.sai-old/
```

**合言葉（`SAI_WEB_TOKEN`）の渡し方**: これまで plist には書かず、`apps/phone/.env` に書いていた。
今回も同じで、段 4 で写した `apps/web/.env` から読む（plist には書かない）。
plist の `EnvironmentVariables` に書く方法でもよいが、その場合は `.env` の同じ項目を消して、値を 1 か所にする。

**確かめ方**:

```bash
plutil -lint ~/Library/LaunchAgents/com.sai.web.plist     # OK
grep -c '__' ~/Library/LaunchAgents/com.sai.web.plist     # 0（埋め忘れが無い）
grep -c 'sai-moon\|apps/phone' ~/Library/LaunchAgents/com.sai.web.plist   # 0
```

---

## 6. 登録済みのプロジェクトごとに `init --update`

**何をするか**: 各プロジェクトの `.mcp.json`・`.claude/settings.json`・`.pi/settings.json`・`.sai/settings.json` にある
古い実行ファイルと `apps/phone/hooks`・`ask_user_phone.ts` の絶対パスを、新しい物に書き直す。block・template も新しい版に置き換わる。

- プロジェクトの種類で命令が違う:

| 種類 | 見分け方 | 命令 |
|---|---|---|
| 研究のプロジェクト | `<root>/.sai/sci/` か `<root>/.sai/tools/` がある | `sai-sci init --update`（中で `sai-link init --update` と `sai-paper init --update` を呼び、`pre_run`・`context`・`write_check`・`job_status` も新しい実行ファイルに書き直す） |
| 論文だけのプロジェクト | `<root>/.sai/paper/` があり、`.sai/sci/` と `.sai/tools/` が無い | `sai-link init --update` と `sai-paper init --update` |
| それ以外 | | `sai-link init --update` |

- **本体のリポジトリ（sai-moon 自身）も 1 つのプロジェクト**として登録簿にある。同じ命令を打つ。
- block の `model:`・`provider:`・`thinking:` は残る。中身が変わった block・template は `.bak` に退避される（消えない）。
- `init` が「`.mcp.json`・`.claude/`・`.pi/` を git commit してください」と出しても、
  **本体のリポジトリでは commit しない**（段 7 で追跡をやめる）。ほかのプロジェクトでは、worktree の cc・pi が読むので commit する。

```bash
python3 -c 'import json,os; [print(p["root"]) for p in json.load(open(os.path.expanduser("~/.sai/registry.json")))]' |
while IFS= read -r root; do
  [ -d "$root/.sai" ] || { echo "== $root: .sai が無いので飛ばす"; continue; }
  if [ -d "$root/.sai/sci" ] || [ -d "$root/.sai/tools" ]; then
    cmd="sai-sci init --update"
  elif [ -d "$root/.sai/paper" ]; then
    cmd="sai-link init --update && sai-paper init --update"
  else
    cmd="sai-link init --update"
  fi
  echo "== $root: $cmd"
  (cd "$root" && SAI_PROJECT_ROOT="$root" sh -c "$cmd") || echo "!! 失敗: $root"
done
```

`!! 失敗` が出たら、そのプロジェクトのエラーを読んで直し、そのプロジェクトだけ同じ命令を打ち直す。

**確かめ方**: 古い名前と古い絶対パスが残っていない（各プロジェクトで）。

```bash
python3 -c 'import json,os; [print(p["root"]) for p in json.load(open(os.path.expanduser("~/.sai/registry.json")))]' |
while IFS= read -r root; do
  echo "== $root"
  grep -nE 'sai-moon-|apps/phone|ask_user_phone|\.local/bin/sai-moon|\.sai-moon' \
    "$root/.mcp.json" "$root/.claude/settings.json" "$root/.pi/settings.json" "$root/.sai/settings.json" 2>/dev/null
done
```

- 何も出ないのが正しい。
- 本体のリポジトリの `.mcp.json` に MCP の `sai` が残らず、`.claude/settings.json` のフックが新しい実行ファイル（`sai-link`）と `apps/web/hooks` を指す。
- **論文だけのプロジェクト**で `.sai/settings.json` の `pre_run` が古い `sai-moon-paper` のまま残ることがある
  （`sai-paper init --update` は `pre_run` を書かない）。出たら次のどちらかにする:
  - 研究の現況を毎回エージェントに渡したい: `(cd "$root" && SAI_PROJECT_ROOT="$root" sai-sci init --update)`。研究の部品がそのプロジェクトに入る。
  - 要らない: `.sai/settings.json` の `pre_run` の項目をエディタで消す。

---

## 7. 生成物の追跡をやめる（本体のリポジトリで）

**何をするか**: `.mcp.json`・`.claude/settings.json`・`.claude/commands/`・`.claude/skills/` は、`init` が作る物でプロジェクトごとに違う（絶対パスを含む）。
`.gitignore` には書いてあるが、これまでの commit で追跡されている。追跡だけをやめる（**ファイルは消えない**）。

```bash
cd "$REPO"
git ls-files .mcp.json .claude | head      # 追跡されている物
git rm --cached .mcp.json .claude/settings.json
git rm -r --cached .claude/commands .claude/skills
git status --short                         # 「D」（追跡からの削除）だけが並ぶ。ファイルは残っている
git commit -m "chore: init が作る生成物の追跡をやめる"
```

**確かめ方**:

```bash
git ls-files .mcp.json .claude             # 何も出ない
ls .mcp.json .claude/settings.json .claude/commands .claude/skills   # ファイルは残っている
git status --short                         # きれい（.gitignore で隠れる）
```

---

## 8. daemon と Web を起動し直す

**何をするか**: Web を launchd で起こし、使うプロジェクトで daemon を起こす。

```bash
launchctl load ~/Library/LaunchAgents/com.sai.web.plist
```

- launchd を使わず手で起こすなら、代わりに次を使う（同時に両方は起こさない）:
  `(cd "$REPO" && nohup sai-web serve >> ~/.sai/web/sai-web.log 2>&1 &)`

**確かめ方**:

```bash
launchctl list | grep com.sai.web          # 1 行出る（PID が数字）
lsof -nP -iTCP:8200 -sTCP:LISTEN           # uvicorn が待ち受けている
tail -n 20 ~/.sai/web/sai-web.log          # エラーが出ていない
```

daemon は、使うプロジェクトごとに起こす。

```bash
(cd "<プロジェクトの root>" && SAI_PROJECT_ROOT="<プロジェクトの root>" sai-link daemon start)
(cd "<プロジェクトの root>" && SAI_PROJECT_ROOT="<プロジェクトの root>" sai-link daemon status)
```

エージェント（cc・pi）の pane は、開き直す（古い MCP の `sai` を持たない新しいプロセスになる）。

---

## 9. 動作を確かめる

どれも、プロジェクトの root で打つ（`SAI_PROJECT_ROOT` を付ける）。

```bash
cd "<プロジェクトの root>" && export SAI_PROJECT_ROOT="$PWD"

sai-link run list --limit 3          # 実行の一覧が出る（これまでの run が見える）
sai-web projects                     # 登録済みのプロジェクトが並ぶ
sai-web questions                    # 質問の一覧（無ければ空でよい）
```

- iPhone と PC で Web（`http://127.0.0.1:8200`、iPhone は今までの住所）を開き直す。
  PWA の名前が `sai` に変わる。ホーム画面のアイコンは、必要なら追加し直す。
  合言葉と下書きは引き継がれる（`SAI_WEB_TOKEN`・DB を移したため）。通知は `vapid.json` を移したので登録し直し不要。
- 研究のプロジェクトでは `sai-sci status --json` が JSON を出す。
- 試しのカードを 1 枚作って動かし、エージェントが `sai-link doc write` で保存できることを確かめる:

```bash
# runner は動く物にする（pi のモデルが使えないときは cc）
echo '{"runner":"cc","block":"main","description":"切り替えの確認","task_text":"`sai-link doc write --filename hello.md --description \"切り替えの確認\"` で、標準入力から「動作確認」と保存して、`sai-link run finalize` で終える。"}' \
  | sai-link card create --json                 # run_id が出る
sai-link card run <run_id> --json
sai-link doc list --run <run_id>                # hello.md が出れば保存できている
```

確かめたら、試しの run を消す: `sai-link run rm <run_id> --yes`（review のままなら先に `sai-link run advance <run_id>`）。

**確かめ方**: 上の 5 つがすべて通る。通らないものがあれば、段 10 に進まず、下の「戻し方」を使う。

---

## 10. 片付け（段 9 を確かめてから）

**何をするか**: 古い実行ファイル・古い置き場・古い Web のフォルダを消す。**戻せなくなる**ので、段 9 が通ってから行う。

### 10-1. 確かめる

```bash
ls -l ~/.local/bin | grep -i sai
ls "$REPO/target/release" | grep -E '^sai-moon-'
git -C "$REPO" ls-files apps/phone | head -3        # 何も出ない（追跡されているのは apps/web）
git -C "$REPO" status --short apps/phone            # 何も出ない（残っているのは追跡外の物だけ）
ls -d ~/.sai-old
```

### 10-2. 消す

```bash
# 古い実行ファイルのリンクと本体
rm -f ~/.local/bin/sai-moon-link ~/.local/bin/sai-moon-link-tui \
      ~/.local/bin/sai-moon-paper ~/.local/bin/sai-moon-paper-tui ~/.local/bin/sai-moon-paper-cite
rm -f ~/.local/bin/sai-paper-old ~/.local/bin/sai-paper-tui-old     # 段 1 で名前を変えた物
rm -f "${REPO:?}"/target/release/sai-moon-{link,link-tui,paper,paper-tui,paper-cite}

# 古い Web のフォルダ（追跡外の .venv・src/data・.env だけが残っている）
rm -rf "${REPO:?}/apps/phone"

# 古い置き場（古い sai-link の ~/.sai と、退避した plist）
rm -rf ~/.sai-old
```

**確かめ方**: `which -a sai-moon-link sai-moon-paper` が何も出さず、`sai-link run list --limit 1` が今までどおり動く。

---

## 戻し方（うまくいかないとき）

古い実行ファイルと `~/.sai-old`・`apps/phone/` は段 10 まで残してあるので、段 10 の前なら戻せる。

1. 新しい launchd を止める: `launchctl unload ~/Library/LaunchAgents/com.sai.web.plist`（手で起こした Web は `lsof -nP -iTCP:8200 -sTCP:LISTEN` の PID を `kill`）。
   起こした daemon も `sai-link daemon stop` で止める。
2. 置き場を戻す: `mv ~/.sai/web ~/.sai/phone && mv ~/.sai ~/.sai-moon && mv ~/.sai-old ~/.sai`
   （`~/.sai-old` が無かったときは最後の `mv` を飛ばす）。
3. 古い plist があれば `~/Library/LaunchAgents/` に戻す（`~/.sai-old/com.sai-moon.plist` があれば `mv` して `launchctl load`）。手で起こしていたなら、元のログの場所 `~/.sai-moon/phone/uvicorn-8200.log` へ向けて起こし直す。
4. `~/.local/bin/sai-paper-old` などを元の名前に戻す。PATH の行（段 1・3 で変えた物）を元に戻す。
5. 古い実行ファイルとフックは、古いコード（`apps/phone`・`sai-moon-*` の名前）を指す。
   本体のフォルダを切り替え前の commit に戻す（`git log` で統合の merge の前を探して `git switch --detach <commit>`）。
   各プロジェクトの `init --update` で書き直した設定も、古い実行ファイルで `sai-moon-link init --update` を打つと戻る（`.bak` も残っている）。
6. `apps/phone/` の `.env`・`src/data/` は段 4 で **写した** だけなので、そのまま使える。

戻したら、うまくいかなかった段と出力を控えて、開発側へ伝える。
