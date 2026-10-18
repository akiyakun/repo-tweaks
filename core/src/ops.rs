//! Locking and unlocking files.
//!
//! Work is split in two so callers can ask the user before anything changes:
//! 1. `plan_lock` / `plan_unlock` resolve paths, add `.meta` pairs, and look up who holds each lock.
//! 2. `Plan::lock` / `Plan::unlock` run `git lfs lock` / `git lfs unlock`.
//!
//! Git LFS keeps a local lock cache, so files are passed to one `git lfs` call instead of running
//! several in parallel. Paths the user named go first; `.meta` partners follow only for files whose
//! own lock or unlock went through, so an asset and its `.meta` never end up held by different people.

use std::collections::HashMap;

use serde::Deserialize;

use crate::git::{self, GitConfig};
use crate::lfs;
use crate::locks::{Lock, LockList};
use crate::paths::RepoContext;

const META: &str = ".meta";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Free,
    Ours(Lock),
    Theirs(Lock),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// Repository-relative path (the lock's own spelling when a lock exists)
    pub path: String,
    /// Set when this is the Unity `.meta` partner added automatically for that path
    pub pair_of: Option<String>,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Carries the lock owner's name as the server reports it (the user's GitHub / GitLab name)
    Locked(Option<String>),
    /// Lock: the current user already holds it
    AlreadyOurs,
    /// Lock: someone else holds it
    HeldByOther(Lock),
    /// Lock: the file is not in the working tree (Git LFS requires it)
    NoSuchFile,
    /// Lock: a folder was given; only files can be locked
    IsDirectory,
    Unlocked,
    /// Unlock with --force of someone else's lock
    ForceUnlocked(Lock),
    /// Unlock: nobody holds it
    NotLocked,
    /// Unlock without --force: someone else holds it
    OthersLock(Lock),
    /// `.meta` partner left alone because its asset could not be locked / unlocked
    SkippedPair,
    Failed(String),
}

impl Outcome {
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Outcome::HeldByOther(_) | Outcome::NoSuchFile | Outcome::IsDirectory | Outcome::OthersLock(_) | Outcome::Failed(_)
        )
    }
}

pub struct Plan {
    ctx: RepoContext,
    force: bool,
    pub targets: Vec<Target>,
}

/// Check the repository and remote, resolve paths, and fetch the current locks once.
fn prepare(config: &GitConfig, inputs: &[String]) -> Result<(RepoContext, Vec<String>, LockList), String> {
    if inputs.is_empty() {
        return Err("ファイルを指定してください".into());
    }
    let ctx = RepoContext::current()?;
    lfs::require_remote(config)?;
    let paths = inputs.iter().map(|p| ctx.repo_path(p)).collect::<Result<Vec<_>, _>>()?;
    let paths = ctx.tracked_case(paths);
    let locks = lfs::query_locks()?;
    Ok((ctx, paths, locks))
}

pub fn plan_lock(config: &GitConfig, inputs: &[String], with_meta: bool) -> Result<Plan, String> {
    let (ctx, paths, locks) = prepare(config, inputs)?;
    // Pair with the .meta (or asset) only when that file exists
    let targets = collect(paths, with_meta, |p| ctx.disk_path(p).is_file());
    Ok(Plan { targets: with_state(targets, &locks), ctx, force: false })
}

/// `force`: also take other people's locks (the caller confirms first, using `others_locks`).
pub fn plan_unlock(config: &GitConfig, inputs: &[String], with_meta: bool, force: bool) -> Result<Plan, String> {
    let (ctx, paths, locks) = prepare(config, inputs)?;
    // Pair only with a partner we may actually unlock: our own, or anyone's under --force
    let targets = collect(paths, with_meta, |p| match find(&locks, p) {
        Some(State::Ours(_)) => true,
        Some(State::Theirs(_)) => force,
        _ => false,
    });
    Ok(Plan { targets: with_state(targets, &locks), ctx, force })
}

/// "Main.unity" <-> "Main.unity.meta"
fn meta_partner(path: &str) -> Option<String> {
    match path.strip_suffix(META) {
        Some(base) if !base.is_empty() && !base.ends_with('/') => Some(base.to_string()),
        Some(_) => None,
        None => Some(format!("{path}{META}")),
    }
}

