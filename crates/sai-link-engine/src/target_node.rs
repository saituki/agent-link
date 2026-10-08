//! 研究の Node（`SAI_TARGET_NODE`）の決め方。
//!
//! run の task.md の `sci_node`（`SEED` 以外）か、子 task が受け継いだ `target_node` を使う。
//! 無ければ親の run をたどる。sai-link は research-tree を知らないので、値の形や Node の
//! 存在は検査しない（検査は sai-sci がする）。

/// 親をたどる深さの上限。
const MAX_DEPTH: usize = 10;

/// task.md の frontmatter から `key` の値を取り出す（空なら `None`）。
fn fm_value(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == key {
                let v = v.trim();
                return (!v.is_empty()).then(|| v.to_string());
            }
        }
    }
    None
}

/// task.md 自身が持つ Node（`sci_node` が先。`SEED` は DB の行が無いので除く）。
fn own_node(text: &str) -> Option<String> {
    fm_value(text, "sci_node")
        .filter(|v| v != "SEED")
        .or_else(|| fm_value(text, "target_node"))
}

/// frontmatter の閉じる `---` の前に `target_node: <node>` を足す。frontmatter が無ければ変えない。
fn with_target_node(text: &str, node: &str) -> String {
    let mut out = String::new();
    let mut seen_open = false;
    let mut done = false;
    for line in text.split_inclusive('\n') {
        if !done && line.trim() == "---" {
            if seen_open {
                out.push_str(&format!("target_node: {node}\n"));
                done = true;
            } else {
                seen_open = true;
            }
        }
        out.push_str(line);
    }
    if done { out } else { text.to_string() }
}

/// run の Node を決める。`read` は run の task.md の本文、`parent_of` は親の run_id を返す。
///
/// 戻り値は `(Node, 親から受け継いだか)`。
pub fn resolve_with(
    run_id: i64,
    read: impl Fn(i64) -> Option<String>,
    parent_of: impl Fn(i64) -> Option<i64>,
) -> Option<(String, bool)> {
    let mut id = run_id;
    for depth in 0..=MAX_DEPTH {
        if let Some(node) = read(id).as_deref().and_then(own_node) {
            return Some((node, depth > 0));
        }
        id = parent_of(id)?;
    }
    None
}

/// 実際の run フォルダと DB で Node を決める。受け継いだときは run の task.md に `target_node` を書く。
pub fn resolve(store: &crate::db::SaiStore, run_id: i64) -> Option<String> {
    let read = |id: i64| {
        let (_, dir) = crate::run_dir::find_run_dir(id)?;
        std::fs::read_to_string(dir.join(crate::run_dir::TASK_FILE)).ok()
    };
    let parent_of = |id: i64| store.get_execution_by_id(id).ok()?.parent_run_id;
    let (node, inherited) = resolve_with(run_id, read, parent_of)?;
    if inherited {
        if let Some((_, dir)) = crate::run_dir::find_run_dir(run_id) {
            let path = dir.join(crate::run_dir::TASK_FILE);
            if let Ok(text) = std::fs::read_to_string(&path) {
                let _ = std::fs::write(path, with_target_node(&text, &node));
            }
        }
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(extra: &str) -> String {
        format!("---\ncreation_date: x\n{extra}---\n\n# task_text\n\nhi\n")
    }

    #[test]
    fn sci_node_と_seed_と_継承() {
        let own = card("sci_node: O5\n");
        assert_eq!(resolve_with(500, |_| Some(own.clone()), |_| None), Some(("O5".into(), false)));
        let seed = card("sci_node: SEED\n");
        assert_eq!(resolve_with(502, |_| Some(seed.clone()), |_| None), None);
        // 501 は sci_node 無し・親 500 が O5。
        let read = |id: i64| Some(if id == 500 { own.clone() } else { card("") });
        let parent = |id: i64| (id == 501).then_some(500);
        assert_eq!(resolve_with(501, read, parent), Some(("O5".into(), true)));
        assert!(with_target_node(&card(""), "O5").contains("\ntarget_node: O5\n---\n"));
        // 書き足したあとは自身の値として読める。
        let written = with_target_node(&card(""), "O5");
        assert_eq!(own_node(&written).as_deref(), Some("O5"));
    }
}
