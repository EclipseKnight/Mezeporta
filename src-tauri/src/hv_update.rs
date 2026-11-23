//! Hunsterverse (1.5.4-hv2): the launcher updates itself from the fork's GitHub
//! releases (github.com/EclipseKnight/Mezeporta).
//!
//! Why our own updater and not Tauri's: Tauri 1's updater only replaces an
//! AppImage on Linux and runs an MSI/NSIS installer on Windows, but players run
//! the launcher as a plain binary inside their game folder (Mezeporta.exe or
//! mezeporta-bin next to dat/, from the release zips and our installers). So
//! this module replaces that one binary in place. It uses the same signature
//! format as Tauri (minisign, keys from `tauri signer generate`), so the
//! releases could move to Tauri's updater later without new keys.
//!
//! Release layout (Docs/HUNSTERVERSE.md, scripts/hv-release.sh):
//! - `latest.json` on the latest (non pre-release) GitHub release:
//!   `{"version": "1.5.4-hv2", "notes": "...", "pub_date": "...",
//!     "platforms": {"windows-i686": {"url": "...", "signature": "..."},
//!                   "linux-x86_64": {"url": "...", "signature": "..."}}}`
//! - each binary as its own asset; `signature` is the content of the `.sig`
//!   file `tauri signer sign` writes (base64 of a minisign signature).
//!
//! Only the launcher binary is replaced. The other files of the release zips
//! (audio, DLLs, fonts) are not touched; a release that changes them needs the
//! installer (or the zip) again, and must say so in its notes.

use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

/// Where the launcher looks for updates. GitHub redirects
/// `releases/latest/download/<asset>` to the asset of the newest release that is
/// not a draft or pre-release, so a pre-release on GitHub never reaches players.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/EclipseKnight/Mezeporta/releases/latest/download/latest.json";

/// Test or staging override of the manifest URL. The signature check still
/// applies, so this cannot install anything we did not sign.
pub const MANIFEST_URL_ENV: &str = "MEZEPORTA_UPDATE_URL";

/// Public half of the release signing key (`tauri signer generate`; the private
/// key and its password live only on the build machine, see
/// Docs/HUNSTERVERSE.md). Base64 of a minisign public key file.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFGOEVBQzVDRTVGRTQzNzEKUldSeFEvN2xYS3lPcjlrb3d2WFVyTFd6Z1BpNUhHOVpqMC9LVlMwTVBkNS9hcXNKcTg3SzF0QVUK";

/// The version this binary reports (Cargo.toml, e.g. "1.5.4-hv2").
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Set by the old process on Windows so the new one waits for it to exit (the
/// single-instance mutex is per executable path).
pub const WAIT_FOR_PID_ENV: &str = "MEZEPORTA_UPDATED_FROM_PID";

const MAX_BINARY_BYTES: u64 = 400 * 1024 * 1024;
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformAsset {
    pub url: String,
    pub signature: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: String,
    #[serde(default)]
    pub platforms: std::collections::HashMap<String, PlatformAsset>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub available: bool,
    pub notes: String,
    pub pub_date: String,
}

/// The manifest key of this build: "windows-i686", "linux-x86_64", ...
pub fn platform_key() -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86") {
        "i686"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    format!("{os}-{arch}")
}

