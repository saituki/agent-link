---
key: data.label
name: ラベル
type: choose
parent: data
depends_on: data.type
min_findings: 2
---
# ラベル

データにどう正解の印を付けるか決める項目である。

印の種類と付け方が決まると、比較の基準が決まる。

## 問い
- どんな種類の印を付けるか
- 誰がその印を付けるか
- 印が食い違うときどう扱うか

## 埋まった条件
- decided(data.label)

印の付け方が 1 つに決まっている。

## 抜き出す項目
- data_source: 印の種類・印の付け方・印の確かさ
- exemplar: 印が付かなかったものをどう扱ったか

## 落とし穴
- 印の付け方を書かずに、研究ごとに違う基準で比べる
- 印が 2 つ以上あるのに、1 つにまとめて扱う
- 印が食い違うものを外すと、対象が変わったことになる

## 次の一手
- untouched: sci-ideate | {title}の付け方を候補で出す | 付け方の候補
- insufficient: sci-search | {title}の付け方を広げて探す | 追加した型
- awaiting_decision: sci-compare | {title}の付け方を比べる | 決まった付け方
- conflict: sci-probe | {title}の付け方の違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の付け方が妥当か確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