/// Given paths (deduplicated, in order), each followed by its partner when `include_pair` says so.
fn collect(paths: Vec<String>, with_meta: bool, include_pair: impl Fn(&str) -> bool) -> Vec<(String, Option<String>)> {
    fn push(out: &mut Vec<(String, Option<String>)>, path: String, pair_of: Option<String>) {
        match out.iter_mut().find(|(p, _)| *p == path) {
            // Named explicitly as well: treat it as the user's own choice
            Some(existing) if pair_of.is_none() => existing.1 = None,
            Some(_) => {}
            None => out.push((path, pair_of)),
        }
    }
    let mut out = Vec::new();
    for path in paths {
        let partner = with_meta.then(|| meta_partner(&path)).flatten().filter(|p| include_pair(p));
        push(&mut out, path.clone(), None);
        if let Some(partner) = partner {
            push(&mut out, partner, Some(path));
        }
    }
    out
}

/// Lock state of a path. Exact match first, then ignoring case (a lock taken under another spelling).
fn find(locks: &LockList, path: &str) -> Option<State> {
    let lower = path.to_lowercase();
    for exact in [true, false] {
        let hit = |l: &&Lock| if exact { l.path == path } else { l.path.to_lowercase() == lower };
        if let Some(lock) = locks.ours.iter().find(hit) {
            return Some(State::Ours(lock.clone()));
        }
        if let Some(lock) = locks.theirs.iter().find(hit) {
            return Some(State::Theirs(lock.clone()));
        }
    }
    None
}

fn with_state(targets: Vec<(String, Option<String>)>, locks: &LockList) -> Vec<Target> {
    let mut renamed: HashMap<String, String> = HashMap::new();
    let mut out: Vec<Target> = targets
        .into_iter()
        .map(|(typed, pair_of)| {
            let state = find(locks, &typed).unwrap_or(State::Free);
            let path = match &state {
                State::Ours(l) | State::Theirs(l) => l.path.clone(),
                State::Free => typed.clone(),
            };
            renamed.insert(typed, path.clone());
            Target { path, pair_of, state }
        })
        .collect();
    // Keep partners pointing at their asset's final spelling
    for t in &mut out {
        if let Some(base) = t.pair_of.take() {
            t.pair_of = Some(renamed.get(&base).cloned().unwrap_or(base));
        }
    }
    out
}

/// What to do with one target: settle it now, or hand its path to git lfs.
enum Step {
    Done(Outcome),
    Run,
}

impl Plan {
    /// Other people's locks among the targets (what `unlock --force` would remove).
    pub fn others_locks(&self) -> Vec<&Lock> {
        self.targets
            .iter()
            .filter_map(|t| match &t.state {
                State::Theirs(lock) => Some(lock),
                _ => None,
            })
            .collect()
    }

    pub fn lock(self) -> Vec<(Target, Outcome)> {
        let ctx = &self.ctx;
        let decide = |t: &Target| match &t.state {
            State::Ours(_) => Step::Done(Outcome::AlreadyOurs),
            State::Theirs(lock) => Step::Done(Outcome::HeldByOther(lock.clone())),
            State::Free => {
                let disk = ctx.disk_path(&t.path);
                if disk.is_dir() {
                    Step::Done(Outcome::IsDirectory)
                } else if !disk.is_file() {
                    Step::Done(Outcome::NoSuchFile)
                } else {
                    Step::Run
                }
            }
        };
        let ok = |o: &Outcome| matches!(o, Outcome::Locked(_) | Outcome::AlreadyOurs);
        let outcomes = self.run_in_two_rounds(decide, ok, run_lock);
        self.targets.into_iter().zip(outcomes).collect()
    }

    pub fn unlock(self) -> Vec<(Target, Outcome)> {
        let (ctx, force) = (&self.ctx, self.force);
        let decide = |t: &Target| match &t.state {
            State::Free => Step::Done(Outcome::NotLocked),
            State::Theirs(lock) if !force => Step::Done(Outcome::OthersLock(lock.clone())),
            // Without --force Git LFS asks `git status` about the file and fails on a missing one
            State::Ours(_) if !force && !ctx.disk_path(&t.path).exists() => Step::Done(Outcome::Failed(
                "手元にファイルが無いため解除できません（削除・移動したファイル）。--force を付けると解除できます".into(),
            )),
            _ => Step::Run,
        };
        let ok = |o: &Outcome| matches!(o, Outcome::Unlocked | Outcome::ForceUnlocked(_) | Outcome::NotLocked);
        let outcomes = self.run_in_two_rounds(decide, ok, |ctx, paths| run_unlock(ctx, paths, force));
        // Report whose lock was removed by force
        self.targets
            .into_iter()
            .zip(outcomes)
            .map(|(t, o)| match (&t.state, o) {
                (State::Theirs(lock), Outcome::Unlocked) => {
                    let lock = lock.clone();
                    (t, Outcome::ForceUnlocked(lock))
                }
                (State::Theirs(_), Outcome::Failed(reason)) => (t, Outcome::Failed(explain_force_failure(&reason))),
                (_, o) => (t, o),
            })
            .collect()
    }

