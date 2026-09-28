# Pi Chat History Export

**Exported:** 2026-09-27 04:43:31
**Session:** /Users/touta/.pi/agent/sessions/--Users-touta-workdir-Sai-sai-paper--/2026-09-27T03-30-04-469Z_01a0e0e9-a835-7d7f-9887-3c67dd37da68.jsonl

---

## 1. sai-radio の起動方法

sai-radio は「画面の無い裏方サーバー」です（`/Users/touta/workdir/Sai/sai-radio/README.md` 冒頭）。起動は `sai-paper` とは別リポジトリで行います。

**本体サーバー（常時起動が必要）**
```bash
cd /Users/touta/workdir/Sai/sai-radio
uv run uvicorn sai_radio.main:app --app-dir src --host 127.0.0.1 --port 8000
```
根拠: `sai-radio/ARCHITECTURE.md:43-47`（「sai-radio は常時起動する必要があります（決定事項#11）… 起動コマンドは次のこと（`127.0.0.1` 固定）」）、`README.md` の「つまずきやすいポイント」も同じコマンド。ホストは `127.0.0.1` 固定（iPhone からは sai-phone が中継するだけです）。

**起動前任務（README「起動しておく必要があるもの」）**
| 起動物 | ポート | 備考 |
|---|---|---|
| AivisSpeech Engine | 10101 | sai-paper は自動起動しない。手で起動（`sai-paper/README.md:289`） |
| sai-radio 本体 | 8000 | `r` キーの前提 |
| sai-phone | 8100 |  dependant 側の画面 |
| TTS プロキシ（ポッドキャストモード時のみ） | 8600 | `uv run uvicorn sai_radio.tts_proxy:app --port 8600`（`ARCHITECTURE.md:329`） |

**sai-paper 側の設定**（本プロジェクトの実値 `.sai/paper/settings.json` → `radio`）
```json
{ "server_url": "http://127.0.0.1:8000", "token_env": "SAI_RADIO_TOKEN", "timeout_sec": 1800 }
```
既定値は `crates/sai-paper-core/src/config.rs` の `impl Default for RadioConfig`。sai-paper からは `POST {server_url}/api/papers/from-extract`（`podcast_client.rs:110`）と `GET {server_url}/api/logs`（`podcast_client.rs:432`）を叩きます。接続できないときは「sai-radio（…）に接続できません。sai-radio を起動してください」（`podcast_client.rs:314`）。

**今の状態（実測）**: `lsof -i :8000` → `sai-radio/.venv/bin/uvicorn sai_radio.main:app --host 127.0.0.1 --port 8000` が起動中（PID 93143）。10101 も稼働中、**8600（TTS プロキシ）は未起動**＝ポッドキャスト（2人対話）モードは今は使えません。

---

## 2. `.sai/blocks` の使用手順

`.sai/blocks/*.md` は **sai-link のブロック定義**（エージェントの指示プロンプト＋実行設定を 1 ファイルにしたもの）です（`sai-link/README.md:1002`）。

1. **置くだけで認識される**: `.sai/blocks/` に置く（ファイル名がブロック ID の推奨値）。同じ名前が `~/.sai/blocks/` にもあればプロジェクト側が優先（`sai-link/README.md:83-84, 382`）。
2. **書式**: Markdown + YAML frontmatter（`id` / `name` / `description` / `model` / `thinking` / `templates` / `subblocks` / `timeout` / `worktree` / `enable` …）。**入力は `task_text` だけ**（`inputs`/`output` は廃止）。本文には `{{input.task_text}}` を書く（`sai-link/README.md:382-430`）。
   - **`subblocks` 未指定だとサブタスクを一切呼べません**（`sai-link/README.md:420`）。本Pjでは `.sai/blocks/kantoku.md:11-13` が `worker` / `sub` を許可しています。
3. **デーモンが必要**: `sai-link daemon start`（未起動なら要求は `pending/` に保留）（`sai-link/README.md` 3.1）。
4. **実行は kanban 経由**（定義ファイル直指定は廃止）:
   ```bash
   sai-link block run --kanban                     # カード選択 → ブロック選択 → 実行
   sai-link block run --kanban --block <ID> --round 1 --global-round 1 --yes
   sai-link block preview .sai/blocks/<ID>.md --round 1 --global-round 1   # 起動コマンドの Viewingのみ
   ```
   オプション表は `sai-link/README.md:176-200`。デーモンが herdr pane を割って `pi` を起動し、子は `task_report`、メインは `sai_finalize` で完了します。
5. **-definition の確認**: `sai-link blockdef list` / `blockdef show <名前>`（`--parent <親>` で `subblocks` 制限を反映）。エージェント側からは MCP `block_discover` / `block_inspect`。
6. **TUI**: `sai-link-tui`。`Shift+B` で `nvim .sai/blocks/`、`Shift+C` でカード作成（`sai-link/README.md:722`）。
7. **コミット推奨**（チーム共有のため）: `sai-link/README.md:1097`。

**本プロジェクトの現状**: `.sai/blocks/` には `main / sub / worker / kantoku / plan / plan-kantoku` の 6 つ。研究用の `paper-*.md`（16 個）は `.sai/blocks_/` に退避されており、`.sai/blocks/` には入っていません。`.sai/blocks_/` には `paper-discuss / paper-library-qa / paper-screen / paper-search` のみ。paper-* は CLI に同梱の `crates/sai-paper-cli/assets/blocks/paper-*.md` として入り、`sai-paper init` が配置します。

---

## 3. `sai-paper init` が作るもの

実装は `crates/sai-paper-cli/src/main.rs:712-830`（`handle_init`）。引数は `--name` / `--root`（既定 `SAI_PAPER_ROOT` または cwd）/ `--assets-only` / `--domain medical|signal-ml|general`（`main.rs:117-129`）。

**通常 init（1 度だけ。`papr.db` が既にあるとエラーで停止）**
| 作るもの | 根拠 |
|---|---|
| `.sai/` が無ければ `sai-link init` をサブプロセス実行 | `main.rs:756-770` |
| `.sai/paper/{papers,extracts,bibtex,plugins}` ディレクトリ | `main.rs:771-773` |
| `.sai/paper/papr.db`（マイグレーション込み） | `main.rs:776-777` |
| `.sai/paper/settings.json`（`Config::load_or_create`） | `main.rs:778` |
| `sai-control registry add`（任意・失敗しても init は成功） | `main.rs:780-802` |
| `.sai/blocks/paper-*.md` **16 個** | `agent_assets.rs:17-34` |
| `.sai/template/*.md` **9 個**（debate-participant, literature-hard-rules, paper-screening, paper-search-strategy, rag-deep-search, rag-simple, research-core, search-external, search-internal） | `agent_assets.rs:37-46` |
| `.sai/template/project-brief.md` のプレースホルダ | `agent_assets.rs:75-84` |
| `.sai/template/domain-{medical,signal-ml,general}.md` + 選んだ内容の `domain.md` | `agent_assets.rs:52-67, 258-283` |
| `.sai/settings.json` への `extensions`（pi 拡張の絶対パス探索）＋ 無ければ `pre_run = [sai-paper, brief, --write]` | `agent_assets.rs:186-215` |
| ルート `.mcp.json` の `mcpServers.sai-paper` = `{command: <sai-paper>, args:["mcp"], env:{SAI_PAPER_ROOT}}`（既にある項目は消さない） | `agent_assets.rs:224-241` |
| `.sai/paper/mind.md` / `state.md` のひな形、`.sai/template/project-brief.md`（実データ） | `main.rs:676-699` |
| 最後に起動案内（`sai-paper-tui` / `sai-link-tui`） | `main.rs:827-829` |

**`--assets-only`**: `.sai/` が既にあり、上から блок/テンプレート/設定/domain/mind・state/brief だけを配る。`papr.db`・`sai-link init`・`sai-control`・既存ファイルには触れない（`main.rs:700-711, 718-748`）。全ファイルは `write_if_missing` なので**既存の編集内容は消えません**（`agent_assets.rs:130-140`、テスト `install_writes_assets_and_keeps_existing_entries` が保証）。

**本プロジェクトの現状**: `.sai/settings.json` は `{}`（拡張・`pre_run` 未登録）、`.mcp.json` に `sai-paper` エントリ無し、`.sai/blocks/` に `paper-*` 無し。**つまりこのプロジェクトでは init による pi-litbase / MCP の配entingがまだ走っていない**（だから今のセッションにも `sai_paper_find` 等のツールが見えていません）。`sai-paper init --assets-only` で入ります。

---

## 4. エージェントが行うベクトル（ハイブリッド）検索の引数

Agent が使う入口は 3 つあり、いずれも中核は `sai_paper_core::retrieval::find`（`retrieval.rs:552`）です。

### (a) pi 用ツール `sai_paper_find`（pi-litbase）— `pi-litbase/tools.ts:305-336`
中では `sai-paper find ... --json` を実行（ラッパ `pi-litbase/sai-paper-cli.ts:60-95`、`cwd` はプロジェクトルート・既定タイムアウト 120 秒）。

| 引数 | 型 | 意味 |
|---|---|---|
| `query` | string（**必須**） | 検索クエリ（自然言語） |
| `mode` | string | `word` / `vector` / `hybrid`（既定は config） |
| `top_k` | number | 上位 K 件（既定は config） |
| `scope` | string | `papers` / `notes` / `all`（既定 `papers`） |
| `paper` | number | 論文 ID（その論文に絞る） |
| `index` | string | 索引名（既定 `settings.json` の `rag.default_index`。埋め込みモデルも索引で決まる） |
| `alpha` | number | ハイブリッド時の vector 側重み 0.0–1.0 |
| `fusion` | string | `weighted` / `rrf` |

補足ツール: `sai_paper_neighbors`（`paper_id` 必須、`top_k` 既定 20、`index`）と `sai_paper_index_status`（引数なし）。

### (b) cc / pi 用の MCP サーバ `sai-paper mcp` — `crates/sai-paper-cli/src/mcp.rs:19-47`
道具は `find` / `neighbors` / `index_status` / `paper` / `search_sets`。

`find` のスキーマ: `query`（必須）、`mode`（enum word/vector/hybrid）、`top_k`、`per_paper`、`scope`（enum **papers/notes/figures/all**）、`paper_id`、`index`、`evidence`(bool)。実装は `mcp.rs:93-133`。
`neighbors`: `paper_id`（必須）、`top_k`、`index`。

### (c) CLI `sai-paper find` — `main.rs:168-201`
`query` に加えて `--mode / --top-k / --per-paper / --scope / --paper / --index / --alpha / --fusion / --fts-expr / --evidence / --json`。

### 実効値（省略時の既定）
`crates/sai-paper-core/src/config.rs:525-546` の `impl Default for RagConfig` と、本プロジェクトの `.sai/paper/settings.json` の実測：
- `default_mode` = `hybrid`、`top_k` = **20**、`per_paper` = **2**、`alpha` = **0.7**、`fusion` = `weighted`、`chunk_strategy` = `paragraph`、`chunk_max_tokens` = 450、`embed_model` = `liquid/lfm-2.5-embedding-350m:free`（OpenRouter、dim 1024）、`default_index` = `default`（settings.json に無いため既定）。
- `alpha` の意味は `score = alpha * vector + (1 - alpha) * word`（`retrieval.rs:433-457`）。※`config.rs:421` の doc コメントは「BM25 のウェイト」と書いていますが、実装は **vector 側の重み** です（コメントと実装が食い違っています）。
- 内部では `top_k × 3`（最大 200）件まで fetched し、`paper_id` 指定や `per_paper = 0` のときだけ 1 論文あたりの制限を解除します（`retrieval.rs:352-353, 555-561`）。

### エージェントに指示されている運用（テンプレートの記述）
`.sai/template/search-internal.md`（paper-* ブロックの `templates` head に必ず入る）:
- 先に `sai-paper index status` で確認。**索引が無くても agent は作らない**（ユーザー頼み）。
- 日本語の問いを英語の検索語 3〜5 個に分解 → 各々 `sai-paper find "<英語>" --mode hybrid --top-k 10 --scope all --json`。
- `--evidence` は費用なので 1 作業 3 回まで。
- `--mode word` は日本語に効かない（FTS5 のトークナイザ制約）。

現状の実測: `sai-paper index status` → `default` 索引に chunks 2060 / embeddings 2060（abstract 115, body 1932, note 13）。つまりベクトル検索はそのまま実行可能です。

## 出典（同定）

| もの | 場所 |
|---|---|
| 実装ガイド（テーマD: 分野の研究役 DSR） | `.sai/outputs/R9/48/guide-D-dsr.md` |
| 実装計画書 D01〜D08 | `.sai/outputs/R9/48/unit-D01-…〜unit-D08-….md` |
| 実行（kantoku run_id=50） | `.sai/outputs/R9/50/task.md`, `task-D01.md`〜`task-D08.md`, `completion-D-dsr.md` |
| 元の素材 | `lib/paper_skills/topic_discuss_skill/`（`Evidence/` 19個 + `Protcol/` 5個、各 `SKILL_sai.md` 等）と整理メモ `memo.md` |
| 実装された実体 | `crates/sai-paper-cli/assets/blocks/paper-*.md`（17個）/ `assets/template/*.md`（13個）/ `src/agent_assets.rs`（`BLOCKS`・`TEMPLATES`） |

run 50 の概要: `task.md` の `description` は「実装ガイド D: 分野の研究役 DSR（D01〜D08）」、`block: kantoku`、`round: 4 / global_round: 9`。完了報告書は「研究プロジェクトには 12 個の研究用ブロック（DSR）が配られる」と記しています。

---

## 1. 元の SKILL を何に変えたのか（memo.md → DSR）

`lib/paper_skills/topic_discuss_skill/memo.md` は 24 個の SKILL を整理したユーザー本人のメモで、方針が明瞭に書かれています。

**メモの診断**（`memo.md:3-12`）
- 「reference は、ワークフローの各ステップに挿入される／ workflows と出力フォーマットがほぼ同じ内容」
- 「禁止ルールは大体同じ → ほぼ不要」
- 「すべてで検索は必要。検索を分離」
- 「各 DSR には**評価の観点**と**出力の形**だけを書く。どのように情報を取得するかは LLM に任せる」

**メモ的使用順序**（`memo.md:92-118` の「使用流れ」）
1. ブレスト＝`clinical-question-clarifier`（「検索はしない／回答をしない／漠然とした疑問を検索可能なクエリに変換する」）
2. 検索＝`biomedical-search-strategy-builder` / `multi-database-literature-collector`
3. 検索＆トピック要約＝`topic-evidence-mapper` など
4. 単体論文調査＝`medical-research-literature-reader-pro` ほか
5. 複数論文＝`high-value-paper-screener` / `evidence-level-ranker` / `contradictory-findings-resolver`

**変換の原則**（`guide-D-dsr.md`「決定事項と理由」）
- 「DSR は **分野によらない共通の型＋差し替えられる分野の観点**（`domain.md`）で作る（ユーザーの決定）。**採用しなかった案: 医学向けの SKILL（`lib/paper_skills/`）をそのまま入れる**」
- 「エージェントへの決まりは**ユーザーが直せる md のテンプレート（`.sai/template/`）**に置き、Rust の中に決まった文として持たない」
- 「索引や埋め込みはエージェントが作らない（ユーザー決定）」
- 「既存の `.sai/blocks/` は init が上書きしない」

つまり **SKILL 1 本 → ブロック 1 本の写し焼きではなく、「評価の観点」と「出力の形」だけを採って、共通部分（作法・検索）をテンプレートへescape** しています。この原則は `unit-D01` / `D03`〜`D06` の「背景」に同じ文言で 들어가、`agent_assets.rs` のテスト `research_blocks_declare_brief_and_review_headings`（`agent_assets.rs:329-360`）が機械的に検査しています（本文 4 節・`templates:` の並び・`review_headings`・`/Users/` を含まない）。

**素材 SKILL → ブロックの対応**（各計画書「材料」節の明記）

| 新ブロック | 主軸 SKILL | 追加で採った観点 |
|---|---|---|
| `paper-clarify` 問い整理役 | `Evidence/clinical-question-clarifier/` | `review_skill/paper_support/scientific-problem-selection/`、`Protcol/study-objective-refiner/` |
| `paper-map` 分野地図役 | `Evidence/topic-evidence-mapper/` | `medical-topic-saturation-and-whitespace-checker/`（飽和と手薄さ）、`disease-mechanism-evidence-map`・`drug-target-evidence-landscape`（証拠地図の観点） |
| `paper-read` 精読役 | `Evidence/medical-research-literature-reader-pro/` | `figure-first-paper-reader/`（図と本文の整合）、`methods-reverse-engineer/` |
| `paper-critic` 批判役 | `review_skill/paper_support/scientific-critical-thinking/` | `result-reliability-checker`・`paper-to-claim-verifier`・`contradictory-findings-resolver` |
| `paper-gap` 空白発見役 | `Evidence/medical-research-gap-finder/` | `method-gap-detector`・`population-gap-detector`（メモが「単体ではなく評価項目」と決めた 3 つを 1 ブロックに統合） |
| `paper-plan` 計画化役 | `Protcol/aim-and-hypothesis-designer`・`primary-plan-recommender`・`validation-strategy-designer` | `novelty-vs-feasibility-assessor`・`medical-research-algorithm-matcher` |
| `paper-template-maker` 説明書づくり役 | （新規。/unit-D07） | — |

既存 4 ブロック（`paper-search` 文献収集役 / `paper-screen` 選別役 / `paper-discuss` / `paper-library-qa`）は memo の 2〜5 段（検索・選別・複数論文・質問）に対応し、D03 で同じ型へ作り直されました。

---

## 2. `.sai/blocks/*.md` の使用想定手順

### 2.1 置かれる場所と、置かれない場所
`sai-paper init` は `crates/sai-paper-cli/assets/blocks/` の内容を `.sai/blocks/` へ**コピ packed するだけ**で、処理は `agent_assets::install`（`agent_assets.rs:144-159`）の `write_if_missing` です。**一度 init すると二度と更新されません**。 Users' run 50 の残課題にも「古い 4 ブロックを新しい型にしたいときは、そのフォルダの `.sai/blocks/paper-search.md` などを消してから `--assets-only`」とあります（`completion-D-dsr.md`「ユーザーが試す手順」1）。

### 2.2 1 つのブロックの読み方（4 節固定）
`paper-read.md` を例に取ると、本文は次の 4 節だけです。

```
## 依頼        ← {{input.task_text}}（依頼文がそのまま入る）
## 役割        ← 何をどの段階でやるか
## 評価の観点  ← 「評価の観点」＝ memo が残した SKILL のチェック項目
## 出力の形    ← ```markdown コード欄里的 ## 見出し（= report の雛形）
## 引き継ぎ    ← research-core の 6 項目
```
共通annya `templates:` の順で本文の前に挿入されます（`sai-link/crates/sai-link-engine/src/parser.rs:159 apply_templates`）。

### 2.3 実行手順（ユーザー側）
run 50 の「ユーザーが試す手順」と README 5.4 / `guide-D-dsr.md`「統合と検証」3 に沿着します。

```bash
# 0) 研究は本リポジトリではなく別フォルダで作る
sai-paper init --root ~/research/<名前> --domain signal-ml     # 初回
sai-paper init --root ~/research/<名前> --assets-only --domain medical  # 追加入替

# 1) 分野観点を直す（任意）
$EDITOR ~/research/<名前>/.sai/template/domain.md

# 2) 索引はユーザーが手で（エージェントは作らない）
sai-paper index build && sai-paper index embed

# 3) sai-link のカードを作ってブロックを選ぶ
sai-link block run --kanban --block paper-clarify --round 1 --global-round 1
#   依頼（カード本文の # task_text）に書く:
#     「てんかんの脳波で発作を予測する研究を調べたい　確認: 多め」

# 4) 直前の report の要点を次の依頼文に写して、段階を進める
# 5) 出力の形を機械で確かめる
sai-paper dsr check <report の md パス> --block paper-critic   # 0/1/2
```

**依頼文に書ける指定**（README 5.4、実体は `research-core.md` と各ブロック本文）
- `確認: 少なめ` / `確認: 多め` — 既定は 2 か所（①問いが固まったとき ②結論を書く前）。少なめ=②のみ、多め=①・②＋③検索語を見せる前
- `深さ: 概要|標準|詳細` — 精読役のみ（`paper-read.md`「役割」節）
- `2018-` のような年の範囲 — 文献収集役のみ

**成果物の置き方**（`research-core.md`「出力」節、`.unit-D01` 共有契約）
- report は 1 回の実行で **1 つだけ**、`mcp__sai__sai_write` で `.sai/outputs/<owner>/<run_id>/` に保存
- `filename` = `<ブロックID>-<短い英字>.md`（例 `paper-map-eeg-datasets.md`）、`description` = `<ブロック名>: <内容1文>`
- report 冒頭に frontmatter（`kind` / `block` / `papers: [37, 40]`）— sai-paper の一覧が読む
- 末尾に `## 引き継ぎ` 6 項目（**名前と順番は変更禁止**、sai-paper が機械で読む）

