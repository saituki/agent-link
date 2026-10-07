# 実装ガイド 5/5: 論文の前処理（sai-prep と束の取り込み）・配備・文書

## ゴール
Papers の検索で見つけた論文を「Library に入れる」と、PDF の無い論文として Library に入る。各自の Mac の sai-prep の画面にその論文が「PDF 待ち」として出て、PDF を drop すると、手元で docling と翻訳（mlx-lm の translateGemma）を行い、束を送り、サーバの Library で PDF・翻訳 PDF・図・節・引用が見られる。BibTeX と PDF を一緒に drop すれば、S2 検索なしで Library に入る。サーバは PDF を落とさず、docling・pdf2zh も動かさない。Mac から `deploy.sh` で Linux へ反映でき、CI は Linux で回る。文書が sai-tree の仕組みに合っている。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U12 | 束の取り込み・PDF 無しの Library 追加・PDF 待ち（sai-paper） | `docs/plan/unit-U12-paper-bundle-core.md`（waggle:1wtca6kw） | `crates/sai-paper-core/src/`（`bundle.rs` ほか）・`crates/sai-paper-cli/src/` | ガイド 1〜4 | 可 |
| U13 | 束の受け口・「Library に入れる」・PDF 待ちの表示（Web） | `docs/plan/unit-U13-paper-bundle-web.md`（waggle:fgUhuwKi） | `bundles.py`（新規）・`papers.py`・`main.py`（論文の住所）・`static/paperimport.js`・`papers.js`・`index.html`・`sw.js`・テスト・`docs/integration.md` | ガイド 1〜4 | 可 |
| U14 | sai-prep（Mac の前処理アプリ） | `docs/plan/unit-U14-sai-prep.md`（waggle:HG2HaDah） | `apps/prep/`（新規） | ガイド 1〜4 | 可 |
| U15 | 配備・CI・文書の書き直し | `docs/plan/unit-U15-deploy-docs.md`（waggle:HDwNGPHd） | `apps/web/server/deploy.sh`・`docs/server.md`（節を足す）・`.github/workflows/ci.yml`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`docs/`・`sai-sci/blocks/`・`sai-sci/template/`（一部） | U12・U13・U14 | 不可 |

## 実行順序
```
U12 ─┐
U13 ─┼─▶ U15
U14 ─┘
```
- U12 ∥ U13 ∥ U14（書くファイルが重ならない。U13 は U12 の CLI（C12・C13）を偽の CLI で、U14 は U13 の住所（C14）を偽のサーバで確かめる）。
- U15 は全部の後（文書が全部の結果を書く）。

