//! `repo-tweaks lock` / `repo-tweaks unlock`.

use std::io::{BufRead, IsTerminal, Write};

use serde::Serialize;
use time::OffsetDateTime;
use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::GitConfig;
use repo_tweaks_core::locks::{Lock, format_jst, format_rfc3339};
use repo_tweaks_core::ops::{self, Outcome, Target};

use crate::text::pad;

pub fn lock(config: &GitConfig, paths: &[String], with_meta: bool, json: bool) -> Result<(), String> {
    let plan = ops::plan_lock(config, paths, with_meta)?;
    report(&plan.lock(), json)
}

pub fn unlock(config: &GitConfig, paths: &[String], with_meta: bool, force: bool, yes: bool, json: bool) -> Result<(), String> {
    let plan = ops::plan_unlock(config, paths, with_meta, force)?;
    let others = plan.others_locks();
    if force && !others.is_empty() && !yes {
        confirm_force(&others, OffsetDateTime::now_utc())?;
    }
    report(&plan.unlock(), json)
}

/// Ask on the terminal before removing other people's locks. Without a terminal, require --yes.
fn confirm_force(others: &[&Lock], now: OffsetDateTime) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("他人のロックの強制解除には確認が必要です。確認なしで実行するには --yes を付けてください".into());
    }
    let path_width = others.iter().map(|l| l.path.width()).max().unwrap_or(0);
    let mut err = std::io::stderr();
    let _ = writeln!(err, "次の他人のロックを強制解除します（持ち主には通知されません）:");
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

fn report(results: &[(Target, Outcome)], json: bool) -> Result<(), String> {
    let now = OffsetDateTime::now_utc();
    if json {
        let rows: Vec<JsonResult> = results.iter().map(|(t, o)| JsonResult::new(t, o)).collect();
        println!("{}", serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?);
    } else {
        print!("{}", render(results, now));
    }
    match results.iter().filter(|(_, o)| o.is_failure()).count() {
        0 => Ok(()),
        n => Err(format!("{n} 件は処理できませんでした")),
    }
}

fn describe(outcome: &Outcome, now: OffsetDateTime) -> (&'static str, Option<String>) {
    let since = |l: &Lock| format_jst(l.locked_at, now);
    match outcome {
        Outcome::Locked => ("ロックしました", None),
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
            Outcome::Locked => ("locked", None, None),
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
            (target("Assets/Main.unity", false), Outcome::Locked),
            (target("Assets/Main.unity.meta", true), Outcome::Locked),
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
        let row = serde_json::to_value(JsonResult::new(&target("a.psd.meta", true), &Outcome::Locked)).unwrap();
        assert_eq!(row["ok"], true);
        assert_eq!(row["meta_pair"], true);
        assert!(row.get("owner").is_none());
    }

    #[test]
    fn failures_set_exit_code() {
        assert!(report(&[(target("a", false), Outcome::NotLocked)], true).is_ok());
        assert_eq!(
            report(&[(target("a", false), Outcome::NoSuchFile), (target("b", false), Outcome::Locked)], true),
            Err("1 件は処理できませんでした".into())
        );
    }
}
