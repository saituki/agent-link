---
key: purpose.metric
name: PURPOSE_METRIC
type: write
parent: purpose
depends_on: purpose.output
min_findings: 2
---
# PURPOSE_METRIC

どの数字で見たら成功と言えるか決めるITEM である。

名前だけでなく「ここまで出たら成功」と数字で書く。

## QUESTION
- どの数字で判断するか
- 既存手法はその数字でいくつ出ているか
- そこを出せなければ研究として成り立つか

## CONDITION
- decided(purpose.metric)

指標名と成功基準の数字が決まっている。

## EXTRACT_FIELDS
- exemplar: その研究が指標と基準をどう書いたか
- compare: 同じ問題の評価がどう違うか

## PITFALL
- 指標名だけ書いて、合格の数字を書かない
- 既存手法の数字を知らずに基準を決める
- 学習したデータでもって基準にすること

## NEXT_ACTION
- untouched: sci-draft | {title}の初稿を書く | 初稿
- insufficient: sci-lookup | {title}の型を Library で探す | 見つけた指標の型
- awaiting_decision: - | {title}の基準の高さを人が決めてもらう | 決まった成功基準
- conflict: sci-debate | {title}の指標の違いを議論する | 合意した指標
- weak_evidence: sci-probe | {title}の基準が妥当か確かめる | 確かめた根拠
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
