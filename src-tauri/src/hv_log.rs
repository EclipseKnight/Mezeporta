//! Hunsterverse (1.5.4-hv6): the launcher's log files and the problem report.
//!
//! Until hv5 the log went to stdout and the webview only (tauri-plugin-log,
//! which nothing in the frontend listened to), so a player's machine kept
//! nothing. Now every line also goes to `<launcher folder>/Mezeporta/logs/`
//! (`log_dir_candidates`): `mezeporta.log` for this run, `mezeporta.1.log` ..
//! `mezeporta.4.log` for the runs before (a run past 1 MiB also moves on, so
//! the files stay under about 5 MiB together). On Linux the game's own output
//! goes to `game.log` (and `game.1.log` for the start before; on Windows
//! only its start and exit code, hv_crash.rs), and
//! `run-mezeporta.sh` writes `run-mezeporta.log` there too.
//!
//! Every line goes through `redact`: values of password, token, secret, key,
//! cookie and session fields, bearer tokens, JWTs and long base64 blobs are
//! replaced, and the home folder is shown as `~`. The game's start data (with
//! the session token and password) is never logged at all.
//!
//! `build_report` makes the "problem report": system info, the launcher's
//! settings that matter, and the end of each log file, as one text file the
//! player attaches in Discord (Settings, Advanced, Problem report).

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOG_STEM: &str = "mezeporta";
pub const GAME_STEM: &str = "game";
pub const SCRIPT_STEM: &str = "run-mezeporta";
pub const REPORT_PREFIX: &str = "problem-report-";
/// Older runs kept beside the current file (`mezeporta.1.log` .. `.4.log`).
pub const KEEP_RUNS: usize = 4;
/// A run that writes more than this moves on to a new file.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
const GAME_KEEP_RUNS: usize = 1;
const REPORTS_KEPT: usize = 3;
/// Lines logged before the file is open (start-up), written to it once open.
const PENDING_MAX: usize = 2000;
const REDACTED: &str = "[redacted]";

// --- time ----------------------------------------------------------------

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn utc_parts(t: SystemTime) -> (i64, u32, u32, u64, u64, u64, u32) {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (y, m, day) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    (y, m, day, rem / 3600, rem / 60 % 60, rem % 60, d.subsec_millis())
}

/// `2026-11-24 10:15:03.123` (UTC).
pub fn utc_stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, ms) = utc_parts(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{ms:03}")
}

/// `20261124-101503` (UTC), for file names.
pub fn file_stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, _) = utc_parts(t);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

// --- redaction -----------------------------------------------------------

/// Field names whose values never go into a log (matched case-insensitively,
/// also inside longer names: `user_token`, `hv_api_token`, `passwordHash`).
const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "authorization",
    "apikey",
    "api_key",
    "api-key",
    "private_key",
    "privatekey",
    "cookie",
    "session",
    "credential",
];

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// End of a value starting at `start`: the closing quote when quoted, else
/// the first space or separator.
fn value_end(bytes: &[u8], start: usize, quote: Option<u8>) -> usize {
    let mut j = start;
    match quote {
        Some(q) => {
            while j < bytes.len() && bytes[j] != q {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            j.min(bytes.len())
        }
        None => {
            while j < bytes.len()
                && !bytes[j].is_ascii_whitespace()
                && !b",;&)}]\"'<>".contains(&bytes[j])
            {
                j += 1;
            }
            j
        }
    }
}

/// `key=value`, `key: value`, `"key":"value"` with a secret key, and
/// `Bearer <token>`: the value becomes `[redacted]`.
fn redact_key_values(s: &str) -> String {
    let bytes = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let lower = lower.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !is_ident_byte(bytes[i - 1]);
        if boundary && lower[i..].starts_with(b"bearer ") {
            let mut start = i + 7;
            while start < bytes.len() && bytes[start] == b' ' {
                start += 1;
            }
            let end = value_end(bytes, start, None);
            if end > start {
                out.push_str(&s[copied..start]);
                out.push_str(REDACTED);
                copied = end;
                i = end;
                continue;
            }
        }
        if let Some(key) = SECRET_KEYS.iter().find(|k| lower[i..].starts_with(k.as_bytes())) {
            let mut j = i + key.len();
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                j += 1;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                // `Authorization: Bearer <token>`: the scheme stays, the
                // token goes; Debug's `Some("...")`: the quoted value goes.
                for prefix in [&b"bearer "[..], b"basic ", b"some("] {
                    if lower[j..].starts_with(prefix) {
                        j += prefix.len();
                        while j < bytes.len() && bytes[j] == b' ' {
                            j += 1;
                        }
                    }
                }
                let quote = match bytes.get(j) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        j += 1;
                        Some(q)
                    }
                    _ => None,
                };
                let end = value_end(bytes, j, quote);
                if end > j {
                    out.push_str(&s[copied..j]);
                    out.push_str(REDACTED);
                    copied = end;
                    i = end;
                    continue;
                }
            }
            i += key.len();
            continue;
        }
        i += 1;
    }
    out.push_str(&s[copied..]);
    out
}

