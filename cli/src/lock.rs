//! `repo-tweaks lock` / `repo-tweaks unlock`.

use std::io::{BufRead, IsTerminal, Write};

use serde::Serialize;
use time::OffsetDateTime;
use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::GitConfig;
use repo_tweaks_core::locks::{Lock, format_jst, format_rfc3339};
use repo_tweaks_core::notify::{self, Notified};
use repo_tweaks_core::ops::{self, Outcome, Target};

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
    if !opts.no_notify && notify::enabled(config) {
        match notify::send_lock_events(".", config, results) {
            Notified::NoServer => eprintln!("repo-tweaks: {}", notify::NO_SERVER_NOTICE),
            Notified::Failed(message) => eprintln!("repo-tweaks: {message}"),
            Notified::Nothing | Notified::Sent(_) => {}
        }
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

fn render(results: &[(Target, Outcome)], now: OffsetDateTime) -> String {
    let rows: Vec<(&str, String, Option<String>)> = results
        .iter()
        .map(|(target, outcome)| {
            let (label, detail) = outcome.describe(now);
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
しませんでした  Assets/Art/hero.psd.meta（自動で追加）  対になるファイル（.meta と元のファイル）の片方を処理できないため、そろえてそのままにしました
";
        assert_eq!(render(&results, NOW), expected);
    }

    #[test]
    fn unlock_without_force_points_to_force() {
        let (label, detail) = Outcome::OthersLock(bob()).describe(NOW);
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

}
