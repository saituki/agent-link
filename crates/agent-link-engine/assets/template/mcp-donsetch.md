# ウェブを調べる（mcp-donsetch）

MCP サーバ `donsetch` は、ウェブの検索とページの取得を行う道具である。道具の名前は `mcp__donsetch__<道具名>` である。
ページはきれいな Markdown で返る（コードブロックと表はそのまま残る）。

| 道具 | 使いどころ |
|---|---|
| `web_search` | 何を読むかを決める。URL・題・要約の一覧だけが返り、本文は読まない |
| `web_fetch` | 決まった URL（最大 12 個まで一度に）を読む |
| `web_crawl` | 公式ドキュメントのサイトなど、複数ページをまとめて読む |

## 探す（`web_search`）

```json
{ "query": "chrome-devtools-mcp emulate viewport", "intent": "code", "max_results": 7, "deadline_ms": 60000 }
```

- `intent` を付ける。`code` は GitHub・Stack Exchange・MDN・HN を足す。`paper` は論文、`news` はニュース。
- 言い方が何通りもあるときは、`query_variants` に別の言い方を 2 つまで入れる。並べて検索される。答えの予想は入れない。
- 結果の `weak: true` は、検索エンジンどうしの一致が少ないという意味である。その結果だけを根拠にしない。
- 結果に付く `S…` の目印（S-handle）は、そのまま `web_fetch` の `url` に渡せる。

## 読む（`web_fetch`）

安い読み方から選ぶ。

| 目的 | 引数 |
|---|---|
| 「X が書いてあるか」だけ確かめる | `must_contain: "X"`（`/正規表現/` も可）。一致したかと短い抜粋だけが返る |
| 長いページのどこに何があるか | `toc: true` で見出しの一覧 → `section: "見出し"` でその節だけ |
| 探す話題が決まっている | `focus: "話題の語"`。関係する段落だけが返る |
| 前に読んだページが変わったか | `since_last: true` |
| 複数の URL | `url` を配列にし、`budget_tokens` で合計の量を決める |

- `deadline_ms` を必ず付ける（例 `60000`）。
- リンクの URL が要るときだけ `links: true` にする。
- JavaScript で中身を作るページは自動でブラウザに切り替わる。ボタンを押す必要があるときは `actions` を使う。

## サイトをまとめて読む（`web_crawl`）

1. まず `mode: "map"` で URL の一覧だけを取る。安い。
2. 必要な場所を `include_paths`（例 `["/docs/*"]`）と `focus` で絞り、`max_pages` を小さく（10 前後）して `mode: "full"` で読む。
3. 途中で止まったら、戻り値の `resume` を渡して続きを読む。

## GitHub を読む

- README・ソースは `raw.githubusercontent.com/<owner>/<repo>/<branch>/<path>` を読むと、画面の飾りが無く短い。
- issue・release・commit のページは `web_fetch` がそのまま読める。
- リポジトリ全体を調べるときは、一時フォルダに `git clone --depth 1` して `grep` で読むほうが早いことが多い。終わったら消す。
- この Mac には `gh` コマンドが無い（2026-09-28 時点で確認）。

## うまくいかないとき

- Claude Code では、`web_fetch` が本文を返さず、`title`・`total_chars` などの情報だけが返ることがある（2026-09-28 に確認）。そのときは、Bash で `curl -sL --max-time 60 <URL>` を使って読む。
- 同じ呼び出しを何度も繰り返さない。2 回失敗したら、別の URL・別の読み方に替える。

## 書き方

- 調べた事実には出典の URL を付ける。検索結果の要約（snippet）だけを根拠にせず、本文を読んで確かめる。
- 公式の文書・リポジトリを先に読み、ブログや比較記事は補いにする。食い違ったら公式を正とし、食い違ったことも書く。
- バージョンや日付で変わる情報（オプション名・既定値）には、確かめた日付か版を書く。
