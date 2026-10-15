# 実装計画書 U04: ほかのアプリ・研究の文書・CI のジョブを消す

## 目的
agent-link のリポジトリから、Radio・sai-ts・sai-prep・MLflow のアプリと、研究・サーバの文書を消す。CI を Rust と Web の 2 つのジョブだけにする。

## 背景
- agent-link は sai-link と Web だけを持つ開発用の道具である（要件定義書 `docs/plan/requirements.md`）。
- 残す文書（`README.md`・`ARCHITECTURE.md`・`AGENTS.md`・`CONCEPT.md`・`docs/integration.md`・`docs/for-sai-developers.md`）は、後のガイド 2 の U07 が名前を変えるときにまとめて書き直す。この作業では消すだけにする。

## 前提（着手時点のコードの状態）
- リポジトリは `~/workdir/Sai/agent-link`（sai-tree の main を clone した物）。
- `.github/workflows/ci.yml` のジョブは 6 つある。`rust`（`:17`）・`web`（`:31`）・`sai-prep`（`:50`）・`radio`（`:62`）・`sai-ts`（`:74`）・`sai-sci-tools`（`:91`）。`web` のジョブの最後の手順（`:47-48`）は `sh apps/web/server/test_sai_in_container.sh` を実行する。
- `docs/plan/` には agent-link の計画書（この文書を含む）だけがある。sai-tree の古い計画書は、複製のときに消してある。
- 同じガイドの U01・U02・U03 と並列で動く。U02 が `apps/web/server/` を消す。

## 変更内容

### 1. アプリを消す
`git rm -r apps/radio apps/ts apps/prep apps/mlflow-server`

### 2. 文書を消す
`git rm docs/server.md docs/sai-sci.md docs/sai-sci-tutorial.md docs/sai-sci-exp-loop.md`

`docs/integration.md`・`docs/for-sai-developers.md` は**残す**（U07 が書き直す）。

### 3. CI（`.github/workflows/ci.yml`）
- ジョブ `sai-prep`・`radio`・`sai-ts`・`sai-sci-tools` を消す。
- `web` のジョブから、コンテナの中継のテストの手順（`:47-48`）を消す。ジョブの名前を `Web（JS・Python）` にする。
- ファイルの頭の注記（`:1-4`）を、Rust と Web だけを回すという内容に直す。
- `rust` のジョブの `env -u … SAI_TARGET_NODE` はそのまま残す（U05 が名前を変えるときに直す）。

### 4. 無視の設定
- `.dockerignore` を消す（Docker のイメージはもう作らない）。
- `.gitignore` から、消したアプリの行（`apps/prep`・`apps/ts`・`apps/radio`・`apps/mlflow-server` の仮想環境や出力の行など）を消す。`.sai/` の行は**消さない**（作業中は sai-moon がこのリポジトリの `.sai/` を使っている）。`lib/` の行は消してよい。

## 共有契約（他の作業と共有する取り決め。worker は変えない）
- C1（ガイド 1 の約束）: この時点では名前を変えない（`sai-link`・`SAI_*`・`.sai/` のまま）。
- C5: `.gitignore` の `.sai/` の行は残す。agent-link のリポジトリは、作業中は sai-moon のプロジェクトで、`.sai/` に実行の記録がある。

## ファイルの所有権
- 書いてよい: `apps/radio/`・`apps/ts/`・`apps/prep/`・`apps/mlflow-server/`（消す）、`docs/server.md`・`docs/sai-sci*.md`（消す）、`.github/workflows/ci.yml`、`.dockerignore`、`.gitignore`
- 読むだけ: なし
- 触らない: `apps/web/**`・`crates/**`・`docs/plan/**`・`docs/integration.md`・`docs/for-sai-developers.md`・`README.md`・`ARCHITECTURE.md`・`AGENTS.md`・`CONCEPT.md`

## 完了条件
- [ ] AC1: `ls apps` が `web` だけになる。`docs/` に `server.md`・`sai-sci*.md` が無い。
- [ ] AC2: `.github/workflows/ci.yml` のジョブが `rust` と `web` の 2 つで、`test_sai_in_container.sh`・`apps/prep`・`apps/ts`・`apps/radio`・`sai-sci` の語を含まない。
- [ ] AC3: `python3 -c "import yaml"` が使えれば、`ci.yml` が YAML として読める。使えなければ、`ruby -ryaml -e 'YAML.load_file(".github/workflows/ci.yml")'` で確かめる。
- [ ] AC4: `git status --ignored` で `.sai/` が無視されたままになっている。

## 検証
```bash
cd ~/workdir/Sai/agent-link   # worktree の中ならその場所
ls apps docs
grep -nE "test_sai_in_container|apps/(prep|ts|radio)|sai-sci" .github/workflows/ci.yml || echo OK
ruby -ryaml -e 'p YAML.load_file(".github/workflows/ci.yml")["jobs"].keys'
git check-ignore -q .sai && echo ".sai ignored"
```
期待: `apps` が `web` だけ。`OK`。`["rust", "web"]`。`.sai ignored`。

## 決定事項と理由
- 残す文書の書き直しは、ガイド 2 の U07 に回す — 理由: 名前を変えるときに全部書き直すので、ここで直すと 2 度手間になる。
- `.gitignore` の `.sai/` は残す — 理由: 作業中は sai-moon が `.sai/` に書く。消すと、worker の `git add -A` が実行の記録を commit してしまう。

## 要確認
- なし
