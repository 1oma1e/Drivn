//! Every call to Windows made by a pass goes through [`SystemActions`]. Three versions: real
//! (`drivn-installer`), simulated ([`Simulated`], R-TST-01) and fake (tests).

use crate::model::{Device, DriverOffer};

/// What WUA answered for one update (`IInstallationResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub succeeded: bool,
    /// `HRESULT` of the download or the installation, 0 when it succeeded.
    pub hresult: i32,
    pub reboot_required: bool,
    /// Why it failed when WUA gave no HRESULT (licence asked, update gone, not downloaded…).
    pub detail: Option<String>,
}

/// What a change to the driver store or a device answered (`pnputil`, `DiRollbackDriver`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeOutcome {
    /// `NeedReboot`, or `pnputil` exit code 3010.
    pub reboot_required: bool,
}

/// Signature of the catalog that covers an INF of the driver store (R-SRC-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerCheck {
    Signed {
        signer: String,
    },
    Unsigned,
    /// Could not be checked: treated like unsigned (the prudent way, R-PRJ-04).
    Unknown {
        reason: String,
    },
}

/// One INF of an extracted package, read by Windows (`SetupOpenInfFile`, never a parser of Drivn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfFacts {
    /// Full path in `passes\<pass>\extract\<n>\`.
    pub path: String,
    /// IDs of its models for this architecture.
    pub hardware_ids: Vec<String>,
    pub version: Option<String>,
    /// `YYYY-MM-DD`.
    pub date: Option<String>,
    pub provider: Option<String>,
    pub class: Option<String>,
    /// The catalog that covers the INF (R-SRC-04).
    pub signer: SignerCheck,
}

/// What a package gave once downloaded, hashed, signature-checked and opened without running it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub infs: Vec<InfFacts>,
    /// Signer of the package itself (R-SRC-03), for the journal.
    pub signer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    Download(String),
    /// R-SRC-02.
    Hash,
    /// R-SRC-03.
    Signature(String),
    /// R-SRC-08: name, size or format refused.
    Archive(String),
}

pub trait SystemActions {
    /// Unix seconds.
    fn now(&mut self) -> i64;
    /// ADR-0034: updates one application with winget, silently; its exit code (0 = done).
    fn upgrade_app(&mut self, _id: &str) -> Result<i32, String> {
        Err("applications are not supported here".to_owned())
    }
    /// The same, uninstalling the old version first (`--uninstall-previous`).
    fn replace_app(&mut self, _id: &str) -> Result<i32, String> {
        Err("applications are not supported here".to_owned())
    }
    /// R-SEC-09 (ADR-0034): copies the package into `passes\<pass>\download\`, checks its
    /// signature on that copy, runs it with the silent switches; its exit code (0 = done).
    fn run_official(
        &mut self,
        _pass_id: &str,
        _n: usize,
        _official: &super::official::Official,
        _file: &str,
    ) -> Result<i32, String> {
        Err("official installers are not supported here".to_owned())
    }
    /// Waits between two reads of the control file.
    fn wait(&mut self, seconds: u64);

    // Reads: done the same way in simulation.
    /// Windows Update search made by the installer itself, during this pass (R-SEC-05).
    fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String>;
    /// The device as it is now; `None` when Windows does not know it.
    fn read_device(&mut self, instance_id: &str) -> Result<Option<Device>, String>;
    /// Raw content of the control file, `None` when there is none (or it is refused).
    fn read_control(&mut self) -> Option<String>;
    /// Sequence numbers of the restore points Windows lists now.
    fn restore_points(&mut self) -> Result<Vec<i64>, String>;
    fn driver_signer(&mut self, inf: &str) -> SignerCheck;
    /// A device other than `except` (this one and its twins) uses this INF: then it is never deleted.
    fn inf_used_elsewhere(&mut self, inf: &str, except: &[String]) -> Result<bool, String>;
    /// Present devices other than `except_instance_id` with `id` among their hardware or
    /// compatible IDs (case ignored): Windows Update would install the offer on them too.
    fn others_with_id(&mut self, id: &str, except_instance_id: &str)
    -> Result<Vec<String>, String>;

