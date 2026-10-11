# sai の概念と用語

この文書は、sai の専門的な概念と用語を 1 か所で説明する。用語の意味を調べるときは、まず §7 の索引から節を引く。

- **読み方**: 各節は短い説明と、「用語・意味・どこにあるか」の表でできている。「どこにあるか」は、画面の住所・CLI の命令・コードの場所（ファイル名と関数名・型名。行番号は書かない）のどれか。
- **言葉の方針**: 専門用語は英単語のまま使う（Next Action・History・PATH・SPROUT・Library・Import・Dashboard など）。案内文と説明は日本語で書く。`data-tab`・住所・API の名前などの識別子も英語のまま。
- **正はコード**: この文書とコードが食い違ったら、コードに合わせて直す。確かめきれていない文には（要確認）を付ける。
- **ほかの文書**: 使い方と入れ方は [`README.md`](./README.md)、層の分け方と守る約束は [`ARCHITECTURE.md`](./ARCHITECTURE.md)、このリポジトリで作業する人への指示は [`AGENTS.md`](./AGENTS.md)、API と画面の住所は [`docs/integration.md`](./docs/integration.md)、CLI の約束と Web が読む JSON は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md)、research-tree の使い方は [`docs/sai-sci.md`](./docs/sai-sci.md)、手を動かして覚えるなら [`docs/sai-sci-tutorial.md`](./docs/sai-sci-tutorial.md)。

---

## 1. 全体の見取り図

sai は、4 つの部品（sai-link・sai-paper・sai-sci・sai-web）と 2 つの周辺（`apps/radio`・`apps/mlflow-server`）でできている。人は CLI と Web を使い、エージェントは CLI を Bash で呼ぶ。画面は Web だけ。

```
 人: ブラウザ（iPhone / PC）              人・エージェント: CLI
        │ HTTPS                                      │
        ▼                                            │
   sai-web（apps/web・8200）                         │
        │  3 つの CLI（--json）と HTTP API だけを使う │
        ├──────────────┬────────────────────────────┤
        ▼              ▼                            ▼
   sai-link ◀──CLI── sai-sci ──ライブラリ──▶ sai-paper
 （sai-engine.db・     （research-tree。     （papr.db・
   .sai/outputs）       papr.db の rt_* 表）   論文のフォルダ）
        sai-web ──HTTP──▶ apps/radio（8201）・MLflow（5001）
```

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **sai** | アプリ全体の名前。 | リポジトリ全体（`ARCHITECTURE.md` §1.1） |
| **sai-link** | 基盤。coding agent の実行・カード・常駐・文書を扱う。他の部品を知らない。 | `crates/sai-link-engine`・`sai-link-view`・`sai-link-cli`。実行ファイルは `sai-link` |
| **sai-paper** | 研究の基盤（論文）。検索・取得・翻訳・音声化の依頼・Library を扱う。sai-link と sai-sci を呼ばない。 | `crates/sai-paper-core`・`sai-paper-cli`。実行ファイルは `sai-paper` |
| **sai-sci** | 研究の仕組み。sai-paper をライブラリで、sai-link を CLI で使うアプリ。単独では動かない。 | `crates/sai-sci-core`・`sai-sci-cli`、md の部品は `sai-sci/`（実行ファイルに埋め込む）。実行ファイルは `sai-sci` |
| **sai-web** | Web の窓口。画面と、質問・通知・使用量・中継だけを持ち、他の DB を開かない。多人数モードではアカウントとプロジェクトのコンテナも管理する。 | `apps/web`、CLI は `apps/web/bin/sai-web`、待ち受けは 127.0.0.1:8200 |
| **apps/radio** | 論文の解説音声を作って配る裏方サーバー。画面を持たない。 | `apps/radio/src/sai_radio/`、8201 番（TTS プロキシは 8202） |
| **apps/mlflow-server** | 実験の記録（MLflow）を溜めるサーバの compose（postgres・minio・mlflow・caddy）。Docker で動かす。 | `apps/mlflow-server/`、5001 番 |
| **waggle** | 文書に短いトークンを付けて受け渡す仕組み。vendor したクレート群。 | `waggle/`（`waggle-core`・`waggle-store`・`waggle-store-sqlite`）。使う側は `crates/sai-link-engine/src/waggle_integration.rs` |
| **LabAgent** | sai-sci が配る研究用の block と template、およびそれで動く agent。 | `sai-sci/blocks/`・`sai-sci/template/`（§4） |
| **research-tree** | Q・A・F・E・S の Node でできた木。研究の「何をやるか」を決める段階を扱う。 | `papr.db` の `rt_*` 表、`crates/sai-sci-core/src/rt/`、画面は Research タブ（§4） |
| **Experiments（実験）** | MLflow に溜まった実験の記録を見る画面。 | サーバは `apps/mlflow-server`、画面は `#/research/experiments`（`mlflow.js`） |
| **`sai-engine.db`** | sai-link の DB。run・card・子タスクなどの記録が入る。sai-sci と sai-web は開かない。 | `<root>/.sai/sai-engine.db` |
| **`papr.db`** | sai-paper の DB。論文・検索の記録・節と塊・research-tree の `rt_*` 表が入る。sai-link は開かない。 | `<root>/.sai/paper/papr.db`。`sai_paper_core::paths` の `PaperRoots`（`data_dir` が `<root>/.sai/paper/`） |
| **プロジェクト（`<root>/.sai/`）** | 作業の単位となるフォルダ。中の `.sai/` に DB・block・template・settings・成果物（`outputs/`）・論文（`paper/`）を置く。 | `sai-link init` が作る。`SAI_PROJECT_ROOT` か、今のフォルダから上へたどって `.sai/` を持つ最初のフォルダ |
| **`SAI_HOME`** | 利用者全体の置き場。環境変数 `SAI_HOME`（空でない値）が優先、無ければ `~/.sai`。登録簿・共通の block / template / settings・`paper.env`・`web/` が入る。プロジェクトの `.sai/` とは別。 | `sai_link_engine::project::sai_home()`・`sai_paper_core::paths::sai_home()`・`registry.sai_home()`（Python）。**3 つの規則をそろえる** |
| **登録簿（registry）** | 全プロジェクトの一覧。`sai-link init` が書き、Web のプロジェクト一覧が読む。 | `$SAI_HOME/registry.json`、読む側は `apps/web/src/sai_web/registry.py` |

---

## 2. 実行と文書（sai-link）

sai-link は、エージェントを動かし（run）、仕事の依頼を札にして（card）、成果を文書として残し（doc）、文書を waggle トークンで受け渡す。

run の一生（フォルダは状態に合わせて移る）:

