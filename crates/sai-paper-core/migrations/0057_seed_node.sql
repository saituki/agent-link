-- 0057_seed_node.sql
-- seed を Node の種類 D にする（ID は D<n>）。1 つの D の下に S（SPROUT）を複数作れるようにする。
-- S の親を NULL に限っていた CHECK を外し、親が D の S を許す。
-- 既存の行は変えない（親 NULL の S はそのまま残る）。「S の親は D」は CHECK に書けないので ops で守る。
-- rt_log・rt_searches が rt_nodes(id) を指すため、`database.rs` の `migrate()` が version 57 の前だけ
-- 外部キーの検査を一時的に外す（0053・0056 と同じ）。

CREATE TABLE rt_nodes_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('Q','A','F','E','S','D')),
    parent_id   INTEGER REFERENCES rt_nodes(id),
    guide_key   TEXT,
    text        TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    stance      TEXT CHECK (stance IS NULL OR stance IN ('support','deny','issue')),
    starred     INTEGER NOT NULL DEFAULT 0,
    cites       TEXT NOT NULL DEFAULT '[]',
    paper_id    INTEGER,
    run_id      INTEGER,
    report_id   TEXT,
    mlflow_ref  TEXT,
    ord         INTEGER NOT NULL DEFAULT 0,
    removed_at  TEXT,
    paths       TEXT NOT NULL DEFAULT '[]',
    created_by  TEXT NOT NULL DEFAULT 'user',
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((kind = 'F') = (stance IS NOT NULL)),
    CHECK (kind = 'Q' OR guide_key IS NULL),
    CHECK (kind IN ('Q','S','D') OR parent_id IS NOT NULL),
    CHECK (kind <> 'D' OR parent_id IS NULL),
    CHECK (kind = 'S' OR paths = '[]'),
    CHECK (kind = 'E' OR mlflow_ref IS NULL)
);
INSERT INTO rt_nodes_new (id, kind, parent_id, guide_key, text, body, stance, starred, cites,
    paper_id, run_id, report_id, mlflow_ref, ord, removed_at, paths, created_by, created_at, updated_at)
SELECT id, kind, parent_id, guide_key, text, body, stance, starred, cites,
    paper_id, run_id, report_id, mlflow_ref, ord, removed_at, paths, created_by, created_at, updated_at
FROM rt_nodes;
DROP TABLE rt_nodes;
ALTER TABLE rt_nodes_new RENAME TO rt_nodes;
CREATE INDEX idx_rt_nodes_parent ON rt_nodes(parent_id);
CREATE INDEX idx_rt_nodes_run ON rt_nodes(run_id);
