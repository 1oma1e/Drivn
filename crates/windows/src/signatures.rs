//! Driver signature information from WMI `Win32_PnPSignedDriver` (one row per device).

use std::collections::HashMap;

use drivn_core::install::actions::SignerCheck;
use drivn_core::model::Signature;
use serde::Deserialize;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SP_INF_SIGNER_INFO_V2_W, SetupVerifyInfFileW,
};
use windows::Win32::Foundation::GetLastError;
use windows::core::HSTRING;
use wmi::WMIConnection;

#[derive(Deserialize)]
#[serde(rename = "Win32_PnPSignedDriver", rename_all = "PascalCase")]
struct SignedDriver {
    #[serde(rename = "DeviceID")]
    device_id: Option<String>,
    is_signed: Option<bool>,
    signer: Option<String>,
}

fn signature(row: &SignedDriver) -> Signature {
    match row.is_signed {
        Some(true) => Signature::Signed {
            signer: row
                .signer
                .as_ref()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
        },
        Some(false) => Signature::Unsigned,
        None => Signature::Unknown,
    }
}

/// Signature per device instance ID (uppercase). An error means "unknown for every device".
pub fn read_signatures() -> Result<HashMap<String, Signature>, String> {
    let connection = WMIConnection::new().map_err(|e| format!("WMI: {e}"))?;
    let rows: Vec<SignedDriver> = connection
        .raw_query("SELECT DeviceID, IsSigned, Signer FROM Win32_PnPSignedDriver")
        .map_err(|e| format!("Win32_PnPSignedDriver: {e}"))?;
    Ok(rows
        .iter()
        .filter_map(|row| Some((row.device_id.as_ref()?.to_uppercase(), signature(row))))
        .collect())
}

/// `SetupVerifyInfFile` checks the INF against the catalog that covers it (WinVerifyTrust) and
/// names its signer (R-SRC-04).
pub fn inf_signer(inf: &str) -> SignerCheck {
    // A third-party INF of the driver store, by its full path in %windir%\INF.
    if !drivn_core::install::plan::is_oem_inf(inf) {
        return SignerCheck::Unknown {
            reason: format!("{inf} is not a third-party INF of the driver store"),
        };
    }
    let path = match crate::security::windows_dir() {
        Ok(dir) => dir.join("INF").join(inf.trim()),
        Err(reason) => return SignerCheck::Unknown { reason },
    };
    inf_path_signer(&path)
}

/// `SetupVerifyInfFile` on an INF at a full path (an extracted package, R-SRC-04).
pub fn inf_path_signer(path: &std::path::Path) -> SignerCheck {
    let mut info = SP_INF_SIGNER_INFO_V2_W {
        cbSize: u32::try_from(size_of::<SP_INF_SIGNER_INFO_V2_W>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: `info` is a valid, sized structure; the INF name is a NUL-terminated string.
    let ok = unsafe { SetupVerifyInfFileW(&HSTRING::from(path.as_os_str()), None, &mut info) };
    if !ok.as_bool() {
        // SAFETY: no arguments.
        let error = unsafe { GetLastError() };
        return SignerCheck::Unknown {
            reason: format!("SetupVerifyInfFile: error 0x{:08X}", error.0),
        };
    }
    let signer = String::from_utf16_lossy(&info.DigitalSigner)
        .trim_end_matches('\0')
        .trim()
        .to_owned();
    if signer.is_empty() {
        SignerCheck::Unsigned
    } else {
        SignerCheck::Signed { signer }
    }
}
