//! Git LFS locks: parsing `git lfs locks --verify --json` and judging how old each lock is.

use serde::Deserialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::macros::{format_description, offset};
use time::{Duration, OffsetDateTime, UtcOffset};

/// A lock older than this many days counts as stale (forgotten) unless configured otherwise.
pub const DEFAULT_STALE_DAYS: u32 = 3;
pub const MAX_STALE_DAYS: u32 = 365;
/// Times are shown in Japan time (no daylight saving, so a fixed offset is exact).
const JST: UtcOffset = offset!(+9);

/// The flag wins over git config; both must be 1..=MAX_STALE_DAYS.
pub fn resolve_stale_days(flag: Option<u32>, configured: Option<String>) -> Result<u32, String> {
    let (value, source) = match (flag, configured) {
        (Some(days), _) => (Some(days), "--stale-days"),
        (None, Some(text)) => (text.trim().parse().ok(), "git config repo-tweaks.staleDays"),
        (None, None) => return Ok(DEFAULT_STALE_DAYS),
    };
    value
        .filter(|d| (1..=MAX_STALE_DAYS).contains(d))
        .ok_or_else(|| format!("{source} は 1〜{MAX_STALE_DAYS} の整数にしてください"))
}

/// "3日以上ロックされたままのものが 2 件あります" (shared by the CLI and the GUI)
pub fn stale_warning(stale_days: u32, count: usize) -> String {
    format!("{stale_days}日以上ロックされたままのものが {count} 件あります")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lock {
    pub id: String,
    /// Relative to the repository root
    pub path: String,
    pub owner: String,
    pub locked_at: OffsetDateTime,
}

impl Lock {
    /// Time since the lock was taken. Never negative, even if the clocks disagree.
    pub fn age(&self, now: OffsetDateTime) -> Duration {
        (now - self.locked_at).max(Duration::ZERO)
    }

    pub fn is_stale(&self, now: OffsetDateTime, stale_days: u32) -> bool {
        self.age(now) >= Duration::days(stale_days.into())
    }
}

/// Locks split into the current user's (`ours`) and everyone else's (`theirs`), oldest first.
#[derive(Debug, Default, PartialEq)]
pub struct LockList {
    pub ours: Vec<Lock>,
    pub theirs: Vec<Lock>,
}

impl LockList {
    pub fn is_empty(&self) -> bool {
        self.ours.is_empty() && self.theirs.is_empty()
    }

    pub fn stale_count(&self, now: OffsetDateTime, stale_days: u32) -> usize {
        self.ours.iter().chain(&self.theirs).filter(|l| l.is_stale(now, stale_days)).count()
    }
}

#[derive(Deserialize)]
struct RawList {
    #[serde(default)]
    ours: Vec<RawLock>,
    #[serde(default)]
    theirs: Vec<RawLock>,
}

#[derive(Deserialize)]
struct RawLock {
    // The spec says string; accept a number too in case a server sends one.
    id: Value,
    path: String,
    #[serde(default)]
    owner: Option<RawOwner>,
    locked_at: String,
}

#[derive(Deserialize)]
struct RawOwner {
    #[serde(default)]
    name: String,
}

/// Parse the JSON printed by `git lfs locks --verify --json`.
pub fn parse_verify_json(json: &str) -> Result<LockList, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("ロック一覧の JSON を読めませんでした（{e}）"))?;
    // serde would also accept `[]` (the output without --verify) as an empty list; refuse it.
    if !value.is_object() {
        return Err("ロック一覧の形式が違います（ours / theirs を含むオブジェクトではありません）".into());
    }
    let raw: RawList =
        serde_json::from_value(value).map_err(|e| format!("ロック一覧の JSON を読めませんでした（{e}）"))?;
    let convert = |locks: Vec<RawLock>| -> Result<Vec<Lock>, String> {
        let mut out = locks.into_iter().map(convert_lock).collect::<Result<Vec<_>, _>>()?;
        out.sort_by(|a, b| a.locked_at.cmp(&b.locked_at).then_with(|| a.path.cmp(&b.path)));
        Ok(out)
    };
    Ok(LockList { ours: convert(raw.ours)?, theirs: convert(raw.theirs)? })
}

fn convert_lock(raw: RawLock) -> Result<Lock, String> {
    let locked_at = OffsetDateTime::parse(&raw.locked_at, &Rfc3339)
        .map_err(|_| format!("ロックの日時を読めませんでした: {}（{}）", raw.locked_at, raw.path))?;
    let id = match raw.id {
        Value::String(s) => s,
        other => other.to_string(),
    };
    let owner = raw.owner.map(|o| o.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "（不明）".into());
    Ok(Lock { id, path: raw.path, owner, locked_at })
}

/// "4日17時間", "9時間", "45分", "1分未満"
pub fn format_age(age: Duration) -> String {
    let minutes = age.whole_minutes().max(0);
    if minutes == 0 {
        return "1分未満".into();
    }
    if minutes < 60 {
        return format!("{minutes}分");
    }
    let hours = minutes / 60;
    let (days, hours) = (hours / 24, hours % 24);
    match (days, hours) {
        (0, h) => format!("{h}時間"),
        (d, 0) => format!("{d}日"),
        (d, h) => format!("{d}日{h}時間"),
    }
}

