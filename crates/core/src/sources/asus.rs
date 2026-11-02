//! ASUS motherboards (ADR-0032; format seen on 2026-11-01, docs/research/Sources phase 3.md):
//! the BIOS list is shown, never downloaded; the driver list has no hardware ID, so an entry is
//! only a hint tied to a device by its family and name (R-MAT-01: never a candidate).

use serde::Deserialize;

use super::hosts::{HttpsUrl, UrlRefused};
use super::text::{DateOrder, plain, slash_date};
use crate::decide::{Version, parse_version};
use crate::model::{Board, Device};

/// Windows 11 64-bit in ASUS's lists (`GetPDOS`).
const OS_WINDOWS_11: u32 = 52;

/// R-SRC-07: ASUS is asked only when the system or its motherboard is made by ASUS.
pub fn is_asus(board: &Board) -> bool {
    [&board.manufacturer, &board.system_manufacturer]
        .into_iter()
        .flatten()
        .any(|m| m.to_ascii_lowercase().starts_with("asus"))
}

/// `ROG STRIX Z790-E GAMING WIFI` → `rog-strix-z790-e-gaming-wifi`, the model key of ASUS's lists.
pub fn slug(product: &str) -> Option<String> {
    let words: Vec<String> = product
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let slug = words.join("-");
    (!slug.is_empty() && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .then_some(slug)
}

pub fn bios_url(slug: &str) -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!(
        "https://rog.asus.com/support/webapi/ProductV2/GetPDBIOS?website=global&model={slug}&cpu="
    ))
}

pub fn drivers_url(slug: &str) -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!(
        "https://rog.asus.com/support/webapi/ProductV2/GetPDDrivers?website=global&model={slug}&osid={OS_WINDOWS_11}&cpu="
    ))
}

/// One file of an ASUS list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// `LAN`, `Chipset`, `BIOS`…
    pub category: String,
    pub title: String,
    pub version: String,
    /// `2026-07-02`.
    pub date: Option<String>,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Result")]
    result: Option<ResultBody>,
}

#[derive(Deserialize)]
struct ResultBody {
    #[serde(rename = "Obj", default)]
    obj: Vec<Category>,
}

#[derive(Deserialize)]
struct Category {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Files", default)]
    files: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "Version", default)]
    version: Option<String>,
    #[serde(rename = "ReleaseDate", default)]
    release_date: Option<String>,
}

/// `2026/07/02` → `2026-07-02`.
fn date(text: &str) -> Option<String> {
    slash_date(text, DateOrder::Ymd)
}

/// `05/19/2025` (BIOS date as the firmware gives it) → `2025-05-19`.
pub fn firmware_date(text: &str) -> Option<String> {
    slash_date(text, DateOrder::Mdy)
}

/// A BIOS or driver list. `Ok(None)`: ASUS does not know this model (`Status: FAIL`), which is
/// "unknown", not a refusal.
pub fn read_list(text: &str) -> Result<Option<Vec<File>>, &'static str> {
    let answer: Answer = serde_json::from_str(text.trim()).map_err(|_| "unreadable JSON")?;
    if answer.status == "FAIL" {
        return Ok(None);
    }
    if answer.status != "SUCCESS" {
        return Err("unexpected status");
    }
    let body = answer.result.ok_or("no result")?;
    Ok(Some(
        body.obj
            .into_iter()
            .flat_map(|c| {
                let category = plain(&c.name);
                c.files.into_iter().filter_map(move |f| {
                    let version = plain(f.version.as_deref()?);
                    (!version.is_empty()).then(|| File {
                        category: category.clone(),
                        title: plain(f.title.as_deref().unwrap_or_default()),
                        version,
                        date: f.release_date.as_deref().and_then(date),
                    })
                })
            })
            .collect(),
    ))
}

/// The installed BIOS against ASUS's newest. `None` when either cannot be read as a number
/// (R-VRAI-04: then nothing is claimed).
pub fn bios_is_newer(installed: &str, latest: &str) -> Option<bool> {
    let number = |s: &str| {
        let s = s.trim();
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    };
    Some(number(latest)? > number(installed)?)
}

/// The newest BIOS of the list (highest numeric version).
pub fn newest_bios(files: &[File]) -> Option<&File> {
    files
        .iter()
        .filter(|f| f.version.bytes().all(|b| b.is_ascii_digit()))
        .max_by_key(|f| f.version.parse::<u64>().unwrap_or(0))
}

