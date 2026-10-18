//! Where the server is and which token to use.
//!
//! Lookup order (first wins):
//! 1. Environment variables REPO_TWEAKS_SERVER / REPO_TWEAKS_TOKEN
//! 2. git config repo-tweaks.server / repo-tweaks.token
//!    (GUI clients such as Fork do not load shell profiles, so git config is the reliable place)

use repo_tweaks_core::git::{GitConfig, KEY_SERVER, KEY_TOKEN};

pub struct Config {
    pub server: String,
    pub token: String,
}

impl Config {
    pub fn load(git: &GitConfig) -> Result<Self, String> {
        let server = lookup("REPO_TWEAKS_SERVER", git, KEY_SERVER).ok_or(
            "サーバーの場所が未設定です。例: git config --global repo-tweaks.server http://localhost:8105",
        )?;
        let token = lookup("REPO_TWEAKS_TOKEN", git, KEY_TOKEN).ok_or(
            "トークンが未設定です。例: git config --global repo-tweaks.token <サーバーの .env と同じ値>",
        )?;
        Ok(Config { server: normalize_server(&server), token })
    }
}

fn lookup(env_name: &str, git: &GitConfig, git_key: &str) -> Option<String> {
    std::env::var(env_name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| git.get(git_key))
        .map(|v| v.trim().to_string())
}

fn normalize_server(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slash_removed() {
        assert_eq!(normalize_server(" http://localhost:8105/ "), "http://localhost:8105");
        assert_eq!(normalize_server("http://localhost:8105"), "http://localhost:8105");
    }
}
