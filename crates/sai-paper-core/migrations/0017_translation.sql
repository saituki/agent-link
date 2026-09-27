-- 翻訳機能用スキーマ（abstract 日本語訳 + PDF翻訳状態管理）
-- 論文の abstract を日本語へ翻訳した結果を papers テーブルにキャッシュする
-- PDF翻訳（pdf2zh）はファイル生成（papers/*-ja.pdf）とともに状態を paper_translations で管理する

-- 1. abstract日本語訳キャッシュ（papers テーブル拡張）
ALTER TABLE papers ADD COLUMN abstract_ja TEXT;
ALTER TABLE papers ADD COLUMN abstract_ja_translated_at TEXT;
ALTER TABLE papers ADD COLUMN abstract_ja_service TEXT;

-- 2. PDF翻訳状態管理（引用抽出の paper_extracts と同パターン）
CREATE TABLE IF NOT EXISTS paper_translations (
    paper_id        TEXT PRIMARY KEY,           -- 抽出キー（s2 40hex or ArXiv:/DOI:）
    status          TEXT NOT NULL DEFAULT 'none', -- none/translating/ready/failed
    pdf_path        TEXT,                       -- 元PDFパス（stale判定用）
    pdf_mtime       TEXT,                       -- 元PDFのmtime（unix秒文字列）
    pdf_ja_path     TEXT,                       -- 生成された日本語PDFパス
    service         TEXT,                       -- 翻訳サービス（google 等）
    lang_in         TEXT DEFAULT 'en',
    lang_out        TEXT DEFAULT 'ja',
    error           TEXT,                       -- 失敗時のメッセージ
    translated_at   TEXT                        -- ISO8601
);

CREATE INDEX IF NOT EXISTS idx_paper_translations_status ON paper_translations(status);
