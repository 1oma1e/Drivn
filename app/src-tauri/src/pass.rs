//! The app's side of a pass (Spec - Installation). The app has no admin rights (R-DRT-01): it writes
//! the plan and the user's orders in `%ProgramData%\Drivn-requests\`, starts `drivn-installer.exe`
//! through the UAC, and follows the pass by reading its journal. It installs nothing itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use drivn_core::time::now_iso_utc;

use drivn_core::decide::Report;
use drivn_core::install::control::Order;
use drivn_core::install::journal::Status as ItemStatus;
use drivn_core::install::journal::{Line, PassView, parse, reconstruct};
use drivn_core::install::plan::{
    Mode, Plan, add_apps, build_plan, new_pass_id, restore_point_plan, sha256_hex, valid_pass_id,
};
use drivn_core::install::reboot::{AfterReboot, after_reboot, reboot_needed, reboot_pending};
use drivn_core::install::setup::{may_launch, plan_mode};
use drivn_core::model::Snapshot;
use drivn_core::sources::winget::AppUpdate;
use drivn_windows::installer::{LaunchError, installer_running, last_boot_unix, launch_elevated};
use drivn_windows::security::{is_reparse_point, program_data, system32, writable_by_this_user};
use drivn_windows::{inf_signer, read_one};
use serde::Serialize;

#[derive(Default)]
pub struct PassState {
    /// The last scan: the plan is built from it, never from what the interface sends back.
    pub last_scan: Mutex<Option<(Snapshot, Report)>>,
    /// The applications winget listed in the last scan (ADR-0034).
    pub last_apps: Mutex<Vec<AppUpdate>>,
    /// Exit code of each installer started by this app.
    exits: Arc<Mutex<HashMap<String, u32>>>,
}

struct Paths {
    drivn: PathBuf,
    requests: PathBuf,
}

fn paths() -> Result<Paths, String> {
    let base = program_data()?;
    Ok(Paths {
        drivn: base.join("Drivn"),
        requests: base.join("Drivn-requests"),
    })
}

fn checked(pass_id: &str) -> Result<(), String> {
    if valid_pass_id(pass_id) {
        Ok(())
    } else {
        Err("invalid pass id".to_owned())
    }
}

/// `real-mode` exists only where an admin created it by hand (the VM, R-TST-01).
fn mode() -> Result<Mode, String> {
    let file = paths()?.drivn.join("real-mode");
    Ok(plan_mode(file.is_file() && !is_reparse_point(&file)))
}

fn installer() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe
        .parent()
        .map(|dir| dir.join("drivn-installer.exe"))
        .ok_or("Drivn's folder not found")?;
    if !path.is_file() {
        return Err(format!("installer_missing: {}", path.display()));
    }
    Ok(path)
}

fn random4() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    sha256_hex(format!("{nanos}-{}", std::process::id()).as_bytes())[..4].to_owned()
}

#[derive(Serialize)]
pub struct Started {
    pass_id: String,
    simulation: bool,
}

/// Starts the installer and remembers its exit code when it ends.
fn launch(state: &PassState, pass_id: &str, args: String) -> Result<(), String> {
    let exe = installer()?;
    // The UAC "Yes" goes to this file: in real mode, never one a user program could replace.
    let writable = writable_by_this_user(&exe) || exe.parent().is_none_or(writable_by_this_user);
    may_launch(mode()?, writable)?;
    if installer_running() {
        return Err("pass_running".to_owned());
    }
    // An exit code from an earlier launch for this pass must not end the new one at once.
    if let Ok(mut map) = state.exits.lock() {
        map.remove(pass_id);
    }
    let child = match launch_elevated(&exe, &args) {
        Ok(child) => child,
        Err(LaunchError::Refused) => return Err("uac_refused".to_owned()),
        Err(LaunchError::Failed(e)) => return Err(format!("installer not started: {e}")),
    };
    let exits = Arc::clone(&state.exits);
    let id = pass_id.to_owned();
    std::thread::spawn(move || {
        let code = child.wait().unwrap_or(u32::MAX);
        if let Ok(mut map) = exits.lock() {
            map.insert(id, code);
        }
    });
    Ok(())
}

fn write_request(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(name);
    if file.exists() && is_reparse_point(&file) {
        return Err(format!("{} is a link", file.display()));
    }
    std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))
}

/// "real" or "simulation": what the next pass will be.
pub fn install_mode() -> Result<&'static str, String> {
    Ok(match mode()? {
        Mode::Real => "real",
        Mode::Simulation => "simulation",
    })
}

