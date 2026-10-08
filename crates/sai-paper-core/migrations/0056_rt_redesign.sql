-- 0056_rt_redesign.sql
-- research-tree を Q・A・F・E・S に作り直す。O・answer_kind・target・rt_logic・rt_proposals を廃止し、
-- F の stance と、質問で取った承認（rt_approvals）を足す。既存の research-tree のデータは無い前提で、行は消す。
DROP TABLE IF EXISTS rt_logic;
DROP TABLE IF EXISTS rt_proposals;
DROP TABLE IF EXISTS rt_log;            -- 追記だけのトリガごと消す
DELETE FROM rt_searches;
UPDATE search_history SET rt_node = NULL WHERE rt_node IS NOT NULL;
DELETE FROM rt_meta;                    -- 未初期化に戻す（init --type で作り直す）
DROP TABLE rt_nodes;

CREATE TABLE rt_nodes (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('Q','A','F','E','S')),
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
    CHECK (kind IN ('Q','S') OR parent_id IS NOT NULL),
    CHECK (kind <> 'S' OR parent_id IS NULL),
    CHECK (kind = 'S' OR paths = '[]'),
    CHECK (kind = 'E' OR mlflow_ref IS NULL)
);
CREATE INDEX idx_rt_nodes_parent ON rt_nodes(parent_id);
CREATE INDEX idx_rt_nodes_run ON rt_nodes(run_id);

CREATE TABLE rt_log (
    id          INTEGER PRIMARY KEY,
    node_id     INTEGER NOT NULL REFERENCES rt_nodes(id),
    action      TEXT NOT NULL CHECK (action IN ('add_q','insert_q','move_a','copy','remove',
                    'star','unstar','edit','set_stance','set_mlflow')),
    detail      TEXT NOT NULL DEFAULT '',
    actor       TEXT NOT NULL DEFAULT 'user',
    approved_by TEXT,
    qid         TEXT,
    question    TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TRIGGER rt_log_no_update BEFORE UPDATE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
CREATE TRIGGER rt_log_no_delete BEFORE DELETE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;

CREATE TABLE rt_approvals (
    id          INTEGER PRIMARY KEY,
    run_id      INTEGER NOT NULL,
    qid         TEXT NOT NULL UNIQUE,
    question    TEXT NOT NULL,
    answer      TEXT NOT NULL DEFAULT '',
    answered_by TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    consumed_at TEXT,
    consumed_ops TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX idx_rt_approvals_run ON rt_approvals(run_id, consumed_at);
