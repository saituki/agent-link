-- 0062_no_question.sql
-- Node の種類 Q を廃止する（種類は A・F・E・S・D の 5 つ）。
-- 版 62 の前に `database.rs` の `fold_old_questions` が、生きた旧形式の Q（親のある Q）を畳む。
-- この SQL は、残った Q の行をすべて A にする。親の無い Q（軸）は「親の無い A」（軸の A）になる。
-- 軸の文は guide_key が 6 つの key のときだけ、K1 の文に書き換える（それ以外は文を変えない）。
-- rt_log・rt_searches・rt_seed_questions ほかが rt_nodes(id) を指すため、`database.rs` の `migrate_upto` が
-- version 62 の前だけ外部キーの検査を一時的に外す（0053・0056・0057 と同じ）。

CREATE TABLE rt_nodes_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('A','F','E','S','D')),
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
    dataset_ref TEXT,
    frozen_qs   TEXT,
    rel         TEXT CHECK (rel IS NULL OR rel IN ('subtype','condition')),
    CHECK ((kind = 'F') = (stance IS NOT NULL)),
    CHECK (kind = 'A' OR guide_key IS NULL),
    CHECK (kind IN ('A','S','D') OR parent_id IS NOT NULL),
    CHECK (kind <> 'D' OR parent_id IS NULL),
    CHECK (kind = 'S' OR paths = '[]'),
    CHECK (kind = 'E' OR mlflow_ref IS NULL)
);
INSERT INTO rt_nodes_new (id, kind, parent_id, guide_key, text, body, stance, starred, cites,
    paper_id, run_id, report_id, mlflow_ref, ord, removed_at, paths, created_by, created_at, updated_at,
    dataset_ref, frozen_qs, rel)
SELECT id,
    CASE WHEN kind = 'Q' THEN 'A' ELSE kind END,
    parent_id, guide_key,
    CASE WHEN kind = 'Q' AND parent_id IS NULL THEN
        CASE guide_key
            WHEN 'task' THEN '解く問題'
            WHEN 'target' THEN '対象'
            WHEN 'data' THEN 'データ'
            WHEN 'method' THEN '手法'
            WHEN 'output' THEN '出力'
            WHEN 'metric' THEN '評価指標'
            ELSE text
        END
    ELSE text END,
    body, stance, starred, cites,
    paper_id, run_id, report_id, mlflow_ref, ord, removed_at, paths, created_by, created_at, updated_at,
    dataset_ref, frozen_qs, rel
FROM rt_nodes;
DROP TABLE rt_nodes;
ALTER TABLE rt_nodes_new RENAME TO rt_nodes;
CREATE INDEX idx_rt_nodes_parent ON rt_nodes(parent_id);
CREATE INDEX idx_rt_nodes_run ON rt_nodes(run_id);
