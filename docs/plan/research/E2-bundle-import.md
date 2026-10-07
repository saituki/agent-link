---
block: explore
run: 444
description: 調査報告書 E2: 前処理の zip と BibTeX を取り込む命令を作るための core・CLI・Web の調査
---

# 調査報告書 E2: 前処理 zip と BibTeX の取り込み命令のための調査

対象: `~/workdir/Sai/sai-moon`（書き換えない）。行番号は 1 始まり（実コードで確認）。
前提は調査報告書 E1（`sai-link doc read hVhoTCmh`）: `after_download` の 3 段、作る物、鍵、設定。

表記: 「事実」は本文で確認したこと、「（推測）」は未確認、「（要確認）」は決められない点。

---

## Q1: `figures::extract_figures` を「docling を呼ばず、すでにある md・json・画像から図を取る」形に分ける境目

### 答え
境目は `crates/sai-paper-core/src/figures.rs:557`（docling 呼び出し）と、その直後の `crates/sai-paper-core/src/figures.rs:565`（`let figures = output.figures;` 以降の保存処理）である。

### 現在の構造（事実）
- `figures::extract_figures`（`crates/sai-paper-core/src/figures.rs:438`）が担当を決める:
  1. PDF の実在確認・pdf_mtime 算出（`:445`・`:450`）、キャッシュ判定（`:457-464`）
  2. 一時フォルダ `<paper_dir>/.docling-figures-tmp` の準備（`:470-472`）
  3. `upsert_figure_extract(status="parsing")`（`:474`）
  4. `run_figure_pipeline(...)` を呼ぶ（`:487`）
  5. 成否で `upsert_figure_extract(ready/empty/failed)`（`:503`・`:517`・`:529`）
- `run_figure_pipeline`（`crates/sai-paper-core/src/figures.rs:540`）の中身:
  - `md_target` を決める（`extract.md` があれば一時側へ、無ければ `extract.md` へ直書き。`:552-556`）
  - **`docling::run_docling_with_images(...)` を await（`:557-563`）→ ここが外部プログラムを呼ぶ最後の行**
  - `let figures = output.figures;`（`:565`）以降が「JSON の結果を `paper_figures` に書く」部分:
    - 0 件なら `replace_figures(&[])`（`:566-569`）
    - `<paper_dir>/figures/` を作り既存 PNG を消す（`:572-581`）
    - 各図を `paper_figure_path(paper_dir, fig_no)` へ `std::fs::copy`（`:601-604`）
    - 言及文を `sentences::find_mention(&output.markdown, label)` で作る（`:606-609`）
    - `image_path` を `to_stored_path(data_dir, ...)` で相対化（`:625-628`）
    - `replace_figures`（`:629`）

### 分け方の案（事実に基づく）
1. `crate::citation::docling` の JSON 読み取りを公開する。
   `parse_figures_from_json(json_path, out_dir)`（`crates/sai-paper-core/src/citation/docling.rs:253`）が本体。今は private。`DoclingFigure`（`docling.rs:78`）は既に public なので、薄い public 関数（例 `read_docling_figures(json_path, out_dir) -> Result<Vec<DoclingFigure>, ExtractError>`）を足すだけでよい。画像 URI の解決は `out_dir` 基準（`docling.rs:336` `resolve_image_path`、`out_dir/<stem>_artifacts/...` を探す `docling.rs:358` `find_file_recursive`）。
2. `run_figure_pipeline` の `:565` 以降を新関数へ切り出す。
   例 `store_figures(conn, key, papers_id, markdown: &str, figures: &[DoclingFigure], paper_dir, data_dir) -> Result<usize, FigureError>`。中身は現在の `:565-630` をそのまま移す（`output.markdown` → 引数の `markdown`）。
3. 新命令は「md を `extract.md` へ置く」→「json を `read_docling_figures` で読む」→「`store_figures` を呼ぶ」→「`upsert_figure_extract(ready/empty)` を書く」の順にする。

### すでに public な部品（そのまま使える。事実）
- `figures::replace_figures`（`figures.rs:138`）— `paper_figures` を全置換。
- `figures::upsert_figure_extract`（`figures.rs:246`）— `paper_figure_extracts` を upsert（`pub mod figures` 経由で使える。lib.rs の re-export には無いが `sai_paper_core::figures::upsert_figure_extract` で届く）。
- `FigureRow`・`FigureExtractRow`（`figures.rs:36`・`:64`。lib.rs:50 で re-export）。

