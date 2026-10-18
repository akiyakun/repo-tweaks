//! repo-tweaks GUI (Tauri). Logic lives in repo-tweaks-core; this crate turns it into screen data.

// No console window next to the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod view;

use repo_tweaks_core::git::{self, GitConfig, KEY_STALE_DAYS};
use repo_tweaks_core::lfs;
use repo_tweaks_core::locks::resolve_stale_days;
use repo_tweaks_core::paths::RepoContext;
use time::OffsetDateTime;

use view::LockReport;

/// The lock list of the repository containing `dir`.
#[tauri::command]
async fn list_locks(dir: String) -> Result<LockReport, String> {
    // git lfs asks the server, which can take seconds: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let ctx = RepoContext::open(&dir)?;
        let config = GitConfig::read_in(&ctx.root);
        let stale_days = resolve_stale_days(None, config.get(KEY_STALE_DAYS))?;
        let locks = lfs::fetch_locks_in(&ctx.root, &config)?;
        Ok(LockReport::new(&ctx.root, &locks, OffsetDateTime::now_utc(), stale_days))
    })
    .await
    .map_err(|e| format!("ロック一覧の取得が途中で止まりました（{e}）"))?
}

/// A folder given on the command line (`repo-tweaks-gui <folder>`, e.g. from a Fork custom command).
#[tauri::command]
fn initial_folder() -> Option<String> {
    std::env::args().nth(1).filter(|a| !a.starts_with('-'))
}

/// Apps opened from Finder get a minimal PATH, so git would not find a Homebrew git-lfs.
#[cfg(target_os = "macos")]
fn extend_path() {
    use std::path::{Path, PathBuf};
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
        if !paths.iter().any(|p| p == Path::new(extra)) {
            paths.push(extra.into());
        }
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: runs first thing in main, before any other thread exists.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    extend_path();
    git::hide_console_windows();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_locks, initial_folder])
        .run(tauri::generate_context!())
        .expect("repo-tweaks の起動に失敗しました");
}
