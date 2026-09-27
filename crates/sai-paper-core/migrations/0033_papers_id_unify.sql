-- 0033_papers_id_unify.sql
-- 引用・本文の引用文・抽出状態・図・翻訳の記録を、抽出キー（S2 40hex /
-- ArXiv:... / DOI:...）ではなく Library の番号（papers.id）で結び付ける
-- ための列をそろえる（段階 2c）。
--
-- 背景:
--   抽出キーは S2 の ID が後から見つかるなどすると変わり、そのたびに
--   過去の記録（引用・引用文・抽出状態・図・翻訳）がその論文から見えなくなる。
--   この後始末の作り直しが過去に 3 回入っている（0024 / 0025 / 0029）。
--   Library の論文については結び付けを papers.id（ON DELETE CASCADE）に統一し、
--   抽出キーは S2 に問い合わせるための列として残す。
--
-- やること:
--   1. papers_id が無い表に列と索引を足す（paper_refs / paper_sentences /
--      paper_translations）。
--   2. 列はあるが ON DELETE CASCADE でない表は、表を作り直して CASCADE にする
--      （paper_figure_extracts。SQLite は外部キーの定義を変更できないため、
--      0032 の paper_extracts / paper_figures と同じ手順。既存データは保持）。
--   3. 主キー（(paper_id, ref_no) 等、抽出キーを含むもの）は変えない。
--
-- 既存行への papers_id の埋め込みは SQL では正規化が書けないため、
-- Rust 側（Database::backfill_papers_id）が Database::open 時に 1 回行う。

-- 1. paper_refs に papers_id を追加。
ALTER TABLE paper_refs ADD COLUMN papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_paper_refs_papers_id ON paper_refs(papers_id);

-- 2. paper_sentences に papers_id を追加。
ALTER TABLE paper_sentences ADD COLUMN papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_paper_sentences_papers_id ON paper_sentences(papers_id);

-- 3. paper_translations に papers_id を追加。
ALTER TABLE paper_translations ADD COLUMN papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_paper_translations_papers_id ON paper_translations(papers_id);

-- 4. paper_figure_extracts を CASCADE 付きで作り直す（0027 の列を置き換える）。
ALTER TABLE paper_figure_extracts RENAME TO paper_figure_extracts_old;

CREATE TABLE paper_figure_extracts (
  paper_id TEXT PRIMARY KEY,                        -- 抽出キー
  papers_id INTEGER REFERENCES papers(id) ON DELETE CASCADE,
  status TEXT NOT NULL,                             -- parsing / ready / empty / failed
  pdf_path TEXT,
  pdf_mtime TEXT,                                   -- PDF のエポック秒（stale 判定用）
  n_figures INTEGER,
  error TEXT,
  extracted_at TEXT
);

INSERT INTO paper_figure_extracts (paper_id, papers_id, status, pdf_path, pdf_mtime,
                                   n_figures, error, extracted_at)
SELECT paper_id, papers_id, status, pdf_path, pdf_mtime, n_figures, error, extracted_at
FROM paper_figure_extracts_old;

DROP TABLE paper_figure_extracts_old;

CREATE INDEX IF NOT EXISTS idx_paper_figure_extracts_papers_id
    ON paper_figure_extracts(papers_id);
