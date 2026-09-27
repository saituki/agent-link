-- 段階 5: 重い処理（抽出・図・翻訳・Radio 等）の記録。
-- TUI / CLI / phone が同じ表に書き、二重起動の判定と TASKS 画面の表示に使う。
CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,              -- extract / figures / translate_pdf / translate_abstract / radio / enrich
    paper_id    INTEGER REFERENCES papers(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'running', -- queued / running / done / failed
    step        TEXT,
    error       TEXT,
    origin      TEXT NOT NULL,              -- tui / cli / phone
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_jobs_active ON jobs(status, kind, paper_id);
