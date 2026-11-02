//! Install of a Microsoft Update Catalog driver (Spec - Sources constructeurs, « Installer un INF »)
//! with a fake system: nothing here touches Windows or the network.
// The fake system's helpers panic on a broken test setup, like the tests themselves.
#![allow(clippy::expect_used)]

use std::collections::HashMap;

use drivn_core::decide::decide;
use drivn_core::install::actions::{
    ChangeOutcome, InfFacts, InstallOutcome, Package, PackageError, SignerCheck, Simulated,
    SystemActions, commands,
};
use drivn_core::install::journal::{ItemRecord, Line, Skip, Status};
use drivn_core::install::plan::{
    DriverFacts, MS_CATALOG, Mode, Plan, PlanItem, build_plan, restore_point_plan, sha256_hex,
};
use drivn_core::install::run::{Config, Journal, WINDOWS_UPDATE_SIGNER, run_pass};
use drivn_core::install::setup::check_copy;
use drivn_core::model::{
    Device, DriverOffer, InstalledDriver, SCHEMA_VERSION, Signature, Snapshot, VendorOffer,
    VendorResult,
};
use drivn_core::sources::SourceId;

const PASS: &str = "2026-11-01T1810-mei1";
const GUID: &str = "ad53b108-4ed3-4bed-a2c3-15f60a6215c0";
const PRECISE: &str = "PCI\\VEN_8086&DEV_7A68&SUBSYS_88821043&REV_11";
const GENERIC: &str = "PCI\\VEN_8086&DEV_7A68";
const ID: &str = "PCI\\VEN_8086&DEV_7A68&SUBSYS_88821043&REV_11\\3&1";

fn old() -> InstalledDriver {
    InstalledDriver {
        version: Some("2603.9.4.0".to_owned()),
        date: Some("2026-01-13".to_owned()),
        provider: Some("Intel".to_owned()),
        inf_path: Some("oem13.inf".to_owned()),
        rank: None,
        matching_device_id: Some(GENERIC.to_owned()),
        signature: Signature::Unknown,
    }
}

fn mei() -> Device {
    Device {
        instance_id: ID.to_owned(),
        description: Some("Intel(R) Management Engine Interface #1".to_owned()),
        class: Some("System".to_owned()),
        class_guid: None,
        manufacturer: Some("Intel".to_owned()),
        present: true,
        problem_code: None,
        hardware_ids: vec![PRECISE.to_owned(), GENERIC.to_owned()],
        compatible_ids: vec!["PCI\\VEN_8086&CC_0780".to_owned()],
        driver: Some(old()),
    }
}

fn inf(ids: &[&str], version: &str, provider: &str, signer: SignerCheck) -> InfFacts {
    InfFacts {
        path: "C:\\ProgramData\\Drivn\\passes\\p\\extract\\1\\heci.inf".to_owned(),
        hardware_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        version: Some(version.to_owned()),
        date: Some("2026-06-30".to_owned()),
        provider: Some(provider.to_owned()),
        class: Some("System".to_owned()),
        signer,
    }
}

fn whcp() -> SignerCheck {
    SignerCheck::Signed {
        signer: WINDOWS_UPDATE_SIGNER.to_owned(),
    }
}

struct Fake {
    devices: HashMap<String, Device>,
    package: Result<Package, PackageError>,
    fetched: usize,
    installs: Vec<(String, String)>,
    cleaned: bool,
    /// `Some(cancel)` once the restore point was closed.
    ended: Option<bool>,
    /// Windows keeps its own driver unless the install is forced (R-SEC-04).
    keeps_own: bool,
    forced: usize,
}

impl Fake {
    fn new(package: Result<Package, PackageError>) -> Self {
        Self {
            devices: HashMap::from([(ID.to_owned(), mei())]),
            package,
            fetched: 0,
            installs: Vec::new(),
            cleaned: false,
            ended: None,
            keeps_own: false,
            forced: 0,
        }
    }
}

fn package(infs: Vec<InfFacts>) -> Result<Package, PackageError> {
    Ok(Package {
        infs,
        signer: "Microsoft Corporation".to_owned(),
    })
}

