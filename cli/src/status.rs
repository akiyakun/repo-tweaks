//! `repo-tweaks status`: files that need a lock (`lockable`), who holds them, and what you changed.

use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::GitConfig;
use repo_tweaks_core::lockable::{self, Change, FileStatus, Problem};
use repo_tweaks_core::locks::format_rfc3339;
use repo_tweaks_core::ops::State;

use crate::text::pad;

pub fn run(config: &GitConfig, all: bool, json: bool) -> Result<(), String> {
    let report = lockable::status(config)?;
    if json {
        let out = JsonStatus { files: report.files.iter().map(JsonFile::new).collect() };
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    } else {
        print!("{}", render(&report.files, all));
    }
    Ok(())
}

fn render(files: &[FileStatus], all: bool) -> String {
    if files.is_empty() {
        return "対象のファイルがありません（.gitattributes で lockable を付けたファイルが無い）\n".into();
    }
    let shown: Vec<&FileStatus> =
        files.iter().filter(|f| all || f.change.is_some() || !matches!(f.state, State::Free)).collect();
    let mut out = if all {
        format!("ロックが必要なファイル（{}）\n", files.len())
    } else {
        format!("ロックが必要なファイル {} 件のうち、ロック中・変更中のもの（{}）\n", files.len(), shown.len())
    };
    if shown.is_empty() {
        out.push_str("  なし\n");
        return out;
    }
    let locks: Vec<String> = shown.iter().map(|f| lock_label(&f.state)).collect();
    let path_width = shown.iter().map(|f| f.path.width()).max().unwrap_or(0);
    let lock_width = locks.iter().map(|l| l.width()).max().unwrap_or(0);
    let change_width = shown.iter().map(|f| change_label(f.change).width()).max().unwrap_or(0);
    for (file, lock) in shown.iter().zip(&locks) {
        let mut line = format!(
            "  {}  {}  {}",
            pad(&file.path, path_width),
            pad(lock, lock_width),
            pad(change_label(file.change), change_width)
        );
        match file.problem() {
            Some(Problem::NotLocked) => line.push_str("  ⚠ ロックが必要"),
            Some(Problem::HeldByOther) => line.push_str("  ⚠ 他人がロック中"),
            None => {}
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let count = |p| files.iter().filter(|f| f.problem() == Some(p)).count();
    let (not_locked, held) = (count(Problem::NotLocked), count(Problem::HeldByOther));
    if not_locked > 0 {
        out.push_str(&format!(
            "\n⚠ ロックせずに変更しているものが {not_locked} 件あります。repo-tweaks lock でロックしてからコミットしてください\n"
        ));
    }
    if held > 0 {
        out.push_str(&format!(
            "\n⚠ 他人がロック中のファイルを変更しているものが {held} 件あります。変更を取り消すか、ロックしている人と相談してください\n"
        ));
    }
    out
}

fn lock_label(state: &State) -> String {
    match state {
        State::Free => "ロックなし".into(),
        State::Ours(_) => "自分がロック中".into(),
        State::Theirs(lock) => format!("{} がロック中", lock.owner),
    }
}

fn change_label(change: Option<Change>) -> &'static str {
    match change {
        None => "",
        Some(Change::Modified) => "変更あり",
        Some(Change::Deleted) => "削除",
        Some(Change::Added) => "追加",
    }
}

/// Machine-readable output for other tools (Unity, GUI): every lockable file. Times are UTC.
#[derive(Serialize)]
struct JsonStatus {
    files: Vec<JsonFile>,
}

#[derive(Serialize)]
struct JsonFile {
    path: String,
    /// "none" / "ours" / "theirs"
    lock: &'static str,
    owner: Option<String>,
    locked_at: Option<String>,
    /// null / "modified" / "deleted" / "added"
    change: Option<&'static str>,
    /// null / "not_locked" / "held_by_other": the change cannot be committed as it is
    problem: Option<&'static str>,
}

impl JsonFile {
    fn new(file: &FileStatus) -> Self {
        let (lock, holder) = match &file.state {
            State::Free => ("none", None),
            State::Ours(l) => ("ours", Some(l)),
            State::Theirs(l) => ("theirs", Some(l)),
        };
        JsonFile {
            path: file.path.clone(),
            lock,
            owner: holder.map(|l| l.owner.clone()),
            locked_at: holder.map(|l| format_rfc3339(l.locked_at)),
            change: file.change.map(|c| match c {
                Change::Modified => "modified",
                Change::Deleted => "deleted",
                Change::Added => "added",
            }),
            problem: file.problem().map(|p| match p {
                Problem::NotLocked => "not_locked",
                Problem::HeldByOther => "held_by_other",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repo_tweaks_core::locks::Lock;
    use time::macros::datetime;

    fn lock(owner: &str) -> Lock {
        Lock { id: "1".into(), path: String::new(), owner: owner.into(), locked_at: datetime!(2026-10-18 00:00 UTC) }
    }

    fn files() -> Vec<FileStatus> {
        let f = |path: &str, state, change| FileStatus { path: path.into(), state, change };
        vec![
            f("Assets/Art/hero.psd", State::Theirs(lock("bob")), None),
            f("Assets/Main.unity", State::Ours(lock("alice")), Some(Change::Modified)),
            f("Assets/New.prefab", State::Free, Some(Change::Added)),
            f("Assets/Quiet.unity", State::Free, None),
            f("Assets/Enemy.prefab", State::Free, Some(Change::Modified)),
        ]
    }

    #[test]
    fn shows_locked_and_changed_files_with_warnings() {
        let expected = "\
ロックが必要なファイル 5 件のうち、ロック中・変更中のもの（4）
  Assets/Art/hero.psd  bob がロック中
  Assets/Main.unity    自分がロック中  変更あり
  Assets/New.prefab    ロックなし      追加
  Assets/Enemy.prefab  ロックなし      変更あり  ⚠ ロックが必要

⚠ ロックせずに変更しているものが 1 件あります。repo-tweaks lock でロックしてからコミットしてください
";
        assert_eq!(render(&files(), false), expected);
    }

    #[test]
    fn all_and_empty() {
        assert!(render(&files(), true).starts_with("ロックが必要なファイル（5）\n"));
        assert!(render(&files(), true).contains("  Assets/Quiet.unity   ロックなし\n"));
        assert!(render(&[], false).starts_with("対象のファイルがありません"));
        let quiet = vec![FileStatus { path: "a.unity".into(), state: State::Free, change: None }];
        assert_eq!(render(&quiet, false), "ロックが必要なファイル 1 件のうち、ロック中・変更中のもの（0）\n  なし\n");
    }

    #[test]
    fn json_shape() {
        let json: Vec<_> = files().iter().map(|f| serde_json::to_value(JsonFile::new(f)).unwrap()).collect();
        assert_eq!(json[0]["lock"], "theirs");
        assert_eq!(json[0]["owner"], "bob");
        assert_eq!(json[0]["locked_at"], "2026-10-18T00:00:00Z");
        assert_eq!(json[0]["change"], serde_json::Value::Null);
        assert_eq!(json[2]["change"], "added");
        assert_eq!(json[2]["problem"], serde_json::Value::Null);
        assert_eq!(json[4]["problem"], "not_locked");
    }
}
