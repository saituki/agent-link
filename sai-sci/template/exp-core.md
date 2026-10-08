# 実験ループの共通の決まり（exp-core）

3 つのブロック（`sci-exp-sprout` / `sci-exp-branch` / `sci-exp-review`）が共通で読む決まり。
`sci-exp-scout` はこの文書ではなく `exp-ref` を読む。
この文書の §4 の表・§5 の欄・§6 の規則は**共有契約**であり、3 つのブロックと `exp_summary.py` のいずれも書き換えない。

## 1. 役割

実験の流れは **scout → sprout → branch → review** の順に進む。scout が参照実装を探して出典表の下書きを作り、sprout が PATH を選んで SPROUT（実験の計画）を作り、その SPROUT の下の E を branch が写してスクリプトに `# src:` を付ける。

**神経のモデル**（ニューロン・神経集団・ネットワークの微分方程式）のときは scout の後に `sci-sim-model`（2 本以上をつなぐなら `sci-sim-compose`）が**モデル定義書**を作り、sprout がそのトークンを実験仕様書の `sim_model` に入れる。branch はそこから式・値・再現の目標を写し、**すべての試行で土台が再現の目標を満たすかを MLflow に記録する**（`§4` の「再現の確かめ」）。review は、目標を外れた土台を成功と数えず debug に戻す（`§6` の R1a）。

| ブロック | 役 | すること | しないこと |
|---|---|---|---|
| `sci-exp-scout` | scout | 実験の前に参照実装（Library・ModelDB・GitHub）を探し、参照コード調査書を保存する。待たずに終わる | 実験仕様書を書かない・スクリプトを書かない・実行しない |
| `sci-sim-model` / `sci-sim-compose` | modeler | 神経のモデルの式・値・再現の目標を、参照コードと論文から出典つきで抜き、モデル定義書を保存する（書式は `sim-core`）。待たずに終わる | 実験仕様書を書かない・スクリプトを書かない・実行しない |
| `sci-exp-sprout` | sprout | research-tree から PATH を選んで SPROUT（実験の計画）を作る。実験仕様書を作り、SPROUT の下に最初の branch を 1〜`max_children` 本起こす。待たずに終わる | スクリプトを書かない・実行しない |
| `sci-exp-branch` | branch | スクリプト 1 本（モデル + パラメータの探索範囲 + 学習 + テスト）を書き、実行先に投げる。始まりを確かめたら、ジョブ待ちの review の CARD を作って終わる | 終わりまで待たない・判定しない |
| `sci-exp-review` | review | ジョブが終わると daemon が起こす。台帳を読んで判定し、次の branch を 0〜`max_children` 本起こすか、止めて完了報告書を書く | スクリプトを書かない・実行しない・台帳の数値を自分で計算しない |

- research-tree への置き方（op。出せる op は `# context` の `## ALLOWED_OPS` を見る）:
  - sprout: SEED から下る PATH（終点の A を 1 つ以上）を集めて `add_sprout` で SPROUT を作る。最初の branch のために、その SPROUT の下に E（1 回の実行）を `mlflow` なしで作り、CARD を作って branch の run_id が決まってから、`set_mlflow` で `ID<branch の run_id>-p01`（run 名の頭）を付ける。
  - 1 回の実行の E（BRANCH）は、SPROUT（S）か、前の E の下に `add_e` で作る。
  - review: 結果は木に F として足さない。REPORT と MLflow に書く（MLflow の run 名と値。例 `ID125-p02 test_acc=0.871`）。続けるなら次の branch の E を足す。
  - 課題が解けたと見たら、A の ID と理由を REPORT に書く。★ を付けるのは利用者である。`link` の op は無い。
- 判定の材料（最良・改善の有無・格子の端・漏洩の疑い・debug の深さ）は**台帳コマンド（§5）が計算する**。LLM はその結果を読み、判定の中身（次に試すパラメータの値、新しいモデルの案、直す点）だけを書く。
- 1 つの run は 1 つの仕事だけをして終わる。sprout も branch も review も、ほかの run の終わりを待たない。

## 2. 文書の渡し方

チャットは次の人に届かない。次に必要なことは文書で渡し、文書だけで完結させる。

- `sai-link doc write` は `--filename`・`--description` と標準入力の本文で保存し、戻ってきた **waggle トークン**（例 `AXANSMpo`）を受け取る。

  ```bash
  sai-link doc write --filename exp-run-A6-ID120-ID125.md --description "作業報告書 A6-ID120 ID125: tune cnn-static — job 7 running" <<'EOF'
  （本文）
  EOF
  ```

