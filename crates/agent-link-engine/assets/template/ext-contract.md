# 拡張（extension）の契約

この文書は、拡張の作者（エージェント）が拡張を作るための契約の正である。この 1 枚だけで拡張を作れるように書いてある。
agent-link 本体（以下 **base**）のリポジトリは読まなくてよい。

用語:
- **base**: agent-link 本体。`agent-link` の実行ファイルと、その中の基盤の block・template。
- **拡張**: base の外に作る、プロジェクトごとに足す部品。CLI・block・template・画面・外部サービスの接続を持てる。
- **置き場**: 拡張のフォルダを置く場所（`AGENT_LINK_EXTENSIONS_DIR`）。
- **有効**: 拡張がプロジェクトで使われている状態。

## 1. 固定部分と拡張できる部分

| 部分 | 固定か拡張か | 中身 |
|---|---|---|
| 実行エンジン（DB・常駐・run・カード・文書・子タスク） | 固定 | 拡張からは変えられない |
| run の決まった時点（hook） | 固定の 3 点。拡張は命令を宣言する | `context`・`write_check`・`question`。呼ぶのは、その拡張の block の run だけ（§3） |
| 基盤の block・template（`init` で配る物） | 固定 | 拡張は名前を足すだけ。基盤の物は上書きしない |
| `agent-link` の命令（`ext` の 1 群を除く） | 固定 | 拡張は `agent-link ext` の下だけで扱う |
| 拡張の CLI（`cli.bin`） | 拡張 | 作者が作る。データは `.agent-link/ext/<name>/` に置く |
| 拡張の block・template | 拡張 | 拡張のフォルダの `blocks/`・`template/` に置く。有効化で写す |
| 拡張の画面（`ui/`） | 拡張 | 公開の部品だけを使う |
| 外部サービス（service） | 拡張（宛先は設定で決める） | 読み取りが中心。POST は宣言した道だけ |

## 2. フォルダと `extension.json`

拡張 1 つ = 置き場の `<name>/` フォルダ（フォルダ名 = `name`）。

```
<拡張のフォルダ>/            （フォルダ名 = name）
├── extension.json          必須
├── bin/                    任意。CLI の実行ファイル（cli.bin が指す）
├── lib/                    任意。CLI が実行時に使う物（Python のパッケージなど）
├── blocks/<name>.md        任意。<name>.md か <name>-*.md だけ
├── template/<name>-*.md    任意。<name>.md か <name>-*.md だけ
└── ui/                     任意。画面の JS・CSS・アイコン
```

`extension.json` の項目:

