# sai-prep

各自の Mac（Apple Silicon）で、論文の PDF を前処理して sai-tree のサーバへ送る小さなアプリです。
PDF を docling で md・json・図に変換し、pdf2zh と mlx-lm の translateGemma で英→日の翻訳を行い、
結果を 1 つの zip（「前処理の束」）にまとめてサーバへ送ります。サーバは PDF を落とさず、docling も
pdf2zh も動かしません。

## 何ができるか

ブラウザで `http://127.0.0.1:8765/` の画面が開きます。画面は左の sidebar と右の本文の 2 つに分かれます（幅が狭いときは sidebar が本文の上に出ます）。

- **Account**: 「設定」。サーバの住所と個人のトークンを入れて「保存」→「確かめる」を押す。
- **Projects**: 自分のプロジェクトの一覧（↻ で読み直す）。プロジェクトを押すと、そのプロジェクトの画面が開きます。次に開いたとき、最後に選んだプロジェクトが開きます。

プロジェクトの画面には次の 3 つの節があります。

1. **PDF 待ち**: サーバの Library で「PDF 待ち」の論文の一覧を見て、行に PDF を drop する。
2. **BibTeX と PDF**: BibTeX（1 件）と PDF を一緒に drop して、Library に無い論文を取り込む（S2 検索は行わない）。
3. **Data upload**: 解析用のデータ（任意の種類のファイル）を送り、サーバのプロジェクトの `data/raw/` に置く。
   - 複数のファイルを一度に選べます。1 件ずつ順に送り、行ごとに結果（送りました・失敗の理由）を出します。
   - 1 件 5GB まで。名前は英数字と `. _ -` だけ（先頭は英数字・128 字まで）。日本語や空白の入った名前は送れません（先に名前を変えてください）。
   - 同じ名前があれば上書きせず、失敗になります（409）。
   - 形式の変換はしません。送ったファイルは `data/raw/` に置くだけです（解析はプロジェクトのコンテナの中で行います）。
   - 下の一覧は `data/raw/` の中身（名前・大きさ・日時）です。

## 入れ方

`sai-prep` 自体は標準ライブラリだけで動きます。重い道具（docling・pdf2zh・mlx-lm）は
**別々の `uv tool` の環境**に入れます。1 つの環境に同居させると `transformers` の版がぶつかります。

```bash
# uv が無ければ入れる（https://docs.astral.sh/uv/）
uv tool install --python 3.12 sai-prep
# 足りない道具を入れる（入れる命令を表示して y/N を聞きます）
sai-prep setup
```

`sai-prep setup` は、入っていない物だけを次の順に入れます。

```bash
uv tool install --python 3.12 docling
uv tool install --python 3.12 pdf2zh
uv tool install --python 3.12 mlx-lm
```

初回はモデルの取得があります（docling 2〜4GB、translateGemma 約 2.2GB、pdf2zh のアセット数百 MB）。

## 使い方

```bash
sai-prep            # 画面を開く（= sai-prep serve）
sai-prep doctor     # 道具とサーバのつながりを確かめる
sai-prep setup      # 足りない道具を入れる
```

画面で「設定」を開き、サーバの住所（例 `https://sai.<tailnet>.ts.net`）と、アカウント画面で作った個人のトークン
（`sai_…`）を入れて「保存」→「確かめる」を押します。sidebar の Projects からプロジェクトを選ぶと、その画面が開きます。

## 置き場

- 設定: `~/.config/sai-prep/config.json`（0600）
- 作業の置き場: `~/Library/Caches/sai-prep/jobs/<uuid>/`（成功したら消します。失敗したときは調べられるよう残します）

## 仕組み

- 仕事は 1 つずつ順に処理します（16GB の Mac で docling と mlx のサーバがメモリを取り合わないように）。
- 翻訳するときだけ mlx のサーバを起こし、列が空になったら止めます。
- 束の形（`manifest.json`）は共有契約 C12 のとおりです。
- 道具の実行ファイルは `PATH` の次に `~/.local/bin` から探します。無いときは
  「`sai-prep setup` を実行してください」と出ます。

### agent からの取り込み（zip）

- `POST /local/jobs?project=…&paper_id=…` に `Content-Type: application/zip` で送ると、
  zip の仕事になります。中身は `paper.pdf`（必須）と、任意の `translated/paper-mono.pdf`・
  `translated/paper-dual.pdf`（両方あるか両方無いか）です。
- zip の仕事は pdf2zh を動かしません。翻訳が入っていれば束の `translation.service` は
  `provided` になり、無ければ翻訳なしになります。
- 合言葉は画面の HTML の `var NONCE = "…";` から読みます（`GET /` に付いています）。

## テスト

```bash
cd apps/prep
python3 -m unittest discover -s tests
```
