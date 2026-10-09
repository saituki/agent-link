# research-tree で動くエージェントの共通の決まり（sci-core）

このファイルは `sai-sci init` が研究プロジェクトに配った最初の版です。ユーザーが自由に直せます。

## context の読み方
最初のメッセージの「# context」は、起動の直前に research-tree から切り出して作り直したものです。「# task_text」（依頼文。目的・NODE・期待する成果）と重ねて読んでください。

- `## TREE`: research-tree のうち、この回で見る部分。node agent は対象とその下、manage agent は SEED から対象までの祖先と対象とその下（対象が `SEED` なら research-tree の全体）。support agent には出ません。対象の行の末尾に `← TARGET` があり、次の行の `>` が対象の本文です。
- `## GUIDE`: 対象（か一番近い祖先）の Q の GUIDE（Extract・Child Q など）。
- `## SEARCH_HISTORY`: TREE の Node の検索の記録（find（Library）と search（外部）の両方）。同じ検索を繰り返さないために読む。
- `## ALLOWED_OPS`: このブロックが出せる OP と、その項目。
- `## PAPER_OUTLINE`: 読む論文の節の一覧（あるときだけ）。
- `## TARGET_PAPERS`: blind_first のときだけ、research-tree の代わりに出る対象の論文。

## research-tree
research-tree は 4 つの layer（SEED・ROOT・SPROUT・BRANCH）でできています。

```
E（BRANCH）── SPROUT か E の下。E の下に次の E（F は置かない）
 │
S（SPROUT）── PATH（終点の A）を 1 つ以上（条件なし）
 │
SEED ──────── RESEARCH_TYPE と mind.md。Node ID は SEED
 │
Q（top_Q）── SEED の直下
 └ A（答えの候補）── ★ は目印（意味を持たない）
     ├ F（根拠。stance が support・deny・issue のどれか）
     └ A の下に child_Q を足せる
```

- SEED は研究の初期の入力です（RESEARCH_TYPE と `mind.md`）。Node ID は `SEED` で、DB の行はありません。
- ROOT は SEED から下へ分かれる Q・A・F の総称です。SEED の直下の Q を top_Q といい、いくつでも並べられます。
- Q は問いです。答えは A（候補）で、Q に直接答える parent_Q はありません。
- SPROUT（ID `S<n>`、kind `S`）は SEED から上へ分かれる node です。PATH（終点の A）を 1 つ以上集め、新しい実験の計画を立てます。範囲は、各 path の A とその祖先（Q・A）と、その A の子の F です。A の子の Q 以下は含みません。
- BRANCH（E）は SPROUT から上へ分かれる実験の実行です。SPROUT（S）か E の下に作ります。E の下に F は置きません。実験の結論は REPORT と MLflow に書きます。
- ★ は目印で、どの種類の Node にも付けられます。道や進み具合を決めません。付けるのは利用者だけです。
- F は事実や知見で、A の下に付きます。stance（`support`・`deny`・`issue`）を必ず持ちます。
- A の本文に `#番号` の出典を書いたら、同じ出典で F（stance `support`）を 1 本足します。F の無い A は Next Action の NO_FINDING に出ます。

## 起動者と質問
- あなたを起こしたのは、CARD の起動者です。`echo "${SAI_IS_SUBTASK:-0}"` が `1` なら子（`task create` で起こされた。報告は親へ返す）、`0` なら root（Card で起こされた。起こしたのは Human か、Human の代わりの Professor）です。
- 確かめ・質問は、**必ず質問ツール**（`AskUserQuestion`）で聞きます。返答の本文だけで尋ねて止まってはいけません。質問は Web の画面にも出るので、利用者も見ています。先に答えた方が採られます。root の質問には、起こした Human か Professor が Web（`sai-web answer`）で答えます。
- 質問の文・選択肢・報告に、鍵・合言葉の値を書きません。値が要るときは環境変数の名前だけを書きます。

## 最初にすること
1. この回の行動の目的を、1 文で言い直して示します。
2. 依頼文の NODE（`Q12` などの ID と文）を先頭に書きます。Node が無いときは推測で選びません。作業を始めず、質問ツールで起動者に「どの Node のための行動か」を確かめて止まります。
3. `# context` の節が「（…範囲を作れませんでした…）」の 1 行だけだったときは、`sai-sci context --run "$SAI_PARENT_RUN_ID"` で取り寄せてから始めます。それでも取れなければ、質問ツールで起動者に伝えて止まります。
4. research-tree がまだ無いときは、起動者に Web の「始める」か `sai-sci init --type <型>` の実行を頼みます。自分では打ちません。

## 確かめの地点
次の 2 か所で止まり、質問ツールで「この方向で進めてよいか」を起動者に聞いてください。
1. 目的を言い直したとき
2. 木を変える op を出す前

依頼文に `確認: 少なめ` とあれば 2 の前だけで止まってください。`確認: 多め` とあれば 1・2 に加えて「3. 検索を始める前（検索語の一覧を見せる）」でも止まってください。依頼文に書いていなければこの既定（1・2）に従ってください。

## 範囲を守る
- TREE と依頼文の NODE と OUT_OF_SCOPE がこの回で扱う範囲です。範囲の外の話を良いからと広げません。
- 範囲の外で気づいたことは、report の「## OUT_OF_SCOPE」に書いて議論しません。利用者が見たうえで、次の CARD に活かします。

## research-tree を変える（op）
research-tree・`mind.md`・`state.md`・`tree.md` を直接書き換えてはいけません。木への変更はすべて `sai-sci op` で出します。

