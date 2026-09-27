# sai-papr TUI — UI デザインと実装方法

このドキュメントは、別のエージェント / 別プロジェクトが `sai-papr` の TUI
（`crates/sai-papr`）の UI を読み解く・移植する・再現するための設計メモです。
コード本体は RUST + `ratatui` 0.29 + `crossterm` 0.28 で実装されています。

---

## 0. このドキュメントの使い方

- **全体像・アーキテクチャ** → セクション 1〜4
- **見た目（デザイン）の再現** → セクション 5（配色・各エリア）
- **実装パターン（コピペ元）** → セクション 6〜9
- **同じ UI を別 TUI に移植するコツ** → セクション 10

対象ソース（全て `crates/sai-papr/src/`）:
| ファイル | 役割 |
|---|---|
| `main.rs` | エントリポイント・イベントループ・キー→`UiAction`→処理の状態遷移 |
| `ui.rs` | **描画のほぼ全て**（レイアウト・各エリア・Markdown・独自Widget） |
| `state.rs` | `App` 構造体・`AppMode`・`Page` 等の全状態定義 |
| `theme.rs` | 配色トークン（ビルトイン19種 + ユーザーTOML） |
| `pdf_viewer.rs` | PDFビューア（自前Kittyエンコード） |
| `settings_modal.rs` | 設定モーダル（4タブ） |
| `graph_view.rs` / `editor.rs` / `citation.rs` / `terminal.rs` / `terminal_input.rs` | 関連モジュール |

---

## 1. 全体アーキテクチャ: 「単一状態 + 描画関数群」方式

sai-papr は、**状態を一つの `App` 構造体に集約し、フレームごとに純粋な描画関数を呼ぶ**方式です。
`impl Widget` するコンポーネントは極めて少なく、**大半は「`Frame` を受け取って標準ウィジェットを組み立てる関数」** です。

```
main.rs (イベントループ)
  └─ terminal.draw(|frame| ui::render(frame, app, theme))   // 毎フレーム
       └─ ui.rs::render: レイアウト分割 → 各エリア描画関数 → モーダル描画
```

### 1-1. 描画エントリポイント `ui::render` (ui.rs:297)

```rust
pub fn render(frame: &mut Frame<'_>, app: &mut App, theme: &Theme) {
    if app.mode == AppMode::PdfView {
        crate::pdf_viewer::draw_pdf_viewer(frame, app);
        return;
    }
    // 全体を背景色で塗る
    frame.render_widget(Block::default().style(Style::default().bg(theme.background)), area);
    // 画面が小さすぎる場合は専用表示
    if area.width < 58 || area.height < 18 { render_too_small(...); return; }
    // 縦3分割: header(3) / main(Min10) / status(1)
    let rows = Layout::vertical([Length(3), Min(10), Length(1)]).split(area);
    render_header(frame, rows[0], app, theme);
    // main は横2分割: sidebar(22) + content(Min32)
    let columns = Layout::horizontal([Length(22), Min(32)]).split(rows[1]);
    render_sidebar(frame, columns[0], app, theme);
    render_content(frame, columns[1], app, theme);
    render_status(frame, rows[2], app, theme);
    // AppMode に応じてオーバーレイ（モーダル）を描画
    match app.mode { AppMode::CommandPalette => render_palette(...), ... }
}
```

**ポイント:**
- 描画は `&mut App` と `&Theme` のみに依存。`AppMode` / `Page` / フォーカス状態で分岐
- オーバーレイ（モーダル）はエリア描画の**後**に上書きで描く
- カーソル制御（`SetCursorStyle`）は描画末尾で `crossterm::execute!` により設定

---

## 2. 状態管理（state.rs）

`App` は画面全体の状態を持つ巨大構造体。主要な区切り:

- **`AppMode`** — 現在の操作モード（`Normal`, `Search`, `PaperDetail`, `NoteEdit`, `PdfView`, `CommandPalette`, `SettingsModal`, `Prompt`, `ConfirmDelete`, ...）
  - 描画分岐・キー分岐の両方で使う
- **`Page`** — ナビゲーションで選ぶワークスペース（`Dashboard`, `Discover`, `Library`, `ReadingQueue`, `Groups`, `Bookmarks`, `Authors`, `Notes`, `Downloads`, `History`, `Statistics`, `Settings`, `Credits`, `Projects`）
  - `Page::ALL`（一覧）、`Page::title()`（表示名）、`Page::config_str()`（設定文字列）
