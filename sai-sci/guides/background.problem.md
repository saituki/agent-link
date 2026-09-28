---
key: background.problem
name: 問題の背景
type: write, verify
parent: background
depends_on: background.goal
min_findings: 2
---
# 問題の背景

問題があると主張すること自体に、根拠を付ける項目である。大目的が決まったあとで、この根拠が要る。

根拠がないと、対象も手法も新規性も全部仮置きになる。Heilmeier の問いに 1 つずつ答える。

- 何をしようとしているか
- 今はどうなっているか
- 何が新しいか
- 誰が気にするか

## 問い
- 誰が何に困っているか
- その困りは今どのくらい続いているか
- なぜ今まで解けなかったのか

## 埋まった条件
- evidence(background.problem, 1)

問題を裏づける根拠が 1 つ以上ある。

## 抜き出す項目
- issue_evidence: 課題を裏づける事実・数値・記録・制度や事情の変化
- background: 問題が生じる背景・誰が関わる問題か

## 落とし穴
- 根拠が個人の感想や主張の引用だけになる
- 数値だけを出し、出典を示さない
- 問題のある場面と、それによる不便を混ぜて書いてしまう

## 次の一手
- untouched: sci-search | {title}の根拠を外部から探す | 根拠になりそうな論文の一覧
- insufficient: sci-lookup | {title}の根拠を Library で探す | 見つかった根拠
- awaiting_decision: sci-probe | {title}の根拠が十分かどうか判断してもらう | 採用した根拠
- conflict: sci-probe | {title}の根拠と食い違う記述を調べる | 食い違いの説明
- weak_evidence: sci-probe | {title}の根拠をもう一度確かめる | 確かめ直した根拠
- recheck: - | 上流の決定が変わったので{title}を見直す | 見直した結果

## 子の項目