| 項目 | 必須 | 型と規則（違えば宣言の誤り） |
|---|---|---|
| `name` | ○ | `^[a-z][a-z0-9-]{0,31}$`。予約名は不可: `link`・`doc`・`mcp`・`ext`・`plan`・`extension`・`main`・`sub`・`web`・`worker`・`review`・`explore`・`kantoku`・`bugfix`（基盤の block・template の名前 `<予約名>-*.md` とぶつかるため） |
| `title` | ○ | 表示名。1〜40 文字（前後の空白を除いて数える） |
| `version` | | 文字列（表示用）。無ければ空として扱う |
| `description` | | 文字列（表示用） |
| `requires` | | 拡張の名前の配列（既定 `[]`）。有効化のとき、ここの拡張がすべて先に有効でなければならない |
| `cli` | | `{bin, web}`。`bin` は拡張のフォルダからの相対パス（`..`・絶対パス・`\` は不可）で、実行できるファイルであること（実行の権限が必要）。`web` は Web から呼んでよいサブ命令の名前の配列（`^[a-z][a-z0-9-]{0,31}$`、既定 `[]`） |
| `init` | | 文字列の配列。有効化のとき `cli` に渡す引数。`cli` が無いのに書いたら誤り |
| `update` | | 文字列の配列。`ext update` のとき `cli` に渡す引数。`cli` が無いのに書いたら誤り |
| `ui` | | `{title?, icon?, scripts, styles?}`。どれも `ui/` からの相対パス（`..`・絶対パスは不可）で、ファイルがあること。`scripts` は `.js`（1 個以上）、`styles` は `.css`、`icon` は `.svg`。`title` を省くと上の `title` |
| `service` | | `{url_env, user_env?, password_env?, post?}`。`*_env` は `^AGENT_LINK_EXT_[A-Z0-9_]{1,40}$`。`post` は POST を中継してよい道（`^[A-Za-z0-9._/-]{1,200}$`、`..` の区切りは不可。**先頭の `/` は付けない**: `/api/ext/<ext>/svc/<path>` の `<path>` と同じ形で書く。例 `"api/runs"`）の配列（既定 `[]`） |
| `hooks` | | `{context?, write_check?, question?}`。各値は文字列の配列（1 要素以上）で、`cli.bin` に渡す引数。`cli` が無いのに書いたら誤り。知らないキーは誤り。使い方は §3 |
| `config` | | 設定値の名前の配列（既定 `[]`・32 個まで・重ねない）。各名前は `^AGENT_LINK_EXT_[A-Z0-9_]{1,40}$`。使い方は §5 |
| （その他） | | **知らない項目は誤り**（綴りの誤りを見つけるため） |

- `blocks/` と `template/` の `.md` は宣言に書かない。フォルダの中身から決まる。ファイル名は `<name>.md` か `<name>-*.md` でなければ誤り。
- block は frontmatter の `id` が、ファイル名（拡張子を除く）と同じでなければ誤り。
- symlink を含むフォルダは複製しない（`ext install` のとき誤り）。`--link` のときは、フォルダそのものを指す symlink を作るだけである。
- `version` と `description` を省いても誤りにならない。

宣言の例（見本の拡張。項目の意味は上の表のとおり）:

```json
{
  "name": "hello",
  "title": "Hello",
  "version": "0.1.0",
  "description": "Sample extension for tests",
  "requires": [],
  "cli": { "bin": "bin/hello", "web": ["greet"] },
  "init": ["init"],
  "update": ["init", "--update"],
  "ui": { "title": "Hello", "icon": "icon.svg", "scripts": ["hello.js"], "styles": ["hello.css"] },
  "service": { "url_env": "AGENT_LINK_EXT_HELLO_URL", "user_env": null, "password_env": null, "post": [] },
  "hooks": { "context": ["context", "{run_id}"], "write_check": ["check", "{filename}"], "question": ["on-answer"] },
  "config": ["AGENT_LINK_EXT_HELLO_TOKEN"]
}
```

## 3. 拡張の hook（`hooks`）

base が run の決まった時点で呼ぶ命令を、拡張が宣言できる。点は次の 3 つだけで、増やせない。

| 点 | base が呼ぶとき | 標準入力 | 結果の使い方 | 上限 |
|---|---|---|---|---|
| `context` | run の最初の文を作るとき | 無し（閉じる） | 標準出力を、最初の文の末尾の `# context` の節に入れる | 20 秒 |
| `write_check` | `agent-link doc write` が文書を保存する直前 | 保存しようとする本文 | 設定の `write_check`（`settings.json`）の後に呼ぶ。終了コード 0 は置き換え・1 は保存しない（下） | 30 秒 |
| `question` | Web の質問に答えが届いたとき（base の質問フックが `agent-link ext hook question` を呼ぶ） | 下の JSON | base は結果を使わない（拡張が答えを記録するための点） | 10 秒 |

- **これは cc の hook ではない。** 拡張は MCP・cc の hook・skill を配らない。この hook は、base が自分の決まった時点で拡張の CLI を呼ぶ点であり、エージェントの道具は CLI 1 本のままである。
- `pre_run`（起動直前）の hook は無い。起動の直前に作りたい物は、`context` の出力に入れる。
- `hooks.<点>` の値は、`cli.bin` に渡す引数の配列である。次の語は、呼ぶときに置き換える: `{run_id}`（run の番号）・`{block}`（block の id）・`{filename}`（`write_check` だけ。ほかの点では空文字）。
- **持ち主**は、次の 3 つを満たす拡張のうち、`name` が一番長い 1 つである。
  1. このプロジェクトで有効である。
  2. その点を `hooks` に宣言している。
  3. run の block の id が `<name>` か `<name>-` で始まる。
  持ち主がいなければ、何も呼ばない。呼ぶのは、その拡張の block の run だけである（ほかの block の run では呼ばない）。
- 呼び方は `ext run` と同じ（シェルを通さない・cwd はプロジェクトの root）。環境変数は親の物に `AGENT_LINK_PROJECT_ROOT`・`AGENT_LINK_BIN`・`AGENT_LINK_PARENT_RUN_ID`（run の番号）・`AGENT_LINK_BLOCK_NAME`（block の id）を足す。`write_check` は加えて `AGENT_LINK_WRITE_FILENAME`（保存するファイル名）・`AGENT_LINK_WRITE_DESCRIPTION`（`--description` の文）を足す。
- 標準出力と標準エラーの読む長さの上限は、標準出力が 200,000 文字、標準エラーが 4,000 文字（前後の空白を除く）である。超えた分は切れる。

