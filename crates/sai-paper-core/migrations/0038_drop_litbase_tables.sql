-- 使われていない litbase_* の表を消す（中身は 2026-09-25 時点で初期値だけ）。
-- 現況は .sai/paper/state.md、ノートは論文フォルダの note.md が正。
DROP TABLE IF EXISTS litbase_rfd_citations;
DROP TABLE IF EXISTS litbase_documents;
DROP TABLE IF EXISTS litbase_reading_list;
DROP TABLE IF EXISTS litbase_discussions;
DROP TABLE IF EXISTS litbase_notes;
DROP TABLE IF EXISTS litbase_search_basis;
DROP TABLE IF EXISTS litbase_search_tiers;
DROP TABLE IF EXISTS litbase_profile;