    /// Round 1: the paths the user named. Round 2: `.meta` partners whose asset succeeded (`ok`).
    /// Returns one outcome per target, in order.
    fn run_in_two_rounds(
        &self,
        decide: impl Fn(&Target) -> Step,
        ok: impl Fn(&Outcome) -> bool,
        mut run: impl FnMut(&RepoContext, &[&str]) -> HashMap<String, Outcome>,
    ) -> Vec<Outcome> {
        let mut outcomes: Vec<Option<Outcome>> = vec![None; self.targets.len()];
        for round in [false, true] {
            let mut to_run = Vec::new();
            for (i, t) in self.targets.iter().enumerate() {
                if t.pair_of.is_some() != round {
                    continue;
                }
                if let Some(base) = &t.pair_of {
                    let base_ok = self.targets.iter().zip(&outcomes).any(|(b, o)| &b.path == base && o.as_ref().is_some_and(&ok));
                    if !base_ok {
                        outcomes[i] = Some(Outcome::SkippedPair);
                        continue;
                    }
                }
                match decide(t) {
                    Step::Done(o) => outcomes[i] = Some(o),
                    Step::Run => to_run.push(i),
                }
            }
            if to_run.is_empty() {
                continue;
            }
            let paths: Vec<&str> = to_run.iter().map(|&i| self.targets[i].path.as_str()).collect();
            let mut results = run(&self.ctx, &paths);
            for i in to_run {
                let outcome = results
                    .remove(&self.targets[i].path)
                    .unwrap_or_else(|| Outcome::Failed("git lfs から結果が返りませんでした".into()));
                outcomes[i] = Some(outcome);
            }
        }
        outcomes.into_iter().map(|o| o.unwrap_or(Outcome::SkippedPair)).collect()
    }
}

/// `reason` is already explained ("<hint>（git lfs: <line>）"); swap the hint for one about forcing.
fn explain_force_failure(reason: &str) -> String {
    let lower = reason.to_lowercase();
    // GitHub answers "You must have admin access to force delete a lock" (checked 2026-10-18)
    let denied = ["admin access", "403", "forbidden", "permission"].iter().any(|w| lower.contains(w));
    if !denied {
        return reason.to_string();
    }
    let original = reason.find("（git lfs:").map(|i| &reason[i..]).unwrap_or(reason);
    format!("他人のロックを強制解除する権限がありません（GitHub ではリポジトリの管理者権限が必要です。持ち主に解除を頼んでください）。{original}")
}

/// `git lfs lock --json <paths>` prints the locks it took; failures go to stderr as
/// "Locking <path> failed: <reason>" (git lfs keeps going after a failure).
fn run_lock(ctx: &RepoContext, paths: &[&str]) -> HashMap<String, Outcome> {
    let mut args = vec!["lfs", "lock", "--json"];
    args.extend(paths);
    match git::output_in(&ctx.root, &args) {
        Err(_) => all_failed(paths, "git が見つかりません"),
        Ok(out) => lock_results(paths, &String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr)),
    }
}

#[derive(Deserialize)]
struct LockedPath {
    path: String,
    #[serde(default)]
    owner: Option<LockOwner>,
}

#[derive(Deserialize)]
struct LockOwner {
    #[serde(default)]
    name: String,
}

fn lock_results(paths: &[&str], stdout: &str, stderr: &str) -> HashMap<String, Outcome> {
    let locked: Vec<LockedPath> = serde_json::from_str(stdout.trim()).unwrap_or_default();
    paths
        .iter()
        .map(|&path| {
            let outcome = if let Some(lock) = locked.iter().find(|l| l.path == path) {
                Outcome::Locked(lock.owner.as_ref().map(|o| o.name.clone()).filter(|n| !n.is_empty()))
            } else {
                // The stderr line naming this path, else the first error line
                let line = stderr.lines().map(str::trim).find(|l| mentions(l, path)).unwrap_or_else(|| lfs::first_line(stderr));
                Outcome::Failed(lfs::explain(line))
            };
            (path.to_string(), outcome)
        })
        .collect()
}

