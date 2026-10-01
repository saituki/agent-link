---
key: data.label
name: DATA_LABEL
type: choose
parent: data
depends_on: data.type
min_findings: 2
---
# DATA_LABEL

データにどう正解の印を付けるか決めるITEM である。

印の種類と付け方が決まると、比較の基準が決まる。

## QUESTION
- どんな種類の印を付けるか
- 誰がその印を付けるか
- 印が食い違うときどう扱うか

## CONDITION
- decided(data.label)

印の付け方が 1 つに決まっている。

## EXTRACT_FIELDS
- data_source: 印の種類・印の付け方・印の確かさ
- exemplar: 印が付かなかったものをどう扱ったか

## PITFALL
- 印の付け方を書かずに、研究ごとに違う基準で比べる
- 印が 2 つ以上あるのに、1 つにまとめて扱う
- 印が食い違うものを外すと、対象が変わったことになる

## NEXT_ACTION
- untouched: sci-ideate | {title}の付け方を OPTION で出す | 付け方の OPTION
- insufficient: sci-search | {title}の付け方を広げて探す | 追加した型
- awaiting_decision: sci-compare | {title}の付け方を比べる | 決まった付け方
- conflict: sci-probe | {title}の付け方の違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の付け方が妥当か確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