- 各ページ専用の選択・スクロール状態（`app.library.selected/scroll`, `app.notes_selected/scroll`, ...）
- `app.content_focused` — フォーカスがサイドバーかコンテンツか
- `app.toast` / `app.toast_timestamp` — トースト通知

---

## 3. イベントループとキー処理（main.rs）

### 3-1. ループ構造: 「イベントを読んでから描画」

```rust
// まず pending イベントを全部処理
while event::poll(Duration::ZERO)? {
    process_terminal_event(event::read()?, ...).await?;
}
// 必要なら待機して1件処理
if event::poll(timeout)? {
    process_terminal_event(event::read()?, ...).await?;
}
// その後に描画 ← キー反映が1フレーム遅れない
```

### 3-2. キー → アクション → 状態遷移

```
process_terminal_event(Event)
  Event::Key => handle_key(app, key)          // → Option<UiAction>
  Event::Mouse => handle_mouse(app, mouse)
  Event::Paste(text) => paste_text_into_active_input(...)

handle_key 内部はモードでルーティング:
  AppMode 固有モーダル → handle_xxx_key
  通常時 → handle_focused_workspace_key(app, key) → ページ別ハンドラ
    library_action() / handle_notes_key() / bookmark_action() / ...
      → UiAction を返す（例: UiAction::OpenLibraryPaper(paper.id)）

UiAction が返ったら apply_ui_action(action, ...) で処理
  → DB 操作 / 非同期タスク起動
  → app.mode = AppMode::PaperDetail などの状態遷移
```

**パターン:** キーハンドラは「副作用を直接やらず `UiAction` を返す」関数群。
`UiAction`（enum, main.rs:423）が一元的に状態遷移・DB・タスク起動を実行します。
別 TUI に移植する場合は「**キー → アクション → 状態遷移**」の3層に分けるのが再現ポイントです。

---

## 4. 状態遷移の実例: 表形式リスト → Enter → PAPER

遷移経路は「キー → `UiAction` → `app.mode = PaperDetail` → 次フレームで `render_paper_detail`」。

| ページ | Enter キーの処理 | 遷移先 |
|---|---|---|
| **Library**（表形式1行） | `library_action` → `OpenLibraryPaper(id)` | **PAPER 詳細** |
| **Discover** | Enter/Right/l → `OpenPaper(remote)` | **PAPER 詳細** |
| Reading Queue | Enter/Right/l → `OpenPdf` | PDF ビューア |
| Bookmarks | Enter/Right/l → `OpenPdf` | PDF ビューア |
| Notes | Enter/Right/l → `OpenPdf` | PDF ビューア |

```rust
// Library の Enter
fn library_action(app: &mut App, key: KeyEvent) -> Option<UiAction> {
    if app.page != Page::Library { return None; }
    if key.code == KeyCode::Enter {
        let paper = *app.filtered_library_papers().get(app.library.selected)?;
        return Some(UiAction::OpenLibraryPaper(paper.id));   // ← ここでアクション化
    }
    ...
}

// apply_ui_action 内
UiAction::OpenLibraryPaper(paper_id) => {
    runtime.database.record_activity("paper_browsed", Some(paper_id), None)?;
    if let Some(idx) = app.filtered_library_papers().iter().position(|p| p.id == paper_id) {
        app.library.selected = idx;           // 選択同期
    }
    app.mode = AppMode::PaperDetail;          // ★遷移
    app.paper_detail_scroll = 0;
}
```

---

## 5. 見た目（デザイン）の再現

### 5-0. 全体レイアウト

```
┌─ header（高さ3）────────────────────────────────────┐
│  [ P A P R ]  <ページ名>                   Ctrl+B  Browse │
├────────────────────────────────────────────────────┤
│  sidebar(幅22) │             main                 │
│  NAVIGATION    │   WORKSPACE（ページごと）         │
│                │                                   │
├────────────────────────────────────────────────────┤
│  status bar（高さ1・カラーのキーキャップ）            │
└────────────────────────────────────────────────────┘
```

- 縦: header = `Length(3)`, main = `Min(10)`, status = `Length(1)`
- 横: sidebar = `Length(22)`, content = `Min(32)`
- 画面が狭い場合（幅 < 58 または 高さ < 18）は `render_too_small`

### 5-1. 配色システム（theme.rs）

