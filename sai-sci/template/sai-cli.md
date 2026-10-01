# 研究プロジェクトでの命令（sai-cli）

`sai-moon-link` と `sai-moon-paper` は、シェル（Bash）から記録を調べたり、次のカードを作ったりするときに使う。
`sai-link`・`sai-paper`（名前に `moon` が無いほう）は旧版で、別の記録を読み書きする。使わない。
文書の保存と読み込みは CLI ではなく `sai_write` / `sai_read` で行う。
自分の run_id は環境変数 `SAI_PARENT_RUN_ID` に入っている。
時間のかかる命令・ネットワークを使う命令には、実行前に `perl -e 'alarm <秒>; exec @ARGV' <命令>` と並べて上限を付ける。

## 実行を調べる（sai-moon-link）

| やりたいこと | コマンド |
|---|---|
| 実行の一覧（todo / running / review / done ごと） | `sai-moon-link phone tree` |
| 実行の詳細（ブロック・状態・時間・コミット） | `sai-moon-link block show <run_id>` |
| 依頼文・会話・報告・起動時の固定部分を読む | `sai-moon-link phone md --kind task\|message\|report\|system <run_id>` |
| その実行が保存した文書 | `sai-moon-link phone waggle <run_id>`（`--token <トークン>` で本文。残りも `sai_read` で読める） |
| 過去の会話を言葉で探す | `sai-moon-link search -w <語,語> [--block <名>] [--json]` |
| 実行の中身（log / stats / history） | `sai-moon-link phone run --view log\|stats\|history <run_id>` |
| 3 層の図（root / seed / branch） | `sai-moon-link phone rtree` |

結果はほとんどが JSON で返る。長いときは必要な部分だけ取り出して読む。

## 研究方針の木（sai-moon-paper sci）

| やりたいこと | コマンド |
|---|---|
| 現況の要点（GATE・DEBT・NEXT_ACTION） | `sai-moon-paper sci status --brief` |
| 現況の全体（SEED_SENTENCE・GATE・DEBT・木・NEXT_ACTION） | `sai-moon-paper sci status [--json]` |
| NODE 1 つの詳細 | `sai-moon-paper sci node <N12>` |
| 実行のための入力（実行の SCOPE の説明） | `sai-moon-paper sci context --run <run_id>` |
| TRIAGE 待ちの PROPOSAL | `sai-moon-paper sci proposals [--json]` |
| TRIAGE（ACCEPT・REJECT・REVERT） | `sai-moon-paper sci triage <P56> (--accept \| --reject \| --revert) [--note 文]` |

`sci status --brief` は現在の状態を最も短く示す。最初に読むのはこれ。全体（`sci status`）は大きいので、要るときだけ読む。

### NODE のための実行（CARD）を作る

```bash
sai-moon-paper sci card --node <N12> --block <sci-read> \
  --purpose <行動の目的> [--question <Q7>] [--expect <期待する成果>] \
  [--input <waggle トークン>]… --executer professor \
  --origin-run "$SAI_PARENT_RUN_ID" --json
```

- `--node`（対象の NODE）と `--purpose`（行動の目的）は必ず書く。
- `--block`（実行するブロックの id）・`--question`（答える QUESTION）・`--expect`（期待する成果）を足せる。
- `--input` は依頼文に載せる文書の waggle トークン。繰り返し可。長い決定事項は `sai_write` で保存してから渡す。
- `--executer professor` を付けると、Human が Web からこのカードを起こしても動かず、professor が子として起こす。付けない（既定の `human`）と、人が起こすカードになる。
- `--origin-run` は自分がその NODE を選んだ実行の run_id。省略すると `SAI_PARENT_RUN_ID`。
- `--json` を付けると戻り値が JSON になり、カードの `run_id` が分かる。

## Web と同じ操作（phone_api.py）

研究プロジェクトの `.sai/tools/phone_api.py` を呼ぶ。`apps/phone` の画面と同じ HTTP API を使うので、どちらで答えても同じ質問・承認になる。
合言葉はヘッダで渡すので、**値を出力に書かない**。

```bash
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" questions [--run <run_id>] [--project <名前>]
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" question <qid>
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" answer <qid> (--choice <番号> | --choices <番号,番号> | --text <文>) …
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" approvals [--run <run_id>]
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" approve <aid> <allow|always|deny> [--reason 文]
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" send <project> <run_id> <文>
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" mlflow experiments
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" mlflow runs <experiment_id> [--filter <式>] [--max <n>]
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" mlflow run <mlflow_run_id>
python3 "$SAI_PROJECT_ROOT/.sai/tools/phone_api.py" mlflow metric <mlflow_run_id> <key> [--max <n>]
```

- 終了コード 0 = 終わった、1 = サーバがエラーを返した・引数が違う、2 = sai-phone につながなかった。
- 2 のときは、常駐が動いていない可能性がある。自分で起動しない。親がある run（operator など）は要点を親に返し、親が無い run（professor）は Human に上げる。

## 実行先

```bash
python3 "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" check <target>
python3 "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" status <job_id> [--json]
```

`exp_exec.py check <target>` は実行先の接続と前提を確かめる、`status <job_id>` は投げたジョブの状態を返す（`--json` で JSON）。

## 人が起こすカード（task.md）を作る

これは `executer: human` のカード（Human が Web の ▶ で起こすもの）の作り方である。研究方針の木の NODE のための CARD は、上の `sci card --executer professor` で作り、professor が `task_create` の `card` で起こす。この節ではない。

`sai-moon-link phone card options` で使えるブロック・runner・model・thinking を確かめ、標準入力に JSON を渡して `sai-moon-link phone card create` する。
戻りの `path` を編集して `description:` と `origin_run_id:` を埋める。**実行は Human が行う。あなたはカードを作るだけで、実行しない。**

## 使わないコマンド

次のコマンドは、実行の開始・削除・設定の変更になる。Human の指示が無い限り使わない。

`sai-moon-link phone card run` / `sai-moon-link phone finalize` / `sai-moon-link phone advance` / `sai-moon-link block run` / `sai-moon-link block rm` / `sai-moon-link phone rm` / `sai-moon-link daemon` / `sai-moon-link init` / `sai-moon-link migrate-outputs` / `sai-moon-link task manage` / `sai-moon-link phone md --write`

`sai-moon-paper init` / `sai-moon-paper sci init` / `sai-moon-paper sci apply` も、実行の記録や木を書き換えるので、Human の指示が無い限り打たない。ただし professor（逆向きの `professor-backward` を含む）だけは、木の TRIAGE のために `sci triage` と `sci apply`（標準入力に op の JSON。`decide` など、人がする op）を使ってよい。課金・鍵・要件の範囲・成功基準に関わる決定は使わず、Human に上げる。operator は、木を書き換える `sci` の命令を打たない（読む命令だけ）。

## researcher に渡さないもの

命令や報告の中で、合言葉・鍵の値・ファイルのパスを人に求める形の指示を書かない。ファイルのパスは iPhone から受け取らない。