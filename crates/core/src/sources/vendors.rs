//! Asks the fragile sources about this PC, after the scan (Spec - Sources constructeurs, 4):
//! only for devices with a manufacturer driver (`oem#.inf`) or an installed game component,
//! one request at a time, through [`Guard`] (R-SRC-11). Every failure is "unknown", never "up
//! to date" (R-VRAI-04).

use std::collections::BTreeMap;

use super::SourceId;
use super::guard::{Guard, Permit, RETRY_SECONDS};
use super::http::{FetchError, Http, MAX_PAGE_BYTES, Request, fetch};
use super::{asus, intel, ms_catalog, nvidia, oracle};
use crate::decide::{Version, parse_version};
use crate::install::plan::is_oem_inf;
use crate::model::{BiosCheck, Board, Device, HintKind, VendorHint, VendorOffer, VendorResult};

/// ADR-0033: `www.intel.com` answered 403 to Drivn on 2026-11-01. Without a workaround
/// (R-SRC-11), the scan does not ask Intel; Intel DSA is offered instead. The source stays coded
/// and tested ([`check_intel`]) in case Intel accepts Drivn later.
pub const ASK_INTEL: bool = false;

pub struct VendorContext<'a> {
    pub now: i64,
    /// `DisplayVersion` of Windows (`26H2`), for the Catalog search.
    pub os_release: Option<&'a str>,
    /// `DisplayVersion` of each installed Java 8 (`8.0.5110.9`).
    pub java_versions: &'a [String],
    /// The motherboard, read in the registry (R-SRC-07, ADR-0032).
    pub board: Option<&'a Board>,
}

enum Got {
    Body(String),
    /// The source must not be called again in this scan.
    Stop(String),
}

fn get(
    http: &dyn Http,
    guard: &mut Guard,
    source: SourceId,
    key: &str,
    request: Request,
    now: i64,
) -> Got {
    match guard.permit(source, key, now) {
        Permit::Cached(body) => Got::Body(body),
        Permit::Blocked { reason, .. } => Got::Stop(format!("blocked for 24 h: {reason}")),
        Permit::LimitReached => Got::Stop("limit of 60 requests reached for this scan".to_owned()),
        Permit::Go => match fetch(http, request) {
            Ok(bytes) => {
                let body = String::from_utf8_lossy(&bytes).into_owned();
                guard.keep(source, key, now, body.clone());
                Got::Body(body)
            }
            Err(error @ (FetchError::Blocked(_) | FetchError::Refused(_))) => {
                guard.block(source, now, &error.to_string());
                Got::Stop(error.to_string())
            }
            Err(error) => Got::Stop(error.to_string()),
        },
    }
}

/// Asks and reads the answer. R-SRC-11 (ADR-0031): an unreadable page (CAPTCHA, changed site,
/// passing page) is asked again once, the same, after 5 s; unreadable again = no call to this
/// source for 24 h, and the page is kept for analysis.
fn ask<T>(
    http: &dyn Http,
    guard: &mut Guard,
    source: SourceId,
    key: &str,
    request: &Request,
    now: i64,
    read: impl Fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    let mut asked_again = false;
    loop {
        let body = match get(http, guard, source, key, request.clone(), now) {
            Got::Body(body) => body,
            Got::Stop(why) => return Err(why),
        };
        match read(&body) {
            Ok(value) => return Ok(value),
            Err(what) => {
                guard.forget(source, key);
                if !asked_again {
                    asked_again = true;
                    http.pause(RETRY_SECONDS);
                    continue;
                }
                guard.block(source, now, &what);
                guard.keep_rejected(source, &body);
                return Err(format!("unexpected answer: {what}"));
            }
        }
    }
}

fn installed_version(device: &Device) -> Option<Version> {
    device
        .driver
        .as_ref()?
        .version
        .as_deref()
        .and_then(parse_version)
}

