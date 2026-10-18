# sai-tree と agent-link を同じ Mac で分けて使う（片付けの手順）

対象: 同じ Mac で sai-tree（`~/workdir/Sai/sai-tree`）と agent-link（`~/workdir/Sai/agent-link`）の両方を使う利用者。
目的: 2 つの App が互いのデータ・プロセス・設定を壊さず、取り違えないこと。

> **エージェントはこの文書の手順を実行しない。** 点検も片付けも、利用者が自分の端末で行う。エージェントは `~/.zshrc`・`~/.local/bin`・`~/.agent-link`・`~/.sai`・登録簿（`registry.json`）・相手のリポジトリを変えない（決定 K4・2026-10-18）。

この文書は次の順に読む: 1 前提 → 2 点検（読むだけ）→ 3 決めること → 4 片付けの手順 → 5 戻し方 → 6 拡張を入れたあとの点検。

すべての命令で、動いている物を止めるときは **PID で止める**。`pkill -f` のような名前での一括の停止は使わない。消す前には、必ず中身を見る。

---

## 1. 目的と前提

### 1.1 名前空間は既に分かれている

2 つの App の名前（環境変数・置き場・ファイル名・ポート・実行ファイル）は、もう分かれている。この分離を崩さないことが前提になる。

| 資源 | sai-tree | agent-link |
|---|---|---|
| 環境変数の頭 | `SAI_*` | `AGENT_LINK_*` |
| 置き場 | `~/.sai` | `~/.agent-link` |
| プロジェクト置き場 | `<root>/.sai/` | `<root>/.agent-link/` |
| DB | `sai-engine.db` | `agent-link.db` |
| waggle の実体・発行者 | `.sai/sai-engine/waggle-blobs`・`sai-engine` | `.agent-link/engine/waggle-blobs`・`agent-link` |
| Web の既定ポート | 8200 | 8400 |
| Web の CLI | `sai-web` | `agent-link-web` |
| エンジンの実行ファイル | `sai-link` | `agent-link` |
| cc への設定の渡し方 | プロジェクトの `.claude/settings.json` に書く | `.agent-link/cc-settings.json` を作り `--settings` で渡す |
| 遠隔の子 | 無し | `~/.agent-link-remote`・`remote-gate`・`~/.ssh/authorized_keys` |
| 登録簿 | `~/.sai/registry.json` | `~/.agent-link/registry.json` |
| Cargo クレート | `sai-link-*` | `agent-link-*` |

### 1.2 共有が前提の物

次は分けられない。混ぜないことが目標になる。

- `claude` の実体とログイン
- `~/.claude/projects`（会話の記録）・`~/.claude/skills`・`plugins`・`CLAUDE.md`・`settings.json`（cc の全体のデータ）
- 外部 MCP（serena・donsetch・headroom・playwright）。各プロジェクトの `.mcp.json` が指す
- uv・Docker・Tailscale・macOS

### 1.3 この文書の根拠

§2 の「ぶつかっている物 C1〜C10」は、2026-10 の実測（要件定義書の下書き `docs/plan/requirements-extension.md` §1.3.3・git 未追跡）による。コードの状態は HEAD の時点で読んだ。

---

## 2. 点検（読むだけ）

この節の命令はすべて**読むだけ**。書き換えも止めることもしない。各項目の「良い状態」に当てはまれば、その項目は済んでいる。

まとめて見るには、agent-link のリポジトリで台本の `doctor` を使う（読むだけ）。

```bash
cd ~/workdir/Sai/agent-link && scripts/agent-link-setup doctor
```

`doctor` の `ownership` の行（§2 の C2）と `extensions` の行（拡張の宣言の誤り）が、この節の点検の一部を担う。

### C1 登録簿が互いを載せている

```bash
python3 -c 'import json,os;print([e["name"] for e in json.load(open(os.path.expanduser("~/.agent-link/registry.json")))])'
python3 -c 'import json,os;print([e["name"] for e in json.load(open(os.path.expanduser("~/.sai/registry.json")))])'
```

