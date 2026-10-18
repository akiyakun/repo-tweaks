//! Turning user-given paths into repository-relative paths (the form Git LFS locks use).

use std::path::{Path, PathBuf};

use crate::git;

/// Where the repository is and where the current directory sits inside it.
pub struct RepoContext {
    /// Repository root as git prints it (forward slashes, also on Windows)
    pub root: String,
    /// Current directory relative to the root, with a trailing '/' (empty at the root)
    pub prefix: String,
}

impl RepoContext {
    pub fn current() -> Result<Self, String> {
        let output = git::output(&["rev-parse", "--show-toplevel", "--show-prefix"])
            .map_err(|_| "git が見つかりません。Git をインストールしてください".to_string())?;
        if !output.status.success() {
            return Err("Git のリポジトリの中で実行してください".into());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let root = lines.next().unwrap_or("").trim().to_string();
        let prefix = lines.next().unwrap_or("").trim().to_string();
        Ok(RepoContext { root, prefix })
    }

    /// Repository-relative path for what the user typed (relative to the current directory, or absolute).
    pub fn repo_path(&self, input: &str) -> Result<String, String> {
        to_repo_path(&self.root, &self.prefix, input)
    }

    /// Location on disk of a repository-relative path.
    pub fn disk_path(&self, repo_path: &str) -> PathBuf {
        Path::new(&self.root).join(repo_path)
    }

    /// Rewrite paths to the letter case git tracks them with.
    ///
    /// Mac and Windows file systems ignore case, so `assets/main.unity` opens `Assets/Main.unity`,
    /// but Git LFS locks compare paths exactly: without this, a second lock could be taken
    /// under a different spelling next to someone else's.
    pub fn tracked_case(&self, paths: Vec<String>) -> Vec<String> {
        let mut args = vec!["ls-files".to_string(), "-z".into(), "--full-name".into(), "--".into()];
        args.extend(paths.iter().map(|p| format!(":(top,icase,literal){p}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let tracked: Vec<String> = git::output_in(&self.root, &args)
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
            .unwrap_or_default();
        match_case(paths, &tracked)
    }
}

/// Replace each path with the tracked path that equals it ignoring case (only when that match is unique).
fn match_case(paths: Vec<String>, tracked: &[String]) -> Vec<String> {
    paths
        .into_iter()
        .map(|path| {
            if tracked.contains(&path) {
                return path;
            }
            let lower = path.to_lowercase();
            let mut matches = tracked.iter().filter(|t| t.to_lowercase() == lower);
            match (matches.next(), matches.next()) {
                (Some(only), None) => only.clone(),
                _ => path,
            }
        })
        .collect()
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/') || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

/// Git Bash / MSYS spells `C:/x` as `/c/x`; turn it back when the repository root has a drive letter.
fn from_msys(input: &str, root: &str) -> Option<String> {
    let root_has_drive = root.as_bytes().get(1) == Some(&b':');
    let b = input.as_bytes();
    let msys = b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/');
    (root_has_drive && msys).then(|| format!("{}:{}", b[1] as char, &input[2..]))
}

fn to_repo_path(root: &str, prefix: &str, input: &str) -> Result<String, String> {
    let input = input.trim().replace('\\', "/");
    if input.is_empty() {
        return Err("パスが空です".into());
    }
    let input = from_msys(&input, root).unwrap_or(input);
    let joined = if is_absolute(&input) {
        let root = root.trim_end_matches('/');
        // Windows paths are case-insensitive; macOS usually is too, so compare loosely.
        let head = input.get(..root.len()).filter(|h| h.eq_ignore_ascii_case(root));
        match (head, input.as_bytes().get(root.len())) {
            (Some(_), Some(b'/')) => input[root.len() + 1..].to_string(),
            _ => return Err(format!("リポジトリの外のパスです: {input}")),
        }
    } else {
        format!("{prefix}{input}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("リポジトリの外のパスです: {input}"));
                }
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(format!("ファイルを指定してください: {input}"));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/work/game";

    #[test]
    fn relative_paths_use_current_directory() {
        assert_eq!(to_repo_path(ROOT, "", "README.md").unwrap(), "README.md");
        assert_eq!(to_repo_path(ROOT, "Assets/", "Scenes/Main.unity").unwrap(), "Assets/Scenes/Main.unity");
        assert_eq!(to_repo_path(ROOT, "Assets/Scenes/", "../Art/./hero.psd").unwrap(), "Assets/Art/hero.psd");
    }

    #[test]
    fn windows_separators_and_absolute_paths() {
        assert_eq!(to_repo_path(ROOT, "", r"Assets\Scenes\Main.unity").unwrap(), "Assets/Scenes/Main.unity");
        assert_eq!(to_repo_path(ROOT, "x/", "/work/game/a/b.txt").unwrap(), "a/b.txt");
        let win_root = "C:/local/dev/game";
        assert_eq!(to_repo_path(win_root, "", r"c:\local\dev\game\Assets\a.psd").unwrap(), "Assets/a.psd");
        // Git Bash style
        assert_eq!(to_repo_path(win_root, "", "/c/local/dev/game/Assets/a.psd").unwrap(), "Assets/a.psd");
        // On Mac/Linux a one-letter top folder is just a folder
        assert!(to_repo_path(ROOT, "", "/c/local/file").is_err());
    }

    #[test]
    fn case_follows_tracked_paths() {
        let tracked = vec!["Assets/Main.unity".to_string(), "README.md".into(), "a.txt".into(), "A.txt".into()];
        let got = match_case(
            vec!["assets/main.unity".into(), "README.md".into(), "new.psd".into(), "a.TXT".into()],
            &tracked,
        );
        // Untracked files stay as typed; ambiguous matches are left alone
        assert_eq!(got, ["Assets/Main.unity", "README.md", "new.psd", "a.TXT"]);
    }

    #[test]
    fn rejects_paths_outside_the_repository() {
        assert!(to_repo_path(ROOT, "", "../other/file").is_err());
        assert!(to_repo_path(ROOT, "", "/work/gamex/file").is_err());
        assert!(to_repo_path(ROOT, "", "/elsewhere/file").is_err());
        assert!(to_repo_path(ROOT, "", ".").is_err());
        assert!(to_repo_path(ROOT, "", "  ").is_err());
    }
}