impl SystemActions for Fake {
    fn now(&mut self) -> i64 {
        1_800_000_000
    }
    fn wait(&mut self, _: u64) {}
    fn search_offers(&mut self) -> Result<Vec<DriverOffer>, String> {
        panic!("a Catalog-only pass must not search Windows Update");
    }
    fn read_device(&mut self, id: &str) -> Result<Option<Device>, String> {
        Ok(self.devices.get(id).cloned())
    }
    fn read_control(&mut self) -> Option<String> {
        None
    }
    fn restore_points(&mut self) -> Result<Vec<i64>, String> {
        Ok(if self.installs.is_empty() {
            vec![]
        } else {
            vec![7]
        })
    }
    fn driver_signer(&mut self, _: &str) -> SignerCheck {
        whcp()
    }
    fn inf_used_elsewhere(&mut self, _: &str, _: &[String]) -> Result<bool, String> {
        Ok(false)
    }
    fn others_with_id(&mut self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn begin_restore_point(&mut self, _: &str) -> Result<i64, String> {
        self.installs.push(("restore".to_owned(), String::new()));
        Ok(7)
    }
    fn end_restore_point(&mut self, _: i64, cancel: bool) -> Result<(), String> {
        self.ended = Some(cancel);
        Ok(())
    }
    fn remove_restore_point(&mut self, _: i64) -> Result<(), String> {
        Ok(())
    }
    fn export_driver(&mut self, _: &str, _: &str, n: usize) -> Result<String, String> {
        Ok(format!("backup\\{n}"))
    }
    fn install_update(&mut self, _: &str, _: i32) -> InstallOutcome {
        panic!("a Catalog driver is never installed through Windows Update");
    }
    fn rollback_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn add_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn delete_driver(&mut self, _: &str) -> Result<ChangeOutcome, String> {
        Ok(ChangeOutcome {
            reboot_required: false,
        })
    }
    fn remove_backup(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn fetch_package(
        &mut self,
        pass_id: &str,
        _: usize,
        source: &str,
        update_id: &str,
    ) -> Result<Package, PackageError> {
        assert_eq!((pass_id, source, update_id), (PASS, MS_CATALOG, GUID));
        self.fetched += 1;
        self.package.clone()
    }
    fn install_inf(&mut self, id: &str, inf: &str) -> InstallOutcome {
        self.installs.push((id.to_owned(), inf.to_owned()));
        if self.keeps_own {
            return InstallOutcome {
                succeeded: true,
                hresult: 0,
                reboot_required: false,
                detail: Some("Windows kept its driver".to_owned()),
            };
        }
        // Windows takes the new driver.
        if let Some(d) = self.devices.get_mut(ID) {
            d.driver = Some(InstalledDriver {
                version: Some("2627.10.5.0".to_owned()),
                date: Some("2026-06-30".to_owned()),
                provider: Some("Intel".to_owned()),
                inf_path: Some("oem70.inf".to_owned()),
                ..old()
            });
        }
        InstallOutcome {
            succeeded: true,
            hresult: 0,
            reboot_required: false,
            detail: None,
        }
    }
    fn install_inf_forced(&mut self, id: &str, inf: &str) -> InstallOutcome {
        self.forced += 1;
        self.keeps_own = false;
        self.install_inf(id, inf)
    }
    fn clean_work(&mut self, _: &str) -> Result<(), String> {
        self.cleaned = true;
        Ok(())
    }
}

#[derive(Default)]
struct Lines(Vec<Line>);
impl Journal for Lines {
    fn append(&mut self, line: &Line) -> Result<(), String> {
        self.0.push(line.clone());
        Ok(())
    }
}

fn item() -> PlanItem {
    PlanItem {
        device_instance_id: ID.to_owned(),
        name: Some("Intel MEI".to_owned()),
        hardware_id: GENERIC.to_owned(),
        source: MS_CATALOG.to_owned(),
        update_id: GUID.to_owned(),
        revision: 0,
        checked_by_user: true,
        installed: Some(DriverFacts::of(&old())),
        offer: DriverFacts {
            inf: None,
            title: Some("Intel System Driver Update (2627.10.5.0)".to_owned()),
            version: Some("2627.10.5.0".to_owned()),
            date: Some("2026-06-29".to_owned()),
            provider: Some("Intel".to_owned()),
        },
    }
}

fn plan(mode: Mode) -> Plan {
    Plan {
        schema: 2,
        pass_id: PASS.to_owned(),
        mode,
        items: vec![item()],
        restore_point_only: false,
        no_restore_point: false,
        apps: Vec::new(),
        packages: Vec::new(),
    }
}

fn config() -> Config {
    Config {
        enforce_signer: true,
        ..Config::default()
    }
}

fn run(fake: &mut Fake) -> ItemRecord {
    let mut lines = Lines::default();
    let items = run_pass(&plan(Mode::Real), false, fake, &mut lines, &config()).expect("pass");
    items.into_iter().next().expect("one item")
}

fn skipped(package: Result<Package, PackageError>) -> (Skip, Fake) {
    let mut fake = Fake::new(package);
    let record = run(&mut fake);
    assert_eq!(record.status, Status::Skipped);
    assert!(
        fake.installs.iter().all(|(id, _)| id == "restore"),
        "nothing installed"
    );
    (record.skip.expect("a reason"), fake)
}

#[test]
fn catalog_driver_is_installed_on_the_most_precise_id_and_the_work_is_cleaned() {
    let mut fake = Fake::new(package(vec![inf(
        &[GENERIC, PRECISE],
        "2627.10.5.0",
        "Intel",
        whcp(),
    )]));
    let record = run(&mut fake);
    assert_eq!(record.status, Status::Installed, "{record:?}");
    assert_eq!(record.new.version.as_deref(), Some("2627.10.5.0"));
    let installed: Vec<&(String, String)> = fake
        .installs
        .iter()
        .filter(|(id, _)| id != "restore")
        .collect();
    // R-SEC-02: the device's most precise ID that the INF lists.
    assert_eq!(installed[0].0, PRECISE);
    assert!(fake.cleaned);
}

#[test]
fn r_src_02_hash_differs_nothing_opened_nothing_installed() {
    assert_eq!(skipped(Err(PackageError::Hash)).0, Skip::RefusedHash);
}

#[test]
fn r_src_03_bad_signature_is_refused_without_option() {
    let (skip, _) = skipped(Err(PackageError::Signature("unexpected signer".to_owned())));
    assert_eq!(skip, Skip::RefusedSignature);
}

#[test]
fn r_src_08_archive_refused_or_no_inf_for_the_device() {
    assert_eq!(
        skipped(Err(PackageError::Archive("..\\x".to_owned()))).0,
        Skip::ExtractFailed
    );
    let other_device = inf(&["PCI\\VEN_8086&DEV_9999"], "2627.10.5.0", "Intel", whcp());
    assert_eq!(skipped(package(vec![other_device])).0, Skip::NoMatchingInf);
    let other_provider = inf(&[GENERIC], "2627.10.5.0", "Contoso", whcp());
    assert_eq!(
        skipped(package(vec![other_provider])).0,
        Skip::NoMatchingInf
    );
}

#[test]
fn r_sec_02_compatible_id_only_is_skipped() {
    let compatible = inf(&["PCI\\VEN_8086&CC_0780"], "2627.10.5.0", "Intel", whcp());
    assert_eq!(skipped(package(vec![compatible])).0, Skip::CompatibleIdOnly);
}

#[test]
fn r_ver_03_announced_newer_but_inf_not_newer_is_skipped() {
    let same = inf(&[GENERIC], "2603.9.4.0", "Intel", whcp());
    assert_eq!(skipped(package(vec![same])).0, Skip::NoLongerNewer);
}

#[test]
fn r_src_04_catalog_unsigned_or_other_signer_is_skipped() {
    let unsigned = inf(&[GENERIC], "2627.10.5.0", "Intel", SignerCheck::Unsigned);
    assert_eq!(skipped(package(vec![unsigned])).0, Skip::RefusedCatalog);
    let other = inf(
        &[GENERIC],
        "2627.10.5.0",
        "Intel",
        SignerCheck::Signed {
            signer: "Contoso".to_owned(),
        },
    );
    assert_eq!(skipped(package(vec![other])).0, Skip::RefusedCatalog);
}

#[test]
fn r_sec_04_first_try_without_force_then_forced_when_windows_keeps_its_own() {
    assert_eq!(
        commands::INSTALL_INF_FLAGS,
        0x4,
        "first try: INSTALLFLAG_NONINTERACTIVE only"
    );
    assert_eq!(commands::INSTALL_INF_FORCED_FLAGS, 0x5);
    let mut fake = Fake::new(package(vec![inf(
        &[GENERIC],
        "2627.10.5.0",
        "Intel",
        whcp(),
    )]));
    fake.keeps_own = true;
    let record = run(&mut fake);
    assert_eq!(record.status, Status::Installed, "{record:?}");
    assert!(record.forced);
    let order: Vec<&str> = fake
        .installs
        .iter()
        .map(|(id, _)| id.as_str())
        .filter(|id| *id != "restore")
        .collect();
    assert_eq!(order, [GENERIC, GENERIC]);
    assert_eq!(fake.forced, 1);
}

#[test]
fn r_sec_04_a_driver_taken_at_once_is_never_forced() {
    let mut fake = Fake::new(package(vec![inf(
        &[GENERIC],
        "2627.10.5.0",
        "Intel",
        whcp(),
    )]));
    let record = run(&mut fake);
    assert_eq!(record.status, Status::Installed);
    assert!(!record.forced);
    assert_eq!(fake.forced, 0);
}

#[test]
fn r_tst_01_simulation_checks_the_package_but_installs_nothing() {
    let fake = Fake::new(package(vec![inf(
        &[GENERIC],
        "2627.10.5.0",
        "Intel",
        whcp(),
    )]));
    let mut lines = Lines::default();
    let mut sim = Simulated(fake);
    let items = run_pass(
        &plan(Mode::Simulation),
        true,
        &mut sim,
        &mut lines,
        &config(),
    )
    .expect("pass");
    let fake = sim.0;
    assert_eq!(items[0].status, Status::Simulated);
    assert_eq!(
        fake.fetched, 1,
        "download, hash, signature and INF read in simulation too"
    );
    assert!(fake.installs.is_empty());
}

#[test]
fn r_sec_05_unknown_source_or_catalog_in_a_schema_1_plan_is_refused() {
    let mut bad = plan(Mode::Real);
    bad.items[0].source = "nvidia".to_owned();
    let text = serde_json::to_vec(&bad).expect("json");
    assert!(check_copy(PASS, &sha256_hex(&text), &text).is_err());
    let mut old_schema = plan(Mode::Real);
    old_schema.schema = 1;
    let text = serde_json::to_vec(&old_schema).expect("json");
    assert!(check_copy(PASS, &sha256_hex(&text), &text).is_err());
    let text = serde_json::to_vec(&plan(Mode::Real)).expect("json");
    assert!(check_copy(PASS, &sha256_hex(&text), &text).is_ok());
}

fn snapshot(installable: bool, source: SourceId) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        tool_version: "test".to_owned(),
        created_utc: String::new(),
        machine: None,
        anonymized: true,
        includes_absent_devices: false,
        devices: vec![mei()],
        windows_update: None,
        vendors: vec![VendorResult {
            source,
            error: None,
            requests: 1,
            offers: vec![VendorOffer {
                device_instance_id: ID.to_owned(),
                hardware_id: GENERIC.to_owned(),
                title: Some("Intel System Driver Update (2627.10.5.0)".to_owned()),
                version: Some("2627.10.5.0".to_owned()),
                vendor_version: None,
                announced_newer: false,
                date: Some("2026-06-29".to_owned()),
                provider: Some("Intel".to_owned()),
                publisher: None,
                size_bytes: None,
                size_text: None,
                update_id: Some(GUID.to_owned()),
                installable,
                download_url: None,
                sha512: None,
            }],
            unchecked: Vec::new(),
            hints: Vec::new(),
            java8_latest_update: None,
            bios: None,
        }],
    }
}

