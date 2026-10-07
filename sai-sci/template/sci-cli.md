# 研究プロジェクトでの命令（sci-cli）

`sai-link`・`sai-paper`・`sai-sci`・`sai-web` は、シェル（Bash）から記録を調べたり、次の CARD を作ったりするときに使う。
文書の保存と読み込みは `sai-link doc write` / `sai-link doc read` で行う。
自分の run_id は環境変数 `SAI_PARENT_RUN_ID` に入っている。
時間のかかる命令・ネットワークを使う命令には、実行前に `perl -e 'alarm <秒>; exec @ARGV' <命令>` と並べて上限を付ける。

## 実行を調べる（sai-link）

| やりたいこと | コマンド |
|---|---|
| 実行の一覧（todo / running / review / done ごと） | `sai-link run tree` |
| 実行の詳細（ブロック・状態・時間・コミット） | `sai-link run show <run_id>` |
| 依頼文・会話・報告・起動時の固定部分を読む | `sai-link run md --kind task\|message\|report\|system <run_id>` |
| その実行が保存した文書 | `sai-link doc list --run <run_id> --all`（本文は `sai-link doc read <トークン>`） |
| 過去の会話を言葉で探す | `sai-link session search -w <語,語> [--block <名>] [--json]` |
| 実行の中身（log / stats / history） | `sai-link run log\|stats\|history <run_id>` |

結果はほとんどが JSON で返る。長いときは必要な部分だけ取り出して読む。

## research-tree（sai-sci）

| やりたいこと | コマンド |
|---|---|
| 現況の要点 | `sai-sci status --brief` |
| 現況の全体 | `sai-sci status [--json]` |
| Node 1 つの詳細 | `sai-sci node <Q12> [--json]` |
| 実行のための入力（`# context`） | `sai-sci context --run <run_id> [--tree]` |
| PENDING の PROPOSAL | `sai-sci proposals [--json]` |
| TRIAGE（採用・却下・取り消し） | `sai-sci triage <P56> (--accept \| --reject \| --revert)` |
| READ_CARD（`--runner user` の CARD）の中身 | `sai-sci read <run_id>` |

`sai-sci status --brief` は最初に読む。いつも JSON で、次の項目を返す。全体（`sai-sci status`）は大きいので、要るときだけ読む。

- `progress`: 進み具合。
- `next`: Next Action の一覧。各項目は `kind`・`node`・`block`・`purpose`・`expect` と、なぜそれを勧めるかを示す `why`（コード）。
- `pending_proposals`: PENDING の PROPOSAL の数。
- `running`: 動いている実行。
- `warnings`: 注意。

### Node のための実行（CARD）を作る

```bash
sai-sci card --node <Q12> --block <sci-read> \
  --purpose <行動の目的> [--expect <期待する成果>] \
  [--papers <番号,番号>] [--runner pi|cc|user] [--depth abstract|figures|full] \
  [--input <waggle トークン>]… --executer agent \
  --origin-run "$SAI_PARENT_RUN_ID" --json
```

- `--node`（対象の Node。Q・O・F・E）・`--block`・`--purpose` は必ず書く。
- `--runner user` は、利用者が自分で READ_CARD になる。`--depth` はその読む深さ。
- `--input` は依頼文に載せる文書の waggle トークン。繰り返し可。長い決定事項は `sai-link doc write` で保存してから渡す。
- `--executer agent` を付けると、Human が Web からこの CARD を起こしても動かず、役（professor など）が子として起こす。付けない（既定の `human`）と、人が起こす CARD になる。
- `--origin-run` は自分がその Node を選んだ実行の run_id。省略すると `SAI_PARENT_RUN_ID`。
- `--json` を付けると戻り値が JSON になり、CARD の `run_id` が分かる。

## Web と同じ操作（sai-web）

`sai-web` は、Web（`apps/web`）の画面と同じ HTTP API を使うので、どちらで答えても同じ質問・承認になる。
接続先は環境変数 `SAI_WEB_URL`（既定 `http://127.0.0.1:8200`）。合言葉は環境変数 `SAI_WEB_TOKEN`、無ければ `<SAI_HOME>/web/token` を使う。合言葉はヘッダで渡すので、**値を出力に書かない**。

```bash
sai-web questions [--run <run_id>] [--project <名前>]
sai-web question <qid>
sai-web answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
sai-web approvals [--run <run_id>]
sai-web approve <aid> <allow|always|deny> [--reason 文]
sai-web send <project> <run_id> <文>
sai-web mlflow experiments
sai-web mlflow runs <experiment_id> [--filter <式>] [--max <n>]
sai-web mlflow run <mlflow_run_id>
sai-web mlflow metric <mlflow_run_id> <key> [--max <n>]
sai-web api <METHOD> <path> [--data <JSON>|-]
```

- 終了コード 0 = 成功、のこりは 2 = 入力の誤り・HTTP 400/422、3 = 見つからない・HTTP 404、4 = 状態の衝突・HTTP 409、1 = その他・つながらない。
- つながらないとき（終了コード 1）は、常駐が動いていない可能性がある。自分で起動しない。親がある run（operator など）は要点を親に返し、親が無い run（professor）は Human に上げる。

## 実行先

```bash
python3 "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" check <target>
python3 "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" status <job_id> [--json]
```

`exp_exec.py check <target>` は実行先の接続と前提を確かめる、`status <job_id>` は投げたジョブの状態を返す（`--json` で JSON）。

## 人が起こす CARD（task.md）を作る

これは `executer: human` の CARD（Human が Web の ▶ で起こすもの）の作り方である。research-tree の Node のための CARD は、上の `sai-sci card --executer agent` で作り、役（professor など）が `sai-link task create` の `card` で起こす。この節ではない。

`sai-link card options` で使えるブロック・runner・model・thinking を確かめ、標準入力に JSON を渡して `sai-link card create` する。
戻りの `path` を編集して `description:` と `origin_run_id:` を埋める。**実行は Human が行う。あなたは CARD を作るだけで、実行しない。**

## 使わないコマンド

次のコマンドは、実行の開始・削除・設定の変更になる。Human の指示が無い限り使わない。

`sai-link card run` / `sai-link run finalize` / `sai-link run advance` / `sai-link run rm` / `sai-link daemon` / `sai-link init` / `sai-link task send\|interrupt\|delete\|finalize\|requeue` / `sai-link run md --write`

`sai-paper init` / `sai-sci init` / `sai-sci apply` も、実行の記録や research-tree を書き換えるので、Human の指示が無い限り打たない。ただし professor（逆向きの `professor-backward` を含む）だけは、research-tree の TRIAGE のために `sai-sci triage` と、利用者が行う操作の `sai-sci apply`（標準入力に 1 件の JSON）を使ってよい。**★（Answer）は利用者だけが付ける。professor も付けない。** 課金・鍵・要件の範囲・成功基準に関わる決定は使わず、Human に上げる。operator は、research-tree を書き換える `sai-sci` の命令を打たない（読む命令だけ）。

## researcher に渡さないもの

命令や報告の中で、合言葉・鍵の値・ファイルのパスを人に求める形の指示を書かない。ファイルのパスは iPhone から受け取らない。