### `context`

- 終了コード 0 で、標準出力に空でない中身があれば、最初の文の末尾に `\n\n# context\n\n<標準出力>` を足す（標準出力の最後の改行 1 つは除く）。中身が空なら何も足さない。
- 失敗（0 以外・起動できない・時間切れ）のときは、run を止めない。同じ `# context` の節に `（拡張 <name> の context を作れなかった: <理由>）` の 1 行を入れる。理由は標準エラーの最後の空でない行で、無ければ `exit code N`。
- `block preview` のように run の無い場面では呼ばない。

### `write_check`

- 設定の `write_check` が先に動く。設定が断れば、拡張の `write_check` は呼ばれない。
- 終了コード 0: 標準出力が空でなければ、それを本文として置き換えて保存する（空なら元の本文のまま）。置き換えの本文が 200,000 文字を超えると切れるので、超えないようにする。
- 終了コード 1: 保存しない。標準エラーの全文が、エージェントへの理由になる（`Not saved. The document breaks the rules:` の後ろに付く）。理由は読んで直せる文にする。
- それ以外（起動できない・時間切れを含む）: 元の本文のまま保存し、警告 `The extension checker did not run (<理由>). Saved without checking.` を返す。

### `question`

標準入力に渡す JSON（`ext hook question` が受け取った物をそのまま渡す）:

```json
{
  "run_id": 12, "block": "hello-agent", "qid": "<Web の質問 id>",
  "questions": [ /* cc の AskUserQuestion の questions そのまま */ ],
  "answers": { "<質問文>": "<答え>" },
  "answered_by": "user"
}
```

- `agent-link ext hook question [--json]` は、標準入力にこの JSON を受け、`run_id`（整数）と `block`（文字列）が無ければ終了コード 2。持ち主を探して `question` の命令を呼び、標準出力に `{"ext": <名前>|null, "ran": <真偽値>, "code": <拡張の終了コード>|null}` を出す。持ち主がいない・拡張が失敗した・起動できなかった・時間切れのときも、終了コードは 0 である（`ran` と `code` で分かる。起動できない・時間切れは `ext` に名前を入れ、`ran: false`・`code: null`）。呼び出し側は失敗を無視してよい。
- 拡張の `question` の命令は、答えを自分のデータ（`.agent-link/ext/<name>/`）に記録するなどに使う。失敗しても、答えは cc に返る。

## 4. 待ち条件付きカード

カード（todo の run）に「この命令が 0 を返したら実行する」という待ち条件を付けられる。外の仕事（学習・計算・ジョブ）が終わるのを待ってから、エージェントを起こしたいときに使う。

```bash
agent-link card wait <run_id> [--every <分>] [--json] -- <命令> [<引数>…]
agent-link card wait <run_id> --clear [--json]
```

拡張の使い方の例（拡張の CLI は、仕事が終わっていれば 0・まだなら 1 を返す。ここでは `hello` の `job-done` サブ命令）:

```bash
agent-link card wait 12 --every 15 -- agent-link ext run hello -- job-done J1
```

- カードは todo にあること（無ければ終了コード 3）。`card create` で作ったあと、`card wait` で付ける。`card create` の `meta` には書けない（`wait`・`wait_every`・`wait_note` は予約で、終了コード 2）。
- `--every` は 1〜1440（分）で、既定は 10。範囲の外は終了コード 2。命令は空でなく、改行を含まず、全部で 4,000 文字までである。`--clear` は命令・`--every` と同時に使えない。
- 付けると、カードの `scheduled_at` を今にして、daemon がすぐ見に行く。外すと `scheduled_at` も消える。
- `--json` の出力は `{"run_id", "wait", "every", "scheduled_at"}`。`--clear` のときは `wait`・`every`・`scheduled_at` が `null`。
- task.md の frontmatter には次のキーが入る（`card wait` だけが書く）。

  | キー | 値 |
  |---|---|
  | `wait` | 命令の JSON 配列（1 行）。例 `["agent-link","ext","run","hello","--","job-done","J1"]` |
  | `wait_every` | 見に行く間隔（分・1〜1440。読めなければ 10 として扱う） |
  | `wait_note` | 最後の失敗の `YYYY-MM-DD HH:MM <理由>`（理由は 1 行・200 文字まで） |

