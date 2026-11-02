//! Runs winget, Windows' own package manager (ADR-0034): the list of applications with a newer
//! version (read-only, as the user), and the silent update of one application (in the pass).
//! winget is part of Windows (App Installer); it is called through its execution alias.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use drivn_core::sources::winget::{
    AppUpdate, LIST_ARGS, parse_installer_url, parse_upgrades, replace_args, show_args,
    upgrade_args,
};

/// No console window flashes over the app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn winget() -> Result<PathBuf, String> {
    let alias = crate::security::local_app_data()?
        .join("Microsoft")
        .join("WindowsApps")
        .join("winget.exe");
    if alias.exists() {
        Ok(alias)
    } else {
        Err("winget_missing".to_owned())
    }
}

/// Runs winget with `args`; stops it after `limit`. Returns its exit code and its output.
fn run(args: &[String], limit: Duration) -> Result<(i32, String), String> {
    let mut child = Command::new(winget()?)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("winget : {e}"))?;
    let mut stdout = child.stdout.take().ok_or("winget : pas de sortie")?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        out
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            return Err("winget_timeout".to_owned());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let out = reader.join().unwrap_or_default();
    Ok((
        status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out).into_owned(),
    ))
}

/// Applications with a newer version (read-only, no agreement accepted, no admin rights).
pub fn upgrades() -> Result<Vec<AppUpdate>, String> {
    let args: Vec<String> = LIST_ARGS.iter().map(|a| (*a).to_owned()).collect();
    let (_, out) = run(&args, Duration::from_secs(180))?;
    Ok(parse_upgrades(&out))
}

/// Read only: the link winget will download this application's installer from.
pub fn installer_url(id: &str) -> Result<Option<String>, String> {
    let args = show_args(id).ok_or("winget id refused")?;
    let (_, out) = run(&args, Duration::from_secs(60))?;
    Ok(parse_installer_url(&out))
}

/// Updates one application silently. Returns winget's exit code (0 = done).
pub fn upgrade(id: &str) -> Result<i32, String> {
    let args = upgrade_args(id).ok_or("winget id refused")?;
    run(&args, Duration::from_secs(30 * 60)).map(|(code, _)| code)
}

/// The same, the old version uninstalled first (its new version installs another way).
pub fn replace(id: &str) -> Result<i32, String> {
    let args = replace_args(id).ok_or("winget id refused")?;
    run(&args, Duration::from_secs(30 * 60)).map(|(code, _)| code)
}
