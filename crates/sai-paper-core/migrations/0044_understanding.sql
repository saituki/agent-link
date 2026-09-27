-- 0044_understanding.sql
-- トピックごとの「ユーザーの理解の状態」を残す表（N04）。
--
--   ok      : 理解している
--   vague   : あいまい
--   unknown : まだ確かめていない
--
-- 書き足していくだけで履歴を残す（画面と brief はトピックごとの最新の 1 件を使う）。
-- 保存するのは「理解の確認」の DSR がユーザーの確認を取った後だけである。
CREATE TABLE IF NOT EXISTS understanding (
  id INTEGER PRIMARY KEY,
  topic TEXT NOT NULL CHECK (length(trim(topic)) > 0),
  state TEXT NOT NULL CHECK (state IN ('ok', 'vague', 'unknown')),
  evidence TEXT NOT NULL DEFAULT '',
  run_id INTEGER,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_understanding_topic
  ON understanding(topic, created_at DESC);
