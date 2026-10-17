"""会話記録ファイル（.jsonl）の読み取り・変換．

読み込むのは Run の `transcript_path` 1件だけ（どのファイルを見るかは
runs.py が agent-link の書き出しファイルから決める．）。

iPhone に出す形は4種類（user / agent / tool / event）にする．
event は XML 風タグの特殊メッセージ（<task-notification> など）のカード。
会話の中身は記録ファイルからのみ作る。
pane の画面読み取りは使わない．
（隣の枠線が混ざる・右端が切れる・流れた行は残らないため）

方針:
- ファイルは丸ごと読まない（数万行になる）．会話は行単位で読む．
- 差分取得はファイル内のバイト位置（cursor）を目印にする．
- エージェントの種類ごとにクラスは作らない．読む関数1つで足りる．
"""

from __future__ import annotations

import json
import re
import shlex
from pathlib import Path

# 作業の見出し（detail）に出す文字数．iPhone で閉じた状態でも重くならないよう切る．
_DETAIL_LIMIT = 4000
# カーソルの区切り文字（後ろはバイト位置）
_CURSOR_SEP = ":"

# 行の先頭がタグで始まるか（本文の途中にある < と誤判定しないよう，行単位で見る）
_TAG_START_RE = re.compile(r"^<([a-z][a-z0-9-]*)>")
# 画面に出さないタグ（人が書いていない前置き・注意書きのため）
_HIDE_TAGS = frozenset({"system-reminder", "local-command-caveat"})
# コマンド実行の記録．行の順序が入れ替わることがあるので1枚のカードにまとめる
_COMMAND_TAGS = ("command-name", "command-message", "command-args")
# 直前の項目の時刻（transcript のパス → 時刻）．
# 行に timestamp が無い場合に「直前の項目の時刻」で補うための覚え書き
_last_time_cache: dict[str, str] = {}


# --- 会話の変換 ---


def _mcp_short_name(name: str) -> str | None:
    """mcp__<サーバ>__<ツール> から <ツール> だけを取り出す（MCP の名前でなければ None）．"""
    parts = (name or "").split("__", 2)
    if len(parts) == 3 and parts[0] == "mcp" and parts[1] and parts[2]:
        return parts[2]
    return None


def _readable(value: object, indent: int = 0) -> str:
    """辞書・配列を「キー: 値」の行にする（字下げでネストを表す）．JSON の括弧と引用符は出さない．"""
    pad = "  " * indent
    if isinstance(value, dict):
        if not value:
            return f"{pad}(none)"
        lines: list[str] = []
        for key, val in value.items():
            if _is_block(val):
                lines.append(f"{pad}{key}:")
                lines.append(_readable(val, indent + 1))
            else:
                lines.append(f"{pad}{key}: {_scalar(val)}")
        return "\n".join(lines)
    if isinstance(value, list):
        if not value:
            return f"{pad}(none)"
        lines = []
        for val in value:
            if _is_block(val):
                lines.append(f"{pad}-")
                lines.append(_readable(val, indent + 1))
            else:
                lines.append(f"{pad}- {_scalar(val)}")
        return "\n".join(lines)
    if isinstance(value, str):
        return "\n".join(f"{pad}{line}" for line in value.split("\n"))
    return f"{pad}{_scalar(value)}"


def _is_block(val: object) -> bool:
    """値を次の行から字下げして書くべきか（辞書・配列・改行を含む文字列）．"""
    return isinstance(val, (dict, list)) or (isinstance(val, str) and "\n" in val)


def _scalar(val: object) -> str:
    """1 行に収まる値の文字．文字列は引用符なし，数・真偽・null は JSON の書き方．"""
    return val if isinstance(val, str) else json.dumps(val, ensure_ascii=False)


def _mcp_result_text(text: str) -> str:
    """戻り値の本文が JSON（辞書か配列）なら _readable に通し，そうでなければそのまま返す．"""
    if text.strip()[:1] in ("{", "["):
        try:
            return _readable(json.loads(text))
        except ValueError:
            pass
    return text