### 注意（事実）
- `extract_figures` のキャッシュ判定（`figures.rs:457-464`）は `pdf_path`（data_dir 相対）と `pdf_mtime`（エポック秒）の一致を見る。前処理 zip を取り込むとき `pdf_mtime` を Mac の値のまま書くと、サーバ側の実ファイルの mtime と食い違い、次回 `sai-paper figures` / `after-download` が docling を呼ぶ。**行を書くときにサーバ側で `pdf_mtime` を計算し直す**のが安全（E1 の指摘どおり）。
- 「docling の JSON は保存しない」設計（E1）。前処理 zip 方式では JSON を初めて保存することになる。新しい保存名を決める必要がある（要確認。`<paper_dir>/extract.json` など）。

---

## Q2: `citation::ensure_extract` が docling を呼ばずに済む条件

### 答え
条件は 2 段ある。**どちらか一方**を満たせば docling は呼ばれない。

1. **DB キャッシュ**（最優先）: `paper_extracts` に `status='ready'` の行があり、`pdf_path` と `pdf_mtime` が現在の値と一致すること（`crates/sai-paper-core/src/citation/mod.rs:228-237`）。一致すれば DB の `paper_refs` / `paper_sentences` をそのまま返して終わる（`:238-255`）。**行を先に書いておけば docling は絶対に走らない。**
2. **extract.md の再利用**（キャッシュが無いとき）: `reusable_markdown(md_path, pdf_path)`（`citation/mod.rs:358`）が
   - `metadata(md_path).modified() >= metadata(pdf_path).modified()`（`md` が PDF より**古くない**。`:360`）
   - 中身が空でない（`:363-364`）
   の両方を満たすと、その md を使って docling を飛ばす（呼び出しは `citation/mod.rs:477`）。

### md_path の場所（事実）
`ensure_extract_with` は自分の手で md_path を決める（`citation/mod.rs:261-275`）:
- Library 登録済み（`papers_id` が引ける）→ `paper_folder(data_dir, db_id, title).join("extract.md")`（`paths.rs:504`・`paths.rs:483`）。**フォルダは書き込む前に作る**（`:266`）。
- 未登録 → `<data_dir>/extracts/<sanitize(key)>.md`（`:272-275`）。

**したがって取り込みでは、`paper_folder(...)` が返すフォルダに `extract.md` を置けばよい。** `resolve_extract_md`（`citation/mod.rs:542`）も 2 番目の候補として同じ論文フォルダの `extract.md` を見る（`:604-616`）ので、節の表（`sections.rs:441`）もこれで足りる。

### 取り込みで要る物（事実 → 対策）
- `extract.md` の中身が空でないこと（`reusable_markdown` の条件）。
- `extract.md` の mtime が `paper.pdf` の mtime 以上であること。zip 解凍の既定では mtime が保たれる／揃わない場合がある。**取り込みの最後に `extract.md` の mtime を `paper.pdf` 以上にする（touch 相当）か、`paper_extracts` の ready 行を先に書く。**
- `paper_extracts` 行を書くなら `md_path` は data_dir 相対（`to_stored_path`、`paths.rs:367`）で、`status='ready'`・`pdf_path`・`pdf_mtime` をサーバ側の値で入れる。部品は `citation::db::upsert_extract` と `ExtractRow`（`crates/sai-paper-core/src/citation/db.rs:398`・`:67`。`pub mod db` は `citation/mod.rs:16`）。

