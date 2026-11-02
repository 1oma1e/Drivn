//! Authenticode signature of a file (Spec - Sources constructeurs, « Signatures »): `WinVerifyTrust`,
//! then the signer's name read in the **validated** chain (leaf certificate), never as a text of
//! the file. Revocation: whole chain online in the installer; Windows' cache only in the app.

use std::ffi::c_void;
use std::io::Read;
use std::path::Path;

use drivn_core::sources::pe::{HEADER_BYTES, nothing_after_signature};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_CHAIN,
    WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::{GUID, HSTRING, PCWSTR};

/// `online`: revocation checked online for the whole chain (installer). Otherwise only what
/// Windows has in its cache (app, no network for revocation). A refusal says why.
pub fn file_signer(path: &Path, online: bool) -> Result<String, String> {
    let wide = HSTRING::from(path.as_os_str());
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: u32::try_from(size_of::<WINTRUST_FILE_INFO>()).unwrap_or(0),
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: u32::try_from(size_of::<WINTRUST_DATA>()).unwrap_or(0),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: if online {
            WTD_REVOCATION_CHECK_CHAIN
        } else {
            WTD_REVOCATION_CHECK_CHAIN | WTD_CACHE_ONLY_URL_RETRIEVAL
        },
        ..Default::default()
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let data_ptr: *mut c_void = std::ptr::from_mut(&mut data).cast();
    // SAFETY: `data` and `file` live until the CLOSE call below; no window is shown.
    let status = unsafe { WinVerifyTrust(HWND(std::ptr::null_mut()), &mut action, data_ptr) };
    let signer = if status == 0 {
        // SAFETY: the state data is valid between VERIFY and CLOSE.
        unsafe { leaf_name(data.hWVTStateData) }
    } else {
        Err(format!(
            "WinVerifyTrust refused the file (0x{:08X})",
            status.cast_unsigned()
        ))
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: releases the state of the VERIFY call.
    let _ = unsafe {
        WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            std::ptr::from_mut(&mut data).cast(),
        )
    };
    signer
}

/// A publisher's executable, checked before it is run: nothing after the signed zone (R-SRC-08),
/// then a valid signature of `expected` (R-SRC-03). Shared by the app and the installer.
pub fn check_signed_exe(path: &Path, expected: &str, online: bool) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let mut header = vec![0u8; HEADER_BYTES];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut header))
        .map_err(|e| e.to_string())?;
    header.truncate(read);
    nothing_after_signature(&header, len)?;
    let signer = file_signer(path, online)?;
    if signer == expected {
        Ok(())
    } else {
        Err(format!("signed by « {signer} », not by {expected}"))
    }
}

/// Simple display name of the signer's leaf certificate, from the chain WinVerifyTrust built.
unsafe fn leaf_name(state: HANDLE) -> Result<String, String> {
    // SAFETY: `state` comes from a successful VERIFY; every pointer is checked before use.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return Err("no provider data".to_owned());
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        if signer.is_null() || (*signer).pasCertChain.is_null() || (*signer).csCertChain == 0 {
            return Err("no signer chain".to_owned());
        }
        let cert = (*(*signer).pasCertChain).pCert;
        if cert.is_null() {
            return Err("no signer certificate".to_owned());
        }
        let mut buffer = [0u16; 512];
        let length = CertGetNameStringW(
            cert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut buffer),
        );
        let length = usize::try_from(length).unwrap_or(0).saturating_sub(1);
        let name = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
        if name.trim().is_empty() {
            Err("signer without a name".to_owned())
        } else {
            Ok(name.trim().to_owned())
        }
    }
}
