// Hunsterverse (1.5.4-hv3): "Fix text scaling on high-DPI screens".
//
// On Windows the game runs inside the launcher's own process (meze-butter
// loads mhfo-hd.dll into Mezeporta.exe), and the launcher's UI toolkit (tao)
// makes that process per-monitor DPI aware. On monitors with scaling above
// 100 % the game's text then breaks. The known fix (r/MHF "Problem with font
// resolution") is Mezeporta.exe > Properties > Compatibility > Change high
// DPI settings > "Override high DPI scaling behaviour, scaling performed by:
// System". That checkbox writes, for the current user,
//
//   HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers
//   value = full path of the exe, REG_SZ, tokens separated by spaces:
//   "System" = DPIUNAWARE, "System (Enhanced)" = GDIDPISCALING DPIUNAWARE,
//   "Application" = HIGHDPIAWARE; Windows 8+ puts "~" as the first token.
//
// (Microsoft documents the same value for Visual Studio:
// https://learn.microsoft.com/en-us/previous-versions/visualstudio/visual-studio-2017/designers/disable-dpi-awareness
// "reg add ...\AppCompatFlags\Layers /v <exe> /t REG_SZ /d DPIUNAWARE".)
// HKLM holds the "for all users" copy; it needs admin, so we never touch it.
//
// Windows applies the layer when the process starts, so a newly written
// value takes effect from the next launcher start. We merge our token into
// whatever is there (other compatibility flags stay) and, when the player
// turns the setting off, remove only plain DPIUNAWARE.

const DPI_UNAWARE: &str = "DPIUNAWARE";
const GDI_DPI_SCALING: &str = "GDIDPISCALING";
const HIGH_DPI_AWARE: &str = "HIGHDPIAWARE";
const TILDE: &str = "~";

fn has_token(tokens: &[&str], token: &str) -> bool {
    tokens.iter().any(|t| t.eq_ignore_ascii_case(token))
}

/// The Layers string with DPIUNAWARE added, or None when nothing should
/// change: already "System" or "System (Enhanced)", or the player chose
/// "Application" (HIGHDPIAWARE) in Windows themselves.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn merge_dpi_unaware(existing: Option<&str>) -> Option<String> {
    let tokens: Vec<&str> = existing.unwrap_or("").split_whitespace().collect();
    if has_token(&tokens, DPI_UNAWARE) || has_token(&tokens, HIGH_DPI_AWARE) {
        return None;
    }
    let mut out: Vec<&str> = Vec::with_capacity(tokens.len() + 2);
    // Windows 8+ writes "~" first; keep it first, add it to a new value.
    if tokens.is_empty() || tokens[0] == TILDE {
        out.push(TILDE);
    }
    out.extend(tokens.iter().copied().filter(|t| *t != TILDE));
    out.push(DPI_UNAWARE);
    Some(out.join(" "))
}

/// The change that removes our DPIUNAWARE: None = nothing to do,
/// Some(None) = delete the value (nothing else left), Some(Some(s)) = write s.
/// "System (Enhanced)" (GDIDPISCALING DPIUNAWARE) is the player's own choice
/// and stays.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn remove_dpi_unaware(existing: Option<&str>) -> Option<Option<String>> {
    let tokens: Vec<&str> = existing?.split_whitespace().collect();
    if !has_token(&tokens, DPI_UNAWARE) || has_token(&tokens, GDI_DPI_SCALING) {
        return None;
    }
    let rest: Vec<&str> = tokens
        .into_iter()
        .filter(|t| !t.eq_ignore_ascii_case(DPI_UNAWARE))
        .collect();
    if rest.iter().all(|t| *t == TILDE) {
        Some(None)
    } else {
        Some(Some(rest.join(" ")))
    }
}

