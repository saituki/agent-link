---
key: data.access
name: DATA_ACCESS
type: choose, verify
parent: data
depends_on: data.type
min_findings: 2
---
# DATA_ACCESS

そのデータを手にできるかを確かめる ITEM である。

「公開されている」と「使える」は違う。条件と手続きを確認して初めて使えるものになる。

## QUESTION
- 公開されているデータはあるか
- 使うときの条件（申請・ライセンス・倫理審査）は何か
- 入手までどれくらい時間がかかるか

## CONDITION
- decided(data.access) かつ evidence(data.access, 1)

入手の行き先が決まって、条件と手続きが分かっている。

## EXTRACT_FIELDS
- data_source: 入手先・申請先・ライセンス・利用の制限
- exemplar: そのデータを実際にどう手に入れたか

## PITFALL
- 論文にデータ名があっても、今も入手できるとは限らない（リンク切れ・申請制）
- ライセンスの条件を見ずに使い始める
- 倫理審査に時間がかかることを考えず、計画に組み込む

## NEXT_ACTION
- untouched: sci-search | {title}の方法（公開先・申請・ライセンス）を外部から探す | 入手方法の一覧
- insufficient: sci-search | {title}の入手先を広げて探す | 追加した入手先
- awaiting_decision: sci-compare | {title}の入手条件を確かめてもらう | 決まった入手の見通し
- conflict: sci-probe | {title}の入手条件の食い違いを調べる | 食い違いの説明
- weak_evidence: sci-probe | {title}が今も入手できるか確かめる | 確かめた結果
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## CHILD_ITEM
