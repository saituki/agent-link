---
block: web-research
run: 442
description: 調査報告書 R2 (r2): Mac の sai-prep に docling・pdf2zh・mlx-lm の translateGemma を入れる方法
---

# ウェブ調査報告 R2: Mac の sai-prep に docling・pdf2zh・mlx-lm の translateGemma を入れる方法

- 確かめた日: 2026-10-07（すべてこの日付で読んだ版に基づく）。
- 対象: Apple Silicon の Mac（想定 16GB）に配る小さなアプリ `sai-prep`。
- この報告書は調査だけを行う。このプロジェクトのファイルは書き換えていない。
- r2 の変更点: mlx サーバの起動例から誤った `--chat-template-args`（`add_generation_prompt` はサーバが既に付けるので二重指定になり得る）を削除。誤字を修正。本文の内容は r1 と同じ。

## 前提: いまの sai-moon の呼び方（コードで確認）

- `crates/sai-paper-core/src/translation/pdf2zh.rs:129` の `translategemma_prompt_template()` は
  `<<<source>>>${lang_in}<<<target>>>${lang_out}<<<text>>>${text}` を返す。
- 同ファイルの `run_pdf2zh` は `pdf2zh <pdf> -o <dir> -li <in> -lo <out> --service openailiked [--prompt <file>]` を呼び、
  `OPENAILIKED_BASE_URL`・`OPENAILIKED_API_KEY`・`OPENAILIKED_MODEL` を子プロセスに渡す。
- 出力は `<stem>-mono.pdf` / `<stem>-dual.pdf` を前提にしている（同ファイルの `run_pdf2zh`）。
- 3 つとも CLI を持つので、`sai-prep` は **import せず subprocess で呼ぶ**形にすれば依存の衝突を避けられる。

## 問いと答え

### Q1: mlx-lm の OpenAI 互換サーバの起動方法と、Python から起こす方法

- 答え: 版は **mlx-lm 0.32.0（PyPI, 2026-10-01）**、要求 Python **>=3.11**。依存は `mlx>=0.32.2`（Darwin）、`numpy`、`transformers>=5.7.0`。
  起動は `mlx_lm.server --model <hf-repo-or-path>`。既定は **host=127.0.0.1 / port=8080**。
  `/v1/chat/completions` と `/v1/models` に対応する。
- 出典: https://pypi.org/project/mlx-lm/ 、https://github.com/ml-explore/mlx-lm/blob/main/mlx_lm/SERVER.md 、
  https://github.com/ml-explore/mlx-lm/blob/main/mlx_lm/server.py 、https://github.com/ml-explore/mlx-lm/blob/main/pyproject.toml （2026-10-07 確認）
- 主な起動オプション（`mlx_lm/server.py` の `main()`）:
  `--model` / `--host`（既定 127.0.0.1）/ `--port`（既定 8080）/ `--chat-template` / `--chat-template-args` /
  `--temp`（既定 0.0）/ `--top-p`（1.0）/ `--top-k`（0）/ `--min-p`（0.0）/ `--max-tokens`（既定 512）/
  `--prefill-step-size`（既定 2048）/ `--kv-bits` / `--trust-remote-code` / `--draft-model`（投機的デコード）。
  `add_generation_prompt=True` はサーバが内部で付けるので、`--chat-template-args` で渡さないこと。
- 対応エンドポイント:
  - `POST /v1/chat/completions`（`server.py:1106`）。
  - `GET /v1/models`（`server.py:1649`）。`huggingface_hub.scan_cache_dir()` で **ローカルにダウンロード済みの MLX モデル**を列挙して返す（`server.py:1696-1724`）。読み込み中の 1 つだけを返すわけではない。
- リクエスト: `messages`（role/content）、`model`（任意。ローカルパスは起動ディレクトリ相対）、`max_tokens`（既定 512）、`temperature`、`stream`、`stop` など（SERVER.md）。
- Python から起こす方法: `server.py` は `if __name__ == "__main__": main()` を持つが、直接呼ぶと
  「Calling `python -m mlx_lm.server...` directly is deprecated. Use `mlx_lm.server...` or `python -m mlx_lm server ...` instead.」と出る（`server.py:1947-1951`）。
  console script は `mlx_lm.server = mlx_lm.server:main`（`pyproject.toml:62`）。**別プロセスとして `subprocess.Popen(["mlx_lm.server", ...])` で起こすのが素直**。
  同じプロセス内に読み込む公開 API（サーバを非同期で回す関数）は文書化されていない（要確認）。
