#!/bin/sh
# Mac で sai-tree を動かしながら直すための補助（本番の配備は deploy.sh）。
#
#   apps/web/server/dev.sh up            直したあとはこれだけ: image → containers → assets → prep → web
#   apps/web/server/dev.sh status        今の状態（Web・イメージ・各コンテナ）
#   apps/web/server/dev.sh web           Web を止めて起動し直す（Python を直したとき）
#   apps/web/server/dev.sh image         イメージを作り直す（crates・hooks・sai-sci を直したとき）
#   apps/web/server/dev.sh containers    古いイメージ・古い置き場のコンテナを作り直し、止まっていれば起こす（-f で作業中でも）
#   apps/web/server/dev.sh assets        動いている各プロジェクトに block・template を配り直す（sai-sci init --update）
#   apps/web/server/dev.sh prep          sai-prep を入れ直す（そのあと sai-prep を起動し直す）
#   apps/web/server/dev.sh all           up と同じ
#
# 画面（static/*）だけを直したときは何も要らない。sw.js の CACHE と index.html の ?v= を
# 上げてブラウザをリロードする。
#
# プロジェクトのデータ（PDF・papr.db・.sai/outputs）は users/ の中にあり、コンテナを作り直しても残る。
# 置き場は ~/sai-tree-dev（SAI_DEV_ROOT で変えられる）。中に home/（SAI_HOME）・users/・bin/
# （CLI の中継）・web.log・web.pid を作る。既に別の置き場で動かしているときは、
# SAI_DEV_HOME・SAI_DEV_USERS・SAI_DEV_BIN で指す。Web は 127.0.0.1:8299。
# 利用者は `SAI_HOME=… SAI_WEB_USERS_ROOT=… apps/web/bin/sai-web user add …` で作る（README）。
set -eu

REPO=$(cd "$(dirname "$0")/../../.." && pwd)
ROOT=${SAI_DEV_ROOT:-$HOME/sai-tree-dev}
HOME_DIR=${SAI_DEV_HOME:-$ROOT/home}
USERS=${SAI_DEV_USERS:-$ROOT/users}
BIN=${SAI_DEV_BIN:-$ROOT/bin}
IMAGE=${SAI_DEV_IMAGE:-sai-tree:dev}
PORT=${SAI_DEV_PORT:-8299}
PID=$ROOT/web.pid
LOG=$ROOT/web.log

# 端末に残った SAI_PROJECT_ROOT などが中継を誤動作させるので、先に外す。
unset SAI_PROJECT_ROOT SAI_PARENT_RUN_ID SAI_BLOCK_NAME SAI_ORIGIN

web_env() {
  export SAI_HOME="$HOME_DIR" SAI_WEB_USERS_ROOT="$USERS" SAI_IMAGE="$IMAGE"
  export SAI_LINK_BIN="$BIN/sai-link" SAI_PAPER_BIN="$BIN/sai-paper" SAI_SCI_BIN="$BIN/sai-sci"
  export SAI_WEB_COOKIE_SECURE=0 SAI_WEB_NO_WATCH=1 SAI_WEB_TOKEN=
  # コンテナのフック（質問など）をこの Web へ向ける（既定の 8200 番は別の Web）
  export SAI_WEB_URL_FOR_CONTAINERS="${SAI_WEB_URL_FOR_CONTAINERS:-http://host.docker.internal:$PORT}"
}

prepare() {
  mkdir -p "$HOME_DIR" "$USERS" "$BIN"
  for n in sai-link sai-paper sai-sci; do
    ln -sf "$REPO/apps/web/server/sai-in-container" "$BIN/$n"
  done
}

web_stop() {
  [ -f "$PID" ] || return 0
  p=$(cat "$PID")
  # 別のプロセスに番号が移っていたら殺さない。
  if ps -p "$p" -o command= 2>/dev/null | grep -q "sai-web\|sai_web\|uvicorn"; then
    kill "$p" 2>/dev/null || true
    for _ in 1 2 3 4 5; do kill -0 "$p" 2>/dev/null || break; sleep 1; done
  fi
  rm -f "$PID"
}

web_start() {
  prepare
  web_env
  if curl -s -m 2 "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then
    echo "$PORT 番が既に使われています（このスクリプトが起こした物ではない）。止めてから実行してください。" >&2
    exit 1
  fi
  nohup "$REPO/apps/web/bin/sai-web" serve --port "$PORT" >"$LOG" 2>&1 &
  echo $! >"$PID"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -m 2 "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then
      echo "Web を起動しました: http://127.0.0.1:$PORT/ （ログ ${LOG}）"
      return 0
    fi
    sleep 1
  done
  echo "Web が起動しません。$LOG を見てください。" >&2
  tail -5 "$LOG" >&2 || true
  exit 1
}