fn manufacturer_driver(device: &Device) -> bool {
    device.present
        && device
            .driver
            .as_ref()
            .and_then(|d| d.inf_path.as_deref())
            .is_some_and(is_oem_inf)
}

fn all_ids(device: &Device) -> Vec<String> {
    device
        .hardware_ids
        .iter()
        .chain(&device.compatible_ids)
        .cloned()
        .collect()
}

pub fn check_vendors(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
) -> Vec<VendorResult> {
    check_vendors_with_progress(devices, http, guard, context, &mut |_, _, _| {})
}

/// What is being asked, for the scan screen (Spec - Scan complet): the source and the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asking<'a> {
    pub source: SourceId,
    /// The device (or component) the question is about.
    pub about: &'a str,
}

fn name(device: &Device) -> &str {
    device
        .description
        .as_deref()
        .unwrap_or(device.instance_id.as_str())
}

/// Same as [`check_vendors`], and calls `progress(done, total, asking)` before each question: one
/// search per Catalog term, one look-up per NVIDIA card, Oracle once; `None` at the end. Measured,
/// never guessed (R-VRAI-01).
pub fn check_vendors_with_progress(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
    progress: &mut dyn FnMut(usize, usize, Option<Asking<'_>>),
) -> Vec<VendorResult> {
    let java = usize::from(asks_oracle(context));
    let board = asus_board(context);
    let total = catalog_terms(devices).len()
        + nvidia_cards(devices).len()
        + if ASK_INTEL {
            intel_pages(devices).len()
        } else {
            0
        }
        + 2 * usize::from(board.is_some())
        + java;
    let mut done = 0;
    progress(0, total, None);
    let mut tick = |asking: Asking<'_>| {
        progress(done.min(total), total, Some(asking));
        done += 1;
    };
    let catalog = ms_catalog_check(devices, http, guard, context, &mut tick);
    let nvidia = nvidia_check(devices, http, guard, context, &mut tick);
    let intel = ASK_INTEL.then(|| intel_check(devices, http, guard, context, &mut tick));
    let asus = asus_check(devices, http, guard, context, &mut tick);
    if java == 1 {
        tick(Asking {
            source: SourceId::Oracle,
            about: "Java 8",
        });
    }
    let oracle = oracle_check(http, guard, context);
    progress(total, total, None);
    [Some(catalog), Some(nvidia), intel, Some(asus), Some(oracle)]
        .into_iter()
        .flatten()
        .collect()
}

/// One search per term: devices that share it (twins) are answered together.
fn catalog_terms(devices: &[Device]) -> BTreeMap<String, Vec<&Device>> {
    let mut by_term: BTreeMap<String, Vec<&Device>> = BTreeMap::new();
    for device in devices.iter().filter(|d| manufacturer_driver(d)) {
        if let Some(term) = device
            .hardware_ids
            .first()
            .and_then(|id| ms_catalog::search_term(id))
        {
            by_term.entry(term).or_default().push(device);
        }
    }
    by_term
}

fn nvidia_cards(devices: &[Device]) -> Vec<&Device> {
    devices
        .iter()
        .filter(|d| manufacturer_driver(d))
        .filter(|d| {
            d.hardware_ids
                .iter()
                .any(|id| id.to_ascii_uppercase().starts_with("PCI\\VEN_10DE&"))
        })
        .collect()
}

/// R-SRC-11: Oracle only for a Java 8 installed on this PC.
fn asks_oracle(context: &VendorContext<'_>) -> bool {
    context
        .java_versions
        .iter()
        .any(|v| oracle::installed_java8_update(v).is_some())
}

fn empty(source: SourceId) -> VendorResult {
    VendorResult {
        source,
        error: None,
        requests: 0,
        offers: Vec::new(),
        unchecked: Vec::new(),
        hints: Vec::new(),
        java8_latest_update: None,
        bios: None,
    }
}

