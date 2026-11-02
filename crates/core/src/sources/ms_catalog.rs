//! Microsoft Update Catalog (Spec - Sources constructeurs, 4.1; formats seen on 2026-11-01 in
//! docs/research/Sources phase 3.md). Pure: builds the requests and reads the answers.

use super::hosts::{HttpsUrl, UrlRefused, encode_query};
use super::http::{Http, MAX_PAGE_BYTES, Request, fetch};
use super::text::{base64_decode, hex, plain};
use crate::decide::{Version, parse_version};

const SITE: &str = "https://www.catalog.update.microsoft.com";
const FILES: &str = "https://catalog.s.download.windowsupdate.com";
/// The Catalog returns at most this many results: above, the list is cut.
pub const RESULT_CAP: u32 = 1000;

/// Windows releases, newest first. The search adds one: an ID alone gives more than 1000 rows.
pub const RELEASES: &[&str] = &["26H2", "26H1", "25H2", "24H2", "23H2", "22H2", "21H2"];

/// The releases to try for a PC on `current` (its `DisplayVersion`): this one and the 2 before,
/// because a driver published "for 24H2 and later" is not listed under 26H2.
pub fn releases_from(current: Option<&str>) -> Vec<&'static str> {
    let start = current
        .and_then(|c| {
            RELEASES
                .iter()
                .position(|r| r.eq_ignore_ascii_case(c.trim()))
        })
        .unwrap_or(0);
    RELEASES.iter().skip(start).take(3).copied().collect()
}

/// The search term for a device ID: without the bus prefix, `SUBSYS_` and `REV_`, which the
/// Catalog does not index (`PCI\VEN_8086&DEV_7A68&SUBSYS_…&REV_11` → `VEN_8086&DEV_7A68`).
pub fn search_term(hardware_id: &str) -> Option<String> {
    let body = hardware_id.split_once('\\').map_or(hardware_id, |(_, b)| b);
    let kept: Vec<&str> = body
        .split('&')
        .filter(|part| {
            let upper = part.to_ascii_uppercase();
            !upper.starts_with("SUBSYS_") && !upper.starts_with("REV_")
        })
        .collect();
    let term = kept.join("&");
    let upper = term.to_ascii_uppercase();
    (upper.starts_with("VEN_") || upper.starts_with("VID_") || upper.starts_with("FUNC_"))
        .then_some(term)
}

pub fn search_url(term: &str, release: &str) -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!(
        "{SITE}/Search.aspx?q={}&scol=DriverVerVersion&sdir=desc",
        encode_query(&format!("{term} {release}"))
    ))
}

pub fn details_url(update_id: &str) -> Result<HttpsUrl, UrlRefused> {
    if !is_guid(update_id) {
        return Err(UrlRefused::Malformed);
    }
    HttpsUrl::parse(&format!(
        "{SITE}/ScopedViewInline.aspx?updateid={update_id}"
    ))
}

pub fn download_dialog_url() -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse(&format!("{SITE}/DownloadDialog.aspx"))
}

/// POST body of `DownloadDialog.aspx` for one update.
pub fn download_dialog_form(update_id: &str) -> Option<String> {
    is_guid(update_id).then(|| {
        let json = format!(
            "[{{\"size\":0,\"languages\":\"\",\"uidInfo\":\"{update_id}\",\"updateID\":\"{update_id}\"}}]"
        );
        format!("updateIDs={}", encode_query(&json))
    })
}

pub fn is_guid(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub update_id: String,
    pub title: String,
    pub products: String,
    pub classification: String,
    /// `YYYY-MM-DD`.
    pub date: Option<String>,
    pub version: Option<Version>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    pub rows: Vec<Row>,
    /// Total announced by the page ("1 - 25 of N"); 0 when the page says nothing was found.
    pub total: u32,
}

impl SearchPage {
    /// More results than the Catalog returns: the newest may be missing (never "up to date").
    pub fn capped(&self) -> bool {
        self.total >= RESULT_CAP
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Not the expected page (CAPTCHA, error page, changed site): R-SRC-11 blocks the source.
    UnexpectedFormat(&'static str),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedFormat(what) => write!(f, "unexpected page: {what}"),
        }
    }
}

