# 仕様書: sai-paper-tui 再編

本仕様は `sai-paper-tui` の論文管理モデルを再定義する。

- 対象 App: `sai-paper-tui`（`sai-paper-core` / `sai-paper-cli` に影響）
- 状態: 仕様確定・未実装
- 作成日: 2026-09-01

変更内容は種別ごとに整理する:

- **A. 削除のみを要求するもの**（§4）
- **B. 削除と統合・UI 軽微修正を含むもの**（§3 / §5 / §6）
- **C. 処理機能の追加**（§5）

---

## 1. 背景と目的

### 1.1 現状の問題

| 問題 | 現状の動作 | 影響 |
|---|---|---|
| Groups ↔ フォルダ対応 | サブフォルダ = コレクション。Finder での移動を scan で追跡するため、起動時に PDF 全走査 + ハッシュ再計算が発生 | 論文数増加（500〜1000 本）で起動が数秒〜数十秒 |
| Downloads ページ | download_dir の実ファイルを列挙し、ハッシュで DB 照合。残りカス削除（remove_file）も行う | 処理が重い。`i` の候補ディレクトリと設定 download_path の不一致という不整合もある |
| 分類はフォルダ構造に縛られる | 1 論文は 1 フォルダにしか置けない | 柔軟な整理ができない |
| content_hash の実質 | App 経由で DL/import すればメタデータの論文 ID で一意性は担保できる | 起動時の再計算が純粋な無駄 |
| ページ過多 | 14 ページのうち Downloads / Bookmarks / Reading Queue / Projects / History / Statistics は使用頻度・実益が薄い | UI・コードの保守負担 |

### 1.2 目標

- 実ファイルの操作はすべて App 経由とする。App 外（Finder）での移動・複製は前提としない
- PDF は一律 `sai-paper/papers/*.pdf`（平置き）に格納する。サブフォルダを作らない
- 分類は **タグ（複数付与可）** で行う
- 起動時のハッシュ計算・全走査を廃止し、起動を常に 1 秒未満に保つ

---

## 2. 最終ページ構成

| 変更前（14 項目） | 変更後（8 項目） |
|---|---|
| Dashboard | Dashboard — **再設計**（§3.2: カード 5 枚 + RA 全件リスト） |
| Discover | Discover — 維持 |
| Library | Library — 維持（タグフィルタ対応） |
| Collections | **削除** → `Tags`（新設） |
| Authors | Authors — 維持 |
| Downloads | **削除** |
| Bookmarks | **削除** |
| Notes | Notes — **仕様変更**（herdr + nvim、§3.4） |
| Reading Queue | **削除** |
| Projects | **削除** |
| History | **削除** → RA に統合 |
| Statistics | **削除**（統合なし・全廃） |
| Settings | Settings — 維持（§4 の設定項目削除を除く） |
| Credits | Credits — 維持 |

---

## 3. 削除と統合・UI 変更

### 3.1 タグ管理への移行（Groups → Tags）

**削除**: Groups（フォルダベースコレクション）、サブフォルダ = コレクション対応

**新設**: タグ管理（DB のみ・フォルダ非依存）

```sql
CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE paper_tags (
    paper_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    tag_id   INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (paper_id, tag_id)
);
```

- タグ名は大文字小文字を保持（表示用）。照合時は case-insensitive
- 論文 1 件に **複数** タグを付与できる
- サイドバー: `Tags`（タグ一覧、DB 由来・使用頻度順）。タグ選択 → 該当タグを持つ論文のみを Library 一覧に表示（単一タグ選択。複合フィルタは `/` の検索クエリで `tag:xxx` をサポート）
- キーバインド:
  - `t`: 選択中論文にタグを付与/解除するモーダル。既存タグから選択 + 新規タグ入力（複数可）。既付タグはトグルで外せる
  - `g`: 削除（`g` 単独は「リスト先頭」の既存挙動のみ残す）
- 既存 collections の移行: 各コレクション名を同名タグへ変換し、所属論文に付与する
- 既存サブフォルダ（`papers/test/` 等）の PDF は、移行時の初回起動で一度だけ `papers/*.pdf` へ平置きに統合する（移動は App が実行し、DB パスを同時更新）

### 3.2 Dashboard 再設計

**削除**: `Today in Research` 見出し / QUEUE・DOWNLOADED・READ・STREAK カード / STORAGE ペイン / **Statistics ページ（統合なし・全廃）**

**変更後のレイアウト**:

```
┌ PAPER ┬ TAG ┬ NOTE ┬ PDF ┬ DB ┐
├────────┴────┴──────┴─────┴────┤
│ RECENT ACTIVITY（全件・スクロール）│
│ date │ action │ description   │
└───────────────────────────────┘
```

| カード | 内容 |
|---|---|
| `PAPER` | 登録論文数 |
| `TAG` | タグ数 |
| `NOTE` | ノート数（notes ディレクトリの .md 数、または notes index） |
| `PDF` | PDF 合計容量（GiB） |
| `DB` | papr.db 容量（GiB） |

