-- ポッドキャスト生成状態管理（paper_translations と同パターン）
CREATE TABLE IF NOT EXISTS paper_podcasts (
    paper_id INTEGER NOT NULL UNIQUE,
    status TEXT NOT NULL,       -- 'generating' | 'ready' | 'failed'
    generated_at TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_paper_podcasts_status ON paper_podcasts(status);
