# agent-link CLI の使い方（link-cli）

`agent-link` コマンドは、シェル（Bash）から agent-link の記録を調べたり、次のカード（task.md）を作ったりするときに使う。
文書の保存と読み込みは `agent-link doc write` / `agent-link doc read` で行う。
自分の run_id は環境変数 `AGENT_LINK_PARENT_RUN_ID` に入っている。

## 過去の実行を調べる

| やりたいこと | コマンド |
|---|---|
| 実行の一覧（todo / running / review / 日付ごと。子の実行は親の下） | `agent-link run tree` |
| 1 つの実行の詳細（ブロック・状態・時間・コミット） | `agent-link run show <run_id>` |
| その実行の依頼文・報告・会話 | `agent-link run md <run_id> --kind task`（`message` / `report` / `system` も可） |
| その実行が保存した文書 | `agent-link doc list --run <run_id> --all`（本文は `agent-link doc read <トークン>` で読む） |
| 過去の会話を言葉で探す | `agent-link session search -w <語,語>`（`--block <名>`・`--json` で絞り込み） |

結果はほとんどが JSON で返る。長いときは必要な部分だけ取り出して読む。

## 次のカード（task.md）を作る

作業を次のエージェントに引き継ぐとき、todo にカードを作る。**実行はユーザーが行う。** あなたはカードを作るだけで、実行しない。

1. 使えるブロック・model・thinking の値を確かめる。

```bash
agent-link card options
```

2. 標準入力に JSON を渡してカードを作る。戻り値の `run_id` と `path` を控える。

```bash
agent-link card create <<'EOF'
{
  "block": "kantoku",
  "model": "sonnet",
  "thinking": "high",
  "task_text": "下の実装ガイドに従って実装を完遂せよ。\n\nwaggle:<ガイドのトークン>"
}
EOF
```

- 空の項目は既定値になる。`model` / `thinking` を空にすると、settings.json の既定が入り、実行時は block の frontmatter の値になる。
- `task_text` に `waggle:<トークン>` を書くと、実行時に本文が埋め込まれる。

3. 戻り値の `path` のファイルを編集し、frontmatter の 2 行を埋める。
   - `description:` に、カードの説明を 1 行で書く（TUI のカードに表示される）。

4. ユーザーに、作ったカードの run_id と内容を伝える。

## 使わないコマンド

次のコマンドは、実行の開始・削除・設定の変更になる。ユーザーの指示が無い限り使わない。

`agent-link card run` / `agent-link run finalize` / `agent-link run advance` / `agent-link run rm` / `agent-link daemon` / `agent-link init` / `agent-link run md --write`

ただし `agent-link card run` は、役の説明に起動の手順として書いてあるときは使う。その場合も、起動した run は `agent-link run wait <run_id>` で待つ。