/// Text of the element with this `id` up to its closing `</td>`, `</span>` or `</div>`.
fn cell(html: &str, id: &str) -> Option<String> {
    let start = html.find(&format!("id=\"{id}\""))?;
    let open_end = start + html[start..].find('>')? + 1;
    let rest = &html[open_end..];
    let end = ["</td>", "</span>", "</div>"]
        .iter()
        .filter_map(|tag| rest.find(tag))
        .min()?;
    Some(plain(&rest[..end]))
}

/// `M/D/YYYY` → `YYYY-MM-DD`.
fn us_date(text: &str) -> Option<String> {
    let mut parts = text.trim().split('/');
    let (m, d, y) = (parts.next()?, parts.next()?, parts.next()?);
    let (m, d, y): (u32, u32, u32) = (m.parse().ok()?, d.parse().ok()?, y.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d) && (1990..=2100).contains(&y))
        .then(|| format!("{y:04}-{m:02}-{d:02}"))
}

pub fn read_search(html: &str) -> Result<SearchPage, ReadError> {
    if !html.contains("Microsoft Update Catalog") {
        return Err(ReadError::UnexpectedFormat("not a Catalog page"));
    }
    let Some(count) = cell(html, "ctl00_catalogBody_searchDuration") else {
        // The Catalog's own "no results" page has the results area but no counter.
        if html.contains("id=\"ctl00_catalogBody_noResultText\"") {
            return Ok(SearchPage {
                rows: Vec::new(),
                total: 0,
            });
        }
        return Err(ReadError::UnexpectedFormat("results without a counter"));
    };
    let total = count
        .split(" of ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .ok_or(ReadError::UnexpectedFormat("unreadable counter"))?;
    let mut rows = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<tr id=\"") {
        rest = &rest[at + 8..];
        let Some(end) = rest.find('"') else { break };
        let row_id = &rest[..end];
        let Some((update_id, index)) = row_id.split_once("_R") else {
            continue;
        };
        if !is_guid(update_id) || index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let c = |n: u8| cell(html, &format!("{update_id}_C{n}_R{index}"));
        let title = c(1).ok_or(ReadError::UnexpectedFormat("row without title"))?;
        rows.push(Row {
            update_id: update_id.to_ascii_lowercase(),
            title,
            products: c(2).unwrap_or_default(),
            classification: c(3).unwrap_or_default(),
            date: c(4).as_deref().and_then(us_date),
            version: c(5).as_deref().and_then(parse_version),
            size: cell(html, &format!("{update_id}_originalSize")).and_then(|s| s.parse().ok()),
        });
    }
    if total > 0 && rows.is_empty() {
        return Err(ReadError::UnexpectedFormat("counter without rows"));
    }
    Ok(SearchPage { rows, total })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub class: Option<String>,
    /// Who published it at Microsoft (often a PC maker): shown, never used to match.
    pub company: Option<String>,
    pub version: Option<Version>,
    /// Lower case, as the Catalog writes them.
    pub hardware_ids: Vec<String>,
}

