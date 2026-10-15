# 子タスクの使い方（link-task）

子タスクは、別の pane で動く別のエージェントである。あなたの会話は見えない。子に伝えたいことは、すべて `task_text` に書く。

## 流れ

```
agent-link def list --parent <自分の block> → agent-link task create → agent-link task execute → agent-link task wait（待つ・見張る）→ 確かめる → agent-link task finalize <id>
                                                                                          │
                                                                                          └ 止まった・長引いた → agent-link task capture <id> → agent-link task interrupt / send / delete
```

| 命令 | 使いどころ |
|---|---|
| `agent-link def list --parent <自分の block>` | 呼べる子ブロックの id を確かめる。`agent-link task create` の前に必ず実行する |
| `agent-link task create` | 子を作る。標準入力に子の配列の JSON を渡す（`[ { client_id, subject, block_type, task_text, depends_on, worktree, cwd, timeout, model, thinking } ]`）。`model`（省略可。`opus` / `sonnet` / `haiku` / `inherit` / `claude…`）と `thinking`（省略可。`none` / `low` / `medium` / `high` / `xhigh` / `max`）は、省くと block の frontmatter の値が使われる。`runner` は書かない（cc だけ）。`cwd` は別のプロジェクトで動かすとき、`timeout`（秒）は「この時間を超えたら様子を見たい」目安があるときに付ける。実行すると子の run フォルダに task.md（description と task_text）ができ、TUI と Web で見られる。出力は `batch_id` と、子ごとの `tasks[]`（`client_id`・`status`・`task_id`） |
| `agent-link task execute --batch <batch_id>` / `--task <task_id>` | 実行する（`--batch` で一括、`--task` で 1 件）。すぐ戻る |
| `agent-link task wait <task_id>… [--timeout-ms N]` | 子を待つ。1 回の待ちは最大 90 秒（`--timeout-ms` はそれを超えても 90 秒に切り詰められる。既定も 90 秒。`--timeout-ms 0` なら即時に返る）。完了なら `report`（短い報告）と `outputs`（子が保存した文書の `waggle:<tok>: <description>` 一覧）を返す。止まった・長引いたときは完了前でも `stalled` で戻る。複数の子は `agent-link task wait <task_id> <task_id> …` で 1 回にまとめて待つ（下の「子を待つ・見張る」） |
| `agent-link task show <task_id>` / `agent-link task list` | 状態を見る。`worktree_info` にマージ用のブランチ名と作業用コピーの場所が入っている |
| `agent-link task capture <task_id> [--lines N]` | 子の画面を見る（`--lines`、既定 50）。止まった・長引いた原因を調べるときに使う |
| `agent-link task send <task_id> --message <文>` | 子に追加指示を送る。下の表 |

子を操作する命令:

| 命令 | すること | 使える状態 |
|---|---|---|
| `agent-link task send <task_id> --message <文>` | 同じ子に追加指示を送る | 入力待ちの作業中の子（その場で届く）、完了した子（次のラウンドになる） |
| `agent-link task interrupt <task_id>` | esc を送って、作業中の子を止め、入力待ちに戻す（人間の esc） | 作業中の子 |
| `agent-link task delete <task_id>` | 子を削除する。作業中なら pane を閉じて強制終了する（人間の ctrl+d） | いつでも |
| `agent-link task finalize <task_id>` | 完了を確定し、pane を閉じる | 確かめ終わった子 |
| `agent-link task requeue <task_id>` | 最初からやり直す | `failed` / `deleted` の子 |

命令が断ったときは、終了コードと標準エラーの最後の行に理由と次の手順が出る。その手順に従う。

本文を標準入力で渡す命令の例:

```bash
agent-link task create <<'EOF'
[
  { "client_id": "U01", "subject": "U01: placeholder の文中展開を直す", "block_type": "worker", "task_text": "下の計画書に従って実装せよ。\n\nwaggle:<トークン>", "worktree": "isolation" }
]
EOF

agent-link task report <<'EOF'
結果: DONE
要約: placeholder の文中展開を実装した
問題点: なし
親への依頼: なし
EOF
```

## task_text の書き方

```
あなたの担当は <ID>「<件名>」である。下の文書が作業の正である。

waggle:<文書のトークン>

## 追加指示
- <補足・訂正。無ければ「なし」>

完了したら作業報告書を `agent-link doc write` で保存し、最後に `agent-link task report` を実行すること。
```

