# 研究方針の木で動くエージェントの共通の決まり（sci-core）

このファイルは sai-moon-paper が研究プロジェクトに配った最初の版です。ユーザーが自由に直せます。

## 今回の SCOPE
最初のメッセージの「# context」の節は、あなたが起動する直前に研究方針の木から切り出して作り直されたものです。SCOPE を決めるための文書です。同じメッセージの「# task_text」（依頼文）と重ねて読んでください。「# context」の中の「## CURRENT_ACTION（sai-sci より）」から、その回の行動・NODE・SCOPE の各節が並びます。

## 起動者と質問
- あなたを起こしたのは、カードの起動者です。`echo "${SAI_IS_SUBTASK:-0}"` が `1` なら子（professor が起こした）、`0` なら root（利用者 Human が起こした）です。
- 確かめ・質問は、**必ず質問ツール**で聞きます（pi は `ask_user_phone`、Claude Code は `AskUserQuestion`）。返答の本文だけで尋ねて止まってはいけません。質問は Web の画面にも出るので、利用者も見ています。先に答えた方が採られます。
- 質問の文・選択肢・報告に、鍵・合言葉の値を書きません。値が要るときは環境変数の名前だけを書きます。

## 最初にすること
1. この回の行動の目的を、1 文で言い直して示します。
2. 最初のメッセージの「# context」の「## CURRENT_ACTION」に NODE の指定があれば、その ITEM（NODE の ID と名前）を先頭に書きます。無いときは推測で ITEM を選び直しません。作業を始めてはいけません。質問ツールで起動者に「研究方針のどの ITEM のための行動か」を確かめて止まります。
3. 「# context」の節が「（…範囲を作れませんでした…）」の 1 行だけだったときは、MCP の `sci_context`（pi では `sai-moon-paper sci context --run "$SAI_PARENT_RUN_ID"`）で取り寄せてから始めます。それでも取れなければ、質問ツールでそのことを起動者に伝えて止まります。木がまだ無いときは、起動者に `sai-moon-paper init --assets-only --sci --type <型>` の実行（または Web の Research-tree の画面の「始める」）を頼みます。

## 確かめの地点
次の 2 か所で止まり、質問ツールで「この方向で進めてよいか」を起動者に聞いてください。
1. 目的を言い直したとき
2. PROPOSAL を出す前

依頼文に `確認: 少なめ` とあれば 2 の前だけで止まってください。`確認: 多め` とあれば 1・2 に加えて「3. 検索を始める前（検索語の一覧を見せる）」でも止まってください。依頼文に書いていなければこの既定（1・2）に従ってください。

## SCOPE と ASSUMPTION を守る
- SCOPE の節（TARGET_NODE・UPSTREAM_PATH・OUT_OF_SCOPE・ASSUMPTION）はこの回で扱う範囲です。OUT_OF_SCOPE の話を良いからと広げません。
- SCOPE の外で気づいたことは、report の「## OUT_OF_SCOPE」に書いて議論しません。利用者が見たうえで、次のカードに活かします。
- ASSUMPTION（この回では問い直さないと決めたもの）は前提として使います。矛盾を見つけたときは「## OUT_OF_SCOPE」か「## PROPOSAL」に書き、利用者が直すようにします。
- このサイクルの他のテンプレート（`search-internal` など）に `project-brief` の記載や「引き継ぎ」の書き方があっても、**このファイルの決まりが優先**です。引き継ぎは書きません。

## 研究方針の木への PROPOSAL
研究方針の木・`mind.md`・`state.md`・`tree.md` を直接書き換えてはいけません。木への変更はすべて PROPOSAL として出します。

- MCP の `sci_propose`（pi では `sai-moon-paper sci propose`）で出します。人が画面（Research-tree）で作業するときの操作と同じ JSON を書きます。

### PROPOSAL の書き方
PROPOSAL は 1 つの JSON か、`{"proposals": [ … ]}`（20 件まで）です。全部を先に検査し、1 つでも通らなければ何も保存されません。`reason`（なぜこの PROPOSAL を出すか、1〜1000 字）は**必ず**書きます。`{"error": "<n> 件目: <理由>"}` の形で、どの PROPOSAL が通らなかったかが返ります。形が通らなければ、その場で直して出し直してください。

NODE・QUESTION・FINDING・LINK・PROPOSAL・SEED の ID は `N12`・`Q7`・`F31`・`L3`・`P55`・`S1` の形です。数だけ（`12`）は書きません。

```json
{"proposals": [
  {"op": "add_finding", "node": "N20", "text": "頭皮脳波の発作起始の推定は、頭蓋内脳波との一致率が 6 割前後にとどまる。", "cites": ["#37.5.2"], "polarity": "support", "level": 2, "reason": "OPTION N20 の実現性の根拠"},
  {"op": "add_option", "node": "N12", "title": "発作の予兆の検出", "reason": "#40 が臨床の要望として挙げている"}
]}
```

### 使える op
ここで説明しているのは、エージェントが出せる 14 種類です。LINK を外す・READ_STATE を変える・FINDING を消す・QUESTION を消す・線を消す操作は人の操作なので、ここには載せません。

