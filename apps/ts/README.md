# sai-ts（時系列の確認と判断の道具）

人が時系列を見て判断し、その判断をイベントとして LLM agent に渡すための道具。
段階 A は Mac で直接動かす（`127.0.0.1:8210`）。

## 入れ方と起動

```bash
cd apps/ts && uv sync          # .venv を作る（fastapi・uvicorn・numpy）
cd ~/workdir/Sai/sai-tree      # .sai/ のあるプロジェクトの中
apps/ts/bin/sai-ts demo demo   # 合成データ（ts/datasets/demo）を作る
apps/ts/bin/sai-ts serve       # http://127.0.0.1:8210/ を開く
```

## 環境変数

| 名前 | 既定 | 中身 |
|---|---|---|
| `SAI_PROJECT_ROOT` | cwd から `.sai/` をたどる | プロジェクトの場所 |
| `SAI_TS_PORT` | `8210` | サーバの番号 |
| `SAI_TS_HOST` | `127.0.0.1` | 待ち受ける住所。`127.0.0.1` 以外は `SAI_TS_TOKEN` が要る |
| `SAI_TS_TOKEN` | 空（認証なし） | 設定すると API と `/ext/` は `Authorization: Bearer` か `?token=` を要する。画面には URL の `?token=<値>`（Mac）か `#token=<値>`（sai-web の入口）で渡す。画面は値をそのタブの `sessionStorage`（キー `ts_token`）に保存し、URL からは消す |
| `SAI_LINK_BIN` | `sai-link` | チャットが呼ぶ sai-link |
| `SAI_TS_PYTHON` | 実行中の Python | スクリプトと拡張の server.py を動かす Python |

置き場は `<project>/ts/` の中だけ。記録は `datasets/<ds>/records/<rec>/meta.json` と `<var>.f32`（float32・リトルエンディアン・ヘッダ無し）。

`meta.json` には任意の `info`（患者の情報。object）を足せる。key は表示名、値は文字か数（それ以外は JSON の文字で出る）。画面の「患者」タブで、選んだ記録の `info` が表に出る。sai-ts は `info` を読むだけで、検査はしない（`version` は 1 のまま）。

```json
"info": {"age": 63, "diagnosis": "focal epilepsy"}
```

## 画面の操作（PC・幅 1280px 前提）

- 範囲の帯（ヘッダの下）: `表示 start 〜 end` と記録全体の長さを出す。`start`（秒）と `window`（幅・秒）を入れて Enter か「適用」で、その範囲を出す。記録の外は記録の中に収める。パン・ズーム・記録の切り替えでも数字は変わる。
- 全体の倍率: 帯の右の select（x0.1〜x10）か ↑↓ キーで 1 段ずつ変える。
- ch の倍率: 波形の名前の欄を押すと、その ch の倍率を選ぶ小窓が出る（外を押すと閉じる）。名前の後ろに `x2` などが付く。全体と ch の倍率は両方掛かる。
- 倍率はブラウザの `localStorage`（キー `ts_gain:<データセット>`）に、データセットごとに覚える。view.json には入れない（画面の手元の好み）。
- 患者タブ: 一覧（患者＝記録）を押すと記録を切り替える。絞り込みは部分一致。選んでいる患者の情報は `info`、続けて srate・長さ・変数の数。

## CLI

画面と同じことを CLI で行う。成功は `--json` で標準出力に JSON 1 つ。失敗は標準出力に何も書かず、標準エラーの最後の行に理由が 1 行出る。終了コードは 2（入力の誤り）・3（見つからない）・4（状態の衝突）・1（その他）。

