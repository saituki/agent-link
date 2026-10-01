---
key: method.adapt
name: METHOD_ADAPT
type: write
parent: method
depends_on: method.analog
min_findings: 2
---
# METHOD_ADAPT

同型の問題で使われている手法を、自分の問題にどう入れて壊すかを書く。取り入れる要素と、入れない要素を分けて書く。

## QUESTION
- 同型の問題で使われていた手法から、どの要素をそのまま持ってくるか。
- 自分の条件で変えて使う要素はどれか。変えると何が変わるか。
- 入れない要素はどれか。外すと何が変わるか。

## CONDITION
- decided(method.adapt)

持ち込む要素、変えて使う要素、外す要素の 3 つが揃い、変えて使う要素に理由があるときに埋まる。

## EXTRACT_FIELDS
- method_source: 同型の問題で使われていた手法の構成要素
- issue_evidence: 自分の条件で構成要素が成立しない根拠

## PITFALL
- 入れる要素と入れない要素を、1 つの段落にまとめて書いている。
- 理由を「厳密だから」「新しいから」と書くと、対象に即した理由になっていない。
- 外す要素の理由を書かずに落とす。あとで元の実装と同じ結果を説明できなくなる。

## NEXT_ACTION
- untouched: sci-draft | 取り入れ方を書き始める | 要素の一覧と理由
- insufficient: sci-lookup | 同型の問題での使われ方を読み直す | 使われ方の記述
- awaiting_decision: - | 変える要素と変えない要素を決める | 決めた要素の分け方
- conflict: sci-debate | 入れ方から別々の案を出し合う | 採用する取り入れ方
- weak_evidence: sci-probe | 変えた要素が効くか予備で確かめる | 確かめた結果
- recheck: - | 手法が変わったので取り入れ方を見直す | 見直した結果

## CHILD_ITEM
<!-- この ITEM に子は無い -->
