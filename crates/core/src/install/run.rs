//! The pass itself (Spec - Installation, « Ce que fait l'installeur »): restore point, then one
//! driver at a time — orders, check again, backup, install, read again, signature, journal —
//! then the end of the pass. Also the rollback of one driver and the resume of a cut pass.

use super::actions::{InfFacts, InstallOutcome, PackageError, SignerCheck, SystemActions};
use super::control::{Order, StopMode, Waiting, accepted};
use super::journal::{
    Event, ItemRecord, Line, PassView, Refusal, Restore, Skip, Started, Status, Step,
};
use super::plan::{DriverFacts, MS_CATALOG, Plan, PlanItem, WINDOWS_UPDATE, is_oem_inf};
use crate::decide::{State, decide, parse_version};
use crate::model::{
    Device, DriverOffer, InstalledDriver, SCHEMA_VERSION, Snapshot, WindowsUpdateResult,
};

/// Signer expected for drivers from Windows Update (docs/SOURCES.md, to be checked in the VM).
pub const WINDOWS_UPDATE_SIGNER: &str = "Microsoft Windows Hardware Compatibility Publisher";
/// Signers accepted for a Catalog `.cab` (R-SRC-03). The exact name is to be checked in the VM;
/// both are Microsoft's, and the file's SHA-256 given by the Catalog is checked first (R-SRC-02).
pub const CATALOG_PACKAGE_SIGNERS: &[&str] = &[
    "Microsoft Corporation",
    "Microsoft Windows Hardware Compatibility Publisher",
];
/// A paused pass stops after this long without an order (24 h).
pub const MAX_WAIT_SECONDS: i64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub expected_signer: String,
    /// Off until the real signer names are seen in the VM: an unexpected signer is then only noted
    /// (Spec - Installation, « À vérifier »). An unsigned driver is always put back.
    pub enforce_signer: bool,
    pub poll_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            expected_signer: WINDOWS_UPDATE_SIGNER.to_owned(),
            enforce_signer: false,
            poll_seconds: 2,
        }
    }
}

/// Where journal lines go; each line is on disk before the next step starts.
pub trait Journal {
    fn append(&mut self, line: &Line) -> Result<(), String>;
}

/// The other passes, for the 5-pass limit (R-SEC-07).
pub trait PassStore {
    /// `(pass_id, journal text)` of every pass folder.
    fn passes(&mut self) -> Vec<(String, String)>;
    fn append(&mut self, pass_id: &str, line: &Line) -> Result<(), String>;
}

/// How a checked driver is installed.
enum How {
    WindowsUpdate,
    /// `id`: the device's most precise Hardware ID listed by the INF (R-SEC-02).
    Inf {
        id: String,
        inf: InfFacts,
    },
}

struct Pass<'a> {
    sim: bool,
    sys: &'a mut dyn SystemActions,
    journal: &'a mut dyn Journal,
    config: &'a Config,
    last_order: Option<String>,
    paused: bool,
    stop: Option<StopMode>,
    items: Vec<ItemRecord>,
    offers: Result<Vec<DriverOffer>, String>,
    /// R-SEC-09: the point of a restore-point pass is kept although no driver was touched.
    keep_point: bool,
}

