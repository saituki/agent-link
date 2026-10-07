# 実装計画書 U05: Mac 専用の機能を削る（遠隔ブラウザ・launchd・キーチェーン・5 時間枠）

## 目的
Linux サーバでは動かない・意味の無い Mac 専用の機能を sai-tree から消す。消したあとも、ほかの機能（Papers の手動取り込み・PDF 翻訳の API・`paper.env` の普通の `KEY=value`・Usage のほかのカード）は今のまま動く。

## 背景
- sai-tree は sai-moon を複製した Linux の多人数サーバ版である（要件: Mac 専用の機能を削る）。
- 消す物の定義・呼び出し元・テスト・文書の全一覧は、調査 E3（`docs/plan/research/E3-mac-only-removal.md` の §1〜§4）にある。この計画書はそこから必要な所を写している。

## 前提（着手時点のコードの状態）
- ガイド 1・2（U01〜U04）が終わっている。statusLine の CLI（`sai-link cc-statusline`）は U02 が消した。Web の herdr は U03・U04 が消した。
- 遠隔ブラウザ:
  - サーバ `apps/web/src/sai_web/rbrowser.py`（493 行）、画面 `static/rbrowser.js`・`static/rbrowser.css`。
  - API は `main.py:1024-1073` の 6 本（`POST …/papers/browser/open`・`GET …/browser/state`・`GET …/browser/screen`・`POST …/browser/input`・`POST …/browser/import`・`DELETE …/papers/browser`）。`main.py:40` の import、`main.py:131` の `_shutdown` の `rbrowser.close()`。
  - `papers.py:37` の import と `papers.py:1495` `request_browser_import`（`:1509` が `rbrowser.download_path` を呼ぶ）。
  - 画面の入口: `static/papers.js:2705-2716`・`:2898-2904`（「Mac のブラウザで開く」ボタン。`typeof rbOpenHash === "function"` で守られている）、`static/index.html:26`（css）・`:511-512`（`<section id="view-papers-browser">`）・`:662`（script）、`static/sw.js:56`（コメント）・`:91`（`ASSETS` の 2 つ）。
  - テスト: `apps/web/tests/test_rbrowser.js`・`test_rbrowser.py`。**`test_rbrowser.py` の `test_translate_pdf`（`:143`）と `test_routes` の最後の 1 行（`:258` `{paper_id:int}/translate-pdf`）は遠隔ブラウザと無関係なので、先に別のファイルへ移す。**
- launchd: `apps/web/launchd/com.sai.web.plist.template` の 1 ファイルだけ。コードからは呼ばれない。
- キーチェーン: `crates/sai-paper-core/src/paths.rs:115`（`load_keychain_env(&base)` の呼び出し）・`:120-150` `load_keychain_env`・`:153` `KeychainLine`・`:162` `keychain_lines`・`:215` `shell_words`・`:247-266` `keychain_value`・テスト `:656-696` `mod keychain_tests`。
- 5 時間枠: `apps/web/src/sai_web/usage.py:35` `cc_limits_path`・`:99` `cc_limits`・`:434`（`summary()` の `"cc": cc_limits()`）・冒頭の説明 `:5`。画面 `static/usage.js:137-168` `usageCcCard`・`:275`（`usageData.cc`）・`:36` `usageSavedAgo`・`:54` `usageResetText`（5 時間枠だけが使うなら消す）。テスト `test_usage.py:269-`。

## 変更内容

### 1. 遠隔ブラウザを消す
1. `test_rbrowser.py` の `test_translate_pdf` と、`translate-pdf` のルートの登録を確かめる 1 行を、新しいファイル `apps/web/tests/test_papers_translate.py` に移す（中身は変えない。必要な import と偽の CLI の準備を一緒に写す）。
2. `rbrowser.py`・`static/rbrowser.js`・`static/rbrowser.css`・`test_rbrowser.js`・`test_rbrowser.py` を消す。
3. `main.py` の 6 本の API・import・`_shutdown` の 1 行を消す。`papers.py` の import と `request_browser_import` を消す。
4. `papers.js` の 2 か所のボタンと説明文を消す。`index.html` の 3 か所を消す。`sw.js` の `ASSETS` から 2 つを消し、コメントを直し、`CACHE` を 1 つ上げる。`index.html` の `?v=` をそろえる。
5. `docs/integration.md:216-220`・`:389-390` の遠隔ブラウザの記述を消す。