/// JWTs (`eyJ...`) and runs of 160 or more base64 characters (an encoded
/// launch config or a key) become `[redacted]`.
fn redact_blobs(s: &str) -> String {
    let is_blob_char = |c: char| c.is_ascii_alphanumeric() || "+/=_-.".contains(c);
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let jwt = run.starts_with("eyJ") && run.len() >= 20 && run.matches('.').count() >= 1;
        let blob = run.len() >= 160 && !run.contains('.');
        if jwt || blob {
            out.push_str(REDACTED);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if is_blob_char(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// The user's home folder(s), shown as `~` in logs.
fn home_dirs() -> Vec<String> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|h| h.trim_end_matches(['/', '\\']).to_string())
        .filter(|h| h.len() > 3)
        .collect()
}

fn redact_homes(s: &str, homes: &[String]) -> String {
    let mut out = s.to_string();
    for home in homes {
        out = out.replace(home.as_str(), "~");
        let slashed = home.replace('\\', "/");
        if slashed != *home {
            out = out.replace(slashed.as_str(), "~");
        }
    }
    out
}

/// What every log line and the report go through (module doc).
pub fn redact(s: &str) -> String {
    redact_with_homes(s, &home_dirs())
}

pub fn redact_with_homes(s: &str, homes: &[String]) -> String {
    redact_homes(&redact_blobs(&redact_key_values(s)), homes)
}

// --- files ---------------------------------------------------------------

/// `mezeporta.log` for n = 0, `mezeporta.<n>.log` after.
pub fn file_name(stem: &str, n: usize) -> String {
    if n == 0 {
        format!("{stem}.log")
    } else {
        format!("{stem}.{n}.log")
    }
}

/// Moves `stem.log` to `stem.1.log`, `.1` to `.2` and so on; `stem.<keep>.log`
/// is deleted. Errors are ignored (a file another process holds stays).
pub fn rotate(dir: &Path, stem: &str, keep: usize) {
    let _ = fs::remove_file(dir.join(file_name(stem, keep)));
    for n in (0..keep).rev() {
        let from = dir.join(file_name(stem, n));
        if from.exists() {
            let _ = fs::rename(&from, dir.join(file_name(stem, n + 1)));
        }
    }
}

/// A log file that moves on (`rotate`) when it is opened and when it grows
/// past `max_bytes`.
pub struct RotatingFile {
    dir: PathBuf,
    stem: String,
    keep: usize,
    max_bytes: u64,
    file: Option<File>,
    written: u64,
}

impl RotatingFile {
    pub fn open(dir: &Path, stem: &str, keep: usize, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let mut this = Self {
            dir: dir.to_path_buf(),
            stem: stem.to_string(),
            keep,
            max_bytes,
            file: None,
            written: 0,
        };
        this.next_file()?;
        Ok(this)
    }

    fn next_file(&mut self) -> io::Result<()> {
        self.file = None;
        rotate(&self.dir, &self.stem, self.keep);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(file_name(&self.stem, 0)))?;
        self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        if self.written >= self.max_bytes {
            // The rename failed (a file held open elsewhere): go on in this
            // file and try again after another max_bytes, not at every line.
            self.written = 0;
        }
        self.file = Some(file);
        Ok(())
    }

    pub fn write_line(&mut self, line: &str) {
        if self.written >= self.max_bytes && self.next_file().is_err() {
            return;
        }
        if let Some(file) = self.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.written += line.len() as u64;
            }
        }
    }
}

