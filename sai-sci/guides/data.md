---
key: data
name: データ
type: choose, verify
parent:
depends_on: target
min_findings: 2
---
# データ

使うデータを 1 つに決める項目である。子の 4 つ（型・量・ラベル・入手）がすべて決まって決まる。

決まるのは候補の比較までで、入手できることは実際に確かめるまで確定しない。

## 問い
- どのデータを使うか
- そのデータで何ができるか
- そのデータを手にできるか

## 埋まった条件
- decided(data.type) かつ decided(data.size) かつ decided(data.label) かつ decided(data.access)

型・量・ラベル・入手の 4 つが決まり、入手できることを確かめた。

## 抜き出す項目
- data_source: データ名・件数・クラス数・記録条件・ライセンス・入手方法
- exemplar: 使ったデータ・前処理・評価の分け方

## 落とし穴
- 名前だけを書いて、件数と記録条件が分からない
- ラベルの作り方が違う研究と比べる
- 候補の中で 1 つだけを決めて、比較の記録を残さない

## 次の一手
- untouched: sci-search | {title}の候補を探す | 候補のデータと件数・ライセンスの一覧
- insufficient: sci-search | {title}の範囲を広げて候補を探す | 追加した候補
- awaiting_decision: sci-compare | {title}の候補を入手・件数・ラベルで比べる | 比較表と推奨
- conflict: sci-probe | {title}の候補の食い違いを調べる | 食い違いの説明
- weak_evidence: sci-probe | {title}の候補を入手できるか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
- data.type
- data.size
- data.label
- data.access
