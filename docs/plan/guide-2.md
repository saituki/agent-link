# 実装ガイド 2/2: 名前を agent-link に変え、文書と切り替えの手順書を書く（U05〜U07）

## ゴール
ガイド 1 のあとの `~/workdir/Sai/agent-link` の外から見える名前を、全部 agent-link に変える。実行ファイルは `agent-link`・`agent-link-web`、環境変数は `AGENT_LINK_*`、フォルダは `.agent-link/`、置き場は `~/.agent-link` にする。文書と CI を合わせ、利用者が sai-moon から切り替えるための手順書 `docs/switch-over.md` を書く。

完了したときに利用者が確かめられること（要件定義書 `docs/plan/requirements.md` の 1〜8）:
- `cargo build --release` で `target/release/agent-link` ができ、テストが全部通る。
- 一時フォルダで `agent-link init` すると `.agent-link/` ができる。`agent-link-web` を合言葉付きで起こすと、sai-link の画面だけが出る。
- `git grep` で古い名前が、許した所にしか出ない。
- `docs/switch-over.md` に切り替えの手順がある。

## 前提
- ガイド 1（U01〜U04）が agent-link の main に入っている。kantoku 1 の完了報告書は agent-link のプロジェクトの run にある（`sai-link doc list` で探す）。
- このガイドと計画書は `docs/plan/` にある。worker には計画書のパスを渡す。
- worker の runner は `cc` にする。名前を変えたあとのコードは `AGENT_LINK_*` だけを読む。なので、sai-moon が入れる `SAI_*` は混ざらない。ただし U05 と U06 が終わってマージするまでは、半分だけ変わった状態がありうる。テストは各計画書の検証の命令で回す。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U05 | Rust（sai-link）の名前を agent-link に変える | `docs/plan/unit-U05-rename-rust.md` | `crates/**`・`Cargo.toml`・`Cargo.lock` | ガイド 1 | U06 と可 |
| U06 | Web（apps/web）の名前を agent-link に変える | `docs/plan/unit-U06-rename-web.md` | `apps/web/**` | ガイド 1 | U05 と可 |
| U07 | 文書・CI を agent-link に合わせ、切り替えの手順書を書く | `docs/plan/unit-U07-docs-switchover.md` | `AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`docs/**`（`docs/plan` を除く）・`.github/**`・`.gitignore` | U05・U06 | 不可 |

## 実行順序
```
U05 ─┐
     ├─▶（マージ・統合の確認 1）─▶ U07 ─▶（統合の確認 2）
U06 ─┘
```
- U05 と U06 は書くファイルが重ならないので、同時に動かす。名前の約束（共有契約 C2）だけで結ばれている。
- U07 は、U05・U06 をマージした main から始める。文書に書く事実（環境変数の一覧・テストの本数・命令）は、名前を変えたあとのコードから取るためである。
- 例外: U07 の担当でも、コードの注記の中の文書の名前（`for-sai-developers.md` → `for-developers.md`）だけは `crates/**`・`apps/web/**` を直してよい。U05・U06 のあとに動くので、ぶつからない。

## 所有権の表
| ファイル | U05 | U06 | U07 |
|---|---|---|---|
| `crates/**`・`Cargo.toml`・`Cargo.lock` | 書く | | 注記の文書名だけ |
| `apps/web/**` | | 書く | 注記の文書名だけ |
| `AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`CONCEPT.md`・`docs/**`（`docs/plan` を除く）・`.github/**`・`.gitignore` | | | 書く |
| `docs/plan/**`・`CLAUDE.md`・`waggle/**` | | | |

## 共有契約
- C2: 名前の対応表。U05・U06・U07 の計画書に全文を写してある。主な行:
  - `sai-link` → `agent-link`、`sai-web` → `agent-link-web`、`sai_web` → `agent_link_web`、crate `sai-link-*` → `agent-link-*`
  - `SAI_<名前>` → `AGENT_LINK_<名前>`（`SAI_LINK_BIN` だけ `AGENT_LINK_BIN`）。`SAI_` で始まる定数も同じ規則
  - `.sai/` → `.agent-link/`、`.sai/sai-engine/sai-engine.db` → `.agent-link/engine/agent-link.db`、`~/.sai` → `~/.agent-link`、`web/sai_web.db` → `web/agent_link_web.db`
  - template `sai-*.md` → `link-*.md`、`/sai-finalize` → `/agent-link-finalize`、sharer `sai-engine` → `agent-link`
  - 画面の名前 `agent-link`、版 `agent-link-web-v133`・`?v=133`
  - 中だけの名前（`sai_home()`・`saiApi`・`window.SaiRoutes`）は変えない。古い名前を読む互換の処理は作らない

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| `target/release/agent-link` ができ、`agent-link schema --json` の `bin` が `agent-link` | U05 | 統合の確認 2 |
| Rust・Web の JS・Web の Python のテストが `env -u` 無しで通る | U05・U06 | 統合の確認 1・2 |
| 一時フォルダの `agent-link init` で `.agent-link/` ができ、cc の設定が `agent-link` を指す | U05 | 統合の確認 2 |
| `agent-link-web` を合言葉付きで起こすと、sai-link の画面だけが iPhone 幅と PC 幅で使える | U06 | 統合の確認 2 |
| 古い名前の grep が、許した所だけ | U05・U06・U07 | 統合の確認 2 |
| `AGENTS.md` のテストの命令と本数が合う。`docs/switch-over.md` がある | U07 | 統合の確認 2 |
| sai-moon と sai-tree のリポジトリが変わっていない | 全員 | `git -C ~/workdir/Sai/sai-moon status --short` と `git -C ~/workdir/Sai/sai-tree log -1` が、始める前と同じ |

## 統合と検証
**統合の確認 1**（U05・U06 をマージしたあと）:
```bash
cd ~/workdir/Sai/agent-link
perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
(cd apps/web && uv sync)
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```

**統合の確認 2**（U07 をマージしたあと）:
1. `AGENTS.md` に書いたテストの命令を、上から順にそのまま実行する。
2. release を作り、一時フォルダで通しの確認をする（8297 番。必ず止める。本物のプロジェクトと `~/.agent-link` は使わない）:
   ```bash
   cargo build --release
   B="$PWD/target/release/agent-link"
   H=$(mktemp -d); P=$(mktemp -d)/check; mkdir -p "$P"; (cd "$P" && git init -q)
   (cd "$P" && AGENT_LINK_HOME="$H" "$B" init --name check)
   ls -a "$P" "$P/.agent-link/engine"; ls "$P/.claude/commands"
   AGENT_LINK_HOME="$H" "$B" schema --json | python3 -c "import json,sys;print(json.load(sys.stdin)['bin'])"
   AGENT_LINK_WEB_NO_WATCH=1 AGENT_LINK_HOME="$H" AGENT_LINK_WEB_TOKEN=check-token AGENT_LINK_BIN="$B" \
     perl -e 'alarm 300; exec @ARGV' apps/web/bin/agent-link-web serve --port 8297 &
   ```
   playwright で `http://127.0.0.1:8297/` を幅 390px と 1280px で開く。合言葉 `check-token` を入れ、Home・Projects（`check`）・Chats・Account の Usage が開くことを確かめる。console に error が無いことも確かめる。title が `agent-link` であることを見る。最後に `kill %1; rm -rf "$H" "$(dirname "$P")"`。
3. 古い名前の grep（U07 の変更内容 7 の命令）の結果が、C2 の「残してよい古い名前」だけであることを見る。
4. `docs/switch-over.md` を読み、命令の名前とパスが実物と合っているかを確かめる（実行はしない）。
5. 作業用コピー（worktree）とブランチを片付ける。

## 決定事項と理由 / 採用しなかった案
- 外から見える名前を全部変える（利用者の選択）。理由: 開発の道具とアプリ（sai-tree）の環境変数が混ざる事故を、名前で防ぐ。
- 中だけで使う名前は変えない。理由: 外から見えず、差分が増えるだけ。
- 古い記録（sai-tree の `.sai/`、run 1〜289）は移さない（利用者の選択）。DB の絶対パスの書き換えが重い。古い記録は sai-moon に残る。
- 切り替え（sai-moon を止める・PATH・`init`・Tailscale の ACL）は手順書だけにし、agent は実行しない。利用者の環境を変える操作だからである。
- Linux のテスト利用者は tailnet に招き、ACL で絞る（利用者の選択。node sharing は採らなかった）。

## リスクと対処
- **grep で見逃した古い名前**: 統合の確認 2 の 3 で見つける。U05・U06 の範囲なら、kantoku が自分で直してよい（名前の置き換えだけの場合）。
- **`agent-link init` で cc のフックが壊れる**（`apps/web/hooks` が見つからない）: `cc_command.rs` の `web_hooks_dir()` は `CARGO_MANIFEST_DIR` からの相対パスで探す。crate のフォルダが変わってもパスの形は同じ（`crates/agent-link-engine/../../apps/web/hooks`）。統合の確認 2 で `.claude/settings.json` のフックのパスが実在するかを見る。
- **iPhone に sai-moon の画面が残る**: `swIsOldCache` が `sai-web-v*` も消す（U06）。
- **kantoku が自分で決めてよい範囲**: テストの直し方、行番号のずれ、名前の置き換えの漏れ。
- **利用者に聞く範囲**: C2 の対応表に無い名前を決める必要が出たとき。sai-moon・sai-tree に手を入れないと進めないと分かったとき。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`
- ガイド 1: `docs/plan/guide-1.md`
