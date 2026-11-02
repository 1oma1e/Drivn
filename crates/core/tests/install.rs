//! Tests of the install pass (docs/specs/Spec - Installation.md, « Règles et tests ») with a fake
//! system: nothing here touches Windows. One test per line of the spec's table, named after its rule.
// The fake system's helpers panic on a broken test setup, like the tests themselves.
#![allow(clippy::expect_used)]

use std::collections::{HashMap, HashSet};

use drivn_core::decide::decide;
use drivn_core::install::actions::{
    ChangeOutcome, InstallOutcome, SignerCheck, Simulated, SystemActions, commands,
};
use drivn_core::install::control::{Order, StopMode, Waiting, accepted};
use drivn_core::install::journal::AppStatus;
use drivn_core::install::journal::{
    Event, ItemRecord, Line, PassView, Refusal, Restore, Skip, Status, Step, parse, reconstruct,
    to_line,
};
use drivn_core::install::official::{Official, official, valid_file};
use drivn_core::install::plan::{
    Mode, PackageItem, Plan, add_apps, build_plan, is_oem_inf, new_pass_id, sha256_hex,
    valid_pass_id,
};
use drivn_core::install::reboot::{AfterReboot, after_reboot, reboot_needed, reboot_pending};
use drivn_core::install::retention::{KEEP_PASSES, KEEP_POINTS, points_to_remove, prune, to_prune};
use drivn_core::install::run::{
    Config, Journal, PassStore, WINDOWS_UPDATE_SIGNER, resume_pass, revert, rollback_one, run_apps,
    run_pass,
};
use drivn_core::install::setup::{
    FolderFacts, PlanFile, check_before_copy, check_copy, check_root, may_launch, plan_mode,
    simulation,
};
use drivn_core::model::{
    Device, DriverOffer, InstalledDriver, SCHEMA_VERSION, Signature, Snapshot, WindowsUpdateResult,
};

const PASS: &str = "2026-11-01T1410-a3f9";

// ---------- fake system ----------

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Activate,
    PendingReboot,
    NotSelected,
    Fail(i32),
}

struct Fake {
    clock: i64,
    devices: HashMap<String, Device>,
    /// Driver Windows kept for `DiRollbackDriver`.
    previous: HashMap<String, InstalledDriver>,
    offers: Result<Vec<DriverOffer>, String>,
    restore: Result<i64, String>,
    restore_listed: bool,
    /// Restore points Windows lists (sequence numbers).
    points: Vec<i64>,
    export_fails: HashSet<String>,
    answers: HashMap<String, Answer>,
    unsigned: HashSet<String>,
    rollback_works: bool,
    /// Exit code of a publisher's installer; `None`: this system cannot run one.
    official: Option<i32>,
    /// Restore points Windows already deleted by itself: removing them fails.
    gone_points: Vec<i64>,
    /// winget's exit codes, one per call, in order; none left: winget cannot run here.
    winget: Vec<i32>,
    control: Option<String>,
    /// Word written in the control file while this update installs (a click during a driver).
    click_during: HashMap<String, String>,
    /// Device changed by someone else while this update installs.
    change_during: HashMap<String, (String, InstalledDriver)>,
    /// Words written one by one at each wait (the user answering later).
    later: Vec<Option<String>>,
    next_inf: u32,
    /// Every call that changes the system, in order.
    changes: Vec<String>,
}

fn drv(inf: &str, version: &str, date: &str, provider: &str, matching: &str) -> InstalledDriver {
    InstalledDriver {
        version: Some(version.to_owned()),
        date: Some(date.to_owned()),
        provider: Some(provider.to_owned()),
        inf_path: Some(inf.to_owned()),
        rank: None,
        matching_device_id: Some(matching.to_owned()),
        signature: Signature::Unknown,
    }
}

fn hwid(i: usize) -> String {
    format!("PCI\\VEN_10EC&DEV_{i:04}")
}

fn device(i: usize, driver: Option<InstalledDriver>) -> Device {
    Device {
        instance_id: format!("PCI\\DEV{i}\\0"),
        description: Some(format!("Carte {i}")),
        class: Some("Net".to_owned()),
        class_guid: None,
        manufacturer: Some("Contoso".to_owned()),
        present: true,
        problem_code: None,
        hardware_ids: vec![hwid(i)],
        compatible_ids: Vec::new(),
        driver,
    }
}

fn old_driver(i: usize) -> InstalledDriver {
    drv(
        &format!("oem{i}.inf"),
        "1.0.0.0",
        "2024-01-01",
        "Contoso",
        &hwid(i),
    )
}

fn offer(i: usize, version: &str) -> DriverOffer {
    DriverOffer {
        title: Some(format!("Contoso - Net - Carte {i} ({version})")),
        hardware_id: Some(hwid(i).to_lowercase()),
        driver_date: Some("2025-06-01".to_owned()),
        provider: Some("Contoso".to_owned()),
        manufacturer: None,
        model: None,
        class: Some("Net".to_owned()),
        update_id: Some(format!("upd-{i}")),
        revision: Some(200 + i32::try_from(i).unwrap_or(0)),
        download_url: None,
    }
}

impl Fake {
    /// `count` devices with an old `oem#.inf` driver, each with a newer Windows Update offer.
    fn new(count: usize) -> Self {
        let devices = (1..=count)
            .map(|i| {
                let d = device(i, Some(old_driver(i)));
                (d.instance_id.clone(), d)
            })
            .collect();
        Self {
            clock: 1_800_000_000,
            devices,
            previous: HashMap::new(),
            offers: Ok((1..=count).map(|i| offer(i, "2.0.0.0")).collect()),
            restore: Ok(431),
            restore_listed: true,
            points: vec![100],
            export_fails: HashSet::new(),
            answers: HashMap::new(),
            unsigned: HashSet::new(),
            rollback_works: true,
            official: None,
            gone_points: Vec::new(),
            winget: Vec::new(),
            control: None,
            click_during: HashMap::new(),
            change_during: HashMap::new(),
            later: Vec::new(),
            next_inf: 50,
            changes: Vec::new(),
        }
    }

    fn snapshot(&self, order: &[String]) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            tool_version: "test".to_owned(),
            created_utc: "2026-11-01T14:10:00Z".to_owned(),
            machine: None,
            anonymized: true,
            includes_absent_devices: false,
            devices: order.iter().map(|id| self.devices[id].clone()).collect(),
            windows_update: Some(WindowsUpdateResult {
                criteria: String::new(),
                online: true,
                last_search_utc: None,
                offers: self.offers.clone().unwrap_or_default(),
                error: None,
            }),
            vendors: Vec::new(),
        }
    }

    /// The plan the app would build for devices 1..=count, all ticked.
    fn plan(&self, count: usize) -> Plan {
        let ids: Vec<String> = (1..=count).map(id).collect();
        let snapshot = self.snapshot(&ids);
        let report = decide(&snapshot);
        build_plan(&snapshot, &report, &ids, PASS, Mode::Real).expect("plan")
    }

    fn driver(&self, i: usize) -> Option<InstalledDriver> {
        self.devices[&id(i)].driver.clone()
    }
}

fn id(i: usize) -> String {
    format!("PCI\\DEV{i}\\0")
}

