//! Install plan (`plan.json`, schema 2), written by the app and read by the elevated installer.
//! The installer trusts none of it: every item is checked again on the live system (R-SEC-05).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::decide::{Report, State, parse_version};
use crate::model::{InstalledDriver, Snapshot};
use crate::sources::winget::{self, AppUpdate};
use crate::sources::{SourceId, ms_catalog};

/// Schema 2 adds the source `ms_catalog` (Spec - Sources constructeurs, « Installeur »); a plan of
/// schema 1 (Windows Update only) is still read.
pub const PLAN_SCHEMA: u32 = 2;
/// A larger plan file is refused before it is read (Spec - Installation, step 2).
pub const MAX_PLAN_BYTES: u64 = 1024 * 1024;
/// The only source of phase 2 (docs/SOURCES.md).
pub const WINDOWS_UPDATE: &str = "windows_update";
/// Microsoft Update Catalog (phase 3, ADR-0024): the installer looks the update up again itself.
pub const MS_CATALOG: &str = "ms_catalog";

/// R-SEC-05: any other source in a plan is refused.
pub fn known_source(schema: u32, source: &str) -> bool {
    source == WINDOWS_UPDATE || (schema >= 2 && source == MS_CATALOG)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Simulation,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub schema: u32,
    pub pass_id: String,
    pub mode: Mode,
    pub items: Vec<PlanItem>,
    /// R-SEC-09: a pass without driver that only makes a restore point, before an official tool
    /// installs something Drivn cannot back up (Spec - Sources constructeurs, section 3).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore_point_only: bool,
    /// R-SEC-01 (ADR-0036): the user answered "no" to the restore point before this pass.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_restore_point: bool,
    /// Applications winget updates after the drivers, in the same elevated pass (ADR-0034).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppItem>,
    /// Publishers' installers run silently after the drivers (R-SEC-09, ADR-0034).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageItem>,
}

/// One publisher's package of the plan: the installer checks it again before running it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageItem {
    /// The device it updates (the row of the app).
    pub device_instance_id: String,
    /// `SourceId::key()`: one row of `official::OFFICIAL`.
    pub source: String,
    pub name: String,
    pub from: String,
    pub to: String,
    /// The package downloaded and checked by the app, in its cache.
    pub file: String,
}

/// One application of the plan. Only its ID reaches winget, checked again by the installer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub from: String,
    pub to: String,
}

/// The ticked applications (`app:<winget id>`) of the last scan's list, in its order.
pub fn add_apps(plan: &mut Plan, all: &[AppUpdate], selected: &[String]) -> Result<(), String> {
    for wanted in selected.iter().filter_map(|s| s.strip_prefix("app:")) {
        let app = all
            .iter()
            .find(|a| a.id == wanted)
            .ok_or_else(|| format!("{wanted}: not in the scan"))?;
        if !winget::valid_id(&app.id) {
            return Err(format!("{wanted}: invalid winget id"));
        }
        plan.apps.push(AppItem {
            id: app.id.clone(),
            name: app.name.clone(),
            from: app.version.clone(),
            to: app.available.clone(),
        });
    }
    Ok(())
}

