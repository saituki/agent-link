//! 試験用の偽 sai-link。
//!
//! sci-core の単体試験では本物の `sai-moon-link` を使わない。
//! [`crate::bins::link_bin`] が（`cfg(test)` のとき）このモジュールの sh を返す。
//!
//! 偽 sai-link は次を行う。
//!
//! - `run list --json --limit 0`: `<root>/.fake-link/runs.json` があればそれを出す。
//!   無ければ `<root>/.sai/outputs/<lane>/<id>/task.md` を走査して組み立てる。
//! - `card create`: 標準入力を `<root>/.fake-link/card-<n>.json` に残し、
//!   `<root>/.sai/outputs/todo/<n>/task.md` を作って `{"run_id","path","executer"}` を出す。
//!   `<root>/.fake-link/card-exit` があれば、その番号で終わる。
//! - 呼ばれた引数を毎回 `<root>/.fake-link/calls.log` に足す。
//!
//! 試験は [`write_runs`] で明示の run の一覧を渡せる。渡さなければ実行フォルダを
//! 走査した結果を使う（多くの試験はそのままでよい）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

/// 偽 sai-link の sh を作り、パスを返す（1 プロセスに 1 つ）。
pub(crate) fn script() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("sai-sci-fake-link-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("fake-link.sh");
        std::fs::write(&path, SCRIPT).expect("偽 sai-link を書ける");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("実行の権限を付けられる");
        }
        path
    })
    .clone()
}

/// 明示の run の一覧を書く（`run list` はこれを出す）。
pub(crate) fn write_runs(root: &Path, runs: &[Value]) {
    let value = serde_json::json!({ "runs": runs });
    let dir = root.join(".fake-link");
    std::fs::create_dir_all(&dir).expect("偽 sai-link の置き場を作れる");
    std::fs::write(dir.join("runs.json"), value.to_string()).expect("run の一覧を書ける");
}

/// 試験用の run 1 件（よく使う項目だけ）。
pub(crate) fn run_value(
    run_id: i64,
    lane: &str,
    dir: &Path,
    block: &str,
    tag: &str,
    status: &str,
) -> Value {
    serde_json::json!({
        "run_id": run_id,
        "block": block,
        "tag": tag,
        "status": status,
        "lane": lane,
        "parent_run_id": null,
        "global_round": "",
        "round": "",
        "created_at": "",
        "started_at": "",
        "finished_at": "",
        "description": "",
        "dir": dir.display().to_string(),
    })
}

/// `card create` が失敗するようにする（終了コードを指定）。
pub(crate) fn fail_card_create(root: &Path, code: i32) {
    let dir = root.join(".fake-link");
    std::fs::create_dir_all(&dir).expect("偽 sai-link の置き場を作れる");
    std::fs::write(dir.join("card-exit"), code.to_string()).expect("終了コードを書ける");
}

/// 呼ばれた引数の記録。
pub(crate) fn calls(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(".fake-link/calls.log"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// `card create` に渡された JSON の並び（番号順）。
pub(crate) fn cards(root: &Path) -> Vec<Value> {
    let dir = root.join(".fake-link");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<(i64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().to_string();
            let rest = name.strip_prefix("card-")?.strip_suffix(".json")?;
            Some((rest.parse::<i64>().ok()?, path))
        })
        .collect();
    paths.sort_by_key(|(n, _)| *n);
    paths
        .into_iter()
        .filter_map(|(_, path)| {
            let text = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&text).ok()
        })
        .collect()
}

/// `card create` に渡された最後の JSON。
pub(crate) fn last_card(root: &Path) -> Option<Value> {
    cards(root).pop()
}

/// 偽 sai-link の sh 本体。
const SCRIPT: &str = r##"#!/bin/sh
# 試験用の偽 sai-link（sci-core の単体試験だけが使う）。
root="${SAI_PROJECT_ROOT:-$PWD}"
state="$root/.fake-link"
mkdir -p "$state" 2>/dev/null
printf '%s\n' "$*" >> "$state/calls.log"

case "$1 $2" in
  "run list")
    if [ -f "$state/runs.json" ]; then
      cat "$state/runs.json"
      exit 0
    fi
    python3 - "$root" <<'PY'
import json, os, re, sys
root = sys.argv[1]
runs = []
out = os.path.join(root, ".sai", "outputs")
if os.path.isdir(out):
    for lane in sorted(os.listdir(out)):
        lane_dir = os.path.join(out, lane)
        if not os.path.isdir(lane_dir):
            continue
        for name in sorted(os.listdir(lane_dir)):
            run_dir = os.path.join(lane_dir, name)
            if not os.path.isdir(run_dir) or not name.isdigit():
                continue
            fm = {}
            task = os.path.join(run_dir, "task.md")
            if os.path.isfile(task):
                try:
                    text = open(task, encoding="utf-8").read()
                except OSError:
                    text = ""
                m = re.match(r"^---\n(.*?)\n---", text, re.S)
                if m:
                    for line in m.group(1).splitlines():
                        if ":" in line:
                            k, v = line.split(":", 1)
                            fm[k.strip()] = v.strip()
            parent = fm.get("parent_run_id", "")
            runs.append({
                "run_id": int(name),
                "block": fm.get("block", ""),
                "tag": fm.get("tag", ""),
                "status": fm.get("status", ""),
                "lane": lane,
                "parent_run_id": int(parent) if parent.isdigit() else None,
                "global_round": fm.get("global_round", ""),
                "round": fm.get("round", ""),
                "created_at": fm.get("creation_date", ""),
                "started_at": fm.get("execution_date", ""),
                "finished_at": "",
                "description": fm.get("description", ""),
                "dir": run_dir,
            })
print(json.dumps({"runs": runs}, ensure_ascii=False))
PY
    exit 0
    ;;
  "card create")
    n=1
    while [ -f "$state/card-$n.json" ]; do n=$((n+1)); done
    cat > "$state/card-$n.json"
    if [ -f "$state/card-exit" ]; then
      echo "sai-link card create failed" >&2
      exit "$(cat "$state/card-exit")"
    fi
    taskdir="$root/.sai/outputs/todo/$n"
    mkdir -p "$taskdir"
    python3 - "$state/card-$n.json" "$taskdir/task.md" "$n" <<'PY'
import json, sys
src, dst, run_id = sys.argv[1], sys.argv[2], sys.argv[3]
d = json.load(open(src, encoding="utf-8"))
lines = [
    "---",
    "runner: " + str(d.get("runner", "")),
    "creation_date: 2026/01/01 00:00:00",
    "execution_date: -",
    "run_id: " + run_id,
    "block: " + str(d.get("block", "")),
    "thinking: ",
    "model: ",
    "provider: ",
    "round: " + str(d.get("round", "")),
    "global_round: " + str(d.get("global_round", "")),
    "tag: " + str(d.get("tag", "")),
    "description: " + str(d.get("description", "")),
    "executer: " + str(d.get("executer", "")),
]
meta = d.get("meta") or {}
for key in sorted(meta):
    if str(meta[key]) != "":
        lines.append(key + ": " + str(meta[key]))
lines += ["---", "", "# task_text", ""]
lines.append(str(d.get("task_text", "")))
open(dst, "w", encoding="utf-8").write("\n".join(lines) + "\n")
PY
    executer=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8")).get("executer",""))' "$state/card-$n.json")
    printf '{"run_id":%s,"path":"%s","executer":"%s"}\n' "$n" "$taskdir/task.md" "$executer"
    exit 0
    ;;
esac

echo "fake-link: unknown command: $*" >&2
exit 1
"##;
