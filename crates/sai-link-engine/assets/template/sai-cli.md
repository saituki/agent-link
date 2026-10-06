# sai-link CLI の使い方（sai-cli）

`sai-link` コマンドは、シェル（Bash）から sai-link の記録を調べたり、次のカード（task.md）を作ったりするときに使う。
文書の保存と読み込みは `sai-link doc write` / `sai-link doc read` で行う。
自分の run_id は環境変数 `SAI_PARENT_RUN_ID` に入っている。

## 過去の実行を調べる

| やりたいこと | コマンド |
|---|---|
| 実行の一覧（todo / running / review / done ごと） | `sai-link run tree` |
| 1 つの実行の詳細（ブロック・状態・時間・コミット） | `sai-link run show <run_id>` |
| その実行の依頼文・振り返り・会話 | `sai-link run md <run_id> --kind task`（`review` / `message` も可） |
| その実行が保存した文書 | `sai-link doc list --run <run_id> --all`（本文は `sai-link doc read <トークン>` で読む） |
| 過去の会話を言葉で探す | `sai-link session search -w <語,語>`（`--block <名>`・`-R <グローバルラウンド>`・`--json` で絞り込み） |

結果はほとんどが JSON で返る。長いときは必要な部分だけ取り出して読む。

## 次のカード（task.md）を作る

作業を次のエージェントに引き継ぐとき、todo にカードを作る。**実行はユーザーが行う。** あなたはカードを作るだけで、実行しない。

1. 使えるブロック・runner・model・thinking の値を確かめる。

```bash
sai-link card options
```

2. 標準入力に JSON を渡してカードを作る。戻り値の `run_id` と `path` を控える。

```bash
sai-link card create <<'EOF'
{
  "runner": "pi",
  "block": "kantoku",
  "thinking": "high",
  "global_round": "16",
  "round": "1",
  "task_text": "下の実装ガイドに従って実装を完遂せよ。\n\nwaggle:<ガイドのトークン>"
}
EOF
```

- 空の項目は既定値になる。`model` / `provider` を空にすると、ブロック定義の値が使われる。
- `task_text` に `waggle:<トークン>` を書くと、実行時に本文が埋め込まれる。
- `done_ids` に run_id を並べると、その実行の振り返り（review）がカードに付く。振り返りがまだ無い実行は指定できない。

3. 戻り値の `path` のファイルを編集し、frontmatter の 2 行を埋める。
   - `description:` に、カードの説明を 1 行で書く（TUI のカードに表示される）。
   - 自分の実行から引き継ぐときは、`origin_run_id:` に自分の run_id を書く（TUI の Workflow に線が引かれる）。

4. ユーザーに、作ったカードの run_id と内容を伝える。

## 使わないコマンド

次のコマンドは、実行の開始・削除・設定の変更になる。ユーザーの指示が無い限り使わない。

`sai-link card run` / `sai-link run finalize` / `sai-link run advance` / `sai-link run rm` / `sai-link daemon` / `sai-link init` / `sai-link run md --write`
