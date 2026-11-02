//! Pass journal (`journal.jsonl`): one JSON line per step, written before and after, flushed at
//! each line. The app follows a pass by reading it, and an interrupted pass is rebuilt from it
//! (R-SEC-03, R-SEC-06). Never holds the content of an invalid order (R-SEC-08).

use serde::{Deserialize, Serialize};

use super::control::StopMode;
use super::plan::DriverFacts;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Unix seconds.
    pub at: i64,
    pub simulation: bool,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Verify,
    Backup,
    Install,
    Reread,
    Signature,
    /// R-SEC-04 (ADR-0034): the newer, signed INF installed again with `INSTALLFLAG_FORCE`.
    Force,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Restore {
    Created {
        seq: i64,
    },
    Failed {
        reason: String,
        /// Windows gave back the number of a point it already had (one per 24 h): older than the
        /// pass, so it also puts everything back, but it is not Drivn's (never removed, R-SEC-07).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        existing: Option<i64>,
    },
    /// A simulated pass makes no restore point.
    Simulated,
    /// R-SEC-01 (ADR-0036): the user answered "no" before the pass.
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Installed,
    InstalledPendingReboot,
    NotSelectedByWindows,
    Failed,
    RolledBackUnsigned,
    /// Signature refused and the rollback failed: shown as an alert, with the restore point.
    UnsignedNotReverted,
    Skipped,
    /// Simulation: checked again, nothing installed.
    Simulated,
    /// Read again after an interruption: the old driver is still active.
    NotInstalled,
    /// Read again after an interruption: neither the old nor the new driver.
    Unknown,
    Reverted,
    RevertedPendingReboot,
}

impl Status {
    /// The new driver is (or will be after a restart) on the device.
    pub fn installed(self) -> bool {
        matches!(
            self,
            Self::Installed
                | Self::InstalledPendingReboot
                | Self::UnsignedNotReverted
                | Self::Unknown
        )
    }

    /// Nothing reached the system for this driver: only then may the restore point be cancelled.
    pub fn untouched(self) -> bool {
        matches!(self, Self::Skipped | Self::Simulated | Self::NotInstalled)
    }

    pub fn pending_reboot(self) -> bool {
        matches!(
            self,
            Self::InstalledPendingReboot | Self::RevertedPendingReboot
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skip {
    Source,
    OfferGone,
    DeviceAbsent,
    IdMismatch,
    DriverChanged,
    NoLongerNewer,
    RiskyNotChecked,
    WindowsDriver,
    /// Another device has the offer's ID: Windows Update would change it too, without backup.
    SharedId,
    BackupFailed,
    ReadFailed,
    // Packages Drivn downloads itself (Spec - Sources constructeurs, « Installeur »).
    DownloadFailed,
    /// R-SRC-02: the file's hash differs from the source's.
    RefusedHash,
    /// R-SRC-03: unsigned, bad signature or unexpected signer.
    RefusedSignature,
    ExtractFailed,
    /// R-SRC-08: no INF for this device's ID, from the expected provider.
    NoMatchingInf,
    /// The only INF ID equal to the device's is a Compatible ID: too many devices (R-SEC-02).
    CompatibleIdOnly,
    /// R-SRC-04: the INF's catalog is unsigned or not from the expected signer.
    RefusedCatalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The device no longer uses the driver this pass installed.
    Changed,
    RebootFirst,
    /// The new INF also serves another device.
    InfShared,
    NothingInstalled,
    /// Backups of this pass were removed (R-SEC-07).
    Expired,
}

/// Everything needed to put this one driver back (R-SEC-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRecord {
    pub n: usize,
    pub device_instance_id: String,
    pub name: Option<String>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<Skip>,
    /// Driver before the pass (`None`: the device had none).
    pub old: Option<DriverFacts>,
    /// The offer, then the driver read on the device after the installation.
    pub new: DriverFacts,
    pub backup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hresult: Option<i32>,
    pub reboot_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    /// Status found by reading the device again after an interruption (R-SEC-06).
    #[serde(default)]
    pub after_interruption: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Other devices with the offer's ID and the same old driver, updated with it (R-SEC-02).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub twins: Vec<String>,
    /// R-SEC-02 (ADR-0034): installed without a Drivn backup (export failed, driver of Windows).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_backup: bool,
    /// R-SEC-04 (ADR-0034): installed again with `INSTALLFLAG_FORCE` after Windows kept its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    PassStart {
        pass_id: String,
        items: usize,
    },
    /// `download\` or `extract\` of the pass could not be deleted at its end.
    WorkNotCleaned {
        error: String,
    },
    OffersSearched {
        count: usize,
        error: Option<String>,
    },
    RestoreStart,
    RestoreDone {
        restore: Restore,
    },
    WaitingRestoreConsent,
    ContinueNoRestore,
    InvalidOrder,
    Paused,
    Resumed,
    StopRequested {
        mode: StopMode,
    },
    /// No order for 24 h while waiting: the pass stops as `cancel_keep`.
    WaitTimeout,
    ItemStart {
        n: usize,
        device_instance_id: String,
        name: Option<String>,
        old: Option<DriverFacts>,
        offer: DriverFacts,
    },
    StepStart {
        n: usize,
        step: Step,
    },
    StepEnd {
        n: usize,
        step: Step,
        ok: bool,
        /// Backup step: the backup folder.
        detail: Option<String>,
    },
    ItemEnd {
        record: ItemRecord,
    },
    RevertStart {
        n: usize,
    },
    RevertEnd {
        n: usize,
        status: Option<Status>,
        refusal: Option<Refusal>,
        detail: Option<String>,
    },
    RestoreEnd {
        seq: i64,
        cancelled: bool,
        error: Option<String>,
    },
    Resume {
        stop: Option<StopMode>,
    },
    PassEnd {
        not_started: usize,
        stopped: Option<StopMode>,
    },
    /// R-SEC-07: backups and restore point of this pass removed (or the try failed).
    Pruned {
        ok: bool,
        error: Option<String>,
    },
    /// R-SEC-07 (ADR-0036): this pass's restore point removed, a newer one being the 4th of Drivn.
    PointRemoved {
        seq: i64,
        ok: bool,
        error: Option<String>,
    },
    /// ADR-0034: an application started by winget, after the drivers.
    AppStart {
        n: usize,
        id: String,
        name: String,
    },
    AppEnd {
        n: usize,
        id: String,
        status: AppStatus,
        /// winget's exit code, or why it could not run.
        detail: Option<String>,
    },
    /// R-BUT-01: the busy applications (`n` of their `AppStart`) are tried again after a wait.
    AppsRetry {
        ns: Vec<usize>,
        seconds: u64,
    },
    /// R-SEC-09 (ADR-0034): a publisher's installer started silently, after the drivers.
    PackageStart {
        n: usize,
        device_instance_id: String,
        name: String,
    },
    PackageEnd {
        n: usize,
        status: AppStatus,
        /// The installer's exit code, or why it was not run.
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    Updated,
    Simulated,
    Failed,
}

/// One application of the pass as its journal tells it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppRecord {
    pub n: usize,
    pub id: String,
    pub name: String,
    /// `None`: started, not finished.
    pub status: Option<AppStatus>,
    pub detail: Option<String>,
}

/// A driver started and not finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Started {
    pub n: usize,
    pub device_instance_id: String,
    pub name: Option<String>,
    pub old: Option<DriverFacts>,
    pub offer: DriverFacts,
    pub step: Option<Step>,
    pub backup: Option<String>,
}

