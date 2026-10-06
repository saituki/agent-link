# 子タスクの使い方（sai-task）

子タスクは、別の pane で動く別のエージェントである。あなたの会話は見えない。子に伝えたいことは、すべて `task_text` に書く。

## 流れ

```
block_discover → task_create → task_execute → task_output（待つ・見張る）→ 確かめる → task_manage finalize
                                                  │
                                                  └ 止まった・長引いた → task_capture → task_manage interrupt / send / delete
```

| 道具 | 使いどころ |
|---|---|
| `block_discover` | 呼べる子ブロックの id を確かめる。`task_create` の前に必ず呼ぶ |
| `task_create` | 子を作る。`tasks: [{ client_id, subject, block_type, task_text, depends_on, worktree, cwd, timeout }]`。`cwd` は別のプロジェクトで動かすとき、`timeout`（秒）は「この時間を超えたら様子を見たい」目安があるときに付ける。実行すると子の run フォルダに task.md（description と task_text）ができ、TUI と Web で見られる |
| `task_execute` | 実行する（`batch_id` で一括、`task_id` で 1 件）。すぐ戻る |
| `task_output` | 子を待つ。1 回の待ちは最大 90 秒（`timeout_ms` はそれを超えても 90 秒に切り詰められる。既定も 90 秒）。完了なら `report`（短い報告）と `outputs`（子が保存した文書の `waggle:<tok>: <description>` 一覧）を返す。止まった・長引いたときは完了前でも `stalled` で戻る。複数の子は `task_ids: [..]` で 1 回で待つ（下の「子を待つ・見張る」） |
| `task_status` / `task_list` | 状態を見る。`worktree_info` にマージ用のブランチ名と作業用コピーの場所が入っている |
| `task_capture` | 子の画面を見る（`lines`、既定 50）。止まった・長引いた原因を調べるときに使う |
| `task_manage` | 子を操作する。下の表 |

`task_manage` の action:

| action | すること | 使える状態 |
|---|---|---|
| `send` | 同じ子に追加指示を送る（`message` 必須） | 入力待ちの作業中の子（その場で届く）、完了した子（次のラウンドになる） |
| `interrupt` | esc を送って、作業中の子を止め、入力待ちに戻す（人間の esc） | 作業中の子 |
| `delete` | 子を削除する。作業中なら pane を閉じて強制終了する（人間の ctrl+d） | いつでも |
| `finalize` | 完了を確定し、pane を閉じる | 確かめ終わった子 |
| `requeue` | 最初からやり直す | `failed` / `deleted` の子 |

`task_manage` が断ったときは、エラーで理由と次の手順が返る。その手順に従う。

## task_text の書き方

```
あなたの担当は <ID>「<件名>」である。下の文書が作業の正である。

waggle:<文書のトークン>

## 追加指示
- <補足・訂正。無ければ「なし」>

完了したら作業報告書を sai_write で保存し、最後に task_report を呼ぶこと。
```

- 文書はトークンで渡す。中身を要約し直さない。要約すると情報が落ちる。
- 長い文書を丸ごと埋め込むと、子のコストが増える。関係する事実だけを書くか、「`sai_read` で `<トークン>` を読むこと」と書く。
- `subject` は `<ID>: <件名>`、`client_id` は文書の ID にする。順番があるものは `depends_on` で表す。

## 作業場所

- `worktree` を付けないと、ブロック定義の `worktree:` に従う。`"isolation"` なら作業用コピーを作って、その中で動かす。`"none"` なら作業用コピーを作らず、本体のフォルダで動かす（ブロック定義が `isolation` でも打ち消す）。
- `"none"` は、ファイルを書き換えない調査や、コード以外の作業に使う。コードを書き換える子は作業用コピーで動かす。
- 同じファイルを書く子を並列にしない。並列にしてよいのは、書くファイルが重ならず、作業用コピーで動く子だけである。
- 作業用コピーの変更は、全員が終わってから依存順にマージする。マージごとにビルドとテストを行う。競合したら `git merge --abort` し、マージ済みの状態からその作業をやり直させる。
- 終わった作業用コピーは片付ける。

```bash
git worktree unlock <絶対パス>
git worktree remove <絶対パス>
git branch -d <ブランチ名>
```

## 子を待つ・見張る

`task_output` の戻り値の `status` を見て動く。

| status（reason） | 意味 | すること |
|---|---|---|
| `completed` | 子が `task_report` を呼んだ | 下の「報告を受け取ったあと」へ |
| `failed` | 子の実行が失敗した | 原因を調べ、直してから `requeue` するか、作り直す |
| `timeout` / `in_progress` | `task_output` の待ち時間が切れただけ（最大 90 秒。`timeout_ms: 0` なら即時に返る）。子はまだ作業中 | もう一度 `task_output` を呼ぶ |
| `stalled`（`idle` / `blocked` / `error`） | 子が入力待ち・確認待ち・エラーで止まった | `task_capture` で画面を見る。`idle` なら `send` で指示を送る。`blocked` / `error` なら先に `interrupt` してから `send` |
| `stalled`（`long_running`） | 子が作業中のまま長く続いている（既定 20 分ごとに届く） | `task_capture` で画面を見て、進み具合を判断する（下） |
| `stalled`（`timeout`） | `task_create` の `timeout` を超えた（同じ間隔で繰り返し届く）。子は止めていない | 同上 |

`long_running` / `timeout` を受けたときの判断:

- 画面が進んでいる（命令が次々に実行されている）なら、続けさせる。もう一度 `task_output` を呼ぶ。
- 1 つの命令が終わらずに止まっている（画面が進まず、同じ命令の終了を待ち続けている）なら、`interrupt` で止める。次に `send` で「その命令が終わらない原因を調べ、時間の上限を付けるか、別の方法で進めよ」と送る。
- 間違った作業を続けているなら、`interrupt` で止め、`send` で正しい方向を書いて送る。
- `interrupt` しても止まらない、または作り直すほうが早いなら、`delete` して新しく作る。

- 作業中（`working`）の子に `send` はできない。方向を変えるときは、必ず `interrupt` → `send` の順にする。
- `stalled`（質問を含む）が届くのは、その子の `task_id` を `task_output` で待っているときだけである。
- 子を並列に動かすときは、`task_output` を子ごとに重ねて呼ばない。`task_ids: [..]` に待ちたい子をまとめて渡し、1 回で待つ。
  - 最初に完了した子か `stalled` になった子が 1 件返る。残りの子の `task_id` は `pending` に入る。
  - その子を処理したら、次の呼び出しの `task_ids` は `pending` の子だけにする。返った子を入れたままにすると、読み終えた子を待ち続ける。
  - 誰も準備できないまま 90 秒たつと、`status: "in_progress"` と `pending`（全件）で返る。もう一度 `task_ids` を渡して呼ぶ。

## 報告を受け取ったあと

- 報告を信じるだけで `finalize` しない。変更（`git diff`・コミット）とテスト結果を自分で確かめる。
- 足りなければ `send` で、何が足りないか・どうなれば完了かを書いて送る。
- 終わる前に、子をすべて `finalize` 済みにする。未完了の子を残さない。
