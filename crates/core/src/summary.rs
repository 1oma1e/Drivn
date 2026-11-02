//! Short French text summary printed by `drivn-scan`. Only counts what was measured (R-VRAI-01):
//! no score, no "problems found" that Windows did not report.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::model::{Signature, Snapshot};

const UNKNOWN_CLASS: &str = "(classe inconnue)";

pub fn summary(snapshot: &Snapshot) -> String {
    let devices = &snapshot.devices;
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    for device in devices {
        *by_class
            .entry(device.class.as_deref().unwrap_or(UNKNOWN_CLASS))
            .or_default() += 1;
    }
    let without_driver = devices.iter().filter(|d| d.driver.is_none()).count();
    let with_problem = devices.iter().filter(|d| d.problem_code.is_some()).count();
    let drivers: Vec<_> = devices.iter().filter_map(|d| d.driver.as_ref()).collect();
    let unknown_version = drivers.iter().filter(|d| d.version.is_none()).count();
    let unsigned = drivers
        .iter()
        .filter(|d| d.signature == Signature::Unsigned)
        .count();
    let unknown_signature = drivers
        .iter()
        .filter(|d| d.signature == Signature::Unknown)
        .count();

    let mut text = String::new();
    // Writing to a String cannot fail.
    let _ = writeln!(text, "Appareils : {}", devices.len());
    for (class, count) in &by_class {
        let _ = writeln!(text, "  {class} : {count}");
    }
    let _ = writeln!(text, "Sans driver : {without_driver}");
    let _ = writeln!(text, "Signalés en problème par Windows : {with_problem}");
    let _ = writeln!(text, "Version de driver inconnue : {unknown_version}");
    let _ = writeln!(text, "Drivers non signés : {unsigned}");
    let _ = writeln!(text, "Signature inconnue : {unknown_signature}");
    let _ = match &snapshot.windows_update {
        None => writeln!(text, "Windows Update : non interrogé (option --wu)"),
        Some(result) => match &result.error {
            Some(error) => writeln!(
                text,
                "Windows Update : échec de la recherche ({error}) — rien n'est connu"
            ),
            None => writeln!(
                text,
                "Windows Update : {} driver(s) proposé(s)",
                result.offers.len()
            ),
        },
    };
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SCHEMA_VERSION, WindowsUpdateResult};
    use crate::scan::tests::{device, driver};

    fn snapshot(windows_update: Option<WindowsUpdateResult>) -> Snapshot {
        let mut broken = device("PCI\\B", "Net", None);
        broken.problem_code = Some(28);
        Snapshot {
            schema_version: SCHEMA_VERSION,
            tool_version: "0.1.0".to_owned(),
            created_utc: "2026-10-30T00:00:00Z".to_owned(),
            machine: None,
            anonymized: true,
            includes_absent_devices: false,
            devices: vec![
                device(
                    "PCI\\A",
                    "Net",
                    Some(driver(Some("1.0"), Signature::Signed { signer: None })),
                ),
                broken,
                device("USB\\C", "USB", Some(driver(None, Signature::Unsigned))),
            ],
            windows_update,
            vendors: Vec::new(),
        }
    }

    #[test]
    fn r_vrai_01_counts_only_what_was_measured() {
        let text = summary(&snapshot(None));
        assert!(text.contains("Appareils : 3"));
        assert!(text.contains("  Net : 2"));
        assert!(text.contains("Sans driver : 1"));
        assert!(text.contains("Signalés en problème par Windows : 1"));
        assert!(text.contains("Version de driver inconnue : 1"));
        assert!(text.contains("Drivers non signés : 1"));
        assert!(text.contains("non interrogé"));
    }

    #[test]
    fn r_vrai_04_failed_windows_update_search_is_not_shown_as_up_to_date() {
        let failed = WindowsUpdateResult {
            criteria: String::new(),
            online: true,
            last_search_utc: None,
            offers: vec![],
            error: Some("0x8024402C".to_owned()),
        };
        let text = summary(&snapshot(Some(failed)));
        assert!(text.contains("échec de la recherche (0x8024402C)"));
        assert!(!text.contains("0 driver(s) proposé(s)"));
    }
}