impl SystemActions for Fake {
    fn now(&mut self) -> i64 {
        self.clock += 1;
        self.clock
    }
    fn wait(&mut self, seconds: u64) {
        self.clock += i64::try_from(seconds).unwrap_or(0);
        if !self.later.is_empty() {
            let word = self.later.remove(0);
            if word.is_some() {
                self.control = word;
            }
        }
    }
    fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String> {
        self.offers.clone()
    }
    fn read_device(&mut self, instance_id: &str) -> Result<Option<Device>, String> {
        Ok(self.devices.get(instance_id).cloned())
    }
    fn read_control(&mut self) -> Option<String> {
        self.control.clone()
    }
    fn restore_points(&mut self) -> Result<Vec<i64>, String> {
        Ok(self.points.clone())
    }
    fn driver_signer(&mut self, inf: &str) -> SignerCheck {
        if self.unsigned.contains(inf) {
            SignerCheck::Unsigned
        } else {
            SignerCheck::Signed {
                signer: WINDOWS_UPDATE_SIGNER.to_owned(),
            }
        }
    }
    fn others_with_id(&mut self, id: &str, except: &str) -> Result<Vec<String>, String> {
        let id = id.to_lowercase();
        Ok(self
            .devices
            .values()
            .filter(|d| d.present && d.instance_id != except)
            .filter(|d| {
                d.hardware_ids
                    .iter()
                    .chain(&d.compatible_ids)
                    .any(|x| x.to_lowercase() == id)
            })
            .map(|d| d.instance_id.clone())
            .collect())
    }
    fn inf_used_elsewhere(&mut self, inf: &str, except: &[String]) -> Result<bool, String> {
        Ok(self.devices.values().any(|d| {
            !except.contains(&d.instance_id)
                && d.driver.as_ref().and_then(|x| x.inf_path.as_deref()) == Some(inf)
        }))
    }
    fn begin_restore_point(&mut self, _: &str) -> Result<i64, String> {
        self.changes.push("restore_begin".to_owned());
        self.restore.clone()
    }
    fn end_restore_point(&mut self, seq: i64, cancel: bool) -> Result<(), String> {
        self.changes
            .push(format!("restore_end {seq} cancel={cancel}"));
        // Like Windows: a point is listed only once closed (seen on 2026-11-02).
        if !cancel && self.restore_listed && !self.points.contains(&seq) {
            self.points.push(seq);
        }
        Ok(())
    }
    fn remove_restore_point(&mut self, seq: i64) -> Result<(), String> {
        self.changes.push(format!("restore_remove {seq}"));
        if self.gone_points.contains(&seq) {
            Err(format!("restore point {seq} is not listed by Windows"))
        } else {
            Ok(())
        }
    }
    fn export_driver(&mut self, pass: &str, inf: &str, n: usize) -> Result<String, String> {
        self.changes.push(format!(
            "pnputil {}",
            commands::export_driver(inf, &format!("backup\\{n}")).join(" ")
        ));
        if self.export_fails.contains(inf) {
            return Err("exit code 1".to_owned());
        }
        Ok(format!(
            "C:\\ProgramData\\Drivn\\passes\\{pass}\\backup\\{n}"
        ))
    }
    fn install_update(&mut self, update_id: &str, _: i32) -> InstallOutcome {
        self.changes.push(format!("install {update_id}"));
        let i: usize = update_id
            .trim_start_matches("upd-")
            .parse()
            .unwrap_or_default();
        if let Some(word) = self.click_during.get(update_id) {
            self.control = Some(word.clone());
        }
        if let Some((other, driver)) = self.change_during.get(update_id).cloned()
            && let Some(d) = self.devices.get_mut(&other)
        {
            d.driver = Some(driver);
        }
        let answer = *self.answers.get(update_id).unwrap_or(&Answer::Activate);
        let ok = InstallOutcome {
            succeeded: true,
            hresult: 0,
            reboot_required: answer == Answer::PendingReboot,
            detail: None,
        };
        match answer {
            Answer::Activate => {
                let inf = format!("oem{}.inf", self.next_inf);
                self.next_inf += 1;
                let dev = self.devices.get_mut(&id(i)).expect("device");
                if let Some(old) = dev.driver.take() {
                    self.previous.insert(id(i), old);
                }
                dev.driver = Some(drv(&inf, "2.0.0.0", "2025-06-01", "Contoso", &hwid(i)));
                ok
            }
            Answer::PendingReboot | Answer::NotSelected => ok,
            Answer::Fail(hresult) => InstallOutcome {
                succeeded: false,
                hresult,
                reboot_required: false,
                detail: None,
            },
        }
    }
    fn replace_app(&mut self, id: &str) -> Result<i32, String> {
        self.changes.push(format!("winget replace {id}"));
        if self.winget.is_empty() {
            return Err("cannot run".to_owned());
        }
        Ok(self.winget.remove(0))
    }
    fn upgrade_app(&mut self, id: &str) -> Result<i32, String> {
        self.changes.push(format!("winget {id}"));
        if self.winget.is_empty() {
            return Err("cannot run".to_owned());
        }
        Ok(self.winget.remove(0))
    }
    fn run_official(
        &mut self,
        _: &str,
        n: usize,
        official: &Official,
        file: &str,
    ) -> Result<i32, String> {
        self.changes
            .push(format!("official {n} {} {file}", official.args.join(" ")));
        self.official.ok_or_else(|| "cannot run".to_owned())
    }
    fn rollback_driver(&mut self, instance_id: &str) -> Result<ChangeOutcome, String> {
        self.changes.push(format!(
            "rollback {instance_id} flags={}",
            commands::ROLLBACK_FLAGS
        ));
        if !self.rollback_works {
            return Err("no rollback information".to_owned());
        }
        let previous = self.previous.remove(instance_id).ok_or("nothing kept")?;
        self.devices.get_mut(instance_id).ok_or("device")?.driver = Some(previous);
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn add_driver(&mut self, backup: &str) -> Result<ChangeOutcome, String> {
        self.changes.push(format!(
            "pnputil {}",
            commands::add_driver(&format!("{backup}\\*.inf")).join(" ")
        ));
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn delete_driver(&mut self, inf: &str) -> Result<ChangeOutcome, String> {
        self.changes.push(format!(
            "pnputil {}",
            commands::delete_driver(inf).join(" ")
        ));
        // Windows takes the best driver left: the one put back, if any.
        for (key, dev) in &mut self.devices {
            if dev.driver.as_ref().and_then(|d| d.inf_path.as_deref()) == Some(inf) {
                dev.driver = self.previous.remove(key);
            }
        }
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn remove_backup(&mut self, pass: &str) -> Result<(), String> {
        self.changes.push(format!("remove_backup {pass}"));
        Ok(())
    }
}

#[derive(Default)]
struct Memory(Vec<Line>);

impl Journal for Memory {
    fn append(&mut self, line: &Line) -> Result<(), String> {
        self.0.push(line.clone());
        Ok(())
    }
}

impl Memory {
    fn text(&self) -> String {
        self.0.iter().map(|l| to_line(l).expect("line")).collect()
    }
    fn view(&self) -> PassView {
        reconstruct(&self.text()).expect("journal")
    }
    fn has(&self, f: impl Fn(&Event) -> bool) -> bool {
        self.0.iter().any(|l| f(&l.event))
    }
    fn position(&self, f: impl Fn(&Event) -> bool) -> Option<usize> {
        self.0.iter().position(|l| f(&l.event))
    }
}

fn config() -> Config {
    Config {
        poll_seconds: 600,
        ..Config::default()
    }
}

fn real(fake: &mut Fake, plan: &Plan) -> (Vec<ItemRecord>, Memory) {
    let mut journal = Memory::default();
    let items = run_pass(plan, false, fake, &mut journal, &config()).expect("pass");
    (items, journal)
}

fn statuses(items: &[ItemRecord]) -> Vec<Status> {
    items.iter().map(|i| i.status).collect()
}

// ---------- R-SEC-01 ----------

#[test]
fn r_sec_01_restore_point_failed_goes_on_without_a_question() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.restore = Err("protection off".to_owned());
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed, Status::Installed]);
    // Noted before the first driver, never waited for.
    let noted = journal
        .position(|e| *e == Event::ContinueNoRestore)
        .expect("noted");
    let first_item = journal
        .position(|e| matches!(e, Event::ItemStart { .. }))
        .expect("item");
    assert!(noted < first_item);
    assert!(!journal.has(|e| *e == Event::WaitingRestoreConsent));
    assert!(journal.view().no_restore_chosen);
}

#[test]
fn r_sec_01_new_point_not_yet_in_the_windows_list_is_the_pass_point() {
    // Seen on 2026-11-02: points in System Restore, not yet in the list read right after.
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.restore_listed = false;
    let (items, journal) = real(&mut fake, &plan);
    assert!(journal.has(|e| matches!(
        e,
        Event::RestoreDone {
            restore: Restore::Created { seq: 431 }
        }
    )));
    assert_eq!(statuses(&items), [Status::Installed]);
}

// ---------- R-SEC-02 ----------

#[test]
fn r_sec_02_failed_export_installs_anyway_and_says_so() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    fake.export_fails.insert("oem2.inf".to_owned());
    let (items, mut journal) = real(&mut fake, &plan);
    assert_eq!(
        statuses(&items),
        [Status::Installed, Status::Installed, Status::Installed]
    );
    assert!(items[1].no_backup && items[1].backup.is_none());
    assert!(!items[0].no_backup && items[0].backup.is_some());
    assert!(fake.changes.contains(&"install upd-2".to_owned()));
    // Its rollback: no backup to put back, the new INF removed brings the old one back.
    fake.rollback_works = false;
    fake.changes.clear();
    let status = revert(&mut fake, &mut journal, false, &items[1]).expect("revert");
    assert_eq!(status, Some(Status::Reverted));
    assert_eq!(fake.driver(2), Some(old_driver(2)));
    assert!(!fake.changes.iter().any(|c| c.contains("/add-driver")));
}

#[test]
fn r_sec_02_driver_of_windows_is_installed_without_export() {
    let mut fake = Fake::new(1);
    fake.devices.get_mut(&id(1)).expect("dev").driver = Some(drv(
        "netrtl64.inf",
        "1.0.0.0",
        "2024-01-01",
        "Contoso",
        &hwid(1),
    ));
    let plan = fake.plan(1);
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Installed);
    assert!(items[0].no_backup);
    assert!(!fake.changes.iter().any(|c| c.contains("export")));
    assert!(is_oem_inf("OEM12.INF") && !is_oem_inf("oem.inf") && !is_oem_inf("net.inf"));
}

#[test]
fn r_sec_02_device_without_driver_is_installed_and_reverted_by_removal() {
    let mut fake = Fake::new(1);
    fake.devices.get_mut(&id(1)).expect("dev").driver = None;
    let plan = fake.plan(1);
    let (items, mut journal) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Installed);
    assert!(items[0].backup.is_none() && items[0].old.is_none());
    assert!(!fake.changes.iter().any(|c| c.contains("export")));
    fake.changes.clear();
    let status = revert(&mut fake, &mut journal, false, &items[0]).expect("revert");
    assert_eq!(status, Some(Status::Reverted));
    assert!(fake.driver(1).is_none());
    assert!(!fake.changes.iter().any(|c| c.contains("add-driver")));
    assert!(fake.changes.iter().any(|c| c.contains("/delete-driver")));
}

// ---------- R-SEC-03 ----------

#[test]
fn r_sec_03_each_installed_line_can_revert_this_driver_alone() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    let (_, journal) = real(&mut fake, &plan);
    let ends: Vec<&ItemRecord> = journal
        .0
        .iter()
        .filter_map(|l| match &l.event {
            Event::ItemEnd { record } if record.status == Status::Installed => Some(record),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 2);
    for r in ends {
        let old = r.old.as_ref().expect("old");
        assert!(!r.device_instance_id.is_empty());
        assert!(old.inf.is_some() && old.version.is_some() && old.date.is_some());
        assert!(old.provider.is_some());
        assert!(r.new.inf.as_deref().is_some_and(is_oem_inf));
        assert!(r.new.version.is_some() && r.new.date.is_some() && r.new.provider.is_some());
        assert!(r.backup.as_deref().is_some_and(|b| b.contains("backup")));
    }
}

