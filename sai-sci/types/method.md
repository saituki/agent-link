---
id: method
name: METHOD_DRIVEN
given: method
---
# METHOD_DRIVEN

手元で新しい手法が使える状態が先にあり、「それを何に使うか」を決める型である。手法の中身は既に決まっている。

最初に与えられていることが多いのは手法（何ができる新しい技術か）である。与えられていないのは、使う場面、対象、比較の相手、VALUE_AXES、新規性の主張である。手法が劣っていても、VALUE_AXES が違えば使う理由ができる。

典型例は、自己教師ありの時系列分解を手元にもっている状態で、「てんかん焦点の短い区間の時系列を表示できるか」を考えるものである。手法は決まっているが、対象も比較の相手も決まっていない。

## ITEM_ORDER
- background
- method
- purpose
- target
- data
- existing
- novelty

## SEED_SENTENCE
{method}を{target}の{purpose.subgoal}に使う。{existing}に劣っても、{mind:VALUE_AXES}で価値がある。

## GATE
- METHOD_DECIDED: decided(method)
- PURPOSE_SUBGOAL_DECIDED: decided(purpose.subgoal)
- TARGET_DECIDED: decided(target)
- DATA_ACCESS_DECIDED: decided(data.access)
- EXISTING_COUNT_1: count(existing, 1)
- MIND_VALUE_AXES: mind(VALUE_AXES)
- NOVELTY_DIFF_DECIDED: decided(novelty.diff)