impl Pass<'_> {
    fn log(&mut self, event: Event) -> Result<(), String> {
        let line = Line {
            at: self.sys.now(),
            simulation: self.sim,
            event,
        };
        self.journal.append(&line)
    }

    /// A new word in the control file, if any. The same word read again is not a new order, and
    /// the content of an unknown word is never written anywhere (R-SEC-08).
    fn order(&mut self, state: Waiting) -> Result<Option<Order>, String> {
        let raw = self.sys.read_control();
        if raw == self.last_order {
            return Ok(None);
        }
        self.last_order = raw.clone();
        let Some(raw) = raw else { return Ok(None) };
        match Order::parse(&raw).filter(|o| accepted(*o, state)) {
            Some(order) => Ok(Some(order)),
            None => {
                self.log(Event::InvalidOrder)?;
                Ok(None)
            }
        }
    }

    fn request_stop(&mut self, mode: StopMode) -> Result<(), String> {
        self.stop = Some(mode);
        self.paused = false;
        self.log(Event::StopRequested { mode })
    }

    /// Orders between two drivers; waits while paused (R-SEC-08). 24 h without order = `cancel_keep`.
    fn gate(&mut self) -> Result<(), String> {
        let since = self.sys.now();
        loop {
            let state = if self.paused {
                Waiting::Paused
            } else {
                Waiting::Running
            };
            match self.order(state)? {
                Some(Order::Pause) => {
                    self.paused = true;
                    self.log(Event::Paused)?;
                }
                Some(Order::Run) => {
                    self.paused = false;
                    self.log(Event::Resumed)?;
                }
                Some(Order::CancelKeep) => return self.request_stop(StopMode::Keep),
                Some(Order::CancelRevert) => return self.request_stop(StopMode::Revert),
                Some(Order::ContinueNoRestore) | None => {}
            }
            if !self.paused {
                return Ok(());
            }
            if self.sys.now() - since >= MAX_WAIT_SECONDS {
                self.paused = false;
                self.stop = Some(StopMode::Keep);
                return self.log(Event::WaitTimeout);
            }
            self.sys.wait(self.config.poll_seconds);
        }
    }

    /// R-SEC-01 (ADR-0034). `Ok(true)`: go on. A point that cannot be made is noted and the pass
    /// goes on without a question; the end of the pass says so.
    fn restore_point(&mut self, plan: &Plan) -> Result<(Option<i64>, bool), String> {
        let pass_id = plan.pass_id.as_str();
        if self.sim {
            self.log(Event::RestoreDone {
                restore: Restore::Simulated,
            })?;
            return Ok((None, true));
        }
        // ADR-0036: "no" was answered before the pass; never for a restore-point pass.
        if plan.no_restore_point && !plan.restore_point_only {
            self.log(Event::RestoreDone {
                restore: Restore::Declined,
            })?;
            return Ok((None, true));
        }
        self.log(Event::RestoreStart)?;
        // The point is the pass's only if its number is new: absent from the list read just before,
        // present once closed. Never a point Drivn did not create (R-SEC-07).
        let mut existing = None;
        let made = self.sys.restore_points().and_then(|before| {
            let seq = self.sys.begin_restore_point(&format!("Drivn {pass_id}"))?;
            if before.contains(&seq) {
                existing = Some(seq);
                return Err(format!(
                    "Windows gave the number {seq} of an older restore point"
                ));
            }
            // Closed at once: the snapshot is taken when it begins. A new number that Windows
            // accepted and closed is the pass's point: on 2026-11-02 points 73 and later were in
            // System Restore while the list read by WMI right after did not show them yet.
            self.sys
                .end_restore_point(seq, false)
                .map_err(|e| format!("restore point {seq} could not be closed: {e}"))?;
            Ok(seq)
        });
        match made {
            Ok(seq) => {
                self.log(Event::RestoreDone {
                    restore: Restore::Created { seq },
                })?;
                Ok((Some(seq), true))
            }
            Err(reason) => {
                self.log(Event::RestoreDone {
                    restore: Restore::Failed { reason, existing },
                })?;
                self.log(Event::ContinueNoRestore)?;
                Ok((None, true))
            }
        }
    }

    fn step<T>(
        &mut self,
        n: usize,
        step: Step,
        work: impl FnOnce(&mut Self) -> Result<T, Skip>,
        detail: impl Fn(&T) -> Option<String>,
    ) -> Result<Result<T, Skip>, String> {
        self.log(Event::StepStart { n, step })?;
        let out = work(self);
        self.log(Event::StepEnd {
            n,
            step,
            ok: out.is_ok(),
            detail: out.as_ref().ok().and_then(&detail),
        })?;
        Ok(out)
    }

    /// R-SEC-05: nothing from the app or from earlier drivers is believed.
    fn verify(&mut self, item: &PlanItem) -> Result<(Device, DriverOffer, Vec<String>), Skip> {
        if item.source != WINDOWS_UPDATE {
            return Err(Skip::Source);
        }
        let offers = self.offers.as_ref().map_err(|_| Skip::OfferGone)?;
        let offer = offers
            .iter()
            .find(|o| {
                o.update_id.as_deref() == Some(item.update_id.as_str())
                    && o.revision == Some(item.revision)
            })
            .cloned()
            .ok_or(Skip::OfferGone)?;
        let device = self
            .sys
            .read_device(&item.device_instance_id)
            .map_err(|_| Skip::ReadFailed)?
            .filter(|d| d.present)
            .ok_or(Skip::DeviceAbsent)?;
        // R-MAT-01: an ID of the device equal to the offer's, ignoring case.
        let offer_id = offer.hardware_id.as_deref().map(str::to_lowercase);
        let equal = |id: &String| Some(id.to_lowercase()) == offer_id;
        if offer_id.as_deref() != Some(&item.hardware_id.to_lowercase())
            || !device
                .hardware_ids
                .iter()
                .chain(&device.compatible_ids)
                .any(equal)
        {
            return Err(Skip::IdMismatch);
        }
        // R-SEC-02: Windows Update installs the offer on every present device with this ID. A twin
        // is allowed only with exactly the same driver now: the one backup then covers it too.
        let twins = self
            .sys
            .others_with_id(&item.hardware_id, &item.device_instance_id)
            .map_err(|_| Skip::ReadFailed)?;
        for twin in &twins {
            let driver = self
                .sys
                .read_device(twin)
                .map_err(|_| Skip::ReadFailed)?
                .ok_or(Skip::SharedId)?
                .driver;
            match (&item.installed, &driver) {
                (None, None) => {}
                (Some(planned), Some(now)) if planned.is(now) => {}
                _ => return Err(Skip::SharedId),
            }
        }
        // The installed driver, read now, is still the one of the plan.
        match (&item.installed, &device.driver) {
            (None, None) => {}
            (Some(planned), Some(now)) if planned.is(now) => {}
            _ => return Err(Skip::DriverChanged),
        }
        // R-VER-01, R-MAT-02: the engine decides again on the state read now.
        let snapshot = Snapshot {
            schema_version: SCHEMA_VERSION,
            tool_version: "drivn-installer".to_owned(),
            created_utc: String::new(),
            machine: None,
            anonymized: false,
            includes_absent_devices: false,
            devices: vec![device.clone()],
            windows_update: Some(WindowsUpdateResult {
                criteria: String::new(),
                online: true,
                last_search_utc: None,
                offers: vec![offer.clone()],
                error: None,
            }),
            vendors: Vec::new(),
        };
        let decision = decide(&snapshot).decisions.into_iter().next();
        match decision.map(|d| d.state) {
            Some(State::Update) => {}
            Some(State::UpdateRisky { .. }) if item.checked_by_user => {}
            Some(State::UpdateRisky { .. }) => return Err(Skip::RiskyNotChecked),
            _ => return Err(Skip::NoLongerNewer),
        }
        Ok((device, offer, twins))
    }

    /// R-SEC-05 for a Catalog driver: the installer gets the package itself, then decides again on
    /// the INF it holds (R-VER-03), never on the version the site announced.
    fn verify_inf(
        &mut self,
        pass_id: &str,
        n: usize,
        item: &PlanItem,
    ) -> Result<(Vec<String>, How), Skip> {
        if item.source != MS_CATALOG || !crate::sources::ms_catalog::is_guid(&item.update_id) {
            return Err(Skip::Source);
        }
        let device = self
            .sys
            .read_device(&item.device_instance_id)
            .map_err(|_| Skip::ReadFailed)?
            .filter(|d| d.present)
            .ok_or(Skip::DeviceAbsent)?;
        let lower = |s: &String| s.to_lowercase();
        let all: Vec<String> = device
            .hardware_ids
            .iter()
            .chain(&device.compatible_ids)
            .map(lower)
            .collect();
        if !all.contains(&item.hardware_id.to_lowercase()) {
            return Err(Skip::IdMismatch);
        }
        match (&item.installed, &device.driver) {
            (None, None) => {}
            (Some(planned), Some(now)) if planned.is(now) => {}
            _ => return Err(Skip::DriverChanged),
        }
        let package = self
            .sys
            .fetch_package(pass_id, n, &item.source, &item.update_id)
            .map_err(|e| match e {
                PackageError::Download(_) => Skip::DownloadFailed,
                PackageError::Hash => Skip::RefusedHash,
                PackageError::Signature(_) => Skip::RefusedSignature,
                PackageError::Archive(_) => Skip::ExtractFailed,
            })?;
        let lists =
            |inf: &InfFacts, id: &str| inf.hardware_ids.iter().any(|x| x.eq_ignore_ascii_case(id));
        // R-MAT-01 and R-SEC-02: the most precise ID of the device that an INF lists. Hardware IDs
        // first; then a Compatible ID that still names the exact chip: Windows files
        // `PCI\VEN_8086&DEV_7A68` as compatible, and Intel's INF (like the driver installed) uses it.
        let found = device
            .hardware_ids
            .iter()
            .chain(device.compatible_ids.iter().filter(|c| names_the_chip(c)))
            .find_map(|id| {
                package
                    .infs
                    .iter()
                    .find(|inf| lists(inf, id))
                    .map(|inf| (id.clone(), inf.clone()))
            });
        let Some((id, inf)) = found else {
            let compatible = package
                .infs
                .iter()
                .any(|inf| device.compatible_ids.iter().any(|c| lists(inf, c)));
            return Err(if compatible {
                Skip::CompatibleIdOnly
            } else {
                Skip::NoMatchingInf
            });
        };
        // R-SRC-08: the provider read at the source when the plan was made.
        let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
            _ => false,
        };
        if !same(&inf.provider, &item.offer.provider) {
            return Err(Skip::NoMatchingInf);
        }
        // R-VER-03: the DriverVer of the extracted INF, strictly newer, never an older date.
        if let Some(now) = &device.driver {
            let newer = match (
                inf.version.as_deref().and_then(parse_version),
                now.version.as_deref().and_then(parse_version),
            ) {
                (Some(new), Some(old)) => new > old,
                _ => false,
            };
            let older_date = matches!((&inf.date, &now.date), (Some(a), Some(b)) if a < b);
            if !newer || older_date {
                return Err(Skip::NoLongerNewer);
            }
        }
        // R-SRC-04: its catalog is signed by the expected signer.
        match &inf.signer {
            SignerCheck::Signed { signer }
                if !self.config.enforce_signer || *signer == self.config.expected_signer => {}
            _ => return Err(Skip::RefusedCatalog),
        }
        // R-SEC-02: Windows updates every present device with this ID.
        let twins = self
            .sys
            .others_with_id(&id, &item.device_instance_id)
            .map_err(|_| Skip::ReadFailed)?;
        for twin in &twins {
            let driver = self
                .sys
                .read_device(twin)
                .map_err(|_| Skip::ReadFailed)?
                .ok_or(Skip::SharedId)?
                .driver;
            match (&item.installed, &driver) {
                (None, None) => {}
                (Some(planned), Some(now)) if planned.is(now) => {}
                _ => return Err(Skip::SharedId),
            }
        }
        Ok((twins, How::Inf { id, inf }))
    }

    fn check(
        &mut self,
        pass_id: &str,
        n: usize,
        item: &PlanItem,
    ) -> Result<(Vec<String>, How), Skip> {
        if item.source == WINDOWS_UPDATE {
            self.verify(item)
                .map(|(_, _, twins)| (twins, How::WindowsUpdate))
        } else {
            self.verify_inf(pass_id, n, item)
        }
    }

    fn record(item: &PlanItem, n: usize, status: Status) -> ItemRecord {
        ItemRecord {
            n,
            device_instance_id: item.device_instance_id.clone(),
            name: item.name.clone(),
            status,
            skip: None,
            old: item.installed.clone(),
            new: item.offer.clone(),
            backup: None,
            hresult: None,
            reboot_required: false,
            signer: None,
            after_interruption: false,
            detail: None,
            twins: Vec::new(),
            no_backup: false,
            forced: false,
        }
    }

    fn finish(&mut self, record: ItemRecord) -> Result<(), String> {
        self.items.push(record.clone());
        self.log(Event::ItemEnd { record })
    }

    fn install_item(&mut self, pass_id: &str, n: usize, item: &PlanItem) -> Result<(), String> {
        self.log(Event::ItemStart {
            n,
            device_instance_id: item.device_instance_id.clone(),
            name: item.name.clone(),
            old: item.installed.clone(),
            offer: item.offer.clone(),
        })?;
        let mut record = Self::record(item, n, Status::Skipped);

        let how = match self.step(
            n,
            Step::Verify,
            |p| p.check(pass_id, n, item),
            |(_, how)| match how {
                How::WindowsUpdate => None,
                How::Inf { inf, .. } => Some(format!(
                    "INF {} {}",
                    inf.version.as_deref().unwrap_or("?"),
                    inf.date.as_deref().unwrap_or("?")
                )),
            },
        )? {
            Err(skip) => {
                record.skip = Some(skip);
                return self.finish(record);
            }
            Ok((twins, how)) => {
                record.twins = twins;
                how
            }
        };
        // What the device must run after the install: the offer of Windows Update, or the INF
        // read in the package (R-VER-03), never the version announced by the site.
        let expected = match &how {
            How::WindowsUpdate => item.offer.clone(),
            How::Inf { inf, .. } => DriverFacts {
                inf: None,
                title: item.offer.title.clone(),
                version: inf.version.clone(),
                date: inf.date.clone(),
                provider: inf.provider.clone(),
            },
        };
        record.new = expected.clone();

        // R-SEC-02 (ADR-0034): the current driver is exported first. A driver of Windows cannot be
        // (to be checked), an export can fail: the install goes on, noted "without Drivn backup";
        // the restore point of the pass still covers it.
        let backup = self.step(
            n,
            Step::Backup,
            |p| match &item.installed {
                None => Ok(None),
                Some(old) => {
                    let inf = old.inf.as_deref().unwrap_or_default();
                    if !is_oem_inf(inf) {
                        Err(Skip::WindowsDriver)
                    } else if p.sim {
                        Ok(None)
                    } else {
                        p.sys
                            .export_driver(pass_id, inf, n)
                            .map(Some)
                            .map_err(|_| Skip::BackupFailed)
                    }
                }
            },
            Clone::clone,
        )?;
        match backup {
            Ok(path) => record.backup = path,
            Err(_) => record.no_backup = true,
        }

        // R-TST-01: the simulation stops right before the call that changes the system.
        self.log(Event::StepStart {
            n,
            step: Step::Install,
        })?;
        if self.sim {
            self.log(Event::StepEnd {
                n,
                step: Step::Install,
                ok: true,
                detail: Some("simulation".to_owned()),
            })?;
            record.status = Status::Simulated;
            return self.finish(record);
        }
        // R-SEC-04: first without the forcing flag.
        let outcome = match &how {
            How::WindowsUpdate => self.sys.install_update(&item.update_id, item.revision),
            How::Inf { id, inf } => self.sys.install_inf(id, &inf.path),
        };
        self.installed(n, Step::Install, &outcome, &mut record)?;
        self.reread(n, item, &expected, &outcome, &mut record)?;
        // R-SEC-04 (ADR-0034): Windows kept its own driver although this INF was checked newer
        // (R-VER-03) and signed (R-SRC-04) by `verify_inf`: installed again, forced. Never for a
        // Windows Update offer (WUA has no such flag) nor an older or unsigned driver.
        if record.status == Status::NotSelectedByWindows
            && let How::Inf { id, inf } = &how
        {
            self.log(Event::StepStart {
                n,
                step: Step::Force,
            })?;
            let forced = self.sys.install_inf_forced(id, &inf.path);
            record.forced = true;
            self.installed(n, Step::Force, &forced, &mut record)?;
            self.reread(n, item, &expected, &forced, &mut record)?;
        }

        // R-SRC-04: the catalog of the active INF. Pending reboot: checked at the next launch.
        if record.status == Status::Installed
            || (record.status == Status::Unknown && record.new.inf.is_some())
        {
            return self.signature(record);
        }
        self.finish(record)
    }

    /// The end of an install call, in the journal and the record.
    fn installed(
        &mut self,
        n: usize,
        step: Step,
        outcome: &InstallOutcome,
        record: &mut ItemRecord,
    ) -> Result<(), String> {
        self.log(Event::StepEnd {
            n,
            step,
            ok: outcome.succeeded,
            detail: outcome.detail.clone(),
        })?;
        record.reboot_required = outcome.reboot_required;
        if outcome.succeeded {
            record.hresult = None;
            record.detail = None;
        } else {
            record.hresult = Some(outcome.hresult);
            record.detail = outcome.detail.clone();
        }
        Ok(())
    }

    /// The device is read again whatever Windows answered: only what it runs now says what happened.
    fn reread(
        &mut self,
        n: usize,
        item: &PlanItem,
        expected: &DriverFacts,
        outcome: &InstallOutcome,
        record: &mut ItemRecord,
    ) -> Result<(), String> {
        self.log(Event::StepStart {
            n,
            step: Step::Reread,
        })?;
        let now = self
            .sys
            .read_device(&item.device_instance_id)
            .ok()
            .flatten()
            .and_then(|d| d.driver);
        let old_still = match (&item.installed, &now) {
            (None, None) => true,
            (Some(old), Some(now)) => old.is(now),
            _ => false,
        };
        let active_new = now.as_ref().filter(|d| expected.same_release(d));
        record.status = match (active_new, old_still) {
            (Some(driver), _) => {
                record.new = DriverFacts {
                    title: item.offer.title.clone(),
                    ..DriverFacts::of(driver)
                };
                Status::Installed
            }
            (None, true) if !outcome.succeeded => Status::Failed,
            (None, true) if outcome.reboot_required => Status::InstalledPendingReboot,
            // R-SEC-04: Windows kept the old, better ranked driver. A result, not an error.
            (None, true) => Status::NotSelectedByWindows,
            // Neither the old nor the offered driver: never called "not selected" or "failed".
            (None, false) => {
                record.new = now
                    .as_ref()
                    .map_or_else(DriverFacts::default, DriverFacts::of);
                Status::Unknown
            }
        };
        self.log(Event::StepEnd {
            n,
            step: Step::Reread,
            ok: true,
            detail: None,
        })
    }

    /// R-SRC-04 on a driver found active: signed by the expected signer, or put back at once.
    /// If it cannot be put back, it is never shown as fine: `UnsignedNotReverted`.
    fn signature(&mut self, mut record: ItemRecord) -> Result<(), String> {
        let n = record.n;
        self.log(Event::StepStart {
            n,
            step: Step::Signature,
        })?;
        let inf = record.new.inf.clone().unwrap_or_default();
        let bad = match self.sys.driver_signer(&inf) {
            SignerCheck::Signed { signer } => {
                let bad = self.config.enforce_signer && signer != self.config.expected_signer;
                record.signer = Some(signer);
                bad
            }
            SignerCheck::Unsigned => true,
            SignerCheck::Unknown { reason } => {
                record.detail = Some(reason);
                true
            }
        };
        self.log(Event::StepEnd {
            n,
            step: Step::Signature,
            ok: !bad,
            detail: record.signer.clone(),
        })?;
        if !bad {
            return self.finish(record);
        }
        // One `ItemEnd` only, with the final status: never a "✓" line for an unsigned driver, even
        // if the PC goes off during the rollback (the resume then reads the device again).
        let status = revert(self.sys, &mut *self.journal, self.sim, &record)?;
        match status {
            Some(Status::Reverted) => record.status = Status::RolledBackUnsigned,
            // The old driver comes back at the next restart: offered, like any pending result.
            Some(Status::RevertedPendingReboot) => record.status = Status::RevertedPendingReboot,
            _ => record.status = Status::UnsignedNotReverted,
        }
        self.finish(record)
    }

    fn end(&mut self, pass_id: &str, seq: Option<i64>, not_started: usize) -> Result<(), String> {
        // Downloaded and extracted packages are never kept (Spec, « Installer un INF », 6).
        if let Err(error) = self.sys.clean_work(pass_id) {
            self.log(Event::WorkNotCleaned { error })?;
        }
        if let Some(seq) = seq.filter(|_| !self.sim) {
            // The point is already closed. Removed only when no driver touched the system and
            // nothing else runs after the drivers; otherwise it stays, even for an uncertain result.
            let cancel = !self.keep_point && self.items.iter().all(|i| i.status.untouched());
            let error = if cancel {
                self.sys.remove_restore_point(seq).err()
            } else {
                None
            };
            self.log(Event::RestoreEnd {
                seq,
                cancelled: cancel && error.is_none(),
                error,
            })?;
        }
        self.log(Event::PassEnd {
            not_started,
            stopped: self.stop,
        })
    }

    fn revert_all(&mut self) -> Result<(), String> {
        // Last to first; drivers not started are not touched.
        let installed: Vec<ItemRecord> = self.items.iter().rev().cloned().collect();
        for record in installed.iter().filter(|r| r.status.installed()) {
            if let Some(status) = revert(self.sys, &mut *self.journal, self.sim, record)?
                && let Some(item) = self.items.iter_mut().find(|i| i.n == record.n)
            {
                item.status = status;
            }
        }
        Ok(())
    }

    /// The drivers from `from` (0-based) to the end of the plan.
    fn drivers(&mut self, plan: &Plan, from: usize) -> Result<usize, String> {
        for (index, item) in plan.items.iter().enumerate().skip(from) {
            self.gate()?;
            if self.stop.is_some() {
                return Ok(plan.items.len() - index);
            }
            self.install_item(&plan.pass_id, index + 1, item)?;
        }
        // An order given during the last driver still counts.
        if !self.paused
            && let Some(order) = self.order(Waiting::Running)?
        {
            match order {
                Order::CancelKeep => self.request_stop(StopMode::Keep)?,
                Order::CancelRevert => self.request_stop(StopMode::Revert)?,
                _ => {}
            }
        }
        Ok(0)
    }
}