- `--description` の形は **`<文書の種類> <ID>: <1 文>`** にする。他のエージェントは一覧の `description` だけを見て、読むかどうかを決める。
- 文書を渡すときは必ず **トークン**で渡す。ファイルパスだけで渡さない（run フォルダは移動するので、パスはすぐに無効になる）。依頼文には `waggle:<トークン>` と書けば、実行時にその位置へ本文が埋め込まれる（1 段だけ）。
- あとで読み返すときは `sai-link doc read <トークン>`。一覧を出すだけなら `sai-link doc list`。
- 改訂したら `-r2` を付けた新しい名前で保存する。古いトークンを参照している文書があれば、それも改訂する。
- 作業メモや途中経過は保存しない。保存するのは最終成果物だけである。

## 3. 実験仕様書

- ファイル名 `exp-spec-<study>.md`、description `実験仕様書 <study>: <研究の 1 文>`。
- **sprout だけが作る。** 途中で書き換えない。
- 本文の先頭に、次の JSON を ```json のコードブロックで 1 つだけ置く。値の出どころは最初のメッセージの「# context」と `sai-sci status --json` の `nodes[].guide`（`purpose.metric` の Q）。

```json
{"study": "S3-ID120", "node": "S3", "sprout_run_id": 120, "project": "west",
 "metric": "test_acc", "direction": "maximize", "success": 0.85, "min_delta": 0.0001,
 "max_branches": 8, "max_children": 2, "max_debug": 3, "max_fail": 3,
 "allowed_actions": ["draft", "tune", "debug", "model", "ablation"],
 "epochs": 25, "trial_timeout_s": 5400, "device": "auto",
 "artifact_location": "", "workdir": "experiments/S3-ID120",
 "reference_code": [{"id": "miscco/NM_TC", "sha": "1c8f5f09d3a9", "path": "…", "role": "土台"}],
 "sim_model": null,
 "return_to": {"block": "professor-backward", "run_id": 5}}
```

| 欄 | 中身 |
|---|---|
| `study` | `<S の ID>-ID<sprout の run_id>`。sprout の run_id は環境変数 `SAI_PARENT_RUN_ID` |
| `node` | 実験の対象の Node（SPROUT の ID。例 `S3`） |
| `sprout_run_id` | sprout の run_id |
| `project` | `SAI_PROJECT_ROOT` の末尾のフォルダ名 |
| `metric` / `direction` / `success` | 最終の指標名（テスト集合で測るもの。例 `test_acc`）・向き・成功基準。指標の Q（`sai-sci status --json` と `sai-sci node` で探す）から読む。決まらなければ質問ツールで起動者に聞く（推測で埋めない） |
| `min_delta` | 「よくなった」と数える最小の幅。既定 `0.0001` |
| `max_branches` | この sprout の branch の総数の上限。既定 `8` |
| `max_children` | 1 回の review（と sprout）が起こす branch の数の上限。既定 `2` |
| `max_debug` | debug が続けてよい回数。既定 `3` |
| `max_fail` | 同じ experiment で改善しない branch が続いてよい本数。既定 `3` |
| `allowed_actions` | 使ってよい action。既定は 5 つ全部。再現（逆向きの段 3）は `["draft", "tune", "debug"]`（モデルを変えない） |
| `epochs` | 1 試行の epoch 数 |
| `trial_timeout_s` | 1 試行の時間の上限（秒）。既定 `5400`。スクリプトに環境変数 `TRIAL_TIMEOUT_S` で渡す |
| `device` | `cuda`（GPU が無ければ止める）/ `auto`（cuda → mps → cpu）/ `cpu`。既定 `auto`。実行先が GPU の PC なら `cuda` |
| `artifact_location` | 成果物の置き場（空なら環境変数 `EXP_ARTIFACT_LOCATION`、それも無ければ `s3://mlflow-local/`）。大きなモデルで別の置き場（さくらのオブジェクトストレージなど）を使うときに書く |
| `workdir` | スクリプトを置くフォルダ。既定 `experiments/<study>` |
| `return_to` | sprout を起こしたのが professor なら `{"block": "<そのブロック>", "run_id": <その run_id>}`。人が起こしたなら `null`。実験の流れが終わったときに起こし直す相手（§11） |
| `reference_code` | 参照実装の一覧。`sci-exp-scout` の参照コード調査書からそのまま写す（`{"id", "url", "sha", "path", "license", "role", "paper"}`）。`role` は `土台` か `参考`。参照実装なしで進むと起動者が決めたときだけ `[]` にして、理由を本文に書く |
| `sim_model` | モデル定義書のトークン（`sci-sim-model` / `sci-sim-compose` の出力）。神経のモデルでない実験は `null`。`null` でなければ、branch は式・値・再現の目標をそこから写し、`§4` の再現の確かめを記録する |

