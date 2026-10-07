# 実装計画書 U13: 前処理の束の受け口・「Library に入れる」・PDF 待ちの表示（Web）

## 目的
1. sai-prep が個人のトークンで送る「前処理の束」（zip）を受け、検査して展開し、プロジェクトのコンテナの中の `sai-paper paper import-bundle` で取り込む。
2. sai-prep が「PDF 待ち」の論文の一覧を取れるようにする。
3. Papers の画面で、検索結果などの「⬇ Download」（サーバで PDF を落とす）を「Library に入れる」（PDF 無しでメタデータだけ入れる）に替える。PDF の無い論文には「PDF 待ち（sai-prep から送ってください）」を出す。サーバに PDF を直接上げる手動取り込みは消す。
4. sai-prep の入れ物（wheel）を Web から落とせるようにする。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。Library の S2 検索とメタデータはサーバ、PDF 本体と前処理（docling・翻訳）は各自の Mac の sai-prep（要件）。
- CLI 側（C12・C13）は U12 が作った（下に全文を写す）。Rust に zip の依存が無いので、zip の展開と検査は Web が行い、CLI には展開したフォルダを渡す。
- 今の手動取り込みの形（202 ですぐ返し、背景の job で CLI を走らせ、`GET …/papers/downloads` で進み具合を見せる）は `apps/web/src/sai_web/papers.py:1429-1500` `request_manual_import`・`_manual_import_job` と `main.py:963-1020` にある。アップロードの本文の読み方（Content-Type・大きさ・先頭の検査）は `papers.read_pdf_upload`（`papers.py:1577`）。FastAPI の `UploadFile` は `python-multipart` が要るので使わない（依存を増やさない）。
- Download の画面の入口（調査 E2 Q8）: `static/paperimport.js:710`（論文情報の板の「⬇ Download」）→ `pimStartDownload`（`:809`）→ `POST /papers/download`（`:814`）、検索結果から `pimStartLogDownload`（`:940`）、Download の帯 `papersRenderDownloadBand`・`papersRenderProgressButton`（`paperimport.js:827-828`）。手動取り込み: `paperimport.js:1083`（manual/upload）・`:1093`（manual/import）、`papers.js:2696`・`:2888`（「手動で取り込む」）。サーバの住所: `main.py:963`（download）・`:987`（manual/candidates）・`:993`（manual/import）・`:1006`（manual/upload）。
- 個人のトークン（`Authorization: Bearer sai_…`）は U06 が作った。Cookie と同じ住所を使える。

## 前提（着手時点のコードの状態）
- ガイド 1〜4 と U12 が終わっている。U14（sai-prep）は並列で進み、この作業の住所（C14）だけに頼る。

## 変更内容

