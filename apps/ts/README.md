# sai-ts（時系列の確認と判断の道具）

人が時系列を見て判断し、その判断をイベントとして LLM agent に渡すための道具。
段階 A は Mac で直接動かす（`127.0.0.1:8210`）。段階 B は Linux のサーバで、プロジェクトのコンテナの中で動く（§多人数モード）。

全体の流れ: 記録を読む → 画面でイベントと選択を付ける → アルゴリズムで処理を足す → データセットを書き出す（スナップショット） → research-tree の E に結ぶ。画面と CLI は同じ物を読み書きする。

## 入れ方と起動

```bash
cd apps/ts && uv sync          # .venv を作る（fastapi・uvicorn・numpy）
cd ~/workdir/Sai/sai-tree      # .sai/ のあるプロジェクトの中
apps/ts/bin/sai-ts demo demo   # 合成データ（ts/datasets/demo）を作る
apps/ts/bin/sai-ts serve       # http://127.0.0.1:8210/ を開く
```

`PATH` に入れる場合は次を足す。利用者のシェルの設定は、利用者自身が書く。

```bash
export PATH="$HOME/workdir/Sai/sai-tree/apps/ts/bin:$PATH"
```

## 環境変数

| 名前 | 既定 | 中身 |
|---|---|---|
| `SAI_PROJECT_ROOT` | cwd から `.sai/` をたどる | プロジェクトの場所 |
| `SAI_TS_PORT` | `8210` | サーバの番号 |
| `SAI_TS_HOST` | `127.0.0.1` | 待ち受ける住所。`127.0.0.1` 以外は `SAI_TS_TOKEN` が要る |
| `SAI_TS_TOKEN` | 空（認証なし） | 設定すると API と `/ext/` は `Authorization: Bearer` か `?token=` を要する。画面には URL の `?token=<値>`（Mac）か `#token=<値>`（sai-web の入口）で渡す。画面は値をそのタブの `sessionStorage`（キー `ts_token`）に保存し、URL からは消す |
| `SAI_LINK_BIN` | `sai-link` | sai-link（block の定義の書き込みに使う） |
| `SAI_SCI_BIN` | `sai-sci` | `snapshot link` が呼ぶ sai-sci |
| `SAI_TS_PYTHON` | 実行中の Python | スクリプトと拡張の server.py を動かす Python |

## 置き場

すべて `<project>/ts/` の中にある。読み書きはこの中だけ。

```
ts/
  datasets/<ds>/records/<rec>/meta.json   記録の meta（srate・n_samples・vars。任意の info）
  datasets/<ds>/records/<rec>/<var>.f32   ch のデータ（float32・リトルエンディアン・ヘッダ無し）
  datasets/<ds>/events.jsonl              イベント（追記だけ）
  datasets/<ds>/schema.json               カテゴリ・メトリクスの定義
  datasets/<ds>/channels.json             ch 設定（並び・label・色・倍率・スペクトル）
  datasets/<ds>/view.json                 画面の状態（人と agent が書く）
  datasets/<ds>/runs.jsonl                アルゴリズムの実行の記録（追記だけ）
  scripts/<name>.py・<name>.json          プロジェクトのアルゴリズム（仕様つき）
  ext/<name>/                             拡張
  snapshots/<name>/                       スナップショット（作ったあとは変えない）
```

`meta.json` には任意の `info`（object）を足せる。key は表示名、値は文字か数（それ以外は JSON の文字で出る）。画面の「患者」タブで、選んだ記録の `info` が表に出る。sai-ts は `info` を読むだけで、検査はしない（`version` は 1 のまま）。

```json
"info": {"age": 63, "diagnosis": "focal epilepsy"}
```

## 画面

PC（幅 1280px）の前提。ヘッダと、左のサイドバー（タブ）と、波形の欄からできている。

- ヘッダ: データセットと記録の選択（◀ ▶ で記録を送る）・マウスのモード（作成 `e`・移動 `v`・拡大 `z`）・作成するイベントの定義・表示の範囲（`start` と `window` を入れて「適用」）・全体の倍率（x0.1〜x10。↑↓ キーで 1 段ずつ）・「💬 Chats で開く」。
- 波形の欄: 名前を押すと、その ch の倍率を選べる（名前の後ろに `x2` などが付く）。ch のスペクトルは ch 設定の `spectrum.on` で出す。全体と ch の倍率は両方掛かる。
- 倍率はブラウザの `localStorage`（キー `ts_gain:<データセット>`）に、データセットごとに覚える。`view.json` には入れない（画面の手元の好み）。
- サイドバーのタブ: 変数（ch の表示・並び・ch 設定）・イベント（一覧と定義・編集）・アルゴリズム（一覧・仕様の入力・実行・結果の表）・データセット（スナップショットの作成・一覧・結び付け）・患者（記録の一覧と `info`）・拡張（有効な拡張）。
- 範囲の帯: 記録の長さを出す。パン・ズーム・記録の切り替えでも数字は変わる。

