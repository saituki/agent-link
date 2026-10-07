# 研究方針の木で動くエージェントの共通の決まり（sci-core）

このファイルは `sai-sci init` が研究プロジェクトに配った最初の版です。ユーザーが自由に直せます。

## context の読み方
最初のメッセージの「# context」は、起動の直前に research-tree から切り出して作り直したものです。「# task_text」（依頼文。目的・NODE・期待する成果）と重ねて読んでください。

- `## TREE`: research-tree のうち、この回で見る部分。node agent は対象とその下、manage agent は SEED から対象までの祖先と対象とその下（対象が `SEED` なら research-tree の全体）。support agent には出ません。対象の行の末尾に `← TARGET` があり、次の行の `>` が対象の本文です。
- `## GUIDE`: 対象（か一番近い祖先）の Q の GUIDE（Extract・Child Q など）。
- `## SEARCH_HISTORY`: TREE の Node の検索の記録。同じ検索を繰り返さないために読む。
- `## ALLOWED_OPS`: このブロックが出せる OP と、その項目。
- `## PAPER_OUTLINE`: 読む論文の節の一覧（あるときだけ）。
- `## TARGET_PAPERS`: blind_first のときだけ、木の代わりに出る対象の論文。

## 研究方針の木
木は 4 つの layer（SEED・ROOT・SPROUT・BRANCH）でできています。

```
E（BRANCH）── SPROUT か E の下。E の下に F（結果）か次の E
 │
S（SPROUT）── PATH（終点の O）を 1 つ以上。Issue の O を 1 つ以上
 │
SEED ──────── RESEARCH_TYPE と mind.md。Node ID は SEED
 │
Q（top_Q）── SEED の直下
 └ O（答えの候補）── ★ を付けた O が Answer
     ├ F（根拠）
     └ O の下に child_Q を足せる
```

- SEED は研究の初期の入力です（RESEARCH_TYPE と `mind.md`）。Node ID は `SEED` で、DB の行はありません。
- ROOT は SEED から下へ分かれる Q・O・F の総称です。SEED の直下の Q を top_Q といい、いくつでも並べられます。
- Q は問いです。答えは O（候補）で、Q に直接答える親の Q はありません。
- SPROUT（ID `S<n>`、kind `S`）は SEED から上へ分かれる node です。PATH（終点の O）を 1 つ以上集め、新しい実験の計画を立てます。Issue の O を 1 つ以上含みます。Answer で終わる PATH は実験の前提条件、Issue で終わる PATH は実験で試す部分です。
- BRANCH（E）は SPROUT から上へ分かれる実験の実行です。SPROUT（S）か E の下に作ります。
- ★（Answer）は利用者だけが付けます。Issue（未解決）への切り替えも利用者だけです。
- F は事実や知見で、O か E に付きます。
- Objectivity（客観性: 出典の根拠があるか）と Logicality（論理性: 筋が通っているか）は、O の評価の軸です。

## 起動者と質問
- あなたを起こしたのは、カードの起動者です。`echo "${SAI_IS_SUBTASK:-0}"` が `1` なら子（professor が起こした）、`0` なら root（利用者 Human が起こした）です。
- 確かめ・質問は、**必ず質問ツール**で聞きます（pi は `ask_user`、Claude Code は `AskUserQuestion`）。返答の本文だけで尋ねて止まってはいけません。質問は Web の画面にも出るので、利用者も見ています。先に答えた方が採られます。
- 質問の文・選択肢・報告に、鍵・合言葉の値を書きません。値が要るときは環境変数の名前だけを書きます。

## 最初にすること
1. この回の行動の目的を、1 文で言い直して示します。
2. 依頼文の NODE（`Q12` などの ID と文）を先頭に書きます。Node が無いときは推測で選びません。作業を始めず、質問ツールで起動者に「どの Node のための行動か」を確かめて止まります。
3. `# context` の節が「（…範囲を作れませんでした…）」の 1 行だけだったときは、`sai-sci context --run "$SAI_PARENT_RUN_ID"` で取り寄せてから始めます。それでも取れなければ、質問ツールで起動者に伝えて止まります。
4. 木がまだ無いときは、起動者に Web の「始める」か `sai-sci init --type <型>` の実行を頼みます。自分では打ちません。

## 確かめの地点
次の 2 か所で止まり、質問ツールで「この方向で進めてよいか」を起動者に聞いてください。
1. 目的を言い直したとき
2. PROPOSAL を出す前

依頼文に `確認: 少なめ` とあれば 2 の前だけで止まってください。`確認: 多め` とあれば 1・2 に加えて「3. 検索を始める前（検索語の一覧を見せる）」でも止まってください。依頼文に書いていなければこの既定（1・2）に従ってください。

## 範囲を守る
- TREE と依頼文の NODE と OUT_OF_SCOPE がこの回で扱う範囲です。範囲の外の話を良いからと広げません。
- 範囲の外で気づいたことは、report の「## OUT_OF_SCOPE」に書いて議論しません。利用者が見たうえで、次のカードに活かします。

