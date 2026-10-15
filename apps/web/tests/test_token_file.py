"""合言葉のファイル（`<sai_home>/web/token`）を書く処理のテスト（assert だけ）．

`apps/web` は起動時に `main.write_token_file()` で
`<sai_home>/web/token` に合言葉（`AGENT_LINK_WEB_TOKEN`）を 0600 で書く．
エージェント側の道具 `phone_api.py` はこのファイルを読む（要件 D13）．

確かめるもの:
    - 合言葉があるときファイルが書かれる
    - ファイルの権限が 0600 である（既に別の桁で作られていても戻す）
    - 合言葉が無ければファイルは消える
    - 書けない場所でも例外を出さない

実行:
    PYTHONPATH=apps/web/src apps/web/.venv/bin/python apps/web/tests/test_token_file.py
"""

from __future__ import annotations

import dataclasses
import os
import shutil
import stat
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path

# リポジトリ直下から実行しなくても import できるようにしておく
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from agent_link_web import main as phone_main  # noqa: E402
from agent_link_web import registry  # noqa: E402


@contextmanager
def with_token(token: str | None, home: str):
    """`settings.api_token` と `AGENT_LINK_HOME` を差し替えて、終わりに戻す。

    `Settings` は frozen なので `dataclasses.replace` で新しい値を入れる。
    本物の設定ファイルは読まないし、書き換えない。
    """
    original = phone_main.settings
    phone_main.settings = dataclasses.replace(original, api_token=token)
    os.environ["AGENT_LINK_HOME"] = home
    try:
        yield
    finally:
        phone_main.settings = original


def main() -> None:
    tmp = tempfile.mkdtemp(prefix="agent-link-web-token-")
    home = os.path.join(tmp, "sai-home")
    os.environ["AGENT_LINK_HOME"] = home
    path = registry.sai_home() / "web" / "token"

    # 合言葉があるとき：ファイルが書かれ、0600 になる
    with with_token("first-secret", home):
        phone_main.write_token_file()
    assert path.is_file(), f"{path} が作られませんでした"
    assert path.read_text(encoding="utf-8") == "first-secret", "合言葉の内容が違います"
    mode = stat.S_IMODE(path.stat().st_mode)
    assert mode == 0o600, f"権限が 0600 ではなく {oct(mode)} です"

    # 既に 0644 で出来ている場合も 0600 へ戻す（os.open の umask に依存させない）
    path.chmod(0o644)
    with with_token("second-secret", home):
        phone_main.write_token_file()
    assert (
        stat.S_IMODE(path.stat().st_mode) == 0o600
    ), "既存のファイルの権限が 0600 に戻りませんでした"
    assert path.read_text(encoding="utf-8") == "second-secret", "内容が上書きされませんでした"

    # 合言葉が無ければファイルは消える（古い合言葉を残さないため）
    with with_token(None, home):
        phone_main.write_token_file()
    assert not path.exists(), "合言葉の無いときはファイルが消えるはずです"

    # ファイルが無い状態で起動し直しても壊れず、また書き戻せる
    with with_token(None, home):
        phone_main.write_token_file()
    with with_token("third-secret", home):
        phone_main.write_token_file()
    assert path.read_text(encoding="utf-8") == "third-secret", "書き戻しができませんでした"

    # 書けない場所でも例外を出さない（サーバを止めないため）
    blocked = os.path.join(tmp, "not-a-dir")
    with open(blocked, "w", encoding="utf-8") as fh:
        fh.write("x")
    with with_token("secret", blocked):
        phone_main.write_token_file()  # 例外が出なければよい

    shutil.rmtree(tmp, ignore_errors=True)
    os.environ.pop("AGENT_LINK_HOME", None)
    print("test_token_file: すべて通りました")


if __name__ == "__main__":
    main()