### 3.3 History → Recent Activity (RA) 統合

Dashboard の `RECENT ACTIVITY`（RA）を **activity_log 全件を表示するスクロール可能なリスト**に変更する。History ページは削除する。

- 各行は **date / action / description** の 3 項目:
  - date: `YYYY-MM-DD hh:mm:ss`
  - action: `download` / `discover` / `notes`
  - description: action に応じて `download` → 論文名、`discover` → 検索クエリ、`notes` → 論文名
- activity_log への記録種別は上記 3 種に整理する（既存の opened / searched / bookmarked 等は対応付け、対応しない旧種別は移行時に破棄）
- j/k でスクロール、`enter` で該当論文を開く（paper_id から遷移）

### 3.4 Notes の仕様変更（herdr pane + nvim）

ノート本文は DB 保存（paper_note）→ **Markdown ファイル**に変更。編集は **herdr pane で nvim** を起動（`s` の citation sidecar 起動と同一パターン）。

- ノート保存先: `<project>/notes/<正規化論文 ID>.md`
  - ファイルがなければ、論文タイトル・著者・リンク（arXiv / DOI）を含む front matter を自動生成してから開く
- 起動フロー（`s` と同一の手順）:
  1. `herdr pane split --direction right --no-focus` で新ペイン作成（失敗時はフォールバックで直接 spawn）
  2. `herdr pane run <pane_id> "nvim <note path>"` で編集開始
  3. TUI 側はファイルを監視せず、nvim 終了時に自然に TUI へ復帰する
- DB の note 列は廃止。既存ノート本文はマイグレーション時に .md へ書き出して移行する
- NoteEdit モード・独自エディタ（editor.rs）・autosave ロジックを削除
- Notes ページは維持: 「ノートを持つ論文の一覧」を notes ディレクトリの走査（または notes index テーブル）で表示。`n` で nvim を開く

### 3.5 Reading Queue / Bookmarks の削除

- 両ページ・サイドバー項目・キーバインド（`a` / `B`）を削除
- 「後で読む」は **タグ** で代替（マイグレーション時に既存 reading_queue の内容を `reading-queue` タグへ変換し、queue_order 順に paper_tags へ登録）
- 「ページ単位の栞」は廃止
- DB: `reading_queue` / `bookmarks` テーブルは data 移行後に DROP（または読み書き停止して残置）
- `reading_status`（unread / reading / read）は `papers` テーブルの列として**維持**（Library 一覧のステータスバッジで使用）

### 3.6 Downloads の削除

- Downloads ページ・サイドバー項目を削除
- DL 済み論文の PDF は Library 一覧から `enter`（PAPER 表示）→ `p`（外部ビューア）でアクセス
- Discover から `d` で DL した論文は即座に Library に登録され、PDF パスが DB に紐付く
- DL 完了メッセージはトーストで通知（Downloads 一覧依存の表示は廃止）
- `i` 実行時、対象論文に既に PDF パスが紐付いている場合は候補表示の前に確認モーダル（上書き or キャンセル）

---

## 4. 削除される機能の一覧

| 分類 | 削除対象 |
|---|---|
| ページ | Downloads / Bookmarks / Reading Queue / Projects / History / Statistics / Collections |
| 仕組み | Groups↔フォルダ対応 / 起動時ハッシュ計算 / 残りカス削除 / バックグラウンド Library 全走査 / STORAGE ペイン（カードへ統合） |
| 処理 | Typst ビルドワーカー / プロジェクトファイルウォッチャー / プロジェクト内テキストエディタ / NoteEdit モード（独自エディタ・autosave） |
| DB | collections 関連（読み書き廃止→移行後 DROP）/ reading_queue / bookmarks / papers.content_hash（新規書込停止・列残置）/ papers の note 列 |
| CLI | index コマンド / プロジェクト関連サブコマンド |
| 設定 | 既定プロジェクトコンパイラ / projects ディレクトリ |
| キー | `a`（キュー追加）/ `B`（栞）/ `g`（コレクション割当） |

---

## 4'. 処理機能の追加

### `i`（手動取り込み）の維持と変更

App 経由でのダウンロードが不可能なケース（ブラウザでしか取得できない PDF など）のため、`i` は**維持する**。

フロー:

1. `i` を入力 → macOS 標準のダウンロードディレクトリ（既存の解決順: XDG → `~/Downloads` → `~/ダウンロード`）から **直近 5 分以内に追加された PDF** を候補一覧表示（現行仕様どおり）
2. 対象論文は `i` を押した時点で選択済み（PAPER 画面 / Library 一覧の選択行）。**論文 ID は確定している**
3. 選択した PDF を `papers/*.pdf` へ mv（正規名にリネーム、衝突回避）
4. DB の該当論文に PDF パスを紐付け
5. 先頭 5 バイトの `%PDF-` マジック検証のみ実施（現行維持）

**ハッシュ計算は廃止する。** 同一 PDF の二重登録リスクは「対象論文に既に PDF パスがある場合は確認モーダルを表示」ことで回避する（§3.6）。

