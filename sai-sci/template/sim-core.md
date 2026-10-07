# 神経シミュレーションの共通の決まり（sim-core）

`sci-sim-model`・`sci-sim-compose` が共通で読む決まり。神経のモデルの**式・状態変数・パラメータ・単位・数値計算・再現の目標**をどう出典つきでまとめるか、どこで止めるかを定める。

## 1. 目的と実験の流れ

- **式と値は、参照実装か論文から写す。記憶や目安で埋めない。** 出典の無い式や値は、`## GAP` に質問の形に書いて止める（§9）。
- 実験の流れは次の順に進む。

```
scout（参照実装を探す） → sim-model / sim-compose（モデル定義書） → sprout → branch → review
```

| 段 | 受け取るもの | 渡すもの |
|---|---|---|
| `sci-exp-scout` | 依頼文と対象 | 参照コード調査書（参照実装の在処と、定数の出典表の下書き） |
| `sci-sim-model` / `sci-sim-compose` | scout の参照コード調査書のトークン（compose は部品ごとのモデル定義書のトークンも） | **モデル定義書**（式と値の正） |
| `sci-exp-sprout` | モデル定義書のトークン | 実験仕様書（`sim_model` にモデル定義書のトークンを入れる） |
| `sci-exp-branch` | 実験仕様書と依頼書（`- モデル定義書:` の行） | `train.py`（式と値はモデル定義書から写し、再現の確かめを MLflow に記録する）と作業報告書 |
| `sci-exp-review` | 台帳（`target.repro_pass`・`target.repro`） | 判定書（再現の目標を外れたら debug、使い切ったら `repro_failed` で止める） |

- **モデル定義書が式と値の正である。** `sprout` と `branch` はそこから写す（`exp-core` §3・§7）。実験仕様書のパラメータの出典表は、モデル定義書の「## パラメータの出典表」をもとに作る。
- 神経のモデルでない実験（画像・音声・言語の実験など）では、この実験の流れは使わない。実験仕様書の `sim_model` は `null` になる（`exp-core` §3）。

## 2. モデル定義書の書式

`sci-sim-model` と `sci-sim-compose` が書く文書の形である。**この節の書式から変えない。**

