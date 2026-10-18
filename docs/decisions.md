# agent-link の過去の決定

対象: **agent-link を直す人**。目的: 過去の決定を、日付・理由・状態つきで 1 か所に集める。現在の設計は [`design.md`](./design.md)。

- 1 決定 = 1 行。古くなった決定は消さず、状態を「やめた（日付・置き換えた決定）」に変え、新しい決定を新しい行に足す。
- 日付は、出典の計画書の最後のコミットの日（`git log --format=%ad --date=short -- <ファイル>`）か、waggle の文書を書いた run の開始日（`sai-link run show <run_id>`）。
- `docs/plan/` の終わった計画書は書き換えない（`AGENTS.md`）。古くなった記述はこの表の「やめた」で示す。

出典の略称:

| 略称 | 中身 |
|---|---|
| 取り出し | `docs/plan/requirements.md`・`guide-1.md`・`guide-2.md`・`unit-U01`〜`unit-U07`（sai-tree から agent-link を取り出して改名した作業。2026-10-15） |
| 簡素化 | 要件定義書 `waggle:KAoVMwoR`（run 11。2026-10-15） |
| 遠隔の子 | 要件定義書 `waggle:VEGRQwuS`（run 65。2026-10-17）・手順書 `docs/remote.md` |
| 見直し | 要件定義書 `waggle:t3Lc36eG`（run 76。2026-10-17）・利用者の下書き `docs/plan/requirements-design-review.md` |

文書は `sai-link doc read <トークン>` で読める。

---

## 1. 取り出しと改名（sai-tree → agent-link）

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-15 | 元のコードは sai-tree（sai-moon ではない） | sai-link が新しく、Mac 専用の機能もすでに消してある | 有効 | 取り出し `requirements.md` |
| 2026-10-15 | sai-tree を複製して別のリポジトリにする（固定版を使う案は採らない） | 使わない機能を消したい。直しを 2 つのリポジトリに入れる手間は了解済み | 有効 | 取り出し `requirements.md` |
| 2026-10-15 | Mac 専用の機能（launchd・承認・遠隔ブラウザ・5 時間枠・pi）は戻さない。daemon と Web は手で起こす | 利用者の選択。「遠隔ブラウザ」は画面の遠隔操作で、遠隔の子（§3）とは別 | 有効 | 取り出し `requirements.md` |
| 2026-10-15 | 論文・研究・Radio・MLflow・sai-ts・sai-prep・Customer サポート・多人数モードを消す | 利用者の選択。開発の道具では使わない | 有効 | 取り出し `requirements.md` |
| 2026-10-15 | 実行ファイル・環境変数・フォルダ・置き場・DB の名前を全部変える（`sai-link`→`agent-link`・`SAI_*`→`AGENT_LINK_*`・`.sai/`→`.agent-link/`・`~/.sai`→`~/.agent-link`） | sai-tree（開発するアプリ）と道具が同じ名前だと、道具の環境変数がアプリのテストに混ざる事故が起きた | 有効 | 取り出し `requirements.md` |
| 2026-10-15 | sai-tree の古い記録（run 1〜289）は移さない。空の `.agent-link/` から始める | DB に絶対パスが多く、移すと重い。古い記録を読む互換の処理も作らない | 有効 | 取り出し `requirements.md` |
| 2026-10-16 | sai-moon と並べて使う（sai-moon は止めない）。Web は agent-link が 8400 番・sai-moon が 8200 番 | 利用者の選択。名前と番号を重ねない | 有効 | 取り出し `docs/switch-over.md` |
| 2026-10-15 | P-8: 内部だけで使う名前（`saiApi`・`window.SaiRoutes`・`sai_home()`・`SaiStore` など）は変えない | 外から見えない。変えると差分が増えるだけ | 有効（2026-10-17 に利用者が「変えない」と決め直した。§5） | 取り出し `requirements.md`・`guide-2.md` |
| 2026-10-15 | Linux のテスト利用者は tailnet に招き、ACL で Linux だけを許す | 利用者の選択。手順は `docs/switch-over.md` | 有効 | 取り出し `requirements.md` |

