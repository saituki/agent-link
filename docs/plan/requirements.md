# 要件定義書: sai-tree（sai-moon を研究室の Linux 多人数サーバにした別プロジェクト）

## 何を

`~/workdir/Sai/sai-moon` を複製して、別のプロジェクト **sai-tree**（`~/workdir/Sai/sai-tree`）を作る。sai-tree は研究室の Linux PC の上で 1 つだけ動く Web サーバである。研究室のメンバーは、Tailscale で同じ tailnet に入った自分の PC・iPhone から使う。sai-moon（Mac で 1 人が使う物）は今のまま残し、直さない。

```
sandbox | cnt1 | cnt2 | cnt3 |   … プロジェクトごとのコンテナ（エージェント・papr.db・md・pdf）
--------+----------------------
Linux   | sai-tree の Web サーバ … ログイン・プロジェクト・Chats・Papers・Research・Usage。
        |                         コンテナでエージェントを起動し、データを送り、会話を中継する
--------+----------------------
各自の Mac | sai-prep            … PDF の前処理（docling・翻訳）を手元で行い、結果をサーバへ送る
```

1. **エージェントは headless**: herdr を使わない。claude（`claude -p` の stream-json）と pi（`pi --mode json`）を子プロセスとして動かす。子タスクも pane を作らず、裏のプロセスとして動く。利用者は端末を見ない。会話はすべて Web の Chats から行う。
2. **Chats**: 今の Chats 画面を直して使う（Open WebUI などの既存の画面は使わない）。1 つのチャットは、task.md で起動した 1 つの run との会話である。runner・provider・model は task.md で決まる。生成中の文が少しずつ流れ、続きを送ることも、中断することもできる。Chats から新しいチャット（＝新しいカードを作って実行）を始められる。エージェントの質問は、チャットの中の質問カードと Home の待ちの一覧に出て、答えるとエージェントが続ける。
3. **アカウント**: 管理者がアカウントを作る。ログインの ID は**大学のメールアドレス**。各アカウントには、管理者が決める**短い名前**（例 `saito`）があり、フォルダ名とコンテナ名に使う。初回ログインでパスワードを変える。役割は管理者と一般の 2 つ。
4. **プロジェクト = sandbox**: ログインした人が Web からプロジェクトを作る。1 つのプロジェクトに rootless Docker のコンテナを 1 つ割り当てる。コンテナには、そのプロジェクトのフォルダと、そのプロジェクト専用の HOME だけを見せる（他人のプロジェクトも、同じ人の別のプロジェクトも、エージェントから物理的に見えない）。
5. **鍵**: 各自が「アカウント」画面で OpenRouter・Semantic Scholar などの API キーと、Claude の長命トークン（各自が自分の Mac で `claude setup-token` を実行して発行した物。研究室の Team プランに大学のメールアドレスでログインして発行する）を保存する。保存した値は自分のコンテナにだけ渡り、画面には末尾 4 文字しか出ない。
6. **論文**: Library の S2 検索とメタデータの取得はサーバで行う。PDF 本体はサーバでは落とさない。各自の Mac の **sai-prep**（ローカルの Web 画面のアプリ）が、PDF を受け取り、docling（md・json・図）と pdf2zh（翻訳。LLM は sai-prep が手元で起こす mlx-lm の translateGemma）で前処理し、結果をまとめてサーバへ送る。サーバは docling・pdf2zh を動かさず、送られた結果から図・節・引用の表を作る軽い処理だけを行う。BibTeX と PDF を sai-prep へ一緒に drag&drop すれば、S2 検索をしなくても Library に取り込める。
7. **今の機能を保つ**: Home・Projects・Chats・Papers・Research・Usage・コマンド画面・質問カード・プッシュ通知は、サーバでも同じように使える。

## なぜ

