//! Removes what identifies one machine before a snapshot is shared or kept as a test fixture
//! (docs/TESTS.md). Hardware and compatible IDs stay: they identify a model, not a PC.

use crate::model::Snapshot;

/// Keeps the bus prefix (`PCI`, `USB`…) and replaces the instance path, which can hold serial numbers.
fn anonymous_instance_id(instance_id: &str, index: usize) -> String {
    let bus = instance_id.split('\\').next().unwrap_or("DEVICE");
    format!("{bus}\\DEVICE-{index:04}")
}

pub fn anonymize(mut snapshot: Snapshot) -> Snapshot {
    snapshot.machine = None;
    snapshot.anonymized = true;
    let mut renamed = std::collections::HashMap::new();
    for (index, device) in snapshot.devices.iter_mut().enumerate() {
        let new = anonymous_instance_id(&device.instance_id, index);
        renamed.insert(std::mem::replace(&mut device.instance_id, new.clone()), new);
    }
    // The sources name devices by instance ID too: same new names, nothing left behind.
    let rename = |id: &mut String| {
        *id = renamed
            .get(id.as_str())
            .cloned()
            .unwrap_or_else(|| "DEVICE\\UNKNOWN".to_owned());
    };
    for result in &mut snapshot.vendors {
        result
            .offers
            .iter_mut()
            .for_each(|o| rename(&mut o.device_instance_id));
        result.unchecked.iter_mut().for_each(rename);
        result
            .hints
            .iter_mut()
            .for_each(|h| rename(&mut h.device_instance_id));
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SCHEMA_VERSION;
    use crate::scan::tests::device;

    #[test]
    fn machine_and_serial_numbers_are_removed_but_hardware_ids_stay() {
        let mut usb = device("USB\\VID_046D&PID_0825\\A1B2C3D4", "Camera", None);
        usb.hardware_ids = vec!["USB\\VID_046D&PID_0825&REV_0012".to_owned()];
        let snapshot = Snapshot {
            schema_version: SCHEMA_VERSION,
            tool_version: "0.1.0".to_owned(),
            created_utc: "2026-10-30T00:00:00Z".to_owned(),
            machine: Some("HOME-PC".to_owned()),
            anonymized: false,
            includes_absent_devices: false,
            devices: vec![usb],
            windows_update: None,
            vendors: Vec::new(),
        };
        let anonymous = anonymize(snapshot);
        let json = serde_json::to_string(&anonymous).unwrap();
        assert!(!json.contains("HOME-PC"));
        assert!(!json.contains("A1B2C3D4"));
        assert!(json.contains("VID_046D&PID_0825&REV_0012"));
        assert_eq!(anonymous.devices[0].instance_id, "USB\\DEVICE-0000");
        assert!(anonymous.anonymized);
    }
}