def _mcp_use_detail(name: str, args: dict) -> str:
    """tool_use の detail．MCP は引数を読みやすい文に，他は今までの JSON．"""
    if not args:
        return ""
    if _mcp_short_name(name) is None:
        return json.dumps(args, ensure_ascii=False)
    return "Arguments\n" + _readable(args)


def _join_result(target: dict, plain: str) -> None:
    """tool_result の本文 plain を tool 項目 target の detail へ足す（cc と pi で共通）．"""
    if _mcp_short_name(target.get("tool") or "") is not None or target.get("cli"):
        plain = "Result\n" + _mcp_result_text(plain)
    prev = target.get("detail") or ""
    target["detail"] = f"{prev}\n---\n{plain}" if prev else plain


# --- agent-link の CLI の呼び出し（Bash の道具） ---
# Bash の command が agent-link の CLI の呼び出しかを調べ、今の MCP の道具と同じ緑の
# 見出しで出す。見出しは「CLI の名前 + 命令の語（1〜2 語）」。

# agent-link の CLI の名前（実行ファイル名。パスが付いていてもファイル名で見る）
_CLI_NAMES = frozenset({"agent-link", "agent-link-web"})
# 下位の命令を持つ語（この語が 1 語目のときだけ 2 語目も見出しに付ける）。
# 表に無い新しい命令は 1 語目までの見出しになり、壊れない。
_CLI_SUBWORDS = {
    "agent-link": frozenset(
        {"project", "daemon", "card", "run", "doc", "task", "def", "session", "usage"}
    ),
}
# 命令の語として認める形（小文字で始まり、小文字・数字・ハイフンだけ）
_CLI_WORD_RE = re.compile(r"^[a-z][a-z0-9-]*$")
# 先頭で飛ばす `NAME=値`（env の代入）
_ASSIGN_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")


def _sai_cli_call(command: str) -> tuple[str, str, str] | None:
    """Bash で実行した agent-link の CLI の呼び出しかを決める。

    返すのは `(CLI の名前, 見出し, 引数の文)`。agent-link の CLI でなければ None。
    引数の文は、命令の語より後ろの 1 行目の残りと 2 行目以降をつなげたもの
    （ヒアドキュメントの本文などもそのまま残す）。
    """
    if not isinstance(command, str) or not command.strip():
        return None
    lines = command.split("\n")
    try:
        words = shlex.split(lines[0])
    except ValueError:
        words = lines[0].split()
    if not words:
        return None
    # 先頭の `NAME=値` と `env` とその引数（-u NAME・-i・NAME=値）を飛ばす。
    i = 0
    while i < len(words):
        word = words[i]
        if _ASSIGN_RE.match(word):
            i += 1
            continue
        if word == "env":
            i += 1
            while i < len(words):
                arg = words[i]
                if arg in ("-i", "--ignore-environment"):
                    i += 1
                elif arg in ("-u", "--unset") and i + 1 < len(words):
                    i += 2
                elif _ASSIGN_RE.match(arg):
                    i += 1
                else:
                    break
            continue
        break
    if i >= len(words):
        return None
    cli = words[i].rsplit("/", 1)[-1]
    if cli not in _CLI_NAMES:
        return None
    rest = words[i + 1 :]
    used = 0
    parts = [cli]
    if rest and _CLI_WORD_RE.match(rest[0]):
        parts.append(rest[0])
        used = 1
        if (
            len(rest) > 1
            and rest[0] in _CLI_SUBWORDS.get(cli, frozenset())
            and _CLI_WORD_RE.match(rest[1])
        ):
            parts.append(rest[1])
            used = 2
    label = " ".join(parts)
    tail = " ".join(rest[used:])
    extra = "\n".join(lines[1:])
    if extra:
        args_text = f"{tail}\n{extra}" if tail else extra
    else:
        args_text = tail
    return cli, label, args_text