## 2. 取り出しの時点の決定で、やめた物

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-15 | P-1: Web を合言葉 `AGENT_LINK_WEB_TOKEN` 付きで起こす。Usage が使える | 公開された Web を守るため | **やめた**（2026-10-15・簡素化の「合言葉を消す」）。今は合言葉が無い。Usage は Home のカード | 取り出し `requirements.md:54` |
| 2026-10-15 | P-2: `.claude/settings.json` のフックと許可を `agent-link` に向ける | cc のフックの置き場 | **やめた**（簡素化。sai-moon と取り合わないため）。今はフックを `.agent-link/cc-settings.json` に書き、run の cc に `--settings` で渡す。`.claude/settings.json` は書かない | 取り出し `requirements.md:53` |
| 2026-10-15 | P-3: 8200 番で合言葉付きの Web を起こす | sai-moon の置き換えを前提にした手順 | **やめた**（並べて使う決定。§1）。今は 8400 番・合言葉なし・Tailscale 4446。8200 番は sai-moon | 取り出し `requirements.md:56` |
| 2026-10-15 | P-4: 残す Web の機能に「Usage・コマンド画面」を入れる | sai-tree の機能をそのまま残す | **やめた**（2026-10-15・簡素化）。どちらも消えた | 取り出し `requirements.md:12` |
| 2026-10-15 | P-5: 下のタブは 4 つ（Home・Projects・Chats・Account） | Account に Usage の詳細を置く | **やめた**（簡素化の R1）。今は 3 つ（Home・Projects・Chats） | 取り出し `guide-1.md:8,80`・`unit-U03-strip-web-screens.md:69` |
| 2026-10-15 | P-6: 合言葉を `.env` に書く・8200 番・Tailscale `:4444` | P-1・P-3 と同じ | **やめた**（P-1・P-3 と同じ。Tailscale は 4446。4444 は sai-moon） | 取り出し `guide-2.md:8,57,82,85`・`unit-U07-docs-switchover.md:74-78` |
| 2026-10-15 | P-7: run の流れに `outputs/…/done`・`web/token`・OpenRouter を含める | sai-tree の機能の写し | **やめた**（簡素化）。`done/` は無い（Workflow に移した run は日付フォルダ）。`web/token`・OpenRouter も無い | 取り出し `unit-U05-rename-rust.md:43,90`・`unit-U06-rename-web.md:21,77` |

## 3. 簡素化（cc だけ・1 人用・英語）

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-15 | 実行者は Claude Code（cc）だけ。`runner` の選択（`runner: user`・`card start`/`card finish`・gl・pi）・`provider`・`tag` をやめる。古い項目は読み飛ばし、書き出さない | Mac で 1 人が cc だけを使う | 有効 | 簡素化 R8 |
| 2026-10-15 | Account タブと Usage の詳細をやめ、Claude の使用量は Home のカードにする。OpenRouter・DeepSeek・pi の使用量と環境変数を消す | 他の LLM を使わない | 有効 | 簡素化 R1 |
| 2026-10-15 | 合言葉（`AGENT_LINK_WEB_TOKEN`・`web/token`）をやめる。入口の守りは 127.0.0.1 の待ち受けと `require_internal` | 1 人用。Tailscale serve は tailnet が自分だけのときだけ使う（README の注意） | 有効 | 簡素化 R2 |
| 2026-10-15 | 「コマンド」画面と API（`/api/projects/{name}/commands[/run]`）をやめる。`agent-link schema --json` は残す | 使わない。`schema --json` は CLI の約束 | 有効 | 簡素化 R3 |
| 2026-10-15 | `done`（状態）をやめる。流れは todo → running → review →（`run advance`）→ Workflow。Kanban は 3 レーン | 利用者の確認のあとに置く状態は要らない | 有効 | 簡素化 R4 |
| 2026-10-15 | `round`・`global_round` をやめ、Workflow に移した run は日付フォルダ `outputs/<yymmdd>/<run_id>/` に置く。日付は一番上の親 run の実行開始日（ローカル時刻） | 子が日付を跨いでも親のフォルダに入る。DB に新しい列が要らない（`execution_history.started_at` から決まる） | 有効 | 簡素化 R5 |
| 2026-10-15 | 日付フォルダの中は平ら（`<yymmdd>/<run_id>/`）。親子の入れ子は表示だけで表す | 文書の参照の形 `waggle:<tok>:path:.agent-link/outputs/*/<run_id>/<file>` を壊さないため | 有効 | 簡素化 |
| 2026-10-15 | Workflow の子の表示・非表示は Rust で配置を計算して分ける（`agent-link workflow --subtasks`）。子の並びは「`task create` 1 回 = 1 段」 | 画面だけで隠すと子の分の隙間が残る。並びは利用者の選択 | 有効 | 簡素化 R7 |
| 2026-10-15 | 画面の文言と、CLI の標準エラー・`--json` の人向けの文を英語にする（文書とエージェント向けの文は日本語のまま） | CLI の理由の行がそのままトーストに出るため、Web だけでは画面が英語にならない | 有効 | 簡素化 R9 |
| 2026-10-15 | iPhone から Tailscale serve で使う道は残す（自分の tailnet だけ） | iPhone の画面と通知を保つ | 有効 | 簡素化 R2 |
| 2026-10-15 | `executer`（`human` / `agent`）は残す | 簡素化の範囲外 | 有効 | 簡素化 R8 |

