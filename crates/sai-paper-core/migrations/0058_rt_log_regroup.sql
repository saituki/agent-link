-- 0058_rt_log_regroup.sql
-- rt_log の action に move_node（A を部分木ごと別の Q へ移す）と regroup（中間の A と細分の Q を作り、A をまとめて移す）を足す。
-- SQLite は CHECK を変えられないので、表を作り直して写す。行の id は保つ。追記だけのトリガは作り直す。
-- rt_log を指す表は無いので、外部キーの検査を外さなくてよい（database.rs の migrate_upto は変えない）。

CREATE TABLE rt_log_new (
    id          INTEGER PRIMARY KEY,
    node_id     INTEGER NOT NULL REFERENCES rt_nodes(id),
    action      TEXT NOT NULL CHECK (action IN ('add_q','insert_q','move_a','move_node','regroup','copy','remove',
                    'star','unstar','edit','set_stance','set_mlflow')),
    detail      TEXT NOT NULL DEFAULT '',
    actor       TEXT NOT NULL DEFAULT 'user',
    approved_by TEXT,
    qid         TEXT,
    question    TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT INTO rt_log_new (id, node_id, action, detail, actor, approved_by, qid, question, created_at)
SELECT id, node_id, action, detail, actor, approved_by, qid, question, created_at FROM rt_log;
DROP TABLE rt_log;
ALTER TABLE rt_log_new RENAME TO rt_log;
CREATE TRIGGER rt_log_no_update BEFORE UPDATE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
CREATE TRIGGER rt_log_no_delete BEFORE DELETE ON rt_log
BEGIN SELECT RAISE(ABORT, 'rt_log は追記だけです'); END;
