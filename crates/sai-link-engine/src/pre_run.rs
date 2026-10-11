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
//!
//! # 範囲（context）
//!
//! [`context_text`] は研究ブロック（`context: true`）のためのものです。
//! `.sai/settings.json` の `context` の命令を起動の直前に走らせ、その標準出力を
//! 最初のメッセージの `# context` の節の中身として返す（節の見出しは
//! `split_fixed_and_variable` が付ける）。`pre_run` と違いファイルは作りません
//! （並列に起動しても中身が混ざらないようにするため）。

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// 準備コマンドの打ち切り時間。
const PRE_RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// 失敗の警告に含める出力の最大文字数。
const MAX_OUTPUT_CHARS: usize = 500;

/// [`context_text`] が命令の出力として受け取る最大文字数。
const MAX_CAPTURE_CHARS: usize = 200_000;

/// [`run_capture`] が失敗理由として返す最大文字数。
const MAX_CAPTURE_REASON_CHARS: usize = 200;

/// `.sai/settings.json` に `job_status` が無いときの文（常駐と `card wait` で同じ）。
pub const JOB_STATUS_MISSING: &str =
    "ジョブの状態を確かめる命令（.sai/settings.json の job_status）がありません";

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
/// 作業フォルダと環境変数（`SAI_PROJECT_ROOT`）には
/// [`crate::project::project_root`] を使います。打ち切りは 20 秒です。
pub fn run_if_configured() -> PreRunOutcome {
    let (settings, _warnings) = crate::settings::load_settings_quiet();
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
/// * `root` — 作業フォルダ。`SAI_PROJECT_ROOT` にも入れる
/// * `timeout` — この時間を超えたらプロセスを kill する
///
/// 標準入力は閉じ、標準出力・標準エラーは読み取ります（失敗のときだけ
/// 先頭 [`MAX_OUTPUT_CHARS`] 文字を理由に含めます）。
pub fn run_with(cmd: &[String], root: &Path, timeout: Duration) -> PreRunOutcome {
    if cmd.is_empty() {
        return PreRunOutcome::NotConfigured;
    }
    let (status, stdout, stderr) = match run_child(cmd, root, &[], None, timeout) {
        Ok(v) => v,
        Err(reason) => return PreRunOutcome::Failed(reason),
    };
    match status {
        Some(status) if status.success() => PreRunOutcome::Ok,
        Some(status) => PreRunOutcome::Failed(with_output(
            format!("コマンドが{} で失敗しました", exit_text(status)),
            &stdout,
            &stderr,
        )),
        None => PreRunOutcome::Failed(format!("時間切れ（{} 秒）", timeout.as_secs())),
    }
}

/// 命令を実行して、0 で終わったら標準出力を返す（最大 [`MAX_CAPTURE_CHARS`] 文字）。
///
/// 失敗・時間切れは理由（先頭 [`MAX_CAPTURE_REASON_CHARS`] 文字）を `Err` で返す。
/// `envs` は [`run_with`] が入れる環境変数に加えて足す環境変数。
///
/// # 引数
///
/// * `cmd` — 配列の先頭がプログラム、残りが引数（シェルは通さない）
/// * `root` — 作業フォルダ。`SAI_PROJECT_ROOT` にも入る
/// * `envs` — 追加の環境変数（例: `SAI_PARENT_RUN_ID` / `SAI_BLOCK_NAME`）
/// * `timeout` — この時間を超えたらプロセスを kill する
pub fn run_capture(
    cmd: &[String],
    root: &Path,
    envs: &[(&str, String)],
    timeout: Duration,
) -> Result<String, String> {
    if cmd.is_empty() {
        return Err("命令が空です".to_string());
    }
    let (status, stdout, stderr) = match run_child(cmd, root, envs, None, timeout) {
        Ok(v) => v,
        Err(reason) => return Err(truncate_reason(reason)),
    };
    match status {
        Some(status) if status.success() => {
            Ok(String::from_utf8_lossy(&stdout)
                .chars()
                .take(MAX_CAPTURE_CHARS)
                .collect())
        }
        Some(status) => Err(truncate_reason(with_output(
            format!("コマンドが{} で失敗しました", exit_text(status)),
            &stdout,
            &stderr,
        ))),
        None => Err(truncate_reason(format!(
            "時間切れ（{} 秒）",
            timeout.as_secs()
        ))),
    }
}

/// 子プロセスを起動して走らせ、終了状態と標準出力・標準エラーを返す。
///
/// - 終了状態は `Some` が通常の終了、`None` が時間切れ（kill 済み）です。
/// - 起動できなかった・待機に失敗したときは、その理由を `Err` で返します
///   （時間切れと区別するため）。
/// - 子が大量に出力しても現れないよう、パイプは別スレッドで読み切ります。
fn run_child(
    cmd: &[String],
    root: &Path,
    envs: &[(&str, String)],
    stdin_text: Option<&str>,
    timeout: Duration,
) -> Result<(Option<std::process::ExitStatus>, Vec<u8>, Vec<u8>), String> {
    let program = &cmd[0];
    let mut command = Command::new(program);
    command
        .args(&cmd[1..])
        .current_dir(root)
        .env("SAI_PROJECT_ROOT", root)
        .stdin(if stdin_text.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return Err(format!("起動できません: {program}: {e}")),
    };

    // 本文は別スレッドで書いて閉じる（子が読まずに終わっても詰まらない）。
    if let (Some(text), Some(mut stdin)) = (stdin_text, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        });
    }

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
                return Err(format!("プロセスの待機に失敗しました: {e}"));
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
    Ok((status, stdout, stderr))
}