| 命令 | 中身 |
|---|---|
| `sai-ts serve [--host H] [--port N]` | サーバを起こす（既定は 127.0.0.1 だけ。127.0.0.1 以外は `SAI_TS_TOKEN` が要る） |
| `sai-ts init [--update]` | `ts/` を作り、block `ts-agent` を sai-link に入れる |
| `sai-ts demo <ds> [--records N] [--vars N] [--seconds S] [--srate HZ]` | 合成データを作る |
| `sai-ts datasets` | データセットと記録の一覧 |
| `sai-ts meta <ds> <rec>` | 記録の meta |
| `sai-ts events list <ds> [--record R] [--category C] [--type line\|area]` | イベントの一覧 |
| `sai-ts events add <ds>` | 標準入力に 1 件の JSON か配列 |
| `sai-ts events update <ds> <id>` | 標準入力に変える項目の JSON |
| `sai-ts events rm <ds> <id>` | イベントを消す |
| `sai-ts events export <ds>` | BIDS 互換の TSV（`--json` なら `{"tsv": "…"}`） |
| `sai-ts schema get <ds>` / `sai-ts schema set <ds>` | スキーマを読む・置き換える（標準入力に全体）。カテゴリに `type`（`area`／`line`）・`ch`（`single`／`all`）を書くと、イベントの追加・変更で検査する |
| `sai-ts channels get <ds>` / `sai-ts channels set <ds>` | ch 設定（`channels.json`：並び・label・色・倍率・スペクトル）を読む・置き換える（標準入力に全体） |
| `sai-ts view [--json]` / `sai-ts view set` | 画面の状態を読む・一部を書く（標準入力に一部。`updated_by` は agent） |
| `sai-ts run <script> --dataset <ds> [--record R]... [--vars a,b] [--events id,id] [--params JSON] [--timeout SEC]` | アルゴリズムを動かす（仕様があれば検査し、既定で埋める。実行は runs.jsonl に残る） |
| `sai-ts algos list` | アルゴリズムの一覧（名前・出どころ・仕様の誤り） |
| `sai-ts ext list` | 拡張の一覧（有効・読めない理由） |

```bash
apps/ts/bin/sai-ts datasets --json
apps/ts/bin/sai-ts events list demo --json
apps/ts/bin/sai-ts view --json
apps/ts/bin/sai-ts run burst --dataset demo --json
```

## agent に使わせる

- チャットは sai-web の Chats で、block `ts-engineer` を選んで行う。sai-ts の中にチャットは無い。
- 画面の「💬 Chats で開く」は、sai-web の Chats を別のタブで開く。開いた欄は、このプロジェクトと block `ts-engineer` を選んだ状態になる。
  - sai-web の Projects から開いたときだけ押せる（sai-web が `#token=…&web=…&project=…` の断片を付けて渡す）。押せないときは、釦の title に理由が出る。
  - Mac で直に開くときは、利用者が断片を自分で付ける: `http://127.0.0.1:8210/#web=http://127.0.0.1:8200&project=<名前>`（`web` は sai-web の origin だけ。path は付けない）。
- 利用者の判断は CLI で読む。agent は波形を見られないので、`events list` と `view` を読んで答える。
- `datasets/<ds>/chat.json` は使わなくなった（読まれない。消さずに残してある）。
- `PATH` に入れる場合は次を足す。利用者のシェルの設定は、利用者自身が書く。

```bash
export PATH="$HOME/workdir/Sai/sai-tree/apps/ts/bin:$PATH"
```

## 拡張の書き方

置き場は `ts/ext/<name>/`。`manifest.json` と ES モジュールの `main.js` を置くと、画面が自分で読み込む（保存すれば読み直す）。

```json
{"name": "hello", "title": "挨拶", "entry": "main.js", "enabled": true}
```

```js
export default function activate(ts) {
  const btn = ts.ui.addHeaderButton({
    id: "hello", label: "挨拶", title: "通知を出す",
    onClick: () => ts.ui.toast("こんにちは"),
  });
  const key = ts.ui.addKey({ key: "h", desc: "挨拶", handler: () => ts.ui.toast("こんにちは") });
  return () => { btn(); key(); };   // 戻り値は後始末の関数（任意）
}
```