```
card（todo）    …… .sai/outputs/todo/<run_id>/      task.md だけがある。まだ動いていない
   │ sai-link card run
running         …… .sai/outputs/running/<run_id>/   runner が pane の中で動く
   │ 文書を保存する（sai-link doc write）と `file_written` の通知が残り、常駐が完了を検出する
review          …… .sai/outputs/review/<run_id>/    利用者の確認待ち
   │ sai-link run advance（次の状態へ進める。review → done → Workflow）
done            …… .sai/outputs/done/<run_id>/      確認済み（Workflow へ移すと R<global_round>/ になる）
```

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **run（実行）** | エージェントが 1 回動いた記録。`run_id`（数）で呼ぶ。成果物は run フォルダに置く。 | `sai-link run list\|show\|tree\|log\|stats\|history\|md\|finalize\|advance\|rm\|export`、`sai-engine.db` |
| **`run_id`** | run の番号。run を指す引数を省くと、sai-link は環境変数 `SAI_PARENT_RUN_ID`（sai-link が起動時に渡す「自分の run」）を使う。 | `sai-link` の引数 `RUN_ID`、`cmd_run.rs` |
| **run フォルダ** | run の成果物（`task.md`・`message.md`・`system-prompt.md`・`session/`・文書）を集めたフォルダ。状態に合わせて `todo`・`running`・`review`・`done`、Workflow へ送ると `R<global_round>` に移る。 | `.sai/outputs/<状態>/<run_id>/`。`crates/sai-link-engine/src/run_dir.rs` の `owner_for`・`find_run_dir` |
| **card（カード）** | まだ実行していない run。todo にある `task.md`（frontmatter と依頼文）。 | `sai-link card create\|schedule\|run\|wait\|options\|docs`、`crates/sai-link-engine/src/kanban.rs` |
| **`task.md`** | card と run の依頼書。frontmatter（`runner`・`block`・`model`・`thinking`・`provider`・`executer`・`scheduled_at`・`after_job`・`source_doc` など。研究では `sci_node`・`sci_purpose` なども）と本文の `# task_text` でできる。 | run フォルダの直下。読み書きは `kanban.rs`（`FIXED_FM_KEYS`・`META_FM_KEYS`）・`rt::task_keys` |
| **`# task_text`** | `task.md` の依頼文の節。エージェントが読んで動く。`waggle:<トークン>` と書くと、実行時にその位置へ文書の本文が埋め込まれる（1 段だけ）。 | `task.md` の本文。展開は `crates/sai-link-engine/src/placeholder.rs` |
| **doc（文書）** | waggle トークンの付いた文書。run フォルダに保存し、他の run が読む。 | `sai-link doc write\|read\|list\|search` |
| **waggle トークン** | 文書を指す短い名前（例 `AXANSMpo`）。文書は必ずトークンで渡し、パスは渡さない。参照は `waggle:<トークン>:path:.sai/outputs/*/<run_id>/<ファイル名>` の形（正はトークン）。 | `sai-link waggle resolve`、`waggle/` |
| **block** | エージェントの役の定義。frontmatter に `model`・`thinking`（既定値。親の指定が優先）・`templates`・`worktree`・`enable` などを書き、本文が役の指示になる。 | `.sai/blocks/<id>.md`・`$SAI_HOME/blocks/`。既定の定義は `crates/sai-link-engine/assets/blocks/`。型は `block_config.rs` |
| **template** | block の本文に差し込む共通の決まり。block の `templates:` に名前を並べる。 | `.sai/template/<名前>.md`・`$SAI_HOME/template/`。既定は `crates/sai-link-engine/assets/template/` |
| **def（定義）** | block・template・settings の総称と、それを読み書きする命令。 | `sai-link def list\|show\|write\|delete\|preview`、画面は Projects の Settings（設定） |
| **settings** | プロジェクトの設定（`context`・`job_status` など）。`$SAI_HOME/settings.json` が、プロジェクトの `.sai/settings.json` を**キー単位で上書き**する。 | `.sai/settings.json`・`$SAI_HOME/settings.json` |
| **runner** | card を動かす実行者。cc（Claude Code。既定）・`gl`（sai-gemini-tui）・`user`（利用者が自分で行う札）。pi は廃止し、古い値は cc として読む。起動は 2 つの経路（Card で待たずに起こす・`task create` で待つ）。 | `crates/sai-link-engine/src/runner.rs` の `RunnerKind`、`sai-link card start\|finish` |
| **`executer`**（廃止） | card を**起こす人**の区別。`sai-sci card` に `--executer` は無い。作る Card は人が起こす Card で、人の代わりに Professor が起こす。古い Card の `executer` 項目は読むだけ（`agent`・`professor` は `agent` として読む）。`runner` とは別の項目。 | `kanban.rs` の `parse_executer`・`executer()` |
| **常駐（daemon）** | プロジェクトごとの背景のプロセス。予約した card・ジョブ待ちの card の実行と、状態の見張り・完了の検出をする。 | `sai-link daemon start\|stop\|status\|reload\|recover`、`crates/sai-link-engine/src/daemon.rs` |
| **`job_status`** | `.sai/settings.json` の項目。実験などのジョブの状態を確かめる命令（文字の配列。`{job}` がジョブの ID に置き換わる）。出力の `state` が `done`・`failed`・`cancelled`・`lost` になればジョブは終わり。 | `daemon.rs` の `job_status_command`。`sai-sci init` が書く |
| **review** | run の状態のひとつ。完了して利用者の確認を待っている。 | `.sai/outputs/review/`、`run_dir.rs` の `REVIEW_OWNER` |
| **Workflow** | Projects の画面のひとつ。run を札と線で描いた図と、Kanban。配置は sai-link が計算する。 | `#/project/<名前>/workflow`、`sai-link workflow`、`workflow.js` |
| **Kanban** | card と run を 4 つのレーン（todo・running・review・done）に分けた見方。 | `kanban.rs` の `KanbanLane`・`classify_lane` |
| **擬似 pane（headless）** | headless の run の区画。id は `hl-` で始まる（`hl-` + 12 桁の 16 進）。状態は `state.json` に書かれ、Web は一覧の `live` の判定に使う。sai-tree では herdr の窓は使わない（`herdr_pane_id` のキー名だけ残る）。 | `crates/sai-link-engine/src/headless.rs`、`state.json`（C1） |
| **子タスク** | 親の run が作る子の run。親は `task create` で依頼し、子は最後に `task report` で報告し、親が `task finalize` で確定する。 | `sai-link task create\|execute\|wait\|list\|show\|capture\|send\|interrupt\|delete\|finalize\|requeue\|report\|question` |
| **worktree（作業用コピー）** | block の `worktree:` が決める、run の作業フォルダの分け方。`none`（分けない）・`isolation`（git の worktree とブランチ `<block 名>-<run_id>` を作る）・ブランチ名そのまま。 | `orchestrator.rs` の `resolve_worktree_branch` |
| **finalize** | 完了を確定すること。root の run は `sai-link run finalize [RUN_ID] --report <文>`、子タスクは親が `sai-link task finalize <タスク ID>`。子タスクの run フォルダは、確定するまで `running` のまま。 | `cli.rs` の `RunCmd::Finalize`・`TaskCmd::Finalize`、`run_dir.rs` の `subtask_owner` |
| **`context`（settings）** | block の `context: true` のとき、実行の直前に `.sai/settings.json` の `context` の命令を実行し、標準出力を最初の文の `# context` の節に入れる。 | `crates/sai-link-engine/src/pre_run.rs`、研究では `sai-sci context` |

---

## 3. 論文（sai-paper）

論文は、Library に取り込み、本文を `extract.md` にし、節と塊に分けて読み、検索し、引用する。論文の番号 `papers.id` が、すべての入口になる。

### 3.1 論文と置き場

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **`papers.id`（論文の番号）** | Library の番号。文書では `#57` と書く。 | `papers` 表の主キー。画面の `#/papers/library/<番号>` |
| **論文の識別子** | 外から付いた論文の名前。`s2_paper_id`（Semantic Scholar）・`arxiv_id`・`doi`。 | `papers` 表。同じ論文を見分けるキー |
| **論文フォルダ** | 論文 1 本の PDF・本文・図・ノート・音声の置き場。 | `<root>/.sai/paper/papers/{id:04}_短いタイトル/`。`paths.rs` の `paper_file_names` |
| **`extract.md`** | docling が PDF から作る Markdown の本文。画像の行は検索と節の分け方から除く。 | 論文フォルダ。作るのは `figures.rs`（図の抽出と同時） |
| **図（figure）** | PDF から抜き出した図表。画像は `figures/fig-NNN.png`。 | 論文フォルダ。`sai-paper paper figures`、`figures.rs` |
| **図の抽出キー** | 図を探す名前。`s2_paper_id` → `arxiv_id` → `doi` の順で最初の値。どれも無ければ `paper:<papers.id>`。 | `figures.rs` の `extract_key_for_paper`・`figure_key_for_paper` |
| **`note.md`** | 論文ごとの利用者のメモ。検索の対象（scope `notes`）にもなる。 | 論文フォルダ。`sai-paper paper note` |
| **`paper.env`** | 全プロジェクト共通の API キー。プロジェクトの `.env` より優先順位が低い。 | `$SAI_HOME/paper.env` |

