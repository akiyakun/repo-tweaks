//! repo-tweaks GUI (Tauri). Logic lives in repo-tweaks-core; this crate turns it into screen data.

// No console window next to the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod view;

use std::sync::Mutex;
use std::time::Duration;

use repo_tweaks_core::git::{self, GitConfig, KEY_STALE_DAYS};
use repo_tweaks_core::lfs;
use repo_tweaks_core::locks::resolve_stale_days;
use repo_tweaks_core::notify::{self, Notified};
use repo_tweaks_core::ops::{self, Outcome, Target};
use repo_tweaks_core::paths::RepoContext;
use time::OffsetDateTime;

use view::{LockReport, OpReport, OthersLock};

/// git lfs shares a lock cache per repository (.git/lfs/lockcache.db), so its calls must not overlap.
/// The GUI runs one at a time; a hung git is stopped by the time limit, which frees the next one.
static GIT_LFS: Mutex<()> = Mutex::new(());

/// Longest any single git process may run (see `git::set_timeout`).
const GIT_TIME_LIMIT: Duration = Duration::from_secs(60);

/// Run `work` off the UI thread (git lfs asks the server, which can take seconds), one at a time.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _turn = GIT_LFS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        work()
    })
    .await
    .map_err(|e| format!("処理が途中で止まりました（{e}）"))?
}

/// The lock list of the repository containing `dir`.
#[tauri::command]
async fn list_locks(dir: String) -> Result<LockReport, String> {
    blocking(move || {
        let ctx = RepoContext::open(&dir)?;
        let config = GitConfig::read_in(&ctx.root);
        let stale_days = resolve_stale_days(None, config.get(KEY_STALE_DAYS))?;
        let locks = lfs::fetch_locks_in(&ctx, &config)?;
        let name = git::repo_name(&config, Some(&ctx.root)).unwrap_or_else(|| ctx.root.clone());
        Ok(LockReport::new(&ctx.root, name, &locks, OffsetDateTime::now_utc(), stale_days))
    })
    .await
}

/// Lock files of the repository at `root` (with their Unity `.meta`). Paths are absolute or
/// relative to `root`.
#[tauri::command]
async fn lock_files(root: String, paths: Vec<String>) -> Result<OpReport, String> {
    blocking(move || {
        let ctx = RepoContext::open(&root)?;
        let config = GitConfig::read_in(&ctx.root);
        let root = ctx.root.clone();
        let results = ops::plan_lock_in(ctx, &config, &paths, true)?.lock();
        Ok(report(&root, &config, &results))
    })
    .await
}

/// Tell the server (Discord) what changed, as the CLI does, and pack it with the results.
fn report(root: &str, config: &GitConfig, results: &[(Target, Outcome)]) -> OpReport {
    let notified = if notify::enabled(config) { notify::send_lock_events(root, config, results) } else { Notified::Nothing };
    OpReport::new(results, notified, OffsetDateTime::now_utc())
}

/// Other people's locks that a forced unlock of `paths` would remove (for the confirmation).
#[tauri::command]
async fn preview_force_unlock(root: String, paths: Vec<String>) -> Result<Vec<OthersLock>, String> {
    blocking(move || {
        let ctx = RepoContext::open(&root)?;
        let config = GitConfig::read_in(&ctx.root);
        let plan = ops::plan_unlock_in(ctx, &config, &paths, true, true)?;
        let now = OffsetDateTime::now_utc();
        Ok(plan.others_locks().into_iter().map(|l| OthersLock::new(l, now)).collect())
    })
    .await
}

/// Unlock files (and the `.meta` partners we may unlock). With `force`, other people's locks are
/// removed only if every one of them is among `confirmed` (lock IDs the user agreed to): the plan
/// is made again here, and a lock that changed since the confirmation stops everything.
#[tauri::command]
async fn unlock_files(root: String, paths: Vec<String>, force: bool, confirmed: Vec<String>) -> Result<OpReport, String> {
    blocking(move || {
        let ctx = RepoContext::open(&root)?;
        let config = GitConfig::read_in(&ctx.root);
        let root = ctx.root.clone();
        let plan = ops::plan_unlock_in(ctx, &config, &paths, true, force)?;
        if plan.others_locks().iter().any(|l| !confirmed.contains(&l.id)) {
            return Err("確認したあとにロックの状態が変わりました。一覧を更新して、もう一度やり直してください（何も変更していません）".into());
        }
        Ok(report(&root, &config, &plan.unlock()))
    })
    .await
}

/// The first of `paths` that is a folder (for a drag and drop onto the window).
#[tauri::command]
fn dropped_folder(paths: Vec<String>) -> Option<String> {
    paths.into_iter().find(|p| std::path::Path::new(p).is_dir())
}

/// A folder given on the command line (`repo-tweaks-gui <folder>`, e.g. from a Fork custom command),
/// made absolute so that it still means the same folder when remembered for the next start.
#[tauri::command]
fn initial_folder() -> Option<String> {
    let arg = std::env::args_os().nth(1)?;
    if arg.to_string_lossy().starts_with('-') {
        return None;
    }
    let path = std::path::absolute(&arg).unwrap_or_else(|_| arg.into());
    Some(path.to_string_lossy().into_owned())
}

/// The app has no terminal to answer git's questions on, so make git fail instead of waiting
/// (e.g. `npm run tauri dev` started from a terminal without saved credentials).
fn disable_terminal_prompts() {
    // SAFETY: runs first thing in main, before any other thread exists.
    unsafe { std::env::set_var("GIT_TERMINAL_PROMPT", "0") };
}

/// Apps opened from Finder get a minimal PATH (/usr/bin:/bin:...), so git would not find a Homebrew
/// git-lfs. Put the package managers' bins first, as a terminal usually does, so the same git as the
/// CLI's is used (and /usr/bin/git, which needs the Command Line Tools, only as a fallback).
#[cfg(target_os = "macos")]
fn extend_path() {
    use std::path::{Path, PathBuf};
    let current = std::env::var_os("PATH").unwrap_or_default();
    let extras = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];
    let mut paths: Vec<PathBuf> = extras.iter().map(PathBuf::from).collect();
    paths.extend(std::env::split_paths(&current).filter(|p| !extras.iter().any(|e| p == Path::new(e))));
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: runs first thing in main, before any other thread exists.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

fn main() {
    disable_terminal_prompts();
    #[cfg(target_os = "macos")]
    extend_path();
    git::hide_console_windows();
    git::set_timeout(GIT_TIME_LIMIT);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            list_locks,
            lock_files,
            preview_force_unlock,
            unlock_files,
            dropped_folder,
            initial_folder
        ])
        .run(tauri::generate_context!())
        .expect("repo-tweaks の起動に失敗しました");
}