fn new_pass<'a>(
    sim: bool,
    sys: &'a mut dyn SystemActions,
    journal: &'a mut dyn Journal,
    config: &'a Config,
) -> Pass<'a> {
    Pass {
        sim,
        sys,
        journal,
        config,
        last_order: None,
        paused: false,
        stop: None,
        items: Vec::new(),
        offers: Err(String::new()),
        keep_point: false,
    }
}

fn search(pass: &mut Pass<'_>) -> Result<(), String> {
    pass.offers = pass.sys.search_offers();
    let (count, error) = match &pass.offers {
        Ok(offers) => (offers.len(), None),
        Err(e) => (0, Some(e.clone())),
    };
    pass.log(Event::OffersSearched { count, error })
}

/// One full pass. `sim` comes from [`super::setup::simulation`]; in simulation `sys` must be a
/// [`super::actions::Simulated`] system.
pub fn run_pass(
    plan: &Plan,
    sim: bool,
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    config: &Config,
) -> Result<Vec<ItemRecord>, String> {
    let mut pass = new_pass(sim, sys, journal, config);
    // R-SEC-09: the publishers' installers and the applications run after the drivers, under this point.
    pass.keep_point = plan.restore_point_only || !plan.packages.is_empty() || !plan.apps.is_empty();
    // A word already in the control file before the pass is not an order of this pass (R-SEC-08).
    pass.last_order = pass.sys.read_control();
    pass.log(Event::PassStart {
        pass_id: plan.pass_id.clone(),
        items: plan.items.len(),
    })?;
    if wants_search(plan) {
        search(&mut pass)?;
    }
    let (seq, go) = pass.restore_point(plan)?;
    let not_started = if go {
        pass.drivers(plan, 0)?
    } else {
        plan.items.len()
    };
    if pass.stop == Some(StopMode::Revert) {
        pass.revert_all()?;
    }
    pass.end(&plan.pass_id, seq, not_started)?;
    Ok(pass.items)
}