fn ms_catalog_check(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
    tick: &mut dyn FnMut(Asking<'_>),
) -> VendorResult {
    let source = SourceId::MsCatalog;
    let mut result = empty(source);
    let by_term = catalog_terms(devices);
    let releases = ms_catalog::releases_from(context.os_release);
    let mut empty: BTreeMap<&str, usize> = BTreeMap::new();
    let mut alive: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut stopped: Option<String> = None;
    for (term, group) in &by_term {
        tick(Asking {
            source,
            about: name(group[0]),
        });
        if stopped.is_some() {
            result
                .unchecked
                .extend(group.iter().map(|d| d.instance_id.clone()));
            continue;
        }
        let mut page = None;
        for release in &releases {
            // R-SRC-11: a release with nothing for 2 devices and nothing for any other device
            // is not asked again in this scan (26H2 had no driver at all on 2026-11-01).
            if !alive.contains(release) && empty.get(release).copied().unwrap_or(0) >= 2 {
                continue;
            }
            let key = format!("search|{term}|{release}");
            let Ok(url) = ms_catalog::search_url(term, release) else {
                continue;
            };
            let request = Request::get(url, MAX_PAGE_BYTES);
            match ask(http, guard, source, &key, &request, context.now, |body| {
                ms_catalog::read_search(body).map_err(|e| e.to_string())
            }) {
                Err(why) => {
                    stopped = Some(why);
                    break;
                }
                Ok(found) if found.total == 0 => {
                    *empty.entry(release).or_insert(0) += 1;
                }
                Ok(found) => {
                    alive.insert(release);
                    page = Some(found);
                    break;
                }
            }
        }
        if stopped.is_some() {
            result
                .unchecked
                .extend(group.iter().map(|d| d.instance_id.clone()));
            continue;
        }
        let Some(page) = page else { continue };
        for device in group {
            if stopped.is_some() {
                result.unchecked.push(device.instance_id.clone());
                continue;
            }
            match catalog_offer(device, &page, http, guard, context.now) {
                Ok(Some(offer)) => result.offers.push(offer),
                Ok(None) if page.capped() => result.unchecked.push(device.instance_id.clone()),
                Ok(None) => {}
                Err(why) => {
                    stopped = Some(why);
                    result.unchecked.push(device.instance_id.clone());
                }
            }
        }
    }
    result.error = stopped;
    result.requests = guard.requests(source);
    result
}

/// The newest row newer than the installed driver whose details list an ID of the device.
/// At most 2 rows are opened (2 requests).
fn catalog_offer(
    device: &Device,
    page: &ms_catalog::SearchPage,
    http: &dyn Http,
    guard: &mut Guard,
    now: i64,
) -> Result<Option<VendorOffer>, String> {
    let source = SourceId::MsCatalog;
    let Some(current) = installed_version(device) else {
        return Ok(None);
    };
    let mut rows: Vec<&ms_catalog::Row> = page
        .rows
        .iter()
        .filter(|r| r.version.is_some_and(|v| v > current))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.version));
    rows.dedup_by(|a, b| a.version == b.version);
    let ids = all_ids(device);
    for row in rows.into_iter().take(2) {
        let key = format!("details|{}", row.update_id);
        let Ok(url) = ms_catalog::details_url(&row.update_id) else {
            continue;
        };
        let request = Request::get(url, MAX_PAGE_BYTES);
        let details = ask(http, guard, source, &key, &request, now, |body| {
            ms_catalog::read_details(body).map_err(|e| e.to_string())
        })?;
        // R-MAT-01: an ID of the device equal to an ID the update lists, case ignored.
        let Some(matched) = ids
            .iter()
            .find(|id| details.hardware_ids.contains(&id.to_lowercase()))
        else {
            continue;
        };
        return Ok(Some(VendorOffer {
            device_instance_id: device.instance_id.clone(),
            hardware_id: matched.clone(),
            title: Some(row.title.clone()),
            version: row.version.map(|v| v.map(|p| p.to_string()).join(".")),
            vendor_version: None,
            announced_newer: false,
            date: row.date.clone(),
            provider: details.provider,
            publisher: details.company,
            size_bytes: row.size,
            size_text: None,
            update_id: Some(row.update_id.clone()),
            installable: true,
            download_url: None,
            sha512: None,
        }));
    }
    Ok(None)
}

