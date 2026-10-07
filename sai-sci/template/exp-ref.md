# 参照コードと出典の決まり（exp-ref）

`sci-exp-scout`・`sci-exp-sprout`・`sci-exp-branch` が共通で読む決まり。参照実装をどう探すか、値にどう出典を付けるか、どこで止めるかを定める。

## 1. 目的

- モデルの式と数値は、**参照実装か論文から写す。記憶から書かない。**
- 外から取ったコードは `refcode.py` で取る。置き場は `<sai_home>/refcode/`（`<sai_home>` は環境変数 `SAI_HOME`、無ければ `~/.sai`）。手で `git clone` しない。
- 書いた値には `# src:` で出典を書く（§5）。出典が無い値と、出典が「記憶」の値は使わない（§7）。

## 2. 探す順

見つかったら次へ進まなくてよい。上の段階で見つかったものを採用してよい。

1. **Library**（取り込み済みの論文の本文とメモが持つリンク）。

   ```bash
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" links --json
   ```

   返りは `{"links": [{"paper_id", "title", "cite", "source_kind", "kind", "url", "raw", "model_id", "ref", "subpath", "candidates": []}], "mentions": [{"paper_id", "title", "cite", "text"}]}`。対象の論文に絞るときは `--paper 9` を足す。
2. **ModelDB のカタログ**（公開されている神経細胞モデル）。

   ```bash
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/modeldb.py" search <語> --json
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/modeldb.py" search --doi <DOI> --json
   ```

   カタログが無いときは終了コード 1 で返る。**そのときは `modeldb.py sync` を勝手に走らせない。** 全件で数十分かかり、外のサーバに負荷をかけるので、質問ツールで起動者に「sync を走らせてよいか」を聞く（§7）。
3. **GitHub**。

   ```bash
   uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" gh-search "<語>" --json
   ```

   返りは `[{"full_name", "url", "description", "stars", "language", "pushed_at", "license"}]`。検索の上限（10 回/分）に当たったら**待たずに次へ進み**、そのことを REPORT に書く。

## 3. 取る

```bash
uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" fetch <url> --json
uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" fetch modeldb:<id> --json
```

- 返りは `{"url", "kind", "id", "sha", "ref", "path", "cached", "license", "files"}`。**返りの `path` を控える**（依頼書に書くのはこの `path`）。
- 1 つの実験で取るのは **3 本まで**。
- 返りに `candidates` があるときは、順に `fetch` を試す（どれを取るかは、読んだ中身で決める）。
- Zenodo など `fetch` が扱わないものは終了コード 2。取り直さず、次の候補へ進む。
- 取り直しは `fetch <url> --ref <tag や枝>` でできる。

## 4. 読む

- README を読む。モデルの式・定数のあるファイルを読み、どの論文のどの式を実装しているかを確かめる。
- 定数を抜き出すときは次の命令を使う。

  ```bash
  uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py" params <path> --json
  ```

  返りは `[{"name", "value", "number", "unit", "comment", "file", "line", "src"}]`。`file` と `line` が `# src:` に書く材料になる。
- 抜き出した定数と、Library の F や塊にある値が食い違うときは、両方を出典表に書いて「食い違い」と記す（どちらが正しいかは、この段では決めない）。

## 5. 出典の注記 `# src:`

書き方は `# src: <出典>[; <出典>]` である。出典は次の 5 つの形のどれか 1 つにする。

| 形 | 書き方 | 例 |
|---|---|---|
| ref | `<owner>/<repo>@<sha 7〜40 桁の 16 進>:<path>:<line か line-line>`、または `modeldb/<id>@<sha か v<数字>>:<path>:<line>` | `miscco/NM_TC@1c8f5f09d3a9:Thalamic_Column.h:122` |
| chunk | Library の塊の ID `#<論文>.<n>[.<n>]` | `#9.13.1` |
| f | research-tree の F の ID | `F42` |
| spec | 実験の決まりで決めた値。`仕様書` と書く | `仕様書` |
| guess | 根拠の無い仮の値。`目安` と書く | `目安` |

例（`train.py` の書き方。5 つの形をすべて出す）:

```python
# ref の例（参照実装）
QMAX = 0.4  # src: miscco/NM_TC@1c8f5f09d3a9:Thalamic_Column.h:122
# chunk の例（Library の塊）
VTH = -50.0  # src: #9.13.1
# f の例（research-tree の F）
N_UNITS = 80  # src: F42
# spec の例（実験の決まりで決めた値）
EPOCHS = 25  # src: 仕様書
# guess の例（根拠の無い仮の値）
TAU_DECAY = 30.0  # src: 目安
```

