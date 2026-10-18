//! HTTP client for the repo-tweaks server.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use repo_tweaks_core::git::RepoInfo;

// Keep hooks snappy: never stall a commit for long when the server is down.
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Event {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl Event {
    pub fn new(event: String, info: RepoInfo) -> Self {
        Event { event, repo: info.repo, remote: info.remote, branch: info.branch, commit: info.commit, user: info.user }
    }
}

/// An event as listed by the server.
#[derive(Deserialize)]
pub struct ReceivedEvent {
    pub id: u64,
    pub received_at: String,
    #[serde(flatten)]
    pub event: Event,
}

impl fmt::Display for ReceivedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let e = &self.event;
        let or_dash = |v: &Option<String>| v.as_deref().unwrap_or("-").to_string();
        let commit: String = e.commit.as_deref().unwrap_or("-").chars().take(8).collect();
        write!(
            f,
            "#{} {} {} repo={} remote={} branch={} commit={commit} user={}",
            self.id,
            self.received_at,
            e.event,
            or_dash(&e.repo),
            or_dash(&e.remote),
            or_dash(&e.branch),
            or_dash(&e.user),
        )
    }
}

pub struct Client {
    config: Config,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(config: Config) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Client { config, agent }
    }

    pub fn server(&self) -> &str {
        &self.config.server
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.server, path)
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.config.token)
    }

    pub fn ping(&self) -> Result<String, String> {
        let body = self.handle(self.agent.get(&self.url("/api/ping")).header("Authorization", &self.auth()).call())?;
        Ok(body["version"].as_str().unwrap_or("?").to_string())
    }

    pub fn send_event(&self, event: &Event) -> Result<u64, String> {
        let body = self.handle(
            self.agent.post(&self.url("/api/events")).header("Authorization", &self.auth()).send_json(event),
        )?;
        Ok(body["id"].as_u64().unwrap_or(0))
    }

    pub fn events(&self) -> Result<Vec<ReceivedEvent>, String> {
        let body = self.handle(self.agent.get(&self.url("/api/events")).header("Authorization", &self.auth()).call())?;
        serde_json::from_value(body["events"].clone()).map_err(|_| "サーバーの応答を読めませんでした".to_string())
    }

    /// Turn a response into its JSON body, or a user-facing error message.
    fn handle(&self, result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, String> {
        let mut response = result.map_err(|e| match e {
            ureq::Error::Timeout(_) => format!("サーバーが応答しません（{} 秒で打ち切り）: {}", TIMEOUT.as_secs(), self.server()),
            _ => format!("サーバーにつながりません: {}（{e}）", self.server()),
        })?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        if (200..300).contains(&status) {
            return serde_json::from_str(&text).map_err(|_| "サーバーの応答を読めませんでした".to_string());
        }
        Err(format!("サーバーがエラーを返しました（{status}）: {}", error_detail(&text)))
    }
}

/// Explain an error body: the server's JSON message, or the first line of a plain-text body.
fn error_detail(text: &str) -> String {
    if let Some(message) = serde_json::from_str::<Value>(text).ok().and_then(|v| v["error"].as_str().map(str::to_string)) {
        return message;
    }
    let line: String = text.lines().next().unwrap_or("").trim().chars().take(200).collect();
    match line.as_str() {
        // Starlette's TrustedHostMiddleware
        "Invalid host header" => {
            "このホスト名は許可されていません。サーバーの .env の REPO_TWEAKS_ALLOWED_HOSTS に、接続先のホスト名か IP を足してください".into()
        }
        "" => "理由不明（応答の本文が空です）".into(),
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_are_not_sent() {
        let event = Event::new("post-commit".into(), RepoInfo { repo: Some("demo".into()), ..Default::default() });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json, serde_json::json!({"event": "post-commit", "repo": "demo"}));
    }

    #[test]
    fn error_details() {
        assert_eq!(error_detail(r#"{"ok": false, "error": "トークンが正しくありません"}"#), "トークンが正しくありません");
        assert!(error_detail("Invalid host header").contains("REPO_TWEAKS_ALLOWED_HOSTS"));
        assert_eq!(error_detail("Bad Gateway\nmore"), "Bad Gateway");
        assert!(error_detail("").contains("理由不明"));
    }

    #[test]
    fn received_event_is_parsed_and_shown() {
        let json = serde_json::json!({
            "id": 3, "received_at": "2026-10-18T03:00:00Z", "event": "post-commit",
            "repo": "demo", "branch": "main", "commit": "0123456789abcdef", "user": "alice"
        });
        let e: ReceivedEvent = serde_json::from_value(json).unwrap();
        assert_eq!(
            e.to_string(),
            "#3 2026-10-18T03:00:00Z post-commit repo=demo remote=- branch=main commit=01234567 user=alice"
        );
    }
}