fn nvidia_check(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
    tick: &mut dyn FnMut(Asking<'_>),
) -> VendorResult {
    let source = SourceId::Nvidia;
    let mut result = empty(source);
    let cards = nvidia_cards(devices);
    let audio: Vec<&Device> = devices
        .iter()
        .filter(|d| manufacturer_driver(d))
        .filter(|d| {
            d.hardware_ids.iter().any(|id| {
                id.to_ascii_uppercase()
                    .starts_with("HDAUDIO\\FUNC_01&VEN_10DE&")
            })
        })
        .collect();
    let mut newest: Option<nvidia::Release> = None;
    for card in &cards {
        tick(Asking {
            source,
            about: name(card),
        });
        let Some(pfid) = nvidia::product_id(&card.hardware_ids) else {
            result.unchecked.push(card.instance_id.clone());
            continue;
        };
        let key = format!("lookup|{pfid}");
        let Ok(url) = nvidia::lookup_url(pfid) else {
            continue;
        };
        let request = Request::get(url, MAX_PAGE_BYTES);
        let release = match ask(http, guard, source, &key, &request, context.now, |body| {
            nvidia::read_lookup(body).map_err(str::to_owned)
        }) {
            Ok(Some(release)) => release,
            Ok(None) => {
                result.unchecked.push(card.instance_id.clone());
                continue;
            }
            Err(why) => {
                result.error = Some(why);
                result.unchecked.push(card.instance_id.clone());
                break;
            }
        };
        let installed = card
            .driver
            .as_ref()
            .and_then(|d| d.version.as_deref())
            .and_then(nvidia::nvidia_version);
        match installed {
            Some(current) if release.version > current => {
                result.offers.push(VendorOffer {
                    device_instance_id: card.instance_id.clone(),
                    hardware_id: card.hardware_ids.first().cloned().unwrap_or_default(),
                    title: Some(release.name.clone()),
                    version: None,
                    vendor_version: Some(release.version_text.clone()),
                    announced_newer: true,
                    date: release.date.clone(),
                    provider: Some("NVIDIA".to_owned()),
                    publisher: None,
                    size_bytes: None,
                    size_text: Some(release.size_text.clone()),
                    update_id: None,
                    // Spec 4.2: never installed from its INF by Drivn; NVIDIA's installer does it (ADR-0030).
                    installable: false,
                    download_url: Some(release.download_url.to_string()),
                    sha512: None,
                });
            }
            Some(_) => {}
            None => result.unchecked.push(card.instance_id.clone()),
        }
        if newest.as_ref().is_none_or(|n| release.version > n.version) {
            newest = Some(release);
        }
    }
    // Spec - Tout mettre a jour (ADR-0034): NVIDIA's package also updates its HD Audio driver, so
    // HD Audio gets the card's offer and the same installer. Without a newer package, a hint.
    let package = result.offers.first().cloned();
    for device in &audio {
        if let Some(card) = &package {
            result.offers.push(VendorOffer {
                device_instance_id: device.instance_id.clone(),
                hardware_id: device.hardware_ids.first().cloned().unwrap_or_default(),
                title: Some(format!(
                    "{} (paquet NVIDIA, avec la carte graphique)",
                    card.vendor_version.clone().unwrap_or_default()
                )),
                ..card.clone()
            });
            continue;
        }
        result.hints.push(VendorHint {
            device_instance_id: device.instance_id.clone(),
            kind: if newest.is_some() {
                HintKind::PackageMayContainNewer
            } else {
                HintKind::VendorMayHaveNewer
            },
            text: newest.as_ref().map(|n| {
                format!(
                    "{} ({}, {})",
                    n.version_text,
                    n.date.clone().unwrap_or_default(),
                    n.size_text
                )
            }),
        });
    }
    result.requests = guard.requests(source);
    result
}

/// Intel pages to read, each with the devices of this PC it covers (ADR-0033): devices with a
/// manufacturer driver and an ID equal to one of Drivn's table (R-MAT-01).
fn intel_pages(
    devices: &[Device],
) -> BTreeMap<&'static str, (&'static intel::Entry, Vec<&Device>)> {
    let mut pages: BTreeMap<&'static str, (&'static intel::Entry, Vec<&Device>)> = BTreeMap::new();
    for device in devices.iter().filter(|d| manufacturer_driver(d)) {
        if let Some(entry) = intel::entry_for(&all_ids(device)) {
            pages
                .entry(entry.page)
                .or_insert((entry, Vec::new()))
                .1
                .push(device);
        }
    }
    pages
}