- **daemon が呼ぶ。** 予定の時刻ごとに、命令を呼ぶ（シェルを通さない・cwd はプロジェクトの root・上限 60 秒）。環境変数は `AGENT_LINK_PROJECT_ROOT` と `AGENT_LINK_BIN`。命令の先頭が `agent-link` なら、daemon が base の実行ファイルに置き換える。
- 命令の終了コードの読み方:

  | 終了コード | daemon がすること |
  |---|---|
  | 0 | 待ち条件を外してカードを実行する |
  | 1 | まだ。予定を `wait_every` 分後へずらして、待ち続ける |
  | それ以外・時間切れ・起動できない | `wait_note` に理由を書いて、待ち続ける（予定は `wait_every` 分後へずらす） |

- 命令を呼ぶ間に `card wait --clear` で待ち条件が外されたカードや、todo から消えたカードは、命令の結果が 0 でも実行しない（結果を書く前にカードを読み直して確かめる）。
- 拡張の CLI は、「終わっていれば 0・まだなら 1」を守る。失敗は 2 以上で返し、標準エラーの最後の行に英語の理由を出す（それが `wait_note` に残る）。

## 5. 設定値と秘密

拡張が読む設定値（外のサービスの URL・鍵・フォルダの場所など）は、`extension.json` の `config` に名前で宣言する。

- 名前は `AGENT_LINK_EXT_` で始め、残りは `A-Z`・`0-9`・`_` の 1〜40 文字。重ねて書けない。32 個まで。
- 値は次の順で決まる。環境変数が空でなければそれ。無ければ `$AGENT_LINK_HOME/settings.json` の `config` のその名前の値。どちらも無ければ「無し」。
- 値は利用者が決める（`settings.json` の `config` か環境変数）。**拡張は利用者全体のデータの置き場を持たない。** 共有したい物（データのフォルダなど）の場所は、設定値で利用者が指す。
- 読み方は 2 通り。

  | 読み方 | 使う人 |
  |---|---|
  | `agent-link ext config <name> --json` → `{"name", "values": {"<名前>": "<値>"\|null}}`（キーは辞書順） | 拡張の CLI。自分の宣言した名前だけ読める。プロジェクトは要らない。無い名前の値は `null` |
  | `agent-link ext config <name>`（`--json` 無し） | 人。`<名前>: set` か `<名前>: not set` だけを出し、値は出さない（宣言の順） |

  `ext list --json` の各拡張にも `config: [{"name", "set"}]` が出る（値は出ない）。
- **値を、標準出力・文書・記録に書かない。** 値は秘密でありうる。拡張の CLI は、読んだ値を標準出力・エラーの文・`doc write` の本文・ログに出さない。使うときは、自分のプロセスの中だけで使う。
- **`agent-link ext config` をエージェントに呼ばせない。** block・template の本文に `ext config` を呼べと書かない（エージェントの会話の記録に値が残る）。値が要る処理は、拡張の CLI の中で `ext config` を呼んで済ませ、エージェントには結果だけを返す。

## 6. 置き場と記録

| 物 | 場所 |
|---|---|
| 拡張の置き場 | `AGENT_LINK_EXTENSIONS_DIR`（環境変数 → `$AGENT_LINK_HOME/settings.json` の `config`）。無ければ `$AGENT_LINK_HOME/extensions`。拡張 1 つ = `<置き場>/<name>`（フォルダ、または `--link` の symlink） |
| 有効の記録 | `<root>/.agent-link/settings.json` の `"extensions": ["hello", …]`。プロジェクトのファイルだけを読む。書くときは他のキーを保つ |
| 拡張のデータ | `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/`。拡張の CLI が作って使う。base は作らず、消さない |
| 配った block・template | `<root>/.agent-link/blocks/`・`<root>/.agent-link/template/` |
| service の宛先 | `config_var(service.url_env)`（環境変数 → `config`）。値が `http://` か `https://` で始まれば ready |

- 拡張のデータは必ず `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/` に置く。`.sai/`・`~/.sai`・`SAI_*` の環境変数を使わない。
- 拡張の CLI の環境変数は `AGENT_LINK_*` の名前を使う。

## 7. `agent-link ext` の命令

すべて `--json` を受ける。失敗のとき、標準出力は空にし、標準エラーの最後の行に英語の理由を 1 行出す。終了コードは 2 入力の誤り・3 見つからない・4 状態の衝突・1 その他。

