-- 0049_search_actor.sql
-- 検索の記録（search_sets）に、検索を実行した sai-link の run_id を残す。
-- Agent の検索は run_id が入り、窓口（人）の検索は NULL。

ALTER TABLE search_sets ADD COLUMN run_id INTEGER;
