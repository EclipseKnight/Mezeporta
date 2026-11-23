//! Hunsterverse (1.5.4-hv2): choose the client patch (Stable or a pre-release
//! slot) from the launcher.
//!
//! There is one source of truth, the account's `users.patch_channel`: the
//! website's Account page sets it through the REST API
//! (`GET/PUT /api/account/patch-channel`, signed in with `/api/auth/signin`),
//! and Erupe reads it at each launcher login to answer with a `patchServer`:
//! the stable patch (the wrapper's own `/check`) or
//! `<PreRelease.PatchServer>/<slot>` (`/prerelease/<slot>/check`) for an
//! eligible account whose chosen slot is published. The launcher calls the same
//! API with the player's own username and password, then logs in again so the
//! normal patch flow (restore, download, move, `patcher.rs`) applies it.
//!
//! What is installed is recorded per server in
//! `<game>/Mezeporta/Servers/<host_port>/hv-patch.json` when a patch finishes
//! (patcher.rs), with the version name once the launcher has seen the server
//! give that same patch (versions come from the API, not from the game files).

use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::Path};

/// The REST API the website uses (js/account-patch-channel.js).
pub const HUNSTERVERSE_API: &str = "https://api.hvmhfz.eclipseforge.xyz";
/// Hosts of the Hunsterverse game server, as the installers set it up.
pub const HUNSTERVERSE_HOSTS: &[&str] = &["129.213.54.221"];
/// Test override: an API base URL used for every server.
pub const ACCOUNT_API_ENV: &str = "MEZEPORTA_HV_ACCOUNT_API";

const RECORD_FILE: &str = "hv-patch.json";
const SERVER_CACHE_DIR: &str = "Mezeporta/Servers";

/// The account API for a server host, if it is Hunsterverse.
pub fn account_api_for(host: &str) -> Option<String> {
    if let Some(url) = std::env::var(ACCOUNT_API_ENV)
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
    {
        return Some(url);
    }
    let host = host.trim().to_ascii_lowercase();
    HUNSTERVERSE_HOSTS
        .iter()
        .any(|h| *h == host)
        .then(|| HUNSTERVERSE_API.to_string())
}

/// The channel a patchServer URL stands for: "pre:<slot>" for Erupe's
/// `.../prerelease/<slot>`, else "stable".
pub fn served_channel(patch_server: &str) -> String {
    let url = patch_server.trim().trim_end_matches('/');
    let url = url.strip_suffix("/check").unwrap_or(url);
    let mut parts = url.rsplit('/');
    match (parts.next(), parts.next()) {
        (Some(slot), Some("prerelease")) if valid_slot(slot) => format!("pre:{slot}"),
        _ => "stable".to_string(),
    }
}

/// Erupe's slot names: `^[a-z0-9][a-z0-9-]{0,23}$` (server/api/prerelease.go).
pub fn valid_slot(slot: &str) -> bool {
    let b = slot.as_bytes();
    !b.is_empty()
        && b.len() <= 24
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && slot != "slots"
        && slot != "check"
}

