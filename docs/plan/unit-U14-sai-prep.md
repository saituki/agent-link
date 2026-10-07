# 実装計画書 U14: sai-prep（各自の Mac の PDF 前処理アプリ）

## 目的
各自の Mac（Apple Silicon）に配る小さなアプリ `sai-prep` を作る。`uv tool install` で入れ、`sai-prep` を起動すると、ブラウザで `http://127.0.0.1:8765/` の画面が開く。画面で次ができる。
1. サーバ（sai-tree の Web）の住所と個人のトークンを設定し、自分のプロジェクトを選ぶ。
2. サーバの Library で「PDF 待ち」の論文の一覧を見て、行に PDF を drop する。
3. BibTeX（1 件）と PDF を一緒に drop して、Library に無い論文を取り込む（S2 検索なし）。
4. drop した PDF を、手元で docling（md・json・図）と pdf2zh（英→日の翻訳。LLM は手元で起こす mlx-lm の translateGemma）で前処理し、「前処理の束」（zip）にしてサーバへ送り、取り込みの結果を見る。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である。docling と翻訳は重いので、サーバではなく各自の PC で行う（要件）。利用者の PC とサーバは Tailscale でつながっている。
- 道具の版と入れ方は調査 R2（`docs/plan/research/R2-sai-prep-stack.md`）にある。要点:
  - docling 2.134.0・pdf2zh 1.9.11・mlx-lm 0.32.0。**3 つは 1 つの Python の環境に同居させない**（transformers の版の重なりが狭い）。`uv tool install --python 3.12 <名前>` で別々の環境に入れ、CLI として呼ぶ。
  - docling: `docling convert <pdf> --to md --to json --image-export-mode referenced --output <dir> --device mps`。出力は `<dir>/<stem>.md`・`<stem>.json`・`<stem>_artifacts/`。初回にモデル（2〜4GB）を落とす。
  - mlx-lm: `mlx_lm.server --model mlx-community/translategemma-4b-it-4bit_immersive-translate --host 127.0.0.1 --port <番号>`。`GET /v1/models` で起動を確かめる。初回にモデル（約 2.2GB）を落とす。**素の `mlx-community/translategemma-4b-it-4bit` は `<<<source>>>` の形の文字列を受けない**ので、`_immersive-translate` 版を使う。
  - pdf2zh: `pdf2zh <pdf> -o <dir> -li en -lo ja --service openailiked --prompt <file>`。環境変数 `OPENAILIKED_BASE_URL`（`/v1` で終わる）・`OPENAILIKED_API_KEY`（`EMPTY`）・`OPENAILIKED_MODEL`。出力は `<stem>-mono.pdf`・`<stem>-dual.pdf`。
  - 16GB の Mac では、docling と mlx のサーバを同時に動かさない（docling → mlx を起こして翻訳 → mlx を止める、の順）。
- 送る束の形（C12）と Web の住所（C14）は下の共有契約にある。サーバ側は U12・U13 が作る（並列で進む）。

## 前提（着手時点のコードの状態）
- sai-tree に `apps/prep/` は無い。この作業で新しく作る。
- サーバ側（U13）はこの作業の作業用コピーには無い。確かめは偽のサーバ（標準ライブラリの `http.server`）で行う。

## 変更内容

### 1. パッケージ — `apps/prep/`
```
apps/prep/
  pyproject.toml         # name = "sai-prep"、requires-python = ">=3.11"、依存なし、[project.scripts] sai-prep = "sai_prep.cli:main"
  README.md              # 入れ方・使い方（日本語）
  src/sai_prep/
    __init__.py          # __version__ = "0.1.0"
    cli.py               # sai-prep [serve|setup|doctor]
    config.py            # 設定の読み書き
    server.py            # 標準ライブラリの http.server で画面と API を出す
    pipeline.py          # docling・mlx・pdf2zh を順に動かし、束を作る
    client.py            # サーバへの要求（urllib）
    static/index.html    # 画面（素の HTML・CSS・JS。1 ファイル）
  tests/test_*.py        # unittest
```
- **依存は標準ライブラリだけ**にする（docling・pdf2zh・mlx-lm は別の `uv tool` の環境に入れ、CLI で呼ぶ）。

### 2. 命令 — `cli.py`
- `sai-prep`（= `sai-prep serve`）: `127.0.0.1:8765`（`--port` で変更）で画面を出し、`webbrowser.open` で開く。
- `sai-prep setup`: 足りない道具を入れる。`uv tool install --python 3.12 docling`・`pdf2zh`・`mlx-lm` を、入っていない物だけ順に実行する（実行する命令を先に表示して `y/N` を聞く。`--yes` で聞かない）。最後に `doctor` を走らせる。
- `sai-prep doctor`: `docling --version`・`pdf2zh --version`・`mlx_lm.server --help` が動くか、Apple Silicon か、サーバの住所とトークンで `GET /api/auth/me` が通るかを表で出す。