pub fn read_details(html: &str) -> Result<Details, ReadError> {
    let start = html
        .find("id=\"driverhwIDs\"")
        .ok_or(ReadError::UnexpectedFormat("details without hardware IDs"))?;
    let end = html[start..]
        .find("id=\"driverupdateIDs\"")
        .map_or(html.len(), |e| start + e);
    let mut hardware_ids = Vec::new();
    let mut rest = &html[start..end];
    while let Some(open) = rest.find("<div") {
        rest = &rest[open..];
        let Some(gt) = rest.find('>') else { break };
        rest = &rest[gt + 1..];
        let close = rest.find('<').unwrap_or(rest.len());
        let id = plain(&rest[..close]).to_lowercase();
        if !id.is_empty() && id.contains('\\') && !id.contains(' ') {
            hardware_ids.push(id);
        }
        rest = &rest[close..];
    }
    let field =
        |name: &str| cell(html, &format!("ScopedViewHandler_{name}")).filter(|v| !v.is_empty());
    Ok(Details {
        provider: field("driverProvider"),
        model: field("driverModel"),
        class: field("driverClass"),
        company: field("company"),
        version: field("version").as_deref().and_then(parse_version),
        hardware_ids,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub url: HttpsUrl,
    /// Hash given by the source (R-SRC-02), lower-case hex.
    pub sha256: String,
}

/// The one file of a driver update. The link must be on the Catalog's file host with the exact
/// path of a driver `.cab` whose name ends with its SHA-1; anything else is refused (R-SRC-01).
pub fn read_download_dialog(text: &str) -> Result<Download, ReadError> {
    let value = |key: &str| -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("downloadInformation[0].files[")?;
                let (_, rest) = rest.split_once(']')?;
                let rest = rest.strip_prefix(key)?.trim_start();
                let rest = rest.strip_prefix('=')?.trim();
                let rest = rest.strip_prefix('\'')?;
                Some(rest.split('\'').next()?.to_owned())
            })
            .collect()
    };
    let urls = value(".url");
    let hashes = value(".sha256");
    let ([url], [sha256]) = (urls.as_slice(), hashes.as_slice()) else {
        return Err(ReadError::UnexpectedFormat("not exactly one file"));
    };
    if !driver_cab_link(url) {
        return Err(ReadError::UnexpectedFormat(
            "link not in the expected format",
        ));
    }
    let url = HttpsUrl::parse(url).map_err(|_| ReadError::UnexpectedFormat("link refused"))?;
    let sha256 = base64_decode(sha256)
        .filter(|b| b.len() == 32)
        .map(|b| hex(&b))
        .ok_or(ReadError::UnexpectedFormat("unreadable SHA-256"))?;
    Ok(Download { url, sha256 })
}

/// The installer's own look-up of one update (R-SEC-05): the link and hash come from the Catalog
/// now, never from the plan. One try, 2 MB at most, redirects only to allowed hosts.
pub fn look_up_download(http: &dyn Http, update_id: &str) -> Result<Download, String> {
    let form = download_dialog_form(update_id).ok_or("the update ID is not a GUID")?;
    let url = download_dialog_url().map_err(|e| e.to_string())?;
    let body =
        fetch(http, Request::post_form(url, form, MAX_PAGE_BYTES)).map_err(|e| e.to_string())?;
    read_download_dialog(&String::from_utf8_lossy(&body)).map_err(|e| e.to_string())
}

/// R-SRC-02: the file's SHA-256 equals the one the Catalog gave.
pub fn hash_matches(bytes: &[u8], expected_hex: &str) -> bool {
    crate::install::plan::sha256_hex(bytes) == expected_hex.trim().to_lowercase()
}

/// `https://catalog.s.download.windowsupdate.com/<x>/msdownload/update/driver/drvs/YYYY/MM/<guid>_<sha1>.cab`
pub fn driver_cab_link(url: &str) -> bool {
    let Some(path) = url.strip_prefix(FILES) else {
        return false;
    };
    let parts: Vec<&str> = path.split('/').collect();
    let [
        "",
        letter,
        "msdownload",
        "update",
        "driver",
        "drvs",
        year,
        month,
        file,
    ] = parts.as_slice()
    else {
        return false;
    };
    let Some(name) = file.strip_suffix(".cab") else {
        return false;
    };
    let Some((guid, sha1)) = name.split_once('_') else {
        return false;
    };
    letter.len() == 1
        && letter.bytes().all(|b| b.is_ascii_lowercase())
        && year.len() == 4
        && year.bytes().all(|b| b.is_ascii_digit())
        && month.len() == 2
        && month.bytes().all(|b| b.is_ascii_digit())
        && is_guid(guid)
        && sha1.len() == 40
        && sha1.bytes().all(|b| b.is_ascii_hexdigit())
}