/// Builds the plan of the ticked rows from the last scan, writes it, then the UAC. Blocks until the
/// user answered the UAC.
pub fn install_start(
    state: &PassState,
    ids: &[String],
    restore_point: bool,
) -> Result<Started, String> {
    let (snapshot, report) = state
        .last_scan
        .lock()
        .map_err(|_| "state unreadable")?
        .clone()
        .ok_or("scan_first")?;
    let pass_id = next_pass_id()?;
    let mode = mode()?;
    // ADR-0034: drivers (`instance id`), publishers' packages (`pkg:<instance id>`) and
    // applications (`app:<winget id>`) in one pass.
    let (apps, rest): (Vec<String>, Vec<String>) =
        ids.iter().cloned().partition(|id| id.starts_with("app:"));
    let (packages, drivers): (Vec<String>, Vec<String>) =
        rest.into_iter().partition(|id| id.starts_with("pkg:"));
    let mut plan = if drivers.is_empty() {
        restore_point_plan(&pass_id, mode)?
    } else {
        build_plan(&snapshot, &report, &drivers, &pass_id, mode)?
    };
    let known = state
        .last_apps
        .lock()
        .map_err(|_| "state unreadable")?
        .clone();
    add_apps(&mut plan, &known, &apps)?;
    // R-SEC-01 (ADR-0036): the answer given just before the pass.
    plan.no_restore_point = !restore_point;
    // R-SEC-09: each package was downloaded and checked by `package_fetch` before this call.
    for id in &packages {
        let device = id.strip_prefix("pkg:").unwrap_or_default();
        plan.packages
            .push(crate::package::planned(&snapshot, device)?);
    }
    start_plan(state, &plan)
}

/// R-SEC-09: a pass that only makes a restore point, before an official tool installs something.
pub fn restore_point_start(state: &PassState) -> Result<Started, String> {
    let plan = restore_point_plan(&next_pass_id()?, mode()?)?;
    start_plan(state, &plan)
}

fn next_pass_id() -> Result<String, String> {
    new_pass_id(&now_iso_utc(), &random4()).ok_or_else(|| "no pass id".to_owned())
}

/// Writes the plan where the installer reads it, then starts it with the plan's SHA-256: the
/// installer refuses a plan changed in between (R-SEC-05).
fn start_plan(state: &PassState, plan: &Plan) -> Result<Started, String> {
    let bytes = serde_json::to_vec_pretty(plan).map_err(|e| e.to_string())?;
    let pass_id = &plan.pass_id;
    write_request(
        &paths()?.requests.join("plans"),
        &format!("{pass_id}.json"),
        &bytes,
    )?;
    let sha = sha256_hex(&bytes);
    launch(state, pass_id, format!("--pass {pass_id} --sha256 {sha}"))?;
    Ok(Started {
        pass_id: pass_id.clone(),
        simulation: plan.mode == Mode::Simulation,
    })
}

#[derive(Serialize)]
pub struct PlanRow {
    n: usize,
    id: String,
    name: Option<String>,
}

#[derive(Serialize)]
pub struct Status {
    pass_id: String,
    lines: Vec<Line>,
    view: Option<PassView>,
    plan: Vec<PlanRow>,
    /// Exit code of the installer, once it ended (`None`: still running, or started elsewhere).
    exit: Option<u32>,
    error: Option<String>,
    running: bool,
    reboot: Vec<String>,
    reboot_pending: bool,
    /// R-SRC-04 after a restart: the drivers that waited for it, read again now (read-only).
    after_reboot: Vec<Checked>,
}

#[derive(Serialize)]
pub struct Checked {
    n: usize,
    /// `verified`, `unsigned`, `not_active` or `waiting`.
    state: &'static str,
    signer: Option<String>,
}

/// The new driver of each `installed_pending_reboot` item, once Windows has restarted: active?
/// signed? Nothing is changed; an unsigned one is shown with its rollback (which asks the UAC).
fn checked_after_reboot(view: &PassView, boot: i64) -> Vec<Checked> {
    view.items
        .iter()
        .filter(|i| i.status == ItemStatus::InstalledPendingReboot)
        .filter_map(|item| {
            let now = read_one(&item.device_instance_id).and_then(|d| d.driver);
            let check = after_reboot(view, item.n, boot, now.as_ref(), inf_signer)?;
            let (state, signer) = match check {
                AfterReboot::Waiting => ("waiting", None),
                AfterReboot::Verified { signer } => ("verified", Some(signer)),
                AfterReboot::Unsigned => ("unsigned", None),
                AfterReboot::NotActive => ("not_active", None),
            };
            Some(Checked {
                n: item.n,
                state,
                signer,
            })
        })
        .collect()
}

