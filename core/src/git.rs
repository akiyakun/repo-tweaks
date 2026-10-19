//! Small helpers that call the `git` command.
//!
//! Each `git` process costs tens of milliseconds on Windows, and `notify` runs inside a hook,
//! so values are read in as few calls as possible: one `git config` and one `git rev-parse`.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};

pub const KEY_SERVER: &str = "repo-tweaks.server";
pub const KEY_TOKEN: &str = "repo-tweaks.token";
/// Git lowercases keys in `--get-regexp` output, so this is `staleDays` as users write it.
pub const KEY_STALE_DAYS: &str = "repo-tweaks.staledays";
pub const KEY_NOTIFY: &str = "repo-tweaks.notify";
const KEY_USER: &str = "user.name";
const KEY_REMOTE: &str = "remote.origin.url";

static HIDE_CONSOLE: AtomicBool = AtomicBool::new(false);

/// Stop each git process from flashing a console window on Windows (for the GUI; the CLI keeps
/// its console so git can still ask for credentials there). No effect on other platforms.
pub fn hide_console_windows() {
    HIDE_CONSOLE.store(true, Ordering::Relaxed);
}

fn command() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("git");
    #[cfg(windows)]
    if HIDE_CONSOLE.load(Ordering::Relaxed) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// What it means when `git` cannot be started: it is not installed (or not on PATH).
pub const GIT_MISSING: &str = "git が見つかりません。Git をインストールしてください";

/// Like `output`, with a git that cannot be started turned into GIT_MISSING.
pub fn run(args: &[&str]) -> Result<Output, String> {
    output(args).map_err(|_| GIT_MISSING.to_string())
}

/// Like `output_in`, with a git that cannot be started turned into GIT_MISSING.
/// A missing `dir` also stops git from starting, so that is checked first and reported as such.
pub fn run_in(dir: &str, args: &[&str]) -> Result<Output, String> {
    if !Path::new(dir).is_dir() {
        return Err(format!("フォルダが見つかりません: {dir}"));
    }
    output_in(dir, args).map_err(|_| GIT_MISSING.to_string())
}

/// Run `git <args>` and return the raw output (for callers that need stderr).
pub fn output(args: &[&str]) -> std::io::Result<Output> {
    command().args(args).output()
}

/// Like `output`, but run in `dir` (e.g. the repository root, for root-relative paths).
pub fn output_in(dir: &str, args: &[&str]) -> std::io::Result<Output> {
    command().current_dir(dir).args(args).output()
}

/// Run `git <args>` in `dir` and return stdout, or None on any failure.
fn git_in(dir: &str, args: &[&str]) -> Option<String> {
    stdout(output_in(dir, args))
}

/// Run `git <args>` and return stdout, or None on any failure.
fn git(args: &[&str]) -> Option<String> {
    stdout(output(args))
}

fn stdout(output: std::io::Result<Output>) -> Option<String> {
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The git config values this tool uses. Later entries (local over global) win, as in git itself.
pub struct GitConfig(HashMap<String, String>);

impl GitConfig {
    pub fn read() -> Self {
        Self::read_in(".")
    }

    /// Like `read`, but for the repository at `dir` (the GUI opens a folder instead of using the current directory).
    pub fn read_in(dir: &str) -> Self {
        let pattern = r"^(repo-tweaks\.(server|token|staledays|notify)|user\.name|remote\..+\.url)$";
        // Exit code 1 (no match) also lands here as an empty config.
        Self::parse(&git_in(dir, &["config", "--get-regexp", pattern]).unwrap_or_default())
    }

    fn parse(text: &str) -> Self {
        let mut values = HashMap::new();
        for line in text.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            values.insert(key.to_ascii_lowercase(), value.trim().to_string());
        }
        GitConfig(values)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).filter(|v| !v.is_empty()).cloned()
    }

    /// Whether any remote is configured (Git LFS needs one to ask for locks).
    pub fn has_remote(&self) -> bool {
        self.0.keys().any(|k| k.starts_with("remote.") && k.ends_with(".url"))
    }
}

/// Root of the repository containing the current directory, or None outside a repository.
pub fn toplevel() -> Option<String> {
    git(&["rev-parse", "--show-toplevel"]).map(|s| s.trim().to_string())
}

/// Information about the repository in the current directory. Missing values stay None.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct RepoInfo {
    pub repo: Option<String>,
    /// Stable identity across clones: host/path of remote.origin.url without credentials
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub user: Option<String>,
}

