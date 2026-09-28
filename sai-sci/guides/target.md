---
key: target
name: 対象
type: choose
parent:
depends_on: background
min_findings: 2
---
# 対象

研究の対象を決める項目である。何を、どの範囲の事例として扱うかを 1 つに決める。

対象が定まらないと、データの型も出力の形も成功基準も決められない。

## 問い
- 何を取り扱うか（どの事例・どの範囲か）
- なぜその対象を選ぶのか
- ほかの候補を外した理由は何か

## 埋まった条件
- decided(target)

対象が 1 つに決まり、外したものに理由がある。

## 抜き出す項目
- data_source: 対象の選び方・件数・属性・範囲
- exemplar: その対象を扱った研究が、対象をどう絞ったか

## 落とし穴
- 対象が広すぎて、データの型も出力の形も決められない
- 有名な事例に寄せたあと、その理由を書かない
- 「すべての」を対象にして、比較の基準が立たない

## 次の一手
- untouched: sci-ideate | {title}の候補を出す | 対象候補の一覧
- insufficient: sci-search | {title}の範囲を広げて候補を探す | 追加した候補
- awaiting_decision: sci-compare | {title}の候補を 1 つに絞る | 選んだ対象と外した理由
- conflict: sci-probe | {title}の候補が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の候補を 1 つずつ確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
- target.subgroup
