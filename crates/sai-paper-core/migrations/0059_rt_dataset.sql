-- 0059_rt_dataset.sql
-- E（BRANCH）に dataset の参照（sai-ts のスナップショット ts:<name>@<sha12> など）を持たせる。
-- 文字の形は engine では検査しない。E 以外に入れないことは ops で守る（mlflow_ref と同じ）。
-- rt_log の action に set_dataset を足す（CHECK は変えられないので、0056 と同じく表を作り直す）。
-- action の値は 0056 の値に、別の枝（0058）の move_node・regroup を足した和集合にする。
-- どちらの順で migration が当たっても、後から当たる側の値で壊れないようにするため。

ALTER TABLE rt_nodes ADD COLUMN dataset_ref TEXT;

CREATE TABLE rt_log_new (
    id          INTEGER PRIMARY KEY,
    node_id     INTEGER NOT NULL REFERENCES rt_nodes(id),
    action      TEXT NOT NULL CHECK (action IN ('add_q','insert_q','move_a','move_node','regroup',
                    'copy','remove','star','unstar','edit','set_stance','set_mlflow','set_dataset')),
    detail      TEXT NOT NULL DEFAULT '',
    actor       TEXT NOT NULL DEFAULT 'user',
    approved_by TEXT,
    qid         TEXT,
    question    TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT INTO rt_log_new (id, node_id, action, detail, actor, approved_by, qid, question, created_at)
SELECT id, node_id, action, detail, actor, approved_by, qid, question, created_at FROM rt_log;
DROP TABLE rt_log;                      -- 追記だけのトリガもいっしょに消える
ALTER TABLE rt_log_new RENAME TO rt_log;
CREATE TRIGGER rt_log_no_update BEFORE UPDATE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
CREATE TRIGGER rt_log_no_delete BEFORE DELETE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
