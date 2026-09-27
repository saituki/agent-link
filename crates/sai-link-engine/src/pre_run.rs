//! # 実行直前の準備コマンド（pre_run）
//!
//! `.sai/settings.json` の `pre_run` に設定したコマンドを、エージェントを
//! 起動する**直前**に 1 回だけ実行します。
//!
//! 研究プロジェクトでは `sai-paper brief --write` を設定し、プロジェクトの
//! 状況のまとめ（`.sai/template/project-brief.md`）を毎回作り直すために使います。
//! 作り直されたファイルは、既存のテンプレート差し込み（
//! [`crate::parser::apply_templates`]）でそのままエージェントの最初の指示に入ります。
//!
//! - 呼び出し側（メインブロック / 子タスク / `block preview`）は、ブロック定義を
//!   パースする直前に [`run_if_configured`] を呼びます。
//! - 失敗しても実行は止めません（前回作ったファイルが使われます）。呼び出し側は
//!   警告を出すだけにします。
//! - シェルは通しません。配列の先頭がプログラム、残りが引数です。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 準備コマンドの打ち切り時間。
const PRE_RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// 失敗の警告に含める出力の最大文字数。
const MAX_OUTPUT_CHARS: usize = 500;

/// 準備コマンドの実行結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreRunOutcome {
    /// `pre_run` が未設定、または空だった（何もしない）。
    NotConfigured,
    /// コマンドが 0 で終わった。
    Ok,
    /// 起動できなかった・0 以外で終わった・時間切れ。理由を保持する。
    Failed(String),
}

/// `.sai/settings.json` の `pre_run` を読んで実行する。
///
/// 未設定・空なら何もせず [`PreRunOutcome::NotConfigured`] を返します。
/// 作業フォルダと環境変数（`SAI_PROJECT_ROOT` / `SAI_PAPER_ROOT`）には
/// [`crate::project::project_root`] を使います。打ち切りは 20 秒です。
pub fn run_if_configured() -> PreRunOutcome {
    let (settings, _warnings) = crate::pi_command::load_settings_quiet();
    match settings.pre_run {
        Some(cmd) if !cmd.is_empty() => {
            run_with(&cmd, &crate::project::project_root(), PRE_RUN_TIMEOUT)
        }
        _ => PreRunOutcome::NotConfigured,
    }
}

/// 準備コマンドを、設定とルートと打ち切り時間を明示して実行する（テスト用）。
///
/// # 引数
///
/// * `cmd` — 配列の先頭がプログラム、残りが引数（シェルは通さない）
/// * `root` — 作業フォルダ。`SAI_PROJECT_ROOT` / `SAI_PAPER_ROOT` にも入れる
/// * `timeout` — この時間を超えたらプロセスを kill する
///
/// 標準入力は閉じ、標準出力・標準エラーは読み取ります（失敗のときだけ
/// 先頭 [`MAX_OUTPUT_CHARS`] 文字を理由に含めます）。
pub fn run_with(cmd: &[String], root: &Path, timeout: Duration) -> PreRunOutcome {
    if cmd.is_empty() {
        return PreRunOutcome::NotConfigured;
    }
    let program = &cmd[0];
    let mut child = match Command::new(program)
        .args(&cmd[1..])
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root)
        .env("SAI_PAPER_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return PreRunOutcome::Failed(format!("起動できません: {program}: {e}")),
    };

    // 子が大量に出力してもブロックしないよう、パイプは別スレッドで読み切る。
    let stdout_reader = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_reader = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    // 時間切れ: kill してから待つ（ゾンビを残さない）。
                    // 待つことでパイプが閉じ、読み取りスレッドも終わる。
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                // try_wait 自体の失敗。プロセスを始末して失敗を返す。
                let _ = child.kill();
                let _ = child.wait();
                return PreRunOutcome::Failed(format!("プロセスの待機に失敗しました: {e}"));
            }
        }
    };

    // プロセスが終了（または kill）されるとパイプが閉じ、読み取りが完了する。
    let stdout = stdout_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    match status {
        Some(status) if status.success() => PreRunOutcome::Ok,
        Some(status) => {
            let code = status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "不明".to_string());
            PreRunOutcome::Failed(with_output(
                format!("コマンドが終了コード {code} で失敗しました"),
                &stdout,
                &stderr,
            ))
        }
        None => PreRunOutcome::Failed(format!("時間切れ（{} 秒）", timeout.as_secs())),
    }
}

