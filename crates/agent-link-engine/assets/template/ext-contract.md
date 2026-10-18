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
  "service": { "url_env": "AGENT_LINK_EXT_HELLO_URL", "user_env": null, "password_env": null, "post": [] }
}
```

## 3. 置き場と記録

| 物 | 場所 |
|---|---|
| 拡張の置き場 | `AGENT_LINK_EXTENSIONS_DIR`（環境変数 → `$AGENT_LINK_HOME/settings.json` の `config`）。無ければ `$AGENT_LINK_HOME/extensions`。拡張 1 つ = `<置き場>/<name>`（フォルダ、または `--link` の symlink） |
| 有効の記録 | `<root>/.agent-link/settings.json` の `"extensions": ["hello", …]`。プロジェクトのファイルだけを読む。書くときは他のキーを保つ |
| 拡張のデータ | `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/`。拡張の CLI が作って使う。base は作らず、消さない |
| 配った block・template | `<root>/.agent-link/blocks/`・`<root>/.agent-link/template/` |
| service の宛先 | `config_var(service.url_env)`（環境変数 → `config`）。値が `http://` か `https://` で始まれば ready |

- 拡張のデータは必ず `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/` に置く。`.sai/`・`~/.sai`・`SAI_*` の環境変数を使わない。
- 拡張の CLI の環境変数は `AGENT_LINK_*` の名前を使う。

## 4. `agent-link ext` の命令

すべて `--json` を受ける。失敗のとき、標準出力は空にし、標準エラーの最後の行に英語の理由を 1 行出す。終了コードは 2 入力の誤り・3 見つからない・4 状態の衝突・1 その他。

| 命令 | プロジェクト | すること | 主な失敗 |
|---|---|---|---|
| `ext list` | 要らない | 置き場の拡張を名前順に並べる | — |
| `ext install <dir> [--link]` | 要らない | 宣言を検査し、`<置き場>/<name>` を作る。既にあれば置き換える（中身・symlink を消してから作る）。複製は `extension.json`・`bin/`・`lib/`・`blocks/`・`template/`・`ui/` だけで、実行の権限は保つ | 2: 宣言の誤り・`<dir>` が無い・symlink を含む |
| `ext uninstall <name>` | 要らない | `<置き場>/<name>` を消す（symlink なら link だけ） | 3: 無い。4: どこかのプロジェクトで有効（`enabled_in` が空でない） |
| `ext enable <name>` | 要る | (1) `requires` がすべてこのプロジェクトで有効か確かめる (2) block と template を写す（`update=false`） (3) `init` があれば起こす (4) `extensions` に足す。既に有効なら (2)(3) を行い、(4) は足さない | 3: 入っていない。4: `requires` が未有効。1: `init` が失敗（記録しない） |
| `ext disable <name>` | 要る | このプロジェクトで有効な他の拡張が `requires` していないか確かめる。`extensions` から外す。写した block・template のうち、拡張の版と同じ物だけ消す（利用者が直した物は残し、`kept` に出す） | 3: このプロジェクトで有効でない。4: 他の拡張が要る |
| `ext update [<name>]` | 要る | 有効な拡張（名前を指定すればそれだけ）の block と template を `update=true` で写し、`update` の引数があれば起こす。最初の失敗で止まる | 3: 指定した名前が有効でない。1: `update` の失敗 |
| `ext run <name> [-- <args>…]` | 要る | 拡張の CLI を起こし、標準出力をそのまま出す（`--json` は付けても同じ。中身は拡張の物） | 3: 入っていない。4: このプロジェクトで有効でない。2: `cli` が無い。それ以外は子の終了コード（2・3・4 はそのまま、ほかは 1） |

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
      "blocks": ["hello-agent.md"], "templates": ["hello-rules.md"],
      "ui": {"title": "Hello", "icon": "icon.svg", "scripts": ["hello.js"], "styles": ["hello.css"]},
      "service": {"url_env": "AGENT_LINK_EXT_HELLO_URL", "user_env": null, "password_env": null, "post": [], "ready": false},
      "enabled_in": ["myproj"]
    }
  ],
  "errors": [{"path": "/Users/me/.agent-link/extensions/broken", "error": "extension.json: unknown key 'hooks'"}]
}
```

- 無い項目は `null`（`cli`・`ui`・`service`）か `[]`（`requires`・`init`・`update`・`blocks`・`templates`）になる。`version` は無ければ `""`。
- `enabled_in` は、登録簿（`$AGENT_LINK_HOME/registry.json`）の各 root の `<root>/.agent-link/settings.json` の `extensions` に名前があるプロジェクトの `name` である（登録簿の順）。
- 宣言が壊れた拡張は `extensions` に入れず、`errors` に入れる（命令は成功にする）。置き場が無ければ `extensions: []`。

その他の `--json` の形:

- `ext enable --json` → `{"name", "enabled": true, "assets": [{"path", "outcome"}], "init": "ran" | "none"}`。`outcome` は `created`・`updated`・`unchanged`・`kept`。
- `ext disable --json` → `{"name", "enabled": false, "removed": [<path>…], "kept": [<path>…]}`。
- `ext update --json` → `{"updated": [{"name", "assets": [...], "update": "ran" | "none"}]}`。
- `ext install --json` → `{"name", "path", "linked"}`。
- `ext uninstall --json` → `{"name", "removed": true}`。

`--json` が無いときは、人向けの英語の文を 1 行以上出す。

## 5. CLI の約束

- base が拡張の CLI を起こすとき（`init`・`update`・`ext run`）は、シェルを通さない。cwd はプロジェクトの root になる。環境変数は親の物に `AGENT_LINK_PROJECT_ROOT=<root>` と `AGENT_LINK_BIN=<agent-link の絶対パス>` を足す。
- 時間の上限は `init`・`update` が 300 秒、`ext run` が 120 秒。超えたら子を止め、終了コード 1（`Extension command timed out after N seconds.`）。
- `init`・`update` が 0 以外で終わると、base は失敗にし、標準エラーの最後の行を理由として出す。
- run の中（エージェント）では、そのプロジェクトで有効な拡張について、`<拡張のフォルダ>/<cli.bin のフォルダ>` を PATH の先頭側に足す。エージェントは `hello` のように名前だけで呼べる。
- 拡張の CLI が守ること:
  - データは `$AGENT_LINK_PROJECT_ROOT/.agent-link/ext/<name>/` に書く。`.sai/` と `~/.sai` を読み書きしない。環境変数は `AGENT_LINK_*` を使い、`SAI_*` を使わない。
  - `cli.web` に書いたサブ命令は、成功したら標準出力に JSON を 1 つ出す。失敗したら標準出力を空にし、標準エラーの最後の行に英語の理由を出し、終了コード 2・3・4・1 を返す。

## 6. 画面の約束

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
| `ctx.cli(args)` | `POST /api/projects/<project>/ext/<ext>/cli` に `{args}` を送り、JSON を返す（`saiApi` を使う）。呼べるのは `cli.web` のサブ命令だけ |
| `ctx.svc(path, {method, query, body})` | `/api/ext/<ext>/svc/<path>` を呼び、JSON を返す（`method` の既定は `GET`） |
| `ctx.url(file)` | `/ext/<ext>/<file>`（`ui/` の中のファイルの住所） |

- 住所: `#/ext/<ext>` を開くと、最後に選んだプロジェクト（`sessionStorage`）か、`enabled_in` の最初の物の `#/ext/<ext>/<project>` へ置き換える。`enabled_in` が空なら空の表示になる。
- `#/ext/<ext>/<project>[/<path>]` は、画面の上にプロジェクトを選ぶ `<select>`（`enabled_in` だけ）を出し、その下に拡張の `mount` を置く。
- 画面に出す拡張: `ui` が null でなく、`enabled_in` が空でなく、`service` があるなら `ready` が true の物。
- 拡張の画面は、基盤の HTML の目印（`<!-- ext:styles -->`・`<!-- ext:tabs -->`・`<!-- ext:scripts -->`）に base が差し込む。タブのボタン・`<link>`・`<script>` は、拡張の宣言から作られる。
- 拡張が使ってよい base の部品（公開部品。名前と形を変えるときは、この文書も直す）:
  `saiApi`・`saiToast`・`saiConfirm`・`saiPrompt`・`saiEmpty`・`saiSheet`・`saiEl`・`saiIsWide`・`saiSplitShow`・`renderMarkdown`。
