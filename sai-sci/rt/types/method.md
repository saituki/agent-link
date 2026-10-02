---
id: method
name: METHOD_DRIVEN
root: この研究は、手元のどの手法を、何に使うか
---
# METHOD_DRIVEN

手元で新しい手法が使える状態が先にあり、「それを何に使うか」を決める型である。手法の中身は既に決まっている。

最初に与えられていることが多いのは手法（何ができる新しい技術か）である。与えられていないのは、使う場面、対象、比較の相手、VALUE_AXES、新規性の主張である。手法が劣っていても、VALUE_AXES が違えば使う理由ができる。

典型例は、自己教師ありの時系列分解を手元にもっている状態で、「てんかん焦点の短い区間の時系列を表示できるか」を考えるものである。手法は決まっているが、対象も比較の相手も決まっていない。

## Q
- background
  - background.goal
  - background.problem
- target
  - target.subgroup
- method
  - method.analog
  - method.adapt
- purpose
  - purpose.subgoal
    + purpose.output
    + purpose.metric
    + data
      - data.type
      - data.size
      - data.label
      - data.access
    + existing
    + novelty
      - novelty.closest
      - novelty.diff

## SEED
{method}を{target}の{purpose.subgoal}に使う。{existing}に劣っても、{mind:VALUE_AXES}で価値がある。