- 今は 1 台の Mac で、1 人の Claude のログインと環境変数の API キーを使っている。研究室の数人で使うには、人ごとにデータ・鍵・ログインを分ける必要がある。
- herdr の pane とキー送信に頼る作りは、画面の無いサーバでは使えない。サーバのエージェントの画面を利用者が直接見ることもない。
- docling と翻訳は重い。多人数ぶんをサーバで回すと負荷が大きいので、各自の PC で行う。

## 決定事項と理由（利用者と決めたこと）

| 決定 | 理由・補足 |
|---|---|
| sai-moon とは別のプロジェクト `~/workdir/Sai/sai-tree` にする。sai-moon を `git clone` して作る | sai-moon の今の動き（herdr・Mac）を壊さない。今の機能とテストを引き継ぐ |
| herdr を使わない。claude・pi は headless、子タスクも裏のプロセス | 利用者が端末を見ないため。Web から会話できれば足りる |
| チャットは今の Chats を直して使う | Open WebUI は「利用者が送った文への返事」しか表示できず、task.md から自分で動き続ける run を見る用途に合わない（調査 R1） |
| 質問はチャットの中の質問カードと Home の一覧で答える | 今の仕組みを使う |
| ログインは sai-web のアカウントだけ。ID は大学のメールアドレス。フォルダ名とコンテナ名は管理者が決める短い名前 | 利用者の選択 |
| Claude は `claude setup-token` の長命トークンを、各自が画面に貼って保存し、その人のコンテナへ `CLAUDE_CODE_OAUTH_TOKEN` で渡す | 利用者の選択。1 つのログイン情報を複数のコンテナで共有すると更新がぶつかってログアウトが連鎖する（anthropics/claude-code#56339・#24317）が、長命トークンは更新が無い。**Anthropic の規約は資格情報の保存・仲介を禁じているため、灰色であり、使う人の責任とする。画面と文書にその旨を出す**。Web の端末でのログインの中継は作らない |
| 各自の API キーは画面で設定し、自分のコンテナにだけ渡す | 利用者の選択 |
| プロジェクトは持ち主 1 人だけが見る・操作する。管理者は全員のコンテナの一覧を見て、止める・起動することだけができる（中身は見ない） | 利用者の選択。共有は後で足す |
| Radio（apps/radio）と MLflow は全員で 1 つを共有する | プロジェクトのフォルダ名を `<短い名前>-<プロジェクト名>` にして、名前の衝突を防ぐ |
| Mac 専用の機能を削る（遠隔ブラウザ・launchd・キーチェーン・cc の statusline の 5 時間枠） | サーバでは動かない・意味が無い |
| sandbox は rootless Docker。コンテナは root（＝ホストの利用者 `sai`）で動かす | Web が乗っ取られてもホストの root は取られない。ファイルの持ち主がそろい、ホストの Web とコンテナが同じファイルを読み書きできる |
| Claude Code は `--dangerously-skip-permissions` の代わりに「すべての道具を許可する一覧」で起動する | Claude Code は root で `--dangerously-skip-permissions` を拒否する（調査 R3）。公式の範囲で動かす |
| 利用者の PC・iPhone と Linux の Web サーバは、すべて Tailscale でつなぐ | 一般公開しない |
| 翻訳の LLM は各自の Mac の sai-prep が mlx-lm で起こす（translateGemma）。dgx の vLLM は使わない | 利用者の選択 |
| sai-prep はローカルの Web 画面（`uv tool install` で入れる Python のアプリ） | 利用者の選択。docling・pdf2zh・mlx-lm は別々の `uv tool` の環境に入れ、CLI で呼ぶ（依存の衝突を避ける。調査 R2） |
| Library の S2 検索とメタデータはサーバ、PDF 本体は sai-prep から送る。BibTeX と PDF の drag&drop でも取り込める | 利用者の選択 |
| kantoku は sai-tree のプロジェクトで動かす。計画書は sai-tree のリポジトリの `docs/plan/` に置いて commit する | 利用者の選択（先に sai-tree を用意する）。waggle のトークンはプロジェクトをまたいで使えないので、パスで渡す |