- **良い状態**: `~/.agent-link/registry.json` に sai-tree のプロジェクトが無い。`~/.sai/registry.json` に agent-link のプロジェクトが無い。

### C2 同じ root に両方の置き場がある

```bash
ls -d <root>/.sai <root>/.agent-link 2>&1
```

`<root>` は各プロジェクトのフォルダ（例: `~/workdir/Sai/agent-link`）。`doctor` では `ownership` の行に出る。

- **良い状態**: 各 root に `.sai/` と `.agent-link/` の**片方だけ**がある（例外にした root は §3 の D1 で決めたものだけ）。`doctor` の `ownership` が `ok ownership no project has .sai`（agent-link の root に `.sai` が無い）。

### C3 交差実行のプロセスが動いている

```bash
ps -axo pid,command | grep -E 'sai-link|agent-link' | grep -v grep
ls -d <root>/.sai/outputs/running/* <root>/.agent-link/outputs/running/* 2>/dev/null
```

- **良い状態**: 出てくる実行の行が、その root の物（`<root>/.sai/…` の run は sai-tree の root、`<root>/.agent-link/…` の run は agent-link の root）。相手の root のフォルダで動く run が無い。

### C4 同じ root で共有するファイルが取り合いになる

```bash
ls -la <root>/.mcp.json* <root>/.claude/commands <root>/.claude/skills <root>/.serena 2>&1
```

- **良い状態**: `.bak` が init のたびに増えていない。`frontend-design`・`ui-ux-rules`（同名の skill）が両方の App の物で入れ替わっていない（`.claude/skills/` の中身の日付が、最後に init した App と合う）。

### C5 旧い設定が残っている

```bash
grep -n 'sai-moon' <root>/.claude/settings.json ~/.claude/settings.json 2>/dev/null
```

- **良い状態**: 何も出ない。出た行は、古い App（sai-moon）のパスを指している。その行の設定は、§4 の片付けで扱う（sai-tree のリポジトリ自体は読むだけ。直さない）。

### C6 名前の汚染が残っている（コード）

```bash
grep -rn 'sai_link_exe\|sai_link_bin\|sai-engine' ~/workdir/Sai/agent-link/crates ~/workdir/Sai/agent-link/apps ~/workdir/Sai/agent-link/scripts | grep -v '/tests/'
```

- **良い状態**: 「由来の説明」以外に出ない。
- **今の状態（2026-10-18・HEAD）**: `sai-engine` は 0 件。`sai_link_exe`（`cc_command.rs`・`orchestrator.rs`）と `sai_link_bin`（`apps/web` の `config.py`・`control.py`）は残っている。これは改名（決定 P7・要件の C7）の作業が済んでいないためで、利用者の環境の問題ではない。

### C7 動いている daemon と Web の実体

```bash
ps -o pid,command -p $(cat <root>/.agent-link/engine/daemon.pid)
ps -o pid,command -p $(cat <root>/.sai/sai-engine/daemon.pid)
lsof -nP -iTCP:8400 -sTCP:LISTEN
lsof -nP -iTCP:8200 -sTCP:LISTEN
```

- **良い状態**: daemon の行の実行ファイルが、その root の App のビルド（`…/agent-link/target/release/agent-link`・`…/sai-tree/…`）。ポート 8400 は `agent-link-web`、8200 は `sai-web`（どちらも自分の App の物）。相手の App の `target/` から動いている行が無い。
- `daemon.pid` が無い、または PID が無い（`ps` が何も出さない）なら、動いていない。

### C8 古い App の残骸

```bash
ls -la ~/.local/bin | grep -E 'sai-moon|sai-paper-tui'
ls -la ~/.sai 2>&1
```