// ---------- R-SEC-04 ----------

#[test]
fn r_sec_04_no_forcing_flag_in_any_command() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    let (items, mut journal) = real(&mut fake, &plan);
    fake.rollback_works = false; // also run the pnputil path of the rollback
    for item in items.iter().rev() {
        revert(&mut fake, &mut journal, false, item).expect("revert");
    }
    let built = [
        commands::export_driver("oem1.inf", "x"),
        commands::add_driver("x\\*.inf"),
        commands::delete_driver("oem1.inf"),
    ];
    let all: Vec<String> = fake
        .changes
        .iter()
        .cloned()
        .chain(built.iter().map(|c| c.join(" ")))
        .collect();
    for line in &all {
        let lower = line.to_lowercase();
        assert!(!lower.contains("/force") && !lower.contains("install_flag_force"));
        assert!(!lower.contains("installflag_force") && !lower.contains("force_inf"));
    }
    assert_eq!(commands::ROLLBACK_FLAGS & 0x2, 0); // DIIRFLAG_FORCE_INF
    assert!(all.iter().any(|c| c.contains("/delete-driver")));
}

#[test]
fn r_sec_04_not_selected_by_windows_is_a_result_not_an_error() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.answers.insert("upd-1".to_owned(), Answer::NotSelected);
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(
        statuses(&items),
        [Status::NotSelectedByWindows, Status::Installed]
    );
    assert!(items[0].hresult.is_none());
    // Windows Update has no forcing flag: never forced.
    assert!(!items[0].forced);
}

#[test]
fn r_sec_04_failure_of_windows_update_keeps_its_code() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.answers
        .insert("upd-1".to_owned(), Answer::Fail(0x8024_0022_u32 as i32));
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Failed);
    assert_eq!(items[0].hresult, Some(0x8024_0022_u32 as i32));
}

// ---------- R-SEC-05 ----------

fn plan_bytes(fake: &Fake) -> Vec<u8> {
    serde_json::to_vec(&fake.plan(1)).expect("json")
}

#[test]
fn r_sec_05_plan_with_another_hash_is_refused() {
    let fake = Fake::new(1);
    let bytes = plan_bytes(&fake);
    let good = sha256_hex(&bytes);
    assert!(check_copy(PASS, &good, &bytes).is_ok());
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(check_copy(PASS, &good, &changed).is_err());
}

#[test]
fn r_sec_05_bad_or_other_pass_id_is_refused() {
    let fake = Fake::new(1);
    let bytes = plan_bytes(&fake);
    let hash = sha256_hex(&bytes);
    assert!(check_copy("2026-11-01T1410-zzzz", &hash, &bytes).is_err());
    let file = PlanFile {
        reparse_point: false,
        size: 10,
    };
    for bad in [
        "..\\..\\Windows",
        "2026-11-01T1410-A3F9",
        "2026-11-01T1410-a3f9\\x",
        "2026-11-01 1410-a3f9",
        "",
    ] {
        assert!(!valid_pass_id(bad), "{bad}");
        assert!(check_before_copy(bad, false, &file).is_err());
    }
    assert!(valid_pass_id(PASS));
    assert_eq!(
        new_pass_id("2026-11-01T14:10:59Z", "a3f9").as_deref(),
        Some(PASS)
    );
}

#[test]
fn r_sec_05_existing_pass_folder_or_linked_plan_is_refused() {
    let ok = PlanFile {
        reparse_point: false,
        size: 100,
    };
    assert!(check_before_copy(PASS, false, &ok).is_ok());
    assert!(check_before_copy(PASS, true, &ok).is_err());
    let link = PlanFile {
        reparse_point: true,
        size: 100,
    };
    assert!(check_before_copy(PASS, false, &link).is_err());
    let big = PlanFile {
        reparse_point: false,
        size: 2 * 1024 * 1024,
    };
    assert!(check_before_copy(PASS, false, &big).is_err());
}

#[test]
fn r_sec_05_root_with_junction_or_unexpected_owner_is_refused() {
    let folder = |reparse_point, owner: Option<&str>| FolderFacts {
        path: "C:\\ProgramData\\Drivn".to_owned(),
        reparse_point,
        owner_sid: owner.map(str::to_owned),
    };
    assert!(
        check_root(&[
            folder(false, Some("S-1-5-32-544")),
            folder(false, Some("S-1-5-18"))
        ])
        .is_ok()
    );
    assert!(check_root(&[folder(true, Some("S-1-5-32-544"))]).is_err());
    assert!(check_root(&[folder(false, Some("S-1-5-21-1-2-3-1001"))]).is_err());
    assert!(check_root(&[folder(false, None)]).is_err());
}

#[test]
fn r_sec_05_absent_device_is_skipped() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.devices.get_mut(&id(1)).expect("dev").present = false;
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::DeviceAbsent));
    assert_eq!(items[1].status, Status::Installed);
}

#[test]
fn r_sec_05_r_mat_01_other_id_is_skipped() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // Windows now offers this update for another ID: not equal to any ID of the device.
    if let Ok(offers) = fake.offers.as_mut() {
        offers[0].hardware_id = Some("PCI\\VEN_10EC&DEV_9999".to_owned());
    }
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::IdMismatch));
    assert!(!fake.changes.iter().any(|c| c.starts_with("install")));
}

#[test]
fn r_sec_05_driver_changed_between_plan_and_driver_n_is_skipped() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    // While driver 1 installs, something else updates device 2.
    fake.change_during.insert(
        "upd-1".to_owned(),
        (
            id(2),
            drv("oem77.inf", "1.5.0.0", "2024-06-01", "Contoso", &hwid(2)),
        ),
    );
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Installed);
    assert_eq!(items[1].skip, Some(Skip::DriverChanged));
}

#[test]
fn r_sec_05_r_ver_01_offer_no_longer_newer_is_skipped() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // Same update, but Windows now says it is 1.0.0.0: not newer than the installed driver.
    fake.offers = Ok(vec![offer(1, "1.0.0.0")]);
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::NoLongerNewer));
}

#[test]
fn r_sec_05_offer_missing_from_the_new_search_is_skipped() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.offers = Ok(Vec::new());
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::OfferGone));
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.offers = Err("no network".to_owned());
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::OfferGone));
}

#[test]
fn r_sec_05_source_other_than_windows_update_is_skipped() {
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.items[0].source = "contoso.example".to_owned();
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::Source));
}

// ---------- R-SEC-06 ----------

/// A full pass, cut after each journal line (and in the middle of the next one).
#[test]
fn r_sec_06_journal_cut_anywhere_forgets_no_driver() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    fake.click_during
        .insert("upd-3".to_owned(), "cancel_revert".to_owned());
    let (_, journal) = real(&mut fake, &plan);
    let full = journal.text();
    let ends: Vec<usize> = full.match_indices('\n').map(|(i, _)| i + 1).collect();
    for (k, end) in ends.iter().enumerate() {
        let next = ends.get(k + 1).copied().unwrap_or(full.len());
        for cut in [*end, end + (next - end) / 2] {
            let view = reconstruct(&full[..cut]).expect("rebuilt");
            for n in 1..=3 {
                let finished = view.items.iter().any(|i| i.n == n);
                let started = view.started.as_ref().is_some_and(|s| s.n == n);
                let not_started = n > view.last_n();
                assert_eq!(
                    u8::from(finished) + u8::from(started) + u8::from(not_started),
                    1,
                    "cut at {cut}, driver {n}"
                );
            }
            if cut < full.len() {
                assert!(view.interrupted(), "cut at {cut}");
            }
        }
    }
    assert!(!reconstruct(&full).expect("full").interrupted());
}

#[test]
fn r_sec_06_resume_reads_the_cut_driver_and_goes_on() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    let (_, journal) = real(&mut fake, &plan);
    // Cut right after "install" of driver 2 started: Windows had time to install it.
    let cut = journal
        .position(|e| {
            matches!(
                e,
                Event::StepEnd {
                    n: 2,
                    step: Step::Install,
                    ..
                }
            )
        })
        .expect("step");
    // Driver 3 never ran: the fake goes back to before it.
    let mut fake3 = Fake::new(3);
    for i in 1..=2 {
        let new = fake.driver(i);
        fake3.devices.get_mut(&id(i)).expect("dev").driver = new;
    }
    let mut cut_journal = Memory(journal.0[..cut].to_vec());
    let view = cut_journal.view();
    assert!(view.interrupted());
    assert_eq!(view.started.as_ref().map(|s| s.n), Some(2));
    let items =
        resume_pass(&plan, &view, None, &mut fake3, &mut cut_journal, &config()).expect("resume");
    assert_eq!(
        statuses(&items),
        [Status::Installed, Status::Installed, Status::Installed]
    );
    assert!(items[1].after_interruption);
    assert!(items[1].backup.is_some());
    let after = cut_journal.view();
    assert!(!after.interrupted());
    // The point was closed as soon as it was made: the resume neither closes nor removes it.
    assert!(!fake3.changes.iter().any(|c| c.starts_with("restore_")));
    assert_eq!(after.restore_seq(), Some(431));
}