- 重要（この案件に効く事実）: `server.py:543` で `process_message_content(messages)` が走り、**content が list のときは text 断片を連結した 1 本の文字列に潰してから** `apply_chat_template` する（`server.py:560`）。
  つまり OpenAI 互換 API 経由では、TranslateGemma 公式の「list of dicts（`source_lang_code`/`target_lang_code`/`text`）」は形が壊れる。
  文字列 `<<<source>>>...` を受けて展開する chat template を持つモデル（後述の immersive-translate 版）なら通る。
- 注意: SERVER.md のとおり「not recommended for production as it only implements basic security checks」。localhost 専用で使う。

### Q2: translateGemma の MLX 版はあるか。名前・量子化・大きさ・プロンプト・規約

- 答え: **ある**。Hugging Face の `mlx-community` に 4B/12B/27B の it 版が 4bit・8bit で揃っている（作成 2026-01-15、`mlx-lm 0.29.1` で変換、license は `gemma`、`gated=false`）。
- 主なリポジトリと実測サイズ（Hugging Face API `?blobs=true` より、2026-10-07）:
  | リポジトリ | 合計 |
  |---|---|
  | `mlx-community/translategemma-4b-it-4bit` | 2.22 GB（model.safetensors 2183 MB） |
  | `mlx-community/translategemma-4b-it-8bit` | 4.16 GB |
  | `mlx-community/translategemma-12b-it-4bit` | 6.66 GB |
  | `mlx-community/translategemma-27b-it-4bit` | （未計測。27B 4bit は約 14 GB 前後とみられる＝推測） |
  出典: https://huggingface.co/mlx-community/translategemma-4b-it-4bit 、https://huggingface.co/api/models?search=translategemma&author=mlx-community
- **この案件の鍵**: `mlx-community/translategemma-4b-it-4bit_immersive-translate`（作成 2026-04-18、2.22 GB）がある。
  これは `mlx-community/translategemma-4b-it-4bit` を元に、**Immersive Translate が送る `<<<source>>>...` の文字列を chat template で解釈して公式の「professional translator」指示へ展開する**よう作り直したもの。
  EOS も `[106, 1]` に直してある。
  出典: https://huggingface.co/mlx-community/translategemma-4b-it-4bit_immersive-translate
- 事実: **sai-moon が使っている `<<<source>>>` プロンプトは、この immersive-translate 版のための形式である。**
  素の `mlx-community/translategemma-4b-it-4bit` の chat template は公式テンプレートで、`content` を
  `{"type":"text","source_lang_code":..,"target_lang_code":..,"text":..}` の list（ちょうど 1 要素）で要求する。
  文字列を渡すと `jinja2.exceptions.TemplateError: User role must provide 'content' as an iterable with exactly one item...` になる。
  出典: https://github.com/ml-explore/mlx-lm/issues/783 、https://huggingface.co/google/translategemma-4b-it
- 公式モデルカードの推奨プロンプト（Figure 3 由来。ollama も同文を掲載）:
  ```
  You are a professional {SOURCE_LANG} ({SOURCE_CODE}) to {TARGET_LANG} ({TARGET_CODE}) translator. Your goal is to accurately convey the meaning and nuances of the original {SOURCE_LANG} text while adhering to {TARGET_LANG} grammar, vocabulary, and cultural sensitivities.
  Produce only the {TARGET_LANG} translation, without any additional explanations or commentary. Please translate the following {SOURCE_LANG} text into {TARGET_LANG}:

  {TEXT}
  ```
  （訳すテキストの前に空行が 2 つ）。
  出典: https://ollama.com/library/translategemma 、https://huggingface.co/google/translategemma-4b-it