| op | 主な指定 | 結果 | 危険度 |
|---|---|---|---|
| `add_finding` | `node`（N）・`question`（Q・任意）・`text`（1〜3 文・1〜1200 字）・`cites`（文字列の配列・10 個まで）・`polarity`（`support` 支持 / `refute` 反証 / `neutral` 中立 / `none` 該当なし）・`level`（1〜4・任意）・`paper`（論文の番号・任意） | FINDING `F<id>` | 自動 |
| `add_question` | `node`（N）・`text`（1〜300 字） | QUESTION `Q<id>` | 自動 |
| `answer_question` | `question`（Q）・`text`（1〜1200 字） | QUESTION `Q<id>` | 自動 |
| `link` | `node`（N）・`target`（`paper:37` / `report:R-0012` / `note:37` / `set:5` / `run:120`）・`role`（`paper` のときは必須）・`read_state`（`paper` のときは任意・既定 `unread`） | LINK `L<id>` | 自動 |
| `add_seed` | `title`（1〜120 字）・`sentence`（1〜600 字）・`path`（N の配列）・`doc`（waggle の参照・任意） | SEED `S<id>` | 自動 |
| `add_option` | `node`（CHOOSE か LIST の ITEM）・`title`（1〜120 字）・`body`（2000 字まで・任意） | OPTION `N<id>` | 人 |
| `add_child` | `node`（ITEM か OPTION）・`title`・`types`（`choose` 選ぶ / `list` 並べる / `write` 書く / `verify` 確かめる。1 つ以上）・`key`（ITEM_GUIDE のキー・任意） | ITEM `N<id>` | 人 |
| `decide` | `node`（ITEM）・`option`（N・CHOOSE の ITEM で OPTION があるときは必須）・`tentative`（TENTATIVE にするなら true）・`evidence`（F の配列・任意）・`rejected`（`[{"option": "N21", "reason": "…"}]`）・`counter_searched`（真偽） | DECISION_LOG | 人 |
| `reopen` | `node`（ITEM） | DECISION_LOG | 人 |
| `exclude_item` | `node`（ITEM） | DECISION_LOG | 人 |
| `option_state` | `node`（OPTION）・`state`（`rejected` / `dormant` / `candidate`） | DECISION_LOG | 人 |
| `exclude` | `target`（`paper:37` か `candidate:<dedupe_key>`）・`node`（任意）・`title`（任意） | TOMBSTONE | 人 |
| `edit_text` | `node`（N）・`title`（1〜120 字）・`body`（2000 字まで）。`title` と `body` のどちらか 1 つは必須 | ITEM `N<id>` | 人 |
| `add_edge` | `kind`（`depends` DEPENDS / `derived` DERIVED / `analogous` ANALOGOUS。`parent` 親子は使えない）・`from`（N）・`to`（N）・`note`（200 字まで・任意）。線の向きは `from` が上流、`to` が下流 | `from` の ITEM | 人 |

### AUTO_OP と REVIEW_OP
- **AUTO_OP**（`add_finding` `add_question` `answer_question` `link` `add_seed`）：出した PROPOSAL はすぐ木に入り、自動で反映された印が付きます。利用者は 1 押しで取り消せます。迷ったときに取り消せるので、集計に必要な FINDING はためらわず出してください。
- **REVIEW_OP**（上の残り 9 種類）：PROPOSAL は TRIAGE の箱で待っています。利用者が ACCEPT すると反映され、REJECT すると反映されません。木の形が変わる操作（OPTION や ITEM の追加、決定、線を引く）は、利用者が受け入れるまで反映されません。
- このブロックの frontmatter の `sci_ops` に無い op を出すと、「このブロックは `<op>` を提案できません」と言って拒否されます。
- 出したら、自分の report の「## PROPOSAL」に PROPOSAL の ID と状態を書いてください。

## 根拠
- 出典は `literature-hard-rules` に従います。文献を推測で書かない。API の返り値に無い PMID・DOI・題名・著者・年・誌は書いてはいけません。
- FINDING の `cites` は 塊の ID（`#37.5.2`）か `#37 Fig.3` の形です。論文全体を指す `#37` だけも使えます。
- エージェントの FINDING には `cites` が 1 つ以上必要です。0 個だと通りません。
- 文に数字（0〜9）が含まれるときは `cites` が要ります。無いと「数値を含む知見には出典が要ります」と弾かれます。
- `cites` の番号が Library に無い場合も弾かれます。先に `search` で Library に入れてから出してください。
- 根拠が足りないときは「Library の資料では答えられない」と書き、`[GAP: 推奨検索: <英語の検索語>]` を残します。

## 迎合しない
- いまの研究方針に不利な根拠を、少なくとも 1 つ探して書くようにします。反対の向き（`polarity: "refute"`、POLARITY は REFUTE）で出します。
- 探したが見つからなかったときは「探したが見つからなかった」と report に書いてください。支持の報告だけにしません。
- 事実（論文に書いてあること）と解釈（あなたの考え）を分けて書きます。