#[test]
fn r_sec_06_cut_driver_still_old_is_not_installed_and_unknown_otherwise() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    let (_, journal) = real(&mut fake, &plan);
    let cut = journal
        .position(|e| {
            matches!(
                e,
                Event::StepStart {
                    n: 1,
                    step: Step::Install
                }
            )
        })
        .expect("step")
        + 1;
    // Windows had not changed anything yet: the old driver is active.
    let mut untouched = Fake::new(2);
    let mut j = Memory(journal.0[..cut].to_vec());
    let view = j.view();
    let items = resume_pass(
        &plan,
        &view,
        Some(StopMode::Keep),
        &mut untouched,
        &mut j,
        &config(),
    )
    .expect("resume");
    assert_eq!(items[0].status, Status::NotInstalled);
    assert_eq!(items.len(), 1, "driver 2 not started, not touched");
    // A third driver nobody expected.
    let mut other = Fake::new(2);
    other.devices.get_mut(&id(1)).expect("dev").driver =
        Some(drv("oem90.inf", "3.0.0.0", "2025-09-09", "Other", &hwid(1)));
    let mut j = Memory(journal.0[..cut].to_vec());
    let items = resume_pass(
        &plan,
        &j.view(),
        Some(StopMode::Keep),
        &mut other,
        &mut j,
        &config(),
    )
    .expect("resume");
    assert_eq!(items[0].status, Status::Unknown);
}

#[test]
fn r_sec_06_revert_cut_in_the_middle_is_finished_on_resume() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (items, mut journal) = real(&mut fake, &plan);
    // The rollback started, Windows put the old driver back, then the PC went off.
    journal.0.push(Line {
        at: 0,
        simulation: false,
        event: Event::RevertStart { n: 1 },
    });
    fake.rollback_driver(&id(1)).expect("rollback");
    let view = journal.view();
    assert!(view.interrupted() && view.reverting == Some(1));
    let after = resume_pass(
        &plan,
        &view,
        Some(StopMode::Keep),
        &mut fake,
        &mut journal,
        &config(),
    )
    .expect("resume");
    assert_eq!(after[0].status, Status::Reverted);
    assert_eq!(items[0].status, Status::Installed);
    assert!(!journal.view().interrupted());
}

#[test]
fn r_sec_06_truncated_last_line_is_ignored_but_a_bad_middle_line_is_not() {
    let good = to_line(&Line {
        at: 1,
        simulation: false,
        event: Event::PassStart {
            pass_id: PASS.to_owned(),
            items: 1,
        },
    })
    .expect("line");
    assert_eq!(
        parse(&format!("{good}{{\"at\":2,\"simu"))
            .expect("ok")
            .len(),
        1
    );
    assert!(parse(&format!("{{oops\n{good}")).is_err());
}

// ---------- R-SEC-07 ----------

/// A finished pass with an ID of day `day`.
fn pass_view(day: u32, real: bool, restore: bool, backup: bool, ended: bool) -> PassView {
    let pass_id = format!("2026-10-{day:02}T1000-aaaa");
    let mut lines = vec![Line {
        at: 1,
        simulation: !real,
        event: Event::PassStart {
            pass_id: pass_id.clone(),
            items: 1,
        },
    }];
    if restore {
        lines.push(Line {
            at: 2,
            simulation: !real,
            event: Event::RestoreDone {
                restore: Restore::Created {
                    seq: i64::from(day),
                },
            },
        });
    }
    lines.push(Line {
        at: 3,
        simulation: !real,
        event: Event::ItemEnd {
            record: ItemRecord {
                n: 1,
                device_instance_id: id(1),
                name: None,
                status: if real {
                    Status::Installed
                } else {
                    Status::Simulated
                },
                skip: None,
                old: None,
                new: Default::default(),
                backup: backup.then(|| "backup\\1".to_owned()),
                hresult: None,
                reboot_required: false,
                signer: None,
                after_interruption: false,
                detail: None,
                twins: Vec::new(),
                no_backup: false,
                forced: false,
            },
        },
    });
    if ended {
        lines.push(Line {
            at: 4,
            simulation: !real,
            event: Event::PassEnd {
                not_started: 0,
                stopped: None,
            },
        });
    }
    reconstruct(
        &lines
            .iter()
            .map(|l| to_line(l).expect("l"))
            .collect::<String>(),
    )
    .expect("v")
}

#[test]
fn r_sec_07_sixth_real_pass_prunes_the_oldest() {
    let views: Vec<PassView> = (1..=6)
        .map(|d| pass_view(d, true, true, true, true))
        .collect();
    let newest = views[5].pass_id.clone();
    let pruned = to_prune(&views, &newest);
    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0].pass_id, views[0].pass_id);
    assert_eq!(KEEP_PASSES, 5);
}

#[test]
fn r_sec_07_simulated_interrupted_or_empty_passes_never_count() {
    let mut views: Vec<PassView> = (1..=5)
        .map(|d| pass_view(d + 10, true, true, true, true))
        .collect();
    views.push(pass_view(1, false, false, false, true)); // simulated
    views.push(pass_view(2, true, true, true, false)); // interrupted
    views.push(pass_view(3, true, false, false, true)); // nothing to keep
    let newest = views[4].pass_id.clone();
    assert!(to_prune(&views, &newest).is_empty());
    views.push(pass_view(20, true, true, true, true));
    let pruned = to_prune(&views, "2026-10-20T1000-aaaa");
    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0].pass_id, "2026-10-11T1000-aaaa");
}

#[test]
fn r_sec_07_no_pruning_when_the_new_pass_failed_its_safety_nets() {
    let mut views: Vec<PassView> = (1..=5)
        .map(|d| pass_view(d, true, true, true, true))
        .collect();
    views.push(pass_view(9, true, false, false, true));
    assert!(to_prune(&views, "2026-10-09T1000-aaaa").is_empty());
}

#[test]
fn r_sec_07_cancelled_point_is_never_removed() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // The driver changed since the plan: skipped, nothing installed.
    fake.devices.get_mut(&id(1)).expect("dev").driver = Some(drv(
        "oem77.inf",
        "1.5.0.0",
        "2024-06-01",
        "Contoso",
        &hwid(1),
    ));
    let (_, journal) = real(&mut fake, &plan);
    // Nothing went in: the point is removed, and this number is no longer the pass's.
    assert!(fake.changes.contains(&"restore_remove 431".to_owned()));
    assert_eq!(journal.view().restore_seq(), None);

    struct Store(Vec<(String, String)>);
    impl PassStore for Store {
        fn passes(&mut self) -> Vec<(String, String)> {
            self.0.clone()
        }
        fn append(&mut self, pass: &str, line: &Line) -> Result<(), String> {
            let entry = self.0.iter_mut().find(|(p, _)| p == pass).ok_or("pass")?;
            entry.1.push_str(&to_line(line)?);
            Ok(())
        }
    }
    // Oldest pass: point 1 cancelled, one backup.
    let mut oldest = journal.text().replace(PASS, "2026-10-01T1000-aaaa");
    oldest = oldest.replace("\"status\":\"skipped\"", "\"status\":\"installed\"");
    oldest = oldest.replace("\"backup\":null", "\"backup\":\"b\"");
    let mut passes = vec![("2026-10-01T1000-aaaa".to_owned(), oldest)];
    for d in 2..=7 {
        let v = pass_view(d, true, true, true, true);
        let mut t = String::new();
        for line in [
            Event::PassStart {
                pass_id: v.pass_id.clone(),
                items: 1,
            },
            Event::RestoreDone {
                restore: Restore::Created { seq: i64::from(d) },
            },
            Event::PassEnd {
                not_started: 0,
                stopped: None,
            },
        ] {
            t.push_str(
                &to_line(&Line {
                    at: 1,
                    simulation: false,
                    event: line,
                })
                .expect("l"),
            );
        }
        passes.push((v.pass_id, t));
    }
    let mut store = Store(passes);
    let mut sys = Fake::new(0);
    sys.points = (1..=7).chain([431]).collect();
    prune(&mut store, &mut sys, "2026-10-07T1000-aaaa");
    // 431 was cancelled: never Drivn's to remove. Pass 1 is the only one with backups: kept.
    assert!(!sys.changes.iter().any(|c| c == "restore_remove 431"));
    assert!(!sys.changes.iter().any(|c| c.starts_with("remove_backup")));
    // ADR-0038: Drivn's 5 newest points stay (3 to 7); the oldest, 2, goes once.
    assert_eq!(
        sys.changes
            .iter()
            .filter(|c| c.starts_with("restore_remove"))
            .collect::<Vec<_>>(),
        ["restore_remove 2"]
    );
    let first = reconstruct(&store.0[0].1).expect("v");
    assert!(!first.pruned);
    let second = reconstruct(&store.0[1].1).expect("v");
    assert_eq!(second.restore_seq(), None);
}

#[test]
fn r_sec_07_a_point_windows_already_deleted_is_noted_gone_without_a_removal() {
    struct Store(Vec<(String, String)>);
    impl PassStore for Store {
        fn passes(&mut self) -> Vec<(String, String)> {
            self.0.clone()
        }
        fn append(&mut self, pass: &str, line: &Line) -> Result<(), String> {
            let entry = self.0.iter_mut().find(|(p, _)| p == pass).ok_or("pass")?;
            entry.1.push_str(&to_line(line)?);
            Ok(())
        }
    }
    let passes = (1..=6)
        .map(|d| {
            let id = format!("2026-10-{d:02}T1000-aaaa");
            let text: String = [
                Event::PassStart {
                    pass_id: id.clone(),
                    items: 0,
                },
                Event::RestoreDone {
                    restore: Restore::Created { seq: i64::from(d) },
                },
                Event::PassEnd {
                    not_started: 0,
                    stopped: None,
                },
            ]
            .into_iter()
            .map(|event| {
                to_line(&Line {
                    at: 1,
                    simulation: false,
                    event,
                })
                .expect("l")
            })
            .collect();
            (id, text)
        })
        .collect();
    let mut store = Store(passes);
    let mut sys = Fake::new(0);
    // Windows dropped point 1 itself (no disk space left for it): the removal is tried, fails,
    // and the point is absent from the list.
    sys.points = (2..=6).collect();
    sys.gone_points = vec![1];
    prune(&mut store, &mut sys, "2026-10-06T1000-aaaa");
    let removals = |sys: &Fake| {
        sys.changes
            .iter()
            .filter(|c| c.starts_with("restore_remove"))
            .count()
    };
    assert_eq!(removals(&sys), 1);
    let oldest = reconstruct(&store.0[0].1).expect("v");
    assert_eq!(oldest.restore_seq(), None);
    // Next pass: nothing is asked again for it.
    prune(&mut store, &mut sys, "2026-10-06T1000-aaaa");
    assert_eq!(removals(&sys), 1);
}

