---
block: explore
run: 440
description: 調査報告書 E1: 論文の前処理（docling・翻訳・節）が作るファイルと表、取り込みの経路
---

# 調査報告書 E1: 論文の前処理が作る物と取り込みの経路

対象: `~/workdir/Sai/sai-moon`。行番号は 1 始まり（実コードで確認済み）。

## 問い 1: `after_download` の 3 段が作る物

`pipeline::after_download`（`crates/sai-paper-core/src/pipeline.rs:222`）は `pending_after_download`（同 :162）で未済の段だけを、論文 1 本の中は順番に（図 → 節 → 翻訳）、論文どうしは `pipeline.max_parallel` 本まで並べて走らせる。各段は `jobs` 表に記録され、同じ論文・同じ種類の処理が実行中なら始めない（`crates/sai-paper-core/src/jobs.rs:51`・`:134`）。

### 1a. 図の抽出（`extract_figures`、pipeline.rs:42 → `figures::extract_figures`、crates/sai-paper-core/src/figures.rs:438）

(a) 外部プログラム — docling を 1 回だけ呼ぶ（`figures.rs:557` → `citation/docling.rs` の `run_docling_with_images`、実行は `citation/docling.rs:110-127`）:

```
docling convert <PDFの絶対パス> --to md --to json --image-export-mode referenced --quiet --output <paper_dir>/.docling-figures-tmp
```

- 一時出力先は `<paper_dir>/.docling-figures-tmp/`（成功・失敗どちらでも後始末、figures.rs:471・572）。
- docling の出力規約: `<out_dir>/<PDFのstem>.md` と `<out_dir>/<stem>.json`、画像は `<out_dir>/<stem>_artifacts/`（`citation/docling.rs:150`・`:208`・`:15`）。

(b) 作るファイル（PDF のある論文フォルダ `<data_dir>/papers/{id:04}_slug/` 起点の相対名）:

| ファイル | 場所 | 説明 |
|---|---|---|
| `extract.md` | figures.rs:469（名前の定義は paths.rs:483） | docling の Markdown 全文。**無いときだけ**ここに保存し、既にあれば一時側だけに入れて上書きしない（figures.rs:552-555。引用抽出が先に作っていた場合の保護） |
| `figures/fig-001.png` … | figures.rs:600 付近（パスは paths.rs:636 `paper_figure_path`） | docling JSON の `pictures` から取り出した図。再抽出時は既存 PNG を消してから書く |

(c) 書く表（`papr.db`）:

| 表・列 | 場所 |
|---|---|
| `paper_figure_extracts`（paper_id=抽出キー, papers_id, status: parsing→ready/empty/failed, pdf_path, pdf_mtime, n_figures, error, extracted_at） | figures.rs:225 `upsert_figure_extract`（呼び出しは :483・:495・:515） |
| `paper_figures`（paper_id, papers_id, fig_no, label, title, caption, image_path, mention, page, created_at） | figures.rs:138 `replace_figures`（既存行を消して全置換） |

- `image_path`・`pdf_path` は DB には `data_dir`（`.sai/paper/`）からの**相対パス**（区切りは常に `/`）で書く（figures.rs:603-605、paths.rs:367 `to_stored_path`）。
- キャッシュ: 同じ PDF（pdf_path + pdf_mtime が一致）で status が ready/empty なら再実行しない（figures.rs:455-462）。

### 1b. 節の表（`build_sections`、pipeline.rs:178 → `sections::ensure`、crates/sai-paper-core/src/sections.rs:427）

(a) 外部プログラムなし。`extract.md` を読んで Ruby なしの Rust だけで分割する。
- 入力の探し方: `citation::resolve_extract_md`（`citation/mod.rs:542`）— ① `paper_extracts.md_path`（status='ready'、papers_id + キー → papers_id → キーの 3 段階）② 論文フォルダの `extract.md` の順。

(b) 作るファイル: なし（extract.md を読むだけ。ハッシュは DB に書く）。

(c) 書く表（sections.rs:451-497）:

| 表 | 中身 |
|---|---|
| `paper_sections`（paper_id=papers.id, sec_no, heading, label, level, kind, n_passages, n_sentences, n_chars, lead, summary） | :456 |
| `paper_passages`（paper_id, sec_no, pas_no, text, n_sentences, char_start, char_end） | :469 |
| `paper_section_builds`（paper_id, extract_path, extract_sha256, built_at, summary_model） | :489。extract.md の SHA256 が同じなら作り直さない（:415・:436-439） |

