"""herdr コマンドの呼び出し．

必要なのは2つだけ: 一覧を取る・1つの状態を取る．
herdr は JSON を標準出力に返す．動いていない時は終了コード1で
標準エラーに JSON が来るが，その時は「動作中なし」として扱い，
サーバーは落とさない．
"""

from __future__ import annotations

import json
import subprocess
import time

from sai_web.config import settings

# 一覧の連続アクセスで毎回プロセスを起動しないよう，短く覚えておく秒数．
# 長く覚えると「質問待ち」の表示が遅れるので 1〜2 秒だけにする．
_CACHE_SEC = 1.5

_cached_list: tuple[float, list[dict]] | None = None
_cached_panes: tuple[float, list[dict]] | None = None


def _run_herdr(*args: str, timeout_sec: float = 5.0) -> dict | None:
    """herdr を起動し，標準出力の JSON を辞書で返す．

    herdr が動いていない・コマンドが無い場合は None を返す．
    （呼び出し側は「動作中セッションなし」として扱う）
    """
    try:
        proc = subprocess.run(
            [settings.herdr_bin, *args],
            capture_output=True,
            text=True,
            timeout=timeout_sec,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return None


def list_agents() -> list[dict]:
    """動作中のエージェント一覧を返す．失敗時は空リスト．"""
    global _cached_list
    now = time.monotonic()
    if _cached_list is not None and now - _cached_list[0] < _CACHE_SEC:
        return _cached_list[1]
    data = _run_herdr("agent", "list")
    agents: list[dict] = []
    if data:
        try:
            agents = data["result"]["agents"]
        except (KeyError, TypeError):
            agents = []
    _cached_list = (now, agents)
    return agents


def list_panes() -> list[dict]:
    """ペイン一覧（ペイン名 label 付き）を返す．失敗時は空リスト．

    `agent list` には label が付かないことがあるため，
    実行とペインの対応づけ（00 §3.7: label == herdr_pane_label）には
    こちらを使う．同梱の短いキャッシュは list_agents と同じ秒数．
    """
    global _cached_panes
    now = time.monotonic()
    if _cached_panes is not None and now - _cached_panes[0] < _CACHE_SEC:
        return _cached_panes[1]
    data = _run_herdr("pane", "list")
    panes: list[dict] = []
    if data:
        try:
            panes = data["result"]["panes"]
        except (KeyError, TypeError):
            panes = []
    _cached_panes = (now, panes)
    return panes


def pane_gone(pane_id: str) -> bool:
    """そのペインが herdr から消えたか（＝その agent は終了した）．

    sai-link の agent は sai-finalize でも強制終了でもペインが消える。
    herdr に繋がらない（一覧が空）ときは「分からない」ので False。
    """
    # ponytail: ペインが1つも無い状態は herdr 停止と区別できないので消えた扱いにしない
    panes = list_panes()
    return bool(panes) and all(p.get("pane_id") != pane_id for p in panes)


def get_agent(pane_id: str) -> dict | None:
    """1つの pane の状態を返す．取れなければ None．"""
    data = _run_herdr("agent", "get", pane_id)
    if not data:
        return None
    try:
        result = data["result"]
        # get は agent を一段包んで返す（list と形が違うためここで剥がす）
        if isinstance(result, dict) and "agent" in result and isinstance(result["agent"], dict):
            return result["agent"]
        return result
    except (KeyError, TypeError):
        return None


def _submit_keys(text: str, agent: str | None) -> tuple[str, ...]:
    """確定に使うキーを決める（計画書 §2・実機で確認したキーの並び）．

    Claude Code で「/」で始まる文（slash コマンド）を送ると，入力した時点で
    補完メニューが出る．補完候補が選ばれている間は chat:submit（alt+enter）が
    効かない（実機でも同じ．補完メニューの中では alt+enter を送っても送信されず
    入力欄に残ることを確認した）ため，slash のときだけ chat:queueSubmit の
    既定キー ctrl+x → enter で確定する．pi や普通の文は alt+enter．

    実機（cc 2.1.283）で試した結果:
    - slash: `pane send-text` + ctrl+x + enter …… コマンドとして実行された
    - slash: `pane send-text` + tab + alt+enter …… 実行された（補完が確定する）
    - slash: `pane send-text` + esc + alt+enter …… 実行された（メニューを閉じてから）
    - slash: `agent prompt` + alt+enter …… 1 回目は実行されたが 2 回目以降は
      文字として送られた（後述の残存改行のため．ctrl+x + enter でも同じ）．
    """
    if agent == "claude" and text.lstrip().startswith("/"):
        return ("ctrl+x", "enter")
    return ("alt+enter",)


def prompt_agent(
    pane_id: str, text: str, agent: str | None = None, timeout_sec: float = 15.0
) -> tuple[bool, str]:
    """テキストを送って確定する．(成功かどうか, 理由) を返す．

    herdr は文字を入力欄に置くだけで確定しない．
    確定キーは alt+enter（~/.claude/keybindings.json で enter は chat:newline，
    alt+enter が chat:submit．pi も alt+enter）．
    Enter や ctrl+m を送ると改行が入るだけで送信されないため使わない．
    keybindings.json を変えた場合はここも変える．

    agent が "claude" のときは `agent prompt`（貼り付け）ではなく
    `pane send-text`（1 文字ずつ入力）で入れる．理由は次のとおり（実機で確認）:

    `agent prompt` は「文字列を貼り付け → 少し待って Enter」を送る．
    この環境の cc は Enter が chat:newline なので，貼り付けたあとに改行が
    1 個入力欄に残る．確定キーはその改行を消費しないため，入力欄は
    「改行 + 文」のまま送信される．すると 2 回目以降の slash コマンドは
    `/context` ではなく `\n/context` として届き，cc はコマンドとして解釈せず
    ただの文として LLM に送ってしまう（= 依頼の不具合）．
    1 回目は入力欄が空なので成功することがあり，「再現しない」ように見える．

    `pane send-text` は末尾に Enter を付けないので残らない．
    220 文字の日本語・引用符・括弧・改行を含む文も 1 文字も落とさず届くこと，
    作業中でも届くこと，cc が自動で括弧を補完しないことを実機で確認した．

    ただし agent が "claude" で「/」で始まる文のときは補完メニューが
    出るため alt+enter が効かない．そのときだけ確定キーを差し替える
    （詳しくは _submit_keys）．
    """
    keys = _submit_keys(text, agent)
    state = get_agent(pane_id) or {}
    if state.get("agent_status") == "blocked":
        # `agent prompt` なら herdr が agent_blocked を返すが，
        # `pane send-text` にはそのガードが無いので先に確かめる
        return (False, "blocked")
    if agent is None:
        # agent が分からないとき（pane 側要素に agent が無い）は herdr に聞く
        # （cc を pi と取り違えて壊さないため）
        agent = state.get("agent")
        keys = _submit_keys(text, agent)
    if agent == "claude":
        if not send_text(pane_id, text, timeout_sec=timeout_sec):
            return (False, "herdr に繋がりませんでした")
        send_keys(pane_id, *keys)
        return (True, "")
    try:
        proc = subprocess.run(
            [settings.herdr_bin, "agent", "prompt", pane_id, text],
            capture_output=True,
            text=True,
            timeout=timeout_sec,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return (False, "herdr に繋がりませんでした")
    if proc.returncode == 0:
        # ponytail: 確定済みかを画面から判定せず毎回確定キーを送る．
        # 二重確定が問題になったら入力欄の行を読んで分岐する．
        send_keys(pane_id, *keys)
        return (True, "")
    err = (proc.stderr or proc.stdout or "").strip()
    # 確定されずに止まった場合も同じ確定キーで確定を試みる
    if "agent_prompt_stalled" in err:
        if send_keys(pane_id, *keys):
            return (True, "")
        return (False, "送信できなかった")
    if "agent_blocked" in err:
        return (False, "blocked")
    return (False, err[-200:] if err else "送信できなかった")


def send_keys(pane_id: str, *keys: str, timeout_sec: float = 10.0) -> bool:
    """キーを送る（down / enter / 数字 など）．成功すれば True．"""
    try:
        proc = subprocess.run(
            [settings.herdr_bin, "agent", "send-keys", pane_id, *keys],
            capture_output=True,
            text=True,
            timeout=timeout_sec,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False
    return proc.returncode == 0


def read_pane(pane_id: str, lines: int = 60) -> str | None:
    """画面に見えている文字を読む（質問・承認画面の確認用・05 §3.2）．

    herdr pane read --format text は文字列をそのまま返す（JSON ではない）．
    動かなければ None（呼び出し側は「読めない」扱いにする）．
    """
    try:
        proc = subprocess.run(
            [
                settings.herdr_bin,
                "pane",
                "read",
                pane_id,
                "--source",
                "visible",
                "--lines",
                str(lines),
                "--format",
                "text",
            ],
            capture_output=True,
            text=True,
            timeout=5.0,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    out = proc.stdout or ""
    # 保険: 万一 JSON で返ってきたら文字列部分だけ取り出す
    if out.lstrip().startswith("{"):
        try:
            data = json.loads(out)
            inner = data.get("result") if isinstance(data, dict) else None
            if isinstance(inner, dict):
                for key in ("text", "output", "content"):
                    if isinstance(inner.get(key), str):
                        return inner[key]
        except json.JSONDecodeError:
            pass
    return out


def send_text(pane_id: str, text: str, timeout_sec: float = 10.0) -> bool:
    """文字列をそのままペインに入れる（質問の自由入力欄の入力用・05 §3.2）．

    `agent prompt` は質問・承認画面の最中（blocked）に使えない
    （agent_blocked エラーになる・実機で確認済み）ため，
    `pane send-text` を使う．確定は別途キー（enter / alt+enter）で行う．
    """
    try:
        proc = subprocess.run(
            [settings.herdr_bin, "pane", "send-text", pane_id, text],
            capture_output=True,
            text=True,
            timeout=timeout_sec,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False
    return proc.returncode == 0