impl RepoInfo {
    pub fn current(config: &GitConfig) -> Self {
        let (toplevel, commit, branch) = match git(&["rev-parse", "--show-toplevel", "HEAD", "--abbrev-ref", "HEAD"]) {
            Some(out) => {
                let mut lines = out.lines().map(|l| Some(l.trim().to_string()));
                (lines.next().flatten(), lines.next().flatten(), lines.next().flatten())
            }
            // No commits yet: HEAD does not resolve, but the repository may still exist.
            None => (toplevel(), None, None),
        };
        Self::build(config, toplevel, commit, branch)
    }

    fn build(config: &GitConfig, toplevel: Option<String>, commit: Option<String>, branch: Option<String>) -> Self {
        RepoInfo {
            repo: repo_name(config, toplevel.as_deref()),
            remote: remote_id(config),
            branch,
            commit,
            user: config.get(KEY_USER),
        }
    }
}

/// `host/owner/repo` of remote.origin.url, the identity shared by every clone.
pub fn remote_id(config: &GitConfig) -> Option<String> {
    config.get(KEY_REMOTE).and_then(|url| normalize_remote(&url))
}

/// Name to show for a repository: the remote's last path segment, else the folder name.
/// The remote comes first because the local folder name may differ per clone.
pub fn repo_name(config: &GitConfig, toplevel: Option<&str>) -> Option<String> {
    remote_id(config)
        .as_deref()
        .and_then(|r| r.rsplit('/').next())
        .map(str::to_string)
        .or_else(|| toplevel.and_then(|t| Path::new(t).file_name().map(|n| n.to_string_lossy().into_owned())))
}

/// Turn a remote URL into `host/owner/repo`, dropping scheme, credentials, port and `.git`.
///
/// https://user:token@GitHub.com/owner/repo.git -> github.com/owner/repo
/// git@github.com:owner/repo.git                -> github.com/owner/repo
/// ssh://git@gitlab.example.com:2222/g/sub/r    -> gitlab.example.com/g/sub/r
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (authority, path) = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?
    } else if let Some((authority, path)) = url.split_once(':') {
        // scp-like syntax: [user@]host:path (a local path such as C:\repo has no '@' and a 1-char host)
        if authority.contains('/') || authority.len() <= 1 {
            return None;
        }
        (authority, path)
    } else {
        return None; // local path
    };
    let host = authority.rsplit('@').next()?; // drop credentials
    let host = host.split(':').next()?.to_ascii_lowercase(); // drop port
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_are_normalized() {
        let cases = [
            ("https://github.com/example-org/repo-tweaks.git", "github.com/example-org/repo-tweaks"),
            ("https://user:dummy-token@GitHub.com/example-org/repo-tweaks.git", "github.com/example-org/repo-tweaks"),
            ("git@github.com:example-org/repo-tweaks.git", "github.com/example-org/repo-tweaks"),
            ("ssh://git@gitlab.example.com:2222/group/sub/game.git", "gitlab.example.com/group/sub/game"),
            ("https://gitlab.example.com:8443/group/game/", "gitlab.example.com/group/game"),
        ];
        for (url, expected) in cases {
            assert_eq!(normalize_remote(url).as_deref(), Some(expected), "{url}");
        }
    }

    #[test]
    fn local_paths_are_not_remotes() {
        for url in ["/srv/git/game.git", "../game", r"C:\repos\game", "C:/repos/game", ""] {
            assert_eq!(normalize_remote(url), None, "{url}");
        }
    }

    #[test]
    fn config_last_value_wins() {
        let config = GitConfig::parse(
            "user.name global user\nrepo-tweaks.server http://a\nuser.name local user\nremote.origin.url git@h:o/r.git\n",
        );
        assert_eq!(config.get(KEY_USER).as_deref(), Some("local user"));
        assert_eq!(config.get(KEY_SERVER).as_deref(), Some("http://a"));
        assert_eq!(config.get(KEY_TOKEN), None);
        assert!(config.has_remote());
        assert!(!GitConfig::parse("user.name a\n").has_remote());
    }

    #[test]
    fn repo_name_prefers_remote_over_folder() {
        let config = GitConfig::parse("remote.origin.url https://github.com/o/game.git\n");
        let info = RepoInfo::build(&config, Some("/work/game-main".into()), None, None);
        assert_eq!(info.repo.as_deref(), Some("game"));
        assert_eq!(info.remote.as_deref(), Some("github.com/o/game"));

        let info = RepoInfo::build(&GitConfig::parse(""), Some("/work/game-main".into()), None, None);
        assert_eq!(info.repo.as_deref(), Some("game-main"));
        assert_eq!(info.remote, None);
    }
}
