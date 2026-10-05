-- 0051_find_log.sql
-- 内部検索（find）の履歴。外部検索の search_sets とは別に持つ。
CREATE TABLE find_logs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    searched_at   TEXT    NOT NULL,
    log_date      TEXT    NOT NULL,
    query         TEXT    NOT NULL,
    mode          TEXT    NOT NULL DEFAULT '',
    top_k         INTEGER,
    scope         TEXT    NOT NULL DEFAULT '',
    range_text    TEXT    NOT NULL DEFAULT '',
    papers_json   TEXT    NOT NULL DEFAULT '[]',
    node_id       INTEGER,
    subtree       INTEGER NOT NULL DEFAULT 0,
    each_m        INTEGER,
    widen_reason  TEXT    NOT NULL DEFAULT '',
    origin        TEXT    NOT NULL,
    run_id        INTEGER
);
CREATE TABLE find_log_hits (
    log_id    INTEGER NOT NULL REFERENCES find_logs(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    paper_id  INTEGER,
    hit_json  TEXT    NOT NULL,
    PRIMARY KEY (log_id, position)
);
CREATE INDEX idx_find_logs_searched ON find_logs(searched_at);