## 4. 遠隔の子（run 65）

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-17 | agent-link は Mac だけに置く。遠隔の機械には claude だけを置き、子タスクの claude を ssh で動かす | 遠隔にも agent-link を入れる案は、機械ごとの Rust のビルド・Mac の版とのずれ・DB と waggle トークンが 2 つになる同期が要り、大きい。スクリプトで `claude -p` を送るだけの案は、Web に出ず、途中の様子・質問・中断が使えない | 有効 | 遠隔の子 |
| 2026-10-17 | 遠隔から Mac への ssh は、強制コマンドの鍵（`authorized_keys` の `command="<agent-link> remote-gate",restrict`）で許す。`agent-link remote setup` が 1 行足す | フックが Mac の `state.json` に届かないと、最初の文が `PromptUnconfirmed` で失敗する。通す命令は門番が決める | 有効 | 遠隔の子 |
| 2026-10-17 | 最初に対応する遠隔は `wsl-server` だけ | dgx は claude を入れれば同じ仕組みで動くが、今回は確かめない | 有効 | 遠隔の子 |
| 2026-10-17 | 遠隔で直したコードは git のブランチで戻す（親が push し、子がコミットし、親が fetch してマージする） | patch ファイルを文書で返す案より、履歴が残る | 有効 | 遠隔の子 |
| 2026-10-17 | 遠隔の子の会話記録（`~/.claude/projects/*/<session_id>.jsonl`）を Mac に写す | `message.md`・セッションの統計・検索が Mac の記録を読む | 有効 | 遠隔の子 |
| 2026-10-17 | 遠隔の子では作業用コピー（`worktree: isolation`）を作らない。`cwd` の存在は起動の前に ssh で確かめる | 作業用コピーと `cwd` の確認は Mac のファイルで行うため | 有効 | 遠隔の子 |
| 2026-10-17 | 実機（`wsl-server`）での確認は利用者が行う。agent や worker は `agent-link remote setup` を実行しない | Mac の `~/.ssh/authorized_keys` と遠隔の機械を書き換えるため | 有効 | 遠隔の子 |

## 5. 設計の見直し（run 76）