    // Changes: never called by a simulated pass.
    /// `SRSetRestorePoint(BEGIN_SYSTEM_CHANGE, DEVICE_DRIVER_INSTALL)`; the sequence number.
    fn begin_restore_point(&mut self, description: &str) -> Result<i64, String>;
    /// `END_SYSTEM_CHANGE`, or `CANCELLED_OPERATION` (which removes the point) when `cancel`.
    fn end_restore_point(&mut self, seq: i64, cancel: bool) -> Result<(), String>;
    fn remove_restore_point(&mut self, seq: i64) -> Result<(), String>;
    /// `pnputil /export-driver <inf> <backup\n>`; the backup folder, checked not empty.
    fn export_driver(&mut self, pass_id: &str, inf: &str, n: usize) -> Result<String, String>;
    /// WUA download then install, one update only in the collection, no forcing flag (R-SEC-04).
    fn install_update(&mut self, update_id: &str, revision: i32) -> InstallOutcome;
    /// `DiRollbackDriver` without `DIIRFLAG_FORCE_INF`.
    fn rollback_driver(&mut self, instance_id: &str) -> Result<ChangeOutcome, String>;
    /// `pnputil /add-driver <backup>\*.inf /install`.
    fn add_driver(&mut self, backup: &str) -> Result<ChangeOutcome, String>;
    /// `pnputil /delete-driver <oem#.inf> /uninstall`, never `/force`.
    fn delete_driver(&mut self, inf: &str) -> Result<ChangeOutcome, String>;
    /// Deletes `passes\<pass>\backup\` (R-SEC-07).
    fn remove_backup(&mut self, pass_id: &str) -> Result<(), String>;

    // Phase 3: packages Drivn downloads itself. Reads (done in simulation too) then one change.
    /// Asks the source again for `update_id` (never a URL from the plan, R-SEC-05), downloads to
    /// `passes\<pass>\download\<n>\` (an old folder is deleted first, R-SEC-06), compares the
    /// hash (R-SRC-02), checks the signature (R-SRC-03), opens it without running anything into
    /// `extract\<n>\` (R-SRC-08) and reads its INFs.
    fn fetch_package(
        &mut self,
        _pass_id: &str,
        _n: usize,
        _source: &str,
        _update_id: &str,
    ) -> Result<Package, PackageError> {
        Err(PackageError::Download("not supported here".to_owned()))
    }
    /// `UpdateDriverForPlugAndPlayDevicesW(id, inf, INSTALLFLAG_NONINTERACTIVE)`, without
    /// `INSTALLFLAG_FORCE` (R-SEC-04). "Not better for Windows" succeeds with the old driver kept.
    fn install_inf(&mut self, _hardware_id: &str, _inf: &str) -> InstallOutcome {
        InstallOutcome {
            succeeded: false,
            hresult: 0,
            reboot_required: false,
            detail: Some("not supported here".to_owned()),
        }
    }
    /// R-SEC-04 (ADR-0034): the same with `INSTALLFLAG_FORCE`, only for an INF the pass checked
    /// newer and signed, after Windows kept its own driver.
    fn install_inf_forced(&mut self, _hardware_id: &str, _inf: &str) -> InstallOutcome {
        InstallOutcome {
            succeeded: false,
            hresult: 0,
            reboot_required: false,
            detail: Some("not supported here".to_owned()),
        }
    }
    /// Deletes `download\` and `extract\` of a pass at its end (Drivn's own files).
    fn clean_work(&mut self, _pass_id: &str) -> Result<(), String> {
        Ok(())
    }
}

/// The command lines the real system runs. Built here so R-SEC-04 is tested on them.
pub mod commands {
    pub fn export_driver(inf: &str, folder: &str) -> Vec<String> {
        vec!["/export-driver".into(), inf.into(), folder.into()]
    }

    pub fn add_driver(inf_pattern: &str) -> Vec<String> {
        vec!["/add-driver".into(), inf_pattern.into(), "/install".into()]
    }

    pub fn delete_driver(inf: &str) -> Vec<String> {
        vec!["/delete-driver".into(), inf.into(), "/uninstall".into()]
    }

    /// `UpdateDriverForPlugAndPlayDevicesW` flags of the first try: `INSTALLFLAG_NONINTERACTIVE`
    /// (0x4) only, without `INSTALLFLAG_FORCE` (0x1) (R-SEC-04).
    pub const INSTALL_INF_FLAGS: u32 = 0x4;
    /// The second try, for a newer and signed INF that Windows ranked lower (R-SEC-04, ADR-0034).
    pub const INSTALL_INF_FORCED_FLAGS: u32 = 0x4 | 0x1;

    /// `DiRollbackDriver` flags: 0, never `DIIRFLAG_FORCE_INF` (0x2).
    pub const ROLLBACK_FLAGS: u32 = 0;

    /// ADR-0036: Windows makes one restore point per 24 h unless this value is 0 (minutes,
    /// documented with `SRSetRestorePoint`). Set to 0 only around the creation, then put back.
    pub const RESTORE_FREQUENCY_KEY: &str =
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore";
    pub const RESTORE_FREQUENCY_VALUE: &str = "SystemRestorePointCreationFrequency";

    /// `reg.exe` arguments: read the value.
    pub fn frequency_query() -> Vec<String> {
        vec![
            "query".into(),
            RESTORE_FREQUENCY_KEY.into(),
            "/v".into(),
            RESTORE_FREQUENCY_VALUE.into(),
        ]
    }

    /// `reg.exe` arguments: set the value (a DWORD of minutes).
    pub fn frequency_set(minutes: u32) -> Vec<String> {
        vec![
            "add".into(),
            RESTORE_FREQUENCY_KEY.into(),
            "/v".into(),
            RESTORE_FREQUENCY_VALUE.into(),
            "/t".into(),
            "REG_DWORD".into(),
            "/d".into(),
            minutes.to_string(),
            "/f".into(),
        ]
    }