見た目はすべて **色トークン** で制御。`Theme` 構造体は ratatui の `Color` を持つ。

| トークン | 用途 |
|---|---|
| `background` | 画面背景 |
| `surface` | パネル背景・選択行のハイライト背景 |
| `text` | 本文・タイトル |
| `muted` | ラベル・説明・メタ情報 |
| `accent` | 選択・フォーカス・ロゴ・リンク |
| `secondary` | サブアクセント・バッジ |
| `success` / `warning` / `error` | 状態色（トースト・DL状態・キーキャップ） |
| `border` | 枠線・区切り |

- ビルトイン19種（catppuccin系 / tokyo-night / gruvbox / nord / dracula / light 等）
- ユーザーは TOML で `ThemeSpec`（`#RRGGBB` 文字列）を定義可能
- `Theme::load(name_or_path)` で読込

**互換性のコツ:** UI コードに直接色を書かず、必ず `theme.*` トークン経由にする。これでテーマ切替が透過的に効く。

### 5-2. Header（ui.rs:462 `render_header`）

- ロゴ `const LOGO = "[ P A P R ]"` → `fg(accent)` + `BOLD`
- `"   "` + ページ名（`app.page.title()`）→ `fg(text)` + `BOLD`
- 下端 `Borders::BOTTOM`（`fg(border)`）
- 右端18文字は `"Ctrl+B  Browse"` 右寄せ・`fg(muted)`

### 5-3. Sidebar（ui.rs:489 `render_sidebar`）

- `focus_block(" NAVIGATION ", focused, theme)` 枠付きパネル
- 中身は `List`（ページ名一覧、各項目 `"  名前"`）
- フォーカス時: 選択 = `fg(background)+bg(accent)+BOLD`・シンボル `"›"`・枠は太枠+accent
- 非フォーカス: 全項目 `fg(muted)`・選択 = `bg(surface)+fg(text)`

**`focus_block`（ui.rs:65）— このアプリの中核ビジュアル部品:**
```rust
fn focus_block(title, focused, theme) -> Block {
    let border_style = if focused {
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else { Style::default().fg(theme.border) };
    let title_style = if focused {
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else { Style::default().fg(theme.muted) };
    let mut block = Block::default().title(Line::styled(title, title_style))
        .borders(Borders::ALL).border_style(border_style);
    if focused { block = block.border_type(BorderType::Thick).border_set(border::THICK); }
    block
}
```
→ フォーカス状態で「**太枠 + accent + BOLD**」vs「**細枠 + border**」を切り替える統一ルール。

### 5-4. Main / Content（ui.rs:516 `render_content`）

- `focus_block(" WORKSPACE ", content_focused, theme)` 枠付きパネル
- 内側は左右1セルの余白（`inset`）
- 検索可能ページは上部に検索バー（高さ3）: タイトル `" Local Search (Press: > to toggle) "`、フォーカス時 accent枠
- `Page` ごとに `render_dashboard / render_library / render_projects / render_organization / ...` へ分岐

#### Dashboard
- 見出し `"Today in Research"`（text+BOLD）
- 5枚のカード（LIBRARY/QUEUE/DOWNLOADED/READ/STREAK）を `Ratio(1,5)` 等幅5分割・`Borders::ALL`
- ラベル `fg(muted)`、数値 `fg(各色)+BOLD`、中央寄せ

### 5-5. Paper List（論文一覧）— 見た目の核

**形式 (a): 表形式1行（Library / Downloads）— `paper_list_line`（ui.rs:3331）**

```
<first author(12列)> <year(5列)> <タイトル…>
例) Smith             2024     Attention Is All You Need
```
- 固定幅列: author=12（`LIST_AUTHOR_WIDTH`）, year=5（`LIST_YEAR_WIDTH`）, 間にスペース1
- はみ出しは `fit_column` / `safe_truncate` で切詰、短ければ空白パディング
- author・year = `fg(muted)`、title = `fg(text)`。行全体の色指定も可（DL状態）
- リスト本体: タイトル `" {N} PAPERS "`（`Borders::TOP` のみ）、選択 = `bg(surface)+fg(accent)`、シンボル `"> "`

**形式 (b): 2行形式（ReadingQueue / Bookmarks / Notes）— `build_paper_lines`（ui.rs:5358）**

