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
//!
//! Hunsterverse (1.5.4-hv5): a launcher in a folder the player cannot change
//! (Program Files, or files an administrator created) is updated through a
//! short elevated step on Windows (the UAC prompt): see `install` and
//! `run_apply_update_helper`. On Linux the player gets a clear message.

use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
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

/// Why replace_executable failed. `needs_admin`: Windows answered "access
/// denied" (or Linux "permission denied"), which administrator rights may fix.
#[derive(Debug)]
pub struct SwapError {
    pub message: String,
    pub needs_admin: bool,
    /// The file is in use (a sharing violation), or access was denied: what is
    /// left when even an elevated swap fails (antivirus, another launcher).
    pub denied_or_in_use: bool,
}

impl SwapError {
    fn from_io(what: &str, exe: &Path, e: io::Error) -> Self {
        // Windows: 32 ERROR_SHARING_VIOLATION, 33 ERROR_LOCK_VIOLATION.
        let in_use = cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(33));
        let denied = e.kind() == io::ErrorKind::PermissionDenied;
        SwapError {
            message: format!("{what} ({}): {e}", exe.display()),
            needs_admin: denied,
            denied_or_in_use: denied || in_use,
        }
    }
}

/// Puts `data` in place of the executable `exe`, keeping the old one as
/// `<exe>.old` until the next start. Either the new binary is in place or the
/// old one is (never neither, never half a file): the new file is written and
/// flushed beside the old one first, then renamed.
pub fn replace_executable(exe: &Path, data: &[u8]) -> Result<(), SwapError> {
    let new = sibling(exe, ".new");
    let old = sibling(exe, ".old");
    let fail = |what: &str, e: io::Error| SwapError::from_io(what, exe, e);

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

// --- Hunsterverse (1.5.4-hv5): updates that need administrator rights ------
//
// A player reported "access is denied" when the update moved Mezeporta.exe
// aside; "Run as administrator" fixed it. The launcher was in a folder the
// player cannot change (under Program Files, or its files were created by an
// elevated process). Our Windows build has no requestedExecutionLevel in its
// manifest, so as a 32-bit program it runs with UAC file virtualization:
// writing a new file there "works" (Windows redirects it to VirtualStore),
// but renaming the real Mezeporta.exe is denied. So both the write test and a
// denied rename lead to the elevated step.
//
// The elevated step is the installed launcher itself (`Mezeporta.exe
// --hv-apply-update <staged file> <signature>`), not the downloaded file: the
// installed one sits in a folder the player cannot write, the staged copy in
// the player's temp folder could be swapped by any program of the player. The
// step reads the staged file once, checks those bytes against the public key
// built into it, writes them beside itself and swaps; it never moves the
// staged file. It replaces only its own file (current_exe), whatever it is
// given. Then it exits and the normal (not elevated) launcher starts the new
// version as before, so neither the launcher nor the game runs elevated.

/// First argument of the elevated helper mode (main.rs checks it first).
pub const APPLY_UPDATE_ARG: &str = "--hv-apply-update";

/// Exit codes of the helper mode.
pub const HELPER_SWAPPED: i32 = 0;
pub const HELPER_FAILED: i32 = 1;
pub const HELPER_BAD_ARGS: i32 = 2;
/// The staged file is not a signed launcher for this system.
pub const HELPER_REFUSED: i32 = 3;
/// Still "access denied" or "file in use" with administrator rights
/// (5 like ERROR_ACCESS_DENIED).
pub const HELPER_DENIED: i32 = 5;

/// How long the launcher waits for the elevated step once it was allowed.
#[cfg(windows)]
const HELPER_TIMEOUT_MS: u32 = 5 * 60 * 1000;

/// What came of the elevated step (only Windows has one).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub enum Elevation {
    Swapped,
    /// The player answered No to the UAC prompt.
    Declined,
    /// Elevated, and still access denied or the file in use.
    DeniedWhenElevated,
    /// The elevated step refused the file (signature or not an executable).
    Refused,
    Failed(String),
}

