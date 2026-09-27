-- 0019_citation_count.sql
-- 論文リスト表示（R4）のための被引用数カラム追加。
--
-- 変更内容:
--   1. papers テーブルに citation_count INTEGER カラムを追加（NULL 許容）
--      - S2 検索ヒット・ダウンロード完了時に自動補完する
--      - 引用数が取得できない既存論文は NULL のまま（表示は '-'）

ALTER TABLE papers ADD COLUMN citation_count INTEGER;