## CLI

画面と同じことを CLI で行う。成功は `--json` で標準出力に JSON 1 つ。失敗は標準出力に何も書かず、標準エラーの最後の行に理由が 1 行出る。終了コードは 2（入力の誤り）・3（見つからない）・4（状態の衝突）・1（その他）。

| 命令 | 中身 |
|---|---|
| `sai-ts serve [--host H] [--port N]` | サーバを起こす（既定は 127.0.0.1 だけ） |
| `sai-ts init [--update]` | `ts/` を作り、block `ts-engineer` を sai-link に入れる。`--update` で既にある block を書き換える（§agent） |
| `sai-ts demo <ds> [--records N] [--vars N] [--seconds S] [--srate HZ]` | 合成データを作る（試すための物） |
| `sai-ts datasets` | データセットと記録の一覧 |
| `sai-ts meta <ds> <rec>` | 記録の meta |
| `sai-ts events list <ds> [--record R] [--category C] [--type line\|area]` | イベントの一覧 |
| `sai-ts events add <ds>` | 標準入力の JSON で追加（1 件か配列） |
| `sai-ts events update <ds> <id>` | 標準入力の JSON で変更 |
| `sai-ts events rm <ds> <id>` | イベントを消す |
| `sai-ts events export <ds>` | BIDS 互換の TSV（`--json` なら `{"tsv": "…"}`） |
| `sai-ts schema get <ds>` | スキーマを読む |
| `sai-ts schema set <ds>` | スキーマを置き換える（標準入力に全体。使われているカテゴリは消せない） |
| `sai-ts channels get <ds>` | ch 設定を読む |
| `sai-ts channels set <ds>` | ch 設定を置き換える（標準入力に全体） |
| `sai-ts view [--json]` | 画面の状態を読む |
| `sai-ts view set` | 画面の状態の一部を書く（標準入力に一部。`updated_by` は agent） |
| `sai-ts run <name> --dataset <ds> [--record R]... [--vars a,b] [--events id,id] [--params JSON] [--timeout SEC]` | アルゴリズムを動かす（仕様があれば検査し、既定で埋める。既定の上限は 1800 秒） |
| `sai-ts algos list` | アルゴリズムの一覧（名前・出どころ・`error`） |
| `sai-ts ext list` | 拡張の一覧（有効・読めない理由） |
| `sai-ts snapshot create <ds> --name N [--record R]... [--vars a,b] [--categories c,d]` | スナップショットを書き出す |
| `sai-ts snapshot list` | スナップショットの一覧（新しい順） |
| `sai-ts snapshot show <name>` | スナップショットの中身（manifest と links） |
| `sai-ts snapshot link <name> --node S3\|E12 [--text T]` | research-tree に結ぶ（§データセット） |

```bash
apps/ts/bin/sai-ts datasets --json
apps/ts/bin/sai-ts events list demo --json
apps/ts/bin/sai-ts run burst --dataset demo --json
```

## アルゴリズム

置き場は 2 つ。`ts/scripts/<name>.py`（プロジェクト）と `apps/ts/src/sai_ts/algos/<name>.py`（組み込み）。同じ名前ならプロジェクトが勝つ。仕様は同じ場所の `<name>.json`。**仕様が無いスクリプトは画面から動かせない**（`algos list` の `error` に出る。`sai-ts run` は今までどおり動く）。

### 仕様（`<name>.json`）

`kind` は `channel`（ch を足す）・`event`（イベントを足す）・`measure`（イベントに指標を書く）。`target` は `vars`（ch を選ぶ）・`events`（イベントを選ぶ）・`none`。

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
- 検査（`algos.check_spec`・`check_request`）は誤りを 400（CLI は終了コード 2）で返す。未知の項目・`params` の知らない名前も同じ。

### 実行

- 画面（HTTP）: `POST /api/datasets/<ds>/algos/<name>/runs` に `{"records", "params", "target": {"vars", "events"}}`。`202 {"run": "<id>"}` をすぐ返し、裏で動かす。結果は `GET /api/datasets/<ds>/runs?limit=50`。
- CLI: `sai-ts run <name> --dataset <ds> … --json`。終わるまで待つ。`by` は `agent`（親の run があれば `agent:<その値>`）。
- 実行の記録は `ts/datasets/<ds>/runs.jsonl`（追記だけ）。`start` と `end` を `id` でまとめて読む。`end` が無いものは、このサーバのプロセスの分なら `running`、そうでなければ `lost`。

