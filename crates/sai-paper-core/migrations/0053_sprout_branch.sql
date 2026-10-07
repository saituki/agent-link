-- 0053_sprout_branch.sql
-- SPROUT を rt_nodes の kind 'S' にする（旧 rt_seeds をやめる）。E（BRANCH）の親は S か E にする。
--
-- 外部キー（rt_logic・rt_log・rt_searches・search_sets が rt_nodes(id) を指す）を保ったまま
-- rt_nodes を作り直すため、`database.rs` の `migrate()` が version 53 の前だけ
-- `PRAGMA foreign_keys = OFF` にしている（トランザクションの中では効かないため）。
--
-- 移し方:
--   1. rt_nodes を paths 列付きの新しい形で作り直し、行をそのまま写す。
--   2. 旧 rt_seeds の行を S の Node にする。id は「移す前の rt_nodes の最大 id + seed の id」。
--   3. 親が O の E を、その O を paths に含む S の下へ移す。
--      含む S が無い O には、S を 1 つ作る（既存の E を捨てないため）。
--   4. rt_proposals の add_seed を add_sprout にし、S の参照を新しい id に直す。
--   5. rt_seeds を消す。

CREATE TABLE rt_nodes_new (
    id             INTEGER PRIMARY KEY,
    kind           TEXT NOT NULL CHECK (kind IN ('Q','O','F','E','S')),
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
    proposal_id    INTEGER,
    paths          TEXT NOT NULL DEFAULT '[]',
    created_by     TEXT NOT NULL DEFAULT 'user',
    created_at     TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (kind <> 'O' OR answer_kind IS NOT NULL),
    CHECK (kind = 'O' OR (answer_kind IS NULL AND starred = 0)),
    CHECK (kind = 'Q' OR (target IS NULL AND guide_key IS NULL)),
    CHECK (kind IN ('Q','S') OR parent_id IS NOT NULL),
    CHECK (kind <> 'S' OR parent_id IS NULL),
    CHECK (kind = 'S' OR paths = '[]')
);

-- 1. 行を写す（S 以外は paths を空にする）。
INSERT INTO rt_nodes_new (id, kind, parent_id, guide_key, text, body, answer_kind, starred,
    target, cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, proposal_id,
    paths, created_by, created_at, updated_at)
SELECT id, kind, parent_id, guide_key, text, body, answer_kind, starred,
    target, cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, proposal_id,
    '[]', created_by, created_at, updated_at
FROM rt_nodes;

-- 移す前の最大 id と、いちばん大きい seed の id。
CREATE TEMP TABLE rt53_base AS
SELECT (SELECT COALESCE(MAX(id), 0) FROM rt_nodes) AS node_max,
       (SELECT COALESCE(MAX(id), 0) FROM rt_seeds) AS seed_max;

-- 2. 旧 rt_seeds を S にする。
INSERT INTO rt_nodes_new (id, kind, parent_id, guide_key, text, body, answer_kind, starred,
    target, cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, proposal_id,
    paths, created_by, created_at, updated_at)
SELECT (SELECT node_max FROM rt53_base) + s.id, 'S', NULL, NULL, s.title, s.sentence, NULL, 0,
    NULL, '[]', NULL, s.run_id, NULL, NULL, 0, s.removed_at, NULL,
    CASE WHEN json_valid(s.path) THEN s.path ELSE '[]' END,
    s.created_by, s.created_at, s.created_at
FROM rt_seeds s;

-- O の id → S の id。S が複数あるときは id の小さい S を残す。
CREATE TEMP TABLE rt53_map (o_id INTEGER PRIMARY KEY, s_id INTEGER NOT NULL);
INSERT OR IGNORE INTO rt53_map (o_id, s_id)
SELECT CAST(j.value AS INTEGER), (SELECT node_max FROM rt53_base) + s.id
FROM rt_seeds s, json_each(CASE WHEN json_valid(s.path) THEN s.path ELSE '[]' END) j
ORDER BY s.id;

-- 親が O の E があるのに、どの S の paths にも入っていない O。
CREATE TEMP TABLE rt53_orphan AS
SELECT DISTINCT e.parent_id AS o_id
FROM rt_nodes e
JOIN rt_nodes p ON p.id = e.parent_id
WHERE e.kind = 'E' AND p.kind = 'O'
  AND e.parent_id NOT IN (SELECT o_id FROM rt53_map);

-- 3. その O に S を 1 つ作る。id は seed の次の番号から。
CREATE TEMP TABLE rt53_new AS
SELECT o_id, (SELECT node_max + seed_max FROM rt53_base) + ROW_NUMBER() OVER (ORDER BY o_id) AS s_id
FROM rt53_orphan;

INSERT INTO rt_nodes_new (id, kind, parent_id, guide_key, text, body, answer_kind, starred,
    target, cites, paper_id, run_id, report_id, mlflow_ref, ord, removed_at, proposal_id,
    paths, created_by, created_at, updated_at)
SELECT n.s_id, 'S', NULL, NULL, substr(o.text, 1, 120), '', NULL, 0,
    NULL, '[]', NULL, NULL, NULL, NULL, 0, NULL, NULL,
    json_array(n.o_id), 'migration', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
FROM rt53_new n JOIN rt_nodes o ON o.id = n.o_id;

INSERT OR IGNORE INTO rt53_map (o_id, s_id) SELECT o_id, s_id FROM rt53_new;

-- 親が O の E を、その O を含む S の下へ移す。
UPDATE rt_nodes_new
SET parent_id = (SELECT m.s_id FROM rt53_map m WHERE m.o_id = rt_nodes_new.parent_id)
WHERE kind = 'E' AND parent_id IN (SELECT o_id FROM rt53_map);

DROP TABLE rt_nodes;
ALTER TABLE rt_nodes_new RENAME TO rt_nodes;

CREATE INDEX idx_rt_nodes_parent ON rt_nodes(parent_id);
CREATE INDEX idx_rt_nodes_run ON rt_nodes(run_id);

-- 4. 提案の参照を直す。
UPDATE rt_proposals SET op = 'add_sprout' WHERE op = 'add_seed';
UPDATE rt_proposals
SET result_ref = 'S' || ((SELECT node_max FROM rt53_base) + CAST(substr(result_ref, 2) AS INTEGER))
WHERE op = 'add_sprout' AND result_ref GLOB 'S[0-9]*';

-- 5. 旧 rt_seeds を消す。
DROP TABLE rt_seeds;
