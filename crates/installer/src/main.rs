//! `drivn-installer.exe`, the only elevated part of Drivn (ADR-0006, R-DRT-01). One launch = one
//! UAC = one of:
//!   --pass <passe> --sha256 <hash>      a pass from the plan the app wrote
//!   --resume <passe> [--stop keep|revert]   a pass cut by a crash (R-SEC-06)
//!   --rollback <passe> <n>              one driver put back
//! It takes no path: it builds every path itself from a checked pass ID (Spec - Installation).
#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(windows, allow(unsafe_code))]

#[cfg(windows)]
mod files;
#[cfg(windows)]
mod package;
#[cfg(windows)]
mod system;

use std::process::ExitCode;

use drivn_installer::{exit, parse_args};

#[cfg(windows)]
mod run {
    use drivn_core::install::actions::{Simulated, SystemActions};
    use drivn_core::install::journal::reconstruct;
    use std::io::Read;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    use drivn_core::install::plan::{MAX_PLAN_BYTES, Plan};
    use drivn_core::install::retention::prune;
    use drivn_core::install::run::{Config, resume_pass, rollback_one, run_apps, run_pass};
    use drivn_core::install::setup::{PlanFile, check_before_copy, check_copy, simulation};
    use drivn_windows::installer::{INSTALLER_MUTEX, last_boot_unix};
    use drivn_windows::security::is_reparse_point;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    use windows::Win32::System::Com::{
        COINIT_MULTITHREADED, CoInitializeEx, CoInitializeSecurity, EOAC_NONE,
        RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::HSTRING;

    use crate::files::{FileJournal, Passes, Root};
    use crate::system::RealSystem;
    use drivn_installer::Task;

    pub struct Mutex(HANDLE);

    impl Drop for Mutex {
        fn drop(&mut self) {
            // SAFETY: handle from CreateMutexW, closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// One pass at a time (`Global\Drivn-Installer`). `None`: another one runs.
    pub fn single() -> Option<Mutex> {
        // SAFETY: named mutex; GetLastError is read right after the call, as documented.
        unsafe {
            let handle = CreateMutexW(None, true, &HSTRING::from(INSTALLER_MUTEX)).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Mutex(handle))
        }
    }

    pub fn com() {
        // SAFETY: called once, first thing, before any COM object exists. WMI needs impersonation.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            );
        }
    }

    /// Step 2 of the spec. The plan the app wrote is read through **one** handle (link test, size
    /// and content of the same file), its hash checked in memory, and only then copied into the
    /// pass folder: a file the app did not write never reaches a folder Users can read.
    fn take_plan(root: &Root, pass_id: &str, sha256: &str) -> Result<Plan, String> {
        let request = root.plan_request(pass_id);
        let pass_dir = root.pass_dir(pass_id);
        root.ensure(&root.passes())?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&request)
            .map_err(|e| format!("plan {}: {e}", request.display()))?;
        let meta = file.metadata().map_err(|e| format!("plan: {e}"))?;
        let facts = PlanFile {
            reparse_point: meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
                || !meta.is_file(),
            size: meta.len(),
        };
        check_before_copy(pass_id, pass_dir.exists(), &facts)?;
        let mut bytes = Vec::new();
        file.take(MAX_PLAN_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("plan: {e}"))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PLAN_BYTES {
            return Err("the plan file is too large".to_owned());
        }
        check_copy(pass_id, sha256, &bytes)?;
        root.ensure(&pass_dir)?;
        let copy = pass_dir.join("plan.json");
        std::fs::write(&copy, &bytes).map_err(|e| format!("plan copy: {e}"))?;
        // From here on, only the copy is read.
        let copied = std::fs::read(&copy).map_err(|e| format!("plan copy: {e}"))?;
        check_copy(pass_id, sha256, &copied)
    }