```
<タイトル>                                        ← text + BOLD
<著者>  |  <status>  |  <年>  |  <サイズ> ...      ← muted + border区切り
```
- 1行目 title（text+BOLD）
- 2行目: 著者 + `"  |  "`（`fg(border)`）区切りで統計spanを連結
  - 統計: reading_status / year / journal / DOI / ページ / コレクション名 / ファイルサイズ / availability / DLラベル
- コレクション名は `fg(secondary)+bg(surface)+BOLD` のバッジ、`(` `)` 囲み
- 幅が足りない場合は行分割

**リスト共通のクセ:** フォーカスがコンテンツにある時だけ選択ハイライトを描画
（`workspace_highlight_selection` = `app.content_focused.then_some(selected)`）。

### 5-6. Status Bar（bottom / 高さ1）（ui.rs:3998 `render_status`）

色付きキーキャップ + 説明の一行:
```
 j/k  navigate   enter  open   ?  help   q  quit
```
- キー `" j/k "` → `fg(background)+bg(accent)`
- enter系 → `bg(secondary)`・`?` ヘルプ → `bg(warning)`・`q` 終了 → `bg(error)`
- 説明文 → `fg(muted)`
- プロジェクトファイルツリー選択中は専用行に切替
- トースト通知は右寄せ `fg(success)` でステータス行に重ね描き

### 5-7. "PAPER" 論文明細（ui.rs:3448 `render_paper_detail`）

全画面モーダル:
- `Clear` + 背景塗り → マージン（横3・縦1）で内側を囲む
- 最上部 `"PAPER"` 見出し → `fg(accent)+BOLD`、下端 `Borders::BOTTOM`
- 本文 `Paper::wrap().scroll()`、最下部にキーバインド行（`fg(muted)`）
- 本文は「ラベル列（muted）+ 値（text/accent）」のメタデータ表:
```
<タイトル>（text+BOLD, 必要なら " (Downloaded) " バッジ: fg(success)+bg(surface)+BOLD）

<著者>（secondary）

Year   2024        ← ラベル muted / 値 text
Venue  ...
DOI    ...         ← accent
URL    ...         ← accent
PDF    ...         ← accent

ABSTRACT
<本文…>
RESOURCES
<パス…>
```

### 5-8. モーダル共通の見た目

- すべて `Clear` で背景消し → 枠付きパネル重ね
- パネルは `focus_block`（フォーカス: accent太枠 / 非: border細枠）
- 入力欄は `fg(text)+bg(surface)`、`frame.set_cursor_position` で実際のカーソルを追従
- **中央配置ヘルパー:** `centered(width, height, area)`（ui.rs:4922）で `Clear` 領域を算出
- 空状態は中央寄せ + muted の説明文（例: `"No local papers indexed"`）

---

## 6. 使用ウィジェット分類

### 6-1. ratatui 標準（default）— 骨格のほとんど
`Block`, `BorderType`, `Borders`, `Clear`, `Padding`, `List`, `ListItem`, `ListState`,
`Paragraph`, `Wrap`, `Alignment`, `Line`, `Span`, `Layout`, `Constraint`, `Direction`, `Rect`

### 6-2. サードパーティ
- **`ratatui-image`**（`StatefulImage`, `Resize`, `Picker`, `ProtocolType`, v8.1.1）— PDFプレビュー用。
  ただしパフォーマンス上の理由から **Kittyプロトコルを自前管理する方式にほぼ置換** 済み（下記）。
- 他は描画補助: `pulldown-cmark`(0.13, Markdown解析), `syntect`(5.2, シンタックスハイライト),
  `chrono`/`iana-time-zone`(時刻), `arboard`(クリップボード)

### 6-3. 独自実装（`impl Widget` を直接実装したもの — わずか3つ）
1. **`MarkdownHyperlinks`**（ui.rs:1684）— OSC8 ハイパーリンクを `Buffer` に2セル単位で書く。
   ratatui の ANSI 幅計算バグ回避（ratatui#902）のため `\x1b]8;;url\x07` を手動構成。
2. **`KittyBlit`**（pdf_viewer.rs:1035）— 事前エンコード済み Kitty フレームをユニコードプレースホルダとして `Buffer` へ。
3. **`KittyCleanup`**（pdf_viewer.rs:1045）— Kitty 画像クリーンアップシーケンスをバッファセルへ。

**設計原則:** 標準ウィジェットで済む部分は `impl Widget` せず関数で組み立てる。
`impl Widget` は「`Buffer` への直接書き込みが必要な特殊描画（リンク・画像）」に限定。

