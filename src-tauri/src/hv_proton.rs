//! Hunsterverse: starting the game through Proton on Linux (Steam Deck).
//!
//! On a Steam Deck our installer sets the Wine prefix mode to "proton" and
//! gives the launcher a Proton command in `MEZEPORTA_PROTON_CMD`
//! (`Hunsterverse-setup/proton-host.sh`, which starts Steam's Proton outside
//! the Flatpak runtime the launcher runs in). SteamOS has no Wine. Launchers
//! before 1.5.4-hv3 reset "proton" to "portable" whenever Dev Mode was off,
//! so they ran plain `wine` and failed ("failed to run wine reg"). The rules
//! here keep such players working: a Wine mode with no Wine installed falls
//! back to the installer's Proton.
#![cfg_attr(windows, allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The installer's Proton command (an executable path).
pub const INSTALLER_PROTON_ENV: &str = "MEZEPORTA_PROTON_CMD";

const PROTON: &str = "proton";

/// The Wine prefix mode to start the game with. The setting wins, except
/// that a Wine mode (portable, system, custom) with no Wine on this system
/// becomes "proton" when the installer gave a Proton command that exists.
pub fn launch_mode(configured: &str, wine_found: bool, installer_proton_found: bool) -> String {
    if configured != PROTON && !wine_found && installer_proton_found {
        PROTON.to_string()
    } else {
        configured.to_string()
    }
}

/// The installer's Proton command, if the variable is set (not blank).
pub fn installer_proton_command(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Where Proton keeps its Wine prefix: `$STEAM_COMPAT_DATA_PATH/pfx`.
pub fn compat_prefix(compat_data: Option<&Path>) -> Option<PathBuf> {
    compat_data
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.join("pfx"))
}

/// Proton refuses to run without `STEAM_COMPAT_DATA_PATH` (a Python
/// KeyError the player would never understand): say what is missing instead.
pub fn check_compat_data(compat_data: Option<&Path>) -> Result<(), String> {
    if compat_prefix(compat_data).is_some() {
        Ok(())
    } else {
        Err(
            "Proton needs STEAM_COMPAT_DATA_PATH: start Hunsterverse with run-hunsterverse.sh \
             or through Steam"
                .to_string(),
        )
    }
}

/// How the game starts through Proton (1.5.4-hv7). Up to hv6 it was `proton
/// run meze-deps.exe --stdin-b64`: `run` starts the game through Proton's
/// steam.exe, which creates it with `CreateProcessW(..., bInheritHandles
/// FALSE, ...)` and no `STARTF_USESTDHANDLES` (Proton's
/// `steam_helper/steam.c`), so meze-deps.exe had no stdin, read no start
/// data and ended with exit code 5 (Steam Deck report, 2026-11-24).
/// `runinprefix` runs Proton's wine on the game directly: its stdin is the
/// launcher's pipe, as with plain Wine. It skips Proton's prefix set-up
/// (create, upgrade, DXVK), so `getcompatpath` runs first: Proton does that
/// set-up for it, then only converts a path. Its stdin is /dev/null, so it
/// cannot take the start data. `$1` is the Proton command, the rest the
/// game's command line.
pub const GAME_SCRIPT: &str = r#"proton=$1
shift
"$proton" getcompatpath "$PWD" </dev/null >/dev/null || echo "== proton getcompatpath (prefix set-up) ended with exit code $?"
exec "$proton" runinprefix "$@""#;

/// `argv[0]` of GAME_SCRIPT (shown by `ps`).
const GAME_SCRIPT_NAME: &str = "mezeporta-proton";

/// The command line before the game's own (`meze-deps.exe --stdin-b64`):
/// GAME_SCRIPT through `sh`, or without a shell only `proton runinprefix`
/// (the prefix must then exist already).
pub fn game_command(proton: &Path, sh: Option<&Path>) -> Vec<OsString> {
    match sh {
        Some(sh) => vec![
            sh.into(),
            "-c".into(),
            GAME_SCRIPT.into(),
            GAME_SCRIPT_NAME.into(),
            proton.into(),
        ],
        None => vec![proton.into(), "runinprefix".into()],
    }
}

/// Proton's own log in a debug launch: `PROTON_LOG=1` writes
/// `$PROTON_LOG_DIR/steam-$SteamGameId.log` (version, options, Wine's and
/// DXVK's output), but only when `SteamGameId` is set; with `PROTON_LOG` and
/// no `SteamGameId` Proton sets `WINEDEBUG=-all` (its `init_session`), so
/// the debug launch would lose everything.
pub const PROTON_LOG_GAME_ID: &str = "hunsterverse";
pub const PROTON_LOG_FILE: &str = "steam-hunsterverse.log";
pub const PROTON_LOG_VARS: [&str; 3] = ["PROTON_LOG", "PROTON_LOG_DIR", "SteamGameId"];