/// The Windows Update search is made only when the plan holds a Windows Update driver.
fn wants_search(plan: &Plan) -> bool {
    plan.items.iter().any(|i| i.source == WINDOWS_UPDATE)
}

/// R-SEC-06: a pass cut by a crash, taken up again (`stop: None`) or stopped (keep / revert) on the
/// user's choice. The driver started without an end is read again before anything else.
pub fn resume_pass(
    plan: &Plan,
    view: &PassView,
    stop: Option<StopMode>,
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    config: &Config,
) -> Result<Vec<ItemRecord>, String> {
    let sim = view.simulation;
    let mut pass = new_pass(sim, sys, journal, config);
    // R-SEC-09: the publishers' installers and the applications run after the drivers, under this point.
    pass.keep_point = plan.restore_point_only || !plan.packages.is_empty() || !plan.apps.is_empty();
    pass.last_order = pass.sys.read_control();
    pass.items = view.items.clone();
    pass.log(Event::Resume { stop })?;
    if let Some(started) = &view.started {
        let record = recover(pass.sys, sim, started);
        // A driver found active after the cut gets its signature check too (R-SRC-04).
        if record.status == Status::Installed && !sim {
            pass.signature(record)?;
        } else {
            pass.finish(record)?;
        }
    }
    if let Some(n) = view.reverting
        && let Some(record) = pass.items.iter().find(|i| i.n == n).cloned()
    {
        // The revert checks first whether the old driver is already back.
        if let Some(status) = revert(pass.sys, &mut *pass.journal, sim, &record)?
            && let Some(item) = pass.items.iter_mut().find(|i| i.n == n)
        {
            item.status = status;
        }
    }
    pass.stop = stop.or(view.stop);
    let mut seq = view.restore_open.or(view.restore_seq());
    let done = view.last_n();
    let mut not_started = plan.items.len().saturating_sub(done);
    if pass.stop.is_none() && not_started > 0 {
        if wants_search(plan) {
            search(&mut pass)?;
        }
        // R-SEC-01 (ADR-0034): cut before the point was tried: tried again, never a question.
        if view.restore.is_none() {
            seq = pass.restore_point(plan)?.0;
        }
        not_started = pass.drivers(plan, done)?;
    }
    if pass.stop == Some(StopMode::Revert) {
        pass.revert_all()?;
    }
    pass.end(
        &plan.pass_id,
        seq.filter(|_| !view.restore_cancelled),
        not_started,
    )?;
    Ok(pass.items)
}