- 変換手順（自分で作る場合）: `mlx_lm.convert --model google/translategemma-4b-it -q`（Python API は `from mlx_lm import convert`）。
  ただし元の `google/translategemma-4b-it` は **HF 上で gated（Google の利用規約への同意が要る）**。
  出典: https://raw.githubusercontent.com/ml-explore/mlx-lm/main/README.md 、https://huggingface.co/google/translategemma-4b-it
- gemma の利用規約: `google/translategemma-4b-it` は同意必須（ログインして承諾）。`mlx-community` の写しは `gated=false`（同意画面なしで落とせる）だが、**license は `gemma` のままで、利用には Gemma の規約が及ぶ**。
- 注意（要確認）: pdf2zh は `-li en -lo ja` の値を `${lang_in}`/`${lang_out}` に差し込むので、`<<<source>>>en<<<target>>>ja<<<text>>>...` になる。
  immersive-translate 版の例は `<<<source>>>English<<<target>>>French<<<text>>>...` と言語**名**だった。ISO コード（`en`/`ja`）でも template が名前へ展開するかは未確認。

### Q3: pdf2zh（PDFMathTranslate）の現行版・パッケージ名・入れ方・openailiked・モデル

- 答え（安定版・PyPI）: パッケージ名 **`pdf2zh`**、版 **1.9.11（PyPI, 2025-07-11）**、要求 Python **>=3.10,<3.13**。
  `uv tool install --python 3.12 pdf2zh` または `pip install pdf2zh` で入る。CLI は `pdf2zh`、GUI は `pdf2zh -i`（ポート 7860）。
  出典: https://pypi.org/project/pdf2zh/ 、https://github.com/PDFMathTranslate/PDFMathTranslate
- 出力: `<stem>-mono.pdf` と `<stem>-dual.pdf`。sai-moon の前提と一致する（v1 系の規約）。
- `openailiked` サービスの環境変数: `OPENAILIKED_BASE_URL`・`OPENAILIKED_API_KEY`・`OPENAILIKED_MODEL`
  （+ 任意で `OPENAILIKED_STOP_TOKENS`・`OPENAILIKED_MAX_TOKENS`）。
  **`BASE_URL` は `/v1` で終わる必要がある**（例 `http://127.0.0.1:8080/v1`）。終わらないと 404。
  出典: https://github.com/PDFMathTranslate/PDFMathTranslate/blob/main/docs/ADVANCED.md
- プロンプトの渡し方: `pdf2zh <pdf> --prompt <file>`。ファイルは Python の `string.Template` として読まれ、`${lang_in}`・`${lang_out}`・`${text}` を差し込む。
  **システムプロンプトは今は非対応**。出典: 同 ADVANCED.md
- 初回に落とすモデル: レイアウト解析の `wybxc/DocLayout-YOLO-DocStructBench-onnx`（`doclayout_yolo_docstructbench_imgsz1024.onnx` が **75 MB**、`gated=false`）。
  BabelDOC のフォント等のアセットは `funstory-ai/BabelDOC-Assets`（main で 190 ファイル・合計 **約 319 MB**。実行時に必要な分だけ落ちるので実ダウンロード量はこれ以下＝要確認）。
  キャッシュは `~/.cache/babeldoc` と `~/.cache/huggingface`。
  出典: https://huggingface.co/wybxc/DocLayout-YOLO-DocStructBench-onnx 、https://github.com/funstory-ai/BabelDOC-Assets
- 依存（PyPI メタデータ）: `babeldoc<0.3.0,>=0.1.22`・`onnx`・`onnxruntime`・`opencv-python-headless`・`numpy`・`huggingface-hub`・`pymupdf<1.25.3`・`pikepdf`・`gradio<5.36` など。**torch と transformers は依存に無い**。
  1.9.11 が使う babeldoc は 0.2.9 で、`numpy>=2.0.2`・`pymupdf>=1.25.1`・`onnxruntime>=1.16.1` を要求。
