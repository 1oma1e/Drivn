//! The real [`SystemActions`]: Windows Update Agent, System Restore, `pnputil`, `DiRollbackDriver`,
//! `SetupVerifyInfFile`. Runs elevated. Every change here is reached only through the pass of
//! `drivn-core`, after its checks; a simulated pass wraps this in `Simulated` (R-TST-01).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use drivn_core::install::actions::{
    ChangeOutcome, InstallOutcome, Package, PackageError, SignerCheck, SystemActions, commands,
};
use drivn_core::install::official::Official;
use drivn_core::model::{Device, DriverOffer};
use drivn_core::scan::{DeviceSource, UpdateSource};
use drivn_windows::authenticode::check_signed_exe;
use drivn_windows::security::{is_reparse_point, system32};
use drivn_windows::{CfgMgrDevices, WindowsUpdateAgent, inf_signer, read_one};
use serde::Deserialize;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIROLLBACKDRIVER_FLAGS, DiRollbackDriver, SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList,
    SetupDiDestroyDeviceInfoList, SetupDiOpenDeviceInfoW, UPDATEDRIVERFORPLUGANDPLAYDEVICES_FLAGS,
    UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Restore::{
    BEGIN_SYSTEM_CHANGE, CANCELLED_OPERATION, DEVICE_DRIVER_INSTALL, END_SYSTEM_CHANGE,
    RESTOREPOINTINFOW, SRRemoveRestorePoint, SRSetRestorePointW, STATEMGRSTATUS,
};
use windows::Win32::System::UpdateAgent::{
    IUpdateCollection, IUpdateSession, UpdateCollection, UpdateSession, orcSucceeded,
    orcSucceededWithErrors,
};
use windows::core::{BSTR, HSTRING};
use wmi::WMIConnection;

use crate::files::Root;
use crate::package;
use drivn_installer::is_guid;

pub struct RealSystem {
    pub root: Root,
    pub pass_id: String,
    pub pnputil: PathBuf,
}

impl RealSystem {
    pub fn new(root: Root, pass_id: &str) -> Result<Self, String> {
        Ok(Self {
            root,
            pass_id: pass_id.to_owned(),
            pnputil: system32()?.join("pnputil.exe"),
        })
    }

