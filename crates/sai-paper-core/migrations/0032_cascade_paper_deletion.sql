-- 0032_cascade_paper_deletion.sql
-- 索引済み・抽出済みの論文を Library から削除できるようにする。
--
-- 背景:
--   chunks.paper_id / paper_extracts.papers_id / paper_figures.papers_id が
--   ON DELETE CASCADE でなかったため、索引（チャンク・埋め込み）や抽出結果が
--   ある論文を DELETE すると「FOREIGN KEY constraint failed」で失敗していた。
--   SQLite は外部キーの定義を変更できないため、表を作り直して中身を移す
--   （0031 の activity_log と同じ手順）。既存データはすべて保持する。
--
-- 注意（順序が重要）:
--   chunk_embeddings は chunks を参照している。chunks を先に差し替えて
--   DROP すると、そのときの CASCADE で埋め込みが消えてしまう。そのため
--   「chunks を作り直す → 中身を移す → chunk_embeddings を新しい chunks
--   を参照する形へ作り直す → 古い表を DROP する」の順で処理する。

-- 1. chunks を CASCADE 付きの新しい定義へ作り直す。
--    （この時点では chunk_embeddings が旧 chunks を参照したままなので
--      まだ DROP しない）
ALTER TABLE chunks RENAME TO chunks_old;

CREATE TABLE chunks (
  id INTEGER PRIMARY KEY,
  source_kind TEXT NOT NULL,     -- abstract|body|note|document
  source_ref TEXT NOT NULL,      -- abstract|sentence_id|paragraph_id|note_id|document_id
  paper_id INTEGER REFERENCES papers(id) ON DELETE CASCADE,
  ord INTEGER NOT NULL,          -- チャンク内の順序
  text TEXT NOT NULL,
  char_start INTEGER,            -- ソーステキストの開始位置
  char_end INTEGER,              -- ソーステキストの終了位置
  strategy TEXT NOT NULL,        -- abstract|sentence|paragraph|window
  created_at TEXT NOT NULL
);

INSERT INTO chunks (id, source_kind, source_ref, paper_id, ord, text,
                    char_start, char_end, strategy, created_at)
SELECT id, source_kind, source_ref, paper_id, ord, text,
       char_start, char_end, strategy, created_at
FROM chunks_old;

-- 2. chunk_embeddings を「新しい chunks を参照する」定義へ作り直す。
--    （ renaming により旧 chunks を参照する形に変わったため差し替える）
ALTER TABLE chunk_embeddings RENAME TO chunk_embeddings_old;

CREATE TABLE chunk_embeddings (
  chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
  model_id INTEGER NOT NULL REFERENCES embedding_models(id) ON DELETE CASCADE,
  vec BLOB NOT NULL,             -- f32 little-endian の連結
  PRIMARY KEY (chunk_id, model_id)
);

INSERT INTO chunk_embeddings (chunk_id, model_id, vec)
SELECT chunk_id, model_id, vec FROM chunk_embeddings_old;

-- 3. 旧表を破棄する。ここで旧 chunks の FTS トリガーと索引も一緒に消える。
DROP TABLE chunk_embeddings_old;
DROP TABLE chunks_old;

-- 4. 索引と FTS トリガーを新しい chunks に対して作り直す。
CREATE INDEX idx_chunks_src ON chunks(source_kind, source_ref);
CREATE INDEX idx_chunks_paper ON chunks(paper_id);

-- chunk_fts への同期トリガー（migration 0023 と同じ形）
CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunk_fts(rowid, text)
  VALUES (new.id, new.text);
END;

CREATE TRIGGER chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunk_fts(chunk_fts, rowid, text)
  VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER chunks_fts_update AFTER UPDATE OF text ON chunks BEGIN
  INSERT INTO chunk_fts(chunk_fts, rowid, text)
  VALUES ('delete', old.id, old.text);
  INSERT INTO chunk_fts(rowid, text)
  VALUES (new.id, new.text);
END;

-- 表を作り直したので、外部コンテンツ形式の chunk_fts を中身から再構築する。
INSERT INTO chunk_fts(chunk_fts) VALUES('rebuild');

-- 5. paper_extracts を CASCADE 付きで作り直す（0025 で追加した papers_id を置き換える）。
ALTER TABLE paper_extracts RENAME TO paper_extracts_old;

CREATE TABLE paper_extracts (
    paper_id     TEXT PRIMARY KEY,            -- 抽出キー
    status       TEXT NOT NULL DEFAULT 'none', -- none/parsing/ready/failed
    style        TEXT,                        -- ジャーナルスタイル（現在は Generic 固定）
    pdf_path     TEXT,
    pdf_mtime    TEXT,                        -- ISO8601
    n_refs       INTEGER,
    n_sentences  INTEGER,
    md_path      TEXT,
    error        TEXT,                        -- 失敗時のエラーメッセージ
    extracted_at TEXT,                        -- ISO8601
    papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE
);

INSERT INTO paper_extracts (paper_id, status, style, pdf_path, pdf_mtime,
                            n_refs, n_sentences, md_path, error, extracted_at, papers_id)
SELECT paper_id, status, style, pdf_path, pdf_mtime,
       n_refs, n_sentences, md_path, error, extracted_at, papers_id
FROM paper_extracts_old;

DROP TABLE paper_extracts_old;

CREATE INDEX IF NOT EXISTS idx_paper_extracts_papers_id
    ON paper_extracts(papers_id);

-- 6. paper_figures を CASCADE 付きで作り直す。
ALTER TABLE paper_figures RENAME TO paper_figures_old;

CREATE TABLE paper_figures (
  paper_id TEXT NOT NULL,
  papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE,
  fig_no INTEGER NOT NULL,          -- 1 起算の図番号
  label TEXT,                       -- "Figure 1" / "Table 2" 等
  title TEXT,                       -- 図のタイトル（docling から取れない場合は NULL）
  caption TEXT,                     -- キャプション本文（ラベルを除いた残り）
  image_path TEXT NOT NULL,         -- 保存した PNG の絶対パス
  mention TEXT,                     -- 本文中の言及文（見つからない場合は NULL）
  page INTEGER,                     -- 掲載ページ（取得できる場合）
  created_at TEXT NOT NULL,
  PRIMARY KEY (paper_id, fig_no)
);

INSERT INTO paper_figures (paper_id, papers_id, fig_no, label, title, caption,
                           image_path, mention, page, created_at)
SELECT paper_id, papers_id, fig_no, label, title, caption,
       image_path, mention, page, created_at
FROM paper_figures_old;

DROP TABLE paper_figures_old;

CREATE INDEX idx_paper_figures_papers_id ON paper_figures(papers_id);