標準入力は `{"dataset", "records", "params", "target", "run"}`。環境変数は `SAI_TS_SOURCE`（`script:<name>@<sha8>`。イベントの `source` に残る）・`SAI_TS_RUN`・`SAI_TS_SHA256`・`SAI_TS_TARGET`（対象。大きすぎるときは無い。その場合は標準入力の `target` を読む）。

標準出力の最後の行の JSON（すべて省略可）: `message`（画面に出す文）・`vars`・`events`（件数）・`table`（`{"columns", "rows"}`。1 列目が `id` なら、行を押すとそのイベントを選ぶ）。

### sdk（`from sai_ts import sdk`）

- 読む: `sdk.dataset(name)` / `sdk.records(name)` / `sdk.load(name, record) -> (meta, data)` / `sdk.events(name, **filters)` / `sdk.view()` / `sdk.target()`
- 書く: `sdk.add_events(name, events)`・`sdk.add_var(name, record, var, values, unit, params, inputs)`（`derived` に script・全長の sha256・params・inputs・run が残る）・`sdk.ensure_category(name, cat, color, type, ch)`・`sdk.ensure_metrics(name, defs)`・`sdk.update_metrics(name, {id: {メトリクス: 値}})`（件数を返す）。`ensure_*` は既にあるものを変えない。

`.f32` と `meta.json` と `events.jsonl` は直接書かない（`sdk.add_var`・`sdk.add_events`・`sai-ts events` を使う）。

組み込みの部品（`_dsp.py`）は、プロジェクトのスクリプトからも `import _dsp` で読める（子の `PYTHONPATH` に `algos/` が入る）。`from sai_ts.algos import _dsp` は使えない。

### 組み込みの 3 本

| 名前 | kind・target | 中身 |
|---|---|---|
| `bandpass` | channel・vars | 帯域通過の ch（`<var>_bp<lo>-<hi>`）と包絡の ch（`<var>_env<lo>-<hi>`） |
| `hfo_detect` | event・vars | 包絡が基準振幅の `thr_min` 倍以上で続く区間をイベントにする。最大振幅が `thr_max` 倍以上、または長すぎる区間は雑音（`noise_category` が空なら捨てる） |
| `event_features` | measure・events | 区間の指標（`duration_s`・`peak_abs`・`rms`・`line_length`・`peak_hz`・`zero_cross`）を書く |

RIPPLELAB（MATLAB）との違い: フィルタは FFT（cheby2 とは数値が一致しない）・基準は記録の全体から取る（使用区間は無い）・結合の幅 `merge_ms` は ms で正しく換算する（RIPPLELAB の EMT 換算は 4 倍になる疑い）。RIPPLELAB は 500 Hz に固定されている（`burst.m:9`）。sai-ts は記録の `srate` を使う。注意点と各 params の既定は、block `ts-engineer` の §5 にまとめてある。

### 書き方の例

仕様のある ch のスクリプトと、イベント・指標の例は、block `ts-engineer` の §4 に全部ある（`apps/ts/blocks/ts-engineer.md`）。最小の形だけ示す。

```python
import json, sys
from sai_ts import sdk
req = json.load(sys.stdin)
ds = req["dataset"]
sdk.ensure_category(ds, "burst", "#f59e0b", type="line", ch="single")
found = []   # 検出したイベントを found に入れる（省略）
rows = sdk.add_events(ds, found)
print(json.dumps({"message": f"{len(rows)} 件", "events": len(rows)}, ensure_ascii=False))
```

- `sdk.add_events` の `source` は `script:<name>@<sha8>` になる。どの版が足したかが残る。
- 足す前に、カテゴリが `schema` に無いと終了コード 2 で止まる。

## 拡張

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
- 拡張の読み込みの誤りは `sai-ts ext list` の `error` に出る。

## データセットと research-tree

前処理とイベントを終えたデータを、変わらない一式（スナップショット）として書き出し、research-tree の S（新しい E を作る）か E（`dataset` を結ぶ）に結ぶ。画面はサイドバーの「データセット」タブ、CLI は次のとおり。

```bash
sai-ts snapshot create demo --name hfo-r01-v1 --record r01 --vars ch01,ch02 --categories HFO
sai-ts snapshot list
sai-ts snapshot show hfo-r01-v1
sai-ts snapshot link hfo-r01-v1 --node S3 --text "HFO の検出"   # S なら新しい E を作る
sai-ts snapshot link hfo-r01-v1 --node E12                      # E なら dataset を結ぶ
```