### 3.2 取り込み

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **Library** | 取り込んだ論文の一覧と階層。画面では左に検索の階層、右に検索結果と論文の中身を出す。 | `#/papers/library`、`sai-paper library list\|tree`、`papers.js` |
| **Import（BibTeX）** | BibTeX（`.bib`）を読み込み、一覧から選んだ論文を登録する。 | `#/papers/import`、`sai-paper import list\|add\|remove`、`paperimport.js` |
| **Download** | 論文の PDF を取って登録する。2 段で、PDF の取得と登録（`paper download`）のあと、`after-download` が続く。 | `sai-paper paper download`、Web は背景で走らせ 202 を返す |
| **`after-download`** | Download の後の自動処理。**図の抽出 → 節の表（`paper_sections`・`paper_passages`）→ PDF の翻訳**の順で走る。 | `sai-paper paper after-download`、`crates/sai-paper-core/src/pipeline.rs` の `after_download` |
| **翻訳** | 要旨の日本語訳（`paper translate`）と、PDF の翻訳（`paper translate-pdf`。日本語版は `paper-ja.pdf`）。 | `sai-paper paper translate\|translate-pdf` |
| **解説音声（Radio）** | 論文の解説音声。`sai-paper paper radio` が `apps/radio` に依頼して作らせる。 | `apps/radio`、論文フォルダの `radio.m4a` |

### 3.3 節・塊・チャンク

`extract.md` は、見出しで**節**に、空行で**塊**に分けて、`papr.db` に保存する。ID は `extract.md` の中身の順番だけで決まるので、同じ本文なら作り直しても変わらない。

| 書き方 | 指すもの |
|---|---|
| `#57` | 論文 57 の全体 |
| `#57.3` | 論文 57 の 3 番目の節 |
| `#57.3.2` | 論文 57 の 3 番目の節の 2 つ目の塊 |
| `#57.3.2-4` | 同じ節の塊 2〜4 の範囲 |

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **節（section）** | 見出しで分けた本文のまとまり。番号は 1 始まりで、最初の見出しより前の本文は節 0。種類は `front`・`abstract`・`body`・`references`。 | `paper_sections` 表。`crates/sai-paper-core/src/sections.rs` |
| **塊（passage）** | 節を空行で分けた本文の単位。1 つは 1500 字まで、100 字より短い段落は前の塊につなぐ。 | `paper_passages` 表。`sections.rs` の `build_from_markdown` |
| **塊の ID（`#x.y.z`）** | `#<論文>.<章>.<段落>`。引用・読む命令・検索結果の出典で使う。 | `sections.rs` の `parse_ref`・`PassageRef` |
| **チャンク（画面）** | 画面（論文詳細の「チャンク」一覧）と `content chunks` で、**塊**のことをこう呼ぶ。次の表の `chunks` 表の行とは別物。 | `sai-paper content chunks`、`papers.js`、`papers.py` |
| **`chunks`（RAG の索引）** | `sai-paper index` が検索用に切った断片の表の行。索引ごとに持ち、作り直すと ID が変わる。 | `chunks`・`chunk_fts` 表、`sai-paper index build\|rebuild\|embed`、`chunking.rs` |
| **チャンク化の戦略** | RAG の断片の切り方。`abstract`・`sentence`・`paragraph`・`window`・`section` の 5 つ。 | `chunking.rs` の `Strategy` |
| **`content`** | 論文の本文・節・塊を標準出力へ出す命令群。`outline`（節の一覧）・`read`（ID で読む）・`chunks`（全塊）・`paper`（全文）・`chunk`（`chunks` 表の 1 行）・`build`（表を作る）。 | `sai-paper content …`、`crates/sai-paper-cli/src/content.rs` |

「チャンク」の 2 つの意味の違い:

| 項目 | 塊（画面のチャンク） | `chunks` 表の行（RAG の索引） |
|---|---|---|
| 何か | `extract.md` を見出しと空行で分けた、読むための単位 | 検索のために切った断片（戦略ごと） |
| ID | `#<論文>.<章>.<段落>`。作り直しても変わらない | `chunks.id`。索引ごとで、作り直すと変わる |
| 表 | `paper_sections`・`paper_passages` | `chunks`・`chunk_fts` |
| 作る命令 | `sai-paper content build`（`after-download` でも作る） | `sai-paper index build\|rebuild\|embed` |
| 使う命令 | `content outline\|read\|chunks` | `sai-paper find` |
| 両者の対応 | 検索のヒットの出典は、`sections::passages_for_hits` が本文の先頭 120 字で塊を探して `#x.y.z` に引き直す。対象は `source_kind` が `body` のヒットだけで、節の表がまだ無い論文は飛ばす | — |

### 3.4 検索

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **外部 DB の検索** | Semantic Scholar・arXiv・PubMed・bioRxiv・OpenAlex など外の DB を検索する。結果は検索履歴に残る。 | `sai-paper search <語> --db …`、`sai-paper providers` |
| **内部 DB の検索** | Library の中（論文の本文・ノート・図）を検索する。結果は内部検索の履歴に残る。 | `sai-paper find <語>` |
| **検索モード** | 内部 DB の検索の方式。`word`（FTS5）・`vector`（埋め込み）・`hybrid`（両方を融合。`--alpha`・`--fusion`）。 | `sai-paper find --mode` |
| **検索の範囲（scope）** | 内部 DB の検索で絞る論文の集合。Library 全体・`--papers`（番号の組）・`--node`・`--subtree`。`--node` は範囲を絞るだけ（記録を Node に結ぶのは `SAI_TARGET_NODE`）。`--node` は research-tree の Node（`Q12` など）で、**`sai-sci find` だけにある**。範囲の論文は、その Node（と直の子。`--subtree` なら子孫すべて）の F の `paper_id` と `cites` の論文番号を集めたもの。**その論文の全文（全チャンク）が範囲**で、`#x.y.z` の 1 塊だけではない。Node の下に論文が 0 本のときは Library 全体にせず 0 件で記録する。 | `crates/sai-sci-core/src/rt/scope.rs` の `resolve_scope`、`crates/sai-sci-cli/src/find_search.rs` |
| **find** | Library（内部 DB）の検索。履歴は `find_history`。 | `sai-paper find`・`sai-sci find` |
| **search** | 外部の論文 DB の検索。履歴は `search_history`。 | `sai-paper search`・`sai-sci search` |
| **search_history / find_history** | 検索 1 回ごとの記録（検索語・条件・結果・`executer`・`run_id`・実行日時）。外部 DB の検索は `search_history`・`search_history_items`、内部 DB の検索は `find_history`・`find_history_hits` に残る。 | `search_history.rs`・`find_history.rs`、`sai-paper search-history\|find-history` |
| **`executer`（検索の履歴）** | 検索を実行したのが `human` か `agent` か。カードの `executer`（起動者）と同じ値。`SAI_ORIGIN=phone`（Web）は `human`、それ以外で `SAI_PARENT_RUN_ID` が数なら `agent`、他は `human`。 | `search_history.executer`・`find_history.executer` |
| **`SAI_TARGET_NODE` / `target_node`** | Node を対象に動く Agent と、その子 task に sai-link が入れる環境変数（子の task.md には `target_node`）。決め方は task.md の `sci_node`（`SEED` 以外）→ `target_node` → 親の run の順。`sai-sci find`・`search` の記録がその Node に結ぶ。 | `crates/sai-link-engine/src/target_node.rs` |
| **廃止した物** | 「検索セット」と選別（`pending`・`keep`・`drop`）は廃止した。検索結果は履歴に残るだけで、選別の段は無い。 | ― |
| **外部 DB** | Library の見方のひとつ。外部 DB の検索の履歴（`query`）と、検索に結び付かない Import の論文を、日付ごとの階層で並べる。ここから外部 DB の検索もする。 | `index.html` の `data-kind="external"` |
| **内部 DB** | Library の見方のひとつ。Library の中を検索し、その履歴（`find_history`）を日付ごとに並べる。 | `data-kind="internal"`、`papers.js`（`#/papers/library/find/<番号>`） |
| **Paper** | Library の見方のひとつ。取り込んだ全論文を番号順に並べる。 | `data-kind="paper"` |
| **`rt_searches`** | research-tree の検索の記録。どの Node のために、どのツール（`find`・`search`）で、どの範囲を検索し、何件当たったかを残し、検索の履歴（`log_id`）と結ぶ。 | `papr.db`、`sai-sci find\|search` |

