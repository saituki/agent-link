---
key: purpose.subgoal
name: PURPOSE_SUBGOAL
type: choose
parent: purpose
depends_on: background.goal, target
min_findings: 2
---
# PURPOSE_SUBGOAL

1 回の研究で到達したい到達点を 1 つに決める ITEM である。

大きすぎると点数にならない。そのため、大目的を少し下げる。

## QUESTION
- 1 回の研究で何を達成するか
- その到達点を 1 文で言えるか
- ほかの小目的とどこが違うか

## CONDITION
- decided(purpose.subgoal)

小目的が 1 つに決まっている。

## EXTRACT_FIELDS
- exemplar: その研究が 1 回の研究で何を達成したか
- compare: 小目的が近い研究との違い

## PITFALL
- 大目的をそのまま小目的にする（大きすぎて評価できない）
- 複数のことを 1 つの小目的にまとめる
- 対象もデータも未確定のまま小目的を決める

## NEXT_ACTION
- untouched: sci-ideate | {title}の OPTION を出す | OPTION の一覧
- insufficient: sci-search | {title}の OPTION を広げて探す | 追加した OPTION
- awaiting_decision: sci-compare | {title}の OPTION を 1 つに絞る | 選んだ小目的と理由
- conflict: sci-probe | {title}の OPTION が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の OPTION を評価できる形で確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
