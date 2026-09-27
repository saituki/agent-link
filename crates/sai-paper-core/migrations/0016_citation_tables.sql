-- sai-paper 引用文献パイプライン用テーブル（v2 統合計画）
-- papr.db に論文メタデータ（既存 papers 等）と引用文献情報を統合する。
-- 論文 ↔ S2 ID の対応付けは papers.s2_paper_id で行う。

-- 論文に S2 paperId 列を追加（DL完了時に sai-papr が書き込む）
ALTER TABLE papers ADD COLUMN s2_paper_id TEXT;
CREATE UNIQUE INDEX idx_papers_s2_paper_id
    ON papers(s2_paper_id) WHERE s2_paper_id IS NOT NULL;

-- 引用文献リスト（docling 抽出 + S2 マッチング結果）
CREATE TABLE IF NOT EXISTS paper_refs (
    paper_id        TEXT NOT NULL,           -- 抽出キー（S2 40hex または ArXiv:/DOI: プレフィックス付き）
    ref_no          INTEGER NOT NULL,        -- 引用番号（論文内での順序）
    first_author    TEXT,                    -- 第一著者
    year            INTEGER,                 -- 出版年
    year_suffix     TEXT,                    -- 年号サフィックス（例: "a", "b"）
    journal         TEXT,                    -- ジャーナル名
    title           TEXT,                    -- 引用文献タイトル
    doi             TEXT,                    -- DOI
    raw_text        TEXT NOT NULL,           -- docling 抽出元テキスト
    -- S2 マッチング結果（マッチ成功時のみ設定）
    s2_paper_id     TEXT,                    -- S2 40hex（マッチした引用文献の S2 ID）
    s2_title        TEXT,
    s2_authors      TEXT,                    -- JSON配列
    s2_year         INTEGER,
    s2_venue        TEXT,
    s2_journal      TEXT,                    -- JSON
    s2_external_ids TEXT,                    -- JSON
    s2_citation_count INTEGER,
    s2_url          TEXT,
    PRIMARY KEY (paper_id, ref_no)
);

-- 本文中の引用文
CREATE TABLE IF NOT EXISTS paper_sentences (
    paper_id    TEXT NOT NULL,
    sent_no     INTEGER NOT NULL,
    text        TEXT NOT NULL,
    cite_refs   TEXT NOT NULL,               -- 言及されている ref_no の JSON配列（例: "[1, 5, 12]"）
    PRIMARY KEY (paper_id, sent_no)
);

-- 抽出状態管理（stale 判定・進捗表示用）
CREATE TABLE IF NOT EXISTS paper_extracts (
    paper_id     TEXT PRIMARY KEY,           -- 抽出キー
    status       TEXT NOT NULL DEFAULT 'none', -- none/parsing/ready/failed
    style        TEXT,                        -- ジャーナルスタイル（現在は Generic 固定）
    pdf_path     TEXT,
    pdf_mtime    TEXT,                        -- ISO8601
    n_refs       INTEGER,
    n_sentences  INTEGER,
    md_path      TEXT,
    error        TEXT,                        -- 失敗時のエラーメッセージ
    extracted_at TEXT                         -- ISO8601
);

CREATE INDEX IF NOT EXISTS idx_paper_refs_paper_id      ON paper_refs(paper_id);
CREATE INDEX IF NOT EXISTS idx_paper_refs_s2_paper_id   ON paper_refs(s2_paper_id);
CREATE INDEX IF NOT EXISTS idx_paper_sentences_paper_id ON paper_sentences(paper_id);
