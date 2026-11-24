//! Hunsterverse (1.5.4-hv6): the game that closes or crashes right after it
//! starts ("instant crash on launch").
//!
//! - Every game start writes `game.log` in the log folder (hv_log.rs): a
//!   header line `== <time> UTC game start: ...`, the game's output (Linux /
//!   Steam Deck: Wine or Proton and meze-deps.exe), then one end line:
//!   `== <time> UTC game process ended, exit code N, after S s`. On Linux
//!   `/bin/sh` around the game command writes it (the launcher has closed by
//!   then); on Windows the game runs inside the launcher process, so a small
//!   watcher (this launcher started again with `--hv-game-watch`) waits for
//!   that process and writes the end line with the full exit code, also when
//!   the game crashes the process or ends it with ExitProcess. When the game
//!   returns to the launcher, or meze-butter cannot start it, the launcher
//!   writes the end line itself (`game returned ...`, `game did not start:`).
//! - At the next start the launcher reads the last game start (`last_exit`);
//!   when it ended within `EARLY_EXIT_SECS`, crashed, or did not start
//!   (`verdict`), it logs that and shows the player a short message offering
//!   "Save problem report" (HunsterversePanel.vue). Once the player has seen
//!   it, `== <time> UTC launcher: noted` is appended so it shows once.
//! - `describe_code` turns an exit code into words: Windows NTSTATUS /
//!   exception codes; on Linux the shell's status (signals, 126/127) and,
//!   since Wine passes on only the last byte of a Windows code, the codes
//!   that byte may stand for.
//! - The problem report also gets: the last game start (`summary_lines`,
//!   with Wine's error lines from game.log), the game files' sizes, times and
//!   sha256 (`game_files`), hvmods' own crash files in the game folder
//!   (`hvmods_files`; hvmhfz_re mods/hvmods: `hvcrash-<UTC>.txt`,
//!   `hvdiag.txt`, `hvmods.log`, `hvsend-crash.txt`) and on Windows the
//!   Windows Error Reporting entries and crash dumps of the launcher / game
//!   (`wer_reports`).

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::hv_log;

/// A game that ends within this many seconds of its start ended "early".
pub const EARLY_EXIT_SECS: u64 = 60;
/// The game asked to go back to the launcher (main.rs RETURN_TO_LAUNCHER_EXIT_CODE).
pub const RETURN_TO_LAUNCHER: i64 = 102;
/// What a debug game start sets (Linux, Wine or Proton): exceptions and
/// every DLL load, with times and threads, into game.log. Proton keeps a
/// WINEDEBUG that is set (it only defaults to -all). Proton's own log
/// (PROTON_LOG) only where the Proton command passes on what it needs
/// (hv_proton::proton_log_passes).
#[cfg_attr(windows, allow(dead_code))]
pub const DEBUG_WINEDEBUG: &str = "+timestamp,+pid,+tid,+seh,+loaddll";

/// The player asked for a debug launch (the crash message or Settings): the
/// next game start only. `MEZEPORTA_GAME_DEBUG=1` makes every start one.
static DEBUG_NEXT: AtomicBool = AtomicBool::new(false);

pub fn set_debug_launch(on: bool) {
    DEBUG_NEXT.store(on, Ordering::SeqCst);
}

pub fn debug_launch_set() -> bool {
    DEBUG_NEXT.load(Ordering::SeqCst)
}

#[cfg_attr(windows, allow(dead_code))]
pub fn take_debug_launch() -> bool {
    DEBUG_NEXT.swap(false, Ordering::SeqCst)
        || std::env::var_os("MEZEPORTA_GAME_DEBUG").is_some_and(|v| v == "1")
}

/// Linux: `sh -c <this> mezeporta-game <command...>` runs the game command (`$@`) and appends its exit
/// code to game.log (its stdout), which the launcher, closed by then, cannot.
#[cfg(not(windows))]
pub const GAME_EXIT_CODE_SCRIPT: &str = r#"s=$(date +%s 2>/dev/null)
"$@"
code=$?
e=$(date +%s 2>/dev/null)
after='?'
[ -n "$s" ] && [ -n "$e" ] && after=$((e - s))
printf '\n== %s UTC game process ended, exit code %s, after %s s\n' "$(date -u '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$code" "$after"
exit "$code""#;

pub const START_MARK: &str = " game start: ";
const END_MARKS: &[&str] = &[" game process ended, exit code ", " game returned to the launcher, exit code "];
const NOT_STARTED_MARK: &str = " game did not start: ";
const NOTED_MARK: &str = " launcher: noted";

// --- exit codes ----------------------------------------------------------

