-- Suggest（agent が勧める論文）。候補は検索履歴の (history_id, position) を指す（論文の情報は写さない）。
-- history_id に外部キーを付けない（search_history_items の FK が削除の足枷になった前例があるため）。

CREATE TABLE IF NOT EXISTS paper_suggests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    date TEXT NOT NULL,
    executer TEXT NOT NULL,
    run_id INTEGER,
    block TEXT,
    reason TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS paper_suggest_items (
    suggest_id INTEGER NOT NULL REFERENCES paper_suggests(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    history_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    note TEXT,
    PRIMARY KEY (suggest_id, ord)
);
