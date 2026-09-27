-- 外部の引用（Library に無い論文）の照合の結果を残す表。
--
-- `sai-paper report check` を実行したときだけ公開の API（Crossref・arXiv・PubMed）に
-- 問い合わせ、その結果をここに残す。短い間に何度も問い合わせて断られないよう、
-- `found` / `not_found` は 30 日の間は問い合わせ直さない（`offline` / `error` は次回また問う）。
--
-- - ref_id は引用の指し先（`DOI:10.1000/xyz` / `arXiv:2401.12345` / `PMID:12345678`）。
-- - status は `found`（見つかった）/ `not_found`（見つからない）/ `offline`（ネットに
--   つながらない）/ `error`（応答は来たが解析できないなど）。
-- - title / year / first_author は `found` のときだけ入れる。
-- - checked_at は最後に問い合わせた時刻（ISO8601）。

CREATE TABLE IF NOT EXISTS citation_checks (
  ref_id TEXT PRIMARY KEY,          -- 'DOI:10.…' / 'arXiv:2401.12345' / 'PMID:123'
  status TEXT NOT NULL CHECK (status IN ('found', 'not_found', 'offline', 'error')),
  title TEXT,
  year INTEGER,
  first_author TEXT,
  checked_at TEXT NOT NULL
);