/// Windows NTSTATUS / exception codes a crashing game ends with.
const NTSTATUS: &[(u32, &str)] = &[
    (0xC000_0005, "access violation (the game read or wrote memory it may not: a broken or mismatched game file or patch, or an overlay or antivirus hooking into the game)"),
    (0xC000_0006, "in-page error (a game file could not be read from the disk)"),
    (0xC000_0017, "out of memory"),
    (0xC000_001D, "illegal instruction"),
    (0xC000_0022, "access denied"),
    (0xC000_008C, "array bounds exceeded"),
    (0xC000_008E, "float divide by zero"),
    (0xC000_0090, "float invalid operation"),
    (0xC000_0094, "integer divide by zero"),
    (0xC000_0095, "integer overflow"),
    (0xC000_0096, "privileged instruction"),
    (0xC000_009A, "insufficient system resources"),
    (0xC000_00FD, "stack overflow"),
    (0xC000_007B, "bad image (a DLL or exe is damaged, or 64-bit where the 32-bit game needs 32-bit)"),
    (0xC000_0135, "DLL not found (a DLL the game needs is missing, e.g. a DirectX 9 or Visual C++ runtime DLL)"),
    (0xC000_0139, "entry point not found (a DLL of the wrong version)"),
    (0xC000_0142, "DLL initialization failed"),
    (0xC000_013A, "closed with Ctrl+C or by Windows (console closed, logoff)"),
    (0xC000_0374, "heap corruption"),
    (0xC000_0409, "stack buffer overrun or fail-fast (the program stopped itself after a security check)"),
    (0xC000_0417, "invalid C runtime parameter"),
    (0xC06D_007E, "delay-loaded DLL not found"),
    (0xC06D_007F, "delay-loaded function not found"),
    (0x8000_0003, "breakpoint"),
    (0x4001_0004, "ended by a debugger or the task manager"),
    (0xE06D_7363, "unhandled C++ exception"),
    (0xCFFF_FFFF, "ended by Windows (the program stopped responding)"),
];

/// meze-deps.exe's own exit codes (Linux helper, src-tauri/meze-butter/src/bin).
const MEZE_DEPS: &[(i64, &str)] = &[
    (1, "meze-deps.exe: started without --stdin-b64"),
    (2, "meze-deps.exe: could not read its start data"),
    (3, "meze-deps.exe: start data not base64"),
    (4, "meze-deps.exe: start data not text"),
    (5, "meze-deps.exe: start data missing or not readable"),
    (6, "meze-deps.exe: could not start the game (its error is in game.log)"),
];

const SIGNALS: &[(i64, &str)] = &[
    (1, "SIGHUP, hang-up (the terminal or session closed)"),
    (2, "SIGINT, interrupted (Ctrl+C)"),
    (3, "SIGQUIT"),
    (4, "SIGILL, illegal instruction"),
    (5, "SIGTRAP"),
    (6, "SIGABRT, aborted (Wine or Proton stopped itself)"),
    (7, "SIGBUS, bus error (a file or memory went away)"),
    (8, "SIGFPE, arithmetic error"),
    (9, "SIGKILL, killed (out of memory, or ended by the system or the user)"),
    (11, "SIGSEGV, segmentation fault (Wine or Proton itself crashed)"),
    (13, "SIGPIPE"),
    (15, "SIGTERM, asked to end (by the system, Steam or the user)"),
];
const CRASH_SIGNALS: &[i64] = &[4, 5, 6, 7, 8, 9, 11];

pub fn ntstatus_name(code: u32) -> Option<&'static str> {
    NTSTATUS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// An exit code in words. `windows_process`: the code is a Windows process's
/// own (the launcher's watcher), else the Linux shell's status of Wine /
/// Proton (0-255).
pub fn describe_code(code: i64, windows_process: bool) -> String {
    if code == 0 {
        return "0 (normal exit)".into();
    }
    if code == RETURN_TO_LAUNCHER {
        return "102 (the game asked to go back to the launcher)".into();
    }
    if windows_process {
        let c = code as u32;
        return match ntstatus_name(c) {
            Some(name) => format!("0x{c:08X} {name}"),
            None if c >= 0xC000_0000 => format!("0x{c:08X} (a Windows error status)"),
            None if c > 0xFFFF => format!("0x{c:08X}"),
            None => format!("{c}"),
        };
    }
    match code {
        126 => return "126 (the game command could not be run: not executable?)".into(),
        127 => return "127 (the game command was not found)".into(),
        129..=192 => {
            let sig = code - 128;
            let name = SIGNALS
                .iter()
                .find(|(s, _)| *s == sig)
                .map(|(_, n)| (*n).to_string())
                .unwrap_or_else(|| format!("signal {sig}"));
            return format!("{code} (ended by {name})");
        }
        _ => {}
    }
    let mut maybe: Vec<String> = MEZE_DEPS
        .iter()
        .filter(|(c, _)| *c == code)
        .map(|(_, n)| (*n).to_string())
        .collect();
    for (c, name) in NTSTATUS {
        if *c >= 0xC000_0000 && i64::from(*c & 0xFF) == code {
            let short = name.split(" (").next().unwrap_or(name);
            maybe.push(format!("0x{c:08X} {short} (Wine passes on only a Windows code's last byte)"));
        }
    }
    if maybe.is_empty() {
        format!("{code}")
    } else {
        format!("{code} (maybe: {})", maybe.join("; or "))
    }
}

