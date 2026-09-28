---
id: data
name: データ駆動
given: data
---
# データ駆動

手元にあるデータセットが先にあり、「このデータで何が言えるか」を問う型である。集めたときの目的とは限らない。

最初から与えられていることが多いのはデータ（何が手元にあるか）である。与えられていないのは、小目的、既存手法、その手法の限界、改善の方向、新規性の主張である。序盤はデータの意味を読む時間に使われる。

典型例は、病院の生理信号が 1 つのデータベースにまとまっていて、その中のどの話をするかから始めるものである。データの型は決まっているが、小目的も比較対象も決まっていない。

## 項目の順序
- background
- data
- purpose
- target
- existing
- issue
- method
- novelty

## 種の文
手元の{data.type}で{purpose.subgoal}を行う。既存の{existing}は{issue.limit}が弱い。

## 関門
- データ: decided(data.access)
- 小目的: decided(purpose.subgoal)
- 既存手法の網羅: saturated(existing)
- 課題の確認: level(issue.limit, 2)
- 改善の方向: decided(method)
- 新規性: count(novelty.closest, 3) かつ decided(novelty.diff)
