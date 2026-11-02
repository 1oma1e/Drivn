//! Tests of the decision engine, one fabricated case per line of the spec
//! (docs/specs/Spec - Moteur de decision.md) plus the real anonymized snapshot of the reference PC.

use drivn_core::decide::{
    Rejection, Risk, State, UnknownReason, Unmatched, decide, parse_version, title_version,
};
use drivn_core::model::{
    Device, DriverOffer, InstalledDriver, SCHEMA_VERSION, Signature, Snapshot, WindowsUpdateResult,
};

const SUBSYS: &str = "PCI\\VEN_8086&DEV_51F0&SUBSYS_00948086";
const GENERIC: &str = "PCI\\VEN_8086&DEV_51F0";

fn driver(version: &str, date: &str, provider: &str, matching: &str) -> InstalledDriver {
    InstalledDriver {
        version: Some(version.to_owned()),
        date: Some(date.to_owned()),
        provider: Some(provider.to_owned()),
        inf_path: Some("oem1.inf".to_owned()),
        rank: None,
        matching_device_id: Some(matching.to_owned()),
        signature: Signature::Unknown,
    }
}

/// A Wi-Fi card installed on its most precise ID, driver 23.30.0.6 of 2024-01-10 by Intel.
fn wifi() -> Device {
    Device {
        instance_id: "PCI\\DEVICE-0000".to_owned(),
        description: Some("Intel Wi-Fi 6E AX211".to_owned()),
        class: Some("Net".to_owned()),
        class_guid: None,
        manufacturer: Some("Intel".to_owned()),
        present: true,
        problem_code: None,
        hardware_ids: vec![format!("{SUBSYS}&REV_01"), SUBSYS.to_owned()],
        compatible_ids: vec![GENERIC.to_owned()],
        driver: Some(driver("23.30.0.6", "2024-01-10", "Intel", SUBSYS)),
    }
}

fn offer(title: Option<&str>, id: Option<&str>, date: Option<&str>, provider: &str) -> DriverOffer {
    DriverOffer {
        title: title.map(str::to_owned),
        hardware_id: id.map(str::to_owned),
        driver_date: date.map(str::to_owned),
        provider: Some(provider.to_owned()),
        manufacturer: None,
        model: None,
        class: None,
        update_id: None,
        revision: None,
        download_url: None,
    }
}

/// The usual good offer: same ID as the installed driver, same provider, newer.
fn newer() -> DriverOffer {
    offer(
        Some("Intel net Driver Update (23.60.1.2)"),
        Some(&SUBSYS.to_lowercase()),
        Some("2024-07-03"),
        "Intel",
    )
}

fn snapshot(devices: Vec<Device>, offers: Option<Vec<DriverOffer>>) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        tool_version: "test".to_owned(),
        created_utc: "2026-10-31T00:00:00Z".to_owned(),
        machine: None,
        anonymized: true,
        includes_absent_devices: true,
        devices,
        windows_update: offers.map(|offers| WindowsUpdateResult {
            criteria: String::new(),
            online: true,
            last_search_utc: None,
            offers,
            error: None,
        }),
        vendors: Vec::new(),
    }
}

fn state_of(device: Device, offers: Vec<DriverOffer>) -> State {
    decide(&snapshot(vec![device], Some(offers)))
        .decisions
        .remove(0)
        .state
}

fn rejections(device: Device, offers: Vec<DriverOffer>) -> Vec<Rejection> {
    decide(&snapshot(vec![device], Some(offers)))
        .decisions
        .remove(0)
        .rejected
        .iter()
        .map(|r| r.reason)
        .collect()
}

// ---- Reading versions (ADR-0012) ----

#[test]
fn versions_are_one_to_four_numbers_up_to_65535() {
    assert_eq!(parse_version("24.60.0.3"), Some([24, 60, 0, 3]));
    assert_eq!(parse_version("10.1"), Some([10, 1, 0, 0]));
    assert_eq!(parse_version("1.2.x"), None);
    assert_eq!(parse_version("70000.0.0.0"), None);
    assert_eq!(parse_version("1.2.3.4.5"), None);
    assert_eq!(parse_version(""), None);
    assert_eq!(
        title_version("Intel net Driver Update (24.80.0.2)"),
        Some([24, 80, 0, 2])
    );
    assert_eq!(title_version("Realtek Storage Driver Update"), None);
}

// ---- R-MAT-01: matching by ID only ----

#[test]
fn r_mat_01_id_equal_ignoring_case_is_a_candidate() {
    assert_eq!(state_of(wifi(), vec![newer()]), State::Update);
}

