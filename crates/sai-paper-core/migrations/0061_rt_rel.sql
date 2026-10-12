-- 0061_rt_rel.sql
-- A の子の関係。NULL は subtype と読む。追加だけの移行（既存の行・CHECK は変えない）。

ALTER TABLE rt_nodes ADD COLUMN rel TEXT CHECK (rel IS NULL OR rel IN ('subtype','condition'));