| 命令 | プロジェクト | すること | 主な失敗 |
|---|---|---|---|
| `ext list` | 要らない | 置き場の拡張を名前順に並べる | — |
| `ext install <dir> [--link]` | 要らない | 宣言を検査し、`<置き場>/<name>` を作る。既にあれば置き換える（中身・symlink を消してから作る）。複製は `extension.json`・`bin/`・`lib/`・`blocks/`・`template/`・`ui/` だけで、実行の権限は保つ | 2: 宣言の誤り・`<dir>` が無い・symlink を含む |
| `ext uninstall <name>` | 要らない | `<置き場>/<name>` を消す（symlink なら link だけ） | 3: 無い。4: どこかのプロジェクトで有効（`enabled_in` が空でない） |
| `ext enable <name>` | 要る | (1) `requires` がすべてこのプロジェクトで有効か確かめる (2) block と template を写す（`update=false`） (3) `init` があれば起こす (4) `extensions` に足す。既に有効なら (2)(3) を行い、(4) は足さない | 3: 入っていない。4: `requires` が未有効。1: `init` が失敗（記録しない） |
| `ext disable <name>` | 要る | このプロジェクトで有効な他の拡張が `requires` していないか確かめる。`extensions` から外す。写した block・template のうち、拡張の版と同じ物だけ消す（利用者が直した物は残し、`kept` に出す） | 3: このプロジェクトで有効でない。4: 他の拡張が要る |
| `ext update [<name>]` | 要る | 有効な拡張（名前を指定すればそれだけ）の block と template を `update=true` で写し、`update` の引数があれば起こす。最初の失敗で止まる | 3: 指定した名前が有効でない。1: `update` の失敗 |
| `ext run <name> [--stdin] [-- <args>…]` | 要る | 拡張の CLI を起こし、標準出力をそのまま出す（`--json` は付けても同じ。中身は拡張の物）。`--stdin` を付けると、自分の標準入力（UTF-8・1,000,000 バイトまで）を拡張の CLI の標準入力に渡す（付けないと子の標準入力は閉じている） | 3: 入っていない。4: このプロジェクトで有効でない。2: `cli` が無い・`--stdin` の入力が上限を超える・UTF-8 でない。それ以外は子の終了コード（2・3・4 はそのまま、ほかは 1） |
| `ext hook question [--json]` | 要る | 標準入力の JSON（§3 `question`）を、持ち主の拡張の `question` の命令にそのまま渡す。base の質問フックが呼ぶ。拡張の作者が自分で呼ぶ物ではない | 2: 標準入力が JSON でない・`run_id`（整数）か `block`（文字列）が無い・点の名前が `question` でない。それ以外は 0（持ち主がいない・拡張が失敗した・時間切れでも 0） |
| `ext config <name> [--json]` | 要らない | 拡張が `config` に宣言した設定値を読む（§5）。`--json` 無しは `set`・`not set` だけを出し、値は出さない | 3: 入っていない |

`ext list --json` の形:

```json
{
  "dir": "/Users/me/.agent-link/extensions",
  "extensions": [
    {
      "name": "hello", "title": "Hello", "version": "0.1.0", "description": "…",
      "path": "/Users/me/.agent-link/extensions/hello", "linked": false,
      "requires": [],
      "cli": {"bin": "bin/hello", "web": ["greet"]},
      "init": ["init"], "update": ["init", "--update"],
      "hooks": {"context": ["context", "{run_id}"], "write_check": null, "question": ["on-answer"]},
      "config": [{"name": "AGENT_LINK_EXT_HELLO_TOKEN", "set": false}],
      "blocks": ["hello-agent.md"], "templates": ["hello-rules.md"],
      "ui": {"title": "Hello", "icon": "icon.svg", "scripts": ["hello.js"], "styles": ["hello.css"]},
      "service": {"url_env": "AGENT_LINK_EXT_HELLO_URL", "user_env": null, "password_env": null, "post": [], "ready": false},
      "enabled_in": ["myproj"]
    }
  ],
  "errors": [{"path": "/Users/me/.agent-link/extensions/broken", "error": "extension.json: unknown field `colour`, expected one of …"}]
}
```

