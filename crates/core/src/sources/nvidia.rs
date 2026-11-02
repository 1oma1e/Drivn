//! NVIDIA's driver lookup service (Spec - Sources constructeurs, 4.2; format seen on 2026-11-01).
//! The version is only announced: the graphics driver is never installed by Drivn in this phase.

use serde::Deserialize;

use super::hosts::{HttpsUrl, UrlRefused};
use super::text::{percent_decode, plain};

/// NVIDIA product IDs (`pfid`) by PCI device ID of NVIDIA (`VEN_10DE&DEV_xxxx`). From NVIDIA's own
/// product list (copied by the community in `ZenitH-AT/nvidia-data`); a card not listed is
/// "unknown", never matched by its name (R-MAT-01). Only pairs seen on a real PC are listed
/// (`DEV_2702` in the reference snapshot, 2026-11-01); a pair from memory would be "à vérifier".
const PRODUCTS: &[(&str, u32, &str)] = &[("2702", 1041, "GeForce RTX 4080 SUPER")];
/// Windows 11 in NVIDIA's lookup.
const OS_WINDOWS_11: u32 = 135;

/// The `pfid` of an NVIDIA graphics card from one of its hardware IDs.
pub fn product_id(hardware_ids: &[String]) -> Option<u32> {
    hardware_ids.iter().find_map(|id| {
        let upper = id.to_ascii_uppercase();
        let rest = upper.strip_prefix("PCI\\VEN_10DE&DEV_")?;
        let dev = rest.get(..4)?;
        PRODUCTS
            .iter()
            .find(|(d, _, _)| *d == dev)
            .map(|(_, pfid, _)| *pfid)
    })
}

pub fn lookup_url(pfid: u32) -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!(
        "https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup&pfid={pfid}&osID={OS_WINDOWS_11}&languageCode=1033&isWHQL=1&dch=1&sort1=0&numberOfResults=1"
    ))
}

/// NVIDIA's version (`616.92`) from a Windows DriverVer (`32.0.16.1692`): its last 5 digits.
pub fn nvidia_version(driver_ver: &str) -> Option<(u32, u32)> {
    let digits: String = driver_ver.split('.').skip(2).collect();
    if digits.len() < 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let last = &digits[digits.len() - 5..];
    Some((last[..3].parse().ok()?, last[3..].parse().ok()?))
}

/// `621.01` → (621, 1).
pub fn parse_release(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.trim().split_once('.')?;
    if minor.len() != 2 {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: (u32, u32),
    pub version_text: String,
    pub date: Option<String>,
    pub name: String,
    pub download_url: HttpsUrl,
    pub size_text: String,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "Success")]
    success: String,
    #[serde(rename = "IDS", default)]
    ids: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "downloadInfo")]
    info: Info,
}

#[derive(Deserialize)]
struct Info {
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "ReleaseDateTime", default)]
    release_date: String,
    #[serde(rename = "DownloadURL")]
    download_url: String,
    #[serde(rename = "DownloadURLFileSize", default)]
    size: String,
}

/// `Wed Oct 21, 2026` → `2026-10-21`.
fn release_date(text: &str) -> Option<String> {
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut words = text.split_whitespace().skip(1);
    let month = months.iter().position(|m| Some(*m) == words.next())? + 1;
    let day: u32 = words.next()?.trim_end_matches(',').parse().ok()?;
    let year: u32 = words.next()?.parse().ok()?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// The JSON answer (served as `text/html`, sometimes wrapped in `<body>`).
pub fn read_lookup(text: &str) -> Result<Option<Release>, &'static str> {
    let start = text.find('{').ok_or("no JSON")?;
    let end = text.rfind('}').ok_or("no JSON")?;
    let answer: Answer = serde_json::from_str(&text[start..=end]).map_err(|_| "unreadable JSON")?;
    if answer.success != "1" {
        return Ok(None);
    }
    let Some(entry) = answer.ids.into_iter().next() else {
        return Ok(None);
    };
    let info = entry.info;
    let version = parse_release(&info.version).ok_or("unreadable version")?;
    // R-SRC-01: the package must be on NVIDIA's download host, in its driver folder.
    if !info
        .download_url
        .starts_with("https://us.download.nvidia.com/Windows/")
    {
        return Err("download link not in the expected format");
    }
    let download_url = HttpsUrl::parse(&info.download_url).map_err(|_| "download link refused")?;
    Ok(Some(Release {
        version,
        version_text: plain(&info.version),
        date: release_date(&percent_decode(&info.release_date)),
        name: plain(&percent_decode(&info.name)),
        download_url,
        size_text: plain(&info.size),
    }))
}
