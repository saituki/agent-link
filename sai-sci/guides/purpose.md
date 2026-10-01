---
key: purpose
name: PURPOSE
type: choose
parent:
depends_on: background, target
min_findings: 2
---
# PURPOSE

この研究で何をしたいかを 1 つに決める ITEM である。

PURPOSE_SUBGOAL・PURPOSE_OUTPUT・PURPOSE_METRIC の 3 つがすべて決まって、決まる。

## QUESTION
- この研究で何を達成したいか
- 達成したかどうかをどの数字で表すか
- その数字まで誰が必要とするか

## CONDITION
- decided(purpose.subgoal) かつ decided(purpose.output) かつ decided(purpose.metric)

小目的が 1 つ、出力の型、数値の成功基準が決まった。

## EXTRACT_FIELDS
- exemplar: その研究が達成しようとしていたこと
- compare: 同じ目的を別のやり方で狙った研究

## PITFALL
- 大目的をそのまま小目的にする（大きすぎて評価できない）
- 出力の形と指標が噛み合わない
- 数字の基準を書かずに「良い結果」とだけ書く

## NEXT_ACTION
- untouched: sci-ideate | {title}の OPTION を出す | OPTION の一覧
- insufficient: sci-search | {title}の型を広げて OPTION を探す | 追加した OPTION
- awaiting_decision: sci-compare | {title}の OPTION を比べる | 比較表と推奨
- conflict: sci-probe | {title}の OPTION が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の OPTION を確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
- purpose.subgoal
- purpose.output
- purpose.metric