- **良い状態**: `~/.local/bin` に `sai-moon-*`・`sai-paper-tui` の link が無い。`~/.sai` に、sai-tree が使わない旧い物（`operator-notes.md`・`paper.env`・`refcode`・`sci`）が無い。

### C9 PATH と設定の重複

```bash
grep -n 'agent-link\|PATH' ~/.zshrc
readlink -f "$(command -v agent-link)"
readlink -f "$(command -v sai-link)"
```

- **良い状態**: `~/.zshrc` の PATH の追記が、各 App につき 1 行だけ（重複が無い）。`agent-link` は `~/workdir/Sai/agent-link/target/release/agent-link` を指す。`sai-link` は sai-tree の `target/release/sai-link` を指す（sai-moon を指さない）。`agent-link-web-restart` の関数が 1 つだけ。

### C10 プロジェクトの中に散らばった物

```bash
ls -la <root> | grep -E '\.pi|\.playwright-mcp|\.worktrees|\.mcp\.json\.bak'
```

- **良い状態**: 残っている物の帰属（どの App の物か）と、消してよいかが分かる。分からない物は消さずに §4 の 6 で確かめる。

---

## 3. 決めること（利用者が決める）

次の 5 つは、利用者が決める。決めたあとに §4 の片付けの手順の該当する所だけを行う。エージェントは決めない。

### D1 sai-tree の root をどちらの App が持つか（未決）

**未決である。** 次のどちらかを選ぶ。

| 選択 | 意味 | 片付けの違い |
|---|---|---|
| (a) agent-link が所有する | sai-tree の root には `.agent-link/` が入る。sai-tree 自身は、その root を init しない。agent-link は道具として sai-tree の開発に使う | `<sai-tree>/.sai/` を退避する（中身を見てから）。`agent-link init` を sai-tree で 1 回だけ行う（利用者の手で）。`~/.sai/registry.json` の sai-tree 行は sai-tree 側の物なので、そのままにするか外すかを D2 で決める。同じ root を両方が init しない |
| (b) sai-tree が所有する | sai-tree の root には `.sai/` が入る。agent-link は**自分のプロジェクトだけ**（別の root）で使う | `<sai-tree>/.agent-link/` が無いことを確かめる。あれば退避する。agent-link の登録簿（`~/.agent-link/registry.json`）から sai-tree を外す（§4 の 2） |

どちらを選んでも、R5 の受け入れは同じである（各 root に `.sai/` と `.agent-link/` の片方だけ）。

### D2 登録簿から相手を外す運用

| 選択 | 意味 |
|---|---|
| 手で外す | §4 の 2 を利用者が行う。相手の root が増えたら、その都度外す |
| 点検で見つけて外す | `doctor` の `ownership` の行と C1 の命令で見つけ、手で外す |
| App が自分の root だけを見る | 登録簿を読むとき、相手の root を無視する。**コードの変更が要る。今は行わない** |

### D3 同名の skill（`frontend-design`・`ui-ux-rules`）

| 選択 | 意味 |
|---|---|
| 共有の root を作らない | D1 で (b) を選べば、同じ root に両方が init することが無くなり、同名の取り合いも起きない（推奨） |
| 名前を前置して分ける | 片方の skill の名前を変える。**コードの変更が要る。今は行わない** |

### D4 古い App（sai-moon）の残骸

| 選択 | 意味 |
|---|---|
| 消す | `~/.local/bin/sai-moon-*` の link・`~/.sai` の旧い物・古い `.claude/settings.json` の行を消す（中身を見てから） |
| 残す（退避する） | 同じ中身を退避の場所（例 `~/agent-link-check/old/`）へ移し、元の場所から外す |

### D5 `~/.zshrc` の重複と PATH の持ち方

| 選択 | 意味 |
|---|---|
| 各 App 1 行に整理する（推奨） | `agent-link` の PATH と `agent-link-web-restart` を 1 つずつにする。sai-tree の PATH を載せるかは、別に決める |
| 今のまま | 重複を残す。起動する `agent-link` が、どの PATH の行が先に効くかで変わる |

