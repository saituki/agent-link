#!/bin/sh
# U08 AC2: `sai-in-container`（Web が CLI をコンテナの中で動かす中継）の検査。
#
# 偽の `docker` を PATH の先頭に置き、呼ばれた引数を記録して確かめる。
# 実物の Docker は使わない（AC3 で別に行う）。
#
#   sh apps/web/server/test_sai_in_container.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

bindir="$tmp/bin"
log="$tmp/docker.log"
mkdir -p "$bindir"
: > "$log"

# 偽の docker。`inspect` は State.Running を返し、それ以外は引数を記録する。
cat > "$bindir/docker" <<'FAKE'
#!/bin/sh
case "${1:-}" in
  inspect)
    if [ "${FAKE_DOCKER_RUNNING:-true}" = "true" ]; then echo true; else echo false; fi
    exit 0
    ;;
esac
printf '%s\n' "$*" >> "$SAI_TEST_LOG"
exit 0
FAKE
chmod +x "$bindir/docker"

# 中継を CLI の名前で呼べるようにする（本番の /opt/sai/bin と同じ形）。
for n in sai-link sai-paper sai-sci; do
  ln -s "$here/sai-in-container" "$bindir/$n"
done

PATH="$bindir:$PATH"
export PATH
SAI_TEST_LOG="$log"
export SAI_TEST_LOG

fail() {
  echo "NG: $1" >&2
  exit 1
}

# 1. 動いているコンテナへの中継（C7 の形）。
: > "$log"
SAI_PROJECT_ROOT=/srv/sai/users/alice/projects/alice-x SAI_ORIGIN=phone \
  sai-link run tree --json
expected='exec -i -w /srv/sai/users/alice/projects/alice-x -e SAI_PROJECT_ROOT -e SAI_ORIGIN sai-alice-x sai-link run tree --json'
actual=$(cat "$log")
[ "$actual" = "$expected" ] || fail "中継の引数が違う
  期待: $expected
  実際: $actual"

# 2. プロジェクトの要らない命令は使い捨てのコンテナで動かす（外への口を与えない）。
: > "$log"
env -u SAI_PROJECT_ROOT SAI_IMAGE=sai-tree:check sai-link schema --json
expected='run --rm -i --network none sai-tree:check sai-link schema --json'
actual=$(cat "$log")
[ "$actual" = "$expected" ] || fail "使い捨ての引数が違う
  期待: $expected
  実際: $actual"

# 3. 名前が正しくない（`..` の入った場所）→ 終了コード 2。
set +e
SAI_PROJECT_ROOT=/srv/sai/users/alice/projects/../x SAI_ORIGIN=phone \
  sai-link schema --json >/dev/null 2>&1
code=$?
set -e
[ "$code" = "2" ] || fail "`../x` の終了コードが $code（2 のはず）"

# 4. 名前が正しくない（使えない文字）→ 終了コード 2。
set +e
SAI_PROJECT_ROOT=/srv/sai/users/alice/projects/alice_x SAI_ORIGIN=phone \
  sai-link schema --json >/dev/null 2>&1
code=$?
set -e
[ "$code" = "2" ] || fail "`alice_x` の終了コードが $code（2 のはず）"

# 5. コンテナが動いていない → 終了コード 4。
set +e
FAKE_DOCKER_RUNNING=false \
SAI_PROJECT_ROOT=/srv/sai/users/alice/projects/alice-x SAI_ORIGIN=phone \
  sai-link schema --json >/dev/null 2>&1
code=$?
set -e
[ "$code" = "4" ] || fail "停止中の終了コードが $code（4 のはず）"

echo "OK: sai-in-container"