/// Can this process create (and delete) a file in `folder`? Under UAC file
/// virtualization this says yes for Program Files (see above); the denied
/// rename that follows then leads to the elevated step.
pub fn folder_is_writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".mezeporta-write-test-{}", std::process::id()));
    let created = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe);
    match created {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(e) => {
            info!("launcher folder {} is not writable: {}", folder.display(), e);
            false
        }
    }
}

fn not_writable_message(folder: &Path) -> String {
    format!(
        "The update cannot write to the launcher's folder {}. Make that folder writable for your user \
         (or move the game folder into your home folder), then try again. The launcher was not changed.",
        folder.display()
    )
}

fn elevation_result(outcome: Elevation, exe: &Path, folder: &Path) -> Result<(), String> {
    match outcome {
        Elevation::Swapped => Ok(()),
        Elevation::Declined => Err(format!(
            "The update needs administrator rights because the launcher is in {}. Click \"Update and restart\" \
             again and allow the Windows prompt, or move the game folder somewhere you own, e.g. C:\\Games. \
             The launcher was not changed.",
            folder.display()
        )),
        Elevation::DeniedWhenElevated => Err(format!(
            "Windows refused to replace {} even with administrator rights. Close every other Mezeporta window \
             and check that your antivirus is not blocking or scanning Mezeporta.exe, then try again. \
             The launcher was not changed.",
            exe.display()
        )),
        Elevation::Refused => Err(
            "The administrator step refused the downloaded file (its signature did not match). \
             The launcher was not changed."
                .to_string(),
        ),
        Elevation::Failed(why) => Err(format!("The update could not be installed: {why}")),
    }
}

/// The decision, with the file system and the UAC step passed in (tests).
/// `folder_writable`: folder_is_writable of the launcher's folder.
/// `elevate`: None where there is no UAC (Linux, the Steam Deck).
pub fn install_with<S, E>(
    exe: &Path,
    data: &[u8],
    signature: &str,
    folder_writable: bool,
    swap_in_place: S,
    elevate: Option<E>,
) -> Result<(), String>
where
    S: FnOnce(&Path, &[u8]) -> Result<(), SwapError>,
    E: FnOnce(&Path, &str) -> Elevation,
{
    let folder = exe.parent().unwrap_or(Path::new("."));
    if folder_writable {
        match swap_in_place(exe, data) {
            Ok(()) => return Ok(()),
            Err(e) if !e.needs_admin => return Err(e.message),
            Err(e) => warn!("launcher update: {}; needs administrator rights", e.message),
        }
    }
    let Some(elevate) = elevate else {
        return Err(not_writable_message(folder));
    };
    // Staged in the player's temp folder: the launcher's folder may not be
    // writable. The elevated step re-checks what it reads from here.
    static STAGING_RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = STAGING_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = std::env::temp_dir().join(format!("Mezeporta-update-{}-{run}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let staged = staging.join(sibling(exe, ".new").file_name().unwrap_or_default());
    let written = fs::create_dir_all(&staging).and_then(|_| fs::write(&staged, data));
    if let Err(e) = written {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!(
            "The update could not be saved in the temp folder ({}): {e}. The launcher was not changed.",
            staging.display()
        ));
    }
    info!("launcher update: asking for administrator rights to replace {}", exe.display());
    let outcome = elevate(&staged, signature);
    let _ = fs::remove_dir_all(&staging);
    info!("launcher update: administrator step: {:?}", outcome);
    elevation_result(outcome, exe, folder)
}

/// Installs verified update bytes in place of `exe`: directly when the folder
/// allows it, else (Windows) through the elevated step. `hwnd`: the launcher
/// window, so the UAC prompt comes to the front.
pub fn install(exe: &Path, data: &[u8], signature: &str, hwnd: Option<isize>) -> Result<(), String> {
    let folder = exe.parent().unwrap_or(Path::new("."));
    let writable = folder_is_writable(folder);
    #[cfg(windows)]
    let elevate = Some(|staged: &Path, sig: &str| run_elevated_helper(exe, staged, sig, hwnd));
    #[cfg(not(windows))]
    let elevate = {
        let _ = hwnd;
        None::<fn(&Path, &str) -> Elevation>
    };
    install_with(exe, data, signature, writable, replace_executable, elevate)
}

/// The helper's command line after the program name. The signature (base64,
/// no spaces) and a temp path (no quotes on Windows) need no other escaping.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn helper_parameters(staged: &Path, signature: &str) -> Option<String> {
    let signature = signature.trim();
    let base64_only = !signature.is_empty()
        && signature
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=');
    let path = staged.to_str()?;
    if !base64_only || path.contains('"') || path.ends_with('\\') {
        return None;
    }
    Some(format!("{APPLY_UPDATE_ARG} \"{path}\" {signature}"))
}

