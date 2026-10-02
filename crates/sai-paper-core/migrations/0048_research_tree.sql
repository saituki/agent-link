-- 研究方針の木（Q・O・F・E）。古い sci_* の表は消さず、読まない。
CREATE TABLE IF NOT EXISTS rt_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rt_nodes (
    id             INTEGER PRIMARY KEY,
    kind           TEXT NOT NULL CHECK (kind IN ('Q','O','F','E')),
    parent_id      INTEGER REFERENCES rt_nodes(id),
    guide_key      TEXT,
    text           TEXT NOT NULL,
    body           TEXT NOT NULL DEFAULT '',
    answer_kind    TEXT CHECK (answer_kind IS NULL OR answer_kind IN ('answer','issue')),
    starred        INTEGER NOT NULL DEFAULT 0,
    target         INTEGER,
    cites          TEXT NOT NULL DEFAULT '[]',
    paper_id       INTEGER,
    run_id         INTEGER,
    report_id      TEXT,
    mlflow_ref     TEXT,
    ord            INTEGER NOT NULL DEFAULT 0,
    removed_at     TEXT,
    removed_reason TEXT,
    proposal_id    INTEGER,
    created_by     TEXT NOT NULL DEFAULT 'user',
    created_at     TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (kind <> 'O' OR answer_kind IS NOT NULL),
    CHECK (kind = 'O' OR (answer_kind IS NULL AND starred = 0)),
    CHECK (kind = 'Q' OR (target IS NULL AND guide_key IS NULL)),
    CHECK (kind = 'Q' OR parent_id IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS idx_rt_nodes_parent ON rt_nodes(parent_id);
CREATE INDEX IF NOT EXISTS idx_rt_nodes_run ON rt_nodes(run_id);

CREATE TABLE IF NOT EXISTS rt_logic (
    id          INTEGER PRIMARY KEY,
    q_id        INTEGER NOT NULL REFERENCES rt_nodes(id),
    value       TEXT NOT NULL CHECK (value IN ('ok','contradiction')),
    note        TEXT NOT NULL DEFAULT '',
    findings    TEXT NOT NULL DEFAULT '[]',
    f_count     INTEGER NOT NULL,
    run_id      INTEGER,
    proposal_id INTEGER,
    created_by  TEXT NOT NULL DEFAULT 'user',
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_rt_logic_q ON rt_logic(q_id);

CREATE TABLE IF NOT EXISTS rt_log (
    id          INTEGER PRIMARY KEY,
    node_id     INTEGER NOT NULL REFERENCES rt_nodes(id),
    action      TEXT NOT NULL CHECK (action IN ('star','unstar','remove','set_kind','edit','set_target','copy')),
    detail      TEXT NOT NULL DEFAULT '',
    reason      TEXT NOT NULL DEFAULT '',
    actor       TEXT NOT NULL DEFAULT 'user',
    proposal_id INTEGER,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TRIGGER IF NOT EXISTS rt_log_no_update BEFORE UPDATE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
CREATE TRIGGER IF NOT EXISTS rt_log_no_delete BEFORE DELETE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;

CREATE TABLE IF NOT EXISTS rt_proposals (
    id          INTEGER PRIMARY KEY,
    run_id      INTEGER,
    block       TEXT,
    op          TEXT NOT NULL,
    node_id     INTEGER,
    payload     TEXT NOT NULL,
    reason      TEXT NOT NULL DEFAULT '',
    risk        TEXT NOT NULL CHECK (risk IN ('auto','human')),
    status      TEXT NOT NULL CHECK (status IN ('pending','applied','accepted','rejected','reverted')),
    result_ref  TEXT,
    decided_by  TEXT,
    decided_at  TEXT,
    decide_note TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_rt_proposals_status ON rt_proposals(status);
CREATE INDEX IF NOT EXISTS idx_rt_proposals_run ON rt_proposals(run_id);

CREATE TABLE IF NOT EXISTS rt_seeds (
    id         INTEGER PRIMARY KEY,
    title      TEXT NOT NULL,
    sentence   TEXT NOT NULL,
    path       TEXT NOT NULL DEFAULT '[]',
    run_id     INTEGER,
    removed_at TEXT,
    created_by TEXT NOT NULL DEFAULT 'user',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS rt_searches (
    id           INTEGER PRIMARY KEY,
    run_id       INTEGER,
    node_id      INTEGER REFERENCES rt_nodes(id),
    tool         TEXT NOT NULL,
    query        TEXT NOT NULL,
    scope        TEXT NOT NULL DEFAULT '',
    widen_reason TEXT NOT NULL DEFAULT '',
    hits         INTEGER NOT NULL DEFAULT 0,
    hit_papers   TEXT NOT NULL DEFAULT '[]',
    set_id       INTEGER,
    created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_rt_searches_node ON rt_searches(node_id);

ALTER TABLE search_sets ADD COLUMN rt_node INTEGER;