/// "1.5.4-hv2" -> [1, 5, 4, 2]; "1.5.4" -> [1, 5, 4, 0]. Our builds are
/// upstream's version plus "-hvN"; semver would order "hv10" before "hv2".
pub fn parse_version(text: &str) -> Option<[u64; 4]> {
    let text = text.trim().trim_start_matches('v');
    let (base, hv) = match text.split_once('-') {
        Some((base, suffix)) => {
            let n = suffix.strip_prefix("hv")?.parse::<u64>().ok()?;
            (base, n)
        }
        None => (text, 0),
    };
    let mut parts = base.split('.').map(|p| p.parse::<u64>().ok());
    let v = [parts.next()??, parts.next()??, parts.next()??, hv];
    if parts.next().is_some() {
        return None;
    }
    Some(v)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn manifest_url() -> String {
    std::env::var(MANIFEST_URL_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MANIFEST_URL.to_string())
}

pub async fn fetch_manifest(client: &reqwest::Client) -> Result<ReleaseManifest, String> {
    let resp = client
        .get(manifest_url())
        .timeout(MANIFEST_TIMEOUT)
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| format!("update check failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("update check failed: HTTP {}", resp.status().as_u16()));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("update check failed: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("update manifest unreadable: {e}"))
}

pub fn update_info(manifest: &ReleaseManifest) -> UpdateInfo {
    let available = is_newer(&manifest.version, CURRENT_VERSION)
        && manifest.platforms.contains_key(&platform_key());
    UpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        latest_version: manifest.version.trim().to_string(),
        available,
        notes: manifest.notes.clone(),
        pub_date: manifest.pub_date.clone(),
    }
}

/// Checks `data` against a `.sig` file's content (base64 of a minisign
/// signature) with a public key in the same encoding as PUBLIC_KEY.
pub fn verify_signature(data: &[u8], signature_b64: &str, public_key_b64: &str) -> Result<(), String> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let decode = |what: &str, value: &str| -> Result<String, String> {
        let bytes = engine
            .decode(value.trim())
            .map_err(|_| format!("{what} is not base64"))?;
        String::from_utf8(bytes).map_err(|_| format!("{what} is not text"))
    };
    let key = minisign_verify::PublicKey::decode(&decode("public key", public_key_b64)?)
        .map_err(|e| format!("bad public key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&decode("signature", signature_b64)?)
        .map_err(|e| format!("bad signature: {e}"))?;
    key.verify(data, &sig, true)
        .map_err(|_| "the signature does not match: update refused".to_string())
}

/// The file starts like an executable for this platform (a wrong asset, such as
/// an HTML error page or the other platform's binary, is refused before it
/// replaces anything; the signature already proves it is ours).
pub fn looks_like_executable(data: &[u8]) -> bool {
    if cfg!(windows) {
        data.starts_with(b"MZ")
    } else {
        data.starts_with(b"\x7fELF")
    }
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let req = client.get(url).timeout(DOWNLOAD_TIMEOUT).send();
    let mut resp = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, req)
        .await
        .map_err(|_| "download timed out".to_string())?
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status().as_u16()));
    }
    let mut data = Vec::new();
    loop {
        let chunk = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, resp.chunk())
            .await
            .map_err(|_| "download stalled".to_string())?
            .map_err(|e| format!("download failed: {e}"))?;
        let Some(chunk) = chunk else { break };
        data.extend_from_slice(&chunk);
        if data.len() as u64 > MAX_BINARY_BYTES {
            return Err("download too large".into());
        }
    }
    Ok(data)
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Puts `data` in place of the executable `exe`, keeping the old one as
/// `<exe>.old` until the next start. Either the new binary is in place or the
/// old one is (never neither, never half a file): the new file is written and
/// flushed beside the old one first, then renamed.
pub fn replace_executable(exe: &Path, data: &[u8]) -> Result<(), String> {
    let new = sibling(exe, ".new");
    let old = sibling(exe, ".old");
    let fail = |what: &str, e: std::io::Error| format!("{what} ({}): {e}", exe.display());

    {
        let mut f = fs::File::create(&new).map_err(|e| fail("cannot write next to the launcher", e))?;
        f.write_all(data).map_err(|e| fail("cannot write the update", e))?;
        f.sync_all().map_err(|e| fail("cannot write the update", e))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(exe).map(|m| m.permissions().mode()).unwrap_or(0o755) | 0o111;
        fs::set_permissions(&new, fs::Permissions::from_mode(mode))
            .map_err(|e| fail("cannot make the update executable", e))?;
    }
    if old.exists() {
        // Left from the previous update; on Windows it may still be locked by
        // an old process, which only matters if it fails here.
        fs::remove_file(&old).map_err(|e| {
            let _ = fs::remove_file(&new);
            fail("cannot remove the previous backup", e)
        })?;
    }

    #[cfg(unix)]
    {
        // Keep the old binary as .old (hard link, so the running one keeps its
        // name until the atomic rename below).
        if fs::hard_link(exe, &old).is_err() {
            let _ = fs::copy(exe, &old);
        }
        if let Err(e) = fs::rename(&new, exe) {
            let _ = fs::remove_file(&new);
            return Err(fail("cannot replace the launcher", e));
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: a running .exe cannot be overwritten or deleted, but it can
        // be renamed. Rename it away, then put the new one under its name.
        fs::rename(exe, &old).map_err(|e| {
            let _ = fs::remove_file(&new);
            fail("cannot move the running launcher aside", e)
        })?;
        if let Err(e) = fs::rename(&new, exe) {
            let _ = fs::rename(&old, exe);
            let _ = fs::remove_file(&new);
            return Err(fail("cannot put the update in place", e));
        }
    }
    Ok(())
}

/// Downloads, verifies and installs the update for this platform. Returns the
/// installed version. Does not restart.
pub async fn download_and_install(
    client: &reqwest::Client,
    manifest: &ReleaseManifest,
    exe: &Path,
    public_key_b64: &str,
) -> Result<String, String> {
    if !is_newer(&manifest.version, CURRENT_VERSION) {
        return Err(format!("{} is not newer than {}", manifest.version, CURRENT_VERSION));
    }
    let key = platform_key();
    let asset = manifest
        .platforms
        .get(&key)
        .ok_or_else(|| format!("this release has no {key} build"))?;
    info!("launcher update: downloading {} for {}", manifest.version, key);
    let data = download(client, &asset.url).await?;
    verify_signature(&data, &asset.signature, public_key_b64)?;
    if !looks_like_executable(&data) {
        return Err("the downloaded file is not a launcher for this system".into());
    }
    replace_executable(exe, &data)?;
    info!("launcher update: {} installed at {}", manifest.version, exe.display());
    Ok(manifest.version.trim().to_string())
}

/// Removes what an update leaves behind (`<exe>.old`, an interrupted
/// `<exe>.new`). Called at start-up; errors are only logged.
pub fn cleanup_after_update(exe: &Path) {
    for suffix in [".old", ".new"] {
        let path = sibling(exe, suffix);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                warn!("could not remove {}: {}", path.display(), e);
            }
        }
    }
}

