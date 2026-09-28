-- 0046_sci_tree.sql
-- sai-sci の研究方針の木（Research-tree）。正はこの表で、state.md と tree.md は書き出し。

-- プロジェクトに 1 つの値（type = 研究の型の id / initialized_at = sci を始めた時刻）
CREATE TABLE IF NOT EXISTS sci_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- 節点（項目と候補）。消さない（対象外・却下・休眠にする）
CREATE TABLE IF NOT EXISTS sci_nodes (
  id               INTEGER PRIMARY KEY,
  kind             TEXT NOT NULL CHECK (kind IN ('item', 'option')),
  item_key         TEXT,
  item_types       TEXT NOT NULL DEFAULT '',
  title            TEXT NOT NULL CHECK (length(trim(title)) > 0),
  body             TEXT NOT NULL DEFAULT '',
  state            TEXT NOT NULL,
  fixed            INTEGER NOT NULL DEFAULT 0 CHECK (fixed IN (0, 1)),
  ord              INTEGER NOT NULL DEFAULT 0,
  created_by       TEXT NOT NULL DEFAULT 'user',
  created_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  state_changed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (
    (kind = 'item' AND state IN ('open', 'tentative', 'decided', 'excluded'))
    OR (kind = 'option' AND state IN ('candidate', 'selected', 'rejected', 'dormant'))
  )
);
-- 項目ガイドから作った項目は、キーごとに 1 つだけ
CREATE UNIQUE INDEX IF NOT EXISTS idx_sci_nodes_key
  ON sci_nodes(item_key) WHERE item_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS sci_edges (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('parent', 'depends', 'derived', 'analogous')),
  from_node  INTEGER NOT NULL REFERENCES sci_nodes(id),
  to_node    INTEGER NOT NULL REFERENCES sci_nodes(id),
  note       TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (from_node != to_node),
  UNIQUE (kind, from_node, to_node)
);
CREATE INDEX IF NOT EXISTS idx_sci_edges_to ON sci_edges(to_node, kind);

CREATE TABLE IF NOT EXISTS sci_questions (
  id         INTEGER PRIMARY KEY,
  node_id    INTEGER NOT NULL REFERENCES sci_nodes(id),
  text       TEXT NOT NULL CHECK (length(trim(text)) > 0),
  state      TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'answered', 'withdrawn')),
  answer     TEXT NOT NULL DEFAULT '',
  from_guide INTEGER NOT NULL DEFAULT 0 CHECK (from_guide IN (0, 1)),
  created_by TEXT NOT NULL DEFAULT 'user',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sci_questions_node ON sci_questions(node_id);

