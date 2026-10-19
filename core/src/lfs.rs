//! Running `git lfs locks` and turning its quirks into clear errors.
//!
//! `git lfs locks --verify --json` exits 0 with an empty list outside a repository, without a remote,
//! and even when authentication fails (the reason only goes to stderr). So the repository and remote
//! are checked first, and stderr is inspected even on success. Network errors are worse: with `--json`
//! they print nothing at all, so an empty list is confirmed once more without `--json`.

use crate::git::{self, GitConfig};
use crate::locks::{parse_verify_json, LockList};
use crate::paths::RepoContext;

pub fn fetch_locks(config: &GitConfig) -> Result<LockList, String> {
    fetch_locks_in(&RepoContext::current()?, config)
}

/// Like `fetch_locks`, for a repository the GUI opened. `config` must be read there too (`GitConfig::read_in`).
pub fn fetch_locks_in(ctx: &RepoContext, config: &GitConfig) -> Result<LockList, String> {
    require_remote(config)?;
    query_locks_in(&ctx.root)
}

pub(crate) fn require_remote(config: &GitConfig) -> Result<(), String> {
    if config.has_remote() {
        Ok(())
    } else {
        Err("このリポジトリには remote（GitHub / GitLab の URL）がありません。ロックはサーバーで管理されるため、remote が必要です".into())
    }
}

/// Ask the server for all locks. Callers have already checked the repository and remote.
pub(crate) fn query_locks() -> Result<LockList, String> {
    query_locks_in(".")
}

fn query_locks_in(dir: &str) -> Result<LockList, String> {
    let output = git::run_in(dir, &["lfs", "locks", "--verify", "--json"])?;
    let locks = interpret(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )?;
    if locks.is_empty() {
        confirm_empty(dir)?;
    }
    Ok(locks)
}

/// `git lfs locks --verify --json` turns a network error into exit 0 and an empty list with no message
/// (git-lfs 3.7). The text form does report it, so ask again that way (one lock is enough).
fn confirm_empty(dir: &str) -> Result<(), String> {
    let output = git::run_in(dir, &["lfs", "locks", "--verify", "--limit", "1"])?;
    confirmation(output.status.success(), &String::from_utf8_lossy(&output.stderr))
}

fn confirmation(success: bool, stderr: &str) -> Result<(), String> {
    if success {
        Ok(())
    } else {
        Err(format!("ロック一覧を取得できませんでした。{}", explain(first_line(stderr))))
    }
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
        return Err(format!("ロック一覧を取得できませんでした。{}", explain(line)));
    }
    if !success {
        return Err(format!("ロック一覧を取得できませんでした。{}", explain(first_line(stderr))));
    }
    parse_verify_json(stdout)
}

/// The first non-empty line of git lfs's error output.
pub(crate) fn first_line(stderr: &str) -> &str {
    stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("理由不明")
}

/// A Japanese hint for a git lfs error line, followed by the original line.
pub(crate) fn explain(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let hint = if lower.contains("uncommitted changes") {
        "未コミットの変更があるため解除できません。コミットしてから解除するか、--force を付けてください。"
    } else if lower.contains("lock exists") || lower.contains("already locked") || lower.contains("already created") {
        "ほかの人がすでにロックしています。repo-tweaks list で確かめてください。"
    } else if lower.contains("could not read username")
        || lower.contains("authentication")
        || lower.contains("401")
        || lower.contains("publickey")
    {
        "サーバーにログインできませんでした。Fork や git push が使えるか（認証情報や SSH の鍵が使えるか）を確かめてください。"
    } else if lower.contains("admin access") {
        "この操作にはリポジトリの管理者権限が必要です（GitHub では、他人のロックの強制解除は管理者だけができます）。"
    } else if lower.contains("403") || lower.contains("forbidden") || lower.contains("permission") {
        "権限がありません。ログインしているアカウントに、このリポジトリへの書き込み権限があるか確かめてください。"
    } else if lower.contains("locking api") || lower.contains("not supported") || lower.contains("does not support") {
        "このサーバー（remote）はロック機能に対応していないようです。"
    } else if lower.contains("could not resolve host")
        || lower.contains("no such host")
        || lower.contains("dial tcp")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
    {
        "サーバーにつながりませんでした。ネットワークを確かめてください。"
    } else {
        ""
    };
    format!("{hint}（git lfs: {line}）")
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
        assert!(err.starts_with("ロック一覧を取得できませんでした。"), "{err}");
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
    fn lock_and_unlock_failures_are_explained() {
        assert!(explain("Cannot unlock file with uncommitted changes").contains("未コミットの変更"));
        assert!(explain("Lock exists").contains("すでにロック"));
        assert!(explain("api error: Forbidden (403)").contains("書き込み権限"));
        assert!(explain("You must have admin access to force delete a lock").contains("管理者権限"));
        // SSH key problems are login problems, not permissions on locks
        assert!(explain("git@github.com: Permission denied (publickey).").contains("ログインできませんでした"));
        assert_eq!(first_line("\n  \nfirst\nsecond"), "first");
    }

    #[test]
    fn empty_list_is_confirmed() {
        // Captured from git-lfs 3.7 (text form): with --json the same failure printed nothing and exited 0
        let stderr = "Error while retrieving locks: Post \"https://invalid.example/o/r.git/info/lfs/locks/verify\": dial tcp: lookup invalid.example: no such host\n";
        let err = confirmation(false, stderr).unwrap_err();
        assert!(err.contains("サーバーにつながりませんでした"), "{err}");
        assert!(err.contains("no such host"), "{err}");
        assert!(confirmation(true, "").is_ok());
    }

    #[test]
    fn other_failures_show_stderr() {
        let err = interpret(false, "", "\nsomething odd happened\n").unwrap_err();
        assert!(err.contains("something odd happened"), "{err}");
    }
}