### 3. 設定 — `config.py`
- 置き場: `~/.config/sai-prep/config.json`（0600）。項目: `server_url`（例 `https://sai.<tailnet>.ts.net`）・`token`（`sai_…`）・`project`（最後に選んだ名前）・`translate`（真偽。既定は真）・`mlx_model`（既定 `mlx-community/translategemma-4b-it-4bit_immersive-translate`）・`mlx_port`（既定 8766）・`docling_device`（既定 `mps`）。
- 作業の置き場: `~/Library/Caches/sai-prep/jobs/<uuid>/`（終わったら消す。失敗したときは調べられるよう 7 日残す）。

### 4. 画面と API — `server.py`・`static/index.html`
- `server.py` は `127.0.0.1` だけで待ち受ける（外から届かないように）。画面からの要求には、起動のたびに作る合言葉（`secrets.token_urlsafe(16)`）を `index.html` に埋めて付けさせ、無い要求は 403（ほかのサイトのページが localhost を叩けないように）。
- 画面（日本語。幅 390px と 1280px で崩れない）:
  1. 「設定」: サーバの住所・トークン（`type="password"`）・「確かめる」（`GET /api/auth/me` の名前を出す）。
  2. 「プロジェクト」: `GET /api/projects` の名前から選ぶ。
  3. 「PDF 待ち」: `GET /api/projects/{name}/papers/pending` の表（題名・年・DOI か arXiv）。各行に PDF を drop できる（`<input type="file" accept="application/pdf">` でも選べる）。
  4. 「BibTeX と PDF」: `.bib`（1 件。2 件以上なら「1 件だけにしてください」）と PDF を一緒に drop する場所。
  5. 「翻訳する」のチェック（設定の `translate`）。
  6. 「処理の列」: 仕事ごとに段（docling → 翻訳 → 束 → 送信 → サーバの取り込み）・経過・失敗の理由。
- 画面の API（`server.py`。JSON）: `GET/PUT /local/config`・`POST /local/check`・`GET /local/projects`・`GET /local/pending?project=`・`POST /local/jobs`（本文は multipart を使わず、`Content-Type: application/pdf` の本文と、クエリ `project`・`paper_id`、または `X-Sai-Bibtex` ヘッダ（base64 の BibTeX））・`GET /local/jobs`。

### 5. 前処理 — `pipeline.py`
仕事は 1 つずつ順に処理する（列は 1 本のスレッド）。1 つの仕事の手順:
1. PDF を作業の置き場の `paper.pdf` に置き、SHA-256 を計算する。先頭が `%PDF-` でなければ失敗。
2. docling: `docling convert paper.pdf --to md --to json --image-export-mode referenced --output docling --device <docling_device>`（上限 30 分）。`docling/paper.md`・`docling/paper.json` ができたことを確かめる。
3. 翻訳（`translate` が真のとき）:
   - mlx のサーバが動いていなければ `mlx_lm.server --model <mlx_model> --host 127.0.0.1 --port <mlx_port>` を起こし、`GET http://127.0.0.1:<port>/v1/models` が 200 になるまで待つ（初回はモデルの取得があるので最大 30 分。画面に「翻訳のモデルを準備中」と出す）。
   - プロンプトのファイル（作業の置き場の `prompt.txt`）に `<<<source>>>English<<<target>>>Japanese<<<text>>>${text}` を書く（言語は ISO コードではなく名前で書く。`_immersive-translate` 版のテンプレートが名前を前提にしているため。調査 R2）。
   - `pdf2zh paper.pdf -o translated -li en -lo ja --service openailiked --prompt prompt.txt` を、環境変数 `OPENAILIKED_BASE_URL=http://127.0.0.1:<port>/v1`・`OPENAILIKED_API_KEY=EMPTY`・`OPENAILIKED_MODEL=<mlx_model>` で呼ぶ（上限 60 分）。`translated/paper-mono.pdf`・`paper-dual.pdf` を確かめる。
   - 列が空になったら mlx のサーバを止める（メモリを空けるため）。
4. 束: C12 の形で zip を作る（`manifest.json` の `target` は、行に drop したなら `{"paper_id": <id>}`、BibTeX なら `{"bibtex": "<文字列>"}`。`tools` に各道具の版を入れる）。
5. 送信: `POST {server_url}/api/projects/{project}/papers/bundle`（`Authorization: Bearer <token>`・`Content-Type: application/zip`）。202 の `key` を取り、`GET …/papers/jobs/{key}` を 3 秒ごとに読んで `done` / `failed` を画面に出す（上限 15 分）。
6. 成功したら作業の置き場を消す。
- 道具の実行ファイルは PATH から探し、無ければ `~/.local/bin/<名前>` も見る。見つからなければ「`sai-prep setup` を実行してください」で失敗させる。
- サーバへの要求は `urllib.request`（標準ライブラリ）。`server_url` は `https://` か `http://127.0.0.1` / `http://localhost` だけを受ける（トークンを平文で外に送らないため）。

