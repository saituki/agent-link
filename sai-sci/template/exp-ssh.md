# sai-ssh: リモートPCで計算を動かす

この文書では、Mac の sai-link エージェントが SSH でリモートPCに計算を投入する方法を説明する。
`sai-ssh` コマンドは Mac（手元）で動かす。リモートPCは「ホスト」と呼ぶ。
数分以上かかる計算は `sai-ssh run` で投入し、結果は自動で手元に届く。

## 1. 使う場面の規則

- 数分以上かかる計算は、必ず `sai-ssh run` で投入する。`ssh <host> '<長いコマンド>'` で直接動かしてはならない。理由: ssh が切れると計算が止まり、結果も回収されない。
- 短い調べもの（`ls`、`nvidia-smi`、`cat` など）は、`ssh <host> '...'` で直接行ってよい。
- 結果は自動で回収される。手で `rsync` や `scp` をしてはならない。急いでいるときは `sai-ssh collect` を実行する。

## 2. 手順

1. 使えるホストを確認する。

   ```bash
   sai-ssh hosts
   ```

2. 初めて使うホストには、先に準備をする。まず次を実行し、出力をユーザーに見せて承認をもらう。承認後にだけ `--yes` を付けて実行する。

   ```bash
   sai-ssh setup <host>
   sai-ssh setup <host> --yes    # ユーザーの承認後
   ```

3. プロジェクトのフォルダ（`train.py` のある所）でジョブを投入する。今いるフォルダがプロジェクトとして送られる。`--data` は一緒に送るデータのフォルダ（省略可）。

   ```bash
   sai-ssh run <host> --name <短い名前> [--data <DIR>] -- <command...>
   ```

   例: `sai-ssh run dgx --name train-small -- uv run python train.py --epochs 1`

4. 標準出力の 1 行目に出る job_id を、ユーザーに伝える。
5. 状態とログを確認する。job_id は省略できる（省略時は全ジョブの一覧）。

   ```bash
   sai-ssh status [<job_id>]
   sai-ssh logs <job_id> [-n 50]
   ```

6. 回収が終わったら、結果報告書を読む。`sai_read {}`（pi では `sai-read {}`）で文書の一覧を出し、`結果報告書 <job_id>:` で始まる description を探して、そのトークンで本文を読む。

## 3. ジョブの中で使える環境変数

投入したコマンドの中では、次の環境変数が使える。

- `SAI_SSH_OUTPUTS`: 成果物を書くフォルダ。ここに書いたファイルが結果報告書に一覧される。
- `SAI_SSH_DATA`: `--data` で送ったデータのフォルダ。
- `SAI_SSH_JOB_ID`: job_id。
- `SAI_SSH_JOB_DIR`: ジョブのフォルダ。
- `MLFLOW_TRACKING_URI`: mlflow の保存先。設定済みなので、コードの中で上書きしてはならない。

`sai-ssh run` に環境変数を渡す引数は無い。**投入する命令の前に書く。** 実験のスクリプトでは、次の形になる（`EXP_BRANCH_RUN_ID` は実験ループの決まりで、実行している branch の run_id だ）。

```bash
sai-ssh run <host> -- env EXP_BRANCH_RUN_ID=<自分の run_id> uv run --with mlflow python <スクリプトのパス>
```

## 4. Python の環境

依存はプロジェクトの `pyproject.toml` と uv で管理する。コマンドは `uv run python train.py` の形で書く。送ったフォルダの中で `uv run` が動き、`pyproject.toml` の依存がホストで解決される。

## 5. 危険な操作の確認（必ず守る）

次の (a)〜(d) は、実行する前に必ずユーザーの承認を得る。

- (a) ジョブ用フォルダの外にあるファイルの削除や上書き
- (b) 道具の導入
- (c) 他の人のジョブやプロセスの停止
- (d) リモートPCの設定の変更

確認の方法:

- Claude Code: `AskUserQuestion` ツールで尋ねる。
- pi: 返答の中でユーザーに尋ね、返事を待つ。

sai-link は cc を権限確認なしで起動する。そのため、この確認はエージェントである自分の指示として守る。

確認なしで実行してよい操作の例:

- `sai-ssh` の各コマンド（`hosts`、`setup`、`run`、`status`、`logs`、`cancel`、`collect`、`watch`、`ui`）
- `~/.sai-ssh` の中を読むこと
- `status` と `logs`
- `sai-ssh cancel <job_id>`（自分が投入したジョブのときだけ）

## 6. ホストごとの注意

- dgx: 共用マシンである。root の vLLM が GPU メモリを約 60GB 使っている。他の人のプロセスは止めない。aarch64（ARM）なので、PyTorch などは aarch64 と CUDA 13 に対応した版を使う。GB10 は unified memory なので、`nvidia-smi` のメモリは `[N/A]` になる。
- wsl-server: まだ接続できない（Windows の PC が起動していない）。

## 7. mlflow の画面

回収した記録は中央の MLflow（sai-moon の実験タブ）で見られる。利用者にはそこを案内する。
`sai-ssh ui` でリモート側の画面を見ることもできるが、記録の正は中央の MLflow である。

```bash
sai-ssh ui [--port 5000]
```
