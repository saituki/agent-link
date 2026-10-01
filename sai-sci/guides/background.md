---
key: background
name: BACKGROUND
type: write
parent:
depends_on:
min_findings: 2
---
# BACKGROUND

大目的と、問題がなぜ外在的な事情として起きるのかをまとめるITEM である。ここで書いた大目的は、木のすべての ITEM の行き先になる。

行き先が決まる前にデータや手法の話を進めると、あとで引き直すことになる。

## QUESTION
- この研究で何をしようとしているのか
- なぜこの研究が必要なのか（誰が困っているか）
- 今はどのような状況なのか

## CONDITION
- not_empty(background) かつ decided(background.goal)

大目的を専門語なしの 1 文で書け、根拠が 1 つ以上ある。

## EXTRACT_FIELDS
- background: 問題の背景・誰が困っているか・現状はどうなっているか
- issue_evidence: 課題を裏づける事実・数値・記録

## PITFALL
- 手段から書き出す（「〇〇の技術」で始めると背景ではなく手法になる）
- 大目的が大きすぎて、1 回の研究で何をどこまでやるかが決まらない
- 問題があると主張するだけで、根拠が 1 つもない

## NEXT_ACTION
- untouched: sci-draft | {title}の初稿を書く | 大目的と問題の状況の初稿
- insufficient: sci-lookup | {title}を具体化するための資料を Library で探す | 見つかった記述の一覧
- awaiting_decision: - | {title}の書き方を人が決めてもらう | 決まった本文
- conflict: sci-debate | {title}で食い違う点を議論する | 合意した書き方
- weak_evidence: sci-probe | {title}の根拠を外部で確かめる | 根拠の一覧
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
- background.goal
- background.problem