### 1. 束の受け口 — `apps/web/src/sai_web/bundles.py`（新規）と `main.py`
- `POST /api/projects/{name}/papers/bundle`（`Depends(require_token)`。202）: 本文は zip そのもの（`Content-Type: application/zip`。違えば 415）。
  1. `papers.resolve_project(name)` で今の利用者のプロジェクトか確かめる（無ければ 404）。
  2. 本文を読む。`Content-Length` が無い・300MB を超えるなら 413。
  3. `zipfile` で開き、検査する: 項目の数 ≤ 5000、展開後の合計 ≤ 1GB、名前に絶対パス・`..`・`\`・NUL が無い、シンボリックリンクが無い。`manifest.json` があり、C12 の形（`format == "sai-prep-bundle"`・`version == 1`・`target` が `{"paper_id": 正の整数}` か `{"bibtex": 64KB 以下の文字列}`・`pdf` などのパスが zip の中にある）。`paper.pdf` の先頭が `%PDF-`。違えば 400 で理由を返す（CLI を呼ばない）。
  4. `<project_root>/.sai/paper/inbox/<uuid4 の 16 進>/` に展開する（このフォルダはホストとコンテナで同じパスに見える。C4）。
  5. job を作って 202 で返す: `{"key": "bundle:<uuid>", "status": "processing"}`。job は今の `_download_jobs` と同じ表に入れ、`GET /api/projects/{name}/papers/downloads` に出す（題名は manifest の bibtex の題名か、`paper_id` の論文の題名）。
  6. 背景で `sai-paper paper import-bundle --dir <展開したフォルダ> --json` を 600 秒の上限で呼ぶ（`control._phone_json` か `papers.py` の CLI の呼び方に倣う。コンテナの中で動く）。終わったら（成否どちらでも）展開したフォルダを消す。成功なら job を `done`（結果の `paper_id`・`figures`・`translated` を入れる）、失敗なら `failed`（CLI の標準エラーの最後の行）。通知（`notify`）は今の Download の完了と同じ文の形で出す。
- `GET /api/projects/{name}/papers/pending`（`Depends(require_token)`）→ `sai-paper library pending --json` の結果をそのまま返す。
- `GET /api/projects/{name}/papers/jobs/{key}`（`Depends(require_token)`）→ その job 1 件（sai-prep が進み具合を見る）。無ければ 404。

### 2. 「Library に入れる」 — `papers.py`・`main.py`
- `POST /api/projects/{name}/papers/keep`（`Depends(require_token)`）: body は今の download と同じ（`{"log_id","position"}` / `{"import_id"}` / `{"cite_paper_id","ref_no"}`。`papers._manual_target` と同じ検査）→ `sai-paper paper keep … --json` の結果を返す（202 ではなく 200。すぐ終わる）。
- `POST /api/projects/{name}/papers/download`・`manual/candidates`・`manual/import`・`manual/upload` の住所と、`papers.py` の `request_download`・`_download_job`・`request_manual_import`・`_manual_import_job`・`manual_candidates`・`read_pdf_upload` を消す（`_download_jobs` と `list_downloads` は束の job で使うので残す）。消したテストの名前を作業報告書に書く。

### 3. 画面 — `paperimport.js`・`papers.js`
- 「⬇ Download」のボタン（`paperimport.js:710`・`:940` の入口）を「＋ Library に入れる」にし、`POST …/papers/keep` を呼ぶ。結果が `added` なら `saiToast("Library に入れました（PDF は sai-prep から送ってください）")`、`exists` なら「もう Library にあります」。Download の帯の進み具合（`papersRenderDownloadBand`）は、束の取り込みの job の表示として残す。
- 手動取り込みの画面（`paperimport.js:1083`・`:1093`・`papers.js:2696`・`:2888` の「手動で取り込む」）を消す。
- 論文の詳細で PDF が無いとき、「PDF 待ち — 各自の Mac の sai-prep から送ってください」と、sai-prep の入れ方の案内（§4 の住所へのリンクと `uv tool install <落としたファイル>` の 1 行）を出す。
- `sw.js` の `CACHE` を上げ、`index.html` の `?v=` をそろえる。

### 4. sai-prep の入れ物 — `main.py`
- `GET /api/prep/download`（`Depends(require_token)`）: リポジトリの `apps/prep/dist/` の中で一番新しい `sai_prep-*.whl` を `FileResponse` で返す（ファイル名はそのまま）。無ければ 404「sai-prep の入れ物がまだ作られていません（管理者が deploy.sh を実行してください）」。リポジトリの場所は `Path(__file__)` から求める（環境変数 `SAI_PREP_DIST` があればそちら）。

### 5. 文書
- `docs/integration.md`: 新しい住所（§1・§2・§4）と、消した住所を直す。C12 の束の形を書く。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C12 前処理の束**（U12。全文）
- zip の中身: `manifest.json`・`paper.pdf`・`docling/paper.md`・`docling/paper.json`・`docling/paper_artifacts/…`・（翻訳したときだけ）`translated/paper-mono.pdf`・`translated/paper-dual.pdf`。
- `manifest.json`: `{"format":"sai-prep-bundle","version":1,"target":{"paper_id":<int>}|{"bibtex":"<1 件>"},"pdf":"paper.pdf","pdf_sha256":"<hex>","docling":{"md":"docling/paper.md","json":"docling/paper.json","dir":"docling"},"translation":{"mono":"…","dual":"…","service":"…"}|null,"tools":{"sai_prep","docling","pdf2zh"},"created_at":"RFC3339"}`。
- CLI: `sai-paper paper import-bundle --dir <絶対パス> --json` → `{"paper_id","title","figures","sections","refs","translated"}`。誤り 2・対象なし 3・PDF あり 4。

**C13 Library の追加と PDF 待ちの CLI**（U12。全文）
- `sai-paper paper keep (--log <id> --position <n> | --import-id <id> | --cite <paper_id> --ref <n>) --json` → `{"paper_id","title","status":"added"|"exists"}`。
- `sai-paper library pending --json` → `{"papers":[{"id","title","year","authors","doi","arxiv_id","s2_paper_id","created_at"}]}`。

**C14 sai-prep が使う Web の住所**（U14 が使う）
- 認証: `Authorization: Bearer sai_…`（個人のトークン。U06）。
- `GET /api/auth/me` → `{"mode":"multi","name","email",…}`（トークンの確かめに使う）。
- `GET /api/projects` → 今の利用者のプロジェクトの一覧（各行の `name`）。
- `GET /api/projects/{name}/papers/pending` → C13 の `library pending` の JSON。
- `POST /api/projects/{name}/papers/bundle`（本文は zip、`Content-Type: application/zip`、300MB まで）→ 202 `{"key":"bundle:<uuid>","status":"processing"}`／400（理由）／404／413／415。
- `GET /api/projects/{name}/papers/jobs/{key}` → `{"key","status":"processing"|"done"|"failed","title","paper_id"?,"figures"?,"translated"?,"error"?}`。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/bundles.py`（新規）・`papers.py`・`main.py`（論文の住所の部分だけ）・`static/paperimport.js`・`static/papers.js`・`static/index.html`・`static/sw.js`・`apps/web/tests/test_bundles.py`（新規）・Papers の既存のテスト（消す・直す）・`docs/integration.md`
- 読むだけ: `apps/web/src/sai_web/control.py`・`registry.py`・`auth.py`
- 触らない: `crates/`・`apps/prep/`（U14）

