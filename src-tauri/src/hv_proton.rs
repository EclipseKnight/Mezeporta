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

#[cfg(test)]
mod tests {
    use super::*;

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
