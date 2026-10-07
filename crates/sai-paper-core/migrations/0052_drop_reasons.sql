-- 0052_drop_reasons.sql
-- research-tree の「理由」を使わなくなったので列を消す（OP の reason・TRIAGE のメモ・検索を広げた理由）。
ALTER TABLE rt_log DROP COLUMN reason;
ALTER TABLE rt_proposals DROP COLUMN reason;
ALTER TABLE rt_proposals DROP COLUMN decide_note;
ALTER TABLE rt_nodes DROP COLUMN removed_reason;
ALTER TABLE rt_searches DROP COLUMN widen_reason;
ALTER TABLE find_logs DROP COLUMN widen_reason;
