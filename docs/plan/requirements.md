# 要件定義書: agent-link（sai-tree から sai-link と Web だけを取り出した開発用の道具）

## 何を

`~/workdir/Sai/sai-tree` を複製して、新しいリポジトリ **agent-link**（`~/workdir/Sai/agent-link`）を作る。agent-link はアプリ開発専用の道具で、Mac で 1 人が使う（単独モード）。sai-tree を開発するときに使う。

agent-link が持つ物と持たない物:

| 残す | 消す |
|---|---|
| sai-link（`crates/sai-link-engine`・`sai-link-cli`・`sai-link-view`）: run・カード・文書（waggle）・子タスク・daemon・Workflow・History・cc の起動 | sai-paper（`crates/sai-paper-*`）・sai-sci（`crates/sai-sci-*`・`sai-sci/`）・research-tree |
| Web（`apps/web`）の単独モード: Home・Projects・Chats・Workflow・カード・文書・質問と通知・Usage・コマンド画面・session search・音声入力（`voiceask.js`） | Web の Paper・Research・Radio・MLflow・sai-ts・sai-prep・Customer サポート（feedback・`user-*` block）・多人数モード（ログイン・利用者・アカウント・コンテナ・`claude_login.py`・`apps/web/server/`） |
| | `apps/radio`・`apps/ts`・`apps/prep`・`apps/mlflow-server` |

さらに、外から見える名前をすべて agent-link に変える。

| 種類 | 今 | agent-link |
|---|---|---|
| 実行ファイル | `sai-link`・`sai-web` | `agent-link`・`agent-link-web` |
| 環境変数 | `SAI_*` | `AGENT_LINK_*` |
| プロジェクトのフォルダ | `.sai/`（DB は `.sai/sai-engine/sai-engine.db`） | `.agent-link/`（DB は `.agent-link/engine/agent-link.db`） |
| 置き場 | `~/.sai`（`SAI_HOME`） | `~/.agent-link`（`AGENT_LINK_HOME`） |

名前の対応の全文は、実装計画書 U05・U06・U07 の共有契約 C2 にある。

## なぜ

- 今の開発の道具は sai-moon（`~/workdir/Sai/sai-moon`）である。PATH の `sai-link`、Mac の 8200 番の Web、daemon、Radio は、どれも sai-moon から動いている。sai-moon は 10/07（commit `df43628`）のまま止まっていて、そのあとの sai-link の改善（`run wait`、`task create` の model 指定、History、tmux の輸送路）は sai-tree にしか無い。
- 利用者は Paper・Research・sci・Radio を開発の道具では使っていない。
- sai-tree（開発するアプリ）と開発の道具が同じ名前（`sai-link`・`SAI_PROJECT_ROOT`・`.sai/`）を使っている。そのため、道具の環境変数がアプリのテストに混ざる事故が起きた（子が本物のプロジェクトで `init` した）。名前を分ければ混ざらなくなる。

## 決定事項と理由（利用者と決めたこと）

