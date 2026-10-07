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

echo "== 1/5 ソースを取り込む（git pull --ff-only）"
git pull --ff-only

echo "== 2/5 コンテナのイメージを作る（上限 60 分）"
timeout 3600 docker build -f apps/web/server/Dockerfile -t sai-tree:latest .

echo "== 3/5 apps/web の仮想環境を合わせる（uv sync）"
(cd apps/web && uv sync)

echo "== 4/5 sai-prep の入れ物を作る（apps/prep/dist/・GET /api/prep/download が配る）"
(cd apps/prep && uv build)

echo "== 5/5 Web を起動し直す（sudo は systemctl restart sai-web だけ許可）"
sudo systemctl restart sai-web

echo "反映しました。"
echo "古いイメージのコンテナを新しくするには、止まっていることを確かめてから docker rm し、"
echo "Projects の「起動」を押す（docs/server.md の 13）。実行中のエージェントがいるコンテナには触らない。"
