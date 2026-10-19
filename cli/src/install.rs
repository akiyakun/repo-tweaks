//! `repo-tweaks install-hooks`: put the pre-commit and post-commit hooks into this repository.
//!
//! The hooks folder comes from git (`core.hooksPath` is honored, but not one set for every repository).
//! A hook that is already there is replaced only when it is ours (an older version) or Git LFS's
//! standard one; anything else is left alone unless --force, which first keeps it as `<name>.bak`.
//! Missing Git LFS hooks are written too: `git lfs install` stops at the first hook that exists.

use std::fs;
use std::path::{Path, PathBuf};

use repo_tweaks_core::git;
use repo_tweaks_core::paths::RepoContext;

const HOOKS: [(&str, &str); 2] = [
    ("pre-commit", include_str!("../../hooks/pre-commit")),
    ("post-commit", include_str!("../../hooks/post-commit")),
];
/// Every hook this tool writes says so in its header
const MARKER: &str = "hook for repo-tweaks";
/// The hooks Git LFS needs besides post-commit (which ours includes)
const LFS_HOOKS: [&str; 3] = ["pre-push", "post-checkout", "post-merge"];

#[derive(Debug, PartialEq)]
enum Action {
    Create,
    Same,
    /// Ours, from an older version
    Update,
    /// Git LFS's own hook (ours runs the same `git lfs ...` step)
    ReplaceLfs,
    /// Someone else's hook: kept unless --force
    Foreign,
}

pub fn run(force: bool) -> Result<(), String> {
    let ctx = RepoContext::current()?;
    if let Some(shared) = shared_hooks_path(&ctx)? {
        return Err(format!(
            "core.hooksPath が、すべてのリポジトリに効く設定（{shared}）になっているため、入れませんでした。\n  入れると、ほかのリポジトリでもコミットの確認と通知が動きます。このリポジトリだけで使うときは、hooks/ のサンプルを手で組み込んでください"
        ));
    }
    let dir = hooks_dir(&ctx)?;
    fs::create_dir_all(&dir).map_err(|e| format!("{} を作れませんでした（{e}）", dir.display()))?;
    println!("hook のフォルダ: {}", dir.display());

    let mut kept = 0;
    for (name, content) in HOOKS {
        let path = dir.join(name);
        let existing = read_existing(&path)?;
        let action = decide(name, existing.as_deref(), content);
        let message = match (&action, force) {
            (Action::Same, _) => format!("{name}: 入っています（同じ内容）"),
            (Action::Foreign, false) => {
                kept += 1;
                format!("{name}: 別の hook があるため、そのままにしました（--force で {name}.bak に残して上書きします）")
            }
            (Action::Foreign, true) => {
                let backup = backup_path(&dir, name);
                fs::rename(&path, &backup).map_err(|e| format!("{name} を {} に残せませんでした（{e}）", backup.display()))?;
                write_hook(&path, content)?;
                format!("{name}: 入れました（元の hook は {} に残しました）", file_name(&backup))
            }
            (Action::Create, _) => {
                write_hook(&path, content)?;
                format!("{name}: 入れました")
            }
            (Action::Update, _) => {
                write_hook(&path, content)?;
                format!("{name}: 新しくしました")
            }
            (Action::ReplaceLfs, _) => {
                write_hook(&path, content)?;
                format!("{name}: Git LFS の hook を置き換えました（Git LFS の処理も続けて呼びます）")
            }
        };
        println!("{message}");
    }

    // Git LFS's own hooks keep lockable files read-only and check locks on push
    for name in LFS_HOOKS {
        let path = dir.join(name);
        match read_existing(&path)? {
            None => {
                write_hook(&path, &lfs_hook(name))?;
                println!("{name}: Git LFS の hook を入れました");
            }
            Some(text) if text.contains(&format!("git lfs {name}")) => {}
            Some(_) => println!(
                "⚠ {name}: 別の hook があり、Git LFS の処理が入っていません。その hook に git lfs {name} \"$@\" の行を足してください"
            ),
        }
    }
    match kept {
        0 => Ok(()),
        n => Err(format!("{n} 個の hook は入れていません")),
    }
}

/// core.hooksPath when it comes from the user's or the system's git config (shared by every repository).
fn shared_hooks_path(ctx: &RepoContext) -> Result<Option<String>, String> {
    let get = |scope: Option<&str>| -> Result<Option<String>, String> {
        let mut args = vec!["config"];
        args.extend(scope);
        args.extend(["--get", "core.hooksPath"]);
        let output = git::run_in(&ctx.root, &args)?;
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((output.status.success() && !value.is_empty()).then_some(value))
    };
    let effective = get(None)?;
    let in_repo = get(Some("--local"))?.is_some() || get(Some("--worktree"))?.is_some();
    Ok(effective.filter(|_| !in_repo))
}