### 2.4 典型フロー（README 5.4 の表）
```
探索期     paper-clarify → paper-search → paper-screen
探索期の終 paper-map（地図の Canvas も下書き作成）
精読・議論  paper-read（深さ指定）→ paper-critic / paper-discuss
絞り込み   paper-gap
計画化     paper-plan（最後に「引き渡し書」）
いつでも   paper-library-qa / paper-brainstorm / paper-template-maker / paper-debate
```
`paper-map` は report に加えて `sai-paper canvas new … --draft` の下書き Canvas を作ります（エージェントはユーザーの Canvas を上書きしません）。`paper-template-maker` はテンプレートの**下書き**を report として出し、`.sai/template/` へ置くかどうかはユーザーが決めます（`.unit-D07` 決定事項）。`paper-debate` は司会で、`paper-debater` / `paper-reviewer` は子としてしか呼ばれません（`agent_assets.rs` のテスト `debate_participants_declare_neither_brief_nor_core` が、2 つは `project-brief` を持たないと保証）。

---

## 3. `.sai/template/*.md` の使用想定手順

### 3.1 3 層の差し込み
`templates:` の並びが「型」です（`guide-D-dsr.md` 共有契約）:

```
project-brief   ← 実行直前に pre_run（sai-paper brief --write）が作り直す動的 Situationまとめ
research-core   ← 共通作法（research-core.md, 63行）
search-internal ← Library 検索の手順（search-internal.md, 99行）
search-external ← 外部DB 検索の手順（search-external.md, 97行。外部検索する block だけ）
literature-hard-rules ← 出典の書き方
domain          ← 分野の観点（domain.md）
```
先頭の `project-brief` は毎回 overwritten されます（だから `paper-debater` / `paper-reviewer` からは意図的に外してあります）。`paper-template-maker` は検索しないので `domain` -layered な `search-internal` / `search-external` を受け取りません。

### 3.2 各テンプレートの役割
| ファイル | 中身 | 誰の_cityだ |
|---|---|---|
| `research-core.md` | 依頼の言い直し→確認（既定2か所）→検索→根拠 `[#37]` →迎合しない→出力（sai_write 1つ）→引き継ぎ6項目 | 全研究用 block |
| `search-internal.md` | `index status`→英文検索語3-5個→`find --mode hybrid --top-k 10 --scope all --json`→統合→`neighbors`→`[GAP:]`。索引が無いときは **agent が作らずユーザーに頼む** | `search-internal` を持つ block |
| `search-external.md` | DB 選択（`domain` に従う）→英文検索語（同義/上位/除外）→`sai-paper search --save-as` →重複→引用網 1 段 | `search-external` を持つ block（paper-search など） |
| `literature-hard-rules.md` | 出典の書き方・捏造禁止 | 全 block |
| `domain.md` | 問いを整理する枠／証拠の強さ／落とし穴 の3節 | 全 block（`paper-template-maker` 以外） |
| `paper-screening.md` | keep/drop の基準 | `paper-screen` |
| `debate-participant.md` | 議論参加者用の共通指示 | `paper-debater` / `paper-reviewer` |
| `project-brief.md` | 実行直前に生成（プレースホルダは init が置く） | 全 block |
| `paper-search-strategy.md` / `rag-simple.md` / `rag-deep-search.md` | **旧版**。先頭に `> このテンプレートは search-external / search-internal に移りました…` の1行だけ足して互換残置（D01 決定） | 誰からも使われない |

### 3.3 ユーザーが編集する位置づけ
- `research-core.md` の冒頭は「sai-paper が研究プロジェクトに配った**最初の版**です。**ユーザーが自由に直せます**」と明記
- `domain-*.md` の先頭は「**自由に直してください**」
- **エージェントは `.sai/template/` に直接書かない**（`.unit-D07` 決定:「ユーザーが主導する」）。下書きは `paper-template-maker` が report として出すだけ
- テンプレートの总数目安は 20 まで（`.unit-D07`）
- `--assets-only` は既存ファイルを上書きしない（`write_if_missing`）。`domain.md` があるときは案内だけ出して触らない（`agent_assets.rs:266-276`）

---

## 4. 検証された範囲（run 50 の記録より）

完了報告書の「全体の完了条件」:
- 一時プロジェクトで `init --root $T --domain signal-ml` → **12 ブロック・13 テンプレート**が配置、`domain.md` の先頭は「分野の観点: 信号処理・機械学習（signal-ml）」
- 12 ブロックすべて `sai-link block preview` がテンプレートのエラー無しで通過
- `dsr check` は見出し不足で exit=1、block 不在で exit=2
- `cargo test --workspace` 430 件通過

**計画との違い（報告より）**
- YAML で `: ` が壊れるため description の「（推奨: モデル）」を**全角コロン**「（推奨：…）」へ変更
- 「出力の形」は `## ` の行を ```markdown コード欄に並べる形に統一（`dsr check` が箇条書きを読まないため）
- 計画どおり**使われなくなった 3 テンプレートも配り続ける**（D01 の「互換のため残す」決定）

**残った申し送り（現状も未解決）**
- README 5.4 の「名前」列（`📚 Library 質問応答` 等の frontmatter 名）と表の表記のずれ → 要ユーザー決定
- `--assets-only` を `--domain` 無しで実行すると `general` 扱いになり `domain.md は既にあります` の案内が出る
- `init` が `.sai/settings.json` の `extensions` に `pi-litbase/index.ts` を書く一方、sai-link は「pi は MCP で道具を使う」として無視し注意を出す（`agent_assets::pi_extensions` の整理は未着手）
- 作業ブランチ `feat/dsr` は **main に未 merge**

---

## 5. このリポジトリの現状（注意点）

本 `sai-paper` リポジトリは**開発リポジトリ**であり、研究プロジェクトではありません。

- `.sai/blocks/` にあるのは `main / sub / worker / kantoku / plan / plan-kantoku` の 6 つで、**DSR ではない**（実装は `stealth/space-bunny-alpha`、`templates: sai-base,sai-user,sai-cli`）
- 研究用 `paper-*.md` は `.sai/blocks_/` に 4 つ（`paper-discuss` / `paper-library-qa` / `paper-screen` / `paper-search`）退避されており、`sai-paper init --assets-only` を走らせても `.sai/blocks/` には入りません（別フォルダが正）
- 配布元は `crates/sai-paper-cli/assets/blocks/` と `assets/template/` に `include_str!` としてコンパイルAndreas込まれています（`agent_assets.rs:17-67`）
- 実際に DSR を使うには、run 50 の完了報告どおり **別フォルダ**（例 `~/research/<名前>`）で `sai-paper init` を実行します

---

## 2. 3 つの見本の中身の違い

形は完全に同一で、`unit-D02-domain-templates.md` の指定どおり **3 節だけ**（`## 問いを整理するときの枠` / `## 証拠の強さの見方` / `## よくある落とし穴`）。テスト `install_domain_writes_samples_and_selected_domain`（`agent_assets.rs:363-410`）が「先頭が `<!--` のコメント」「3 節が全部ある」「`\n## ` が 3 個」を保証しています。中身は全面的に書き替わります。

| 節 | `general`（既定, 26行） | `medical`（33行） | `signal-ml`（32行） |
|---|---|---|---|
| 問いを整理するときの枠 | 問い / 対象 / 方法 / 評価 / 限界 の 5 項目 | 問いの種類を 1 つに決め、**PICO / PECO / PICOTS**、診断精度、予後の枠 | **データセット**（被験者数・記録条件・ラベルの付け方）/ 前処理 / 特徴量・モデル / **評価の分け方**（被験者をまたぐか、時間で分けるか）/ 指標（balanced accuracy, F1, AUC）/ ベースライン |
| 証拠の強さの見方 | 一次資料か二次か / 再現・追試 / 査読かプレプリント | **研究デザインの階層**（メタアナリシス > RCT > コホート > 症例対照 > 症例報告 > 基礎研究）＋実行の質・登録確認 | 公開データで再現可能か / **外部データでの検証** / 検証の深さ 5 段階 / 比較の公平性 / テストデータでの HP 調整 / 複数シード |
| よくある落とし穴 | 1 本への依存、新しさの過大評価、別分野の一般化 | 代理エンドポイント、**選択バイアス**、多重比較、関連を因果と書く、利益相反 | **被験者内分割の情報漏れ**、前処理リーク、クラス不均衡と比較不能、SOTA 数値だけ見る |

`medical` の内容は `lib/paper_skills/topic_discuss_skill/Evidence/clinical-question-clarifier/SKILL_sai.md`（PICO/PECO）と `evidence-level-ranker/SKILL_sai.md`（デザインの階層）から、`unit-D02`「材料」節のとおり抜き出されたものです。`signal-ml` は**素材に無いので最小の版**で置かれ、使ううちにユーザーが直す前提（`unit-D02` 決定事項「`signal-ml` は最小の版にする — 理由: 材料の SKILL に無い」）。

---

## 3. その変化が実際に効く仕組み

`domain` は sai-link の**テンプレート名**です。ブロックの frontmatter で `templates:` の最後に入れてあり、sai-link が `apply_templates`（`sai-link/crates/sai-link-engine/src/parser.rs:159-205`）で `.sai/template/domain.md` を読んで**本文の前に差し込む**だけです。名前解決は「末尾に `.md` を付ける」だけなので、`domain.md` の中身を直接手で書き換えてもそのまま反映されます。

実際に `domain` を受け取っているブロック（実測 17 個中 **16 個**）:
```
paper-brainstorm / paper-check / paper-clarify / paper-critic / paper-debate /
paper-debater / paper-discuss / paper-gap / paper-library-qa / paper-map /
paper-next / paper-plan / paper-read / paper-reviewer / paper-screen / paper-search
```
**唯一の例外は `paper-template-maker`**（`templates:` は `project-brief, research-core, literature-hard-rules` のみ）。検索をしないので「問いを整理する枠」も「証拠の枠」も要らない、という設計です。テスト `research_blocks_declare_brief_and_domain`（`agent_assets.rs:413-440`）がこの 1 件の例外を明示的に許しています。

本文で `domain` を名指ししているのは 2 ブロックだけですが、意味は明確です:
- `paper-clarify.md:40` 「問いの枠の選び方や使う言葉は `domain` に従っているか」／出力の見出し `## 選んだ枠（domain の枠のどれを使ったか）`
- `paper-map.md:64` 「枠の選び方や使う言葉は `domain` に従っているか」

つまり `--domain medical` にすると `paper-clarify` が PICO を選び、`--domain signal-ml` にすると「データセット・評価の分け方・指標」を選ぶ、という**成果物のQualifierが変わる**。他の 14 ブロックは `templates:` 経由でこの影響を受けます（特に `paper-critic` の証拠評価、`paper-gap` の対象集団、`paper-plan` の計画）。

---

## 4. 使いCorminal手順と注意

```bash
# 初回：分野を決めて作る
sai-paper init --root ~/research/epilepsy --domain signal-ml
# → 3 つの見本が置かれ、domain.md に signal-ml の中身が入る

# 途中から分野を変える（既存のプロジェクト・既存のファイルは保持）
rm ~/research/epilepsy/.sai/template/domain.md
sai-paper init --root ~/research/epilepsy --assets-only --domain medical
# または cp ~/research/epilepsy/.sai/template/domain-signal-ml.md domain.md
# 内容を部分的に直すだけなら init 不要（domain.md を直接編集）
```

注意点（コードと run 50 の記録より）:
- **`--assets-only` を `--domain` 無しで実行すると `general` 扱い**になり、毎回「`domain.md` は既にあります…」の案内が出る（run 50 `completion-D-dsr.md`「残った課題」:「害は無いが、うるさく感じるなら案内を出す条件を変える」と申し送りあり）
- `domain.md` は**ボタン一発で戻らない**。消さない限り init は触りません
- `dsr check`（`sai-paper-core/src/dsr_check.rs`）はブロック本文の `## 出力の形` だけを読むので、`--domain` を変えても判定は変わりません。変わるのは中身です
- `--domain` は `sai-paper init` にしか無い引数です。TUI にも config (`settings.json`) にも項目はありません（`grep domain crates/*/src` でヒットするのは `main.rs` と `agent_assets.rs` のみ）
- 本 `sai-paper` 開発リポジトリの `.sai/template/` には `domain-*.md` / `domain.md` が**ありません**（`agent_assets.rs` の `DOMAIN_SAMPLES` は CLI に同梱されているが、このプロジェクトでは init を走らせていない）。研究は別フォルダで行う前提です

---

## 👤 User (7)
> 2026-09-27T04:00:12.619Z

エージェントが，ウェブ論文データベースから論文を検索する手順は？

---

## 🤖 Assistant (8)
> 2026-09-27T04:01:01.698Z

# エージェントがウェブの論文 DB を検索する手順

手順の正本は `.sai/template/search-external.md`（`sai-paper init` が配る共通テンプレートの 1 本）です。`templates:` に `search-external` を含むブロック — 現状では **`paper-search` だけ**（`crates/sai-paper-cli/assets/blocks/paper-search.md:7-13` が `project-brief, research-core, search-internal, search-external, literature-hard-rules, domain`）— がこの手順を受け取ります。`search-internal`（Library 内検索）を先にAKER 完之后「足りなければ」外部へ、という順序が `research-core.md`「## 検索」で定められています。

---

## 6 ステップ（`search-external.md` の見出しそのまま）

### ステップ 1: DB を選ぶ
5 つの DB。**どの観点を重視するかは `domain.md` に従う**とテンプレートが明記しています（`--domain medical` なら PICO/研究デザイン、`signal-ml` なら公開データ・比較の公平性）。

| `--db` の値 | 実装 | 対象 |
|---|---|---|
| `pubmed` | `api/pubmed.rs`（NCBI E-utilities） | 医学・生物医学の査読論文、MeSH / field tags |
| `biorxiv` / `medrxiv` | `api/biorxiv.rs`（`PreprintServer::Biorxiv/Medrxiv`） | 生命科学のプレプリント |
| `s2` | `api/s2.rs`（Semantic Scholar Graph API） | 全分野・引用ネットワーク |
| `arxiv` | `api/arxiv.rs` | CS・物理・数学、方法論 |
| `openalex` | `api/openalex.rs` | 全分野の journal メタデータ・ジャーナル指標 |

テンプレートの目安: 医学は `pubmed` + `biorxiv/medrxiv`、工学・CS は `arxiv` + `s2`、広く探すときは 3 つ。**1 つの DB だけに偏らないこと**が `paper-search` の評価の観点（`paper-search.md`「評価の観点」）にιασύです。

### ステップ 2: 英語の検索語（3〜5 個）を作る
日本語の問いを英語に直し、各検索語に **同義語 / 上位語 / 除外語** を用意します。DB ごとに書き方が違います。
- **PubMed**: `--raw-query` で Boolean と field tags（`[tiab]` タイトル・抄録、`[mh]` MeSH、`[au]` 著者、`[pt]` 出版種別、`[dp]` 出版日 `YYYY:YYYY`）。例 `(glioma[tiab] OR glioblastoma[mh]) AND ferroptosis[tiab]`
- **bioRxiv/medRxiv**: キーワードを AND でつなぐ
- **S2 / arXiv**: 自然言語

実装側の受け口は `api/mod.rs:22-38` の `SearchQuery { text, raw_query, year_from, year_to, limit, offset }`（`raw_query` は `text` を上書きする実装もあります）。

### ステップ 3: 検索して**必ず検索セットに保存する**
```bash
sai-paper search "<自然文の質問>" \
  --db pubmed --db biorxiv --db s2 \
  --limit 100 --year-from 2018 \
  --raw-query '(glioma[tiab] OR glioblastoma[mh]) AND ferroptosis[tiab]' \
  --save-as glioma-ferroptosis-2024 \
  --json
```
実引数（`main.rs` の `Cmd::Search`）:
- `query`（必須）、`--db`（**複数可・繰り返し指定**）、`--limit`（**既定 10**）、`--year-from` / `--year-to`、`--raw-query`、`--save-as`、`--json`
- `--db` を省略すると `settings.json` の `discover.providers`（既定 `["s2"]`。本プロジェクトは `["arxiv","s2"]`）

**`--save-as` は必須**（「あとで絞り込むときに使います」）。同じ名前で何度 `--save-as` しても 1 つのセットに集まり、`dedupe_key` で重複が入りません（`main.rs:864-895` `save_search_results`）。保存後の確認は `sai-paper set show <セット名>`。

実処理は `api/mod.rs:182-221 run_search`:
1. `PaperSourceResolver::resolve(db_id)` で各 DB のクライアントを作る
2. **1 つでも DB が失敗しても止まらない**。失敗した DB と理由を `(db_id, 理由)` で返して合成
3. `dedupe()` で重複除去
4. `citations::fill_citation_counts` — arXiv のように被引用数を返さない DB の分だけ OpenAlex で補う

重複キーの優先順（`api/mod.rs:129-149`）: **DOI → S2 paperId（40桁hex）→ 正規化タイトル**。arXiv ID への言及は「外部 ID 抽出は別途実装」とコメントされ、**現状は未実装**（テンプレートの「DOI → arXiv ID → S2 paperId → 正規化タイトル」という記述と実コードにはずれがあります）。

### ステップ 4: 重複は自動（手作業不要）
`sai-paper search` が DB 間で自動で除きます。複数回に分けても同じ `--save-as` にまとめられます。

### ステップ 5: 引用の網を**1 段だけ**広げる
```bash
sai-paper expand 37 88 120 --save-as glioma-ferroptosis-2024 --direction both --limit 20
```
`Cmd::Expand` の実引数: `papers: Vec<i64>`（`search --json` の `papers.id`）、`--direction`（`both`/`refs`/`cited`、既定 `both`）、`--limit`（1 本・1 向きあたり、既定 20）、`--save-as`（必須）、`--json`。**`--depth` は無 designating**（常に 1 段。中身を見たいときだけ `sai-paper graph-show <ID>`）。入れた候補は `decision='pending'`。

### ステップ 6: 絞り込みは `paper-screen` に委ねる、DL はユーザー
- `search-external` の役目は「候補を集める」まで。抄録を見ての keep/drop 判定は `paper-screen` ブロック（`templates:` に `paper-screening`）が行う
- **PDF のダウンロードと Library 取り込みはユーザーが TUI で行う**。エージェントは検索セットに入れるところまで
- 文献完全性: メタデータは `sai-paper search` の返り値だけを使う。取得できなかった項目は `DOI [利用不可]` のように明示する（捏造禁止）

---

## エージェントが実際に叩く入口

| 実行者 | 入口 | 引数 |
|---|---|---|
| pi（拡張） | **`sai_paper_search_db`** ツール（`pi-litbase/tools.ts:196-240`） | `query`（必須）, `databases[]`, `limit`（既定 50）, `year_from`, `year_to`, `raw_query`, `save_as` → `sai-paper search … --json` |
| pi（拡張） | `sai_paper_set_list` / `sai_paper_set_show`（`--decision` で pending/keep/drop/downloaded）/ `sai_paper_set_mark`（`positions[]` + `decision` + `reason`）/ `sai_paper_set_merge` | — |
| bash | `sai-paper search` / `set` / `expand` / `graph-show` | 上のとおり |
| cc（Claude Code） | MCP `search_sets`（`mcp.rs:60-68`）＋ `Bash` | `set_id` 省略で一覧、指定で中身 |

**`expand` に対応する MCP ツールも pi-litbase ツールも存在しません**（`pi-litbase/tools.ts:378-386` の 15 ツール一覧に無い）。引用の網を広げる工程は、pi/cc どちらも `bash` で `sai-paper expand` を叩く必要があります。

---

## 実行前 derrot prerequisites

- **S2**: `discover.s2_api_key_env`（既定 `SEMANTICSCHOLAR_API_KEY`、本プロジェクトは `.env` の `SCHOLAR_API_KEY`）— キーが無くても動くが制限が厳しい
- **OpenAlex**: `openalex_api_key_env`（既定 `OPENALEX_API_KEY`）。被引用数の補完に''
- **ルート `.env` は CLI が起動時に読む**（`main.rs:865` `sai_paper_core::paths::load_project_dotenv()`）。キーの値は settings.json には書きません（環境変数の名前だけ）
- ダウンロードと取り込みの自動化は禁止。`sai-paper get-pdf` / `litbase ensure-pdf` はツールとして存在しますが、`search-external.md` は「PDF の DL と取り込みはユーザーが TUI で行う」と明示しており、探索期のエージェントには渡しません

---

## 典型チェーン（探索期）

```
paper-clarify（問い整理）  検索はしない。英文検索語に落とす
      ↓ 「明確にした問い」を依頼文に書く
paper-search（search-external）  search → set 保存 → expand 1 段 → report
      ↓ セット名を依頼文に書く
paper-screen（paper-screening）  抄録で keep/drop → set mark
      ↓ 残った論文番号を渡す
paper-map（分野地図）      サントピック別の表と Canvas 下書き
```
各段階の report は `## 引き継ぎ` で「使った論文」「次に使う DSR」を次の依頼文へ渡します（`research-core.md` の 6 項目、`sai-paper dsr check` が見出しを検査します）。

---

## 👤 User (9)
> 2026-09-27T04:06:45.498Z

paper-screen/paper-map は何を入力して，何をして，何を出力する？

---

## 🤖 Assistant (10)
> 2026-09-27T04:07:28.034Z

# `paper-screen` / `paper-map` — 入力・処理・出力

