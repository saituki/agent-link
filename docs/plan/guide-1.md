# 実装ガイド 1/5: エージェントを herdr 無しで動かす（headless）

## ゴール
sai-tree（`~/workdir/Sai/sai-tree`。sai-moon の複製）の `sai-link daemon` が、cc・pi を herdr の pane ではなく子プロセス（`claude -p` の stream-json・`pi --mode json`）で動かす。子タスクも pane を作らず裏で動く。カードの実行・子タスク（create / execute / wait / send / interrupt / capture / finalize）・`sai-link run send` / `run interrupt`・質問（cc の AskUserQuestion・pi の ask_user）が headless で動く。herdr の実行ファイルは 1 度も呼ばれない。cc の statusLine と承認のフックは消える。

## 実装計画書の一覧
計画書はリポジトリの `docs/plan/` にある（sai-moon のプロジェクトにも同じ物が waggle で保存してある）。

| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U01 | headless の輸送路（擬似 pane） | `docs/plan/unit-U01-headless-transport.md`（waggle:9XbcNnBU） | engine の `headless.rs`（新規）・`herdr.rs`・`orchestrator.rs`・`task_manager.rs`・`cc_command.rs`（`build`）・`pi_command.rs`・`runner.rs`・`phone_export.rs`（cc-cost の切り出し）・`lib.rs` | - | - |
| U02 | 送信・中断の CLI、headless の質問、statusLine と承認のフックの削除 | `docs/plan/unit-U02-send-cli-and-question-hooks.md`（waggle:iZWydjW2） | engine の `agent_api.rs`・`cc_command.rs`（フック・設定）・`phone_export.rs`（statusline）・CLI の `cli.rs`・`cmd_run.rs`・`main.rs`・`apps/web/hooks/*.py`・`ask_user.ts`・`docs/for-sai-developers.md` | U01 | 不可 |

## 実行順序
U01 → U02（直列）。U02 は U01 の擬似 pane と環境変数（`HERDR_PANE_ID`・`SAI_AGENT_TRANSPORT`）を使い、`cc_command.rs` と `phone_export.rs` を両方が書くため。

## 所有権の表
| ファイル | U01 | U02 |
|---|---|---|
| `crates/sai-link-engine/src/headless.rs`・`herdr.rs`・`orchestrator.rs`・`task_manager.rs`・`pi_command.rs`・`runner.rs`・`lib.rs` | 書く | 読む |
| `crates/sai-link-engine/src/cc_command.rs` | 書く（`build` と権限） | 書く（`cc_hooks_json`・`cc_settings_json`）※直列 |
| `crates/sai-link-engine/src/phone_export.rs` | 書く（cc-cost の切り出し） | 書く（statusline の削除・`pane_id_from_session_dir` の公開）※直列 |
| `crates/sai-link-engine/src/agent_api.rs`・`crates/sai-link-cli/src/cli.rs`・`cmd_run.rs`・`main.rs` | 読む（`card options` の runner の一覧だけ U01） | 書く |
| `apps/web/hooks/*.py`・`apps/web/src/sai_web/extensions/ask_user.ts`・`docs/for-sai-developers.md` | - | 書く |

## 共有契約
- C1 headless の書き出し（U01 が書き、U02・ガイド 2 の U03 が読む）: 擬似 pane の id `hl-<12 桁 16 進>`、置き場 `<root>/.sai/sai-engine/headless/<id>/`、`state.json` の形、ターンのプロセスの環境変数。全文は U01・U02 の計画書。
- C2 送信・中断の CLI（U02 が作り、ガイド 2 の U03 が呼ぶ）。
- C3 フック・拡張と Web の約束（U02 が作り、ガイド 2 の U03・ガイド 4 の U10 が Web 側を作る）。

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| テストがすべて通る | U01・U02 | `cargo test --workspace` と Web のテスト |
| headless で子タスクが完了まで進み、herdr が呼ばれない | U01 | U01 の AC4（一時フォルダの daemon・偽のエージェント・偽の herdr） |
| `run send` / `run interrupt` が動く | U02 | U02 の AC1 |
| headless の質問に Web で答えられる | U02 | U02 の AC3（偽の Web）・AC5（実物） |
| 実物の cc・pi が headless で動く | U01・U02 | 作業報告書の実物の確認の記録 |

## 統合と検証
- マージごとに `env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME perl -e 'alarm 1800; exec @ARGV' cargo test --workspace` と Web の JS・Python のテスト。
- 最後に kantoku 自身が、一時フォルダのプロジェクト（`SAI_HOME` も一時フォルダ）で daemon を起こし、費用の小さい model の cc で「`sai-link doc write` で 1 行の文書を保存せよ」というカードを `card run` し、完了まで進むことを確かめる。daemon は必ず止め、一時フォルダを消す。
- `cargo build --release` を本体フォルダで行い、`target/release/sai-link`・`sai-paper`・`sai-sci` を作り直す。

## 決定事項と理由 / 採用しなかった案
- sai-tree は sai-moon とは別のプロジェクト。sai-moon は直さない。herdr は使わない（利用者の決定）。
- herdr の操作は `HerdrClient` の中で擬似 pane に向ける。pane を作る 2 か所だけ明示の分岐にする。/ runner の trait で全部を書き換える案は差分が大きいので採らない。`HerdrClient` の名前の付け替えも後回し。
- ターンごとにプロセスを起こし、2 回目からは `--resume`（cc）・`--continue`（pi）。/ stdin で多ターンを流す方式（`--input-format stream-json`）は文書が薄く、中断の制御が非公開なので採らない。Agent SDK は規約上サブスクのログインを使えないので採らない。
- cc は `--permission-mode acceptEdits --allowedTools <一覧>` で起動する。コンテナは root（rootless Docker）で動き、Claude Code は root で `--dangerously-skip-permissions` を拒否するため（利用者の決定）。/ 公式文書に無い `IS_SANDBOX=1` は採らない。
- cc の質問は、フックが Web の答えを待ち、拒否の理由の文で答えを返す。
- 承認（PermissionRequest）と statusLine は headless では来ないので消す。費用は `result` 行から書く。

## リスクと対処
- `claude -p`・`pi --mode json` が stdin の依頼を読まない: U01 の要確認のとおり、位置引数かファイルに切り替えてよい（kantoku が決めてよい）。
- `--allowedTools` で一部の道具が拒まれる（`mcp__*` の形が通らないなど）: 一覧を直してよい（kantoku が決めてよい）。どうしても全部許せないなら、利用者に聞く。
- `claude -p` で `AskUserQuestion` が使えない: 文書に書いて先へ進む（kantoku が決めてよい）。headless の質問をあきらめる・別の方式（MCP の道具を足すなど）にするかは、利用者に聞く。
- herdr の CLI を前提にしたテストが落ちる: 擬似 pane に直すか消す。消したテストは作業報告書に名前を書く。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（waggle:y7EoLAgV）
- 調査 X1（起動経路と herdr 依存）: `docs/plan/research/X1-agent-lifecycle.md`
- 調査 R3（rootless Docker・Claude Code の root の制限）: `docs/plan/research/R3-rootless-docker.md`
- 調査 E3（Mac 専用の機能と Web の herdr）: `docs/plan/research/E3-mac-only-removal.md`
