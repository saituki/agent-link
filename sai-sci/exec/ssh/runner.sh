#!/usr/bin/env bash
#
# runner.sh - ホスト（Linux）で 1 つのジョブを実行する（ssh の実行先 adapter が置いて起動する）。
#   使い方: runner.sh <job_dir>   （<job_dir> は絶対パス。<root>/jobs/<job_id>）
#   呼び出し側は setsid nohup で切り離して起動する。状態は
#   <job_dir>/state, exit_code, pid, started_at, finished_at に書き、コマンドの
#   標準出力と標準エラーは <job_dir>/log に追記する。
#   同時実行の上限は環境変数 EXP_MAX_JOBS（無ければ 1）。<root>/slots/<n>.lock を flock で 1 つ取る。
#   MLFLOW_TRACKING_URI などは <job_dir>/env から読む（ここでは上書きしない。中央の MLflow に直接書くため）。

set -u

job_dir="${1:-}"
if [ -z "$job_dir" ]; then
  echo "usage: runner.sh <job_dir>" >&2
  exit 2
fi
job_dir="${job_dir%/}"
job_id="$(basename "$job_dir")"
ROOT="$(dirname "$(dirname "$job_dir")")"

state_file="$job_dir/state"
log_file="$job_dir/log"

now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
write_state() { printf '%s\n' "$1" >"$state_file"; }

# --- 中止の扱い -------------------------------------------------------------
# bash はフォアグラウンドの子プロセスが終わるまで trap を実行しない。そこで
# コマンドは専用のプロセスグループ（setsid）でバックグラウンド起動し、wait で
# 待つ。こうすると SIGTERM で trap がすぐ動き、cancelled が failed に上書き
# されない。待機中（queued）の中止も同じ経路で扱う。
cancelled=0
child_pid=""
sleep_pid=""

on_signal() {
  cancelled=1
  if [ -n "$child_pid" ]; then
    kill -TERM -- "-$child_pid" 2>/dev/null || true
  fi
  if [ -n "$sleep_pid" ]; then
    kill "$sleep_pid" 2>/dev/null || true
  fi
}
trap on_signal TERM INT

finish_cancelled() {
  write_state "cancelled"
  now >"$job_dir/finished_at"
}

finish_failed() { # $1=exit_code
  printf '%s\n' "$1" >"$job_dir/exit_code"
  write_state "failed"
  now >"$job_dir/finished_at"
}

# --- pid（プロセスグループ ID）を書く -------------------------------------
# setsid で起動されるので通常は $$ がグループ ID になるが、ps で確かめて書く。
pgid="$(ps -o pgid= -p $$ 2>/dev/null | tr -d '[:space:]')"
[ -n "$pgid" ] || pgid=$$
printf '%s\n' "$pgid" >"$job_dir/pid"

max_jobs="${EXP_MAX_JOBS:-1}"
case "$max_jobs" in ''|*[!0-9]*) max_jobs=1 ;; esac
[ "$max_jobs" -ge 1 ] || max_jobs=1

if ! mkdir -p "$ROOT/slots" 2>/dev/null; then
  printf 'runner.sh: cannot create %s\n' "$ROOT/slots" >>"$log_file"
  finish_failed 1
  exit 1
fi

# --- スロットを 1 つ取るまで待つ -------------------------------------------
# ponytail: 5 秒ポーリングで厳密な FIFO ではない。順序が問題になったら待ち行列ファイルにする。
lock_fd=200
acquired=""
while [ -z "$acquired" ] && [ "$cancelled" -eq 0 ]; do
  i=1
  while [ "$i" -le "$max_jobs" ]; do
    exec 200>"$ROOT/slots/$i.lock"
    if flock -n "$lock_fd" 2>/dev/null; then
      acquired="$i"
      break
    fi
    exec 200>&-
    i=$((i + 1))
  done
  if [ -z "$acquired" ] && [ "$cancelled" -eq 0 ]; then
    sleep 5 &
    sleep_pid=$!
    wait "$sleep_pid" 2>/dev/null || true
    sleep_pid=""
  fi
done

if [ "$cancelled" -ne 0 ]; then
  finish_cancelled
  exit 0
fi

if [ ! -f "$job_dir/cmd" ]; then
  printf 'runner.sh: cmd file not found: %s\n' "$job_dir/cmd" >>"$log_file"
  finish_failed 1
  exit 1
fi

write_state "running"
now >"$job_dir/started_at"

export EXP_JOB_ID="$job_id"
export EXP_JOB_DIR="$job_dir"
export EXP_OUTPUTS="$job_dir/outputs"

cmd_str="$(cat "$job_dir/cmd")"

# コマンドは専用プロセスグループで起動する。$1=code フォルダ、$2=env ファイル、
# $3=コマンド文字列。env は子の中だけで読み込む（runner.sh の変数を汚さない）。
# shellcheck disable=SC2016
setsid bash -c '
cd "$1" || exit 1
if [ -f "$2" ]; then
  set -a
  . "$2"
  set +a
fi
shift 2
exec bash -lc "$1"
' bash "$job_dir/code" "$job_dir/env" "$cmd_str" >>"$log_file" 2>&1 </dev/null &
child_pid=$!

wait "$child_pid"
rc=$?

if [ "$cancelled" -ne 0 ]; then
  # コマンドのプロセスグループを確実に止めてから cancelled を書く。
  kill -TERM -- "-$child_pid" 2>/dev/null || true
  n=0
  while [ "$n" -lt 100 ]; do
    kill -0 -- "-$child_pid" 2>/dev/null || break
    sleep 0.1
    n=$((n + 1))
  done
  kill -KILL -- "-$child_pid" 2>/dev/null || true
  wait "$child_pid" 2>/dev/null || true
  finish_cancelled
  exit 0
fi

printf '%s\n' "$rc" >"$job_dir/exit_code"
now >"$job_dir/finished_at"
if [ "$rc" -eq 0 ]; then
  write_state "done"
else
  write_state "failed"
fi
exit 0