---

## 4. 片付けの手順（利用者が自分で行う）

順番に行う。各ステップの前に、**消す前に中身を見る**。動いている物は**PID で止める**（`kill <pid>`）。`pkill -f` は使わない。

### 1 動いている物を確かめる（止めない）

§2 の C3・C7 の命令で、動いている daemon と Web の PID を控える。

- 止めるのは、利用者が**自分で起こした** PID だけ。8400 番の Web は、利用者が決めたとき以外は止めない。
- daemon は、その root の App の CLI で止めるのが先。PID で止めるのは、CLI で止まらないときだけにする。

```bash
cd <agent-link の root> && agent-link daemon stop    # agent-link の root の daemon
cd <sai-tree の root> && sai-link daemon stop        # sai-tree の root の daemon（sai-tree 側の CLI。命令の名前は sai-tree 側の文書で確かめる）
```

### 2 相手のプロジェクトを登録簿から外す（agent-link 側）

**先に複製を取る。** 複製は `~/agent-link-check/` のような、利用者が決めた場所へ置く。

```bash
mkdir -p ~/agent-link-check
cp ~/.agent-link/registry.json ~/agent-link-check/registry.before.json
```

外す相手の root（絶対パス）を指定して、新しいファイルを書く。元のファイルは、中身を見てから置き換える。

```bash
python3 -c 'import json,os,sys
p = os.path.expanduser("~/.agent-link/registry.json")
rows = json.load(open(p, encoding="utf-8"))
keep = [e for e in rows if e.get("root") != sys.argv[1]]
json.dump(keep, open(p + ".new", "w", encoding="utf-8"), ensure_ascii=False, indent=2)
print(len(rows), "->", len(keep))' "<相手の root の絶対パス>"

cat ~/.agent-link/registry.json.new    # 中身を見る
mv ~/.agent-link/registry.json.new ~/.agent-link/registry.json
```

- 外すのは `root` が相手のパスの行だけ。自分の root の行は残す。
- 登録簿は `agent-link` の CLI（`agent-link init`）が書く物なので、以後の `init` で戻ることがある。`init` は同じ `root` を 2 度足さない（既に登録があれば何もしない）。

### 3 `~/.zshrc` の重複を 1 行にする

```bash
cp ~/.zshrc ~/agent-link-check/zshrc.before
grep -n 'agent-link\|PATH' ~/.zshrc    # 重複の行番号を見る
```

- `export PATH="…agent-link…"`（`agent-link` の bin の行）と `agent-link-web-restart` の関数は、それぞれ 1 つだけ残す。
- 編集は手で行う（エディタで重複行を消す）。台本（`scripts/agent-link-setup install`）は、シェルの設定を書き換えない（足す 1 行を表示するだけ）。
- 変えたあとは、新しいターミナルを開いて `command -v agent-link` を確かめる。

### 4 `~/.local/bin` の古い symlink を外す

```bash
ls -la ~/.local/bin > ~/agent-link-check/local-bin.before
readlink ~/.local/bin/sai-moon-* ~/.local/bin/sai-paper-tui 2>/dev/null   # 行き先を見る
```

- 行き先が `sai-moon` のフォルダの物だけ、`rm` で外す。`rm` は link だけを消す（`rm -r` を使わない）。
- 行き先が agent-link や sai-tree の物は、外さない。
- 台本の `scripts/agent-link-setup uninstall` は、agent-link 自身のリンク（行き先が agent-link の `target/` の物）だけを消す。古い symlink は消さないので、この手で行う。

### 5 古い置き場の残骸（`~/.sai` の旧い物）

```bash
ls -la ~/.sai
```

- `operator-notes.md`・`paper.env`・`refcode`・`sci` の中身を見る。sai-tree が今も使う物（sai-tree の `web/`・`registry.json` など）は、消さない。
- 旧い物は、要るなら退避（`mv` で `~/agent-link-check/old/` へ）し、要らないなら消す。どちらにするかは D4 で決めた方に従う。

