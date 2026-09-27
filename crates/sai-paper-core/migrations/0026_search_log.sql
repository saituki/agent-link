-- 0026_search_log.sql
-- 検索実行ログ: Discover で実行した検索とそのヒット論文を保存する。
--
-- 既存の search_sets（CLI / エージェント向けの検索セット）とは別物。
-- こちらは「いつ・どんな条件で検索し、何がヒットしたか」を自動記録するための
-- 実行履歴で、Library の階層表示と QUERY 画面が参照する。

CREATE TABLE IF NOT EXISTS search_logs (
    id          INTEGER PRIMARY KEY,
    searched_at TEXT NOT NULL,              -- 検索実行日時（ISO8601）
    log_date    TEXT NOT NULL,              -- 階層表示の日付グループ（YYYY/MM/DD）
    query       TEXT NOT NULL,              -- 検索ワード
    year        TEXT,                       -- 発行年区間（例: 2010-2020）
    limit_txt   TEXT,                       -- 取得件数（入力値そのまま）
    source_dbs  TEXT,                       -- 検索対象 DB（カンマ区切り）
    reason      TEXT                        -- QUERY 画面で入力する検索理由
);

CREATE TABLE IF NOT EXISTS search_log_items (
    log_id      INTEGER NOT NULL REFERENCES search_logs(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    remote_json TEXT NOT NULL,              -- RemotePaper の JSON
    dedupe_key  TEXT NOT NULL,              -- api::dedupe_key の返り値
    PRIMARY KEY (log_id, position)
);

CREATE INDEX IF NOT EXISTS idx_search_log_items_log
    ON search_log_items(log_id);