- 仕様書の本文には「## パラメータの出典表」を置く（書き方は `exp-ref` §6）。モデルの名前付き定数と、掃引の軸の幅を、出典つきで載せる。
- `sim_model` が `null` でなければ、**本文の「## パラメータの出典表」はモデル定義書の「## パラメータの出典表」を写す**（scout の下書きより優先する）。さらに**「## 再現の目標」の表も本文に写す**。

## 4. MLflow の記録の決まり

- **branch 1 つ = experiment 1 つの中の run の列。run 1 つ = パラメータの組 1 つの 1 回の実行（学習 + テスト）。run は入れ子にしない。**
- experiment 名は `<project>/ID<sprout_run_id>/<model>`（例 `west/ID120/cnn-static`）。`model` は `^[a-z0-9][a-z0-9-]{0,39}$`。tune と debug は親の branch と同じ `model`（= 同じ experiment）。draft・model・ablation は新しい `model`（= 新しい experiment）。ablation は `<元の model>-abl-<外した成分>`。
- experiment が無ければ `create_experiment(name, artifact_location=…, tags=…)` で作る。`mlflow.set_experiment` は使わない（`/mlruns` に作られ、実行先から `log_artifact` が `PermissionError` になる）。接続先は環境変数 `MLFLOW_TRACKING_URI` に任せ、スクリプトの中で上書きしない。
- experiment のタグ（作るときに付ける。すべて文字列）: `sai.project`・`sai.sprout_run_id`・`sci.node`・`exp.study`・`exp.model`・`exp.metric`・`exp.direction`。
- run 名は `ID<branch の run_id>-p<NN>`（`NN` は GRID の組の順の 2 桁、01 から）。
- run のタグ:

| タグ | 中身 | 例 |
|---|---|---|
| `sai.project` | プロジェクトのフォルダ名 | `west` |
| `sai.run_id` | branch の run_id（環境変数 `EXP_BRANCH_RUN_ID`。**無ければスクリプトは止まる**） | `125` |
| `sai.sprout_run_id` | sprout の run_id | `120` |
| `sci.node` | 実験の対象の Node（`node` と同じ値） | `S3` |
| `exp.study` | study | `S3-ID120` |
| `exp.model` | model | `cnn-static` |
| `exp.action` | `draft` / `tune` / `debug` / `model` / `ablation` | `tune` |
| `exp.parent_branch` | 親の branch の run_id（無ければ空文字） | `121` |
| `exp.trial` | `NN` | `02` |
| `exp.metric` / `exp.direction` | 仕様書の値 | `test_acc` / `maximize` |
| `exp.device` | 実際に使った装置 | `cuda:0` |
| `exp.error` | 失敗した試行だけ。例外の 1 行目（500 字まで） | `timeout: 5400s を超えました` |

- params: GRID の値と、固定の設定（`FIXED`）を全部。
- metrics: epoch ごとに `train_loss`・`val_loss`（`step` = epoch、1 始まり。必須。神経のシミュレーションの再現の確かめは `EPOCHS = 1` で `train_loss` を省いてよい）。ほかの途中の指標（`val_acc` など）も同じ形で記録してよい。試行の最後に、仕様書の `metric`（例 `test_acc`）と `exec_time_s` を step 無しで 1 回。
- 標準出力（`exp_exec.py logs` と `started` が読む。必須）: 最初に `device=<cuda:0|mps|cpu>`。試行の始まりに `[trial <NN>/<総数>] <組の JSON>`。epoch ごとに `[epoch <e>/<E>] train_loss=<x> val_loss=<y> (<経過秒>s)`。
- 失敗した試行は run が `FAILED` で閉じ、`exp.error` が付く。1 つの組が失敗しても残りの組を続ける。
- 1 試行の時間が `TRIAL_TIMEOUT_S` を超えたら、その試行を `exp.error = "timeout: …"` で失敗にして次の組へ進む。
- `device` が `cuda` で GPU が見えないときは、run を 1 つも作らずに終了コード 3 で止まる（CPU で黙って走らない）。
- 成果物: 各 run にスクリプト本体（`code/`）。
- スクリプトの置き場: `<workdir>/ID<branch の run_id>/train.py`。共通の部品（データの読み込みなど）は `<workdir>/common/` に置いて import してよい（branch ごとに同じファイルを写さない）。

### 再現の確かめ（`sim_model` があるとき）

