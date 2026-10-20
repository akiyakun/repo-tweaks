//! Small helpers that call the `git` command.
//!
//! Each `git` process costs tens of milliseconds on Windows, and `notify` runs inside a hook,
//! so values are read in as few calls as possible: one `git config` and one `git rev-parse`.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const KEY_SERVER: &str = "repo-tweaks.server";
pub const KEY_TOKEN: &str = "repo-tweaks.token";
/// Git lowercases keys in `--get-regexp` output, so this is `staleDays` as users write it.
pub const KEY_STALE_DAYS: &str = "repo-tweaks.staledays";
pub const KEY_NOTIFY: &str = "repo-tweaks.notify";
const KEY_USER: &str = "user.name";
const KEY_REMOTE: &str = "remote.origin.url";

static HIDE_CONSOLE: AtomicBool = AtomicBool::new(false);
/// Seconds before a git process is stopped; 0 means no limit (the CLI).
static TIMEOUT_SECS: AtomicU64 = AtomicU64::new(0);

/// Stop each git process from flashing a console window on Windows (for the GUI; the CLI keeps
/// its console so git can still ask for credentials there). No effect on other platforms.
pub fn hide_console_windows() {
    HIDE_CONSOLE.store(true, Ordering::Relaxed);
}

/// For the GUI: stop any git process that runs longer than `limit`, together with what it started
/// (git-lfs, ssh). Those processes also lose the terminal, so they fail instead of asking on it.
/// The CLI does not call this: there, git may ask for a password and the user can press Ctrl+C.
pub fn set_timeout(limit: Duration) {
    TIMEOUT_SECS.store(limit.as_secs().max(1), Ordering::Relaxed);
}

fn command() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("git");
    #[cfg(windows)]
    if HIDE_CONSOLE.load(Ordering::Relaxed) {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Run to the end, or stop the whole process tree when the GUI's time limit passes.
fn exec(command: Command) -> io::Result<Output> {
    match TIMEOUT_SECS.load(Ordering::Relaxed) {
        0 => { command }.output(),
        secs => exec_limited(command, secs),
    }
}

fn exec_limited(mut command: Command, secs: u64) -> io::Result<Output> {
    detach_from_terminal(&mut command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());
    let deadline = Instant::now() + Duration::from_secs(secs);
    let timed_out = || {
        let message = format!(
            "git が {secs} 秒たっても終わらないため止めました。ネットワークや、ログインの画面が出ていないかを確かめてください"
        );
        io::Error::new(io::ErrorKind::TimedOut, message)
    };
    // Waited for here, not in another thread: until try_wait reaps it, the pid (and so the process
    // group to kill) cannot be given to another process.
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(timed_out());
        }
        std::thread::sleep(POLL);
    };
    // Something git started may still hold the pipes; never wait past the deadline for them.
    let left = |d: Instant| d.saturating_duration_since(Instant::now());
    match (stdout.recv_timeout(left(deadline)), stderr.recv_timeout(left(deadline))) {
        (Ok(stdout), Ok(stderr)) => Ok(Output { status, stdout, stderr }),
        _ => Err(timed_out()),
    }
}

const POLL: Duration = Duration::from_millis(10);

fn read_in_background(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buffer);
        }
        let _ = sender.send(buffer);
    });
    receiver
}

/// A new session has no controlling terminal, so ssh cannot stop to ask for a passphrase or a host key.
#[cfg(unix)]
fn detach_from_terminal(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe, as pre_exec requires.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_from_terminal(_command: &mut Command) {}

/// The process was made a session (and group) leader, so the group id is its pid.
#[cfg(unix)]
fn kill_tree(pid: u32) {
    // SAFETY: plain system call; a negative pid addresses the whole process group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(any(unix, windows)))]
fn kill_tree(_pid: u32) {}

/// What it means when `git` cannot be started: it is not installed (or not on PATH).
pub const GIT_MISSING: &str = "git が見つかりません。Git をインストールしてください";

/// A message for a git process that could not be run to the end.
pub fn spawn_error(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => GIT_MISSING.into(),
        // exec's own message
        io::ErrorKind::TimedOut => error.to_string(),
        _ => format!("git を実行できませんでした（{error}）"),
    }
}

/// Like `output`, with a failure to run git turned into a message.
pub fn run(args: &[&str]) -> Result<Output, String> {
    output(args).map_err(|e| spawn_error(&e))
}

/// Like `output_in`, with a failure to run git turned into a message.
/// A folder that git cannot be started in is checked first, so it is not mistaken for a missing git.
pub fn run_in(dir: &str, args: &[&str]) -> Result<Output, String> {
    check_dir(dir)?;
    output_in(dir, args).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => format!("フォルダを開けません（権限がありません）: {dir}"),
        _ => spawn_error(&e),
    })
}

