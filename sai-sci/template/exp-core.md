# 実験ループの共通の決まり（exp-core）

2 つのブロック（`sci-exp-seed` / `sci-exp-branch`）が共通で読む決まり。
この文書の §4 の表と §6 の規則は**共有契約**であり、`sci-exp-seed`・`sci-exp-branch`・`exp_summary.py` のいずれも書き換えない。

## 1. 役割

| ブロック | すること | しないこと |
|---|---|---|
| `sci-exp-seed` | 世代 g の計画を立ててスクリプトを書き、branch を並列に起動する。世代 2 以降は前の世代の判定もする。 | スクリプトを実行しない。台帳の数値を自分で計算しない。 |
| `sci-exp-branch` | seed から受け取ったスクリプトを 1 本だけ実行し、MLflow に記録させて報告する。 | 判定をしない。スクリプトの中身を変えない。 |

- 世代は 1 始まりで数える。1 世代は `sci-exp-branch` の 1 つ以上の実行で、**並列に**走る。
- 判定の材料（最良・改善の有無・格子の端・漏洩の疑い）は**台帳コマンド（§5）が計算する**。LLM はその結果を読み、判定の中身（次に試すパラメータの値、新しい手法の案）だけを書く。
- seed は branch をすべて終えると、次の世代の `sci-exp-seed` を自分で起こす。**この往復を人に頼んだりはしない。**

## 2. 文書の渡し方

チャットは次の人に届かない。引き継ぎは文書だけで完結させる。

- `sai_write` は 3 つの箱（`filename`・`content`・`description`）で保存し、戻ってきた **waggle トークン**（例 `AXANSMpo`）を受け取る。

  ```json
  {"filename": "exp-run-N12-s120-g2-s03.md", "content": "本文", "description": "作業報告書 N12-s120 g2 s03: tune — 試行 6 件・失敗 0 件"}
  ```

- `description` の形は **`<文書の種類> <ID>: <1 文>`** にする。他のエージェントは一覧の `description` だけを見て、読むかどうかを決める。
- 文書を渡すときは必ず **トークン**で渡す。ファイルパスだけで渡さない（run フォルダは移動するので、パスはすぐに無効になる）。依頼文には `waggle:<トークン>` と書けば、実行時にその位置へ本文が埋め込まれる（1 段だけ）。
- あとで読み返すときは `sai_read { "token": "<トークン>" }`。一覧を出すだけなら `sai_read {}`。
- 改訂したら `-r2` を付けた新しい名前で保存する。古いトークンを参照している文書があれば、それも改訂する。
- 作業メモや途中経過は保存しない。保存するのは最終成果物だけである。

## 3. 実験仕様書

- ファイル名 `exp-spec-<study>.md`、description `実験仕様書 <study>: <研究の 1 文>`。
- **最初の世代の seed だけが作る。** 途中で書き換えない（変えたいなら往復を止めて利用者に聞く）。
- 本文の先頭に、次の JSON を ```json のコードブロックで 1 つだけ置く。値の出どころは `{{context}}` と `sai-moon-paper sci status --json` の `purpose.metric`（指標と成功基準）。

```json
{"study": "N12-s120", "sci_node": "N12", "first_seed_run_id": 120,
 "metric": "val_auc", "direction": "maximize", "success": 0.85,
 "min_delta": 0.0001, "max_generations": 8, "max_branches": 4,
 "max_fail": 3, "max_debug": 3,
 "mlflow_experiment": "local", "workdir": "experiments/N12-s120"}