/// `git lfs unlock --json [--force] <paths>` prints `{path, unlocked, reason}` per path.
fn run_unlock(ctx: &RepoContext, paths: &[&str], force: bool) -> HashMap<String, Outcome> {
    let mut args = vec!["lfs", "unlock", "--json"];
    if force {
        args.push("--force");
    }
    args.extend(paths);
    match git::output_in(&ctx.root, &args) {
        Err(_) => all_failed(paths, "git が見つかりません"),
        Ok(out) => unlock_results(paths, &String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr)),
    }
}

#[derive(Deserialize)]
struct UnlockedPath {
    #[serde(default)]
    path: String,
    unlocked: bool,
    #[serde(default)]
    reason: String,
}

fn unlock_results(paths: &[&str], stdout: &str, stderr: &str) -> HashMap<String, Outcome> {
    let rows: Vec<UnlockedPath> = serde_json::from_str(stdout.trim()).unwrap_or_default();
    paths
        .iter()
        .map(|&path| {
            let outcome = match rows.iter().find(|r| r.path == path) {
                Some(r) if r.unlocked => Outcome::Unlocked,
                Some(r) => Outcome::Failed(lfs::explain(&r.reason)),
                None => Outcome::Failed(lfs::explain(lfs::first_line(stderr))),
            };
            (path.to_string(), outcome)
        })
        .collect()
}

/// Whether `line` names `path` as a whole word (so "b.psd" does not match "b.psd.meta").
fn mentions(line: &str, path: &str) -> bool {
    line.match_indices(path).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let after = line[i + path.len()..].chars().next();
        matches!(before, None | Some(' ')) && matches!(after, None | Some(' ' | ':'))
    })
}

