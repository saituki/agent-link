-- 段階 7: 索引（分け方・大きさ・埋め込みモデルの組）に名前を付けて並べて持つ。
-- 今ある断片はすべて default 索引のもの。
ALTER TABLE chunks ADD COLUMN index_name TEXT NOT NULL DEFAULT 'default';
CREATE INDEX IF NOT EXISTS idx_chunks_index_name ON chunks(index_name);