/// Intel alone, whatever [`ASK_INTEL`] says: for the tests of this source.
pub fn check_intel(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
) -> VendorResult {
    intel_check(devices, http, guard, context, &mut |_| {})
}

/// One request per page. The page must still list the product of the table; the version is
/// announced (R-VER-03) and Intel's installer installs it (ADR-0033).
fn intel_check(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
    tick: &mut dyn FnMut(Asking<'_>),
) -> VendorResult {
    let source = SourceId::Intel;
    let mut result = empty(source);
    for (key, (entry, group)) in intel_pages(devices) {
        tick(Asking {
            source,
            about: name(group[0]),
        });
        let unchecked = |result: &mut VendorResult| {
            result
                .unchecked
                .extend(group.iter().map(|d| d.instance_id.clone()));
        };
        if result.error.is_some() {
            unchecked(&mut result);
            continue;
        }
        let page = intel::page_url(entry)
            .map_err(|e| e.to_string())
            .and_then(|url| {
                ask(
                    http,
                    guard,
                    source,
                    key,
                    &Request::get(url, MAX_PAGE_BYTES),
                    context.now,
                    |body| intel::read_page(body).map_err(str::to_owned),
                )
            });
        let page = match page {
            Ok(page) => page,
            Err(why) => {
                result.error = Some(why);
                unchecked(&mut result);
                continue;
            }
        };
        // The table and Intel must agree on the product, or nothing is said (R-VRAI-04).
        let Some(latest) =
            intel::release(&page.version).filter(|_| page.products.contains(&entry.sku))
        else {
            unchecked(&mut result);
            continue;
        };
        for device in group {
            let installed = device
                .driver
                .as_ref()
                .and_then(|d| d.version.as_deref())
                .and_then(intel::release);
            match installed {
                Some(current) if latest > current => result.offers.push(VendorOffer {
                    device_instance_id: device.instance_id.clone(),
                    hardware_id: entry.hardware_id.to_owned(),
                    title: Some(format!("Intel {} {}", entry.family, page.version)),
                    version: None,
                    vendor_version: Some(page.version.clone()),
                    announced_newer: true,
                    date: page.date.clone(),
                    provider: Some("Intel".to_owned()),
                    publisher: None,
                    size_bytes: None,
                    size_text: page.size_text.clone(),
                    update_id: None,
                    // ADR-0033: never installed from its INF by Drivn; Intel's installer does it.
                    installable: false,
                    download_url: Some(page.package.to_string()),
                    sha512: Some(page.sha512.clone()),
                }),
                Some(_) => {}
                None => result.unchecked.push(device.instance_id.clone()),
            }
        }
    }
    result.requests = guard.requests(source);
    result
}

/// R-SRC-07 (ADR-0032): the motherboard, with its ASUS model key, only when it is made by ASUS.
fn asus_board<'a>(context: &VendorContext<'a>) -> Option<(&'a Board, String)> {
    let board = context.board.filter(|b| asus::is_asus(b))?;
    let slug = asus::slug(board.product.as_deref()?)?;
    Some((board, slug))
}