- 拡張の CSS は、クラス名を `x-<name>-` で始める（例: `x-hello-text`）。
- 画面の文言は英語にする。390px（iPhone）と 1280px（PC）の両方で使えること。

## 7. 外部サービス（service）

- 宛先は環境変数の名前（`service.url_env`）で指す。値は `AGENT_LINK_EXT_<NAME>_URL` の形で、`$AGENT_LINK_HOME/settings.json` の `config` に書く（環境変数で上書きもできる）。例: `AGENT_LINK_EXT_HELLO_URL` → `http://127.0.0.1:9000`。
- 値が `http://` か `https://` で始まれば ready。ready でなければ、拡張の画面は出ない。
- `GET /api/ext/<ext>/svc/<path>` は、どの道でも中継する。
- `POST /api/ext/<ext>/svc/<path>` は、`service.post` にある道だけ中継する。無ければ 403（`This path is not allowed for POST.`）。
- 宛先が未設定なら 404（`Extension service is not configured.`）。宛先の失敗は 502（`Extension service returned <code>.` / `Could not reach the extension service.`）。
- 中継の規則: 時間の上限 15 秒。リダイレクトは追わない（3xx は 502）。応答は 5 MB まで（超えたら 502）。POST の本文は 1 MB まで、`Content-Type: application/json` で送る。応答の `Content-Type` はそのまま返す。
- `user_env` と `password_env` の両方に値があれば、Basic 認証を付けて中継する。
- 中継は読み取りが中心である。書き込みは `post` に宣言した道だけに限る。

## 8. 開発と確かめ方

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

## 9. 守ること

- 拡張の JS は、base と同じオリジンで動く。サンドボックスは無い。個人のローカルで使うことが前提である。
- base（agent-link のリポジトリ）を書き換えない。拡張の作業は拡張のフォルダの中だけで行う。
- base に足りない物があれば、拡張の計画書に「base への依頼」の節を作って書く。その依頼は、利用者に伝える。作者が base を直して済ませない。
- 拡張の名前を base のソースに書かない。拡張の名前は拡張のリポジトリにだけ置く。
- 利用者の環境（`~/.agent-link/`・シェルの設定・`~/.local/bin`）を変える操作は、文書に書くだけにする。

## 10. 見本

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
