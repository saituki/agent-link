---
key: issue.root
name: ISSUE_ROOT
type: write, verify
parent: issue
depends_on: issue.limit
min_findings: 2
---
# ISSUE_ROOT

既存手法が暗黙に依拠している条件のなかで、どれが原因になっているかを書く。表面的に現れた症状ではなく、手法の背後にある前提を書き出す。

## QUESTION
- 既存手法が依拠している条件は、どれか。
- その条件を崩したら、どの部分が壊れるか。
- 崩したあとで、既存手法として使える部分は何か。

## CONDITION
- decided(issue.root) かつ evidence(issue.root, 1)

支持の FINDING が 1 つも無いまま文章だけを書いても、埋まったとは見なさない。条件を崩したときに何が起きるかを、数えるか図で示す。

## EXTRACT_FIELDS
- issue_evidence: 原因を確かめた経路・支持の根拠・条件を崩したときの変化
- method_source: 同じ原因を持つ他の分野での解法

## PITFALL
- 症状を原因として書く。「精度が出ない」のではなく「その手法の仮定が成り立たない」と書く。
- 条件を 1 つ崩したら手法全体が壊れると思い込む。残って使える部分も書いておく。
- 自分の推測だけで書き始める。先に予備実験か比較で確かめる。

## NEXT_ACTION
- untouched: sci-probe | {title}の原因を確かめる | 原因を確かめた経路
- insufficient: sci-lookup | 同じ原因を持つ研究を Library で探す | 原因を支持する記述
- awaiting_decision: - | 原因を 1 つに絞って書く | 絞った原因
- conflict: sci-debate | 原因について別々の説明を突き合わせる | どちらの説明が妥当かの判断
- weak_evidence: sci-probe | 原因を確かめる予備実験を組む | 予備実験の設計
- recheck: - | 課題が変わったので原因を見直す | 見直した結果

## CHILD_ITEM
<!-- この ITEM に子は無い -->