#[test]
fn r_sec_07_a_failed_removal_of_a_listed_point_is_retried() {
    struct Store(Vec<(String, String)>);
    impl PassStore for Store {
        fn passes(&mut self) -> Vec<(String, String)> {
            self.0.clone()
        }
        fn append(&mut self, pass: &str, line: &Line) -> Result<(), String> {
            let entry = self.0.iter_mut().find(|(p, _)| p == pass).ok_or("pass")?;
            entry.1.push_str(&to_line(line)?);
            Ok(())
        }
    }
    let passes = (1..=6)
        .map(|d| {
            let id = format!("2026-10-{d:02}T1000-aaaa");
            let text: String = [
                Event::PassStart {
                    pass_id: id.clone(),
                    items: 0,
                },
                Event::RestoreDone {
                    restore: Restore::Created { seq: i64::from(d) },
                },
                Event::PassEnd {
                    not_started: 0,
                    stopped: None,
                },
            ]
            .into_iter()
            .map(|event| {
                to_line(&Line {
                    at: 1,
                    simulation: false,
                    event,
                })
                .expect("l")
            })
            .collect();
            (id, text)
        })
        .collect();
    let mut store = Store(passes);
    let mut sys = Fake::new(0);
    // Point 1 is still listed (or its description is not Drivn's): the refusal is kept, retried.
    sys.points = (1..=6).collect();
    sys.gone_points = vec![1];
    prune(&mut store, &mut sys, "2026-10-06T1000-aaaa");
    let oldest = reconstruct(&store.0[0].1).expect("v");
    assert_eq!(oldest.restore_seq(), Some(1));
}

// ---------- R-SEC-08 ----------

#[test]
fn r_sec_08_pause_during_a_driver_takes_effect_after_it() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    fake.click_during
        .insert("upd-1".to_owned(), "pause".to_owned());
    fake.later = vec![None, Some("run".to_owned())];
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed; 3]);
    let end1 = journal
        .position(|e| matches!(e, Event::ItemEnd { record } if record.n == 1))
        .expect("end 1");
    let paused = journal.position(|e| *e == Event::Paused).expect("paused");
    let resumed = journal.position(|e| *e == Event::Resumed).expect("resumed");
    let start2 = journal
        .position(|e| matches!(e, Event::ItemStart { n: 2, .. }))
        .expect("start 2");
    assert!(end1 < paused && paused < resumed && resumed < start2);
}

#[test]
fn r_sec_08_revert_all_goes_last_to_first_and_leaves_not_started_alone() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    fake.click_during
        .insert("upd-2".to_owned(), "cancel_revert".to_owned());
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Reverted, Status::Reverted]);
    let reverts: Vec<usize> = journal
        .0
        .iter()
        .filter_map(|l| match l.event {
            Event::RevertStart { n } => Some(n),
            _ => None,
        })
        .collect();
    assert_eq!(reverts, [2, 1]);
    assert!(!fake.changes.contains(&"install upd-3".to_owned()));
    assert_eq!(fake.driver(3), Some(old_driver(3)));
    assert_eq!(fake.driver(1), Some(old_driver(1)));
    assert!(journal.has(|e| matches!(
        e,
        Event::PassEnd {
            not_started: 1,
            stopped: Some(StopMode::Revert)
        }
    )));
}

#[test]
fn r_sec_08_cancel_keep_keeps_what_is_installed() {
    let mut fake = Fake::new(3);
    let plan = fake.plan(3);
    fake.click_during
        .insert("upd-1".to_owned(), "cancel_keep".to_owned());
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed]);
    assert_eq!(fake.driver(2), Some(old_driver(2)));
}

#[test]
fn r_sec_08_unknown_or_out_of_state_word_is_ignored_and_never_written() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.click_during
        .insert("upd-1".to_owned(), "secret-word-123".to_owned());
    fake.click_during
        .insert("upd-2".to_owned(), "continue_no_restore".to_owned());
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed, Status::Installed]);
    assert!(journal.has(|e| *e == Event::InvalidOrder));
    assert!(!journal.text().contains("secret-word-123"));
    assert!(!accepted(Order::Run, Waiting::Running));
    assert!(!accepted(Order::ContinueNoRestore, Waiting::Paused));
    assert!(accepted(Order::CancelRevert, Waiting::Paused));
    assert_eq!(Order::parse(" pause\r\n"), Some(Order::Pause));
}

#[test]
fn r_sec_08_pause_without_order_for_24_hours_stops_as_cancel_keep() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.click_during
        .insert("upd-1".to_owned(), "pause".to_owned());
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed]);
    assert!(journal.has(|e| *e == Event::WaitTimeout));
    assert!(journal.has(|e| matches!(
        e,
        Event::PassEnd {
            stopped: Some(StopMode::Keep),
            ..
        }
    )));
}

// ---------- R-SRC-04 ----------

#[test]
fn r_src_04_unsigned_active_driver_is_put_back() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.unsigned.insert("oem50.inf".to_owned());
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(
        statuses(&items),
        [Status::RolledBackUnsigned, Status::Installed]
    );
    assert_eq!(fake.driver(1), Some(old_driver(1)));
    assert_eq!(journal.view().items[0].status, Status::RolledBackUnsigned);
}

#[test]
fn r_src_04_unexpected_signer_is_put_back_only_when_enforced() {
    struct Other(Fake);
    impl SystemActions for Other {
        fn now(&mut self) -> i64 {
            self.0.now()
        }
        fn wait(&mut self, s: u64) {
            self.0.wait(s)
        }
        fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String> {
            self.0.search_offers()
        }
        fn read_device(&mut self, i: &str) -> Result<Option<Device>, String> {
            self.0.read_device(i)
        }
        fn read_control(&mut self) -> Option<String> {
            self.0.read_control()
        }
        fn restore_points(&mut self) -> Result<Vec<i64>, String> {
            self.0.restore_points()
        }
        fn driver_signer(&mut self, _: &str) -> SignerCheck {
            SignerCheck::Signed {
                signer: "Someone Else".to_owned(),
            }
        }
        fn others_with_id(&mut self, i: &str, e: &str) -> Result<Vec<String>, String> {
            self.0.others_with_id(i, e)
        }
        fn inf_used_elsewhere(&mut self, i: &str, e: &[String]) -> Result<bool, String> {
            self.0.inf_used_elsewhere(i, e)
        }
        fn begin_restore_point(&mut self, d: &str) -> Result<i64, String> {
            self.0.begin_restore_point(d)
        }
        fn end_restore_point(&mut self, s: i64, c: bool) -> Result<(), String> {
            self.0.end_restore_point(s, c)
        }
        fn remove_restore_point(&mut self, s: i64) -> Result<(), String> {
            self.0.remove_restore_point(s)
        }
        fn export_driver(&mut self, p: &str, i: &str, n: usize) -> Result<String, String> {
            self.0.export_driver(p, i, n)
        }
        fn install_update(&mut self, u: &str, r: i32) -> InstallOutcome {
            self.0.install_update(u, r)
        }
        fn rollback_driver(&mut self, i: &str) -> Result<ChangeOutcome, String> {
            self.0.rollback_driver(i)
        }
        fn add_driver(&mut self, b: &str) -> Result<ChangeOutcome, String> {
            self.0.add_driver(b)
        }
        fn delete_driver(&mut self, i: &str) -> Result<ChangeOutcome, String> {
            self.0.delete_driver(i)
        }
        fn remove_backup(&mut self, p: &str) -> Result<(), String> {
            self.0.remove_backup(p)
        }
    }
    for enforce in [false, true] {
        let fake = Fake::new(1);
        let plan = fake.plan(1);
        let mut sys = Other(fake);
        let mut journal = Memory::default();
        let cfg = Config {
            enforce_signer: enforce,
            ..config()
        };
        let items = run_pass(&plan, false, &mut sys, &mut journal, &cfg).expect("pass");
        let expected = if enforce {
            Status::RolledBackUnsigned
        } else {
            Status::Installed
        };
        assert_eq!(journal.view().items[0].status, expected);
        assert_eq!(items[0].signer.as_deref(), Some("Someone Else"));
    }
}

#[test]
fn r_src_04_pending_reboot_is_checked_after_restart_and_not_reverted_before() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.answers
        .insert("upd-1".to_owned(), Answer::PendingReboot);
    let (items, mut journal) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::InstalledPendingReboot);
    let view = journal.view();
    let end = view.ended_at.expect("ended");
    // Before the restart: refused, "restart first".
    assert_eq!(
        rollback_one(&view, 1, end - 100, &mut fake, &mut journal).expect("rb"),
        None
    );
    assert!(journal.has(|e| matches!(
        e,
        Event::RevertEnd {
            refusal: Some(Refusal::RebootFirst),
            ..
        }
    )));
    let new = drv("oem60.inf", "2.0.0.0", "2025-06-01", "Contoso", &hwid(1));
    let signed = |_: &str| SignerCheck::Signed {
        signer: WINDOWS_UPDATE_SIGNER.to_owned(),
    };
    assert_eq!(
        after_reboot(&view, 1, end - 100, Some(&new), signed),
        Some(AfterReboot::Waiting)
    );
    assert_eq!(
        after_reboot(&view, 1, end + 100, Some(&new), |_| SignerCheck::Unsigned),
        Some(AfterReboot::Unsigned)
    );
    assert!(matches!(
        after_reboot(&view, 1, end + 100, Some(&new), signed),
        Some(AfterReboot::Verified { .. })
    ));
    assert_eq!(
        after_reboot(&view, 1, end + 100, Some(&old_driver(1)), signed),
        Some(AfterReboot::NotActive)
    );
    // After the restart, the new driver is active: the rollback is allowed.
    fake.previous.insert(id(1), old_driver(1));
    fake.devices.get_mut(&id(1)).expect("dev").driver = Some(new);
    assert_eq!(
        rollback_one(&view, 1, end + 100, &mut fake, &mut journal).expect("rb"),
        Some(Status::Reverted)
    );
}

