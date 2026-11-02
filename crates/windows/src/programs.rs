//! Installed programs from the "Uninstall" registry keys, read-only and without administrator
//! rights (docs/specs/Spec - Composants de jeu.md, R-DRT-01). Used to find game components.

use drivn_core::games::InstalledProgram;
use drivn_core::model::Board;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey,
    RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::{PCWSTR, PWSTR, w};

/// Every entry that has a display name, from the machine (64 and 32 bits) and user keys.
pub fn installed_programs() -> Vec<InstalledProgram> {
    let keys = [
        (
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        ),
        (
            HKEY_CURRENT_USER,
            w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        ),
    ];
    let mut programs = Vec::new();
    for (root, path) in keys {
        let mut key = HKEY::default();
        // SAFETY: `path` is a static NUL-terminated string; `key` is a valid out pointer.
        if unsafe { RegOpenKeyExW(root, path, None, KEY_READ, &mut key) } != ERROR_SUCCESS {
            continue; // a missing key (no 32-bit programs, nothing per user) is normal
        }
        for index in 0.. {
            let mut name = [0u16; 256]; // registry key names are at most 255 characters
            let mut length = name.len() as u32;
            // SAFETY: `name` holds `length` characters; the call writes a NUL-terminated name.
            let result = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if result != ERROR_SUCCESS {
                break; // ERROR_NO_MORE_ITEMS, or a read error: stop on this key
            }
            let entry = PCWSTR(name.as_ptr());
            if let Some(display) = text(key, entry, w!("DisplayName")) {
                programs.push(InstalledProgram {
                    name: display,
                    version: text(key, entry, w!("DisplayVersion")),
                    publisher: text(key, entry, w!("Publisher")),
                });
            }
        }
        // SAFETY: `key` was opened above and is not used after this.
        unsafe {
            let _ = RegCloseKey(key);
        }
    }
    programs
}

/// `DisplayVersion` of Windows (`26H2`): the Catalog search names the release (read-only).
pub fn windows_release() -> Option<String> {
    text(
        HKEY_LOCAL_MACHINE,
        w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
        w!("DisplayVersion"),
    )
}

/// The motherboard and BIOS as the firmware reports them (SMBIOS, copied by Windows at boot),
/// read-only and without administrator rights (R-SRC-07, ADR-0032; keys to verify on each PC).
pub fn board() -> Board {
    let key = w!(r"HARDWARE\DESCRIPTION\System\BIOS");
    let read = |value| text(HKEY_LOCAL_MACHINE, key, value);
    Board {
        system_manufacturer: read(w!("SystemManufacturer")),
        manufacturer: read(w!("BaseBoardManufacturer")),
        product: read(w!("BaseBoardProduct")),
        bios_version: read(w!("BIOSVersion")),
        bios_date: read(w!("BIOSReleaseDate")),
    }
}

/// A string value of `key\entry`, trimmed; `None` when missing, empty or not a string.
fn text(key: HKEY, entry: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: first call only asks for the size in bytes.
    let result = unsafe {
        RegGetValueW(
            key,
            entry,
            value,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if result != ERROR_SUCCESS || size < 2 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `size` bytes; RegGetValueW NUL-terminates REG_SZ data.
    let result = unsafe {
        RegGetValueW(
            key,
            entry,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]);
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_programs_are_read() {
        // Read-only (KEY_READ). Any Windows has at least one program with a display name.
        assert!(!installed_programs().is_empty());
    }
}
