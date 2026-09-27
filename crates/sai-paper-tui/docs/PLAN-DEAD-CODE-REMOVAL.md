# 実装計画: sai-paper-tui デッドコード削除（機能変更なし・削除のみ）

- 作成日: 2026-09-01
- 対象: `crates/sai-paper-tui`
- 目的: コンパイラ / clippy が「未使用」と確定させたコード、および到達不能になったコードのみを削除する。機能の追加・修正は一切行わない。

## 前提

- **原則**： 機能の追加・修正は一切行わない。動作しているコード・UI 表示は一切変えない。
- **検証**： 各フェーズ後に `cargo build -p sai-paper-tui && cargo clippy -p sai-paper-tui && cargo test -p sai-paper-tui` でビルド・テストが変わらないことを確認する。
- **今回やらないこと**： `Page` 14→8 項目化、キーバインド（`a`/`B`/`g`）削除、NoteEdit 削除、設定項目削除 — これらは「仕様未反映の機能削除」であり、現状動作を変えるため本計画の対象外とする。

## Phase 1: 完全デッドファイルの削除

| 対象 | 根拠 |
|---|---|
| `src/graph_view.rs`（79 行）削除 | `mod` 宣言なし・参照ゼロ |

検証: ビルド + 全テスト。

## Phase 2: `ui.rs` の未使用描画関数削除

削除対象（すべて "never used" 確定）:

- `render_history`
- `render_statistics`
- `render_metric`

注意点:

- 削除前に `grep` で呼び出し箇所ゼロを再確認（clippy の指摘に依存しない）。
- `render_metric` は `render_statistics` 専用ヘルパの可能性 → 依存順に注意（statistics を先に消す）。
- `render_today_research` は呼び出しの有無を再確認し、未使用なら削除、使用中なら残す。

## Phase 3: `editor.rs` の未使用関数群削除

editor.rs は「設定エディタ（config editor）」系が未使用。**使用中の関数（`config_editor_line_start/end/wrap_rows`、`cursor_visual_position` 等）は残す。**

削除対象:

- `EditorInsertResult` enum
- `apply_editor_insert_key`
- `handle_config_editor_key` / `handle_config_editor_command` / `execute_config_editor_command`
- `handle_config_editor_normal_key` / `handle_config_editor_insert_key`
- `move_config_editor_logical_line` / `move_config_editor_page`
- `invalidate_document` / `jump_to_page`
- `PROJECT_EDITOR_PENDING_SEQUENCE_TIMEOUT` 定数
- `state.rs` 側: `config_editor_viewport_height` フィールド（never read）
- `main.rs` 側: 上記に付随する未使用 import の整理（`main.rs:37` の use リストから実態のない名前を除去）

手順: editor.rs 内の依存グラフを確認（削除関数同士が呼び合う場合はまとめて削除）→ 残存関数が壊れないことをビルドで確認。

## Phase 4: `main.rs` の未使用関数・残骸削除

- `index_library`（起動走査廃止で呼び出し消失）
- `reap_finished_citation_workers`
- `should_open_generated_pdf`
- `move_config_editor_page`（Phase 3 と重複する場合は main.rs 側）
- `database_tags_preview`
- `is_control_character_shortcut`
- `activity_kind` / `download_status_label` / `format_duration` / `format_decimal`
- `main.rs:4045` 空の `if app.page == Page::Collections {}` ブロック削除
- `state.rs`: `collection_roots` フィールド削除（`main.rs:188` での代入元も追って整理。ただし `Locations` 構造体側は core 由来のため、TUI 内の未使用 field のみ削除）

## Phase 5: 未使用変数・import・clippy 警告の清掃（削除可能なもののみ）

- 未使用変数（`sender`, `runtime`, `download_dir`, `database`, `bibtex`, `bib_path`, `last_pdf_page_cached`, `key`, `title`, `t`）:
  - **削除で済むものは削除**、関数シグネチャの引数として必要なものは `_` への改名のみ（挙動変更なし）。