// ---------- R-RBT-01 ----------

#[test]
fn r_rbt_01_restart_offered_only_when_a_result_asks_for_it() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    let (_, journal) = real(&mut fake, &plan);
    assert!(reboot_needed(&journal.view()).is_empty());

    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    fake.answers
        .insert("upd-2".to_owned(), Answer::PendingReboot);
    let (_, journal) = real(&mut fake, &plan);
    let view = journal.view();
    assert_eq!(reboot_needed(&view), ["Carte 2"]);
    let end = view.ended_at.expect("end");
    assert!(reboot_pending(&view, end - 10));
    assert!(!reboot_pending(&view, end + 10));
}

// ---------- R-TST-01 ----------

#[test]
fn r_tst_01_simulation_changes_nothing_and_follows_the_same_steps() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(2);
    let mut sim_journal = Memory::default();
    let mut sim = Simulated(Fake::new(2));
    let items = run_pass(&plan, true, &mut sim, &mut sim_journal, &config()).expect("pass");
    assert_eq!(statuses(&items), [Status::Simulated, Status::Simulated]);
    assert!(sim.0.changes.is_empty(), "{:?}", sim.0.changes);
    assert!(sim_journal.0.iter().all(|l| l.simulation));
    assert_eq!(sim.0.driver(1), Some(old_driver(1)));

    // Same steps as a real pass, up to the call that changes the system.
    let (_, real_journal) = real(&mut fake, &plan);
    let steps = |j: &Memory| -> Vec<(usize, Step)> {
        j.0.iter()
            .filter_map(|l| match l.event {
                Event::StepStart { n, step } => Some((n, step)),
                _ => None,
            })
            .filter(|(_, s)| matches!(s, Step::Verify | Step::Backup | Step::Install))
            .collect()
    };
    assert_eq!(steps(&sim_journal), steps(&real_journal));
}

#[test]
fn r_tst_01_without_real_mode_file_it_is_a_simulation() {
    assert!(simulation(Mode::Real, false));
    assert!(simulation(Mode::Simulation, true));
    assert!(!simulation(Mode::Real, true));
    assert_eq!(plan_mode(false), Mode::Simulation);
    let mut sim = Simulated(Fake::new(1));
    assert!(sim.begin_restore_point("x").is_err());
    assert!(sim.export_driver(PASS, "oem1.inf", 1).is_err());
    assert!(!sim.install_update("upd-1", 201).succeeded);
    assert!(sim.delete_driver("oem1.inf").is_err());
    assert!(sim.0.changes.is_empty());
}

#[test]
fn r_tst_01_simulated_pass_is_never_shown_as_real() {
    let plan = Fake::new(1).plan(1);
    let mut sim = Simulated(Fake::new(1));
    let mut journal = Memory::default();
    run_pass(&plan, true, &mut sim, &mut journal, &config()).expect("pass");
    let view = journal.view();
    assert!(view.simulation);
    assert!(view.items.iter().all(|i| !i.status.installed()));
    assert!(reboot_needed(&view).is_empty());
    assert!(journal.has(|e| matches!(
        e,
        Event::RestoreDone {
            restore: Restore::Simulated
        }
    )));
}

// ---------- R-DRT-01 ----------

#[test]
fn r_drt_01_installer_writable_by_users_is_not_launched_in_real_mode() {
    assert!(may_launch(Mode::Real, true).is_err());
    assert!(may_launch(Mode::Real, false).is_ok());
    assert!(may_launch(Mode::Simulation, true).is_ok());
}

// ---------- Rollback ----------

#[test]
fn rollback_refused_when_the_driver_changed_since() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (items, mut journal) = real(&mut fake, &plan);
    fake.devices.get_mut(&id(1)).expect("dev").driver = Some(drv(
        "oem50.inf",
        "3.0.0.0",
        "2025-12-01",
        "Contoso",
        &hwid(1),
    ));
    fake.changes.clear();
    let status = revert(&mut fake, &mut journal, false, &items[0]).expect("revert");
    assert_eq!(status, None);
    assert!(journal.has(|e| matches!(
        e,
        Event::RevertEnd {
            refusal: Some(Refusal::Changed),
            ..
        }
    )));
    assert!(fake.changes.is_empty(), "never below a newer version");
}

#[test]
fn rollback_uses_the_backup_when_windows_kept_nothing() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (items, mut journal) = real(&mut fake, &plan);
    fake.rollback_works = false;
    let status = revert(&mut fake, &mut journal, false, &items[0]).expect("revert");
    assert_eq!(status, Some(Status::Reverted));
    let add = fake.changes.iter().position(|c| c.contains("/add-driver"));
    let del = fake
        .changes
        .iter()
        .position(|c| c.contains("/delete-driver"));
    assert!(add.is_some() && add < del);
}

#[test]
fn rollback_refused_when_the_inf_serves_another_device() {
    let mut fake = Fake::new(2);
    let plan = fake.plan(1);
    let (items, mut journal) = real(&mut fake, &plan);
    fake.rollback_works = false;
    let shared = fake.driver(1);
    fake.devices.get_mut(&id(2)).expect("dev").driver = shared;
    let status = revert(&mut fake, &mut journal, false, &items[0]).expect("revert");
    assert_eq!(status, None);
    assert!(journal.has(|e| matches!(
        e,
        Event::RevertEnd {
            refusal: Some(Refusal::InfShared),
            ..
        }
    )));
}

#[test]
fn plan_holds_only_updates_with_a_windows_update_identity() {
    let fake = Fake::new(2);
    let ids = [id(1), id(2)];
    let snapshot = fake.snapshot(&ids);
    let report = decide(&snapshot);
    let plan = build_plan(&snapshot, &report, &[id(2)], PASS, Mode::Simulation).expect("plan");
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].update_id, "upd-2");
    assert_eq!(
        plan.items[0]
            .installed
            .as_ref()
            .and_then(|d| d.inf.as_deref()),
        Some("oem2.inf")
    );
    let mut no_identity = snapshot.clone();
    if let Some(wu) = no_identity.windows_update.as_mut() {
        wu.offers[0].update_id = None;
    }
    let report = decide(&no_identity);
    assert!(build_plan(&no_identity, &report, &[id(1)], PASS, Mode::Real).is_err());
    assert!(build_plan(&snapshot, &decide(&snapshot), &[], PASS, Mode::Real).is_err());
}

// ---------- Fixes after the first review (2026-11-01) ----------

#[test]
fn r_sec_07_older_restore_point_number_is_never_taken_as_the_pass_point() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // Within 24 h, Windows answers with the number of the point it already has.
    fake.restore = Ok(100);
    fake.later = vec![Some("continue_no_restore".to_owned())];
    let (_, journal) = real(&mut fake, &plan);
    assert!(journal.has(|e| matches!(
        e,
        Event::RestoreDone {
            restore: Restore::Failed { .. }
        }
    )));
    assert_eq!(journal.view().restore_seq(), None);
    assert!(
        !fake
            .changes
            .iter()
            .any(|c| c.starts_with("restore_end 100"))
    );
}

#[test]
fn r_sec_07_no_pruning_after_a_pass_without_restore_point() {
    let mut views: Vec<PassView> = (1..=5)
        .map(|d| pass_view(d, true, true, true, true))
        .collect();
    // "Continue without a restore point": backups only.
    views.push(pass_view(9, true, false, true, true));
    assert!(to_prune(&views, "2026-10-09T1000-aaaa").is_empty());
}

#[test]
fn r_sec_07_restore_point_kept_when_a_driver_touched_the_system() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.answers.insert("upd-1".to_owned(), Answer::Fail(5));
    real(&mut fake, &plan);
    assert!(
        fake.changes
            .contains(&"restore_end 431 cancel=false".to_owned())
    );
}

#[test]
fn r_src_04_unsigned_driver_that_cannot_be_put_back_is_never_shown_as_fine() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.unsigned.insert("oem50.inf".to_owned());
    fake.rollback_works = false;
    // The backup path fails too: the INF also serves another device.
    let other = drv("oem50.inf", "2.0.0.0", "2025-06-01", "Contoso", &hwid(9));
    let mut second = device(9, Some(other));
    second.instance_id = "PCI\\OTHER\\0".to_owned();
    fake.devices.insert(second.instance_id.clone(), second);
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::UnsignedNotReverted);
    assert_eq!(journal.view().items[0].status, Status::UnsignedNotReverted);
    assert!(
        fake.changes
            .contains(&"restore_end 431 cancel=false".to_owned())
    );
}