/// 本文を標準入力で渡して命令を動かした結果（`write_check` 用）。
pub enum StdinOutcome {
    /// 終了コード 0。標準出力（先頭 [`MAX_CAPTURE_CHARS`] 文字まで）。
    Ok(String),
    /// 終了コード 1。標準エラー（先頭 4000 文字まで、前後の空白を除く）。
    Rejected(String),
    /// 起動できない・時間切れ・0 と 1 以外の終了コード。理由の 1 行（200 文字まで）。
    Broken(String),
}

/// 本文を標準入力で渡して命令を動かす。終了コードで [`StdinOutcome`] を分ける。
pub fn run_with_stdin(
    cmd: &[String],
    root: &Path,
    envs: &[(&str, String)],
    stdin_text: &str,
    timeout: Duration,
) -> StdinOutcome {
    if cmd.is_empty() {
        return StdinOutcome::Broken("命令が空です".to_string());
    }
    match run_child(cmd, root, envs, Some(stdin_text), timeout) {
        Err(reason) => StdinOutcome::Broken(truncate_reason(reason)),
        Ok((None, ..)) => StdinOutcome::Broken(format!("{} 秒で終わりませんでした", timeout.as_secs())),
        Ok((Some(status), stdout, stderr)) => match status.code() {
            Some(0) => StdinOutcome::Ok(
                String::from_utf8_lossy(&stdout).chars().take(MAX_CAPTURE_CHARS).collect(),
            ),
            Some(1) => StdinOutcome::Rejected(
                String::from_utf8_lossy(&stderr).trim().chars().take(4000).collect(),
            ),
            _ => StdinOutcome::Broken(truncate_reason(exit_text(status))),
        },
    }
}

/// 失敗理由の 1 行を作る。
///
/// 設定が無い・実行番号が無い・命令が失敗した・時間切れのどれでも
/// 本文に入れるのは 1 行だけなので、失敗しても実行は止めません。
fn failed_line(reason: &str) -> String {
    format!("（{reason}）")
}

/// 失敗理由を先頭 [`MAX_CAPTURE_REASON_CHARS`] 文字に切り詰める。
fn truncate_reason(reason: String) -> String {
    reason
        .chars()
        .take(MAX_CAPTURE_REASON_CHARS)
        .collect()
}

/// 終了状態を文にする（終了コード → シグナル番号の順。どちらも取れなければ「不明」）。
///
/// 「不明」になるのは、シグナルで止まったのにシグナル番号を取れなかったときだけ。
fn exit_text(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("終了コード {code}");
    }
    if let Some(signal) = status.signal() {
        return format!("シグナル {signal}");
    }
    "終了コード 不明".to_string()
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