    /// `reg.exe` of System32 with arguments built by `drivn-core`; its output when it succeeds.
    fn reg(&self, args: &[String]) -> Result<String, String> {
        let out = Command::new(system32()?.join("reg.exe"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("reg: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!("reg: exit code {:?}", out.status.code()))
        }
    }

    /// Exit code of `pnputil`, after the command line built by `drivn-core` (R-SEC-04).
    fn pnputil(&self, args: &[String]) -> Result<i32, String> {
        let status = Command::new(&self.pnputil)
            .args(args)
            .status()
            .map_err(|e| format!("pnputil: {e}"))?;
        status
            .code()
            .ok_or_else(|| "pnputil: no exit code".to_owned())
    }

    fn change(&self, args: &[String], ok_codes: &[i32]) -> Result<ChangeOutcome, String> {
        let code = self.pnputil(args)?;
        if code == commands::REBOOT_EXIT_CODE || ok_codes.contains(&code) {
            Ok(ChangeOutcome {
                reboot_required: code == commands::REBOOT_EXIT_CODE,
            })
        } else {
            Err(format!("pnputil {} : exit code {code}", args.join(" ")))
        }
    }
}

fn wide256(text: &str) -> [u16; 256] {
    let mut out = [0u16; 256];
    for (slot, unit) in out.iter_mut().take(255).zip(text.encode_utf16()) {
        *slot = unit;
    }
    out
}

fn restore_point(event: u32, kind: u32, seq: i64, description: &str) -> Result<i64, String> {
    let info = RESTOREPOINTINFOW {
        dwEventType: windows::Win32::System::Restore::RESTOREPOINTINFO_EVENT_TYPE(event),
        dwRestorePtType: windows::Win32::System::Restore::RESTOREPOINTINFO_TYPE(kind),
        llSequenceNumber: seq,
        szDescription: wide256(description),
    };
    let mut status = STATEMGRSTATUS::default();
    // SAFETY: both structures are valid and live for the call.
    let ok = unsafe { SRSetRestorePointW(&info, &mut status) };
    let code = status.nStatus;
    if ok.as_bool() {
        Ok(status.llSequenceNumber)
    } else {
        Err(format!("SRSetRestorePoint refused (code {})", code.0))
    }
}

fn install_one(update_id: &str, revision: i32) -> Result<InstallOutcome, (i32, String)> {
    if !is_guid(update_id) {
        return Err((0, "update id is not a GUID".to_owned()));
    }
    let hr = |e: windows::core::Error| (e.code().0, e.message());
    // SAFETY: plain COM calls on interfaces owned by this function.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let session: IUpdateSession =
            CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER).map_err(hr)?;
        session
            .SetClientApplicationID(&BSTR::from("Drivn"))
            .map_err(hr)?;
        let searcher = session.CreateUpdateSearcher().map_err(hr)?;
        searcher.SetOnline(VARIANT_TRUE).map_err(hr)?;
        let found = searcher
            .Search(&BSTR::from(format!(
                "UpdateID='{update_id}' and RevisionNumber={revision} and IsInstalled=0 and Type='Driver'"
            )))
            .map_err(hr)?;
        let updates = found.Updates().map_err(hr)?;
        if updates.Count().map_err(hr)? != 1 {
            return Err((0, "update not found again".to_owned()));
        }
        let update = updates.get_Item(0).map_err(hr)?;
        // Drivn never accepts a licence for the user.
        if update.EulaAccepted().map_err(hr)? == VARIANT_FALSE {
            return Err((0, "this update asks to accept a licence".to_owned()));
        }
        // One update only, so the pass can stop between two drivers (R-SEC-08).
        let one: IUpdateCollection =
            CoCreateInstance(&UpdateCollection, None, CLSCTX_INPROC_SERVER).map_err(hr)?;
        one.Add(&update).map_err(hr)?;
        let downloader = session.CreateUpdateDownloader().map_err(hr)?;
        downloader.SetUpdates(&one).map_err(hr)?;
        let downloaded = downloader.Download().map_err(hr)?;
        let code = downloaded.ResultCode().map_err(hr)?;
        if code != orcSucceeded && code != orcSucceededWithErrors {
            return Err((
                downloaded.HResult().unwrap_or(0),
                "download failed".to_owned(),
            ));
        }
        // No forcing flag exists on IUpdateInstaller; none is set (R-SEC-04).
        if update.IsDownloaded().map_err(hr)? == VARIANT_FALSE {
            return Err((
                downloaded.HResult().unwrap_or(0),
                "not downloaded".to_owned(),
            ));
        }
        let installer = session.CreateUpdateInstaller().map_err(hr)?;
        installer.SetAllowSourcePrompts(VARIANT_FALSE).map_err(hr)?;
        installer.SetUpdates(&one).map_err(hr)?;
        let result = installer.Install().map_err(hr)?;
        let one_result = result.GetUpdateResult(0).map_err(hr)?;
        let code = one_result.ResultCode().map_err(hr)?;
        Ok(InstallOutcome {
            detail: (code != orcSucceeded).then(|| format!("installation result code {}", code.0)),
            succeeded: code == orcSucceeded,
            hresult: one_result.HResult().unwrap_or(0),
            reboot_required: one_result.RebootRequired().map_err(hr)? == VARIANT_TRUE
                || result.RebootRequired().map_err(hr)? == VARIANT_TRUE,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename = "SystemRestore", rename_all = "PascalCase")]
struct RestorePointRow {
    sequence_number: Option<u32>,
    description: Option<String>,
}

impl SystemActions for RealSystem {
    /// ADR-0034: winget updates the application itself (it checks its installer's hash).
    fn upgrade_app(&mut self, id: &str) -> Result<i32, String> {
        drivn_windows::winget::upgrade(id)
    }

    fn replace_app(&mut self, id: &str) -> Result<i32, String> {
        drivn_windows::winget::replace(id)
    }

    /// R-SEC-09 (ADR-0034): the app's package is copied into this pass's own folder, where only an
    /// administrator can write; its signature is checked on that copy, which is then run silently
    /// and deleted.
    fn run_official(
        &mut self,
        pass_id: &str,
        n: usize,
        official: &Official,
        file: &str,
    ) -> Result<i32, String> {
        let source = PathBuf::from(file);
        if is_reparse_point(&source) {
            return Err("the package is a link".to_owned());
        }
        let dir = self.root.pass_dir(pass_id).join("download");
        self.root.ensure(&dir)?;
        let copy = dir.join(format!("official-{n}.exe"));
        let _ = std::fs::remove_file(&copy);
        std::fs::copy(&source, &copy).map_err(|e| format!("copy: {e}"))?;
        // Revocation checked online: a stolen certificate revoked since is refused.
        let result = check_signed_exe(&copy, official.signer, true).and_then(|()| {
            run_program(
                &copy,
                official.args,
                Duration::from_secs(official.max_minutes * 60),
            )
        });
        let _ = std::fs::remove_file(&copy);
        result
    }

    fn now(&mut self) -> i64 {
        drivn_core::time::unix_now()
    }

    fn wait(&mut self, seconds: u64) {
        std::thread::sleep(Duration::from_secs(seconds));
    }

    fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String> {
        let result = WindowsUpdateAgent { online: true }.driver_offers();
        match result.error {
            Some(error) => Err(error),
            None => Ok(result.offers),
        }
    }

    fn read_device(&mut self, instance_id: &str) -> Result<Option<Device>, String> {
        Ok(read_one(instance_id))
    }

    fn read_control(&mut self) -> Option<String> {
        self.root.read_control(&self.pass_id)
    }

    /// The list of Windows (WMI `SystemRestore`). The pass compares it before and after its own
    /// point, so a number Windows gives back for an older point is never taken as the pass's.
    fn restore_points(&mut self) -> Result<Vec<i64>, String> {
        let connection = WMIConnection::with_namespace_path("ROOT\\DEFAULT")
            .map_err(|e| format!("WMI SystemRestore: {e}"))?;
        let rows: Vec<RestorePointRow> = connection
            .raw_query("SELECT SequenceNumber FROM SystemRestore")
            .map_err(|e| format!("WMI SystemRestore: {e}"))?;
        Ok(rows
            .iter()
            .filter_map(|r| r.sequence_number.map(i64::from))
            .collect())
    }

    fn driver_signer(&mut self, inf: &str) -> SignerCheck {
        inf_signer(inf)
    }

    fn others_with_id(&mut self, id: &str, except: &str) -> Result<Vec<String>, String> {
        let id = id.to_lowercase();
        Ok(CfgMgrDevices
            .devices(false)?
            .into_iter()
            .filter(|d| !d.instance_id.eq_ignore_ascii_case(except))
            .filter(|d| {
                d.hardware_ids
                    .iter()
                    .chain(&d.compatible_ids)
                    .any(|x| x.to_lowercase() == id)
            })
            .map(|d| d.instance_id)
            .collect())
    }

    fn inf_used_elsewhere(&mut self, inf: &str, except: &[String]) -> Result<bool, String> {
        // Absent devices too: `/delete-driver /uninstall` takes the driver off them as well.
        let devices = CfgMgrDevices.devices(true)?;
        Ok(devices.iter().any(|d| {
            !except.iter().any(|e| d.instance_id.eq_ignore_ascii_case(e))
                && d.driver
                    .as_ref()
                    .and_then(|x| x.inf_path.as_deref())
                    .is_some_and(|i| i.eq_ignore_ascii_case(inf))
        }))
    }

    /// ADR-0036: a new point at every pass. Windows makes one per 24 h unless its frequency is 0:
    /// set to 0 for this call only, then the value of before is put back (removed if there was none).
    fn begin_restore_point(&mut self, description: &str) -> Result<i64, String> {
        let before = self
            .reg(&commands::frequency_query())
            .ok()
            .and_then(|out| commands::parse_frequency(&out));
        let lifted = self.reg(&commands::frequency_set(0)).is_ok();
        let made = restore_point(
            BEGIN_SYSTEM_CHANGE.0,
            DEVICE_DRIVER_INSTALL.0,
            0,
            description,
        );
        if lifted {
            let _ = before.map_or_else(
                || self.reg(&commands::frequency_delete()),
                |minutes| self.reg(&commands::frequency_set(minutes)),
            );
        }
        made
    }

    fn end_restore_point(&mut self, seq: i64, cancel: bool) -> Result<(), String> {
        let kind = if cancel {
            CANCELLED_OPERATION.0
        } else {
            DEVICE_DRIVER_INSTALL.0
        };
        restore_point(END_SYSTEM_CHANGE.0, kind, seq, "").map(|_| ())
    }

    /// R-SEC-07: removed only if Windows lists this number **and** its description is the one Drivn
    /// gives its points (`Drivn <pass>`), so a number reused by Windows never takes someone else's.
    fn remove_restore_point(&mut self, seq: i64) -> Result<(), String> {
        let number = u32::try_from(seq).map_err(|_| format!("bad restore point number {seq}"))?;
        let connection = WMIConnection::with_namespace_path("ROOT\\DEFAULT")
            .map_err(|e| format!("WMI SystemRestore: {e}"))?;
        let rows: Vec<RestorePointRow> = connection
            .raw_query(format!(
                "SELECT SequenceNumber, Description FROM SystemRestore WHERE SequenceNumber = {number}"
            ))
            .map_err(|e| format!("WMI SystemRestore: {e}"))?;
        let description = rows
            .first()
            .ok_or_else(|| format!("restore point {seq} is not listed by Windows"))?
            .description
            .clone()
            .unwrap_or_default();
        if !description.starts_with("Drivn ") {
            return Err(format!(
                "restore point {seq} is « {description} », not one of Drivn's: kept"
            ));
        }
        // SAFETY: plain call with a number.
        match unsafe { SRRemoveRestorePoint(number) } {
            0 => Ok(()),
            code => Err(format!("SRRemoveRestorePoint: error {code}")),
        }
    }

    fn export_driver(&mut self, pass_id: &str, inf: &str, n: usize) -> Result<String, String> {
        let folder = self
            .root
            .pass_dir(pass_id)
            .join("backup")
            .join(n.to_string());
        self.root.ensure(&folder)?;
        let code = self.pnputil(&commands::export_driver(inf, &folder.display().to_string()))?;
        let files = std::fs::read_dir(&folder).map(|d| d.count()).unwrap_or(0);
        if code != 0 || files == 0 {
            return Err(format!(
                "export of {inf}: exit code {code}, {files} file(s)"
            ));
        }
        Ok(folder.display().to_string())
    }

    fn install_update(&mut self, update_id: &str, revision: i32) -> InstallOutcome {
        install_one(update_id, revision).unwrap_or_else(|(hresult, why)| InstallOutcome {
            succeeded: false,
            hresult,
            reboot_required: false,
            detail: Some(why),
        })
    }

    fn rollback_driver(&mut self, instance_id: &str) -> Result<ChangeOutcome, String> {
        // SAFETY: the device info set is created, used and destroyed here.
        unsafe {
            let set = SetupDiCreateDeviceInfoList(None, None).map_err(|e| e.to_string())?;
            let mut data = SP_DEVINFO_DATA {
                cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(0),
                ..Default::default()
            };
            let mut need_reboot = windows::core::BOOL(0);
            let result =
                SetupDiOpenDeviceInfoW(set, &HSTRING::from(instance_id), None, 0, Some(&mut data))
                    .and_then(|()| {
                        DiRollbackDriver(
                            set,
                            &data,
                            None,
                            DIROLLBACKDRIVER_FLAGS(commands::ROLLBACK_FLAGS),
                            Some(&mut need_reboot),
                        )
                    });
            let _ = SetupDiDestroyDeviceInfoList(set);
            result.map_err(|e| format!("DiRollbackDriver: {e}"))?;
            Ok(ChangeOutcome {
                reboot_required: need_reboot.as_bool(),
            })
        }
    }

    fn add_driver(&mut self, backup: &str) -> Result<ChangeOutcome, String> {
        // Only `passes\<this pass>\backup\<n>` of Drivn, compared part by part.
        let folder = PathBuf::from(backup);
        let backups = self.root.pass_dir(&self.pass_id).join("backup");
        let numbered = folder
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if folder.parent() != Some(backups.as_path()) || !numbered || is_reparse_point(&folder) {
            return Err(format!("{backup} is not a backup of this pass"));
        }
        self.root.ensure(&folder)?;
        // 259 (ERROR_NO_MORE_ITEMS): added to the store, no device updated by it.
        self.change(
            &commands::add_driver(&folder.join("*.inf").display().to_string()),
            &[0, 259],
        )
    }

    fn delete_driver(&mut self, inf: &str) -> Result<ChangeOutcome, String> {
        if !drivn_core::install::plan::is_oem_inf(inf) {
            return Err(format!("{inf} is not a third-party driver"));
        }
        self.change(&commands::delete_driver(inf), &[0])
    }

    fn remove_backup(&mut self, pass_id: &str) -> Result<(), String> {
        let folder = self.root.pass_dir(pass_id).join("backup");
        if !folder.exists() {
            return Ok(());
        }
        self.root.ensure(&folder)?;
        std::fs::remove_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))
    }

