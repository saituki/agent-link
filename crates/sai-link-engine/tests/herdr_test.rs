//! # herdr 連携モジュールの単体テスト
//!
//! 実際の herdr 環境に依存せず、`tests/fixtures/mock_herdr.sh` を使用して
//! テストを実行します。モックスクリプトは以下の herdr CLI サブコマンドを
//! シミュレートします:
//!
//! - `pane list` / `pane get` / `pane split` / `pane run` / `pane send-text` / `pane send-keys`
//!
//! テスト実行前に環境変数 `MOCK_HERDR_BINARY` にモックスクリプトの
//! 絶対パスが設定されていることを確認します。

use sai_link_engine::herdr::{
    layout_for_n, shell_escape_single_quotes, HerdrClient, HerdrError, PaneInfo, SplitDirection,
    TabInfo, MAX_BATCH_PANES,
};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// ヘルパー
// ---------------------------------------------------------------------------

/// モック herdr スクリプトの絶対パスを取得します。
fn mock_herdr_path() -> PathBuf {
    // テストはプロジェクトルートから実行される
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_herdr.sh");
    assert!(
        path.exists(),
        "モック herdr スクリプトが見つかりません: {}",
        path.display()
    );
    path
}

/// モック herdr を使用する `HerdrClient` を作成します。
fn mock_client() -> HerdrClient {
    HerdrClient::with_binary(mock_herdr_path().to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// テストケース
// ---------------------------------------------------------------------------

/// `is_herdr_available()` がモック herdr で `true` を返すことを確認。
#[test]
fn test_herdr_available() {
    let client = mock_client();
    assert!(
        client.is_herdr_available(),
        "モック herdr が利用可能であるべき"
    );
}

/// スタンドアロン版 `is_herdr_available()` のテスト。
#[test]
fn test_herdr_available_standalone() {
    // スタンドアロン版はデフォルトパス "herdr" を使用するため、
    // モック環境では false になる想定。
    let available = sai_link_engine::herdr::is_herdr_available();
    // 実際の herdr がインストールされているかどうかで結果が変わるため、
    // アサーションは行わない（エラーにならないことだけ確認）。
    let _ = available;
}

/// `list_panes()` が正しく複数のペインを返すことを確認。
#[test]
fn test_list_panes() {
    let client = mock_client();
    let panes = client.list_panes().expect("ペイン一覧の取得に成功するべき");

    assert_eq!(panes.len(), 2, "2つのペインが返されるべき");

    // 1つ目のペイン検証
    assert_eq!(panes[0].pane_id, "pane_test_001");
    assert!(panes[0].focused, "1つ目のペインはフォーカスされているべき");
    assert_eq!(panes[0].agent_status, "idle");
    assert_eq!(panes[0].revision, 1);
    assert_eq!(panes[0].agent.as_deref(), Some("pi"));

    // 2つ目のペイン検証
    assert_eq!(panes[1].pane_id, "pane_test_002");
    assert!(
        !panes[1].focused,
        "2つ目のペインはフォーカスされていないべき"
    );
    assert_eq!(panes[1].cwd.as_deref(), Some("/home/test/project/subdir"));
    assert!(
        panes[1].agent.is_none(),
        "2つ目のペインは agent が null であるべき"
    );
}

/// `split_pane(Right, None)` が新規 `PaneInfo` を返すことを確認。
#[test]
fn test_split_pane_right() {
    let client = mock_client();
    let pane = client
        .split_pane(SplitDirection::Right, None)
        .expect("ペイン分割に成功するべき");

    assert!(
        pane.pane_id.starts_with("pane_split_"),
        "新規 pane_id は 'pane_split_' で始まるべき: {}",
        pane.pane_id
    );
    assert!(
        !pane.focused,
        "分割後の新規ペインはフォーカスされていないべき"
    );
    assert_eq!(pane.agent_status, "idle");
    assert_eq!(pane.workspace_id, "ws_test_001");
    // cwd が指定されていないので、デフォルト値が設定される
    assert_eq!(pane.cwd.as_deref(), Some("/home/test/project"));
}

/// `split_pane(Down, Some(path))` がカスタム cwd で動作することを確認。
#[test]
fn test_split_pane_down_with_cwd() {
    let client = mock_client();
    let pane = client
        .split_pane(SplitDirection::Down, Some("/custom/path"))
        .expect("ペイン分割（Down, cwd指定）に成功するべき");

    assert!(
        pane.pane_id.starts_with("pane_split_"),
        "新規 pane_id は 'pane_split_' で始まるべき"
    );
    assert!(!pane.focused);
    assert_eq!(
        pane.cwd.as_deref(),
        Some("/custom/path"),
        "指定した cwd が設定されているべき"
    );
}

/// `run_in_pane()` がエラーなく完了することを確認。
#[test]
fn test_run_in_pane() {
    let client = mock_client();
    let result = client.run_in_pane("pane_test_001", "echo hello");
    assert!(result.is_ok(), "run_in_pane は成功するべき");
}

/// `run_in_pane()` でシングルクォートを含むコマンドが正しく処理されることを確認。
#[test]
fn test_run_in_pane_with_single_quote() {
    let client = mock_client();
    // シングルクォートを含むコマンド
    let result = client.run_in_pane("pane_test_001", "echo 'hello world'");
    assert!(
        result.is_ok(),
        "シングルクォートを含むコマンドも成功するべき"
    );
}

/// `send_text()` がエラーなく完了することを確認。
#[test]
fn test_send_text() {
    let client = mock_client();
    let result = client.send_text("pane_test_001", "Hello, world!");
    assert!(result.is_ok(), "send_text は成功するべき");
}

/// `send_text()` でシングルクォートを含むテキストが正しく処理されることを確認。
#[test]
fn test_send_text_with_single_quote() {
    let client = mock_client();
    let result = client.send_text("pane_test_001", "it's a test");
    assert!(
        result.is_ok(),
        "シングルクォートを含むテキストも成功するべき"
    );
}

/// `send_keys()` がエラーなく完了することを確認。
#[test]
fn test_send_keys() {
    let client = mock_client();
    let result = client.send_keys("pane_test_001", "Enter");
    assert!(result.is_ok(), "send_keys は成功するべき");

    let result = client.send_keys("pane_test_001", "Ctrl+c");
    assert!(result.is_ok(), "send_keys Ctrl+c も成功するべき");
}

/// `get_pane_status()` が正しい `PaneInfo` を返すことを確認。
#[test]
fn test_get_pane_status() {
    let client = mock_client();
    let pane = client
        .get_pane_status("pane_test_001")
        .expect("ペイン情報の取得に成功するべき");

    assert_eq!(pane.pane_id, "pane_test_001");
    assert!(pane.focused);
    assert_eq!(pane.agent_status, "idle");
    assert_eq!(pane.agent.as_deref(), Some("pi"));
    assert_eq!(pane.revision, 1);
    assert_eq!(pane.cwd.as_deref(), Some("/home/test/project"));

    let pane2 = client
        .get_pane_status("pane_test_002")
        .expect("2つ目のペイン情報の取得に成功するべき");

    assert_eq!(pane2.pane_id, "pane_test_002");
    assert!(!pane2.focused);
    assert!(pane2.agent.is_none());
    assert_eq!(pane2.cwd.as_deref(), Some("/home/test/project/subdir"));
}

/// `shell_escape_single_quotes` のエスケープが
/// TypeScript 版 `shellEscape()` と等価であることを確認。
///
/// TypeScript 版: シングルクォート `'` を `'\"'\"'` に置換し、全体を `'...'` で囲む。
/// Rust 版: シングルクォート `'` を `'\''` に置換し、全体を `'...'` で囲む。
///
/// 両者は機能的に等価（どちらもシェルで正しく解釈される）。
#[test]
fn test_shell_escape() {
    // エスケープ不要の文字列
    assert_eq!(shell_escape_single_quotes("hello"), "'hello'");
    assert_eq!(shell_escape_single_quotes("hello world"), "'hello world'");
    assert_eq!(shell_escape_single_quotes(""), "''");
    assert_eq!(shell_escape_single_quotes("42"), "'42'");

    // シングルクォートを含む文字列
    // TypeScript 版: "'it'\"'\"'s'" → シェルで "it's" として解釈される
    // Rust 版:      "'it'\\''s'" → シェルで "it's" として解釈される
    // 両者は異なるエスケープ方法（TS: "\"'\"'", Rust: "\\''"）だが
    // シェル上での解釈結果は同じ。
    assert_eq!(shell_escape_single_quotes("it's"), "'it'\\''s'");
    assert_eq!(shell_escape_single_quotes("'"), "''\\'''");

    // 複数のシングルクォート
    assert_eq!(shell_escape_single_quotes("a'b'c"), "'a'\\''b'\\''c'");
}

/// `HerdrClient` がデフォルトパス（実際の "herdr"）で利用不可の場合、
/// 適切にエラーが返ることを確認。
#[test]
fn test_herdr_not_found_error() {
    // 存在しないパスを指定
    let client = HerdrClient::with_binary("/nonexistent/herdr_binary");

    // is_herdr_available は false を返す
    assert!(!client.is_herdr_available());

    // list_panes は HerdrNotFound または IoError を返す
    let result = client.list_panes();
    match result {
        Err(HerdrError::IoError(_)) => {
            // IoError も許容（バイナリが見つからない場合の OS エラー）
        }
        Err(HerdrError::HerdrNotFound) => {
            // モックが exit 127 を返す場合もある
        }
        other => {
            panic!("期待されるエラー型ではありません: {:?}", other);
        }
    }
}

/// モックを使用して、`get_pane_status()` が存在しないペインに対して
/// エラーを返すことを確認。
///
/// 実際の herdr CLI でも exit code 1 で stderr にエラーメッセージが出力される。
#[test]
fn test_get_pane_status_not_found() {
    let client = mock_client();
    let result = client.get_pane_status("nonexistent_pane");
    match result {
        Err(HerdrError::HerdrCliError {
            exit_code: Some(code),
            stderr,
        }) => {
            assert_eq!(code, 1, "存在しないペインの取得は exit code 1 を返すべき");
            assert!(
                stderr.contains("pane not found"),
                "stderr に 'pane not found' が含まれるべき: {}",
                stderr
            );
        }
        other => {
            panic!(
                "HerdrCliError(exit_code=1) が期待されるが {:?} が返された",
                other
            );
        }
    }
}

/// 全ペイン一覧と個別取得の結果が一貫していることを確認。
#[test]
fn test_list_and_get_consistency() {
    let client = mock_client();
    let panes = client.list_panes().expect("ペイン一覧取得成功");

    for pane in &panes {
        let fetched = client
            .get_pane_status(&pane.pane_id)
            .unwrap_or_else(|_| panic!("ペイン {} の取得に成功するべき", pane.pane_id));
        assert_eq!(fetched.pane_id, pane.pane_id);
        assert_eq!(fetched.focused, pane.focused);
        assert_eq!(fetched.agent_status, pane.agent_status);
    }
}

/// `HerdrClient` が `Default` トレイトを実装していることを確認（`Default::default()` で作成可能）。
#[test]
fn test_client_default_impl() {
    let client: HerdrClient = Default::default();
    // デフォルトは "herdr" パス
    let _ = client;
}

/// `PaneInfo` が Debug トレイトを実装していることを確認（エラー出力用）。
#[test]
fn test_pane_info_debug() {
    let client = mock_client();
    let pane = client
        .get_pane_status("pane_test_001")
        .expect("ペイン情報の取得成功");
    // Debug 出力がパニックしないことを確認
    let debug_str = format!("{:?}", pane);
    assert!(!debug_str.is_empty());
    assert!(debug_str.contains("pane_test_001"));
}

/// `SplitDirection` の Display 実装を確認。
#[test]
fn test_split_direction_display() {
    assert_eq!(SplitDirection::Right.to_string(), "right");
    assert_eq!(SplitDirection::Down.to_string(), "down");
}

/// `rename_pane()` がペインにラベルを設定できることを確認。
#[test]
fn test_rename_pane() {
    let client = mock_client();
    let result = client.rename_pane("pane_test_001", "my-custom-label");
    assert!(
        result.is_ok(),
        "ラベル設定に成功するべき: {:?}",
        result.err()
    );

    let pane = result.unwrap();
    assert_eq!(pane.pane_id, "pane_test_001");
    assert_eq!(pane.label.as_deref(), Some("my-custom-label"));
}

/// `rename_pane()` で動的 pane_id にもラベルを設定できることを確認。
#[test]
fn test_rename_pane_dynamic_id() {
    let client = mock_client();
    let result = client.rename_pane("pane_dynamic_999", "dynamic-label");
    assert!(result.is_ok(), "動的 pane_id でも成功するべき");

    let pane = result.unwrap();
    assert_eq!(pane.label.as_deref(), Some("dynamic-label"));
}

/// `clear_pane_label()` がペインのラベルをクリアできることを確認。
#[test]
fn test_clear_pane_label() {
    let client = mock_client();

    // まずラベルを設定
    let _ = client.rename_pane("pane_test_001", "temporary-label");

    // ラベルをクリア
    let result = client.clear_pane_label("pane_test_001");
    assert!(result.is_ok(), "ラベルクリアに成功するべき");

    let pane = result.unwrap();
    assert_eq!(pane.label, None, "ラベルが None になるべき");
}

/// herdr CLI がハングした場合にタイムアウトでエラーを返すことを確認。
///
/// ペイン削除直後などに herdr CLI がハングするケースがあり、
/// デーモンの実行パイプラインを保護するためのタイムアウト機構のテスト。
#[test]
fn test_command_timeout() {
    // スリープするモックスクリプトを作成（タイムアウト 30 秒より長くスリープ）
    // 並列テストでの固定ファイル名の競合を避けるため PID を付与する
    // `exec sleep` で bash を置換し、kill 時に孤立プロセスを残さない（フレーキー対策）
    let script_path =
        std::env::temp_dir().join(format!("mock_herdr_slow_{}.sh", std::process::id()));
    std::fs::write(&script_path, "#!/usr/bin/env bash\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(
        &script_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let client = HerdrClient::with_binary(script_path.to_string_lossy().to_string());
    let result = client.list_panes();
    assert!(
        result.is_err(),
        "ハングする herdr CLI はタイムアウトエラーになるべき"
    );
    match result.unwrap_err() {
        HerdrError::HerdrTimeout { .. } => {}
        // 環境起因（Text file busy など）でモックが起動できない場合も許容する
        HerdrError::IoError(_) => {}
        other => panic!("HerdrTimeout を期待: {:?}", other),
    }

    let _ = std::fs::remove_file(&script_path);
}

// --------------------------------------------------------------------------
// U2・R4/R5: タブ作成・タブ内均等分割（pane 総数の事前カウントに使用）
// --------------------------------------------------------------------------

/// `layout_for_n` の均等レイアウト計算を検証する（U2・R4）。
///
/// 実際の実装（`layout_for_wip`）の計算式:
/// - `rows = if n <= 4 { 1 } else { 2 }`（最大 2 行。n>4 のときのみ 2 行目を使用）
/// - `cols = if n <= 4 { n } else { ceil(n/2) }`（列数）
#[test]
fn test_layout_for_n() {
    assert_eq!(layout_for_n(1), (1, 1));
    assert_eq!(layout_for_n(2), (2, 1));
    assert_eq!(layout_for_n(3), (3, 1));
    assert_eq!(layout_for_n(4), (4, 1));
    assert_eq!(layout_for_n(5), (3, 2)); // 3列 x 2行
    assert_eq!(layout_for_n(6), (3, 2));
    assert_eq!(layout_for_n(7), (4, 2));
    assert_eq!(layout_for_n(8), (4, 2)); // 縦2 x 横4 = 8 分割
    assert_eq!(layout_for_n(0), (0, 0));
}

/// バッチ同時実行の pane 上限が 8 であることを確認する（U2・R5）。
#[test]
fn test_max_batch_panes_is_eight() {
    assert_eq!(MAX_BATCH_PANES, 8);
}

/// タブ操作対応のモック herdr スクリプトを一時ディレクトリに生成する。
///
/// 並列テストでの固定ファイル名の競合を避けるため、状態ディレクトリを
/// スクリプト内に埋め込み、呼び出しログ（`calls`）と pane→tab 対応
/// （`panes/<pane_id>`）を記録する。
///
/// 対応コマンド: `--version` / `tab create` / `pane list` / `pane split` / `pane get`
///
/// # 戻り値
///
/// (スクリプトパス, 状態ディレクトリ)
fn write_tab_mock(uniq: &str) -> (PathBuf, PathBuf) {
    let state = std::env::temp_dir().join(format!("mock_herdr_tab_state_{}", uniq));
    let _ = std::fs::create_dir_all(state.join("panes"));
    let script = std::env::temp_dir().join(format!("mock_herdr_tab_{}.sh", uniq));

    // 注意: 生文字列 + format! のため bash の ${var} は {{var}} と書く。
    // JSON のダブルクォートはそのまま（raw 文字列では \" にならない）。
    let state_display = state.display();
    let body = format!(
        r##"#!/usr/bin/env bash
set -euo pipefail

STATE='{state_display}'
mkdir -p "$STATE/panes"

log() {{ echo "$*" >> "$STATE/calls"; }}

next_seq() {{
    if [[ -f "$STATE/seq" ]]; then cat "$STATE/seq"; else echo 0; fi
}}
bump_seq() {{
    local s=$(next_seq)
    echo $((s+1)) > "$STATE/seq"
    echo "$s"
}}

main() {{
    local subcommand="$1"
    shift
    case "$subcommand" in
        --version)
            echo "herdr 1.0.0"
            exit 0
            ;;
        tab)
            local action="$1"
            shift
            case "$action" in
                create)
                    local s=$(bump_seq)
                    local tab="tab_${{s}}"
                    local pane="ptab_${{s}}"
                    echo "$tab" > "$STATE/panes/$pane"
                    for a in "$@"; do log "tab create arg $a"; done
                    log "tab create"
                    cat <<ENVELOPE
{{"id":"cli:tab:create","result":{{"root_pane":{{"pane_id":"$pane","workspace_id":"ws","tab_id":"$tab","focused":false,"cwd":"/tmp","agent":null,"agent_status":"unknown","revision":0}},"tab":{{"tab_id":"$tab","workspace_id":"ws","label":"$tab","number":$s,"pane_count":1,"focused":false,"agent_status":"unknown"}}}},"type":"tab_created"}}
ENVELOPE
                    exit 0
                    ;;
                *)
                    echo "{{\"error\":{{\"message\":\"unknown tab action: $action\"}}}}"
                    exit 1
                    ;;
            esac
            ;;
        pane)
            local action="$1"
            shift
            case "$action" in
                list)
                    local entries=""
                    for f in "$STATE"/panes/*; do
                        [[ -e "$f" ]] || continue
                        local pid="$(basename "$f")"
                        local tab="$(cat "$f")"
                        entries="$entries{{\"pane_id\":\"$pid\",\"workspace_id\":\"ws\",\"tab_id\":\"$tab\",\"focused\":false,\"cwd\":\"/tmp\",\"agent\":null,\"agent_status\":\"idle\",\"revision\":0}},"
                    done
                    entries="${{entries%,}}"
                    cat <<ENVELOPE
{{"result":{{"panes":[$entries]}}}}
ENVELOPE
                    exit 0
                    ;;
                split)
                    local source="$1"
                    shift
                    local direction="right"
                    local cwd=""
                    while [[ $# -gt 0 ]]; do
                        case "$1" in
                            --direction)
                                shift
                                [[ $# -gt 0 ]] && direction="$1" && shift
                                ;;
                            --no-focus)
                                shift
                                ;;
                            --cwd)
                                shift
                                [[ $# -gt 0 ]] && cwd="$1" && shift
                                ;;
                            *)
                                echo "{{\"error\":{{\"message\":\"unknown split option: $1\"}}}}" >&2
                                exit 1
                                ;;
                        esac
                    done
                    local tab
                    if [[ -f "$STATE/panes/$source" ]]; then
                        tab="$(cat "$STATE/panes/$source")"
                    else
                        tab="tab_unknown"
                    fi
                    local s=$(bump_seq)
                    local pane="psplit_${{s}}"
                    echo "$tab" > "$STATE/panes/$pane"
                    log "pane split $direction"
                    cat <<ENVELOPE
{{"result":{{"pane":{{"pane_id":"$pane","workspace_id":"ws","tab_id":"$tab","focused":false,"cwd":"${{cwd:-/tmp}}","agent":null,"agent_status":"idle","revision":0}}}}}}
ENVELOPE
                    exit 0
                    ;;
                get)
                    local pane="$1"
                    if [[ -f "$STATE/panes/$pane" ]]; then
                        local tab="$(cat "$STATE/panes/$pane")"
                    else
                        echo "{{\"error\":{{\"message\":\"pane not found: $pane\"}}}}" >&2
                        exit 1
                    fi
                    cat <<ENVELOPE
{{"result":{{"pane":{{"pane_id":"$pane","workspace_id":"ws","tab_id":"$tab","focused":false,"cwd":"/tmp","agent":null,"agent_status":"idle","revision":0}}}}}}
ENVELOPE
                    exit 0
                    ;;
                *)
                    echo "{{\"error\":{{\"message\":\"unknown pane action: $action\"}}}}"
                    exit 1
                    ;;
            esac
            ;;
        *)
            echo "{{\"error\":{{\"message\":\"unknown subcommand: $subcommand\"}}}}"
            exit 1
            ;;
    esac
}}

main "$@"
"##
    );
    // 一時ファイルに書き込んでからリネームする（write 直後の exec での
    // ETXTBSY（Text file busy）を回避する）。
    let script_tmp = script.with_extension("sh.tmp");
    std::fs::write(&script_tmp, body).unwrap();
    std::fs::set_permissions(
        &script_tmp,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::fs::rename(&script_tmp, &script).unwrap();

    (script, state)
}

/// ユニークなモック識別子を生成する（並列テストでの競合回避）。
fn unique_mock_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}_{}_{}",
        prefix,
        nanos,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// `create_tab()` が `TabInfo` とルート `PaneInfo` を返すことを確認する（U2・R4 案X）。
#[test]
fn test_create_tab_returns_tab_info() {
    let uniq = unique_mock_id("ctab");
    let (script, state) = write_tab_mock(&uniq);
    let client = HerdrClient::with_binary(script.to_string_lossy().to_string());

    let (tab, root_pane): (TabInfo, PaneInfo) =
        client.create_tab(None, None).expect("タブ作成に成功するべき");
    assert!(!tab.tab_id.is_empty(), "tab_id が設定されるべき");
    assert_eq!(tab.workspace_id, "ws");
    assert_eq!(tab.pane_count, 1, "作成直後のタブは pane 1 枚");
    assert_eq!(tab.label.as_deref(), Some(tab.tab_id.as_str()));
    assert!(!root_pane.pane_id.is_empty(), "ルート pane が返されるべき");

    // 呼び出しログ: tab create が 1 回（workspace 未指定 → 従来動作）
    let calls = std::fs::read_to_string(state.join("calls")).unwrap();
    assert_eq!(calls.lines().filter(|l| *l == "tab create").count(), 1);

    let _ = std::fs::remove_dir_all(&state);
    let _ = std::fs::remove_file(&script);
}

/// `create_tab(Some(workspace_id))` が `--workspace <id>` を herdr CLI に渡すことを確認する（U11・R23）。
/// 別 space で herdr を操作中でも、メインブロックのプロジェクト space へタブが作成されるようにする。
#[test]
fn test_create_tab_passes_workspace() {
    let uniq = unique_mock_id("ctabws");
    let (script, state) = write_tab_mock(&uniq);
    let client = HerdrClient::with_binary(script.to_string_lossy().to_string());

    let (tab, _root_pane): (TabInfo, PaneInfo) = client
        .create_tab(Some("ws_main_block"), None)
        .expect("タブ作成に成功するべき");
    assert!(!tab.tab_id.is_empty());

    // 呼び出しログに `--workspace ws_main_block` が含まれること
    let calls = std::fs::read_to_string(state.join("calls")).unwrap();
    let has_ws = calls.lines().any(|l| l == "tab create arg --workspace");
    let has_ws_value = calls.lines().any(|l| l == "tab create arg ws_main_block");
    assert!(has_ws, "--workspace 引数が渡されるべき (R23)");
    assert!(has_ws_value, "workspace ID の値が渡されるべき (R23)");

    let _ = std::fs::remove_dir_all(&state);
    let _ = std::fs::remove_file(&script);
}

/// `split_pane_in_tab()` がタブ内で分割し、分割先 pane の `tab_id` が
/// 元のタブと同一になることを確認する（U2・R4）。
#[test]
fn test_split_pane_in_tab_same_tab_id() {
    let uniq = unique_mock_id("sptab");
    let (script, state) = write_tab_mock(&uniq);
    let client = HerdrClient::with_binary(script.to_string_lossy().to_string());

    let (tab, _root_pane): (TabInfo, PaneInfo) =
        client.create_tab(None, None).expect("タブ作成に成功するべき");
    let tab_id = tab.tab_id.clone();

    // n=3: タブ 1 枚 + 分割 2 回 = pane 3 枚（同一タブ）
    let pane2 = client
        .split_pane_in_tab(&tab_id, SplitDirection::Right, None)
        .expect("タブ内分割 Right に成功するべき");
    assert_eq!(pane2.tab_id, tab_id, "分割先 pane は同一タブに属するべき");

    let pane3 = client
        .split_pane_in_tab(&tab_id, SplitDirection::Down, Some("/custom/path"))
        .expect("タブ内分割 Down に成功するべき");
    assert_eq!(pane3.tab_id, tab_id, "2回目の分割も同一タブに属するべき");
    assert_eq!(
        pane3.cwd.as_deref(),
        Some("/custom/path"),
        "指定した cwd が設定されるべき"
    );

    // pane list の全 pane が同一タブに属する
    let panes = client.list_panes().expect("ペイン一覧取得成功");
    assert_eq!(panes.len(), 3, "tab create 1 回 + split 2 回 = 3 pane");
    for p in &panes {
        assert_eq!(p.tab_id, tab_id, "全 pane が同一タブに属するべき (AE3)");
    }

    // 呼び出しログ: tab create 1 回 + pane split が n-1 = 2 回
    let calls = std::fs::read_to_string(state.join("calls")).unwrap();
    let tab_creates = calls.lines().filter(|l| *l == "tab create").count();
    let splits = calls
        .lines()
        .filter(|l| l.starts_with("pane split"))
        .count();
    assert_eq!(tab_creates, 1, "tab create は 1 回だけ呼ばれるべき");
    assert_eq!(splits, 2, "pane split は n-1 = 2 回呼ばれるべき");

    let _ = std::fs::remove_dir_all(&state);
    let _ = std::fs::remove_file(&script);
}

/// 存在しないタブに対する `split_pane_in_tab()` がエラーになることを確認する。
#[test]
fn test_split_pane_in_tab_unknown_tab() {
    let uniq = unique_mock_id("uktab");
    let (script, _state) = write_tab_mock(&uniq);
    let client = HerdrClient::with_binary(script.to_string_lossy().to_string());

    // タブを作成せずに分割 → タブ内 pane が見つからずエラー
    let result = client.split_pane_in_tab("tab_nonexistent", SplitDirection::Right, None);
    match result {
        Err(HerdrError::ApiError { message }) => {
            assert!(
                message.contains("tab_nonexistent"),
                "エラーにタブ ID が含まれるべき: {}",
                message
            );
        }
        other => panic!("ApiError を期待: {:?}", other),
    }

    let _ = std::fs::remove_file(&script);
}
