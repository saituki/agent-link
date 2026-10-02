# sai-moon アーキテクチャ（開発者向け）

**対象読者:** sai-moon を直す人と、`crates/`（Rust）または `apps/radio` に Web 向けの機能を足す人。
使い方は [`README.md`](./README.md)、API と住所の一覧は [`docs/integration.md`](./docs/integration.md) を正とする。
ここには、全体の考え方・層の分け方・守る約束・置き場を書く。**正はコード**である。この文書とコードが食い違ったら、コードに合わせて直す。

---

## 1. 3 つの考え

### 1.1 統合 — 中身は 1 つ、入口は 3 つ

sai-moon を「中身（コア）」と「入口（UI）」に分ける。旧 4 リポジトリを 1 つの Cargo ワークスペースと 2 つの Python アプリにまとめた。

| 区分 | クレート・フォルダ | 持つもの |
|---|---|---|
| **コア** | `crates/sai-link-engine` | 実行の DB（`sai-engine.db`）・常駐（daemon）・ブロック定義の解析と検査・Workflow の配置の計算・書き出し |
| **コア** | `crates/sai-link-widgets` | ダッシュボードの部品群（L3）。`left_tree`・`block_log`・`block_history`・`card_wizard` など。TUI と `phone` CLI が共有する |
| **コア** | `crates/sai-paper-core` | 論文の DB（`papr.db`）・検索と取得・BibTeX・Download の後の登録・翻訳・図の抽出・Library の階層 |
| **コア** | `crates/sai-paper-cli` / `crates/sai-paper-tui` | 論文の CLI と TUI の実装（コアの上） |
| （周辺） | `apps/radio` | 解説音声の生成と配信（HTTP API・8201 番） |
| **入口** | `crates/sai-link-tui` → `sai-moon-link-tui` | 端末の画面（ratatui） |
| **入口** | `crates/sai-paper-tui` → `sai-moon-paper-tui` | 端末の画面（ratatui） |
| **入口** | `apps/phone` | Web の画面（FastAPI + 素の JavaScript） |

- **入口の中にしか無い処理を作らない。** 以前は TUI の中にしか無い処理（Workflow の配置、BibTeX の保存、Download の後の登録、Library の階層）があった。
  それらをコアのクレート（`sai_link_widgets::workflow`・`sai_paper_core::bibtex` / `downloads` / `library_tree`）へ移し、**TUI も同じ関数を呼ぶ**ようにした。
  Web のために CLI へ処理を写すことはしない（写すと処理が 2 か所になり、片方だけ直る不具合が起きるため）。
- 1 つのバイナリに 3 つの入口を押し込まず、`[[bin]]` で分ける。**ライブラリ crate 名（`sai_link_engine` / `sai_link_widgets` / `sai_paper_core`）は旧名のまま**にして、移植 معدل直言が crate の外に出ないようにしている。

### 1.2 Web ユーザーファースト

`apps/phone` を sai-moon の主な入口にする。端末に慣れていない人が、Web（iPhone と PC のブラウザ）だけで sai-link と sai-paper を使えるようにする。

- **TUI でできる操作を Web でもできるようにする。** Workflow・定義の編集・BibTeX・Download・論文の Project を移し終えた。残りは §8。
- **iPhone と PC の両方を前提にする。** 幅 900px（`ui.js` の `SAI_WIDE_PX`）を境に、狭い画面は下のタブの 1 列、広い画面は左の縦のタブと 2 列にする。
- **言葉で迷わせない。** 表示は日本語にする（`data-tab` や住所などの識別子は英語のまま）。
  sai 独自の言葉は `saiTerm` の札で説明する。空の画面には次の一手を出す（`saiEmpty`）。
- **入口はホーム。** 開いたらまず「いま決めること」（質問・承認・review・要対応）を 1 画面で見せる。
- **答えは先着。** 質問と承認は PC と iPhone の両方に同時に出し、先に答えた方を採用する。

