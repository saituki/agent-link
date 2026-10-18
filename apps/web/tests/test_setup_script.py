"""`scripts/agent-link-setup`（install / doctor / uninstall）を確かめる。

利用者の本物の `~` は使わない。`HOME` と `AGENT_LINK_HOME` は一時フォルダ、`cargo` と `uv` は
呼ばれた記録だけ残す偽物（`fakebin/`）に差し替える。

実行:
    cd <作業用コピー>
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_setup_script.py
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SCRIPT = REPO / "scripts" / "agent-link-setup"

T = Path(tempfile.mkdtemp(prefix="agent-link-setup-"))
HOME = T / "home"
ALHOME = T / "alhome"
BIN = T / "bin"
FAKE = T / "fakebin"
EXTBIN = T / "extbin"  # 拡張の確かめ用の agent-link の偽物だけを置く
for d in (HOME, ALHOME, BIN, FAKE, EXTBIN):
    d.mkdir()
for tool in ("cargo", "uv"):
    f = FAKE / tool
    f.write_text(f'#!/bin/sh\necho "$0 $*" >> {T}/calls\nexit 0\n')
    f.chmod(0o755)


def run(*args: str, alhome: Path = ALHOME, path: str | None = None) -> subprocess.CompletedProcess:
    env = {
        "HOME": str(HOME),
        "AGENT_LINK_HOME": str(alhome),
        "AGENT_LINK_WEB_PORT": "9",  # 応えない番号
        "PATH": path or f"{FAKE}:/usr/bin:/bin",
    }
    return subprocess.run(
        [str(SCRIPT), *args], env=env, capture_output=True, text=True, timeout=60
    )


def main() -> None:
    try:
        # 1. install: リンクが 2 つ、cargo / uv が呼ばれ、PATH に足す行を表示するだけ
        r = run("install", "--bin-dir", str(BIN))
        assert r.returncode == 0, r.stderr
        assert os.readlink(BIN / "agent-link") == str(REPO / "target/release/agent-link")
        assert os.readlink(BIN / "agent-link-web") == str(REPO / "apps/web/bin/agent-link-web")
        calls = (T / "calls").read_text()
        assert "cargo build --release" in calls and "uv sync" in calls, calls
        assert "export PATH=" in r.stdout, r.stdout
        assert not (HOME / ".zshrc").exists()

        # 2. doctor: 各行の形と項目
        r = run("doctor")
        assert r.returncode in (0, 1), r.stderr
        lines = r.stdout.splitlines()
        assert lines and all(re.match(r"^(ok|warn|NG) +\S+", ln) for ln in lines), r.stdout
        items = {ln.split()[1]: ln.split()[0] for ln in lines}
        for k in ("tmux", "agent-link", "web", "web-venv", "hooks", "registry"):
            assert k in items, r.stdout
        assert items["web"] == "warn", r.stdout
        assert (r.returncode == 1) == ("NG" in items.values()), r.stdout

        # 2b. 壊れた登録簿は NG で終了コード 1（.venv の python が要るので、無ければ飛ばす）
        if (REPO / "apps/web/.venv/bin/python").exists():
            (ALHOME / "registry.json").write_text("{not json")
            r = run("doctor")
            assert r.returncode == 1, r.stdout
            assert re.search(r"^NG +registry\b", r.stdout, re.M), r.stdout
            (ALHOME / "registry.json").unlink()

        # 2c. 登録簿の root に .sai があると ownership が warn（無ければ ok）
        proj = T / "proj"
        (proj / ".agent-link").mkdir(parents=True)
        (ALHOME / "registry.json").write_text(
            json.dumps([{"name": "proj", "root": str(proj), "kind": "link"}])
        )
        r = run("doctor")
        assert re.search(r"^ok +ownership +no project has \.sai$", r.stdout, re.M), r.stdout
        (proj / ".sai").mkdir()
        r = run("doctor")
        assert re.search(r"^warn +ownership +\S.* has both \.agent-link and \.sai", r.stdout, re.M), r.stdout
        assert "no project has .sai" not in r.stdout, r.stdout
        (ALHOME / "registry.json").unlink()

        # 2d. extensions: 偽物の agent-link（ext list --json の固定の JSON を返す）を PATH の先頭に置く
        if (REPO / "apps/web/.venv/bin/python").exists():
            (EXTBIN / "agent-link").write_text(f"#!/bin/sh\ncat {T}/ext.json\n")
            (EXTBIN / "agent-link").chmod(0o755)
            ext_path = f"{EXTBIN}:{FAKE}:/usr/bin:/bin"
            (T / "ext.json").write_text(json.dumps({
                "dir": "x", "extensions": [{"name": "hello"}],
                "errors": [{"path": "/x/broken", "error": "extension.json: unknown key 'hooks'"}],
            }))
            r = run("doctor", path=ext_path)
            assert re.search(
                r"^warn +extensions +/x/broken: extension\.json: unknown key 'hooks'$", r.stdout, re.M
            ), r.stdout
            (T / "ext.json").write_text(json.dumps({"dir": "x", "extensions": [{"name": "hello"}], "errors": []}))
            r = run("doctor", path=ext_path)
            assert re.search(r"^ok +extensions +1 installed$", r.stdout, re.M), r.stdout
            (T / "ext.json").write_text("not json")
            r = run("doctor", path=ext_path)
            assert re.search(r"^warn +extensions +could not run agent-link ext list$", r.stdout, re.M), r.stdout

        # 3. 普通のファイルは上書きしない
        (BIN / "agent-link").unlink()
        (BIN / "agent-link").write_text("mine\n")
        r = run("install", "--bin-dir", str(BIN))
        assert r.returncode == 1, r.stdout
        assert (BIN / "agent-link").read_text() == "mine\n"

        # 4. uninstall: リンクだけ消す。普通のファイルは残す
        r = run("uninstall", "--bin-dir", str(BIN))
        assert r.returncode == 0, r.stderr
        assert not os.path.lexists(BIN / "agent-link-web")
        assert (BIN / "agent-link").read_text() == "mine\n"
        (BIN / "agent-link").unlink()

        # 5. --purge-home: $AGENT_LINK_HOME を消す。HOME と同じなら消さない
        (ALHOME / "registry.json").write_text("[]\n")
        r = run("uninstall", "--bin-dir", str(BIN), "--purge-home")
        assert r.returncode == 0, r.stderr
        assert not ALHOME.exists()
        r = run("uninstall", "--bin-dir", str(BIN), "--purge-home", alhome=HOME)
        assert r.returncode == 1, r.stdout
        assert HOME.is_dir()

        # 知らない引数は 2
        assert run("install", "--nope").returncode == 2
        assert run().returncode == 2
        print("ok")
    finally:
        shutil.rmtree(T, ignore_errors=True)


if __name__ == "__main__":
    main()
