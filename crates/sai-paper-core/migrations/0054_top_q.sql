-- 0054_top_q.sql
-- 一番上の Q をやめ、SEED の下に top_Q（`parent_id` が NULL の Q）を複数並べる。
--
-- 対象: 親が NULL の生きた Q で、生きた O の子が無く、生きた Q の子が 1 つ以上ある物。
--       その Q の子の Q を top_Q（`parent_id` = NULL）に上げ、その Q を消した Node にする。
--       生きた O の子を持つ一番上の Q は、答えの記録を失わないためそのまま top_Q として残す。
--
-- 例（~/research/west）: Q1（子の Q が 7 つ・O なし）→ 7 つの Q が top_Q になり、Q1 は消える。

CREATE TEMP TABLE rt54_target AS
SELECT q.id AS q_id
FROM rt_nodes q
WHERE q.kind = 'Q'
  AND q.parent_id IS NULL
  AND q.removed_at IS NULL
  AND NOT EXISTS (
      SELECT 1 FROM rt_nodes o
      WHERE o.parent_id = q.id AND o.kind = 'O' AND o.removed_at IS NULL
  )
  AND EXISTS (
      SELECT 1 FROM rt_nodes c
      WHERE c.parent_id = q.id AND c.kind = 'Q' AND c.removed_at IS NULL
  );

-- 子の Q を top_Q に上げる（先に上げてから、親を消す）。
UPDATE rt_nodes
SET parent_id = NULL, updated_at = CURRENT_TIMESTAMP
WHERE kind = 'Q'
  AND removed_at IS NULL
  AND parent_id IN (SELECT q_id FROM rt54_target);

-- 一番上の Q を消した Node にする。
UPDATE rt_nodes
SET removed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
WHERE id IN (SELECT q_id FROM rt54_target);

DROP TABLE rt54_target;