#[test]
fn plan_takes_a_ticked_catalog_offer_never_a_shown_only_one() {
    let snap = snapshot(true, SourceId::MsCatalog);
    let built =
        build_plan(&snap, &decide(&snap), &[ID.to_owned()], PASS, Mode::Real).expect("plan");
    assert_eq!(built.items[0].source, MS_CATALOG);
    assert_eq!(built.items[0].update_id, GUID);
    let shown = snapshot(false, SourceId::Nvidia);
    assert!(build_plan(&shown, &decide(&shown), &[ID.to_owned()], PASS, Mode::Real).is_err());
}

// ---------- R-SEC-09: restore point alone, before an official tool ----------

#[test]
fn r_sec_09_restore_point_pass_makes_and_keeps_a_point_and_installs_nothing() {
    let mut fake = Fake::new(Err(PackageError::Hash));
    let mut lines = Lines::default();
    let plan = restore_point_plan(PASS, Mode::Real).expect("plan");
    let items = run_pass(&plan, false, &mut fake, &mut lines, &config()).expect("pass");
    assert!(items.is_empty());
    assert_eq!(fake.installs, [("restore".to_owned(), String::new())]);
    assert_eq!(
        fake.ended,
        Some(false),
        "END_SYSTEM_CHANGE: the point is kept"
    );
    assert_eq!(fake.fetched, 0);
}