#[test]
fn r_src_04_driver_found_active_after_a_cut_gets_its_signature_check() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (_, journal) = real(&mut fake, &plan);
    let cut = journal
        .position(|e| {
            matches!(
                e,
                Event::StepEnd {
                    n: 1,
                    step: Step::Install,
                    ..
                }
            )
        })
        .expect("step");
    let mut j = Memory(journal.0[..cut].to_vec());
    fake.unsigned.insert("oem50.inf".to_owned());
    let items = resume_pass(&plan, &j.view(), None, &mut fake, &mut j, &config()).expect("resume");
    assert_eq!(items[0].status, Status::RolledBackUnsigned);
    assert_eq!(fake.driver(1), Some(old_driver(1)));
}

#[test]
fn r_sec_01_resume_after_a_failed_point_goes_on_without_a_question() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.restore = Err("off".to_owned());
    let (_, journal) = real(&mut fake, &plan);
    // Cut right after "restore failed".
    let cut = journal
        .position(|e| matches!(e, Event::RestoreDone { .. }))
        .expect("done")
        + 1;
    let mut j = Memory(journal.0[..cut].to_vec());
    let mut again = Fake::new(1);
    again.restore = Err("off".to_owned());
    let items = resume_pass(&plan, &j.view(), None, &mut again, &mut j, &config()).expect("resume");
    assert_eq!(statuses(&items), [Status::Installed]);
    assert!(!j.has(|e| *e == Event::WaitingRestoreConsent));
}

#[test]
fn r_sec_08_word_written_before_the_pass_is_not_an_order() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // Written by another program before the installer started.
    fake.control = Some("cancel_keep".to_owned());
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed]);
    assert!(accepted(Order::CancelKeep, Waiting::RestoreConsent));
    assert!(!accepted(Order::Run, Waiting::RestoreConsent));
}

#[test]
fn r_rbt_01_installed_driver_that_asks_for_a_restart_offers_it() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (_, journal) = real(&mut fake, &plan);
    let mut text = journal.text();
    text = text.replace("\"reboot_required\":false", "\"reboot_required\":true");
    let view = reconstruct(&text).expect("view");
    assert_eq!(reboot_needed(&view), ["Carte 1"]);
}

// ---------- Fixes after the second review (2026-11-01) ----------

#[test]
fn r_sec_02_twin_with_another_driver_blocks_the_install() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // A second card with the same ID but another driver: its backup is not made.
    let mut twin = device(
        1,
        Some(drv(
            "oem33.inf",
            "1.0.0.0",
            "2024-01-01",
            "Contoso",
            &hwid(1),
        )),
    );
    twin.instance_id = "PCI\\DEV1\\1".to_owned();
    fake.devices.insert(twin.instance_id.clone(), twin);
    let (items, _) = real(&mut fake, &plan);
    assert_eq!(items[0].skip, Some(Skip::SharedId));
    assert!(!fake.changes.iter().any(|c| c.starts_with("install")));
}

#[test]
fn r_sec_02_twins_with_the_same_driver_are_installed_and_put_back_together() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    // Like the two "Alienware Command Center" devices of the reference PC: same ID, same oem1.inf.
    let mut twin = device(1, Some(old_driver(1)));
    twin.instance_id = "PCI\\DEV1\\1".to_owned();
    fake.devices.insert(twin.instance_id.clone(), twin.clone());
    let (items, mut journal) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Installed);
    assert_eq!(items[0].twins, [twin.instance_id.clone()]);
    // Windows updated the twin too (the fake does it by hand here).
    let new = fake.driver(1);
    fake.devices
        .get_mut(&twin.instance_id)
        .expect("twin")
        .driver = new;
    fake.rollback_works = false;
    let status = revert(&mut fake, &mut journal, false, &items[0]).expect("revert");
    assert_eq!(status, Some(Status::Reverted));
    assert_eq!(fake.driver(1), Some(old_driver(1)));
    assert!(
        fake.devices[&twin.instance_id]
            .driver
            .as_ref()
            .is_none_or(|d| d.inf_path.as_deref() != Some("oem50.inf"))
    );
}

#[test]
fn r_vrai_01_driver_changed_by_a_failed_install_is_unknown_not_failed() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.answers.insert("upd-1".to_owned(), Answer::Fail(1));
    // WUA said "failed", but the device now runs another driver.
    fake.change_during.insert(
        "upd-1".to_owned(),
        (
            id(1),
            drv("oem61.inf", "1.9.0.0", "2025-01-01", "Contoso", &hwid(1)),
        ),
    );
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(items[0].status, Status::Unknown);
    assert_eq!(items[0].new.inf.as_deref(), Some("oem61.inf"));
    assert!(journal.has(|e| matches!(
        e,
        Event::StepStart {
            step: Step::Signature,
            ..
        }
    )));
    // The restore point stays: something changed.
    assert!(
        fake.changes
            .contains(&"restore_end 431 cancel=false".to_owned())
    );
}

#[test]
fn r_src_04_unsigned_driver_never_gets_a_line_that_says_installed() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    fake.unsigned.insert("oem50.inf".to_owned());
    let (_, journal) = real(&mut fake, &plan);
    let ends: Vec<Status> = journal
        .0
        .iter()
        .filter_map(|l| match &l.event {
            Event::ItemEnd { record } => Some(record.status),
            _ => None,
        })
        .collect();
    assert_eq!(ends, [Status::RolledBackUnsigned]);
}

// ---------- Applications in the pass (ADR-0034) ----------

fn apps_of_scan() -> Vec<drivn_core::sources::winget::AppUpdate> {
    ["Git.Git", "VideoLAN.VLC", "Oracle.JavaRuntimeEnvironment"]
        .iter()
        .map(|id| drivn_core::sources::winget::AppUpdate {
            name: (*id).to_owned(),
            id: (*id).to_owned(),
            version: "1.0".to_owned(),
            available: "2.0".to_owned(),
        })
        .collect()
}

#[test]
fn r_but_01_ticked_applications_join_the_drivers_in_one_pass() {
    let fake = Fake::new(1);
    let mut plan = fake.plan(1);
    let ticked = [
        id(1),
        "app:VideoLAN.VLC".to_owned(),
        "app:Git.Git".to_owned(),
    ];
    add_apps(&mut plan, &apps_of_scan(), &ticked).unwrap();
    assert_eq!(plan.items.len(), 1);
    let ids: Vec<&str> = plan.apps.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["VideoLAN.VLC", "Git.Git"]);
    // Only what the scan listed: an ID sent by the interface alone is refused (R-SEC-05).
    let mut other = fake.plan(1);
    assert!(add_apps(&mut other, &apps_of_scan(), &["app:Evil.Thing".to_owned()]).is_err());
}

#[test]
fn r_but_02_a_failed_application_never_stops_the_others() {
    let fake = Fake::new(1);
    let mut plan = fake.plan(1);
    let ticked = ["app:Git.Git".to_owned(), "app:VideoLAN.VLC".to_owned()];
    add_apps(&mut plan, &apps_of_scan(), &ticked).unwrap();
    // The fake system cannot run winget: each one fails, each one is tried and noted.
    let mut real = Fake::new(1);
    let mut journal = Memory(Vec::new());
    run_pass(&plan, false, &mut real, &mut journal, &config()).unwrap();
    run_apps(&plan, false, &mut real, &mut journal).unwrap();
    let view = journal.view();
    assert_eq!(view.apps.len(), 2);
    assert!(view.apps.iter().all(|a| a.status
        == Some(drivn_core::install::journal::AppStatus::Failed)
        && a.detail.is_some()));
    // R-TST-01: in simulation the same path runs, winget is never called.
    let mut sim = Simulated(Fake::new(1));
    let mut journal = Memory(Vec::new());
    run_pass(&plan, true, &mut sim, &mut journal, &config()).unwrap();
    run_apps(&plan, true, &mut sim, &mut journal).unwrap();
    let view = journal.view();
    assert!(
        view.apps
            .iter()
            .all(|a| a.status == Some(drivn_core::install::journal::AppStatus::Simulated))
    );
}

// ---------- R-SEC-09 (ADR-0034) ----------

const NVIDIA_FILE: &str = "C:\\Users\\u\\AppData\\Local\\Drivn\\cache\\nvidia\\621.01-desktop-win10-win11-64bit-international-dch-whql.exe";

fn nvidia(file: &str) -> PackageItem {
    PackageItem {
        device_instance_id: "PCI\\VEN_10DE&DEV_2702\\4&1".to_owned(),
        source: "nvidia".to_owned(),
        name: "NVIDIA GeForce RTX 4080 SUPER".to_owned(),
        from: "32.0.15.8180".to_owned(),
        to: "621.01".to_owned(),
        file: file.to_owned(),
    }
}

#[test]
fn r_sec_09_official_installer_runs_silently_after_the_drivers_in_the_same_pass() {
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.packages.push(nvidia(NVIDIA_FILE));
    fake.official = Some(0);
    let (_, mut journal) = real(&mut fake, &plan);
    run_apps(&plan, false, &mut fake, &mut journal).unwrap();
    // After the drivers and their restore point, never before.
    let end = journal
        .position(|e| matches!(e, Event::PassEnd { .. }))
        .expect("end");
    let started = journal
        .position(|e| matches!(e, Event::PackageStart { .. }))
        .expect("started");
    assert!(end < started);
    // Silent switches, one run.
    let runs: Vec<&String> = fake
        .changes
        .iter()
        .filter(|c| c.starts_with("official"))
        .collect();
    assert_eq!(runs, [&format!("official 1 -s -noreboot {NVIDIA_FILE}")]);
    let view = journal.view();
    assert_eq!(view.packages.len(), 1);
    assert_eq!(view.packages[0].status, Some(AppStatus::Updated));
}

