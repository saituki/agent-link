---
id: method
name: 手法駆動
given: method
---
# 手法駆動

手元で新しい手法が使える状態が先にあり、「それを何に使うか」を決める型である。手法の中身は既に決まっている。

最初に与えられていることが多いのは手法（何ができる新しい技術か）である。与えられていないのは、使う場面、対象、比較の相手、価値の軸、新規性の主張である。手法が劣っていても、価値の軸が違えば使う理由ができる。

典型例は、自己教師ありの時系列分解を手元にもっている状態で、「てんかん焦点の短い区間の時系列を表示できるか」を考えるものである。手法は決まっているが、対象も比較の相手も決まっていない。

## 項目の順序
- background
- method
- purpose
- target
- data
- existing
- novelty

## 種の文
{method}を{target}の{purpose.subgoal}に使う。{existing}に劣っても、{mind:価値の軸}で価値がある。

## 関門
- 手法: decided(method)
- 使う場面: decided(purpose.subgoal)
- 対象: decided(target)
- データの入手: decided(data.access)
- 比較の相手: count(existing, 1)
- 価値の軸: mind(価値の軸)
- 新規性: decided(novelty.diff)