def _tool_item(item_id: str, timestamp: str, name: str, args: dict) -> dict:
    """tool の 1 項目を作る（cc と pi で同じ形・agent-link の CLI を見分ける）。"""
    item = {
        "id": item_id,
        "kind": "tool",
        "time": timestamp,
        "label": _tool_label(name, args),
        "tool": name,
        "detail": _mcp_use_detail(name, args),
        "ok": True,
    }
    if (name or "").lower() == "bash" and isinstance(args, dict):
        command = args.get("command")
        call = _sai_cli_call(command) if isinstance(command, str) else None
        if call is not None:
            cli, label, args_text = call
            item["label"] = label
            item["cli"] = cli
            item["detail"] = f"Arguments\n{args_text}" if args_text else ""
    return item


def _tool_label(name: str, args: dict) -> str:
    """ツール名と引数から，日本語の見出し1行を作る．凝った仕組みは作らない．"""
    short = _mcp_short_name(name)
    if short is not None:
        return short
    lower = (name or "").lower()

    def basename(p: object) -> str:
        if not isinstance(p, str) or not p:
            return ""
        return p.rsplit("/", 1)[-1]

    # ファイル系は引数のそれらしい場所からファイル名だけ取る
    file_arg = ""
    if isinstance(args, dict):
        for key in ("path", "file_path", "file", "filename", "pathname"):
            if isinstance(args.get(key), str) and args[key]:
                file_arg = basename(args[key])
                break
    if lower in ("read",):
        return f"Read {file_arg}" if file_arg else "Read a file"
    if lower in ("edit",):
        return f"Edited {file_arg}" if file_arg else "Edited a file"
    if lower in ("write",):
        return f"Wrote {file_arg}" if file_arg else "Wrote a file"
    if lower in ("bash",):
        cmd = ""
        if isinstance(args, dict) and isinstance(args.get("command"), str):
            cmd = args["command"].strip().split("\n")[0][:30]
        return f"Ran {cmd}" if cmd else "Ran a command"
    if lower in ("grep", "glob"):
        pattern = ""
        if isinstance(args, dict):
            for key in ("pattern", "path", "query"):
                if isinstance(args.get(key), str) and args[key]:
                    pattern = str(args[key])[:30]
                    break
        return f"Searched for \"{pattern}\"" if pattern else "Searched"
    if lower in ("task",):
        return "Delegated to another agent"
    if lower in ("askuserquestion", "ask_user", "ask_user_phone"):
        # 閉じた状態のツール項目は「⚙ 質問」の1行（⚙ は app.js が付ける）。
        # 古い名前 ask_user_phone も残す（古い会話の記録を同じ見出しで出すため）。
        return "Question"
    return f"Ran {name}"


def _text_of(blocks: object) -> str:
    """content 配列の中の text ブロックをつなげる．"""
    if isinstance(blocks, str):
        return blocks
    if not isinstance(blocks, list):
        return ""
    parts: list[str] = []
    for b in blocks:
        if isinstance(b, dict) and b.get("type") == "text" and isinstance(b.get("text"), str):
            parts.append(b["text"])
    return "\n".join(parts).strip()


# --- 特殊メッセージ（XML 風タグ）の判定と整形 ---


def _scan_blocks(text: str) -> list[tuple[str, str, str]]:
    """テキストを行単位で走査し，(種類, タグ名, 中身) の並びを作る．

    種類は "tag"（タグのブロック）か "text"（それ以外の行）。
    行の先頭が <タグ名> で始まるものだけをタグとして扱う
    （本文の途中の < を誤判定しないため）。
    ``` で囲まれた部分はタグと見なさない（コード例の誤判定防止）。
    """
    lines = text.split("\n")
    out: list[tuple[str, str, str]] = []
    in_fence = False
    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            out.append(("text", "", line))
            i += 1
            continue
        m = None if in_fence else _TAG_START_RE.match(stripped)
        if m is None:
            out.append(("text", "", line))
            i += 1
            continue
        name = m.group(1)
        close = f"</{name}>"
        # 同一行に閉じタグがある場合（<command-name>/x</command-name> など）
        if close in stripped:
            end = stripped.index(close)
            out.append(("tag", name, stripped[m.end():end]))
            rest = stripped[end + len(close):]
            if rest.strip():
                out.append(("text", "", rest))
            i += 1
            continue
        # 複数行にわたるブロックは，閉じタグの行までを中身にする
        content_lines: list[str] = []
        head = stripped[m.end():]
        if head.strip():
            content_lines.append(head)
        j = i + 1
        rest = ""
        while j < len(lines):
            if close in lines[j]:
                k = lines[j].index(close)
                if lines[j][:k].strip():
                    content_lines.append(lines[j][:k])
                rest = lines[j][k + len(close):]
                break
            content_lines.append(lines[j])
            j += 1
        closed = j < len(lines)
        out.append(("tag", name, "\n".join(content_lines)))
        if closed and rest.strip():
            out.append(("text", "", rest))
        i = j + 1 if closed else len(lines)
    return out