2026-10-17 の決定。要件定義書の「決定事項と理由」の全行と、見直しの中で足した決定。

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-17 | 機能は 1 つも変えない | 形の整理と振る舞いの変化を混ぜると、原因を切り分けられない | 有効 | 見直し |
| 2026-10-17 | 見直しの基準は `feat/remote-host`（遠隔の子）を含む状態。遠隔の子は機能として残す | 利用者の決定 | 有効 | 見直し |
| 2026-10-17 | 不変条件 I1 は「状態・DB・文書・Web の主人はその Mac。子の claude だけは ssh 先の機械で動いてよい」 | 下書きの I1「遠隔の話にしない」は遠隔の子と矛盾する | 有効（`design.md` §1） | 見直し |
| 2026-10-17 | P-8（決め直し）: 内部の名前 `sai`（`SaiStore`・`sai_home`・JS の `sai*`・`window.SaiRoutes`）は変えない。`AGENTS.md` の `sai` の接頭辞の規則も残す | 外から見えず、差分だけが大きい（取り出しの決定を、利用者が「変えない」と決め直した） | 有効 | 見直し |
| 2026-10-17 | Web と daemon は手で起こす（`launchd` を持たない） | 利用者の決定（§1 の決定を決め直した） | 有効 | 見直し |
| 2026-10-17 | `docs/contract.md` は作らない。契約の正は `docs/for-developers.md`（CLI と Web の間）と `docs/integration.md`（HTTP の住所と画面の住所） | 契約の中身は既にこの 2 枚にある。ほかの文書の重複はリンクに替える | 有効 | 見直し |
| 2026-10-17 | 入れ方・点検・消し方はシェルの台本 1 本（`scripts/agent-link-setup install`・`doctor`・`uninstall`）にする。CLI の命令は足さない | CLI の命令の一覧を前後で同じに保てる。点検は Web の `.venv` とポートを見るが、コアは Web を知らない（`design.md` B3） | 有効 | 見直し |
| 2026-10-17 | 設定ファイルは既存の `$AGENT_LINK_HOME/settings.json` に `config` を足す。キーは環境変数の名前そのまま。環境変数が空でなければ環境変数が勝つ | 新しいファイルを増やさない。名前の対応表が要らない。`settings.json` は Rust が既に読み、知らないキーを無視する | 有効 | 見直し |
| 2026-10-17 | データに残る名前（DB の列 `herdr_pane_id`・書き出し JSON と run フォルダの `herdr_pane_id`・環境変数 `HERDR_PANE_ID`）は変えない | 変えると DB の移行と、遠隔の機械の `remote setup` のやり直しが要る。データの移行は範囲外 | 有効 | 見直し |
| 2026-10-17 | `docs/plan/` の終わった計画書は書き換えない。決定は `docs/decisions.md` に集め、古くなった物は「やめた」と書く | `AGENTS.md` の約束（終わった計画は読むだけ） | 有効 | 見直し |
| 2026-10-17 | 確認は一時フォルダで行う。本物のプロジェクトで `init` しない。8400 番の Web を止めない | 利用者の環境を変えない | 有効 | 見直し |
| 2026-10-17 | クレート 3 分割（`engine`・`view`・`cli`）を残す。`agent-link-view` は「CLI が使う表示用の組み立て」として名前が合うので、説明だけ直す | `engine → view` をコンパイラが禁じる。1 クレートに畳むと、この規則が約束事に格下げになる | 有効 | 見直し（`design.md` §2.2） |
| 2026-10-17 | `phone` と `herdr` の名前を実態に直した（`phone_export.rs` → `run_export.rs`・`registry.rs`・`transcript.rs`・`cc_cost.rs`、`herdr.rs` → `pane.rs`、`phone.rs` → `views.rs`、`phone_def.rs` → `def_files.rs`、`phone_view.rs` → `run_docs.rs`、Web の `control.phone_*` → `control.cli_*`） | 消えた製品・多重化の名前が現役のコードに残り、説明を信じると間違える | 有効 | 見直し（U01〜U07） |
| 2026-10-17 | `sai-progress`（標準エラーの進み具合の行）を契約から消した | 出す側は 2026-10-15 に消えている。公開する前に片付ける | 有効 | 見直し |
| 2026-10-17 | 利用者の設定を `settings.json` の `config` にも書けるようにした。書ける名前は `README.md` の環境変数の表にある。`AGENT_LINK_HOME` は書けない | 配布の段で設定ファイルにする。`AGENT_LINK_HOME` は `settings.json` の場所を決める名前なので書けない | 有効 | 見直し（U08） |
| 2026-10-17 | 次を持たない: TUI・MCP の道具・合言葉・Docker 化・多人数・crates.io 公開・Linux 向けの配布（agent-link 本体を Linux に置く） | 不変条件 I1〜I6 と、実装を 2 つにしないため（`design.md` §4） | 有効 | 見直し |

## 6. 拡張の仕組み（run 108・2026-10-18）

2026-10-18 の決定。要件定義書（`docs/plan/requirements-extension.md`）の K1〜K4（利用者の決定）と、§2.4 の P1〜P8（実装の前に決めた物）を、日付・理由・状態つきで置く。拡張の作者向けの契約の全文は [`crates/agent-link-engine/assets/template/ext-contract.md`](../crates/agent-link-engine/assets/template/ext-contract.md) にある（ここには要点だけ）。