### docling を呼ばずに済まない経路（事実）
- `paper_extracts` 行が無く、`extract.md` も無い／古いとき（`citation/mod.rs:477` の else → `run_docling_fn`）。
- **チャンク化は `paper_extracts` の ready 行が必須**。`chunking::chunk_paragraph`（`crates/sai-paper-core/src/chunking.rs:706`）と `chunk_section`（`:818`）は `SELECT ... FROM paper_extracts WHERE md_path IS NOT NULL AND status='ready'`（`:709-721`）で読む。extract.md を置いても行が無ければチャンクが作られない（E1 の指摘どおり）。→ **取り込みで `paper_extracts` ready 行を書くべき。**
- `paper extract <id>`（`crates/sai-paper-cli/src/main.rs:1514` → `handle_extract`）は PDF が無ければ S2 で取得もする古い経路。取り込み後は PDF があるので docling の有無だけが問題。
- `sai-paper paper figures --all`（`crates/sai-paper-cli/src/figures.rs`）は `paper_figure_extracts` の ready 行が無ければ docling を呼ぶ（`:457-464`）。

---

## Q3: `paper-ja.pdf` / `paper-dual.pdf` を置いて `paper_translations` に ready 行を書く既存関数

### 答え
**無い。** 書き込みは `pipeline::translate_pdf`（`crates/sai-paper-core/src/pipeline.rs:72`）の中の private closure `record`（`:93-100`）に直接 SQL があるだけ。

- 開始: `INSERT ... status='translating'`（`pipeline.rs:104-108`）
- 完了: `INSERT ... status='ready', pdf_ja_path, translated_at`（`:132-137`）
- 失敗: `INSERT ... status='failed', error`（`:145-149`）
- いずれも `INSERT ... ON CONFLICT(paper_id) DO UPDATE`。`pdf_path` は `db.store_path`（data_dir 相対）、`pdf_mtime` は元 PDF のエポック秒、`service` は `translate.pdf.service`。

### 書く場所の案（事実に基づく）
- `pipeline.rs` の `translate_pdf` の隣に public 関数を足す（例 `pub fn register_translation(db_file: &Path, paper_id: i64, translated_pdf: &Path, service: &str) -> Result<(), String>`）。中身は `:104`/`:132` と同じ INSERT（status='ready'）を 1 回。`translated_at` は `chrono::Utc::now().to_rfc3339()`（`:130`）。
- 注意: `after_download` の「翻訳済みか」判定は **DB ではなくファイルの実在**。`pending_after_download`（`pipeline.rs:162`）→ `paths::translated_pdf_path`（`crates/sai-paper-core/src/paths.rs:606`）が `<元PDFのstem>-dual.pdf` を優先し、無ければ `<stem>-ja.pdf` を見る（`:611-620`）。元 PDF は常に `paper.pdf`（`paths.rs:477`・`paper_file_path` `:588`）なので、`paper-dual.pdf` / `paper-ja.pdf` を論文フォルダに置けば自動処理は飛ぶ。**DB の `paper_translations` 行は表示・記録のためで、動作の判定には使われない。**
- `paper-ja.pdf` の場所は `crates/sai-paper-cli/src/phone.rs` の `paper show` が `translated_pdf_path` で解決して `files.pdf_ja` に入れる（`phone.rs:314-317`）。`paper_translations` の `pdf_ja_path` は直接読まれない（`paper show` では使っていない）。

---

## Q4: (A) PDF を既存論文に付ける — `import_pdf` と `register_downloaded_pdf` の違い

### 答え
- `downloads::register_downloaded_pdf(db, paper: &RemotePaper, downloaded, data_dir)`（`crates/sai-paper-core/src/downloads.rs:181`）は **ダウンロード専用**。`.pdf.part` を `.pdf` に rename し（`:195-198`）、`LibraryIndexer::inspect`（`:199`）して `db.attach_download(paper, ...)`（`:202`）で**新しい論文を作る／S2 ID・DOI・arXiv で既存行を探して更新する**。`%PDF-` 検査は無い。対象は常に `RemotePaper`。対象 `paper_id` を指定する口は無い。
- `manual_import::import_pdf(db, data_dir, download_dir, source, target: &ManualTarget)`（`crates/sai-paper-core/src/manual_import.rs:201`）は **手動取り込みの共通口**。`%PDF-` 検査（`:206-208`）→ 保存先決定（`:212-230`）→ 移動（`:233`）→ `inspect`（`:236`）→ 登録（`:239-246`、Local は `attach_manual_pdf`、Remote は `attach_download`）→ 出来事 `downloaded`（`:249`）。
- `ManualTarget`（`manual_import.rs:18`）:
  - `Local { paper_id, title }` → 保存先は `<paper_folder>/paper.pdf`（`paper_file_path(&folder,"pdf")`）。既に `paper.pdf` があればダウンロードフォルダへ `<題名> (2).pdf` と逃がす（`:216-228`）。登録は `Database::attach_manual_pdf(paper_id, &pdf)`（`database.rs:1004`、`papers` を**作らない**、`pdf_path`/`file_size`/`content_hash` を更新）。
  - `Remote(remote)` → 保存先はダウンロードフォルダ（`:229-230`）。登録は `Database::attach_download`（`database.rs:878`、`papers` を新規作成または既存更新し、PDF を論文フォルダへ移す）。
