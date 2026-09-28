---
id: phenomenon
name: 現象駆動
given: phenomenon
---
# 現象駆動

必ず起きている現象を先に捉え、「なぜそうなるのか」を問う型である。使う手法より先に、観察できる事実がある。説明の型と確かめ方が決まってから、はじめて研究になる。

最初から与えられていることが多いのは現象（何が起きることを誰も疑っていないか）である。与えられていないのは、観察した対象、原因を探すデータ、原因を確かめる方法、既存手法とその限界、そして新規性の主張である。

典型例は、安静時と運動後で指標の挙動が変わるという現象から、その切り替わりがどの仕組みで起きるのかを調べるものである。現象は用意済みだが、原因を測るデータと確かめ方はまだ決まっていない。

## 項目の順序
- background
- phenomenon
- target
- data
- method
- issue
- novelty

## 種の文
{target}の{phenomenon}の原因を、{data.type}と{method}で調べる。{issue.limit}は{method.adapt}で補う。

## 関門
- 出発点: decided(background.goal)
- 現象: decided(phenomenon)
- 対象: decided(target)
- データの入手: decided(data.access)
- 手法: decided(method)
- 手法の限界: level(issue.limit, 2)
- 新規性: count(novelty.closest, 3) かつ decided(novelty.diff)