- 節の要約は設定があるときだけ LLM を呼ぶ（pipeline.rs:196: `config.rag.sections.summarize` かつ `rag.evidence.model` が空でない。`sections::summarize_file` sections.rs:972 → `UPDATE paper_sections SET summary`）。
- 失敗してもあとの翻訳は続ける（pipeline.rs:175 付近のコメント）。

### 1c. PDF 翻訳（`translate_pdf`、pipeline.rs:72 → `translation/pdf2zh.rs` の `run_pdf2zh`、:135）

(a) 外部プログラム — pdf2zh（PDFMathTranslate）:

```
pdf2zh <PDFの絶対パス> -o <PDFのあるフォルダ> -li en -lo ja --service <translate.pdf.service>
  （--prompt <一時ファイル> は translate.prompt_format == "translategemma" のとき）
環境変数: OPENAILIKED_BASE_URL / OPENAILIKED_API_KEY / OPENAILIKED_MODEL を子プロセスへ渡す
```

- 引数の組み立ては pdf2zh.rs:120 `pdf2zh_args`、実行は :175-183。時間の上限は 60 分（pdf2zh.rs:19）。
- API キーは設定ファイルに置かず、`translate.pdf.api_key_env` で指定した環境変数から読む（pdf2zh.rs:86 `pdf2zh_api_key`。空なら vLLM 用ダミー `EMPTY`）。

(b) 作るファイル:
- pdf2zh が `<PDFのstem>-mono.pdf` と `<stem>-dual.pdf` を PDF と同じフォルダに作る（pdf2zh.rs:226-240）。
- mono を既定に `<paper_dir>/paper-ja.pdf` へリネーム（pdf2zh.rs:245-256）。`after_download` 経路ではさらに論文フォルダへ移す（pipeline.rs:112-122、名前は paths.rs:479 `PDF_JA`）。dual は消さずに残る。
- 翻訳済み PDF の判定は `<stem>-dual.pdf` 優先、無ければ `<stem>-ja.pdf`（paths.rs:611 `translated_pdf_path`）。

(c) 書く表（pipeline.rs:96-130、直接 SQL）:

| 表・列 | 値 |
|---|---|
| `paper_translations`（paper_id=抽出キー, papers_id, status: translating→ready/failed, pdf_path, pdf_mtime, service, lang_in='en', lang_out='ja', pdf_ja_path, translated_at, error） | 開始・完了・失敗の 3 回 upsert。pdf_path は data_dir 相対（`db.store_path`） |

### docling 出力の再利用の経路（重要）

`extract.md` は 1 本につき 1 回 docling を動かして作り、次の 5 か所で再利用される:

| 再利用先 | 根拠 |
|---|---|
| 引用抽出（`paper cite` 系） | `citation/mod.rs:355` `reusable_markdown`（extract.md が PDF より新しく空でなければ docling を飛ばす）＋ figures.rs 冒頭コメント（モジュール文書） |
| 節の表 | sections.rs:427（`ensure` が extract.md から構築） |
| RAG のチャンク | chunking.rs:748・855（`paper_extracts.md_path` から読む。下の問い 3） |
| `sai-paper content`（本文出力） | crates/sai-paper-cli/src/content.rs:242・:261（`resolve_extract_md` で extract.md を出力） |
| Radio の音声化 | crates/sai-paper-cli/src/phone.rs:201-207（音声化の元テキストとして `resolve_extract_md` で探す） |

docling の **JSON は保存しない**（一時フォルダで図を取り出した後に消える）。図の JSON 再解析が必要なら docling を再実行することになる。

## 問い 2: 引用抽出で docling を使う処理

- CLI の入り口（2 つ）:
  - `sai-paper phone citations-extract <paper_id>`（`crates/sai-paper-cli/src/main.rs:366` 定義、:1446 で `phone_cite::citations_extract`、crates/sai-paper-cli/src/phone_cite.rs:151）— TUI の `s` キーと Web の「引用の抜き出し」が使う。
  - `sai-paper extract <paper_id>`（main.rs:1514 `handle_extract`。古い形。PDF が無ければ S2 で取得もする）。