- **(A) は `ManualTarget::Local { paper_id, title }` がそのまま使える。**

### CLI の対象指定（事実）
`ManualImportArgs`（`crates/sai-paper-cli/src/phone_cite.rs:267`）と `manual_target`（`phone_cite.rs:395`）:
- `--paper-id <papers.id>` → `ManualTarget::Local`（`:428-435`）＝ **(A)**
- `--import-id <RemotePaper.id>` → `<data_dir>/bibtex/` の .bib 群から 1 件を探して `ManualTarget::Remote`（`:414-422`）＝ **(B)**
- `--log/--position`（検索履歴）、`--cite/--ref`（引用行）も `Remote`。
- 中身は `--file <ダウンロードフォルダの最近の PDF 名>` か `--stdin` の 1 つ（`phone_cite.rs:332-353`）。Web は `--stdin`（アップロード）を使う（`apps/web/src/sai_web/papers.py:1486` 付近）。

### 論文フォルダの決まり（事実）
`paths::paper_folder(data_dir, paper_id, title)`（`crates/sai-paper-core/src/paths.rs:504`）:
- 置き場は常に `<data_dir>/papers/`（`data_dir` は `<root>/.sai/paper/`）。
- 既存の `{id:04}_*` または `{id:04}` フォルダを辞書順で最初に見つけて返す（`:510-526`）。無ければ `{id:04}_{slug}`（英数字のみ・最大 40 文字、`short_title_slug` `:551`）の名前を返す。**フォルダは作らない**（作るのは書き込む側）。
- DB のパスは data_dir 相対（`to_stored_path` `:367` / `resolve_stored_path` `:398`、`database.rs:283` `store_path`）。

---

## Q5: (B) BibTeX から Library に論文を作る既存の経路

### 答え（関数の有無）
- **BibTeX の文字列 1 件 → `RemotePaper` 1 件** の関数はあるが private。`bibtex::papers_from_source(&str) -> Result<Vec<RemotePaper>, String>`（`crates/sai-paper-core/src/bibtex.rs:147`）。その実体 `entry_to_remote_paper`（`:326`）も private。
- public なのは:
  - `bibtex::parse_bibtex_file(path) -> Result<Vec<RemotePaper>, BibtexError>`（`:138`）— ファイルパスが要る。
  - `bibtex::save_bib_file(dir, name, content) -> Result<SavedBib, BibtexError>`（`:189`）— 文字列を解析して検査し（`:191-198`）、`<dir>/<name>` に書く。Web の Import 画面が使う（`apps/web/src/sai_web/papers.py:728` で `sai-paper import add --name <file>` → `phone_import::import_add`）。
  - `bibtex::load_bibtex_dir_detailed(dir) -> Result<BibtexScan, BibtexError>`（`:97`）— フォルダ内の .bib を全部読む。
- **`papers.id` を返す関数**は `Database::attach_download`（`database.rs:878`。RemotePaper と PDF から作る）と `Database::ensure_remote_paper`（`database.rs:1047`。PDF 無しでメタデータ行だけ作る）。どちらも `RemotePaper` を受ける。文字列から直接 `papers.id` を返す関数は無い。

**提案（事実に基づく）**: `bibtex.rs` に `pub fn parse_bibtex_str(source: &str) -> Result<Vec<RemotePaper>, BibtexError>` を足す（中身は `papers_from_source` を呼ぶだけ）。これで (B) は「1 件目の `RemotePaper` を取り、`import_pdf(..., ManualTarget::Remote)` を呼ぶ」で `papers.id` が得られる。