### `n`（ノート編集）の新設フロー

- `n` → ノート .md がなければ front matter（論文タイトル・著者・arXiv/DOI リンク）を自動生成 → herdr pane で nvim を開く（§3.4）
- Notes ページは「ノートを持つ論文の一覧」として維持する

### 重複 DL 判定の新設

- Discover からの `d` 実行時、DL 開始前に「論文 ID（S2 paperId / DOI / arXiv ID）の DB 検索」を行い、既に実体があればトースト「Already in library」でスキップする（現行はハッシュ照合に依存）

### CLI 整合チェックコマンド（新設）

- `sai-paper index` の代替。DB 上の pdf_path 一覧と実ファイルの存在を照合し、レポートのみ出力（自動削除・自動再配置はしない）

---

## 5. ファイル配置と実体管理

### 5.1 配置ルール

- すべての PDF は **`<project>/papers/*.pdf` に平置き**する。サブディレクトリは使用しない
- ファイル名: 取得時のメタデータ由来の正規名（現在の自動 DL と同一規則）。衝突時は ` (2)` 連番
- PDF の移動・リネーム・削除は App の操作のみで行う。ユーザーが Finder で動かした場合の動作は保証しない（DB のパスと実体が乖離した場合、該当論文は「実体なし」状態になる。詳細 §6）
- ノートは `<project>/notes/<正規化論文 ID>.md`（§3.4）
- ノートの実体は App 経由（herdr pane + nvim）で編集するため、scan・ハッシュともに不要

### 5.2 起動処理の変更

- 起動時の残りカス削除・ハッシュ照合を**廃止** → 起動時間からハッシュ計算が消える
- `pdf_storage_stats`（PDF 容量集計の全走査）は**バックグラウンド化**し、初回描画は前回値キャッシュ。更新結果は数秒後に反映
- バックグラウンド処理は「DB の pdf_path が指す実体の存在確認」のみに縮小

### 5.3 実体なし PDF の扱い

App 経由以外でファイルが失われた（消された・移された）場合の挙動:

- 起動時および Library 一覧表示時に、軽量な存在確認（パス単位の stat のみ。走査ではない）を実施
- 実体がなければ Library 一覧で **「missing」バッジ**を表示する
- `r`（re-link）で再取得を提案: arXiv ID があれば自動 DL、なければ S2 検索 URL を開く
- CLI 整合チェックコマンドでレポートを出力（自動削除・自動再配置はしない）

---

## 6. CLI の変更

- `sai-paper index`（全 PDF 走査 + ハッシュ + ingest）を廃止 → 整合チェックコマンドに置換（§5.3）
- `get-pdf` は論文 ID（S2 paperId / DOI / arXiv ID）必須とし、DL 完了時に DB へパス登録まで行う

---

## 7. データモデルの変更一覧

| 種別 | 対象 | 操作 |
|---|---|---|
| 新設 | `tags` / `paper_tags` | マイグレーション `0018_tags.sql` |
| 移行 | collections → 同名タグ / reading_queue → `reading-queue` タグ / note 列 → `<project>/notes/*.md` | マイグレーション時に data 変換 |
| 廃止（書込停止→DROP） | collections / collection_papers / reading_queue / bookmarks / projects 関連 | UI・API から撤退後 DROP |
| 廃止（書込停止・残置） | papers.content_hash | 読み取りもしない |
| 廃止（列削除） | papers の note 列 | .md 移行後 |
| 維持 | `reading_status`（Library バッジ用）/ activity_log（種別整理） | 変更なし |

## 8. 移行手順

1. マイグレーション `0018_tags.sql`: tags テーブル作成、collections → tags 変換、reading_queue → `reading-queue` タグ変換、note 列 → .md 書き出し、collection 配下の PDF を `papers/*.pdf` へ移動、activity_log 種別の整理
2. `sai-paper-core`: tags API（付与・解除・一覧・タグ別論文取得）、collections / queue / bookmark / project / Typst 関連 API の削除
3. `sai-paper-tui`: サイドバー再構成（8 項目）/ Dashboard 再設計 / Notes → herdr + nvim / Downloads・キーバインド廃止 / 起動処理簡素化
4. `sai-paper-cli`: index コマンド廃止、整合チェックコマンド追加、プロジェクト関連サブコマンド削除

## 9. 非機能要件

- 起動〜初回描画: **1 秒未満**（1000 本維持時。ハッシュ計算ゼロ、走査ゼロ）
- 起動時のファイルシステムアクセス: DB の pdf_path 単位の stat のみ
- タグ付与は同期的に DB 反映（UI は即反映、非同期不要）

## 10. スコープ外（将来課題）

- PDF 実体の重複チェック（同一内容・別論文レコードの検出）。必要になった時点で「サイズ+mtime+ハッシュ」の遅延計算として追加する
- タグの階層化・色付け
- タグ一括操作（複数論文への一括付与）
- ページ単位の栞（Bookmarks の再検討）