- ファイル名 `sim-model-<NODE>-ID<自分の run_id>.md`。
- description `モデル定義書 <NODE>: <name>（<kind>）— <一言>`。例 `モデル定義書 O42: costa-multicol（組み合わせ）— #9 の 1 柱に #57 の I_M・I_Nap と柱間結合を足す`。
- 本文の先頭に、次の JSON を ```json のコードブロックで 1 つだけ置く。

```json
{"sim_model": {
  "name": "costa-multicol",
  "kind": "組み合わせ",
  "level": "神経集団",
  "units": {"time": "ms", "voltage": "mV", "rate": "1/ms", "conductance": "mS/cm2", "current": "uA/cm2"},
  "integrator": "euler-maruyama",
  "dt_ms": 0.1,
  "warmup_ms": 5000,
  "components": [
    {"id": "C1", "name": "Costa2016b の視床皮質の 1 柱", "paper": "#9", "ref": "miscco/NM_TC@1c8f5f09d3a9", "path": "<fetch の path>", "license": "MIT", "doc": null},
    {"id": "C2", "name": "#57 の PY の I_M・I_Nap", "paper": "#57", "ref": null, "path": null, "license": null, "doc": "<その部品のモデル定義書のトークン、無ければ null>"}
  ],
  "baseline": {"what": "C1 だけ（I_M・I_Nap なし・柱間結合 0）。論文 #9 の N3 の値", "switches": {"use_im_nap": false, "col_coupling": 0.0}},
  "repro": [
    {"name": "so_peak_hz", "range": [0.5, 1.5], "unit": "Hz", "measure": "PY の膜電位の Welch で 0.1〜4 Hz の最大の周波数", "src": "#9.5.2", "component": "C1"}
  ]
}}
```

| 欄 | 中身 |
|---|---|
| `name` | MLflow の model 名に使える形 `^[a-z0-9][a-z0-9-]{0,39}$` |
| `kind` | `再現`（論文のモデルをそのまま）・`改良`（1 本の土台に、値の変更・電流の追加・時間変化などを足す）・`組み合わせ`（2 本以上の論文の部品をつなぐ）。`sci-sim-model` は `再現` か `改良`、`sci-sim-compose` は `組み合わせ` だけを書く |
| `level` | `単一ニューロン`・`ネットワーク`・`神経集団`・`平均場` のどれか |
| `units` | このモデルで使う単位。式と表の値は、すべてこの単位にそろえる |
| `integrator` / `dt_ms` / `warmup_ms` | 積分法・刻み・捨てる最初の時間。土台の参照コードと同じにする。変えるときは本文の「## 改良点」に理由を書く |
| `components` | 部品の一覧。`ref` は `<owner>/<repo>@<sha12>` か `modeldb/<id>@<sha12 か v<n>>`、無ければ `null`。`doc` は、`sci-sim-compose` が部品ごとのモデル定義書を使ったときのトークン |
| `baseline` | 再現の確かめに使う設定。`what` は 1 文、`switches` は改良・つなぎを外すための値（`FIXED` / `GRID` の名前と値） |
| `repro` | 再現の目標。1 個以上。`range` は `[下限, 上限]`（同じ単位）。`src` は `exp-ref` §5 の chunk・f・ref のどれか（`目安` と `仕様書` は不可）。`component` は、どの部品の目標か（組み合わせのつなぎ全体の目標なら `"all"`） |

- JSON に続く節は、この順で書く（書くことが無い節は「なし」と書く）。

```markdown
## 状態変数
## 式
## パラメータの出典表
## 再現の目標
## 改良点
## つなぎ方
## 食い違い
## GAP
## PROPOSAL
```

各節の表の列は次のとおりである。

| 節 | 表の列 | 中身 |
|---|---|---|
| `## 状態変数` | 名前・意味・単位・初期値・出典・部品 | 方程式の解になる量（膜電位・活動率・状態変数など） |
| `## 式` | ID（`EQ1` から）・式・意味・出典・部品 | 式は 1 行の文字の式で書く（例 `dVp/dt = -(I_L + I_AMPA + I_GABA) / tau_p`）。**出典は `exp-ref` §5 の ref か chunk か f だけ**。`目安` の式は書かない |
| `## パラメータの出典表` | 名前・値・単位・出典・意味・部品・備考 | `exp-ref` §6 の列に、部品と備考を足す。備考には単位の換算（元の値と単位・換算の式）を書く |
| `## 再現の目標` | 名前・範囲・単位・測り方・出典・部品 | JSON の `repro` と同じ中身 |
| `## 改良点` | 何を変えたか・前・後・出典・理由 | 再現なら「なし」 |
| `## つなぎ方` | 送り手（部品と変数）・受け手（部品と変数）・式・換算（単位・倍率）・出典 | 再現・改良なら「なし」 |

- 「## 食い違い」「## GAP」「## PROPOSAL」は表でなくてよい。「## 食い違い」と「## GAP」は `sci-exp-scout` の同じ見出しと同じ形で書く。
- 「## PROPOSAL」は `sci-core` の op の表に従う。

## 3. 式と値の取り方

優先の順は次のとおりである。上の段で見つかれば、下の段は読まずに済ます。**どれで見つけたかも出典の形に残す。**