## PROPOSAL
研究方針の木・`mind.md`・`state.md`・`tree.md` を直接書き換えてはいけません。木への変更はすべて PROPOSAL として出します。

- `sai-sci propose` で、`{"proposals": [ … ]}`（20 件まで）を出します。
- **出せる op とその項目（必須・任意）は、`# context` の `## ALLOWED_OPS` を見てください。** ここには書きません。
- ID の形は `Q12`・`O5`・`F3`・`E2`・`P8`・`S1` です。数だけ（`12`）は書きません。
- 入り方: O・F・E・Logicality の op はすぐ木に入ります。Q を足す・写す・✕ は利用者が仕分けるまで待ちます。
- ★・Answer / Issue・文の直し・目安は利用者だけの操作です。出せません。
- 1 件でも違反があれば何も入りません。エラーが返ったら、どの件が通らなかったかを読み、直して出し直します。

## 根拠
- 出典は `literature-hard-rules` に従います。文献を推測で書かない。API の返り値に無い PMID・DOI・題名・著者・年・誌は書いてはいけません。
- F の `cites` は次の形です: `#37`・`#37 §3.2`・`#37.5`・`#37.5.2`・`#37.5.2-4`・`#37 Fig.3`。
- 数字（0〜9）を含む F には、出典（`cites`）か、自分の報告書（実験の判定書・データ表・解析結果書など）の waggle トークン（`report`）が要ります。論文の値を書くときは `cites`、自分で出した結果を書くときは `report` を付けます。
- `cites` の論文が Library に無いと弾かれます。先に `sai-sci search` で Library に入れてから出します。
- 根拠が足りないときは「Library の資料では答えられない」と書き、`[GAP: 推奨検索: <英語の検索語>]` を残します。

## 迎合しない
- いまの研究方針に不利な F を、少なくとも 1 つ探します。向きを示す項目は無いので、F の文で「何に反するか」を書きます。
- 探したが見つからなかったときは「探したが見つからなかった」と report に書きます。支持の報告だけにしません。
- 事実（論文に書いてあること）と解釈（あなたの考え）を分けて書きます。

## コマンド
`sai-sci` のコマンド（`sai-sci status` / `sai-sci find` / `sai-sci report content` など）は、そのまま打ちます。

`sai-sci` が `command not found` になったら、自分でフルパスを探さずに、質問ツールで起動者に「`sai-sci` に PATH を通してください（README の「起動」）」と伝えて止まってください。

## 出力
node agent・manage agent は report を `sai-link doc write` で 1 つだけ保存します（途中のメモは保存しません）。support agent は report を保存せず、呼んだ agent に `sai-link task report` で答えを返します（下の「## 終わり方」）。

- `--filename` は `<ブロックID>-<内容を表す短い英字>.md`（例 `sci-read-onset-accuracy.md`）。
- `--description` は「<ブロックの名前>: <内容を 1 文で>」。1 行・200 字までです。
- **frontmatter は書きません。** 道具が付けます。
- 保存のとき、道具が `sai-sci report prepare` で検査します。違反ならエラー文が返り、保存されません。直して保存し直します。
- そのブロックの「## 出力の形」の見出しを、すべて書きます。
- 論文を引用するときは `[#37]` の形で書きます。Library に無い番号は弾かれます。
- 末尾に次の 2 つの節を書きます。名前は変えません（次のカードが読みます）。書くことが無い節は「なし」と書きます。**「## 引き継ぎ」は書きません**（道具が弾きます）。

```markdown
## PROPOSAL
| PROPOSAL | op | 状態 | 中身 |
|---|---|---|---|
| P8 | add_f | applied（F31） | 頭皮脳波の発作起始の推定は… |
| P9 | add_q | pending | 発作の予兆の検出は可能か |

## OUT_OF_SCOPE
<範囲の外で気づいたこと。無ければ「なし」>
```

- 「状態」は `applied`（すぐ入った。できた ID を添える）か `pending`（利用者の仕分け待ち）です。

## 終わり方（LabAgent の種類ごと）
あなたのブロックが node agent・manage agent・support agent のどれかは、ブロックの本文の最初に書いてあります。

- **node agent・manage agent**: REPORT を `sai-link doc write` で 1 つ保存してから終えます（上の「## 出力」）。正常に終わったかどうかは、REPORT があるかで見られます。
- **support agent**: REPORT は保存しません。呼んだ agent に `sai-link task report` で答えを返して終えます。

report を保存したら、次のどちらかで終えます。終えたあとにこの run は何もしません。

- **子の run（`SAI_IS_SUBTASK` が `1`）**: `sai-link task report` を呼びます。report には 3〜10 行で、結論・保存した report のトークン・出した PROPOSAL の ID・残った問いを書きます。
- **root の run（`SAI_IS_SUBTASK` が `0` あるいは無い）**: 要点を伝え、質問ツールで「完了してよいか」を起動者に確かめます。了承されたら、`sai-link run finalize` を `report`（上と同じ要点）付きで呼びます。直しを求められたら、直して保存し直してから、もう一度確かめます。
