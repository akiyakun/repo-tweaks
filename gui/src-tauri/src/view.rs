//! Screen data for the lock list, formatted the same way as `repo-tweaks list`.

use std::path::Path;

use repo_tweaks_core::locks::{Lock, LockList, format_age, format_jst};
use serde::Serialize;
use time::OffsetDateTime;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LockReport {
    root: String,
    name: String,
    stale_days: u32,
    stale_count: usize,
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
    pub fn new(root: &str, locks: &LockList, now: OffsetDateTime, stale_days: u32) -> Self {
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
        let name = Path::new(root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.into());
        LockReport {
            root: root.into(),
            name,
            stale_days,
            stale_count: locks.stale_count(now, stale_days),
            ours: rows(&locks.ours),
            theirs: rows(&locks.theirs),
        }
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
        let report = LockReport::new("/work/game", &locks, datetime!(2026-10-18 18:30 UTC), 3);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["name"], "game");
        assert_eq!(json["staleDays"], 3);
        assert_eq!(json["staleCount"], 1);
        assert_eq!(json["ours"][0]["lockedAt"], "10/18 01:30");
        assert_eq!(json["ours"][0]["age"], "1日2時間");
        assert_eq!(json["ours"][0]["stale"], false);
        assert_eq!(json["theirs"][0]["owner"], "bob");
        assert_eq!(json["theirs"][0]["age"], "4日17時間");
        assert_eq!(json["theirs"][0]["stale"], true);
    }
}