// --- the last game start ---------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameExit {
    /// The header line's time (`2026-11-24 10:15:03.123`).
    pub started: String,
    pub ended: String,
    pub code: Option<i64>,
    pub secs: Option<u64>,
    /// `game did not start: <why>`.
    pub not_started: Option<String>,
    /// The code is a Windows process's (watcher or launcher), not Wine's.
    pub windows_process: bool,
    /// The launcher has shown the player its message already.
    pub noted: bool,
    /// Wine's / Proton's / meze-deps' error lines from the game's output.
    pub errors: Vec<String>,
    pub debug: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Ended normally (or still running: no end line yet).
    Normal,
    /// Ended within EARLY_EXIT_SECS.
    Early,
    /// A crash code (Windows exception, Linux crash signal), at any time.
    Crash,
    /// meze-butter could not start it.
    NotStarted,
}

/// The time at the start of an `== <time> UTC ...` line.
fn line_time(line: &str) -> String {
    line.trim_start_matches("== ")
        .split(" UTC")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn leading_number(s: &str) -> Option<i64> {
    let digits: String = s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn after_secs(line: &str) -> Option<u64> {
    let rest = &line[line.rfind(", after ")? + 8..];
    leading_number(rest).and_then(|n| u64::try_from(n).ok())
}

/// Wine, Proton and meze-deps lines worth showing first.
fn is_error_line(line: &str) -> bool {
    const MARKS: &[&str] = &[
        "Unhandled exception",
        "Unhandled page fault",
        "Unhandled illegal",
        "Unhandled stack overflow",
        "err:module:",
        "err:seh:",
        "err:virtual:",
        "err:ntdll:",
        "wine: could not",
        "wine: cannot",
        "Bad EXE format",
        "starting debugger",
        "error running meze-deps",
        "error parsing config",
        "error reading stdin",
        "error decoding base64",
        "Permission denied",
        "No such file or directory",
        "Proton: ",
        "ERROR: ",
    ];
    MARKS.iter().any(|m| line.contains(m))
}

/// The last game start in game.log's `text`, or None when it has none.
#[cfg(test)]
pub fn parse_game_log(text: &str) -> Option<GameExit> {
    parse_lines(text.lines().map(str::to_string))
}

fn parse_lines(lines: impl Iterator<Item = String>) -> Option<GameExit> {
    let mut current: Option<GameExit> = None;
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.starts_with("== ") && line.contains(START_MARK) {
            current = Some(GameExit {
                started: line_time(line),
                debug: line.contains("debug launch"),
                ..GameExit::default()
            });
            continue;
        }
        let Some(exit) = current.as_mut() else {
            continue;
        };
        if line.starts_with("== ") {
            if exit.ended.is_empty() {
                if let Some(i) = line.find(NOT_STARTED_MARK) {
                    exit.ended = line_time(line);
                    let why = &line[i + NOT_STARTED_MARK.len()..];
                    exit.not_started = Some(why.split(", after ").next().unwrap_or(why).to_string());
                    exit.secs = after_secs(line);
                    exit.windows_process = true;
                    continue;
                }
                if let Some(mark) = END_MARKS.iter().find(|m| line.contains(*m)) {
                    let i = line.find(mark).unwrap_or(0) + mark.len();
                    exit.ended = line_time(line);
                    exit.code = leading_number(&line[i..]);
                    exit.secs = after_secs(line);
                    exit.windows_process = line[i..].contains("(0x") || mark.contains("returned");
                    continue;
                }
            }
            if line.contains(NOTED_MARK) {
                exit.noted = true;
            }
            continue;
        }
        if exit.ended.is_empty() && exit.errors.len() < 20 && is_error_line(line) {
            let mut l = line.trim().to_string();
            if l.len() > 300 {
                let mut cut = 300;
                while !l.is_char_boundary(cut) {
                    cut -= 1;
                }
                l.truncate(cut);
                l.push_str(" [...]");
            }
            if !exit.errors.contains(&l) {
                exit.errors.push(l);
            }
        }
    }
    current
}

/// The early-exit rule (module doc).
pub fn verdict(exit: &GameExit) -> Verdict {
    if exit.not_started.is_some() {
        return Verdict::NotStarted;
    }
    let Some(code) = exit.code else {
        return Verdict::Normal;
    };
    if code == RETURN_TO_LAUNCHER {
        return Verdict::Normal;
    }
    let crash = if exit.windows_process {
        let c = code as u32;
        (c >= 0xC000_0000 && c != 0xC000_013A) || c == 0x8000_0003 || c == 0xE06D_7363
    } else {
        code > 128 && CRASH_SIGNALS.contains(&(code - 128))
    };
    if crash {
        return Verdict::Crash;
    }
    match exit.secs {
        Some(s) if s < EARLY_EXIT_SECS => Verdict::Early,
        _ => Verdict::Normal,
    }
}

/// One line for the log and the player's message.
pub fn headline(exit: &GameExit) -> String {
    let after = exit
        .secs
        .map(|s| format!("after {s} s"))
        .unwrap_or_else(|| "after an unknown time".into());
    match (&exit.not_started, exit.code) {
        (Some(why), _) => format!("the game did not start: {why}"),
        (None, Some(code)) => format!(
            "the game ended {after}, exit code {}",
            describe_code(code, exit.windows_process)
        ),
        (None, None) => "the game has not ended yet (or was stopped before it could say)".into(),
    }
}

pub fn summary_lines(exit: &GameExit) -> Vec<(String, String)> {
    let mut out = vec![
        ("Started".to_string(), format!("{} UTC{}", exit.started, if exit.debug { " (debug launch)" } else { "" })),
        ("Result".to_string(), headline(exit)),
        (
            "Verdict".to_string(),
            match verdict(exit) {
                Verdict::Normal => "normal",
                Verdict::Early => "ended early (within 60 s)",
                Verdict::Crash => "crashed",
                Verdict::NotStarted => "did not start",
            }
            .to_string(),
        ),
    ];
    for (i, e) in exit.errors.iter().enumerate() {
        out.push((format!("Wine/helper error {}", i + 1), e.clone()));
    }
    out
}

/// The last game start from `game.log` in `dir`.
/// Read line by line: a debug launch's file can be large.
pub fn last_exit(dir: &Path) -> Option<GameExit> {
    use std::io::{BufRead, BufReader};
    let file = File::open(dir.join(hv_log::file_name(hv_log::GAME_STEM, 0))).ok()?;
    parse_lines(
        BufReader::new(file)
            .split(b'\n')
            .map_while(Result::ok)
            .map(|l| String::from_utf8_lossy(&l).into_owned()),
    )
}

/// `== <now> UTC <text>` appended to game.log in `dir`.
pub fn append_game_line(dir: &Path, text: &str) {
    use std::io::Write;
    let path = dir.join(hv_log::file_name(hv_log::GAME_STEM, 0));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(
            format!("\n== {} UTC {}\n", hv_log::utc_stamp(SystemTime::now()), hv_log::redact(text)).as_bytes(),
        );
    }
}

