//! Intel's download pages (ADR-0033; format seen on 2026-11-01, docs/research/Sources phase 3.md):
//! one page per package, read as plain HTML. The page has no hardware ID: Drivn's own table ties
//! an ID of the device (equal, R-MAT-01) to a page, and the page must still name the product.
//! The version is only announced (R-VER-03); Intel's own installer installs.

use std::io::Read;

use sha2::{Digest, Sha512};

use super::hosts::{HttpsUrl, UrlRefused};
use super::text::{DateOrder, hex, plain, slash_date};

/// A hardware ID, the Intel download page for it and the product (SKU) the page must list.
/// Only pairs seen on a real PC (the reference PC, 2026-11-01); still to verify in the package's INF.
pub struct Entry {
    pub hardware_id: &'static str,
    pub family: &'static str,
    pub page: &'static str,
    pub sku: u32,
}

pub const TABLE: &[Entry] = &[
    Entry {
        hardware_id: "PCI\\VEN_8086&DEV_7A70",
        family: "Wi-Fi",
        page: "/content/www/us/en/download/19351/intel-wireless-wi-fi-drivers-for-windows-10-and-windows-11.html",
        sku: 204_837,
    },
    Entry {
        hardware_id: "USB\\VID_8087&PID_0033",
        family: "Bluetooth",
        page: "/content/www/us/en/download/18649/intel-wireless-bluetooth-drivers-for-windows-10-and-windows-11.html",
        sku: 204_837,
    },
];

/// The table entry of a device: one of its IDs equal to the table's, case ignored (R-MAT-01).
pub fn entry_for(ids: &[String]) -> Option<&'static Entry> {
    TABLE
        .iter()
        .find(|e| ids.iter().any(|id| id.eq_ignore_ascii_case(e.hardware_id)))
}

pub fn page_url(entry: &Entry) -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!("https://www.intel.com{}", entry.page))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Version of the package (`24.80.0`).
    pub version: String,
    pub date: Option<String>,
    pub products: Vec<u32>,
    pub package: HttpsUrl,
    /// Lowercase hex, 128 characters.
    pub sha512: String,
    pub size_text: Option<String>,
}

fn meta<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let at = html.find(&format!("<meta name=\"{name}\" content=\""))?;
    let rest = &html[at..];
    let start = rest.find("content=\"")? + 9;
    let end = rest[start..].find('"')?;
    Some(&rest[start..start + end])
}

/// Every distinct value that starts with `prefix` and is made of `allowed` bytes.
fn all_after(html: &str, prefix: &str, allowed: impl Fn(u8) -> bool) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(prefix) {
        let tail = &rest[at + prefix.len()..];
        let end = tail.bytes().position(|b| !allowed(b)).unwrap_or(tail.len());
        let value = tail[..end].to_owned();
        if !found.contains(&value) {
            found.push(value);
        }
        rest = &tail[end..];
    }
    found
}

/// `10/20/2026 07:00:00` → `2026-10-20`.
fn us_date(text: &str) -> Option<String> {
    slash_date(text, DateOrder::Mdy)
}

/// Reads a download page. Exactly one 64-bit `.exe` and one SHA-512, or the page is unreadable:
/// two packages on one page could pair a link with the hash of the other.
pub fn read_page(html: &str) -> Result<Page, &'static str> {
    let version = plain(meta(html, "DownloadVersion").ok_or("no version")?);
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("unreadable version");
    }
    let products = meta(html, "RelatedProductIds")
        .ok_or("no product list")?
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    let link_byte = |b: u8| b.is_ascii_alphanumeric() || b"/-_.".contains(&b);
    let links: Vec<String> = all_after(html, "https://downloadmirror.intel.com/", link_byte)
        .into_iter()
        .filter(|l| l.ends_with(".exe"))
        .collect();
    let [link] = links.as_slice() else {
        return Err("not exactly one package");
    };
    // R-SRC-01: `<number>/<file>.exe`, nothing else.
    let (folder, file) = link.split_once('/').ok_or("package link format")?;
    if folder.is_empty()
        || !folder.bytes().all(|b| b.is_ascii_digit())
        || file.contains('/')
        || file.starts_with('.')
    {
        return Err("package link format");
    }
    let package = HttpsUrl::parse(&format!("https://downloadmirror.intel.com/{link}"))
        .map_err(|_| "package link refused")?;
    let hashes: Vec<String> = all_after(html, "SHA512: ", |b| b.is_ascii_hexdigit())
        .into_iter()
        .filter(|h| h.len() == 128)
        .map(|h| h.to_ascii_lowercase())
        .collect();
    let [sha512] = hashes.as_slice() else {
        return Err("not exactly one SHA-512");
    };
    let size_text = all_after(html, "Size: ", |b| {
        b.is_ascii_alphanumeric() || b == b'.' || b == b' '
    })
    .into_iter()
    .next()
    .map(|s| plain(&s));
    Ok(Page {
        version,
        date: meta(html, "LastUpdate").and_then(us_date),
        products,
        package,
        sha512: sha512.clone(),
        size_text,
    })
}

/// The first 3 numbers of a version: the package `24.80.0` covers drivers `24.80.0.x`.
pub fn release(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u32>().ok());
    let mut out = [0u32; 3];
    for slot in &mut out {
        *slot = parts.next().flatten()?;
    }
    Some(out)
}

/// R-SRC-02: the SHA-512 of a file, in lowercase hex, read in pieces.
pub fn sha512_hex(mut reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Sha512::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_compare_on_three_numbers() {
        assert_eq!(release("24.80.0"), Some([24, 80, 0]));
        assert_eq!(release("24.80.0.2"), Some([24, 80, 0]));
        assert!(release("24.90.0") > release("24.80.0.2"));
        assert_eq!(release("24.80"), None);
    }

    #[test]
    fn sha512_of_known_text() {
        assert_eq!(
            sha512_hex(&b"abc"[..]).unwrap(),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }
}