/// Is `path` inside one of Windows' Program Files folders? There UAC file
/// virtualization sends our writes to `%LOCALAPPDATA%\VirtualStore`, where
/// "Open log folder" would not find them (hv5 README).
pub fn under_any(path: &Path, roots: &[PathBuf]) -> bool {
    let p = path.to_string_lossy().to_lowercase().replace('/', "\\");
    roots.iter().any(|r| {
        let r = r.to_string_lossy().to_lowercase().replace('/', "\\");
        let r = r.trim_end_matches('\\');
        !r.is_empty() && (p == r || p.starts_with(&format!("{r}\\")))
    })
}

/// Where the logs go, best first: `<launcher folder>/Mezeporta/logs` (beside
/// `config.json`; the folder the Steam Deck's sandbox can write), then the
/// user's own app data folder.
pub fn log_dir_candidates(launcher_folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if !(cfg!(windows) && under_any(launcher_folder, &program_files)) {
        out.push(launcher_folder.join("Mezeporta").join("logs"));
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Mezeporta").join("logs"));
        }
    } else if let Some(state) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(state).join("Mezeporta").join("logs"));
    } else if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(home).join(".local/state/Mezeporta/logs"));
    }
    out.push(std::env::temp_dir().join("Mezeporta-logs"));
    out
}

// --- the logger ----------------------------------------------------------

struct Inner {
    file: Option<RotatingFile>,
    quiet_stdout: bool,
    dir: Option<PathBuf>,
    pending: Vec<String>,
}

struct HvLogger {
    inner: Mutex<Inner>,
}

static LOGGER: HvLogger = HvLogger {
    inner: Mutex::new(Inner {
        file: None,
        quiet_stdout: false,
        dir: None,
        pending: Vec::new(),
    }),
};

/// Our own modules log from debug up, libraries from warn up.
fn level_for(target: &str) -> LevelFilter {
    if target == "app" || target.starts_with("app::") || target.starts_with("meze_butter") {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    }
}

pub fn format_line(t: SystemTime, level: Level, target: &str, message: &str) -> String {
    let target = target.strip_prefix("app::").unwrap_or(target);
    format!("{} UTC {:<5} [{}] {}\n", utc_stamp(t), level, target, redact(message))
}

impl Log for HvLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= level_for(metadata.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            SystemTime::now(),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        // As before (tauri-plugin-log's Stdout target), unless
        // run-mezeporta.sh sends stdout to its own log file and ours is open
        // (MEZEPORTA_LOG_TO_STDOUT=0). A GUI process on Windows has no
        // console, so errors are ignored.
        if inner.file.is_none() || !inner.quiet_stdout {
            let _ = io::stdout().lock().write_all(line.as_bytes());
        }
        if let Some(file) = inner.file.as_mut() {
            file.write_line(&line);
        } else if inner.pending.len() < PENDING_MAX {
            inner.pending.push(line);
        }
    }

    fn flush(&self) {}
}

/// Installs the logger (stdout at once; the file from `attach_file`) and a
/// panic hook that logs the panic. Call first thing in `main`.
pub fn init() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Debug);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}");
            previous(info);
        }));
    }
}

/// Opens this run's log file in the first usable folder of
/// `log_dir_candidates` and writes what was logged before. Returns the folder.
pub fn attach_file(launcher_folder: &Path) -> Option<PathBuf> {
    let mut inner = match LOGGER.inner.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if inner.dir.is_some() {
        return inner.dir.clone();
    }
    inner.quiet_stdout = std::env::var_os("MEZEPORTA_LOG_TO_STDOUT").is_some_and(|v| v == "0");
    for dir in log_dir_candidates(launcher_folder) {
        if let Ok(mut file) = RotatingFile::open(&dir, LOG_STEM, KEEP_RUNS, MAX_FILE_BYTES) {
            for line in inner.pending.drain(..) {
                file.write_line(&line);
            }
            inner.file = Some(file);
            inner.dir = Some(dir.clone());
            return Some(dir);
        }
    }
    None
}

pub fn log_dir() -> Option<PathBuf> {
    match LOGGER.inner.lock() {
        Ok(g) => g.dir.clone(),
        Err(poisoned) => poisoned.into_inner().dir.clone(),
    }
}