- 文書はトークンで渡す。中身を要約し直さない。要約すると情報が落ちる。
- 長い文書を丸ごと埋め込むと、子のコストが増える。関係する事実だけを書くか、「`agent-link doc read <トークン>` で読むこと」と書く。
- `subject` は `<ID>: <件名>`、`client_id` は文書の ID にする。順番があるものは `depends_on` で表す。
- `tasks[]` の `task_id` が null のときは、`agent-link task list` の `server_id` で確かめる。
- 子でない run（`card run` で起こした run）は `agent-link task wait` で待てない。`agent-link run wait <run_id>` を使う。

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

`agent-link task wait` の戻り値の `status` を見て動く。

| status（reason） | 意味 | すること |
|---|---|---|
| `completed` | 子が `agent-link task report` を実行した | 下の「報告を受け取ったあと」へ |
| `failed` | 子の実行が失敗した | 原因を調べ、直してから `agent-link task requeue <task_id>` するか、作り直す |
| `timeout` / `in_progress` | `agent-link task wait` の待ち時間が切れただけ（最大 90 秒。`--timeout-ms 0` なら即時に返る）。子はまだ作業中 | もう一度 `agent-link task wait <task_id>` を実行する |
| `stalled`（`idle` / `blocked` / `error`） | 子が入力待ち・確認待ち・エラーで止まった | `agent-link task capture <task_id>` で画面を見る。`idle` なら `agent-link task send <task_id> --message <文>` で指示を送る。`blocked` / `error` なら先に `agent-link task interrupt <task_id>` してから `agent-link task send` |
| `stalled`（`long_running`） | 子が作業中のまま長く続いている（既定 20 分ごとに届く） | `agent-link task capture <task_id>` で画面を見て、進み具合を判断する（下） |
| `stalled`（`timeout`） | `agent-link task create` の `timeout` を超えた（同じ間隔で繰り返し届く）。子は止めていない | 同上 |

`long_running` / `timeout` を受けたときの判断:

- 画面が進んでいる（命令が次々に実行されている）なら、続けさせる。もう一度 `agent-link task wait <task_id>` を実行する。
- 1 つの命令が終わらずに止まっている（画面が進まず、同じ命令の終了を待ち続けている）なら、`agent-link task interrupt <task_id>` で止める。次に `agent-link task send <task_id> --message <文>` で「その命令が終わらない原因を調べ、時間の上限を付けるか、別の方法で進めよ」と送る。
- 間違った作業を続けているなら、`agent-link task interrupt <task_id>` で止め、`agent-link task send <task_id> --message <文>` で正しい方向を書いて送る。
- `agent-link task interrupt <task_id>` しても止まらない、または作り直すほうが早いなら、`agent-link task delete <task_id>` して新しく作る。

- 作業中（`working`）の子に `agent-link task send` はできない。方向を変えるときは、必ず `agent-link task interrupt <task_id>` → `agent-link task send <task_id> --message <文>` の順にする。
- `stalled`（質問を含む）が届くのは、その子の `task_id` を `agent-link task wait <task_id>` で待っているときだけである。
- 子を並列に動かすときは、`agent-link task wait` を子ごとに重ねて呼ばない。待ちたい子の `task_id` を 1 つの `agent-link task wait <task_id> <task_id> …` にまとめ、1 回で待つ。
  - 最初に完了した子か `stalled` になった子が 1 件返る。残りの子の `task_id` は `pending` に入る。
  - その子を処理したら、次の `agent-link task wait` には `pending` の子だけを渡す。返った子を入れたままにすると、読み終えた子を待ち続ける。
  - 誰も準備できないまま 90 秒たつと、`status: "in_progress"` と `pending`（全件）で返る。もう一度 `agent-link task wait` を実行する。

## 報告を受け取ったあと

- 報告を信じるだけで `agent-link task finalize <task_id>` しない。変更（`git diff`・コミット）とテスト結果を自分で確かめる。
- 足りなければ `agent-link task send <task_id> --message <文>` で、何が足りないか・どうなれば完了かを書いて送る。
- 終わる前に、子をすべて `agent-link task finalize <task_id>` 済みにする。未完了の子を残さない。
