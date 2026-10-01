-- 論文の節と塊（U01）。extract.md の中身の順番から ID を決める（作り直しても同じ extract.md なら同じ ID）。
CREATE TABLE paper_sections (
  paper_id    INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  sec_no      INTEGER NOT NULL,   -- 1 始まり。extract.md の見出しの順。最初の見出しより前の本文は 0
  heading     TEXT NOT NULL,      -- 見出しの文字（# と ** を外したもの）。節 0 は ''
  label       TEXT,               -- 見出しの先頭の番号（'4.1'）。無ければ NULL
  level       INTEGER NOT NULL,   -- 番号があれば「点の数 + 1」、無ければ 1。節 0 は 0
  kind        TEXT NOT NULL,      -- 'front' | 'abstract' | 'body' | 'references'
  n_passages  INTEGER NOT NULL,
  n_sentences INTEGER NOT NULL,
  n_chars     INTEGER NOT NULL,
  lead        TEXT NOT NULL,      -- 節の最初の 2 文（300 字まで）
  summary     TEXT,               -- LLM の要約（作っていなければ NULL）
  PRIMARY KEY (paper_id, sec_no)
);
CREATE TABLE paper_passages (
  paper_id    INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
  sec_no      INTEGER NOT NULL,
  pas_no      INTEGER NOT NULL,   -- 節の中で 1 始まり
  text        TEXT NOT NULL,
  n_sentences INTEGER NOT NULL,
  char_start  INTEGER NOT NULL,   -- 画像を除いた本文の上の位置（chunks.char_start と同じ座標）
  char_end    INTEGER NOT NULL,
  PRIMARY KEY (paper_id, sec_no, pas_no)
);
CREATE TABLE paper_section_builds (
  paper_id       INTEGER PRIMARY KEY REFERENCES papers(id) ON DELETE CASCADE,
  extract_path   TEXT NOT NULL,
  extract_sha256 TEXT NOT NULL,
  built_at       TEXT NOT NULL,   -- UTC の RFC 3339
  summary_model  TEXT             -- 要約に使った model（作っていなければ NULL）
);