| 日付 | 決定 | 理由 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-18 | K1: 拡張の UI はサイドバーのタブ（拡張ごとに 1 つ）。開くとプロジェクトを選び、その下に拡張の画面（`#/ext/<name>/<project>`） | 利用者の決定 | 採用 | 拡張 K1 |
| 2026-10-18 | K2: `ext install` は既定で複製し、`--link` で symlink にする | 利用者の決定。拡張は別のリポジトリで開発するため | 採用 | 拡張 K2 |
| 2026-10-18 | K3: extensions リポジトリ（`~/workdir/Sai/agent-link-extensions`）の作成は最後の kantoku が行う。カードは作るだけ | 利用者の決定 | 採用 | 拡張 K3 |
| 2026-10-18 | K4: 第 1 部（sai-tree との分離）は、コードの部分（改名・doctor）と手順書（`docs/separation.md`）を今回に含める。環境の片付けは利用者が行う | 利用者の決定。エージェントは利用者の環境（`~/.zshrc`・`~/.local/bin`・`~/.agent-link`・`~/.sai`・登録簿）を変えない | 採用 | 拡張 K4 |
| 2026-10-18 | P1: 「Tool」は拡張の CLI のこと。MCP・cc の hook・skill は配らない（P1 の「hook」は cc の hook を指す。Q1 で文言を明確化） | I4（エージェントの道具は CLI 1 本）に反するため | 採用（Q1 で文言を明確化） | 拡張 P1 |
| 2026-10-18 | P2: 拡張の block と template は、enable のときに `install_one` の規則で写す | block の探し方が 3 通りあり、そこに手を入れないため | 採用 | 拡張 P2 |
| 2026-10-18 | P3: 有効の記録はプロジェクトの `.agent-link/settings.json` の `extensions`。`$AGENT_LINK_HOME/settings.json` の `extensions` は読まない | 記録を 1 か所にするため | 採用 | 拡張 P3 |
| 2026-10-18 | P4: service は拡張の種類ではなく能力。`service.url_env` の値（環境変数 → `config`）が `http://` か `https://` で始まれば ready | 有効・無効の規則を 1 つにするため | 採用 | 拡張 P4 |
| 2026-10-18 | P5: Web から拡張の CLI を呼ぶのは `agent-link ext run` を通してだけ。呼べるのは `cli.web` のサブ命令だけ。外から来る値は CLI の前で検査する | Web は agent-link の CLI だけを呼ぶ（B4） | 採用 | 拡張 P5 |
| 2026-10-18 | P6: 拡張の JS は同じオリジンで動き、サンドボックスは無い | 個人ローカル専用（I5・I6） | 採用 | 拡張 P6 |
| 2026-10-18 | P7: 改名は `sai_link_exe` → `agent_link_exe`（Rust）と `sai_link_bin` → `agent_link_bin`（Python）だけ。画面の頭 `sai*` と `sai_home` は残す | `sai*` は画面の決まり（P-8・2026-10-17）で、他の App の名前ではないため | 採用 | 拡張 P7 |
| 2026-10-18 | P8: 下書きの C6「`sai-engine` のコメントが残る」は、HEAD（`crates/`・`apps/`・`scripts/`）では 0 件。事実として訂正する | 下書きの実測と現在のコードが食い違った。コードを正とする | 採用 | 拡張 P8 |

## 7. 拡張の基盤の拡張（2026-10-18）

2026-10-18 の決定。ある拡張を例に base の不足を洗い出し、利用者と決めた（出典の「拡張の基盤 Q」。要件定義書 `requirements.md` の D1〜D13 から起こした）。拡張の作者向けの契約の全文は [`ext-contract.md`](../crates/agent-link-engine/assets/template/ext-contract.md)。

