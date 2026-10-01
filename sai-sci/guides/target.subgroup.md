---
key: target.subgroup
name: TARGET_SUBGROUP
type: choose
parent: target
depends_on:
min_findings: 2
---
# TARGET_SUBGROUP

対象を、見てみたい範囲で分ける ITEM である。分けないなら分けないと決める。

分け方が決まると、データの切り方と出力の形も決まる。

## QUESTION
- 対象をどこで分けるか
- 分けたそれぞれの部分で何を見るのか
- 分けない場合はどう扱うか

## CONDITION
- decided(target.subgroup)

分け方が決まり、分けなかった場合の扱いも決まっている。

## EXTRACT_FIELDS
- exemplar: 対象をどのように分けているか
- compare: 分け方の違う研究が、結果をどう比べているか

## PITFALL
- 分け方を決めないまま、集めた数字だけを出す
- 分ける基準が対象によって変わり、比較できない
- 分けたあと、分けなかった場合の結果を捨てる

## NEXT_ACTION
- untouched: sci-ideate | {title}の分け方を出す | 分け方の OPTION
- insufficient: sci-search | {title}の分け方を広げて OPTION を探す | 追加した分け方
- awaiting_decision: sci-compare | {title}の分け方を比べる | 決まった分け方
- conflict: sci-probe | {title}の分け方の違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の分け方が妥当か確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