/// What the device runs now, for a driver cut in the middle (R-SEC-06).
fn recover(sys: &mut dyn SystemActions, sim: bool, started: &Started) -> ItemRecord {
    let now = sys
        .read_device(&started.device_instance_id)
        .ok()
        .flatten()
        .and_then(|d| d.driver);
    let is_old = match (&started.old, &now) {
        (None, None) => true,
        (Some(old), Some(now)) => old.is(now),
        _ => false,
    };
    let new = now.as_ref().filter(|d| started.offer.same_release(d));
    let status = if sim {
        Status::Simulated
    } else if new.is_some() {
        Status::Installed
    } else if is_old {
        Status::NotInstalled
    } else {
        Status::Unknown
    };
    ItemRecord {
        n: started.n,
        device_instance_id: started.device_instance_id.clone(),
        name: started.name.clone(),
        status,
        skip: None,
        old: started.old.clone(),
        new: new.map_or_else(|| started.offer.clone(), DriverFacts::of),
        backup: started.backup.clone(),
        hresult: None,
        reboot_required: false,
        signer: None,
        after_interruption: true,
        detail: None,
        twins: Vec::new(),
        no_backup: started.backup.is_none() && started.old.is_some(),
        forced: false,
    }
}

fn driver_now(sys: &mut dyn SystemActions, id: &str) -> Result<Option<InstalledDriver>, String> {
    Ok(sys.read_device(id)?.and_then(|d| d.driver))
}

