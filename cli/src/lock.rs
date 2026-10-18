//! `repo-tweaks lock` / `repo-tweaks unlock`.

use std::io::{BufRead, IsTerminal, Write};

use serde::Serialize;
use time::OffsetDateTime;
use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::{GitConfig, KEY_NOTIFY, RepoInfo};
use repo_tweaks_core::lfs;
use repo_tweaks_core::locks::{Lock, format_jst, format_rfc3339};
use repo_tweaks_core::ops::{self, Outcome, State, Target};

use crate::client::{Client, Event, EventFile};
use crate::config::{self, Config};
use crate::text::pad;

/// Shared by lock and unlock
pub struct Options {
    pub with_meta: bool,
    pub json: bool,
    /// --no-notify
    pub no_notify: bool,
}

pub fn lock(config: &GitConfig, paths: &[String], opts: &Options) -> Result<(), String> {
    let plan = ops::plan_lock(config, paths, opts.with_meta)?;
    finish(config, &plan.lock(), opts)
}

pub fn unlock(config: &GitConfig, paths: &[String], opts: &Options, force: bool, yes: bool) -> Result<(), String> {
    let plan = ops::plan_unlock(config, paths, opts.with_meta, force)?;
    let others = plan.others_locks();
    if force && !others.is_empty() && !yes {
        confirm_force(&others, OffsetDateTime::now_utc())?;
    }
    finish(config, &plan.unlock(), opts)
}

/// Print the results, tell the server (best effort), then turn failures into the exit status.
fn finish(config: &GitConfig, results: &[(Target, Outcome)], opts: &Options) -> Result<(), String> {
    print_results(results, opts.json)?;
    if notify_enabled(opts.no_notify, config.get(KEY_NOTIFY)) {
        send_events(config, notify_events(results), lfs_user(results));
    }
    exit_status(results)
}

/// Ask on the terminal before removing other people's locks. Without a terminal, require --yes.
fn confirm_force(others: &[&Lock], now: OffsetDateTime) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("他人のロックの強制解除には確認が必要です。確認なしで実行するには --yes を付けてください".into());
    }
    let path_width = others.iter().map(|l| l.path.width()).max().unwrap_or(0);
    let mut err = std::io::stderr();
    let _ = writeln!(err, "次の他人のロックを強制解除します:");
    for lock in others {
        let _ = writeln!(err, "  {}  {}  {} から", pad(&lock.path, path_width), lock.owner, format_jst(lock.locked_at, now));
    }
    let _ = write!(err, "よろしいですか？ [y/N]: ");
    let _ = err.flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).map_err(|e| e.to_string())?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err("中止しました（何も変更していません）".into()),
    }
}

fn print_results(results: &[(Target, Outcome)], json: bool) -> Result<(), String> {
    if json {
        let rows: Vec<JsonResult> = results.iter().map(|(t, o)| JsonResult::new(t, o)).collect();
        println!("{}", serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?);
    } else {
        print!("{}", render(results, OffsetDateTime::now_utc()));
    }
    Ok(())
}

fn exit_status(results: &[(Target, Outcome)]) -> Result<(), String> {
    match results.iter().filter(|(_, o)| o.is_failure()).count() {
        0 => Ok(()),
        n => Err(format!("{n} 件は処理できませんでした")),
    }
}

/// On unless --no-notify, or git config repo-tweaks.notify is false / no / off / 0.
fn notify_enabled(no_notify: bool, configured: Option<String>) -> bool {
    let off = configured.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "no" | "off" | "0"));
    !no_notify && !off
}

