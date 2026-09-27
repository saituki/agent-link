> このテンプレートは search-external / search-internal に移りました。新しいブロックはそちらを使います（互換のため残しています）。

# 論文検索戦略ガイド

このテンプレートは、複数のデータベースから系統的に論文を検索するためのエージェント向け指示です。

## ステップ 1: 検索対象の明確化

ユーザーからの要求に基づいて、以下を整理する:
- **研究トピック/臨床質問**: 何を調べるのか
- **対象範囲**: 年号、言語、研究デザイン
- **必要なデータベース**: 下表を参考に選定
- **重複排除の基準**: DOI → arXiv ID → S2 paperId → 正規化タイトル

## ステップ 2: データベース選択の判断

| DB | 対象 | 用途 |
|---|---|---|
| PubMed | 医学・生物医学 | 査読論文、MeSH が重要 |
| bioRxiv/medRxiv | プレプリント | 査読前の最新研究、プレプリント識別必須 |
| Semantic Scholar | 全分野 | 広い範囲、引用ネットワーク |
| arXiv | CS、物理、数学 | プレプリント、方法論 |
| OpenAlex | 全分野の journal | メタデータ、journal 指標取得 |

デフォルト戦略:
- 医学系: **PubMed + bioRxiv**
- CS 系: **arXiv + S2**
- 広域探索: **PubMed + S2 + bioRxiv**

## ステップ 3: クエリ構築

### PubMed（`--raw-query` で Boolean クエリを渡す）

**PICO 分解と MeSH マッピング**:
- P (Population): `diabetes mellitus[mh]`
- I (Intervention): `metformin[nm]` または `drug therapy[sh]`
- C (Comparison): `lifestyle modification[tiab]`
- O (Outcome): `glycemic control[tiab]`

**Boolean 構築**:
```
(diabetes mellitus[mh] OR "diabetes mellitus, type 2"[mh]) AND 
(metformin[nm] OR "diabetes mellitus"/drug therapy[mh]) AND 
(randomized controlled trial[pt] OR meta-analysis[pt])
```

Field tags:
- `[tiab]` = title/abstract
- `[mh]` = MeSH term
- `[au]` = author
- `[pt]` = publication type
- `[dp]` = publication date（形式: `YYYY:YYYY`）

### bioRxiv/medRxiv

キーワード検索が基本。複数キーワードは AND で結合:
```
glioma ferroptosis cell death
```

### Semantic Scholar/arXiv

自然言語クエリが基本。同義語を複数試す:
```
glioblastoma ferroptosis immunotherapy
```

## ステップ 4: CLI での実行

```bash
sai-paper search "自然文の質問" \
  --db pubmed \
  --db biorxiv \
  --db s2 \
  --limit 100 \
  --year-from 2020 \
  --year-to 2025 \
  --raw-query '(glioma[tiab] OR glioblastoma[mh]) AND ferroptosis[tiab]' \
  --save-as <セット名> \
  --json
```

オプション説明:
- `--db`: 複数指定可。省略時は config の `[discover] providers`
- `--limit`: 返す件数（デフォルト 20）
- `--year-from/to`: 年号フィルタ
- `--raw-query`: DB 固有構文をそのまま渡す（PubMed Boolean）
- `--save-as`: 検索結果を検索セットとして保存。同じ名前なら 1 つのセットにまとめられ、重複した候補は足されません。検索のたびに別の名前を作る必要はなく、同じ名前で何度 `--save-as` しても構いません（一時的なセットを作って `set delete` で消さない）。
- `--json`: JSON で結果を返す

## ステップ 5: 結果の確認と重複除外

CLI は自動で以下の順で dedupe:
1. DOI（完全一致）
2. arXiv ID
3. Semantic Scholar paperId
4. 正規化タイトル（`crates/sai-paper-core/src/citation/matcher.rs` の実装）

## 既知のベストプラクティス

- **PubMed の MeSH**: `meshb.nlm.nih.gov` で事前検証（毎年更新される）
- **広い検索ほど早く**: 最初は `--limit 50` から開始して増やす
- **文献レビューのコツ**: 質問が広いなら、最初は論文サンプル 10-20 件を読んで絞り込みを決定

## 既知の制限

- PubMed の結果は最大 10000 件
- bioRxiv の日付検索は API が日付範囲による絞り込みのみ対応
- arXiv は 3 req/sec で polite に使用
