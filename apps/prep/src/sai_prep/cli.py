"""sai-prep の命令。

- `sai-prep`（= `sai-prep serve`）: ローカルの画面を出す。
- `sai-prep setup`: 足りない道具を `uv tool install` で入れる。
- `sai-prep doctor`: 道具とサーバのつながりを表で出す。
"""

from __future__ import annotations

import argparse
import platform
import shutil
import subprocess
import sys
import webbrowser
from typing import Callable

from . import __version__, config as config_mod
from .client import Client, ClientError
from .pipeline import Pipeline, find_tool, tool_version

# 入れる道具: (uv tool install に渡す名前, 実行ファイル名)
TOOLS = [
    ("docling", "docling"),
    ("pdf2zh", "pdf2zh"),
    ("mlx-lm", "mlx_lm.server"),
]
PYTHON_VERSION = "3.12"


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="sai-prep",
        description="各自の Mac で論文 PDF を前処理してサーバへ送る小さなアプリ",
    )
    parser.add_argument("--version", action="version", version=f"sai-prep {__version__}")
    subparsers = parser.add_subparsers(dest="command")

    serve = subparsers.add_parser("serve", help="ローカルの画面を出す")
    serve.add_argument("--port", type=int, default=8765, help="待ち受けるポート（既定 8765）")
    serve.add_argument("--no-browser", action="store_true", help="ブラウザを開かない")

    setup = subparsers.add_parser("setup", help="足りない道具を入れる")
    setup.add_argument("--yes", action="store_true", help="確認せずに入れる")

    subparsers.add_parser("doctor", help="道具とサーバのつながりを確かめる")
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    commands = {"serve", "setup", "doctor"}
    # 命令を省いたら serve（`sai-prep` だけで起動できるように）。
    if not argv or argv[0] not in commands:
        if argv and argv[0] in ("-h", "--help", "--version"):
            pass
        else:
            argv = ["serve", *argv]

    parser = _build_parser()
    args = parser.parse_args(argv)
    if args.command == "setup":
        return cmd_setup(args)
    if args.command == "doctor":
        return cmd_doctor()
    return cmd_serve(args)


# -- serve -----------------------------------------------------------------


def cmd_serve(args: argparse.Namespace) -> int:
    from . import server as server_mod

    pipeline = Pipeline(config_mod.load)
    httpd = server_mod.create_server(config_mod.load, pipeline, port=args.port)
    url = f"http://127.0.0.1:{args.port}/"
    print(f"sai-prep の画面: {url}")
    print("終わるときは Ctrl+C を押してください。")
    if not args.no_browser:
        try:
            webbrowser.open(url)
        except Exception:
            pass
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n止めました。")
    finally:
        httpd.server_close()
        pipeline.close()
    return 0


# -- setup -----------------------------------------------------------------


def _uv_binary() -> str | None:
    return shutil.which("uv")


def cmd_setup(args: argparse.Namespace) -> int:
    uv = _uv_binary()
    if not uv:
        print("uv が見つかりません。先に uv を入れてください。")
        return 1
    missing = [(pkg, cmd) for pkg, cmd in TOOLS if not find_tool(cmd)]
    if not missing:
        print("道具はそろっています。")
        return cmd_doctor()
    for package, command in missing:
        argv = [uv, "tool", "install", "--python", PYTHON_VERSION, package]
        print("実行する命令: " + " ".join(argv))
        if not args.yes:
            answer = input(f"{package} を入れますか？ [y/N] ").strip().lower()
            if answer not in ("y", "yes"):
                print(f"{package} を飛ばしました。")
                continue
        done = subprocess.run(argv)
        if done.returncode != 0:
            print(f"{package} を入れられませんでした。")
            return done.returncode
    return cmd_doctor()


# -- doctor ----------------------------------------------------------------


def _is_apple_silicon() -> bool:
    return platform.system() == "Darwin" and platform.machine() == "arm64"


def doctor_rows(client_factory: Callable[[str, str], Client] = Client) -> list[tuple[str, str]]:
    """doctor の表の中身（テストできるよう分けてある）。"""
    rows: list[tuple[str, str]] = []

    if _is_apple_silicon():
        rows.append(("Apple Silicon", f"はい（{platform.machine()}）"))
    else:
        rows.append(
            ("Apple Silicon", f"いいえ（{platform.system()} {platform.machine()}）— 翻訳には Apple Silicon が要ります")
        )

    for _, command in TOOLS:
        binary = find_tool(command)
        if not binary:
            rows.append((command, "無い — `sai-prep setup` を実行してください"))
            continue
        if command == "mlx_lm.server":
            # `--help` が動くかを見る（`--version` は無い）。
            try:
                done = subprocess.run(
                    [binary, "--help"], capture_output=True, text=True, timeout=60
                )
                ok = done.returncode == 0
            except (OSError, subprocess.TimeoutExpired):
                ok = False
            rows.append((command, f"あり（{binary}）" if ok else f"動きません（{binary}）"))
        else:
            version = tool_version(binary)
            rows.append((command, f"あり（{version or binary}）"))

    config = config_mod.load()
    if not config.server_url:
        rows.append(("サーバ", "未設定 — 画面で住所とトークンを入れてください"))
    elif not config.token:
        rows.append(("サーバ", "トークンが未設定です"))
    else:
        try:
            me = client_factory(config.server_url, config.token).me()
            rows.append(("サーバ", f"つながりました（{me.get('name', '')}）"))
        except ClientError as error:
            rows.append(("サーバ", f"つながりません: {error}"))
    return rows


def cmd_doctor() -> int:
    print(f"sai-prep {__version__} の状態")
    rows = doctor_rows()
    width = max(len(label) for label, _ in rows)
    for label, value in rows:
        print(f"  {label.ljust(width)} : {value}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
