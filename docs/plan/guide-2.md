# 実装ガイド 2/5: Web から herdr を外し、Chats で headless の実行と会話する

## ゴール
`apps/web` から herdr と承認（PermissionRequest）の仕組みがすべて消える。Chats の会話画面で、headless の実行の生成中の文が 1 秒ごとに流れて見え、入力欄から続きを送り、⏸ で中断できる。Chats の一覧の「＋」から、プロジェクト・block・runner・model・thinking・最初の文を選んで新しいチャット（＝新しいカード）を始められる。質問カードに答えると、cc・pi のエージェントが続ける。プッシュ通知（質問・長い作業の終わり）は `state.json` と質問の登録から出る。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U03 | Web のサーバから herdr を外し、headless と会話する | `docs/plan/unit-U03-web-headless-server.md`（waggle:VEE38F99） | `apps/web/src/sai_web/` の `herdr.py`・`approvals.py`（消す）・`runs.py`・`main.py`・`control.py`・`questions.py`・`store.py`・`home.py`・`watch.py`・`notify.py`・`webcli.py`・`config.py`・`sessions.py`・Python のテスト・`docs/integration.md`・`docs/for-sai-developers.md` | ガイド 1 | 可 |
| U04 | Chats の画面と、承認・herdr の画面の削除 | `docs/plan/unit-U04-chat-screen.md`（waggle:ipNeqWgg） | `static/app.js`・`chatorder.js`・`newchat.js`（新規）・`home.js`・`linkview.js`・`index.html`・`sw.js`・`style.css`・JS のテスト | ガイド 1 | 可 |

## 実行順序
U03 ∥ U04（並列）。書くファイルが重ならない。U04 は U03 の API（C6。U04 の計画書に全文）だけに頼る。
- U04 の AC2（実際のサーバで逐次表示を確かめる）は、U04 の作業用コピーに U03 が無いので、**両方をマージした後に kantoku が行う**。U04 の worker には「AC2 は偽の応答を返す `saiApi` の差し替えでの画面写真まで。実サーバでの確認は kantoku が行う」と追加指示で伝える。
- U03 の作業用コピーでは JS のテストが落ちることがある（API の形の変更）。U03 の worker には「JS のテストは U04 の範囲。落ちたら理由を報告に書けばよい」と伝える。両方のマージ後に JS・Python のテストがすべて通ることを kantoku が確かめる。

## 所有権の表
| ファイル | U03 | U04 |
|---|---|---|
| `apps/web/src/sai_web/*.py`・Python のテスト・`docs/` | 書く | 読む |
| `apps/web/src/sai_web/static/*`・JS のテスト | 読む | 書く |

## 共有契約
- C1 headless の書き出し・C2 送信・中断の CLI（ガイド 1）。U03 の計画書に全文。
- C6 チャットの API（U03 が作り U04 が使う。両方の計画書に全文）。

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| Web に herdr と承認が残らない | U03・U04 | U03 の AC1、U04 の AC4 |
| 生成中の文が逐次に見える | U03・U04 | 統合後の U04 の AC2 |
| 送信・中断が動く | U03・U04 | U03 の AC3、統合後の実物の確認 |
| 新しいチャットが始められる | U04 | U04 の AC3 |
| 質問に答えられる（cc・pi） | U03 | U03 の AC4、統合後の実物の確認 |
| 通知が出る | U03 | U03 の AC5 |

## 統合と検証
- U03 → U04 の順にマージし、毎回 Web のテスト（JS・Python）をすべて回す。
- 統合後、kantoku が一時フォルダのプロジェクト（`SAI_HOME` も一時フォルダ）で daemon を起こし、8299 番の Web（同じ `SAI_HOME`。`SAI_LINK_BIN` は `target/release/sai-link`）から、費用の小さい model の cc で「新しいチャット」を始め、逐次表示・続きの送信・中断・質問カード（「AskUserQuestion で好きな色を聞いてから答えよ」）を確かめる。pi でも質問カードを 1 回確かめる。画面写真（390px・1280px）を完了報告書に付ける。daemon とサーバを止め、一時フォルダを消す。本番の Web（8200 番）には触らない。

## 決定事項と理由 / 採用しなかった案
- チャットは今の Chats を直して使う（利用者の決定）。/ Open WebUI は「利用者が送った文への返事」しか表示できず、task.md から自分で動き続ける run を見る用途に合わないので採らない（調査 R1）。
- 確定した会話は今の jsonl から、生成中の文だけ `state.json` から読む。Web は stream を読み解かない。
- 配信は 1 秒のポーリング。SSE・WebSocket は足さない。
- 新しいチャットは既存の「カード作成 → 実行」の API を続けて呼ぶ。runner・model・thinking はカード（task.md）に書かれる。
- 承認は丸ごと消す（headless では来ない）。

## リスクと対処
- メインの実行が `doc write` の後に review へ移り、`status` が `running` でなくなる: headless の実行は `state.json` が `stopped` でない限り会話を続けられる（C6）。画面の入力欄もそれに従う。
- 新しいチャットの直後は実行の書き出しがまだ無い（約 5 秒）: U04 の「起動を待っています」で吸収する。
- 既存のテストが herdr・承認の偽装に頼っている: 消してよい（消したテストの名前を報告に書く）。pi の質問のテストは残す。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（waggle:y7EoLAgV）
- 調査 E3（Web 側の herdr・承認・質問の今の経路）: `docs/plan/research/E3-mac-only-removal.md`
- 調査 R1（Open WebUI を採らなかった理由）: `docs/plan/research/R1-openwebui.md`
