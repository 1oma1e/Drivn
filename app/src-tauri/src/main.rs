//! Drivn desktop app. Serves the interface (app/ui, ADR-0014): a real read-only scan followed by the
//! decision engine, then the install pass. Nothing here installs anything: the app writes the plan
//! and starts the elevated `drivn-installer.exe`, which does the pass (R-DRT-01, `pass.rs`).
//! Errors the interface words itself are short codes (`scan_first`…); other ones are technical
//! details, in English, shown under the translated message.
#![windows_subsystem = "windows"]

mod package;
mod pass;
mod tools;

use std::process::ExitCode;

use drivn_core::decide::{Report, decide};
use drivn_core::games::{GameComponent, game_components};
use drivn_core::model::{Device, Snapshot};
use drivn_core::scan::{ScanContext, ScanEvent, ScanOptions, scan_with_progress};
use drivn_core::sources::winget::AppUpdate;
use drivn_core::time::now_iso_utc;
use drivn_windows::vendors::ask_vendors_with_progress;
use drivn_windows::{CfgMgrDevices, WindowsUpdateAgent, installed_programs};
use serde::Serialize;
use tauri::Manager;
use tauri::ipc::Channel;

use crate::pass::PassState;

/// Runs blocking work (UAC, downloads, Windows calls) on a worker thread, never on the async
/// runtime that serves the window: the interface keeps moving while it waits.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
enum Progress {
    /// Step 1: whole percent of the devices read, and the device just read (Spec - Scan complet).
    Reading {
        percent: u8,
        current: Option<String>,
    },
    /// All devices are read; Windows Update may still be answering.
    HardwareRead { devices: Vec<Device> },
    /// Windows Update answered; the publishers' sites are asked, one question at a time:
    /// `done` out of `total` questions (Catalog terms, NVIDIA cards, Oracle); `source` and
    /// `about`: the question being asked now.
    Vendors {
        done: usize,
        total: usize,
        source: Option<String>,
        about: Option<String>,
    },
    /// winget is listing the applications with a newer version (no progress of its own).
    Apps,
}

/// Whole percent of `done` out of `total`; an empty list counts as fully read.
fn percent(done: usize, total: usize) -> u8 {
    if total == 0 {
        return 100;
    }
    u8::try_from(done.min(total) * 100 / total).unwrap_or(100)
}

#[derive(Serialize)]
struct ScanResult {
    snapshot: Snapshot,
    report: Report,
    /// Software games need (ADR-0016), read from the registry: installed version only.
    games: Vec<GameComponent>,
    /// Applications with a newer version, from winget (ADR-0034).
    apps: Vec<AppUpdate>,
    /// Why winget could not list them, said on screen (R-VRAI-04).
    apps_error: Option<String>,
}

/// Reads the hardware without admin rights (R-DRT-01) while Windows Update is asked, then decides.
/// Windows Update is searched online at every scan (ADR-0018), then the publishers' sites of
/// the allow-list (R-SRC-01); no other network access (R-VRAI-02).
#[tauri::command]
async fn scan(
    on_progress: Channel<Progress>,
    state: tauri::State<'_, PassState>,
) -> Result<ScanResult, String> {
    let result = blocking(move || {
        let context = ScanContext {
            tool_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_utc: now_iso_utc(),
            machine: None,
        };
        let options = ScanOptions {
            include_absent: true,
            windows_update: true,
            anonymize: false,
        };
        let mut snapshot = scan_with_progress(
            &CfgMgrDevices,
            &WindowsUpdateAgent { online: true },
            &options,
            context,
            &mut |event| {
                // A closed window drops the channel: the scan just finishes unseen.
                let _ = on_progress.send(match event {
                    ScanEvent::Reading {
                        done,
                        total,
                        current,
                    } => Progress::Reading {
                        percent: percent(done, total),
                        current: current.map(str::to_owned),
                    },
                    ScanEvent::HardwareRead(devices) => Progress::HardwareRead {
                        devices: devices.to_vec(),
                    },
                });
            },
        )?;
        // Catalog, NVIDIA, Oracle… (fragile sources, R-SRC-11), after Windows Update.
        snapshot.vendors =
            ask_vendors_with_progress(&snapshot.devices, &mut |done, total, asking| {
                let _ = on_progress.send(Progress::Vendors {
                    done,
                    total,
                    source: asking.map(|a| a.source.key().to_owned()),
                    about: asking.map(|a| a.about.to_owned()),
                });
            });
        let report = decide(&snapshot);
        let games = game_components(installed_programs());
        // ADR-0034: the applications too, from winget, read-only.
        let _ = on_progress.send(Progress::Apps);
        let (apps, apps_error) = match drivn_windows::winget::upgrades() {
            Ok(apps) => (apps, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        Ok(ScanResult {
            snapshot,
            report,
            games,
            apps,
            apps_error,
        })
    })
    .await?;
    // The install plan is built from this scan only, never from what the interface sends.
    if let Ok(mut last) = state.last_scan.lock() {
        *last = Some((result.snapshot.clone(), result.report.clone()));
    }
    if let Ok(mut apps) = state.last_apps.lock() {
        apps.clone_from(&result.apps);
    }
    Ok(result)
}

/// R-SEC-09: restore point alone, before an official tool. Waits for the UAC.
#[tauri::command]
async fn restore_point_start(app: tauri::AppHandle) -> Result<pass::Started, String> {
    blocking(move || pass::restore_point_start(&app.state::<PassState>())).await
}

/// Read only: where winget will download this application from (shown in its sheet).
#[tauri::command]
async fn app_source(id: String) -> Result<Option<String>, String> {
    blocking(move || drivn_windows::winget::installer_url(&id)).await
}

#[tauri::command]
fn tool_check(kind: &str) -> Result<tools::ToolCheck, String> {
    tools::tool_check(kind)
}

#[tauri::command]
fn tool_launch(kind: &str) -> Result<(), String> {
    tools::tool_launch(kind)
}

#[tauri::command]
fn tool_page(kind: &str) -> Result<(), String> {
    tools::tool_page(kind)
}

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// A newer Drivn on GitHub Releases (ADR-0027). `None`: up to date; an error: not checked.
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version,
        notes: u.body,
    }))
}

