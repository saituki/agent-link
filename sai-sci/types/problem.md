---
id: problem
name: PROBLEM_DRIVEN
given: background
---
# PROBLEM_DRIVEN

現場で、今のやり方では形にならない問題が残っており、「これをどうにかしたい」という課題から研究が始まる型である。最初から測るものも手法も決まっていない。決まっているのは、誰がどの場面で何に困っているか、つまり大目的だけである。手法を先に考えず、課題を「観測できる問い」に落とすところまでが仕事になる。

最初から与えられていることが多いのは BACKGROUND（どの現場で、誰の不便さをなくしたいか）である。与えられていないのは、対象・データ・目的・既存手法・その手法が効かない理由・改善の方向・新規性の主張である。序盤は現場の課題を研究の問題として言い切ることに労力を使う。課題が 1 つに決まらないまま手法を並べ始めると、比較の基準が立たず、どこにも進めない。

典型例は、てんかん外科の支援という大目的から「頭皮脳波だけで発作部位を推定できるか」という問題に絞るものである。対象もデータも既存の臨床記録で足りると分かるので、残りは「既存の定位手法がこの条件では何に弱いのか」という説明を組み立てることに使う。

## ITEM_ORDER
- background
- target
- data
- purpose
- existing
- issue
- method
- novelty

## SEED_SENTENCE
{target}の{data.type}で{purpose.subgoal}を行う。既存の{existing}は{issue.limit}が弱い。{method.analog}の{method}で解く。新しさは{novelty.diff}。

## GATE
- BACKGROUND_GOAL_DECIDED: decided(background.goal)
- PURPOSE_SUBGOAL_DECIDED: decided(purpose.subgoal)
- DATA_ACCESS_DECIDED: decided(data.access)
- EXISTING_SATURATED: saturated(existing)
- ISSUE_LIMIT_LEVEL_2: level(issue.limit, 2)
- METHOD_DECIDED: decided(method)
- NOVELTY_CLOSEST_AND_DIFF: count(novelty.closest, 3) かつ decided(novelty.diff)