## 所有権の表
| ファイル | U12 | U13 | U14 | U15 |
|---|---|---|---|---|
| `crates/sai-paper-core/`・`crates/sai-paper-cli/` | 書く | - | - | - |
| `apps/web/src/sai_web/`・`apps/web/tests/` | - | 書く | - | - |
| `apps/prep/` | - | - | 書く | 読む |
| `apps/web/server/deploy.sh`・`docs/server.md`・`.github/`・`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`docs/`（`docs/plan/` を除く）・`sai-sci/blocks/`・`sai-sci/template/` | - | `docs/integration.md` だけ | - | 書く ※U13 の後 |

## 共有契約
- C12 前処理の束（zip の中身と `manifest.json`。U12・U13・U14 の計画書に全文）。
- C13 Library の追加と PDF 待ちの CLI（U12 が作り U13 が呼ぶ。両方に全文）。
- C14 sai-prep が使う Web の住所（U13 が作り U14 が使う。両方に全文）。個人のトークンはガイド 3 の U06 が作った。

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| 束から図・節・引用・翻訳が、docling・pdf2zh を呼ばずに入る | U12 | U12 の AC1・AC2 |
| PDF 無しで Library に入れられ、PDF 待ちに出る | U12・U13 | U12 の AC3、U13 の AC3・AC4 |
| 束の受け口が危ない zip を CLI に渡さない | U13 | U13 の AC1 |
| sai-prep が束を作って送れる | U14 | U14 の AC1・AC4 |
| 配備・CI・文書がそろう | U15 | U15 の AC1〜AC4 |
| 端から端まで動く | 全員 | 下の「統合と検証」 |

## 統合と検証
- U12 → U13 → U14 → U15 の順にマージし、毎回 AGENTS.md の「テストと確認」をすべて回す（U15 のマージ後は sai-prep のテストも含む）。
- U12・U13・U14 のマージ後（端から端まで）: ガイド 4 の確認と同じ Mac の Docker の多人数モードの 8299 番で、
  1. 利用者を作り、プロジェクトを作り、Papers で論文を 1 本「Library に入れる」（S2 の検索は Mac の環境の `SEMANTICSCHOLAR_API_KEY` を `PUT /api/me/keys` で入れて行う。無ければ BibTeX の取り込みで代える）。
  2. アカウント画面で個人のトークンを作る。
  3. U14 の AC4 で入れた一時の sai-prep（`UV_TOOL_DIR` を一時フォルダにした物）を 8299 番に向け、PDF 待ちの論文に小さな PDF を drop し、取り込みまで通す。BibTeX と PDF の drop も 1 回通す。
  4. Papers の論文の詳細で、PDF・翻訳 PDF・図・節が見られることを画面写真で確かめる。
  - 終わったら確認用のコンテナ・イメージ・一時フォルダ・一時の sai-prep を消す。
- `cargo build --release` を本体フォルダで行い、3 つの実行ファイルを作り直す。
- 最後の kantoku として、利用者が Linux の実機で確かめる手順（`docs/server.md` の入れ方 → 管理者の作成 → プロジェクト → チャット → sai-prep での取り込み → `deploy.sh`）を完了報告書にまとめる。

## 決定事項と理由 / 採用しなかった案
- Library の S2 検索とメタデータはサーバ、PDF 本体は sai-prep から送る。BibTeX と PDF の drop でも取り込める（利用者の決定）。
- サーバでは docling・pdf2zh を動かさず、PDF も落とさない（要件）。送られた docling の md・json から、Rust の軽い処理で図・節・引用を作る。
- 束の展開は Web（Python の `zipfile`）、CLI はフォルダを受ける。/ Rust に zip の依存を足す案は、依存を増やさないため採らない。
- sai-prep はローカルの Web 画面（利用者の決定）。依存は標準ライブラリだけで、docling・pdf2zh・mlx-lm は別々の `uv tool` の環境（調査 R2）。
- 翻訳の LLM は sai-prep が起こす mlx-lm の translateGemma（`_immersive-translate` 版）。dgx の vLLM は使わない（利用者の決定）。
- sai-prep の入れ物は Web から配る（外部の remote が無いため）。
- 配備は bare リポジトリへの push と ssh の 1 命令。GitHub に置くかは利用者が決める。

## リスクと対処
- 束の取り込みで、`ensure_extract` の S2 の参考文献の取得が遅い: 引用の作成を後回し（`after-download` 相当）に分けてよい（kantoku が決めてよい）。
- mlx の translateGemma が言語名のプロンプトで日本語にならない: U14 の要確認のとおり、公式の推奨プロンプトに替えてよい（kantoku が決めてよい）。
- モデルの取得（合計 5GB ほど）で実物の確認が長引く: 60 分で打ち切り、どこまで通ったかを報告に書く。偽の道具のテストが通っていれば先へ進んでよい。
- 消すテスト（Download・手動取り込み）が多い: 消したテストの名前を報告に書く。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（waggle:y7EoLAgV）
- 調査 E1（前処理が作る物と取り込みの経路）: `docs/plan/research/E1-paper-preprocess.md`
- 調査 E2（束と BibTeX の取り込み）: `docs/plan/research/E2-bundle-import.md`
- 調査 R2（sai-prep の道具）: `docs/plan/research/R2-sai-prep-stack.md`
- 調査 E3（文書の中の Mac 専用の記述）: `docs/plan/research/E3-mac-only-removal.md`
