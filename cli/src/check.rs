//! `repo-tweaks check`: run from the pre-commit hook. Stops a commit that changes or deletes a
//! lockable file you have not locked. If the locks cannot be read, it warns and lets the commit through.

use std::process::ExitCode;

use unicode_width::UnicodeWidthStr;

use repo_tweaks_core::git::GitConfig;
use repo_tweaks_core::lockable::{self, CheckResult, FileStatus, Problem};
use repo_tweaks_core::ops::State;

use crate::text::pad;

pub fn run(config: &GitConfig) -> ExitCode {
    match lockable::check_staged(config) {
        CheckResult::Nothing | CheckResult::Passed => ExitCode::SUCCESS,
        CheckResult::Unverified { files, reason } => {
            eprintln!("{}", unverified_message(files, &reason));
            ExitCode::SUCCESS
        }
        CheckResult::Blocked(files) => {
            eprint!("{}", blocked_message(&files));
            ExitCode::FAILURE
        }
    }
}

fn unverified_message(files: usize, reason: &str) -> String {
    let what = if files > 0 { format!("ロックが必要なファイル {files} 件の") } else { String::new() };
    format!("repo-tweaks: {what}ロックを確かめられなかったため、確かめずにコミットします。{reason}")
}

fn blocked_message(files: &[FileStatus]) -> String {
    let mut out = String::from("repo-tweaks: ロックしていないファイルを変更しているため、コミットを止めました\n");
    let width = files.iter().map(|f| f.path.width()).max().unwrap_or(0);
    for file in files {
        let why = match (&file.state, file.problem()) {
            (State::Theirs(lock), _) => format!("{} がロック中", lock.owner),
            (_, Some(Problem::NotLocked)) => "ロックなし".into(),
            _ => String::new(),
        };
        out.push_str(&format!("  {}  {why}\n", pad(&file.path, width)));
    }
    let free: Vec<&str> =
        files.iter().filter(|f| f.problem() == Some(Problem::NotLocked)).map(|f| f.path.as_str()).collect();
    if !free.is_empty() {
        let args: Vec<String> = free.iter().map(|p| quote(p)).collect();
        out.push_str(&format!("ロックしてから、もう一度コミットしてください:\n  repo-tweaks lock {}\n", args.join(" ")));
    }
    if files.iter().any(|f| f.problem() == Some(Problem::HeldByOther)) {
        out.push_str("他人がロック中のファイルは、変更を取り消すか、ロックしている人と相談してください\n");
    }
    out
}

/// Quote a path for the shell when it needs it, so the suggested command can be pasted as is.
/// Double quotes for spaces (cmd.exe understands them too); single quotes when double quotes would
/// still expand something (`$`, backquotes, ...).
fn quote(path: &str) -> String {
    if path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@".contains(c) || !c.is_ascii()) {
        path.to_string()
    } else if path.contains(['$', '`', '"', '\\', '!']) {
        format!("'{}'", path.replace('\'', r"'\''"))
    } else {
        format!("\"{path}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repo_tweaks_core::lockable::Change;
    use repo_tweaks_core::locks::Lock;
    use time::macros::datetime;

    #[test]
    fn blocked_message_lists_files_and_the_fix() {
        let bob = Lock { id: "1".into(), path: "b.psd".into(), owner: "bob".into(), locked_at: datetime!(2026-10-18 0:00 UTC) };
        let files = vec![
            FileStatus { path: "Assets/Main.unity".into(), state: State::Free, change: Some(Change::Modified) },
            FileStatus { path: "Art/My Hero.psd".into(), state: State::Free, change: Some(Change::Deleted) },
            FileStatus { path: "b.psd".into(), state: State::Theirs(bob), change: Some(Change::Modified) },
        ];
        let expected = "\
repo-tweaks: ロックしていないファイルを変更しているため、コミットを止めました
  Assets/Main.unity  ロックなし
  Art/My Hero.psd    ロックなし
  b.psd              bob がロック中
ロックしてから、もう一度コミットしてください:
  repo-tweaks lock Assets/Main.unity \"Art/My Hero.psd\"
他人がロック中のファイルは、変更を取り消すか、ロックしている人と相談してください
";
        assert_eq!(blocked_message(&files), expected);
    }

    #[test]
    fn suggested_paths_survive_the_shell() {
        assert_eq!(quote("Assets/Main.unity"), "Assets/Main.unity");
        assert_eq!(quote("Assets/背景.psd"), "Assets/背景.psd");
        assert_eq!(quote("Art/My Hero.psd"), "\"Art/My Hero.psd\"");
        assert_eq!(quote("Assets/$Temp/a.unity"), "'Assets/$Temp/a.unity'");
        assert_eq!(quote("it's $x"), r"'it'\''s $x'");
    }

    #[test]
    fn unverified_message_says_the_commit_goes_through() {
        assert_eq!(
            unverified_message(2, "サーバーにつながりませんでした。"),
            "repo-tweaks: ロックが必要なファイル 2 件のロックを確かめられなかったため、確かめずにコミットします。サーバーにつながりませんでした。"
        );
        assert!(unverified_message(0, "x").starts_with("repo-tweaks: ロックを確かめられなかった"));
    }
}
