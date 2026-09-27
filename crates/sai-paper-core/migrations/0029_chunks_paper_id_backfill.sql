-- 本文チャンクの paper_id を補正する（chunking.rs の結合キー修正に伴うデータ移行）。
--
-- 背景: 0024 と同じ問題が残っていた。chunk_paragraph / chunk_sentence /
-- chunk_window が paper_extracts.paper_id / paper_sentences.paper_id（いずれも
-- 抽出キーの TEXT）を papers.id（INTEGER）と直接結合していたため、
-- 生成されたチャンクの paper_id が NULL になっていた。
--
-- 本マイグレーションは既存チャンクの paper_id をキー列の照合で埋め直す。
-- 新規生成分は chunking.rs 側のクエリ修正で正しく入る。

-- paragraph 戦略: source_ref = 'extract_<抽出キー>'
UPDATE chunks
SET paper_id = (
  SELECT p.id FROM papers p
  WHERE ('extract_' || p.s2_paper_id) = chunks.source_ref COLLATE NOCASE
     OR ('extract_ArXiv:' || p.arxiv_id) = chunks.source_ref COLLATE NOCASE
     OR ('extract_DOI:' || p.doi) = chunks.source_ref COLLATE NOCASE
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND strategy = 'paragraph'
  AND source_ref LIKE 'extract_%';

-- sentence 戦略: source_ref = '<抽出キー>_<sent_no>'
UPDATE chunks
SET paper_id = (
  SELECT p.id FROM papers p
  WHERE chunks.source_ref LIKE (p.s2_paper_id || '\_%') ESCAPE '\' COLLATE NOCASE
     OR chunks.source_ref LIKE ('ArXiv:' || p.arxiv_id || '\_%') ESCAPE '\' COLLATE NOCASE
     OR chunks.source_ref LIKE ('DOI:' || p.doi || '\_%') ESCAPE '\' COLLATE NOCASE
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND strategy = 'sentence';

-- window 戦略: source_ref = 'window_<抽出キー>_<ord>'
UPDATE chunks
SET paper_id = (
  SELECT p.id FROM papers p
  WHERE chunks.source_ref LIKE ('window_' || p.s2_paper_id || '\_%') ESCAPE '\' COLLATE NOCASE
     OR chunks.source_ref LIKE ('window_ArXiv:' || p.arxiv_id || '\_%') ESCAPE '\' COLLATE NOCASE
     OR chunks.source_ref LIKE ('window_DOI:' || p.doi || '\_%') ESCAPE '\' COLLATE NOCASE
  LIMIT 1
)
WHERE source_kind = 'body'
  AND paper_id IS NULL
  AND strategy = 'window';