    /// `reg.exe` arguments: remove the value (it did not exist before).
    pub fn frequency_delete() -> Vec<String> {
        vec![
            "delete".into(),
            RESTORE_FREQUENCY_KEY.into(),
            "/v".into(),
            RESTORE_FREQUENCY_VALUE.into(),
            "/f".into(),
        ]
    }

    /// The minutes in the output of `reg query` (`… REG_DWORD 0x5a0`); `None`: no such value.
    pub fn parse_frequency(out: &str) -> Option<u32> {
        out.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != RESTORE_FREQUENCY_VALUE || words.next()? != "REG_DWORD" {
                return None;
            }
            u32::from_str_radix(words.next()?.trim_start_matches("0x"), 16).ok()
        })
    }

    /// `pnputil` exit code for "done, restart needed".
    pub const REBOOT_EXIT_CODE: i32 = 3010;
}

/// R-TST-01: the same pass on the same system, minus every call that changes it. A change asked
/// by mistake is refused here, so a bug in the pass cannot install anything in simulation.
pub struct Simulated<S>(pub S);

const SIMULATION: &str = "simulation: nothing is changed";

impl<S: SystemActions> SystemActions for Simulated<S> {
    /// R-TST-01: same path, the final call is not made.
    fn upgrade_app(&mut self, _: &str) -> Result<i32, String> {
        Ok(0)
    }
    fn replace_app(&mut self, _: &str) -> Result<i32, String> {
        Ok(0)
    }
    fn run_official(
        &mut self,
        _: &str,
        _: usize,
        _: &super::official::Official,
        _: &str,
    ) -> Result<i32, String> {
        Ok(0)
    }
    fn now(&mut self) -> i64 {
        self.0.now()
    }
    fn wait(&mut self, seconds: u64) {
        self.0.wait(seconds);
    }
    fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String> {
        self.0.search_offers()
    }
    fn read_device(&mut self, instance_id: &str) -> Result<Option<Device>, String> {
        self.0.read_device(instance_id)
    }
    fn read_control(&mut self) -> Option<String> {
        self.0.read_control()
    }
    fn restore_points(&mut self) -> Result<Vec<i64>, String> {
        self.0.restore_points()
    }
    fn driver_signer(&mut self, inf: &str) -> SignerCheck {
        self.0.driver_signer(inf)
    }
    fn inf_used_elsewhere(&mut self, inf: &str, except: &[String]) -> Result<bool, String> {
        self.0.inf_used_elsewhere(inf, except)
    }
    fn others_with_id(
        &mut self,
        id: &str,
        except_instance_id: &str,
    ) -> Result<Vec<String>, String> {
        self.0.others_with_id(id, except_instance_id)
    }
    fn begin_restore_point(&mut self, _: &str) -> Result<i64, String> {
        Err(SIMULATION.to_owned())
    }
    fn end_restore_point(&mut self, _: i64, _: bool) -> Result<(), String> {
        Err(SIMULATION.to_owned())
    }
    fn remove_restore_point(&mut self, _: i64) -> Result<(), String> {
        Err(SIMULATION.to_owned())
    }
    fn export_driver(&mut self, _: &str, _: &str, _: usize) -> Result<String, String> {
        Err(SIMULATION.to_owned())
    }
    fn install_update(&mut self, _: &str, _: i32) -> InstallOutcome {
        InstallOutcome {
            succeeded: false,
            hresult: 0,
            reboot_required: false,
            detail: Some(SIMULATION.to_owned()),
        }
    }
    fn rollback_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Err(SIMULATION.to_owned())
    }
    fn add_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Err(SIMULATION.to_owned())
    }
    fn delete_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Err(SIMULATION.to_owned())
    }
    fn remove_backup(&mut self, _: &str) -> Result<(), String> {
        Err(SIMULATION.to_owned())
    }
    fn fetch_package(
        &mut self,
        pass_id: &str,
        n: usize,
        source: &str,
        update_id: &str,
    ) -> Result<Package, PackageError> {
        self.0.fetch_package(pass_id, n, source, update_id)
    }
    fn install_inf(&mut self, _: &str, _: &str) -> InstallOutcome {
        InstallOutcome {
            succeeded: false,
            hresult: 0,
            reboot_required: false,
            detail: Some(SIMULATION.to_owned()),
        }
    }
    fn install_inf_forced(&mut self, _: &str, _: &str) -> InstallOutcome {
        InstallOutcome {
            succeeded: false,
            hresult: 0,
            reboot_required: false,
            detail: Some(SIMULATION.to_owned()),
        }
    }
    fn clean_work(&mut self, pass_id: &str) -> Result<(), String> {
        self.0.clean_work(pass_id)
    }
}
