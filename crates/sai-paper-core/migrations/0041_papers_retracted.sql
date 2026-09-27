-- 撤回（retraction）された論文の印。
--
-- 撤回された論文を根拠にした結論は研究の方向を誤らせるため、OpenAlex の
-- `is_retracted` を論文ごとに残す。ネットにつなぐのはユーザーが押したときだけで、
-- `sai-paper enrich --retraction`（既にある論文）か、既存のメタデータの補完の
-- ついでに確かめる。
--
-- - is_retracted は NULL = 未確認、0 = 撤回されていない、1 = 撤回。
--   「確かめていない（NULL）」と「撤回されていない（0）」を分けることで、
--   確かめていないものを「問題なし」と見せない。
-- - retraction_checked_at は最後に確かめた時刻（ISO8601）。NULL なら未確認。
--   次の `enrich --retraction` はこの列が NULL の論文だけを問い合わせる
--   （`--force` を付けると全部）。

ALTER TABLE papers ADD COLUMN is_retracted INTEGER;
ALTER TABLE papers ADD COLUMN retraction_checked_at TEXT;
