---
key: purpose.output
name: 出力の型
type: choose
parent: purpose
depends_on: purpose.subgoal
min_findings: 2
---
# 出力の型

研究が何を形にして出すか決める項目である。

区間か確率か順位かで、必要な指標も変わる。

## 問い
- 何の形で結果を出すか
- その形で現場はどう使うか
- その形にすると評価しやすくなるか

## 埋まった条件
- decided(purpose.output)

出力の形が 1 つに決まっている。

## 抜き出す項目
- exemplar: その研究が出したものの形
- compare: 同じ小目的で別の形を出した研究

## 落とし穴
- 出力の形を決めずに指標を選ぶ
- 確率を出すのに区間だけの指標で評価する
- 現場が使えそうにない形を選ぶ

## 次の一手
- untouched: sci-ideate | {title}の候補を出す | 候補の一覧
- insufficient: sci-search | {title}の型を広げて候補を探す | 追加した候補
- awaiting_decision: sci-compare | {title}の型を 1 つに決める | 選んだ型と理由
- conflict: sci-probe | {title}の型の違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の型が小目的に合うか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