/// Windows: starts this launcher elevated in helper mode (the UAC prompt) and
/// waits for it.
#[cfg(windows)]
fn run_elevated_helper(exe: &Path, staged: &Path, signature: &str, hwnd: Option<isize>) -> Elevation {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HWND, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let Some(parameters) = helper_parameters(staged, signature) else {
        return Elevation::Failed("the update's temp file or signature cannot be passed on".into());
    };
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe);
    let parameters = HSTRING::from(parameters);
    let directory = HSTRING::from(exe.parent().unwrap_or(Path::new(".")));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: HWND(hwnd.unwrap_or(0)),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        // The helper is a console program with no window of its own.
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Elevation::Declined;
        }
        return Elevation::Failed(format!("the administrator step could not start: {e}"));
    }
    // Copied out: the struct is packed on 32-bit Windows (no references to it).
    let process = info.hProcess;
    if process.is_invalid() {
        return Elevation::Failed("the administrator step could not start".into());
    }
    let waited = unsafe { WaitForSingleObject(process, HELPER_TIMEOUT_MS) };
    let mut code = 0u32;
    let got_code = unsafe { GetExitCodeProcess(process, &mut code) }.is_ok();
    unsafe {
        let _ = CloseHandle(process);
    }
    if waited != WAIT_OBJECT_0 || !got_code {
        return Elevation::Failed(
            "the administrator step did not finish in 5 minutes. Close the launcher and start it again".into(),
        );
    }
    match code as i32 {
        HELPER_SWAPPED => Elevation::Swapped,
        HELPER_REFUSED => Elevation::Refused,
        HELPER_DENIED => Elevation::DeniedWhenElevated,
        other => Elevation::Failed(format!(
            "the administrator step ended with code {other}. The launcher was not changed"
        )),
    }
}

/// Helper mode: when this process was started as `<exe> --hv-apply-update
/// <staged file> <signature>`, does the swap and returns the exit code; None
/// for a normal start. Runs before anything else in main (no window, no
/// single-instance lock).
pub fn run_apply_update_helper() -> Option<i32> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().map(|a| a.as_os_str()) != Some(std::ffi::OsStr::new(APPLY_UPDATE_ARG)) {
        return None;
    }
    let (Some(staged), Some(signature), None) = (args.get(1), args.get(2).and_then(|s| s.to_str()), args.get(3))
    else {
        return Some(HELPER_BAD_ARGS);
    };
    let Ok(exe) = std::env::current_exe() else {
        return Some(HELPER_FAILED);
    };
    Some(apply_staged_update(&exe, Path::new(staged), signature, PUBLIC_KEY))
}

/// The helper's work: read the staged file once, check those bytes, swap.
pub fn apply_staged_update(exe: &Path, staged: &Path, signature: &str, public_key_b64: &str) -> i32 {
    let mut data = Vec::new();
    let read = fs::File::open(staged)
        .and_then(|f| f.take(MAX_BINARY_BYTES + 1).read_to_end(&mut data));
    if read.is_err() || data.len() as u64 > MAX_BINARY_BYTES {
        return HELPER_FAILED;
    }
    if verify_signature(&data, signature, public_key_b64).is_err() || !looks_like_executable(&data) {
        return HELPER_REFUSED;
    }
    match replace_executable(exe, &data) {
        Ok(()) => HELPER_SWAPPED,
        Err(e) if e.denied_or_in_use => HELPER_DENIED,
        Err(_) => HELPER_FAILED,
    }
}

