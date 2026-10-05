-- 0050_rt_search_log.sql
-- 検索の記録（rt_searches）に、その 1 回の検索の履歴（search_sets の行）を結ぶ。
-- set_id は --save-as の名前付きセット（複数回の検索が足される）、log_id は毎回の履歴。

ALTER TABLE rt_searches ADD COLUMN log_id INTEGER;