| 日付 | 決定 | 理由（D は要件定義書の番号）・採用しなかった案 | 状態 | 出典 |
|---|---|---|---|---|
| 2026-10-18 | Q1: 拡張の hook。点は `context`・`write_check`・`question` の 3 つ。呼ぶのは、その拡張の block の run だけ（block の id が `<name>` か `<name>-` で始まる拡張。複数なら name の長い方）。命令は `cli.bin` に渡す引数の配列。P1 の「hook」は cc の hook を指す（P1 の文は「MCP・cc の hook・skill は配らない」と読み替える） | 理由（D1・D8〜D10）: 起動直前に作る物は `context` の出力に入れられるので `pre_run` は作らない。base が呼ぶだけで、エージェントの道具は増えない（I4 を保つ）。持ち主が 1 つに決まり、順序の取り決めが要らない。採用しなかった案: base を変えず settings に手で書く・hook をやめる | 採用 | 拡張の基盤 Q1 |
| 2026-10-18 | Q2: 待ち条件付きカード（`card wait`・task.md の `wait`・`wait_every`・`wait_note`）。daemon が予定の時刻ごとに命令を呼び、0 で実行・1 でまだ・それ以外は理由を残して待つ。命令の先頭の `agent-link` は base の実行ファイルに置き換える | 理由（D2・D11）: 拡張の名前を知らずに作れる。daemon の PATH に拡張の bin は無いので、`agent-link ext run <name> -- …` で拡張の CLI を呼ばせる。採用しなかった案: `card schedule` で見に行くカードを予約し直す（待つ間に run が増える）・待ちをやめる | 採用 | 拡張の基盤 Q2 |
| 2026-10-18 | Q3: Web の拡張の CLI の呼び出しに `input` を足し（`ctx.cli(args, {input})`）、CLI に `ext run --stdin` を足す。1,000,000 文字（バイト）まで | 理由（D3）: 編集の画面を持つ拡張すべてに要る。引数は 1 本 1,000 文字までで改行を含められない。採用しなかった案: なし（要件定義書に挙げた物は無い） | 採用 | 拡張の基盤 Q3 |
| 2026-10-18 | Q4: 拡張の設定値。`extension.json` の `config`（名前の宣言）と `agent-link ext config`。値は環境変数 → `settings.json` の `config` → 無し。`--json` 無しは値を出さない。値は CLI で読ませ、エージェントには読ませない | 理由（D4・D12）: API の鍵を使う拡張すべてに要る。読む規則を base の 1 か所（`config_var`）に保ち、秘密を pane の起動の文字や記録に出さない。採用しなかった案: なし（要件定義書に挙げた物は無い） | 採用 | 拡張の基盤 Q4 |
| 2026-10-18 | Q5: 遠隔の子から拡張を名前で呼べる。遠隔の PATH の先頭に中継（`ext-bin/<name>`）を置き、Mac の `ext run <name> [--stdin] -- …` を門番経由で呼ぶ。門番は `ext run` だけを通す。拡張の CLI は Mac で動き、上限は 120 秒 | 理由（D5・D13）: 遠隔で動く拡張の block すべてに要る。拡張は Mac で動く（I1 を保つ）。遠隔の子からは、他の命令と同じ門番を通る。採用しなかった案: なし（要件定義書に挙げた物は無い。拡張の CLI を遠隔へ配ると状態の主人が 2 つになるので、考えない） | 採用 | 拡張の基盤 Q5 |
| 2026-10-18 | Q6: 拡張の利用者全体のデータの置き場は足さない。共有したい物の場所は、設定値（Q4）で利用者が指す | 理由（D6）: I2（実行の状態はプロジェクトごとに閉じる）を変えない。利用者の決定。採用しなかった案: `$AGENT_LINK_HOME/ext/<name>/` を契約に足す | 採用 | 拡張の基盤 Q6 |
| 2026-10-18 | Q7: 今も動くが契約に無かった物を契約（`ext-contract.md`）に書いた。拡張の CLI が呼んでよい base の命令・requires 先の呼び方（`ext run`）・block の frontmatter の拡張の項目（`<name>_…`）・`lib/` の置き場・他の画面へのリンク。block の `timeout` の誤りの文も直した（上限は無く、断るのは 0 だけ） | 理由（D7）: 拡張の作者が契約だけで作れるようにする。呼んでよい base の命令は、読む命令と `card create`・`card schedule`・`card wait` に絞る（run の実行・完了は人とエージェントの仕事）。採用しなかった案: 拡張の CLI に base の命令の全部を許す（状態を大きく変える命令まで拡張の判断になる） | 採用 | 拡張の基盤 Q7 |