```

| 欄 | 中身 |
|---|---|
| `study` | `<sci_node>-s<最初の seed の run_id>`。最初の seed の run_id は環境変数 `SAI_PARENT_RUN_ID`。 |
| `sci_node` | research-tree の節点（`N12` のような形） |
| `first_seed_run_id` | 最初の seed の run_id（`study` の `s` の後ろの数字） |
| `metric` | 指標名（`val_auc` など） |
| `direction` | `maximize`（大きいほどよい）か `minimize`（小さいほどよい）か |
| `success` | 成功基準の数字 |
| `min_delta` | 「よくなった」と数える最小の幅。既定 `0.0001` |
| `max_generations` | 世代の数の上限。既定 `8` |
| `max_branches` | 1 世代に並列で走らせる branch の数の上限。既定 `4` |
| `max_fail` | 同じスクリプトが改善せずに終わった回数の上限（`stale_rounds`）。既定 `3` |
| `max_debug` | 同じスクリプトを直しながら続ける回数の上限。既定 `3` |
| `mlflow_experiment` | MLflow の experiment 名。既定 `"local"` |
| `workdir` | スクリプトを置くフォルダ。既定 `experiments/<study>` |

- research-tree に書かれていない値は上の既定値を使う。**決まらない値（指標・向き・成功基準）を推測で埋めない。** 利用者に質問して待つ。

## 4. MLflow の記録の決まり

- branch 1 つ = MLflow の親 run 1 つ（`exp.role=script`）。パラメータの組ごとに、入れ子の子 run（`exp.role=trial`、`mlflow.start_run(nested=True)`）。
- experiment は仕様書の `mlflow_experiment`（既定 `local`）。接続先は環境変数 `MLFLOW_TRACKING_URI` に任せる。**スクリプトの中で上書きしない。**
- 親 run と子 run の両方に付けるタグ（すべて文字列）:

| タグ | 中身 | 例 |
|---|---|---|
| `sai.project` | `SAI_PROJECT_ROOT` の末尾のフォルダ名（seed が定数で書く） | `west` |
| `sai.run_id` | その branch の run_id（スクリプトが環境変数 `EXP_BRANCH_RUN_ID` から読む） | `121` |
| `sai.seed_run_id` | その branch を起動した seed の run_id（定数） | `120` |
| `exp.study` | `<sci_node>-s<最初の seed の run_id>` | `N12-s120` |
| `exp.generation` | 世代（1 始まり） | `2` |
| `exp.script` | `s` + 2 桁の連番。debug と tune では同じ ID を使う | `s03` |
| `exp.action` | `draft` / `tune` / `new` / `debug` / `ablation` | `tune` |
| `exp.parent_script` | 参考にしたスクリプトの ID（無ければ空文字） | `s01` |
| `exp.role` | `script` / `trial` | `trial` |
| `sci.node` | research-tree の節点 | `N12` |
| `exp.error` | 失敗した試行だけ。例外の 1 行目（500 字まで） | `CUDA out of memory` |

- 親 run だけに付けるタグ: `exp.metric`（仕様書の指標名）・`exp.direction`（`maximize` / `minimize`）。
- metric:
  - 試行ごとに、仕様書の指標名で最終値を記録する（学習曲線は `step` 付きで記録してよい）。`exec_time_s` も記録する。
  - 試行が全部終わったら、**親 run にも同じ指標名で最良値を 1 回記録する**。
- 失敗した試行は `status=FAILED` で終え、`exp.error` を付ける（雛形のとおり、例外を子 run の `with` の外へ出して閉じる）。
- 成果物: 親 run にスクリプト本体（`code/`）。

### スクリプトの雛形

seed が定数と `GRID` だけを書き換える。`train(params)` の `...` の部分が研究の本体である。MLflow に関する行は書き換えない。

```python
import itertools, os, time, mlflow

# 定数（seed が世代ごとに書き換える）
STUDY, SCI_NODE, SCRIPT = "N12-s120", "N12", "s01"
GENERATION, ACTION, PARENT = 1, "draft", ""
SEED_RUN_ID, PROJECT = "120", "west"
METRIC, DIRECTION, EXPERIMENT = "val_auc", "maximize", "local"

# 実行している branch の run_id。seed がスクリプトを書く時点では分からないので、
# 実行する branch が環境変数 EXP_BRANCH_RUN_ID に入れて渡す。
BRANCH_RUN_ID = os.environ.get("EXP_BRANCH_RUN_ID", "")

GRID = {"lr": [0.01, 0.03, 0.1], "depth": [3, 5]}

TAGS = {"sai.project": PROJECT, "sai.run_id": BRANCH_RUN_ID,
        "sai.seed_run_id": SEED_RUN_ID, "exp.study": STUDY,
        "exp.generation": str(GENERATION), "exp.script": SCRIPT,
        "exp.action": ACTION, "exp.parent_script": PARENT,
        "sci.node": SCI_NODE}


def train(params: dict) -> float:
    ...  # 研究の本体。METRIC の値を 1 つ返す