仕様書の `sim_model` が `null` でなければ、**すべての試行（run）で**次の 2 つを **step 無しで 1 回だけ** MLflow に記録する。

- `repro.<name>`（`name` はモデル定義書の `repro[].name` ごと）: `baseline` の設定で測った値。
- `repro_pass`: `range` に入った目標の割合（0〜1）。

**土台が論文どおりに動くかの確かめは、改良や組み合わせより先である。** 1 つも記録しない試行があれば、台帳の `target.repro_pass` が `null` になり、review は debug に戻す（`§6` の R1a）。

- `baseline` の計算はスクリプトの中で 1 回だけ行い、すべての試行で同じ値を記録してよい（`_reference` の関数に置く）。
- 神経のシミュレーションは学習が無いので `EPOCHS = 1` とする。`train_loss` が無いと台帳の `has_curves` が false になるが、判定の規則は `has_curves` を使わないので、そのままでよい。`[epoch …]` の行を出すために `log_epoch(1, …)` に指標の材料（特徴量）を渡してよい。

定数（`REPRO` と `BASELINE_SWITCHES`）と、次の関数を、`train()` を書くところに足す。**`train()` の返り値に `**repro_check(params)` を混ぜれば**、雛形の `main` がそのままこの 2 つを `log_metric` する。

```python
# モデル定義書の「## 再現の目標」と「## パラメータの出典表」から写す
REPRO = {
    "so_peak_hz": (0.5, 1.5),  # src: #9.5.2
}
BASELINE_SWITCHES = {"use_im_nap": False, "col_coupling": 0.0}

_BASE: dict = {}


def repro_check(params: dict) -> dict:
    """baseline の設定で 1 回だけ測り、モデル定義書の再現の目標と比べる。"""
    if not _BASE:
        measured = features(simulate({**params, **BASELINE_SWITCHES}))
        hits = [lo <= measured[name] <= hi for name, (lo, hi) in REPRO.items()]
        _BASE.update({f"repro.{name}": measured[name] for name in REPRO}, repro_pass=sum(hits) / len(hits))
    return dict(_BASE)


def train(params: dict, device: str, log_epoch) -> dict:
    measured = features(simulate(params))
    log_epoch(1, **measured)  # 学習が無いので epoch は 1 回。特徴量を途中の指標として出す
    return {METRIC: score(measured), **repro_check(params)}
```

- `REPRO` は `{名前: (下限, 上限)}` の形で、**各行に `# src:` にモデル定義書の `repro[].src` を書く**（出典が無い値は使わない）。
- `BASELINE_SWITCHES` はモデル定義書の `baseline.switches` をそのまま使う（改良・つなぎを外すための値）。
- `simulate`・`features`・`score` は branch が書く関数（模擬・特徴量・仕様書の `metric` の計算）。**tune と debug で親のスクリプトを写すときも `repro_check` を残す。**

### スクリプトの雛形

branch が定数・`FIXED`・`GRID`・`train` の中身だけを書く。MLflow と出力の行は書き換えない。**先頭の PEP 723 の欄は消さない。** 追加の道具はここに書き足す（`uv run <script>.py` はこの欄の依存を読む）。`boto3` が無いと `s3://` への成果物の書き込みができない。

**モデルの数値の定数と、数値を使う関数には `# src:` を付ける**（書き方は `exp-ref` §5）。雛形の定数（`PROJECT` … `RUN_TAGS`）には要らない。`FIXED`・`GRID` の行には、値の出どころ（`仕様書` か `目安`）を付ける。