pub fn mark_noted(dir: &Path) {
    append_game_line(dir, NOTED_MARK.trim_start());
}

/// The end line the launcher or its watcher writes on Windows.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn end_text(code: u32, secs: u64) -> String {
    format!("game process ended, exit code {code} (0x{code:08X}), after {secs} s")
}

pub fn not_started_text(why: &str, secs: u64) -> String {
    format!("game did not start: {why}, after {secs} s")
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub fn returned_text(code: i64, secs: u64) -> String {
    format!("game returned to the launcher, exit code {code}, after {secs} s")
}

// --- files on disk -------------------------------------------------------

fn file_time(meta: &fs::Metadata) -> String {
    meta.modified()
        .map(|t| format!("{} UTC", &hv_log::utc_stamp(t)[..19]))
        .unwrap_or_else(|_| "?".into())
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The files that decide whether the game starts.
pub const GAME_FILES: &[&str] = &[
    "mhf.exe",
    "mhfo-hd.dll",
    "mhfo.dll",
    "hvmods.dll",
    "mhf.ini",
    "Mezeporta/bin/meze-deps.exe",
];

/// Each of `GAME_FILES` in `root`: size, time and sha256, or "MISSING".
pub fn game_files(root: &Path) -> Vec<(String, String)> {
    GAME_FILES
        .iter()
        .map(|name| {
            let path = root.join(name);
            let value = match fs::metadata(&path) {
                Ok(meta) if meta.is_file() => format!(
                    "{} bytes, {}, sha256 {}",
                    meta.len(),
                    file_time(&meta),
                    sha256_file(&path).unwrap_or_else(|| "not readable".into())
                ),
                Ok(_) => "not a file".into(),
                Err(_) => "MISSING".into(),
            };
            (name.to_string(), value)
        })
        .collect()
}

fn newest_matching(dir: &Path, prefix: &str, suffix: &str, max: usize) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with(&prefix.to_lowercase()) && n.ends_with(&suffix.to_lowercase())
        })
        .map(|e| {
            let t = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
            (t, e.path())
        })
        .collect();
    found.sort_by(|a, b| b.cmp(a));
    found.into_iter().take(max).map(|(_, p)| p).collect()
}

