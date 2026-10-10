# 実験ループ調査メモ（開発者向け）

実験担当 `sci-exp-*` と `sai-sci/tools` の読み合わせ結果。運用の理解と改修前の参照用。
`docs/plan/` は読むだけ。本書は開発メモであり仕様変更ではない。

## 1. 全体像

```
scout → [modeler] → sprout → branch → review → branch → … → stop
```

* 誰も他人の終わりを待たない。文書トークンで引き継ぐ。
* 木を変えるのは `sprout/review` のみ。`branch` は判定しない。
* 判定材料は台帳が計算し、LLM は読むだけ。

## 2. block 別の依存と出力

### 2.1 scout `sai-sci/blocks/sci-exp-scout.md`

* 使う tools： `refcode.py links/gh-search/fetch/params`、`modeldb.py search`
* 使う CLI： `sai-sci node --json`（祖先確認）、`sai-sci op add_f`、`sai-link doc write`、`sai-link task report`
* 出力： 木に F、doc に `ref-scout-<NODE>-ID<run>.md`（先頭 JSON `reference_code[]`＋候補/出典下書き/食い違い/GAP）
* 次へ： トークンを `sprout` 依頼文に `waggle:<トークン>` で埋め込む。`sprout` は `doc read` で読む。

### 2.2 sprout `sai-sci/blocks/sci-exp-sprout.md`

* 使う block： 前＝`scout` 調査書、`sim-model/compose` 定義書（神経モデルのみ）。後＝`sci-exp-branch` を CARD で起こす。
* 使う CLI： `sai-link doc read/write`、`sai-sci status/node --json`（指標取得）、`sai-sci op add_sprout/add_e/add_a`、`sai-sci card`＋`sai-link card run/tree`、`sai-link run finalize|task report`
* 出力： 木に S＋E（1〜`max_children`）、doc に `exp-spec-<study>.md`＋`branch-order-<study>-NN.md`（先頭 5 行 study/sprout_run_id/action/parent_branch/model）
* 次へ： `branch` は CARD `--input` の展開で受け取る。子 `task create` で起こさない（親なし run）。

### 2.3 branch `sai-sci/blocks/sci-exp-branch.md`

* 使う block： 前＝`sprout/review` 依頼書。後＝`sci-exp-review` の CARD 作成。
* 使う tools： `refcode.py check`（`# src:` 検査、NG なら `BLOCKED`）、`exp_exec.py check/run/started`
* 使う CLI： `sai-link doc read/write`、`sai-sci op set_mlflow` のみ、`sai-sci card`＋`sai-link card wait --job`、`sai-link run finalize`
* 出力： FS に `<workdir>/ID<run>/train.py`＋`common/`、MLflow へ直書き、doc に `exp-run-<study>-ID<run>.md`
* 次へ： `review` は daemon が job 終了で起こし、仕様書＋作業報告書の 2 トークンを読む。

### 2.4 review `sai-sci/blocks/sci-exp-review.md`

* 使う tools： `exp_exec.py status/logs` 1 回、`exp_summary.py --sprout --metric --direction --branch`
* 使う CLI： `sai-link doc write`（`review-<study>-ID<branch>.md` または `exp-final-<study>.md`）、`sai-sci op add_e`（continue 時のみ CARD より先）、`sai-sci card`＋`sai-link card run`、`professor` 復帰は `run md --kind task`＋`card create/run`、`sai-link run finalize`
* 出力： 木に E のみ（stop 時は無し）。F 追加なし。MLflow は読むだけ。
* 判定： `exp-core §6 R0〜R8＋R1a` を上から適用。`total_branches＋新規 <= max_branches`。

## 3. 共通契約

* `sai-sci/template/exp-core.md`： 役割表、文書渡し（waggle トークン、1 段のみ）、実験仕様書 JSON、MLflow 記録則（branch＝experiment 内 run 列、run 名 `ID<branch>-pNN`）、台帳 JSON（`best/experiments/branches/target/leak_suspects`）、判定規則、実行場所規則（`exp_exec.py run＋started` のみ）、作業報告書・完了報告書・`return_to` 終了処理。
* `sai-sci/template/exp-run.md`： target は `exp_exec.py` 経由のみ。`ssh/scp` 直操作禁止。`workdir` へ `cd` して投げる。結果は MLflow 直書き。確認ジョブ禁止。
* `sai-sci/template/exp-ref.md`： Library→ModelDB→GitHub の順。`fetch` 最大 3 本。`# src:` は ref/chunk/F/仕様書/目安の 5 種のみ。記憶・出典なしは `BLOCKED`＋質問。`license:null` は写経禁止。