/// 失敗理由に出力の先頭 [`MAX_OUTPUT_CHARS`] 文字を足す（空なら理由だけ）。
fn with_output(base: String, stdout: &[u8], stderr: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr).trim().to_string();
    let out = String::from_utf8_lossy(stdout).trim().to_string();
    let mut detail = String::new();
    if !err.is_empty() {
        detail.push_str(&err);
    }
    if !out.is_empty() {
        if !detail.is_empty() {
            detail.push_str(" / ");
        }
        detail.push_str(&out);
    }
    if detail.is_empty() {
        return base;
    }
    let truncated: String = detail.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{base}: {truncated}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = vec!["true".to_string()];
        assert_eq!(
            run_with(&cmd, dir.path(), Duration::from_secs(5)),
            PreRunOutcome::Ok
        );
    }

    #[test]
    fn non_zero_returns_failed() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = vec!["false".to_string()];
        match run_with(&cmd, dir.path(), Duration::from_secs(5)) {
            PreRunOutcome::Failed(msg) => assert!(
                msg.contains("終了コード"),
                "理由に終了コードが含まれる: {msg}"
            ),
            other => panic!("Failed を期待したが {other:?}"),
        }
    }

    #[test]
    fn missing_command_returns_failed() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = vec!["sai-link-pre-run-command-does-not-exist-xyz".to_string()];
        match run_with(&cmd, dir.path(), Duration::from_secs(5)) {
            PreRunOutcome::Failed(msg) => {
                assert!(msg.contains("起動できません"), "理由: {msg}")
            }
            other => panic!("Failed を期待したが {other:?}"),
        }
    }

    #[test]
    fn timeout_kills_process() {
        let dir = tempfile::tempdir().unwrap();
        // 子プロセス自身に自分の PID をファイルへ書かせ、kill 後に消えていることを見る。
        // `exec` で sh 自身を sleep に置き換え、孤児を作らない。
        let pid_file = dir.path().join("child.pid");
        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo $$ > {}; exec sleep 5", pid_file.display()),
        ];
        assert_eq!(
            run_with(&cmd, dir.path(), Duration::from_secs(1)),
            PreRunOutcome::Failed("時間切れ（1 秒）".to_string())
        );

        // 打ち切り後、子プロセスが残っていないこと（PID が存在しない）。
        let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
        let pid = pid.trim();
        assert!(!pid.is_empty(), "子プロセスの PID が取得できる");
        let alive = Command::new("kill")
            .args(["-0", pid])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(!alive, "打ち切り後に子プロセスが残っていない (pid={pid})");
    }

    #[test]
    fn passes_project_root_env() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "test \"$SAI_PAPER_ROOT\" = \"$SAI_PROJECT_ROOT\" \
             && test \"$SAI_PAPER_ROOT\" = \"$PWD\""
                .to_string(),
        ];
        // 作業フォルダもプロジェクトルートであること（$PWD で確認）を併せて検証する。
        let outcome = run_with(&cmd, &root, Duration::from_secs(5));
        assert_eq!(outcome, PreRunOutcome::Ok);
    }

    #[test]
    fn empty_command_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run_with(&[], dir.path(), Duration::from_secs(5)),
            PreRunOutcome::NotConfigured
        );
    }

    #[test]
    fn failure_output_is_included_and_truncated() {
        let dir = tempfile::tempdir().unwrap();
        // 600 文字を stderr に出す（先頭 500 文字だけが理由に含まれる）。
        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf 'x%.0s' $(seq 1 600) >&2; exit 3".to_string(),
        ];
        match run_with(&cmd, dir.path(), Duration::from_secs(5)) {
            PreRunOutcome::Failed(msg) => {
                assert!(msg.contains("終了コード 3"), "理由: {msg}");
                let detail = msg.split_once(": ").map(|(_, d)| d).unwrap_or("");
                assert_eq!(
                    detail.chars().count(),
                    MAX_OUTPUT_CHARS,
                    "500 文字に切り詰め"
                );
            }
            other => panic!("Failed を期待したが {other:?}"),
        }
    }
}
