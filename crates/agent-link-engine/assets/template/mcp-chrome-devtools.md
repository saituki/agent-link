# 画面を実物で調べる（mcp-chrome-devtools）

MCP サーバ `chrome-devtools` は、画面の出ない Chrome を開き、ページの中身・エラーの記録・通信・CSS を読む道具である。道具の名前は `mcp__chrome-devtools__<道具名>` である。
**画面の不具合は、JS や CSS を読んで想像する前に、まず実物を見る。** 大きいファイル（`papers.js`・`app.js`・`style.css` など）を読み込むより、ずっと少ない量で原因の場所が分かる。

- ブラウザは道具を初めて使ったときに起動し、中身は毎回まっさらである（`--isolated`）。ログインや前の状態は残っていない。
- ほとんどの道具に `pageId`（今のタブの番号）が要る。最初は `1`。分からなければ `list_pages` で見る。

## 見る順番

1. **開く** → `navigate_page { "pageId": 1, "type": "url", "url": "http://127.0.0.1:8400/#/home" }`
   - 画面の住所（`#/…`）は `docs/integration.md` の一覧にある。
2. **幅を決める** → `emulate`
   - iPhone: `{ "pageId": 1, "viewport": "390x844x3,mobile,touch" }`
   - PC: `{ "pageId": 1, "viewport": "1280x900x1" }`（900px 以上で 2 列になる）
   - 不具合がどちらの幅で出るか分からなければ、両方で見る。
3. **中身を文字で読む** → `take_snapshot { "pageId": 1 }`。見出し・ボタン・文字が `uid` 付きの一覧で返る。**画面写真より先にこれを使う。** 量が少なく、`uid` で次の操作ができる。
4. **エラーを読む** → `list_console_messages { "pageId": 1, "types": ["error", "warn"] }`。くわしくは `get_console_message { "msgid": … }`（元のファイルの行が出る）。
5. **通信を読む** → `list_network_requests { "pageId": 1, "resourceTypes": ["fetch", "xhr"] }` で API の呼び出しだけを見る。中身は `get_network_request { "reqid": … }`。400・500・503 はここで分かる。
6. **見た目の原因** → `get_css_styles { "pageId": 1, "uid": "<一番新しい snapshot の uid>" }`。`take_snapshot` を撮り直すと `uid` が変わり、前の `uid` は「not found」になる。効いている CSS の規則と、`style.css:196` のようなファイルと行が返る。打ち消された規則には `[overloaded]` が付く。
7. **値を直接確かめる** → `evaluate_script { "pageId": 1, "function": "() => document.querySelector('.x')?.getBoundingClientRect()" }`。読むだけのときは `"waitForStableDom": false`。
8. **最後に写真** → 見た目そのものが問題のときだけ `take_screenshot { "pageId": 1, "format": "jpeg", "quality": 60 }`。報告に付けるなら `filePath` で一時フォルダに保存する。

## 操作する

- `click` / `fill` / `fill_form` / `press_key` / `hover` は、`take_snapshot` の `uid` で要素を指す。操作のあとは、もう一度 `take_snapshot` してから次の `uid` を使う（古い `uid` は使えない）。
- 読み込みを待つときは `wait_for { "pageId": 1, "text": ["<出るはずの文字>"] }`。決まった秒数を待たない。
- 確認の窓が出たら `handle_dialog`。

## agent-link での注意

- **使うのは 8400 番（agent-link の Web）と確認用の 8297 番だけ。** 8200〜8202 番は sai-moon の物なので開かない。サーバの起動と停止は `AGENTS.md` の約束に従う。
- **8400 番のサーバがどのフォルダから動いているかを確かめる**（`lsof -nP -iTCP:8400 -sTCP:LISTEN` の PID を `ps -o command= -p <PID>` で見る）。本体のフォルダのサーバは、作業用コピー（worktree）で直したファイルを配らない。
- **古い画面が出ることがある。** `sw.js` が画面のファイルを溜めて（キャッシュして）いる。ファイルを直したあとは、次を実行してから `navigate_page { "pageId": 1, "type": "reload", "ignoreCache": true }` する。

```js
async () => { for (const r of await navigator.serviceWorker.getRegistrations()) await r.unregister(); for (const k of await caches.keys()) await caches.delete(k); return 'ok'; }
```

- 画面で書き込みの操作（保存・削除・実行・送信のボタン）はしない。依頼で指示されたときだけ、確認用の物（例: block `guide-check`）で行い、最後に消す。
- 見たことは、道具の名前と返った値を報告にそのまま写す（例: 「`list_console_messages`: `TypeError: … (papers.js:812)`」）。