/// The plan of a restore point alone (schema 2).
pub fn restore_point_plan(pass_id: &str, mode: Mode) -> Result<Plan, String> {
    if !valid_pass_id(pass_id) {
        return Err(format!("invalid pass id {pass_id}"));
    }
    Ok(Plan {
        schema: PLAN_SCHEMA,
        pass_id: pass_id.to_owned(),
        mode,
        items: Vec::new(),
        restore_point_only: true,
        no_restore_point: false,
        apps: Vec::new(),
        packages: Vec::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub device_instance_id: String,
    /// Shown in the journal and the pass screen; never used to match anything (R-MAT-01).
    #[serde(default)]
    pub name: Option<String>,
    pub hardware_id: String,
    pub source: String,
    pub update_id: String,
    pub revision: i32,
    pub checked_by_user: bool,
    /// `None`: the device had no driver.
    pub installed: Option<DriverFacts>,
    pub offer: DriverFacts,
}

/// What identifies one driver: INF, DriverVer version and date, provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DriverFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inf: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub version: Option<String>,
    pub date: Option<String>,
    pub provider: Option<String>,
}

impl DriverFacts {
    pub fn of(driver: &InstalledDriver) -> Self {
        Self {
            inf: driver.inf_path.clone(),
            title: None,
            version: driver.version.clone(),
            date: driver.date.clone(),
            provider: driver.provider.clone(),
        }
    }

    /// Same version, date and provider (the INF name is compared apart: Windows reuses `oem#.inf`).
    pub fn same_release(&self, driver: &InstalledDriver) -> bool {
        let version = |v: &Option<String>| v.as_deref().and_then(parse_version);
        let eq = |a: &Option<String>, b: &Option<String>| {
            a.as_deref().map(|s| s.trim().to_lowercase())
                == b.as_deref().map(|s| s.trim().to_lowercase())
        };
        version(&self.version).is_some()
            && version(&self.version) == version(&driver.version)
            && eq(&self.date, &driver.date)
            && eq(&self.provider, &driver.provider)
    }

    /// The same driver as `driver`, INF name included.
    pub fn is(&self, driver: &InstalledDriver) -> bool {
        let inf = |v: &Option<String>| v.as_deref().map(str::to_lowercase);
        self.same_release(driver) && inf(&self.inf) == inf(&driver.inf_path)
    }
}

/// `oem#.inf`: a third-party driver that `pnputil` can export. Anything else is a driver of Windows.
pub fn is_oem_inf(inf: &str) -> bool {
    let lower = inf.trim().to_lowercase();
    lower
        .strip_prefix("oem")
        .and_then(|rest| rest.strip_suffix(".inf"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `AAAA-MM-JJTHHMM-xxxx`, `x` in `[a-z0-9]`. Checked before any path is built from it.
pub fn valid_pass_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 15 => *c == b'-',
            10 => *c == b'T',
            16..=19 => c.is_ascii_digit() || c.is_ascii_lowercase(),
            _ => c.is_ascii_digit(),
        })
}

/// Pass ID from a UTC time `YYYY-MM-DDTHH:MM:SSZ` and 4 random characters.
pub fn new_pass_id(utc: &str, random: &str) -> Option<String> {
    let id = format!(
        "{}T{}{}-{}",
        utc.get(..10)?,
        utc.get(11..13)?,
        utc.get(14..16)?,
        random
    );
    valid_pass_id(&id).then_some(id)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::sources::text::hex(&Sha256::digest(bytes))
}

/// The plan of the checked rows, in the order shown (Spec - Installation, « Le plan »).
/// Only a device the engine says to update, with a Windows Update identity, can be in it.
pub fn build_plan(
    snapshot: &Snapshot,
    report: &Report,
    selected: &[String],
    pass_id: &str,
    mode: Mode,
) -> Result<Plan, String> {
    if !valid_pass_id(pass_id) {
        return Err(format!("invalid pass id {pass_id}"));
    }
    let mut items = Vec::new();
    for id in selected {
        let index = snapshot
            .devices
            .iter()
            .position(|d| d.instance_id == *id)
            .ok_or_else(|| format!("{id}: not in the scan"))?;
        let device = &snapshot.devices[index];
        let decision = report
            .decisions
            .get(index)
            .filter(|d| d.instance_id == *id)
            .ok_or_else(|| format!("{id}: no decision"))?;
        let chosen = decision
            .chosen
            .as_ref()
            .ok_or_else(|| format!("{id}: no offer"))?;
        // Windows Update, or an installable Catalog offer the user ticked (never Drivn, R-SRC-06).
        let (source, update_id, revision) = match (&decision.state, chosen.offer.source) {
            (State::Update | State::UpdateRisky { .. }, None) => {
                let (Some(update_id), Some(revision)) =
                    (&chosen.offer.update_id, chosen.offer.revision)
                else {
                    return Err(format!("{id}: the offer has no Windows Update identity"));
                };
                (WINDOWS_UPDATE, update_id, revision)
            }
            (State::Announced { .. }, Some(SourceId::MsCatalog)) if chosen.offer.installable => {
                let Some(update_id) = chosen
                    .offer
                    .update_id
                    .as_ref()
                    .filter(|u| ms_catalog::is_guid(u))
                else {
                    return Err(format!("{id}: the offer has no Catalog identity"));
                };
                (MS_CATALOG, update_id, 0)
            }
            _ => return Err(format!("{id}: not an installable update")),
        };
        items.push(PlanItem {
            device_instance_id: id.clone(),
            name: device.description.clone(),
            hardware_id: chosen.offer.hardware_id.clone(),
            source: source.to_owned(),
            update_id: update_id.clone(),
            revision,
            checked_by_user: true,
            installed: device.driver.as_ref().map(DriverFacts::of),
            offer: DriverFacts {
                inf: None,
                title: chosen.offer.title.clone(),
                version: chosen.offer.version.clone(),
                date: chosen.offer.date.clone(),
                provider: chosen.offer.provider.clone(),
            },
        });
    }
    if items.is_empty() {
        return Err("nothing selected".to_owned());
    }
    Ok(Plan {
        schema: PLAN_SCHEMA,
        pass_id: pass_id.to_owned(),
        mode,
        items,
        restore_point_only: false,
        no_restore_point: false,
        apps: Vec::new(),
        packages: Vec::new(),
    })
}
