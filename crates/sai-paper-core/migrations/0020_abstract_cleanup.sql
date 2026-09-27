-- 0020_abstract_cleanup.sql
-- PAPER 画面の abstract に混入していた JATS XML・分類情報・余分な空白の除去。
--
-- 背景:
--   - Crossref が返す abstract には `<jats:title>` / `<jats:p>` / `<jats:list>` 等の
--     JATS タグと、本文と無関係な分類情報（`This article is categorized under: ...`）が
--     混入する。旧実装は `<jats:p>` / `<jats:sec>` のみ除去していたため、残りのタグが
--     DB（`papers.abstract`）に保存され、PAPER 画面にそのまま表示されていた。
--   - 日本語訳（`papers.abstract_ja`）にも JATS 残骸・分類訳文・改行が残っていた。
--
-- 変更内容:
--   1. `papers.abstract` から JATS タグ由来の残骸を除去する。
--      - `<jats:` / `jats:` / `<jats` を含む行は分類リスト等の残骸とみなし削除する。
--      - 残った `<...>` タグ断片は削除する。
--      - `This article is categorized under:`（英語）および
--        `この記事は次のように分類されています` / `この論文は次のように分類されています`
--        （日本語訳）以降は本文と無関係なため切り捨てる。
--      - HTML エンティティ（`&gt;` 等）をデコードする。
--   2. `papers.abstract_ja` も同様に整形し、改行を全てなくす（1 段落の連続文に平坦化）。
--
-- 注意: SQLite の `replace()` / `instr()` / `substr()` のみで実装する。
-- タグ除去は Rust 側の `abstracts::sanitize_abstract` / `flatten_abstract_ja` が
-- 正規の処理であり、こちらは既存 DB 行の救済用である。

-- 1a. 英語の分類情報マーカー以降を切り捨てる（abstract）。
UPDATE papers
SET abstract = substr(abstract, 1, instr(abstract, 'This article is categorized under:') - 1)
WHERE abstract LIKE '%This article is categorized under:%';

-- 1b. 日本語訳に残った分類訳文以降を切り捨てる（abstract_ja）。
UPDATE papers
SET abstract_ja = substr(abstract_ja, 1, instr(abstract_ja, 'この記事は次のように分類されています') - 1)
WHERE abstract_ja LIKE '%この記事は次のように分類されています%';

UPDATE papers
SET abstract_ja = substr(abstract_ja, 1, instr(abstract_ja, 'この論文は次のように分類されています') - 1)
WHERE abstract_ja LIKE '%この論文は次のように分類されています%';

-- 2. JATS タグ残骸の除去（abstract / abstract_ja 共通）。
-- `<jats:title>ABSTRACT</jats:title>` のような見出しは内容ごと除去する。
-- `<jats:italic toggle="yes">...` のように属性付き・他タグ名の JATS もあるため、
-- `<jats:` を含む行全体ではなくタグ部分のみを空白化する。SQLite には正規表現が無いため、
-- 代表形を列挙して置換する（残りは Rust 側 `sanitize_abstract` が表示時に除去する）。
UPDATE papers SET abstract = replace(abstract, '<jats:title>ABSTRACT</jats:title>', ' ')
WHERE abstract LIKE '%<jats:title>ABSTRACT</jats:title>%';
UPDATE papers SET abstract = replace(abstract, '<jats:title>Abstract</jats:title>', ' ')
WHERE abstract LIKE '%<jats:title>Abstract</jats:title>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:title>要約</jats:title>', ' ')
WHERE abstract_ja LIKE '%<jats:title>要約</jats:title>%';

-- 残りの `<jats:...>` 閉じタグは空白に置換する（単語連結防止）。
UPDATE papers SET abstract = replace(abstract, '</jats:p>', ' ')
WHERE abstract LIKE '%</jats:p>%';
UPDATE papers SET abstract = replace(abstract, '</jats:list>', ' ')
WHERE abstract LIKE '%</jats:list>%';
UPDATE papers SET abstract = replace(abstract, '</jats:sec>', ' ')
WHERE abstract LIKE '%</jats:sec>%';
UPDATE papers SET abstract = replace(abstract, '</jats:title>', ' ')
WHERE abstract LIKE '%</jats:title>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:p>', ' ')
WHERE abstract_ja LIKE '%</jats:p>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:list>', ' ')
WHERE abstract_ja LIKE '%</jats:list>%';

-- 開きタグ（属性付きを含む）は完全一致で置換できないため、代表的な形を除去する。
-- 属性付き `<jats:list list-type="simple">` / `<jats:list-item>` 系が主な残骸である。
UPDATE papers SET abstract = replace(abstract, '<jats:p>', ' ')
WHERE abstract LIKE '%<jats:p>%';
UPDATE papers SET abstract = replace(abstract, '<jats:sec>', ' ')
WHERE abstract LIKE '%<jats:sec>%';
UPDATE papers SET abstract = replace(abstract, '<jats:list list-type="simple">', ' ')
WHERE abstract LIKE '%<jats:list list-type="simple">%';
UPDATE papers SET abstract = replace(abstract, '<jats:list-item>', ' ')
WHERE abstract LIKE '%<jats:list-item>%';
UPDATE papers SET abstract = replace(abstract, '</jats:list-item>', ' ')
WHERE abstract LIKE '%</jats:list-item>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:p>', ' ')
WHERE abstract_ja LIKE '%<jats:p>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:list list-type="simple">', ' ')
WHERE abstract_ja LIKE '%<jats:list list-type="simple">%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:list-item>', ' ')
WHERE abstract_ja LIKE '%<jats:list-item>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:list-item>', ' ')
WHERE abstract_ja LIKE '%</jats:list-item>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:リスト項目>', ' ')
WHERE abstract_ja LIKE '%<jats:リスト項目>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:リスト項目>', ' ')
WHERE abstract_ja LIKE '%</jats:リスト項目>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:list>', ' ')
WHERE abstract_ja LIKE '%<jats:list>%';