/// How an ASUS entry is tied to a device (ADR-0032): its category, a word of its title, the
/// Windows class of the device and a word of the device's name. Utilities and chipset INF
/// packages (which cover many devices) are never tied.
struct Rule {
    category: &'static str,
    title_any: &'static [&'static str],
    class: &'static str,
    name_any: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        category: "Chipset",
        title_any: &["Intel ME", "Management Engine"],
        class: "System",
        name_any: &["Management Engine"],
    },
    Rule {
        category: "SATA",
        title_any: &["Rapid Storage"],
        class: "SCSIAdapter",
        name_any: &["RST", "VMD", "Rapid Storage"],
    },
    Rule {
        category: "LAN",
        title_any: &["Intel"],
        class: "Net",
        name_any: &["Ethernet"],
    },
    Rule {
        category: "LAN",
        title_any: &["Realtek"],
        class: "Net",
        name_any: &["Realtek"],
    },
    Rule {
        category: "Wireless",
        title_any: &["Intel"],
        class: "Net",
        name_any: &["Wi-Fi", "Wireless"],
    },
    Rule {
        category: "Bluetooth",
        title_any: &["Intel"],
        class: "Bluetooth",
        name_any: &["Intel"],
    },
    Rule {
        category: "Bluetooth",
        title_any: &["Realtek"],
        class: "Bluetooth",
        name_any: &["Realtek"],
    },
    Rule {
        category: "Audio",
        title_any: &["Realtek"],
        class: "MEDIA",
        name_any: &["Realtek"],
    },
    Rule {
        category: "VGA Drivers",
        title_any: &["Intel"],
        class: "Display",
        name_any: &["Intel"],
    },
    Rule {
        category: "Thunderbolt",
        title_any: &["Thunderbolt"],
        class: "",
        name_any: &["Thunderbolt"],
    },
];

fn has(text: &str, words: &[&str]) -> bool {
    let text = text.to_ascii_lowercase();
    words.iter().any(|w| text.contains(&w.to_ascii_lowercase()))
}

/// Model words of a title (`I226`, `AX210`, `ALC4080`): letters then at least 3 digits.
fn models(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| {
            let letters = w.bytes().take_while(u8::is_ascii_alphabetic).count();
            let digits = w
                .bytes()
                .skip(letters)
                .take_while(u8::is_ascii_digit)
                .count();
            // `V2627` is a version, not a model.
            let version = letters == 1 && w.as_bytes()[0].eq_ignore_ascii_case(&b'v');
            (1..=3).contains(&letters) && digits >= 3 && !version
        })
        .map(str::to_ascii_uppercase)
        .collect()
}

/// Chip makers a title may name; the installed driver must be from the same one.
const MAKERS: &[&str] = &["Intel", "Realtek", "Marvell", "MediaTek", "AMD", "Aquantia"];

fn ties(file: &File, device: &Device) -> bool {
    let name = device.description.as_deref().unwrap_or_default();
    let class = device.class.as_deref().unwrap_or_default();
    // Seen on the reference PC (2026-11-01): "Intel WiFi driver" must not go to Microsoft's virtual
    // Wi-Fi Direct adapter. The maker of the title is the provider of the installed driver.
    let provider = device
        .driver
        .as_ref()
        .and_then(|d| d.provider.as_deref())
        .unwrap_or_default();
    match MAKERS.iter().find(|m| has(&file.title, &[m])) {
        Some(maker) if has(provider, &[maker]) => {}
        _ => return false,
    }
    // A USB audio driver is for a USB device, an HD audio driver for the others.
    let usb_device = device.instance_id.to_ascii_uppercase().starts_with("USB\\");
    if file.category == "Audio" && has(&file.title, &["USB"]) != usb_device {
        return false;
    }
    // A model named in the title (I226) must be in the device's name.
    let named = models(&file.title);
    if !named.is_empty() && !named.iter().any(|m| name.to_ascii_uppercase().contains(m)) {
        return false;
    }
    RULES.iter().any(|r| {
        r.category.eq_ignore_ascii_case(&file.category)
            && has(&file.title, r.title_any)
            && (r.class.is_empty() || r.class.eq_ignore_ascii_case(class))
            && has(name, r.name_any)
    })
}

/// The newest ASUS entry tied to `device` and newer than its driver: a hint only (ADR-0032).
pub fn hint_for<'a>(device: &Device, files: &'a [File], installed: Version) -> Option<&'a File> {
    files
        .iter()
        .filter(|f| ties(f, device))
        .filter_map(|f| Some((parse_version(&f.version)?, f)))
        .filter(|(v, _)| *v > installed)
        .max_by_key(|(v, _)| *v)
        .map(|(_, f)| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_of_motherboard_names() {
        assert_eq!(
            slug("ROG STRIX Z790-E GAMING WIFI").as_deref(),
            Some("rog-strix-z790-e-gaming-wifi")
        );
        assert_eq!(
            slug(" TUF GAMING B650-PLUS WIFI ").as_deref(),
            Some("tuf-gaming-b650-plus-wifi")
        );
        assert_eq!(slug("PRIME Z790-P/CSM"), None, "never a path in the URL");
        assert_eq!(slug("   "), None);
    }

    #[test]
    fn model_words_of_titles() {
        assert_eq!(models("Intel  I226 LAN driver V2.1.5.10"), ["I226"]);
        assert_eq!(models("Intel WiFi Driver V23.80.0.7"), Vec::<String>::new());
        assert_eq!(
            models("Intel ME V2627.10.5.0 for Windows 11"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn bios_numbers() {
        assert_eq!(bios_is_newer("3001", "3202"), Some(true));
        assert_eq!(bios_is_newer("3202", "3202"), Some(false));
        assert_eq!(bios_is_newer("ALASKA - 1072009", "3202"), None);
        assert_eq!(firmware_date("05/19/2025").as_deref(), Some("2025-05-19"));
        assert_eq!(firmware_date("2025-05-19"), None);
    }
}