- core: `citation::ensure_extract`（`crates/sai-paper-core/src/citation/mod.rs:157`）。
  - docling 実行（画像なし版・引用用）: `docling <pdf> --to md --output <ディレクトリ>`（`citation/docling.rs:64`・:122-127 `run_docling`）。
  - S2 references の取得に失敗しても docling のみで続行（mod.rs:236-242）。docling が失敗しても部分成功として扱う（phone_cite.rs:184-190）。
- 作る物:
  - ファイル: `extract.md`。Library 登録済み（papers_id が分かる）なら論文フォルダ、無ければ `<data_dir>/extracts/<正規化キー>.md`（mod.rs:261-274）。
  - 表: `paper_refs`（引用文献行）、`paper_sentences`（本文中の引用文）、`paper_extracts`（status/pdf_path/pdf_mtime/n_refs/n_sentences/md_path/error、主キー paper_id=抽出キー）— mod.rs:12 の文書、`citation/db.rs:398-433` `upsert_extract`。
- キャッシュ: 同じ PDF（pdf_path + pdf_mtime）で status='ready' なら DB の結果を返すだけ（mod.rs:228-237）。

## 問い 3: RAG の埋め込み・チャンクの依存

- **チャンクは docling 出力に依存する**。`chunking.rs` の `chunk_paragraph`（crates/sai-paper-core/src/chunking.rs:700）と `chunk_section`（:818）は、`paper_extracts` の `md_path`（status='ready'）が指す extract.md を読んで分割する（:748・:855）。PDF を直接読む処理はない。作る表は `chunks`（`source_ref = "extract_<抽出キー>"`、strategy: paragraph/section、index_name 別）。
- **埋め込みは PDF・docling に直接依存しない**。`embeddings.rs` の `embed_pending`（crates/sai-paper-core/src/embeddings.rs:392）は `chunks` 表の未埋め込み行だけを HTTP（OpenAI 互換 `/v1/embeddings`）で埋め込み、`chunk_embeddings` に書く。
- つまり依存は「PDF → extract.md（docling）→ paper_extracts.md_path → chunks → chunk_embeddings」の 1 本道。extract.md が無い論文はチャンクが作れない（chunking.rs:750-752 で skip）。

## 問い 4: PDF 取り込みの経路（Web → core まで）

```
POST /api/projects/{name}/papers/manual/upload   apps/web/src/sai_web/main.py:1007
  → papers.request_manual_import                  apps/web/src/sai_web/papers.py:1417
  → （202 ですぐ返す）_manual_import_job（背景）    papers.py:1455
      → sai-paper phone import-pdf <対象> --stdin  papers.py:1486（120 秒まで）
          → CLI main.rs:418 ImportPdf → phone_cite.rs:303 manual_import_command
              → manual_import::import_pdf          crates/sai-paper-core/src/manual_import.rs:201
      → 成功なら _run_after_download               papers.py:1068
          → sai-paper phone after-download <id>    papers.py:1077（2000 秒まで）
              → phone_import.rs:480 after_download → pipeline::after_download
```

- `import_pdf`（manual_import.rs:201-278）がやること: `%PDF-` 検査 → 保存先決定（Local なら論文フォルダの `paper.pdf`、既にあれば上書きせず別名。Remote ならダウンロードフォルダの衝突しない名前。その後 `attach_download` が論文フォルダへ移すことがある）→ 移動 → `LibraryIndexer::inspect`（大きさ・内容ハッシュ）→ 登録（`attach_manual_pdf` / `attach_download`）→ 出来事 `downloaded` を記録。**前処理はしない**。
- **取り込みのあと `after_download` は自動で走る**（Web の Download・manual/import・manual/upload・browser/import のすべてで同じ `_run_after_download` を通る。papers.py:1068 の docstring、:1152・:1488）。
- **止める設定は無い**。`config.rs` に skip/disable 系の項目は無い（全項目を確認）。飛ばせるのはキャッシュだけ:
  - 図: `paper_figure_extracts` に ready/empty の行が同じ pdf_path + pdf_mtime であれば（pipeline.rs:162 `pending_after_download`・figures.rs:455）。
  - 翻訳: `paper-ja.pdf` か `paper-dual.pdf` が実在すれば（pipeline.rs:167 → paths.rs:611）。
  - 節: extract.md の SHA256 が一致すれば（sections.rs:436）。
  - → **サーバ版では「前処理済みの物（ファイル＋DB の行）を先に置いておけば、取り込み後の自動処理は全部飛ばす」**。