- 無い項目は `null`（`cli`・`ui`・`service`・`hooks`）か `[]`（`requires`・`init`・`update`・`blocks`・`templates`・`config`）になる。`version` は無ければ `""`。
- `hooks` があれば 3 つのキー（`context`・`write_check`・`question`）が必ず出る。宣言の無い点は `null`。
- `config` は宣言の順で、各項目は `{"name", "set"}`。`set` は値があるかどうかで、値そのものは出ない。
- `enabled_in` は、登録簿（`$AGENT_LINK_HOME/registry.json`）の各 root の `<root>/.agent-link/settings.json` の `extensions` に名前があるプロジェクトの `name` である（登録簿の順）。
- 宣言が壊れた拡張は `extensions` に入れず、`errors` に入れる（命令は成功にする）。置き場が無ければ `extensions: []`。

その他の `--json` の形:

- `ext enable --json` → `{"name", "enabled": true, "assets": [{"path", "outcome"}], "init": "ran" | "none"}`。`outcome` は `created`・`updated`・`unchanged`・`kept`。
- `ext disable --json` → `{"name", "enabled": false, "removed": [<path>…], "kept": [<path>…]}`。
- `ext update --json` → `{"updated": [{"name", "assets": [...], "update": "ran" | "none"}]}`。
- `ext install --json` → `{"name", "path", "linked"}`。
- `ext hook question --json` → `{"ext": <名前>|null, "ran": <真偽値>, "code": <拡張の終了コード>|null}`（`--json` が無くても同じ JSON を出す）。
- `ext config --json` → `{"name", "values": {<名前>: <値>|null}}`（キーは辞書順）。
- `ext uninstall --json` → `{"name", "removed": true}`。

`--json` が無いときは、人向けの英語の文を 1 行以上出す。

## 8. CLI の約束

- base が拡張の CLI を起こすとき（`init`・`update`・`ext run`・hook）は、シェルを通さない。cwd はプロジェクトの root になる。環境変数は親の物に `AGENT_LINK_PROJECT_ROOT=<root>` と `AGENT_LINK_BIN=<agent-link の絶対パス>` を足す。
- 時間の上限は `init`・`update` が 300 秒、`ext run` が 120 秒。超えたら子を止め、終了コード 1（`Extension command timed out after N seconds.`）。
- `init`・`update` が 0 以外で終わると、base は失敗にし、標準エラーの最後の行を理由として出す。
- run の中（エージェント）では、そのプロジェクトで有効な拡張について、`<拡張のフォルダ>/<cli.bin のフォルダ>` を PATH の先頭側に足す。エージェントは `hello` のように名前だけで呼べる。
- 拡張の CLI が守ること:
  - データは `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/` に書く。`.sai/` と `~/.sai` を読み書きしない。環境変数は `AGENT_LINK_*` を使い、`SAI_*` を使わない。
  - `cli.web` に書いたサブ命令は、成功したら標準出力に JSON を 1 つ出す。失敗したら標準出力を空にし、標準エラーの最後の行に英語の理由を出し、終了コード 2・3・4・1 を返す。
- **拡張の CLI から base の命令を呼ぶ。** `AGENT_LINK_BIN`（無ければ PATH の `agent-link`）で呼んでよい。
  - 呼んでよい物: 読む命令（`run show`・`run tree`・`run md`・`doc list`・`doc read`・`card options`・`card docs`）と、`card create`・`card schedule`・`card wait`。
  - 呼ばない物: 状態を大きく変える命令（`card run`・`run finalize`・`run rm`・`daemon`・`init`・`ext` の `install`・`uninstall`・`enable`・`disable`・`update`）。run を動かす・終わらせる仕事は、人とエージェントのものである。
- **requires 先の拡張の CLI** は `"$AGENT_LINK_BIN" ext run <other> -- <args…>` で呼ぶ。Web から起こされたときも、run の中でも、同じ形で動く。上限は 120 秒（呼ばれた側の `ext run` の上限）。標準入力を渡すときは `--stdin` を付ける。
- **block の frontmatter** の、base が知らない項目は無視される。拡張は自分の block に自分の項目（例 `hello_mode: x`）を書き、自分の CLI で読んでよい。項目名は拡張の名前で始める（`<name>_…`。base が将来足す項目とぶつからないため）。
- **拡張のフォルダの置き方。** `ext install` が写すフォルダは `bin/`・`lib/`・`blocks/`・`template/`・`ui/` だけである。台本・見本・型の定義などは `lib/` に置く。プロジェクトごとの物にしたいファイルは、拡張の `init` で `.agent-link/ext/<name>/` へ写す。
- **設定値**は `agent-link ext config <name> --json` で読む（§5）。値を標準出力・文書・記録に書かない。
- **遠隔の子の中でも、拡張は名前で呼べる。** 遠隔の機械（`remote` の子）の PATH の先頭に `<遠隔の home>/.agent-link-remote/runs/<run_id>/ext-bin/` があり、そのプロジェクトで有効で `cli` を持つ拡張ごとに、同じ名前の中継（`hello` など）が入る。中継は Mac の `agent-link ext run <name> [--stdin] -- <args…>` を、門番（`remote-gate`）経由で呼ぶ。つまり拡張の CLI は **Mac で動く**（遠隔の機械では動かない）。上限は 120 秒。標準入力が端末でなければ `--stdin` が付く（上限は同じ 1,000,000 バイト）。門番が通すのは `ext run <name> [--stdin] -- …` だけで、ほかの `ext` の命令は通さない。

