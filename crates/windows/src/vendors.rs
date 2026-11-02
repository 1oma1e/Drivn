//! Asks the fragile sources after a scan (Spec - Sources constructeurs, 4), for the app and
//! `drivn-scan` alike. The R-SRC-11 state (blocks, answers kept 24 h) is saved in
//! `%LocalAppData%\Drivn\cache\sources.json`, so a block survives a restart of the app.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use drivn_core::games::{GameKind, game_components};
use drivn_core::model::{Device, VendorResult};
use drivn_core::sources::guard::{Guard, GuardState};
use drivn_core::sources::vendors::{Asking, VendorContext, check_vendors_with_progress};

use crate::http::WinHttp;
use crate::{board, installed_programs, windows_release};

fn guard_path() -> Option<PathBuf> {
    crate::security::local_app_data()
        .ok()
        .map(|dir| dir.join("Drivn").join("cache").join("sources.json"))
}

fn load() -> GuardState {
    guard_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(state: &GuardState) {
    let Some(path) = guard_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(state) {
        let _ = std::fs::write(&path, text);
    }
}

/// Network access (one request at a time, at least 1 s apart); no administrator rights.
pub fn ask_vendors(devices: &[Device]) -> Vec<VendorResult> {
    ask_vendors_with_progress(devices, &mut |_, _, _| {})
}

/// Same, with `progress(done, total, asking)` before each question asked.
pub fn ask_vendors_with_progress(
    devices: &[Device],
    progress: &mut dyn FnMut(usize, usize, Option<Asking<'_>>),
) -> Vec<VendorResult> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default();
    let mut guard = Guard::new(load(), now);
    let java: Vec<String> = game_components(installed_programs())
        .into_iter()
        .filter(|c| c.kind == GameKind::Java)
        .filter_map(|c| c.version)
        .collect();
    let release = windows_release();
    let board = board();
    let context = VendorContext {
        now,
        os_release: release.as_deref(),
        java_versions: &java,
        board: Some(&board),
    };
    let results = check_vendors_with_progress(devices, &WinHttp, &mut guard, &context, progress);
    save(&guard.state);
    results
}
