# 実装ガイド 4/5: 1 プロジェクト 1 コンテナ（rootless Docker）と鍵

## ゴール
Linux PC で `docs/server.md` のとおりに入れると、研究室のメンバーが tailnet からログインし、Projects でプロジェクトを作るとコンテナが起動して常駐が動く。コンテナには、そのプロジェクトの root と専用の HOME だけが見え、同じ人の別のプロジェクトも見えない。アカウント画面で API キーと Claude の長命トークン（`claude setup-token`）を保存でき、自分のコンテナにだけ渡る。コンテナの中のエージェントの質問は持ち主にだけ出る。管理者は全員のコンテナを見て止める・起動できる。

## 実装計画書の一覧
| ID | 件名 | 文書 | 書くファイル | 依存 | 並列 |
|---|---|---|---|---|---|
| U08 | イメージ・CLI の中継・rootless Docker の入れ方 | `docs/plan/unit-U08-container-image.md`（waggle:Mk5iViZ9） | `apps/web/server/`（新規）・`.dockerignore`・`docs/server.md` | ガイド 1〜3 | 可（U09 と） |
| U09 | プロジェクト作成・コンテナ・API キーと Claude のトークン | `docs/plan/unit-U09-projects-containers.md`（waggle:rA6fwyAw） | `containers.py`（新規）・`main.py`・`registry.py`（`daemon_alive`）・`store.py`（`project_tokens`）・`test_containers.py`・`docs/integration.md` | ガイド 1〜3 | 可（U08 と） |
| U10 | 内部の合言葉・質問の持ち主・Usage の鍵 | `docs/plan/unit-U10-internal-auth.md`（waggle:fZZkr1f4） | `main.py`・`store.py`・`watch.py`・`notify.py`・`usage.py`・`containers.py`（1 行）・`test_internal_auth.py`・`docs/integration.md` | U09 | 可（U11 と） |
| U11 | プロジェクトとコンテナ・鍵の画面 | `docs/plan/unit-U11-server-screens.md`（waggle:S5AeV4tB） | `static/project.js`・`account.js`（区画）・`index.html`・`sw.js`・`style.css`・JS のテスト | U09 | 可（U10 と） |

## 実行順序
```
U08 ─┐
     ├─▶ U10 ∥ U11
U09 ─┘
```
- U08 ∥ U09（書くファイルが重ならない）。U09 の AC5（実物の Docker）は U08 のイメージが要るので、**U08 と U09 をマージした後に kantoku が行う**（U09 の worker には偽の docker のテストまでを行うよう追加指示で伝える）。
- U10 は U09 の後（`main.py`・`store.py`・`containers.py` を両方が書き、U10 は U09 の `project_tokens` を使う）。
- U11 は U09 の後（画面が U09 の API を使う）。U10 とは書くファイルが重ならない（U10 はサーバ、U11 は画面）ので並列にしてよい。

## 所有権の表
| ファイル | U08 | U09 | U10 | U11 |
|---|---|---|---|---|
| `apps/web/server/`・`.dockerignore`・`docs/server.md` | 書く | 読む | 読む | - |
| `apps/web/src/sai_web/containers.py` | - | 書く | 書く（1 行）※U09 の後 | - |
| `apps/web/src/sai_web/main.py`・`store.py` | - | 書く | 書く ※U09 の後 | - |
| `apps/web/src/sai_web/registry.py` | - | 書く（`daemon_alive`） | 読む | - |
| `apps/web/src/sai_web/watch.py`・`notify.py`・`usage.py` | - | - | 書く | - |
| `apps/web/src/sai_web/static/*` | - | - | - | 書く |
| `docs/integration.md` | - | 書く | 書く ※U09 の後 | - |

## 共有契約
- C4 多人数モードの置き場（ガイド 3 の U06）。C7 コンテナと中継（U08）。C8 `docker run` の引数（U09）。C9 プロジェクト・コンテナ・キーの API（U09）。C3 フック・拡張と Web の約束（ガイド 1 の U02）。どれも使う計画書に全文がある。

## 全体の完了条件
| 条件 | 担当 ID | 確認方法 |
|---|---|---|
| イメージが作れて CLI が中で動く（docling・pdf2zh は無い） | U08 | U08 の AC1・AC3 |
| Web からプロジェクトを作ると常駐まで動く | U08・U09 | 統合後の U09 の AC5 |
| コンテナから別のプロジェクトが見えない | U08・U09 | U08 の AC3、統合後の U09 の AC5 |
| 鍵が値を出さずに保存され、コンテナに届く | U09・U11 | U09 の AC2、統合後にコンテナの中で `printenv` |
| 内部の要求が持ち主に結び付き、他人に見えない | U10 | U10 の AC1 |
| 画面がそろい、単独モードは変わらない | U11 | U11 の AC2・AC3 |

