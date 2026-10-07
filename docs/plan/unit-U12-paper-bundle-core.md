# 実装計画書 U12: 前処理の束の取り込み・PDF 無しの Library 追加・PDF 待ちの一覧（sai-paper の core と CLI）

## 目的
サーバの `sai-paper` に次の命令を足し、サーバでは PDF を落とさず、docling・pdf2zh も動かさない形にする。
1. `sai-paper paper keep <対象>` — 検索結果・BibTeX・引用から、**PDF 無しで** Library に論文（メタデータ）を入れる。
2. `sai-paper library pending --json` — Library のうち PDF の無い論文の一覧（sai-prep が「PDF 待ち」として出す）。
3. `sai-paper paper import-bundle --dir <フォルダ> --json` — 各自の Mac の sai-prep が作った「前処理の束」（PDF・docling の md と json と図・翻訳 PDF）を取り込み、図・節・引用の表を、docling を呼ばずに作る。対象は「Library にある論文（`paper_id`）」か「BibTeX 1 件（新しく作る）」。
4. サーバで PDF を落とす命令（`paper download`・`paper get-pdf`・`paper extract` の PDF 取得）を止め、docling・pdf2zh が無いときの文に「sai-prep」を案内する。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。Library の S2 検索とメタデータはサーバで行い、PDF 本体は sai-prep から送る。BibTeX と PDF を sai-prep へ drop すれば S2 検索なしでも取り込める（要件）。
- 前処理が作る物と、その再利用の経路は調査 E1（`docs/plan/research/E1-paper-preprocess.md`）、取り込みの命令を作るための関数の切れ目は調査 E2（`docs/plan/research/E2-bundle-import.md`）にある。要点:
  - 図: `figures::extract_figures`（`crates/sai-paper-core/src/figures.rs:438`）→ `run_figure_pipeline`（`:540`）。docling を呼ぶのは `:557-563`、その後の `:565-630` が「docling の結果を `paper_figures` に書く」部分。docling の JSON を読む本体 `parse_figures_from_json`（`crates/sai-paper-core/src/citation/docling.rs:253`）は private、`DoclingFigure`（`docling.rs:78`）は public。`replace_figures`（`figures.rs:138`）・`upsert_figure_extract`（`figures.rs:246`）は public。
  - 引用: `citation::ensure_extract`（`citation/mod.rs:157`）は、`paper_extracts` に `status='ready'` で `pdf_path`・`pdf_mtime` が今の値と一致する行があれば docling を呼ばない（`citation/mod.rs:228-237`）。無ければ、論文フォルダの `extract.md` が PDF より新しく空でなければ docling を呼ばずにそれを使う（`reusable_markdown`、`citation/mod.rs:358`）。行は `citation::db::upsert_extract`（`citation/db.rs:398`、`ExtractRow` は `:67`）で書ける。
  - RAG のチャンクは `paper_extracts` の ready 行が必須（`chunking.rs:709-721`）。
  - 節: `sections::ensure`（`sections.rs:427`）は `extract.md` から作る（docling 不要）。
  - 翻訳: `paper_translations` の ready 行を書く公開の関数は無い（`pipeline.rs:93-149` の private な `record`）。自動処理の「翻訳済みか」の判定はファイルの実在（`paths::translated_pdf_path`、`paths.rs:606-620`。`paper-dual.pdf` 優先、無ければ `paper-ja.pdf`）。
  - PDF の取り込み: `manual_import::import_pdf`（`manual_import.rs:201`）。`ManualTarget::Local { paper_id, title }` は既存の論文に付ける（`attach_manual_pdf`）、`ManualTarget::Remote(remote)` は論文を作るか既存を更新する（`attach_download`）。
  - BibTeX の文字列 1 件 → `RemotePaper` は private な `bibtex::papers_from_source`（`bibtex.rs:147`）。`RemotePaper.id` は `DOI:` → `ArXiv:` → `BibTeX:<key>` の順で作られる（`bibtex.rs:344-352`）。
  - PDF 無しで論文の行を作る関数は `Database::ensure_remote_paper`（`database.rs:1047`）。今はテストからしか呼ばれていない。
  - 論文の鍵: 正規化キー（`citation_key_for_paper`、`database.rs:1023`）と `papers.id`。DB に書くパスは data_dir（`.sai/paper/`）からの相対（`to_stored_path`、`paths.rs:367`）。論文フォルダは `paths::paper_folder(data_dir, papers_id, title)`（`paths.rs:504`）。
  - `pdf_mtime` は機械ごとにずれる。行を書くときはサーバ側のファイルから計算し直す（E2 Q1 の注意）。
  - Web の Papers 画面の一覧 `sai-paper library list`（`phone.rs:261`）は `doi`・`arxiv_id`・`s2_paper_id` を返さない（E2 Q6）。
  - 検索結果・BibTeX・引用から対象の `RemotePaper` を探す既存の関数: `phone_cite.rs:395` `manual_target`（`--log/--position`・`--import-id`・`--cite/--ref`・`--paper-id`）、`phone_import.rs:317` `find_target_paper`。

