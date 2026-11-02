//! Decision engine (docs/specs/Spec - Moteur de decision.md). For each device it gives one honest
//! state and keeps the evidence behind it. Pure: it only reads a snapshot, never Windows or the network.

use serde::Serialize;

use crate::model::{Device, DriverOffer, Snapshot, VendorHint, VendorResult};
use crate::sources::SourceId;

/// DriverVer version `w.x.y.z`; a missing part is 0.
pub type Version = [u16; 4];

/// 1 to 4 numbers from 0 to 65535 separated by dots, nothing else.
pub fn parse_version(text: &str) -> Option<Version> {
    let parts: Vec<&str> = text.trim().split('.').collect();
    if parts.len() > 4 {
        return None;
    }
    let mut version = [0; 4];
    for (slot, part) in version.iter_mut().zip(&parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(version)
}

/// Windows Update gives no version: it is read between the parentheses that end the title (ADR-0012).
pub fn title_version(title: &str) -> Option<Version> {
    let inner = title.trim_end().strip_suffix(')')?;
    let open = inner.rfind('(')?;
    parse_version(&inner[open + 1..])
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn norm(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Why a proposed driver is not ticked by default (R-MAT-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// The offer matches an ID further down the device list than the installed driver's.
    LessPrecise,
    OtherProvider,
    CannotJudge,
    NoInstalledDriver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    UnknownVersion,
    UnknownDate,
    NotNewer,
    /// Higher version but older date.
    Contradictory,
    InstalledUnreadable,
    /// Valid, but another candidate was chosen.
    NotChosen,
}

impl Rejection {
    /// Rejections that leave a doubt: without a retained candidate, the device is "unknown", not "up to date".
    fn doubtful(self) -> bool {
        matches!(
            self,
            Self::UnknownVersion
                | Self::UnknownDate
                | Self::Contradictory
                | Self::InstalledUnreadable
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    NoSearch,
    SearchFailed,
    Absent,
    DoubtfulOffer,
    InstalledUnreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    Update,
    UpdateRisky {
        risk: Risk,
    },
    UpToDate,
    /// R-VER-03: a fragile source announces a newer driver; confirmed only at install time.
    Announced {
        risk: Option<Risk>,
    },
    NoDriver,
    Unknown {
        reason: UnknownReason,
    },
}

/// What an offer said, kept as evidence (R-VRAI-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferEvidence {
    pub offer_index: usize,
    pub hardware_id: String,
    pub title: Option<String>,
    pub version: Option<String>,
    pub date: Option<String>,
    pub provider: Option<String>,
    /// WUA identity of the offer, carried to the install plan (Spec - Installation).
    pub update_id: Option<String>,
    pub revision: Option<i32>,
    /// `None`: Windows Update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceId>,
    /// The publisher's own numbering (NVIDIA `621.01`), display only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_text: Option<String>,
    /// `false` for an offer Drivn only shows (NVIDIA graphics).
    pub installable: bool,
    /// The exact place it comes from, shown before the install: Windows Update's download link,
    /// the Catalog page of this update, or the publisher's package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    #[serde(flatten)]
    pub offer: OfferEvidence,
    pub risk: Option<Risk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejected {
    #[serde(flatten)]
    pub offer: OfferEvidence,
    pub reason: Rejection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceDecision {
    pub instance_id: String,
    #[serde(flatten)]
    pub state: State,
    pub chosen: Option<Candidate>,
    pub rejected: Vec<Rejected>,
    /// Newer drivers announced by other sources while another offer is chosen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub announced: Vec<Candidate>,
    /// Sources that should have checked this device and could not (R-VRAI-04): the device is
    /// then "up to date according to Windows Update", never "up to date".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unchecked_by: Vec<SourceId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<VendorHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unmatched {
    NoHardwareId,
    NoDevice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmatchedOffer {
    pub offer_index: usize,
    pub title: Option<String>,
    pub hardware_id: Option<String>,
    pub reason: Unmatched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// One per device, in snapshot order.
    pub decisions: Vec<DeviceDecision>,
    pub unmatched_offers: Vec<UnmatchedOffer>,
}

/// All IDs of a device, Hardware IDs first then Compatible IDs, in Windows order.
fn device_ids(device: &Device) -> Vec<String> {
    device
        .hardware_ids
        .iter()
        .chain(&device.compatible_ids)
        .map(|id| norm(id))
        .collect()
}

pub fn decide(snapshot: &Snapshot) -> Report {
    let search = match &snapshot.windows_update {
        None => Err(UnknownReason::NoSearch),
        Some(result) if result.error.is_some() => Err(UnknownReason::SearchFailed),
        Some(result) => Ok(result.offers.as_slice()),
    };
    let offers = search.unwrap_or(&[]);
    let present: Vec<Vec<String>> = snapshot
        .devices
        .iter()
        .filter(|d| d.present)
        .map(device_ids)
        .collect();
    let unmatched_offers = offers
        .iter()
        .enumerate()
        .filter_map(|(offer_index, offer)| {
            let reason = match &offer.hardware_id {
                None => Unmatched::NoHardwareId,
                Some(id) if !present.iter().any(|ids| ids.contains(&norm(id))) => {
                    Unmatched::NoDevice
                }
                Some(_) => return None,
            };
            Some(UnmatchedOffer {
                offer_index,
                title: offer.title.clone(),
                hardware_id: offer.hardware_id.clone(),
                reason,
            })
        })
        .collect();
    let decisions = snapshot
        .devices
        .iter()
        .map(|device| {
            let mut decision = decide_device(device, search);
            if device.present {
                apply_vendors(&mut decision, device, &snapshot.vendors);
            }
            decision
        })
        .collect();
    Report {
        decisions,
        unmatched_offers,
    }
}

fn decide_device(device: &Device, search: Result<&[DriverOffer], UnknownReason>) -> DeviceDecision {
    let mut out = DeviceDecision {
        instance_id: device.instance_id.clone(),
        state: State::Unknown {
            reason: UnknownReason::Absent,
        },
        chosen: None,
        rejected: Vec::new(),
        announced: Vec::new(),
        unchecked_by: Vec::new(),
        hints: Vec::new(),
    };
    if !device.present {
        return out;
    }
    let offers = match search {
        Ok(offers) => offers,
        Err(reason) => {
            out.state = State::Unknown { reason };
            return out;
        }
    };

    let ids = device_ids(device);
    let installed = device.driver.as_ref();
    let mut retained: Vec<(Version, Candidate)> = Vec::new();
    for (offer_index, offer) in offers.iter().enumerate() {
        // R-MAT-01: only an ID equal (ignoring case) to one of the device's IDs counts.
        let Some(hardware_id) = &offer.hardware_id else {
            continue;
        };
        let Some(position) = ids.iter().position(|id| *id == norm(hardware_id)) else {
            continue;
        };
        let version = offer.title.as_deref().and_then(title_version);
        let evidence = OfferEvidence {
            offer_index,
            hardware_id: hardware_id.clone(),
            title: offer.title.clone(),
            version: version.map(|v| v.map(|p| p.to_string()).join(".")),
            date: offer.driver_date.clone(),
            provider: offer.provider.clone(),
            update_id: offer.update_id.clone(),
            revision: offer.revision,
            source: None,
            vendor_version: None,
            publisher: None,
            size_bytes: None,
            size_text: None,
            installable: true,
            source_url: offer.download_url.clone(),
        };
        let mut reject = |reason| {
            out.rejected.push(Rejected {
                offer: evidence.clone(),
                reason,
            })
        };
        let Some(version) = version else {
            reject(Rejection::UnknownVersion);
            continue;
        };
        let Some(date) = offer.driver_date.as_deref().filter(|d| is_date(d)) else {
            reject(Rejection::UnknownDate);
            continue;
        };
        let risk = match installed {
            None => Some(Risk::NoInstalledDriver),
            Some(driver) => {
                let current = driver.version.as_deref().and_then(parse_version);
                let current_date = driver.date.as_deref().filter(|d| is_date(d));
                let (Some(current), Some(current_date)) = (current, current_date) else {
                    reject(Rejection::InstalledUnreadable);
                    continue;
                };
                // R-VER-01: strictly newer version and a date that is not older, never a downgrade.
                if version <= current {
                    reject(Rejection::NotNewer);
                    continue;
                }
                if date < current_date {
                    reject(Rejection::Contradictory);
                    continue;
                }
                judge_risk(driver, &ids, position, offer.provider.as_deref())
            }
        };
        retained.push((
            version,
            Candidate {
                offer: evidence,
                risk,
            },
        ));
    }

    if !retained.is_empty() {
        // A candidate without risk first, then the highest version.
        retained.sort_by(|a, b| {
            a.1.risk
                .is_some()
                .cmp(&b.1.risk.is_some())
                .then(b.0.cmp(&a.0))
        });
        let mut rest = retained.into_iter().map(|(_, c)| c);
        if let Some(chosen) = rest.next() {
            out.state = match chosen.risk {
                None => State::Update,
                Some(risk) => State::UpdateRisky { risk },
            };
            out.chosen = Some(chosen);
        }
        out.rejected.extend(rest.map(|c| Rejected {
            offer: c.offer,
            reason: Rejection::NotChosen,
        }));
        return out;
    }
    // R-VER-02 and R-VRAI-04: no retained candidate means no "update", and a doubt is never "up to date".
    out.state = if out.rejected.iter().any(|r| r.reason.doubtful()) {
        State::Unknown {
            reason: UnknownReason::DoubtfulOffer,
        }
    } else {
        match installed {
            None => State::NoDriver,
            Some(d)
                if d.version.as_deref().and_then(parse_version).is_none()
                    || !d.date.as_deref().is_some_and(is_date) =>
            {
                State::Unknown {
                    reason: UnknownReason::InstalledUnreadable,
                }
            }
            Some(_) => State::UpToDate,
        }
    };
    out
}

/// R-MAT-02: never silently replace a more precise (often manufacturer) driver, or a driver of
/// another provider. `position`: index of the offer's ID in the device's ID list.
fn judge_risk(
    driver: &crate::model::InstalledDriver,
    ids: &[String],
    position: usize,
    offered_provider: Option<&str>,
) -> Option<Risk> {
    let matched = driver
        .matching_device_id
        .as_deref()
        .and_then(|m| ids.iter().position(|id| *id == norm(m)));
    let providers = offered_provider
        .map(norm)
        .zip(driver.provider.as_deref().map(norm));
    match (matched, providers) {
        (Some(matched), Some((offered, current))) => {
            if position > matched {
                Some(Risk::LessPrecise)
            } else if offered != current {
                Some(Risk::OtherProvider)
            } else {
                None
            }
        }
        _ => Some(Risk::CannotJudge),
    }
}

/// Adds what the other sources said (Spec - Sources constructeurs, « Moteur »). A Windows Update
/// offer already chosen stays chosen; the others are noted. Otherwise the newest announced
/// offer gives the state `Announced` (R-VER-03), never `Update`.
fn apply_vendors(out: &mut DeviceDecision, device: &Device, vendors: &[VendorResult]) {
    let ids = device_ids(device);
    let installed = device.driver.as_ref();
    let current = installed.and_then(|d| d.version.as_deref().and_then(parse_version));
    let current_date = installed.and_then(|d| d.date.as_deref().filter(|d| is_date(d)));
    let mut found: Vec<(Option<Version>, Candidate)> = Vec::new();
    for result in vendors {
        if result.unchecked.contains(&device.instance_id)
            && !out.unchecked_by.contains(&result.source)
        {
            out.unchecked_by.push(result.source);
        }
        out.hints.extend(
            result
                .hints
                .iter()
                .filter(|h| h.device_instance_id == device.instance_id)
                .cloned(),
        );
        for (offer_index, offer) in result.offers.iter().enumerate() {
            if offer.device_instance_id != device.instance_id {
                continue;
            }
            // R-MAT-01: the ID the source matched must still be one of the device's IDs.
            let Some(position) = ids.iter().position(|id| *id == norm(&offer.hardware_id)) else {
                continue;
            };
            let version = offer.version.as_deref().and_then(parse_version);
            let older_date = matches!(
                (offer.date.as_deref(), current_date),
                (Some(d), Some(cd)) if is_date(d) && d < cd
            );
            // R-VER-01 on the announced version: strictly newer, never an older date.
            let newer = match (version, current) {
                (Some(v), Some(c)) => v > c && !older_date,
                (None, _) => offer.announced_newer,
                (Some(_), None) => installed.is_none(),
            };
            if !newer {
                continue;
            }
            let risk = match installed {
                None => Some(Risk::NoInstalledDriver),
                Some(driver) => judge_risk(driver, &ids, position, offer.provider.as_deref()),
            };
            found.push((
                version,
                Candidate {
                    offer: OfferEvidence {
                        offer_index,
                        hardware_id: offer.hardware_id.clone(),
                        title: offer.title.clone(),
                        version: version.map(|v| v.map(|p| p.to_string()).join(".")),
                        date: offer.date.clone(),
                        provider: offer.provider.clone(),
                        update_id: offer.update_id.clone(),
                        revision: None,
                        source: Some(result.source),
                        vendor_version: offer.vendor_version.clone(),
                        publisher: offer.publisher.clone(),
                        size_bytes: offer.size_bytes,
                        size_text: offer.size_text.clone(),
                        installable: offer.installable,
                        source_url: offer.download_url.clone().or_else(|| {
                            (result.source == SourceId::MsCatalog)
                                .then_some(offer.update_id.as_deref())
                                .flatten()
                                .and_then(|id| crate::sources::ms_catalog::details_url(id).ok())
                                .map(|u| u.to_string())
                        }),
                    },
                    risk,
                },
            ));
        }
    }
    if found.is_empty() {
        return;
    }
    // Installable first, then without risk, then the highest version.
    found.sort_by(|a, b| {
        b.1.offer
            .installable
            .cmp(&a.1.offer.installable)
            .then(a.1.risk.is_some().cmp(&b.1.risk.is_some()))
            .then(b.0.cmp(&a.0))
    });
    let mut candidates = found.into_iter().map(|(_, c)| c);
    if matches!(out.state, State::Update | State::UpdateRisky { .. }) {
        out.announced.extend(candidates);
        return;
    }
    if let Some(chosen) = candidates.next() {
        out.state = State::Announced { risk: chosen.risk };
        out.chosen = Some(chosen);
    }
    out.announced.extend(candidates);
}