- 論文フォルダの決まり（`crates/sai-paper-core/src/paths.rs:504` `paper_folder`）:
  - 置き場は常に `<data_dir>/papers/`。`data_dir` は `<root>/.sai/paper/`（config.rs:57-66 `Paths::discover`、paths.rs `paper_roots`）。
  - 既存フォルダを `{id:04}_` 接頭辞 / `{id:04}` 完全一致で探し（タイトルが変わっても見つかる。複数候補は辞書順の最初）、無ければ `{id:04}_{短いタイトル}` の名前を返す（フォルダは作らない。作るのは書き込む側）。短いタイトルは ASCII 英数字のみ・ハイフン区切り・最大 40 文字（paths.rs:559 `short_title_slug`）。
  - DB に書くパスはすべて `data_dir` からの相対（paths.rs:367 `to_stored_path` / :398 `resolve_stored_path`、database.rs:283 `store_path`）。**前処理結果を送る側もこの規約（相対パス・区切り `/`）に揃える必要がある。**

## 問い 5: 論文 1 本を識別する鍵

| 鍵 | 根拠 | サーバ移行への利用可否 |
|---|---|---|
| `papers.id`（連番 rowid） | database.rs:268-281 | サーバの DB と番号が一致する前提が要る。**サーバ側の番号に写像して使う鍵（推測）**。前処理結果の各行は `papers_id` と抽出キーの両方を持つ設計なので、送る側が空で送り、サーバ側で埋めるのが安全（読み出しは papers_id 無しでもキーで引ける） |
| `citation_key_for_paper`（database.rs:1023） | `papers` 表の `s2_paper_id` → `arxiv_id` → `doi` の優先で 1 つ返し、`citation::normalize_paper_id`（citation/mod.rs:94）で `ArXiv:` / `DOI:` 接頭辞付きに正規化 | **最良の鍵**。`paper_figures`・`paper_figure_extracts`・`paper_extracts`・`paper_translations`・`chunks.source_ref` の主キー（paper_id 列）はこの正規化キーそのもの |
| DOI | 同上 | 安定して移せる（出版社由来・機械間で不変） |
| arXiv ID | 同上 | 安定して移せる |
| S2 40hex | 同上 | S2 由来の論文にしか無い。無い論文は DOI / arXiv に落ちる |
| タイトル | — | 鍵ではない（フォルダ名には入るが、DB の鍵には使われない） |

- 読み出しは 3 段階フォールバック（papers_id + 現在のキー → papers_id だけ → キーだけ）なので、キーが変わっても過去の記録が見える（figures.rs:187 `get_figures_for`・:323 `get_figure_extract_for`、citation 同様）。**前処理済み行には抽出キー（正規化済み）を必ず入れれば、papers_id が後からでも結び付く。**
- PDF の同一性の補助鍵: pdf_path（data_dir 相対）+ pdf_mtime（エポック秒）。mtime は機械間でずれるので（推測）、移行時はキャッシュ判定（ready + mtime 一致）を通すため mtime も合わせて送るか、サーバ側で再計算して揃える必要がある。

## 問い 6: 前処理の設定