### 6. テスト — `apps/prep/tests/`
- 偽の `docling`・`pdf2zh`・`mlx_lm.server`（sh か Python の小さなスクリプト。決まった出力のファイルを作る、`/v1/models` に答える）を PATH の先頭に置く。
- 偽のサーバ（`http.server`）で C14 の住所に答え、送られた zip の中身と `Authorization` を記録する。
- テスト: 行への drop → 束の `manifest.json` が C12 の形で `target.paper_id` を持つ。BibTeX の仕事 → `target.bibtex`。翻訳を切ると `translation: null` で `translated/` が無い。mlx のサーバが起こされ、列が空になると止まる。道具が無いと「`sai-prep setup` を実行してください」。`http://` の外の住所は設定で拒まれる。localhost の画面の API は合言葉が無いと 403。

## 共有契約（他の作業と共有する取り決め。worker は変えない）

**C12 前処理の束**（U12。全文）
- zip の中身（直下からの相対パス）: `manifest.json`・`paper.pdf`・`docling/paper.md`・`docling/paper.json`・`docling/paper_artifacts/…`・（翻訳したときだけ）`translated/paper-mono.pdf`・`translated/paper-dual.pdf`。
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
  `target` は `{"paper_id": <int>}` か `{"bibtex": "<BibTeX 1 件の文字列>"}`。翻訳しなかったときは `"translation": null`。

**C14 sai-prep が使う Web の住所**（U13。全文）
- 認証: `Authorization: Bearer sai_…`（個人のトークン）。
- `GET /api/auth/me` → `{"mode":"multi","name","email",…}`。
- `GET /api/projects` → 今の利用者のプロジェクトの一覧（各行の `name`）。
- `GET /api/projects/{name}/papers/pending` → `{"papers":[{"id","title","year","authors","doi","arxiv_id","s2_paper_id","created_at"}]}`。
- `POST /api/projects/{name}/papers/bundle`（本文は zip、`Content-Type: application/zip`、300MB まで）→ 202 `{"key":"bundle:<uuid>","status":"processing"}`／400（理由）／404／413／415。
- `GET /api/projects/{name}/papers/jobs/{key}` → `{"key","status":"processing"|"done"|"failed","title","paper_id"?,"figures"?,"translated"?,"error"?}`。

## ファイルの所有権
- 書いてよい: `apps/prep/`（新規。すべて）
- 触らない: `apps/web/`・`crates/`（U12・U13 が書く）

## 完了条件
- [ ] AC1: `cd apps/prep && python3 -m unittest discover -s tests` がすべて通る（§6 のテスト）。
- [ ] AC2: `uv build` で `apps/prep/dist/sai_prep-0.1.0-py3-none-any.whl` ができ、一時の場所に `uv tool install` して `sai-prep doctor` が動く（道具が無ければ「無い」と出る）。確かめたら `uv tool uninstall sai-prep` で消す。
- [ ] AC3: 画面を 390px と 1280px で開いた画面写真（偽のサーバにつないだ状態で、PDF 待ちの一覧と処理の列が出ている物）を作業報告書に付ける。
- [ ] AC4: 実物の確認（この Mac で 1 回だけ。利用者の環境を汚さないよう、`UV_TOOL_DIR` と `UV_TOOL_BIN_DIR` を一時フォルダにして入れ、最後に消す）: 1〜2 ページの小さな英語の PDF で、docling → mlx の translateGemma → pdf2zh が通り、束ができる（送り先は偽のサーバ）。かかった時間と、`<<<source>>>English…` のプロンプトで日本語に訳されたかを作業報告書に書く。モデルの取得（合計 5GB ほど）に時間がかかるときは、最大 60 分で打ち切り、どこまで通ったかを書く。

## 検証
```bash
cd ~/workdir/Sai/sai-tree/apps/prep
python3 -m unittest discover -s tests
uv build
```

## 決定事項と理由
- ローカルの Web 画面にする — 理由: 利用者の決定。ブラウザだけでは手元の docling を動かせないので、手元で動くサーバが要る。
- 依存は標準ライブラリだけ、重い道具は別々の `uv tool` の環境 — 理由: docling・pdf2zh・mlx-lm を 1 つの環境に入れると transformers の版がぶつかる（調査 R2）。sai-prep 自体を軽く保ち、道具を個別に更新できる。
- 翻訳の LLM は mlx-lm の translateGemma（`_immersive-translate` 版）— 理由: 利用者の決定（sai-prep に mlx-lm を内蔵）。sai-moon が使う `<<<source>>>` の形のプロンプトを受けられるのはこの版だけ（調査 R2）。
- 仕事は 1 つずつ、docling と mlx を重ねない — 理由: 16GB の Mac でメモリを取り合わないため。
- localhost の画面に合言葉を付ける — 理由: ほかのサイトのページから localhost の API を叩かれ、トークンを使われるのを防ぐ。

## 要確認
- `_immersive-translate` 版で、言語名（English・Japanese）のプロンプトが期待どおりに日本語訳になるか（AC4 で確かめる）。ならなければ、公式の推奨プロンプト（調査 R2 §Q2）の形に替え、報告に書く。
- pdf2zh 1.9.11 が Python 3.12 の `uv tool` で入るか（PyPI の要求は `>=3.10,<3.13`）。