mlflow.set_experiment(EXPERIMENT)  # 接続先は環境変数 MLFLOW_TRACKING_URI
with mlflow.start_run(
        run_name=f"{STUDY}-{SCRIPT}-g{GENERATION}",
        tags={**TAGS, "exp.role": "script",
              "exp.metric": METRIC, "exp.direction": DIRECTION}):
    mlflow.log_artifact(__file__, artifact_path="code")
    best = None
    for values in itertools.product(*GRID.values()):
        params = dict(zip(GRID, values))
        try:
            # 例外が with を抜けると、mlflow がこの子 run だけを FAILED で閉じる
            with mlflow.start_run(nested=True, tags={**TAGS, "exp.role": "trial"}):
                mlflow.log_params(params)
                start = time.time()
                try:
                    value = train(params)
                except Exception as exc:
                    mlflow.set_tag("exp.error", str(exc)[:500])
                    raise
                mlflow.log_metric(METRIC, value)
                mlflow.log_metric("exec_time_s", time.time() - start)
                if best is None or (value > best if DIRECTION == "maximize" else value < best):
                    best = value
        except Exception:
            continue  # 1 つの組が失敗しても残りの組を続ける
    if best is not None:
        mlflow.log_metric(METRIC, best)  # 親 run にも最良値を 1 回だけ記録する
```

- 入れ子の run の中で `mlflow.end_run()` を呼ばない。呼ぶと、`with` を抜けるときに親の run まで閉じてしまう。
- スクリプトの置き場所は `<workdir>/<script>/g<世代>.py`。**1 世代で同じ script が 2 回走らない**ため、世代がファイル名に入る。

## 5. 台帳

### 台帳コマンド `exp_summary.py`

置き場所は `$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py`（`sai-moon-paper sci init` が配る。`SAI_PROJECT_ROOT` は sai-link が必ず入れる）。**呼び出すときは必ずこの形を使う**（別のファイルを探したり、パスを作り変えたりしない）。ファイルが無いときは `BLOCKED` で報告して終える。

`--python ">=3.10"` は、`~/.python-version` などが入っていない Python を指していても、使える Python で動かすための指定である。**外さない。**

呼び出し:

```bash
uv run --no-project --python ">=3.10" --with mlflow python "$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py" \
  --study <study> --metric <名前> --direction maximize|minimize \
  [--min-delta 0.0001] [--leak-max 0.999] [--leak-min 1e-6] \
  [--experiment <名前>] [--tracking-uri <URI>]
```

- 終了コード: 0 = 成功（該当 run が 0 件でも 0。そのとき `scripts` と `rounds` は空配列）、1 = 実行中の失敗、2 = 引数の誤り・接続先が無い。
- **`--direction` は省略しない**（必須引数）。
- 標準出力（JSON 1 つ）:

```json
{
  "study": "N12-s120", "metric": "val_auc", "direction": "maximize", "min_delta": 0.0001,
  "best": {"script": "s02", "round": 3, "run_id": "…", "value": 0.871, "params": {"lr": "0.03"}},
  "last": {"round": 3, "script": "s02", "action": "tune", "trials": 6, "failed": 0},
  "rounds": [{"round": 1, "script": "s01", "action": "draft", "best_so_far": 0.84, "improved": true}],
  "scripts": [{"script": "s01", "action_first": "draft", "parent_script": "", "rounds": [1, 2],
               "trials": 12, "failed": 1, "best": 0.84, "best_params": {"lr": "0.1"},
               "stale_rounds": 1, "debug_rounds": 0}],
  "param_edges": [{"script": "s01", "param": "lr", "side": "max"}],
  "ablation_done": false,
  "leak_suspects": [{"script": "s02", "round": 3, "run_id": "…", "value": 0.9995}]
}
```

### 各欄の意味

- `round` は**世代**（`exp.generation`）を指す。同じ世代の branch は全部数える。
- `best`: direction に従った最良の値。成功試行が 0 件なら `null`。
- `last`: いちばん大きい世代の内容（`trials` は試行の数、`failed` は失敗の数）。
- `rounds[]`: 世代ごとの累積。`best_so_far` はその世代までの累積の最良、`improved` は前の世代より `min_delta` を超えてよくなったか。
- `scripts[]`: スクリプト ID ごとのまとめ。`rounds` はそのスクリプトが走った世代、`stale_rounds` は自分の最良を `min_delta` を超えて更新しなかった世代が何回続いたか、`debug_rounds` は `exp.action=debug` が何回続いたか。
- `param_edges`: 最良の試行の値が、そのスクリプトの試行の中で最小か最大だったパラメータ（2 種類以上の値が試されたものだけ）。
- `ablation_done`: `exp.action=ablation` の親 run が 1 つでもあれば true。
- `leak_suspects`: maximize なら `value >= leak_max`、minimize なら `value <= leak_min` の成功試行。

**この数値を自分で作り直さない。** 命令の結果をそのまま読む。

## 6. 判定の規則

seed が世代 2 以降に行う。台帳（§5）の欄だけが材料である。

### 判定書

- ファイル名 `review-<study>-g<世代>.md`、description `判定書 <study> g<世代>: <decision> — <理由 1 文>`。
- 本文の先頭に、次の JSON を ```json のコードブロックで 1 つだけ置く。

