# sai-moon 連携ガイド（`crates/`・`apps/radio` を直す人向け）

作成日: 2026-09-27
対象: **`crates/`（Rust 8 クレート）か `apps/radio` のどれかを直す人**。
目的: 直した結果として **`apps/phone`（Web の窓口・8200 番）を壊さないこと**。
全体の話は [`integration.md`](./integration.md)。文中のコマンドはすべて 2026-09-27 に実行して確かめている。

旧 sai-phone の `docs/for-sai-developers.md` を土台に、sai-moon の値（バイナリ名・パス・ポート・置き場）に置き換えたもの。
JSON の項目など API 契約は変えていないので、表の構造は流用できる。

---

## 0. 最初に読む節の早見表

| 直すもの | その節 | 最低限守ること |
|---|---|---|
| **sai-link 側**（`crates/sai-link-*`） | [§2](#2-cratessai-link-)・[§3](#3-プロジェクト一覧とdashboard) | `sai-moon-link phone export` と、書き出す JSON の `schema_version` / `run_id` / `project_name`。加えて `phone tree` / `run` / `md` / `waggle` / `card …` の形、`phone summary` / `phone dashboard` の項目名 |
| **sai-paper 側**（`crates/sai-paper-*`） | [§4](#4-cratessai-paper-) | `phone …` の JSON の項目名と、`target/release/sai-moon-paper` に作り直す手順 |
| **apps/radio** | [§5](#5-appsradio) | `/api/papers` 以下のパスと JSON、論文 ID は**英数字**、Range（206）応答 |
| **apps/phone 自分** | [§6](#6-appsphone-自身を直す人向け) | 画面は `window.SaiRoutes` に登録、`sw.js` の `CACHE` を上げる |

### 共通の約束（どの CLI も同じ）

| | 約束 | 違えたときの画面 |
|---|---|---|
| 成功 | **終了コード 0**・標準出力に **JSON 1 個**（前後に余計な文字を出さない） | そのまま表示 |
| 失敗 | **終了コード 1**・標準出力に `{"error": "<日本語のメッセージ>"}` | 400 で error 文をそのまま表示 |
| 出力が読めない | （事故）JSON でない文字列 | 502「…の応答を読めませんでした」 |
| 起動できない | （事故）実行ファイルが無い・PATH 間違い | 503「…が見つかりません（設定の場所を確認してください）」 |
| 時間切れ | （事故）既定 20 秒（sai-paper 検索 120 秒・訳 90 秒） | 504「…が時間内に応答しませんでした」 |
| 標準エラー | 何を出してもよい（先頭 300 文字がログに残る） | 画面には出ない |

標準出力に `println!` やデバッグ表示を 1 行でも足すと **502 になる**（`apps/phone/src/sai_phone/cli.py`）。

---

## 1. 呼ばれ方の全体像

| アプリ | `apps/phone` から呼ばれる形 | cwd / 環境変数 | 時間切れ |
|---|---|---|---|
| sai-link | `sai-moon-link phone export` / `phone tree` / `phone run` / `phone md` / `phone waggle` / `phone card options` / `card inputs` / `card create` / `phone card run` / `phone summary` / `phone dashboard`、`sai-moon-link daemon <start\|stop>` | **そのプロジェクトの root**（`SAI_PROJECT_ROOT` も同じ root を明示で渡す） | 10 秒（export）・20 秒（daemon 30） |
| sai-paper | `sai-moon-paper phone …` / `content paper … --json` | **そのプロジェクトの root**（プロジェクト名から `papers.resolve_project` で引く）・`SAI_PAPER_ROOT` も同じ値 | 30 秒（訳 90・検索 120・音声化 2000） |
| apps/radio | HTTP（中継 `/api/radio/…` → `127.0.0.1:8201/api/…`） | ― | 質問（`…/ask`）180 秒・他 30 秒 |

- どの呼び出しも **シェルは使わない**（`shell=True` 禁止）。引数は配列で渡される。
- 呼ばれる CLI の場所は環境変数で変えられる（`SAI_LINK_BIN` / `SAI_PAPER_BIN` / `SAI_RADIO_URL`）。**既定値を動く形から変えない。**
- **バイナリ名を変えてはいけない。** 次の名前がそのまま PATH や既定値で使われる。
  `sai-moon-link`（`sai-link` ではない）・`sai-moon-paper`（`sai-paper` ではない）。
  名前の髓を変えると 503 になる。
- **`sai-moon-link` は PATH に無い。** `apps/phone` を起動するときは
  `PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH` を足すか、`SAI_LINK_BIN` に絶対パスを書くこと。
  `SAI_PAPER_BIN` の既定は絶対パス（`target/release/sai-moon-paper`）なので追加は不要。

---

<a id="2-cratessai-link-"></a>
## 2. `crates/sai-link-*`

### 2.1 `apps/phone` が使っているもの

| 使うもの | 場所 |
|---|---|
| `sai-moon-link phone export`（プロジェクトの実行を JSON で書き出す） | CLI: `crates/sai-link-cli/src/cli.rs`（`handle_phone_export`）／本体: `crates/sai-link-engine/src/phone_export.rs` |
| `sai-moon-link phone tree`（左サイドバーと同じツリーを JSON で返す） | CLI: `crates/sai-link-cli/src/phone.rs`（`tree`）／組み立て: `sai_link_widgets` の `left_tree`（TUI と共有）。lane が todo の run ノードは、実行予約があるとき `scheduled_at`（`YYYY-MM-DD HH:MM`）を足す（`apps/phone` のホームが読む） |
| `sai-moon-link phone run <id> --view log\|stats\|history`（run の中身） | 同 `phone.rs`（`run_view`）／`block_log::build`・`cache_stats`・`block_history::build`（`sai-link-widgets`） |
| `sai-moon-link phone md <id> --kind task\|review\|message`（無ければ **終了コード 3・stderr「まだありません」**） | 同 `phone.rs`（`md`） |
| `sai-moon-link phone waggle <id> [--token <t>]`（成果物の一覧と本文。一覧の各項目に task_text に貼れる `ref` がある） | 同 `phone.rs`（`waggle`） |
| `sai-moon-link phone card options`（作成フォームの選択肢） | 同 `phone.rs`（`card_options`）／`sai_link_widgets::card_wizard`（TUI と共有） |
| `sai-moon-link phone card inputs --block <id>`（手順 4 の入力と候補） | 同 `phone.rs`（`card_inputs`） |
| `sai-moon-link phone card create`（**stdin に JSON**・作成後は todo に配置） | 同 `phone.rs`（`card_create`）／`validate_card_spec` + `create_card`（TUI と同じ検査） |
| `sai-moon-link phone card run <run_id>`（新形式 todo の ▶ 実行。未実行カードが無ければ **終了コード 3・stderr「未実行のカードが見つかりません」**） | 同 `phone.rs`（`card_run`） |
| `sai-moon-link phone card schedule <run_id> [--at "YYYY-MM-DD HH:MM"]`（todo の task.md の `scheduled_at:` を書く・省略で消す。常駐が 5 秒ごとに見回って実行する） | 同 `phone.rs`（`card_schedule`）・`daemon.rs`（`run_scheduled_cards`） |
| `sai-moon-link phone card docs [--limit N]`（最近の文書 `{items: [{token, label, description, status, ref}]}`） | 同 `phone.rs`（`card_docs`） |
| `sai-moon-link phone rtree`（research-tree の実行の図。root / seed / branch の 3 層 DAG） | 同 `phone.rs`（`rtree`）／組み立て: `sai_link_widgets` の `research_dag::load_research_dag`（§2.2.2） |
| `sai-moon-link phone rm <run_id>`（実行を消す・子と run フォルダも。実行中は終了コード 1、無ければ 3「実行が見つかりません」） | 同 `phone.rs`（`rm`）・`run_dir::delete_run` |
| `sai-moon-link phone user start <run_id>`（`runner: user` の札を running にする。未実行カードが無ければ **終了コード 3・stderr「未実行のカードが見つかりません」**、`runner: user` で無ければ **終了コード 1**） | 同 `phone.rs`（`user_start`）／本体: `sai_link_widgets::user_run`（TUI の Enter と共通） |
| `sai-moon-link phone user finish <run_id>`（実行中の利用者の札を review へ移す。実行が無ければ **終了コード 3**、`runner: user` で状態 `running` で無ければ **終了コード 1**） | 同 `phone.rs`（`user_finish`）／本体: 同 `user_run`（TUI の `e` と共通） |
| `apps/phone` が呼ぶ側 | `apps/phone/src/sai_phone/control.py`（`phone_tree` / `phone_run_view` / `phone_md` / `phone_waggle` / `phone_card_options` / `phone_card_inputs` / `phone_card_create` / `phone_card_start` / `user_start` / `user_finish`。値の検査と 400 / 404 / 502 の変換もここで行う） |
| 書き出し先 `~/.sai-moon/phone/runs/<project_name>/<run_id>.json` | 同上（一時ファイル → rename で書く） |
| `apps/phone` が読む側 | `apps/phone/src/sai_phone/runs.py`（`load_runs`）・`control.py`（`export_runs`＝60 秒に 1 回だけ呼び直す） |
| 共通定義の置き場 | `crates/sai-link-engine/src/project.rs:79`（`sai_home()`）と `sai_home_path(rel)` |

- 常駐の起動停止と一覧の集計は `sai-moon-link` 自身が持つ（`daemon …` と `phone summary` / `phone dashboard`）。それは §3。
- `sai-moon-link` 自体は **DB を直接開かれていない**（`apps/phone` はこの JSON だけを読む）。

### 2.2 `apps/phone` が読んでいる JSON の項目

`sai-moon-link phone export` が書く 1 ファイル分（`runs.py` の `_read_run`）。**足し算は安全、減らすのは危険。**

| 項目 | 型 | 何に使う | 無い・型が違うとき |
|---|---|---|---|
| `schema_version` | 数値（**1 固定**） | 受け入れ判定 | `1` 以外は**そのファイルを丸ごと捨てる**（ログに 1 行） |
| `run_id` | 数値 | 実行の ID・URL `#/run/<project>/<run_id>` | 無ければ捨てる |
| `project_name` | 文字列 | 一覧の並び・URL | 無ければ捨てる |
| `runner` | 文字列（`cc` / `pi` …） | 記録の読み分け・表示順 | `"user"` は**一覧に出さない** |
| `status` | 文字列 | 一覧に出す判定 | `running / completed / failed / cancelled` のみ表示。**それ以外（todo・pending）は出さない** |
| `started_at` / `updated_at` | 文字列（無い時刻 TZ） | どの日の実行か（**UTC とみて日本時間に変換**）・並び順 | 無ければ日付なしで一覧から落ちる |
| `finished_at` / `duration_ms` | 文字列 / 数値 | 実行ごとの表示 | 空欄でも動く |
| `description` / `block_name` | 文字列 | 一覧の行・検索 | 空文字扱い |
| `parent_run_id` | 数値 \| null | サブタスクを親の直下に字下げ | 通常の位置に出る |
| `model` | 文字列 \| null | 統計画面 | ― |
| `stats` | オブジェクト \| null | 日ごとの費用（`stats.cost_usd.total`・`cost_is_estimate`） | 実行中は `null`（想定内） |
| `agent_session_id` / `transcript_path` | 文字列 \| null | 質問カード・会話の中身 | 会話が読めない（メッセージは空表示） |
| `herdr_pane_id` / `herdr_pane_label` | 文字列 \| null | 実行中の pane への文字送り | 「この実行は終了しています」と出る |

**`sai-moon-link phone user start` / `phone user finish` → `{"run_id": N, "status": "running" \| "review"}`**

| 項目 | 型 | `apps/phone` の使いところ | 無いとき |
|---|---|---|---|
| `run_id` | 数値 | 読む札の画面の行き先 `#/papers/read/<run_id>` | 画面が遷移先を失う |
| `status` | 文字列（`"running"` / `"review"`） | 「始める」で running、「終える」で review へ移ったことを知らせる | 遷移は済んでいるので画面の文言が古くなる |

- **終了コード 3** は 404（`control._phone_json` の決まり）、**終了コード 1** とそれ以外は 502。省略は無い。
- 中身は `sai_link_widgets::user_run`（`start_user_run`・`finish_user_run`・`move_running_to_review`）。TUI も同じ関数を呼ぶので、**片方だけ直さない。**

### 2.2.1 `.sai/settings.json` の項目（agent に渡す環境変数）

settings は 2 か所で書ける。**後者（`~/.sai-moon`）が前者（プロジェクト）をキー単位で上書きする。**

1. `<project>/.sai/settings.json`
2. `~/.sai-moon/settings.json`（**2. が優先**）

読み込みは `crates/sai-link-engine/src/pi_command.rs` の `load_settings()` / `load_settings_quiet()`。
TUI など alternate screen から呼ぶときは `load_settings_quiet()`（警告を stderr に出すと枠外に残る）。

| 項目 | 型 | 何に使う | 壊れる・壊れない |
|---|---|---|---|
| `default_provider` / `default_model` / `default_thinking_level` | 文字列 \| null | ブロックの frontmatter に無いときの既定 | 壊れない |
| `wip` / `max_concurrent` | 数値 \| null | タブごとの pane 上限・同時実行数 | 壊れない |
| `extensions` | 文字列の配列 | 起動時に読み込む拡張 | 壊れない |
| `pre_run` / `context` | 文字列の配列 \| null | 起動直前の準備・作業範囲の準備（シェルは通さない） | 壊れない |
| `env` | オブジェクト（KEY → 値） | **agent の起動コマンドの先頭に付ける環境変数** | 壊れない（壊した項目はその項目だけ破棄される） |

#### `env` の決まり

- キーは `^[A-Za-z_][A-Za-z0-9_]*$`。値は 1 行（`\n` / `\r` を含む値は破棄）・長さ 1〜2000。
- **予約キー** `SAI_PROJECT_ROOT` / `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME` / `SAI_IS_SUBTASK` は書いても破棄される（sai-link 自身が必ず正しい値を入れるため）。
- 通らない項目は**エラーにせず、警告を 1 行出して破棄する**（`extensions` と同じ方針。設定が 1 つ壊れていて実行が止まるほうが困る）。
- 並びは `SAI_*` → `env` → エージェントの実行ファイル。値は shell のシングルクォート 1 つで囲むので、`export` や連結の文字列を書く必要はない。
- **`export` を書かなくてよい。** 接続情報はここ 1 か所に書けば、そのプロジェクトの全 agent（pi / cc / サブタスク）に渡る。

```json
"env": {
  "MLFLOW_TRACKING_URI": "http://127.0.0.1:5001",
  "MLFLOW_S3_ENDPOINT_URL": "http://127.0.0.1:9000",
  "AWS_ACCESS_KEY_ID": "…",
  "AWS_SECRET_ACCESS_KEY": "…",
  "AWS_DEFAULT_REGION": "us-east-1"
}
```

- 実装は `pi_command::agent_env_pairs()`（検証済みの `(String, String)` の並びを返す）と `orchestrator::prepare_pipeline` の 1 か所。`prepare_pipeline` 以外に agent を起動する経路を増やさないこと。
- `env` は 1 か所でも **平文の鍵・合言葉が入るので `.gitignore` にする**（利用者が決める。sai-moon は `.gitignore` を書き換えない）。

### 2.2.2 `layer` キーと `phone rtree`（research-tree の実行の図）

**ブロック定義の `layer` キー（C-01）**

ブロック定義（`.sai/blocks/<id>.md`）の frontmatter に `layer:` を書くと、そのブロックの run が
research-tree の「実行の図」に node として出る。

```yaml
---
id: sci-exp-seed
name: "🌱 exp-seed"
layer: seed        # root / seed / branch のどれか
---
```

- 値は `root` / `seed` / `branch` の 3 つだけ。**書かなければ図に出さない**（`block_metadata.layer` は NULL）。
- 3 つ以外の値は**検査エラーで登録されない**（ブロックごと読み飛ばされる）。
- 保存先は `block_metadata.layer`（TEXT・NULL 可）。読み書きは `orchestrator::upsert_block_metadata` と `SaiStore::{insert_block, update_block, get_block_by_*, list_blocks}`。
- 検査は `validation::validate`（有効な値は `validation::VALID_LAYERS`）。

**`sai-moon-link phone rtree` の JSON（C-02）**

cwd はプロジェクトの root。終了コード 0 で標準出力に JSON を 1 個出す（項目順も契約）。
DB が開けないときは終了コード 4。実行が 0 件でも空の配列で成功する。

```json
{
  "nodes": [
    {"run_id": 120, "block": "sci-exp-seed", "name": "🌱 exp-seed", "layer": "seed",
     "status": "completed", "created_at": "2026-09-30 01:02:03", "finished_at": "2026-09-30 02:00:00",
     "parent_run_id": null, "description": "…", "sci_node": "N12",
     "global_round": 5, "round": 1}
  ],
  "edges": [
    {"from": 120, "to": 121, "kind": "delegation"},
    {"from": 121, "to": 125, "kind": "waggle_token"},
    {"from": 121, "to": 120, "kind": "task_output"}
  ]
}
```

| 項目 | 型 | 何 |
|---|---|---|
| `nodes[].run_id` | 数値 | `execution_history.id`。1 run = 1 node。`run_id` の昇順に並ぶ |
| `nodes[].block` | 文字列 | ブロックの ID（`block_metadata.name`） |
| `nodes[].name` | 文字列 | ブロックの表示名（emoji 可） |
| `nodes[].layer` | 文字列 | `root` / `seed` / `branch` |
| `nodes[].status` | 文字列 | `execution_history.status`（`completed` など） |
| `nodes[].created_at` / `finished_at` | 文字列 / null | 作成・終了時刻 |
| `nodes[].parent_run_id` | 数値 \| null | サブタスクの親 |
| `nodes[].description` | 文字列 | カード（task.md）の frontmatter の `description`（無ければ空文字） |
| `nodes[].sci_node` | 文字列 \| null | カードの frontmatter の `sci_node:`（`N12` の形。カードが無いとき・キーが無いときは null） |
| `nodes[].global_round` / `round` | 数値 \| null | グローバルラウンド / ローカルラウンド |
| `edges[].from` / `to` | 数値 | 実行 ID（**両端が node に入っている線だけ**） |
| `edges[].kind` | 文字列 | `delegation`（DB の `task_delegation`）/ `waggle_token` / `task_output` の 3 つだけ |

- 値が無い項目は `null`（`description` だけは空文字）。
- `edges[]` を作る線は **既存の 3 種だけ**（新しい `connection_type` は作らない）。同じ `(from, to, kind)` は 1 本にまとめる。
- `phone workflow` とは別の図。`phone workflow` は `workflow_moved = true` の run だけとフェーズ合成を出すが、こちらは **`workflow_moved` を見ない**（Kanban 側の run も 1 枚で node にする）。
- 中身は `sai_link_widgets::research_dag::load_research_dag`（`crates/sai-link-widgets/src/research_dag.rs`）。TUI も同じ読み方を共有する。

### 2.3 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| 項目の**追加** | **壊れない**（読まないだけ） |
| `schema_version` を 2 にする | **壊れる**（全ファイルが無視され会話タブが空になる） |
| `run_id` / `project_name` の型変更（文字列化など） | **壊れる**（ファイルごと捨てる） |
| `status` の語の変更（`done` など） | **壊れる**（一覧に出なくなる） |
| フォルダの形 `~/.sai-moon/phone/runs/<project>/<id>.json` の変更 | **壊れる** |
| 書き出しを止める（export を外す） | **壊れる**（会話タブ・Dashboard の「直近の実行」が開けない。`linkable: false` の行ばかりになる） |
| ファイル名の一時ファイル（`.` 始まり）を増やす | **壊れない**（`.` 始まりは無視される） |
| **バイナリ名 `sai-moon-link` を `sai-link` に戻す** | **壊れる**（既定値がそのまま `sai-moon-link` なので 503） |

### 2.4 直したあとに流す確認（読み取りだけ）

```bash
# 0) ビルド
cd ~/workdir/Sai/sai-moon
env -u SAI_PROJECT_ROOT perl -e 'alarm 900; exec @ARGV' cargo build --workspace

# 1) 書き出しが動くか（実際に書かずに JSON を見せる。書き込みはしない）
./target/release/sai-moon-link phone export --dry-run | head -c 300; echo

# 2) 必須項目があるか
./target/release/sai-moon-link phone export --dry-run | python3 -c "
import sys, json
d = json.load(sys.stdin)
row = d[0] if isinstance(d, list) else d
need = ['schema_version','run_id','project_name','runner','status','started_at']
missing = [k for k in need if k not in row]
print('不足:', missing or 'なし'); assert not missing
assert row['schema_version'] == 1, 'schema_version が 1 でない'
print('OK')"

# 3) 既存の書き出しが読めているか（apps/phone が読む場所）
ls ~/.sai-moon/phone/runs/ | head

# 4) 作成フォームの選択肢と手順 4（読み取りだけ）
./target/release/sai-moon-link phone card options | head -c 300; echo
./target/release/sai-moon-link phone card inputs --block sub | head -c 300; echo

# 5) 置き場が ~/.sai-moon になっているか（SAI_SAI_HOME で上書きできること・ファイルは作らない）
SAI_PROJECT_ROOT=$PWD SAI_SAI_HOME=/tmp/smoke-sai-home ./target/release/sai-moon-link phone def list
ls /tmp/smoke-sai-home   # → No such file or directory（作っていないこと）
```

実測結果: `--dry-run` は説明文とオプション（`--run` `--dry-run`）を出力、`~/.sai-moon/phone/runs/` に `sai-moon` 1 プロジェクト分がある。
`SAI_SAI_HOME=/tmp/smoke-sai-home` を渡すと `global_root` がそのパスになり、**そのフォルダは作られない**。

### 2.5 変更したらすること

1. `cargo build`（`apps/phone` は release を叩くので、通常は `cargo build --release`）。
2. 既存の書き出しを 1 回作り直す: `./target/release/sai-moon-link phone export`（プロジェクトの root で）。
   → **常駐が動いていると自動で書き出されるので、再起動不要なことが多い。**
   `phone card …` を直したときも同じ（フォームは都度取りに行くので再起動不要）。`card create` / `card run` は**カードを作る・実行する**ので流さない。
3. `apps/phone` 側を直す場所（項目を増減した場合）: `apps/phone/src/sai_phone/runs.py`。
4. **置き場を変えた場合**は 3 か所すべてを直す（`project.rs:79`・`paths.rs:172`・`registry.py:41`）。
5. この文書と `README.md`・`docs/integration.md` の該当表を更新。

---

<a id="3-プロジェクト一覧とdashboard"></a>
## 3. プロジェクト一覧・Dashboard（sai-link 側）

一覧・Dashboard・常駐の操作はすべて `sai-moon-link` が提供する。以下は `crates/sai-link-*` 側を直す人が守るべき形。

### 3.1 `apps/phone` が使っているもの

| コマンド | 呼び出し側 | 実装場所 |
|---|---|---|
| （登録簿 `~/.sai-moon/registry.json` を直接読む） | `registry.load_registry()` | `apps/phone` 側 `apps/phone/src/sai_phone/registry.py` |
| `sai-moon-link phone summary` | `control._project_summary()`（`GET /api/projects`） | `crates/sai-link-cli/src/phone.rs` の `summary()` |
| `sai-moon-link phone dashboard --period week\|month\|all` | `control.dashboard()` | 同 `phone.rs` の `dashboard()` |
| `sai-moon-link daemon <start\|stop>`（**文字出力**・JSON ではない） | `control.daemon()` | `crates/sai-link-cli/src/cli.rs` の `handle_daemon_start` / `handle_daemon_stop` |

- 3 つとも **cwd = プロジェクトの root**、かつ `SAI_PROJECT_ROOT` も root を明示で渡す
  （`control._phone_env`）。`sai-moon-link` は `SAI_PROJECT_ROOT` を最優先で DB 解決に使うため、
  ここを消すと全プロジェクトが同じ DB を開く。
- `summary` と `dashboard` は **DB が無いプロジェクトで呼ぶと空の DB を作ってしまう**。
  `apps/phone` 側は `registry.load_registry()` の `db_exists` が真のものだけ呼ぶ。
- 一覧（`list_projects`）は登録簿から作る。`name` / `root` / `kind` / `root_exists` /
  `db_exists` / `daemon_alive` は登録簿とファイルの確認だけで決まり、`running` と
  `last_executed_at` だけが `phone summary` から来る。**`summary` の結果は 5 秒だけ覚える**
  （失敗しても一覧は止めない。`running=0` / `last_executed_at=null` にする）。
- 登録簿の並びは `last_opened` の降順、`list_projects` はそのあと `last_executed_at` の
  あるものを新しい順に並べ、無いものを後ろに回す。

### 3.2 `apps/phone` が読んでいる JSON の項目

**`phone summary`（オブジェクト）**

| 項目 | 型 | 使うところ | 無い・違うとき |
|---|---|---|---|
| `running` | 数値 | 「実行中 N」 | 0 |
| `last_executed_at` | 文字列 \| null | 「5分前」と並び順 | 最後尾・時刻なし |

**`phone dashboard`（オブジェクト）** — 画面（`static/project.js`）が読むもの:

| 階層 | 項目 | 型 | 使途 | 無いとき |
|---|---|---|---|---|
| 直下 | `name` / `since` / `until` | 文字列 | 見出しと期間 | ― |
| 直下 | `period` | 文字列 | グラフ横軸のラベルの出し分け | 今週扱い |
| 直下 | `daemon_alive` | 真偽値 | ランプ・ボタン | 停止中 |
| 直下 | `daily` | 配列 | 日ごとの棒グラフ | グラフ空 |
| `daily[]` | `date` / `executions` / `tokens` | 文字列 / 数値 / 数値 | 棒とタップ中の 1 行 | 0 の棒 |
| 直下 | `by_model` | 配列 | ドーナツ | 「データがありません」 |
| `by_model[]` | `model` / `tokens` | 文字列 / 数値 | モデル名と割合 | ― |
| 直下 | `by_block` | 配列 | 横棒（上位 8） | 「データがありません」 |
| `by_block[]` | `block` / `tokens` / `executions` | 文字列 / 数値 / 数値 | 名前・長さ・回数 | ― |
| 直下 | `recent` | 配列 | 直近の実行 | 「この期間の実行はありません」 |
| `recent[]` | `run_id` / `parent_run_id` / `block` / `status` / `started_at` / `duration_ms` | 数値 / 数値 \| null / 文字列 / 文字列 / 文字列 / 数値 | 行の形と `#/run/...` の URL | ― |
| `summary` | `executions` / `completed` / `failed` / `running` / `success_rate` / `tokens_total` / `tokens_input` / `tokens_output` / `tokens_cache` / `duration_ms` / `cost_usd` | 数値 | 2×2 の大きな数字と補足行 | 0 |

`apps/phone` 側が `recent[]` に**追加で足す**項目: `description`（文字列）・`linkable`（真偽値・
メッセージ画面が開けるか）・`project`（文字列）。これは `sai-moon-link` の側では不要。

### 3.3 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| JSON の**項目の追加** | **壊れない** |
| 項目の**削除・名前変更**（`daemon_alive` → `alive` 等） | **壊れる**（ランプ・数字が消える／行が押せない） |
| **型の変更**（数値を文字列に、真偽値を 0/1 に） | **壊れる**（`Number(...)` で見えるものもあるが `bool` 判定は崩れる） |
| 配列をオブジェクトに包む（`{projects: [...]}` に） | **壊れる**（502「応答を読めませんでした」相当） |
| サブコマンド名の変更（`dashboard` → `dash`） | **壊れる**（`sai-moon-link` の起動失敗として 503/504 になる） |
| `--period` の値を減らす／外す | **壊れる**（400 または 502） |
| 標準出力への余計な 1 行（デバッグ表示） | **壊れる**（502） |
| `period` の値を増やす（`year` 等） | **`apps/phone` 側の検査で 400**（`control.py` の `_PERIODS` と画面のボタンも直す） |
| 失敗時の終了コード 1 + `{"error": "…"}` の形を守る範囲の文言変更 | **壊れない**（文言は画面にそのまま出るだけ） |
| 登録簿の `kind` の絞り込みを変える | **壊れる**（一覧に出る・出ないプロジェクトが変わる） |

### 3.4 直したあとに流す確認（実際に動くものは流さない）

```bash
cd ~/workdir/Sai/sai-moon
B=./target/release/sai-moon-link

# 1) サブコマンドが生きているか
$B phone --help | head -20

# 2) summary の形（プロジェクトの root で実行する）
env -u SAI_PROJECT_ROOT $B phone summary | python3 -c "
import sys, json
d = json.load(sys.stdin)
for k in ('running', 'last_executed_at'): assert k in d, k
print('summary OK:', d)"

# 3) dashboard の形
env -u SAI_PROJECT_ROOT $B phone dashboard --period week | python3 -c "
import sys, json
d = json.load(sys.stdin)
for k in ('name','period','since','until','daemon_alive','daily','by_model','by_block','recent','summary'):
    assert k in d, k
for k in ('executions','completed','failed','running','success_rate','tokens_total','duration_ms','cost_usd'):
    assert k in d['summary'], k
print('dashboard OK:', d['since'], '〜', d['until'], '実行', d['summary']['executions'])"

# 4) period 3 つの形が同じか
for p in week month all; do env -u SAI_PROJECT_ROOT $B phone dashboard --period $p | head -3; done
```

`daemon start` / `daemon stop` は**実行すると常駐を起動・停止するので流さない**（画面から押して確認する）。

### 3.5 変更したらすること

1. `cargo build --release`（`apps/phone` は release の実行ファイルを叩く）。**PATH 上の名前 `sai-moon-link` は変えない。**
2. `apps/phone` 側を直す場所: 項目変更 → `apps/phone/src/sai_phone/control.py` と `static/project.js`、期間値の追加 → `control.py` の `_PERIODS`。
3. 動作確認は 8200 番で（[§6.4](#64-確認は別ポート)）。
4. この文書の表を更新。

---

<a id="4-cratessai-paper-"></a>
## 4. `crates/sai-paper-*`

### 4.1 `apps/phone` が使っているもの

| コマンド | 呼び出し側 | 実装場所 |
|---|---|---|
| `phone library` / `phone tree` / `phone paper <id>` / `phone log <id>` / `phone providers` | `papers.py` | `crates/sai-paper-cli/src/phone.rs`（共通化は `crates/sai-paper-core/src/library_tree.rs`） |
| `phone search <query> [--db …] [--year-from …] [--year-to …] [--limit …] -- <語>` | `papers.build_search_args()` | 同 `phone.rs` |
| `phone download <log/import/cite の指定>`（**数分かかる**） | `papers._download_job()`（背景・300 秒） | `crates/sai-paper-cli/src/phone_import.rs` |
| `phone citations <paper_id>`（読み取りだけ） | `papers.py` | `crates/sai-paper-cli/src/phone_cite.rs`（照合は `sai-paper-core/src/citation/library.rs`） |
| `phone citations-extract <paper_id>`（**数分かかる。標準エラーを 1 行ずつ読む**） | `papers._run_paper_command_lines()`（背景） | 同（抽出は core の `citation::ensure_extract`） |
| `phone manual-candidates [--minutes <N>]` | `papers.py` | 同（候補は core の `manual_import`） |
| `phone manual-import <対象> (--file <名前> [--minutes N] | --stdin)` | `papers.py` | 同（登録は core の `manual_import::import_pdf`） |
| `phone after-download <paper_id>`（**数分かかる。標準エラーを 1 行ずつ読む**） | `papers._run_paper_command_lines()`（背景・2000 秒） | 同（core の `pipeline::after_download`） |
| `phone translate <id> [--force]` | `papers.translate()` | 同（訳は `sai-paper-core` の translation） |
| `phone radio <id>`（**数分・終わるまで戻らない**） | `papers._radio_job()`（背景・2000 秒） | 同（core の `podcast_client.rs` → apps/radio `POST /api/papers/from-extract`） |
| `content paper <id> --json`（既存・本文） | `papers.text()` | `crates/sai-paper-cli/src/content.rs` |
| `phone sci status` / `init` / `node` / `apply` / `proposals` / `triage` / `card` / `read` / `finish-read`（`apply`・`card`・`triage` の edit は **標準入力に JSON**） | `sci.py` | `crates/sai-paper-cli/src/phone_sci.rs`（中の処理は `crates/sai-paper-core/src/sci/`） |

- **cwd と環境変数 `SAI_PAPER_ROOT` は「論文の置き場があるプロジェクト」の root**（`papers.resolve_project(name)` が引く一覧の `root`）を必ず付ける（`.env` の API キーを読ませるため）。iPhone からはパスを受け取らない。
- 実行ファイルは **`target/release/sai-moon-paper`**（PATH に無く、既定が絶対パス）。
- 検索語は最後に `--` を置いて渡す（`-` で始まる語を clap がオプションと間違えないため）。
- `phone search` は失敗した DB を**標準エラー**に（`sai-paper search` と同じ `<db>: <理由>` の 1 行）と、**標準出力の `failed`** へ出す（下記 4.2）。
- **`phone after-download` の標準エラーは 1 行 = 1 文**にする。`apps/phone` が `readline()` で読む 2 つの形:
  - `sai-progress step=<n>` — 段。`0 = PDF取得` / `1 = 図抽出` / `2 = 翻訳` / `3 = 完了`。0 と 3 はサーバーが決める（`after-download` は 1 と 2 を知らせる）。
  - `sai-progress msg=<文>` — ログの 1 行。段落内の改行は空白に置き換える。
  - 別の形（`warning: …` など）は進捗ではなくサーバーのログに出る。標準出力の JSON は変えない。
- **`phone citations-extract` の標準エラーは 1 行**で、`sai-progress step=1`（引用の抽出中）だけを出す。段は `0 = PDF確認`（サーバー）→ `1 = 引用抽出` → `2 = 完了`（サーバー）。
- 共通 API キーは **`~/.sai-moon/paper.env`**（`crates/sai-paper-core/src/paths.rs:152`）。プロジェクトの `.env` より優先順位が低い。
- **`phone sci` の 3 つの命令は標準入力で JSON を読む**: `apply`（C-07 の op）と `card`（カードの JSON）は常に読み、`triage` は `--action accept` のときの `edit` だけを読む（**空なら無し**）。

#### 研究方針の検索（`find`・`search`・`set create`）

`apps/phone` からは呼ばないが、**MCP の `find` と `pi-litbase` が読む `--json` の形は変えない**こと。
範囲の変換は `crates/sai-paper-core/src/retrieval.rs` の `resolve_paper_scope` にあり、CLI（`find`）と MCP（`find`）が同じ関数を使う。

```text
sai-moon-paper find <query> … [--papers <番号,…>] [--node <N> [--subtree]] [--each <m>] [--question <Q>] [--widen <理由>]
sai-moon-paper search <query> … [--node <N>] [--question <Q>] [--purpose <文>] [--extract <文>] [--widen <理由>]
sai-moon-paper set create <name> … [--node <N>] [--question <Q>] [--purpose <文>] [--extract <文>]
```

- **`--papers` と `--node` を両方書いたら、両方の論文を合わせた範囲**を探す。`--each <m>` は範囲の論文ごとに上位 m 件を返す（論文番号の昇順、各論文の中はスコアの順）。範囲が無いと `--each` は使えない。
- **墓標の論文**（`sci_tombstones.paper_id`）は、範囲の指定に関係なく結果から除かれる。
- `--node` か `--question` があれば `sci_searches` に 1 行記録される。**`--json` の形は今のまま変えない。**

### 4.2 `apps/phone` が読んでいる JSON の項目

**`phone library` → `{"papers": [...]}`**

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `id` | 数値 | 行を押したときの `#/paper/<name>/<id>` | その行が開けない |
| `title` | 文字列 | 1 行目 | 空行 |
| `authors` | 文字列（`"A, B"`） | 2 行目（最初の 1 人＋「ほか」） | 著者なし表示 |
| `year` / `journal` | 文字列 \| null | 2 行目 | 部分を省略 |
| `reading_status` | 文字列 | 札（`unread/queued/reading/read`） | 札なし |
| `podcast` | 文字列 \| null | 札（`ready` →「🎧 音声あり」／`generating` →「音声作成中」） | 札なし |
| `created_at` / `has_pdf` / `has_extract` / `is_favorite` | ― | **画面では未使用**（返っていれば無くても動く） | ― |

**`phone tree` → `{"groups": [...}`**

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `kind` | `"query"` / `"import"` | 見出しの形（`query` は「検索結果 ›」も出る） | そのまとまりが出ない |
| `log_id` | 数値 \| null | 「検索結果 ›」の行き先 `#/papers/log/<name>/<log_id>` | リンクなし |
| `date` / `query` | 文字列 \| null | 見出しの文言 | ― |
| `hit_count` | 数値 | 「N 件」 | 0 件扱い |
| `paper_ids` | 数値の配列 | 中の論文を引く（絞り込みにも使う） | そのまとまりが空 |
| `source_dbs` | 文字列 | 一覧の見出しでは未使用（検索結果画面で使用） | ― |

**`phone paper <id>` → `{"paper", "files", "note", "refs", "podcast"}`**

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `paper` | `id` `title` `authors` `year` `journal` `doi` `citation_count` `abstract` `abstract_ja` `reading_status` | 各種 | 題名・著者・年・掲載誌・被引用・DOI リンク・要旨の切替 | ― |
| `paper` | `arxiv_id` `s2_paper_id` | ― | **画面未使用** | ― |
| `files` | `pdf` / `pdf_ja` / `extract` | 絶対パス \| null | 「PDF」「日本語 PDF」「本文」ボタン。**パスは表示せず、開けるかだけ見る** | そのボタンが出ない |
| `files` | `note` | ― | 未使用（本文は `note` の文字列を使う） | ― |
| `note` | 文字列 \| null | ノートの本文 | 「ノートはありません」 |
| `refs[]` | `title` `authors` `year` `journal` `doi` `local_paper_id` | 各種 | 引用文献の行。`local_paper_id` があれば Library へ移動 | ― |
| `refs[]` | `ref_no` `s2_paper_id` | ― | 未使用（番号は配列の添字） | ― |
| `podcast` | `status` | `"ready"/"generating"/"failed"` \| null | 音声ボタンの出し分け（`generating` は 10 秒ごとに読み直す） | なし扱い |
| `podcast` | `error` | 文字列 \| null | 「前回は失敗しました: …」 | ― |
| `podcast` | `generated_at` | ― | 未使用 | ― |
| ― | 論文が無いとき | | **終了コード 1 + `{"error":"論文が見つかりません"}`** → 画面は 400 で文言表示 | ― |

**`phone log <id>` / `phone search` → `{"log", "items"}`**

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `log` | `id` | 数値 | 画面を移っても結果を取り直すための鍵 | 検索直後の復元が出来ない |
| `log` | `query` / `date` / `source_dbs` | 文字列 | 見出しの下の小さい行 | ― |
| `log` | `searched_at` / `year` | ― | 未使用 | ― |
| `items[]` | `position` | 数値 | 行を押したときの `#/papers/log/<name>/<log_id>/<position>` | 開けない |
| `items[]` | `title` `authors`（**配列**）`year` `journal` `doi` `abstract` `citation_count` | 各種 | 行と詳細の表示 | ― |
| `items[]` | `local_paper_id` | 数値 \| null | 「✓ Library」の札と行き先 | 未ダウンロード論文の情報画面 |
| `failed[]`（**`phone search` だけ**。`phone log` には無い） | `db` / `reason` | 文字列 | 検索の失敗の帯。`reason` の 429 は「混雑」、403 は「キーが使えない」 | 帯を出さない |

**`phone providers` → `{"default": [...], "available": [...]}`** — 両方**文字列の配列**。`available` が来ないと検索を始められない（502「検索先を読めませんでした」）。`default` は初期選択。

**`content paper <id> --json` → `{"id", "md_path", "text"}`** — 画面は `id` と `text` のみ（`md_path` は渡さない）。

**`phone translate <id>` → `{"id", "abstract_ja"}`** — 画面は `abstract_ja` のみ。

**`phone citations <paper_id>` → `{"paper", "key", "has_pdf", "extracted", "refs"}`**

引用一覧（Citations 画面）。抜き出し済みの論文でも未済みの論文でも同じ形を返す。

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `paper` | `id` / `title` | 数値 / 文字列 | 画面の見出しと戻る先 | ― |
| 直下 | `key` | 文字列 \| null | 引用の抜き出しが要るかを見る（`ArXiv:…` / `DOI:…` / 40hex） | 抜き出し不可 |
| 直下 | `has_pdf` | 真偽値 | 「先に PDF を取り込んでください」の出し分け | ― |
| 直下 | `extracted` | 真偽値 | 引用の行と引用文の出し分け（`refs` が空でも引用文が皆有る） | ― |
| `refs[]` | `ref_no` | 数値 | 行番号と `--ref` に渡す値 | ― |
| `refs[]` | `title` / `first_author` / `year` / `journal` | 文字列 \| null / 文字列 \| null / 数値 \| null / 文字列 \| null | 行の表示 | ― |
| `refs[]` | `doi` / `s2_paper_id` / `raw_text` | 文字列 \| null | 詳細表示 | ― |
| `refs[]` | `local_paper_id` | 数値 \| null | 「✓ Library」の行き先 | 未取り込み扱い |
| `refs[]` | `url` | 文字列 \| null | 開くボタン（無ければ出し分ける） | ボタンなし |
| `refs[]` | `can_download` | 真偽値 | 「取りに行く」ボタン（題名だけの行は S2 の題名検索で ID を探す） | 押せない |
| `refs[]` | `can_import` | 真偽値 | 手動取り込みボタン（`s2_paper_id` か DOI が要る） | 押せない |
| `refs[]` | `sentences[]` | 配列 | その引用を引用している文（`sent_no` の順・`text` は 300 字まで） | ― |
| `refs[]` | `sentences[]` | `sent_no` / `text` | 数値 / 文字列 | ― |
| ― | 論文が無いとき | | **終了コード 1 + `{"error":"論文が見つかりません"}`** | ― |

**`phone citations-extract <paper_id>` → `{"paper_id", "refs", "sentences", "docling_error"}`**

引用を PDF から抜き出す。**標準エラーに `sai-progress step=1` を 1 行出す**。
`docling` が無い・失敗したときも終了コード 0 で、`docling_error` に理由が入る（S2 の引用だけが入る）。

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `paper_id` | 数値 | 続きの処理に渡す論文 | ― |
| `refs` / `sentences` | 数値 | 抜き出した件数（「N 件の引用を抜き出しました」） | 0 件扱い |
| `docling_error` | 文字列 \| null | 「本文からの引用は取れませんでした」の帯 | 帯を出さない |

段（`sai-progress step=`）の決まり: **`0 = PDF確認`（サーバーが出す）` / `1 = 引用抽出` / `2 = 完了`（サーバーが出す）**。
`arXiv ID か DOI が無いとき`・`PDF が無いとき` は終了コード 1 の `{"error": …}`。

**`phone manual-candidates [--minutes <N>]` → `{"dir", "minutes", "items"}`**

ダウンロードフォルダ（`XDG_DOWNLOAD_DIR` → `~/.config/user-dirs.dirs` → `~/Downloads` → `~/ダウンロード`）の最近の PDF。
`--minutes` の既定は 5 分（Web は 30 を渡す）。

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| 直下 | `dir` | 文字列 \| null | 見出しのフォルダ名（ホームを `~` に置き換える） | 候補を出さない |
| 直下 | `minutes` | 数値 | 表示の期間 | ― |
| `items[]` | `name` | 文字列 | 選ぶときのラベル。**フルパスは返らない** | ― |
| `items[]` | `size` / `modified_at` | 数値 / 文字列 | 大きさ・更新時刻（ローカルの時刻つき ISO8601） | ― |

**`phone manual-import <対象> (--file <名前> [--minutes N] | --stdin)` → `{"paper_id", "title", "status", "pdf_path"}`**

ブラウザで保存した PDF（または Web のアップロード）を Library に入れる。

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `paper_id` | 数値 | 続きの `after-download` に渡す論文 | ― |
| `title` | 文字列 | 完了の文 | ― |
| `status` | 文字列（`"imported"`） | 完了の出し分け | ― |
| `pdf_path` | 文字列（絶対パス） | **表示せず**、開けるかだけ見る | ― |

- 対象は 1 つだけ: `--log <id> --position <p>` / `--import-id <id>` / `--cite <paper_id> --ref <ref_no>` / `--paper-id <id>`。
- 中身は 1 つだけ: `--file <名前>`（**ファイル名だけ**。`/`・`\`・`..` は失敗。ダウンロードフォルダの最近 `--minutes` 分の PDF に同じ名前が要る。**移動される**）か `--stdin`（**100MB まで**）。
- 図の抽出と PDF の翻訳は**しない**。Web が続けて `phone after-download <paper_id>` を呼ぶ。
- `--cite` の取り込み先が既に Library にあるときは、**`--file` の元のファイルを動かさず** 終了コード 1（`この論文は既に Library にあります（ID <n>）`）。PDF を付け替えたいときは `--paper-id` を使う。
- `phone download --cite <paper_id> --ref <ref_no>` の出力は `phone download` と同じ形（`exists` / `downloaded`）。

#### 研究方針の木（`phone sci …`）

`apps/phone` の `sci.py` が検査してから呼ぶ。上の共通約束（終了コード 0 と JSON 1 行 / 失敗は `{"error": …}` と終了コード 1）はそのまま。

**`phone sci status` → 木全体**（`sci status --json` と MCP `sci_status` が同じものを返す）

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| 直下 | `initialized` | 真偽値 | 始めていないときの案内 | 常に木を表示してしまう |
| 直下 | `type` | `{"id","name"}` \| null | 見出しの型の名前 | 型名が空欄 |
| 直下 | `seed` | `{"text","parts"}` \| null | 種の文と、未決所の差し込み | 種の文が出ない |
| `seed.parts[]` | `text` / `key` / `node` / `filled` / `tentative` | 文字列 / 文字列 \| null / 文字列 \| null / 真偽値 / 真偽値 | 種の文のどの所が空欄かの印 | 全部同じ見た目 |
| 直下 | `gate` | `{"current","total","name","list"}` \| null | 関門の進み（現在の番号・全体数・名前） | 関門の進みが出ない |
| `gate.list[]` | `name` / `when` / `passed` | 文字列 / 文字列 / 真偽値 | 関門の一覧の 1 行 | 関門の一覧が細かにならない |
| 直下 | `debts` | `{"pending_proposals","unreflected_papers","unscreened_sets","empty_reviews"}` | 負債の 4 つの数字 | 負債が 0 に見える |
| 直下 | `nodes[]` | 配列 | 木そのもの | 木が描かれない |
| `nodes[]` | `id` | 文字列（`N12`） | 節点そのもの、行き先 `#/papers/tree/<id>` | 節点が選べない |
| `nodes[]` | `kind` | `"item"` / `"option"` | 項目の行と候補の行の出し分け | 候補が項目に見える |
| `nodes[]` | `key` | 文字列 \| null | 項目の名前 | ― |
| `nodes[]` | `title` | 文字列 | 行のラベル | ― |
| `nodes[]` | `types` | 文字列の配列 | 選ぶ・並べる・書く・確かめるの印 | 型が分からない |
| `nodes[]` | `state` | 文字列 | 状態の色分け（未決定・仮決定・決定・対象外 / 候補・選択・却下・休眠） | ― |
| `nodes[]` | `fixed` | 真偽値 | 与えられている項目の印 | ― |
| `nodes[]` | `parents` / `depth` / `ord` | 文字列の配列 / 数値 / 数値 | 並び（一番上が 0、その下は深さ優先） | 並びが壊れる |
| `nodes[]` | `value` | 文字列 | 種の文に入る値 | ― |
| `nodes[]` | `blank` | 文字列 \| null（未決定の項目だけ） | 空欄の種類の印（未着手・調査中・情報不足・判断待ち・対立） | 空欄の理由が分からない |
| `nodes[]` | `marks` | 文字列の配列（決定・仮決定の項目だけ） | 印（根拠不足・要再確認） | 印が出ない |
| `nodes[]` | `counts` | `{"questions_open","options","support","refute","papers","pending"}` | 行の小さな数字 | 数が空 |
| 直下 | `edges[]` | `{"kind","from","to"}`（`parent` 以外の線だけ） | 根の図の線 | 線が出ない |
| 直下 | `next[]` | `{"node","block","purpose","expect","why","kind"}` | 「次の一手」の 1 行とカード作成の種 | 次の一手が出ない |
| 直下 | `running[]` | `{"run_id","node","block","owner"}` | 動いている実行の行 | ― |
| 直下 | `seeds[]` | `{"id","title","created_at"}` | 種の一覧 | ― |
| 直下 | `warnings[]` | 文字列 | そのまま見せる注意 | 注意が隠される |
| ― | 始めていないとき | | `type`・`seed`・`gate` は `null`、配列は空、`debts` は全部 0 | ― |

**`phone sci node <N>` → 節点 1 つ**（`sci node <N> --json` と MCP `sci_node` と同じ）

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| `node` | `id` / `title` / `key` / `kind` / `types` / `state` / `blank` / `marks` / `counts` / `value` / `fixed` / `parents` / `depth` / `ord` | status の `nodes[]` と同じ | 板の見出し | ― |
| `node` | `body` | 文字列 | 板の本文（利用者が書く説明） | 本文が空 |
| `guide` | `key` / `name` / `explain` / `condition` / `condition_text` / `condition_met` / `questions[]` / `pitfalls[]` / `extract[]` | 各種 \| null（ガイドが無ければ `null`） | 項目の説明と「埋まった条件」 | 説明が出ない |
| `questions[]` | `id` / `text` / `state` / `answer` / `from_guide` | 文字列 / 文字列 / 文字列 / 文字列 / 真偽値 | 問いの行。`from_guide` が false の行は「人が足した問い」 | ― |
| `options[]` | `id` / `title` / `state` / `body` / `support` / `refute` / `papers` | 各種 | 候補の行 | 候補が整理されない |
| `findings[]` | `id` / `node` / `text` / `cites[]` / `polarity` / `level` / `source` / `report_id` / `paper_id` / `run_id` / `question` / `auto` / `proposal` / `created_at` | 各種 | 知見の行（`polarity` で向き、`cites` で出典、`auto` で自動反映の印） | 知見が整理されない |
| `links[]` | `id` / `node` / `target` / `title` / `role` / `read_state` / `auto` / `proposal` / `reason` / `created_at` | 各種 | 結び付きの行（論文・report・ノート・検索セット・実行） | 結び付きが出ない |
| `decisions[]` | `action` / `option` / `reason` / `evidence[]` / `rejected[]` / `counter_searched` / `actor` / `created_at` | 各種 | 決定の記録の履歴 | 履歴が出ない |
| `actions[]` | `run_id` / `block` / `owner` / `runner` / `purpose` / `created` | 各種 | 節点のための実行（カード）の行 | ― |
| `funnel` | `searched` / `screened` / `read` / `reflected` | 数値 | 検索の出口 | ― |
| `pending[]` | `id` / `op` / `payload` / `reason` / `run_id` / `block` / `created_at` | 各種 | 仕分け待ちの行 | ― |
| `blocks[]` | `block` / `purpose` / `expect` / `why` | 文字列 | カード候補の行（3 つまで） | ― |
| `axes[]` | 文字列 | 配列 | 候補の表の見出し（mind.md の「価値の軸」） | 軸の見出しが出ない |
| `searches[]` | `query` / `scope` / `hits` / `created_at` | 各種 | 節点に結び付いた検索の履歴 | ― |

**`phone sci read <run_id>` → 読む札**（`sci read <run_id> --json` と同じ）

| 階層 | 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|---|
| 直下 | `run_id` | 数値 | 読む画面の行き先 `#/papers/read/<run_id>` | 開けない |
| 直下 | `owner` | 文字列 | 札。まだ todo なら「始める」を出す | ― |
| 直下 | `node` | `{"id","title"}` | 読む札の節点名 | ― |
| 直下 | `purpose` / `role` / `depth` | 文字列 | 読む目的・論文の役割・深さ | ― |
| 直下 | `paper` | `{"id","title","year"}` | 論文情報 | ― |
| 直下 | `questions[]` | 文字列の配列 | 答える問いの入力欄 | ― |
| 直下 | `findings[]` | `{"id","text","polarity"}` | この札で書いた知見の一覧 | ― |

**`phone sci card` が見る要求の JSON（標準入力）** — 画面が作るもの。`apps/phone` が先に検査し、知らない項目は落とす。

| 項目 | 型 | 使うところ | 無いとき |
|---|---|---|---|
| `node` | 文字列（`N12` の形） | どの節点のためのカードか。**必須** | 400 |
| `block` | 文字列（`^sci-[a-z-]{2,30}$`） | どのブロックを動かすか。**必須** | 400 |
| `purpose` | 文字列（1〜300 字） | 依頼文。**必須** | 400 |
| `question` | 文字列（`Q7` の形） | 答える問い | 問いなし |
| `expect` | 文字列（300 字まで） | 依頼文の「期待する成果」 | ― |
| `scope` | 文字列（`node` / `path` / `subtree` / `siblings` / `summary` / `blind`） | 渡す範囲 | 既定の範囲 |
| `assume` | 文字列の配列（5 個まで・各 `N12=値`） | 仮置き | 仮置きなし |
| `mode` | 文字列（**任意**。`purpose_first` 目的先行 / `blind_first` 目的後付け。空なら付けない） | 読む札の読み方 | 既定（目的先行） |
| `papers` | 整数の配列（20 個まで） | 対象論文 | 対象なし |
| `runner` | 文字列（`pi` / `cc` / `user`・既定 `pi`） | 実行主体 | `pi` |
| `role` / `depth` | 文字列 | 論文の役割と読む深さ | ― |
| `origin_run` | 数値 | 親にする実行 | ― |

`runner` が `user` のときは `papers` が 1 つで、`role` が必須。**ファイルのパスは受け取らない。**

**その他の応答**

- `phone sci proposals` → `{"proposals": [...]}`。1 件は `node` の `pending[]` と同じ形に `"node": "N12"` を足したもの。
- `phone sci card` → `{"run_id": N, "path": "…/task.md"}`。
- `phone sci finish-read <run_id>` → `{"run_id": N, "findings": 2, "reflected": true}`（理由で終えたときは `"findings": 0`）。
- `phone sci init --type <型>` → `{"type","created_items","created_edges","warnings","backup","assets"}`。**`assets` は配った部品の 1 行ずつの文字列の配列**（無いファイルは書かない）。
- 書き込む命令（`init` / `apply` / `triage` / `card` / `finish-read`）は、終わる前に `export::export` を呼ぶ。`apps/phone` は書き込みのあと画面を読み直します。

### 4.3 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| JSON の**項目の追加** | **壊れない** |
| サブコマンド名の変更（`phone` → `mobile`） | **壊れる**（503「`sai-moon-paper` が見つかりません」） |
| 項目の削除・名前変更（`reading_status` → `status`） | **壊れる**（札・ボタンが消える、絞り込み不能） |
| `authors` を**配列→文字列**やその逆に | **壊れる**（著者名が `A,B` とそのまま／`A, B` のように壊れて表示） |
| `local_paper_id` を**無いとき `0`** にする | **壊れる**（`0` が「ある」とみなされて `#/paper/<name>/0` へ行く） |
| `podcast` をオブジェクトに包む（`{"status": ...}`） | **壊れる**（Library の「🎧 音声あり」が消える。論文情報側は `podcast` がオブジェクト前提なので**両方揃えて直す**） |
| 論文が無いときの `{"error": "論文が見つかりません"}` の変更 | 文言が変わるだけ（**終了コード 1 の形は必ず守る**） |
| 標準出力への余計な表示 | **壊れる**（502） |
| **`phone sci` の項目の名前・値の名前（C-01 の `blank`・`marks`・`state`・`next` の `kind` など）を変える** | **壊れる**（Web の研究方針の画面が壊れる。`scitree.js`・`sciedit.js`・`sciread.js`・`scigraph.js` が名前で読んでいる） |
| `phone sci card` の標準入力を読まない | **壊れる**（カードが作れない。`apply`・`triage` の `edit` も同じ） |
| `phone search` の `--` なし渡し・`--db` の ID の増減 | 増減は `providers` の `available` と**同時に**変える（`apps/phone` は `available` で検査して 400 にする） |
| `SAI_PAPER_ROOT` 無しでも動く形にする（`.env` を読まない） | 検索・音声化が壊れる（API キー不足） |
| **バイナリ名 `sai-moon-paper` を `sai-paper` に戻す** | **壊れる**（既定値が絶対パス `…/sai-moon/target/release/sai-moon-paper` なので起動しない） |

### 4.4 直したあとに流す確認

```bash
# 0) 作り直し（必ず release。PATH 上には無い）
cd ~/workdir/Sai/sai-moon
perl -e 'alarm 1800; exec @ARGV' cargo build --release
B=./target/release/sai-moon-paper
export SAI_PAPER_ROOT=$PWD
# cwd もルート（apps/phone と同じ条件）

# 1) サブコマンドが生きているか
$B phone --help | head -20

# 2) providers（available が無ければ検索が 502 になる）
$B phone providers

# 3) 一覧・階層・詳細（読み取りだけ）
$B phone library | head -c 300; echo
$B phone tree     | head -c 300; echo
$B phone paper 49 | python3 -c "
import sys, json
d = json.load(sys.stdin)
for k in ('paper','files','note','refs','podcast'): assert k in d, k
for k in ('id','title','authors','abstract'): assert k in d['paper'], k
print('paper OK / podcast =', d['podcast'])"

# 4) 本文
$B content paper 49 --json | head -c 200; echo

# 5) 失敗の形（終了コード 1）
$B phone paper 999999; echo "exit=$?"     # {"error":"論文が見つかりません"} exit=1

# 6) 研究方針の木（書き込みは一時フォルダの中だけで行う）
TMP=$(mktemp -d)
(cd "$TMP" && SAI_PAPER_ROOT="$TMP" "$B" init --sci --type problem) # 一時フォルダの中に木を作る
(cd "$TMP" && SAI_PAPER_ROOT="$TMP" "$B" phone sci status | head -c 300; echo)
(cd "$TMP" && SAI_PAPER_ROOT="$TMP" "$B" phone sci node N1 | head -c 300; echo)
rm -rf "$TMP"
```

**流さないこと**: `phone radio`（数分の生成）、`phone search`（検索履歴が 1 件増える。履歴を増やさずに済むのは `--help` だけ）、`phone translate`（DB に訳を書き込む）、`phone sci init`（本物の `papr.db` と `.sai/` を書く。上の 6 のように **一時フォルダの中で**行う）。

実測結果: `providers` = `{"available":["s2","pubmed","biorxiv","medrxiv","openalex","arxiv"],"default":["arxiv","s2"]}`、`phone paper 999999` = `{"error":"論文が見つかりません"}` `exit=1`。

### 4.5 変更したらすること

1. **`cargo build --release`**（`apps/phone` は release の実行ファイルを叩く。直しても作り直さないと古いまま）。
2. `apps/phone` 側を直す場所: JSON の形が変わったら `apps/phone/src/sai_phone/papers.py` と `static/papers.js`。
3. 8200 番で画面確認（[§6.4](#64-確認は別ポート)）。
4. 置き場を変えた場合は `paths.rs:172` の `sai_home()` と `project.rs:79`・`registry.py:41` をそろえる。
5. この文書の表を更新。

---

<a id="5-appsradio"></a>
## 5. `apps/radio`

`apps/phone` は **HTTP の中継しかしない**（`apps/phone/src/sai_radio/` は開かない。合言葉は `Authorization: Bearer` でだけ付ける）。

| 呼ばれる API | 実装場所 | 画面 |
|---|---|---|
| `GET /api/papers` | `apps/radio/src/sai_radio/main.py` `api_list_papers` | Radio 一覧 |
| `GET /api/papers/{id}` | `api_get_paper` | 再生画面（題名・文字起こし `sections`） |
| `GET /api/papers/{id}/audio`（**Range に対応**） | `api_get_audio`（`FileResponse`） | 再生・ダウンロード |
| `POST /api/papers/{id}/listen` | `api_listen` | 最初の再生で 1 回 |
| `POST /api/papers/{id}/favorite` | `api_toggle_favorite` | ★ |
| `GET /api/stats` | `api_stats` | カレンダー |
| `GET /api/logs?paper_id=` | `api_logs` | 質問の履歴 |
| `POST /api/papers/{id}/ask` | `api_ask` | 文字の質問（中継の時間切れ 180 秒） |
| `POST /api/papers/{id}/live-session` | `api_live_session` | 声の質問の合鍵（iPhone は Google へ直結） |
| `POST /api/papers/{id}/question-log` | `api_question_log` | 声の質問の記録 |
| `POST /api/papers/from-extract` | `api_from_extract` | **`apps/phone` からは中継禁止（403）**。`sai-moon-paper` が直接呼ぶ |
| `POST /api/papers/upload` | ― | **中継禁止（403）・削除済み**（`tests/test_main_routes.py` が確かめている） |

- 中継先は `SAI_RADIO_URL`（既定 **`http://127.0.0.1:8201`**）。合言葉 `SAI_RADIO_TOKEN` は `apps/phone` 側にも同じ値を設定する。
- 中継で **パスの文字は `A-Za-z0-9_./-` のみ**（それ以外・`..` は 400）。`..` を含むパスや `/` の増減で新しい API を足すと通らないことがある。
- **画面（`static/`）と PDF アップロードは削除済み。** `127.0.0.1:8201` で動く前提で読む。残っているのは API だけ。
- TTS プロキシ（`tts_proxy.py`）は 8202 番の別プロセス。使わなければ起動しなくてよい。

### 5.2 `apps/phone` が読んでいる JSON の項目（`static/radio.js`）

| API | 項目 | 型 | 使うところ | 無い・違うとき |
|---|---|---|---|---|
| `GET /api/papers` | `id` | **英数字の文字列** | 行の URL `#/radio/<id>`・ダウンロード判定 | 行が開けない |
| 同 | `title` | 文字列 | 1 行目・絞り込み | 「(無題)」 |
| 同 | `created_at` | 数値（UNIX 秒） | 「9/22」表示と新着順 | ― |
| 同 | `favorite` | 真偽値 | ★ | ― |
| 同 | `duration` | 数値 \| null | 「長さ 23:10」 | 長さの表示なし |
| 同 | `mode` | ― | **未使用** | ― |
| `GET /api/papers/{id}` | `title` / `sections` | 文字列 / 配列 | 題名・文字起こし（`sections[]` は `start` `end` `title`） | 「無題の論文」・文字起こしなし |
| `POST …/favorite` | `favorite` | 真偽値 | 星の反映 | 反映されない（開き直すと直る） |
| `GET /api/stats` | `listened_papers` | 数値 | 「これまでに聴いた論文 N 本」 | 0 |
| 同 | `by_day` | 配列（`date` 文字列・`count` 数値） | カレンダー | 空 |
| `GET /api/logs?paper_id=` | `created_at`（UNIX 秒） / `kind` / `payload.position_sec` / `payload.question` / `payload.answer` | 各種 | 質問の吹き出し | 行が出ない（`kind=="question"` のみ表示） |
| `POST …/ask` | `answer` | 文字列 | 回答の吹き出し | 「考え中…」が残る |
| `POST …/live-session` | `token` / `ws_url` / `system_instruction` / `model` | 文字列 | Google への直結 | 声の質問が失敗（エラー文を表示） |
| 見つからないとき | `{"detail": "論文が見つかりません"}`（**FastAPI 形**・`{"error"}` ではない） | | 再生画面は「音声が見つかりません」 | ― |

### 5.3 壊れる変更・壊れない変更

| 変更 | 結果 |
|---|---|
| JSON の**項目の追加** | **壊れない** |
| パスの変更（`/api/papers` → `/api/audio-list`） | **壊れる**（一覧が開けない） |
| `id` を**数値に変える**／ID に `-` `.` 以外の記号を足す | **壊れる**（`isRadioPaperId` を通らない。**ID は英数字という前提**。数字だけと当て込むのも壊れる: `4ee6474932af` や `demo001` が実在する） |
| `duration` を文字列にする | **壊れる**（`formatSec` が `NaN:NaN` を出す） |
| `sections[]` の `start`/`end` の削除 | **壊れる**（文字起こしの位置が飛べない） |
| 音声ファイルへの **Range（206・`Content-Range`）応答をやめる** | **壊れる**（Safari の途中からの再生ができなくなる） |
| `audio` の `Content-Type` を `audio/mp4` 以外の不定なものに | 再生できないことがある（`.m4a` は `audio/mp4` に統一されている） |
| `from-extract` の入力形（`paper_id` / `title` / `markdown`）の変更 | **壊れる**（音声化の依頼が全て失敗） |
| 音声生成が長時間ブロックする書き戻し | **壊れる**（`asyncio.to_thread` を外さない。`tests/test_ingest_nonblocking.py` が防ぐ） |
| **ポートを 8201 以外に変える** | **`apps/phone` 側が繋がらない**（既定 `SAI_RADIO_URL=http://127.0.0.1:8201`。変えないなら `SAI_RADIO_URL` も変える） |

### 5.4 直したあとに流す確認（読み取りだけ）

```bash
cd ~/workdir/Sai/sai-moon

# 1) 一覧と統計（8201 は本プロセス。8198 などで別の番号で確認してもよい）
curl -s localhost:8201/api/papers | head -c 300; echo
curl -s localhost:8201/api/stats  | head -c 200; echo

# 2) Range（206 が返らなければ再生が壊れる）
ID=$(curl -s localhost:8201/api/papers | python3 -c "import sys,json;print(json.load(sys.stdin)[0]['id'])")
curl -s -D - -o /dev/null -H 'Range: bytes=0-99' "localhost:8201/api/papers/$ID/audio" | head -6
# → 206 Partial Content / content-type: audio/mp4 / content-length: 100

# 3) 見つからない ID（404 が FastAPI 形であること）
curl -s -o /dev/null -w '%{http_code}\n' localhost:8201/api/papers/999999    # 404

# 4) テストと import（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)
PYTHONPATH=apps/radio/src apps/radio/.venv/bin/python -c "import sai_radio.main"

# 5) 動作確認は別ポートで（8201 は再起動しない）
PYTHONPATH=apps/radio/src apps/radio/.venv/bin/uvicorn sai_radio.main:app --host 127.0.0.1 --port 8198 &
curl -s localhost:8198/api/papers | head -c 200; echo
kill %1
```

**流さないこと**: `POST …/ask` と `/live-session`（LLM・Google を実際に使う）、`from-extract`（数分の生成）、`purge_non_paper_audio.py --apply`（音声を消す。確認だけなら `--apply` なし）。

実測結果: テスト 3 本は「すべて通りました」（`test_ingest_nonblocking.py`・`test_main_routes.py`・`test_podcast_request_settings.py`）。

### 5.5 変更したらすること

1. **8201 は再起動しない**（動いている間に直すのは中身だけで、確認は 8198 など別ポート）。
2. 画面を復活させるなら、`apps/phone` の中継（`radio.py`）の禁止パスの判定を同時に直す。
3. `apps/phone` 側を直す場所: パス・項目の変更 → `apps/phone/src/sai_phone/radio.py`（中継の検査）と `static/radio.js`。
4. 「長い論文が音声化できない」等の制限を直したら `integration.md` の注意点を更新。
5. `apps/radio/README.md`・`apps/radio/ARCHITECTURE.md` も更新。
6. この文書の表を更新。

---

<a id="6-appsphone-自身を直す人向け"></a>
## 6. `apps/phone` 自身を直す人向け

### 6.1 画面を足す手順

1. `static/index.html` に `<section id="view-…" class="view" hidden>` を置く。
2. 画面ファイル（例 `static/hoge.js`）の先頭〜末尾で登録する:

   ```js
   (window.SaiRoutes = window.SaiRoutes || []).push({
     match(hash) { const m = hash.match(/^#\/hoge\/(.+)$/); return m ? { id: m[1] } : null; },
     view: "view-hoge",
     tab: "project",        // "project" | "agent" | "papers"
     hideTabbar: false,     // 入力欄がある画面は true
     interval: 10000,       // 定期更新 ms（0 で無効）
     async open(params) { /* 最初の読み込み */ window.SaiShell.ok(); },
     async poll(params) { /* 定期更新 */ },
   });
   ```

3. `index.html` で **`app.js` より前**に `<script src="/hoge.js"></script>` を読む。
4. `static/sw.js`: `CACHE` を 1 つ上げ（今は `sai-phone-v28`）、`ASSETS` に `/hoge.js` を足す。**上げないと iPhone に古い画面が残る。**
5. `app.js` の分岐（`parseRoute`）は増やさない（`window.SaiRoutes` の登録だけで通る）。

### 6.2 共通部品

| 部品 | 場所 | 役割 |
|---|---|---|
| `apiFetch(path, opts)` | `static/app.js` | 合言葉（`Authorization: Bearer`）を自動で付ける fetch |
| `window.SaiShell.ok() / .fail()` | `static/app.js` | 通信の成否で帯（`.offline-bar`）を出し入れ |
| `renderMarkdown(text)` | `static/markdown.js` | md の整形表示（見出し折りたたみ・表・コードの枠・frontmatter） |
| `attachVoiceInput(btn, ta, onGrow)` | `static/app.js` | 🎤 音声入力の部品 |
| `parseListHash` `listHash` `el` `formatTokens` など | `static/linkview.js` / `static/stats.js` | List の住所・要素作り（作成フォームもこれを使う） |
| `formatTokens` `formatDuration` `formatMoney` `svgEl` `el` | `static/stats.js` | 数字の整形・SVG 作成（グローバル関数） |
| `run_json` / `run_cli` | `src/sai_phone/cli.py` | CLI 呼び出しとエラー変換（503/504/400/502）。**JSON 契約はここで守られる** |
| `require_token` | `src/sai_phone/main.py` | 合言葉認証（`?token=` も可） |
| `.fold-item` `.bubble-user` `.bubble-agent` `.inputbar` | `static/style.css` | 会話・入力欄の見た目 |
| CSS 変数（`--bg` `--text` `--accent` `--danger` …） | `static/style.css` | **色はこれだけ使う**（ダークモード自動対応） |

### 6.3 テストの流し方（毎回個別に実行する）

```bash
cd ~/workdir/Sai/sai-moon

# JS（19 本。全部通ると "test_…: すべて通りました"）
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done

# Python（17 本。PYTHONPATH が必要）
(cd apps/phone && export PYTHONPATH=src && for f in tests/test_*.py; do .venv/bin/python "$f" || exit 1; done)

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)

plutil -lint apps/phone/launchd/com.sai-moon.plist
```

- `SAI_PROJECT_ROOT` などが混ざっていると Rust の History テスト 1 件が落ちる。
  `cargo test` は `env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME` を付ける。
- `apps/phone` の Python テストは **`PYTHONPATH=src` が無いと `sai_phone` が import できない**。

<a id="64-確認は別ポート"></a>
### 6.4 確認は 8200 / 8198

**旧構成の 8100 番（旧 sai-phone）と 8000 番（旧 sai-radio）は再起動しない。**

```bash
cd ~/workdir/Sai/sai-moon
PATH=$HOME/workdir/Sai/sai-moon/target/release:$PATH \
PYTHONPATH=apps/phone/src SAI_PHONE_TOKEN= SAI_PROJECT_ROOT= SAI_PARENT_RUN_ID= SAI_BLOCK_NAME= \
  apps/phone/.venv/bin/uvicorn sai_phone.main:app --host 127.0.0.1 --port 8200 &

curl -s localhost:8200/api/health            # {"ok":true,...}
curl -s localhost:8200/api/projects | head -c 300; echo   # プロジェクト一覧（sai-moon が出ること）
curl -s localhost:8200/api/paper-projects | head -c 200; echo
curl -s localhost:8200/api/radio/papers | head -c 200; echo
kill %1
```

失敗の形も環境変数で再現できる（**確認後に必ず元の環境変数で戻す**）:

```bash
# CLI が無い → 503
SAI_LINK_BIN=/nonexistent-sai-moon-link … uvicorn … --port 8200
curl -s localhost:8200/api/projects/sai-moon/tree
# {"detail":"sai-moon-link が見つかりません（設定の場所を確認してください）"}  503

# apps/radio が無い → 503
SAI_RADIO_URL=http://127.0.0.1:59999 … uvicorn … --port 8200
curl -s localhost:8200/api/radio/papers
# {"detail":"Mac の apps/radio が応答しません"}  503

# PATH が無い / CLI が JSON でない応答を返す → 502（SAI_PAPER_BIN=/bin/echo で再現）
# CLI が error JSON で終了コード 1 → 400（error 文がそのまま出る）
```

---

<a id="7-連携が壊れたときの見分け方"></a>
## 7. 連携が壊れたときの見分け方

### 7.1 iPhone の画面に出る文言 → 原因

| 画面の文言（`detail` の中身） | 状態 | 原因と直し方 |
|---|---|---|
| `…が見つかりません（設定の場所を確認してください）` | 503 | CLI の場所。`SAI_LINK_BIN` / `SAI_PAPER_BIN` と PATH（launchd は plist の `PATH`）を確認。**バイナリ名やサブコマンド名を変えても同じ文が出る** |
| `Mac の apps/radio が応答しません` | 503 | `apps/radio`（8201）が止まっている／`SAI_RADIO_URL` 間違い |
| `…の応答を読めませんでした` | 502 | 終了コード 0 なのに JSON でない（**標準出力への余計な表示**を疑う） |
| CLI の error 文（例 `論文が見つかりません` `プロジェクト '…' が見つかりません（registry に未登録）`） | 400 | CLI が失敗した。文言は相手側の `{"error"}` のまま |
| `period は week / month / all のいずれかにしてください` `action は start / stop を指定してください` `検索先が分かりません: …` など | 400 | **`apps/phone` 自身の検査**。相手側を疑わない |
| `view は log / stats / history のいずれかにしてください` `kind は task / review / message のいずれかにしてください` `run_id は 1 以上の整数にしてください` `token の指定が正しくありません` | 400 | **List の `apps/phone` 自身の検査**。`sai-moon-link phone …` を呼ぶ前に弾いている |
| `block の指定が正しくありません` `task_text は 100KB までです` `runner は user / pi / cc / gl のいずれか（空でスキップ）` `thinking は none / …` `トークンは waggle: 形式で指定してください` `global_round は数字にしてください（空でスキップ）` `tag は 64 文字までの 1 行にしてください` | 400 | **作成フォームの `apps/phone` 自身の検査**（`control._validate_card_payload`・CLI を呼ぶ前） |
| `まだありません` | 404 | その run に md（task / review / message）が無い（`phone md` の終了コード 3） |
| `未実行のカードが見つかりません` | 404 | その run の未実行カードが無い（`phone card run` の終了コード 3・▶ 実行） |
| `sai-moon-link` の stderr の先頭（例 `実行が見つかりません: ID9999`） | 502 | `phone …` が失敗した。理由は相手の stderr の先頭 200 文字。ログ（7.2）に全文が出る |
| `プロジェクトが見つかりません` / `このプロジェクトにはまだ実行の記録がありません` | 404 | 登録簿に無い／`db_exists` が偽 |
| `PDF がありません` / `音声が見つかりません` | 404 | ファイル・音声が無い（相手側の 404） |
| `この操作は apps/phone からは使えません` | 403 | 中継先で `papers/upload` / `papers/from-extract` を呼ぼうとした（仕様） |
| `認証エラー` | 401 | 合言葉が合わない（`SAI_PHONE_TOKEN` / `SAI_RADIO_TOKEN`） |
| `…が時間内に応答しませんでした` | 504 | 時間切れ（既定 20 秒。重い操作は個別の時間切れ） |
| 画面の帯 `接続できていません` | 通信失敗 | 8200 自体に届かない（サーバー・tailscale・ネットワーク） |
| プロジェクト一覧が **空** | ― | `SAI_SAI_HOME` が `~/.sai-moon` を指しているか、`~/.sai-moon/registry.json` にそのプロジェクトがあるか確かめる |

### 7.2 ログの見方

| どこを見る | 中身 |
|---|---|
| **`apps/phone`**: `apps/phone/src/data/sai-moon.log`（launchd 起動時）／起動したターミナル（手動起動時） | `sai-moon-link の標準エラー: …`（先頭 300 文字）、`最小集計に失敗（一覧は続けます）: …`、`sai-moon-paper の応答を読めませんでした` の前の標準エラー、書き出し失敗、音声化の例外。**CLI の失敗原因はまずここ** |
| 手動起動したとき | そのターミナルの stderr |
| **apps/radio** | 起動しているターミナル。`sai-moon-paper phone radio の標準エラー`（`apps/phone` 側）と、apps/radio 自身の例外・LLM のエラー |
| **crates/sai-paper-*・crates/sai-link-*** | 単発の CLI なので、`apps/phone` のログに流れた標準エラーが手がかり。直接試すときは手元で `…; echo $?` |
| Safari（iPhone / 開発者ツール） | ネットワークの応答の `detail` を見る（上の表と突き合わせる） |
| 音声化の状態 | sai-paper の DB の `paper_podcasts`（`generating` / `ready` / `failed` + `error`）。論文情報画面にも「前回は失敗しました: …」が出る |

### 7.3 直す順番

1. 画面の `detail` 文から **503 / 502 / 400 / 404 のどれか**を決める。
2. 503 → 場所（PATH・設定・常駐）。502 → **出力の混入**。400 → error 文の元のアプリ。404 → データの有無。
3. 上の各アプリの「確認コマンド」を**読み取りだけで**流す。
4. 直したら「変更したらすること」のビルド・作り直し・`sw.js` の `CACHE`・文書更新を行う。
5. 動作確認は必ず **8200**（`apps/radio` は 8198）。**旧の 8100 番・8000 番には触らない。**
