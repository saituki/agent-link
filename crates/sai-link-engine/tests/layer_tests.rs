//! `layer`（research-tree の層）の統合テスト（U01）。
//!
//! ブロック定義の frontmatter に `layer: root / seed / branch` を書くと
//! `block_metadata.layer` に保存される。3 つ以外の値は登録がスキップされる
//! （検査のエラー）。

use sai_link_engine::block_config::BlockConfig;
use sai_link_engine::db::SaiStore;
use sai_link_engine::orchestrator::upsert_block_metadata;
use sai_link_engine::validation::validate;

/// 層だけを変えた有効なブロック定義を作るヘルパー。
fn config_with_layer(layer: Option<&str>) -> BlockConfig {
    BlockConfig {
        name: "exp-seed".to_string(),
        description: Some("テスト用のブロック".to_string()),
        model: Some("anthropic/claude-sonnet-4-5".to_string()),
        provider: None,
        thinking: None,
        tools: None,
        extensions: None,
        skills: None,
        subblocks: None,
        timeout: None,
        worktree: None,
        enable: None,
        category: None,
        tag: None,
        round: None,
        layer: layer.map(|s| s.to_string()),
        templates: None,
        context: None,
        id: Some("exp-seed".to_string()),
    }
}

/// `layer: seed` を書いたブロックを登録すると `block_metadata.layer` が `seed` になること。
#[test]
fn upsert_block_metadata_saves_layer() {
    let store = SaiStore::open_in_memory().unwrap();
    let path = std::path::Path::new(".sai/blocks/exp-seed.md");

    let id =
        upsert_block_metadata(&store, &config_with_layer(Some("seed")), path).expect("登録できる");
    let block = store.get_block_by_id(id).unwrap();
    assert_eq!(block.layer.as_deref(), Some("seed"));
    assert_eq!(block.name, "exp-seed");

    // 層を書かなければ NULL（図に出さない）
    let id2 = upsert_block_metadata(&store, &config_with_layer(None), path).expect("登録できる");
    assert_eq!(store.get_block_by_id(id2).unwrap().layer, None);
}

/// `layer: foo` は検査のエラーで登録されないこと。
#[test]
fn upsert_block_metadata_rejects_unknown_layer() {
    let store = SaiStore::open_in_memory().unwrap();
    let path = std::path::Path::new(".sai/blocks/exp-seed.md");

    let result = validate(&config_with_layer(Some("foo")));
    assert!(!result.is_valid(), "foo は検査を通らない");
    let msg = result.errors[0].to_string();
    assert!(msg.contains("layer"), "{msg}");
    assert!(msg.contains("foo"), "{msg}");

    assert!(
        upsert_block_metadata(&store, &config_with_layer(Some("foo")), path).is_err(),
        "検査の通らないブロックは登録されない"
    );
    assert!(
        store.get_block_by_name("exp-seed").is_err(),
        "登録されていない"
    );
}

/// 3 つの層はすべて通ること。
#[test]
fn all_three_layers_are_valid() {
    for layer in ["root", "seed", "branch"] {
        let result = validate(&config_with_layer(Some(layer)));
        assert!(
            result.is_valid(),
            "{} は通らなければならない: {:?}",
            layer,
            result.errors
        );
    }
}