1. **参照コード**（scout の参照コード調査書の `reference_code` の `path`）。

   ```bash
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" eqs <path> --json
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" params <path> --json
   ```

   - `eqs` は式の行を、`params` は定数を、**どちらも出典（`src`）つきで**返す。返りの `src` をそのまま出典に書く（`exp-ref` §5 の ref の形）。
   - `eqs` の返りは `[{"file", "line", "block", "kind", "text", "src"}]`。`kind` は `mod`（NEURON の `.mod` の DERIVATIVE・BREAKPOINT・KINETIC・PROCEDURE・FUNCTION・INITIAL・NET_RECEIVE・LINEAR・NONLINEAR の中の文）・`ode`（XPP の式の行）・`dxdt`（`d<x>/dt = …` を含む行）・`euler`（`dt` を掛ける代入の行）である。
   - 拡張子で絞り込むときは `--ext .mod,.ode` を足す。
   - 終了コードは 0 = 成功（0 件を含む）、1 = パスが無い、2 = 引数の誤り。0 件なら次の段へ進む。
2. **論文の本文の塊**。検索の決まりは `sci-find` のとおりである。

   ```bash
   sai-sci find "<英語の語>" --node <NODE> --json
   ```

   - **式の塊が `formula-not-decoded`（数式として読めない状態）のときは、そこから式を推測しない。** その節を `## GAP` に書いて質問にする。
3. **research-tree の F**（`add_f` で出した根拠）。

- 参照コードと論文が食い違えば、**両方を「## 食い違い」に書く。** 式は参照コードの形を採ってよい。論文の値と違う定数は、どちらを採ったかを理由と一緒に書く。
- ファイルの種類ごとの読み方:

| 種類 | 式を書くファイル | 網の組み方・刺激・刻みを書くファイル |
|---|---|---|
| NEURON | `.mod` | `.hoc`（`dt` を含む） |
| XPP | `.ode` | `.ode` の `init` の行と `@` の行（`dt`・`total`） |
| Brian2 | Python の式の文字列（`equations`） | Python のコード（`dt` と `run`） |
| C/C++ | `dt` を掛ける更新式 | 初期化と刺激の部分 |

  `eqs` の `kind` がその目安になる。

## 4. 単位

- JSON の `units` に決めた単位へ、**すべての式と値をそろえる。** 時間が `ms` なら `dt_ms` は ms、電流が `uA/cm2` なら「## パラメータの出典表」の電流の値も `uA/cm2` で書く。
- 換算した値は、**出典表の備考に「元の値と単位・換算の式」を書く。** 例 `0.1 S/cm2 → 100 mS/cm2`（`1 S/cm2 = 1000 mS/cm2`）。
- NEURON の `.mod` の単位（`(mV)`・`(mA/cm2)`・`(S/cm2)` など）は `params` の `unit` に出る。換算の根拠になる。
- 単位が決められない値（無次元の比など）は、単位の欄に `—` を書き、備考に「無次元」と書く。

## 5. 数値計算

- **積分法と `dt` は、土台の参照コードと同じにする。** 参照コードが `dt` を掛ける更新式（`kind: euler`）なら `integrator` は `euler`、確率的な更新があれば `euler-maruyama` や `stochastic` に合わせて書く。
- 確率的なモデルは、**乱数の種を `FIXED` に入れて、その値を「## パラメータの出典表」に書く。** `baseline` でも同じ種を使う。
- `warmup_ms` の間の値は、**どの指標にも使わない。** 指標はすべて `warmup_ms` より後の値を使う。
- これらを変えるときは「## 改良点」に理由を書く。**再現（`kind: 再現`）では変えない。**

## 6. 再現の目標の決め方

- 再現の目標は、**論文の図・表・本文の数値**（周波数・発火率・振幅・持続時間・同期率など）から取る。
- 論文が数値で示していればその値の範囲を `range` に書く。図の軸しか無ければ、その図が示している範囲を書き、`measure` にどの図を読んだかを書く。
- `measure`（測り方）は、**何をどの指標で測るか**を 1 文で書く。branch がそのまま書けるように書く。例「PY の膜電位の Welch で 0.1〜4 Hz の最大の周波数」。
- `src` は `exp-ref` §5 の **chunk・f・ref のどれか**。参照コードの README や出力の例なら ref（`miscco/NM_TC@1c8f5f09d3a9:README.md:42`）。`目安` と `仕様書` は **`repro` には使わない。**
- **1 個も書けないときは BLOCKED として質問ツールで起動者に聞く。** 記憶や目安で目標を作らない。
- 記録と判定のしかたは `exp-core` §4（再現の確かめ）と §6（R1a）に従う。`repro.<name>` を MLflow に記録し、`repro_pass` で判定する。**この判定で改良の結論は出さない。** `stop_reason` が `repro_failed` のときは、土台が論文どおりに動いていない。