## 4. `train.py` / `workdir/` / `common/`

* `workdir/`： 既定 `experiments/<study>`。`sprout` が仕様書で決める。投げる前にここへ `cd` し、ここが実行先に送られる。仕様書外への書き込み禁止。
* `train.py`： `<workdir>/ID<branch>/train.py`。`branch` のみ書く。定数＋`FIXED`＋`GRID`＋`train()`＋不変 `main`。`draft` 新規、`tune` は GRID のみ変更、`debug` は `fix` のみ修正。`py_compile`＋`refcode.py check` 後に `run`。
* `common/`： `<workdir>/common/`。データ読み込み等の共通部品。あれば import、無ければ作る。`check` は `train.py` 本体しか見ないため、数字隠しの抜け穴になる。

## 5. `sai-sci/tools`

自作 5 本＋借り物 2 箱。ほぼ標準ライブラリのみ。`--json` で JSON 1 つ。終了コード 0/1/2。

| 道具 | 役割 |
|---|---|
| `tools/exp_exec.py` | 分配器。`run/check/started/status/logs/cancel/list`。Mac・LAN・DOK を同形で扱う。`EXP_TARGET`（空なら `local`）。 |
| `tools/exp_summary.py` | 台帳。MLflow から最良・改善・端・漏洩疑い・再現率を計算。 |
| `tools/refcode.py` | 参照コード。`links/gh-search/fetch/params/eqs/check`。置き場 `<sai_home>/refcode/`。手 `clone` 禁止。 |
| `tools/modeldb.py` | 神経モデル図鑑。`catalog.jsonl` に写す。`search/show` はネット不要。`sync` は重いので承認必須。 |
| `tools/data_result.py` | 統計検算。`check`（K1-K9）・`table` は標準のみ、`log` のみ mlflow。 |
| `tools/eda/` | 表データのざっと見。K-Dense 写し。`capability/eda_analyzer/tabular_profile/missingness/distribution`。 |
| `tools/biosignal/` | 生体信号用。K-Dense 写し。`inspect/plan_epochs/validate/generate` は標準のみ、`ecg_hrv/eda_pipeline` のみ neurokit2。 |
| `tools/test_*.py` | 各道具のテスト。偽サーバで確認。 |

実行先本体は `sai-sci/exec/local|ssh|dok/`。`tools/exp_exec.py` が呼ぶ側、`exec/*/adapter` が動かす側。

## 6. eda / biosignal の使用例の在り処

* 呼び方表： `tools/eda/README.md`、`tools/biosignal/README.md`、`--help`、先頭 docstring、`tools/test_data_eda.py`。
* 順番と読み方： `template/data-core.md §5`（表データ全文例）、`§7`（生体信号全文例）、出力の読み方（欠損は疑い、外れ値は削除規則でない）。
* block 指示： `blocks/sci-data-explore.md §3-4,7`（出力先 `analysis/<study>/ID<run>/eda/`、`data_result.py log --kind eda` 例）。
* 版固定： `template/stat-core.md`（neurokit2==0.2.13、pandas<3）。
* `docs/sai-sci.md` は写しである旨のみ。`docs/sai-sci-tutorial.md` に eda 固有例なし。`report_scaffold.py` は配布形では動かない（README 記載）ため使わない。

## 7. MLflow 連携

```
[train.py] --直接--> [中央 MLflow :5001]（指標）
         --直接--> [S3]（成果物、直接方式 --no-serve-artifacts）
[review] <- exp_summary.py <- MLflow 読取
[人] <- Web /api/mlflow/* <- MLflow 読取（書込なし）
```