fn check_dir(dir: &str) -> Result<(), String> {
    match std::fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!("フォルダではありません: {dir}")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(format!("フォルダが見つかりません: {dir}")),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            Err(format!("フォルダを開けません（権限がありません）: {dir}"))
        }
        Err(e) => Err(format!("フォルダを開けません（{e}）: {dir}")),
    }
}

/// Run `git <args>` and return the raw output (for callers that need stderr).
pub fn output(args: &[&str]) -> io::Result<Output> {
    let mut command = command();
    command.args(args);
    exec(command)
}

/// Like `output`, but run in `dir` (e.g. the repository root, for root-relative paths).
pub fn output_in(dir: &str, args: &[&str]) -> io::Result<Output> {
    let mut command = command();
    command.current_dir(dir).args(args);
    exec(command)
}

/// Run `git <args>` in `dir` and return stdout, or None on any failure.
fn git_in(dir: &str, args: &[&str]) -> Option<String> {
    stdout(output_in(dir, args))
}

/// Run `git <args>` and return stdout, or None on any failure.
fn git(args: &[&str]) -> Option<String> {
    stdout(output(args))
}

fn stdout(output: io::Result<Output>) -> Option<String> {
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The git config values this tool uses. Later entries (local over global) win, as in git itself.
#[derive(Default)]
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
        Self::in_dir(".", config)
    }

    /// Like `current`, for the repository at `dir` (the GUI). `config` must be read there too.
    pub fn in_dir(dir: &str, config: &GitConfig) -> Self {
        let (toplevel, commit, branch) = match git_in(dir, &["rev-parse", "--show-toplevel", "HEAD", "--abbrev-ref", "HEAD"]) {
            Some(out) => {
                let mut lines = out.lines().map(|l| Some(l.trim().to_string()));
                (lines.next().flatten(), lines.next().flatten(), lines.next().flatten())
            }
            // No commits yet: HEAD does not resolve, but the repository may still exist.
            None => (git_in(dir, &["rev-parse", "--show-toplevel"]).map(|s| s.trim().to_string()), None, None),
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

    #[cfg(unix)]
    #[test]
    fn time_limit_stops_the_whole_process_tree() {
        let pid_file = std::env::temp_dir().join(format!("repo-tweaks-timeout-{}", std::process::id()));
        let mut command = Command::new("sh");
        // The shell stands in for git, the background sleep for git-lfs / ssh started by it.
        command.args(["-c", &format!("sleep 30 & echo $! > {}; wait", pid_file.display())]);
        let started = std::time::Instant::now();
        let err = exec_limited(command, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(err.to_string().contains("1 秒たっても終わらない"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let grandchild: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        std::fs::remove_file(&pid_file).ok();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!is_running(grandchild), "the background process was left running");
    }

    /// Killed processes may linger as zombies (state Z) until reaped, which `kill(pid, 0)` still sees.
    #[cfg(unix)]
    fn is_running(pid: i32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat.rsplit(')').next().and_then(|rest| rest.split_whitespace().next()) != Some("Z"),
            Err(_) if Path::new("/proc/self").exists() => false,
            // SAFETY: signal 0 only checks whether the process exists (macOS, which has no /proc).
            Err(_) => (unsafe { libc::kill(pid, 0) }) == 0,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_holding_the_pipes_does_not_hang() {
        // The shell (git) ends at once; the background sleep (git-lfs, ssh) keeps stdout open
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 3 & echo started"]);
        let started = std::time::Instant::now();
        let err = exec_limited(command, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn finished_commands_keep_their_output() {
        let mut command = Command::new("git");
        command.arg("--version");
        let output = exec_limited(command, 30).unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("git version"));
    }

    #[test]
    fn missing_or_wrong_folders_are_named() {
        let missing = std::env::temp_dir().join("repo-tweaks-missing").join(std::process::id().to_string());
        let missing = missing.to_str().unwrap();
        assert_eq!(run_in(missing, &["status"]).unwrap_err(), format!("フォルダが見つかりません: {missing}"));
        let file = std::env::current_exe().unwrap();
        let file = file.to_str().unwrap();
        assert_eq!(run_in(file, &["status"]).unwrap_err(), format!("フォルダではありません: {file}"));
    }

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
