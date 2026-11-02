//! Device and installed-driver reader based on CfgMgr32 (docs/research/Windows - drivers.md, §1).
//! Read-only and meant to run without administrator rights (R-DRT-01).

use drivn_core::model::{Device, InstalledDriver, Signature};
use drivn_core::scan::DeviceSource;
use drivn_core::time::date_from_filetime;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_NONE, CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW,
    CM_Get_DevNode_Status, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW,
    CM_LOCATE_DEVNODE_PHANTOM, CM_Locate_DevNodeW, CR_BUFFER_SMALL, CR_SUCCESS, DN_HAS_PROBLEM,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_Class, DEVPKEY_Device_ClassGuid, DEVPKEY_Device_CompatibleIds,
    DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_DriverDate, DEVPKEY_Device_DriverInfPath,
    DEVPKEY_Device_DriverProvider, DEVPKEY_Device_DriverRank, DEVPKEY_Device_DriverVersion,
    DEVPKEY_Device_FriendlyName, DEVPKEY_Device_HardwareIds, DEVPKEY_Device_IsPresent,
    DEVPKEY_Device_Manufacturer, DEVPKEY_Device_MatchingDeviceId, DEVPROPTYPE,
};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::core::PCWSTR;

use crate::signatures::read_signatures;

/// Reads every device through CfgMgr32, and driver signatures through WMI.
pub struct CfgMgrDevices;

impl DeviceSource for CfgMgrDevices {
    fn devices(&self, include_absent: bool) -> Result<Vec<Device>, String> {
        self.devices_with_progress(include_absent, &mut |_, _, _| {})
    }

    fn devices_with_progress(
        &self,
        include_absent: bool,
        read: &mut dyn FnMut(usize, usize, Option<&str>),
    ) -> Result<Vec<Device>, String> {
        let ids = device_ids(include_absent)?;
        let total = ids.len();
        read(0, total, None);
        // The WMI query is one slow call: it runs alongside, so every device read moves the progress.
        let (mut devices, signatures) = std::thread::scope(|scope| {
            let signatures = scope.spawn(read_signatures);
            let mut devices = Vec::new();
            for (index, instance_id) in ids.into_iter().enumerate() {
                // None: removed between the list and this call.
                let Some(devinst) = locate(&instance_id) else {
                    read(index + 1, total, None);
                    continue;
                };
                let device = read_device(instance_id, devinst);
                let name = device.description.clone();
                devices.push(device);
                read(index + 1, total, name.as_deref());
            }
            (devices, signatures.join())
        });
        // A WMI failure must not hide the devices: their signature simply stays unknown (R-VRAI-04).
        let signatures = signatures.ok().and_then(Result::ok).unwrap_or_default();
        for device in &mut devices {
            if let (Some(signature), Some(driver)) = (
                signatures.get(&device.instance_id.to_uppercase()),
                device.driver.as_mut(),
            ) {
                driver.signature = signature.clone();
            }
        }
        Ok(devices)
    }
}

/// One device as it is now (signature unknown: no WMI call). `None` when Windows does not know it.
pub fn read_one(instance_id: &str) -> Option<Device> {
    locate(instance_id).map(|devinst| read_device(instance_id.to_owned(), devinst))
}

fn device_ids(include_absent: bool) -> Result<Vec<String>, String> {
    let flags = if include_absent {
        CM_GETIDLIST_FILTER_NONE
    } else {
        CM_GETIDLIST_FILTER_PRESENT
    };
    let mut length = 0u32;
    // SAFETY: `length` is a valid out pointer; a null filter means "all enumerators".
    let result = unsafe { CM_Get_Device_ID_List_SizeW(&mut length, PCWSTR::null(), flags) };
    if result != CR_SUCCESS {
        return Err(format!(
            "CM_Get_Device_ID_List_Size failed (CONFIGRET {})",
            result.0
        ));
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: the buffer has the size CfgMgr32 just asked for.
    let result = unsafe { CM_Get_Device_ID_ListW(PCWSTR::null(), &mut buffer, flags) };
    if result != CR_SUCCESS {
        return Err(format!(
            "CM_Get_Device_ID_List failed (CONFIGRET {})",
            result.0
        ));
    }
    Ok(split_multi_sz(&buffer))
}

fn locate(instance_id: &str) -> Option<u32> {
    let wide: Vec<u16> = instance_id
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut devinst = 0u32;
    // SAFETY: `wide` is NUL-terminated and outlives the call. PHANTOM also finds absent devices.
    let result = unsafe {
        CM_Locate_DevNodeW(
            &mut devinst,
            PCWSTR(wide.as_ptr()),
            CM_LOCATE_DEVNODE_PHANTOM,
        )
    };
    (result == CR_SUCCESS).then_some(devinst)
}

/// The signature comes later, from WMI: unknown until then.
fn read_device(instance_id: String, devinst: u32) -> Device {
    let inf_path = string_property(devinst, &DEVPKEY_Device_DriverInfPath);
    let version = string_property(devinst, &DEVPKEY_Device_DriverVersion);
    let driver = (inf_path.is_some() || version.is_some()).then(|| InstalledDriver {
        version,
        date: filetime_property(devinst, &DEVPKEY_Device_DriverDate).map(date_from_filetime),
        provider: string_property(devinst, &DEVPKEY_Device_DriverProvider),
        inf_path,
        rank: u32_property(devinst, &DEVPKEY_Device_DriverRank),
        matching_device_id: string_property(devinst, &DEVPKEY_Device_MatchingDeviceId),
        signature: Signature::Unknown,
    });
    Device {
        description: string_property(devinst, &DEVPKEY_Device_FriendlyName)
            .or_else(|| string_property(devinst, &DEVPKEY_Device_DeviceDesc)),
        class: string_property(devinst, &DEVPKEY_Device_Class),
        class_guid: guid_property(devinst, &DEVPKEY_Device_ClassGuid),
        manufacturer: string_property(devinst, &DEVPKEY_Device_Manufacturer),
        present: bool_property(devinst, &DEVPKEY_Device_IsPresent).unwrap_or(true),
        problem_code: problem_code(devinst),
        hardware_ids: list_property(devinst, &DEVPKEY_Device_HardwareIds),
        compatible_ids: list_property(devinst, &DEVPKEY_Device_CompatibleIds),
        driver,
        instance_id,
    }
}

fn problem_code(devinst: u32) -> Option<u32> {
    let mut status = Default::default();
    let mut problem = Default::default();
    // SAFETY: both out pointers are valid locals.
    let result = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, devinst, 0) };
    (result == CR_SUCCESS && status & DN_HAS_PROBLEM == DN_HAS_PROBLEM).then_some(problem.0)
}