両方とも入力は **`task_text` だけ**です（sai-link の 2026-09-25 の仕様変更で「入力欄」は廃止、`sai-link/README.md:430`「ブロックへの入力は `task_text` の 1 つだけ」）。依頼文はカード本文の `# task_text` 節に書き、`{{input.task_text}}` で本文に差し込まれます。

---

## A. `paper-screen`（🔬 論文スクリーニング / 選別役）

`crates/sai-paper-cli/assets/blocks/paper-screen.md`

### フロントマター
```yaml
id: paper-screen
description: 探索期。検索セットの抄録を読んで keep / drop を判定し、候補を絞り込む（推奨：量をこなすモデル）
templates: [project-brief, research-core, search-internal, paper-screening, literature-hard-rules, domain]
timeout: 7200   worktree: none   thinking: high
```
`search-external` は**無い**＝このブロックは外部 DB を叩かない。共有テンプレート `paper-screening.md` だけで手順が写得ます。

### 入力（依頼文に書くもの）
1. **対象、検索セット名**（`paper-search` の report の「検索セットの名前と件数」節から写す）
2. **採用・除外の条件**（例「RCT のみ」「2023 年以降」「in vivo 研究」「主題はてんかん ONLY」）
3. 任意の指定（`確認: 少なめ/多め`）

公式の実例は run 50 の完了報告:「sai-link のカード作成で、… `paper-clarify` → `paper-map` → `paper-read` → `paper-critic` → `paper-gap` → `paper-plan`」。

### 処理（`paper-screening.md` の 4 ステップ）
```bash
# 1) セットを読む
sai-paper set show <セット名> [--decision pending|keep|drop|downloaded]
# 2) 各論文の抄録と基本情報（タイトル・著者・年・ジャーナル）を読み、条件を照合
# 3) 判定をセットに書き戻す（1 件ずつ / 一括）
sai-paper set mark <セット名> --position 1 --position 3 --decision drop --reason "in vitro のみ"
# 4) 進捗確認
sai-paper set show <セット名> --decision pending   # 0 になれば完了
```
- 読み取るデータは `search_set_items` の `position` / `remote_json` / `decision` / `reason`（`search_sets.rs:195-197`）。`remote_json` は `RemotePaper`（`title, authors, abstract_text, published, journal_ref, citation_count, doi…` / `models.rs:8-31`）
- 判定は 3 値: `keep`（次段階へ）／`drop`（除外理由が明白）／`pending`（**説明できないものは pending** — 「判定の理由を 1 行で説明できるか、説明できないものは pending にする」）
- `--reason` は 50 字以内推奨。`source` に `biorxiv` / `medrxiv` が含まれるか、タイトルに「preprint」「in press」がないかで**査読済みとプレプリントを区別**（`paper-screening.md`「既知の注意」）
- セットの結合 `set merge <A> <B> --as <名前>` も使える

### 出力
**(1) セット自体の更新**（副作用として永続）: `decision` と `reason` が `search_set_items` に書き込まれる。TUI の検索セット画面と `set show --decision` で後から読める。

**(2) report 1 つだけ**（`mcp__sai__sai_write`、`filename` = `paper-screen-<英字>.md`）:
```markdown
## 対象の検索セット（セット名）
## 判定の基準（使った条件）
## keep・drop の件数
## keep にした論文と理由（1 行ずつ）
## 迷った論文（pending とその理由）
## 引き継ぎ（research-core のひな形のまま）
```
＋ 冒頭の frontmatter（`kind: 検索結果` 等 / `block: paper-screen` / `papers: […]`）と末尾の `## 引き継ぎ` 6 項目。**秘 слова抄録や本文は report に入れず（`research-core.md`「出力」節）、1 行の理由だけ**。

**伝わらないもの**: ブロック本文に「## 依頼 / ## 役割 / ## 評価の観点 / ## 出力の形」の 4 節しかない — 手順はすべて `paper-screening.md` 側。DL も全文読みもしない（`paper-screening.md` の範囲）。

---

## B. `paper-map`（🗺 分野地図役）

`crates/sai-paper-cli/assets/blocks/paper-map.md`

### フロントマター
```yaml
id: paper-map
description: 探索期の終わり。研究の広がりと手薄な所を表にする（推奨：深い推論のモデル）
templates: [project-brief, research-core, search-internal, literature-hard-rules, domain]
timeout: 5400   worktree: none   thinking: high
```
`search-external` が無いのは「候補はすでに入手済み」の立場だからです。

### 入力
1. **対象トピック**（`paper-clarify` report の「## 明確にした問い」か、`paper-search`／`paper-screen` の残った論文群）
2. **範囲**（含めるもの / 含めないもの。広すぎると混雑と空白を同時に見誤ると明記）
3. 代表論文の候補（Library にあるもの）

### 処理 — 2 段構成

**(1) 観察表を作る（report の本体）**
「## 評価の観点」にある 4 軸を**別々に**見ます。1 つにまとめないのが評価基準。

| 軸 | 見方 |
|---|---|
| 研究の量 | 論文数・年の分布 |
| 証拠の強さ | 関連性の根拠 / 仕組みの根拠 / 前段階の実験 / 橋渡し / 人での検証（直接・間接・推論を区別） |
| 新しさ | — |
| 手薄さ | 「意味のある余地」か「見かけの目新しさ」か |

判定は必ず根拠（論文数・年の分布・繰り返しの型・代表グループの占有）を添える。**「未解決の課題」は書かない**（地図は観察まで。断定は `paper-gap` の仕事）。代表論文は Library に実在するものだけ `[#番号]` / `[DOI:…]` で。

**(2) Canvas を作る（`## 地図の Canvas` 節）** — このブロック固有の手順:
```sh
sai-paper understanding list --json                       # トピック名を必ずここの一覧から選ぶ
sai-paper canvas new map-<英字>-<YYYYMMDD> --draft          # 必ず先に作る
sai-paper canvas add-group map-… --label "<サブトピック>" --topic "<トピック名>"
sai-paper canvas add-text  map-… "研究の量 高 / 手薄さ 中" --group <id>
sai-paper canvas add-paper map-… 37 --group <id>            # Library にある論文だけ
sai-paper canvas add-report map-… <report ID> --group <id>
```
設計上の制約（コードで確認）:
- **色は指定しない**。`--topic` の理解記録から自動で決まる: `ok`=緑 / `vague`=黄 / 記録なし=灰（`canvas_edit.rs:277-283` `understanding_color`）。トピック名が揺れると灰のまま
- **`--draft` が必須**。既存の（ユーザーの）Canvas に足すには `--force`。`canvas.rs:353` のガードが「先に `canvas new … --draft` で作ってください」と明示
- `add-paper` / `add-report` は論文にタグも付ける（TUI 同期用 / `canvas.rs:7-10`）
- 最後に report の中で `地図の Canvas: \`map-…\`（sai-paper-tui の Canvas で開けます）` と書く（引き継ぎの前）

### 出力
**(1) 下書き Canvas** — `map-<英字>-<YYYYMMDD>`。ノード = サブトピック（枠）+ 文字札（量/手薄さ）+ 論文札 + report 札。**下書きなのでユーザーの何も上書きしません**（README 5.7 / `canvas.rs:7`）。ユーザーが TUI の Canvas で開いて並べ直します。

**(2) report 1 つ**（`mcp__sai__sai_write`）:
```markdown
## 対象のトピックと範囲
## トピック × 観点の表
| サブトピック | 研究の量 | 証拠の強さ | 新しさ | 手薄さ | 代表論文 |
|---|---|---|---|---|---|
（値は高/中/低。代表論文は [#番号] か [DOI:…]）
## 盛んな領域
## 手薄な領域（なぜ手薄と言えるか）
## 代表論文（5〜10 本）
## 次に使う DSR
地図の Canvas: `map-…`（sai-paper-tui の Canvas で開けます）
## 引き継ぎ（research-core のひな形のまま）
```
表の列（サブトピック / 研究の量 / 証拠の強さ / 新しさ / 手薄さ / 代表論文）は**共有契約**で、後続の視覚化テーマ（`guide-V`）が読みます（`guide-D-dsr.md` 共有契約）。

**伝えないもの**: 断定した未解決課題（→ `paper-gap`）、個別の論文の批判（→ `paper-critic`）。

---

## 対比表

| | `paper-screen` | `paper-map` |
|---|---|---|
| 入力 | セット名 + 採用/除外条件 | トピック + 範囲 + 代表論文候補 |
| 受け取るテンプレート | `paper-screening` 独有（外部検索なし） | 独有手順は本文（`## 地図の Canvas`） |
| 使う道具 | `set show` / `set mark` / `set merge` | `understanding list` / `canvas new/add-*` |
| 副作用（永続） | セットの `decision` / `reason` | 下書き Canvas（+ 論文タグ） |
| report の見出し | 5 つ | 7 つ + Canvas 名 |
| モデル | 量をこなす | 深い推論 |
| 次 | `paper-map` / `paper-read` | `paper-read` / `paper-gap` |

どちらも**`research-core` の作法（依頼の言い直し → 確認（既定2か所）→ 根拠 `[#37]` → 迎合しない → report 1 つだけ保存 → 引き継ぎ 6 項目）**が先頭に差し込まれ、出力 формыは `sai-paper dsr check <report> --block paper-screen|paper-map` で LLM なしに検証できます（`crates/sai-paper-core/src/dsr_check.rs`、見出し不足で exit 1）。

---

## 👤 User (11)
> 2026-09-27T04:18:34.056Z

変数名や関数名は不要です．

`.sai/blocks/*.md` の実行順番の樹形図を作成して．
いくつか例を作成すること．

各 block:
- 何を入力命令するのか
- 何を処理するのか(sai-paper cli --option とそれの出力がどこに反映されるか)
- 何を出力するのか
- ユーザーは途中で何をするのか
- 次にアクションは何か？

ルール:
- how ではなく，what で答えよ．
- 開発者目線ではなく，ユーザー目線で説明せよ．
- ユーザーが実行するときに何を操作し，どこで結果を確認し，何を考える，何をどこに書くべきかを説明する．
- 日本語で説明せよ．

---

## 🤖 Assistant (12)
> 2026-09-27T04:20:38.759Z

# `.sai/blocks/*.md` の実行順番（樹形図）

## 全体図

```
【毎回、実行の直前に自動で入るもの】
  ・プロジェクトの状況（目標・段階・今の作業・次の一手・更新日／ユーザーの考え／前回の引き継ぎ／レビュー）
  ・研究用の共通の作法（依頼を1文で言い直す → 途中で2回止まる → 根拠に番号を付ける → 迎合しない → report は1つだけ保存 → 引き継ぎ6項目）
  ・研究全体のhelp（分野ごとに「問いの枠／証拠の強さ／落とし穴」）
        ↓
【入口】sai-paper-tui の Project ページ
  ├─ A（要対応の一覧）…… citations/推薦/新建論文/review/質問がここに並ぶ。Enter で開き、x で「対応済み」
  ├─ U（理解の確認カード）……→ paper-check
  └─ N（次の一手カード）……→ paper-next
  ＋ S（ブロック選択モーダル）…… 任意のブロックを直接カードにする
        ↓
【探索期】問いを立て、候補を集め、絞り、地図にする
  │
  ├─ 🧭 paper-clarify ── 問い整理役 ───── 深い推論 ── 検索はしない
  │     出力: 明確にした問い（平易/研究方向/英語検索語）＋「次に使う DSR」
  │        ↓（明確にした問い を依頼文に書く）
  ├─ 🔍 paper-search ── 文献収集役 ───── 量をこなす ── 外部DB（PubMed/bioRxiv/S2/arXiv/OpenAlex）
  │     出力: 使ったDBと検索語、検索セット名と件数、代表候補 → paper-screen へ
  │        ↓（セット名を依頼文に書く）
  ├─ 🔬 paper-screen ── 選別役 ──────── 量をこなす ── 抄録だけを読んで keep/drop/pending
  │     出力: 採用の論文と理由1行、迷ったもの → paper-read / paper-map へ
  │        ↓（残った論文の集合を渡す）
  └─ 🗺 paper-map ───── 分野地図役 ──── 深い推論 ── 地図の表 ＋ 下書き Canvas
        ↓
【精読・議論期】1本ずつ読み、疑う
  ├─ 📖 paper-read ──── 精読役 ──────── どちらでも ── 深さ:概要/標準/詳細
  ├─ 🧨 paper-critic ── 批判役 ──────── 深い推論 ── 主張ごとの証拠の強さ／方法の弱点／結果の信頼性
  ├─ 💬 paper-discuss ─ 論文ディスカッション ── どちらでも
  └─ 🏛 paper-debate ── 議論の司会 ──── 深い推論 ──（費用大）
        └─ 子として呼ばれる: 👤 paper-debater ／ 🔎 paper-reviewer（ユーザーは実行しない）
        ↓
【絞り込み期】
  └─ 🕳 paper-gap ───── 空白発見役 ──── 深い推論 ── 方法/対象集団/証拠 の3つの空白 ＋ 埋める研究の案
        ↓
【計画化期】
  └─ 🧪 paper-plan ──── 計画化役 ───── 深い推論 ── 目的と仮説／進め方／確かめ方／引き渡し書
        ↓
【いつでも使える横道】
  ├─ 📚 paper-library-qa ── Library への質問 ── 根拠付き回答、無い分は [GAP:] で明示
  ├─ 💡 paper-brainstorm ── 相談役（制約を外す。検索しない）
  └─ 📝 paper-template-maker ─ 説明書づくり役（テンプレートの下書きだけ。_segments には置かない）
        ↓
【記録を締める】
  ├─ 🎓 paper-check ── 理解の確認 ── 3〜5問を1問ずつ／理解の判定を保存
  └─ 🧭 paper-next ─── 次の一手 ──── 候補を最大3つ／選んだものはカードになる
  ＋ 毎日: J（研究日誌）… 昨日までの経緯を見るだけ
```

---

## 結果の「置き場所」一覧（オプション → 反映先）

| 実行すること | 対応する指定 | 結果が残る場所（ユーザーが確認する画面） |
|---|---|---|
| 外部DBの検索 | 検索語・DB を複数・件数・年の範囲・DB固有の式・**セット名** | **検索セット**（TUI のセット画面。セット内の1件に keep/drop が付く） |
| 候補の絞り込み | 位置・判定（keep/drop）・理由（50字以内） | 同じ**検索セット**の1件の状態と理由 |
| 引用の網を1段広げる | 論文番号・向き・1本あたりの上限・行き先セット名 | 検索セットに追加（判定は pending） |
| Library 内検索 | 検索モード・上位件数・対象（論文/ノート/全部）・索引名 | **どこにも残らない**。report の本文にだけ焼き付く |
| 索引づくり | 索引名・分け方・埋め込み | Library の断片数と埋め込み数（TUI で論文を開いたときの RAG がreaction する） |
| PDF の入手・取り込み | — | **論文フォルダ**（TUI の `d` / インポート画面。エージェントはここまで来ない） |
| 引用の抽出 | — | 論文フォルダの抽出Markdown（TUI の `s` キー。抽出済みなら 2 回目から即表示） |
| 地図の Canvas | 名前（`map-…-<日付>`）・下書きフラグ・枠・札 | **下書き Canvas**（TUI の Canvas 画面。ユーザーのものは上書きされない） |
| 理解の判定 | トピック名・判定（分かっている/あいまい/未確認）・根拠 | トピックの色（緑/黄/灰）と Project ページの「理解の確認」 |
| report の保存 | ファイル名（ブロックID+英字）・説明 | `.sai/outputs/<実行>/…` → TUI の PAPER 画面 REPORTS 欄、Project ページの最近の引き継ぎ |
| 次のカード | カード名 | sai-link の todo（`N` キーで作ったものと同じ場所） |
| 要対応 | — | Project ページの「要対応」（`A`） |

---

## 各ブロック：入力命令 / 処理 / 出力 / ユーザーの行動 / 次のアクション

### 🧭 paper-clarify（問い整理役）
- **入力する命令**：依頼文に「調べたいこと」を書く。`確認: 少なめ / 多め` を付けられる。**検索語は書かなくてよい**（ Making them is the job）
- **処理**：依頼を1文に言い直し、問いの種類（比較/判別/予後/予測/曝露/机制/実施/橋渡し/探索）を見分け、足りない要素（対象・比較対象・期間・場所・サブ集団・目指す証拠）を 列挙し、执行可能な枠（`domain.md` の方針）と英語の検索語候補に落とす。**DB を叩かない**
- **出力**：report 1つ。見出し＝解釈／問いの種類／足りない要素と確認したこと／選んだ枠／明確にした問い（平易・研究として・英語検索語と組み合わせ）／範囲／次に使う DSR／引き継ぎ
- **ユーザーが途中ですること**：最初に止まる「問いが固まったとき」で**自分の.month の言葉**に直す。`確認: 多め` なら検索語一覧を見せられるので通す・直すを決める
- **次のアクション**：「明確にした問い」節と「次に使う DSR」節を丸ごagadелиして次カードの依頼文に書く

### 🔍 paper-search（文献収集役）
- **入力する命令**：依頼文に「何を探すか」「年の範囲（例 `2018-`）」「どのDBを優先」。集合名は**韭菜ある**（下の処理で必ずセットに保存される）
- **処理**：依頼文の Quem を英語3〜5個に割る→`--db` で複数DB（PubMed / bioRxiv・medRxiv / Semantic Scholar / arXiv / OpenAlex）→`--year-from/--year-to`→PubMed は `--raw-query` で Boolean と検索式→`--limit` で件数→**`--save-as` で検索セットに保存**→重複を自動除去（DOI → S2 の論文ID → タイトルを正規化したもの）→DB が1つ失敗しても止めず、失敗したDBと理由を報告→引用を返さないDB分は OpenAlex で被引用数を補う→最後に**引用の網を1段だけ**広げて同じセット（または `-expand` を付けたセット）に追加
- **出力**：report 1つ。見出し＝検索の目的／使ったDBと検索語／**検索セットの名前と件数**／重複を除いた数／代表候補5〜10本／次にすること／引き継ぎ。加えて**検索セットという永続データ**が残る
- **ユーザーが途中ですること**：`確認: 多め` で検索語一覧を見る。止まったら「1本に絞る」「年を古くする」。**Depoisには選別段階へ進むだけ。PDF の取得はここではやらない**
- **次のアクション**：「検索セットの名前と件数」を依頼文に書いて paper-screen のカードを作る

### 🔬 paper-screen（選別役）
- **入力する命令**：依頼文に「対象セット名」と**採用・除外の条件**（RCT のみ／2023年以降／in vivo／主題は○○のみ）
- **処理**：`set show` でセットを引く（`--decision pending|keep|drop|downloaded` で絞り込み）→各論文の**抄録と基本情報だけ**を読み、条件と照合→`set mark` で1件ずつ、または位置をまとめて `keep` / `drop` と理由を記録→`set show --decision pending` で残りを確認。**理由が1行で言えないものは pending のまま置く**。プレプリント（biorxiv/medRxiv の出所、タイトルに preprint/in press）は査読済みと区別
- **出力**：（1）**セット内の1件の状態と理由**が保存される（画面に残るのはこちら）、（2）report 1つ。見出し＝対象の検索セット／判定の基準／keep・drop の件数／keep の論文と理由1行ずつ／迷った論文（pending と理由）／引き継ぎ
- **ユーザーが途中ですること**：結果が **`pending` 0** になるまで待つ。止まったら基準を緩める／締める（例「in vivo だけ」を「in vivo ＋ in silico 予測」にする）。抽出の無い論文は**ここで-depth 決めない**
- **次のアクション**：keep になった論文番号を列挙して paper-read（1本ずつ）／paper-map（一括）へ

### 🗺 paper-map（分野地図役）
- **入力する命令**：依頼文に「対象トピック」と「含めるもの／含めないもの」。前のカードで残った論文群
- **処理**：①`understanding list --json` で**既存のトピック名をそのまま使う**→②`canvas new map-<英字>-<日付> --draft` で**必ず先に**下書き Canvas を作る→③表の行ごとに `canvas add-group --label … --topic …` で枠を作る（**色は指定しない**。理解の記録から自動で緑/黄/灰になる）→④`canvas add-text` で「研究の量 高 / 手薄さ 中」の札、`canvas add-paper` でLibraryにある論文の札、`canvas add-report` で関係する report の札を置く。観察の軸は**4 つ別々**（研究の量／証拠の強さ／新しさ／手薄さ）。断定した「未解決の課題」は書かない
- **出力**：（1）**下書き Canvas**（ユーザーの Canvas は壊されない。Topicが揺れると枠が灰のまま）、（2）report 1つ。見出し＝対象のトピックと範囲／**トピック×観点の表**（サブトピック・研究の量・証拠の強さ・新しさ・手薄さ・代表論文）／盛んな領域／手薄な領域（なぜ手薄か）／代表論文5〜10本／次に使う DSR／**地図の Canvas 名**／引き継ぎ
- **ユーザーが途中ですること**：`確認: 多め` なら着手前に手薄さの判定材料（論文数・年の分布・繰り返しの型・代表グループの占有）を見せて確認する。終わったら**TUI の Canvas で並べ替え**、使わonuclei枠は捨てる
- **次のアクション**：表と代表論文を paper-read（`#37 を読んで 深さ: 概要`）へ。空きが本物かどうかは paper-gap で確かめる

