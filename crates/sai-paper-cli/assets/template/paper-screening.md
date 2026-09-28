# 論文スクリーニングガイド

既存の検索セットから、抄録を読んで絞り込むためのエージェント向け指示です。

## ステップ 1: 検索セットの確認

```bash
sai-moon-paper set show <セット名> [--decision pending|keep|drop|downloaded]
```

ユーザーが示した絞り込み条件を理解する（例: 「RCT のみ」「2023 年以降」「in vivo 研究」）

## ステップ 2: 抄録の読み込みと判定

各論文の抄録と基本情報（タイトル、著者、年、ジャーナル）を確認:
- **判定基準**: ユーザーの条件と照合
- **不確実な場合**: 理由を明記した上で `pending` で保留

判定ルール:
- `keep`: 条件に合致、次段階（フルテキスト読む、データ抽出など）へ進める
- `drop`: 対象外の理由が明白（例: 別疾患、別の研究デザイン）
- `pending`: 判断保留（抄録の情報不足、曖昧な点がある）

## ステップ 3: 判定の記録

```bash
sai-moon-paper set mark <セット名> \
  --position <n> \
  --decision keep|drop \
  --reason "スクリーニング理由の簡潔な説明"
```

複数件を一括マーク:
```bash
sai-moon-paper set mark <セット名> \
  --position 1 --position 3 --position 5 \
  --decision drop \
  --reason "プレプリント版（著者不明）"
```

## ステップ 4: 進捗の確認

```bash
sai-moon-paper set show <セット名> --decision pending
```

`pending` が 0 になれば、スクリーニング完了。

## 記録のポイント

`--reason` には以下の情報を記載:
- **除外理由**: 「言語が日本語のみ」「in vitro のみ」など
- **採択理由**: 「RCT で対象患者が合致」「最新の手法」など
- 簡潔に（50 字以内推奨）

## 既知の注意

- **プレプリント識別**: source に `biorxiv` または `medrxiv` が含まれるか確認
- **査読状況**: タイトルに「preprint」「in press」がないか確認
- **言語**: 英語以外は言語名を明記（例：「中国語のみ」）

## セット操作

### セットの統合（複数の検索結果を合わせる）

```bash
sai-moon-paper set merge <セットA> <セットB> --as <統合後の名前>
```

### セットの削除

```bash
sai-moon-paper set delete <セット名>
```