/// Whether the Proton command (its text) passes the PROTON_LOG variables on.
/// The installer's `proton-host.sh` starts Proton outside the Steam Deck's
/// Flatpak sandbox with `flatpak-spawn --host`, which passes only the
/// variables it names (`--env=`): up to now not `PROTON_LOG_DIR` and
/// `SteamGameId`. Anything else (Proton's own script, another wrapper) is
/// taken to pass the environment on.
pub fn proton_log_passes(command_text: &str) -> bool {
    !command_text.contains("flatpak-spawn")
        || PROTON_LOG_VARS.iter().all(|v| command_text.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GAME_SCRIPT with a stand-in Proton that records each call and its
    /// stdin; `prep_code`: getcompatpath's exit code.
    #[cfg(unix)]
    fn run_game_script(prep_code: i32) -> (Option<i32>, String, String) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        let dir = std::env::temp_dir().join(format!(
            "hv-proton-{}-{}",
            std::process::id(),
            prep_code
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let proton = dir.join("proton host.sh");
        std::fs::write(
            &proton,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{log}'\nprintf 'stdin=' >> '{log}'\ncat >> '{log}'\necho >> '{log}'\n\
                 [ \"$1\" = getcompatpath ] && {{ echo 'Z:\\game'; exit {prep_code}; }}\nexit 6\n",
                log = dir.join("calls.log").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&proton, std::fs::Permissions::from_mode(0o755)).unwrap();
        let argv = game_command(&proton, Some(Path::new("/bin/sh")));
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .arg("/g/meze-deps.exe")
            .arg("--stdin-b64")
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"cGF5bG9hZA==").unwrap();
        let out = child.wait_with_output().unwrap();
        let calls = std::fs::read_to_string(dir.join("calls.log")).unwrap();
        let calls = calls.replace(&dir.display().to_string(), "<dir>");
        let _ = std::fs::remove_dir_all(&dir);
        (
            out.status.code(),
            calls,
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn game_script_sets_up_the_prefix_then_runs_the_game_with_stdin() {
        let (code, calls, stdout) = run_game_script(0);
        // The prefix set-up gets no stdin; the game gets all of it, through
        // runinprefix (not run: steam.exe would drop it); its exit code is kept.
        assert_eq!(
            calls,
            "getcompatpath|<dir>|stdin=\nruninprefix|/g/meze-deps.exe|--stdin-b64|stdin=cGF5bG9hZA==\n"
        );
        assert_eq!(code, Some(6));
        assert_eq!(stdout, "", "getcompatpath's path is not shown");

        // A failed set-up is said and the game still starts.
        let (code, calls, stdout) = run_game_script(3);
        assert!(calls.ends_with("runinprefix|/g/meze-deps.exe|--stdin-b64|stdin=cGF5bG9hZA==\n"), "{calls}");
        assert_eq!(code, Some(6));
        assert_eq!(stdout, "== proton getcompatpath (prefix set-up) ended with exit code 3\n");
    }

    #[test]
    fn game_command_without_a_shell_is_runinprefix() {
        let p = Path::new("/g/proton-host.sh");
        assert_eq!(game_command(p, None), vec![OsString::from("/g/proton-host.sh"), "runinprefix".into()]);
        let with_sh = game_command(p, Some(Path::new("/bin/sh")));
        assert_eq!(with_sh[0], "/bin/sh");
        assert_eq!(with_sh[1], "-c");
        assert_eq!(with_sh[4], "/g/proton-host.sh");
        assert!(!with_sh.iter().any(|a| a == "run"));
    }

    #[test]
    fn proton_log_only_where_the_variables_get_through() {
        // Proton's own script, or a plain wrapper: the environment goes on.
        assert!(proton_log_passes("#!/usr/bin/env python3\n# proton\n"));
        // The installer's proton-host.sh up to now: PROTON_LOG only.
        let host = "args=(--host \"--directory=$PWD\")\nfor v in STEAM_COMPAT_CLIENT_INSTALL_PATH STEAM_COMPAT_DATA_PATH GDK_BACKEND WINIT_UNIX_BACKEND \\\n         SDL_VIDEODRIVER WINEESYNC WINEFSYNC WINEDEBUG WINEDLLOVERRIDES PROTON_LOG; do\n  if [[ -n \"${!v+set}\" ]]; then args+=(\"--env=$v=${!v}\"); fi\ndone\nexec flatpak-spawn \"${args[@]}\" \"$proton\" \"$@\"\n";
        assert!(!proton_log_passes(host));
        let newer = host.replace("PROTON_LOG; do", "PROTON_LOG PROTON_LOG_DIR SteamGameId; do");
        assert!(proton_log_passes(&newer));
    }

    #[test]
    fn proton_setting_is_kept() {
        for wine in [true, false] {
            for installer in [true, false] {
                assert_eq!(launch_mode("proton", wine, installer), "proton");
            }
        }
    }

    #[test]
    fn wine_mode_without_wine_uses_the_installers_proton() {
        for mode in ["portable", "system", "custom"] {
            assert_eq!(launch_mode(mode, false, true), "proton", "{mode}");
            // Wine installed: the player's choice stands.
            assert_eq!(launch_mode(mode, true, true), mode);
            // No installer Proton: unchanged (the missing-tools warning says why).
            assert_eq!(launch_mode(mode, false, false), mode);
        }
    }

    #[test]
    fn installer_command_from_the_variable() {
        assert_eq!(installer_proton_command(None), None);
        assert_eq!(installer_proton_command(Some("  ")), None);
        assert_eq!(
            installer_proton_command(Some("/g/Hunsterverse-setup/proton-host.sh")),
            Some(PathBuf::from("/g/Hunsterverse-setup/proton-host.sh"))
        );
    }

    #[test]
    fn prefix_is_pfx_in_the_compat_data() {
        assert_eq!(
            compat_prefix(Some(Path::new("/home/deck/Games/MHFZ-proton"))),
            Some(PathBuf::from("/home/deck/Games/MHFZ-proton/pfx"))
        );
        assert_eq!(compat_prefix(Some(Path::new(""))), None);
        assert_eq!(compat_prefix(None), None);
        assert!(check_compat_data(None)
            .unwrap_err()
            .contains("STEAM_COMPAT_DATA_PATH"));
        assert!(check_compat_data(Some(Path::new("/x"))).is_ok());
    }
}
