-- 0043_activity_open.sql
-- activity_log の種別を増やす（N01「開いた」「エージェントが読んだ」の記録）。
--
--   open       : ユーザーが開いた（TUI・sai-link・sai-phone）
--   agent_read : エージェントが読んだ（MCP の paper / CLI の content）
--
-- SQLite は CHECK 制約を変えられないため、作り直して中身を移す（0031 と同じ手順）。
--
-- 注意: 後続のテーマ（小さな改良）が `resolved` を足すときも、ここで足した
-- `open` と `agent_read` を必ず残すこと。CHECK の一覧から落とすと、それまでに
-- 記録した行が移せなくなる。
CREATE TABLE activity_log_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (
                    kind IN (
                        'download', 'discover', 'notes', 'import', 'translate', 'radio',
                        'open', 'agent_read'
                    )
                ),
    paper_id    INTEGER REFERENCES papers(id) ON DELETE CASCADE,
    detail      TEXT,
    occurred_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO activity_log_new (id, kind, paper_id, detail, occurred_at)
SELECT id, kind, paper_id, detail, occurred_at FROM activity_log;

DROP TABLE activity_log;

ALTER TABLE activity_log_new RENAME TO activity_log;

CREATE INDEX idx_activity_occurred ON activity_log(occurred_at DESC);
CREATE INDEX idx_activity_paper ON activity_log(paper_id, occurred_at DESC);