### 3.5 引用・ノート・そのほか

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **引用（cite）** | 論文の出どころの書き方。`#57`・`#57.3`・`#57.3.2`・`#57.3.2-4` のあとに、空白と短いメモを付けてよい。F の `cites` や報告書で使う。 | `crates/sai-paper-core/src/textcheck.rs` の `parse_cite` |
| **`textcheck`** | 報告書の検査。日本語の文に混ざったハングル・キリル文字・簡体字と、引用の形を確かめる。 | `textcheck.rs` の `foreign_chars`・`parse_cite`、`sai-sci report` |
| **引用の抜き出し** | PDF から引用文献を取り出す処理。時間がかかるので背景で走る。 | `sai-paper paper citations-extract\|citations`、画面は `#/paper/<プロジェクト>/<論文の番号>/cites`（`cite.js`） |

---

## 4. 研究（sai-sci の research-tree）

research-tree は、研究の方向づけ（実験の前の「何をやるか」）を、問い（Q）・答え（A）・根拠（F）・実験（E）・seed（D）・SPROUT（S）の木で扱う。木は seed・root・sprout の 3 つの役に分かれる（§4.1）。正は `papr.db` の `rt_*` 表で、絵と文章（`state.md`・`tree.md`）はそこからの書き出し。木を変えられるのは、agent の `sai-sci op`、利用者の `sai-sci apply` と Web だけである。

### 4.1 層（seed・SEED・ROOT・SPROUT・BRANCH）と Node

- **root** は分野の可能性の幅で、seed に縛られない。Q は「〜の種類は何があるか」の形、A は選択肢 1 つ、F は論文の事実。全部の seed が同じ root を使う。
- **seed（D）** は 1 つの研究の制約で、親を持たず、複数持てる。
- **sprout（S）** は 1 研究＝研究計画で、1 つの seed と、軸ごとに選んだ root の path（A）の組。
- 1 研究＝1 つの S＝1 つの D ＋ 軸ごとに 1 本の path の組。先行研究を論文から復元した研究と自分の研究は、木の上で区別しない（どちらも D と S）。自分の研究の新しさは、先行研究の S と違う seed・path を通ることで示す（agent の文で報告する）。

```
E（BRANCH）── S か E の下。実験の 1 回
 │
S（SPROUT）── 1 研究＝研究計画。親は D。paths（軸ごとの A）を 1 つ以上
 │
D（seed）──── 1 研究の制約（9 項目）。親なし。複数。今の SEED の札の横に並ぶ
SEED ──────── root の根（RESEARCH_TYPE と mind.md）。Node ID は SEED・DB の行なし
 │
Q（軸の Q＝top_Q）── 「〜の種類は何があるか」。task・target・data・method・output・metric
 └ A（選択肢）── 出典を書かない（op が断る）
     ├ F（論文の事実。stance は support・deny・issue）
     └ child_Q（その A の下の種類・条件）…
```

seed の本文は 9 見出し（`## 背景`・`## 解く問題（task）`・`## 対象（target）`・`## データ（data）`・`## 手法（method）`・`## 出力（output）`・`## 評価指標（metric）`・`## 成功基準`・`## 制約`）で、軸の 6 項目は見出しの次の行が `決定: <値>` か `空欄`。S の body（研究計画）は `## 研究計画`・`## 成功基準`・`## 矛盾`・`## 先行研究との違い`。対象が S の `# context` には `## STUDY_SEED`（その S の seed）が出る。

| 種類 | 意味 | ID の例 | 行 |
|---|---|---|---|
| `Q` | 問い。child_Q を持てる | `Q12` | `rt_nodes` |
| `A` | 答えの候補（旧 `O`） | `A15` | `rt_nodes` |
| `F` | 根拠。stance を持つ。出典 `cites` か REPORT のトークン `report` | `F3` | `rt_nodes` |
| `E` | 実験。MLflow の記録（`mlflow`）を結べる | `E2` | `rt_nodes` |
| `S` | SPROUT（研究計画）。親は D。PATH（A の ID）を集める | `S3` | `rt_nodes` |
| `D` | seed。1 つの研究の制約。親なし | `D1` | `rt_nodes` |
| `SEED` | root の根（RESEARCH_TYPE と mind.md）。DB の行は持たない。seed（D）とは別 | `SEED` | `rt/mod.rs` の `SEED_ID` |

- ID の形は `^[QAFESD][1-9][0-9]{0,8}$`。頭文字が種類である。`O` の種類は無くなった。
- 親子の規則は `rt/ops.rs` の `execute` だけが守る（DB は守らない）。

| 子 | 置ける親 |
|---|---|
| top_Q（親 NULL） | `SEED` |
| Q | 生きた A の子を持たない Q、または A |
| A | 生きた Q の子を持たない Q |
| F | A だけ |
| E | S か E |
| S | D（seed） |
| D | 親なし |

- Q の下は「A だけ」か「child_Q だけ」で、混ぜない。A の下は F と Q を混ぜてよい。
- **child_Q** は Q の子の Q。**parent_Q** は親の Q。**bottom_Q** は生きた child_Q を持たない Q（JSON の項目は `bottom_q`）。
- **★** は利用者が付ける目印である。計算には使わない（`star`・`unstar` は利用者だけ）。
- **RESEARCH_TYPE** は研究の始まり方の型（5 本）。軸（6 つの Q）は全部の型で共通で、型の違いは seed のどの項目が最初から「決定」になりやすいか（`## SEED` の節）。**GUIDE** は軸の Q のガイド（6 本。Q の文・Child Q・Extract・Pitfall）。
- A に出典（`#番号`）は書けない（op が断る）。出典は A の下の F に書く。D の下に生きた S があると、その D は消せない。

### 4.2 PATH と SPROUT（S）

- **PATH** は軸の Q（top_Q）から A に至る道。終点の A で表す（例 `Q2 › Q12 › A15`）。PATH の終点になれるのは A だけ。
- **SPROUT**（`add_sprout`）は、`seed`（D の ID。必須）と `paths`（A の ID を 1〜20 個）を持つ。同じ A は 2 回書けない。1 つの seed から複数の S を作れる。
- S を作るのは `lab-sprout`（`--node D<n>`）か Create SPROUT。`lab-sprout` は seed との矛盾と、最も近い先行研究の S との違いを報告する。`sci-exp-sprout`（`--node S<n>`）は既にある S から実験を始める。root の骨組みを review 論文から作るのが `lab-map`。
- **範囲**（`range`）は、`paths` の各 A について、その A・A の祖先の Q と A・A の子の生きた F を集めて数える（重ならないように数える）。出すのは Q・A の数と F の stance ごとの数。A の子の Q 以下は入れない。
- 画面の操作: **Create SEED** で seed を作る。**Create SPROUT** で作成パネルを開き、先に SEED（D）を選び、A の板の **Add PATH** / **Delete PATH** で PATH を選ぶ。

