---
key: purpose
name: 目的
type: choose
parent:
depends_on: background, target
min_findings: 2
---
# 目的

この研究で何をしたいかを 1 つに決める項目である。

小目的・出力の型・評価指標の 3 つがすべて決まって、決まる。

## 問い
- この研究で何を達成したいか
- 達成したかどうかをどの数字で表すか
- その数字まで誰が必要とするか

## 埋まった条件
- decided(purpose.subgoal) かつ decided(purpose.output) かつ decided(purpose.metric)

小目的が 1 つ、出力の型、数値の成功基準が決まった。

## 抜き出す項目
- exemplar: その研究が達成しようとしていたこと
- compare: 同じ目的を別のやり方で狙った研究

## 落とし穴
- 大目的をそのまま小目的にする（大きすぎて評価できない）
- 出力の形と指標が噛み合わない
- 数字の基準を書かずに「良い結果」とだけ書く

## 次の一手
- untouched: sci-ideate | {title}の候補を出す | 候補の一覧
- insufficient: sci-search | {title}の型を広げて候補を探す | 追加した候補
- awaiting_decision: sci-compare | {title}の候補を比べる | 比較表と推奨
- conflict: sci-probe | {title}の候補が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の候補を確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
- purpose.subgoal
- purpose.output
- purpose.metric
