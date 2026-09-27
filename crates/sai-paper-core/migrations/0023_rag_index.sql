-- RAG インデックス: チャンク化とベクトル検索の基盤

CREATE TABLE chunks (
  id INTEGER PRIMARY KEY,
  source_kind TEXT NOT NULL,     -- abstract|body|note|document
  source_ref TEXT NOT NULL,       -- abstract|sentence_id|paragraph_id|note_id|document_id
  paper_id INTEGER REFERENCES papers(id),
  ord INTEGER NOT NULL,           -- チャンク内の順序
  text TEXT NOT NULL,
  char_start INTEGER,             -- ソーステキストの開始位置
  char_end INTEGER,               -- ソーステキストの終了位置
  strategy TEXT NOT NULL,         -- abstract|sentence|paragraph|window
  created_at TEXT NOT NULL
);
CREATE INDEX idx_chunks_src ON chunks(source_kind, source_ref);
CREATE INDEX idx_chunks_paper ON chunks(paper_id);

-- FTS5 仮想テーブル（全文検索）
CREATE VIRTUAL TABLE chunk_fts USING fts5(
  text,
  content='chunks',
  content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);

-- chunk_fts への同期トリガ（migration 0002 の papers_search_* と同じ形）
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

-- 埋め込みモデルメタデータ
CREATE TABLE embedding_models (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  endpoint TEXT NOT NULL,
  dim INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

-- チャンク埋め込み（ベクトル）
CREATE TABLE chunk_embeddings (
  chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
  model_id INTEGER NOT NULL REFERENCES embedding_models(id) ON DELETE CASCADE,
  vec BLOB NOT NULL,             -- f32 little-endian の連結
  PRIMARY KEY (chunk_id, model_id)
);