### 4.3 op と承認

op は research-tree を変える操作 1 つ。`rt/ops.rs` の `OP_SPECS` が正。危険度は 3 つに分かれる。

- `Free`: agent がそのまま出せる。
- `Approval`: agent は承認（利用者の「許可する」）が要る。`needs_approval(op)` で判定する。
- `UserOnly`: 利用者（Web・`sai-sci apply`）だけ。

| op | 危険度 | 内容 |
|---|---|---|
| `add_q` | Approval | Q を足す。親は `SEED` か ID（4.1 の表どおり） |
| `insert_q` | Approval | Q の下に新しい Q を挟み、その Q の生きた A を全部その下へ移す |
| `move_a` | Approval | A を別の Q へ移す（A の子孫へは移さない） |
| `copy` | Approval | A と、その生きた F を別の Q へ写す（A の下の Q と E は写さない） |
| `remove` | Approval | Node を下の Node ごと消す |
| `add_a` | Free | A を足す。作ったあと GUIDE の `+` の子 Q を作る |
| `add_f` | Free | F を足す（3 文まで）。数字を含む文は出典（`cites` か `report`）が要る |
| `add_e` | Free | E を足す |
| `add_sprout` | Free | SPROUT を作る（4.2） |
| `set_stance` | Free | F の stance を変える |
| `set_mlflow` | Free | E の MLflow の結び付けを変える |
| `star` / `unstar` | UserOnly | ★ を付ける・外す |
| `edit` | UserOnly | 文と本文を直す |

- block の frontmatter `sci_ops` に書いた op だけを agent が出せる。`sci_ops` が無い block は、`Free` の op 全部を出せる。
- 利用者の `sai-sci apply` は、どの op も承認なしで行える。rt_log の `actor` は `user`、`approved_by`・`qid`・`question` は NULL になる。

**承認の流れ**（許可が要る op を agent が出すとき）:

1. 承認を取る質問は 1 回 1 問。先頭の選択肢を「許可する」、2 つ目を「許可しない」にする。質問文に、これから行う op の一覧（op 名と対象の ID・文）を書く（template の決まり）。
2. 利用者が「許可する」を選ぶ。cc の質問フック（`cc_question.py`）が、`SAI_PARENT_RUN_ID` のある run で `sai-sci approval record` を呼び、`rt_approvals` に 1 行足す（回答の記録）。
3. agent が `sai-sci op` で op を出す。許可を 1 件取り、全部通ったら書いて、その許可を消費する。rt_log の許可が要る op の行に `approved_by`・`qid`・`question` が入る。
4. 許可が無いと終了コード 4 で、「許可がありません: 先に質問ツールで「許可する」を選んでもらってください」と出る。何も書かれない。
- agent は `sai-sci approval record` を自分で呼ばない。
- 1 回の `sai-sci op` は 1 件の許可で、そのバッチ全体が通る。途中で 1 件でも弾かれると、何も書かれず許可も残る。

### 4.4 Action・Next Action・CARD

- **Action** は、Node 1 つに向けた 1 回の実行（CARD）。task.md を持つ。目的を 1 行で書く。
- **足す子の型**（`sci_child`・CLI は `sai-sci card --child <Q|A|F>`）は、対象の種類で決まる。Q → `Q` か `A`、A → `Q` か `F`。それ以外の組は終了コード 2。
- **Next Action** は、次にやるとよい Action の候補。上の帯に先頭 3 件が並ぶ。`kind` は全部 `card`。規則の順位（小さいほど先）は次のとおり。

| 順 | why | 対象 | block | 目的 | CARD の追加 |
|---|---|---|---|---|---|
| 1 | `NO_CHILD` | 生きた子の無い Q | `lab-grow` | 「<Q の文>」の子（A か child_Q）を作る | なし |
| 2 | `NO_FINDING` | 生きた F の子の無い A | `lab-search` | 「<A の文>」の F を探す | `child: F` |
| 3 | `UNVERIFIED` | 生きた子が 1 つ以上ある Q・A で、`sci-verify` がその Node を対象にしたことが無い、または直の子の一番新しい `created_at` が、その Node を対象にした一番新しい `sci-verify` の run より後 | `sci-verify` | 「<文>」と子の関係を確かめる | なし |

- **READ_CARD**（`runner: user`）の既定の block は `lab-read`。利用者が論文を読んで F を書く。
- **REPORT** は node agent・manage agent が `sai-link doc write` で保存する報告書。support agent は持たない。
- **LabAgent** は、ブロックの frontmatter の `sci_type`（node・manage・support）で動く agent。LabAgent の 6 本（`lab-read`・`lab-search`・`lab-grow`・`lab-survey`・`lab-place`・`lab-progress`）と、検証 agent（`sci-verify`）がある。
- **検証 agent**（`sci-verify`）は、target_node と子の関係を評価し、評価の報告・op・次の CARD を作る。参加者に `paper-debater`・`paper-reviewer` を子として使える。
- **mlflow**（E の記録）は、E が持つ MLflow の記録への参照。Experiments の画面が読む。
- **書き出し**（`state.md`・`tree.md`）は research-tree の文章と図。手で直しても次の書き出しで消える。
- **mind.md** はプロジェクトの覚え書き（`ORIGIN`・`INTERESTS`・`NOTES`）。利用者が書き、エージェントは直接書き換えない。研究ごとの制約は seed（D）に書く。
- **`rt_*` の表**: `rt_meta`・`rt_nodes`・`rt_log`・`rt_approvals`・`rt_seeds`・`rt_searches`（`crates/sai-paper-core/migrations/0056_rt_redesign.sql`）。古い `sci_*` の表は読まず、消さない。

### 4.5 History

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **History（Node ごと）** | 1 つの Node の変更の記録。先頭が `init`、残りは古い順。`Add Q`・`Add A`・`Add F`・`Add E`・`Run`・決定（`edit` など）を含む。 | `sai-sci node <ID> --json` の `history`（`node_detail.history`）。Node の板の「History」 |
| **History（全 Node）** | 全 Node の変更の記録を、`Add *` を除いて古い順に並べた物。 | `sai-sci status --json` の `history`（`Status.history`）。上の帯の「History」（折りたたみ） |

### 4.6 見た番号の台帳と CLI 呼び出しの History

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **見た番号の台帳**（`seen_ids`） | run ごとに「agent が CLI で見た論文の番号（`#37`・`#37.5`・`#37.5.2`）」を 1 行 1 つ追記するファイル。agent が見ていない番号を引用する（作り話の引用）のを止める。MLflow とは別の物。 | `<root>/.sai/seen_ids/<run_id>`。`sai-paper`・`sai-sci` が出力した番号、`sai-link` が run の最初の文と `doc read` の本文の番号を書く |
| **台帳の照合** | `sai-sci op` の `add_f` の `cites` と、`sai-sci report prepare` の引用の番号を、見た番号の台帳と照らす。無い番号があれば何も書かず（`op` は終了コード 2、`report prepare` は終了コード 1 で保存しない。`台帳に無い番号です: …`）。節・塊は完全一致、論文（`#37`）は `#37` か `#37.*` があれば通る。 | `crates/sai-paper-core/src/seen_ids.rs` の `check_for` |
| **実験の台帳**（`exp_summary.py`） | 実験（MLflow）の記録を集計して、最良・格子の端・失敗などを出す道具。見た番号の台帳とは別の物。 | `.sai/tools/exp_summary.py`・`exp-core` §5 |
| **CLI の呼び出し（History）** | run の transcript の toolCall と toolResult を 1 行にまとめた物。命令名（`sai-paper …` など）・結果の先頭 200 字（`preview`）・`is_error`・入力に使った番号を持つ。全文（2 万字まで）は `--call` で取る。 | `sai-link run history <run_id> --json`。Web の run の History |
| **cli 集計** | 呼び出しの数・失敗の分類（`sai-*` の呼び出しだけ数える。`ledger`・`input`・`not_found`・`conflict`・`other`）・番号の使用/未使用（`seen`・`used`・`unused`）。`kaizen` が読んで改善計画書に書く。 | `run history` の `cli` |

