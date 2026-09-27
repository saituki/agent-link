-- 断片（チャンク）に、その文がどの章にあったかを残す（R02）。
--
-- 検索の手掛かりを「論文」から「章」まで細かくするための欄。
-- 値の形は見出しを ` > ` でつないだもの（例 `3 Methods > 3.2 Preprocessing`）。
-- 要旨の断片は `Abstract`、ノートの断片は `note`。
-- 文単位（sentence）と窓（window）の断片は、元の表に章の情報が無いので NULL。
-- 既存の断片は索引を作り直す（`sai-paper index rebuild`）まで空のまま。
ALTER TABLE chunks ADD COLUMN section TEXT;
