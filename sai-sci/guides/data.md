---
key: data
name: DATA
type: choose, verify
parent:
depends_on: target
min_findings: 2
---
# DATA

使うデータを 1 つに決める ITEM である。子の 4 つ（DATA_TYPE・DATA_SIZE・DATA_LABEL・DATA_ACCESS）がすべて決まって決まる。

決まるのは OPTION の比較までで、入手できることは実際に確かめるまで確定しない。

## QUESTION
- どのデータを使うか
- そのデータで何ができるか
- そのデータを手にできるか

## CONDITION
- decided(data.type) かつ decided(data.size) かつ decided(data.label) かつ decided(data.access)

DATA_TYPE・DATA_SIZE・DATA_LABEL・DATA_ACCESS の 4 つが決まり、入手できることを確かめた。

## EXTRACT_FIELDS
- data_source: データ名・件数・クラス数・記録条件・ライセンス・入手方法
- exemplar: 使ったデータ・前処理・評価の分け方

## PITFALL
- 名前だけを書いて、件数と記録条件が分からない
- ラベルの作り方が違う研究と比べる
- OPTION の中で 1 つだけを決めて、比較の記録を残さない

## NEXT_ACTION
- untouched: sci-search | {title}の OPTION を探す | OPTION のデータと件数・ライセンスの一覧
- insufficient: sci-search | {title}の範囲を広げて OPTION を探す | 追加した OPTION
- awaiting_decision: sci-compare | {title}の OPTION を入手・件数・ラベルで比べる | 比較表と推奨
- conflict: sci-probe | {title}の OPTION の食い違いを調べる | 食い違いの説明
- weak_evidence: sci-probe | {title}の OPTION を入手できるか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
- data.type
- data.size
- data.label
- data.access