#[test]
fn r_sec_09_simulated_restore_point_pass_makes_no_point() {
    let mut lines = Lines::default();
    let plan = restore_point_plan(PASS, Mode::Simulation).expect("plan");
    let mut sim = Simulated(Fake::new(Err(PackageError::Hash)));
    run_pass(&plan, true, &mut sim, &mut lines, &config()).expect("pass");
    assert!(sim.0.installs.is_empty());
}

#[test]
fn r_sec_09_restore_point_plan_holds_no_driver() {
    let mut plan = restore_point_plan(PASS, Mode::Real).expect("plan");
    let text = serde_json::to_vec(&plan).expect("json");
    assert!(check_copy(PASS, &sha256_hex(&text), &text).is_ok());
    plan.items.push(item());
    let text = serde_json::to_vec(&plan).expect("json");
    assert!(check_copy(PASS, &sha256_hex(&text), &text).is_err());
}

#[test]
fn r_mat_01_compatible_id_naming_the_chip_is_used_as_on_a_real_pc() {
    // The real IDs of the reference PC's Intel ME (2026-11-02): `VEN&DEV` is a Compatible ID for Windows.
    let mut fake = Fake::new(package(vec![inf(
        &[GENERIC],
        "2627.10.5.0",
        "Intel",
        whcp(),
    )]));
    if let Some(d) = fake.devices.get_mut(ID) {
        d.hardware_ids = vec![
            PRECISE.to_owned(),
            r"PCI\VEN_8086&DEV_7A68&SUBSYS_88821043".to_owned(),
            r"PCI\VEN_8086&DEV_7A68&CC_078000".to_owned(),
            r"PCI\VEN_8086&DEV_7A68&CC_0780".to_owned(),
        ];
        d.compatible_ids = vec![
            r"PCI\VEN_8086&DEV_7A68&REV_11".to_owned(),
            GENERIC.to_owned(),
            r"PCI\VEN_8086&CC_078000".to_owned(),
            r"PCI\VEN_8086".to_owned(),
            r"PCI\CC_0780".to_owned(),
        ];
    }
    let record = run(&mut fake);
    assert_eq!(record.status, Status::Installed, "{record:?}");
    let used: Vec<&str> = fake
        .installs
        .iter()
        .map(|(id, _)| id.as_str())
        .filter(|id| *id != "restore")
        .collect();
    assert_eq!(used, [GENERIC]);
    // A class or a vendor alone still never is.
    use drivn_core::install::run::names_the_chip;
    assert!(names_the_chip(GENERIC) && names_the_chip(r"USB\VID_046D&PID_C52B"));
    assert!(!names_the_chip(r"PCI\VEN_8086&CC_0780") && !names_the_chip(r"PCI\VEN_8086"));
    assert!(!names_the_chip(r"USB\Class_03&SubClass_01"));
}