/// A fresh `game.log` (the one before as `game.1.log`), starting with
/// `header`: on Linux the game's stdout/stderr go on into it; on Windows
/// (game inside the launcher) its end line (hv_crash.rs). None when there is
/// no log folder.
pub fn game_output_file(header: &str) -> Option<File> {
    let dir = log_dir()?;
    rotate(&dir, GAME_STEM, GAME_KEEP_RUNS);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file_name(GAME_STEM, 0)))
        .ok()?;
    let _ = file.write_all(
        format!("== {} UTC {}\n", utc_stamp(SystemTime::now()), redact(header)).as_bytes(),
    );
    Some(file)
}

// --- system info ---------------------------------------------------------

#[cfg(not(windows))]
fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `KEY=value` lines of an os-release file, quotes removed.
#[cfg(not(windows))]
fn os_release_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(not(windows))]
fn os_release_summary(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = vec![os_release_field(&text, "PRETTY_NAME")
        .or_else(|| os_release_field(&text, "NAME"))
        .unwrap_or_else(|| "?".into())];
    for key in ["ID", "VERSION_ID", "BUILD_ID", "VARIANT_ID"] {
        if let Some(v) = os_release_field(&text, key) {
            parts.push(format!("{}={}", key.to_lowercase(), v));
        }
    }
    Some(parts.join(", "))
}

/// Finds `program` on PATH (or as given when it has a folder part).
#[cfg(not(windows))]
pub fn find_program(program: &Path) -> Option<PathBuf> {
    if program.is_absolute() || program.components().count() > 1 {
        return program.exists().then(|| program.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|c| c.is_file())
}

/// First line of `program args` (stdout, else stderr), or None after 5 s.
#[cfg(not(windows))]
fn program_output(program: &Path, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(5) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Some("(no answer in 5 s)".into());
            }
        }
    }
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut text);
    }
    if text.trim().is_empty() {
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut text);
        }
    }
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// Environment variables that matter for starting the game (values go
/// through `redact`; nothing else of the environment is read).
const ENV_NAMES: &[&str] = &[
    "MEZEPORTA_PROTON_CMD",
    "PROTON_CMD",
    "MEZEPORTA_WINE_CMD",
    "MHF_WINE_CMD",
    "WINEPREFIX",
    "WINEARCH",
    "WINEESYNC",
    "WINEFSYNC",
    "WINEDEBUG",
    "WINEDLLOVERRIDES",
    "SteamDeck",
    "SteamGamepadUI",
    "SteamAppId",
    "SteamGameId",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "GAMESCOPE_WAYLAND_DISPLAY",
    "GDK_BACKEND",
    "WEBKIT_DMABUF_RENDERER_FORCE_SHM",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "HV_HOST_DISPLAY",
    "FLATPAK_ID",
    "container",
    "LANG",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_ARCHITEW6432",
];
const ENV_PREFIXES: &[&str] = &["PROTON_", "STEAM_COMPAT_", "DXVK_", "VKD3D_"];