### 1.3 全体の再構成 — 層を分ける

```
┌──────────────────────── 入口（UI） ────────────────────────┐
│ sai-moon-link-tui    sai-moon-paper-tui    apps/phone（Web）│
│   │                       │              iPhone / PC ブラウザ │
│   │                       │                   │ HTTPS（Tailscale serve :4443）
│   │                       │            127.0.0.1:8200 FastAPI（窓口）
└───┼───────────────────────┼───────────────────┼───────────┘
    │ Rust の関数を直接       │                   │ 公開された約束だけを使う
    │                  ┌────┴───────────────────┴──────────────┐
    │                  │ `sai-moon-link phone …` / `daemon …`（JSON）│
    │                  │ `sai-moon-paper phone …`（JSON）          │
    │                  │ `~/.sai-moon/phone/runs/*.json`（書き出し）   │
    │                  │ `~/.sai-moon/registry.json`（登録簿）        │
    │                  │ apps/radio の HTTP API（127.0.0.1:8201）   │
    │                  └────┬───────────────────┬──────────────┘
┌───┼───────────────────────┼───────────────────┼───────────┐
│ crates/sai-link-*（engine・widgets・cli・tui） crates/sai-paper-* │
│   sai-engine.db                                   papr.db      │
└──────────────────────── コア ─────────────────────────────────┘
```

- **コア**は DB と処理を持つ。
- **公開された約束**（`phone` CLI の JSON と終了コード・書き出し・登録簿・HTTP API）が、コアと Web の境目である。`apps/phone` はここだけを使う。
  約束の中身は [`docs/for-sai-developers.md`](./docs/for-sai-developers.md) に書いてあり、コア側を直す人はそれを守る。
- **入口**は表示と入力だけを受け持つ。TUI は Rust の関数を直接呼び、`apps/phone` は CLI を呼ぶ。どちらも同じ関数に行き着く。
- **研究方針の木（sai-sci）**: 定義の md は `sai-sci/`、処理は `crates/sai-paper-core/src/rt/`、入口は `sai-moon-paper sci` / `phone sci` / MCP / Web / TUI。
  木には 3 役が乗じる（詳しくは [`sai-sci/README.md`](./sai-sci/README.md) の 3.3.1）。
  木は Q（問い）・O（答えの候補。Answer か Issue）・F（知見）・E（実験）でできている。評価は Q ごとの Objectivity（F の本数の目安）と Logicality（筋の確かめ）、研究の 1 文は SEED。
  木に向けた 1 回の実行が Action（task.md）で、Next Action がその候補、終わった Action の完了基準を満たしていない残りが DEBT。
  - `professor`（`sai-sci/blocks/professor.md`）— 研究の舵取り。研究の現在地を読み、次の一手を決めてカードを作り、researcher（`sci-*`）を**自分の子として**起こす。root の run。
  - `operator`（`sai-sci/blocks/operator.md`）— sai-moon の操作係。故障の対処・監視・情報の抽出・Web と同じ操作だけ。professor の子。研究の判断はしない。
  - `professor-backward`（`sai-sci/blocks/professor-backward.md`）— 既存の論文から始める逆向き（backward）の舵取り。抽出 → 評価 → 再現 → 更新 → 比較の順に researcher を起こす。役目の分け方は `professor` と同じ。
  - `sci-*` の 23 本 — 研究作業そのもの。木の Node（Q・O・F・E）に向けて動く。実験の 3 本（`sci-exp-sprout`・`sci-exp-branch`・`sci-exp-review`）は互いの終わりを待たず、ジョブの終わりで review が起きる。review が完了報告書 `exp-final-<study>.md` を書き、実験仕様書の `return_to` があれば、その professor のカードを作って起こし直す。うち 3 本（`sci-restore`・`sci-cite-check`・`sci-result-compare`）は逆向きのためのもの。役の 4 本目の `kaizen` は、実行履歴から改善計画書を作る。
  - カード（task.md）の `executer` は起動者（`human` / `professor`）。`--executer professor` のカードは人が起こせず（`phone card run` が断る）、professor が `task_create` の `card` で起こす。runner は pi だけ。
  - 質問は**そのカードの起動者が答える**（professor が起こしたカードなら professor）。親の `task_output` に `stalled`（`reason: "question"`、`question_id` 付き）で届き、`phone_api.py question` / `answer` で答える。Web で人が先に答えた場合は 400 で断られ、その答えを採る。
  - 報告は run フォルダの `finalize-report.md`。子は `task_report`、root は `sai_finalize` の `report`。`sai-moon-link phone md --kind report <run_id>` で読む。
  - 3 役が使う共通の決まりは `sai-sci/template/` の `sai-agent.md`・`sai-task.md`・`sai-cli.md`（Web と同じ操作は `sai-sci/tools/phone_api.py`）。`sai-moon-paper init --sci`（と `--assets-only`）で配る。
- **ブロックの起動時の文は 2 つ**（固定部分と最初の文）。固定部分（template と block.md の本文）は run フォルダの `system-prompt.md` に書き、
  pi は `--append-system-prompt <ファイル>`、cc は `--append-system-prompt-file <ファイル>` で渡す。最初の文は sai-link が自動で作る
  `# context`（`context: true` のとき。範囲は研究方針の木から切り出したもの）と `# task_text`（依頼文）の節だけで、順番は context → task_text。
  ブロック本体に `{{input.task_text}}`・`{{context}}` を書く必要はない（残していても互換で読む）。Web の会話の最初の吹き出しと `message.md` には可変部分だけが出るので、
  依頼が見やすくなります。固定部分は run の画面の「system」の帯で読む（`phone md --kind system <run_id>`）。
  作り方は `docs/for-sai-developers.md` の「起動時の文の分け方（C10）」。

---

## 2. 大原則

1. **`apps/phone` は窓口だけ。** 処理とデータの保管はコアがする。
   - 他のアプリの DB（`sai-engine.db` / `papr.db` / `sai_radio.db`）を**開かない**。DB の形が変わったときに `apps/phone` が黙って壊れるのを防ぐため。
   - `apps/phone` 自身が持つのは、質問・承認・pane の対応表だけ（`apps/phone/src/data/sai_phone.db`・`store.py`）。
   - Download と音声化の進み具合は `apps/phone` のメモリにだけ持つ（再起動で消えてよい。結果はコアに残る）。
2. **足りない機能はコアに足す。** Web に要る処理が TUI の中にしか無ければ、コアのクレートへ移して TUI もそれを使うようにし、`phone` CLI で出す。`apps/phone` で計算し直さない。
3. **外から来る値は、CLI を呼ぶ前に `apps/phone` が検査する。** 名前の形・大きさ・種類の一覧を `apps/phone` が先に確かめる。通らない値では CLI を 1 度も呼ばない（400）。ファイルのパスは iPhone から受け取らない。置き場はコアが名前から決める。
4. **依存を増やさない。** サーバは FastAPI と標準ライブラリ、画面は素の JavaScript（フレームワーク・ビルド無し）。グラフと Workflow の線は SVG を手で描く。md の色分けも自前の簡易版で行う。
5. **合言葉（`SAI_PHONE_TOKEN`）で入口を守る。** 127.0.0.1 でだけ待ち受け、外へは Tailscale serve で tailnet の中にだけ出す。
6. **エラーは日本語でそのまま画面に出す。** 何が起きたか、次に何をすればよいかが分かる文にする。

---

## 3. 置き場（`~/.sai-moon/`）

利用者全体で共有する置き場は **`~/.sai-moon/`**。旧構成の `~/.sai/` ではない。環境変数 `SAI_SAI_HOME` があればそれが優先する。

### 3.1 決める場所

| 場所 | 定義している場所 | 既定値 |
|---|---|---|
| Rust（sai-link） | `sai_link_engine::project::sai_home()`（`crates/sai-link-engine/src/project.rs:79`） | `SAI_SAI_HOME` → `~/.sai-moon` → `HOME` も無ければ `None` |
| Rust（sai-paper） | `sai_paper_core::paths::sai_home()`（`crates/sai-paper-core/src/paths.rs:172`） | 同じ規則の小さなコピー。**値は必ず engine 側と一致させること** |
| `apps/phone` | `registry.sai_home()`（`apps/phone/src/sai_phone/registry.py:41`） | `SAI_SAI_HOME` → `~/.sai-moon` |

3 つは同じ規則のコピーが 3 枚ある。**どれか 1 つだけ直さないこと。**

### 3.2 `~/.sai-moon/` に入るもの

| 物 | 参照している場所 |
|---|---|
| `registry.json`（プロジェクトの登録簿） | `phone_export.rs:230`・`registry.py` |
| `phone/runs/<project_name>/<run_id>.json`（書き出し） | `phone_export.rs` |
| `cc-cost` | `phone_export.rs` |
| 共通の `blocks/` | `common.rs:72`・`orchestrator.rs:2725`・`handoff.rs:96`・`runs.rs:170`・`dsr.rs:96` |
| 共通の `template/` | `parser.rs:168` |
| 共通の `settings.json` | `pi_command.rs:115` |
| `paper.env`（全プロジェクト共通の API キー） | `paths.rs:152` |

### 3.3 `~/.sai-moon/` に入らないもの

- **プロジェクトの中の `<root>/.sai/` は変わらない。** `sai-engine.db`・`blocks/`・`template/`・`settings.json`・`outputs/` はここ。
  `sai_link_engine::project::project_root()` / `sai_path()` と `sai_paper_core::paths::project_root()` は変更していない。
- `apps/phone/src/sai_phone/usage.py` の `cc_limits_path()` は `<sai_home>/phone/cc-rate-limits.json` を読む。
  書くのは `sai-moon-link cc-statusline`（`phone_export.rs` の `cc_rate_limits_path()`）で、置き場は同じ `~/.sai-moon/`。
  旧 `~/.sai/phone/` の同名ファイルは読まない（古いまま残っている）。
- `~/.pi/agent/models.json`・`~/.claude/projects`・`~/Downloads` などは元の置き場のまま（`common.rs:177`・`config.py`・`litbase.rs:281`）。

### 3.4 移行の状況（利用者が把握しておくこと）

- `~/.sai-moon/registry.json` は kantoku が作ったもので、中身は sai-moon の 1 件だけ。
  古いプロジェクトを新旧どちらの常駐にも同時に扱わせないための決定。
- 旧 `~/.sai/blocks` は空だったので、写すものは無かった。
- 別に旧 `~/.sai/registry.json` にも sai-moon を登録してある（sai-moon 自身の開発を旧 sai-link で管理するため）。
- `~/.sai/phone/runs` に溜まっている過去分は `~/.sai-moon/phone/runs` に自動で移らない。`sai-moon-link phone export` を流的と新しい場所へ出る。

---

## 4. `apps/phone` の中身

### 4.1 サーバ（`apps/phone/src/sai_phone/`）

| ファイル | 受け持ち |
|---|---|
| `main.py` | FastAPI の住所の登録・合言葉（`require_token`）・Mac 自身からだけ受ける内部 API（`require_local`） |
| `cli.py` | 外部 CLI を呼ぶ共通の受け口。起動できない → 503、応答が JSON でない → 502 |
| `control.py` | sai-moon-link の呼び出し（プロジェクト・List・Workflow・定義の編集・カードの作成と実行・常駐）。cwd と `SAI_PROJECT_ROOT` を必ずプロジェクトの root にする |
| `papers.py` | sai-moon-paper の呼び出し（Library・Discover・Import・Download・Project・音声化の依頼）。cwd と `SAI_PAPER_ROOT` を選んだプロジェクトの root にする |
| `home.py` | ホームの集計（`GET /api/home`）。プロジェクトごとに並べて呼び、失敗は `errors` に入れる。10 秒覚える |
| `registry.py` | 登録簿 `~/.sai-moon/registry.json` の読み込み（プロジェクトの一覧）と `sai_home()` |
| `runs.py` / `sessions.py` | sai-moon-link の書き出し（`~/.sai-moon/phone/runs/`）と会話の記録（.jsonl）の読み取り |
| `questions.py` / `approvals.py` / `store.py` | 質問と cc のコマンド承認の保管と回答（先着） |
| `herdr.py` / `watch.py` / `notify.py` | herdr の呼び出し・2 秒ごとの状態の監視・プッシュ通知 |
| `radio.py` | `apps/radio`（127.0.0.1:8201）への中継 |
| `usage.py` | 使用量（Claude Code・OpenRouter・DeepSeek・pi の記録） |
| `extensions/ask_user_phone.ts` | pi に iPhone 連携付きの質問ツールを足す拡張（既定 URL は `http://127.0.0.1:8200`） |
| `hooks/`（`apps/phone/hooks/`） | cc のフック（質問・承認・セッションの登録）。何も出力せず、必ず終了コード 0 で終わる |

### 4.2 CLI の失敗を HTTP に直す

| コアの返し方 | `apps/phone` の返し方 |
|---|---|
| sai-moon-link の終了コード 0 と JSON | 200（JSON が読めなければ 502） |
| sai-moon-link の終了コード 2（検査に通らない） | 400（stderr の理由） |
| sai-moon-link の終了コード 3（見つからない） | 404 |
| sai-moon-link の終了コード 1 で「同じ名前の定義があります」 | 409 |
| sai-moon-paper の `{"error": "…"}` と終了コード 1 | 400（その文） |
| CLI が起動できない | 503（設定の場所を確かめる文） |
| それ以外 | 502（stderr の先頭 200 文字） |

### 4.3 画面（`apps/phone/src/sai_phone/static/`）

- **枠**: `index.html` が全画面の `<section class="view">` とタブを持ち、JS を決まった順に読む。
  - CSS: `style.css` → `markdown.css`・`home.css`・`workflow.css`・`defs.css`・`papers.css` → `scitree.css`・`sciedit.css`・`scigraph.css`
  - JS: `markdown.js` → `ui.js` → 各画面の JS → `home.js`・`workflow.js`・`defs.js`・`paperimport.js`（`workflow.js` と `scigraph.js` は `canvaslayout.js` の関数を使うので後）→ 最後に `app.js`
- **画面の登録**: 各画面の JS が `window.SaiRoutes` に「住所の照合・描画」を登録し、`app.js` が照合して切り替える。**`app.js` に画面ごとの分岐を足さない。**
- **共通部品**（`ui.js`）: `saiApi`（通信と日本語のエラー）・`saiToast`・`saiConfirm`・`saiPrompt`・`saiSheet`（下から出る板）・`saiEmpty`（空の画面の案内）・`saiTerm`（用語の札。用語は `SAI_TERMS`）・`saiIsWide` / `saiSplitShow`（広い画面の 2 列）。新しい画面はこれを使い、同じ物を作り直さない。
- **md の表示**（`markdown.js`）: `renderMarkdown(text, {toolbar: true})` で、色分け・全部たたむ / 開く・目次・長いコードのたたみが付く。コードの色分けは `mdHighlight`。
- **名前の重なり**: 画面ファイルは `<script>` で順に読むので、一番外側の名前が重なると後の物で上書きされる。画面ごとの頭（`sai` 共通・`hm` ホーム・`wf` Workflow・`df` 設定・`pim` Import など）を付け、`apps/phone/tests/test_globals.js` で検査する。
- **版**: `sw.js` の `CACHE`（今は `sai-phone-v36`）と `index.html` の `?v=` をそろえて上げる。上げないと iPhone に古い画面が残る。画面ファイルを足したら `ASSETS` にも足す。

| ファイル | 画面 |
|---|---|
| `app.js` | 切り替えの本体・会話（日付・実行・メッセージ）・タブ |
| `home.js` | ホーム |
| `project.js` | プロジェクト一覧・Dashboard・上部の切り替え |
| `workflow.js` | Workflow（sai-moon-link-tui の Workflow と**同じ並び**の図。配置は Rust 側が計算する） |
| `linkview.js` / `cardform.js` / `stats.js` | List・task.md の作成フォーム・統計 |
| `defs.js` | 設定（block / template / settings） |
| `papers.js` / `paperimport.js` | 論文の各画面・Import と Download の見張り |
| `scitree.js` / `sciedit.js` / `sciread.js` / `scigraph.js` / `sciset.js` | 研究方針（`#/papers/tree`・`#/papers/triage`・`#/papers/read/<run_id>`・`#/papers/sets`。一覧と板（`scitree.js`）・操作と仕分け（`sciedit.js`）・読む札（`sciread.js`）・方針の図（`scigraph.js`）・検索セットの選別（`sciset.js`）） |
| `canvaslayout.js` | 配置・全体表示・拡大の純関数（研究方針の根の図と Workflow が使う） |
| `radio.js` / `pcm-worklet.js` / `voiceask.js` | Radio・声の質問・質問の読み上げと音声回答 |
| `usage.js` | 使用量 |
| `chatorder.js` | 会話の中に行やカードを置く位置（DOM を使わない関数） |

### 4.4 処理の流れの型

- **すぐ返る操作**（一覧・中身・保存）: 画面 → `saiApi` → `apps/phone` の住所 → 値の検査 → CLI（JSON）→ 画面。
- **時間がかかる操作**（Download・音声化）: `apps/phone` が 202 を返し、背景で CLI を走らせる。画面は一覧の住所を見張り、終わったらプッシュ通知を出す。
  Download は 2 段で、`phone download`（PDF を取って登録。ここで「保存しました」を通知）→ `phone after-download`（図の抽出・PDF の翻訳）。
- **エージェントからの質問**: cc のフックと pi の拡張が `POST /api/questions`（Mac 自身からだけ）に登録する → 画面に出て通知 → PC と iPhone の先に答えた方を採用する。

---

## 5. 機能を足すときの手順

1. **処理はどこにあるか。** コアにあればそれを使う。TUI の中にしか無ければ、コアのクレートへ移して TUI もそれを使うようにする（§1.1）。
2. **`phone` CLI で出す。** `sai-moon-link phone …` / `sai-moon-paper phone …`。JSON 1 個を stdout に出し、失敗は決まった終了コードで返す。
   `docs/for-sai-developers.md` に、`apps/phone` が読む項目を書き足す。
3. **`apps/phone` に住所を足す。** `control.py` か `papers.py` に関数を書き、値の検査を CLI を呼ぶ前に置く。`main.py` に登録する（固定の住所は `{paper_id:int}` のような変数の住所より先に登録する）。
   テストは `FakeCli` などで CLI を偽物にして、検査で CLI が呼ばれないことまで確かめる。
4. **画面を足す。** 新しい JS に `window.SaiRoutes` で登録し、`index.html` に枠と読み込みを足し、`sw.js` の版と `ASSETS` を上げる。
   `ui.js` の部品を使い、390px と 1280px の両方で確かめる。
5. **文書を直す。** README（使い方）・この文書（仕組みが変わったとき）・`docs/integration.md`（API と住所）・`docs/for-sai-developers.md`（コア側の約束）。

---

## 6. テストと確認

```bash
cd ~/workdir/Sai/sai-moon

# Rust（24 ターゲット）。SAI_PROJECT_ROOT などが混ざっていると History テスト 1 件が落ちる
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace

# Web（JS 24 本・Python 18 本）
for f in apps/phone/tests/test_*.js; do node "$f" || exit 1; done
(cd apps/phone && export PYTHONPATH=src && for f in tests/test_*.py; do .venv/bin/python "$f" || exit 1; done)

# Radio（3 本）
(cd apps/radio && for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done)
```

- JS のテストは DOM を使わない関数を確かめる（画面の関数は DOM を使わない部分に分けて書く）。
- 画面の確認は **8200 / 8201 / 8202 番**で行う。**旧構成の 8100 番（旧 sai-phone）と 8000 番（旧 sai-radio）は、確認のために再起動しない。**
- サーバ・ブラウザ・ネットワークを使う確認の命令には時間の上限を付ける（例 `perl -e 'alarm 180; exec @ARGV' <命令>`）。背景で起動したサーバは最後に止める。
- 書き込みを確かめるときは、確認用の物（例: block `guide-check`・確認用の .bib）か一時フォルダを使い、最後に消す。本物の定義・`papr.db` の中身を確かめのために変えない。
- **旧 4 リポジトリ（`~/workdir/Sai/sai-link`・`sai-paper`・`sai-phone`・`sai-radio`）は読み取り専用。** 触る前に `git status --short` を控え、終わりに同じであることを確かめる。

---

## 7. ファイル構成

```
sai-moon/
├── crates/                   # Rust のワークスペース（8 クレート）
│   ├── sai-link-engine/      # lib  実行エンジン
│   ├── sai-link-cli/         # bin  sai-moon-link
│   ├── sai-link-tui/         # bin  sai-moon-link-tui
│   ├── sai-link-widgets/     # lib  ダッシュボードの部品群
│   ├── sai-paper-core/       # lib  検索/取得/解析/保存
│   ├── sai-paper-cli/        # bin  sai-moon-paper
│   ├── sai-paper-tui/        # bin  sai-moon-paper-tui
│   └── sai-paper-cite/       # bin  sai-moon-paper-cite
├── waggle/                   # 独立 [workspace] の vendor クレート群（exclude 済み）
├── pi-litbase/               # pi の拡張（TypeScript・Cargo の外）
├── sai-sci/                  # 研究方針の木（Research-tree）
│   ├── types/                # 研究の型 5 本（Q の階層の雛形。sai-moon-paper に埋め込まれる）
│   ├── guides/               # ガイド 25 本（Q の文・Child Q・Extract・Pitfall・目安）
│   ├── template/             # 9 本。sci-core・sci-find・sci-match・bw-core（研究の決まり）、exp-core・exp-run（実験の決まり）、sai-agent・sai-task・sai-cli（professor と operator の決まり）
│   ├── tools/                # 台帳 exp_summary.py・実行先への分配器 exp_exec.py・Web と同じ操作の phone_api.py とテスト（sci init が .sai/tools/ へ配る）
│   ├── exec/                 # 実行先の見本（local・ssh・dok・_example。sci init が .sai/exec/ へ配る）
│   ├── mind.md               # mind.md（研究の土台）のひな形
│   └── blocks/               # 27 本のブロック（役 4 本 professor・professor-backward・operator・kaizen と 23 本の sci-*）
├── apps/
│   ├── phone/                # Web の窓口（8200）
│   │   ├── src/sai_phone/    # サーバ（§4.1）
│   │   │   ├── static/       # 画面（§4.3）
│   │   │   └── extensions/   # pi の拡張
│   │   ├── src/data/         # sai_phone.db（質問・承認・pane）とログ（.gitignore 済み）
│   │   ├── hooks/            # cc のフック
│   │   ├── launchd/          # com.sai-moon.plist
│   │   └── tests/            # test_*.js（node）と test_*.py（素の python）
│   ├── radio/                # Radio の裏方サーバー（8201・TTS プロキシは 8202）
│   │   ├── src/sai_radio/
│   │   ├── config/           # podcast-creator 用の設定 JSON
│   │   └── tests/            # 3 本
│   └── mlflow-server/        # MLflow の compose（postgres・minio・mlflow・caddy。Docker で動かす）
├── docs/                     # 統合の経緯・API 一覧・コアを直す人向けの約束
├── .sai/                     # このプロジェクト自身の中のデータ（.gitignore 済み）
├── AGENTS.md                 # エージェントへの指示
├── CLAUDE.md                 # @AGENTS.md の 1 行
├── README.md                 # 3 プロセスの起動手順
└── ARCHITECTURE.md           # この文書
```

---

## 8. 今回移植しなかったもの

`sai-moon` は v1.0 の最初の統合。以下は**意図的に入れていない**。必要になったら別計画で入れる。

| 入れていないもの | 状況 |
|---|---|
| **`lib/`**（参考リポジトリ置き場） | 旧 sai-phone の `ARCHITECTURE.md` にあったが、実体は存在しなかった（`~/workdir/Sai/sai-phone/lib` は無い）。sai-moon では作っていない |
| **global の cc フック・pi 拡張** | `~/.claude/settings.json` と `~/.pi/agent/extensions/` には**入れない**。`sai-moon-link init` がプロジェクトの `.claude/settings.json`（フック 3 本）と `.pi/settings.json`（`ask_user_phone.ts`）に書く。手順は `README.md` の「エージェントとつなぐ」 |
| **launchd の登録** | `apps/phone/launchd/com.sai-moon.plist` はあるが、`launchctl load` は**していない**。旧 `com.sai-phone` は有効のまま |
| **単一ディスパッチャ CLI** | 「1 つの `sai-moon` コマンドですべてをまとめる」案は**不採用**。`sai-moon-link` と `sai-moon-paper` の 2 系統に分けたままにした。理由: 旧 sai-link・sai-paper のバイナリの分け方をそのまま引き継げ、v1.0 の差分が最も小さいため。必要になれば後からまとめる入口を足せる |
| **`src/tools/cc_usage.py`** | 旧 sai-phone にある単体ツール。移植の対象外とした（使われていない） |
| **Rust のプリセット領域** | `sai-moon-paper-tui` の `assets/`・`docs/`・`package-metadata/` は旧 sai-paper からそのまま写した。sai-moon 用の見直しはしていない |

## 9. 経緯

- 2026-09-27: v1.0。旧 `sai-link` と `sai-paper` の 8 クレートを 1 つの Cargo ワークスペース（`crates/`）に統合し、バイナリ名を `sai-moon-*` にリネーム（U01）。
  利用者全体の置き場を `~/.sai` から `~/.sai-moon` へ切り替え、`SAI_SAI_HOME` で上書きできるようにした（U01-r3）。
  旧 sai-phone の Web 層を `apps/phone/` へ移植し、既定値を 8200・`sai-moon-link`・`~/.sai-moon`・8201 に変更（U02）。
  旧 sai-radio を `apps/radio/` へ移植し、ポートを 8201 / 8202 に変更（U03）。
  文書一式（`AGENTS.md`・`README.md`・`ARCHITECTURE.md`・`docs/`）を書き直し、最終統合確認（U04）。

## 10. 次に回すもの

- sai-link: 会話の検索（`sai-moon-link search`）・Summary.md・実行の削除・runner=user の成果物の登録・子タスクの操作・Model の画面・セッションの再開。
- sai-paper: Tags・Authors・Notes・Search Sets・RAG 検索・report の表示・PDF の翻訳のボタン・引用文献の抽出・論文を block に渡すカード・研究日誌・PDF の手動取り込み・DOI / arXiv ID での取り込み。
- 両方: プロジェクトの作成（`sai-moon-link init` / `sai-moon-paper init`）と常駐の起動までの案内。
- 統合: §8 の「移植しなかったもの」のうち、cc フック・pi 拡張・launchd の実運用切り替え。