## 前提（着手時点のコードの状態）
- ガイド 1〜4 が終わっている。ただしこの作業は `crates/sai-paper-core` と `crates/sai-paper-cli` だけを書き、ほかの作業と重ならない。

## 変更内容

### 1. core の部品 — `crates/sai-paper-core`
- `citation/docling.rs`: `pub fn read_docling_figures(json_path: &Path, out_dir: &Path) -> Result<Vec<DoclingFigure>, ExtractError>` を足す（中身は `parse_figures_from_json` を呼ぶだけ）。
- `figures.rs`: `run_figure_pipeline` の `:565-630` を `pub fn store_figures(conn, key: &str, papers_id: Option<i64>, markdown: &str, figures: &[DoclingFigure], paper_dir: &Path, data_dir: &Path) -> Result<usize, FigureError>` に切り出し、`run_figure_pipeline` はそれを呼ぶ形にする（動きは変えない）。
- `bibtex.rs`: `pub fn parse_bibtex_str(source: &str) -> Result<Vec<RemotePaper>, BibtexError>`（`papers_from_source` を呼ぶだけ）。
- `pipeline.rs`: `pub fn register_translation(db_file: &Path, paper_id: i64, translated_pdf: &Path, service: &str) -> Result<(), String>`（`translate_pdf` の完了の INSERT（`:132-137`）と同じ形で `status='ready'` の行を 1 回書く）。
- 新しいモジュール `crates/sai-paper-core/src/bundle.rs`（`lib.rs` に `pub mod bundle;`）: `pub fn import_bundle(db: &mut Database, data_dir: &Path, dir: &Path) -> Result<BundleResult, BundleError>`。手順:
  1. `dir/manifest.json` を読み、共有契約 C12 の形か確かめる（`format`・`version`・`target`・ファイルの相対パスが `dir` の中に収まり実在すること。`..`・絶対パスは誤り）。`pdf_sha256` と実際の PDF の SHA-256 が一致すること。
  2. 対象を決める: `target.paper_id` なら `ManualTarget::Local`（無ければ「論文が見つかりません」）。`target.bibtex` なら `parse_bibtex_str` の 1 件目で `ManualTarget::Remote`（0 件なら「BibTeX に論文がありません」）。
  3. `manual_import::import_pdf(db, data_dir, <一時の置き場>, &dir/paper.pdf, &target)` で PDF を取り込み、`papers.id` と論文フォルダ（`paper_folder`）を得る。対象が Local で、論文フォルダにすでに `paper.pdf` があるときは「この論文にはもう PDF があります」で失敗する（上書きしない）。
  4. docling の md を論文フォルダの `extract.md` に写す（既にあれば置き換える）。mtime を `paper.pdf` より新しくする。
  5. `read_docling_figures(docling.json, docling の出力フォルダ)` → `store_figures(…)` → `upsert_figure_extract`（図が 0 なら `empty`、あれば `ready`。`pdf_path`・`pdf_mtime` はサーバの `paper.pdf` から計算する）。
  6. `upsert_extract` で `paper_extracts` に `status='ready'`・`md_path`（相対）・`pdf_path`・`pdf_mtime` を書く。続けて `citation::ensure_extract` を呼び、引用の表（`paper_refs`・`paper_sentences`）を作る（docling は呼ばれない。S2 の参考文献の取得に失敗しても続ける — 今の `ensure_extract` の動き）。
  7. `sections::ensure` で節の表を作る。
  8. 翻訳があれば、`mono` を論文フォルダの `paper-ja.pdf`、`dual` を `paper-dual.pdf` に写し、`register_translation`（`service` は manifest の値）。
  9. `BundleResult { paper_id, title, figures, sections, refs, translated }` を返す。
  - どの段で失敗しても、それまでに写したファイルは残してよい（キャッシュとして再利用される）。ただし 3 の前に失敗したら何も書かない。
