---
id: improve
name: IMPROVE_DRIVEN
given: existing
---
# IMPROVE_DRIVEN

すでに動いている手法があり、「これをよくする」ことを目標にする型である。何を直すかは、おおむね決まっている。

最初から与えられていることが多いのは既存手法（今、何が動いているか）である。与えられていないのは、その手法の弱点、根本の原因、改善の型、確かめ方のデータ、新規性の主張である。

典型例は、自動注釈が特定の記録条件で壊れるので、壊れる条件を整理して直す工夫を考えるものである。手法は既にあるので、どこに手を入れるかを決めるところが本番になる。

## ITEM_ORDER
- background
- existing
- issue
- method
- data
- target
- novelty

## SEED_SENTENCE
{existing}の{issue.limit}を、{issue.root}に着目して{method}で直す。

## GATE
- EXISTING_COUNT_1: count(existing, 1)
- ISSUE_LIMIT_LEVEL_2: level(issue.limit, 2)
- ISSUE_ROOT_DECIDED: decided(issue.root)
- METHOD_DECIDED: decided(method)
- DATA_ACCESS_DECIDED: decided(data.access)
- NOVELTY_CLOSEST_AND_DIFF: count(novelty.closest, 3) かつ decided(novelty.diff)
