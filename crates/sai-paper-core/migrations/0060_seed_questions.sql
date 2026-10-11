-- 0060_seed_questions.sql
-- seed（D）の問いと、その回答（D か S が持つ。テキストか path の A）。S の凍結（完成した時点の問いの一覧）。
CREATE TABLE IF NOT EXISTS rt_seed_questions (
    id          INTEGER PRIMARY KEY,
    seed_id     INTEGER NOT NULL REFERENCES rt_nodes(id),
    text        TEXT NOT NULL,
    axis        TEXT CHECK (axis IS NULL OR axis IN ('task','target','data','method','output','metric')),
    ord         INTEGER NOT NULL DEFAULT 0,
    removed_at  TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_rt_seed_questions_seed ON rt_seed_questions(seed_id);
CREATE TABLE IF NOT EXISTS rt_answers (
    id          INTEGER PRIMARY KEY,
    owner_id    INTEGER NOT NULL REFERENCES rt_nodes(id),          -- D（seed の回答＝固定）か S（sprout の回答＝新規）
    question_id INTEGER NOT NULL REFERENCES rt_seed_questions(id),
    a_id        INTEGER REFERENCES rt_nodes(id),                   -- path の回答（A）
    text        TEXT,                                              -- テキストの回答
    removed_at  TEXT,
    created_by  TEXT NOT NULL DEFAULT 'user',
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((a_id IS NULL) <> (text IS NULL))
);
CREATE INDEX IF NOT EXISTS idx_rt_answers_owner ON rt_answers(owner_id);
-- S の凍結: 完成した時点の問いの ID の JSON 配列。NULL は未完成。
ALTER TABLE rt_nodes ADD COLUMN frozen_qs TEXT;