### 既存の (B) の流れ（Web Import 画面）
1. `.bib` を `POST /papers/import`（`apps/web/src/sai_web/main.py:945`）→ `sai-paper import add --name X --stdin`（`papers.py:728`）→ `<data_dir>/bibtex/X.bib` に保存（`phone_import.rs:127` → `bibtex::save_bib_file`）。
2. `GET /papers/import`（`main.py:939`）→ `sai-paper import list`（`papers.py:711`）= `phone_import::import_list`（`phone_import.rs:83`）。各 entry に `id`/`title`/`doi`/`local_paper_id` を返す（`:99-116`）。**S2 は呼ばない。**
3. 「⬇ Download」→ `POST /papers/download`（`main.py:963`）→ `paper download --import-id <id>`（`phone_import.rs:171`）。
   - `find_target_paper`（`:317`）が `--import-id` なら `load_bibtex_dir_detailed` から探す（**S2 を呼ばない**、`:330-338`）。
   - 既に Library にあれば `status:"exists"`。
   - 無ければ `pdf_candidates(client, &paper)`（`:206`）→ `oa_key`（`:389`）が DOI か arXiv を持つときだけ `litbase::oa_pdf_candidates` で **S2 に OA PDF を問い合わせる**（`:363`）。DOI も arXiv も無ければ候補 0 でエラー（`:208-212`）。
   - 取れたら `register_downloaded_pdf` → `attach_download`（`:243`）。
4. 手動取り込み（PDF を既に持っているとき）は `--import-id` でも `--file`/`--stdin` なら S2 を呼ばない（`manual_target` `:414-422` → `import_pdf`）。**サーバ版 (B) はこちらを使う。**

### RemotePaper の id（事実）
`entry_to_remote_paper`（`bibtex.rs:326`）は `id` を `DOI:<doi>` → `ArXiv:<arxiv_id>` → `BibTeX:<entry.key>` の優先で作る（`:344-352`）。`attach_download` は `paper.id` を `arxiv_id` と `s2_paper_id` の両方に入れる（`database.rs:918-925` の UPDATE / `:932-948` の INSERT、パラメータ `?4`・`?5` = `paper.id`）。よって BibTeX 由来の論文の抽出キーは `normalize_paper_id("DOI:...")` 等になる。`citation_key_for_paper`（`database.rs:1023`）は `s2_paper_id → arxiv_id → doi` の優先で返す。

---

## Q6: 「PDF の無い Library の論文」の一覧を返す既存の命令

### 答え
**専用の命令は無い。** Web の Papers 画面が使う `sai-paper library list`（`crates/sai-paper-cli/src/main.rs:1042` → `phone::library` `crates/sai-paper-cli/src/phone.rs:261`）の各項目に `has_pdf` があり、**利用側で `has_pdf == false` を絞る**。

- Web の経路: `GET /api/projects/{name}/papers/library`（`apps/web/src/sai_web/main.py:872`）→ `papers.library(project)`（`apps/web/src/sai_web/papers.py:153`）→ `sai-paper library list`。
- `library list` の JSON（`phone.rs:271-287`）の `papers[]` の項目: `id`・`title`・`authors`・`year`・`journal`・`reading_status`・`is_favorite`・`created_at`・`has_pdf`・`has_extract`・`podcast`。
  - **`doi`・`arxiv_id`・`s2_paper_id` は入っていない。** PDF の有無は `has_pdf`（`library_tree::paper_pdf_exists`、`crates/sai-paper-core/src/library_tree.rs:18`）。
- DOI / arXiv / S2 ID まで要るときは `sai-paper paper show <id>`（`main.rs:1407` → `phone::paper` `phone.rs:301`）。JSON は `paper{ id, title, authors, year, journal, doi, arxiv_id, s2_paper_id, ... }` と `files{ pdf, pdf_ja, extract, note }`（`phone.rs:424-452`）。