### 📖 paper-read（精読役）
- **入力する命令**：依頼文に「論文番号（`#37`）」と**深さ**（`深さ: 概要 / 標準 / 詳細`）
- **処理**：Library 内のその論文の**抽出 Markdown を読む**（Library 内検索で該当箇所を絞る）。研究デザイン（著者の自称ではなく実際の方法から）、問いと主張の対応、主張を支える**データと手順の分け**方、証拠の強度（関連/予測/因果/検証）、図と本文の食い違い、限界、看読できた部分と「読めなかった」部分を切り分け。`詳細` では再現できる手順に分解
- **出力**：report 1つ。見出し＝論文の種類／問いと主張／方法の分解／図と本文の食い違い／限界／次に読む論文／引き継ぎ。`概要` のときは「論文の種類」「問いと主張」「引き継ぎ」だけ
- **ユーザーが途中でする**こと**：結論を書く前の停止で「ここまでの読みでよいか」を決める。**図が読めていないときは「読めなかった」と返すのが正解**（推測で埋めさせない）
- **次のアクション**：「次に読む論文」を依頼文にして paper-read を続けるか、「限界」「次の読む論文」をCritic へ渡す

### 🧨 paper-critic（批判役）
- **入力する命令**：依頼文に「論文番号（または 批評対象の report の要点）」
- **処理**：**仮説不利于証拠を少なくとも1つ**探しに行く。各主張について根拠を突き合わせ、**強さを高/中/低**で採点。交絡・選択バイアス・多重性・欠測・検証の深さ・再現性（ソフトウェア名と版・パラメータ・データとコードの公開）を点検。結果の信頼性、食い違う報告factory あるならその理由まで。judged的低は**「探したが見つからなかった」**とも書ける
- **出力**：report 1つ。見出し＝**主張ごとの証拠の強さの表**（主張／根拠`#番号`／強さ／理由）／方法の弱点／結果の信頼性／食い違う報告とその理由／著者に聞きたいこと／引き継ぎ
- **ユーザーが途中ですること**：結論を書く前の停止で「弱い部分をどこまで踏み込むか」を決める。**反論abby Science  等、追試や反証の論文があるなら「次に読む論文」として依頼文に足す**
- **次のアクション**：「限界」「著者に聞きたいこと」を paper-gap（本当に未解決か）か paper-discuss（別論文と議論）へ

### 💬 paper-discuss（論文ディスカッション）
- **入力する命令**：依頼文に「論文番号」と「，某些論点（どの切り口で議論するか）」
- **処理**：1本の論文を要約したうえで、依頼で示されたトピック（研究分野への示唆・方法論・臨床/応用・異分野比較）ごとに強みと限界を並べる。関連論文があれば `#番号` で引く
- **出力**：report 1つ。見出し＝論文の要旨／トピック／強み／限界と批判／この研究分野への示唆／関連論文／引き継ぎ
- **ユーザーが途中ですること**：「次に使うDSR」で出た debate 系（費用大）に進むか、#elif で止めるかを決める
- **次のアクション**：strategic な mirror が必要なら paper-debate Critique

### 🏛 paper-debate（議論の司会）
- **入力する命令**：依頼文に「議題」と「型」（相談／対話／**円卓**）と「回す人数」。査読会なら「査読会」と書く
- **処理**：司会が**参加者（批判役・精読役・空白発見役など）を子として呼ぶ**。1回あたりの子実行が往復数だけ増えるので**費用が大きい**。合意点・対立点（それぞれの根拠）を分けて記録
- **出力**：議論録 1つ（種類: 議論録、参加者と実行番号つき）。査読会なら別形式。故障時は**失敗した子の画面が残る**ので原因を見てから閉じる
- **ユーザーが途中ですること**：** Sergius に何を debating させるか**を途中で指定（1发言人の削除や追加）。opted Lc すべて承認は出ない
- **次のアクション**：合意点と未解決を次のカードの依頼文へ。対立が残れば paper-plan へ

### 🕳 paper-gap（空白発見役）
- **入力する命令**：依頼文に「対象トピック」（＋ 集めた論文の番号やセット名）
- **処理**：まず Library と外部DBで文献を集め、**既に使われている 것과未解決のまま残ることを分けて**示す。空白を **3 つの軸**で探す——①方法（設計・偏りと交絡・解析の厳密さ・検証の深さ・再現性・ｒ)（他研究への持ち歩きやすさ）②対象集団（見る軸・状態の区別・名目だけの層別 vs 意味のある層別・証拠の深さ段階）③証拠（件数・査読済み/プレプリント・直接/隣接の分離）。**「もっと検証を」「もっと大きなデータ」のような疑似の空白は退ける**。残った空白ごとに信頼度（高/中/低）
- **出力**：report 1つ。見出し＝対象のトピック／方法の空白／対象集団の空白／証拠の空白／それぞれの根拠（`#番号`・`DOI:`）／埋める研究の案／引き継ぎ
- **ユーザーが途中ですること**：收集段階の停止で「集める範囲」を決める（PubMed だけか arXiv まで広げるか）。**曖昧な空白を「研究課題」に変換 Família 打击 ない か 見直す**
- **次のアクション**：「埋める研究の案」を依頼文にして paper-plan へ

### 🧪 paper-plan（計画化役）
- **入力する命令**：依頼文に「絞り込んだ問い（空白発見の『埋める研究の案』）」
- **処理**：主目的を1つに絞り、仮説を**確かめられる形**にする。実行できる進め方を並べ、**主なものを1つ選ぶ**（複雑さや目新しさではなく、問いへの当てはまりで）。代わりの進め方と**選ばない理由**（時期尚早／依存が多い／問いとずれる／検証が足りない）を併記。検証を**内部・外部データ・時間を分ける**の3種に区別。新しさと実現しやすさ（データ・サンプル・計算・検証・協力者・時間の実際の負担）、ボトルネックと対策。最後に**外部ツールへそのまま渡せる「引き渡し書」**（目的／使うデータと入手先と利用条件／前処理／方法の候補／評価の指標と分け方／成功の基準（数値）／参考論文（`#番号` と DOI）／分かっていない点）
- **出力**：report 1つ。見出し＝目的と仮説／主な進め方と代わりの進め方／確かめ方／新しさと実現しやすさ／ボトルネックと対策／**引き渡し書**／引き継ぎ
- **ユーザーが途中ですること**：結論を書く前の停止で**進め方の選択にAcquire する**（別部署の細かな合計で却下される）。引き渡し書は**他人に渡しても.starts できるレベル**になっているかをここで読む
- **次のアクション**：実装は**別ツール**に「引き渡し書」を渡す。sai-paper 側の作業はここでおわり

### 📚 paper-library-qa（Library への質問）
- **入力する命令**：依頼文に質問文だけ（peringあり）。**いつでも起動**
- **処理**：日本語の問いを**英語の検索語3〜5個**に割る→`--mode hybrid`（埋め込みサーバが止まっていれば `--mode word`、英語のみ）・`--top-k 10`・`--scope all` で Library 検索→同じ論文の断片はまとめ、複数検索語に出た論文を上、body > note の順→足りなければ近傍検索で広げ→**Public で答えられない部分を `[GAP: 推奨検索: <英語>]` で残す**。索引が0件なら**勝手に作らず**ユーザーに `index build` を頼む
- **出力**：report 1つ。見出し＝質問／回答／根拠（サブトピックごと、出典は `#番号`）／補足・関連知見／**Library では答えられない部分**／引き継ぎ
- **ユーザーが途中ですること**：索引が無く止まったら `index build` → `index embed` を自分で打つ（1回作れば以後使える）。止めたいときは `確認: 少なめ`。**廣pool广Daily 質問はメモに溜めていき、まとめて1回聞いたほうが安い**
- **次のアクション**：得た `#番号` を paper-read / paper-critic へ

### 💡 paper-brainstorm（相談役）
- **入力する命令**：依頼文に「いま詰まっていること」「制約asks」。**検索しない**
- **処理**：制約を外した案、他分野からの類推、中心の前提を逆にした案。**3案に絞り、確かめ方を附帯**
- **出力**：report 1つ。見出し＝制約を外した案／他分野からの類推／前提を逆にした案／有望な案3つと確かめ方／引き継ぎ
- **ユーザーが途中ですること**：止められた3案から**1つ選ぶ**。選ばなかった案の理由を一言残すと次の依頼に使える
- **次のアクション**：選んだ案を paper-plan の依頼文へ

### 📝 paper-template-maker（説明書づくり役）
- **入力する命令**：依頼文に「材料の report（`waggle:` で埋め込むか token で指定）」と「何を残したいか」
- **処理**：うまくいった進め方から**分野に依存しない部分だけ**を抜き、下書きを作る（手順／観点／置き場所の案／既存テンプレートとの重なり）。`.sai/template/` には**直接書かない**。テンプレートの総数は**20まで**の目安
- **出力**：report 1つ（下書き）。見出し＝材料／残す進め方の要約／テンプレートの下書き（md 全文）／置き場所の案（どのブロックの `templates:` に足すかも）／既存との重なり／引き継ぎ
- **ユーザーが途中ですること**：**置くかどうかはユーザーが決める**。`.sai/template/` に名前をつけて保存し、使いたいブロックの `templates:` の並びに1行足す
- **次のアクション**：以後そのブロックのすべての実行がその Hawking を受ける

### 🎓 paper-check（理解の確認）
- **入力する命令**：依頼文は任意（トピックを指定すれば絞る）。**TUI の `U` キーでカードが作れる**
- **処理**：`understanding list --json` と `progress --json` を読み、**選ぶ順は固定**——①引き継ぎの「ユーザーへの質問」で未回答、②「未解決」、③理解が「あいまい」のトピック、④report はあるのに開いていないトピック。そこから**3〜5問を1問ずつ**、最初「飛ばしてよい」と伝えてから聞く（採点しない）。**5問を超えない**。補足には論文番号と場所 `#37 §3.2` を添える。最後に**判定の表を見せてユーザーの直しを反映してから**、確認を取って1行ずつ保存（`ok` / `vague` / `unknown`）。**mind.md と state.md は直接書き換えない**
- **出力**：（1）**各トピックの理解の記録と色**（緑/黄/灰）（2）要対応（`A`）で確認が要るもの（3）report 1つ。見出し＝問いと答えの要約／補足（根拠付き）／判定の表／**mind.md への追記案**／引き継ぎ
- **ユーザーが途中ですること**：**1問ずつ答える（5問まで）**。保存前に判定表を直す。保存を断ったら保存されない。** technologically思ったことはmind.md への追記案に書いてもらい、`M` で開いて自分で追記**（エージェントは書かない）
- **次のアクション**：Project ページの「理解の確認」のあいまいな数が減る。減らなければ paper-brainstorm や再検索

### 🧭 paper-next（次の一手）
- **入力する命令**：依頼文は任意。**TUI の `N` キーでカードが作れる**。未実行の同じカードがあれば作らない
- **処理**：`progress --json`（段階・記録）と `handoff list --tag` を読む→**候補を最大3つ**、表（番号／種類／ブロック／理由／依頼文）で提示→選ばれたものについて `handoff create --block` で**カードそのものを作る**（作るだけで実行はしない）
- **出力**：（1）**sai-link の todo にカード**（2）report 1つ。見出し＝状況の読み取り／候補の表／選ばれた候補と作ったカード（run 番号）／引き継ぎ
- **ユーザーが途中ですること**：3つから選ぶ・数量の追加を求める。**カードを作っただけで Metropolis 率は進まない**ので、sai-link で実行をかけるか TUI の todo レーンから走らせるかを選ぶ
- **次のアクション**：できたカードをそのまま実行する。繰り返し（毎回 `N` → 実行）offer the 节奏

---

## 例 1：初めてあるテーマを調べる（探索期をまるごと）

**0｜準備（ユーザー、Terminal）**
```bash
sai-paper init --root ~/research/epilepsy-eeg --domain signal-ml
$EDITOR ~/research/epilepsy-eeg/.sai/template/domain.md   # 自分のテーマに直す
sai-paper index build && sai-paper index embed            # 索引は自分で1回
```

**1｜問い整理** — カード本文に
```
てんかんの脳波解析で発作を予測する研究を調べたい　確認: 多め
```
`確認: 多め` なので検索語一覧を見せて止まる。**「対象は 成人の Focal epilepsy だけ」
に直してから続行。** 出力の `## 明確にした問い` と `## 次に使う DSR` を丸ごと次の依頼文に貼る。

**2｜文献収集**
```
明确的になった問いの検索語（英語）で集める。2018年以降、PubMed と S2 と arXiv。　確認: 少なめ
```
`--save-as` でセット（例 `epilepsy-seizure-prediction`）が残る。止まったら「arXiv を外す」等の判断。出力の `## 検索セットの名前と件数` を次へ。

**3｜選別**
```
セット epilepsy-seizure-prediction を絞る。採用: 被験者内検証あり・複数データセット、除外: 動物実験のみ、方法論の主張のみ　確認: 多め
```
セットの1件に keep/drop が付き、**TUI のセット画面で pending が減っていく**。`pending` が 0 になったら次。

**4｜地図**（残った論文 всей 一括）
```
keep になった論文を前提に、てんかん発作予測研究の分野地図を作る。範囲: 2020年以降_partition/ 表層 …
```
`map-epilepsy-<日付>` が**下書き Canvas** としてできる。TUI の Canvas で並べ替え、使わぬ枠を捨てる。表と代表論文を次の依頼文へ。

---

## 例 2：1本の論文を疑う（精読・議論期）

**1｜深さを選ぶ**
```
#37 を読んで　深さ: 標準
```
抽出 Markdown がない場合、-agent は**止まって理由を見せる**。TUI で `s` キー（引用文献抽出）か `d` キー（PDF 入手）を先に済ませる。

**2｜批判**
```
#37 の主張を全部洗い出して、証拠の強さと方法の弱点を評価して
```
`確認: 少なめ` を付けると結論前の停止だけなのでTerraform 早くなる。`## 主張ごとの証拠の強さ` の表で**弱い主張が1つあることに気づく**。

**3｜空白の確認**（本当に未解決なのか？）
```
さっきの批判を受け入れられるかを確かめて。Population gap と方法 gap を見る
```
**「もっと大きなデータで検証」だけの理由が退けられる**のを-user 側で確認する。

**4｜計画**
```
採用できた問い（sample: 交差サブジェクト検証＋外部データ検証が未実施）で研究計画を作って
```
`## 引き渡し書` の「使うデータ（名前・入手先・利用条件）」「成功の基準（数値）」が**そのまま別ツールに渡せるか**をユーザーが読む。

---

## 例 3：随时疑问を溜めてまとめて聞く

**0｜毎日**
TUI の `J`（研究日誌）で昨日までの経緯を見る。

**1｜Phone からメモ**
`sai-paper phone mind-append` で思いついたことを `mind.md` の `## メモ` に足す。**書いても良いのはここ**（mind.md は agent が直接書かない）。

**2｜カード**
```
最近行政审批した知识的确认。次の理解を確かめて。（確認: 多め）
```
`paper-check` を選んで実行。**3〜5問を1問ずつ**、飛ばしていい。北「保存してよいか」を訊かれたら、判定表を直してから `ok`（=Gantt緑）。

**3｜要対応の片付け**
TUI Project ページで `A` → 該当を選び `Enter` で元を開く → 片子済みなら `x`。`Enter` の飛び先は質問カード/推荐/review.md/state.md/出典の report のどれか（それぞれ別の場所が開く）。

**4｜次の1手**
`N` を押すとカードが1枚できる → sai-link で実行するか捨てて作り直す。**実行するのはユーザー**。

---

## ユーザーが「何を考えるか」のチェックリスト

1. **停止で止まっているのは「設計のذهب」か「 passagem 却不采」か** — 設計なら `確認: 少なめ`、不懂なら `確認: 多め` にして同じ …
2. **report の表に予想外の弱点はあるか** — 「この分野に未解決 именно は本当か」を詰めるのは空白発見役の仕事
3. **Library で答えられないとされたものはあるか** — あるなら「[GAP:] の推奨検索語をそのままを 依頼文に」。**外部検索の入口は `paper-search` だけ**
4. **地図の「手薄さ」が本物か** — 自分の知らない分野が白く出ていないか（Topic 名が揺れると灰のまま）
5. **費用** — `paper-debate` の円卓は1回あたりの子実行が跳ね上がる。**通常は discussion で足りる**
6. **成果物の置き場所** — 自分の考えは `mind.md`、現況の更新は `state.md`（どちらもエージェントが直接は書かない。**改名案が出るので、ユーザーは `M` / `W` で開いて自分で書く**）

---

## 👤 User (13)
> 2026-09-27T04:24:14.079Z

現状の想定するワークフローにおいて，ソフトウェアが対処できない課題点，問題点を多角的に調査して説明して．

ユーザー側に現れる問題点をメインに説明し，それがなぜ発生するかは簡潔に説明すること．

---

## 🤖 Assistant (14)
> 2026-09-27T04:25:41.942Z

# 現行ワークフローでソフトウェアが対処できていない課題（多角的に）

調査対象: `crates/sai-paper-cli/assets/{blocks,templates}`（DSR 一式）、`README.md` §4.7・5.4・5.7・5.8、`.sai/outputs/R9/5*/completion*.md`（テーマ M/N/C/B/R/S/V/D の完了報告の「残った課題」）、`.sai/outputs/R10/{119,120}/completion*.md`（最新テーマ U）、`sai-link` 側の実装と実測。

表記: **症状**＝ユーザーに見えること／**原因**＝なぜこうなるか（1〜2 行）。

---

## A. 手順が 3 つの画面に分断されている

| 症状（ユーザー側） | 原因 |
|---|---|
| 検索・取り込みは sai-paper-tui、カードの実行は sai-link-tui、Experimental 記録はスマホ。**起動し直しながら来回する** | 3 つのアプリが別プロセスで、別 DB を使い、認証・設定も別（`.sai/paper/settings.json` / `.sai/settings.json` / `.env`） |
| 論文を選んでエージェントに渡すのに、TUI で `S` → モーダルでブロック選択 → 説明文入力 → 再度 sai-link-tui を開く、の 2 画面移動 | TUI は「カードを配る」だけ、`S` は task.md を発行するだけで実行は別卦。`block run` は既定で kanban 対話（`sai-link/README.md` 3.2） |
| 「カードを作ったのに研究が進まない」実態が頻出 | カード作成と実行は別操作。`N` キーで作ったカードも**実行はユーザー**（`paper-next` の出力にも「作ったカード（run 番号）」だけと出る） |
| 失敗しても放置される | 失敗時は **pane が残る**（原因調査用）。閉じたつもりでも次に「この pane なんだ」と混乱（`sai-link/README.md` 3.2 の表） |

**回避**: 1 回の流れを「TUI で候補を作る → 同じ画面から `S` → sai-link-tui の todo レーンで実行」の 2 段と決めておく。

---

## B. 資産（ブロック・テンプレート）が二重管理になる

| 症状 | 原因 |
|---|---|
| **`sai-paper init` を実行しても、古いプロジェクトではルールが更新されない** | 既存ファイルは絶対に上書きしない設計（`agent_assets.rs:130-140` の「書いたら true」）。`--assets-only` は「足りない物だけ」で、**古い版がそのまま残る**（R9/51 の申し送り 2 で明示的に課題として举がっている） |
| `domain.md` を `--domain` で変えられると思っても変わらない | 既に `domain.md` があれば上書きせず案内だけ（`agent_assets.rs:263-268`）。しかも **`--domain` 無しで `--assets-only` を実行すると毎回「既にあります」と出てうるさい**（R9/50 申し送り） |
| 開発リポジトリで DSR を試せない | ここには `.sai/blocks/` に Sai-link 用の 6 ブロックしか無く、`paper-*` は init を走らせていないので入らない。**研究は別フォルダ前提**だが、その 2 つ目を毎回도가得る |
| 案内が実際の名前と違う | README 5.4 の「名前」列（文献収集役…）と frontmatter の `name`（🔍 論文検索…）が不一致、放置 Decide 中（R9/50 申し送り 1） |
| 古いテンプレートを読んでしまう | 互換残置の `paper-search-strategy.md` / `rag-simple.md` / `rag-deep-search.md` に **`index status --json` という存在しないオプション**が書いてある（実測: このオプションは弾かれる）。旧 block.md を参照している証拠も残っている |

**回避**: プロジェクトの `templates/` を更新したいときは、**自分で差分を見てから** `rm` して `--assets-only`。自動化は無い。

---

## C. 「依頼文」だけが操作edes ない（入力欄が消えた）

| 症状 | 原因 |
|---|---|
| 停止の多さ・精読の深さ・年の範囲を、**コマンドで指定できない**。全部依頼文に日本語で書くしかない | 2026-09-25 にカードの入力欄（inputs/outputs）が廃止され、入力は `task_text` だけになった。値は**自然言語のキーワード**としてだけ効く（`research-core.md` を読むだけ） |
| 依頼文の書き方の差で結果が大きく変わる | 同じ `paper-clarify` でも「確認: 多め」があるのと無しで途中の確認回数が変わる。`paper-read` の「深さ」も 3 段階の自然語のみ |
| 何を書けばよいか分からない手で止まる | 依頼文の雛形は各ブロックに 1 例も無い（「例: #37 を読んで 深さ: 概要」は README 側の記述） |

---

## D. 検索と索引の実務（ユーザー要做作業）