* サーバは `apps/mlflow-server/`（mlflow＋postgres＋S3＋Caddy）。
* 接続先は env 渡し。`exec/local/target.toml` は `http://127.0.0.1:5001`、`ssh` は LAN IP、`dok` は Funnel https＋Basic 認証。`EXP_ARTIFACT_LOCATION` も同様。
* 記録則： `create_experiment(name, artifact_location=..., tags=...)` で作る。`set_experiment` 禁止。タグ・params・metrics・標準出力（`device=/[trial]/[epoch]`）は雛形通り。
* 読取： `review` は台帳経由、Web は `sai-web` 中継（`SAI_MLFLOW_URL`）＋画面 `mlflow.js/mlfchart.js`。

## 8. 対応範囲と非対応

対応： 文献調査、表・時系列・生体信号（ECG/EDA/HRV）の EDA、統計検定（t/U/χ²/ANOVA/相関/生存/κ/ICC 等、frequentist/bayes/ml/descriptive）、ML 指標最適化の GRID 探索、神経微分方程式の再現確認付きシミュレーション。分野観点は `domain-general/medical/signal-ml`。

非対応： 湿式・被験者介入・臨床実施、機器作製・ロボット・リアルタイム閉ループ、大規模分散学習、独自バイナリ・DB 直結の深掘り、個人情報付き共有、課金ジョブ自動継続。1 run 30 分超は実験側へ委任。読めない形式は停止。

## 9. 長時間ジョブ

1 run 1 day でも回る設計。`branch` は `started` 1 回で `finalize` 帰宅し、`task.md` の `after_job: <job_id>` を daemon が見回って `review` を起こす。条件は daemon 継続、`settings.json:job_status` 必須（無ければ `card wait` exit 2）、`trial_timeout_s` 延長（既定 5400s）、電源・回線・`jobs/*.json` 維持、MLflow 長期開放。

## 10. 見つかった問題点

* 非効率： 人承認 3 箇所（PATH・調査書有無・指標）で停止、`started` 最大 600s＋2 回投げ直しで `timeout:7200` 消費、`workdir` 丸ごと転送（データ同梱で重複）、`TRIAL_TIMEOUT_S` は `log_epoch` 内のみ有効。
* 曖昧： waggle 1 段開封で多段参照の到達保証なし、`tune` の FIXED 可否の文面揺れ、`min_delta`（`>`）と成功（`>=`）の非対称、漏洩閾値一律・カテゴリ変数は端検出なし。
* 矛盾： 待たない建前と承認/`started`/`card wait` 待ちの混在、`license:null` 写経禁止と土台写し指示の衝突、結果三重管理（木・doc・MLflow）の正本不明。
* 穴： 並行 `review` の上限レース、同時実行数は local のみ自前、`review` の 1 回 `status` で空振り→R4 誤判定・daemon 文脈の質問、`repro_pass` 最小値採用の脆さ、env（`EXP_BRANCH_RUN_ID` 等）無しで即死、`lost` 再投入禁止、`job_status` 無しで全滅。
* 欠点： 障害時切替なし（`BLOCKED` 止まり）、PEP723 下限のみで lock なし、GPU 要求はキュー消費後の exit 3、観測・コスト上限なし。

## ソース

* block： `sai-sci/blocks/sci-exp-scout.md`、`sci-exp-sprout.md`、`sci-exp-branch.md`、`sci-exp-review.md`、`sci-data-explore.md`
* template： `sai-sci/template/exp-core.md`、`exp-run.md`、`exp-ref.md`、`data-core.md`、`stat-core.md`、`domain-general.md`、`domain-medical.md`、`domain-signal-ml.md`
* tools： `sai-sci/tools/exp_exec.py`、`exp_summary.py`、`refcode.py`、`modeldb.py`、`data_result.py`、`eda/README.md`、`biosignal/README.md`、`test_data_eda.py`
* exec： `sai-sci/exec/local/adapter`、`local/target.toml`、`ssh/target.toml`、`dok/target.toml`、`dok/README.md`、`dok/SETUP.md`
* CLI/daemon： `crates/sai-link-engine/src/kanban.rs`、`daemon.rs`、`pre_run.rs`、`crates/sai-link-cli/src/phone.rs`、`crates/sai-sci-cli/src/agent_assets.rs`
* Web/MLflow： `apps/web/src/sai_web/mlflow_proxy.py`、`config.py`、`main.py`、`static/mlflow.js`、`apps/mlflow-server/`
* docs： `docs/sai-sci.md`、`docs/sai-sci-tutorial.md`
