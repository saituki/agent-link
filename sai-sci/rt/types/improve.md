---
id: improve
name: IMPROVE_DRIVEN
root: この研究は、どの既存手法の、何を良くするか
---
# IMPROVE_DRIVEN

すでに動いている手法があり、「これをよくする」ことを目標にする型である。何を直すかは、おおむね決まっている。

最初から与えられていることが多いのは既存手法（今、何が動いているか）である。与えられていないのは、その手法の弱点、根本の原因、改善の型、確かめ方のデータ、新規性の主張である。

典型例は、自動注釈が特定の記録条件で壊れるので、壊れる条件を整理して直す工夫を考えるものである。手法は既にあるので、どこに手を入れるかを決めるところが本番になる。

## Q
- background
  - background.goal
  - background.problem
- target
  - target.subgroup
- existing
- issue
  - issue.limit
    + issue.root
    + method
      - method.analog
      - method.adapt
    + data
      - data.type
      - data.size
      - data.label
      - data.access
    + novelty
      - novelty.closest
      - novelty.diff

## SEED
{existing}の{issue.limit}を、{issue.root}に着目して{method}で直す。
