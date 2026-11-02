//! A publisher's own installer, opened by Drivn: NVIDIA's for the graphics card (ADR-0030),
//! Intel's for Wi-Fi and Bluetooth (ADR-0033). Drivn downloads the official package, checks it,
//! then starts the publisher's installer, which installs the drivers and their components. Drivn
//! never installs these drivers from their INF. No backup or rollback by Drivn (R-SEC-09).
//! ADR-0034: a package of `official::OFFICIAL` (NVIDIA) is downloaded and checked here, then run
//! silently by the elevated installer inside the pass, which checks it again on its own copy.

use std::path::{Path, PathBuf};

use drivn_core::install::official;
use drivn_core::install::plan::PackageItem;
use drivn_core::model::{Snapshot, VendorOffer};
use drivn_core::sources::SourceId;
use drivn_core::sources::hosts::HttpsUrl;
use drivn_core::sources::intel::sha512_hex;
use drivn_windows::authenticode::check_signed_exe;
use drivn_windows::http::download_to_file;
use drivn_windows::security::is_reparse_point;
use serde::Serialize;
use tauri::ipc::Channel;

use crate::pass::PassState;

/// What each publisher's package must be.
struct Publisher {
    source: SourceId,
    signer: &'static str,
    /// R-SRC-01: the link starts with this and ends with `.exe`.
    prefix: &'static str,
    folder: &'static str,
    max_bytes: u64,
    /// R-SRC-02: the source gives a SHA-512 and it must be compared before anything.
    needs_hash: bool,
}

const PUBLISHERS: &[Publisher] = &[
    Publisher {
        source: SourceId::Nvidia,
        signer: "NVIDIA Corporation",
        prefix: "https://us.download.nvidia.com/Windows/",
        folder: "nvidia",
        // The package is about 1 GB.
        max_bytes: 3 * 1024 * 1024 * 1024,
        needs_hash: false,
    },
    Publisher {
        source: SourceId::Intel,
        // To verify on the real package (ADR-0033).
        signer: "Intel Corporation",
        prefix: "https://downloadmirror.intel.com/",
        folder: "intel",
        // Wi-Fi and Bluetooth packages are 60-75 MB.
        max_bytes: 512 * 1024 * 1024,
        needs_hash: true,
    },
];

#[derive(Clone, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum PackageProgress {
    Download { mb: u64 },
    Check,
    Launched,
}

/// The offer of the last scan for this device, never one sent by the interface (R-SEC-05).
fn offer(state: &PassState, device_id: &str) -> Result<(&'static Publisher, VendorOffer), String> {
    let scan = state
        .last_scan
        .lock()
        .map_err(|_| "state unreadable")?
        .clone()
        .ok_or("scan_first")?;
    offer_in(&scan.0, device_id)
}

fn offer_in(
    snapshot: &Snapshot,
    device_id: &str,
) -> Result<(&'static Publisher, VendorOffer), String> {
    snapshot
        .vendors
        .iter()
        .find_map(|r| {
            let publisher = PUBLISHERS.iter().find(|p| p.source == r.source)?;
            let offer = r
                .offers
                .iter()
                .find(|o| o.device_instance_id == device_id && o.download_url.is_some())?;
            Some((publisher, offer.clone()))
        })
        .ok_or_else(|| "no_package".to_owned())
}

fn package_url(publisher: &Publisher, offer: &VendorOffer) -> Result<HttpsUrl, String> {
    let link = offer.download_url.as_deref().unwrap_or_default();
    if !link.starts_with(publisher.prefix) || !link.to_ascii_lowercase().ends_with(".exe") {
        return Err("package link refused".to_owned());
    }
    HttpsUrl::parse(link).map_err(|e| e.to_string())
}

fn cache_file(publisher: &Publisher, url: &HttpsUrl) -> Result<PathBuf, String> {
    let name = url
        .path()
        .rsplit('/')
        .next()
        .filter(|n| {
            !n.is_empty()
                && !n.starts_with('.')
                && n.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        .ok_or("file name refused")?;
    let dir = drivn_windows::security::local_app_data()?
        .join("Drivn")
        .join("cache")
        .join(publisher.folder);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

/// R-SRC-02 first (hash of the source), then R-SRC-08 (nothing after the signed zone) and
/// R-SRC-03 (valid signature of the publisher).
fn check(publisher: &Publisher, offer: &VendorOffer, path: &Path) -> Result<(), String> {
    if is_reparse_point(path) {
        return Err("the file is a link".to_owned());
    }
    if publisher.needs_hash {
        let expected = offer
            .sha512
            .as_deref()
            .ok_or("no SHA-512 given by the source")?;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let actual = sha512_hex(file).map_err(|e| e.to_string())?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("SHA-512 differs from the official page".to_owned());
        }
    }
    check_signed_exe(path, publisher.signer, false)
}

/// Downloads the package of this device's offer (or reuses the cached one) and checks it.
pub fn fetch(
    state: &PassState,
    device_id: &str,
    progress: &Channel<PackageProgress>,
) -> Result<PathBuf, String> {
    let (publisher, offer) = offer(state, device_id)?;
    let url = package_url(publisher, &offer)?;
    let path = cache_file(publisher, &url)?;
    // A package already downloaded is reused only if it passes every check again.
    if check(publisher, &offer, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let mut last = 0;
        download_to_file(&url, &path, publisher.max_bytes, &mut |written| {
            let mb = written / (1024 * 1024);
            if mb != last {
                last = mb;
                let _ = progress.send(PackageProgress::Download { mb });
            }
        })?;
    }
    let _ = progress.send(PackageProgress::Check);
    if let Err(why) = check(publisher, &offer, &path) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("package refused: {why}"));
    }
    Ok(path)
}

/// The package of this device for the plan (R-SEC-09): already fetched and still passing every
/// check, of a publisher whose installer runs silently in the pass.
pub fn planned(snapshot: &Snapshot, device_id: &str) -> Result<PackageItem, String> {
    let (publisher, offer) = offer_in(snapshot, device_id)?;
    let source = publisher.source.key();
    let known = official::official(source).ok_or("no silent install for this package")?;
    let url = package_url(publisher, &offer)?;
    let path = cache_file(publisher, &url)?;
    check(publisher, &offer, &path).map_err(|why| format!("package refused: {why}"))?;
    let file = path.to_string_lossy().into_owned();
    if !official::valid_file(known, &file) {
        return Err("package path refused".to_owned());
    }
    let device = snapshot.devices.iter().find(|d| d.instance_id == device_id);
    Ok(PackageItem {
        device_instance_id: device_id.to_owned(),
        source: source.to_owned(),
        name: device
            .and_then(|d| d.description.clone())
            .unwrap_or_else(|| device_id.to_owned()),
        from: device
            .and_then(|d| d.driver.as_ref())
            .and_then(|d| d.version.clone())
            .unwrap_or_default(),
        to: offer.vendor_version.unwrap_or_default(),
        file,
    })
}

/// The manual button: the publisher's installer opened with its own windows.
pub fn install(
    state: &PassState,
    device_id: &str,
    progress: &Channel<PackageProgress>,
) -> Result<(), String> {
    let path = fetch(state, device_id, progress)?;
    // Not elevated: the publisher's installer asks Windows for the rights itself.
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("installer not started: {e}"))?;
    let _ = progress.send(PackageProgress::Launched);
    Ok(())
}