pub fn environment() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .filter(|(k, _)| {
            ENV_NAMES.contains(&k.as_str()) || ENV_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .collect();
    out.sort();
    if std::env::var_os("LD_PRELOAD").is_some() {
        out.push(("LD_PRELOAD".into(), "(set)".into()));
    }
    out
}

/// Proton's own version file (`<proton folder>/version`, "<time> proton-9.0-3"),
/// also through the installer's `proton-host.sh` (its `proton=` line).
#[cfg(not(windows))]
fn proton_version(command: &Path) -> String {
    let mut proton = command.to_path_buf();
    if let Ok(text) = fs::read_to_string(command) {
        if let Some(line) = text.lines().find_map(|l| l.strip_prefix("proton=")) {
            let path = line.trim().trim_matches('\'').trim_matches('"').replace("\\ ", " ");
            proton = PathBuf::from(path);
        }
    }
    let version = proton
        .parent()
        .and_then(|d| read_trimmed(d.join("version")))
        .unwrap_or_else(|| "version file not readable from here".into());
    format!("{} ({})", proton.display(), version)
}

/// What the report and the start of each log say about this machine;
/// `wine_version`: also run `wine --version` (may take a moment).
pub fn system_info(wine_version: bool) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str, v: String| out.push((k.to_string(), v));
    add(
        "Launcher",
        format!(
            "{} ({}-{})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    if let Ok(exe) = std::env::current_exe() {
        add("Launcher file", exe.display().to_string());
    }
    if let Ok(cwd) = std::env::current_dir() {
        add("Working folder", cwd.display().to_string());
    }
    if let Some(dir) = log_dir() {
        add("Log folder", dir.display().to_string());
    }
    #[cfg(not(windows))]
    {
        let flatpak = Path::new("/.flatpak-info").exists();
        if flatpak {
            add(
                "OS (host)",
                os_release_summary("/run/host/os-release")
                    .or_else(|| os_release_summary("/run/host/usr/lib/os-release"))
                    .unwrap_or_else(|| "not readable".into()),
            );
            add(
                "Flatpak sandbox",
                fs::read_to_string("/.flatpak-info")
                    .ok()
                    .and_then(|t| os_release_field(&t, "runtime"))
                    .unwrap_or_else(|| "yes".into()),
            );
            add(
                "Sandbox runtime OS",
                os_release_summary("/etc/os-release").unwrap_or_else(|| "?".into()),
            );
        } else {
            add(
                "OS",
                os_release_summary("/etc/os-release")
                    .or_else(|| os_release_summary("/usr/lib/os-release"))
                    .unwrap_or_else(|| "?".into()),
            );
        }
        add(
            "Kernel",
            read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "?".into()),
        );
        let vendor = read_trimmed("/sys/devices/virtual/dmi/id/sys_vendor").unwrap_or_default();
        let product = read_trimmed("/sys/devices/virtual/dmi/id/product_name").unwrap_or_default();
        let deck = vendor == "Valve" && (product == "Jupiter" || product == "Galileo");
        add(
            "Device",
            format!(
                "{} {}{}",
                vendor,
                product,
                if deck { " (Steam Deck)" } else { "" }
            )
            .trim()
            .to_string(),
        );
        let wine_cmd = std::env::var("MEZEPORTA_WINE_CMD")
            .ok()
            .or_else(|| std::env::var("MHF_WINE_CMD").ok())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "wine".into());
        match find_program(Path::new(&wine_cmd)) {
            Some(wine) if wine_version => {
                let version = program_output(&wine, &["--version"]).unwrap_or_else(|| "?".into());
                add("Wine", format!("{} ({})", wine.display(), version));
            }
            Some(wine) => add("Wine", wine.display().to_string()),
            None => add("Wine", format!("{wine_cmd} not found")),
        }
        for var in ["MEZEPORTA_PROTON_CMD", "PROTON_CMD"] {
            if let Some(cmd) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                let cmd = PathBuf::from(cmd);
                let found = if cmd.exists() { "" } else { " (file not found)" };
                add(
                    &format!("Proton ({var})"),
                    format!("{}{}", proton_version(&cmd), found),
                );
            }
        }
        if let Some(data) = std::env::var_os("STEAM_COMPAT_DATA_PATH") {
            let pfx = PathBuf::from(data).join("pfx");
            add(
                "Proton prefix",
                format!(
                    "{} ({})",
                    pfx.display(),
                    if pfx.join("system.reg").is_file() {
                        "exists"
                    } else {
                        "not created yet, or not visible from here"
                    }
                ),
            );
        }
    }
    #[cfg(windows)]
    {
        let _ = wine_version;
        add("OS", win::os_version());
        if let Some(wine) = win::wine_version() {
            add("Running under Wine/Proton", wine);
        }
        if let Some(folder) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            add(
                "Launcher folder writable",
                crate::hv_update::folder_is_writable(&folder).to_string(),
            );
        }
    }
    for (k, v) in environment() {
        out.push((format!("env {k}"), v));
    }
    out
}