-- 知見。外すときは removed_at を入れる（行は消さない）
CREATE TABLE IF NOT EXISTS sci_findings (
  id          INTEGER PRIMARY KEY,
  node_id     INTEGER NOT NULL REFERENCES sci_nodes(id),
  question_id INTEGER REFERENCES sci_questions(id),
  text        TEXT NOT NULL CHECK (length(trim(text)) > 0),
  cites       TEXT NOT NULL DEFAULT '[]',
  polarity    TEXT NOT NULL CHECK (polarity IN ('support', 'refute', 'neutral', 'none')),
  level       INTEGER CHECK (level IS NULL OR level BETWEEN 1 AND 4),
  source      TEXT NOT NULL CHECK (source IN ('report', 'note', 'user', 'run')),
  report_id   TEXT,
  paper_id    INTEGER,
  run_id      INTEGER,
  proposal_id INTEGER,
  auto        INTEGER NOT NULL DEFAULT 0 CHECK (auto IN (0, 1)),
  removed_at  TEXT,
  created_by  TEXT NOT NULL DEFAULT 'user',
  created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sci_findings_node ON sci_findings(node_id);

-- 結び付き。外すときは removed_at を入れる（同じ組をもう一度結ぶと removed_at を消して戻す）
CREATE TABLE IF NOT EXISTS sci_links (
  id          INTEGER PRIMARY KEY,
  node_id     INTEGER NOT NULL REFERENCES sci_nodes(id),
  target_kind TEXT NOT NULL CHECK (target_kind IN ('paper', 'report', 'note', 'set', 'run')),
  target_ref  TEXT NOT NULL CHECK (length(trim(target_ref)) > 0),
  role        TEXT CHECK (role IS NULL OR role IN
                ('exemplar', 'compare', 'issue_evidence', 'method_source', 'data_source', 'background', 'counter')),
  read_state  TEXT CHECK (read_state IS NULL OR read_state IN ('unread', 'read', 'reflected', 'irrelevant')),
  reason      TEXT NOT NULL DEFAULT '',
  proposal_id INTEGER,
  auto        INTEGER NOT NULL DEFAULT 0 CHECK (auto IN (0, 1)),
  removed_at  TEXT,
  created_by  TEXT NOT NULL DEFAULT 'user',
  created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK ((target_kind = 'paper') = (role IS NOT NULL)),
  CHECK ((target_kind = 'paper') = (read_state IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sci_links_unique
  ON sci_links(node_id, target_kind, target_ref, COALESCE(role, ''));
CREATE INDEX IF NOT EXISTS idx_sci_links_target ON sci_links(target_kind, target_ref);

-- 決定の記録。追記するだけ（直す・消すはトリガーが止める）
CREATE TABLE IF NOT EXISTS sci_decisions (
  id               INTEGER PRIMARY KEY,
  node_id          INTEGER NOT NULL REFERENCES sci_nodes(id),
  action           TEXT NOT NULL CHECK (action IN
                     ('decide', 'tentative', 'reopen', 'exclude_item', 'reject_option', 'dormant_option', 'restore_option')),
  option_id        INTEGER REFERENCES sci_nodes(id),
  reason           TEXT NOT NULL DEFAULT '',
  evidence         TEXT NOT NULL DEFAULT '[]',
  rejected         TEXT NOT NULL DEFAULT '[]',
  counter_searched INTEGER NOT NULL DEFAULT 0 CHECK (counter_searched IN (0, 1)),
  actor            TEXT NOT NULL,
  proposal_id      INTEGER,
  created_at       TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sci_decisions_node ON sci_decisions(node_id, created_at);
CREATE TRIGGER IF NOT EXISTS sci_decisions_no_update BEFORE UPDATE ON sci_decisions
BEGIN SELECT RAISE(ABORT, 'sci_decisions は追記だけです'); END;
CREATE TRIGGER IF NOT EXISTS sci_decisions_no_delete BEFORE DELETE ON sci_decisions
BEGIN SELECT RAISE(ABORT, 'sci_decisions は追記だけです'); END;

CREATE TABLE IF NOT EXISTS sci_proposals (
  id          INTEGER PRIMARY KEY,
  run_id      INTEGER,
  block       TEXT NOT NULL DEFAULT '',
  op          TEXT NOT NULL CHECK (op IN
                ('add_finding', 'add_question', 'answer_question', 'link', 'add_seed',
                 'add_option', 'add_child', 'decide', 'reopen', 'exclude_item', 'option_state',
                 'exclude', 'edit_text', 'add_edge')),
  node_id     INTEGER REFERENCES sci_nodes(id),
  payload     TEXT NOT NULL,
  reason      TEXT NOT NULL DEFAULT '',
  risk        TEXT NOT NULL CHECK (risk IN ('auto', 'human')),
  status      TEXT NOT NULL CHECK (status IN ('applied', 'pending', 'accepted', 'rejected', 'reverted')),
  result_ref  TEXT,
  decided_by  TEXT,
  decided_at  TEXT,
  decide_note TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sci_proposals_status ON sci_proposals(status, node_id);

-- 墓標（外した論文・検索の候補）。再検索・再読を防ぐために残す
CREATE TABLE IF NOT EXISTS sci_tombstones (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('paper', 'candidate')),
  paper_id   INTEGER,
  doi        TEXT,
  arxiv_id   TEXT,
  dedupe_key TEXT,
  title      TEXT NOT NULL DEFAULT '',
  reason     TEXT NOT NULL CHECK (length(trim(reason)) > 0),
  node_id    INTEGER REFERENCES sci_nodes(id),
  run_id     INTEGER,
  created_by TEXT NOT NULL DEFAULT 'user',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (paper_id IS NOT NULL OR doi IS NOT NULL OR arxiv_id IS NOT NULL OR dedupe_key IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS idx_sci_tombstones_paper ON sci_tombstones(paper_id);
CREATE INDEX IF NOT EXISTS idx_sci_tombstones_doi ON sci_tombstones(doi);
CREATE INDEX IF NOT EXISTS idx_sci_tombstones_dedupe ON sci_tombstones(dedupe_key);

CREATE TABLE IF NOT EXISTS sci_seeds (
  id         INTEGER PRIMARY KEY,
  title      TEXT NOT NULL CHECK (length(trim(title)) > 0),
  sentence   TEXT NOT NULL,
  path       TEXT NOT NULL DEFAULT '[]',
  doc_ref    TEXT NOT NULL DEFAULT '',
  run_id     INTEGER,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 問いに結び付けた検索の記録（find / search / expand）
CREATE TABLE IF NOT EXISTS sci_searches (
  id           INTEGER PRIMARY KEY,
  run_id       INTEGER,
  node_id      INTEGER REFERENCES sci_nodes(id),
  question_id  INTEGER REFERENCES sci_questions(id),
  tool         TEXT NOT NULL CHECK (tool IN ('find', 'search', 'expand')),
  query        TEXT NOT NULL,
  scope        TEXT NOT NULL,
  widen_reason TEXT NOT NULL DEFAULT '',
  hits         INTEGER NOT NULL DEFAULT 0,
  hit_papers   TEXT NOT NULL DEFAULT '[]',
  set_id       INTEGER,
  created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sci_searches_node ON sci_searches(node_id, created_at);

-- 検索セットに、何のための検索かを持たせる（要件 R-R6）
ALTER TABLE search_sets ADD COLUMN sci_node INTEGER;
ALTER TABLE search_sets ADD COLUMN sci_question INTEGER;
ALTER TABLE search_sets ADD COLUMN purpose TEXT;
ALTER TABLE search_sets ADD COLUMN extract TEXT;
