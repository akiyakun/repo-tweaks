//! Files that must be locked before they are edited: those marked `lockable` in .gitattributes.
//!
//! Git LFS already keeps such files read-only until you lock them. This adds the checks on top:
//! `status` (which of them are locked or changed) and the pre-commit `check` (a commit may change or
//! delete one only while you hold its lock; new files need no lock unless someone else holds that path).

use std::collections::{BTreeMap, HashSet};

use crate::git::{self, GitConfig};
use crate::lfs;
use crate::locks::LockList;
use crate::ops::{LockIndex, State};
use crate::paths::RepoContext;

/// Pathspec for files with the `lockable` attribute (run from the repository root)
const LOCKABLE: &str = ":(attr:lockable)";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Modified,
    Deleted,
    /// New file: needs no lock (nobody else can be editing it), unless someone else locked the path first
    Added,
}

/// Why a change to a lockable file may not be committed
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Problem {
    NotLocked,
    HeldByOther,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileStatus {
    pub path: String,
    pub state: State,
    pub change: Option<Change>,
}

impl FileStatus {
    pub fn problem(&self) -> Option<Problem> {
        match (self.change, &self.state) {
            (None, _) | (_, State::Ours(_)) | (Some(Change::Added), State::Free) => None,
            (_, State::Free) => Some(Problem::NotLocked),
            (_, State::Theirs(_)) => Some(Problem::HeldByOther),
        }
    }
}

pub struct StatusReport {
    /// Every lockable file (tracked, or deleted / changed in the working tree), sorted by path
    pub files: Vec<FileStatus>,
}

/// Lockable files with their lock state and local changes (index and working tree).
pub fn status(config: &GitConfig) -> Result<StatusReport, String> {
    let ctx = RepoContext::current()?;
    lfs::require_remote(config)?;
    let tracked = run_git(&ctx, &["ls-files", "-z", "--", LOCKABLE])?;
    let changes = parse_status(&run_git(
        &ctx,
        &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=no", "--", LOCKABLE],
    )?);
    let locks = lfs::query_locks()?;
    Ok(StatusReport { files: combine(split_z(&tracked), changes, &locks) })
}

fn combine(tracked: Vec<String>, changes: Vec<(String, Change)>, locks: &LockList) -> Vec<FileStatus> {
    let mut files: BTreeMap<String, Option<Change>> = tracked.into_iter().map(|p| (p, None)).collect();
    for (path, change) in changes {
        files.insert(path, Some(change));
    }
    let index = LockIndex::new(locks);
    files.into_iter().map(|(path, change)| FileStatus { state: index.state(&path), path, change }).collect()
}

/// What the pre-commit check found
#[derive(Debug, PartialEq)]
pub enum CheckResult {
    /// No staged change to a lockable file: nothing was asked over the network
    Nothing,
    /// Every such change is covered by our own lock
    Passed,
    /// Changes that may not be committed
    Blocked(Vec<FileStatus>),
    /// The locks could not be read (offline, no remote, ...): the caller lets the commit through
    Unverified { files: usize, reason: String },
}

/// Check the staged changes before a commit.
pub fn check_staged(config: &GitConfig) -> CheckResult {
    let unverified = |files, reason| CheckResult::Unverified { files, reason };
    let ctx = match RepoContext::current() {
        Ok(ctx) => ctx,
        Err(reason) => return unverified(0, reason),
    };
    let staged = match staged_changes(&ctx) {
        Ok(staged) => staged,
        Err(reason) => return unverified(0, reason),
    };
    if staged.is_empty() {
        return CheckResult::Nothing;
    }
    let locks = match lfs::require_remote(config).and_then(|_| lfs::query_locks()) {
        Ok(locks) => locks,
        Err(reason) => return unverified(staged.len(), reason),
    };
    judge(staged, &locks)
}

fn judge(staged: Vec<(String, Change)>, locks: &LockList) -> CheckResult {
    let index = LockIndex::new(locks);
    let blocked: Vec<FileStatus> = staged
        .into_iter()
        .map(|(path, change)| FileStatus { state: index.state(&path), path, change: Some(change) })
        .filter(|f| f.problem().is_some())
        .collect();
    if blocked.is_empty() { CheckResult::Passed } else { CheckResult::Blocked(blocked) }
}

/// Staged changes to lockable files (new files too: someone else may have locked that path).
///
/// While finishing a merge, files taken unchanged from the other side are left out too:
/// only what differs from both parents was edited here.
fn staged_changes(ctx: &RepoContext) -> Result<Vec<(String, Change)>, String> {
    let diff = ["diff", "--cached", "--name-status", "-z", "--no-renames"];
    let mut staged = parse_name_status(&run_git(ctx, &[&diff[..], &["--", LOCKABLE]].concat())?);
    if !staged.is_empty() && merging(ctx) {
        let theirs: HashSet<String> =
            parse_name_status(&run_git(ctx, &[&diff[..], &["MERGE_HEAD", "--", LOCKABLE]].concat())?)
                .into_iter()
                .map(|(path, _)| path)
                .collect();
        staged.retain(|(path, _)| theirs.contains(path));
    }
    Ok(staged)
}

fn merging(ctx: &RepoContext) -> bool {
    git::output_in(&ctx.root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok_and(|o| o.status.success())
}

/// Run git in the repository root (so pathspecs and printed paths are root-relative).
fn run_git(ctx: &RepoContext, args: &[&str]) -> Result<String, String> {
    let output = git::run_in(&ctx.root, args)?;
    if !output.status.success() {
        return Err(format!(
            "git {} に失敗しました（{}）",
            args[0],
            lfs::first_line(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn split_z(text: &str) -> Vec<String> {
    text.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// `git diff --name-status -z --no-renames`: "M\0path\0D\0path\0..."
fn parse_name_status(text: &str) -> Vec<(String, Change)> {
    let fields = split_z(text);
    fields
        .chunks(2)
        .filter_map(|pair| match pair {
            [status, path] => Some((path.clone(), match status.as_bytes().first() {
                Some(b'A') => Change::Added,
                Some(b'D') => Change::Deleted,
                _ => Change::Modified,
            })),
            _ => None,
        })
        .collect()
}

/// `git status --porcelain=v1 -z --no-renames`: "XY path\0" (X = index, Y = working tree)
fn parse_status(text: &str) -> Vec<(String, Change)> {
    split_z(text)
        .into_iter()
        .filter_map(|entry| {
            let (xy, path) = (entry.get(..2)?, entry.get(3..)?);
            let change = match xy.as_bytes() {
                [b'A', _] => Change::Added,
                [b'D', _] | [_, b'D'] => Change::Deleted,
                _ => Change::Modified,
            };
            Some((path.to_string(), change))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locks::parse_verify_json;

    fn locks() -> LockList {
        parse_verify_json(
            r#"{"ours":[{"id":"1","path":"Assets/Mine.unity","owner":{"name":"alice"},"locked_at":"2026-10-18T00:00:00Z"}],
                "theirs":[{"id":"2","path":"Assets/Art/hero.psd","owner":{"name":"bob"},"locked_at":"2026-10-18T00:00:00Z"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_git_output() {
        assert_eq!(
            parse_name_status("M\0a.unity\0D\0b c.psd\0A\0new.unity\0T\0d.prefab\0"),
            [
                ("a.unity".to_string(), Change::Modified),
                ("b c.psd".into(), Change::Deleted),
                ("new.unity".into(), Change::Added),
                ("d.prefab".into(), Change::Modified),
            ]
        );
        assert_eq!(
            parse_status("M  a.unity\0 M b.unity\0D  c.psd\0 D d.psd\0A  new.unity\0AM e.unity\0UU f.unity\0"),
            [
                ("a.unity".to_string(), Change::Modified),
                ("b.unity".into(), Change::Modified),
                ("c.psd".into(), Change::Deleted),
                ("d.psd".into(), Change::Deleted),
                ("new.unity".into(), Change::Added),
                ("e.unity".into(), Change::Added),
                ("f.unity".into(), Change::Modified),
            ]
        );
    }

    #[test]
    fn changes_need_our_lock_but_new_files_do_not() {
        let staged = vec![
            ("Assets/Mine.unity".to_string(), Change::Modified),
            ("Assets/Art/hero.psd".into(), Change::Deleted),
            ("Assets/Free.prefab".into(), Change::Modified),
        ];
        match judge(staged, &locks()) {
            CheckResult::Blocked(files) => {
                let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.problem())).collect();
                assert_eq!(
                    got,
                    [("Assets/Art/hero.psd", Some(Problem::HeldByOther)), ("Assets/Free.prefab", Some(Problem::NotLocked))]
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(judge(vec![("assets/mine.unity".into(), Change::Modified)], &locks()), CheckResult::Passed);
        // New files need no lock, unless someone else already locked that path
        assert_eq!(judge(vec![("Assets/New.prefab".into(), Change::Added)], &locks()), CheckResult::Passed);
        match judge(vec![("Assets/Art/hero.psd".into(), Change::Added)], &locks()) {
            CheckResult::Blocked(files) => assert_eq!(files[0].problem(), Some(Problem::HeldByOther)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn status_merges_tracked_files_and_changes() {
        let files = combine(
            vec!["Assets/Mine.unity".into(), "Assets/Art/hero.psd".into(), "b.unity".into()],
            vec![("Assets/Art/hero.psd".into(), Change::Modified), ("gone.unity".into(), Change::Deleted)],
            &locks(),
        );
        let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.change, f.problem())).collect();
        assert_eq!(
            got,
            [
                ("Assets/Art/hero.psd", Some(Change::Modified), Some(Problem::HeldByOther)),
                ("Assets/Mine.unity", None, None),
                ("b.unity", None, None),
                ("gone.unity", Some(Change::Deleted), Some(Problem::NotLocked)),
            ]
        );
        assert!(matches!(files[1].state, State::Ours(_)));
    }
}