/// The old driver (or no driver, if there was none) is active again.
fn back_to_old(old: Option<&DriverFacts>, now: Option<&InstalledDriver>) -> bool {
    match (old, now) {
        (Some(old), Some(now)) => old.same_release(now),
        (None, None) => true,
        // No driver before: back only when the device has none again.
        (None, Some(_)) => false,
        (Some(_), None) => false,
    }
}

/// Puts one driver back (Spec - Installation, « Retour arrière »). Written to the journal before and
/// after. Returns the new status, `None` when refused or failed.
pub fn revert(
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    sim: bool,
    record: &ItemRecord,
) -> Result<Option<Status>, String> {
    let n = record.n;
    let log = |sys: &mut dyn SystemActions, journal: &mut dyn Journal, event: Event| {
        write(sys, journal, sim, event)
    };
    let refuse = |refusal: Refusal, detail: Option<String>| Event::RevertEnd {
        n,
        status: None,
        refusal: Some(refusal),
        detail,
    };
    if record.status == Status::InstalledPendingReboot {
        log(sys, journal, Event::RevertStart { n })?;
        log(sys, journal, refuse(Refusal::RebootFirst, None))?;
        return Ok(None);
    }
    if !record.status.installed() || sim {
        log(sys, journal, Event::RevertStart { n })?;
        log(sys, journal, refuse(Refusal::NothingInstalled, None))?;
        return Ok(None);
    }
    log(sys, journal, Event::RevertStart { n })?;
    let id = record.device_instance_id.as_str();
    let old = record.old.as_ref();
    let new_inf = record.new.inf.clone().unwrap_or_default();
    let now = driver_now(sys, id);
    let now = match now {
        Ok(now) => now,
        Err(e) => {
            log(sys, journal, refuse(Refusal::Changed, Some(e)))?;
            return Ok(None);
        }
    };
    // Already back (a revert cut after the change, R-SEC-06).
    if back_to_old(old, now.as_ref()) && now.as_ref().is_none_or(|d| !record.new.is(d)) {
        log(
            sys,
            journal,
            Event::RevertEnd {
                n,
                status: Some(Status::Reverted),
                refusal: None,
                detail: None,
            },
        )?;
        return Ok(Some(Status::Reverted));
    }
    // Precondition: the device still uses the new INF of the journal, same release.
    if !now.as_ref().is_some_and(|d| record.new.is(d)) {
        log(sys, journal, refuse(Refusal::Changed, None))?;
        return Ok(None);
    }
    let mut reboot = false;
    let twins = record.twins.clone();
    // Every device that may run the new driver: this one and its twins (R-SEC-02).
    let all_back = |sys: &mut dyn SystemActions| {
        let main = driver_now(sys, id).ok().flatten();
        back_to_old(old, main.as_ref())
            && twins.iter().all(|t| {
                driver_now(sys, t)
                    .ok()
                    .flatten()
                    .is_none_or(|d| !record.new.is(&d))
            })
    };
    // 1. DiRollbackDriver, if Windows kept the previous driver.
    if old.is_some() {
        let mut ok = true;
        for device in std::iter::once(id).chain(twins.iter().map(String::as_str)) {
            match sys.rollback_driver(device) {
                Ok(outcome) => reboot |= outcome.reboot_required,
                Err(_) => ok = false,
            }
        }
        if ok && all_back(sys) {
            return finish_revert(sys, journal, sim, n, Status::Reverted).map(Some);
        }
    }
    // 2. The backup put back, then the new INF removed so Windows takes the best one left.
    let mut mine: Vec<String> = twins.clone();
    mine.push(id.to_owned());
    if sys.inf_used_elsewhere(&new_inf, &mine).unwrap_or(true) {
        log(sys, journal, refuse(Refusal::InfShared, None))?;
        return Ok(None);
    }
    if old.is_some() {
        match &record.backup {
            Some(backup) => match sys.add_driver(backup) {
                Ok(outcome) => reboot |= outcome.reboot_required,
                Err(e) => return fail_revert(sys, journal, sim, n, e),
            },
            // R-SEC-02 (ADR-0034): no Drivn backup; the old INF is still in the driver store (a
            // driver of Windows, or an export that failed): removing the new one brings it back.
            None if record.no_backup => {}
            None => {
                log(sys, journal, refuse(Refusal::Expired, None))?;
                return Ok(None);
            }
        }
    }
    match sys.delete_driver(&new_inf) {
        Ok(outcome) => reboot |= outcome.reboot_required,
        Err(e) => return fail_revert(sys, journal, sim, n, e),
    }
    let status = if all_back(sys) {
        Status::Reverted
    } else if reboot {
        Status::RevertedPendingReboot
    } else {
        return fail_revert(
            sys,
            journal,
            sim,
            n,
            "the old driver is not active".to_owned(),
        );
    };
    finish_revert(sys, journal, sim, n, status).map(Some)
}