### サーバ側で論文を引くための関数（事実）
- `Database::papers_id_for_extract_key(conn, key)`（`crates/sai-paper-core/src/database.rs:455`）— 正規化キー（S2 40hex / `ArXiv:…` / `DOI:…`）から `papers.id`。**PDF の有無に関係なく引ける。**
- `Database::library_match_entries()`（`database.rs:1540`）＋ `citation::library::library_match(entries, s2_paper_id, doi, title)`（`crates/sai-paper-core/src/citation/library.rs:20`）— ただし `library_match` は **実在 PDF のある行しか一致させない**（`:33-39`）。
- `library_tree::match_downloaded_paper(papers, remote)`（`library_tree.rs:30`）— これも `paper_pdf_exists` を条件にする（`:33`・`:43`・`:56`）。
- → **(A)「PDF の無い論文」を DOI / arXiv / タイトルで探すには、上記のどれも使えない。`papers` 表を直接 `doi`/`arxiv_id`/`s2_paper_id` で引く新しい照合（`papers_id_for_extract_key` を使う、または `title` 正規化で照合）が要る（要確認）。**

---

## Q7: docling・pdf2zh が無いときの失敗と、「sai-prep で前処理してください」を出す場所

### 事実（各経路の失敗の形）
- **docling 実行**: `citation/docling.rs:160` のエラー文
  `failed to execute docling: {e}（インストール済みか確認してください。DOCLING_BIN 環境変数で明示指定も可能です）`。
  docling の場所は `DOCLING_BIN` → `which docling` → venv（`docling.rs:24-76`）。
- **pdf2zh 起点**: `translation/pdf2zh.rs:253` のエラー文
  `pdf2zh の起動に失敗しました: {e}（インストール済みか確認してください。PDF2ZH_BIN で明示指定も可能です。bin=…）`。
  場所は `PDF2ZH_BIN` → `which pdf2zh` → venv（`pdf2zh.rs:28-80`）。

### 経路ごとの見え方（事実）
| 経路 | 失敗の扱い | 出す場所 |
|---|---|---|
| Download / manual upload のあとの自動処理 | `pipeline::after_download`（`pipeline.rs:222`）が図の失敗を `"図抽出失敗: {e}"`、翻訳の失敗を `"PDF翻訳失敗: {e}"` として `notify` に渡す（`:236-256`）だけ。CLI `paper after-download`（`phone_import.rs:480`）は `messages` に集めて **終了コード 0** で返す（`:515-524`）。Web `_run_after_download`（`papers.py:1068`）は `reason`/`code` しか見ないので、**利用者には失敗が見えない**（`done` になる）。 | `pipeline.rs:236-243`・`:250-256`（文）、`phone_import.rs:515-524`（JSON）、`papers.py:1091-1103`（Web が messages を捨てている） |
| `sai-paper paper citations-extract` | `phone_cite::citations_extract`（`crates/sai-paper-cli/src/phone_cite.rs:151`）が `ExtractError::Docling(reason)` を**部分成功**として扱い、`docling_error` に理由を入れて終了コード 0（`:182-191`）。Web `request_cite_extract`（`papers.py:1270`）は `docling_error` を messages に足して `done` にする（`:1303-1310`）。 | `phone_cite.rs:184-190`、`papers.py:1304-1309` |
| `sai-paper paper translate-pdf` | `phone::translate_pdf`（`phone.rs:742`）がエラーを投げ、CLI は非 0。Web `_translate_pdf_job`（`papers.py:1541`）が `failed` にして通知（`:1552-1556`）。 | `phone.rs:745-747`、`papers.py:1548-1560` |
| `sai-paper paper figures` | CLI `figures::handle_figures`（`crates/sai-paper-cli/src/figures.rs`）が 1 件ごとに `失敗: {error}` を出し、全部失敗なら終了 1。Web からの呼び出しは無い。 | `crates/sai-paper-cli/src/figures.rs:81-89` |
| TUI の図抽出ボタン | **sai-moon に TUI は無い**（`AGENTS.md`「TUI は無い」、`ratatui`/`crossterm` 依存も無い）。該当なし。 | — |

### 直す場所（提案。事実に基づく）
1. `docling.rs:160` と `pdf2zh.rs:253` の文面に「sai-prep で前処理した zip を取り込んでください」を足す（1 行で済む）。
2. Web で利用者に見えるようにする:
   - `papers.py:1068` `_run_after_download` が `paper after-download` の stdout JSON の `messages` を読み、docling/pdf2zh 関連の文があれば `job["messages"]` に入れる（`papers.py:1104` は今 messages を入れていない）。
   - `papers.py:1304-1309` の `docling_error` の文を「sai-prep で前処理してください」に変える。
   - `papers.py:1548-1560` の翻訳失敗の通知本文（pdf2zh のエラーがそのまま入る）をそのまま使うか、言い換える。
