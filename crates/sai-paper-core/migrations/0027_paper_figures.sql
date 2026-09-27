-- 図表（Figure / Table）抽出の保存先。
--
-- 背景: docling は `--image-export-mode referenced` で画像を PNG として書き出し、
-- Markdown / JSON から参照できる。本テーブルは論文ごとの図を 1 行 1 図で保持する。
--
-- paper_id は引用抽出と同じテキストキー（S2 40hex / ArXiv:... / DOI:...）。
-- papers_id は papers(id) への INTEGER 参照（NULL 許容。0025 と同じ考え方）。
-- 画像実体は `<paper_dir>/figures/fig-NNN.png` に置く（image_path は絶対パス）。

CREATE TABLE paper_figures (
  paper_id TEXT NOT NULL,
  papers_id INTEGER REFERENCES papers(id),
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
CREATE INDEX idx_paper_figures_papers_id ON paper_figures(papers_id);

-- 図抽出の実行状態（"未抽出" と "図が 0 件" を区別するために必要）。
-- status: parsing | ready | empty | failed
CREATE TABLE paper_figure_extracts (
  paper_id TEXT PRIMARY KEY,
  papers_id INTEGER REFERENCES papers(id),
  status TEXT NOT NULL,
  pdf_path TEXT,
  pdf_mtime TEXT,                   -- PDF のエポック秒（stale 判定用）
  n_figures INTEGER,
  error TEXT,
  extracted_at TEXT
);
CREATE INDEX idx_paper_figure_extracts_papers_id ON paper_figure_extracts(papers_id);
