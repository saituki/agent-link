-- 0018_tags_library_refactor.sql
-- Library 再編: タグ管理への移行 / Reading Queue・Bookmarks 廃止 / activity_log 種別整理
--
-- 変更内容:
--   1. collections -> 同名タグ変換（collection_papers 所属も引き継ぐ）
--   2. reading_queue -> 'reading-queue' タグ変換（queue_order を登録順として保持）
--   3. bookmarks テーブル廃止
--   4. reading_queue テーブル・トリガ廃止
--   5. collections / collection_papers テーブル廃止（タグへ完全移行）
--   6. activity_log の種別を download / discover / notes の 3 種に整理
--   7. app_meta（KV テーブル）新設（レガシーノート書き出し済みフラグ等に使用）

PRAGMA foreign_keys=OFF;

-- 1) collections -> 同名タグ
INSERT OR IGNORE INTO tags (name)
SELECT name FROM collections;

INSERT OR IGNORE INTO paper_tags (paper_id, tag_id)
SELECT cp.paper_id, t.id
FROM collection_papers cp
JOIN collections c ON c.id = cp.collection_id
JOIN tags t ON t.name = c.name COLLATE NOCASE;

-- 2) reading_queue -> 'reading-queue' タグ（queue_order 順に登録）
INSERT OR IGNORE INTO tags (name) VALUES ('reading-queue');

INSERT INTO paper_tags (paper_id, tag_id)
SELECT rq.paper_id, t.id
FROM reading_queue rq
JOIN tags t ON t.name = 'reading-queue'
ORDER BY rq.queue_order ASC;

-- 3) bookmarks 廃止
DROP TABLE bookmarks;

-- 4) reading_queue 廃止（トリガを先に削除しないと papers 更新が阻害される）
DROP TRIGGER IF EXISTS remove_from_queue_on_status_change;
DROP TABLE reading_queue;

-- 5) collections 廃止
DROP TABLE collection_papers;
DROP TABLE collections;

-- 6) activity_log 種別整理
--    downloaded -> download / search -> discover / note_opened -> notes
--    旧種別（paper_opened, pdf_opened, bookmarked, tagged, collected,
--    paper_browsed, project_*）は仕様により移行対象外（破棄）
CREATE TABLE activity_log_new (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (
                    kind IN ('download', 'discover', 'notes')
                ),
    paper_id    INTEGER REFERENCES papers(id) ON DELETE CASCADE,
    detail      TEXT,
    occurred_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO activity_log_new (id, kind, paper_id, detail, occurred_at)
SELECT id,
       CASE kind
           WHEN 'downloaded' THEN 'download'
           WHEN 'search'     THEN 'discover'
           WHEN 'note_opened' THEN 'notes'
       END,
       paper_id,
       detail,
       occurred_at
FROM activity_log
WHERE kind IN ('downloaded', 'search', 'note_opened');

DROP TABLE activity_log;

ALTER TABLE activity_log_new RENAME TO activity_log;

CREATE INDEX idx_activity_occurred ON activity_log(occurred_at DESC);
CREATE INDEX idx_activity_paper ON activity_log(paper_id, occurred_at DESC);

-- 7) app_meta KV テーブル
CREATE TABLE IF NOT EXISTS app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

PRAGMA foreign_keys=ON;
