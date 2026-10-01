---
key: target
name: TARGET
type: choose
parent:
depends_on: background
min_findings: 2
---
# TARGET

研究の対象を決める ITEM である。何を、どの範囲の事例として扱うかを 1 つに決める。

対象が定まらないと、データの型も出力の形も成功基準も決められない。

## QUESTION
- 何を取り扱うか（どの事例・どの範囲か）
- なぜその対象を選ぶのか
- ほかの OPTION を外した理由は何か

## CONDITION
- decided(target)

対象が 1 つに決まり、外したものに理由がある。

## EXTRACT_FIELDS
- data_source: 対象の選び方・件数・属性・範囲
- exemplar: その対象を扱った研究が、対象をどう絞ったか

## PITFALL
- 対象が広すぎて、データの型も出力の形も決められない
- 有名な事例に寄せたあと、その理由を書かない
- 「すべての」を対象にして、比較の基準が立たない

## NEXT_ACTION
- untouched: sci-ideate | {title}の OPTION を出す | 対象の OPTION の一覧
- insufficient: sci-search | {title}の範囲を広げて OPTION を探す | 追加した OPTION
- awaiting_decision: sci-compare | {title}の OPTION を 1 つに絞る | 選んだ対象と外した理由
- conflict: sci-probe | {title}の OPTION が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の OPTION を 1 つずつ確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
- target.subgroup
