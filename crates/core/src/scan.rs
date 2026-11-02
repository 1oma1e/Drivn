//! Scan orchestration. The Windows readers are injected, so the rules about what the scan may do
//! (no network without `--wu`, nothing guessed) are tested here with fakes.

use crate::anonymize::anonymize;
use crate::model::{Device, SCHEMA_VERSION, Snapshot, WindowsUpdateResult};

/// Reads the devices and their installed drivers. Must work without administrator rights (R-DRT-01).
pub trait DeviceSource {
    fn devices(&self, include_absent: bool) -> Result<Vec<Device>, String>;

    /// Same as [`DeviceSource::devices`], and calls `read(done, total, name)` as devices are read,
    /// `name` being the device just read. The default reports once at the end; a reader that knows
    /// its count overrides it.
    fn devices_with_progress(
        &self,
        include_absent: bool,
        read: &mut dyn FnMut(usize, usize, Option<&str>),
    ) -> Result<Vec<Device>, String> {
        let found = self.devices(include_absent)?;
        read(found.len(), found.len(), None);
        Ok(found)
    }
}

/// What the scan reports while it runs, so the interface shows only measured progress (R-VRAI-01).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScanEvent<'a> {
    /// Step 1: `done` devices out of `total` are read; `current` is the one just read (Spec - Scan
    /// complet: the screen names the device, as Driver Booster does).
    Reading {
        done: usize,
        total: usize,
        current: Option<&'a str>,
    },
    /// Step 1 done: the devices are read; step 2 (Windows Update, which gives no progress) starts.
    HardwareRead(&'a [Device]),
}

/// Asks Windows Update which drivers it offers. This is the only network access of a scan.
/// `Sync` because the search runs on its own thread, alongside the hardware read.
pub trait UpdateSource: Sync {
    fn driver_offers(&self) -> WindowsUpdateResult;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOptions {
    pub include_absent: bool,
    pub windows_update: bool,
    pub anonymize: bool,
}

/// Values that come from the running process, passed in so the scan stays deterministic in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanContext {
    pub tool_version: String,
    pub created_utc: String,
    pub machine: Option<String>,
}

pub fn scan(
    devices: &dyn DeviceSource,
    updates: &dyn UpdateSource,
    options: &ScanOptions,
    context: ScanContext,
) -> Result<Snapshot, String> {
    scan_with_progress(devices, updates, options, context, &mut |_| {})
}