/// hvmods' files in the game folder (module doc), as (title, text) blocks.
pub fn hvmods_files(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let crashes = newest_matching(root, "hvcrash-", ".txt", 5);
    if !crashes.is_empty() {
        let names: Vec<String> = crashes
            .iter()
            .map(|p| {
                let meta = fs::metadata(p).ok();
                format!(
                    "{} ({})",
                    p.file_name().unwrap_or_default().to_string_lossy(),
                    meta.as_ref().map(file_time).unwrap_or_default()
                )
            })
            .collect();
        out.push(("hvmods crash files (newest 5)".to_string(), names.join("\n") + "\n"));
        if let Some(text) = hv_log::tail_lines(&crashes[0], 250) {
            let name = crashes[0].file_name().unwrap_or_default().to_string_lossy().into_owned();
            out.push((format!("{name} (newest crash)"), text));
        }
    }
    for (name, lines) in [("hvsend-crash.txt", 250), ("hvmods.log", 150), ("hvdiag.txt", 200)] {
        let path = root.join(name);
        if fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
            if let Some(text) = hv_log::tail_lines(&path, lines) {
                let when = fs::metadata(&path).map(|m| file_time(&m)).unwrap_or_default();
                out.push((format!("{name} ({when}, last {lines} lines)"), text));
            }
        }
    }
    out
}

/// Windows Error Reporting's text report (`Report.wer`, UTF-16) as
/// `Name: value` lines of its `Sig[n]` fields.
pub fn wer_summary(bytes: &[u8]) -> String {
    let text = if bytes.starts_with(&[0xFF, 0xFE]) || (bytes.len() > 1 && bytes[1] == 0) {
        let start = if bytes.starts_with(&[0xFF, 0xFE]) { 2 } else { 0 };
        let units: Vec<u16> = bytes[start..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()))
    };
    let mut parts = Vec::new();
    for n in 0..10 {
        let (Some(name), Some(value)) = (field(&format!("Sig[{n}].Name=")), field(&format!("Sig[{n}].Value="))) else {
            continue;
        };
        let value = match (name.as_str(), u32::from_str_radix(value.trim_start_matches("0x"), 16)) {
            ("Exception Code", Ok(c)) => match ntstatus_name(c) {
                Some(n) => format!("{value} ({})", n.split(" (").next().unwrap_or(n)),
                None => value,
            },
            _ => value,
        };
        parts.push(format!("{name}: {value}"));
    }
    if parts.is_empty() {
        "(no crash fields)".into()
    } else {
        parts.join("; ")
    }
}