#[test]
fn r_sec_09_package_outside_the_cache_or_unknown_publisher_is_never_run() {
    let nvidia_row = official("nvidia").expect("nvidia");
    assert!(valid_file(nvidia_row, NVIDIA_FILE));
    for bad in [
        "C:\\Windows\\Temp\\setup.exe",
        "C:\\Users\\u\\AppData\\Local\\Drivn\\cache\\intel\\x.exe",
        "C:\\Users\\u\\AppData\\Local\\Drivn\\cache\\nvidia\\..\\x.exe",
        "C:\\Users\\u\\AppData\\Local\\Drivn\\cache\\nvidia\\x.msi",
        "\\\\server\\Drivn\\cache\\nvidia\\x.exe",
        "C:\\Users\\u\\AppData\\Local\\Drivn\\cache\\nvidia\\x y.exe",
    ] {
        assert!(!valid_file(nvidia_row, bad), "{bad}");
    }
    assert!(
        official("intel").is_none(),
        "Intel's silent switch is unknown"
    );
    // R-SEC-05: the installer refuses it before the run, and the pass goes on.
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.packages.push(nvidia("C:\\Windows\\Temp\\setup.exe"));
    let mut other = nvidia(NVIDIA_FILE);
    other.source = "contoso".to_owned();
    plan.packages.push(other);
    fake.official = Some(0);
    let (_, mut journal) = real(&mut fake, &plan);
    run_apps(&plan, false, &mut fake, &mut journal).unwrap();
    assert!(!fake.changes.iter().any(|c| c.starts_with("official")));
    let view = journal.view();
    assert!(
        view.packages
            .iter()
            .all(|p| p.status == Some(AppStatus::Failed))
    );
}

#[test]
fn r_but_02_a_failed_installer_never_stops_the_applications() {
    let fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.packages.push(nvidia(NVIDIA_FILE));
    add_apps(&mut plan, &apps_of_scan(), &["app:Git.Git".to_owned()]).unwrap();
    let mut real = Fake::new(1);
    real.official = Some(1);
    let mut journal = Memory(Vec::new());
    run_pass(&plan, false, &mut real, &mut journal, &config()).unwrap();
    run_apps(&plan, false, &mut real, &mut journal).unwrap();
    let view = journal.view();
    assert_eq!(view.packages[0].status, Some(AppStatus::Failed));
    assert_eq!(view.packages[0].detail.as_deref(), Some("exit code 1"));
    assert_eq!(view.apps.len(), 1, "the application was still tried");
}

#[test]
fn r_sec_01_point_is_closed_and_found_before_the_first_install() {
    let mut fake = Fake::new(1);
    let plan = fake.plan(1);
    let (_, journal) = real(&mut fake, &plan);
    assert!(journal.has(|e| matches!(
        e,
        Event::RestoreDone {
            restore: Restore::Created { seq: 431 }
        }
    )));
    let at = |what: &str| fake.changes.iter().position(|c| c == what);
    let closed = at("restore_end 431 cancel=false").expect("closed");
    let installed = at("install upd-1").expect("installed");
    assert!(closed < installed);
}

#[test]
fn r_sec_09_point_kept_when_installers_or_applications_follow() {
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.packages.push(nvidia(NVIDIA_FILE));
    // The driver changed since the plan: skipped, but NVIDIA still runs after it.
    fake.devices.get_mut(&id(1)).expect("dev").driver = Some(drv(
        "oem77.inf",
        "1.5.0.0",
        "2024-06-01",
        "Contoso",
        &hwid(1),
    ));
    let (_, journal) = real(&mut fake, &plan);
    assert!(!fake.changes.iter().any(|c| c.starts_with("restore_remove")));
    assert_eq!(journal.view().restore_seq(), Some(431));
}

#[test]
fn r_vrai_02_installer_error_but_new_driver_active_is_updated() {
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    let package = nvidia(NVIDIA_FILE);
    let mut card = fake.devices[&id(1)].clone();
    card.instance_id = package.device_instance_id.clone();
    card.driver = Some(drv(
        "oem80.inf",
        "32.0.16.2101",
        "2026-10-19",
        "NVIDIA",
        r"PCI\VEN_10DE&DEV_2702",
    ));
    fake.devices.insert(card.instance_id.clone(), card);
    plan.packages.push(package);
    // E_ACCESSDENIED from a side component, seen on 2026-11-02 with 621.01 installed.
    fake.official = Some(-2_147_024_891);
    let (_, mut journal) = real(&mut fake, &plan);
    run_apps(&plan, false, &mut fake, &mut journal).unwrap();
    let view = journal.view();
    assert_eq!(view.packages[0].status, Some(AppStatus::Updated));
    assert!(
        view.packages[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("621.01"))
    );
    // Still the old driver: a failure.
    let mut old = Fake::new(1);
    let mut plan = old.plan(1);
    plan.packages.push(nvidia(NVIDIA_FILE));
    old.official = Some(1);
    let (_, mut journal) = real(&mut old, &plan);
    run_apps(&plan, false, &mut old, &mut journal).unwrap();
    assert_eq!(journal.view().packages[0].status, Some(AppStatus::Failed));
}

#[test]
fn r_but_01_a_busy_application_is_tried_again_at_the_end() {
    let fake = Fake::new(1);
    let mut plan = fake.plan(1);
    let ticked = ["app:Git.Git".to_owned(), "app:VideoLAN.VLC".to_owned()];
    add_apps(&mut plan, &apps_of_scan(), &ticked).unwrap();
    let mut real = Fake::new(1);
    // Git: another installation running (0x8A150102), then fine; VLC: fine.
    real.winget = vec![0x8A15_0102_u32.cast_signed(), 0, 0];
    let mut journal = Memory(Vec::new());
    run_pass(&plan, false, &mut real, &mut journal, &config()).unwrap();
    run_apps(&plan, false, &mut real, &mut journal).unwrap();
    let calls: Vec<&String> = real
        .changes
        .iter()
        .filter(|c| c.starts_with("winget"))
        .collect();
    assert_eq!(
        calls,
        ["winget Git.Git", "winget VideoLAN.VLC", "winget Git.Git"]
    );
    let view = journal.view();
    assert!(
        view.apps
            .iter()
            .all(|a| a.status == Some(AppStatus::Updated))
    );
    // The retry is announced before the wait, so the screen never looks finished.
    assert!(journal.has(|e| matches!(e, Event::AppsRetry { ns, .. } if ns == &vec![1])));
    assert!(journal.view().retrying.is_empty());
    // Installs another way (0x8A15008E): tried again, the old version uninstalled first.
    let mut real = Fake::new(1);
    real.winget = vec![0x8A15_008E_u32.cast_signed(), 0, 0];
    let mut journal = Memory(Vec::new());
    run_pass(&plan, false, &mut real, &mut journal, &config()).unwrap();
    run_apps(&plan, false, &mut real, &mut journal).unwrap();
    assert!(real.changes.contains(&"winget replace Git.Git".to_owned()));
    assert_eq!(journal.view().apps[0].status, Some(AppStatus::Updated));
    // Any other failure is not tried again, and says why.
    let mut real = Fake::new(1);
    real.winget = vec![1, 0];
    let mut journal = Memory(Vec::new());
    run_pass(&plan, false, &mut real, &mut journal, &config()).unwrap();
    run_apps(&plan, false, &mut real, &mut journal).unwrap();
    assert_eq!(
        real.changes
            .iter()
            .filter(|c| c.starts_with("winget"))
            .count(),
        2
    );
    assert!(
        journal.view().apps[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("0x00000001"))
    );
}

// ---------- ADR-0036 ----------

#[test]
fn r_sec_01_no_answered_before_the_pass_makes_no_point_and_installs() {
    let mut fake = Fake::new(1);
    let mut plan = fake.plan(1);
    plan.no_restore_point = true;
    let (items, journal) = real(&mut fake, &plan);
    assert_eq!(statuses(&items), [Status::Installed]);
    assert!(!fake.changes.iter().any(|c| c.starts_with("restore_")));
    assert!(journal.has(|e| matches!(
        e,
        Event::RestoreDone {
            restore: Restore::Declined
        }
    )));
}

#[test]
fn r_sec_01_the_24_hour_limit_is_lifted_then_put_back() {
    let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\SystemRestore\r\n    SystemRestorePointCreationFrequency    REG_DWORD    0x5a0\r\n";
    assert_eq!(commands::parse_frequency(out), Some(1440));
    assert_eq!(commands::parse_frequency("ERROR: not found"), None);
    let set = commands::frequency_set(0);
    assert_eq!(set[0], "add");
    assert!(set.contains(&"REG_DWORD".to_owned()) && set.contains(&"0".to_owned()));
    assert_eq!(commands::frequency_delete()[0], "delete");
}

fn with_point(day: u32, seq: i64) -> PassView {
    let mut view = pass_view(day, true, true, true, true);
    view.restore = Some(Restore::Created { seq });
    view
}

#[test]
fn r_sec_07_keeps_the_5_newest_points_of_drivn_only() {
    let mut views: Vec<PassView> = (1..=7).map(|d| with_point(d, 100 + i64::from(d))).collect();
    // A point Windows already had: never Drivn's, never removed.
    let mut other = pass_view(8, true, false, true, true);
    other.restore = Some(Restore::Failed {
        reason: "older".to_owned(),
        existing: Some(99),
    });
    views.push(other);
    let current = views[6].pass_id.clone();
    let gone: Vec<i64> = points_to_remove(&views, &current)
        .into_iter()
        .map(|(_, seq)| seq)
        .collect();
    assert_eq!(KEEP_POINTS, 5);
    assert_eq!(gone, [102, 101]);
    // No new point of its own: nothing removed.
    let mut declined = pass_view(9, true, false, true, true);
    declined.restore = Some(Restore::Declined);
    views.push(declined);
    assert!(points_to_remove(&views, "2026-10-09T1000-aaaa").is_empty());
}
