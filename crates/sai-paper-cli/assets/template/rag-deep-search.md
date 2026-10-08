> このテンプレートは search-external / search-internal に移りました。新しいブロックはそちらを使います（互換のため残しています）。

# RAG 深度検索ガイド

Library 内で複数の検索角度から論文・ノート・ドキュメントを掘り込み、結果を再ランクして出典付きでまとめるテンプレート。

## ステップ 1: インデックス状態の確認

```bash
sai-paper index status
```

チャンク化と埋め込みが完了しているか確認する。

## ステップ 2: クエリ拡張戦略の決定

ユーザーの質問から以下を抽出:
- **主要キーワード**: 核となる概念
- **下位概念・同義語**: 別の言い方
- **背景知識**: 前提知識や関連分野

例：「神経幹細胞の分化メカニズム」→
- 主要: `neural stem cell`, `differentiation`
- 下位/同義: `neurogenesis`, `cell fate`, `neural progenitor`
- 背景: `transcription factor`, `signaling pathway`

## ステップ 3: 複数クエリの実行

主要キーワード + 下位概念・同義語 + 背景知識で 3-5 回のクエリを実行:

```bash
# クエリ 1: 主要キーワード
sai-paper find "neural stem cell differentiation" \
  --mode hybrid \
  --top-k 15 \
  --scope all \
  --json > results1.json

# クエリ 2: 下位概念
sai-paper find "neurogenesis cell fate determination" \
  --mode hybrid \
  --top-k 15 \
  --scope all \
  --json > results2.json

# クエリ 3: 関連メカニズム
sai-paper find "transcription factor neural differentiation signaling" \
  --mode hybrid \
  --top-k 15 \
  --scope all \
  --json > results3.json
```

オプション説明:
- `--mode hybrid`: word と vector の両方で検索
- `--top-k 15`: 各クエリで 15 件取得（重複除外前）
- `--scope all`: 論文、ノート、ドキュメント全体
- `--json`: JSON で返す

## ステップ 4: 結果の統合と重複除外

複数クエリの結果を統合:
1. すべてのヒットを 1 つのリストに統合
2. 同じ chunk_id を持つヒットを除外（最初に出現したスコアを保持）
3. 同じ論文/ドキュメントの複数チャンクは保持（異なるセクションを重要視）

## ステップ 5: 再ランク

統合された全ヒットを以下の基準で再ランク:

1. **スコア（元のハイブリッド検索スコア）** — 高いほど重要
2. **出現頻度** — 複数のクエリで引っかかったら加点
3. **源種別** — 同点の場合、body > note > abstract > document の順

例:
```
# 最終ランク
1. chunk_id=512, score=0.92 (クエリ 1,2 で出現) → 論文本文
2. chunk_id=301, score=0.85 (クエリ 1 のみ) → 論文本文
3. chunk_id=205, score=0.78 (クエリ 2,3 で出現) → ノート
```

## ステップ 6: 引用をたどって補充

主要ヒットについて、関連論文を探す:

```bash
# 中心的な論文 (paper_id=123) の近傍チャンクを検索
sai-paper neighbors 123 --top-k 10 --json
```

結果から、既に Library に入っている関連論文や、さらに掘り込むべき論文を特定。

Library に未収録の重要な論文がある場合は、ユーザーに「次に検索すべき論文」として報告。

## ステップ 7: 出典付きでまとめる

再ランク後の結果を以下の形式で記録:

```markdown
# [ユーザーの質問]

## 回答

[総合的な回答 — 複数の根拠から統合]

### 主要な知見

- [ポイント 1]：xxx による（出典 1）
- [ポイント 2]：yyy による（出典 2）

### 詳細な考察

[3-5 セクションで掘り込み]

### 出典

1. **[論文タイトル]** （[著者], [年]）  
   DOI: [DOI]  
   source_kind: body / abstract  
   引用元: [該当セクション / チャンク]

2. **[別の論文]** （[著者], [年]）  
   DOI: [DOI]  
   source_kind: note  
   ノートパス: [パス]

3. **[ドキュメント]**  
   ドキュメント種別: recommendation / recap / outline  
   パス: [パス]

### 未読だが関連する論文

[引用グラフから識別された関連論文 — 次に検索すべきキーワード]

```

## 書誌情報の記録ルール

- **出源情報（著者、年、DOI、タイトル）**: `sai-paper find` または `sai-paper neighbors` の戻り値のみを使う
- **分からない部分**: `[GAP: 著者名不明]` と記載（捏造しない）
- **ノート / ドキュメント**: パスのみ記録（内容は既に Library に保存されている）

## 既知の注意

- **日本語の単語検索**: `--mode word` は日本語に当たらない（FTS5 のトークナイザの制限）。日本語ノートを検索する場合は埋め込みサーバを起動した上で `--mode hybrid` または `--mode vector` を使う。
- **引用グラフの制限**: `sai-paper graph build <論文ID>` で Library の引用ネットワークを確認可能。`--library-only` フラグで Library 内の論文だけをたどる。

## 深度検索の工夫

- **クエリは 3-5 個が目安**: 多すぎると重複が増え、少なすぎると漏れがある
- **背景知識を最後に**: 主要キーワード → 下位概念 → 背景という順で拡張する
- **同じ論文の複数ヒット**: その論文が特に関連度が高い可能性。フルテキストを読むか検討
- **時系列の確認**: 古い論文と新しい論文の両方があるか確認（分野の発展を理解するため）

## 次のステップ

出典の論文をフルテキストで読んで、ユーザーの質問に対する詳細な回答を作成。