```python
# /// script
# requires-python = ">=3.10"
# dependencies = ["mlflow", "boto3"]
# ///
import itertools, json, os, time
import mlflow
from mlflow.tracking import MlflowClient

# 定数（branch が書く。値は依頼文と実験仕様書から）
PROJECT, SCI_NODE, STUDY = "west", "S3", "A6-ID120"
SPROUT_RUN_ID, MODEL = "120", "cnn-static"
ACTION, PARENT_BRANCH = "draft", ""
METRIC, DIRECTION = "test_acc", "maximize"
EPOCHS, DEVICE = 25, "auto"
EXPERIMENT = f"{PROJECT}/ID{SPROUT_RUN_ID}/{MODEL}"
ARTIFACT_LOCATION = os.environ.get("EXP_ARTIFACT_LOCATION") or "s3://mlflow-local/"
BRANCH_RUN_ID = os.environ["EXP_BRANCH_RUN_ID"]  # 無ければここで止まる（sai.run_id を空にしない）
TRIAL_TIMEOUT_S = float(os.environ.get("TRIAL_TIMEOUT_S", "5400"))

FIXED = {"batch_size": 50}  # src: 仕様書
GRID = {"lr": [0.01, 0.03], "dropout": [0.3, 0.5]}  # src: 目安

EXP_TAGS = {"sai.project": PROJECT, "sai.sprout_run_id": SPROUT_RUN_ID, "sci.node": SCI_NODE,
            "exp.study": STUDY, "exp.model": MODEL, "exp.metric": METRIC, "exp.direction": DIRECTION}
RUN_TAGS = {**EXP_TAGS, "sai.run_id": BRANCH_RUN_ID, "exp.action": ACTION,
            "exp.parent_branch": PARENT_BRANCH}


class TrialTimeout(Exception):
    pass


def pick_device() -> str:
    try:
        import torch
    except ImportError:
        torch = None
    has_cuda = bool(torch is not None and torch.cuda.is_available())
    if DEVICE == "cuda" and not has_cuda:
        print("device=none（仕様書は cuda を求めているが GPU が見えない）", flush=True)
        raise SystemExit(3)
    if DEVICE == "cpu":
        return "cpu"
    if has_cuda:
        return "cuda:0"
    mps = getattr(getattr(torch, "backends", None), "mps", None)
    if mps is not None and mps.is_available():
        return "mps"
    return "cpu"


def train(params: dict, device: str, log_epoch) -> dict:
    """研究の本体。epoch ごとに log_epoch(epoch, train_loss=…, val_loss=…) を呼び、
    最後に {METRIC: 値}（ほかの最終の指標も入れてよい）を返す。"""
    ...


def main() -> None:
    device = pick_device()
    print(f"device={device}", flush=True)
    client = MlflowClient()
    exp = client.get_experiment_by_name(EXPERIMENT)
    exp_id = exp.experiment_id if exp else client.create_experiment(
        EXPERIMENT, artifact_location=ARTIFACT_LOCATION, tags=EXP_TAGS)
    combos = [dict(zip(GRID, values)) for values in itertools.product(*GRID.values())]
    for i, combo in enumerate(combos, 1):
        nn = f"{i:02d}"
        print(f"[trial {nn}/{len(combos):02d}] {json.dumps(combo)}", flush=True)
        start = time.time()

        def log_epoch(epoch: int, **metrics: float) -> None:
            for key, value in metrics.items():
                mlflow.log_metric(key, float(value), step=epoch)
            shown = " ".join(f"{k}={float(v):.4f}" for k, v in metrics.items())
            print(f"[epoch {epoch}/{EPOCHS}] {shown} ({time.time() - start:.1f}s)", flush=True)
            if time.time() - start > TRIAL_TIMEOUT_S:
                raise TrialTimeout(f"timeout: {TRIAL_TIMEOUT_S:.0f}s を超えました")

        try:
            with mlflow.start_run(experiment_id=exp_id, run_name=f"ID{BRANCH_RUN_ID}-p{nn}",
                                  tags={**RUN_TAGS, "exp.trial": nn, "exp.device": device}):
                mlflow.log_params({**FIXED, **combo})
                mlflow.log_artifact(__file__, artifact_path="code")
                try:
                    final = train({**FIXED, **combo}, device, log_epoch)
                except Exception as exc:
                    mlflow.set_tag("exp.error", str(exc)[:500])
                    raise
                for key, value in final.items():
                    mlflow.log_metric(key, float(value))
                mlflow.log_metric("exec_time_s", time.time() - start)
        except Exception as exc:  # 1 つの組が失敗しても残りの組を続ける
            print(f"[trial {nn}] failed: {str(exc)[:200]}", flush=True)


if __name__ == "__main__":
    main()
```

## 5. 台帳

ここでいう台帳は**実験の台帳**（`exp_summary.py`。MLflow の記録を集計する）である。論文の番号を記録する「見た番号の台帳」（`.sai/seen_ids/<run_id>`。`sci-core` の引用の決まり）とは別の物である。

### 台帳コマンド `exp_summary.py`

置き場所は `$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py`（`sai-sci init` が配る。`SAI_PROJECT_ROOT` は sai-link が必ず入れる）。**呼び出すときは必ずこの形を使う**（別のファイルを探したり、パスを作り変えたりしない）。ファイルが無いときは `BLOCKED` で報告して終える。

`--python ">=3.10"` は、`~/.python-version` などが入っていない Python を指していても、使える Python で動かすための指定である。**外さない。**

呼び出し:

```bash
uv run --no-project --python ">=3.10" --with mlflow python "$SAI_PROJECT_ROOT/.sai/tools/exp_summary.py" \
  --sprout <id> --metric <名前> --direction maximize|minimize \
  [--branch <id>] [--min-delta 0.0001] [--leak-max 0.999] [--leak-min 1e-6] [--tracking-uri <URI>]
```

