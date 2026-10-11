"""sai-ts の CLI（C12）。agent と人の入口。画面と同じ関数を呼ぶ。

約束（ARCHITECTURE.md §2.3）: 成功は --json で標準出力に JSON 1 つ。失敗は標準出力に何も書かず、
標準エラーの最後の行に日本語の理由を 1 行。終了コードは 2（入力の誤り）・3（見つからない）・
4（状態の衝突）・1（その他）。
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Callable

from . import algos, channels, dataset, demo, events, linkcli, paths, scripts, sdk, snapshots, view
from . import ext as ext_mod, init as init_mod

_HEADINGS = {"positional arguments": "位置引数", "options": "オプション"}


class _Formatter(argparse.HelpFormatter):
    # argparse の固定文言（英語）を日本語にする
    def add_usage(self, usage, actions, groups, prefix=None):
        # 子の prog を作るとき prefix は "" で呼ばれるので、None のときだけ差し替える
        super().add_usage(usage, actions, groups, "使い方: " if prefix is None else prefix)

    def start_section(self, heading):
        super().start_section(_HEADINGS.get(heading, heading))


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        # 終了コード 2。最後の行を「引数の誤り」の 1 行にする
        self.print_usage(sys.stderr)
        sys.stderr.write(f"引数の誤り: {message.splitlines()[0]}\n")
        raise SystemExit(2)


def _json(sp) -> None:
    # SUPPRESS: 親（view）の --json を、子の既定値で上書きしない
    sp.add_argument("--json", action="store_true", default=argparse.SUPPRESS,
                    help="結果を JSON 1 つで標準出力に出す")


def _new(subs, name: str, text: str, json_flag: bool = True):
    sp = subs.add_parser(name, help=text, description=text, add_help=False, formatter_class=_Formatter)
    sp.add_argument("-h", "--help", action="help", help="この説明を表示する")
    if json_flag:
        _json(sp)
    return sp


def _source() -> str:
    # CLI から来た変更の来歴は agent（スクリプトから呼ばれたときは SAI_TS_SOURCE）
    return os.environ.get("SAI_TS_SOURCE") or "agent"


def _stdin_json():
    try:
        return json.loads(sys.stdin.read())
    except ValueError:
        raise ValueError("標準入力が JSON ではありません")


def _params(text: str):
    try:
        return json.loads(text)
    except ValueError:
        raise ValueError("--params は JSON にしてください")


def _msg(exc: KeyError) -> str:
    return str(exc.args[0]) if exc.args else str(exc)


def _classify(e: Exception) -> tuple[int, str]:
    if isinstance(e, paths.ProjectNotFound):
        return 2, str(e)
    if isinstance(e, (linkcli.LinkError, snapshots.SciError)):
        return {400: 2, 404: 3, 409: 4}.get(e.status, 1), e.detail
    if isinstance(e, ValueError):
        return 2, str(e)
    if isinstance(e, KeyError):
        # 「○○がありません: <名前>」の名前だけを出す
        return 3, f"見つかりません: {_msg(e).rsplit(': ', 1)[-1]}"
    if isinstance(e, FileExistsError):
        return 4, str(e)
    if isinstance(e, TimeoutError):
        return 1, f"時間の上限を超えました: {e}"
    return 1, str(e) or type(e).__name__


def _fail(code: int, msg: str) -> int:
    # 標準出力には何も書かない。標準エラーの最後の行に理由を 1 行で
    sys.stderr.write(" ".join(str(msg).split()) + "\n")
    return code


def _run(fn: Callable):
    """共通の包み。fn は (結果, 人向けの文か None) を返す。None なら結果を整形して出す。"""
    def go(a) -> int:
        try:
            result, text = fn(a)
        except Exception as e:
            code, msg = _classify(e)
            return _fail(code, msg or "失敗しました")
        if getattr(a, "json", False):
            out = json.dumps(result, ensure_ascii=False)
        elif text is not None:
            out = text
        else:
            out = json.dumps(result, ensure_ascii=False, indent=2)
        sys.stdout.write(out if out.endswith("\n") else out + "\n")
        return 0
    return go


def _event_line(r: dict) -> str:
    on = f"{r['t']:.3f}" if r["type"] == "line" else f"{r['t0']:.3f}-{r['t1']:.3f}"
    head = " ".join(r["text"].split())[:40]
    return f"{r['id']} {r['record']} {on} {r['category']} {head}".rstrip()


# --- 命令 ---

def _cmd_serve(a) -> int:
    from .server import serve  # FastAPI と uvicorn は要るときだけ読む

    return serve(a.port, a.host)


def _cmd_init(a):
    res = init_mod.init(a.update)
    return res, f"ts/ を用意しました: {res['ts_root']}（block ts-agent: {res['block']}）"


def _cmd_demo(a):
    if a.records < 1 or a.vars < 1 or not (a.seconds > 0 and a.srate > 0):
        raise ValueError("records・vars は 1 以上、seconds・srate は正の数にしてください")
    demo.make(paths.datasets_dir() / paths.check_name(a.ds), a.records, a.vars, a.seconds, a.srate)
    recs = sdk.records(a.ds)
    return {"dataset": a.ds, "records": recs}, f"合成データを作りました: {a.ds}（記録 {len(recs)} 件）"


def _cmd_datasets(a):
    res = {"datasets": dataset.list_datasets()}
    text = "\n".join(f"{d['name']}: {', '.join(d['records']) or '記録なし'}" for d in res["datasets"])
    return res, text or "データセットはありません"


def _cmd_meta(a):
    return dataset.load_meta(a.ds, a.rec), None


def _cmd_events_list(a):
    rows = events.list_events(a.ds, record=a.record, category=a.category, type=a.type)
    return {"events": rows, "count": len(rows)}, "\n".join(_event_line(r) for r in rows) or "イベントはありません"


def _cmd_events_add(a):
    body = _stdin_json()
    if isinstance(body, dict):
        rows = [events.add_event(a.ds, body, _source())]
    elif isinstance(body, list):
        rows = events.add_events(a.ds, body, _source())
    else:
        raise ValueError("標準入力は JSON の object か配列にしてください")
    return {"events": rows, "count": len(rows)}, "\n".join(_event_line(r) for r in rows) or "追加はありません"


def _cmd_events_update(a):
    row = events.update_event(a.ds, a.id, _stdin_json(), _source())
    return row, _event_line(row)


def _cmd_events_rm(a):
    events.delete_event(a.ds, a.id, _source())
    return {"id": a.id, "deleted": True}, f"削除しました: {a.id}"


def _cmd_events_export(a):
    tsv = events.export_tsv(a.ds)
    return {"tsv": tsv}, tsv


def _cmd_schema_get(a):
    return events.load_schema(a.ds), None


def _cmd_schema_set(a):
    return events.save_schema(a.ds, _stdin_json()), None


def _cmd_channels_get(a):
    return channels.load_channels(a.ds), None


def _cmd_channels_set(a):
    return channels.save_channels(a.ds, _stdin_json()), None


def _cmd_view(a):
    return view.load_view(), None


def _cmd_view_set(a):
    return view.save_view(_stdin_json(), "agent"), None


def _ids(text: str | None) -> list[str]:
    return [x.strip() for x in (text or "").split(",") if x.strip()]


def _cmd_run(a):
    params = _params(a.params)
    if not a.timeout > 0:
        raise ValueError("--timeout は正の数にしてください")
    body = {"records": a.record or [], "params": params, "target": {"vars": _ids(a.vars), "events": _ids(a.events)}}
    try:
        spec = algos.find(a.script)[1]
    except algos.NoSpec:
        spec = None  # 仕様が無いものは今までどおり、値をそのまま渡す
    if spec is not None:
        body = algos.check_request(spec, body)
    parent = os.environ.get("SAI_PARENT_RUN_ID")
    by = f"agent:{parent}" if parent else "agent"
    res = scripts.run_script(a.script, a.dataset, body["records"], body["params"], a.timeout,
                             target=body["target"], by=by)
    return res, None


def _cmd_algos_list(a):
    algs = algos.list_algos()
    text = "\n".join(f"{x['name']}  {x['origin']}  {x['title']}  {x['kind']}  {x['error']}".rstrip() for x in algs)
    return {"algos": algs}, text or "アルゴリズムはありません"


def _cmd_ext_list(a):
    exts = ext_mod.list_exts()
    text = "\n".join(f"{e['name']}  {e['title']}  {'有効' if e['enabled'] else '無効'}  {e['error']}".rstrip()
                     for e in exts)
    return {"exts": exts}, text or "拡張はありません"


def _snap_by() -> str:
    parent = os.environ.get("SAI_PARENT_RUN_ID")
    return f"agent:{parent}" if parent else "agent"


def _cmd_snapshot_create(a):
    res = snapshots.create(a.ds, a.name, a.record or [], _ids(a.vars), _ids(a.categories), _snap_by())
    return res, f"書き出しました: {res['name']}（{res['ref']}・イベント {res['events']['count']} 件）"


def _cmd_snapshot_list(a):
    rows = snapshots.list_snapshots()
    text = "\n".join(f"{s['name']}  {s['ref']}  {s['dataset']}  {','.join(s['records'])}" for s in rows)
    return {"snapshots": rows}, text or "スナップショットはありません"


def _cmd_snapshot_show(a):
    return snapshots.show(a.name), None


def _cmd_snapshot_link(a):
    res = snapshots.link(a.name, a.node, a.text, _snap_by())
    return res, f"{res['node']} に結びました（{res['op']}）"


# --- 組み立て ---

def _build() -> argparse.ArgumentParser:
    top = _Parser(prog="sai-ts", description="sai-ts: 時系列を見て判断し、その判断を agent に渡す道具の CLI。",
                  add_help=False, formatter_class=_Formatter)
    top.add_argument("-h", "--help", action="help", help="この説明を表示する")
    subs = top.add_subparsers(dest="cmd", required=True, title="命令", metavar="<命令>", help="次のどれか")

    sp = _new(subs, "serve", "サーバを起こす（既定は 127.0.0.1 だけで待つ）", json_flag=False)
    sp.add_argument("--host", help="待ち受ける住所（既定は環境変数 SAI_TS_HOST、無ければ 127.0.0.1。127.0.0.1 以外は SAI_TS_TOKEN が必須）")
    sp.add_argument("--port", type=int, help="待ち受けの番号（既定は環境変数 SAI_TS_PORT、無ければ 8210）")
    sp.set_defaults(func=_cmd_serve)

    sp = _new(subs, "init", "ts/ を作り、block ts-agent を sai-link の定義に入れる")
    sp.add_argument("--update", action="store_true", help="既にある block ts-agent を書き換える")
    sp.set_defaults(func=_run(_cmd_init))

    sp = _new(subs, "demo", "合成データを作る（試すための物。ts/datasets/<ds>）")
    sp.add_argument("ds", help="データセットの名前")
    sp.add_argument("--records", type=int, default=3, help="記録の数")
    sp.add_argument("--vars", type=int, default=4, help="変数の数")
    sp.add_argument("--seconds", type=float, default=60.0, help="記録の長さ（秒）")
    sp.add_argument("--srate", type=float, default=500.0, help="サンプリング周波数（Hz）")
    sp.set_defaults(func=_run(_cmd_demo))

    sp = _new(subs, "datasets", "データセットと記録の一覧を出す")
    sp.set_defaults(func=_run(_cmd_datasets))

    sp = _new(subs, "meta", "記録の meta（srate・n_samples・vars）を出す")
    sp.add_argument("ds")
    sp.add_argument("rec")
    sp.set_defaults(func=_run(_cmd_meta))

    ev = _new(subs, "events", "イベントを一覧・追加・変更・削除・書き出す", json_flag=False)
    evs = ev.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(evs, "list", "イベントを一覧する（記録・時刻の順）")
    sp.add_argument("ds")
    sp.add_argument("--record", help="この記録だけ")
    sp.add_argument("--category", help="このカテゴリだけ")
    sp.add_argument("--type", choices=["line", "area"], help="この種類だけ")
    sp.set_defaults(func=_run(_cmd_events_list))

    sp = _new(evs, "add", "イベントを追加する（標準入力に 1 件の JSON か配列）")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_events_add))

    sp = _new(evs, "update", "イベントを変える（標準入力に変える項目の JSON）")
    sp.add_argument("ds")
    sp.add_argument("id", help="イベントの id（e- で始まる）")
    sp.set_defaults(func=_run(_cmd_events_update))

    sp = _new(evs, "rm", "イベントを消す")
    sp.add_argument("ds")
    sp.add_argument("id", help="イベントの id（e- で始まる）")
    sp.set_defaults(func=_run(_cmd_events_rm))

    sp = _new(evs, "export", "BIDS 互換の TSV を書き出す")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_events_export))

    sc = _new(subs, "schema", "カテゴリ・メトリクスの定義を読む・書く", json_flag=False)
    scs = sc.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(scs, "get", "スキーマを出す")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_schema_get))

    sp = _new(scs, "set", "スキーマを置き換える（標準入力に全体の JSON）")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_schema_set))

    chn = _new(subs, "channels", "ch 設定（並び・label・色・倍率・スペクトル）を読む・書く", json_flag=False)
    chns = chn.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(chns, "get", "ch 設定を出す")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_channels_get))

    sp = _new(chns, "set", "ch 設定を置き換える（標準入力に全体の JSON）")
    sp.add_argument("ds")
    sp.set_defaults(func=_run(_cmd_channels_set))

    vw = _new(subs, "view", "画面の状態（view.json）を出す。view set で一部を書く")
    vw.set_defaults(func=_run(_cmd_view))
    vws = vw.add_subparsers(dest="view_cmd", title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(vws, "set", "画面の状態の一部を書く（標準入力に JSON。updated_by は agent）")
    sp.set_defaults(func=_run(_cmd_view_set))

    sp = _new(subs, "run", "スクリプト ts/scripts/<name>.py を動かす")
    sp.add_argument("script", help="スクリプトの名前")
    sp.add_argument("--dataset", required=True, help="対象のデータセット")
    sp.add_argument("--record", action="append", help="対象の記録（繰り返せる。省略で全記録）")
    sp.add_argument("--params", default="{}", help="スクリプトに渡す値（JSON の object）")
    sp.add_argument("--timeout", type=float, default=scripts.TIMEOUT, help="時間の上限（秒）")
    sp.add_argument("--vars", help="対象の ch（カンマで区切る。例 ch01,ch02）")
    sp.add_argument("--events", help="対象のイベントの ID（カンマで区切る）")
    sp.set_defaults(func=_run(_cmd_run))

    al = _new(subs, "algos", "アルゴリズムの一覧を出す", json_flag=False)
    als = al.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(als, "list", "アルゴリズムの一覧（名前・出どころ・仕様の誤り）")
    sp.set_defaults(func=_run(_cmd_algos_list))

    ex = _new(subs, "ext", "拡張の一覧を出す", json_flag=False)
    exs = ex.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(exs, "list", "拡張の一覧（名前・有効・読めない理由）")
    sp.set_defaults(func=_run(_cmd_ext_list))

    snp = _new(subs, "snapshot", "データセットを書き出す・一覧・show・research-tree に結ぶ", json_flag=False)
    snps = snp.add_subparsers(dest="action", required=True, title="命令", metavar="<命令>", help="次のどれか")
    sp = _new(snps, "create", "スナップショットを書き出す（変わらない一式）")
    sp.add_argument("ds", help="データセットの名前")
    sp.add_argument("--name", required=True, help="スナップショットの名前（同じ名前は作れない）")
    sp.add_argument("--record", action="append", help="対象の記録（繰り返せる。省略で全記録）")
    sp.add_argument("--vars", help="対象の ch（カンマで区切る。省略で各記録の全 ch）")
    sp.add_argument("--categories", help="対象のカテゴリ（カンマで区切る。省略で全部）")
    sp.set_defaults(func=_run(_cmd_snapshot_create))

    sp = _new(snps, "list", "スナップショットの一覧（新しい順）")
    sp.set_defaults(func=_run(_cmd_snapshot_list))

    sp = _new(snps, "show", "スナップショットの中身（manifest と links）を出す")
    sp.add_argument("name", help="スナップショットの名前")
    sp.set_defaults(func=_run(_cmd_snapshot_show))

    sp = _new(snps, "link", "スナップショットを research-tree の S（新しい E を作る）か E（dataset を結ぶ）に結ぶ")
    sp.add_argument("name", help="スナップショットの名前")
    sp.add_argument("--node", required=True, help="S か E の ID（例 S3・E12）")
    sp.add_argument("--text", help="S に結ぶとき作る E の文（省略で「データセット <名前>」）")
    sp.set_defaults(func=_run(_cmd_snapshot_link))

    return top


def main(argv=None) -> int:
    a = _build().parse_args(argv)
    return a.func(a)


if __name__ == "__main__":
    sys.exit(main())
