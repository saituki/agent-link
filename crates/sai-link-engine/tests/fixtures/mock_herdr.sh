#!/usr/bin/env bash
#
# mock_herdr.sh — herdr CLI モックスクリプト
#
# 実際の herdr 環境がなくてもテストを実行できるようにするための
# モック実装です。以下のサブコマンドをシミュレートします:
#
#   pane list      — 固定の PaneListEnvelope JSON を出力
#   pane get       — 固定の PaneGetEnvelope JSON を出力（状態ファイルで agent_status 制御可能）
#   pane split     — 新規 pane_id をランダム生成して PaneSplitEnvelope JSON を出力
#   pane run       — 成功（exit 0, 出力なし）
#   pane send-text — 成功（exit 0, 出力なし、送信内容をログファイルに記録）
#   pane send-keys — 成功（exit 0, 出力なし）
#   --version      — "herdr 1.0.0" を出力
#
# ## 状態ファイルによる agent_status 制御
#
# `/tmp/mock_herdr_state_<pane_id>` ファイルにステータス文字列
# （idle / working / blocked / done）を書き込むと、次回
# `pane get <pane_id>` の応答の agent_status がその値になります。
# 状態ファイルが存在しない場合はデフォルト値（idle）が使用されます。
#
# ## 送信テキストログ
#
# `pane send-text <pane_id> <text>` で送信されたテキストは
# `/tmp/mock_herdr_sent_text_<pane_id>` に追記されます。
# テストコードから読み取ることで、送信内容を検証できます。
#
# 使用方法:
#   HERDR_BINARY=tests/fixtures/mock_herdr.sh cargo test
#

set -euo pipefail

# ---------------------------------------------------------------------------
# 固定のペイン情報
# ---------------------------------------------------------------------------
PANE_1_ID="pane_test_001"
PANE_2_ID="pane_test_002"

# pane 1 のステータスを状態ファイルから取得する関数
get_pane_1_status() {
    local state_file="/tmp/mock_herdr_state_${PANE_1_ID}"
    if [[ -f "$state_file" ]]; then
        cat "$state_file"
    else
        echo "idle"
    fi
}

# pane 2 のステータスを状態ファイルから取得する関数
get_pane_2_status() {
    local state_file="/tmp/mock_herdr_state_${PANE_2_ID}"
    if [[ -f "$state_file" ]]; then
        cat "$state_file"
    else
        echo "idle"
    fi
}

# pane 情報 JSON を取得する関数
get_pane_json_1() {
    local status
    status=$(get_pane_1_status)
    cat <<EOF
{"pane_id":"${PANE_1_ID}","workspace_id":"ws_test_001","tab_id":"tab_test_001","focused":true,"cwd":"/home/test/project","agent":"pi","agent_status":"${status}","revision":1}
EOF
}

get_pane_json_2() {
    local status
    status=$(get_pane_2_status)
    cat <<EOF
{"pane_id":"${PANE_2_ID}","workspace_id":"ws_test_001","tab_id":"tab_test_001","focused":false,"cwd":"/home/test/project/subdir","agent":null,"agent_status":"${status}","revision":2}
EOF
}

# ---------------------------------------------------------------------------
# メイン処理
# ---------------------------------------------------------------------------
main() {
    if [[ $# -eq 0 ]]; then
        echo '{"error":{"message":"missing subcommand"}}'
        exit 1
    fi

    local subcommand="$1"
    shift

    case "$subcommand" in
        --version)
            echo "herdr 1.0.0"
            exit 0
            ;;
        pane)
            handle_pane "$@"
            ;;
        *)
            echo "{\"error\":{\"message\":\"unknown subcommand: ${subcommand}\"}}"
            exit 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# pane サブコマンド処理
# ---------------------------------------------------------------------------
handle_pane() {
    if [[ $# -eq 0 ]]; then
        echo '{"error":{"message":"missing pane subcommand"}}'
        exit 1
    fi

    local action="$1"
    shift

    case "$action" in
        list)
            handle_pane_list "$@"
            ;;
        get)
            handle_pane_get "$@"
            ;;
        split)
            handle_pane_split "$@"
            ;;
        run)
            handle_pane_run "$@"
            ;;
        send-text)
            handle_pane_send_text "$@"
            ;;
        send-keys)
            handle_pane_send_keys "$@"
            ;;
        close)
            handle_pane_close "$@"
            ;;
        rename)
            handle_pane_rename "$@"
            ;;
        *)
            echo "{\"error\":{\"message\":\"unknown pane action: ${action}\"}}"
            exit 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# pane list
