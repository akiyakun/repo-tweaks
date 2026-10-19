//! `repo-tweaks list`: show who holds which lock, and flag stale ones.

use serde::Serialize;
use time::OffsetDateTime;
use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::{GitConfig, KEY_STALE_DAYS};
use repo_tweaks_core::lfs;

use crate::text::pad;
use repo_tweaks_core::locks::{
    Lock, LockList, format_age, format_jst, format_rfc3339, resolve_stale_days, stale_warning,
};

pub fn run(config: &GitConfig, json: bool, stale_days: Option<u32>) -> Result<(), String> {
    let stale_days = resolve_stale_days(stale_days, config.get(KEY_STALE_DAYS))?;
    let locks = lfs::fetch_locks(config)?;
    let now = OffsetDateTime::now_utc();
    if json {
        let out = JsonList::new(&locks, now, stale_days);
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    } else {
        print!("{}", render(&locks, now, stale_days));
    }
    Ok(())
}

fn render(locks: &LockList, now: OffsetDateTime, stale_days: u32) -> String {
    if locks.is_empty() {
        return "ロックはありません\n".into();
    }
    // Align the path column across both sections
    let path_width = locks.ours.iter().chain(&locks.theirs).map(|l| l.path.width()).max().unwrap_or(0);
    let mut out = String::new();
    section(&mut out, "自分のロック", &locks.ours, false, path_width, now, stale_days);
    out.push('\n');
    section(&mut out, "他人のロック", &locks.theirs, true, path_width, now, stale_days);
    let stale = locks.stale_count(now, stale_days);
    if stale > 0 {
        out.push_str(&format!("\n⚠ {}\n", stale_warning(stale_days, stale)));
    }
    out
}

fn section(
    out: &mut String,
    title: &str,
    locks: &[Lock],
    show_owner: bool,
    path_width: usize,
    now: OffsetDateTime,
    stale_days: u32,
) {
    out.push_str(&format!("{title}（{}）\n", locks.len()));
    if locks.is_empty() {
        out.push_str("  なし\n");
        return;
    }
    let owner_width = locks.iter().map(|l| l.owner.width()).max().unwrap_or(0);
    let ages: Vec<String> = locks.iter().map(|l| format_age(l.age(now))).collect();
    let age_width = ages.iter().map(|a| a.width()).max().unwrap_or(0);
    for (lock, age) in locks.iter().zip(&ages) {
        let mut line = format!("  {}  ", pad(&lock.path, path_width));
        if show_owner {
            line.push_str(&format!("{}  ", pad(&lock.owner, owner_width)));
        }
        line.push_str(&format!("{} から  ", format_jst(lock.locked_at, now)));
        if lock.is_stale(now, stale_days) {
            line.push_str(&format!("{}  ⚠ 古い", pad(age, age_width)));
        } else {
            line.push_str(age);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

/// Machine-readable output for other tools (Unity, GUI). Times are UTC.
#[derive(Serialize)]
struct JsonList {
    stale_days: u32,
    ours: Vec<JsonLock>,
    theirs: Vec<JsonLock>,
}

#[derive(Serialize)]
struct JsonLock {
    id: String,
    path: String,
    owner: String,
    locked_at: String,
    age_seconds: i64,
    stale: bool,
}

impl JsonList {
    fn new(locks: &LockList, now: OffsetDateTime, stale_days: u32) -> Self {
        let convert = |list: &[Lock]| {
            list.iter()
                .map(|l| JsonLock {
                    id: l.id.clone(),
                    path: l.path.clone(),
                    owner: l.owner.clone(),
                    locked_at: format_rfc3339(l.locked_at),
                    age_seconds: l.age(now).whole_seconds(),
                    stale: l.is_stale(now, stale_days),
                })
                .collect()
        };
        JsonList { stale_days, ours: convert(&locks.ours), theirs: convert(&locks.theirs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repo_tweaks_core::locks::parse_verify_json;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-10-18 18:30 UTC);

    fn sample() -> LockList {
        parse_verify_json(
            r#"{"ours":[{"id":"1","path":"afl/README.md","owner":{"name":"alice"},"locked_at":"2026-10-17T16:30:00Z"}],
                "theirs":[{"id":"2","path":"Assets/Scenes/Main.unity","owner":{"name":"bob"},"locked_at":"2026-10-14T01:00:00Z"},
                          {"id":"3","path":"Assets/Art/hero.psd","owner":{"name":"山田"},"locked_at":"2026-10-18T09:20:00Z"}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn renders_sections_with_stale_mark() {
        let expected = "\
自分のロック（1）
  afl/README.md             10/18 01:30 から  1日2時間

他人のロック（2）
  Assets/Scenes/Main.unity  bob   10/14 10:00 から  4日17時間  ⚠ 古い
  Assets/Art/hero.psd       山田  10/18 18:20 から  9時間

⚠ 3日以上ロックされたままのものが 1 件あります
";
        assert_eq!(render(&sample(), NOW, 3), expected);
    }

    #[test]
    fn renders_empty_states() {
        assert_eq!(render(&LockList::default(), NOW, 3), "ロックはありません\n");
        let mut only_theirs = sample();
        only_theirs.ours.clear();
        assert!(render(&only_theirs, NOW, 3).starts_with("自分のロック（0）\n  なし\n"));
    }

    #[test]
    fn json_output_shape() {
        let json = serde_json::to_value(JsonList::new(&sample(), NOW, 3)).unwrap();
        assert_eq!(json["stale_days"], 3);
        assert_eq!(json["ours"][0]["locked_at"], "2026-10-17T16:30:00Z");
        assert_eq!(json["ours"][0]["age_seconds"], 26 * 3600);
        assert_eq!(json["theirs"][0]["stale"], true);
        assert_eq!(json["theirs"][1]["stale"], false);
    }
}