-- `<jats:bold>見出し:</jats:bold>` / `<jats:monospace>用語</jats:monospace>` のような
-- 強調タグは内容を残す（タグのみ除去）。
UPDATE papers SET abstract = replace(abstract, '<jats:bold>', ' ')
WHERE abstract LIKE '%<jats:bold>%';
UPDATE papers SET abstract = replace(abstract, '</jats:bold>', ' ')
WHERE abstract LIKE '%</jats:bold>%';
UPDATE papers SET abstract = replace(abstract, '<jats:monospace>', ' ')
WHERE abstract LIKE '%<jats:monospace>%';
UPDATE papers SET abstract = replace(abstract, '</jats:monospace>', ' ')
WHERE abstract LIKE '%</jats:monospace>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:bold>', ' ')
WHERE abstract_ja LIKE '%<jats:bold>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:bold>', ' ')
WHERE abstract_ja LIKE '%</jats:bold>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:monospace>', ' ')
WHERE abstract_ja LIKE '%<jats:monospace>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:monospace>', ' ')
WHERE abstract_ja LIKE '%</jats:monospace>%';

-- `<jats:ext-link ...>URL` のような外部リンクは URL 本文を残す（タグ開始部のみ除去）。
-- 属性値が DOI / URL ごとに異なるため、共通接頭辞で置換する。
UPDATE papers SET abstract = replace(abstract, '<jats:ext-link ', ' ')
WHERE abstract LIKE '%<jats:ext-link %';
UPDATE papers SET abstract = replace(abstract, '</jats:ext-link>', ' ')
WHERE abstract LIKE '%</jats:ext-link>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:ext-link ', ' ')
WHERE abstract_ja LIKE '%<jats:ext-link %';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:ext-link>', ' ')
WHERE abstract_ja LIKE '%</jats:ext-link>%';

-- 属性付き・他タグ名の JATS（例: `<jats:italic toggle="yes">`）。内容（強調語）は残す。
UPDATE papers SET abstract = replace(abstract, '<jats:italic toggle="yes">', ' ')
WHERE abstract LIKE '%<jats:italic toggle="yes">%';
UPDATE papers SET abstract = replace(abstract, '</jats:italic>', ' ')
WHERE abstract LIKE '%</jats:italic>%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '<jats:italic toggle="yes">', ' ')
WHERE abstract_ja LIKE '%<jats:italic toggle="yes">%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '</jats:italic>', ' ')
WHERE abstract_ja LIKE '%</jats:italic>%';

-- 3. HTML エンティティのデコード（残存が多い `&gt;` / `&lt;` / `&amp;` のみ）。
UPDATE papers SET abstract = replace(abstract, '&gt;', '>')
WHERE abstract LIKE '%&gt;%';
UPDATE papers SET abstract = replace(abstract, '&lt;', '<')
WHERE abstract LIKE '%&lt;%';
UPDATE papers SET abstract = replace(abstract, '&amp;', '&')
WHERE abstract LIKE '%&amp;%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '&gt;', '>')
WHERE abstract_ja LIKE '%&gt;%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '&lt;', '<')
WHERE abstract_ja LIKE '%&lt;%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '&amp;', '&')
WHERE abstract_ja LIKE '%&amp;%';

-- 4. 余分な空白の圧縮（3 文字以上の連続空白を 1 文字に。複数回適用で畳む）。
UPDATE papers SET abstract = replace(abstract, '   ', ' ')
WHERE abstract LIKE '%   %';
UPDATE papers SET abstract = replace(abstract, '  ', ' ')
WHERE abstract LIKE '%  %';
UPDATE papers SET abstract_ja = replace(abstract_ja, '   ', ' ')
WHERE abstract_ja LIKE '%   %';
UPDATE papers SET abstract_ja = replace(abstract_ja, '  ', ' ')
WHERE abstract_ja LIKE '%  %';

-- 5. 日本語訳の改行を全てなくす（改行・復帰を空白に置換した後、連続空白を畳む）。
UPDATE papers SET abstract_ja = replace(abstract_ja, char(13) || char(10), ' ')
WHERE abstract_ja LIKE '%' || char(13) || char(10) || '%';
UPDATE papers SET abstract_ja = replace(abstract_ja, char(10), ' ')
WHERE abstract_ja LIKE '%' || char(10) || '%';
UPDATE papers SET abstract_ja = replace(abstract_ja, char(13), ' ')
WHERE abstract_ja LIKE '%' || char(13) || '%';
UPDATE papers SET abstract_ja = replace(abstract_ja, '  ', ' ')
WHERE abstract_ja LIKE '%  %';

-- 6. 先頭に残った改行・空白を除去する（`clean_crossref_abstract` は見出し除去後の
-- 先頭空白を畳むが、SQL では段落整形まで行わないため最低限の前処理に留める）。
UPDATE papers SET abstract = ltrim(abstract, ' ' || char(10) || char(13) || char(9))
WHERE abstract != ltrim(abstract, ' ' || char(10) || char(13) || char(9));
UPDATE papers SET abstract_ja = ltrim(abstract_ja, ' ' || char(10) || char(13) || char(9))
WHERE abstract_ja != ltrim(abstract_ja, ' ' || char(10) || char(13) || char(9));

-- 7. 前後の空白を除去する。
UPDATE papers SET abstract = trim(abstract) WHERE abstract != trim(abstract);
UPDATE papers SET abstract_ja = trim(abstract_ja) WHERE abstract_ja != trim(abstract_ja);