---

## 7. 独自 Markdown 描画（Notes エディタのプレビューでのみ使用）

- **`MarkdownRenderer`**（ui.rs:1721）— `pulldown-cmark` の `Event` を `Line`/`Span` へ変換。
  見出し・強調・リスト・引用（`"│ "` プレフィックス）・テーブル・コードブロック（syntect ハイライト）対応。
- **`markdown_preview(body, theme)`**（ui.rs:1933）— Markdown → `MarkdownPreview{ lines, hyperlinks }`
- **`MarkdownHyperlinks`**（ui.rs:1684）— `impl Widget`、プレビュー上に重ねて OSC8 リンク描画

**重要:** この独自Markdownは **Notes のノート編集プレビュー（`render_note_editor`, ui.rs:1571）でのみ** 表示されます。
Notes リスト画面（`render_organization`）は 2 行形式リストであって Markdown ではありません。
PAPER 詳細の ABSTRACT も素の `Paragraph` テキストなので注意。

```
Notes リストで論文選択 → n キー
  → UiAction::OpenNote(Local(paper.id)) → AppMode::NoteEdit
  → render_note_editor: TAB で note_preview をトグル
  → プレビューONなら markdown_preview(body, theme)
  → MarkdownHyperlinks を重ねてクリック可能リンク描画
```

---

## 8. PDF ビューア（pdf_viewer.rs）— パフォーマンス最適化

独特な点のみ:
- **自前 Kitty エンコード:** `StatefulImage` を素直に使うとスクロール毎に全画像を再エンコードして遅い。
  そこで `CropKey(path_fp, page, dpi, pixel_w, crop_y, crop_h)` をキャッシュキーに、
  `transmit_virtual` を1回だけ実行し escape 文字列を保存 → 以降はプレースホルダ行を再描画するだけ。
- `Arc<RgbaImage>` でピクセルを共有、`pdftoppm` によるラスタライズは OS スレッドで非同期（前後各2ページをプリレンダ）。
- キャッシュキーは `(path, generation, page, dpi, pixel_w)` でパススコープ（異なるPDFの取り違え防止）。

---

## 9. ターミナル設定（terminal.rs）

`TerminalSession` が alternate screen / raw mode / キーボードエンハンスメント / マウスキャプチャ / ペーストを管理。
**`Drop` で確実に復元**（raw mode off, bracketed paste off, alt screen leave, カーソル表示）。
`supports_keyboard_enhancement()` で対応端末のみ `KeyCode::Delete` 等の区別を有効化。

---

## 10. 別 TUI / 別プロジェクトへ移植する際のチェックリスト

1. **配色をトークン化** — コードに直書き色なし。`Theme.background/surface/text/...` 経由。
2. **3層の入力処理** — キー → `UiAction`（enum）→ `apply_ui_action` で状態遷移・DB・タスク起動。
3. **単一 `App` 状態 + `AppMode`・`Page` で分岐** — 描画とキーの両方でモード切替。
4. **描画は関数群、`impl Widget` は必要最小限** — `Buffer` 直書きが必要な特殊描画だけ独自Widget化。
5. **`focus_block` でフォーカス視覚化を統一** — 太枠+accent（フォーカス）vs 細枠+border（非）を全パネルで共通化。
6. **リスト選択はコンテンツフォーカス時のみハイライト** — `workspace_highlight_selection`。
7. **表形式リストは固定幅列** — author(12) / year(5) + 1スペースで整列、`safe_truncate` で切詰。
8. **状態色の使い分け** — success(トースト/DL成功) / warning(進捗/ヘルプ) / error(削除/終了) / accent(選択/リンク/ロゴ) / secondary(バッジ/enterキー)。
9. **モーダルは `Clear` + `centered()` + 枠付きパネル**、入力欄は `set_cursor_position` でカーソル追従。
10. **PDF・画像はキャッシュキー + 非同期ラスタライズ**、OSC8リンクは `Buffer` へ2セル単位書き込み。

---

## 11. 依存バージョン（workspace Cargo.toml）

| クレート | バージョン |
|---|---|
| ratatui | 0.29 |
| crossterm | 0.28（features = `event-stream`） |
| ratatui-image | 8.1.1（crossterm, image-defaults） |
| pulldown-cmark | 0.13 |
| syntect | 5.2（default-fancy） |