---

## 5. Web の画面

画面は `apps/web/src/sai_web/static/` にある。新しい画面は `window.SaiRoutes` に登録し、`app.js` に画面ごとの分岐を足さない。幅 900px（`ui.js` の `SAI_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列になる。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **Home** | 最初に出る画面。全プロジェクトの合計・Questions（質問）・todo・最近のアクティビティ・動いているもの・止まっている常駐・論文の要対応を並べる。 | `#/home`、`home.js`、`GET /api/home` |
| **Projects** | プロジェクトの一覧と、その中の Dashboard・Workflow・List・Settings（設定）・コマンド・会話検索（セッション検索）。 | `#/projects`、`#/project/<名前>/…`（`project.js`・`linkview.js`） |
| **Chats** | coding agent の会話を読み、指示を送り、質問に答える。 | `data-tab="agent"`、`app.js` |
| **Papers** | 論文の画面。中の切り替えは Dashboard・Library・Import・Radio の 4 つ。 | `#/papers/dashboard\|library\|import\|radio`、`papers.js` の `PAPERS_SEG` |
| **Research** | 研究の画面。中の切り替えは research-tree と Experiments の 2 つ。 | `#/research/tree`・`#/research/experiments`、`RESEARCH_SEG` |
| **Usage** | LLM の料金と制限。 | `#/usage`、`usage.js` |
| **Dashboard** | 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ。Projects と Papers にそれぞれある（Papers は数字カードと出来事の一覧）。 | `#/project/<名前>`・`#/papers/dashboard` |
| **List** | Projects の画面のひとつ。run のツリーから 1 つ選び、Log・Stats・History・task・message・report・system・waggle の表示で見る。 | `#/project/<名前>/list[/<run_id>/<表示>]`、`linkview.js` |
| **Settings（設定）** | Projects の画面のひとつ（画面の表示は `Settings`）。block・template・settings を読み書きする。 | `#/project/<名前>/defs`、`defs.js`、`linkview.js` の `LV_MODES` |
| **コマンド** | Projects の画面のひとつ（画面の表示は `コマンド`）。3 つの CLI の `schema --json` から、`web: true` の命令を入力欄つきで出して実行する。 | `#/project/<名前>/commands`、`commands.js` |
| **会話検索（セッション検索）** | Projects の画面のひとつ（画面の表示は `会話検索`）。会話の履歴を検索する。 | `#/project/<名前>/sessions`、`sessionsearch.js`、`sai-link session search` |
| **Radio** | Papers の画面のひとつ。論文の解説音声を聴く。声や文字で質問もできる。 | `#/papers/radio`、`radio.js`・`voiceask.js` |
| **DAG** | research-tree の中央の図。札（Node）と線（Parent / Child・SEED → top_Q・SEED → SPROUT）を描く。 | `scigraph.js`（`sgRenderGraph`） |
| **一覧** | research-tree の左の、Node の一覧と種類の絞り込み。 | `scitree.js`（`stRenderSidebar`） |
| **SB** | research-tree の右のサイドバー。選んだ Node の板で、何も選ばないと出ない。狭い画面では下から出る（`saiSheet`）。 | `scitree.js`（`stRenderBoard`）、住所 `#/research/tree/<ID>` |
| **Next Action の帯** | research-tree の上の帯。Next Action の先頭 3 件を並べ、Create CARD の釦を付ける。 | `scitree.js`（`stNextBand`） |
| **History の帯** | research-tree の上の帯。History（全 Node）を折りたたんで出す。 | `scitree.js`（`stHistoryBand`） |
| **作成パネル** | SPROUT 作成中だけ、History の帯と DAG の間に出るパネル。作成中も DAG・一覧・SB を操作できる。 | `scitree.js`（`stRenderSprout`） |
| **先着の答え** | 質問を PC と iPhone の両方に同時に出し、先に答えた方を採用する決まり。 | `apps/web/src/sai_web/questions.py` |
| **多人数モード** | 研究室の Linux サーバで動かす形。利用者は Web にログインして使い、プロジェクトごとに 1 つのコンテナで動く。`SAI_WEB_USERS_ROOT` が絶対パスのとき有効。 | `registry.py`（`users_root()`）、`auth.py`、`containers.py`、`docs/server.md` |
| **単独モード** | 1 人が Mac で使う形。`SAI_WEB_USERS_ROOT` が空のとき。合言葉（`SAI_WEB_TOKEN`）は任意。 | `registry.py`（`users_root()` が `None`） |
| **内部の合言葉** | コンテナの中の CLI・フック・拡張が Web を呼ぶための値（`SAI_WEB_INTERNAL_TOKEN`）。内部用の 3 つの住所だけで使える。利用者の画面には出さない。 | `containers.py`、`main.py`（`_INTERNAL_POST_EXACT`） |
| **個人のトークン** | 利用者が Account 画面（`#/account`）で発行する `sai_…` の値。sai-prep と CLI が Web に入るときに使う。他人と共有しない。 | `account.js` |
| **sai-prep** | 各自の Mac で論文の PDF を前処理し、前処理の束をサーバへ送るアプリ。標準ライブラリだけで動き、docling・pdf2zh・mlx-lm は `uv tool` の別の環境に入れる。 | `apps/prep/`、`sai-prep`、`README.md` の「sai-prep」 |
| **前処理の束** | sai-prep が作る zip。PDF・docling の md と json・図・翻訳 PDF を入れ、`manifest.json`（`format` が `sai-prep-bundle`）で中身を説明する（共有契約 C12）。サーバは展開して `import-bundle` で Library に入れる。 | `crates/sai-paper-core`（`bundle.rs`）、`apps/web/src/sai_web/bundles.py` |
| **移植の束** | 論文を別のプロジェクトへ移すためのフォルダ。`manifest.json`（`format` が `sai-paper-migration`）・論文の表だけを入れた `papr.db`・`papers/<フォルダ名>/` からなる（共有契約 C-M1）。`library export` で作り、`library import-migration` で取り込む。取り込みは両方の DB にある列だけを写し、論文の id は移植先で振り直す。 | `crates/sai-paper-core`（`migration.rs`） |
| **PDF 待ち** | PDF の無い論文。Library に `＋ Library に入れる` で入ると、PDF が届くまでこの状態になる。 | `sai-paper library pending`、`GET /api/projects/{name}/papers/pending`、`papers.js` |
| **`SaiRoutes`** | 画面を登録する場所。各画面の JS が「住所の照合と描画」を `window.SaiRoutes` に足す。 | `app.js` |
| **`saiTerm`（用語の札）** | 画面の用語に付ける、押すと説明が出る札。説明は `SAI_TERMS` に書く。 | `ui.js` |
| **画面の版** | 画面ファイルを変えて配るとき、`sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。 | `sw.js`・`index.html` |
| **`data-tab`** | 下のタブ（左の縦のタブ）の識別子。`home`・`project`・`agent`・`papers`・`research`・`usage`。 | `index.html` |
| **住所（`#/…`）** | 画面の URL の `#` 以降。research-tree は `stParseHash`、Library は `parseLibraryHash`、Experiments は `mlflow.js` が読む。古い住所（`#/papers/tree`・`#/papers/read/<run_id>`・`#/mlflow…`）は `stLegacyHash` が新しい住所へ送る。 | `scitree.js`・`papers.js`・`mlflow.js` |

---

## 6. CLI の約束

`sai-link`・`sai-paper`・`sai-sci` は同じ約束を守る。全文は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md)（§1）と [`ARCHITECTURE.md`](./ARCHITECTURE.md)（§2.3）。ここには 1 行の意味だけを書く。