## 統合と検証
- マージごとに AGENTS.md の「テストと確認」をすべて回す。
- U08・U09 のマージ後: Mac の Docker で `sai-tree:check` を作り、ホーム直下の一時フォルダを `SAI_WEB_USERS_ROOT` にした 8299 番の多人数モードの Web（`SAI_LINK_BIN` などを中継に向け、`SAI_IMAGE=sai-tree:check`）で、ログイン → プロジェクト作成 → 常駐が動く、を確かめる（U09 の AC5）。Mac の Docker は rootless ではないが、引数の形と動きの確認には足りる。
- 全部のマージ後（最終確認）: 同じ環境で、新しいチャット（費用の小さい model。API キーは利用者に聞かず、Mac の環境にある `OPENROUTER_API_KEY` を `PUT /api/me/keys` で入れて pi を使う）→ 逐次表示 → 質問カード → 子タスクを 1 つ作らせて完了、までを確かめる。cc は、利用者の Claude のトークンが無ければ確かめない（報告に書く）。画面写真を完了報告書に付ける。
- 確認で作ったコンテナ（`sai-*` のうち確認用の物だけ）・イメージ・一時フォルダは必ず消す。本番の 8200 番・`apps/mlflow-server` の compose・利用者の環境（`~/.sai` など）には触らない。
- `cargo build --release` を本体フォルダで行い、3 つの実行ファイルを作り直す。

## 決定事項と理由 / 採用しなかった案
- sandbox は rootless Docker（利用者の決定）。コンテナは root（＝ホストの `sai`）で動かし、ファイルの持ち主をそろえる。/ `--user` を付けて非 root で動かす案は、ホストの Web が書けなくなるので採らない（調査 R3）。
- コンテナにはプロジェクトの root と専用の HOME だけを見せる（要件: 同じ人の別のプロジェクトも見えなくする）。
- Claude は、各自が `claude setup-token` で発行したトークンを預かり、`CLAUDE_CODE_OAUTH_TOKEN` で渡す（利用者の決定。規約上の灰色は画面と文書で知らせる）。/ Web の端末で `claude auth login` を中継する案は、プロジェクトごとにログインが要るので、利用者が採らなかった。
- CLI は Web を変えず、`SAI_*_BIN` を中継のスクリプトに向けてコンテナの中で動かす。
- ホストとコンテナで同じパス（`/srv/sai/users/...`）を使い、記録のパスを読み替えない。
- Web は `0.0.0.0:8200` で待ち受け、防火壁で tailnet とコンテナ以外を止める。Tailscale serve はそこへ中継する。コンテナからは `host.docker.internal`（届かなければ `SAI_WEB_URL_FOR_CONTAINERS`）。
- 鍵はコンテナの `--env-file`、変えたら実行中でないコンテナを作り直す。暗号化して DB に置く案は依存が増えるので採らない。
- Radio・MLflow は全員で 1 つを共有（利用者の決定）。フォルダ名が `<user>-<project>` なので名前は衝突しない。
- プロジェクトは持ち主 1 人だけ（利用者の決定）。管理者はコンテナの一覧と起動・停止だけを行う。
- Docker は CLI で呼び、SDK を足さない。隔離は rootless Docker と資源の上限（`--cap-drop ALL`・`no-new-privileges`・メモリ・CPU・pids）。

## リスクと対処
- イメージのビルドが通らない（OS のライブラリ・npm のパッケージ名）: U08 の worker が Dockerfile を直してよい（形の見本である）。
- `--cap-drop ALL` で何かが動かない: 足す capability を最小にし、理由を書く（kantoku が決めてよい）。
- Mac の Docker（arm64・rootless でない）では動くが Linux（amd64・rootless）で動かない: 手順書に「その機械でビルドする」と書いてあれば足りる。Linux の実機での確認は利用者が行う（完了報告書に、利用者が Linux で確かめる手順を書く: `docs/server.md` の 1〜10 と、プロジェクトの作成・チャット・`host.docker.internal` への到達）。
- 1 つの Claude のトークンを複数のコンテナで同時に使うと問題が出る: 報告に書き、利用者に聞く（1 人 1 コンテナずつ動かす運用にするかなど）。

## 参照文書
- 要件定義書: `docs/plan/requirements.md`（waggle:y7EoLAgV）
- 調査 R3（rootless Docker・コンテナの中の Claude Code）: `docs/plan/research/R3-rootless-docker.md`
- 調査 X3（認証・置き場・Mac 依存）: `docs/plan/research/X3-multiuser.md`