fn write(
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    sim: bool,
    event: Event,
) -> Result<(), String> {
    journal.append(&Line {
        at: sys.now(),
        simulation: sim,
        event,
    })
}

fn finish_revert(
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    sim: bool,
    n: usize,
    status: Status,
) -> Result<Status, String> {
    journal.append(&Line {
        at: sys.now(),
        simulation: sim,
        event: Event::RevertEnd {
            n,
            status: Some(status),
            refusal: None,
            detail: None,
        },
    })?;
    Ok(status)
}

fn fail_revert(
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
    sim: bool,
    n: usize,
    detail: String,
) -> Result<Option<Status>, String> {
    journal.append(&Line {
        at: sys.now(),
        simulation: sim,
        event: Event::RevertEnd {
            n,
            status: None,
            refusal: None,
            detail: Some(detail),
        },
    })?;
    Ok(None)
}

/// Rollback of one driver outside a pass (`--rollback <passe> <n>`). `last_boot`: Unix seconds of
/// the last start of Windows; a driver that waited for a restart can go back once it happened.
pub fn rollback_one(
    view: &PassView,
    n: usize,
    last_boot: i64,
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
) -> Result<Option<Status>, String> {
    let mut record = view
        .items
        .iter()
        .find(|i| i.n == n)
        .cloned()
        .ok_or_else(|| format!("no driver {n} in pass {}", view.pass_id))?;
    if view.pruned {
        write(sys, journal, view.simulation, Event::RevertStart { n })?;
        write(
            sys,
            journal,
            view.simulation,
            Event::RevertEnd {
                n,
                status: None,
                refusal: Some(Refusal::Expired),
                detail: None,
            },
        )?;
        return Ok(None);
    }
    let restarted = view.ended_at.is_some_and(|end| last_boot >= end);
    if record.status == Status::InstalledPendingReboot && restarted {
        // The new driver is active now: its INF is read on the device.
        if let Some(driver) =
            driver_now(sys, &record.device_instance_id)?.filter(|d| record.new.same_release(d))
        {
            record.new = DriverFacts {
                title: record.new.title.clone(),
                ..DriverFacts::of(&driver)
            };
            record.status = Status::Installed;
        }
    }
    revert(sys, journal, view.simulation, &record)
}

