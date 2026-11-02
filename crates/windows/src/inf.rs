//! Reads an INF of an extracted package with Windows' own INF API (`SetupOpenInfFile`), never with a
//! parser written for Drivn (Spec - Sources constructeurs, « Surface d'attaque »). Read-only.

use std::path::Path;

use drivn_core::install::actions::InfFacts;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    INF_STYLE_WIN4, INFCONTEXT, SetupCloseInfFile, SetupDiGetActualModelsSectionW,
    SetupFindFirstLineW, SetupFindNextLine, SetupGetFieldCount, SetupGetStringFieldW,
    SetupOpenInfFileW,
};
use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::core::{HSTRING, PCWSTR};

use crate::signatures::inf_path_signer;

fn field(context: &INFCONTEXT, index: u32) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    let mut needed = 0u32;
    // SAFETY: `buffer` holds 1024 characters; the call writes a NUL-terminated string.
    unsafe { SetupGetStringFieldW(context, index, Some(&mut buffer), Some(&mut needed)) }.ok()?;
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Every line of `section`, as its fields 1..=n.
fn lines(inf: *mut std::ffi::c_void, section: &str) -> Vec<(INFCONTEXT, u32)> {
    let mut out = Vec::new();
    let mut context = INFCONTEXT::default();
    let name = HSTRING::from(section);
    // SAFETY: `inf` is an open INF handle; `context` is a valid out structure.
    if unsafe { SetupFindFirstLineW(inf, &name, PCWSTR::null(), &mut context) }.is_err() {
        return out;
    }
    loop {
        // SAFETY: `context` points at a line of the open INF.
        let count = unsafe { SetupGetFieldCount(&context) };
        out.push((context, count));
        let mut next = INFCONTEXT::default();
        // SAFETY: same handle, next line.
        if unsafe { SetupFindNextLine(&context, &mut next) }.is_err() {
            break;
        }
        context = next;
    }
    out
}

/// `MM/DD/YYYY` → `YYYY-MM-DD`.
fn driver_date(text: &str) -> Option<String> {
    let mut parts = text.trim().split('/');
    let (m, d, y): (u32, u32, u32) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| format!("{y:04}-{m:02}-{d:02}"))
}

pub fn read_inf(path: &Path) -> Result<InfFacts, String> {
    let wide = HSTRING::from(path.as_os_str());
    let mut error_line = 0u32;
    // SAFETY: plain call; the handle is closed below.
    let inf =
        unsafe { SetupOpenInfFileW(&wide, PCWSTR::null(), INF_STYLE_WIN4, Some(&mut error_line)) };
    if inf.is_null() || inf == INVALID_HANDLE_VALUE.0 {
        return Err(format!(
            "{}: unreadable INF (line {error_line})",
            path.display()
        ));
    }
    let version_line = |key: &str| -> Option<INFCONTEXT> {
        let mut context = INFCONTEXT::default();
        // SAFETY: open handle, valid out structure.
        unsafe {
            SetupFindFirstLineW(
                inf,
                &HSTRING::from("Version"),
                &HSTRING::from(key),
                &mut context,
            )
        }
        .ok()
        .map(|()| context)
    };
    let driver_ver = version_line("DriverVer");
    let date = driver_ver
        .as_ref()
        .and_then(|c| field(c, 1))
        .and_then(|d| driver_date(&d));
    let version = driver_ver.as_ref().and_then(|c| field(c, 2));
    let provider = version_line("Provider").as_ref().and_then(|c| field(c, 1));
    let class = version_line("Class").as_ref().and_then(|c| field(c, 1));

    // [Manufacturer] → the models section Windows itself picks for this PC, then its IDs.
    let mut hardware_ids = Vec::new();
    for (manufacturer, _) in lines(inf, "Manufacturer") {
        let mut section = vec![0u16; 512];
        let mut needed = 0u32;
        // SAFETY: `manufacturer` is a line of the open INF; `section` holds 512 characters.
        let ok = unsafe {
            SetupDiGetActualModelsSectionW(
                &manufacturer,
                None,
                Some(&mut section),
                Some(&mut needed),
                None,
            )
        };
        if ok.is_err() {
            continue;
        }
        let end = section
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(section.len());
        let name = String::from_utf16_lossy(&section[..end]);
        for (model, count) in lines(inf, &name) {
            hardware_ids.extend((2..=count).filter_map(|i| field(&model, i)));
        }
    }
    // SAFETY: closes the handle opened above, once.
    unsafe { SetupCloseInfFile(inf) };
    hardware_ids.sort();
    hardware_ids.dedup();
    Ok(InfFacts {
        path: path.display().to_string(),
        hardware_ids,
        version,
        date,
        provider,
        class,
        signer: inf_path_signer(path),
    })
}