## 論文と ITEM の一致
論文が研究方針の ITEM の条件にどれくらい合うかを、点数にしないで表で示します。ITEM ごとに「一致・部分・不一致」と、その根拠（塊の ID `#37.5.2`）を書いてください。

```markdown
| 条件 | 項目の値 | 論文（#37） | 一致 | 根拠 |
|---|---|---|---|---|
| 対象 | 成人のてんかん患者 | 小児 | 不一致 | #37.3.1 |
| データの型 | 頭皮脳波 | 頭皮脳波 | 一致 | #37.3.2 |
| 出力の型 | 発作部位 | 発作の有無 | 部分 | #37.4 |
| 制約 | 少数・多施設 | 単施設 | 部分 | #37.3.3 |
```

- 合計点や重み付きの点数は書きません。「何が違うか」が分かるように書きます。
- ITEM が「選ぶ・並べる・書く・確かめる」のどれかによって、照合する条件を変えて構いません（選ぶなら OPTION の比較、確かめるなら「どこまで確めればよいか」）。

## 検索
- 検索するときは `--node` と `--question` を付けます。どの ITEM のための、どの QUESTION のための検索かが残っていなければ、木の上の掘り下げになりません。
- 日本語の問いは英語の検索語に直してから検索します。3〜5 個の小さな問いに分けることを勧めます。

```bash
sai-moon-paper find "<英語の検索語>" --node N12 --question Q7 --mode word --json
sai-moon-paper find "<英語の検索語>" --node N12 --subtree --each 3 --json
sai-moon-paper find "<英語の検索語>" --widen "NODE の論文で 0 件だったため" --question Q7 --json
sai-moon-paper search "<英語の検索語>" --db s2 --save-as <名前> --node N12 --question Q7 --purpose "<目的>" --extract "<EXTRACT_FIELDS>" --json
```

- 検索の幅は次の順に広げます。NODE の論文 → SUBTREE → SEARCH_SET → Library → 外部。
- 広げたら `--widen <理由>` を付けます。広げた理由が残っていないと、次に同じことを繰り返します。

## コマンド
`sai-moon-paper` のコマンド（`sai-moon-paper sci status` / `sai-moon-paper find` / `sai-moon-paper content report` など）は、そのまま打ちます。

`sai-moon-paper` が `command not found` になったら、自分でフルパスを探さずに、質問ツールで起動者に「`sai-moon-paper` に PATH を通してください（README の「起動」）」と伝えて止まってください。

## 出力
そのブロックの「## 出力の形」の見出しに従ってください。report は `mcp__sai__sai_write` で 1 つだけ保存してください（途中のメモは保存しないでください）。`filename` は `<ブロックID>-<内容を表す短い英字>.md`（例 `sci-read-onset-accuracy.md`）、`description` は「<ブロックの名前>: <内容を 1 文で>」にしてください。保存したら要点を伝え、末節の「完了のしかた」のとおりに終えます。

report の先頭には、次の frontmatter を必ず書いてください。無い・形が違う report は一覧で「形が読めない」と表示されます。

```markdown
---
kind: <report の種類。例: 回答 / 比較表 / 地図 / 種の札 / まとめ / 聞き取り>
block: <自分のブロック ID。例 sci-compare>
papers: [37, 40]
sci_node: N12
sci_question: Q7
---
```

- `papers` には、本文で `[#番号]` として引用した Library の論文の番号をすべて書きます。無ければ `[]` と書きます。
- `sci_node` には、あなたが扱った NODE を書きます。`sci_question` は、この実行で扱った QUESTION があるときだけ書きます。

report の末尾には、次の 2 つの節を必ず書いてください。この見出しは変えないでください（次のカードが読みます）。書くことが無い節は「なし」と書きます。**「## 引き継ぎ」は書きません。**

```markdown
## PROPOSAL
| PROPOSAL | 種類 | 状態 | 中身 |
|---|---|---|---|
| P55 | add_finding | AUTO_OP で反映（F31） | 頭皮脳波の発作起始の推定は… |
| P56 | add_option | REVIEW_OP（TRIAGE 待ち） | 発作の予兆の検出 |

## OUT_OF_SCOPE
<SCOPE の外で気づいたこと。無ければ「なし」>
```

- 「状態」には `AUTO_OP で反映（できた ID）` か `REVIEW_OP（TRIAGE 待ち）` を書きます。

## 完了のしかた
report を保存したら、次のどちらかで終えます。終えたあとにこの run は何もしません。

- **子の run（`SAI_IS_SUBTASK` が `1`）**: `task_report` を呼びます。report には 3〜10 行で、結論・保存した report のトークン・出した PROPOSAL の ID・残った問いを書きます。
- **root の run（`SAI_IS_SUBTASK` が `0` あるいは無い）**: 要点を伝え、質問ツールで「完了してよいか」を起動者に確かめます。了承されたら、`sai_finalize` を `report`（上と同じ要点）付きで呼びます。直しを求められたら、直して保存し直してから、もう一度確かめます。
