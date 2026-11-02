//! Snapshot schema written by `drivn-scan` (docs/specs/Spec - Scan lecture seule.md).
//! Every value that Windows did not give us stays `None` / `Unknown`: we never guess (R-VRAI-04).

use serde::{Deserialize, Serialize};

use crate::sources::SourceId;

/// Bump when a field changes meaning or disappears; adding an optional field does not need it.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub tool_version: String,
    pub created_utc: String,
    pub machine: Option<String>,
    pub anonymized: bool,
    pub includes_absent_devices: bool,
    pub devices: Vec<Device>,
    /// `None` when the Windows Update search was not requested (no network call was made).
    pub windows_update: Option<WindowsUpdateResult>,
    /// Sources other than Windows Update (phase 3); empty when none was asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vendors: Vec<VendorResult>,
}

/// What one fragile source answered for this PC (Spec - Sources constructeurs, section 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorResult {
    pub source: SourceId,
    /// The whole source failed or is blocked for 24 h (R-SRC-11): "unknown" for it alone.
    pub error: Option<String>,
    pub requests: u32,
    pub offers: Vec<VendorOffer>,
    /// Devices this source should have checked but could not fully (limit reached, list cut by
    /// the site, error): never "up to date" on its word (R-VRAI-04).
    #[serde(default)]
    pub unchecked: Vec<String>,
    #[serde(default)]
    pub hints: Vec<VendorHint>,
    /// Oracle: latest Java 8 update number (`1.8.0_511` → 511).
    #[serde(default)]
    pub java8_latest_update: Option<u32>,
    /// ASUS: the BIOS of this motherboard and ASUS's newest, shown only (ADR-0032).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bios: Option<BiosCheck>,
}

/// The motherboard and BIOS read without admin rights in `HKLM\HARDWARE\DESCRIPTION\System\BIOS`
/// (R-SRC-07, ADR-0032).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    pub system_manufacturer: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
}

/// BIOS installed against the newest one of the motherboard's maker. Never downloaded nor
/// installed: the BIOS stays out of Drivn (VISION).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiosCheck {
    pub board: String,
    pub installed: Option<String>,
    /// `2025-05-19`, from the registry's `05/19/2025`.
    #[serde(default)]
    pub installed_date: Option<String>,
    pub latest: String,
    pub latest_date: Option<String>,
    /// `None` when the two versions cannot be compared (R-VRAI-04).
    pub newer: Option<bool>,
}

/// A newer driver announced by a fragile source (R-VER-03: announced, confirmed at install).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorOffer {
    pub device_instance_id: String,
    /// The device's own ID, equal (case ignored) to an ID of the source (R-MAT-01).
    pub hardware_id: String,
    pub title: Option<String>,
    /// DriverVer-style version when the source gives one (Catalog).
    pub version: Option<String>,
    /// The version as the publisher writes it (NVIDIA `621.01`), for display only.
    #[serde(default)]
    pub vendor_version: Option<String>,
    /// The source compared it in its own numbering (NVIDIA) and found it newer.
    #[serde(default)]
    pub announced_newer: bool,
    pub date: Option<String>,
    pub provider: Option<String>,
    /// Who published it at the source (a PC maker for some Catalog drivers).
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub size_text: Option<String>,
    /// Catalog update ID: what the installer looks up again itself (R-SEC-05).
    #[serde(default)]
    pub update_id: Option<String>,
    /// `false`: shown with its link, never put in a plan (NVIDIA graphics, Spec 4.2).
    pub installable: bool,
    /// NVIDIA: the official package, opened by NVIDIA's own installer (ADR-0030). Checked
    /// again against the host list before use (R-SRC-01).
    #[serde(default)]
    pub download_url: Option<String>,
    /// SHA-512 of the package given by the source (Intel, ADR-0033), compared before anything
    /// (R-SRC-02).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorHint {
    pub device_instance_id: String,
    pub kind: HintKind,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintKind {
    /// NVIDIA HD Audio: the NVIDIA package may hold a newer audio driver (known only once opened).
    PackageMayContainNewer,
    /// The publisher may have newer; Drivn could not check (site blocked or nothing readable).
    VendorMayHaveNewer,
    /// The motherboard maker lists a higher version for a driver tied by family and name, not by
    /// hardware ID (ADR-0032): a hint, never an update.
    PublisherListsNewer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub instance_id: String,
    pub description: Option<String>,
    pub class: Option<String>,
    pub class_guid: Option<String>,
    pub manufacturer: Option<String>,
    pub present: bool,
    /// Device Manager problem code (`CM_PROB_*`); `None` when the device has no problem.
    pub problem_code: Option<u32>,
    pub hardware_ids: Vec<String>,
    pub compatible_ids: Vec<String>,
    /// `None` when no driver is installed for this device.
    pub driver: Option<InstalledDriver>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledDriver {
    pub version: Option<String>,
    /// `YYYY-MM-DD`, from the INF DriverVer date.
    pub date: Option<String>,
    pub provider: Option<String>,
    /// `oemNN.inf` for third-party drivers, the inbox INF name otherwise.
    pub inf_path: Option<String>,
    pub rank: Option<u32>,
    /// The hardware or compatible ID Windows used to pick this driver.
    pub matching_device_id: Option<String>,
    pub signature: Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Signature {
    Signed { signer: Option<String> },
    Unsigned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowsUpdateResult {
    pub criteria: String,
    /// `true`: searched online now. `false`: the list Windows kept from its own last automatic
    /// search (ADR-0017). Snapshots made before this field were all online.
    #[serde(default = "searched_online")]
    pub online: bool,
    /// Last successful search by Windows itself (`YYYY-MM-DDTHH:MM:SSZ`): the age of a cached list.
    #[serde(default)]
    pub last_search_utc: Option<String>,
    pub offers: Vec<DriverOffer>,
    /// Set when the search failed; `offers` is then empty and must not be read as "up to date".
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverOffer {
    pub title: Option<String>,
    pub hardware_id: Option<String>,
    /// `YYYY-MM-DD`, from `IWindowsDriverUpdate::DriverVerDate`.
    pub driver_date: Option<String>,
    pub provider: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub class: Option<String>,
    /// WUA identity (`IUpdate::Identity`): what the installer asks Windows Update to install.
    /// Snapshots made before phase 2 do not have it.
    #[serde(default)]
    pub update_id: Option<String>,
    #[serde(default)]
    pub revision: Option<i32>,
    /// Where Windows Update downloads it from (`IUpdateDownloadContent::DownloadUrl`), shown to
    /// the user before the install. Not every offer gives one.
    #[serde(default)]
    pub download_url: Option<String>,
}

fn searched_online() -> bool {
    true
}
