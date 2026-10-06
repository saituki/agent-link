"""cc のコマンド承認（PermissionRequest フック）の登録と回答（05 §3.3）．

方針:
- **PC と iPhone の両方に同時に出し，先に答えた方を採用する**（質問と同じ）．
  フックは何も出力せずすぐ終了コード 0 で終わるため，PC の端末には普段どおり
  承認画面が出たままになる（フックで待ってはいけない）．
- iPhone で答えたら，herdr で番号キーを送って PC の承認画面を確定させる．
  キーは実機（cc 2.1.280 / --permission-mode acceptEdits）で観察して決めた
  **推測で作らない**（05 §4 の確認作業で画面を読んだ）．
- 送る前に画面を読み，「承認画面であって，登録したのと同じコマンドがまだ
  出ている」ことを確かめる（別の画面に誤って答えない・05 §3.3）．
- PC で先に答えた場合（ペインが blocked でなくなった）は
  「PC で回答済み」にする（質問と同じ判定）．

観察した承認画面（そのまま）:

    Bash command
    Tip: auto mode handles these prompts for you — choose
    "switch to auto mode" below

      rm -v /tmp/sai05-rm-27tI5f/victim2.txt
      Delete the confirmed target file

    Do you want to proceed?
    ❯ 1. Yes
      2. Yes, and always allow access to
         /tmp/sai05-rm-27tI5f from this project
      3. Yes, and switch to auto mode · auto mode handles
         these prompts for you
      4. No

    Esc to cancel · Tab to amend

- 決定キーは **番号キー**（`1` / `4` など）．送ると即座に画面が閉じる．
- 2 番は permission_suggestions があるときだけ出る（＝「今後も許可」）．
- 拒否（4. No）を選ぶと cc は「Interrupted · What should Claude do instead?」と
  出し，通常の入力欄に理由を書ける（送信は alt+enter・keybindings.json より）．
"""

from __future__ import annotations

import re
import time

from sai_web import herdr, store

# 画面の何行まで読むか
_SCREEN_LINES = 60
# キー送り後の待ち
_KEY_WAIT = 0.4
_SCREEN_TRIES = 6

# 承認画面の目印（空白を落として比較する）
_APPROVAL_MARK = "Doyouwanttoproceed"

# 拒否のときに走る選択肢のラベル（画面の先頭部分）
_NO_LABEL = "no"


def _norm(text: str) -> str:
    """折り返しで文が切れないように，空白を全部落として比べる用．"""
    return re.sub(r"\s+", "", text or "")


def summary_of(tool_name: str, tool_input: dict) -> str:
    """カードに出す1行（例: `Bash: rm -rf build/`・05 §1 の3番）．"""
    tool_input = tool_input or {}
    body = ""
    for key in ("command", "file_path", "path", "pattern", "query"):
        value = tool_input.get(key)
        if isinstance(value, str) and value.strip():
            body = value.strip().split("\n")[0]
            break
    if not body:
        body = str(tool_input.get("description") or "").strip()
    body = re.sub(r"\s+", " ", body)
    if len(body) > 120:
        body = body[:120] + "…"
    return f"{tool_name}: {body}" if body else tool_name


def match_text_of(tool_name: str, tool_input: dict) -> str:
    """「その承認がまだ画面に出ているか」を確かめるための文字（空白なしで比較）．"""
    tool_input = tool_input or {}
    for key in ("command", "file_path", "path"):
        value = tool_input.get(key)
        if isinstance(value, str) and value.strip():
            # パスは画面で省略されることがあるため，ファイル名だけを残すことも考慮
            return value.strip().split("\n")[0]
    return tool_name


def register(
    session_id: str,
    pane_id: str,
    tool_use_id: str,
    tool_name: str,
    tool_input: dict,
    permission_suggestions: list,
) -> dict:
    """フックからの承認登録（重複は tool_use_id で防ぐ）とプッシュ通知（05 §1 の3番）．"""
    a = store.save_approval(
        session_id=session_id,
        pane_id=pane_id,
        tool_use_id=tool_use_id or "",
        tool_name=tool_name,
        tool_input=tool_input or {},
        permission_suggestions=permission_suggestions or [],
        match_text=match_text_of(tool_name, tool_input or {}),
    )
    _notify_if_run(session_id, tool_name)
    return a


def _notify_if_run(session_id: str, tool_name: str) -> None:
    """そのセッションが sai-link の実行なら「承認を求めています」を通知する．

    手動で起動したエージェントには鳴らさない（00 §5）．
    """
    from urllib.parse import quote

    from sai_web import notify, runs, watch

    try:
        for run in runs.load_runs():
            if session_id in runs.run_session_ids(run):
                project = quote(str(run.get("project_name") or ""), safe="")
                url = f"/#/run/{project}/{run.get('run_id', '')}"
                # 見出しはプロジェクト名（無ければ sai・計画書 A の U05）
                notify.broadcast(
                    watch.notify_heading(run), f"承認を求めています: {tool_name}", url
                )
                return
    except Exception:
        # 通知の失敗で登録は止めない
        return