/// A pass as its journal tells it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct PassView {
    pub pass_id: String,
    pub simulation: bool,
    pub planned: usize,
    pub started_at: i64,
    pub restore: Option<Restore>,
    /// The user chose to go on without a restore point.
    pub no_restore_chosen: bool,
    pub waiting_consent: bool,
    /// `BEGIN_SYSTEM_CHANGE` noted and not ended.
    pub restore_open: Option<i64>,
    /// `CANCELLED_OPERATION` was called: this number must never be removed.
    pub restore_cancelled: bool,
    /// Its restore point was removed (or found already gone) to keep 5 (ADR-0038).
    pub point_removed: bool,
    pub paused: bool,
    pub stop: Option<StopMode>,
    /// Finished drivers, latest status (a revert updates it).
    pub items: Vec<ItemRecord>,
    pub started: Option<Started>,
    pub reverting: Option<usize>,
    pub ended_at: Option<i64>,
    pub pruned: bool,
    pub resumed: bool,
    /// Applications of the pass (ADR-0034), started or finished.
    pub apps: Vec<AppRecord>,
    /// Applications waiting for their second try.
    pub retrying: Vec<usize>,
    /// Publishers' installers of the pass (R-SEC-09); `id` is the device's instance ID.
    pub packages: Vec<AppRecord>,
}

impl PassView {
    pub fn ended(&self) -> bool {
        self.ended_at.is_some()
    }

    /// No `pass_end`, or a revert started and not finished (R-SEC-06).
    pub fn interrupted(&self) -> bool {
        !self.ended() || self.reverting.is_some()
    }

    /// Highest driver number seen (finished or started).
    pub fn last_n(&self) -> usize {
        let finished = self.items.iter().map(|i| i.n).max().unwrap_or(0);
        finished.max(self.started.as_ref().map_or(0, |s| s.n))
    }

    pub fn restore_seq(&self) -> Option<i64> {
        match self.restore {
            Some(Restore::Created { seq }) if !self.restore_cancelled && !self.point_removed => {
                Some(seq)
            }
            _ => None,
        }
    }