- 置き場は `ts/snapshots/<name>/`。`manifest.json`・`schema.json`・`events.jsonl`・`records/<rec>/meta.json` と `<var>.f32` が入る。`links.json` は最初に結んだときに作られ、結ぶたびに変わる。同じ名前は作れない（終了コード 4・HTTP 409）。
- `manifest.json` の `ref` は `ts:<name>@<sha256 の先頭 12 桁>`。`version` は 1。`files` は各ファイルの sha256、`sha256` は `files` を `sort_keys` の JSON にしたものの sha256。`vars[].derived` に前処理の script・sha256・params・inputs・run、`runs` にその実行の記録が入る。
- `.f32` は元のファイルへの hard link（できなければ複製）。元の派生変数を作り直しても、スナップショットの中身は変わらない。
- 結ぶときは `SAI_SCI_BIN` の `sai-sci apply --json` を呼ぶ。`S` なら `add_e`（`dataset` は `ts:<name>@<sha12>`。`--text` を省くと「データセット <name>」）、`E` なら `set_dataset`。成功すると `links.json` に 1 件足す。
- 実験のスクリプトからは numpy で読める（`dtype` は `<f4`、長さは `meta.json` の `n_samples`）。

```python
import json
from pathlib import Path
import numpy as np

d = Path("ts/snapshots/hfo-r01-v1")
meta = json.loads((d / "records/r01/meta.json").read_text(encoding="utf-8"))
x = np.fromfile(d / "records/r01/ch01.f32", dtype="<f4")
assert x.shape == (meta["n_samples"],)
```

- 実験ループ（`sci-exp-sprout`・`sci-exp-review`）は、親の E の `dataset` を次の E に引き継ぐ。`sci-exp-branch` は依頼書の「データセット」の行から読む。MLflow の experiment には `sai.dataset` タグとして残る（既にある experiment には付かない）。
- スナップショットは `ts/` の中にあり、遠隔の実行先（ssh・DOK）には送られない。送られるのは workdir の中身だけ（sai-sci の `exp-core` §8）。

## agent（ts-engineer と Chats で開く）

- チャットは sai-web の Chats で、block `ts-engineer` を選んで行う。sai-ts の中にチャットは無い。`ts-engineer` は、アルゴリズム・ch・イベント定義・スナップショットを CLI で扱う（手順と禁止は block の本文）。
- 画面の「💬 Chats で開く」は、sai-web の Chats を別のタブで開く。開いた欄は、このプロジェクトと block `ts-engineer` を選んだ状態になる。
  - sai-web の Projects から開いたときだけ押せる（sai-web が `#token=…&web=…&project=…` の断片を付けて渡す）。押せないときは、釦の title に理由が出る。
  - Mac で直に開くときは、利用者が断片を自分で付ける: `http://127.0.0.1:8210/#web=http://127.0.0.1:8200&project=<名前>`（`web` は sai-web の origin だけ。path は付けない）。
- 利用者の判断は CLI で読む。agent は波形を見られないので、`events list` と `view` を読んで答える。
- block は `sai-ts init` が `ts-engineer` を入れる（sai-web は時系列を開くたびに `init` を呼ぶ）。旧の block `ts-agent` は既存のプロジェクトの `.sai/blocks/` に残るが、使わなくなった。利用者の物なので、sai-ts は消さない。
- `datasets/<ds>/chat.json` は使わなくなった（読まれない。消さずに残してある）。

## 多人数モード（段階 B）

- サーバではプロジェクトのコンテナの中で `sai-ts serve --host 0.0.0.0 --port 8210` が動く。`SAI_TS_TOKEN` は必ず設定する（同じ docker ネットワークの他の利用者のコンテナからも届くため）。
- 利用者は sai-web の Projects の「時系列」から開く。住所は `https://<機械>.ts.net:8443/p/<プロジェクト>/`。
- 合言葉は sai-web が出す入場券を入口が取り替え、URL の `#token=<値>` で渡す。入口は `web` と `project` も断片に足す（画面が sessionStorage に覚える）。
- 詳しくは `docs/server.md` §17。

## 試験

```bash
cd apps/ts
for f in tests/test_*.py; do env -u SAI_PROJECT_ROOT -u SAI_PARENT_RUN_ID -u SAI_BLOCK_NAME -u SAI_TARGET_NODE -u SAI_LINK_BIN -u SAI_SCI_BIN PYTHONPATH=src .venv/bin/python "$f" || exit 1; done
for f in tests/test_*.js; do node "$f" || exit 1; done
```

Python 19 本・JS 12 本（`ls tests/test_*.py | wc -l` で数える）。`test_server_data.py` は `httpx`（dev グループ）を使う。`test_init.py` は偽の sai-link（`tests/fake_sai_link.sh`）で `def write` の引数を確かめる。