#[test]
fn r_mat_01_same_name_but_other_id_is_not_a_candidate() {
    let other = offer(
        Some("Intel Wi-Fi 6E AX211 (99.0.0.0)"),
        Some("PCI\\VEN_8086&DEV_FFFF"),
        Some("2025-01-01"),
        "Intel",
    );
    let report = decide(&snapshot(vec![wifi()], Some(vec![other])));
    assert_eq!(report.decisions[0].state, State::UpToDate);
    assert!(report.decisions[0].chosen.is_none());
    assert_eq!(report.unmatched_offers[0].reason, Unmatched::NoDevice);
}

#[test]
fn r_mat_01_offer_without_hardware_id_is_unmatched() {
    let report = decide(&snapshot(
        vec![wifi()],
        Some(vec![offer(
            Some("x (1.0)"),
            None,
            Some("2025-01-01"),
            "Intel",
        )]),
    ));
    assert_eq!(report.unmatched_offers[0].reason, Unmatched::NoHardwareId);
    assert_eq!(report.decisions[0].state, State::UpToDate);
}

// ---- R-VER-01 / R-VER-02: never a downgrade, never a fake update ----

#[test]
fn r_ver_01_equal_version_is_not_newer() {
    let equal = offer(
        Some("Intel (23.30.0.6)"),
        Some(SUBSYS),
        Some("2024-01-10"),
        "Intel",
    );
    assert_eq!(
        rejections(wifi(), vec![equal.clone()]),
        vec![Rejection::NotNewer]
    );
    assert_eq!(state_of(wifi(), vec![equal]), State::UpToDate);
}

#[test]
fn r_ver_02_lower_version_is_never_an_update() {
    let lower = offer(
        Some("Intel (23.10.0.1)"),
        Some(SUBSYS),
        Some("2025-01-10"),
        "Intel",
    );
    assert_eq!(state_of(wifi(), vec![lower]), State::UpToDate);
}

#[test]
fn r_ver_01_higher_version_with_older_date_is_contradictory_and_unknown() {
    let odd = offer(
        Some("Intel (23.40.0.1)"),
        Some(SUBSYS),
        Some("2023-01-01"),
        "Intel",
    );
    assert_eq!(
        rejections(wifi(), vec![odd.clone()]),
        vec![Rejection::Contradictory]
    );
    assert_eq!(
        state_of(wifi(), vec![odd]),
        State::Unknown {
            reason: UnknownReason::DoubtfulOffer
        }
    );
}

#[test]
fn r_ver_01_missing_or_bad_title_and_missing_date_are_doubtful() {
    for title in [
        None,
        Some("Intel Driver Update"),
        Some("Intel (1.2.x)"),
        Some("Intel (70000.0.0.0)"),
    ] {
        let bad = offer(title, Some(SUBSYS), Some("2025-01-01"), "Intel");
        assert_eq!(
            rejections(wifi(), vec![bad.clone()]),
            vec![Rejection::UnknownVersion]
        );
        assert_eq!(
            state_of(wifi(), vec![bad]),
            State::Unknown {
                reason: UnknownReason::DoubtfulOffer
            }
        );
    }
    let no_date = offer(Some("Intel (24.0.0.0)"), Some(SUBSYS), None, "Intel");
    assert_eq!(
        rejections(wifi(), vec![no_date]),
        vec![Rejection::UnknownDate]
    );
}

#[test]
fn r_ver_01_unreadable_installed_driver_is_never_compared() {
    let mut device = wifi();
    if let Some(d) = device.driver.as_mut() {
        d.version = None;
    }
    assert_eq!(
        rejections(device.clone(), vec![newer()]),
        vec![Rejection::InstalledUnreadable]
    );
    assert_eq!(
        state_of(device.clone(), vec![newer()]),
        State::Unknown {
            reason: UnknownReason::DoubtfulOffer
        }
    );
    assert_eq!(
        state_of(device, vec![]),
        State::Unknown {
            reason: UnknownReason::InstalledUnreadable
        }
    );
}

#[test]
fn r_ver_01_a_retained_offer_wins_over_a_contradictory_one() {
    let odd = offer(
        Some("Intel (23.50.0.1)"),
        Some(SUBSYS),
        Some("2023-01-01"),
        "Intel",
    );
    let decision = decide(&snapshot(vec![wifi()], Some(vec![odd, newer()])))
        .decisions
        .remove(0);
    assert_eq!(decision.state, State::Update);
    assert_eq!(
        decision.chosen.map(|c| c.offer.version),
        Some(Some("23.60.1.2".to_owned()))
    );
    assert_eq!(decision.rejected[0].reason, Rejection::Contradictory);
}

#[test]
fn r_ver_01_highest_version_is_chosen_and_the_other_kept_as_evidence() {
    let higher = offer(
        Some("Intel (23.70.0.0)"),
        Some(SUBSYS),
        Some("2024-09-01"),
        "Intel",
    );
    let decision = decide(&snapshot(vec![wifi()], Some(vec![newer(), higher])))
        .decisions
        .remove(0);
    assert_eq!(decision.chosen.map(|c| c.offer.offer_index), Some(1));
    assert_eq!(decision.rejected[0].reason, Rejection::NotChosen);
}