| 症状 | 原因 |
|---|---|
| **初めて使うと検索が 0 件**で agent が止まる | 索引は**手動で 1 回だけ**作る運用。agent は索引を作らない（ユーザー決定、`search-internal.md` ステップ 1） |
| 索引作りは**費用**がかかる | 埋め込みが OpenRouter 経由。本物の Library なら索引 1 つで 1,700〜2,700 塊＝実費 |
| S2 だけ 403 になる・検索が薄い | 本environmentalの S2 キーが無効（`SCHOLAR_API_KEY` は 403、新しい名前の環境変数は空。R10/119 申し送り 3） |
| **2 ページ目以降が取れない**。arXiv/PubMed/OpenAlex/bioRxiv は一度MOREで終わる | ページ送りは S2 のみ実装。年の範囲も S2・bioRxiv・medRxiv のみ対応（R10/119 申し送り 5） |
| 同じ論文が検索セットに重複して入る | 重複判定の優先は DOI → S2 の論文ID → タイトル正規化で、**arXiv ID での突合は未実装**（`api/mod.rs:137-140` に「暫定: タイトルベースで以下に続く」とコメント）。arXiv 経由の論文は 2 本並ぶ |
| 同じ論文が複数の検索の下に重複表示される | 候補紐付きと題名/DOI 照合の 2 経路が別々に並ぶ（R10/119 申し送り 7。仕様通りだがielder は「重複」と感じる） |
| 検索セットが空のまま保存されることがある | 原因未確認（R10/119 申し送り 6）。途中結果の保存に変えたのは後追い |
| `--limit` の既定が 10 なのに見つからない | 既定 10。探索用に 100 を出すには依頼文側で指定しないと 10 で終わる |
| 上位 20 件が 1 本の論文の切れ端で埋まる | 1 本あたり 2 件に絞る既定がある。`--per-paper 0` か `--paper` を指定しないと回避できない |

---

## E. 検証・出典（ユーザーが自分で確認する羽目になる）

| 症状 | 原因 |
|---|---|
| 開いた report の先頭に「出典: 要確認 40 件」のような**大量出水**が出る | 検査が引用ごとに 1 件入る。**小数点を含む数字をすべて無根拠扱い**（`v1.2`・`図 1.2`・`R 4.0` も対象）。段落内の**別の論文の年**と比自己 Ransomware「年が違います」が出る（R9/51 申し送り 1 の実例あり） |
| 本物の report で誤検出率-Carousel か分からない | 本物の report がまだ無く**検査の厳しさの基準が未確定**（同上・ユーザー判断待ち） |
| `C` を押さないと実照合されない | 外部照合は**ネットワーク必須・手動**。オフラインだと確認永远是「要確認」のまま |
| 撤を自分で觉得必要 | `enrich --retraction` は手動実行（1 秒に 1 本の制限あり） |
| 廷Crucially 撤退の「[撤回]」印が古い | 印は最後に `enrich --retraction` を走らせた時点の事実 |

---

## F. 確認（止まる場所）を turbo 回せない

| 症状 | 原因 |
|---|---|
| agent が質問して止まったとき、**スマホに届かない**ことがある | 通知は `ask_user_phone` 拡張が担うが、これは `~/.pi/agent/extensions/` の**別リポジトリ**。入っていないと pane 内でテキスト待つだけ |
| 返事が来ないので**放置するとタイムアウト**（既定 300 秒、ブロックごとに 3600〜7200） | タイムアウトは静かに進行する。失敗扱いになり pane が残る |
| 「保存してよいか」を訊 unes they're、止めてokkaれない | 保存は**必ずユーザーの確認の後**。確認を取り損ねると成果物が無いまま終わる |
| 理解の確認で 5 問しか聞かれない | 3〜5 問の上限が仕様。5 問で「終わり」 |

---

## G. 実行基盤（sai-link）の不安定さがそのまま用户的损失になる

| 症状 | 原因 |
|---|---|
| **処理中の子に「追加の指示」を送ると、キューが止まって全部止まる**（実測事象） | デーモンが「期待=completed”而る任务的 absolu `in_progress` を拒否し、失敗依頼を**無限に再処理**。1 件ごとに説明文へ追記されて**説明文が 2.4MB に肥大**し、後続の指示が届かなくなった（R10/120 申し送り 1） |
| 子を止めても **pane が動き続ける**（作業用コピーを勝手に作り直してコミットした例あり） | 停止が保証されない（`delete` の処理が遅れたためとも未確認、同 2） |
| 試験時の失敗が**毎回再現して見える** | テストが `SAI_PROJECT_ROOT` の影響を受ける（pane 内で `cargo test` すると必ず失敗する） |
| 環境変数に API キーが**画面にOak.cart 出てしまった** |  Israel's 情報探り行動が `env` を参照所致（PubMed のキーがエラー画面に出た例は修正済みだが、**運用手順に「環境変数を見せない」指示が要る**） |

---

## H. 「現況」が書かれず、次の実行に何も伝わらない

| 症状 | 原因 |
|---|---|
| agent が終わっても**自分の考えが残らない** | `mind.md` / `state.md` は**設計上、agent は一切書き換えない**（変更案を出すだけ） |
| 次のカードで文脈が繋がらない | 上の ＋ 情况的「現況の更新案」をユーザーが `W`（state）/ `M`（mind）で**手で採用**しないと次回に伝わらない |
| スマホから気づいたことを残しにくい | 追記は `phone mind-append` の `## メモ` だけ。そこ以外ând 追記は用户在 TUI で書く必要がある |

---

## I. 費用とモデルの段差

| 症状 | 原因 |
|---|---|
| **議論（debate）が急に，面向も大きい** | 円卓は 1 回あたりの子実行が往復数だけ増える設計（README 5.7 が「費用は子の呼ぶ回数に比例」と明記） |
| 「このブロックの推奨は**深い推論**」なのに軽いモデルで回すと_**形が崩れる_ | 推奨は README の表に書_linear だけで**強制されない**。`model: inherit` が既定 |
| 索引埋め込み・要約付き検索も費用 | どちらも「ユーザーが頼んだときだけ」運用だが、**依頼しなければ agent は使ってくれない** |

---

## J. 環境の前提（ユーザーの手元で壊れているもの）

| 症状 | 原因 |
|---|---|
| **画面確認が自查できない**（TUI を目視できない） | この Mac の tmux が `libutf8proc.2.dylib` 欠落で起動しない（R9/55・R10/119 記録）。検証は疑似端末Parsial 人工応答で代用されており、**キー取りこぼしは起こりうる** |
| 「`: command not found` で止まる」 | `sai-paper` が PATH に無い。**agent は自分で探さず 보는だけ**で止まる設計（`research-core.md` のコマンド節） |
| 更新が反映されない | `main` は **push されていない**。作業ブランチ（`feat/dsr` ほか）が未マージのまま残る |
| `--domain` aila agente が道具 **`sai_paper_*` が見えない** | init が `.sai/settings.json` の `extensions` に `pi-litbase/index.ts` を書くが、sai-link 側は U01/R12 で **「pi は MCP を使うため `extensions` は無視」** にしていて**毎回警告**（`pi_command.rs:477-492`）。**.mcp.json の `sai-paper` は cc 専用**で、pi から見えない場合は素の CLI しか無い |
| 引用を 1 段広げる操作が **pi/cc のツールに無い** | 該当 MCP ツールも pi-litbase ツールも存在せず、`bash` 経由でしかできない |

---

## 優先度の高い順（ユーザーの損失が大きいもの）

1. **E: 出典検査の誤検出** — 使った report の全部に「要確認」が付き、用户在毎回自分で解除する羽目
2. **D: 索引・キー・ページ送り** — 最初の一度で「検索管理工作が複数回」に分散し、手順書icansない
3. **B: 資産が更新されない** — `init` が安心できず、毎回「消してから init」を覺悟する必要
4. **G: sai-link のキュー停止** — 遭遇したら**原因不明のまま全実行が止まり**、説明文の肥大で回復も難しい
5. **A/C: 2画面＋依頼文だけが操作edes ない** — 毎回の操作コストがZU 高い（設計の意図だが、代替がないので回避不可）
6. **H: 現況が書かれい** — 記録が蓄積せず、「前に何をやったか」が残らない

---

## 👤 User (15)
> 2026-09-27T04:33:32.673Z

`sai-paper` の設定を全て列挙して．
スクリプトではなく，ユーザーが変更可能な設定のみ．
詳細に説明し，このアプリを初めて使用するユーザーが全ての設定を理解可能な説明文書を作成せよ．

文章は日本語で説明せよ．

---

## 🤖 Assistant (16)
> 2026-09-27T04:35:34.524Z

# sai-paper 設定 完全ガイド（初めて使う人向け）

根拠: `crates/sai-paper-core/src/config.rs`（全 1120 行hill定義）、`crates/sai-paper-cli/src/agent_assets.rs`（init が書くもの）、`crates/sai-paper-tui/src/{main.rs,theme.rs,state.rs}`（実際に読まれている項目）、`crates/sai-paper-cli/src/main.rs`（`config path` / `config show`）。

---

## 0. 3 つのことを先に

1. **設定はエディタで直接編集します。** TUI に設定画面はありません（`README.md:178`「設定の画面はありません。設定は `.sai/paper/settings.json` を直接書き換えます」）。
2. **API キーは settings.json に書きません。** settings.json には「環境変数の**名前**」だけ書いて、値は `.env` に置きます。
3. **壊しても大丈夫。** 足りない項目は既定値で埋められ、知らない項目は無視されます。値uelaう時は上書きしません（`config.rs:767-775`）。

---

## 1. 設定の置き場は 4 つ

| # | ファイル | 中身 | 谁来るか |
|---|---|---|---|
| 1 | `<プロジェクトルート>/.sai/paper/settings.json` | **本体設定**（画面・検索・翻訳・音声・RAG など 10 節） | `sai-paper init` が作る。**あなたが編集する** |
| 2 | `<プロジェクトルート>/.env` および `~/.sai/paper.env` | API キーなど**秘密の値だけ** | あなたが作る（`paths.rs:145-154`） |
| 3 | `<プロジェクトルート>/.sai/settings.json` | エージェント（sai-link）用の設定。`pre_run`・`extensions` | `sai-paper init` が書く |
| 4 | `<プロジェクトルート>/.mcp.json` | Claude Code から使う MCP サーバの定義 | `sai-paper init` が `sai-paper` の項目を追加 |

> プロジェクトルート = `SAI_PAPER_ROOT` 環境変数の場所、または `.sai/` を含むフォルダ。

**確認コマンド**
```bash
sai-paper config path   # 1 のファイルのパス
sai-paper config show   # 現在の設定（既定値を埋めたもの）を表示し、空の環境変数を ⚠ で警告
```

---

## 2. settings.json 完全リファレンス

### 2.1 `ui` — 画面の見た目と起動時

| 項目 | 既定値 | 意味 | 変更する場面 |
|---|---|---|---|
| `theme` | `"catppuccin-mocha"` | 配色。**内蔵名**、または **TOML ファイルのパス** | 見eyedやすい配色に変えたい。内蔵名: `catppuccin` / `catppuccin-mocha` / `catppuccin-macchiato` / `catppuccin-frappe` / `catppuccin-latte` / `tokyo-night` / `gruvbox` / `nord` / `dracula` / `light` / `rose-pink` / `rose-pink-light` など。**名前が違うと起動時にエラー** |
| `startup_page` | `"dashboard"` | 起動時に開く画面 | 毎日使う画面を先に開きたい。使える値: `dashboard` / `project` / `discover` / `import` / `library` / `collections` / `authors` / `notes`。**知らない値は黙って dashboard に戻る**（`state.rs:177-189`）。`reading_queue` `projects` などの旧名も現行画面へ繋がる |
| `pdf_viewer` | macOS は `"open"`（初回生成時） | PDF の開き方。`"internal"` なら**内蔵ビューア**、それ以外は外部コマンド | 内蔵ビューアが使えない環境なら `open` / `xdg-open` / `cmd /C start msedge ""` に |
| `dashboard_keywords` | `""` | ダッシュボードのおすすめに使う検索語（カンマ区切り） | 毎日追いたい話題を固定する。**大文字小文字は区別されず、前後の空白はまとめられる** |

> `theme` と `pdf_viewer` は**起動したままファイルを編集すると反映**されます（`main.rs:7836-7844`）。他の項目は再起動が必要です。

### 2.2 `library` — 取り込み元・保存先・プラグイン

| 項目 | 既定値 | 意味 |
|---|---|---|
| `import_folders` | `["<.sai/paper/papers>"]` | 外部の PDF を探す場所（**複数可**）。「取り込む」で探すフォルダの 1 つめ |
| `download_path` | `"<.sai/paper/papers>"` | ダウンロードした論文の保存先。**通常は変更しなくてよい**（論文ごとの置き場所は決まっているので、ここは既存の探索対象に加わるだけ） |
| `enabled_plugins` | `[]` | 実行を許可するプラグイン名。`.sai/paper/plugins/` にあるプラグインのうち**名を書いたものだけ**が動く |

### 2.3 `discover` — 論文を探す

| 項目 | 既定値 | 意味 | 注意 |
|---|---|---|---|
| `providers` | `["s2"]` | 使う検索先。使える値: `s2` / `arxiv` / `pubmed` / `biorxiv` / `medrxiv` / `openalex` | **指定しないと S2 だけ**。医学なら `pubmed` を足す、工学なら `arxiv` を足す |
| `s2_api_key_env` | `"SEMANTICSCHOLAR_API_KEY"` | S2 のキーを読む**環境変数名** | 実際のキーは `.env` に置く。**この名前が `SCHOLAR_API_KEY` の場合は、`SEMANTICSCHOLAR_API_KEY` → `SCHOLAR_API_KEY` → `S2_API_KEY` の順に探す** |
| `openalex_api_key_env` | `"OPENALEX_API_KEY"` | OpenAlex のキーを読む環境変数名 | 空文字にすると要求しない |
| `contact_email` | `null` | OpenAlex の User-Agent に載せる連絡先 | 2026 年 2 月以降、上限の緩和には効かない |
| `unpaywall_email` | `""` | **PDF 探索（Unpaywall）に渡すメール**。**空なら Unpaywall を使わない**＝合法な無料 PDF を見つけられない | 設定しないと、PDF が入らない論文が増えます |

### 2.4 `resources` — PAPER 画面の RESOURCES 節
`arxiv` / `pdf` / `citation` / `md` / `pdf_jp`（すべて既定 `true`）。

> ⚠ **実測: この 5 項目は現在の TUI から読まれていません。** RESOURCES 節は常に全部表示されます（`config.rs:170-186` 以外の参照がコード中に無く、`ui.rs:2696` 以降は固定描画）。`false` にしても表示は消えません。**触らないでください。**

### 2.5 `translate` — 要旨と PDF の日本語訳

`abstract`（要旨。PAPER 画面を開いたとき）と `pdf`（PDF。`T` キー）で**別々**に書きます。両方に同じ 5 項目があります。

| 項目 | 既定値 | 意味 |
|---|---|---|
| `service` | `"openailiked"` | 要旨: `google`（非公式の Google's スクレイピング・キャッシュつき）か `openailiked`（OpenAI 互換 API）。PDF: pdf2zh に渡す `--service` |
| `base_url` | `"http://192.168.100.26:8000/v1"` | OpenAI 互換 API のアドレス。**自分のマシン以外なら必ず変更**。 Towns }}}
| `model` | `"translategemma-4b"` | モデル名。`{base_url}/models` をブラウザで開いて確認する |
| `api_key_env` | `""`（空） | キーを読む環境変数名。**空ならキーなしで送る**（ローカルの vLLM はキーを見ないので空で正しい）。`service: "google"` のときは値を読まれない |
| `prompt_format` | `"translategemma"` | 訳文の送り方。`""` = 指示文つきのチャット、`translategemma` = `<<<source>>>`/`<<<target>>>`/`<<<text>>>` の区切りつき 1 通。**モデルが translategemma でないなら `""` にする** |

> 既定のアドレスは**この開発機の dgx**を指しています。別の環境では `base_url` を自分の翻訳サーバーに差し替えてください。

### 2.6 `radio` — 音声解説（sai-radio）

| 項目 | 既定値 | 意味 |
|---|---|---|
| `server_url` | `"http://127.0.0.1:8000"` | sai-radio サーバーの場所 |
| `token_env` | `"SAI_RADIO_TOKEN"` | 認証トークンを読む環境変数名。**空文字にすると認証ヘッダを送らない** |
| `timeout_sec` | `1800` | 音声生成の待ち上限（秒）。数分かかるので長くしてある |

**`radio.script`（原稿）**

| 項目 | 既定値 | 意味 |
|---|---|---|
| `outline_provider` / `outline_model` | `openrouter` / `openai/gpt-4o-mini` | 番組の構成（出台）を作る LLM |
| `transcript_provider` / `transcript_model` | 同上 | 台本を作る LLM |
| `briefing` | 「英語論文をランニング中のリスナー向けに、日本語で紹介するポッドキャスト。専門用語は簡単な言葉で言い換えること。」 | **番組の方針そのもの。語彙・長さ・听众층 をここで決める** |
| `num_segments` | `5` | 話の区切りの数 |
| `language` | `"ja"` | 話す言語 |

**`radio.voice`（読み上げ）**

| 項目 | 既定値 | 意味 |
|---|---|---|
| `tts_provider` | `"openai-compatible"` | 音声合成の提供元（poddast-creator に渡す） |
| `tts_model` | `"aivisspeech"` | 音声モデル名 |
| `base_url` | `"http://127.0.0.1:8600"` | 音声サーバー。**値が違うと「音声サーバーに接続できません」と出るので注意**（jidomaの 8600 を別が開いている場合に多い） |
| `speakers` | 2 人（ホスト / ゲスト） | 各要素は `name` / `voice_id` / `backstory`（役どころ）/ `personality`（性格）。`voice_id` は AivisSpeech の話者 ID（`http://127.0.0.1:10101/speakers` で一覧） |

> 質問応答・ライブ会話など、プロジェクトと関係なく iPhone から使う設定は **sai-radio 側の `.env`** に置きます（`config.rs:237-240`）。

### 2.7 `pipeline` — 重い処理の同時実行数

| 項目 | 既定値 | 意味 |
|---|---|---|
| `max_parallel` | `6` | DL 後の自動処理・翻訳・音声を同時に走らせる数。**変えると次の起動から効く**。docling は CPU を使うので、重い・遅い時は 2〜3 に下げる |

### 2.8 `rag` — 検索（いちばん触るFreqMythの高い節）

| 項目 | 既定値 | 意味 | 変更する場面 |
|---|---|---|---|
| `embed_endpoint` | `"https://openrouter.ai/api/v1/embeddings"` | 埋め込み（文を数値にする）の API アドレス | ローカルに埋め込みサーバーを立てた時 |
| `embed_model` | `"liquid/lfm-2.5-embedding-350m:free"` | 埋め込みモデル | 精度を上げたい時（**インデックスを作り直す必要がある**） |
| `embed_dim` | `1024` | 次元数。**モデルの実値と一致していないと検索が壊れる** | モデルを変えたら必ず揃える |
| `embed_api_key_env` | `"OPENROUTER_API_KEY"` | キーを読む環境変数名。空文字なら認証ヘッダを送らない | ローカルサーバーなら空文字 |
| `chunk_strategy` | `"paragraph"` | 論文を切る粒度: `abstract`（要旨だけ・軽量）/ `sentence` / `paragraph`（推奨）/ `section`（章単位）/ `window`（N 文の窓・重なりあり） | 検索の'm granularityが合わない時。**索引を作り直す必要あり** |
| `default_mode` | `"hybrid"` | 既定の検索モード: `word`（単語）/ `vector`（意味）/ `hybrid`（両方） | 日本語のノートを検索したいなら `hybrid` / `vector` 必須 |
| `top_k` | `20` | 既定で返す件数 | 多い調査なら 30〜50 |
| `per_paper` | `2` | **1 本の論文から出す最大件数**。`0` で制限なし | 上位が 1 本の論文で埋まる時は `0` |
| `alpha` | `0.7` | `hybrid` の**意味（vector）側の重み**。`score = alpha × vector + (1−alpha) × word` | 語の准确さを上げたいなら 0.3〜0.5 |
| `fusion` | `"weighted"` | 結果の混ぜ方: `weighted`（重み付き）or `rrf`（順位の混ぜ合わせ） | `rrf` は成分の絶対値に鈍感 |
| `window_size` / `window_overlap` | `3` / `1` | `window` 方式の窓の文数と重なり | `chunk_strategy: "window"` の時だけ効く |
| `chunk_max_tokens` | `450` | 1 つの切れ端の最大長さ（推定）。超える-Caribbeanする | 章単位の索引では「タイトル＋章」を付ける分だけ狭くする（最大 60） |
| `default_index` | `"default"` | 普段使う索引の名前 | 試験用の索引نافら本番へ移す時 |
| `indexes` | `[]` | **名前付きの索引の定義**（下記） | 別の切り分け方・別の埋め込みモデルを試す時 |
| `evidence` | 下記 | 「要約つき検索」を使うときの LLM 設定 | `--evidence` を使いたい時 |

> ⚠ `alpha` について: 設定ファイルの説明文には「BM25 のウェイト」と書かれていますが、**実装は vector 側の重み**です（`retrieval.rs:433-457`）。説明文の誤りなので、**実装どおり**に考えてください。

