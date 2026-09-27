"""slash コマンドの送り方の小さなテスト（assert だけ）・計画書 B U07．

確かめること:
- claude には `agent prompt`（貼り付け）ではなく `pane send-text` で入れる
  （貼り付けだと末尾の Enter が改行として残り，2 回目以降の slash コマンドが
  `\n/context` になって「文字として」送られる・実機で確認した）
- claude の slash だけ ctrl+x → enter（chat:queueSubmit），普通の文は alt+enter
- pi と agent 不明のときは従来どおり `agent prompt` + alt+enter
- blocked の agent には送らない（pane send-text には herdr のガードが無いため）

実行:
    PYTHONPATH=src uv run python tests/test_herdr_submit.py
"""

from __future__ import annotations

import json
import subprocess

from sai_phone import herdr


class _FakeProc:
    """subprocess.run の戻り値の代わり（終了コードと標準出力・標準エラー）．"""

    def __init__(self, returncode: int, stdout: str = "", stderr: str = "") -> None:
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


class _FakeSubprocess:
    """herdr の起動（subprocess.run）を差し替え，実際のペインを触らない．

    herdr.py が例外処理で参照する TimeoutExpired だけ本物から借りる．
    `agent get` には状態不明（JSON を返さない既定値）だが，
    agent 名を herdr 側に訊く場合のテストでは _agent_json を渡す．
    """

    TimeoutExpired = subprocess.TimeoutExpired

    def __init__(self, returncode: int = 0, stderr: str = "", agent_json: str = "") -> None:
        self.returncode = returncode
        self.stderr = stderr
        self.agent_json = agent_json
        self.calls: list[list[str]] = []

    def run(self, args, **kwargs):  # type: ignore[no-untyped-def]
        self.calls.append(list(args))
        if len(args) > 2 and args[1] == "agent" and args[2] == "get":
            if not self.agent_json:
                return _FakeProc(1)
            return _FakeProc(0, stdout=self.agent_json)
        return _FakeProc(self.returncode, stderr=self.stderr)


def _install(monkey_state: dict, **kwargs) -> _FakeSubprocess:
    """差し替えた実体を monkey_state に入れて，テストごとに戻せるようにする．"""
    fake = _FakeSubprocess(**kwargs)
    monkey_state["subprocess"] = herdr.subprocess
    monkey_state["send_keys"] = herdr.send_keys
    monkey_state["send_text"] = herdr.send_text
    herdr.subprocess = fake  # type: ignore[assignment]
    monkey_state["keys"] = []
    monkey_state["texts"] = []
    herdr.send_keys = lambda pane_id, *keys: (monkey_state["keys"].append(keys) or True)  # type: ignore[assignment]
    herdr.send_text = lambda pane_id, text, timeout_sec=10.0: (  # type: ignore[assignment]
        monkey_state["texts"].append(text) or True
    )
    return fake


def _restore(monkey_state: dict) -> None:
    herdr.subprocess = monkey_state["subprocess"]  # type: ignore[assignment]
    herdr.send_keys = monkey_state["send_keys"]  # type: ignore[assignment]
    herdr.send_text = monkey_state["send_text"]  # type: ignore[assignment]


def test_claude_uses_send_text_and_slash_uses_queue_submit() -> None:
    """claude は send-text で入力し，slash だけ ctrl+x enter，それ以外は alt+enter．"""
    state: dict = {}
    fake = _install(state)
    try:
        # claude の slash コマンド → send-text + chat:queueSubmit（ctrl+x → enter）
        ok, reason = herdr.prompt_agent("pane-1", "/cost", "claude")
        assert ok is True, reason
        assert state["texts"] == ["/cost"]
        assert state["keys"] == [("ctrl+x", "enter")]
        # agent prompt（貼り付け）は使わない
        assert not [c for c in fake.calls if c[1:3] == ["agent", "prompt"]]

        # 先頭に空白があっても slash とみなす
        ok, reason = herdr.prompt_agent("pane-1", "   /compact", "claude")
        assert ok is True, reason
        assert state["keys"][-1] == ("ctrl+x", "enter")

        # claude の普通の文 → send-text + alt+enter
        ok, reason = herdr.prompt_agent("pane-1", "こんにちは", "claude")
        assert ok is True, reason
        assert state["texts"][-1] == "こんにちは"
        assert state["keys"][-1] == ("alt+enter",)

        # スラッシュが途中にあるだけの文も普通の文として扱う
        ok, reason = herdr.prompt_agent("pane-1", "a / b", "claude")
        assert ok is True, reason
        assert state["keys"][-1] == ("alt+enter",)
    finally:
        _restore(state)


def test_pi_and_unknown_agent_keep_agent_prompt() -> None:
    """pi と agent 不明は従来どおり agent prompt + alt+enter（変更しない）．"""
    state: dict = {}
    fake = _install(state)
    try:
        ok, reason = herdr.prompt_agent("pane-1", "/x", "pi")
        assert ok is True, reason
        assert [c[1:3] for c in fake.calls if c[1] == "agent"][-1] == ["agent", "prompt"]
        assert state["keys"][-1] == ("alt+enter",)
        assert state["texts"] == []  # pi は send-text を使わない

        # agent が分からないとき（None）も agent prompt のまま
        ok, reason = herdr.prompt_agent("pane-1", "/x")
        assert ok is True, reason
        assert state["keys"][-1] == ("alt+enter",)
        assert state["texts"] == []
    finally:
        _restore(state)


def test_unknown_agent_asks_herdr_who_it_is() -> None:
    """agent が不明でも herdr に聞けば claude として扱える（slash を壊さない）．"""
    state: dict = {}
    _install(state, agent_json=json.dumps({"result": {"agent": {"agent": "claude"}}}))
    try:
        ok, reason = herdr.prompt_agent("pane-1", "/cost")
        assert ok is True, reason
        assert state["texts"] == ["/cost"]
        assert state["keys"] == [("ctrl+x", "enter")]
    finally:
        _restore(state)


def test_blocked_agent_is_not_sent() -> None:
    """blocked（質問・承認中）には送らない（pane send-text にガードが無いため）．"""
    state: dict = {}
    _install(state, agent_json=json.dumps({"result": {"agent": {"agent": "claude", "agent_status": "blocked"}}}))
    try:
        ok, reason = herdr.prompt_agent("pane-1", "/cost", "claude")
        assert ok is False
        assert reason == "blocked"
        assert state["texts"] == []
        assert state["keys"] == []
    finally:
        _restore(state)


def test_stalled_prompt_uses_same_keys() -> None:
    """agent_prompt_stalled のとき（agent prompt 側の2か所目）も同じ確定キーを使う．"""
    state: dict = {}
    _install(state, returncode=1, stderr="agent_prompt_stalled: 入力が確定していません")
    try:
        ok, reason = herdr.prompt_agent("pane-1", "/cost", "pi")
        assert ok is True, reason
        assert state["keys"] == [("alt+enter",)]

        state["keys"].clear()
        ok, reason = herdr.prompt_agent("pane-1", "こんにちは", "pi")
        assert ok is True, reason
        assert state["keys"] == [("alt+enter",)]
    finally:
        _restore(state)


def main() -> None:
    test_claude_uses_send_text_and_slash_uses_queue_submit()
    test_pi_and_unknown_agent_keep_agent_prompt()
    test_unknown_agent_asks_herdr_who_it_is()
    test_blocked_agent_is_not_sent()
    test_stalled_prompt_uses_same_keys()
    print("test_herdr_submit: すべて通りました")


if __name__ == "__main__":
    main()
