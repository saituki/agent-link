---
id: data
name: DATA_DRIVEN
root: この研究は、手元のどのデータで、何を明らかにするか
---
# DATA_DRIVEN

手元にあるデータセットが先にあり、「このデータで何が言えるか」を問う型である。集めたときの目的とは限らない。

最初から与えられていることが多いのはデータ（何が手元にあるか）である。与えられていないのは、小目的、既存手法、その手法の限界、改善の方向、新規性の主張である。序盤はデータの意味を読む時間に使われる。

典型例は、病院の生理信号が 1 つのデータベースにまとまっていて、その中のどの話をするかから始めるものである。データの型は決まっているが、小目的も比較対象も決まっていない。

## Q
- background
  - background.goal
  - background.problem
- target
  - target.subgroup
- data
  - data.type
  - data.size
  - data.label
  - data.access
- purpose
  - purpose.subgoal
    + purpose.output
    + purpose.metric
    + existing
    + issue
      - issue.limit
      - issue.root
    + method
      - method.analog
      - method.adapt
    + novelty
      - novelty.closest
      - novelty.diff

## SEED
手元の{data.type}で{purpose.subgoal}を行う。既存の{existing}は{issue.limit}が弱い。