def _inner(content: str, tag: str) -> str:
    """タグで包まれた1個の値を取り出す（無ければ空文字）．"""
    m = re.search(f"<{tag}>([\\s\\S]*?)</{tag}>", content)
    return m.group(1).strip() if m else ""


def _duration_jp(ms: int) -> str:
    """ミリ秒を「1分20秒」のように直す．"""
    total = max(0, int(ms) // 1000)
    minutes, seconds = divmod(total, 60)
    return f"{minutes}m {seconds}s" if minutes else f"{seconds}s"


def _usage_line(content: str) -> str | None:
    """<usage> の中身を「トークン 12,345 · ツール 8 回 · 1分20秒」の1行にする．

    実物は <subagent_tokens>…</subagent_tokens> の形だが，
    JSON で来るものもあるので両方に対応する。取れなければ None（生の文字は出さない）。
    """

    def pick_int(*names: str) -> int | None:
        for n in names:
            m = re.search(f"<{n}>(\\d+)</{n}>", content)
            if m:
                return int(m.group(1))
        return None

    tokens = pick_int("subagent_tokens", "tokens", "token_count", "total_tokens")
    tools = pick_int("tool_uses", "tool_call_count", "tool_calls")
    dur_ms = pick_int("duration_ms", "elapsed_ms")
    if tokens is None and tools is None and dur_ms is None:
        # JSON 形式（{"tokens": …, "duration_ms": …}）の可能性
        try:
            data = json.loads(content)
        except json.JSONDecodeError:
            data = None
        if isinstance(data, dict):
            def jpick(*names: str) -> int | None:
                for n in names:
                    v = data.get(n)
                    if isinstance(v, (int, float)) and not isinstance(v, bool):
                        return int(v)
                return None

            tokens = jpick("subagent_tokens", "tokens", "token_count", "total_tokens")
            tools = jpick("tool_uses", "tool_call_count", "tool_calls")
            dur_ms = jpick("duration_ms", "elapsed_ms")
    parts: list[str] = []
    if tokens is not None:
        parts.append(f"Tokens {tokens:,}")
    if tools is not None:
        parts.append(f"Tool calls {tools}")
    if dur_ms is not None:
        parts.append(_duration_jp(dur_ms))
    if not parts:
        return None
    return " · ".join(parts)


def _new_event(
    event_type: str,
    title: str,
    status: str,
    summary: str = "",
    detail: str = "",
    fields: list | None = None,
) -> dict:
    """event カードを作る．status は ok / error / info。"""
    return {
        "event_type": event_type,
        "title": title,
        "status": status,
        "summary": summary,
        "fields": fields or [],
        "detail": detail,
    }


def _event_task_notification(content: str) -> dict:
    """<task-notification> を event カードにする．"""
    status = _inner(content, "status")
    summary = _inner(content, "summary")
    output_file = _inner(content, "output-file")
    if status == "completed":
        tone, title = "ok", "Background task completed"
    elif status in ("failed", "killed", "error", "cancelled", "timeout", "timed_out"):
        tone, title = "error", "Background task failed"
    else:
        tone = "info"
        title = f"Background task {status}" if status else "Background task"
    fields: list[list[str]] = []
    if status:
        fields.append(["Status", status])
    if output_file:
        fields.append(["Output file", output_file])
    # 補足: <event> の JSON を整形して入れ，無ければ <result> の中身を使う
    detail = ""
    raw_event = _inner(content, "event")
    if raw_event:
        try:
            detail = json.dumps(json.loads(raw_event), ensure_ascii=False, indent=2)
        except json.JSONDecodeError:
            detail = raw_event
    if not detail:
        detail = _inner(content, "result")
    # 中の <usage> は1行に直して先頭に添える（生のタグは出さない）
    usage = re.search(r"<usage>([\s\S]*?)</usage>", content)
    if usage:
        line = _usage_line(usage.group(1))
        if line:
            detail = f"{line}\n{detail}" if detail.strip() else line
    return _new_event("task_notification", title, tone, summary, detail, fields)


def _event_for_block(name: str, content: str) -> dict | None:
    """タグ1個から event カードを作る．表示しないタグは None．"""
    if name in _HIDE_TAGS:
        return None
    if name == "task-notification":
        return _event_task_notification(content)
    if name == "local-command-stdout":
        return _new_event("command_output", "Command output", "info", "", content.strip())
    if name == "persisted-output":
        # 中身は「Output too large …」の先頭だけdetailに入れる
        return _new_event("persisted_output", "Output saved", "info", "", content.strip()[:1000])
    # 上記以外のタグはタグ名をそのまま見出しにした info カード
    return _new_event(name, name, "info", "", content.strip())


def _command_line(cmd: dict[str, str]) -> str:
    """<command-*> から1行の説明（名前と引数）を作る．"""
    name = (cmd.get("command-name") or "").strip()
    message = (cmd.get("command-message") or "").strip()
    args = (cmd.get("command-args") or "").strip()
    base = name or message
    return " ".join(p for p in (base, args) if p)


def analyze_special(text: str) -> tuple[str, list[dict]]:
    """特殊メッセージ（XML 風タグ）を判定する．

    戻り値は (画面に残す本文, event カードの並び)。
    - 行の先頭がタグで始まるものだけをタグとして扱う。
    - <command-*> は何個あっても1枚のカードにまとめる。
    - system-reminder / local-command-caveat は表示しない。
    - <usage> は1行の文字に直して本文側へ回す。
    タグが1つも無ければ text をそのまま返す。
    """
    tokens = _scan_blocks(text)
    if not any(kind == "tag" for kind, _, _ in tokens):
        return (text, [])
    plain_parts: list[str] = []
    events: list[dict] = []
    cmd: dict[str, str] = {}
    cmd_pos = -1  # コマンドのカードを置く位置（並び順を保つための空け）
    for kind, name, body in tokens:
        if kind == "text":
            plain_parts.append(body)
            continue
        if name == "usage":
            line = _usage_line(body)
            if line:
                plain_parts.append(line)
            continue
        if name in _HIDE_TAGS:
            continue  # 表示しない
        if name in _COMMAND_TAGS:
            if cmd_pos < 0:
                cmd_pos = len(events)
                events.append(None)  # type: ignore[arg-type]  # 後で詰める
            cmd[name] = body
            continue
        ev = _event_for_block(name, body)
        if ev is not None:
            events.append(ev)
    if cmd:
        line = _command_line(cmd)
        events[cmd_pos] = _new_event(
            "command",
            f"Command {line}" if line else "Command",
            "info",
            "",
            line,
        )
    real_events = [e for e in events if e is not None]
    return ("\n".join(plain_parts).strip(), real_events)


def _append_text_items(text: str, base_id: str, timestamp: str, items: list[dict]) -> None:
    """user の文章1件を項目にする．タグがあれば event カードに分ける．"""
    plain, events = analyze_special(text)
    if plain:
        items.append({"id": base_id, "kind": "user", "time": timestamp, "text": plain})
    _extend_events(events, base_id, timestamp, items)


def _extend_events(events: list[dict], base_id: str, timestamp: str, items: list[dict]) -> None:
    """event カードを項目として足す．同じ行に複数出ても ID が重複しないようにする．"""
    for n, ev in enumerate(events):
        eid = f"{base_id}-ev{n}" if base_id else f"ev-{len(items)}"
        item = {"id": eid, "kind": "event", "time": timestamp}
        item.update(ev)
        items.append(item)


def _encode_cursor(path: Path, pos: int) -> str:
    """カーソル（どこまで読んだか）を作る．ファイル名とバイト位置の組み合わせ．"""
    return f"{path.name}{_CURSOR_SEP}{pos}"


def _decode_cursor(cursor: str | None, path: Path) -> int:
    """カーソルからバイト位置を取り出す．変なら最初から読む（0）．

    記録ファイルが別のファイルに変わった（pi の仮ファイル → 実ファイル等）ときも
    前のファイルの位置は使えないので最初から読む（同じ項目は画面側で重複を省く）。
    """
    if not cursor or _CURSOR_SEP not in cursor:
        return 0
    name, _, pos = cursor.rpartition(_CURSOR_SEP)
    if name != path.name:
        return 0
    try:
        return int(pos)
    except ValueError:
        return 0


def read_conversation(
    agent: str, path: Path, after: str | None = None
) -> tuple[list[dict], str]:
    """会話を読む．after（カーソル）より後ろの分だけ返す．

    戻り値は (items, next_cursor)．
    ファイルが大きいので行単位で読み，バイト位置で続きを管理する．
    """
    start = _decode_cursor(after, path)
    try:
        size = path.stat().st_size
    except OSError:
        return ([], _encode_cursor(path, 0))
    if start > size:
        start = 0
    items: list[dict] = []
    # 対応づけ待ちの tool_use（ID → 項目の位置）
    pending: dict[str, dict] = {}
    pos = start
    try:
        with open(path, encoding="utf-8") as f:
            f.seek(start)
            while True:
                line_start = f.tell()
                line = f.readline()
                if not line:
                    pos = f.tell()
                    break
                pos = f.tell()
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if agent == "claude":
                    _convert_claude_row(row, items, pending)
                else:
                    _convert_pi_row(row, items, pending)
                _ = line_start
    except OSError:
        return (items, _encode_cursor(path, start))
    # 見出しの中身が長すぎるものは切る（event カードの detail も含む）
    for item in items:
        if isinstance(item.get("detail"), str) and len(item["detail"]) > _DETAIL_LIMIT:
            item["detail"] = item["detail"][:_DETAIL_LIMIT]
    # 時刻は全項目で必須．行に timestamp が無ければ直前の項目の時刻を使う
    last_time = _last_time_cache.get(str(path), "")
    for item in items:
        t = item.get("time")
        if isinstance(t, str) and t:
            last_time = t
        elif last_time:
            item["time"] = last_time
    if last_time:
        _last_time_cache[str(path)] = last_time
    return (items, _encode_cursor(path, pos))


def _is_tool_error_cc(block: dict) -> bool:
    """cc の tool_result が失敗かどうか．"""
    if block.get("is_error") is True:
        return True
    content = block.get("content")
    if isinstance(content, str) and content.startswith("Exit code 1"):
        return True
    return False


def _append_queued_prompt(row: dict, items: list[dict]) -> None:
    """実行中に送った文（cc の待ち行列の文）を user の項目にする．task-notification は出さない．"""
    att = row.get("attachment")
    if row.get("isSidechain") is True or not isinstance(att, dict):
        return
    if att.get("type") != "queued_command" or att.get("commandMode") != "prompt":
        return
    prompt = att.get("prompt")
    if not isinstance(prompt, str) or not prompt.strip():
        return
    timestamp = row.get("timestamp") or att.get("timestamp") or ""
    _append_text_items(prompt, row.get("uuid") or "", timestamp, items)


def _convert_claude_row(row: dict, items: list[dict], pending: dict[str, dict]) -> None:
    """cc の1行を items に足す．表示しない行は捨てる．"""
    row_type = row.get("type")
    if row_type == "attachment":
        # 実行中に送った文は cc が待ち行列に書く（queued_command）
        _append_queued_prompt(row, items)
        return
    if row_type not in ("user", "assistant"):
        return
    # subagent の作業は既定では表示しない
    if row.get("isSidechain") is True:
        return
    msg = row.get("message") or {}
    content = msg.get("content")
    uid = row.get("uuid") or row.get("parentUuid") or ""
    timestamp = row.get("timestamp") or ""
    if row_type == "user" and row.get("isMeta") is True:
        # スキル読み込みの前置きなど，人が書いていない文は出さない
        return
    if isinstance(content, str):
        # 素の文字列の user 行は指示文そのもの．先頭がタグの特殊メッセージもここに来る
        if row_type == "user" and content.strip():
            _append_text_items(content, uid, timestamp, items)
        return
    if not isinstance(content, list):
        return
    for block in content:
        if not isinstance(block, dict):
            continue
        btype = block.get("type")
        if btype == "thinking":
            continue
        if btype == "text":
            text = block.get("text") or ""
            if not text.strip():
                continue
            if row_type == "user":
                # user の文章にもタグが混ざっていることがある
                _append_text_items(text, uid, timestamp, items)
            else:
                # エージェントの発言は特殊メッセージに判定しない
                items.append({"id": uid, "kind": "agent", "time": timestamp, "text": text})
        elif btype == "tool_use":
            tool_id = block.get("id") or ""
            name = block.get("name") or ""
            args = block.get("input") if isinstance(block.get("input"), dict) else {}
            item = _tool_item(tool_id or uid, timestamp, name, args)
            items.append(item)
            if tool_id:
                pending[tool_id] = item
        elif btype == "tool_result":
            tool_id = block.get("tool_use_id") or ""
            target = pending.get(tool_id)
            if target is None:
                # 対応する tool_use が無い（範囲外の）結果は単独では出さない
                continue
            result_text = _text_of(block.get("content"))
            if result_text:
                # タグがあれば整形して event に，残った本文だけを detail へ
                plain, events = analyze_special(result_text)
                _extend_events(events, tool_id or uid, timestamp, items)
                if plain:
                    _join_result(target, plain)
            if _is_tool_error_cc(block):
                target["ok"] = False
        # それ以外のブロックは表示しない


def _convert_pi_row(row: dict, items: list[dict], pending: dict[str, dict]) -> None:
    """pi の1行を items に足す．表示しない行は捨てる．"""
    if row.get("type") != "message":
        return
    msg = row.get("message") or {}
    role = msg.get("role")
    content = msg.get("content")
    uid = row.get("id") or ""
    timestamp = row.get("timestamp") or ""
    if role == "user":
        text = _text_of(content)
        if text.strip():
            # pi の行も cc と同じ特殊メッセージ判定に通す
            _append_text_items(text, uid, timestamp, items)
        return
    if role == "assistant":
        if not isinstance(content, list):
            return
        for block in content:
            if not isinstance(block, dict):
                continue
            btype = block.get("type")
            if btype == "thinking":
                continue
            if btype == "text":
                text = block.get("text") or ""
                if text.strip():
                    items.append({"id": uid, "kind": "agent", "time": timestamp, "text": text})
            elif btype == "toolCall":
                tool_id = block.get("id") or ""
                name = block.get("name") or ""
                args = block.get("arguments") if isinstance(block.get("arguments"), dict) else {}
                item = _tool_item(tool_id or uid, timestamp, name, args)
                items.append(item)
                if tool_id:
                    pending[tool_id] = item
            # それ以外は表示しない
        return
    if role == "toolResult":
        tool_id = row.get("toolCallId") or msg.get("toolCallId") or ""
        target = pending.get(tool_id)
        if target is None:
            return
        result_text = _text_of(content)
        if result_text:
            # タグがあれば整形して event に，残った本文だけを detail へ
            plain, events = analyze_special(result_text)
            _extend_events(events, tool_id or uid, timestamp, items)
            if plain:
                _join_result(target, plain)
        details = msg.get("details") if isinstance(msg.get("details"), dict) else {}
        if msg.get("isError") is True or details.get("isError") is True:
            target["ok"] = False
        return
    # model_change 等はここに来ない（type が message ではないため）