## 9. 画面の約束

拡張の JS（`ui/<script>.js`）は、IIFE で包み、一番外側に名前を作らない。

```js
// 拡張の ui/<script>.js（IIFE で包み、一番外側に名前を作らない）
(() => {
  AgentLinkExt.register("hello", {
    async mount(el, ctx) { /* el は毎回新しい空の div。ctx は下 */ },
    async poll(ctx) { /* 任意。interval ミリ秒ごと（0 なら呼ばない） */ },
    interval: 0,
  });
})();
```

| `ctx` | 中身 |
|---|---|
| `ctx.ext` / `ctx.project` | 拡張の名前 / 選んでいるプロジェクトの名前 |
| `ctx.path` | `#/ext/<ext>/<project>/` より後ろの文字列（無ければ `""`） |
| `ctx.go(path)` | `location.hash = "#/ext/<ext>/<project>/" + path` |
| `ctx.cli(args, {input})` | `POST /api/projects/<project>/ext/<ext>/cli` に `{args, input}` を送り、JSON を返す（`saiApi` を使う）。呼べるのは `cli.web` のサブ命令だけ。`input`（文字列・任意・1,000,000 文字まで。超えたら 400）を渡すと、`ext run --stdin` で拡張の CLI の標準入力になる。`input` を省けば標準入力は閉じている |
| `ctx.svc(path, {method, query, body})` | `/api/ext/<ext>/svc/<path>` を呼び、JSON を返す（`method` の既定は `GET`） |
| `ctx.url(file)` | `/ext/<ext>/<file>`（`ui/` の中のファイルの住所） |

- 住所: `#/ext/<ext>` を開くと、最後に選んだプロジェクト（`sessionStorage`）か、`enabled_in` の最初の物の `#/ext/<ext>/<project>` へ置き換える。`enabled_in` が空なら空の表示になる。
- `#/ext/<ext>/<project>[/<path>]` は、画面の上にプロジェクトを選ぶ `<select>`（`enabled_in` だけ）を出し、その下に拡張の `mount` を置く。
- 画面に出す拡張: `ui` が null でなく、`enabled_in` が空でなく、`service` があるなら `ready` が true の物。
- 拡張の画面は、基盤の HTML の目印（`<!-- ext:styles -->`・`<!-- ext:tabs -->`・`<!-- ext:scripts -->`）に base が差し込む。タブのボタン・`<link>`・`<script>` は、拡張の宣言から作られる。
- 拡張が使ってよい base の部品（公開部品。名前と形を変えるときは、この文書も直す）:
  `saiApi`・`saiToast`・`saiConfirm`・`saiPrompt`・`saiEmpty`・`saiSheet`・`saiEl`・`saiIsWide`・`saiSplitShow`・`renderMarkdown`。
- 他の画面へ移るときは、普通のリンク（`<a href="#/ext/<other>/<project>/<path>">`）か `location.hash` を使ってよい。使ってよい住所は、`#/ext/…` と、base の画面の住所だけ（`#/home`・`#/projects`・`#/project/<name>`（`/workflow`・`/list`・`/list/<run_id>/<view>`・`/defs`・`/sessions` を続けてもよい）・`#/`・`#/day/<YYYY-MM-DD>`・`#/run/<project>/<run_id>`）。ファイルのパスや外の URL は使わない。
- 拡張の CSS は、クラス名を `x-<name>-` で始める（例: `x-hello-text`）。
- 画面の文言は英語にする。390px（iPhone）と 1280px（PC）の両方で使えること。

## 10. 外部サービス（service）