3. 画面: `apps/web/src/sai_web/static/papers.js:3150` `figuresCard()` が `state === "not_extracted"` のとき「(図は未抽出)」を出す。ここに手前の案内を足す場所（要確認。実際の文言は読んでいない）。

---

## Q8: Web でサーバが PDF を S2 / 出版社から落とす機能（止める・隠す場所）

### 事実（呼び出しの連鎖）
1. **Discover / Import の Download**
   - 画面: `apps/web/src/sai_web/static/paperimport.js:710`（論文情報の板の「⬇ Download」）→ `pimStartDownload`（`:809`）→ `POST /papers/download`（`:814`）。
   - 検索結果から: `pimStartLogDownload`（`paperimport.js:940`）。
   - サーバ: `POST /api/projects/{name}/papers/download`（`main.py:963`）→ `papers.request_download`（`papers.py:916`）→ `_download_job`（`:1106`）→ `sai-paper paper download …`（`:1129`）。
   - CLI: `PaperCmd::Download`（`crates/sai-paper-cli/src/main.rs:374-397`）→ `phone_import::download`（`phone_import.rs:171`）→ `pdf_candidates`（`:206`、S2 の `openAccessPdf` 等）→ `downloads::register_downloaded_pdf`（`:243`）。
   - 実体: `downloads::DownloadManager::download`（`downloads.rs:83`）・`download_destination`（`:128`）・`register_downloaded_pdf`（`:181`）。
2. **`sai-paper paper get-pdf`**（`main.rs:1494` → `litbase::ensure_pdf`、`crates/sai-paper-cli/src/litbase.rs:18`）— S2 / arXiv / Europe PMC / Unpaywall から落とす。
3. **`sai-paper paper extract`**（`main.rs:1514` → `main.rs:1517` で `ensure_pdf`）— PDF が無ければ落とす。
4. **手動取り込み（ローカル PDF / アップロード / Mac のブラウザ）** — サーバは落とさない。
   - `POST /papers/manual/candidates`（`main.py:987`・`papers.py` `manual_candidates`）、`POST /papers/manual/import`（`main.py:993`）、`POST /papers/manual/upload`（`main.py:1006`）→ `papers.request_manual_import`（`papers.py:1424`）→ `sai-paper paper import-pdf …`（`:1472` 付近、`--file`/`--stdin`）→ `phone_cite::manual_import_command`（`phone_cite.rs:303`）→ `manual_import::import_pdf`（`manual_import.rs:201`）。
   - 画面: `paperimport.js:1083`（manual/upload）・`:1093`（manual/import）、`papers.js:2696`・`:2888`（「手動で取り込む」）。
   - リモートブラウザ: `POST /papers/browser/*`（`main.py:1027-1069`）→ `apps/web/src/sai_web/rbrowser.py`（Mac の Chrome を操作。PDF の取得は Mac 側）。
5. **Download 後の自動処理**: `_run_after_download`（`papers.py:1068`）→ `sai-paper paper after-download`（`:1094`）→ `phone_import::after_download`（`phone_import.rs:480`）→ `pipeline::after_download`（`pipeline.rs:222`）。

### サーバ版で止める・隠す場所（一覧。事実に基づく）
- サーバの口: `POST /api/projects/{name}/papers/download`（`main.py:963`）と `papers.request_download`（`papers.py:916`）・`_download_job`（`:1106`）。ここを 409/410 などで止めるか、`sai-prep` の取り込みに差し替える。
- CLI: `PaperCmd::Download`（`main.rs:374`）・`phone_import::download`（`phone_import.rs:171`）・`downloads::register_downloaded_pdf`（`downloads.rs:181`）。サーバの CLI からは呼べないようにする（環境変数で禁止など。要確認）。
- `paper get-pdf`（`main.rs:1494`）と `paper extract`（`main.rs:1514`）も S2 から落とすので同じ扱いが要る。
- 画面: `paperimport.js:710`・`:814`・`:940`（Download ボタンと依頼）を隠す／「sai-prep で前処理した zip を取り込む」に差し替える。`papers.js` の Download 表示（`papersRenderDownloadBand`・`papersRenderProgressButton`、`paperimport.js:827-828`）も外す。
- 残すもの: 手動取り込み（manual import/upload）とブラウザ取り込みは、サーバ版では「zip の取り込み」に置き換わる。`manual/import`（`main.py:993`）・`manual/upload`（`:1006`）を残すか消すかは設計判断（要確認）。
- Download の「exists」判定は `phone_import::existing_library_index`（`phone_import.rs:300`）で、PDF を持つ論文を探す。サーバ版で使うなら流用可能。