- 終了コード: 0 = 成功（該当 run が 0 件でも 0）、1 = 実行中の失敗、2 = 引数の誤り・接続先が無い。
- **`--direction` は省略しない**（必須引数）。
- 標準出力（JSON 1 つ）:

```json
{"sprout": 120, "metric": "test_acc", "direction": "maximize", "min_delta": 0.0001,
 "best": {"experiment": "west/ID120/cnn-static", "branch": 125, "run_name": "ID125-p02",
          "run_id": "…", "value": 0.871, "params": {"lr": "0.01"}},
 "experiments": [{"name": "west/ID120/cnn-static", "experiment_id": "7", "model": "cnn-static",
                  "branches": [121, 125], "runs": 9, "failed": 1, "best": 0.871, "best_branch": 125,
                  "stale": 0, "has_curves": true}],
 "branches": [{"branch": 125, "experiment": "west/ID120/cnn-static", "model": "cnn-static",
               "action": "tune", "parent_branch": 121, "depth": 2, "debug_depth": 0,
               "runs": 6, "failed": 0, "best": 0.871, "best_params": {"lr": "0.01"},
               "improved": true, "has_curves": true, "param_edges": [{"param": "lr", "side": "max"}],
               "repro_pass": null}],
 "target": {"branch": 125, "runs": 6, "failed": 0, "best": 0.871, "improved": true,
            "repro_pass": null, "repro": null},
 "total_branches": 3, "ablation_done": false,
 "leak_suspects": [{"branch": 125, "run_name": "ID125-p05", "value": 0.9995}],
 "warnings": []}
```

### 各欄の意味

- `best`: direction に従った最良の run。成功した run が 0 件なら `null`。
- `experiments[]`: experiment ごとのまとめ。`stale` は、その experiment の branch を古い順に見て、最後から数えて「それまでの最良を `min_delta` を超えて更新しなかった branch」が続いた本数。
- `branches[]`: branch ごとのまとめ。`depth` は親をたどった深さ（親が無ければ 1）。`debug_depth` は自分から親へ `debug` が続く本数（自分を含む）。`improved` は同じ experiment の前の branch の最良を `min_delta` を超えてよくしたか。`param_edges` は最良の run の値が試した値の端だったパラメータ。`repro_pass` は、その branch の成功した run が記録した `repro_pass` の最小値。記録が 1 つも無ければ `null`。
- `target`: `--branch` を付けたときの、その branch の要約。`repro_pass` は `branches[]` と同じ。`repro` は、`repro_pass` が最小の run の `repro.<name>` の値（`{"so_peak_hz": 2.3}` の形。頭の `repro.` を外す）。記録が無ければ `null`。
- `total_branches`: run のある branch の数。
- `ablation_done`: `exp.action=ablation` の branch が 1 つでもあれば true。
- `leak_suspects`: maximize なら `value >= leak_max`、minimize なら `value <= leak_min` の成功 run。

**この数値を自分で作り直さない。** 命令の結果をそのまま読む。

## 6. 判定の規則

review が使う。台帳（§5）の欄だけが材料である。

### 判定書

- ファイル名 `review-<study>-ID<branch>.md`、description `判定書 <study> ID<branch>: <decision> <rule> — <理由 1 文>`。
- 本文の先頭に、次の JSON を ```json のコードブロックで 1 つだけ置く。

```json
{"study": "A6-ID120", "sprout_run_id": 120, "branch": 125, "job_state": "done",
 "decision": "continue", "rule": "R7",
 "next": [{"action": "tune", "parent_branch": 125, "model": "cnn-static",
           "grid": {"lr": [0.1, 0.3]}, "idea": "", "fix": ""}],
 "stop_reason": "none"}