/// Windows Error Reporting entries (`AppCrash_<exe>_*` folders in
/// `wer_roots`) and crash dumps (`<exe>*.dmp` in `dump_dirs`) for the
/// launcher (the game runs in it) and mhf.exe, newest first.
pub fn wer_reports(wer_roots: &[PathBuf], dump_dirs: &[PathBuf], max: usize) -> Vec<(String, String)> {
    let mut found: Vec<(SystemTime, String, String)> = Vec::new();
    for root in wer_roots {
        for exe in ["mezeporta.exe", "mhf.exe"] {
            for dir in newest_matching(root, &format!("AppCrash_{exe}_"), "", max) {
                let t = fs::metadata(&dir).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
                let summary = fs::read(dir.join("Report.wer"))
                    .map(|b| wer_summary(&b))
                    .unwrap_or_else(|_| "Report.wer not readable".into());
                found.push((
                    t,
                    format!("WER {} ({} UTC)", dir.file_name().unwrap_or_default().to_string_lossy(), &hv_log::utc_stamp(t)[..19]),
                    summary,
                ));
            }
        }
    }
    for dir in dump_dirs {
        for exe in ["mezeporta.exe", "mhf.exe"] {
            for dump in newest_matching(dir, exe, ".dmp", max) {
                let meta = fs::metadata(&dump).ok();
                let t = meta.as_ref().and_then(|m| m.modified().ok()).unwrap_or(UNIX_EPOCH);
                found.push((
                    t,
                    format!("crash dump ({} UTC)", &hv_log::utc_stamp(t)[..19]),
                    format!(
                        "{} ({} bytes; not attached: send it only if we ask)",
                        dump.display(),
                        meta.map(|m| m.len()).unwrap_or(0)
                    ),
                ));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(max).map(|(_, k, v)| (k, v)).collect()
}

/// Where Windows keeps them (empty elsewhere).
pub fn wer_locations() -> (Vec<PathBuf>, Vec<PathBuf>) {
    if !cfg!(windows) {
        return (Vec::new(), Vec::new());
    }
    let mut wer = Vec::new();
    let mut dumps = Vec::new();
    for var in ["LOCALAPPDATA", "ProgramData"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base).join("Microsoft").join("Windows").join("WER");
            wer.push(base.join("ReportArchive"));
            wer.push(base.join("ReportQueue"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dumps.push(PathBuf::from(local).join("CrashDumps"));
    }
    (wer, dumps)
}

// --- the Windows watcher -------------------------------------------------

pub const WATCH_ARG: &str = "--hv-game-watch";

#[cfg(windows)]
pub fn epoch_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Windows: starts `<launcher> --hv-game-watch <pid> <start secs> <log dir>`
/// before the game runs in this process (module doc). Best effort.
#[cfg(windows)]
pub fn spawn_watcher(log_dir: &Path, started: SystemTime) {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let result = Command::new(exe)
        .arg(WATCH_ARG)
        .arg(std::process::id().to_string())
        .arg(epoch_secs(started).to_string())
        .arg(log_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    match result {
        Ok(child) => log::info!("game exit watcher started (pid {})", child.id()),
        Err(e) => log::warn!("could not start the game exit watcher: {e}"),
    }
}

/// The watcher itself: when started with `--hv-game-watch`, waits for the
/// process, writes its exit code to game.log and returns the code to exit
/// with. None for any other start.
pub fn run_watcher_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(WATCH_ARG) {
        return None;
    }
    let pid: u32 = args.get(2)?.parse().ok()?;
    let started = UNIX_EPOCH + Duration::from_secs(args.get(3)?.parse().ok()?);
    let dir = PathBuf::from(args.get(4)?);
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        let Ok(handle) = handle else {
            append_game_line(&dir, "game exit watcher: could not open the game process");
            return Some(1);
        };
        let mut code = 0u32;
        let ok = unsafe {
            WaitForSingleObject(handle, INFINITE);
            let ok = GetExitCodeProcess(handle, &mut code).is_ok();
            let _ = CloseHandle(handle);
            ok
        };
        let secs = SystemTime::now().duration_since(started).unwrap_or_default().as_secs();
        if ok {
            append_game_line(&dir, &end_text(code, secs));
        } else {
            append_game_line(&dir, &format!("game exit watcher: no exit code, after {secs} s"));
        }
    }
    #[cfg(not(windows))]
    let _ = (pid, started, dir);
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hv-crash-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hv_crash_decodes_windows_codes() {
        assert!(describe_code(0xC000_0005, true).starts_with("0xC0000005 access violation"));
        assert!(describe_code(0xC000_0135, true).starts_with("0xC0000135 DLL not found"));
        assert!(describe_code(0xC000_007B, true).starts_with("0xC000007B bad image"));
        assert!(describe_code(0xC000_0409, true).starts_with("0xC0000409 stack buffer overrun"));
        assert_eq!(describe_code(0xC0AB_CDEF, true), "0xC0ABCDEF (a Windows error status)");
        assert_eq!(describe_code(0, true), "0 (normal exit)");
        assert_eq!(describe_code(1, true), "1");
        assert!(describe_code(102, true).contains("back to the launcher"));
        // GetExitCodeProcess's u32 written as a decimal number, read back as i64.
        assert_eq!(ntstatus_name(3_221_225_477), ntstatus_name(0xC000_0005));
    }

    #[test]
    fn hv_crash_decodes_linux_status() {
        assert!(describe_code(139, false).contains("SIGSEGV"));
        assert!(describe_code(134, false).contains("SIGABRT"));
        assert!(describe_code(137, false).contains("SIGKILL"));
        assert!(describe_code(127, false).contains("not found"));
        // Wine keeps a Windows code's last byte.
        let five = describe_code(5, false);
        assert!(five.contains("0xC0000005 access violation"), "{five}");
        assert!(five.contains("meze-deps.exe"), "{five}");
        assert!(describe_code(53, false).contains("0xC0000135 DLL not found"));
        assert!(describe_code(123, false).contains("0xC000007B bad image"));
        assert_eq!(describe_code(42, false), "42");
    }

    fn exit(code: Option<i64>, secs: Option<u64>, windows: bool) -> GameExit {
        GameExit { code, secs, windows_process: windows, ..GameExit::default() }
    }

    #[test]
    fn hv_crash_early_exit_rule() {
        use Verdict::*;
        assert_eq!(verdict(&exit(Some(0), Some(3), false)), Early);
        assert_eq!(verdict(&exit(Some(5), Some(59), false)), Early);
        assert_eq!(verdict(&exit(Some(5), Some(60), false)), Normal);
        assert_eq!(verdict(&exit(Some(0), Some(3600), true)), Normal);
        assert_eq!(verdict(&exit(Some(0xC000_0005), Some(3600), true)), Crash);
        assert_eq!(verdict(&exit(Some(0xC000_0135), Some(0), true)), Crash);
        assert_eq!(verdict(&exit(Some(0xC000_013A), Some(3600), true)), Normal);
        assert_eq!(verdict(&exit(Some(139), Some(7200), false)), Crash);
        assert_eq!(verdict(&exit(Some(143), Some(7200), false)), Normal);
        assert_eq!(verdict(&exit(Some(143), Some(2), false)), Early);
        // Back to the launcher is a choice, however soon.
        assert_eq!(verdict(&exit(Some(102), Some(5), true)), Normal);
        // Still running (no end line).
        assert_eq!(verdict(&exit(None, None, false)), Normal);
        let mut not = exit(None, Some(0), true);
        not.not_started = Some("unable to load mhfo-hd.dll".into());
        assert_eq!(verdict(&not), NotStarted);
    }

    #[test]
    fn hv_crash_parses_game_log() {
        let text = "\
== 2026-11-24 09:00:00.000 UTC game start: old
== 2026-11-24 09:30:00 UTC game process ended, exit code 0, after 1800 s
== 2026-11-24 10:15:03.123 UTC game start: \"/usr/bin/wine\" \"meze-deps.exe\" \"--stdin-b64\" in ~/Games/MHFZ (debug launch: WINEDEBUG=+seh)
0024:err:module:import_dll Library d3dx9_43.dll (which is needed by L\"Z:\\\\mhfo-hd.dll\") not found
0024:err:module:import_dll Library d3dx9_43.dll (which is needed by L\"Z:\\\\mhfo-hd.dll\") not found
fixme:heap:something harmless
wine: Unhandled page fault on read access to 00000000 at address 1000ABCD (thread 0024), starting debugger...

== 2026-11-24 10:15:06 UTC game process ended, exit code 5, after 3 s
";
        let e = parse_game_log(text).unwrap();
        assert_eq!(e.started, "2026-11-24 10:15:03.123");
        assert_eq!(e.ended, "2026-11-24 10:15:06");
        assert_eq!(e.code, Some(5));
        assert_eq!(e.secs, Some(3));
        assert!(!e.windows_process && e.debug && !e.noted);
        assert_eq!(e.errors.len(), 2, "{:?}", e.errors);
        assert!(e.errors[0].contains("d3dx9_43.dll"));
        assert_eq!(verdict(&e), Verdict::Early);
        assert!(headline(&e).starts_with("the game ended after 3 s, exit code 5 (maybe:"));

        let noted = format!("{text}== 2026-11-24 10:20:00.000 UTC launcher: noted\n");
        assert!(parse_game_log(&noted).unwrap().noted);

        // Windows: the launcher's own line first, then the watcher's.
        let win = "\
== 2026-11-24 10:15:03.123 UTC game start: in the launcher process (meze-butter) in ~\\MHFZ
== 2026-11-24 10:15:04.000 UTC game process ended, exit code 3221225477 (0xC0000005), after 1 s
";
        let e = parse_game_log(win).unwrap();
        assert_eq!(e.code, Some(3_221_225_477));
        assert!(e.windows_process);
        assert_eq!(verdict(&e), Verdict::Crash);
        assert!(headline(&e).contains("0xC0000005 access violation"), "{}", headline(&e));

        let ret = format!(
            "== 2026-11-24 10:00:00.000 UTC game start: x\n== 2026-11-24 10:00:30.000 UTC {}\n== 2026-11-24 10:00:31.000 UTC {}\n",
            returned_text(102, 30),
            end_text(0, 31)
        );
        let e = parse_game_log(&ret).unwrap();
        assert_eq!((e.code, e.secs), (Some(102), Some(30)));
        assert_eq!(verdict(&e), Verdict::Normal);

        let failed = format!(
            "== 2026-11-24 10:00:00.000 UTC game start: x\n== 2026-11-24 10:00:00.500 UTC {}\n",
            not_started_text("unable to load mhfo-hd.dll: The specified module could not be found. (0x8007007E)", 0)
        );
        let e = parse_game_log(&failed).unwrap();
        assert_eq!(verdict(&e), Verdict::NotStarted);
        assert!(e.not_started.unwrap().ends_with("(0x8007007E)"));

        // Running: a start line and nothing after it.
        let e = parse_game_log("== 2026-11-24 10:00:00.000 UTC game start: x\nsome output\n").unwrap();
        assert_eq!((e.code, verdict(&e)), (None, Verdict::Normal));
        assert!(parse_game_log("no start here\n").is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn hv_crash_exit_wrapper_writes_the_end_line() {
        let run = |cmd: &str| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(GAME_EXIT_CODE_SCRIPT)
                .arg("mezeporta-game")
                .arg("/bin/sh")
                .arg("-c")
                .arg(cmd)
                .output()
                .unwrap();
            let text = format!(
                "== 2026-11-24 10:00:00.000 UTC game start: x\n{}",
                String::from_utf8_lossy(&out.stdout)
            );
            (out.status.code(), parse_game_log(&text).unwrap())
        };
        let (status, e) = run("echo game output; exit 5");
        assert_eq!(status, Some(5));
        assert_eq!((e.code, e.secs), (Some(5), Some(0)));
        assert_eq!(verdict(&e), Verdict::Early);
        let (_, e) = run("kill -SEGV $$");
        assert_eq!(e.code, Some(139));
        assert_eq!(verdict(&e), Verdict::Crash);
        assert!(headline(&e).contains("SIGSEGV"));
    }

    #[test]
    fn hv_crash_files_on_disk() {
        let root = temp_dir("files");
        fs::write(root.join("mhf.exe"), b"abc").unwrap();
        fs::create_dir_all(root.join("Mezeporta/bin")).unwrap();
        let files = game_files(&root);
        assert_eq!(files[0].0, "mhf.exe");
        assert!(files[0].1.starts_with("3 bytes, "), "{}", files[0].1);
        assert!(files[0].1.ends_with("sha256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert_eq!(files[1], ("mhfo-hd.dll".to_string(), "MISSING".to_string()));

        fs::write(root.join("hvcrash-20261124-101500.txt"), "HVCRASH 1\nexception: c0000005 access violation\n").unwrap();
        fs::write(root.join("hvmods.log"), "hvmods 53 loaded\n").unwrap();
        fs::write(root.join("hvsend-crash.txt"), "").unwrap();
        let blocks = hvmods_files(&root);
        let titles: Vec<&str> = blocks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles.len(), 3, "{titles:?}");
        assert!(blocks[1].1.contains("c0000005"));
        assert!(titles[2].starts_with("hvmods.log"));

        // A WER report folder (UTF-16 with BOM) and a dump.
        let wer = root.join("ReportArchive");
        let entry = wer.join("AppCrash_Mezeporta.exe_abc_def_123");
        fs::create_dir_all(&entry).unwrap();
        let report = "Version=1\r\nSig[0].Name=Application Name\r\nSig[0].Value=Mezeporta.exe\r\nSig[3].Name=Fault Module Name\r\nSig[3].Value=mhfo-hd.dll\r\nSig[6].Name=Exception Code\r\nSig[6].Value=c0000005\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(report.encode_utf16().flat_map(|u| u.to_le_bytes()));
        fs::write(entry.join("Report.wer"), bytes).unwrap();
        let dumps = root.join("CrashDumps");
        fs::create_dir_all(&dumps).unwrap();
        fs::write(dumps.join("Mezeporta.exe.4242.dmp"), b"MDMP").unwrap();
        let found = wer_reports(&[wer, root.join("nope")], &[dumps], 5);
        assert_eq!(found.len(), 2, "{found:?}");
        let wer_line = found.iter().find(|(k, _)| k.starts_with("WER")).unwrap();
        assert_eq!(
            wer_line.1,
            "Application Name: Mezeporta.exe; Fault Module Name: mhfo-hd.dll; Exception Code: c0000005 (access violation)"
        );
        assert!(found.iter().any(|(k, v)| k.starts_with("crash dump") && v.contains("4 bytes")));

        // The launcher's lines in game.log, then last_exit and noted.
        let logs = temp_dir("logs");
        fs::write(logs.join("game.log"), "== 2026-11-24 10:00:00.000 UTC game start: x\n").unwrap();
        append_game_line(&logs, &end_text(0xC000_0135, 0));
        let e = last_exit(&logs).unwrap();
        assert_eq!(verdict(&e), Verdict::Crash);
        mark_noted(&logs);
        assert!(last_exit(&logs).unwrap().noted);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(logs);
    }
}