/// What to tell the server: one event per kind of change that actually happened.
fn notify_events(results: &[(Target, Outcome)]) -> Vec<(&'static str, Vec<EventFile>)> {
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
/// after a plain lock; send_events then asks the server.)
fn lfs_user(results: &[(Target, Outcome)]) -> Option<String> {
    results.iter().find_map(|(target, outcome)| match (outcome, &target.state) {
        (Outcome::Locked { owner: Some(name), .. }, _) => Some(name.clone()),
        (_, State::Ours(lock)) => Some(lock.owner.clone()),
        _ => None,
    })
}

const NO_SERVER_NOTICE: &str = "repo-tweaks: 通知の送り先（repo-tweaks.server）が未設定なので、Discord には送っていません（この表示を消すには git config --global repo-tweaks.notify false）";

/// Best effort: the locks have already changed, so a missing or unreachable server is only a warning.
fn send_events(config: &GitConfig, events: Vec<(&'static str, Vec<EventFile>)>, lfs_user: Option<String>) {
    if events.is_empty() {
        return;
    }
    // Say so once: a forgotten setting would otherwise look like "nobody is locking anything".
    // Users who do not want notifications turn this off with repo-tweaks.notify false.
    if !config::server_is_set(config) {
        eprintln!("{NO_SERVER_NOTICE}");
        return;
    }
    let server = match Config::load(config) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("repo-tweaks: 通知を送れませんでした（ロック・解除は済んでいます）: {message}");
            return;
        }
    };
    let client = Client::new(server);
    let mut info = RepoInfo::current(config);
    // Prefer the lock server's name, so notices match `list`; fall back to git's user.name.
    // One extra `git lfs locks` call, only when notifying and the name is not known yet.
    let lfs_user = lfs_user.or_else(|| {
        lfs::fetch_locks(config).ok().and_then(|locks| locks.ours.first().map(|l| l.owner.clone()))
    });
    if lfs_user.is_some() {
        info.user = lfs_user;
    }
    let total = events.len();
    for (i, (kind, files)) in events.into_iter().enumerate() {
        if let Err(message) = client.send_event(&Event::new(kind.into(), info.clone()).with_files(files)) {
            // Stop at the first failure: an unreachable server would cost the timeout again for each event
            let rest = if i + 1 < total { "残りの通知も送っていません。" } else { "" };
            eprintln!("repo-tweaks: 通知を送れませんでした（ロック・解除は済んでいます。{rest}）: {message}");
            return;
        }
    }
}

fn describe(outcome: &Outcome, now: OffsetDateTime) -> (&'static str, Option<String>) {
    let since = |l: &Lock| format_jst(l.locked_at, now);
    match outcome {
        Outcome::Locked { .. } => ("ロックしました", None),
        Outcome::AlreadyOurs => ("ロック済み", Some("自分がロック中".into())),
        Outcome::HeldByOther(l) => ("できません", Some(format!("{} さんが {} からロック中", l.owner, since(l)))),
        Outcome::NoSuchFile => ("できません", Some("ファイルがありません".into())),
        Outcome::IsDirectory => ("できません", Some("フォルダはロックできません。ファイルを指定してください".into())),
        Outcome::SkippedPair => ("しませんでした", Some("元のファイルを処理できなかったため".into())),
        Outcome::Unlocked => ("解除しました", None),
        Outcome::ForceUnlocked(l) => ("強制解除しました", Some(format!("{} さんのロック", l.owner))),
        Outcome::NotLocked => ("ロックなし", None),
        Outcome::OthersLock(l) => (
            "できません",
            Some(format!("{} さんが {} からロック中。強制解除するには --force", l.owner, since(l))),
        ),
        Outcome::Failed(message) => ("できません", Some(message.clone())),
    }
}

