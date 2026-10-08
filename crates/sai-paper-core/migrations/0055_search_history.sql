-- 0055_search_history.sql
-- 検索セットと選別（keep / drop）を廃止し、履歴の名前をそろえる。
--   外部検索: search_sets → search_history、search_set_items → search_history_items（set_id → history_id）
--   内部検索: find_logs → find_history、find_log_hits → find_history_hits
-- 履歴の各行に、実行者 executer（human / agent）を記録する。既存の行は消さない。

ALTER TABLE search_sets RENAME TO search_history;
ALTER TABLE search_set_items RENAME TO search_history_items;
ALTER TABLE search_history_items RENAME COLUMN set_id TO history_id;
ALTER TABLE find_logs RENAME TO find_history;
ALTER TABLE find_log_hits RENAME TO find_history_hits;

-- 既存の行の実行者: run_id がある行・origin が agent / mcp の行は agent、他は human。
ALTER TABLE search_history ADD COLUMN executer TEXT NOT NULL DEFAULT 'human';
UPDATE search_history SET executer = 'agent'
 WHERE origin IN ('agent', 'mcp') OR run_id IS NOT NULL;
ALTER TABLE find_history ADD COLUMN executer TEXT NOT NULL DEFAULT 'human';
UPDATE find_history SET executer = 'agent'
 WHERE origin IN ('agent', 'mcp') OR run_id IS NOT NULL;

-- 選別の廃止: 判定と理由の列を消す（取り込み済みの印は paper_id で表す）。
ALTER TABLE search_history_items DROP COLUMN decision;
ALTER TABLE search_history_items DROP COLUMN reason;
