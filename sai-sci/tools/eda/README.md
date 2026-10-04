# eda: データの中身をざっと見る道具

## 出典

- リポジトリ: [K-Dense-AI/scientific-agent-skills](https://github.com/K-Dense-AI/scientific-agent-skills)
- commit: `1549884`（plugin.json の version 2.72.0）
- 写し元: `skills/exploratory-data-analysis/scripts/`
- ライセンス: MIT（`LICENSE` に全文を置いてある。Copyright (c) 2025 K-Dense Inc.）

## 使う前の決まり

**中身は写し元のまま。** 直すときは写し元（`~/workdir/Agent/sci-skills/k-dense`）を
更新してから写し直す。日本語に直したり sai-sci 向けに書き換えたりしない。

 一字ずつ同じである。差分を確認するには次を実行する。

```bash
diff -r -x __pycache__ -x __init__.py \
  ~/workdir/Agent/sci-skills/k-dense/skills/exploratory-data-analysis/scripts \
  sai-sci/tools/eda
```

違うのは `LICENSE` と `README.md` の 2 つだけのはずである。

## 呼び方

`EDA="$SAI_PROJECT_ROOT/.sai/tools/eda"` と置いたときの呼び方。どれも
**標準ライブラリだけ**で動く（`uv run --no-project --python ">=3.11"`）。

| 命令 | 何をするか |
|---|---|
| `uv run --no-project --python ">=3.11" python "$EDA/capability_manifest.py" inspect <file> --root <dir>` | 形式を判定する（読める形式か） |
| `… "$EDA/eda_analyzer.py" <file> --root <dir> --reveal-identifiers --format json --output <out.json>` | 形式に合わせた EDA の集約 |
| `… "$EDA/tabular_profile.py" <csv> --root <dir> --reveal-identifiers --output <out.json>` | 表の列ごとの型・欠損・分布 |
| `… "$EDA/missingness_leakage_audit.py" <csv> --root <dir> [--group-column G] [--entity-column E] [--split-column S] [--time-column T] --reveal-identifiers --output <out.json>` | 欠損の偏りと分割の漏洩 |
| `… "$EDA/distribution_sensitivity.py" <csv> --root <dir> [--column C] --reveal-identifiers --output <out.json>` | 外れ値・裾の重さ・変換したときの感度（データは変えない） |
| `… "$EDA/report_scaffold.py" --input <file> --root <dir> --output <out.md>` | EDA レポートの骨組み |

`--root` は入出力してよいフォルダの境目である。値は読み取りだけ（`distribution_sensitivity.py`
はデータを変えない）。識別子は `--reveal-identifiers` を付けないと伏せ字になる。

## `report_scaffold.py` について

この 1 本だけは同梱の `assets/report_template.md` を読み、`scripts/` と同じ階層の
`assets/` を探す（`report_scaffold.py:21`）。その `assets/` は写し元の `scripts/` の
**外**にあるため、今回の写しには含まれない。テンプレートが無いときは理由を出して
終了コード 2 で終える（例外ではない）。レポートの骨組みが要るときは、写し元の
`skills/exploratory-data-analysis/assets/report_template.md` も持ってきて
`sai-sci/tools/assets/` に置く（配る先を U06 と相談すること）。

## このフォルダのファイル

| ファイル | 何 |
|---|---|
| `eda_analyzer.py` | 入口。形式を判断して適切な集約へ回す |
| `capability_manifest.py` | その形式を読めるか調べる |
| `tabular_profile.py` | 表の列ごとの集約 |
| `missingness_leakage_audit.py` | 欠損の偏りと分割の漏洩 |
| `distribution_sensitivity.py` | 分布の感度（外れ値・裾・変換） |
| `report_scaffold.py` | レポートの骨組み |
| `image_inspector.py` | 画像の集約 |
| `sequence_inspector.py` | 配列（FASTA/FASTQ など）の集約 |
| `_capabilities.py` `_common.py` `_structured.py` `_tabular.py` | 上のスクリプトが使う共通部分 |