fn render(results: &[(Target, Outcome)], now: OffsetDateTime) -> String {
    let rows: Vec<(&str, String, Option<String>)> = results
        .iter()
        .map(|(target, outcome)| {
            let (label, detail) = describe(outcome, now);
            let path = if target.pair_of.is_some() { format!("{}（自動で追加）", target.path) } else { target.path.clone() };
            (label, path, detail)
        })
        .collect();
    let label_width = rows.iter().map(|(l, _, _)| l.width()).max().unwrap_or(0);
    let path_width = rows.iter().map(|(_, p, _)| p.width()).max().unwrap_or(0);
    let mut out = String::new();
    for (label, path, detail) in rows {
        let line = match detail {
            Some(detail) => format!("{}  {}  {detail}", pad(label, label_width), pad(&path, path_width)),
            None => format!("{}  {path}", pad(label, label_width)),
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Machine-readable result per file (for Unity / GUI). Times are UTC.
#[derive(Serialize)]
struct JsonResult {
    path: String,
    meta_pair: bool,
    /// locked / already_ours / held_by_other / no_such_file / is_directory / unlocked / force_unlocked /
    /// not_locked / others_lock / skipped_pair / failed
    result: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    locked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl JsonResult {
    fn new(target: &Target, outcome: &Outcome) -> Self {
        let (result, lock, message) = match outcome {
            Outcome::Locked { .. } => ("locked", None, None),
            Outcome::AlreadyOurs => ("already_ours", None, None),
            Outcome::HeldByOther(l) => ("held_by_other", Some(l), None),
            Outcome::NoSuchFile => ("no_such_file", None, None),
            Outcome::IsDirectory => ("is_directory", None, None),
            Outcome::SkippedPair => ("skipped_pair", None, None),
            Outcome::Unlocked => ("unlocked", None, None),
            Outcome::ForceUnlocked(l) => ("force_unlocked", Some(l), None),
            Outcome::NotLocked => ("not_locked", None, None),
            Outcome::OthersLock(l) => ("others_lock", Some(l), None),
            Outcome::Failed(m) => ("failed", None, Some(m.clone())),
        };
        JsonResult {
            path: target.path.clone(),
            meta_pair: target.pair_of.is_some(),
            result,
            ok: !outcome.is_failure(),
            owner: lock.map(|l| l.owner.clone()),
            locked_at: lock.map(|l| format_rfc3339(l.locked_at)),
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repo_tweaks_core::ops::State;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-10-18 06:00 UTC);

    fn target(path: &str, meta_pair: bool) -> Target {
        let pair_of = meta_pair.then(|| path.trim_end_matches(".meta").to_string());
        Target { path: path.into(), pair_of, state: State::Free }
    }

    fn bob() -> Lock {
        Lock { id: "2".into(), path: "Assets/Art/hero.psd".into(), owner: "bob".into(), locked_at: datetime!(2026-10-14 01:00 UTC) }
    }

    #[test]
    fn renders_aligned_results() {
        let results = vec![
            (target("Assets/Main.unity", false), Outcome::Locked { id: None, owner: None }),
            (target("Assets/Main.unity.meta", true), Outcome::Locked { id: None, owner: None }),
            (target("README.md", false), Outcome::AlreadyOurs),
            (target("Assets/Art/hero.psd", false), Outcome::HeldByOther(bob())),
            (target("Assets/Art/hero.psd.meta", true), Outcome::SkippedPair),
        ];
        let expected = "\
ロックしました  Assets/Main.unity
ロックしました  Assets/Main.unity.meta（自動で追加）
ロック済み      README.md                               自分がロック中
できません      Assets/Art/hero.psd                     bob さんが 10/14 10:00 からロック中
しませんでした  Assets/Art/hero.psd.meta（自動で追加）  元のファイルを処理できなかったため
";
        assert_eq!(render(&results, NOW), expected);
    }

    #[test]
    fn unlock_without_force_points_to_force() {
        let (label, detail) = describe(&Outcome::OthersLock(bob()), NOW);
        assert_eq!(label, "できません");
        assert!(detail.unwrap().contains("--force"));
    }

    #[test]
    fn json_results() {
        let row = serde_json::to_value(JsonResult::new(&target("a.psd", false), &Outcome::HeldByOther(bob()))).unwrap();
        assert_eq!(row["result"], "held_by_other");
        assert_eq!(row["ok"], false);
        assert_eq!(row["owner"], "bob");
        assert_eq!(row["locked_at"], "2026-10-14T01:00:00Z");
        let row = serde_json::to_value(JsonResult::new(&target("a.psd.meta", true), &Outcome::Locked { id: None, owner: None })).unwrap();
        assert_eq!(row["ok"], true);
        assert_eq!(row["meta_pair"], true);
        assert!(row.get("owner").is_none());
    }

    #[test]
    fn failures_set_exit_code() {
        assert!(exit_status(&[(target("a", false), Outcome::NotLocked)]).is_ok());
        assert_eq!(
            exit_status(&[(target("a", false), Outcome::NoSuchFile), (target("b", false), Outcome::Locked { id: None, owner: None })]),
            Err("1 件は処理できませんでした".into())
        );
    }

    #[test]
    fn notifications_can_be_turned_off() {
        assert!(notify_enabled(false, None));
        assert!(notify_enabled(false, Some("true".into())));
        assert!(!notify_enabled(true, None));
        for off in ["false", "No", "OFF", "0"] {
            assert!(!notify_enabled(false, Some(off.into())), "{off}");
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
        let events = notify_events(&results);
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
        assert!(notify_events(&[(target("b.psd", false), Outcome::HeldByOther(bob()))]).is_empty());
    }

    #[test]
    fn lfs_user_comes_from_the_lock_server() {
        let mine = Lock { owner: "alice-gh".into(), ..bob() };
        let ours = Target { path: "c.psd".into(), pair_of: None, state: State::Ours(mine) };
        assert_eq!(lfs_user(&[(target("a.psd", false), Outcome::Locked { id: Some("9".into()), owner: Some("alice-gh".into()) })]).as_deref(), Some("alice-gh"));
        assert_eq!(lfs_user(&[(ours, Outcome::Unlocked)]).as_deref(), Some("alice-gh"));
        // Only other people's locks: unknown, the caller falls back to user.name
        assert_eq!(lfs_user(&[(target("b.psd", false), Outcome::ForceUnlocked(bob()))]), None);
    }
}