```

- `decision` は `continue` か `stop`。`stop_reason` は `success` / `max_branches` / `not_allowed` / `cancelled` / `no_records` / `repro_failed` / `none`。
- `next` は 0〜`max_children` 個（`continue` なら 1 個以上）。`grid` は tune のとき、`idea` は model と ablation のとき、`fix` は debug のときに入れる。起こした branch は `--origin-run` で結ばれ、`sai-link run tree` で分かるので、判定書には書かない。

### 規則

上から順に当てはめ、最初に当てはまったものを採る。台帳は `--branch <その branch>` で取る。「達した」は maximize なら `best.value >= success`、minimize なら `<= success`。

| # | 条件 | decision | next |
|---|---|---|---|
| R0 | ジョブの state が `cancelled` | stop（`cancelled`） | 無し |
| R1 | `total_branches >= max_branches` | stop（`max_branches`） | 無し |
| R1a | 仕様書の `sim_model` が `null` でなく、成功した run が 1 つ以上ある（`target.runs > target.failed`）のに、`target.repro_pass` が `null` か 1 未満 | `debug_depth < max_debug` なら continue。そうでなければ stop（`repro_failed`） | debug を 1 本（親はこの branch、同じ experiment）。`fix` に、外れた目標（台帳の `target.repro` の値とモデル定義書の範囲）か「repro_pass が記録されていない」と書く |
| R2 | `best` が達し、`ablation_done == false`、`ablation` が使える | continue | ablation を 1 本（親は `best.branch`、新しい experiment） |
| R3 | `best` が達した（R2 に当たらない） | stop（`success`） | 無し |
| R4 | `target.runs == 0`（記録が 1 つも無い）で、親の branch も `runs == 0` | stop（`no_records`）。MLflow への接続を疑い、質問ツールで起動者に聞く | 無し |
| R5 | 成功した run が 0（`target.runs == target.failed`）で、`debug_depth < max_debug` | continue | debug を 1 本（親はこの branch、同じ experiment。`fix` に直す点。`logs` と `exp.error` から書く） |
| R6 | `leak_suspects` にこの branch の run がある | continue | debug を 1 本（`fix` に漏洩を疑う点） |
| R7 | `debug_depth >= max_debug`、または この experiment の `stale >= max_fail` | `model` が使えれば continue、使えなければ stop（`not_allowed`） | model を 1 本（親は `best.branch`、無ければこの branch。新しい experiment。`idea` に新しいモデルの案） |
| R8 | 上のどれにも当てはまらない | continue | tune を 1 本（親はこの branch、同じ experiment）。`param_edges` があれば端の外側へ、無ければ最良の近くを細かく。`max_children >= 2`・`model` が使える・この experiment の `stale >= 1` なら、model を 1 本足してよい |

- 起こす本数は、`total_branches + 起こす本数 <= max_branches` に収める（収まらない分は起こさない）。
- パラメータの値だけを変えるのが tune、スクリプトの誤りを直すのが debug。どちらも同じ `model`（同じ experiment）。`train()` の中身（モデルの構造・手法）を変えるのが model、成分を外すのが ablation。どちらも新しい `model`。

## 7. branch の依頼書

- sprout と review は、branch 1 本ごとに**依頼書**を `sai-link doc write` で保存し、そのトークンを `sai-sci card` の `--input` で渡す（`sai-sci card` には本文を渡す引数が無い。`--input` のトークンは依頼文の末尾に `waggle:` で載り、実行時に本文が埋め込まれる）。
- ファイル名 `branch-order-<study>-<連番 2 桁>.md`（連番はその run の中で 01 から）、description `branch の依頼書 <study>: <action> <model>（親 ID<parent_branch>）`。
- 本文の**先頭の 5 行は `key: value`**:

```text
study: A6-ID120
sprout_run_id: 120
action: tune
parent_branch: 125
model: cnn-static

### スクリプトに書く中身
- 元のスクリプト: experiments/A6-ID120/ID125/train.py（tune と debug のとき。写して直す）
- 土台のコード: <reference_code の path>（式と値はここと仕様書の出典表から写す）
- モデル定義書: <sim_model のトークン>（式・値・再現の目標・baseline はここから写す。無ければこの行を書かない）
- GRID: {"lr": [0.1, 0.3]}
- 直す点 / 新しいモデルの案: …
- 期待する効果: test_acc を 0.87 以上にする
```

- `parent_branch` は無ければ `-`。draft のときは「元のスクリプト」の代わりに「モデルの案」と、データの置き場・読み込み方（research-tree の F と仕様書から）を書く。
- CARD の作り方（sprout と review で共通）:

```bash
sai-sci card --node <Node の ID> --block sci-exp-branch \
  --purpose "<study> <action> <model>（親 ID<parent_branch>）" \
  --input <実験仕様書のトークン> --input <依頼書のトークン> [--input <判定書のトークン>] \
  --origin-run "$SAI_PARENT_RUN_ID" --executer human --json