/// Where git looks for hooks (honors core.hooksPath and worktrees), as an absolute path.
fn hooks_dir(ctx: &RepoContext) -> Result<PathBuf, String> {
    let output = git::run_in(&ctx.root, &["rev-parse", "--git-path", "hooks"])?;
    if !output.status.success() {
        return Err("hook のフォルダが分かりませんでした".into());
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Relative paths are relative to where git ran (the root)
    Ok(Path::new(&ctx.root).join(text))
}

fn decide(name: &str, existing: Option<&str>, ours: &str) -> Action {
    let Some(existing) = existing else { return Action::Create };
    let normalize = |s: &str| s.replace("\r\n", "\n").trim_end().to_string();
    if normalize(existing) == normalize(ours) {
        Action::Same
    } else if existing.contains(MARKER) {
        Action::Update
    } else if is_lfs_hook(name, existing) {
        Action::ReplaceLfs
    } else {
        Action::Foreign
    }
}

/// The hook `git lfs install` writes: a shebang, a check that git-lfs exists, and `git lfs <name> "$@"`.
fn is_lfs_hook(name: &str, text: &str) -> bool {
    let call = format!("git lfs {name} \"$@\"");
    let mut has_call = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == call {
            has_call = true;
        } else if !(line.starts_with("#!") || line.starts_with("command -v git-lfs")) {
            return false;
        }
    }
    has_call
}

/// The hook as `git lfs install` (git-lfs 3.7) writes it
fn lfs_hook(name: &str) -> String {
    format!(
        "#!/bin/sh\ncommand -v git-lfs >/dev/null 2>&1 || {{ printf >&2 \"\\n%s\\n\\n\" \"This repository is configured for Git LFS but 'git-lfs' was not found on your path. If you no longer wish to use Git LFS, remove this hook by deleting the '{name}' file in the hooks directory (set by 'core.hookspath'; usually '.git/hooks').\"; exit 2; }}\ngit lfs {name} \"$@\"\n"
    )
}

/// An existing hook's text (None when there is none). Bytes that are not UTF-8 are kept as
/// replacement characters, so such a hook is judged "someone else's" and never overwritten silently.
fn read_existing(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} を読めませんでした（{e}）", path.display())),
    }
}

/// `<name>.bak`, or `<name>.bak2`, `.bak3`, ... when that is taken
fn backup_path(dir: &Path, name: &str) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{name}.bak") } else { format!("{name}.bak{n}") }))
        .find(|p| !p.exists())
        .expect("some backup name is free")
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn write_hook(path: &Path, content: &str) -> Result<(), String> {
    // Always LF: the hook runs in sh, also on Windows (Git for Windows)
    fs::write(path, content.replace("\r\n", "\n")).map_err(|e| format!("{} に書けませんでした（{e}）", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{} を実行できるようにできませんでした（{e}）", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LFS_POST_COMMIT: &str = "#!/bin/sh\ncommand -v git-lfs >/dev/null 2>&1 || { printf >&2 \"\\n%s\\n\\n\" \"This repository is configured for Git LFS but 'git-lfs' was not found on your path.\"; exit 2; }\ngit lfs post-commit \"$@\"\n";

    #[test]
    fn existing_hooks_are_judged() {
        let ours = HOOKS[1].1;
        assert_eq!(decide("post-commit", None, ours), Action::Create);
        assert_eq!(decide("post-commit", Some(ours), ours), Action::Same);
        assert_eq!(decide("post-commit", Some(&ours.replace('\n', "\r\n")), ours), Action::Same);
        assert_eq!(decide("post-commit", Some("#!/bin/sh\n# post-commit hook for repo-tweaks.\nold\n"), ours), Action::Update);
        assert_eq!(decide("post-commit", Some(LFS_POST_COMMIT), ours), Action::ReplaceLfs);
        // Someone's own steps around the LFS call make it theirs
        let custom = LFS_POST_COMMIT.replace("git lfs post-commit", "make docs\ngit lfs post-commit");
        assert_eq!(decide("post-commit", Some(&custom), ours), Action::Foreign);
        // The LFS hook of another name does not count
        assert_eq!(decide("pre-commit", Some(LFS_POST_COMMIT), HOOKS[0].1), Action::Foreign);
    }

    #[test]
    fn both_hooks_carry_the_marker() {
        for (name, content) in HOOKS {
            assert!(content.contains(MARKER), "{name}");
            assert!(content.starts_with("#!/bin/sh\n"), "{name}");
        }
    }

    #[test]
    fn backups_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("repo-tweaks-install-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(file_name(&backup_path(&dir, "pre-commit")), "pre-commit.bak");
        fs::write(dir.join("pre-commit.bak"), "x").unwrap();
        assert_eq!(file_name(&backup_path(&dir, "pre-commit")), "pre-commit.bak2");
        // A hook that is not UTF-8 (e.g. Shift_JIS comments) is someone else's, not a missing one
        let sjis = dir.join("pre-commit");
        fs::write(&sjis, b"#!/bin/sh\n# \x93\xfa\x96\x7b\nmake check\n").unwrap();
        let existing = read_existing(&sjis).unwrap();
        assert_eq!(decide("pre-commit", existing.as_deref(), HOOKS[0].1), Action::Foreign);
        assert_eq!(read_existing(&dir.join("none")).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lfs_hooks_look_like_git_lfs_ones() {
        for name in LFS_HOOKS {
            assert!(is_lfs_hook(name, &lfs_hook(name)), "{name}");
        }
        // Same text as git-lfs 3.7 writes, apart from the hook name
        assert_eq!(lfs_hook("post-commit").lines().count(), LFS_POST_COMMIT.lines().count());
    }
}