/// Japan time as "10/18 01:30", with the year only when it differs from `now`'s year.
pub fn format_jst(at: OffsetDateTime, now: OffsetDateTime) -> String {
    let at = at.to_offset(JST);
    let result = if at.year() == now.to_offset(JST).year() {
        at.format(format_description!("[month]/[day] [hour]:[minute]"))
    } else {
        at.format(format_description!("[year]/[month]/[day] [hour]:[minute]"))
    };
    result.unwrap_or_default()
}

/// UTC in RFC 3339 ("2026-10-17T16:30:58Z"), for machine-readable output.
pub fn format_rfc3339(at: OffsetDateTime) -> String {
    at.to_offset(UtcOffset::UTC).format(&Rfc3339).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    // Real output captured on GitHub (see HANDOFF), with the owner replaced by a placeholder.
    const SAMPLE: &str = r#"{"ours":[{"id":"57707244","path":"afl/README.md","owner":{"name":"alice"},"locked_at":"2026-10-17T16:30:58Z"}],"theirs":[]}"#;

    #[test]
    fn parses_real_sample() {
        let list = parse_verify_json(SAMPLE).unwrap();
        assert_eq!(list.theirs, vec![]);
        assert_eq!(
            list.ours,
            vec![Lock {
                id: "57707244".into(),
                path: "afl/README.md".into(),
                owner: "alice".into(),
                locked_at: datetime!(2026-10-17 16:30:58 UTC),
            }]
        );
    }

    #[test]
    fn sorts_oldest_first_and_tolerates_variants() {
        let json = r#"{"ours":[],"theirs":[
            {"id":"2","path":"b.psd","owner":{"name":"bob"},"locked_at":"2026-10-17T09:20:00Z"},
            {"id":1,"path":"a.unity","locked_at":"2026-10-14T01:00:00.123456+00:00"}
        ]}"#;
        let list = parse_verify_json(json).unwrap();
        let paths: Vec<_> = list.theirs.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(paths, ["a.unity", "b.psd"]);
        assert_eq!(list.theirs[0].id, "1");
        assert_eq!(list.theirs[0].owner, "（不明）");
    }

    #[test]
    fn rejects_broken_input() {
        assert!(parse_verify_json("[]").is_err());
        assert!(parse_verify_json("not json").is_err());
        let bad_date = r#"{"ours":[{"id":"1","path":"x","locked_at":"yesterday"}]}"#;
        assert!(parse_verify_json(bad_date).unwrap_err().contains("yesterday"));
    }

    #[test]
    fn staleness_uses_threshold_boundary() {
        let lock = parse_verify_json(SAMPLE).unwrap().ours.remove(0);
        let at = datetime!(2026-10-17 16:30:58 UTC);
        assert!(!lock.is_stale(at + Duration::days(3) - Duration::seconds(1), 3));
        assert!(lock.is_stale(at + Duration::days(3), 3));
        // Lock time in the future (clock skew) counts as zero age
        assert_eq!(lock.age(at - Duration::hours(1)), Duration::ZERO);
    }

    #[test]
    fn stale_count_covers_both_lists() {
        let json = r#"{"ours":[{"id":"1","path":"a","locked_at":"2026-10-01T00:00:00Z"}],
                       "theirs":[{"id":"2","path":"b","locked_at":"2026-10-02T00:00:00Z"},
                                 {"id":"3","path":"c","locked_at":"2026-10-17T00:00:00Z"}]}"#;
        let list = parse_verify_json(json).unwrap();
        assert_eq!(list.stale_count(datetime!(2026-10-18 00:00 UTC), 3), 2);
    }

    #[test]
    fn stale_days_sources() {
        assert_eq!(resolve_stale_days(None, None), Ok(3));
        assert_eq!(resolve_stale_days(None, Some("7".into())), Ok(7));
        assert_eq!(resolve_stale_days(Some(1), Some("7".into())), Ok(1));
        assert!(resolve_stale_days(Some(0), None).unwrap_err().contains("--stale-days"));
        assert!(resolve_stale_days(None, Some("三日".into())).unwrap_err().contains("staleDays"));
        assert!(resolve_stale_days(None, Some("366".into())).is_err());
    }

    #[test]
    fn formats_age() {
        assert_eq!(format_age(Duration::seconds(7)), "1分未満");
        assert_eq!(format_age(Duration::ZERO), "1分未満");
        assert_eq!(format_age(Duration::minutes(1)), "1分");
        assert_eq!(format_age(Duration::minutes(45)), "45分");
        assert_eq!(format_age(Duration::hours(9) + Duration::minutes(59)), "9時間");
        assert_eq!(format_age(Duration::days(1) + Duration::hours(2)), "1日2時間");
        assert_eq!(format_age(Duration::days(4)), "4日");
    }

    #[test]
    fn formats_japan_time() {
        let now = datetime!(2026-10-18 03:00 UTC);
        // 16:30 UTC is 01:30 the next day in Japan
        assert_eq!(format_jst(datetime!(2026-10-17 16:30:58 UTC), now), "10/18 01:30");
        // Already 2026 in Japan, so no year
        assert_eq!(format_jst(datetime!(2025-12-31 16:00 UTC), now), "01/01 01:00");
        assert_eq!(format_jst(datetime!(2025-12-31 14:00 UTC), now), "2025/12/31 23:00");
        assert_eq!(format_rfc3339(datetime!(2026-10-17 16:30:58 UTC)), "2026-10-17T16:30:58Z");
    }
}
