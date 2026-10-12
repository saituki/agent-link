# ontology の決まり（sci-onto）

医療の用語を ontology（MONDO＝疾患、MeSH＝医学用語）で引く命令の使い方です。`lab-*` の block と `sci-lookup` が読みます。`sci-core` と合わせて守ります。

## 何か
- `sai-sci onto` は、共有データ（環境変数 `SAI_SHARED_DIR` の `onto.db`）を読むだけの命令です。外部へ通信しません。
- **使うかどうかは、あなたが決めます。** 使わなくても作業は止めません。
- **返すのは事実だけです。** 警告・点数・閾値はありません。粒度が合っているか、どこに置くかは、あなたが判断します。

## 命令
```bash
sai-sci onto lookup "<用語>" [--source mondo|mesh] --json
sai-sci onto show <ID> --json
sai-sci onto relation <ID_A> <ID_B> --json
sai-sci onto ancestors <ID> --json
```

- `lookup`: 別名と旧称も当たります。完全一致、前方一致、部分一致の順に返します。日本語の語は英語に直してから引きます。
- `show`: 1 件の詳細（親・子・別名・xref）。
- `relation`: 2 件の関係。`relation` の値は次のとおりです。
  - `same`: 同じ概念
  - `descendant`: a は b の子孫
  - `ancestor`: a は b の祖先
  - `unrelated`: 関係が無い。`nearest_common_ancestors` に共通の祖先を出す
  - `different_source`: 出典が違う（MONDO と MeSH）
- `ancestors`: 祖先を距離の近い順に出す。
- ID の形は `MONDO:0005027`・`MESH:D013036` です。
- 終了コード: 0 は成功。2 は引数・ID の形の誤り。3 は共有データか ID が無い。1 はその他。

## 使いどころ
- 新しい A を足す前に、既存の親と兄弟の A を `relation` で比べ、祖先関係を確かめる。
- 旧称や別名が同じ概念か確かめる（例: `West syndrome` と `IESS`）。
- review で、兄弟の詳しさがそろうか `ancestors` と `show` で確かめる。

## 読み方の注意
1. **map は正解ではありません。** 食い違っても、論文と臨床の慣習を優先してよいです。その理由を報告に書きます。
2. **多重親があり、`depth` は参考値です。** 深さは粒度を表しません。祖先関係（`relation`・`ancestors`）で見ます。
3. **医療の用語だけが対象です。** 動物種・計算モデル・手法（method）の A には使えません。当たらなくてよいです。
4. **ID は外部の map と対応づけたときだけ書きます。** 報告か、A の本文の注記に書きます。A の文には ID を書きません。`map_version` も同じ所に書いてよいです。
5. **使えなかったとき**（終了コード 3 で共有データが無い）は、報告に「ontology tool が使えなかった」と書いて作業を続けます。ID が無いだけなら、別の語で引き直します。
