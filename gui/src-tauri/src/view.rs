//! Screen data for the lock list, formatted the same way as `repo-tweaks list`.

use repo_tweaks_core::locks::{Lock, LockList, format_age, format_jst, stale_warning};
use repo_tweaks_core::ops::{Outcome, State, Target};
use serde::Serialize;
use time::OffsetDateTime;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LockReport {
    root: String,
    name: String,
    stale_days: u32,
    stale_count: usize,
    /// The CLI's warning about stale locks, or None when there are none
    stale_warning: Option<String>,
    /// Japan time of this fetch, e.g. "10/19 18:50"
    fetched_at: String,
    ours: Vec<LockRow>,
    theirs: Vec<LockRow>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LockRow {
    id: String,
    path: String,
    owner: String,
    locked_at: String,
    age: String,
    stale: bool,
}

impl LockReport {
    /// `name` is the repository's display name (`git::repo_name`: the remote's, not the folder's).
    pub fn new(root: &str, name: String, locks: &LockList, now: OffsetDateTime, stale_days: u32) -> Self {
        let rows = |list: &[Lock]| {
            list.iter()
                .map(|l| LockRow {
                    id: l.id.clone(),
                    path: l.path.clone(),
                    owner: l.owner.clone(),
                    locked_at: format_jst(l.locked_at, now),
                    age: format_age(l.age(now)),
                    stale: l.is_stale(now, stale_days),
                })
                .collect()
        };
        let stale_count = locks.stale_count(now, stale_days);
        LockReport {
            root: root.into(),
            name,
            stale_days,
            stale_count,
            stale_warning: (stale_count > 0).then(|| stale_warning(stale_days, stale_count)),
            fetched_at: format_jst(now, now),
            ours: rows(&locks.ours),
            theirs: rows(&locks.theirs),
        }
    }
}

/// The result of locking or unlocking one file, worded like the CLI's output.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    path: String,
    /// The Unity `.meta` partner added automatically
    auto_added: bool,
    label: &'static str,
    detail: Option<String>,
    ok: bool,
    /// Our own lock that git lfs refused to remove (uncommitted changes, deleted file):
    /// a forced unlock (by lock ID) would go through
    can_force: bool,
}

pub fn op_results(results: &[(Target, Outcome)], now: OffsetDateTime) -> Vec<OpResult> {
    results
        .iter()
        .map(|(target, outcome)| {
            let (label, detail) = outcome.describe(now);
            OpResult {
                path: target.path.clone(),
                auto_added: target.pair_of.is_some(),
                label,
                detail,
                ok: !outcome.is_failure(),
                can_force: matches!((&target.state, outcome), (State::Ours(_), Outcome::Failed(_))),
            }
        })
        .collect()
}

/// Someone else's lock that a forced unlock would remove, for the confirmation dialog.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OthersLock {
    id: String,
    path: String,
    owner: String,
    locked_at: String,
}

impl OthersLock {
    pub fn new(lock: &Lock, now: OffsetDateTime) -> Self {
        OthersLock { id: lock.id.clone(), path: lock.path.clone(), owner: lock.owner.clone(), locked_at: format_jst(lock.locked_at, now) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repo_tweaks_core::locks::parse_verify_json;
    use time::macros::datetime;

    #[test]
    fn report_matches_cli_wording() {
        let locks = parse_verify_json(
            r#"{"ours":[{"id":"1","path":"afl/README.md","owner":{"name":"alice"},"locked_at":"2026-10-17T16:30:00Z"}],
                "theirs":[{"id":"2","path":"Assets/Scenes/Main.unity","owner":{"name":"bob"},"locked_at":"2026-10-14T01:00:00Z"}]}"#,
        )
        .unwrap();
        let report = LockReport::new("/work/game-work2", "game".into(), &locks, datetime!(2026-10-18 18:30 UTC), 3);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["root"], "/work/game-work2");
        assert_eq!(json["name"], "game");
        assert_eq!(json["staleDays"], 3);
        assert_eq!(json["staleCount"], 1);
        assert_eq!(json["staleWarning"], "3日以上ロックされたままのものが 1 件あります");
        assert_eq!(json["fetchedAt"], "10/19 03:30");
        assert_eq!(json["ours"][0]["lockedAt"], "10/18 01:30");
        assert_eq!(json["ours"][0]["age"], "1日2時間");
        assert_eq!(json["ours"][0]["stale"], false);
        assert_eq!(json["theirs"][0]["owner"], "bob");
        assert_eq!(json["theirs"][0]["age"], "4日17時間");
        assert_eq!(json["theirs"][0]["stale"], true);
    }

    #[test]
    fn own_lock_refused_by_git_lfs_can_be_forced() {
        let locks = parse_verify_json(
            r#"{"ours":[{"id":"1","path":"a.unity","owner":{"name":"alice"},"locked_at":"2026-10-17T16:30:00Z"}],"theirs":[]}"#,
        )
        .unwrap();
        let ours = State::Ours(locks.ours[0].clone());
        let target = |path: &str, pair_of: Option<&str>, state: State| Target {
            path: path.into(),
            pair_of: pair_of.map(str::to_string),
            state,
        };
        let results = [
            (target("a.unity", None, ours.clone()), Outcome::Failed("未コミットの変更があるため解除できません。".into())),
            (target("a.unity.meta", Some("a.unity"), State::Free), Outcome::SkippedPair),
            (target("b.psd", None, State::Free), Outcome::Failed("x".into())),
        ];
        let json = serde_json::to_value(op_results(&results, datetime!(2026-10-18 18:30 UTC))).unwrap();
        assert_eq!(json[0]["label"], "できません");
        assert_eq!(json[0]["ok"], false);
        assert_eq!(json[0]["canForce"], true);
        assert_eq!(json[1]["autoAdded"], true);
        assert_eq!(json[1]["label"], "しませんでした");
        assert_eq!(json[1]["canForce"], false);
        // Not our lock: nothing to force
        assert_eq!(json[2]["canForce"], false);
    }
}
