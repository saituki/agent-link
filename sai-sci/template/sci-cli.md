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
| 木を変える（ALLOWED_OPS の op。標準入力に JSON） | `sai-sci op [--run <run_id>] [--block <ブロックの id>]` |
| 利用者が木を直す（agent は打たない。例外は下の段落） | `sai-sci apply`（標準入力に op 1 件の JSON） |
| 承認の記録（フックだけが呼ぶ。agent は打たない） | `sai-sci approval record --qid … --question … --answer … --answered-by …` |
| READ_CARD（`--runner user` の CARD）の中身 | `sai-sci read <run_id>` |

`sai-sci status --brief` は最初に読む。いつも JSON で、次の項目を返す。全体（`sai-sci status`）は大きいので、要るときだけ読む。

- `progress`: 進み具合。
- `next`: Next Action の一覧。各項目は `kind`・`node`・`block`・`purpose`・`expect` と、なぜそれを勧めるかを示す `why`（コード）。
- `running`: 動いている実行。
- `warnings`: 注意。

### Node のための実行（CARD）を作る

```bash
sai-sci card --node <Q12> --block <lab-read> \
  --purpose <行動の目的> [--expect <期待する成果>] \
  [--papers <番号,番号>] [--runner cc|user] [--depth abstract|figures|full] \
  [--input <waggle トークン>]… [--model <m>] [--thinking <t>] \
  --origin-run "$SAI_PARENT_RUN_ID" --json
```

- `--node`（対象の Node。Q・A・F・E・S・D・SEED）・`--block`・`--purpose` は必ず書く。
- `--runner` は `cc`（既定）か `user`。`user` は、利用者が自分で READ_CARD になる。`--depth` はその読む深さ。
- `--model`・`--thinking` は、起こす側が選ぶ（下の「別の agent を起こす」）。省くと block の frontmatter の値になる。
- `--input` は依頼文に載せる文書の waggle トークン。繰り返し可。長い決定事項は `sai-link doc write` で保存してから渡す。
- `--child`（任意）は、CARD が足す子の型（`Q`・`A`・`F`）。Next Action の `card` が `child` を持つときは、その値をそのまま付ける（`F` を探す CARD は `--child F`）。
- `--origin-run` は自分がその Node を選んだ実行の run_id。省略すると `SAI_PARENT_RUN_ID`。
- `--json` を付けると戻り値が JSON になり、CARD の `run_id` が分かる。

## 別の agent を起こす（2 つだけ）

| 場面 | 使う物 | 待ち方 |
|---|---|---|
| 結果を待たない（研究の Node の作業・実験の連鎖） | `sai-sci card … --json` で Card を作り、`sai-link card run <run_id>` で起こす | 待たない。結果は木に入る。Professor は `sai-link run wait` で終わりと止まりを見る |
| ジョブの終わりに起こす（branch → review） | `sai-sci card … --json` で Card を作り、`sai-link card wait <run_id> --job <job_id>` | 常駐がジョブの終わりに起こす |
| 結果を待つ（support agent を呼ぶ・調査書を次に使う） | `sai-link task create`（要素に `model`・`thinking` を書ける）→ `task execute` → `task wait` | `task wait` |

- model と thinking は、起こす側が選ぶ。省くと block の frontmatter の値になる。目安: 判断・評価は opus/high、設計・実装は sonnet/high、読み・まとめは sonnet/medium、調査は haiku/low。
- runner は cc だけ（`task create` では書かない）。pi は使えない。
- Card を `task create` の `card` で子にする経路は使わない。

## Web と同じ操作（sai-web）

`sai-web` は、Web（`apps/web`）の画面と同じ HTTP API を使うので、どちらで答えても同じ質問になる。
接続先は環境変数 `SAI_WEB_URL`（既定 `http://127.0.0.1:8200`）。合言葉は環境変数 `SAI_WEB_TOKEN`、無ければ `<SAI_HOME>/web/token` を使う。合言葉はヘッダで渡すので、**値を出力に書かない**。

```bash
sai-web questions [--run <run_id>] [--project <名前>]
sai-web question <qid>
sai-web answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
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

research-tree の Node のための CARD は、上の `sai-sci card` で作る。この節ではない。

`sai-link card options` で使えるブロック・runner・model・thinking を確かめ、標準入力に JSON を渡して `sai-link card create` する。

```bash
sai-link card create <<'EOF'
{"runner": "cc", "block": "<ブロックの id>", "model": "sonnet", "thinking": "high", "task_text": "<依頼文>"}
EOF
```

戻りの `path` を編集して `description:` を埋める。**実行は Human が行う。あなたは CARD を作るだけで、実行しない。**

## 使わないコマンド

次のコマンドは、実行の開始・削除・設定の変更になる。Human の指示が無い限り使わない。

`sai-link run advance` / `sai-link run rm` / `sai-link daemon` / `sai-link init` / `sai-link task send\|interrupt\|delete\|finalize\|requeue` / `sai-link run md --write`

`sai-link card run` は、上の表の場面だけで使う（Card を起こすのは Professor と Human）。`sai-link run finalize` は、root の run の終わりに使う（`sci-core` の「終わり方」）。

`sai-paper init` / `sai-sci init` / `sai-sci apply` も、実行の記録や research-tree を書き換えるので、Human の指示が無い限り打たない。`sai-sci op` は、`# context` の `## ALLOWED_OPS` の範囲で木を変えるときだけ使う。`sai-sci approval record` は、質問の答えを受けたフックだけが呼ぶ。**★ は利用者だけが付ける。** 課金・鍵・要件の範囲・成功基準に関わる決定は使わず、Human に上げる。professor と operator は木を変えない（読む命令だけ）。木を変える作業は、子の block が `sai-sci op` で行う。例外は `professor-backward` だけで、Human が答えた `edit` の文を `sai-sci apply` で反映してよい。

## researcher に渡さないもの

命令や報告の中で、合言葉・鍵の値・ファイルのパスを人に求める形の指示を書かない。ファイルのパスは iPhone から受け取らない。