| 用語 | 意味 | どこにあるか |
|---|---|---|
| **終了コード** | 0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他の失敗。失敗のとき標準出力は空で、理由は標準エラーの最後の行。 | `docs/for-sai-developers.md` §1 |
| **`--json`** | 成功したとき、標準出力に JSON を 1 つだけ書く指定。 | 各 CLI の命令 |
| **`schema --json`** | 命令の一覧（`bin`・`version`・`commands[]`）を出す命令。プロジェクトを要らない。Web の「コマンド」画面が読む。 | `sai-link\|sai-paper\|sai-sci schema --json` |
| **`sai-progress`** | 時間のかかる命令が標準エラーに出す進み具合の行。`sai-progress step=<数>`・`sai-progress msg=<文>`。 | 標準エラー。Web の Download が読む |
| **`SAI_PROJECT_ROOT`** | 対象のプロジェクトの root。空なら、今のフォルダから上へたどって `.sai/` を持つ最初のフォルダ。 | 環境変数 |
| **`SAI_ORIGIN`** | 記録の出どころ。無ければ `cli`。Web が呼ぶときは `phone`。 | 環境変数 |
| **`SAI_PARENT_RUN_ID`** | エージェントが動いている run の番号。sai-link が起動時に渡し、`run_id` を省いた命令が「自分の run」として読む。 | 環境変数（`pre_run.rs` が渡す） |

`SAI_HOME`（置き場）は §1。`SAI_WEB_*`・`SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN` などの表は [`README.md`](./README.md)。

---

## 7. 用語の索引

本文の表の左の列にある用語を、「用語 — 一言 — 節」の形で並べる。アルファベット（A〜Z）が先、日本語は五十音の順。詳しい意味と「どこにあるか」は、節の表を見る。

### A〜Z

- **4 layer** — research-tree の 4 つの層 — §4.1
- **Action** — research-tree の Node に向けた 1 回の実行 — §4.4
- **Add PATH** — 作成中に、A の板の釦で A を PATH に加える — §4.2
- **`after-download`** — Download の後の自動処理 — §3.2
- **承認（「許可する」）** — 許可が要る op を agent が出すための利用者の回答。1 件で 1 バッチ分 — §4.3
- **見た番号の台帳（seen_ids）** — agent が CLI で見た論文の番号の記録。MLflow とは別 — §4.6
- **実験の台帳（exp_summary）** — 実験の記録を集計する道具。見た番号の台帳とは別 — §4.6
- **CLI の呼び出し（History）** — toolCall と toolResult を 1 行にした物 — §4.6
- **apps/mlflow-server** — 実験の記録（MLflow）を溜めるサーバの compose — §1
- **apps/radio** — 論文の解説音声を作って配る裏方サーバー — §1
- **Free（危険度）** — agent がそのまま出せる op — §4.3
- **block** — エージェントの役の定義 — §2
- **bottom_Q** — child_Q を持たない Q — §4.1
- **BRANCH** — SPROUT から上へ分かれる E — §4.1
- **card（カード）** — まだ実行していない run — §2
- **CARD** — エージェントへの依頼 — §4.4
- **Chats** — coding agent の会話を読み、指示を送り、質問に答える — §5
- **child_Q** — 子にあたる Q — §4.1
- **`chunks`（RAG の索引）** — `sai-paper index` が検索用に切った断片の表の行 — §3.3
- **`content`** — 論文の本文・節・塊を標準出力へ出す命令群 — §3.3
- **`context`（settings）** — block の `context: true` のとき、実行の直前に走る settings の命令 — §2
- **`context`（`# context`）** — 最初の文に入れる、research-tree の範囲の節 — §4.4
- **Create CARD** — Next Action や Node の板から、CARD のフォームを開く釦 — §4.4
- **Create SPROUT** — SPROUT を作る釦と作成パネル — §4.2
- **DAG** — research-tree の中央の図 — §5
- **Dashboard** — 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ — §5
- **`data-tab`** — 下のタブ（左の縦のタブ）の識別子 — §5
- **def（定義）** — block・template・settings の総称と、それを読み書きする命令 — §2
- **Delete PATH** — 加えた A を PATH から外す — §4.2
- **doc（文書）** — waggle トークンの付いた文書 — §2
- **Download** — 論文の PDF を取って登録する — §3.2
- **E（実験）** — SPROUT か E の下に置く実験 — §4.1
- **`executer`** — card を起こす人（`human` か `agent`） — §2
- **Experiments（実験）** — MLflow に溜まった実験の記録を見る画面 — §1
- **`extract.md`** — docling が PDF から作る Markdown の本文 — §3.1
- **F（根拠）** — A の下に置く根拠。stance は support・deny・issue — §4.1
- **finalize** — 完了を確定すること — §2
- **GUIDE** — Q の項目ガイド — §4.1
- **History（Node ごと）** — 1 つの Node の変更の記録 — §4.5
- **History（全 Node）** — 全 Node の変更の記録（`Add *` を除き古い順） — §4.5
- **History の帯** — research-tree の上の帯 — §5
- **Home** — 最初に出る画面 — §5
- **Import（BibTeX）** — BibTeX を読み込んで論文を登録する — §3.2
- **`job_status`** — ジョブの状態を確かめる settings の命令 — §2
- **`--json`** — 成功したとき、標準出力に JSON を 1 つだけ書く指定 — §6
- **Kanban** — card と run を 4 レーンに分けた見方 — §2
- **LabAgent** — sai-sci が配る研究用の block と template、それで動く agent — §1
- **Library** — 取り込んだ論文の一覧と階層 — §3.2
- **List** — Projects の画面のひとつ — §5
- **sci_child** — CARD で足す子の型（`Q`・`A`・`F`）— §4.4
- **`mind.md`** — プロジェクトの覚え書き — §4.4
- **`mlflow`（E の記録）** — E が持つ MLflow の記録への参照 — §4.4
- **Next Action** — 次にやるとよい Action の候補 — §4.4
- **Next Action の帯** — research-tree の上の帯 — §5
- **Node** — research-tree の 1 つの要素 — §4.1
- **`note.md`** — 論文ごとの利用者のメモ — §3.1
- **A（答え）** — Q の下に置く答えの候補（旧 O）— §4.1
- **範囲（range）** — SPROUT が数える Q・A と F の数 — §4.2
- **OP（C-OP）** — research-tree を変える操作 1 つ — §4.3
- **擬似 pane（headless）** — headless の run の区画 — §5
- **Paper** — Library の見方のひとつ — §3.4
- **`paper.env`** — 全プロジェクト共通の API キー — §3.1
- **Papers** — 論文の画面 — §5
- **`papers.id`（論文の番号）** — Library の番号 — §3.1
- **`papr.db`** — sai-paper の DB — §1
- **parent_Q** — 親にあたる Q — §4.1
- **PATH** — 軸の Q から A に至る道 — §4.2
- **検証 agent（sci-verify）** — target_node と子の関係を評価する固定の block — §4.4
- **professor** — 研究を舵取りする root の役 — §4.4
- **Projects** — プロジェクトの一覧と、その中の画面 — §5
- **Approval（危険度）** — agent が出すと利用者の承認が要る op — §4.3
- **Q（問い）** — 研究の穴を表す Node — §4.1
- **Radio** — Papers の画面のひとつ — §5
- **READ_CARD** — `runner: user` の CARD — §4.4
- **REPORT** — エージェントが終わりに `sai-link doc write` で保存する報告書 — §4.4
- **Research** — 研究の画面 — §5
- **research-tree** — Q・A・F・E・S の Node でできた木 — §1
- **RESEARCH_TYPE** — 研究の始まり方の型 — §4.1
- **review** — run の状態のひとつ — §2
- **ROOT** — SEED から下へ分かれる Q・A・F の総称。分野の可能性の幅 — §4.1
- **`rt_searches`** — research-tree の検索の記録 — §3.4
- **`rt_*` の表** — research-tree の DB の表 — §4.4
- **run（実行）** — エージェントが 1 回動いた記録 — §2
- **`run_id`** — run の番号 — §2
- **runner** — card を動かす実行者 — §2
- **run フォルダ** — run の成果物を集めたフォルダ — §2
- **S** — SPROUT の Node の頭文字 — §4.1
- **sai** — アプリ全体の名前 — §1
- **`sai-engine.db`** — sai-link の DB — §1
- **`SAI_HOME`** — 利用者全体の置き場（既定は `~/.sai`） — §1
- **sai-link** — 基盤（実行・カード・常駐・文書） — §1
- **`SAI_ORIGIN`** — 記録の出どころ — §6
- **sai-paper** — 研究の基盤（論文） — §1
- **`SAI_PARENT_RUN_ID`** — エージェントが動いている run の番号 — §6
- **`sai-progress`** — 時間のかかる命令が標準エラーに出す進み具合の行 — §6
- **`SAI_PROJECT_ROOT`** — 対象のプロジェクトの root — §6
- **`SaiRoutes`** — 画面を登録する場所 — §5
- **sai-sci** — 研究の仕組み（research-tree） — §1
- **`saiTerm`（用語の札）** — 画面の用語に付ける、押すと説明が出る札 — §5
- **sai-web** — Web の窓口 — §1
- **SB** — research-tree の右のサイドバー — §5
- **`schema --json`** — 命令の一覧を出す命令 — §6
- **`sci_type`** — LabAgent の 3 分類 — §4.4
- **`sci-*` ブロック** — 研究作業そのものを行う block — §4.4
- **SEED** — root の根（RESEARCH_TYPE と mind.md）。seed（D）とは別 — §4.1
- **seed（D）** — 1 つの研究の制約。ID は `D<n>`。画面の表示は `SEED（D1）`・`Create SEED` — §4.1
- **STUDY_SEED** — 対象が S のとき `# context` に出る、その S の seed の節 — §4.1
- **settings** — プロジェクトの設定（`context`・`job_status` など） — §2
- **Settings（設定）** — Projects の画面のひとつ（画面の表示は `Settings`） — §5
- **SPROUT** — 1 つの seed（D）と軸ごとの path の組＝1 研究（ID は `S<n>`） — §4.1
- **STAR（★）** — 利用者だけが付ける目印。計算には使わない — §4.1
- **UserOnly（危険度）** — 利用者（Web・`sai-sci apply`）だけが出せる op — §4.3
- **`task.md`** — card と run の依頼書 — §2
- **`# task_text`** — `task.md` の依頼文の節 — §2
- **template** — block の本文に差し込む共通の決まり — §2
- **`textcheck`** — 報告書の検査 — §3.5
- **top_Q** — SEED の直下の Q（`parent_id` が空）。軸の Q の 6 つ — §4.1
- **軸の Q** — task・target・data・method・output・metric の 6 つの top_Q。「〜の種類は何があるか」 — §4.1
- **NO_CHILD・NO_FINDING・UNVERIFIED** — Next Action の why の 3 分類 — §4.4
- **`lab-*` ブロック** — LabAgent の 6 本（`lab-read`・`lab-search`・`lab-grow`・`lab-survey`・`lab-place`・`lab-progress`）— §4.4
- **Usage** — LLM の料金と制限 — §5
- **`sai-sci op`** — agent が research-tree を変える命令（標準入力は JSON・1〜20 件）— §4.3
- **waggle** — 文書にトークンを付けて受け渡す仕組み — §1
- **waggle トークン** — 文書を指す短い名前（例 `AXANSMpo`） — §2
- **Workflow** — Projects の画面のひとつ — §2
- **worktree（作業用コピー）** — block の `worktree:` が決める作業フォルダの分け方 — §2
- **headless** — 端末の窓を使わず、エージェントを子プロセスとして動かす方式 — §5
- **sai-prep** — 各自の Mac の PDF 前処理アプリ — §5

