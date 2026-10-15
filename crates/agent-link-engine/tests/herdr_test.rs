//! # herdr 連携モジュールのテスト（sai-tree は headless だけ）
//!
//! sai-tree では `HerdrClient` は擬似 pane（`hl-` で始まる id）だけを受け付け、
//! herdr の実行ファイルは呼ばない。ここでは次を確かめる:
//!
//! - 純関数（`layout_for_n`・`shell_escape_single_quotes`）はそのまま
//! - `is_herdr_available()` は常に `false`
//! - 擬似 pane でない id への操作は「herdr は使えません」のエラーになる
//! - `rename_pane` / `clear_pane_label` は herdr を呼ばず `Ok` を返す

use agent_link_engine::herdr::{
    layout_for_n, shell_escape_single_quotes, HerdrClient, HerdrError, SplitDirection,
    MAX_BATCH_PANES,
};

// ---------------------------------------------------------------------------
// 純関数
// ---------------------------------------------------------------------------

/// `is_herdr_available()` は常に `false`（sai-tree は headless だけ）。
#[test]
fn test_herdr_available_is_always_false() {
    assert!(!HerdrClient::new().is_herdr_available());
    assert!(!agent_link_engine::herdr::is_herdr_available());
}

/// `layout_for_n` の均等レイアウト計算を検証する（U2・R4）。
#[test]
fn test_layout_for_n() {
    assert_eq!(layout_for_n(1), (1, 1));
    assert_eq!(layout_for_n(2), (2, 1));
    assert_eq!(layout_for_n(3), (3, 1));
    assert_eq!(layout_for_n(4), (4, 1));
    assert_eq!(layout_for_n(5), (3, 2));
    assert_eq!(layout_for_n(6), (3, 2));
    assert_eq!(layout_for_n(7), (4, 2));
    assert_eq!(layout_for_n(8), (4, 2));
    assert_eq!(layout_for_n(0), (0, 0));
}

/// バッチ同時実行の pane 上限が 8 であることを確認する（U2・R5）。
#[test]
fn test_max_batch_panes_is_eight() {
    assert_eq!(MAX_BATCH_PANES, 8);
}

/// `shell_escape_single_quotes` のエスケープを確認。
#[test]
fn test_shell_escape() {
    assert_eq!(shell_escape_single_quotes("hello"), "'hello'");
    assert_eq!(shell_escape_single_quotes("hello world"), "'hello world'");
    assert_eq!(shell_escape_single_quotes(""), "''");
    assert_eq!(shell_escape_single_quotes("it's"), "'it'\\''s'");
    assert_eq!(shell_escape_single_quotes("a'b'c"), "'a'\\''b'\\''c'");
}

/// `SplitDirection` の Display 実装を確認。
#[test]
fn test_split_direction_display() {
    assert_eq!(SplitDirection::Right.to_string(), "right");
    assert_eq!(SplitDirection::Down.to_string(), "down");
}

/// `HerdrClient` が `Default` を実装していることを確認。
#[test]
fn test_client_default_impl() {
    let client: HerdrClient = Default::default();
    let _ = client;
}

// ---------------------------------------------------------------------------
// 擬似 pane 以外は herdr を呼ばずエラー
// ---------------------------------------------------------------------------

/// エラーが「headless」であることを確かめる。
fn assert_disabled<T: std::fmt::Debug>(result: Result<T, HerdrError>) {
    match result {
        Err(HerdrError::ApiError { message }) => {
            assert!(
                message.contains("headless"),
                "headless の案内が出るべき: {}",
                message
            );
        }
        other => panic!("ApiError を期待: {:?}", other),
    }
}

#[test]
fn test_non_headless_operations_are_disabled() {
    let client = HerdrClient::new();
    assert_disabled(client.run_in_pane("pane_test_001", "echo hi"));
    assert_disabled(client.send_text("pane_test_001", "hello"));
    assert_disabled(client.send_keys("pane_test_001", "Enter"));
    assert_disabled(client.get_pane_status("pane_test_001"));
    assert_disabled(client.list_panes());
    assert_disabled(client.split_pane(SplitDirection::Right, None));
    assert_disabled(client.split_pane_from("pane_test_001", SplitDirection::Right, None));
    assert_disabled(client.split_pane_in_tab("tab_1", SplitDirection::Right, None));
    assert_disabled(client.create_tab(None, None));
    assert_disabled(client.workspace_list());
    assert_disabled(client.list_panes_in("ws_1"));
    assert_disabled(client.snapshot_layout("tab_1"));
    assert_disabled(client.close_tab("tab_1"));
    assert_disabled(client.close_pane("pane_test_001"));
    assert_disabled(client.read_pane_output("pane_test_001", 50));
    assert_eq!(client.find_workspace_for_project(std::path::Path::new("/tmp")), None);
}

/// ラベル操作は herdr を呼ばず `Ok` を返す（合成 `PaneInfo`）。
#[test]
fn test_rename_and_clear_are_ok() {
    let client = HerdrClient::new();
    let renamed = client
        .rename_pane("pane_test_001", "my-label")
        .expect("rename は Ok");
    assert_eq!(renamed.pane_id, "pane_test_001");
    assert_eq!(renamed.label.as_deref(), Some("my-label"));

    let cleared = client
        .clear_pane_label("pane_test_001")
        .expect("clear は Ok");
    assert_eq!(cleared.pane_id, "pane_test_001");
    assert_eq!(cleared.label, None);
}
