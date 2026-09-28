---
id: improve
name: 改良駆動
given: existing
---
# 改良駆動

すでに動いている手法があり、「これをよくする」ことを目標にする型である。何を直すかは、既にある一定程度で決まっている。

最初から与えられていることが多いのは既存手法（今、何が動いているか）である。与えられていないのは、その手法の弱点、根本の原因、改善の型、確かめ方のデータ、新規性の主張である。

典型例は、自動注釈が特定の記録条件で壊れるので、壊れる条件を整理して直す工夫を考えるものである。手法は既にあるので、どこに手を入れるかを決めるところが本番になる。

## 項目の順序
- background
- existing
- issue
- method
- data
- target
- novelty

## 種の文
{existing}の{issue.limit}を、{issue.root}に着目して{method}で直す。

## 関門
- 対象の手法: count(existing, 1)
- 課題の確認: level(issue.limit, 2)
- 根本の原因: decided(issue.root)
- 改善の方向: decided(method)
- データの入手: decided(data.access)
- 新規性: count(novelty.closest, 3) かつ decided(novelty.diff)
