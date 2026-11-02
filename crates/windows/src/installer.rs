//! The app's side of the installer (Spec - Installation, « Le plan »): start `drivn-installer.exe`
//! through the UAC, know whether a pass runs, and when Windows last started. Changes nothing itself:
//! every change is made by the elevated installer, after its own checks (R-DRT-01).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANCELLED, HANDLE};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, INFINITE, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, w};

/// Named mutex held by the installer during a pass: one pass at a time.
pub const INSTALLER_MUTEX: &str = "Global\\Drivn-Installer";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The user said "No" to the UAC (`ERROR_CANCELLED`).
    Refused,
    Failed(String),
}

/// The installer process started by [`launch_elevated`].
pub struct Installer(isize);

impl Installer {
    /// Waits for the installer and gives its exit code.
    pub fn wait(self) -> Option<u32> {
        let handle = HANDLE(self.0 as *mut _);
        // SAFETY: the handle came from ShellExecuteEx (SEE_MASK_NOCLOSEPROCESS) and is closed once.
        unsafe {
            WaitForSingleObject(handle, INFINITE);
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code).is_ok();
            let _ = CloseHandle(handle);
            ok.then_some(code)
        }
    }
}

/// `ShellExecuteEx` with the verb `runas`: one UAC window.
pub fn launch_elevated(exe: &Path, args: &str) -> Result<Installer, LaunchError> {
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; `info` is a valid SHELLEXECUTEINFOW.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(Installer(info.hProcess.0 as isize)),
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err(LaunchError::Refused),
        Err(e) => Err(LaunchError::Failed(e.to_string())),
    }
}

/// A pass (or a rollback) is running: the installer holds its mutex.
pub fn installer_running() -> bool {
    // SAFETY: the handle, if any, is closed right away.
    match unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            false,
            &HSTRING::from(INSTALLER_MUTEX),
        )
    } {
        Ok(handle) => {
            let _ = unsafe { CloseHandle(handle) };
            true
        }
        // It exists, but this user may not open it.
        Err(e) => e.code() == ERROR_ACCESS_DENIED.to_hresult(),
    }
}

/// Unix seconds of the last start of Windows (now minus the uptime).
pub fn last_boot_unix() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // SAFETY: no arguments.
    let uptime = unsafe { GetTickCount64() } / 1000;
    i64::try_from(now.saturating_sub(uptime)).unwrap_or(0)
}