```json
{"study": "N12-s120", "sci_node": "N12", "generation": 2,
 "decision": "extend | new_script | debug | ablation | stop",
 "rule": "R1〜R7 のどれを当てはめたか",
 "next": [{"action": "tune", "script": "s01", "parent_script": "",
           "grid": {"lr": [0.1, 0.3]}, "idea": "", "fix": ""}],
 "stop_reason": "success | max_generations | none"}
```

- `next` は次の世代の **branch ごとの計画の配列**で、1〜`max_branches` 個。それぞれの `action` は `draft` / `tune` / `new` / `debug` / `ablation`。`grid` は `tune` のとき、`idea` は `new` と `ablation` のとき、`fix` は `debug` のときに入れる（空文字でもよい）。
- 判定は `stop` でないかぎりは `next` に必ず 1 つ入れる。

### 規則（上から順に当てはめ、最初に当てはまったものを採る。数字は仕様書の値）

| # | 条件（台帳の欄で判定） | 判定 | 次にすること |
|---|---|---|---|
| R1 | `last.round >= max_generations` | `stop`（`stop_reason` は `max_generations`） | 終える。完了報告書を書く |
| R2 | `best` が成功基準 `success` に達し、`ablation_done == false` | `ablation` | 最良のスクリプトから成分を 1 つ外した branch を 1 つ |
| R3 | `best` が成功基準に達し、`ablation_done == true` | `stop`（`stop_reason` は `success`） | 終える。完了報告書を書く |
| R4 | 直前の世代で成功した試行が 1 つも無い（全部失敗、または記録が無い） | `debug` | 原因の当たった script を 1 つだけ選び、`fix` に直す点を書いて動かす |
| R5 | `leak_suspects` に `round == last.round` の試行がある | `debug` | 漏洩を疑う点を `fix` に書いて 1 つ |
| R6 | 直前の世代の script の `debug_rounds >= max_debug`、または `stale_rounds >= max_fail` | `new_script` | `best` の script を `parent_script` にして、新しい script を 1 つ |
| R7 | 上のどれにも当てはまらない | `extend` | `param_edges` があれば端の外側へ、無ければ最良の近くを細かくする branch を 1〜`max_branches` 個 |

- 数字だけを変える（パラメータの値だけを変える、`train()` の構造は変えない）のは `extend` と `debug` の違いではない。両方とも同じ `script` ID を使い続ける。
- `new` と `ablation` は **使ったことのない次の script 番号**（`s02` など）を作り、`parent_script` に参考にした script を入れる。

## 7. branch への依頼文の形

seed が `sci-exp-branch` に渡す依頼文（`task_create` の `task_text`）。**先頭の 7 行は `key: value`** で書く。

```text
study: N12-s120
generation: 2
script: s03
action: tune
parent_script: s01
spec: <実験仕様書のトークン>
seed_run_id: 125

## スクリプトに書く中身
- 実行するファイル: <workdir>/s03/g2.py
- action: tune。s01/g1.py を写し、GRID を次のように替える。
  {"lr": [0.1, 0.3], "depth": [4, 6]}
- 期待する効果: val_auc を 0.87 以上にする。
```

- `study`・`generation`・`script`・`action`・`parent_script` は §4 のタグの値と同じ。
- `spec` には**実験仕様書の waggle トークン**を書く。本文が埋め込まれるので、branch は仕様書を読んだことにする。
- `seed_run_id` は**この seed 自身の run_id**（環境変数 `SAI_PARENT_RUN_ID`）。branch は `sai.seed_run_id` の定数の確認に使う。
- 7 行の後の節は自由。script の中身（`GRID` の値・直す点・期待する効果）を書く。
- 文書を渡すときは `waggle:<トークン>` と書く。本文を要約し直さない。

## 8. 実行場所の規則

