# agent-link の設計（現在の形）

対象: **agent-link を直す人**。
目的: **なぜそうなっているか**と、**変えてはいけない物**を 1 枚にする。

**これは「現在の設計」。** 役割を分けてある。

| 知りたいこと | どこ |
|---|---|
| 守る約束（終了コード・JSON の項目） | [`for-developers.md`](./for-developers.md)（CLI と Web の間） |
| 守る約束（Web の HTTP の住所・画面の住所） | [`integration.md`](./integration.md) |
| 過去の決定と理由（やめた物） | [`decisions.md`](./decisions.md) |
| 置き場・ファイル構成・機能を足す手順 | [`../ARCHITECTURE.md`](../ARCHITECTURE.md) |
| 用語の意味 | [`../CONCEPT.md`](../CONCEPT.md) |
| 遠隔の機械で子の claude を動かす手順 | [`remote.md`](./remote.md) |

**正はコード。**

---

## 1. 不変条件（これを壊す変更は却下する）

機能から逆算した物だけを書く。**由来（sai-tree など）ではなく、アプリが何であるか**から出る。

| # | 不変条件 | 何から出るか | 壊すと |
|---|---|---|---|
| **I1** | **状態・DB・文書・Web の主人は、その Mac（そのプロジェクト）。** 子の `claude` だけは、ssh 先の機械で動いてよい（遠隔の子。[`remote.md`](./remote.md)）。`git`・`tmux`・ファイルは Mac で使う。 | エージェントに仕事をさせる。仕事の一部は別の機械で動かしたい | 状態の主人が 2 つになり、DB と文書が機械ごとに分かれる |
| **I2** | **実行の状態はプロジェクトごとに閉じる**（`<root>/.agent-link/`）。利用者全体の置き場（`$AGENT_LINK_HOME`）にあるのは、全プロジェクト共通の定義（`blocks/`・`template/`・`settings.json`）・登録簿・Web 自身の物（質問・通知・書き出し）だけ。**横断の集計は読み取りだけ。** | プロジェクトを単位に使う | プロジェクトを移す・消す・配るが壊れる |
| **I3** | 人は **PC と iPhone の両方**から同じ物を見る。答えは**先着**。 | 席を離れて使う | 片方だけの設計に戻る |
| **I4** | エージェントの道具は **CLI 1 本だけ**。agent-link は MCP を出さない。 | エージェントが Bash で呼ぶ | 道具が 2 系統になり、片方だけ直る |
| **I5** | **1 人・Mac 1 台**（遠隔の機械は子の `claude` を動かすだけ）。多人数・ログイン・コンテナを持たない。 | 単独で使う | 合言葉・権限・テナントの話が入る |
| **I6** | 外に出るのは **窓口 1 つ**。127.0.0.1 でだけ待ち受ける。Tailscale は**利用者が任意で足す**。 | 守るのは入口だけ | 入口ごとに守りを作る羽目になる |

> **不変条件ではない物**（履歴から来た製品判断。理由と日付は [`decisions.md`](./decisions.md)）: `launchd` を持たないこと・`cc` だけであること・TUI を持たないこと。

## 2. 層と境界

**部品は 2 つ、入口は 2 つ、クレートは 3 つ。**

```
 人（PC / iPhone）                    人・エージェント
      │ HTTPS（iPhone は Tailscale serve :4446・任意）    │ Bash
      ▼                                                  ▼
┌─────────────────────┐                    ┌─────────────────────┐
│ agent-link-web      │  入口①（人）        │ agent-link          │  入口②（人・エージェント）
│ FastAPI 127.0.0.1:8400 │                 │ CLI                 │
│ 画面・質問・通知     │                    │ （engine + view）    │
└──────────┬──────────┘                    └──────────┬──────────┘
           │ 公開された契約だけ                          │ 直接
           │ （CLI の JSON・終了コード・schema・          │
           │   web/runs/*.json・registry.json）          │
           └──────────────────┬───────────────────────┘
                              ▼
                  <root>/.agent-link/        $AGENT_LINK_HOME/
                  （実行の状態・出力）         （共通の定義・登録簿・Web の物）
```

| 部品 | 実体 | 入口 | 持つ物 | 知ってよい物 |
|---|---|---|---|---|
| **実行エンジン** | `crates/agent-link-engine`・`agent-link-view`・`agent-link-cli` → 実行ファイル `agent-link` | ② CLI | 実行の DB・常駐・定義の解析・起動時の組立・Workflow の配置・書き出し | 自分の DB・`<root>/.agent-link/`・登録簿。遠隔の子へは ssh と門番（`agent-link remote-gate`）を通る |
| **Web の窓口** | `apps/web` → `agent-link-web` | ① ブラウザ | 画面・質問・通知・Claude の使用量 | `agent-link` の CLI・登録簿・書き出し・`state.json` |

### 2.1 境界の規則（この 4 本が設計の核）

| # | 規則 | 理由 |
|---|---|---|
| **B1** | **Web はプロジェクトの DB（`agent-link.db`）を開かない。** | DB の形を変えるたびに Web が黙って壊れるのを防ぐ |
| **B2** | **処理はコアに 1 つ。** Web は CLI か HTTP を使うだけ。写さない。 | 同じ処理が 2 か所になると、片方だけ直る不具合が起きる |
| **B3** | **コアは Web を知らない。** | 向きを 1 本にして、コアだけでも動くようにする |
| **B4** | 部品の間の受け渡しは **公開された契約だけ**（CLI の JSON・終了コード・`schema --json`・`web/runs/*.json`・`registry.json`）。 | 契約を 1 か所で保つ（正は [`for-developers.md`](./for-developers.md) と [`integration.md`](./integration.md)） |