image_build() {
  echo "== イメージを作る（${IMAGE}）。Rust は 2 回目から差分だけ"
  # provenance（時刻入り）を付けると中身が同じでも ID が変わり、毎回コンテナを作り直してしまう
  (cd "$REPO" && DOCKER_BUILDKIT=1 docker build --provenance=false -f apps/web/server/Dockerfile -t "$IMAGE" .)
}

# プロジェクトのコンテナ（label sai.owner が付いた物）の名前。
project_containers() {
  docker ps -a --filter label=sai.owner --format '{{.Names}}'
}

# 実行中のエージェント（claude・pi）がいるか（見つけ方は目安）。
busy() {
  docker exec "$1" pgrep -f '(^|/)claude( |$)|pi --mode json' >/dev/null 2>&1
}

containers_recreate() {
  force=${1:-}
  new=$(docker image inspect -f '{{.Id}}' "$IMAGE" 2>/dev/null) || {
    echo "イメージ $IMAGE がありません。先に image を実行してください。" >&2
    exit 1
  }
  prepare
  web_env
  for c in $(project_containers); do
    cur=$(docker inspect -f '{{.Image}}' "$c")
    if [ "$cur" = "$new" ] && project_root "$c" | grep -q "^$USERS/"; then
      if [ "$(docker inspect -f '{{.State.Running}}' "$c")" = "true" ]; then
        echo "$c: 最新"
      else
        docker start "$c" >/dev/null && echo "$c: 最新（起こした）"
      fi
      continue
    fi
    if [ "$force" != "-f" ] && busy "$c"; then
      echo "$c: 作業中のため飛ばします（作り直すなら containers -f）" >&2
      continue
    fi
    echo "$c: 作り直す"
    docker rm -f "$c" >/dev/null
    # sai-<利用者>-<プロジェクト> → プロジェクトのフォルダ名は <利用者>-<プロジェクト>
    (cd "$REPO/apps/web" && PYTHONPATH=src .venv/bin/python -c \
      "import sys; from sai_web import containers; print(containers.start(sys.argv[1]))" "${c#sai-}")
  done
}

# コンテナのプロジェクトの場所（環境変数 SAI_PROJECT_ROOT）。
project_root() {
  docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$1" | sed -n 's/^SAI_PROJECT_ROOT=//p'
}

# 動いている各プロジェクトに、イメージの中の block・template を配り直す（変わる物は .bak に退避）。
assets_update() {
  for c in $(project_containers); do
    [ "$(docker inspect -f '{{.State.Running}}' "$c")" = "true" ] || continue
    root=$(project_root "$c")
    if docker exec -w "$root" -e SAI_PROJECT_ROOT="$root" "$c" sai-sci init --update --json >/dev/null; then
      echo "$c: block・template を配り直した"
    else
      echo "$c: block・template を配り直せませんでした" >&2
    fi
  done
}

prep_install() {
  uv tool install --python 3.12 --reinstall "$REPO/apps/prep" >/dev/null 2>&1 \
    && echo "sai-prep を入れ直した（動いている sai-prep は Ctrl+C で止めて起動し直す）" \
    || echo "sai-prep を入れ直せませんでした（uv を確かめる）" >&2
}

status() {
  if curl -s -m 2 "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then
    echo "Web: 動いています（http://127.0.0.1:$PORT/）$( [ -f "$PID" ] && echo "pid $(cat "$PID")" )"
  else
    echo "Web: 止まっています"
  fi
  if new=$(docker image inspect -f '{{.Id}} {{.Created}}' "$IMAGE" 2>/dev/null); then
    echo "イメージ $IMAGE: ${new#* }"
    id=${new%% *}
    for c in $(project_containers); do
      if [ "$(docker inspect -f '{{.Image}}' "$c")" = "$id" ]; then s=最新; else s="古い（containers で作り直す）"; fi
      echo "  $c: $(docker inspect -f '{{.State.Status}}' "$c")・$s"
    done
  else
    echo "イメージ $IMAGE: まだありません（image で作る）"
  fi
}

case "${1:-}" in
  status) status ;;
  web) web_stop; web_start ;;
  image) image_build ;;
  containers) containers_recreate "${2:-}" ;;
  assets) assets_update ;;
  prep) prep_install ;;
  up|all) image_build; containers_recreate "${2:-}"; assets_update; prep_install; web_stop; web_start ;;
  *)
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
