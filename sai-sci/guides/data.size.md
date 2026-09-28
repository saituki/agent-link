---
key: data.size
name: データの量
type: write, verify
parent: data
depends_on: data.type
min_findings: 2
---
# データの量

データがいくつあるかを書き、足りるかを確かめる項目である。

数字だけを書いて終わりにしない。1 件がどれだけ重いかまで書くと、足りるかが判断できる。

## 問い
- データはいくつあるか
- クラスごとにいくつあるか
- その量で調べたいことを分けられるか

## 埋まった条件
- decided(data.size)

件数が書けて、足りるかも判断できている。

## 抜き出す項目
- data_source: 件数・クラスごとの件数・1 件の記録時間
- compare: 件数と結果の関係をどう書いているか

## 落とし穴
- 総数だけ書いて、クラスごとの数を書かない
- 項目数が多い項目と少ない項目を足して、1 件と数える
- 学習と検証で分けられるかを確かめずに件数だけ書く

## 次の一手
- untouched: sci-search | {title}の目安を外部から探す | 目安になった論文の一覧
- insufficient: sci-lookup | {title}の目安を Library で探す | 見つかった目安
- awaiting_decision: sci-probe | {title}が足りるかを判断してもらう | 決まった目標件数
- conflict: sci-probe | {title}の目安が食い違う理由を調べる | 理由の説明
- weak_evidence: sci-probe | {title}の件数を実際に確かめる | 確かめた件数
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
