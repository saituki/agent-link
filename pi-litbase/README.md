# pi-litbase

litbase 連携の pi 拡張。pi agent が `sai-paper` CLI を決定論的に操作するための
ツール群を提供する。slash command は登録せず、コマンドは sai-link の
block.md として配る。

## 構成

```ascii
pi-litbase/
├── index.ts                 # ツール登録
├── tools.ts                 # 全ツール定義
├── test_tools.mjs           # 引数と標準入力の組み立ての確かめ
├── sai-paper-cli.ts         # sai-paper CLI ラッパ（root 解決 + JSON 実行）
└── pi-extension-api.d.ts / node-stubs.d.ts   # 型検査用スタブ
```

確かめ:

```bash
tsc --noEmit -p pi-litbase
node pi-litbase/test_tools.mjs
```

## セットアップ

### 1. sai-paper の初期化

研究プロジェクトルートで 1 度だけ実行する。

```bash
cd <research-project-root>
sai-paper init --name <project-name>
```

`.sai/` が無ければ `sai-link init` を実行し、`.sai/paper/`（`papr.db`）を作成する。
legacy `data/` の移行と `.sai/blocks/` の操作は行わない。

### 2. 拡張の登録

sai-link が pi を起動する際、`--extension` でこの拡張を読み込む。方法は 2 つ。

- **block ごとに指定**: `.sai/blocks/*.md` の `extensions` に `pi-litbase/index.ts` の絶対パスを書く。
- **グローバルに指定**: `~/.sai/settings.json` の `extensions` に絶対パスを追加する。全 block で読み込まれる。

```json
{
  "extensions": [
    "/abs/path/to/sai-link/pi-sai-link/extension.ts",
    "/abs/path/to/sai-paper/pi-litbase/index.ts"
  ]
}
```

### 3. block の配置

ブロックは `sai-paper init` が `crates/sai-paper-cli/assets/blocks/` から研究プロジェクトの
`.sai/blocks/` へ配る。このリポジトリにはブロックを同梱しない。

## 環境変数

| 変数 | 既定 | 説明 |
|---|---|---|
| `SAI_PAPER_ROOT` | cwd から上向き探索 | 研究プロジェクトルート |
| `SAI_PAPER_BIN` | `sai-paper` | sai-paper バイナリ名または絶対パス |

## ツール一覧

| tool | 用途 |
|---|---|
| `sai_paper_resolve` | paper_id / DOI / arXiv / タイトルから論文を解決 |
| `sai_paper_recent` | Library の最近の論文一覧 |
| `sai_paper_lookup` | S2 メタデータ取得（Paper Weight 用） |
| `sai_paper_ensure_pdf` | PDF 確保（DB → arXiv → S2 OA → Europe PMC → Unpaywall） |
| `sai_paper_text` | docling 抽出の本文 Markdown |
| `sai_paper_import_pdf` | 手動 PDF 取込 |
| `sai_paper_recent_downloads` | ダウンロードフォルダの PDF 候補 |
| `sai_paper_search_db` | 複数データベースの横断検索（検索セット作成） |
| `sai_paper_set_list` / `show` / `mark` / `merge` | 検索セットの一覧・表示・判定・統合 |
| `sai_paper_find` | ハイブリッド検索（word/vector/hybrid）。sci の範囲（papers・node・subtree・each・question） |
| `sai_paper_neighbors` | 論文の近傍チャンク検索 |
| `sai_paper_index_status` | インデックス状態 |
| `sai_paper_sci_context` | 今の実行の範囲（研究方針の木から切り出した入力） |
| `sai_paper_sci_status` | 研究方針の木の全体（種の文・関門・項目・負債・次の一手） |
| `sai_paper_sci_node` | 節点 1 つの中身 |
| `sai_paper_sci_propose` | 研究方針の木への変更を提案 |

## 設計原則

- **pi agent は DB を直接触らない。** すべて `sai-paper` CLI（JSON）経由。
- **PDF は agent が書かない。** 取得・検証・保存は `sai_paper_ensure_pdf` / `sai_paper_import_pdf` の内部で完結する。
- **書誌データは捏造しない。** `sai_paper_lookup` の戻り値のみを使う。
- **ノートは論文フォルダの `note.md`、現況は `.sai/paper/state.md`。** エージェント用の重複した保存先は持たない。