- 版の注意（事実）: GitHub の main README には 2026-10 時点の新機能（`--ultrafast`・OCR・`--mode precise`）が載っているが、**PyPI で入る `pdf2zh` は 1.9.11 のままで、それらは入っていない**（リリースは v1.9.11 が最新）。
  活発に開発されている後継は別パッケージ **`pdf2zh-next` 2.9.0（PyPI, 2026-05-15、Python >=3.10,<3.14、BabelDOC ベース）**。
  sai-moon の呼び方（`-o` / `-li` / `-lo` / `--service openailiked` / `-mono.pdf`）は v1 系なので、**`pdf2zh` 1.9.11 を使うのが噛み合う**。
  出典: https://pypi.org/project/pdf2zh-next/ 、https://github.com/PDFMathTranslate/PDFMathTranslate/releases
- 旧称・別名: v2 は「PDFMathTranslate-next」だが、sai-prep で使う分には v1 の `pdf2zh` でよい（要判断）。

### Q4: docling の現行版・MPS・CLI・モデルの大きさ

- 答え: 版 **docling 2.134.0（PyPI, 2026-10-06）**、要求 Python **>=3.10,<4.0**。`uv tool install docling` または `pip install docling`。
  出典: https://pypi.org/project/docling/
- CLI: `docling convert <source> --to md --to json --image-export-mode referenced` は**今も使える**。
  `--to` は `md,json,yaml,html,html_split_page,text,doctags,chunks,latex,doclang,dclx,vtt` を受け付ける。
  `--image-export-mode` は `placeholder` / `embedded`（既定）/ `referenced`。`referenced` は図を PNG で書き出し、md/json/html から参照する。
  主なオプション: `--output`（既定 `.`）/ `--device auto|cpu|cuda|mps|xpu`（既定 auto）/ `--num-threads`（既定 4）/ `--page-batch-size`（既定 4）/ `--ocr`/`--no-ocr`（既定 true）/ `--tables`/`--no-tables`（既定 true）/ `--pipeline` など。
  出典: https://github.com/docling-project/docling/blob/main/docs/reference/cli.md
- Apple Silicon: `--device auto` は CUDA → MPS → CPU の順に選ぶ。MPS はサポートされている（`--device mps` でも可）。
  速度の目安（独立フォーク `docling-mps` のベンチ。公式ではない点に注意）: 156 ページの財務 PDF・TableFormer accurate で
  CPU 約 102〜121 秒 に対し **MPS 約 50〜61 秒（約 2 倍）**。
  出典: https://docling.org/config-generator/ 、https://pypi.org/project/docling-mps/
- 初回に落とすモデル: レイアウト・表構造・OCR のモデルで**約 2〜4 GB**。既定の置き場は `$HOME/.cache/docling/models`。`--artifacts-path` / 環境変数 `DOCLING_CACHE_DIR` で変えられる。事前取得は `docling-tools models download --all`。
  出典: https://docling.org/install/ 、CLI リファレンスの `docling-tools models download`（既定 `-o $HOME/.cache/docling/models`）

### Q5: docling・pdf2zh・mlx-lm を 1 つの Python 環境に同居できるか

- 結論: **技術的には Python 3.12 で 1 環境にまとめられる見込みがある。ただし transformers の重なりが狭く、おすすめしない。**
- Python の版: docling >=3.10 / mlx-lm >=3.11 / pdf2zh >=3.10,<3.13 → **共通は 3.11 か 3.12（3.12 を推奨）**。
- 依存の突き合わせ（PyPI メタデータ、2026-10-07）:
  | 項目 | mlx-lm 0.32.0 | docling 2.134.0（standard） | pdf2zh 1.9.11（+babeldoc 0.2.9） |
  |---|---|---|---|
  | transformers | `>=5.7.0` | `docling-ibm-models` 経由で `>=4.42.0,<6.0.0`（`all`/`vlm` extra はさらに 5.9〜5.15 を除外） | 不要 |
  | numpy | なし（無指定） | `>=1.24.0,<3.0.0` | babeldoc が `>=2.0.2` |
  | onnxruntime | なし | `rapidocr`（standard 同梱）経由 | `>=1.16.1` |
  | torch | なし | `torch>=2.2.2`（standard 同梱） | なし |
  | pymupdf | なし | なし | `<1.25.3`（babeldoc は `>=1.25.1`） |
  - **唯一の緊張点は transformers**: mlx-lm が `>=5.7.0`、docling 側が `<6.0.0` かつ（`all` extra では）5.9〜5.15 を除外。
    両方を満たすのは **5.7.x か 5.8.x という狭い窓だけ**。ここが崩れると 1 環境は壊れる。バージョンは日付で動くので将来も安全とは言えない。
  - numpy 2.x・onnxruntime・torch は衝突しない。pdf2zh/babeldoc は torch も transformers も使わない。