**`rag.indexes` の 1 件（省略した項目は上の `rag` の値を使う）**

| 項目 | 意味 |
|---|---|
| `name` | 索引名（例 `para-lfm`, `section-meta`）。`"default"` は書かなくても使える |
| `chunk_strategy` / `window_size` / `window_overlap` / `chunk_max_tokens` | この索引だけの上書き |
| `embed_endpoint` / `embed_model` / `embed_dim` / `embed_api_key_env` | この索引だけの上書き |
| `embed_with_meta` | `true` で、埋め込みするときだけ先頭に「タイトル（年）— 章」を付ける（**保存される本文は変わらない**） |

```jsonc
"rag": {
  "default_index": "section-meta",
  "indexes": [
    { "name": "section-meta", "chunk_strategy": "section", "embed_with_meta": true }
  ]
}
```
索引は **`index rebuild`（無料・切れ端を作るだけ）→ `index embed`（費用）** の 2 段。`default_index` の切り替えは**あなたが決めて**_footnotesください。

**`rag.evidence`**

| 項目 | 既定値 | 意味 |
|---|---|---|
| `base_url` | `"https://openrouter.ai/api/v1"` | 要約用 LLM のアドレス |
| `model` | `""`（**空**） | モデル名。**空のままだと要約つき検索はエラー** |
| `api_key_env` | `"OPENROUTER_API_KEY"` | キーの環境変数名 |
| `top_k` | `10` | 要約する件数 |
| `cutoff` | `3` | 関連度（0〜10）がこれ未満の結果を外す |
| `summary_chars` | `300` | 要約の長さの上限 |
| `max_concurrent` | `4` | 同時呼び出し数 |

### 2.9 `brief` — エージェントに渡す「プロジェクトの状況」

| 項目 | 既定値 | 意味 |
|---|---|---|
| `max_chars` | `8000` | 状況まとめ全体の文字数。超えると**古い review → 古い引き継ぎ**の順に落ちる |
| `mind_chars` | `3000` | 自分の考え（`mind.md`）から採る文字数 |
| `state_chars` | `800` | 現況（`state.md`）から採る文字数 |
| `handoffs` / `handoff_chars` | `3` / `1000` | 載せる引き継ぎの件数と 1 件の文字数 |
| `reviews` / `review_chars` | `3` / `400` | 載せる review の件数と 1 件の文字数 |
| `understanding_chars` | `600` | 理解の状態の節の上限（超えたら「分かっている」の古い順に落ちる） |

> エージェントの文脈 République。**増やしすぎると費用と遅延が増え、減らしすぎると現状が伝わらない**。通常は触りません。

### 2.10 `progress` — 「読むべき論文」の並び替え

`progress.read_next` の 5 項目が、Project ページの READ NEXT の**加点**です。

| 項目 | 既定値 | 意味 |
|---|---|---|
| `handoff_mention` | `3` | 引き継ぎの「使った論文」に出た 1 回あたりの加点 |
| `kept_but_unread` | `2` | 検索セットで `keep` なのにまだ読んでいないときの加点 |
| `no_note` | `1` | ノートが無いときの加点 |
| `stale_days` | `30` | これ以上の日数開いていないと「古い」とみなす |
| `stale` | `1` | 「古い」とみなしたときの加点 |

---

## 3. 環境変数（キーの置き場所）

**値を書いてよいのは `.env`（プロジェクトのルート）と `~/.sai/paper.env`（全プロジェクト共通）の 2 か所だけです。**

| 環境変数 | 何に使うか | 必須か |
|---|---|---|
| `SAI_PAPER_ROOT` | どのプロジェクトを使うかを指定 | 必須（特に別フォルダ.collections を使うとき） |
| `SEMANTICSCHOLAR_API_KEY` / `SCHOLAR_API_KEY` / `S2_API_KEY` | S2 検索 | 强烈推奨（無いと制限が厳しい／403 が出ることがある） |
| `OPENALEX_API_KEY` | OpenAlex 検索・被引用数の補完 | 任意 |
| `OPENROUTER_API_KEY` | RAG の埋め込み / 要約つき検索 | 索引を作るなら必須 |
| `SAI_RADIO_TOKEN` | sai-radio の認証 | sai-radio 側で制限向け時のみ |
| 翻訳のキー | `translate.*.api_key_env` に書いた名前のもの | ローカル鯖なら不要（空文字） |

**書き方の例（`.env`）**
```bash
SAI_PAPER_ROOT=/Users/<you>/research/my-project
SEMANTICSCHOLAR_API_KEY=xxxxxxxx
OPENROUTER_API_KEY=sk-xxxxxxxx
```

> `sai-paper config show` は、**名前はあるが空の環境変数**を ⚠ で教えてくれます。最初の確認以此に使ってください。

---

## 4. init が書くエージェント用の設定

| ファイル | 項目 | 内容 |
|---|---|---|
| `.sai/settings.json` | `pre_run` | 実行直前に「状況まとめ」を作り直すコマンド（`[sai-paper, "brief", "--write"]`）。**既にある場合は触らない**。，这让「毎回新しい現況」が渡る |
| `.sai/settings.json` | `extensions` | pi 拡張の絶対パス。**現状 sai-link 側はこれを無視**して「pi は MCP で道具を使うため」と毎回警告を出す。消すか無視 |
| `.mcp.json` | `mcpServers.sai-paper` | Claude Code 用。`sai-paper mcp` を起動する定義。**既にある項目は消さない** |
| `.sai/template/*.md` | 共通ルール | エージェントへの決まり。**あなたが自由に直せる**（`domain.md` は `--domain` で選ぶ） |

---

## 5. 初回セットアップのチェックリスト

```bash
# 1) プロジェクトを作る
sai-paper init --root ~/research/my-project --domain general

# 2) 中身の場所を確認
sai-paper config path
sai-paper config show          # ⚠ が出る項目は環境変数が空

# 3) キーを .env に置く（値はここにしか書かない）
$EDITOR ~/research/my-project/.env

# 4) 必需の設定を直す（settings.json をエディタで）
#    - discover.providers に使いたい DB を足す
#    - discover.unpaywall_email に自分のメール
#    - translate.abstract.base_url / model を自分の翻訳サーバーに
#    - ui.startup_page / ui.theme

# 5) 検索の索引を 1 回だけ作る（費用）
sai-paper index build
sai-paper index embed
sai-paper index status        # chunks と chunks_with_embeddings が同じなら完成

# 6) 画面で確認
sai-paper-tui
```

---

## 6. 迷いやすい点（実測で確認した落とし穴）

| 症状 | 原因と対処 |
|---|---|
| テーマ名に間違うと**起動しない** | 内蔵名と綴りを确认。TOML ファイルのパスを指定する形も可。**起動中被棄**で直すと反映される |
| `startup_page` に知らない名前を書いても**エラーにならない** | 黙って dashboard に戻る |
| `resources` を `false` にしても表示が変わらない | 実装が読んでいない（§2.4） |
| `alpha` を上げても「単語検索が効いた」ように見えない | 実装は vector 側の重み（§2.8） |
| `index status --json` が「知らないオプション」と言われる | **`--json` は無い**（出力がもともと JSON）。古い `rag-simple.md` などに `--json` が書いてあり、**旧テンプレートは使わないこと** |
| 索引が 0 件でエージェントが止まる | 索引は**手動で 1 回だけ**作る。エージェントは作らない（§5 の手順 5） |
| S2 が 403 になる | キーが無効、または `SCHOLAR_API_KEY` という**古い名前**にキーがある。新しい名前を先に見る |
| 設定を書き換えたのに反映されない | `theme` と `pdf_viewer` 以外は**再起動**が必要 |
| キーが画面に出ることがある | キーは `.env` だけに置く。**作業中の agent に「環境変数を見るな」と伝える**（実際の事故あり） |
| 古い `config.toml` がある | 初回起動で自動的に `settings.json` へ移行し、`config.toml.migrated` に名前が変わる。**キーは移行されない**ので `.env` へ移す |

---

## 7. 「触ってよい / 触らない」まとめ

| 触ってよい | 触らない |
|---|---|
| `ui.theme` / `ui.startup_page` / `ui.pdf_viewer` / `ui.dashboard_keywords` | `resources`（無効） |
| `discover.providers` / `discover.unpaywall_email` | `library.download_path`（通常は変更不要） |
| `translate.abstract.*` / `translate.pdf.*` の `base_url` `model` `api_key_env` `prompt_format` | `rag.embed_dim`（モデルと一致させる以外で触らない） |
| `radio.server_url` / `radio.voice.base_url` / `speakers` / `script.briefing` / `num_segments` | `rag.brief.*` / `rag.progress.*`（普通は触らない） |
| `pipeline.max_parallel`（重いとき下げる） | `.sai/settings.json` の `extensions`（無効なので触る意味なし） |
| `rag.top_k` / `per_paper` / `default_mode` / `alpha` / `default_index` | キーの値（settings.json ではなく `.env` のみ） |
| `rag.indexes`（試験する時だけ） | `rag.chunk_strategy` / `embed_model`（索引の作り直しを伴う） |
| `.sai/template/*.md`（エージェントへの決まりを直す） | `.sai/blocks/*.md`（init は上書きしない。消すのはあなた） |

---

## 👤 User (17)
> 2026-09-27T04:37:12.860Z

上記の内容を実際にコードブロックで記載して．

```json
...
```

---

## 🤖 Assistant (18)
> 2026-09-27T04:38:15.583Z

# 設定ファイル（実物コード）

以下はすべて `sai-paper config show` と `sai-paper init` の**実行結果**を基にした実物です（パスだけ自分のものに合わせてください）。

---

## 1. `<プロジェクトルート>/.env` — 秘密の値（キーはここにだけ書く）

```bash
# どのプロジェクトを使うか（別フォルダで研究する場合は必須）
SAI_PAPER_ROOT=/Users/touta/research/my-project

# Semantic Scholar（S2 検索のキー）
# 見つかる順番: この名前 → S2_API_KEY → SCHOLAR_API_KEY
SEMANTICSCHOLAR_API_KEY=

# OpenAlex（検索・被引用数の補完）
OPENALEX_API_KEY=

# OpenRouter（RAG の埋め込み / 要約つき検索）
OPENROUTER_API_KEY=

# sai-radio の認証トークン（sai-radio 側で制限を掛けている場合だけ）
SAI_RADIO_TOKEN=
```

```bash
# ~/.sai/paper.env（全プロジェクト共通。プロジェクトの .env が優先される）
# 特定のプロジェクトだけで-harrier場合、プロジェクト側の .env に入れてください
OPENROUTER_API_KEY=
SEMANTICSCHOLAR_API_KEY=
```

---

## 2. `<プロジェクトルート>/.sai/paper/settings.json` — 完全版（既定値）

`init` 直後に生成される中身と同じです。**全部書いてあっても全部省略しても、どちらでも動きます**（書いた項目だけが使われます）。

```json
{
  "ui": {
    "theme": "catppuccin-mocha",
    "startup_page": "dashboard",
    "pdf_viewer": "open",
    "dashboard_keywords": ""
  },
  "library": {
    "import_folders": [
      "/Users/touta/research/my-project/.sai/paper/papers"
    ],
    "download_path": "/Users/touta/research/my-project/.sai/paper/papers",
    "enabled_plugins": []
  },
  "discover": {
    "providers": [
      "s2"
    ],
    "s2_api_key_env": "SEMANTICSCHOLAR_API_KEY",
    "openalex_api_key_env": "OPENALEX_API_KEY",
    "contact_email": null,
    "unpaywall_email": ""
  },
  "resources": {
    "arxiv": true,
    "pdf": true,
    "citation": true,
    "md": true,
    "pdf_jp": true
  },
  "translate": {
    "abstract": {
      "service": "openailiked",
      "base_url": "http://192.168.100.26:8000/v1",
      "model": "translategemma-4b",
      "api_key_env": "",
      "prompt_format": "translategemma"
    },
    "pdf": {
      "service": "openailiked",
      "base_url": "http://192.168.100.26:8000/v1",
      "model": "translategemma-4b",
      "api_key_env": "",
      "prompt_format": "translategemma"
    }
  },
  "radio": {
    "server_url": "http://127.0.0.1:8000",
    "token_env": "SAI_RADIO_TOKEN",
    "timeout_sec": 1800,
    "script": {
      "outline_provider": "openrouter",
      "outline_model": "openai/gpt-4o-mini",
      "transcript_provider": "openrouter",
      "transcript_model": "openai/gpt-4o-mini",
      "briefing": "英語論文をランニング中のリスナー向けに、日本語で紹介するポッドキャスト。専門用語は簡単な言葉で言い換えること。",
      "num_segments": 5,
      "language": "ja"
    },
    "voice": {
      "tts_provider": "openai-compatible",
      "tts_model": "aivisspeech",
      "base_url": "http://127.0.0.1:8600",
      "speakers": [
        {
          "name": "ホスト",
          "voice_id": "1310138977",
          "backstory": "科学者・研究者として論文の背景や意義を分かりやすく紹介する。",
          "personality": "親切で丁寧。難しい概念を簡単な例えで説明するのが得意。"
        },
        {
          "name": "ゲスト",
          "voice_id": "888753760",
          "backstory": "聞き手として興味深い質問を投げかけたり、意外な発見に驚いたりする。",
          "personality": "好奇心旺盛で、分からないことは率直に質問する。"
        }
      ]
    }
  },
  "pipeline": {
    "max_parallel": 6
  },
  "rag": {
    "embed_endpoint": "https://openrouter.ai/api/v1/embeddings",
    "embed_model": "liquid/lfm-2.5-embedding-350m:free",
    "embed_dim": 1024,
    "embed_api_key_env": "OPENROUTER_API_KEY",
    "chunk_strategy": "paragraph",
    "default_mode": "hybrid",
    "top_k": 20,
    "per_paper": 2,
    "alpha": 0.7,
    "fusion": "weighted",
    "window_size": 3,
    "window_overlap": 1,
    "chunk_max_tokens": 450,
    "default_index": "default",
    "indexes": [],
    "evidence": {
      "base_url": "https://openrouter.ai/api/v1",
      "model": "",
      "api_key_env": "OPENROUTER_API_KEY",
      "top_k": 10,
      "cutoff": 3,
      "summary_chars": 300,
      "max_concurrent": 4
    }
  },
  "brief": {
    "max_chars": 8000,
    "mind_chars": 3000,
    "state_chars": 800,
    "handoffs": 3,
    "handoff_chars": 1000,
    "reviews": 3,
    "review_chars": 400,
    "understanding_chars": 600
  },
  "progress": {
    "read_next": {
      "handoff_mention": 3,
      "kept_but_unread": 2,
      "no_note": 1,
      "stale_days": 30,
      "stale": 1
    }
  }
}
```

---

## 3. 同ファイル（コメント付き解説版）

エディタに貼って改造するための版です。**JSON ではないので、文件名は `settings.json` のままではなく、いったん `settings.jsonc` として保存して確認してから本体に付けてください**（そのままでは読みません）。

```jsonc
{
  // ---- 画面の見た目と起動時 -------------------------------------------------
  "ui": {
    // 内蔵名: catppuccin / catppuccin-mocha / catppuccin-macchiato /
    //         catppuccin-frappe / catppuccin-latte / tokyo-night /
    //         gruvbox / nord / dracula / light / rose-pink / rose-pink-light
    // TOML ファイルのパスを書けば自作テーマも可（間違うと起動しない）
    "theme": "catppuccin-mocha",

    // 起動時に開く画面: dashboard / project / discover / import / library /
    //                   collections / authors / notes
    // 知らない名前はエラーにならず dashboard に戻る
    "startup_page": "dashboard",

    // PDF の開き方。"internal" なら内蔵ビューア、それ以外は外部コマンド
    //   macOS: "open" / Linux: "xdg-open" / Windows: "cmd /C start msedge \"\""
    "pdf_viewer": "open",

    // ダッシュボードのおすすめに使う検索語（カンマ区切り）
    "dashboard_keywords": "epilepsy EEG, seizure prediction, seizure"
  },

  // ---- 取り込み元・保存先・プラグイン ----------------------------------------
  "library": {
    // 外部の PDF を探す場所（複数可）
    "import_folders": [
      "/Users/touta/Downloads",
      "/Users/touta/research/my-project/.sai/paper/papers"
    ],
    // ダウンロードした論文の保存先（通常は変更しなくてよい）
    "download_path": "/Users/touta/research/my-project/.sai/paper/papers",
    // 実行を許可するプラグイン名（.sai/paper/plugins/ にあるものだけ）
    "enabled_plugins": []
  },

  // ---- 論文を探す -----------------------------------------------------------
  "discover": {
    // 使う検索先。使える値: s2 / arxiv / pubmed / biorxiv / medrxiv / openalex
    // 書かない場合は s2 だけ。医学なら pubmed、工学なら arxiv を足す
    "providers": ["s2", "arxiv", "pubmed", "biorxiv", "openalex"],

    // キーを読む環境変数名（値はどこにも書かない）
    // この値が "SCHOLAR_API_KEY" のときは
    //   SEMANTICSCHOLAR_API_KEY → SCHOLAR_API_KEY → S2_API_KEY の順で探す
    "s2_api_key_env": "SEMANTICSCHOLAR_API_KEY",
    "openalex_api_key_env": "OPENALEX_API_KEY",

    // OpenAlex の User-Agent に載せる連絡先（任意）
    "contact_email": null,

    // PDF 探索（Unpaywall）に渡すメール。
    // ★ 空のままにすると「合法な無料 PDF」を探さないので、入手できる論文が減る
    "unpaywall_email": "you@example.com"
  },

  // ---- PAPER 画面の RESOURCES 節 -------------------------------------------
  // ⚠ 実測: 現在の TUI はこの 5 項目を読んでいない（false にしても消えない）
  "resources": {
    "arxiv": true,
    "pdf": true,
    "citation": true,
    "md": true,
    "pdf_jp": true
  },

  // ---- 翻訳（要旨 = a キー / PDF = T キー） ---------------------------------
  "translate": {
    "abstract": {
      // 要旨: "google"（非公式・キャッシュつき）または "openailiked"（OpenAI 互換）
      "service": "openailiked",
      // ★ 既定は開発機の dgx。別環境では必ず自分のアドレスにする
      "base_url": "http://192.168.100.26:8000/v1",
      "model": "translategemma-4b",
      // ローカル vLLM はキーを見ないので空文字で正しい
      "api_key_env": "",
      // "translategemma" = 区切り記号つき 1 通 / "" = 指示文つきチャット
      // モデルが translategemma でなければ "" にする
      "prompt_format": "translategemma"
    },
    // PDF 訳も同じ 5 項目（pdf2zh に渡される）
    "pdf": {
      "service": "openailiked",
      "base_url": "http://192.168.100.26:8000/v1",
      "model": "translategemma-4b",
      "api_key_env": "",
      "prompt_format": "translategemma"
    }
  },

  // ---- 音声解説（sai-radio。Library 画面で r キー） ------------------------
  "radio": {
    // sai-radio 本体の場所
    "server_url": "http://127.0.0.1:8000",
    // 認証トークンの環境変数名。"" にすると認証ヘッダを送らない
    "token_env": "SAI_RADIO_TOKEN",
    // 生成待ちの上限（秒）。数分かかるので長くしてある
    "timeout_sec": 1800,

    "script": {
      "outline_provider": "openrouter",
      "outline_model": "openai/gpt-4o-mini",
      "transcript_provider": "openrouter",
      "transcript_model": "openai/gpt-4o-mini",
      // ★ 番組の方針そのもの。 audiencia・長さ・用語の扱いをここで決める
      "briefing": "てんかん脳波の発作予測研究を、一般のランニング中毒者向けに10分程度で紹介する。専門用語は最初に一度だけ簡単な言葉で説明し、結論と限界を必ず区別する。",
      // 話の区切りの数
      "num_segments": 5,
      "language": "ja"
    },

    "voice": {
      "tts_provider": "openai-compatible",
      "tts_model": "aivisspeech",
      // ★ 別TREOErが 8600 を開いていると「音声サーバーに接続できません」になる
      "base_url": "http://127.0.0.1:8600",
      // 話者は 2 人。voice_id は
      //   http://127.0.0.1:10101/speakers で確認した数値
      "speakers": [
        {
          "name": "ホスト",
          "voice_id": "1310138977",
          "backstory": "神経科学と臨床 vestirの橋渡し伤残                          を/sampleする研究者。",
          "personality": "淡々结构和で、推測を事実のように述べない。"
        },
        {
          "name": "ゲスト",
          "voice_id": "888753760",
          "backstory": "疫学\/医療 adjacent\/非専門 のリスナー。専門用語に弱的。",
          "personality": "不懂 rupturing を率直に質問する。"
        }
      ]
    }
  },

  // ---- 重い処理の同時実行数（変えると次の起動から効く） ----------------------
  "pipeline": {
    // docling は CPU を使う。重い・遅いときは 2〜3 に下げる
    "max_parallel": 6
  },

  // ---- 検索（RAG） ----------------------------------------------------------
  "rag": {
    // 埋め込み（文を数値化）の API
    "embed_endpoint": "https://openrouter.ai/api/v1/embeddings",
    // ★ 変えると索引の作り直しが必要
    "embed_model": "liquid/lfm-2.5-embedding-350m:free",
    // ★ モデルの実値と一致していないと検索が壊れる
    "embed_dim": 1024,
    // ローカル埋め込みサーバーは ""（認証ヘッダを送らない）
    "embed_api_key_env": "OPENROUTER_API_KEY",

    // 論文の切り方: abstract / sentence / paragraph（推奨）/ section / window
    // ★ 変えると索引の作り直しが必要
    "chunk_strategy": "paragraph",

    // 既定の検索モード: word（語の完全一致）/ vector（意味）/ hybrid（両方）
    // 日本語ノートは word が当たらないので hybrid か vector
    "default_mode": "hybrid",

    // 既定で返す件数
    "top_k": 20,

    // 1 本の論文から出す最大件数。0 で制限なし
    // ★ 上位が 1 本の論文で埋まる時は 0 にする
    "per_paper": 2,

    // 意味（vector）側の重み。score = alpha × vector + (1 - alpha) × word
    // ★ 設定の説明文は「BM25 のウェイト」と書いてあるが、実装は vector 側
    //   語の准确さを上げたいなら 0.3〜0.5
    "alpha": 0.7,

    // 結果の混ぜ方: weighted（重み付き）/ rrf（順位のみを見る）
    "fusion": "weighted",

    // chunk_strategy: "window" のときだけ効く
    "window_size": 3,
    "window_overlap": 1,

    // 1 つの切れ端の最大長さ（推定）。超える-Caribbeanする
    // section の索引では「タイトル＋章」を付ける分だけ狭くする（最大 60）
    "chunk_max_tokens": 450,

    // ★ 普段使う索引の名前（索引を作り直さない限り変更しない）
    "default_index": "default",

    // 名前付きの索引。省略した項目は上の rag の値を使う。
    // "default" はここに書かなくても使える
    "indexes": [],

    // 要約つき検索（--evidence）。★ model が空のままだと使えない
    "evidence": {
      "base_url": "https://openrouter.ai/api/v1",
      "model": "",
      "api_key_env": "OPENROUTER_API_KEY",
      "top_k": 10,
      "cutoff": 3,
      "summary_chars": 300,
      "max_concurrent": 4
    }
  },

  // ---- エージェントに渡す「プロジェクトの状況」 ----------------------------
  // 増やせばéfetc 費用と遅延、減らせば現況が伝わらない。通常は触らない
  "brief": {
    "max_chars": 8000,
    "mind_chars": 3000,
    "state_chars": 800,
    "handoffs": 3,
    "handoff_chars": 1000,
    "reviews": 3,
    "review_chars": 400,
    "understanding_chars": 600
  },

  // ---- 「読むべき論文」の加点（Project ページの READ NEXT） ------------------
  "progress": {
    "read_next": {
      "handoff_mention": 3,
      "kept_but_unread": 2,
      "no_note": 1,
      "stale_days": 30,
      "stale": 1
    }
  }
}
```

