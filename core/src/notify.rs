//! Telling the repo-tweaks server (and through it Discord) what a lock / unlock changed.
//!
//! Best effort: the locks have already changed, so a missing or unreachable server is reported
//! back to the caller (CLI or GUI) instead of failing the operation.

use crate::git::{GitConfig, KEY_NOTIFY, RepoInfo};
use crate::lfs;
use crate::locks::format_rfc3339;
use crate::ops::{Outcome, State, Target};
use crate::paths::RepoContext;
use crate::server::{self, Client, Config, Event, EventFile};

/// Shown when no server is set. Users who do not want notifications turn this off with
/// repo-tweaks.notify false (then `enabled` is false and nothing is said).
pub const NO_SERVER_NOTICE: &str = "通知の送り先（repo-tweaks.server）が未設定なので、Discord には送っていません（この表示を消すには git config --global repo-tweaks.notify false）";

/// What happened to the notification.
#[derive(Debug, PartialEq)]
pub enum Notified {
    /// Nothing changed, so nothing was sent
    Nothing,
    /// No server is configured (repo-tweaks.server), so nothing was sent
    NoServer,
    /// This many events reached the server
    Sent(usize),
    /// Why the notification could not be sent (the locks have changed anyway)
    Failed(String),
}

/// Off when git config repo-tweaks.notify is false / no / off / 0.
pub fn enabled(config: &GitConfig) -> bool {
    !is_off(config.get(KEY_NOTIFY))
}

fn is_off(configured: Option<String>) -> bool {
    configured.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "no" | "off" | "0"))
}

/// What to tell the server: one event per kind of change that actually happened.
pub fn lock_events(results: &[(Target, Outcome)]) -> Vec<(&'static str, Vec<EventFile>)> {
    let own = |t: &Target, id: Option<String>| EventFile { path: t.path.clone(), id, owner: None, locked_at: None };
    let our_id = |t: &Target| match &t.state {
        State::Ours(lock) => Some(lock.id.clone()),
        _ => None,
    };
    let mut locked = Vec::new();
    let mut unlocked = Vec::new();
    let mut forced = Vec::new();
    for (target, outcome) in results {
        match outcome {
            Outcome::Locked { id, .. } => locked.push(own(target, id.clone())),
            Outcome::Unlocked => unlocked.push(own(target, our_id(target))),
            Outcome::ForceUnlocked(lock) => forced.push(EventFile {
                path: target.path.clone(),
                id: Some(lock.id.clone()),
                owner: Some(lock.owner.clone()),
                locked_at: Some(format_rfc3339(lock.locked_at)),
            }),
            _ => {}
        }
    }
    [("lock", locked), ("unlock", unlocked), ("force_unlock", forced)]
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect()
}

/// The current user's name on the lock server (GitHub / GitLab), the same name `list` shows as owner.
/// Taken from a lock we just took or one of ours among the targets; None if neither is there.
/// (GitHub leaves the owner out of the lock it returns from `git lfs lock`, so this is often None
/// after a plain lock; `send_lock_events` then asks the server.)
fn lfs_user(results: &[(Target, Outcome)]) -> Option<String> {
    results.iter().find_map(|(target, outcome)| match (outcome, &target.state) {
        (Outcome::Locked { owner: Some(name), .. }, _) => Some(name.clone()),
        (_, State::Ours(lock)) => Some(lock.owner.clone()),
        _ => None,
    })
}