/// `context: true` のブロックの範囲の文を作る（C-11）。
///
/// `.sai/settings.json` の `context` の命令を走らせ、その標準出力を返します。
/// 戻り値は最初のメッセージの `# context` の節の中身になります（節の見出しは
/// `split_fixed_and_variable` が付けます）。
/// 失敗しても止めずに、理由の 1 行を返します（その 1 行がそのまま節の中身になる）。
/// ファイルは作りません。
///
/// # 引数
///
/// * `run_id` — 実行 ID。`None` のときは命令を走らせない
/// * `block_id` — ブロック ID（`SAI_BLOCK_NAME` と `{block}` に入る）
pub fn context_text(run_id: Option<i64>, block_id: &str) -> String {
    // 実行番号が無いと範囲を作れない（block preview などがこれに当たる）
    let Some(run_id) = run_id else {
        return failed_line("実行番号が無いため、範囲を作れませんでした");
    };

    let (settings, _warnings) = crate::settings::load_settings_quiet();
    let Some(cmd) = settings.context.filter(|c| !c.is_empty()) else {
        return failed_line(".sai/settings.json に context がありません。範囲を作れませんでした");
    };

    // 引数の中の {run_id} と {block} を置き換える
    let cmd: Vec<String> = cmd
        .iter()
        .map(|arg| {
            arg.replace("{run_id}", &run_id.to_string())
                .replace("{block}", block_id)
        })
        .collect();

    let root = crate::project::project_root();
    let envs = [
        ("SAI_PARENT_RUN_ID", run_id.to_string()),
        ("SAI_BLOCK_NAME", block_id.to_string()),
    ];
    match run_capture(&cmd, &root, &envs, PRE_RUN_TIMEOUT) {
        // 末尾の改行を 1 つだけ落とす（命令が 1 行だけ出したときの余白を避ける）
        Ok(text) => text.strip_suffix('\n').unwrap_or(&text).to_string(),
        Err(reason) => failed_line(&format!("範囲を作れませんでした: {reason}")),
    }
}

/// `.sai/settings.json` の `job_status` を読む（プロジェクトのルート基準）。
///
/// ジョブの状態を確かめる命令（文字の配列。先頭が実行ファイル）を返す。
/// `{job}` は呼び出し側がジョブの ID に置き換える。
/// 無い・空・形が違う（配列でない・空配列・先頭が空文字）ときは `None`。
/// 研究のプロジェクトでは `sai-sci init` が書く（sai-link は読むだけ）。
pub fn job_status_template() -> Option<Vec<String>> {
    job_status_template_at(&crate::project::project_root())
}