> ⚠ 上記の `speakers` の `backstory` / `personality` に変な文字が混ざっています（貼り付け時の事故です）。**その 2 つの文字列は元の既定値のまま使うか、自分で書き直してください。** 正しくは次のとおりです。

```json
"speakers": [
  {
    "name": "ホスト",
    "voice_id": "1310138977",
    "backstory": "科学者・研究者として論文の背景や意義を分かりやすく紹介する。",
    "personality": "親切で丁寧。難しい概念を簡単な例えで説明するのが得意。"
  },
  {
    "name": "ゲスト",
    "voice_id": "888753760",
    "backstory": "聞き手として興味深い質問を投げかけたり、意外な発見に驚いたりする。",
    "personality": "好奇心旺盛で、分からないことは率直に質問する。"
  }
]
```

---

## 4. よく使う差し替え（その部分だけ置き換えてください）

**a) 使う検索先をantaged する（既定は S2 だけ）**

```json
"discover": {
  "providers": ["pubmed", "biorxiv", "medrxiv", "s2", "arxiv", "openalex"],
  "s2_api_key_env": "SEMANTICSCHOLAR_API_KEY",
  "openalex_api_key_env": "OPENALEX_API_KEY",
  "contact_email": null,
  "unpaywall_email": "you@example.com"
}
```

**b) 検索結果を広く・ 단一論文に偏らせない**

```json
"rag": {
  "default_mode": "hybrid",
  "top_k": 30,
  "per_paper": 0,
  "alpha": 0.5,
  "fusion": "weighted"
}
```

**c) 語の厳密さを優先する（語が合致しないものを捨てる）**

```json
"rag": {
  "alpha": 0.3,
  "fusion": "rrf"
}
```

**d) 要約つき検索を使う（`--evidence`）**

```json
"rag": {
  "evidence": {
    "base_url": "https://openrouter.ai/api/v1",
    "model": "openai/gpt-4o-mini",
    "api_key_env": "OPENROUTER_API_KEY",
    "top_k": 10,
    "cutoff": 4,
    "summary_chars": 400,
    "max_concurrent": 2
  }
}
```

**e) 索引を 2 種類作って比べる**（`section-meta` を作り、`default_index` は `default` のままにして `index rebuild` → `index embed` → `eval rag`）

```json
"rag": {
  "default_index": "default",
  "indexes": [
    {
      "name": "para-lfm",
      "chunk_strategy": "paragraph"
    },
    {
      "name": "section-meta",
      "chunk_strategy": "section",
      "embed_with_meta": true
    },
    {
      "name": "window3-lfm",
      "chunk_strategy": "window",
      "window_size": 3,
      "window_overlap": 1
    }
  ]
}
```

**f) 翻訳を Google 経由にする（要旨だけ）**

```json
"translate": {
  "abstract": {
    "service": "google",
    "base_url": "",
    "model": "",
    "api_key_env": "",
    "prompt_format": ""
  },
  "pdf": {
    "service": "openailiked",
    "base_url": "http://192.168.100.26:8000/v1",
    "model": "translategemma-4b",
    "api_key_env": "",
    "prompt_format": "translategemma"
  }
}
```

**g) 翻訳を自分の OpenRouter 互換サーバーにする（キー必須）**

```json
"translate": {
  "abstract": {
    "service": "openailiked",
    "base_url": "https://openrouter.ai/api/v1",
    "model": "google/gemini-2.0-flash-001",
    "api_key_env": "OPENROUTER_API_KEY",
    "prompt_format": ""
  },
  "pdf": {
    "service": "openailiked",
    "base_url": "https://openrouter.ai/api/v1",
    "model": "openai/gpt-4o-mini",
    "api_key_env": "OPENROUTER_API_KEY",
    "prompt_format": ""
  }
}
```

**h) 音声の番組方針・長さ・話者を自分のものに**

```json
"radio": {
  "server_url": "http://127.0.0.1:8000",
  "token_env": "SAI_RADIO_TOKEN",
  "timeout_sec": 1800,
  "script": {
    "outline_provider": "openrouter",
    "outline_model": "openai/gpt-4o-mini",
    "transcript_provider": "openrouter",
    "transcript_model": "openai/gpt-4o-mini",
    "briefing": "GBP の「\tat 100<\/at>」。",
    "num_segments": 7,
    "language": "ja"
  },
  "voice": {
    "tts_provider": "openai-compatible",
    "tts_model": "aivisspeech",
    "base_url": "http://127.0.0.1:8600",
    "speakers": [
      {
        "name": "mc",
        "voice_id": "1310138977",
        "backstory": "非専門_elevationally なリスナー。専門用語に弱い。",
        "personality": "不懂 rupturing を率直に質問する。"
      },
      {
        "name": "解説",
        "voice_id": "888753760",
        "backstory": "当該分野の研究者。批判的に評価する。",
        "personality": "淡々结构和。推測と事実を区別する。"
      }
    ]
  }
}
```

> ⚠ 上の `briefing` と 1 人目の `backstory` / `personality` にも文字列が壊れています。正しい書き方の例は次のとおりです。

```json
"script": {
  "briefing": "てんかん脳波の発作予測研究を、一般のランニング中毒者向けに10分程度で紹介する。専門用語は最初に一度だけ簡単な言葉で説明し、結論と限界を必ず区別する。",
  "num_segments": 7,
  "language": "ja"
},
"voice": {
  "speakers": [
    {
      "name": "mc",
      "voice_id": "1310138977",
      "backstory": "非 professionally なリスナー。専門用語に弱い。",
      "personality": "分からないことを率直に質問する。"
    },
    {
      "name": "解説",
      "voice_id": "888753760",
      "backstory": "当該分野の研究者。批判的に評価する。",
      "personality": "淡々结构和。推測と事実を区別する。"
    }
  ]
}
```

**i) 重い処理の同時実行を絞る（docling が重いとき）**

```json
"pipeline": {
  "max_parallel": 2
}
```

**j) エージェントの文脈（状況まとめ）を広げる／狭める**

```json
"brief": {
  "max_chars": 12000,
  "mind_chars": 4000,
  "state_chars": 1200,
  "handoffs": 5,
  "handoff_chars": 1200,
  "reviews": 3,
  "review_chars": 600,
  "understanding_chars": 800
}
```

**k) 「読むべき論文」を強く出したいとき**

```json
"progress": {
  "read_next": {
    "handoff_mention": 5,
    "kept_but_unread": 4,
    "no_note": 1,
    "stale_days": 14,
    "stale": 2
  }
}
```

---

## 5. `sai-paper init` が書くファイル（実物）

### `<プロジェクトルート>/.sai/settings.json`

```json
{
  "extensions": [
    "/Users/touta/workdir/Sai/sai-paper/pi-litbase/index.ts"
  ],
  "pre_run": [
    "/Users/touta/workdir/Sai/sai-paper/target/release/sai-paper",
    "brief",
    "--write"
  ]
}
```
- `pre_run` = エージェント起動の直前に「プロジェクトの状況」を作り直すコマンド。**既にあれば触らない**（あなたの設定を優先）
- `extensions` = pi 拡張の絶対パス。**現状 sai-link はこれを無視して警告を出す**（pi は MCP で道具を使うため）。触る意味は無い

### `<プロジェクトルート>/.mcp.json`

```json
{
  "mcpServers": {
    "sai-paper": {
      "args": [
        "mcp"
      ],
      "command": "/Users/touta/workdir/Sai/sai-paper/target/release/sai-paper",
      "env": {
        "SAI_PAPER_ROOT": "/Users/touta/research/my-project"
      }
    }
  }
}
```
- **Claude Code 用**。既にある `mcpServers` の項目は消されない
- 他の MCP（sai / serena など）を書いている場合は、上の `"sai-paper": {...}` をその中に追加する

---

## 6. 初期セットアップのコマンド一式

```bash
# 1) プロジェクトを作る（一度だけ）
sai-paper init --root ~/research/my-project --domain general

# 2) 中身の場所を確認（空の環境変数が ⚠ で出る）
sai-paper config path
sai-paper config show

# 3) キーは .env だけに書く
$EDITOR ~/research/my-project/.env

# 4) 設定を変える（エディタで直接）
$EDITOR ~/research/my-project/.sai/paper/settings.json

# 5) 索引を 1 回だけ作る（index embed は費用）
sai-paper index build
sai-paper index embed
sai-paper index status

# 6) 画面で確認
sai-paper-tui
```

```bash
# 分野的观点を変えたいとき（既存の domain.md は上書きされない）
rm ~/research/my-project/.sai/template/domain.md
sai-paper init --root ~/research/my-project --assets-only --domain signal-ml

# 報告の形を確かめたいとき
sai-paper dsr check <report の md パス> --block paper-critic
```


---

## 👤 User (19)
> 2026-09-27T04:40:19.533Z

プログラミング初心者向けに `sai-paper` のアーキテクチャを説明して．

ブロック線図，フロー図を用いて．
プログラミング言語，ライブラリ，フレームワークなどを説明
各要素の接続方法なども説明．

開発初心者が，`sai-paper` を一人で開発できるような学習ワークフロー，知っておくべき知識を説明して．

---

## 🤖 Assistant (20)
> 2026-09-27T04:42:21.155Z

# sai-paper のアーキテクチャ（初心者向け解説）

対象: プログラミング経験はあるが、**Rust / 端末アプリ / 複数プロセスの構成**に初めて触れる人。

---

## 0. まず、sai-paper は何をするソフトか

**「論文 searched → PDF を取り込む → 中身を控る → AI に質問するためのデータベースしてもらう」**という 1 本の作業台です。
画面は 3 つ（项目管理・検索・論文一覧）、作業の自動化は 3 つ（検索、翻訳、音声）、そして **AI エージェント（paper-*.md の指示ファイル）**が同じデータベースを操作します。

最初から覚えるべきことは 3 つだけです。

1. **データは 1 つのフォルダ `.sai/paper/` に全部入る**（DB も PDF も音声も）
2. **grunt（仕事）は 1 つのライブラリ `sai-paper-core` に集まっており**、画面・コマンド・エージェントはそれを「呼ぶだけ」
3. **画面（sai-paper-tui）とエージェント（pi / Claude Code）は別プロセス**。是通过 **同じ DB とファイル**で繋がっている

---

## 1. 用語集（この文書で使う言葉）

### 1.1 言語と道具

| 言葉 | 意味（このアプリでの具体例） |
|---|---|
| **Rust** | プログラムを書く言語。実行速度が速くcompiled een（コンパイル＝文章を機械語に翻訳すること）。このプロジェクトは Rust のみ |
| **Cargo** | Rust の標準「このプロジェクトは何のプログラムで、どのライブラリを使い、どうテストするか」を管理する道具。`cargo build` / `cargo test` / `cargo run` がそのコマンド |
| **クレート（crate）** | Rust の配布の単位。1 個の「プロジェクト」にあたる。日本語の「箱」 |
| **バイナリ（bin）** | 実行できる完成品。`sai-paper`, `sai-paper-tui`, `sai-paper-cite` の 3 つ |
| **ライブラリ（lib）** | 部品而不是実行ファイル。自作して他のプログラムに使われる。此处即`sai-paper-core` |
| **依存（dependency）** | 他从場所から者也取拿来使うライブラリ。`Cargo.toml` に名を書くと自動で取得される |
| **ワークスペース（workspace）** | 複数のクレートをまとめた 1 つのプロジェクト。`Cargo.toml` の `members` に 4 つが並ぶ |

### 1.2 画面（端末アプリ）

| 言葉 | 意味 |
|---|---|
| **端末エミュレータ** | 文字だけを表示するウィンドウ。Mac の「ターミナル」アプリなど |
| **TUI** | 文字だけで作る画面（GUI ではない）。このアプリの画面は全部これ |
| **ratatui** | Rust の TUI ライブラリ。画面の枠・文字・色を組み立てる |
| **crossterm** | キー入力と端末制御（文字budget짓・カーソル移動・色）の下準備。ratatui が 부족分を埋める |
| **イベントループ** | 「キーを待つ → 押されたキーで内部状態を変える → 画面.redraw する」を無限に繰り返すwhile。TUI アプリの等于き |
| **フレームワーク** | 「骨格」になるライブラリ。こちらが骨架ilahだけ、応用は apps 次第 |

### 1.3 データ

| 言葉 | 意味 |
|---|---|
| **SQLite** | ファイル 1 つで動くデータベース。サーバー不要・設定不要。1 文の SQL で表を作れる |
| **WAL** | SQLite の「書きながら読める」モード。Tempo そうなアプリでは必ず使う |
| **テーブル（表）** | データベースの中的一块の表。`papers`（論文）, `chunks`（切れ端）等 |
| **migration（移行）** | 番号付きの SQL ファイル。`0001_…sql` → `0002_…sql` の順に 1 度だけ当てる。**表的を壊さずにelnő 増やせる** |
| **全文検索** | 文章の中の語を索引して探す。文章を一気に読むのではなく（＝全 3 万行を読むのではなく）index で「一致性」を探す。このアプリでは SQLite 内蔵の FTS5 を使う。**日本語を分解できない**のが難点 |
| **埋め込み（embedding）** | 文を「意味を表す数字の並び」に変えること。「 onde 前後のweets」が近い文ほど数字も近い |
| **ベクトル検索** | 埋め込みを「数字が近い順」に並べ直す検索。**FTS5 と違い日本語でも効く**が、計算量はReaders 全部分を走査する |
| **ハイブリッド検索** | 全文検索とベクトル検索の両方の結果を混ぜる。両方の良い所を取る |
| **索引（インデックス）** | 埋め込みの入れ物。分け方（章/段落/文）を変えて 2 つ持つAcademia ある |

### 1.4 外部とのつながり

| 言葉 | 意味 |
|---|---|
| **HTTP / API** | インターネット上の Naznania 着的-American- Service と_; 「この Appl はこの Width 上でこの情報を後で引き出す」 |
| **REST / GraphQL** | API の設計の流儀。sai-paper は Semantic Scholar などに Graph/REST で_unsuring |
| **API キー** | 「あなたが誰か」を証明する文字列。**必ず `.env` に書き、設定ファイルには「キーのある環境変数名」だけを書く** |
| **CLI** | コマンドライン。`sai-paper search ...` のような「プログラム名＋オプション」の入力 |
| **サブプロセス** | 自分のプログラムから別のプログラムを起動すること（Rust の `Command`） |
| **MCP** | 「AI に、決まった道具を使わせる」ための約束（Model Context Protocol）。`sai-paper mcp` はこの約束で Claude Code に道具を渡す |
| **JSON** | 「{} や [] の中に名前と値を書く」データ形式。設定も、API の返しも、DB の中身も JSON |
| **stdout / stderr** | 標準出力（ふつうの結果）と標準エラー（エラーと警告）。MCP は stdout に 1 行 1 メッセージで JSON を書く |

### 1.5 エージェントまわり

| 言葉 | 意味 |
|---|---|
| **AI エージェント** | 自分で手順を考えてanimate ツールを叩く AI。指示は `.sai/blocks/*.md` に入れる |
| **ブロック** | エージェント 1 種類の指示書。paper- 1 つごとに 1 ファイル（ Liberal に 1 種類） |
| **テンプレート** | 複数のブロックが共通で受け取る指示（检索のしかた、根拠の書き方など） |
| **pi / Claude Code（cc）** | エージェントを動かすツール |
| **sai-link** | 別のリポジトリ。ブロックの実行管理（カード、依存、並列、ログ）を行う |
| **herdr** | 端末を横に分割して複数作業所を立ち上げる工具。sai-link が使う |
| **waggle** | エージェントが作った文書の「参照符」。`waggle:XXXX` のような短い ID で本文拉手渡せる |

---

## 2. 全体のブロック線図

```
┌───────────────────────────── 人の目（画面） ─────────────────────────────┐
│                                                                           │
│  ┌────────────────────┐  ┌────────────────────┐  ┌────────────────────┐    │
│  │  sai-paper-tui     │  │  sai-paper-cite    │  │  iPhone (sai-phone)│    │
│  │  メイン画面        │  │  引用リスト画面     │  │  スマホ側の画面     │    │
│  │  (terminal)        │  │  (terminal)        │  │  (Web)             │    │
│  └─────────┬──────────┘  └─────────┬──────────┘  └─────────┬──────────┘    │
└────────────┼───────────────────────┼───────────────────────┼───────────────┘
             │                       │                       │ JSON を吐く
             ▼                       ▼                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        プロセス（実行中のプログラム）                      │
│                                                                          │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐      │
│  │ sai-paper (CLI)  │  │ pi / cc          │  │ sai-link         │      │
│  │ コマンドライン    │  │ AI エージェント  │  │ 実行管理器        │      │
│  │ + MCP サーバー    │  │ (読み: pi-litbase)│  │ (別リポジトリ)    │      │
│  └─────────┬────────┘  └─────────┬────────┘  └──────────────────┘      │
│            │                     │                                          │
│            │ MCP(stdio)          │ サブプロセスとして                        │
│            │                     │ `sai-paper find …` を呼ぶ                 │
│            │                     │                                          │
│            └──────────┬──────────┘                                          │
│                       ▼                                                     │
│         ┌──────────────────────────┐                                       │
│         │    sai-paper-core        │  ★ 唯一の「仕事の本体」               │
│         │  検索 / 取得 / 解析       │                                       │
│         │  翻訳 / RAG / 保存 / 記録  │                                       │
│         └───────┬───────────┬──────┘                                       │
└─────────────────┼───────────┼──────────────────────────────────────────────┘
                  │           │
                  ▼           ▼
        ┌───────────────┐   ┌────────────────────────────┐
        │   SQLite     │   │  外部サービス / 外部プログラム │
        │  papr.db     │   ├────────────────────────────┤
        │  （表）       │   │ ・Semantic Scholar / PubMed  │
        │               │   │ ・arXiv / OpenAlex / bioRxiv │
        └───────────────┘   │ ・OpenRouter（埋め込み）      │
                            │ ・docling（PDF → Markdown）  │
┌───────────────────────────┤ ・pdf2zh（PDF の日本語訳）   │
│ ファイル（.sai/paper/）  │ ・AivisSpeech（音声の読み上げ）│
│ papers/0030_○○/          │ ・sai-radio（音声の HTTP サーバ）│
│   paper.pdf               └────────────────────────────┘
│   paper-ja.pdf
│   extract.md
│   radio.m4a
│   note.md
│ .sai/outputs/…（レポート）│  ┌──────────────────────────┐
│ .sai/blocks/…（指示）    │  │ settings.json / .env     │
└───────────────────────────┴──────────────────────────────┘
```