---

## 新命令を作るときの要点（この調査からの設計メモ）

1. **置くファイル**: 論文フォルダ（`paths::paper_folder(data_dir, papers_id, title)`）に `paper.pdf`・`extract.md`・`paper-ja.pdf`・（あれば）`paper-dual.pdf`。図は `figures/fig-NNN.png` を直接置くか、JSON から `store_figures` に作らせる。
2. **書く行**:
   - `paper_figures` + `paper_figure_extracts`（status ready/empty、`pdf_path`・`pdf_mtime` は**サーバ側で計算**）— `replace_figures`（`figures.rs:138`）＋ `upsert_figure_extract`（`figures.rs:246`）。
   - `paper_extracts`（status ready、`md_path` 相対、`pdf_path`・`pdf_mtime` サーバ値）— `citation::db::upsert_extract`（`citation/db.rs:398`）。これで `ensure_extract` もチャンク化も docling を呼ばない。
   - `paper_refs`・`paper_sentences` — 「軽い処理」として `ensure_extract` を走らせれば作れる（`citation/mod.rs` は docling を飛ばしても refs/sentences は md から作る）。または bundle に含める（要確認）。
   - `paper_sections`・`paper_passages`・`paper_section_builds` — `sections::ensure`（`crates/sai-paper-core/src/sections.rs:427`）を走らせる（docling 不要）。
   - `paper_translations`（status ready）— **既存関数が無い。新設が要る**（`pipeline.rs` の `translate_pdf` の SQL `:104`・`:132` を参考）。
3. **mtime**: 取り込み後に `extract.md` の mtime を `paper.pdf` 以上にするか、上の DB 行（`pdf_mtime` サーバ値）を先に書く。`paper_figure_extracts` も同じ。
4. **(A) の対象解決**: サーバに `papers.id` を渡すのが最も確実（`library list` の `id`）。DOI / arXiv / タイトルから引く照合は `papers_id_for_extract_key`（`database.rs:455`）を使うが、`library_match`（`citation/library.rs:20`）は PDF 必須なので使えない。
5. **(B)**: `bibtex::parse_bibtex_str`（新設）→ 最初の `RemotePaper` → `manual_import::import_pdf(..., ManualTarget::Remote)`。S2 は呼ばない（PDF を bundle から受け取るため）。
6. **`after_download` はキャッシュで全部飛ぶ**（E1 の指摘）: 図は ready 行、翻訳は `paper-dual.pdf`/`paper-ja.pdf` の実在。節と要約だけは走る（`pipeline.rs:200-220`）。

---

## 分からなかったこと
- `papers.id` を跨いだ移行の前提（サーバと Mac で同じ DB を使うのか、サーバ専用 DB に作り直すのか）はこの調査では決められない。S2 メタデータ行がサーバにあるなら (A) はサーバの `papers.id` を渡すのが最短で、BibTeX の id（`DOI:…`）からの照合は `papers_id_for_extract_key` で足りる見込み。
- `paper_figures.image_path`・`paper_extracts.md_path` 等に旧レイアウトの絶対パスが残っている実データがあるかは見ていない（E1 と同じ未確認）。
- `apps/web/src/sai_web/static/papers.js` の `figuresCard()`（`:3150` 付近）の実文字列は行だけ確認し、全文は読んでいない（文言を変えるなら要確認）。
- 取り込み後の `paper_extracts` 行を書くか、`extract.md` の mtime だけでも足りる運用にするかは、チャンク化まで含めると行を書く方が確実（この調査の結論）。