#[cfg(windows)]
mod win {
    use windows::core::{s, w, PCWSTR};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};

    const KEY: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");

    fn reg_sz(name: PCWSTR) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                KEY,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut _),
                Some(&mut size),
            )
            .ok()?;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    fn reg_dword(name: PCWSTR) -> Option<u32> {
        let mut value = 0u32;
        let mut size = 4u32;
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                KEY,
                name,
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut _),
                Some(&mut size),
            )
            .ok()?;
        }
        Some(value)
    }

    /// "Windows 10 Pro 22H2, build 19045.5131" (Windows 11 also calls itself
    /// Windows 10 here; build 22000 and up is 11).
    pub fn os_version() -> String {
        let name = reg_sz(w!("ProductName")).unwrap_or_else(|| "Windows".into());
        let display = reg_sz(w!("DisplayVersion")).or_else(|| reg_sz(w!("ReleaseId")));
        let build = reg_sz(w!("CurrentBuildNumber")).unwrap_or_else(|| "?".into());
        let ubr = reg_dword(w!("UBR")).map(|u| format!(".{u}")).unwrap_or_default();
        match display {
            Some(d) => format!("{name} {d}, build {build}{ubr}"),
            None => format!("{name}, build {build}{ubr}"),
        }
    }

    /// Wine's version when the Windows launcher runs under Wine or Proton.
    pub fn wine_version() -> Option<String> {
        unsafe {
            let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
            let f = GetProcAddress(ntdll, s!("wine_get_version"))?;
            let f: extern "C" fn() -> *const std::ffi::c_char = std::mem::transmute(f);
            let p = f();
            if p.is_null() {
                return Some("yes".into());
            }
            Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }
}

/// Logs `system_info` (in a thread: `wine --version` may take a moment).
/// Logs `system_info` at start: at once (a launcher that dies next, e.g.
/// GTK with no display, still leaves it), and Wine's version from a thread.
pub fn log_system_info() {
    for (k, v) in system_info(false) {
        log::info!("system: {k}: {v}");
    }
    #[cfg(not(windows))]
    std::thread::spawn(|| {
        if let Some((_, v)) = system_info(true).into_iter().find(|(k, _)| k == "Wine") {
            log::info!("system: Wine version: {v}");
        }
    });
}

// --- the report ----------------------------------------------------------