// ---- R-MAT-02: manufacturer drivers are protected ----

#[test]
fn r_mat_02_generic_offer_over_subsys_driver_is_less_precise() {
    let generic = offer(
        Some("Intel (23.60.1.2)"),
        Some(GENERIC),
        Some("2024-07-03"),
        "Intel",
    );
    assert_eq!(
        state_of(wifi(), vec![generic]),
        State::UpdateRisky {
            risk: Risk::LessPrecise
        }
    );
}

#[test]
fn r_mat_02_other_provider_is_risky() {
    let other = offer(
        Some("Intel (23.60.1.2)"),
        Some(SUBSYS),
        Some("2024-07-03"),
        "Intel Corporation",
    );
    assert_eq!(
        state_of(wifi(), vec![other]),
        State::UpdateRisky {
            risk: Risk::OtherProvider
        }
    );
}

#[test]
fn r_mat_02_unknown_provider_or_matching_id_cannot_be_judged() {
    let mut no_match = wifi();
    if let Some(d) = no_match.driver.as_mut() {
        d.matching_device_id = None;
    }
    assert_eq!(
        state_of(no_match, vec![newer()]),
        State::UpdateRisky {
            risk: Risk::CannotJudge
        }
    );
    let mut no_provider = wifi();
    if let Some(d) = no_provider.driver.as_mut() {
        d.provider = None;
    }
    assert_eq!(
        state_of(no_provider, vec![newer()]),
        State::UpdateRisky {
            risk: Risk::CannotJudge
        }
    );
}

#[test]
fn r_mat_02_a_safe_candidate_is_chosen_before_a_risky_higher_one() {
    let risky = offer(
        Some("Intel (25.0.0.0)"),
        Some(GENERIC),
        Some("2025-01-01"),
        "Intel",
    );
    let decision = decide(&snapshot(vec![wifi()], Some(vec![risky, newer()])))
        .decisions
        .remove(0);
    assert_eq!(decision.state, State::Update);
    assert_eq!(decision.chosen.map(|c| c.offer.offer_index), Some(1));
}

// ---- R-VRAI-04: what we cannot know is said "unknown" ----

#[test]
fn r_vrai_04_no_search_or_failed_search_is_unknown_never_up_to_date() {
    let none = decide(&snapshot(vec![wifi()], None))
        .decisions
        .remove(0)
        .state;
    assert_eq!(
        none,
        State::Unknown {
            reason: UnknownReason::NoSearch
        }
    );
    let mut failed = snapshot(vec![wifi()], Some(vec![newer()]));
    if let Some(wu) = failed.windows_update.as_mut() {
        wu.error = Some("0x8024402C".to_owned());
    }
    let report = decide(&failed);
    assert_eq!(
        report.decisions[0].state,
        State::Unknown {
            reason: UnknownReason::SearchFailed
        }
    );
    assert!(report.decisions[0].chosen.is_none());
}

#[test]
fn r_vrai_04_absent_device_gets_no_candidate() {
    let mut absent = wifi();
    absent.present = false;
    let decision = decide(&snapshot(vec![absent], Some(vec![newer()])))
        .decisions
        .remove(0);
    assert_eq!(
        decision.state,
        State::Unknown {
            reason: UnknownReason::Absent
        }
    );
    assert!(decision.chosen.is_none());
}

#[test]
fn r_mat_02_device_without_driver_with_and_without_offer() {
    let mut bare = wifi();
    bare.driver = None;
    assert_eq!(
        state_of(bare.clone(), vec![newer()]),
        State::UpdateRisky {
            risk: Risk::NoInstalledDriver
        }
    );
    assert_eq!(state_of(bare, vec![]), State::NoDriver);
}

// ---- R-VRAI-01: the decision carries its evidence ----

#[test]
fn r_vrai_01_chosen_offer_keeps_id_version_date_and_provider() {
    let decision = decide(&snapshot(vec![wifi()], Some(vec![newer()])))
        .decisions
        .remove(0);
    let json = serde_json::to_value(&decision).unwrap_or_default();
    assert_eq!(json["state"], "update");
    assert_eq!(json["chosen"]["version"], "23.60.1.2");
    assert_eq!(json["chosen"]["date"], "2024-07-03");
    assert_eq!(json["chosen"]["provider"], "Intel");
    assert_eq!(json["chosen"]["hardware_id"], SUBSYS.to_lowercase());
}

// ---- Demo cases (also shown by the interface), one device per rule ----