    fn fetch_package(
        &mut self,
        pass_id: &str,
        n: usize,
        source: &str,
        update_id: &str,
    ) -> Result<Package, PackageError> {
        package::fetch_catalog(&self.root, pass_id, n, source, update_id)
    }

    fn install_inf(&mut self, hardware_id: &str, inf: &str) -> InstallOutcome {
        self.update_driver(hardware_id, inf, commands::INSTALL_INF_FLAGS)
    }

    fn install_inf_forced(&mut self, hardware_id: &str, inf: &str) -> InstallOutcome {
        self.update_driver(hardware_id, inf, commands::INSTALL_INF_FORCED_FLAGS)
    }

    fn clean_work(&mut self, pass_id: &str) -> Result<(), String> {
        package::clean(&self.root, pass_id)
    }
}

/// Runs `path` with `args` and waits for it; stopped after `limit`.
fn run_program(path: &Path, args: &[&str], limit: Duration) -> Result<i32, String> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("start: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.code().unwrap_or(-1));
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            return Err("the installer did not finish in time".to_owned());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

impl RealSystem {
    /// `UpdateDriverForPlugAndPlayDevicesW` on an INF of this pass; `flags` from `commands`.
    fn update_driver(&mut self, hardware_id: &str, inf: &str, flags: u32) -> InstallOutcome {
        // Only an INF extracted by this pass, under `passes\<pass>\extract\`.
        let extract = self.root.pass_dir(&self.pass_id).join("extract");
        let path = PathBuf::from(inf);
        if !path.starts_with(&extract) || is_reparse_point(&path) {
            return InstallOutcome {
                succeeded: false,
                hresult: 0,
                reboot_required: false,
                detail: Some(format!("{inf} is not an INF of this pass")),
            };
        }
        let mut reboot = windows::core::BOOL(0);
        // SAFETY: two NUL-terminated strings and a valid out BOOL; no window (R-SEC-04).
        let result = unsafe {
            UpdateDriverForPlugAndPlayDevicesW(
                None,
                &HSTRING::from(hardware_id),
                &HSTRING::from(path.as_os_str()),
                UPDATEDRIVERFORPLUGANDPLAYDEVICES_FLAGS(flags),
                Some(&mut reboot),
            )
        };
        match result {
            Ok(()) => InstallOutcome {
                succeeded: true,
                hresult: 0,
                reboot_required: reboot.as_bool(),
                detail: None,
            },
            // ERROR_NO_MORE_ITEMS: Windows keeps its better-ranked driver; the reread says so.
            Err(e) if e.code().0 == 0x8007_0103_u32.cast_signed() => InstallOutcome {
                succeeded: true,
                hresult: 0,
                reboot_required: false,
                detail: Some("Windows kept its driver (not a better match)".to_owned()),
            },
            Err(e) => InstallOutcome {
                succeeded: false,
                hresult: e.code().0,
                reboot_required: false,
                detail: Some(e.message()),
            },
        }
    }
}