| 設定 | 読む場所 | 既定値 |
|---|---|---|
| `translate.pdf.*`（service/base_url/model/api_key_env/prompt_format） | `.sai/paper/settings.json`（config.rs:87 `SETTINGS_FILE`、:796 `Config::load`。構造体 `TranslateService` config.rs:184-205） | service=`openailiked`、base_url=`http://192.168.100.26:8000/v1`、model=`translategemma-4b`、api_key_env=空、prompt_format=`translategemma`。API キーだけは環境変数（`api_key_env` で指定。`OPENAILIKED_*` も子プロセスへ渡す） |
| `pipeline.max_parallel` | `PipelineConfig`（config.rs:341-348）。読むのは pipeline.rs:26 `slots`（Semaphore。**起動後最初に読んだ値で固定**） | 6 |
| `rag.sections.summarize` / `rag.evidence.model` | config.rs:432-437 / :354-379（要約を LLM でやるかどうか。evidence 既定は OpenRouter・model 空＝無効） | summarize の既定は settings.json に無ければ false（model 空でも実質無効） |
| docling の場所 | `citation/docling.rs:23` `resolve_docling_bin`: ① `DOCLING_BIN` 環境変数 ② `which docling` ③ `<root>/.venv/bin/docling`（と親フォルダ） | PATH 上の `docling` |
| pdf2zh の場所 | `translation/pdf2zh.rs:28` `resolve_pdf2zh_bin`: ① `PDF2ZH_BIN` ② `which pdf2zh` ③ venv 候補 + `<root>/../lib2/PDFMathTranslate/.venv/bin/pdf2zh` | PATH 上の `pdf2zh` |
| 翻訳の時間の上限 | pdf2zh.rs:19（定数。設定項目ではない） | 60 分 |

設定ファイルは `.sai/paper/settings.json`（プロジェクトごと）。docling・pdf2zh は settings ではなく**環境変数と venv 探索**で決まる点に注意（サーバ版では `DOCLING_BIN` / `PDF2ZH_BIN` の明示指定が確実）。

## 問い 7: 前処理結果の書き出し / 取り込みに近い既存命令

**「前処理結果の export / import / bundle」は存在しない。** コマンド一覧（crates/sai-paper-cli/src/main.rs:43-427）と実装を確認。近いものは:

| 命令 | 何をやる | 流用可否 |
|---|---|---|
| `sai-paper phone import-pdf` / `candidates` / `after-download`（main.rs:400-426） | PDF 本体の取り込みと取り込み後の自動処理 | PDF の受け取りの型（`--stdin`・マジック検査・衝突しない保存先）は流用できる。**前処理結果（md・図・DB 行）の取り込みは無いので新設が要る** |
| `sai-paper phone import list/add/remove`（phone_import.rs:39-73） | BibTeX（.bib）ファイルの管理 | 形式が違う。参考程度 |
| `sai-paper content <id>`（content.rs:242） | extract.md 全文を stdout へ | 書き出し側の 1 部品になり得る（図の表の JSON 出力は無い） |
| `sai-paper relayout`（main.rs:265） | 論文フォルダ名の整理（移動はしない） | 無関係 |
| `sai-paper figures <id…>` / `translate-pdf <id>` / `citations-extract <id>`（main.rs:331-376） | 各前処理を手動で 1 本ずつ実行 | サーバ側で「足りない前処理を催促する」のに使える |

## 分からなかったこと

- `paper_figures.image_path` 等の相対パスは data_dir 起点だが、旧レイアウト（絶対パスで書かれた行）が実データに残っている可能性（paths.rs:398 の docstring に「古い絶対パスのままの行」とあり）。実 DB の中身は見ていない（要確認）。
- `radio.mp3` / `radio.m4a`（paths.rs:484-485）は apps/radio が論文フォルダに書くが、書き込み経路（Web → radio の API）は今回の範囲外として追っていない。
- attach_download が「ダウンロードフォルダから論文フォルダへ PDF を移す」条件の細部（database.rs 側）は `import_pdf` のテスト（manual_import.rs:470 付近）で動作は確認したが、DB 列レベルの全分岐は追っていない。

## サーバ版を作るときの要点（要約）

1. 前処理が作る物は「ファイル 4 種（extract.md・figures/fig-NNN.png・paper-ja.pdf・（副産物）paper-dual.pdf）＋ 表 6 種（paper_figures・paper_figure_extracts・paper_sections・paper_passages・paper_section_builds・paper_translations。引用抽出を含めるなら paper_refs・paper_sentences・paper_extracts）」。
2. どの表・ファイルも鍵は「正規化抽出キー（S2 40hex / `ArXiv:…` / `DOI:…`）＋ papers_id」。パスは data_dir（`.sai/paper/`）起点の相対（`/` 区切り）。
3. Mac 側で前処理した物は、同じ相対パス規約でファイルを置き、上の表の行を送ればよい。`after_download` はキャッシュ（ready/empty・翻訳 PDF 実在・SHA256 一致）で全部飛ばすので、取り込み後の自動処理を止める設定は不要。

