//! What the elevated installer checks before it touches anything (Spec - Installation, steps 2-3,
//! « Fichiers et droits », « Où est l'installeur »). The Windows side gathers the facts; the
//! decisions are here, so each refusal is tested (R-SEC-05, R-TST-01, R-DRT-01).

use super::plan::{
    MAX_PLAN_BYTES, Mode, PLAN_SCHEMA, Plan, known_source, sha256_hex, valid_pass_id,
};

/// Owners allowed for `%ProgramData%\Drivn\` and every folder under it: Administrators, SYSTEM.
pub const TRUSTED_OWNERS: [&str; 2] = ["S-1-5-32-544", "S-1-5-18"];

/// One folder of the installer's root, as read on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFacts {
    pub path: String,
    /// Junction, symbolic link or any other reparse point.
    pub reparse_point: bool,
    /// Owner SID, `None` when unreadable.
    pub owner_sid: Option<String>,
}

/// A reparse point or an unexpected owner could make the elevated installer write elsewhere:
/// the pass is refused, with a clear message.
pub fn check_root(folders: &[FolderFacts]) -> Result<(), String> {
    for folder in folders {
        if folder.reparse_point {
            return Err(format!(
                "{} is a link or junction: Drivn refuses to write there",
                folder.path
            ));
        }
        match &folder.owner_sid {
            Some(sid) if TRUSTED_OWNERS.contains(&sid.as_str()) => {}
            other => {
                return Err(format!(
                    "{} has an unexpected owner ({}): Drivn refuses to write there",
                    folder.path,
                    other.as_deref().unwrap_or("unreadable")
                ));
            }
        }
    }
    Ok(())
}

/// The plan file proposed by the app, as read on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFile {
    pub reparse_point: bool,
    pub size: u64,
}

/// Before the copy: pass ID well formed, pass folder new, plan file a plain small file.
pub fn check_before_copy(
    pass_id: &str,
    pass_folder_exists: bool,
    file: &PlanFile,
) -> Result<(), String> {
    if !valid_pass_id(pass_id) {
        return Err("invalid pass id".to_owned());
    }
    if pass_folder_exists {
        return Err(format!("pass {pass_id} already exists"));
    }
    if file.reparse_point {
        return Err("the plan file is a link".to_owned());
    }
    if file.size > MAX_PLAN_BYTES {
        return Err("the plan file is too large".to_owned());
    }
    Ok(())
}

/// After the copy, on the copy only: hash given at launch, schema, pass ID.
pub fn check_copy(pass_id: &str, expected_sha256: &str, copy: &[u8]) -> Result<Plan, String> {
    if sha256_hex(copy) != expected_sha256.trim().to_lowercase() {
        return Err("the plan changed after the app wrote it (hash differs)".to_owned());
    }
    let plan: Plan = serde_json::from_slice(copy).map_err(|e| format!("unreadable plan: {e}"))?;
    if plan.schema == 0 || plan.schema > PLAN_SCHEMA {
        return Err(format!("unknown plan schema {}", plan.schema));
    }
    if let Some(item) = plan
        .items
        .iter()
        .find(|i| !known_source(plan.schema, &i.source))
    {
        return Err(format!("unknown source {}", item.source));
    }
    if plan.pass_id != pass_id {
        return Err("the plan is for another pass".to_owned());
    }
    // R-SEC-09: only a restore-point pass has no driver, and it has none at all.
    // R-SEC-05 (ADR-0034): an application ID is checked again before it reaches winget.
    if plan
        .apps
        .iter()
        .any(|a| !crate::sources::winget::valid_id(&a.id))
    {
        return Err("invalid application in the plan".to_owned());
    }
    // R-SEC-09 (ADR-0034): a package of a known publisher, from the app's cache only.
    if plan.packages.iter().any(|p| {
        super::official::official(&p.source)
            .is_none_or(|o| !super::official::valid_file(o, &p.file))
    }) {
        return Err("invalid package in the plan".to_owned());
    }
    if plan.items.is_empty() != plan.restore_point_only {
        return Err(if plan.restore_point_only {
            "a restore-point pass holds no driver".to_owned()
        } else {
            "empty plan".to_owned()
        });
    }
    if plan.restore_point_only && plan.schema < 2 {
        return Err("a restore-point pass needs schema 2".to_owned());
    }
    Ok(plan)
}

/// R-TST-01: real only when the plan asks for it **and** an admin created `real-mode` (in the VM).
pub fn simulation(plan_mode: Mode, real_mode_file: bool) -> bool {
    !(plan_mode == Mode::Real && real_mode_file)
}

/// The mode the app writes in the plan: real only where `real-mode` exists.
pub fn plan_mode(real_mode_file: bool) -> Mode {
    if real_mode_file {
        Mode::Real
    } else {
        Mode::Simulation
    }
}

/// The UAC "Yes" goes to a file: in real mode, the app never elevates an installer that
/// Users or Authenticated Users could replace.
pub fn may_launch(mode: Mode, installer_writable_by_users: bool) -> Result<(), String> {
    if mode == Mode::Real && installer_writable_by_users {
        return Err(
            "drivn-installer.exe can be changed by a standard user: Drivn refuses to elevate it"
                .to_owned(),
        );
    }
    Ok(())
}