誤りの例（**こうは書かない**）:

```python
# 出典が「記憶」の値。止める。
WMAX = 0.03  # src: 記憶（公開コードの記憶）
# 出典が無い値。止める。
ALPHA = 0.7
```

- **「記憶」や `memory` を書くのは誤り。** 書いてよいのは上の 5 つだけ。
- `refcode.py check` が見るのは次の 2 つだけである。
  - **R1**: 最上位の代入の中で、自明でない数値を含む行。文の 1 行目は、すぐ上の行のコメントでもよい。
  - **R2**: `pick_device` と `main` を除く最上位の関数。自明でない数値があれば、関数の中に `# src:` が 1 つ以上要る。
- 自明な数値は、絶対値が 0・1・2・0.5 の数（これらは出典が要らない）。
- 雛形の名前（`PROJECT, SCI_NODE, STUDY, SPROUT_RUN_ID, MODEL, ACTION, PARENT_BRANCH, METRIC, DIRECTION, EPOCHS, DEVICE, EXPERIMENT, ARTIFACT_LOCATION, BRANCH_RUN_ID, TRIAL_TIMEOUT_S, EXP_TAGS, RUN_TAGS`）は見ない。雛形の定数に `# src:` は要らない。

## 6. パラメータの出典表

実験仕様書（`exp-core` §3）の本文に、次の形の表を置く。参照コード調査書（`sci-exp-scout`）にも同じ形の表を下書きとして置く。

| 名前 | 値 | 単位 | 出典 | 意味 |
|---|---|---|---|---|
| `QMAX` | 0.4 | — | `miscco/NM_TC@1c8f5f09d3a9:Thalamic_Column.h:122` | 闘値の最大値 |
| `LR` の掃引の幅 | 0.01 〜 0.1 | — | `目安` | 学習率。感度を見る |

- 載せるのは、**モデルの名前付き定数のすべて**と、**掃引の軸の幅**である。
- 出典が `目安` の行で、成功基準や結論に効く値（掃引の幅・成熟の曲線の長さなど）は、次のどちらかにする。どちらにしたかを備考に書く。
  1. `GRID` に入れて感度を測る。
  2. Issue の O として research-tree に出す（`sai-sci propose` の `add_o` で `kind: issue`）。実験の前提を確かめずに決めない。
- 出典が「記憶」の行は書かない（§7）。

## 7. 止める条件

次のどれかに当たったら、値を推測で埋めない。

- **出典が「記憶」の値**と、**出典の無い値**は使わない。使う必要があるときは、GAP として質問ツールで起動者に聞く。
- **`refcode.py check <train.py>` が終了コード 1 のとき**は投げない。`errors` の行に出典を付けて直す。出典表にも参照コードにも無い値しか直せないときは、作業報告書に `BLOCKED`（出典の無い値の一覧）と書き、review の CARD は作らずに終える。起動者には質問ツールで知らせる（`exp-core` §8 の `check` が失敗したときと同じ扱い）。`guesses`（目安）の数は作業報告書に書く。
- **式が読めず、参照実装も補足の資料も無いとき**は、記憶で埋めずに止める。起動者に質問ツールで知らせ、`BLOCKED` で報告する。
- **ModelDB のカタログが無いとき**は `sync` を走らせず、起動者に聞く。

## 8. ライセンス

- `fetch` の返りの `license` を、出典表の近くに書く。
- `license` が `null` のリポジトリは、**コードを写さない。** 数値と式を出典付きで使い、コードは自分で書く。使うときはその旨を REPORT に書く。

## 9. 呼び方

共通契約 C2 どおり、次の形で呼ぶ。別のファイルを探したり、パスを作り変えたりしない。

```bash
REF='uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/refcode.py"'
MDB='uv run --no-project --python ">=3.11" python "$SAI_PROJECT_ROOT/.sai/tools/modeldb.py"'
```

- 終了コードは 0 = 成功、1 = 失敗（理由は標準エラー）、2 = 引数の誤り（`modeldb.py sync` の中断は 130）。`--json` を付けたとき、標準出力は JSON 1 つだけである。
- この節の命令は全文で書いた形で示す。上の 2 行の変数に入れても同じ。