# Library の検索（search-internal）

sai-paper の Library の中（取り込み済みの論文・ノート・ドキュメント）を検索する手順です。
外部の論文 DB を検索するときは `search-external` を使います。

## ステップ 1: 索引の状態を確かめる

```bash
sai-paper index status
```

`indexes.<名前>.chunks` と `chunks_with_embeddings` を見て、Library の断片と埋め込みができているか確かめます。

- 索引がまだ無い・件数が 0 のときは、**自分で作りません**。ユーザーに `sai-paper index build` を頼んでください（索引と埋め込みはユーザーが手動で作るときだけ作る、という決まりです）。
- pi からも cc からも MCP の `index_status` で同じ内容を読めます。

## ステップ 2: 問いを英語の検索語に分ける

日本語の問いは、まず英語の検索語に直します（Library の本文は英語が多く、`--mode word` で日本語が当たりにくいためです）。
問いを 3〜5 個の小さな問いに分け、それぞれ英語の検索語を作ります。

例：「てんかんの脳波解析で使われる深層学習の手法は？」
- `epilepsy EEG deep learning`
- `seizure detection neural network`
- `epilepsy EEG feature extraction`
- `seizure prediction convolutional neural network`

## ステップ 3: それぞれ検索する

```bash
sai-paper find "<英語の検索語>" \
  --mode hybrid \
  --top-k 10 \
  --scope all \
  --json
```

引数の意味:
- `--mode hybrid`: 単語（word）と意味（vector）の両方で検索して再ランクします。
- `--top-k 10`: 上位 10 件を返します。
- `--scope all`: 論文（papers）・ノート（notes）の両方を対象にします。論文だけなら `--scope papers`、ノートだけなら `--scope notes`。
- `--json`: JSON で返します。

pi からも cc からも MCP の `find` で同じ検索ができます。

広い問いで結果が多すぎるときは `--evidence`（MCP は `evidence: true`）を使ってよい。費用がかかるので 1 回の作業で 3 回まで。

## ステップ 4: 結果をまとめる

`--json` の各ヒットは次の欄を持ちます。

- `score`: スコア（大きいほど関連が強い。絶対値ではなく相対的な目安）
- `chunk_id`: 断片の ID
- `source_kind`: `abstract`（抄録）/ `body`（本文）/ `note`（ノート）/ `document` のどれか
- `source_ref`: 出所の参照（論文 ID やノート ID など）
- `paper_id`: 論文 ID（論文のとき。Library で出典を示す番号）
- `title` / `authors` / `year`: 出典情報（あるとき）
- `text`: ヒットした本文
- `md_path`: Markdown のパス（あるとき）

まとめ方:
- 同じ論文（同じ `paper_id`）の複数の断片は 1 つにまとめます。
- 複数の検索語で出た論文は関連が強いので上に置きます。
- `source_kind` が同じくらいなら本文（`body`）を優先します。

## ステップ 5: 足りなければ近い論文を広げる

上位の論文について、その論文の中の近い断片や関連論文を探します。

```bash
sai-paper neighbors <論文ID> --json
```

cc からは MCP の `neighbors` でも同じです。

## ステップ 6: それでも足りなければ GAP を残す

Library に無い根拠が必要なときは、答えを捏造せず、次の形で残します。

```
[GAP: 推奨検索: <英語の検索語>]
```

外部 DB を検索できるブロックなら `search-external` の手順に進みます。

## 既知の注意

- **日本語の単語検索**: `--mode word` は日本語に当たりません（FTS5 のトークナイザの制限）。日本語のノートを検索するときは、埋め込みサーバを起動したうえで `--mode hybrid` または `--mode vector` を使ってください。
- **スコアの相対性**: `score` は絶対値ではなく相対的な目安です。上位のヒットほど関連が強いとみてください。

## 質問集への提供（任意）

良い答えが見つかったときは、report の末尾（引き継ぎの前）に、質問集に足せる 1 行を書いてよい。

```json
{ "q": "<英語の問い>", "expect": [ { "paper": <番号> } ] }
```

質問集のファイル（`.sai/paper/eval/rag-questions.json`）に足すのはユーザーです。