## 完了条件
- [ ] AC1: `apps/web/tests/test_bundles.py`（偽の CLI）: 正しい zip で 202 が返り、`inbox/<uuid>/` に展開され、`sai-paper paper import-bundle --dir <そこ> --json` が呼ばれ、終わるとフォルダが消え、job が `done` になる。`../x` の名前・シンボリックリンク・`manifest.json` の無い zip・`%PDF-` で始まらない PDF・`format` の違い・300MB 超は、CLI を呼ばずに 400 / 413。`application/pdf` は 415。他人のプロジェクトは 404。
- [ ] AC2: 同じテストで、個人のトークン（Bearer）で `GET …/papers/pending` と `POST …/papers/bundle` が通る（多人数モード）。
- [ ] AC3: `POST …/papers/keep` が `sai-paper paper keep --import-id … --json` を呼ぶ。消した住所（download・manual/*）は 404 / 405。
- [ ] AC4: 8299 番のサーバで、Papers の検索結果に「＋ Library に入れる」が出て「⬇ Download」と「手動で取り込む」が出ない。PDF の無い論文の詳細に「PDF 待ち」の案内が出る。390px と 1280px の画面写真を作業報告書に付ける。
- [ ] AC5: Web のテスト（JS・Python）がすべて通る。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
```

## 決定事項と理由
- zip の検査と展開は Web が行う — 理由: 外から来る値は CLI を呼ぶ前に Web が検査する約束（AGENTS.md）。Python の `zipfile` は標準ライブラリにある。
- 展開先はプロジェクトの `.sai/paper/inbox/` — 理由: ホストとコンテナで同じパスに見え、コンテナの中の CLI がそのまま読める。ほかのプロジェクトのコンテナからは見えない。
- サーバへ PDF を直接上げる手動取り込みは消す — 理由: サーバには docling が無く、PDF だけでは前処理できない。PDF は必ず sai-prep を通す（要件）。
- sai-prep の入れ物は Web から配る — 理由: リポジトリに外部の remote が無い（GitHub に置く前提にしない）。利用者は tailnet の中で Web にログインしているので、そこから落とせばよい。

## 要確認
- なし。
