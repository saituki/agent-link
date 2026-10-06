# 用語集

sai の文書とコードで使う言葉の意味をまとめる。画面の文字は日本語にし、`data-tab`・住所・API の名前などの識別子は英語のまま使う。
研究方針の木の名詞（Q・O・F・E・SEED・Sprout・PROPOSAL・TRIAGE・Action・Next Action・DEBT・CARD・SCOPE など）は [`../sai-sci/README.md`](../sai-sci/README.md) の「1. sai-sci とは」の表にある。

## 1. アプリと部品

| 用語 | 意味 |
|---|---|
| **sai** | アプリ全体の名前 |
| **sai-link** | 基盤。coding agent の実行・カード・常駐・文書を扱う。CLI は `sai-link`、クレートは `sai-link-*` |
| **sai-paper** | 研究の基盤（論文）。検索・取得・翻訳・音声化・Library・検索セットを扱う。CLI は `sai-paper`、クレートは `sai-paper-*` |
| **sai-sci** | 研究の仕組み。sai-link と sai-paper の CLI を使うアプリ。CLI は `sai-sci`、クレートは `sai-sci-*`、md の部品のフォルダは `sai-sci/` |
| **sai-web** | Web の窓口。フォルダは `apps/web`、CLI は `sai-web`（`apps/web/bin/sai-web`） |
| **apps/radio** | 論文の解説音声を作って配る裏方サーバー（FastAPI・8201 番）。画面は持たない |
| **apps/mlflow-server** | 実験の記録（MLflow）を溜めるサーバの compose（postgres・minio・mlflow・caddy）。Docker で動かす |
| **waggle** | 文書に waggle トークンを付けて受け渡す仕組み。`waggle/` に vendor したクレート群がある |
| **LabAgent** | sai-sci が配る研究用の block・template |
| **research-tree（研究方針の木）** | Q / O / F / E の Node でできた木。データは `papr.db` の `rt_*` 表、画面は Research タブ |
| **Experiments（実験）** | MLflow の記録。サーバは `apps/mlflow-server`、画面は Research タブの Experiments |

## 2. 実行と文書

| 用語 | 意味 |
|---|---|
| **run（実行）** | エージェントが 1 回動いた記録。`run_id` で呼ぶ。`sai-engine.db` に残り、run フォルダ（`.sai/outputs/…/<run_id>/`）に成果物が置かれる |
| **card（カード）** | まだ実行していない run。todo にある `task.md`（frontmatter と依頼文）。`sai-link card create` で作り、`card run` で実行する |
| **doc（文書）** | waggle トークンの付いた文書。`sai-link doc write` で run フォルダに保存し、`doc read`・`doc list` で読む |
| **waggle トークン** | 文書を指す短い名前（例 `AXANSMpo`）。文書を渡すときはトークンを使い、ファイルのパスは渡さない。`task_text` などに `waggle:<トークン>` と書くと、実行時にその位置へ文書の本文が埋め込まれる（1 段だけ）。参照は `waggle:<トークン>:path:.sai/outputs/*/<run_id>/<ファイル名>` の形で書く（正はトークン） |
| **block** | エージェントの役の定義（`.sai/blocks/<id>.md`）。runner・model・template などを frontmatter に書く |
| **template** | block の本文に差し込む共通の決まり（`.sai/template/<名前>.md`） |
| **runner** | カードを動かす実行者。`pi`・`cc`・`gl`・`user`（`user` は利用者の札） |
| **`executer`** | カードを**起こす人**。`human`（人が起こす。既定）か `agent`（professor など、エージェントが自分の子として起こす）。`runner` とは別の項目 |
| **常駐（daemon）** | プロジェクトごとの背景のプロセス。予約したカードの実行・ジョブ待ちのカードの実行・状態の見張りをする。`sai-link daemon start\|stop\|status` |
| **`job_status`** | `.sai/settings.json` の項目。実験などのジョブの状態を確かめる命令（文字の配列。`{job}` がジョブの ID に置き換わる）。`sai-sci init` が書き、sai-link の常駐と `card wait` が読む。出力の `state` が `done`・`failed`・`cancelled`・`lost` になったらジョブは終わり |
| **review** | run の状態のひとつ。完了して利用者の確認を待っている。`run advance` で done → Workflow へ進める |
| **Workflow** | Projects の画面のひとつ。run を札と線で描いた図と、Kanban の 4 レーン（todo・running・review・done） |
| **`SAI_HOME`** | 利用者全体の置き場（`~/.sai`）。登録簿・共通の block / template / settings・`paper.env`・`web/` が入る。プロジェクトの中の `<root>/.sai/` とは別 |
| **登録簿（registry）** | `$SAI_HOME/registry.json`。`sai-link init` が書き、Web のプロジェクト一覧が読む |

## 3. CLI

| 用語 | 意味 |
|---|---|
| **`--json`** | 成功したとき標準出力に JSON を 1 つだけ書く指定 |
| **`schema --json`** | 命令の一覧（`bin`・`version`・`commands[]`）を出す命令。Web の「コマンド」画面が読む |
| **終了コード** | 0 成功・2 入力の誤り・3 見つからない・4 状態の衝突・1 その他の失敗 |
| **`sai-progress`** | 時間のかかる命令が標準エラーに出す進み具合の行（`sai-progress step=<数>`・`sai-progress msg=<文>`） |
| **`SAI_PROJECT_ROOT`** | 対象のプロジェクトの root。空なら、今のフォルダから上へたどって `.sai/` を持つ最初のフォルダ |
| **`SAI_ORIGIN`** | 記録の出どころ（無ければ `cli`。Web が呼ぶときは `phone`） |

## 4. Web の画面

| 用語 | 意味 |
|---|---|
| **Home** | 開いたとき最初に出る画面。いま決めること（質問・承認・確認待ち・要対応）を 1 枚に出す |
| **Projects** | プロジェクトの一覧と、Dashboard・Workflow・List・設定 |
| **Chats** | coding agent の会話を読み、指示を送り、質問と承認に答える |
| **Papers** | Dashboard・Library・Import・Radio・検索セット |
| **Research** | 研究方針の木・仕分け・読む札・Experiments |
| **Usage** | LLM の料金と制限 |
| **Dashboard** | 期間ごとの実行数・成功率・トークン量・時間の数字とグラフ（Projects と Papers にそれぞれある） |
| **List** | Projects の画面のひとつ。run のツリーから 1 つ選び、Log・Stats・History・task・review・message・waggle で見る |
| **設定** | Projects の画面のひとつ。block・template・settings を読み書きする |
| **コマンド** | Projects の画面のひとつ（`#/project/<名前>/commands`）。3 つの CLI の `schema --json` から命令を出し、入力欄から実行する |
| **セッション検索** | Projects の画面のひとつ（`#/project/<名前>/sessions`）。会話の履歴を検索する（`sai-link session search`） |
| **Library** | Papers の画面のひとつ。取り込んだ論文の一覧と階層 |
| **Import** | Papers の画面のひとつ。BibTeX の読み込み |
| **検索セット** | 検索の結果を候補として並べ、keep / drop で選別する入れ物。`sai-paper set …` |
| **Radio** | 論文の解説音声を聴く画面。声や文字で質問もできる |
| **読む札** | runner が `user` のカード。利用者が 1 本の論文を読んで F を付ける（`#/research/read/<run_id>`） |
| **仕分け（triage）** | エージェントが出した提案を採用・却下・取り消しする画面（`#/research/triage`） |
