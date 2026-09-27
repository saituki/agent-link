-- agent の report（md ファイル）の登録。
--
-- 背景: agent（cc / pi）が sai_paper_report ツールで保存した md は
-- sai-link の出力フォルダ（.sai/outputs/<owner>/<run_id>/）に置かれたまま、
-- sai-paper 側は「どの report がどの論文・引用箇所を参照したか」だけを記録する。
--
-- md の内容は sai-paper から書き換えない（閲覧のみ）。
-- report_sources.kind='report' により report 間の関係（複数 report から
-- 新しい report を作る）も表現できる。

CREATE TABLE reports (
  id TEXT PRIMARY KEY,             -- report_id（例: R-0007）
  title TEXT,
  md_path TEXT NOT NULL,           -- md の絶対パス（run の進行で移動する場合がある）
  run_id TEXT,                     -- 生成元の agent 実行 ID
  block TEXT,                      -- 実行した block id（分かる場合）
  created_at TEXT NOT NULL,
  summary TEXT                     -- md 先頭数行（Canvas カード表示用のキャッシュ）
);
CREATE INDEX idx_reports_md_path ON reports(md_path);
CREATE INDEX idx_reports_run ON reports(run_id);

CREATE TABLE report_sources (
  report_id TEXT NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,              -- paper | chunk | report
  ref_id TEXT NOT NULL,            -- papers.id / chunks.id / reports.id
  role TEXT,
  PRIMARY KEY (report_id, kind, ref_id)
);
CREATE INDEX idx_report_sources_kind_ref ON report_sources(kind, ref_id);
