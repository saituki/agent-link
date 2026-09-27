-- paper_extracts に papers_id (INTEGER) を追加
--
-- 背景: paper_extracts.paper_id は抽出キー(ArXiv:..., DOI:..., S2 ID)で、
-- papers.id との参照がなかった。チャンク化パイプライン(RAG インデックス)が
-- papers_id を必要とするため、既存データから逆引きして埋める。
--
-- 処理:
--   - paper_extracts.paper_id が "ArXiv:XXXX" 形式 → papers.arxiv_id で JOIN
--   - paper_extracts.paper_id が "DOI:XXXX" 形式 → papers.doi で JOIN
--   - その他(S2 ID など) → NULL のまま

ALTER TABLE paper_extracts ADD COLUMN papers_id INTEGER REFERENCES papers(id);

-- ArXiv フォーマット: "ArXiv:2401.12345" → papers.arxiv_id = "2401.12345"
UPDATE paper_extracts
SET papers_id = (
  SELECT p.id
  FROM papers p
  WHERE p.arxiv_id = substr(paper_extracts.paper_id, 7)
  LIMIT 1
)
WHERE paper_extracts.paper_id LIKE 'ArXiv:%';

-- DOI フォーマット: "DOI:10.1234/..." → papers.doi = "10.1234/..."
UPDATE paper_extracts
SET papers_id = (
  SELECT p.id
  FROM papers p
  WHERE p.doi = substr(paper_extracts.paper_id, 5)
  LIMIT 1
)
WHERE paper_extracts.paper_id LIKE 'DOI:%'
  AND papers_id IS NULL;

-- インデックス
CREATE INDEX IF NOT EXISTS idx_paper_extracts_papers_id
    ON paper_extracts(papers_id);
