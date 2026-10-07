# 外部の論文 DB の検索（search-external）

外部の論文データベースから系統的に候補を集める手順です。
Library（sai-paper の中）の検索は `search-internal` を使います。

## ステップ 1: データベースを選ぶ

分野に合う DB を選びます。どの観点を重視するかは、ブロックに入っている `domain` の指示に従ってください。

| DB | 対象 | 用途 |
|---|---|---|
| PubMed | 医学・生物医学 | 査読論文。MeSH が重要 |
| bioRxiv / medRxiv | 生命科学のプレプリント | 査読前の最新研究。プレプリントの識別が必須 |
| Semantic Scholar | 全分野 | 広い範囲。引用ネットワーク |
| arXiv | CS・物理・数学 | プレプリント。方法論 |
| OpenAlex | 全分野の journal | メタデータ。ジャーナル指標 |

選び方の目安:
- 医学・生物医学: PubMed + bioRxiv / medRxiv
- 工学・情報（CS）: arXiv + Semantic Scholar
- 広く探す: PubMed + Semantic Scholar + bioRxiv

## ステップ 2: 英語の検索語を作る

日本語の問いは英語の検索語に直し、3〜5 個の小さな問いに分けます。各検索語について次を用意します。

- **同義語**: 同じ意味の別の言い方（例 `epilepsy` / `seizure disorder`）
- **上位語**: 一段広い言葉（例 `deep learning` に対する `machine learning`）
- **除外語**: 混ざってほしくない話題（例 `-review` など、DB の書き方に合わせて）

DB ごとの書き方:
- **PubMed**（`--raw-query` で Boolean を渡す）: MeSH と field tags を使います。`[tiab]` = タイトル・抄録、`[mh]` = MeSH、`[au]` = 著者、`[pt]` = 出版種別、`[dp]` = 出版日（`YYYY:YYYY`）。
  例: `(glioma[tiab] OR glioblastoma[mh]) AND ferroptosis[tiab]`
- **bioRxiv / medRxiv**: キーワードを AND でつなぐ（例 `glioma ferroptosis cell death`）。
- **Semantic Scholar / arXiv**: 自然言語のクエリ。同義語を複数試す。

## ステップ 3: 検索する（結果は search_history に自動で残る）

```bash
sai-paper search "<自然文の質問>" \
  --db <DB1> --db <DB2> \
  --limit 100 \
  --year-from <開始年> \
  --year-to <終了年> \
  --raw-query '<DB 固有のクエリ>' \
  --json
```

引数の意味:
- `--db`: 使う DB（`s2` / `arxiv` / `pubmed` / `biorxiv` / `openalex`）。複数指定できます。省略すると設定の既定を使います。
- `--limit`: 返す件数（既定 10）。
- `--year-from` / `--year-to`: 年号の範囲（指定するときだけ）。
- `--raw-query`: DB 固有のクエリをそのまま渡します（PubMed の Boolean など）。
- `--json`: JSON で返します。

検索の結果は search_history に 1 回として残ります。あとで見るときは `sai-paper search-history <log_id>` を使います。

## ステップ 4: 重複を除く

`sai-paper search` は結果を自動で重複除去します。優先する順は DOI → arXiv ID → Semantic Scholar paperId → 正規化タイトルです。

## ステップ 5: 引用の網を 1 段だけ広げる

集まった候補のうち、関係が最も深い 2〜3 本について、引用関係を 1 段だけたどり、Library に無いものを候補として集めます。

```bash
sai-paper expand <論文ID>... --json
```

引数の意味:
- `<論文ID>`: Library に取り込み済みの論文の `papers.id`（`sai-paper library` で確かめる）。2〜3 本まで指定します。
- `--direction`: たどる向き（`both` / `refs` / `cited`。既定 `both`）。
- `--limit`: 1 本・1 向きあたりに取る最大件数（既定 20）。

expand の結果も search_history に 1 回として残ります。

`--depth` を指定する必要はありません（このコマンドは常に 1 段だけたどります）。グラフの中身を自分で見たいときだけ `sai-paper graph show <論文ID>` を使います。

## ステップ 6: 取り込みは利用者が行う

このテンプレートの役目は候補を集めるところまでです。

PDF のダウンロードと Library への取り込みは、ユーザーが sai-paper の TUI で行います。エージェントは検索して、取り込みを勧める候補を報告するところまでにしてください。

## 文献完全性

- 報告するメタデータ（タイトル・著者・年・DOI など）は `sai-paper search` の返り値だけを使います。捏造しないでください。
- 取得できなかった項目は明示します（例 `DOI [利用不可]`）。