/// The last `max_lines` lines of a file, read from at most its last 2 MiB.
pub fn tail_lines(path: &Path, max_lines: usize) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(2 * 1024 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(max_lines);
    let mut out = String::new();
    if from > 0 || start > 0 {
        out.push_str("[... earlier lines left out ...]\n");
    }
    for line in &lines[from..] {
        if line.len() > 2000 {
            let mut cut = 2000;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push_str(&line[..cut]);
            out.push_str(" [...]\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Some(out)
}

/// The log files in the report, newest first, with how many lines of each.
const REPORT_FILES: &[(&str, usize, usize)] = &[
    (LOG_STEM, 0, 1500),
    (LOG_STEM, 1, 1500),
    (LOG_STEM, 2, 500),
    (GAME_STEM, 0, 400),
    (GAME_STEM, 1, 200),
    (SCRIPT_STEM, 0, 300),
    (SCRIPT_STEM, 1, 150),
];

/// The problem report's text: `system`, then `settings`, then `blocks`
/// (title, text: the last game start, game files, crash files; hv_crash.rs),
/// then the end of each log file in `dir`. Everything goes through `redact`.
pub fn build_report(
    dir: Option<&Path>,
    now: SystemTime,
    system: &[(String, String)],
    settings: &[(String, String)],
    blocks: &[(String, String)],
) -> String {
    let mut out = String::new();
    out.push_str("Mezeporta launcher problem report (Hunsterverse)\n");
    out.push_str(&format!("Made: {} UTC\n", utc_stamp(now)));
    out.push_str(
        "Passwords, session tokens and keys are never in it; your home folder is shown as ~.\n",
    );
    for (title, items) in [("System", system), ("Launcher settings", settings)] {
        out.push_str(&format!("\n== {title}\n"));
        for (k, v) in items {
            out.push_str(&format!("{k}: {v}\n"));
        }
    }
    for (title, text) in blocks {
        out.push_str(&format!("\n== {title}\n{text}"));
        if !text.ends_with('\n') {
            out.push('\n');
        }
    }
    match dir {
        None => out.push_str("\n== Logs\nno log folder could be written\n"),
        Some(dir) => {
            for &(stem, n, lines) in REPORT_FILES {
                let name = file_name(stem, n);
                if let Some(text) = tail_lines(&dir.join(&name), lines) {
                    out.push_str(&format!("\n== {name} (last {lines} lines)\n"));
                    out.push_str(&text);
                }
            }
        }
    }
    redact(&out)
}

/// Writes `text` as `problem-report-<time>.txt` in `dir` (keeping the newest
/// three reports) and returns its path.
pub fn save_report(dir: &Path, now: SystemTime, text: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{REPORT_PREFIX}{}.txt", file_stamp(now)));
    fs::write(&path, text)?;
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(REPORT_PREFIX) && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    let excess = reports.len().saturating_sub(REPORTS_KEPT);
    for old in reports.into_iter().take(excess) {
        let _ = fs::remove_file(old);
    }
    Ok(path)
}

/// Opens the log folder in the file manager (Windows: with the report
/// selected). Best effort: the UI shows the path as well.
pub fn open_folder(dir: &Path, select: Option<&Path>) -> Result<(), String> {
    #[cfg(windows)]
    let result = match select {
        Some(file) => std::process::Command::new("explorer")
            .arg(format!("/select,{}", file.display()))
            .spawn(),
        None => std::process::Command::new("explorer").arg(dir).spawn(),
    };
    #[cfg(not(windows))]
    let result = {
        let _ = select;
        std::process::Command::new("xdg-open").arg(dir).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hv-log-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hv_log_redacts_passwords_tokens_and_keys() {
        let homes: Vec<String> = vec![];
        let cases = [
            ("login password=hunter2 ok", "login password=[redacted] ok"),
            (r#"{"username":"bob","password":"a b\"c","x":1}"#, r#"{"username":"bob","password":"[redacted]","x":1}"#),
            ("user_token: abc123def, rights: 14", "user_token: [redacted], rights: 14"),
            ("GET /api?token=s3cr3t&x=1", "GET /api?token=[redacted]&x=1"),
            ("Authorization: Bearer abc.def.ghi", "Authorization: Bearer [redacted]"),
            ("authorization=basic dXNlcjpwdw== next", "authorization=basic [redacted] next"),
            ("header bearer xyz", "header bearer [redacted]"),
            ("TAURI_KEY_PASSWORD = 'pw pw'", "TAURI_KEY_PASSWORD = '[redacted]'"),
            ("hv_api_token: Some(\"t\")", "hv_api_token: Some(\"[redacted]\")"),
            ("private_key: AAAA", "private_key: [redacted]"),
            ("Cookie: JSESSIONID=1234", "Cookie: [redacted]"),
            ("secretWord=x", "secretWord=[redacted]"),
        ];
        for (input, want) in cases {
            let got = redact_with_homes(input, &homes);
            assert_eq!(got, want, "input: {input}");
        }
        // Words without a value stay.
        for keep in [
            "Log in first.",
            "the session ended",
            "token expired, signing in again",
            "patch 2a1f... installed for 129.213.54.221:8080",
        ] {
            assert_eq!(redact_with_homes(keep, &homes), keep);
        }
    }

    #[test]
    fn hv_log_redacts_jwts_blobs_and_home() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJib2IifQ.c2lnbmF0dXJl";
        let got = redact_with_homes(&format!("signed in, {jwt} ok"), &[]);
        assert_eq!(got, "signed in, [redacted] ok");

        let blob = "QUJD".repeat(60); // 240 base64 characters
        let got = redact_with_homes(&format!("payload {blob} end"), &[]);
        assert_eq!(got, "payload [redacted] end");
        // A sha256 is not a blob.
        let sha = "a".repeat(64);
        assert_eq!(redact_with_homes(&sha, &[]), sha);

        let homes = vec!["/home/deck".to_string(), "C:\\Users\\Alice".to_string()];
        assert_eq!(
            redact_with_homes("/home/deck/Games/MHFZ and C:\\Users\\Alice\\MHFZ and C:/Users/Alice/x", &homes),
            "~/Games/MHFZ and ~\\MHFZ and ~/x"
        );
    }

    #[test]
    fn hv_log_line_format_and_time() {
        // 2026-11-24 10:15:03.123 UTC
        let t = UNIX_EPOCH + Duration::from_millis(1_795_515_303_123);
        assert_eq!(utc_stamp(t), "2026-11-24 10:15:03.123");
        assert_eq!(file_stamp(t), "20261124-101503");
        assert_eq!(utc_stamp(UNIX_EPOCH), "1970-01-01 00:00:00.000");
        assert_eq!(utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00:00.000");
        let line = format_line(t, Level::Warn, "app::hv_update", "login password=x");
        assert_eq!(line, "2026-11-24 10:15:03.123 UTC WARN  [hv_update] login password=[redacted]\n");
    }

    #[test]
    fn hv_log_rotates_runs_and_size() {
        let dir = temp_dir("rotate");
        // Six runs: only the current one and four before are kept.
        for run in 1..=6 {
            let mut f = RotatingFile::open(&dir, "mezeporta", KEEP_RUNS, MAX_FILE_BYTES).unwrap();
            f.write_line(&format!("run {run}\n"));
        }
        let read = |n| fs::read_to_string(dir.join(file_name("mezeporta", n))).unwrap();
        assert_eq!(read(0), "run 6\n");
        assert_eq!(read(1), "run 5\n");
        assert_eq!(read(4), "run 2\n");
        assert!(!dir.join("mezeporta.5.log").exists());
        let _ = fs::remove_dir_all(&dir);

        // Past the size limit a run moves on; the total stays bounded.
        let dir = temp_dir("size");
        let mut f = RotatingFile::open(&dir, "mezeporta", 4, 1000).unwrap();
        let line = format!("{}\n", "x".repeat(99));
        for _ in 0..200 {
            f.write_line(&line);
        }
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 5, "{files:?}");
        let total: u64 = files.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
        assert!(total <= 5 * 1000, "total {total}");
        for p in &files {
            assert!(fs::metadata(p).unwrap().len() <= 1000);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hv_log_report_has_logs_and_no_secrets() {
        let dir = temp_dir("report");
        fs::write(dir.join("mezeporta.log"), "line a\nlogin password=hunter2\n").unwrap();
        fs::write(dir.join("mezeporta.1.log"), "previous run\n").unwrap();
        fs::write(dir.join("game.log"), "err:module:import_dll\n").unwrap();
        let system = vec![("OS".to_string(), "SteamOS".to_string())];
        let settings = vec![("Token".to_string(), "x".to_string()), ("Wine prefix mode".to_string(), "proton".to_string())];
        let now = UNIX_EPOCH + Duration::from_secs(1_795_515_303);
        let blocks = vec![("Last game start".to_string(), "Result: the game ended after 3 s".to_string())];
        let text = build_report(Some(&dir), now, &system, &settings, &blocks);
        assert!(text.contains("== System\nOS: SteamOS\n"));
        assert!(text.contains("Wine prefix mode: proton"));
        assert!(text.contains("\n== Last game start\nResult: the game ended after 3 s\n\n== mezeporta.log"));
        assert!(text.contains("== mezeporta.log (last 1500 lines)\nline a\n"));
        assert!(text.contains("previous run"));
        assert!(text.contains("err:module:import_dll"));
        assert!(!text.contains("hunter2"));
        assert!(text.contains("Token: [redacted]"));

        for i in 0..5 {
            save_report(&dir, now + Duration::from_secs(i), &text).unwrap();
        }
        let reports = fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(REPORT_PREFIX))
            .count();
        assert_eq!(reports, REPORTS_KEPT);
        assert!(dir.join("problem-report-20261124-101507.txt").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hv_log_tail_and_folders() {
        let dir = temp_dir("tail");
        let path = dir.join("x.log");
        let text: String = (1..=10).map(|i| format!("{i}\n")).collect();
        fs::write(&path, text).unwrap();
        assert_eq!(tail_lines(&path, 3).unwrap(), "[... earlier lines left out ...]\n8\n9\n10\n");
        assert_eq!(tail_lines(&path, 20).unwrap().lines().count(), 10);
        assert!(tail_lines(&dir.join("missing.log"), 3).is_none());

        let roots = vec![PathBuf::from("C:\\Program Files (x86)")];
        assert!(under_any(Path::new("C:\\Program Files (x86)\\Hunsterverse"), &roots));
        assert!(under_any(Path::new("c:/program files (x86)/Hunsterverse"), &roots));
        assert!(!under_any(Path::new("C:\\Games\\MHFZ"), &roots));
        assert!(!under_any(Path::new("C:\\Program Files (x86)2"), &roots));

        let first = &log_dir_candidates(Path::new("/games/mhfz"))[0];
        assert_eq!(first, &Path::new("/games/mhfz").join("Mezeporta").join("logs"));
        let _ = fs::remove_dir_all(dir);
    }
}