    fn apply(&mut self, line: Line) {
        match line.event {
            Event::AppStart { n, id, name } => self.apps.push(AppRecord {
                n,
                id,
                name,
                status: None,
                detail: None,
            }),
            Event::AppsRetry { ns, .. } => self.retrying = ns,
            Event::AppEnd {
                n, status, detail, ..
            } => {
                self.retrying.retain(|r| *r != n);
                if let Some(app) = self.apps.iter_mut().find(|a| a.n == n) {
                    app.status = Some(status);
                    app.detail = detail;
                }
            }
            Event::PackageStart {
                n,
                device_instance_id,
                name,
            } => self.packages.push(AppRecord {
                n,
                id: device_instance_id,
                name,
                status: None,
                detail: None,
            }),
            Event::PackageEnd { n, status, detail } => {
                if let Some(p) = self.packages.iter_mut().find(|p| p.n == n) {
                    p.status = Some(status);
                    p.detail = detail;
                }
            }
            Event::PassStart { pass_id, items } => {
                self.pass_id = pass_id;
                self.planned = items;
                self.started_at = line.at;
                self.simulation = line.simulation;
            }
            Event::RestoreDone { restore } => {
                if let Restore::Created { seq } = restore {
                    self.restore_open = Some(seq);
                }
                self.restore = Some(restore);
            }
            Event::WaitingRestoreConsent => self.waiting_consent = true,
            Event::ContinueNoRestore => {
                self.waiting_consent = false;
                self.no_restore_chosen = true;
            }
            Event::Paused => self.paused = true,
            Event::Resumed => self.paused = false,
            Event::StopRequested { mode } => {
                self.stop = Some(mode);
                self.paused = false;
                self.waiting_consent = false;
            }
            Event::WaitTimeout => {
                self.stop = Some(StopMode::Keep);
                self.paused = false;
                self.waiting_consent = false;
            }
            Event::ItemStart {
                n,
                device_instance_id,
                name,
                old,
                offer,
            } => {
                self.started = Some(Started {
                    n,
                    device_instance_id,
                    name,
                    old,
                    offer,
                    step: None,
                    backup: None,
                });
            }
            Event::StepStart { n, step } => {
                if let Some(s) = self.started.as_mut().filter(|s| s.n == n) {
                    s.step = Some(step);
                }
            }
            Event::StepEnd {
                n,
                step,
                ok,
                detail,
            } => {
                if let Some(s) = self.started.as_mut().filter(|s| s.n == n)
                    && step == Step::Backup
                    && ok
                {
                    s.backup = detail;
                }
            }
            Event::ItemEnd { record } => {
                if self.started.as_ref().is_some_and(|s| s.n == record.n) {
                    self.started = None;
                }
                match self.items.iter_mut().find(|i| i.n == record.n) {
                    Some(existing) => *existing = record,
                    None => self.items.push(record),
                }
            }
            Event::RevertStart { n } => self.reverting = Some(n),
            Event::RevertEnd { n, status, .. } => {
                if self.reverting == Some(n) {
                    self.reverting = None;
                }
                if let (Some(status), Some(item)) =
                    (status, self.items.iter_mut().find(|i| i.n == n))
                {
                    item.status = status;
                }
            }
            Event::RestoreEnd { cancelled, .. } => {
                self.restore_open = None;
                self.restore_cancelled |= cancelled;
            }
            Event::Resume { stop } => {
                self.resumed = true;
                self.ended_at = None;
                if stop.is_some() {
                    self.stop = stop;
                }
            }
            Event::PassEnd { .. } => {
                self.ended_at = Some(line.at);
                self.paused = false;
                self.waiting_consent = false;
            }
            Event::Pruned { ok, .. } => self.pruned |= ok,
            Event::PointRemoved { ok, .. } => self.point_removed |= ok,
            Event::OffersSearched { .. }
            | Event::WorkNotCleaned { .. }
            | Event::RestoreStart
            | Event::InvalidOrder => {}
        }
    }
}

/// Parses a journal. A last line cut by a crash (no final newline, not valid JSON) is ignored;
/// any other bad line makes the journal unreadable.
pub fn parse(text: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    let raw: Vec<&str> = text.split('\n').collect();
    for (index, line) in raw.iter().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Line>(line) {
            Ok(parsed) => lines.push(parsed),
            Err(_) if index + 1 == raw.len() => {} // truncated last line
            Err(e) => return Err(format!("journal line {}: {e}", index + 1)),
        }
    }
    Ok(lines)
}

/// The pass rebuilt from its journal (R-SEC-06): finished, started and not started drivers.
pub fn reconstruct(text: &str) -> Result<PassView, String> {
    let mut view = PassView::default();
    for line in parse(text)? {
        view.apply(line);
    }
    if view.pass_id.is_empty() {
        return Err("journal without pass_start".to_owned());
    }
    Ok(view)
}

pub fn to_line(line: &Line) -> Result<String, String> {
    serde_json::to_string(line)
        .map(|mut s| {
            s.push('\n');
            s
        })
        .map_err(|e| e.to_string())
}
