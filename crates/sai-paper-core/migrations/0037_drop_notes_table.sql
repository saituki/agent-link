-- 段階 9: ノートは論文フォルダの note.md だけにする。
-- notes 表の中身は、この migration の前に Database::migrate が note.md へ移す
-- （note.md が無ければ書き、中身が違えば末尾に足す）。
DROP TABLE IF EXISTS notes;
-- どこからも使っていない表（中身は空）。
DROP TABLE IF EXISTS litbase_jobs;