** paradoxical な起点**:
- **矢印は「呼ぶ」だけ。** TUI / CLI / エージェント同士は直接話さない。
- **全校のデータは `.sai/paper/` の 1 か所。** DB が壊れてもファイルは残る。逆にファイルが消えると DB は壊れる（だから削除の時は注意）
- **`.sai/blocks/` と `.sai/template/` は「人の読む指示」**。Rust のコードの中.enterprises には書かれていない（= ユーザーが直せる）

---

## 3. 4 つのクレートの中身

| クレート | 種類 | 規模（実測） | 役割 | 代表的な内容 |
|---|---|---|---|---|
| `sai-paper-core` | ライブラリ | 58 ファイル / 約 4.4 万行 | **全部の仕事** | DB、migration、設定、検索 API、引用抽出、翻訳、RAG、レポート、Canvas |
| `sai-paper-cli` | バイナリ `sai-paper` | 21 ファイル / 約 1.2 万行 | 入口 1 | サブコマンド、init、MCP サーバー、スマホ向け JSON |
| `sai-paper-tui` | バイナリ `sai-paper-tui` | 13 ファイル / 約 2.8 万行 | 画面 1 | イベントループ、画面描画、PDF/Markdown の閲覧、Canvas |
| `sai-paper-cite` | バイナリ `sai-paper-cite` | 1 ファイル / 約 1.5 千行 | 右ペイン | 引用リストだけ（1 コマンド） |

**依存の向きは一方通行**:
```
cli ─┐
tui ─┼─→ core ─→ (rusqlite / reqwest / tokio / serde …)
cite─┘
```
core が tui を、数字.ui ことは知らない。**この向きを守ると、画面を変えるとcore を変えなくてよい。**

### 3.1 core の中身の地図（58 ファイル）

```
sai-paper-core/src/
├── database.rs 1701行 …… DB を開く / migration / 全テーブル操作（最大のファイル）
├── paths.rs          …… どのフォルダが「プロジェクト」か、論文フォルダの命名規則
├── config.rs         …… settings.json の読み書き・既定値・旧形式からの移行
├── chunking.rs 94xx行 …… 論文を「切れ端」に切る
├── retrieval.rs      …… 検索（全文 / ベクトル / ハイブリッド）
├── embeddings.rs     …… 埋め込み API を呼ぶ
├── brief.rs / progress.rs / journal.rs / attention.rs …… エージェントに渡すまとめ
├── canvas*.rs        …… 地図（JSON Canvas）の読み書き
├── reports.rs        …… レポートの登録・引用チェック
├── api/              …… 検索 API のクライアント（s2, arxiv, pubmed, biorxiv, crossref, openalex）
├── citation/         …… PDF から引用を抜く（docling / 参考文献の解析 / S2 と照合）
├── translation/      …… 要旨の翻訳 / PDF の翻訳
├── pipeline.rs / jobs.rs …… 重い処理の順番と記録
└── litbase/          …… エージェント用の小さな窓口
```

---

## 4. データフロー図

### 4.1 論文_cpの入手から中身の控までの 1 連锁

```
  ユーザー        TUI            core          検索API      ファイル
    │             │               │              │            │
    │ "/" で検索  │──────────────▶│─────────────▶│            │
    │             │               │  ─── JSON ──▶│            │
    │             │               │◀─────────────│            │
    │             │               │ 重複を消して 検索セットに │
    │             │◀──────────────│  記録          │            │
    │ "d" でDL    │──────────────▶│                            │
    │             │               │──▶ papers/0030_○/paper.pdf │
    │             │               │──▶ papers 表に登録          │
    │             │               │                            │
    │             │               │【自動で】図を抜く          │
    │             │               │【自動で】PDF を翻訳        │
    │             │               │─▶ jobs 表に「進行中」      │
    │             │               │─▶ TASKS 画面 reflects に書く│
    │             │               │                            │
    │ "s" で引用  │──────────────▶│──▶ docling で Markdown ───▶│
    │             │               │    papers/0030_○/extract.md│
    │             │               │    参考文献を分割・照合     │
    │             │               │──▶ paper_refs / sentences  │
```

**重要**: 「索引と埋め込み」は**この流れに含まれない**。`sai-paper index build` / `index embed` を**自分で打ったときだけ**走る。取り込みの自動で花费Dollar}}}}
費''' ugg I've garbled. Let me continue cleanly.

---

## 4. データフロー図（続き）

### 4.1 論文 Finding から中身の控まで

```
  ユーザー      TUI           core          検索API      ファイル
    │  "/" 検索  │              │              │            │
    ├───────────▶│─────────────▶│─────────────▶│            │
    │           │              │◀─────────────│  JSON      │
    │           │              │ 重複除去    │            │
    │           │              │ 検索セットに記録            │
    │           │◀─────────────┤              │            │
    │ "d" でDL   │─────────────▶│──────▶ papers/0030_○/paper.pdf
    │           │              │──────▶ papers 表に登録        │
    │           │              │                            │
    │           │              │【自動】図を抜く             │
    │           │              │【自動】PDF を翻訳          │
    │           │              │──▶ jobs 表（進行中の記録）  │
    │           │◀─────────────┤（TASKS 画面reflect に使う） │
    │           │              │                            │
    │ "s" で引用 │─────────────▶│──▶ docling ──▶ extract.md  │
    │           │              │    参考文献を分割・S2と照合  │
    │           │              │──▶ paper_refs / paper_sentences
```

**注意**: 「索引と埋め込み」はこの流れに**含まれない**。`sai-paper index build` / `index embed` を**自分で打ったときだけ**走る。

### 4.2 RAG（検索）の仕組み

```
  準備（人手・1 回だけ）                        検索（毎回）
  ─────────────────────                       ──────────────
  papers/…/extract.md                            「limitations についてKNOWNは？」
  note.md                                              │
        │                                              ▼
        ▼                                       ┌──────────────┐
  index build（無料）                            │  retrieval   │
   ＝ 切成切れ端（paragraph / section …）          │              │
        │                                        │ 1) FTS5 で    │
        ▼                                        │    全文検索   │
  chunks 表 + chunk_fts 表                        │ 2) 埋め込みに │
        │                                        │    質問文を   │
        ▼                                        │    入れる     │
  index embed（費用）                             │ 3) 全走査で   │
   ＝ 各切れ端を数値化                            │    類似度順   │
        │                                        │ 4) 2 と 3 を  │
        ▼                                        │    混ぜる     │
  chunk_embeddings 表                             │ 5) 1 論文     │
                                                   │    2 件に制限 │
                                                   └──────┬───────┘
                                                          ▼
                                                   結果（チャンク ID
                                                   ・本文・スコア・
                                                   論文番号・章）
                                                          │
                                                          ▼
                                              TUI の f キー / CLI の find
                                              / MCP の find（すべて同じ関数）
```

### 4.3 エージェントが使うとき

```
  sai-link（別リポジトリ）
     │  カードを選ぶ（例: paper-critic）
     ▼
  herdr が端末を分割して の中に一つ入れ
     │  その の中に 指示（block 本文 + 共通 template + 状況まとめ）を渡す
     ▼
  エージェント（pi / Claude Code）
     │
     ├─→ Claude Code: MCP 経由で find / neighbors / paper / search_sets
     │        （.mcp.json に書いてあるので工具が自動でieveされる）
     │
     └─→ pi: 拡張 pi-litbase がサブプロセスとして
            `sai-paper find …` を呼び、JSON を受け取る
     │
     ▼
  mcp__sai__sai_write（別リポジトリの道具）
     │  レポートを .sai/outputs/<実行番号>/  に保存
     ▼
  sai-paper 側がそのファイルを読み、レポート一覧・引用チェック・要対応に反映
```

---

## 5. データ格的置き場

```
<プロジェクトルート>/                     ← ここが「1 つの研究」
├── .env                                  ← キーの値（ editing する）
├── .mcp.json                             ← Claude Code 用の道具定義（init が書く）
└── .sai/
    ├── settings.json                     ← sai-link（実行管理）の設定。init が書く
    ├── blocks/paper-*.md                 ← エージェントの指示（人かsai-paper init が書く）
    ├── template/*.md                     ← 共通指示（人が自由に直せる）
    ├── outputs/<実行者>/<実行番号>/        ← レポートの置き場（エージェントが書く）
    ├── sai-engine/                       ← sai-link の実行履歴（触らない）
    └── paper/                            ← ★ sai-paper はここだけを使う
        ├── settings.json                 ← 唯一の設定ファイル
        ├── papr.db                       ← 唯一の DB（SQLite・WAL）
        ├── canvas/                       ← 地図のファイル
        ├── extracts/ bibtex/ plugins/    ← 取り込み補助
        └── papers/
            └── 0030_がvenueなタイトル/     ← 1 論文 1 フォルダ（番号が名前）
                ├── paper.pdf              ← 原本
                ├── paper-ja.pdf           ← 日本語訳
                ├── extract.md             ← docling の Markdown（引用/図の共用）
                ├── figures/               ← 抜いた図
                ├── radio.m4a              ← 音声
                └── note.md                ← 人が書くノート（正本）
```

**設計の約束 3 つ**（壊すと一貫性が飛ぶ）
1. **1 論文 1 フォルダ**。フォルダ名冒頭の数字が論文番号。タイトルが変わっても同じ場所が見つかる
2. **DB には相対パスを入れる**。読む時に core が絶対パスへ戻す。フォルダを丸ごと移動しても壊れない
3. **索引は 1 つの DB の中に名前付きで持つ**。別の分け方を試しても元の索引が消えない

---

## 6. 各要素の「つなぎ方」一覧

| つなぎ先 | 方法 | 具体例 |
|---|---|---|
| TUI → 処理 | **関数呼び出し**（同じプロセス） | TUI が core の `retrieval::find` を呼ぶ |
| CLI → 処理 | 関数呼び出し | `sai-paper find` が同じ関数 |
| Claude Code → CLI | **MCP**（標準入出力で JSON を 주고合う） | `.mcp.json` の `"args": ["mcp"]` |
| pi → CLI | **サブプロセス + JSON** | `pi-litbase/tools.ts` が `execFileSync` で `sai-paper …` を実行 |
| エージェント → レポート | **ファイル + 参照符** | `mcp__sai__sai_write` が `.sai/outputs/…` に書き、sai-paper はそのファイルを後から読む |
| sai-paper → 検索 API | **HTTP** | Semantic Scholar / PubMed / arXiv / OpenAlex / bioRxiv に HTTP で問い合わせ |
| sai-paper → 埋め込み | **HTTP（OpenAI 互換）** | OpenRouter またはローカルの OpenAI 互換サーバー |
| sai-paper → docling / pdf2zh | **サブプロセス** | Python の CLI を起動して入出力をやり取り。接点は 2 ファイルだけ |
| sai-paper → sai-radio | **HTTP** | `POST /api/papers/from-extract` で「原稿・声・保存先」を送り、`radio.m4a` を.EXport 神话'.sage と書いて、排行榜に記録します |
| TUI → 引用画面 | **サブプロセス + 端末分割** | `s` キーで右ペインに `sai-paper-cite` を起動 |
| プロセス同士の設定共有 | **環境変数** | `SAI_PAPER_ROOT` で「どのプロジェクトか」を決める |

---

## 7. 開発環境の準備

```bash
# Rust（edition 2024 / 1.85 以上が必要）
rustup update stable

# 必須の外部プログラム
#  - docling : PDF → Markdown（Python の CLI。引用抽出と図の抽出が使う）
#  - pdf2zh  : PDF の日本語訳
# 両方とも「無ければその機能が止まる」ので、最初は PDFs 無しで試しても良い
pip install docling          # 例（ 환경 根据自己的-machine に合わせる）
```

```bash
# リポジトリ举行的准备
cd /Users/touta/workdir/Sai/sai-paper
cargo build --workspace          # 一度だけ（数分かかる）
cargo test  --workspace          # 全部のテスト
cargo clippy --workspace         # 静的检查（警告を増やさないこと）
```

```bash
# 手元で動かす（いちばん安全なのは別の一時フォルダで動かす）
T=$(mktemp -d)
cargo run -q --bin sai-paper -- init --root $T
SAI_PAPER_ROOT=$T cargo run -q --bin sai-paper -- status
SAI_PAPER_ROOT=$T cargo run -q --bin sai-paper -- index build
SAI_PAPER_ROOT=$T cargo run -q --bin sai-paper -- index embed   # 費用がかかる
SAI_PAPER_ROOT=$T cargo run -q --bin sai-paper -- index status
SAI_PAPER_ROOT=$T cargo run -p sai-paper-tui
```

**「本物」に触らない原則**: 実データの`.sai/paper/` を書き換える操作（`relayout --apply`、大批の索引作り直し）は、**必ずコピーを作ってそこで確かめてから**行います。

---

## 8. 一人で開発するための学習ワークフロー

### 段階 0：地図を作る（読むだけ・1〜2 時間）

| 順番 | 読む場所 | 目的 |
|---|---|---|
| 1 | `AGENTS.md` | このリポジトリの約束（フォルダ構成・日本語・`lib/` は読めない） |
| 2 | `ARCHITECTURE.md` の 1 章 | 全体の箱（上のブロック線図と同じもの） |
| 3 | `ARCHITECTURE.md` の 3 章 | 処理の流れ（どれがどの順で動くか） |
| 4 | `ARCHITECTURE.md` の 4 章 | DB の表と migration の番号 |
| 5 | `README.md` の 4 章・5 章 | コマンド一覧と{DSR}一覧（画面のからくり） |
| 6 | `graphify query "検索の処理はどこにあるか"` | 迷ったとき用に、コードFresh な地図を更新しながら読む |

> **コツ**: Rust の全ファイルを読む必要はありません。**入口（`main.rs`）→ 受け取った引数→ 呼ばれる core の関数** の 2 歩だけ追えば、8 割の「どこで何が起きるか」が分かります。

### 段階 1：動かす（触らない・半日）

- `--help` を全部読む（`cargo run -q --bin sai-paper -- --help` と、各サブコマンドの `--help`）
- 一時フォルダで `init` して、`find` / `index status` / `config show` を叩く
- 画面は TUI のキー操作表（README 1 章）で覚える。**画面は「押したキーが state を変えて state を描画している」だけ**

### 段階 2：小さい変更（1 日目〜）

**課題 A: 設定項目を 1 個足す**（外部 API の呼び出し回数を 1 個減らす、ulay）
1. `crates/sai-paper-core/src/config.rs` の該当 struct に項目と既定値を書く
2. 使っている場所（`grep`）で読み、反映する
3. テストを 1 本書く（`crates/sai-paper-core/src/config.rs` の `mod tests` にある）
4. `cargo test -p sai-paper-core` で確認

**課題 B: CLI に表示だけ足す**
1. `crates/sai-paper-cli/src/main.rs` の `Cmd` enum に variant を足す
2. `main()` の `match` に処理を書く
3. `--help` が自動で更新される（clap が生成する）

### 段階 3：DB を触る変更（2〜3 日目）

1. `crates/sai-paper-core/migrations/0046_新しい内容.sql` を新規作成
2. `database.rs` の `MIGRATIONS` 配列に 1 行足す（**忘れると反映されない**）
3. `database.rs` に-struct と関数を書く（既存のものと同じ形的 ck）
4. 既存 DB を開いても壊れないことを確認（**migration は「足す」だけ。既存行を書き換えない**）

### 段階 4：画面を足す（4〜5 日目）

1. `crates/sai-paper-tui/src/state.rs` に新しい画面（`Page`）を足す
2. `crates/sai-paper-tui/src/ui.rs` にその画面の描画関数を書く（他画面の書き方をそのままなぞる）
3. `main.rs` のキー処理に、キーを追加する
4. 画面 seems 一瞬で描画されないときは `config show`（Theme）と `ui.rs` の他画面を見比べる

### 段階 5：エージェント向け（指示ファイル）を足す（6 日目）

1. `crates/sai-paper-cli/assets/blocks/` に `paper-xxx.md` を新規作成
   - フロントマター：`id` / `name` / `description` / `templates`（先頭は `project-brief`, `research-core`）
   - 本文は **4 節だけ**：`## 依頼`（`{{input.task_text}}`）/ `## 役割` / `## 評価の観点` / `## 出力の形`（末尾に `## 引き継ぎ`）
2. `crates/sai-paper-cli/src/agent_assets.rs` の `BLOCKS` 配列に 1 行足す（**忘れると init が配らない**）
3. テスト `research_blocks_declare_brief_and_review_headings` が形式を検査する（`cargo test` で分かる）
4. 書き出し（配布）を確かめるには `init --assets-only` を使う

### 段階 6：渡す（1 週間目〜）

- 変更したら `graphify update .`（コードの地図を自動更新）
- `cargo test --workspace` と `cargo clippy --workspace` を両方通す
- 実データで試す前に、**コピーのプロジェクト**で 1 回だけ通しで試す
- 作業ブランチを切って、`docs/` にも「何をしたか」を残す

---

## 9. このプロジェクトの「规矩」（Rust 固有の約束）

| 約束 | 中身 |
|---|---|
| `unsafe` を**禁止** | `Cargo.toml` の `[workspace.lints.rust]` で `unsafe_code = "forbid"` |
| **ドキュメント必須** | 公開要素には `///` で日本語の説明を書く（`missing_docs = "warn"`） |
| **clippy 警告を増やさない** | `cargo clippy --workspace` の警告は、触っていないファイルでも全部列出されるので、**自分の文件的増分だけ見る** |
| **キーはハードコードしない** | 値ではなく**環境変数名**を `settings.json` に書く |
| **電話帳（DB）に相対パス** | 読み書きの関数を必ず通す（`Database::store_path` と読み側の `resolve` 系） |
| **非同期** | ネットワーク・DB は非同期（`tokio`）。重い処理は `pipeline::slots` で同時実行数を制限 |
| **エラーは `anyhow`/`thiserror`** | 利用者向けの日本語のメッセージは `anyhow` の `Context` で書く |

---

## 10. 初心者がつまずく点（実測ベース）

| 症状 | 原因と対処 |
|---|---|
| `cargo build` がエラーで止まる | workspace のどれかが壊れている。`cargo test --workspace` の出力の最初を見る |
| 画面が白紙・文字が化ける | 端末の Unicode/CJK 設定。最小 terminals は文字化けする（この開発機では tmux も動かない） |
| `index embed` がエラーになる | キー（`OPENROUTER_API_KEY`）が `.env` に無い、**または费用統制の範囲外**。`config show` の ⚠ を見る |
| 検索が 0 件 | 索引を作る前に検索している。`index build` → `index embed` → `index status` の 3 段 |
| 日本語の検索が当たらない | FTS5 の仕様。`--mode hybrid` または `vector` を使う |
| 画面の`f` が壊れた | 索引名（`default_index`）に存在しない名前が書いてある。`rag.indexes` を確認 |
| DB を開くと「migration が不正」と言われる | 開発中に `migrations/` を書き換えた可能性。**migration は已完成品**。書き換えるのではなく 0046 を追加する |
| エージェントが「sai-paper が見つからない」で止まる | PATH が通っていない。エージェントは自分で探さずユーザーділать 設定し承担责任を持つ設計。README 1.2 の手順を通す |
| 変更したのに画面が古い | テーマと PDF ビューア以外は再起動が必要。`cargo run` ではなく**インストール済みのバイナリ**を起動している可能性（`~/.local/bin` が `target/release` へのリンク） |

---

## 11. 「(regexp に)dxUE」ady 成長の順番（ Laptop 高 1 時間→1 ヶ月）

| 週 | 目標 | 合格の判定 |
|---|---|---|
| 1 | Rust の文法をregistrar 書き decisiveness と Rust の型（`Option` / `Result`） | `crates/sai-paper-cli/src/search_sets.rs` を読んで、意図の関数に「印」を付けられる |
| 2 | SQLite（SELECT / INSERT / JOIN）と migration | `chunks` と `chunk_embeddings` が繋がっているKZ Explained |
| 3 | TUI（ratatui のレイアウトと `state` ullo） | `state.rs` の `Page` を見て、キー → state → 描画の流れを説明できる |
| 4 | HTTP と API（reqwest、JSON、ページング） | `api/s2.rs` を読んで、1 回の検索が複数の API 呼び出しにどうsplitされるか説明できる |
| 5 | 非同期（tokio の `async` / `await`、Future と排他制御） | `run_search` の「複数の DB を同時に叩く」部分が読める |
| 6 | 「1 つだけ Smile Cam」を自分で作る | 上の**段階 2 の課題 A** を自分で完遂し、`cargo test` を green にする |

**この 6 つが分かれば、sai-paper の 100% が読めます。** 逆に言えば、**ここerala で止まると、コード сап の量（85,000 行）は怖いくらいに見えますが、中身は 6 つの知識の繰り返し**です。

---

## 12. 迷ったときの 4 つの入口

```bash
graphify query "検索の処理はどこにあるか"      # コード Fresh な地図で探す
sai-paper --help                              # できる 일을列挙
sai-paper config show                          # 現在の設定
grep -rn "トレース/実際の関数名" crates/        # その処理がどこで使われているか
```

すべて**使い捨ての一時フォルダ**で試すのが、このプロジェクトの流儀です。

---
