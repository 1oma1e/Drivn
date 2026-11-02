//! Official tools of a publisher (Java Update, Intel DSA): started only when signed by the expected signer,
//! without admin rights (R-SRC-10). Before, the screen offers a restore point (R-SEC-09, `pass.rs`).

use std::path::PathBuf;

use drivn_core::sources::tools::{Launch, OfficialTool, decide_launch, page_url, tool};
use drivn_windows::authenticode::file_signer;
use drivn_windows::security::{is_reparse_point, windows_dir};
use serde::Serialize;

#[derive(Serialize)]
pub struct ToolCheck {
    name: &'static str,
    /// `false`: only the official page is offered, for the reason in `why`.
    run: bool,
    why: Option<String>,
}

/// Under `Program Files (x86)` first (Java 8 is 32 bits), then `Program Files`.
fn find(tool: &OfficialTool) -> Option<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| PathBuf::from(dir).join(tool.relative_path))
        .find(|p| p.is_file() && !is_reparse_point(p))
}

/// The app is not elevated: what it starts is not either (R-DRT-01).
fn check(kind: &str) -> Result<(&'static OfficialTool, Option<PathBuf>, Launch), String> {
    let tool = tool(kind).ok_or("unknown tool")?;
    let path = find(tool);
    // App side: revocation from Windows' cache only, no network for it.
    let signer = path.as_ref().map(|p| file_signer(p, false));
    let launch = decide_launch(
        tool,
        path.is_some(),
        match &signer {
            Some(Ok(name)) => Ok(name.as_str()),
            Some(Err(why)) => Err(why.as_str()),
            None => Err("absent"),
        },
    );
    Ok((tool, path, launch))
}

pub fn tool_check(kind: &str) -> Result<ToolCheck, String> {
    let (tool, _, launch) = check(kind)?;
    Ok(match launch {
        Launch::Run => ToolCheck {
            name: tool.name,
            run: true,
            why: None,
        },
        Launch::LinkOnly(why) => ToolCheck {
            name: tool.name,
            run: false,
            why: Some(why),
        },
    })
}

/// Checks again right before starting it (the file could have changed since).
pub fn tool_launch(kind: &str) -> Result<(), String> {
    let (_, path, launch) = check(kind)?;
    match (launch, path) {
        (Launch::Run, Some(path)) => std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("not started: {e}")),
        (Launch::LinkOnly(why), _) => Err(why),
        (Launch::Run, None) => Err("tool not found".to_owned()),
    }
}

/// The official page in the user's browser (a "lien seulement" host: Drivn itself never calls it).
pub fn tool_page(kind: &str) -> Result<(), String> {
    let tool = tool(kind).ok_or("unknown tool")?;
    std::process::Command::new(windows_dir()?.join("explorer.exe"))
        .arg(page_url(tool))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