/// Send the events for `results` of the repository at `dir` ("." for the current directory).
/// The caller checks `enabled` first.
pub fn send_lock_events(dir: &str, config: &GitConfig, results: &[(Target, Outcome)]) -> Notified {
    let events = lock_events(results);
    if events.is_empty() {
        return Notified::Nothing;
    }
    // Say so: a forgotten setting would otherwise look like "nobody is locking anything".
    if !server::server_is_set(config) {
        return Notified::NoServer;
    }
    let server = match Config::load(config) {
        Ok(server) => server,
        Err(message) => return Notified::Failed(format!("通知を送れませんでした（ロック・解除は済んでいます）: {message}")),
    };
    let client = Client::new(server);
    let mut info = RepoInfo::in_dir(dir, config);
    // Prefer the lock server's name, so notices match `list`; fall back to git's user.name.
    // One extra `git lfs locks` call, only when notifying and the name is not known yet.
    let lfs_user = lfs_user(results).or_else(|| {
        let locks = RepoContext::open(dir).and_then(|ctx| lfs::fetch_locks_in(&ctx, config)).ok()?;
        locks.ours.first().map(|l| l.owner.clone())
    });
    if lfs_user.is_some() {
        info.user = lfs_user;
    }
    let total = events.len();
    for (i, (kind, files)) in events.into_iter().enumerate() {
        if let Err(message) = client.send_event(&Event::new(kind.into(), info.clone()).with_files(files)) {
            // Stop at the first failure: an unreachable server would cost the timeout again for each event
            let rest = if i + 1 < total { "。残りの通知も送っていません" } else { "" };
            return Notified::Failed(format!("通知を送れませんでした（ロック・解除は済んでいます{rest}）: {message}"));
        }
    }
    Notified::Sent(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locks::Lock;
    use time::macros::datetime;

    fn target(path: &str, meta_pair: bool) -> Target {
        let pair_of = meta_pair.then(|| path.trim_end_matches(".meta").to_string());
        Target { path: path.into(), pair_of, state: State::Free }
    }

    fn bob() -> Lock {
        Lock { id: "2".into(), path: "Assets/Art/hero.psd".into(), owner: "bob".into(), locked_at: datetime!(2026-10-14 01:00 UTC) }
    }

    #[test]
    fn notifications_can_be_turned_off() {
        assert!(!is_off(None));
        assert!(!is_off(Some("true".into())));
        for off in ["false", "No", "OFF", "0"] {
            assert!(is_off(Some(off.into())), "{off}");
        }
    }

    #[test]
    fn events_group_what_changed() {
        let results = vec![
            (target("a.unity", false), Outcome::Locked { id: Some("11".into()), owner: None }),
            (target("a.unity.meta", true), Outcome::Locked { id: None, owner: None }),
            (target("b.psd", false), Outcome::HeldByOther(bob())),
            (target("c.psd", false), Outcome::Unlocked),
            (target("Assets/Art/hero.psd", false), Outcome::ForceUnlocked(bob())),
            (target("d.psd", false), Outcome::Failed("x".into())),
        ];
        let events = lock_events(&results);
        let kinds: Vec<_> = events.iter().map(|(k, f)| (*k, f.iter().map(|f| f.path.as_str()).collect::<Vec<_>>())).collect();
        assert_eq!(
            kinds,
            vec![
                ("lock", vec!["a.unity", "a.unity.meta"]),
                ("unlock", vec!["c.psd"]),
                ("force_unlock", vec!["Assets/Art/hero.psd"]),
            ]
        );
        let forced = &events[2].1[0];
        assert_eq!(forced.id.as_deref(), Some("2"));
        assert_eq!(events[0].1[0].id.as_deref(), Some("11"));
        assert_eq!(forced.owner.as_deref(), Some("bob"));
        assert_eq!(forced.locked_at.as_deref(), Some("2026-10-14T01:00:00Z"));
        // Nothing changed: nothing to send
        assert!(lock_events(&[(target("b.psd", false), Outcome::HeldByOther(bob()))]).is_empty());
    }

    #[test]
    fn nothing_changed_sends_nothing() {
        let results = [(target("b.psd", false), Outcome::HeldByOther(bob()))];
        assert_eq!(send_lock_events(".", &GitConfig::default(), &results), Notified::Nothing);
    }

    #[test]
    fn lfs_user_comes_from_the_lock_server() {
        let mine = Lock { owner: "alice-gh".into(), ..bob() };
        let ours = Target { path: "c.psd".into(), pair_of: None, state: State::Ours(mine) };
        assert_eq!(
            lfs_user(&[(target("a.psd", false), Outcome::Locked { id: Some("9".into()), owner: Some("alice-gh".into()) })])
                .as_deref(),
            Some("alice-gh")
        );
        assert_eq!(lfs_user(&[(ours, Outcome::Unlocked)]).as_deref(), Some("alice-gh"));
        // Only other people's locks: unknown, the caller falls back to user.name
        assert_eq!(lfs_user(&[(target("b.psd", false), Outcome::ForceUnlocked(bob()))]), None);
    }
}