### 6 プロジェクトの中に散らばった物（`.pi/`・`.playwright-mcp/`・`.mcp.json.bak*`）

```bash
ls -la <root>/.pi <root>/.playwright-mcp <root>/.mcp.json.bak* 2>&1
```

- 中身の日付と内容を見て、どの App の物かを判断する。
- `.mcp.json.bak*` は、`init` が古い設定を退避した物。新しい `.mcp.json` と比べて、要らなければ消す。
- 判断できない物は消さない。

---

## 5. 戻し方

片付けの各ステップには、戻すための複製を取ってある（§4 の 2・3・4）。戻すときは、複製を元の場所へ戻す。

| 片付けたもの | 戻し方 |
|---|---|
| 登録簿 | `cp ~/agent-link-check/registry.before.json ~/.agent-link/registry.json` |
| `~/.zshrc` | `cp ~/agent-link-check/zshrc.before ~/.zshrc`（新しいターミナルで確かめる） |
| `~/.local/bin` の link | `ls -la` の記録（`~/agent-link-check/local-bin.before`）を見て、同じ行き先で `ln -s <行き先> <名前>` を作る |
| 退避した物（`~/.sai` の旧い物・プロジェクトの中の物） | 退避の場所（`~/agent-link-check/old/`）から元の場所へ `mv` で戻す |

戻したあとは、`scripts/agent-link-setup doctor` で点検する（読むだけ）。

消した物を複製なしで戻すことはできない。消す前に §4 の複製を取ること。

---

## 6. 拡張を入れたあとの点検

拡張（`agent-link ext`）を有効にしても、sai-tree の物（`<sai-tree の root>/.sai/`・`~/.sai`・8200 番）に変化が無いことを確かめる。拡張のデータは `<root>/.agent-link/ext/<name>/` に入るので、変化は agent-link の root の中だけに出るのが正しい。

**拡張を有効にする前**に、次の記録を取る。

```bash
mkdir -p ~/agent-link-check
ls -laR ~/.sai > ~/agent-link-check/sai-home.before 2>&1
ls -laR <sai-tree の root>/.sai > ~/agent-link-check/sai-root.before 2>&1
lsof -nP -iTCP:8200 -sTCP:LISTEN > ~/agent-link-check/port8200.before 2>&1
```

**拡張を有効にしたあと**（`agent-link ext enable <name>` を実行したあと）に、同じ記録を取り直して比べる。

```bash
ls -laR ~/.sai > ~/agent-link-check/sai-home.after 2>&1
ls -laR <sai-tree の root>/.sai > ~/agent-link-check/sai-root.after 2>&1
lsof -nP -iTCP:8200 -sTCP:LISTEN > ~/agent-link-check/port8200.after 2>&1
diff ~/agent-link-check/sai-home.before ~/agent-link-check/sai-home.after
diff ~/agent-link-check/sai-root.before ~/agent-link-check/sai-root.after
diff ~/agent-link-check/port8200.before ~/agent-link-check/port8200.after
```

- **良い状態**: 3 つの `diff` がどれも空。
- `diff` に出た物があれば、拡張の作者が sai の置き場に書いている（`ext-contract.md` の約束に反する）。その拡張は使わない。
- agent-link の root の `.agent-link/ext/<name>/` と `settings.json` の `extensions` が変わるのは正しい。

---

## 付録: 文書の位置

- 作業の約束（エージェント向け）: [`../AGENTS.md`](../AGENTS.md)
- 過去の決定（K1〜K4・P1〜P8 を含む）: [`decisions.md`](./decisions.md) の §6
- 拡張の作者向けの契約の正: [`../crates/agent-link-engine/assets/template/ext-contract.md`](../crates/agent-link-engine/assets/template/ext-contract.md)
- 遠隔の機械の手順: [`remote.md`](./remote.md)
