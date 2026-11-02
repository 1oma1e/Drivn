//! Asks the Windows Update Agent which drivers it offers, without downloading anything
//! (docs/research/Windows - drivers.md, §3). An online search is the only network access of a scan
//! (R-VRAI-02); the cached search uses none (ADR-0017).

use drivn_core::model::{DriverOffer, WindowsUpdateResult};
use drivn_core::scan::UpdateSource;
use drivn_core::time::{date_from_ole, iso_utc_from_ole};
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::UpdateAgent::{
    AutomaticUpdates, IAutomaticUpdates2, IUpdate, IUpdateSession, IWindowsDriverUpdate,
    OperationResultCode, UpdateSession, orcSucceeded,
};
use windows::Win32::System::Variant::{VARIANT, VT_DATE};
use windows::core::{BSTR, Interface};

const CRITERIA: &str = "IsInstalled=0 and Type='Driver'";

/// Searches the default update service of this PC: online, or in the list Windows kept from its
/// own last automatic search (`online: false`, no network, ADR-0017).
pub struct WindowsUpdateAgent {
    pub online: bool,
}

impl UpdateSource for WindowsUpdateAgent {
    fn driver_offers(&self) -> WindowsUpdateResult {
        let last_search_utc = last_search_utc();
        let found = if !self.online && last_search_utc.is_none() {
            // An empty cache is not "nothing to update" (R-VRAI-04).
            Err("Windows has never searched for updates on this PC".to_owned())
        } else {
            search(self.online)
        };
        let (offers, error) = match found {
            Ok(offers) => (offers, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        WindowsUpdateResult {
            criteria: CRITERIA.to_owned(),
            online: self.online,
            last_search_utc,
            offers,
            error,
        }
    }
}

/// When Windows itself last searched successfully (UTC, Microsoft Learn); `None` if never or unreadable.
fn last_search_utc() -> Option<String> {
    // SAFETY: plain COM calls on interfaces owned by this function (see `search`).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let updates: IAutomaticUpdates2 =
            CoCreateInstance(&AutomaticUpdates, None, CLSCTX_INPROC_SERVER).ok()?;
        let date = updates.Results().ok()?.LastSearchSuccessDate().ok()?;
        iso_utc_from_ole(ole_date(&date)?)
    }
}

/// The DATE held by a VT_DATE VARIANT. `VariantToDouble` refuses VT_DATE ("Type mismatch", seen on
/// the reference PC on 2026-10-31), so the field is read directly; any other type (VT_EMPTY: never) is `None`.
fn ole_date(value: &VARIANT) -> Option<f64> {
    // SAFETY: the union is read as a DATE only when the VARIANT says it holds one.
    unsafe {
        let inner = &value.Anonymous.Anonymous;
        (inner.vt == VT_DATE).then(|| inner.Anonymous.date)
    }
}

fn text(value: windows::core::Result<BSTR>) -> Option<String> {
    let value = value.ok()?.to_string();
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The first download link WUA gives for this update (read only, nothing is downloaded).
fn download_url(update: &IUpdate) -> Option<String> {
    // SAFETY: COM getters on a live update of this search; no pointer is kept.
    unsafe {
        let contents = update.DownloadContents().ok()?;
        let count = contents.Count().ok()?;
        (0..count).find_map(|i| text(contents.get_Item(i).ok()?.DownloadUrl()))
    }
}

fn search(online: bool) -> Result<Vec<DriverOffer>, String> {
    // SAFETY: plain COM calls on interfaces owned by this function. CoInitializeEx may return
    // S_FALSE (already initialised, e.g. by the WMI reader on this thread): that is fine.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let session: IUpdateSession = CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("UpdateSession: {e}"))?;
        let searcher = session
            .CreateUpdateSearcher()
            .map_err(|e| format!("CreateUpdateSearcher: {e}"))?;
        searcher
            .SetOnline(if online { VARIANT_TRUE } else { VARIANT_FALSE })
            .map_err(|e| format!("Online: {e}"))?;
        let result = searcher
            .Search(&BSTR::from(CRITERIA))
            .map_err(|e| format!("Search: {e}"))?;
        let code: OperationResultCode = result
            .ResultCode()
            .map_err(|e| format!("ResultCode: {e}"))?;
        if code != orcSucceeded {
            return Err(format!("search finished with result code {}", code.0));
        }
        let updates = result.Updates().map_err(|e| format!("Updates: {e}"))?;
        let count = updates.Count().map_err(|e| format!("Count: {e}"))?;
        let mut offers = Vec::new();
        for index in 0..count {
            let update = updates
                .get_Item(index)
                .map_err(|e| format!("Item {index}: {e}"))?;
            let Ok(driver) = update.cast::<IWindowsDriverUpdate>() else {
                continue; // Type='Driver' should only return drivers; skip anything else
            };
            let identity = update.Identity().ok();
            offers.push(DriverOffer {
                title: text(update.Title()),
                hardware_id: text(driver.DriverHardwareID()),
                driver_date: driver.DriverVerDate().ok().and_then(date_from_ole),
                provider: text(driver.DriverProvider()),
                manufacturer: text(driver.DriverManufacturer()),
                model: text(driver.DriverModel()),
                class: text(driver.DriverClass()),
                update_id: identity.as_ref().and_then(|i| text(i.UpdateID())),
                revision: identity.as_ref().and_then(|i| i.RevisionNumber().ok()),
                download_url: download_url(&update),
            });
        }
        Ok(offers)
    }
}