- 未使用 import の削除:
  - `Deserialize`/`Serialize`、`CitationSource`/`CompletionSource`/`validate_collection_name`、`AtomicBool`/`BufReader`/`BufWriter`/`Component`/`Ordering`/`TryRecvError`、`Write`、`project_editor_line_at`、`Color`、`BufRead` のうち未使用のもの。
- `unreachable pattern` 2 件: 削除対象 variant への match アームか確認し、到達不能アームのみ除去（match 網羅性は保持）。
- **対象外（修正を伴うため）**: `manual_map`, `redundant_closure`, `useless_conversion` 等の「書き換えを要する」警告。削除のみで解消するものに限定。

## Phase 6: 最終検証

1. `cargo build --workspace` — エラーゼロ
2. `cargo clippy -p sai-paper-tui` — 「never used / unused」系警告ゼロを確認
3. `cargo test -p sai-paper-tui` — 既存テスト全件パス（テストは変更しない）
4. 起動して目視確認: 全 14 ページの描画・キー操作に変化がないこと（`render_sidebar` が `Page::ALL` 依存のため `Page` 自体は触らないことで担保）

## 進め方

- 各 Phase を順に実行し、毎回ビルド+テストを通してから次へ。
- 削除行数の見積り: 約 1,500〜2,000 行（graph_view 79 + editor 約 400 + main/ui/state 約 800〜1,000）。
- リスク: Phase 3/4 で「使っているつもりのない関数がテストから参照される」可能性 → 削除前に `rg` で全参照確認を必須とする。テストから参照される場合は当該テスト関数ごと未使用であれば削除、使用中であれば対象を除外する。

---

## 実行結果（2026-09-01 実施済み）

| Phase | 結果 |
|---|---|
| 1 | `graph_view.rs` 削除（79 行） |
| 2 | `ui.rs`: `render_history` / `render_statistics` / `render_metric` / `activity_kind` / `format_duration` / `format_decimal` / `download_status_label` 削除 |
| 3 | `editor.rs`: `byte_index_for_char_column` / `cursor_from_visual_position` / `project_editor_line_at` 削除。`main.rs`: config editor デッドチェーン（`handle_config_editor_key` 以下 8 関数・`EditorInsertResult`・`apply_editor_insert_key`・`move_config_editor_page` / `move_config_editor_vertical`・`PROJECT_EDITOR_PENDING_SEQUENCE_TIMEOUT`）と専用テスト 3 件を削除。`state.rs`: `config_editor_viewport_height` / `config_editor_wrap_width` フィールド削除 |
| 4 | `main.rs`: `index_library` / `reap_finished_citation_workers` / `should_open_generated_pdf`（＋専用テスト）/ `database_tags_preview` / `is_control_character_shortcut` / 空 `if Page::Collections` ブロック / 到達不能 match アーム（Dashboard/Discover）/ UiAction デッド variant（`AddToQueue`・`RetryDownload`・`OpenCollection`・`OpenDownload`・`InsertProjectCitation`）とそのディスパッチアーム・`open_download` / `apply_retry_download` 削除。`pdf_viewer.rs`: `jump_to_page` 削除（`invalidate_document` はテストから参照されるため保持）。`citation.rs`: `fetch_and_insert_project_citation` 削除 |
| 5 | 未使用 import 一括削除（main / ui / citation / editor / state / テストモジュール）、未使用変数は削除または `_` 改名、孤立 doc comment + `#[must_use]` 削除 |
| 6 | `cargo build --workspace` エラーゼロ / `cargo test -p sai-paper-tui` 97 passed / 0 failed / clippy 61→11 警告（残りは書き換えを要する修正系のため対象外） |

対象外（仕様上の機能削除は別タスク）: `Page` 14→8 項目化、キーバインド `a`/`B`/`g`、NoteEdit・`sai-paper-core` 側のデッドコード。