sai-link card run <返った run_id>
```

- `sai-link run tree` で、その run が `running` になったことを確かめる。動いていなければ `sai-link card run` をもう一度打ち、返り値を確かめる。

## 8. 実行場所の規則

スクリプトは**実行先（target）**に投げて動かす。実行先は名前を持つフォルダ（`.sai/exec/<名前>/`）で、Mac 本体・LAN の PC・クラウドなどを利用者が個別に用意する。**どの実行先でも、呼び方は同じ**である（使い方は `exp-run`）。

- 実行先の名前は環境変数 `EXP_TARGET`（空なら `local`）。別の実行先を勝手に選ばない。投げる前に仕様書の `workdir` へ `cd` する。実行先には、この cwd の中身（コード）が送られる。スクリプトは workdir からの相対パスで書く。
- 投げる命令:

```bash
cd "$SAI_PROJECT_ROOT/<workdir>"
uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" \
  run "${EXP_TARGET:-local}" --name ID<自分の run_id> -- \
  env EXP_BRANCH_RUN_ID=<自分の run_id> TRIAL_TIMEOUT_S=<仕様書の trial_timeout_s> \
  uv run --python ">=3.10" --with mlflow ID<自分の run_id>/train.py
```

  `artifact_location` が空でなければ `EXP_ARTIFACT_LOCATION=<値>` も `env` に足す。**スクリプトの前に `python` を付けない**（PEP 723 の欄が読まれなくなる）。
- 1 行目の job_id を控え、**`started` を 1 回だけ呼ぶ**:

```bash
uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/exp_exec.py" started <job_id> --timeout 600 --json
```

| `started` の結果 | branch がすること |
|---|---|
| `started: true` | 始まった。review の CARD を作って終わる |
| `started: false` で `state` が `queued` / `running` | 実行先の待ち行列にいる（または最初の epoch が長い）。そのまま review の CARD を作って終わる |
| `started: false` で `state` が `failed` | `log_tail` を読む。スクリプトの誤り（import・構文・パス・定数）なら直して投げ直す（**2 回まで**）。3 回目も失敗したら、そのまま review の CARD を作って終わる（review が debug を起こす） |
| `started: false` で `state` が `lost` / `cancelled` | review の CARD を作って終わる |

- **ジョブの終わりまで待たない。`sleep` で待ち合わせない。**
- review の CARD:

```bash
sai-sci card --node <Node の ID> --block sci-exp-review \
  --purpose "<study> ID<自分の run_id> の結果の判定" \
  --input <実験仕様書のトークン> --input <自分の作業報告書のトークン> \
  --origin-run "$SAI_PARENT_RUN_ID" --executer human --json
sai-link card wait <返った run_id> --job <job_id>
```

- `sai-sci init` が `.sai/settings.json` に `job_status` を書く。無いと `sai-link card wait` は終了コード 2。

- 記録は実行先から**中央の MLflow に直接**入る（回収の手順は無い）。branch は自分で台帳を取り直さない。
- 実行先が使えないとき（`check` が失敗、`run` が断られる）は、直そうとせず作業報告書に `BLOCKED` と書き、review の CARD は作らずに終える（起動者に質問ツールで知らせる）。

## 9. 作業報告書（branch）

- ファイル名 `exp-run-<study>-ID<自分の run_id>.md`、description `作業報告書 <study> ID<run_id>: <action> <model> — job <job_id> <state>`。
- 表: スクリプト（`<workdir>/ID<run_id>/train.py`）・experiment 名・実行先・job_id・`started` の結果（state・started・device・最初の進捗の行）・試行の数（GRID の組の数）・親から変えた点・投げ直した回数。判断（次に何をするか）は書かない。

## 10. 完了報告書（止めた review）

- ファイル名 `exp-final-<study>.md`、description `完了報告書 <study>: <stop_reason> — 最良 <metric> <value>`。
- 本文: 最良（experiment・branch・run 名・値・パラメータ）／experiment ごとの表（model・branch の数・run の数・失敗・最良）／branch ごとの表（branch・親・action・model・最良・作業報告書のトークン・判定書のトークン）／research-tree に出した op（`sai-sci op` の結果）／止めた理由（`stop_reason` と rule）。値は台帳から写す（自分で計算しない）。

## 11. 止める条件と、実験の流れの終わり

- 止めるのは判定が `stop` のときだけ（R0・R1・R1a・R3・R4・R7 の一部）。
- 止めた review は、完了報告書を保存したあと、仕様書の `return_to` を見る:
  - `return_to` があれば、`sai-link run md --kind task <return_to.run_id>` の frontmatter から `runner`・`model`・`thinking` を読み、`sai-link card create` で同じブロックの CARD を作って `sai-link card run` で起こす（JSON の形は `sci-exp-review` に書く）。`sai-link run tree` で running を確かめる。
  - `return_to` が `null` なら起こさない。
- どちらの場合も最後に `sai-link run finalize` を呼ぶ（review は daemon が起こした root の run）。