- 重さ: docling（torch + torchvision + docling-ibm-models + rapidocr）、mlx-lm（mlx + transformers）、pdf2zh（onnx + opencv + pymupdf + babeldoc）を 1 環境に置くと数 GB の巨大環境になり、更新のたびに他が壊れやすい。
- 出典: 上記 PyPI メタデータ（`https://pypi.org/pypi/<pkg>/json` の `requires_dist`）、https://pypi.org/project/docling-slim/ 、https://pypi.org/project/babeldoc/ 、https://pypi.org/project/mlx-lm/

### Q6: 16GB の Mac で 3 つを順に動かしたときのメモリと時間の目安

- 方針: **同時に走らせず、順に**。docling（torch）と mlx サーバ（MLX）は unified memory を取り合うので重ねない。
- ディスク（実測できた値）: docling モデル 2〜4 GB + BabelDOC アセット（75 MB + フォント、実ダウンロードは数百 MB 以内）+ translateGemma 4b-4bit 2.22 GB ≒ **おおよそ 5〜7 GB**。
- docling: モデルが 2〜4 GB。実行時のピーク RAM は数 GB（推測）。時間は MPS で 156 ページが約 50〜60 秒（上記ベンチ）。論文 1 本（10〜20 ページ）ならウォーム後は **数秒〜20 秒程度（推測）**。初回はモデル取得とウォームで数分。
- pdf2zh: レイアウト ONNX 75 MB。ピーク RAM は 1〜2 GB 程度（推測）。翻訳時間は後ろの LLM サーバの速度と `-t`/QPS（既定 4）で決まる。
- translateGemma 4b-4bit（MLX）: 重み 2.22 GB。RSS は KV 込みで 3〜4 GB 程度（推測）。速度はチップ依存で、4B 4bit なら M1/M2 でも概ね **毎秒数十トークン（推測）**。1 段落は短いので 1 段落あたり 1 秒未満が期待できる（推測）。
- 16GB の見立て: 3 つを順に（mlx サーバを翻訳中だけ起こす形で）なら収まる。mlx サーバを常駐させたまま docling を回すと 2〜4 GB を常時占有し、docling のピークと重なると苦しい（推測）。

## 比べた案: 1 環境か、別々か

| 案 | できること | 弱いところ | 出典 |
|---|---|---|---|
| A. 1 つの venv に 3 つ同居 | import で直接使える。1 回の `uv sync` で入る | transformers が 5.7.x/5.8.x の狭い窓だけ。torch+mlx+onnx で数 GB。どれかの更新で全体が壊れやすい | Q5 の PyPI メタデータ |
| B. `uv tool install` で別々（推奨） | uv がツールごとに独立 venv を作るので衝突ゼロ。CLI で呼べる。個別更新できる | 環境が 3 つに分かれる（sai-prep から見れば subprocess 呼び出しなので影響なし） | https://pypi.org/project/pdf2zh/ の `uv tool install` 例、BabelDOC README も `uv tool install --python 3.12 BabelDOC` を推奨 |

## このプロジェクトへの当てはめ

- sai-moon は既に pdf2zh を **CLI（subprocess）で呼ぶ**設計（`crates/sai-paper-core/src/translation/pdf2zh.rs` の `run_pdf2zh`、`resolve_pdf2zh_bin`）。
  sai-prep も同じ形にすれば、重い 3 つを import する必要がなく、**案 A の衝突を最初から回避できる**。
- いちばん注意が要るのは **プロンプトとモデルの組み合わせ**である。
  sai-moon の `<<<source>>>` テンプレートは **`..._immersive-translate` 版の MLX モデル専用**で、公式 `translategemma` の chat template では動かない（Q2）。
  `sai-prep` の設定で `translate.model`（= `OPENAILIKED_MODEL`）を `mlx-community/translategemma-4b-it-4bit_immersive-translate` にする必要がある。
