---
key: data.type
name: データの型
type: choose
parent: data
depends_on: target
min_findings: 2
---
# データの型

データの型を決める項目である。波形か、数値か、画像か、文章か。扱いがすべて変わるので、最初に決める。

型が決まると、量とラベルの付け方も決まる。

## 問い
- どんな型のデータか
- その型で何が測れるか
- ほかの型と混ぜなくてよいか

## 埋まった条件
- decided(data.type)

型が 1 つに決まっている。

## 抜き出す項目
- data_source: データの型・測定機器・サンプリング・単位
- exemplar: その型を前提とした前処理・予測の設定

## 落とし穴
- 型を「データ」など抽象的に書いて、決める意味を失う
- 処理しやすさを基準に選んで、測っている対象を無視する

## 次の一手
- untouched: sci-ideate | {title}の候補を出す | 型の候補の一覧
- insufficient: sci-search | {title}の型を広げて候補を探す | 追加した候補
- awaiting_decision: sci-compare | {title}の型を 1 つに決める | 選んだ型と理由
- conflict: sci-probe | {title}の型の候補の食い違いを調べる | 違いの説明
- weak_evidence: sci-probe | {title}の型が対象に合うか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
