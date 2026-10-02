# 検索の決まり（sci-find）

検索するブロックが読みます。`sci-core` と合わせて守ります。

## 検索
- 検索には `--node <ID>` を付けます。記録に残り、`# context` の `## SEARCH_HISTORY` と Web の画面に出ます。付けないと、どの Node のための検索かが残りません。
- 日本語の問いは英語の検索語に直してから検索します。3〜5 個の小さな問いに分けることを勧めます。
- 検索の前に `## SEARCH_HISTORY` を読み、同じ検索を繰り返しません。

```bash
sai-moon-paper find "<英語の検索語>" --node Q12 --mode word --json
sai-moon-paper find "<英語の検索語>" --node O5 --subtree --each 3 --json
sai-moon-paper find "<英語の検索語>" --widen "Node の論文で 0 件だったため" --node Q12 --json
sai-moon-paper search "<英語の検索語>" --db s2 --save-as <名前> --node Q12 --purpose "<目的>" --extract "<抜き出すもの>" --json
```

## 広げる順
Node の F の論文 → 部分木（`--subtree`）→ 検索セット → Library → 外部。
広げたら `--widen <理由>` を付けます。理由が残っていないと、次に同じことを繰り返します。

## 検索セット
- 検索セット（`--save-as`）を作ったら、選別（keep / drop）が終わるまで Action は終わりません（DEBT の「未選別」）。
- 選別は `sci-screen` か、利用者が Web の検索セットの画面で行います。選別が済んでいなければ、report の「## OUT_OF_SCOPE」ではなく本文に「未選別」と書きます。
