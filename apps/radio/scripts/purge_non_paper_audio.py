"""sai-paper と結びついていない古い音声を削除する道具（既定は確認だけ）．

やること:
1. papers テーブルから，ID が数字だけの論文（= sai-paper と結びついた論文）を除いた
   もの，つまり「試しに作った古い音声」の候補を洗い出す
2. 既定では一覧を表で出すだけ（何も消さない）
3. `--apply` を付けたときだけ，events → papers → 音声ファイル の順に消す

なぜ ID で判断するか:
    sai-paper から来た音声の ID は sai-paper の論文 ID（例 "49"）で，必ず数字だけ．
    ブラウザのアップロードや CLI で作った音声は 16進文字列や demo001 など，
    数字だけではない ID になる．

使い方（確認だけ・消さない）:
    PYTHONPATH=src python scripts/purge_non_paper_audio.py
    PYTHONPATH=src python scripts/purge_non_paper_audio.py --db <DBパス> --audio-dir <音声フォルダ>

実際に消す（必ず一度，確認の出力を読んでから）:
    PYTHONPATH=src python scripts/purge_non_paper_audio.py --apply
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path

# プロジェクトルートを基準に src を読み込めるようにする．
# (リポジトリ直下の scripts/ から実行する想定)
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from sai_radio.config import AUDIO_DIR, DB_PATH  # noqa: E402


def _connect(db_path: Path, read_only: bool) -> sqlite3.Connection:
    """DB を開く．確認だけのときは読み取り専用で開き，誤って書き込めないようにする．"""
    if read_only:
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    else:
        conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    return conn


def _find_targets(conn: sqlite3.Connection) -> list[sqlite3.Row]:
    """sai-paper と結びついていない論文（ID が数字だけではないもの）を返す．"""
    rows = conn.execute(
        "SELECT id, title, audio_path FROM papers ORDER BY created_at DESC"
    ).fetchall()
    return [r for r in rows if not str(r["id"]).isdigit()]


def _event_count(conn: sqlite3.Connection, paper_id: str) -> int:
    row = conn.execute(
        "SELECT COUNT(*) AS c FROM events WHERE paper_id = ?", (paper_id,)
    ).fetchone()
    return int(row["c"])


def _print_table(targets: list[sqlite3.Row], conn: sqlite3.Connection, audio_dir: Path) -> None:
    """対象を表で出す（id・title・音声ファイルのパス・events の件数）．"""
    if not targets:
        print("対象の音声はありません（sai-paper と結びついていない音声は 0 件）．")
        return

    header = ("ID", "TITLE", "AUDIO", "EVENTS")
    rows = []
    for r in targets:
        audio_file = audio_dir / (r["audio_path"] or "")
        rows.append((
            str(r["id"]),
            str(r["title"] or ""),
            f"{audio_file}{'（ファイル無し）' if not audio_file.exists() else ''}",
            str(_event_count(conn, str(r["id"]))),
        ))

    widths = [len(h) for h in header]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    def _line(cells: tuple[str, ...]) -> str:
        return "  ".join(cell.ljust(widths[i]) for i, cell in enumerate(cells))

    print(_line(header))
    print(_line(tuple("-" * w for w in widths)))
    for row in rows:
        print(_line(row))
    print(f"\n対象: {len(targets)} 件（この状態では何も消していません）")


def _apply_delete(targets: list[sqlite3.Row], conn: sqlite3.Connection, audio_dir: Path) -> None:
    """実際に消す: events → papers → 音声ファイル の順に削除する．"""
    events_deleted = 0
    papers_deleted = 0
    files_deleted = 0

    for r in targets:
        paper_id = str(r["id"])
        events_deleted += _event_count(conn, paper_id)
        conn.execute("DELETE FROM events WHERE paper_id = ?", (paper_id,))
        conn.execute("DELETE FROM papers WHERE id = ?", (paper_id,))
        papers_deleted += 1

        audio_file = audio_dir / (r["audio_path"] or "")
        if audio_file.exists():
            audio_file.unlink()
            files_deleted += 1

    conn.commit()
    print(f"削除しました: 論文 {papers_deleted} 件 / 履歴 {events_deleted} 件 / 音声ファイル {files_deleted} 個")


def main() -> None:
    parser = argparse.ArgumentParser(description="sai-paper と結びついていない古い音声を削除する")
    parser.add_argument("--db", type=Path, default=DB_PATH, help="SQLite DB のパス（既定は sai_radio.config の値）")
    parser.add_argument("--audio-dir", type=Path, default=AUDIO_DIR, help="音声フォルダ（既定は sai_radio.config の値）")
    parser.add_argument(
        "--apply",
        action="store_true",
        help="この印を付けたときだけ実際に消す（無ければ一覧を出すだけ）",
    )
    args = parser.parse_args()

    if not args.db.exists():
        print(f"DB が見つかりません: {args.db}", file=sys.stderr)
        sys.exit(1)

    # 既定（確認だけ）は読み取り専用で開き，--apply のときだけ書き込みで開く．
    with _connect(args.db, read_only=not args.apply) as conn:
        targets = _find_targets(conn)
        if not args.apply:
            _print_table(targets, conn, args.audio_dir)
            return
        if not targets:
            print("対象の音声はありません（消すものがありません）．")
            return
        _print_table(targets, conn, args.audio_dir)
        print()
        _apply_delete(targets, conn, args.audio_dir)


if __name__ == "__main__":
    main()