| 決定 | 理由・補足 |
|---|---|
| 元のコードは sai-tree（sai-moon ではない） | sai-link が新しく、Mac 専用の機能もすでに消してある |
| 複製して別のリポジトリにする（sai-tree の固定版を使う案は採らない） | 利用者の選択。使わない機能を消したい。sai-link の直しを sai-tree と agent-link の両方に入れる手間が続くことは了解済み |
| 複製の前に、sai-tree の `feat/customer-support` と `fix/shm-before-open` を main に入れる | 質問の取り下げ・停止の改善など、汎用の改善を含めるため |
| Mac 専用の機能（launchd・承認・遠隔ブラウザ・5 時間枠・pi）は戻さない | 利用者の選択。daemon と Web は手で起動する |
| Customer サポート・多人数モード・MLflow・sai-ts・sai-prep も消す | 利用者の選択 |
| 実行ファイル・環境変数・フォルダ・置き場・DB の名前を全部変える | 利用者の選択。sai-tree と混ざらないようにする |
| sai-tree プロジェクトの今の記録（run 1〜289）は移さない。agent-link は空の `.agent-link/` から始める | 利用者の選択。DB に絶対パスが多く入っていて（`execution_history.session_dir`・`block_metadata.file_path`・waggle の `manifests`）、移すと重い。古い `.sai/` は消さずに残し、必要なら sai-moon の CLI で読む |
| sai-moon は Web・daemon・Radio を止めて、フォルダとデータは残す | 利用者の選択。west などの研究プロジェクトも sai-moon に残す |
| 作業は agent-link を sai-moon の project にして行う。計画書は agent-link の `docs/plan/` に置き、パスで渡す | 利用者の選択。waggle のトークンはプロジェクトをまたげない。agent-link の作業用コピー（worktree）を使って worker を並列にできる |
| JS・Rust・Python の中だけで使う名前（`saiApi`・`window.SaiRoutes`・`sai_home()` など）は変えない | 外から見えない。変えると差分が大きくなるだけで、利用者には何も変わらない（plan の判断） |
| Linux のテスト利用者は tailnet に招き、ACL で Linux だけを許す | 利用者の選択。切り替えの手順書に ACL の例を書く |

## 完了したときに利用者が確かめられること

1. `~/workdir/Sai/agent-link` の `crates/` には `agent-link-engine`・`agent-link-cli`・`agent-link-view` だけ、`apps/` には `web` だけがある。
2. `cargo build --release` で `target/release/agent-link` ができ、`agent-link --help` と `agent-link schema --json` が動く。
3. agent-link のテスト（Rust・Web の JS・Web の Python）がすべて通る。命令は agent-link の `AGENTS.md` に書いてあり、CI と同じ。
4. 一時フォルダで `agent-link init` を実行すると、`.agent-link/` ができ、`.sai/` はできない。`.claude/settings.json` のフックと許可は `agent-link` を指す。
5. `agent-link-web serve --port 8297` を合言葉（`AGENT_LINK_WEB_TOKEN`）付きで起こすと、Home・Projects・Chats・Workflow・Usage が使える。Paper・Research のタブも、ログイン画面も無い。iPhone（幅 390px）と PC（幅 1280px）の両方で見られる。
6. `git grep` で古い名前（`sai-link`・`sai-web`・`SAI_`・`.sai/`・`sai-engine`・`sai_web`・`sai-paper`・`sai-sci`）を探すと、許した所（U07 の一覧）しか出ない。
7. 切り替えの手順書（`docs/switch-over.md`）に次の手順が書いてある。sai-moon を止める、PATH を agent-link に向ける、sai-tree で `agent-link init` する、8200 番で合言葉付きの Web を起こす、Tailscale の ACL を設定する。利用者がこの手順どおりに切り替えると、sai-tree のプロジェクトを agent-link で動かせる。
8. sai-moon のリポジトリは変わっていない。

## 範囲外（今回はしない）

- sai-tree プロジェクトの今の記録（run 1〜289）を agent-link に移すこと。
- Mac 専用の機能（launchd・承認・遠隔ブラウザ・5 時間枠・pi）を戻すこと。
- sai-tree のコードを直すこと。例外は複製の前の merge と、切り替えの手順書で利用者が行う `.gitignore` への 1 行の追加。
- agent-link を Linux に置くこと。
- これから sai-tree で直した sai-link を agent-link に移す仕組み（手で移す）。
- 切り替えそのもの（sai-moon を止める・PATH・Tailscale の ACL）。手順書を書くだけで、実行は利用者が行う。

## 参照文書

- 調査報告書 X1（sai-link の crate の研究向けの部分）: sai-tree プロジェクトの waggle:MizF9uqb（run 290）
- 調査報告書 X3（名前の変え方とデータの移し方）: sai-tree プロジェクトの waggle:m88AApdz（run 291）
- 調査報告書 X2（apps/web の残す・消す）: 保存されていない。要点は計画書 U02・U03 に写した