def refresh(session_id: str, pane_id: str | None) -> None:
    """PC で先に答えられていないか確かめる（質問と同じ・画面を正にする）．"""
    if not pane_id:
        return
    # ペインが消えた＝agent は終了した。答えの届け先が無いので片付ける
    if herdr.pane_gone(pane_id):
        store.expire_open_for_pane("approvals", session_id, pane_id)
        return
    # 画面を読めれば画面を正にする（blocked 判定は当てにならない場合がある）
    screen = herdr.read_pane(pane_id, _SCREEN_LINES)
    if screen is not None:
        if _is_approval_screen(screen):
            return  # 承認画面はそのまま
        store.mark_approval_resolved_elsewhere(session_id)
        return
    state = herdr.get_agent(pane_id)
    if state is None:
        return
    if state.get("agent_status") != "blocked":
        store.mark_approval_resolved_elsewhere(session_id)


def _read_screen(pane_id: str) -> str:
    return herdr.read_pane(pane_id, _SCREEN_LINES) or ""


def _wait_screen(pane_id: str, predicate) -> str | None:
    for _ in range(_SCREEN_TRIES):
        screen = _read_screen(pane_id)
        if predicate(screen):
            return screen
        time.sleep(_KEY_WAIT)
    return None


def _is_approval_screen(screen: str) -> bool:
    return _APPROVAL_MARK in _norm(screen)


def _no_option_number(screen: str) -> int | None:
    """「4. No」のように，拒否の番号を画面から数える．見つからなければ None．"""
    marker = _norm(_APPROVAL_MARK)
    normalized_index = _norm(screen).find(marker)
    if normalized_index < 0:
        return None
    # 正規化すると位置が変わるので，元の画面で「Do you want to proceed?」以降を対象にする
    pos = screen.find("Do you want to proceed?")
    region = screen[pos:] if pos >= 0 else screen
    for line in region.splitlines():
        m = re.match(r"\s*(\d+)\.\s*(\S+)", line)
        if m and m.group(2).lower().rstrip(",.") == _NO_LABEL:
            return int(m.group(1))
    return None


def answer(aid: str, decision: str, reason: str = "") -> tuple[bool, str]:
    """承認に答える（成功かどうか, 理由）を返す．

    decision: "allow"（許可） / "always"（今後も許可） / "deny"（拒否）
    """
    a = store.get_approval(aid)
    if a is None:
        return (False, "承認が見つかりません")
    if a["state"] != "open":
        return (False, "この承認は回答済みです")
    if decision not in ("allow", "always", "deny"):
        return (False, "選択が不正です")
    if decision == "always" and not a.get("permission_suggestions"):
        return (False, "この承認に「今後も許可」の選択肢はありません")

    pane_id = a.get("pane_id") or ""
    if not pane_id:
        return (False, "端末に繋がりませんでした")

    # まだ同じ承認画面が出ているかを画面から確かめる（05 §3.3）．
    # blocked 判定は当てにならない場合がある（質問画面中でも done になる実例あり）
    # ため，画面を読めれば画面を正とする．
    screen = _read_screen(pane_id)
    # 解決済みの書き換えは refresh（ポーリング）の仕事．回答の失敗では
    # 状態を変えない（失敗してもカードが消えずに，もう一度試せるため）。
    if not screen:
        state = herdr.get_agent(pane_id)
        if state is None:
            return (False, "端末に繋がりませんでした")
        if state.get("agent_status") != "blocked":
            return (False, "端末側で回答済みです")
    elif not _is_approval_screen(screen):
        return (False, "端末側で回答済みです")
    if not _matches_screen(screen, a.get("match_text") or ""):
        return (False, "承認画面が見つかりません。PC で答えてください")

    if decision == "deny":
        number = _no_option_number(screen)
        if number is None:
            return (False, "承認画面が見つかりません。PC で答えてください")
        key = str(number)
    elif decision == "always":
        # 2 番 = 「Yes, and always allow …」（permission_suggestions があるときだけ出る）
        key = "2"
    else:
        key = "1"

    if not herdr.send_keys(pane_id, key):
        return (False, "送信できなかった")
    # 画面が閉じたか確かめる（閉じていなければ途中まで選んだことにならない）
    closed = _wait_screen(pane_id, lambda s: not _is_approval_screen(s))
    if closed is None:
        return (False, "送信できませんでした。PC で答えてください")

    # 拒否に理由を書く（理由欄は通常の入力欄・確定は alt+enter・観察済み）
    if decision == "deny" and (reason or "").strip():
        if not herdr.send_text(pane_id, reason.strip()):
            store.mark_approval_answered(aid, {"decision": decision, "reason": reason})
            return (True, "")
        time.sleep(_KEY_WAIT)
        herdr.send_keys(pane_id, "alt+enter")

    store.mark_approval_answered(
        aid, {"decision": decision, "reason": (reason or "").strip()}
    )
    return (True, "")


def _matches_screen(screen: str, match_text: str) -> bool:
    """画面に出ているのが登録した承認と同じかを確かめる．

    画面は長いパスを短縮表示することがあるため，次のいずれかが
    画面にあれば同じものとみなす（どれも無ければ答えない）:
    - 登録時の文字列そのもの（コマンド全体など）
    - そのファイル名（パスの短縮表示に対応）
    """
    needle = _norm(match_text)
    if not needle:
        return True  # 照合材料が無ければ画面の目印だけで判定（登録時に tool 名）
    if needle in _norm(screen):
        return True
    base = needle.rstrip("/").rsplit("/", 1)[-1]
    return bool(base) and base in _norm(screen)