- 標準入力に `{"ops": [ {"op": "add_a", "q": "Q3", "text": "…"}, … ]}` の形で渡します（20 件まで）。
- **出せる op とその項目（必須・任意）は、`# context` の `## ALLOWED_OPS` を見てください。** ここには書きません。ALLOWED_OPS に無い op は出せません。
- 1 回の `op` は全部で 1 つのまとまりです。1 件でも弾かれたら、何も入りません。エラーの最後の行は「`<n> 件目（<op>）: <理由>`」なので、その件を直して出し直します。
- 段を重ねるとき（新しい Q の下に A を出すなど）は、段ごとに分けて出します。返った ID（`result`）を次の段で使います。
- ID の形は `Q12`・`A5`・`F3`・`E2`・`S1` です。数だけ（`12`）は書きません。
- 許可が要る op（`add_q`・`insert_q`・`move_a`・`copy`・`remove`）は、先に質問ツールで許可を取ってから出します（下の「許可を取る」）。
- `add_f` の親は A だけです。`stance` が要ります。E の下に F を置きません。
- ★・文の直し（`edit`）・星（`star`・`unstar`）は利用者だけの操作です。出せません。

## 許可を取る
許可が要る op を出すときは、質問ツールで次の決まりに従って聞きます。

- 1 回の質問に 1 問だけです。
- 先頭の選択肢を `許可する`、2 つ目を `許可しない` にします。
- 質問文に、これから行う op の一覧（op 名と対象の ID・文）を書きます。
- `許可する` が選ばれなかったら、その op は出しません。
- `sai-sci approval record` は自分で呼びません。質問の答えは、フックが記録します。
- 許可 1 件は、許可が要る op を含む `op` の 1 回で使われます。使い切ったら、次の許可を取り直します。

## 根拠
- 出典は `literature-hard-rules` に従います。文献を推測で書かない。API の返り値に無い PMID・DOI・題名・著者・年・誌は書いてはいけません。
- F の `cites` は次の形です: `#37`・`#37 §3.2`・`#37.5`・`#37.5.2`・`#37.5.2-4`・`#37 Fig.3`。
- 数字（0〜9）を含む F には、出典（`cites`）か、自分の REPORT（実験の判定書・データ表・解析結果書など）の waggle トークン（`report`）が要ります。論文の値を書くときは `cites`、自分で出した結果を書くときは `report` を付けます。
- **`cites` と報告の引用に書けるのは、この run で CLI（`sai-paper`・`sai-sci`・`sai-link doc read`）が出力した番号だけです。** 見ていない番号は、記憶や推測で書かず、先に `sai-paper` で読んでから引きます。見た番号は台帳（`.sai/seen_ids/<run_id>`。MLflow とは別の物）に残っていて、無い番号は `台帳に無い番号です: #37.5.2` で終了コード 2 になります（何も書かれません）。節・塊（`#37.5`・`#37.5.2`）は読んだとおりの番号を書きます。論文（`#37`）は、その論文のどれかの番号を見ていれば通ります。
- `cites` の論文が Library に無いと弾かれます。Library に無い論文は、`sai-sci search` で見つけた候補を利用者に取り込んでもらってから出します（search は Library に入れません）。
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

- `--filename` は `<ブロックID>-<内容を表す短い英字>.md`（例 `lab-read-onset-accuracy.md`）。
- `--description` は「<ブロックの名前>: <内容を 1 文で>」。1 行・200 字までです。
- **frontmatter は書きません。** 道具が付けます。
- 保存のとき、道具が `sai-sci report prepare` で検査します。違反ならエラー文が返り、保存されません。直して保存し直します。
- そのブロックの「## 出力の形」の見出しを、すべて書きます。
- 論文を引用するときは `[#37]` の形で書きます。Library に無い番号は弾かれます。
- 末尾に次の 2 つの節を書きます。名前は変えません（次の CARD が読みます）。書くことが無い節は「なし」と書きます。**「## 引き継ぎ」は書きません**（道具が弾きます）。

```markdown
## 木への変更
| 結果 | op | 中身 |
|---|---|---|
| F31 | add_f | 頭皮脳波の発作起始の推定は… |
| Q9 | add_q | 発作の予兆の検出は可能か（許可を取って入った） |
| 未実行 | add_q | 発作の前兆の検出は可能か（許可が無かった） |

## OUT_OF_SCOPE
<範囲の外で気づいたこと。無ければ「なし」>
```

- 「結果」は `sai-sci op` が返した ID（`result`）です。出せなかったものは「未実行」と書き、理由を添えます。

## 終わり方（LabAgent の種類ごと）
あなたのブロックが node agent・manage agent・support agent のどれかは、ブロックの本文の最初に書いてあります。

- **node agent・manage agent**: REPORT を `sai-link doc write` で 1 つ保存してから終えます（上の「## 出力」）。正常に終わったかどうかは、REPORT があるかで見られます。
- **support agent**（子として呼ばれたとき）: REPORT は保存しません。呼んだ agent に `sai-link task report` で答えを返して終えます。

report を保存したら、次のどちらかで終えます。終えたあとにこの run は何もしません。

- **子の run（`SAI_IS_SUBTASK` が `1`）**: `sai-link task report` を呼びます。report には 3〜10 行で、結論・保存した report のトークン・`sai-sci op` で出した ID・残った問いを書きます。
- **root の run（`SAI_IS_SUBTASK` が `0` あるいは無い）**: 要点を伝え、質問ツールで「完了してよいか」を起動者に確かめます。了承されたら、`sai-link run finalize` を `report`（上と同じ要点）付きで呼びます。直しを求められたら、直して保存し直してから、もう一度確かめます。
- 例外: 実験の連鎖（`sci-exp-branch`・`sci-exp-review`）は、確かめずに `sai-link run finalize` で終えます（ブロックの本文のとおり）。
