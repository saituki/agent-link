# biosignal: 生体信号（ECG・EDA など）を調べる道具

## 出典

- リポジトリ: [K-Dense-AI/scientific-agent-skills](https://github.com/K-Dense-AI/scientific-agent-skills)
- commit: `1549884`（plugin.json の version 2.72.0）
- 写し元: `skills/neurokit2/scripts/`
- ライセンス: MIT（`LICENSE` に全文を置いてある。Copyright (c) 2025 K-Dense Inc.）

## 使う前の決まり

**中身は写し元のまま。** 直すときは写し元（`~/workdir/Agent/sci-skills/k-dense`）を
更新してから写し直す。日本語に直したり sai-sci 向けに書き換えたりしない。

一字ずつ同じである。差分を確認するには次を実行する。

```bash
diff -r -x __pycache__ -x __init__.py \
  ~/workdir/Agent/sci-skills/k-dense/skills/neurokit2/scripts \
  sai-sci/tools/biosignal
```

違うのは `LICENSE` と `README.md` の 2 つだけのはずである。

## 呼び方

`BIO="$SAI_PROJECT_ROOT/.sai/tools/biosignal"` と置いたときの呼び方。

**`ecg_hrv_pipeline.py` と `eda_pipeline.py` 以外は標準ライブラリだけ**で動く
（`uv run --no-project --python ">=3.11"`）。

| 命令 | 何をするか | 依存 |
|---|---|---|
| `uv run --no-project --python ">=3.11" python "$BIO/inspect_signal.py" --input <csv> --root <dir> --columns <c1,c2> [--time-column T] [--sampling-rate HZ] --output <out.json>` | 信号の欠落・平坦な区間・時刻の順・サンプリング率 | 標準 |
| `… "$BIO/plan_epochs.py" --events-csv <csv> --sampling-rate HZ --recording-samples N --epoch-start S --epoch-end E --output <out.json>` | イベントに合わせた区間（epoch）の計画 | 標準 |
| `… "$BIO/validate_multimodal.py" --manifest <json> --root <dir> --output <out.json>` | 複数の信号のそろい | 標準 |
| `… "$BIO/generate_synthetic.py" …` | 試しの信号の作り出し | 標準 |
| `uv run --no-project --python ">=3.11" --with neurokit2==0.2.13 --with "pandas<3" --with numpy --with scipy python "$BIO/ecg_hrv_pipeline.py" --input <csv> --column ecg --sampling-rate HZ --root <dir> --output <out.json>` | ECG から HRV | neurokit2 |
| 同上で `… "$BIO/eda_pipeline.py" --input <csv> --column eda --sampling-rate HZ …` | 皮膚電気活動の分解と SCR | neurokit2 |

`--events-csv` を使うときは `--deidentified` が要る。読み取りだけ。
`--root` は入出力してよいフォルダの境目である。

## このフォルダのファイル

| ファイル | 何 | 依存 |
|---|---|---|
| `inspect_signal.py` | 信号の品質（欠落・平坦な区間・時刻・サンプリング率） | 標準 |
| `plan_epochs.py` | イベントに合わせた区間の計画 | 標準 |
| `validate_multimodal.py` | 複数の信号のそろい | 標準 |
| `generate_synthetic.py` | 試しの信号の作り出し | 標準 |
| `ecg_hrv_pipeline.py` | ECG から HRV | neurokit2 |
| `eda_pipeline.py` | 皮膚電気活動の分解と SCR | neurokit2 |
| `_common.py` | 上のスクリプトが使う共通部分 | 標準 |