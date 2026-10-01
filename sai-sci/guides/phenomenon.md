---
key: phenomenon
name: PHENOMENON
type: write, verify
parent:
depends_on: background
min_findings: 2
---
# PHENOMENON

必ず起きていることを書き、「なぜそうなるのか」を問う ITEM である。PHENOMENON_DRIVEN で使う。

観察した事実と、思った説明を分けて書く。

## QUESTION
- 必ず起きていることは何か
- 誰が何の計測でそれを見たか
- 起きないのはどんなときか

## CONDITION
- not_empty(phenomenon) かつ evidence(phenomenon, 2)

現象が書けて、それを裏づける根拠が 2 つ以上ある。

## EXTRACT_FIELDS
- background: 現象そのもの・誰が見たか・どの条件で起きたか
- counter: 同じ現象が起きないという報告
- exemplar: その現象を調べた研究がどう確かめたか

## PITFALL
- 説明を観察として書く
- 誰が見たかも条件も書かない
- 1 つの例だけを全体像にする

## NEXT_ACTION
- untouched: sci-search | {title}の観察を外部から探す | 観察の記述の一覧
- insufficient: sci-lookup | {title}の観察を Library で探す | 見つかった観察
- awaiting_decision: sci-probe | {title}の説明の案を比べる | 比較表と推奨
- conflict: sci-probe | {title}の説明が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}が本当に起きるか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