### 2.2 Cargo の依存の向き

| 向き | 可否 |
|---|---|
| `agent-link-cli` → `agent-link-engine`・`agent-link-view` | よい |
| `agent-link-view` → `agent-link-engine` | よい |
| `agent-link-engine` → `agent-link-cli`・`agent-link-view` | **いけない** |

- **`view` の利用者は今のところ CLI だけ**（Web は Python なので使えない）。それでも**クレートの境界は残す**。クレートを分けておくと、`engine → view` を**コンパイラが禁じる**。1 つのクレートに畳むと、この規則は「約束事」に格下げになり、いつか DB のコードが組立のコードを呼ぶ。
- 説明は「CLI が使う表示用の組み立て（描画は持たない）」に直した。名前は表示用の組み立てとして合うので残す。

### 2.3 拡張の境界

拡張（extension）は、base（実行エンジン・Web の枠）の上に後から載る機能。固定と拡張できる部分の表・流れは [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §1.5、作り方と契約の全文は [`../crates/agent-link-engine/assets/template/ext-contract.md`](../crates/agent-link-engine/assets/template/ext-contract.md)。ここには境界だけを書く。

- **B1〜B4 は拡張にも当てはまる。** Web は拡張の CLI を `agent-link ext run` を通してだけ呼ぶ（呼べるのは `cli.web` のサブ命令だけ）。拡張の CLI は `.sai/`・`~/.sai` を読み書きせず、自分のデータ（`<root>/.agent-link/ext/<name>/`）だけを書く。
- **base は拡張の名前を知らない。** 拡張の名前を基盤のコードに書かない（テスト `ext_names.rs` で固定）。拡張が増えても基盤の変更は要らない。
- **拡張の JS はサンドボックスの外で動く。** 同じオリジンで動き、Web の API をそのまま呼べる。個人ローカル専用（I5）で、入口は 127.0.0.1 だけ（I6）という前提があるからこの形にする。この前提が崩れたら（多人数・外からの待ち受け）、拡張の UI を見直す。
- **拡張の hook は base が呼ぶ点であり、エージェントの道具ではない**（`context`・`write_check`・`question` の 3 点。呼ぶのは、その拡張の block の run だけ）。エージェントの道具は CLI 1 本のまま（I4 を保つ）。cc の hook・MCP・skill は配らない。
- **拡張の CLI は Mac で動く。** 遠隔の子からは、門番（`remote-gate`）を通して `ext run <name> [--stdin] -- …` だけを呼べる（I1 を保つ。[`remote.md`](./remote.md)）。
- 拡張の配布（レジストリ・署名）は持たない。利用者が `agent-link ext install` でフォルダか symlink を入れる（`docs/for-developers.md` §2.2）。

## 3. 守る良い所（見直しで壊さない）

| 良い所 | なぜ効くか |
|---|---|
| **論理が 1 か所**（Web は DB を開かない） | DB の形を変えても、Web が黙って壊れない |
| **契約が 1 つ**（CLI の JSON と終了コード） | 人・エージェント・Web が同じ入口を使う。テストも 1 つで済む |
| **状態の主人が 1 つ** | 二重に書く所が無い |
| **依存が少ない**（FastAPI・標準ライブラリ・素の JS） | 何年も動く。供給網の心配が小さい |
| **Web は無くても動く** | Web が止まっていても cc は普通に動く（`README.md`）。質問は端末でも答えられる |

## 4. やらないこと（明示）

| やらない物 | 理由 |
|---|---|
| **Docker 化**（mem0 型） | I1 が成り立たない（Mac の `git`・`tmux`・`claude` を使う） |
| **多人数・クラウド・ログイン** | I5 に反する |
| **MCP の道具を配る** | I4 に反する（道具が 2 系統になる） |
| **TUI の復活** | 実装を 2 つにしない |
| **crates.io 公開** | Rust と Python の 2 部品。片方しか配れない |
| **Linux 向けの配布**（agent-link 本体を Linux に置く） | I1（状態の主人は Mac）に反する。`tmux` と `claude` を Mac で使う |
| **拡張の UI のサンドボックス** | 拡張の JS は同じオリジンで動く（§2.3）。サンドボックスを作ると、拡張の画面が base の部品（`saiApi` など）を使えなくなる。個人ローカル専用（I5・I6）の前提で足りる（P6・2026-10-18） |
| **拡張の配布の仕組み**（レジストリ・署名・自動更新） | 1 人が自分の拡張を入れるだけ（I5）。配布を作ると、取り違えと供給網の心配が入る。入れ方は `ext install`（フォルダか `--link`）だけ（K2・2026-10-18） |
| **拡張からの MCP・cc の hook・skill の配布** | エージェントの道具は CLI 1 本（I4）。拡張が MCP や cc の hook を足すと、道具が 2 系統になる。拡張の道具は CLI だけ（P1・2026-10-18）。base が呼ぶ拡張の hook（`context`・`write_check`・`question`）は別物（Q1・2026-10-18） |

過去の決定は [`decisions.md`](./decisions.md) に、日付・理由・状態つきで置く。