/// Same as [`scan`], and reports [`ScanEvent`]s: devices read so far, then the full list.
/// The (slow) Windows Update search does not need the devices: it runs at the same time, on its own
/// thread, so the scan lasts about as long as the search alone.
pub fn scan_with_progress(
    devices: &dyn DeviceSource,
    updates: &dyn UpdateSource,
    options: &ScanOptions,
    context: ScanContext,
    progress: &mut dyn FnMut(ScanEvent<'_>),
) -> Result<Snapshot, String> {
    let (found, windows_update) = std::thread::scope(|scope| {
        let search = options
            .windows_update
            .then(|| scope.spawn(|| updates.driver_offers()));
        let found =
            devices.devices_with_progress(options.include_absent, &mut |done, total, current| {
                progress(ScanEvent::Reading {
                    done,
                    total,
                    current,
                });
            });
        let found = found.map(|mut found| {
            found.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
            progress(ScanEvent::HardwareRead(&found));
            found
        });
        let windows_update = search.map(|search| {
            search
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
        (found, windows_update)
    });
    let found = found?;
    let snapshot = Snapshot {
        schema_version: SCHEMA_VERSION,
        tool_version: context.tool_version,
        created_utc: context.created_utc,
        machine: context.machine,
        anonymized: false,
        includes_absent_devices: options.include_absent,
        devices: found,
        windows_update,
        vendors: Vec::new(),
    };
    Ok(if options.anonymize {
        anonymize(snapshot)
    } else {
        snapshot
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{DriverOffer, InstalledDriver, Signature};

    pub(crate) fn device(
        instance_id: &str,
        class: &str,
        driver: Option<InstalledDriver>,
    ) -> Device {
        Device {
            instance_id: instance_id.to_owned(),
            description: Some(format!("{class} device")),
            class: Some(class.to_owned()),
            class_guid: None,
            manufacturer: None,
            present: true,
            problem_code: None,
            hardware_ids: vec![format!("{instance_id}-HWID")],
            compatible_ids: vec![],
            driver,
        }
    }

    pub(crate) fn driver(version: Option<&str>, signature: Signature) -> InstalledDriver {
        InstalledDriver {
            version: version.map(str::to_owned),
            date: None,
            provider: Some("Contoso".to_owned()),
            inf_path: Some("oem1.inf".to_owned()),
            rank: None,
            matching_device_id: None,
            signature,
        }
    }

    struct FakeDevices(Vec<Device>);
    impl DeviceSource for FakeDevices {
        fn devices(&self, _include_absent: bool) -> Result<Vec<Device>, String> {
            Ok(self.0.clone())
        }
    }

    /// Stands for the network: any call is a test failure.
    struct NetworkForbidden;
    impl UpdateSource for NetworkForbidden {
        fn driver_offers(&self) -> WindowsUpdateResult {
            panic!("the scan reached Windows Update although it was not asked to");
        }
    }

    struct OneOffer;
    impl UpdateSource for OneOffer {
        fn driver_offers(&self) -> WindowsUpdateResult {
            WindowsUpdateResult {
                criteria: "IsInstalled=0 and Type='Driver'".to_owned(),
                online: true,
                last_search_utc: None,
                offers: vec![DriverOffer {
                    title: Some("Contoso - Net - 2.0".to_owned()),
                    hardware_id: Some("PCI\\VEN_1234&DEV_5678".to_owned()),
                    driver_date: Some("2026-01-02".to_owned()),
                    provider: Some("Contoso".to_owned()),
                    manufacturer: None,
                    model: None,
                    class: Some("Net".to_owned()),
                    update_id: None,
                    revision: None,
                    download_url: None,
                }],
                error: None,
            }
        }
    }

    fn context() -> ScanContext {
        ScanContext {
            tool_version: "0.1.0".to_owned(),
            created_utc: "2026-10-30T00:00:00Z".to_owned(),
            machine: Some("HOME-PC".to_owned()),
        }
    }

    #[test]
    fn r_vrai_02_scan_without_wu_never_touches_windows_update() {
        let devices = FakeDevices(vec![device("PCI\\A", "Net", None)]);
        let snapshot = scan(
            &devices,
            &NetworkForbidden,
            &ScanOptions::default(),
            context(),
        )
        .unwrap();
        assert_eq!(snapshot.windows_update, None);
        assert_eq!(snapshot.devices.len(), 1);
    }

    #[test]
    fn windows_update_offers_are_kept_when_asked() {
        let options = ScanOptions {
            windows_update: true,
            ..ScanOptions::default()
        };
        let snapshot = scan(&FakeDevices(vec![]), &OneOffer, &options, context()).unwrap();
        assert_eq!(snapshot.windows_update.unwrap().offers.len(), 1);
    }

    #[test]
    fn r_vrai_01_progress_reports_devices_read_then_the_full_list() {
        let devices = FakeDevices(vec![
            device("PCI\\A", "Net", None),
            device("USB\\B", "USB", None),
        ]);
        let mut events = Vec::new();
        scan_with_progress(
            &devices,
            &NetworkForbidden,
            &ScanOptions::default(),
            context(),
            &mut |event| {
                events.push(match event {
                    ScanEvent::Reading { done, total, .. } => format!("read {done}/{total}"),
                    ScanEvent::HardwareRead(found) => format!("hardware {}", found.len()),
                });
            },
        )
        .unwrap();
        assert_eq!(events, ["read 2/2", "hardware 2"]);
    }

    #[test]
    fn devices_are_sorted_and_snapshot_round_trips_through_json() {
        let devices = FakeDevices(vec![
            device(
                "USB\\B",
                "USB",
                Some(driver(Some("1.0"), Signature::Unknown)),
            ),
            device("PCI\\A", "Net", None),
        ]);
        let snapshot = scan(
            &devices,
            &NetworkForbidden,
            &ScanOptions::default(),
            context(),
        )
        .unwrap();
        assert_eq!(snapshot.devices[0].instance_id, "PCI\\A");
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn r_vrai_04_missing_values_stay_unknown_in_json() {
        let devices = FakeDevices(vec![device(
            "PCI\\A",
            "Net",
            Some(driver(None, Signature::Unknown)),
        )]);
        let snapshot = scan(
            &devices,
            &NetworkForbidden,
            &ScanOptions::default(),
            context(),
        )
        .unwrap();
        let json: serde_json::Value = serde_json::to_value(&snapshot).unwrap();
        let driver = &json["devices"][0]["driver"];
        assert!(
            driver["version"].is_null(),
            "a missing version must stay null, not be invented"
        );
        assert_eq!(driver["signature"]["state"], "unknown");
    }

    #[test]
    fn device_source_errors_are_reported() {
        struct Broken;
        impl DeviceSource for Broken {
            fn devices(&self, _: bool) -> Result<Vec<Device>, String> {
                Err("CM_Get_Device_ID_List failed".to_owned())
            }
        }
        let error = scan(
            &Broken,
            &NetworkForbidden,
            &ScanOptions::default(),
            context(),
        )
        .unwrap_err();
        assert!(error.contains("CM_Get_Device_ID_List"));
    }
}
