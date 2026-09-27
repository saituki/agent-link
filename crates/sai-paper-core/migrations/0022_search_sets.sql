-- 0022_search_sets.sql
-- 検索セット: リモート検索結果の永続化と注釈付け
--
-- 候補はまだ `papers` テーブルに入らない。DL / import した時点で
-- `papers` に行ができ、`paper_id` が埋まり `decision='downloaded'` になる。

CREATE TABLE IF NOT EXISTS search_sets (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    source_dbs  TEXT NOT NULL,
    query       TEXT NOT NULL,
    raw_query   TEXT,
    created_by  TEXT NOT NULL DEFAULT 'user',
    note        TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS search_set_items (
    set_id      INTEGER NOT NULL REFERENCES search_sets(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    remote_json TEXT NOT NULL,
    dedupe_key  TEXT NOT NULL,
    paper_id    INTEGER REFERENCES papers(id),
    decision    TEXT NOT NULL DEFAULT 'pending'
                CHECK (decision IN ('pending', 'keep', 'drop', 'downloaded')),
    reason      TEXT,
    added_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (set_id, position)
);

CREATE INDEX IF NOT EXISTS idx_set_items_dedupe
    ON search_set_items(set_id, dedupe_key);
