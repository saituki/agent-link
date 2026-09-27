//! card 配置・round 区切り線のドラッグによるズレ量の永続化（`.sai/ui-layout.json`）。
//!
//! `calculate_layout` が決めた初期位置は変えず、そこからのズレ量（オフセット）だけを
//! 保存する。DB スキーマとは無関係の TUI 専用ファイルであり、欠けても機能に影響しない
//! （読み書きの失敗は無視して表示を続行する）。

use std::collections::HashMap;

/// `.sai/ui-layout.json` の内容。
///
/// ```json
/// { "nodes": { "12": [4, -2] }, "separators": { "3": 5 } }
/// ```
/// - `nodes`: exec_id → ズーム 1.0 基準の (dx, dy)
/// - `separators`: global_round → dy（その round 以降のノード・区切り線に累積で効く）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UiLayout {
    #[serde(default)]
    pub nodes: HashMap<i64, (i32, i32)>,
    #[serde(default)]
    pub separators: HashMap<i64, i32>,
}

impl UiLayout {
    /// 起動時に 1 回読み込む。ファイル無し・壊れている場合は空（デフォルト）を返す。
    pub fn load() -> Self {
        let path = crate::paths::resolve_ui_layout_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// drag 終了時に保存する。書き込み失敗は無視する。
    pub fn save(&self) {
        let path = crate::paths::resolve_ui_layout_path();
        let Ok(json) = serde_json::to_string_pretty(self) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        let _guard = crate::app::file_test_lock();
        let tmp =
            std::env::temp_dir().join(format!("sai_ui_layout_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let layout = UiLayout::load();
        assert!(layout.nodes.is_empty());
        assert!(layout.separators.is_empty());

        std::env::remove_var("SAI_PROJECT_ROOT");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_then_load_round_trips() {
        let _guard = crate::app::file_test_lock();
        let tmp =
            std::env::temp_dir().join(format!("sai_ui_layout_roundtrip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SAI_PROJECT_ROOT", &tmp);

        let mut layout = UiLayout::default();
        layout.nodes.insert(12, (4, -2));
        layout.separators.insert(3, 5);
        layout.save();

        let loaded = UiLayout::load();
        assert_eq!(loaded.nodes.get(&12), Some(&(4, -2)));
        assert_eq!(loaded.separators.get(&3), Some(&5));

        std::env::remove_var("SAI_PROJECT_ROOT");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