### 2. launchd を消す
- `apps/web/launchd/` を消す。`apps/web/src/sai_web/config.py:44` のコメント（launchd の話）を直す。文書（README・ARCHITECTURE・AGENTS・`CONCEPT.md`）は U15 が書き直すので、この作業では触らない。

### 3. キーチェーンを消す
- `paths.rs` の上の範囲を消す。`load_project_dotenv` は `dotenvy::from_path(base)` だけを呼ぶ形にする。使われなくなる import（`dotenvy::from_read_override` など）を整理する。`paper.env` の普通の `KEY=value` は今のまま読む。
- `crates/sai-paper-cli/src/main.rs:1109` の古い文言「`.env か ~/.sai-moon/paper.env に置いてください`」を「`.env か ~/.sai/paper.env に置いてください`」に直す。

### 4. 5 時間枠を消す
- `usage.py` の `cc_limits_path`・`cc_limits`・`summary()` の `"cc"` キー・冒頭の説明を消す。
- `usage.js` の `usageCcCard` と、それだけが使う補助（`usageSavedAgo`・`usageResetText` がほかで使われていなければ）を消す。`usageData.cc` を読む所を消す。
- `test_usage.py` の 5 時間枠のテストを消す。`docs/integration.md:50,76` の記述を直す。
- cc の費用（`cost_usd`・`cost_source`）は U01 が `result` 行から書くので、Usage と統計の費用の表示はそのまま残す。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- `GET /api/usage` の応答から `cc` のキーが無くなる。ほかのキーは変えない。
- `/api/projects/{name}/papers/browser*` の住所は無くなる。

## ファイルの所有権
- 書いてよい: `apps/web/src/sai_web/rbrowser.py`（消す）・`main.py`（遠隔ブラウザの部分だけ）・`papers.py`（遠隔ブラウザの部分だけ）・`usage.py`・`config.py`（コメントだけ）・`static/rbrowser.js`・`rbrowser.css`（消す）・`static/papers.js`（2 か所だけ）・`static/usage.js`・`static/index.html`・`static/sw.js`・`apps/web/tests/test_rbrowser.*`（消す）・`test_papers_translate.py`（新規）・`test_usage.py`・`apps/web/launchd/`（消す）・`crates/sai-paper-core/src/paths.rs`・`crates/sai-paper-cli/src/main.rs`（1 行だけ）・`docs/integration.md`
- 触らない: README・ARCHITECTURE・AGENTS・`CONCEPT.md`（U15 が書く）

## 完了条件
- [ ] AC1: `grep -rn "rbrowser\|rbOpenHash" apps/web` が 0 件。`apps/web/launchd/` が無い。`grep -n "security\|keychain" crates/sai-paper-core/src/paths.rs` が 0 件。`grep -rn "cc_limits\|cc-rate-limits\|usageCcCard" apps/web` が 0 件。
- [ ] AC2: `test_papers_translate.py` が通り、PDF 翻訳のテストが残っている。
- [ ] AC3: Rust・Web（JS・Python）のテストがすべて通る。
- [ ] AC4: 8299 番のサーバで、Papers の論文の詳細と Usage の画面が崩れない（390px と 1280px の画面写真を作業報告書に付ける）。

## 検証
```bash
cd ~/workdir/Sai/sai-tree
env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME \
  perl -e 'alarm 1800; exec @ARGV' cargo test --workspace
for f in apps/web/tests/test_*.js; do node "$f" || exit 1; done
for f in apps/web/tests/test_*.py; do PYTHONPATH=apps/web/src apps/web/.venv/bin/python "$f" || exit 1; done
```

## 決定事項と理由
- 遠隔ブラウザ・launchd・キーチェーン・5 時間枠を消す — 理由: 利用者の決定。Linux サーバでは動かない（Chrome の `.app`・`/usr/bin/security`・launchd）か、headless では値が来ない（statusLine）。
- PDF 翻訳のテストは消さずに移す — 理由: 遠隔ブラウザと無関係のテストが同じファイルにあるため（調査 E3 §1.3）。

## 要確認
- なし。