/// Starts the updated launcher with the same arguments and ends this process.
/// Linux: exec in place, so the process keeps its PID (on the Steam Deck the
/// launcher is the main process of a Flatpak sandbox, which ends with it).
/// Windows: start the new process, which waits for this one to exit
/// (WAIT_FOR_PID_ENV) before it takes the single-instance mutex.
pub fn restart(exe: &Path) -> Result<(), String> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(exe).args(&args).exec();
        Err(format!("could not restart the launcher: {err}"))
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(exe)
            .args(&args)
            .env(WAIT_FOR_PID_ENV, std::process::id().to_string())
            .spawn()
            .map_err(|e| format!("could not restart the launcher: {e}"))?;
        std::process::exit(0);
    }
}

/// Windows: when started by `restart`, wait (up to 20 s) for the old process
/// to exit before the single-instance check.
#[cfg(windows)]
pub fn wait_for_previous_process() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    let Some(pid) = std::env::var(WAIT_FOR_PID_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
    else {
        return;
    };
    std::env::remove_var(WAIT_FOR_PID_ENV);
    if let Ok(handle) = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        unsafe {
            WaitForSingleObject(handle, 20_000);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
pub fn wait_for_previous_process() {
    std::env::remove_var(WAIT_FOR_PID_ENV);
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key pair made for these tests only (tauri signer generate,
    // no password; the private half was deleted) and its signature of PAYLOAD.
    const TEST_PUBLIC_KEY: &str = include_str!("../tests/hv_update/test-key.pub");
    const TEST_SIGNATURE: &str = include_str!("../tests/hv_update/payload.bin.sig");
    const PAYLOAD: &[u8] = include_bytes!("../tests/hv_update/payload.bin");

    #[test]
    fn versions_order_hv_builds_numerically() {
        assert_eq!(parse_version("1.5.4-hv2"), Some([1, 5, 4, 2]));
        assert_eq!(parse_version("v1.5.4"), Some([1, 5, 4, 0]));
        assert_eq!(parse_version("1.5.4-beta"), None);
        assert_eq!(parse_version("1.5"), None);
        assert!(is_newer("1.5.4-hv10", "1.5.4-hv2"));
        assert!(is_newer("1.5.5", "1.5.4-hv9"));
        assert!(is_newer("1.5.4-hv1", "1.5.4"));
        assert!(!is_newer("1.5.4-hv2", "1.5.4-hv2"));
        assert!(!is_newer("1.5.4-hv1", "1.5.4-hv2"));
        assert!(!is_newer("garbage", "1.5.4-hv2"));
    }

    #[test]
    fn current_version_parses() {
        assert!(parse_version(CURRENT_VERSION).is_some(), "Cargo.toml version {CURRENT_VERSION}");
    }

    #[test]
    fn update_available_only_for_newer_with_our_platform() {
        let mut m = ReleaseManifest {
            version: "99.0.0-hv1".into(),
            notes: String::new(),
            pub_date: String::new(),
            platforms: Default::default(),
        };
        assert!(!update_info(&m).available, "no build for this platform");
        m.platforms.insert(
            platform_key(),
            PlatformAsset { url: "http://x/y".into(), signature: String::new() },
        );
        assert!(update_info(&m).available);
        m.version = CURRENT_VERSION.into();
        assert!(!update_info(&m).available);
    }

    #[test]
    fn signature_checks() {
        verify_signature(PAYLOAD, TEST_SIGNATURE, TEST_PUBLIC_KEY).expect("good signature");
        let mut changed = PAYLOAD.to_vec();
        changed[0] ^= 1;
        assert!(verify_signature(&changed, TEST_SIGNATURE, TEST_PUBLIC_KEY).is_err());
        // Signed with the test key, so the release key refuses it.
        assert!(verify_signature(PAYLOAD, TEST_SIGNATURE, PUBLIC_KEY).is_err());
        assert!(verify_signature(PAYLOAD, "not base64!", TEST_PUBLIC_KEY).is_err());
    }

    #[test]
    fn release_key_decodes() {
        use base64::Engine;
        let text = String::from_utf8(
            base64::engine::general_purpose::STANDARD.decode(PUBLIC_KEY).unwrap(),
        )
        .unwrap();
        minisign_verify::PublicKey::decode(&text).expect("PUBLIC_KEY is a minisign public key");
    }

    #[test]
    fn replace_keeps_old_and_cleanup_removes_it() {
        let dir = std::env::temp_dir().join(format!("hv-update-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mezeporta-bin");
        fs::write(&exe, b"old binary").unwrap();
        replace_executable(&exe, b"new binary").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new binary");
        assert_eq!(fs::read(sibling(&exe, ".old")).unwrap(), b"old binary");
        assert!(!sibling(&exe, ".new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0);
        }
        // A second update replaces the earlier backup.
        replace_executable(&exe, b"newer binary").unwrap();
        assert_eq!(fs::read(sibling(&exe, ".old")).unwrap(), b"new binary");
        cleanup_after_update(&exe);
        assert!(!sibling(&exe, ".old").exists());
        assert_eq!(fs::read(&exe).unwrap(), b"newer binary");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_failure_leaves_old_binary() {
        let dir = std::env::temp_dir().join(format!("hv-update-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mezeporta-bin");
        fs::write(&exe, b"old binary").unwrap();
        // A directory where the backup goes: removing it as a file fails.
        fs::create_dir_all(sibling(&exe, ".old").join("x")).unwrap();
        assert!(replace_executable(&exe, b"new binary").is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"old binary");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn executable_magic() {
        if cfg!(windows) {
            assert!(looks_like_executable(b"MZ\x90\x00"));
        } else {
            assert!(looks_like_executable(b"\x7fELF\x02"));
        }
        assert!(!looks_like_executable(b"<!DOCTYPE html>"));
    }
}
