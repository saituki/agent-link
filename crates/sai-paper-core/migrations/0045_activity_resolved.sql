-- 0045_activity_resolved.sql
-- activity_log の種別を増やす（S02「要対応」を `x` で対応済みにした記録）。
--
--   resolved : ユーザーが要対応の 1 件を「対応済み」にした。
--              `detail` の JSON は {"key": "<要対応の key>", "text": "<その時の本文の先頭 200 字>"}。
--
-- SQLite は CHECK 制約を変えられないため、作り直して中身を移す（0043 と同じ手順）。
--
-- 注意: 0043 で足した `open` / `agent_read` を必ず残すこと。CHECK の一覧から
-- 落とすと、それまでに記録した行が移せなくなる。
CREATE TABLE activity_log_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (
                    kind IN (
                        'download', 'discover', 'notes', 'import', 'translate', 'radio',
                        'open', 'agent_read', 'resolved'
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
