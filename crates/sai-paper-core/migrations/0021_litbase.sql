-- 0021_litbase.sql
-- litbase 統合: 研究プロファイル / 検索語 / 論文ノート / 研究ドキュメント / 引用索引
--
-- 設計方針:
--   - 論文ノート（1論文=1件）は既存 notes テーブルと対で使う（notes.body が Markdown 本文）。
--   - 研究ドキュメント（recommendation/recap/rfd/outline/handoff）は論文に紐づかない。
--   - 研究プロジェクトごとに 1 つの papr.db（.sai/paper/papr.db）に格納する。

-- 1. 研究プロファイル（1行のみ）
CREATE TABLE IF NOT EXISTS litbase_profile (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    topic       TEXT NOT NULL DEFAULT '',
    framework   TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT '',
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 2. 階層検索語（Tier 1..3）
CREATE TABLE IF NOT EXISTS litbase_search_tiers (
    tier        INTEGER PRIMARY KEY CHECK (tier BETWEEN 1 AND 3),
    label       TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    terms_json  TEXT NOT NULL DEFAULT '[]',
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 3. 検索語の根拠（based_on_notes 相当）
CREATE TABLE IF NOT EXISTS litbase_search_basis (
    paper_id    INTEGER PRIMARY KEY REFERENCES papers(id) ON DELETE CASCADE,
    added_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 4. 論文ノートの構造化データ（notes.body と同一トランザクションで更新）
CREATE TABLE IF NOT EXISTS litbase_notes (
    paper_id      INTEGER PRIMARY KEY REFERENCES papers(id) ON DELETE CASCADE,
    sections_json TEXT NOT NULL DEFAULT '{}',
    discussion_md TEXT NOT NULL DEFAULT '',
    updated_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 5. 論文に紐づく議論ログ
CREATE TABLE IF NOT EXISTS litbase_discussions (
    id          INTEGER PRIMARY KEY,
    paper_id    INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    asked_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    question    TEXT NOT NULL DEFAULT '',
    answer      TEXT NOT NULL DEFAULT '',
    implication TEXT
);

CREATE INDEX IF NOT EXISTS idx_litbase_discussions_paper
    ON litbase_discussions(paper_id, asked_at);

-- 6. 読書リスト（トラッカー）
CREATE TABLE IF NOT EXISTS litbase_reading_list (
    paper_id    INTEGER PRIMARY KEY REFERENCES papers(id) ON DELETE CASCADE,
    status      TEXT NOT NULL DEFAULT 'unread'
                CHECK (status IN ('unread', 'reading', 'read')),
    category    TEXT NOT NULL DEFAULT '',
    added_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 7. 論文に紐づかない研究ドキュメント
CREATE TABLE IF NOT EXISTS litbase_documents (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL
                CHECK (kind IN ('recommendation', 'recap', 'rfd', 'outline', 'handoff')),
    title       TEXT NOT NULL DEFAULT '',
    body_md     TEXT NOT NULL DEFAULT '',
    meta_json   TEXT NOT NULL DEFAULT '{}',
    version     INTEGER NOT NULL DEFAULT 1,
    status      TEXT NOT NULL DEFAULT 'draft',
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_litbase_documents_kind
    ON litbase_documents(kind, created_at DESC);

-- 8. RFD 引用索引（文献完全性監査の要）
CREATE TABLE IF NOT EXISTS litbase_rfd_citations (
    document_id INTEGER NOT NULL REFERENCES litbase_documents(id) ON DELETE CASCADE,
    paper_id    INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    section     TEXT NOT NULL DEFAULT '',
    role        TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (document_id, paper_id, section)
);

-- 9. litbase ジョブ状態（将来の非同期実行用）
CREATE TABLE IF NOT EXISTS litbase_jobs (
    id           INTEGER PRIMARY KEY,
    kind         TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'queued',
    payload_json TEXT NOT NULL DEFAULT '{}',
    error        TEXT,
    created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at  TEXT
);

-- 10. プロファイル・Tier の初期行をシード
INSERT OR IGNORE INTO litbase_profile (id) VALUES (1);
INSERT OR IGNORE INTO litbase_search_tiers (tier, label, description) VALUES
    (1, 'Tier 1 · Core', '研究課題に直接対応する'),
    (2, 'Tier 2 · Adjacent', '同一機序・手法を別の疾患文脈で扱う'),
    (3, 'Tier 3 · Wild Card', '学際的な着想・手法重視');
