-- 段階 6: 検索の記録を検索セット（search_sets）1 つにまとめる。
-- Discover の自動記録（search_logs）を検索セットへ写し、古い表を消す。
ALTER TABLE search_sets ADD COLUMN year TEXT;
ALTER TABLE search_sets ADD COLUMN limit_txt TEXT;
ALTER TABLE search_sets ADD COLUMN origin TEXT NOT NULL DEFAULT 'cli'; -- discover / cli / agent / phone
ALTER TABLE search_sets ADD COLUMN searched_at TEXT;                   -- 検索した日時（ISO8601）
ALTER TABLE search_sets ADD COLUMN log_date TEXT;                      -- Library の階層表示の日付（YYYY/MM/DD）

-- 既存の検索セット（CLI / エージェントが作ったもの）
UPDATE search_sets
   SET searched_at = created_at,
       log_date = replace(substr(created_at, 1, 10), '-', '/'),
       origin = CASE created_by WHEN 'agent' THEN 'agent' ELSE 'cli' END;

-- Discover の検索ログ → 検索セット（名前は重ならないよう discover-log-<旧番号>）。
-- 検索理由（reason）は検索セットのメモ（note）に入れる。
INSERT INTO search_sets (name, source_dbs, query, created_by, note, created_at,
                         year, limit_txt, origin, searched_at, log_date)
SELECT 'discover-log-' || id, COALESCE(source_dbs, ''), query, 'user', reason, searched_at,
       year, limit_txt, 'discover', searched_at, log_date
  FROM search_logs;

INSERT INTO search_set_items (set_id, position, remote_json, dedupe_key, decision, added_at)
SELECT s.id, i.position, i.remote_json, i.dedupe_key, 'pending', s.searched_at
  FROM search_log_items i
  JOIN search_sets s ON s.name = 'discover-log-' || i.log_id;

DROP TABLE search_log_items;
DROP TABLE search_logs;

CREATE INDEX IF NOT EXISTS idx_search_set_items_dedupe_key ON search_set_items(dedupe_key);