fn all_failed(paths: &[&str], message: &str) -> HashMap<String, Outcome> {
    paths.iter().map(|p| (p.to_string(), Outcome::Failed(message.into()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locks::parse_verify_json;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn locks() -> LockList {
        parse_verify_json(
            r#"{"ours":[{"id":"1","path":"mine.psd","owner":{"name":"alice"},"locked_at":"2026-10-17T00:00:00Z"}],
                "theirs":[{"id":"2","path":"Assets/Theirs.psd","owner":{"name":"bob"},"locked_at":"2026-10-17T00:00:00Z"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn meta_partners() {
        assert_eq!(meta_partner("Assets/Main.unity").as_deref(), Some("Assets/Main.unity.meta"));
        assert_eq!(meta_partner("Assets/Main.unity.meta").as_deref(), Some("Assets/Main.unity"));
        assert_eq!(meta_partner(".meta"), None);
        assert_eq!(meta_partner("Assets/.meta"), None);
    }

    #[test]
    fn collect_adds_existing_pairs_after_each_path() {
        let exists = |p: &str| p == "a.unity.meta" || p == "b.psd";
        let got = collect(paths(&["a.unity", "b.psd.meta", "c.txt"]), true, exists);
        assert_eq!(
            got,
            vec![
                ("a.unity".into(), None),
                ("a.unity.meta".into(), Some("a.unity".into())),
                ("b.psd.meta".into(), None),
                ("b.psd".into(), Some("b.psd.meta".into())),
                ("c.txt".into(), None),
            ]
        );
    }

    #[test]
    fn collect_deduplicates_and_respects_explicit_names() {
        let all = |_: &str| true;
        let got = collect(paths(&["a.unity", "a.unity.meta", "a.unity"]), true, all);
        assert_eq!(got, vec![("a.unity".into(), None), ("a.unity.meta".into(), None)]);
        assert_eq!(collect(paths(&["a.unity"]), false, all), vec![("a.unity".into(), None)]);
    }

    #[test]
    fn find_falls_back_to_case_insensitive_match() {
        assert!(matches!(find(&locks(), "mine.psd"), Some(State::Ours(_))));
        assert!(matches!(find(&locks(), "assets/theirs.PSD"), Some(State::Theirs(l)) if l.owner == "bob"));
        assert_eq!(find(&locks(), "free.psd"), None);
        // The target takes the lock's spelling, so unlock uses the path the server knows
        let targets = with_state(vec![("assets/theirs.psd".into(), None), ("free.psd".into(), None)], &locks());
        assert_eq!(targets[0].path, "Assets/Theirs.psd");
        assert_eq!(targets[1].state, State::Free);
        // A partner follows its asset's new spelling
        let targets = with_state(vec![("assets/theirs.psd".into(), None), ("assets/theirs.psd.meta".into(), Some("assets/theirs.psd".into()))], &locks());
        assert_eq!(targets[1].pair_of.as_deref(), Some("Assets/Theirs.psd"));
    }

    #[test]
    fn lock_results_from_json_and_stderr() {
        // Shapes from git-lfs 3.7 commands/command_lock.go
        let stdout = r#"[{"id":"9","path":"a.psd","owner":{"name":"alice"},"locked_at":"2026-10-18T00:00:00Z"}]"#;
        let stderr = "Locking b.psd.meta failed: other\nLocking b.psd failed: Lock exists\n";
        let got = lock_results(&["a.psd", "b.psd"], stdout, stderr);
        assert_eq!(got["a.psd"], Outcome::Locked(Some("alice".into())));
        assert!(matches!(&got["b.psd"], Outcome::Failed(m) if m.contains("すでにロック") && m.contains("b.psd")));
    }

    #[test]
    fn unlock_results_from_json() {
        // Shape from git-lfs 3.7 commands/command_unlock.go
        let stdout = r#"[{"path":"a.psd","unlocked":true},{"path":"b.psd","unlocked":false,"reason":"Cannot unlock file with uncommitted changes"}]"#;
        let got = unlock_results(&["a.psd", "b.psd", "c.psd"], stdout, "fatal: something\n");
        assert_eq!(got["a.psd"], Outcome::Unlocked);
        assert!(matches!(&got["b.psd"], Outcome::Failed(m) if m.contains("未コミットの変更")));
        assert!(matches!(&got["c.psd"], Outcome::Failed(m) if m.contains("something")));
    }

    fn plan(targets: Vec<Target>) -> Plan {
        let ctx = RepoContext { root: "/nonexistent".into(), prefix: String::new() };
        Plan { ctx, force: false, targets }
    }

    fn target(path: &str, pair_of: Option<&str>, state: State) -> Target {
        Target { path: path.into(), pair_of: pair_of.map(str::to_string), state }
    }

    #[test]
    fn pairs_wait_for_their_asset() {
        let bob = locks().theirs[0].clone();
        let p = plan(vec![
            target("a.unity", None, State::Theirs(bob.clone())),
            target("a.unity.meta", Some("a.unity"), State::Free),
            target("b.unity", None, State::Free),
            target("b.unity.meta", Some("b.unity"), State::Free),
        ]);
        let mut rounds = Vec::new();
        let results = p.run_in_two_rounds(
            |_| Step::Run,
            |o| matches!(o, Outcome::Locked(_)),
            |_, paths| {
                rounds.push(paths.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                paths.iter().map(|p| (p.to_string(), Outcome::Locked(None))).collect()
            },
        );
        // Named paths go in the first git lfs call, partners in the second
        assert_eq!(rounds, vec![vec!["a.unity", "b.unity"], vec!["a.unity.meta", "b.unity.meta"]]);
        assert_eq!(results, vec![Outcome::Locked(None); 4]);

        let p = plan(vec![
            target("a.unity", None, State::Theirs(bob)),
            target("a.unity.meta", Some("a.unity"), State::Free),
        ]);
        let results = p.run_in_two_rounds(
            |t| match &t.state {
                State::Theirs(l) => Step::Done(Outcome::HeldByOther(l.clone())),
                _ => Step::Run,
            },
            |o| matches!(o, Outcome::Locked(_)),
            |_, _| panic!("nothing should be locked"),
        );
        assert!(matches!(results[0], Outcome::HeldByOther(_)));
        assert_eq!(results[1], Outcome::SkippedPair);
    }

    #[test]
    fn force_failures_explain_permissions() {
        assert!(explain_force_failure("api error: Forbidden (403)").contains("強制解除する権限"));
        assert_eq!(explain_force_failure("other"), "other");
        // Real GitHub message, after lfs::explain (which has no hint for it)
        let got = explain_force_failure(&lfs::explain("You must have admin access to force delete a lock"));
        assert!(got.starts_with("他人のロックを強制解除する権限がありません"), "{got}");
        assert!(got.ends_with("（git lfs: You must have admin access to force delete a lock）"), "{got}");
        // A generic 403 hint from lfs::explain is replaced, not repeated
        let got = explain_force_failure(&lfs::explain("Forbidden (403)"));
        assert!(!got.contains("書き込み権限"), "{got}");
    }

    #[test]
    fn failures() {
        assert!(Outcome::NoSuchFile.is_failure());
        assert!(Outcome::IsDirectory.is_failure());
        assert!(Outcome::Failed("x".into()).is_failure());
        assert!(!Outcome::AlreadyOurs.is_failure());
        assert!(!Outcome::NotLocked.is_failure());
        assert!(!Outcome::SkippedPair.is_failure());
    }
}
