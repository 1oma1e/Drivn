//! File-system facts the app and the installer check before trusting a path (Spec - Installation,
//! « Fichiers et droits », « Où est l'installeur »). Reads only.

use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    AccessCheck, DACL_SECURITY_INFORMATION, DuplicateToken, GENERIC_MAPPING,
    GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, PSECURITY_DESCRIPTOR,
    PSID, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_APPEND_DATA,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, WRITE_DAC, WRITE_OWNER,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_ProgramData, FOLDERID_System, FOLDERID_Windows,
    KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::{HSTRING, PWSTR};

const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

fn known_folder(id: &windows::core::GUID) -> Result<PathBuf, String> {
    // SAFETY: the returned string is copied, then freed with CoTaskMemFree as documented.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let out = path.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(path.0 as *const _));
        out.map(PathBuf::from)
    }
}

/// DLLs are then looked up in the program's own folder and in System32 only, never in the current
/// folder or the `PATH` (DLL planting). Microsoft's advice for every program, elevated ones first
/// (Microsoft Learn, « SetDefaultDllDirectories function »).
pub fn harden_dll_search() {
    // SAFETY: plain call with constant flags, made first thing in `main`.
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    }
}

/// `%LocalAppData%` of the user, asked to Windows (an environment variable can be missing,
/// depending on how the app was started).
pub fn local_app_data() -> Result<PathBuf, String> {
    known_folder(&FOLDERID_LocalAppData)
}

/// `%ProgramData%`, read from Windows and not from the environment (which the caller controls).
pub fn program_data() -> Result<PathBuf, String> {
    known_folder(&FOLDERID_ProgramData)
}

/// `C:\Windows`.
pub fn windows_dir() -> Result<PathBuf, String> {
    known_folder(&FOLDERID_Windows)
}

/// `C:\Windows\System32`, where `pnputil.exe` is.
pub fn system32() -> Result<PathBuf, String> {
    known_folder(&FOLDERID_System)
}

/// Junction, symbolic link or other reparse point. A path that cannot be read counts as one.
pub fn is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
        .unwrap_or(true)
}

struct Descriptor {
    sd: PSECURITY_DESCRIPTOR,
    owner: PSID,
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by GetNamedSecurityInfoW, freed once with LocalFree.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.sd.0)));
        }
    }
}

fn descriptor(path: &Path) -> Result<Descriptor, String> {
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: out pointers are valid; `owner` points into `sd`, which outlives it in `Descriptor`.
    let result = unsafe {
        GetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        )
    };
    if result.is_err() {
        return Err(format!(
            "{}: security unreadable ({})",
            path.display(),
            result.0
        ));
    }
    Ok(Descriptor { sd, owner })
}

/// Owner SID (`S-1-5-32-544`…) of a file or folder.
pub fn owner_sid(path: &Path) -> Result<String, String> {
    let d = descriptor(path)?;
    let mut text = PWSTR::null();
    // SAFETY: `d.owner` is valid while `d` lives; the string is freed with LocalFree.
    unsafe {
        ConvertSidToStringSidW(d.owner, &mut text).map_err(|e| e.to_string())?;
        let sid = text.to_string().map_err(|e| e.to_string());
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// Can this (non-elevated) process change, replace or delete `path`? Asked to Windows with the
/// process's own token (`AccessCheck`), so every group of the user counts, and the Administrators
/// group of a UAC-filtered token does not. Unreadable = writable (the prudent answer).
pub fn writable_by_this_user(path: &Path) -> bool {
    check_write(path).unwrap_or(true)
}

fn check_write(path: &Path) -> Result<bool, String> {
    let d = descriptor(path)?;
    // SAFETY: plain token calls on handles owned and closed here.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| e.to_string())?;
        let mut impersonation = HANDLE::default();
        let dup = DuplicateToken(token, SecurityImpersonation, &mut impersonation);
        let _ = CloseHandle(token);
        dup.map_err(|e| e.to_string())?;
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ.0,
            GenericWrite: FILE_GENERIC_WRITE.0,
            GenericExecute: FILE_GENERIC_EXECUTE.0,
            GenericAll: FILE_ALL_ACCESS.0,
        };
        let mut privileges = PRIVILEGE_SET::default();
        let mut length = u32::try_from(size_of::<PRIVILEGE_SET>()).unwrap_or(0);
        let mut granted = 0u32;
        let mut status = windows::core::BOOL::default();
        let checked = AccessCheck(
            d.sd,
            impersonation,
            MAXIMUM_ALLOWED,
            &mapping,
            Some(&mut privileges),
            &mut length,
            &mut granted,
            &mut status,
        );
        let _ = CloseHandle(impersonation);
        checked.map_err(|e| e.to_string())?;
        let write = FILE_WRITE_DATA.0
            | FILE_APPEND_DATA.0
            | FILE_ADD_FILE.0
            | FILE_ADD_SUBDIRECTORY.0
            | FILE_DELETE_CHILD.0
            | FILE_WRITE_ATTRIBUTES.0
            | DELETE.0
            | WRITE_DAC.0
            | WRITE_OWNER.0;
        Ok(status.as_bool() && granted & write != 0)
    }
}