- `ts` は画面の `window.TS` そのもの。`ts.state`・`ts.on(kind, fn)`・`ts.api(path, opts)`・`ts.setView(patch)` を使う。
- 計算が要るなら manifest に `"server": "server.py"` を書く。画面から `POST /api/ext/<name>/call/<fn>` で呼べる（時間の上限 60 秒）。

## アルゴリズムの書き方

置き場は 2 つ。`ts/scripts/<name>.py`（プロジェクト）と `apps/ts/src/sai_ts/algos/<name>.py`（組み込み）。同じ名前ならプロジェクトが勝つ。
仕様は同じ場所の `<name>.json`。仕様が無いスクリプトは `sai-ts algos list` の `error` に出て、画面からは動かせない（`sai-ts run` は今までどおり動く）。

仕様（`<name>.json`）。`kind` は `channel`（ch を足す）・`event`（イベントを足す）・`measure`（イベントに指標を書く）。`target` は `vars`（ch を選ぶ）・`events`（イベントを選ぶ）・`none`。

```json
{"version": 1, "title": "バンドパスと包絡", "kind": "channel", "target": "vars",
 "desc": "選んだ ch を帯域通過し、包絡の ch も足す",
 "params": [
   {"name": "low_hz", "type": "number", "default": 80, "min": 0, "unit": "Hz", "desc": "通過域の下端"},
   {"name": "envelope", "type": "boolean", "default": true, "desc": "包絡の ch も足す"},
   {"name": "mode", "type": "enum", "values": ["fft", "box"], "default": "fft"}
 ]}
```

- `params` の型は number・integer・string・boolean・enum（メトリクスと同じ決まり）。`default` は型に合うこと。省略時は boolean が false、それ以外は渡さない。
- 検査（`algos.check_spec`）は誤りを 400（CLI は終了コード 2）で返す。`params` の知らない名前も同じ。

実行:

- 画面（HTTP）: `POST /api/datasets/<ds>/algos/<name>/runs` に `{"records", "params", "target": {"vars", "events"}}`。`202 {"run": "<id>"}` をすぐ返し、裏で動かす。結果は `GET /api/datasets/<ds>/runs?limit=50`。
- CLI: `sai-ts run <name> --dataset <ds> [--record R]... [--vars a,b] [--events id,id] [--params JSON] --json`。終わるまで待つ。`by` は `agent`（親の run があれば `agent:<その値>`）。

実行の記録は `ts/datasets/<ds>/runs.jsonl`（追記だけ）。`start` と `end` を `id` でまとめて読む。`end` が無いものは、このサーバのプロセスの分なら `running`、そうでなければ `lost`。

標準入力（`{"dataset", "records", "params", "target", "run"}`）と環境変数:

- `SAI_TS_SOURCE=script:<name>@<sha8>`（イベントの `source` に残る）・`SAI_TS_RUN`（実行の ID）・`SAI_TS_SHA256`（スクリプトの sha256 全長）・`SAI_TS_TARGET`（対象。大きすぎるときは無い。`sdk.target()` は `{}` を返すので、対象は `req["target"]` を読む）。

標準出力の最後の JSON（すべて省略可）: `message`（画面に出す文）・`vars`・`events`（件数）・`table`（`{"columns", "rows"}`。1 列目が `id` なら、行を押すとそのイベントを選ぶ）。

sdk（`from sai_ts import sdk`）:

- 読む: `sdk.dataset(name)` / `sdk.records(name)` / `sdk.load(name, record) -> (meta, data)` / `sdk.events(name, **filters)` / `sdk.view()` / `sdk.target()`
- 書く: `sdk.add_events(name, events)`・`sdk.add_var(name, record, var, values, unit, params, inputs)`（`derived` に `script`・全長の `sha256`・`params`・`inputs`・`run` が残る）・`sdk.ensure_category(name, cat, color, type, ch)`・`sdk.ensure_metrics(name, defs)`（どちらも既にあるものは変えない）・`sdk.update_metrics(name, {id: {メトリクス: 値}})`（既存の指標に重ねる。件数を返す）