/// A channel the API accepts from us: "stable" or "pre:<slot>".
pub fn valid_channel(channel: &str) -> bool {
    channel == "stable" || channel.strip_prefix("pre:").is_some_and(valid_slot)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPatch {
    /// "stable" or "pre:<slot>"
    pub channel: String,
    /// The /check ETag of what was installed.
    pub etag: String,
    /// "v1.30", "v1.31-pre2"; empty until known.
    #[serde(default)]
    pub version: String,
}

fn sanitize_server_key(server: &str) -> String {
    server
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn record_path(root: &Path, server: &str) -> std::path::PathBuf {
    root.join(SERVER_CACHE_DIR)
        .join(sanitize_server_key(server))
        .join(RECORD_FILE)
}

pub fn load_installed(root: &Path, server: &str) -> Option<InstalledPatch> {
    fs::read_to_string(record_path(root, server))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save_installed(root: &Path, server: &str, record: &InstalledPatch) {
    let path = record_path(root, server);
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(path.parent().unwrap_or(root))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
        fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        warn!("could not save {}: {}", path.display(), e);
    }
}

/// Called by the patcher when the files for `etag` from `patch_url` are in
/// place. Keeps a known version name when nothing changed.
pub fn record_installed(root: &Path, server: &str, patch_url: &str, etag: &str) {
    let channel = served_channel(patch_url);
    let etag = etag.trim().to_string();
    if etag.is_empty() {
        return;
    }
    let version = load_installed(root, server)
        .filter(|old| old.channel == channel && old.etag == etag)
        .map(|old| old.version)
        .unwrap_or_default();
    save_installed(root, server, &InstalledPatch { channel, etag, version });
}

/// The version name the API gives for a channel: the slot's version for
/// "pre:<slot>" (when published), `currentPatch` for stable.
pub fn version_for(channel: &str, api: &Value) -> String {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match channel.strip_prefix("pre:") {
        Some(slot) => api["slots"]
            .as_array()
            .and_then(|slots| slots.iter().find(|s| s["slot"].as_str() == Some(slot)))
            .map(|s| text(&s["version"]))
            .unwrap_or_default(),
        None => text(&api["currentPatch"]),
    }
}

// --- the account API (same calls as the website) ---------------------------

#[derive(Debug)]
pub enum ApiError {
    /// 401/403: sign in again.
    Unauthorized,
    /// Anything the player should read (the API's own message when it has one).
    Message(String),
}

impl ApiError {
    pub fn text(&self) -> String {
        match self {
            Self::Unauthorized => "The account server did not accept the sign-in.".into(),
            Self::Message(m) => m.clone(),
        }
    }
}

fn message_of(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .filter(|m| !m.trim().is_empty())
}

pub async fn sign_in(client: &reqwest::Client, api: &str, username: &str, password: &str) -> Result<String, ApiError> {
    let resp = client
        .post(format!("{api}/api/auth/signin"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|_| ApiError::Message("Could not reach the account server. Try again in a moment.".into()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    match status {
        200..=299 => serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["accessToken"].as_str().or(v["token"].as_str()).map(str::to_string))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ApiError::Message("The account server gave no sign-in token.".into())),
        401 | 403 => Err(ApiError::Message(
            "The account server refused this username and password.".into(),
        )),
        429 => Err(ApiError::Message(
            message_of(&body).unwrap_or_else(|| "Too many attempts. Try again in a few minutes.".into()),
        )),
        _ => Err(ApiError::Message(format!("The account server answered {status}."))),
    }
}

pub async fn get_channel(client: &reqwest::Client, api: &str, token: &str) -> Result<Value, ApiError> {
    let resp = client
        .get(format!("{api}/api/account/patch-channel"))
        .bearer_auth(token)
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|_| ApiError::Message("Could not reach the account server. Try again in a moment.".into()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    match status {
        200..=299 => {
            let v: Value = serde_json::from_str(&body)
                .map_err(|_| ApiError::Message("The account server's answer was unreadable.".into()))?;
            if !v["slots"].is_array() {
                return Err(ApiError::Message("The account server does not offer patch choices.".into()));
            }
            Ok(v)
        }
        401 | 403 => Err(ApiError::Unauthorized),
        _ => Err(ApiError::Message(
            message_of(&body).unwrap_or_else(|| format!("The account server answered {status}.")),
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelChange {
    pub ok: bool,
    pub channel: String,
    pub message: String,
    /// The API wants the player to accept the pre-release explanation first
    /// (409); `message` is that explanation.
    pub confirmation_required: bool,
}

pub async fn put_channel(
    client: &reqwest::Client,
    api: &str,
    token: &str,
    channel: &str,
    confirm: bool,
) -> Result<ChannelChange, ApiError> {
    let body = if confirm {
        serde_json::json!({ "channel": channel, "confirm": true })
    } else {
        serde_json::json!({ "channel": channel })
    };
    let resp = client
        .put(format!("{api}/api/account/patch-channel"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|_| ApiError::Message("Could not reach the account server. Try again in a moment.".into()))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let message = v["message"].as_str().unwrap_or_default().to_string();
    match status {
        200..=299 => Ok(ChannelChange {
            ok: true,
            channel: v["channel"].as_str().unwrap_or(channel).to_string(),
            message,
            confirmation_required: false,
        }),
        409 if v["confirmationRequired"].as_bool() == Some(true) => Ok(ChannelChange {
            ok: false,
            channel: channel.to_string(),
            message,
            confirmation_required: true,
        }),
        401 | 403 => Err(ApiError::Unauthorized),
        _ => Err(ApiError::Message(if message.is_empty() {
            format!("The account server answered {status}.")
        } else {
            message
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_from_patch_server() {
        assert_eq!(served_channel("http://129.213.54.221:8080"), "stable");
        assert_eq!(served_channel("http://129.213.54.221:8080/"), "stable");
        assert_eq!(served_channel("http://129.213.54.221:8080/prerelease/mods"), "pre:mods");
        assert_eq!(served_channel("http://129.213.54.221:8080/prerelease/mods/"), "pre:mods");
        assert_eq!(served_channel("http://h:8080/prerelease/balance-2/check"), "pre:balance-2");
        assert_eq!(served_channel("http://h:8080/prerelease/slots"), "stable");
        assert_eq!(served_channel("http://h:8080/prerelease/Mods"), "stable");
        assert_eq!(served_channel(""), "stable");
    }

    #[test]
    fn channels_the_api_takes() {
        assert!(valid_channel("stable"));
        assert!(valid_channel("pre:mods"));
        assert!(!valid_channel("pre:"));
        assert!(!valid_channel("pre:../x"));
        assert!(!valid_channel("pre:check"));
        assert!(!valid_channel("beta"));
        assert!(!valid_channel(&format!("pre:{}", "a".repeat(25))));
    }

    #[test]
    fn hunsterverse_hosts_only() {
        if std::env::var(ACCOUNT_API_ENV).is_ok() {
            return; // overridden in this environment
        }
        assert_eq!(account_api_for("129.213.54.221").as_deref(), Some(HUNSTERVERSE_API));
        assert_eq!(account_api_for("example.org"), None);
    }

    #[test]
    fn versions_from_api_answer() {
        let api = serde_json::json!({
            "currentPatch": "v1.30",
            "slots": [{"slot": "mods", "version": "v1.31-pre2"}, {"slot": "x", "version": null}]
        });
        assert_eq!(version_for("stable", &api), "v1.30");
        assert_eq!(version_for("pre:mods", &api), "v1.31-pre2");
        assert_eq!(version_for("pre:x", &api), "");
        assert_eq!(version_for("pre:gone", &api), "");
    }

    #[test]
    fn record_keeps_version_only_for_same_patch() {
        let root = std::env::temp_dir().join(format!("hv-patch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let server = "129.213.54.221:8080";
        record_installed(&root, server, "http://129.213.54.221:8080", "\"e1\"");
        let mut r = load_installed(&root, server).unwrap();
        assert_eq!((r.channel.as_str(), r.etag.as_str(), r.version.as_str()), ("stable", "\"e1\"", ""));
        r.version = "v1.30".into();
        save_installed(&root, server, &r);
        record_installed(&root, server, "http://129.213.54.221:8080/", "\"e1\"");
        assert_eq!(load_installed(&root, server).unwrap().version, "v1.30");
        record_installed(&root, server, "http://129.213.54.221:8080/prerelease/mods", "\"e2\"");
        let r = load_installed(&root, server).unwrap();
        assert_eq!((r.channel.as_str(), r.version.as_str()), ("pre:mods", ""));
        assert!(root.join("Mezeporta/Servers/129.213.54.221_8080/hv-patch.json").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
