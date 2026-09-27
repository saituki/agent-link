-- report の自動登録（`sai-paper report scan`）で使う列と、検査の結果を残す表。
--
-- 背景: report の md は研究用のエージェントが sai-link の出力フォルダに保存する。
-- sai-paper は成果物のフォルダを読んで登録し、引用（出典）と先頭情報の形を検査する。
--
-- - file_name は実行フォルダ直下のファイル名。実行フォルダは todo → running →
--   review → R<n> と移るため、同じ report かどうかは「run_id + file_name」で見分ける。
-- - content_hash は md 本文の sha256。変わっていれば出典と問題を読み直す。
-- - header_ok は先頭情報（frontmatter）の検査に error が無いこと。崩れた report も
--   登録はする（黙って捨てない）が、header_ok = 0 として一覧で気づけるようにする。

ALTER TABLE reports ADD COLUMN file_name TEXT;
ALTER TABLE reports ADD COLUMN content_hash TEXT;
ALTER TABLE reports ADD COLUMN header_ok INTEGER NOT NULL DEFAULT 1;
ALTER TABLE reports ADD COLUMN checked_at TEXT;
ALTER TABLE reports ADD COLUMN check_errors INTEGER NOT NULL DEFAULT 0;
ALTER TABLE reports ADD COLUMN check_warnings INTEGER NOT NULL DEFAULT 0;

-- 同じ実行の同じファイル名を 1 つに限る（子タスクが並んで登録しても重複させない）。
CREATE UNIQUE INDEX IF NOT EXISTS idx_reports_run_file
  ON reports(run_id, file_name) WHERE file_name IS NOT NULL;

-- report の検査で見つけた問題（要確認・注意）。header 以外の kind は後の作業が入れる。
CREATE TABLE IF NOT EXISTS report_issues (
  report_id TEXT NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
  level TEXT NOT NULL CHECK (level IN ('error', 'warning')),
  kind TEXT NOT NULL,
  detail TEXT NOT NULL,
  line INTEGER
);
CREATE INDEX IF NOT EXISTS idx_report_issues_report ON report_issues(report_id);