指標を書くアルゴリズムの例（`ts/scripts/duration.py`。`duration.json` の `kind` は `measure`、`target` は `events`）:

```python
import json, sys
from sai_ts import sdk
req = json.load(sys.stdin)
sdk.ensure_metrics(req["dataset"], [{"name": "duration_s", "type": "number", "unit": "s"}])
n = sdk.update_metrics(req["dataset"], {"e-3f9a1c2b7d10": {"duration_s": 0.042}})
print(json.dumps({"message": f"{n} 件に指標を書きました"}))
```

最小の例（`ts/scripts/burst.py`。仕様が無いので CLI だけで動く）:

```python
import json, sys
import numpy as np
from sai_ts import sdk
req = json.load(sys.stdin)
ds = req["dataset"]
found = []
for rec in req["records"] or sdk.records(ds):
    meta, data = sdk.load(ds, rec)
    x = data["ch01"]; sr = meta["srate"]
    idx = np.flatnonzero(np.abs(x) > req["params"].get("thr", 3.0) * np.std(x))
    for i in idx[::int(sr)]:   # 1 秒に 1 件まで
        found.append({"type": "line", "record": rec, "vars": ["ch01"], "t": float(i / sr), "category": "burst", "text": "", "metrics": {}})
sdk.add_events(ds, found)
print(json.dumps({"added": len(found)}))
```

- `sdk.add_events` の `source` は `script:<name>@<sha8>` になる。どの版が足したかが残る。
- `sdk.add_var` で派生変数を足す。`.f32` と `meta.json` は直接書かない。

## 組み込みのアルゴリズム

`apps/ts/src/sai_ts/algos/` に入っている（`origin: "builtin"`。`sai-ts algos list` で見える）。numpy だけで動く。

- `bandpass`（channel・target は vars）: 帯域通過の ch（`<var>_bp<lo>-<hi>`）と包絡の ch（`<var>_env<lo>-<hi>`）を足す。
- `hfo_detect`（event・target は vars）: 包絡が基準振幅の `thr_min` 倍以上で続く区間をイベントにする。最大振幅が `thr_max` 倍以上、または長すぎる区間は雑音（`noise_category` が空なら捨てる）。RIPPLELAB の HFOL の手順を移したもの。
- `event_features`（measure・target は events）: 区間の指標（`duration_s`・`peak_abs`・`rms`・`line_length`・`peak_hz`・`zero_cross`）をイベントに書く。line のイベントは飛ばす。

RIPPLELAB との違い: フィルタは FFT（cheby2 とは数値が一致しない）・基準は記録の全体から取る（使用区間は無い）・結合の幅 `merge_ms` は ms で正しく換算する（RIPPLELAB の EMT の式は 4 倍になる疑い）。

共通の部品は `algos/_dsp.py`（スクリプトからは `import _dsp`）。`algos/` に `__init__.py` は置かない（置くと package が `algos.py` を隠す）。

## 多人数モード（段階 B）

- サーバではプロジェクトのコンテナの中で `sai-ts serve --host 0.0.0.0 --port 8210` が動く。`SAI_TS_TOKEN` は必ず設定する（同じ docker ネットワークの他の利用者のコンテナからも届くため）。
- 利用者は sai-web の Projects の「時系列」から開く。住所は `https://<機械>.ts.net:8443/p/<プロジェクト>/`。
- 合言葉は sai-web が出す入場券を入口が取り替え、URL の `#token=<値>` で渡す（読み方は上の表のとおり）。
- 詳しくは `docs/server.md`。

## 試験

```bash
cd apps/ts
for f in tests/test_*.py; do env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME -u SAI_TARGET_NODE -u SAI_LINK_BIN PYTHONPATH=src .venv/bin/python "$f" || exit 1; done
for f in tests/test_*.js; do node "$f" || exit 1; done
```

`test_server_data.py` は `httpx`（dev グループ）を使う。
