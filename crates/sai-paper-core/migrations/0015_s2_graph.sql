-- S2 graph cache for papr TUI (shared with core)
CREATE TABLE IF NOT EXISTS s2_graph_nodes (
    root_id TEXT NOT NULL,
    paper_id TEXT NOT NULL,
    depth INTEGER NOT NULL,
    direction TEXT NOT NULL,
    title TEXT,
    year INTEGER,
    venue TEXT,
    citation_count INTEGER,
    PRIMARY KEY (root_id, paper_id)
);
CREATE TABLE IF NOT EXISTS s2_graph_edges (
    root_id TEXT NOT NULL,
    src TEXT NOT NULL,
    dst TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (root_id, src, dst)
);
CREATE INDEX IF NOT EXISTS idx_s2_nodes_root ON s2_graph_nodes(root_id);
CREATE INDEX IF NOT EXISTS idx_s2_edges_root ON s2_graph_edges(root_id);
