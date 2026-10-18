//! Running `git lfs locks` and turning its quirks into clear errors.
//!
//! `git lfs locks --verify --json` exits 0 with an empty list outside a repository, without a remote,
//! and even when authentication fails (the reason only goes to stderr). So the repository and remote
//! are checked first, and stderr is inspected even on success.

use crate::git::{self, GitConfig};
use crate::locks::{parse_verify_json, LockList};

pub fn fetch_locks(config: &GitConfig) -> Result<LockList, String> {
    if git::toplevel().is_none() {
        return Err("Git のリポジトリの中で実行してください".into());
    }
    if !config.has_remote() {
        return Err("このリポジトリには remote（GitHub / GitLab の URL）がありません。ロックはサーバーで管理されるため、remote が必要です".into());
    }
    let output = git::output(&["lfs", "locks", "--verify", "--json"])
        .map_err(|_| "git が見つかりません。Git をインストールしてください".to_string())?;
    interpret(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

fn interpret(success: bool, stdout: &str, stderr: &str) -> Result<LockList, String> {
    if stderr.contains("'lfs' is not a git command") {
        return Err("Git LFS が入っていません。https://git-lfs.com からインストールしてください".into());
    }
    let problem = stderr.lines().map(str::trim).find(|l| {
        let lower = l.to_ascii_lowercase();
        lower.starts_with("fatal:") || lower.starts_with("error") || lower.contains("not supported") || lower.contains("does not support")
    });
    if let Some(line) = problem {
        return Err(explain(line));
    }
    if !success {
        let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("理由不明");
        return Err(explain(first));
    }
    parse_verify_json(stdout)
}

fn explain(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let hint = if lower.contains("could not read username") || lower.contains("authentication") || lower.contains("401") || lower.contains("403") {
        "サーバーにログインできませんでした。Fork や git push が使えるか（認証情報が保存されているか）を確かめてください。"
    } else if lower.contains("locking api") || lower.contains("not supported") || lower.contains("does not support") {
        "このサーバー（remote）はロック機能に対応していないようです。"
    } else if lower.contains("could not resolve host") || lower.contains("timed out") || lower.contains("connection") {
        "サーバーにつながりませんでした。ネットワークを確かめてください。"
    } else {
        ""
    };
    format!("ロック一覧を取得できませんでした。{hint}（git lfs: {line}）")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = r#"{"ours":[],"theirs":[]}"#;

    #[test]
    fn success_parses_stdout() {
        assert_eq!(interpret(true, EMPTY, "").unwrap(), LockList::default());
    }

    #[test]
    fn auth_failure_with_exit_zero_is_an_error() {
        // Captured from git-lfs 3.7: exit 0, empty list, reason only on stderr
        let stderr = "fatal: could not read Username for 'https://github.com': terminal prompts disabled\n";
        let err = interpret(true, EMPTY, stderr).unwrap_err();
        assert!(err.contains("ログインできませんでした"), "{err}");
        assert!(err.contains("could not read Username"), "{err}");
    }

    #[test]
    fn missing_lfs_is_explained() {
        let stderr = "git: 'lfs' is not a git command. See 'git --help'.\n";
        assert!(interpret(false, "", stderr).unwrap_err().contains("Git LFS が入っていません"));
    }

    #[test]
    fn unsupported_server_is_explained() {
        let stderr = "Remote \"origin\" does not support the Git LFS locking API.\n";
        assert!(interpret(true, EMPTY, stderr).unwrap_err().contains("ロック機能に対応していない"));
    }

    #[test]
    fn other_failures_show_stderr() {
        let err = interpret(false, "", "\nsomething odd happened\n").unwrap_err();
        assert!(err.contains("something odd happened"), "{err}");
    }
}
