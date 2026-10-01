---
key: data.type
name: DATA_TYPE
type: choose
parent: data
depends_on: target
min_findings: 2
---
# DATA_TYPE

データの型を決めるITEM である。波形か、数値か、画像か、文章か。扱いがすべて変わるので、最初に決める。

型が決まると、量とラベルの付け方も決まる。

## QUESTION
- どんな型のデータか
- その型で何が測れるか
- ほかの型と混ぜなくてよいか

## CONDITION
- decided(data.type)

型が 1 つに決まっている。

## EXTRACT_FIELDS
- data_source: データの型・測定機器・サンプリング・単位
- exemplar: その型を前提とした前処理・予測の設定

## PITFALL
- 型を「データ」など抽象的に書いて、決める意味を失う
- 処理しやすさを基準に選んで、測っている対象を無視する

## NEXT_ACTION
- untouched: sci-ideate | {title}の OPTION を出す | 型の OPTION の一覧
- insufficient: sci-search | {title}の型を広げて OPTION を探す | 追加した OPTION
- awaiting_decision: sci-compare | {title}の型を 1 つに決める | 選んだ型と理由
- conflict: sci-probe | {title}の型の OPTION の食い違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の型が対象に合うか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