/// ASUS's BIOS list (shown, never downloaded) and driver list (hints tied by family and name,
/// never offers: the list has no hardware ID, R-MAT-01). Two requests, kept 24 h (R-SRC-11).
fn asus_check(
    devices: &[Device],
    http: &dyn Http,
    guard: &mut Guard,
    context: &VendorContext<'_>,
    tick: &mut dyn FnMut(Asking<'_>),
) -> VendorResult {
    let source = SourceId::Asus;
    let mut result = empty(source);
    let Some((board, slug)) = asus_board(context) else {
        return result;
    };
    let product = board.product.clone().unwrap_or_default();
    let read = |body: &str| asus::read_list(body).map_err(str::to_owned);
    tick(Asking {
        source,
        about: &product,
    });
    let bios = asus::bios_url(&slug)
        .map_err(|e| e.to_string())
        .and_then(|url| {
            ask(
                http,
                guard,
                source,
                "bios",
                &Request::get(url, MAX_PAGE_BYTES),
                context.now,
                read,
            )
        });
    match bios {
        Ok(Some(files)) => {
            if let Some(latest) = asus::newest_bios(&files) {
                result.bios = Some(BiosCheck {
                    board: product.clone(),
                    installed: board.bios_version.clone(),
                    installed_date: board.bios_date.as_deref().and_then(asus::firmware_date),
                    latest: latest.version.clone(),
                    latest_date: latest.date.clone(),
                    newer: board
                        .bios_version
                        .as_deref()
                        .and_then(|v| asus::bios_is_newer(v, &latest.version)),
                });
            }
        }
        Ok(None) => result.error = Some(format!("model unknown at ASUS: {product}")),
        Err(why) => {
            result.error = Some(why);
            result.requests = guard.requests(source);
            return result;
        }
    }
    tick(Asking {
        source,
        about: &product,
    });
    let drivers = asus::drivers_url(&slug)
        .map_err(|e| e.to_string())
        .and_then(|url| {
            ask(
                http,
                guard,
                source,
                "drivers",
                &Request::get(url, MAX_PAGE_BYTES),
                context.now,
                read,
            )
        });
    match drivers {
        Ok(Some(files)) => {
            for device in devices.iter().filter(|d| d.present) {
                let Some(installed) = installed_version(device) else {
                    continue;
                };
                if let Some(file) = asus::hint_for(device, &files, installed) {
                    result.hints.push(VendorHint {
                        device_instance_id: device.instance_id.clone(),
                        kind: HintKind::PublisherListsNewer,
                        text: Some(match &file.date {
                            Some(date) => format!("{} · {date}", file.title),
                            None => file.title.clone(),
                        }),
                    });
                }
            }
        }
        Ok(None) => {}
        Err(why) => result.error = Some(why),
    }
    result.requests = guard.requests(source);
    result
}

fn oracle_check(http: &dyn Http, guard: &mut Guard, context: &VendorContext<'_>) -> VendorResult {
    let source = SourceId::Oracle;
    let mut result = empty(source);
    // R-SRC-11: only for a component installed on this PC.
    if !asks_oracle(context) {
        return result;
    }
    let key = "baseline";
    let Ok(url) = oracle::baseline_url() else {
        return result;
    };
    let request = Request::get(url, 64 * 1024);
    match ask(http, guard, source, key, &request, context.now, |body| {
        oracle::latest_java8_update(body).ok_or_else(|| "no Java 8 line".to_owned())
    }) {
        Ok(update) => result.java8_latest_update = Some(update),
        Err(why) => result.error = Some(why),
    }
    result.requests = guard.requests(source);
    result
}