/// Bytes of the new setup received so far, and its size when the server gives it.
#[derive(Clone, Serialize)]
struct UpdateProgress {
    done: u64,
    total: Option<u64>,
}

/// Downloads the new setup, checks its signature with the public key built into Drivn (the
/// private key never leaves its author's PC), runs it and restarts. Never during a pass.
#[tauri::command]
async fn update_install(
    app: tauri::AppHandle,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if drivn_windows::installer::installer_running() {
        return Err("pass_running".to_owned());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("up_to_date")?;
    let mut done = 0u64;
    let mut sent = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                done += u64::try_from(chunk).unwrap_or(0);
                // About every 256 KB: enough to see it move, light on the window.
                if done - sent >= 256 * 1024 || total == Some(done) {
                    sent = done;
                    let _ = on_progress.send(UpdateProgress { done, total });
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart()
}

/// The publisher's own installer: NVIDIA for the graphics card (ADR-0030), Intel for Wi-Fi and
/// Bluetooth (ADR-0033). Downloads up to 1 GB.
#[tauri::command]
async fn package_install(
    app: tauri::AppHandle,
    device_id: String,
    on_progress: Channel<package::PackageProgress>,
) -> Result<(), String> {
    blocking(move || package::install(&app.state::<PassState>(), &device_id, &on_progress)).await
}

/// R-SEC-09 (ADR-0034): downloads and checks a package before the pass that runs it silently.
#[tauri::command]
async fn package_fetch(
    app: tauri::AppHandle,
    device_id: String,
    on_progress: Channel<package::PackageProgress>,
) -> Result<(), String> {
    blocking(move || {
        package::fetch(&app.state::<PassState>(), &device_id, &on_progress).map(|_| ())
    })
    .await
}

#[tauri::command]
fn install_mode() -> Result<&'static str, String> {
    pass::install_mode()
}

/// Waits for the UAC.
#[tauri::command]
async fn install_start(
    app: tauri::AppHandle,
    ids: Vec<String>,
    restore_point: Option<bool>,
) -> Result<pass::Started, String> {
    // ADR-0036: "yes" unless the user said no.
    let point = restore_point.unwrap_or(true);
    blocking(move || pass::install_start(&app.state::<PassState>(), &ids, point)).await
}

#[tauri::command]
fn pass_status(pass_id: &str, state: tauri::State<'_, PassState>) -> Result<pass::Status, String> {
    pass::pass_status(&state, pass_id)
}

#[tauri::command]
fn pass_control(pass_id: &str, word: &str) -> Result<(), String> {
    pass::pass_control(pass_id, word)
}

#[tauri::command]
async fn pass_rollback(
    app: tauri::AppHandle,
    pass_id: String,
    n: usize,
) -> Result<pass::Status, String> {
    blocking(move || pass::pass_rollback(&app.state::<PassState>(), &pass_id, n)).await
}

#[tauri::command]
async fn pass_resume(
    app: tauri::AppHandle,
    pass_id: String,
    stop: Option<String>,
) -> Result<(), String> {
    blocking(move || pass::pass_resume(&app.state::<PassState>(), &pass_id, stop.as_deref())).await
}

#[tauri::command]
fn passes_list() -> Result<Vec<pass::Status>, String> {
    pass::passes_list()
}

#[tauri::command]
fn reboot_now() -> Result<(), String> {
    pass::reboot_now()
}

#[tauri::command]
fn open_protection() -> Result<(), String> {
    pass::open_protection()
}

fn main() -> ExitCode {
    drivn_windows::security::harden_dll_search();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PassState::default())
        .invoke_handler(tauri::generate_handler![
            scan,
            install_mode,
            install_start,
            pass_status,
            pass_control,
            pass_rollback,
            pass_resume,
            passes_list,
            reboot_now,
            open_protection,
            restore_point_start,
            tool_check,
            tool_launch,
            tool_page,
            update_check,
            update_install,
            package_install,
            package_fetch,
            app_source
        ])
        .run(tauri::generate_context!());
    match app {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("drivn: {error}");
            ExitCode::FAILURE
        }
    }
}