#[test]
fn r_mat_02_demo_cases_give_one_state_per_rule() {
    let Ok(cases) = serde_json::from_str::<Snapshot>(include_str!("fixtures/cases.json")) else {
        panic!("fixture does not parse")
    };
    let report = decide(&cases);
    let states: Vec<(&str, &State)> = cases
        .devices
        .iter()
        .zip(&report.decisions)
        .map(|(d, x)| (d.description.as_deref().unwrap_or(""), &x.state))
        .collect();
    let unknown = |reason| State::Unknown { reason };
    let risky = |risk| State::UpdateRisky { risk };
    assert_eq!(
        states,
        vec![
            ("Intel Wi-Fi 6E AX211", &State::Update),
            (
                "Realtek PCIe 2.5GbE (driver Dell)",
                &risky(Risk::LessPrecise)
            ),
            ("Intel SMBus (chipset)", &risky(Risk::OtherProvider)),
            ("Intel UHD Graphics 770", &State::UpToDate),
            (
                "Pavé tactile Synaptics",
                &unknown(UnknownReason::DoubtfulOffer)
            ),
            (
                "Lecteur de cartes SD",
                &unknown(UnknownReason::DoubtfulOffer)
            ),
            (
                "Capteur de lumière ambiante",
                &risky(Risk::NoInstalledDriver)
            ),
            ("Périphérique inconnu", &State::NoDriver),
            ("Webcam USB (débranchée)", &unknown(UnknownReason::Absent)),
        ]
    );
}

// ---- Real snapshots, anonymized ----

type Update = (String, String, String);

/// (device, installed version, proposed version) of every device with a chosen offer, sorted.
fn updates_of(snapshot: &Snapshot) -> Vec<Update> {
    let report = decide(snapshot);
    let mut updates: Vec<Update> = snapshot
        .devices
        .iter()
        .zip(&report.decisions)
        .filter_map(|(device, d)| {
            Some((
                device.description.clone()?,
                device.driver.as_ref()?.version.clone()?,
                d.chosen.as_ref()?.offer.version.clone()?,
            ))
        })
        .collect();
    updates.sort();
    updates
}

fn owned(list: &[(&str, &str, &str)]) -> Vec<Update> {
    list.iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect()
}

#[test]
fn r_ver_01_test_vm_with_old_drivers_shows_exactly_the_four_newer_ones() {
    // The VM got old drivers of the reference PC on fake devices (tools/vm-guest-old-drivers.ps1).
    // Windows Update offers newer ones for 4 of them; the 4 others stay "up to date" for it.
    let text = include_str!("fixtures/vm-old-drivers-2026-10-31.json");
    let Ok(vm) = serde_json::from_str::<Snapshot>(text) else {
        panic!("fixture does not parse")
    };
    let mut updates = updates_of(&vm);
    updates.dedup(); // two Voice Clarity devices share the same ID
    let expected = owned(&[
        ("Alienware AW3225QF(DisplayPort HDR)", "1.0.0.0", "1.1.0.0"),
        ("Intel(R) Wi-Fi 6E AX211 160MHz", "24.50.0.4", "24.80.0.2"),
        ("Intel(R) Wireless Bluetooth(R)", "24.40.10.8", "24.80.0.2"),
        ("Voice Clarity", "1.0.3.56670", "1.0.4.7057"),
    ]);
    assert_eq!(updates, expected);
    let report = decide(&vm);
    assert!(
        report
            .decisions
            .iter()
            .filter(|d| d.chosen.is_some())
            .all(|d| d.state == State::Update)
    );
}

#[test]
fn r_ver_02_reference_pc_has_exactly_the_three_expected_updates() {
    let text = include_str!("fixtures/pc-a-2026-10-30.json");
    let Ok(pc) = serde_json::from_str::<Snapshot>(text) else {
        panic!("fixture does not parse")
    };
    let report = decide(&pc);
    let mut updates = updates_of(&pc);
    // Two "Alienware Command Center" software devices share the same ID: the offer counts for each.
    assert_eq!(updates.len(), 4);
    updates.dedup();
    let expected = [
        (
            "Alienware Command Center Win32 Component Device",
            "2.8.5.0",
            "2.9.2.0",
        ),
        ("Intel(R) Wi-Fi 6E AX211 160MHz", "24.60.0.3", "24.80.0.2"),
        ("Intel(R) Wireless Bluetooth(R)", "24.40.11.1", "24.80.0.2"),
    ];
    let expected: Vec<(String, String, String)> = expected
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect();
    assert_eq!(updates, expected);
    assert!(
        report
            .decisions
            .iter()
            .filter(|d| d.chosen.is_some())
            .all(|d| d.state == State::Update)
    );
    assert_eq!(report.unmatched_offers.len(), 1);
    assert_eq!(report.unmatched_offers[0].reason, Unmatched::NoDevice);
}
