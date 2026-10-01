---
key: method
name: METHOD
type: list, choose
parent:
depends_on: issue
min_findings: 2
---
# METHOD

課題を解く手筋を OPTION として並べ、その中から 1 つ選ぶ。並べるのは名前だけでない。何に適用し、何を入力として受け、何が出力になり、どの条件で効くか、まで書く。

OPTION を増やすのに使うのが、同型の問題の探し方である。問題を「入力の型 × 出力の型 × 制約（少数・不均衡・多施設など）」で言い直し、同じ形の問題を他分野で探す。例: 頭皮脳波での発作部位の予測 → 多チャネル時系列で異常の場所を推定する問題 → 設備センサの故障箇所の特定。この 3 つの例は分野が違うが、解こうとしている問題の形は同じである。

## QUESTION
- 課題の原因に真っ直ぐ効く手筋は何か。
- 課題の原因に直接効く手筋は何か。
- 自分の条件で、効果を示せると言えるものはどれか。

## CONDITION
- count(method.analog, 1) かつ decided(method)

同型の問題が 1 つ以上あり、取り入れる手法を 1 つ選び、移せる理由が書けているときに埋まる。OPTION が 3 つ並んで止まるのは、1 つも選んでいないからである。

## EXTRACT_FIELDS
- method_source: OPTION の手法名・使う場面・必要な入力・出力・効かない条件
- compare: OPTION 同士で結果が変わる条件

## PITFALL
- OPTION の名前と 1 行の要約だけで、適用条件を書いていない。
- 新しさだけで選ぶ。効く理由が説明できない OPTION は選ばない。
- 3 つ並べたまま、どれを取るかで止まる。

## NEXT_ACTION
- untouched: sci-ideate | {title}の解け方を増やす | 手法の OPTION 一覧
- insufficient: sci-analogy | 同じ形の問題を他分野で探す | 同型の問題の OPTION
- awaiting_decision: sci-compare | OPTION 同士を比べて 1 つ選ぶ | 選んだ手法と理由
- conflict: sci-debate | 別々の解法について議論する | 採用する手法の判断
- weak_evidence: sci-probe | OPTION が自分の条件で効くか確かめる | 確かめた結果
- recheck: - | 課題が変わったので手法を選び直す | 選び直した結果

## CHILD_ITEM
- method.analog
- method.adapt
