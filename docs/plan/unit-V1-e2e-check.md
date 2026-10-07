# V1: 論文取り込みの端から端までの確認

あなたの担当は V1「論文取り込みの端から端までの確認」である。作業の場所はこのリポジトリ（sai-tree の作業用コピー。U12〜U15 が全部マージ済みの状態）。コードは原則直さず、確認して報告する。直す必要のある不具合を見つけたら、最小の修正をコミットし、報告書に書く。

先に読む: `docs/plan/guide-5.md`（「統合と検証」）・`docs/plan/guide-4.md` の 8299 番の確認の項・`docs/plan/unit-U08-container-image.md` と `unit-U09-projects-containers.md` の確認手順・`docs/plan/unit-U13-paper-bundle-web.md`・`unit-U14-sai-prep.md`・`apps/prep/README.md`・`docs/server.md`。AGENTS.md の約束に従う（8200・8100・8000 番は触らない。サーバとブラウザには時間の上限を付け、終わったら必ず止める）。

## 手順（Mac の Docker の多人数モードの 8299 番）
1. `docker build -f apps/web/server/Dockerfile -t sai-tree:check .`（上限 60 分）。ホーム直下の一時フォルダを `SAI_WEB_USERS_ROOT` にし、`SAI_LINK_BIN`・`SAI_PAPER_BIN`・`SAI_SCI_BIN` を release の実行ファイル（本体フォルダでなく、この作業用コピーで `cargo build --release` した物）に向け、`SAI_IMAGE=sai-tree:check` で 8299 番の多人数モードの Web を起こす（ガイド 4 の U09 の AC5 と同じ起こし方）。
2. 利用者を作り、ログインし、プロジェクトを作る。Papers で論文を 1 本「Library に入れる」（S2 検索は Mac の環境の `SEMANTICSCHOLAR_API_KEY` を `PUT /api/me/keys` で入れて行う。無い・使えないときは BibTeX の取り込みで代える。どちらにしたか報告に書く）。
3. アカウント画面で個人のトークンを作る。
4. U14 の AC4 の入れ方で、一時の sai-prep（`UV_TOOL_DIR`・`UV_TOOL_BIN_DIR` を一時フォルダにして、Web の配布 `GET /api/prep/download` の wheel などから入れる）を 8299 番に向け、PDF 待ちの論文に小さな PDF を drop し、取り込みまで通す。BibTeX と PDF の drop も 1 回通す。docling・mlx-lm・pdf2zh は Mac にすでに入っている物を使ってよい（`uv tool list`）。モデルの取得で 60 分を超えるなら打ち切り、どこまで通ったかを書く。
5. Papers の論文の詳細で、PDF・翻訳 PDF・図・節が見られることを、playwright で 390px と 1280px の画面写真にして `docs/plan/evidence-5/` に `v1-*.png` として保存する。
6. 終わったら、確認用のコンテナ（`docker ps -a` で sai-tree:check や確認用に起こした物だけ。c_caddy・c_mlflow など既存の物は触らない）・イメージ `sai-tree:check`・一時フォルダ・一時の sai-prep・起こしたサーバとブラウザを消す。

## 報告
作業報告書に、各段の結果（通った/落ちた・出力の要点）・見つけた不具合・画面写真のパスを書く。変更をコミットし、`sai-link doc write` で保存し、最後に `sai-link task report` を実行すること。実物が通らなかった段は、通ったふりをせず、そのまま書く。
