#!/bin/sh
# サーバ（Linux）へ sai-tree を反映する（docs/server.md の「更新のしかた」）。
#
# 使い方（Mac から）:
#   git push server main
#   ssh sai@<マシン名> /srv/sai/app/apps/web/server/deploy.sh
#
# 各段で何をするかを表示してから行う。どこかで失敗したら止まる（set -e）。
set -eu

# サーバの作業用のリポジトリ（bare の /srv/sai/repo.git から clone したもの）。
APP=/srv/sai/app

# ssh のとき PATH（uv の入る ~/.local/bin）と XDG_RUNTIME_DIR・DOCKER_HOST が入らないので足す。
export PATH="$HOME/.local/bin:$PATH"
# rootless Docker の socket を指す。
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export DOCKER_HOST="${DOCKER_HOST:-unix://$XDG_RUNTIME_DIR/docker.sock}"

cd "$APP"

echo "== 1/7 ソースを取り込む（git pull --ff-only）"
git pull --ff-only

echo "== 2/7 コンテナのイメージを作る（上限 60 分）"
timeout 3600 docker build -f apps/web/server/Dockerfile -t sai-tree:latest .

echo "== 3/7 apps/web の仮想環境を合わせる（uv sync）"
(cd apps/web && uv sync)

echo "== 4/7 sai-prep の入れ物を作る（apps/prep/dist/・GET /api/prep/download が配る）"
(cd apps/prep && uv build)

echo "== 5/7 Web を起動し直す（sudo は systemctl restart sai-web だけ許可）"
sudo systemctl restart sai-web

# 作業中か（claude が動いているか）。見つけ方は目安（dev.sh の busy と同じ規則）。
busy() {
  docker exec "$1" pgrep -f '(^|/)claude( |$)' >/dev/null 2>&1
}

# コンテナ 1 つを新しいイメージで作り直す。失敗は 1 を返す（呼び出し側で続ける）。
recreate_one() {
  c=$1
  cur=$(docker inspect -f '{{.Image}}' "$c") || return 1
  if [ "$cur" = "$new" ]; then
    echo "$c: 最新"
    return 0
  fi
  if busy "$c"; then
    echo "$c: 作業中のため飛ばします（作業が終わってから deploy.sh をもう一度実行）"
    return 0
  fi
  if [ "$(docker inspect -f '{{.State.Running}}' "$c")" != "true" ]; then
    # 止まっているコンテナは消すだけにする（起動し直すと Web が新しいイメージで作る）
    echo "$c: 止まっているので消します（次に起動するとき新しいイメージで作ります）"
    docker rm -f "$c" >/dev/null || return 1
    return 0
  fi
  echo "$c: 作り直す"
  docker rm -f "$c" >/dev/null || return 1
  # containers.start が読む名前は sai- を除いた <利用者>-<プロジェクト>。値は sai-web.service と同じにする
  (cd "$APP/apps/web" && SAI_WEB_USERS_ROOT=/srv/sai/users SAI_IMAGE=sai-tree:latest PYTHONPATH=src .venv/bin/python -c \
    "import sys; from sai_web import containers; print(containers.start(sys.argv[1]))" "${c#sai-}") >/dev/null || return 1
}

# 動いているコンテナに block・template・tools・exec を配り直す（源の版にそろえる・違う物は .bak に退避）。
assets_one() {
  c=$1
  if busy "$c"; then
    echo "$c: 作業中のため飛ばします（作業が終わってから deploy.sh をもう一度実行）"
    return 0
  fi
  root=$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$c" | sed -n 's/^SAI_PROJECT_ROOT=//p') || return 1
  [ -n "$root" ] || return 1
  echo "$c: 部品を配り直す（$root）"
  docker exec -w "$root" -e SAI_PROJECT_ROOT="$root" "$c" sai-sci init --update --json >/dev/null || return 1
  # 時系列（段階 B）の ts/ があれば block ts-engineer も新しい版にそろえる
  docker exec -w "$root" -e SAI_PROJECT_ROOT="$root" "$c" sh -c '[ ! -d ts ] || sai-ts init --update --json' >/dev/null || return 1
}

echo "== 6/7 古いイメージのコンテナを作り直す（作業中は飛ばす）"
new=$(docker image inspect -f '{{.Id}}' sai-tree:latest)
for c in $(docker ps -a --filter label=sai.owner --format '{{.Names}}'); do
  recreate_one "$c" || echo "$c: 作り直しに失敗しました（ほかのコンテナは続けます）" >&2
done

echo "== 7/7 動いているコンテナに部品を配り直す（作業中は飛ばす）"
for c in $(docker ps --filter label=sai.owner --format '{{.Names}}'); do
  assets_one "$c" || echo "$c: 部品の配り直しに失敗しました（ほかのコンテナは続けます）" >&2
done

echo "反映しました。"
echo "作業中で飛ばしたコンテナは、作業が終わってから deploy.sh をもう一度実行してください。"
