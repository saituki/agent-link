# コードを記号で読む・直す（mcp-serena）

MCP サーバ `serena` は、言語サーバーを使ってコードを「記号」（関数・型・メソッドなど）の単位で扱う道具である。
ファイルを丸ごと読むより、読む量がずっと少なくて済む。道具の名前は `mcp__serena__<道具名>` である。

## 使える範囲

- 記号で扱えるのは、このプロジェクトで言語サーバーが動いている言語だけである。sai-moon では今 **Rust（`crates/`）だけ**（プロジェクト直下の `.serena/project.yml` の `language_servers`）。
- `apps/phone`・`apps/radio` の Python と JavaScript、Markdown・JSON・CSS は、serena では記号として読めない。`grep` と `Read`（または `sed -n`）で読む。
- 最初の 1 回だけ、道具が遅い（言語サーバーの起動待ち）。止まったと思わずに待つ。

## 読む順番

1. **どこにあるか分からない** → `grep -rn` で名前を探して、ファイルと記号名を決める。
2. **ファイルの中身の見取り図** → `get_symbols_overview { "relative_path": "crates/x/src/a.rs" }`。記号の一覧だけが返る。
3. **1 つの記号の本文** → `find_symbol { "name_path_pattern": "Type/method", "relative_path": "…", "include_body": true }`。
   - 型のメソッド一覧だけ欲しいときは `include_body: false`・`depth: 1`。
   - 名前の一部しか分からないときは `substring_matching: true`。
4. **誰が呼んでいるか** → `find_referencing_symbols { "name_path": "fn_name", "relative_path": "…" }`。直す関数の呼び出し元は必ずこれで全部確かめる。
5. **定義へ飛ぶ・実装を探す** → `find_declaration` / `find_implementations`（trait の実装一覧など）。
6. **コンパイルエラー・警告** → `get_diagnostics_for_file`。`cargo build` より速く、1 ファイルの問題が分かる。

- ファイルを丸ごと読んだあとで、同じファイルを serena で読み直さない。
- 互いに関係の無い呼び出しは、1 回にまとめて並べて送る。往復の回数が減る。
- 返る行番号は **0 始まり** である。文書に `path:line` を書くときは 1 を足す。

## 直す

| したいこと | 道具 |
|---|---|
| 関数・型を丸ごと書き換える | `replace_symbol_body` |
| 関数の前後に新しい記号を足す | `insert_before_symbol` / `insert_after_symbol` |
| 数行だけ直す | `replace_content`（文字列か正規表現） |
| 同じ直しを複数のファイルに | `replace_in_files`（先に `dry_run` で差分を見る） |
| 名前を変える（呼び出し元も全部） | `rename_symbol` |
| 使われていないことを確かめて消す | `safe_delete_symbol` |

- `rename_symbol` と `safe_delete_symbol` が成功したら、参照はすべて直っている。確かめのために読み直さない。
- 直したあとのビルドとテストは、依頼や計画書の「検証」のとおりに行う。

## メモ（memory）

- `list_memories` / `read_memory` で、前の実行が残したこのプロジェクトのメモを読める。名前を見て、関係するものだけ読む。
- メモを書く・消す（`write_memory` / `edit_memory` / `delete_memory`）のは、依頼で指示されたときだけにする。引き継ぎは文書（`sai-link doc write`）で行う。