/// `.sai/settings.json` の `job_status` を、指定したルートで読む（テスト用）。
pub fn job_status_template_at(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join(".sai/settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let out: Vec<String> = value
        .get("job_status")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    if out.is_empty() || out[0].trim().is_empty() {
        return None;
    }
    Some(out)
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
    fn signal_death_names_the_signal() {
        // AC3: シグナルで死んだら「不明」ではなくシグナル番号を出す（kill -9 = 9）
        let dir = tempfile::tempdir().unwrap();
        let cmd = vec!["sh".to_string(), "-c".to_string(), "kill -9 $$".to_string()];
        match run_with(&cmd, dir.path(), Duration::from_secs(5)) {
            PreRunOutcome::Failed(msg) => {
                assert!(msg.contains("シグナル 9"), "理由: {msg}");
                assert!(!msg.contains("不明"), "理由: {msg}");
            }
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
            "test \"$SAI_PROJECT_ROOT\" = \"$PWD\""
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

    // ── 範囲（context）のテスト ──

    /// 環境変数を退避してから設定し、終了時に復元する補助。
    /// グローバル設定（`~/.sai/settings.json`）も空の一時フォルダに向ける。
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        project_root: Option<std::ffi::OsString>,
        sai_home: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        /// 一時フォルダを `SAI_PROJECT_ROOT` にして、空の一時フォルダを
        /// `SAI_HOME` にする（テスト中は他のテストと干渉しない）。
        fn new(root: &Path) -> Self {
            let lock = crate::common::SAI_ENV_TEST_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let project_root = std::env::var_os("SAI_PROJECT_ROOT");
            let sai_home = std::env::var_os("SAI_HOME");
            std::env::set_var("SAI_PROJECT_ROOT", root);
            let empty_home = root.join("empty-home");
            std::fs::create_dir_all(&empty_home).unwrap();
            std::env::set_var("SAI_HOME", &empty_home);
            Self {
                _lock: lock,
                project_root,
                sai_home,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.project_root {
                Some(v) => std::env::set_var("SAI_PROJECT_ROOT", v),
                None => std::env::remove_var("SAI_PROJECT_ROOT"),
            }
            match &self.sai_home {
                Some(v) => std::env::set_var("SAI_HOME", v),
                None => std::env::remove_var("SAI_HOME"),
            }
        }
    }

    /// 一時フォルダに `.sai/settings.json` を書く。
    fn write_settings(root: &Path, body: &str) {
        let dir = root.join(".sai");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), body).unwrap();
    }

    /// 成功した命令の標準出力をそのまま返す（末尾の改行は落とさない）。
    #[test]
    fn run_capture_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cmd = vec!["/bin/echo".to_string(), "範囲の文".to_string()];
        let out = run_capture(&cmd, &root, &[], Duration::from_secs(5)).unwrap();
        assert_eq!(out, "範囲の文\n");
    }

    /// 0 以外で終わったときと時間切れのときは理由が返る。
    #[test]
    fn run_capture_reports_failure_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // 起動できないプログラム
        let cmd = vec!["sai-link-pre-run-command-does-not-exist-xyz".to_string()];
        let reason = run_capture(&cmd, &root, &[], Duration::from_secs(5)).unwrap_err();
        assert!(reason.contains("起動できません"), "理由: {reason}");

        // 0 以外で終わる（理由は先頭 200 字まで）
        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf 'x%.0s' $(seq 1 600) >&2; exit 3".to_string(),
        ];
        let reason = run_capture(&cmd, &root, &[], Duration::from_secs(5)).unwrap_err();
        assert!(reason.contains("終了コード 3"), "理由: {reason}");
        assert!(
            reason.chars().count() <= MAX_CAPTURE_REASON_CHARS,
            "理由が {} 文字に切り詰められている",
            reason.chars().count()
        );

        // 時間切れ（打ち切って理由が返る）
        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "exec sleep 5".to_string(),
        ];
        let reason = run_capture(&cmd, &root, &[], Duration::from_secs(1)).unwrap_err();
        assert!(reason.contains("時間切れ"), "理由: {reason}");
    }

    /// 引数の中の `{run_id}` と `{block}` を置き換え、標準出力を返す。
    #[test]
    fn context_text_substitutes_run_and_block() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write_settings(
            &root,
            r#"{"context": ["/bin/echo", "run={run_id}", "block={block}"]}"#,
        );
        let _env = EnvGuard::new(&root);
        let text = context_text(Some(42), "sci-impl");
        assert_eq!(text, "run=42 block=sci-impl");
    }

    /// 実行番号が無いとき・設定が無いときは理由の 1 行を返す。
    #[test]
    fn context_text_messages_without_run_or_setting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // 設定が無い（settings.json 自体が無い）
        let _env = EnvGuard::new(&root);
        let text = context_text(Some(42), "sci-impl");
        assert_eq!(
            text,
            "（.sai/settings.json に context がありません。範囲を作れませんでした）"
        );
        // 設定はあるが空
        write_settings(&root, r#"{"context": []}"#);
        let text = context_text(Some(42), "sci-impl");
        assert_eq!(
            text,
            "（.sai/settings.json に context がありません。範囲を作れませんでした）"
        );
        // 実行番号が無い（命令は走らない）
        write_settings(
            &root,
            r#"{"context": ["/bin/echo", "run={run_id}", "block={block}"]}"#,
        );
        let text = context_text(None, "sci-impl");
        assert_eq!(text, "（実行番号が無いため、範囲を作れませんでした）");
    }

    /// 命令が失敗したときも実行は止めず、理由の 1 行を返す。
    #[test]
    fn context_text_reports_command_failure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write_settings(
            &root,
            r#"{"context": ["sh", "-c", "echo 途中 >&2; exit 1"]}"#,
        );
        let _env = EnvGuard::new(&root);
        let text = context_text(Some(7), "sci-impl");
        assert!(text.starts_with("（範囲を作れませんでした: "), "理由: {text}");
        assert!(text.contains("終了コード"), "理由: {text}");
    }

    /// `.sai/settings.json` の `job_status` を読む。無い・空・形が違うときは None。
    #[test]
    fn job_status_template_reads_array_or_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // 設定が無い
        assert_eq!(job_status_template_at(&root), None);
        // 正しい形（配列。先頭が実行ファイル）
        write_settings(&root, r#"{"job_status": ["uv", "run", "{job}"]}"#);
        assert_eq!(
            job_status_template_at(&root),
            Some(vec!["uv".to_string(), "run".to_string(), "{job}".to_string()])
        );
        // 空・形が違う
        for body in [
            r#"{"job_status": []}"#,
            r#"{"job_status": "uv run"}"#,
            r#"{"job_status": ["", "run"]}"#,
            r#"{"job_status": ["uv", 1]}"#,
        ] {
            write_settings(&root, body);
            assert_eq!(job_status_template_at(&root), None, "body={body}");
        }
    }

    /// 範囲の文を入れてもファイルを作らない。
    #[test]
    fn context_text_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write_settings(&root, r#"{"context": ["/bin/echo", "{block}"]}"#);
        let template_dir = root.join(".sai/template");
        let _env = EnvGuard::new(&root);
        let text = context_text(Some(1), "sci-impl");
        assert_eq!(text, "sci-impl");
        assert!(
            !template_dir.exists(),
            ".sai/template/ が作られていない: {}",
            template_dir.display()
        );
    }
}
