# 実行先: 実験のスクリプトを動かす場所

実験のスクリプトは、この PC・LAN の別の PC・クラウドのどこでも動かせる。動かす場所を**実行先（target）**と呼ぶ。
実行先は利用者が用意する（増えたり減ったりする）。**どの実行先でも、エージェントの呼び方は同じ**である。

## 1. 規則

- スクリプトの実行は、**必ず分配器 `exp_exec.py` で行う。** `ssh`・`scp`・`rsync`・`docker` で直接動かしてはならない。理由: 実行先ごとの違い（転送・切り離し・回収）は実行先の側が知っている。直接動かすと、状態も中止もできなくなる。
- **投げる前に、スクリプトのある作業フォルダ（仕様書の `workdir`）へ `cd` する。** 実行先には cwd のコードが送られる（`.git/`・`.venv/`・`.sai/`・`mlruns/` などは除く。足すときは cwd の `.exp-ignore` に 1 行 1 パターンで書く）。スクリプトは workdir からの相対パスで書く。
- 実行先の名前は環境変数 `EXP_TARGET`。空なら `local`（この PC）。**別の実行先を勝手に選ばない。**
- 実行先が使えないとき（`check` が失敗、`run` が断られる）は、直そうとせず `BLOCKED` で報告する。
- 結果は実行先から**中央の MLflow に直接**入る。手で回収しない。
- 確認だけのためにジョブを投げない（データの有無・GPU は、スクリプトの最初の `device=` の行と最初の試行で確かめる）。

## 2. 呼び方

`EXP` は次の 1 行の略。**`--python` は外さない**（`~/.python-version` が入っていない版を指していても動かすため）。

```bash
EXP='uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py"'
```

| したいこと | 命令 |
|---|---|
| 実行先の一覧 | `eval $EXP list` |
| 使えるか確かめる | `eval $EXP check <target>` |
| 投げる | `eval $EXP run <target> --name <短い名前> -- <command...>` |
| 始まりを確かめる | `eval $EXP started <job_id> --timeout 600 --json` |
| 状態を見る | `eval $EXP status <job_id> --json` |
| ログを見る | `eval $EXP logs <job_id> -n 50` |
| 中止する | `eval $EXP cancel <job_id>` |

- `run` は投げてすぐ戻り、**標準出力の 1 行目に job_id** が出る。控える。
- `command` には、環境変数を渡す引数が無い。**命令の前に `env NAME=値` を書く**。
  例: `... run local -- env EXP_BRANCH_RUN_ID=121 uv run --python ">=3.10" --with mlflow ID121/train.py`（先に `cd <workdir>`）
- **スクリプトの前に `python` を付けない。** `uv run <script>.py` はスクリプトの PEP 723 の欄（`# /// script`）の依存を読む。`python <script>.py` の形にすると欄が読まれず、必要な道具が入らない。
- `status` の `state` は次のとおり。

| state | 意味 | 次にすること |
|---|---|---|
| `queued` | 投入済み・まだ動いていない | branch は待たない。`started` の結果で決める（`exp-core` §8）。review は `status` を 1 回読むだけ |
| `running` | 動いている | 同上 |
| `done` | 終了コード 0 で終わった | 記録を確かめる |
| `failed` | 0 以外で終わった | `logs` を読んで報告する |
| `cancelled` | 中止された | 報告する |
| `lost` | 行方が分からない（実行先が落ちた、など） | 報告する。自分で再投入しない |

- **終わりまで待たない。** 待つのは `started` の 1 回だけ（最大 600 秒）。

## 3. 危険な操作の確認（必ず守る）

次は、実行する前に必ず**起動者**（CARD を起こした人）の承認を得る。

- (a) スクリプトの置き場の外にあるファイルの削除や上書き
- (b) 道具の導入
- (c) 他の人のジョブやプロセスの停止（**自分が投入した job_id だけ**を扱う）
- (d) 実行先の設定（`target.toml`・adapter）の変更

確認の方法: 質問ツールで尋ねる（`AskUserQuestion`）。返答の本文だけで尋ねて止まらない。

sai-link は cc を権限確認なしで起動する。そのため、この確認はエージェントである自分の指示として守る。

## 4. 秘密の扱い

- 鍵・合言葉の値（`settings.json` の `env`・`.env` の中身）を、質問・報告・スクリプト・task.md に書かない。値が要るときは環境変数の名前だけを書く。
- AWS の鍵と S3 の endpoint は環境変数で受け取る。値をスクリプトに埋め込まない。