    fn read_plan(root: &Root, pass_id: &str) -> Result<Plan, String> {
        let copy = root.pass_dir(pass_id).join("plan.json");
        root.ensure(&root.pass_dir(pass_id))?;
        let bytes = std::fs::read(&copy).map_err(|e| format!("plan copy: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("plan copy: {e}"))
    }

    fn read_view(
        root: &Root,
        pass_id: &str,
    ) -> Result<drivn_core::install::journal::PassView, String> {
        let path = root.pass_dir(pass_id).join("journal.jsonl");
        if is_reparse_point(&path) {
            return Err("the journal is a link".to_owned());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("journal: {e}"))?;
        reconstruct(&text)
    }

    pub fn task(task: Task) -> Result<(), String> {
        let root = Root::locate()?;
        root.ensure(&root.drivn)?;
        let config = Config::default();
        match task {
            Task::Pass { pass_id, sha256 } => {
                let plan = take_plan(&root, &pass_id, &sha256)?;
                let sim = simulation(plan.mode, root.real_mode());
                let mut journal = FileJournal::new(&root, &pass_id);
                let real = RealSystem::new(Root::locate()?, &pass_id)?;
                // ADR-0034: the applications right after the drivers, under the same UAC.
                if sim {
                    let mut sys = Simulated(real);
                    run_pass(&plan, true, &mut sys, &mut journal, &config)?;
                    run_apps(&plan, true, &mut sys, &mut journal)?;
                } else {
                    let mut real = real;
                    run_pass(&plan, false, &mut real, &mut journal, &config)?;
                    run_apps(&plan, false, &mut real, &mut journal)?;
                    prune(&mut Passes(&root), &mut real, &pass_id);
                }
            }
            Task::Resume { pass_id, stop } => {
                let plan = read_plan(&root, &pass_id)?;
                let view = read_view(&root, &pass_id)?;
                if !view.interrupted() {
                    return Err("this pass is not interrupted".to_owned());
                }
                real_allowed(&root, &view)?;
                let mut journal = FileJournal::new(&root, &pass_id);
                let real = RealSystem::new(Root::locate()?, &pass_id)?;
                if view.simulation {
                    resume_pass(
                        &plan,
                        &view,
                        stop,
                        &mut Simulated(real),
                        &mut journal,
                        &config,
                    )?;
                } else {
                    let mut real = real;
                    resume_pass(&plan, &view, stop, &mut real, &mut journal, &config)?;
                    prune(&mut Passes(&root), &mut real, &pass_id);
                }
            }
            Task::Rollback { pass_id, n } => {
                let view = read_view(&root, &pass_id)?;
                real_allowed(&root, &view)?;
                let mut journal = FileJournal::new(&root, &pass_id);
                let real = RealSystem::new(Root::locate()?, &pass_id)?;
                let mut sys: Box<dyn SystemActions> = if view.simulation {
                    Box::new(Simulated(real))
                } else {
                    Box::new(real)
                };
                rollback_one(&view, n, last_boot_unix(), sys.as_mut(), &mut journal)?;
            }
        }
        Ok(())
    }

    /// R-TST-01, second barrier: a real resume or rollback also needs `real-mode` now.
    fn real_allowed(
        root: &Root,
        view: &drivn_core::install::journal::PassView,
    ) -> Result<(), String> {
        if view.simulation || root.real_mode() {
            Ok(())
        } else {
            Err("real-mode is missing: real changes are off on this PC".to_owned())
        }
    }

    /// Best effort, one file per pass: the app shows it when the installer stopped with an error.
    pub fn note_error(pass_id: &str, message: &str) {
        let Ok(root) = Root::locate() else { return };
        let dir = root.drivn.join("errors");
        if root.ensure(&dir).is_ok() {
            let _ = std::fs::write(dir.join(format!("{pass_id}.txt")), message);
        }
    }
}

#[cfg(windows)]
fn main() -> ExitCode {
    drivn_windows::security::harden_dll_search();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = match parse_args(&args) {
        Ok(task) => task,
        Err(e) => {
            eprintln!("drivn-installer: {e}");
            return ExitCode::from(exit::BAD_ARGS);
        }
    };
    run::com();
    // Checked by parse_args: safe as a file name.
    let pass_id = task.pass_id().to_owned();
    let Some(_mutex) = run::single() else {
        run::note_error(&pass_id, "another pass is running");
        return ExitCode::from(exit::ALREADY_RUNNING);
    };
    match run::task(task) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            run::note_error(&pass_id, &e);
            ExitCode::from(exit::REFUSED)
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("drivn-installer runs on Windows only");
    ExitCode::from(exit::BAD_ARGS)
}