## 完了したときに利用者が確かめられること

1. Linux PC で `docs/server.md` の手順どおりに入れると、`https://<マシン名>.<tailnet>.ts.net/` にログイン画面が出る。PC と iPhone の両方で使える。
2. 管理者が `sai-web user add <短い名前> --email <メールアドレス> --admin` で最初の管理者を作り、ログインできる。管理画面で一般のアカウントを作れる。
3. 一般の人がメールアドレスでログインし、初回にパスワードを変えさせられる。他人のプロジェクト・会話・質問は見えない（URL を直接打っても 404）。
4. 「アカウント」画面で API キーと Claude のトークンを保存できる。値は再表示されず、末尾 4 文字だけが出る。トークンの規約上の注意が画面に出る。
5. Projects で新しいプロジェクトを作ると、コンテナが起動し、常駐（daemon）が動いている表示になる。コンテナの中から、同じ人の別のプロジェクトのフォルダが見えない。
6. Chats の「新しいチャット」から block・runner・model・最初の文を選んで始めると、応答が少しずつ流れて表示される。入力欄から続けて会話でき、「中断」で止められる。
7. エージェント（cc・pi の両方）が質問すると、チャットに質問カードが出る。答えるとエージェントが続きを始める。
8. エージェントが子タスクを作ると、子は画面を開かずに裏で動き、親の完了報告まで進む。
9. Papers の検索で見つけた論文を「Library に入れる」と、PDF の無い論文として Library に入る。sai-prep の画面にその論文が「PDF 待ち」として出て、PDF を drop すると、前処理のあと、サーバの Library で PDF・翻訳 PDF・図・節が見られる。
10. sai-prep に BibTeX と PDF を一緒に drop すると、S2 検索をしなくても Library に論文が入る。
11. sai-tree のテスト（Rust・Web・Radio・sai-sci の道具・sai-prep）がすべて通る。CI に Linux のジョブがある。Mac から `deploy.sh` で Linux へ反映する手順が文書にある。

## 範囲外（今回はしない）

- プロジェクトを複数のメンバーで共有すること。
- Web からプロジェクトを消すこと（管理者が手で行う。手順は `docs/server.md`）。
- cc の承認（PermissionRequest）。headless では承認が来ないので、承認の仕組みは消す。
- Claude Code の 5 時間枠の表示。
- daemon が止まったときの headless の実行の引き継ぎ（daemon を再起動すると、動いていた headless の実行は失敗の扱いになる）。
- 一般公開（Tailscale Funnel など）。
- サーバでの PDF のダウンロード（S2・出版社から）と、サーバでの docling・pdf2zh の実行。
- sai-prep の Windows・Intel Mac 版。
- sai-moon（Mac 版）を直すこと。

## 参照文書（sai-moon のプロジェクトに保存してある。`sai-link doc read <トークン>` で読める）

- 調査 X1（エージェントの起動経路と herdr 依存）: waggle:7bTv9kKd
- 調査 X3（認証・置き場・Mac 依存）: waggle:F3eM7a9W
- 調査 R1（Open WebUI を会話画面にする方法。不採用の根拠）: waggle:8oEzEd1B
- 調査 R2（Mac の sai-prep に docling・pdf2zh・mlx-lm を入れる方法）: waggle:jHBtJVfj
- 調査 R3（rootless Docker・コンテナの中の Claude Code のログイン）: waggle:brp8Ra6M
- 調査 E1（論文の前処理が作る物と取り込みの経路）: waggle:hVhoTCmh
- 調査 E2（前処理の zip と BibTeX の取り込み）: waggle:TUnovHdL
- 調査 E3（Mac 専用の機能と Web の herdr を削る範囲）: waggle:CAAQvf67
- 前の計画 406（sai-moon の中に多人数モードを足す案。この計画で置き換えた）: 要件 waggle:6A29ym8Q
