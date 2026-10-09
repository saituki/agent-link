"""sai-ts の CLI。U01 は serve と demo だけ（U06 が本実装に置き換える）。"""

from __future__ import annotations

import argparse
import sys


def _fail(msg: str, code: int) -> int:
    print(msg, file=sys.stderr)
    return code


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="sai-ts")
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve")
    s.add_argument("--port", type=int)
    d = sub.add_parser("demo")
    d.add_argument("ds")
    d.add_argument("--records", type=int, default=3)
    d.add_argument("--vars", type=int, default=4)
    d.add_argument("--seconds", type=float, default=60.0)
    d.add_argument("--srate", type=float, default=500.0)
    a = p.parse_args(argv)

    from . import paths

    try:
        if a.cmd == "serve":
            from .server import serve

            return serve(a.port)
        from . import demo

        # 名前は引数のまま検査する（Path は ../ を畳むので、畳んだ後では見えない）
        demo.make(paths.datasets_dir() / paths.check_name(a.ds), a.records, a.vars, a.seconds, a.srate)
        return 0
    except paths.ProjectNotFound as e:
        return _fail(str(e), 2)
    except ValueError as e:
        return _fail(str(e), 2)
    except KeyError as e:
        return _fail(str(e.args[0]) if e.args else str(e), 3)
    except FileExistsError as e:
        return _fail(str(e), 4)


if __name__ == "__main__":
    sys.exit(main())