- docling・pdf2zh が無いときの文（`citation/docling.rs:160`・`translation/pdf2zh.rs:253`）を、「サーバでは docling（pdf2zh）を動かしません。各自の PC の sai-prep で前処理して送ってください」に変える。

### 2. CLI — `crates/sai-paper-cli`
- `sai-paper paper import-bundle --dir <絶対パス> --json` → `import_bundle` の結果を `{"paper_id","title","figures","sections","refs","translated"}` で出す。manifest の誤りは終了コード 2、対象が無いときは 3、PDF がすでにあるときは 4。
- `sai-paper paper keep` — 対象の指定は `paper download` と同じ（`--log <id> --position <n>`・`--import-id <id>`・`--cite <paper_id> --ref <n>`）。`manual_target` と同じ方法で `RemotePaper` を探し（共通の関数に切り出してよい）、`Database::ensure_remote_paper` で行を作る。出力 `{"paper_id","title","status":"added"|"exists"}`（既にあれば `exists`）。
- `sai-paper library pending --json` → `{"papers":[{"id","title","year","authors","doi","arxiv_id","s2_paper_id","created_at"}]}`（`papers` のうち `library_tree::paper_pdf_exists` が偽の物。新しい順）。
- サーバで PDF を落とす命令を止める: `paper download`（`main.rs:374-397` → `phone_import::download`）・`paper get-pdf`（`main.rs:1494`）・`paper extract` の PDF 取得（`main.rs:1517` の `ensure_pdf`）は、終了コード 4「サーバでは PDF を落としません。sai-prep から送ってください」で止める。`paper after-download` は残す（取り込み後のキャッシュの確認で、何もせずに終わる）。
- `sai-paper schema --json` に新しい 3 つが出る（`web: true`）。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C12 前処理の束**（U13 が展開し、U14 が作る。全文）
- zip の中身（すべて zip の直下からの相対パス）:
  ```
  manifest.json
  paper.pdf
  docling/paper.md
  docling/paper.json
  docling/paper_artifacts/…（docling が書いた画像。--image-export-mode referenced）
  translated/paper-mono.pdf   （翻訳したときだけ）
  translated/paper-dual.pdf   （翻訳したときだけ）
  ```
- `manifest.json`:
  ```json
  {"format": "sai-prep-bundle", "version": 1,
   "target": {"paper_id": 12},
   "pdf": "paper.pdf", "pdf_sha256": "<64 桁の 16 進>",
   "docling": {"md": "docling/paper.md", "json": "docling/paper.json", "dir": "docling"},
   "translation": {"mono": "translated/paper-mono.pdf", "dual": "translated/paper-dual.pdf",
                   "service": "mlx:mlx-community/translategemma-4b-it-4bit_immersive-translate"},
   "tools": {"sai_prep": "0.1.0", "docling": "2.134.0", "pdf2zh": "1.9.11"},
   "created_at": "2026-10-07T12:00:00+09:00"}
  ```
  `target` は `{"paper_id": <int>}` か `{"bibtex": "<BibTeX 1 件の文字列>"}` のどちらか。翻訳しなかったときは `"translation": null`。
- サーバの CLI: `sai-paper paper import-bundle --dir <展開したフォルダの絶対パス> --json` → `{"paper_id","title","figures","sections","refs","translated"}`。誤り 2・対象なし 3・PDF あり 4。

