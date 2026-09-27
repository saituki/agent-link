-- 本文チャンクの paper_id を補正する。
--
-- 背景: paragraph/sentence/window 戦略で生成された本文チャンク (source_kind='body')
-- の paper_id が NULL になっていた。source_ref から対応する paper_id を逆引きして埋める。

-- paragraph 戦略: source_ref = 'extract_<hash>'
-- paper_extracts を paper_id + md_path で特定し、pdf_path 経由で papers と結合
UPDATE chunks
SET paper_id = (
  SELECT p.id
  FROM paper_extracts pe
  LEFT JOIN papers p ON p.pdf_path = pe.pdf_path
  WHERE pe.paper_id = SUBSTR(chunks.source_ref, 9)
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND source_ref LIKE 'extract_%'
  AND strategy = 'paragraph';

-- sentence 戦略: source_ref = '<S2_paperId>_<sent_no>'
-- paper_sentences.paper_id が papers の s2_paper_id と一致する場合
UPDATE chunks
SET paper_id = (
  SELECT p.id
  FROM paper_sentences ps
  LEFT JOIN papers p ON p.s2_paper_id = ps.paper_id
  WHERE ps.paper_id || '_' || ps.sent_no = chunks.source_ref
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND strategy = 'sentence';

-- window 戦略: source_ref = 'window_<S2_paperId>_<ord>'
-- paper_sentences.paper_id が papers の s2_paper_id と一致する場合
UPDATE chunks
SET paper_id = (
  SELECT p.id
  FROM paper_sentences ps
  LEFT JOIN papers p ON p.s2_paper_id = ps.paper_id
  WHERE 'window_' || ps.paper_id || '_%' GLOB chunks.source_ref
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND strategy = 'window';
