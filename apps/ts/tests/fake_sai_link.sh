#!/bin/sh
# 偽の sai-link（テスト用）。SAI_LINK_BIN で差し替える。本物の sai-link は呼ばない。
#
# $FAKE_LINK_DIR/calls/NNNN に引数（1 行に 1 つ）、NNNN.stdin に標準入力を残す。
# 応答は「先頭 2 語」を鍵にする（例: card create → card_create）。
#   $FAKE_LINK_DIR/out/<鍵>.json  標準出力（無ければ {}）
#   $FAKE_LINK_DIR/out/<鍵>.rc    終了コード（無ければ 0）。1 回使ったら消す
#   $FAKE_LINK_DIR/out/<鍵>.err   標準エラー（無ければ空）
D="$FAKE_LINK_DIR"
mkdir -p "$D/calls" "$D/out"
n=$(cat "$D/n" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$D/n"
id=$(printf '%04d' "$n")
printf '%s\n' "$@" > "$D/calls/$id"
cat > "$D/calls/$id.stdin"
key="${1}_${2}"
if [ -f "$D/out/$key.err" ]; then cat "$D/out/$key.err" >&2; fi
if [ -f "$D/out/$key.json" ]; then cat "$D/out/$key.json"; else echo '{}'; fi
rc=0
if [ -f "$D/out/$key.rc" ]; then rc=$(cat "$D/out/$key.rc"); rm -f "$D/out/$key.rc"; fi
exit "$rc"
