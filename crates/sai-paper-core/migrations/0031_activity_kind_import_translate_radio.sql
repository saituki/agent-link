-- 0031_activity_kind_import_translate_radio.sql
-- activity_log の種別を増やす（Dashboard の出来事をカテゴリごとに色分けして出すため）
--
--   import    : PDF の取り込み（これまでは download と同じ扱いで記録されていなかった）
--   translate : 要旨・PDF の日本語訳
--   radio     : 解説音声（sai-radio）の作成
--
-- SQLite は CHECK 制約を変えられないため、作り直して中身を移す（0018 と同じ手順）。
CREATE TABLE activity_log_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (
                    kind IN ('download', 'discover', 'notes', 'import', 'translate', 'radio')
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