/// A Compatible ID that names one chip or product (`PCI\VEN_8086&DEV_7A68`, `USB\VID_x&PID_y`),
/// never a class (`CC_`, `Class_`) or a vendor alone (`PCI\VEN_8086`): those match many devices.
pub fn names_the_chip(id: &str) -> bool {
    let up = id.to_ascii_uppercase();
    (up.contains("&DEV_") || up.contains("&PID_")) && !up.contains("CC_") && !up.contains("CLASS")
}

/// The device of a package runs the version it announced (NVIDIA: `32.0.16.2101` is `621.01`).
fn active_release(sys: &mut dyn SystemActions, package: &super::plan::PackageItem) -> bool {
    let Ok(Some(device)) = sys.read_device(&package.device_instance_id) else {
        return false;
    };
    let version = device.driver.and_then(|d| d.version).unwrap_or_default();
    match package.source.as_str() {
        "nvidia" => crate::sources::nvidia::nvidia_version(&version)
            .is_some_and(|v| Some(v) == crate::sources::nvidia::parse_release(&package.to)),
        _ => false,
    }
}

/// ADR-0034, R-BUT-02, R-SEC-09: after the drivers, the publishers' installers then the
/// applications of the plan, one at a time. A failure is noted and the next one starts; nothing
/// stops the list.
pub fn run_apps(
    plan: &Plan,
    simulation: bool,
    sys: &mut dyn SystemActions,
    journal: &mut dyn Journal,
) -> Result<(), String> {
    use super::journal::AppStatus;
    for (index, package) in plan.packages.iter().enumerate() {
        let n = index + 1;
        write(
            sys,
            journal,
            simulation,
            Event::PackageStart {
                n,
                device_instance_id: package.device_instance_id.clone(),
                name: package.name.clone(),
            },
        )?;
        let known = super::official::official(&package.source)
            .filter(|o| super::official::valid_file(o, &package.file));
        let (status, detail) = match known {
            None => (AppStatus::Failed, Some("package refused".to_owned())),
            Some(official) => match sys.run_official(&plan.pass_id, n, official, &package.file) {
                Ok(0) if simulation => (AppStatus::Simulated, None),
                Ok(0) => (AppStatus::Updated, None),
                // What the device runs now says what happened, whatever the installer answered:
                // NVIDIA's can end with an error from a side component, its driver installed.
                Ok(code) if !simulation && active_release(sys, package) => (
                    AppStatus::Updated,
                    Some(format!("exit code {code}, driver {} active", package.to)),
                ),
                Ok(code) => (AppStatus::Failed, Some(format!("exit code {code}"))),
                Err(why) => (AppStatus::Failed, Some(why)),
            },
        };
        write(
            sys,
            journal,
            simulation,
            Event::PackageEnd { n, status, detail },
        )?;
    }
    let mut again = Vec::new();
    for (index, app) in plan.apps.iter().enumerate() {
        let n = index + 1;
        write(
            sys,
            journal,
            simulation,
            Event::AppStart {
                n,
                id: app.id.clone(),
                name: app.name.clone(),
            },
        )?;
        let (status, detail, code) = upgrade(sys, simulation, &app.id, false);
        if code.is_some_and(|c| crate::sources::winget::RETRY_LATER.contains(&c)) {
            again.push((n, app, false));
        } else if code == Some(crate::sources::winget::REPLACE) {
            again.push((n, app, true));
        }
        write(
            sys,
            journal,
            simulation,
            Event::AppEnd {
                n,
                id: app.id.clone(),
                status,
                detail,
            },
        )?;
    }
    // R-BUT-01: one more try at the end for what was only busy (another installer still running,
    // the application open): its line in the journal is ended again with the new result.
    if !again.is_empty() {
        // Said before the wait, so the screen never looks finished while it waits.
        write(
            sys,
            journal,
            simulation,
            Event::AppsRetry {
                ns: again.iter().map(|(n, _, _)| *n).collect(),
                seconds: RETRY_WAIT_SECONDS,
            },
        )?;
        sys.wait(RETRY_WAIT_SECONDS);
    }
    for (n, app, replace) in again {
        let (status, detail, _) = upgrade(sys, simulation, &app.id, replace);
        write(
            sys,
            journal,
            simulation,
            Event::AppEnd {
                n,
                id: app.id.clone(),
                status,
                detail: detail.or_else(|| Some("second try".to_owned())),
            },
        )?;
    }
    Ok(())
}

/// Before the second try of the busy applications.
pub const RETRY_WAIT_SECONDS: u64 = 30;

/// One winget update: status, detail and winget's exit code when it ran.
/// `replace`: the old version uninstalled first (winget `--uninstall-previous`), for a new version
/// that installs another way.
fn upgrade(
    sys: &mut dyn SystemActions,
    simulation: bool,
    id: &str,
    replace: bool,
) -> (super::journal::AppStatus, Option<String>, Option<i32>) {
    use super::journal::AppStatus;
    if !crate::sources::winget::valid_id(id) {
        return (AppStatus::Failed, Some("invalid id".to_owned()), None);
    }
    let ran = if replace {
        sys.replace_app(id)
    } else {
        sys.upgrade_app(id)
    };
    match ran {
        Ok(0) if simulation => (AppStatus::Simulated, None, Some(0)),
        Ok(0) => (AppStatus::Updated, None, Some(0)),
        Ok(code) => (
            AppStatus::Failed,
            Some(crate::sources::winget::why(code)),
            Some(code),
        ),
        Err(why) => (AppStatus::Failed, Some(why), None),
    }
}