/// Downloads, verifies and installs the update for this platform. Returns the
/// installed version. Does not restart.
pub async fn download_and_install(
    client: &reqwest::Client,
    manifest: &ReleaseManifest,
    exe: &Path,
    public_key_b64: &str,
    hwnd: Option<isize>,
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
    // Blocking (the UAC prompt and the elevated step can take a while).
    let exe_owned = exe.to_path_buf();
    let signature = asset.signature.clone();
    tauri::async_runtime::spawn_blocking(move || install(&exe_owned, &data, &signature, hwnd))
        .await
        .map_err(|e| format!("the update was interrupted: {e}"))??;
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

    // --- hv5: updates that need administrator rights ---------------------

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hv-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn denied() -> SwapError {
        SwapError::from_io(
            "cannot move the running launcher aside",
            Path::new("Mezeporta.exe"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        )
    }

    type Elevate = fn(&Path, &str) -> Elevation;

    #[test]
    fn writable_folder_swaps_in_place_without_asking() {
        let dir = scratch("inplace");
        assert!(folder_is_writable(&dir));
        assert!(!folder_is_writable(&dir.join("missing")));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "the write test leaves nothing behind");
        let exe = dir.join("mezeporta-bin");
        fs::write(&exe, b"old").unwrap();
        let elevate: Option<Elevate> = Some(|_, _| panic!("no UAC prompt for a writable folder"));
        install_with(&exe, b"new", "sig", true, replace_executable, elevate).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn denied_rename_asks_for_admin_with_the_verified_bytes() {
        // The Windows case of the report: the folder looks writable (UAC file
        // virtualization), the rename of Mezeporta.exe is denied.
        let exe = Path::new("C:/Program Files/Hunsterverse/Mezeporta.exe");
        let mut seen = None;
        let result = install_with(
            exe,
            b"signed bytes",
            "c2lnbmF0dXJl",
            true,
            |_, _| Err(denied()),
            Some(|staged: &Path, sig: &str| {
                seen = Some((staged.to_path_buf(), fs::read(staged).unwrap(), sig.to_string()));
                Elevation::Swapped
            }),
        );
        assert_eq!(result, Ok(()));
        let (staged, bytes, sig) = seen.expect("the elevated step ran");
        assert_eq!(bytes, b"signed bytes");
        assert_eq!(sig, "c2lnbmF0dXJl");
        assert!(staged.starts_with(std::env::temp_dir()));
        assert!(!staged.exists(), "the staged copy is removed afterwards");
    }

    #[test]
    fn not_writable_folder_goes_straight_to_admin() {
        let exe = Path::new("C:/Program Files/Hunsterverse/Mezeporta.exe");
        let result = install_with(
            exe,
            b"x",
            "c2ln",
            false,
            |_, _| panic!("no in-place swap in a folder that is not writable"),
            Some(|_: &Path, _: &str| Elevation::Swapped),
        );
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn declined_prompt_says_why_and_how() {
        let exe = Path::new("C:/Program Files/Hunsterverse/Mezeporta.exe");
        let err = install_with(exe, b"x", "c2ln", false, |_, _| Ok(()), Some(|_: &Path, _: &str| Elevation::Declined))
            .unwrap_err();
        assert!(err.contains("needs administrator rights because the launcher is in C:/Program Files/Hunsterverse"), "{err}");
        assert!(err.contains("C:\\Games") && err.contains("not changed"), "{err}");
    }

    #[test]
    fn denied_when_elevated_names_the_file() {
        let exe = Path::new("C:/Games/MHFZ/Mezeporta.exe");
        let err = install_with(exe, b"x", "c2ln", true, |_, _| Err(denied()), Some(|_: &Path, _: &str| {
            Elevation::DeniedWhenElevated
        }))
        .unwrap_err();
        assert!(err.contains("C:/Games/MHFZ/Mezeporta.exe"), "{err}");
        assert!(err.contains("other Mezeporta window") && err.contains("antivirus"), "{err}");
        assert!(err.contains("not changed"), "{err}");
    }

    #[test]
    fn linux_has_no_prompt_only_a_clear_message() {
        let exe = Path::new("/opt/mhfz/mezeporta-bin");
        let err = install_with(exe, b"x", "c2ln", false, |_, _| Ok(()), None::<Elevate>).unwrap_err();
        assert!(err.contains("cannot write to the launcher's folder /opt/mhfz"), "{err}");
        let err = install_with(exe, b"x", "c2ln", true, |_, _| Err(denied()), None::<Elevate>).unwrap_err();
        assert!(err.contains("/opt/mhfz") && err.contains("not changed"), "{err}");
    }

    #[test]
    fn other_swap_errors_do_not_prompt() {
        let exe = Path::new("C:/Games/MHFZ/Mezeporta.exe");
        let elevate: Option<Elevate> = Some(|_, _| panic!("a full disk is not fixed by admin rights"));
        let err = install_with(exe, b"x", "c2ln", true, |_, _| {
            Err(SwapError { message: "disk full".into(), needs_admin: false, denied_or_in_use: false })
        }, elevate)
        .unwrap_err();
        assert_eq!(err, "disk full");
    }

    #[test]
    fn helper_parameters_quote_the_path_and_refuse_odd_input() {
        let p = helper_parameters(Path::new("C:/Users/A B/Temp/x/Mezeporta.exe.new"), " YWJj+/= \n").unwrap();
        assert_eq!(p, "--hv-apply-update \"C:/Users/A B/Temp/x/Mezeporta.exe.new\" YWJj+/=");
        assert!(helper_parameters(Path::new("a\"b"), "YWJj").is_none());
        assert!(helper_parameters(Path::new("a"), "YW Jj").is_none());
        assert!(helper_parameters(Path::new("a"), "").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn elevated_step_checks_the_bytes_itself() {
        const KEY2: &str = include_str!("../tests/hv_update/test-key-2.pub");
        const ELF: &[u8] = include_bytes!("../tests/hv_update/elf-payload.bin");
        const ELF_SIG: &str = include_str!("../tests/hv_update/elf-payload.bin.sig");
        let dir = scratch("helper");
        let exe = dir.join("mezeporta-bin");
        let staged = dir.join("staged.new");
        fs::write(&exe, b"old binary").unwrap();

        // Changed after the launcher checked it, or signed with another key.
        let mut changed = ELF.to_vec();
        changed[10] ^= 1;
        fs::write(&staged, &changed).unwrap();
        assert_eq!(apply_staged_update(&exe, &staged, ELF_SIG, KEY2), HELPER_REFUSED);
        fs::write(&staged, ELF).unwrap();
        assert_eq!(apply_staged_update(&exe, &staged, ELF_SIG, PUBLIC_KEY), HELPER_REFUSED);
        // Signed, but not an executable for this system.
        fs::write(&staged, PAYLOAD).unwrap();
        assert_eq!(apply_staged_update(&exe, &staged, TEST_SIGNATURE, TEST_PUBLIC_KEY), HELPER_REFUSED);
        assert_eq!(apply_staged_update(&exe, &dir.join("missing"), ELF_SIG, KEY2), HELPER_FAILED);
        assert_eq!(fs::read(&exe).unwrap(), b"old binary", "nothing replaced so far");

        fs::write(&staged, ELF).unwrap();
        assert_eq!(apply_staged_update(&exe, &staged, ELF_SIG, KEY2), HELPER_SWAPPED);
        assert_eq!(fs::read(&exe).unwrap(), ELF);
        assert_eq!(fs::read(sibling(&exe, ".old")).unwrap(), b"old binary");
        assert!(staged.exists(), "the staged file is read, never moved");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn swap_errors_know_access_denied() {
        let e = denied();
        assert!(e.needs_admin && e.denied_or_in_use);
        let e = SwapError::from_io("x", Path::new("a"), io::Error::from(io::ErrorKind::NotFound));
        assert!(!e.needs_admin && !e.denied_or_in_use);
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