- 宛先は環境変数の名前（`service.url_env`）で指す。値は `AGENT_LINK_EXT_<NAME>_URL` の形で、`$AGENT_LINK_HOME/settings.json` の `config` に書く（環境変数で上書きもできる）。例: `AGENT_LINK_EXT_HELLO_URL` → `http://127.0.0.1:9000`。
- 値が `http://` か `https://` で始まれば ready。ready でなければ、拡張の画面は出ない。
- `GET /api/ext/<ext>/svc/<path>` は、どの道でも中継する。
- `POST /api/ext/<ext>/svc/<path>` は、`service.post` にある道だけ中継する。無ければ 403（`This path is not allowed for POST.`）。
- 宛先が未設定なら 404（`Extension service is not configured.`）。宛先の失敗は 502（`Extension service returned <code>.` / `Could not reach the extension service.`）。
- 中継の規則: 時間の上限 15 秒。リダイレクトは追わない（3xx は 502）。応答は 5 MB まで（超えたら 502）。POST の本文は 1 MB まで、`Content-Type: application/json` で送る。応答の `Content-Type` はそのまま返す。
- `user_env` と `password_env` の両方に値があれば、Basic 認証を付けて中継する。
- 中継は読み取りが中心である。書き込みは `post` に宣言した道だけに限る。

## 11. 開発と確かめ方

開発中は、拡張のフォルダを置き場へ symlink で入れる。

```bash
agent-link ext install --link <拡張のフォルダ>
```

確かめるときは、一時フォルダを使い、利用者の環境（`~/.agent-link`・シェルの設定）を変えない。

```bash
T=$(mktemp -d); P=$(mktemp -d)
export AGENT_LINK_HOME="$T" AGENT_LINK_EXTENSIONS_DIR="$T/extensions" AGENT_LINK_PROJECT_ROOT="$P"
(cd "$P" && agent-link init)                     # 一時フォルダのプロジェクト
agent-link ext install --link <拡張のフォルダ>
(cd "$P" && agent-link ext enable <name>)
(cd "$P" && agent-link ext run <name> -- greet x)
```

- Web で画面を確かめるときは、8297 番で起こす。8400 番の Web は止めない。
  ```bash
  AGENT_LINK_WEB_NO_WATCH=1 AGENT_LINK_HOME="$T" perl -e 'alarm 180; exec @ARGV' agent-link-web serve --port 8297 &
  # … curl http://127.0.0.1:8297/ などで確かめる …
  kill %1
  ```
- 確かめが終わったら、起こしたサーバを止め（自分で起こした PID だけ）、`rm -rf "$T" "$P"` で一時フォルダを消す。
- 時間の上限を付ける（`perl -e 'alarm 180; exec @ARGV' <命令>`）。サーバやブラウザを背景で起こしたら、作業を終える前に止める。

## 12. 守ること

- 拡張の JS は、base と同じオリジンで動く。サンドボックスは無い。個人のローカルで使うことが前提である。
- base（agent-link のリポジトリ）を書き換えない。拡張の作業は拡張のフォルダの中だけで行う。
- base に足りない物があれば、拡張の計画書に「base への依頼」の節を作って書く。その依頼は、利用者に伝える。作者が base を直して済ませない。
- 拡張の名前を base のソースに書かない。拡張の名前は拡張のリポジトリにだけ置く。
- 利用者の環境（`~/.agent-link/`・シェルの設定・`~/.local/bin`）を変える操作は、文書に書くだけにする。

## 13. 見本

見本の拡張 `hello` の宣言と画面の JS の全文。拡張 1 つを作るときは、この形を写して名前と中身を変える。

`extension.json`:

```json
{
  "name": "hello",
  "title": "Hello",
  "version": "0.1.0",
  "description": "Sample extension for tests and the kantoku check.",
  "cli": { "bin": "bin/hello", "web": ["greet"] },
  "init": ["init"],
  "ui": { "icon": "icon.svg", "scripts": ["hello.js"], "styles": ["hello.css"] }
}
```

`ui/hello.js`:

```js
(() => {
  AgentLinkExt.register("hello", {
    async mount(el, ctx) {
      const out = await ctx.cli(["greet", ctx.project]);
      el.appendChild(saiEl("p", { class: "x-hello-text" }, JSON.stringify(out)));
    },
    interval: 0,
  });
})();
```

見本の `bin/hello`（シェルの台本。実行の権限が必要）は、`init`・`greet` の 2 つのサブ命令を持つ。`greet` は `{"hello": "<名前>"}` を 1 行の JSON で出す。