fn read_plan(dir: &Path) -> Vec<PlanRow> {
    std::fs::read(dir.join("plan.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Plan>(&b).ok())
        .map(|plan| {
            plan.items
                .into_iter()
                .enumerate()
                .map(|(i, item)| PlanRow {
                    n: i + 1,
                    id: item.device_instance_id,
                    name: item.name,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn status_of(pass_id: &str, exit: Option<u32>) -> Result<Status, String> {
    let paths = paths()?;
    let dir = paths.drivn.join("passes").join(pass_id);
    let text = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap_or_default();
    let view = reconstruct(&text).ok();
    let error = exit.filter(|code| *code != 0).map(|code| {
        let file = paths.drivn.join("errors").join(format!("{pass_id}.txt"));
        let why = std::fs::read_to_string(file).unwrap_or_default();
        if why.is_empty() {
            format!("code {code}")
        } else {
            format!("code {code}: {why}")
        }
    });
    // The pass is over: its plan and control files are no longer needed (an old word left there
    // could only be replayed by mistake).
    if view.as_ref().is_some_and(|v| !v.interrupted()) && !installer_running() {
        let _ = std::fs::remove_file(paths.requests.join("plans").join(format!("{pass_id}.json")));
        let _ = std::fs::remove_file(
            paths
                .requests
                .join("control")
                .join(format!("{pass_id}.cmd")),
        );
    }
    let boot = last_boot_unix();
    Ok(Status {
        pass_id: pass_id.to_owned(),
        lines: parse(&text).unwrap_or_default(),
        reboot: view.as_ref().map(reboot_needed).unwrap_or_default(),
        reboot_pending: view.as_ref().is_some_and(|v| reboot_pending(v, boot)),
        after_reboot: view
            .as_ref()
            .map(|v| checked_after_reboot(v, boot))
            .unwrap_or_default(),
        view,
        plan: read_plan(&dir),
        exit,
        error,
        running: installer_running(),
    })
}

pub fn pass_status(state: &PassState, pass_id: &str) -> Result<Status, String> {
    checked(pass_id)?;
    let exit = state
        .exits
        .lock()
        .ok()
        .and_then(|m| m.get(pass_id).copied());
    status_of(pass_id, exit)
}

/// One word for the installer, read between two drivers (R-SEC-08).
pub fn pass_control(pass_id: &str, word: &str) -> Result<(), String> {
    checked(pass_id)?;
    let order = Order::parse(word).ok_or("unknown order")?;
    write_request(
        &paths()?.requests.join("control"),
        &format!("{pass_id}.cmd"),
        order.word().as_bytes(),
    )
}

/// One driver put back: a UAC, then waits for the installer.
pub fn pass_rollback(state: &PassState, pass_id: &str, n: usize) -> Result<Status, String> {
    checked(pass_id)?;
    launch(state, pass_id, format!("--rollback {pass_id} {n}"))?;
    loop {
        if let Some(code) = state
            .exits
            .lock()
            .ok()
            .and_then(|m| m.get(pass_id).copied())
        {
            return status_of(pass_id, Some(code));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// R-SEC-06: an interrupted pass taken up again, or stopped (keep / revert), on the user's choice.
pub fn pass_resume(state: &PassState, pass_id: &str, stop: Option<&str>) -> Result<(), String> {
    checked(pass_id)?;
    let args = match stop {
        None => format!("--resume {pass_id}"),
        Some("keep") => format!("--resume {pass_id} --stop keep"),
        Some("revert") => format!("--resume {pass_id} --stop revert"),
        Some(_) => return Err("unknown choice".to_owned()),
    };
    if let Ok(mut map) = state.exits.lock() {
        map.remove(pass_id);
    }
    launch(state, pass_id, args)
}

/// Every pass on this PC, newest first (History, interrupted pass, pending restart).
pub fn passes_list() -> Result<Vec<Status>, String> {
    let dir = paths()?.drivn.join("passes");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|id| valid_pass_id(id))
        .collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids.iter().map(|id| status_of(id, None)).collect()
}

/// « Redémarrer maintenant »: the normal restart of Windows, on the user's click only, and only
/// when a real pass waits for it (R-RBT-01).
pub fn reboot_now() -> Result<(), String> {
    if !passes_list()?.iter().any(|p| p.reboot_pending) {
        return Err("no_reboot_pending".to_owned());
    }
    std::process::Command::new(system32()?.join("shutdown.exe"))
        .args(["/r", "/t", "0"])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens the System Protection settings of Windows (it asks for the UAC itself).
pub fn open_protection() -> Result<(), String> {
    std::process::Command::new(system32()?.join("SystemPropertiesProtection.exe"))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