# ---------------------------------------------------------------------------
# 使用法: herdr pane list
# 出力: PaneListEnvelope JSON (2つのペイン)
handle_pane_list() {
    # 余分な引数がある場合はエラー
    if [[ $# -gt 0 ]]; then
        echo "{\"error\":{\"message\":\"unexpected arguments: $*\"}}"
        exit 1
    fi

    local pane1_json pane2_json
    pane1_json=$(get_pane_json_1)
    pane2_json=$(get_pane_json_2)

    cat <<ENVELOPE
{"result":{"panes":[${pane1_json},${pane2_json}]}}
ENVELOPE
    exit 0
}

# ---------------------------------------------------------------------------
# pane get
# ---------------------------------------------------------------------------
# 使用法: herdr pane get <pane_id>
# 出力: PaneGetEnvelope JSON（状態ファイルで agent_status 制御可能）
handle_pane_get() {
    if [[ $# -ne 1 ]]; then
        >&2 echo "{\"error\":{\"message\":\"usage: herdr pane get <pane_id>\"}}"
        exit 1
    fi

    local pane_id="$1"

    case "$pane_id" in
        "$PANE_1_ID")
            local pane_json
            pane_json=$(get_pane_json_1)
            cat <<ENVELOPE
{"result":{"pane":${pane_json}}}
ENVELOPE
            exit 0
            ;;
        "$PANE_2_ID")
            local pane_json
            pane_json=$(get_pane_json_2)
            cat <<ENVELOPE
{"result":{"pane":${pane_json}}}
ENVELOPE
            exit 0
            ;;
        *)
            # 状態ファイルが存在する場合、動的 pane_id として扱う
            local state_file="/tmp/mock_herdr_state_${pane_id}"
            local agent_status="idle"
            if [[ -f "$state_file" ]]; then
                agent_status=$(cat "$state_file")
            fi
            # pane not found → exit 1（存在しない pane_id のデフォルト動作）
            >&2 echo "{\"error\":{\"message\":\"pane not found: ${pane_id}\"}}"
            exit 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# pane split
# ---------------------------------------------------------------------------
# 使用法: herdr pane split <source_pane_id> --direction <right|down> --no-focus [--cwd <path>]
# 出力: PaneSplitEnvelope JSON (新規 pane_id をランダム生成)
handle_pane_split() {
    # 最低限の引数チェック
    if [[ $# -lt 3 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane split <source> --direction <right|down> --no-focus\"}}"
        exit 1
    fi

    local source_id="$1"
    shift

    local direction="right"
    local cwd=""

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --direction)
                shift
                if [[ $# -gt 0 ]]; then
                    direction="$1"
                    shift
                fi
                ;;
            --no-focus)
                shift
                ;;
            --cwd)
                shift
                if [[ $# -gt 0 ]]; then
                    cwd="$1"
                    shift
                fi
                ;;
            *)
                echo "{\"error\":{\"message\":\"unknown split option: $1\"}}"
                exit 1
                ;;
        esac
    done

    # 新規 pane_id をランダム生成
    local new_pane_id
    new_pane_id="pane_split_$(date +%s)_$$_${RANDOM}"

    # cwd が未設定の場合は source の cwd を引き継ぐ
    if [[ -z "$cwd" ]]; then
        cwd="/home/test/project"
    fi

    cat <<ENVELOPE
{"result":{"pane":{"pane_id":"${new_pane_id}","workspace_id":"ws_test_001","tab_id":"tab_test_001","focused":false,"cwd":"${cwd}","agent":null,"agent_status":"idle","revision":1}}}
ENVELOPE
    exit 0
}

# ---------------------------------------------------------------------------
# pane run
# ---------------------------------------------------------------------------
# 使用法: herdr pane run <pane_id> '<command>'
# 出力: なし（exit 0）
handle_pane_run() {
    if [[ $# -lt 2 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane run <pane_id> <command>\"}}"
        exit 1
    fi

    # 成功: 出力なし、exit 0
    exit 0
}

# ---------------------------------------------------------------------------
# pane send-text
# ---------------------------------------------------------------------------
# 使用法: herdr pane send-text <pane_id> '<text>'
# 出力: なし（exit 0、送信内容をログファイルに記録）
handle_pane_send_text() {
    if [[ $# -lt 2 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane send-text <pane_id> <text>\"}}"
        exit 1
    fi

    local pane_id="$1"
    shift

    # 残りの引数すべてをテキストとして結合
    local text="$*"

    # 送信テキストをログファイルに記録
    local log_file="/tmp/mock_herdr_sent_text_${pane_id}"
    echo "${text}" >> "$log_file"

    # 成功: exit 0
    exit 0
}

# ---------------------------------------------------------------------------
# pane send-keys
# ---------------------------------------------------------------------------
# 使用法: herdr pane send-keys <pane_id> <keys>
# 出力: なし（exit 0）
handle_pane_send_keys() {
    if [[ $# -lt 2 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane send-keys <pane_id> <keys>\"}}"
        exit 1
    fi

    # 成功: 出力なし、exit 0
    exit 0
}

# ---------------------------------------------------------------------------
# pane close
# ---------------------------------------------------------------------------
# 使用法: herdr pane close <pane_id>
# 出力: なし（exit 0）
handle_pane_close() {
    if [[ $# -ne 1 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane close <pane_id>\"}}"
        exit 1
    fi

    # 成功: 出力なし、exit 0
    exit 0
}

# ---------------------------------------------------------------------------
# pane rename
# ---------------------------------------------------------------------------
# 使用法: herdr pane rename <pane_id> <label>|--clear
# 出力: PaneGetEnvelope JSON (label フィールドを含む)
handle_pane_rename() {
    if [[ $# -lt 2 ]]; then
        echo "{\"error\":{\"message\":\"usage: herdr pane rename <pane_id> <label>|--clear\"}}"
        exit 1
    fi

    local pane_id="$1"
    shift

    # 動的 pane_id や固定 pane_id に対応
    local label_json="null"
    if [[ "$1" == "--clear" ]]; then
        label_json="null"
    else
        label_json="\"$1\""
    fi

    # 状態ファイルから動的 pane_id のステータスを取得
    local state_file="/tmp/mock_herdr_state_${pane_id}"
    local agent_status="idle"
    if [[ -f "$state_file" ]]; then
        agent_status=$(cat "$state_file")
    fi

    cat <<ENVELOPE
{"result":{"pane":{"pane_id":"${pane_id}","workspace_id":"ws_test_001","tab_id":"tab_test_001","focused":false,"cwd":"/home/test/project","agent":null,"agent_status":"${agent_status}","label":${label_json},"revision":1}}}
ENVELOPE
    exit 0
}

# ---------------------------------------------------------------------------
# エントリーポイント
# ---------------------------------------------------------------------------
main "$@"