/// Raw property bytes, or `None` when the device does not have this property.
fn raw_property(devinst: u32, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    let mut kind = DEVPROPTYPE::default();
    let mut size = 0u32;
    // SAFETY: size query with no buffer; all pointers are valid locals.
    let result = unsafe { CM_Get_DevNode_PropertyW(devinst, key, &mut kind, None, &mut size, 0) };
    if result != CR_BUFFER_SMALL || size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    // SAFETY: the buffer has exactly the size CfgMgr32 reported.
    let result = unsafe {
        CM_Get_DevNode_PropertyW(
            devinst,
            key,
            &mut kind,
            Some(buffer.as_mut_ptr()),
            &mut size,
            0,
        )
    };
    (result == CR_SUCCESS).then_some(buffer)
}

fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect()
}

fn split_multi_sz(wide: &[u16]) -> Vec<String> {
    wide.split(|&c| c == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn string_property(devinst: u32, key: &DEVPROPKEY) -> Option<String> {
    let value = split_multi_sz(&utf16(&raw_property(devinst, key)?))
        .into_iter()
        .next()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn list_property(devinst: u32, key: &DEVPROPKEY) -> Vec<String> {
    raw_property(devinst, key)
        .map(|bytes| split_multi_sz(&utf16(&bytes)))
        .unwrap_or_default()
}

fn u32_property(devinst: u32, key: &DEVPROPKEY) -> Option<u32> {
    let bytes: [u8; 4] = raw_property(devinst, key)?.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn filetime_property(devinst: u32, key: &DEVPROPKEY) -> Option<u64> {
    let bytes: [u8; 8] = raw_property(devinst, key)?.get(..8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn bool_property(devinst: u32, key: &DEVPROPKEY) -> Option<bool> {
    // DEVPROP_BOOLEAN: 0x00 = false, 0xFF = true.
    raw_property(devinst, key)?.first().map(|&b| b != 0)
}

fn guid_property(devinst: u32, key: &DEVPROPKEY) -> Option<String> {
    let b = raw_property(devinst, key)?;
    if b.len() < 16 {
        return None;
    }
    let data1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let data2 = u16::from_le_bytes([b[4], b[5]]);
    let data3 = u16::from_le_bytes([b[6], b[7]]);
    Some(format!(
        "{{{data1:08x}-{data2:04x}-{data3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_sz_is_split_and_empty_parts_dropped() {
        let wide: Vec<u16> = "PCI\\A\0PCI\\B\0\0".encode_utf16().collect();
        assert_eq!(split_multi_sz(&wide), vec!["PCI\\A", "PCI\\B"]);
    }

    /// Runs the real reader on this PC. It is read-only, so it is allowed outside the VM.
    #[test]
    fn r_drt_01_device_scan_works_without_admin_rights() {
        let devices = CfgMgrDevices
            .devices(false)
            .expect("device enumeration failed");
        assert!(!devices.is_empty(), "a running Windows always has devices");
        assert!(
            devices.iter().any(|d| !d.hardware_ids.is_empty()),
            "hardware IDs must be readable"
        );
        assert!(
            devices
                .iter()
                .any(|d| d.driver.as_ref().is_some_and(|x| x.version.is_some())),
            "driver versions must be readable"
        );
    }
}
