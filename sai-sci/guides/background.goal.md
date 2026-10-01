---
key: background.goal
name: BACKGROUND_GOAL
type: write
parent: background
depends_on:
min_findings: 2
---
# BACKGROUND_GOAL

この研究が目指している大きな目的を 1 文で書くITEM である。この 1 文が、ほかのすべての ITEM の行き先になる。

Heilmeier の問いに 1 つずつ答える機会でもある。次の 4 問に、1 行ずつ答える。

- 何をしようとしているか
- 今はどうなっているか
- 何が新しいか
- 誰が気にするか

## QUESTION
- 何をすればよいと分かると、人は嬉しくなるのか
- その嬉しさを 1 文にするとどうなるか
- 専門語を使わずに説明できるか

## CONDITION
- decided(background.goal)

専門語を使わずに 1 文で書けている。

## EXTRACT_FIELDS
- background: 目的そのもの・誰の願いか
- issue_evidence: 目的を必要とする状況・数値・記録

## PITFALL
- 手段（データや手法）を並べただけの文章になる
- 大きすぎて、1 回の研究で何をどこまでやるかが決まらない
- 専門語が多いと、読み手が自分の課題だと認められない

## NEXT_ACTION
- untouched: sci-draft | {title}を 1 文で書く | 大目的 1 文の初稿
- insufficient: sci-lookup | {title}にかなう実現姿を Library で探す | 案の成果の一覧
- awaiting_decision: - | {title}の範囲を人が決めてもらう | 決まった大目的
- conflict: sci-debate | {title}が広すぎるところを議論する | 合意した範囲
- weak_evidence: sci-probe | {title}が本当に求められているか外部で確かめる | 確かめた根拠
- recheck: - | {title}の変更がほかの ITEM に与える影響を見直す | 影響の一覧

## CHILD_ITEM