## 7. 改良の決まり

`kind: 改良`（`sci-sim-model` の `改良`）のときの決まり。

- 土台はそのまま「## 式」「## パラメータの出典表」に書く。**土台の値を出典つきで残す。**
- 変える点だけを「## 改良点」に、**前・後・出典・理由**つきで足す。出典が無い変更は「## GAP」にする。
- `baseline.switches` で**改良を外せるようにする。** 名前は `FIXED` / `GRID` の名前と同じにする（例 `{"use_im_nap": false, "col_coupling": 0.0}`）。**再現の確かめは改良を外した設定で行う。**
- 改良してよいのは GAP に限らない。**根拠（論文の別の図や表・research-tree の F）があれば、出典を付けてよい。**

## 8. 組み合わせの決まり

`kind: 組み合わせ`（`sci-sim-compose`）のときの決まり。

- **部品ごとに状態変数と式を残す。** 部品どうしで同じ名前の値が違ったら「## 食い違い」に書く。
- **つなぐ変数**（例: 発火率 → シナプスの入力）を「## つなぎ方」に、**式と換算つきで**書く。列は送り手（部品と変数）・受け手（部品と変数）・式・換算（単位と倍率）・出典。
- **同じ現象を 2 つの部品が持つときは、どちらか 1 つにして二重に数えない。**
- 部品の `repro` は、**つなぎを外した設定（`baseline.switches`）で測れるもの**を写す。つなぎ全体の目標が論文にあれば、`component: "all"` で足す。
- つなぎの強さや遅れの値に出典が無いときは、**`目安` で置く。** `exp-ref` §6 のとおり、GRID に入れるか Issue の O にするかを備考に書いてから使う。`FIXED` で固定しない。

## 9. 止める条件

次のどれかに当たったら、式や値を推測で埋めない。

- **参照コードにも論文にも無い式・値**：`## GAP` に質問の形に書いて、質問ツールで起動者に聞く。BLOCKED で報告する。
- **再現の目標が 1 個も無い**（`repro` が空）：質問ツールで起動者に聞く。記憶や目安で目標を作らない（§6）。
- **式の塊が `formula-not-decoded`** で、参照実装も補足の資料も無い：`exp-ref` §7 どおり止めて質問する。
- **`license` が `null` のコードを写す必要があるとき**（`exp-ref` §8）：**式と値は使ってよいが、コードは写さない。** コードは自分で書く。その旨を REPORT に書く。
- **ModelDB のカタログが無いとき**は `sync` を走らせず、起動者に聞く（`exp-ref` §7）。

## 10. 呼び方

`exp-ref` §9 と同じく、次の形で呼ぶ。別のファイルを探したり、パスを作り変えたりしない。

```bash
REF='uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py"'
```

- 式を抜き出す（`eqs`）。

  ```bash
  uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" eqs <path> [--ext .mod,.ode] --json
  ```

- 定数を抜き出す（`params`）。

  ```bash
  uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" params <path> --json
  ```

- 参照実装を取る（`fetch`）。決まりは `exp-ref` §3。

  ```bash
  uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" fetch <url> --json
  ```

- 終了コードは 0 = 成功（0 件を含む）、1 = パスが無い、2 = 引数の誤り。`--json` を付けたとき、標準出力は JSON 1 つだけである。
- この節の命令は全文で書いた形で示す。上の 1 行の変数に入れても同じ。