スクリプトは**実行先（target）**に投げて動かす。実行先は名前を持つフォルダ（`.sai/exec/<名前>/`）で、Mac 本体・LAN の PC・クラウドなどを利用者が個別に用意する。**どの実行先でも、呼び方は同じ**である（使い方は `exp-run`）。

- 実行先の名前は環境変数 `EXP_TARGET`。**空なら `local`**（この PC）。別の実行先を勝手に選ばない。
- 投入は分配器 `exp_exec.py` で行う。branch は実行の前に、自分の `SAI_PARENT_RUN_ID` を `EXP_BRANCH_RUN_ID` に入れる。**環境変数を渡す引数は無いので、コマンドの前に `env EXP_BRANCH_RUN_ID=…` と書く**（どの実行先でも効く）。

  ```bash
  uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" \
    run "${EXP_TARGET:-local}" --name <script>-g<世代> -- \
    env EXP_BRANCH_RUN_ID=<自分の run_id> uv run --python ">=3.10" --with mlflow python <スクリプトのパス>
  ```

- 標準出力の 1 行目に出る job_id を控え、`status <job_id> --json` で `done` / `failed` / `cancelled` / `lost` のどれかになるまで待つ。**1 回の待ち合わせは `sleep 300` まで**にして、1 つの長いコマンドで待たない。
- 記録は実行先から**中央の MLflow に直接**入る（回収の手順は無い）。実行先が MLflow に届かないと「記録なし」になる。branch は自分で台帳を取り直さない。
- 実行先が使えないとき（`check` が失敗、`run` が断られる）は、直そうとせず `BLOCKED` で報告して終える。

## 9. 作業報告書（branch）

- ファイル名 `exp-run-<study>-g<世代>-<script>.md`。
- description `作業報告書 <study> g<世代> <script>: <action> — 試行 <n> 件・失敗 <m> 件`。`n` と `m` は、台帳の `scripts[]` にある**自分の script の欄**の `trials` と `failed`（記録が無いときは 0 と 0）。
- 本文は次の表を置く。「—」は「該当なし」の意味である。

| 項目 | 値 |
|---|---|
| スクリプト | `<workdir>/<script>/g<世代>.py` |
| 実行場所 | 実行先の名前（`local` など） |
| job_id | `run` の 1 行目に出た job_id |
| 状態 | 成功 / 失敗 / 記録なし |
| MLflow の親 run | run_id |
| 試行 / 失敗 | 台帳の `scripts[]` の自分の欄の `trials` / `failed` |
| 最良値 | 台帳の `scripts[]` の自分の欄の `best` |
| 見積もりと実際の時間 | 分 |

- この表以外の節（実行ログ、原因、次の世代へのメモ）は、短い記入だけでよい。**判断は seed がする。**
- 報告書には「次に何をするか」を書かない。

## 10. 完了報告書（最後の seed）

- ファイル名 `exp-final-<study>.md`。
- description `完了報告書 <study>: <stop_reason> — 最良 <metric> <value>`。
- 本文:

  ```markdown
  ## 最良
  - スクリプト: <script>
  - 世代: <世代>
  - 値: <metric> = <value>
  - パラメータ: <params>
  - MLflow の run_id: <run_id>

  ## 世代ごとの表
  | 世代 | script | action | decision | rule | best_so_far | 作業報告書 | 判定書 |

  ## research-tree に出した提案
  - P<id> ...

  ## 停止の理由
  <stop_reason と、その判定書の rule>
  ```

- 最良の値・パラメータ・script・世代は、台帳の `best` と `rounds[]` から書く。**自分で再計算しない。**
- 世代ごとの表の「作業報告書」と「判定書」の列には **waggle のトークン**を書く。

## 11. 止める条件

- 止めるのは、**判定が `stop` のときだけ**である（規則 R1・R3）。世代の上限に達した R1 は `stop` に入る。
- 次の 2 つのときにも止める。この 2 つは止まる判断ではなく、利用者に質問するための合図である。
  - 同じ世代で branch の記録が 1 つも無いことが **2 代続けて続いた**とき。MLflow への接続を疑い、利用者に質問する。（1 代だけなら R4 の `debug` として次へ進む）
  - 判定書が 2 回続けて作られなかったとき。台帳や木の読み込みで異常があったと疑い、利用者に質問する。
- 上の 2 つに当たらなければ、`sai_finalize` は**利用者が指示したときだけ**呼ぶ。勝手に完了にしない。