/// Before the game starts: make sure the launcher exe (the game's process on
/// Windows) has, or no longer has, the "System" DPI override. Logs what it
/// did; never fails the launch.
pub(crate) fn apply_before_game(enabled: bool) {
    #[cfg(windows)]
    {
        match win::apply(enabled) {
            Ok(msg) => log::info!("high-DPI fix: {msg}"),
            Err(e) => log::warn!("high-DPI fix: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
    }
}

#[cfg(windows)]
mod win {
    use windows::core::{s, w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        REG_VALUE_TYPE,
    };

    const LAYERS: PCWSTR =
        w!("Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers");

    fn running_under_wine() -> bool {
        unsafe {
            match GetModuleHandleW(w!("ntdll.dll")) {
                Ok(ntdll) => GetProcAddress(ntdll, s!("wine_get_version")).is_some(),
                Err(_) => false,
            }
        }
    }

    fn check(result: windows::core::Result<()>, what: &str) -> Result<(), String> {
        result.map_err(|e| format!("{what} failed ({e})"))
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn read(key: &Key, name: &HSTRING) -> Result<Option<String>, String> {
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let code =
            unsafe { RegQueryValueExW(key.0, name, None, Some(&mut kind), None, Some(&mut size)) };
        if let Err(e) = &code {
            if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
                return Ok(None);
            }
        }
        check(code, "reading the Layers value")?;
        if kind != REG_SZ {
            return Err(format!(
                "Layers value has type {}, not REG_SZ; left as is",
                kind.0
            ));
        }
        let mut buf = vec![0u16; (size as usize + 1) / 2 + 1];
        let mut size = (buf.len() * 2) as u32;
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                name,
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            )
        };
        check(code, "reading the Layers value")?;
        buf.truncate(size as usize / 2);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(Some(String::from_utf16_lossy(&buf)))
    }

    fn write(key: &Key, name: &HSTRING, value: &str) -> Result<(), String> {
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        check(
            unsafe { RegSetValueExW(key.0, name, 0, REG_SZ, Some(bytes)) },
            "writing the Layers value",
        )
    }

    pub(super) fn apply(enabled: bool) -> Result<String, String> {
        if running_under_wine() {
            return Ok("skipped (Wine/Proton)".into());
        }
        let exe = std::env::current_exe().map_err(|e| format!("no exe path: {e}"))?;
        let name = HSTRING::from(exe.as_os_str());
        let mut hkey = HKEY::default();
        check(
            unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    LAYERS,
                    0,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_QUERY_VALUE | KEY_SET_VALUE,
                    None,
                    &mut hkey,
                    None,
                )
            },
            "opening HKCU AppCompatFlags\\Layers",
        )?;
        let key = Key(hkey);
        let existing = read(&key, &name)?;
        let shown = existing.as_deref().unwrap_or("(none)");
        let path = exe.display();
        if enabled {
            match super::merge_dpi_unaware(existing.as_deref()) {
                None => Ok(format!("{path}: left as is ({shown})")),
                Some(new) => {
                    write(&key, &name, &new)?;
                    Ok(format!(
                        "{path}: {shown} -> {new} (takes effect from the next launcher start)"
                    ))
                }
            }
        } else {
            match super::remove_dpi_unaware(existing.as_deref()) {
                None => Ok(format!("off; {path}: left as is ({shown})")),
                Some(Some(new)) => {
                    write(&key, &name, &new)?;
                    Ok(format!("off; {path}: {shown} -> {new}"))
                }
                Some(None) => {
                    check(
                        unsafe { RegDeleteValueW(key.0, &name) },
                        "deleting the Layers value",
                    )?;
                    Ok(format!("off; {path}: removed ({shown})"))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hv_dpi_merge_adds_token() {
        assert_eq!(merge_dpi_unaware(None).as_deref(), Some("~ DPIUNAWARE"));
        assert_eq!(merge_dpi_unaware(Some("")).as_deref(), Some("~ DPIUNAWARE"));
        assert_eq!(
            merge_dpi_unaware(Some("  ")).as_deref(),
            Some("~ DPIUNAWARE")
        );
        assert_eq!(
            merge_dpi_unaware(Some("~ WIN8RTM")).as_deref(),
            Some("~ WIN8RTM DPIUNAWARE")
        );
        assert_eq!(
            merge_dpi_unaware(Some("~ RUNASADMIN  WIN8RTM")).as_deref(),
            Some("~ RUNASADMIN WIN8RTM DPIUNAWARE")
        );
        // An old-style value without "~" stays without it.
        assert_eq!(
            merge_dpi_unaware(Some("WIN7RTM")).as_deref(),
            Some("WIN7RTM DPIUNAWARE")
        );
    }

    #[test]
    fn hv_dpi_merge_leaves_existing_choice() {
        assert_eq!(merge_dpi_unaware(Some("~ DPIUNAWARE")), None);
        assert_eq!(merge_dpi_unaware(Some("~ dpiunaware WIN8RTM")), None);
        assert_eq!(merge_dpi_unaware(Some("~ GDIDPISCALING DPIUNAWARE")), None);
        assert_eq!(merge_dpi_unaware(Some("~ HIGHDPIAWARE")), None);
    }

    #[test]
    fn hv_dpi_remove_only_our_token() {
        assert_eq!(remove_dpi_unaware(None), None);
        assert_eq!(remove_dpi_unaware(Some("~ WIN8RTM")), None);
        assert_eq!(remove_dpi_unaware(Some("~ DPIUNAWARE")), Some(None));
        assert_eq!(remove_dpi_unaware(Some("DPIUNAWARE")), Some(None));
        assert_eq!(
            remove_dpi_unaware(Some("~ WIN8RTM DPIUNAWARE")),
            Some(Some("~ WIN8RTM".to_string()))
        );
        // "System (Enhanced)" is the player's choice: keep.
        assert_eq!(remove_dpi_unaware(Some("~ GDIDPISCALING DPIUNAWARE")), None);
    }

    #[test]
    fn hv_dpi_round_trip() {
        for start in ["~ WIN8RTM", "~ RUNASADMIN WIN8RTM", "WIN7RTM"] {
            let merged = merge_dpi_unaware(Some(start)).unwrap();
            assert_eq!(
                remove_dpi_unaware(Some(&merged)),
                Some(Some(start.to_string()))
            );
        }
        let merged = merge_dpi_unaware(None).unwrap();
        assert_eq!(remove_dpi_unaware(Some(&merged)), Some(None));
    }
}