### 五十音

- **一覧** — research-tree の左の、Node の一覧と種類の絞り込み — §5
- **引用（cite）** — 論文の出どころの書き方 — §3.5
- **引用の抜き出し** — PDF から引用文献を取り出す処理 — §3.5
- **解説音声（Radio）** — 論文の解説音声 — §3.2
- **会話検索（セッション検索）** — Projects の画面のひとつ（画面の表示は `会話検索`） — §5
- **書き出し（`state.md`・`tree.md`）** — research-tree を文章と図にした書き出し — §4.4
- **塊（passage）** — 節を空行で分けた本文の単位 — §3.3
- **塊の ID（`#x.y.z`）** — `#<論文>.<章>.<段落>` — §3.3
- **外部 DB** — Library の見方のひとつ — §3.4
- **外部 DB の検索** — Semantic Scholar など外の DB を検索する — §3.4
- **画面の版** — `sw.js` の `CACHE` と `index.html` の `?v=` をそろえて上げる版 — §5
- **検索の範囲（scope）** — 内部 DB の検索で絞る論文の集合 — §3.4
- **検索モード** — 内部 DB の検索の方式 — §3.4
- **find / search** — 内部 DB の検索 / 外部 DB の検索 — §3.4
- **search_history / find_history** — 検索 1 回ごとの記録 — §3.4
- **executer（検索の履歴）** — 検索を実行した `human` か `agent` — §3.4
- **SAI_TARGET_NODE** — Node を対象に動く Agent に入る環境変数 — §3.4
- **子タスク** — 親の run が作る子の run — §2
- **コマンド** — Projects の画面のひとつ（画面の表示は `コマンド`） — §5
- **作成パネル** — SPROUT 作成中だけ出るパネル — §5
- **終了コード** — 0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他 — §6
- **出典（cites）** — F が持つ論文の出どころ — §4.1
- **住所（`#/…`）** — 画面の URL の `#` 以降 — §5
- **常駐（daemon）** — プロジェクトごとの背景のプロセス — §2
- **図（figure）** — PDF から抜き出した図表 — §3.1
- **図の抽出キー** — 図を探す名前 — §3.1
- **節（section）** — 見出しで分けた本文のまとまり — §3.3
- **先着の答え** — 質問は先に答えた方を採用する決まり — §5
- **チャンク（画面）** — 画面と `content chunks` で、塊のことをこう呼ぶ — §3.3
- **チャンク化の戦略** — RAG の断片の切り方 — §3.3
- **登録簿（registry）** — 全プロジェクトの一覧 — §1
- **内部 DB** — Library の見方のひとつ — §3.4
- **内部 DB の検索** — Library の中（論文の本文・ノート・図）を検索する — §3.4
- **プロジェクト（`<root>/.sai/`）** — 作業の単位となるフォルダ — §1
- **翻訳** — 要旨の日本語訳と PDF の翻訳 — §3.2
- **論文の識別子** — 外から付いた論文の名前 — §3.1
- **論文フォルダ** — 論文 1 本の PDF・本文・図・ノートの置き場 — §3.1
- **PDF 待ち** — PDF の無い論文（Library に入れた直後の状態） — §5
- **個人のトークン** — 利用者が Account で発行する `sai_…` の値 — §5
- **前処理の束** — sai-prep が作る PDF・docling・図・翻訳の zip（C12） — §5
- **多人数モード** — 研究室の Linux サーバで動かす形 — §5
- **単独モード** — 1 人が Mac で使う形 — §5
- **擬似 pane（headless）** — headless の run の区画（`hl-`） — §5
- **内部の合言葉** — コンテナの CLI・フックが Web を呼ぶ値（`SAI_WEB_INTERNAL_TOKEN`） — §5