- 5.8.x の transformers を選ぶ案 A は「今だけ通る」可能性が高い。依存を増やさない方針（AGENTS.md）にも、案 B のほうが合う。
- pdf2zh の `${lang_in}`/`${lang_out}` は ISO コード（`en`/`ja`）になる。immersive-translate 版の例は言語名だったので、コードで通るかは**実機で 1 回確かめる**こと（Q2 の要確認）。

## 推奨の入れ方と起動の命令

### 入れ方（推奨 = 別々の `uv tool` 環境）

```bash
# Python 3.12 を uv が用意する。3 つはそれぞれ独立した venv に入る。
uv tool install --python 3.12 pdf2zh
uv tool install --python 3.12 docling
uv tool install --python 3.12 mlx-lm
# sai-prep 本体（軽い。重い依存は持たせない）
uv tool install --python 3.12 sai-prep
```

- `uv tool install` は既定でツールごとに環境を分ける。これで transformers の衝突を考えなくてよい。
- どうしても 1 環境にしたいときだけ、`uv venv --python 3.12` を作って
  `transformers>=5.7.0,<5.9` を明示して 3 つを入れる（**非推奨**。5.9〜5.15 は docling 側が除外している）。

### 起動の命令の例

1) translateGemma の OpenAI 互換サーバ（翻訳する間だけ起こす）

```bash
mlx_lm.server \
  --model mlx-community/translategemma-4b-it-4bit_immersive-translate \
  --host 127.0.0.1 --port 8080
# 初回はモデル（約 2.2 GB）を HF から取得する
# 確認: curl http://127.0.0.1:8080/v1/models
```

2) pdf2zh で英→日翻訳（sai-moon と同じ形。`/v1` を忘れない）

```bash
export OPENAILIKED_BASE_URL="http://127.0.0.1:8080/v1"
export OPENAILIKED_API_KEY="EMPTY"
export OPENAILIKED_MODEL="mlx-community/translategemma-4b-it-4bit_immersive-translate"
printf '<<<source>>>${lang_in}<<<target>>>${lang_out}<<<text>>>${text}\n' > /tmp/tg-prompt.txt
pdf2zh input.pdf -o outdir -li en -lo ja --service openailiked --prompt /tmp/tg-prompt.txt
# 出力: outdir/input-mono.pdf, outdir/input-dual.pdf
```

3) docling で md・json・図へ変換（MPS を使う）

```bash
docling convert input.pdf \
  --to md --to json \
  --image-export-mode referenced \
  --output outdir --device mps
# 初回はモデル（約 2〜4 GB）を ~/.cache/docling/models に取得する
```

### 順番（16GB 向け）

1. docling で変換 → 2. mlx サーバを起こして pdf2zh で翻訳 → 3. mlx サーバを止める。
   ピークを重ねない。mlx サーバを常駐させるのは、翻訳を何本も続けるときだけにする。

## 分からなかったこと

- immersive-translate 版の template が ISO コード（`en`/`ja`）を言語名へ展開するか。実機で `curl /v1/chat/completions` に `<<<source>>>en<<<target>>>ja<<<text>>>Hello` を送って確かめるのが早い。
- pdf2zh 1.9.11 + babeldoc 0.2.9 が初回に実際に落とすアセットの総量（BabelDOC-Assets 全体は約 319 MB だが、実行時に必要な分だけのはず）。`babeldoc --warmup` 相当か `~/.cache/babeldoc` を見れば分かる。
- 16GB Mac での translateGemma 4b-4bit の実測トークン/秒と、docling の実測ピーク RAM。チップ（M1/M2/M3/M4、無印/Pro）で大きく違う。実機で `mlx_lm.benchmark` と `/usr/bin/time -l` を 1 回走らせれば確定する。
- mlx-lm を同一プロセス内で非同期に回す公開 API の有無（文書化が見当たらない）。当面は subprocess で起こす前提で問題ない。
- `pdf2zh-next`（v2, 2.9.0）の CLI 名と、sai-moon の v1 前提の引数・出力名がそのまま使えるか。移るなら別途確認が要る。