**C13 Library の追加と PDF 待ちの CLI**（U13 が呼ぶ）
- `sai-paper paper keep (--log <id> --position <n> | --import-id <id> | --cite <paper_id> --ref <n>) --json` → `{"paper_id","title","status":"added"|"exists"}`。
- `sai-paper library pending --json` → `{"papers":[{"id","title","year","authors","doi","arxiv_id","s2_paper_id","created_at"}]}`。
- `paper download`・`paper get-pdf` は終了コード 4。

## ファイルの所有権
- 書いてよい: `crates/sai-paper-core/src/`（`bundle.rs`（新規）・`lib.rs`・`figures.rs`・`citation/docling.rs`・`bibtex.rs`・`pipeline.rs`・`translation/pdf2zh.rs`（文だけ））・`crates/sai-paper-cli/src/`（`main.rs`・`phone.rs`・`phone_import.rs`・`phone_cite.rs`・新しいファイル）・`crates/sai-paper-core/tests/`・`crates/sai-paper-cli/tests/`
- 触らない: `apps/`・`crates/sai-link-*`・`crates/sai-sci-*`

## 完了条件
- [ ] AC1: `crates/sai-paper-core/tests/bundle_import.rs`: 一時フォルダの `papr.db` に論文（PDF 無し）を 1 本作り、小さな PDF と、手で作った docling の md・json（図 2 つと画像 2 枚）と翻訳 PDF 2 つを C12 の形で置いて `import_bundle` を呼ぶと、論文フォルダに `paper.pdf`・`extract.md`・`figures/fig-001.png`・`fig-002.png`・`paper-ja.pdf`・`paper-dual.pdf` ができ、`paper_figures` が 2 行、`paper_figure_extracts`・`paper_extracts`・`paper_translations` が ready、`paper_sections` が 1 行以上。この間 docling・pdf2zh を呼ばない（`DOCLING_BIN`・`PDF2ZH_BIN` を、呼ばれたら印のファイルを作る偽の物にして確かめる）。続けて `pipeline::pending_after_download` が `(false, false)` を返す。
- [ ] AC2: 同じテストで、`target.bibtex` の束で新しい論文ができる。manifest の `pdf_sha256` の不一致・`../` のパスは誤り。PDF がある論文への 2 回目は「この論文にはもう PDF があります」。
- [ ] AC3: CLI のテスト: `paper keep --import-id` で PDF 無しの論文ができ、2 回目は `exists`。`library pending` にその論文が `doi` 付きで出る。`paper download` は終了コード 4。
- [ ] AC4: `cargo test --workspace` がすべて通る。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
```

## 決定事項と理由
- サーバは docling・pdf2zh を動かさず、送られた md・json から図・節・引用を作る — 理由: 要件（重い処理は各自の PC）。図の取り出し・節・引用は Rust の軽い処理で、docling の出力さえあれば作れる（調査 E1・E2）。
- 束の展開（zip）は Web が行い、CLI はフォルダを受ける — 理由: Rust の依存に zip が無い（`Cargo.lock` に `zip` クレートが無い）。Python の `zipfile` は標準ライブラリにあり、外から来る値の検査は Web が行う約束（AGENTS.md）。
- 行の `pdf_mtime` はサーバで計算し直す — 理由: Mac の値のままだと、次の自動処理がキャッシュを外して docling を呼ぼうとする（調査 E2）。
- PDF 無しの論文を作る命令を足す — 理由: 要件（S2 検索とメタデータはサーバ、PDF は sai-prep）。既存の `ensure_remote_paper` を使う。
- サーバの PDF の取得は止める — 理由: 要件の範囲外（サーバでは PDF を落とさない）。落としても docling が無いので前処理できない。

## 要確認
- `ensure_extract` が S2 の参考文献を取りに行くのに時間がかかる（または失敗する）とき、取り込み全体の時間。`import-bundle` は Web から 600 秒の上限で呼ばれる（U13）。超える見込みなら、引用の作成を `after-download` 相当に分けてよい（作業報告書に書く）。
